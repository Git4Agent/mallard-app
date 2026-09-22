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

// Where Tauri puts this build's bundles, and the `<Product>_<version>_<arch>`
// stem its DMG carries; the updater archive is renamed to the same stem so
// the two macOS builds never upload a file of the same name.
async function bundleLayout(buildArgs) {
  const target = buildTarget(buildArgs);
  const architecture = target === "aarch64-apple-darwin" || (!target && process.arch === "arm64")
    ? "aarch64"
    : "x64";
  const tauriConfig = JSON.parse(await readFile(resolve("src-tauri", "tauri.conf.json"), "utf8"));
  const fileProductName = tauriConfig.productName.replaceAll(" ", "_");
  const bundleDirectory = join(
    await cargoTargetDirectory(),
    ...(target ? [target] : []),
    "release",
    "bundle",
  );
  return {
    bundleDirectory,
    productName: tauriConfig.productName,
    stem: `${fileProductName}_${tauriConfig.version}_${architecture}`,
  };
}

async function macosDmgPath(buildArgs) {
  const { bundleDirectory, stem } = await bundleLayout(buildArgs);
  return join(bundleDirectory, "dmg", `${stem}.dmg`);
}

// Tauri names the macOS updater archive after the product alone. Give it the
// DMG's stem and require its signature; a build that asked for updater
// artifacts and produced none must not pass.
async function stageMacosUpdaterArchive(buildArgs) {
  const { bundleDirectory, productName, stem } = await bundleLayout(buildArgs);
  const generated = join(bundleDirectory, "macos", `${productName}.app.tar.gz`);
  const staged = join(bundleDirectory, "macos", `${stem}.app.tar.gz`);
  for (const suffix of ["", ".sig"]) {
    if (!existsSync(`${generated}${suffix}`)) {
      throw new Error(`Expected updater artifact at ${generated}${suffix}.`);
    }
  }
  for (const suffix of ["", ".sig"]) {
    await rm(`${staged}${suffix}`, { force: true });
    await rename(`${generated}${suffix}`, `${staged}${suffix}`);
  }
  // The renamed archive leaves no `Mallard.app.tar.gz` beside the `.app`, and
  // tauri-action reacts to that by tarring the directory itself and uploading
  // the result under the very name the signed archive claims. That copy is
  // unsigned and the staged `.sig` does not cover it, so the bundle directory
  // must not keep the `.app` once its signed archive exists. The DMG is
  // already built and verification reads the DMG and the archive.
  await rm(join(bundleDirectory, "macos", `${productName}.app`), {
    recursive: true,
    force: true,
  });
  return staged;
}

async function requireWindowsUpdaterSignature(buildArgs) {
  const { bundleDirectory, stem } = await bundleLayout(buildArgs);
  const signature = join(bundleDirectory, "nsis", `${stem}-setup.exe.sig`);
  if (!existsSync(signature)) throw new Error(`Expected updater signature at ${signature}.`);
  return signature;
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

// Updater artifacts (`.app.tar.gz`/`-setup.exe` plus a minisign `.sig`) need
// the private key, so they are requested per build rather than committed in
// tauri.conf.json: a local build without the key stays an ordinary build, and
// a release build that lacks it fails closed instead of shipping unsigned.
function updaterArtifactsRequested() {
  const hasKey = Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY);
  if (process.env.MALLARD_REQUIRE_UPDATER_ARTIFACTS === "1" && !hasKey) {
    throw new Error("MALLARD_REQUIRE_UPDATER_ARTIFACTS needs TAURI_SIGNING_PRIVATE_KEY.");
  }
  return hasKey;
}

function withPackagedCliConfig(buildArgs) {
  if (buildArgs[0] !== "build") return buildArgs;
  const bundle = { externalBin: ["binaries/mallard-cli"] };
  if (updaterArtifactsRequested()) bundle.createUpdaterArtifacts = true;
  const overlay = JSON.stringify({ bundle });
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

// Resolve the overlay first: a release job missing the updater key fails
// here, before any compilation.
const tauriArgs = withPackagedCliConfig(args);
await stageCliBinary(args);
await runChecked(process.execPath, [tauriCliScript, ...tauriArgs]);

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

const updaterRequested = args[0] === "build" && updaterArtifactsRequested();
const updaterArchivePath = updaterRequested && macosBuild ? await stageMacosUpdaterArchive(args) : undefined;
if (updaterRequested && process.platform === "win32") await requireWindowsUpdaterSignature(args);

if (credentials || verifyBundle) {
  const dmgPath = await macosDmgPath(args);
  if (!existsSync(dmgPath)) throw new Error(`Expected macOS DMG at ${dmgPath}.`);

  if (credentials) await notarizeDmg(dmgPath, credentials);

  if (verifyBundle) {
    const target = buildTarget(args);
    const expectedArch = target === "x86_64-apple-darwin" || (!target && process.arch === "x64")
      ? "x86_64"
      : "arm64";
    await runChecked(process.execPath, [
      resolve("scripts", "verify-macos-bundle.mjs"),
      dmgPath,
      expectedArch,
      ...(updaterArchivePath ? [updaterArchivePath] : []),
    ]);
  }
}
