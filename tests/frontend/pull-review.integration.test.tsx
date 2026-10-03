import { projectOutcomeAction } from "../../src/components/project-sync/syncOutcome";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  BundleReadiness,
  ConversationBlocker,
  ConversationReviewAction,
  ConversationReviewRow,
  LocalProjectSummary,
  ProjectBinding,
  PullReview,
  RestoreResult,
} from "../../src/types";
import RestorePlanView, {
  pullSelectableResourceIds,
} from "../../src/components/project-sync/RestorePlanView";
import SyncOutcomeNotice from "../../src/components/sync-review/SyncOutcomeNotice";
import {
  buildPullOutcome,
  type SyncOutcome,
} from "../../src/components/project-sync/syncOutcome";
import { REVIEW_EXPIRED, REVIEW_SUPERSEDED, compareAgainOffered } from "../../src/components/project-sync/pullReviewFlow";
import {
  buildPullReviewItems,
  buildPullReviewSelection,
  fitPullRecommendation,
  pullActionResourceIds,
  pullBudgetTotal,
  recommendedPullSelection,
} from "../../src/components/project-sync/pullReviewModel";
import { ReviewBudgetGlance, reviewBudgetSentence } from "../../src/components/sync-review/ReviewBudgetMeter";
import {
  applyPullReview,
  beginPullReview,
  canAcceptPullMetadata,
  acceptPullMetadata,
  type PullReviewApi,
} from "../../src/components/project-sync/pullReviewFlow";

function renderOutcome(outcome: SyncOutcome) {
  return renderToStaticMarkup(
    <SyncOutcomeNotice presentAction={projectOutcomeAction}
      outcome={outcome}
      onAction={() => undefined}
      onDismiss={() => undefined}
    />,
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve;
  });
  return { promise, resolve };
}

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica-gam2",
  local_project_id: "project-gam2",
  bundle_id: "df29babc833808e68ad0efa4d01d7d6d",
  project_root: "/Users/test/project/gam2",
  canonical_project_root: "/Users/test/project/gam2",
  profile_ids: { codex: "profile-codex" },
  state: "active",
  revision: 0,
  updated_at: 1,
};

const localThreadId = "019fac21-4aba-7fc2-bb7d-df711c950987";
const cloudThreadId = "cloudthread-019fac21-4aba-7fc2-bb7d-df711c950987";
const preservedThreadId = "019fac21-4aba-7fc2-bb7d-df711c950988";
const preservedDisplayTitle = "Diverged conversation (Preserved 1 · Machine A)";

const MB = 1024 * 1024;
const LIMIT = 512 * MB;

const restorePlan: PullReview = {
  apply_limit_bytes: LIMIT,
  schema_version: 1,
  plan_id: "plan-restore",
  storage_id: "storage-local",
  bundle_id: binding.bundle_id,
  host_id: binding.host_id,
  replica_id: binding.replica_id,
  expected_host_identity_revision: 0,
  revision: 1,
  commit_id: "commit-restore",
  manifest_sha256: "a".repeat(64),
  binding_revision: binding.revision,
  created_at: 1,
  expires_at: 2,
  conversation_review: [],
  actions: [{
    action_id: "action-session",
    resource_id: "codex:session:thread-a",
    kind: {
      kind: "materialize_conversation",
      provider: "codex",
      logical_path: "state/codex/sessions/2026/07/18/thread-a.jsonl",
    },
    target_path: "/Users/test/.codex/sessions/2026/07/18/thread-a.jsonl",
    source_sha256: "b".repeat(64),
    expected_target_sha256: null,
    requires_explicit_approval: false,
  }],
};

function conversationRow(
  action: ConversationReviewAction,
  actionId: string | null = null,
  blocker: ConversationBlocker | null = null,
  displayTitle: string | null = null,
  sourceSize: number | null = null,
  storedEndedAt: number | null = null,
): ConversationReviewRow {
  return {
    stored_without_credential_redaction: false,
    provider: "codex",
    thread_id: localThreadId,
    cloud_thread_id: cloudThreadId,
    resource_id: `codex:conversation:${localThreadId}`,
    action,
    action_id: actionId,
    blocker,
    display_title: displayTitle,
    source_size: sourceSize,
    stored_ended_at: storedEndedAt,
  };
}

/** A storage-only session of its own identity, sized and timed. */
function storedSession(key: string, sourceSize: number, storedEndedAt: number): ConversationReviewRow {
  return {
    ...conversationRow("create_local", `selection-${key}`, null, `Stored ${key}`, sourceSize, storedEndedAt),
    thread_id: null,
    cloud_thread_id: `cloudthread-${key}`,
    resource_id: `codex:conversation:cloudthread-${key}`,
  };
}

function result(overrides: Partial<RestoreResult> = {}): RestoreResult {
  return {
    success: true,
    message: "Applied restore actions",
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    readiness_error: null,
    plan_id: restorePlan.plan_id,
    reviewed_base_advanced: true,
    applied_action_ids: [],
    failed_actions: [],
    preserved_threads: [],
    after_pull_actions: [],
    not_pulled_action_ids: [],
    ...overrides,
  };
}

function renderReview(
  plan: PullReview,
  error: string | null = null,
  onRefreshPlan?: () => void,
  phase: "idle" | "restoring" = "idle",
) {
  return renderToStaticMarkup(
    <RestorePlanView onSessionActionError={() => undefined}
      projectName="gam2"
      storageName="Local storage 1"
      plan={plan}
      binding={binding}
      phase={phase}
      supportLoading={false}
      busy={false}
      error={error}
      onDismissError={() => undefined}
      onApply={() => undefined}
      onBack={() => undefined}
      onRefreshPlan={onRefreshPlan}
    />,
  );
}

test("the review opens with nothing ticked and its primary says so", () => {
  const html = renderReview(restorePlan);

  // Nothing is selected at open, so the apply is the acknowledge-only one:
  // the verb must not promise a Pull that restores nothing.
  const primary = html.indexOf("sync-pull-apply-button");
  assert.ok(primary >= 0);
  const footer = html.slice(primary);
  assert.match(footer, /Mark as reviewed<\/button>/);
  assert.doesNotMatch(footer, />Pull<\/button>|Pull \d+<\/button>/);
});

