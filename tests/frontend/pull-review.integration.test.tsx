import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  BundleReadiness,
  ConversationPlanAction,
  ConversationPlanActionType,
  ConversationReasonCode,
  ConversationRelation,
  ProjectBinding,
  RestorePlan,
  RestoreResult,
} from "../../src/types";
import RestorePlanView, {
  pullSelectableResourceIds,
} from "../../src/components/project-sync/RestorePlanView";
import SyncOutcomeNotice from "../../src/components/project-sync/SyncOutcomeNotice";
import { buildPullOutcome, type SyncOutcome } from "../../src/components/project-sync/syncOutcome";
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

/**
 * The finished Pull no longer lives in the review — it closes and reports on
 * the storage card — so outcome intents are asserted against the data
 * `buildPullOutcome` produces and the banner `SyncOutcomeNotice` renders.
 */
function renderOutcome(outcome: SyncOutcome) {
  return renderToStaticMarkup(
    <SyncOutcomeNotice outcome={outcome} onAction={() => undefined} onDismiss={() => undefined} />,
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
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
  generation: 1,
  commit_id: "commit-restore",
  manifest_sha256: "a".repeat(64),
  binding_revision: binding.revision,
  created_at: 1,
  expires_at: 2,
  conversation_actions: [],
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

test("a replacement storage history offers unlinking from Pull review", () => {
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      storageName="Local storage 1"
      profileLabel="myconf2"
      plan={{
        ...restorePlan,
        remote_continuity: {
          state: "replaced",
          base_generation: 1,
          remote_generation: 2,
        },
      }}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
      onUnlinkStorage={() => undefined}
    />,
  );

  assert.match(html, /class="sync-callout warning" role="status"/);
  assert.match(html, /<strong>Storage history changed<\/strong>/);
  assert.match(
    html,
    /Local storage 1 was reset\. Pulling adopts its new history; local content is backed up first\./,
  );
  assert.match(html, /class="btn btn-ghost danger">Unlink storage…<\/button>/);
  // The notice is in-flow, not a gate: the primary Pull action stays enabled.
  assert.match(html, /class="btn btn-primary sync-pull-apply-button">/);
  assert.match(html, /svg>Pull<\/button>/);
});

test("post-fork replans finalize only when conversation rows are informational", () => {
  assert.equal(hasPendingRestoreMutations({
    ...restorePlan,
    actions: [],
    conversation_actions: [
      conversationAction("no_op", "identical", "identical"),
      conversationAction("keep_local", "cloud_missing", "cloud_missing"),
    ],
  }), false);
  assert.equal(hasPendingRestoreMutations({
    ...restorePlan,
    actions: [],
    conversation_actions: [
      conversationAction("fast_forward_local", "local_prefix_of_cloud", "local_prefix_of_cloud"),
    ],
  }), true);
  assert.equal(hasPendingRestoreMutations(restorePlan), true);
});

function conversationAction(
  action: ConversationPlanActionType,
  reasonCode: ConversationReasonCode,
  relation: ConversationRelation,
): ConversationPlanAction {
  return {
    action_id: `conversation-${action}`,
    direction: "pull",
    action,
    reason_code: reasonCode,
    relation,
    provider: "codex",
    host_id: binding.host_id,
    replica_id: binding.replica_id,
    local_project_id: binding.local_project_id,
    provider_profile_id: "profile-codex",
    thread_id: "thread-cloud",
    source_resource_id: "codex:conversation:thread-cloud",
    required_codec: "codex.lineage.v2",
    pinned_generation: restorePlan.generation,
    pinned_commit_id: restorePlan.commit_id,
    pinned_manifest_sha256: restorePlan.manifest_sha256,
    expected_local_target: { state: "absent" },
    expected_provider_state: [],
    expected_profile_revision: 0,
    expected_project_binding_revision: binding.revision,
    expected_host_identity_revision: restorePlan.expected_host_identity_revision,
    expected_bindings_document_revision: 0,
    expires_at: restorePlan.expires_at,
  };
}

function divergenceAction(): ConversationPlanAction {
  return {
    ...conversationAction("resolve_divergence", "diverged", "diverged"),
    action_id: "conversation-divergence",
    thread_id: "thread-shared",
    source_resource_id: "codex:session:thread-shared",
    expected_local_target: { state: "sha256", sha256: "d".repeat(64) },
    expected_provider_state: [{
      target_path: "/Users/test/.codex/sessions/2026/07/29/rollout-thread-shared.jsonl",
      expectation: { state: "sha256", sha256: "d".repeat(64) },
    }],
    expected_binding_revision: 2,
    divergence: {
      available_resolutions: [
        "preserve_local_with_native_fork",
        "use_cloud_only",
      ],
      recommended_resolution: "preserve_local_with_native_fork",
      local_branch_preserved: false,
      preserved_thread_id: "thread-preserved",
      source_provider_thread_id: "019fac21-4aba-7fc2-bb7d-df711c950987",
      local_lineage_digest: "e".repeat(64),
      local_semantic_digest: "f".repeat(64),
      local_message_count: 3,
      pinned_local_title: "Analyze codebase knowledge graph",
      pinned_cloud_title: "Analyze codebase knowledge graph",
      desired_fork_name: "Analyze codebase knowledge graph — local before Pull · 2026-07-29 18:42Z",
      desired_original_name: "Analyze codebase knowledge graph",
      codex_executable: "/usr/local/bin/codex",
      codex_executable_sha256: "1".repeat(64),
      available_executors: ["app_server_stdio", "native_pty"],
    },
  };
}

test("conversation divergence defaults to keep-both in the inline row control", () => {
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [divergenceAction()],
  };
  const items = buildPullReviewItems(conflictPlan);
  const selected = new Set(items.filter((item) => item.conversationActions.length > 0).map((item) => item.resourceId));
  const selection = buildPullReviewSelection(
    items,
    selected,
    new Set(),
    new Set(),
  );
  assert.deepEqual(selection.conversationResolutions, [{
    action_id: "conversation-divergence",
    resolution: "preserve_local_with_native_fork",
    destructive_approved: false,
  }]);

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictPlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="history"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /class="sync-divergence-inline"/);
  assert.match(html, /role="radiogroup" aria-label="Conflict resolution"/);
  assert.match(
    html,
    /role="radio" aria-checked="true" class="active" title="The local branch becomes a Codex fork — nothing is lost">Keep both</,
    "the recommended keep-both resolution is pre-selected",
  );
  assert.match(
    html,
    /role="radio" aria-checked="false" class="" title="Replaces the local branch">Use cloud only</,
  );
  assert.doesNotMatch(
    html,
    /sync-divergence-inline-confirm/,
    "keep-both requires no destructive acknowledgement",
  );
  assert.doesNotMatch(html, /sync-divergence-card|Keep both branches|Use cloud version only/);
});

