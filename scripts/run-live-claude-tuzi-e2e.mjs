import { spawnSync } from "node:child_process";
import { constants } from "node:fs";
import { chmod, lstat, mkdir, mkdtemp, open, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const connectionPath = join(projectRoot, ".env.local");
const liveGate = "MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E";
const requiredClaudeVersion = "2.1.260";
const requiredModel = "claude-sonnet-5";
const minimumNodeVersion = [20, 15, 0];
const maximumConnectionBytes = 4_096;
const maximumKeyBytes = 1_024;
const maximumCatalogBytes = 4 * 1024 * 1024;
const maximumVersionOutputBytes = 64 * 1024;
const catalogTimeoutMs = 20_000;
const versionTimeoutMs = 15_000;

const removedChildEnvironmentVariables = new Set([
  "AIMOCK_DUMMY_KEY_MARKER",
  "ALL_PROXY",
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_AUTH_TOKEN",
  "ANTHROPIC_BASE_URL",
  "ANTHROPIC_BEDROCK_BASE_URL",
  "ANTHROPIC_BETAS",
  "ANTHROPIC_CUSTOM_HEADERS",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  "ANTHROPIC_DEFAULT_OPUS_MODEL",
  "ANTHROPIC_DEFAULT_SONNET_MODEL",
  "ANTHROPIC_FOUNDRY_API_KEY",
  "ANTHROPIC_FOUNDRY_BASE_URL",
  "ANTHROPIC_FOUNDRY_RESOURCE",
  "ANTHROPIC_MODEL",
  "ANTHROPIC_VERTEX_BASE_URL",
  "ANTHROPIC_VERTEX_PROJECT_ID",
  "AWS_ACCESS_KEY_ID",
  "AWS_SECRET_ACCESS_KEY",
  "AWS_SESSION_TOKEN",
  "AZURE_OPENAI_API_KEY",
  "AZURE_OPENAI_ENDPOINT",
  "CLAUDE_CODE_OAUTH_TOKEN",
  "CLAUDE_CODE_USE_BEDROCK",
  "CLAUDE_CODE_USE_FOUNDRY",
  "CLAUDE_CODE_USE_MANTLE",
  "CLAUDE_CODE_USE_VERTEX",
  "CLAUDE_CONFIG_DIR",
  "CODEX_API_KEY",
  "CODEX_HOME",
  "GEMINI_API_KEY",
  "GOOGLE_API_KEY",
  "HTTPS_PROXY",
  "HTTP_PROXY",
  "MALLARD_CODEX_PROVIDER_API_KEY",
  "MALLARD_CODEX_PROVIDER_URL",
  "MALLARD_LIVE_CLAUDE_PROVIDER_KEY",
  "MALLARD_LIVE_CLAUDE_PROVIDER_URL",
  "MALLARD_LIVE_CLAUDE_SOURCE_HOME",
  "MALLARD_LIVE_CODEX_PROVIDER_KEY",
  "MALLARD_LIVE_CODEX_PROVIDER_URL",
  "MALLARD_LIVE_CODEX_SOURCE_HOME",
  "MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E",
  "MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E",
  "MALLARD_RUN_LIVE_CLAUDE_TUZI_POC",
  "MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E",
  "MALLARD_RUN_LIVE_CODEX_E2E",
  "NO_PROXY",
  "OPENAI_API_KEY",
  "OPENAI_BASE_URL",
  "OPENAI_ORGANIZATION",
  "OPENAI_ORG_ID",
  "OPENAI_PROJECT_ID",
  "all_proxy",
  "http_proxy",
  "https_proxy",
  "no_proxy",
]);

function fail(message) {
  console.error(message);
  process.exit(1);
}

function versionAtLeast(actual, minimum) {
  const parts = actual.split(".").map((part) => Number.parseInt(part, 10));
  if (
    parts.length !== 3 ||
    parts.some((part) => !Number.isSafeInteger(part) || part < 0)
  ) {
    return false;
  }
  for (let index = 0; index < minimum.length; index += 1) {
    if (parts[index] !== minimum[index]) {
      return parts[index] > minimum[index];
    }
  }
  return true;
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

function validConnectionMetadata(metadata) {
  return (
    !metadata.isSymbolicLink() &&
    metadata.isFile() &&
    metadata.size >= 1 &&
    metadata.size <= maximumConnectionBytes &&
    (process.platform === "win32" || (metadata.mode & 0o7777) === 0o600)
  );
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
  const before = await lstat(connectionPath);
  if (!validConnectionMetadata(before)) {
    throw new Error("connection_metadata");
  }
  const noFollow =
    typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
  const closeOnExec =
    typeof constants.O_CLOEXEC === "number" ? constants.O_CLOEXEC : 0;
  const handle = await open(
    connectionPath,
    constants.O_RDONLY | noFollow | closeOnExec,
  );
  let bytes;
  try {
    const descriptorBefore = await handle.stat();
    if (
      !validConnectionMetadata(descriptorBefore) ||
      !stableMetadata(before, descriptorBefore)
    ) {
      throw new Error("connection_changed");
    }
    bytes = await handle.readFile();
    const descriptorAfter = await handle.stat();
    const after = await lstat(connectionPath);
    if (
      bytes.length !== descriptorBefore.size ||
      bytes.length > maximumConnectionBytes ||
      !stableMetadata(descriptorBefore, descriptorAfter) ||
      !validConnectionMetadata(after) ||
      !stableMetadata(descriptorAfter, after)
    ) {
      throw new Error("connection_changed");
    }
  } finally {
    await handle.close();
  }

  const text = bytes.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(bytes)) {
    throw new Error("connection_utf8");
  }
  const document = JSON.parse(text);
  if (document === null || typeof document !== "object" || Array.isArray(document)) {
    throw new Error("connection_document");
  }
  const fields = Object.keys(document).sort();
  if (
    fields.length !== 3 ||
    fields[0] !== "_type" ||
    fields[1] !== "key" ||
    fields[2] !== "url" ||
    document._type !== "newapi_channel_conn" ||
    typeof document.key !== "string" ||
    document.key.length === 0 ||
    document.key !== document.key.trim() ||
    Buffer.byteLength(document.key, "utf8") > maximumKeyBytes ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(document.key)
  ) {
    throw new Error("connection_document");
  }
  const baseUrl = normalizeTuziBaseUrl(document.url);
  if (baseUrl === undefined) {
    throw new Error("connection_url");
  }
  return { baseUrl, key: document.key };
}

async function readBoundedResponse(response) {
  const contentLength = response.headers.get("content-length");
  if (
    contentLength !== null &&
    (!/^\d+$/u.test(contentLength) || Number(contentLength) > maximumCatalogBytes)
  ) {
    throw new Error("catalog_size");
  }
  if (response.body === null) {
    throw new Error("catalog_empty");
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
      if (length > maximumCatalogBytes) {
        throw new Error("catalog_size");
      }
      chunks.push(Buffer.from(value));
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  return Buffer.concat(chunks, length);
}

async function verifyExactModel(connection) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), catalogTimeoutMs);
  try {
    const response = await fetch(new URL("/v1/models", connection.baseUrl), {
      method: "GET",
      headers: {
        Accept: "application/json",
        Authorization: `Bearer ${connection.key}`,
      },
      redirect: "error",
      signal: controller.signal,
    });
    if (response.status !== 200) {
      await response.body?.cancel().catch(() => {});
      throw new Error("catalog_status");
    }
    const bytes = await readBoundedResponse(response);
    if (bytes.indexOf(Buffer.from(connection.key, "utf8")) !== -1) {
      throw new Error("catalog_secret_echo");
    }
    const document = JSON.parse(bytes.toString("utf8"));
    if (
      !Array.isArray(document?.data) ||
      document.data.length > 20_000 ||
      !document.data.some(
        (entry) =>
          entry !== null &&
          typeof entry === "object" &&
          !Array.isArray(entry) &&
          entry.id === requiredModel,
      )
    ) {
      throw new Error("catalog_model");
    }
  } finally {
    clearTimeout(timer);
  }
}

