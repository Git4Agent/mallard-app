#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  open,
  opendir,
  readFile,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import {
  basename,
  delimiter,
  dirname,
  isAbsolute,
  join,
  resolve,
} from "node:path";
import { pathToFileURL } from "node:url";

import { computeCliProjectBucket } from "./qoder-cli-resume.mjs";

export const QODER_CLI_PACKAGE_NAME = "@qoder-ai/qodercli";
export const QODER_CLI_VERSION = "1.1.40";
export const QODER_CLI_BUNDLE_SHA256 =
  "a5b9d9f116c16c512fd3000a62cb87e6d7e72ff4edbb196e24773834cef13eba";

const PASS_VERDICT = "AUTHENTICATED_NATIVE_RESUME_PASS";
const CROSS_PATH_PASS_VERDICT = "AUTHENTICATED_CROSS_PATH_RESUME_PASS";
const MATERIALIZED_CROSS_PATH_PASS_VERDICT =
  "AUTHENTICATED_MATERIALIZED_CROSS_PATH_RESUME_PASS";
const CLEAN_HOME_TRANSFER_PASS_VERDICT =
  "AUTHENTICATED_CLEAN_HOME_CROSS_PATH_TRANSFER_PASS";
const FAIL_VERDICT = "AUTHENTICATED_NATIVE_RESUME_NOT_PROVED";
const CROSS_PATH_FAIL_VERDICT = "AUTHENTICATED_CROSS_PATH_RESUME_NOT_PROVED";
const MATERIALIZED_CROSS_PATH_FAIL_VERDICT =
  "AUTHENTICATED_MATERIALIZED_CROSS_PATH_RESUME_NOT_PROVED";
const CLEAN_HOME_TRANSFER_FAIL_VERDICT =
  "AUTHENTICATED_CLEAN_HOME_CROSS_PATH_TRANSFER_NOT_PROVED";
const ROOT_PREFIX = "mallard-qoder-manual-resume-";
const ROOT_MARKER_NAME = ".mallard-qoder-manual-resume";
const ROOT_MARKER_CONTENT = "mallard-qoder-manual-resume-v1\n";
const SECURITY_GUARD_LOG_NAME = "security-command-attempted.log";
const DEFAULT_TIMEOUT_MS = 180_000;
const MIN_TIMEOUT_MS = 30_000;
const MAX_TIMEOUT_MS = 900_000;
const MAX_OUTPUT_BYTES = 1024 * 1024;
const MAX_BUNDLE_BYTES = 64 * 1024 * 1024;
const MAX_TRANSCRIPT_BYTES = 64 * 1024 * 1024;
const MAX_MATERIALIZED_MEMBERS = 128;
const MAX_MATERIALIZED_MEMBER_BYTES = 64 * 1024 * 1024;
const MAX_MATERIALIZED_TOTAL_BYTES = 128 * 1024 * 1024;
const SESSION_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

function safeErrorCode(error) {
  if (
    error &&
    typeof error === "object" &&
    typeof error.code === "string" &&
    /^[a-z0-9_]{1,96}$/i.test(error.code)
  ) {
    return error.code.toLowerCase();
  }
  return "unexpected_error";
}

function validateTimeout(value) {
  const timeoutMs = value ?? DEFAULT_TIMEOUT_MS;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < MIN_TIMEOUT_MS ||
    timeoutMs > MAX_TIMEOUT_MS
  ) {
    fail("invalid_timeout");
  }
  return timeoutMs;
}

function validateSessionId(value) {
  if (typeof value !== "string" || !SESSION_ID_PATTERN.test(value)) {
    fail("invalid_source_session_id");
  }
  return value.toLowerCase();
}

async function validateCanonicalRegularFile(filePath, code) {
  if (
    typeof filePath !== "string" ||
    !isAbsolute(filePath) ||
    filePath.includes("\0")
  ) {
    fail(code);
  }
  const canonical = await realpath(filePath).catch(() => fail(code));
  if (canonical !== resolve(filePath)) {
    fail(code);
  }
  const metadata = await lstat(canonical).catch(() => fail(code));
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(code);
  }
  return { canonical, metadata };
}

async function hashBoundedRegularFile(filePath, maximumBytes, code) {
  const before = await lstat(filePath, { bigint: true }).catch(() => fail(code));
  if (
    !before.isFile() ||
    before.isSymbolicLink() ||
    before.size < 1n ||
    before.size > BigInt(maximumBytes)
  ) {
    fail(code);
  }
  const handle = await open(
    filePath,
    fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
  ).catch(() => fail(code));
  try {
    const digest = createHash("sha256");
    const buffer = Buffer.allocUnsafe(64 * 1024);
    let size = 0;
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, null);
      if (bytesRead === 0) {
        break;
      }
      size += bytesRead;
      if (size > maximumBytes) {
        fail(code);
      }
      digest.update(buffer.subarray(0, bytesRead));
    }
    const after = await handle.stat({ bigint: true });
    if (
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      before.mtimeNs !== after.mtimeNs ||
      before.ctimeNs !== after.ctimeNs
    ) {
      fail(code);
    }
    return { size, digest: digest.digest("hex") };
  } finally {
    await handle.close();
  }
}

async function readBoundedRegularFile(filePath, maximumBytes, code) {
  const before = await lstat(filePath, { bigint: true }).catch(() => fail(code));
  if (
    !before.isFile() ||
    before.isSymbolicLink() ||
    before.size < 1n ||
    before.size > BigInt(maximumBytes)
  ) {
    fail(code);
  }
  const handle = await open(
    filePath,
    fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
  ).catch(() => fail(code));
  try {
    const chunks = [];
    const buffer = Buffer.allocUnsafe(64 * 1024);
    let size = 0;
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, null);
      if (bytesRead === 0) {
        break;
      }
      size += bytesRead;
      if (size > maximumBytes) {
        fail(code);
      }
      chunks.push(Buffer.from(buffer.subarray(0, bytesRead)));
    }
    const after = await handle.stat({ bigint: true });
    if (
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      before.mtimeNs !== after.mtimeNs ||
      before.ctimeNs !== after.ctimeNs
    ) {
      fail(code);
    }
    return Buffer.concat(chunks, size);
  } finally {
    await handle.close();
  }
}

async function validateQoderCliBundle(filePath) {
  const { canonical: entryPath, metadata } =
    await validateCanonicalRegularFile(filePath, "invalid_cli_bundle");
  if (
    basename(entryPath) !== "qodercli.js" ||
    basename(dirname(entryPath)) !== "bundle"
  ) {
    fail("unexpected_cli_entrypoint");
  }

  const packagePath = join(dirname(dirname(entryPath)), "package.json");
  const { canonical: checkedPackagePath } = await validateCanonicalRegularFile(
    packagePath,
    "invalid_cli_package",
  );
  const packageBytes = await readBoundedRegularFile(
    checkedPackagePath,
    1024 * 1024,
    "invalid_cli_package",
  );
  let packageDocument;
  try {
    packageDocument = JSON.parse(packageBytes.toString("utf8"));
  } catch {
    fail("invalid_cli_package");
  }
  if (packageDocument?.name !== QODER_CLI_PACKAGE_NAME) {
    fail("unexpected_cli_package");
  }
  if (packageDocument?.version !== QODER_CLI_VERSION) {
    fail("unexpected_cli_version");
  }

  const bundle = await hashBoundedRegularFile(
    entryPath,
    MAX_BUNDLE_BYTES,
    "invalid_cli_bundle",
  );
  if (bundle.digest !== QODER_CLI_BUNDLE_SHA256) {
    fail("unexpected_cli_bundle_digest");
  }
  return {
    entryPath,
    name: QODER_CLI_PACKAGE_NAME,
    version: QODER_CLI_VERSION,
    bundleSha256: bundle.digest,
    identity: {
      dev: metadata.dev,
      ino: metadata.ino,
      size: metadata.size,
      mtimeMs: metadata.mtimeMs,
      ctimeMs: metadata.ctimeMs,
    },
  };
}

export async function assertPinnedQoderCliBundle(checkedCli) {
  const { canonical, metadata } = await validateCanonicalRegularFile(
    checkedCli.entryPath,
    "cli_bundle_changed",
  );
  const expected = checkedCli.identity;
  if (
    canonical !== checkedCli.entryPath ||
    !expected ||
    metadata.dev !== expected.dev ||
    metadata.ino !== expected.ino ||
    metadata.size !== expected.size ||
    metadata.mtimeMs !== expected.mtimeMs ||
    metadata.ctimeMs !== expected.ctimeMs
  ) {
    fail("cli_bundle_changed");
  }
  const current = await hashBoundedRegularFile(
    canonical,
    MAX_BUNDLE_BYTES,
    "cli_bundle_changed",
  );
  if (current.digest !== checkedCli.bundleSha256) {
    fail("cli_bundle_changed");
  }
}

async function validateNode() {
  const major = Number(process.versions.node.split(".")[0]);
  if (!Number.isSafeInteger(major) || major < 20) {
    fail("unsupported_node_version");
  }
  const { canonical } = await validateCanonicalRegularFile(
    process.execPath,
    "invalid_node_path",
  );
  return canonical;
}

