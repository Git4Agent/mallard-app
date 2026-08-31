import type {
  BundleRecipe,
  ProjectResourceCategory,
  ProjectResourceDescriptor,
  ProjectResourceKind,
  ProviderKey,
  ResourceInventory,
} from "../../types";

/** A rename either landed, or failed with something to say, or was superseded
 *  by a host-identity change (`message: null`) and has nothing to report. */
export type RenameResult = { ok: true } | { ok: false; message: string | null };

/** Machine-local alias wins over the shared repo name wherever a project is labelled. */
/**
 * Thread states where the storage holds work this machine does not, so a Push
 * would have to be preceded by a Pull.
 *
 * `unknown` is deliberately absent. The Push review is authoritative, and an
 * unresolvable row must remain removable from the Push selection.
 */
export const STORAGE_BLOCKING_STATES: ReadonlySet<string> = new Set([
  "storage_only",
  "storage_ahead",
  "diverged",
]);

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

export function repositoryFingerprintsRequireAcknowledgement(
  remoteFingerprint: string | null | undefined,
  localFingerprint: string | null | undefined,
): boolean {
  return (remoteFingerprint ?? null) !== (localFingerprint ?? null);
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
   that tells a user what will happen to their file must say so in words. */
const KIND_LABELS: Record<ProjectResourceKind, string> = {
  conversation: "Session",
  provider_state: "Provider state",
  project_content_file: "Project file",
  project_content_directory: "Project folder",
  requirement: "Requirement",
};

const APPLY_POLICY_LABELS: Record<string, string> = {
  safe_file: "Applied when safe",
  merge: "Merged on Pull",
  explicit_review: "Applied after review",
  manual_only: "Manual apply only",
  never: "Never applied",
};

export function kindLabel(kind: ProjectResourceKind): string {
  return KIND_LABELS[kind];
}

export function applyPolicyLabel(policy: string): string {
  return APPLY_POLICY_LABELS[policy] ?? policy;
}

const CATEGORY_BY_KIND: Record<ProjectResourceKind, ProjectResourceCategory> = {
  conversation: "conversations",
  provider_state: "conversations",
  project_content_file: "project_files",
  project_content_directory: "project_files",
  requirement: "tools",
};

export function categoryFor(resource: ProjectResourceDescriptor): ProjectResourceCategory {
  return resource.category ?? CATEGORY_BY_KIND[resource.kind];
}

export function inventoryResources(inventory: ResourceInventory | null): ProjectResourceDescriptor[] {
  return inventory?.resources ?? [];
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
