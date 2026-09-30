import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { chmod, mkdir, open, readFile, rename, rm, stat, truncate, utimes, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { ServiceWorker } from "./service.mjs";
import { files, nativeCutoff } from "./native.mjs";
import { safeFailure } from "./report.mjs";

const NEW_SESSION = "019fb020-0000-7000-8000-000000000001";
const CHILD_SESSION = "019fb020-0000-7000-8000-000000000002";
const NEXT_ROLLOUT = "019fb020-0000-7000-8000-000000000003";
const FILE_LIMIT = 100 * 1024 * 1024;
const incomplete = new Set(["unknown", "unavailable"]);

function check(value, code) {
  if (!value) { const error = new Error(code); error.reconciliationCode = code; throw error; }
}
const encode = (records) => Buffer.from(`${records.map((record) => JSON.stringify(record)).join("\n")}\n`);
const decode = (bytes) => bytes.toString("utf8").trimEnd().split("\n").map((line) => JSON.parse(line));
const row = (comparison, provider, id) => comparison.entries.find((entry) => entry.provider === provider && entry.provider_thread_id === id);
const signature = (comparison) => comparison.entries.map((entry) => ({
  id: entry.conversation_resource_id, state: entry.state, local: entry.local_present,
  bytes: entry.capture_bytes, children: entry.subagent_selection,
})).sort((a, b) => a.id.localeCompare(b.id));

export function appendedRecords(provider, bytes, id, project) {
  const records = decode(bytes);
  if (provider === "codex") {
    const ordinal = records.at(-1).ordinal + 1;
    assert.ok(Number.isSafeInteger(ordinal), "native ordinal is required");
    return encode([{ ordinal, timestamp: "2026-09-28T01:00:00Z", type: "response_item", payload: {
      type: "message", id: randomUUID(), role: "user", content: [{ type: "input_text", text: "MALLARD_RECONCILIATION_APPEND" }],
    } }]);
  }
  const previous = records.findLast((record) => ["user", "assistant"].includes(record.type) && record.uuid);
  assert.ok(previous, "native Claude chain is required");
  return encode([{ type: "user", uuid: randomUUID(), parentUuid: previous.uuid,
    sessionId: id, cwd: project, isSidechain: previous.isSidechain ?? false,
    ...(previous.agentId ? { agentId: previous.agentId } : {}),
    message: { role: "user", content: "MALLARD_RECONCILIATION_APPEND" } }]);
}

export function rewrittenBytes(bytes) {
  const changed = Buffer.from(bytes.toString("utf8").replaceAll("MIXED_", "FIXED_"));
  check(changed.length === bytes.length && !changed.equals(bytes), "rewrite_fixture_missing");
  return changed;
}

export function nextRolloutName(path, id) {
  const timestamp = basename(path).slice("rollout-".length, "rollout-".length + 19);
  const iso = timestamp.replace(/T(\d{2})-(\d{2})-(\d{2})$/u, "T$1:$2:$3Z");
  const next = new Date(Date.parse(iso) + 1000).toISOString().slice(0, 19).replaceAll(":", "-");
  return `rollout-${next}-${id}_${NEXT_ROLLOUT}.jsonl`;
}

function truncatedBytes(provider, bytes) {
  const records = decode(bytes);
  const firstAssistant = records.findIndex((record) => provider === "codex"
    ? record.payload?.role === "assistant" || record.payload?.type === "agent_message"
    : record.type === "assistant");
  check(firstAssistant > 0, "truncate_fixture_missing");
  return encode(records.slice(0, firstAssistant));
}

async function sourcePath(replica, provider, id) {
  if (provider === "codex") return (await nativeCutoff(replica.codex, id)).path;
  const candidates = (await files(join(replica.claude, "projects"))).filter((path) => path.endsWith(`/${id}.jsonl`));
  check(candidates.length === 1, "claude_source_not_unique");
  return candidates[0];
}

async function writeNewSession(context) {
  const records = decode(context.bytes);
  const path = join(dirname(context.path), context.provider === "codex"
    ? `rollout-2026-09-29T00-00-00-${NEW_SESSION}.jsonl` : `${NEW_SESSION}.jsonl`);
  if (context.provider === "codex") {
    records[0].payload.id = NEW_SESSION;
  } else {
    const ids = new Map(records.filter((record) => record.uuid).map((record) => [record.uuid, randomUUID()]));
    for (const record of records) {
      if (record.sessionId) record.sessionId = NEW_SESSION;
      if (record.uuid) record.uuid = ids.get(record.uuid);
      if (record.parentUuid) record.parentUuid = ids.get(record.parentUuid) ?? null;
    }
  }
  await writeFile(path, encode(records), { mode: 0o600 });
  return path;
}

async function writeNextRollout(context) {
  const path = join(dirname(context.path), nextRolloutName(context.path, context.id));
  await writeFile(path, Buffer.concat([context.bytes,
    appendedRecords("codex", context.bytes, context.id, context.replica.project)]), { mode: 0o600 });
  return path;
}

async function writeChild(context, present) {
  const main = decode(context.bytes);
  let child; let childBytes;
  if (context.provider === "codex") {
    const ordinal = main.at(-1).ordinal + 1;
    main.push({ ordinal, timestamp: "2026-09-28T01:00:00Z", type: "event_msg", payload: { type: "sub_agent_activity", event_id: "reconciliation-child",
      kind: "started", agent_thread_id: CHILD_SESSION, agent_path: ["worker"], occurred_at_ms: 42 } });
    const meta = structuredClone(main[0]);
    meta.ordinal = 0;
    meta.payload.id = CHILD_SESSION;
    meta.payload.parent_thread_id = context.id;
    meta.payload.subagent_history_start_ordinal = 1;
    meta.payload.source = { subagent: { thread_spawn: { parent_thread_id: context.id, depth: 1, agent_path: ["worker"] } } };
    delete meta.payload.history_base;
    delete meta.payload.forked_from_id;
    childBytes = encode([meta, { ordinal: 1, timestamp: "2026-09-28T01:00:00Z", type: "response_item", payload: { type: "message", id: "child-question",
      role: "user", content: [{ type: "input_text", text: "MALLARD_RECONCILIATION_CHILD" }] } }]);
    child = join(dirname(context.path), `rollout-2026-09-29T00-00-00-${CHILD_SESSION}.jsonl`);
  } else {
    const previous = main.findLast((record) => ["user", "assistant"].includes(record.type) && record.uuid);
    main.push({ type: "user", uuid: randomUUID(), parentUuid: previous.uuid, sessionId: context.id,
      cwd: context.replica.project, isSidechain: false,
      message: { role: "user", content: [{ type: "tool_result", tool_use_id: "child-call", content: "done" }] },
      toolUseResult: { agentId: "reconciliation", isAsync: true } });
    child = join(dirname(context.path), context.id, "subagents", "agent-reconciliation.jsonl");
    childBytes = encode([{ type: "user", uuid: randomUUID(), parentUuid: null, sessionId: context.id,
      cwd: context.replica.project, agentId: "reconciliation", isSidechain: true,
      message: { role: "user", content: "MALLARD_RECONCILIATION_CHILD" } }]);
  }
  await writeFile(context.path, encode(main));
  await mkdir(dirname(child), { recursive: true, mode: 0o700 });
  if (present) await writeFile(child, childBytes, { mode: 0o600 });
  return { path: child, bytes: childBytes };
}

// Each case restores the accepted native fixture, starts a real Rust worker,
// warms its observation cache, and then changes files without an event input.
// Failed cases stay failed. Run every case before returning the report.
export async function sessionReconciliation({ binary, root, replicas, checkpoints, native, nodes, workers }) {
  const results = [];
  const run = async (provider, name, change) => {
    await checkpoints.restore("accepted");
    const node = nodes.find((candidate) => candidate.provider === provider && !candidate.parent
      && !nodes.some((child) => child.parent === candidate.label));
    check(node, "independent_reconciliation_source_missing");
    const id = native.sessionIds.get(node.label);
    const replica = replicas.a;
    const context = { provider, id, replica, path: await sourcePath(replica, provider, id), worker: null, writers: [], cleanup: [] };
    context.bytes = await readFile(context.path);
    context.start = async () => {
      context.worker = new ServiceWorker(binary, root, replica, [], workers);
      await context.worker.open();
    };
    context.stop = async () => {
      if (context.worker) { await context.worker.close().catch(() => context.worker.kill()); context.worker = null; }
    };
    context.compare = () => context.worker.compare();
    context.selected = (comparison) => row(comparison, provider, id);
    context.append = async (path = context.path, bytes = context.bytes) => {
      const writer = await open(path, "a"); context.writers.push(writer);
      await writer.writeFile(appendedRecords(provider, bytes, id, replica.project)); await writer.sync();
    };
    try {
      await context.start();
      for (let n = 0; n < 2; n++) check(context.selected(await context.compare())?.state === "synced", "warm_baseline_not_synced");
      await change(context);
      // Closing a writer must not be needed to obtain this result. A new
      // process must agree, including when it loads the persisted cache.
      const warm = signature(await context.compare());
      await context.stop(); await context.start();
      check(JSON.stringify(signature(await context.compare())) === JSON.stringify(warm), "restart_changed_result");
      results.push({ provider, case: name, status: "passed" });
    } catch (error) {
      const failure = safeFailure(error, "session_reconciliation");
      results.push({ provider, case: name, status: "failed", code: error.reconciliationCode ?? failure.code,
        ...(failure.assertion ? { assertion: failure.assertion } : {}),
        ...(failure.location ? { location: failure.location } : {}) });
    } finally {
      for (const writer of context.writers) await writer.close().catch(() => {});
      for (const cleanup of context.cleanup.reverse()) await cleanup();
      await context.stop();
    }
  };
  const changed = async (context) => {
    const entry = context.selected(await context.compare());
    check(entry && entry.state !== "synced", "changed_source_remained_synced");
  };
  const ahead = async (context) => check(context.selected(await context.compare())?.state === "local_ahead", "append_not_local_ahead");
  const blocked = async (context) => {
    const entry = context.selected(await context.compare());
    check(entry && incomplete.has(entry.state) && !entry.push_decision.selectable, "incomplete_source_not_blocked");
  };

  for (const provider of ["codex", "claude"]) {
    await run(provider, "append_open_writer", async (c) => { await c.append(); await ahead(c); });
    await run(provider, "rapid_writes_and_late_close", async (c) => {
      await c.append(); await ahead(c);
      await c.append(c.path, await readFile(c.path)); await ahead(c);
      const transient = await writeNewSession(c); await rm(transient);
      const before = signature(await c.compare());
      for (const writer of c.writers.reverse()) { await writer.close(); check(JSON.stringify(signature(await c.compare())) === JSON.stringify(before), "late_close_changed_result"); }
      c.writers.length = 0;
    });
    await run(provider, "equal_size_overwrite", async (c) => {
      const old = await stat(c.path); await writeFile(c.path, rewrittenBytes(c.bytes)); await utimes(c.path, old.atime, old.mtime); await changed(c);
    });
    await run(provider, "truncate", async (c) => { await writeFile(c.path, truncatedBytes(provider, c.bytes)); await changed(c); });
    await run(provider, "atomic_replacement", async (c) => {
      const replacement = `${c.path}.replacement`; const old = await stat(c.path);
      await writeFile(replacement, rewrittenBytes(c.bytes), { mode: 0o600 }); await utimes(replacement, old.atime, old.mtime);
      await rename(replacement, c.path); await changed(c);
    });
    await run(provider, "new_and_deleted_inventory", async (c) => {
      const added = await writeNewSession(c);
      check(row(await c.compare(), provider, NEW_SESSION)?.state === "local_only", "new_session_missing");
      await rm(added);
      check(!row(await c.compare(), provider, NEW_SESSION)?.local_present, "deleted_session_still_local");
      await rm(c.path);
      check(c.selected(await c.compare())?.local_present === false, "deleted_accepted_session_still_local");
    });
    await run(provider, "rename", async (c) => {
      const moved = join(dirname(dirname(c.path)), "reconciliation-moved", basename(c.path));
      await mkdir(dirname(moved), { recursive: true }); await rename(c.path, moved); c.path = moved;
      check(c.selected(await c.compare())?.state === "synced", "rename_lost_source");
      await c.append(); await ahead(c);
    });
    await run(provider, "provider_root_replacement", async (c) => {
      const directory = join(c.replica[provider], provider === "codex" ? "sessions" : "projects");
      const parked = join(root, `parked-${provider}-root`); await rename(directory, parked);
      c.cleanup.push(() => rm(parked, { recursive: true, force: true }));
      await mkdir(dirname(c.path), { recursive: true }); await writeFile(c.path, c.bytes, { mode: 0o600 });
      await c.append(); await ahead(c);
      check((await c.compare()).entries.filter((entry) => entry.provider === provider && entry.local_present).length === 1, "replaced_root_reused_inventory");
    });
    await run(provider, "restart_with_stale_disk_cache", async (c) => {
      await c.stop(); await c.append(); await c.start(); await ahead(c);
    });
    await run(provider, "file_size_limit", async (c) => { await truncate(c.path, FILE_LIMIT + 1); await blocked(c); });
    if (process.getuid?.() !== 0 && process.platform !== "win32") {
      await run(provider, "read_error", async (c) => {
        const mode = (await stat(c.path)).mode & 0o777; c.cleanup.push(() => chmod(c.path, mode));
        await chmod(c.path, 0); await blocked(c);
      });
    }
    await run(provider, "child_change_and_return", async (c) => {
      const child = await writeChild(c, true);
      await c.worker.call("open", { replica: "a", project: c.replica.project, include_children: true, workers });
      await c.worker.push();
      check(c.selected(await c.compare())?.state === "synced", "child_baseline_not_synced");
      const parent = await readFile(c.path);
      await c.append(child.path, child.bytes); await ahead(c);
      const parked = `${child.path}.parked`; await rename(child.path, parked); await blocked(c);
      await rename(parked, child.path); await ahead(c);
      check((await readFile(c.path)).equals(parent), "child_case_changed_parent");
    });
    await run(provider, "previously_missing_child_appears", async (c) => {
      const child = await writeChild(c, false);
      await c.worker.call("open", { replica: "a", project: c.replica.project, include_children: true, workers });
      const parent = await readFile(c.path);
      for (let n = 0; n < 2; n++) check(c.selected(await c.compare())?.subagent_selection.keys.length === 0, "missing_child_was_present");
      await writeFile(child.path, child.bytes, { mode: 0o600 });
      check(c.selected(await c.compare())?.subagent_selection.keys.length === 1, "new_child_membership_stale");
      check((await readFile(c.path)).equals(parent), "appearance_changed_parent");
    });
    await run(provider, "two_projects_share_source", async (c) => {
      const machine = join(replicas.c.base, "machine"); await rm(machine, { recursive: true, force: true }); await mkdir(machine);
      const store = join(root, "reconciliation-storage"); await rm(store, { recursive: true, force: true }); await mkdir(store);
      const shared = { replica: "c", source_replica: "a", project: replicas.a.project,
        storage_id: "reconciliation-second-store", separate_storage: true, workers };
      await c.worker.call("open", shared); await c.worker.push();
      check(c.selected(await c.compare())?.state === "synced", "shared_source_baseline_not_synced");
      await c.worker.open(); await c.append(); await ahead(c);
      await c.worker.call("open", shared); await ahead(c);
      await c.worker.open(); await ahead(c);
    });
  }
  await run("codex", "archive_move", async (c) => {
    const archived = join(c.replica.codex, "archived_sessions", basename(c.path));
    await mkdir(dirname(archived), { recursive: true }); await rename(c.path, archived); c.path = archived;
    check(c.selected(await c.compare())?.state === "synced", "archive_lost_source");
    await c.append(); await ahead(c);
  });
  await run("codex", "new_physical_rollout", async (c) => { await writeNextRollout(c); await ahead(c); });
  return { status: results.every((result) => result.status !== "failed") ? "passed" : "failed", cases: results };
}
