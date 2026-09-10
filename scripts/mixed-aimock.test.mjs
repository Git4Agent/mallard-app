import assert from "node:assert/strict";
import test from "node:test";
import { fixture, fixtureIdentity, closure, body, scenarioDocument, textScenario, transported, CANARIES, MODELS } from "./mixed-aimock/scenario.mjs";
import { options } from "./mixed-aimock/options.mjs";
import * as codex from "./live-codex-aimock-server.mjs";
import * as claude from "./live-claude-aimock-server.mjs";
import { environment } from "./mixed-aimock/process.mjs";

const source = "/fixture/source/project";
const destination = "/fixture/destination/project";

test("xlarge preserves the original counts, tiers, depth, and selected dependency closure", () => {
  const corpus = fixture("xlarge");
  const { nodes } = corpus;
  assert.equal(nodes.length, 120);
  assert.equal(nodes.filter((node) => node.provider === "codex").length, 100);
  assert.equal(nodes.filter((node) => node.parent).length, 80);
  assert.equal(nodes.reduce((n, node) => n + node.turns, 0), 1048);
  assert.equal(nodes.reduce((n, node) => n + node.turns * node.body_bytes, 0), 6424 * 1024);
  assert.equal(closure(nodes, corpus.selection.filter((id) => id.startsWith("C"))).length, 31);
  assert.equal(closure(nodes, ["C032"]).length, 13);
  assert.equal(new Set(nodes.map((node) => node.label)).size, 120);
  for (const scale of [10, 50, 100]) {
    const small = fixture("xlarge", scale).nodes;
    assert.equal(closure(small, small.map((node) => node.label)).length, scale * 1.2);
  }
  assert.throws(() => closure([{ label: "A", parent: "B" }, { label: "B", parent: "A" }], ["A"]));
});

test("default corpus has 25 Codex and 15 Claude sessions with early, late, deep, and nested forks", () => {
  const corpus = fixture();
  const { nodes } = corpus;
  assert.deepEqual(corpus.sessions, { codex: 25, claude: 15 });
  assert.equal(nodes.length, 40);
  assert.equal(nodes.filter((node) => node.provider === "codex").length, 25);
  assert.equal(nodes.filter((node) => node.provider === "claude").length, 15);
  assert.equal(nodes.filter((node) => node.parent).length, 20);
  assert.equal(nodes.reduce((count, node) => count + node.turns, 0), 316);
  assert.equal(nodes.reduce((count, node) => count + node.turns * node.body_bytes, 0), 1744 * 1024);
  assert.equal(Math.max(...nodes.map((node) => closure(nodes, [node.label]).length - 1)), 8);
  assert.deepEqual(nodes.filter((node) => node.early).map((node) => node.label), ["C014", "C015"]);
  assert.equal(nodes.filter((node) => node.parent === "C002" && !node.early).length, 5);
  assert.deepEqual(closure(nodes, ["C025"]), ["C003", "C021", "C022", "C024", "C025"]);
  assert.equal(nodes.filter((node) => node.provider === "claude" && node.tool).length, 4);
  assert.deepEqual(new Set(nodes.filter((node) => node.provider === "claude").map((node) => node.turns)), new Set([4, 12, 24]));
  assert.equal(closure(nodes, corpus.selection.filter((id) => id.startsWith("C"))).length, 19);
  assert.equal(nodes.length + corpus.conflicts.flat().length, 48);
});

test("each profile supplies complete selections, conflicts, benchmark targets, and independent graph controls", () => {
  for (const corpus of [fixture(), ...[10, 50, 100].map((scale) => fixture("xlarge", scale))]) {
    const { nodes } = corpus;
    const labels = new Set(nodes.map((node) => node.label));
    assert.equal(labels.size, nodes.length);
    assert.equal(closure(nodes, [...labels]).length, nodes.length);
    const targets = [...corpus.selection, ...corpus.conflicts.flat(), ...corpus.roundtrip, ...corpus.detail, ...Object.values(corpus.incremental).flat()];
    assert.ok(targets.every((target) => labels.has(target)));
    assert.equal(new Set(corpus.incremental.ten).size, 10);
    assert.equal(new Set(corpus.detail).size, 3);
    assert.equal(new Set(corpus.conflicts.flat()).size, corpus.conflicts.flat().length);
    for (const group of corpus.conflicts) assert.deepEqual(new Set(group.map((id) => nodes.find((node) => node.label === id).provider)), new Set(["codex", "claude"]));
    if (corpus.fault) {
      assert.ok(nodes.find((node) => node.label === corpus.fault.child).parent);
      assert.equal(nodes.find((node) => node.label === corpus.fault.independent).parent, null);
      assert.ok(!closure(nodes, [corpus.fault.child]).includes(corpus.fault.independent));
    }
  }
  assert.notEqual(fixtureIdentity(fixture()), fixtureIdentity(fixture("xlarge")));
  const changed = fixture(); changed.conflicts[0].pop();
  assert.notEqual(fixtureIdentity(fixture()), fixtureIdentity(changed));
});

