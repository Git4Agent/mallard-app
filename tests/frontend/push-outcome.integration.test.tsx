import assert from "node:assert/strict";
import test from "node:test";
import type {
  ConversationReviewAction,
  ProjectOperationResult,
  ThreadSyncComparison,
  ThreadSyncDecision,
  ThreadSyncEntry,
} from "../../src/types";
import { renderToStaticMarkup } from "react-dom/server";
import { NoticeZone } from "../../src/components/notice/Notice";
import { pushFailureNotice } from "../../src/components/project-sync/pushFailureNotice";
import { buildPushOutcome } from "../../src/components/project-sync/syncOutcome";
import { storageAccessErrorMessage } from "../../src/storageError";
import { i18n } from "../../src/i18n";
import {
  isPushFinished,
  pushFinished,
  finishPushOffered,
  ownUnfinishedPushSentence,
  unfinishedPushSentence,
} from "../../src/unfinishedPush";

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
    message: "Published version 2 — 1 session not pushed",
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
    message: "Published version 3",
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
    message: "Published version 2",
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

test("a changed storage and an unconfirmed push reach the shell banner word for word", () => {
  // The backend sends these two sentences as the Push error. Nothing between
  // the command and the banner may rewrite them, and the banner prints the
  // severity word itself, so neither sentence names one.
  const storage = { provider_label: "Cloudflare R2" };
  for (const sentence of [
    "The storage changed while pushing. Compare again.",
    "Couldn't confirm the push. Refresh to see whether it arrived.",
    unfinishedPushSentence("Attic R2"),
    ownUnfinishedPushSentence("Attic R2"),
  ]) {
    assert.equal(storageAccessErrorMessage(storage, sentence), sentence);
    assert.equal(storageAccessErrorMessage(storage, new Error(sentence)), sentence);
    assert.doesNotMatch(sentence, /\b(error|warning|note|done)\b/i);
    const markup = renderToStaticMarkup(
      <NoticeZone
        items={[pushFailureNotice(sentence, "Attic R2", { busy: false, onFinishPush: () => undefined, onDismiss: () => undefined })]}
        subject="messages from Mallard"
        className="sync-backend-banner"
        timing="blocking"
      />,
    );
    assert.ok(markup.includes(sentence.replace("'", "&#x27;")), markup);
    assert.match(markup, /Error/);
  }
});

test("only the two unfinished-push sentences, for this storage, offer Finish that push", () => {
  assert.equal(unfinishedPushSentence("Attic R2"), "A push to Attic R2 did not finish. Finish it, then compare again.");
  assert.equal(
    ownUnfinishedPushSentence("Attic R2"),
    "This computer's earlier push to Attic R2 did not finish. Finish it, then compare again.",
  );
  assert.equal(finishPushOffered(unfinishedPushSentence("Attic R2"), "Attic R2"), true);
  assert.equal(finishPushOffered(ownUnfinishedPushSentence("Attic R2"), "Attic R2"), true);
  assert.equal(finishPushOffered(unfinishedPushSentence("Attic R2"), "Other storage"), false);
  assert.equal(finishPushOffered("The storage changed while pushing. Compare again.", "Attic R2"), false);
  assert.equal(finishPushOffered(pushFinished(), "Attic R2"), false);
  assert.equal(finishPushOffered(null, "Attic R2"), false);
  assert.doesNotMatch(pushFinished(), /\b(error|warning|note|done)\b/i);
  // The done tone follows the sentence in either language, never another one.
  assert.equal(isPushFinished(pushFinished()), true);
  assert.equal(isPushFinished("已完成。请重新比较。"), true);
  assert.equal(isPushFinished("The storage changed while pushing. Compare again."), false);
});

test("a push outcome in Chinese keeps the backend's reason and the thread titles as written", () => {
  const reason = "conversation 'torn' does not end at a record boundary; refresh and push again";
  const result: ProjectOperationResult = {
    success: false,
    message: "Published version 2 — 1 session not pushed",
    revision: 2,
    resources_changed: 1,
    results: [{ resource_id: "claude:conversation:torn", state: "failed", message: reason }],
  };
  const build = () => buildPushOutcome({
    projectId: "project-1",
    storageId: "storage-1",
    selectedResourceIds: selected,
    threadComparison: threads,
    result,
  });
  void i18n.changeLanguage("zh-CN");
  try {
    const outcome = build();
    assert.equal(outcome.title, "已上传 1/2 个会话");
    assert.deepEqual(outcome.stats.map((stat) => [stat.label, stat.value]), [["已上传", "1"], ["未上传", "1"], ["Version", "2"]]);
    assert.deepEqual(outcome.rows.map((row) => [row.title, row.reason ?? null]), [["Good thread", null], ["Torn thread", reason]]);
    assert.deepEqual(outcome.technical, [{ label: "消息", value: "Published version 2 — 1 session not pushed" }]);
    const clean = buildPushOutcome({
      projectId: "project-1",
      storageId: "storage-1",
      selectedResourceIds: new Set(["claude:conversation:good"]),
      threadComparison: threads,
      result: { success: true, message: "", revision: 3, resources_changed: 1, results: [] },
    });
    assert.equal(clean.title, "上传完成");
    assert.deepEqual(clean.stats[0], { icon: "upload", value: "1", label: "个会话" });
  } finally {
    void i18n.changeLanguage("en");
  }
  assert.equal(build().title, "Pushed 1 of 2 sessions");
});
