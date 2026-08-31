import type {
  ConversationPlanAction,
  ConversationReviewRow,
  ProviderKey,
  RestoreAction,
  RestorePlan,
} from "../../types";
import type { PullReviewSelection } from "./pullReviewFlow";

type PullReviewCategory = "project_data" | "project_content";

export interface PullReviewItem {
  resourceId: string;
  category: PullReviewCategory;
  /** What kind of thing this is, and what the review list prints. Conversations
   *  share the one label "Conversation" — itemKindLabel and the review's
   *  grouping both read it, so it is not a place to put a session's name. */
  title: string;
  /** A conversation's own name: "" when the stored thread has none, and null
   *  when the item is not a conversation at all. */
  threadTitle: string | null;
  detail: string;
  provider: ProviderKey | null;
  restoreActions: RestoreAction[];
  conversationActions: ConversationPlanAction[];
  conversationReviewRows: ConversationReviewRow[];
  projectContentPath: string | null;
  projectContentEntryType: "file" | "directory" | null;
  projectContentOperation: "add" | "replace" | "create_directory" | "delete_file" | "delete_directory" | null;
  /** Project-relative path the local version is copied to before a diverged
   *  replace — the pull is never lossy. */
  projectContentPreservedCopy: string | null;
}

const NON_DIRECT_RESTORE_KINDS = new Set(["manual"]);

