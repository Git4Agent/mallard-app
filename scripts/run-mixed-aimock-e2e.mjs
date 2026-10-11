import assert from "node:assert/strict";
import { mkdtemp, realpath, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir, cpus, totalmem, platform, release, arch } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";
import { CANARIES, fixture, fixtureIdentity } from "./mixed-aimock/scenario.mjs";
import { directories, nativePreflight, NativeDriver, relocatedHistory, auditStore, verifyGraph, digestFiles } from "./mixed-aimock/native.mjs";
import { ServiceWorker } from "./mixed-aimock/service.mjs";
import { Checkpoints } from "./mixed-aimock/checkpoint.mjs";
import { exclusionControls } from "./mixed-aimock/controls.mjs";
import { incrementalCheckpoints } from "./mixed-aimock/incremental.mjs";
import { graphFaults } from "./mixed-aimock/faults.mjs";
import { sessionReconciliation } from "./mixed-aimock/reconciliation.mjs";
import { Ledger } from "./mixed-aimock/roundtrip.mjs";
import { buildWorker, harnessIdentity } from "./mixed-aimock/build.mjs";
import { confirmRegression } from "./compare-mixed-aimock-benchmark.mjs";
import { runBenchmark } from "./mixed-aimock/benchmark.mjs";
import { safeFailure, summary } from "./mixed-aimock/report.mjs";
import { stopOwned } from "./mixed-aimock/process.mjs";

import { options } from "./mixed-aimock/options.mjs";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");

