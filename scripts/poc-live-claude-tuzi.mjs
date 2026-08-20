import { spawn, spawnSync } from "node:child_process";
import { constants } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readdir,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const connectionPath = join(projectRoot, ".env.local");
const paidUseGate = "MALLARD_RUN_LIVE_CLAUDE_TUZI_POC";
const requiredClaudeVersion = "2.1.235";
const model = "claude-sonnet-5";
const effort = "low";
const expectedMarker = "MALLARD_CLAUDE_TUZI_POC_OK";
const maximumBudgetUsd = 0.05;
const maximumConnectionBytes = 4_096;
const maximumKeyBytes = 1_024;
const maximumCatalogBytes = 4 * 1024 * 1024;
const maximumProcessOutputBytes = 2 * 1024 * 1024;
const maximumTemporaryFileBytes = 8 * 1024 * 1024;
const maximumTemporaryTreeBytes = 32 * 1024 * 1024;
const maximumTemporaryEntries = 4_096;
const catalogTimeoutMs = 20_000;
const claudeTimeoutMs = 120_000;
const versionTimeoutMs = 15_000;

class PocError extends Error {
  constructor(code) {
    super(code);
    this.name = "PocError";
    this.code = code;
  }
}

function fail(code) {
  console.error(`Claude Code Tuzi POC failed: ${code}.`);
  process.exitCode = 1;
}

function stableMetadata(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.size === right.size &&
    left.mtimeMs === right.mtimeMs &&
    left.ctimeMs === right.ctimeMs
  );
}

function isPrivateRegularFile(metadata, maximumBytes, allowEmpty = false) {
  return (
    !metadata.isSymbolicLink() &&
    metadata.isFile() &&
    (allowEmpty ? metadata.size >= 0 : metadata.size >= 1) &&
    metadata.size <= maximumBytes &&
    (process.platform === "win32" || (metadata.mode & 0o7777) === 0o600)
  );
}

async function readStableRegularFile(
  path,
  maximumBytes,
  { requirePrivateMode = false, allowEmpty = false } = {},
) {
  const before = await lstat(path);
  const validBefore = requirePrivateMode
    ? isPrivateRegularFile(before, maximumBytes, allowEmpty)
    : !before.isSymbolicLink() &&
      before.isFile() &&
      (allowEmpty ? before.size >= 0 : before.size >= 1) &&
      before.size <= maximumBytes;
  if (!validBefore) {
    throw new PocError("unsafe_file");
  }

  const noFollow =
    typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
  const closeOnExec =
    typeof constants.O_CLOEXEC === "number" ? constants.O_CLOEXEC : 0;
  const handle = await open(
    path,
    constants.O_RDONLY | noFollow | closeOnExec,
  );
  try {
    const descriptorBefore = await handle.stat();
    if (!stableMetadata(before, descriptorBefore)) {
      throw new PocError("file_changed");
    }
    const bytes = await handle.readFile();
    const descriptorAfter = await handle.stat();
    const after = await lstat(path);
    if (
      bytes.length !== descriptorBefore.size ||
      bytes.length > maximumBytes ||
      !stableMetadata(descriptorBefore, descriptorAfter) ||
      !stableMetadata(descriptorAfter, after)
    ) {
      throw new PocError("file_changed");
    }
    return bytes;
  } finally {
    await handle.close();
  }
}

function normalizeTuziBaseUrl(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value !== value.trim() ||
    Buffer.byteLength(value, "utf8") > 2_048 ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(value)
  ) {
    return undefined;
  }

  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    return undefined;
  }
  if (
    parsed.protocol !== "https:" ||
    parsed.hostname !== "api.tu-zi.com" ||
    parsed.port !== "" ||
    parsed.username !== "" ||
    parsed.password !== "" ||
    parsed.search !== "" ||
    parsed.hash !== "" ||
    (parsed.pathname !== "/" && parsed.pathname !== "/v1")
  ) {
    return undefined;
  }
  return parsed.origin;
}

