import assert from "node:assert/strict";
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

  assert.match(html, />Storage history changed</);
  assert.match(html, /Applying this Pull adopts the replacement history/);
  assert.match(html, /unlinking does not delete files/);
  assert.match(html, />Unlink storage<\/button>/);
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
      available_executors: ["app_server_stdio", "native_pty", "manual"],
      pre_existing_children: [],
    },
  };
}

test("conversation divergence defaults to keep-both and renders the reviewed fork contract", () => {
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
  assert.match(html, /Conversation conflict/);
  assert.match(html, /Keep both branches <em>Recommended<\/em>/);
  assert.match(html, /Use cloud version only/);
  assert.match(html, /app-server → native PTY → manual/);
  assert.match(html, /local before Pull/);
  assert.match(
    html,
    /type="radio"[^>]*checked=""/,
    "the keep-both radio is selected initially",
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
  assert.match(html, /Adopt cloud version <em>Recommended<\/em>/);
  assert.match(html, /already preserved in a Codex fork/);
  assert.match(html, /Fork again/);
  assert.doesNotMatch(
    html,
    /I understand the divergent local branch/,
    "the witnessed overwrite renders no destructive acknowledgement",
  );
  assert.doesNotMatch(html, /Keep both branches <em>Recommended<\/em>/);
});

test("manual fork checkpoints expose only verify and cancel continuation actions", () => {
  const action = divergenceAction();
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [action],
  };
  const checkpoint: RestoreResult = {
    success: false,
    status: "awaiting_manual_fork",
    message: "Create the reviewed fork in Codex.",
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
      status: "awaiting_manual_fork",
      message: "Create the reviewed fork in Codex, then verify it here",
    }],
  };
  const html = renderToStaticMarkup(
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
      onVerifyCodexFork={() => undefined}
      onCancelCodexFork={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /Create the local fork in Codex/);
  assert.match(html, /Finish any active turn first/);
  assert.match(html, /without sending a new message/);
  assert.match(html, /Verify fork/);
  assert.match(html, />Cancel</);
  assert.doesNotMatch(html, /Use cloud version only/);
  assert.doesNotMatch(html, /Force/);
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

test("a fork checkpoint queue verifies one fork at a time in the same review", () => {
  const first = divergenceAction();
  const second = secondDivergenceAction();
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [first, second],
  };
  const queued: RestoreResult = {
    success: true,
    status: "awaiting_manual_fork",
    message: "Applied 0 restore actions; 2 Codex fork checkpoint(s) need verification",
    plan_id: conflictPlan.plan_id,
    applied_action_ids: [],
    failed_actions: [],
    codex_divergences: [],
    codex_fork_checkpoints: [
      {
        operation_id: "operation-divergence-first",
        plan_id: conflictPlan.plan_id,
        action_id: first.action_id,
        thread_id: first.thread_id,
        source_provider_thread_id: first.divergence!.source_provider_thread_id,
        suggested_fork_name: first.divergence!.desired_fork_name,
        status: "awaiting_manual_fork",
        message: "Create the reviewed fork in Codex, then verify it here",
      },
      {
        operation_id: "operation-divergence-second",
        plan_id: conflictPlan.plan_id,
        action_id: second.action_id,
        thread_id: second.thread_id,
        source_provider_thread_id: second.divergence!.source_provider_thread_id,
        suggested_fork_name: second.divergence!.desired_fork_name,
        status: "awaiting_manual_fork",
        message: "Create the reviewed fork in Codex, then verify it here",
      },
    ],
  };
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictPlan}
      binding={binding}
      readiness={null}
      restoreResult={queued}
      phase="complete"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onVerifyCodexFork={() => undefined}
      onCancelCodexFork={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /Fork checkpoint 1 of 2/);
  assert.match(html, /the rest of this Pull is already applied/);
  assert.match(html, /019fac21-4aba-7fc2-bb7d-df711c950987/);
  assert.doesNotMatch(
    html,
    /019fac21-4aba-7fc2-bb7d-df711c950988/,
    "only the first checkpoint of the queue is actionable at a time",
  );
  assert.match(html, /Verify fork/);
  assert.match(html, />Cancel</);
});

