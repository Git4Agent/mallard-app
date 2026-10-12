// Opt-in Windows POC. Every write and chat belongs to a fresh temporary profile.
// --phase=integration exercises the Rust production placement code.
// --phase=registration-owned proves automatic registration with Desktop's own backend.
// --phase=registration-integration tests automatic registration through production Rust.
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, mkdir, readFile, writeFile, open, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";
import { randomUUID } from "node:crypto";
import assert from "node:assert/strict";

const run = promisify(execFile);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const psQuote = (s) => `'${s.replaceAll("'", "''")}'`;
const ps = join(process.env.SystemRoot, "System32/WindowsPowerShell/v1.0/powershell.exe");
const args = Object.fromEntries(process.argv.slice(2).map((arg) => {
  const p = arg.indexOf("=");
  if (!arg.startsWith("--") || p < 0) throw new Error("Use --name=value arguments");
  return [arg.slice(2, p), arg.slice(p + 1)];
}));

function cleanEnv() {
  return Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    !/^(CODEX_|OPENAI_|ANTHROPIC_|CLAUDE_|CHATGPT_|PSModulePath$)/i.test(key)));
}

async function powershell(code) {
  const wrapped = `$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue';
    try { ${code} } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }`;
  try {
    const result = await run(ps, ["-NoProfile", "-NonInteractive", "-EncodedCommand",
      Buffer.from(wrapped, "utf16le").toString("base64")], {
      env: cleanEnv(), windowsHide: true, timeout: 20_000, maxBuffer: 1024 * 1024,
    });
    return result.stdout.trim();
  } catch (error) {
    throw new Error(error.killed ? "POC PowerShell command timed out" :
      String(error.stderr || "POC PowerShell command failed").trim().slice(0, 4096));
  }
}

async function save(state) {
  await writeFile(join(state.root, "poc-state.json"), JSON.stringify(state, null, 2));
}

async function loadState() {
  const root = resolve(args.root ?? "");
  if (!root.startsWith(join(tmpdir(), "mallard-native-placement-"))) {
    throw new Error("Only this POC's temporary directories are accepted");
  }
  const state = JSON.parse(await readFile(join(root, "poc-state.json"), "utf8"));
  if (state.root !== root || state.marker !== "mallard-native-placement-v1") {
    throw new Error("POC identity mismatch");
  }
  if (state.home !== join(root, "selected-home") || state.userData !== join(root, "desktop-data") ||
      state.project !== join(root, "Placement fixture")) throw new Error("POC directory mismatch");
  return state;
}

async function connect(state) {
  const socket = new WebSocket(state.wsUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", () => reject(new Error("backend_connection_failed")), { once: true });
  });
  let next = 1;
  const waiting = new Map();
  const notifications = [];
  const listeners = [];
  socket.addEventListener("message", ({ data }) => {
    const value = JSON.parse(data);
    if (value.method) {
      notifications.push(value);
      for (const listener of listeners) listener(value);
    }
    const pending = waiting.get(value.id);
    if (pending) {
      waiting.delete(value.id);
      clearTimeout(pending.timer);
      value.error ? pending.reject(new Error(JSON.stringify(value.error))) : pending.resolve(value.result);
    }
  });
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = next++;
    const timer = setTimeout(() => { waiting.delete(id); reject(new Error(`${method}: timeout`)); }, 20_000);
    waiting.set(id, { resolve, reject, timer });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const initialized = await request("initialize", {
    clientInfo: { name: "mallard-placement-poc", version: "0.1.0" },
    capabilities: { experimentalApi: true },
  });
  socket.send(JSON.stringify({ method: "initialized" }));
  if (resolve(initialized.codexHome).toLowerCase() !== resolve(state.home).toLowerCase()) {
    socket.close(); throw new Error("Backend selected-home mismatch");
  }
  return { request, close: () => socket.close(), initialized, notifications,
    until: (method) => new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`${method}: timeout`)), 30_000);
      listeners.push((value) => { if (value.method === method) { clearTimeout(timer); resolve(value.params); } });
    }) };
}

