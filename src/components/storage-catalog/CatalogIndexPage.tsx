import { useTranslation } from "../../i18n";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import ProviderMark from "../sync-review/ProviderMark";
import LandingPicker, { type LandingDoor } from "../project-sync/LandingPicker";
import { compactProjectPath } from "../project-sync/model";
import { catalogDoors, catalogScopeIcon, catalogScopeKindLabel, catalogScopeLabel } from "./model";
import type { CatalogCapabilityDescriptor, CatalogScopeView } from "./types";

interface Props {
  scopes: CatalogScopeView[];
  /** The registry the Add chooser offers: with nothing listed, one door per
   *  shape it registers, each opening Add on that shape. */
  capabilities: CatalogCapabilityDescriptor[];
  busy: boolean;
  appActions: PathBarMenuGroup[];
  onOpenScope: (scopeId: string) => void;
  /** Add, with the chooser's Kind already made when a door names one. */
  onAddScope: (choiceKey?: string) => void;
}

/** The stable root for Catalog navigation: one door per configured resource
 * scope, drawn from the scope and its profile alone — no Tauri call and no
 * Catalog state. The index does not know which storage a scope links to;
 * that fact lives on the scope's own page. */
export default function CatalogIndexPage({
  scopes,
  capabilities,
  busy,
  appActions,
  onOpenScope,
  onAddScope,
}: Props) {
  const { t } = useTranslation("library");
  const doors: LandingDoor[] = catalogDoors(capabilities).map((door) => ({
    key: door.key,
    icon: door.icon,
    label: door.label,
    ariaLabel: t("index.add", { label: door.label }),
    marks: door.providers.map((capability) => (
      <ProviderMark key={capability.provider} provider={capability.provider} displayName={capability.provider_label} />
    )),
    onOpen: () => onAddScope(door.key),
  }));
  return (
    <>
      <SyncPathBar
        crumbs={[{ id: "catalog", label: t("index.title"), icon: "catalog", tone: "root" }]}
        overflow={appActions}
        overflowLabel={t("bar.appActions")}
        ariaLabel={t("index.path")}
      />
      <main className="sync-main sync-project-links-page sync-landing-page">
        <LandingPicker
          title={t("index.title")}
          count={t("index.count", { count: scopes.length })}
          actionLabel={t("index.addResource")}
          busy={busy}
          onAction={() => onAddScope()}
          /* A registry that has not answered yet still leaves one door: the
             chooser itself. */
          doors={doors.length > 0 ? doors : [{
            key: "any",
            icon: "catalog",
            label: t("index.addResource"),
            ariaLabel: t("index.addResourceLabel"),
            onOpen: () => onAddScope(),
          }]}
          caption={{ icons: ["cloud"], text: t("index.caption") }}
          cards={scopes.map((scope) => {
            const label = catalogScopeLabel(scope);
            const mark = <ProviderMark provider={scope.scope.provider} displayName={scope.profile.display_name} />;
            return {
              key: scope.scope.scope_id,
              icon: catalogScopeIcon(scope),
              title: catalogScopeKindLabel(scope),
              ariaLabel: t("index.open", { label }),
              marks: mark,
              location: compactProjectPath(scope.profile.configured_home),
              /* A row this build cannot operate on says so where its profile
                 fact goes: the door still opens, onto the page that states
                 the same reason and offers no sync. */
              detail: (
                <>
                  <ProviderMark provider={scope.scope.provider} displayName={scope.profile.display_name} size={13} />
                  {scope.profile.display_name}
                  {!scope.available && ` — ${scope.unavailable_reason ?? t("index.unsupported")}`}
                </>
              ),
              onOpen: () => onOpenScope(scope.scope.scope_id),
            };
          })}
        />
      </main>
    </>
  );
}
