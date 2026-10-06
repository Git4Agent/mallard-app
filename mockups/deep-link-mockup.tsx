import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon, { type IconName } from "../src/components/Icons";
import Notice, { NoticeZone, type NoticeItem, type NoticeZoneProps } from "../src/components/notice/Notice";
import NewProjectFolderStep from "../src/components/project-sync/NewProjectFolderStep";
import ProjectSidebar from "../src/components/project-sync/ProjectSidebar";
import { projectSyncApi } from "../src/components/project-sync/api";
import AddCatalogScopeWorkspace from "../src/components/storage-catalog/AddCatalogScopeWorkspace";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import type {
  CatalogCapabilityDescriptor,
  CatalogCloudRepository,
  CatalogProfile,
  CatalogScopeView,
} from "../src/components/storage-catalog/types";
import SyncPathBar, { PathBarBelow, type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import type {
  LocalProjectSummary,
  ManagedCloudRepository,
  ManagedCloudRepositoryResolution,
  ProviderDescriptor,
  ProviderProfileSummary,
  StorageSummary,
} from "../src/types";
import "../src/App.css";

/**
 * Specimen: Open in Mallard — where a `mallard://` link from the website
 * lands in the desktop.
 *
 * The website's repository page already hands the browser
 * `mallard://open?address=<the page's address>` (mallardInternal,
 * web/src/pull.ts); no desktop answers it yet. This draws what answering it
 * looks like:
 *
 * - The map: which page of the website lands on which page of the app.
 * - Ten arrivals, each in the app window at its default size, played from
 *   the link: four landings, and the six ways a link stops or waits.
 * - Three ways to arrive: A · straight to the page (recommended), B · a
 *   sheet that asks first, C · a line under the bar that asks first.
 *
 * The landing pages are the shipped ones: the real New project and Add
 * Catalog resource pages on a stand-in backend, filled the way a paste and
 * Look up fills them. Pages the link only opens (a project, a Catalog item,
 * a review) are stand-ins. New here: the lines under the bar, B's sheet, and
 * one hint (dotted). `npm run dev`, then /mockups/deep-link-mockup.html
 * (`?scene=map|project-new|project-here|item-new|item-here|no-account|accounts|unseen|other-host|bad-link|held`,
 * `?dir=a|b|c`, `?theme=dark`).
 */

/* ——— data ————————————————————————————————————————————————————————— */

const HOME = "/Users/you";
const HOST = "mallard-ai.com";
const ORIGIN = `https://${HOST}`;
const now = Math.floor(Date.now() / 1000);
const DAY = 86_400;
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));
const noop = () => {};

const cloud = (id: string, name: string, login: string): StorageSummary => ({
  storage_id: id,
  name,
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: login, is_path: false },
    { field_id: "service_origin", label: "Service", value: ORIGIN, is_path: false },
  ],
  settings: { service_origin: ORIGIN, account_id: `user_${login}`, account_login: login },
  secrets: [{ secret_id: "account_token", present: true }],
});
const PERSONAL = cloud("storage-personal", "Mallard Cloud", "Rika321");
const WORK = cloud("storage-work", "Mallard Cloud work", "rika-acme");

const project = (name: string, rel: string, git: boolean) => ({
  local_project_id: `project-${name}`,
  bundle_id: `bundle-${name}`,
  display_name: name,
  revision: 1,
  project_root: `${HOME}/${rel}`,
  canonical_project_root: `${HOME}/${rel}`,
  is_git_repository: git,
  profile_ids: { codex: "p-codex" },
}) as LocalProjectSummary;
const API_SERVER = project("api-server", "code/api-server", true);
const PHYSICS = project("physics 225", "Desktop/physics 225", false);
const PROJECTS = [API_SERVER, PHYSICS];

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, has_cli: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const agentProfile = (profileId: string, provider: string, dir: string): ProviderProfileSummary => ({
  profile_id: profileId, provider, display_name: `~/${dir}`, path: `${HOME}/${dir}`, canonical_path: `${HOME}/${dir}`,
  revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
});
const PROFILES = [agentProfile("p-codex", "codex", ".codex"), agentProfile("p-claude", "claude", ".claude")];

