import { spawn } from "node:child_process";
import { stat } from "node:fs/promises";
import { isAbsolute } from "node:path";
import { parseArgs } from "node:util";

// Read-only feasibility probe of the installed Codex app-tools connector.
// Never supplies executor identity or invokes a project/chat mutation.
const MAX_OUTPUT_BYTES = 8 * 1024 * 1024;
const TIMEOUT_MS = 10_000;
const TOOL_NAMES = ["list_projects", "list_threads", "move_thread_to_project"];

function responseStatus(response) {
  const error = response.error;
  const result = response.result;
  if (!error && result && !result.isError) return "ok";
  const message = error?.message ?? result?.content
    ?.filter((item) => item.type === "text")
    .map((item) => item.text).join("\n") ?? "";
  if (message.includes("CODEX_APP_TOOLS_PIPE_PATH")) return "desktop_endpoint_missing";
  if (message.includes("require thread metadata from the executor")) {
    return "executor_context_required";
  }
  return "connector_request_failed";
}

async function probe(serverPath, selectedHome, desktopPipe) {
  const env = { ...process.env, CODEX_HOME: selectedHome };
  for (const key of Object.keys(env)) {
    if (/^CODEX_(APP_TOOLS_|THREAD|TURN)/i.test(key)) delete env[key];
  }
  if (desktopPipe) env.CODEX_APP_TOOLS_PIPE_PATH = desktopPipe;

  const child = spawn(process.execPath, [serverPath], {
    env,
    windowsHide: true,
    stdio: ["pipe", "pipe", "ignore"],
  });
  const closed = new Promise((resolve) => child.once("close", resolve));
  let pending = null;
  let stopped = null;
  let nextId = 1;
  let buffer = Buffer.alloc(0);
  let outputBytes = 0;
  const fail = (code) => {
    stopped ??= new Error(code);
    pending?.reject(stopped);
    pending = null;
    child.kill();
  };
  const timer = setTimeout(() => fail("connector_timeout"), TIMEOUT_MS);
  child.on("error", () => fail("connector_start_failed"));
  child.on("close", () => fail("connector_closed"));
  child.stdin.on("error", () => fail("connector_input_failed"));
  child.stdout.on("data", (chunk) => {
    outputBytes += chunk.length;
    if (outputBytes > MAX_OUTPUT_BYTES) return fail("connector_output_limit");
    buffer = Buffer.concat([buffer, chunk]);
    let newline;
    while ((newline = buffer.indexOf(10)) !== -1) {
      const line = buffer.subarray(0, newline).toString("utf8").trim();
      buffer = buffer.subarray(newline + 1);
      if (!line) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        return fail("connector_invalid_json");
      }
      if (!message || message.jsonrpc !== "2.0") return fail("connector_invalid_rpc");
      if (pending && message.id === pending.id) {
        const request = pending;
        pending = null;
        request.resolve(message);
      }
    }
  });
  const send = (message) => child.stdin.write(`${JSON.stringify(message)}\n`);
  const request = (method, params) => new Promise((resolve, reject) => {
    if (stopped) return reject(stopped);
    const id = nextId++;
    pending = { id, resolve, reject };
    send({ jsonrpc: "2.0", id, method, params });
  });
  const report = {
    connection: desktopPipe ? "inherited_desktop_endpoint" : "standalone_environment",
    selectedHomePassedToConnector: env.CODEX_HOME === selectedHome,
    desktopProfileConfirmed: false,
    assignmentAttempted: false,
  };

  try {
    const initialized = await request("initialize", {
      protocolVersion: "2025-11-25",
      capabilities: {},
      clientInfo: { name: "mallard-desktop-placement-poc", version: "0.1.0" },
    });
    report.initialize = responseStatus(initialized);
    if (report.initialize !== "ok") return report;
    send({ jsonrpc: "2.0", method: "notifications/initialized" });
    const listing = await request("tools/list", {});
    report.discovery = responseStatus(listing);
    if (report.discovery !== "ok") return report;
    const tools = listing.result.tools;
    if (!Array.isArray(tools) || tools.length > 512) {
      throw new Error("connector_invalid_catalog");
    }
    report.capabilities = Object.fromEntries(
      TOOL_NAMES.map((name) => [name, tools.some((tool) => tool.name === name)]),
    );
    const move = tools.find((tool) => tool.name === "move_thread_to_project");
    report.moveAdvertisesHomeArgument = ["codexHome", "codex_home", "home", "homePath"]
      .some((key) => Object.hasOwn(move?.inputSchema?.properties ?? {}, key));
    if (report.capabilities.list_projects) {
      // Deliberately omit _meta: standalone Mallard has no executor identity.
      // Stop at the connector's boundary; never invent a calling chat or turn.
      const read = await request("tools/call", { name: "list_projects", arguments: {} });
      report.projectReadWithoutExecutor = responseStatus(read);
    }
    return report;
  } catch (error) {
    // Only locally generated categories leave this process, never raw responses.
    report.failure = /^connector_[a-z_]+$/.test(error.message)
      ? error.message : "connector_probe_failed";
    return report;
  } finally {
    clearTimeout(timer);
    child.kill();
    await closed;
  }
}

async function main() {
  const { values } = parseArgs({
    options: { server: { type: "string" }, "codex-home": { type: "string" } },
    allowPositionals: false,
  });
  const serverPath = values.server;
  const selectedHome = values["codex-home"];
  if (!serverPath || !selectedHome || !isAbsolute(serverPath) || !isAbsolute(selectedHome)) {
    throw new Error("absolute_server_and_codex_home_arguments_required");
  }
  if (!(await stat(serverPath)).isFile() || !(await stat(selectedHome)).isDirectory()) {
    throw new Error("server_file_and_codex_home_directory_required");
  }
  const standalone = await probe(serverPath, selectedHome, null);
  const pipe = process.env.CODEX_APP_TOOLS_PIPE_PATH;
  const inherited = pipe
    ? await probe(serverPath, selectedHome, pipe)
    : { connection: "inherited_desktop_endpoint", discovery: "endpoint_not_inherited" };
  console.log(JSON.stringify({
    scope: "read_only_desktop_connector_feasibility",
    standalone,
    inherited,
    automaticPlacementProven: false,
  }, null, 2));
  // A negative feasibility result is evidence, not a successful integration.
  process.exitCode = standalone.failure || inherited.failure ? 1 : 0;
}

main().catch(() => {
  console.error("POC failed. Supply --server and --codex-home as existing absolute paths; raw errors are suppressed.");
  process.exitCode = 1;
});