test("an unpreserved cloud-only default demands the discard confirmation before Pull", () => {
  const action = divergenceAction();
  action.divergence = {
    ...action.divergence!,
    recommended_resolution: "use_cloud_only",
    local_branch_preserved: false,
  };
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={{ ...restorePlan, actions: [], conversation_actions: [action] }}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="history"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /aria-checked="true" class="active" title="Replaces the local branch">Use cloud only</);
  assert.match(html, /class="sync-divergence-inline-confirm"/);
  assert.match(html, /Discard the local branch — I understand it won&#x27;t remain in Codex/);
  assert.match(
    html,
    /class="btn btn-primary sync-pull-apply-button" disabled="" title="Confirm the discard above first"/,
    "an unconfirmed discard blocks the Pull action with its reason",
  );
});

test("a preserved divergence recommends cloud adoption and waives the destructive acknowledgement", () => {
  const action = divergenceAction();
  action.divergence = {
    ...action.divergence!,
    recommended_resolution: "use_cloud_only",
    local_branch_preserved: true,
  };
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [action],
  };
  const items = buildPullReviewItems(conflictPlan);
  const selected = new Set(items.filter((item) => item.conversationActions.length > 0).map((item) => item.resourceId));
  const selection = buildPullReviewSelection(
    items,
    selected,
    new Set(),
    new Set(),
  );
  assert.deepEqual(selection.conversationResolutions, [{
    action_id: "conversation-divergence",
    resolution: "use_cloud_only",
    destructive_approved: false,
  }], "the plan-recommended cloud adoption is the default selection");

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictPlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="history"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(
    html,
    /aria-checked="true" class="active" title="The local branch is already preserved in a Codex fork; restore the cloud conversation">Adopt cloud</,
    "the plan-recommended cloud adoption is pre-selected",
  );
  assert.match(
    html,
    /title="Keep another copy of the local branch as a new Codex fork before restoring the cloud conversation">Fork again</,
  );
  assert.doesNotMatch(
    html,
    /sync-divergence-inline-confirm/,
    "the witnessed overwrite renders no destructive acknowledgement",
  );
  assert.doesNotMatch(html, />Keep both<|>Use cloud only</, "preserved rows swap to the fork-again vocabulary");
  assert.match(
    html,
    /class="btn btn-primary sync-pull-apply-button">/,
    "a preserved cloud adoption keeps Pull enabled without a discard confirmation",
  );
});

