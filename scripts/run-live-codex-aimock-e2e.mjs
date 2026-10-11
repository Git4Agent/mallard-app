import { spawnSync } from "node:child_process";
import { chmod, mkdir, mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const requiredAimockVersion = "1.39.0";
const minimumNodeVersion = [20, 15, 0];
const codexSandboxPreflightOutputBytes = 64 * 1024;
const codexSandboxPreflightTimeoutMs = 15_000;
const removedChildEnvironmentVariables = [
  "AIMOCK_DUMMY_KEY_MARKER",
  "AIMOCK_PROVIDER_ANTHROPIC_KEY",
  "AIMOCK_PROVIDER_AZURE_KEY",
  "AIMOCK_PROVIDER_COHERE_KEY",
  "AIMOCK_PROVIDER_ELEVENLABS_KEY",
  "AIMOCK_PROVIDER_FAL_KEY",
  "AIMOCK_PROVIDER_GEMINI_KEY",
  "AIMOCK_PROVIDER_GROK_KEY",
  "AIMOCK_PROVIDER_OLLAMA_KEY",
  "AIMOCK_PROVIDER_OPENAI_KEY",
  "AIMOCK_PROVIDER_OPENROUTER_KEY",
  "AIMOCK_PROVIDER_VEO_KEY",
  "ALL_PROXY",
  "ANTHROPIC_API_KEY",
  "AWS_ACCESS_KEY_ID",
  "AWS_SECRET_ACCESS_KEY",
  "AWS_SESSION_TOKEN",
  "AZURE_OPENAI_API_KEY",
  "AZURE_OPENAI_ENDPOINT",
  "CLAUDE_CODE_OAUTH_TOKEN",
  "CLAUDE_CONFIG_DIR",
  "CODEX_API_KEY",
  "CODEX_HOME",
  "GEMINI_API_KEY",
  "GOOGLE_API_KEY",
  "HTTPS_PROXY",
  "HTTP_PROXY",
  "MALLARD_CODEX_PROVIDER_API_KEY",
  "MALLARD_CODEX_PROVIDER_URL",
  "MALLARD_LIVE_CODEX_PROVIDER_KEY",
  "MALLARD_LIVE_CODEX_PROVIDER_URL",
  "MALLARD_LIVE_CODEX_SOURCE_HOME",
  "MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E",
  "MALLARD_RUN_LIVE_CODEX_E2E",
  "MALLARD_TUZI_POC_KEY",
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
];

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
  for (const variable of removedChildEnvironmentVariables) {
    delete environment[variable];
  }
  environment.MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E = "1";
  environment.NO_PROXY = "127.0.0.1";
  environment.no_proxy = "127.0.0.1";
  return environment;
}

async function verifyRepositoryPrerequisites() {
  const manifestPath = join(projectRoot, "src-tauri", "Cargo.toml");
  const harnessPath = join(projectRoot, "scripts", "live-codex-aimock-server.mjs");
  const packagePath = join(
    projectRoot,
    "node_modules",
    "@copilotkit",
    "aimock",
    "package.json",
  );
  const [manifestMetadata, harnessMetadata, packageMetadata, packageContents] =
    await Promise.all([
      stat(manifestPath),
      stat(harnessPath),
      stat(packagePath),
      readFile(packagePath, "utf8"),
    ]);
  if (
    !manifestMetadata.isFile() ||
    !harnessMetadata.isFile() ||
    !packageMetadata.isFile() ||
    Buffer.byteLength(packageContents, "utf8") > 64 * 1024
  ) {
    return false;
  }
  let packageDocument;
  try {
    packageDocument = JSON.parse(packageContents);
  } catch {
    return false;
  }
  if (
    packageDocument?.name !== "@copilotkit/aimock" ||
    packageDocument?.version !== requiredAimockVersion
  ) {
    return false;
  }
  // Do not execute a third-party dependency in this parent process: it
  // still carries the caller's environment so it can construct the tightly
  // sanitized child environment below. The server child performs the real
  // import after all provider credentials and proxy variables are removed;
  // readiness is the functional dependency check.
  return true;
}

