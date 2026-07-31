import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import PushResourceWorkspace, {
  nextPushReviewStep,
  preserveSyncedPushRecipeEntries,
  pushReviewBlockers,
  pushReviewPullHolds,
  recommendedPushSelection,
  sanitizePushSelection,
} from "../../src/components/project-sync/PushResourceWorkspace";
import ResourceInventory from "../../src/components/project-sync/ResourceInventory";
import { recipeWithSelection } from "../../src/components/project-sync/model";
import type {
  BundleRecipe,
  ConversationPlanActionType,
  ConversationReasonCode,
  ProjectResourceDescriptor,
  ThreadSyncComparison,
  ThreadSyncDecision,
} from "../../src/types";

function decision(
  action: ConversationPlanActionType,
  reasonCode: ConversationReasonCode,
  selectable = false,
): ThreadSyncDecision {
  return {
    action,
    reason_code: reasonCode,
    selectable,
    selected_by_default: selectable,
    requires_explicit_approval: false,
  };
}

function memoryResource(
  name: string,
  blockedReason: string | null = null,
): ProjectResourceDescriptor {
  return {
    resource_id: `codex:memory:${name}`,
    kind: "project_memory",
    provider: "codex",
    scope: "provider_state",
    display_name: name,
    provenance: { kind: "unknown" },
    apply_policy: blockedReason ? "explicit_review" : "safe_file",
    codec_version: 1,
    metadata: {},
    category: "conversations",
    logical_paths: [`state/codex/memory/${name}/context.md`],
    default_selected: false,
    blocked_reason: blockedReason,
  };
}

function conversation(resourceId: string): ProjectResourceDescriptor {
  return {
    resource_id: resourceId,
    kind: "conversation",
    provider: "codex",
    scope: "provider_state",
    display_name: resourceId,
    provenance: { kind: "conversation" },
    apply_policy: "safe_file",
    codec_version: 1,
    metadata: {},
    category: "conversations",
  };
}

test("recommended Push selection includes only locally changed sessions", () => {
  const localAhead = conversation("codex:session:local-ahead");
  const synced = conversation("codex:session:synced");
  const storageAhead = conversation("codex:session:storage-ahead");
  const sessionIndex = conversation("codex:session-index");
  const memory = memoryResource("frontend-memory");
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 1, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: "local-ahead",
        resource_id: localAhead.resource_id,
        display_name: "Local ahead",
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("fast_forward_cloud", "cloud_prefix_of_local", true),
        pull_decision: decision("keep_local", "cloud_prefix_of_local"),
      },
      {
        thread_id: "synced",
        resource_id: synced.resource_id,
        display_name: "Synced",
        state: "synced",
        local_present: true,
        storage_present: true,
        push_decision: decision("no_op", "identical"),
        pull_decision: decision("no_op", "identical"),
      },
      {
        thread_id: "storage-ahead",
        resource_id: storageAhead.resource_id,
        display_name: "Storage ahead",
        state: "storage_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("require_pull", "local_prefix_of_cloud"),
        pull_decision: decision("fast_forward_local", "local_prefix_of_cloud", true),
      },
    ],
  };

  const recommended = recommendedPushSelection(
    new Set([synced.resource_id, storageAhead.resource_id, sessionIndex.resource_id, memory.resource_id]),
    [localAhead, synced, storageAhead, sessionIndex, memory],
    comparison,
  );

  assert.deepEqual(
    [...recommended].sort(),
    [localAhead.resource_id, sessionIndex.resource_id, memory.resource_id].sort(),
  );

  const sourceRecipe: BundleRecipe = {
    schema_version: 1,
    revision: 3,
    entries: Object.fromEntries([synced, sessionIndex, memory].map((resource) => [resource.resource_id, {
      resource_id: resource.resource_id,
      apply_policy: resource.apply_policy,
      required: false,
    }])),
  };
  const selectedRecipe: BundleRecipe = {
    ...sourceRecipe,
    entries: { [memory.resource_id]: sourceRecipe.entries[memory.resource_id] },
  };
  const publishedRecipe = preserveSyncedPushRecipeEntries(selectedRecipe, sourceRecipe, comparison);
  assert.deepEqual(
    Object.keys(publishedRecipe.entries).sort(),
    [synced.resource_id, sessionIndex.resource_id, memory.resource_id].sort(),
    "unchecked synced sessions stay stored without appearing in the recommended action set",
  );
});

test("non-uploadable sessions cannot remain in the active Push selection", () => {
  const localAhead = conversation("codex:session:local-ahead");
  const synced = conversation("codex:session:synced");
  const storageAhead = conversation("codex:session:storage-ahead");
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 1, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: "local-ahead",
        resource_id: localAhead.resource_id,
        display_name: "Local ahead",
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("fast_forward_cloud", "cloud_prefix_of_local", true),
        pull_decision: decision("keep_local", "cloud_prefix_of_local"),
      },
      {
        thread_id: "synced",
        resource_id: synced.resource_id,
        display_name: "Synced",
        state: "synced",
        local_present: true,
        storage_present: true,
        push_decision: decision("no_op", "identical"),
        pull_decision: decision("no_op", "identical"),
      },
      {
        thread_id: "storage-ahead",
        resource_id: storageAhead.resource_id,
        display_name: "Storage ahead",
        state: "storage_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("require_pull", "local_prefix_of_cloud"),
        pull_decision: decision("fast_forward_local", "local_prefix_of_cloud", true),
      },
    ],
  };

  assert.deepEqual(
    [...sanitizePushSelection(
      new Set([localAhead.resource_id, synced.resource_id, storageAhead.resource_id]),
      [localAhead, synced, storageAhead],
      comparison,
    )],
    [localAhead.resource_id],
  );
});

