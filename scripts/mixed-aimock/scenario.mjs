import assert from "node:assert/strict";
import { createHash } from "node:crypto";

export const MODELS = { codex: "gpt-5.6-luna", claude: "claude-sonnet-4-6" };
export const CANARIES = Object.freeze({
  key: "sk-ant-api03-fakemixedcorpusaaaabbbb",
  password: "hunter2fakelive",
});
export const sha256 = (value) => createHash("sha256").update(value).digest("hex");
export const label = (number) => `C${String(number).padStart(3, "0")}`;
const XLARGE_SELECTION = [32, 33, 45, 60, 67, 92, 99, 100].map(label).concat("L011", "L017");
const XLARGE_CONFLICTS = [
  ["C007", "C033", "C060", "C099", "L011", "L017"],
  ["C008", "C045", "C067", "C100", "L012", "L018"],
];

function xlargeSpecification(scale) {
  assert.ok([10, 50, 100].includes(scale), "invalid scale");
  const parents = new Map();
  for (let n = 21; n <= 32; n++) parents.set(n, n === 21 ? 1 : n - 1);
  for (let n = 33; n <= 56; n++) parents.set(n, 2);
  for (let root = 3; root <= 6; root++) {
    const first = 57 + (root - 3) * 7;
    parents.set(first, root);
    parents.set(first + 1, first);
    parents.set(first + 2, first);
    for (let offset = 3; offset <= 6; offset++) parents.set(first + offset, first + (offset < 5 ? 1 : 2));
  }
  [60, 61, 62, 63, 67, 68, 69, 70].forEach((parent, i) => parents.set(85 + i, parent));
  for (let n = 93; n <= 100; n++) parents.set(n, n - 8);
  const short = (n) => n <= 6 || (n >= 21 && n <= 32) || (n >= 57 && n <= 98);
  const long = (n) => (n >= 49 && n <= 56) || n >= 99;
  const nodes = Array.from({ length: scale }, (_, i) => {
    const n = i + 1;
    return {
      label: label(n), provider: "codex", parent: parents.has(n) ? label(parents.get(n)) : null,
      turns: short(n) ? 4 : long(n) ? 24 : 12, body_bytes: short(n) ? 1024 : 8192,
      early: n >= 33 && n <= 44,
    };
  });
  for (let i = 1; i <= scale / 5; i++) nodes.push({
    label: `L${String(i).padStart(3, "0")}`, provider: "claude", parent: null,
    turns: i <= 10 ? 4 : i <= 16 ? 12 : 24, body_bytes: i <= 10 ? 1024 : 8192,
    tool: i >= 11 && i <= 16,
  });
  const labels = new Set(nodes.map((node) => node.label));
  for (const node of nodes) assert.ok(!node.parent || labels.has(node.parent), "subset has a missing parent");
  return nodes;
}

function mixedSpecification() {
  const parents = new Map();
  for (let n = 6; n <= 13; n++) parents.set(n, n === 6 ? 1 : n - 1);
  for (let n = 14; n <= 20; n++) parents.set(n, 2);
  for (const [child, parent] of [[21, 3], [22, 21], [23, 21], [24, 22], [25, 24]]) parents.set(child, parent);
  const nodes = Array.from({ length: 25 }, (_, i) => {
    const n = i + 1;
    const turns = [20, 25].includes(n) ? 24 : [13, 18, 19].includes(n) ? 12 : 4;
    return {
      label: label(n), provider: "codex", parent: parents.has(n) ? label(parents.get(n)) : null,
      turns, body_bytes: turns === 4 ? 1024 : 8192, early: n === 14 || n === 15,
    };
  });
  for (let n = 1; n <= 15; n++) nodes.push({
    label: `L${String(n).padStart(3, "0")}`, provider: "claude", parent: null,
    turns: n <= 8 ? 4 : n <= 12 ? 12 : 24, body_bytes: n <= 8 ? 1024 : 8192,
    tool: n >= 9 && n <= 12,
  });
  return nodes;
}

