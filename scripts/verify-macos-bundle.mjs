import { execFile, spawn } from "node:child_process";
import { constants } from "node:fs";
import { access, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

function run(command, args, options = {}) {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(command, args, { stdio: "inherit", ...options });
    child.once("error", rejectRun);
    child.once("exit", (code) => {
      if (code === 0) resolveRun();
      else rejectRun(new Error(`${command} exited with status ${code ?? 1}.`));
    });
  });
}

function capture(command, args) {
  return new Promise((resolveCapture, rejectCapture) => {
    execFile(command, args, { maxBuffer: 1024 * 1024 }, (error, stdout, stderr) => {
      if (error) rejectCapture(error);
      else resolveCapture({ stdout, stderr });
    });
  });
}

async function verifyCliHelp(cliPath) {
  const { stdout, stderr } = await capture(cliPath, ["--help"]);
  const output = `${stdout}\n${stderr}`;
  if (!/Usage:\s+mallard-cli/i.test(output) || !/storage/i.test(output)) {
    throw new Error(`Packaged ${basename(cliPath)} did not return the expected help output.`);
  }
}

export async function verifyMacosBundle(dmgPath) {
  if (process.platform !== "darwin") {
    throw new Error("macOS bundle verification can only run on macOS.");
  }

  await access(dmgPath, constants.R_OK);
  await run("hdiutil", ["verify", dmgPath]);

  const mountPath = await mkdtemp(join(tmpdir(), "mallard-release-dmg-"));
  const installPath = await mkdtemp(join(tmpdir(), "mallard-release-cli-"));
  let mounted = false;
  try {
    await run("hdiutil", ["attach", "-nobrowse", "-readonly", "-mountpoint", mountPath, dmgPath]);
    mounted = true;

    const appPath = join(mountPath, "Mallard.app");
    const cliPath = join(appPath, "Contents", "MacOS", "mallard-cli");
    await access(appPath, constants.R_OK);
    await access(cliPath, constants.R_OK | constants.X_OK);

    await run("codesign", ["--verify", "--deep", "--strict", "--verbose=4", appPath]);
    await run("codesign", ["--verify", "--strict", "--verbose=4", cliPath]);
    await verifyCliHelp(cliPath);

    const installedApp = join(installPath, "Mallard.app");
    await run("ditto", [appPath, installedApp]);
    await run("hdiutil", ["detach", mountPath]);
    mounted = false;

    const installedCli = join(installedApp, "Contents", "MacOS", "mallard-cli");
    await access(installedCli, constants.R_OK | constants.X_OK);
    await run("codesign", ["--verify", "--deep", "--strict", "--verbose=4", installedApp]);
    await run("codesign", ["--verify", "--strict", "--verbose=4", installedCli]);
    await verifyCliHelp(installedCli);
  } finally {
    if (mounted) await run("hdiutil", ["detach", mountPath]);
    await rm(mountPath, { recursive: true, force: true });
    await rm(installPath, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const dmgPath = process.argv[2];
  if (!dmgPath) throw new Error("Usage: node scripts/verify-macos-bundle.mjs <Mallard.dmg>");
  await verifyMacosBundle(dmgPath);
}
