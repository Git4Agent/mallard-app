import { spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readdir,
  realpath,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const POC_PREFIX = "mallard-qoder-provider-poc-";
const INTERNAL_RUNNER_ARGUMENT = "--internal-qoder-runner";
const MAX_CREDENTIAL_BYTES = 64 * 1024;
const MAX_HTTP_RESPONSE_BYTES = 1024 * 1024;
const MAX_CHILD_OUTPUT_BYTES = 2 * 1024 * 1024;
const MAX_SCAN_FILES = 10_000;
const MAX_SCAN_BYTES = 64 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS = 60_000;
const FIXED_PROMPT = "Reply with exactly OK. Do not use tools.";
const QODER_PAT_ENV = "MALLARD_QODER_PAT";

const QODER_TOOL_DENYLIST = Object.freeze([
  "Agent",
  "AskUserQuestion",
  "Bash",
  "Edit",
  "Glob",
  "Grep",
  "ImageGen",
  "ImageSearch",
  "NotebookEdit",
  "Read",
  "Skill",
  "TaskCreate",
  "TaskGet",
  "TaskList",
  "TaskUpdate",
  "TodoWrite",
  "UpdateGoal",
  "WebFetch",
  "WebSearch",
  "Write",
]);

export const POC_STATUS = Object.freeze({
  productionReady: false,
  verdict: "POC_ONLY",
  qoderAppSupport: "UNPROVED",
  providerCompatibility: "REQUIRES_LIVE_PROBE",
});

export class PocError extends Error {
  constructor(code) {
    super(code);
    this.name = "PocError";
    this.code = code;
  }
}

function fail(code) {
  throw new PocError(code);
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function safeErrorCode(error) {
  if (error instanceof PocError) {
    return error.code;
  }
  if (error && typeof error === "object" && error.name === "AbortError") {
    return "operation_timed_out";
  }
  return "unexpected_failure";
}

function assertBoundedString(value, label, maximum, pattern) {
  if (typeof value !== "string" || value.length === 0 || value.length > maximum) {
    fail(`invalid_${label}`);
  }
  if (value.trim() !== value || /[\u0000-\u001f\u007f]/u.test(value)) {
    fail(`invalid_${label}`);
  }
  if (pattern && !pattern.test(value)) {
    fail(`invalid_${label}`);
  }
  return value;
}

function validateModel(value) {
  return assertBoundedString(value, "model", 256);
}

function validateProvider(value) {
  return assertBoundedString(
    value,
    "provider",
    64,
    /^[A-Za-z0-9][A-Za-z0-9._-]*$/u,
  );
}

function validateStyle(value) {
  return assertBoundedString(
    value,
    "style",
    64,
    /^[A-Za-z0-9][A-Za-z0-9._-]*$/u,
  );
}

function validateSecret(value, label) {
  if (
    typeof value !== "string" ||
    value.length < 8 ||
    value.length > 8192 ||
    value.trim() !== value ||
    /[\r\n\u0000]/u.test(value)
  ) {
    fail(`invalid_${label}`);
  }
  return value;
}

function validateBaseUrl(value) {
  assertBoundedString(value, "url", 2048);
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    fail("invalid_url");
  }
  if (parsed.protocol !== "https:") {
    fail("url_requires_https");
  }
  if (parsed.username || parsed.password || parsed.search || parsed.hash) {
    fail("url_contains_forbidden_component");
  }
  parsed.pathname = parsed.pathname.replace(/\/+$/u, "") || "/";
  return parsed.toString().replace(/\/$/u, "");
}

function apiUrl(baseUrl, endpoint) {
  const parsed = new URL(`${baseUrl}/`);
  let path = parsed.pathname.replace(/\/+$/u, "");
  if (!path.endsWith("/v1")) {
    path = `${path}/v1`;
  }
  parsed.pathname = `${path}/${endpoint}`.replace(/\/{2,}/gu, "/");
  return parsed.toString();
}

async function validateRegularAbsoluteFile(filePath, label) {
  if (typeof filePath !== "string" || !isAbsolute(filePath)) {
    fail(`${label}_must_be_absolute`);
  }
  const normalized = resolve(filePath);
  let metadata;
  let canonical;
  try {
    metadata = await lstat(normalized);
    canonical = await realpath(normalized);
  } catch {
    fail(`${label}_unavailable`);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || canonical !== normalized) {
    fail(`${label}_must_be_regular_canonical_file`);
  }
  return normalized;
}

export async function readCredentialFile(filePath) {
  const normalized = await validateRegularAbsoluteFile(
    filePath,
    "credential_file",
  );
  let handle;
  try {
    handle = await open(
      normalized,
      fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
    );
    const before = await handle.stat();
    if (!before.isFile() || before.size <= 0 || before.size > MAX_CREDENTIAL_BYTES) {
      fail("invalid_credential_file_size");
    }
    if (process.platform !== "win32" && (before.mode & 0o077) !== 0) {
      fail("credential_file_permissions_too_broad");
    }
    const source = await handle.readFile({ encoding: "utf8" });
    const after = await handle.stat();
    if (
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      before.mtimeMs !== after.mtimeMs ||
      before.ctimeMs !== after.ctimeMs
    ) {
      fail("credential_file_changed_during_read");
    }

    let value;
    try {
      value = JSON.parse(source);
    } catch {
      fail("invalid_credential_json");
    }
    if (!isPlainObject(value)) {
      fail("invalid_credential_shape");
    }
    const keys = Object.keys(value).sort();
    if (keys.join("\u0000") !== ["_type", "key", "url"].sort().join("\u0000")) {
      fail("invalid_credential_fields");
    }

    return Object.freeze({
      type: validateProvider(value._type),
      key: validateSecret(value.key, "provider_key"),
      url: validateBaseUrl(value.url),
    });
  } finally {
    await handle?.close().catch(() => {});
  }
}

export function createModelPolicyResult({
  credential,
  model,
  provider,
  style,
}) {
  if (!credential || typeof credential !== "object") {
    fail("invalid_credential");
  }
  const selectedModel = validateModel(model);
  const selectedProvider = validateProvider(provider ?? credential.type);
  const selectedStyle = validateStyle(style ?? "openai");
  const selectedUrl = validateBaseUrl(credential.url);
  const key = validateSecret(credential.key, "provider_key");
  return {
    model: {
      model: selectedModel,
      provider: selectedProvider,
      api_key: key,
      url: selectedUrl,
      style: selectedStyle,
    },
  };
}

class LeakScanner {
  constructor(secrets) {
    this.secrets = [...new Set(secrets.filter((secret) => secret.length >= 8))];
    this.matchCount = 0;
    this.paths = new Set();
  }

  scan(label, input) {
    const bytes = Buffer.isBuffer(input) ? input : Buffer.from(String(input));
    for (const secret of this.secrets) {
      const needle = Buffer.from(secret);
      let offset = 0;
      let matched = false;
      while (offset <= bytes.length - needle.length) {
        const index = bytes.indexOf(needle, offset);
        if (index < 0) {
          break;
        }
        matched = true;
        this.matchCount += 1;
        offset = index + needle.length;
      }
      if (matched) {
        this.paths.add(label);
      }
    }
  }

  report() {
    const paths = [...this.paths].sort().slice(0, 64);
    return {
      leakMatchCount: this.matchCount,
      leakPathCount: this.paths.size,
      leakPaths: paths,
      pathListTruncated: this.paths.size > paths.length,
    };
  }
}

async function readBoundedResponse(response, label, scanner) {
  if (!response || typeof response.status !== "number") {
    fail("invalid_http_response");
  }
  const chunks = [];
  let byteCount = 0;
  if (response.body && typeof response.body.getReader === "function") {
    const reader = response.body.getReader();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }
        const chunk = Buffer.from(value);
        byteCount += chunk.length;
        if (byteCount > MAX_HTTP_RESPONSE_BYTES) {
          await reader.cancel().catch(() => {});
          fail("http_response_too_large");
        }
        chunks.push(chunk);
      }
    } finally {
      reader.releaseLock();
    }
  } else {
    const chunk = Buffer.from(await response.arrayBuffer());
    byteCount = chunk.length;
    if (byteCount > MAX_HTTP_RESPONSE_BYTES) {
      fail("http_response_too_large");
    }
    chunks.push(chunk);
  }
  const body = Buffer.concat(chunks);
  scanner.scan(label, body);
  if (!response.ok) {
    const error = new PocError("http_request_rejected");
    error.httpStatus = response.status;
    throw error;
  }
  let parsed;
  try {
    parsed = JSON.parse(body.toString("utf8"));
  } catch {
    fail("invalid_http_json");
  }
  if (!isPlainObject(parsed)) {
    fail("invalid_http_json_shape");
  }
  return { body: parsed, byteCount, status: response.status };
}

