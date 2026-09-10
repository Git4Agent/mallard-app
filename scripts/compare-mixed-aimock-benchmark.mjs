import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function percentile(values, fraction) {
  if (!Array.isArray(values) || values.length < 20 || values.some((n) => !Number.isFinite(n) || n < 0)) throw new Error("invalid_samples");
  const sorted = [...values].sort((a, b) => a - b);
  if (fraction === 0.5) return (sorted[Math.floor((sorted.length - 1) / 2)] + sorted[Math.ceil((sorted.length - 1) / 2)]) / 2;
  return sorted[Math.ceil(sorted.length * fraction) - 1];
}

export function compareBenchmark(baseline, candidate) {
  try {
    if (candidate.version !== 1 || candidate.correctness !== "passed" || !Array.isArray(candidate.operations) || candidate.operations.length === 0) throw new Error("candidate_invalid");
    if (candidate.environment?.build !== "release") throw new Error("candidate_build_invalid");
    for (const operation of candidate.operations) {
      if (!/^[a-z0-9_]{1,64}$/u.test(operation.name)) throw new Error("operation_name_invalid");
      percentile(operation.samples.map((s) => s.ms), 0.5);
      percentile(operation.samples.map((s) => s.peak_rss_bytes), 1);
      if (operation.samples.some((s) => s.cpu_ms !== undefined && (!Number.isFinite(s.cpu_ms) || s.cpu_ms < 0))) throw new Error("cpu_sample_invalid");
    }
    if (!baseline) return { status: "uncalibrated", comparisons: [] };
    if (baseline.version !== 1 || baseline.correctness !== "passed") throw new Error("baseline_invalid");
    for (const key of ["fixture", "environment", "versions", "harness_digest"]) {
      if (JSON.stringify(baseline[key]) !== JSON.stringify(candidate[key]) || candidate[key] === undefined) throw new Error(`baseline_mismatch_${key}`);
    }
    const expected = new Map(baseline.operations.map((operation) => [operation.name, operation]));
    if (expected.size !== baseline.operations.length || expected.size !== candidate.operations.length || new Set(candidate.operations.map((o) => o.name)).size !== expected.size) throw new Error("operation_set_mismatch");
    const comparisons = [];
    for (const operation of candidate.operations) {
      const previous = expected.get(operation.name);
      if (!previous) throw new Error("operation_missing");
      const fast = /compare|timeline/u.test(operation.name);
      for (const [metric, fraction, ratio, floor] of [["median_ms", 0.5, 0.20, fast ? 5 : 100], ["p95_ms", 0.95, 0.25, fast ? 10 : 250], ["peak_rss_bytes", 1, 0.15, 32 * 1024 * 1024]]) {
        const field = metric === "peak_rss_bytes" ? metric : "ms";
        const before = percentile(previous.samples.map((s) => s[field]), fraction);
        const after = percentile(operation.samples.map((s) => s[field]), fraction);
        const allowance = Math.max(before * ratio, floor);
        comparisons.push({ operation: operation.name, metric, baseline: before, candidate: after, allowance, regression: after - before > allowance });
      }
    }
    return { status: comparisons.some((c) => c.regression) ? "regression" : "passed", comparisons };
  } catch (error) { return { status: "invalid", code: error.message, comparisons: [] }; }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [baselinePath, candidatePath, outputPath] = process.argv.slice(2);
  if (!baselinePath || !candidatePath || !outputPath) throw new Error("usage: compare-mixed-aimock-benchmark BASELINE CANDIDATE OUTPUT");
  const result = compareBenchmark(JSON.parse(await readFile(baselinePath, "utf8")), JSON.parse(await readFile(candidatePath, "utf8")));
  await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`);
  console.log(`Mixed benchmark: ${result.status}.`);
  if (result.status !== "passed") process.exitCode = 1;
}

export function confirmRegression(first, second) {
  if (first.status !== "regression") return first;
  if (!["passed", "regression"].includes(second.status)) return second;
  const repeated = first.comparisons.filter((item) => item.regression).some((item) =>
    second.comparisons.some((next) => next.regression && next.operation === item.operation && next.metric === item.metric));
  return { status: repeated ? "regression" : "unstable", comparisons: second.comparisons, confirmation: true };
}