async function assertIsolationRoot(root) {
  if (typeof root !== "string" || !isAbsolute(root)) {
    fail("unsafe_isolation_root");
  }
  const canonicalRoot = await realpath(root).catch(() =>
    fail("unsafe_isolation_root"),
  );
  const canonicalTemp = await realpath(tmpdir()).catch(() =>
    fail("unsafe_isolation_root"),
  );
  const metadata = await lstat(canonicalRoot).catch(() =>
    fail("unsafe_isolation_root"),
  );
  if (
    canonicalRoot !== resolve(root) ||
    dirname(canonicalRoot) !== canonicalTemp ||
    !basename(canonicalRoot).startsWith(ROOT_PREFIX) ||
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    (metadata.mode & 0o077) !== 0
  ) {
    fail("unsafe_isolation_root");
  }
  if (
    typeof process.getuid === "function" &&
    metadata.uid !== process.getuid()
  ) {
    fail("unsafe_isolation_root");
  }

  const markerPath = join(canonicalRoot, ROOT_MARKER_NAME);
  const markerMetadata = await lstat(markerPath).catch(() =>
    fail("unsafe_isolation_root"),
  );
  if (
    !markerMetadata.isFile() ||
    markerMetadata.isSymbolicLink() ||
    (markerMetadata.mode & 0o077) !== 0 ||
    (typeof process.getuid === "function" &&
      markerMetadata.uid !== process.getuid())
  ) {
    fail("unsafe_isolation_root");
  }
  if ((await readFile(markerPath, "utf8")) !== ROOT_MARKER_CONTENT) {
    fail("unsafe_isolation_root");
  }
  return canonicalRoot;
}

async function createIsolation(onIsolationRoot) {
  const created = await mkdtemp(join(tmpdir(), ROOT_PREFIX));
  const root = await realpath(created);
  try {
    await chmod(root, 0o700);
    await writeFile(join(root, ROOT_MARKER_NAME), ROOT_MARKER_CONTENT, {
      flag: "wx",
      mode: 0o600,
    });
    const paths = {
      root,
      homeRoot: join(root, "home"),
      configRoot: join(root, "qoder-config"),
      projectRoot: join(root, "empty-project"),
      resumeProjectRoot: join(root, "second-empty-project"),
      tempRoot: join(root, "tmp"),
      runtimeRoot: join(root, "runner"),
    };
    for (const directory of Object.values(paths).slice(1)) {
      await mkdir(directory, { mode: 0o700 });
    }
    await writeRuntimeControls(paths);
    await assertIsolationRoot(root);
    onIsolationRoot?.(root);
    return paths;
  } catch (error) {
    try {
      await rm(root, { recursive: true, force: false });
      if (error && typeof error === "object") {
        error.setupCleanupAttempted = true;
      }
    } catch {
      const cleanupError = new Error("setup_cleanup_failed");
      cleanupError.code = "setup_cleanup_failed";
      cleanupError.retainedIsolationRoot = root;
      cleanupError.setupCleanupAttempted = true;
      throw cleanupError;
    }
    throw error;
  }
}

async function writeRuntimeControls(paths) {
  await writeFile(
    join(paths.runtimeRoot, "settings.json"),
    `${JSON.stringify({ general: { enableAutoUpdate: false } })}\n`,
    { flag: "wx", mode: 0o600 },
  );
  await writeFile(
    join(paths.runtimeRoot, "mcp.json"),
    `${JSON.stringify({ mcpServers: {} })}\n`,
    { flag: "wx", mode: 0o600 },
  );
  await writeFile(
    join(paths.runtimeRoot, "security"),
    [
      "#!/bin/sh",
      "umask 077",
      'printf "%s\\n" blocked > "$MALLARD_QODER_SECURITY_GUARD_LOG"',
      "exit 77",
      "",
    ].join("\n"),
    { flag: "wx", mode: 0o700 },
  );
}

async function createTransferMachine(root, machineName, pathName, projectName) {
  const machineRoot = join(root, machineName);
  const paths = {
    machineRoot,
    homeRoot: join(machineRoot, "home"),
    configRoot: join(machineRoot, "qoder-config"),
    projectRoot: join(machineRoot, pathName, projectName),
    tempRoot: join(machineRoot, "tmp"),
    runtimeRoot: join(machineRoot, "runner"),
  };
  for (const directory of Object.values(paths)) {
    await mkdir(directory, { recursive: true, mode: 0o700 });
  }
  await writeRuntimeControls(paths);
  return paths;
}

async function createCleanHomeTransferIsolation(onIsolationRoot) {
  const created = await mkdtemp(join(tmpdir(), ROOT_PREFIX));
  const root = await realpath(created);
  try {
    await chmod(root, 0o700);
    await writeFile(join(root, ROOT_MARKER_NAME), ROOT_MARKER_CONTENT, {
      flag: "wx",
      mode: 0o600,
    });
    const machineA = await createTransferMachine(
      root,
      "machine-a",
      "A",
      "project_a",
    );
    const machineB = await createTransferMachine(
      root,
      "machine-b",
      "B",
      "project_B",
    );
    await assertIsolationRoot(root);
    onIsolationRoot?.(root);
    return {
      root,
      machineA,
      machineB,
      portableRoot: join(root, "portable-session-bundle"),
    };
  } catch (error) {
    try {
      await rm(root, { recursive: true, force: false });
      if (error && typeof error === "object") {
        error.setupCleanupAttempted = true;
      }
    } catch {
      const cleanupError = new Error("setup_cleanup_failed");
      cleanupError.code = "setup_cleanup_failed";
      cleanupError.retainedIsolationRoot = root;
      cleanupError.setupCleanupAttempted = true;
      throw cleanupError;
    }
    throw error;
  }
}

async function removeIsolation(root) {
  const canonicalRoot = await assertIsolationRoot(root);
  await rm(canonicalRoot, { recursive: true, force: false });
}

function safeEnvironmentValue(value, pattern, fallback) {
  return typeof value === "string" && pattern.test(value) ? value : fallback;
}

function addSafeProxyEnvironment(environment) {
  for (const name of [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
  ]) {
    const value = process.env[name];
    if (!value) {
      continue;
    }
    try {
      const parsed = new URL(value);
      if (!parsed.username && !parsed.password) {
        environment[name] = value;
      }
    } catch {
      // Ignore malformed parent proxy configuration.
    }
  }
  for (const name of ["NO_PROXY", "no_proxy"]) {
    const value = process.env[name];
    if (typeof value === "string" && value.length <= 4096) {
      environment[name] = value;
    }
  }
}

function buildCleanEnvironment(paths, nodePath) {
  const environment = {
    HOME: paths.homeRoot,
    USER: "mallard-qoder-probe",
    LOGNAME: "mallard-qoder-probe",
    SHELL: "/bin/sh",
    PATH: [paths.runtimeRoot, dirname(nodePath), "/usr/bin", "/bin"].join(
      delimiter,
    ),
    TMPDIR: paths.tempRoot,
    TMP: paths.tempRoot,
    TEMP: paths.tempRoot,
    XDG_CACHE_HOME: join(paths.homeRoot, ".cache"),
    XDG_CONFIG_HOME: join(paths.homeRoot, ".config"),
    XDG_DATA_HOME: join(paths.homeRoot, ".local", "share"),
    XDG_STATE_HOME: join(paths.homeRoot, ".local", "state"),
    QODER_CONFIG_DIR: paths.configRoot,
    QODER_CLI_HOME: paths.configRoot,
    QODER_FORCE_FILE_STORAGE: "true",
    MALLARD_QODER_SECURITY_GUARD_LOG: join(
      paths.runtimeRoot,
      SECURITY_GUARD_LOG_NAME,
    ),
    NO_BROWSER: "1",
    NO_COLOR: "1",
    TERM: safeEnvironmentValue(
      process.env.TERM,
      /^[a-zA-Z0-9._+-]{1,64}$/,
      "xterm-256color",
    ),
    LANG: safeEnvironmentValue(
      process.env.LANG,
      /^[a-zA-Z0-9._@-]{1,64}$/,
      "en_US.UTF-8",
    ),
  };
  for (const name of ["SSL_CERT_FILE", "SSL_CERT_DIR", "NODE_EXTRA_CA_CERTS"]) {
    const value = process.env[name];
    if (
      typeof value === "string" &&
      isAbsolute(value) &&
      !value.includes("\0")
    ) {
      environment[name] = value;
    }
  }
  addSafeProxyEnvironment(environment);
  return Object.freeze(environment);
}

async function assertNoSecurityCommandAttempt(paths) {
  const guardLog = join(paths.runtimeRoot, SECURITY_GUARD_LOG_NAME);
  const content = await readFile(guardLog).catch((error) => {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  });
  if (content !== null) {
    fail("native_keychain_command_attempted");
  }
}

function killChild(child, signal) {
  try {
    if (process.platform !== "win32" && child.pid) {
      process.kill(-child.pid, signal);
    } else {
      child.kill(signal);
    }
  } catch {
    child.kill(signal);
  }
}

function createChildStopper(child) {
  let forceTimer;
  return {
    stop() {
      if (forceTimer) {
        return;
      }
      killChild(child, "SIGTERM");
      forceTimer = setTimeout(() => killChild(child, "SIGKILL"), 2_000);
    },
    clear() {
      clearTimeout(forceTimer);
      forceTimer = undefined;
    },
  };
}

