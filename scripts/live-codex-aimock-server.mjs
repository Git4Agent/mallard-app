import { LLMock } from "@copilotkit/aimock";
import { constants } from "node:fs";
import { lstat, open, writeFile } from "node:fs/promises";
import { basename, dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCENARIO_VERSION = 1;
const MAX_SCENARIO_BYTES = 4 * 1024 * 1024;
const MAX_SCENARIOS = 128;
const MAX_EXPECTED_HISTORY_ITEMS = 256;
const MAX_INPUT_MESSAGES = 512;
const MAX_COMMAND_BYTES = 4 * 1024;
const MAX_WORKDIR_BYTES = 1024;
const MAX_AUDIT_BYTES = 128 * 1024;
const MAX_SHUTDOWN_BYTES = 256;
const MAX_JOURNAL_ENTRIES = 128;
const MARKER_PATTERN = /^[A-Z][A-Z0-9_]*_[A-Z0-9_]+$/u;
const ID_PATTERN = /^[a-z][a-z0-9_]{0,63}$/u;
const CALL_ID_PATTERN = /^[A-Za-z0-9_-]{1,96}$/u;
const PATCH_PREFIX = "apply_patch <<'MALLARD_PATCH'\n";
const PATCH_SUFFIX = "MALLARD_PATCH\n";

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function assertExactKeys(value, expected, label) {
  if (!isRecord(value)) {
    throw new Error(`${label}_not_object`);
  }
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (
    actual.length !== wanted.length ||
    actual.some((key, index) => key !== wanted[index])
  ) {
    throw new Error(`${label}_fields`);
  }
}

function byteLength(value) {
  return Buffer.byteLength(value, "utf8");
}

function isBoundedString(value, maximumBytes) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    byteLength(value) <= maximumBytes &&
    !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u.test(value)
  );
}

function isMarker(value) {
  return (
    isBoundedString(value, 128) &&
    MARKER_PATTERN.test(value) &&
    value.includes("_")
  );
}

// A text fixture is one leading marker followed by a bounded exact body.
// The request oracle compares the entire assistant body, including whitespace.
function contentMarker(value) {
  return typeof value === "string" ? value.split("\n", 1)[0] : null;
}

function isAssistantContent(value) {
  return isBoundedString(value, 32 * 1024) && isAssistantMarker(contentMarker(value));
}

function isUserMarker(value) {
  return isMarker(value) && (value.includes("_USER_") || value.endsWith("_USER"));
}

function isAssistantMarker(value) {
  return isMarker(value) && !isUserMarker(value);
}

function validateExpectedHistory(value, label) {
  if (!Array.isArray(value) || value.length > MAX_EXPECTED_HISTORY_ITEMS) {
    throw new Error(`${label}_count`);
  }
  return value.map((entry, index) => {
    assertExactKeys(entry, ["marker", "role"], `${label}_${index}`);
    if (entry.role !== "user" && entry.role !== "assistant") {
      throw new Error(`${label}_${index}_role`);
    }
    if (
      (entry.role === "user" && !isUserMarker(entry.marker)) ||
      (entry.role === "assistant" && !isAssistantContent(entry.marker))
    ) {
      throw new Error(`${label}_${index}_marker`);
    }
    return { role: entry.role, marker: entry.marker };
  });
}

