import assert from "node:assert/strict";
import { ServiceWorker } from "./service.mjs";
import { compareBenchmark, percentile } from "../compare-mixed-aimock-benchmark.mjs";
import { auditStore } from "./native.mjs";
import { CANARIES } from "./scenario.mjs";
import { join } from "node:path";

function quiet(reply) {
  return reply.metrics.length === 2 && reply.metrics.every((m) => ["transcript_bytes_read", "files_head_read", "directories_visited", "probe_bytes_read", "append_bytes_read", "selected_captures", "full_graph_resolves"].every((key) => m[key] === 0));
}
async function prime(worker) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (quiet(await worker.call("compare"))) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error("warm_cache_did_not_settle");
}
function sample(reply, workers) {
  assert.equal(reply.ok, true, "benchmark operation failed");
  if (reply.value?.result) assert.equal(reply.value.result.success, true, "benchmark Apply failed");
  const phases = reply.value?.phases ?? [];
  for (const phase of phases.filter((p) => p.name.startsWith("prepare transcript "))) {
    assert.ok(phase.name.endsWith("2 semantic analyses)"), "Pull semantic analysis count");
  }
  const safePhases = phases.map(({ name, ms }) => {
    const prepared = name.match(/^prepare transcript (codex|claude):\S+ \((\d+) source bytes, (\d+) output bytes, (\d+) semantic analyses\)$/u);
    if (prepared) return { name: "prepare_transcript", provider: prepared[1], source_bytes: Number(prepared[2]), output_bytes: Number(prepared[3]), semantic_analyses: Number(prepared[4]), ms };
    const peak = name.match(/^prepare and install conversations \(peak (\d+) jobs, (\d+) source bytes\)$/u);
    if (peak) {
      assert.ok(Number(peak[1]) <= workers, "Pull exceeded worker cap");
      return { name: "prepare_and_install_conversations", peak_jobs: Number(peak[1]), peak_source_bytes: Number(peak[2]), ms };
    }
    return { name: name.replace(/(codex|claude):(?:conversation|session):[^\s]+/gu, "$1:fixture"), ms };
  });
  return { ms: reply.ms, peak_rss_bytes: reply.usage.peak_rss_bytes,
    cpu_ms: reply.usage.cpu_ms - reply.cpu_before.cpu_ms, metrics: reply.metrics,
    phases: safePhases };
}

