// Opt-in native test. All provider and Electron state uses a temporary home.
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, mkdir, readFile, writeFile, open, realpath } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { LLMock } from "@copilotkit/aimock";

if (process.platform !== "darwin") throw new Error("This native test requires macOS.");
const repository = dirname(dirname(fileURLToPath(import.meta.url)));
const root = await mkdtemp("/private/tmp/mallard-mac-placement-");
const resources = "/Applications/ChatGPT.app/Contents/Resources";
const executable = "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT";
const cli = join(resources, "codex-cli/bin/codex");
const state = { marker: "mallard-mac-placement-v1", root,
  home: join(root, "selected-home"), userData: join(root, "desktop-data"),
  project: join(root, "Placement fixture"), projectId: randomUUID() };
const report = { scope: "macOS production native project placement", passed: false, checks: [] };
const run = promisify(execFile);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const cleanEnv = () => Object.fromEntries(Object.entries(process.env).filter(([key]) =>
  !/^(CODEX_|OPENAI_|ANTHROPIC_|CLAUDE_|CHATGPT_|ELECTRON_)/i.test(key)));
const pass = (name) => { report.checks.push(name); console.log(`PASS ${name}`); };
let core;
let desktop;
let mock;

function rpc(child) {
  let next = 1, buffer = "";
  const waiting = new Map(), events = [];
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    let end;
    while ((end = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
      if (!line.trim()) continue;
      const value = JSON.parse(line), pending = waiting.get(value.id);
      if (value.method) events.push(value);
      if (pending) {
        waiting.delete(value.id); clearTimeout(pending.timer);
        value.error ? pending.reject(new Error(JSON.stringify(value.error))) : pending.resolve(value.result);
      }
    }
  });
  return { events, request: (method, params) => new Promise((resolve, reject) => {
    const id = next++;
    const timer = setTimeout(() => { waiting.delete(id); reject(new Error(`${method} timeout`)); }, 25_000);
    waiting.set(id, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ id, method, params }) + "\n");
  }) };
}

async function processes() {
  const { stdout } = await run("/bin/ps", ["-axo", "pid=,ppid=,lstart=,command="]);
  return stdout.trim().split("\n").map((line) => {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(.{24})\s+(.*)$/);
    return match ? { pid: Number(match[1]), parent: Number(match[2]),
      identity: match[3] + match[4], command: match[4] } : null;
  }).filter(Boolean);
}

async function stopDesktop() {
  if (!desktop) return;
  const all = await processes(), main = all.find((p) => p.pid === desktop.pid);
  if (main) {
    assert.ok(main.command.includes(executable) && main.command.includes(`--user-data-dir=${state.userData}`));
    const ids = new Set([main.pid]);
    for (let depth = 0; depth < 8; depth++) for (const p of all) if (ids.has(p.parent)) ids.add(p.pid);
    const recorded = all.filter((p) => ids.has(p.pid));
    desktop.kill("SIGTERM"); await sleep(1500);
    for (const p of await processes()) {
      if (recorded.some((old) => old.pid === p.pid && old.identity === p.identity)) {
        try { process.kill(p.pid, "SIGTERM"); } catch {}
      }
    }
    await sleep(500);
    report.fixtureProcessesRemaining = (await processes()).filter((p) =>
      recorded.some((old) => old.pid === p.pid && old.identity === p.identity)).map((p) => p.pid);
    assert.deepEqual(report.fixtureProcessesRemaining, []);
  }
  desktop = undefined;
}

async function launchDesktop() {
  const log = await open(join(root, "desktop.log"), "w");
  desktop = spawn(executable, [`--user-data-dir=${state.userData}`, `codex://threads/${state.threadId}`], {
    env: { ...cleanEnv(), CODEX_HOME: state.home, CODEX_ELECTRON_USER_DATA_PATH: state.userData },
    cwd: state.project, stdio: ["ignore", log.fd, log.fd],
  });
  await log.close(); state.desktopPid = desktop.pid;
  await writeFile(join(root, "state.json"), JSON.stringify(state, null, 2));
  for (let attempt = 0; attempt < 180; attempt++) {
    const output = await readFile(join(root, "desktop.log"), "utf8");
    if (output.includes("vmEvent=thread_resumed") && output.includes(`threadId=${state.threadId}`)) return;
    await sleep(250);
  }
  throw new Error("Isolated Desktop did not load its fixture chat.");
}

