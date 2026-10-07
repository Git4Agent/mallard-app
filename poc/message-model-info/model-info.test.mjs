import assert from "node:assert/strict";
import { mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { extractModelInfo, readRecords } from "./model-info.mjs";

const context = (turn, model, effort) => ({ type: "turn_context", payload: { turn_id: turn, model, effort } });
const item = (turn, id, type) => ({ type: "event_msg", payload: { type: "item_completed", turn_id: turn, item: { id, type } } });
const user = (id, parent = null) => ({ type: "user", uuid: id, parentUuid: parent, sessionId: "fixture", message: { role: "user", content: "Synthetic input" } });
const assistant = (id, parent, model = "claude-opus-4-6", effort = "high", messageId = id) => ({ type: "assistant", uuid: id, parentUuid: parent, sessionId: "fixture", effort, message: { id: messageId, role: "assistant", model, content: [{ type: "text", text: "Synthetic reply" }] } });

test("Codex joins both roles to the exact turn and preserves changes", () => {
  const rows = extractModelInfo("codex", [
    context("t1", "gpt-5.6-luna", "low"),
    item("t1", "u1", "UserMessage"), item("t1", "a1", "AgentMessage"),
    context("t2", "gpt-5.6-sol", "high"),
    item("t2", "u2", "UserMessage"), item("t2", "a2", "AgentMessage"),
  ]);
  assert.deepEqual(rows.map((row) => [row.role, row.model.value, row.thinking_level.value]), [
    ["user", "gpt-5.6-luna", "low"], ["assistant", "gpt-5.6-luna", "low"],
    ["user", "gpt-5.6-sol", "high"], ["assistant", "gpt-5.6-sol", "high"],
  ]);
  assert.ok(rows.every((row) => row.thinking_level.status === "recorded"));
});

test("Codex backfills a user before context but does not label an earlier reply", () => {
  const rows = extractModelInfo("codex", [
    item("t1", "u1", "UserMessage"), item("t1", "a0", "AgentMessage"),
    context("t1", "gpt-5.6-sol", "high"), item("t1", "a1", "AgentMessage"),
  ]);
  assert.equal(rows[0].thinking_level.value, "high");
  assert.equal(rows[1].thinking_level.status, "unknown");
  assert.equal(rows[2].thinking_level.value, "high");
});

test("Codex never borrows a previous turn's model or effort", () => {
  const rows = extractModelInfo("codex", [
    context("old", "gpt-5.6-sol", "high"),
    item("new", "u1", "UserMessage"), item(undefined, "a1", "AgentMessage"),
  ]);
  assert.ok(rows.every((row) => row.model.status === "unknown" && row.thinking_level.status === "unknown"));
});

test("Codex does not count replay records or item starts twice", () => {
  const completed = item("t1", "u1", "UserMessage");
  const rows = extractModelInfo("codex", [
    context("t1", "gpt-5.6-sol", "medium"),
    { type: "response_item", payload: { type: "message", role: "user" } },
    { type: "event_msg", payload: { ...completed.payload, type: "item_started" } },
    completed, completed,
  ]);
  assert.equal(rows.length, 1);
});

test("Codex keeps context changes within a turn in message order", () => {
  const rows = extractModelInfo("codex", [
    context("t1", "gpt-5.6-sol", "low"), item("t1", "a1", "AgentMessage"),
    context("t1", "gpt-5.6-sol", "high"), item("t1", "a2", "AgentMessage"),
  ]);
  assert.deepEqual(rows.map((row) => row.thinking_level.value), ["low", "high"]);
});

test("Claude reads model and effort from each assistant, then links the input", () => {
  const rows = extractModelInfo("claude", [
    user("u1"), assistant("a1", "u1", "claude-opus-4-6", "low"),
    user("u2", "a1"), assistant("a2", "u2", "claude-sonnet-4-6", "high"),
  ]);
  assert.deepEqual(rows.map((row) => row.model.value), ["claude-opus-4-6", "claude-opus-4-6", "claude-sonnet-4-6", "claude-sonnet-4-6"]);
  assert.deepEqual(rows.map((row) => row.thinking_level.value), ["low", "low", "high", "high"]);
  assert.equal(rows[0].model.status, "inferred");
  assert.equal(rows[1].model.status, "recorded");
  assert.deepEqual(rows[0].reply_messages, [2]);
});

test("Claude traverses attachments and tool results without counting them as inputs", () => {
  const rows = extractModelInfo("claude", [
    user("u1"),
    { type: "attachment", uuid: "attach", parentUuid: "u1", sessionId: "fixture" },
    assistant("a1", "attach"),
    { ...user("tool", "a1"), message: { role: "user", content: [{ type: "tool_result", tool_use_id: "call", content: "Synthetic output" }] } },
    assistant("a2", "tool"),
  ]);
  assert.deepEqual(rows.map((row) => row.role), ["user", "assistant", "assistant"]);
  assert.deepEqual(rows[0].reply_messages, [2, 3]);
});

test("Claude groups content blocks by message ID", () => {
  const rows = extractModelInfo("claude", [
    user("u1"), assistant("a1", "u1", "claude-opus-4-6", "high", "message1"),
    assistant("a2", "a1", "claude-opus-4-6", "high", "message1"),
  ]);
  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0].reply_messages, [2]);
});

