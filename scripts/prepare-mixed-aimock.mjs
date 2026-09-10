import { mkdtemp, realpath, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { buildWorker } from "./mixed-aimock/build.mjs";

const [baselineRef] = process.argv.slice(2);
if (!/^[a-f0-9]{40}$/u.test(baselineRef ?? "")) throw new Error("supply_reviewed_baseline_commit");
const root = await realpath(await mkdtemp(join(tmpdir(), "mallard-mixed-build-")));
try {
  await buildWorker(resolve("."), root, false, baselineRef);
  console.log("Candidate and baseline release workers are ready in the Cargo cache.");
} finally { await rm(root, { recursive: true, force: true }); }
