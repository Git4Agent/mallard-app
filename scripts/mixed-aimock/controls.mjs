import assert from "node:assert/strict";
import { readFile, writeFile, rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { files, nativeCutoff } from "./native.mjs";

// Deliberately injected exclusion controls. These are not native sessions and
// never count toward either profile's native source-session requirement.
export async function exclusionControls(native, worker) {
  const replica = native.replicas.a;
  const controlId = "018f3f2e-7b91-7ad0-8000-000000000999";
  const codex = await nativeCutoff(replica.codex, native.sessionIds.get("C010"));
  const codexRecords = (await readFile(codex.path, "utf8")).trimEnd().split("\n").map((line) => JSON.parse(line));
  codexRecords[0].payload.id = controlId;
  codexRecords[0].payload.thread_source = "subagent";
  const codexPath = join(dirname(codex.path), `rollout-control-${controlId}.jsonl`);
  const claudePath = (await files(join(replica.claude, "projects"))).find((path) => path.endsWith(`${native.sessionIds.get("L001")}.jsonl`));
  assert.ok(claudePath, "Claude exclusion source missing");
  const foreign = replica.project.replace(/project-a$/u, "project_a");
  assert.equal(foreign.replace(/[^A-Za-z0-9]/gu, "-"), replica.project.replace(/[^A-Za-z0-9]/gu, "-"), "Claude bucket collision control");
  const claudeRecords = (await readFile(claudePath, "utf8")).trimEnd().split("\n").map((line) => {
    const record = JSON.parse(line);
    if (Object.hasOwn(record, "cwd")) record.cwd = foreign;
    if (Object.hasOwn(record, "sessionId")) record.sessionId = controlId;
    return record;
  });
  const foreignPath = join(dirname(claudePath), `${controlId}.jsonl`);
  const before = (await worker.compare()).entries;
  try {
    await writeFile(codexPath, `${codexRecords.map((r) => JSON.stringify(r)).join("\n")}\n`, { flag: "wx", mode: 0o600 });
    await writeFile(foreignPath, `${claudeRecords.map((r) => JSON.stringify(r)).join("\n")}\n`, { flag: "wx", mode: 0o600 });
    const rows = (await worker.compare()).entries;
    assert.equal(rows.length, before.length, "injected controls entered comparison");
    assert.ok(rows.every((row) => row.provider_thread_id !== controlId), "injected control identity selected");
    const review = (await worker.call("push_review")).value;
    assert.equal(review.thread_comparison.entries.length, before.length, "injected controls entered Push review");
    return { internal_codex_subagent_excluded: true, foreign_claude_bucket_excluded: true };
  } finally { await rm(codexPath, { force: true }); await rm(foreignPath, { force: true }); }
}
