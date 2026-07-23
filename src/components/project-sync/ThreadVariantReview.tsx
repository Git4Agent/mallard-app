import type {
  ConversationPlanActionType,
  ConversationReasonCode,
  ConversationRelation,
  ThreadSyncEntry,
} from "../../types";

const ACTION_LABELS: Record<ConversationPlanActionType, string> = {
  no_op: "No changes",
  fast_forward_local: "Update this thread",
  fast_forward_cloud: "Update cloud variant",
  keep_local: "Keep local branch",
  require_pull: "Pull required",
  add_alternate: "Keep as another variant",
  store_only: "Store without resume",
  promote_local: "Make preferred here",
  promote_cloud: "Make cloud primary",
  replace_local: "Replace local branch",
  tombstone_variant: "Delete variant",
  tombstone_group: "Delete conversation",
  blocked: "Blocked",
};

const RELATION_LABELS: Record<ConversationRelation, string> = {
  identical: "Identical",
  cloud_prefix_of_local: "Local continuation",
  local_prefix_of_cloud: "Cloud continuation",
  diverged: "Diverged branches",
  unknown: "Ancestry unknown",
  local_missing: "Cloud only",
  cloud_missing: "Local only",
  both_missing: "Missing on both sides",
};

const REASON_LABELS: Record<ConversationReasonCode, string> = {
  identical: "The lineage and semantic content match.",
  structural_rewrite_only: "Only the verified thread ID or project path differs.",
  cloud_prefix_of_local: "This branch continues the cloud variant.",
  local_prefix_of_cloud: "The cloud variant continues this branch.",
  diverged: "Both branches continue from different messages.",
  unknown_lineage: "Mallard cannot prove one safe message ancestry.",
  local_missing: "This variant is not present on this host.",
  cloud_missing: "This variant has not been published.",
  both_missing: "No content is available for this variant.",
  provider_not_quiescent: "The provider may still be writing this thread.",
  codec_unsupported: "This provider representation cannot be resumed safely.",
  ambiguous_binding: "More than one cloud ancestor is compatible.",
  missing_binding: "No verified local-to-cloud binding exists.",
  tombstone_unchanged: "The reviewed unchanged copy can be deleted.",
  tombstone_modified: "The local copy changed and will be preserved.",
  stale_plan: "The reviewed state changed. Refresh before applying.",
};

const EXPLICIT_ACTIONS = new Set<ConversationPlanActionType>([
  "promote_local",
  "promote_cloud",
  "replace_local",
  "tombstone_variant",
  "tombstone_group",
]);

const NON_MUTATING_ACTIONS = new Set<ConversationPlanActionType>([
  "no_op",
  "keep_local",
  "require_pull",
  "blocked",
]);

export function conversationActionLabel(action: ConversationPlanActionType): string {
  return ACTION_LABELS[action];
}

export function conversationRelationLabel(relation: ConversationRelation): string {
  return RELATION_LABELS[relation];
}

export function conversationReasonLabel(reason: ConversationReasonCode): string {
  return REASON_LABELS[reason];
}

export function conversationActionRequiresExplicitApproval(
  action: ConversationPlanActionType,
): boolean {
  return EXPLICIT_ACTIONS.has(action);
}

export function conversationActionSelectedByDefault(entry: ThreadSyncEntry): boolean {
  const action = entry.proposed_action;
  return !!action
    && !NON_MUTATING_ACTIONS.has(action)
    && !entry.requires_explicit_approval
    && !conversationActionRequiresExplicitApproval(action);
}

export function groupThreadVariants(
  entries: readonly ThreadSyncEntry[],
): Array<{ conflictGroupId: string; entries: ThreadSyncEntry[] }> {
  const groups = new Map<string, ThreadSyncEntry[]>();
  for (const entry of entries) {
    if (!entry.conflict_group_id || !entry.variant_id) continue;
    const group = groups.get(entry.conflict_group_id) ?? [];
    group.push(entry);
    groups.set(entry.conflict_group_id, group);
  }
  return [...groups.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([conflictGroupId, variants]) => ({
      conflictGroupId,
      entries: variants.sort((left, right) => (
        Number(!!right.cloud_primary) - Number(!!left.cloud_primary)
        || Number(!!right.local_preferred) - Number(!!left.local_preferred)
        || (left.variant_id ?? "").localeCompare(right.variant_id ?? "")
      )),
    }));
}

