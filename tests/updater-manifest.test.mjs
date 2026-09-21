import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import test from "node:test";
import {
  buildUpdaterManifest,
  updaterAssetName,
  UPDATER_PLATFORMS,
} from "../scripts/build-updater-manifest.mjs";

const run = promisify(execFile);

const signatures = {
  "darwin-aarch64": "untrusted comment: signature from tauri secret key\nAAA\n",
  "darwin-x86_64": "untrusted comment: signature from tauri secret key\nBBB\n",
  "windows-x86_64": "untrusted comment: signature from tauri secret key\nCCC\n",
};

test("the manifest names every platform's release asset under the tag it was built from", () => {
  const manifest = buildUpdaterManifest({
    version: "0.1.3",
    tag: "v0.1.3",
    signatures,
    pubDate: "2026-09-20T12:00:00.000Z",
  });
  assert.equal(manifest.version, "0.1.3");
  assert.equal(manifest.pub_date, "2026-09-20T12:00:00.000Z");
  assert.equal(manifest.notes, "https://github.com/Git4Agent/mallard-app/releases/tag/v0.1.3");
  assert.deepEqual(Object.keys(manifest.platforms), UPDATER_PLATFORMS);
  assert.equal(
    manifest.platforms["darwin-aarch64"].url,
    "https://github.com/Git4Agent/mallard-app/releases/download/v0.1.3/Mallard_0.1.3_aarch64.app.tar.gz",
  );
  assert.equal(
    manifest.platforms["darwin-x86_64"].url,
    "https://github.com/Git4Agent/mallard-app/releases/download/v0.1.3/Mallard_0.1.3_x64.app.tar.gz",
  );
  assert.equal(
    manifest.platforms["windows-x86_64"].url,
    "https://github.com/Git4Agent/mallard-app/releases/download/v0.1.3/Mallard_0.1.3_x64-setup.exe",
  );
  // Signatures are the .sig file's text, whitespace-trimmed, never re-encoded.
  assert.equal(manifest.platforms["windows-x86_64"].signature, "untrusted comment: signature from tauri secret key\nCCC");
  // The asset names match what tauri-build.mjs stages for each target.
  assert.equal(updaterAssetName("darwin-x86_64", "1.2.3"), "Mallard_1.2.3_x64.app.tar.gz");
});

test("the manifest refuses a missing signature, a foreign platform, a mismatched tag, or a bad version", () => {
  const { "windows-x86_64": _dropped, ...twoOnly } = signatures;
  assert.throws(() => buildUpdaterManifest({ version: "0.1.3", tag: "v0.1.3", signatures: twoOnly }), /windows-x86_64/);
  assert.throws(
    () => buildUpdaterManifest({ version: "0.1.3", tag: "v0.1.3", signatures: { ...signatures, "linux-x86_64": "x" } }),
    /Unexpected updater platforms: linux-x86_64/,
  );
  assert.throws(
    () => buildUpdaterManifest({ version: "0.1.3", tag: "v0.1.3", signatures: { ...signatures, "darwin-aarch64": "  " } }),
    /darwin-aarch64/,
  );
  assert.throws(() => buildUpdaterManifest({ version: "0.1.3", tag: "v0.1.4", signatures }), /does not match/);
  assert.throws(() => buildUpdaterManifest({ version: "0.1", tag: "v0.1", signatures }), /MAJOR\.MINOR\.PATCH/);
  assert.throws(() => buildUpdaterManifest({ version: "0.1.3", tag: "v0.1.3", signatures, pubDate: "soon" }), /not a date/);
  assert.throws(
    () => buildUpdaterManifest({ version: "0.1.3", tag: "v0.1.3", repository: "https://evil.example/x", signatures }),
    /owner\/name/,
  );
});

test("the CLI reads .sig files, pins the version to package.json, and writes latest.json", async () => {
  const directory = await mkdtemp(join(tmpdir(), "mallard-updater-manifest-"));
  try {
    const packageVersion = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8")).version;
    const args = ["scripts/build-updater-manifest.mjs", "--version", packageVersion, "--tag", `v${packageVersion}`, "--output", join(directory, "latest.json")];
    for (const platform of UPDATER_PLATFORMS) {
      const path = join(directory, `${platform}.sig`);
      await writeFile(path, signatures[platform]);
      args.push("--signature", `${platform}=${path}`);
    }
    await run(process.execPath, args, { cwd: new URL("../", import.meta.url) });
    const written = JSON.parse(await readFile(join(directory, "latest.json"), "utf8"));
    assert.equal(written.version, packageVersion);
    assert.equal(Object.keys(written.platforms).length, 3);
    assert.match(written.platforms["darwin-aarch64"].url, /releases\/download\/v[\d.]+\/Mallard_[\d.]+_aarch64\.app\.tar\.gz$/);

    await assert.rejects(
      run(process.execPath, [...args.slice(0, 2), "9.9.9", ...args.slice(3)], { cwd: new URL("../", import.meta.url) }),
      /does not match package\.json/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