async function fetchJson({
  fetchImpl,
  url,
  key,
  method,
  body,
  timeoutMs,
  label,
  scanner,
}) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    let response;
    try {
      response = await fetchImpl(url, {
        method,
        headers: {
          Accept: "application/json",
          Authorization: `Bearer ${key}`,
          ...(body ? { "Content-Type": "application/json" } : {}),
        },
        ...(body ? { body: JSON.stringify(body) } : {}),
        redirect: "error",
        signal: controller.signal,
      });
    } catch (error) {
      if (error && typeof error === "object" && error.name === "AbortError") {
        fail("http_request_timed_out");
      }
      fail("http_request_failed");
    }
    return await readBoundedResponse(response, label, scanner);
  } finally {
    clearTimeout(timeout);
  }
}

export async function runDirectProbe({
  credential,
  model,
  fetchImpl = globalThis.fetch,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  scanner = new LeakScanner([credential.key]),
}) {
  if (typeof fetchImpl !== "function") {
    fail("fetch_unavailable");
  }
  const selectedModel = validateModel(model);
  const models = await fetchJson({
    fetchImpl,
    url: apiUrl(credential.url, "models"),
    key: credential.key,
    method: "GET",
    timeoutMs,
    label: "$direct/models-response",
    scanner,
  });
  if (!Array.isArray(models.body.data)) {
    fail("invalid_models_response");
  }
  const modelIds = models.body.data.flatMap((item) =>
    isPlainObject(item) && typeof item.id === "string" ? [item.id] : [],
  );

  const chat = await fetchJson({
    fetchImpl,
    url: apiUrl(credential.url, "chat/completions"),
    key: credential.key,
    method: "POST",
    body: {
      model: selectedModel,
      messages: [{ role: "user", content: FIXED_PROMPT }],
      max_tokens: 8,
      temperature: 0,
      stream: false,
    },
    timeoutMs,
    label: "$direct/chat-response",
    scanner,
  });
  if (!Array.isArray(chat.body.choices)) {
    fail("invalid_chat_response");
  }
  const contentChoiceCount = chat.body.choices.filter(
    (choice) =>
      isPlainObject(choice) &&
      isPlainObject(choice.message) &&
      typeof choice.message.content === "string" &&
      choice.message.content.length > 0,
  ).length;

  return {
    modelsRequest: {
      httpStatus: models.status,
      responseByteCount: models.byteCount,
      modelCount: modelIds.length,
      selectedModelPresent: modelIds.includes(selectedModel),
    },
    chatRequest: {
      httpStatus: chat.status,
      responseByteCount: chat.byteCount,
      choiceCount: chat.body.choices.length,
      contentChoiceCount,
    },
  };
}

