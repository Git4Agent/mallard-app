import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  access,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  QODER_CLI_PACKAGE_NAME,
  QODER_CLI_VERSION,
  assertPinnedQoderCliBundle,
  executeAuthenticatedResumeSmoke,
} from "./qoder-cli-authenticated-resume.mjs";

const TEST_MARKER =
  "MALLARD-QODER-RESUME-0123456789abcdef0123456789abcdef";
const TEST_SESSION_ID = "77777777-7777-4777-8777-777777777777";

async function makeMockCli(t, behavior = {}) {
  const root = await realpath(
    await mkdtemp(join(tmpdir(), "mallard-qoder-auth-resume-mock-")),
  );
  t.after(async () => {
    await rm(root, { recursive: true, force: true });
  });
  const bundleRoot = join(root, "bundle");
  await mkdir(bundleRoot, { mode: 0o700 });
  await writeFile(
    join(root, "package.json"),
    `${JSON.stringify({
      name: QODER_CLI_PACKAGE_NAME,
      version: QODER_CLI_VERSION,
      type: "module",
    })}\n`,
    { mode: 0o600 },
  );
  const script = join(bundleRoot, "qodercli.js");
  const configuration = JSON.stringify({
    attemptSecurity: behavior.attemptSecurity === true,
    loginFails: behavior.loginFails === true,
    mutateBundleAfterSource: behavior.mutateBundleAfterSource === true,
    oversizedSource: behavior.oversizedSource === true,
    wrongResumeMarker: behavior.wrongResumeMarker === true,
  });
  await writeFile(
    script,
    `import { spawnSync } from "node:child_process";
import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

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
function requireTurnControls() {
  for (const flag of [
    "--bare",
    "--print",
    "--strict-mcp-config",
    "--disable-builtin-skills",
  ]) {
    if (!args.includes(flag)) throw new Error("missing " + flag);
  }
  if (args.includes("--no-session-persistence")) {
    throw new Error("persistence disabled");
  }
  if (value("--tools") !== "") throw new Error("tools enabled");
  if (value("--setting-sources") !== "") throw new Error("settings enabled");
  if (value("--permission-mode") !== "dont_ask") {
    throw new Error("wrong permission mode");
  }
  if (value("--max-turns") !== "1") throw new Error("turns not bounded");
  if (value("--max-output-tokens") !== "128") {
    throw new Error("output not bounded");
  }
}

const configRoot = value("--config-dir");
const projectRoot = value("--cwd");
if (process.cwd() !== projectRoot) throw new Error("cwd mismatch");
if (process.env.HOME === undefined || process.env.HOME.includes(".qoder")) {
  throw new Error("home isolation missing");
}
if (process.env.QODER_CONFIG_DIR !== configRoot) {
  throw new Error("config isolation missing");
}
if (process.env.QODER_FORCE_FILE_STORAGE !== "true") {
  throw new Error("file storage control missing");
}
for (const name of [
  "QODER_PERSONAL_ACCESS_TOKEN",
  "TUZI_API_KEY",
  "OPENAI_API_KEY",
  "NODE_OPTIONS",
]) {
  if (process.env[name] !== undefined) {
    throw new Error("credential or injection environment inherited");
  }
}

if (args.at(-1) === "login") {
  if (behavior.attemptSecurity) {
    spawnSync("security", ["default-keychain"], { stdio: "ignore" });
  }
  await mkdir(configRoot, { recursive: true });
  await writeFile(join(configRoot, "mock-login-state"), "isolated\\n");
  process.exitCode = behavior.loginFails ? 19 : 0;
} else {
  requireTurnControls();
  const mcp = JSON.parse(await readFile(value("--mcp-config"), "utf8"));
  const settings = JSON.parse(await readFile(value("--settings"), "utf8"));
  if (Object.keys(mcp.mcpServers).length !== 0) throw new Error("mcp enabled");
  if (settings.general.enableAutoUpdate !== false) {
    throw new Error("auto-update enabled");
  }
  const separator = args.indexOf("--");
  if (separator < 0 || separator + 1 !== args.length - 1) {
    throw new Error("prompt boundary missing");
  }
  const prompt = args.at(-1);
  const projectBucket = join(configRoot, "projects", bucket(projectRoot));
  await mkdir(projectBucket, { recursive: true });

  if (args.includes("--resume")) {
    const sessionId = value("--resume");
    const transcriptPath = join(projectBucket, sessionId + ".jsonl");
    const transcript = await readFile(transcriptPath, "utf8");
    const marker = transcript.match(/MALLARD-QODER-RESUME-[0-9a-f]{32}/)?.[0];
    const sourcePid = Number(transcript.match(/"sourcePid":([0-9]+)/)?.[1]);
    if (!marker || sourcePid === process.pid) throw new Error("not a new process");
    if (prompt.includes(marker)) throw new Error("marker reached resume prompt");
    await appendFile(
      transcriptPath,
      JSON.stringify({ type: "assistant", resumed: true }) + "\\n",
    );
    process.stdout.write(JSON.stringify({
      type: "result",
      subtype: "success",
      is_error: false,
      session_id: sessionId,
      result: behavior.wrongResumeMarker ? "WRONG" : marker,
    }));
  } else {
    if (behavior.oversizedSource) {
      process.on("SIGTERM", () => {});
      process.stdout.write("X".repeat(2 * 1024 * 1024));
      setInterval(() => {}, 1_000);
      await new Promise(() => {});
    }
    const marker = prompt.match(/MALLARD-QODER-RESUME-[0-9a-f]{32}/)?.[0];
    if (!marker) throw new Error("source marker missing");
    const transcriptPath = join(projectBucket, "${TEST_SESSION_ID}.jsonl");
    await writeFile(
      transcriptPath,
      JSON.stringify({ type: "source", sourcePid: process.pid, marker }) + "\\n",
    );
    if (behavior.mutateBundleAfterSource) {
      await appendFile(process.argv[1], Buffer.from([10]));
    }
    process.stdout.write(JSON.stringify({
      type: "result",
      subtype: "success",
      is_error: false,
      session_id: "${TEST_SESSION_ID}",
      result: marker,
    }));
  }
}
`,
    { mode: 0o700 },
  );
  return script;
}

