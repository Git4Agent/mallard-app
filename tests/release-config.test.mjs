import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

const projectUrl = new URL("../", import.meta.url);
const read = (path) => readFile(new URL(path, projectUrl), "utf8");

test("release version is 0.1.3 in every source", async () => {
  const [tauriConfigSource, workspaceCargoSource, cargoSource, packageSource, packageLockSource] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read("Cargo.toml"),
    read("src-tauri/Cargo.toml"),
    read("package.json"),
    read("package-lock.json"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const packageConfig = JSON.parse(packageSource);
  const packageLock = JSON.parse(packageLockSource);

  assert.equal(tauriConfig.version, "0.1.3");
  assert.match(workspaceCargoSource, /^version = "0\.1\.3"$/m);
  assert.match(cargoSource, /^version\.workspace = true$/m);
  assert.equal(packageConfig.version, "0.1.3");
  assert.equal(packageLock.version, "0.1.3");
  assert.equal(packageLock.packages[""].version, "0.1.3");
});

// The signed updater was removed in af20e1a because no Apple signing existed
// and an in-place update to an unnotarized app fails Gatekeeper. Trusted
// releases restored the precondition; docs/DESIGN_APP_UPDATES.md is the
// design. These assertions pin the trust boundary, not the UI.
test("application ships the signed in-app updater", async () => {
  const [
    tauriConfigSource,
    cargoSource,
    libSource,
    capabilitySource,
    packageSource,
    settingsSource,
    tauriBuildScript,
    releaseGuide,
  ] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read("src-tauri/Cargo.toml"),
    read("src-tauri/src/lib.rs"),
    read("src-tauri/capabilities/default.json"),
    read("package.json"),
    read("crates/mallard-infrastructure/src/app_settings.rs"),
    read("scripts/tauri-build.mjs"),
    read("docs/RELEASING.md"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const capability = JSON.parse(capabilitySource);
  const packageConfig = JSON.parse(packageSource);

  await access(new URL("src/components/AppUpdater.tsx", projectUrl));
  await access(new URL("tests/frontend/app-updater.integration.test.tsx", projectUrl));
  await access(new URL("docs/DESIGN_APP_UPDATES.md", projectUrl));

  // One endpoint, on GitHub, over HTTPS, resolved by the "latest" published
  // release; a draft never reaches an installed copy.
  assert.deepEqual(tauriConfig.plugins.updater.endpoints, [
    "https://github.com/Git4Agent/mallard-app/releases/latest/download/latest.json",
  ]);
  assert.match(tauriConfig.plugins.updater.pubkey, /^[A-Za-z0-9+/=_-]+$/, "a committed minisign public key");
  assert.equal(tauriConfig.plugins.updater.windows?.installMode, "passive");
  // Artifact creation is a per-build request from the wrapper, never a
  // committed setting: a local build without the private key must still build.
  assert.equal(tauriConfig.bundle.createUpdaterArtifacts, undefined);
  assert.match(tauriBuildScript, /process\.env\.TAURI_SIGNING_PRIVATE_KEY/);
  assert.match(tauriBuildScript, /MALLARD_REQUIRE_UPDATER_ARTIFACTS === "1" && !hasKey/);
  assert.match(tauriBuildScript, /if \(updaterArtifactsRequested\(\)\) bundle\.createUpdaterArtifacts = true;/);
  // Both macOS builds emit `Mallard.app.tar.gz`; the stem rename keeps them apart.
  assert.match(tauriBuildScript, /`\$\{productName\}\.app\.tar\.gz`/);
  assert.match(tauriBuildScript, /`\$\{stem\}\.app\.tar\.gz`/);
  assert.match(tauriBuildScript, /`\$\{stem\}-setup\.exe\.sig`/);
  assert.match(tauriBuildScript, /\.\.\.\(updaterArchivePath \? \[updaterArchivePath\] : \[\]\)/);

  assert.match(cargoSource, /^tauri-plugin-updater = "2"$/m);
  assert.match(cargoSource, /^tauri-plugin-process = "2"$/m);
  assert.match(libSource, /\.plugin\(tauri_plugin_process::init\(\)\)/);
  assert.match(libSource, /\.plugin\(tauri_plugin_updater::Builder::new\(\)\.build\(\)\)/);
  assert.ok(capability.permissions.includes("updater:default"));
  assert.ok(capability.permissions.includes("process:allow-restart"));
  assert.match(packageConfig.dependencies["@tauri-apps/plugin-updater"], /^\^2/);
  assert.match(packageConfig.dependencies["@tauri-apps/plugin-process"], /^\^2/);

  // The scheduled check is the user's to switch off, and switching it must
  // not purge caches like a tuning change would.
  assert.match(settingsSource, /pub update_check_enabled: bool,/);
  assert.match(settingsSource, /update_check_enabled: true,/);
  const tuningDiffers = settingsSource.match(/fn tuning_differs[\s\S]*?\n    \}/)?.[0] ?? "";
  assert.doesNotMatch(tuningDiffers, /update_check_enabled/);

  assert.match(releaseGuide, /TAURI_SIGNING_PRIVATE_KEY/);
  assert.match(releaseGuide, /latest\.json/);
});

test("release packaging contains no legacy Codex fork executor", async () => {
  const [tauriConfigSource, infrastructureCargo, tauriCargo, infrastructureLib] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read("crates/mallard-infrastructure/Cargo.toml"),
    read("src-tauri/Cargo.toml"),
    read("crates/mallard-infrastructure/src/lib.rs"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);

  await assert.rejects(
    access(new URL("crates/mallard-infrastructure/src/codex_native_fork.rs", projectUrl)),
    (error) => error?.code === "ENOENT",
  );
  await assert.rejects(
    access(new URL("scripts/codex-fork-and-exit", projectUrl)),
    (error) => error?.code === "ENOENT",
  );
  assert.doesNotMatch(infrastructureCargo, /portable-pty/);
  assert.doesNotMatch(tauriCargo, /portable-pty/);
  assert.doesNotMatch(infrastructureLib, /codex_native_fork/);
  assert.equal(tauriConfig.bundle.resources, undefined);
  assert.equal(tauriConfig.bundle.externalBin, undefined);
});

test("Mallard CLI is built and staged as a target-specific external binary", async () => {
  const [tauriConfigSource, cliCargoSource, buildScript, verifyScript, packageSource] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read("crates/mallard-cli/Cargo.toml"),
    read("scripts/tauri-build.mjs"),
    read("scripts/verify-macos-bundle.mjs"),
    read("package.json"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const packageConfig = JSON.parse(packageSource);

  // The generated sidecar must be required only by wrapped Tauri builds, so
  // ordinary Cargo checks never depend on an uncommitted binary artifact.
  assert.equal(tauriConfig.bundle.externalBin, undefined);
  assert.match(cliCargoSource, /^clap = \{ version = "4\.5", features = \["derive"\] \}$/m);
  assert.match(cliCargoSource, /^mallard-application = \{ path = "\.\.\/mallard-application" \}$/m);
  assert.match(buildScript, /command !== "build" && command !== "dev"/);
  assert.match(buildScript, /if \(command === "dev"\) return/);
  assert.match(buildScript, /\["build", "--locked", "-p", "mallard-cli"\]/);
  assert.match(buildScript, /cargoArgs\.push\("--target", explicitTarget\)/);
  assert.match(buildScript, /`mallard-cli-\$\{target\}\$\{extension\}`/);
  assert.match(buildScript, /target\.includes\("windows"\)/);
  assert.match(buildScript, /const extension = windows \? "\.exe" : ""/);
  assert.match(buildScript, /externalBin: \["binaries\/mallard-cli"\]/);
  assert.match(buildScript, /withPackagedCliConfig\(args\)/);
  assert.match(buildScript, /verify-macos-bundle\.mjs/);
  assert.match(verifyScript, /join\(appPath, "Contents", "MacOS", "mallard-cli"\)/);
  assert.match(verifyScript, /capture\(cliPath, \["--help"\]\)/);
  assert.match(verifyScript, /"codesign", \["--verify", "--strict", "--verbose=4", cliPath\]/);
  assert.equal(packageConfig.scripts["test:cli"], "cargo test -p mallard-cli");
});

test("desktop release builds verified Apple Silicon, Intel, and Windows artifacts", async () => {
  const [
    tauriConfigSource,
    workflow,
    releaseGuide,
    packageSource,
    tauriBuildScript,
    macosVerifier,
    windowsVerifier,
    entitlements,
    infoPlist,
  ] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read(".github/workflows/release.yml"),
    read("docs/RELEASING.md"),
    read("package.json"),
    read("scripts/tauri-build.mjs"),
    read("scripts/verify-macos-bundle.mjs"),
    read("scripts/verify-windows-bundle.ps1"),
    read("src-tauri/Entitlements.plist"),
    read("src-tauri/Info.plist"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const packageConfig = JSON.parse(packageSource);
  const macosJobIndex = workflow.indexOf("  publish_macos:");
  const intelJobIndex = workflow.indexOf("  publish_macos_intel:");
  const windowsJobIndex = workflow.indexOf("  publish_windows:");
  const manifestJobIndex = workflow.indexOf("  publish_updater_manifest:");
  const macosJob = workflow.slice(macosJobIndex, intelJobIndex);
  const intelJob = workflow.slice(intelJobIndex, windowsJobIndex);
  const windowsJob = workflow.slice(windowsJobIndex, manifestJobIndex);
  const manifestJob = workflow.slice(manifestJobIndex);

  // Local builds without the certificate stay ad-hoc; CI overrides the
  // identity through APPLE_SIGNING_IDENTITY, which the Tauri CLI reads first.
  assert.deepEqual(tauriConfig.bundle.macOS, {
    signingIdentity: "-",
    hardenedRuntime: true,
    entitlements: "Entitlements.plist",
    minimumSystemVersion: "12.0",
  });
  // osascript Apple Events (Open in Terminal, Codex App refresh) are refused
  // under the hardened runtime without this entitlement.
  assert.match(entitlements, /<key>com\.apple\.security\.automation\.apple-events<\/key>\s*<true\/>/);
  assert.match(infoPlist, /<key>NSAppleEventsUsageDescription<\/key>\s*<string>[^<]+<\/string>/);
  assert.equal(tauriConfig.bundle.windows?.nsis?.installMode, "currentUser");
  assert.ok(macosJobIndex >= 0, "macOS release job is present");
  assert.ok(intelJobIndex > macosJobIndex, "Intel release job follows the Apple Silicon job");
  assert.ok(windowsJobIndex > intelJobIndex, "Windows release job follows the Intel job");
  assert.match(workflow, /--target aarch64-apple-darwin/);
  assert.match(workflow, /--target x86_64-apple-darwin/);
  assert.match(workflow, /--target x86_64-pc-windows-msvc/);
  assert.doesNotMatch(workflow, /universal-apple-darwin|aarch64-pc-windows|i686-pc-windows/);
  // `app` must be requested explicitly: Tauri only emits the `.app.tar.gz`
  // updater archive for that target, and `dmg` alone builds the app bundle
  // without counting it as one.
  assert.match(workflow, /--bundles app,dmg/);
  assert.doesNotMatch(workflow, /--bundles dmg\b/);
  // Staging renames the signed archive; the unsigned `.app` left behind would
  // otherwise be tarred by tauri-action and uploaded under the same name.
  assert.match(tauriBuildScript, /rm\(join\(bundleDirectory, "macos", `\$\{productName\}\.app`\)/);
  // The Intel job cross-compiles on the Apple Silicon runner and attaches to
  // the draft that tauri-action created in the Apple Silicon job.
  assert.match(intelJob, /needs: publish_macos/);
  assert.match(intelJob, /runs-on: macos-latest/);
  assert.match(intelJob, /softwareupdate --install-rosetta --agree-to-license/);
  assert.match(intelJob, /MALLARD_VERIFY_MACOS_BUNDLE: "1"/);
  assert.match(intelJob, /MALLARD_REQUIRE_TRUSTED_MACOS: "1"/);
  assert.match(intelJob, /gh release view "\$RELEASE_TAG" .* --json isDraft/);
  assert.match(intelJob, /gh release upload "\$RELEASE_TAG" "\$\{dmgs\[0\]\}"/);
  assert.doesNotMatch(intelJob, /--clobber|tauri-action/);
  assert.match(windowsJob, /needs: publish_macos/);
  assert.match(windowsJob, /runs-on: windows-latest/);
  assert.match(windowsJob, /npm run tauri build --/);
  assert.match(windowsJob, /run: npm run test:cli/);
  assert.match(windowsJob, /--bundles nsis/);
  assert.match(windowsJob, /verify-windows-bundle\.ps1/);
  assert.match(
    windowsJob,
    /INSTALLER_PATH: \$\{\{ steps\.verify_windows\.outputs\.installer_path \}\}/,
  );
  assert.match(windowsJob, /gh release view .* --json isDraft/);
  assert.match(windowsJob, /gh release upload \$env:RELEASE_TAG \$env:INSTALLER_PATH \$signature/);
  assert.doesNotMatch(windowsJob, /--clobber/);
  assert.match(workflow, /releaseDraft: true/);

  // Updater artifacts: every build job signs with the same key, fails closed
  // without it, and hands its signature to one manifest job that writes
  // latest.json whole after all three have attached their assets — so no job
  // uploads a partial manifest and none needs --clobber to repair another's.
  for (const job of [macosJob, intelJob, windowsJob]) {
    assert.match(job, /TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/);
    assert.match(job, /TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}/);
    assert.match(job, /MALLARD_REQUIRE_UPDATER_ARTIFACTS: "1"/);
    assert.match(job, /name: updater-sig-(darwin-aarch64|darwin-x86_64|windows-x86_64)/);
  }
  assert.doesNotMatch(workflow, /uploadUpdaterJson|updaterJson|--clobber/);
  assert.match(macosJob, /Mallard_\*_aarch64\.app\.tar\.gz/);
  assert.match(macosJob, /gh release upload "\$RELEASE_TAG" "\$\{archives\[0\]\}" "\$\{archives\[0\]\}\.sig"/);
  assert.match(intelJob, /Mallard_\*_x64\.app\.tar\.gz/);
  assert.match(intelJob, /"\$\{archives\[0\]\}" "\$\{archives\[0\]\}\.sig"/);
  assert.match(windowsJob, /\$signature = "\$env:INSTALLER_PATH\.sig"/);
  assert.match(manifestJob, /needs: \[publish_macos, publish_macos_intel, publish_windows\]/);
  assert.match(manifestJob, /pattern: updater-sig-\*/);
  assert.match(manifestJob, /node scripts\/build-updater-manifest\.mjs/);
  assert.match(manifestJob, /gh release view "\$RELEASE_TAG" .* --json isDraft/);
  assert.match(manifestJob, /--json assets/);
  assert.match(manifestJob, /gh release upload "\$RELEASE_TAG" latest\.json/);
  assert.match(manifestJob, /name: updater-manifest/);
  assert.match(packageConfig.scripts["test:release-config"], /tests\/updater-manifest\.test\.mjs/);
  assert.match(workflow, /MALLARD_VERIFY_MACOS_BUNDLE: "1"/);
  assert.match(workflow, /MALLARD_REQUIRE_TRUSTED_MACOS: "1"/);
  assert.match(workflow, /run: npm run test:cli/);
  for (const secret of [
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_TEAM_ID",
    "APPLE_API_KEY",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY_CONTENT",
  ]) {
    assert.match(workflow, new RegExp(`${secret}: \\$\\{\\{ secrets\\.${secret} \\}\\}`));
  }
  assert.match(workflow, /key_path="\$RUNNER_TEMP\/AuthKey\.p8"/);
  assert.match(workflow, /APPLE_API_KEY_PATH=\$key_path" >> "\$GITHUB_ENV"/);
  assert.doesNotMatch(workflow, /echo[^\n]*APPLE_(CERTIFICATE|API_KEY_CONTENT)\b/);
  // Manual dispatch is a dry run that never creates or touches a release.
  assert.match(workflow, /^  workflow_dispatch:$/m);
  const dispatchSteps = workflow.split(/\n      - name: /).filter((step) => step.includes("workflow_dispatch"));
  assert.ok(dispatchSteps.length >= 2, "dry-run steps exist");
  for (const step of dispatchSteps) {
    assert.doesNotMatch(step, /tagName|releaseDraft|gh release/);
  }
  assert.match(workflow, /uses: actions\/upload-artifact@v6/);
  // Windows Authenticode signing and Apple ID password notarization are not
  // configured; the workflow must not pretend otherwise.
  assert.doesNotMatch(
    workflow,
    /WINDOWS_CERTIFICATE|certificateThumbprint|signtool|APPLE_ID\b|APPLE_PASSWORD/i,
  );
  assert.doesNotMatch(workflow, /echo[^\n]*TAURI_SIGNING_PRIVATE_KEY\b/);
  assert.equal(packageConfig.scripts.tauri, "node scripts/tauri-build.mjs");
  assert.match(tauriBuildScript, /verify-macos-bundle\.mjs/);
  // Tauri notarizes the .app; the wrapper notarizes and staples the DMG too.
  assert.match(tauriBuildScript, /"notarytool", "submit", dmgPath/);
  assert.match(tauriBuildScript, /status !== "Accepted"/);
  assert.match(tauriBuildScript, /"stapler", "staple", dmgPath/);
  assert.match(tauriBuildScript, /requireTrusted && !credentials/);
  assert.match(macosVerifier, /MALLARD_REQUIRE_TRUSTED_MACOS/);
  assert.match(macosVerifier, /Authority=Developer ID Application:/);
  assert.match(macosVerifier, /flags=0x\[0-9a-f\]\+\\\(\.\*runtime/);
  assert.match(macosVerifier, /com\.apple\.security\.automation\.apple-events/);
  assert.match(macosVerifier, /"spctl", \["--assess", "--type", "execute", "--verbose=4", appPath\]/);
  assert.match(macosVerifier, /source=Notarized Developer ID/);
  assert.match(macosVerifier, /"--type", "open", "--context", "context:primary-signature"/);
  assert.match(macosVerifier, /"stapler", "validate", appPath/);
  assert.match(macosVerifier, /"stapler", "validate", dmgPath/);
  assert.match(macosVerifier, /if \(requireTrusted\) await verifyTrustedApp\(installedApp\)/);
  assert.match(macosVerifier, /"hdiutil", \["verify", dmgPath\]/);
  assert.match(macosVerifier, /"lipo", \["-archs", binaryPath\]/);
  assert.match(macosVerifier, /expectedArch !== "arm64" && expectedArch !== "x86_64"/);
  assert.match(tauriBuildScript, /target === "x86_64-apple-darwin"/);
  assert.match(tauriBuildScript, /verify-macos-bundle\.mjs"\),\s*dmgPath,\s*expectedArch,/);
  assert.match(macosVerifier, /export async function verifyUpdaterArchive/);
  assert.match(macosVerifier, /"tar", \["-xzf", archivePath, "-C", extractPath\]/);
  assert.match(macosVerifier, /if \(updaterArchivePath\) await verifyUpdaterArchive\(updaterArchivePath, expectedArch\);/);
  assert.match(macosVerifier, /"hdiutil", \["attach", "-nobrowse", "-readonly"/);
  assert.match(macosVerifier, /join\(mountPath, "Mallard\.app"\)/);
  assert.match(
    macosVerifier,
    /"codesign", \["--verify", "--deep", "--strict", "--verbose=4"/,
  );
  assert.match(macosVerifier, /join\(appPath, "Contents", "MacOS", "mallard-cli"\)/);
  assert.match(macosVerifier, /"ditto", \[appPath, installedApp\]/);
  assert.match(macosVerifier, /join\(installedApp, "Contents", "MacOS", "mallard-cli"\)/);
  assert.match(macosVerifier, /verifyCliHelp\(installedCli\)/);
  assert.match(macosVerifier, /"hdiutil", \["detach", mountPath\]/);
  assert.match(windowsVerifier, /Get-ChildItem -LiteralPath \$bundleRoot -Filter "\*\.exe" -File/);
  assert.match(windowsVerifier, /\$installers\.Count -ne 1/);
  assert.match(windowsVerifier, /\.Arguments = "\/S \/NS \/D=\$InstallDirectory"/);
  assert.match(windowsVerifier, /\.Arguments = "\/S _\?=\$InstallDirectory"/);
  assert.match(windowsVerifier, /Join-Path \$installDirectory "mallard-cli\.exe"/);
  assert.match(windowsVerifier, /Join-Path \$installDirectory "uninstall\.exe"/);
  assert.match(windowsVerifier, /Copy-Item -LiteralPath \$installedUninstaller/);
  assert.match(windowsVerifier, /Invoke-Uninstaller -UninstallerPath \$copiedUninstaller/);
  assert.match(
    windowsVerifier,
    /Assert-DirectChildPath -Parent \$verificationRoot -Child \$installDirectory/,
  );
  assert.match(
    windowsVerifier,
    /Registry::HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Mallard/,
  );
  assert.match(
    windowsVerifier,
    /The NSIS uninstaller left the temporary installation directory behind/,
  );
  assert.match(
    windowsVerifier,
    /The NSIS uninstaller left current-user uninstall metadata behind/,
  );
  assert.match(windowsVerifier, /New-ProcessStartInfo -FilePath \$CliPath -Arguments @\("--help"\)/);
  assert.match(windowsVerifier, /\$process\.ExitCode -ne 0/);
  assert.match(windowsVerifier, /\^Usage:\\s\+mallard-cli/);
  assert.match(windowsVerifier, /Add-Content -LiteralPath \$GitHubOutputPath -Value "installer_path=/);
  assert.match(windowsVerifier, /GetEnvironmentVariable\("Path", "User"\)/);
  assert.match(windowsVerifier, /GetEnvironmentVariable\("Path", "Machine"\)/);
  assert.match(
    windowsVerifier,
    /Remove-Item -LiteralPath \$verificationRoot -Recurse -Force/,
  );
  assert.doesNotMatch(
    windowsVerifier,
    /Remove-Item[^\r\n]*(?:TemporaryRoot|bundleRoot|BundleDirectory|filesystemRoot)/,
  );
  assert.doesNotMatch(
    `${workflow}\n${windowsVerifier}`,
    /\bsetx(?:\.exe)?\b|SetEnvironmentVariable\s*\(|Microsoft\.PowerShell_profile|GITHUB_PATH|::add-path|\$env:Path\s*=/i,
  );
  assert.doesNotMatch(tauriBuildScript, /"bundle",\s*"macos"/);
  assert.match(releaseGuide, /Apple Silicon/i);
  assert.match(releaseGuide, /x86_64-apple-darwin/);
  assert.match(releaseGuide, /Rosetta/);
  assert.match(releaseGuide, /Windows x64 NSIS/i);
  assert.match(releaseGuide, /x86_64-pc-windows-msvc/);
  assert.match(releaseGuide, /does not add the CLI to `PATH`/i);
  assert.match(releaseGuide, /verify-windows-bundle\.ps1/);
  assert.match(releaseGuide, /`\/NS` flag/);
  assert.match(releaseGuide, /current-user uninstall\s+registry entry to be gone/i);
  assert.match(releaseGuide, /Developer ID Application/);
  assert.match(releaseGuide, /notariz/i);
  assert.match(releaseGuide, /APPLE_API_KEY_CONTENT/);
  assert.match(releaseGuide, /MALLARD_REQUIRE_TRUSTED_MACOS=1/);
  assert.match(releaseGuide, /workflow_dispatch|Run workflow/);
  assert.doesNotMatch(releaseGuide, /Open Anyway/i);
  assert.match(releaseGuide, /Updater signing/);
  assert.match(releaseGuide, /gh release edit .* --draft/);
});
