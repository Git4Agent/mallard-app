import type {
  ConversationPlanAction,
  ConversationResolutionSelection,
  ProjectProvider,
  RestoreAction,
  RestorePlan,
} from "../../types";
import type { PullReviewSelection } from "./pullReviewFlow";

type PullReviewCategory = "project_data" | "project_content";

export interface PullReviewItem {
  resourceId: string;
  category: PullReviewCategory;
  title: string;
  detail: string;
  provider: ProjectProvider | null;
  restoreActions: RestoreAction[];
  conversationActions: ConversationPlanAction[];
  projectContentPath: string | null;
  projectContentEntryType: "file" | "directory" | null;
  projectContentOperation: "add" | "replace" | "create_directory" | "delete_file" | "delete_directory" | null;
}

const NON_DIRECT_RESTORE_KINDS = new Set(["manual"]);

function humanize(value: string): string {
  return value
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function finalPathPart(value: string): string {
  const parts = value.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? value;
}

function restoreActionCopy(action: RestoreAction): {
  title: string;
  detail: string;
  provider: ProjectProvider | null;
  category: PullReviewCategory;
  projectContentPath?: string;
  projectContentEntryType?: "file" | "directory";
  projectContentOperation?: PullReviewItem["projectContentOperation"];
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
    const conversationProvider = action.resource_id.startsWith("codex:session:")
      ? "codex"
      : action.resource_id.startsWith("claude:session:")
        ? "claude"
        : null;
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
    provider: ProjectProvider | null,
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
      detail,
      provider,
      restoreActions: [],
      conversationActions: [],
      projectContentPath: null,
      projectContentEntryType: null,
      projectContentOperation: null,
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
  }

  for (const action of plan.conversation_actions ?? []) {
    const resourceId = action.source_resource_id
      ?? `${action.provider}:conversation:${action.thread_id}`;
    const detail = action.action === "resolve_divergence"
      ? "Preserves the local branch as a Codex-native fork, then restores the cloud branch into the original conversation."
      : action.action === "create_local"
        ? "Restores this cloud thread as a new local conversation."
        : action.action === "fast_forward_local"
          ? "Updates the local conversation to the reviewed cloud continuation."
          : action.action === "store_only"
            ? "Stores the cloud transcript without offering unsafe resume behavior."
            : action.action === "keep_local"
              ? "The local branch is ahead and will remain unchanged."
              : action.action === "no_op"
                ? "This conversation is already up to date."
                : action.action === "blocked"
                  ? "This conversation cannot be changed safely."
                  : humanize(action.action);
    const item = ensure(
      resourceId,
      "project_data",
      "Conversation",
      detail,
      action.provider,
    );
    item.conversationActions.push(action);
    item.detail = detail;
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
    .filter((action) => conversationActionSelectable(action))
    .map((action) => action.action_id);
  return [...fileActions, ...conversationActions];
}

export function pendingIds(item: PullReviewItem, completedActionIds: ReadonlySet<string>): string[] {
  return restoreActionIds(item).filter((actionId) => !completedActionIds.has(actionId));
}

export function requiresApproval(item: PullReviewItem): boolean {
  return item.restoreActions
    .filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind))
    .some((action) => action.requires_explicit_approval)
    || item.conversationActions.some((action) => conversationActionRequiresApproval(action));
}

export function keepsLocalConversation(item: PullReviewItem): boolean {
  return item.conversationActions.some((action) => (
    action.action === "keep_local" || action.action === "require_pull"
  )) || (/^(codex|claude):session:/.test(item.resourceId)
    && item.restoreActions.some((action) => (
      action.kind.kind === "manual"
      && Boolean(action.target_path)
      && Boolean(action.source_sha256)
      && Boolean(action.expected_target_sha256)
    )));
}

export function requiresManualResolution(item: PullReviewItem): boolean {
  return item.conversationActions.some((action) => action.action === "blocked")
    || (item.restoreActions.some((action) => action.kind.kind === "manual")
      && !keepsLocalConversation(item));
}

export function defaultSelected(item: PullReviewItem): boolean {
  if (item.category === "project_content") return false;
  const actions = item.restoreActions.filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind));
  const selectedConversationActions = item.conversationActions.filter(conversationActionSelectable);
  return (actions.length > 0 || selectedConversationActions.length > 0)
    && actions.every((action) => !action.requires_explicit_approval)
    && selectedConversationActions.every((action) => !conversationActionRequiresApproval(action));
}

export function itemKindLabel(item: PullReviewItem): string {
  if (
    item.conversationActions.length > 0
    || /^(codex|claude):session:/.test(item.resourceId)
    || item.restoreActions.some((action) => action.kind.kind === "materialize_conversation")
  ) return "Conversation";
  if (item.projectContentEntryType === "directory") return "Project folder";
  if (item.projectContentEntryType === "file") return "Project file";
  if (item.restoreActions.some((action) => action.kind.kind === "write_file" || action.kind.kind === "merge_file")) return "File";
  return "Definition";
}

function conversationActionSelectable(action: ConversationPlanAction): boolean {
  return [
    "fast_forward_local",
    "resolve_divergence",
    "create_local",
    "replace_local",
  ].includes(action.action);
}

function conversationActionRequiresApproval(action: ConversationPlanAction): boolean {
  return action.action === "replace_local";
}

export function buildPullReviewSelection(
  items: PullReviewItem[],
  selected: ReadonlySet<string>,
  completedActionIds: ReadonlySet<string>,
  completedResourceIds: ReadonlySet<string>,
  conversationResolutions: ReadonlyMap<string, ConversationResolutionSelection> = new Map(),
): PullReviewSelection {
  const chosen = items.filter((item) => selected.has(item.resourceId) && !completedResourceIds.has(item.resourceId));
  const chosenConversationActions = chosen.flatMap((item) => (
    item.conversationActions.filter((action) => (
      conversationActionSelectable(action) && !completedActionIds.has(action.action_id)
    ))
  ));
  return {
    resourceIds: chosen
      .filter((item) => pendingIds(item, completedActionIds).length > 0)
      .map((item) => item.resourceId),
    restoreActionIds: chosen.flatMap((item) => restoreActionIds(item).filter((id) => !completedActionIds.has(id))),
    conversationResolutions: chosenConversationActions
      .filter((action) => action.action === "resolve_divergence")
      .map((action) => conversationResolutions.get(action.action_id) ?? {
        action_id: action.action_id,
        resolution: "preserve_local_with_native_fork",
        destructive_approved: false,
      }),
  };
}
