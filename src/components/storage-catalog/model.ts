import type { IconName } from "../Icons";
import { clockOptions, formatRelativeTime } from "../../clock";
import { t, uiLocale } from "../../i18n";
import { backendText } from "../../i18n/backend";
import { errorMessage } from "../../storageError";
import type { ThreadStatusIconName } from "../sync-review/ThreadStatusIcon";
import { isPullReviewStatus } from "../sync-review/pullReviewStatus";
import { nameMatchesSearch } from "../sync-review/nameSearch";
import type { StatusScope } from "../sync-review/statusScopes";
import type {
  CatalogCapabilityDescriptor,
  CatalogCoordinate,
  CatalogProfile,
  CatalogProfileHolding,
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

/** A capability's labels in the interface language: the backend's English,
 * or Chinese looked up by the presentation hint beside them. */
export function capabilityLabel(capability: Pick<CatalogCapabilityDescriptor, "presentation" | "label">): string {
  return backendText(`capability.${capability.presentation.icon}.label`, capability.label);
}

export function capabilitySingular(capability: Pick<CatalogCapabilityDescriptor, "presentation" | "singular_label">): string {
  return backendText(`capability.${capability.presentation.icon}.singular`, capability.singular_label);
}

export function capabilityFormatLabel(capability: Pick<CatalogCapabilityDescriptor, "presentation">): string {
  return backendText(`capability.${capability.presentation.icon}.formatLabel`, capability.presentation.format_label);
}

/** A row's own label: its registered capability's, in the interface
 * language, or the raw coordinate an unavailable row carries. */
export function catalogScopeKindLabel(
  scope: Pick<CatalogScopeDto, "label"> & { capability?: CatalogScopeView["capability"] },
): string {
  return scope.capability
    ? backendText(`capability.${scope.capability.presentation.icon}.label`, scope.label)
    : scope.label;
}

/** "Skills · Codex home" — the row label is the capability's label plus the
 * profile's current name, so renaming a profile renames its rows. An
 * unavailable row carries its raw coordinate as the label. */
export function catalogScopeLabel(
  scope: Pick<CatalogScopeDto, "label"> & {
    capability?: CatalogScopeView["capability"];
    profile: Pick<CatalogProfile, "display_name">;
  },
): string {
  return `${catalogScopeKindLabel(scope)} · ${scope.profile.display_name}`;
}

export function catalogScopeIcon(scope: Pick<CatalogScopeView, "capability">): IconName {
  return catalogPresentation(scope.capability).icon;
}

/** The words a page uses for one kind of resource. */
export interface CatalogNouns {
  /** A sentence about this kind, handed its noun for `count` things — the
   *  plural when no count is given — to pass as `{{noun}}`. Chinese writes
   *  `{{noun}}` straight against the Han beside it; the space a Latin noun
   *  ("Skill") needs there is put in here, so 安装 Skill and 安装插件 both read
   *  right. English sentences carry their own spaces. */
  say: (sentence: (noun: string) => string, count?: number) => string;
  /** A sentence that opens on the singular noun: English capitalizes it
   *  ("Skill info"). */
  lead: (sentence: (noun: string) => string) => string;
  /** The noun a shared component prints after a bare count: "3 skills",
   *  "3 个 Skill". */
  counted: (count: number) => string;
}

type NounKind = "skill" | "plugin" | "petV1" | "petV2";

/** The kinds this build has words for, by the presentation hint the glyphs
 *  above read. */
const NOUN_KINDS: Record<string, NounKind> = { skill: "skill", plugin: "plugin", pet: "petV1", "pet-v2": "petV2" };

const NOUNS: Record<NounKind, (count: number) => string> = {
  skill: (count) => t("library:noun.skill", { count }),
  plugin: (count) => t("library:noun.plugin", { count }),
  petV1: (count) => t("library:noun.petV1", { count }),
  petV2: (count) => t("library:noun.petV2", { count }),
};

const COUNTED: Record<NounKind, (count: number) => string> = {
  skill: (count) => t("library:counted.skill", { count }),
  plugin: (count) => t("library:counted.plugin", { count }),
  petV1: (count) => t("library:counted.petV1", { count }),
  petV2: (count) => t("library:counted.petV2", { count }),
};

/** Any count English words in the plural. */
const MANY = 2;

/** The words for a capability's kind: from the catalog for a kind this build
 *  knows, else from the backend's own labels — lowercased, as a sentence
 *  holds a noun. */
export function catalogNouns(
  capability: Pick<CatalogCapabilityDescriptor, "presentation"> | null,
  labels: { plural: string; singular: string },
): CatalogNouns {
  const kind = capability ? NOUN_KINDS[capability.presentation.icon] : undefined;
  const fallback = { singular: labels.singular.toLowerCase(), plural: labels.plural.toLowerCase() };
  const noun = (count: number) => (kind ? NOUNS[kind](count) : t("library:noun.other", { count, ...fallback }));
  /* The noun is a value of its sentence, so `src/i18n` settles its spacing in
     Chinese: 安装 Skill, 安装插件. */
  const say = (sentence: (noun: string) => string, count = MANY) => sentence(noun(count));
  return {
    say,
    lead: (sentence) => {
      const one = noun(1);
      return sentence(`${one.charAt(0).toUpperCase()}${one.slice(1)}`);
    },
    counted: (count) => (kind ? COUNTED[kind](count) : t("library:counted.other", { count, ...fallback })),
  };
}

/** A Catalog row's words: its capability's, or its own label's when this
 *  build registers none. */
export function catalogScopeNouns(scope: Pick<CatalogScopeView, "label" | "capability">): CatalogNouns {
  return catalogNouns(scope.capability, {
    plural: scope.label,
    singular: scope.capability?.singular_label ?? scope.label,
  });
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

/** The portable shape a capability names — kind, format, version — as the
 * key a Kind door hands to Add. */
export function catalogChoiceKey(
  capability: Pick<CatalogCoordinate, "resource_kind" | "format" | "format_version">,
): string {
  return `${capability.resource_kind}/${capability.format}@${capability.format_version}`;
}

export function catalogAddChoices(
  capabilities: readonly CatalogCapabilityDescriptor[],
): CatalogAddChoice[] {
  const choices: CatalogAddChoice[] = [];
  for (const capability of capabilities) {
    const key = catalogChoiceKey(capability);
    const existing = choices.find((choice) => choice.key === key);
    if (existing) existing.providers.push(capability);
    else choices.push({ key, label: capabilityLabel(capability), providers: [capability] });
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
  if (labels.length === 1) return t("library:providers.only", { provider: labels[0] });
  return t("library:providers.list", {
    list: labels.slice(0, -1).join(t("library:providers.separator")),
    last: labels[labels.length - 1],
  });
}

// i18n-ignore: backend text
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
  if (catalogScopeRemovalRequiresCleanup(reason)) return t("library:rowRemoval.unlinkFirst", { label });
  // i18n-ignore: backend text
  if (message.includes("Catalog row changed")) return t("library:rowRemoval.changed", { label });
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

/** The part of the list a row falls in: the badge it wears, in the scope
 * bar's words. The path bar's Push, Pull and attention counts read the same
 * scopes, so the strip and the bar always agree. */
export function catalogStatusScope(row: CatalogScopeRow): StatusScope {
  switch (rowStatus(row)) {
    case "local-outstanding":
      return "push";
    case "cloud-outstanding":
      return "pull";
    case "synced":
      return "synced";
    case "untracked":
      return "unknown";
    default:
      return "attention";
  }
}

/** How many rows each scope holds; a scope no row falls in is absent. */
export function catalogStatusScopeCounts(rows: readonly CatalogScopeRow[]): Map<StatusScope, number> {
  const counts = new Map<StatusScope, number>();
  for (const row of rows) {
    const scope = catalogStatusScope(row);
    counts.set(scope, (counts.get(scope) ?? 0) + 1);
  }
  return counts;
}

/** The rows the list draws: those in the scope (null is every row) whose
 * name matches the search. */
export function catalogMatchingRows(
  rows: readonly CatalogScopeRow[],
  query: string,
  scope: StatusScope | null,
): CatalogScopeRow[] {
  return rows.filter((row) => (
    (scope === null || catalogStatusScope(row) === scope)
    && nameMatchesSearch(row.display_name, query)
  ));
}

/** The badge's sentence: what this row's state means for this storage, said
 * in the one place the state is shown. */
export function rowStatusLabel(row: CatalogScopeRow, storageName: string | null): string {
  const storage = storageName ?? t("library:status.anyStorage");
  /* No state with a storage chosen: that storage refused the account, and
     the page's note says why. */
  if (!row.state) {
    return storageName
      ? t("library:status.notCompared", { storage: storageName })
      : t("library:status.onlyHere");
  }
  switch (row.state) {
    case "in_sync":
      return t("library:status.inSync", { storage });
    case "new":
      return t("library:status.new", { storage });
    case "local_changed":
      return t("library:status.localChanged", { storage });
    case "remote_missing":
      return t("library:status.remoteMissing", { storage });
    case "storage_changed":
      return t("library:status.storageChanged", { storage });
    case "local_missing":
      if (!row.remote_present) return t("library:status.nowhere", { storage });
      return t("library:status.localMissing", { storage });
    case "unreviewed_remote":
    case "diverged":
      return t("library:status.diverged", { storage });
    case "blocked":
      return row.blocker ?? t("library:status.blocked");
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
export function filterCatalogPullReviewItems(items: readonly CatalogScopePullItem[]): CatalogScopePullItem[] {
  // The reviewed plan is authoritative, including items discovered while
  // connecting a repository that were absent from the previous page.
  return items.filter(({ item }) => ["storage_only", "storage_changed", "changed_both", "unavailable_locally"].includes(item.relation));
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
  const exact = `${when.toLocaleDateString(uiLocale(), { year: "numeric", month: "short", day: "numeric" })} · ${when.toLocaleTimeString(uiLocale(), clockOptions())}`;
  return {
    seconds,
    detail: local == null ? t("library:stamp.stored", { when: exact }) : t("library:stamp.edited", { when: exact }),
  };
}

/** The sentence a Pull action reduces to. The four kinds are the protocol's
 * own outcomes, the same for every format. */
export function actionLabel(action: CatalogPullAction): string {
  switch (action.kind.kind) {
    case "preserve_as_copy_and_overwrite":
      return t("library:action.keepLocalAs", { name: action.kind.copy.copy_name });
    case "overwrite_with_storage":
      return t("library:action.overwriteLocal");
    case "install":
      return t("library:action.installStored");
    case "update":
      return t("library:action.updateFromStorage");
  }
}

/* ——— Add Catalog resource's two steps ——————————————————————————————— */

/** New project's rule for a provider's profile before anyone clicks: the
 * one profile this computer has for it, or none — never a guess among
 * several. */
export function unambiguousProfile(
  profiles: readonly CatalogProfile[],
  provider: string,
): CatalogProfile | null {
  const candidates = profiles.filter((profile) => profile.provider === provider);
  return candidates.length === 1 ? candidates[0] ?? null : null;
}

/** One card of the setup page's Resource band: a capability the home's
 * provider registers, what the home holds of it once the inspection has
 * answered, and the row that already exists. */
export interface CatalogKindCard {
  capability: CatalogCapabilityDescriptor;
  /** The Kind door's key for this shape. */
  key: string;
  holding: CatalogProfileHolding | null;
  existingScopeId: string | null;
}

export function catalogKindCards(
  capabilities: readonly CatalogCapabilityDescriptor[],
  profile: Pick<CatalogProfile, "profile_id" | "provider">,
  scopes: readonly Pick<CatalogScopeDto, "scope">[],
  holdings: readonly CatalogProfileHolding[] | null,
): CatalogKindCard[] {
  return capabilities
    .filter((capability) => capability.provider === profile.provider)
    .map((capability) => {
      const holding = holdings?.find((candidate) => sameCatalogCoordinate(candidate.capability, capability)) ?? null;
      const scope = scopes.find((candidate) => (
        candidate.scope.profile_id === profile.profile_id && sameCatalogCoordinate(candidate.scope, capability)
      )) ?? null;
      return {
        capability,
        key: catalogChoiceKey(capability),
        holding,
        existingScopeId: holding?.scope_id ?? scope?.scope.scope_id ?? null,
      };
    });
}

/** The card chosen before anyone clicks: the Kind door's, else the first
 * with something in it and no row yet. Nothing found: nothing guessed. */
export function catalogDefaultKind(cards: readonly CatalogKindCard[], doorKey: string | null): string | null {
  const open = (card: CatalogKindCard) => card.existingScopeId === null;
  const door = doorKey ? cards.find((card) => card.key === doorKey && open(card)) : null;
  if (door) return door.key;
  return cards.find((card) => open(card) && (card.holding?.count ?? 0) > 0)?.key ?? null;
}

/** A card's foot line: the count and the newest stamp, "Nothing here yet",
 * or the format label while the count is unknown. */
export function catalogKindCardFoot(card: CatalogKindCard): string {
  const count = card.holding?.count;
  if (count == null) return capabilityFormatLabel(card.capability);
  if (count === 0) return t("library:kindCard.empty");
  const nouns = catalogNouns(card.capability, { plural: card.capability.label, singular: card.capability.singular_label });
  const newest = card.holding?.newest_modified_at;
  return nouns.say((noun) => (newest
    ? t("library:kindCard.countNewest", { count, noun, when: formatRelativeTime(newest) })
    : t("library:kindCard.count", { count, noun })), count);
}

/** The storage a new row starts on: the one a Catalog row used last, else
 * the first of the storages as the shell orders them — Mallard Cloud, the
 * other clouds, then local folders — else none. */
export function catalogDefaultStorage(
  storages: readonly { id: string }[],
  lastUsed: string | null | undefined,
): string | null {
  if (lastUsed && storages.some((storage) => storage.id === lastUsed)) return lastUsed;
  return storages[0]?.id ?? null;
}

/** Why the storage default is the default, in a few words; nothing when the
 * row says it already. */
export function catalogStorageHint(
  storages: readonly { id: string; provider: string }[],
  chosen: string | null,
  lastUsed: string | null | undefined,
): string | null {
  if (storages.length === 0) return t("library:storageHint.none");
  if (!chosen) return null;
  if (chosen === lastUsed) return t("library:storageHint.lastUsed");
  if (storages.length === 1) return t("library:storageHint.only");
  return storages[0]?.id === chosen && storages[0].provider === "managed_cloud" ? t("library:storageHint.cloudFirst") : null;
}