test("a Pull of a storage that was cleared and pushed again is an ordinary first Pull", () => {
  const html = renderReview({
    ...restorePlan,
    remote_continuity: { state: "replaced", base_revision: 1, remote_revision: 2 },
  });

  // No banner about the storage's history, and no unlink or reset verb.
  assert.doesNotMatch(html, /Storage history changed|was reset|Another computer cleared/);
  assert.doesNotMatch(html, /Unlink storage/);
  assert.match(html, /sync-pull-apply-button/);
  const primary = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[\s\S]*?<\/button>/)?.[0] ?? "";
  assert.match(primary, /data-thread-status="cloud-outstanding"/);
  assert.doesNotMatch(primary, /data-icon="download"/);
});

test("keep-both is one reviewed action with no second resolution payload", () => {
  const plan: PullReview = {
    ...restorePlan,
    actions: [],
    conversation_review: [conversationRow("keep_both", "conversation-keep-both")],
  };
  const items = buildPullReviewItems(plan);
  const selected = new Set(items.map((item) => item.resourceId));
  const selection = buildPullReviewSelection(items, selected);

  assert.deepEqual(selection, {
    resourceIds: [`codex:conversation:${localThreadId}`],
    restoreActionIds: ["conversation-keep-both"],
  });
  assert.deepEqual(
    pullSelectableResourceIds(items),
    selected,
  );

  const html = renderReview(plan);
  assert.match(html, /Not selected/);
  assert.match(html, /Select Conversation/);
  assert.match(html, /class="btn sync-sync-review-select"[^>]*>.*?Select <span class="sync-path-count">1<\/span>/);
  assert.doesNotMatch(html, /radiogroup|Fork and overwrite|Discard the local branch/);
});

test("a mixed Pull starts empty and selecting one project file restores only that file", () => {
  const conversation = conversationRow("materialize_cloud", "selection-materialize", null, "Stored conversation");
  const plan: PullReview = {
    ...restorePlan,
    project_content_eligibility: {
      state: "eligible",
      reason: "Not inside a Git work tree.",
    },
    actions: [{
      action_id: "write-ghost-story",
      resource_id: "file-ghost-story",
      kind: {
        kind: "write_project_file",
        logical_path: "project/ghost_story.docx",
        mode: 0o644,
        source_mtime: 1,
      },
      target_path: "/Users/test/project/gam2/ghost_story.docx",
      source_sha256: "d".repeat(64),
      expected_target_sha256: null,
      requires_explicit_approval: true,
    }],
    conversation_review: [
      conversation,
    ],
  };
  const items = buildPullReviewItems(plan);
  const recommendedIds = items
    .filter(recommendedPullSelection)
    .map((item) => item.resourceId);
  // The stored-only file is an add — nothing local replaced — so the files
  // step's Recommended now offers it alongside the conversation.
  assert.deepEqual(recommendedIds, [conversation.resource_id, "file-ghost-story"]);

  const html = renderReview(plan);
  assert.match(html, /aria-label="Sessions, 0 selected"/);
  assert.match(html, /aria-label="Project files, 0 selected"/);
  assert.match(html, /class="btn sync-sync-review-select"[^>]*>.*?Select <span class="sync-path-count">1<\/span>/);

  const selection = buildPullReviewSelection(
    items,
    new Set(["file-ghost-story"]),
  );
  assert.deepEqual(selection, {
    resourceIds: ["file-ghost-story"],
    restoreActionIds: ["write-ghost-story"],
  });

  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan,
    result: result({ applied_action_ids: ["write-ghost-story"] }),
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items,
    completedResourceIds: new Set(["file-ghost-story"]),
  });
  assert.deepEqual(outcome.stats[0], {
    icon: "download",
    value: "1",
    label: "restored",
  });
  assert.deepEqual(outcome.rows.map((row) => row.title), ["ghost_story.docx"]);
});