function pathInside(root, candidate) {
  const rel = relative(root, candidate);
  return rel === "" || (!rel.startsWith(`..${sep}`) && rel !== "..");
}

async function scanOutputTree(root, scanner) {
  let fileCount = 0;
  let byteCount = 0;
  let unsafeEntryCount = 0;
  let complete = true;

  async function visit(directory) {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      if (!complete) {
        return;
      }
      const path = join(directory, entry.name);
      const rel = relative(root, path);
      if (!pathInside(root, path)) {
        unsafeEntryCount += 1;
        complete = false;
        continue;
      }
      if (entry.isSymbolicLink()) {
        unsafeEntryCount += 1;
        complete = false;
        continue;
      }
      if (entry.isDirectory()) {
        await visit(path);
        continue;
      }
      if (!entry.isFile()) {
        unsafeEntryCount += 1;
        complete = false;
        continue;
      }
      fileCount += 1;
      if (fileCount > MAX_SCAN_FILES) {
        complete = false;
        return;
      }
      const handle = await open(
        path,
        fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0),
      );
      try {
        const metadata = await handle.stat();
        if (!metadata.isFile()) {
          unsafeEntryCount += 1;
          complete = false;
          return;
        }
        byteCount += metadata.size;
        if (byteCount > MAX_SCAN_BYTES) {
          complete = false;
          return;
        }
        scanner.scan(rel, await handle.readFile());
      } finally {
        await handle.close();
      }
    }
  }

  await visit(root);
  return { complete, fileCount, byteCount, unsafeEntryCount };
}

