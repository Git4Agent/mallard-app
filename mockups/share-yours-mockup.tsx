import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon, { type IconName } from "../src/components/Icons";
import { NoticeZone, type NoticeItem, type NoticeZoneProps } from "../src/components/notice/Notice";
import ProjectPicker from "../src/components/project-sync/ProjectPicker";
import ProjectSidebar from "../src/components/project-sync/ProjectSidebar";
import CatalogIndexPage from "../src/components/storage-catalog/CatalogIndexPage";
import type { CatalogCapabilityDescriptor, CatalogProfile, CatalogScopeView } from "../src/components/storage-catalog/types";
import SyncPathBar, { PathBarBelow, type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import type { LocalProjectSummary, ProviderProfileSummary, StorageSummary } from "../src/types";
import "../src/App.css";

/**
 * Specimen: Share yours — where the link from the website's listing lands
 * (docs/PLAN_DEEP_LINK_SHARE_YOURS.md; the website's half is
 * mallardInternal …/web/share-yours-mockup.html).
 *
 * The listing's band hands the browser
 * `mallard://open?address=<the listing's address>`. There is nothing to ask
 * the service, so there is no sheet: Mallard comes forward on Projects, or
 * on the Catalog when one of the listing's Skills, Plugins or Pets chips was
 * on. The sheet appears only when the link cannot land, and the link waits
 * under the bar while the shell is in the middle of something, as a
 * repository link does.
 *
 * The Projects page, the Catalog and the sidebar are the shipped ones. The
 * page the link arrives over is a stand-in, and the sheet is the shipped
 * `AppLinkSheet`'s markup, frozen in the frame. New here: two destinations,
 * and the words *Sign in to Mallard Cloud to share.* and *Projects is
 * waiting.* `npm run dev`, then /mockups/share-yours-mockup.html
 * (`?scene=projects|catalog|welcome|no-account|other-host|waiting`,
 * `?theme=dark`).
 */

const HOME = "/Users/you";
const ORIGIN = "https://mallard-ai.com";
const noop = () => {};
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC((command) => (command === "plugin:path|resolve_directory" ? HOME : null));

const CLOUD: StorageSummary = {
  storage_id: "storage-personal",
  name: "Mallard Cloud",
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: "Rika321", is_path: false },
    { field_id: "service_origin", label: "Service", value: ORIGIN, is_path: false },
  ],
  settings: { service_origin: ORIGIN, account_id: "user_Rika321", account_login: "Rika321" },
  secrets: [{ secret_id: "account_token", present: true }],
};

const project = (name: string, rel: string, git: boolean, linked: boolean) => ({
  local_project_id: `project-${name}`,
  bundle_id: `bundle-${name}`,
  display_name: name,
  revision: 1,
  project_root: `${HOME}/${rel}`,
  canonical_project_root: `${HOME}/${rel}`,
  is_git_repository: git,
  profile_ids: { codex: "p-codex" },
  linked_storage_ids: linked ? [CLOUD.storage_id] : [],
}) as LocalProjectSummary;
const PROJECTS = [
  project("api-server", "code/api-server", true, true),
  project("physics 225", "Desktop/physics 225", false, true),
  project("dotfiles", "code/dotfiles", true, false),
];
const PROFILES: ProviderProfileSummary[] = [{
  profile_id: "p-codex", provider: "codex", display_name: "~/.codex", path: `${HOME}/.codex`, canonical_path: `${HOME}/.codex`,
  revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
}];

const capability = (resourceKind: string, format: string, version: number, label: string, singular: string, icon: string): CatalogCapabilityDescriptor => ({
  provider: "codex", resource_kind: resourceKind, format, format_version: version, provider_label: "Codex", label, singular_label: singular,
  presentation: { icon, kind_label: label, format_label: format }, capture: "directory_tree", apply: "automatic", targets: [],
});
const PETS = capability("desktop_pet", "openai.codex-pet", 2, "Pets v2", "Pet v2", "pet-v2");
const SKILLS = capability("agent_skill", "agentskills.io.skill", 1, "Skills", "Skill", "skill");
const CATALOG_PROFILE: CatalogProfile = {
  profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Default Codex", configured_home: "~/.codex", revision: 1, created_at: 0, updated_at: 0,
};
const scope = (id: string, descriptor: CatalogCapabilityDescriptor): CatalogScopeView => ({
  scope: { scope_id: id, profile_id: CATALOG_PROFILE.profile_id, provider: descriptor.provider, resource_kind: descriptor.resource_kind, format: descriptor.format, format_version: descriptor.format_version, revision: 1, created_at: 0, updated_at: 0 },
  profile: CATALOG_PROFILE, label: descriptor.label, available: true, provider_label: "Codex", kind_label: descriptor.label, format_label: descriptor.format, capability: descriptor,
});
const SCOPES = [scope("scope-pets", PETS), scope("scope-skills", SKILLS)];

