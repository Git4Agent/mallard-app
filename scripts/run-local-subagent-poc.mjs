import { spawn, spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

// Run: node scripts/run-local-subagent-poc.mjs
// All provider state is temporary. Only the result summary leaves the child.
const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = resolve(dirname(scriptPath), "..");
const PARENT_PROMPT = "MALLARD_POC_PARENT_USER Start one subagent, wait for it, and report its result.";
const CHILD_PROMPT = "MALLARD_POC_CHILD_USER Return the fixture result.";
const CHILD_RESULT = "MALLARD_POC_CHILD_OK";
const PARENT_RESULT = "MALLARD_POC_PARENT_OK";
const CONTROL_RESULT = "MALLARD_POC_CONTROL_OK";
const CODEX_MODEL = "gpt-5.4";
const PLACEHOLDER = "mallard-local-aimock-placeholder";
const MAX_BYTES = 2 * 1024 * 1024;
const MAX_REQUESTS = 8;
const TEMP_ROOT = process.platform === "darwin" ? "/private/tmp" : tmpdir();

function check(condition, code) {
  if (!condition) throw new Error(code);
}

async function makeEnvironment(root) {
  for (const name of ["home", "config", "project", "tmp"]) {
    await mkdir(join(root, name), { recursive: true, mode: 0o700 });
  }
  return {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: join(root, "home"),
    USERPROFILE: join(root, "home"),
    TMPDIR: join(root, "tmp"),
    CODEX_HOME: join(root, "config"),
    CLAUDE_CONFIG_DIR: join(root, "config"),
    NO_PROXY: "127.0.0.1",
    no_proxy: "127.0.0.1",
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    DISABLE_TELEMETRY: "1",
    DISABLE_ERROR_REPORTING: "1",
    DISABLE_UPDATES: "1",
  };
}

async function runCli(command, args, environment, cwd, diagnosticPath) {
  return await new Promise((accept, reject) => {
    const child = spawn(command, args, {
      cwd, env: environment, stdio: ["ignore", "pipe", "pipe"], detached: process.platform !== "win32",
    });
    let stdout = "";
    let stderr = "";
    let bytes = 0;
    let failure = null;
    const stop = () => {
      try { if (process.platform === "win32") child.kill("SIGKILL"); else process.kill(-child.pid, "SIGKILL"); } catch { /* Already exited. */ }
    };
    const timer = setTimeout(() => {
      failure = "cli_timeout";
      stop();
    }, 30_000);
    for (const [stream, capture] of [[child.stdout, true], [child.stderr, false]]) {
      stream.on("data", (chunk) => {
        bytes += chunk.length;
        if (bytes > MAX_BYTES) {
          failure = "cli_output_limit";
          stop();
        } else if (capture) stdout += chunk.toString("utf8");
        else if (diagnosticPath) stderr += chunk.toString("utf8");
      });
    }
    child.on("error", () => {
      clearTimeout(timer);
      reject(new Error("cli_start_failed"));
    });
    child.on("close", async (code, signal) => {
      clearTimeout(timer);
      if (failure || code !== 0 || signal !== null) {
        if (diagnosticPath) await writeFile(diagnosticPath, stdout + "\n" + stderr, { mode: 0o600 });
        reject(new Error(failure ?? `${command}_exit_${code ?? "signal"}`));
      } else accept(stdout);
    });
  });
}

async function transcripts(root) {
  const documents = [];
  let entries = 0;
  async function walk(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      check(++entries <= 256, "transcript_entry_limit");
      check(!entry.isSymbolicLink(), "transcript_symlink");
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await walk(path);
      else if (entry.isFile() && entry.name.endsWith(".jsonl")) {
        const bytes = await readFile(path);
        check(bytes.length <= MAX_BYTES, "transcript_byte_limit");
        documents.push(bytes.toString("utf8").trim().split("\n").map(JSON.parse));
      }
    }
  }
  await walk(root);
  return documents;
}

function textContent(content) {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) return content.map((part) => part.text ?? "").join("");
  return "";
}

