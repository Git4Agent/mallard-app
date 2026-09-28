import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmod, lstat, mkdir, mkdtemp, open, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// Run with sdk.mjs from an isolated installation of the pinned SDK:
// node scripts/run-claude-session-delete-poc.mjs /path/to/sdk.mjs
// No model calls, real provider homes, or Mallard state are used.
const SDK_VERSION = "0.3.280";
const script = fileURLToPath(import.meta.url);
const repository = resolve(dirname(script), "..");
const targetId = "11111111-1111-4111-8111-111111111111";
const parentId = "22222222-2222-4222-8222-222222222222";
const siblingId = "33333333-3333-4333-8333-333333333333";
const forkId = "44444444-4444-4444-8444-444444444444";

async function exists(path) {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function write(path, body) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, body, { mode: 0o600 });
}

function transcript(sessionId, cwd, extra = {}) {
  return JSON.stringify({
    type: "user",
    uuid: "55555555-5555-4555-8555-555555555555",
    parentUuid: null,
    sessionId,
    cwd,
    timestamp: "2026-09-26T00:00:00.000Z",
    message: { role: "user", content: "Synthetic deletion POC" },
    ...extra,
  }) + "\n";
}

async function runWorker(sdkPath) {
  const { deleteSession } = await import(pathToFileURL(sdkPath).href);
  const root = process.env.MALLARD_CLAUDE_DELETE_POC_ROOT;
  assert.ok(root && process.env.CLAUDE_CONFIG_DIR === join(root, "config"));
  const cases = [];
  let sequence = 0;

  async function fixture() {
    const project = join(root, `project-${++sequence}`);
    await mkdir(project);
    const bucket = join(process.env.CLAUDE_CONFIG_DIR, "projects", project.replace(/[^a-zA-Z0-9]/g, "-"));
    const main = join(bucket, `${targetId}.jsonl`);
    const children = join(bucket, targetId);
    await write(main, transcript(targetId, project));
    return { project, bucket, main, children };
  }

  // The proposed best-effort boundary: call once and report the outcome.
  async function tryDelete(fixture) {
    try {
      await deleteSession(targetId, { dir: fixture.project });
      return { status: "deleted" };
    } catch (error) {
      return {
        status: "failed",
        error: /not found/i.test(error.message) ? "session_not_found" : error.code ?? error.name,
      };
    }
  }

  async function scenario(name, run) {
    try {
      cases.push({ name, passed: true, ...await run() });
    } catch (error) {
      cases.push({ name, passed: false, error: error.code ?? error.name });
    }
  }

  await scenario("root_transcript", async () => {
    const f = await fixture();
    const outcome = await tryDelete(f);
    assert.equal(outcome.status, "deleted");
    assert.equal(await exists(f.main), false);
    return { outcome, rootRemoved: true };
  });

  await scenario("descendants_and_scope", async () => {
    const f = await fixture();
    const keep = [];
    for (const [id, extra] of [[parentId, {}], [siblingId, {}], [forkId, { forkedFrom: { sessionId: targetId } }]]) {
      const path = join(f.bucket, `${id}.jsonl`);
      const body = transcript(id, f.project, extra);
      await write(path, body);
      keep.push([path, body]);
    }
    const parentChild = join(f.bucket, parentId, "subagents", "agent-parent-child.jsonl");
    await write(parentChild, "parent-owned child\n");
    keep.push([parentChild, "parent-owned child\n"]);
    // Inherited parent content must not cause deletion of its original file.
    await write(f.main, transcript(parentId, f.project) + transcript(targetId, f.project));
    await write(join(f.children, "subagents", "agent-child.jsonl"), transcript(targetId, f.project, { agentId: "child", isSidechain: true }));
    await write(join(f.children, "subagents", "agent-grandchild.jsonl"), transcript(targetId, f.project, { agentId: "grandchild", isSidechain: true }));
    await write(join(f.children, "subagents", "agent-grandchild.meta.json"), JSON.stringify({ parentAgentId: "child" }));
    await write(join(f.children, "nested", "subagents", "agent-deeper.jsonl"), "nested fixture\n");
    await write(join(f.children, "tool-results", "result.txt"), "extra session file\n");
    const sidecar = join(process.env.CLAUDE_CONFIG_DIR, "file-history", targetId, "file@v1");
    await write(sidecar, "file-history fixture\n");
    const outcome = await tryDelete(f);
    assert.equal(outcome.status, "deleted");
    assert.equal(await exists(f.main), false);
    assert.equal(await exists(f.children), false);
    for (const [path, body] of keep) assert.equal(await readFile(path, "utf8"), body);
    assert.equal(await readFile(sidecar, "utf8"), "file-history fixture\n");
    return { outcome, rootAndDescendantDirectoryRemoved: true, parentSiblingForkUnchanged: true, extraSessionFilesRemoved: true, externalFileHistoryRetained: true };
  });

  await scenario("repeat_delete", async () => {
    const f = await fixture();
    assert.equal((await tryDelete(f)).status, "deleted");
    const outcome = await tryDelete(f);
    assert.deepEqual(outcome, { status: "failed", error: "session_not_found" });
    return { outcome, alreadyAbsent: true };
  });

  await scenario("new_file_after_preview", async () => {
    const f = await fixture();
    const reviewed = [f.main];
    const lateFile = join(f.children, "subagents", "agent-late.jsonl");
    await write(lateFile, "created after the simulated preview\n");
    assert.equal(reviewed.includes(lateFile), false);
    const outcome = await tryDelete(f);
    assert.equal(outcome.status, "deleted");
    assert.equal(await exists(lateFile), false);
    return { outcome, unreviewedFileAlsoRemoved: true };
  });

  await scenario("partial_failure_and_retry", async () => {
    if (process.platform === "win32" || process.getuid?.() === 0) return { skipped: "requires non-root POSIX permissions" };
    const f = await fixture();
    const child = join(f.children, "agent-child.jsonl");
    await write(child, "child fixture\n");
    await chmod(f.children, 0o500);
    try {
      const outcome = await tryDelete(f);
      assert.equal(outcome.status, "failed");
      assert.equal(await exists(f.main), false);
      assert.equal(await exists(child), true);
      const retry = await tryDelete(f);
      assert.deepEqual(retry, { status: "failed", error: "session_not_found" });
      assert.equal(await exists(child), true);
      return { outcome, rootRemoved: true, childRetained: true, retry };
    } finally {
      await chmod(f.children, 0o700);
    }
  });

  await scenario("open_writer_and_recreation", async () => {
    if (process.platform === "win32") return { skipped: "POSIX open-file behavior" };
    const f = await fixture();
    const writer = await open(f.main, "a");
    let outcome;
    try {
      const before = await writer.stat();
      outcome = await tryDelete(f);
      assert.equal(outcome.status, "deleted");
      assert.equal(await exists(f.main), false);
      await writer.write("writer still holds the deleted file\n");
      const after = await writer.stat();
      assert.ok(after.size > before.size);
      assert.equal(after.nlink, 0);
    } finally {
      await writer.close();
    }
    await write(f.main, transcript(targetId, f.project));
    assert.equal(await exists(f.main), true);
    return { outcome, openWriterContinued: true, simulatedWriterRecreatedTranscript: true };
  });

  return { sdkVersion: SDK_VERSION, nodeVersion: process.version, platform: process.platform, fixtureType: "synthetic temporary transcripts", modelCalls: 0, cases };
}