function waitForChild(child, { timeoutMs, abortSignal, stopper }) {
  return new Promise((complete) => {
    let settled = false;
    let timedOut = false;
    let aborted = false;
    const finish = (value) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timeoutTimer);
      stopper.clear();
      abortSignal?.removeEventListener("abort", onAbort);
      complete({ ...value, timedOut, aborted });
    };
    const stop = (reason) => {
      if (settled) {
        return;
      }
      if (reason === "timeout") {
        timedOut = true;
      } else {
        aborted = true;
      }
      stopper.stop();
    };
    const onAbort = () => stop("abort");
    const timeoutTimer = setTimeout(() => stop("timeout"), timeoutMs);
    abortSignal?.addEventListener("abort", onAbort, { once: true });
    child.once("error", () =>
      finish({ exitCode: null, signal: null, spawnFailed: true }),
    );
    child.once("close", (exitCode, signal) =>
      finish({ exitCode, signal, spawnFailed: false }),
    );
    if (abortSignal?.aborted) {
      onAbort();
    }
  });
}

function abortedOutcome() {
  return {
    exitCode: null,
    signal: null,
    spawnFailed: false,
    timedOut: false,
    aborted: true,
  };
}

async function runInteractiveCommand({
  nodePath,
  cliBundle,
  args,
  cwd,
  environment,
  timeoutMs,
  abortSignal,
}) {
  if (abortSignal?.aborted) {
    return abortedOutcome();
  }
  const child = spawn(nodePath, [cliBundle, ...args], {
    cwd,
    env: environment,
    stdio: "inherit",
    shell: false,
    detached: process.platform !== "win32",
  });
  const stopper = createChildStopper(child);
  return waitForChild(child, { timeoutMs, abortSignal, stopper });
}

function appendBounded(chunks, chunk, state, stopper) {
  const bytes = Buffer.from(chunk);
  state.bytes += bytes.length;
  if (state.bytes > MAX_OUTPUT_BYTES) {
    state.exceeded = true;
    stopper.stop();
    return;
  }
  chunks.push(bytes);
}

async function runCapturedCommand({
  nodePath,
  cliBundle,
  args,
  cwd,
  environment,
  timeoutMs,
  abortSignal,
}) {
  if (abortSignal?.aborted) {
    return {
      ...abortedOutcome(),
      outputExceeded: false,
      stdout: Buffer.alloc(0),
      stderr: Buffer.alloc(0),
    };
  }
  const child = spawn(nodePath, [cliBundle, ...args], {
    cwd,
    env: environment,
    stdio: ["ignore", "pipe", "pipe"],
    shell: false,
    detached: process.platform !== "win32",
  });
  const stdoutChunks = [];
  const stderrChunks = [];
  const stdoutState = { bytes: 0, exceeded: false };
  const stderrState = { bytes: 0, exceeded: false };
  const stopper = createChildStopper(child);
  child.stdout.on("data", (chunk) =>
    appendBounded(stdoutChunks, chunk, stdoutState, stopper),
  );
  child.stderr.on("data", (chunk) =>
    appendBounded(stderrChunks, chunk, stderrState, stopper),
  );
  const outcome = await waitForChild(child, {
    timeoutMs,
    abortSignal,
    stopper,
  });
  return {
    ...outcome,
    outputExceeded: stdoutState.exceeded || stderrState.exceeded,
    stdout: Buffer.concat(stdoutChunks),
    stderr: Buffer.concat(stderrChunks),
  };
}

function requireSuccessfulCommand(outcome, label) {
  if (outcome.aborted) {
    fail("interrupted");
  }
  if (outcome.timedOut) {
    fail(`${label}_timeout`);
  }
  if (outcome.spawnFailed) {
    fail(`${label}_spawn_failed`);
  }
  if (outcome.outputExceeded) {
    fail(`${label}_output_too_large`);
  }
  if (outcome.exitCode !== 0) {
    fail(`${label}_failed`);
  }
}

function parseSuccessfulCliResult(outcome, label) {
  requireSuccessfulCommand(outcome, label);
  let document;
  try {
    document = JSON.parse(outcome.stdout.toString("utf8").trim());
  } catch {
    fail(`${label}_invalid_json`);
  }
  if (
    !document ||
    document.type !== "result" ||
    document.subtype !== "success" ||
    document.is_error === true ||
    typeof document.result !== "string"
  ) {
    fail(`${label}_unsuccessful_result`);
  }
  return document;
}

function makeMarker() {
  return `MALLARD-QODER-RESUME-${randomBytes(16).toString("hex")}`;
}

function sourcePrompt(marker) {
  return [
    "Remember this exact random marker for the next session.",
    "Reply with only the marker, without quotes or Markdown:",
    marker,
  ].join(" ");
}

function resumePrompt() {
  return [
    "What exact random marker did I ask you to remember in the previous turn?",
    "Reply with only that marker, without quotes or Markdown.",
    "Do not inspect files.",
  ].join(" ");
}

function commonTurnArguments(paths, projectRoot = paths.projectRoot) {
  return [
    "--bare",
    "--config-dir",
    paths.configRoot,
    "--cwd",
    projectRoot,
    "--setting-sources",
    "",
    "--settings",
    join(paths.runtimeRoot, "settings.json"),
    "--print",
    "--output-format",
    "json",
    "--tools",
    "",
    "--permission-mode",
    "dont_ask",
    "--strict-mcp-config",
    "--mcp-config",
    join(paths.runtimeRoot, "mcp.json"),
    "--max-turns",
    "1",
    "--max-output-tokens",
    "128",
    "--disable-builtin-skills",
  ];
}

async function snapshotTranscript(configRoot, projectRoot, sessionId) {
  const projectBucket = computeCliProjectBucket(projectRoot);
  const transcriptPath = join(
    configRoot,
    "projects",
    projectBucket,
    `${sessionId}.jsonl`,
  );
  const snapshot = await hashBoundedRegularFile(
    transcriptPath,
    MAX_TRANSCRIPT_BYTES,
    "invalid_session_transcript",
  );
  return { ...snapshot, transcriptPath };
}

async function snapshotTranscriptIfPresent(configRoot, projectRoot, sessionId) {
  const projectBucket = computeCliProjectBucket(projectRoot);
  const transcriptPath = join(
    configRoot,
    "projects",
    projectBucket,
    `${sessionId}.jsonl`,
  );
  const metadata = await lstat(transcriptPath).catch((error) => {
    if (error?.code === "ENOENT") {
      return null;
    }
    fail("invalid_session_transcript");
  });
  if (metadata === null) {
    return null;
  }
  const snapshot = await hashBoundedRegularFile(
    transcriptPath,
    MAX_TRANSCRIPT_BYTES,
    "invalid_session_transcript",
  );
  return { ...snapshot, transcriptPath };
}

async function pathExists(entryPath) {
  try {
    await lstat(entryPath);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

async function findOnlySessionTranscript(bucketPath) {
  const directory = await opendir(bucketPath).catch(() =>
    fail("source_project_bucket_missing"),
  );
  const candidates = [];
  for await (const entry of directory) {
    if (!entry.name.endsWith(".jsonl")) {
      continue;
    }
    const sessionId = entry.name.slice(0, -".jsonl".length);
    if (!SESSION_ID_PATTERN.test(sessionId)) {
      continue;
    }
    const transcriptPath = join(bucketPath, entry.name);
    const metadata = await lstat(transcriptPath).catch(() =>
      fail("invalid_session_transcript"),
    );
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      fail("invalid_session_transcript");
    }
    candidates.push({
      sessionId: sessionId.toLowerCase(),
      transcriptPath,
    });
  }
  if (candidates.length !== 1) {
    fail(
      candidates.length === 0
        ? "source_transcript_not_found"
        : "source_transcript_ambiguous",
    );
  }
  return candidates[0];
}

function findMarkerInValue(value, markers) {
  if (typeof value === "string") {
    for (const match of value.matchAll(/MALLARD-QODER-RESUME-[0-9a-f]{32}/g)) {
      markers.add(match[0]);
    }
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      findMarkerInValue(item, markers);
    }
    return;
  }
  if (value && typeof value === "object") {
    for (const item of Object.values(value)) {
      findMarkerInValue(item, markers);
    }
  }
}

async function readSourceMarker(transcriptPath) {
  const bytes = await readBoundedRegularFile(
    transcriptPath,
    MAX_TRANSCRIPT_BYTES,
    "invalid_session_transcript",
  );
  const markers = new Set();
  for (const line of bytes.toString("utf8").split(/\r?\n/)) {
    if (!line) {
      continue;
    }
    try {
      findMarkerInValue(JSON.parse(line), markers);
    } catch {
      fail("invalid_session_transcript");
    }
  }
  if (markers.size !== 1) {
    fail("source_marker_not_found");
  }
  return [...markers][0];
}

