import { constants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_BYTES = 16 * 1024 * 1024;
const MAX_RECORDS = 10_000;

function scalar(value) {
  if (typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9_.:/+-]{0,127}$/.test(value)) return value;
  if (typeof value === "number" && Number.isFinite(value) && value >= 0) return value;
  return null;
}

function field(value, source, status = "recorded") {
  const safe = scalar(value);
  return safe === null
    ? { value: null, status: "unknown", source: null }
    : { value: safe, status, source };
}

function consensus(fields, source, inferred = false) {
  if (fields.length === 0) return field(null);
  const values = [...new Set(fields.flatMap((item) => item.candidates ?? (item.value === null ? [] : [item.value])))];
  if (values.length > 1 || fields.some((item) => item.status === "conflict")) {
    return { value: null, status: "conflict", source, candidates: values };
  }
  if (fields.some((item) => item.status === "unknown")) return field(null);
  return field(values[0], source, inferred ? "inferred" : "recorded");
}

function codexRows(records) {
  // Only the current canonical item stream is used. response_item and
  // item_started records can duplicate the same visible message.
  const contexts = new Map();
  records.forEach((record, index) => {
    if (record.type !== "turn_context" || typeof record.payload?.turn_id !== "string") return;
    const turn = record.payload.turn_id;
    if (!contexts.has(turn)) contexts.set(turn, []);
    contexts.get(turn).push({ index, payload: record.payload });
  });
  const rows = [];
  const seen = new Set();
  records.forEach((record, index) => {
    const payload = record.payload;
    if (record.type !== "event_msg" || payload?.type !== "item_completed") return;
    const item = payload.item;
    const role = item?.type === "UserMessage" ? "user" : item?.type === "AgentMessage" ? "assistant" : null;
    if (!role) return;
    const key = JSON.stringify([payload.turn_id, item.id]);
    if (typeof item.id === "string" && seen.has(key)) return;
    seen.add(key);
    const candidates = contexts.get(payload.turn_id) ?? [];
    const preceding = candidates.filter((context) => context.index <= index);
    // A user item can be saved before its turn context. Never borrow settings
    // from another turn, or apply future settings to an earlier assistant.
    const context = preceding.at(-1) ?? (role === "user" ? candidates[0] : undefined);
    rows.push({
      message: rows.length + 1,
      role,
      model: field(context?.payload.model, "turn_context.payload.model"),
      thinking_level: field(context?.payload.effort, "turn_context.payload.effort"),
    });
  });
  return rows;
}