function validatePatchCommand(value) {
  if (
    !isBoundedString(value, MAX_COMMAND_BYTES) ||
    !/^[\x20-\x7e\n]+$/u.test(value) ||
    !value.startsWith(PATCH_PREFIX) ||
    !value.endsWith(PATCH_SUFFIX) ||
    value.includes("\r") ||
    value.includes("`") ||
    value.includes("$(") ||
    value.includes("&&") ||
    value.includes("||") ||
    value.includes(";") ||
    value.split(PATCH_PREFIX).length !== 2 ||
    value.split(PATCH_SUFFIX).length !== 2
  ) {
    throw new Error("tool_command_shape");
  }

  const patch = value.slice(PATCH_PREFIX.length, -PATCH_SUFFIX.length);
  if (
    !patch.startsWith("*** Begin Patch\n") ||
    !patch.endsWith("*** End Patch\n") ||
    patch.match(/^\*\*\* Begin Patch$/gmu)?.length !== 1 ||
    patch.match(/^\*\*\* End Patch$/gmu)?.length !== 1 ||
    patch.match(/^\*\*\* Add File: .+$/gmu)?.length !== 4
  ) {
    throw new Error("tool_patch_shape");
  }

  for (const line of patch.split("\n")) {
    if (
      line.length !== 0 &&
      line !== "*** Begin Patch" &&
      line !== "*** End Patch" &&
      !line.startsWith("*** Add File: ") &&
      !line.startsWith("+")
    ) {
      throw new Error("tool_patch_line");
    }
  }
}

function validateWorkdir(value) {
  if (
    !isBoundedString(value, MAX_WORKDIR_BYTES) ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(value) ||
    !isAbsolute(value) ||
    resolve(value) !== value ||
    value === "/"
  ) {
    throw new Error("tool_workdir");
  }
}

function validateResponse(value, label) {
  if (!isRecord(value) || typeof value.kind !== "string") {
    throw new Error(`${label}_shape`);
  }
  if (value.kind === "text") {
    assertExactKeys(value, ["content", "kind"], label);
    if (!isAssistantContent(value.content)) {
      throw new Error(`${label}_content`);
    }
    return { kind: "text", content: value.content };
  }
  if (value.kind === "tool_call") {
    assertExactKeys(
      value,
      ["arguments", "call_id", "kind", "name"],
      label,
    );
    if (value.name !== "exec_command" || !CALL_ID_PATTERN.test(value.call_id)) {
      throw new Error(`${label}_tool`);
    }
    assertExactKeys(value.arguments, ["cmd", "workdir"], `${label}_arguments`);
    validateWorkdir(value.arguments.workdir);
    const read = /^cat -- '([^'\r\n]+\/mixed-read\.txt)'$/u.exec(value.arguments.cmd);
    if (read) {
      if (!isAbsolute(read[1]) || resolve(read[1]) !== read[1] || dirname(read[1]) !== value.arguments.workdir) {
        throw new Error("tool_read_path");
      }
    } else {
      validatePatchCommand(value.arguments.cmd);
    }
    return {
      kind: "tool_call",
      name: value.name,
      call_id: value.call_id,
      arguments: {
        cmd: value.arguments.cmd,
        workdir: value.arguments.workdir,
      },
    };
  }
  throw new Error(`${label}_kind`);
}

