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
  /** What a pull of this item installs from the storage: its conversation
   *  rows' and file actions' storage-side sizes. Null when nothing is sized.
   *  The review meter sums it. */
  pullBytes: number | null;
  /** The stored session's last-message time, for a conversation; null for
   *  anything else, or when the presentation recorded none. */
  storedEndedAt: number | null;
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
      pullBytes: null,
      storedEndedAt: null,
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
    if (action.source_size != null) item.pullBytes = (item.pullBytes ?? 0) + action.source_size;
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
    if (row.source_size != null) item.pullBytes = (item.pullBytes ?? 0) + row.source_size;
    if (row.stored_ended_at != null) item.storedEndedAt = row.stored_ended_at;
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

/** What the current selection would pull: the selected sessions, the selected
 *  agent setup & tools files and the selected project files, each by the
 *  storage-side bytes the review carries for it. Nothing is reserved — a Pull
 *  carries nothing it does not show as a row. */
export function pullBudgetTotal(
  items: PullReviewItem[],
  selected: ReadonlySet<string>,
  limit: number,
): { sessions: number; other: number; projectFiles: number; total: number; limit: number } {
  let sessions = 0;
  let other = 0;
  let projectFiles = 0;
  for (const item of items) {
    if (!selected.has(item.resourceId) || item.pullBytes == null) continue;
    if (item.category === "project_content") projectFiles += item.pullBytes;
    else if (itemKindLabel(item) === "Conversation") sessions += item.pullBytes;
    else other += item.pullBytes;
  }
  return { sessions, other, projectFiles, total: sessions + other + projectFiles, limit };
}

/** The sessions step's recommended set under the limit: every safe agent
 *  setup & tools item, then the safe sessions newest first by the stored
 *  session's last-message time until the pull is full. A strict time prefix,
 *  as on Push: the walk ends at the first session that does not fit, so the
 *  ticked rows are one block at the top of the list; only a session that
 *  could not fit an empty pull is skipped rather than ending the walk.
 *  Whatever is ticked outside `stepItems` — the files step — is charged
 *  first, so the result always passes the meter with it. Returns the step's
 *  recommended ids only. */
export function fitPullRecommendation(
  items: PullReviewItem[],
  stepItems: PullReviewItem[],
  selected: ReadonlySet<string>,
  limit: number,
): Set<string> {
  const stepIds = new Set(stepItems.map((item) => item.resourceId));
  const safe = stepItems.filter(recommendedPullSelection);
  const fitted = new Set(safe
    .filter((item) => itemKindLabel(item) !== "Conversation")
    .map((item) => item.resourceId));
  const charged = new Set([
    ...[...selected].filter((resourceId) => !stepIds.has(resourceId)),
    ...fitted,
  ]);
  const room = limit - pullBudgetTotal(items, charged, limit).total;
  let remaining = room;
  const sessions = safe
    .filter((item) => itemKindLabel(item) === "Conversation")
    .sort((left, right) => (
      (right.storedEndedAt ?? 0) - (left.storedEndedAt ?? 0)
      || (left.resourceId < right.resourceId ? -1 : left.resourceId > right.resourceId ? 1 : 0)
    ));
  for (const item of sessions) {
    const bytes = item.pullBytes ?? 0;
    if (bytes > remaining) {
      if (bytes > room) continue;
      break;
    }
    fitted.add(item.resourceId);
    remaining -= bytes;
  }
  return fitted;
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
