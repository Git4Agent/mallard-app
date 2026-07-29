import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  BundleReadiness,
  ConversationPlanAction,
  ConversationPlanActionType,
  ConversationReasonCode,
  ConversationRelation,
  DependencyPlan,
  DependencyResult,
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

test("a valid Pull plan renders an enabled non-modal Apply workspace while supporting checks fail", async () => {
  const dependencies = deferred<DependencyPlan>();
  const readiness = deferred<BundleReadiness>();
  const calls: string[] = [];
  const api: PullReviewApi = {
    planRestore: async () => {
      calls.push("plan-restore");
      return restorePlan;
    },
    planDependencies: (restorePlanId) => {
      assert.equal(restorePlanId, restorePlan.plan_id);
      calls.push("plan-dependencies");
      return dependencies.promise;
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
  assert.deepEqual(calls, ["plan-restore", "plan-dependencies", "readiness"]);
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
      dependencyPlan={null}
      readiness={null}
      restoreResult={null}
      dependencyResult={null}
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

  dependencies.reject(new Error("invalid generated id prefix"));
  readiness.reject(new Error("readiness unavailable"));
  const support = await review.support;
  assert.equal(support.dependencyPlan, null);
  assert.equal(support.readiness, null);
  assert.deepEqual(support.errors, [
    "Dependency checks: invalid generated id prefix",
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
      dependencyPlan={null}
      readiness={null}
      restoreResult={null}
      dependencyResult={null}
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
  assert.match(html, />Next: Skills</, "the first step advances instead of applying");
});

test("the embedded Pull footer omits redundant helper copy", () => {
  const html = renderToStaticMarkup(
    <RestorePlanView
      embedded
      projectName="gam2"
      profileLabel="myconf2"
      plan={restorePlan}
      binding={binding}
      dependencyPlan={null}
      readiness={null}
      restoreResult={null}
      dependencyResult={null}
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

function dependencyPlan(actions: DependencyPlan["actions"]): DependencyPlan {
  return {
    schema_version: 1,
    plan_id: "plan-dependencies",
    storage_id: restorePlan.storage_id,
    bundle_id: restorePlan.bundle_id,
    host_id: restorePlan.host_id,
    replica_id: restorePlan.replica_id,
    expected_host_identity_revision: restorePlan.expected_host_identity_revision,
    generation: restorePlan.generation,
    commit_id: restorePlan.commit_id,
    manifest_sha256: restorePlan.manifest_sha256,
    binding_revision: restorePlan.binding_revision,
    created_at: restorePlan.created_at,
    expires_at: restorePlan.expires_at,
    actions,
    blockers: [],
    warnings: [],
  };
}

test("restore and native dependency actions merge into one global-tool row per resource", () => {
  const mergedRestore: RestorePlan = {
    ...restorePlan,
    actions: [
      {
        action_id: "restore-custom-skill",
        resource_id: "codex:standalone-skill:frontend-skill",
        kind: { kind: "install_custom_skill", provider: "codex", skill_name: "frontend-skill" },
        target_path: "/Users/test/.codex/skills/frontend-skill",
        requires_explicit_approval: true,
      },
    ],
  };
  const dependencies = dependencyPlan([
    {
      action_id: "dependency:codex:skill:frontend-skill",
      resource_id: "codex:standalone-skill:frontend-skill",
      kind: "install_standalone_skill",
      display_name: "frontend-skill",
      provider: "codex",
      argv: ["skill", "add", "frontend-skill"],
      requires_explicit_approval: true,
    },
    {
      action_id: "dependency:codex:plugin:computer-use",
      resource_id: "codex:plugin:computer-use",
      kind: "install_codex_plugin",
      display_name: "computer-use@openai-bundled",
      provider: "codex",
      argv: ["plugin", "add", "computer-use@openai-bundled"],
      requires_explicit_approval: true,
    },
  ]);

  const items = buildPullReviewItems(mergedRestore, dependencies);
  assert.equal(items.length, 2);
  assert.ok(items.every((item) => item.category === "global_tool"));
  const skill = items.find((item) => item.resourceId === "codex:standalone-skill:frontend-skill");
  assert.equal(skill?.restoreActions.length, 1);
  assert.equal(skill?.dependencyActions.length, 1);
  const plugin = items.find((item) => item.resourceId === "codex:plugin:computer-use");
  assert.equal(plugin?.restoreActions.length, 0);
  assert.equal(plugin?.dependencyActions.length, 1);
  const selection = buildPullReviewSelection(
    items,
    new Set(items.map((item) => item.resourceId)),
    new Set(),
    new Set(),
  );
  assert.deepEqual(selection.restoreActionIds, ["restore-custom-skill"]);
  assert.deepEqual(
    [...selection.dependencyActionIds].sort(),
    ["dependency:codex:plugin:computer-use", "dependency:codex:skill:frontend-skill"],
  );

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={mergedRestore}
      binding={binding}
      dependencyPlan={dependencies}
      readiness={null}
      restoreResult={null}
      dependencyResult={null}
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
  assert.doesNotMatch(html, /Plugins &amp; standalone skills/);
  assert.doesNotMatch(html, /Install selected/);
  assert.doesNotMatch(html, /deferred to the native dependency runner/);
  assert.doesNotMatch(html, /Ready to restore|Preparing installer/);
  assert.match(html, /aria-label="Skills, 0 selected, needs attention/);
  assert.match(html, /aria-label="Plugins, 0 selected, needs attention/);
  assert.match(html, />Optional changes not selected</);
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
        action_id: "restore-custom-skill",
        resource_id: "codex:standalone-skill:frontend-skill",
        kind: { kind: "overwrite_custom_skill", provider: "codex", skill_name: "frontend-skill" },
        target_path: "/Users/test/.codex/skills/frontend-skill",
        requires_explicit_approval: true,
      },
    ],
  };
  const items = buildPullReviewItems(conflictedPlan, null);
  const selection = buildPullReviewSelection(
    items,
    new Set(items.map((item) => item.resourceId)),
    new Set(),
    new Set(),
  );
  assert.deepEqual(selection.restoreActionIds, ["action-session", "restore-custom-skill"]);
  assert.doesNotMatch(selection.restoreActionIds.join(" "), /manual-conflict/);
  assert.deepEqual(
    [...pullSelectableResourceIds(items, new Set(), new Set())].sort(),
    ["codex:session:thread-a", "codex:standalone-skill:frontend-skill"].sort(),
  );

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictedPlan}
      binding={binding}
      dependencyPlan={null}
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
      dependencyResult={null}
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
  assert.match(html, /Apply will skip frontend-skill and leave it unchanged/);
  assert.doesNotMatch(html, />Needs approval</);
  assert.doesNotMatch(html, /resolve.*separately/i);
  assert.doesNotMatch(html, /Project files are not fully materialized/);
  assert.match(html, />Apply changes</);

  const historyHtml = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={conflictedPlan}
      binding={binding}
      dependencyPlan={null}
      readiness={null}
      restoreResult={null}
      dependencyResult={null}
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
        return { success: true, message: "Local conversation kept", applied_action_ids: [], failed_actions: [] };
      },
      applyDependencies: async () => {
        throw new Error("no dependency action expected");
      },
      getRestoreReadiness: async () => {
        calls.push("verify");
        return { bundle_id: binding.bundle_id, state: "ready", issues: [] };
      },
    },
    keepLocalOnlyPlan,
    null,
    { resourceIds: [], restoreActionIds: [], dependencyActionIds: [] },
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
  const items = buildPullReviewItems(storedOnlyPlan, null);
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
      dependencyPlan={null}
      readiness={null}
      restoreResult={null}
      dependencyResult={null}
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

test("skipping a required global tool cannot produce a ready Pull outcome", () => {
  const dependencies = dependencyPlan([{
    action_id: "install-plugin",
    resource_id: "codex:plugin:computer-use",
    kind: "install_codex_plugin",
    display_name: "computer-use@openai-bundled",
    provider: "codex",
    argv: ["plugin", "add", "computer-use@openai-bundled"],
    requires_explicit_approval: true,
  }]);
  const applied: RestoreResult = {
    success: true,
    message: "Applied 1 restore action",
    applied_action_ids: ["action-session"],
    failed_actions: [],
  };
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={restorePlan}
      binding={binding}
      dependencyPlan={dependencies}
      readiness={{ bundle_id: binding.bundle_id, state: "ready", issues: [] }}
      restoreResult={applied}
      dependencyResult={null}
      phase="complete"
      supportLoading={false}
      completedActionIds={new Set(["action-session"])}
      completedResourceIds={new Set(["codex:session:thread-a"])}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /Project restored; 1 item left unchanged/);
  assert.doesNotMatch(html, />Pull complete</);
  assert.match(html, /computer-use@openai-bundled(?:<!-- -->)? left unchanged/);
  assert.match(html, />Remaining items</);
  assert.match(html, /Readiness is not confirmed/);
  assert.match(html, /aria-label="Use recommended selection \(0\)"/);
  const doneText = html.lastIndexOf(">Done<");
  assert.notEqual(doneText, -1, "a completed data restore can be dismissed even when optional setup was skipped");
  const doneTagStart = html.lastIndexOf("<button", doneText);
  const doneTagEnd = html.indexOf(">", doneTagStart);
  assert.doesNotMatch(html.slice(doneTagStart, doneTagEnd), /disabled/);
});

test("dependency failures show the plugin name instead of its hashed action id", () => {
  const hashedActionId = "dependency:codex:plugin:sha256-0313d68879b46d2035391eb489f0aae0c13bb8fc40c65494ee596f437bdc7f01";
  const dependencies = dependencyPlan([{
    action_id: hashedActionId,
    resource_id: "codex:plugin:sha256-0313d68879b46d2035391eb489f0aae0c13bb8fc40c65494ee596f437bdc7f01",
    kind: "install_codex_plugin",
    display_name: "documents@openai-primary-runtime",
    provider: "codex",
    argv: ["plugin", "add", "documents@openai-primary-runtime"],
    requires_explicit_approval: true,
  }]);
  const dependencyResult: DependencyResult = {
    success: false,
    message: "Applied 0 dependencies; 1 failed",
    applied_action_ids: [],
    failed_actions: [{
      action_id: hashedActionId,
      display_name: "documents@openai-primary-runtime",
      message: "managed marketplace is unavailable",
    }],
  };

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="gam2"
      profileLabel="myconf2"
      plan={restorePlan}
      binding={binding}
      dependencyPlan={dependencies}
      readiness={{ bundle_id: binding.bundle_id, state: "needs_setup", issues: [] }}
      restoreResult={null}
      dependencyResult={dependencyResult}
      phase="complete"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set([dependencies.actions[0].resource_id])}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );

  assert.match(html, /documents@openai-primary-runtime/);
  assert.match(html, /managed marketplace is unavailable/);
  assert.doesNotMatch(html, new RegExp(hashedActionId));
});

test("the composite Apply coordinator restores, installs, then verifies one aligned generation", async () => {
  const dependencies = dependencyPlan([{
    action_id: "install-plugin",
    resource_id: "codex:plugin:computer-use",
    kind: "install_codex_plugin",
    display_name: "computer-use@openai-bundled",
    provider: "codex",
    argv: ["plugin", "add", "computer-use@openai-bundled"],
    requires_explicit_approval: true,
  }]);
  const restoreResult: RestoreResult = {
    success: true,
    message: "Applied 1 restore action",
    applied_action_ids: ["action-session"],
    failed_actions: [],
  };
  const dependencyResult: DependencyResult = {
    success: true,
    message: "Applied 1 dependency action",
    applied_action_ids: ["install-plugin"],
    failed_actions: [],
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
      applyDependencies: async () => {
        calls.push("install");
        return dependencyResult;
      },
      getRestoreReadiness: async (restorePlanId) => {
        assert.equal(restorePlanId, restorePlan.plan_id);
        calls.push("verify");
        return ready;
      },
    },
    restorePlan,
    dependencies,
    {
      resourceIds: ["codex:session:thread-a", "codex:plugin:computer-use"],
      restoreActionIds: ["action-session"],
      dependencyActionIds: ["install-plugin"],
    },
    (phase) => phases.push(phase),
  );
  assert.deepEqual(calls, ["restore", "install", "verify"]);
  assert.deepEqual(phases, ["restoring", "installing", "verifying", "complete"]);
  assert.equal(result.success, true);

  const partialPhases: string[] = [];
  const partial = await applyPullReview(
    {
      applyRestore: async () => restoreResult,
      applyDependencies: async () => {
        throw new Error("native runner unavailable");
      },
      getRestoreReadiness: async () => ({ ...ready, state: "needs_setup" }),
    },
    restorePlan,
    dependencies,
    {
      resourceIds: ["codex:session:thread-a", "codex:plugin:computer-use"],
      restoreActionIds: ["action-session"],
      dependencyActionIds: ["install-plugin"],
    },
    (phase) => partialPhases.push(phase),
  );
  assert.equal(partial.restoreResult, restoreResult, "an applied restore is retained when native installation throws");
  assert.match(partial.error ?? "", /native runner unavailable/);
  assert.equal(partial.failedPhase, "installing");
  assert.equal(partial.success, false);
  assert.deepEqual(partialPhases, ["restoring", "installing", "verifying", "complete"]);

  await assert.rejects(
    applyPullReview(
      {
        applyRestore: async () => {
          calls.push("unexpected-restore");
          return restoreResult;
        },
        applyDependencies: async () => dependencyResult,
        getRestoreReadiness: async () => ready,
      },
      restorePlan,
      { ...dependencies, generation: dependencies.generation + 1 },
      {
        resourceIds: ["codex:plugin:computer-use"],
        restoreActionIds: [],
        dependencyActionIds: ["install-plugin"],
      },
      () => undefined,
    ),
    /different bundle generations/,
  );
  assert.doesNotMatch(calls.join(","), /unexpected-restore/);

  await assert.rejects(
    applyPullReview(
      {
        applyRestore: async () => restoreResult,
        applyDependencies: async () => dependencyResult,
        getRestoreReadiness: async () => ready,
      },
      restorePlan,
      { ...dependencies, host_id: "ffeeddccbbaa99887766554433221100" },
      {
        resourceIds: ["codex:plugin:computer-use"],
        restoreActionIds: [],
        dependencyActionIds: ["install-plugin"],
      },
      () => undefined,
    ),
    /different bundle generations/,
    "a dependency plan from another host must not share one approval click",
  );

  await assert.rejects(
    applyPullReview(
      {
        applyRestore: async () => restoreResult,
        applyDependencies: async () => dependencyResult,
        getRestoreReadiness: async () => ready,
      },
      restorePlan,
      {
        ...dependencies,
        expected_host_identity_revision: dependencies.expected_host_identity_revision + 1,
      },
      {
        resourceIds: ["codex:plugin:computer-use"],
        restoreActionIds: [],
        dependencyActionIds: ["install-plugin"],
      },
      () => undefined,
    ),
    /different bundle generations/,
    "a dependency plan from another host-identity revision must not share one approval click",
  );

  await assert.rejects(
    applyPullReview(
      {
        applyRestore: async () => restoreResult,
        applyDependencies: async () => dependencyResult,
        getRestoreReadiness: async () => ready,
      },
      restorePlan,
      { ...dependencies, generation: dependencies.generation + 1 },
      {
        resourceIds: ["codex:session:thread-a"],
        restoreActionIds: ["action-session"],
        dependencyActionIds: [],
      },
      () => undefined,
    ),
    /different bundle generations/,
    "a visible stale tool plan invalidates the whole composite review even when no tool is checked",
  );
});

test("a primary restore-plan failure still rejects the Pull review", async () => {
  const api: PullReviewApi = {
    planRestore: async () => {
      throw new Error("bundle head changed");
    },
    planDependencies: async () => {
      throw new Error("must not run");
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