// Keep all profile-dependent labels and counts together. Both profiles run the
// same native, restore, fault, and performance checks.
export function fixture(profile = "mixed", scale) {
  assert.ok(["mixed", "xlarge"].includes(profile), "invalid fixture profile");
  assert.ok(profile === "xlarge" || scale === undefined, "scale requires the xlarge profile");
  const xlarge = profile === "xlarge";
  const size = xlarge ? scale ?? 100 : 25;
  const nodes = xlarge ? xlargeSpecification(size) : mixedSpecification();
  const full = xlarge && size === 100;
  const selection = !xlarge ? [13, 14, 16, 20, 22, 23, 24, 25].map(label).concat("L009", "L015")
    : full ? XLARGE_SELECTION : ["C001", "C002", "L001"];
  return {
    profile, version: xlarge ? "mixed-xlarge-v1" : "mixed-25-15-v1", nodes,
    sessions: { codex: size, claude: xlarge ? size / 5 : 15 },
    selection,
    conflicts: !xlarge ? [["C006", "C020", "L009", "L013"], ["C007", "C025", "L010", "L014"]]
      : full ? XLARGE_CONFLICTS : [["C001", "L001"], ["C002", "L002"]],
    roundtrip: !xlarge ? ["C025", "L015"] : full ? ["C099", "L020"] : ["C010", "L002"],
    incremental: {
      one: [!xlarge ? "C025" : full ? "C100" : "C010"],
      claude: [!xlarge ? "L015" : full ? "L020" : "L002"],
      ten: !xlarge || full ? selection : [1, 2, 3, 4, 5, 6, 7, 8].map(label).concat("L001", "L002"),
    },
    detail: !xlarge ? ["C013", "C025", "L015"] : full ? ["C032", "C099", "L020"] : ["C001", "C010", "L002"],
    fault: !xlarge ? { child: "C013", independent: "C005" }
      : size >= 50 ? { child: "C032", independent: "C020" } : null,
  };
}

export function closure(nodes, selected) {
  const byLabel = new Map(nodes.map((node) => [node.label, node]));
  const result = new Set();
  const visit = (id, active = new Set()) => {
    assert.ok(!active.has(id), "fixture graph cycle");
    if (result.has(id)) return;
    const node = byLabel.get(id);
    assert.ok(node, "unknown fixture node");
    const next = new Set(active).add(id);
    if (node.parent) visit(node.parent, next);
    result.add(id);
  };
  selected.forEach((id) => visit(id));
  return [...result];
}

export function body(marker, bytes, projectRoot, literalDestination) {
  const header = `${marker}\n` +
    `Project file: ${projectRoot}/nested/report.txt\n` +
    `URI: file://${projectRoot}/nested/report.txt\n` +
    `Literal destination: ${literalDestination}/already-local.txt\n` +
    `Literal marker: /__mallard_root__/b; and /__mallard_escape__/r\n` +
    `TOKEN=${CANARIES.key}\nDB_PASSWORD=${CANARIES.password}\n` +
    'JSON: {"status":"complete","steps":["read","check","save"]}\n';
  const paragraph = "The fixture contains an exact response. Read the file. Check the result. Keep each message in order.\n";
  const left = bytes - Buffer.byteLength(header);
  assert.ok(left >= 0, "fixture header exceeds its body tier");
  return header + paragraph.repeat(Math.ceil(left / paragraph.length)).slice(0, left);
}

// Independent expected edits. This does not call a Mallard codec.
export function transported(text, sourceRoot, destinationRoot) {
  return text.replaceAll(`Project file: ${sourceRoot}`, `Project file: ${destinationRoot}`)
    .replaceAll(`URI: file://${sourceRoot}`, `URI: file://${destinationRoot}`)
    .replaceAll(CANARIES.key, "<anthropic-api-key>")
    .replaceAll(CANARIES.password, "<password>");
}

export function textScenario(provider, id, prompt, history, response) {
  return {
    id, prompt_marker: prompt, expected_history: history.map((entry) => ({ ...entry })),
    require_tool_result: false, expected_tool_call_id: null,
    ...(provider === "claude" ? { expected_tool_result_marker: null } : {}),
    response: { kind: "text", content: response },
  };
}

export function scenarioDocument(provider, scenarios, probes) {
  assert.ok(scenarios.length > 0 && scenarios.length <= 64, "mock request batch limit");
  const document = {
    version: 1, model: MODELS[provider], expected_request_count: scenarios.length,
    ...(provider === "claude" ? { expected_probe_request_count: probes } : {}), scenarios,
  };
  assert.ok(Buffer.byteLength(JSON.stringify(document)) <= 4 * 1024 * 1024, "mock document byte limit");
  return document;
}

export function fixtureIdentity(specification) {
  return sha256(JSON.stringify({ specification, models: MODELS, canaries: CANARIES, body: sha256(body("MIXED_BODY_SAMPLE", 8192, "/fixture/source", "/fixture/destination")) }));
}