test("unacknowledged fork checkpoints expose only an explicit retry", () => {
  const action = divergenceAction();
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [action],
  };
  const checkpoint: RestoreResult = {
    success: false,
    status: "fork_unacknowledged",
    message: "Mallard did not acknowledge a fork from this attempt.",
    plan_id: conflictPlan.plan_id,
    applied_action_ids: [],
    failed_actions: [],
    codex_divergences: [],
    codex_fork_checkpoints: [{
      operation_id: "operation-divergence",
      origin: "pull_preserve",
      plan_id: conflictPlan.plan_id,
      action_id: action.action_id,
      thread_id: action.thread_id,
      source_provider_thread_id: action.divergence!.source_provider_thread_id,
      suggested_fork_name: action.divergence!.desired_fork_name,
      status: "fork_unacknowledged",
      message: "Mallard did not acknowledge a fork from this attempt.",
    }],
  };
  const outcomeFor = (result: RestoreResult) => buildPullOutcome({
    projectId: binding.local_project_id,
    plan: conflictPlan,
    result,
    readiness: null,
    items: buildPullReviewItems(conflictPlan),
    completedResourceIds: new Set<string>(),
    error: null,
  });

  // The review itself no longer carries the fork decision: the checkpoint
  // parks it, so the conflicted row's control is hidden and the footer offers
  // nothing but the Pull.
  const reviewHtml = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictPlan}
      binding={binding}
      readiness={null}
      restoreResult={checkpoint}
      phase="complete"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.doesNotMatch(
    reviewHtml,
    /sync-divergence-inline/,
    "the parked checkpoint hides the row conflict control",
  );
  assert.doesNotMatch(reviewHtml, /sync-fork-checkpoint/, "the old checkpoint section markup is gone");
  assert.doesNotMatch(
    reviewHtml,
    /Retry fork|Verify fork|>Done<\/button>|>Cancel<|Force/,
    "the closed-outcome model leaves the review with no fork verbs of its own",
  );
  assert.match(reviewHtml, /class="btn btn-primary sync-pull-apply-button">/);
  assert.match(reviewHtml, /svg>Pull<\/button>/);

  const outcome = outcomeFor(checkpoint);
  assert.equal(outcome.tone, "warning");
  assert.equal(outcome.title, "Pull complete — fork not confirmed");
  assert.equal(outcome.defaultOpen, true, "a parked decision opens the banner instead of folding away");
  assert.deepEqual(outcome.action, {
    kind: "retry_fork",
    label: "Retry fork",
    operationId: "operation-divergence",
  }, "the one affordance is an explicit retry of this checkpoint");
  assert.deepEqual(outcome.rows, [{
    key: "operation-divergence",
    // The trailing date stamp goes the way the history list already sends it.
    title: "Analyze codebase knowledge graph — local before Pull",
    status: "failed",
    forkCount: 0,
    // The row leads with the thing that matters — nothing of theirs was lost.
    // The backend's own sentence is support detail and lives in the technical
    // block; the thread id rides along for the same reason.
    reason: "Codex didn't confirm the fork — your local session is untouched",
    threadId: "019fac21-4aba-7fc2-bb7d-df711c950987",
  }], "the parked row keeps the reassurance and carries the thread id");

  const html = renderOutcome(outcome);
  // A decision holds the banner open: no collapsed class, and the fold button
  // reports the state it would toggle away from.
  assert.match(html, /class="sync-callout sync-outcome-notice warning"/);
  assert.match(html, /class="sync-history-commit-toggle" aria-expanded="true"/);
  assert.match(html, /<strong>Pull complete — fork not confirmed<\/strong>/);
  assert.match(
    html,
    /<strong>Analyze codebase knowledge graph — local before Pull<\/strong>/,
    "the suggested fork name is reachable without opening a technical block",
  );
  assert.match(
    html,
    /<code>019fac21-4aba-7fc2-bb7d-df711c950987<\/code>/,
    "the row carries the complete thread id, not an 8-character stub",
  );
  assert.match(html, /your local session is untouched/);
  assert.match(html, /<dt>Message<\/dt><dd>Mallard did not acknowledge a fork from this attempt\.<\/dd>/);
  assert.equal(
    (html.match(/>Retry fork<\/button>/g) ?? []).length,
    1,
    "one parked checkpoint exposes exactly one retry",
  );
  assert.doesNotMatch(html, /Restart /, "nothing forked, so no App restart is offered");

  // The restart offer belongs to a fork that actually landed; an app that
  // reports itself restartable cannot conjure one out of a parked checkpoint.
  assert.deepEqual(outcomeFor({
    ...checkpoint,
    codex_app_refresh: {
      operation_id: "operation-divergence",
      app_kind: "codex",
      app_name: "Codex App",
      state: "restart_available",
      message: "Codex App may need a restart to refresh its conversation list.",
    },
  }).action, {
    kind: "retry_fork",
    label: "Retry fork",
    operationId: "operation-divergence",
  }, "a restartable app does not turn an unconfirmed fork into a restart");

  // A checkpoint that needs recovery rather than a retry still reports the
  // session as unforked and carries its own message onto the banner.
  const recoveryRequired: RestoreResult = {
    ...checkpoint,
    status: "recovery_required",
    message: "The acknowledged fork needs recovery.",
    codex_fork_checkpoints: checkpoint.codex_fork_checkpoints.map((item) => ({
      ...item,
      status: "recovery_required",
      message: "The acknowledged fork needs recovery.",
    })),
  };
  const recoveryHtml = renderOutcome(outcomeFor(recoveryRequired));
  assert.match(recoveryHtml, /<dt>Message<\/dt><dd>The acknowledged fork needs recovery\.<\/dd>/);
  assert.match(
    recoveryHtml,
    /class="sync-thread-sync-indicator unavailable"/,
    "the session is still reported as unforked",
  );
});

function secondDivergenceAction(): ConversationPlanAction {
  const base = divergenceAction();
  return {
    ...base,
    action_id: "conversation-divergence-second",
    thread_id: "thread-shared-second",
    source_resource_id: "codex:session:thread-shared-second",
    divergence: {
      ...base.divergence!,
      preserved_thread_id: "thread-preserved-second",
      source_provider_thread_id: "019fac21-4aba-7fc2-bb7d-df711c950988",
      pinned_local_title: "Draft the release notes",
      pinned_cloud_title: "Draft the release notes",
      desired_fork_name: "Draft the release notes — local before Pull · 2026-07-29 18:42Z",
      desired_original_name: "Draft the release notes",
    },
  };
}

test("a fork checkpoint queue retries one fork at a time in the same review", () => {
  const first = divergenceAction();
  const second = secondDivergenceAction();
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [first, second],
  };
  const queued: RestoreResult = {
    success: true,
    status: "fork_unacknowledged",
    message: "Applied 0 restore actions; 2 Codex fork attempts were not acknowledged",
    plan_id: conflictPlan.plan_id,
    applied_action_ids: [],
    failed_actions: [],
    codex_divergences: [],
    codex_fork_checkpoints: [
      {
        operation_id: "operation-divergence-first",
        origin: "pull_preserve",
        plan_id: conflictPlan.plan_id,
        action_id: first.action_id,
        thread_id: first.thread_id,
        source_provider_thread_id: first.divergence!.source_provider_thread_id,
        suggested_fork_name: first.divergence!.desired_fork_name,
        status: "fork_unacknowledged",
        message: "Mallard did not acknowledge a fork from this attempt.",
      },
      {
        operation_id: "operation-divergence-second",
        origin: "pull_preserve",
        plan_id: conflictPlan.plan_id,
        action_id: second.action_id,
        thread_id: second.thread_id,
        source_provider_thread_id: second.divergence!.source_provider_thread_id,
        suggested_fork_name: second.divergence!.desired_fork_name,
        status: "fork_unacknowledged",
        message: "Mallard did not acknowledge a fork from this attempt.",
      },
    ],
  };
  const outcome = buildPullOutcome({
    projectId: binding.local_project_id,
    plan: conflictPlan,
    result: queued,
    readiness: null,
    items: buildPullReviewItems(conflictPlan),
    completedResourceIds: new Set<string>(),
    error: null,
  });
  assert.deepEqual(outcome.action, {
    kind: "retry_fork",
    label: "Retry fork",
    operationId: "operation-divergence-first",
  }, "the queue offers its first checkpoint, never the second");
  assert.deepEqual(
    outcome.stats.find((stat) => stat.label === "fork pending"),
    { icon: "git-fork", value: "2", label: "fork pending" },
    "the summary line counts the whole queue",
  );
  assert.deepEqual(outcome.rows.map((row) => [row.key, row.title, row.status]), [
    [
      "operation-divergence-first",
      "Analyze codebase knowledge graph — local before Pull",
      "failed",
    ],
    [
      "operation-divergence-second",
      "Draft the release notes — local before Pull",
      "failed",
    ],
  ], "both parked forks are reported by name even though only one can be retried");

  const html = renderOutcome(outcome);
  assert.equal(
    (html.match(/>Retry fork<\/button>/g) ?? []).length,
    1,
    "two parked checkpoints still expose one retry, not one per row",
  );
  assert.match(html, /<b>2<\/b>fork pending/);
  assert.match(html, /<strong>Draft the release notes — local before Pull<\/strong>/);
  assert.doesNotMatch(html, /Verify fork|>Cancel<|Restart /);
});

