import { useState } from "react";
import { createRoot } from "react-dom/client";
import Icon, { storageIconName } from "./components/Icons";
import StorageIndexPage from "./components/project-sync/StorageIndexPage";
import StorageProviderPicker from "./components/project-sync/StorageProviderPicker";
import ProjectSidebar from "./components/project-sync/ProjectSidebar";
import type { StorageProviderDescriptor, StorageSummary } from "./types";
import "./App.css";

// Presentation fixture. These components receive sample data and local callbacks;
// the preview makes no Tauri calls and reads no provider homes or storage data.
const definitions = [
  ["local", "Local folder", "Local workspace", "~/Projects/shared-storage"],
  ["managed_cloud", "Mallard Cloud", "Mallard Cloud", "Demo workspace"],
  ["feishu", "Feishu", "Feishu storage", "Team files"],
  ["dropbox", "Dropbox", "Dropbox storage", "Shared projects"],
  ["r2", "Cloudflare R2", "R2 storage", "team-backups"],
];
const providers: StorageProviderDescriptor[] = definitions.map(([key, label], index) => ({
  key, config_version: 1, display_name: label, description: `Connect ${label} storage.`,
  display_rank: index, icon: key === "local" ? "drive" : "cloud", clear_mode: "unavailable",
  fields: [], secrets: [],
}));
const storages: StorageSummary[] = definitions.map(([provider, providerLabel, name, detail]) => ({
  storage_id: `preview-${provider}`, provider, provider_label: providerLabel, name,
  config_version: 1, icon: provider === "local" ? "drive" : "cloud", available: true,
  summary: [{ field_id: "location", label: "Location", value: detail, is_path: provider === "local" }],
  settings: {}, secrets: [],
}));
const noop = () => {};

function Preview() {
  const [dark, setDark] = useState(false);
  const [sidebar, setSidebar] = useState(false);
  const [all, setAll] = useState(false);
  const [picker, setPicker] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const shown = all ? storages : storages.slice(0, 3);
  const open = (id: string) => setSelected(storages.find((storage) => storage.storage_id === id)?.name ?? id);
  const choose = () => setPicker(true);
  return (
    <>
      <div className="storage-icons-preview-controls">
        <strong>Storage icon preview</strong>
        <button className="btn" onClick={() => {
          document.documentElement.dataset.theme = dark ? "light" : "dark";
          setDark(!dark);
        }}>{dark ? "Light theme" : "Dark theme"}</button>
        <button className="btn" onClick={() => setSidebar(!sidebar)}>{sidebar ? "Hide sidebar" : "Show sidebar"}</button>
        <button className="btn" onClick={() => setAll(!all)}>{all ? "Show three storages" : "Show all five"}</button>
        <span role="status">{selected ? `Selected: ${selected}` : "Sample data · no storage access"}</span>
      </div>
      <div className={`sync-app storage-icons-preview${sidebar ? "" : " sidebar-hidden"}`}>
        {sidebar && <ProjectSidebar
          projects={[]} drafts={[]} activeDraftId={null} storages={shown} storageDrafts={[]}
          storageProviders={providers} storageUsage={{}} activeProjectId={null} activeStorageId={null}
          catalogScopes={[]} activeCatalogScopeId={null} addingProject={false} addingCatalogScope={false} addingStorage={false}
          catalogIndexOpen={false} storageIndexOpen settingsOpen={false}
          loading={false} busy={false} activityOpen={false} unreadLogs={0}
          onOpenProjects={noop} onSelectProject={noop} onRemoveProject={noop} onSelectDraft={noop}
          onDiscardDraft={noop} onToggleActivity={noop} onAddProject={noop}
          onOpenStorage={open} onOpenStorageDraft={noop} onDiscardStorageDraft={noop}
          onRemoveStorage={noop} onAddStorage={choose} onOpenCatalogIndex={noop}
          onOpenCatalogScope={noop} onAddCatalogScope={noop} onRemoveCatalogScope={noop}
          onOpenStorageIndex={() => setPicker(false)} onOpenSettings={noop}
        />}
        <div className="sync-workspace">
          {picker ? <StorageProviderPicker providers={providers} busy={false} appActions={[]}
            onChoose={(key) => { setSelected(key); setPicker(false); }} onCancel={() => setPicker(false)} />
            : <StorageIndexPage storages={shown} links={[]} storageIssues={{}} drafts={[]}
              storageProviders={providers} busy={false} appActions={[]}
              onOpenStorage={open} onOpenStorageDraft={noop} onAddStorage={choose} />}
        </div>
      </div>
      <section className="storage-icons-preview-samples" aria-label="Provider icon set">
        <h2>Provider icon set</h2>
        <div>{providers.map((provider) => <div key={provider.key}>
          <span className="landing-picker-card-title"><Icon name={storageIconName(provider.key)} size={24} /></span>
          <span>{provider.display_name}</span>
          <span className="storage-icons-preview-small"><Icon name={storageIconName(provider.key)} size={14} />14px <Icon name={storageIconName(provider.key)} size={18} />18px</span>
        </div>)}</div>
      </section>
      <style>{`
        html, body, #root { height: auto; min-height: 100%; overflow: visible; }
        .storage-icons-preview-controls { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; padding: 12px 24px; background: var(--surface-card); border-bottom: 1px solid var(--border); }
        .storage-icons-preview-controls > strong { margin-right: auto; }
        .storage-icons-preview-controls > span { color: var(--text-2); font-size: 12px; }
        .storage-icons-preview.sync-app { height: 590px; min-height: 0; padding: 8px; }
        .storage-icons-preview .sync-workspace { min-width: 0; }
        .storage-icons-preview .sync-main { overflow-y: auto; }
        .storage-icons-preview-samples { padding: 20px 40px; border-top: 1px solid var(--border); }
        .storage-icons-preview-samples h2 { font: var(--weight-bold) 20px var(--font-display); margin-bottom: 20px; }
        .storage-icons-preview-samples > div { display: grid; grid-template-columns: repeat(5, 1fr); gap: 20px; }
        .storage-icons-preview-samples > div > div { display: flex; flex-direction: column; align-items: center; gap: 10px; }
        .storage-icons-preview-samples .landing-picker-card-title > svg { margin: 0; }
        .storage-icons-preview-small { display: flex; align-items: center; gap: 5px; font: 11px var(--font-mono); color: var(--text-2); }
        @media (max-width: 600px) { .storage-icons-preview-samples > div { grid-template-columns: repeat(3, 1fr); } }
      `}</style>
    </>
  );
}

const previewRoot = createRoot(document.getElementById("root")!);
previewRoot.render(<Preview />);
if (import.meta.hot) import.meta.hot.dispose(() => previewRoot.unmount());