async function loadConnection() {
  let bytes;
  try {
    bytes = await readStableRegularFile(connectionPath, maximumConnectionBytes, {
      requirePrivateMode: true,
    });
  } catch {
    throw new PocError("connection_file_invalid");
  }

  const text = bytes.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(bytes)) {
    throw new PocError("connection_file_invalid_utf8");
  }

  let document;
  try {
    document = JSON.parse(text);
  } catch {
    throw new PocError("connection_file_invalid_json");
  }
  if (document === null || typeof document !== "object" || Array.isArray(document)) {
    throw new PocError("connection_document_invalid");
  }
  const fields = Object.keys(document).sort();
  if (
    fields.length !== 3 ||
    fields[0] !== "_type" ||
    fields[1] !== "key" ||
    fields[2] !== "url" ||
    document._type !== "newapi_channel_conn"
  ) {
    throw new PocError("connection_document_invalid");
  }
  if (
    typeof document.key !== "string" ||
    document.key.length === 0 ||
    document.key !== document.key.trim() ||
    Buffer.byteLength(document.key, "utf8") > maximumKeyBytes ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(document.key)
  ) {
    throw new PocError("connection_key_invalid");
  }
  const baseUrl = normalizeTuziBaseUrl(document.url);
  if (baseUrl === undefined) {
    throw new PocError("connection_url_not_tuzi_root");
  }
  return { baseUrl, key: document.key };
}

function isolatedBaseEnvironment(root, claudeConfig) {
  return {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: root,
    USERPROFILE: root,
    CLAUDE_CONFIG_DIR: claudeConfig,
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    DISABLE_TELEMETRY: "1",
    DISABLE_ERROR_REPORTING: "1",
    DISABLE_UPDATES: "1",
  };
}

async function verifyClaudeVersion() {
  let preflightRoot;
  try {
    preflightRoot = await mkdtemp(
      join(tmpdir(), "mallard-claude-tuzi-version-"),
    );
    await chmod(preflightRoot, 0o700);
    const home = join(preflightRoot, "home");
    const claudeConfig = join(preflightRoot, "claude-config");
    await Promise.all([
      mkdir(home, { mode: 0o700 }),
      mkdir(claudeConfig, { mode: 0o700 }),
    ]);
    const result = spawnSync("claude", ["--version"], {
      cwd: preflightRoot,
      env: isolatedBaseEnvironment(home, claudeConfig),
      encoding: "utf8",
      input: "",
      maxBuffer: 64 * 1024,
      timeout: versionTimeoutMs,
      killSignal: "SIGKILL",
    });
    if (
      result.error ||
      result.status !== 0 ||
      result.signal !== null ||
      result.stderr !== "" ||
      result.stdout.trim() !== `${requiredClaudeVersion} (Claude Code)`
    ) {
      throw new PocError("claude_version_mismatch");
    }
  } finally {
    if (preflightRoot !== undefined) {
      await rm(preflightRoot, { recursive: true, force: true });
    }
  }
}

async function readBoundedResponse(response, maximumBytes) {
  const contentLength = response.headers.get("content-length");
  if (
    contentLength !== null &&
    (!/^\d+$/u.test(contentLength) || Number(contentLength) > maximumBytes)
  ) {
    throw new PocError("catalog_response_too_large");
  }
  if (response.body === null) {
    throw new PocError("catalog_response_empty");
  }

  const reader = response.body.getReader();
  const chunks = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      length += value.byteLength;
      if (length > maximumBytes) {
        throw new PocError("catalog_response_too_large");
      }
      chunks.push(Buffer.from(value));
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  return Buffer.concat(chunks, length);
}

async function assertModelVisible(baseUrl, key) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), catalogTimeoutMs);
  try {
    const response = await fetch(new URL("/v1/models", baseUrl), {
      method: "GET",
      headers: {
        Accept: "application/json",
        Authorization: `Bearer ${key}`,
      },
      redirect: "error",
      signal: controller.signal,
    });
    if (response.status !== 200) {
      await response.body?.cancel().catch(() => {});
      throw new PocError(`catalog_http_${response.status}`);
    }

    const bytes = await readBoundedResponse(response, maximumCatalogBytes);
    if (containsBytes(bytes, Buffer.from(key, "utf8"))) {
      throw new PocError("credential_leaked_to_catalog_response");
    }
    let document;
    try {
      document = JSON.parse(bytes.toString("utf8"));
    } catch {
      throw new PocError("catalog_response_invalid");
    }
    const entries = Array.isArray(document?.data) ? document.data : undefined;
    if (entries === undefined || entries.length > 20_000) {
      throw new PocError("catalog_response_invalid");
    }
    const visible = entries.some(
      (entry) =>
        entry !== null &&
        typeof entry === "object" &&
        !Array.isArray(entry) &&
        entry.id === model,
    );
    if (!visible) {
      throw new PocError("exact_model_not_visible");
    }
  } catch (error) {
    if (error instanceof PocError) {
      throw error;
    }
    throw new PocError("catalog_request_failed");
  } finally {
    clearTimeout(timer);
  }
}

