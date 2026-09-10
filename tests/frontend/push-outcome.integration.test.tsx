import assert from "node:assert/strict";
import test from "node:test";
import type {
  ConversationReviewAction,
  ProjectOperationResult,
  ThreadSyncComparison,
  ThreadSyncDecision,
  ThreadSyncEntry,
} from "../../src/types";
import { buildPushOutcome } from "../../src/components/project-sync/syncOutcome";

function decision(action: ConversationReviewAction, selectable: boolean): ThreadSyncDecision {
  return { action, selectable };
}

function entry(id: string, title: string, overrides: Partial<ThreadSyncEntry> = {}): ThreadSyncEntry {
  return {
    provider: "claude",
    provider_display_name: "Claude",
    access: "bound_ready",
    thread_id: id,
    conversation_resource_id: `claude:conversation:${id}`,
    resource_id: `claude:conversation:${id}`,
    display_name: title,
    state: "local_only",
    local_present: true,
    storage_present: false,
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: id,
    push_decision: decision("create_cloud", true),
    pull_decision: decision("report_local_only", false),
    ...overrides,
  };
}

function comparison(entries: ThreadSyncEntry[]): ThreadSyncComparison {
  return {
    project_id: "project-1",
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    compared_at: 1_752_804_000,
    entries,
    counts: { synced: 0, local: entries.length, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    remote_continuity: { state: "in_sync", revision: 1 },
  };
}

const selected = new Set(["claude:conversation:good", "claude:conversation:torn"]);
const threads = comparison([entry("good", "Good thread"), entry("torn", "Torn thread")]);

test("a push outcome lists the thread the backend could not publish", () => {
  const result: ProjectOperationResult = {
    success: false,
    message: "Published revision 2 — 1 session not pushed",
    revision: 2,
    resources_changed: 1,
    results: [
      { resource_id: "claude:cloud-conversation:minted", state: "synced" },
      {
        resource_id: "claude:conversation:torn",
        state: "failed",
        message: "conversation 'torn' does not end at a record boundary; refresh and push again",
      },
    ],
  };
  const outcome = buildPushOutcome({
    projectId: "project-1",
    storageId: "storage-1",
    selectedResourceIds: selected,
    threadComparison: threads,
    result,
  });

  assert.equal(outcome.title, "Pushed 1 of 2 sessions");
  assert.equal(outcome.tone, "warning");
  assert.equal(outcome.defaultOpen, true);
  assert.deepEqual(
    outcome.rows.map((row) => [row.status, row.title]),
    [["synced", "Good thread"], ["failed", "Torn thread"]],
  );
  assert.equal(
    outcome.rows[1].reason,
    "conversation 'torn' does not end at a record boundary; refresh and push again",
  );
  assert.equal(outcome.rows[1].threadId, "torn");
  assert.deepEqual(outcome.actions, []);
  assert.ok(outcome.stats.some((stat) => stat.label === "not pushed" && stat.value === "1"));
  assert.ok(outcome.stats.some((stat) => stat.label === "pushed" && stat.value === "1"));
});

test("a thread the apply-time review held for Pull offers the Pull review", () => {
  const result: ProjectOperationResult = {
    success: true,
    message: "Published revision 3",
    revision: 3,
    resources_changed: 1,
    results: [
      {
        resource_id: "claude:conversation:torn",
        state: "held",
        message: "Storage has changes this Push did not include. Pull first, then push again.",
      },
    ],
  };
  const outcome = buildPushOutcome({
    projectId: "project-1",
    storageId: "storage-1",
    selectedResourceIds: selected,
    threadComparison: threads,
    result,
  });

  assert.equal(outcome.title, "Pushed 1 of 2 sessions");
  assert.deepEqual(
    outcome.rows.map((row) => [row.status, row.title]),
    [["synced", "Good thread"], ["held", "Torn thread"]],
  );
  assert.deepEqual(outcome.actions, [{ kind: "open_pull", label: "Open Pull review" }]);
  assert.ok(outcome.stats.some((stat) => stat.label === "held for Pull" && stat.value === "1"));
});

test("a push with nothing to report keeps its plain face", () => {
  const result: ProjectOperationResult = {
    success: true,
    message: "Published revision 2",
    revision: 2,
    resources_changed: 2,
    results: [
      { resource_id: "claude:cloud-conversation:one", state: "synced" },
      { resource_id: "claude:cloud-conversation:two", state: "synced" },
    ],
  };
  const outcome = buildPushOutcome({
    projectId: "project-1",
    storageId: "storage-1",
    selectedResourceIds: selected,
    threadComparison: threads,
    result,
  });

  assert.equal(outcome.title, "Push complete");
  assert.equal(outcome.tone, "success");
  assert.equal(outcome.defaultOpen, false);
  assert.deepEqual(outcome.rows.map((row) => row.status), ["synced", "synced"]);
  assert.ok(!outcome.stats.some((stat) => stat.label === "not pushed"));
});
