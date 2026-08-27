import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  BundleReadiness,
  ConversationBlocker,
  ConversationPlanAction,
  ConversationPlanActionType,
  ConversationReviewAction,
  ConversationReviewRow,
  ProjectBinding,
  RestorePlan,
  RestoreResult,
} from "../../src/types";
import RestorePlanView, {
  pullSelectableResourceIds,
} from "../../src/components/project-sync/RestorePlanView";
import SyncOutcomeNotice from "../../src/components/sync-review/SyncOutcomeNotice";
import {
  buildPullOutcome,
  syncOutcomeView,
  type SyncOutcome,
} from "../../src/components/project-sync/syncOutcome";
import { isIrrecoverableSyncError } from "../../src/components/project-sync/syncError";
import {
  buildPullReviewItems,
  buildPullReviewSelection,
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
      view={syncOutcomeView(outcome)}
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

const restorePlan: RestorePlan = {
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
  conversation_actions: [],
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

function conversationAction(
  action: ConversationPlanActionType,
): ConversationPlanAction {
  return {
    action_id: `conversation-${action}`,
    action,
    provider: "codex",
    thread_id: "019fac21-4aba-7fc2-bb7d-df711c950987",
    source_resource_id: "codex:conversation:019fac21-4aba-7fc2-bb7d-df711c950987",
    proposed_provider_thread_id: null,
    keep_both: null,
    pins: {
      context: {
        host_id: binding.host_id,
        replica_id: binding.replica_id,
        local_project_id: binding.local_project_id,
        provider_profile_id: "profile-codex",
        host_identity_revision: restorePlan.expected_host_identity_revision,
        profile_revision: 0,
        project_binding_revision: binding.revision,
        bindings_document_revision: 0,
      },
      revision: {
        bundle_revision: restorePlan.revision,
        commit_id: restorePlan.commit_id,
        manifest_sha256: restorePlan.manifest_sha256,
        scope_revision: restorePlan.revision,
        scope_sha256: "c".repeat(64),
      },
      tail: {},
      target: { local: { state: "absent" }, provider_state: [] },
      codec: "codex.lineage.v1",
      expires_at: restorePlan.expires_at,
    },
  };
}

function conversationRow(
  action: ConversationReviewAction,
  actionId: string | null = null,
  blocker: ConversationBlocker | null = null,
  displayTitle: string | null = null,
): ConversationReviewRow {
  return {
    provider: "codex",
    thread_id: "019fac21-4aba-7fc2-bb7d-df711c950987",
    resource_id: "codex:conversation:019fac21-4aba-7fc2-bb7d-df711c950987",
    action,
    action_id: actionId,
    blocker,
    display_title: displayTitle,
  };
}

function keepBothAction(): ConversationPlanAction {
  return {
    ...conversationAction("keep_both"),
    action_id: "conversation-keep-both",
    keep_both: {
      preserved_thread_id: "019fac21-4aba-7fc2-bb7d-df711c950988",
      remote_provider_thread_id: "019fac21-4aba-7fc2-bb7d-df711c950989",
      preserved: {
        preserved_from: "019fac21-4aba-7fc2-bb7d-df711c950987",
        ordinal: 1,
      },
      preserved_display_title: "Diverged conversation (Preserved 1 · Machine A)",
    },
    pins: {
      ...conversationAction("keep_both").pins,
      target: {
        local: { state: "sha256", sha256: "d".repeat(64) },
        provider_state: [],
      },
    },
  };
}

function result(overrides: Partial<RestoreResult> = {}): RestoreResult {
  return {
    success: true,
    status: "complete",
    message: "Applied restore actions",
    plan_id: restorePlan.plan_id,
    reviewed_base_advanced: true,
    applied_action_ids: [],
    failed_actions: [],
    preserved_threads: [],
    codex_app_refresh: null,
    ...overrides,
  };
}

function renderReview(
  plan: RestorePlan,
  error: string | null = null,
  onResetMetadata?: () => void,
) {
  return renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      storageName="Local storage 1"
      profileLabel="myconf2"
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
      initialStep="history"
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

test("only actionable or blocked conversation rows keep a Pull review open", () => {
  const plan = (
    rows: ConversationReviewRow[],
    actions: ConversationPlanAction[] = [],
  ): RestorePlan => ({
    ...restorePlan,
    actions: [],
    conversation_actions: actions,
    conversation_review: rows,
  });

  assert.equal(hasPendingRestoreMutations(plan([
    conversationRow("no_changes"),
    conversationRow("report_push_eligible"),
    conversationRow("report_local_only"),
  ])), false);
  const materialize = conversationAction("materialize_cloud");
  assert.equal(hasPendingRestoreMutations(plan(
    [conversationRow("materialize_cloud", materialize.action_id)],
    [materialize],
  )), true);
  assert.equal(hasPendingRestoreMutations(plan([
    conversationRow("blocked", null, "unknown_evidence"),
  ])), true);
});

test("keep-both is one reviewed action with no second resolution payload", () => {
  const plan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [keepBothAction()],
    conversation_review: [conversationRow("keep_both", "conversation-keep-both")],
  };
  const items = buildPullReviewItems(plan);
  const selected = new Set(items.map((item) => item.resourceId));
  const selection = buildPullReviewSelection(items, selected, new Set(), new Set());

  assert.deepEqual(selection, {
    resourceIds: ["codex:conversation:019fac21-4aba-7fc2-bb7d-df711c950987"],
    restoreActionIds: ["conversation-keep-both"],
  });
  assert.deepEqual(
    pullSelectableResourceIds(items, new Set(), new Set()),
    selected,
  );

  const html = renderReview(plan);
  assert.match(html, /Keep both/);
  assert.match(html, /Select Conversation/);
  assert.doesNotMatch(html, /radiogroup|Fork and overwrite|Discard the local branch/);
});

test("blocked conversation evidence is visible but cannot be selected", () => {
  const plan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [],
    conversation_review: [conversationRow("blocked", null, "unknown_evidence")],
  };
  const items = buildPullReviewItems(plan);

  assert.deepEqual(pullSelectableResourceIds(items, new Set(), new Set()), new Set());
  const html = renderReview(plan);
  assert.match(html, /Can&#x27;t sync/);
  assert.doesNotMatch(html, /Select Conversation/);
});

/* The row's name comes from the stored thread, and when the stored thread has
   none the row says so. It must never be named after its id: a CloudThreadId's
   first characters are the mandatory "cloudthread-" namespace, so every
   untitled cloud row would carry the identical name — which is what the
   deleted `Task ${threadId.slice(0, 8)}` fallback did. */
test("a restored row is named by its stored title, or reads untitled", () => {
  const materialize = conversationAction("materialize_cloud");
  const restoredFor = (displayTitle: string | null) => {
    const plan = {
      ...restorePlan,
      actions: [],
      conversation_actions: [materialize],
      conversation_review: [
        conversationRow("materialize_cloud", materialize.action_id, null, displayTitle),
      ],
    };
    return buildPullOutcome({
      projectId: binding.local_project_id,
      plan,
      result: result({ applied_action_ids: [materialize.action_id] }),
      readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
      items: buildPullReviewItems(plan),
      completedResourceIds: new Set([materialize.source_resource_id!]),
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

test("keep-both completion reports the independent session and optional Codex refresh", () => {
  const action = keepBothAction();
  const plan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [action],
    conversation_review: [conversationRow("keep_both", action.action_id)],
  };
  const completed = result({
    applied_action_ids: [action.action_id],
    preserved_threads: [{
      action_id: action.action_id,
      original_thread_id: action.thread_id,
      preserved_thread_id: action.keep_both!.preserved_thread_id,
      preserved_display_title: action.keep_both!.preserved_display_title,
    }],
    codex_app_refresh: {
      operation_id: "operation-materialize",
      app_kind: "codex",
      app_name: "Codex",
      state: "restart_available",
      message: "Restart Codex to refresh its task list.",
    },
  });
  const items = buildPullReviewItems(plan);
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan,
    result: completed,
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items,
    completedResourceIds: new Set([action.source_resource_id!]),
  });

  assert.equal(outcome.rows[0].status, "preserved");
  assert.equal(outcome.rows[0].threadId, action.keep_both!.preserved_thread_id);
  // The row is named by the marker title the copy was renamed to.
  assert.equal(outcome.rows[0].title, "Diverged conversation (Preserved 1 · Machine A)");
  assert.deepEqual(outcome.action, {
    kind: "restart_codex",
    label: "Restart Codex",
    title: "Shows restored sessions — active Codex chats are interrupted",
    operationId: "operation-materialize",
    appKind: "codex",
  });

  const html = renderOutcome(outcome);
  assert.match(html, /Kept independently/);
  assert.match(html, /previous local history was kept as an independent session/);
  assert.doesNotMatch(html, /native fork|Retry fork/i);
});

test("recovery-required Pull directs the user back through the current Pull path", () => {
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result: result({ status: "recovery_required" }),
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set(),
  });

  assert.equal(outcome.tone, "warning");
  assert.equal(outcome.title, "Pull paused for recovery");
  assert.deepEqual(outcome.action, { kind: "open_pull", label: "Open Pull review" });
  assert.match(renderOutcome(outcome), /Sync recovery is required/);
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
  assert.deepEqual(outcome.action, {
    kind: "reset_metadata",
    label: "Reset local sync state",
  });
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
    async applyRestore(planId, actionIds) {
      calls.push([planId, actionIds]);
      return applied;
    },
    async getRestoreReadiness(planId) {
      calls.push([planId]);
      return ready;
    },
  }, restorePlan, {
    resourceIds: ["codex:session:thread-a"],
    restoreActionIds: ["action-session"],
  }, (phase) => phases.push(phase));

  assert.deepEqual(calls, [
    [restorePlan.plan_id, ["action-session"]],
    [restorePlan.plan_id],
  ]);
  assert.deepEqual(phases, ["restoring", "verifying", "complete"]);
  assert.equal(appliedResult.success, true);
});

test("the Apply coordinator rejects action ids outside the immutable plan", async () => {
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
