import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  BundleReadiness,
  ConversationBlocker,
  ConversationReviewAction,
  ConversationReviewRow,
  ProjectBinding,
  PullReview,
  RestoreResult,
} from "../../src/types";
import RestorePlanView, {
  pullSelectableResourceIds,
} from "../../src/components/project-sync/RestorePlanView";
import SyncOutcomeNotice from "../../src/components/project-sync/SyncOutcomeNotice";
import {
  buildPullOutcome,
  type SyncOutcome,
} from "../../src/components/project-sync/syncOutcome";
import { isIrrecoverableSyncError } from "../../src/components/project-sync/syncError";
import {
  buildPullReviewItems,
  buildPullReviewSelection,
  pullActionResourceIds,
  recommendedPullSelection,
} from "../../src/components/project-sync/pullReviewModel";
import {
  applyPullReview,
  beginPullReview,
  hasPendingRestoreMutations,
  type PullReviewApi,
} from "../../src/components/project-sync/pullReviewFlow";

function renderOutcome(outcome: SyncOutcome) {
  return renderToStaticMarkup(
    <SyncOutcomeNotice
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

const restorePlan: PullReview = {
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
): ConversationReviewRow {
  return {
    provider: "codex",
    thread_id: localThreadId,
    cloud_thread_id: cloudThreadId,
    resource_id: `codex:conversation:${localThreadId}`,
    action,
    action_id: actionId,
    blocker,
    display_title: displayTitle,
  };
}

function result(overrides: Partial<RestoreResult> = {}): RestoreResult {
  return {
    success: true,
    message: "Applied restore actions",
    plan_id: restorePlan.plan_id,
    reviewed_base_advanced: true,
    applied_action_ids: [],
    failed_actions: [],
    preserved_threads: [],
    after_pull_actions: [],
    ...overrides,
  };
}

function renderReview(
  plan: PullReview,
  error: string | null = null,
  onResetMetadata?: () => void,
) {
  return renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      storageName="Local storage 1"
      plan={plan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={error}
      onApply={() => undefined}
      onBack={() => undefined}
      onUnlinkStorage={() => undefined}
      onResetMetadata={onResetMetadata}
    />,
  );
}

test("a replacement storage history offers unlinking without gating Pull", () => {
  const html = renderReview({
    ...restorePlan,
    remote_continuity: { state: "replaced", base_revision: 1, remote_revision: 2 },
  });

  assert.match(html, /<strong class="notice-lead">Storage history changed<\/strong>/);
  assert.match(html, /Local storage 1 was reset/);
  assert.match(html, /Unlink storage…/);
  assert.match(html, /sync-pull-apply-button/);
  const primary = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[\s\S]*?<\/button>/)?.[0] ?? "";
  assert.match(primary, /data-thread-status="cloud-outstanding"/);
  assert.doesNotMatch(primary, /data-icon="download"/);
});

test("only cloud-side or divergent conversation actions keep a Pull review open", () => {
  const plan = (
    rows: ConversationReviewRow[],
  ): PullReview => ({
    ...restorePlan,
    actions: [],
    conversation_review: rows,
  });

  const adopt = conversationRow("adopt_cloud_metadata", "selection-adopt");
  assert.equal(hasPendingRestoreMutations(plan([
    conversationRow("no_changes"),
    adopt,
    conversationRow("report_push_eligible"),
    conversationRow("report_local_only"),
  ])), false);
  const materialize = conversationRow("materialize_cloud", "selection-materialize");
  assert.equal(hasPendingRestoreMutations(plan(
    [materialize],
  )), true);
  assert.equal(hasPendingRestoreMutations(plan([
    conversationRow("blocked", null, "unknown_evidence"),
  ])), false);
});

test("keep-both is one reviewed action with no second resolution payload", () => {
  const plan: PullReview = {
    ...restorePlan,
    actions: [],
    conversation_review: [conversationRow("keep_both", "conversation-keep-both")],
  };
  const items = buildPullReviewItems(plan);
  const selected = new Set(items.map((item) => item.resourceId));
  const selection = buildPullReviewSelection(items, selected, new Set(), new Set());

  assert.deepEqual(selection, {
    resourceIds: [`codex:conversation:${localThreadId}`],
    restoreActionIds: ["conversation-keep-both"],
  });
  assert.deepEqual(
    pullSelectableResourceIds(items, new Set(), new Set()),
    selected,
  );

  const html = renderReview(plan);
  assert.match(html, /Not selected/);
  assert.match(html, /Select Conversation/);
  assert.match(html, /Use recommended selection \(1\)/);
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
  assert.match(html, /aria-label="Use recommended selection \(1\)"/);

  const selection = buildPullReviewSelection(
    items,
    new Set(["file-ghost-story"]),
    new Set(),
    new Set(),
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
  assert.deepEqual(pullSelectableResourceIds(items, new Set(), new Set()), new Set());
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
    new Set(),
    new Set(),
  );
  const failedRows = rows.slice(35);
  const completed = result({
    success: false,
    message: "Applied 35 actions; 3 require attention",
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
    async getRestoreReadiness() {
      return {
        bundle_id: binding.bundle_id,
        state: "blocked",
        issues: [{
          issue_id: "restore-project-data",
          category: "project_files",
          title: "3 project data changes were left unchanged",
          severity: "error",
        }],
      };
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
    { icon: "git-branch", label: "Revision", value: "1", lead: true },
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

test("irrecoverable sync metadata failures offer a local reset instead of retry", () => {
  const message = "conversation Pull action 'action-a' is stale at SemanticDigest";
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: result({
      success: false,
      failed_actions: [{ action_id: "action-a", display_name: "Task A", message }],
    }),
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(),
    error: message,
  });

  assert.equal(isIrrecoverableSyncError(message), true);
  assert.equal(isIrrecoverableSyncError("storage is temporarily unavailable"), false);
  assert.deepEqual(outcome.actions, [{
    kind: "reset_metadata",
    label: "Reset local sync state",
  }]);
  const html = renderOutcome(outcome);
  assert.match(html, /Reset local sync state/);
  assert.match(html, /data-icon="trash"/);
  assert.doesNotMatch(html, />Retry</);

  const reviewHtml = renderReview(restorePlan, message, () => undefined);
  assert.match(reviewHtml, /Reset local sync state/);
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

test("the Apply coordinator sends only approved action ids and verifies readiness", async () => {
  const calls: unknown[][] = [];
  const phases: string[] = [];
  const ready: BundleReadiness = { bundle_id: binding.bundle_id, state: "ready", issues: [] };
  const applied = result({ applied_action_ids: ["action-session"] });

  const appliedResult = await applyPullReview({
    async applyRestore(review, actionIds) {
      calls.push([review, actionIds]);
      return applied;
    },
    async getRestoreReadiness(review) {
      calls.push([review]);
      return ready;
    },
  }, restorePlan, {
    resourceIds: ["codex:session:thread-a"],
    restoreActionIds: ["action-session"],
  }, (phase) => phases.push(phase));

  assert.deepEqual(calls, [
    [restorePlan, ["action-session"]],
    [restorePlan],
  ]);
  assert.deepEqual(phases, ["restoring", "verifying", "complete"]);
  assert.equal(appliedResult.success, true);
});

test("the Apply coordinator rejects action ids outside the comparison", async () => {
  await assert.rejects(() => applyPullReview({
    async applyRestore() {
      throw new Error("must not execute");
    },
    async getRestoreReadiness() {
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