function materializationLabel(entry: ThreadSyncEntry): string {
  switch (entry.materialization_state) {
    case "resumable": return "Resumable";
    case "pending": return "Pending";
    case "stored_only": return "Stored only";
    default: return entry.resumable ? "Resumable" : "Not materialized";
  }
}

interface Props {
  entries: readonly ThreadSyncEntry[];
  selectedResourceIds?: ReadonlySet<string>;
  disabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  onOpenProviderThread?: (providerThreadId: string) => void;
}

export default function ThreadVariantReview({
  entries,
  selectedResourceIds,
  disabled = false,
  onToggleResource,
  onOpenProviderThread,
}: Props) {
  const groups = groupThreadVariants(entries);
  if (groups.length === 0) return null;
  return (
    <div className="v3-thread-variant-review" aria-label="Conversation variants">
      {groups.map((group) => (
        <section
          className="v3-thread-variant-group"
          data-conflict-group={group.conflictGroupId}
          key={group.conflictGroupId}
        >
          <header>
            <strong>{group.entries[0]?.display_name || "Untitled conversation"}</strong>
            <span>{group.entries.length} {group.entries.length === 1 ? "variant" : "variants"}</span>
          </header>
          <div className="v3-thread-variant-list">
            {group.entries.map((entry) => {
              const action = entry.proposed_action;
              const selected = selectedResourceIds
                ? selectedResourceIds.has(entry.resource_id)
                : conversationActionSelectedByDefault(entry);
              const selectable = !!action && !NON_MUTATING_ACTIONS.has(action);
              const explicit = !!action && (
                entry.requires_explicit_approval
                || conversationActionRequiresExplicitApproval(action)
              );
              return (
                <article
                  className="v3-thread-variant-row"
                  data-variant-id={entry.variant_id ?? undefined}
                  key={`${entry.resource_id}:${entry.variant_id}`}
                >
                  <div className="v3-thread-variant-main">
                    {selectable && (
                      <input
                        type="checkbox"
                        aria-label={`${selected ? "Exclude" : "Include"} ${entry.display_name} ${entry.variant_id}`}
                        checked={selected}
                        disabled={disabled}
                        onChange={() => onToggleResource?.(entry.resource_id)}
                      />
                    )}
                    <div>
                      <div className="v3-thread-variant-title">
                        <strong>{entry.display_name}</strong>
                        <code>{entry.variant_id}</code>
                      </div>
                      <div className="v3-thread-variant-badges">
                        {entry.cloud_primary && <span>Cloud primary</span>}
                        {entry.local_preferred && <span>Preferred here</span>}
                        <span>{materializationLabel(entry)}</span>
                        {entry.origin_label && <span>{entry.origin_label}</span>}
                        {explicit && <span>Review required</span>}
                      </div>
                      {entry.relation && <p>{conversationRelationLabel(entry.relation)}</p>}
                      {entry.reason_code && <p>{conversationReasonLabel(entry.reason_code)}</p>}
                      {action && <p className="v3-thread-variant-action">{conversationActionLabel(action)}</p>}
                    </div>
                  </div>
                  {entry.provider_thread_id && (
                    <details>
                      <summary>Continuation details</summary>
                      <code>{entry.provider_thread_id}</code>
                      {entry.resumable && onOpenProviderThread && (
                        <button
                          type="button"
                          disabled={disabled}
                          onClick={() => onOpenProviderThread(entry.provider_thread_id!)}
                        >
                          Resume
                        </button>
                      )}
                    </details>
                  )}
                </article>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