function runBoundedProcess(command, arguments_, options) {
  return new Promise((resolvePromise, rejectPromise) => {
    let stdoutLength = 0;
    let stderrLength = 0;
    const stdout = [];
    const stderr = [];
    let outputExceeded = false;
    let timedOut = false;
    let settled = false;

    const child = spawn(command, arguments_, {
      cwd: options.cwd,
      env: options.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, options.timeoutMs);

    function collect(chunks, chunk, kind) {
      if (kind === "stdout") {
        stdoutLength += chunk.length;
        if (stdoutLength > options.maximumOutputBytes) {
          outputExceeded = true;
          child.kill("SIGKILL");
          return;
        }
      } else {
        stderrLength += chunk.length;
        if (stderrLength > options.maximumOutputBytes) {
          outputExceeded = true;
          child.kill("SIGKILL");
          return;
        }
      }
      chunks.push(Buffer.from(chunk));
    }

    child.stdout.on("data", (chunk) => collect(stdout, chunk, "stdout"));
    child.stderr.on("data", (chunk) => collect(stderr, chunk, "stderr"));
    child.on("error", () => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        rejectPromise(new PocError("claude_process_start_failed"));
      }
    });
    child.on("close", (status, signal) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      resolvePromise({
        status,
        signal,
        stdout: Buffer.concat(stdout),
        stderr: Buffer.concat(stderr),
        outputExceeded,
        timedOut,
      });
    });
  });
}

function containsBytes(haystack, needle) {
  return needle.length > 0 && haystack.indexOf(needle) !== -1;
}

function classifyClaudeFailure(stdout, stderr) {
  let structured = "";
  try {
    const output = JSON.parse(stdout.toString("utf8"));
    if (typeof output?.result === "string") {
      structured = output.result;
    } else if (typeof output?.error?.message === "string") {
      structured = output.error.message;
    }
  } catch {
    structured = "";
  }
  const text = `${structured}\n${stderr.toString("utf8")}`.toLowerCase();
  if (text.includes("401") || text.includes("authentication")) {
    return "claude_authentication_rejected";
  }
  if (text.includes("403") || text.includes("permission")) {
    return "claude_permission_rejected";
  }
  if (text.includes("402") || text.includes("balance") || text.includes("billing")) {
    return "claude_billing_rejected";
  }
  if (
    text.includes("model") &&
    (text.includes("not found") ||
      text.includes("unsupported") ||
      text.includes("unavailable"))
  ) {
    return "claude_model_unavailable";
  }
  if (text.includes("404") || text.includes("not found")) {
    return "claude_endpoint_or_model_not_found";
  }
  if (
    text.includes("400") ||
    text.includes("422") ||
    text.includes("invalid_request") ||
    text.includes("invalid request")
  ) {
    return "claude_request_contract_rejected";
  }
  if (text.includes("429") || text.includes("rate limit")) {
    return "claude_rate_limited";
  }
  if (text.includes("529") || text.includes("overload")) {
    return "claude_provider_overloaded";
  }
  if (text.includes("overload") || /\b5\d\d\b/u.test(text)) {
    return "claude_provider_transient_failure";
  }
  if (text.includes("budget")) {
    return "claude_budget_rejected";
  }
  return "claude_process_failed";
}