test("blocked conversation evidence stays out of Pull review", () => {
  const plan = {
    ...restorePlan,
    actions: [],
    conversation_review: [conversationRow("blocked", null, "unknown_evidence")],
  };
  const items = buildPullReviewItems(plan);

  assert.deepEqual(items, []);
  assert.deepEqual(pullSelectableResourceIds(items), new Set());
  const html = renderReview(plan);
  assert.doesNotMatch(html, /Can&#x27;t sync/);
  assert.match(html, /Nothing to pull/);
  assert.doesNotMatch(html, /Select Conversation/);
});

/* The row's name comes from the stored thread, and when the stored thread has
   none the row says so. It must never be named after its id: a CloudThreadId's
   first characters are the mandatory "cloudthread-" namespace, so every
   untitled cloud row would carry the identical name — which is what the
   deleted `Task ${threadId.slice(0, 8)}` fallback did. */
test("a restored row is named by its stored title, or reads untitled", () => {
  const materialize = conversationRow("materialize_cloud", "selection-materialize");
  const restoredFor = (displayTitle: string | null) => {
    const plan = {
      ...restorePlan,
      actions: [],
      conversation_review: [
        conversationRow("materialize_cloud", materialize.action_id, null, displayTitle),
      ],
    };
    return buildPullOutcome({
      projectId: binding.local_project_id,
      plan,
      result: result({ applied_action_ids: [materialize.action_id!] }),
      readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
      items: buildPullReviewItems(plan),
      completedResourceIds: new Set([materialize.resource_id]),
    });
  };

  const titled = restoredFor("Can you create a pelican-riding-a-bicycle svg");
  assert.equal(titled.rows[0].status, "synced");
  assert.equal(titled.rows[0].title, "Can you create a pelican-riding-a-bicycle svg");

  const untitled = restoredFor(null);
  assert.equal(untitled.rows[0].title, "Untitled session");
  // Neither the id-derived stub nor the bare kind label may name a row.
  assert.doesNotMatch(untitled.rows[0].title, /^Task /);
  assert.notEqual(untitled.rows[0].title, "Conversation");
  // The whole identifier still rides line 2.
  assert.equal(untitled.rows[0].threadId, materialize.thread_id);
});

test("keep-both completion reports the independent session and the provider's after-pull offer", () => {
  const action = conversationRow("keep_both", "conversation-keep-both");
  const plan = {
    ...restorePlan,
    actions: [],
    conversation_review: [action],
  };
  const completed = result({
    applied_action_ids: [action.action_id!],
    preserved_threads: [{
      action_id: action.action_id!,
      original_thread_id: action.thread_id!,
      preserved_thread_id: preservedThreadId,
      preserved_display_title: preservedDisplayTitle,
    }],
    after_pull_actions: [{
      provider: "codex",
      action_id: "codex.app.restart",
      label: "Restart Codex",
      message: "Restart Codex to refresh its task list.",
      available: true,
      arguments: { operation_id: "operation-materialize", app_kind: "codex" },
    }],
  });
  const items = buildPullReviewItems(plan);
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan,
    result: completed,
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items,
    completedResourceIds: new Set([action.resource_id]),
  });

  assert.equal(outcome.rows[0].status, "preserved");
  assert.equal(outcome.rows[0].threadId, preservedThreadId);
  // The row is named by the marker title the copy was renamed to.
  assert.equal(outcome.rows[0].title, "Diverged conversation (Preserved 1 · Machine A)");
  // Label and title are the offer's own words; the offer rides along so the
  // shell can hand it straight back to the generic maintenance command.
  assert.deepEqual(outcome.actions, [{
    kind: "after_pull_action",
    label: "Restart Codex",
    title: "Restart Codex to refresh its task list.",
    offer: completed.after_pull_actions[0],
  }]);

  const html = renderOutcome(outcome);
  assert.match(html, /Kept independently/);
  assert.match(html, /previous local history was kept as an independent session/);
  assert.doesNotMatch(html, /native fork|Retry fork/i);
});

test("Pull reports 35 restored and 3 failed threads from Review selection IDs", async () => {
  const rows: ConversationReviewRow[] = Array.from({ length: 38 }, (_, index) => ({
    stored_without_credential_redaction: false,
    provider: "codex",
    thread_id: null,
    cloud_thread_id: `cloudthread-fixture-${index}`,
    resource_id: `codex:conversation:cloudthread-fixture-${index}`,
    action: "create_local",
    action_id: `selection-fixture-${index}`,
    display_title: index < 35 ? `Restored thread ${index}` : "Hello from the second machine",
  }));
  const plan: PullReview = {
    ...restorePlan,
    actions: [],
    conversation_review: rows,
  };
  const items = buildPullReviewItems(plan);
  const selection = buildPullReviewSelection(
    items,
    new Set(items.map((item) => item.resourceId)),
  );
  const failedRows = rows.slice(35);
  const completed = result({
    success: false,
    message: "Applied 35 actions; 3 require attention",
    readiness: {
        bundle_id: binding.bundle_id,
        state: "blocked",
        issues: [{
          issue_id: "restore-project-data",
          category: "project_files",
          title: "3 project data changes were left unchanged",
          severity: "error",
        }],
      },
    reviewed_base_advanced: false,
    applied_action_ids: rows.slice(0, 35).map((row) => row.action_id!),
    failed_actions: failedRows.map((row) => ({
      action_id: row.action_id!,
      display_name: null,
      message: "Wire record failed its round-trip gate",
    })),
  });
  const applied = await applyPullReview({
    async applyRestore(review, actionIds) {
      assert.equal(review, plan);
      assert.deepEqual(new Set(actionIds), new Set(rows.map((row) => row.action_id)));
      return completed;
    },
  }, plan, selection, () => undefined);
  const resourceByAction = pullActionResourceIds(plan);
  const completedResourceIds = new Set(completed.applied_action_ids
    .map((actionId) => resourceByAction.get(actionId)!));
  const failedResourceIds = new Set(completed.failed_actions
    .map((failure) => resourceByAction.get(failure.action_id)!));
  assert.deepEqual(completedResourceIds, new Set(rows.slice(0, 35).map((row) => row.resource_id)));
  assert.deepEqual(failedResourceIds, new Set(failedRows.map((row) => row.resource_id)));

  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan,
    result: applied.restoreResult!,
    readiness: applied.readiness,
    items,
    completedResourceIds,
    error: applied.error,
  });
  assert.equal(outcome.title, "3 changes need attention");
  assert.deepEqual(outcome.stats, [
    { icon: "download", value: "35", label: "restored" },
    { icon: "octagon-x", value: "3", label: "failed" },
    { icon: "git-branch", label: "Version", value: "1", lead: true },
  ]);
  assert.equal(outcome.rows.filter((row) => row.status === "synced").length, 35);
  assert.deepEqual(outcome.rows.filter((row) => row.status === "failed"), failedRows.map((row) => ({
    key: row.action_id,
    title: row.display_title,
    status: "failed",
    reason: "Wire record failed its round-trip gate",
    threadId: row.cloud_thread_id,
  })));
  assert.equal(outcome.rows.filter((row) => row.title === "Push is still blocked").length, 1);
  assert.deepEqual(outcome.actions, [{ kind: "open_pull", label: "Open Pull review" }]);
  const html = renderOutcome(outcome);
  assert.match(html, /3 changes need attention/);
  assert.match(html, /Hello from the second machine/);
  assert.doesNotMatch(html, /selection-fixture-|3 project data changes were left unchanged/);
});