test("Claude reports multiple models or efforts for the same input", () => {
  const rows = extractModelInfo("claude", [
    user("u1"), assistant("a1", "u1", "claude-opus-4-6", "low"),
    assistant("a2", "a1", "claude-sonnet-4-6", "high"),
  ]);
  assert.equal(rows[0].model.status, "conflict");
  assert.deepEqual(rows[0].model.candidates, ["claude-opus-4-6", "claude-sonnet-4-6"]);
  assert.equal(rows[0].thinking_level.status, "conflict");
});

test("Claude does not infer effort from thinking text, a budget, or current settings", () => {
  const reply = assistant("a1", "u1");
  delete reply.effort;
  reply.thinkingMetadata = { maxThinkingTokens: 31999 };
  reply.message.content.unshift({ type: "thinking", thinking: "Synthetic reasoning" });
  const rows = extractModelInfo("claude", [
    { type: "settings", effortLevel: "high" }, user("u1"), reply, user("u2", "a1"),
  ]);
  assert.ok(rows.every((row) => row.thinking_level.status === "unknown"));
  assert.equal(rows[2].model.status, "unknown");
  assert.ok(!JSON.stringify(rows).includes("Synthetic reasoning"));
});

test("Claude follows parent IDs instead of chronological proximity", () => {
  const rows = extractModelInfo("claude", [
    user("u1"), user("u2"), assistant("a1", "u1", "claude-opus-4-6", "low"),
    assistant("a2", "u2", "claude-sonnet-4-6", "high"),
  ]);
  assert.equal(rows[0].model.value, "claude-opus-4-6");
  assert.equal(rows[1].model.value, "claude-sonnet-4-6");
});

test("Claude leaves unverified per-message effort overrides unknown", () => {
  const rows = extractModelInfo("claude", [
    user("u1"), { ...assistant("a1", "u1"), perTurnEffort: "low" },
  ]);
  assert.ok(rows.every((row) => row.thinking_level.status === "unknown"));
  assert.equal(rows[1].model.status, "recorded");
});

test("Claude leaves broken, cyclic, and cross-session parent links unassigned", () => {
  const rows = extractModelInfo("claude", [
    user("u1"), assistant("a1", "missing"),
    { type: "attachment", uuid: "cycle", parentUuid: "cycle", sessionId: "fixture" },
    assistant("a2", "cycle"), { ...assistant("a3", "u1"), sessionId: "different" },
  ]);
  assert.equal(rows[0].model.status, "unknown");
  assert.deepEqual(rows[0].reply_messages, []);
  assert.ok(rows.slice(1).every((row) => row.model.status === "recorded"));
});

test("Claude excludes local commands and synthetic errors", () => {
  const rows = extractModelInfo("claude", [
    { ...user("command"), message: { role: "user", content: "<command-name>/model</command-name>" } },
    user("u1"), { ...assistant("error", "u1"), isApiErrorMessage: true },
    { ...assistant("synthetic", "u1"), isSynthetic: true },
  ]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].model.status, "unknown");
});

test("Extracted metadata does not contain message text, IDs, or paths", () => {
  const input = user("private-session-id");
  input.cwd = "/private/fixture-project";
  input.message.content = "PRIVATE_PROMPT_MARKER";
  const reply = assistant("private-reply-id", input.uuid);
  reply.message.content[0].text = "PRIVATE_REPLY_MARKER";
  const output = JSON.stringify(extractModelInfo("claude", [input, reply]));
  for (const omitted of [input.uuid, reply.uuid, input.cwd, input.message.content, reply.message.content[0].text]) {
    assert.ok(!output.includes(omitted));
  }
});

test("The file reader is bounded and rejects symlinks and malformed records", async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "mallard-model-fields-test-")));
  try {
    const file = join(root, "fixture.jsonl");
    await writeFile(file, `${JSON.stringify(user("u1"))}\n`);
    assert.equal((await readRecords(file)).length, 1);
    const link = join(root, "link.jsonl");
    await symlink(file, link);
    await assert.rejects(readRecords(link), /canonical_path_required/);
    await writeFile(file, '{"unfinished":');
    await assert.rejects(readRecords(file), /invalid_json/);
    await writeFile(file, " ".repeat(16 * 1024 * 1024 + 1));
    await assert.rejects(readRecords(file), /input_not_admitted/);
    assert.throws(() => extractModelInfo("claude", [null]), /invalid_record/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