export function buildIsolatedEnvironment({
  qoderConfigDir,
  emptyCwd,
  tempDir,
  homeDir,
  nodePath = process.execPath,
}) {
  for (const path of [qoderConfigDir, emptyCwd, tempDir, homeDir]) {
    if (!isAbsolute(path)) {
      fail("isolation_path_must_be_absolute");
    }
  }
  return Object.freeze({
    HOME: homeDir,
    USER: "mallard-poc",
    LOGNAME: "mallard-poc",
    PATH: `${dirname(nodePath)}:/usr/bin:/bin`,
    TMPDIR: tempDir,
    XDG_CACHE_HOME: join(homeDir, ".cache"),
    XDG_CONFIG_HOME: join(homeDir, ".config"),
    XDG_DATA_HOME: join(homeDir, ".local", "share"),
    XDG_STATE_HOME: join(homeDir, ".local", "state"),
    QODER_CONFIG_DIR: qoderConfigDir,
    NO_COLOR: "1",
    TERM: "dumb",
    LANG: "C",
  });
}

function appendBounded(chunks, chunk, state) {
  const bytes = Buffer.from(chunk);
  state.byteCount += bytes.length;
  if (state.byteCount > MAX_CHILD_OUTPUT_BYTES) {
    state.exceeded = true;
    return;
  }
  chunks.push(bytes);
}

async function runQoderChild({
  sdkModule,
  qoderCli,
  credential,
  model,
  provider,
  style,
  pat,
  isolated,
  scanner,
  timeoutMs,
  spawnImpl = spawn,
}) {
  const scriptPath = fileURLToPath(import.meta.url);
  const child = spawnImpl(process.execPath, [scriptPath, INTERNAL_RUNNER_ARGUMENT], {
    cwd: isolated.emptyCwd,
    env: isolated.env,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout = [];
  const stderr = [];
  const stdoutState = { byteCount: 0, exceeded: false };
  const stderrState = { byteCount: 0, exceeded: false };
  child.stdout.on("data", (chunk) => {
    appendBounded(stdout, chunk, stdoutState);
    if (stdoutState.exceeded) {
      child.kill("SIGTERM");
    }
  });
  child.stderr.on("data", (chunk) => {
    appendBounded(stderr, chunk, stderrState);
    if (stderrState.exceeded) {
      child.kill("SIGTERM");
    }
  });

  const payload = JSON.stringify({
    sdkModule,
    qoderCli,
    credential,
    model,
    provider,
    style,
    pat,
    qoderConfigDir: isolated.qoderConfigDir,
    emptyCwd: isolated.emptyCwd,
    env: isolated.env,
  });
  child.stdin.end(payload);

  const exit = await new Promise((resolveExit) => {
    let settled = false;
    let forceTimer;
    const timer = setTimeout(() => {
      if (!settled) {
        child.kill("SIGTERM");
        forceTimer = setTimeout(() => {
          if (!settled) {
            child.kill("SIGKILL");
          }
        }, 2_000);
      }
    }, timeoutMs);
    child.once("error", () => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        clearTimeout(forceTimer);
        resolveExit({ code: null, signal: null, spawnError: true });
      }
    });
    child.once("close", (code, signal) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        clearTimeout(forceTimer);
        resolveExit({ code, signal, spawnError: false });
      }
    });
  });

  const stdoutBytes = Buffer.concat(stdout);
  const stderrBytes = Buffer.concat(stderr);
  scanner.scan("$qoder-runner/stdout", stdoutBytes);
  scanner.scan("$qoder-runner/stderr", stderrBytes);
  if (stdoutState.exceeded || stderrState.exceeded) {
    fail("qoder_runner_output_too_large");
  }
  if (exit.spawnError) {
    fail("qoder_runner_spawn_failed");
  }
  if (exit.signal !== null) {
    fail("qoder_runner_timed_out");
  }
  if (exit.code !== 0) {
    fail("qoder_runner_failed");
  }
  let result;
  try {
    result = JSON.parse(stdoutBytes.toString("utf8"));
  } catch {
    fail("qoder_runner_protocol_error");
  }
  if (!isPlainObject(result) || result.schemaVersion !== 1 || result.ok !== true) {
    fail("qoder_runner_rejected");
  }
  return {
    resolveModelCallCount:
      Number.isSafeInteger(result.resolveModelCallCount) &&
      result.resolveModelCallCount >= 0
        ? result.resolveModelCallCount
        : 0,
    messageCount:
      Number.isSafeInteger(result.messageCount) && result.messageCount >= 0
        ? result.messageCount
        : 0,
    resultMessageCount:
      Number.isSafeInteger(result.resultMessageCount) &&
      result.resultMessageCount >= 0
        ? result.resultMessageCount
        : 0,
    toolUseMessageCount:
      Number.isSafeInteger(result.toolUseMessageCount) &&
      result.toolUseMessageCount >= 0
        ? result.toolUseMessageCount
        : 0,
    controls: {
      toolsDisabled: result.controls?.toolsDisabled === true,
      sessionPersistenceDisabled:
        result.controls?.sessionPersistenceDisabled === true,
      debugLoggingDisabled: result.controls?.debugLoggingDisabled === true,
      broadEnvironmentDisabled:
        result.controls?.broadEnvironmentDisabled === true,
    },
  };
}

