import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";

import {
  fixedFixtureResponse,
  parseArguments,
  parseScenarioDocument,
  sanitizeProviderEnvironment,
  validateScenarioRequest,
  waitForShutdown,
} from "./live-claude-aimock-server.mjs";

const model = "claude-sonnet-4-6";
const readPath = "/private/tmp/mallard-claude-read-marker.txt";
const readCallId = "mallard_claude_read_r1";
const readMarker = "MALLARD_CLAUDE_READ_FILE";
const placeholderApiKey = "mallard-local-aimock-placeholder";

function textScenario(overrides = {}) {
  return {
    id: "single_text",
    prompt_marker: "TEST_USER_CURRENT",
    expected_history: [],
    require_tool_result: false,
    expected_tool_call_id: null,
    expected_tool_result_marker: null,
    response: { kind: "text", content: "TEST_ASSISTANT_OK" },
    ...overrides,
  };
}

function readScenarios() {
  return [
    textScenario({
      id: "read_initial",
      prompt_marker: "TOOL_USER_R1",
      response: {
        kind: "read_tool",
        call_id: readCallId,
        file_path: readPath,
      },
    }),
    textScenario({
      id: "read_final",
      prompt_marker: "TOOL_USER_R1",
      require_tool_result: true,
      expected_tool_call_id: readCallId,
      expected_tool_result_marker: readMarker,
      response: { kind: "text", content: "TOOL_ASSISTANT_BASE" },
    }),
  ];
}

function documentFor(
  scenarios = [textScenario()],
  expectedProbeRequestCount = 0,
) {
  return {
    version: 1,
    model,
    expected_request_count: scenarios.length,
    expected_probe_request_count: expectedProbeRequestCount,
    scenarios,
  };
}

function parse(value) {
  return parseScenarioDocument(Buffer.from(JSON.stringify(value)));
}

function normalizedRequest(messages, overrides = {}) {
  return {
    model,
    stream: true,
    messages,
    tools: [
      {
        type: "function",
        function: { name: "Read", description: "Read a file", parameters: {} },
      },
    ],
    ...overrides,
  };
}