test("multiple conversation conflicts each keep their own inline resolution", () => {
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [divergenceAction(), secondDivergenceAction()],
  };
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictPlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="history"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.equal(
    (html.match(/class="sync-divergence-inline"/g) ?? []).length,
    2,
    "each conflicted row hosts its own resolution control",
  );
  assert.equal((html.match(/role="radiogroup" aria-label="Conflict resolution"/g) ?? []).length, 2);
  assert.equal(
    (html.match(/aria-checked="true" class="active" title="The local branch becomes a Codex fork — nothing is lost">Keep both</g) ?? []).length,
    2,
    "both conflicts default to their own keep-both selection",
  );
  assert.equal(
    (html.match(/aria-label="Select Conversation"/g) ?? []).length,
    2,
    "both conflicts stay independently selectable",
  );
});

test("completed divergence reports the preserved fork, its naming outcome, refresh warning, and optional App restart", () => {
  const action = divergenceAction();
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [action],
  };
  const completed: RestoreResult = {
    success: true,
    status: "complete",
    message: "Restored the cloud conversation and preserved the local fork.",
    plan_id: conflictPlan.plan_id,
    applied_action_ids: [action.action_id],
    failed_actions: [],
    codex_fork_checkpoints: [],
    codex_app_refresh: {
      operation_id: "operation-divergence",
      app_kind: "codex",
      app_name: "Codex App",
      state: "restart_available",
      message: "Codex App may need a restart to refresh its conversation list.",
    },
    codex_divergences: [{
      action_id: action.action_id,
      original_thread_id: "thread-shared",
      original_provider_thread_id: "019fac21-4aba-7fc2-bb7d-df711c950987",
      resolution: "preserve_local_with_native_fork",
      preserved_thread_id: "thread-preserved",
      preserved_provider_thread_id: "019fac73-c1ff-79b3-bb8e-1673ed0d644c",
      executor: "native_pty",
      desired_fork_name: action.divergence!.desired_fork_name,
      fork_name_status: "applied",
      desired_original_name: action.divergence!.desired_original_name,
      original_name_status: "unchanged",
      inventory_refresh_status: "failed",
    }],
  };
  const outcomeFor = (
    result: RestoreResult,
    readiness: BundleReadiness | null = null,
  ) => buildPullOutcome({
    projectId: binding.local_project_id,
    plan: conflictPlan,
    result,
    readiness,
    items: buildPullReviewItems(conflictPlan),
    completedResourceIds: new Set([action.source_resource_id!]),
    error: null,
  });

  const outcome = outcomeFor(completed);
  assert.equal(outcome.tone, "success");
  assert.equal(outcome.title, "Pull complete");
  assert.equal(outcome.defaultOpen, false, "a fork that landed folds itself away");
  assert.deepEqual(outcome.rows, [{
    key: "conversation-divergence",
    title: "Analyze codebase knowledge graph",
    status: "forked",
    forkCount: 0,
    // This apply reported inventory_refresh_status "failed": the fork landed
    // but Codex's conversation list did not reload, which is exactly the case
    // where "it worked" alone would leave the user hunting for it.
    reason: "local session kept as a Codex fork — Codex's conversation list did not refresh",
  }], "the fork row names the original conversation and says the discovery refresh failed");
  assert.deepEqual(
    outcome.stats.find((stat) => stat.label === "session forked"),
    { icon: "git-fork", value: "1", label: "session forked" },
  );
  assert.deepEqual(outcome.action, {
    kind: "restart_codex",
    label: "Restart Codex App",
    title: "Shows the restored fork — active Codex App chats are interrupted",
    operationId: "operation-divergence",
    appKind: "codex",
  }, "the restart names the app, its operation, and what it costs");

  const html = renderOutcome(outcome);
  assert.match(
    html,
    /class="sync-callout sync-outcome-notice success collapsed"/,
    "a clean fork folds itself away; the sessions stay in the DOM behind the fold",
  );
  assert.match(html, /class="sync-history-commit-toggle" aria-expanded="false"/);
  assert.match(html, /class="sync-thread-sync-indicator diverged-forked"/);
  assert.match(html, /<strong>Analyze codebase knowledge graph<\/strong>/);
  assert.match(html, /local session kept as a Codex fork/);
  assert.match(
    html,
    /title="Shows the restored fork — active Codex App chats are interrupted"/,
  );
  assert.match(html, /svg>Restart Codex App<\/button>/);

  // The restart is earned only by a fork that produced a child thread in an
  // app that can be restarted, from an apply that finished.
  assert.equal(outcomeFor({
    ...completed,
    codex_divergences: completed.codex_divergences.map((divergence) => ({
      ...divergence,
      preserved_provider_thread_id: null,
    })),
  }).action, undefined, "a fork with no child thread has nothing for Codex to reload");
  assert.equal(outcomeFor({
    ...completed,
    codex_divergences: completed.codex_divergences.map((divergence) => ({
      ...divergence,
      resolution: "use_cloud_only" as const,
    })),
  }).action, undefined, "an adopted cloud branch forked nothing to show");
  assert.equal(outcomeFor({
    ...completed,
    codex_app_refresh: { ...completed.codex_app_refresh!, state: "manual_restart" },
  }).action, undefined, "an app Mallard cannot restart offers no button");
  assert.equal(
    outcomeFor({ ...completed, status: "fork_unacknowledged" }).action,
    undefined,
    "an unfinished apply offers no restart",
  );

  // Setup that still needs attention rides the same banner and opens it,
  // without withdrawing the restart the fork earned.
  const withSetupIssue = outcomeFor(completed, {
    bundle_id: binding.bundle_id,
    state: "needs_setup",
    issues: [{
      issue_id: "codex-profile",
      category: "agent_setup",
      title: "Codex profile is missing",
      detail: "Add the profile before resuming.",
      severity: "error",
    }],
  });
  assert.equal(withSetupIssue.tone, "warning");
  assert.equal(withSetupIssue.defaultOpen, true);
  assert.deepEqual(withSetupIssue.rows[1], {
    key: "codex-profile",
    title: "Codex profile is missing",
    status: "failed",
    reason: "Add the profile before resuming.",
  });
  assert.equal(withSetupIssue.action?.kind, "restart_codex");

  // A ready verdict leaves the clean success alone.
  const readyOutcome = outcomeFor(completed, {
    bundle_id: binding.bundle_id,
    state: "ready",
    issues: [],
  });
  assert.equal(readyOutcome.tone, "success");
  assert.equal(readyOutcome.title, "Pull complete");
  assert.equal(readyOutcome.defaultOpen, false);
});