/* ——— the scenes ——————————————————————————————————————————————————— */

type Landing = "projects" | "catalog";

interface Scene {
  id: string;
  n: string;
  label: string;
  group: "lands" | "stops";
  /** What the browser hands over. */
  link: string;
  /** What this computer holds when it arrives. */
  projects: LocalProjectSummary[];
  storages: StorageSummary[];
  before: "project" | "review" | "settings";
  /** A link that cannot land: the sheet's words, and its verb when there is one. */
  stop: { severity: "info" | "warning"; icon: IconName; message: string; action: string | null } | null;
  landing: Landing;
  shows: string;
  fresh: string;
}

const appLink = (page: string) => `mallard://open?address=${encodeURIComponent(page)}`;

const SCENES: Scene[] = [
  {
    id: "projects", n: "1", label: "Projects", group: "lands",
    link: appLink(`${ORIGIN}/repositories`), projects: PROJECTS, storages: [CLOUD], before: "project", stop: null, landing: "projects",
    shows: "No chip, or Projects, was on. Mallard comes forward on Projects: pick one, and Push is on its page. No sheet: there is nothing to look up and nothing to choose between.",
    fresh: "Nothing on the page. It is where the sidebar's Projects goes.",
  },
  {
    id: "catalog", n: "2", label: "A Catalog kind", group: "lands",
    link: appLink(`${ORIGIN}/repositories?type=desktop_pet`), projects: PROJECTS, storages: [CLOUD], before: "project", stop: null, landing: "catalog",
    shows: "Skills, Plugins or Pets was on: skills, plugins and pets are pushed from the Catalog, so it opens there. Pick the row, and Push is on its page.",
    fresh: "Nothing on the page. It is where the sidebar's Catalog goes.",
  },
  {
    id: "welcome", n: "3", label: "No project yet", group: "lands",
    link: appLink(`${ORIGIN}/repositories`), projects: [], storages: [CLOUD], before: "settings", stop: null, landing: "projects",
    shows: "Projects with nothing on it is its welcome: the two ways Add starts. The link adds nothing itself.",
    fresh: "Nothing. The shipped welcome.",
  },
  {
    id: "no-account", n: "4", label: "No account", group: "stops",
    link: appLink(`${ORIGIN}/repositories`), projects: PROJECTS, storages: [], before: "project",
    stop: { severity: "info", icon: "storage-mallard", message: "Sign in to Mallard Cloud to share.", action: "Set up Mallard Cloud" },
    landing: "projects",
    shows: "No storage signs in to the link's service, so nothing here can be shared on it yet. The shipped sheet says so; its verb opens the sign-in, and Finish setup lands on Projects. Decided on this computer: nothing is asked of the service.",
    fresh: "The words. The sheet and its verb are the repository link's.",
  },
  {
    id: "other-host", n: "5", label: "Another service", group: "stops",
    link: appLink("https://staging.mallard-ai.com/repositories"), projects: PROJECTS, storages: [CLOUD], before: "project",
    stop: { severity: "warning", icon: "alert-triangle", message: "No account here for staging.mallard-ai.com.", action: null },
    landing: "projects",
    shows: "A link never introduces a service: only a host a storage already signs in to counts. The same words a repository link gets.",
    fresh: "Nothing.",
  },
  {
    id: "waiting", n: "6", label: "A review is open", group: "stops",
    link: appLink(`${ORIGIN}/repositories`), projects: PROJECTS, storages: [CLOUD], before: "review", stop: null, landing: "projects",
    shows: "A link does not close a review or interrupt an operation or a dialog. It waits under the bar until you open it; Open is off while an operation runs.",
    fresh: "The line names the page: Projects, or Catalog.",
  },
];

/* ——— the app window ——————————————————————————————————————————————— */

type Stage = "before" | "arrival" | "setup" | "landed";

function zoneProps(items: NoticeItem[]): NoticeZoneProps {
  return { items, subject: "messages from Mallard", className: "sync-backend-banner", timing: "progress" };
}

const STORAGE_CRUMB: PathBarCrumb = { id: "storage", label: "Mallard Cloud", icon: "storage-mallard", tone: "root" };
const crumb = (id: string, label: string, icon?: IconName): PathBarCrumb => ({ id, label, icon });

