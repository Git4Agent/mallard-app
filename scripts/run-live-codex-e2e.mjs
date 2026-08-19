import { spawnSync } from "node:child_process";
import { constants } from "node:fs";
import { lstat, open, stat } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const connectionPath = join(projectRoot, ".env.local");
const maximumConnectionBytes = 4_096;
const maximumKeyBytes = 1_024;
const removedChildEnvironmentVariables = [
  "OPENAI_API_KEY",
  "CODEX_API_KEY",
  "AZURE_OPENAI_API_KEY",
  "MALLARD_LIVE_CODEX_PROVIDER_KEY",
  "MALLARD_LIVE_CODEX_PROVIDER_URL",
  "MALLARD_TUZI_POC_KEY",
  "MALLARD_CODEX_PROVIDER_API_KEY",
];

function fail(message) {
  console.error(message);
  process.exit(1);
}

function normalizeProviderUrl(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value !== value.trim() ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(value) ||
    !/^https:\/\//iu.test(value) ||
    value.slice("https://".length).split("/", 1)[0].includes("@") ||
    value.includes("?") ||
    value.includes("#")
  ) {
    return undefined;
  }

  let providerUrl;
  try {
    providerUrl = new URL(value);
  } catch {
    return undefined;
  }

  if (
    providerUrl.protocol !== "https:" ||
    providerUrl.hostname.length === 0 ||
    providerUrl.username.length !== 0 ||
    providerUrl.password.length !== 0 ||
    providerUrl.search.length !== 0 ||
    providerUrl.hash.length !== 0 ||
    (providerUrl.pathname !== "/" && providerUrl.pathname !== "/v1")
  ) {
    return undefined;
  }

  return `${providerUrl.origin}/v1`;
}

function isValidConnectionDocument(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }

  const keys = Object.keys(value).sort();
  if (
    keys.length !== 3 ||
    keys[0] !== "_type" ||
    keys[1] !== "key" ||
    keys[2] !== "url"
  ) {
    return false;
  }

  if (value._type !== "newapi_channel_conn") {
    return false;
  }

  if (
    typeof value.key !== "string" ||
    value.key.length === 0 ||
    value.key !== value.key.trim() ||
    Buffer.byteLength(value.key, "utf8") > maximumKeyBytes ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(value.key)
  ) {
    return false;
  }

  return normalizeProviderUrl(value.url) !== undefined;
}

function isValidConnectionMetadata(metadata) {
  return (
    !metadata.isSymbolicLink() &&
    metadata.isFile() &&
    metadata.size >= 1 &&
    metadata.size <= maximumConnectionBytes &&
    (process.platform === "win32" || (metadata.mode & 0o7777) === 0o600)
  );
}

function hasSameStableMetadata(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.size === right.size &&
    left.mtimeMs === right.mtimeMs &&
    left.ctimeMs === right.ctimeMs
  );
}

async function validateConnectionFile() {
  const pathMetadataBeforeOpen = await lstat(connectionPath);
  if (!isValidConnectionMetadata(pathMetadataBeforeOpen)) {
    return false;
  }

  const noFollowFlag =
    typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
  const connectionFile = await open(
    connectionPath,
    constants.O_RDONLY | noFollowFlag,
  );

  let contents;
  try {
    const descriptorMetadataBeforeRead = await connectionFile.stat();
    if (
      !isValidConnectionMetadata(descriptorMetadataBeforeRead) ||
      !hasSameStableMetadata(
        pathMetadataBeforeOpen,
        descriptorMetadataBeforeRead,
      )
    ) {
      return false;
    }

    const bytes = await connectionFile.readFile();
    const descriptorMetadataAfterRead = await connectionFile.stat();
    const pathMetadataAfterRead = await lstat(connectionPath);
    if (
      bytes.length !== descriptorMetadataBeforeRead.size ||
      !hasSameStableMetadata(
        descriptorMetadataBeforeRead,
        descriptorMetadataAfterRead,
      ) ||
      !isValidConnectionMetadata(pathMetadataAfterRead) ||
      !hasSameStableMetadata(descriptorMetadataAfterRead, pathMetadataAfterRead)
    ) {
      return false;
    }

    contents = bytes.toString("utf8");
    if (!Buffer.from(contents, "utf8").equals(bytes)) {
      return false;
    }
  } finally {
    await connectionFile.close();
  }

  let connection;
  try {
    connection = JSON.parse(contents);
  } catch {
    return false;
  }

  return isValidConnectionDocument(connection);
}

function sanitizedChildEnvironment() {
  const environment = { ...process.env };
  for (const variable of removedChildEnvironmentVariables) {
    delete environment[variable];
  }
  environment.MALLARD_RUN_LIVE_CODEX_E2E = "1";
  return environment;
}

if (process.env.MALLARD_RUN_LIVE_CODEX_E2E !== "1") {
  fail(
    "Live Codex integration testing is disabled. Set MALLARD_RUN_LIVE_CODEX_E2E=1 to acknowledge authenticated, paid API use.",
  );
}

try {
  const [connectionIsValid, manifestMetadata] = await Promise.all([
    validateConnectionFile(),
    stat(join(projectRoot, "src-tauri", "Cargo.toml")),
  ]);
  if (!connectionIsValid || !manifestMetadata.isFile()) {
    fail("Live Codex integration prerequisites are not valid.");
  }
} catch {
  fail("Live Codex integration prerequisites are missing.");
}

const childEnvironment = sanitizedChildEnvironment();
const cargoCheck = spawnSync("cargo", ["--version"], {
  env: childEnvironment,
  stdio: "ignore",
});
if (cargoCheck.error || cargoCheck.status !== 0) {
  fail("Live Codex integration testing requires Cargo on PATH.");
}

console.log("Running the isolated live Codex integration test.");

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
    env: childEnvironment,
    stdio: "inherit",
  },
);

if (result.error) {
  fail("The live Codex integration test could not be started.");
}
process.exitCode = result.status ?? 1;
