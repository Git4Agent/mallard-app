import { execFile, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, copyFile, mkdir, readFile, rename, rm } from "node:fs/promises";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
const tauriCliScript = resolve("node_modules", "@tauri-apps", "cli", "tauri.js");

function run(command, commandArgs) {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(command, commandArgs, { stdio: "inherit" });
    child.once("error", rejectRun);
    child.once("exit", (code) => resolveRun(code ?? 1));
  });
}

async function runChecked(command, commandArgs) {
  const exitCode = await run(command, commandArgs);
  if (exitCode !== 0) {
    throw new Error(`${command} exited with status ${exitCode}.`);
  }
}

function buildTarget(buildArgs) {
  const targetIndex = buildArgs.indexOf("--target");
  const shortTargetIndex = buildArgs.indexOf("-t");
  const inlineTargetArgument = buildArgs.find((arg) => arg.startsWith("--target="));
  if (inlineTargetArgument !== undefined) {
    const inlineTarget = inlineTargetArgument.slice("--target=".length);
    if (!inlineTarget) throw new Error("--target requires a Rust target triple.");
    return inlineTarget;
  }
  const index = targetIndex >= 0 ? targetIndex : shortTargetIndex;
  const target = index >= 0 ? buildArgs[index + 1] : undefined;
  if (index >= 0 && !target) throw new Error("--target requires a Rust target triple.");
  return target;
}

function capture(command, commandArgs) {
  return new Promise((resolveCapture, rejectCapture) => {
    execFile(command, commandArgs, { maxBuffer: 1024 * 1024 }, (error, stdout) => {
      if (error) rejectCapture(error);
      else resolveCapture(stdout);
    });
  });
}

async function rustHostTarget() {
  const verboseVersion = await capture("rustc", ["-vV"]);
  const host = verboseVersion.split(/\r?\n/).find((line) => line.startsWith("host: "))?.slice(6).trim();
  if (!host) throw new Error("rustc -vV did not report a host target triple.");
  return host;
}

function cargoTargetDirectory() {
  return new Promise((resolveMetadata, rejectMetadata) => {
    execFile(
      "cargo",
      ["metadata", "--format-version", "1", "--no-deps"],
      { maxBuffer: 64 * 1024 * 1024 },
      (error, stdout) => {
        if (error) rejectMetadata(error);
        else resolveMetadata(JSON.parse(stdout).target_directory);
      },
    );
  });
}

async function macosDmgPath(buildArgs) {
  const target = buildTarget(buildArgs);
  const architecture = target === "aarch64-apple-darwin" || (!target && process.arch === "arm64")
    ? "aarch64"
    : "x64";
  const tauriConfig = JSON.parse(await readFile(resolve("src-tauri", "tauri.conf.json"), "utf8"));
  const fileProductName = tauriConfig.productName.replaceAll(" ", "_");

  return join(
    await cargoTargetDirectory(),
    ...(target ? [target] : []),
    "release",
    "bundle",
    "dmg",
    `${fileProductName}_${tauriConfig.version}_${architecture}.dmg`,
  );
}

async function stageCliBinary(buildArgs) {
  const command = buildArgs[0];
  if (command !== "build" && command !== "dev") return;

  const explicitTarget = buildTarget(buildArgs);
  const configuredTarget = explicitTarget || process.env.CARGO_BUILD_TARGET;
  const target = configuredTarget || await rustHostTarget();
  if (target === "universal-apple-darwin") {
    throw new Error("Universal macOS CLI staging is not supported; build one concrete Apple target at a time.");
  }
  const release = command === "build"
    ? !buildArgs.includes("--debug") && !buildArgs.includes("-d")
    : buildArgs.includes("--release");
  const profile = release ? "release" : "debug";
  const cargoArgs = ["build", "--locked", "-p", "mallard-cli"];
  if (release) cargoArgs.push("--release");
  if (explicitTarget) cargoArgs.push("--target", explicitTarget);
  await runChecked("cargo", cargoArgs);

  // In development Cargo places both binaries in the same target/profile
  // directory, which is exactly where `get_mallard_cli_path` resolves the
  // CLI. Packaging additionally needs Tauri's target-suffixed staging copy.
  if (command === "dev") return;

  const windows = target.includes("windows");
  const extension = windows ? ".exe" : "";
  const targetDirectory = await cargoTargetDirectory();
  const source = join(
    targetDirectory,
    ...(configuredTarget ? [configuredTarget] : []),
    profile,
    `mallard-cli${extension}`,
  );
  if (!existsSync(source)) throw new Error(`Expected built Mallard CLI at ${source}.`);

  const binariesDirectory = resolve("src-tauri", "binaries");
  const destination = join(binariesDirectory, `mallard-cli-${target}${extension}`);
  const temporary = `${destination}.tmp-${process.pid}`;
  await mkdir(binariesDirectory, { recursive: true });
  try {
    await copyFile(source, temporary);
    if (!windows) await chmod(temporary, 0o755);
    if (process.platform === "win32") await rm(destination, { force: true });
    await rename(temporary, destination);
  } finally {
    await rm(temporary, { force: true });
  }
}