async function launch(state, url) {
  const receipt = join(state.root, `launch-${randomUUID()}.json`);
  const env = { CODEX_HOME: state.home, CODEX_ELECTRON_USER_DATA_PATH: state.userData,
    CODEX_CLI_PATH: state.cli };
  if (!state.desktopOwnsBackend) env.CODEX_APP_SERVER_WS_URL = state.wsUrl;
  const argv = [`--user-data-dir=${state.userData}`, url].filter(Boolean);
  const winQuote = (arg) => '"' + arg.replace(/(\\*)"/g, '$1$1\\"').replace(/(\\+)$/, '$1$1') + '"';
  const child = `$ErrorActionPreference='Stop'; try {
    $p=New-Object System.Diagnostics.ProcessStartInfo;
    $p.UseShellExecute=$false; $p.CreateNoWindow=$true;
    $p.FileName=${psQuote(state.executable)}; $p.Arguments=${psQuote(argv.map(winQuote).join(" "))};
    foreach($key in @($p.EnvironmentVariables.Keys)) { if ($key -match '^(CODEX_|OPENAI_|ANTHROPIC_|CLAUDE_|CHATGPT_)') { $p.EnvironmentVariables.Remove($key) } }
    ${Object.entries(env).map(([key, value]) => `$p.EnvironmentVariables[${psQuote(key)}]=${psQuote(value)};`).join("\n")}
    $child=[System.Diagnostics.Process]::Start($p);
    @{pid=$child.Id}|ConvertTo-Json -Compress|Set-Content -LiteralPath ${psQuote(receipt)} -Encoding UTF8
  } catch { @{error=$_.Exception.Message}|ConvertTo-Json -Compress|Set-Content -LiteralPath ${psQuote(receipt)} -Encoding UTF8 }`;
  await powershell(`$ErrorActionPreference='Stop'; Invoke-CommandInDesktopPackage -PackageFamilyName ${psQuote(state.family)} -AppId 'App' -Command ${psQuote(ps)} -Args ${psQuote("-NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand " + Buffer.from(child, "utf16le").toString("base64"))}`);
  for (let n = 0; n < 80; n++) {
    try {
      const result = JSON.parse((await readFile(receipt, "utf8")).replace(/^\uFEFF/, ""));
      if (result.error) throw new Error(result.error);
      return result.pid;
    } catch (error) { if (error.code !== "ENOENT") throw error; }
    await sleep(250);
  }
  throw new Error("Desktop launch receipt timeout");
}

async function findPipe(state) {
  const verifyBackend = state.desktopOwnsBackend
    ? `& { ${await readFile(new URL("./poc-windows-profile-owner.ps1", import.meta.url), "utf8")} } -DesktopProcessId ${state.desktopPid} -SelectedHome ${psQuote(state.home)} | Out-Null`
    : `$outgoing=@(Get-NetTCPConnection -OwningProcess ${state.desktopPid} -RemotePort ${new URL(state.wsUrl).port} -State Established -ErrorAction SilentlyContinue);
       $incoming=@(Get-NetTCPConnection -OwningProcess ${state.backendPid} -LocalPort ${new URL(state.wsUrl).port} -State Established -ErrorAction SilentlyContinue);
       if (!($outgoing | Where-Object { $incoming.RemotePort -contains $_.LocalPort })) { throw 'Desktop is not connected to the verified selected-home backend' }`;
  const result = await powershell(`
    $ErrorActionPreference='Stop';
    $desktop=Get-CimInstance Win32_Process -Filter "ProcessId=${state.desktopPid}";
    if (!$desktop -or !$desktop.CommandLine.Contains(${psQuote(state.userData)})) { throw 'Isolated Desktop process identity mismatch' }
    ${verifyBackend}
    Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; using Microsoft.Win32.SafeHandles; public class PocPipe { [DllImport("kernel32.dll", SetLastError=true)] public static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe, out uint pid); }';
    $matches=@();
    foreach($path in [IO.Directory]::GetFiles('\\\\.\\pipe\\','codex-browser-use*')) {
      $name=$path.Substring(9); $stream=$null;
      try { $stream=New-Object IO.Pipes.NamedPipeClientStream('.', $name, [IO.Pipes.PipeDirection]::InOut); $stream.Connect(200);
        [uint32]$serverPid=0; if([PocPipe]::GetNamedPipeServerProcessId($stream.SafePipeHandle,[ref]$serverPid) -and $serverPid -eq ${state.desktopPid}) { $matches += $path }
      } catch {} finally { if($stream) { $stream.Dispose() } }
    }; ConvertTo-Json -InputObject @($matches) -Compress`);
  // Windows enumerates every instance of a busy named pipe, including repeats.
  const paths = [...new Set(JSON.parse(result))];
  if (paths.length === 0) throw new Error("No native endpoint owned by the isolated Desktop");
  // Opening another window can add endpoints inside the same verified process.
  // They share the profile; never select an endpoint owned by a different PID.
  return paths[0];
}

async function appTools(state, requestedHome = args["codex-home"] ?? state.home) {
  const selectedHome = await realpath(requestedHome);
  if (selectedHome.toLowerCase() !== (await realpath(state.home)).toLowerCase()) {
    throw new Error("Selected CODEX_HOME does not match the isolated Desktop; no native request sent");
  }
  if (!state.desktopOwnsBackend) {
    const backend = await connect(state);
    backend.close();
  }
  const pipe = await findPipe(state);
  const child = spawn(process.execPath, [state.connector], {
    env: { ...cleanEnv(), CODEX_HOME: state.home, CODEX_APP_TOOLS_PIPE_PATH: pipe },
    windowsHide: true, stdio: ["pipe", "pipe", "pipe"],
  });
  let next = 1, buffer = "", bytes = 0;
  const waiting = new Map();
  child.stdout.on("data", (chunk) => {
    bytes += chunk.length;
    if (bytes > 8 * 1024 * 1024) { child.kill(); return; }
    buffer += chunk;
    let p;
    while ((p = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, p); buffer = buffer.slice(p + 1);
      if (!line.trim()) continue;
      const value = JSON.parse(line), pending = waiting.get(value.id);
      if (!pending) continue;
      waiting.delete(value.id); clearTimeout(pending.timer);
      value.error ? pending.reject(new Error(JSON.stringify(value.error))) : pending.resolve(value.result);
    }
  });
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = next++;
    const timer = setTimeout(() => { waiting.delete(id); reject(new Error(`${method}: timeout`)); }, 30_000);
    waiting.set(id, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  });
  await request("initialize", { protocolVersion: "2025-11-25", capabilities: {},
    clientInfo: { name: "mallard-native-placement-poc", version: "0.1.0" } });
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
  // As in Mallard's production connection, discovery initializes Desktop's
  // dispatcher. An MCP initialize reply alone does not establish readiness.
  try {
    const catalog = await request("tools/list", {});
    for (const name of ["list_projects", "list_threads", "read_thread", "move_thread_to_project", "navigate_to_codex_page"]) {
      assert.ok(catalog.tools?.some((tool) => tool.name === name), `Required native tool missing: ${name}`);
    }
  } catch (error) { child.kill(); throw error; }
  return {
    call: (name, input) => request("tools/call", { name, arguments: input,
      _meta: { "openai/threadId": state.threadId } }),
    close: () => child.kill(),
  };
}

