import { spawnSync } from "node:child_process";
import { constants } from "node:fs";
import { chmod, lstat, mkdir, mkdtemp, open, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const requiredAimockVersion = "1.39.0";
const requiredClaudeVersion = "2.1.260";
const minimumNodeVersion = [20, 15, 0];
const maximumVersionOutputBytes = 64 * 1024;
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
  "MALLARD_LIVE_CLAUDE_AIMOCK_AUDIT",
  "MALLARD_LIVE_CLAUDE_AIMOCK_SCENARIO",
  "MALLARD_LIVE_CLAUDE_PROVIDER_KEY",
  "MALLARD_LIVE_CLAUDE_PROVIDER_URL",
  "MALLARD_LIVE_CLAUDE_SOURCE_HOME",
  "MALLARD_LIVE_CODEX_PROVIDER_KEY",
  "MALLARD_LIVE_CODEX_PROVIDER_URL",
  "MALLARD_LIVE_CODEX_SOURCE_HOME",
  "MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E",
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

function sanitizedChildEnvironment() {
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
  environment.MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E = "1";
  environment.NO_PROXY = "127.0.0.1";
  environment.no_proxy = "127.0.0.1";
  return environment;
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

async function readBoundedRegularFile(path, maximumBytes) {
  const before = await lstat(path);
  if (
    before.isSymbolicLink() ||
    !before.isFile() ||
    before.size < 1 ||
    before.size > maximumBytes
  ) {
    throw new Error("bounded_file");
  }
  const noFollow = typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
  const handle = await open(path, constants.O_RDONLY | noFollow);
  try {
    const descriptorBefore = await handle.stat();
    if (!stableMetadata(before, descriptorBefore)) {
      throw new Error("bounded_file_changed");
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
      throw new Error("bounded_file_changed");
    }
    return bytes.toString("utf8");
  } finally {
    await handle.close();
  }
}

async function verifyRepositoryPrerequisites() {
  const manifestPath = join(projectRoot, "src-tauri", "Cargo.toml");
  const harnessPath = join(
    projectRoot,
    "scripts",
    "live-claude-aimock-server.mjs",
  );
  const packagePath = join(
    projectRoot,
    "node_modules",
    "@copilotkit",
    "aimock",
    "package.json",
  );
  const [manifestMetadata, harnessMetadata, packageMetadata] =
    await Promise.all([
      lstat(manifestPath),
      lstat(harnessPath),
      lstat(packagePath),
    ]);
  if (
    manifestMetadata.isSymbolicLink() ||
    !manifestMetadata.isFile() ||
    harnessMetadata.isSymbolicLink() ||
    !harnessMetadata.isFile() ||
    packageMetadata.isSymbolicLink() ||
    !packageMetadata.isFile() ||
    packageMetadata.size < 1 ||
    packageMetadata.size > 64 * 1024
  ) {
    return false;
  }
  const packageContents = await readBoundedRegularFile(packagePath, 64 * 1024);
  let packageDocument;
  try {
    packageDocument = JSON.parse(packageContents);
  } catch {
    return false;
  }
  return (
    packageDocument?.name === "@copilotkit/aimock" &&
    packageDocument?.version === requiredAimockVersion
  );
}

function verifyCargo(environment) {
  const result = spawnSync("cargo", ["--version"], {
    cwd: projectRoot,
    env: environment,
    stdio: "ignore",
  });
  return !result.error && result.status === 0;
}

async function verifyClaudeVersion(environment) {
  let disposableRoot;
  try {
    disposableRoot = await mkdtemp(
      join(tmpdir(), "mallard-claude-version-preflight-"),
    );
    await chmod(disposableRoot, 0o700);
    const processHome = join(disposableRoot, "home");
    const claudeConfig = join(disposableRoot, "claude-config");
    await Promise.all([
      mkdir(processHome, { mode: 0o700 }),
      mkdir(claudeConfig, { mode: 0o700 }),
    ]);
    const result = spawnSync("claude", ["--version"], {
      cwd: disposableRoot,
      env: {
        PATH: environment.PATH ?? "/usr/bin:/bin",
        HOME: processHome,
        USERPROFILE: processHome,
        CLAUDE_CONFIG_DIR: claudeConfig,
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        DISABLE_TELEMETRY: "1",
        DISABLE_ERROR_REPORTING: "1",
        NO_PROXY: "127.0.0.1",
        no_proxy: "127.0.0.1",
      },
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

if (process.env.MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E !== "1") {
  fail(
    "Live Claude aimock integration testing is disabled. Set MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1 to acknowledge the heavyweight credentialless local Claude Code integration test.",
  );
}

if (!versionAtLeast(process.versions.node, minimumNodeVersion)) {
  fail("Live Claude aimock integration testing requires Node 20.15.0 or newer.");
}

try {
  if (!(await verifyRepositoryPrerequisites())) {
    fail(
      `Live Claude aimock integration testing requires @copilotkit/aimock ${requiredAimockVersion} from npm install.`,
    );
  }
} catch {
  fail("Live Claude aimock integration test repository prerequisites are missing.");
}

const childEnvironment = sanitizedChildEnvironment();
try {
  if (!(await verifyClaudeVersion(childEnvironment))) {
    fail(
      `Live Claude aimock integration testing requires Claude Code ${requiredClaudeVersion} exactly.`,
    );
  }
} catch {
  fail(
    `Live Claude aimock integration testing requires an installed Claude Code ${requiredClaudeVersion} executable on PATH.`,
  );
}

if (!verifyCargo(childEnvironment)) {
  fail("Live Claude aimock integration testing requires Cargo on PATH.");
}

console.log(
  "Running the heavyweight credentialless local Claude Code aimock integration test.",
);

const result = spawnSync(
  "cargo",
  [
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--test",
    "live_claude_aimock_e2e",
    "live_claude_aimock_setup_push_pull_divergence",
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
  fail("The live Claude aimock integration test could not be started.");
}
process.exitCode = result.status ?? 1;
