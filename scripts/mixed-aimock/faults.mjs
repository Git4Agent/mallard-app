import assert from "node:assert/strict";
import { readFile, writeFile, rm, symlink } from "node:fs/promises";
import { ServiceWorker } from "./service.mjs";
import { nativeCutoff } from "./native.mjs";

export async function graphFaults({ binary, root, replicas, checkpoints, native, fault, workers }) {
  if (!fault) return { status: "not_applicable", reason: "small_subset_has_no_forks" };
  const results = [];
  const childId = native.sessionIds.get(fault.child);
  const independentId = native.sessionIds.get(fault.independent);
  const cases = ["missing_base", "byte_cutoff", "ordinal_cutoff", "cycle", "truncated_base", "symlink"];
  for (const name of cases) {
    await checkpoints.restore("accepted");
    const child = await nativeCutoff(replicas.a.codex, childId);
    const base = await nativeCutoff(replicas.a.codex, child.base.thread_id);
    if (name === "missing_base") await rm(base.path);
    else if (name === "symlink") { await rm(child.path); await symlink(base.path, child.path); }
    else if (name === "truncated_base") {
      const bytes = await readFile(base.path);
      await writeFile(base.path, bytes.subarray(0, child.base.end_byte_offset - 11));
    }
    else {
      const bytes = await readFile(child.path);
      const records = bytes.toString("utf8").trimEnd().split("\n").map((line) => JSON.parse(line));
      if (name === "byte_cutoff") records[0].payload.history_base.end_byte_offset--;
      if (name === "ordinal_cutoff") records[0].payload.history_base.end_ordinal_exclusive--;
      if (name === "cycle") records[0].payload.history_base.thread_id = childId;
      await writeFile(child.path, `${records.map((r) => JSON.stringify(r)).join("\n")}\n`);
    }
    const worker = new ServiceWorker(binary, root, replicas.a, [], workers);
    try {
      await worker.open();
      const result = await worker.call("graph", { ids: [childId] }, true);
      assert.equal(result.ok, false, `graph fault was accepted: ${name}`);
      if (name !== "symlink") assert.equal((await worker.call("graph", { ids: [independentId] })).ok, true, "independent graph failed");
      results.push({ case: name, status: "passed" });
    } finally { await worker.close().catch(() => worker.kill()); }
  }
  return { status: "passed", cases: results };
}
