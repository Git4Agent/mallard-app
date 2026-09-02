import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  APP_AUTH_READY_MESSAGE,
  classifyProjection,
  executeQoderAppProjectionProbe,
} from "./qoder-app-projection-probe.mjs";

const SOURCE_MARKER_NAME = ".mallard-qoder-manual-resume";
const SOURCE_MARKER_CONTENT = "mallard-qoder-manual-resume-v1\n";
const SESSION_ID = "11111111-2222-4333-8444-555555555555";
const FAKE_APP_MAIN = "export {};\n";
const FAKE_APP_MAIN_SHA256 = createHash("sha256")
  .update(FAKE_APP_MAIN)
  .digest("hex");

async function createDirectory(prefix) {
  const created = await mkdtemp(join(tmpdir(), prefix));
  const canonical = await realpath(created);
  await chmod(canonical, 0o700);
  return canonical;
}

async function createSourceFixture() {
  const root = await createDirectory("mallard-qoder-manual-resume-test-");
  await writeFile(join(root, SOURCE_MARKER_NAME), SOURCE_MARKER_CONTENT, {
    mode: 0o600,
  });
  const config = join(root, "qoder-config");
  const bucket = join(config, "projects", "-private-tmp-source-project");
  await mkdir(bucket, { recursive: true, mode: 0o700 });
  const transcript = join(bucket, `${SESSION_ID}.jsonl`);
  await writeFile(
    transcript,
    `${JSON.stringify({ type: "summary", summary: "fixture" })}\n`,
    { mode: 0o600 },
  );
  return { root, config, transcript };
}

async function createAppSourceFixture() {
  const root = await createDirectory("mallard-qoder-app-src-test-");
  await mkdir(join(root, "out", "main"), { recursive: true, mode: 0o700 });
  await writeFile(
    join(root, "package.json"),
    `${JSON.stringify({
      name: "qoder",
      version: "0.1.4",
      type: "module",
      main: "./out/main/index.js",
    })}\n`,
    { mode: 0o600 },
  );
  await writeFile(join(root, "out", "main", "index.js"), FAKE_APP_MAIN, {
    mode: 0o600,
  });
  return root;
}

async function createResourcesFixture() {
  const root = await createDirectory("mallard-qoder-app-resources-test-");
  await mkdir(join(root, "bundled-resources"), { mode: 0o700 });
  await mkdir(join(root, "umid"), { mode: 0o700 });
  await writeFile(
    join(root, "product.json"),
    `${JSON.stringify({
      productId: "qoder",
      displayName: "Qoder",
      electronApplicationName: "Qoder App",
      appId: "com.qoder.app",
      dataDirectoryName: "com.qoder.app.stable",
      packageName: "qoder",
      protocolScheme: "qoder",
      windowsLauncherDirectory: "Qoder\\Qoder Launcher",
      windowsLauncherFileName: "Qoder Launcher.exe",
      desktopUpdateDownloadBaseUrl: "https://download.example.invalid/releases",
    })}\n`,
    { mode: 0o600 },
  );
  return root;
}

async function createFakeElectronFixture() {
  const root = await createDirectory("mallard-qoder-fake-electron-test-");
  const executable = join(root, "fake-electron.mjs");
  await writeFile(
    executable,
    [
      `#!${process.execPath}`,
      'import { execFileSync } from "node:child_process";',
      'import { mkdirSync, symlinkSync } from "node:fs";',
      'import { join } from "node:path";',
      'const data = join(process.env.QODER_E2E_PROFILE_ROOT, "com.qoder.app.demo");',
      "mkdirSync(data, { recursive: true, mode: 0o700 });",
      'symlinkSync("missing-singleton-target", join(data, "SingletonSocket"));',
      'const database = join(data, "main.sqlite");',
      "execFileSync(\"/usr/bin/sqlite3\", [database, \"CREATE TABLE chat_sessions (session_id TEXT PRIMARY KEY); PRAGMA user_version=17;\"]);",
      `process.stderr.write(${JSON.stringify(APP_AUTH_READY_MESSAGE)} + "\\n");`,
      'process.on("SIGTERM", () => process.exit(0));',
      "setInterval(() => {}, 1000);",
      "",
    ].join("\n"),
    { mode: 0o700 },
  );
  await chmod(executable, 0o700);
  return { root, executable };
}

async function removeFixture(path) {
  await rm(path, { recursive: true, force: true });
}

