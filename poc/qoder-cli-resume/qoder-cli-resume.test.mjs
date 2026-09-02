import assert from "node:assert/strict";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  CREDENTIAL_CANARY,
  POC_STATUS,
  SESSION_ID,
  SESSION_TITLE,
  computeCliProjectBucket,
  createSyntheticMembers,
  executeCliResumePoc,
} from "./qoder-cli-resume.mjs";

async function makeMockCli(t, behavior = {}) {
  const root = await realpath(
    await mkdtemp(join(tmpdir(), "mallard-qoder-resume-mock-")),
  );
  t.after(async () => {
    await rm(root, { recursive: true, force: true });
  });
  const bundleRoot = join(root, "bundle");
  await mkdir(bundleRoot, { mode: 0o700 });
  const script = join(bundleRoot, "qodercli.js");
  await writeFile(
    join(root, "package.json"),
    `${JSON.stringify({
      name: "@qoder-ai/qodercli",
      version: behavior.packageVersion ?? "1.1.40",
    })}\n`,
    { mode: 0o600 },
  );
  const configuration = JSON.stringify({
    omitListMarkers: behavior.omitListMarkers === true,
    mutateTranscript: behavior.mutateTranscript === true,
    leakCanary: behavior.leakCanary === true,
    tryNetwork: behavior.tryNetwork === true,
  });
  await writeFile(
    script,
    `import { appendFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

const behavior = ${configuration};
const args = process.argv.slice(2);
function value(name) {
  const index = args.indexOf(name);
  if (index < 0 || index + 1 >= args.length) throw new Error("missing " + name);
  return args[index + 1];
}
function bucket(projectPath) {
  const processed = projectPath.replace(/[^a-zA-Z0-9]/g, "-");
  if (processed.length <= 200) return processed;
  let hash = 5381;
  for (let index = 0; index < projectPath.length; index += 1) {
    hash = (33 * hash) ^ projectPath.charCodeAt(index);
  }
  return processed.slice(0, 200) + "-" + Math.abs(hash).toString(36);
}

const configRoot = value("--config-dir");
const projectRoot = value("--cwd");
if (process.env.HOME === undefined || process.env.QODER_CONFIG_DIR !== configRoot) {
  throw new Error("isolation missing");
}
for (const name of ["QODER_PERSONAL_ACCESS_TOKEN", "QODER_ACCESS_TOKEN", "OPENAI_API_KEY"]) {
  if (process.env[name] !== undefined) throw new Error("credential environment inherited");
}
const bucketRoot = join(configRoot, "projects", bucket(projectRoot));
const transcriptPath = join(bucketRoot, "${SESSION_ID}.jsonl");
const runtimeRoot = join(dirname(configRoot), "mock-runtime");
await mkdir(runtimeRoot, { recursive: true });

if (args.includes("--list-sessions")) {
  const records = (await readFile(transcriptPath, "utf8"))
    .trim()
    .split("\\n")
    .map((line) => JSON.parse(line));
  const workspace = records.find((record) => record.type === "workspace-directories");
  const leaf = records.find((record) => record.type === "active-leaf");
  if (!Array.isArray(workspace?.directories) || workspace.directories[0] !== projectRoot) {
    throw new Error("workspace record mismatch");
  }
  if (leaf?.explicit !== true) throw new Error("active leaf mismatch");
  await writeFile(join(runtimeRoot, "list-state.json"), "created-by-list\\n");
  await writeFile(join(runtimeRoot, "delete-on-resume.json"), "delete-me\\n");
  if (!behavior.omitListMarkers) {
    process.stdout.write("${SESSION_ID} ${SESSION_TITLE}\\n");
  }
  process.exitCode = 0;
} else if (args.includes("--resume")) {
  for (const relative of [
    "${SESSION_ID}.jsonl",
    "${SESSION_ID}/state.json",
    "${SESSION_ID}/compression-v2/state.json",
    "${SESSION_ID}/subagents/agent-explore-0123456789abcdef.jsonl",
    "${SESSION_ID}/subagents/agent-explore-0123456789abcdef.meta.json",
    "${SESSION_ID}/subagents/task-fedcba9876543210.json",
  ]) {
    await readFile(join(bucketRoot, ...relative.split("/")));
  }
  await writeFile(join(runtimeRoot, "list-state.json"), "modified-by-resume\\n");
  await rm(join(runtimeRoot, "delete-on-resume.json"));
  await writeFile(join(runtimeRoot, "resume-state.json"), "created-by-resume\\n");
  if (behavior.mutateTranscript) {
    await appendFile(transcriptPath, JSON.stringify({ type: "mutation" }) + "\\n");
  }
  if (behavior.tryNetwork) {
    try {
      await fetch("https://example.invalid/");
    } catch {}
  }
  if (behavior.leakCanary) process.stderr.write("${CREDENTIAL_CANARY}\\n");
  process.stderr.write("Not logged in. Run qodercli login to authenticate.\\n");
  process.exitCode = 23;
} else {
  throw new Error("unknown operation");
}
`,
    { mode: 0o700 },
  );
  return script;
}

