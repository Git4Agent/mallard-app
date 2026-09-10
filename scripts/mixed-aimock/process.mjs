import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile, lstat } from "node:fs/promises";
import { join } from "node:path";
import { MODELS } from "./scenario.mjs";

const owned = new Set();
export async function stopOwned() {
  const children = [...owned];
  const waits = children.map((child) => child.exitCode !== null || child.signalCode !== null ? Promise.resolve()
    : new Promise((resolve) => child.once("close", resolve)));
  for (const child of children) killGroup(child);
  let timer;
  await Promise.race([Promise.all(waits), new Promise((resolve) => { timer = setTimeout(resolve, 5000); })]);
  clearTimeout(timer);
}
function killGroup(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  try { process.kill(-child.pid, "SIGKILL"); } catch { child.kill("SIGKILL"); }
}
export function environment(home, extra = {}) {
  return {
    PATH: process.env.PATH, LANG: "en_US.UTF-8", LC_ALL: "en_US.UTF-8",
    HOME: home, USERPROFILE: home, TMPDIR: join(home, "tmp"),
    NO_PROXY: "127.0.0.1", no_proxy: "127.0.0.1",
    DISABLE_TELEMETRY: "1", DISABLE_ERROR_REPORTING: "1",
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1", ...extra,
    CLAUDE_CODE_MAX_RETRIES: "0",
  };
}

export class LineProcess {
  constructor(command, args, { env, cwd, prefix = "", maxBytes = 512 * 1024 * 1024 } = {}) {
    this.child = spawn(command, args, { env, cwd, detached: true, stdio: ["pipe", "pipe", "pipe"] });
    owned.add(this.child);
    this.lines = []; this.completed = []; this.waiter = null; this.failure = null; this.stderr = "";
    this.counter = 0; this.prefix = prefix; let pending = ""; let count = 0;
    this.done = new Promise((resolve) => {
      this.child.once("error", () => { this.fail("process_spawn"); resolve(-1); });
      this.child.once("close", (code) => { owned.delete(this.child); this.fail("process_eof"); resolve(code); });
    });
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk) => {
      count += Buffer.byteLength(chunk);
      if (count > maxBytes) { this.fail("process_output_limit"); killGroup(this.child); return; }
      pending += chunk.toString("utf8");
      if (Buffer.byteLength(pending) > 32 * 1024 * 1024) { this.fail("process_line_limit"); killGroup(this.child); return; }
      let index;
      while ((index = pending.indexOf("\n")) >= 0) {
        const line = pending.slice(0, index); pending = pending.slice(index + 1);
        if (prefix && !line.startsWith(prefix)) continue;
        let value;
        try { value = JSON.parse(prefix ? line.slice(prefix.length) : line); }
        catch { this.fail("process_json"); killGroup(this.child); return; }
        if (this.waiter) { const waiter = this.waiter; this.waiter = null; waiter.resolve(value); }
        else { this.lines.push(value); if (this.lines.length > 8192) { this.fail("process_queue_limit"); killGroup(this.child); } }
      }
    });
    this.child.stderr.on("data", (chunk) => {
      if (this.stderr.length < 64 * 1024) this.stderr += chunk.toString("utf8").slice(0, 64 * 1024 - this.stderr.length);
    });
    this.child.stdin.on("error", () => this.fail("process_stdin"));
  }
  fail(code) { this.failure ??= code; if (this.waiter) { const error = new Error(code); error.privateDetail = this.stderr; this.waiter.reject(error); this.waiter = null; } }
  send(value) { this.child.stdin.write(`${JSON.stringify(value)}\n`); }
  next(timeout = 180_000) {
    if (this.lines.length) return Promise.resolve(this.lines.shift());
    if (this.failure) { const error = new Error(this.failure); error.privateDetail = this.stderr; return Promise.reject(error); }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.waiter = null; killGroup(this.child); reject(new Error("process_timeout")); }, timeout);
      this.waiter = { resolve: (v) => { clearTimeout(timer); resolve(v); }, reject: (e) => { clearTimeout(timer); reject(e); } };
    });
  }
  async request(method, params) {
    const id = ++this.counter; this.send({ id, method, params });
    for (;;) {
      const message = await this.next();
      if (message.id !== undefined) {
        assert.equal(message.id, id, "RPC identity");
        if (message.error) { const error = new Error(`native_rpc_${method.replaceAll("/", "_")}`); error.privateDetail = JSON.stringify(message.error); throw error; }
        return message.result;
      }
      if (message.method === "turn/completed") this.completed.push(message);
    }
  }
  async close(shutdown) {
    if (shutdown) this.send(shutdown);
    this.child.stdin.end();
    const timer = setTimeout(() => killGroup(this.child), 30_000);
    const code = await this.done; clearTimeout(timer);
    assert.equal(code, 0, "process did not exit cleanly");
  }
  kill() { killGroup(this.child); }
}

