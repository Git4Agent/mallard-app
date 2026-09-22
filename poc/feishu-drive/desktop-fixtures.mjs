import { createHash, randomBytes } from "node:crypto";
import { lstat, mkdir, readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { pathToFileURL } from "node:url";
import { makeRecord, STREAM } from "./probe.mjs";
import { buildSelector } from "./selector.mjs";

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

// Create a new synthetic tree. Never read a provider home or reuse a directory.
export async function prepareDesktopFixtures() {
  const selector = buildSelector();
  const runId = randomBytes(8).toString("hex");
  const folderName = `mallard-feishu-desktop-poc-${runId}`;
  const directory = join(await realpath(tmpdir()), folderName);
  await mkdir(directory, { mode: 0o700 });
  const upload = join(directory, folderName);
  const root = makeRecord(selector, "1".repeat(24), null, 1_790_000_000_000);
  const winner = makeRecord(selector, "2".repeat(24), root, 1_790_000_000_001);
  const loser = makeRecord(selector, "3".repeat(24), root, 1_790_000_000_002);
  const small = Buffer.from(Array.from({ length: 4096 }, (_, index) => index % 256));
  const files = new Map([
    ["binary-4096.bin", small],
    ["over-20MiB.bin", Buffer.alloc(20 * 1024 * 1024 + 17, 0x6d)],
    ["same-name.bin", Buffer.from("Synthetic duplicate A. No user data.\n")],
    [winner.name, winner.bytes],
    [".mallard/_storage.json", Buffer.from('{"format":"mallard-append-storage","layout_version":1}')],
    [`${STREAM}/${root.name}`, root.bytes],
    [`${STREAM}/${winner.name}`, winner.bytes],
    [`${STREAM}/${loser.name}`, loser.bytes],
  ]);
  const expected = [];
  for (const [path, bytes] of files) {
    const target = join(upload, path);
    await mkdir(resolve(target, ".."), { recursive: true, mode: 0o700 });
    await writeFile(target, bytes, { flag: "wx", mode: 0o600 });
    expected.push({ path, size: bytes.length, sha256: digest(bytes) });
  }
  for (const [name, bytes] of [
    ["repeat", files.get("same-name.bin")],
    ["conflict", Buffer.from("Synthetic duplicate B. Different bytes.\n")],
  ]) {
    await mkdir(join(directory, name), { mode: 0o700 });
    await writeFile(join(directory, name, "same-name.bin"), bytes, { flag: "wx", mode: 0o600 });
  }
  const manifest = { format: "feishu-desktop-fixture-v1", folder_name: folderName,
    data: "synthetic_only", expected,
    record_name_length: winner.name.length,
    selected_chain: selector({ action: "select", names: [loser.name, root.name, winner.name] }).chain };
  const manifestPath = join(directory, "expected.json");
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + "\n", { flag: "wx", mode: 0o600 });
  return { directory, upload, manifest: manifestPath, folder_name: folderName,
    files: expected.length, record_name_length: winner.name.length };
}

// Verify an extracted folder download by exact relative path, size, and SHA-256.
// The caller must supply the root inside the downloaded archive.
export async function verifyDesktopDownload(manifestPath, downloadedRoot) {
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  if (manifest.format !== "feishu-desktop-fixture-v1") throw new Error("invalid_fixture_manifest");
  const actual = new Map();
  async function walk(directory, prefix = "") {
    if (!(await lstat(directory)).isDirectory()) throw new Error("download_directory_required");
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = prefix + entry.name;
      if (entry.isDirectory()) await walk(join(directory, entry.name), path + "/");
      else if (entry.isFile()) {
        const bytes = await readFile(join(directory, entry.name));
        actual.set(path, { size: bytes.length, sha256: digest(bytes) });
      } else throw new Error("download_special_file_rejected");
    }
  }
  await walk(downloadedRoot);
  const recordNames = [...actual.keys()].filter((path) => path.startsWith(STREAM + "/"))
    .map((path) => path.slice(STREAM.length + 1));
  const selected = buildSelector()({ action: "select", names: recordNames });
  const selectionMatches = JSON.stringify(selected.chain) === JSON.stringify(manifest.selected_chain);
  const missing = [];
  const changed = [];
  for (const expected of manifest.expected) {
    const file = actual.get(expected.path);
    if (!file) missing.push(expected.path);
    else if (file.size !== expected.size || file.sha256 !== expected.sha256) changed.push(expected.path);
    actual.delete(expected.path);
  }
  return { verdict: missing.length || changed.length || actual.size || !selectionMatches ? "FAIL" : "PASS",
    expected_files: manifest.expected.length, matched_files: manifest.expected.length - missing.length - changed.length,
    missing, changed, extra: [...actual.keys()], record_count: recordNames.length,
    selected_chain_length: selected.chain.length, selection_matches: selectionMatches };
}

async function main() {
  const [action, manifest, downloadedRoot, ...extra] = process.argv.slice(2);
  if (action === "prepare" && !manifest) console.log(JSON.stringify(await prepareDesktopFixtures(), null, 2));
  else if (action === "verify" && manifest && downloadedRoot && !extra.length) {
    const result = await verifyDesktopDownload(manifest, downloadedRoot);
    console.log(JSON.stringify(result, null, 2));
    if (result.verdict !== "PASS") process.exitCode = 1;
  } else throw new Error("usage: prepare | verify <expected.json> <downloaded-root>");
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1; });
}
