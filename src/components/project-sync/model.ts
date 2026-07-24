import type {
  BundleRecipe,
  ProjectProvider,
  ProjectResourceCategory,
  ProjectResourceDescriptor,
  ResourceInventory,
} from "../../types";
import { userFacingRepoTerms } from "../../terminology";

/** Machine-local alias wins over the shared repo name wherever a project is labelled. */
export function projectLabel(project: { display_name: string; local_alias?: string | null }): string {
  return project.local_alias?.trim() || project.display_name;
}

export const RESOURCE_GROUPS: Array<{
  id: ProjectResourceCategory;
  label: string;
  description: string;
}> = [
  { id: "conversations", label: "Conversations", description: "Tasks, sessions, memory, and their filtered indexes" },
  { id: "skills", label: "Skills", description: "Selected standalone skills from the provider home" },
  { id: "plugins", label: "Plugins", description: "Portable install intent; payloads and caches never sync" },
  { id: "project_files", label: "Project files", description: "Exact files and folders selected from a non-Git project" },
  { id: "tools", label: "Requirements", description: "Environment requirements this backup expects on each machine" },
];

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
  skill: "skills",
  standalone_skill: "skills",
  plugin_skill: "skills",
  plugin: "plugins",
  tool: "tools",
  requirement: "tools",
};

export function categoryFor(resource: ProjectResourceDescriptor): ProjectResourceCategory {
  const explicit = resource.category;
  if (explicit === "conversations" || explicit === "skills" || explicit === "plugins" || explicit === "project_files" || explicit === "tools") {
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

export function providerLabel(provider?: ProjectProvider | null): string {
  if (provider === "codex") return "Codex";
  if (provider === "claude") return "Claude";
  return "Shared";
}

export const PROJECT_PROVIDERS = ["codex", "claude"] as const satisfies readonly ProjectProvider[];

/** A project is configured against one machine-local agent profile at a time. */
export function configuredProjectProvider<T>(
  profiles: Partial<Record<ProjectProvider, T>> | null | undefined,
): ProjectProvider | null {
  return PROJECT_PROVIDERS.find((provider) => profiles?.[provider] != null) ?? null;
}

/** Keep only the selected agent's value when the user changes agent type. */
export function singleProviderSelection<T>(
  profiles: Partial<Record<ProjectProvider, T>>,
  provider: ProjectProvider,
): Partial<Record<ProjectProvider, T>> {
  const selection = profiles[provider];
  return selection == null ? {} : { [provider]: selection };
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
