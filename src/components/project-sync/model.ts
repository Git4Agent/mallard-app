import type {
  BundleRecipe,
  InventoryResource,
  MaintenanceActionDescriptor,
  ProjectContentEntry,
  ProjectResourceCategory,
  ProviderDescriptor,
  ProviderKey,
  ResourceInventory,
  StorageProviderDescriptor,
  StorageProviderKey,
} from "../../types";
import { storageIconName, type IconName } from "../Icons";

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

/** One project-file policy for every grouped card list. Search still examines
 * the complete inventory; this bounds only the rendered result. */
export const PROJECT_FILE_CARD_RENDER_LIMIT = 500;

/** The nouns the project lists count in — the search row and tile heads. */
export const sessionNoun = (count: number) => (count === 1 ? "session" : "sessions");
export const fileNoun = (count: number) => (count === 1 ? "file" : "files");

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

/** The file rows' Kind readout, Finder's vocabulary for the common suspects.
 * An unknown extension stays honest and terse: the extension, uppercased. */
const FILE_KIND_LABELS: Record<string, string> = {
  md: "Markdown",
  txt: "Text",
  doc: "Word",
  docx: "Word",
  pdf: "PDF",
  sh: "Shell",
  py: "Python",
  js: "JavaScript",
  ts: "TypeScript",
  json: "JSON",
  yaml: "YAML",
  yml: "YAML",
  toml: "TOML",
  html: "HTML",
  css: "CSS",
  csv: "CSV",
  png: "PNG image",
  jpg: "JPEG image",
  jpeg: "JPEG image",
  gif: "GIF image",
  svg: "SVG image",
  webp: "WebP image",
  psd: "Photoshop",
  wav: "WAV audio",
  mp3: "MP3 audio",
  mp4: "MP4 video",
  mov: "QuickTime video",
  zip: "ZIP archive",
  gz: "Gzip archive",
};

export function fileKindLabel(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "File";
  const extension = name.slice(dot + 1).toLocaleLowerCase();
  return FILE_KIND_LABELS[extension] ?? extension.toUpperCase();
}

/* A scanned file's warning in one word pair, for the row chip on both the
   Files activity and the Push file review: the chip names the finding. */
const PROJECT_FILE_WARNING_LABELS: Record<string, string> = {
  possible_secret: "Possible secret",
  executable: "Executable",
};

export function projectFileWarningLabel(code: string): string {
  return PROJECT_FILE_WARNING_LABELS[code] ?? code.replace(/_/g, " ");
}

export function inventoryResources(inventory: ResourceInventory | null): InventoryResource[] {
  return inventory?.resources ?? [];
}

/**
 * The review row the Push chooser lists for one scanned project entry. The
 * entry carries the bare portable descriptor; the scan files it under
 * Project files, and the Files page renders it from the entry itself, so its
 * presentation is the inventory's own row when the inventory has one and the
 * Files review's vocabulary otherwise. It is selectable unless the scan
 * blocked it: the backend built the descriptor from its registered contracts.
 */
export function projectContentReviewRow(
  entry: ProjectContentEntry,
  known?: InventoryResource,
): InventoryResource {
  return {
    ...entry.descriptor,
    category: "project_files",
    kind_label: known?.kind_label ?? (entry.entry_type === "directory" ? "Project folder" : "Project file"),
    apply_policy_label: known?.apply_policy_label ?? "Explicit review",
    selectable: known?.selectable ?? true,
    logical_paths: [entry.logical_path],
    default_selected: entry.selected_in_recipe,
    blocked_reason: entry.blocked_reason,
    description: entry.relative_path,
  };
}

export function recipeSelection(recipe: BundleRecipe): Set<string> {
  return new Set(Object.keys(recipe.entries));
}

export function recipeWithSelection(
  recipe: BundleRecipe,
  resources: InventoryResource[],
  selected: Set<string>,
): BundleRecipe {
  return {
    ...recipe,
    entries: Object.fromEntries(resources
      .filter((resource) => (
        selected.has(resource.resource_id)
        && !resource.blocked_reason
        && resource.selectable
      ))
      .map((resource) => [resource.resource_id, recipe.entries[resource.resource_id] ?? {
        resource_id: resource.resource_id,
        // Copied verbatim: the backend aligns recipe policies to the descriptor.
        apply_policy: resource.apply_policy,
        required: false,
      }])),
  };
}

/** The doors of an empty Storages list: one per provider the build ships,
 *  in the backend's order. A draft is bound to its provider from the moment
 *  it exists, so the door is the choice the picker would otherwise ask for.
 *  Descriptors are per config version; the door is per provider. */
export interface StorageDoor {
  key: StorageProviderKey;
  icon: IconName;
  label: string;
}

/** Storages in the order both setup pages list them — Mallard Cloud, then
 *  the other clouds, then local folders: the registry's display rank, and
 *  the name among equals. A provider this build does not register sorts
 *  last. The default a setup page opens on is the first of these. */
export function orderStorages<T extends { provider: string; name: string }>(
  storages: readonly T[],
  providers: readonly StorageProviderDescriptor[],
): T[] {
  const rank = new Map<string, number>(providers.map((provider) => [provider.key, provider.display_rank]));
  const rankOf = (storage: T) => rank.get(storage.provider) ?? Number.MAX_SAFE_INTEGER;
  return [...storages].sort((left, right) => rankOf(left) - rankOf(right) || left.name.localeCompare(right.name));
}

export function storageDoors(providers: readonly StorageProviderDescriptor[]): StorageDoor[] {
  const doors: StorageDoor[] = [];
  for (const provider of providers) {
    if (doors.some((door) => door.key === provider.key)) continue;
    doors.push({
      key: provider.key,
      icon: storageIconName(provider.key, provider.icon),
      label: provider.display_name,
    });
  }
  return doors;
}

export function compactProjectPath(path?: string | null): string {
  if (!path) return "Not mapped on this machine";
  // macOS and Linux homes alike; the Folder field compacts by the real home.
  return path.replace(/^\/(?:Users|home)\/[^/]+/, "~");
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

/** Find one declared maintenance action by its stable ID. */
export function maintenanceActionFor(
  providers: ProviderDescriptor[],
  provider: ProviderKey,
  actionId: string | null | undefined,
): MaintenanceActionDescriptor | undefined {
  if (!actionId) return undefined;
  return providers
    .find((candidate) => candidate.key === provider)
    ?.maintenance_actions.find((action) => action.action_id === actionId);
}
