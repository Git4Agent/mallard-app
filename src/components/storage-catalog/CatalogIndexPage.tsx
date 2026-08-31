import Icon from "../Icons";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import { catalogKindMeta, catalogScopeLabel } from "./model";
import type { CatalogScopeView } from "./types";

interface Props {
  scopes: CatalogScopeView[];
  busy: boolean;
  appActions: PathBarMenuGroup[];
  onOpenScope: (scopeId: string) => void;
  onAddScope: () => void;
}

/** The stable root for Catalog navigation. It lists each configured resource
 * scope without calling Tauri or owning any Catalog state. */
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
        actions={(
          <button
            type="button"
            className="sync-path-sync"
            disabled={busy}
            onClick={onAddScope}
          >
            <Icon name="plus" size={14} />
            <span className="sync-path-sync-label">Add Catalog resource</span>
          </button>
        )}
        ariaLabel="Catalog path"
      />
      <main className="sync-main sync-project-links-page sync-storage-index-page">
        <section className="profile-links-section" aria-label="Catalog resources">
          {scopes.length === 0 ? (
            <div className="profile-links-empty">
              <Icon name="catalog" size={24} />
              <span>Add a resource to sync an agent profile through storage.</span>
              <button type="button" className="btn" onClick={onAddScope} disabled={busy}>
                <Icon name="plus" size={15} /> Add Catalog resource
              </button>
            </div>
          ) : (
            <div className="sync-storage-index-list">
              {scopes.map((scope) => {
                const label = catalogScopeLabel(scope.scope, scope.profile);
                const kind = catalogKindMeta(scope.scope.resource_kind);
                return (
                  <button
                    key={scope.scope.scope_id}
                    type="button"
                    className="sync-storage-index-row"
                    disabled={busy}
                    onClick={() => onOpenScope(scope.scope.scope_id)}
                    title={`Open ${label}`}
                    aria-label={`Open ${label}`}
                  >
                    <span className="sync-storage-repository-icon">
                      <Icon name={kind.icon} size={14} />
                    </span>
                    <span className="sync-storage-index-copy">
                      <strong>{label}</strong>
                      <small>{scope.profile.configured_home}</small>
                    </span>
                    <Icon
                      name="chevron-right"
                      size={12}
                      className="sync-storage-index-open"
                      style={{ gridColumn: 5 }}
                      aria-hidden="true"
                    />
                  </button>
                );
              })}
            </div>
          )}
        </section>
      </main>
    </>
  );
}