function toolResponse(name, args, id = "poc_spawn") {
  return { toolCalls: [{ id, name, arguments: JSON.stringify(args) }] };
}

function codexArgs(url) {
  const args = [
    "--ask-for-approval", "never", "exec", "--json", "--ignore-user-config",
    "--ignore-rules", "--skip-git-repo-check", "--sandbox", "read-only",
    "--model", CODEX_MODEL,
  ];
  for (const setting of [
    'model_provider="mallard_subagent_poc"',
    'model_reasoning_effort="low"',
    "features.plugins=false",
    "features.enable_request_compression=false",
    "features.multi_agent=true",
    "features.multi_agent_v2=true",
    'web_search="disabled"',
    'model_providers.mallard_subagent_poc.name="Mallard local subagent POC"',
    `model_providers.mallard_subagent_poc.base_url="${url}/v1"`,
    'model_providers.mallard_subagent_poc.wire_api="responses"',
    "model_providers.mallard_subagent_poc.requires_openai_auth=false",
    "model_providers.mallard_subagent_poc.request_max_retries=0",
    "model_providers.mallard_subagent_poc.stream_max_retries=0",
    "model_providers.mallard_subagent_poc.stream_idle_timeout_ms=15000",
  ]) args.push("--config", setting);
  return [...args, PARENT_PROMPT];
}

function claudeArgs(bare) {
  return [
    ...(bare ? ["--bare"] : [
      "--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}',
      "--settings", '{"disableAllHooks":true}',
    ]),
    "--print", "--output-format", "json", "--model", "claude-sonnet-4-6",
    "--permission-mode", "dontAsk", "--tools=Agent", "--allowedTools=Agent",
    "--agents", JSON.stringify({
      poc: { description: "Return a test result.", prompt: "Return the fixture result.", tools: [], model: "inherit" },
    }),
    "--max-turns", "4", "--no-chrome", PARENT_PROMPT,
  ];
}

// aimock 1.39.0 drops Responses namespaces and agent_message input items.
// This small mounted endpoint tests those native fields without changing aimock.
function mountCodexResponses(mock, state) {
  let releaseChild;
  const parentWaiting = new Promise((accept) => { releaseChild = accept; });
  mock.mount("/v1/responses", {
    async handleRequest(incoming, outgoing) {
      try {
        check(incoming.method === "POST", "responses_method");
        const chunks = [];
        let bytes = 0;
        for await (const chunk of incoming) {
          bytes += chunk.length;
          check(bytes <= MAX_BYTES, "responses_byte_limit");
          chunks.push(chunk);
        }
        const raw = JSON.parse(Buffer.concat(chunks));
        check(raw.stream === true && raw.model === CODEX_MODEL, "responses_shape");
        const sequence = ++state.modelRequests;
        check(sequence <= MAX_REQUESTS, "responses_request_limit");
        const isParent = raw.input.some((item) => item.role === "user" && textContent(item.content) === PARENT_PROMPT);
        let response;
        let wakeChild = false;
        if (!isParent) {
          check(raw.input.some((item) => item.type === "agent_message" && JSON.stringify(item.content).includes(CHILD_PROMPT)), "codex_child_prompt");
          check(++state.childRequests === 1, "codex_duplicate_child");
          await parentWaiting;
          response = { content: CHILD_RESULT };
        } else {
          state.parentRequests += 1;
          const tools = raw.tools?.find((tool) => tool.type === "namespace" && tool.name === "collaboration")?.tools ?? [];
          state.agentToolExposed ||= tools.some((tool) => tool.name === "spawn_agent");
          check(state.agentToolExposed, "codex_agent_tool_missing");
          const results = raw.input.filter((item) => item.type === "function_call_output");
          const spawn = results.find((item) => item.call_id === "poc_spawn");
          const wait = results.find((item) => item.call_id === "poc_wait");
          if (!spawn) {
            response = toolResponse("spawn_agent", { task_name: "poc_child", message: CHILD_PROMPT, fork_turns: "none" });
          } else if (!wait) {
            check(JSON.parse(spawn.output).task_name === "/root/poc_child", "codex_spawn_result");
            response = toolResponse("wait_agent", { timeout_ms: 10000 }, "poc_wait");
            wakeChild = true;
          } else {
            check(JSON.parse(wait.output).timed_out === false, "codex_wait_timeout");
            state.childResultReceived = raw.input.some((item) => item.type === "agent_message" && JSON.stringify(item.content).includes(CHILD_RESULT));
            check(state.childResultReceived, "codex_child_result_missing");
            response = { content: PARENT_RESULT };
          }
        }
        sendResponses(outgoing, response, sequence);
        if (wakeChild) releaseChild();
      } catch (error) {
        state.protocolFailure = true;
        state.failureCode ??= error.message;
        releaseChild();
        outgoing.writeHead(422, { "content-type": "application/json" });
        outgoing.end('{"error":{"message":"POC request mismatch"}}');
      }
      return true;
    },
  });
  return releaseChild;
}

