import Icon from "../Icons";
import type { ProjectStorageLink, StorageConfig } from "../../types";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import { storageSubtitle } from "./ProjectLinksWorkspace";

/**
 * The page the storage trail is rooted at: every storage page begins its path
 * with a Storages crumb, and a crumb has to lead somewhere real. This is that
 * somewhere — one row per configured storage, each opening its page.
 *
 * The sidebar lists the same storages as a shortcut rail; this page is the
 * destination those shortcuts skip past, so it stays a plain index: no
 * per-row menus, no editors. Every verb lives on the storage's own page.
 */

interface Props {
  storages: StorageConfig[];
  links: ProjectStorageLink[];
  storageIssues: Record<string, string>;
  busy: boolean;
  /** Theme and app-wide commands; the bar here is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  onOpenStorage: (storageId: string) => void;
  onAddStorage: () => void;
}

export default function StorageIndexPage({
  storages,
  links,
  storageIssues,
  busy,
  appActions,
  onOpenStorage,
  onAddStorage,
}: Props) {
  return (
    <>
      <SyncPathBar
        crumbs={[{ id: "storages", label: "Storages", icon: "drive", tone: "root" }]}
        overflow={appActions}
        overflowLabel="App actions"
        actions={(
          <button
            type="button"
            className="sync-path-sync"
            disabled={busy}
            onClick={onAddStorage}
          >
            <Icon name="plus" size={14} />
            <span className="sync-path-sync-label">Add storage</span>
          </button>
        )}
        ariaLabel="Storages path"
      />
      <main className="sync-main sync-project-links-page sync-storage-index-page">
        <section className="profile-links-section" aria-label="Storages">
          {storages.length === 0 ? (
            <div className="profile-links-empty">
              <Icon name="drive" size={24} />
              <span>Add a storage to choose where projects sync.</span>
              <button type="button" className="btn" onClick={onAddStorage} disabled={busy}>
                <Icon name="plus" size={15} /> Add storage
              </button>
            </div>
          ) : (
            <div className="sync-storage-index-list">
              {storages.map((storage) => {
                const name = storage.name || "(unnamed)";
                const subtitle = storageSubtitle(storage);
                const issue = storageIssues[storage.id] ?? null;
                const linked = links.filter((link) => link.storage_id === storage.id).length;
                return (
                  <button
                    key={storage.id}
                    type="button"
                    className="sync-storage-index-row"
                    disabled={busy}
                    onClick={() => onOpenStorage(storage.id)}
                    title={issue ? `Open ${name} — ${issue}` : `Open ${name}`}
                    aria-label={issue ? `Open ${name}. Storage issue: ${issue}` : `Open ${name}`}
                  >
                    <span className="sync-storage-repository-icon">
                      <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={14} />
                    </span>
                    <span className="sync-storage-index-copy">
                      <strong>{name}</strong>
                      <small>{subtitle}</small>
                    </span>
                    {issue && (
                      <Icon name="alert-triangle" size={13} className="sync-storage-index-warning" aria-hidden="true" />
                    )}
                    <span className="sync-storage-repo-chip">
                      <span>Projects</span>
                      <b>{linked}</b>
                    </span>
                    <Icon name="chevron-right" size={12} className="sync-storage-index-open" aria-hidden="true" />
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
