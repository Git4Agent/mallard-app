import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "../src/components/Icons";
import ProjectSidebar from "../src/components/project-sync/ProjectSidebar";
import ProjectPicker from "../src/components/project-sync/ProjectPicker";
import NewProjectFolderStep from "../src/components/project-sync/NewProjectFolderStep";
import type {
  AgentFolder,
  LocalProjectSummary,
  ProjectFolderProbe,
  ProviderDescriptor,
  SetupDraftSummary,
} from "../src/types";
import "../src/App.css";

/**
 * Mockup: the sidebar's + while New project is the open page.
 *
 * Today the rail's + (and an empty rail's folder door) stay live on New
 * project and call ProjectSync.beginAddProject again. On "On this computer"
 * that changes nothing, so the button reads as broken. Opened on From
 * Mallard Cloud (the welcome page's Cloud door, or a Cloud account's
 * landing), NewProjectFolderStep is keyed by its source, so the + remounts it
 * on "On this computer" and drops the Cloud pick.
 *
 *   Before     the + is live.
 *   A · Dimmed the + (and the door) are disabled, in the stock :disabled
 *              grammar the rail already uses while busy. Chosen and shipped
 *              as ProjectSidebar's `addingProject`; Add Catalog resource and
 *              the storage type picker hold their rail's + the same way.
 *
 * Only New project holds the +. A draft's setup page keeps it live: it leads
 * somewhere else, and the draft is already saved as a rail row.
 *
 * Real ProjectSidebar, ProjectPicker, NewProjectFolderStep and App.css on a
 * mocked backend. `npm run dev`, then /mockups/sidebar-add-current-mockup.html
 * (`?treatment=now|dimmed&screen=landing|new|draft|project&rail=some|empty&theme=dark`).
 */

/* ——— data ——————————————————————————————————————————————————————— */

const HOME = "/Users/you";
interface Row { name: string; rel: string; providers: string[]; ago: number; git: boolean; kind: "new" | "added" | "draft" }
const ROWS: Row[] = [
  { name: "mallard-app", rel: "Desktop/project/mallard-app", providers: ["claude", "codex"], ago: 30, git: true, kind: "draft" },
  { name: "physics 225", rel: "Desktop/physics 225", providers: ["codex"], ago: 3_600, git: false, kind: "added" },
  { name: "dujiangyan", rel: "Documents/ChatGPT/dujiangyan", providers: ["codex"], ago: 3_700, git: true, kind: "added" },
  { name: "new-chat", rel: "Documents/Codex/2026-09-22/new-chat", providers: ["codex"], ago: 3_800, git: false, kind: "new" },
  { name: "mallardInternal", rel: "Desktop/project/mallardInternal", providers: ["codex"], ago: 79_200, git: true, kind: "new" },
  { name: "project", rel: "Desktop/project", providers: ["codex"], ago: 79_300, git: false, kind: "new" },
  { name: "blog", rel: "code/blog", providers: ["claude"], ago: 200_000, git: true, kind: "new" },
  { name: "notes", rel: "notes", providers: ["claude"], ago: 400_000, git: false, kind: "new" },
];
const pathOf = (row: Row) => `${HOME}/${row.rel}`;

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, maintenance_actions: [] },
] as ProviderDescriptor[];

const PROJECTS = ROWS.filter((row) => row.kind === "added").map((row) => ({
  local_project_id: `project-${row.name}`,
  bundle_id: `bundle-${row.name}`,
  display_name: row.name,
  revision: 1,
  project_root: pathOf(row),
  canonical_project_root: pathOf(row),
  is_git_repository: row.git,
  profile_ids: { codex: "profile-codex" },
})) as LocalProjectSummary[];

const DRAFTS: SetupDraftSummary[] = ROWS.filter((row) => row.kind === "draft").map((row) => ({
  draft_id: `draft-${row.name}`,
  display_name: row.name,
  project_root: pathOf(row),
  updated_at: Math.floor(Date.now() / 1000) - row.ago,
  revision: 1,
  status: "draft",
}));