async function setup({ seedProject = true, launchDesktop = true } = {}) {
  for (const key of ["cli", "executable", "connector", "family"]) if (!args[key]) throw new Error(`Missing --${key}`);
  const root = await mkdtemp(join(tmpdir(), "mallard-native-placement-"));
  const state = { marker: "mallard-native-placement-v1", root, cli: resolve(args.cli),
    executable: resolve(args.executable), connector: resolve(args.connector), family: args.family,
    home: join(root, "selected-home"), userData: join(root, "desktop-data"), project: join(root, "Placement fixture") };
  for (const dir of [state.home, state.userData, state.project]) await mkdir(dir);
  await writeFile(join(state.project, "README.md"), "# Mallard native placement fixture\n");
  // The original placement probe seeds an existing project. Registration
  // probes leave this file entirely to Desktop, including the initial project.
  if (seedProject) {
    state.fixtureProjectId = randomUUID();
    await writeFile(join(state.home, ".codex-global-state.json"), JSON.stringify({
      "local-projects": { [state.fixtureProjectId]: { id: state.fixtureProjectId,
        name: "Placement fixture", rootPaths: [state.project], createdAt: Date.now(), updatedAt: Date.now() } },
    }));
  }
  await writeFile(join(state.home, "auth.json"), JSON.stringify({ OPENAI_API_KEY: "poc-local-only-not-a-credential" }));
  await writeFile(join(state.home, "config.toml"), `model = "gpt-5.4"\nmodel_provider = "placement_poc"\n[model_providers.placement_poc]\nname = "Local POC (no inference)"\nbase_url = "http://127.0.0.1:1/v1"\nwire_api = "responses"\nrequires_openai_auth = false\n[analytics]\nenabled = false\n`);
  const server = createServer(); await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port; await new Promise((resolve) => server.close(resolve));
  state.wsUrl = `ws://127.0.0.1:${port}`;
  const output = await open(join(root, "backend.log"), "w");
  const backend = spawn(state.cli, ["app-server", "--listen", state.wsUrl], {
    env: { ...cleanEnv(), CODEX_HOME: state.home }, cwd: state.project,
    detached: true, windowsHide: true, stdio: ["ignore", output.fd, output.fd],
  });
  state.backendPid = backend.pid; backend.unref(); await output.close(); await save(state);
  await sleep(1500);
  const rpc = await connect(state);
  try {
    const created = await rpc.request("thread/start", { cwd: state.project, ephemeral: false,
      modelProvider: "placement_poc", model: "gpt-5.4", approvalPolicy: "never", sandbox: "read-only" });
    state.threadId = created.thread.id;
    state.threadPath = created.thread.path;
    await rpc.request("thread/name/set", { threadId: state.threadId, name: "Mallard existing-chat placement POC" });
    await save(state);
  } finally { rpc.close(); }
  if (launchDesktop) state.desktopPid = await launch(state, seedProject ? `codex://threads/new?path=${encodeURIComponent(state.project)}` : undefined);
  await save(state);
  return state;
}