test("one unnamed failed thread has one failure and a separate Push hold", () => {
  const row = conversationRow("keep_both", "selection-unnamed", null, "  ");
  const plan: PullReview = {
    ...restorePlan,
    actions: [],
    conversation_review: [row],
  };
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan,
    result: result({
      success: false,
      reviewed_base_advanced: false,
      failed_actions: [{ action_id: row.action_id!, display_name: null, message: "Source changed" }],
    }),
    readiness: null,
    items: buildPullReviewItems(plan),
    completedResourceIds: new Set(),
  });
  assert.equal(outcome.title, "1 change needs attention");
  assert.equal(outcome.stats.find((stat) => stat.label === "failed")?.value, "1");
  assert.equal(outcome.rows[0].title, "Untitled session");
  assert.equal(outcome.rows[0].threadId, localThreadId);
  assert.equal(outcome.rows[1].status, "held");
  assert.equal(outcome.rows[1].title, "Push is still blocked");
});

test("an empty acknowledge-only apply is titled as a mark-as-reviewed, not a Pull", () => {
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: result({ reviewed_base_advanced: true, applied_action_ids: [] }),
    readiness: null,
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(),
  });
  assert.equal(outcome.title, "Nothing pulled — storage marked as reviewed");
  assert.equal(outcome.tone, "success");
});

test("a Push hold without an action failure keeps Pull available without a failure count", () => {
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: result({ reviewed_base_advanced: false }),
    readiness: null,
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(),
  });
  assert.equal(outcome.title, "Pull complete — Push still blocked");
  assert.equal(outcome.tone, "warning");
  assert.equal(outcome.defaultOpen, true);
  assert.equal(outcome.stats.some((stat) => stat.label === "failed"), false);
  assert.deepEqual(outcome.actions, [{ kind: "open_pull", label: "Open Pull review" }]);
});

test("independent readiness errors stay visible without adding to failed action counts", () => {
  const applied = result({ applied_action_ids: ["action-session"] });
  const readiness: BundleReadiness = {
    bundle_id: binding.bundle_id,
    state: "needs_setup",
    issues: [{
      issue_id: "provider-home-codex",
      category: "sessions",
      title: "Codex needs setup",
      detail: "Select a Codex profile.",
      severity: "error",
    }],
  };
  const outcomeFor = (restoreResult: RestoreResult) => buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: restoreResult,
    readiness,
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(["codex:session:thread-a"]),
  });
  const setupOnly = outcomeFor(applied);
  assert.equal(setupOnly.title, "1 change needs attention");
  assert.equal(setupOnly.stats.some((stat) => stat.label === "failed"), false);
  assert.equal(setupOnly.rows[1].title, "Codex needs setup");
  assert.deepEqual(setupOnly.actions, [{ kind: "recheck", label: "Recheck" }]);

  const withFailure = outcomeFor(result({
    success: false,
    failed_actions: [{ action_id: "action-other", display_name: "Other thread", message: "Write failed" }],
  }));
  assert.equal(withFailure.title, "2 changes need attention");
  assert.equal(withFailure.stats.find((stat) => stat.label === "failed")?.value, "1");
  assert.equal(withFailure.rows.some((row) => row.title === "Codex needs setup"), true);
});

test("a refused review offers Compare again, and no Pull surface offers a reset", () => {
  for (const sentence of [REVIEW_SUPERSEDED, REVIEW_EXPIRED]) {
    const html = renderReview(restorePlan, `Restore failed: ${sentence}`, () => undefined);
    assert.match(html, /Compare again<\/button>/);
    assert.match(html, new RegExp(sentence.replace(/[.]/g, "\\.")));
    assert.doesNotMatch(html, /Reset local sync state/);
  }
  // Any other apply error keeps the banner without that button.
  const other = renderReview(restorePlan, "Restore failed: storage is temporarily unavailable", () => undefined);
  assert.doesNotMatch(other, /Compare again<\/button>/);
  // Every failure closes: the review stays open under it.
  for (const html of [renderReview(restorePlan, `Restore failed: ${REVIEW_SUPERSEDED}`, () => undefined), other]) {
    assert.equal(html.match(/class="sync-notice-dismiss"/g)?.length, 1, "one close control, drawn by the zone");
    assert.match(html, /class="sync-notice-dismiss"[^>]*aria-label="Dismiss: Error\. Restore failed/);
  }
  assert.doesNotMatch(other, /Reset local sync state/);
  assert.equal(compareAgainOffered(`Restore failed: ${REVIEW_SUPERSEDED}`), true);
  assert.equal(compareAgainOffered("storage is temporarily unavailable"), false);
});

test("a stopped Pull says so first, lists what it did not reach, and never says Push is blocked", () => {
  const sentence = "Part of this version is missing from Local storage 1. Compare again.";
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: result({
      success: false,
      message: sentence,
      stopped: sentence,
      reviewed_base_advanced: false,
      not_pulled_action_ids: ["action-a"],
    }),
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(),
  });
  assert.equal(outcome.title, "Pull stopped");
  assert.equal(outcome.message, sentence);
  assert.deepEqual(outcome.rows.map((row) => row.status), ["not_pulled"]);
  assert.equal(outcome.stats.find((stat) => stat.label === "not pulled")?.value, "1");
  assert.equal(outcome.rows.some((row) => row.title === "Push is still blocked"), false);
  assert.deepEqual(outcome.actions, [{ kind: "open_pull", label: "Open Pull review" }]);
  const html = renderOutcome(outcome);
  assert.match(html, /Pull stopped/);
  assert.match(html, /Part of this version is missing from Local storage 1\. Compare again\./);
  assert.match(html, /Not pulled/);
  assert.doesNotMatch(html, /Push is still blocked|Reset local sync state/);
});

test("a session that failed alone prints its sentence and offers no reset", () => {
  const sentence = "This stored session cannot be read by this version of Mallard.";
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: result({
      success: false,
      failed_actions: [{ action_id: "action-a", display_name: "Task A", message: sentence }],
    }),
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(),
  });
  assert.equal(outcome.rows.find((row) => row.status === "failed")?.reason, sentence);
  assert.deepEqual(outcome.actions, [{ kind: "open_pull", label: "Open Pull review" }]);
  const html = renderOutcome(outcome);
  assert.match(html, /This stored session cannot be read by this version of Mallard\./);
  assert.doesNotMatch(html, /Reset local sync state/);
});