test("a valid Pull plan renders an enabled non-modal Apply workspace while supporting checks fail", async () => {
  const readiness = deferred<BundleReadiness>();
  const calls: string[] = [];
  const api: PullReviewApi = {
    planRestore: async () => {
      calls.push("plan-restore");
      return restorePlan;
    },
    getRestoreReadiness: (restorePlanId) => {
      assert.equal(restorePlanId, restorePlan.plan_id);
      calls.push("readiness");
      return readiness.promise;
    },
  };

  const review = await beginPullReview(api, {
    storageId: "storage-local",
    bundleId: binding.bundle_id,
    binding,
  });
  assert.equal(review.restorePlan, restorePlan);
  // Planning fetches the bundle itself — a separate prefetch would download
  // the full payload twice.
  assert.deepEqual(calls, ["plan-restore", "readiness"]);
  let supportFinished = false;
  void review.support.then(() => {
    supportFinished = true;
  });
  await Promise.resolve();
  assert.equal(supportFinished, false, "supporting checks remain non-blocking context");

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={review.restorePlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /sync-pull-review-workspace/);
  assert.doesNotMatch(html, /role="tablist"/, "a single-step review renders no tab strip");
  assert.doesNotMatch(html, /role="tab"/);
  assert.doesNotMatch(html, /aria-modal/);
  assert.doesNotMatch(html, /sync-modal-backdrop/);
  assert.doesNotMatch(html, /Verify project readiness/, "the pre-flight step strip is gone");
  const applyText = html.lastIndexOf(">Pull</");
  assert.notEqual(applyText, -1);
  const applyTagStart = html.lastIndexOf("<button", applyText);
  const applyTagEnd = html.indexOf(">", applyTagStart);
  assert.match(html.slice(applyTagStart, applyTagEnd), /sync-pull-apply-button/);
  assert.doesNotMatch(
    html.slice(applyTagStart, applyTagEnd),
    /disabled/,
    "the default-approved conversation keeps Pull enabled",
  );

  readiness.reject(new Error("readiness unavailable"));
  const support = await review.support;
  assert.equal(support.readiness, null);
  assert.deepEqual(support.errors, [
    "Readiness checks: readiness unavailable",
  ]);
});

test("a Pull review opens on the history step and tabs appear only with a second step", () => {
  const render = (plan: RestorePlan) => renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
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
      error={null}
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );

  const singleStep = render(restorePlan);
  assert.doesNotMatch(singleStep, /role="tablist"/, "one step is not a choice, so no tab strip renders");
  assert.match(singleStep, /id="sync-review-history-panel"/, "the review opens on the history step");
  assert.doesNotMatch(singleStep, />Next:|>Back</, "there is no wizard footer");
  assert.match(singleStep, /svg>Pull<\/button>/, "the footer applies directly");

  const twoStep = render({
    ...restorePlan,
    project_content_eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  });
  assert.match(
    twoStep,
    /aria-selected="true"[^>]*aria-label="Git &amp; sessions, 1 selected"/,
    "the history tab opens selected with its count",
  );
  assert.match(twoStep, /aria-selected="false"[^>]*aria-label="Project files, 0 selected"/);
  assert.equal((twoStep.match(/role="tab"/g) ?? []).length, 2, "only history and project files exist as steps");
  assert.doesNotMatch(twoStep, />Review</);
});

