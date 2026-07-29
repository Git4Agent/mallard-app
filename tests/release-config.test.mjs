import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

const projectUrl = new URL("../", import.meta.url);
const read = (path) => readFile(new URL(path, projectUrl), "utf8");

test("release version is 0.1.2 in every source", async () => {
  const [tauriConfigSource, cargoSource, packageSource, packageLockSource] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read("src-tauri/Cargo.toml"),
    read("package.json"),
    read("package-lock.json"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const packageConfig = JSON.parse(packageSource);
  const packageLock = JSON.parse(packageLockSource);

  assert.equal(tauriConfig.version, "0.1.2");
  assert.match(cargoSource, /^version = "0\.1\.2"$/m);
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

test("desktop release builds one verified Apple Silicon DMG", async () => {
  const [tauriConfigSource, workflow, releaseGuide, packageSource, tauriBuildScript] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read(".github/workflows/release.yml"),
    read("docs/RELEASING.md"),
    read("package.json"),
    read("scripts/tauri-build.mjs"),
  ]);
  const tauriConfig = JSON.parse(tauriConfigSource);
  const packageConfig = JSON.parse(packageSource);

  assert.deepEqual(tauriConfig.bundle.macOS, { signingIdentity: "-" });
  assert.match(workflow, /--target aarch64-apple-darwin/);
  assert.doesNotMatch(workflow, /x86_64-apple-darwin|windows-latest|x86_64-pc-windows-msvc/);
  assert.match(workflow, /--bundles dmg/);
  assert.match(workflow, /releaseDraft: true/);
  assert.match(workflow, /MALLARD_VERIFY_MACOS_BUNDLE: "1"/);
  assert.doesNotMatch(workflow, /TAURI_SIGNING|uploadUpdater|updaterJson|APPLE_[A-Z_]+/);
  assert.equal(packageConfig.scripts.tauri, "node scripts/tauri-build.mjs");
  assert.match(tauriBuildScript, /"hdiutil", \["verify", dmgPath\]/);
  assert.match(tauriBuildScript, /"hdiutil", \["attach", "-nobrowse", "-readonly"/);
  assert.match(tauriBuildScript, /join\(mountPath, "Mallard\.app"\)/);
  assert.match(
    tauriBuildScript,
    /"codesign", \["--verify", "--deep", "--strict", "--verbose=4"/,
  );
  assert.match(tauriBuildScript, /"hdiutil", \["detach", mountPath\]/);
  assert.doesNotMatch(tauriBuildScript, /"bundle",\s*"macos"/);
  assert.match(releaseGuide, /Apple Silicon/i);
  assert.match(releaseGuide, /Privacy & Security/i);
  assert.doesNotMatch(releaseGuide, /updater|TAURI_SIGNING|latest\.json/i);
});
