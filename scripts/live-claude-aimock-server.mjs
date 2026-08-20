import { constants } from "node:fs";
import { lstat, open, writeFile } from "node:fs/promises";
import { basename, dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCENARIO_VERSION = 1;
const CLAUDE_MODEL = "claude-sonnet-4-6";
const MAX_SCENARIO_BYTES = 512 * 1024;
const MAX_SCENARIOS = 64;
const MAX_EXPECTED_HISTORY_ITEMS = 64;
const MAX_MARKER_CATALOG_ITEMS = 256;
const MAX_INPUT_MESSAGES = 512;
const MAX_MESSAGE_TEXT_BYTES = 256 * 1024;
const MAX_TOOLS = 8;
const MAX_PATH_BYTES = 1024;
const MAX_AUDIT_BYTES = 128 * 1024;
const MAX_SHUTDOWN_BYTES = 256;
const MAX_JOURNAL_ENTRIES = 128;
const MAX_RAW_REQUESTS = 256;
const MAX_RAW_TARGET_BYTES = 2048;
const MARKER_PATTERN = /^[A-Z][A-Z0-9_]*_[A-Z0-9_]+$/u;
const ID_PATTERN = /^[a-z][a-z0-9_]{0,63}$/u;
const CALL_ID_PATTERN = /^[A-Za-z0-9_-]{1,96}$/u;
const PLACEHOLDER_API_KEY = "mallard-local-aimock-placeholder";
const EXPECTED_MESSAGES_TARGET = "/v1/messages?beta=true";
const EXPECTED_HELLO_TARGET = "/api/hello";
const MOCK_CONFIGURATION = Object.freeze({
  auth: Object.freeze({
    apiKeys: Object.freeze([PLACEHOLDER_API_KEY]),
  }),
  host: "127.0.0.1",
  port: 0,
  strict: true,
  logLevel: "silent",
  journalMaxEntries: MAX_JOURNAL_ENTRIES,
  fixtureCountsMaxTestIds: 16,
});

const removedEnvironmentVariables = new Set([
  "AIMOCK_DUMMY_KEY_MARKER",
  "ALL_PROXY",
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_AUTH_TOKEN",
  "ANTHROPIC_BASE_URL",
  "ANTHROPIC_BEDROCK_BASE_URL",
  "ANTHROPIC_BETAS",
  "ANTHROPIC_CUSTOM_HEADERS",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  "ANTHROPIC_DEFAULT_OPUS_MODEL",
  "ANTHROPIC_DEFAULT_SONNET_MODEL",
  "ANTHROPIC_FOUNDRY_API_KEY",
  "ANTHROPIC_FOUNDRY_BASE_URL",
  "ANTHROPIC_FOUNDRY_RESOURCE",
  "ANTHROPIC_MODEL",
  "ANTHROPIC_VERTEX_BASE_URL",
  "ANTHROPIC_VERTEX_PROJECT_ID",
  "AWS_ACCESS_KEY_ID",
  "AWS_SECRET_ACCESS_KEY",
  "AWS_SESSION_TOKEN",
  "AZURE_OPENAI_API_KEY",
  "AZURE_OPENAI_ENDPOINT",
  "CLAUDE_CODE_OAUTH_TOKEN",
  "CLAUDE_CODE_USE_BEDROCK",
  "CLAUDE_CODE_USE_FOUNDRY",
  "CLAUDE_CODE_USE_MANTLE",
  "CLAUDE_CODE_USE_VERTEX",
  "CLAUDE_CONFIG_DIR",
  "CODEX_API_KEY",
  "CODEX_HOME",
  "GEMINI_API_KEY",
  "GOOGLE_API_KEY",
  "HTTPS_PROXY",
  "HTTP_PROXY",
  "NO_PROXY",
  "OPENAI_API_KEY",
  "OPENAI_BASE_URL",
  "OPENAI_ORGANIZATION",
  "OPENAI_ORG_ID",
  "OPENAI_PROJECT_ID",
  "all_proxy",
  "http_proxy",
  "https_proxy",
  "no_proxy",
]);

function sanitizeProviderEnvironment(environment = process.env) {
  for (const name of Object.keys(environment)) {
    if (
      removedEnvironmentVariables.has(name) ||
      name.startsWith("AIMOCK_PROVIDER_") ||
      name.startsWith("ANTHROPIC_") ||
      name.startsWith("AWS_") ||
      name.startsWith("AZURE_") ||
      name.startsWith("CLAUDE_CODE_") ||
      name.startsWith("CODEX_") ||
      name.startsWith("GEMINI_") ||
      name.startsWith("GOOGLE_") ||
      name.startsWith("OPENAI_")
    ) {
      delete environment[name];
    }
  }
}

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
  return isBoundedString(value, 128) && MARKER_PATTERN.test(value);
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
      (entry.role === "assistant" && !isAssistantMarker(entry.marker))
    ) {
      throw new Error(`${label}_${index}_marker`);
    }
    return { role: entry.role, marker: entry.marker };
  });
}