function parseScenarioDocument(input) {
  const bytes = Buffer.isBuffer(input) ? input : Buffer.from(input);
  if (bytes.length === 0 || bytes.length > MAX_SCENARIO_BYTES) {
    throw new Error("scenario_bytes");
  }
  const text = bytes.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(bytes)) {
    throw new Error("scenario_utf8");
  }
  let value;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error("scenario_json");
  }
  assertExactKeys(
    value,
    ["expected_request_count", "model", "scenarios", "version"],
    "scenario_document",
  );
  if (value.version !== SCENARIO_VERSION) {
    throw new Error("scenario_version");
  }
  if (value.model !== "gpt-5.6-luna") {
    throw new Error("scenario_model");
  }
  if (
    !Number.isSafeInteger(value.expected_request_count) ||
    value.expected_request_count < 1 ||
    value.expected_request_count > MAX_SCENARIOS ||
    !Array.isArray(value.scenarios) ||
    value.scenarios.length !== value.expected_request_count
  ) {
    throw new Error("scenario_count");
  }

  const ids = new Set();
  const scenarios = value.scenarios.map((scenario, index) => {
    const label = `scenario_${index}`;
    assertExactKeys(
      scenario,
      [
        "expected_history",
        "expected_tool_call_id",
        "id",
        "prompt_marker",
        "require_tool_result",
        "response",
      ],
      label,
    );
    if (typeof scenario.id !== "string" || !ID_PATTERN.test(scenario.id)) {
      throw new Error(`${label}_id`);
    }
    if (ids.has(scenario.id)) {
      throw new Error("duplicate_scenario_id");
    }
    ids.add(scenario.id);
    if (!isUserMarker(scenario.prompt_marker)) {
      throw new Error(`${label}_prompt_marker`);
    }
    if (typeof scenario.require_tool_result !== "boolean") {
      throw new Error(`${label}_require_tool_result`);
    }
    if (
      scenario.require_tool_result
        ? typeof scenario.expected_tool_call_id !== "string" ||
          !CALL_ID_PATTERN.test(scenario.expected_tool_call_id)
        : scenario.expected_tool_call_id !== null
    ) {
      throw new Error(`${label}_expected_tool_call_id`);
    }
    return {
      id: scenario.id,
      prompt_marker: scenario.prompt_marker,
      expected_history: validateExpectedHistory(
        scenario.expected_history,
        `${label}_history`,
      ),
      require_tool_result: scenario.require_tool_result,
      expected_tool_call_id: scenario.expected_tool_call_id,
      response: validateResponse(scenario.response, `${label}_response`),
    };
  });

  const byPrompt = new Map();
  for (const scenario of scenarios) {
    const group = byPrompt.get(scenario.prompt_marker) ?? [];
    group.push(scenario);
    byPrompt.set(scenario.prompt_marker, group);
  }
  for (const group of byPrompt.values()) {
    if (group.length === 1) {
      continue;
    }
    const initial = group.find((scenario) => !scenario.require_tool_result);
    const final = group.find((scenario) => scenario.require_tool_result);
    if (
      group.length !== 2 ||
      !initial ||
      !final ||
      initial.response.kind !== "tool_call" ||
      final.response.kind !== "text" ||
      final.expected_tool_call_id !== initial.response.call_id ||
      JSON.stringify(initial.expected_history) !==
        JSON.stringify(final.expected_history)
    ) {
      throw new Error("duplicate_prompt_marker");
    }
  }

  return {
    version: SCENARIO_VERSION,
    model: value.model,
    expected_request_count: value.expected_request_count,
    scenarios,
  };
}

function extractLeadingMarker(text) {
  if (typeof text !== "string" || text.length === 0) {
    return null;
  }
  const match = /^([A-Z][A-Z0-9_]*_[A-Z0-9_]+)(?=$|[^A-Za-z0-9_])/u.exec(text);
  return match?.[1] ?? null;
}

function messageText(message) {
  if (typeof message?.content === "string") {
    return message.content;
  }
  if (!Array.isArray(message?.content)) {
    return null;
  }
  return message.content
    .filter((part) => isRecord(part) && typeof part.text === "string")
    .map((part) => part.text)
    .join("");
}

