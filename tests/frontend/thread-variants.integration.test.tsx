import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ThreadVariantReview, {
  conversationActionLabel,
  conversationActionRequiresExplicitApproval,
  conversationActionSelectedByDefault,
  conversationReasonLabel,
  conversationRelationLabel,
  groupThreadVariants,
} from "../../src/components/project-sync/ThreadVariantReview";
import type {
  ConversationPlanActionType,
  ConversationReasonCode,
  ConversationRelation,
  ThreadSyncEntry,
} from "../../src/types";

function variant(overrides: Partial<ThreadSyncEntry>): ThreadSyncEntry {
  return {
    thread_id: "provider-thread",
    provider_thread_id: "provider-thread",
    resource_id: "codex:conversation:group-main:variant-v1",
    display_name: "Same conversation title",
    state: "diverged",
    local_present: true,
    storage_present: true,
    conflict_group_id: "group-main",
    variant_id: "variant-v1",
    relation: "diverged",
    proposed_action: "add_alternate",
    reason_code: "diverged",
    cloud_primary: false,
    local_preferred: false,
    materialization_state: "resumable",
    resumable: true,
    origin_label: "This host",
    available_actions: ["add_alternate", "promote_cloud", "tombstone_variant"],
    requires_explicit_approval: false,
    ...overrides,
  };
}

test("thread variants stay grouped while cloud primary and local preference remain independent", () => {
  let openCalls = 0;
  const entries = [
    variant({
      variant_id: "variant-cloud",
      resource_id: "codex:conversation:group-main:variant-cloud",
      thread_id: "cloud-source-shared",
      provider_thread_id: "provider-cloud-local",
      cloud_primary: true,
      local_preferred: false,
      materialization_state: "stored_only",
      resumable: false,
      origin_label: "Host 7F3A",
      proposed_action: "store_only",
      reason_code: "unknown_lineage",
      relation: "unknown",
    }),
    variant({
      variant_id: "variant-local",
      resource_id: "codex:conversation:group-main:variant-local",
      thread_id: "cloud-source-shared",
      provider_thread_id: "provider-local-active",
      cloud_primary: false,
      local_preferred: true,
      materialization_state: "resumable",
      origin_label: "Work laptop",
    }),
    variant({
      conflict_group_id: "group-pending",
      variant_id: "variant-pending",
      resource_id: "codex:conversation:group-pending:variant-pending",
      display_name: "Pending conversation",
      materialization_state: "pending",
      provider_thread_id: "provider-pending",
    }),
  ];
  const groups = groupThreadVariants(entries);
  assert.equal(groups.length, 2);
  assert.deepEqual(
    groups[0].entries.map((entry) => entry.variant_id),
    ["variant-cloud", "variant-local"],
  );

  const html = renderToStaticMarkup(
    <ThreadVariantReview
      entries={entries}
      onOpenProviderThread={() => { openCalls += 1; }}
      onToggleResource={() => undefined}
    />,
  );
  assert.equal(openCalls, 0, "rendering a Pull review must not open or resume a provider");
  assert.ok((html.match(/Same conversation title/g) ?? []).length >= 3);
  assert.match(html, /data-conflict-group="group-main"/);
  assert.match(html, /data-variant-id="variant-cloud"/);
  assert.match(html, /data-variant-id="variant-local"/);
  assert.match(html, /Cloud primary/);
  assert.match(html, /Preferred here/);
  assert.match(html, /Stored only/);
  assert.match(html, /Resumable/);
  assert.match(html, /Pending/);
  assert.match(html, /This host/);
  assert.match(html, /Host 7F3A/);
  assert.match(html, /Work laptop/);
  assert.match(html, /<details>.*provider-local-active.*Resume.*<\/details>/s);
  assert.doesNotMatch(html.split("<details>")[0], /provider-local-active/);
});

test("safe defaults exclude promotion replacement deletion and blocked actions", () => {
  assert.equal(
    conversationActionSelectedByDefault(variant({ proposed_action: "fast_forward_local" })),
    true,
  );
  assert.equal(
    conversationActionSelectedByDefault(variant({ proposed_action: "add_alternate" })),
    true,
  );
  for (const action of [
    "promote_local",
    "promote_cloud",
    "replace_local",
    "tombstone_variant",
    "tombstone_group",
  ] as const) {
    assert.equal(conversationActionRequiresExplicitApproval(action), true);
    assert.equal(conversationActionSelectedByDefault(variant({ proposed_action: action })), false);
  }
  for (const action of ["no_op", "keep_local", "require_pull", "blocked"] as const) {
    assert.equal(conversationActionSelectedByDefault(variant({ proposed_action: action })), false);
  }
  assert.equal(
    conversationActionSelectedByDefault(variant({
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
    "add_alternate",
    "store_only",
    "promote_local",
    "promote_cloud",
    "replace_local",
    "tombstone_variant",
    "tombstone_group",
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
        ...actions.map((action, index) => variant({
          conflict_group_id: `group-action-${index}`,
          variant_id: `variant-action-${index}`,
          resource_id: `codex:conversation:group-action-${index}:variant-action-${index}`,
          relation: relations[index % relations.length],
          proposed_action: action,
          reason_code: reasons[index % reasons.length],
          materialization_state: (["resumable", "pending", "stored_only"] as const)[index % 3],
          requires_explicit_approval: conversationActionRequiresExplicitApproval(action),
        })),
        ...reasons.map((reason, index) => variant({
          conflict_group_id: `group-reason-${index}`,
          variant_id: `variant-reason-${index}`,
          resource_id: `codex:conversation:group-reason-${index}:variant-reason-${index}`,
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
  assert.equal((rendered.match(/Review required/g) ?? []).length, 5);
  assert.match(conversationReasonLabel("stale_plan"), /Refresh/);
  assert.match(conversationReasonLabel("provider_not_quiescent"), /still be writing/);
  assert.match(conversationReasonLabel("ambiguous_binding"), /More than one/);
});