function validateTimeout(value) {
  const timeout = value ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isSafeInteger(timeout) || timeout < 1_000 || timeout > 300_000) {
    fail("invalid_timeout");
  }
  return timeout;
}

async function makeIsolation(onTempRoot) {
  const createdRoot = await mkdtemp(join(tmpdir(), POC_PREFIX));
  const root = await realpath(createdRoot);
  try {
    await chmod(root, 0o700);
    const paths = {
      root,
      qoderConfigDir: join(root, "qoder-config"),
      emptyCwd: join(root, "empty-cwd"),
      tempDir: join(root, "tmp"),
      homeDir: join(root, "home"),
    };
    for (const path of Object.values(paths).slice(1)) {
      await mkdir(path, { mode: 0o700 });
    }
    onTempRoot?.(root);
    return paths;
  } catch (error) {
    await rm(root, { recursive: true, force: false }).catch(() => {});
    throw error;
  }
}

async function removeIsolation(root) {
  if (
    typeof root !== "string" ||
    !isAbsolute(root) ||
    !basename(root).startsWith(POC_PREFIX)
  ) {
    fail("unsafe_cleanup_target");
  }
  const canonical = await realpath(root);
  if (canonical !== resolve(root)) {
    fail("unsafe_cleanup_target");
  }
  const metadata = await lstat(root);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    fail("unsafe_cleanup_target");
  }
  await rm(root, { recursive: true, force: false });
}

