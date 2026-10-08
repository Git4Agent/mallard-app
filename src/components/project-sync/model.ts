import type {
  BundleRecipe,
  InventoryResource,
  LocalSessionDeleteResult,
  MaintenanceActionDescriptor,
  ProjectContentEntry,
  ProjectResourceCategory,
  ProviderDescriptor,
  ProviderKey,
  ResourceInventory,
  StorageProviderDescriptor,
  StorageProviderKey,
} from "../../types";
import { t, UI_LANGUAGES } from "../../i18n";
import { storageIconName, type IconName } from "../Icons";
import { providerName } from "./storageText";

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

/** The nouns the project lists count in — the search row and tile heads,
 *  which print them after the number ("3 sessions", "3 个会话"). */
export const sessionNoun = (count: number) => t("reviews:nouns.session", { count });
export const fileNoun = (count: number) => t("reviews:nouns.file", { count });

/** The backend's sentence when Codex's app holds a session it was asked to
 *  delete; it stays English, like every backend error. */
// i18n-ignore: backend text, compared against
export const CODEX_DELETE_ERROR = "Codex provider request failed (code -32600).";

export const codexDeleteCloseNotice = () => t("projects:deleteCodexClose");

/** Whether an error carries the Close Codex notice, in whichever language
 *  it was written: the notice's button survives a language switch. */
export function offersCloseCodex(message: string): boolean {
  return UI_LANGUAGES.some((lng) => message.includes(t("projects:deleteCodexClose", { lng })));
}

export function localSessionDeleteError(provider: ProviderKey, result: LocalSessionDeleteResult): string | null {
  if (provider === "codex" && result.provider_error === CODEX_DELETE_ERROR) return codexDeleteCloseNotice();
  return [
    result.provider_error && t("reviews:model.sessionFilesError", { error: result.provider_error }),
    result.metadata_error && t("reviews:model.bindingError", { error: result.metadata_error }),
  ].filter(Boolean).join(" ") || null;
}

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

/** The inventory's groups, in order. Their words are read when drawn. */
export const RESOURCE_GROUPS: ReadonlyArray<{
  id: ProjectResourceCategory;
  label: () => string;
  description: () => string;
}> = [
  { id: "conversations", label: () => t("reviews:groups.conversations"), description: () => t("reviews:groups.conversationsDescription") },
  { id: "project_files", label: () => t("reviews:groups.projectFiles"), description: () => t("reviews:groups.projectFilesDescription") },
  { id: "tools", label: () => t("reviews:groups.tools"), description: () => t("reviews:groups.toolsDescription") },
];

/** The file rows' Kind readout, Finder's vocabulary for the common suspects.
 * An unknown extension stays honest and terse: the extension, uppercased.
 * A format's own name reads the same in every language; a kind described in
 * words is translated when it is drawn. */
const FILE_FORMAT_NAMES: Record<string, string> = {
  md: "Markdown",
  doc: "Word",
  docx: "Word",
  pdf: "PDF",
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
  psd: "Photoshop",
};

const FILE_KIND_LABELS: Record<string, () => string> = {
  txt: () => t("reviews:fileKind.text"),
  sh: () => t("reviews:fileKind.shell"),
  png: () => t("reviews:fileKind.pngImage"),
  jpg: () => t("reviews:fileKind.jpegImage"),
  jpeg: () => t("reviews:fileKind.jpegImage"),
  gif: () => t("reviews:fileKind.gifImage"),
  svg: () => t("reviews:fileKind.svgImage"),
  webp: () => t("reviews:fileKind.webpImage"),
  wav: () => t("reviews:fileKind.wavAudio"),
  mp3: () => t("reviews:fileKind.mp3Audio"),
  mp4: () => t("reviews:fileKind.mp4Video"),
  mov: () => t("reviews:fileKind.quickTimeVideo"),
  zip: () => t("reviews:fileKind.zipArchive"),
  gz: () => t("reviews:fileKind.gzipArchive"),
};

export function fileKindLabel(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return t("reviews:fileKind.file");
  const extension = name.slice(dot + 1).toLocaleLowerCase();
  return FILE_FORMAT_NAMES[extension] ?? FILE_KIND_LABELS[extension]?.() ?? extension.toUpperCase();
}

/* A scanned file's warning in one word pair, for the row chip on both the
   Files activity and the Push file review: the chip names the finding. */
const PROJECT_FILE_WARNING_LABELS: Record<string, () => string> = {
  possible_secret: () => t("reviews:model.possibleSecret"),
  executable: () => t("reviews:model.executable"),
};

/** An unknown code prints as the backend wrote it, underscores spaced. */
export function projectFileWarningLabel(code: string): string {
  return PROJECT_FILE_WARNING_LABELS[code]?.() ?? code.replace(/_/g, " ");
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
    kind_label: known?.kind_label ?? (entry.entry_type === "directory" ? t("reviews:kind.projectFolder") : t("reviews:kind.projectFile")),
    apply_policy_label: known?.apply_policy_label ?? t("reviews:model.explicitReview"),
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
    // Feishu is temporarily hidden from storage setup.
    if (provider.key === "feishu") continue;
    if (doors.some((door) => door.key === provider.key)) continue;
    doors.push({
      key: provider.key,
      icon: storageIconName(provider.key, provider.icon),
      label: providerName(provider),
    });
  }
  return doors;
}

export function compactProjectPath(path?: string | null): string {
  if (!path) return t("reviews:model.notMapped");
  // macOS and Linux homes alike; the Folder field compacts by the real home.
  return path.replace(/^\/(?:Users|home)\/[^/]+/, "~");
}

/**
 * Generic display label for a provider key. Registry display names arrive on
 * backend descriptors and session rows; this fallback only capitalizes the
 * validated key, so no component carries a fixed provider list.
 */
export function providerLabel(provider?: ProviderKey | null): string {
  if (!provider) return t("reviews:model.shared");
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
