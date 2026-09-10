import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { makeEnvironment, runCli, textContent, toolResponse } from "./run-local-subagent-poc.mjs";

// Native format and cold-resume gate. All model requests go to a loopback mock.
const script = fileURLToPath(import.meta.url);
const repository = resolve(dirname(script), "..");
const START = "MALLARD_CLAUDE_PARENT Start the fixture child.";
const CHILD = "MALLARD_CLAUDE_CHILD Remember the fixture result.";
const RESULT = "MALLARD_CLAUDE_CHILD_RESULT_7419";
const RESUME = "MALLARD_CLAUDE_CHILD_RESUME Return the prior fixture result.";
const RESTORED = "MALLARD_CLAUDE_PARENT_RESTORED";
const PLACEHOLDER = "mallard-local-aimock-placeholder";

function check(value, code) {
  if (!value) throw new Error(code);
}

async function documents(root) {
  const found = [];
  let entries = 0;
  async function walk(path) {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      check(++entries <= 256 && !entry.isSymbolicLink(), "unsafe_fixture_tree");
      const file = join(path, entry.name);
      if (entry.isDirectory()) await walk(file);
      else if (entry.name.endsWith(".jsonl")) {
        const bytes = await readFile(file);
        check(bytes.length <= 2 * 1024 * 1024, "fixture_size_limit");
        found.push({ path: file, records: bytes.toString("utf8").trim().split("\n").map(JSON.parse) });
      }
    }
  }
  await walk(root);
  return found;
}

function args(prompt, parent) {
  return [
    "--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}',
    "--settings", '{"disableAllHooks":true}', "--print", "--output-format", "json",
    "--model", "claude-sonnet-4-6", "--permission-mode", "dontAsk",
    "--tools=Agent,TaskOutput,SendMessage", "--allowedTools=Agent,TaskOutput,SendMessage",
    "--agents", JSON.stringify({ fixture: { description: "Return a fixture result", prompt: CHILD, tools: [], model: "inherit" } }),
    "--max-turns", "8", "--no-chrome", ...(parent ? ["--resume", parent] : []), prompt,
  ];
}