async function snapshotRegularTree(root) {
  const members = [];
  const pending = [{ absolute: root, relative: "" }];
  let totalBytes = 0;
  while (pending.length > 0) {
    const current = pending.pop();
    const directory = await opendir(current.absolute).catch(() =>
      fail("invalid_source_project_bucket"),
    );
    for await (const entry of directory) {
      if (members.length >= MAX_MATERIALIZED_MEMBERS) {
        fail("materialized_member_limit_exceeded");
      }
      const absolute = join(current.absolute, entry.name);
      const relative = current.relative
        ? join(current.relative, entry.name)
        : entry.name;
      const metadata = await lstat(absolute).catch(() =>
        fail("invalid_source_project_bucket"),
      );
      if (metadata.isSymbolicLink()) {
        fail("source_project_bucket_contains_symlink");
      }
      if (metadata.isDirectory()) {
        members.push({ kind: "directory", relative, mode: metadata.mode & 0o777 });
        pending.push({ absolute, relative });
        continue;
      }
      if (!metadata.isFile()) {
        fail("source_project_bucket_contains_special_file");
      }
      const snapshot = await hashBoundedRegularFile(
        absolute,
        MAX_MATERIALIZED_MEMBER_BYTES,
        "invalid_source_project_member",
      );
      totalBytes += snapshot.size;
      if (totalBytes > MAX_MATERIALIZED_TOTAL_BYTES) {
        fail("materialized_size_limit_exceeded");
      }
      members.push({
        kind: "file",
        relative,
        mode: metadata.mode & 0o777,
        size: snapshot.size,
        digest: snapshot.digest,
      });
    }
  }
  return { members, totalBytes };
}

async function verifyRegularTree(root, expected, allowTranscriptGrowth = false) {
  const actual = await snapshotRegularTree(root);
  if (actual.members.length !== expected.members.length) {
    fail("materialized_tree_changed");
  }
  const expectedByPath = new Map(
    expected.members.map((member) => [`${member.kind}:${member.relative}`, member]),
  );
  for (const member of actual.members) {
    const expectedMember = expectedByPath.get(`${member.kind}:${member.relative}`);
    if (!expectedMember) {
      fail("materialized_tree_changed");
    }
    if (member.kind === "file") {
      const transcript = member.relative.endsWith(".jsonl");
      if (
        allowTranscriptGrowth &&
        transcript &&
        member.size > expectedMember.size &&
        member.digest !== expectedMember.digest
      ) {
        continue;
      }
      if (
        member.size !== expectedMember.size ||
        member.digest !== expectedMember.digest
      ) {
        fail("materialized_tree_changed");
      }
    }
  }
  return actual;
}

async function copyRegularTree(sourceRoot, destinationRoot, snapshot) {
  await mkdir(destinationRoot, { mode: 0o700 });
  const directories = snapshot.members
    .filter((member) => member.kind === "directory")
    .sort((left, right) => left.relative.length - right.relative.length);
  for (const member of directories) {
    const destination = join(destinationRoot, member.relative);
    await mkdir(destination, { mode: member.mode & 0o700 });
    await chmod(destination, member.mode & 0o700);
  }
  for (const member of snapshot.members) {
    if (member.kind !== "file") {
      continue;
    }
    const source = join(sourceRoot, member.relative);
    const destination = join(destinationRoot, member.relative);
    await copyFile(source, destination, fsConstants.COPYFILE_EXCL);
    await chmod(destination, member.mode & 0o600);
  }
  await verifyRegularTree(destinationRoot, snapshot);
}

async function requireEmptyProject(projectRoot) {
  const directory = await opendir(projectRoot).catch(() =>
    fail("invalid_test_project"),
  );
  for await (const _entry of directory) {
    fail("test_project_not_empty");
  }
}

async function requireEmptyDirectory(directoryPath, code) {
  const directory = await opendir(directoryPath).catch(() => fail(code));
  for await (const _entry of directory) {
    fail(code);
  }
}

function assertSessionOnlySnapshot(snapshot, sessionId) {
  const transcriptName = `${sessionId}.jsonl`;
  let transcriptFound = false;
  for (const member of snapshot.members) {
    if (member.relative === transcriptName) {
      if (member.kind !== "file" || transcriptFound) {
        fail("invalid_portable_session_group");
      }
      transcriptFound = true;
      continue;
    }
    if (
      member.relative === sessionId ||
      member.relative.startsWith(`${sessionId}/`)
    ) {
      continue;
    }
    fail("portable_bundle_contains_non_session_state");
  }
  if (!transcriptFound) {
    fail("portable_bundle_missing_transcript");
  }
}

function baseResult(ok, crossPath = false) {
  return {
    schemaVersion: 1,
    ok,
    verdict: ok
      ? crossPath
        ? CROSS_PATH_PASS_VERDICT
        : PASS_VERDICT
      : crossPath
        ? CROSS_PATH_FAIL_VERDICT
        : FAIL_VERDICT,
    productionRestoreReady: false,
  };
}