export async function runBounded(command, args, options, timeout = 180_000) {
  const child = spawn(command, args, { ...options, detached: true, stdio: ["ignore", "pipe", "pipe"] });
  owned.add(child); let output = Buffer.alloc(0); let overflow = false;
  let errors = Buffer.alloc(0);
  for (const [stream, isError] of [[child.stdout, false], [child.stderr, true]]) stream.on("data", (data) => {
    if (isError) errors = Buffer.concat([errors, data]); else output = Buffer.concat([output, data]);
    if (output.length + errors.length > 8 * 1024 * 1024) { overflow = true; killGroup(child); }
  });
  const timer = setTimeout(() => { overflow = true; killGroup(child); }, timeout);
  const code = await new Promise((resolve) => { child.once("close", resolve); child.once("error", () => resolve(-1)); });
  clearTimeout(timer); owned.delete(child);
  if (overflow || code !== 0) {
    const error = new Error(errors.includes(Buffer.from("sandbox_apply")) ? "native_nested_sandbox" : `native_process_${overflow ? "limit" : "failure"}`);
    const details = [];
    for (const line of output.toString("utf8").split("\n")) {
      try { const event = JSON.parse(line); if (event.type === "error" || event.type === "turn.failed" || event.is_error === true) details.push(event.message ?? event.error ?? event.result ?? event.subtype); } catch {}
    }
    error.privateDetail = errors.toString("utf8") || JSON.stringify(details); throw error;
  }
  return output.toString("utf8");
}

export async function startMock(repository, root, provider, document) {
  await mkdir(root, { mode: 0o700 });
  const scenarioPath = join(root, "aimock-scenario.json"); const auditPath = join(root, "aimock-audit.json");
  await writeFile(scenarioPath, JSON.stringify(document), { mode: 0o600, flag: "wx" });
  const process = new LineProcess("node", [join(repository, "scripts", `live-${provider}-aimock-server.mjs`), "--scenario", scenarioPath, "--audit", auditPath],
    { env: environment(root), cwd: root, maxBytes: 1024 * 1024 });
  const ready = await process.next(30_000);
  assert.equal(ready.version, 1); assert.ok(Number.isInteger(ready.port) && ready.port > 0 && ready.port <= 65535);
  assert.equal(ready.type, provider === "codex" ? "mallard_aimock_ready" : "mallard_claude_aimock_ready");
  return {
    url: `http://127.0.0.1:${ready.port}/v1`,
    async finish() {
      await process.close({ command: "shutdown", version: 1 });
      const audit = JSON.parse(await readFile(auditPath, "utf8"));
      assert.equal(audit.request_count, document.expected_request_count, "mock request count");
      for (const key of ["unknown_request_count", "strict_miss_count", "history_mismatch_count", "duplicate_request_count", "unused_scenario_count"]) assert.equal(audit[key], 0, `mock ${key}`);
      assert.equal(audit.all_scenarios_consumed_once, true);
      assert.equal(audit.journal_saturated, false);
      if (provider === "claude") {
        assert.equal(audit.raw_probe_request_count, document.expected_probe_request_count);
        assert.equal(audit.raw_unexpected_request_count, 0);
      }
      return { provider, requests: audit.request_count, probes: document.expected_probe_request_count ?? 0,
        scenarios: audit.scenarios.map((item) => ({ id: item.id, requests: item.request_count })) };
    }, kill: () => process.kill(),
  };
}

export function codexArguments(url) {
  const provider = "mallard_aimock";
  return [
    `model_provider="${provider}"`, 'model_reasoning_effort="low"', "features.plugins=false",
    "features.enable_request_compression=false", `model_providers.${provider}.name="Mallard local mock"`,
    `model_providers.${provider}.base_url=${JSON.stringify(url)}`, `model_providers.${provider}.wire_api="responses"`,
    `model_providers.${provider}.requires_openai_auth=false`, `model_providers.${provider}.request_max_retries=0`,
    `model_providers.${provider}.stream_max_retries=0`,
  ].flatMap((value) => ["--config", value]);
}