async function inspectTemporaryTree(root, sessionId, key, requireTranscript) {
  const stack = [root];
  const files = [];
  const keyBytes = Buffer.from(key, "utf8");
  let entriesSeen = 0;
  let bytesSeen = 0;

  while (stack.length > 0) {
    const directory = stack.pop();
    const entries = await readdir(directory, { withFileTypes: true });
    for (const entry of entries) {
      entriesSeen += 1;
      if (entriesSeen > maximumTemporaryEntries) {
        throw new PocError("temporary_tree_too_large");
      }
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) {
        throw new PocError("temporary_tree_symlink");
      }
      if (entry.isDirectory()) {
        stack.push(path);
        continue;
      }
      if (!entry.isFile()) {
        throw new PocError("temporary_tree_special_file");
      }
      const bytes = await readStableRegularFile(path, maximumTemporaryFileBytes, {
        allowEmpty: true,
      });
      bytesSeen += bytes.length;
      if (bytesSeen > maximumTemporaryTreeBytes) {
        throw new PocError("temporary_tree_too_large");
      }
      if (containsBytes(bytes, keyBytes)) {
        throw new PocError("credential_leaked_to_temporary_tree");
      }
      files.push({ path, bytes });
    }
  }

  const transcripts = files.filter(
    ({ path }) => basename(path) === `${sessionId}.jsonl`,
  );
  if (transcripts.length === 0 && !requireTranscript) {
    return undefined;
  }
  if (transcripts.length !== 1) {
    throw new PocError("native_transcript_missing");
  }

  const transcriptMetadata = await lstat(transcripts[0].path);
  if (
    process.platform !== "win32" &&
    (transcriptMetadata.mode & 0o7777) !== 0o600
  ) {
    throw new PocError("native_transcript_mode_invalid");
  }

  const lines = transcripts[0].bytes
    .toString("utf8")
    .split("\n")
    .filter((line) => line.length > 0);
  if (lines.length === 0 || lines.length > 1_024) {
    throw new PocError("native_transcript_invalid");
  }
  let markerFound = false;
  const transcriptModels = new Set();
  for (const line of lines) {
    let record;
    try {
      record = JSON.parse(line);
    } catch {
      throw new PocError("native_transcript_invalid");
    }
    if (record?.sessionId !== undefined && record.sessionId !== sessionId) {
      throw new PocError("native_transcript_session_mismatch");
    }
    const serialized = JSON.stringify(record);
    if (serialized.includes(expectedMarker)) {
      markerFound = true;
    }
    if (typeof record?.message?.model === "string") {
      transcriptModels.add(record.message.model);
    }
  }
  if (!markerFound) {
    throw new PocError("native_transcript_marker_missing");
  }
  return transcriptModels;
}