function isolatedClaudeEnvironment(home, claudeConfig) {
  return {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: home,
    USERPROFILE: home,
    CLAUDE_CONFIG_DIR: claudeConfig,
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    DISABLE_TELEMETRY: "1",
    DISABLE_ERROR_REPORTING: "1",
    DISABLE_UPDATES: "1",
  };
}

async function verifyClaudeVersion() {
  let disposableRoot;
  try {
    disposableRoot = await mkdtemp(
      join(tmpdir(), "mallard-claude-tuzi-e2e-version-"),
    );
    await chmod(disposableRoot, 0o700);
    const home = join(disposableRoot, "home");
    const claudeConfig = join(disposableRoot, "claude-config");
    await Promise.all([
      mkdir(home, { mode: 0o700 }),
      mkdir(claudeConfig, { mode: 0o700 }),
    ]);
    const result = spawnSync("claude", ["--version"], {
      cwd: disposableRoot,
      env: isolatedClaudeEnvironment(home, claudeConfig),
      encoding: "utf8",
      input: "",
      maxBuffer: maximumVersionOutputBytes,
      timeout: versionTimeoutMs,
      killSignal: "SIGKILL",
    });
    return (
      !result.error &&
      result.status === 0 &&
      result.signal === null &&
      result.stderr === "" &&
      result.stdout.trim() === `${requiredClaudeVersion} (Claude Code)`
    );
  } finally {
    if (disposableRoot !== undefined) {
      await rm(disposableRoot, { recursive: true, force: true });
    }
  }
}

