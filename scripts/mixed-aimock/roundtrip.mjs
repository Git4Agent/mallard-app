import assert from "node:assert/strict";
import { relocatedHistory } from "./native.mjs";

// Expectations come from authored fixture turns and explicit transport hops.
// The product supplies identities only; it never supplies expected message text.
export class Ledger {
  constructor(native, nodes, replicas, workers, clouds) {
    Object.assign(this, { native, replicas, workers });
    this.entries = new Map(nodes.map((node) => [node.label, { node, cloud: clouds.get(node.label),
      local: { a: native.histories.get(node.label), b: relocatedHistory(native.histories.get(node.label), replicas.a.project, replicas.b.project) },
      remote: { history: native.histories.get(node.label), root: replicas.a.project } }]));
  }
  async rows(replica) { return (await this.workers[replica].compare()).entries.filter((row) => row.local_present); }
  row(rows, entry) {
    const row = rows.find((row) => row.cloud_thread_id === entry.cloud && row.provider === entry.node.provider);
    assert.ok(row?.provider_thread_id, "ledger native identity missing"); return row;
  }
  async append(replica, labels, suffix, rich = false) {
    const rows = await this.rows(replica);
    for (const label of labels) {
      const entry = this.entries.get(label); assert.ok(entry, "ledger label missing");
      const row = this.row(rows, entry);
      const result = await this.native.turns(this.replicas[replica], { ...entry.node, richAppend: rich }, 1, 1,
        entry.local[replica], row.provider_thread_id, null, suffix);
      entry.local[replica] = result.history;
    }
  }
  async publish(replica, labels = [...this.entries.keys()]) {
    const rows = await this.rows(replica);
    await this.workers[replica].push(new Set(labels.map((label) => this.row(rows, this.entries.get(label)).provider_thread_id)));
    for (const label of labels) {
      const entry = this.entries.get(label);
      entry.remote = { history: structuredClone(entry.local[replica]), root: this.replicas[replica].project };
    }
  }
  async pull(replica, conflicts = []) {
    const worker = this.workers[replica]; const before = await this.rows(replica);
    const plan = (await worker.call("pull_review")).value;
    if (plan.conversation_review.filter((row) => row.action === "keep_both").length !== conflicts.length) {
      const error = new Error("unexpected_conflict_count");
      error.privateDetail = JSON.stringify({ replica, expected: conflicts.length, actual: plan.conversation_review.filter((row) => row.action === "keep_both")
        .map((row) => [...this.entries].find(([, entry]) => entry.cloud === row.cloud_thread_id)?.[0] ?? "unmapped") });
      throw error;
    }
    const result = (await worker.call("pull_apply")).value.result;
    assert.equal(result.success, true, "ledger Pull failed");
    assert.equal(result.failed_actions.length, 0, "ledger Pull action failed");
    assert.equal(result.preserved_threads.length, conflicts.length, "Keep both receipt count");
    const after = await this.rows(replica); const added = [];
    for (const label of conflicts) {
      const entry = this.entries.get(label); const original = this.row(before, entry);
      const reviewed = plan.conversation_review.find((row) => row.cloud_thread_id === entry.cloud && row.action === "keep_both");
      const receipt = result.preserved_threads.find((item) => item.original_thread_id === reviewed?.thread_id);
      assert.ok(receipt, "Keep both receipt identity");
      const preserved = after.find((row) => row.provider_thread_id === original.provider_thread_id && row.provider === entry.node.provider);
      assert.ok(preserved.conversation_resource_id.endsWith(receipt.preserved_thread_id), "Keep both logical binding");
      assert.equal(preserved.provider_thread_id, original.provider_thread_id, "Keep both retained alias");
      assert.ok(!preserved.cloud_thread_id, "Keep both branch is already published");
      const branchLabel = `K${String(this.entries.size + added.length + 1).padStart(3, "0")}`;
      added.push({ branchLabel, entry, preserved });
    }
    for (const entry of this.entries.values()) {
      const action = plan.conversation_review.find((row) => row.cloud_thread_id === entry.cloud)?.action;
      if (!entry.local[replica] || ["keep_both", "materialize_cloud", "create_local"].includes(action)) {
        entry.local[replica] = relocatedHistory(entry.remote.history, entry.remote.root, this.replicas[replica].project);
      }
    }
    // Publish retained branches explicitly. Their identity is fixed by the receipt.
    if (added.length) {
      await worker.push(new Set(added.map(({ preserved }) => preserved.provider_thread_id)));
      const published = await this.rows(replica);
      for (const { branchLabel, entry, preserved } of added) {
        const row = published.find((row) => row.thread_id === preserved.thread_id);
        assert.ok(row.cloud_thread_id, "preserved branch cloud identity");
        // The native ID still addresses the old local branch. Save its expectation
        // before replacing the tracking branch below (captured in pull's caller).
        const old = this.divergent.get(entry.node.label);
        this.entries.set(branchLabel, { node: { ...entry.node, label: branchLabel, parent: null }, cloud: row.cloud_thread_id,
          local: { [replica]: old }, remote: { history: old, root: this.replicas[replica].project } });
      }
    }
  }
  async conflicts(groups, saveCheckpoint) {
    const initialCount = this.entries.size;
    for (const [index, group] of groups.entries()) {
      const winner = index === 0 ? "a" : "b"; const receiver = winner === "a" ? "b" : "a";
      await this.append("a", group, `CONFLICT_A_${index}`);
      await this.append("b", group, `CONFLICT_B_${index}`);
      await this.publish(winner, group);
      if (index === 0) await saveCheckpoint?.("conflict");
      this.divergent = new Map(group.map((label) => [label, structuredClone(this.entries.get(label).local[receiver])]));
      await this.pull(receiver, group);
      await this.pull(winner);
      for (const replica of ["a", "b"]) assert.equal((await this.rows(replica)).length, initialCount + groups.slice(0, index + 1).flat().length, "conflict convergence count");
    }
  }
  async verify(replica, suffix) {
    const rows = await this.rows(replica);
    assert.equal(rows.length, this.entries.size, "final logical session count");
    for (const [label, entry] of this.entries) {
      const row = this.row(rows, entry);
      assert.ok(entry.local[replica], "missing expected local history");
      await this.native.reopen(this.replicas[replica], entry.node, row.provider_thread_id, entry.local[replica], `${suffix}_${label}`);
    }
  }
  async roundtrip(labels) {
    console.log("Round trip: restore C."); await this.pull("c");
    await this.verify("c", "ROUNDTRIP_C");
    await this.append("c", labels, "ROUNDTRIP_C", true);
    await this.publish("c"); console.log("Round trip: restore A and B from C."); await this.pull("a"); await this.pull("b");
    await this.append("a", labels, "ROUNDTRIP_A", true);
    await this.publish("a"); console.log("Round trip: restore B from A."); await this.pull("b");
    await this.append("b", labels, "ROUNDTRIP_B", true);
    await this.publish("b"); console.log("Round trip: restore C and A from B."); await this.pull("c"); await this.pull("a");
    await this.verify("a", "FINAL_A"); await this.verify("b", "FINAL_B");
    const before = (await this.workers.a.call("inspect")).value;
    await this.publish("a"); await this.pull("b");
    const after = (await this.workers.a.call("inspect")).value;
    assert.deepEqual(after.head, before.head, "final no-op revision changed");
    assert.deepEqual(after.manifest, before.manifest, "final no-op manifest changed");
    return { total: this.entries.size,
      codex: [...this.entries.values()].filter((entry) => entry.node.provider === "codex").length,
      claude: [...this.entries.values()].filter((entry) => entry.node.provider === "claude").length };
  }
}
