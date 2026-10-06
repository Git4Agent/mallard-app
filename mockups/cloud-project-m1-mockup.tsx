import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "../src/components/Icons";
import Notice, { FieldMessage, type NoticeZoneProps } from "../src/components/notice/Notice";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { PathBarBelow } from "../src/components/sync-review/SyncPathBar";
import ProjectFolderField from "../src/components/project-sync/ProjectFolderField";
import type { FolderVerdict } from "../src/components/project-sync/projectFolder";
import type { ProviderDescriptor } from "../src/types";
import "../src/App.css";

/**
 * Mockup, round 5 of "a project from a Mallard Cloud link". Owner's picks:
 * the From Mallard Cloud door on New project, the account first (Storage
 * pills), the shipped Folder field, one verb that connects without pulling,
 * and cloning as best effort — gh, then git, else an empty folder. A
 * repository off GitHub always gets an empty folder. Fewer words throughout.
 *
 * `npm run dev` (or `tauri dev`), then /mockups/cloud-project-m1-mockup.html
 * (`?show=all|<state id>&agents=default|ambiguous&reason=auth|network&theme=dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Kind = "github" | "other" | "plain";
/** What can clone on this computer. */
type Tools = "gh" | "git" | "none";
type Phase = "empty" | "found" | "exists" | "checkout" | "running" | "fallback" | "failed" | "done" | "after";
type Reason = "auth" | "network";
type Agents = "default" | "ambiguous";
type Storages = "one" | "several" | "none";
type AccountId = "personal" | "work" | "staging";
interface Knobs { show: string; agents: Agents; reason: Reason; theme: "light" | "dark" }

interface Scene {
  id: string;
  title: string;
  note: string;
  storages: Storages;
  /** The checked pill: the only storage, or none until picked. */
  checked: AccountId | null;
  /** The accounts that can see the repository. */
  seers: AccountId[];
  kind: Kind;
  tools: Tools;
  phase: Phase;
}