if (process.argv[2] === "--worker") {
  console.log(JSON.stringify(await runWorker(process.argv[3])));
} else {
  assert.ok(process.argv[2], "Pass the isolated SDK sdk.mjs path");
  const sdkPath = resolve(process.argv[2]);
  const metadata = JSON.parse(await readFile(join(dirname(sdkPath), "package.json"), "utf8"));
  assert.equal(metadata.version, SDK_VERSION);
  const root = await mkdtemp(join(process.platform === "darwin" ? "/private/tmp" : tmpdir(), "mallard-claude-delete-fixtures-"));
  try {
    for (const directory of ["home", "config", "tmp"]) await mkdir(join(root, directory), { mode: 0o700 });
    const result = spawnSync(process.execPath, [script, "--worker", sdkPath], {
      cwd: root,
      env: {
        PATH: process.env.PATH ?? "/usr/bin:/bin",
        HOME: join(root, "home"),
        USERPROFILE: join(root, "home"),
        TMPDIR: join(root, "tmp"),
        CLAUDE_CONFIG_DIR: join(root, "config"),
        MALLARD_CLAUDE_DELETE_POC_ROOT: root,
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        DISABLE_TELEMETRY: "1",
        DISABLE_ERROR_REPORTING: "1",
      },
      encoding: "utf8",
      timeout: 30_000,
      maxBuffer: 1024 * 1024,
    });
    assert.equal(result.status, 0, `POC worker failed: ${result.error?.code ?? result.signal ?? "nonzero exit"}`);
    const report = JSON.parse(result.stdout);
    report.status = report.cases.every((entry) => entry.passed) ? "passed" : "failed";
    const output = join(repository, "output", "claude-session-delete-poc", "results.json");
    await write(output, JSON.stringify(report, null, 2) + "\n");
    console.log(JSON.stringify(report, null, 2));
    process.exitCode = report.status === "passed" ? 0 : 1;
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}