test("conversation divergence holds only its thread instead of blocking Push", () => {
  const resource = conversation("codex:session:diverged");
  const entry = {
    thread_id: "provider-diverged",
    resource_id: resource.resource_id,
    display_name: "Diverged",
    state: "diverged" as const,
    local_present: true,
    storage_present: true,
    push_decision: decision("require_pull", "diverged"),
    pull_decision: decision("resolve_divergence", "diverged", true),
  };
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 0, storage: 0, diverged: 1, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [{
      ...entry,
      cloud_thread_id: "thread-diverged",
    }],
  };

  assert.deepEqual(
    pushReviewBlockers(new Set([resource.resource_id]), comparison),
    [],
    "a divergence never disables the whole Push; its row is simply not pushable",
  );
  assert.deepEqual(
    [...sanitizePushSelection(
      new Set([resource.resource_id]),
      [resource],
      comparison,
    )],
    [],
    "the divergent row itself stays unselectable until a Pull resolves it",
  );
  assert.deepEqual(
    pushReviewPullHolds(comparison).map((held) => held.resource_id),
    [resource.resource_id],
    "the held thread is surfaced for the Pull-first affordance",
  );
});

test("blocked resources remain unselectable in the inventory", () => {
  const reason = "memory directory is unreadable";
  const html = renderToStaticMarkup(
    <ResourceInventory
      resources={[
        memoryResource("review-one", reason),
        memoryResource("review-two", reason),
      ]}
      selected={new Set()}
      statuses={new Map()}
      onToggle={() => undefined}
    />,
  );

  assert.equal((html.match(/sync-resource-row blocked/g) ?? []).length, 2);
  assert.doesNotMatch(html, /sync-resource-row-selectable|sync-selection-placeholder/);
  assert.doesNotMatch(html, /type="checkbox"/);
});

test("blocked resources cannot enter a published recipe", () => {
  const allowed = memoryResource("frontend-memory");
  const blocked = memoryResource("broken-memory", "context.md is unreadable");
  const recipe = recipeWithSelection(
    { schema_version: 1, revision: 3, entries: {} },
    [allowed, blocked],
    new Set([allowed.resource_id, blocked.resource_id]),
  );

  assert.deepEqual(Object.keys(recipe.entries), [allowed.resource_id]);
});

test("the push chooser shows one concise selection summary", () => {
  const resource = memoryResource("review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error={null}
      initialStep="review"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, />Push review</);
  assert.match(html, /sync-sync-review-title/);
  assert.doesNotMatch(html, /Choose what to include/);
  assert.match(html, /aria-label="Use recommended selection \(1\)"/);
  assert.match(html, /aria-label="Clear current selection"/);
  assert.match(html, /aria-label="Close push review"/);
  const recommendedAction = html.indexOf('aria-label="Use recommended selection (1)"');
  const clearAction = html.indexOf('aria-label="Clear current selection"');
  const closeAction = html.indexOf('aria-label="Close push review"');
  assert.ok(recommendedAction < clearAction && clearAction < closeAction);
  assert.match(html, />Review</);
  assert.match(html, /Back: Git &amp; sessions/);
  assert.match(html, /btn btn-secondary sync-sync-review-back/);
  assert.match(html, />Push<\/button>/);
  const footer = html.slice(html.indexOf('<footer class="sync-inline-action-footer sync-push-resource-footer'));
  assert.doesNotMatch(footer, /\d+ included|Push \d+ resources?/);
  assert.doesNotMatch(html, /Choose resources to push/);
  assert.doesNotMatch(html, /Selections are saved after Push succeeds/);
  assert.doesNotMatch(html, /last selection|selection will be remembered|resource selected/i);
  assert.doesNotMatch(html, /Remote history is missing/);
});

test("the push chooser gives a minimal recovery path when remote history is missing", () => {
  const resource = memoryResource("review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error={null}
      storageName="R2 storage 1"
      destinationStatus={{
        state: "remote_history_missing",
        base_generation: 3,
      }}
      initialStep="review"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onOpenStorageSettings={() => undefined}
      onUnlinkStorage={() => undefined}
    />,
  );

  assert.match(html, /role="alert"/);
  assert.match(html, />Remote history is missing</);
  assert.match(html, /previously synced generation 3 to R2 storage 1/);
  assert.match(html, /Mallard cannot determine why/);
  assert.match(html, />Storage settings<\/button>/);
  assert.match(html, />Unlink storage…<\/button>/);
  assert.doesNotMatch(html, /was deleted|expired|lifecycle/i);
  const pushButton = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[^>]*>/)?.[0];
  assert.match(pushButton ?? "", /disabled=""/);
});

test("the push chooser provides contextual back and next actions", () => {
  const resource = memoryResource("review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error={null}
      initialStep="history"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, /Next: Review/);
  assert.doesNotMatch(html, /Back: /, "the first step has nothing to go back to");
  assert.equal(nextPushReviewStep("history"), "review");
  assert.equal(nextPushReviewStep("review"), "review");
  assert.equal(
    nextPushReviewStep("history", ["history", "project_files", "review"]),
    "project_files",
  );
  assert.equal(
    nextPushReviewStep("project_files", ["history", "project_files", "review"]),
    "review",
  );
});
