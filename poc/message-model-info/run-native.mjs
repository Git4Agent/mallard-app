import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, mkdtemp, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { createServer, request } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { LLMock } from "@copilotkit/aimock";
import { extractModelInfo, readRecords } from "./model-info.mjs";

// All client state and prompts in this probe are synthetic. Only the loopback
// mock serves model requests. No credentials are inherited by child processes.
const root = await realpath(await mkdtemp(join(tmpdir(), "mallard-message-model-poc-")));
const project = join(root, "project");
const processHome = join(root, "home");
const codexHome = join(root, "codex");
const claudeHome = join(root, "claude");
for (const path of [project, processHome, codexHome, claudeHome]) {
  await mkdir(path, { mode: 0o700 });
}
const environment = {
  PATH: process.env.PATH,
  HOME: processHome,
  USERPROFILE: processHome,
  TMPDIR: root,
  XDG_CONFIG_HOME: join(root, "config"),
  XDG_CACHE_HOME: join(root, "cache"),
  XDG_DATA_HOME: join(root, "data"),
  CODEX_HOME: codexHome,
  CLAUDE_CONFIG_DIR: claudeHome,
  LANG: "en_US.UTF-8",
  DISABLE_TELEMETRY: "1",
  DISABLE_ERROR_REPORTING: "1",
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
};

async function run(binary, args, extra = {}) {
  return await new Promise((accept, reject) => {
    const child = spawn(binary, args, {
      cwd: project,
      env: { ...environment, ...extra },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let exceeded = false;
    const timer = setTimeout(() => child.kill("SIGKILL"), 60_000);
    for (const [stream, append] of [
      [child.stdout, (data) => { stdout += data; }],
      [child.stderr, (data) => { stderr += data; }],
    ]) {
      stream.on("data", (data) => {
        if (stdout.length + stderr.length > 2 * 1024 * 1024) {
          exceeded = true;
          child.kill("SIGKILL");
        } else append(data.toString("utf8"));
      });
    }
    child.on("error", (error) => { clearTimeout(timer); reject(error); });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      if (code !== 0 || exceeded) {
        // The child has only synthetic state. Remove its temporary path.
        reject(new Error(`${binary}: exit=${code}, signal=${signal}, output_limit=${exceeded}; ${stderr.replaceAll(root, "<temporary-home>").slice(-1200)}`));
      } else accept(stdout);
    });
  });
}

const mock = new LLMock({ host: "127.0.0.1", port: 0, strict: true, logLevel: "silent", journalMaxEntries: 16 });
let responseCount = 0;
mock.onMessage(/MODEL_POC_/, (body) => ({
  id: `msg_model_poc_${++responseCount}`,
  model: body.model,
  content: "MODEL_POC_REPLY",
  usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
}));
const wire = [];
const proxy = createServer(async (incoming, outgoing) => {
  try {
    if (incoming.method !== "POST" || !/^\/v1\/(responses|messages)(\?.*)?$/.test(incoming.url)) {
      outgoing.writeHead(404).end();
      return;
    }
    let size = 0;
    const chunks = [];
    for await (const chunk of incoming) {
      size += chunk.length;
      if (size > 16 * 1024 * 1024 || wire.length >= 8) throw new Error("request_limit");
      chunks.push(chunk);
    }
    const body = Buffer.concat(chunks);
    const value = JSON.parse(body);
    wire.push({
      provider: incoming.url.startsWith("/v1/responses") ? "codex" : "claude",
      model: value.model,
      effort: value.reasoning?.effort ?? value.output_config?.effort ?? null,
      thinking: value.thinking ?? null,
    });
    const forwarded = request(`${mock.url}${incoming.url}`, {
      method: "POST",
      headers: { "content-type": "application/json", "content-length": body.length },
    }, (response) => {
      outgoing.writeHead(response.statusCode, response.headers);
      response.pipe(outgoing);
    });
    forwarded.on("error", () => outgoing.destroy());
    forwarded.end(body);
  } catch {
    outgoing.writeHead(400).end();
  }
});

async function recordsUnder(directory) {
  const records = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) records.push(...await recordsUnder(path));
    if (entry.isFile() && entry.name.endsWith(".jsonl")) {
      records.push(...await readRecords(path));
    }
  }
  return records;
}

function inspectRecords(records) {
  const shapes = new Map();
  for (const value of records) {
    const payload = value.payload ?? {};
    const type = [value.type, payload.type, payload.item?.type].filter(Boolean).join("/");
    if (!shapes.has(type)) shapes.set(type, {
      type,
      count: 0,
      keys: Object.keys(value).sort(),
      payload_keys: Object.keys(payload).sort(),
      message_keys: Object.keys(value.message ?? {}).sort(),
      item_keys: Object.keys(payload.item ?? {}).sort(),
      metadata: [],
    });
    const shape = shapes.get(type);
    shape.count++;
    const metadata = {};
    for (const [scope, object] of [["record", value], ["payload", payload], ["message", value.message], ["item", payload.item]]) {
      for (const [key, content] of Object.entries(object ?? {})) {
        if (/^(model|effort|perTurnEffort|reasoning_effort|reasoningEffort|thinkingMetadata|thinking|outputConfig|output_config)$/.test(key)) {
          metadata[`${scope}.${key}`] = content;
        }
      }
    }
    if (Object.keys(metadata).length) shape.metadata.push(metadata);
  }
  return [...shapes.values()];
}