async function runCase(LLMock, root, asynchronous) {
  const source = await makeEnvironment(join(root, "source"));
  const destination = await makeEnvironment(join(root, "destination"));
  let phase = "source";
  let childIds = [];
  let requests = 0;
  const resumed = new Set();
  let protocolError;
  let childRequests = 0;
  const mock = new LLMock({ host: "127.0.0.1", port: 0, strict: true, logLevel: "silent", journalMaxEntries: 96, auth: { apiKeys: [PLACEHOLDER] } });
  mock.addFixture({
    match: { predicate: () => true },
    response: async (request) => {
      try {
        check(++requests <= 96 && request.model === "claude-sonnet-4-6", "request_contract");
        const messages = request.messages;
        const users = messages.filter((message) => message.role === "user").map((message) => textContent(message.content));
        const isChild = users.some((text) => text.includes(CHILD));
        if (isChild) {
          childRequests++;
          if (phase === "destination") {
            check(messages.some((message) => message.role === "assistant" && textContent(message.content).includes(RESULT)), "child_context_missing");
            check(users.some((text) => text.includes(RESUME)), "child_resume_prompt_missing");
            const id = childIds.find((id) => users.some((text) => text.includes(`${RESUME} ${id}`)));
            check(id, "child_resume_identity_missing");
            resumed.add(id);
          }
          return { content: RESULT };
        }
        const calls = messages.filter((message) => message.role === "tool");
        if (phase === "source") {
          for (let index = 0; index < 3; index++) {
            const launch = calls.find((message) => message.tool_call_id === `fixture_spawn_${index}`);
            if (!launch) return toolResponse("Agent", { description: "Fixture child", prompt: CHILD, subagent_type: "fixture", run_in_background: asynchronous }, `fixture_spawn_${index}`);
            if (asynchronous && !calls.some((message) => message.tool_call_id === `fixture_wait_${index}`)) {
              const id = textContent(launch.content).match(/agentId["\s]*:\s*["']?([a-zA-Z0-9_-]+)/u)?.[1];
              check(id, "async_agent_id_missing");
              return toolResponse("TaskOutput", { task_id: id, block: true, timeout: 10000 }, `fixture_wait_${index}`);
            }
          }
          check(calls.some((message) => textContent(message.content).includes(RESULT)), "source_child_result_missing");
          return { content: "MALLARD_CLAUDE_PARENT_DONE" };
        }
        for (const [index, id] of childIds.entries()) {
          const reply = calls.find((message) => message.tool_call_id === `fixture_resume_${index}`);
          if (!reply) return toolResponse("SendMessage", { to: id, message: `${RESUME} ${id}` }, `fixture_resume_${index}`);
          if (!resumed.has(id)) return toolResponse("TaskOutput", { task_id: id, block: true, timeout: 10000 }, `fixture_resume_wait_${index}`);
        }
        return { content: "MALLARD_CLAUDE_RESTORED_DONE" };
      } catch (error) {
        protocolError = error.message;
        return { error: { message: "fixture mismatch", type: "invalid_request_error" }, status: 422, fallthrough: false };
      }
    },
  });
  await mock.start();
  const env = (base) => ({ ...base, CLAUDE_CODE_TMPDIR: base.TMPDIR, ANTHROPIC_BASE_URL: mock.url, ANTHROPIC_API_KEY: PLACEHOLDER, CLAUDE_CODE_MAX_RETRIES: "0", API_TIMEOUT_MS: "15000", CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK: "1" });
  try {
    const run = async (base, prompt, id) => {
      const output = await runCli("claude", args(prompt, id), env(base), join(base.HOME, "..", "project"), join(root, `${phase}-cli.log`));
      check(!protocolError, protocolError);
      const result = JSON.parse(output);
      check(!result.is_error && result.permission_denials.length === 0, "native_result_failed");
      return result;
    };
    await run(source, `${START} Parent one.`);
    await run(source, `${START} Parent two.`);
    const files = await documents(join(source.CLAUDE_CONFIG_DIR, "projects"));
    const children = files.filter((file) => file.records[0].isSidechain === true);
    const parents = files.filter((file) => file.records[0].isSidechain !== true);
    check(children.length === 6 && parents.length === 2, "native_inventory");
    const sourceBytes = new Map();
    for (const child of children) {
      const childId = child.records[0].agentId;
      const parentId = child.records[0].sessionId;
      check(child.records.every((record) => record.agentId === childId && record.sessionId === parentId && record.isSidechain === true), "child_record_identity");
      const link = parents.flatMap((parent) => parent.records).find((record) => record.toolUseResult?.agentId === childId);
      check(link && link.message.content.some((block) => block.type === "tool_result" && block.tool_use_id.startsWith("fixture_spawn_")), "single_record_link");
      sourceBytes.set(child.path, await readFile(child.path));
      if (asynchronous) {
        check(link.toolUseResult.isAsync === true && typeof link.toolUseResult.outputFile === "string", "async_shape");
        check(!link.toolUseResult.outputFile.startsWith(join(source.CLAUDE_CONFIG_DIR, "projects")), "output_is_external");
        check(link.toolUseResult.outputFile.startsWith(root + "/"), "output_outside_test_home");
        await rm(link.toolUseResult.outputFile, { force: true });
      }
    }
    const bridge = spawnSync(process.env.MALLARD_NATIVE_TEST_BINARY, ["native_claude_owned_transport_bridge", "--ignored", "--nocapture"], {
      cwd: repository, env: { ...process.env, MALLARD_CLAUDE_NATIVE_ROOT: root }, encoding: "utf8", timeout: 180000,
    });
    await writeFile(join(root, "mallard-transport.log"), `${bridge.stdout ?? ""}${bridge.stderr ?? ""}`);
    check(bridge.status === 0, "mallard_transport_failed");
    const transport = JSON.parse(await readFile(join(root, "transport.json"), "utf8"));
    check(transport.parents.length === 2, "mallard_parent_count");
    phase = "destination";
    for (const parent of transport.parents) {
      childIds = parent.children.map((child) => child.id);
      check(childIds.length === 3, "mallard_child_count");
      const sourceParent = children.find((child) => child.records[0].agentId === childIds[0])?.records[0].sessionId;
      check(sourceParent && parent.main_id !== sourceParent, "mallard_destination_identity");
      await run(destination, `${RESTORED} Resume the three fixture children.`, parent.main_id);
      for (const child of parent.children) {
        check(resumed.has(child.id), "cold_child_resume_missing");
        check((await readFile(child.path, "utf8")).includes(`${RESUME} ${child.id}`), "destination_child_not_extended");
      }
    }
    for (const [path, before] of sourceBytes) check((await readFile(path)).equals(before), "source_child_changed");
    check(childRequests >= 12, "child_requests_missing");
    return { asynchronous, parents: 2, children: 6, requests, identity_checked: true, single_record_link: true, source_output_unavailable: asynchronous, cold_resume: true };
  } catch (error) {
    throw new Error(protocolError ?? error.message);
  } finally {
    await mock.stop();
  }
}

if (process.argv[2] !== "--isolated-child") {
  const build = spawnSync("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "--no-run", "--message-format=json"], {
    cwd: repository, encoding: "utf8", timeout: 240000, maxBuffer: 16 * 1024 * 1024,
  });
  check(build.status === 0, "fixture_build_failed");
  const binary = build.stdout.split("\n").filter(Boolean).map(JSON.parse)
    .find((entry) => entry.reason === "compiler-artifact" && entry.profile.test && entry.executable)?.executable;
  check(binary, "fixture_binary_missing");
  const launch = await mkdtemp("/private/tmp/mallard-claude-native-launch-");
  try {
    const result = spawnSync(process.execPath, [script, "--isolated-child"], { cwd: repository, env: { ...await makeEnvironment(launch), MALLARD_NATIVE_TEST_BINARY: binary }, stdio: "inherit", timeout: 360000 });
    process.exitCode = result.status ?? 1;
  } finally {
    await rm(launch, { recursive: true, force: true });
  }
} else {
  const root = await mkdtemp("/private/tmp/mallard-claude-owned-native-");
  try {
    const { LLMock } = await import("@copilotkit/aimock");
    const version = (await runCli("claude", ["--version"], await makeEnvironment(join(root, "version")), root)).trim();
    const results = [];
    for (const asynchronous of [false, true]) results.push(await runCase(LLMock, join(root, asynchronous ? "async" : "sync"), asynchronous));
    console.log(JSON.stringify({ version, transport: "Mallard Push/Pull", results }, null, 2));
    await rm(root, { recursive: true, force: true });
  } catch (error) {
    console.error(JSON.stringify({ error: /^[a-z0-9_]+$/u.test(error.message) ? error.message : "native_fixture_failed", diagnostics: root }));
    process.exitCode = 1;
  }
}
