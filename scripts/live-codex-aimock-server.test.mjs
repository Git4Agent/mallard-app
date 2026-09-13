import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { once } from "node:events";
import test from "node:test";

import {
  extractLeadingMarker,
  parseArguments,
  parseScenarioDocument,
  validateScenarioRequest,
  waitForShutdown,
} from "./live-codex-aimock-server.mjs";

const model = "gpt-5.6-luna";
const safePatchCommand =
  "apply_patch <<'MALLARD_PATCH'\n" +
  "*** Begin Patch\n" +
  "*** Add File: ./generated/one.txt\n" +
  "+ONE\n" +
  "*** Add File: ./generated/two.txt\n" +
  "+TWO\n" +
  "*** Add File: /private/tmp/mallard_external/three.txt\n" +
  "+THREE\n" +
  "*** Add File: /private/tmp/mallard_external/four.txt\n" +
  "+FOUR\n" +
  "*** End Patch\n" +
  "MALLARD_PATCH\n";

function textScenario(overrides = {}) {
  return {
    id: "single_text",
    prompt_marker: "TEST_USER_CURRENT",
    expected_history: [],
    require_tool_result: false,
    expected_tool_call_id: null,
    response: { kind: "text", content: "TEST_ASSISTANT_OK" },
    ...overrides,
  };
}

function documentFor(scenarios = [textScenario()]) {
  return {
    version: 1,
    model,
    expected_request_count: scenarios.length,
    scenarios,
  };
}

function parse(value) {
  return parseScenarioDocument(Buffer.from(JSON.stringify(value)));
}

function requestWith(messages, overrides = {}) {
  return { model, stream: true, messages, ...overrides };
}

async function firstLine(stream, maximumBytes = 1024) {
  let buffered = Buffer.alloc(0);
  for await (const chunk of stream) {
    buffered = Buffer.concat([buffered, Buffer.from(chunk)]);
    if (buffered.length > maximumBytes) throw new Error("line_too_large");
    const newline = buffered.indexOf(0x0a);
    if (newline >= 0) {
      assert.equal(buffered.length, newline + 1);
      return buffered.subarray(0, newline).toString("utf8");
    }
  }
  throw new Error("early_eof");
}

test("scenario parser accepts the closed text and paired tool contracts", () => {
  const initial = textScenario({
    id: "source_tool",
    prompt_marker: "LIVE_USER_R1",
    response: {
      kind: "tool_call",
      name: "exec_command",
      call_id: "call_mallard_r1",
      arguments: {
        cmd: safePatchCommand,
        workdir: "/private/tmp/mallard-project",
      },
    },
  });
  const final = textScenario({
    id: "source_final",
    prompt_marker: "LIVE_USER_R1",
    require_tool_result: true,
    expected_tool_call_id: "call_mallard_r1",
    response: { kind: "text", content: "MALLARD_BASE" },
  });
  const parsed = parse(documentFor([initial, final]));
  assert.equal(parsed.scenarios.length, 2);
  assert.equal(parsed.scenarios[0].response.kind, "tool_call");
  assert.equal(parsed.scenarios[1].require_tool_result, true);
});

test("scenario parser rejects unknown fields, duplicate identities, and bad counts", () => {
  const unknown = documentFor();
  unknown.unexpected = true;
  assert.throws(() => parse(unknown), /scenario_document_fields/u);

  const duplicate = documentFor([
    textScenario(),
    textScenario({ prompt_marker: "OTHER_USER_CURRENT" }),
  ]);
  assert.throws(() => parse(duplicate), /duplicate_scenario_id/u);

  const wrongCount = documentFor();
  wrongCount.expected_request_count = 2;
  assert.throws(() => parse(wrongCount), /scenario_count/u);

  const unknownScenarioField = documentFor();
  unknownScenarioField.scenarios[0].unexpected = true;
  assert.throws(() => parse(unknownScenarioField), /scenario_0_fields/u);
});

test("scenario parser supports the expanded revert suite and retains a count bound", () => {
  const scenarios = Array.from({ length: 129 }, (_, index) => textScenario({
    id: `bounded_${index}`,
    prompt_marker: `TEST_USER_${index}`,
  }));
  assert.equal(parse(documentFor(scenarios.slice(0, 66))).scenarios.length, 66);
  assert.equal(parse(documentFor(scenarios.slice(0, 128))).scenarios.length, 128);
  assert.throws(() => parse(documentFor(scenarios)), /scenario_count/u);
});

