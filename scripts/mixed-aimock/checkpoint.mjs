import assert from "node:assert/strict";
import { cp, mkdir, rm, lstat, readFile, copyFile } from "node:fs/promises";
import { join, basename } from "node:path";
import { copyProviderHome, files } from "./native.mjs";

export class Checkpoints {
  constructor(root, replicas) { this.root = root; this.replicas = replicas; }
  path(name) { assert.match(name, /^[a-z0-9-]{1,40}$/u); return join(this.root, "snapshots", `checkpoint-${name}`); }
  async save(name) {
    const destination = this.path(name);
    await mkdir(destination, { mode: 0o700 });
    for (const replica of Object.values(this.replicas)) {
      const base = join(destination, replica.name); await mkdir(base, { mode: 0o700 });
      for (const name of ["machine", basename(replica.project)]) {
        const from = join(replica.base, name); await files(from);
        await cp(from, join(base, name), { recursive: true, dereference: false });
      }
      await copyProviderHome(replica.codex, join(base, "codex"), "codex");
      await copyProviderHome(replica.claude, join(base, "claude"), "claude");
    }
    await files(join(this.root, "storage"));
    await cp(join(this.root, "storage"), join(destination, "storage"), { recursive: true, dereference: false });
  }
  async restore(name) {
    const source = this.path(name); assert.ok((await lstat(source)).isDirectory(), "checkpoint missing");
    for (const replica of Object.values(this.replicas)) {
      for (const name of ["machine", "codex", "claude", basename(replica.project)]) {
        const destination = join(replica.base, name);
        await rm(destination, { recursive: true, force: true });
        const from = join(source, replica.name, name); await files(from);
        await cp(from, destination, { recursive: true, dereference: false });
      }
    }
    await rm(join(this.root, "storage"), { recursive: true, force: true });
    await cp(join(source, "storage"), join(this.root, "storage"), { recursive: true, dereference: false });
  }
  async installAppend(name) {
    let changed = 0; let appendedBytes = 0; let appendOnly = true;
    for (const provider of ["codex", "claude"]) {
      const from = join(this.path(name), "a", provider);
      for (const source of await files(from)) {
        if (!source.endsWith(".jsonl")) continue;
        const destination = join(this.replicas.a[provider], source.slice(from.length + 1));
        const before = await readFile(destination); const after = await readFile(source);
        if (before.equals(after)) continue;
        appendOnly &&= after.subarray(0, before.length).equals(before);
        appendedBytes += Math.max(0, after.length - before.length); changed++;
        await copyFile(source, destination);
      }
    }
    assert.ok(changed > 0 && changed <= 3, "native append fixture file count");
    return { changed_files: changed, appended_bytes: appendedBytes, append_only: appendOnly };
  }
}
