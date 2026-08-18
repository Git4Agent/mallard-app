import { spawnSync } from "node:child_process";
import { stat } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (process.env.MALLARD_RUN_LIVE_CODEX_E2E !== "1") {
  fail(
    "Live Codex integration testing is disabled. Set MALLARD_RUN_LIVE_CODEX_E2E=1 to acknowledge authenticated, paid API use.",
  );
}

const configuredSourceHome = process.env.MALLARD_LIVE_CODEX_SOURCE_HOME?.trim();
const sourceHome = configuredSourceHome || join(homedir(), ".codex");

try {
  const [sourceHomeMetadata, authMetadata, manifestMetadata] = await Promise.all([
    stat(sourceHome),
    stat(join(sourceHome, "auth.json")),
    stat(join(projectRoot, "src-tauri", "Cargo.toml")),
  ]);
  if (!sourceHomeMetadata.isDirectory() || !authMetadata.isFile() || !manifestMetadata.isFile()) {
    fail("Live Codex integration prerequisites are not valid.");
  }
} catch {
  fail("Live Codex integration prerequisites are missing.");
}

const cargoCheck = spawnSync("cargo", ["--version"], { stdio: "ignore" });
if (cargoCheck.error || cargoCheck.status !== 0) {
  fail("Live Codex integration testing requires Cargo on PATH.");
}

console.log(
  configuredSourceHome
    ? "Running the isolated live Codex integration test with the configured source profile."
    : "Running the isolated live Codex integration test with the default source profile.",
);

const result = spawnSync(
  "cargo",
  [
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--test",
    "live_codex_e2e",
    "live_codex_mallard_setup_push_pull_divergence",
    "--",
    "--ignored",
    "--exact",
    "--nocapture",
    "--test-threads=1",
  ],
  {
    cwd: projectRoot,
    env: process.env,
    stdio: "inherit",
  },
);

if (result.error) {
  fail("The live Codex integration test could not be started.");
}
process.exitCode = result.status ?? 1;
