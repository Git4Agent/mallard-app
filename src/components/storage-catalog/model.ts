import type { IconName } from "../Icons";
import type {
  CatalogLinkState,
  CatalogPullAction,
  CatalogResourceKind,
  CatalogScope,
  LocalCatalogProfile,
} from "./types";

/** The four addable resource kinds. Pet v1 and v2 are distinct rows because
 * the portable format is the discriminator; there is no conversion. */
export const CATALOG_KINDS: Array<{
  kind: CatalogResourceKind;
  label: string;
  plural: string;
  icon: IconName;
}> = [
  { kind: "agent_skill", label: "Skill", plural: "Skills", icon: "file" },
  { kind: "plugin_install_intent", label: "Plugin", plural: "Plugins", icon: "tool" },
  { kind: "codex_pet_v1", label: "Pet v1", plural: "Pets v1", icon: "openai" },
  { kind: "codex_pet_v2", label: "Pet v2", plural: "Pets v2", icon: "openai" },
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

export function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

export function linkStateLabel(state: CatalogLinkState): string {
  return {
    new: "Not pushed",
    in_sync: "In sync",
    local_changed: "Changed locally",
    storage_changed: "Changed in storage",
    diverged: "Diverged",
    local_missing: "Missing locally",
    remote_missing: "Missing in storage",
    unreviewed_remote: "Pull review needed",
    blocked: "Blocked",
  }[state];
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