function validateReadPath(value) {
  if (
    !isBoundedString(value, MAX_PATH_BYTES) ||
    !isAbsolute(value) ||
    resolve(value) !== value ||
    value === "/"
  ) {
    throw new Error("read_file_path");
  }
}

function validateResponse(value, label) {
  if (!isRecord(value) || typeof value.kind !== "string") {
    throw new Error(`${label}_shape`);
  }
  if (value.kind === "text") {
    assertExactKeys(value, ["content", "kind"], label);
    if (!isAssistantMarker(value.content)) {
      throw new Error(`${label}_content`);
    }
    return { kind: "text", content: value.content };
  }
  if (value.kind === "read_tool") {
    assertExactKeys(value, ["call_id", "file_path", "kind"], label);
    if (
      typeof value.call_id !== "string" ||
      !CALL_ID_PATTERN.test(value.call_id)
    ) {
      throw new Error(`${label}_call_id`);
    }
    validateReadPath(value.file_path);
    return {
      kind: "read_tool",
      call_id: value.call_id,
      file_path: value.file_path,
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
    [
      "expected_probe_request_count",
      "expected_request_count",
      "model",
      "scenarios",
      "version",
    ],
    "scenario_document",
  );
  if (value.version !== SCENARIO_VERSION) {
    throw new Error("scenario_version");
  }
  if (value.model !== CLAUDE_MODEL) {
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
  if (
    !Number.isSafeInteger(value.expected_probe_request_count) ||
    value.expected_probe_request_count < 0 ||
    value.expected_probe_request_count > MAX_SCENARIOS
  ) {
    throw new Error("scenario_probe_count");
  }

  const ids = new Set();
  const scenarios = value.scenarios.map((scenario, index) => {
    const label = `scenario_${index}`;
    assertExactKeys(
      scenario,
      [
        "expected_history",
        "expected_tool_call_id",
        "expected_tool_result_marker",
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
          !CALL_ID_PATTERN.test(scenario.expected_tool_call_id) ||
          !isMarker(scenario.expected_tool_result_marker)
        : scenario.expected_tool_call_id !== null ||
          scenario.expected_tool_result_marker !== null
    ) {
      throw new Error(`${label}_tool_result_contract`);
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
      expected_tool_result_marker: scenario.expected_tool_result_marker,
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
      initial.response.kind !== "read_tool" ||
      final.response.kind !== "text" ||
      final.expected_tool_call_id !== initial.response.call_id ||
      JSON.stringify(initial.expected_history) !==
        JSON.stringify(final.expected_history)
    ) {
      throw new Error("duplicate_prompt_marker");
    }
  }

  const readScenarios = scenarios.filter(
    (scenario) => scenario.response.kind === "read_tool",
  );
  if (readScenarios.length > 1) {
    throw new Error("read_scenario_count");
  }
  let readRound = null;
  if (readScenarios.length === 1) {
    const initial = readScenarios[0];
    const final = scenarios.find(
      (scenario) =>
        scenario.prompt_marker === initial.prompt_marker &&
        scenario.require_tool_result,
    );
    if (!final || final.response.kind !== "text") {
      throw new Error("read_scenario_pair");
    }
    readRound = {
      prompt_marker: initial.prompt_marker,
      response_marker: final.response.content,
      call_id: initial.response.call_id,
      file_path: initial.response.file_path,
      result_marker: final.expected_tool_result_marker,
    };
  }

  const markerCatalog = [...new Set(
    scenarios.flatMap((scenario) => [
      scenario.prompt_marker,
      ...scenario.expected_history.map((entry) => entry.marker),
      ...(scenario.response.kind === "text"
        ? [scenario.response.content]
        : []),
    ]),
  )];
  if (markerCatalog.length > MAX_MARKER_CATALOG_ITEMS) {
    throw new Error("marker_catalog_count");
  }

  return {
    version: SCENARIO_VERSION,
    model: value.model,
    expected_request_count: value.expected_request_count,
    expected_probe_request_count: value.expected_probe_request_count,
    scenarios,
    read_round: readRound,
    marker_catalog: markerCatalog,
  };
}

function boundedMarkerOccurrenceCount(text, marker) {
  if (typeof text !== "string" || !isMarker(marker)) {
    return 0;
  }
  let count = 0;
  let offset = 0;
  while (offset <= text.length - marker.length) {
    const index = text.indexOf(marker, offset);
    if (index < 0) {
      break;
    }
    const before = index === 0 ? "" : text[index - 1];
    const afterIndex = index + marker.length;
    const after = afterIndex === text.length ? "" : text[afterIndex];
    if (
      (before === "" || !/[A-Za-z0-9_]/u.test(before)) &&
      (after === "" || !/[A-Za-z0-9_]/u.test(after))
    ) {
      count += 1;
      if (count === 2) {
        return count;
      }
    }
    offset = index + marker.length;
  }
  return count;
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

function readToolCall(message, contract) {
  if (message?.role !== "assistant" || !Array.isArray(message.tool_calls)) {
    return false;
  }
  if (message.tool_calls.length !== 1) {
    return false;
  }
  const call = message.tool_calls[0];
  return (
    call?.id === contract.call_id &&
    call?.type === "function" &&
    call?.function?.name === "Read" &&
    call?.function?.arguments ===
      JSON.stringify({ file_path: contract.file_path })
  );
}

function readToolResult(message, contract) {
  return (
    message?.role === "tool" &&
    message.tool_call_id === contract.call_id &&
    typeof message.content === "string" &&
    message.content.includes(contract.result_marker) &&
    byteLength(message.content) <= 64 * 1024
  );
}

function validateReadRound(request, scenario, contract) {
  if (contract === null) {
    return true;
  }
  const messages = request.messages;
  const promptIndexes = messages.flatMap((message, index) =>
    message?.role === "user" &&
    boundedMarkerOccurrenceCount(
      messageText(message),
      contract.prompt_marker,
    ) === 1
      ? [index]
      : [],
  );
  const responseIndex = messages.findIndex(
    (message) =>
      message?.role === "assistant" &&
      messageText(message) === contract.response_marker,
  );
  const toolCallIndexes = messages.flatMap((message, index) =>
    readToolCall(message, contract) ? [index] : [],
  );
  const toolResultIndexes = messages.flatMap((message, index) =>
    readToolResult(message, contract) ? [index] : [],
  );

  if (scenario.response.kind === "read_tool") {
    return (
      promptIndexes.length === 1 &&
      responseIndex === -1 &&
      toolCallIndexes.length === 0 &&
      toolResultIndexes.length === 0
    );
  }
  if (
    scenario.prompt_marker === contract.prompt_marker &&
    scenario.require_tool_result
  ) {
    return (
      promptIndexes.length === 1 &&
      responseIndex === -1 &&
      toolCallIndexes.length === 1 &&
      toolResultIndexes.length === 1 &&
      promptIndexes[0] < toolCallIndexes[0] &&
      toolCallIndexes[0] < toolResultIndexes[0]
    );
  }
  const historyContainsReadRound = scenario.expected_history.some(
    (entry) =>
      entry.role === "user" && entry.marker === contract.prompt_marker,
  );
  if (!historyContainsReadRound) {
    return (
      promptIndexes.length === 0 &&
      responseIndex === -1 &&
      toolCallIndexes.length === 0 &&
      toolResultIndexes.length === 0
    );
  }
  return (
    promptIndexes.length === 1 &&
    responseIndex >= 0 &&
    toolCallIndexes.length === 1 &&
    toolResultIndexes.length === 1 &&
    promptIndexes[0] < toolCallIndexes[0] &&
    toolCallIndexes[0] < toolResultIndexes[0] &&
    toolResultIndexes[0] < responseIndex
  );
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
    read_round_match: false,
    model_ok: false,
    stream_ok: false,
    tools_ok: false,
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

  result.tools_ok =
    Array.isArray(request.tools) &&
    request.tools.length === 1 &&
    request.tools.length <= MAX_TOOLS &&
    request.tools[0]?.function?.name === "Read";
  if (!result.tools_ok) fail(10, Array.isArray(request.tools) ? request.tools.length : 0);

  const markers = [];
  let assistantShapeMismatch = -1;
  let markerAmbiguityIndex = -1;
  for (let index = 0; index < request.messages.length; index += 1) {
    const message = request.messages[index];
    if (message?.role !== "user" && message?.role !== "assistant") {
      continue;
    }
    const text = messageText(message);
    if (text === null) {
      continue;
    }
    if (byteLength(text) > MAX_MESSAGE_TEXT_BYTES) {
      fail(14, index);
      continue;
    }
    const matches = [];
    for (const marker of document.marker_catalog) {
      const occurrenceCount = boundedMarkerOccurrenceCount(text, marker);
      if (occurrenceCount > 0) {
        matches.push(marker);
      }
      if (occurrenceCount > 1) {
        matches.push(marker);
      }
      if (matches.length > 1) {
        break;
      }
    }
    if (matches.length === 0) {
      continue;
    }
    if (matches.length !== 1) {
      markerAmbiguityIndex = index;
      continue;
    }
    const marker = matches[0];
    if (message.role === "assistant" && text !== marker) {
      assistantShapeMismatch = index;
    }
    markers.push({ role: message.role, marker, input_index: index });
  }
  if (assistantShapeMismatch >= 0) {
    fail(8, assistantShapeMismatch);
  }
  if (markerAmbiguityIndex >= 0) {
    fail(13, markerAmbiguityIndex);
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
        markers[index].marker !== expected[index].marker
      ) {
        fail(7, index);
        break;
      }
    }
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
      currentTurnTools[0].content.includes(scenario.expected_tool_result_marker) &&
      byteLength(currentTurnTools[0].content) <= 64 * 1024;
  } else {
    result.tool_result_match = currentTurnTools.length === 0;
  }
  if (!result.tool_result_match) {
    fail(9, currentTurnTools.length);
  }

  result.read_round_match = validateReadRound(
    request,
    scenario,
    document.read_round,
  );
  if (!result.read_round_match) {
    fail(11);
  }
  result.history_match = result.mismatch_code === 0;
  return result;
}

function fixedFixtureResponse(document, scenario) {
  const overrides = {
    id: `msg_${scenario.id}`,
    model: document.model,
    usage: { input_tokens: 1, output_tokens: 1 },
  };
  if (scenario.response.kind === "text") {
    return { content: scenario.response.content, ...overrides };
  }
  return {
    toolCalls: [
      {
        id: scenario.response.call_id,
        name: "Read",
        // aimock 1.39.0's Claude path requires the explicit JSON string.
        arguments: JSON.stringify({ file_path: scenario.response.file_path }),
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
      userMessage: scenario.prompt_marker,
      hasToolResult: scenario.require_tool_result,
      endpoint: "chat",
    };
    if (scenario.require_tool_result) {
      match.toolCallId = scenario.expected_tool_call_id;
      match.toolResultContains = scenario.expected_tool_result_marker;
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
    (candidate) => candidate.toLowerCase() === name.toLowerCase(),
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

function installRawRequestObserver(mock) {
  const server = mock?.serverInstance?.server;
  if (
    server === undefined ||
    typeof server.prependListener !== "function" ||
    typeof server.off !== "function"
  ) {
    throw new Error("raw_request_observer");
  }
  const counters = {
    request_count: 0,
    expected_request_count: 0,
    model_request_count: 0,
    probe_request_count: 0,
    unexpected_request_count: 0,
    wrong_method_count: 0,
    wrong_path_count: 0,
    auth_rejection_count: 0,
    authorization_header_count: 0,
    alternate_credential_header_count: 0,
    upgrade_request_count: 0,
    saturated: false,
  };
  const beginObservation = () => {
    if (counters.request_count >= MAX_RAW_REQUESTS) {
      counters.saturated = true;
      return false;
    }
    counters.request_count += 1;
    return true;
  };
  const onRequest = (request) => {
    if (!beginObservation()) {
      return;
    }
    const boundedTarget =
      typeof request.url === "string" &&
      byteLength(request.url) <= MAX_RAW_TARGET_BYTES;
    const modelTarget =
      boundedTarget && request.url === EXPECTED_MESSAGES_TARGET;
    const probeTarget =
      boundedTarget && request.url === EXPECTED_HELLO_TARGET;
    const modelRoute = request.method === "POST" && modelTarget;
    const probeRoute = request.method === "HEAD" && probeTarget;
    const apiKey = request.headers?.["x-api-key"];
    const authorization = request.headers?.authorization;
    const authorizationAbsent = authorization === undefined;
    const alternateCredentialHeaderCount = [
      "x-goog-api-key",
      "api-key",
      "xi-api-key",
      "proxy-authorization",
    ].filter((name) => request.headers?.[name] !== undefined).length;
    const modelAuthOk =
      typeof apiKey === "string" &&
      apiKey === PLACEHOLDER_API_KEY &&
      authorizationAbsent &&
      alternateCredentialHeaderCount === 0;
    const probeAuthOk =
      apiKey === undefined &&
      authorizationAbsent &&
      alternateCredentialHeaderCount === 0;
    const methodOkForTarget =
      (modelTarget && request.method === "POST") ||
      (probeTarget && request.method === "HEAD");
    if (!methodOkForTarget) counters.wrong_method_count += 1;
    if (!modelTarget && !probeTarget) counters.wrong_path_count += 1;
    if (!authorizationAbsent) counters.authorization_header_count += 1;
    counters.alternate_credential_header_count +=
      alternateCredentialHeaderCount;
    if ((modelRoute && !modelAuthOk) || (probeRoute && !probeAuthOk)) {
      counters.auth_rejection_count += 1;
    }
    if (modelRoute && modelAuthOk) {
      counters.expected_request_count += 1;
      counters.model_request_count += 1;
    } else if (probeRoute && probeAuthOk) {
      counters.expected_request_count += 1;
      counters.probe_request_count += 1;
    } else {
      counters.unexpected_request_count += 1;
    }
  };
  const onUpgrade = () => {
    if (!beginObservation()) {
      return;
    }
    counters.upgrade_request_count += 1;
    counters.unexpected_request_count += 1;
  };
  server.prependListener("request", onRequest);
  server.prependListener("upgrade", onUpgrade);
  return {
    counters,
    detach() {
      server.off("request", onRequest);
      server.off("upgrade", onUpgrade);
    },
  };
}

function buildAudit(
  document,
  entries,
  fixtureIds,
  observations,
  rawObservations,
) {
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
        : (validation?.mismatch_code ?? 12);
    return {
      id: scenario.id,
      request_count: scenarioEntries.length,
      method_ok: every((entry) => entry.method === "POST"),
      path_ok: every(
        (entry) => entry.path === EXPECTED_MESSAGES_TARGET,
      ),
      content_type_json: every((entry) => isJsonContentType(entry.headers)),
      content_encoding_absent: every(
        (entry) => headerValue(entry.headers, "content-encoding") === undefined,
      ),
      authorization_absent: every(
        (entry) => headerValue(entry.headers, "authorization") === undefined,
      ),
      x_api_key_redacted: every(
        (entry) => headerValue(entry.headers, "x-api-key") === "[REDACTED]",
      ),
      status_ok: every((entry) => entry.response.status === 200),
      model_ok: validation?.model_ok ?? false,
      stream_ok: validation?.stream_ok ?? false,
      tools_ok: validation?.tools_ok ?? false,
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
      read_round_match: validation?.read_round_match ?? false,
      mismatch_code: mismatchCode,
      mismatch_index: validation?.mismatch_index ?? 0,
    };
  });

  const allEntries = entries.length > 0;
  const rawJournalReconciled =
    rawObservations.model_request_count === entries.length;
  const expectedRawRequestCount =
    document.expected_request_count + document.expected_probe_request_count;
  const rawAuditOk =
    !rawObservations.saturated &&
    rawObservations.request_count === expectedRawRequestCount &&
    rawObservations.expected_request_count === expectedRawRequestCount &&
    rawObservations.model_request_count === document.expected_request_count &&
    rawObservations.probe_request_count ===
      document.expected_probe_request_count &&
    rawObservations.unexpected_request_count === 0 &&
    rawObservations.wrong_method_count === 0 &&
    rawObservations.wrong_path_count === 0 &&
    rawObservations.auth_rejection_count === 0 &&
    rawObservations.authorization_header_count === 0 &&
    rawObservations.alternate_credential_header_count === 0 &&
    rawObservations.upgrade_request_count === 0 &&
    rawJournalReconciled;
  return {
    version: SCENARIO_VERSION,
    expected_request_count: document.expected_request_count,
    expected_probe_request_count: document.expected_probe_request_count,
    request_count: entries.length,
    strict_mode: MOCK_CONFIGURATION.strict === true,
    loopback_only: MOCK_CONFIGURATION.host === "127.0.0.1",
    recording_absent: !Object.hasOwn(MOCK_CONFIGURATION, "record"),
    upstream_absent:
      !Object.hasOwn(MOCK_CONFIGURATION, "upstream") &&
      !Object.hasOwn(MOCK_CONFIGURATION, "record"),
    inbound_auth_required:
      MOCK_CONFIGURATION.auth?.apiKeys?.length === 1,
    fixed_placeholder_enforced:
      MOCK_CONFIGURATION.auth?.apiKeys?.[0] === PLACEHOLDER_API_KEY,
    raw_request_count: rawObservations.request_count,
    raw_expected_request_count: rawObservations.expected_request_count,
    raw_model_request_count: rawObservations.model_request_count,
    raw_probe_request_count: rawObservations.probe_request_count,
    raw_unexpected_request_count: rawObservations.unexpected_request_count,
    raw_wrong_method_count: rawObservations.wrong_method_count,
    raw_wrong_path_count: rawObservations.wrong_path_count,
    raw_auth_rejection_count: rawObservations.auth_rejection_count,
    raw_authorization_header_count:
      rawObservations.authorization_header_count,
    raw_alternate_credential_header_count:
      rawObservations.alternate_credential_header_count,
    raw_upgrade_request_count: rawObservations.upgrade_request_count,
    raw_observation_saturated: rawObservations.saturated,
    raw_journal_reconciled: rawJournalReconciled,
    raw_audit_ok: rawAuditOk,
    journal_saturated: entries.length >= MAX_JOURNAL_ENTRIES,
    post_only: allEntries && entries.every((entry) => entry.method === "POST"),
    messages_path_only:
      allEntries &&
      entries.every((entry) => entry.path === EXPECTED_MESSAGES_TARGET),
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
    x_api_key_redacted_only:
      allEntries &&
      entries.every(
        (entry) => headerValue(entry.headers, "x-api-key") === "[REDACTED]",
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
    unused_scenario_count: scenarios.filter(
      (scenario) => scenario.request_count === 0,
    ).length,
    all_scenarios_consumed_once: scenarios.every(
      (scenario) => scenario.request_count === 1,
    ),
    scenarios,
  };
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
    (process.platform !== "win32" && (metadata.mode & 0o7777) !== 0o700)
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
  sanitizeProviderEnvironment();
  const { LLMock } = await import("@copilotkit/aimock");
  const mock = new LLMock(MOCK_CONFIGURATION);
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
    const rawObserver = installRawRequestObserver(mock);
    process.stdout.write(
      `${JSON.stringify({
        type: "mallard_claude_aimock_ready",
        version: SCENARIO_VERSION,
        port: mock.port,
      })}\n`,
    );
    await waitForShutdown(process.stdin);
    const journal = mock.journal;
    await mock.stop();
    started = false;
    rawObserver.detach();
    const entries = journal.getAll();
    const audit = buildAudit(
      document,
      entries,
      fixtureIds,
      observations,
      rawObserver.counters,
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
  runHarness().catch(() => {
    process.exitCode = 1;
  });
}

export {
  MAX_SCENARIO_BYTES,
  buildAudit,
  fixedFixtureResponse,
  parseArguments,
  parseScenarioDocument,
  registerFixtures,
  runHarness,
  sanitizeProviderEnvironment,
  validateScenarioRequest,
  waitForShutdown,
};
