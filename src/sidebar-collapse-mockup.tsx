import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import ProjectSidebar from "./components/project-sync/ProjectSidebar";
import type { CatalogScopeView } from "./components/storage-catalog/types";
import type { LocalProjectSummary, StorageConfig } from "./types";
import "./App.css";

/**
 * Harness: collapsible sidebar sections, on the real ProjectSidebar.
 *
 * This mounts the shipped component over the shipped CSS — every toggle,
 * drag handle, and inert state here is the production code path, with only
 * the data and the surrounding shell faked. Collapse state and the split
 * persist to this origin's localStorage under the real keys.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1100px; margin: 0 auto; padding: 24px 22px 70px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 16px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; align-items: center; margin: 0 0 20px; flex-wrap: wrap; }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 5px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--bg-active); color: var(--text-0); border-color: var(--border-mid); }
  .mk-frame { display: flex; height: 900px; width: 340px; padding: 3px; overflow: hidden; border: 1px dashed var(--border-mid); border-radius: 6px; background: var(--retro-frame-gutter); }
  .mk-frame .sync-sidebar { width: 100%; min-width: 0; }
`;

const project = (id: string, name: string, git: boolean, profiled = true): LocalProjectSummary => ({
  local_project_id: id,
  bundle_id: `bundle-${id}`,
  display_name: name,
  local_alias: name,
  repository_fingerprint: "fingerprint",
  project_root: `/Users/user/${name}`,
  canonical_project_root: `/Users/user/${name}`,
  profile_ids: profiled ? { codex: "profile-codex" } : {},
  profile_names: profiled ? ["Default Codex"] : [],
  is_git_repository: git,
  revision: 1,
});

const scope = (id: string, kind: CatalogScopeView["scope"]["resource_kind"]): CatalogScopeView => ({
  scope: {
    scope_id: id,
    provider: "codex",
    profile_id: "catprofile",
    resource_kind: kind,
    revision: 1,
    created_at: 1,
    updated_at: 1,
  },
  profile: {
    profile_id: "catprofile",
    provider_key: "codex",
    display_name: "Default Codex",
    configured_home: "/Users/user/.codex",
    revision: 1,
    created_at: 1,
    updated_at: 1,
  },
});

const storage = (index: number): StorageConfig => ({
  id: `storage-${index}`,
  name: `Local storage ${index}`,
  kind: "local",
  bucket: "",
  access_key_id: "",
  secret_access_key: "",
  account_id: "",
  s3_endpoint: "",
  region: "",
  local_dir: `/tmp/storage-${index}`,
});

const PROJECTS = [
  project("p1", "mallard-app (Claude)", true),
  project("p2", "mallardInternal", true),
  project("p3", "HorrorGame", false),
  project("p4", "HorrorGame (2)", false, false),
];
const SCOPES: CatalogScopeView[] = [
  scope("s1", "codex_pet_v2"),
  scope("s2", "agent_skill"),
  scope("s3", "plugin_install_intent"),
];
const STORAGES = [1, 2, 3, 4, 5, 6].map(storage);

function Page() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  document.documentElement.dataset.theme = theme;
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Collapsible sidebar sections — live harness</h1>
      <p className="mk-lede">
        The real ProjectSidebar over the shipped CSS. Click a section title to fold or reopen
        it; drag the seams. A gutter beside a closed Catalog re-pins Storages across the
        closed tile; a gutter with no open pane on one side is inert (no cursor, no tint).
        Collapse state and the split persist to this origin&apos;s storage under the real keys.
      </p>
      <div className="mk-bar" role="group" aria-label="Theme">
        <button type="button" className={theme === "light" ? "on" : undefined} onClick={() => setTheme("light")}>Light</button>
        <button type="button" className={theme === "dark" ? "on" : undefined} onClick={() => setTheme("dark")}>Dark</button>
      </div>
      <div className="mk-frame">
        <ProjectSidebar
          projects={PROJECTS}
          drafts={[]}
          activeDraftId={null}
          storages={STORAGES}
          storageDrafts={[]}
          storageUsage={{}}
          storageIssues={{ "storage-6": "The storage folder is missing." }}
          activeProjectId="p1"
          activeStorageId={null}
          catalogScopes={SCOPES}
          activeCatalogScopeId={null}
          addingCatalogScope={false}
          settingsOpen={false}
          loading={false}
          busy={false}
          activityOpen={false}
          unreadLogs={0}
          onSelectProject={() => undefined}
          onRemoveProject={() => undefined}
          onSelectDraft={() => undefined}
          onDiscardDraft={() => undefined}
          onToggleActivity={() => undefined}
          onAddProject={() => undefined}
          onOpenStorage={() => undefined}
          onOpenStorageDraft={() => undefined}
          onDiscardStorageDraft={() => undefined}
          onRemoveStorage={() => undefined}
          onAddStorage={() => undefined}
          onOpenCatalogMenu={() => undefined}
          onOpenCatalogScope={() => undefined}
          onAddCatalogScope={() => undefined}
          onRemoveCatalogScope={() => undefined}
          onOpenSettings={() => undefined}
        />
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
