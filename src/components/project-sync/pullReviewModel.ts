import type {
  ConversationReviewRow,
  ProviderKey,
  RestoreAction,
  PullReview,
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
  conversationReviewRows: ConversationReviewRow[];
  projectContentPath: string | null;
  projectContentEntryType: "file" | "directory" | null;
  projectContentOperation: "add" | "replace" | "create_directory" | null;
  /** Project-relative path the local version is copied to before a diverged
   *  replace — the pull is never lossy. */
  projectContentPreservedCopy: string | null;
  /** Storage-side size of the bytes an apply would write, when known. */
  projectContentSize: number | null;
}

const VISIBLE_PULL_CONVERSATION_ACTIONS = new Set([
  "materialize_cloud",
  "keep_both",
  "create_local",
]);

function humanize(value: string): string {
  return value
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function conversationActionDetail(row: ConversationReviewRow): string {
  switch (row.action) {
    case "materialize_cloud":
      return "Brings the reviewed storage history into this task. Existing provider files are retained.";
    case "keep_both":
      if (row.blocked_reason) return row.blocked_reason;
      return "Keeps the current local history as an independent task and brings the storage history into this task. No messages are deleted.";
    case "create_local":
      return "Creates a local task from the reviewed storage history.";
    default:
      return humanize(row.action);
  }
}

function finalPathPart(value: string): string {
  const parts = value.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? value;
}

function isPinnedProjectContentNoop(action: RestoreAction): boolean {
  if (action.kind.kind === "ensure_project_directory") {
    return action.expected_target_mode === action.kind.mode;
  }
  return action.kind.kind === "write_project_file"
    && action.materialized_sha256 != null
    && action.materialized_sha256 === action.expected_target_sha256
    && action.expected_target_mode === action.kind.mode;
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
  projectContentSize?: number | null;
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
      projectContentSize: action.source_size ?? null,
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
  if (kind === "write_file" || kind === "merge_file") {
    return {
      title: finalPathPart(action.kind.logical_path),
      detail: action.kind.logical_path,
      provider,
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

/** Group file comparisons and conversation review rows by resource ID. */
export function buildPullReviewItems(plan: PullReview): PullReviewItem[] {
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
      conversationReviewRows: [],
      projectContentPath: null,
      projectContentEntryType: null,
      projectContentOperation: null,
      projectContentPreservedCopy: null,
      projectContentSize: null,
    };
    items.set(resourceId, item);
    return item;
  };

  for (const action of plan.actions) {
    if (isPinnedProjectContentNoop(action)) continue;
    if (
      action.kind.kind === "manual"
      || action.kind.kind === "delete_project_file"
      || action.kind.kind === "delete_project_directory"
    ) continue;
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
    if (copy.projectContentSize != null) item.projectContentSize = copy.projectContentSize;
  }

  for (const row of plan.conversation_review) {
    if (!VISIBLE_PULL_CONVERSATION_ACTIONS.has(row.action)) continue;
    const detail = conversationActionDetail(row) + (row.stored_without_credential_redaction ? " Stored without credential redaction." : "");
    const item = ensure(
      row.resource_id,
      "project_data",
      "Conversation",
      detail,
      row.provider,
    );
    item.conversationReviewRows.push(row);
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
  const fileActions = item.restoreActions.map((action) => action.action_id);
  const conversationSelections = item.conversationReviewRows
    .flatMap((row) => row.action_id ? [row.action_id] : []);
  return [...fileActions, ...conversationSelections];
}

/** Apply receipts use the selection IDs from Review. Executable conversation
 *  actions are built by the backend only after Apply starts. */
export function pullActionResourceIds(plan: PullReview): Map<string, string> {
  const resourceIds = new Map(plan.actions.map((action) => [action.action_id, action.resource_id]));
  for (const row of plan.conversation_review) {
    if (row.action_id) resourceIds.set(row.action_id, row.resource_id);
  }
  return resourceIds;
}

export function requiresApproval(item: PullReviewItem): boolean {
  return item.restoreActions.some((action) => action.requires_explicit_approval);
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
  const actions = item.restoreActions;
  const conversationSelections = item.conversationReviewRows.some((row) => row.action_id != null);
  return (actions.length > 0 || conversationSelections)
    && actions.every((action) => !action.requires_explicit_approval);
}

export function itemKindLabel(item: PullReviewItem): string {
  if (
    item.conversationReviewRows.length > 0
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
): PullReviewSelection {
  const chosen = items.filter((item) => selected.has(item.resourceId) && restoreActionIds(item).length > 0);
  return {
    resourceIds: chosen.map((item) => item.resourceId),
    restoreActionIds: chosen.flatMap(restoreActionIds),
  };
}
