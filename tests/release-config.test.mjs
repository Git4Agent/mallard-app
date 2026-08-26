import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

const projectUrl = new URL("../", import.meta.url);
const read = (path) => readFile(new URL(path, projectUrl), "utf8");

test("release version is 0.1.2 in every source", async () => {
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

  assert.equal(tauriConfig.version, "0.1.2");
  assert.match(workspaceCargoSource, /^version = "0\.1\.2"$/m);
  assert.match(cargoSource, /^version\.workspace = true$/m);
  assert.equal(packageConfig.version, "0.1.2");
  assert.equal(packageLock.version, "0.1.2");
  assert.equal(packageLock.packages[""].version, "0.1.2");
});

test("application contains no automatic updater", async () => {
  const [
    tauriConfigSource,
    cargoSource,
    libSource,
    capabilitySource,
    packageSource,
    packageLockSource,
    appSource,
    projectSyncSource,
    projectSidebarSource,
    testRunnerSource,
  ] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read("src-tauri/Cargo.toml"),
    read("src-tauri/src/lib.rs"),
    read("src-tauri/capabilities/default.json"),
    read("package.json"),
    read("package-lock.json"),
    read("src/App.tsx"),
    read("src/components/project-sync/ProjectSync.tsx"),
    read("src/components/project-sync/ProjectSidebar.tsx"),
    read("scripts/run-frontend-integration-tests.mjs"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const packageConfig = JSON.parse(packageSource);

  await assert.rejects(
    access(new URL("src/components/AppUpdater.tsx", projectUrl)),
    (error) => error?.code === "ENOENT",
  );
  await assert.rejects(
    access(new URL("src/components/AppUpdateControl.tsx", projectUrl)),
    (error) => error?.code === "ENOENT",
  );
  await assert.rejects(
    access(new URL("tests/frontend/app-updater.integration.test.tsx", projectUrl)),
    (error) => error?.code === "ENOENT",
  );
  assert.equal(tauriConfig.bundle.createUpdaterArtifacts, undefined);
  assert.equal(tauriConfig.plugins?.updater, undefined);
  assert.equal(packageConfig.dependencies["@tauri-apps/plugin-updater"], undefined);
  assert.equal(packageConfig.dependencies["@tauri-apps/plugin-process"], undefined);
  for (const source of [
    cargoSource,
    libSource,
    capabilitySource,
    packageLockSource,
    appSource,
    projectSyncSource,
    projectSidebarSource,
    testRunnerSource,
  ]) {
    assert.doesNotMatch(
      source,
      /updater|AppUpdater|AppUpdateControl|check-for-updates|onBusyChange|tauri-plugin-process|plugin-process|allow-restart/i,
    );
  }
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

test("desktop release builds verified Apple Silicon and Windows artifacts", async () => {
  const [
    tauriConfigSource,
    workflow,
    releaseGuide,
    packageSource,
    tauriBuildScript,
    macosVerifier,
    windowsVerifier,
  ] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read(".github/workflows/release.yml"),
    read("docs/RELEASING.md"),
    read("package.json"),
    read("scripts/tauri-build.mjs"),
    read("scripts/verify-macos-bundle.mjs"),
    read("scripts/verify-windows-bundle.ps1"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const packageConfig = JSON.parse(packageSource);
  const macosJobIndex = workflow.indexOf("  publish_macos:");
  const windowsJobIndex = workflow.indexOf("  publish_windows:");
  const windowsJob = workflow.slice(windowsJobIndex);

  assert.deepEqual(tauriConfig.bundle.macOS, { signingIdentity: "-" });
  assert.equal(tauriConfig.bundle.windows?.nsis?.installMode, "currentUser");
  assert.ok(macosJobIndex >= 0, "macOS release job is present");
  assert.ok(windowsJobIndex > macosJobIndex, "Windows release job follows the macOS job");
  assert.match(workflow, /--target aarch64-apple-darwin/);
  assert.match(workflow, /--target x86_64-pc-windows-msvc/);
  assert.doesNotMatch(workflow, /x86_64-apple-darwin|aarch64-pc-windows|i686-pc-windows/);
  assert.match(workflow, /--bundles dmg/);
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
  assert.match(windowsJob, /gh release upload \$env:RELEASE_TAG \$env:INSTALLER_PATH/);
  assert.doesNotMatch(windowsJob, /--clobber/);
  assert.match(workflow, /releaseDraft: true/);
  assert.match(workflow, /MALLARD_VERIFY_MACOS_BUNDLE: "1"/);
  assert.match(workflow, /run: npm run test:cli/);
  assert.doesNotMatch(
    workflow,
    /TAURI_SIGNING|WINDOWS_CERTIFICATE|certificateThumbprint|signtool|uploadUpdater|updaterJson|APPLE_[A-Z_]+/i,
  );
  assert.equal(packageConfig.scripts.tauri, "node scripts/tauri-build.mjs");
  assert.match(tauriBuildScript, /verify-macos-bundle\.mjs/);
  assert.match(macosVerifier, /"hdiutil", \["verify", dmgPath\]/);
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
  assert.match(releaseGuide, /Windows x64 NSIS/i);
  assert.match(releaseGuide, /x86_64-pc-windows-msvc/);
  assert.match(releaseGuide, /does not add the CLI to `PATH`/i);
  assert.match(releaseGuide, /verify-windows-bundle\.ps1/);
  assert.match(releaseGuide, /`\/NS` flag/);
  assert.match(releaseGuide, /current-user uninstall\s+registry entry to be gone/i);
  assert.match(releaseGuide, /Privacy & Security/i);
  assert.doesNotMatch(releaseGuide, /updater|TAURI_SIGNING|latest\.json/i);
});
