import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, readFile, copyFile, chmod, readdir } from "node:fs/promises";
import { join } from "node:path";
import { sha256 } from "./scenario.mjs";

function command(name, args, cwd, extra = {}) {
  const result = spawnSync(name, args, { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, timeout: 900_000, ...extra });
  if (result.status !== 0) throw new Error(`${name}_build_preflight_failed`);
  return result.stdout;
}
export async function buildWorker(repository, root, debug, baselineRef) {
  const args = ["test", "--manifest-path", "src-tauri/Cargo.toml", "--test", "live_mixed_aimock_e2e", "--no-run", "--locked", "--message-format=json"];
  if (!debug) args.push("--release");
  if (process.env.MALLARD_MIXED_NETWORK_ISOLATED === "1") args.push("--offline");
  const build = async (directory, name) => {
    const output = command("cargo", args, directory, { env: { ...process.env, CARGO_TARGET_DIR: join(repository, "target") } });
    const artifact = output.split("\n").filter(Boolean).map((line) => JSON.parse(line))
      .find((item) => item.reason === "compiler-artifact" && item.target.name === "live_mixed_aimock_e2e" && item.executable);
    assert.ok(artifact, "mixed worker executable missing");
    const destination = join(root, name); await copyFile(artifact.executable, destination); await chmod(destination, 0o700); return destination;
  };
  const binary = await build(repository, "candidate-worker");
  let baselineBinary;
  if (baselineRef) {
    assert.match(baselineRef, /^[a-f0-9]{40}$/u, "baseline must be a full reviewed commit ID");
    assert.equal(command("git", ["rev-parse", `${baselineRef}^{commit}`], repository).trim(), baselineRef);
    const source = join(root, "baseline-source"); await mkdir(source, { mode: 0o700 });
    const archive = command("git", ["archive", "--format=tar", baselineRef], repository, { encoding: null, maxBuffer: 256 * 1024 * 1024 });
    command("tar", ["-xf", "-", "-C", source], repository, { input: archive });
    // Both product revisions run the exact same private worker and fixture.
    await copyFile(join(repository, "src-tauri/tests/live_mixed_aimock_e2e.rs"), join(source, "src-tauri/tests/live_mixed_aimock_e2e.rs"));
    baselineBinary = await build(source, "baseline-worker");
  }
  return { binary, baselineBinary };
}
export async function harnessIdentity(repository) {
  const paths = ["scripts/run-mixed-aimock-e2e.mjs", "scripts/compare-mixed-aimock-benchmark.mjs", "scripts/live-codex-aimock-server.mjs", "scripts/live-claude-aimock-server.mjs", "src-tauri/tests/live_mixed_aimock_e2e.rs"];
  for (const name of (await readdir(join(repository, "scripts/mixed-aimock"))).sort()) if (name.endsWith(".mjs")) paths.push(`scripts/mixed-aimock/${name}`);
  const digests = [];
  for (const path of paths.sort()) digests.push([path, sha256(await readFile(join(repository, path)))]);
  return { harness_digest: sha256(JSON.stringify(digests)),
    product_commit: command("git", ["rev-parse", "HEAD"], repository).trim(),
    dirty: command("git", ["status", "--porcelain", "--untracked-files=normal"], repository).trim().length > 0,
    source_change_digest: sha256(command("git", ["diff", "HEAD", "--", "crates", "src-tauri", "Cargo.toml", "Cargo.lock"], repository)),
    rust: command("rustc", ["--version"], repository).trim() };
}