export async function executeAuthenticatedResumeSmoke(
  { cliBundle, timeoutMs, keep = false, crossPath = false } = {},
  {
    validateCliBundleImpl = validateQoderCliBundle,
    pinCliBundleImpl = assertPinnedQoderCliBundle,
    makeMarkerImpl = makeMarker,
    onIsolationRoot,
    report = () => {},
  } = {},
) {
  let checkedCli;
  let nodePath;
  let checkedTimeout;
  try {
    checkedCli = await validateCliBundleImpl(cliBundle);
    nodePath = await validateNode();
    checkedTimeout = validateTimeout(timeoutMs);
    if (typeof keep !== "boolean") {
      fail("invalid_keep_value");
    }
    if (typeof crossPath !== "boolean") {
      fail("invalid_cross_path_value");
    }
  } catch (error) {
    return {
      ...baseResult(false, crossPath === true),
      error: { code: safeErrorCode(error) },
      cleanup: {
        attempted: false,
        complete: true,
        kept: false,
        retained: false,
      },
    };
  }

  let paths;
  let sessionId;
  let cleanupComplete = false;
  let result;
  let failureCode;
  let setupRetainedRoot;
  let setupCleanupAttempted = false;
  const abortController = new AbortController();
  const onSignal = () => abortController.abort();
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  try {
    paths = await createIsolation(onIsolationRoot);
    const environment = buildCleanEnvironment(paths, nodePath);
    await requireEmptyProject(paths.projectRoot);
    await requireEmptyProject(paths.resumeProjectRoot);

    report("Authenticate Qoder in the isolated login flow.");
    if (abortController.signal.aborted) {
      fail("interrupted");
    }
    await pinCliBundleImpl(checkedCli);
    const login = await runInteractiveCommand({
      nodePath,
      cliBundle: checkedCli.entryPath,
      args: [
        "--bare",
        "--config-dir",
        paths.configRoot,
        "--cwd",
        paths.projectRoot,
        "login",
      ],
      cwd: paths.projectRoot,
      environment,
      timeoutMs: checkedTimeout,
      abortSignal: abortController.signal,
    });
    await pinCliBundleImpl(checkedCli);
    await assertNoSecurityCommandAttempt(paths);
    requireSuccessfulCommand(login, "login");
    await requireEmptyProject(paths.projectRoot);

    const marker = makeMarkerImpl();
    if (
      typeof marker !== "string" ||
      !/^MALLARD-QODER-RESUME-[0-9a-f]{32}$/.test(marker)
    ) {
      fail("invalid_marker");
    }

    report("Create one persistent marker session.");
    const sourceArguments = [
      ...commonTurnArguments(paths),
      "--name",
      "Mallard manual resume probe",
      "--",
      sourcePrompt(marker),
    ];
    if (abortController.signal.aborted) {
      fail("interrupted");
    }
    await pinCliBundleImpl(checkedCli);
    const sourceOutcome = await runCapturedCommand({
      nodePath,
      cliBundle: checkedCli.entryPath,
      args: sourceArguments,
      cwd: paths.projectRoot,
      environment,
      timeoutMs: checkedTimeout,
      abortSignal: abortController.signal,
    });
    await pinCliBundleImpl(checkedCli);
    await assertNoSecurityCommandAttempt(paths);
    const sourceResult = parseSuccessfulCliResult(sourceOutcome, "source");
    sessionId = validateSessionId(sourceResult.session_id);
    if (sourceResult.result.trim() !== marker) {
      fail("source_marker_mismatch");
    }
    const sourceTranscript = await snapshotTranscript(
      paths.configRoot,
      paths.projectRoot,
      sessionId,
    );
    if (sourceTranscript.size === 0) {
      fail("empty_source_transcript");
    }
    await requireEmptyProject(paths.projectRoot);

    const resumeProjectRoot = crossPath
      ? paths.resumeProjectRoot
      : paths.projectRoot;
    const secondPrompt = resumePrompt();
    const resumeArguments = [
      ...commonTurnArguments(paths, resumeProjectRoot),
      "--resume",
      sessionId,
      "--",
      secondPrompt,
    ];
    if (
      secondPrompt.includes(marker) ||
      resumeArguments.some((argument) => argument.includes(marker)) ||
      Object.values(environment).some((value) => value.includes(marker))
    ) {
      fail("resume_marker_channel_detected");
    }

    report("Resume the exact session in a second Qoder process.");
    if (abortController.signal.aborted) {
      fail("interrupted");
    }
    await pinCliBundleImpl(checkedCli);
    const resumeOutcome = await runCapturedCommand({
      nodePath,
      cliBundle: checkedCli.entryPath,
      args: resumeArguments,
      cwd: resumeProjectRoot,
      environment,
      timeoutMs: checkedTimeout,
      abortSignal: abortController.signal,
    });
    await pinCliBundleImpl(checkedCli);
    await assertNoSecurityCommandAttempt(paths);
    const resumeResult = parseSuccessfulCliResult(resumeOutcome, "resume");
    const resumedSessionId = validateSessionId(resumeResult.session_id);
    if (resumedSessionId !== sessionId) {
      fail("resume_session_id_mismatch");
    }
    if (resumeResult.result.trim() !== marker) {
      fail("resume_marker_mismatch");
    }

    const sourceBucketAfterResume = await snapshotTranscript(
      paths.configRoot,
      paths.projectRoot,
      sessionId,
    );
    const resumeBucketAfterResume = crossPath
      ? await snapshotTranscriptIfPresent(
          paths.configRoot,
          resumeProjectRoot,
          sessionId,
        )
      : sourceBucketAfterResume;
    const sourceBucketGrew =
      sourceBucketAfterResume.size > sourceTranscript.size &&
      sourceBucketAfterResume.digest !== sourceTranscript.digest;
    const resumeBucketGrew =
      resumeBucketAfterResume !== null &&
      resumeBucketAfterResume.size > sourceTranscript.size &&
      resumeBucketAfterResume.digest !== sourceTranscript.digest;
    if (!sourceBucketGrew && !resumeBucketGrew) {
      fail("resume_transcript_did_not_grow");
    }
    if (
      !crossPath &&
      sourceBucketAfterResume.transcriptPath !== sourceTranscript.transcriptPath
    ) {
      fail("resume_transcript_path_changed");
    }
    await requireEmptyProject(paths.projectRoot);
    await requireEmptyProject(paths.resumeProjectRoot);

    const transcriptStorage = sourceBucketGrew
      ? resumeBucketGrew &&
        resumeBucketAfterResume?.transcriptPath !==
          sourceBucketAfterResume.transcriptPath
        ? "both_project_buckets"
        : "source_project_bucket"
      : "resume_project_bucket";

    result = {
      ...baseResult(true, crossPath),
      cli: {
        name: checkedCli.name,
        version: checkedCli.version,
        bundleSha256: checkedCli.bundleSha256,
      },
      authentication: {
        method: "interactive_isolated_profile",
        credentialStorage: "isolated_encrypted_file",
        credentialValueReturned: false,
      },
      source: {
        exitZero: true,
        structuredResult: true,
        markerReturned: true,
        transcriptFound: true,
      },
      resume: {
        exitZero: true,
        sameSessionId: true,
        markerRecovered: true,
        transcriptChanged: true,
        transcriptGrew: true,
        processWorkingDirectoryChanged: crossPath,
        cliWorkingDirectoryChanged: crossPath,
        transcriptStorage,
      },
      isolation: {
        qoderConfiguredForIsolatedHome: true,
        nativeKeychainBypassConfigured: true,
        nativeKeychainCommandAttempted: false,
        runnerReadNormalQoderHome: false,
        outsideFilesystemAccessAudited: false,
        exactWorkingDirectoryReused: !crossPath,
        distinctWorkingDirectoriesUsed: crossPath,
        projectStayedEmpty: true,
        toolsEnabled: false,
        mcpServersEnabled: false,
        builtInSkillsEnabled: false,
      },
      limits: {
        mallardRestoreProved: false,
        sidecarConsumptionProved: false,
        activeLeafAuthorityProved: false,
        crossPathResumeProved: crossPath,
        qoderAppResumeProved: false,
        tuziProviderProved: false,
      },
    };

    const serialized = JSON.stringify(result);
    if (
      serialized.includes(marker) ||
      serialized.includes(sessionId) ||
      serialized.includes(paths.root) ||
      serialized.includes(checkedCli.entryPath)
    ) {
      fail("result_sanitization_failed");
    }
  } catch (error) {
    failureCode = safeErrorCode(error);
    if (
      error &&
      typeof error === "object" &&
      typeof error.retainedIsolationRoot === "string"
    ) {
      setupRetainedRoot = error.retainedIsolationRoot;
    }
    setupCleanupAttempted = error?.setupCleanupAttempted === true;
  } finally {
    process.removeListener("SIGINT", onSignal);
    process.removeListener("SIGTERM", onSignal);
    if (paths && !keep) {
      try {
        await removeIsolation(paths.root);
        cleanupComplete = true;
      } catch {
        failureCode ??= "cleanup_failed";
      }
    }
  }

  if (failureCode || !result) {
    const retainedRoot = paths?.root ?? setupRetainedRoot;
    const retained = Boolean(retainedRoot) && (keep || !cleanupComplete);
    return {
      ...baseResult(false, crossPath),
      error: { code: failureCode ?? "unexpected_error" },
      cleanup: {
        attempted:
          (Boolean(paths) && !keep) || setupCleanupAttempted,
        complete: paths ? cleanupComplete : !setupRetainedRoot,
        kept: Boolean(paths) && keep,
        retained,
      },
      ...(retained ? { inspection: { root: retainedRoot } } : {}),
    };
  }

  result.cleanup = {
    attempted: !keep,
    complete: keep ? false : cleanupComplete,
    kept: keep,
    retained: keep,
  };
  if (keep) {
    result.inspection = { root: paths.root };
  } else if (!cleanupComplete) {
    result.ok = false;
    result.verdict = crossPath ? CROSS_PATH_FAIL_VERDICT : FAIL_VERDICT;
    result.error = { code: "cleanup_failed" };
  }
  return result;
}

function materializedBaseResult(ok) {
  return {
    schemaVersion: 1,
    ok,
    verdict: ok
      ? MATERIALIZED_CROSS_PATH_PASS_VERDICT
      : MATERIALIZED_CROSS_PATH_FAIL_VERDICT,
    productionRestoreReady: false,
  };
}

async function requireExactDirectory(root, relative, code) {
  const expected = join(root, relative);
  const canonical = await realpath(expected).catch(() => fail(code));
  const metadata = await lstat(canonical).catch(() => fail(code));
  if (
    canonical !== expected ||
    !metadata.isDirectory() ||
    metadata.isSymbolicLink()
  ) {
    fail(code);
  }
  return canonical;
}

function makeMaterializedSuccessResult({
  checkedCli,
  root,
  memberCount,
  memberCountAfterResume,
  verificationSource,
}) {
  return {
    ...materializedBaseResult(true),
    cli: {
      name: checkedCli.name,
      version: checkedCli.version,
      bundleSha256: checkedCli.bundleSha256,
    },
    source: {
      nativeCliTranscriptFound: true,
      sourceBackupUnchanged: true,
      sourceSessionUuidPreserved: true,
    },
    materialization: {
      sourceActiveBucketMovedToBackup: true,
      sourceBackupRetained: true,
      destinationBucketCreated: true,
      memberCount,
      memberCountAfterResume,
      byteExactBeforeResume: true,
      structuralPathMappingApplied: false,
    },
    resume: {
      exitZero: true,
      sameSessionId: true,
      markerRecovered: true,
      transcriptChanged: true,
      transcriptGrew: true,
      processWorkingDirectoryChanged: true,
      cliWorkingDirectoryChanged: true,
      transcriptStorage: "destination_project_bucket",
      verificationSource,
    },
    isolation: {
      qoderConfiguredForIsolatedHome: true,
      nativeKeychainBypassConfigured: true,
      nativeKeychainCommandAttempted: false,
      parentCredentialEnvironmentInherited: false,
      exactWorkingDirectoryReused: false,
      distinctWorkingDirectoriesUsed: true,
      projectStayedEmpty: true,
      toolsEnabled: false,
      mcpServersEnabled: false,
      builtInSkillsEnabled: false,
    },
    limits: {
      directCrossPathLookupProved: false,
      pathBucketMaterializationRequired: true,
      structuralPathMappingProved: false,
      mallardRestoreProved: false,
      sidecarConsumptionProved: false,
      activeLeafAuthorityProved: false,
      crossPathResumeProved: true,
      qoderAppResumeProved: false,
    },
    cleanup: {
      attempted: false,
      complete: false,
      kept: true,
      retained: true,
    },
    inspection: { root },
  };
}

function valueContainsExactMarker(value, marker) {
  if (typeof value === "string") {
    return value.trim() === marker;
  }
  if (Array.isArray(value)) {
    return value.some((item) => valueContainsExactMarker(item, marker));
  }
  if (value && typeof value === "object") {
    return Object.values(value).some((item) =>
      valueContainsExactMarker(item, marker),
    );
  }
  return false;
}