function sendResponses(outgoing, response, sequence) {
  const call = response.toolCalls?.[0];
  const item = call ? {
    type: "function_call", id: `fc_${sequence}`, call_id: call.id,
    namespace: "collaboration", name: call.name, arguments: call.arguments, status: "completed",
  } : {
    type: "message", id: `msg_${sequence}`, role: "assistant", phase: "final_answer",
    status: "completed", content: [{ type: "output_text", text: response.content, annotations: [] }],
  };
  const envelope = { id: `resp_${sequence}`, object: "response", model: CODEX_MODEL };
  outgoing.writeHead(200, { "content-type": "text/event-stream" });
  for (const event of [
    { type: "response.created", response: { ...envelope, status: "in_progress", output: [] } },
    { type: "response.output_item.added", output_index: 0, item },
    { type: "response.output_item.done", output_index: 0, item },
    { type: "response.completed", response: { ...envelope, status: "completed", output: [item], usage: { input_tokens: 0, output_tokens: 0, total_tokens: 0 } } },
  ]) outgoing.write(`event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`);
  outgoing.end();
}

async function runCase(LLMock, root, provider, mode) {
  const environment = await makeEnvironment(root);
  const state = {
    parentRequests: 0, childRequests: 0, modelRequests: 0, probeRequests: 0,
    agentToolExposed: false, childResultReceived: false, protocolFailure: false,
    unsupportedCall: false, rawRequests: 0,
  };
  const mock = new LLMock({
    host: "127.0.0.1", port: 0, strict: true, logLevel: "silent", journalMaxEntries: MAX_REQUESTS,
    ...(provider === "claude" ? { auth: { apiKeys: [PLACEHOLDER] } } : {}),
  });
  const bare = mode === "bare_control";
  const native = mode === "native_responses";
  let releaseChild = () => {};
  if (native) releaseChild = mountCodexResponses(mock, state);
  else mock.addFixture({
    match: { predicate: () => true },
    response: (request) => {
      try {
        check(++state.modelRequests <= MAX_REQUESTS, "fixture_request_limit");
        check(request.stream === true, "fixture_not_streaming");
        check(request.model === (provider === "codex" ? CODEX_MODEL : "claude-sonnet-4-6"), "fixture_model");
        const messages = request.messages;
        const child = messages.some((message) => message.role === "user" && textContent(message.content).includes(CHILD_PROMPT));
        if (child) {
          check(!bare && ++state.childRequests === 1, "unexpected_child_request");
          return { content: CHILD_RESULT };
        }
        check(messages.some((message) => message.role === "user" && textContent(message.content).includes(PARENT_PROMPT)), "fixture_parent_prompt");
        state.parentRequests += 1;
        state.agentToolExposed ||= request.tools?.some((tool) => tool.function.name === "Agent") ?? false;
        if (bare) {
          check(!state.agentToolExposed, "bare_now_supports_agents");
          return { content: CONTROL_RESULT };
        }
        const result = messages.find((message) => message.role === "tool" && message.tool_call_id === "poc_spawn");
        if (!result) {
          return provider === "codex"
            ? toolResponse("spawn_agent", { task_name: "poc_child", message: CHILD_PROMPT, fork_turns: "none" })
            : toolResponse("Agent", { description: "Return a test result", prompt: CHILD_PROMPT, subagent_type: "poc", run_in_background: false });
        }
        if (provider === "codex") {
          state.unsupportedCall = textContent(result.content) === "unsupported call: spawn_agent";
          check(state.unsupportedCall, "stock_control_changed");
          return { content: CONTROL_RESULT };
        }
        state.childResultReceived = textContent(result.content).includes(CHILD_RESULT);
        check(state.childResultReceived, "claude_child_result_missing");
        return { content: PARENT_RESULT };
      } catch (error) {
        state.protocolFailure = true;
        state.failureCode ??= error.message;
        return { error: { message: "POC request mismatch", type: "invalid_request_error" }, status: 422, fallthrough: false };
      }
    },
  });
  try {
    await mock.start();
    // This pinned aimock version exposes the same server hook used by the repo's
    // Claude audit. Keep only route/header checks; never keep bodies or headers.
    mock.serverInstance.server.prependListener("request", (request, response) => {
      state.rawRequests += 1;
      const probe = provider === "claude" && request.method === "HEAD" && request.url === "/api/hello";
      const modelRoute = provider === "codex" ? "/v1/responses" : "/v1/messages?beta=true";
      const expected = probe || (request.method === "POST" && request.url === modelRoute);
      const authOk = !request.headers.authorization && (probe || provider === "codex"
        ? !request.headers["x-api-key"] : request.headers["x-api-key"] === PLACEHOLDER);
      if (probe) state.probeRequests += 1;
      if (!expected || !authOk || state.rawRequests > MAX_REQUESTS) state.protocolFailure = true;
      response.on("finish", () => {
        if (!probe && response.statusCode !== 200) state.protocolFailure = true;
      });
    });
    if (provider === "claude") Object.assign(environment, {
      ANTHROPIC_BASE_URL: mock.url, ANTHROPIC_API_KEY: PLACEHOLDER,
      CLAUDE_CODE_MAX_RETRIES: "0", CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK: "1",
      API_TIMEOUT_MS: "15000",
    });
    const stdout = await runCli(provider, provider === "codex" ? codexArgs(mock.url) : claudeArgs(bare), environment, join(root, "project")).catch((error) => {
      throw new Error(state.failureCode ?? error.message);
    });
    check(!state.protocolFailure, "protocol_audit_failed");
    check(state.rawRequests === state.modelRequests + state.probeRequests, "request_accounting_failed");
    const expectedRequests = native ? 4 : bare ? 1 : provider === "codex" ? 2 : 3;
    check(state.modelRequests === expectedRequests, "model_request_count");
    const supported = native || (provider === "claude" && !bare);
    check(state.childRequests === (supported ? 1 : 0), "child_request_count");
    if (provider === "claude") {
      const result = JSON.parse(stdout);
      check(!result.is_error && result.permission_denials.length === 0, "claude_result_failed");
      check(result.result === (supported ? PARENT_RESULT : CONTROL_RESULT), "claude_final_result");
      check(result.subagent_stats.spawned === (supported ? 1 : 0) && result.subagent_stats.completed === (supported ? 1 : 0), "claude_subagent_stats");
    } else {
      const events = stdout.trim().split("\n").map(JSON.parse);
      check(events.some((event) => event.type === "turn.completed"), "codex_turn_incomplete");
      check(events.some((event) => event.type === "item.completed" && event.item?.type === "agent_message" && event.item.text === (supported ? PARENT_RESULT : CONTROL_RESULT)), "codex_final_result");
    }
    const docs = await transcripts(join(root, "config", provider === "codex" ? "sessions" : "projects"));
    check(docs.length === (supported ? 2 : 1), "transcript_count");
    let linked = false;
    if (supported && provider === "codex") {
      const parent = docs.find((records) => records[0].payload?.parent_thread_id === undefined);
      const child = docs.find((records) => records[0].payload?.parent_thread_id !== undefined);
      linked = child?.[0].payload.parent_thread_id === parent?.[0].payload.id && !!child?.[0].payload.source?.subagent;
      check(child?.some((record) => record.type === "event_msg" && record.payload?.type === "task_complete" && record.payload.last_agent_message === CHILD_RESULT), "codex_child_transcript_incomplete");
    } else if (supported) {
      const child = docs.find((records) => records[0].isSidechain === true);
      const parent = docs.find((records) => records[0].isSidechain !== true);
      linked = !!child?.[0].agentId && parent?.some((record) => record.toolUseResult?.agentId === child[0].agentId);
      check(child?.some((record) => record.type === "assistant" && textContent(record.message?.content) === CHILD_RESULT), "claude_child_transcript_incomplete");
    }
    if (supported) check(linked && state.childResultReceived, "parent_child_link_failed");
    return {
      provider, mode, subagent_completed: supported, model_requests: state.modelRequests,
      hello_probes: state.probeRequests, child_requests: state.childRequests,
      transcript_files: docs.length, parent_child_link_verified: linked,
      result_returned_to_parent: state.childResultReceived,
      ...(bare ? { agent_tool_exposed: state.agentToolExposed } : {}),
      ...(provider === "codex" && !native ? { missing_namespace_rejected: state.unsupportedCall } : {}),
    };
  } finally {
    releaseChild();
    await mock.stop();
  }
}