async function waitForPersistedTranscript(root, sessionId, key) {
  const deadline = Date.now() + 5_000;
  let lastError;
  do {
    try {
      return await inspectTemporaryTree(root, sessionId, key, true);
    } catch (error) {
      if (
        !(error instanceof PocError) ||
        ![
          "native_transcript_missing",
          "native_transcript_invalid",
          "native_transcript_marker_missing",
        ].includes(error.code)
      ) {
        throw error;
      }
      lastError = error;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
  } while (Date.now() < deadline);
  throw lastError ?? new PocError("native_transcript_missing");
}

function assertedObservedModel(output, transcriptModels) {
  const modelUsage = output?.modelUsage ?? output?.model_usage;
  const observed = new Set(transcriptModels);
  if (modelUsage !== null && typeof modelUsage === "object" && !Array.isArray(modelUsage)) {
    for (const name of Object.keys(modelUsage)) {
      observed.add(name);
    }
  }
  if (
    observed.size === 0 ||
    [...observed].some(
      (name) => name !== model && !name.startsWith(`${model}-`),
    )
  ) {
    throw new PocError("observed_model_mismatch");
  }
  return [...observed].sort();
}

function validateClaudeOutput(stdout, expectedSessionId) {
  let output;
  try {
    output = JSON.parse(stdout.toString("utf8"));
  } catch {
    throw new PocError("claude_output_invalid_json");
  }
  if (
    output?.type !== "result" ||
    output?.subtype !== "success" ||
    output?.is_error !== false ||
    output?.session_id !== expectedSessionId ||
    output?.result !== expectedMarker ||
    output?.num_turns !== 1
  ) {
    throw new PocError("claude_output_contract_mismatch");
  }
  if (
    typeof output.total_cost_usd !== "number" ||
    !Number.isFinite(output.total_cost_usd) ||
    output.total_cost_usd < 0 ||
    output.total_cost_usd > maximumBudgetUsd
  ) {
    throw new PocError("claude_budget_contract_mismatch");
  }
  return output;
}

async function runPoc() {
  if (process.env[paidUseGate] !== "1") {
    throw new PocError("paid_use_gate_missing");
  }

  await verifyClaudeVersion();
  const connection = await loadConnection();
  await assertModelVisible(connection.baseUrl, connection.key);

  let disposableRoot;
  try {
    disposableRoot = await mkdtemp(join(tmpdir(), "mallard-claude-tuzi-poc-"));
    await chmod(disposableRoot, 0o700);
    const home = join(disposableRoot, "home");
    const claudeConfig = join(disposableRoot, "claude-config");
    const project = join(disposableRoot, "project");
    const processTemp = join(disposableRoot, "tmp");
    await Promise.all([
      mkdir(home, { mode: 0o700 }),
      mkdir(claudeConfig, { mode: 0o700 }),
      mkdir(project, { mode: 0o700 }),
      mkdir(processTemp, { mode: 0o700 }),
    ]);

    const sessionId = randomUUID();
    const prompt = `Return exactly ${expectedMarker}.`;
    const systemPrompt = "Return only the exact marker requested by the user.";
    const arguments_ = [
      "--bare",
      "--print",
      "--output-format",
      "json",
      "--model",
      model,
      "--effort",
      effort,
      "--max-budget-usd",
      maximumBudgetUsd.toFixed(2),
      "--max-turns",
      "1",
      "--permission-mode",
      "dontAsk",
      "--tools=",
      "--no-chrome",
      "--system-prompt",
      systemPrompt,
      "--session-id",
      sessionId,
      prompt,
    ];
    if (
      arguments_.length > 32 ||
      arguments_.some(
        (argument) =>
          Buffer.byteLength(argument, "utf8") > 1_024 ||
          argument.includes(connection.key),
      )
    ) {
      throw new PocError("claude_arguments_invalid");
    }

    const environment = {
      ...isolatedBaseEnvironment(home, claudeConfig),
      TMPDIR: processTemp,
      ANTHROPIC_BASE_URL: connection.baseUrl,
      ANTHROPIC_API_KEY: connection.key,
      CLAUDE_CODE_MAX_RETRIES: "0",
      CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK: "1",
      CLAUDE_CODE_MAX_OUTPUT_TOKENS: "1024",
      API_TIMEOUT_MS: String(claudeTimeoutMs),
    };
    const result = await runBoundedProcess("claude", arguments_, {
      cwd: project,
      env: environment,
      timeoutMs: claudeTimeoutMs,
      maximumOutputBytes: maximumProcessOutputBytes,
    });
    const keyBytes = Buffer.from(connection.key, "utf8");
    if (
      containsBytes(result.stdout, keyBytes) ||
      containsBytes(result.stderr, keyBytes)
    ) {
      throw new PocError("credential_leaked_to_process_output");
    }
    const transcriptModels = await inspectTemporaryTree(
      disposableRoot,
      sessionId,
      connection.key,
      false,
    );
    if (result.timedOut) {
      throw new PocError("claude_process_timeout");
    }
    if (result.outputExceeded) {
      throw new PocError("claude_process_output_too_large");
    }
    if (result.status !== 0 || result.signal !== null) {
      throw new PocError(classifyClaudeFailure(result.stdout, result.stderr));
    }
    if (result.stderr.length !== 0) {
      throw new PocError("claude_unexpected_stderr");
    }

    const output = validateClaudeOutput(result.stdout, sessionId);
    const persistedTranscriptModels =
      transcriptModels !== undefined
        ? transcriptModels
        : await waitForPersistedTranscript(
            disposableRoot,
            sessionId,
            connection.key,
          );
    const observedModels = assertedObservedModel(
      output,
      persistedTranscriptModels,
    );
    console.log(
      JSON.stringify(
        {
          result: "passed",
          claude_code_version: requiredClaudeVersion,
          requested_model: model,
          observed_models: observedModels,
          effort,
          tools_exposed: 0,
          model_catalog_exact_match: true,
          paid_model_turns: 1,
          retries: 0,
          session_persisted: true,
          maximum_budget_usd: maximumBudgetUsd,
          claude_reported_cost_usd: output.total_cost_usd,
        },
        null,
        2,
      ),
    );
  } finally {
    if (disposableRoot !== undefined) {
      await rm(disposableRoot, { recursive: true, force: true });
    }
  }
}

try {
  await runPoc();
} catch (error) {
  fail(error instanceof PocError ? error.code : "unexpected_failure");
}