try {
  await mock.start();
  await new Promise((accept) => proxy.listen(0, "127.0.0.1", accept));
  const baseUrl = `http://127.0.0.1:${proxy.address().port}/v1`;
  const versions = {
    codex: (await run("codex", ["--version"])).trim(),
    claude: (await run("claude", ["--version"])).trim(),
  };
  console.error(JSON.stringify({ stage: "versions", ...versions }));
  let codexId;
  const claudeId = randomUUID();
  const codexModels = ["gpt-5.6-luna", "gpt-5.6-sol"];
  const claudeModels = ["claude-opus-4-6", "claude-sonnet-4-6"];
  for (const [index, effort] of ["low", "high"].entries()) {
    const overrides = {
      model_provider: '"mallard_model_poc"',
      model_reasoning_effort: JSON.stringify(effort),
      "features.plugins": "false",
      "features.enable_request_compression": "false",
      "model_providers.mallard_model_poc.name": '"Mallard model POC"',
      "model_providers.mallard_model_poc.base_url": JSON.stringify(baseUrl),
      "model_providers.mallard_model_poc.wire_api": '"responses"',
      "model_providers.mallard_model_poc.requires_openai_auth": "false",
      "model_providers.mallard_model_poc.request_max_retries": "0",
      "model_providers.mallard_model_poc.stream_max_retries": "0",
      "model_providers.mallard_model_poc.stream_idle_timeout_ms": "15000",
    };
    const args = ["--ask-for-approval", "never", "exec", "--json", "--ignore-user-config", "--ignore-rules", "--skip-git-repo-check", "--model", codexModels[index], "--sandbox", "read-only"];
    for (const [key, value] of Object.entries(overrides)) args.push("--config", `${key}=${value}`);
    if (codexId) args.push("resume", codexId);
    args.push(`MODEL_POC_CODEX_${index}`);
    const output = await run("codex", args);
    const events = output.trim().split("\n").map((line) => JSON.parse(line));
    codexId ??= events.find((event) => event.type === "thread.started")?.thread_id;
    assert.ok(codexId);
    assert.ok(output.includes("MODEL_POC_REPLY"));
    console.error(JSON.stringify({ stage: "codex", effort, passed: true }));
  }
  for (const [index, effort] of ["low", "high"].entries()) {
    const args = [
      "--setting-sources", "", "--settings", '{"disableAllHooks":true,"enabledPlugins":{}}',
      "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--disable-slash-commands",
      "--print", "--output-format", "json", "--model", claudeModels[index], "--effort", effort,
      "--permission-mode", "dontAsk", "--tools", "",
      index === 0 ? "--session-id" : "--resume", claudeId, `MODEL_POC_CLAUDE_${index}`,
    ];
    const output = JSON.parse(await run("claude", args, {
      ANTHROPIC_BASE_URL: baseUrl.slice(0, -3),
      ANTHROPIC_API_KEY: "mallard-model-poc-placeholder",
    }));
    assert.equal(output.is_error, false);
    assert.equal(output.result, "MODEL_POC_REPLY");
    console.error(JSON.stringify({ stage: "claude", effort, passed: true }));
  }
  assert.equal(responseCount, 4);
  const codexRecords = await recordsUnder(join(codexHome, "sessions"));
  const claudeRecords = await recordsUnder(join(claudeHome, "projects"));
  const messages = {
    codex: extractModelInfo("codex", codexRecords),
    claude: extractModelInfo("claude", claudeRecords),
  };
  for (const [provider, models] of [["codex", codexModels], ["claude", claudeModels]]) {
    assert.deepEqual(messages[provider].map((row) => row.role), ["user", "assistant", "user", "assistant"]);
    assert.deepEqual(messages[provider].map((row) => row.model.value), models.flatMap((model) => [model, model]));
    assert.deepEqual(messages[provider].map((row) => row.thinking_level.value), ["low", "low", "high", "high"]);
    assert.deepEqual(wire.filter((row) => row.provider === provider).map((row) => [row.model, row.effort]), [[models[0], "low"], [models[1], "high"]]);
  }
  const report = {
    versions,
    proof: "Native clients with a local mock. No real model inference.",
    validation: { passed: true, model_requests: wire.length, message_rows: messages.codex.length + messages.claude.length },
    wire,
    messages,
    codex: inspectRecords(codexRecords),
    claude: inspectRecords(claudeRecords),
  };
  const outputDirectory = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/message-model-info-poc");
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(join(outputDirectory, "native-report.json"), `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({ versions, wire, messages, passed: true }, null, 2));
} finally {
  proxy.closeAllConnections();
  await new Promise((accept) => proxy.close(accept));
  await mock.stop();
  await rm(root, { recursive: true, force: true });
}
