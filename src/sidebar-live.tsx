import { StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import ProjectSidebar from "./components/project-sync/ProjectSidebar";
import "./App.css";
import "./RetroTheme.css";

/**
 * The shipped sidebar on a first run — nothing added anywhere — so the three
 * panes can be read side by side. Not part of the app bundle: the build has
 * one entry, and this file is reachable only from the dev server.
 */

const noop = () => undefined;

function Harness() {
  return (
    <div className="sync-app" style={{ "--sync-sidebar-width": "320px" } as React.CSSProperties}>
      <ProjectSidebar
        projects={[]}
        drafts={[]}
        activeDraftId={null}
        storages={[]}
        storageUsage={{}}
        activeProjectId={null}
        activeStorageId={null}
        catalogScopes={[]}
        activeCatalogScopeId={null}
        addingCatalogScope={false}
        settingsOpen={false}
        loading={false}
        busy={false}
        activityOpen={false}
        unreadLogs={0}
        onSelectProject={noop}
        onRemoveProject={noop}
        onSelectDraft={noop}
        onDiscardDraft={noop}
        onToggleActivity={noop}
        onAddProject={noop}
        onOpenStorage={noop}
        onRemoveStorage={noop}
        onAddStorage={noop}
        onOpenCatalogMenu={noop}
        onOpenCatalogScope={noop}
        onAddCatalogScope={noop}
        onRemoveCatalogScope={noop}
        onOpenSettings={noop}
      />
    </div>
  );
}

document.documentElement.setAttribute("data-theme", "light");
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