function mockValidation(cliBundle) {
  return async () => {
    const metadata = await lstat(cliBundle);
    const bytes = await readFile(cliBundle);
    return {
      entryPath: cliBundle,
      name: QODER_CLI_PACKAGE_NAME,
      version: QODER_CLI_VERSION,
      bundleSha256: createHash("sha256").update(bytes).digest("hex"),
      identity: {
        dev: metadata.dev,
        ino: metadata.ino,
        size: metadata.size,
        mtimeMs: metadata.mtimeMs,
        ctimeMs: metadata.ctimeMs,
      },
    };
  };
}

async function expectMissing(path) {
  await assert.rejects(access(path), (error) => error.code === "ENOENT");
}

function installParentCredentialCanaries(t) {
  const names = [
    "QODER_PERSONAL_ACCESS_TOKEN",
    "TUZI_API_KEY",
    "OPENAI_API_KEY",
    "NODE_OPTIONS",
  ];
  const previous = Object.fromEntries(names.map((name) => [name, process.env[name]]));
  for (const name of names) {
    process.env[name] = `MALLARD_PARENT_CANARY_${name}`;
  }
  t.after(() => {
    for (const name of names) {
      if (previous[name] === undefined) {
        delete process.env[name];
      } else {
        process.env[name] = previous[name];
      }
    }
  });
}