function probeOf(row: Row): ProjectFolderProbe {
  return {
    kind: "folder",
    canonical_path: pathOf(row),
    is_git_repository: row.git,
    existing_draft_id: row.kind === "draft" ? `draft-${row.name}` : null,
    existing_project_ids: row.kind === "added" ? [`project-${row.name}`] : [],
  };
}

/* The real step runs against these. */
(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC((command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  if (command === "plugin:path|resolve_directory") return HOME;
  if (command === "list_agent_folders") {
    return {
      folders: ROWS.map((row): AgentFolder => ({
        path: pathOf(row),
        providers: row.providers,
        last_active: Math.floor(Date.now() / 1000) - row.ago,
        probe: probeOf(row),
      })),
    };
  }
  if (command === "probe_project_folder") {
    const row = ROWS.find((candidate) => pathOf(candidate) === String(args.projectRoot));
    return row ? probeOf(row) : { kind: "missing" };
  }
  return null;
});

/* ——— knobs ———————————————————————————————————————————————————————— */

type Treatment = "now" | "dimmed";
type Screen = "landing" | "new" | "draft" | "project";
type Rail = "some" | "empty";
type Theme = "light" | "dark";
interface Knobs { treatment: Treatment; screen: Screen; rail: Rail; theme: Theme }

const TREATMENTS: Array<{ id: Treatment; label: string; note: string }> = [
  { id: "now", label: "Before", note: "Live; clicking it on New project does nothing visible." },
  { id: "dimmed", label: "A · Dimmed", note: "Stock :disabled — the same look busy gives it." },
];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    treatment: pick("treatment", ["now", "dimmed"] as const, "dimmed"),
    screen: pick("screen", ["landing", "new", "draft", "project"] as const, "new"),
    rail: pick("rail", ["some", "empty"] as const, "some"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/* ——— the sidebar ———————————————————————————————————————————————— */

function Sidebar({ treatment, screen, rail, onScreen }: {
  treatment: Treatment;
  screen: Screen;
  rail: Rail;
  onScreen: (screen: Screen) => void;
}) {
  return (
    <ProjectSidebar
      projects={rail === "some" ? PROJECTS : []}
      drafts={rail === "some" ? DRAFTS : []}
      activeDraftId={screen === "draft" ? DRAFTS[0].draft_id : null}
      storages={[]}
      storageDrafts={[]}
      storageProviders={[]}
      storageUsage={{}}
      activeProjectId={screen === "project" ? PROJECTS[0].local_project_id : null}
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingProject={screen === "new" && treatment === "dimmed"}
      addingCatalogScope={false}
      addingStorage={false}
      catalogIndexOpen={false}
      storageIndexOpen={false}
      settingsOpen={false}
      loading={false}
      busy={false}
      activityOpen={false}
      unreadLogs={10}
      onOpenProjects={() => onScreen("landing")}
      onSelectProject={() => onScreen("project")}
      onRemoveProject={() => undefined}
      onSelectDraft={() => onScreen("draft")}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => onScreen("new")}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogIndex={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
      onOpenStorageIndex={() => undefined}
      onOpenSettings={() => undefined}
    />
  );
}

/* ——— the live frame —————————————————————————————————————————————— */

function Workspace({ screen, rail, onScreen }: { screen: Screen; rail: Rail; onScreen: (screen: Screen) => void }) {
  const projects = rail === "some" ? PROJECTS : [];
  const title = screen === "landing" ? "Projects"
    : screen === "new" ? "New project"
      : screen === "draft" ? DRAFTS[0].display_name
        : PROJECTS[0].display_name;
  return (
    <div className="sync-workspace">
      <header className="sync-titlebar">
        <div className="sync-titlebar-context">
          <Icon name="folder" size={16} />
          <strong>{title}</strong>
          {screen === "draft" && <span>Draft setup</span>}
        </div>
      </header>
      {screen === "landing" ? (
        <main className="sync-main sync-project-links-page sync-landing-page">
          <ProjectPicker
            projects={projects}
            profiles={[]}
            storages={[]}
            rememberedStorageByProject={{}}
            onAddProject={() => onScreen("new")}
            onAddCloudProject={() => onScreen("new")}
            onSelectProject={() => onScreen("project")}
          />
        </main>
      ) : screen === "new" ? (
        <main className="sync-main sync-project-links-page sync-project-setup-page">
          <section className="profile-links-section sync-project-overview" aria-label="Project setup">
            <NewProjectFolderStep
              workflowEpoch={1}
              isWorkflowCurrent={() => true}
              projects={projects}
              providers={PROVIDERS}
              profiles={[]}
              storages={[]}
              busy={false}
              onCancel={() => onScreen("landing")}
              onChoose={() => onScreen("draft")}
              onOpenProject={() => onScreen("project")}
              onAddCloudAccount={() => undefined}
              onCloudProjectDone={() => undefined}
            />
          </section>
        </main>
      ) : (
        <main className="sync-main">
          <div className="spec-placeholder">
            <strong>{screen === "draft" ? `Set up ${DRAFTS[0].display_name}` : PROJECTS[0].display_name}</strong>
            {screen === "draft"
              ? "The draft's setup page — not part of this specimen. The + stays live here: it opens New project, and this draft keeps its rail row."
              : "The project's workspace — not part of this specimen. The + is live."}
          </div>
        </main>
      )}
    </div>
  );
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = (patch: Partial<Knobs>) => setKnobs((current) => ({ ...current, ...patch }));
  const onScreen = (screen: Screen) => set({ screen });

  useEffect(() => {
    document.documentElement.dataset.theme = knobs.theme;
    const query = new URLSearchParams({ ...knobs });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
  }, [knobs]);

  const segs = <K extends keyof Knobs>(key: K, label: string, options: Array<{ id: Knobs[K]; label: string }>) => (
    <div className="spec-group">
      <span>{label}</span>
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          className="spec-seg"
          aria-pressed={knobs[key] === option.id}
          onClick={() => set({ [key]: option.id } as Partial<Knobs>)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );

  return (
    <div className="spec">
      <h1 className="spec-title">Sidebar + while New project is open</h1>

      <div className="spec-controls">
        {segs("treatment", "Treatment", TREATMENTS)}
        {segs("screen", "Page", [
          { id: "landing", label: "Projects" },
          { id: "new", label: "New project" },
          { id: "draft", label: "Draft setup" },
          { id: "project", label: "Project" },
        ])}
        {segs("rail", "Rail", [{ id: "some", label: "Some" }, { id: "empty", label: "Empty" }])}
        {segs("theme", "Theme", [{ id: "light", label: "Light" }, { id: "dark", label: "Dark" }])}
      </div>

      <div className="spec-card">
        <strong>{TREATMENTS.find((treatment) => treatment.id === knobs.treatment)?.label}</strong>
        <ul>
          <li>{TREATMENTS.find((treatment) => treatment.id === knobs.treatment)?.note}</li>
          <li>Only New project holds the +. Projects, a draft's setup page and a project keep it live — click around the rail to check.</li>
          <li>With an empty rail the dashed folder door is the same action, so it follows the +.</li>
          <li>Hover the held + for its title: <em>Already adding a project</em>.</li>
        </ul>
      </div>

      <div className="spec-frame">
        <div className="sync-app" style={{ ["--sync-sidebar-width" as string]: "300px" }}>
          <Sidebar treatment={knobs.treatment} screen={knobs.screen} rail={knobs.rail} onScreen={onScreen} />
          <Workspace screen={knobs.screen} rail={knobs.rail} onScreen={onScreen} />
        </div>
      </div>

      <h2 className="spec-h2">Side by side, on New project</h2>
      <div className="spec-compare">
        {(["some", "empty"] as const).flatMap((rail) => TREATMENTS.map((treatment) => (
          <figure key={`${rail}-${treatment.id}`}>
            <figcaption>
              {treatment.label} · {rail === "some" ? "rail with rows" : "empty rail"}
              <small>{treatment.note}</small>
            </figcaption>
            <div className="spec-mini">
              <div className="sync-app" style={{ ["--sync-sidebar-width" as string]: "300px" }}>
                <Sidebar treatment={treatment.id} screen="new" rail={rail} onScreen={() => undefined} />
              </div>
            </div>
          </figure>
        )))}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
