#!/usr/bin/env node

import { spawn, execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  access,
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

export const QODER_APP_VERSION = "0.1.4";
export const QODER_APP_PACKAGE_NAME = "qoder";
export const QODER_APP_MAIN_SHA256 =
  "c8b0230de432f9dbe6c027884005504e1348119ee6b3632fbe04714bb2a3ef19";
export const APP_AUTH_READY_MESSAGE =
  "Auth state initialized from isolated Desktop E2E configuration";

const PASS_WITHOUT_PROJECTION =
  "QODER_APP_PROJECTION_REBUILD_DOES_NOT_PASS";
const PASS_WITH_PROJECTION =
  "QODER_APP_DISCOVERY_PASS_CONTINUE_UNPROVED";
const FAIL_VERDICT = "QODER_APP_PROJECTION_PROBE_INCONCLUSIVE";
const SOURCE_ROOT_PREFIX = "mallard-qoder-manual-resume-";
const SOURCE_MARKER_NAME = ".mallard-qoder-manual-resume";
const SOURCE_MARKER_CONTENT = "mallard-qoder-manual-resume-v1\n";
const ROOT_PREFIX = "mallard-qoder-app-resume-poc-";
const ROOT_MARKER_NAME = ".mallard-qoder-app-resume-poc";
const ROOT_MARKER_CONTENT = "mallard-qoder-app-resume-poc-v1\n";
const DEMO_DATA_DIRECTORY = "com.qoder.app.demo";
const SECURITY_GUARD_LOG_NAME = "security-command-attempted.log";
const SESSION_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const DEFAULT_TIMEOUT_MS = 45_000;
const MIN_TIMEOUT_MS = 10_000;
const MAX_TIMEOUT_MS = 180_000;
const DEFAULT_SETTLE_MS = 5_000;
const MAX_CAPTURE_BYTES = 1024 * 1024;
const MAX_TRANSCRIPT_BYTES = 64 * 1024 * 1024;
const MAX_APP_MAIN_BYTES = 32 * 1024 * 1024;
const MAX_JSON_BYTES = 1024 * 1024;
const MAX_PROJECT_BUCKETS = 512;
const MAX_BUCKET_ENTRIES = 2_048;

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

async function validateCanonicalDirectory(directoryPath, code) {
  if (
    typeof directoryPath !== "string" ||
    !isAbsolute(directoryPath) ||
    directoryPath.includes("\0")
  ) {
    fail(code);
  }
  const canonical = await realpath(directoryPath).catch(() => fail(code));
  const metadata = await lstat(canonical).catch(() => fail(code));
  if (
    canonical !== resolve(directoryPath) ||
    !metadata.isDirectory() ||
    metadata.isSymbolicLink()
  ) {
    fail(code);
  }
  return { canonical, metadata };
}

async function validateCanonicalRegularFile(filePath, code, executable = false) {
  if (
    typeof filePath !== "string" ||
    !isAbsolute(filePath) ||
    filePath.includes("\0")
  ) {
    fail(code);
  }
  const canonical = await realpath(filePath).catch(() => fail(code));
  const metadata = await lstat(canonical).catch(() => fail(code));
  if (
    canonical !== resolve(filePath) ||
    !metadata.isFile() ||
    metadata.isSymbolicLink()
  ) {
    fail(code);
  }
  if (executable) {
    await access(canonical, fsConstants.X_OK).catch(() => fail(code));
  }
  return { canonical, metadata };
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

async function hashBoundedRegularFile(filePath, maximumBytes, code) {
  const bytes = await readBoundedRegularFile(filePath, maximumBytes, code);
  return {
    size: bytes.length,
    digest: createHash("sha256").update(bytes).digest("hex"),
  };
}

async function readJsonFile(filePath, code) {
  const bytes = await readBoundedRegularFile(filePath, MAX_JSON_BYTES, code);
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    fail(code);
  }
}

async function validateAppSource(
  appSource,
  expectedMainSha256 = QODER_APP_MAIN_SHA256,
) {
  const { canonical } = await validateCanonicalDirectory(
    appSource,
    "invalid_app_source",
  );
  const packagePath = join(canonical, "package.json");
  await validateCanonicalRegularFile(packagePath, "invalid_app_package");
  const packageDocument = await readJsonFile(packagePath, "invalid_app_package");
  if (packageDocument?.name !== QODER_APP_PACKAGE_NAME) {
    fail("unexpected_app_package");
  }
  if (packageDocument?.version !== QODER_APP_VERSION) {
    fail("unexpected_app_version");
  }
  if (packageDocument?.main !== "./out/main/index.js") {
    fail("unexpected_app_entrypoint");
  }
  const mainPath = join(canonical, "out", "main", "index.js");
  await validateCanonicalRegularFile(mainPath, "invalid_app_entrypoint");
  const main = await hashBoundedRegularFile(
    mainPath,
    MAX_APP_MAIN_BYTES,
    "invalid_app_entrypoint",
  );
  if (main.digest !== expectedMainSha256) {
    fail("unexpected_app_main_digest");
  }
  return {
    path: canonical,
    name: packageDocument.name,
    version: packageDocument.version,
    mainSha256: main.digest,
  };
}

async function validateAppResources(appResources) {
  const { canonical } = await validateCanonicalDirectory(
    appResources,
    "invalid_app_resources",
  );
  const productPath = join(canonical, "product.json");
  await validateCanonicalRegularFile(productPath, "invalid_product_manifest");
  const product = await readJsonFile(productPath, "invalid_product_manifest");
  if (
    product?.productId !== "qoder" ||
    product?.appId !== "com.qoder.app" ||
    product?.packageName !== "qoder" ||
    product?.protocolScheme !== "qoder" ||
    typeof product?.displayName !== "string" ||
    typeof product?.electronApplicationName !== "string" ||
    typeof product?.windowsLauncherDirectory !== "string" ||
    typeof product?.windowsLauncherFileName !== "string" ||
    typeof product?.desktopUpdateDownloadBaseUrl !== "string"
  ) {
    fail("unexpected_product_manifest");
  }
  const bundledResources = join(canonical, "bundled-resources");
  const umid = join(canonical, "umid");
  await validateCanonicalDirectory(
    bundledResources,
    "invalid_bundled_resources",
  );
  await validateCanonicalDirectory(umid, "invalid_umid_resources");
  return { path: canonical, product, bundledResources, umid };
}

async function validateElectronExecutable(electronPath) {
  const { canonical } = await validateCanonicalRegularFile(
    electronPath,
    "invalid_electron_executable",
    true,
  );
  return canonical;
}

async function assertSourceRoot(root) {
  const { canonical, metadata } = await validateCanonicalDirectory(
    root,
    "unsafe_source_root",
  );
  const canonicalTemp = await realpath(tmpdir()).catch(() =>
    fail("unsafe_source_root"),
  );
  if (
    dirname(canonical) !== canonicalTemp ||
    !basename(canonical).startsWith(SOURCE_ROOT_PREFIX) ||
    (metadata.mode & 0o077) !== 0 ||
    (typeof process.getuid === "function" && metadata.uid !== process.getuid())
  ) {
    fail("unsafe_source_root");
  }
  const markerPath = join(canonical, SOURCE_MARKER_NAME);
  const { metadata: markerMetadata } = await validateCanonicalRegularFile(
    markerPath,
    "unsafe_source_root",
  );
  if (
    (markerMetadata.mode & 0o077) !== 0 ||
    (typeof process.getuid === "function" &&
      markerMetadata.uid !== process.getuid()) ||
    (await readFile(markerPath, "utf8")) !== SOURCE_MARKER_CONTENT
  ) {
    fail("unsafe_source_root");
  }
  return canonical;
}

async function validateQoderConfig(configPath) {
  const { canonical } = await validateCanonicalDirectory(
    configPath,
    "invalid_qoder_config",
  );
  if (basename(canonical) !== "qoder-config") {
    fail("invalid_qoder_config");
  }
  const sourceRoot = await assertSourceRoot(dirname(canonical));
  if (canonical !== join(sourceRoot, "qoder-config")) {
    fail("invalid_qoder_config");
  }
  return { path: canonical, sourceRoot };
}

async function discoverTranscript(configPath, requestedSessionId) {
  const projectsRoot = join(configPath, "projects");
  await validateCanonicalDirectory(projectsRoot, "missing_projects_root");
  const candidates = [];
  let bucketCount = 0;
  const projects = await opendir(projectsRoot);
  for await (const projectEntry of projects) {
    bucketCount += 1;
    if (bucketCount > MAX_PROJECT_BUCKETS) {
      fail("project_bucket_limit_exceeded");
    }
    const bucketPath = join(projectsRoot, projectEntry.name);
    const bucketMetadata = await lstat(bucketPath).catch(() =>
      fail("unsafe_project_bucket"),
    );
    if (bucketMetadata.isSymbolicLink()) {
      fail("unsafe_project_bucket");
    }
    if (!bucketMetadata.isDirectory()) {
      continue;
    }
    let entryCount = 0;
    const bucket = await opendir(bucketPath);
    for await (const entry of bucket) {
      entryCount += 1;
      if (entryCount > MAX_BUCKET_ENTRIES) {
        fail("project_entry_limit_exceeded");
      }
      if (!entry.name.endsWith(".jsonl")) {
        continue;
      }
      const sessionId = entry.name.slice(0, -".jsonl".length);
      if (!SESSION_ID_PATTERN.test(sessionId)) {
        continue;
      }
      const transcriptPath = join(bucketPath, entry.name);
      const metadata = await lstat(transcriptPath).catch(() =>
        fail("unsafe_transcript"),
      );
      if (!metadata.isFile() || metadata.isSymbolicLink()) {
        fail("unsafe_transcript");
      }
      if (
        requestedSessionId === undefined ||
        sessionId.toLowerCase() === requestedSessionId
      ) {
        candidates.push({
          sessionId: sessionId.toLowerCase(),
          transcriptPath,
        });
      }
    }
  }
  if (candidates.length === 0) {
    fail("source_transcript_not_found");
  }
  if (candidates.length !== 1) {
    fail("source_transcript_ambiguous");
  }
  const transcript = await hashBoundedRegularFile(
    candidates[0].transcriptPath,
    MAX_TRANSCRIPT_BYTES,
    "unsafe_transcript",
  );
  return { ...candidates[0], ...transcript };
}

async function assertIsolationRoot(root) {
  const { canonical, metadata } = await validateCanonicalDirectory(
    root,
    "unsafe_isolation_root",
  );
  const canonicalTemp = await realpath(tmpdir()).catch(() =>
    fail("unsafe_isolation_root"),
  );
  if (
    dirname(canonical) !== canonicalTemp ||
    !basename(canonical).startsWith(ROOT_PREFIX) ||
    (metadata.mode & 0o077) !== 0 ||
    (typeof process.getuid === "function" && metadata.uid !== process.getuid())
  ) {
    fail("unsafe_isolation_root");
  }
  const markerPath = join(canonical, ROOT_MARKER_NAME);
  const { metadata: markerMetadata } = await validateCanonicalRegularFile(
    markerPath,
    "unsafe_isolation_root",
  );
  if (
    (markerMetadata.mode & 0o077) !== 0 ||
    (typeof process.getuid === "function" &&
      markerMetadata.uid !== process.getuid()) ||
    (await readFile(markerPath, "utf8")) !== ROOT_MARKER_CONTENT
  ) {
    fail("unsafe_isolation_root");
  }
  return canonical;
}

function buildDevelopmentProductCatalog(product) {
  return {
    schemaVersion: 1,
    products: [
      {
        id: "qoder",
        displayName: product.displayName,
        electronApplicationName: product.electronApplicationName,
        appId: product.appId,
        dataDirectoryNames: {
          stable: "com.qoder.app.stable",
          canary: "com.qoder.app.canary",
        },
        protocolScheme: product.protocolScheme,
        iconDirectory: "qoder",
        windowsLauncherDirectory: product.windowsLauncherDirectory,
        windowsLauncherFileName: product.windowsLauncherFileName,
        desktopUpdateDownloadBaseUrl: product.desktopUpdateDownloadBaseUrl,
      },
    ],
  };
}

async function createIsolation(product, onIsolationRoot) {
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
      profileRoot: join(root, "app-profile"),
      tempRoot: join(root, "tmp"),
      runtimeRoot: join(root, "runtime"),
      xdgCacheRoot: join(root, "xdg-cache"),
      xdgConfigRoot: join(root, "xdg-config"),
      xdgDataRoot: join(root, "xdg-data"),
      xdgStateRoot: join(root, "xdg-state"),
    };
    for (const directory of Object.values(paths).slice(1)) {
      await mkdir(directory, { mode: 0o700 });
    }
    paths.databasePath = join(
      paths.profileRoot,
      DEMO_DATA_DIRECTORY,
      "main.sqlite",
    );
    paths.productCatalogPath = join(paths.runtimeRoot, "products.v1.json");
    paths.runtimePath = join(paths.runtimeRoot, "desktop-e2e-runtime");
    paths.securityGuardPath = join(paths.runtimeRoot, "security");
    paths.securityGuardLog = join(
      paths.runtimeRoot,
      SECURITY_GUARD_LOG_NAME,
    );
    await writeFile(
      paths.productCatalogPath,
      `${JSON.stringify(buildDevelopmentProductCatalog(product))}\n`,
      { flag: "wx", mode: 0o600 },
    );
    await writeFile(
      paths.runtimePath,
      [
        "#!/bin/sh",
        'if [ "$1" = "--version" ]; then',
        '  printf "%s\\n" "0.0.0-mallard-e2e"',
        "  exit 0",
        "fi",
        "exit 91",
        "",
      ].join("\n"),
      { flag: "wx", mode: 0o700 },
    );
    await writeFile(
      paths.securityGuardPath,
      [
        "#!/bin/sh",
        "umask 077",
        'printf "%s\\n" blocked > "$MALLARD_QODER_SECURITY_GUARD_LOG"',
        "exit 92",
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
  const canonical = await assertIsolationRoot(root);
  await rm(canonical, { recursive: true, force: false });
}

function safeEnvironmentValue(value, pattern, fallback) {
  return typeof value === "string" && pattern.test(value) ? value : fallback;
}

function buildCleanEnvironment(paths, qoderConfig, resources, electronPath) {
  return Object.freeze({
    HOME: paths.homeRoot,
    USER: "mallard-qoder-app-probe",
    LOGNAME: "mallard-qoder-app-probe",
    SHELL: "/bin/sh",
    PATH: [
      paths.runtimeRoot,
      dirname(electronPath),
      "/usr/bin",
      "/bin",
      "/usr/sbin",
      "/sbin",
    ].join(delimiter),
    TMPDIR: paths.tempRoot,
    TMP: paths.tempRoot,
    TEMP: paths.tempRoot,
    XDG_CACHE_HOME: paths.xdgCacheRoot,
    XDG_CONFIG_HOME: paths.xdgConfigRoot,
    XDG_DATA_HOME: paths.xdgDataRoot,
    XDG_STATE_HOME: paths.xdgStateRoot,
    QODER_CONFIG_DIR: qoderConfig,
    QODERCN_CONFIG_DIR: qoderConfig,
    QODER_E2E: "true",
    QODER_E2E_MODE: "smoke",
    QODER_E2E_PROFILE_ROOT: paths.profileRoot,
    QODER_E2E_RUNTIME_PATH: paths.runtimePath,
    QODER_DEMO_DATA: "true",
    QODER_PRODUCT_CATALOG_PATH: paths.productCatalogPath,
    QODER_DESKTOP_BUNDLED_RESOURCES_DIR: resources.bundledResources,
    QODER_DESKTOP_UMID_RUNTIME_DIR: resources.umid,
    QODER_RUM_DEV_REPORT: "0",
    ELECTRON_ENABLE_LOGGING: "1",
    ELECTRON_DISABLE_SECURITY_WARNINGS: "0",
    MALLARD_QODER_SECURITY_GUARD_LOG: paths.securityGuardLog,
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
  });
}

function appendBounded(current, chunk) {
  if (current.length >= MAX_CAPTURE_BYTES) {
    return current;
  }
  const remaining = MAX_CAPTURE_BYTES - current.length;
  return current + chunk.toString("utf8", 0, remaining);
}

function killChildGroup(child, signal) {
  try {
    if (process.platform !== "win32" && child.pid) {
      process.kill(-child.pid, signal);
    } else {
      child.kill(signal);
    }
  } catch {
    try {
      child.kill(signal);
    } catch {
      // The process already stopped.
    }
  }
}

function wait(milliseconds) {
  return new Promise((complete) => setTimeout(complete, milliseconds));
}

async function fileExists(filePath) {
  try {
    const metadata = await lstat(filePath);
    return metadata.isFile() && !metadata.isSymbolicLink();
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

async function launchAppAndWait({
  electronPath,
  appSource,
  environment,
  databasePath,
  timeoutMs,
  settleMs,
}) {
  const child = spawn(
    electronPath,
    [
      "--use-mock-keychain",
      "--disable-features=DialMediaRouteProvider",
      "--disable-background-networking",
      "--disable-gpu",
      appSource,
    ],
    {
      cwd: appSource,
      env: environment,
      detached: process.platform !== "win32",
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let output = "";
  child.stdout.on("data", (chunk) => {
    output = appendBounded(output, chunk);
  });
  child.stderr.on("data", (chunk) => {
    output = appendBounded(output, chunk);
  });
  let exitResult;
  const exitPromise = new Promise((complete) => {
    child.once("error", (error) => {
      exitResult = { error };
      complete(exitResult);
    });
    child.once("exit", (code, signal) => {
      exitResult = { code, signal };
      complete(exitResult);
    });
  });

  const deadline = Date.now() + timeoutMs;
  let evidenceAt;
  let databaseCreated = false;
  let fakeAuthInitialized = false;
  while (Date.now() < deadline) {
    databaseCreated = await fileExists(databasePath);
    fakeAuthInitialized = output.includes(APP_AUTH_READY_MESSAGE);
    if (databaseCreated && fakeAuthInitialized) {
      evidenceAt ??= Date.now();
      if (Date.now() - evidenceAt >= settleMs) {
        break;
      }
    }
    if (exitResult) {
      break;
    }
    await wait(100);
  }

  const timedOut = !exitResult && Date.now() >= deadline;
  if (!exitResult) {
    killChildGroup(child, "SIGTERM");
    await Promise.race([exitPromise, wait(3_000)]);
  }
  if (!exitResult) {
    killChildGroup(child, "SIGKILL");
    await Promise.race([exitPromise, wait(2_000)]);
  }
  databaseCreated = await fileExists(databasePath);
  fakeAuthInitialized = output.includes(APP_AUTH_READY_MESSAGE);
  return {
    databaseCreated,
    fakeAuthInitialized,
    timedOut,
    exitedBeforeEvidence:
      Boolean(exitResult) && (!databaseCreated || !fakeAuthInitialized),
    outputTruncated: output.length >= MAX_CAPTURE_BYTES,
    exitCode: Number.isInteger(exitResult?.code) ? exitResult.code : null,
    exitSignal:
      typeof exitResult?.signal === "string" ? exitResult.signal : null,
  };
}

function execFileText(file, args, options = {}) {
  return new Promise((complete, reject) => {
    execFile(
      file,
      args,
      { encoding: "utf8", maxBuffer: MAX_CAPTURE_BYTES, ...options },
      (error, stdout) => {
        if (error) {
          reject(error);
        } else {
          complete(stdout);
        }
      },
    );
  });
}

async function inspectDatabase(databasePath, sessionId) {
  await validateCanonicalRegularFile(databasePath, "invalid_app_database");
  const statements = [
    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chat_sessions';",
    "SELECT COUNT(*) FROM chat_sessions;",
    `SELECT COUNT(*) FROM chat_sessions WHERE session_id = '${sessionId}';`,
    "PRAGMA user_version;",
  ].join("\n");
  let stdout;
  try {
    stdout = await execFileText(
      "/usr/bin/sqlite3",
      ["-readonly", "-batch", databasePath, statements],
      { timeout: 10_000 },
    );
  } catch {
    fail("database_inspection_failed");
  }
  const values = stdout
    .trim()
    .split(/\r?\n/)
    .map((value) => Number(value));
  if (
    values.length !== 4 ||
    values.some((value) => !Number.isSafeInteger(value) || value < 0)
  ) {
    fail("database_inspection_failed");
  }
  return {
    chatSessionsTableFound: values[0] === 1,
    totalSessions: values[1],
    sourceSessionRows: values[2],
    schemaUserVersion: values[3],
  };
}

async function findNamedFile(root, expectedName, maximumEntries = 2_048) {
  let entriesSeen = 0;
  const pending = [root];
  while (pending.length > 0) {
    const current = pending.pop();
    const directory = await opendir(current);
    for await (const entry of directory) {
      entriesSeen += 1;
      if (entriesSeen > maximumEntries) {
        fail("app_profile_entry_limit_exceeded");
      }
      const entryPath = join(current, entry.name);
      const metadata = await lstat(entryPath).catch(() =>
        fail("unsafe_app_profile_entry"),
      );
      if (metadata.isSymbolicLink()) {
        if (entry.name === expectedName) {
          fail("unsafe_app_profile_entry");
        }
        continue;
      }
      if (metadata.isDirectory()) {
        pending.push(entryPath);
      } else if (metadata.isFile() && entry.name === expectedName) {
        return true;
      }
    }
  }
  return false;
}

export function classifyProjection(database) {
  if (!database?.chatSessionsTableFound) {
    return {
      verdict: FAIL_VERDICT,
      projectionRebuilt: false,
      appDiscoveredSession: false,
      qoderAppResumeProved: false,
    };
  }
  if (database.sourceSessionRows === 0) {
    return {
      verdict: PASS_WITHOUT_PROJECTION,
      projectionRebuilt: false,
      appDiscoveredSession: false,
      qoderAppResumeProved: false,
    };
  }
  return {
    verdict: PASS_WITH_PROJECTION,
    projectionRebuilt: true,
    appDiscoveredSession: true,
    qoderAppResumeProved: false,
  };
}

function baseResult(ok = false) {
  return {
    schemaVersion: 1,
    ok,
    verdict: ok ? undefined : FAIL_VERDICT,
    productionRestoreReady: false,
  };
}

export async function executeQoderAppProjectionProbe(
  options,
  dependencies = {},
) {
  process.umask(0o077);
  const timeoutMs = validateTimeout(options?.timeoutMs);
  const settleMs = dependencies.settleMs ?? DEFAULT_SETTLE_MS;
  if (!Number.isSafeInteger(settleMs) || settleMs < 0 || settleMs > 30_000) {
    fail("invalid_settle_time");
  }
  const requestedSessionId =
    options?.sessionId === undefined
      ? undefined
      : validateSessionId(options.sessionId);
  let paths;
  let result;
  let failureCode;
  let cleanupComplete = false;
  let setupRetainedRoot;
  let setupCleanupAttempted = false;
  try {
    const [app, resources, electron, config] = await Promise.all([
      validateAppSource(
        options?.appSource,
        dependencies.expectedAppMainSha256 ?? QODER_APP_MAIN_SHA256,
      ),
      validateAppResources(options?.appResources),
      validateElectronExecutable(options?.electron),
      validateQoderConfig(options?.qoderConfig),
    ]);
    const source = await discoverTranscript(config.path, requestedSessionId);
    paths = await createIsolation(resources.product, dependencies.onIsolationRoot);
    const environment = buildCleanEnvironment(
      paths,
      config.path,
      resources,
      electron,
    );
    const launch = await launchAppAndWait({
      electronPath: electron,
      appSource: app.path,
      environment,
      databasePath: paths.databasePath,
      timeoutMs,
      settleMs,
    });
    if (!launch.databaseCreated) {
      fail("app_database_not_created");
    }
    if (!launch.fakeAuthInitialized) {
      fail("isolated_fake_auth_not_initialized");
    }
    const database = await inspectDatabase(paths.databasePath, source.sessionId);
    const sourceAfter = await hashBoundedRegularFile(
      source.transcriptPath,
      MAX_TRANSCRIPT_BYTES,
      "source_transcript_changed",
    );
    if (
      sourceAfter.size !== source.size ||
      sourceAfter.digest !== source.digest
    ) {
      fail("source_transcript_changed");
    }
    const authFileCreated = await findNamedFile(paths.profileRoot, "auth.v1.dat");
    if (authFileCreated) {
      fail("persisted_auth_file_created");
    }
    const securityCommandAttempted = await fileExists(paths.securityGuardLog);
    if (securityCommandAttempted) {
      fail("native_keychain_command_attempted");
    }
    const projection = classifyProjection(database);
    result = {
      ...baseResult(true),
      verdict: projection.verdict,
      app: {
        name: "Qoder",
        version: app.version,
        sourcePackage: app.name,
        mainSha256: app.mainSha256,
        executionMode: "unpackaged_e2e_smoke",
      },
      source: {
        nativeCliTranscriptFound: true,
        transcriptUnchanged: true,
      },
      launch: {
        cleanProfile: true,
        databaseCreated: true,
        fakeAuthConfigured: true,
        fakeAuthInitialized: true,
        persistedAuthBypassed: true,
        authFileCreated: false,
        chromiumMockKeychainConfigured: true,
        nativeSecurityCommandAttempted: false,
        timedOut: launch.timedOut,
        exitedBeforeEvidence: launch.exitedBeforeEvidence,
        outputTruncated: launch.outputTruncated,
      },
      database: {
        initialized: true,
        chatSessionsTableFound: database.chatSessionsTableFound,
        totalSessions: database.totalSessions,
        sourceSessionRows: database.sourceSessionRows,
        schemaUserVersion: database.schemaUserVersion,
        projectionRebuilt: projection.projectionRebuilt,
      },
      resume: {
        appDiscoveredSession: projection.appDiscoveredSession,
        continuationAttempted: false,
        qoderAppResumeProved: projection.qoderAppResumeProved,
      },
      isolation: {
        qoderConfiguredForIsolatedHome: true,
        appConfiguredForCleanProfile: true,
        normalQoderHomeAccessAudited: false,
        outsideFilesystemAccessAudited: false,
        parentCredentialEnvironmentInherited: false,
        realKeychainPersistenceDisabled: true,
      },
      limits: {
        packagedApplicationTested: false,
        appUiContinuationTested: false,
        appDatabaseCopiedOrModified: false,
        mallardRestoreProved: false,
        qoderAppResumeProved: false,
      },
    };
    const serialized = JSON.stringify(result);
    for (const forbidden of [
      source.sessionId,
      source.transcriptPath,
      config.path,
      config.sourceRoot,
      app.path,
      resources.path,
      electron,
      paths.root,
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
    if (paths && !options?.keep) {
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
    const retained = Boolean(retainedRoot) &&
      (Boolean(options?.keep) || !cleanupComplete);
    return {
      ...baseResult(false),
      error: { code: failureCode ?? "unexpected_error" },
      cleanup: {
        attempted: (Boolean(paths) && !options?.keep) || setupCleanupAttempted,
        complete: paths ? cleanupComplete : !setupRetainedRoot,
        kept: Boolean(paths) && Boolean(options?.keep),
        retained,
      },
      ...(retained ? { inspection: { root: retainedRoot } } : {}),
    };
  }

  result.cleanup = {
    attempted: !options?.keep,
    complete: options?.keep ? false : cleanupComplete,
    kept: Boolean(options?.keep),
    retained: Boolean(options?.keep),
  };
  if (options?.keep) {
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
    if (argument === "--electron" && value) {
      options.electron = value;
      index += 1;
    } else if (argument === "--app-source" && value) {
      options.appSource = value;
      index += 1;
    } else if (argument === "--app-resources" && value) {
      options.appResources = value;
      index += 1;
    } else if (argument === "--qoder-config" && value) {
      options.qoderConfig = value;
      index += 1;
    } else if (argument === "--session-id" && value) {
      options.sessionId = value;
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
    "  node qoder-app-projection-probe.mjs \\",
    "    --electron /absolute/path/to/Electron \\",
    "    --app-source /absolute/path/to/extracted/qoder/app \\",
    "    --app-resources /Applications/Qoder.app/Contents/Resources \\",
    "    --qoder-config /private/.../mallard-qoder-manual-resume-*/qoder-config",
    "",
    "Options:",
    "  --session-id <uuid>  Select one native CLI transcript.",
    "  --timeout-ms 45000   App startup timeout; range 10000 to 180000.",
    "  --keep               Keep the disposable App profile.",
    "  --help               Show this text.",
    "",
    "The source configuration must come from the guarded authenticated CLI probe.",
    "The probe uses Qoder's internal unpackaged E2E smoke mode and a clean App DB.",
    "It does not modify the CLI transcript or copy a Qoder App database.",
  ].join("\n");
}

async function main() {
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
  const result = await executeQoderAppProjectionProbe(options);
  if (result.inspection?.root) {
    process.stderr.write(
      `[qoder-app-resume-test] Kept App state: ${result.inspection.root}\n`,
    );
  }
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  process.exitCode = result.ok ? 0 : 1;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath && pathToFileURL(invokedPath).href === import.meta.url) {
  await main();
}