test("a Pull review renders before readiness support resolves", async () => {
  const readiness = deferred<BundleReadiness>();
  const api: PullReviewApi = {
    async planRestore() {
      return restorePlan;
    },
    getRestoreReadiness() {
      return readiness.promise;
    },
  };

  const review = await beginPullReview(api, {
    storageId: restorePlan.storage_id,
    bundleId: restorePlan.bundle_id,
    binding,
  });
  assert.equal(review.restorePlan, restorePlan);

  readiness.resolve({ bundle_id: binding.bundle_id, state: "ready", issues: [] });
  assert.deepEqual(await review.support, {
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    errors: [],
  });
});

test("readiness failure does not hide a valid mutation review", async () => {
  const review = await beginPullReview({
    async planRestore() {
      return restorePlan;
    },
    async getRestoreReadiness() {
      throw new Error("profile unavailable");
    },
  }, {
    storageId: restorePlan.storage_id,
    bundleId: restorePlan.bundle_id,
    binding,
  });

  assert.equal(review.restorePlan, restorePlan);
  assert.deepEqual(await review.support, {
    readiness: null,
    errors: ["Readiness checks: profile unavailable"],
  });
  assert.match(renderReview(restorePlan, "Readiness checks: profile unavailable"), /sync-pull-apply-button/);
});

test("the Apply coordinator uses returned readiness without another request", async () => {
  const calls: unknown[][] = [];
  const phases: string[] = [];
  const ready: BundleReadiness = { bundle_id: binding.bundle_id, state: "ready", issues: [] };
  const applied = result({ applied_action_ids: ["action-session"], readiness: ready });

  const appliedResult = await applyPullReview({
    async applyRestore(review, actionIds) {
      calls.push([review, actionIds]);
      return applied;
    },
  }, restorePlan, {
    resourceIds: ["codex:session:thread-a"],
    restoreActionIds: ["action-session"],
  }, (phase) => phases.push(phase));

  assert.deepEqual(calls, [
    [restorePlan, ["action-session"]],
  ]);
  assert.deepEqual(phases, ["restoring", "idle"]);
  assert.equal(appliedResult.success, true);
});

test("the Apply coordinator rejects action ids outside the comparison", async () => {
  await assert.rejects(() => applyPullReview({
    async applyRestore() {
      throw new Error("must not execute");
    },
  }, restorePlan, {
    resourceIds: [],
    restoreActionIds: ["not-reviewed"],
  }, () => undefined), /no longer belong to this Pull review/);
});

test("after-pull offers keep their order; available actions run and unavailable actions stay prose", () => {
  const action = conversationRow("keep_both", "conversation-keep-both");
  const plan = {
    ...restorePlan,
    actions: [],
    conversation_review: [action],
  };
  const items = buildPullReviewItems(plan);
  const readiness: BundleReadiness = { bundle_id: binding.bundle_id, state: "ready", issues: [] };
  const afterPullActions = [{
    provider: "acme",
    action_id: "acme.app.restart",
    label: "Restart Acme Desktop",
    message: "Restart Acme Desktop to show the restored sessions.",
    available: true,
    arguments: { operation_id: "operation-materialize" },
  }, {
    provider: "acme",
    action_id: "acme.cli.refresh",
    label: "Refresh Acme CLI",
    message: "Run acme refresh by hand to update its session index.",
    available: false,
    arguments: { operation_id: "operation-materialize" },
  }, {
    provider: "other",
    action_id: "other.app.reload",
    label: "Reload Other Desktop",
    message: "Reload Other Desktop to show the restored sessions.",
    available: true,
    arguments: { operation_id: "operation-materialize" },
  }];
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan,
    result: result({
      applied_action_ids: [action.action_id!],
      after_pull_actions: afterPullActions,
    }),
    readiness,
    items,
    completedResourceIds: new Set([action.resource_id]),
  });

  assert.deepEqual(outcome.actions.map((candidate) => (
    candidate.kind === "after_pull_action"
      ? [candidate.label, candidate.offer.available]
      : [candidate.label, true]
  )), [
    ["Restart Acme Desktop", true],
    ["Refresh Acme CLI", false],
    ["Reload Other Desktop", true],
  ]);
  const html = renderOutcome(outcome);
  assert.match(html, />Restart Acme Desktop<\/button>/);
  assert.match(html, />Reload Other Desktop<\/button>/);
  assert.match(html, /title="Restart Acme Desktop to show the restored sessions\."/);
  assert.match(html, /Run acme refresh by hand to update its session index\./);
  assert.doesNotMatch(html, />Refresh Acme CLI<\/button>/);

  // The shell follows the declared confirmation policy, then hands the exact
  // offer back to the generic maintenance command.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /executeProviderMaintenanceAction\(\{\s*provider: action\.offer\.provider,\s*action_id: action\.offer\.action_id,\s*arguments: action\.offer\.arguments,\s*confirmed,\s*\}\)/);
  assert.match(shell, /const descriptor = maintenanceActionFor\(/);
  assert.match(shell, /descriptor\.requires_confirmation/);
  assert.match(shell, /const remainingActions = outcome\.actions\.filter/);
  assert.doesNotMatch(shell, /interrupted mid-turn|Close the affected/);
  assert.doesNotMatch(shell, /CodexApp|codex_app|restart_codex/);
  const model = readFileSync("src/components/project-sync/syncOutcome.ts", "utf8");
  assert.match(model, /result\.after_pull_actions\.map\(\(offer\) =>/);
  assert.doesNotMatch(model, /after_pull_actions\.find/);
  assert.doesNotMatch(model, /codex_app|restart_codex|app_kind/);
});

test("missing-child recovery names the child and remains selectable", () => {
  const row = conversationRow("keep_both", "recover-child");
  row.blocked_reason = "Missing local child: call1. Keep both restores the cloud copy and retains the local files.";
  const items = buildPullReviewItems({ ...restorePlan, actions: [], conversation_review: [row] });
  assert.equal(items[0].detail, row.blocked_reason);
  assert.deepEqual(buildPullReviewSelection(items, new Set([items[0].resourceId])).restoreActionIds, ["recover-child"]);
});