async function verifyMaterializedResumeState({
  backupBucket,
  destinationBucket,
  destinationProjectRoot,
}) {
  const sourceTranscript = await findOnlySessionTranscript(backupBucket);
  const sessionId = validateSessionId(sourceTranscript.sessionId);
  const marker = await readSourceMarker(sourceTranscript.transcriptPath);
  const sourceSnapshot = await snapshotRegularTree(backupBucket);
  const destinationSnapshot = await snapshotRegularTree(destinationBucket);
  const destinationByPath = new Map(
    destinationSnapshot.members.map((member) => [
      `${member.kind}:${member.relative}`,
      member,
    ]),
  );
  for (const sourceMember of sourceSnapshot.members) {
    const destinationMember = destinationByPath.get(
      `${sourceMember.kind}:${sourceMember.relative}`,
    );
    if (!destinationMember) {
      fail("materialized_tree_changed");
    }
    if (
      sourceMember.kind === "file" &&
      !sourceMember.relative.endsWith(".jsonl") &&
      (sourceMember.size !== destinationMember.size ||
        sourceMember.digest !== destinationMember.digest)
    ) {
      fail("materialized_tree_changed");
    }
  }

  const destinationTranscriptPath = join(
    destinationBucket,
    `${sessionId}.jsonl`,
  );
  const [sourceBytes, destinationBytes] = await Promise.all([
    readBoundedRegularFile(
      sourceTranscript.transcriptPath,
      MAX_TRANSCRIPT_BYTES,
      "invalid_source_backup_transcript",
    ),
    readBoundedRegularFile(
      destinationTranscriptPath,
      MAX_TRANSCRIPT_BYTES,
      "invalid_materialized_transcript",
    ),
  ]);
  if (
    destinationBytes.length <= sourceBytes.length ||
    !destinationBytes.subarray(0, sourceBytes.length).equals(sourceBytes)
  ) {
    fail("materialized_transcript_history_changed");
  }
  const appendedText = destinationBytes.subarray(sourceBytes.length).toString("utf8");
  const appendedRecords = [];
  for (const line of appendedText.split(/\r?\n/)) {
    if (!line) {
      continue;
    }
    try {
      appendedRecords.push(JSON.parse(line));
    } catch {
      fail("invalid_materialized_transcript");
    }
  }
  if (appendedRecords.length === 0) {
    fail("materialized_transcript_did_not_grow");
  }
  if (
    appendedRecords.some(
      (record) =>
        typeof record?.sessionId === "string" &&
        record.sessionId.toLowerCase() !== sessionId,
    )
  ) {
    fail("resume_session_id_mismatch");
  }
  const userDisclosedMarker = appendedRecords.some(
    (record) =>
      record?.type === "user" && valueContainsExactMarker(record, marker),
  );
  const assistantRecoveredMarker = appendedRecords.some(
    (record) =>
      record?.type === "assistant" &&
      valueContainsExactMarker(record.message, marker),
  );
  const destinationCwdUsed = appendedRecords.some(
    (record) => record?.cwd === destinationProjectRoot,
  );
  const activeLeafAppended = appendedRecords.some(
    (record) => record?.type === "active-leaf",
  );
  if (
    userDisclosedMarker ||
    !assistantRecoveredMarker ||
    !destinationCwdUsed ||
    !activeLeafAppended
  ) {
    fail("materialized_resume_semantics_not_proved");
  }
  return {
    sessionId,
    marker,
    sourceSnapshot,
    destinationSnapshot,
  };
}

export async function executeMaterializedCrossPathResume(
  { cliBundle, retainedRoot, timeoutMs } = {},
  {
    validateCliBundleImpl = validateQoderCliBundle,
    pinCliBundleImpl = assertPinnedQoderCliBundle,
    report = () => {},
  } = {},
) {
  process.umask(0o077);
  let checkedTimeout;
  let nodePath;
  let checkedCli;
  let root;
  let paths;
  let sourceSessionId;
  let marker;
  let backupCreated = false;
  let destinationCreated = false;
  try {
    checkedTimeout = validateTimeout(timeoutMs);
    nodePath = await validateNode();
    checkedCli = await validateCliBundleImpl(cliBundle);
    root = await assertIsolationRoot(retainedRoot);
    paths = {
      root,
      homeRoot: await requireExactDirectory(root, "home", "invalid_retained_home"),
      configRoot: await requireExactDirectory(
        root,
        "qoder-config",
        "invalid_retained_config",
      ),
      projectRoot: await requireExactDirectory(
        root,
        "empty-project",
        "invalid_retained_source_project",
      ),
      resumeProjectRoot: await requireExactDirectory(
        root,
        "second-empty-project",
        "invalid_retained_destination_project",
      ),
      tempRoot: await requireExactDirectory(root, "tmp", "invalid_retained_tmp"),
      runtimeRoot: await requireExactDirectory(
        root,
        "runner",
        "invalid_retained_runtime",
      ),
    };
    await requireEmptyProject(paths.projectRoot);
    await requireEmptyProject(paths.resumeProjectRoot);
    await assertNoSecurityCommandAttempt(paths);

    const sourceBucketRelative = join(
      "projects",
      computeCliProjectBucket(paths.projectRoot),
    );
    const destinationBucketRelative = join(
      "projects",
      computeCliProjectBucket(paths.resumeProjectRoot),
    );
    let sourceBucket = join(
      paths.configRoot,
      sourceBucketRelative,
    );
    let destinationBucket = join(
      paths.configRoot,
      destinationBucketRelative,
    );
    let backupRoot = join(paths.root, "cross-path-source-backup");
    let backupBucket = join(backupRoot, "source-project-bucket");
    const sourceBucketExists = await pathExists(sourceBucket);
    const destinationBucketExists = await pathExists(destinationBucket);
    const backupRootExists = await pathExists(backupRoot);
    if (sourceBucketExists) {
      sourceBucket = await requireExactDirectory(
        paths.configRoot,
        sourceBucketRelative,
        "invalid_source_project_bucket",
      );
    }
    if (destinationBucketExists) {
      destinationBucket = await requireExactDirectory(
        paths.configRoot,
        destinationBucketRelative,
        "invalid_destination_project_bucket",
      );
    }
    if (backupRootExists) {
      backupRoot = await requireExactDirectory(
        paths.root,
        "cross-path-source-backup",
        "invalid_source_backup",
      );
      backupBucket = await requireExactDirectory(
        paths.root,
        join("cross-path-source-backup", "source-project-bucket"),
        "invalid_source_backup",
      );
    }
    backupCreated = backupRootExists;
    destinationCreated = destinationBucketExists;
    if (
      !sourceBucketExists &&
      destinationBucketExists &&
      backupRootExists
    ) {
      report("Verify the completed materialized resume without another model call.");
      const verified = await verifyMaterializedResumeState({
        backupBucket,
        destinationBucket,
        destinationProjectRoot: paths.resumeProjectRoot,
      });
      sourceSessionId = verified.sessionId;
      marker = verified.marker;
      await assertNoSecurityCommandAttempt(paths);
      await requireEmptyProject(paths.projectRoot);
      await requireEmptyProject(paths.resumeProjectRoot);
      const result = makeMaterializedSuccessResult({
        checkedCli,
        root: paths.root,
        memberCount: verified.sourceSnapshot.members.length,
        memberCountAfterResume: verified.destinationSnapshot.members.length,
        verificationSource: "verified_persisted_transcript",
      });
      const sanitized = { ...result };
      delete sanitized.inspection;
      const serialized = JSON.stringify(sanitized);
      for (const forbidden of [
        sourceSessionId,
        marker,
        paths.root,
        checkedCli.entryPath,
      ]) {
        if (serialized.includes(forbidden)) {
          fail("result_sanitization_failed");
        }
      }
      return result;
    }
    if (!sourceBucketExists || destinationBucketExists || backupRootExists) {
      fail("materialization_destination_exists");
    }
    const sourceTranscript = await findOnlySessionTranscript(sourceBucket);
    sourceSessionId = validateSessionId(sourceTranscript.sessionId);
    marker = await readSourceMarker(sourceTranscript.transcriptPath);
    const sourceSnapshot = await snapshotRegularTree(sourceBucket);
    if (sourceSnapshot.members.length === 0) {
      fail("source_project_bucket_empty");
    }

    report("Move the active source bucket to a recoverable isolated backup.");
    await mkdir(backupRoot, { mode: 0o700 });
    await rename(sourceBucket, backupBucket);
    backupCreated = true;
    await verifyRegularTree(backupBucket, sourceSnapshot);

    report("Materialize a byte-exact session bucket for the destination path.");
    await copyRegularTree(backupBucket, destinationBucket, sourceSnapshot);
    destinationCreated = true;
    const destinationTranscriptBefore = await snapshotTranscript(
      paths.configRoot,
      paths.resumeProjectRoot,
      sourceSessionId,
    );
    if (
      destinationTranscriptBefore.size !==
        sourceSnapshot.members.find(
          (member) => member.relative === `${sourceSessionId}.jsonl`,
        )?.size ||
      destinationTranscriptBefore.digest !==
        sourceSnapshot.members.find(
          (member) => member.relative === `${sourceSessionId}.jsonl`,
        )?.digest
    ) {
      fail("materialized_transcript_mismatch");
    }

    report("Resume the materialized session from the destination path.");
    const environment = buildCleanEnvironment(paths, nodePath);
    const resumeArguments = [
      ...commonTurnArguments(paths, paths.resumeProjectRoot),
      "--resume",
      sourceSessionId,
      "--",
      resumePrompt(),
    ];
    await pinCliBundleImpl(checkedCli);
    const outcome = await runCapturedCommand({
      nodePath,
      cliBundle: checkedCli.entryPath,
      args: resumeArguments,
      cwd: paths.resumeProjectRoot,
      environment,
      timeoutMs: checkedTimeout,
    });
    await pinCliBundleImpl(checkedCli);
    await assertNoSecurityCommandAttempt(paths);
    const resumed = parseSuccessfulCliResult(outcome, "resume");
    const resumedSessionId = validateSessionId(resumed.session_id);
    if (resumedSessionId !== sourceSessionId) {
      fail("resume_session_id_mismatch");
    }
    if (resumed.result.trim() !== marker) {
      fail("resume_marker_mismatch");
    }

    const destinationTranscriptAfter = await snapshotTranscript(
      paths.configRoot,
      paths.resumeProjectRoot,
      sourceSessionId,
    );
    if (
      destinationTranscriptAfter.size <= destinationTranscriptBefore.size ||
      destinationTranscriptAfter.digest === destinationTranscriptBefore.digest
    ) {
      fail("resume_transcript_did_not_grow");
    }
    await verifyRegularTree(backupBucket, sourceSnapshot);
    const destinationSnapshotAfterResume =
      await snapshotRegularTree(destinationBucket);
    await requireEmptyProject(paths.projectRoot);
    await requireEmptyProject(paths.resumeProjectRoot);

    const result = makeMaterializedSuccessResult({
      checkedCli,
      root: paths.root,
      memberCount: sourceSnapshot.members.length,
      memberCountAfterResume: destinationSnapshotAfterResume.members.length,
      verificationSource: "captured_cli_result",
    });
    const sanitized = { ...result };
    delete sanitized.inspection;
    const serialized = JSON.stringify(sanitized);
    for (const forbidden of [
      sourceSessionId,
      marker,
      paths.root,
      checkedCli.entryPath,
    ]) {
      if (serialized.includes(forbidden)) {
        fail("result_sanitization_failed");
      }
    }
    return result;
  } catch (error) {
    return {
      ...materializedBaseResult(false),
      error: { code: safeErrorCode(error) },
      materialization: {
        sourceActiveBucketMovedToBackup: backupCreated,
        sourceBackupRetained: backupCreated,
        destinationBucketCreated: destinationCreated,
      },
      cleanup: {
        attempted: false,
        complete: false,
        kept: Boolean(root),
        retained: Boolean(root),
      },
      ...(root ? { inspection: { root } } : {}),
    };
  }
}