/** A page the link arrives over: its bar is real, its body is not drawn. */
function StandIn({ zone, crumbs, title, children }: { zone: NoticeZoneProps; crumbs: PathBarCrumb[]; title: string; children: ReactNode }) {
  return (
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar crumbs={crumbs} ariaLabel={`${title} path`} />
      <main className="sync-main">
        <div className="spec-standin">
          <strong>{title}</strong>
          {children}
        </div>
      </main>
    </PathBarBelow.Provider>
  );
}

function Titlebar({ icon, title }: { icon: IconName; title: string }) {
  return (
    <header className="sync-titlebar">
      <div className="sync-titlebar-context">
        <Icon name={icon} size={16} />
        <strong>{title}</strong>
      </div>
    </header>
  );
}

/** The shipped `AppLinkSheet` for a link that cannot land, frozen in the frame. */
function StopSheet({ stop, onVerb, onClose }: { stop: NonNullable<Scene["stop"]>; onVerb: () => void; onClose: () => void }) {
  return (
    <div className="spec-scrim">
      <dialog open className="sync-modal sync-bundle-connection-dialog app-link-sheet spec-sheet" aria-label="Open from Mallard Cloud">
        <header className="sync-modal-header">
          <div><h1>Open from Mallard Cloud</h1></div>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close without opening the link"><Icon name="x" size={17} /></button>
        </header>
        <div className="sync-modal-body repo-connect">
          <div className="r2-step-list">
            <div className="storage-found-empty" role="status">
              <Icon name={stop.icon} size={24} />
              <strong>{stop.message}</strong>
            </div>
          </div>
        </div>
        <footer className="sync-modal-footer">
          <div>
            <button type="button" className="btn" onClick={onClose}>{stop.action ? "Cancel" : "Close"}</button>
            {stop.action && <button type="button" className="btn btn-primary" onClick={onVerb}>{stop.action}</button>}
          </div>
        </footer>
      </dialog>
    </div>
  );
}