test("the embedded Pull footer omits redundant helper copy", () => {
  const html = renderToStaticMarkup(
    <RestorePlanView
      embedded
      projectName="gam2"
      profileLabel="myconf2"
      plan={restorePlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );

  assert.match(html, /sync-inline-action-footer/);
  assert.doesNotMatch(html, /safe change(?:s)? selected by default/);
});


test("conversation conflicts become one-time keep-local decisions without blocking safe Pull changes", async () => {
  const conflictedPlan: RestorePlan = {
    ...restorePlan,
    actions: [
      ...restorePlan.actions,
      {
        action_id: "manual-conflict",
        resource_id: "codex:session:thread-conflict",
        kind: {
          kind: "manual",
          message: "Conversation differs locally; this Pull will keep the local version.",
        },
        target_path: "/Users/test/.codex/sessions/thread-conflict.jsonl",
        source_sha256: "c".repeat(64),
        expected_target_sha256: "d".repeat(64),
        requires_explicit_approval: true,
      },
      {
        action_id: "restore-agents-definition",
        resource_id: "codex:config:agents",
        kind: { kind: "write_file", logical_path: "state/codex/config/agents.md" },
        target_path: "/Users/test/.codex/config/agents.md",
        source_sha256: "e".repeat(64),
        requires_explicit_approval: true,
      },
    ],
  };
  const items = buildPullReviewItems(conflictedPlan);
  const selection = buildPullReviewSelection(
    items,
    new Set(items.map((item) => item.resourceId)),
    new Set(),
    new Set(),
  );
  assert.deepEqual(
    [...selection.restoreActionIds].sort(),
    ["action-session", "restore-agents-definition"],
  );
  assert.doesNotMatch(selection.restoreActionIds.join(" "), /manual-conflict/);
  assert.deepEqual(
    [...pullSelectableResourceIds(items, new Set(), new Set())].sort(),
    ["codex:session:thread-a", "codex:config:agents"].sort(),
  );

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictedPlan}
      binding={binding}
      readiness={{
        bundle_id: binding.bundle_id,
        state: "needs_setup",
        issues: [{
          issue_id: "restore-files",
          category: "project_files",
          title: "Project files are not fully materialized",
          detail: "Build a restore plan and approve the intended file actions.",
          severity: "warning",
        }],
      }}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );

  // The keep-local conflict renders as a non-selectable row with its decision
  // chip; it never blocks the safe changes around it.
  assert.match(html, /aria-label="Show details for Conflicted conversation"/);
  assert.match(html, /class="sync-pull-item-status keep-local">Keep local</);
  assert.doesNotMatch(
    html,
    /aria-label="Select Conflicted conversation"/,
    "keep-local conflicts expose no checkbox",
  );
  // The optional definition now lives at the bottom of the history step.
  assert.match(html, /<strong>Agent setup &amp; tools<\/strong>/);
  assert.ok(
    html.indexOf('class="sync-pull-items"') >= 0
      && html.indexOf('class="sync-pull-items"') < html.indexOf("sync-sync-review-other"),
    "Agent setup & tools renders below the session list",
  );
  assert.match(html, /aria-label="Select agents\.md"/);
  assert.match(html, /class="sync-pull-item-status needs-approval">Needs approval</);
  assert.doesNotMatch(html, /resolve.*separately/i);
  assert.doesNotMatch(
    html,
    /Project files are not fully materialized/,
    "pre-result readiness issues stay out of the review",
  );
  assert.doesNotMatch(html, /sync-sync-review-summary/, "the review-step summary is gone");
  assert.match(
    html,
    /class="btn btn-primary sync-pull-apply-button">/,
    "the safe selection keeps Pull enabled",
  );
  assert.match(html, /svg>Pull<\/button>/);
  assert.equal(
    (html.match(/sync-pull-item-selectable/g) ?? []).length,
    2,
    "only the safe conversation and the optional definition are selectable",
  );
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
  assert.doesNotMatch(html, /sync-selection-placeholder/);

  const calls: string[] = [];
  const keepLocalOnlyPlan = { ...conflictedPlan, actions: [conflictedPlan.actions[1]] };
  const result = await applyPullReview(
    {
      applyRestore: async (_planId, actionIds) => {
        calls.push(`restore:${actionIds.length}`);
        return {
          success: true,
          status: "complete",
          message: "Local conversation kept",
          plan_id: keepLocalOnlyPlan.plan_id,
          applied_action_ids: [],
          failed_actions: [],
          codex_divergences: [],
        };
      },
      getRestoreReadiness: async () => {
        calls.push("verify");
        return { bundle_id: binding.bundle_id, state: "ready", issues: [] };
      },
    },
    keepLocalOnlyPlan,
    { resourceIds: [], restoreActionIds: [], conversationResolutions: [] },
    () => undefined,
  );
  assert.deepEqual(calls, ["restore:0", "verify"]);
  assert.equal(result.success, true);
});

test("the Apply coordinator sends the empty apply for an acknowledge-only Pull", async () => {
  const calls: string[] = [];
  // The plan holds a real (unselected) action: before, the coordinator
  // silently skipped the backend call, so the reviewed baseline never moved.
  const result = await applyPullReview(
    {
      applyRestore: async (_planId, actionIds) => {
        calls.push(`restore:${actionIds.length}`);
        return {
          success: true,
          status: "complete",
          message: "Applied 0 restore actions",
          plan_id: restorePlan.plan_id,
          reviewed_base_advanced: true,
          applied_action_ids: [],
          failed_actions: [],
          codex_fork_checkpoints: [],
          codex_divergences: [],
        };
      },
      getRestoreReadiness: async () => {
        calls.push("verify");
        return { bundle_id: binding.bundle_id, state: "ready", issues: [] };
      },
    },
    restorePlan,
    { resourceIds: [], restoreActionIds: [], conversationResolutions: [] },
    () => undefined,
  );
  assert.deepEqual(calls, ["restore:0", "verify"]);
  assert.equal(result.success, true);
});