test("classifies a clean App DB without the CLI UUID as a failed projection rebuild", () => {
  assert.deepEqual(
    classifyProjection({
      chatSessionsTableFound: true,
      sourceSessionRows: 0,
    }),
    {
      verdict: "QODER_APP_PROJECTION_REBUILD_DOES_NOT_PASS",
      projectionRebuilt: false,
      appDiscoveredSession: false,
      qoderAppResumeProved: false,
    },
  );
});

test("does not claim App resume when a matching DB projection exists", () => {
  assert.deepEqual(
    classifyProjection({
      chatSessionsTableFound: true,
      sourceSessionRows: 1,
    }),
    {
      verdict: "QODER_APP_DISCOVERY_PASS_CONTINUE_UNPROVED",
      projectionRebuilt: true,
      appDiscoveredSession: true,
      qoderAppResumeProved: false,
    },
  );
});

test("runs a clean-profile projection probe without changing or disclosing the transcript", async () => {
  execFileSync("/usr/bin/sqlite3", ["--version"], { stdio: "ignore" });
  const source = await createSourceFixture();
  const appSource = await createAppSourceFixture();
  const resources = await createResourcesFixture();
  const electron = await createFakeElectronFixture();
  const transcriptBefore = await readFile(source.transcript);
  let appIsolationRoot;
  try {
    const result = await executeQoderAppProjectionProbe(
      {
        electron: electron.executable,
        appSource,
        appResources: resources,
        qoderConfig: source.config,
        timeoutMs: 10_000,
      },
      {
        settleMs: 20,
        expectedAppMainSha256: FAKE_APP_MAIN_SHA256,
        onIsolationRoot: (root) => {
          appIsolationRoot = root;
        },
      },
    );

    assert.equal(result.ok, true);
    assert.equal(
      result.verdict,
      "QODER_APP_PROJECTION_REBUILD_DOES_NOT_PASS",
    );
    assert.equal(result.database.initialized, true);
    assert.equal(result.database.totalSessions, 0);
    assert.equal(result.database.sourceSessionRows, 0);
    assert.equal(result.database.projectionRebuilt, false);
    assert.equal(result.resume.appDiscoveredSession, false);
    assert.equal(result.resume.continuationAttempted, false);
    assert.equal(result.resume.qoderAppResumeProved, false);
    assert.equal(result.launch.fakeAuthInitialized, true);
    assert.equal(result.launch.authFileCreated, false);
    assert.equal(result.launch.chromiumMockKeychainConfigured, true);
    assert.equal(result.launch.nativeSecurityCommandAttempted, false);
    assert.equal(result.cleanup.complete, true);
    assert.deepEqual(await readFile(source.transcript), transcriptBefore);

    const serialized = JSON.stringify(result);
    assert.equal(serialized.includes(SESSION_ID), false);
    assert.equal(serialized.includes(source.root), false);
    assert.equal(serialized.includes(source.config), false);
    assert.equal(serialized.includes(source.transcript), false);
    assert.equal(serialized.includes(appSource), false);
    assert.equal(serialized.includes(resources), false);
    assert.equal(serialized.includes(electron.executable), false);
    assert.equal(serialized.includes(appIsolationRoot), false);
    await assert.rejects(stat(appIsolationRoot), { code: "ENOENT" });
  } finally {
    await Promise.all([
      removeFixture(source.root),
      removeFixture(appSource),
      removeFixture(resources),
      removeFixture(electron.root),
    ]);
  }
});

test("rejects a Qoder config that is not owned by the guarded CLI probe", async () => {
  const unsafeRoot = await createDirectory("qoder-untrusted-source-test-");
  const unsafeConfig = join(unsafeRoot, "qoder-config");
  await mkdir(unsafeConfig, { mode: 0o700 });
  const appSource = await createAppSourceFixture();
  const resources = await createResourcesFixture();
  const electron = await createFakeElectronFixture();
  try {
    const result = await executeQoderAppProjectionProbe(
      {
        electron: electron.executable,
        appSource,
        appResources: resources,
        qoderConfig: unsafeConfig,
        timeoutMs: 10_000,
      },
      { expectedAppMainSha256: FAKE_APP_MAIN_SHA256 },
    );
    assert.equal(result.ok, false);
    assert.equal(result.verdict, "QODER_APP_PROJECTION_PROBE_INCONCLUSIVE");
    assert.equal(result.error.code, "unsafe_source_root");
    assert.equal(result.cleanup.complete, true);
  } finally {
    await Promise.all([
      removeFixture(unsafeRoot),
      removeFixture(appSource),
      removeFixture(resources),
      removeFixture(electron.root),
    ]);
  }
});