async function populate(state) {
  const { LLMock } = await import("@copilotkit/aimock");
  const mock = new LLMock({ host: "127.0.0.1", port: 0, strict: true, logLevel: "silent" });
  mock.addFixture({ match: { userMessage: /Mallard POC:/u }, response: { content: "Mallard POC fixture completed locally." } });
  const mockUrl = await mock.start();
  const rpc = await connect(state);
  try {
    await rpc.request("thread/resume", { threadId: state.threadId, approvalPolicy: "never",
      config: { "model_providers.placement_poc.base_url": `${mockUrl}/v1` } });
    const completed = rpc.until("turn/completed");
    await rpc.request("turn/start", { threadId: state.threadId,
      input: [{ type: "text", text: "Mallard POC: retain this fixture chat for project placement.", text_elements: [] }] });
    const result = await completed;
    assert.equal(result.turn.status, "completed");
    return result;
  } finally { rpc.close(); await mock.stop(); }
}

async function restart(state) {
  await powershell(`Get-CimInstance Win32_Process -Filter "ProcessId=${state.desktopPid}" | Where-Object { $_.CommandLine.Contains(${psQuote(state.userData)}) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }`);
  await sleep(1000);
  state.desktopPid = await launch(state, state.desktopOwnsBackend ? undefined : `codex://threads/${state.threadId}`);
  await save(state);
}

function unwrap(result) {
  if (result.isError) throw new Error(result.content?.find((item) => item.type === "text")?.text ?? "Native tool failed");
  const text = result.content?.find((item) => item.type === "text")?.text;
  assert.equal(typeof text, "string");
  return JSON.parse(text);
}

async function readyBridge(state) {
  let failure;
  for (let attempt = 0; attempt < 20; attempt++) {
    let bridge;
    try {
      bridge = await appTools(state);
      unwrap(await bridge.call("list_projects", {}));
      return bridge;
    } catch (error) { bridge?.close(); failure = error; }
    await sleep(500);
  }
  throw failure;
}

