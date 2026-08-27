import type { IconName } from "../Icons";
import { clockOptions } from "../../clock";
import { errorMessage } from "../../storageError";
import type { ThreadStatusIconName } from "../sync-review/ThreadStatusIcon";
import type {
  CatalogPullAction,
  CatalogResourceKind,
  CatalogScope,
  CatalogScopeRow,
  LocalCatalogProfile,
} from "./types";

/** The four addable resource kinds. Pet v1 and v2 are distinct rows because
 * the portable format is the discriminator; there is no conversion — so they
 * share one glyph and are told apart by their label.
 *
 * Each kind wears a mark of its own. They used to borrow: `file` said where a
 * skill was stored rather than what it was, `tool` is the cog this app spends
 * on a tool call, and `openai` names the *provider* — which `ProviderMark`
 * already paints on the same row, so it told a reader nothing. */
export const CATALOG_KINDS: Array<{
  kind: CatalogResourceKind;
  label: string;
  plural: string;
  icon: IconName;
}> = [
  { kind: "agent_skill", label: "Skill", plural: "Skills", icon: "lightbulb" },
  { kind: "plugin_install_intent", label: "Plugin", plural: "Plugins", icon: "puzzle" },
  { kind: "codex_pet_v1", label: "Pet v1", plural: "Pets v1", icon: "paw" },
  { kind: "codex_pet_v2", label: "Pet v2", plural: "Pets v2", icon: "paw" },
];

export function catalogKindMeta(kind: CatalogResourceKind) {
  return CATALOG_KINDS.find((item) => item.kind === kind) ?? CATALOG_KINDS[0];
}

/** "Skills · Codex home" — the row label always derives from the kind plus the
 * profile's current name, so renaming a profile renames its rows. */
export function catalogScopeLabel(
  scope: CatalogScope,
  profile: Pick<LocalCatalogProfile, "display_name"> | null,
): string {
  const meta = catalogKindMeta(scope.resource_kind);
  return profile ? `${meta.plural} · ${profile.display_name}` : meta.plural;
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
    case "diverged":
      return "divergent";
    case "unreviewed_remote":
      return "pending";
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
    case "diverged":
      return `Changed here and in ${storage}. Review before syncing.`;
    case "unreviewed_remote":
      return `Held until you Pull: ${storage} has a version this computer has never reviewed.`;
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
  if (!row.remote_present) return false;
  return row.state === "storage_changed"
    || row.state === "local_missing"
    || row.state === "diverged"
    || row.state === "unreviewed_remote";
}

/** The date a row last moved, and which side moved it. Local time wins when
 * the resource is on this Mac, because that is the copy you can edit. */
export function rowStamp(row: CatalogScopeRow): { short: string; detail: string } | null {
  const local = row.local_present ? row.local_modified_at ?? null : null;
  const seconds = local ?? row.remote_updated_at ?? null;
  if (seconds == null) return null;
  const when = new Date(seconds * 1000);
  const elapsed = Date.now() - when.getTime();
  const short = elapsed < 60_000
    ? "just now"
    : elapsed < 3_600_000
      ? `${Math.floor(elapsed / 60_000)}m ago`
      : elapsed < 86_400_000
        ? `${Math.floor(elapsed / 3_600_000)}h ago`
        : when.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  const exact = `${when.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" })} · ${when.toLocaleTimeString(undefined, clockOptions())}`;
  return { short, detail: `${local == null ? "Stored" : "Edited"} ${exact}` };
}

export function actionLabel(action: CatalogPullAction): string {
  switch (action.kind.kind) {
    case "preserve_skill_as_copy_and_overwrite":
    case "preserve_pet_as_copy_and_overwrite":
      return `Keep local as ${action.kind.copy.copy_name} and overwrite`;
    case "overwrite_plugin_with_storage":
    case "overwrite_skill_with_storage":
    case "overwrite_pet_with_storage":
      return "Overwrite local content";
    case "install_plugin":
    case "install_skill":
    case "install_pet":
      return "Install stored content";
    default:
      return "Update from storage";
  }
}