export async function executeProbe(
  {
    mode,
    credentialFile,
    model,
    provider,
    style = "openai",
    sdkModule,
    qoderCli,
    timeoutMs,
  },
  {
    env = process.env,
    fetchImpl = globalThis.fetch,
    spawnImpl = spawn,
    onTempRoot,
  } = {},
) {
  const selectedMode = ["direct", "qoder", "all"].includes(mode)
    ? mode
    : fail("invalid_mode");
  const selectedModel = validateModel(model);
  const timeout = validateTimeout(timeoutMs);
  let credential;
  try {
    credential = await readCredentialFile(credentialFile);
  } catch (error) {
    return {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: false,
      error: { code: safeErrorCode(error) },
      cleanup: { attempted: false, complete: true },
    };
  }
  const selectedProvider = validateProvider(provider ?? credential.type);
  const selectedStyle = validateStyle(style);
  const patRequired = selectedMode === "qoder" || selectedMode === "all";
  let pat;
  if (patRequired) {
    try {
      pat = validateSecret(env[QODER_PAT_ENV], "qoder_pat");
    } catch (error) {
      return {
        schemaVersion: 1,
        ...POC_STATUS,
        ok: false,
        error: { code: safeErrorCode(error) },
        cleanup: { attempted: false, complete: true },
      };
    }
  }

  const scanner = new LeakScanner([credential.key, ...(pat ? [pat] : [])]);
  let isolation;
  let operation = {};
  let errorCode;
  let scan = {
    complete: false,
    fileCount: 0,
    byteCount: 0,
    unsafeEntryCount: 0,
  };
  let cleanupComplete = false;
  try {
    isolation = await makeIsolation(onTempRoot);
    const isolatedEnv = buildIsolatedEnvironment({
      ...isolation,
      nodePath: process.execPath,
    });
    const isolated = { ...isolation, env: isolatedEnv };

    if (selectedMode === "direct" || selectedMode === "all") {
      operation.direct = await runDirectProbe({
        credential,
        model: selectedModel,
        fetchImpl,
        timeoutMs: timeout,
        scanner,
      });
    }
    if (selectedMode === "qoder" || selectedMode === "all") {
      const checkedSdkModule = await validateRegularAbsoluteFile(
        sdkModule,
        "sdk_module",
      );
      const checkedQoderCli = await validateRegularAbsoluteFile(
        qoderCli,
        "qodercli_bundle",
      );
      operation.qoder = await runQoderChild({
        sdkModule: checkedSdkModule,
        qoderCli: checkedQoderCli,
        credential,
        model: selectedModel,
        provider: selectedProvider,
        style: selectedStyle,
        pat,
        isolated,
        scanner,
        timeoutMs: timeout,
        spawnImpl,
      });
    }
  } catch (error) {
    errorCode = safeErrorCode(error);
  } finally {
    if (isolation) {
      try {
        scan = await scanOutputTree(isolation.root, scanner);
      } catch {
        scan.complete = false;
        errorCode ??= "output_scan_failed";
      }
      try {
        await removeIsolation(isolation.root);
        cleanupComplete = true;
      } catch {
        errorCode ??= "cleanup_failed";
      }
    }
  }

  const leakReport = scanner.report();
  if (leakReport.leakMatchCount > 0) {
    errorCode = "credential_leak_detected";
  }
  if (!scan.complete || scan.unsafeEntryCount > 0) {
    errorCode ??= "output_scan_incomplete";
  }
  if (!cleanupComplete) {
    errorCode ??= "cleanup_failed";
  }

  const result = {
    schemaVersion: 1,
    ...POC_STATUS,
    ok: errorCode === undefined,
    mode: selectedMode,
    ...(errorCode ? { error: { code: errorCode } } : { operation }),
    outputScan: { ...scan, ...leakReport },
    cleanup: { attempted: true, complete: cleanupComplete },
  };
  const serialized = JSON.stringify(result);
  if (
    serialized.includes(credential.key) ||
    (pat !== undefined && serialized.includes(pat))
  ) {
    return {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: false,
      error: { code: "sanitization_invariant_failed" },
      cleanup: { attempted: true, complete: cleanupComplete },
    };
  }
  return result;
}

async function readStdinBounded() {
  const chunks = [];
  let byteCount = 0;
  for await (const chunk of process.stdin) {
    const bytes = Buffer.from(chunk);
    byteCount += bytes.length;
    if (byteCount > MAX_CREDENTIAL_BYTES) {
      fail("runner_input_too_large");
    }
    chunks.push(bytes);
  }
  let parsed;
  try {
    parsed = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    fail("runner_input_invalid");
  }
  if (!isPlainObject(parsed)) {
    fail("runner_input_invalid");
  }
  return parsed;
}

function safeMessageType(message) {
  if (
    isPlainObject(message) &&
    typeof message.type === "string" &&
    /^[a-z][a-z0-9_-]{0,63}$/u.test(message.type)
  ) {
    return message.type;
  }
  return "other";
}