test("scenario parser rejects unsafe commands and invalid duplicate prompt pairs", () => {
  const unsafe = textScenario({
    response: {
      kind: "tool_call",
      name: "exec_command",
      call_id: "safe_call",
      arguments: {
        cmd: `${safePatchCommand} && curl https://example.invalid`,
        workdir: "/private/tmp/mallard-project",
      },
    },
  });
  assert.throws(() => parse(documentFor([unsafe])), /tool_command_shape/u);

  const first = textScenario({ id: "first" });
  const second = textScenario({ id: "second" });
  assert.throws(
    () => parse(documentFor([first, second])),
    /duplicate_prompt_marker/u,
  );

  const relative = textScenario({
    response: {
      kind: "tool_call",
      name: "exec_command",
      call_id: "safe_call",
      arguments: { cmd: safePatchCommand, workdir: "relative/project" },
    },
  });
  assert.throws(() => parse(documentFor([relative])), /tool_workdir/u);
});

test("leading marker extraction is position-zero, bounded, and token-aware", () => {
  assert.equal(extractLeadingMarker("GRAPH_USER_BASE_1 suffix"), "GRAPH_USER_BASE_1");
  assert.equal(extractLeadingMarker("MALLARD_BASE"), "MALLARD_BASE");
  assert.equal(extractLeadingMarker("prefix GRAPH_USER_BASE_1"), null);
  assert.equal(extractLeadingMarker("Graph_USER_BASE_1"), null);
  assert.equal(extractLeadingMarker("GRAPH-USER-BASE"), null);
  assert.equal(extractLeadingMarker("GRAPH_USER_BASE_1suffix"), null);
});

test("history validation accepts an exact ordered history before journal truncation", () => {
  const scenario = textScenario({
    expected_history: [
      { role: "user", marker: "GRAPH_USER_BASE_1" },
      { role: "assistant", marker: "GRAPH_BASE_1" },
    ],
  });
  const document = parse(documentFor([scenario]));
  const hugeSystemMessage = "x".repeat(70 * 1024);
  const result = validateScenarioRequest(
    document,
    document.scenarios[0],
    requestWith([
      { role: "system", content: hugeSystemMessage },
      { role: "user", content: "GRAPH_USER_BASE_1 instructions" },
      { role: "assistant", content: "GRAPH_BASE_1" },
      { role: "user", content: "TEST_USER_CURRENT verify" },
    ]),
  );
  assert.equal(result.history_match, true);
  assert.equal(result.observed_prior_history_count, 2);
  assert.equal(result.total_input_count, 4);
  assert.equal(result.current_marker_position, 3);
});

test("history validation rejects missing, extra, reordered, role-swapped, and suffixed assistant markers", () => {
  const scenario = textScenario({
    expected_history: [
      { role: "user", marker: "GRAPH_USER_BASE_1" },
      { role: "assistant", marker: "GRAPH_BASE_1" },
    ],
  });
  const document = parse(documentFor([scenario]));
  const subject = document.scenarios[0];
  const current = { role: "user", content: "TEST_USER_CURRENT verify" };
  const cases = [
    [current],
    [
      { role: "user", content: "GRAPH_USER_BASE_1" },
      { role: "assistant", content: "GRAPH_EXTRA" },
      { role: "assistant", content: "GRAPH_BASE_1" },
      current,
    ],
    [
      { role: "assistant", content: "GRAPH_BASE_1" },
      { role: "user", content: "GRAPH_USER_BASE_1" },
      current,
    ],
    [
      { role: "assistant", content: "GRAPH_USER_BASE_1" },
      { role: "user", content: "GRAPH_BASE_1" },
      current,
    ],
    [
      { role: "user", content: "GRAPH_USER_BASE_1" },
      { role: "assistant", content: "GRAPH_BASE_1 trailing" },
      current,
    ],
  ];
  for (const messages of cases) {
    assert.equal(
      validateScenarioRequest(document, subject, requestWith(messages)).history_match,
      false,
    );
  }
});

