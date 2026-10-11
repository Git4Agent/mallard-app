// Mallard Codex project placement helper. Managed by Mallard.
// Desktop starts this MCP server with its bundled Node. Its native socket
// checks that process ancestry. A standalone Mallard process cannot use it.
import fs from "node:fs";
import net from "node:net";
import readline from "node:readline";
import { randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_FRAME = 8 * 1024 * 1024;
const MAX_REQUEST = 64 * 1024;
const UUID = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i;
const directory = dirname(fileURLToPath(import.meta.url));
const home = fs.realpathSync(process.env.CODEX_HOME || join(homedir(), ".codex"));
const expectedHome = process.argv[2];
const endpoint = process.env.CODEX_APP_TOOLS_PIPE_PATH;
const metadata = fs.lstatSync(directory);
if (home !== expectedHome || directory !== join(home, ".mallard-project-placement") ||
    !metadata.isDirectory() || metadata.isSymbolicLink() || metadata.uid !== process.getuid() ||
    (metadata.mode & 0o077) !== 0) {
  throw new Error("Mallard project placement profile is invalid.");
}

const clients = new Set();
const token = randomBytes(32).toString("hex");
const recordPath = join(directory, `bridge-${process.pid}-${randomUUID()}.json`);
let record;
let stopping = false;

function authenticate(value) {
  return typeof value === "string" && value.length === token.length &&
    timingSafeEqual(Buffer.from(value), Buffer.from(token));
}

async function nativeConnection() {
  if (!endpoint?.startsWith("/tmp/codex-browser-use/") || !endpoint.endsWith(".sock")) {
    throw new Error("Codex Desktop native connection is unavailable.");
  }
  const socket = net.createConnection(endpoint);
  let buffer = Buffer.alloc(0);
  let nextId = 0;
  let pending;
  socket.on("error", () => {});
  socket.on("close", () => pending?.reject(new Error("Codex Desktop connection closed.")));
  socket.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length >= 4) {
      const size = buffer.readUInt32LE(0);
      if (size > MAX_FRAME) return socket.destroy();
      if (buffer.length < size + 4) return;
      let value;
      try { value = JSON.parse(buffer.subarray(4, size + 4)); }
      catch { return socket.destroy(); }
      buffer = buffer.subarray(size + 4);
      if (!pending) return socket.destroy();
      if (value.id !== pending.id) {
        if (++pending.skipped > 32) return socket.destroy();
        continue;
      }
      value.error ? pending.reject(new Error("Codex Desktop rejected the request.")) :
        pending.resolve(value.result);
    }
  });
  await new Promise((resolve, reject) => {
    socket.once("connect", resolve);
    socket.once("error", reject);
    socket.setTimeout(30_000, () => socket.destroy());
  });
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = nextId++;
    const timer = setTimeout(() => socket.destroy(), 30_000);
    pending = { id, skipped: 0,
      resolve: (value) => { clearTimeout(timer); pending = undefined; resolve(value); },
      reject: (error) => { clearTimeout(timer); pending = undefined; reject(error); },
    };
    const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    if (body.length > MAX_REQUEST) {
      pending.reject(new Error("Codex Desktop request is too large."));
      return socket.destroy();
    }
    const header = Buffer.alloc(4);
    header.writeUInt32LE(body.length);
    socket.write(Buffer.concat([header, body]));
  });
  try {
    await request("tools/list", { threadStartKind: "all" });
    return { request, close: () => socket.destroy() };
  } catch (error) {
    socket.destroy();
    throw error;
  }
}

function validateCall(value, threadId) {
  const args = value.arguments;
  if (!args || typeof args !== "object" || Array.isArray(args)) return false;
  switch (value.tool) {
    case "list_projects": return Object.keys(args).length === 0;
    case "list_threads":
      return Number.isInteger(args.limit) && args.limit >= 1 && args.limit <= 50 &&
        Object.keys(args).length === 1;
    case "move_thread_to_project":
      return args.source === "codex" && args.hostId === "local" &&
        args.threadId === threadId && typeof args.projectId === "string" &&
        args.projectId.trim().length > 0 && args.projectId.length <= 256 &&
        Object.keys(args).length === 4;
    case "navigate_to_codex_page":
      return args.threadId === threadId && Object.keys(args).length === 1;
    default: return false;
  }
}

