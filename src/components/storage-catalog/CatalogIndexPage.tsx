import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import ProviderMark from "../sync-review/ProviderMark";
import LandingPicker from "../project-sync/LandingPicker";
import { compactProjectPath } from "../project-sync/model";
import { catalogKindMeta, catalogScopeLabel } from "./model";
import type { CatalogScopeView } from "./types";

interface Props {
  scopes: CatalogScopeView[];
  busy: boolean;
  appActions: PathBarMenuGroup[];
  onOpenScope: (scopeId: string) => void;
  onAddScope: () => void;
}

/** The stable root for Catalog navigation: one door per configured resource
 * scope, drawn from the scope and its profile alone — no Tauri call and no
 * Catalog state. The index does not know which storage a scope links to;
 * that fact lives on the scope's own page. */
export default function CatalogIndexPage({
  scopes,
  busy,
  appActions,
  onOpenScope,
  onAddScope,
}: Props) {
  return (
    <>
      <SyncPathBar
        crumbs={[{ id: "catalog", label: "Catalog", icon: "catalog", tone: "root" }]}
        overflow={appActions}
        overflowLabel="App actions"
        ariaLabel="Catalog path"
      />
      <main className="sync-main sync-project-links-page sync-landing-page">
        <LandingPicker
          title="Catalog"
          count={scopes.length === 1 ? "1 resource on this machine" : `${scopes.length} resources on this machine`}
          actionLabel="Add Catalog resource…"
          emptyTitle="No Catalog resources yet"
          busy={busy}
          onAction={onAddScope}
          cards={scopes.map((scope) => {
            const kind = catalogKindMeta(scope.scope.resource_kind);
            const label = catalogScopeLabel(scope.scope, scope.profile);
            const mark = <ProviderMark provider={scope.scope.provider} displayName={scope.profile.display_name} />;
            return {
              key: scope.scope.scope_id,
              icon: kind.icon,
              title: kind.plural,
              ariaLabel: `Open ${label}`,
              marks: mark,
              location: compactProjectPath(scope.profile.configured_home),
              detail: (
                <>
                  <ProviderMark provider={scope.scope.provider} displayName={scope.profile.display_name} size={13} />
                  {scope.profile.display_name}
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