function validateScenarioRequest(document, scenario, request) {
  const result = {
    mismatch_code: 0,
    mismatch_index: 0,
    observed_prior_history_count: 0,
    total_input_count: 0,
    current_marker_position: -1,
    history_match: false,
    tool_result_match: false,
    model_ok: false,
    stream_ok: false,
  };
  const fail = (code, index = 0) => {
    if (result.mismatch_code === 0) {
      result.mismatch_code = code;
      result.mismatch_index = index;
    }
  };

  result.model_ok = request?.model === document.model;
  result.stream_ok = request?.stream === true;
  if (!result.model_ok) fail(2);
  if (!result.stream_ok) fail(3);
  if (!Array.isArray(request?.messages) || request.messages.length > MAX_INPUT_MESSAGES) {
    fail(4, Array.isArray(request?.messages) ? request.messages.length : 0);
    return result;
  }
  result.total_input_count = request.messages.length;

  const markers = [];
  let assistantShapeMismatch = -1;
  for (let index = 0; index < request.messages.length; index += 1) {
    const message = request.messages[index];
    if (message?.role !== "user" && message?.role !== "assistant") {
      continue;
    }
    const text = messageText(message);
    const marker = extractLeadingMarker(text);
    if (marker === null) {
      continue;
    }
    const expectedBody = scenario.expected_history.find(
      (entry) => entry.role === "assistant" && contentMarker(entry.marker) === marker,
    )?.marker;
    if (message.role === "assistant" && text !== expectedBody) {
      assistantShapeMismatch = index;
    }
    markers.push({ role: message.role, marker, input_index: index });
  }

  const current = markers.filter(
    (entry) => entry.role === "user" && entry.marker === scenario.prompt_marker,
  );
  if (current.length === 1) {
    result.current_marker_position = current[0].input_index;
  } else {
    fail(5, current.length);
  }
  result.observed_prior_history_count = Math.max(0, markers.length - current.length);
  const expected = [
    ...scenario.expected_history,
    { role: "user", marker: scenario.prompt_marker },
  ];
  if (markers.length !== expected.length) {
    fail(6, Math.min(markers.length, expected.length));
  } else {
    for (let index = 0; index < expected.length; index += 1) {
      if (
        markers[index].role !== expected[index].role ||
        markers[index].marker !== contentMarker(expected[index].marker)
      ) {
        fail(7, index);
        break;
      }
    }
  }
  if (assistantShapeMismatch >= 0) {
    fail(8, assistantShapeMismatch);
  }

  let lastUserIndex = -1;
  for (let index = request.messages.length - 1; index >= 0; index -= 1) {
    if (request.messages[index]?.role === "user") {
      lastUserIndex = index;
      break;
    }
  }
  const currentTurnTools = request.messages
    .slice(lastUserIndex + 1)
    .filter((message) => message?.role === "tool");
  if (scenario.require_tool_result) {
    result.tool_result_match =
      currentTurnTools.length === 1 &&
      currentTurnTools[0].tool_call_id === scenario.expected_tool_call_id &&
      typeof currentTurnTools[0].content === "string" &&
      currentTurnTools[0].content.length > 0 &&
      byteLength(currentTurnTools[0].content) <= 64 * 1024;
  } else {
    result.tool_result_match = currentTurnTools.length === 0;
  }
  if (!result.tool_result_match) {
    fail(9, currentTurnTools.length);
  }
  result.history_match = result.mismatch_code === 0;
  return result;
}

function fixedFixtureResponse(document, scenario) {
  const overrides = {
    id: `resp_${scenario.id}`,
    created: 1,
    model: document.model,
    usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
  };
  if (scenario.response.kind === "text") {
    return { content: scenario.response.content, ...overrides };
  }
  return {
    toolCalls: [
      {
        id: scenario.response.call_id,
        name: scenario.response.name,
        arguments: JSON.stringify(scenario.response.arguments),
      },
    ],
    ...overrides,
  };
}

function makeObservations(document) {
  return new Map(
    document.scenarios.map((scenario) => [
      scenario.id,
      {
        request_count: 0,
        valid_request_count: 0,
        duplicate_request_count: 0,
        history_mismatch_count: 0,
        tool_result_mismatch_count: 0,
        first_validation: null,
      },
    ]),
  );
}