function verifyTool(command, args, environment) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    env: environment,
    stdio: "ignore",
  });
  return !result.error && result.status === 0;
}

function isNestedSeatbeltFailure(result) {
  return (
    result.status === 71 &&
    typeof result.stderr === "string" &&
    result.stderr
      .toLowerCase()
      .includes("sandbox-exec: sandbox_apply: operation not permitted")
  );
}

async function verifyCodexSandbox(environment) {
  let disposableRoot;
  try {
    disposableRoot = await mkdtemp(
      join(tmpdir(), "mallard-codex-sandbox-preflight-"),
    );
    await chmod(disposableRoot, 0o700);
    const disposableHome = join(disposableRoot, "home");
    const disposableCodexHome = join(disposableRoot, "codex-home");
    const disposableProject = join(disposableRoot, "project");
    await Promise.all([
      mkdir(disposableHome, { mode: 0o700 }),
      mkdir(disposableCodexHome, { mode: 0o700 }),
      mkdir(disposableProject, { mode: 0o700 }),
    ]);

    const probeCommand = process.platform === "win32"
      ? ["cmd.exe", "/d", "/c", "exit", "0"]
      : ["/usr/bin/true"];
    const result = spawnSync("codex", ["sandbox", "--", ...probeCommand], {
      cwd: disposableProject,
      env: {
        ...environment,
        HOME: disposableHome,
        USERPROFILE: disposableHome,
        CODEX_HOME: disposableCodexHome,
      },
      encoding: "utf8",
      input: "",
      maxBuffer: codexSandboxPreflightOutputBytes,
      timeout: codexSandboxPreflightTimeoutMs,
      killSignal: "SIGKILL",
    });
    return {
      nestedSeatbelt: isNestedSeatbeltFailure(result),
      succeeded: !result.error && result.status === 0,
    };
  } finally {
    if (disposableRoot !== undefined) {
      await rm(disposableRoot, { recursive: true, force: true });
    }
  }
}

if (process.env.MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E !== "1") {
  fail(
    "Live Codex aimock integration testing is disabled. Set MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1 to acknowledge the heavyweight local Codex integration test.",
  );
}

if (!versionAtLeast(process.versions.node, minimumNodeVersion)) {
  fail("Live Codex aimock integration testing requires Node 20.15.0 or newer.");
}

try {
  if (!(await verifyRepositoryPrerequisites())) {
    fail(
      `Live Codex aimock integration testing requires @copilotkit/aimock ${requiredAimockVersion} from npm install.`,
    );
  }
} catch {
  fail("Live Codex aimock integration test repository prerequisites are missing.");
}

const childEnvironment = sanitizedChildEnvironment();
let codexSandboxPreflight;
try {
  codexSandboxPreflight = await verifyCodexSandbox(childEnvironment);
} catch {
  fail(
    "Live Codex aimock integration testing requires an installed Codex CLI with a working sandbox command.",
  );
}
if (codexSandboxPreflight.nestedSeatbelt) {
  fail(
    "Codex sandbox preflight detected an existing macOS Seatbelt sandbox. Run this test from a normal terminal outside an existing Codex sandbox.",
  );
}
if (!codexSandboxPreflight.succeeded) {
  fail(
    "Live Codex aimock integration testing requires an installed Codex CLI with a working sandbox command.",
  );
}
if (!verifyTool("cargo", ["--version"], childEnvironment)) {
  fail("Live Codex aimock integration testing requires Cargo on PATH.");
}

console.log("Running the heavyweight local Codex aimock integration test.");

const result = spawnSync(
  "cargo",
  [
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--test",
    "live_codex_aimock_e2e",
    "live_codex_aimock_setup_push_pull_divergence",
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
  fail("The live Codex aimock integration test could not be started.");
}
process.exitCode = result.status ?? 1;