const PET_COORDINATE = { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 };
const PETS: CatalogCapabilityDescriptor = {
  ...PET_COORDINATE, provider_label: "Codex", label: "Pets v2", singular_label: "Pet v2",
  presentation: { icon: "pet-v2", kind_label: "Pets v2", format_label: "Codex pet v2" }, capture: "directory_tree", apply: "automatic", targets: [],
};
const SKILLS: CatalogCapabilityDescriptor = {
  provider: "codex", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1, provider_label: "Codex", label: "Skills", singular_label: "Skill",
  presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" }, capture: "directory_tree", apply: "automatic", targets: [],
};
const CATALOG_PROFILE: CatalogProfile = {
  profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Default Codex", configured_home: "~/.codex", revision: 1, created_at: 0, updated_at: 0,
};
const scopeView = (id: string, capability: CatalogCapabilityDescriptor): CatalogScopeView => ({
  scope: { scope_id: id, profile_id: CATALOG_PROFILE.profile_id, provider: capability.provider, resource_kind: capability.resource_kind, format: capability.format, format_version: capability.format_version, revision: 1, created_at: 0, updated_at: 0 },
  profile: CATALOG_PROFILE, label: capability.label, available: true, provider_label: "Codex", kind_label: capability.label, format_label: capability.format, capability,
});
const PETS_SCOPE = "scope-pets-v2";
const SCOPES = [scopeView(PETS_SCOPE, PETS), scopeView("scope-skills", SKILLS)];

/* ——— the stand-in backend the shipped pages call ————————————————————— */

const runtime = window as unknown as { isTauri: boolean };
runtime.isTauri = true;
mockIPC((command) => (command === "plugin:path|resolve_directory" ? HOME : null));

const cloudProject = (overrides: Partial<ManagedCloudRepository>): ManagedCloudRepository => ({
  repository_id: "repo_maya_portal", owner_login: "maya", name: "portal", address: "maya/portal",
  website_url: `${ORIGIN}/maya/portal`, bundle_id: "b".repeat(32), repository_class: "git_project", visibility: "public",
  role: null, can_push: true, repository_fingerprint: "f".repeat(64), git_remote: "differs",
  github_url: "https://github.com/maya/portal", revision: 6, updated_at: now - 9 * DAY, resources: 14, registration_only: false,
  description: "Customer portal, built with Codex.", star_count: 7, viewer_starred: true,
  ...overrides,
});
const MAYA_PORTAL = cloudProject({});