const server = net.createServer((socket) => {
  if (stopping || clients.size >= 16) return socket.destroy();
  clients.add(socket);
  socket.setEncoding("utf8");
  let native;
  let threadId;
  let buffer = "";
  let busy = false;
  let count = 0;
  const correlation = randomUUID();
  socket.setTimeout(30_000, () => socket.destroy());
  socket.on("error", () => {});
  socket.on("close", () => {
    native?.close();
    clients.delete(socket);
    if (stopping && clients.size === 0) process.exit(0);
  });
  socket.on("data", async (chunk) => {
    if (busy) return socket.destroy();
    buffer += chunk;
    if (Buffer.byteLength(buffer) > MAX_REQUEST) return socket.destroy();
    const end = buffer.indexOf("\n");
    if (end < 0) return;
    busy = true;
    try {
      const input = JSON.parse(buffer.slice(0, end));
      if (buffer.slice(end + 1).length > 0) throw new Error("Invalid placement request.");
      buffer = "";
      if (!native) {
        if (!authenticate(input.token) || input.home !== home || !UUID.test(input.threadId)) {
          throw new Error("Placement profile does not match.");
        }
        threadId = input.threadId;
        native = await nativeConnection();
        if (socket.destroyed) return native.close();
        socket.write(JSON.stringify({ ready: true, home }) + "\n");
      } else {
        if (!validateCall(input, threadId) || ++count > 4) throw new Error("Invalid placement operation.");
        const result = await native.request("tools/call", {
          namespace: "codex_app", tool: input.tool, arguments: input.arguments,
          callerSource: "codex", hostId: "local", threadId,
          turnId: `mallard-${correlation}`, callId: `mallard-${correlation}-${count}`,
        });
        socket.write(JSON.stringify({ result }) + "\n");
      }
    } catch {
      // Native output can contain private paths. Return a fixed error only.
      socket.end(JSON.stringify({ error: "Codex Desktop project placement failed." }) + "\n");
      native?.close();
    } finally { busy = false; }
  });
});

server.listen(0, "127.0.0.1", () => {
  record = JSON.stringify({ port: server.address().port, token, home });
  fs.writeFileSync(recordPath, record, { flag: "wx", mode: 0o600 });
});
server.on("error", () => process.exit(1));

// No model-facing tool is needed. Desktop keeps this normal MCP connection
// while the chat is loaded; discovery-only connections can close at any time.
const lines = readline.createInterface({ input: process.stdin });
lines.on("line", (line) => {
  if (Buffer.byteLength(line) > MAX_REQUEST) return process.exit(1);
  try {
    const input = JSON.parse(line);
    if (input.id === undefined) return;
    let result;
    switch (input.method) {
      case "initialize":
        result = { protocolVersion: input.params.protocolVersion, capabilities: { tools: {} },
          serverInfo: { name: "mallard-project-placement", version: "1.0.0" } };
        break;
      case "tools/list": result = { tools: [] }; break;
      case "ping": result = {}; break;
      default:
        process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: input.id,
          error: { code: -32601, message: "Method not found" } }) + "\n");
        return;
    }
    process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: input.id, result }) + "\n");
  } catch { process.exit(1); }
});
function close(drain) {
  if (stopping) {
    if (!drain) process.exit(0);
    return;
  }
  stopping = true;
  server.close();
  // Delete only this helper's exact record; never follow a replacement link.
  try {
    const metadata = fs.lstatSync(recordPath);
    if (metadata.isFile() && !metadata.isSymbolicLink() && metadata.size <= 4096 &&
        fs.readFileSync(recordPath, "utf8") === record) fs.unlinkSync(recordPath);
  } catch {}
  // MCP discovery can close stdin while an authenticated placement is in
  // flight. Finish only that connection, within the caller's 30-second bound.
  // Do not detach or accept new requests after Desktop closes the MCP client.
  if (drain && clients.size > 0) {
    setTimeout(() => process.exit(0), 30_000).unref();
  } else {
    for (const client of clients) client.destroy();
    process.exit(0);
  }
}
lines.on("close", () => close(true));
process.on("SIGTERM", () => close(false));
process.on("SIGINT", () => close(false));