function withPackagedCliConfig(buildArgs) {
  if (buildArgs[0] !== "build") return buildArgs;
  const overlay = JSON.stringify({ bundle: { externalBin: ["binaries/mallard-cli"] } });
  const separator = buildArgs.indexOf("--");
  const insertAt = separator === -1 ? buildArgs.length : separator;
  return [
    ...buildArgs.slice(0, insertAt),
    "--config",
    overlay,
    ...buildArgs.slice(insertAt),
  ];
}

// Tauri notarizes and staples Mallard.app but not the disk image around it.
// Notarizing the DMG as well lets Gatekeeper accept the download itself.
function notarizationCredentials() {
  const keyId = process.env.APPLE_API_KEY;
  const issuer = process.env.APPLE_API_ISSUER;
  const keyPath = process.env.APPLE_API_KEY_PATH;
  if (!keyId && !issuer && !keyPath) return undefined;
  if (!keyId || !issuer || !keyPath) {
    throw new Error("APPLE_API_KEY, APPLE_API_ISSUER, and APPLE_API_KEY_PATH must be set together.");
  }
  if (!existsSync(keyPath)) throw new Error(`Expected the App Store Connect API key at ${keyPath}.`);
  return { keyId, issuer, keyPath };
}

async function notarizeDmg(dmgPath, { keyId, issuer, keyPath }) {
  const submission = await capture("xcrun", [
    "notarytool", "submit", dmgPath,
    "--key", keyPath, "--key-id", keyId, "--issuer", issuer,
    "--wait", "--output-format", "json",
  ]);
  const status = JSON.parse(submission).status;
  if (status !== "Accepted") {
    throw new Error(`Notarization of ${dmgPath} finished with status ${status}.`);
  }
  await runChecked("xcrun", ["stapler", "staple", dmgPath]);
}

await stageCliBinary(args);
await runChecked(process.execPath, [tauriCliScript, ...withPackagedCliConfig(args)]);

const requireTrusted = process.env.MALLARD_REQUIRE_TRUSTED_MACOS === "1";
const verifyBundle = process.env.MALLARD_VERIFY_MACOS_BUNDLE === "1";
const macosBuild = process.platform === "darwin" && args[0] === "build";
if ((requireTrusted || verifyBundle) && !macosBuild) {
  throw new Error("MALLARD_REQUIRE_TRUSTED_MACOS and MALLARD_VERIFY_MACOS_BUNDLE need a Tauri build on macOS.");
}

const credentials = macosBuild ? notarizationCredentials() : undefined;
if (requireTrusted && !credentials) {
  throw new Error("MALLARD_REQUIRE_TRUSTED_MACOS needs App Store Connect API credentials.");
}

if (credentials || verifyBundle) {
  const dmgPath = await macosDmgPath(args);
  if (!existsSync(dmgPath)) throw new Error(`Expected macOS DMG at ${dmgPath}.`);

  if (credentials) await notarizeDmg(dmgPath, credentials);

  if (verifyBundle) {
    await runChecked(process.execPath, [resolve("scripts", "verify-macos-bundle.mjs"), dmgPath]);
  }
}
