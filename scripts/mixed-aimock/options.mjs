import assert from "node:assert/strict";

export function options(argv) {
  const result = { profile: "mixed", debug: false, benchmark: false, samples: 20, workers: 4 };
  const seen = new Set();
  for (const arg of argv) {
    const name = arg.split("=", 1)[0];
    assert.ok(!seen.has(name), "duplicate runner argument");
    seen.add(name);
    if (arg === "--debug") result.debug = true;
    else if (arg === "--benchmark") result.benchmark = true;
    else if (/^--profile=(mixed|xlarge)$/u.test(arg)) result.profile = arg.split("=")[1];
    else if (/^--baseline-ref=[a-f0-9]{40}$/u.test(arg)) result.baselineRef = arg.split("=")[1];
    else if (/^--workers=(1|4)$/u.test(arg)) result.workers = Number(arg.split("=")[1]);
    else if (/^--scale=(10|50|100)$/u.test(arg)) result.scale = Number(arg.split("=")[1]);
    else if (/^--samples=\d+$/u.test(arg)) result.samples = Number(arg.split("=")[1]);
    else throw new Error("unknown_runner_argument");
  }
  assert.ok(result.profile === "xlarge" || result.scale === undefined, "scale requires the xlarge profile");
  assert.ok(result.samples >= 20 && result.samples <= 100, "sample count must be 20..100");
  if (result.benchmark && result.debug) throw new Error("benchmark_requires_release_build");
  return result;
}
