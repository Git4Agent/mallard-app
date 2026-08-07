import { execFile, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
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
  const target = targetIndex >= 0 ? buildArgs[targetIndex + 1] : undefined;
  if (targetIndex >= 0 && !target) throw new Error("--target requires a Rust target triple.");
  return target;
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

async function verifyMacosDmg(buildArgs) {
  const dmgPath = await macosDmgPath(buildArgs);
  if (!existsSync(dmgPath)) throw new Error(`Expected macOS DMG at ${dmgPath}.`);

  await runChecked("hdiutil", ["verify", dmgPath]);

  const mountPath = await mkdtemp(join(tmpdir(), "mallard-release-dmg-"));
  let mounted = false;
  try {
    await runChecked("hdiutil", ["attach", "-nobrowse", "-readonly", "-mountpoint", mountPath, dmgPath]);
    mounted = true;

    const appPath = join(mountPath, "Mallard.app");
    if (!existsSync(appPath)) throw new Error(`Expected Mallard.app inside ${dmgPath}.`);

    await runChecked("codesign", ["--verify", "--deep", "--strict", "--verbose=4", appPath]);
  } finally {
    if (mounted) await runChecked("hdiutil", ["detach", mountPath]);
    await rm(mountPath, { recursive: true, force: true });
  }
}

await runChecked(process.execPath, [tauriCliScript, ...args]);

if (process.env.MALLARD_VERIFY_MACOS_BUNDLE === "1") {
  if (process.platform !== "darwin") {
    throw new Error("MALLARD_VERIFY_MACOS_BUNDLE is only supported on macOS release runners.");
  }
  if (args[0] !== "build") {
    throw new Error("MALLARD_VERIFY_MACOS_BUNDLE requires the Tauri build command.");
  }

  await verifyMacosDmg(args);
}