test("proves native resume with a separate mock process", async (t) => {
  const cliBundle = await makeMockCli(t);
  installParentCredentialCanaries(t);
  let isolationRoot;
  let pinCount = 0;
  const messages = [];
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000 },
    {
      validateCliBundleImpl: mockValidation(cliBundle),
      pinCliBundleImpl: async (checkedCli) => {
        pinCount += 1;
        await assertPinnedQoderCliBundle(checkedCli);
      },
      makeMarkerImpl: () => TEST_MARKER,
      onIsolationRoot: (root) => (isolationRoot = root),
      report: (message) => messages.push(message),
    },
  );

  assert.equal(result.ok, true);
  assert.equal(result.verdict, "AUTHENTICATED_NATIVE_RESUME_PASS");
  assert.equal(result.productionRestoreReady, false);
  assert.equal(
    result.authentication.credentialStorage,
    "isolated_encrypted_file",
  );
  assert.equal(result.authentication.credentialValueReturned, false);
  assert.equal(result.source.markerReturned, true);
  assert.equal(result.resume.sameSessionId, true);
  assert.equal(result.resume.markerRecovered, true);
  assert.equal(result.resume.transcriptChanged, true);
  assert.equal(result.resume.transcriptGrew, true);
  assert.equal(result.isolation.qoderConfiguredForIsolatedHome, true);
  assert.equal(result.isolation.nativeKeychainBypassConfigured, true);
  assert.equal(result.isolation.nativeKeychainCommandAttempted, false);
  assert.equal(result.isolation.runnerReadNormalQoderHome, false);
  assert.equal(result.isolation.outsideFilesystemAccessAudited, false);
  assert.equal(result.isolation.projectStayedEmpty, true);
  assert.equal(result.cleanup.complete, true);
  assert.equal(result.cleanup.kept, false);
  assert.equal(result.cleanup.retained, false);
  assert.equal(messages.length, 3);
  assert.equal(pinCount, 6);
  assert.ok(isolationRoot);
  await expectMissing(isolationRoot);

  const serialized = JSON.stringify(result);
  assert.equal(serialized.includes(TEST_MARKER), false);
  assert.equal(serialized.includes(TEST_SESSION_ID), false);
  assert.equal(serialized.includes(isolationRoot), false);
  assert.equal(serialized.includes(cliBundle), false);
  for (const name of [
    "QODER_PERSONAL_ACCESS_TOKEN",
    "TUZI_API_KEY",
    "OPENAI_API_KEY",
  ]) {
    assert.equal(serialized.includes(process.env[name]), false);
  }
});

test("fails closed when the CLI attempts a native Keychain command", async (t) => {
  const cliBundle = await makeMockCli(t, { attemptSecurity: true });
  let isolationRoot;
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000 },
    {
      validateCliBundleImpl: mockValidation(cliBundle),
      makeMarkerImpl: () => TEST_MARKER,
      onIsolationRoot: (root) => (isolationRoot = root),
    },
  );

  assert.equal(result.ok, false);
  assert.equal(result.error.code, "native_keychain_command_attempted");
  assert.equal(result.cleanup.complete, true);
  assert.equal(result.cleanup.retained, false);
  await expectMissing(isolationRoot);
});

test("keeps only an explicitly requested isolated root", async (t) => {
  const cliBundle = await makeMockCli(t);
  let isolationRoot;
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000, keep: true },
    {
      validateCliBundleImpl: mockValidation(cliBundle),
      makeMarkerImpl: () => TEST_MARKER,
      onIsolationRoot: (root) => (isolationRoot = root),
    },
  );

  assert.equal(result.ok, true);
  assert.equal(result.cleanup.attempted, false);
  assert.equal(result.cleanup.complete, false);
  assert.equal(result.cleanup.kept, true);
  assert.equal(result.cleanup.retained, true);
  assert.equal(result.inspection.root, isolationRoot);
  await access(isolationRoot);
  await rm(isolationRoot, { recursive: true, force: false });
});