test("runner defaults to mixed and requires an explicit xlarge profile for scale overrides", () => {
  assert.deepEqual(fixture(options([]).profile).sessions, { codex: 25, claude: 15 });
  assert.deepEqual(fixture(options(["--profile=xlarge"]).profile).sessions, { codex: 100, claude: 20 });
  assert.equal(options(["--profile=xlarge", "--scale=50"]).scale, 50);
  for (const args of [["--scale=100"], ["--profile=small"], ["--profile=xlarge", "--scale=25"], ["--profile=xlarge", "--profile=mixed"], ["--benchmark", "--debug"], ["--samples=19"]]) assert.throws(() => options(args));
  assert.throws(() => fixture("mixed", 100));
  assert.throws(() => fixture("xlarge", 25));
});

test("body sizing and independent path/redaction expectations preserve literal destination", () => {
  const text = body("MIXED_RESPONSE_1", 8192, source, destination);
  assert.equal(Buffer.byteLength(text), 8192);
  const local = transported(text, source, destination);
  assert.ok(local.includes(`Project file: ${destination}/nested/report.txt`));
  assert.ok(local.includes(`Literal destination: ${destination}/already-local.txt`));
  assert.ok(Object.values(CANARIES).every((secret) => !local.includes(secret)));
  assert.ok(local.includes("<anthropic-api-key>") && local.includes("<password>"));
  const returned = transported(local, destination, source);
  assert.ok(returned.includes(`Literal destination: ${destination}/already-local.txt`));
  assert.ok(returned.includes(`Project file: ${source}/nested/report.txt`));
});

for (const [provider, api] of [["codex", codex], ["claude", claude]]) {
  test(`${provider} strict mock checks the complete large response body`, () => {
    const text = body("MIXED_RESPONSE_PREVIOUS", 8192, source, destination);
    const history = [{ role: "user", marker: "MIXED_USER_PREVIOUS" }, { role: "assistant", marker: text }];
    const fixture = textScenario(provider, "body_check", "MIXED_USER_CURRENT", history, "MIXED_RESPONSE_CURRENT");
    const doc = api.parseScenarioDocument(JSON.stringify(scenarioDocument(provider, [fixture], 1)));
    const request = { model: MODELS[provider], stream: true,
      messages: history.map((entry) => ({ role: entry.role, content: entry.marker })).concat({ role: "user", content: fixture.prompt_marker }),
      tools: [{ type: "function", function: { name: "Read" } }],
    };
    assert.equal(api.validateScenarioRequest(doc, doc.scenarios[0], request).history_match, true);
    request.messages[1].content = `${text.slice(0, -1)}?`;
    assert.equal(api.validateScenarioRequest(doc, doc.scenarios[0], request).history_match, false);
    fixture.response.content = `MIXED_RESPONSE_BIG\n${"x".repeat(32768)}`;
    assert.throws(() => api.parseScenarioDocument(JSON.stringify(scenarioDocument(provider, [fixture], 1))));
  });
}