function registerFixtures(mock, document, observations) {
  const fixtureIds = new Map();
  for (const scenario of document.scenarios) {
    const observation = observations.get(scenario.id);
    const match = {
      userMessage: new RegExp(`(?:^|[^A-Za-z0-9_])${scenario.prompt_marker}(?=$|[^A-Za-z0-9_])`, "u"),
      hasToolResult: scenario.require_tool_result,
      endpoint: "chat",
    };
    if (scenario.require_tool_result) {
      match.toolCallId = scenario.expected_tool_call_id;
    }
    const fixture = {
      match,
      response: (request) => {
        observation.request_count += 1;
        if (observation.request_count > 1) {
          observation.duplicate_request_count += 1;
          return {
            error: {
              message: "deterministic scenario was requested more than once",
              type: "invalid_request_error",
              code: "mallard_duplicate_scenario",
            },
            status: 409,
            fallthrough: false,
          };
        }
        const validation = validateScenarioRequest(document, scenario, request);
        observation.first_validation = validation;
        if (!validation.history_match) {
          observation.history_mismatch_count += 1;
          if (!validation.tool_result_match) {
            observation.tool_result_mismatch_count += 1;
          }
          return {
            error: {
              message: `deterministic scenario request did not match: code=${validation.mismatch_code},index=${validation.mismatch_index},observed=${validation.observed_prior_history_count},total=${validation.total_input_count}`,
              type: "invalid_request_error",
              code: "mallard_scenario_mismatch",
            },
            status: 422,
            fallthrough: false,
          };
        }
        observation.valid_request_count += 1;
        return fixedFixtureResponse(document, scenario);
      },
    };
    fixtureIds.set(fixture, scenario.id);
    mock.addFixture(fixture);
  }
  return fixtureIds;
}

function headerValue(headers, name) {
  const key = Object.keys(headers ?? {}).find(
    (candidate) => candidate.toLowerCase() === name,
  );
  return key === undefined ? undefined : headers[key];
}

function isJsonContentType(headers) {
  const value = headerValue(headers, "content-type");
  return (
    typeof value === "string" &&
    value.split(";", 1)[0].trim().toLowerCase() === "application/json"
  );
}

function buildAudit(document, entries, fixtureIds, observations) {
  const grouped = new Map(document.scenarios.map((scenario) => [scenario.id, []]));
  let unknownRequestCount = 0;
  let strictMissCount = 0;
  for (const entry of entries) {
    const id = entry.response.fixture
      ? fixtureIds.get(entry.response.fixture)
      : undefined;
    if (id === undefined) {
      unknownRequestCount += 1;
      if (entry.response.status === 503) strictMissCount += 1;
    } else {
      grouped.get(id).push(entry);
    }
  }

  const scenarios = document.scenarios.map((scenario) => {
    const scenarioEntries = grouped.get(scenario.id);
    const observation = observations.get(scenario.id);
    const validation = observation.first_validation;
    const present = scenarioEntries.length > 0;
    const every = (predicate) => present && scenarioEntries.every(predicate);
    const mismatchCode =
      observation.duplicate_request_count > 0
        ? 1
        : (validation?.mismatch_code ?? 10);
    return {
      id: scenario.id,
      request_count: scenarioEntries.length,
      method_ok: every((entry) => entry.method === "POST"),
      path_ok: every((entry) => entry.path === "/v1/responses"),
      query_absent: every((entry) => !entry.path.includes("?")),
      content_type_json: every((entry) => isJsonContentType(entry.headers)),
      content_encoding_absent: every(
        (entry) => headerValue(entry.headers, "content-encoding") === undefined,
      ),
      authorization_absent: every(
        (entry) => headerValue(entry.headers, "authorization") === undefined,
      ),
      status_ok: every((entry) => entry.response.status === 200),
      model_ok: validation?.model_ok ?? false,
      stream_ok: validation?.stream_ok ?? false,
      expected_prior_history_count: scenario.expected_history.length,
      observed_prior_history_count:
        validation?.observed_prior_history_count ?? 0,
      total_input_count: validation?.total_input_count ?? 0,
      current_marker_position: validation?.current_marker_position ?? -1,
      history_match:
        observation.request_count === 1 &&
        observation.valid_request_count === 1 &&
        observation.history_mismatch_count === 0,
      tool_result_match: validation?.tool_result_match ?? false,
      mismatch_code: mismatchCode,
      mismatch_index: validation?.mismatch_index ?? 0,
    };
  });

  const allEntries = entries.length > 0;
  const audit = {
    version: SCENARIO_VERSION,
    expected_request_count: document.expected_request_count,
    request_count: entries.length,
    journal_saturated: entries.length >= MAX_JOURNAL_ENTRIES,
    post_only: allEntries && entries.every((entry) => entry.method === "POST"),
    responses_path_only:
      allEntries && entries.every((entry) => entry.path === "/v1/responses"),
    query_absent: allEntries && entries.every((entry) => !entry.path.includes("?")),
    content_type_json_only:
      allEntries && entries.every((entry) => isJsonContentType(entry.headers)),
    content_encoding_absent:
      allEntries &&
      entries.every(
        (entry) => headerValue(entry.headers, "content-encoding") === undefined,
      ),
    authorization_absent:
      allEntries &&
      entries.every(
        (entry) => headerValue(entry.headers, "authorization") === undefined,
      ),
    unknown_request_count: unknownRequestCount,
    strict_miss_count: strictMissCount,
    history_mismatch_count: [...observations.values()].reduce(
      (total, observation) => total + observation.history_mismatch_count,
      0,
    ),
    duplicate_request_count: [...observations.values()].reduce(
      (total, observation) => total + observation.duplicate_request_count,
      0,
    ),
    unused_scenario_count: scenarios.filter((scenario) => scenario.request_count === 0)
      .length,
    all_scenarios_consumed_once: scenarios.every(
      (scenario) => scenario.request_count === 1,
    ),
    scenarios,
  };
  return audit;
}