function humanInput(record) {
  if (record.type !== "user" || record.message?.role !== "user"
    || record.isMeta || record.isSynthetic || record.isCompactSummary) return false;
  const content = record.message.content;
  if (Array.isArray(content)) {
    return !content.some((block) => block.type === "tool_result")
      && content.some((block) => ["text", "image", "document"].includes(block.type));
  }
  return typeof content === "string" && content.trim().length > 0
    && !/^\s*(<command-|<local-command-|<system-reminder>|\[Request interrupted by user)/.test(content);
}

function assistantRecord(record) {
  return record.type === "assistant" && record.message?.role === "assistant"
    && !record.isMeta && !record.isSynthetic && !record.isApiErrorMessage
    && record.message.model !== "<synthetic>";
}

function claudeEffort(record) {
  // The native probe verified null perTurnEffort. A non-null value can change
  // per-message API behavior; its effective meaning is outside this POC.
  if (record.perTurnEffort != null) return field(null);
  return field(record.effort, "assistant.effort");
}

function claudeRows(records) {
  const byId = new Map();
  const users = new Map();
  const groups = new Map();
  const entries = [];
  for (const [index, record] of records.entries()) {
    if (typeof record.uuid === "string") byId.set(record.uuid, record);
    if (humanInput(record)) {
      const entry = { index, record, role: "user" };
      // Replayed user records do not create a second visible input.
      if (typeof record.uuid === "string" && users.has(record.uuid)) continue;
      entries.push(entry);
      if (typeof record.uuid === "string") users.set(record.uuid, entry);
    } else if (assistantRecord(record)) {
      const id = JSON.stringify([record.sessionId ?? null, record.message.id ?? record.uuid ?? index]);
      if (!groups.has(id)) {
        const entry = { index, role: "assistant", records: [] };
        groups.set(id, entry);
        entries.push(entry);
      }
      groups.get(id).records.push(record);
    }
  }

  function owner(record) {
    const visited = new Set();
    let parent = record.parentUuid;
    while (typeof parent === "string" && !visited.has(parent)) {
      visited.add(parent);
      const node = byId.get(parent);
      if (!node || node.sessionId !== record.sessionId) return null;
      if (users.has(parent)) return users.get(parent);
      parent = node.parentUuid;
    }
    return null;
  }

  const rows = new Map();
  const replies = new Map();
  for (const [index, entry] of entries.entries()) {
    const row = { message: index + 1, role: entry.role, model: field(null), thinking_level: field(null) };
    if (entry.role === "assistant") {
      row.model = consensus(entry.records.map((record) => field(record.message.model, "assistant.message.model")), "assistant.message.model");
      row.thinking_level = consensus(entry.records.map(claudeEffort), "assistant.effort");
      const owners = new Set(entry.records.map(owner));
      // Do not attach a reply across a missing parent, cycle, or session.
      if (owners.size === 1 && !owners.has(null)) {
        const user = [...owners][0];
        if (!replies.has(user)) replies.set(user, []);
        replies.get(user).push(row);
      }
    }
    rows.set(entry, row);
  }
  for (const [entry, row] of rows) {
    if (entry.role !== "user") continue;
    const linked = replies.get(entry) ?? [];
    row.model = consensus(linked.map((reply) => reply.model), "linked assistant messages", true);
    row.thinking_level = consensus(linked.map((reply) => reply.thinking_level), "linked assistant messages", true);
    row.reply_messages = linked.map((reply) => reply.message);
  }
  return [...rows.values()];
}

export function extractModelInfo(provider, records) {
  if (!Array.isArray(records) || records.length > MAX_RECORDS) throw new Error("record_limit");
  if (records.some((record) => !record || typeof record !== "object" || Array.isArray(record))) {
    throw new Error("invalid_record");
  }
  if (provider === "codex") return codexRows(records);
  if (provider === "claude") return claudeRows(records);
  throw new Error("unsupported_provider");
}

export async function readRecords(path) {
  const absolute = resolve(path);
  if (await realpath(absolute) !== absolute) throw new Error("canonical_path_required");
  const before = await lstat(absolute);
  if (!before.isFile() || before.nlink !== 1 || before.size > MAX_BYTES) throw new Error("input_not_admitted");
  const file = await open(absolute, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const opened = await file.stat();
    if (!opened.isFile() || opened.dev !== before.dev || opened.ino !== before.ino) throw new Error("input_changed");
    const buffer = Buffer.alloc(MAX_BYTES + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      if (!bytesRead) break;
      length += bytesRead;
    }
    const after = await file.stat();
    if (length > MAX_BYTES) throw new Error("input_too_large");
    if (length !== before.size || after.size !== before.size || after.mtimeMs !== before.mtimeMs || after.ctimeMs !== before.ctimeMs) {
      throw new Error("input_changed");
    }
    const lines = buffer.subarray(0, length).toString("utf8").split("\n").filter((line) => line.trim());
    if (lines.length > MAX_RECORDS) throw new Error("record_limit");
    return lines.map((line) => {
      if (Buffer.byteLength(line) > 1024 * 1024) throw new Error("record_too_large");
      try { return JSON.parse(line); } catch { throw new Error("invalid_json"); }
    });
  } finally {
    await file.close();
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [provider, path, ...extra] = process.argv.slice(2);
    if (!path || extra.length) throw new Error("usage: node model-info.mjs <codex|claude> <canonical-jsonl-path>");
    const rows = extractModelInfo(provider, await readRecords(path));
    if (rows.length === 0) throw new Error("no_supported_messages");
    console.log(JSON.stringify({ provider, messages: rows }, null, 2));
  } catch (error) {
    // Filesystem errors can contain private paths. Print only a stable code.
    console.error(error.code ?? error.message);
    process.exitCode = 1;
  }
}
