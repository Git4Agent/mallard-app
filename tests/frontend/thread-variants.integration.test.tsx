import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ThreadVariantReview, {
  conversationActionLabel,
  conversationActionRequiresExplicitApproval,
  conversationActionSelectedByDefault,
  conversationReasonLabel,
  conversationRelationLabel,
  sortedThreadEntries,
} from "../../src/components/project-sync/ThreadVariantReview";
import type {
  ConversationPlanActionType,
  ConversationReasonCode,
  ConversationRelation,
  ThreadSyncEntry,
} from "../../src/types";

function thread(overrides: Partial<ThreadSyncEntry>): ThreadSyncEntry {
  return {
    thread_id: "provider-thread",
    provider_thread_id: "provider-thread",
    resource_id: "codex:conversation:thread-v1",
    display_name: "Same conversation title",
    state: "diverged",
    local_present: true,
    storage_present: true,
    cloud_thread_id: "thread-v1",
    forked_from: null,
    relation: "diverged",
    proposed_action: "fork",
    reason_code: "diverged",
    materialization_state: "resumable",
    resumable: true,
    origin_label: "This host",
    available_actions: ["fork", "replace_cloud", "tombstone_thread"],
    requires_explicit_approval: false,
    ...overrides,
  };
}

test("thread rows stay flat with fork provenance shown only as a hint", () => {
  let openCalls = 0;
  const entries = [
    thread({
      cloud_thread_id: "thread-origin",
      resource_id: "codex:conversation:thread-origin",
      thread_id: "cloud-source-shared",
      provider_thread_id: "provider-cloud-local",
      materialization_state: "stored_only",
      resumable: false,
      origin_label: "Host 7F3A",
      proposed_action: "store_only",
      reason_code: "unknown_lineage",
      relation: "unknown",
      storage_updated_at: 30,
    }),
    thread({
      cloud_thread_id: "thread-forked-local",
      forked_from: "thread-origin",
      resource_id: "codex:conversation:thread-forked-local",
      thread_id: "cloud-source-shared",
      provider_thread_id: "provider-local-active",
      materialization_state: "resumable",
      origin_label: "Work laptop",
      storage_updated_at: 20,
    }),
    thread({
      cloud_thread_id: "thread-pending",
      resource_id: "codex:conversation:thread-pending",
      display_name: "Pending conversation",
      materialization_state: "pending",
      provider_thread_id: "provider-pending",
      storage_updated_at: 10,
    }),
    thread({
      cloud_thread_id: null,
      resource_id: "codex:session:not-a-cloud-thread",
      display_name: "Provider-only row",
    }),
  ];
  const sorted = sortedThreadEntries(entries);
  assert.equal(sorted.length, 3, "rows without a cloud thread are excluded");
  assert.deepEqual(
    sorted.map((entry) => entry.cloud_thread_id),
    ["thread-origin", "thread-forked-local", "thread-pending"],
  );

  const html = renderToStaticMarkup(
    <ThreadVariantReview
      entries={entries}
      onOpenProviderThread={() => { openCalls += 1; }}
      onToggleResource={() => undefined}
    />,
  );
  assert.equal(openCalls, 0, "rendering a Pull review must not open or resume a provider");
  assert.ok((html.match(/Same conversation title/g) ?? []).length >= 2);
  assert.match(html, /data-thread-id="thread-origin"/);
  assert.match(html, /data-thread-id="thread-forked-local"/);
  assert.doesNotMatch(html, /data-conflict-group/);
  assert.equal(
    (html.match(/Forked from another thread/g) ?? []).length,
    1,
    "only the forked thread carries the provenance hint",
  );
  assert.doesNotMatch(html, /Cloud primary/);
  assert.doesNotMatch(html, /Preferred here/);
  assert.match(html, /Stored only/);
  assert.match(html, /Resumable/);
  assert.match(html, /Pending/);
  assert.match(html, /This host/);
  assert.match(html, /Host 7F3A/);
  assert.match(html, /Work laptop/);
  assert.match(html, /<details>.*provider-local-active.*Resume.*<\/details>/s);
  assert.doesNotMatch(html.split("<details>")[0], /provider-local-active/);
});

