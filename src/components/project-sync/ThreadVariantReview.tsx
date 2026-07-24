import type {
  ConversationPlanActionType,
  ConversationReasonCode,
  ConversationRelation,
  ThreadSyncEntry,
} from "../../types";

const ACTION_LABELS: Record<ConversationPlanActionType, string> = {
  no_op: "No changes",
  fast_forward_local: "Update this thread",
  fast_forward_cloud: "Update cloud thread",
  keep_local: "Keep local branch",
  require_pull: "Pull required",
  fork: "Fork into a new thread",
  create_cloud: "Publish as new thread",
  create_local: "Pull as new conversation",
  store_only: "Store without resume",
  replace_local: "Replace local branch",
  replace_cloud: "Replace cloud thread",
  tombstone_thread: "Delete thread",
  blocked: "Blocked",
};

const RELATION_LABELS: Record<ConversationRelation, string> = {
  identical: "Identical",
  cloud_prefix_of_local: "Local continuation",
  local_prefix_of_cloud: "Cloud continuation",
  diverged: "Diverged — will fork",
  unknown: "Ancestry unknown",
  local_missing: "Cloud only",
  cloud_missing: "Local only",
  both_missing: "Missing on both sides",
};

const REASON_LABELS: Record<ConversationReasonCode, string> = {
  identical: "The lineage and semantic content match.",
  structural_rewrite_only: "Only the verified thread ID or project path differs.",
  cloud_prefix_of_local: "This branch continues the cloud thread.",
  local_prefix_of_cloud: "The cloud thread continues this branch.",
  diverged: "Both branches continue from different messages. Forking keeps both.",
  unknown_lineage: "Mallard cannot prove one safe message ancestry. Forking keeps both.",
  local_missing: "This thread is not present on this host.",
  cloud_missing: "This conversation has not been published.",
  both_missing: "No content is available for this thread.",
  provider_not_quiescent: "The provider may still be writing this thread.",
  codec_unsupported: "This provider representation cannot be resumed safely.",
  ambiguous_binding: "More than one cloud ancestor is compatible.",
  missing_binding: "No verified local-to-cloud binding exists.",
  tombstone_unchanged: "The reviewed unchanged copy can be deleted.",
  tombstone_modified: "The local copy changed and will be preserved.",
  stale_plan: "The reviewed state changed. Refresh before applying.",
};

const EXPLICIT_ACTIONS = new Set<ConversationPlanActionType>([
  "replace_local",
  "replace_cloud",
  "tombstone_thread",
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

export function sortedThreadEntries(
  entries: readonly ThreadSyncEntry[],
): ThreadSyncEntry[] {
  return entries
    .filter((entry) => !!entry.cloud_thread_id)
    .sort((left, right) => (
      (right.storage_updated_at ?? 0) - (left.storage_updated_at ?? 0)
      || (left.cloud_thread_id ?? "").localeCompare(right.cloud_thread_id ?? "")
    ));
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
  const threads = sortedThreadEntries(entries);
  if (threads.length === 0) return null;
  return (
    <div className="v3-thread-variant-review" aria-label="Conversation threads">
      <div className="v3-thread-variant-list">
        {threads.map((entry) => {
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
              data-thread-id={entry.cloud_thread_id ?? undefined}
              key={`${entry.resource_id}:${entry.cloud_thread_id}`}
            >
              <div className="v3-thread-variant-main">
                {selectable && (
                  <input
                    type="checkbox"
                    aria-label={`${selected ? "Exclude" : "Include"} ${entry.display_name}`}
                    checked={selected}
                    disabled={disabled}
                    onChange={() => onToggleResource?.(entry.resource_id)}
                  />
                )}
                <div>
                  <div className="v3-thread-variant-title">
                    <strong>{entry.display_name}</strong>
                  </div>
                  <div className="v3-thread-variant-badges">
                    {entry.forked_from && <span>Forked from another thread</span>}
                    {entry.state === "local_only" && <span>Not yet published</span>}
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
    </div>
  );
}
