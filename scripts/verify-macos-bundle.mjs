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

// Trusted mode requires a Developer ID signature, notarization, and a stapled
// ticket on the application and the disk image.
const requireTrusted = process.env.MALLARD_REQUIRE_TRUSTED_MACOS === "1";

async function verifyDeveloperIdSignature(path) {
  const { stderr } = await capture("codesign", ["-dvv", path]);
  if (!/^Authority=Developer ID Application:/m.test(stderr)) {
    throw new Error(`${basename(path)} is not signed with a Developer ID Application certificate.`);
  }
  const teamId = process.env.APPLE_TEAM_ID;
  if (teamId && !stderr.includes(`TeamIdentifier=${teamId}`)) {
    throw new Error(`${basename(path)} is not signed by team ${teamId}.`);
  }
  if (!/^CodeDirectory .*flags=0x[0-9a-f]+\(.*runtime.*\)/m.test(stderr)) {
    throw new Error(`${basename(path)} was not signed with the hardened runtime.`);
  }
}

async function verifyTrustedApp(appPath) {
  await verifyDeveloperIdSignature(appPath);
  await verifyDeveloperIdSignature(join(appPath, "Contents", "MacOS", "mallard-cli"));

  const { stdout: entitlements } = await capture("codesign", ["-d", "--entitlements", "-", "--xml", appPath]);
  if (!entitlements.includes("com.apple.security.automation.apple-events")) {
    throw new Error("Mallard.app is missing the Apple Events automation entitlement.");
  }

  const { stderr: assessment } = await capture("spctl", ["--assess", "--type", "execute", "--verbose=4", appPath]);
  if (!/: accepted$/m.test(assessment) || !/source=Notarized Developer ID/.test(assessment)) {
    throw new Error(`Gatekeeper did not accept Mallard.app as a notarized Developer ID application:\n${assessment}`);
  }
  await run("xcrun", ["stapler", "validate", appPath]);
}

async function verifyTrustedDmg(dmgPath) {
  const { stderr: assessment } = await capture("spctl", [
    "--assess", "--type", "open", "--context", "context:primary-signature", "--verbose=4", dmgPath,
  ]);
  if (!/: accepted$/m.test(assessment)) {
    throw new Error(`Gatekeeper did not accept the DMG as notarized:\n${assessment}`);
  }
  await run("xcrun", ["stapler", "validate", dmgPath]);
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
  if (requireTrusted) await verifyTrustedDmg(dmgPath);

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
    if (requireTrusted) await verifyTrustedApp(appPath);
    await verifyCliHelp(cliPath);

    const installedApp = join(installPath, "Mallard.app");
    await run("ditto", [appPath, installedApp]);
    await run("hdiutil", ["detach", mountPath]);
    mounted = false;

    const installedCli = join(installedApp, "Contents", "MacOS", "mallard-cli");
    await access(installedCli, constants.R_OK | constants.X_OK);
    await run("codesign", ["--verify", "--deep", "--strict", "--verbose=4", installedApp]);
    await run("codesign", ["--verify", "--strict", "--verbose=4", installedCli]);
    if (requireTrusted) await verifyTrustedApp(installedApp);
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
