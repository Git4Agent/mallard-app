import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import { Step, type StepStatus, type StepView } from "../src/components/project-sync/StorageSetupStep";
import { StorageLockedForm } from "../src/components/project-sync/StorageSettings";
import type { StorageProviderDescriptor, StorageSummary } from "../src/types";
import "../src/App.css";

/**
 * Mockup: getting a project from a Mallard Cloud link. The person pastes the
 * repository's page address and picks a parent folder; Mallard clones the
 * Git project there (gh when it is signed in, else git) or makes the
 * non-Git project's folder, adds the project with the link's repository
 * chosen, and lands on the Pull review with nothing ticked.
 *
 * Three containers for one flow, all driven by the same phases:
 *
 *   A · Storage page   the settled Mallard Cloud storage grows a Projects card
 *                      whose lane (the R2 lane's grammar) asks Link, then
 *                      Location, then runs Clone › Add project › Connect.
 *   B · New project    the New project page gets a second source beside the
 *                      folder: From Mallard Cloud, in the setup page's bands.
 *   C · Clone sheet    a modal in GitHub Desktop's "Clone a repository"
 *                      anatomy: Link / Yours tabs, a local path, Clone.
 *
 * Knob "Finish": inline (Mallard adds and connects the project itself) or
 * setup (Mallard clones, then opens the existing setup page prefilled).
 * Static data with generic names. `npm run dev`, then
 * /mockups/cloud-project-mockup.html (`?dir=all|a|b|c&kind=github|other|plain&phase=…&fail=auth|git|network&agents=default|ambiguous&finish=inline|setup&theme=dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Dir = "a" | "b" | "c";
type Kind = "github" | "other" | "plain";
type Phase = "empty" | "signin" | "invite" | "found" | "exists" | "checkout" | "running" | "failed" | "done";
type Fail = "auth" | "git" | "network";
type Agents = "default" | "ambiguous";
type Finish = "inline" | "setup";
interface Knobs { dir: Dir | "all"; kind: Kind; phase: Phase; fail: Fail; agents: Agents; finish: Finish; theme: "light" | "dark" }

const KINDS: [Kind, string][] = [["github", "Git · linked to GitHub"], ["other", "Git · not on GitHub"], ["plain", "No Git"]];
const PHASES: [Phase, string][] = [
  ["empty", "Open"], ["signin", "No storage yet"], ["invite", "Private, not joined"], ["found", "Found"],
  ["exists", "Folder taken"], ["checkout", "Already cloned"], ["running", "Getting"], ["failed", "Failed"], ["done", "Connected"],
];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    dir: pick("dir", ["all", "a", "b", "c"] as const, "all"),
    kind: pick("kind", ["github", "other", "plain"] as const, "github"),
    phase: pick("phase", PHASES.map(([phase]) => phase), "found"),
    fail: pick("fail", ["auth", "git", "network"] as const, "auth"),
    agents: pick("agents", ["default", "ambiguous"] as const, "default"),
    finish: pick("finish", ["inline", "setup"] as const, "inline"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/* ——— data ——————————————————————————————————————————————————————— */

const ME = "Rika321";
const HOST = "mallard-ai.com";
const PARENT = "~/code";

interface Repo {
  owner: string;
  name: string;
  git: boolean;
  /** What Mallard clones, as the person reads it before the click: the
   *  service's GitHub link, or the URL pasted for a remote off GitHub. */
  source: string | null;
  /** The host a failure names. */
  remoteHost: string | null;
  /** How it clones. */
  tool: string | null;
  /** The same clone, for Terminal, when Mallard's attempt fails. */
  command: string | null;
}
const REPO: Record<Kind, Repo> = {
  github: {
    owner: "maya", name: "portal", git: true, source: "github.com/acme/portal", remoteHost: "github.com",
    tool: `gh, signed in as ${ME}`, command: "gh repo clone acme/portal ~/code/portal",
  },
  other: {
    owner: "maya", name: "billing-svc", git: true, source: "git@gitlab.com:acme/billing-svc.git", remoteHost: "gitlab.com",
    tool: "git over SSH", command: "git clone git@gitlab.com:acme/billing-svc.git ~/code/billing-svc",
  },
  plain: { owner: "maya", name: "field-notes", git: false, source: null, remoteHost: null, tool: null, command: null },
};

interface Yours { address: string; git: boolean; visibility: string; role: string; when: string; added?: boolean }
const YOURS: Yours[] = [
  { address: "maya/portal", git: true, visibility: "Private", role: "Member", when: "2h ago" },
  { address: "Rika321/HorrorGame", git: true, visibility: "Private", role: "Owner", when: "Yesterday", added: true },
  { address: "maya/field-notes", git: false, visibility: "Private", role: "Member", when: "Sep 23" },
  { address: "maya/billing-svc", git: true, visibility: "Private", role: "Member", when: "Sep 20" },
];

const STORAGE: StorageSummary = {
  storage_id: "storage-cloud",
  name: `Mallard Cloud ${ME}`,
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: ME, is_path: false },
    { field_id: "service_origin", label: "Service", value: `https://${HOST}`, is_path: false },
  ],
  settings: { service_origin: `https://${HOST}`, account_id: "user_1", account_login: ME },
  secrets: [{ secret_id: "account_token", present: true }],
};
const DESCRIPTOR: StorageProviderDescriptor = {
  key: "managed_cloud",
  config_version: 1,
  display_name: "Mallard Cloud",
  description: "Mallard's own service: sign in with GitHub, push private snapshots, invite collaborators, and read them on the website.",
  default_name: "Mallard Cloud",
  display_rank: 5,
  icon: "cloud",
  clear_mode: "unavailable",
  fields: [],
  secrets: [{ secret_id: "account_token", label: "Sign-in token", required: true, placeholder: "Issued by the browser sign-in" }],
  managed_secret_id: "account_token",
  local_setup: "managed_cloud",
  setup_guide_url: null,
  manual_setup_hint: null,
  setup_doors: null,
  discovery: null,
};