async function registerMissingProject(state) {
  const bridge = await readyBridge(state);
  const projects = async () => unwrap(await bridge.call("list_projects", {})).projects;
  const matching = (items) => items.filter((item) => item.projectKind === "local" &&
    item.hostId === "local" && typeof item.path === "string" &&
    resolve(item.path).toLowerCase() === resolve(state.project).toLowerCase());
  try {
    assert.equal(matching(await projects()).length, 0, "Registration must start with no Desktop project");
    console.log("PASS destination_project_initially_absent");
    const started = Date.now();
    await launch(state, `codex://threads/new?path=${encodeURIComponent(state.project)}`);
    const deadline = started + 30_000;
    let matches = [];
    while (Date.now() < deadline) {
      matches = matching(await projects());
      if (matches.length) break;
      await sleep(250);
    }
    assert.equal(matches.length, 1, "Folder deep link must register exactly one native project within 30 seconds");
    state.fixtureProjectId = matches[0].projectId;
    state.registrationMs = Date.now() - started;
    state.projectRegisteredByDeepLink = true;
    await save(state);
    console.log(`PASS folder_deep_link_registers_native_project (${state.registrationMs} ms)`);
  } finally { bridge.close(); }
}

async function verifyOwnedRegistration(state) {
  const checks = [];
  const observedThreadStatuses = [];
  const pass = (name) => { checks.push(name); console.log(`PASS ${name}`); };
  const original = await readFile(state.threadPath);
  await cleanup(state);
  state.desktopOwnsBackend = true;
  state.desktopPid = await launch(state);
  await save(state);
  let bridge;
  try {
    bridge = await readyBridge(state);
    pass("desktop_owned_backend_profile_and_pipe_verified");
    const call = async (tool, input) => unwrap(await bridge.call(tool, input));
    const read = () => call("read_thread", { threadId: state.threadId, hostId: "local",
      turnLimit: 10, includeOutputs: true, maxOutputCharsPerItem: 4000 });
    assert.equal((await read()).thread.status.type, "notLoaded");
    pass("existing_chat_initially_not_loaded");
    await registerMissingProject(state);
    pass("absent_project_registered_without_seeding_or_manual_action");
    const projects = async () => (await call("list_projects", {})).projects.filter((item) =>
      item.projectKind === "local" && item.hostId === "local" && typeof item.path === "string" &&
      resolve(item.path).toLowerCase() === resolve(state.project).toLowerCase());
    const membership = async () => {
      const listing = await call("list_threads", { limit: 50 });
      const entries = [...listing.pinnedThreads, ...listing.threads];
      assert.equal(entries.length, 1, "Opening the folder must not create an extra chat");
      assert.equal(entries[0].id, state.threadId);
      return entries[0].projectId;
    };
    const openAndPlace = async () => {
      assert.equal((await call("navigate_to_codex_page", { threadId: state.threadId })).navigated, true);
      const deadline = Date.now() + 30_000;
      let detail;
      while (Date.now() < deadline) {
        detail = await read();
        assert.equal(detail.thread.id, state.threadId);
        assert.equal(detail.thread.hostId, "local");
        assert.ok(["idle", "notLoaded"].includes(detail.thread.status.type));
        // An idle backend is not required to view a stored chat. Desktop can
        // report notLoaded while returning its full history and placing it.
        if (detail.turns.length > 0) break;
        await sleep(250);
      }
      assert.ok(detail?.turns.length > 0, "Desktop must return the existing chat's history");
      assert.ok(JSON.stringify(detail.turns).includes("Mallard POC fixture completed locally."));
      observedThreadStatuses.push(detail.thread.status.type);
      // One native move, no retry after an uncertain mutation result.
      const moved = await call("move_thread_to_project", { source: "codex", hostId: "local",
        threadId: state.threadId, projectId: state.fixtureProjectId });
      assert.equal(moved.projectId, state.fixtureProjectId);
      assert.equal(moved.threadId, state.threadId);
      assert.equal((await call("navigate_to_codex_page", { threadId: state.threadId })).navigated, true);
      assert.equal(await membership(), state.fixtureProjectId);
      return detail;
    };
    const before = await openAndPlace();
    pass("same_existing_chat_read_placed_and_opened");
    await launch(state, `codex://threads/new?path=${encodeURIComponent(state.project)}`);
    await openAndPlace();
    assert.equal((await projects()).length, 1);
    pass("repeat_folder_and_chat_open_creates_no_duplicate_project_or_chat");
    await assert.rejects(appTools(state, state.userData), /Selected CODEX_HOME does not match/);
    assert.equal(await membership(), state.fixtureProjectId);
    pass("wrong_home_rejected_before_native_request");
    bridge.close(); bridge = null;
    await restart(state);
    bridge = await readyBridge(state);
    const persisted = await projects();
    assert.equal(persisted.length, 1);
    assert.equal(persisted[0].projectId, state.fixtureProjectId);
    assert.equal(await membership(), state.fixtureProjectId);
    pass("cold_restart_preserves_registered_project_and_membership");
    const after = await openAndPlace();
    assert.equal(after.thread.title, before.thread.title);
    assert.equal(after.thread.preview, before.thread.preview);
    assert.deepEqual(after.turns, before.turns);
    const finalTranscript = await readFile(state.threadPath);
    assert.ok(finalTranscript.subarray(0, original.length).equals(original));
    pass("original_transcript_and_native_history_preserved");
    const report = { scope: "windows_desktop_owned_automatic_project_registration", passed: true,
      checks, seededProject: false, manualProjectRegistration: false, registrationMs: state.registrationMs,
      observedThreadStatuses,
      activeModelTurnRequired: false, productionCodeChanged: false,
      nativeMembershipVerified: true, screenshotVerified: false,
      topology: "Desktop-owned backend; selected database owner and native pipe owner verified" };
    await writeFile(join(state.root, "registration-owned-report.json"), JSON.stringify(report, null, 2) + "\n");
    return report;
  } finally { bridge?.close(); }
}