let root; let stage = "preflight"; const workers = []; const report = { version: 1, status: "invalid", measurements: [] };
const started = performance.now();
let interrupted = null;
const interrupt = (reason) => { interrupted = reason; void stopOwned(); };
process.on("SIGINT", () => interrupt("signal_interrupt"));
process.on("SIGTERM", () => interrupt("signal_terminate"));
const watchdog = setTimeout(() => interrupt("job_watchdog"), 90 * 60 * 1000);
watchdog.unref();
try {
  if (process.env.MALLARD_RUN_MIXED_AIMOCK_E2E !== "1") throw new Error("set_MALLARD_RUN_MIXED_AIMOCK_E2E_1");
  const opts = options(process.argv.slice(2));
  report.profile = opts.profile;
  assert.ok(Number(process.versions.node.split(".")[0]) > 20 || (Number(process.versions.node.split(".")[0]) === 20 && Number(process.versions.node.split(".")[1]) >= 15), "Node 20.15 or later required");
  const aimock = JSON.parse(await readFile(join(repository, "node_modules/@copilotkit/aimock/package.json"), "utf8"));
  assert.equal(aimock.version, "1.39.0", "aimock version pin");
  root = await realpath(await mkdtemp(join(tmpdir(), "mallard-mixed-")));
  const { binary, baselineBinary } = await buildWorker(repository, root, opts.debug, opts.baselineRef);
  const harness = await harnessIdentity(repository);
  const replicas = await directories(root);
  const versions = await nativePreflight(replicas.a);
  const corpus = fixture(opts.profile, opts.scale);
  const { nodes } = corpus;
  Object.assign(report, { fixture: fixtureIdentity(corpus), profile: corpus.profile, versions: { ...versions, aimock: aimock.version, node: process.versions.node, rust: harness.rust },
    environment: { platform: platform(), os: release(), arch: arch(), cpu: cpus()[0].model, memory_bytes: totalmem(), build: opts.debug ? "debug" : "release", storage: "local", workers: opts.workers, network_isolated: process.env.MALLARD_MIXED_NETWORK_ISOLATED === "1" },
    sessions: corpus.sessions, graph_edges: nodes.filter((node) => node.parent).length });
  const native = new NativeDriver(repository, root, replicas);
  const worker = (replica) => { const value = new ServiceWorker(binary, root, replica, report.measurements, opts.workers); workers.push(value); return value; };
  const active = { a: worker(replicas.a) }; const bundle = await active.a.open();
  const checkpoints = new Checkpoints(root, replicas);
  const save = async (name) => {
    const names = Object.keys(active);
    for (const name of names) await active[name].close();
    await checkpoints.save(name);
    for (const name of names) { active[name] = worker(replicas[name]); await active[name].open(name === "a" ? undefined : bundle); }
  };
  let a = active.a;
  const publishedClouds = new Map();
  stage = "native_generation"; console.log(`${corpus.profile} fixture: ${nodes.length} native sessions.`);
  let lastProgress = performance.now();
  await native.generate(nodes, (label, count) => {
    if (performance.now() - lastProgress > 15_000) { console.log(`Native generation: ${count}/${nodes.length} sessions (${label}).`); lastProgress = performance.now(); }
  });
  report.source_turns = native.sourceTurns;
  stage = "native_graph"; report.graph = await verifyGraph(native, nodes, a);
  stage = "exclusion_controls"; report.controls = await exclusionControls(native, a);
  await save("generated"); a = active.a;
  if (opts.benchmark) {
    await a.push();
    const published = (await a.compare()).entries;
    for (const node of nodes) {
      const row = published.find((entry) => entry.provider === node.provider && entry.provider_thread_id === native.sessionIds.get(node.label));
      assert.ok(row?.cloud_thread_id, "published checkpoint cloud mapping");
      publishedClouds.set(node.label, row.cloud_thread_id);
    }
    await save("published");
    await active.a.close(); await checkpoints.restore("generated");
    active.a = worker(replicas.a); await active.a.open(); a = active.a;
  }
  stage = "source_compare";
  const source = await a.compare(); assert.equal(source.entries.length, nodes.length, "source session count");
  const sourceDigests = await Promise.all([digestFiles(join(replicas.a.codex, "sessions")), digestFiles(join(replicas.a.claude, "projects"))]);
  const selectedLabels = corpus.selection;
  stage = "selected_push"; await a.push(new Set(selectedLabels.map((id) => native.sessionIds.get(id))));
  active.b = worker(replicas.b); let b = active.b; await b.open(bundle);
  stage = "selected_pull"; await b.pull();
  let sourceRows = (await a.compare()).entries;
  let destinationRows = (await b.compare()).entries;
  const clouds = new Map();
  for (const node of nodes) {
    const row = sourceRows.find((row) => row.provider_thread_id === native.sessionIds.get(node.label) && row.provider === node.provider);
    if (row?.cloud_thread_id) clouds.set(node.label, row.cloud_thread_id);
  }
  assert.equal(destinationRows.filter((row) => row.local_present).length, selectedLabels.length, "selected Pull count");
  stage = "selected_resume";
  for (const label of selectedLabels) {
    const node = nodes.find((n) => n.label === label);
    const row = destinationRows.find((row) => row.cloud_thread_id === clouds.get(label));
    await native.reopen(replicas.b, node, row.provider_thread_id,
      relocatedHistory(native.histories.get(label), replicas.a.project, replicas.b.project), "SELECTED");
  }
  stage = "full_push"; await a.push();
  assert.deepEqual(await Promise.all([digestFiles(join(replicas.a.codex, "sessions")), digestFiles(join(replicas.a.claude, "projects"))]), sourceDigests, "Push changed a source transcript");
  stage = "full_pull"; await b.pull();
  sourceRows = (await a.compare()).entries; destinationRows = (await b.compare()).entries;
  assert.equal(destinationRows.length, nodes.length, "full Pull count");
  for (const node of nodes) {
    const sourceRow = sourceRows.find((row) => row.provider_thread_id === native.sessionIds.get(node.label) && row.provider === node.provider);
    assert.ok(sourceRow?.cloud_thread_id, "source cloud binding"); clouds.set(node.label, sourceRow.cloud_thread_id);
  }
  stage = "full_resume"; console.log("Verify every restored native history.");
  for (const node of nodes) {
    const row = destinationRows.find((row) => row.cloud_thread_id === clouds.get(node.label));
    assert.ok(row?.provider_thread_id, "destination native binding");
    await native.reopen(replicas.b, node, row.provider_thread_id,
      relocatedHistory(native.histories.get(node.label), replicas.a.project, replicas.b.project), "FULL");
    const timeline = (await b.call("timeline", { resource_id: row.conversation_resource_id })).value;
    const events = timeline.pages.flatMap((page) => page.events);
    assert.equal(new Set(events.map((event) => event.event_id)).size, events.length, "timeline duplicate event");
    const markers = events.filter((event) => ["prompt", "response"].includes(event.kind) && event.text?.startsWith("MIXED_"))
      .map((event) => event.text.split("\n", 1)[0]);
    try { assert.deepEqual(markers, native.histories.get(node.label).map((entry) => entry.marker.split("\n", 1)[0]), "timeline message order"); }
    catch { const error = new Error("timeline_message_order"); error.privateDetail = JSON.stringify({ label: node.label, kinds: events.map((event) => event.kind), markers: markers.filter((m) => /^MIXED_[A-Z0-9_]+$/u.test(m)) }); throw error; }
  }
  stage = "no_op";
  const before = (await a.call("inspect")).value;
  await a.push(); await b.pull();
  const after = (await a.call("inspect")).value;
  assert.deepEqual(after.head, before.head, "no-op changed the bundle head");
  assert.deepEqual(after.manifest, before.manifest, "no-op changed the manifest");
  active.c = worker(replicas.c); await active.c.open(bundle);
  await save("accepted"); a = active.a; b = active.b;
  const ledger = new Ledger(native, nodes, replicas, active, clouds);
  stage = "conflicts"; console.log("Check divergent histories and Keep both in both directions.");
  await ledger.conflicts(corpus.conflicts, opts.benchmark ? save : undefined);
  stage = "path_roundtrip"; console.log("Check three project paths and all final native histories.");
  report.final_sessions = await ledger.roundtrip(corpus.roundtrip);
  stage = "expired_plan";
  await active.b.call("pull_review");
  const expired = await active.b.call("pull_apply", { expire: true }, true);
  assert.equal(expired.ok, false, "expired approval accepted");
  stage = "portable_privacy"; report.storage = await auditStore(join(root, "storage"), Object.values(CANARIES));
  report.native = native.times; report.mock_audit = native.audits;
  for (const value of Object.values(active)) await value.close();
  stage = "graph_faults";
  report.faults = await graphFaults({ binary, root, replicas, checkpoints, native, fault: corpus.fault, workers: opts.workers });
  stage = "session_reconciliation";
  console.log("Check immediate session reconciliation, open writers, and process restarts.");
  report.reconciliation = await sessionReconciliation({ binary, root, replicas, checkpoints, native, nodes, workers: opts.workers });
  assert.equal(report.reconciliation.status, "passed", "session_reconciliation_failed");
  report.status = "passed";
  if (opts.benchmark) {
    stage = "benchmark_preparation";
    await incrementalCheckpoints({ binary, root, replicas, checkpoints, native, nodes, clouds, bundle, workers: opts.workers, incremental: corpus.incremental });
    stage = "benchmark";
    const identity = { ...harness, profile: corpus.profile, sessions: corpus.sessions, conflict_count: corpus.conflicts[0].length, fixture: report.fixture, environment: report.environment, versions: report.versions };
    const detailLabels = corpus.detail;
    const benchmarkOptions = { timelineClouds: new Map(detailLabels.map((label) => [label, clouds.get(label)])),
      validateRestore: async (worker, variant) => {
        console.log(`Verify the ${variant} benchmark restore with native clients.`);
        const rows = (await worker.compare()).entries;
        for (const node of nodes) {
          const row = rows.find((entry) => entry.cloud_thread_id === publishedClouds.get(node.label));
          await native.reopen(replicas.b, node, row.provider_thread_id,
            relocatedHistory(native.histories.get(node.label), replicas.a.project, replicas.b.project), `BENCH_${variant.toUpperCase()}`);
        }
      }, binary, baselineBinary, baselineIdentity: { ...identity, product_commit: opts.baselineRef, dirty: false, source_change_digest: null }, root, replicas, checkpoints, bundle, samples: opts.samples, identity,
      onProgress: (name, count, samples) => { if (count === samples) console.log(`Benchmark ${name}: ${samples} samples.`); } };
    report.benchmark = await runBenchmark(benchmarkOptions);
    if (report.benchmark.comparison.status === "regression") {
      console.log("Repeat the full paired benchmark to check the regression.");
      report.benchmark_confirmation = await runBenchmark(benchmarkOptions);
      report.benchmark.comparison = confirmRegression(report.benchmark.comparison, report.benchmark_confirmation.comparison);
    }
    if (report.benchmark.comparison.status !== "passed") { report.status = report.benchmark.comparison.status; process.exitCode = 2; }
  }
  console.log(`Mixed correctness passed: ${nodes.length} sessions; ${native.sourceTurns} source turns.`);
} catch (error) {
  report.status = stage === "preflight" ? "invalid" : "failed";
  report.failure = safeFailure(error, stage);
  if (error.benchmarkPartial) report.benchmark = { ...error.benchmarkPartial, comparison: { status: "invalid" } };
  console.error(JSON.stringify(report.failure));
  console.error(`Mixed test failed: ${stage} (${report.failure.code}).`);
  process.exitCode = interrupted ? 130 : 1;
} finally {
  clearTimeout(watchdog);
  for (const worker of workers) worker.kill(); await stopOwned();
  report.elapsed_ms = performance.now() - started;
  if (root) {
    try {
    const encoded = JSON.stringify(report, null, 2);
    assert.ok(!encoded.includes(root) && Object.values(CANARIES).every((secret) => !encoded.includes(secret)), "report privacy");
    const destination = join(repository, "output", report.profile === "xlarge" ? "mixed-aimock-xlarge" : "mixed-aimock", new Date().toISOString().replaceAll(":", "-"));
    await mkdir(destination, { recursive: true });
    await writeFile(join(destination, "results.json"), `${encoded}\n`);
    await writeFile(join(destination, "mock-audit.json"), `${JSON.stringify(report.mock_audit ?? [], null, 2)}\n`);
    if (report.benchmark) {
      await writeFile(join(destination, "candidate.json"), `${JSON.stringify(report.benchmark.candidate, null, 2)}\n`);
      if (report.benchmark.baseline) await writeFile(join(destination, "baseline.json"), `${JSON.stringify(report.benchmark.baseline, null, 2)}\n`);
    }
    await writeFile(join(destination, "summary.md"), summary(report));
    } finally {
      // Temporary data is removed even if report validation or writing fails.
      await rm(root, { recursive: true, force: true, maxRetries: 10 });
    }
  }
}