test("Codex fixture reads cannot name another directory or arbitrary file", () => {
  const fixture = textScenario("codex", "read_check", "MIXED_USER_READ", [], "MIXED_RESPONSE_READ");
  fixture.response = { kind: "tool_call", name: "exec_command", call_id: "mixed_read", arguments: { cmd: `cat -- '${source}/mixed-read.txt'`, workdir: source } };
  const final = textScenario("codex", "read_final", fixture.prompt_marker, [], "MIXED_RESPONSE_READ");
  final.require_tool_result = true; final.expected_tool_call_id = "mixed_read";
  const parse = () => codex.parseScenarioDocument(JSON.stringify(scenarioDocument("codex", [fixture, final])));
  assert.doesNotThrow(parse);
  fixture.response.arguments.workdir = destination;
  assert.throws(parse);
  fixture.response.arguments.cmd = "cat -- '/etc/passwd'";
  assert.throws(parse);
});

test("native child environment is closed to inherited provider and injection settings", () => {
  const child = environment(source);
  for (const key of ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "HTTP_PROXY", "NODE_OPTIONS", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "AWS_ACCESS_KEY_ID"]) assert.equal(Object.hasOwn(child, key), false);
  assert.equal(child.HOME, source); assert.equal(child.NO_PROXY, "127.0.0.1");
});

import { compareBenchmark, percentile } from "./compare-mixed-aimock-benchmark.mjs";
const benchmark = (ms = 100) => ({ version: 1, correctness: "passed", fixture: "fixed", harness_digest: "same",
  environment: { build: "release", cpu: "fixed" }, versions: { codex: "pinned" },
  operations: [{ name: "warm_compare", samples: Array.from({ length: 20 }, () => ({ ms, peak_rss_bytes: 64 * 1024 * 1024 })) }],
});
test("benchmark percentiles, injected slowdown, and baseline mismatch cannot pass silently", () => {
  assert.equal(percentile(Array.from({ length: 20 }, (_, i) => i + 1), 0.5), 10.5);
  assert.equal(percentile(Array.from({ length: 20 }, (_, i) => i + 1), 0.95), 19);
  assert.equal(compareBenchmark(null, benchmark()).status, "uncalibrated");
  assert.equal(compareBenchmark(benchmark(), benchmark(119)).status, "passed");
  assert.equal(compareBenchmark(benchmark(), benchmark(150)).status, "regression");
  const mismatch = benchmark(); mismatch.versions.codex = "another";
  assert.equal(compareBenchmark(benchmark(), mismatch).status, "invalid");
  const mixed = benchmark(); mixed.fixture = fixtureIdentity(fixture());
  const xlarge = benchmark(); xlarge.fixture = fixtureIdentity(fixture("xlarge"));
  assert.equal(compareBenchmark(xlarge, mixed).code, "baseline_mismatch_fixture");
  const short = benchmark(); short.operations[0].samples.pop();
  assert.equal(compareBenchmark(benchmark(), short).status, "invalid");
  const nan = benchmark(); nan.operations[0].samples[0].ms = NaN;
  assert.equal(compareBenchmark(benchmark(), nan).status, "invalid");
  const missing = benchmark(); missing.operations = [];
  assert.equal(compareBenchmark(benchmark(), missing).status, "invalid");
});

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { startMock } from "./mixed-aimock/process.mjs";
for (const provider of ["codex", "claude"]) {
  test(`${provider} shared mock lifecycle checks readiness and final request audit`, async () => {
    const root = await mkdtemp(join(tmpdir(), "mallard-mixed-unit-"));
    let mock;
    try {
      const fixture = textScenario(provider, "one", "MIXED_USER_ONE", [], "MIXED_RESPONSE_ONE");
      mock = await startMock(resolve("."), join(root, "control"), provider, scenarioDocument(provider, [fixture], 0));
      const request = provider === "codex" ? { model: MODELS.codex, stream: true, input: [{ role: "user", content: fixture.prompt_marker }] }
        : { model: MODELS.claude, stream: true, max_tokens: 128, messages: [{ role: "user", content: fixture.prompt_marker }], tools: [{ name: "Read", description: "Read a fixture", input_schema: { type: "object", properties: { file_path: { type: "string" } } } }] };
      const response = await fetch(`${mock.url}/${provider === "codex" ? "responses" : "messages?beta=true"}`, {
        method: "POST", headers: { "content-type": "application/json", ...(provider === "claude" ? { "x-api-key": "mallard-local-aimock-placeholder", "anthropic-version": "2023-06-01" } : {}) }, body: JSON.stringify(request),
      });
      assert.equal(response.status, 200); await response.text();
      const audit = await mock.finish(); assert.equal(audit.requests, 1);
    } finally { mock?.kill(); await rm(root, { recursive: true, force: true }); }
  });
}