async function verify(state) {
  const checks = [];
  const pass = (name) => { checks.push(name); console.log(`PASS ${name}`); };
  const rpc = await connect(state);
  let bridge;
  try {
    pass("selected_home_matches_backend_initialize");
    const before = await rpc.request("thread/read", { threadId: state.threadId, includeTurns: true });
    assert.ok(before.thread.preview.includes("Mallard POC:"), "Fixture must have a completed local mock turn");
    assert.ok(before.thread.turns.length > 0, "Fixture history must contain the completed turn");
    assert.equal(before.thread.status.type, "idle");
    bridge = await readyBridge(state);
    pass("desktop_tcp_peer_and_native_pipe_owner_verified");
    let project;
    for (let attempt = 0; attempt < 20 && !project; attempt++) {
      const projects = unwrap(await bridge.call("list_projects", {})).projects;
      project = projects.find((p) => p.hostId === "local" &&
        resolve(p.path).toLowerCase() === resolve(state.project).toLowerCase());
      if (!project) await sleep(500);
    }
    assert.ok(project, "Fixture project must be present in Desktop's native project list");
    pass("native_project_resolved_by_exact_root");
    const backendProjects = await rpc.request("project/list", { limit: 100 });
    const backendProject = backendProjects.data.find((project) => project.roots.some((root) =>
      resolve(root.path).toLowerCase() === resolve(state.project).toLowerCase()));
    assert.ok(backendProject, "Desktop must have migrated the fixture project to its backend");
    const thread = async () => {
      const listing = unwrap(await bridge.call("list_threads", { limit: 20 }));
      const value = [...listing.pinnedThreads, ...listing.threads].find((t) => t.id === state.threadId);
      assert.ok(value, "Existing fixture chat must be visible to Desktop");
      return value;
    };
    const move = async (projectId) => unwrap(await bridge.call("move_thread_to_project", {
      source: "codex", hostId: "local", threadId: state.threadId, projectId,
    }));
    // Establish the user's failing condition with an ordinary native move.
    // Never seed or rewrite the Desktop assignment JSON to manufacture a pass.
    await move(null);
    assert.equal((await thread()).projectId, null);
    pass("existing_chat_starts_projectless");
    const backendUpdate = await rpc.request("thread/metadata/update", {
      threadId: state.threadId, projectId: backendProject.id,
    });
    assert.equal(backendUpdate.thread.projectId, backendProject.id);
    await launch(state, `codex://threads/${state.threadId}`);
    assert.equal((await thread()).projectId, null);
    pass("backend_assignment_and_deep_link_reproduce_failure");
    const moved = await move(project.projectId);
    assert.equal(moved.projectId, project.projectId);
    assert.equal((await thread()).projectId, project.projectId);
    pass("native_move_changes_sidebar_membership");
    for (let attempt = 1; attempt <= 2; attempt++) {
      await launch(state, `codex://threads/${state.threadId}`);
      const navigation = unwrap(await bridge.call("navigate_to_codex_page", { threadId: state.threadId }));
      assert.equal(navigation.navigated, true);
      assert.equal((await thread()).projectId, project.projectId);
      pass(`open_${attempt}_preserves_project_membership`);
    }
    await assert.rejects(appTools(state, state.userData), /Selected CODEX_HOME does not match/);
    assert.equal((await thread()).projectId, project.projectId);
    pass("different_home_rejected_before_native_request");
    bridge.close(); bridge = null;
    await restart(state);
    bridge = await readyBridge(state);
    assert.equal((await thread()).projectId, project.projectId);
    pass("isolated_desktop_restart_preserves_membership");
    const after = await rpc.request("thread/read", { threadId: state.threadId, includeTurns: true });
    assert.equal(after.thread.id, before.thread.id);
    assert.equal(after.thread.preview, before.thread.preview);
    assert.equal(after.thread.name, before.thread.name);
    assert.deepEqual(after.thread.turns, before.thread.turns);
    pass("same_existing_chat_and_history_preserved");
    const report = { scope: "isolated_windows_native_desktop_placement", passed: true, checks,
      activeModelTurnRequired: false, productionCodeChanged: false,
      nativeMembershipVerified: true, screenshotVerified: false,
      topology: "Isolated Desktop pinned to an isolated loopback App Server; native pipe owner PID verified" };
    await writeFile(join(state.root, "report.json"), JSON.stringify(report, null, 2) + "\n");
    return report;
  } finally { bridge?.close(); rpc.close(); }
}

