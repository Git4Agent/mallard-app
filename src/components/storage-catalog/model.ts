import type { IconName } from "../Icons";
import { clockOptions } from "../../clock";
import { errorMessage } from "../../storageError";
import type { ThreadStatusIconName } from "../sync-review/ThreadStatusIcon";
import { isPullReviewStatus } from "../sync-review/pullReviewStatus";
import type {
  CatalogCapabilityDescriptor,
  CatalogCoordinate,
  CatalogProfile,
  CatalogPullAction,
  CatalogScopeDto,
  CatalogScopePullItem,
  CatalogScopeRow,
  CatalogScopeView,
  CatalogStorageRepository,
  CatalogStorageRepositoryView,
} from "./types";

/** What the frontend draws for one capability: the glyph its rows wear, and
 * whether its cards open the artwork column — a preview tile filled inline
 * for local captures and lazily for stored-only rows. */
export interface CatalogGlyph {
  icon: IconName;
  artwork: boolean;
}

/** The frontend's vocabulary for the backend's presentation hints, the way
 * `storageIconName` is for storage identity and `ProviderMark` for provider
 * marks. Each hint is a semantic glyph — `ProviderMark` identifies the
 * provider separately on the same row — so no kind borrows `file`, `tool`,
 * or a provider mark. A hint outside the vocabulary, a capability this build
 * has never heard of, wears the Catalog glyph and opens no artwork. */
const CATALOG_GLYPHS: Record<string, CatalogGlyph> = {
  skill: { icon: "lightbulb", artwork: false },
  plugin: { icon: "catalog-plugin", artwork: false },
  pet: { icon: "catalog-pet-v1", artwork: true },
  /* The second pet format is its own row beside the first, so it wears its
     own mark — a face beside the print — rather than a second paw the eye
     cannot tell from the first. */
  "pet-v2": { icon: "catalog-pet-v2", artwork: true },
};
const UNKNOWN_GLYPH: CatalogGlyph = { icon: "catalog", artwork: false };

export function catalogPresentation(
  capability: Pick<CatalogCapabilityDescriptor, "presentation"> | null,
): CatalogGlyph {
  const hint = capability?.presentation.icon;
  return (hint && CATALOG_GLYPHS[hint]) || UNKNOWN_GLYPH;
}

export function sameCatalogCoordinate(left: CatalogCoordinate, right: CatalogCoordinate): boolean {
  return left.provider === right.provider
    && left.resource_kind === right.resource_kind
    && left.format === right.format
    && left.format_version === right.format_version;
}

/** The capability this build registers for a coordinate, or null: the
 * backend's registry is the authority, and this is only the lookup. */
export function catalogCapabilityFor(
  capabilities: readonly CatalogCapabilityDescriptor[],
  coordinate: CatalogCoordinate,
): CatalogCapabilityDescriptor | null {
  return capabilities.find((candidate) => sameCatalogCoordinate(candidate, coordinate)) ?? null;
}

/** Scope rows joined to the capabilities they register, in the order the
 * backend listed the rows. */
export function catalogScopeViews(
  scopes: readonly CatalogScopeDto[],
  capabilities: readonly CatalogCapabilityDescriptor[],
): CatalogScopeView[] {
  return scopes.map((scope) => ({
    ...scope,
    capability: catalogCapabilityFor(capabilities, scope.scope),
  }));
}

export function catalogStorageRepositoryViews(
  repositories: readonly CatalogStorageRepository[],
  capabilities: readonly CatalogCapabilityDescriptor[],
): CatalogStorageRepositoryView[] {
  return repositories.map((repository) => ({
    ...repository,
    capability: catalogCapabilityFor(capabilities, repository),
  }));
}

/** "Skills · Codex home" — the row label is the capability's label plus the
 * profile's current name, so renaming a profile renames its rows. An
 * unavailable row carries its raw coordinate as the label. */