test("Pull identifies content stored with redaction Off", () => {
  const row = { ...conversationRow("create_local", "off-selection"), stored_without_credential_redaction: true };
  const items = buildPullReviewItems({ ...restorePlan, conversation_review: [row] });
  assert.ok(items.some((item) => item.detail.includes("Stored without credential redaction")));
  assert.match(renderReview({ ...restorePlan, conversation_review: [row] }), /Stored without credential redaction/);
});

test("an Apply exception stays visible and leaves the review available for retry", async () => {
  const phases: Array<"idle" | "restoring"> = [];
  const applied = await applyPullReview({
    async applyRestore() {
      throw new Error("storage offline");
    },
  }, restorePlan, { resourceIds: [], restoreActionIds: [] }, (phase) => phases.push(phase));
  assert.equal(applied.restoreResult, null);
  assert.equal(applied.success, false);
  assert.deepEqual(phases, ["restoring", "idle"]);
  const html = renderReview(restorePlan, applied.error, undefined, phases[phases.length - 1]);
  assert.match(html, /Restore failed: storage offline/);
  assert.match(html, /class="btn btn-primary sync-pull-apply-button"/);
  assert.doesNotMatch(html, /class="btn btn-primary sync-pull-apply-button"[^>]*disabled/);
});

test("readiness failure preserves the successful Apply result", async () => {
  const applied = result({
    applied_action_ids: ["action-session"],
    readiness: null,
    readiness_error: "profile unavailable",
  });
  const outcome = await applyPullReview({ applyRestore: async () => applied }, restorePlan, {
    resourceIds: ["codex:session:thread-a"],
    restoreActionIds: ["action-session"],
  }, () => undefined);
  assert.equal(outcome.restoreResult, applied);
  assert.equal(outcome.success, false);
  assert.match(outcome.error!, /Readiness verification failed: profile unavailable/);
  const displayed = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: applied,
    readiness: outcome.readiness,
    error: outcome.error,
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(["codex:session:thread-a"]),
  });
  assert.deepEqual(displayed.actions, [{ kind: "recheck", label: "Recheck" }]);
});


test("automatic metadata acceptance excludes file changes and hidden blockers", () => {
  const base: PullReview = { ...restorePlan, actions: [], conversation_review: [] };
  assert.equal(canAcceptPullMetadata(base), true);
  for (const action of ["no_changes", "adopt_cloud_metadata", "report_push_eligible", "report_local_only"] as const) {
    assert.equal(canAcceptPullMetadata({ ...base, conversation_review: [conversationRow(action, "metadata-id")] }), true);
  }
  for (const action of ["materialize_cloud", "create_local"] as const) {
    assert.equal(canAcceptPullMetadata({ ...base, conversation_review: [conversationRow(action, "write-id")] }), false);
  }
  for (const action of ["keep_both", "blocked", "not_found", "require_pull"] as const) {
    assert.equal(canAcceptPullMetadata({ ...base, conversation_review: [conversationRow(action)] }), false);
  }
  assert.equal(canAcceptPullMetadata({ ...base, conversation_review: [conversationRow("adopt_cloud_metadata")] }), false);
  assert.equal(canAcceptPullMetadata({ ...base, conversation_review: [conversationRow("no_changes", null, "busy")] }), false);
  assert.equal(canAcceptPullMetadata({ ...base, conversation_review: [{ ...conversationRow("no_changes"), blocked_reason: "Missing profile" }] }), false);
  // A cleared or recreated storage is an ordinary first Pull; only a storage
  // that could not be listed claims no verdict.
  assert.equal(canAcceptPullMetadata({ ...base, remote_continuity: { state: "replaced", base_revision: 1, remote_revision: 2 } }), true);
  assert.equal(canAcceptPullMetadata({ ...base, remote_continuity: { state: "unknown", message: "offline" } }), false);
  for (const kind of [
    { kind: "write_file", logical_path: "state/provider/memory.md" },
    { kind: "write_project_file", logical_path: "project/note.md", mode: 420, source_mtime: 1 },
    { kind: "ensure_project_directory", logical_path: "project/notes", mode: 493, source_mtime: 1 },
    { kind: "delete_project_file", logical_path: "project/note.md", last_sha256: "a".repeat(64) },
    { kind: "delete_project_directory", logical_path: "project/notes" },
    { kind: "merge_file", logical_path: "state/provider/memory.md" },
  ] as const) {
    assert.equal(canAcceptPullMetadata({ ...base, actions: [{ ...restorePlan.actions[0], kind }] }), false, kind.kind);
  }
  assert.equal(canAcceptPullMetadata({ ...base, actions: [{ ...restorePlan.actions[0], kind: { kind: "manual", message: "No safe target" } }] }), false);
});

test("automatic acceptance uses only metadata IDs from the exact checked plan", async () => {
  const plan: PullReview = { ...restorePlan, actions: [], conversation_review: [
    conversationRow("adopt_cloud_metadata", "accept-metadata"),
    conversationRow("report_local_only"),
    conversationRow("report_push_eligible"),
  ] };
  let calls = 0;
  await acceptPullMetadata({ applyRestore: async (actual, ids) => {
    calls += 1;
    assert.equal(actual, plan, "retain the revision, digest, expiry and binding pins");
    assert.deepEqual(ids, ["accept-metadata"]);
    return result();
  } }, plan);
  assert.equal(calls, 1);
  await acceptPullMetadata({ applyRestore: async (actual, ids) => {
    assert.equal(actual.revision, plan.revision);
    assert.deepEqual(ids, []);
    return result();
  } }, { ...plan, conversation_review: [] });
  await assert.rejects(acceptPullMetadata({ applyRestore: async () => assert.fail("must not apply files") }, restorePlan), /Review the changes/);
});