async function cleanup(state) {
  // Only PIDs recorded by this fixture; no process-name based termination.
  return powershell(`${state.desktopPid ? `Get-CimInstance Win32_Process -Filter "ProcessId=${state.desktopPid}" | Where-Object { $_.CommandLine.Contains(${psQuote(state.userData)}) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force };` : ""} Get-CimInstance Win32_Process -Filter "ProcessId=${state.backendPid}" | Where-Object { $_.CommandLine.Contains(${psQuote(state.wsUrl)}) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }; 'Isolated processes stopped'`);
}

async function integration(state) {
  const startedUnregistered = state.fixtureProjectId == null;
  await cleanup(state);
  state.desktopOwnsBackend = true;
  try {
    // Match the production cold launch: no route prepares the chat first.
    state.desktopPid = await launch(state);
    await save(state);
    // The Rust test independently discovers the profile and pipe, verifies
    // native membership, repeats the move/open, and rejects a copied chat in
    // another home. It accepts only this runner's temporary fixture tree.
    for (let attempt = 0; attempt < 2; attempt++) {
      if (attempt) await restart(state);
      const bridge = await readyBridge(state);
      try {
        const initial = unwrap(await bridge.call("read_thread", {
          threadId: state.threadId, hostId: "local", turnLimit: 10,
          includeOutputs: true, maxOutputCharsPerItem: 4000,
        }));
        assert.equal(initial.thread.id, state.threadId);
        assert.ok(JSON.stringify(initial.turns).includes("Mallard POC fixture completed locally."));
        await new Promise((accept, reject) => {
          const child = spawn(args.cargo ?? "cargo", ["test", "-p", "mallard-infrastructure", "--lib",
            "native_desktop_placement_uses_the_selected_home", "--", "--ignored", "--nocapture"], {
            env: { ...cleanEnv(), MALLARD_NATIVE_PLACEMENT_ROOT: state.root },
            windowsHide: true, stdio: "inherit",
          });
          child.once("error", reject);
          child.once("exit", (code) => code === 0 ? accept() : reject(new Error("production_placement_failed")));
        });
        const projects = unwrap(await bridge.call("list_projects", {})).projects.filter((item) =>
          item.projectKind === "local" && item.hostId === "local" && typeof item.path === "string" &&
          resolve(item.path).toLowerCase() === resolve(state.project).toLowerCase());
        assert.equal(projects.length, 1);
        if (state.fixtureProjectId) assert.equal(projects[0].projectId, state.fixtureProjectId);
        state.fixtureProjectId = projects[0].projectId;
        await save(state);
      } finally { bridge.close(); }
    }
    const report = { scope: "windows_production_placement", passed: true,
      nativeMembershipVerified: true, screenshotVerified: false,
      topology: "Desktop-owned backend; selected database owner, process lifetime and native pipe owner verified",
      checks: [...(startedUnregistered ? ["production_registers_missing_folder_once"] : []),
        "notLoaded_restored_chat_placed", "production_move_and_open",
        "repeat_open_keeps_membership", "wrong_home_rejected", "history_preserved",
        "production_path_repeated_after_fixture_restart"] };
    await writeFile(join(state.root, "integration-report.json"), JSON.stringify(report, null, 2) + "\n");
    return report;
  } finally { console.log(await cleanup(state)); }
}