export function catalogScopeLabel(
  scope: Pick<CatalogScopeDto, "label"> & { profile: Pick<CatalogProfile, "display_name"> },
): string {
  return `${scope.label} · ${scope.profile.display_name}`;
}

export function catalogScopeIcon(scope: Pick<CatalogScopeView, "capability">): IconName {
  return catalogPresentation(scope.capability).icon;
}

export function catalogNoun(pluralLabel: string, singularLabel: string, count: number): string {
  return (count === 1 ? singularLabel : pluralLabel).toLowerCase();
}

/** One radio of the Add chooser: a portable shape — kind, format, and
 * version — with every provider the backend registers it for, in the
 * backend's own order. Descriptors group by shape so the chooser reads
 * "Skills · Codex and Claude" the way the menu rows do, and the provider is
 * the next choice rather than a repeat of the kind. */
export interface CatalogAddChoice {
  key: string;
  label: string;
  providers: CatalogCapabilityDescriptor[];
}

export function catalogAddChoices(
  capabilities: readonly CatalogCapabilityDescriptor[],
): CatalogAddChoice[] {
  const choices: CatalogAddChoice[] = [];
  for (const capability of capabilities) {
    const key = `${capability.resource_kind}/${capability.format}@${capability.format_version}`;
    const existing = choices.find((choice) => choice.key === key);
    if (existing) existing.providers.push(capability);
    else choices.push({ key, label: capability.label, providers: [capability] });
  }
  return choices;
}

/** The doors of an empty Catalog: the Add chooser's choices, in its order,
 * each wearing the glyph of the shape it names. The sidebar's empty pane and
 * the index page draw the same list. */
export type CatalogDoor = CatalogAddChoice & { icon: IconName };

export function catalogDoors(capabilities: readonly CatalogCapabilityDescriptor[]): CatalogDoor[] {
  return catalogAddChoices(capabilities).map((choice) => ({
    ...choice,
    icon: catalogPresentation(choice.providers[0]).icon,
  }));
}