function mutationPaths(mutations, type) {
  return mutations[type].map((entry) => entry.path).sort();
}

test("computes the observed Qoder CLI project bucket", () => {
  assert.equal(computeCliProjectBucket("/tmp/a b/project"), "-tmp-a-b-project");
  const longPath = `/${"a".repeat(220)}/project`;
  assert.equal(
    computeCliProjectBucket(longPath),
    `${"-"}${"a".repeat(199)}-kc6100`,
  );
  assert.throws(
    () => computeCliProjectBucket("relative/project"),
    (error) => error.code === "invalid_project_path",
  );
});

test("builds the six-member synthetic fixture", () => {
  const members = createSyntheticMembers({ projectRoot: "/synthetic/project" });
  assert.equal(members.length, 6);
  assert.deepEqual(
    members.map((member) => member.logicalPath),
    [
      `${SESSION_ID}.jsonl`,
      `${SESSION_ID}/compression-v2/state.json`,
      `${SESSION_ID}/state.json`,
      `${SESSION_ID}/subagents/agent-explore-0123456789abcdef.jsonl`,
      `${SESSION_ID}/subagents/agent-explore-0123456789abcdef.meta.json`,
      `${SESSION_ID}/subagents/task-fedcba9876543210.json`,
    ],
  );
  const transcript = members.find((member) =>
    member.logicalPath.endsWith(".jsonl") && !member.logicalPath.includes("/"),
  );
  const records = transcript.bytes
    .toString("utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.deepEqual(records[0].directories, ["/synthetic/project"]);
  assert.equal("workspaceDirectories" in records[0], false);
  assert.equal(records.at(-1).type, "active-leaf");
  assert.equal(records.at(-1).explicit, true);
});

test("lists the session and stops resume without session-content mutation", async (t) => {
  const cliBundle = await makeMockCli(t);
  let isolationRoot;
  const result = await executeCliResumePoc(
    { cliBundle, timeoutMs: 10_000 },
    { onIsolationRoot: (root) => (isolationRoot = root) },
  );

  assert.equal(result.ok, true);
  assert.deepEqual(result.cli, {
    name: "@qoder-ai/qodercli",
    version: "1.1.40",
  });
  assert.deepEqual(
    { productionReady: result.productionReady, verdict: result.verdict },
    POC_STATUS,
  );
  assert.equal(result.restore.memberCount, 6);
  assert.equal(result.restore.verified, true);
  assert.equal(result.evidence.listDiscovered, true);
  assert.equal(result.evidence.authGateObserved, true);
  assert.equal(result.evidence.authenticatedResumeProved, false);
  assert.equal(result.evidence.credentialFindingCount, 0);
  assert.equal(result.evidence.interceptedInterfacePermittedOperationCount, 0);
  assert.equal(result.evidence.deniedOperationCount, 0);
  assert.equal(
    result.commands.list.sessionGroup.matchesContentManifest,
    true,
  );
  assert.equal(
    result.commands.resume.sessionGroup.matchesContentManifest,
    true,
  );
  assert.deepEqual(result.commands.resume.sessionGroup.verifiedProperties, [
    "member_set",
    "size",
    "sha256",
  ]);
  assert.equal(result.commands.list.exitCode, 0);
  assert.equal(result.commands.resume.exitCode, 23);
  assert.deepEqual(result.commands.list.invocation.authEnvironmentKeys, []);
  assert.equal(
    result.commands.list.invocation.cleanEnvironmentKeys.includes(
      "QODER_PERSONAL_ACCESS_TOKEN",
    ),
    false,
  );

  const listCreated = mutationPaths(result.commands.list.mutations, "created");
  assert.ok(listCreated.includes("mock-runtime/list-state.json"));
  assert.ok(listCreated.includes("mock-runtime/delete-on-resume.json"));
  const resumeCreated = mutationPaths(result.commands.resume.mutations, "created");
  const resumeDeleted = mutationPaths(result.commands.resume.mutations, "deleted");
  assert.ok(resumeCreated.includes("mock-runtime/resume-state.json"));
  assert.ok(resumeDeleted.includes("mock-runtime/delete-on-resume.json"));
  assert.ok(
    result.commands.resume.mutations.modified.some(
      (entry) => entry.path === "mock-runtime/list-state.json",
    ),
  );

  assert.equal(JSON.stringify(result).includes(isolationRoot), false);
  assert.equal(
    JSON.stringify(result).includes(
      computeCliProjectBucket(join(isolationRoot, "synthetic-project")),
    ),
    false,
  );
  await assert.rejects(access(isolationRoot), (error) => error.code === "ENOENT");
});

test("reports missing list discovery without exposing raw output", async (t) => {
  const result = await executeCliResumePoc({
    cliBundle: await makeMockCli(t, { omitListMarkers: true }),
    timeoutMs: 10_000,
  });
  assert.equal(result.ok, false);
  assert.ok(result.failures.includes("cli_list_discovery_unproved"));
  assert.equal("stdout" in result.commands.list.output, false);
  assert.equal("stderr" in result.commands.list.output, false);
});

test("fails closed when a credential canary reaches child output", async (t) => {
  const result = await executeCliResumePoc({
    cliBundle: await makeMockCli(t, { leakCanary: true }),
    timeoutMs: 10_000,
  });
  assert.equal(result.ok, false);
  assert.ok(result.failures.includes("credential_material_observed"));
  assert.ok(result.evidence.credentialFindingCount > 0);
  assert.equal(JSON.stringify(result).includes(CREDENTIAL_CANARY), false);
  assert.equal(result.cleanup.complete, true);
});

test("records and rejects a resume content mutation to the synthetic group", async (t) => {
  const result = await executeCliResumePoc({
    cliBundle: await makeMockCli(t, { mutateTranscript: true }),
    timeoutMs: 10_000,
  });
  assert.equal(result.ok, false);
  assert.ok(result.failures.includes("resume_changed_session_group_content"));
  assert.deepEqual(result.commands.resume.sessionGroup.changedContent, [
    `${SESSION_ID}.jsonl`,
  ]);
  assert.ok(
    result.commands.resume.mutations.modified.some(
      (entry) => entry.path.endsWith(`${SESSION_ID}.jsonl`),
    ),
  );
});

test("blocks and records a child network operation", async (t) => {
  const result = await executeCliResumePoc({
    cliBundle: await makeMockCli(t, { tryNetwork: true }),
    timeoutMs: 10_000,
  });
  assert.equal(result.ok, false);
  assert.ok(result.failures.includes("offline_guard_intercepted_operation"));
  assert.equal(result.commands.resume.offlineGuard.networkDeniedCount, 1);
  assert.deepEqual(result.commands.resume.offlineGuard.operations, ["fetch"]);
});

test("rejects a caller-supplied CLI symlink before creating isolation", async (t) => {
  const cliBundle = await makeMockCli(t);
  const symlinkPath = join(tmpdir(), `mallard-qoder-cli-link-${process.pid}-${Date.now()}`);
  await symlink(cliBundle, symlinkPath);
  t.after(async () => {
    await rm(symlinkPath, { force: true });
  });
  const result = await executeCliResumePoc({ cliBundle: symlinkPath });
  assert.equal(result.ok, false);
  assert.equal(result.error.code, "invalid_cli_bundle");
  assert.deepEqual(result.cleanup, { attempted: false, complete: true });
});

test("rejects a relative CLI bundle before creating isolation", async () => {
  const result = await executeCliResumePoc({ cliBundle: "mock-qoder.mjs" });
  assert.equal(result.ok, false);
  assert.equal(result.error.code, "invalid_cli_bundle");
  assert.deepEqual(result.cleanup, { attempted: false, complete: true });
});

test("rejects a Qoder CLI package version mismatch", async (t) => {
  const result = await executeCliResumePoc({
    cliBundle: await makeMockCli(t, { packageVersion: "1.1.39" }),
  });
  assert.equal(result.ok, false);
  assert.equal(result.error.code, "unexpected_cli_version");
  assert.deepEqual(result.cleanup, { attempted: false, complete: true });
});