function cleanHomeTransferBaseResult(ok) {
  return {
    schemaVersion: 1,
    ok,
    verdict: ok
      ? CLEAN_HOME_TRANSFER_PASS_VERDICT
      : CLEAN_HOME_TRANSFER_FAIL_VERDICT,
    productionRestoreReady: false,
  };
}

export async function executeCleanHomeCrossPathTransfer(
  { cliBundle, timeoutMs, keep = false } = {},
  {
    validateCliBundleImpl = validateQoderCliBundle,
    pinCliBundleImpl = assertPinnedQoderCliBundle,
    makeMarkerImpl = makeMarker,
    onIsolationRoot,
    report = () => {},
  } = {},
) {
  process.umask(0o077);
  let checkedCli;
  let nodePath;
  let checkedTimeout;
  try {
    checkedCli = await validateCliBundleImpl(cliBundle);
    nodePath = await validateNode();
    checkedTimeout = validateTimeout(timeoutMs);
    if (typeof keep !== "boolean") {
      fail("invalid_keep_value");
    }
  } catch (error) {
    return {
      ...cleanHomeTransferBaseResult(false),
      error: { code: safeErrorCode(error) },
      cleanup: {
        attempted: false,
        complete: true,
        kept: false,
        retained: false,
      },
    };
  }

  let paths;
  let sessionId;
  let marker;
  let result;
  let failureCode;
  let cleanupComplete = false;
  let setupRetainedRoot;
  let setupCleanupAttempted = false;
  const abortController = new AbortController();
  const onSignal = () => abortController.abort();
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  const authenticate = async (machine, label) => {
    if (abortController.signal.aborted) {
      fail("interrupted");
    }
    await pinCliBundleImpl(checkedCli);
    const login = await runInteractiveCommand({
      nodePath,
      cliBundle: checkedCli.entryPath,
      args: [
        "--bare",
        "--config-dir",
        machine.configRoot,
        "--cwd",
        machine.projectRoot,
        "login",
      ],
      cwd: machine.projectRoot,
      environment: buildCleanEnvironment(machine, nodePath),
      timeoutMs: checkedTimeout,
      abortSignal: abortController.signal,
    });
    await pinCliBundleImpl(checkedCli);
    await assertNoSecurityCommandAttempt(machine);
    requireSuccessfulCommand(login, label);
    await requireEmptyProject(machine.projectRoot);
  };

  try {
    paths = await createCleanHomeTransferIsolation(onIsolationRoot);
    const { machineA, machineB, portableRoot } = paths;
    await requireEmptyProject(machineA.projectRoot);
    await requireEmptyProject(machineB.projectRoot);
    await requireEmptyDirectory(
      machineA.configRoot,
      "source_config_not_initially_empty",
    );
    await requireEmptyDirectory(
      machineB.configRoot,
      "destination_config_not_initially_empty",
    );
    if (
      machineA.configRoot === machineB.configRoot ||
      machineA.projectRoot === machineB.projectRoot
    ) {
      fail("machine_isolation_not_distinct");
    }

    report("Authenticate the isolated machine-A Qoder home.");
    await authenticate(machineA, "source_login");

    marker = makeMarkerImpl();
    if (
      typeof marker !== "string" ||
      !/^MALLARD-QODER-RESUME-[0-9a-f]{32}$/.test(marker)
    ) {
      fail("invalid_marker");
    }

    report("Create one native CLI session at machine-A path /A/project_a.");
    const sourceEnvironment = buildCleanEnvironment(machineA, nodePath);
    const sourceArguments = [
      ...commonTurnArguments(machineA),
      "--name",
      "Mallard clean-home transfer probe",
      "--",
      sourcePrompt(marker),
    ];
    await pinCliBundleImpl(checkedCli);
    const sourceOutcome = await runCapturedCommand({
      nodePath,
      cliBundle: checkedCli.entryPath,
      args: sourceArguments,
      cwd: machineA.projectRoot,
      environment: sourceEnvironment,
      timeoutMs: checkedTimeout,
      abortSignal: abortController.signal,
    });
    await pinCliBundleImpl(checkedCli);
    await assertNoSecurityCommandAttempt(machineA);
    const sourceResult = parseSuccessfulCliResult(sourceOutcome, "source");
    sessionId = validateSessionId(sourceResult.session_id);
    if (sourceResult.result.trim() !== marker) {
      fail("source_marker_mismatch");
    }
    const sourceTranscript = await snapshotTranscript(
      machineA.configRoot,
      machineA.projectRoot,
      sessionId,
    );
    const sourceBucketRelative = join(
      "projects",
      computeCliProjectBucket(machineA.projectRoot),
    );
    const sourceBucket = await requireExactDirectory(
      machineA.configRoot,
      sourceBucketRelative,
      "invalid_source_project_bucket",
    );
    const sourceSnapshot = await snapshotRegularTree(sourceBucket);
    assertSessionOnlySnapshot(sourceSnapshot, sessionId);
    await verifyRegularTree(sourceBucket, sourceSnapshot);

    report("Push only the stable session group into the portable bundle.");
    await copyRegularTree(sourceBucket, portableRoot, sourceSnapshot);
    const portableSnapshot = await snapshotRegularTree(portableRoot);
    assertSessionOnlySnapshot(portableSnapshot, sessionId);
    await verifyRegularTree(sourceBucket, sourceSnapshot);
    if (
      sourceTranscript.size !==
        sourceSnapshot.members.find(
          (member) => member.relative === `${sessionId}.jsonl`,
        )?.size ||
      sourceTranscript.digest !==
        sourceSnapshot.members.find(
          (member) => member.relative === `${sessionId}.jsonl`,
        )?.digest
    ) {
      fail("source_transcript_snapshot_mismatch");
    }

    await requireEmptyDirectory(
      machineB.configRoot,
      "destination_config_changed_before_login",
    );
    report("Authenticate the clean machine-B Qoder home independently.");
    await authenticate(machineB, "destination_login");

    const destinationBucketRelative = join(
      "projects",
      computeCliProjectBucket(machineB.projectRoot),
    );
    const destinationBucket = join(
      machineB.configRoot,
      destinationBucketRelative,
    );
    if (await pathExists(destinationBucket)) {
      fail("destination_session_collision");
    }

    report("Pull the session group into machine-B path /B/project_B.");
    await mkdir(dirname(destinationBucket), { recursive: true, mode: 0o700 });
    await copyRegularTree(portableRoot, destinationBucket, portableSnapshot);
    const destinationBefore = await snapshotTranscript(
      machineB.configRoot,
      machineB.projectRoot,
      sessionId,
    );
    if (
      destinationBefore.size !== sourceTranscript.size ||
      destinationBefore.digest !== sourceTranscript.digest
    ) {
      fail("pulled_transcript_mismatch");
    }

    const secondPrompt = resumePrompt();
    const destinationEnvironment = buildCleanEnvironment(machineB, nodePath);
    const resumeArguments = [
      ...commonTurnArguments(machineB),
      "--resume",
      sessionId,
      "--",
      secondPrompt,
    ];
    if (
      secondPrompt.includes(marker) ||
      resumeArguments.some((argument) => argument.includes(marker)) ||
      Object.values(destinationEnvironment).some((value) =>
        value.includes(marker),
      )
    ) {
      fail("resume_marker_channel_detected");
    }

    report("Resume the pulled session in a new machine-B Qoder process.");
    await pinCliBundleImpl(checkedCli);
    const resumeOutcome = await runCapturedCommand({
      nodePath,
      cliBundle: checkedCli.entryPath,
      args: resumeArguments,
      cwd: machineB.projectRoot,
      environment: destinationEnvironment,
      timeoutMs: checkedTimeout,
      abortSignal: abortController.signal,
    });
    await pinCliBundleImpl(checkedCli);
    await assertNoSecurityCommandAttempt(machineA);
    await assertNoSecurityCommandAttempt(machineB);
    const resumeResult = parseSuccessfulCliResult(resumeOutcome, "resume");
    const resumedSessionId = validateSessionId(resumeResult.session_id);
    if (resumedSessionId !== sessionId) {
      fail("resume_session_id_mismatch");
    }
    if (resumeResult.result.trim() !== marker) {
      fail("resume_marker_mismatch");
    }

    const destinationAfter = await snapshotTranscript(
      machineB.configRoot,
      machineB.projectRoot,
      sessionId,
    );
    if (
      destinationAfter.size <= destinationBefore.size ||
      destinationAfter.digest === destinationBefore.digest
    ) {
      fail("resume_transcript_did_not_grow");
    }
    const verified = await verifyMaterializedResumeState({
      backupBucket: portableRoot,
      destinationBucket,
      destinationProjectRoot: machineB.projectRoot,
    });
    await verifyRegularTree(sourceBucket, sourceSnapshot);
    await verifyRegularTree(portableRoot, portableSnapshot);
    await requireEmptyProject(machineA.projectRoot);
    await requireEmptyProject(machineB.projectRoot);

    result = {
      ...cleanHomeTransferBaseResult(true),
      cli: {
        name: checkedCli.name,
        version: checkedCli.version,
        bundleSha256: checkedCli.bundleSha256,
      },
      authentication: {
        sourceMethod: "interactive_isolated_profile",
        destinationMethod: "interactive_isolated_profile",
        destinationAuthenticatedIndependently: true,
        credentialStorage: "isolated_encrypted_file",
        credentialValueReturned: false,
        authenticationCopied: false,
      },
      source: {
        exitZero: true,
        structuredResult: true,
        markerReturned: true,
        nativeTranscriptFound: true,
        stableCapture: true,
        unchangedAfterDestinationResume: true,
      },
      transfer: {
        sessionOnlyBundle: true,
        authenticationCopied: false,
        machineStateCopied: false,
        memberCount: sourceSnapshot.members.length,
        byteExactPush: true,
        byteExactPull: true,
        destinationCollisionChecked: true,
        sourceSessionUuidPreserved: true,
        structuralPathMappingApplied: false,
      },
      resume: {
        exitZero: true,
        sameSessionId: true,
        markerRecovered: true,
        transcriptChanged: true,
        transcriptGrew: true,
        sourceAndDestinationPathsDiffer: true,
        sourceAndDestinationConfigRootsDiffer: true,
        transcriptStorage: "destination_project_bucket",
      },
      isolation: {
        cleanDestinationConfigBeforeLogin: true,
        separateDisposableMachineHomes: true,
        nativeKeychainBypassConfigured: true,
        nativeKeychainCommandAttempted: false,
        normalQoderHomePassedToRunner: false,
        outsideFilesystemAccessAudited: false,
        projectStayedEmpty: true,
        toolsEnabled: false,
        mcpServersEnabled: false,
        builtInSkillsEnabled: false,
      },
      limits: {
        physicalSecondMachineUsed: false,
        realMallardTransportUsed: false,
        oneSessionTested: true,
        sidecarConsumptionProved: false,
        activeLeafAuthorityProved: false,
        qoderAppResumeProved: false,
      },
      evidence: {
        memberCountAfterResume: verified.destinationSnapshot.members.length,
      },
    };

    const serialized = JSON.stringify(result);
    for (const forbidden of [
      sessionId,
      marker,
      paths.root,
      checkedCli.entryPath,
    ]) {
      if (serialized.includes(forbidden)) {
        fail("result_sanitization_failed");
      }
    }
  } catch (error) {
    failureCode = safeErrorCode(error);
    if (
      error &&
      typeof error === "object" &&
      typeof error.retainedIsolationRoot === "string"
    ) {
      setupRetainedRoot = error.retainedIsolationRoot;
    }
    setupCleanupAttempted = error?.setupCleanupAttempted === true;
  } finally {
    process.removeListener("SIGINT", onSignal);
    process.removeListener("SIGTERM", onSignal);
    if (paths && !keep) {
      try {
        await removeIsolation(paths.root);
        cleanupComplete = true;
      } catch {
        failureCode ??= "cleanup_failed";
      }
    }
  }

  if (failureCode || !result) {
    const retainedRoot = paths?.root ?? setupRetainedRoot;
    const retained = Boolean(retainedRoot) && (keep || !cleanupComplete);
    return {
      ...cleanHomeTransferBaseResult(false),
      error: { code: failureCode ?? "unexpected_error" },
      cleanup: {
        attempted: (Boolean(paths) && !keep) || setupCleanupAttempted,
        complete: paths ? cleanupComplete : !setupRetainedRoot,
        kept: Boolean(paths) && keep,
        retained,
      },
      ...(retained ? { inspection: { root: retainedRoot } } : {}),
    };
  }

  result.cleanup = {
    attempted: !keep,
    complete: keep ? false : cleanupComplete,
    kept: keep,
    retained: keep,
  };
  if (keep) {
    result.inspection = { root: paths.root };
  } else if (!cleanupComplete) {
    result.ok = false;
    result.verdict = CLEAN_HOME_TRANSFER_FAIL_VERDICT;
    result.error = { code: "cleanup_failed" };
  }
  return result;
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") {
      return { help: true };
    }
    if (argument === "--keep") {
      options.keep = true;
      continue;
    }
    if (argument === "--cross-path") {
      options.crossPath = true;
      continue;
    }
    if (argument === "--clean-home-transfer") {
      options.cleanHomeTransfer = true;
      continue;
    }
    const value = argv[index + 1];
    if (argument === "--cli-bundle" && value) {
      options.cliBundle = value;
      index += 1;
    } else if (argument === "--materialize-retained-root" && value) {
      options.materializeRetainedRoot = value;
      index += 1;
    } else if (argument === "--timeout-ms" && value) {
      options.timeoutMs = Number(value);
      index += 1;
    } else {
      fail("invalid_argument");
    }
  }
  if (
    options.materializeRetainedRoot &&
    (options.keep || options.crossPath || options.cleanHomeTransfer)
  ) {
    fail("invalid_argument_combination");
  }
  if (options.cleanHomeTransfer && options.crossPath) {
    fail("invalid_argument_combination");
  }
  return options;
}