function Window({ scene, run }: { scene: Scene; run: number }) {
  const [stage, setStage] = useState<Stage>("before");
  const playRef = useRef(0);
  const waits = scene.before === "review";
  const pageName = scene.landing === "catalog" ? "Catalog" : "Projects";

  useEffect(() => {
    const play = ++playRef.current;
    setStage("before");
    void (async () => {
      await wait(900);
      if (playRef.current !== play) return;
      setStage(scene.stop || waits ? "arrival" : "landed");
    })();
    return () => {
      playRef.current += 1;
    };
  }, [scene, run, waits]);

  const items: NoticeItem[] = stage === "arrival" && waits ? [{
    key: "app-link-waiting",
    severity: "info",
    title: "Link",
    message: `${pageName} is waiting.`,
    dismissalId: String(run),
    action: <button type="button" className="btn" onClick={() => setStage("landed")}>Open</button>,
    onDismiss: () => setStage("before"),
  }] : [];
  const zone = zoneProps(items);
  const landed = stage === "landed";
  const storages = landed && scene.storages.length === 0 ? [CLOUD] : scene.storages;

  let page: ReactNode;
  if (stage === "setup") {
    page = (
      <>
        <Titlebar icon="storage-mallard" title="New storage" />
        <NoticeZone {...zone} />
        <main className="sync-main">
          <div className="spec-standin">
            <strong>Mallard Cloud sign-in</strong>
            The shipped lane: Sign in, Account, Access, Review. A stand-in.
            <div><button type="button" className="btn btn-primary" onClick={() => setStage("landed")}>Finish setup</button></div>
          </div>
        </main>
      </>
    );
  } else if (landed && scene.landing === "catalog") {
    page = (
      <PathBarBelow.Provider value={zone}>
        <CatalogIndexPage scopes={SCOPES} capabilities={[PETS, SKILLS]} busy={false} appActions={[]} onOpenScope={noop} onAddScope={noop} />
      </PathBarBelow.Provider>
    );
  } else if (landed) {
    page = (
      <>
        <Titlebar icon="folder" title="Projects" />
        <NoticeZone {...zone} />
        <main className="sync-main sync-project-links-page sync-landing-page">
          <ProjectPicker projects={scene.projects} profiles={PROFILES} storages={storages} rememberedStorageByProject={{}} onAddProject={noop} onAddCloudProject={noop} onSelectProject={noop} />
        </main>
      </>
    );
  } else if (scene.before === "review") {
    page = (
      <StandIn zone={zone} crumbs={[STORAGE_CRUMB, crumb("project", "physics 225", "folder"), crumb("push", "Push", "upload")]} title="Push review">
        Three sessions ticked, not pushed yet. A stand-in for the shipped review.
      </StandIn>
    );
  } else if (scene.before === "settings") {
    page = (
      <>
        <Titlebar icon="settings" title="Settings" />
        <NoticeZone {...zone} />
        <main className="sync-main">
          <div className="spec-standin"><strong>Settings</strong>Where you were when the link arrived. A stand-in.</div>
        </main>
      </>
    );
  } else {
    page = (
      <StandIn zone={zone} crumbs={[STORAGE_CRUMB, crumb("project", "physics 225", "folder")]} title="physics 225">
        Where you were when the link arrived. A stand-in for the project’s Activity.
      </StandIn>
    );
  }

  return (
    <div className="spec-frame">
      <div className="sync-app" style={{ "--sync-sidebar-width": "248px" } as CSSProperties}>
        <ProjectSidebar
          projects={scene.projects} drafts={[]} activeDraftId={null} storages={storages} storageDrafts={[]} storageProviders={[]}
          storageUsage={Object.fromEntries(storages.map((storage) => [storage.storage_id, 2]))}
          activeProjectId={!landed && stage !== "setup" && scene.before !== "settings" ? "project-physics 225" : null}
          activeStorageId={null} catalogScopes={SCOPES} catalogCapabilities={[PETS, SKILLS]} activeCatalogScopeId={null}
          addingProject={false} addingCatalogScope={false} addingStorage={stage === "setup"}
          catalogIndexOpen={landed && scene.landing === "catalog"} storageIndexOpen={false} settingsOpen={!landed && scene.before === "settings"}
          loading={false} busy={false} activityOpen={false} unreadLogs={0}
          onOpenProjects={noop} onSelectProject={noop} onRemoveProject={noop} onSelectDraft={noop} onDiscardDraft={noop}
          onToggleActivity={noop} onAddProject={noop} onOpenStorage={noop} onOpenStorageDraft={noop} onDiscardStorageDraft={noop}
          onRemoveStorage={noop} onAddStorage={noop} onOpenCatalogIndex={noop} onOpenCatalogScope={noop} onAddCatalogScope={noop}
          onRemoveCatalogScope={noop} onOpenStorageIndex={noop} onOpenSettings={noop}
        />
        <div className="sync-workspace">{page}</div>
      </div>
      {stage === "arrival" && scene.stop && (
        <StopSheet stop={scene.stop} onVerb={() => setStage("setup")} onClose={() => setStage("before")} />
      )}
    </div>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

interface Knobs { scene: string; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const scene = query.get("scene") ?? "projects";
  return {
    scene: SCENES.some((candidate) => candidate.id === scene) ? scene : "projects",
    theme: query.get("theme") === "dark" ? "dark" : "light",
  };
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const [run, setRun] = useState(0);
  const set = (patch: Partial<Knobs>) => setKnobs((current) => ({ ...current, ...patch }));
  const scene = SCENES.find((candidate) => candidate.id === knobs.scene)!;

  useEffect(() => {
    document.documentElement.dataset.theme = knobs.theme;
    window.history.replaceState(null, "", `${window.location.pathname}?${new URLSearchParams({ ...knobs })}`);
  }, [knobs]);

  const sceneButton = (candidate: Scene) => (
    <button key={candidate.id} type="button" className="spec-seg" aria-pressed={knobs.scene === candidate.id} onClick={() => set({ scene: candidate.id })}>
      <b>{candidate.n}</b>{candidate.label}
    </button>
  );

  return (
    <div className="spec">
      <h1 className="spec-title">Share yours<small>where the link from the website’s listing lands</small></h1>

      <div className="spec-controls">
        <div className="spec-group">
          <span>Lands</span>
          {SCENES.filter((candidate) => candidate.group === "lands").map(sceneButton)}
        </div>
        <div className="spec-group">
          <span>Stops or waits</span>
          {SCENES.filter((candidate) => candidate.group === "stops").map(sceneButton)}
        </div>
        <div className="spec-group">
          <span>Theme</span>
          {(["light", "dark"] as const).map((theme) => (
            <button key={theme} type="button" className="spec-seg" aria-pressed={knobs.theme === theme} onClick={() => set({ theme })}>{theme === "light" ? "Light" : "Dark"}</button>
          ))}
        </div>
      </div>

      <div className="spec-card">
        <p><b>{scene.n} · {scene.label}.</b> {scene.shows}</p>
        <p><span>New</span>{scene.fresh}</p>
      </div>
      <div className="spec-browser">
        <span>The browser hands over</span>
        <code>{scene.link}</code>
        <button type="button" className="btn" onClick={() => setRun((current) => current + 1)}><Icon name="refresh" size={13} /> Play again</button>
      </div>
      <Window key={scene.id} scene={scene} run={run} />
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<Specimen />);