export async function runBenchmark({ binary, baselineBinary, root, replicas, checkpoints, bundle, samples, identity, baselineIdentity, onProgress, timelineClouds, validateRestore }) {
  assert.ok(timelineClouds instanceof Map && timelineClouds.size === 3, "benchmark requires three history targets");
  assert.equal(typeof validateRestore, "function", "benchmark requires native restore validation");
  assert.ok(Number.isSafeInteger(samples) && samples >= 20 && samples <= 100, "benchmark sample count must be 20..100");
  const variants = [{ key: "candidate", binary, identity }];
  if (baselineBinary) variants.push({ key: "baseline", binary: baselineBinary, identity: baselineIdentity });
  const reports = Object.fromEntries(variants.map((variant) => [variant.key, { ...variant.identity, version: 1, correctness: "pending", operations: [] }]));
  const definitions = [
    { name: "cold_compare", checkpoint: "accepted", replica: "a", operation: "compare" },
    { name: "warm_compare", checkpoint: "accepted", replica: "a", operation: "compare", warm: true },
    { name: "append_compare_codex", checkpoint: "accepted", replica: "a", operation: "compare", warm: true, append: "one-local" },
    { name: "append_compare_claude", checkpoint: "accepted", replica: "a", operation: "compare", warm: true, append: "claude-local" },
    { name: "push_review_changed", checkpoint: "ten-local", replica: "a", operation: "push_review" },
    { name: "pull_review_accepted", checkpoint: "accepted", replica: "b", operation: "pull_review" },
    ...["one", "ten"].flatMap((size) => [
      { name: `push_${size}`, checkpoint: `${size}-local`, replica: "a", operation: "push_apply", prepare: "push_review" },
      { name: `pull_${size}`, checkpoint: `${size}-cloud`, replica: "b", operation: "pull_apply", prepare: "pull_review" },
    ]),
    { name: "push_review", checkpoint: "generated", replica: "a", operation: "push_review" },
    { name: "push_full", checkpoint: "generated", replica: "a", operation: "push_apply", prepare: "push_review" },
    { name: "pull_review", checkpoint: "published", replica: "b", operation: "pull_review" },
    { name: "pull_full", checkpoint: "published", replica: "b", operation: "pull_apply", prepare: "pull_review" },
    { name: "push_noop", checkpoint: "accepted", replica: "a", operation: "push_apply", prepare: "push_review" },
    { name: "pull_noop", checkpoint: "accepted", replica: "b", operation: "pull_apply", prepare: "pull_review" },

    { name: "pull_keep_both", checkpoint: "conflict", replica: "b", operation: "pull_apply", prepare: "pull_review" },
    ...[...timelineClouds].flatMap(([label, cloud]) => [
      { name: `timeline_${label.toLowerCase()}_first_page`, checkpoint: "accepted", replica: "b", operation: "timeline_first_page", timeline: cloud },
      { name: `timeline_${label.toLowerCase()}_all_pages`, checkpoint: "accepted", replica: "b", operation: "timeline", timeline: cloud },
    ]),
  ];
  for (const definition of definitions) {
    const groups = Object.fromEntries(variants.map((variant) => [variant.key, { name: definition.name, samples: [] }]));
    for (let index = -2; index < samples; index++) {
      const order = index % 2 === 0 ? variants : [...variants].reverse();
      for (const variant of order) {
        await checkpoints.restore(definition.checkpoint);
        const worker = new ServiceWorker(variant.binary, root, replicas[definition.replica], [], identity.environment.workers);
        try {
          await worker.open(definition.replica === "a" ? undefined : bundle);
          if (definition.warm) await prime(worker);
          const mutation = definition.append ? await checkpoints.installAppend(definition.append) : null;
          if (mutation) await new Promise((resolve) => setTimeout(resolve, 50));
          let args = {};
          if (definition.prepare) {
            const prepared = await worker.call(definition.prepare);
            if (definition.prepare === "push_review") {
              assert.ok(prepared.metrics.every((m) => m.selected_captures === 0), "Push review captured a payload");
              args = { resource_ids: prepared.value.thread_comparison.entries.map((row) => row.resource_id) };
            }
          }
          if (definition.timeline) {
            const rows = (await worker.compare()).entries;
            const row = rows.find((entry) => entry.cloud_thread_id === definition.timeline);
            assert.ok(row, "benchmark timeline identity missing");
            args = { resource_id: row.conversation_resource_id };
          }
          const before = definition.name.endsWith("noop") ? (await worker.call("inspect")).value : null;
          const reply = await worker.call(definition.operation, args);
          const measured = sample(reply, identity.environment.workers);
          if (before) {
            const after = (await worker.call("inspect")).value;
            assert.deepEqual(after.head, before.head, "benchmark no-op head changed");
            assert.deepEqual(after.manifest, before.manifest, "benchmark no-op manifest changed");
          }
          if (definition.operation === "push_review") assert.ok(reply.metrics.every((m) => m.selected_captures === 0), "Push review captured a payload");
          if (definition.operation === "push_apply") await auditStore(join(root, "storage"), Object.values(CANARIES));
          if (definition.name === "pull_full") {
            assert.equal((await worker.compare()).entries.filter((row) => row.local_present).length, identity.sessions.codex + identity.sessions.claude, "benchmark full Pull session count");
            if (index === 0) await validateRestore?.(worker, variant.key);
          }
          if (definition.name === "pull_keep_both") assert.equal(reply.value.result.preserved_threads.length, identity.conflict_count, "benchmark Keep both count");
          if (definition.timeline) {
            const events = reply.value.pages.flatMap((page) => page.events);
            assert.ok(events.length > 0, "benchmark timeline is empty");
            assert.equal(new Set(events.map((event) => event.event_id)).size, events.length, "benchmark timeline repeats events");
          }
          if (definition.warm && !definition.append) assert.ok(quiet(reply), "warm comparison repeated filesystem work");
          if (definition.append) {
            assert.equal(reply.value.entries.filter((row) => row.state === "local_ahead").length, 1, "append comparison lost the changed thread");
            assert.ok(reply.metrics.every((m) => m.selected_captures === 0), "append comparison captured a payload");
          }
          if (index >= 0) groups[variant.key].samples.push({ ...measured, ...(mutation ? { mutation } : {}) });
        } catch (error) {
          error.benchmarkPartial = reports;
          error.privateDetail = `${definition.name} ${variant.key} sample ${index}: ${error.privateDetail ?? "operation check"}`;
          throw error;
        } finally { await worker.close().catch(() => worker.kill()); }
      }
      onProgress?.(definition.name, index + 1, samples);
    }
    for (const variant of variants) {
      const group = groups[variant.key];
      group.median_ms = percentile(group.samples.map((s) => s.ms), 0.5);
      group.p95_ms = percentile(group.samples.map((s) => s.ms), 0.95);
      reports[variant.key].operations.push(group);
    }
  }
  for (const report of Object.values(reports)) report.correctness = "passed";
  return { ...reports, comparison: compareBenchmark(reports.baseline, reports.candidate) };
}