test("an all-optional plan keeps Pull enabled for the acknowledge-only apply", () => {
  const optionalPlan: RestorePlan = {
    ...restorePlan,
    actions: [{
      ...restorePlan.actions[0],
      action_id: "action-optional",
      resource_id: "codex:config:agents",
      kind: { kind: "write_file", logical_path: "state/codex/AGENTS.md" },
      requires_explicit_approval: true,
    }],
  };
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      storageName="Local storage 1"
      profileLabel="myconf2"
      plan={optionalPlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="history"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  // Nothing is selected, yet the primary stays a live Pull: the empty apply
  // advances the reviewed baseline, which is what unblocks Push.
  assert.match(html, /class="btn btn-primary sync-pull-apply-button">/);
  assert.match(html, /svg>Pull<\/button>/);
  assert.doesNotMatch(html, /Nothing to pull is selected/);
});

test("a plan with nothing to review acknowledges without opening the review", () => {
  // The storage-card Pull must work when the storage only moved its
  // generation: planRestore runs the empty apply itself instead of opening
  // an empty review the user has to click through. A replaced history is
  // never adopted silently.
  const source = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(source, /!hasPendingRestoreMutations\(review\.restorePlan\)\s*&&\s*review\.restorePlan\.remote_continuity\?\.state !== "replaced"/);
  assert.match(source, /applyRestore\(review\.restorePlan\.plan_id, \[\], \[\]\)/);
  assert.match(source, /finalization\.reviewed_base_advanced === false/);
});

test("the storage-card Pull button applies the open review", () => {
  // The card button is the review's footer primary, not a focus shortcut:
  // with nothing selected it still applies, because the empty apply is how a
  // generation-only storage change is acknowledged. The review reports its
  // primary upward under the footer's own gating, and the card runs it the
  // same way the push card button runs its onContinue.
  const view = readFileSync("src/components/project-sync/RestorePlanView.tsx", "utf8");
  assert.match(view, /disabledReason: primaryDisabledReason,/);
  assert.match(view, /apply: \(\) => onApplyRef\.current\(selectionRef\.current\),/);

  const sync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(sync, /onPrimaryChange=\{setPullReviewPrimary\}/);
  assert.match(sync, /continueDisabled: !pullReviewPrimary \|\| pullReviewPrimary\.disabledReason !== null,/);

  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.equal(
    (workspace.match(/inlineStorageReview\?\.onContinue\?\.\(\);/g) ?? []).length,
    2,
    "the pull card button applies the open review exactly the way the push one does",
  );
  assert.doesNotMatch(
    workspace,
    /Return to the open Pull review|Return to Pull review/,
    "the focus-only behavior is gone, not just unreachable",
  );
});

test("Pull stays disabled until the storage rows have rendered", () => {
  // With a project and a Codex profile the review embeds the history page,
  // whose timeline and "Only in …" storage rows arrive from async fetches.
  // Until they land, the primary must not apply a sight-unseen selection.
  const historyProject = {
    local_project_id: binding.local_project_id,
    bundle_id: binding.bundle_id,
    display_name: "gam2",
    local_alias: "gam2",
    repository_fingerprint: "fingerprint",
    project_root: binding.project_root,
    canonical_project_root: binding.canonical_project_root,
    profile_ids: { codex: "profile-codex" },
    profile_names: ["Default Codex"],
    created_at: 1,
    updated_at: 2,
    revision: 1,
  };
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      storageName="Local storage 1"
      profileLabel="myconf2"
      project={historyProject as never}
      plan={restorePlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="history"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /disabled="" title="Loading storage changes…"/);
  assert.match(html, /svg>Pull<\/button>/);
});

test("a completed apply that could not advance the baseline says so", () => {
  const stalled: RestoreResult = {
    success: true,
    status: "complete",
    message: "Applied 0 restore actions",
    plan_id: restorePlan.plan_id,
    reviewed_base_advanced: false,
    applied_action_ids: [],
    failed_actions: [],
    codex_fork_checkpoints: [],
    codex_divergences: [],
  };
  const outcomeFor = (result: RestoreResult) => buildPullOutcome({
    projectId: binding.local_project_id,
    plan: restorePlan,
    result,
    readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
    items: buildPullReviewItems(restorePlan),
    completedResourceIds: new Set<string>(),
    error: null,
  });

  // Even with readiness green, a stalled baseline must not read as success.
  const outcome = outcomeFor(stalled);
  assert.equal(outcome.tone, "warning");
  assert.equal(outcome.title, "Pull complete — Push still blocked");
  assert.equal(outcome.defaultOpen, true, "the banner opens on the news it has to deliver");
  assert.deepEqual(outcome.rows, [{
    key: "plan-restore:baseline",
    title: "Push is still blocked",
    status: "held",
    reason: "Required changes were skipped, so this computer stays behind storage. Pull them to unblock Push.",
  }], "the stalled baseline is named in the user's terms, not as a lineage number");
  assert.deepEqual(
    outcome.action,
    { kind: "open_pull", label: "Open Pull review" },
    "the way out is the Pull whose changes were skipped",
  );

  const html = renderOutcome(outcome);
  assert.match(html, /class="sync-callout sync-outcome-notice warning"/);
  assert.match(html, /<strong>Pull complete — Push still blocked<\/strong>/);
  assert.match(
    html,
    /Required changes were skipped, so this computer stays behind storage\. Pull them to unblock Push\./,
  );
  assert.doesNotMatch(html, /sync-outcome-notice success/);

  // The identical apply with an advanced baseline is the clean folded
  // success, so the warning above is not boilerplate.
  const advanced = outcomeFor({ ...stalled, reviewed_base_advanced: true });
  assert.equal(advanced.tone, "success");
  assert.equal(advanced.title, "Pull complete");
  assert.equal(advanced.defaultOpen, false);
  assert.deepEqual(advanced.rows, []);
  assert.equal(advanced.action, undefined);
});

test("stored-only Pull conversations have no checkbox or empty alignment gutter", () => {
  const storedOnlyPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [
      conversationAction("store_only", "codec_unsupported", "unknown"),
    ],
  };
  const items = buildPullReviewItems(storedOnlyPlan);
  assert.deepEqual(
    [...pullSelectableResourceIds(items, new Set(), new Set())],
    [],
  );
  assert.deepEqual(
    buildPullReviewSelection(
      items,
      new Set(["codex:conversation:thread-cloud"]),
      new Set(),
      new Set(),
    ).restoreActionIds,
    [],
  );

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={storedOnlyPlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="history"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, />Backup only</);
  assert.doesNotMatch(html, /type="checkbox"/);
  assert.doesNotMatch(html, /sync-pull-item-select|sync-selection-placeholder/);
});