const base = { storages: "several", checked: "work", seers: ["work"], kind: "github", tools: "gh" } as const;
const SCENES: Scene[] = [
  { ...base, id: "open", title: "Open", note: "Several accounts: pick one first.", checked: null, phase: "empty" },
  { ...base, id: "other-account", title: "Wrong account", note: "Looked up with the checked account only.", checked: "personal", phase: "found" },
  { ...base, id: "found", title: "Found", note: "One account, checked for you.", storages: "one", checked: "personal", seers: ["personal"], phase: "found" },
  { ...base, id: "no-tools", title: "No gh or git here", note: "Known before the click.", tools: "none", phase: "found" },
  { ...base, id: "not-github", title: "Not on GitHub", note: "Always an empty folder.", kind: "other", tools: "git", phase: "found" },
  { ...base, id: "no-git", title: "No Git", note: "An empty folder; Pull fills it later.", kind: "plain", phase: "found" },
  { ...base, id: "wrong-service", title: "Staging checked", note: "Refused before any request.", checked: "staging", phase: "found" },
  { ...base, id: "invite", title: "Private, not joined", note: "Join with a code, then as Found.", storages: "one", checked: "personal", seers: [], phase: "found" },
  { ...base, id: "no-account", title: "No account yet", note: "Sign in adds the storage.", storages: "none", checked: null, seers: [], phase: "empty" },
  { ...base, id: "exists", title: "Folder taken", note: "One way on: -2.", phase: "exists" },
  { ...base, id: "checkout", title: "Already cloned", note: "Same repository by fingerprint: just Connect.", phase: "checkout" },
  { ...base, id: "running", title: "Cloning", note: "Cancel stops it; nothing is left behind.", phase: "running" },
  { ...base, id: "fallback", title: "Clone failed", note: "No stop: an empty folder instead.", phase: "fallback" },
  { ...base, id: "after", title: "Then, on the project", note: "Said once, under the path bar.", phase: "after" },
  { ...base, id: "failed", title: "Folder can’t be made", note: "The only stop (or a refused connect).", phase: "failed" },
  { ...base, id: "done", title: "Connecting", note: "Then the project opens. Nothing is pulled.", phase: "done" },
].map((scene) => scene as Scene);

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    show: pick("show", ["all", ...SCENES.map((scene) => scene.id)], "all"),
    agents: pick("agents", ["default", "ambiguous"] as const, "default"),
    reason: pick("reason", ["auth", "network"] as const, "auth"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/* ——— data ——————————————————————————————————————————————————————— */

const HOST = "mallard-ai.com";

interface Account { id: AccountId; name: string; login: string; host: string; role: string }
const ACCOUNTS: Record<AccountId, Account> = {
  personal: { id: "personal", name: "Mallard Cloud Rika321", login: "Rika321", host: HOST, role: "Member" },
  work: { id: "work", name: "Mallard Cloud rika-acme", login: "rika-acme", host: HOST, role: "Admin" },
  staging: { id: "staging", name: "Mallard Cloud staging", login: "Rika321", host: `staging.${HOST}`, role: "Owner" },
};
const accountsOf = (storages: Storages): Account[] => (
  storages === "none" ? [] : storages === "one" ? [ACCOUNTS.personal] : [ACCOUNTS.personal, ACCOUNTS.work, ACCOUNTS.staging]
);

interface Repo { owner: string; name: string; git: boolean; source: string | null; command: string | null }
const REPO: Record<Kind, Repo> = {
  github: { owner: "maya", name: "portal", git: true, source: "github.com/acme/portal", command: "gh repo clone acme/portal ~/code/portal" },
  other: { owner: "maya", name: "billing-svc", git: true, source: null, command: null },
  plain: { owner: "maya", name: "field-notes", git: false, source: null, command: null },
};
const addressOf = (repo: Repo) => `${repo.owner}/${repo.name}`;

interface Row { address: string; git: boolean; role: string; when: string; added?: boolean; current?: boolean }
function rowsOf(account: Account, repo: Repo, sees: boolean): Row[] {
  const rows: Row[] = sees ? [{ address: addressOf(repo), git: repo.git, role: account.role, when: "2h ago", current: true }] : [];
  if (account.id === "personal") {
    rows.push({ address: "Rika321/HorrorGame", git: true, role: "Owner", when: "Yesterday", added: true });
    rows.push({ address: "Rika321/notes", git: false, role: "Owner", when: "Sep 20" });
  } else if (account.id === "work") {
    rows.push({ address: "rika-acme/infra", git: true, role: "Owner", when: "Sep 22" });
  } else {
    rows.push({ address: "Rika321/staging-smoke", git: true, role: "Owner", when: "Sep 14" });
  }
  return rows;
}

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const PROFILE_OPTIONS: Record<string, string[]> = { codex: ["~/.codex"], claude: ["~/.claude", "~/.claude-work"] };

(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC((command) => (command === "plugin:path|resolve_directory" ? "/Users/you" : null));

/* ——— one state, read ————————————————————————————————————————————— */

interface View {
  scene: Scene;
  knobs: Knobs;
  repo: Repo;
  accounts: Account[];
  checked: Account | null;
  looked: boolean;
  wrongService: boolean;
  visible: boolean;
  locked: boolean;
  /** Whether the click tries to clone at all: a GitHub repository with gh or git here. */
  clones: boolean;
}
function viewOf(scene: Scene, knobs: Knobs): View {
  const accounts = accountsOf(scene.storages);
  const checked = scene.checked ? ACCOUNTS[scene.checked] : null;
  const looked = scene.phase !== "empty" && checked !== null;
  const wrongService = looked && checked!.host !== HOST;
  return {
    scene, knobs, repo: REPO[scene.kind], accounts, checked, looked, wrongService,
    clones: scene.kind === "github" && scene.tools !== "none",
    visible: looked && !wrongService && scene.seers.includes(checked!.id),
    locked: scene.phase === "running" || scene.phase === "fallback" || scene.phase === "done",
  };
}

function folderVerdict(view: View): FolderVerdict {
  const { repo, scene } = view;
  if (scene.phase === "exists") {
    return { tone: "warn", message: "Holds files; needs a new or empty folder.", actions: [{ label: `Use ~/code/${repo.name}-2`, value: `~/code/${repo.name}-2` }] };
  }
  if (scene.phase === "checkout") return { tone: "ok", message: "Already its checkout. Nothing to clone." };
  if (!repo.git) return { tone: "ok", message: "New, empty folder." };
  if (scene.kind === "other") return { tone: "ok", message: "New, empty folder: not on GitHub." };
  if (scene.tools === "none") return { tone: "warn", message: "New, empty folder: no gh or git here." };
  return { tone: "ok", message: `New folder. Clones ${repo.source}; empty if that fails.` };
}

function primary(view: View): { label: string; enabled: boolean } {
  const { scene, knobs, visible, clones } = view;
  const verb = clones ? "Clone and connect" : "Create folder and connect";
  if (!visible) return { label: "Connect", enabled: false };
  switch (scene.phase) {
    case "running": return { label: "Cloning…", enabled: false };
    case "fallback": return { label: "Creating…", enabled: false };
    case "done": return { label: "Connecting…", enabled: false };
    case "failed": return { label: "Try again", enabled: true };
    case "checkout": return { label: "Connect", enabled: knobs.agents === "default" };
    case "exists": return { label: verb, enabled: false };
    default: return { label: verb, enabled: knobs.agents === "default" };
  }
}

const reasonText = (reason: Reason) => (reason === "auth" ? "GitHub said no." : "github.com unreachable.");

function footerStatus(view: View): string | null {
  const { repo, scene } = view;
  if (scene.phase === "running") return `Cloning ${repo.source} · 48%`;
  if (scene.phase === "fallback") return "Clone failed · making an empty folder…";
  if (scene.phase === "done") return `Connecting to ${addressOf(repo)}…`;
  return null;
}

/* ——— bands, in the setup page's own markup ——————————————————————— */

const chip = (label: string, value: ReactNode, key: string) => (
  <span key={key} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>
);

function Band({ title, className, children }: { title: string; className?: string; children: ReactNode }) {
  return (
    <section className={`sync-setup-section${className ? ` ${className}` : ""}`}>
      <div className="sync-card-heading"><div><strong>{title}</strong></div></div>
      {children}
    </section>
  );
}

/** New project's two ways in, in the New storage page's door grammar. */
function Doors({ disabled }: { disabled: boolean }) {
  const door = (icon: "folder" | "storage-mallard", title: string, detail: string, checked: boolean) => (
    <div className="storage-mode-option">
      <button type="button" className="storage-mode-door" role="radio" aria-checked={checked} disabled={disabled}>
        <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
        <span className="storage-mode-door-detail">{detail}</span>
      </button>
    </div>
  );
  return (
    <div className="storage-mode-doors" role="radiogroup" aria-label="Where the project comes from">
      {door("folder", "On this computer", "A folder you have.", false)}
      {door("storage-mallard", "From Mallard Cloud", "A repository you can open.", true)}
    </div>
  );
}

/** The Storage band, first: the account the address is looked up with. */
function StorageBand({ view }: { view: View }) {
  if (view.accounts.length === 0) {
    return (
      <Band title="Storage">
        <div className="r2-step-list">
          <div className="storage-found-empty" role="status">
            <Icon name="storage-mallard" size={24} />
            <strong>No Mallard Cloud account yet.</strong>
            <span className="storage-found-empty-verbs">
              <button type="button" className="btn btn-primary"><Icon name="link" size={13} /> Sign in with GitHub</button>
            </span>
          </div>
        </div>
      </Band>
    );
  }
  return (
    <Band title="Storage">
      <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
        {view.accounts.map((account) => {
          const active = view.checked === account;
          return (
            <button
              key={account.id}
              type="button"
              role="radio"
              aria-checked={active}
              aria-label={`${account.name} · Mallard Cloud`}
              title={`${account.login} on ${account.host}`}
              className={`setup-pill${active ? " active" : ""}`}
              disabled={view.locked}
            >
              <Icon name="storage-mallard" size={12} />
              <span className="setup-pill-label">{account.name}</span>
              <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
            </button>
          );
        })}
      </div>
      {!view.checked && <small className="sync-setup-hint">Choose an account first.</small>}
    </Band>
  );
}

/** The Repository band: the shipped address picker, trimmed. */
function RepositoryBand({ view }: { view: View }) {
  const { repo, checked, scene } = view;
  if (!checked) return null;
  const value = scene.phase === "empty" ? "" : `https://${HOST}/${addressOf(repo)}`;
  let answer: ReactNode = null;
  if (view.wrongService) {
    answer = <FieldMessage blocking>That address is on {HOST}, not {checked.host}.</FieldMessage>;
  } else if (view.looked && !view.visible) {
    answer = (
      <div className="r2-step-list">
        <div className="storage-found-empty" role="status">
          <Icon name="search" size={24} />
          <strong>{checked.login} can’t see {addressOf(repo)}.</strong>
          <span>Check the spelling, try another account, or use an invitation code.</span>
        </div>
      </div>
    );
  } else if (view.visible) {
    answer = (
      <div className="r2-step-list">
        <div className="storage-found-list" aria-label="Repository at this address">
          <div className="storage-found-row with-verb selected">
            <span className="sync-storage-repository-icon"><Icon name={repo.git ? "git-folder" : "folder"} size={14} /></span>
            <span className="storage-found-text">
              <span className="storage-found-name">
                {addressOf(repo)}
                <code>{repo.git ? "git project" : "project"}</code>
              </span>
              <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {checked.role}</span>
            </span>
            <span className="storage-found-facts">
              {chip("Visibility", "Private", "visibility")}
              {scene.kind === "github" && chip("Git", repo.source, "source")}
              {scene.kind === "other" && chip("Git", "not on GitHub", "remote")}
            </span>
            <span className="storage-found-verb">
              <button type="button" className="btn-link">Open on website <Icon name="external-link" size={11} /></button>
            </span>
          </div>
        </div>
      </div>
    );
  }
  const rows = rowsOf(checked, repo, !view.wrongService && scene.seers.includes(checked.id));
  return (
    <Band title="Repository" className="repo-connect">
      <label className="sync-folder-field compact">
        <span>Repository address</span>
        <div>
          <input value={value} onChange={() => undefined} disabled={view.locked} spellCheck={false} placeholder={`${checked.host}/owner/name`} />
          <button type="button" className="btn" disabled={!value || view.locked}><Icon name="search" size={12} /> Look up</button>
        </div>
      </label>
      {answer}
      <label className="sync-folder-field compact">
        <span>Invitation code (optional)</span>
        <div>
          <input value={scene.id === "invite" ? "K7QX2-M9PLD-4HT8W-ZR3NB" : ""} onChange={() => undefined} disabled={view.locked} spellCheck={false} placeholder="XXXXX-XXXXX-XXXXX-XXXXX" />
          <button type="button" className="btn" disabled={scene.id !== "invite"}><Icon name="lock" size={12} /> Join</button>
        </div>
      </label>
      <div className="sync-bundle-match-options repo-rows" role="radiogroup" aria-label="Repositories you belong to">
        {rows.map((row) => {
          const active = view.visible && !!row.current;
          return (
            <button
              key={row.address}
              type="button"
              role="radio"
              aria-checked={active}
              className={`sync-bundle-match-option${active ? " active" : ""}`}
              disabled={view.locked}
            >
              <span className="sync-bundle-radio"><span /></span>
              <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.git ? "git-folder" : "folder"} size={24} /></span>
              <span className="sync-bundle-match-copy">
                <strong>{row.address}{row.added && <small className="sync-bundle-recommended">Added</small>}</strong>
                <span>{row.role}</span>
              </span>
              <span className="sync-bundle-match-meta">{row.when}</span>
            </button>
          );
        })}
      </div>
    </Band>
  );
}

/** The Project band: Name follows the repository; the shipped Folder field
 *  (Browse picks the parent and keeps the name). */
function ProjectBand({ view }: { view: View }) {
  const { repo } = view;
  return (
    <Band title="Project">
      <div className="sync-field-grid sync-setup-project-fields">
        <label>
          <span>Name</span>
          <input value={repo.name} readOnly disabled />
        </label>
        <ProjectFolderField
          value={`~/code/${repo.name}`}
          verdict={folderVerdict(view)}
          disabled={view.locked}
          placeholder="~/code/my-app"
          onChange={() => undefined}
          onEnter={() => undefined}
          onBrowsed={() => undefined}
        />
      </div>
    </Band>
  );
}

/** The Agent band's cards, as setup draws them. */
function AgentBand({ view }: { view: View }) {
  const ambiguous = view.knobs.agents === "ambiguous";
  return (
    <Band title="Agent">
      <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
        {PROVIDERS.map((descriptor) => {
          const options = PROFILE_OPTIONS[descriptor.key];
          const value = descriptor.key === "claude" && ambiguous ? "" : options[0];
          return (
            <div key={descriptor.key} className="sync-provider-card enabled">
              <label className="sync-provider-toggle setup-marked-toggle">
                <input type="checkbox" checked readOnly disabled={view.locked} aria-label={`Use ${descriptor.display_name} in this project`} />
                <ProviderMark provider={descriptor.key} displayName={descriptor.display_name} size={12} />
                <strong>{descriptor.display_name}</strong>
                <span>{descriptor.profile_label}</span>
              </label>
              <div className="sync-agent-profile-list">
                <div className={`sync-agent-profile-row${value ? "" : " has-error"}`}>
                  <span className="sync-select-shell">
                    <select value={value} disabled={view.locked} aria-label={`${descriptor.display_name} profile`} onChange={() => undefined}>
                      <option value="">Choose profile</option>
                      {options.map((option) => <option key={option} value={option}>{option}</option>)}
                      <option value="__pending">Custom path…</option>
                    </select>
                  </span>
                  <button type="button" className="btn" disabled={view.locked}><Icon name="folder" size={13} /> Browse</button>
                </div>
              </div>
            </div>
          );
        })}
      </div>
      {ambiguous && <FieldMessage>Choose a profile for Claude.</FieldMessage>}
    </Band>
  );
}

function FromCloud({ view }: { view: View }) {
  const status = footerStatus(view);
  const verb = primary(view);
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section sync-project-overview">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="New project">
          <header className="sync-modal-header">
            <div><h1>New project</h1></div>
            <button type="button" className="btn btn-ghost" aria-label="Close; nothing has been saved"><Icon name="x" size={17} /></button>
          </header>
          <div className="sync-modal-body">
            <Doors disabled={view.locked} />
            <StorageBand view={view} />
            <RepositoryBand view={view} />
            {view.visible && <ProjectBand view={view} />}
            {view.visible && <AgentBand view={view} />}
            {view.scene.phase === "failed" && (
              <Notice severity="danger" timing="blocking" title="Folder not created." className="sync-setup-system-error">
                ~/code isn’t writable.
              </Notice>
            )}
          </div>
          <footer className="sync-modal-footer sync-setup-footer">
            {status && <span className="sync-setup-save-state" aria-live="polite">{status}</span>}
            <div>
              <button type="button" className="btn btn-ghost">Cancel</button>
              <button type="button" className="btn btn-primary" disabled={!verb.enabled}>{verb.label}</button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

/** After the page closes: the project, with the shell's message zone under
 *  its path bar saying the folder is empty. */
function AfterFallback({ view }: { view: View }) {
  const { repo, knobs } = view;
  const zone: NoticeZoneProps = {
    className: "sync-backend-banner",
    subject: "messages from Mallard",
    timing: "progress",
    items: [{
      key: "clone-fallback",
      severity: "warning",
      title: `${repo.name} wasn’t cloned.`,
      message: <>{reasonText(knobs.reason)} The folder is empty; clone into it: <code>{repo.command}</code></>,
      action: <button type="button" className="btn">Copy command</button>,
    }],
  };
  return (
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar
        crumbs={[
          { id: "projects", label: "Projects", tone: "root" },
          { id: "project", label: repo.name, icon: "folder" },
        ]}
        ariaLabel={`${repo.name} path`}
      />
      <main className="sync-main">
        <p className="sync-setup-hint" style={{ padding: "14px 18px" }}>Activity · 9 sessions only in {ACCOUNTS.work.name}</p>
      </main>
    </PathBarBelow.Provider>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({ show: next.show, agents: next.agents, reason: next.reason, theme: next.theme });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const shown = SCENES.filter((scene) => knobs.show === "all" || knobs.show === scene.id);
  const frameKey = `${knobs.agents}-${knobs.reason}`;
  return (
    <div className="spec">
      <h1 className="spec-title">New project › From Mallard Cloud</h1>

      <div className="spec-controls">
        <div className="spec-group">
          <span>Show</span>
          <select value={knobs.show} onChange={(event) => set("show", event.target.value)} aria-label="State">
            <option value="all">All states</option>
            {SCENES.map((scene) => <option key={scene.id} value={scene.id}>{scene.title}</option>)}
          </select>
        </div>
        <div className="spec-group"><span>Clone failed</span>{seg("reason", "auth", "Refused")}{seg("reason", "network", "Offline")}</div>
        <div className="spec-group"><span>Agents</span>{seg("agents", "default", "Defaults")}{seg("agents", "ambiguous", "Two Claude configs")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panel">
        <ul>
          <li><b>Clone and connect</b>: gh, then git. Both fail → an empty folder.</li>
          <li><b>Create folder and connect</b>: no Git, not on GitHub, or no gh/git here.</li>
          <li><b>Connect</b>: already cloned. Never pulls.</li>
        </ul>
      </div>

      <div className="spec-row">
        {shown.map((scene, index) => {
          const view = viewOf(scene, knobs);
          return (
            <div key={scene.id}>
              <h2 className="spec-col-title"><small>{index + 1}</small>{scene.title}</h2>
              <p className="spec-col-note">{scene.note}</p>
              <div className="spec-frame">
                {scene.phase === "after" ? <AfterFallback key={frameKey} view={view} /> : <FromCloud key={frameKey} view={view} />}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