function anthropicBody(messages) {
  return {
    model,
    max_tokens: 128,
    stream: true,
    tools: [
      {
        name: "Read",
        description: "Read a file",
        input_schema: {
          type: "object",
          properties: { file_path: { type: "string" } },
          required: ["file_path"],
        },
      },
    ],
    messages,
  };
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

async function startHarness(t, document) {
  const root = await mkdtemp(join(tmpdir(), "mallard-claude-aimock-test-"));
  t.after(async () => rm(root, { recursive: true, force: true }));
  const scenarioPath = join(root, "aimock-scenario.json");
  const auditPath = join(root, "aimock-audit.json");
  await writeFile(scenarioPath, JSON.stringify(document), { mode: 0o600 });

  const child = spawn(
    process.execPath,
    [
      resolve("scripts/live-claude-aimock-server.mjs"),
      "--scenario",
      scenarioPath,
      "--audit",
      auditPath,
    ],
    {
      env: {
        ...process.env,
        ANTHROPIC_API_KEY: "must-not-be-used-by-server",
        HTTPS_PROXY: "http://must-not-be-used.invalid",
      },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  t.after(() => {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
    }
  });
  const stderr = [];
  child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
  const ready = JSON.parse(await firstLine(child.stdout));
  assert.deepEqual(Object.keys(ready).sort(), ["port", "type", "version"]);
  assert.equal(ready.type, "mallard_claude_aimock_ready");
  assert.equal(ready.version, 1);
  assert.ok(Number.isInteger(ready.port) && ready.port > 0);

  return {
    auditPath,
    baseUrl: `http://127.0.0.1:${ready.port}`,
    child,
    root,
    stderr,
  };
}

async function postAnthropic(baseUrl, body, apiKey = placeholderApiKey) {
  return fetch(`${baseUrl}/v1/messages?beta=true`, {
    method: "POST",
    headers: {
      "anthropic-version": "2023-06-01",
      "content-type": "application/json",
      "x-api-key": apiKey,
    },
    body: JSON.stringify(body),
  });
}

async function stopHarness(subject) {
  subject.child.stdin.end('{"command":"shutdown","version":1}\n');
  const [code, signal] = await once(subject.child, "exit");
  assert.equal(code, 0, Buffer.concat(subject.stderr).toString("utf8"));
  assert.equal(signal, null);
  const auditText = await readFile(subject.auditPath, "utf8");
  if (process.platform !== "win32") {
    assert.equal((await stat(subject.auditPath)).mode & 0o7777, 0o600);
  }
  assert.ok(!auditText.includes(subject.root));
  return JSON.parse(auditText);
}

test("scenario parser accepts the closed text and paired Read contracts", () => {
  const parsed = parse(documentFor(readScenarios()));
  assert.equal(parsed.scenarios.length, 2);
  assert.equal(parsed.scenarios[0].response.kind, "read_tool");
  assert.equal(parsed.scenarios[1].require_tool_result, true);
  assert.deepEqual(parsed.read_round, {
    prompt_marker: "TOOL_USER_R1",
    response_marker: "TOOL_ASSISTANT_BASE",
    call_id: readCallId,
    file_path: readPath,
    result_marker: readMarker,
  });

  const fixtureResponse = fixedFixtureResponse(parsed, parsed.scenarios[0]);
  assert.deepEqual(fixtureResponse.toolCalls, [
    {
      id: readCallId,
      name: "Read",
      arguments: JSON.stringify({ file_path: readPath }),
    },
  ]);
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

test("scenario parser rejects unsafe Read paths and invalid prompt pairs", () => {
  const unsafe = readScenarios();
  unsafe[0].response.file_path = "relative/read-marker.txt";
  assert.throws(() => parse(documentFor(unsafe)), /read_file_path/u);

  const unpaired = documentFor([readScenarios()[0]]);
  assert.throws(() => parse(unpaired), /read_scenario_pair/u);

  const wrongCall = readScenarios();
  wrongCall[1].expected_tool_call_id = "different_call";
  assert.throws(() => parse(documentFor(wrongCall)), /duplicate_prompt_marker/u);
});

test("history validation requires exact markers, the Read round, and only Read exposure", () => {
  const scenarios = [
    ...readScenarios(),
    textScenario({
      id: "resume",
      prompt_marker: "MALLARD_CLAUDE_USER_R2",
      expected_history: [
        { role: "user", marker: "TOOL_USER_R1" },
        { role: "assistant", marker: "TOOL_ASSISTANT_BASE" },
      ],
      response: { kind: "text", content: "MALLARD_CLAUDE_REMOTE_BRANCH" },
    }),
  ];
  const document = parse(documentFor(scenarios));
  const subject = document.scenarios[2];
  const messages = [
    {
      role: "user",
      content: "<provider-wrapper>fixture context</provider-wrapper>\nTOOL_USER_R1 read fixture",
    },
    {
      role: "assistant",
      content: null,
      tool_calls: [
        {
          id: readCallId,
          type: "function",
          function: {
            name: "Read",
            arguments: JSON.stringify({ file_path: readPath }),
          },
        },
      ],
    },
    { role: "tool", tool_call_id: readCallId, content: `1→${readMarker}` },
    { role: "assistant", content: "TOOL_ASSISTANT_BASE" },
    {
      role: "user",
      content: "<provider-wrapper>resume context</provider-wrapper>\nMALLARD_CLAUDE_USER_R2 continue",
    },
  ];
  const accepted = validateScenarioRequest(
    document,
    subject,
    normalizedRequest(messages),
  );
  assert.equal(accepted.history_match, true);
  assert.equal(accepted.read_round_match, true);
  assert.equal(accepted.tools_ok, true);

  const relocatedMessages = structuredClone(messages);
  relocatedMessages[1].tool_calls[0].function.arguments = JSON.stringify({
    file_path: "/private/tmp/replica-b/mallard-claude-read-marker.txt",
  });
  const relocatedReplay = validateScenarioRequest(
    document,
    subject,
    normalizedRequest(relocatedMessages),
  );
  assert.equal(relocatedReplay.history_match, true);
  assert.equal(relocatedReplay.read_round_match, true);

  const resultOnlyReplay = validateScenarioRequest(
    document,
    subject,
    normalizedRequest([messages[0], messages[2], messages[3], messages[4]]),
  );
  assert.equal(resultOnlyReplay.history_match, false);
  assert.equal(resultOnlyReplay.read_round_match, false);

  const missingRead = validateScenarioRequest(
    document,
    subject,
    normalizedRequest([messages[0], messages[3], messages[4]]),
  );
  assert.equal(missingRead.history_match, false);
  assert.equal(missingRead.read_round_match, false);

  const extraTool = validateScenarioRequest(
    document,
    subject,
    normalizedRequest(messages, {
      tools: [
        { type: "function", function: { name: "Read" } },
        { type: "function", function: { name: "Bash" } },
      ],
    }),
  );
  assert.equal(extraTool.history_match, false);
  assert.equal(extraTool.tools_ok, false);

  const duplicateMarker = validateScenarioRequest(
    document,
    subject,
    normalizedRequest([
      ...messages.slice(0, -1),
      {
        role: "user",
        content:
          "MALLARD_CLAUDE_USER_R2 and duplicate MALLARD_CLAUDE_USER_R2",
      },
    ]),
  );
  assert.equal(duplicateMarker.history_match, false);
  assert.equal(duplicateMarker.mismatch_code, 13);
});

test("tool-result validation is scoped to the current turn and exact marker", () => {
  const document = parse(documentFor(readScenarios()));
  const subject = document.scenarios[1];
  const toolCall = {
    role: "assistant",
    content: null,
    tool_calls: [
      {
        id: readCallId,
        type: "function",
        function: {
          name: "Read",
          arguments: JSON.stringify({ file_path: readPath }),
        },
      },
    ],
  };
  const accepted = validateScenarioRequest(
    document,
    subject,
    normalizedRequest([
      { role: "user", content: "TOOL_USER_R1" },
      toolCall,
      { role: "tool", tool_call_id: readCallId, content: readMarker },
    ]),
  );
  assert.equal(accepted.history_match, true);
  assert.equal(accepted.tool_result_match, true);

  const rejected = validateScenarioRequest(
    document,
    subject,
    normalizedRequest([
      { role: "user", content: "TOOL_USER_R1" },
      toolCall,
      { role: "tool", tool_call_id: readCallId, content: "WRONG_RESULT" },
    ]),
  );
  assert.equal(rejected.history_match, false);
  assert.equal(rejected.tool_result_match, false);
});

test("argument, environment, and shutdown parsers reject ambiguous input", async () => {
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

  const environment = {
    PATH: "/usr/bin",
    ANTHROPIC_API_KEY: "secret",
    AIMOCK_PROVIDER_OPENAI_KEY: "secret",
    https_proxy: "http://proxy.invalid",
  };
  sanitizeProviderEnvironment(environment);
  assert.deepEqual(environment, { PATH: "/usr/bin" });

  const stream = new PassThrough();
  const result = waitForShutdown(stream);
  stream.end('{"command":"shutdown","version":1}\n');
  await result;

  const invalid = new PassThrough();
  const failure = waitForShutdown(invalid);
  invalid.end('{"command":"shutdown","version":1,"extra":true}\n');
  await assert.rejects(failure, /shutdown_json/u);
});

test("standalone harness serves one strict Claude Messages scenario and emits a path-free audit", async (t) => {
  const subject = await startHarness(t, documentFor());
  const response = await postAnthropic(
    subject.baseUrl,
    anthropicBody([
      {
        role: "user",
        content: [{ type: "text", text: "TEST_USER_CURRENT verify" }],
      },
    ]),
  );
  assert.equal(response.status, 200);
  assert.match(await response.text(), /TEST_ASSISTANT_OK/u);

  const audit = await stopHarness(subject);
  assert.equal(audit.request_count, 1);
  assert.equal(audit.all_scenarios_consumed_once, true);
  assert.equal(audit.strict_mode, true);
  assert.equal(audit.loopback_only, true);
  assert.equal(audit.recording_absent, true);
  assert.equal(audit.upstream_absent, true);
  assert.equal(audit.inbound_auth_required, true);
  assert.equal(audit.fixed_placeholder_enforced, true);
  assert.equal(audit.raw_request_count, 1);
  assert.equal(audit.raw_expected_request_count, 1);
  assert.equal(audit.raw_unexpected_request_count, 0);
  assert.equal(audit.raw_auth_rejection_count, 0);
  assert.equal(audit.raw_journal_reconciled, true);
  assert.equal(audit.raw_audit_ok, true);
  assert.equal(audit.authorization_absent, true);
  assert.equal(audit.x_api_key_redacted_only, true);
  assert.equal(audit.messages_path_only, true);
  assert.equal(audit.strict_miss_count, 0);
  assert.equal(audit.scenarios[0].history_match, true);
  assert.equal(audit.scenarios[0].tools_ok, true);
  assert.equal(audit.scenarios[0].current_marker_position, 0);
});

test("Claude wire response preserves the exact Read name, call id, and JSON arguments", async (t) => {
  const subject = await startHarness(t, documentFor(readScenarios()));
  const initialMessages = [
    {
      role: "user",
      content: [{ type: "text", text: "TOOL_USER_R1 read fixture" }],
    },
  ];
  const initial = await postAnthropic(
    subject.baseUrl,
    anthropicBody(initialMessages),
  );
  assert.equal(initial.status, 200);
  const wire = await initial.text();
  assert.match(wire, new RegExp(readCallId, "u"));
  assert.match(wire, /Read/u);
  assert.match(wire, /file_path/u);
  const argumentFragments = [...wire.matchAll(/"partial_json":"((?:\\.|[^"])*)"/gu)]
    .map((match) => JSON.parse(`"${match[1]}"`))
    .join("");
  assert.deepEqual(JSON.parse(argumentFragments), { file_path: readPath });

  const final = await postAnthropic(
    subject.baseUrl,
    anthropicBody([
      ...initialMessages,
      {
        role: "assistant",
        content: [
          {
            type: "tool_use",
            id: readCallId,
            name: "Read",
            input: { file_path: readPath },
          },
        ],
      },
      {
        role: "user",
        content: [
          {
            type: "tool_result",
            tool_use_id: readCallId,
            content: `1→${readMarker}`,
          },
        ],
      },
    ]),
  );
  assert.equal(final.status, 200);
  assert.match(await final.text(), /TOOL_ASSISTANT_BASE/u);

  const audit = await stopHarness(subject);
  assert.equal(audit.request_count, 2);
  assert.equal(audit.all_scenarios_consumed_once, true);
  assert.equal(audit.scenarios[0].read_round_match, true);
  assert.equal(audit.scenarios[1].tool_result_match, true);
  assert.equal(audit.scenarios[1].read_round_match, true);
});

test("raw boundary audit fails an unexpected route and method omitted from the AIMock journal", async (t) => {
  const subject = await startHarness(t, documentFor());
  const valid = await postAnthropic(
    subject.baseUrl,
    anthropicBody([
      {
        role: "user",
        content: [{ type: "text", text: "TEST_USER_CURRENT verify" }],
      },
    ]),
  );
  assert.equal(valid.status, 200);
  await valid.text();

  const unexpected = await fetch(`${subject.baseUrl}/unexpected-route`, {
    method: "GET",
    headers: { "x-api-key": placeholderApiKey },
  });
  assert.ok(unexpected.status >= 400);
  await unexpected.text();

  const audit = await stopHarness(subject);
  assert.equal(audit.request_count, 1);
  assert.equal(audit.all_scenarios_consumed_once, true);
  assert.equal(audit.raw_request_count, 2);
  assert.equal(audit.raw_expected_request_count, 1);
  assert.equal(audit.raw_unexpected_request_count, 1);
  assert.equal(audit.raw_wrong_method_count, 1);
  assert.equal(audit.raw_wrong_path_count, 1);
  assert.equal(audit.raw_journal_reconciled, true);
  assert.equal(audit.raw_audit_ok, false);
});

test("fixed placeholder auth rejects and audits a wrong local test key without persisting it", async (t) => {
  const subject = await startHarness(t, documentFor());
  const wrongKey = "wrong-local-test-key";
  const body = anthropicBody([
    {
      role: "user",
      content: [{ type: "text", text: "TEST_USER_CURRENT verify" }],
    },
  ]);
  const rejected = await postAnthropic(subject.baseUrl, body, wrongKey);
  assert.equal(rejected.status, 401);
  await rejected.text();

  const accepted = await postAnthropic(subject.baseUrl, body);
  assert.equal(accepted.status, 200);
  await accepted.text();

  const audit = await stopHarness(subject);
  const auditText = JSON.stringify(audit);
  assert.ok(!auditText.includes(wrongKey));
  assert.ok(!auditText.includes(placeholderApiKey));
  assert.equal(audit.request_count, 1);
  assert.equal(audit.raw_request_count, 2);
  assert.equal(audit.raw_expected_request_count, 1);
  assert.equal(audit.raw_unexpected_request_count, 1);
  assert.equal(audit.raw_auth_rejection_count, 1);
  assert.equal(audit.raw_journal_reconciled, true);
  assert.equal(audit.raw_audit_ok, false);
});

test("raw auth audit rejects alternate credential headers even when AIMock accepts them", async (t) => {
  const subject = await startHarness(t, documentFor());
  const response = await fetch(`${subject.baseUrl}/v1/messages?beta=true`, {
    method: "POST",
    headers: {
      "anthropic-version": "2023-06-01",
      "content-type": "application/json",
      "x-api-key": placeholderApiKey,
      "x-goog-api-key": placeholderApiKey,
    },
    body: JSON.stringify(
      anthropicBody([
        {
          role: "user",
          content: [{ type: "text", text: "TEST_USER_CURRENT verify" }],
        },
      ]),
    ),
  });
  assert.equal(response.status, 200);
  await response.text();

  const audit = await stopHarness(subject);
  assert.equal(audit.request_count, 1);
  assert.equal(audit.raw_request_count, 1);
  assert.equal(audit.raw_expected_request_count, 0);
  assert.equal(audit.raw_unexpected_request_count, 1);
  assert.equal(audit.raw_auth_rejection_count, 1);
  assert.equal(audit.raw_alternate_credential_header_count, 1);
  assert.equal(audit.raw_journal_reconciled, false);
  assert.equal(audit.raw_audit_ok, false);
});

test("raw boundary admits the exact credential-free Claude HEAD hello probe budget", async (t) => {
  const subject = await startHarness(t, documentFor(undefined, 1));
  const probe = await fetch(`${subject.baseUrl}/api/hello`, { method: "HEAD" });
  assert.ok(probe.status >= 400);

  const response = await postAnthropic(
    subject.baseUrl,
    anthropicBody([
      {
        role: "user",
        content: [{ type: "text", text: "TEST_USER_CURRENT verify" }],
      },
    ]),
  );
  assert.equal(response.status, 200);
  await response.text();

  const audit = await stopHarness(subject);
  assert.equal(audit.expected_probe_request_count, 1);
  assert.equal(audit.request_count, 1);
  assert.equal(audit.raw_request_count, 2);
  assert.equal(audit.raw_expected_request_count, 2);
  assert.equal(audit.raw_model_request_count, 1);
  assert.equal(audit.raw_probe_request_count, 1);
  assert.equal(audit.raw_unexpected_request_count, 0);
  assert.equal(audit.raw_auth_rejection_count, 0);
  assert.equal(audit.raw_journal_reconciled, true);
  assert.equal(audit.raw_audit_ok, true);
});

test("raw boundary rejects a Claude hello probe carrying any credential", async (t) => {
  const subject = await startHarness(t, documentFor(undefined, 1));
  const probe = await fetch(`${subject.baseUrl}/api/hello`, {
    method: "HEAD",
    headers: { "x-api-key": placeholderApiKey },
  });
  assert.ok(probe.status >= 400);

  const response = await postAnthropic(
    subject.baseUrl,
    anthropicBody([
      {
        role: "user",
        content: [{ type: "text", text: "TEST_USER_CURRENT verify" }],
      },
    ]),
  );
  assert.equal(response.status, 200);
  await response.text();

  const audit = await stopHarness(subject);
  assert.equal(audit.raw_request_count, 2);
  assert.equal(audit.raw_expected_request_count, 1);
  assert.equal(audit.raw_model_request_count, 1);
  assert.equal(audit.raw_probe_request_count, 0);
  assert.equal(audit.raw_unexpected_request_count, 1);
  assert.equal(audit.raw_auth_rejection_count, 1);
  assert.equal(audit.raw_audit_ok, false);
});

test("duplicate valid scenario returns 409 and makes both semantic and raw audits non-green", async (t) => {
  const subject = await startHarness(t, documentFor());
  const body = anthropicBody([
    {
      role: "user",
      content: [{ type: "text", text: "TEST_USER_CURRENT verify" }],
    },
  ]);
  const first = await postAnthropic(subject.baseUrl, body);
  assert.equal(first.status, 200);
  await first.text();
  const duplicate = await postAnthropic(subject.baseUrl, body);
  assert.equal(duplicate.status, 409);
  assert.ok(!(await duplicate.text()).includes("TEST_USER_CURRENT"));

  const audit = await stopHarness(subject);
  assert.equal(audit.request_count, 2);
  assert.equal(audit.duplicate_request_count, 1);
  assert.equal(audit.all_scenarios_consumed_once, false);
  assert.equal(audit.scenarios[0].request_count, 2);
  assert.equal(audit.scenarios[0].history_match, false);
  assert.equal(audit.raw_request_count, 2);
  assert.equal(audit.raw_model_request_count, 2);
  assert.equal(audit.raw_journal_reconciled, true);
  assert.equal(audit.raw_audit_ok, false);
});

test("thinking responses are bounded and preserve the closed text and Read contracts", () => {
  const thinking = { reasoning: "synthetic reasoning", redacted_thinking: ["b3BhcXVl"] };
  const rows = readScenarios();
  for (const row of rows) row.response.thinking = thinking;
  const parsed = parse(documentFor(rows));
  for (const row of parsed.scenarios) {
    const response = fixedFixtureResponse(parsed, row);
    assert.equal(response.reasoning, thinking.reasoning);
    assert.deepEqual(response.redactedThinking, thinking.redacted_thinking);
  }
  for (const invalid of [null, {}, { ...thinking, extra: true },
    { ...thinking, reasoning: "x".repeat(16 * 1024 + 1) },
    { ...thinking, redacted_thinking: Array(5).fill("opaque") },
    { ...thinking, redacted_thinking: [42] }]) {
    assert.throws(() => parse(documentFor([textScenario({ response: {
      kind: "text", content: "TEST_ASSISTANT_OK", thinking: invalid,
    } })])), /thinking/u);
  }
});

test("Claude stream carries thinking and redacted thinking before visible text", async (t) => {
  const subject = await startHarness(t, documentFor([textScenario({ response: {
    kind: "text", content: "TEST_ASSISTANT_OK",
    thinking: { reasoning: "synthetic reasoning", redacted_thinking: ["b3BhcXVl"] },
  } })]));
  const response = await postAnthropic(subject.baseUrl,
    anthropicBody([{ role: "user", content: "TEST_USER_CURRENT" }]));
  assert.equal(response.status, 200);
  const stream = await response.text();
  const events = stream.split("\n").filter((line) => line.startsWith("data: {"))
    .map((line) => JSON.parse(line.slice(6)));
  const starts = events.filter((event) => event.type === "content_block_start");
  assert.deepEqual(starts.map((event) => event.content_block.type),
    ["redacted_thinking", "thinking", "text"]);
  assert.equal(starts[0].content_block.data, "b3BhcXVl");
  assert.equal(events.filter((event) => event.delta?.type === "thinking_delta")
    .map((event) => event.delta.thinking).join(""), "synthetic reasoning");
  assert.ok(events.some((event) => event.delta?.type === "signature_delta"));
  assert.equal(events.filter((event) => event.delta?.type === "text_delta")
    .map((event) => event.delta.text).join(""), "TEST_ASSISTANT_OK");
  const audit = await stopHarness(subject);
  assert.equal(audit.all_scenarios_consumed_once, true);
  assert.equal(audit.raw_audit_ok, true);
});