async function main() {
  const packageDocument = JSON.parse(await readFile(join(projectRoot, "node_modules/@copilotkit/aimock/package.json"), "utf8"));
  check(packageDocument.version === "1.39.0", "aimock_version_expected_1_39_0");
  const root = await mkdtemp(join(TEMP_ROOT, "mallard-subagent-poc-"));
  try {
    const environment = await makeEnvironment(join(root, "preflight"));
    const versions = {};
    for (const [command, expected] of [["codex", "codex-cli 0.152.0"], ["claude", "2.1.266 (Claude Code)"]]) {
      const output = await runCli(command, ["--version"], environment, root);
      check(output.trim() === expected, `${command}_version_mismatch`);
      versions[command] = expected;
    }
    const { LLMock } = await import("@copilotkit/aimock");
    const results = [];
    for (const [provider, mode] of [
      ["claude", "bare_control"], ["claude", "stock_aimock"],
      ["codex", "stock_aimock_control"], ["codex", "native_responses"],
    ]) {
      try {
        results.push(await runCase(LLMock, join(root, `${provider}-${mode}`), provider, mode));
      } catch (error) {
        throw new Error(`${provider}_${mode}_${error.message}`);
      }
    }
    console.log(JSON.stringify({ versions: { ...versions, aimock: packageDocument.version, node: process.versions.node }, checks_passed: true, results }, null, 2));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

export { makeEnvironment, runCli, transcripts, codexArgs, sendResponses, toolResponse, textContent };

if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
// Strip inherited credentials, provider endpoints, homes, and proxy settings
// before importing aimock or launching a provider CLI.
if (process.argv[2] !== "--isolated-child") {
  check(process.argv.length === 2, "unexpected_arguments");
  const root = await mkdtemp(join(TEMP_ROOT, "mallard-subagent-launch-"));
  try {
    const result = spawnSync(process.execPath, [scriptPath, "--isolated-child"], {
      cwd: root, env: await makeEnvironment(root), stdio: "inherit", timeout: 120_000,
    });
    process.exitCode = result.status ?? 1;
  } finally {
    await rm(root, { recursive: true, force: true });
  }
} else {
  try { await main(); } catch (error) {
    // Error codes are fixed strings. Never print CLI output or captured state.
    console.error(`Subagent POC failed: ${/^[a-z0-9_]+$/u.test(error.message) ? error.message : "unexpected_error"}`);
    process.exitCode = 1;
  }
}

}