export async function codexServer(replica, url) {
  const process = new LineProcess("codex", ["app-server", "--stdio", ...codexArguments(url)],
    { cwd: replica.project, env: environment(replica.home, { CODEX_HOME: replica.codex }) });
  await process.request("initialize", { clientInfo: { name: "mallard-mixed-test", version: "1" }, capabilities: { experimentalApi: true } });
  process.send({ method: "initialized" });
  const parameters = { modelProvider: "mallard_aimock", model: MODELS.codex, cwd: replica.project, approvalPolicy: "never", sandbox: "read-only" };
  return {
    async create(parent) {
      const result = await process.request(parent ? "thread/fork" : "thread/start", parent
        ? { ...parameters, threadId: parent, excludeTurns: true, ephemeral: false }
        : { ...parameters, historyMode: "paginated", ephemeral: false });
      assert.match(result.thread.id, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
      assert.equal(result.thread.historyMode, "paginated");
      assert.equal(result.thread.forkedFromId ?? null, parent ?? null);
      return result.thread.id;
    },
    async resume(id) { const result = await process.request("thread/resume", { ...parameters, threadId: id, excludeTurns: true }); assert.equal(result.thread.id, id); },
    async turn(id, prompt) {
      const result = await process.request("turn/start", { threadId: id, input: [{ type: "text", text: prompt }], approvalPolicy: "never", sandboxPolicy: { type: "readOnly", networkAccess: false } });
      for (;;) {
        const message = process.completed.shift() ?? await process.next();
        if (message.method !== "turn/completed") continue;
        assert.equal(message.params.threadId, id); assert.equal(message.params.turn.id, result.turn.id);
        if (message.params.turn.status !== "completed") { const error = new Error("native_turn_failed"); error.privateDetail = JSON.stringify(message.params.turn.error); throw error; } break;
      }
    },
    close: () => process.close(), kill: () => process.kill(),
  };
}

export async function claudeTurn(replica, url, id, fresh, prompt, expected) {
  const output = await runBounded("claude", ["--bare", "--print", "--output-format", "json", "--model", MODELS.claude,
    "--permission-mode", "dontAsk", "--tools=Read", fresh ? "--session-id" : "--resume", id, prompt],
  { cwd: replica.project, env: environment(replica.home, { CLAUDE_CONFIG_DIR: replica.claude,
    ANTHROPIC_BASE_URL: url.replace(/\/v1$/u, ""), ANTHROPIC_API_KEY: "mallard-local-aimock-placeholder" }) });
  const value = JSON.parse(output);
  assert.equal(value.is_error, false, "Claude turn failed");
  assert.ok(value.result === expected, "Claude response body mismatch");
  assert.equal(value.session_id, id, "Claude resumed a different session");
  const transcript = join(replica.claude, "projects", replica.project.replace(/[^A-Za-z0-9]/gu, "-"), `${id}.jsonl`);
  const deadline = Date.now() + 5000;
  for (;;) {
    let records = [];
    try {
      const metadata = await lstat(transcript);
      assert.ok(metadata.isFile() && !metadata.isSymbolicLink() && metadata.size <= 16 * 1024 * 1024, "Claude transcript type or size");
      const bytes = await readFile(transcript, "utf8");
      if (bytes.endsWith("\n")) records = bytes.trimEnd().split("\n").map((line) => JSON.parse(line));
    } catch (error) { if (error.code !== "ENOENT" && !(error instanceof SyntaxError)) throw error; }
    const text = (record) => typeof record.message?.content === "string" ? record.message.content
      : (record.message?.content ?? []).filter((part) => part.type === "text").map((part) => part.text).join("");
    const user = records.find((record) => record.type === "user" && text(record).includes(prompt));
    const assistant = records.find((record) => record.type === "assistant" && text(record) === expected);
    if (user && assistant) {
      const ids = new Set(records.map((record) => record.uuid).filter(Boolean));
      assert.ok(!assistant.parentUuid || ids.has(assistant.parentUuid), "Claude persisted a missing message parent");
      return;
    }
    if (Date.now() >= deadline) throw new Error("claude_transcript_did_not_settle");
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}