test("tool-result validation is scoped to the current turn and exact call id", () => {
  const scenario = textScenario({
    require_tool_result: true,
    expected_tool_call_id: "call_current",
  });
  const document = parse(documentFor([scenario]));
  const subject = document.scenarios[0];
  const accepted = validateScenarioRequest(
    document,
    subject,
    requestWith([
      { role: "tool", content: "old", tool_call_id: "call_old" },
      { role: "user", content: "TEST_USER_CURRENT" },
      { role: "tool", content: "done", tool_call_id: "call_current" },
    ]),
  );
  assert.equal(accepted.history_match, true);
  assert.equal(accepted.tool_result_match, true);

  const rejected = validateScenarioRequest(
    document,
    subject,
    requestWith([
      { role: "user", content: "TEST_USER_CURRENT" },
      { role: "tool", content: "done", tool_call_id: "call_wrong" },
    ]),
  );
  assert.equal(rejected.history_match, false);
  assert.equal(rejected.tool_result_match, false);
});

test("argument and shutdown parsers reject ambiguous input", async () => {
  assert.deepEqual(
    parseArguments([
      "--scenario",
      "/private/tmp/aimock-scenario.json",
      "--audit",
      "/private/tmp/aimock-audit.json",
    ]),
    {
      scenarioPath: "/private/tmp/aimock-scenario.json",
      auditPath: "/private/tmp/aimock-audit.json",
    },
  );
  assert.throws(
    () => parseArguments(["--scenario", "relative", "--audit", "/tmp/audit"]),
    /arguments/u,
  );

  const stream = new (await import("node:stream")).PassThrough();
  const result = waitForShutdown(stream);
  stream.end('{"command":"shutdown","version":1}\n');
  await result;

  const invalid = new (await import("node:stream")).PassThrough();
  const failure = waitForShutdown(invalid);
  invalid.end('{"command":"shutdown","version":1,"extra":true}\n');
  await assert.rejects(failure, /shutdown_json/u);
});

test("standalone harness binds loopback, serves one strict scenario, and emits a path-free audit", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "mallard-aimock-test-"));
  t.after(async () => rm(root, { recursive: true, force: true }));
  const scenarioPath = join(root, "aimock-scenario.json");
  const auditPath = join(root, "aimock-audit.json");
  await writeFile(scenarioPath, JSON.stringify(documentFor()), { mode: 0o600 });

  const child = spawn(
    process.execPath,
    [
      resolve("scripts/live-codex-aimock-server.mjs"),
      "--scenario",
      scenarioPath,
      "--audit",
      auditPath,
    ],
    { stdio: ["pipe", "pipe", "pipe"] },
  );
  const stderr = [];
  child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
  const ready = JSON.parse(await firstLine(child.stdout));
  assert.deepEqual(Object.keys(ready).sort(), ["port", "type", "version"]);
  assert.equal(ready.type, "mallard_aimock_ready");
  assert.equal(ready.version, 1);
  assert.ok(Number.isInteger(ready.port) && ready.port > 0);

  const response = await fetch(`http://127.0.0.1:${ready.port}/v1/responses`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model,
      stream: true,
      input: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "TEST_USER_CURRENT verify" }],
        },
      ],
    }),
  });
  assert.equal(response.status, 200);
  assert.match(await response.text(), /TEST_ASSISTANT_OK/u);
  child.stdin.end('{"command":"shutdown","version":1}\n');
  const [code, signal] = await once(child, "exit");
  assert.equal(code, 0, Buffer.concat(stderr).toString("utf8"));
  assert.equal(signal, null);

  const auditText = await readFile(auditPath, "utf8");
  if (process.platform !== "win32") {
    assert.equal((await stat(auditPath)).mode & 0o7777, 0o600);
  }
  assert.ok(!auditText.includes(root));
  const audit = JSON.parse(auditText);
  assert.equal(audit.request_count, 1);
  assert.equal(audit.all_scenarios_consumed_once, true);
  assert.equal(audit.authorization_absent, true);
  assert.equal(audit.responses_path_only, true);
  assert.equal(audit.strict_miss_count, 0);
  assert.equal(audit.scenarios[0].history_match, true);
  assert.equal(audit.scenarios[0].current_marker_position, 0);
});