/** `owner/name` out of whatever the field was handed. */
const addressOf = (text: string) => text.trim().replace(/^https?:\/\/[^/]+\//i, "").replace(/\/+$/, "").toLowerCase();

projectSyncApi.listAgentFolders = async () => ({ folders: [] });
projectSyncApi.listManagedCloudRepositories = async () => {
  await wait(250);
  return [];
};
projectSyncApi.listManagedCloudStarred = async () => ({ rows: [], next_cursor: null });
projectSyncApi.listManagedCloudPublic = async () => ({ rows: [], next_cursor: null });
projectSyncApi.resolveManagedCloudRepository = async (_storageId, address): Promise<ManagedCloudRepositoryResolution> => {
  await wait(350);
  const wanted = addressOf(address);
  return wanted === MAYA_PORTAL.address ? { kind: "public", repository: MAYA_PORTAL } : { kind: "not_found", address: wanted };
};
projectSyncApi.probeCloudProjectFolder = async () => ({ folder: { kind: "new" }, can_clone: true });

const MOCHI: CatalogCloudRepository = {
  ...PET_COORDINATE, provider_label: "Codex", kind_label: "Pets v2", format_label: "Codex pet v2",
  repository_id: "repo_maya_mochi", owner_login: "maya", name: "Mochi", address: "maya/Mochi",
  description: "A round rice cake that wobbles when you type.", star_count: 12, viewer_starred: true,
  website_url: `${ORIGIN}/maya/Mochi`, visibility: "public", role: null, can_push: false, revision: 5, updated_at: now - 6 * DAY,
  registration_only: false, resource_id: "mochi", display_name: "Mochi", available: true,
};
storageCatalogApi.listProfiles = async () => [CATALOG_PROFILE];
storageCatalogApi.listCapabilities = async () => [PETS, SKILLS];
storageCatalogApi.listCloudShared = async () => {
  await wait(250);
  return [];
};
storageCatalogApi.listCloudStarred = async () => ({ rows: [], next_cursor: null });
storageCatalogApi.listCloudPublic = async () => ({ rows: [], next_cursor: null });
storageCatalogApi.resolveCloudRepository = async (_storageId, address) => {
  await wait(350);
  const wanted = addressOf(address);
  return wanted === MOCHI.address.toLowerCase() ? { kind: "found", repository: MOCHI } : { kind: "not_found", address: wanted };
};

/* ——— the scenes ——————————————————————————————————————————————————— */

type Dir = "a" | "b" | "c";
type Landing = "new-project" | "new-project-accounts" | "project" | "add-catalog" | "catalog-item" | "none";

/** What the service answered, as B's sheet and C's line say it. */
interface Found {
  mark: IconName;
  /** The found row's second line, as the shipped pickers word it. */
  verdict: string;
  /** Where it stands on this computer. */
  here: string;
  /** C's line, after the address. */
  line: string;
  /** The verb that goes on. */
  verb: string;
}

/** A link that stops: the words, and the one way forward when there is one. */
interface Stop {
  severity: "info" | "warning";
  message: string;
  action?: string;
  /** Decided on this computer, before any request. */
  local: boolean;
}

interface Scene {
  id: string;
  n: string;
  label: string;
  group: "lands" | "stops";
  /** What the browser hands over. */
  link: string;
  subject: string;
  storages: StorageSummary[];
  /** The page the link arrives over. */
  before: "project" | "review";
  found: Found | null;
  stop: Stop | null;
  landing: Landing;
  shows: string;
  /** What this scene adds to the shipped app. */
  fresh: string;
}

const appLink = (page: string) => `mallard://open?address=${encodeURIComponent(page)}`;

const SCENES: Scene[] = [
  {
    id: "project-new", n: "1", label: "Project, not here", group: "lands",
    link: appLink(`${ORIGIN}/maya/portal`), subject: "maya/portal", storages: [PERSONAL], before: "project",
    found: { mark: "git-folder", verdict: "Public · anyone signed in can push", here: "Not on this computer.", line: "a Git project, not on this computer.", verb: "Set up project…" },
    stop: null, landing: "new-project",
    shows: "New project › From Mallard Cloud, with the account ticked and the address looked up. The folder and the verb are still yours: nothing is cloned or connected until you press it.",
    fresh: "Nothing on the page. It is the shipped page after a paste and Look up.",
  },
  {
    id: "project-here", n: "2", label: "Project, already here", group: "lands",
    link: appLink(`${ORIGIN}/Rika321/api-server`), subject: "Rika321/api-server", storages: [PERSONAL], before: "project",
    found: { mark: "git-folder", verdict: "Yours · Owner", here: "On this computer: api-server", line: "connected to api-server.", verb: "Open project" },
    stop: null, landing: "project",
    shows: "The project that repository is connected to, with that storage in the bar. Nothing else happens: no Pull review opens.",
    fresh: "Nothing. The project's page, as the sidebar opens it.",
  },
  {
    id: "item-new", n: "3", label: "Catalog item, not here", group: "lands",
    link: appLink(`${ORIGIN}/maya/Mochi`), subject: "maya/Mochi", storages: [PERSONAL], before: "project",
    found: { mark: "catalog-pet-v2", verdict: "Public · Contributor · v5", here: "Not in your Catalog.", line: "a pet v2, not in your Catalog.", verb: "Add to Catalog…" },
    stop: null, landing: "add-catalog",
    shows: "Add Catalog resource › From Mallard Cloud, with the item found and its row chosen. Add is still yours; nothing is pulled.",
    fresh: "Nothing on the page. It is the shipped page after a paste and Look up.",
  },
  {
    id: "item-here", n: "4", label: "Catalog item, already here", group: "lands",
    link: appLink(`${ORIGIN}/Rika321/Duck`), subject: "Rika321/Duck", storages: [PERSONAL], before: "project",
    found: { mark: "catalog-pet-v2", verdict: "Yours · Owner · v3", here: "In your Catalog: Pets v2 · Default Codex", line: "in Pets v2 · Default Codex.", verb: "Open in Catalog" },
    stop: null, landing: "catalog-item",
    shows: "The item's page in the Catalog row that holds it.",
    fresh: "Nothing. The page Add from Mallard Cloud already lands on.",
  },
  {
    id: "no-account", n: "5", label: "No account", group: "stops",
    link: appLink(`${ORIGIN}/maya/portal`), subject: "maya/portal", storages: [], before: "project",
    found: null,
    stop: { severity: "info", message: "Sign in to Mallard Cloud to open maya/portal.", action: "Set up Mallard Cloud", local: true },
    landing: "new-project",
    shows: "No storage signs in to the link's service, so nothing can be looked up. The link waits under the bar; Finish setup on the shipped sign-in lane picks it up and lands as scene 1.",
    fresh: "The line under the bar, and the hop back from Finish setup (New project already has one).",
  },
  {
    id: "accounts", n: "6", label: "Two accounts", group: "lands",
    link: appLink(`${ORIGIN}/maya/portal`), subject: "maya/portal", storages: [PERSONAL, WORK], before: "project",
    found: { mark: "git-folder", verdict: "Public · anyone signed in can push", here: "Not on this computer.", line: "a Git project, not on this computer.", verb: "Set up project…" },
    stop: null, landing: "new-project-accounts",
    shows: "Both accounts can see it, so neither is ticked: the page's own rule for several accounts. Pick one and its lookup runs. (When only one account can see it, that one is ticked, as in scene 1.)",
    fresh: "The hint names the repository (dotted). Asking each account on the link's service is new; it is the open decision in the plan.",
  },
  {
    id: "unseen", n: "7", label: "Can't see it", group: "stops",
    link: appLink(`${ORIGIN}/acme/billing`), subject: "acme/billing", storages: [PERSONAL], before: "project",
    found: null,
    stop: { severity: "warning", message: "Rika321 can’t see acme/billing.", local: false },
    landing: "none",
    shows: "Private and not shared with this account, removed, or mistyped: the service answers the same for all three. The app stays where it was.",
    fresh: "The line under the bar. The words are the picker's.",
  },
  {
    id: "other-host", n: "8", label: "Another service", group: "stops",
    link: appLink("https://staging.mallard-ai.com/maya/portal"), subject: "maya/portal", storages: [PERSONAL], before: "project",
    found: null,
    stop: { severity: "warning", message: "No account here for staging.mallard-ai.com.", local: true },
    landing: "none",
    shows: "A link never introduces a service. Only a host a storage already signs in to is asked; a release build knows mallard-ai.com alone. No request leaves the computer.",
    fresh: "The line under the bar.",
  },
  {
    id: "bad-link", n: "9", label: "Not a repository", group: "stops",
    link: "mallard://pull?address=https%3A%2F%2Fmallard-ai.com%2Fmaya%2Fportal&now=1", subject: "", storages: [PERSONAL], before: "project",
    found: null,
    stop: { severity: "warning", message: "Mallard can’t open that link.", local: true },
    landing: "none",
    shows: "The only link is open with one address, and the address must be a repository's page. Anything else — another verb, more parameters, a profile or docs page — is refused in the same words, and none of its text is shown.",
    fresh: "The line under the bar.",
  },
  {
    id: "held", n: "10", label: "A review is open", group: "stops",
    link: appLink(`${ORIGIN}/maya/portal`), subject: "maya/portal", storages: [PERSONAL], before: "review",
    found: { mark: "git-folder", verdict: "Public · anyone signed in can push", here: "Not on this computer.", line: "a Git project, not on this computer.", verb: "Set up project…" },
    stop: null, landing: "new-project",
    shows: "A link is not allowed to close a review or interrupt a Push, Pull or clone. It waits under the bar until you open it. One link waits at a time; a newer one replaces it.",
    fresh: "The line under the bar, with Open.",
  },
];

const DIRECTIONS: Array<{ id: Dir; label: string; note: string }> = [
  { id: "a", label: "A · Straight to the page", note: "The link is a paste done for you: it opens the page and stops there. It waits only when the app is in the middle of something." },
  { id: "b", label: "B · Sheet first", note: "Every link opens one sheet that says what it found and offers one verb. Nothing moves until you press it." },
  { id: "c", label: "C · Line first", note: "Every link becomes a line under the bar with one verb. The page never changes by itself." },
];

/* ——— the app window ——————————————————————————————————————————————— */

type Stage = "before" | "opening" | "arrival" | "setup" | "landed";

interface SidebarState {
  storages: StorageSummary[];
  activeProjectId: string | null;
  activeCatalogScopeId: string | null;
  addingProject: boolean;
  addingCatalogScope: boolean;
  addingStorage: boolean;
}

function Sidebar({ storages, activeProjectId, activeCatalogScopeId, addingProject, addingCatalogScope, addingStorage }: SidebarState) {
  return (
    <ProjectSidebar
      projects={PROJECTS} drafts={[]} activeDraftId={null} storages={storages} storageDrafts={[]} storageProviders={[]}
      storageUsage={Object.fromEntries(storages.map((storage) => [storage.storage_id, storage === PERSONAL ? 1 : 0]))}
      activeProjectId={activeProjectId} activeStorageId={null} catalogScopes={SCOPES} catalogCapabilities={[PETS, SKILLS]}
      activeCatalogScopeId={activeCatalogScopeId} addingProject={addingProject} addingCatalogScope={addingCatalogScope} addingStorage={addingStorage}
      catalogIndexOpen={false} storageIndexOpen={false} settingsOpen={false} loading={false} busy={false} activityOpen={false} unreadLogs={0}
      onOpenProjects={noop} onSelectProject={noop} onRemoveProject={noop} onSelectDraft={noop} onDiscardDraft={noop}
      onToggleActivity={noop} onAddProject={noop} onOpenStorage={noop} onOpenStorageDraft={noop} onDiscardStorageDraft={noop}
      onRemoveStorage={noop} onAddStorage={noop} onOpenCatalogIndex={noop} onOpenCatalogScope={noop} onAddCatalogScope={noop}
      onRemoveCatalogScope={noop} onOpenStorageIndex={noop} onOpenSettings={noop}
    />
  );
}

/** The shell's zone under the bar, as the app places it. */
function zoneProps(items: NoticeItem[]): NoticeZoneProps {
  return { items, subject: "messages from Mallard", className: "sync-backend-banner", timing: "progress" };
}

const STORAGE_CRUMB: PathBarCrumb = { id: "storage", label: "Mallard Cloud", icon: "storage-mallard", tone: "root" };
const crumb = (id: string, label: string, icon?: IconName): PathBarCrumb => ({ id, label, icon });

/** A page the link only opens: its bar is real, its body is not drawn. */
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

function typeInto(input: HTMLInputElement, text: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/**
 * Does to the shipped page what the link does: opens From Mallard Cloud,
 * puts the address in the field, and looks it up — the same three steps a
 * person takes. `hintFor` rewords the several-accounts hint, the one new
 * string on the page.
 */
function Driven({ address, door, hintFor, children }: { address: string; door?: boolean; hintFor?: string | null; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let live = true;
    let filled: HTMLInputElement | null = null;
    void (async () => {
      while (live) {
        const root = ref.current;
        if (root) {
          if (door) {
            const cloudDoor = [...root.querySelectorAll<HTMLButtonElement>(".storage-mode-door")].find((candidate) => candidate.textContent?.includes("From Mallard Cloud"));
            if (cloudDoor && cloudDoor.getAttribute("aria-checked") !== "true") cloudDoor.click();
          }
          if (hintFor) {
            const hint = [...root.querySelectorAll<HTMLElement>("small.sync-setup-hint")].find((candidate) => candidate.textContent === "Choose an account first.");
            if (hint) {
              hint.textContent = `Choose an account for ${hintFor}.`;
              hint.classList.add("spec-new");
            }
          }
          const input = root.querySelector<HTMLInputElement>(".cloud-project-repository input");
          if (input && input !== filled && !input.value && !input.disabled) {
            filled = input;
            typeInto(input, address);
            await wait(300);
            if (!live) return;
            root.querySelector<HTMLButtonElement>(".cloud-project-repository .repo-verb")?.click();
          }
        }
        await wait(120);
      }
    })();
    return () => {
      live = false;
    };
  }, [address, door, hintFor]);
  return <div ref={ref} style={{ display: "contents" }}>{children}</div>;
}

function NewProjectPage({ zone, storages, subject, several }: { zone: NoticeZoneProps; storages: StorageSummary[]; subject: string; several: boolean }) {
  return (
    <>
      <Titlebar icon="folder" title="New project" />
      <NoticeZone {...zone} />
      <main className="sync-main sync-project-links-page sync-project-setup-page">
        <section className="profile-links-section sync-project-overview" aria-label="Project setup">
          <Driven address={subject} hintFor={several ? subject : null}>
            <NewProjectFolderStep
              workflowEpoch={1}
              isWorkflowCurrent={() => true}
              projects={PROJECTS}
              providers={PROVIDERS}
              profiles={PROFILES}
              storages={storages}
              busy={false}
              initialSource="cloud"
              initialStorageId={several ? null : storages[0]?.storage_id ?? null}
              onCancel={noop}
              onChoose={noop}
              onOpenProject={noop}
              onAddCloudAccount={noop}
              onCloudProjectDone={noop}
            />
          </Driven>
        </section>
      </main>
    </>
  );
}

function AddCatalogPage({ zone, subject }: { zone: NoticeZoneProps; subject: string }) {
  return (
    <>
      <Titlebar icon="catalog" title="Add Catalog resource" />
      <NoticeZone {...zone} />
      <Driven address={subject} door>
        <AddCatalogScopeWorkspace
          workflowEpoch={1}
          isWorkflowCurrent={() => true}
          busy={false}
          initialChoiceKey={null}
          scopes={SCOPES}
          storages={[]}
          cloudAccounts={[{ id: PERSONAL.storage_id, name: PERSONAL.name, account_login: "Rika321" }]}
          onSetUpCloud={noop}
          onClose={noop}
          onCreated={noop}
          onAdded={noop}
        />
      </Driven>
    </>
  );
}

/** B's sheet: the connect dialog's frame around the picker's found row. */
function Sheet({ scene, looking, onVerb, onClose }: { scene: Scene; looking: boolean; onVerb: () => void; onClose: () => void }) {
  const { found, stop } = scene;
  const verb = looking ? null : found?.verb ?? stop?.action ?? null;
  return (
    <div className="spec-scrim">
      <dialog open className="sync-modal sync-bundle-connection-dialog spec-sheet" aria-label="Open from Mallard Cloud">
        <header className="sync-modal-header">
          <div><h1>Open from Mallard Cloud</h1></div>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close"><Icon name="x" size={17} /></button>
        </header>
        <div className="sync-modal-body repo-connect">
          {looking ? (
            <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Looking up {scene.subject}…</small>
          ) : found ? (
            <>
              <div className="r2-step-list">
                <div className="storage-found-list" aria-label="Repository at this address">
                  <div className="storage-found-row with-verb">
                    <span className="sync-storage-repository-icon"><Icon name={found.mark} size={14} /></span>
                    <span className="storage-found-text">
                      <span className="storage-found-name">{scene.subject}</span>
                      <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {found.verdict}</span>
                    </span>
                  </div>
                </div>
              </div>
              <small className="repo-answer-line" role="status"><Icon name="computer" size={13} /> {found.here}</small>
            </>
          ) : stop ? (
            <Notice severity={stop.severity} timing="inline">{stop.message}</Notice>
          ) : null}
        </div>
        <footer className="sync-modal-footer">
          <span />
          <div>
            <button type="button" className="btn" onClick={onClose}>{verb ? "Cancel" : "Close"}</button>
            {verb && <button type="button" className="btn btn-primary" onClick={onVerb}>{verb}</button>}
          </div>
        </footer>
      </dialog>
    </div>
  );
}

/** One app window, played from the link. */
function Window({ scene, dir, run }: { scene: Scene; dir: Dir; run: number }) {
  const [stage, setStage] = useState<Stage>("before");
  const playRef = useRef(0);
  const held = dir === "a" && scene.before === "review";

  const land = async () => {
    const play = playRef.current;
    setStage("opening");
    await wait(900);
    if (playRef.current === play) setStage("landed");
  };

  useEffect(() => {
    const play = ++playRef.current;
    const live = () => playRef.current === play;
    setStage("before");
    void (async () => {
      await wait(700);
      if (!live()) return;
      if (scene.stop?.local || held) {
        setStage("arrival");
        return;
      }
      setStage("opening");
      await wait(900);
      if (!live()) return;
      setStage(scene.stop || dir !== "a" ? "arrival" : "landed");
    })();
    return () => {
      playRef.current += 1;
    };
  }, [scene, dir, run, held]);

  /* The verb on B's sheet, C's line, or a waiting link. */
  const goOn = () => {
    if (scene.stop?.action) setStage("setup");
    else if (held) void land();
    else setStage("landed");
  };
  const dismiss = () => setStage("before");

  const items: NoticeItem[] = [];
  const verbButton = (label: string) => <button type="button" className="btn" onClick={goOn}>{label}</button>;
  if (dir !== "b") {
    if (stage === "opening") {
      items.push({ key: "link-opening", severity: "info", title: "Link", message: `Opening ${scene.subject}…`, dismissalId: String(run), onDismiss: dismiss });
    } else if (stage === "arrival" && scene.stop) {
      items.push({
        key: "link-stopped", severity: scene.stop.severity, title: "Link", message: scene.stop.message,
        action: scene.stop.action ? verbButton(scene.stop.action) : undefined, dismissalId: String(run), onDismiss: dismiss,
      });
    } else if (stage === "arrival" && held) {
      items.push({
        key: "link-waiting", severity: "info", title: "Link", message: `${scene.subject}. Opening it closes this review.`,
        action: verbButton("Open"), dismissalId: String(run), onDismiss: dismiss,
      });
    } else if (stage === "arrival" && scene.found) {
      items.push({
        key: "link-found", severity: "info", title: "Link", message: `${scene.subject}, ${scene.found.line}`,
        action: verbButton(scene.found.verb), dismissalId: String(run), onDismiss: dismiss,
      });
    }
  }
  const zone = zoneProps(items);

  const over = stage === "before" || stage === "opening" || stage === "arrival";
  const landing = stage === "landed" ? scene.landing : null;
  const storages = stage === "landed" && scene.storages.length === 0 ? [PERSONAL] : scene.storages;
  const sidebar: SidebarState = {
    storages,
    activeProjectId: over ? PHYSICS.local_project_id : landing === "project" ? API_SERVER.local_project_id : null,
    activeCatalogScopeId: landing === "catalog-item" ? PETS_SCOPE : null,
    addingProject: landing === "new-project" || landing === "new-project-accounts",
    addingCatalogScope: landing === "add-catalog",
    addingStorage: stage === "setup",
  };

  let page: ReactNode;
  if (over && scene.before === "review") {
    page = (
      <StandIn zone={zone} crumbs={[STORAGE_CRUMB, crumb("project", PHYSICS.display_name, "folder"), crumb("push", "Push", "upload")]} title="Push review">
        Three sessions ticked, not pushed yet. A stand-in for the shipped review.
      </StandIn>
    );
  } else if (over) {
    page = (
      <StandIn zone={zone} crumbs={[STORAGE_CRUMB, crumb("project", PHYSICS.display_name, "folder")]} title="physics 225">
        Where you were when the link arrived. A stand-in for the project’s Activity.
      </StandIn>
    );
  } else if (stage === "setup") {
    page = (
      <>
        <Titlebar icon="storage-mallard" title="New storage" />
        <NoticeZone {...zone} />
        <main className="sync-main">
          <div className="spec-standin">
            <strong>Mallard Cloud sign-in</strong>
            The shipped lane: Sign in, Account, Access, Review. A stand-in.
            <div><button type="button" className="btn btn-primary" onClick={() => void land()}>Finish setup</button></div>
          </div>
        </main>
      </>
    );
  } else if (landing === "new-project" || landing === "new-project-accounts") {
    page = <NewProjectPage zone={zone} storages={storages} subject={scene.subject} several={landing === "new-project-accounts"} />;
  } else if (landing === "add-catalog") {
    page = <AddCatalogPage zone={zone} subject={scene.subject} />;
  } else if (landing === "catalog-item") {
    page = (
      <StandIn zone={zone} crumbs={[STORAGE_CRUMB, crumb("row", "Pets v2 · Default Codex", "catalog-pet-v2"), crumb("item", "Duck")]} title="Duck">
        The item’s page in its Catalog row. A stand-in for the shipped page.
      </StandIn>
    );
  } else {
    page = (
      <StandIn zone={zone} crumbs={[STORAGE_CRUMB, crumb("project", API_SERVER.display_name, "git-folder")]} title="api-server">
        The project’s Activity, with Mallard Cloud in the bar. A stand-in for the shipped page.
      </StandIn>
    );
  }

  const sheet = dir === "b" && (stage === "opening" || stage === "arrival");
  return (
    <div className="spec-frame">
      <div className="sync-app" style={{ "--sync-sidebar-width": "248px" } as CSSProperties}>
        <Sidebar {...sidebar} />
        <div className="sync-workspace">{page}</div>
      </div>
      {sheet && <Sheet scene={scene} looking={stage === "opening"} onVerb={goOn} onClose={dismiss} />}
    </div>
  );
}

/* ——— the map ——————————————————————————————————————————————————————— */

interface MapRow {
  page: ReactNode;
  here: ReactNode;
  lands: ReactNode;
  yours: ReactNode;
  scene?: string;
  later?: boolean;
}

const MAP: MapRow[] = [
  { page: <><b>A project</b><small>/owner/name</small></>, here: "Not on this computer", lands: <><b>New project › From Mallard Cloud</b><small>account ticked, address looked up</small></>, yours: "Folder, agents, then Clone and connect", scene: "project-new" },
  { page: null, here: "Connected to a project", lands: <><b>That project</b><small>that storage in the bar</small></>, yours: "Nothing", scene: "project-here" },
  { page: <><b>A skill, plugin or pet</b><small>/owner/name</small></>, here: "Not in the Catalog", lands: <><b>Add Catalog resource › From Mallard Cloud</b><small>item found, its row chosen</small></>, yours: "Add", scene: "item-new" },
  { page: null, here: "In a Catalog row", lands: <><b>The item’s page</b><small>in the row that holds it</small></>, yours: "Nothing", scene: "item-here" },
  { page: <><b>A session</b><small>/owner/name/sessions/id · layer 2</small></>, here: "Project here, session here", lands: <><b>The session’s page</b></>, yours: "Nothing", later: true },
  { page: null, here: "Project here, session only in the cloud", lands: <><b>The project</b><small>its Only in Mallard Cloud rows</small></>, yours: "Pull", later: true },
  { page: null, here: "Project not here", lands: <><b>As a project link</b><small>the session is dropped</small></>, yours: "As above", later: true },
  { page: <><b>Anything else</b><small>a profile, the listing, docs, reviews</small></>, here: "—", lands: <><b>Stays put</b><small>“Mallard can’t open that link.”</small></>, yours: "—", scene: "bad-link" },
];

function TheMap({ onScene }: { onScene: (id: string) => void }) {
  const see = (id: string) => {
    const scene = SCENES.find((candidate) => candidate.id === id);
    return scene ? <button type="button" className="spec-seg" onClick={() => onScene(id)}><b>{scene.n}</b>See</button> : null;
  };
  return (
    <>
      <div className="spec-card">
        <table className="spec-map">
          <caption>The page on the website decides the page in the app</caption>
          <thead>
            <tr><th>Page on the website</th><th>On this computer</th><th>Mallard opens</th><th>Left to you</th><th /></tr>
          </thead>
          <tbody>
            {MAP.map((row, index) => (
              <tr key={index} className={row.later ? "later" : undefined}>
                <td>{row.page}</td>
                <td>{row.here}</td>
                <td>{row.lands}</td>
                <td>{row.yours}</td>
                <td>{row.scene ? see(row.scene) : null}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="spec-card">
        <strong>The rule, in order</strong>
        <ol className="spec-rule">
          <li>Read the link. Only <code>mallard://open?address=&lt;a repository page&gt;</code> is one. {see("bad-link")}</li>
          <li>Find the storages that sign in to the address’s host. None: say so, offer setup when it is this build’s service. {see("no-account")} {see("other-host")}</li>
          <li>Ask the service what is at the address. Nothing this account can see: say so. {see("unseen")}</li>
          <li>It is here: open it. {see("project-here")} {see("item-here")}</li>
          <li>It is not: open the page that adds it, filled in. The link presses nothing there. {see("project-new")} {see("item-new")} {see("accounts")}</li>
          <li>The app is in the middle of something: the link waits under the bar. {see("held")}</li>
        </ol>
      </div>

      <div className="spec-pair">
        <div className="spec-card">
          <strong>What a link can never do</strong>
          <ul>
            <li>Sign in, connect, clone, create, join, Pull or Push.</li>
            <li>Name a verb, a folder, an account or a storage.</li>
            <li>Send the account’s token anywhere but its own saved service.</li>
            <li>Close a review or interrupt an operation.</li>
          </ul>
        </div>
        <div className="spec-card">
          <strong>What others do</strong>
          <p>From memory, not re-checked.</p>
          <ul>
            <li><b>GitHub Desktop</b>: already cloned, it switches to it; otherwise the Clone dialog, filled in. This is A.</li>
            <li><b>VS Code</b>: asks before a link opens anything, then asks for the folder. This is B.</li>
            <li><b>Steam, Cursor’s install links</b>: a link that adds something stops at a dialog with one verb.</li>
            <li><b>Slack, Linear, Figma</b>: straight to the object; signed out, sign in first, then continue.</li>
          </ul>
        </div>
      </div>
    </>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

interface Knobs { scene: string; dir: Dir; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const scene = query.get("scene") ?? "map";
  const dir = query.get("dir");
  return {
    scene: scene === "map" || SCENES.some((candidate) => candidate.id === scene) ? scene : "map",
    dir: dir === "b" || dir === "c" ? dir : "a",
    theme: query.get("theme") === "dark" ? "dark" : "light",
  };
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const [run, setRun] = useState(0);
  const set = (patch: Partial<Knobs>) => setKnobs((current) => ({ ...current, ...patch }));
  const scene = SCENES.find((candidate) => candidate.id === knobs.scene) ?? null;
  const direction = DIRECTIONS.find((candidate) => candidate.id === knobs.dir)!;

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
      <h1 className="spec-title">Open in Mallard<small>where a link from the website lands</small></h1>

      <div className="spec-controls">
        <div className="spec-group">
          <button type="button" className="spec-seg" aria-pressed={knobs.scene === "map"} onClick={() => set({ scene: "map" })}>Map</button>
        </div>
        <div className="spec-group">
          <span>Lands</span>
          {SCENES.filter((candidate) => candidate.group === "lands").map(sceneButton)}
        </div>
        <div className="spec-group">
          <span>Stops or waits</span>
          {SCENES.filter((candidate) => candidate.group === "stops").map(sceneButton)}
        </div>
        <div className="spec-group">
          <span>Arrives</span>
          {DIRECTIONS.map((candidate) => (
            <button key={candidate.id} type="button" className={`spec-seg${candidate.id === "a" ? " pick" : ""}`} aria-pressed={knobs.dir === candidate.id} onClick={() => set({ dir: candidate.id })}>
              {candidate.label}
            </button>
          ))}
        </div>
        <div className="spec-group">
          <span>Theme</span>
          {(["light", "dark"] as const).map((theme) => (
            <button key={theme} type="button" className="spec-seg" aria-pressed={knobs.theme === theme} onClick={() => set({ theme })}>{theme === "light" ? "Light" : "Dark"}</button>
          ))}
        </div>
      </div>

      {scene ? (
        <>
          <div className="spec-card">
            <p><b>{scene.n} · {scene.label}.</b> {scene.shows}</p>
            <p><span>Arrives</span><b>{direction.label}.</b> {direction.note}</p>
            <p><span>New</span>{scene.fresh}</p>
          </div>
          <div className="spec-browser">
            <span>The browser hands over</span>
            <code>{scene.link}</code>
            <button type="button" className="btn" onClick={() => setRun((current) => current + 1)}><Icon name="refresh" size={13} /> Play again</button>
          </div>
          <Window key={`${scene.id}:${knobs.dir}`} scene={scene} dir={knobs.dir} run={run} />
        </>
      ) : (
        <TheMap onScene={(id) => set({ scene: id })} />
      )}
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<Specimen />);