function stableMetadata(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.size === right.size &&
    left.mtimeMs === right.mtimeMs &&
    left.ctimeMs === right.ctimeMs
  );
}

async function readScenarioFile(path) {
  const before = await lstat(path);
  if (
    before.isSymbolicLink() ||
    !before.isFile() ||
    before.size < 1 ||
    before.size > MAX_SCENARIO_BYTES ||
    (process.platform !== "win32" && (before.mode & 0o7777) !== 0o600)
  ) {
    throw new Error("scenario_file");
  }
  const noFollow = typeof constants.O_NOFOLLOW === "number" ? constants.O_NOFOLLOW : 0;
  const handle = await open(path, constants.O_RDONLY | noFollow);
  try {
    const descriptorBefore = await handle.stat();
    if (!stableMetadata(before, descriptorBefore)) {
      throw new Error("scenario_file_changed");
    }
    const bytes = await handle.readFile();
    const descriptorAfter = await handle.stat();
    const after = await lstat(path);
    if (
      bytes.length !== descriptorBefore.size ||
      !stableMetadata(descriptorBefore, descriptorAfter) ||
      !stableMetadata(descriptorAfter, after)
    ) {
      throw new Error("scenario_file_changed");
    }
    return bytes;
  } finally {
    await handle.close();
  }
}

function parseArguments(argv) {
  if (
    argv.length !== 4 ||
    argv[0] !== "--scenario" ||
    argv[2] !== "--audit" ||
    !isAbsolute(argv[1]) ||
    !isAbsolute(argv[3]) ||
    resolve(argv[1]) !== argv[1] ||
    resolve(argv[3]) !== argv[3] ||
    dirname(argv[1]) !== dirname(argv[3]) ||
    basename(argv[1]) !== "aimock-scenario.json" ||
    basename(argv[3]) !== "aimock-audit.json"
  ) {
    throw new Error("arguments");
  }
  return { scenarioPath: argv[1], auditPath: argv[3] };
}

async function validateControlDirectory(path) {
  const metadata = await lstat(path);
  if (
    metadata.isSymbolicLink() ||
    !metadata.isDirectory() ||
    (process.platform !== "win32" && (metadata.mode & 0o077) !== 0)
  ) {
    throw new Error("control_directory");
  }
}

async function requireAbsent(path) {
  try {
    await lstat(path);
  } catch (error) {
    if (error?.code === "ENOENT") return;
    throw error;
  }
  throw new Error("audit_exists");
}