test("automatic acceptance rejects stale pins and incomplete acknowledgement", async () => {
  const plan: PullReview = { ...restorePlan, actions: [], conversation_review: [] };
  await assert.rejects(acceptPullMetadata({ applyRestore: async () => { throw new Error("Plan expired"); } }, plan), /Plan expired/);
  for (const update of [{ success: false }, { reviewed_base_advanced: false }, { reviewed_base_advanced: undefined }]) {
    await assert.rejects(acceptPullMetadata({ applyRestore: async () => result({ ...update, message: "Check again" }) }, plan), /Check again/);
  }
});

test("the Pull review meters what it would install in both pinned zones, and each row prints its size", () => {
  const plan: PullReview = {
    ...restorePlan,
    actions: [],
    conversation_review: [storedSession("a", 96 * MB, 300)],
  };
  const html = renderReview(plan);
  // The header glance and the footer gauge carry the same figure, in the
  // pull direction; only the footer's announces.
  assert.equal(html.match(/class="sync-review-budget-figure"/g)?.length, 2);
  assert.match(html, /class="sync-review-budget-glance" title="Sessions 0 B · Setup &amp; tools 0 B · Project files 0 B"/);
  assert.match(html, /class="sync-review-budget-track"/);
  assert.match(html, /class="sync-review-budget-gauge pull"[^>]*role="status"/);
  assert.equal(html.match(/class="sync-review-budget[^"]*"[^>]*role="status"/g)?.length, 1);
  assert.match(html, /role="meter"[^>]*aria-label="Pull size"[^>]*aria-valuemax="536870912"[^>]*aria-valuenow="0"/);
  assert.match(html, /aria-valuetext="This pull · 0 B of 512 MB"/);
  // Nothing is ticked at open, so nothing refuses and the meter is empty.
  assert.doesNotMatch(html, /sync-review-footer-note|class="sync-review-budget-track over"/);
  // The list row prints what a pull of it installs.
  assert.match(html, /Conversation · Codex · 96 MB/);
});

test("pullBudgetTotal sums sessions, setup & tools and project files by their stored sizes, and the sentence refuses past the limit", () => {
  const plan: PullReview = {
    ...restorePlan,
    actions: [
      {
        action_id: "write-recording",
        resource_id: "file-recording",
        kind: { kind: "write_project_file", logical_path: "project/assets/recording.mov", mode: 0o644, source_mtime: 1 },
        target_path: "/Users/test/project/gam2/assets/recording.mov",
        source_sha256: "d".repeat(64),
        expected_target_sha256: null,
        source_size: 500 * MB,
        requires_explicit_approval: true,
      },
      {
        action_id: "write-memory",
        resource_id: "codex:memory",
        kind: { kind: "write_file", logical_path: "state/providers/codex/memory.md" },
        target_path: "/Users/test/.codex/memory.md",
        source_sha256: "e".repeat(64),
        expected_target_sha256: null,
        source_size: 4 * MB,
        requires_explicit_approval: false,
      },
    ],
    conversation_review: [storedSession("a", 96 * MB, 300), storedSession("b", 118 * MB, 200)],
  };
  const items = buildPullReviewItems(plan);
  const everything = new Set(items.map((item) => item.resourceId));
  const total = pullBudgetTotal(items, everything, plan.apply_limit_bytes);
  assert.deepEqual(total, {
    sessions: (96 + 118) * MB,
    other: 4 * MB,
    projectFiles: 500 * MB,
    total: (96 + 118 + 4 + 500) * MB,
    limit: LIMIT,
  });
  assert.equal(
    reviewBudgetSentence(total, "pull"),
    "206 MB over the 512 MB pull limit — exclude sessions or files to pull.",
  );
  assert.equal(
    reviewBudgetSentence(pullBudgetTotal(items, new Set(["codex:conversation:cloudthread-a"]), LIMIT), "pull"),
    "This pull · 96 MB of 512 MB",
  );
  // A row the review did not size weighs nothing, and the item says so.
  const unsized = buildPullReviewItems({ ...plan, actions: [], conversation_review: [conversationRow("materialize_cloud", "selection-x")] });
  assert.equal(unsized[0].pullBytes, null);
  assert.equal(pullBudgetTotal(unsized, new Set([unsized[0].resourceId]), LIMIT).total, 0);
});

test("the sessions step's recommended selection is the newest sessions that fit, after setup & tools and whatever the files step ticked", () => {
  const plan: PullReview = {
    ...restorePlan,
    actions: [
      {
        action_id: "write-recording",
        resource_id: "file-recording",
        kind: { kind: "write_project_file", logical_path: "project/assets/recording.mov", mode: 0o644, source_mtime: 1 },
        target_path: "/Users/test/project/gam2/assets/recording.mov",
        source_sha256: "d".repeat(64),
        expected_target_sha256: null,
        source_size: 500 * MB,
        requires_explicit_approval: true,
      },
      {
        action_id: "write-memory",
        resource_id: "codex:memory",
        kind: { kind: "write_file", logical_path: "state/providers/codex/memory.md" },
        target_path: "/Users/test/.codex/memory.md",
        source_sha256: "e".repeat(64),
        expected_target_sha256: null,
        source_size: 4 * MB,
        requires_explicit_approval: false,
      },
    ],
    // Listed oldest first on purpose: the walk sorts by the stored session's
    // last-message time, not by review order.
    conversation_review: [
      storedSession("oldest", 10 * MB, 100),
      storedSession("older", 300 * MB, 200),
      storedSession("newest", 300 * MB, 300),
    ],
  };
  const items = buildPullReviewItems(plan);
  const sessionsStep = items.filter((item) => item.category !== "project_content");
  // Setup & tools first, then the newest session; the older 300 MB does not
  // fit and ends the walk, so the 10 MB oldest stays unticked — a strict
  // time prefix, one block at the top of the list.
  assert.deepEqual(
    [...fitPullRecommendation(items, sessionsStep, new Set(), LIMIT)].sort(),
    ["codex:conversation:cloudthread-newest", "codex:memory"].sort(),
  );
  // A ticked project file is charged before any session: 500 MB of it and
  // 4 MB of memory leave 8 MB, which no session fits, and none is ticked.
  assert.deepEqual(
    [...fitPullRecommendation(items, sessionsStep, new Set(["file-recording"]), LIMIT)],
    ["codex:memory"],
  );
  // A session that could not fit an empty pull is skipped, not the end of
  // the walk.
  const withHuge = buildPullReviewItems({
    ...plan,
    actions: [],
    conversation_review: [storedSession("huge", 600 * MB, 400), storedSession("newest", 96 * MB, 300)],
  });
  assert.deepEqual(
    [...fitPullRecommendation(withHuge, withHuge, new Set(), LIMIT)],
    ["codex:conversation:cloudthread-newest"],
  );
  // The review's Select reads the fitted set: two sessions are safe, one
  // fits. The words and the count name the button; the title says why.
  const html = renderReview({
    ...plan,
    actions: [],
    conversation_review: [storedSession("older", 300 * MB, 200), storedSession("newest", 300 * MB, 300)],
  });
  assert.match(html, /class="btn sync-sync-review-select" title="Select the newest sessions that fit this pull \(1\)">.*?Select <span class="sync-path-count">1<\/span><\/button>/);
  assert.doesNotMatch(html, /Use recommended selection/);
});

test("only the Pull header draws a track, and past the limit it marks where the limit fell", () => {
  const over = { total: 599 * MB, limit: LIMIT };
  const pull = renderToStaticMarkup(<ReviewBudgetGlance budget={over} verb="pull" breakdown="" />);
  // Scaled to the total, so the fill visibly runs past the tick: 512 of 599
  // MB is 85.4% along.
  assert.match(pull, /class="sync-review-budget-track over"[^>]*>.*?<b style="left:85\.4/);
  assert.match(pull, /class="sync-review-budget-figure over"/);
  // The Push header has no room for the bar; its figure carries the same
  // overshoot and the footer gauge still fills.
  const push = renderToStaticMarkup(<ReviewBudgetGlance budget={over} verb="push" breakdown="" />);
  assert.doesNotMatch(push, /sync-review-budget-track/);
  assert.match(push, /class="sync-review-budget-figure over"[^>]*>.*?<b>\+87 MB<\/b>/);
});

test("a review with sessions paints its list in bone on the first render and holds what measures rows", () => {
  // Regression: the review's list painted nothing until its load effect ran,
  // then nothing again between the sessions read and the comparison, while
  // the meters, the count and a greyed "Mark as reviewed" spoke for rows
  // nobody could see.
  const project: LocalProjectSummary = {
    local_project_id: binding.local_project_id,
    bundle_id: binding.bundle_id,
    display_name: "gam2",
    revision: 1,
    project_root: binding.project_root,
    canonical_project_root: binding.canonical_project_root,
    profile_ids: binding.profile_ids,
  };
  const plan: PullReview = {
    ...restorePlan,
    actions: [],
    conversation_review: [storedSession("older", 3 * MB, 200), storedSession("newest", 5 * MB, 300)],
  };
  const html = renderToStaticMarkup(
    <RestorePlanView onSessionActionError={() => undefined}
      projectName="gam2"
      project={project}
      storageName="Local storage 1"
      plan={plan}
      binding={binding}
      phase="idle"
      supportLoading={false}
      busy={false}
      error={null}
      onDismissError={() => undefined}
      onApply={() => undefined}
      onBack={() => undefined}
    />,
  );

  // Placeholders from the first frame: the review's own ungated shape, the
  // plan's two sessions, both under the "Only in …" heading they land in.
  assert.match(html, /class="loading-scaffold sessions" role="region" aria-label="Loading sessions"/);
  assert.match(html, /Loading sessions…/);
  assert.equal(html.match(/class="loading-scaffold-row"/g)?.length, 2);
  assert.match(html, /<section class="sync-history-storage-only"><div class="sync-history-section-heading">/);
  assert.doesNotMatch(html, /Search sessions/);

  // Nothing measures rows that have not landed; only close stays.
  assert.doesNotMatch(html, /sync-review-budget-glance|sync-review-budget-gauge|sync-sync-review-selected/);
  assert.match(html, /aria-label="Close pull review for gam2"/);

  // The primary wears the wait itself, with its reason, and spins.
  const primary = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[\s\S]*?<\/button>/)?.[0] ?? "";
  assert.match(primary, /disabled=""/);
  assert.match(primary, /title="Loading sessions…"/);
  assert.match(primary, /icon-spin/);
  assert.match(primary, /Loading sessions…<\/button>/);
  assert.doesNotMatch(primary, /Mark as reviewed/);

  // The "ready" line is there and empty: it speaks only when the rows land.
  assert.match(html, /<span class="sync-visually-hidden" role="status"><\/span>/);
});

test("the tools row says Checking… while its readiness checks run", () => {
  const plan: PullReview = {
    ...restorePlan,
    actions: [{
      ...restorePlan.actions[0],
      action_id: "action-skill",
      resource_id: "codex:skill:level-lint",
      kind: { kind: "write_file", logical_path: "skills/level-lint/SKILL.md" } as PullReview["actions"][number]["kind"],
    }],
  };
  const render = (supportLoading: boolean) => renderToStaticMarkup(
    <RestorePlanView onSessionActionError={() => undefined}
      projectName="gam2"
      storageName="Local storage 1"
      plan={plan}
      binding={binding}
      phase="idle"
      supportLoading={supportLoading}
      busy={false}
      error={null}
      onDismissError={() => undefined}
      onApply={() => undefined}
      onBack={() => undefined}
    />,
  );
  const checking = render(true);
  assert.match(checking, /<summary>[\s\S]*?Checking…[\s\S]*?<\/summary>/);
  assert.doesNotMatch(checking, /Checking readiness/);
  assert.match(render(false), /<summary>[\s\S]*?0 selected[\s\S]*?<\/summary>/);
});