test("multiple conversation conflicts stay distinguishable by title", () => {
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
  assert.match(html, /Conversation conflict — Analyze codebase knowledge graph/);
  assert.match(html, /Conversation conflict — Draft the release notes/);
});

test("completed divergence reports both identities, naming outcomes, refresh warning, and optional App restart", () => {
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
  const renderCompleted = (restoreResult: RestoreResult) => renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictPlan}
      binding={binding}
      readiness={null}
      restoreResult={restoreResult}
      phase="complete"
      supportLoading={false}
      completedActionIds={new Set([action.action_id])}
      completedResourceIds={new Set([action.source_resource_id!])}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onRestartCodexApp={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  const html = renderCompleted(completed);
  assert.match(html, /Both Codex branches preserved/);
  assert.match(html, /019fac21-4aba-7fc2-bb7d-df711c950987/);
  assert.match(html, /019fac73-c1ff-79b3-bb8e-1673ed0d644c/);
  assert.match(html, /native PTY/);
  assert.match(html, /Codex conversation discovery refresh failed/);
  assert.match(html, /My active Codex chats are complete/);
  assert.match(html, /Restart Codex App/);
  const deferredHtml = renderCompleted({
    ...completed,
    codex_divergences: completed.codex_divergences.map((divergence) => ({
      ...divergence,
      inventory_refresh_status: "deferred",
    })),
  });
  assert.match(deferredHtml, /finishing in the background/);
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
  assert.match(html, />Git &amp; sessions</);
  assert.match(html, />Review</);
  assert.doesNotMatch(html, /Project files &amp; setup/, "an empty setup group is omitted");
  assert.doesNotMatch(html, /aria-modal/);
  assert.doesNotMatch(html, /sync-modal-backdrop/);
  const applyText = html.lastIndexOf("Apply changes");
  assert.notEqual(applyText, -1);
  const applyTagStart = html.lastIndexOf("<button", applyText);
  const applyTagEnd = html.indexOf(">", applyTagStart);
  assert.doesNotMatch(
    html.slice(applyTagStart, applyTagEnd),
    /disabled/,
    "the default-approved conversation keeps Apply enabled",
  );

  readiness.reject(new Error("readiness unavailable"));
  const support = await review.support;
  assert.equal(support.readiness, null);
  assert.deepEqual(support.errors, [
    "Readiness checks: readiness unavailable",
  ]);
});

test("a Pull review opens on the first step by default", () => {
  const html = renderToStaticMarkup(
    <RestorePlanView
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

  assert.match(html, /aria-selected="true"[^>]*aria-label="Git &amp; sessions, 1 selected"/);
  assert.doesNotMatch(html, /aria-selected="true"[^>]*aria-label="Review"/);
  assert.match(html, />Next: Review</, "the first step advances instead of applying");
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

  assert.match(html, /aria-label="Git &amp; sessions, 1 selected, needs attention/);
  assert.match(html, /1 conversation differs/);
  assert.match(html, /establish a comparison baseline/);
  assert.match(html, /It can be pushed later/);
  assert.match(html, />Optional change not selected</);
  assert.match(html, /Apply will skip agents\.md and leave it unchanged/);
  assert.doesNotMatch(html, /resolve.*separately/i);
  assert.doesNotMatch(html, /Project files are not fully materialized/);
  assert.match(html, />Apply changes</);

  const historyHtml = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictedPlan}
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
  assert.equal((historyHtml.match(/sync-pull-item-selectable/g) ?? []).length, 1);
  assert.doesNotMatch(historyHtml, /sync-selection-placeholder/);
  assert.equal((historyHtml.match(/type="checkbox"/g) ?? []).length, 1);

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

test("a manual fork checkpoint defers only its thread while verification proceeds", async () => {
  const action = divergenceAction();
  const conflictPlan: RestorePlan = {
    ...restorePlan,
    actions: [],
    conversation_actions: [action],
  };
  const checkpoint: RestoreResult = {
    success: true,
    status: "awaiting_manual_fork",
    message: "Applied 0 restore actions; 1 Codex fork checkpoint(s) need verification",
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
      status: "awaiting_manual_fork",
      message: "Create the reviewed fork in Codex, then verify it here",
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