function waitForShutdown(stream) {
  return new Promise((accept, reject) => {
    let bytes = Buffer.alloc(0);
    let totalBytes = 0;
    let parsed = false;
    const cleanup = () => {
      stream.off("data", onData);
      stream.off("end", onEnd);
      stream.off("error", onError);
    };
    const fail = (code) => {
      cleanup();
      reject(new Error(code));
    };
    const onError = () => fail("shutdown_stream");
    const onEnd = () => {
      if (!parsed) {
        fail("shutdown_eof");
        return;
      }
      cleanup();
      accept();
    };
    const onData = (chunk) => {
      if (parsed) {
        fail("shutdown_command");
        return;
      }
      totalBytes += chunk.length;
      bytes = Buffer.concat([bytes, Buffer.from(chunk)]);
      if (totalBytes > MAX_SHUTDOWN_BYTES) {
        fail("shutdown_bytes");
        return;
      }
      const newline = bytes.indexOf(0x0a);
      if (newline < 0) return;
      const line = bytes.subarray(0, newline).toString("utf8");
      const tail = bytes.subarray(newline + 1);
      let command;
      try {
        command = JSON.parse(line);
        assertExactKeys(command, ["command", "version"], "shutdown");
      } catch {
        fail("shutdown_json");
        return;
      }
      if (
        command.command !== "shutdown" ||
        command.version !== SCENARIO_VERSION ||
        tail.length !== 0
      ) {
        fail("shutdown_command");
        return;
      }
      parsed = true;
      bytes = Buffer.alloc(0);
    };
    stream.on("data", onData);
    stream.on("end", onEnd);
    stream.on("error", onError);
    stream.resume();
  });
}

async function writeAudit(path, audit) {
  const encoded = `${JSON.stringify(audit)}\n`;
  if (byteLength(encoded) > MAX_AUDIT_BYTES) {
    throw new Error("audit_bytes");
  }
  await writeFile(path, encoded, { encoding: "utf8", flag: "wx", mode: 0o600 });
}

async function runHarness(argv = process.argv.slice(2)) {
  const { scenarioPath, auditPath } = parseArguments(argv);
  await validateControlDirectory(dirname(scenarioPath));
  await requireAbsent(auditPath);
  const document = parseScenarioDocument(await readScenarioFile(scenarioPath));
  const mock = new LLMock({
    host: "127.0.0.1",
    port: 0,
    strict: true,
    logLevel: "silent",
    journalMaxEntries: MAX_JOURNAL_ENTRIES,
    fixtureCountsMaxTestIds: 8,
  });
  const observations = makeObservations(document);
  const fixtureIds = registerFixtures(mock, document, observations);
  let started = false;
  try {
    const url = await mock.start();
    started = true;
    const expectedUrl = `http://127.0.0.1:${mock.port}`;
    if (
      url !== expectedUrl ||
      mock.port < 1 ||
      mock.port > 65535 ||
      mock.url !== expectedUrl
    ) {
      throw new Error("listen_endpoint");
    }
    process.stdout.write(
      `${JSON.stringify({
        type: "mallard_aimock_ready",
        version: SCENARIO_VERSION,
        port: mock.port,
      })}\n`,
    );
    await waitForShutdown(process.stdin);
    const entries = mock.getRequests();
    await mock.stop();
    started = false;
    const audit = buildAudit(
      document,
      entries,
      fixtureIds,
      observations,
    );
    await writeAudit(auditPath, audit);
    return audit;
  } finally {
    if (started) {
      await mock.stop().catch(() => {});
    }
  }
}

const isMain =
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url));

if (isMain) {
  runHarness().catch((error) => {
    console.error(/^[a-z0-9_]{1,96}$/u.test(error?.message) ? error.message : "harness_failed");
    process.exitCode = 1;
  });
}

export {
  MAX_SCENARIO_BYTES,
  buildAudit,
  extractLeadingMarker,
  parseArguments,
  parseScenarioDocument,
  registerFixtures,
  runHarness,
  validateScenarioRequest,
  waitForShutdown,
};