function humanize(value: string): string {
  return value
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function blockedConversationDetail(row: ConversationReviewRow): string {
  switch (row.blocker) {
    case "codec_unsupported":
      return "This task cannot be transformed safely by the active provider codec.";
    case "ambiguous_binding":
      return "More than one local task could match this stored task.";
    case "missing_binding":
      return "Mallard cannot prove which local task this stored task belongs to.";
    case "detached_requires_pull":
      return "This task's local mapping changed. Refresh Pull to establish the current mapping.";
    case "busy":
      return "Another sync operation still owns this task. Finish recovery, then refresh.";
    default:
      return row.blocked_reason ?? "Mallard cannot prove that this task can be changed safely.";
  }
}

function conversationActionDetail(row: ConversationReviewRow): string {
  switch (row.action) {
    case "adopt_cloud_metadata":
      return "Records the reviewed storage version as accepted. No provider files change.";
    case "materialize_cloud":
      return "Brings the reviewed storage history into this task. Existing provider files are retained.";
    case "keep_both":
      return "Keeps the current local history as an independent task and brings the storage history into this task. No messages are deleted.";
    case "create_local":
      return "Creates a local task from the reviewed storage history.";
    case "report_push_eligible":
      return "This computer has newer messages. The task stays local and can be pushed.";
    case "report_local_only":
      return "This task exists only on this computer. Pull leaves it unchanged.";
    case "not_found":
      return "This task no longer exists locally or in storage.";
    case "blocked":
      return blockedConversationDetail(row);
    default:
      return humanize(row.action);
  }
}

function finalPathPart(value: string): string {
  const parts = value.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? value;
}

function restoreActionCopy(action: RestoreAction): {
  title: string;
  detail: string;
  provider: ProviderKey | null;
  category: PullReviewCategory;
  projectContentPath?: string;
  projectContentEntryType?: "file" | "directory";
  projectContentOperation?: PullReviewItem["projectContentOperation"];
  projectContentPreservedCopy?: string | null;
} {
  const kind = action.kind.kind;
  const provider = "provider" in action.kind ? action.kind.provider : null;
  if (kind === "materialize_conversation") {
    return {
      title: "Conversation",
      detail: action.kind.logical_path,
      provider,
      category: "project_data",
    };
  }
  if (kind === "write_project_file") {
    const projectPath = action.kind.logical_path.replace(/^project\//, "");
    return {
      title: finalPathPart(projectPath),
      detail: projectPath,
      provider: null,
      category: "project_content",
      projectContentPath: projectPath,
      projectContentEntryType: "file",
      projectContentOperation: action.expected_target_sha256 ? "replace" : "add",
      projectContentPreservedCopy: action.kind.preserve
        ? action.kind.preserve.copy_logical_path.replace(/^project\//, "")
        : null,
    };
  }
  if (kind === "ensure_project_directory") {
    const projectPath = action.kind.logical_path.replace(/^project\//, "");
    return {
      title: finalPathPart(projectPath),
      detail: projectPath,
      provider: null,
      category: "project_content",
      projectContentPath: projectPath,
      projectContentEntryType: "directory",
      projectContentOperation: "create_directory",
    };
  }
  if (kind === "delete_project_file" || kind === "delete_project_directory") {
    const projectPath = action.kind.logical_path.replace(/^project\//, "");
    const isDirectory = kind === "delete_project_directory";
    return {
      title: finalPathPart(projectPath),
      detail: projectPath,
      provider: null,
      category: "project_content",
      projectContentPath: projectPath,
      projectContentEntryType: isDirectory ? "directory" : "file",
      projectContentOperation: isDirectory ? "delete_directory" : "delete_file",
    };
  }
  if (kind === "write_file" || kind === "merge_file") {
    return {
      title: finalPathPart(action.kind.logical_path),
      detail: action.kind.logical_path,
      provider,
      category: "project_data",
    };
  }
  if (kind === "manual") {
    const conversationProvider = conversationResourceProvider(action.resource_id);
    return {
      title: conversationProvider ? "Conflicted conversation" : "Manual review",
      detail: action.kind.message,
      provider: conversationProvider,
      category: "project_data",
    };
  }
  return {
    title: humanize(kind),
    detail: action.target_path ?? kind,
    provider,
    category: "project_data",
  };
}

/** Merge the plan's file and conversation actions into one logical row per resource ID. */
export function buildPullReviewItems(plan: RestorePlan): PullReviewItem[] {
  const items = new Map<string, PullReviewItem>();
  const ensure = (
    resourceId: string,
    category: PullReviewCategory,
    title: string,
    detail: string,
    provider: ProviderKey | null,
  ) => {
    const existing = items.get(resourceId);
    if (existing) {
      if (provider) existing.provider = provider;
      return existing;
    }
    const item: PullReviewItem = {
      resourceId,
      category,
      title,
      threadTitle: null,
      detail,
      provider,
      restoreActions: [],
      conversationActions: [],
      conversationReviewRows: [],
      projectContentPath: null,
      projectContentEntryType: null,
      projectContentOperation: null,
      projectContentPreservedCopy: null,
    };
    items.set(resourceId, item);
    return item;
  };

  for (const action of plan.actions) {
    const copy = restoreActionCopy(action);
    const item = ensure(
      action.resource_id,
      copy.category,
      copy.title,
      copy.detail,
      copy.provider,
    );
    item.restoreActions.push(action);
    if (copy.projectContentPath) item.projectContentPath = copy.projectContentPath;
    if (copy.projectContentEntryType) item.projectContentEntryType = copy.projectContentEntryType;
    if (copy.projectContentOperation) item.projectContentOperation = copy.projectContentOperation;
    if (copy.projectContentPreservedCopy) item.projectContentPreservedCopy = copy.projectContentPreservedCopy;
  }

  const actionsById = new Map(plan.conversation_actions.map((action) => [action.action_id, action]));
  for (const row of plan.conversation_review) {
    const detail = conversationActionDetail(row);
    const item = ensure(
      row.resource_id,
      "project_data",
      "Conversation",
      detail,
      row.provider,
    );
    item.conversationReviewRows.push(row);
    if (row.action_id) {
      const action = actionsById.get(row.action_id);
      if (action) {
        item.conversationActions.push(action);
      }
    }
    item.detail = detail;
    // Assigned on the item rather than through ensure(): ensure early-returns
    // for a resource it has already seen and drops its title argument, which
    // is why detail is set here too.
    item.threadTitle = row.display_title?.trim() ?? "";
  }

  return [...items.values()].sort((left, right) => {
    const categoryRank: Record<PullReviewCategory, number> = {
      project_data: 0,
      project_content: 1,
    };
    if (left.category !== right.category) {
      return categoryRank[left.category] - categoryRank[right.category];
    }
    return (left.projectContentPath ?? left.title).localeCompare(right.projectContentPath ?? right.title);
  });
}

export function requiredPullProjectDirectoryIds(
  items: PullReviewItem[],
  selected: ReadonlySet<string>,
): Set<string> {
  const directoriesByPath = new Map(items
    .filter((item) => item.category === "project_content" && item.projectContentEntryType === "directory")
    .map((item) => [item.projectContentPath, item.resourceId]));
  const required = new Set<string>();
  for (const item of items) {
    if (!selected.has(item.resourceId) || item.category !== "project_content" || !item.projectContentPath) continue;
    const parts = item.projectContentPath.split("/");
    for (let end = 1; end < parts.length; end += 1) {
      const resourceId = directoriesByPath.get(parts.slice(0, end).join("/"));
      if (resourceId) required.add(resourceId);
    }
  }
  return required;
}

export function includeRequiredPullProjectDirectories(
  items: PullReviewItem[],
  selected: ReadonlySet<string>,
): Set<string> {
  const next = new Set(selected);
  requiredPullProjectDirectoryIds(items, next).forEach((resourceId) => next.add(resourceId));
  return next;
}

export function restoreActionIds(item: PullReviewItem): string[] {
  const fileActions = item.restoreActions
    .filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind))
    .map((action) => action.action_id);
  const conversationActions = item.conversationActions
    .map((action) => action.action_id);
  return [...fileActions, ...conversationActions];
}

export function pendingIds(item: PullReviewItem, completedActionIds: ReadonlySet<string>): string[] {
  return restoreActionIds(item).filter((actionId) => !completedActionIds.has(actionId));
}

export function requiresApproval(item: PullReviewItem): boolean {
  return item.restoreActions
    .filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind))
    .some((action) => action.requires_explicit_approval);
}

/* The review rows are the backend's signal and the only one. A Manual restore
   action can never stand in for them: `manual_action` in bundle_engine.rs is
   its sole constructor and leaves target_path and both digests unset. */
/** The provider that owns a `<provider>:session:<id>` resource, or null when
 *  the id is not a conversation. Parsed rather than matched against a list, so
 *  registering a provider adapter needs no edit here. */
export function conversationResourceProvider(resourceId: string): ProviderKey | null {
  return /^([a-z0-9_-]+):session:/.exec(resourceId)?.[1] ?? null;
}

export function keepsLocalConversation(item: PullReviewItem): boolean {
  return item.conversationReviewRows.some((row) => (
    row.action === "report_push_eligible" || row.action === "report_local_only"
  ));
}

export function requiresManualResolution(item: PullReviewItem): boolean {
  return item.conversationReviewRows.some((row) => row.action === "blocked")
    || (item.restoreActions.some((action) => action.kind.kind === "manual")
      && !keepsLocalConversation(item));
}

export function recommendedPullSelection(item: PullReviewItem): boolean {
  if (item.category === "project_content") {
    /* The files step's safe selection: operations that only add — nothing
       local is replaced or deleted. Every project-content action carries
       requires_explicit_approval (the backend reviews all file writes), so
       the gate here is the operation, and the review header's Recommended
       is the explicit review. This absorbed the toolbar's separate "Select
       safe additions" button, which was this exact predicate under a third
       icon. */
    return item.projectContentOperation === "add"
      || item.projectContentOperation === "create_directory";
  }
  const actions = item.restoreActions.filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind));
  return (actions.length > 0 || item.conversationActions.length > 0)
    && actions.every((action) => !action.requires_explicit_approval);
}

export function itemKindLabel(item: PullReviewItem): string {
  if (
    item.conversationReviewRows.length > 0
    || item.conversationActions.length > 0
    || /^(codex|claude):session:/.test(item.resourceId)
    || item.restoreActions.some((action) => action.kind.kind === "materialize_conversation")
  ) return "Conversation";
  if (item.projectContentEntryType === "directory") return "Project folder";
  if (item.projectContentEntryType === "file") return "Project file";
  if (item.restoreActions.some((action) => action.kind.kind === "write_file" || action.kind.kind === "merge_file")) return "File";
  return "Definition";
}

export function buildPullReviewSelection(
  items: PullReviewItem[],
  selected: ReadonlySet<string>,
  completedActionIds: ReadonlySet<string>,
  completedResourceIds: ReadonlySet<string>,
): PullReviewSelection {
  const chosen = items.filter((item) => selected.has(item.resourceId) && !completedResourceIds.has(item.resourceId));
  return {
    resourceIds: chosen
      .filter((item) => pendingIds(item, completedActionIds).length > 0)
      .map((item) => item.resourceId),
    restoreActionIds: chosen.flatMap((item) => restoreActionIds(item).filter((id) => !completedActionIds.has(id))),
  };
}