import { confirmRegression } from "./compare-mixed-aimock-benchmark.mjs";
test("a timing breach needs confirmation and an unstable result stays non-green", () => {
  const failure = compareBenchmark(benchmark(), benchmark(150));
  assert.equal(confirmRegression(failure, failure).status, "regression");
  assert.equal(confirmRegression(failure, compareBenchmark(benchmark(), benchmark())).status, "unstable");
  const invalidRss = benchmark(); invalidRss.operations[0].samples[0].peak_rss_bytes = NaN;
  assert.equal(compareBenchmark(null, invalidRss).status, "invalid");
  const debug = benchmark(); debug.environment.build = "debug";
  assert.equal(compareBenchmark(null, debug).status, "invalid");
});

for (const provider of ["codex", "claude"]) {
  test(`${provider} routing does not confuse turn 1 with turn 10`, async () => {
    const root = await mkdtemp(join(tmpdir(), "mallard-mixed-routing-"));
    let mock;
    try {
      const fixtures = [1, 10].map((turn) => textScenario(provider, `turn_${turn}`, `MIXED_USER_TURN_${turn}`, [], `MIXED_RESPONSE_TURN_${turn}`));
      mock = await startMock(resolve("."), join(root, "control"), provider, scenarioDocument(provider, fixtures, 0));
      // Request turn 10 first. A substring router incorrectly consumes turn 1.
      for (const fixture of [...fixtures].reverse()) {
        const request = provider === "codex" ? { model: MODELS.codex, stream: true, input: [{ role: "user", content: fixture.prompt_marker }] }
          : { model: MODELS.claude, stream: true, max_tokens: 128, messages: [{ role: "user", content: fixture.prompt_marker }], tools: [{ name: "Read", description: "Read fixture", input_schema: { type: "object", properties: {} } }] };
        const response = await fetch(`${mock.url}/${provider === "codex" ? "responses" : "messages?beta=true"}`, {
          method: "POST", headers: { "content-type": "application/json", ...(provider === "claude" ? { "x-api-key": "mallard-local-aimock-placeholder", "anthropic-version": "2023-06-01" } : {}) }, body: JSON.stringify(request),
        });
        assert.equal(response.status, 200); await response.text();
      }
      assert.equal((await mock.finish()).requests, 2);
    } finally { mock?.kill(); await rm(root, { recursive: true, force: true }); }
  });
}

import { safeFailure, summary } from "./mixed-aimock/report.mjs";
test("failure exports omit paths, credentials, native IDs, and transcript bodies", () => {
  const error = new Error("native_process_failure");
  error.privateDetail = `C001 at /Users/private-user/project: ${CANARIES.key} ${CANARIES.password} private transcript body; code=6,index=2,observed=1,total=4`;
  const failure = safeFailure(error, "native_generation");
  const encoded = JSON.stringify(failure);
  for (const forbidden of ["/Users", "private-user", "private transcript body", ...Object.values(CANARIES)]) assert.ok(!encoded.includes(forbidden));
  assert.deepEqual(failure.fixture_labels, ["C001"]);
  assert.equal(failure.mock_mismatch, "code=6,index=2,observed=1,total=4");
  assert.ok(summary({ status: "failed", failure, elapsed_ms: 100 }).includes("native_generation"));
});

import { runBenchmark } from "./mixed-aimock/benchmark.mjs";
test("incomplete benchmark runs and missing validation cannot pass", async () => {
  const incomplete = benchmark(); incomplete.correctness = "pending";
  assert.equal(compareBenchmark(benchmark(), incomplete).status, "invalid");
  await assert.rejects(runBenchmark({}), /requires three history targets/u);
  const targets = new Map([["C032", "one"], ["C099", "two"], ["L020", "three"]]);
  await assert.rejects(runBenchmark({ timelineClouds: targets }), /requires native restore validation/u);
  await assert.rejects(runBenchmark({ timelineClouds: targets, validateRestore() {}, samples: 19 }), /sample count/u);
  assert.equal(environment(source).CLAUDE_CODE_MAX_RETRIES, "0");
});
