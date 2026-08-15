import type {
  BundleRecipe,
  ProjectResourceCategory,
  ProjectResourceDescriptor,
  ProviderKey,
  ResourceInventory,
  StorageConfig,
} from "../../types";
import { userFacingRepoTerms } from "../../terminology";

/** Machine-local alias wins over the shared repo name wherever a project is labelled. */
export function projectLabel(project: { display_name: string; local_alias?: string | null }): string {
  return project.local_alias?.trim() || project.display_name;
}

export function repositoryFingerprintMatches(
  remoteFingerprint: string | null | undefined,
  localFingerprint: string | null | undefined,
): boolean {
  return !!remoteFingerprint
    && !!localFingerprint
    && remoteFingerprint === localFingerprint;
}

export const RESOURCE_GROUPS: Array<{
  id: ProjectResourceCategory;
  label: string;
  description: string;
}> = [
  { id: "conversations", label: "Conversations", description: "Project-scoped agent sessions" },
  { id: "project_files", label: "Project files", description: "Exact files and folders selected from a non-Git project" },
  { id: "tools", label: "Requirements", description: "Environment requirements this backup expects on each machine" },
];

/* Wire enums never reach the screen raw: a resource's kind and apply policy
   are backend vocabulary ("manual_only", "explicit_review"), and the field
   that tells a user what will happen to their file must say so in words.
   Runtime kinds are wider than the ProjectResourceKind union (provider-
   prefixed variants arrive from older recipes), so lookups fall back to the
   raw value rather than lying. */
export const KIND_LABELS: Record<string, string> = {
  conversation: "Session",
  codex_conversation: "Session",
  claude_conversation: "Session",
  codex_task: "Session",
  claude_session: "Session",
  provider_state: "Provider state",
  project_content_file: "Project file",
  project_content_directory: "Project folder",
  requirement: "Requirement",
};

export const APPLY_POLICY_LABELS: Record<string, string> = {
  safe_file: "Applied when safe",
  merge: "Merged on Pull",
  explicit_review: "Applied after review",
  manual_only: "Manual apply only",
  never: "Never applied",
};

export function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

export function applyPolicyLabel(policy: string): string {
  return APPLY_POLICY_LABELS[policy] ?? policy;
}

const CATEGORY_BY_KIND: Record<string, ProjectResourceCategory> = {
  codex_conversation: "conversations",
  claude_conversation: "conversations",
  codex_task: "conversations",
  claude_session: "conversations",
  conversation: "conversations",
  task: "conversations",
  session: "conversations",
  memory: "conversations",
  project_memory: "conversations",
  project_content_file: "project_files",
  project_content_directory: "project_files",
  requirement: "tools",
};

export function categoryFor(resource: ProjectResourceDescriptor): ProjectResourceCategory {
  const explicit = resource.category;
  if (explicit === "conversations" || explicit === "project_files" || explicit === "tools") {
    return explicit;
  }
  return CATEGORY_BY_KIND[resource.kind.toLowerCase()] ?? "tools";
}

export function inventoryResources(inventory: ResourceInventory | null): ProjectResourceDescriptor[] {
  return inventory?.resources ?? inventory?.candidates ?? [];
}

export function recipeSelection(recipe: BundleRecipe): Set<string> {
  return new Set(Object.keys(recipe.entries));
}

export function recipeWithSelection(
  recipe: BundleRecipe,
  resources: ProjectResourceDescriptor[],
  selected: Set<string>,
): BundleRecipe {
  return {
    ...recipe,
    entries: Object.fromEntries(resources
      .filter((resource) => (
        selected.has(resource.resource_id)
        && !resource.blocked_reason
        && resource.apply_policy !== "never"
      ))
      .map((resource) => [resource.resource_id, recipe.entries[resource.resource_id] ?? {
        resource_id: resource.resource_id,
        apply_policy: resource.apply_policy,
        required: false,
      }])),
  };
}

export function compactProjectPath(path?: string | null): string {
  if (!path) return "Not mapped on this machine";
  return path.replace(/^\/Users\/[^/]+/, "~");
}

/**
 * Generic display label for a provider key. Registry display names arrive on
 * backend descriptors and session rows; this fallback only capitalizes the
 * validated key, so no component carries a fixed provider list.
 */
export function providerLabel(provider?: ProviderKey | null): string {
  if (!provider) return "Shared";
  return provider.charAt(0).toUpperCase() + provider.slice(1);
}

export function formatRelativeTime(epoch?: number): string {
  if (!epoch) return "Unknown";
  const milliseconds = epoch > 10_000_000_000 ? epoch : epoch * 1000;
  const delta = Date.now() - milliseconds;
  if (delta < 60_000) return "just now";
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
  return new Date(milliseconds).toLocaleDateString();
}

export function errorMessage(error: unknown): string {
  return userFacingRepoTerms(error instanceof Error ? error.message : String(error));
}

export function storageAccessErrorMessage(
  storage: Pick<StorageConfig, "kind"> | null | undefined,
  error: unknown,
): string {
  const message = errorMessage(error);
  const looksLikeR2ConnectionFailure = /^S3 (?:get|body|conditional put)/i.test(message)
    && /dispatch failure|service error|timed out|access denied|credentials?|invalid access key|signature|no such bucket/i.test(message);
  if (storage?.kind !== "s3" || !looksLikeR2ConnectionFailure) return message;
  return "Couldn’t connect to Cloudflare R2. Check the S3 API URL, bucket name, Account ID, and credentials, then try again.";
}