test("the Apply coordinator restores then verifies, rejecting selections outside the plan", async () => {
  const restoreResult: RestoreResult = {
    success: true,
    status: "complete",
    message: "Applied 1 restore action",
    plan_id: restorePlan.plan_id,
    applied_action_ids: ["action-session"],
    failed_actions: [],
    codex_divergences: [],
  };
  const ready: BundleReadiness = { bundle_id: binding.bundle_id, state: "ready", issues: [] };
  const calls: string[] = [];
  const phases: string[] = [];
  const result = await applyPullReview(
    {
      applyRestore: async () => {
        calls.push("restore");
        return restoreResult;
      },
      getRestoreReadiness: async (restorePlanId) => {
        assert.equal(restorePlanId, restorePlan.plan_id);
        calls.push("verify");
        return ready;
      },
    },
    restorePlan,
    {
      resourceIds: ["codex:session:thread-a"],
      restoreActionIds: ["action-session"],
      conversationResolutions: [],
    },
    (phase) => phases.push(phase),
  );
  assert.deepEqual(calls, ["restore", "verify"]);
  assert.deepEqual(phases, ["restoring", "verifying", "complete"]);
  assert.equal(result.success, true);

  const failingPhases: string[] = [];
  const failing = await applyPullReview(
    {
      applyRestore: async () => {
        throw new Error("restore engine unavailable");
      },
      getRestoreReadiness: async () => ({ ...ready, state: "needs_setup" }),
    },
    restorePlan,
    {
      resourceIds: ["codex:session:thread-a"],
      restoreActionIds: ["action-session"],
      conversationResolutions: [],
    },
    (phase) => failingPhases.push(phase),
  );
  assert.match(failing.error ?? "", /restore engine unavailable/);
  assert.equal(failing.failedPhase, "restoring");
  assert.equal(failing.success, false);
  assert.deepEqual(failingPhases, ["restoring", "verifying", "complete"]);

  await assert.rejects(
    applyPullReview(
      {
        applyRestore: async () => {
          calls.push("unexpected-restore");
          return restoreResult;
        },
        getRestoreReadiness: async () => ready,
      },
      restorePlan,
      {
        resourceIds: ["codex:session:thread-a"],
        restoreActionIds: ["action-from-another-plan"],
        conversationResolutions: [],
      },
      () => undefined,
    ),
    /no longer belong to this Pull review/,
  );
  assert.doesNotMatch(calls.join(","), /unexpected-restore/);
});

test("an unacknowledged fork checkpoint defers only its thread while readiness proceeds", async () => {
  const action = divergenceAction();
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [action],
  };
  const checkpoint: RestoreResult = {
    success: true,
    status: "fork_unacknowledged",
    message: "Applied 0 restore actions; the Codex fork was not acknowledged",
    plan_id: conflictPlan.plan_id,
    applied_action_ids: [],
    failed_actions: [],
    codex_divergences: [],
    codex_fork_checkpoints: [{
      operation_id: "operation-divergence",
      origin: "pull_preserve",
      plan_id: conflictPlan.plan_id,
      action_id: action.action_id,
      thread_id: action.thread_id,
      source_provider_thread_id: action.divergence!.source_provider_thread_id,
      suggested_fork_name: action.divergence!.desired_fork_name,
      status: "fork_unacknowledged",
      message: "Mallard did not acknowledge a fork from this attempt.",
    }],
  };
  const calls: string[] = [];
  const phases: string[] = [];
  const result = await applyPullReview(
    {
      applyRestore: async () => {
        calls.push("restore");
        return checkpoint;
      },
      getRestoreReadiness: async () => {
        calls.push("verify");
        return { bundle_id: binding.bundle_id, state: "needs_setup", issues: [] };
      },
    },
    conflictPlan,
    {
      resourceIds: ["codex:session:thread-shared"],
      restoreActionIds: [action.action_id],
      conversationResolutions: [{
        action_id: action.action_id,
        resolution: "preserve_local_with_native_fork",
        destructive_approved: false,
      }],
    },
    (phase) => phases.push(phase),
  );
  assert.deepEqual(calls, ["restore", "verify"]);
  assert.deepEqual(phases, ["restoring", "verifying", "complete"]);
  assert.equal(result.restoreResult, checkpoint);
  assert.equal(result.success, false, "a pending fork checkpoint keeps the composite Pull unsettled");
  assert.equal(result.failedPhase, null);
});

test("a primary restore-plan failure still rejects the Pull review", async () => {
  const api: PullReviewApi = {
    planRestore: async () => {
      throw new Error("bundle head changed");
    },
    getRestoreReadiness: async () => {
      throw new Error("must not run");
    },
  };
  await assert.rejects(
    beginPullReview(api, {
      storageId: "storage-local",
      bundleId: binding.bundle_id,
      binding,
    }),
    /bundle head changed/,
  );
});