try {
  for (const directory of [state.home, state.userData, state.project]) await mkdir(directory);
  await writeFile(join(state.project, "README.md"), "Temporary native project placement fixture.\n");
  await writeFile(join(state.home, "auth.json"), JSON.stringify({ OPENAI_API_KEY: "fixture-local-only-not-a-credential" }));
  await writeFile(join(state.home, "config.toml"), `# Retain this fixture comment.
model = "gpt-5.4"
model_provider = "placement_fixture"
[model_providers.placement_fixture]
name = "Local fixture"
base_url = "http://127.0.0.1:1/v1"
wire_api = "responses"
requires_openai_auth = false
[analytics]
enabled = false
`);
  // Build before starting Desktop. No test uses the user's provider home.
  const build = await run("cargo", ["test", "-p", "mallard-infrastructure", "--lib", "--no-run", "--message-format=json"],
    { cwd: repository, env: { ...process.env, CODEX_HOME: state.home }, timeout: 300_000, maxBuffer: 16 * 1024 * 1024 });
  await writeFile(join(root, "build.log"), build.stderr);
  const binary = build.stdout.split("\n").filter(Boolean).map(JSON.parse).find((item) =>
    item.reason === "compiler-artifact" && item.target?.name === "mallard_infrastructure" && item.executable)?.executable;
  assert.ok(binary);
  pass("production_test_binary_built");

  mock = new LLMock({ host: "127.0.0.1", port: 0, strict: true, logLevel: "silent" });
  mock.addFixture({ match: { userMessage: /Mallard fixture:/u }, response: { content: "Local fixture completed." } });
  const url = await mock.start(), coreLog = await open(join(root, "core.log"), "w");
  core = spawn(cli, ["app-server", "--stdio"], { env: { ...cleanEnv(), CODEX_HOME: state.home },
    cwd: state.project, stdio: ["pipe", "pipe", coreLog.fd] });
  await coreLog.close();
  const backend = rpc(core);
  const initialized = await backend.request("initialize", { clientInfo: { name: "mallard-placement-fixture", version: "1.0.0" },
    capabilities: { experimentalApi: true } });
  assert.equal(await realpath(initialized.codexHome), state.home);
  core.stdin.write(JSON.stringify({ method: "initialized" }) + "\n");
  const created = await backend.request("thread/start", { cwd: state.project, ephemeral: false,
    modelProvider: "placement_fixture", model: "gpt-5.4", approvalPolicy: "never", sandbox: "read-only",
    config: { "model_providers.placement_fixture.base_url": `${url}/v1` } });
  state.threadId = created.thread.id; state.threadPath = created.thread.path;
  assert.ok(state.threadPath.startsWith(join(state.home, "sessions/")));
  await backend.request("thread/name/set", { threadId: state.threadId, name: "Mallard macOS placement fixture" });
  await backend.request("turn/start", { threadId: state.threadId,
    input: [{ type: "text", text: "Mallard fixture: preserve this local chat for project placement.", text_elements: [] }] });
  let completed;
  for (let attempt = 0; attempt < 200; attempt++) {
    completed = backend.events.find((event) => event.method === "turn/completed");
    if (completed) break;
    await sleep(100);
  }
  assert.equal(completed?.params.turn.status, "completed");
  core.kill(); core = undefined; await sleep(500); await mock.stop(); mock = undefined;
  const history = await readFile(state.threadPath);
  pass("completed_local_mock_chat_created");
  // Cold fixture seed only. All later assignments use the native operation.
  await writeFile(join(state.home, ".codex-global-state.json"), JSON.stringify({
    "local-projects": { [state.projectId]: { id: state.projectId, name: "Placement fixture",
      rootPaths: [state.project], createdAt: Date.now(), updatedAt: Date.now() } },
    "projectless-thread-ids": [state.threadId],
  }));
  await writeFile(join(root, "state.json"), JSON.stringify(state, null, 2));
  const testArgs = ["--ignored", "--exact",
    "providers::codex::actions::tests::macos_native_project_placement_uses_the_selected_home", "--nocapture"];
  // Match macOS Open in Codex: register first, then `open -n` starts Desktop.
  // An already loaded chat need not reload a newly registered MCP server.
  const prepared = await run(binary, testArgs, { cwd: repository,
    env: { ...cleanEnv(), CODEX_HOME: state.home, MALLARD_MAC_NATIVE_PLACEMENT_ROOT: root,
      MALLARD_MAC_NATIVE_PLACEMENT_PREPARE_ONLY: "1" }, timeout: 60_000, maxBuffer: 1024 * 1024 });
  assert.match(prepared.stdout, /test result: ok\. 1 passed/);
  pass("production_helper_registered_before_desktop_start");
  for (let attempt = 0; attempt < 2; attempt++) {
    await launchDesktop();
    pass(attempt ? "isolated_desktop_restarted" : "isolated_desktop_loaded_registered_helper");
    const result = await run(binary, testArgs, {
      cwd: repository, env: { ...cleanEnv(), CODEX_HOME: state.home, MALLARD_MAC_NATIVE_PLACEMENT_ROOT: root,
        MALLARD_MAC_NATIVE_PLACEMENT_EXPECT_PROJECTLESS: attempt === 0 ? "1" : "0" },
      timeout: 120_000, maxBuffer: 1024 * 1024,
    });
    await writeFile(join(root, `native-test-${attempt}.log`), result.stdout + result.stderr);
    assert.match(result.stdout, /test result: ok\. 1 passed/);
    pass(attempt ? "native_membership_and_repeat_open_after_restart" : "production_registration_move_navigation_and_membership");
    await stopDesktop();
  }
  const after = await readFile(state.threadPath);
  assert.ok(after.subarray(0, history.length).equals(history));
  const appended = after.subarray(history.length).toString().trim().split("\n").filter(Boolean).map(JSON.parse);
  assert.ok(appended.every((item) => item.type === "event_msg" && item.payload.type === "thread_settings_applied"));
  pass("same_chat_and_complete_history_preserved");
  assert.ok((await readFile(join(state.home, "config.toml"), "utf8")).includes("# Retain this fixture comment."));
  pass("existing_configuration_preserved");
  report.passed = true;
} catch (error) {
  report.error = error.message;
  await writeFile(join(root, "failure.log"), `${error.stack}\n${error.stdout ?? ""}\n${error.stderr ?? ""}`);
  process.exitCode = 1;
} finally {
  core?.kill();
  if (mock) await mock.stop();
  try { await stopDesktop(); } catch (error) { report.cleanupError = error.message; process.exitCode = 1; }
  await writeFile(join(root, "report.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ report: join(root, "report.json"), passed: report.passed }));
}
