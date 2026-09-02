#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  open,
  opendir,
  readFile,
  realpath,
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
const FAIL_VERDICT = "AUTHENTICATED_NATIVE_RESUME_NOT_PROVED";
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
      tempRoot: join(root, "tmp"),
      runtimeRoot: join(root, "runner"),
    };
    for (const directory of Object.values(paths).slice(1)) {
      await mkdir(directory, { mode: 0o700 });
    }
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

function commonTurnArguments(paths) {
  return [
    "--bare",
    "--config-dir",
    paths.configRoot,
    "--cwd",
    paths.projectRoot,
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

async function requireEmptyProject(projectRoot) {
  const directory = await opendir(projectRoot).catch(() =>
    fail("invalid_test_project"),
  );
  for await (const _entry of directory) {
    fail("test_project_not_empty");
  }
}

function baseResult(ok) {
  return {
    schemaVersion: 1,
    ok,
    verdict: ok ? PASS_VERDICT : FAIL_VERDICT,
    productionRestoreReady: false,
  };
}

export async function executeAuthenticatedResumeSmoke(
  { cliBundle, timeoutMs, keep = false } = {},
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
  } catch (error) {
    return {
      ...baseResult(false),
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

    const secondPrompt = resumePrompt();
    const resumeArguments = [
      ...commonTurnArguments(paths),
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
      cwd: paths.projectRoot,
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

    const resumedTranscript = await snapshotTranscript(
      paths.configRoot,
      paths.projectRoot,
      sessionId,
    );
    if (
      resumedTranscript.size <= sourceTranscript.size ||
      resumedTranscript.digest === sourceTranscript.digest
    ) {
      fail("resume_transcript_did_not_grow");
    }
    if (resumedTranscript.transcriptPath !== sourceTranscript.transcriptPath) {
      fail("resume_transcript_path_changed");
    }
    await requireEmptyProject(paths.projectRoot);

    result = {
      ...baseResult(true),
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
      },
      isolation: {
        qoderConfiguredForIsolatedHome: true,
        nativeKeychainBypassConfigured: true,
        nativeKeychainCommandAttempted: false,
        runnerReadNormalQoderHome: false,
        outsideFilesystemAccessAudited: false,
        exactWorkingDirectoryReused: true,
        projectStayedEmpty: true,
        toolsEnabled: false,
        mcpServersEnabled: false,
        builtInSkillsEnabled: false,
      },
      limits: {
        mallardRestoreProved: false,
        sidecarConsumptionProved: false,
        activeLeafAuthorityProved: false,
        crossPathResumeProved: false,
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
      ...baseResult(false),
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
    result.verdict = FAIL_VERDICT;
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
    const value = argv[index + 1];
    if (argument === "--cli-bundle" && value) {
      options.cliBundle = value;
      index += 1;
    } else if (argument === "--timeout-ms" && value) {
      options.timeoutMs = Number(value);
      index += 1;
    } else {
      fail("invalid_argument");
    }
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
    "  --keep               Keep isolated login and session state.",
    "  --help               Show this text.",
    "",
    "The test starts Qoder login and sends two short model requests.",
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

  const result = await executeAuthenticatedResumeSmoke(options, {
    report: (message) => process.stderr.write(`[qoder-resume-test] ${message}\n`),
  });
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