async function internalQoderRunner() {
  const input = await readStdinBounded();
  const credential = Object.freeze({
    type: validateProvider(input.credential?.type),
    key: validateSecret(input.credential?.key, "provider_key"),
    url: validateBaseUrl(input.credential?.url),
  });
  const model = validateModel(input.model);
  const provider = validateProvider(input.provider);
  const style = validateStyle(input.style);
  const pat = validateSecret(input.pat, "qoder_pat");
  const sdkModule = await validateRegularAbsoluteFile(input.sdkModule, "sdk_module");
  const qoderCli = await validateRegularAbsoluteFile(
    input.qoderCli,
    "qodercli_bundle",
  );
  if (!isPlainObject(input.env)) {
    fail("runner_environment_invalid");
  }
  const allowedEnvironmentKeys = new Set([
    "HOME",
    "USER",
    "LOGNAME",
    "PATH",
    "TMPDIR",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "QODER_CONFIG_DIR",
    "NO_COLOR",
    "TERM",
    "LANG",
  ]);
  if (Object.keys(input.env).some((key) => !allowedEnvironmentKeys.has(key))) {
    fail("runner_environment_invalid");
  }

  const sdk = await import(pathToFileURL(sdkModule).href);
  if (typeof sdk.query !== "function" || typeof sdk.accessToken !== "function") {
    fail("sdk_contract_unavailable");
  }
  const scanner = new LeakScanner([credential.key, pat]);
  let resolveModelCallCount = 0;
  let messageCount = 0;
  let resultMessageCount = 0;
  let toolUseMessageCount = 0;
  const query = sdk.query({
    prompt: FIXED_PROMPT,
    options: {
      auth: sdk.accessToken(pat),
      cwd: input.emptyCwd,
      env: input.env,
      pathToQoderCLIExecutable: qoderCli,
      resolveModel: () => {
        resolveModelCallCount += 1;
        return createModelPolicyResult({
          credential,
          model,
          provider,
          style,
        });
      },
      resolveModelTimeoutMs: 2_000,
      tools: [],
      allowedTools: [],
      disallowedTools: [...QODER_TOOL_DENYLIST],
      settingSources: [],
      settings: {},
      skills: [],
      extensions: [],
      plugins: [],
      mcpServers: {},
      allowedMcpServerNames: [],
      strictMcpConfig: true,
      persistSession: false,
      debug: false,
      enableFileCheckpointing: false,
      securityScan: {
        l1StaticCheck: false,
        l2LightweightScan: false,
        l3DeepScan: false,
      },
      includePartialMessages: false,
      includeHookEvents: false,
      promptSuggestions: false,
      permissionMode: "default",
      maxTurns: 1,
      systemPrompt: FIXED_PROMPT,
      stderr: (data) => scanner.scan("$sdk/stderr", data),
    },
  });
  try {
    for await (const message of query) {
      messageCount += 1;
      const serialized = JSON.stringify(message);
      scanner.scan("$sdk/message", serialized);
      const type = safeMessageType(message);
      if (type === "result") {
        resultMessageCount += 1;
      }
      if (type === "tool_use" || serialized.includes('"tool_use"')) {
        toolUseMessageCount += 1;
      }
    }
  } finally {
    if (query && typeof query.close === "function") {
      await query.close().catch(() => {});
    }
  }
  if (scanner.report().leakMatchCount > 0) {
    fail("credential_leak_detected");
  }
  return {
    schemaVersion: 1,
    ok: true,
    resolveModelCallCount,
    messageCount,
    resultMessageCount,
    toolUseMessageCount,
    controls: {
      toolsDisabled: true,
      sessionPersistenceDisabled: true,
      debugLoggingDisabled: true,
      broadEnvironmentDisabled: true,
    },
  };
}

function parseArguments(argv) {
  const [mode, ...rest] = argv;
  const values = { mode };
  const names = new Map([
    ["--credentials", "credentialFile"],
    ["--model", "model"],
    ["--provider", "provider"],
    ["--style", "style"],
    ["--sdk-module", "sdkModule"],
    ["--qodercli", "qoderCli"],
    ["--timeout-ms", "timeoutMs"],
  ]);
  for (let index = 0; index < rest.length; index += 2) {
    const name = rest[index];
    const value = rest[index + 1];
    const property = names.get(name);
    if (!property || value === undefined) {
      fail("invalid_arguments");
    }
    if (Object.hasOwn(values, property)) {
      fail("duplicate_argument");
    }
    values[property] = property === "timeoutMs" ? Number(value) : value;
  }
  return values;
}

async function main() {
  if (process.argv[2] === INTERNAL_RUNNER_ARGUMENT) {
    try {
      const result = await internalQoderRunner();
      process.stdout.write(JSON.stringify(result));
    } catch (error) {
      process.stdout.write(
        JSON.stringify({
          schemaVersion: 1,
          ok: false,
          error: { code: safeErrorCode(error) },
        }),
      );
      process.exitCode = 1;
    }
    return;
  }

  let result;
  try {
    result = await executeProbe(parseArguments(process.argv.slice(2)));
  } catch (error) {
    result = {
      schemaVersion: 1,
      ...POC_STATUS,
      ok: false,
      error: { code: safeErrorCode(error) },
      cleanup: { attempted: false, complete: true },
    };
  }
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  if (!result.ok) {
    process.exitCode = 1;
  }
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  await main();
}