test("fork is selected by default while forces and deletion require explicit approval", () => {
  assert.equal(
    conversationActionSelectedByDefault(thread({ proposed_action: "fast_forward_local" })),
    true,
  );
  assert.equal(
    conversationActionSelectedByDefault(thread({ proposed_action: "fork" })),
    true,
    "fork is always the pre-selected divergence default",
  );
  assert.equal(
    conversationActionSelectedByDefault(thread({ proposed_action: "create_cloud" })),
    true,
  );
  assert.equal(
    conversationActionSelectedByDefault(thread({ proposed_action: "create_local" })),
    true,
  );
  for (const action of [
    "replace_local",
    "replace_cloud",
    "tombstone_thread",
  ] as const) {
    assert.equal(conversationActionRequiresExplicitApproval(action), true);
    assert.equal(conversationActionSelectedByDefault(thread({ proposed_action: action })), false);
  }
  for (const action of ["no_op", "keep_local", "require_pull", "blocked"] as const) {
    assert.equal(conversationActionSelectedByDefault(thread({ proposed_action: action })), false);
  }
  assert.equal(
    conversationActionSelectedByDefault(thread({
      proposed_action: "fast_forward_cloud",
      requires_explicit_approval: true,
    })),
    false,
  );
});

test("every backend relation action and reason code has typed frontend copy", () => {
  const relations: ConversationRelation[] = [
    "identical",
    "cloud_prefix_of_local",
    "local_prefix_of_cloud",
    "diverged",
    "unknown",
    "local_missing",
    "cloud_missing",
    "both_missing",
  ];
  const actions: ConversationPlanActionType[] = [
    "no_op",
    "fast_forward_local",
    "fast_forward_cloud",
    "keep_local",
    "require_pull",
    "fork",
    "create_cloud",
    "create_local",
    "store_only",
    "replace_local",
    "replace_cloud",
    "tombstone_thread",
    "blocked",
  ];
  const reasons: ConversationReasonCode[] = [
    "identical",
    "structural_rewrite_only",
    "cloud_prefix_of_local",
    "local_prefix_of_cloud",
    "diverged",
    "unknown_lineage",
    "local_missing",
    "cloud_missing",
    "both_missing",
    "provider_not_quiescent",
    "codec_unsupported",
    "ambiguous_binding",
    "missing_binding",
    "tombstone_unchanged",
    "tombstone_modified",
    "stale_plan",
  ];
  relations.forEach((relation) => assert.ok(conversationRelationLabel(relation).length > 0));
  actions.forEach((action) => assert.ok(conversationActionLabel(action).length > 0));
  reasons.forEach((reason) => assert.ok(conversationReasonLabel(reason).length > 0));
  const rendered = renderToStaticMarkup(
    <ThreadVariantReview
      entries={[
        ...actions.map((action, index) => thread({
          cloud_thread_id: `thread-action-${index}`,
          resource_id: `codex:conversation:thread-action-${index}`,
          relation: relations[index % relations.length],
          proposed_action: action,
          reason_code: reasons[index % reasons.length],
          materialization_state: (["resumable", "pending", "stored_only"] as const)[index % 3],
          requires_explicit_approval: conversationActionRequiresExplicitApproval(action),
        })),
        ...reasons.map((reason, index) => thread({
          cloud_thread_id: `thread-reason-${index}`,
          resource_id: `codex:conversation:thread-reason-${index}`,
          relation: relations[index % relations.length],
          proposed_action: "store_only",
          reason_code: reason,
        })),
      ]}
    />,
  );
  actions.forEach((action) => assert.match(rendered, new RegExp(conversationActionLabel(action))));
  relations.forEach((relation) => assert.match(rendered, new RegExp(conversationRelationLabel(relation))));
  reasons.forEach((reason) => {
    assert.match(rendered, new RegExp(conversationReasonLabel(reason).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  });
  assert.equal((rendered.match(/Review required/g) ?? []).length, 3);
  assert.match(conversationReasonLabel("stale_plan"), /Refresh/);
  assert.match(conversationReasonLabel("provider_not_quiescent"), /still be writing/);
  assert.match(conversationReasonLabel("ambiguous_binding"), /More than one/);
});