function sanitizedCargoEnvironment() {
  const environment = { ...process.env };
  for (const name of Object.keys(environment)) {
    if (
      removedChildEnvironmentVariables.has(name) ||
      name.startsWith("AIMOCK_PROVIDER_") ||
      name.startsWith("ANTHROPIC_") ||
      name.startsWith("AWS_") ||
      name.startsWith("AZURE_") ||
      name.startsWith("CLAUDE_CODE_") ||
      name.startsWith("CODEX_") ||
      name.startsWith("GEMINI_") ||
      name.startsWith("GOOGLE_") ||
      name.startsWith("MALLARD_LIVE_CLAUDE_") ||
      name.startsWith("MALLARD_LIVE_CODEX_") ||
      name.startsWith("OPENAI_")
    ) {
      delete environment[name];
    }
  }
  environment[liveGate] = "1";
  return environment;
}

if (process.env[liveGate] !== "1") {
  fail(
    `Live Claude Tuzi integration testing is disabled. Set ${liveGate}=1 to acknowledge three authenticated, paid Sonnet 5 turns.`,
  );
}

if (!versionAtLeast(process.versions.node, minimumNodeVersion)) {
  fail("Live Claude Tuzi integration testing requires Node 20.15.0 or newer.");
}

try {
  const manifest = await stat(join(projectRoot, "src-tauri", "Cargo.toml"));
  if (!manifest.isFile()) {
    fail("Live Claude Tuzi integration test repository prerequisites are invalid.");
  }
} catch {
  fail("Live Claude Tuzi integration test repository prerequisites are missing.");
}

try {
  if (!(await verifyClaudeVersion())) {
    fail(
      `Live Claude Tuzi integration testing requires Claude Code ${requiredClaudeVersion} exactly.`,
    );
  }
} catch {
  fail(
    `Live Claude Tuzi integration testing requires an installed Claude Code ${requiredClaudeVersion} executable on PATH.`,
  );
}

try {
  const connection = await loadConnection();
  await verifyExactModel(connection);
  connection.key = "";
} catch {
  fail(
    `Live Claude Tuzi integration testing requires a private valid .env.local whose key exposes exact model ${requiredModel}.`,
  );
}

const childEnvironment = sanitizedCargoEnvironment();
const cargoCheck = spawnSync("cargo", ["--version"], {
  cwd: projectRoot,
  env: childEnvironment,
  stdio: "ignore",
});
if (cargoCheck.error || cargoCheck.status !== 0) {
  fail("Live Claude Tuzi integration testing requires Cargo on PATH.");
}

console.log(
  "Running the isolated three-turn paid Claude Code/Tuzi divergence integration test.",
);

const result = spawnSync(
  "cargo",
  [
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--test",
    "live_claude_aimock_e2e",
    "live_claude_tuzi_setup_push_pull_divergence",
    "--",
    "--ignored",
    "--exact",
    "--nocapture",
    "--test-threads=1",
  ],
  {
    cwd: projectRoot,
    env: childEnvironment,
    stdio: "inherit",
  },
);

if (result.error) {
  fail("The live Claude Tuzi integration test could not be started.");
}
process.exitCode = result.status ?? 1;