function usage() {
  return [
    "Usage:",
    "  node qoder-cli-authenticated-resume.mjs \\",
    "    --cli-bundle /absolute/package/path/bundle/qodercli.js",
    "",
    "Options:",
    "  --timeout-ms 180000  Per-operation timeout; range 30000 to 900000.",
    "  --cross-path         Resume from a second empty working directory.",
    "  --clean-home-transfer",
    "                       Push one session from home A and pull it into an",
    "                       independently authenticated clean home B.",
    "  --materialize-retained-root <path>",
    "                       Move a retained source bucket to a verified backup,",
    "                       materialize it for the second path, and resume it.",
    "  --keep               Keep isolated login and session state.",
    "  --help               Show this text.",
    "",
    "The clean-home test starts two Qoder logins and sends two short model",
    "requests. Other modes start one login.",
    "The runner does not read or copy the normal ~/.qoder directory.",
  ].join("\n");
}

async function main() {
  process.umask(0o077);
  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    process.stdout.write(
      `${JSON.stringify({
        ...baseResult(false),
        error: { code: safeErrorCode(error) },
        cleanup: {
          attempted: false,
          complete: true,
          kept: false,
          retained: false,
        },
      })}\n`,
    );
    process.exitCode = 1;
    return;
  }
  if (options.help) {
    process.stdout.write(`${usage()}\n`);
    return;
  }

  const reporter = {
    report: (message) => process.stderr.write(`[qoder-resume-test] ${message}\n`),
  };
  const result = options.materializeRetainedRoot
    ? await executeMaterializedCrossPathResume(
        {
          cliBundle: options.cliBundle,
          retainedRoot: options.materializeRetainedRoot,
          timeoutMs: options.timeoutMs,
        },
        reporter,
      )
    : options.cleanHomeTransfer
      ? await executeCleanHomeCrossPathTransfer(options, reporter)
      : await executeAuthenticatedResumeSmoke(options, reporter);
  if (result.inspection?.root) {
    process.stderr.write(
      `[qoder-resume-test] Kept sensitive state: ${result.inspection.root}\n`,
    );
  }
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  process.exitCode = result.ok ? 0 : 1;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath && pathToFileURL(invokedPath).href === import.meta.url) {
  await main();
}