(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC((command) => (command === "plugin:path|resolve_directory" ? "/Users/you" : null));

/* ——— the scenario every direction reads ——————————————————————————— */

const resolved = (phase: Phase) => !(phase === "empty" || phase === "signin" || phase === "invite");
const started = (phase: Phase) => phase === "running" || phase === "failed" || phase === "done";
/** Fields lock while the run is going and once it is through; a failure hands them back. */
const locked = (phase: Phase) => phase === "running" || phase === "done";
const pathOf = (repo: Repo) => `${PARENT}/${repo.name}`;
const linkOf = (repo: Repo) => `https://${HOST}/${repo.owner}/${repo.name}`;

/** The verb that starts the run, from what it will do. */
function primaryLabel(knobs: Knobs): string {
  const repo = REPO[knobs.kind];
  if (knobs.phase === "running") return repo.git ? "Cloning…" : "Creating…";
  if (knobs.phase === "done") return knobs.finish === "setup" ? "Opening setup…" : "Opening Pull review…";
  if (knobs.phase === "failed") return "Try again";
  if (!resolved(knobs.phase)) return "Get project";
  const verb = knobs.phase === "checkout" ? null : repo.git ? "Clone" : "Create";
  if (knobs.finish === "setup") return verb ? `${verb} and continue` : "Continue";
  return verb ? `${verb} and connect` : "Connect";
}
function primaryEnabled(knobs: Knobs): boolean {
  if (knobs.phase === "failed") return true;
  if (!resolved(knobs.phase) || started(knobs.phase) || knobs.phase === "exists") return false;
  return !(knobs.finish === "inline" && knobs.agents === "ambiguous");
}

/** The line under the location: where it lands, or why it can't. */
function locationVerdict(knobs: Knobs): { tone: "ok" | "warn"; text: ReactNode; action?: string } {
  const repo = REPO[knobs.kind];
  const path = pathOf(repo);
  if (knobs.phase === "exists") {
    return { tone: "warn", text: <><code>{path}</code> already exists and holds files. A clone needs a new or empty folder.</>, action: `Use ${repo.name}-2` };
  }
  if (knobs.phase === "checkout") {
    return repo.git
      ? { tone: "ok", text: <><code>{path}</code> is already a checkout of {repo.source}. Mallard uses it and skips the clone.</> }
      : { tone: "ok", text: <><code>{path}</code> exists and is empty. Mallard uses it.</> };
  }
  return repo.git
    ? { tone: "ok", text: <>Clones into <code>{path}</code></> }
    : { tone: "ok", text: <>Creates the empty folder <code>{path}</code></> };
}

const chip = (label: ReactNode, value: ReactNode, key?: string) => (
  <span key={key} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>
);
const laneButton = (label: ReactNode, tone: "plain" | "primary" | "ghost" = "plain", disabled = false) => (
  <button
    type="button"
    className={`btn storage-setup-guide-button${tone === "primary" ? " btn-primary" : tone === "ghost" ? " btn-ghost" : ""}`}
    disabled={disabled}
  >
    {label}
  </button>
);

/** A failed step's hand-over, in the diagnostics sheet's grammar. */
function CommandSheet({ label, command }: { label: string; command: string }) {
  return (
    <div className="cpm-command">
      <span className="cpm-command-label">{label}</span>
      <div className="feishu-setup-diagnostics-sheet">
        <dl><dt>Terminal</dt><dd>{command}</dd></dl>
        <button type="button" className="feishu-setup-diagnostics-copy"><Icon name="copy" size={11} /> Copy</button>
      </div>
    </div>
  );
}

/** Clone (or Create folder): the one step that touches the disk. In the
 *  lane (A) the step carries its own Cancel and Try again; a page or sheet
 *  keeps them in its footer, and the step only offers the way around. */
function getStep(knobs: Knobs, laneControls = false): StepView {
  const repo = REPO[knobs.kind];
  const path = pathOf(repo);
  const { phase } = knobs;
  const chooseCloned = laneButton(<><Icon name="folder" size={12} /> Choose the cloned folder…</>);
  if (!repo.git) {
    const status: StepStatus = phase === "running" ? "doing" : phase === "failed" ? "failed" : phase === "done" ? "done" : "todo";
    return {
      key: "get", title: "Create folder", status,
      fact: phase === "checkout" ? <>Not needed · <code>{path}</code> is empty and ready</>
        : status === "doing" ? <>Creating <code>{path}</code>…</>
        : status === "done" ? <>Created <code>{path}</code></>
        : status === "failed" ? <>Couldn’t create <code>{path}</code> · {PARENT} is not writable</>
        : <>Creates <code>{path}</code> · Pull fills it with the files you pick</>,
      controls: status === "failed" ? laneButton(<><Icon name="folder" size={12} /> Choose another parent…</>, "primary") : undefined,
    };
  }
  if (phase === "checkout") {
    return { key: "get", title: "Clone", status: "todo", fact: <>Not needed · <code>{path}</code> is already its checkout</> };
  }
  if (phase === "running") {
    return {
      key: "get", title: "Clone", status: "doing",
      fact: <>Receiving objects · 48% · 31.2 MiB · {repo.tool}</>,
      controls: laneControls ? laneButton("Cancel", "ghost") : undefined,
    };
  }
  if (phase === "done") return { key: "get", title: "Clone", status: "done", fact: <>Cloned into <code>{path}</code></> };
  if (phase === "failed") {
    const host = repo.remoteHost ?? "the remote";
    const fact = knobs.fail === "auth"
      ? (knobs.kind === "github" ? <>GitHub said no · {ME} may not have access to acme/portal</> : <>{host} refused this computer’s SSH key</>)
      : knobs.fail === "git" ? "Git isn’t installed on this Mac"
      : <>Couldn’t reach {host} · check the network</>;
    return {
      key: "get", title: "Clone", status: "failed", fact,
      controls: <>{laneControls && laneButton("Try again", "primary")}{chooseCloned}</>,
      body: knobs.fail === "auth" && repo.command
        ? <CommandSheet label="Or clone it in Terminal, where Git can ask you to sign in, then choose the folder:" command={repo.command} />
        : knobs.fail === "git"
          ? <CommandSheet label="Install Apple’s command line tools (they include Git), then Try again:" command="xcode-select --install" />
          : undefined,
    };
  }
  return { key: "get", title: "Clone", status: "todo", fact: <>Clones {repo.source} with {repo.tool}</> };
}

/** The steps after the disk: inline, Mallard finishes; setup, the page does. */
function afterSteps(knobs: Knobs, agentsFact?: ReactNode): StepView[] {
  const repo = REPO[knobs.kind];
  const done = knobs.phase === "done";
  if (knobs.finish === "setup") {
    return [{
      key: "setup", title: "Finish setup", status: done ? "doing" : "todo",
      fact: done ? <>Opening setup for {repo.name}…</> : "Opens setup with Folder, Agent, Storage and Repository filled in",
    }];
  }
  return [
    {
      key: "add", title: "Add project", status: done ? "done" : "todo",
      fact: done ? <>Added {repo.name}</> : agentsFact ?? <>{repo.name} with Codex and Claude</>,
    },
    {
      key: "connect", title: "Connect", status: done ? "done" : "todo",
      fact: done ? <>Connected to {repo.owner}/{repo.name} · version 12</> : <>To {repo.owner}/{repo.name} on Mallard Cloud</>,
    },
    {
      key: "pull", title: "Review Pull", status: done ? "doing" : "todo",
      fact: done ? "Opening · nothing is ticked" : "Opens with nothing ticked; you pick what lands",
    },
  ];
}

/* ——— shared parts ————————————————————————————————————————————————— */

function LinkField({ knobs, labelHidden = false }: { knobs: Knobs; labelHidden?: boolean }) {
  const value = knobs.phase === "empty" ? "" : linkOf(REPO[knobs.kind]);
  const fixed = locked(knobs.phase);
  return (
    <label className="sync-folder-field compact">
      <span className={labelHidden ? "sync-visually-hidden" : undefined}>Link</span>
      <div>
        <input value={value} readOnly disabled={fixed} spellCheck={false} placeholder={`${HOST}/owner/name`} />
        <button type="button" className="btn" disabled={!value || fixed}><Icon name="search" size={12} /> Look up</button>
      </div>
      {knobs.phase === "empty" && <small>The page address from {HOST}, or owner/name. It opens as you paste.</small>}
    </label>
  );
}

/** What the link named: the project, with what getting it will do. */
function FoundRow({ knobs }: { knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  return (
    <div className="r2-step-list">
      <div className="storage-found-list" aria-label="Project at this link">
        <div className="storage-found-row with-verb">
          <span className="sync-storage-repository-icon"><Icon name={repo.git ? "git-folder" : "folder"} size={14} /></span>
          <span className="storage-found-text">
            <span className="storage-found-name">
              {repo.owner}/{repo.name}
              <code>{repo.git ? "git project" : "project"}</code>
            </span>
            <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> Collaborator · Member</span>
          </span>
          <span className="storage-found-facts">
            {chip("Visibility", "Private")}
            {knobs.kind === "github" && chip("Clone from", repo.source)}
            {knobs.kind === "other" && chip("Git remote", "not on GitHub")}
            {knobs.kind === "plain" && chip("Folder", "new, empty")}
          </span>
          <span className="storage-found-verb">
            <button type="button" className="btn-link">Open on website <Icon name="external-link" size={11} /></button>
          </span>
        </div>
      </div>
    </div>
  );
}

/** A remote off GitHub: the service knows its fingerprint, not its address. */
function CloneUrlField({ knobs, labelHidden = false }: { knobs: Knobs; labelHidden?: boolean }) {
  const fixed = locked(knobs.phase);
  return (
    <div className="cpm-measure">
      <label className="sync-folder-field compact">
        <span className={labelHidden ? "sync-visually-hidden" : undefined}>Clone URL</span>
        <div><input value={REPO.other.source ?? ""} readOnly disabled={fixed} spellCheck={false} /></div>
        <small>
          {REPO.other.owner}/{REPO.other.name} isn’t linked to GitHub, so paste its clone URL as{" "}
          <code>git remote get-url origin</code> prints it. Mallard checks it against the project; an https and an ssh
          spelling don’t match each other.
        </small>
      </label>
      <p className="sync-setup-path-hint ok">Matches the remote {REPO.other.owner} pushed from.</p>
      <button type="button" className="btn-link cpm-aside-link">Already cloned it? Choose the folder…</button>
    </div>
  );
}

function Invite({ knobs }: { knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  return (
    <div className="cpm-measure">
      <div className="r2-step-list">
        <div className="storage-found-empty" role="status">
          <Icon name="search" size={24} />
          <strong>Nothing at {repo.owner}/{repo.name} that you can see.</strong>
          <span>If it’s private, enter the invitation code {repo.owner} sent you. Joining opens it here.</span>
        </div>
      </div>
      <label className="sync-folder-field compact">
        <span>Invitation code</span>
        <div>
          <input value="K7QX2-M9PLD-4HT8W-ZR3NB" readOnly spellCheck={false} />
          <button type="button" className="btn"><Icon name="lock" size={12} /> Join repository</button>
        </div>
        <small>From the repository’s owner or an admin.</small>
      </label>
    </div>
  );
}

/** Link-first: the link arrives before any Mallard Cloud storage exists. */
function SignIn() {
  return (
    <div className="cpm-signin" role="note">
      <Icon name="storage-mallard" size={16} />
      <strong>Sign in to open this link</strong>
      <span>This computer has no Mallard Cloud storage yet. Signing in with GitHub adds one for {HOST}; the link opens right after.</span>
      <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta"><Icon name="link" size={13} /> Sign in with GitHub</button>
    </div>
  );
}

/** Repositories the account belongs to: the second way to name one. */
function YoursRows() {
  return (
    <div className="sync-bundle-match-options" role="radiogroup" aria-label="Yours on Mallard Cloud">
      {YOURS.map((row) => (
        <button key={row.address} type="button" role="radio" aria-checked={false} className="sync-bundle-match-option">
          <span className="sync-bundle-radio"><span /></span>
          <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.git ? "git-folder" : "folder"} size={24} /></span>
          <span className="sync-bundle-match-copy">
            <strong>{row.address}{row.added && <small className="sync-bundle-recommended">Added</small>}</strong>
            <span>{row.visibility} · {row.role}{row.git ? "" : " · no Git"}</span>
          </span>
          <span className="sync-bundle-match-meta">{row.when}</span>
        </button>
      ))}
    </div>
  );
}

function LocationFields({ knobs }: { knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  const fixed = locked(knobs.phase);
  const verdict = locationVerdict(knobs);
  return (
    <>
      <div className="cpm-location">
        <label className="sync-folder-field compact">
          <span>Parent folder</span>
          <div>
            <input value={PARENT} readOnly disabled={fixed} spellCheck={false} />
            <button type="button" className="btn" disabled={fixed}><Icon name="folder" size={13} /> Browse…</button>
          </div>
        </label>
        <label className="sync-folder-field compact">
          <span>Folder name</span>
          <div><input value={repo.name} readOnly disabled={fixed} spellCheck={false} aria-invalid={knobs.phase === "exists" || undefined} /></div>
        </label>
      </div>
      <p className={`sync-setup-path-hint ${verdict.tone}`}>
        {verdict.text}
        {verdict.action && <button type="button" className="btn-link">{verdict.action}</button>}
      </p>
      {knobs.phase === "found" && <small className="cpm-field-note">{PARENT} is where your newest project, api-server, lives.</small>}
    </>
  );
}

/** The agent homes the project binds: the draft's defaults, read back. */
function AgentsLine({ knobs, change = true }: { knobs: Knobs; change?: boolean }) {
  const fixed = locked(knobs.phase);
  return (
    <div className="cpm-agents">
      {chip(<><ProviderMark provider="codex" displayName="Codex" size={12} /> Codex</>, "~/.codex", "codex")}
      {knobs.agents === "default"
        ? chip(<><ProviderMark provider="claude" displayName="Claude" size={12} /> Claude</>, "~/.claude", "claude")
        : (
          <span className="r2-select-shell">
            <select aria-label="Claude config" disabled={fixed} defaultValue="">
              <option value="" disabled>Claude · choose a config…</option>
              <option>~/.claude</option>
              <option>~/.claude-work</option>
            </select>
          </span>
        )}
      {change && <button type="button" className="btn-link" disabled={fixed}>Change…</button>}
    </div>
  );
}

function Band({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="sync-setup-section">
      <div className="sync-card-heading"><div><strong>{title}</strong></div></div>
      {children}
    </section>
  );
}

/** The existing setup page, prefilled — drawn small; nothing on it changes. */
function PrefilledSetup({ knobs }: { knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  return (
    <section className="sync-inline-new-project sync-setup-workspace cpm-setup-mini" aria-label="Setup, prefilled">
      <header className="sync-modal-header">
        <div><h1>Set up {repo.name}</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body">
        <Band title="Project">
          <div className="cpm-setup-row"><Icon name={repo.git ? "git-folder" : "folder"} size={16} /><strong>{repo.name}</strong><code>{pathOf(repo)}</code></div>
        </Band>
        <Band title="Agent">
          <AgentsLine knobs={{ ...knobs, phase: "found" }} change={false} />
        </Band>
        <Band title="Storage">
          <div className="cpm-setup-row"><Icon name="storage-mallard" size={16} /><strong>{STORAGE.name}</strong><span>{HOST}</span></div>
        </Band>
        <Band title="Repository">
          <div className="cpm-setup-row">
            <Icon name="check-circle" size={14} /><code>{repo.owner}/{repo.name}</code>
            {repo.git ? <small className="sync-bundle-recommended">Git match</small> : <span>no Git</span>}
          </div>
        </Band>
      </div>
      <footer className="sync-modal-footer sync-setup-footer">
        <div>
          <button type="button" className="btn btn-ghost">Cancel</button>
          <button type="button" className="btn btn-primary" disabled={knobs.agents === "ambiguous"}>Finish and review Pull</button>
        </div>
      </footer>
    </section>
  );
}

/* ——— A · the storage page's lane ————————————————————————————————— */

function DirectionA({ knobs }: { knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  if (knobs.phase === "signin") {
    return <div className="spec-na">A lives on the storage’s own page, so a storage always exists here. The link-first sign-in is B’s and C’s.</div>;
  }
  const isResolved = resolved(knobs.phase);
  const isStarted = started(knobs.phase);
  const verdict = locationVerdict(knobs);
  const link: StepView = !isResolved
    ? {
      key: "link", title: "Link", status: "doing",
      fact: knobs.phase === "invite" ? <>Nothing at {repo.owner}/{repo.name} that you can see</> : `Paste the project’s page address from ${HOST}`,
      body: (
        <>
          <LinkField knobs={knobs} labelHidden />
          {knobs.phase === "invite" ? <Invite knobs={knobs} /> : (
            <div className="cpm-measure">
              <span className="cpm-list-head">Or one of yours<small>{YOURS.length}</small></span>
              <div className="storage-found-list r2-step-list">
                {YOURS.map((row) => (
                  <div key={row.address} className="storage-found-row with-verb">
                    <span className="sync-storage-repository-icon"><Icon name={row.git ? "git-folder" : "folder"} size={14} /></span>
                    <span className="storage-found-text">
                      <span className="storage-found-name">{row.address}<code>{row.git ? "git project" : "project"}</code></span>
                      <span className="storage-found-verdict ok">{row.visibility} · {row.role} · {row.when}</span>
                    </span>
                    <span className="storage-found-facts" />
                    <span className="storage-found-verb">
                      {row.added
                        ? <span className="storage-found-connected"><Icon name="check" size={12} /> Added · Open</span>
                        : <button type="button" className="btn">Use this</button>}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </>
      ),
    }
    : {
      key: "link", title: "Link", status: "done",
      fact: <><code>{repo.owner}/{repo.name}</code> · {repo.git ? "git project" : "project, no Git"} · you’re a member</>,
      controls: isStarted ? undefined : <button type="button" className="btn-link">Change</button>,
    };
  const location: StepView = !isResolved
    ? { key: "where", title: "Location", status: "todo", fact: "Where it goes on this computer" }
    : isStarted
      ? { key: "where", title: "Location", status: "done", fact: <code>{pathOf(repo)}</code> }
      : {
        key: "where", title: "Location", status: "doing",
        fact: (
          <span className="r2-name-field">
            <input className="r2-name-input cpm-parent-input" value={PARENT} readOnly aria-label="Parent folder" />
            <span>/</span>
            <input className="r2-name-input" value={repo.name} readOnly aria-label="Folder name" aria-invalid={knobs.phase === "exists" || undefined} />
          </span>
        ),
        controls: <>{laneButton(<><Icon name="folder" size={12} /> Browse…</>)}{laneButton(primaryLabel(knobs), "primary", !primaryEnabled(knobs))}</>,
        body: (
          <>
            {knobs.kind === "other" && <CloneUrlField knobs={knobs} />}
            <p className={`sync-setup-path-hint ${verdict.tone}`}>
              {verdict.text}
              {verdict.action && <button type="button" className="btn-link">{verdict.action}</button>}
            </p>
          </>
        ),
      };
  const agentsFact = knobs.agents === "ambiguous"
    ? <>{repo.name} with Codex and <AgentsLine knobs={knobs} change={false} /></>
    : <>{repo.name} with Codex <code>~/.codex</code> and Claude <code>~/.claude</code></>;
  const steps = [link, location, getStep(knobs, true), ...afterSteps(knobs, agentsFact)];
  return (
    <main className="sync-main sync-project-links-page sync-storage-settings-page">
      <section className="profile-links-section" aria-label={STORAGE.name}>
        <div className="sync-storage-settings-below sync-storage-settings-dedicated">
          <div className="sync-storage-details-panel">
            <StorageLockedForm
              storage={STORAGE}
              descriptor={DESCRIPTOR}
              disabled={false}
              onRename={async () => true}
              onRevealSecret={async () => "mc_••••"}
              onTest={() => undefined}
              testing={false}
              verifiedAt={Date.now() - 20_000}
              reconnect={{ open: false, onOpen: () => undefined, band: null }}
            />
            <article className="sync-storage-card cpm-card">
              <header>
                <span className="sync-storage-icon"><Icon name="folder" size={18} /></span>
                <strong className="cpm-card-title">Projects</strong>
                <span className="sync-storage-editor-provider">None on this computer yet</span>
              </header>
              <div className="storage-setup-route-row agent r2-lane" role="group" aria-label="Get a project">
                <Icon name="download" size={13} />
                <span className="storage-setup-route-label">Get a project</span>
                <span className="storage-setup-guide-copy">From its link: Mallard clones it or makes its folder, adds it, and connects it here.</span>
                <ol className="r2-steps">{steps.map((step) => <Step key={step.key} step={step} />)}</ol>
              </div>
            </article>
          </div>
        </div>
      </section>
    </main>
  );
}

/* ——— B · a second source on New project —————————————————————————— */

function DirectionB({ knobs }: { knobs: Knobs }) {
  const isResolved = resolved(knobs.phase);
  const isStarted = started(knobs.phase);
  const repo = REPO[knobs.kind];
  const agentsFact = knobs.agents === "ambiguous" ? <>{repo.name} with Codex and the Claude config you picked</> : undefined;
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section sync-project-overview">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="New project">
          <header className="sync-modal-header">
            <div><h1>New project</h1></div>
            <button type="button" className="btn btn-ghost" aria-label="Close"><Icon name="x" size={17} /></button>
          </header>
          <div className="sync-modal-body">
            <div className="storage-mode-doors" role="radiogroup" aria-label="Where the project comes from">
              <div className="storage-mode-option">
                <button type="button" className="storage-mode-door" role="radio" aria-checked={false} disabled={isStarted}>
                  <span className="storage-mode-door-title"><Icon name="folder" size={17} /><strong>On this computer</strong></span>
                  <span className="storage-mode-door-detail">A folder you already have.</span>
                </button>
              </div>
              <div className="storage-mode-option">
                <button type="button" className="storage-mode-door" role="radio" aria-checked disabled={isStarted}>
                  <span className="storage-mode-door-title"><Icon name="storage-mallard" size={17} /><strong>From Mallard Cloud</strong></span>
                  <span className="storage-mode-door-detail">Paste its link; Mallard clones it or makes its folder.</span>
                </button>
              </div>
            </div>

            <Band title="Link">
              <LinkField knobs={knobs} labelHidden />
              {knobs.phase === "signin" && <div className="cpm-measure"><SignIn /></div>}
              {knobs.phase === "invite" && <Invite knobs={knobs} />}
              {isResolved && <div className="cpm-measure"><FoundRow knobs={knobs} /></div>}
              {isResolved && knobs.kind === "other" && <CloneUrlField knobs={knobs} />}
              {knobs.phase === "empty" && (
                <div className="cpm-measure">
                  <span className="cpm-list-head">Or one of yours<small>{YOURS.length}</small></span>
                  <YoursRows />
                </div>
              )}
            </Band>

            {isResolved && (
              <Band title="Location">
                <LocationFields knobs={knobs} />
              </Band>
            )}

            {isResolved && knobs.finish === "inline" && (
              <Band title="Agents">
                <AgentsLine knobs={knobs} />
                {knobs.agents === "ambiguous" && !isStarted && (
                  <small className="cpm-field-note">Two Claude configs here · pick where {repo.name}’s pulled sessions go.</small>
                )}
              </Band>
            )}

            {isResolved && (
              <Band title="Steps">
                <ol className="r2-steps">{[getStep(knobs), ...afterSteps(knobs, agentsFact)].map((step) => <Step key={step.key} step={step} />)}</ol>
              </Band>
            )}
          </div>
          <footer className="sync-modal-footer sync-setup-footer">
            <div>
              <button type="button" className="btn btn-ghost">Cancel</button>
              <button type="button" className="btn btn-primary" disabled={!primaryEnabled(knobs)}>{primaryLabel(knobs)}</button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

/* ——— C · the clone sheet —————————————————————————————————————————— */

function DirectionC({ knobs }: { knobs: Knobs }) {
  const [tab, setTab] = useState<"link" | "yours">("link");
  const repo = REPO[knobs.kind];
  const isResolved = resolved(knobs.phase);
  const isStarted = started(knobs.phase);
  const steps = [getStep(knobs), ...afterSteps(knobs)];
  return (
    <div className="cpm-stage">
      <dialog open className="sync-modal sync-bundle-connection-dialog cpm-dialog" aria-labelledby="cpm-dialog-title">
        <header className="sync-modal-header">
          <div>
            <span className="sync-eyebrow">Mallard Cloud · {ME}</span>
            <h1 id="cpm-dialog-title">Get a project</h1>
            <p>Paste its link from {HOST}, or pick one of yours. Mallard clones it (or makes its folder) and connects it.</p>
          </div>
          <button type="button" className="btn btn-ghost" aria-label="Close" disabled={knobs.phase === "running"}><Icon name="x" size={17} /></button>
        </header>
        <div className="sync-modal-body">
          {isStarted ? (
            <>
              <div className="cpm-summary">
                <Icon name={repo.git ? "git-folder" : "folder"} size={15} />
                <code>{repo.owner}/{repo.name}</code>
                <Icon name="chevron-right" size={12} />
                <code>{pathOf(repo)}</code>
                {knobs.phase === "failed" && <button type="button" className="btn-link">Change</button>}
              </div>
              <ol className="r2-steps">{steps.map((step) => <Step key={step.key} step={step} />)}</ol>
            </>
          ) : (
            <>
              <div className="sync-segmented" role="tablist" aria-label="How to name the project">
                <button type="button" role="tab" aria-selected={tab === "link"} className={tab === "link" ? "active" : undefined} onClick={() => setTab("link")}>Link</button>
                <button type="button" role="tab" aria-selected={tab === "yours"} className={tab === "yours" ? "active" : undefined} onClick={() => setTab("yours")}>Yours · {YOURS.length}</button>
              </div>
              {tab === "link" ? (
                <>
                  <LinkField knobs={knobs} labelHidden />
                  {knobs.phase === "signin" && <SignIn />}
                  {knobs.phase === "invite" && <Invite knobs={knobs} />}
                  {isResolved && <FoundRow knobs={knobs} />}
                  {isResolved && knobs.kind === "other" && <CloneUrlField knobs={knobs} />}
                </>
              ) : <YoursRows />}
              {isResolved && <LocationFields knobs={knobs} />}
              {isResolved && knobs.finish === "setup" && (
                <small className="cpm-field-note">Next: the setup page opens with everything filled in, for Finish.</small>
              )}
            </>
          )}
        </div>
        <footer className="sync-modal-footer">
          {knobs.finish === "inline" && isResolved ? <AgentsLine knobs={knobs} /> : <span />}
          <div>
            <button type="button" className="btn">Cancel</button>
            <button type="button" className="btn btn-primary" disabled={!primaryEnabled(knobs)}>{primaryLabel(knobs)}</button>
          </div>
        </footer>
      </dialog>
    </div>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

const DIRECTIONS: { id: Dir; title: string; note: string }[] = [
  {
    id: "a",
    title: "A · The storage page’s lane",
    note: "Finish setup already lands here. The settled storage grows a Projects card whose lane asks Link, then Location, then runs Clone › Add project › Connect › Review Pull — each verb on the step it starts, as in the R2 lane. Reachable later from Storages › Mallard Cloud; not from Projects.",
  },
  {
    id: "b",
    title: "B · A second source on New project",
    note: "New project gets two doors in the storage-setup grammar: On this computer (today’s Folder band) or From Mallard Cloud. Finish setup opens it with Mallard Cloud chosen; Add project reaches it any time, with or without a storage (link-first).",
  },
  {
    id: "c",
    title: "C · A clone sheet",
    note: "GitHub Desktop’s Clone a repository, in the app’s modal: Link / Yours tabs, Parent folder + Folder name, one primary verb; progress replaces the form while it runs. Opens after Finish setup, from the storage page, and from a Projects door.",
  },
];

const FLOWS: { name: string; note: string; steps: [string, "" | "manual" | "new"][] }[] = [
  {
    name: "Today", note: "the website’s Pull guide", steps: [
      ["Add storage › Mallard Cloud › Sign in › Finish setup", ""],
      ["Find the Git URL on GitHub", "manual"],
      ["Terminal: git clone into a folder you pick", "manual"],
      ["Projects › Add project › that folder", ""],
      ["Setup: storage + paste mallard-ai.com/maya/portal", ""],
      ["Finish", ""],
      ["Pull", ""],
    ],
  },
  {
    name: "Storage-first", note: "A · B · C after Finish setup", steps: [
      ["Add storage › Mallard Cloud › Sign in › Finish setup", ""],
      ["Paste the link (asked for next)", "new"],
      ["Check the location › Clone and connect", "new"],
      ["Pull review opens, nothing ticked", ""],
    ],
  },
  {
    name: "Link-first", note: "B · C from Projects", steps: [
      ["Add project › From Mallard Cloud › paste the link", "new"],
      ["Sign in with GitHub — first time only, adds the storage", "new"],
      ["Check the location › Clone and connect", "new"],
      ["Pull review opens, nothing ticked", ""],
    ],
  },
];

const STANDARDS: { tool: string; href: string; does: ReactNode; take: ReactNode }[] = [
  {
    tool: "GitHub Desktop",
    href: "https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-a-repository-from-github-to-github-desktop",
    does: <>Clone a repository: GitHub.com / Enterprise / URL tabs, <em>Local path</em> + <em>Choose…</em>, path = parent + repository name, progress in the dialog, opens the repository after. Refuses a folder with files (“Git can only clone to empty folders”). “Open with GitHub Desktop” on the website prefills it.</>,
    take: <>Pick-or-paste in one place; the target must be new or empty; progress where you clicked; a website button can prefill the same flow later.</>,
  },
  {
    tool: "GitHub Desktop issue #13971",
    href: "https://github.com/desktop/desktop/issues/13971",
    does: <>When the single Local path field stopped appending the repository name, people cloned into their parent folder and hit the non-empty error.</>,
    take: <>Keep <em>Parent folder</em> and <em>Folder name</em> as two fields and print the full path under them.</>,
  },
  {
    tool: "VS Code · Git: Clone",
    href: "https://code.visualstudio.com/docs/sourcecontrol/repos-remotes",
    does: <>URL or Clone from GitHub, then pick the local folder, then offer to open it; Workspace Trust before code runs; a <code>git.defaultCloneDirectory</code> setting names the usual parent.</>,
    take: <>The parent is its own choice with a sensible default; show what will be cloned before it runs.</>,
  },
  {
    tool: "JetBrains · Clone Repository",
    href: "https://www.jetbrains.com/help/idea/set-up-a-git-repository.html",
    does: <>URL + Directory, Clone; then Trust and Open Project before anything runs.</>,
    take: <>One confirmation before anything lands: here the Pull review, with nothing ticked.</>,
  },
  {
    tool: "GitKraken · Clone",
    href: "https://srse-git-github-zero2hero.netlify.app/03-collaborative_github_basics/04-clone/",
    does: <>A destination folder plus a full-path preview built from it (labels per community guides; the <a href="https://help.gitkraken.com/gitkraken-desktop/open-clone-init/" target="_blank" rel="noreferrer">official page</a> shows URL input and provider tabs); the connected account’s repositories to pick from.</>,
    take: <>Parent + name + full-path preview; the account’s repositories beside the paste field.</>,
  },
  {
    tool: "Obsidian · Create new vault",
    href: "https://obsidian.md/help/manage-vaults",
    does: <>Vault name + Location (Browse), Create; the vault is exactly that one new folder.</>,
    take: <>No Git: name + parent, and Mallard makes exactly that one folder.</>,
  },
  {
    tool: "gh repo clone",
    href: "https://cli.github.com/manual/gh_repo_clone",
    does: <><code>gh repo clone OWNER/REPO [DIR]</code> uses the configured <code>git_protocol</code> and gh’s own sign-in.</>,
    take: <>Prefer gh when it is installed and signed in; otherwise git. Mallard never asks for Git credentials.</>,
  },
  {
    tool: "git · GIT_TERMINAL_PROMPT",
    href: "https://git-scm.com/docs/git",
    does: <>Set to false, “git will not prompt on the terminal (e.g., when asking for HTTP authentication).”</>,
    take: <>Run clones with prompts off (and SSH in batch mode) so a background clone never waits on a hidden prompt; a refusal ends in a copyable Terminal command and Choose the cloned folder.</>,
  },
  {
    tool: "Deep links (later)",
    href: "https://github.com/microsoft/vscode/issues/235627",
    does: <>VS Code’s <code>vscode://</code> clone links and GitHub’s “Open with GitHub Desktop” open the local clone flow from a web page.</>,
    take: <>A later layer: an “Open in Mallard” button on the website that prefills B or C — never runs anything by itself.</>,
  },
];

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({
      dir: next.dir, kind: next.kind, phase: next.phase, fail: next.fail, agents: next.agents, finish: next.finish, theme: next.theme,
    });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string, disabled = false) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} disabled={disabled} onClick={() => set(key, value)}>{label}</button>
  );
  const shown = DIRECTIONS.filter((direction) => knobs.dir === "all" || knobs.dir === direction.id);
  const frameKey = `${knobs.kind}-${knobs.phase}-${knobs.fail}-${knobs.agents}-${knobs.finish}`;
  return (
    <div className="spec">
      <h1 className="spec-title">A project from a Mallard Cloud link — directions</h1>
      <p className="spec-sub">
        Paste the link, pick the parent folder; Mallard clones the Git project (or makes the non-Git project’s folder),
        adds it with that repository chosen, and opens the Pull review with nothing ticked. Every frame is driven by the
        same knobs, so any state can be compared across A, B and C.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Show</span>{seg("dir", "all", "All")}{seg("dir", "a", "A")}{seg("dir", "b", "B")}{seg("dir", "c", "C")}</div>
        <div className="spec-group"><span>Project</span>{KINDS.map(([kind, label]) => seg("kind", kind, label))}</div>
        <div className="spec-group"><span>State</span>{PHASES.map(([phase, label]) => seg("phase", phase, label))}</div>
        <div className="spec-group"><span>Failure</span>{seg("fail", "auth", "Refused", knobs.phase !== "failed")}{seg("fail", "git", "No Git", knobs.phase !== "failed")}{seg("fail", "network", "Offline", knobs.phase !== "failed")}</div>
        <div className="spec-group"><span>Finish</span>{seg("finish", "inline", "Inline")}{seg("finish", "setup", "Setup page")}</div>
        <div className="spec-group"><span>Agents</span>{seg("agents", "default", "Defaults")}{seg("agents", "ambiguous", "Two Claude configs")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panel spec-flows">
        <strong>How many moves to a pulled project</strong>
        {FLOWS.map((flow) => (
          <div key={flow.name} className="spec-flow">
            <div className="spec-flow-name">{flow.name} · {flow.steps.length}<small>{flow.note}</small></div>
            <ol>{flow.steps.map(([step, tone]) => <li key={step} className={tone || undefined}>{step}</li>)}</ol>
          </div>
        ))}
      </div>

      <div className="spec-panel spec-pick">
        <strong>Recommended: B, finishing on the setup page first</strong>
        <ul>
          <li><b>B</b> puts the flow where people already add projects (Add project), with no new door or modal, and serves both entries: right after Finish setup, and link-first for a teammate who has only the link. C serves both too, but as a modal that holds the app through a multi-minute clone; A is reachable only from the storage.</li>
          <li><b>Finish = Setup page</b> is the smaller first layer: the new code gets the folder (clone or mkdir) and prefills a draft; Agent choice, validation and Finish stay on the page that already owns them. The page needs two small changes (the picker seeded with the resolved repository, and a “Finish and review Pull” verb). <b>Inline</b> is closer to “Mallard links it for me” and can be the first layer instead if one fewer page matters more.</li>
          <li>The storage page links to B (“Get a project…”) rather than growing a second copy of the flow (A); C’s sheet stays the fallback if a page switch after Finish setup feels abrupt.</li>
        </ul>
      </div>

      <div className="spec-row">
        {shown.map((direction) => (
          <div key={direction.id}>
            <h2 className="spec-col-title">{direction.title}</h2>
            <p className="spec-col-note">{direction.note}</p>
            <div className="spec-frame">
              {direction.id === "a" ? <DirectionA key={frameKey} knobs={knobs} />
                : direction.id === "b" ? <DirectionB key={frameKey} knobs={knobs} />
                  : <DirectionC key={frameKey} knobs={knobs} />}
            </div>
            {knobs.finish === "setup" && resolved(knobs.phase) && (
              <div className="spec-then">
                <p>Then, after {primaryLabel({ ...knobs, phase: "found" })}: the existing setup page, prefilled (sketch). Two changes to it: the Repository band’s picker is seeded with the resolved address and its found row — a public repository you don’t belong to isn’t in the listing, so a bare draft choice would show nothing selected — and Finish reads “Finish and review Pull”.</p>
                <div className="spec-frame"><PrefilledSetup knobs={knobs} /></div>
              </div>
            )}
          </div>
        ))}
      </div>

      <h2 className="spec-h2">What established tools do</h2>
      <div className="spec-panel">
        <table className="spec-table">
          <thead><tr><th>Tool</th><th>What it does</th><th>What Mallard takes</th></tr></thead>
          <tbody>
            {STANDARDS.map((row) => (
              <tr key={row.tool}>
                <td><a href={row.href} target="_blank" rel="noreferrer">{row.tool}</a></td>
                <td>{row.does}</td>
                <td>{row.take}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <h2 className="spec-h2">What sits behind every direction</h2>
      <div className="spec-panel">
        <ol>
          <li><b>The storage comes first.</b> A link resolves with the storage’s sign-in, so storage-first starts after Finish setup; link-first signs in (and saves the storage) before the lookup.</li>
          <li><b>Resolve</b> with the shipped <code>resolve_managed_cloud_repository</code> and invitation join. The picker gains a no-checkout mode: no remote comparison (there is no checkout yet, so today it would say “differs”), no New repository row.</li>
          <li><b>Clone source.</b> <code>ManagedCloudRepository</code> gains <code>github_url</code> (Rust and <code>types.ts</code> together). Set on the service when <code>github.com/{"{owner}/{name}"}</code> hashes to the fingerprint or the owner links it on the website. Absent (an organisation’s repository, GitLab), the person pastes the clone URL and Mallard checks it against the fingerprint before cloning.</li>
          <li><b>Location probe</b> (read-only): new, empty, holds files (offer <code>-2</code>), already this project’s checkout by fingerprint (use it, skip the clone), inside Mallard’s data (refused). Default parent: the folder the newest project sits in; no new setting. Default name: the repository name.</li>
          <li><b>Get</b>: <code>gh repo clone owner/repo &lt;dest&gt;</code> when <code>find_binary("gh")</code> finds it signed in, else <code>git clone --progress</code>; <code>GIT_TERMINAL_PROMPT=0</code>, SSH in batch mode, Cancel kills the child. Non-Git: create exactly one new folder under an existing parent. Mallard deletes nothing on failure; git removes a folder it created itself.</li>
          <li><b>Finish</b>: create the setup draft on the new folder with the storage and <code>DraftRepositoryChoice::Existing</code> chosen; Inline finalizes it, Setup page opens it — with <code>RepositoryAddressPicker</code> seeded (initial address and found row), since a draft choice alone only shows when the repository is in the account’s listing. Either way it ends at the Pull review, nothing ticked; apply stays explicit.</li>
          <li><b>Public links</b>: a public repository the person doesn’t belong to resolves as “Public · anyone signed in can push”; getting it is the same flow (Pull and Push both work), with that verdict on the found row instead of Collaborator.</li>
          <li><b>Recovery is the person’s</b>: no automatic retry. A clone that landed but did not connect is found next time as “already this project’s checkout”.</li>
          <li><b>Later, separately</b>: the website’s “Open in Mallard” deep link and “Copy address” button, and the website docs’ “Clone it, then Add project” step.</li>
        </ol>
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
