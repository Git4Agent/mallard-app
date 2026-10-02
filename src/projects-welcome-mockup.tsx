import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import ProjectPicker from "./components/project-sync/ProjectPicker";
import ProjectSidebar from "./components/project-sync/ProjectSidebar";
import "./App.css";

/**
 * Specimen: the Projects landing on a machine with no projects — the real
 * ProjectPicker, which welcomes with a mallard ride when its list is empty.
 * Reopen remounts it, as returning to the page does, so the next ride differs.
 *
 * `npm run dev`, then /projects-welcome-mockup.html
 * (`?width=800|1100|1440&theme=light|dark`). Sample data only: the preview
 * makes no Tauri calls and reads no provider homes or storage data.
 */

const noop = () => {};

function Segmented<T extends string>({ label, value, options, onChange }: {
  label: string;
  value: T;
  options: [T, string][];
  onChange: (value: T) => void;
}) {
  return (
    <span className="welcome-preview-group">
      <span className="welcome-preview-label">{label}</span>
      <span className="sync-segmented" role="group" aria-label={label}>
        {options.map(([key, text]) => (
          <button key={key} type="button" className={value === key ? "active" : undefined} aria-pressed={value === key} onClick={() => onChange(key)}>
            {text}
          </button>
        ))}
      </span>
    </span>
  );
}

function Preview() {
  const params = new URLSearchParams(window.location.search);
  const [visit, setVisit] = useState(0);
  const [dark, setDark] = useState(params.get("theme") === "dark");
  const [width, setWidth] = useState(params.get("width") ?? "1100");
  const [last, setLast] = useState("Sample data · no Tauri calls");

  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);

  return (
    <>
      <div className="welcome-preview-controls">
        <strong>Projects welcome</strong>
        <button type="button" className="btn btn-ghost" onClick={() => setVisit((current) => current + 1)}>
          <Icon name="refresh" size={13} /> Reopen (next ride)
        </button>
        <Segmented label="Window" value={width} onChange={setWidth} options={[["800", "800"], ["1100", "1100"], ["1440", "1440"]]} />
        <Segmented label="Theme" value={dark ? "dark" : "light"} onChange={(next) => setDark(next === "dark")} options={[["light", "Light"], ["dark", "Dark"]]} />
        <span role="status" className="welcome-preview-status">{last}</span>
      </div>
      <div className="welcome-preview-stage">
        <div className="sync-app welcome-preview-window" style={{ width: `${width}px` }}>
          <ProjectSidebar
            projects={[]} drafts={[]} activeDraftId={null} storages={[]} storageDrafts={[]}
            storageProviders={[]} storageUsage={{}} activeProjectId={null} activeStorageId={null}
            catalogScopes={[]} catalogCapabilities={[]} activeCatalogScopeId={null} addingProject={false} addingCatalogScope={false} addingStorage={false}
            catalogIndexOpen={false} storageIndexOpen={false} settingsOpen={false}
            loading={false} busy={false} activityOpen={false} unreadLogs={0}
            onOpenProjects={noop} onSelectProject={noop} onRemoveProject={noop} onSelectDraft={noop}
            onDiscardDraft={noop} onToggleActivity={noop} onAddProject={noop}
            onOpenStorage={noop} onOpenStorageDraft={noop} onDiscardStorageDraft={noop}
            onRemoveStorage={noop} onAddStorage={noop} onOpenCatalogIndex={noop}
            onOpenCatalogScope={noop} onAddCatalogScope={noop} onRemoveCatalogScope={noop}
            onOpenStorageIndex={noop} onOpenSettings={noop}
          />
          <div className="sync-workspace">
            <header className="sync-titlebar">
              <div className="sync-titlebar-context">
                <Icon name="folder" size={16} />
                <strong>Projects</strong>
              </div>
              <div className="sync-titlebar-actions">
                <div className="storage-link-overflow">
                  <button type="button" className="storage-link-more" aria-label="App actions">
                    <Icon name="more" size={16} />
                  </button>
                </div>
              </div>
            </header>
            <main className="sync-main sync-project-links-page sync-landing-page">
              <ProjectPicker
                key={visit}
                projects={[]}
                profiles={[]}
                storages={[]}
                onAddProject={() => setLast("Add project… → New project, Folder")}
                onAddCloudProject={() => setLast("From Mallard Cloud… → New project, Mallard Cloud")}
                onSelectProject={noop}
              />
            </main>
          </div>
        </div>
      </div>
      <style>{`
        html, body, #root { height: auto; min-height: 100%; overflow: visible; }
        body { background: var(--retro-frame-gutter, var(--bg-0)); }
        .welcome-preview-controls { position: sticky; top: 0; z-index: 30; display: flex; gap: 10px 16px; align-items: center; flex-wrap: wrap; padding: 10px 20px; background: var(--surface-card); border-bottom: 1px solid var(--border); font-size: 12px; }
        .welcome-preview-controls > strong { font-family: var(--font-display); font-size: 14px; margin-right: 6px; }
        .welcome-preview-group { display: inline-flex; align-items: center; gap: 6px; }
        .welcome-preview-label { color: var(--text-3); font-family: var(--font-mono); font-size: 10px; letter-spacing: .08em; text-transform: uppercase; }
        .welcome-preview-status { color: var(--text-2); }
        .welcome-preview-stage { padding: 24px; overflow-x: auto; }
        .welcome-preview-window.sync-app { margin: 0 auto; height: 720px; border: var(--box-border); border-radius: 14px; background: var(--bg-0); box-shadow: var(--shadow-card); }
      `}</style>
    </>
  );
}

const previewRoot = createRoot(document.getElementById("root")!);
previewRoot.render(<Preview />);
if (import.meta.hot) import.meta.hot.dispose(() => previewRoot.unmount());