test("fails closed and cleans up after a resume marker mismatch", async (t) => {
  const cliBundle = await makeMockCli(t, { wrongResumeMarker: true });
  let isolationRoot;
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000 },
    {
      validateCliBundleImpl: mockValidation(cliBundle),
      makeMarkerImpl: () => TEST_MARKER,
      onIsolationRoot: (root) => (isolationRoot = root),
    },
  );

  assert.equal(result.ok, false);
  assert.equal(result.verdict, "AUTHENTICATED_NATIVE_RESUME_NOT_PROVED");
  assert.equal(result.error.code, "resume_marker_mismatch");
  assert.equal(result.cleanup.complete, true);
  assert.equal(JSON.stringify(result).includes(TEST_MARKER), false);
  await expectMissing(isolationRoot);
});

test("rejects a modified CLI bundle before login", async (t) => {
  const cliBundle = await makeMockCli(t);
  let isolationCreated = false;
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000 },
    { onIsolationRoot: () => (isolationCreated = true) },
  );

  assert.equal(result.ok, false);
  assert.equal(result.error.code, "unexpected_cli_bundle_digest");
  assert.equal(result.cleanup.attempted, false);
  assert.equal(result.cleanup.complete, true);
  assert.equal(isolationCreated, false);
});

test("an interrupt before login starts no CLI child", async (t) => {
  const cliBundle = await makeMockCli(t);
  let isolationRoot;
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000 },
    {
      validateCliBundleImpl: mockValidation(cliBundle),
      pinCliBundleImpl: () => {
        throw new Error("CLI pin must not run after the interrupt");
      },
      onIsolationRoot: (root) => {
        isolationRoot = root;
        process.emit("SIGINT");
      },
    },
  );

  assert.equal(result.ok, false);
  assert.equal(result.error.code, "interrupted");
  assert.equal(result.cleanup.complete, true);
  await expectMissing(isolationRoot);
});

test("detects a bundle change between validation and execution", async (t) => {
  const cliBundle = await makeMockCli(t);
  const metadata = await lstat(cliBundle);
  const bytes = await readFile(cliBundle);
  const checkedCli = {
    entryPath: cliBundle,
    bundleSha256: createHash("sha256").update(bytes).digest("hex"),
    identity: {
      dev: metadata.dev,
      ino: metadata.ino,
      size: metadata.size,
      mtimeMs: metadata.mtimeMs,
      ctimeMs: metadata.ctimeMs,
    },
  };

  await assertPinnedQoderCliBundle(checkedCli);
  await writeFile(cliBundle, `${bytes.toString("utf8")}\n`, { mode: 0o700 });
  await assert.rejects(
    assertPinnedQoderCliBundle(checkedCli),
    (error) => error.code === "cli_bundle_changed",
  );
});

test("fails closed when the CLI bundle changes during execution", async (t) => {
  const cliBundle = await makeMockCli(t, { mutateBundleAfterSource: true });
  let isolationRoot;
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000 },
    {
      validateCliBundleImpl: mockValidation(cliBundle),
      makeMarkerImpl: () => TEST_MARKER,
      onIsolationRoot: (root) => (isolationRoot = root),
    },
  );

  assert.equal(result.ok, false);
  assert.equal(result.error.code, "cli_bundle_changed");
  assert.equal(result.cleanup.complete, true);
  assert.equal(result.cleanup.retained, false);
  await expectMissing(isolationRoot);
});

test("force-kills a CLI that exceeds the output bound", async (t) => {
  const cliBundle = await makeMockCli(t, { oversizedSource: true });
  let isolationRoot;
  const startedAt = Date.now();
  const result = await executeAuthenticatedResumeSmoke(
    { cliBundle, timeoutMs: 30_000 },
    {
      validateCliBundleImpl: mockValidation(cliBundle),
      makeMarkerImpl: () => TEST_MARKER,
      onIsolationRoot: (root) => (isolationRoot = root),
    },
  );

  assert.equal(result.ok, false);
  assert.equal(result.error.code, "source_output_too_large");
  assert.equal(result.cleanup.complete, true);
  assert.ok(Date.now() - startedAt < 10_000);
  await expectMissing(isolationRoot);
});