async function main() {
  if (args.phase === "registration-integration") {
    const state = await setup({ seedProject: false, launchDesktop: false });
    console.log(JSON.stringify({ root: state.root, seededProject: false }));
    try {
      await populate(state);
      console.log(JSON.stringify(await integration(state), null, 2));
    } finally { console.log(await cleanup(state)); }
    return;
  }
  if (args.phase === "registration-setup") {
    const state = await setup({ seedProject: false });
    console.log(JSON.stringify({ root: state.root, seededProject: false }));
    await populate(state);
    return;
  }
  if (args.phase === "registration" || args.phase === "registration-owned") {
    const state = await setup({ seedProject: false, launchDesktop: args.phase !== "registration-owned" });
    console.log(JSON.stringify({ root: state.root, seededProject: false }));
    try {
      await populate(state);
      if (args.phase === "registration-owned") {
        console.log(JSON.stringify(await verifyOwnedRegistration(state), null, 2));
        return;
      }
      await registerMissingProject(state);
      const placement = await verify(state);
      const report = { ...placement, scope: "windows_automatic_project_registration",
        seededProject: false, manualProjectRegistration: false, registrationMs: state.registrationMs,
        checks: ["destination_project_initially_absent", "folder_deep_link_registers_native_project", ...placement.checks] };
      await writeFile(join(state.root, "registration-report.json"), JSON.stringify(report, null, 2) + "\n");
      console.log(JSON.stringify(report, null, 2));
    } finally { console.log(await cleanup(state)); }
    return;
  }
  if (args.phase === "setup" || args.phase === "run") {
    const state = await setup();
    console.log(JSON.stringify({ root: state.root, selectedHomeConfirmed: true }));
    if (args.phase === "setup") return;
    try {
      await populate(state);
      console.log(JSON.stringify(await verify(state), null, 2));
    } finally { console.log(await cleanup(state)); }
    return;
  }
  const state = await loadState();
  if (args.phase === "integration") {
    console.log(JSON.stringify(await integration(state), null, 2));
  } else if (args.phase === "populate") {
    console.log(JSON.stringify(await populate(state)));
  } else if (args.phase === "verify") {
    console.log(JSON.stringify(await verify(state), null, 2));
  } else if (args.phase === "read" || args.phase === "rpc") {
    const rpc = await connect(state);
    try { console.log(JSON.stringify(await rpc.request(args.phase === "read" ? "thread/read" : args.method,
      args.phase === "read" ? { threadId: state.threadId, includeTurns: true } : JSON.parse(args.input ?? "{}")))); }
    finally { rpc.close(); }
  } else if (args.phase === "restart") {
    await restart(state);
    console.log(JSON.stringify({ desktopPid: state.desktopPid }));
  } else if (args.phase === "open") {
    console.log(JSON.stringify({ launchPid: await launch(state, `codex://threads/${state.threadId}`) }));
  } else if (args.phase === "open-project") {
    console.log(JSON.stringify({ launchPid: await launch(state, `codex://threads/new?path=${encodeURIComponent(state.project)}`) }));
  } else if (args.phase === "native") {
    const bridge = await appTools(state);
    try {
      const input = args.input ? JSON.parse(args.input) : {};
      const result = await bridge.call(args.tool ?? "list_projects", input);
      console.log(JSON.stringify(result));
    } finally { bridge.close(); }
  } else if (args.phase === "cleanup") {
    console.log(await cleanup(state));
  } else throw new Error("Use --phase=run|setup|populate|verify|integration|registration|registration-owned|registration-integration|registration-setup|read|rpc|open|open-project|native|restart|cleanup");
}

main().catch((error) => { console.error(error.message); process.exitCode = 1; })
  .finally(() => { setTimeout(() => process.exit(process.exitCode ?? 0), 250).unref(); });