/** "Codex only", "Codex and Claude" — which providers offer one shape. */
export function catalogProviderList(labels: readonly string[]): string {
  if (labels.length === 1) return `${labels[0]} only`;
  return `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
}

const CATALOG_SCOPE_LINKS_STORAGES = "still links storages";

export function catalogScopeRemovalRequiresCleanup(reason: unknown): boolean {
  return errorMessage(reason).includes(CATALOG_SCOPE_LINKS_STORAGES);
}

/**
 * Removing a Catalog row can be refused for two reasons, and the backend
 * states both for a log rather than for a person. Translate the linked-storage
 * refusal into the exact UI action that clears the row: unlink every storage
 * from the storage crumb's menu.
 *
 * The match is on the backend's wording, which no compiler defends, so
 * `catalog-scope-removal.integration.test.tsx` pins both strings verbatim
 * against their source in `resource_commands.rs`.
 */
export function catalogScopeRemovalMessage(label: string, reason: unknown): string {
  const message = errorMessage(reason);
  if (catalogScopeRemovalRequiresCleanup(reason)) {
    return `Unlink every storage from “${label}”, then remove it again.`;
  }
  if (message.includes("Catalog row changed")) {
    return `“${label}” changed somewhere else. Refresh the Catalog, then try removing it again.`;
  }
  return message;
}

/** The badge a row wears. Every one of these is a state the backend derives,
 * so no badge on the page is decorative — and it is the same vocabulary a
 * project thread wears, because the question is the same one. */
export function rowStatus(row: CatalogScopeRow): ThreadStatusIconName {
  if (!row.state) return "untracked";
  switch (row.state) {
    case "in_sync":
      return "synced";
    case "new":
    case "local_changed":
    case "remote_missing":
      return "local-outstanding";
    case "storage_changed":
    case "local_missing":
      return "cloud-outstanding";
    case "unreviewed_remote":
    case "diverged":
      return "divergent";
    case "blocked":
      return "unavailable";
  }
}

/** The badge's sentence: what this row's state means for this storage, said
 * in the one place the state is shown. */
export function rowStatusLabel(row: CatalogScopeRow, storageName: string | null): string {
  const storage = storageName ?? "storage";
  if (!row.state) return "Only on this Mac. Choose a storage to compare and sync it.";
  switch (row.state) {
    case "in_sync":
      return `Up to date here and in ${storage}.`;
    case "new":
      return `Only on this computer. Push to save it in ${storage}.`;
    case "local_changed":
      return `Newer on this computer. Push to update ${storage}.`;
    case "remote_missing":
      return `Gone from ${storage}. Push to put it back.`;
    case "storage_changed":
      return `Newer in ${storage}. Pull to update this computer.`;
    case "local_missing":
      if (!row.remote_present) return `Not on this computer, and not in ${storage}.`;
      return `Only in ${storage}. Pull to download it here.`;
    case "unreviewed_remote":
    case "diverged":
      return `Changed here and in ${storage}. Review before syncing.`;
    case "blocked":
      return row.blocker ?? "Can't be synced.";
  }
}

/** Push adds to the storage; Pull changes this Mac. A row is offered to
 * exactly one of them, and only when there is something for it to move. */
export function rowPushable(row: CatalogScopeRow): boolean {
  return row.local_present && (row.state === "new" || row.state === "local_changed");
}

export function rowPullable(row: CatalogScopeRow): boolean {
  return row.remote_present && isPullReviewStatus(rowStatus(row));
}

/** Keep one Catalog Pull plan's visible and selectable rows aligned. */
export function filterCatalogPullReviewItems(
  items: readonly CatalogScopePullItem[],
  rows: readonly CatalogScopeRow[],
): CatalogScopePullItem[] {
  const rowsByResource = new Map(rows
    .filter((row): row is CatalogScopeRow & { resource_id: string } => Boolean(row.resource_id))
    .map((row) => [row.resource_id, row]));
  return items.filter((item) => {
    const row = rowsByResource.get(item.item.resource_id);
    return row != null && rowPullable(row);
  });
}

/** A Library Pull's recommended selection: every item's default action, and
 * nothing for an item that has none. `matches` says the selection and the
 * chosen actions already read that way, so recommending again changes nothing. */
export function catalogPullRecommendation(
  items: readonly CatalogScopePullItem[],
  selected: ReadonlySet<string>,
  choice: Readonly<Record<string, string>>,
): { items: CatalogScopePullItem[]; matches: boolean } {
  return {
    items: items.filter((item) => item.item.default_action_id),
    matches: items.every((item) => {
      const id = item.item.resource_id;
      const action = item.item.default_action_id;
      return action ? selected.has(id) && choice[id] === action : !selected.has(id);
    }),
  };
}

/** The exact time a row last moved, and which side moved it. Local time wins
 * when the resource is on this Mac, because that is the copy you can edit. */
export function rowStamp(row: CatalogScopeRow): { seconds: number; detail: string } | null {
  const local = row.local_present ? row.local_modified_at ?? null : null;
  const seconds = local ?? row.remote_updated_at ?? null;
  if (seconds == null) return null;
  const when = new Date(seconds * 1000);
  const exact = `${when.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" })} · ${when.toLocaleTimeString(undefined, clockOptions())}`;
  return {
    seconds,
    detail: `${local == null ? "Stored" : "Edited"} ${exact}`,
  };
}

/** The sentence a Pull action reduces to. The four kinds are the protocol's
 * own outcomes, the same for every format. */
export function actionLabel(action: CatalogPullAction): string {
  switch (action.kind.kind) {
    case "preserve_as_copy_and_overwrite":
      return `Keep local as ${action.kind.copy.copy_name} and overwrite`;
    case "overwrite_with_storage":
      return "Overwrite local content";
    case "install":
      return "Install stored content";
    case "update":
      return "Update from storage";
  }
}
