import { StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import Notice from "../src/components/notice/Notice";
import "../src/App.css";

/**
 * Connecting a project to a Mallard Cloud repository by its address, drawn
 * only with parts the app already has: the Folder field with its trailing
 * verb (`sync-folder-field`), the storage discovery's found row with facts,
 * cautions and a verb (`storage-found-row`), its empty state, the setup
 * page's field error and inline status, and the repository option rows.
 * Before reproduces `BundleConnectionDialog` as it ships for a Mallard Cloud
 * storage; A is the dialog for an existing project; B is the Repository band
 * of the project setup page. `npm run dev`, then /mockups/cloud-connect-mockup.html
 * (`?theme=dark`, `?scene=idle|resolving|member|public|mismatch|missing|origin|create`,
 * `?only=before|A|B`).
 */

/* ——— frozen data ————————————————————————————————————— */

type Scene = "idle" | "resolving" | "member" | "public" | "mismatch" | "missing" | "origin" | "create";
const SCENES: [Scene, string][] = [
  ["idle", "Open"], ["resolving", "Looking up"], ["member", "Found · yours"], ["public", "Found · public"],
  ["mismatch", "Found · other remote"], ["missing", "Not found"], ["origin", "Wrong service"], ["create", "New repository"],
];
type Level = "before" | "A" | "B";

const STORAGE = { name: "Mallard Cloud", host: "mallard-ai.com", login: "Rika321" };
const PROJECT = { folder: "HorrorGame", root: "~/Desktop/project/HorrorGame", bundleId: "7c1e0a5b9d3f4e2a8b6c1d0e9f8a7b6c" };

interface Repo {
  owner: string; name: string; klass: "git_project" | "non_git_project"; visibility: "Private" | "Public";
  role: "Owner" | "Admin" | "Member" | null; gitMatch: boolean; revision: number; resources: number; updated: string; bundleId: string;
}
const MINE: Repo[] = [
  { owner: "Rika321", name: "HorrorGame", klass: "git_project", visibility: "Private", role: "Owner", gitMatch: true, revision: 4, resources: 3, updated: "2 hours ago", bundleId: "3f9a0c7e1b2d4a6f8c0e2b4d6f8a1c3e" },
  { owner: "rikaqu0223-arch", name: "tidewatch", klass: "git_project", visibility: "Private", role: "Member", gitMatch: false, revision: 9, resources: 2, updated: "yesterday", bundleId: "a81b3c5d7e9f1a2b4c6d8e0f2a4b6c8d" },
  { owner: "Rika321", name: "Birding-Log", klass: "non_git_project", visibility: "Public", role: "Owner", gitMatch: false, revision: 2, resources: 2, updated: "3 days ago", bundleId: "c02d4e6f8a1b3c5d7e9f0a2b4c6d8e1f" },
];
const PUBLIC_REPO: Repo = { owner: "rikaqu0223-arch", name: "ledger-api", klass: "git_project", visibility: "Public", role: null, gitMatch: false, revision: 3, resources: 1, updated: "last week", bundleId: "" };

const ADDRESS: Record<Scene, string> = {
  idle: "",
  resolving: "https://mallard-ai.com/Rika321/HorrorGame",
  member: "https://mallard-ai.com/Rika321/HorrorGame",
  public: "mallard-ai.com/rikaqu0223-arch/ledger-api",
  mismatch: "rikaqu0223-arch/tidewatch",
  missing: "https://mallard-ai.com/Rika321/HorrorGame2",
  origin: "https://staging.mallard-ai.com/Rika321/HorrorGame",
  create: "",
};
const RESOLVED: Partial<Record<Scene, Repo>> = { member: MINE[0], public: PUBLIC_REPO, mismatch: MINE[1] };

const fullName = (repo: Repo) => `${repo.owner}/${repo.name}`;
const markOf = (repo: Repo): IconName => (repo.klass === "git_project" ? "git-folder" : "folder");
const chip = (label: string, value: string) => <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>;
/** The project name as the service would store it, and the address it makes. */
const projectName = (scene: Scene) => (scene === "create" ? "HorrorGame laptop" : PROJECT.folder);
const newAddress = (scene: Scene) => (scene === "create" ? `${STORAGE.login}/HorrorGame-laptop` : `${STORAGE.login}/HorrorGame-2`);

/* ——— the parts, each one the app already draws ————————————————— */

/** The Folder field's shape: a name, the input with its verb beside it, a hint under. */
function AddressField({ scene }: { scene: Scene }) {
  const value = ADDRESS[scene];
  const looking = scene === "resolving";
  return (
    <label className="sync-folder-field compact">
      <span>Repository address</span>
      <div>
        <input value={value} readOnly spellCheck={false} placeholder={`${STORAGE.host}/owner/name`} aria-invalid={scene === "origin" || undefined} aria-label="Repository address" />
        <button type="button" className="btn" disabled={!value || looking}>
          <Icon name="search" size={12} /> {looking ? "Looking up…" : "Look up"}
        </button>
      </div>
      <small>The page address from the website, or just owner/name. Picking a row below fills it in.</small>
    </label>
  );
}

/** What the lookup answered, in the discovery list's own rows. */
function Answer({ scene }: { scene: Scene }) {
  if (scene === "resolving") {
    return <small className="sync-setup-inline-status"><span className="status-loader" /> Looking up Rika321/HorrorGame on {STORAGE.host}…</small>;
  }
  if (scene === "origin") {
    return (
      <div className="sync-setup-field-error">
        <Icon name="alert-triangle" size={13} /> That address is on staging.mallard-ai.com; this storage is Mallard Cloud at {STORAGE.host}.
      </div>
    );
  }
  if (scene === "missing") {
    return (
      <div className="r2-step-list">
        <div className="storage-found-empty">
          <Icon name="search" size={24} />
          <strong>Nothing at Rika321/HorrorGame2 that you can see.</strong>
          <span>Check the spelling. A private repository shows up only after its Owner invites you; the code is entered on the website.</span>
          <span className="storage-found-empty-verbs">
            <button type="button" className="btn btn-primary">Create a new repository instead</button>
          </span>
        </div>
      </div>
    );
  }
  const repo = RESOLVED[scene];
  if (!repo) return null;
  const verdict: { tone: string; icon: IconName; text: string } =
    scene === "member" ? { tone: "ok", icon: "check-circle", text: `Yours · ${repo.role}` }
    : scene === "public" ? { tone: "ok", icon: "check-circle", text: "Public · anyone signed in can push" }
    : { tone: "warn", icon: "alert-triangle", text: `Collaborator · ${repo.role}` };
  const remote = scene === "member" ? "this checkout's" : scene === "public" ? "verified" : "differs";
  const cautions = scene === "mismatch" ? ["Different Git remote than this checkout"] : [];
  return (
    <>
      <div className="r2-step-list">
        <div className="storage-found-list">
          <div className="storage-found-row with-verb">
            <span className="sync-storage-repository-icon"><Icon name={markOf(repo)} size={14} /></span>
            <span className="storage-found-text">
              <span className="storage-found-name">{fullName(repo)}<code>{repo.klass === "git_project" ? "git project" : "project"}</code></span>
              <span className={`storage-found-verdict ${verdict.tone}`}><Icon name={verdict.icon} size={12} /> {verdict.text}</span>
            </span>
            <span className="storage-found-facts">
              {chip("Visibility", repo.visibility)}
              {chip("Git remote", remote)}
              {cautions.map((caution) => <span key={caution} className="storage-found-caution"><Icon name="alert-triangle" size={12} /> {caution}</span>)}
            </span>
            <span className="storage-found-verb">
              <button type="button" className="btn-link">Open on website <Icon name="external-link" size={11} /></button>
              <button type="button" className="btn btn-primary">Use & connect</button>
            </span>
          </div>
        </div>
      </div>
      {scene === "mismatch" && (
        <label className="sync-setup-acknowledge">
          <input type="checkbox" defaultChecked />
          <span>This repository was pushed from a different Git remote than this checkout's. Connect it anyway, then review its stored changes from Activity.</span>
        </label>
      )}
    </>
  );
}

/** The repositories the account belongs to, plus New repository first, in the setup page's option rows or the dialog's. */
function Rows({ scene, kind }: { scene: Scene; kind: "match-option" | "choice" }) {
  const row = `sync-bundle-${kind}`;
  const copy = kind === "choice" ? "sync-bundle-choice-copy" : "sync-bundle-match-copy";
  const meta = kind === "choice" ? "sync-bundle-choice-meta" : "sync-bundle-match-meta";
  const selected = scene === "public" ? undefined : RESOLVED[scene];
  return (
    <>
      <button type="button" role="radio" aria-checked={scene === "create"} className={`${row} separate${scene === "create" ? " active" : ""}`}>
        <span className="sync-bundle-radio"><span /></span>
        {kind === "match-option" && <span className="setup-choice-mark" aria-hidden="true"><Icon name="git-fork" size={24} /></span>}
        <span className={copy}>
          <strong>New repository</strong>
          <span>{scene === "create"
            ? `Creates ${newAddress(scene)} in ${STORAGE.name} — from the project name`
            : `Creates ${newAddress(scene)} — the name HorrorGame is already yours`}</span>
        </span>
        <span className={meta}><small>private until released</small></span>
      </button>
      {MINE.map((repo) => {
        const active = selected?.bundleId === repo.bundleId;
        return (
          <button key={repo.bundleId} type="button" role="radio" aria-checked={active} className={`${row}${active ? " active" : ""}`}>
            <span className="sync-bundle-radio"><span /></span>
            {kind === "match-option" && <span className="setup-choice-mark" aria-hidden="true"><Icon name={markOf(repo)} size={24} /></span>}
            <span className={copy}>
              <strong>{fullName(repo)}{repo.gitMatch && <small className="sync-bundle-recommended">Git match</small>}</strong>
              <span>{repo.visibility} · {repo.role}</span>
            </span>
            {kind === "choice" ? (
              <span className={meta}><strong>Version {repo.revision}</strong><span>{repo.resources} resources</span><small>{repo.updated}</small></span>
            ) : (
              <span className={meta}>{repo.updated}</span>
            )}
          </button>
        );
      })}
    </>
  );
}

/* ——— Before: the dialog as it ships ————————————————————— */

function BeforeDialog() {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog mock-dialog" aria-labelledby="before-title">
      <header className="sync-modal-header">
        <div>
          <span className="sync-eyebrow">Repo identity</span>
          <h1 id="before-title">Choose a repo from this storage</h1>
          <p>{STORAGE.name} contains the repos below. Git matches are recommended, but every repo remains available.</p>
        </div>
        <button type="button" className="btn btn-ghost" aria-label="Close repo chooser"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body">
        <div className="sync-bundle-context">
          <Icon name="cloud" size={17} />
          <span><strong>{STORAGE.name}</strong><small>Current local repo {PROJECT.bundleId.slice(0, 12)}…</small></span>
        </div>
        <div className="sync-bundle-choice-list" role="radiogroup" aria-label="Available remote repos">
          {MINE.map((repo, index) => (
            <button key={repo.bundleId} type="button" role="radio" aria-checked={false} className="sync-bundle-choice">
              <span className="sync-bundle-radio"><span /></span>
              <span className="sync-bundle-choice-copy">
                <strong>{repo.name.replace("-", " ")}{index === 0 && <small className="sync-bundle-recommended">Recommended</small>}</strong>
                <span>{repo.bundleId}</span>
              </span>
              <span className="sync-bundle-choice-meta">
                <strong>Version {repo.revision}</strong>
                <span>{repo.resources} resources</span>
                <small>{repo.updated}</small>
              </span>
            </button>
          ))}
        </div>
        <Notice severity="info" timing="standing" label="Notes about connecting this repo">
          Connecting changes only Mallard metadata. It does not move or delete the checkout or provider profile.
        </Notice>
      </div>
      <footer className="sync-modal-footer">
        <button type="button" className="btn btn-ghost">Create separate repo</button>
        <div>
          <button type="button" className="btn">Cancel</button>
          <button type="button" className="btn btn-primary" disabled>Connect and review Pull</button>
        </div>
      </footer>
    </dialog>
  );
}

/* ——— A: the dialog, for a project that already exists ——————————— */

function AfterDialog({ scene }: { scene: Scene }) {
  const chosen = scene === "member" || scene === "mismatch" || scene === "public" || scene === "create";
  const verb = scene === "create" ? "Create and connect" : "Connect and review Pull";
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog mock-dialog" aria-labelledby="after-title">
      <header className="sync-modal-header">
        <div>
          <span className="sync-eyebrow">Repo identity</span>
          <h1 id="after-title">Connect {PROJECT.folder} to {STORAGE.name}</h1>
          <p>Paste a repository's address from the website, or pick one you belong to. Connecting changes only Mallard metadata; the checkout stays where it is.</p>
        </div>
        <button type="button" className="btn btn-ghost" aria-label="Close repo chooser"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body repo-connect">
        <div className="sync-bundle-context">
          <Icon name="cloud" size={17} />
          <span><strong>{STORAGE.name} · {STORAGE.login}</strong><small>{STORAGE.host} · origin github.com/Rika321/HorrorGame</small></span>
        </div>
        <AddressField scene={scene} />
        <Answer scene={scene} />
        <div className="sync-bundle-choice-list repo-rows" role="radiogroup" aria-label="Repositories you belong to">
          <Rows scene={scene} kind="choice" />
        </div>
      </div>
      <footer className="sync-modal-footer">
        <span>{scene === "create" ? `Creates ${newAddress(scene)}, private, version 1` : scene === "public" ? "Pull only" : "Pull reviews the snapshot before anything changes locally"}</span>
        <div>
          <button type="button" className="btn">Cancel</button>
          <button type="button" className="btn btn-primary" disabled={!chosen}>{verb}</button>
        </div>
      </footer>
    </dialog>
  );
}

/* ——— B: the setup page, Repository band ——————————————————— */

function SetupPage({ scene }: { scene: Scene }) {
  const pill = (icon: IconName, label: string, active: boolean) => (
    <button type="button" role="radio" aria-checked={active} className={`setup-pill${active ? " active" : ""}`} title={label}>
      <Icon name={icon} size={12} />
      <span className="setup-pill-label">{label}</span>
      <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
    </button>
  );
  const blocked = scene === "origin";
  return (
    <section className="sync-inline-new-project sync-setup-workspace" aria-labelledby="setup-title">
      <header className="sync-modal-header">
        <div><h1 id="setup-title">Set up {projectName(scene)}</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close setup; the draft is kept"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body">
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>Project</strong></div></div>
          <div className="sync-field-grid sync-setup-project-fields">
            <label>
              <span>Name</span>
              <input value={projectName(scene)} readOnly />
            </label>
            <label>
              <span>Folder</span>
              <div className="sync-profile-select-row">
                <input value={PROJECT.root} readOnly title={PROJECT.root} />
                <button type="button" className="btn"><Icon name="folder" size={12} /> Browse</button>
              </div>
            </label>
          </div>
        </section>
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>Storage</strong></div></div>
          <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
            {pill("computer", "None yet", false)}
            {pill("cloud", STORAGE.name, true)}
            {pill("drive", "Team R2", false)}
          </div>
        </section>
        <section className="sync-setup-section repo-connect">
          <div className="sync-card-heading">
            <div><strong>Repository</strong></div>
            {blocked && <span className="sync-setup-state blocked" aria-label="Action required" title="Action required"><Icon name="alert-triangle" size={14} /></span>}
          </div>
          <AddressField scene={scene} />
          <Answer scene={scene} />
          <div className="sync-bundle-match-options repo-rows" role="radiogroup" aria-label="Repo identity for this project">
            <Rows scene={scene} kind="match-option" />
          </div>
        </section>
      </div>
      <footer className="sync-modal-footer sync-setup-footer">
        <div>
          <button type="button" className="btn btn-ghost">Discard</button>
          <button type="button" className="btn btn-primary" disabled={scene === "idle" || scene === "resolving" || scene === "missing" || scene === "origin"}>Finish</button>
        </div>
      </footer>
    </section>
  );
}

/* ——— the page ———————————————————————————————————————— */

interface Direction { key: Level; title: string; blurb: string; changes: [string, string][] }
const DIRECTIONS: Direction[] = [
  {
    key: "before", title: "Before · as shipped",
    blurb: "Linking a project to a Mallard Cloud storage lists the repositories the account belongs to and tells them apart by display name and bundle id. The cloud address (Rika321/HorrorGame) appears nowhere, there is no way to type one, a public repository you do not belong to cannot be reached, and when the listing is empty or already holds the project's bundle the dialog never opens: the link silently creates {login}/{folder name}.",
    changes: [],
  },
  {
    key: "A", title: "A · Connect by address, in the dialog",
    blurb: "The same dialog, with the address first. The field is the Folder field's shape (name, input, verb, hint), the answer is the storage discovery's found row (mark, name, verdict line, fact chips, cautions, verb), a miss is the discovery's empty state, a wrong service is the setup page's field error. The rows you belong to are the dialog's own rows, named by address; New repository is one of them and says the address it will create. No new control, no bundle id.",
    changes: [
      ["Address field", "sync-folder-field: accepts https://{host}/{owner}/{name}, {host}/{owner}/{name}, or {owner}/{name}; a trailing /sessions/… or /release is dropped; a host that is not this storage's is refused before any request, as a field error"],
      ["Lookup", "GET /v1/repositories/by-name/{owner}/{name} with the storage's token; the inline status while it runs; 404 is the discovery empty state"],
      ["Found row", "storage-found-row: owner/name with the class beside it, a verdict line (Yours · Owner · Git remote matches / Public · anyone signed in can push / Collaborator · a different remote), Visibility · Version · Updated chips, cautions, Open on website, Use & connect"],
      ["Public", "Use & connect: anyone signed in pulls the latest push and pushes over it; the Owner decides what is released"],
      ["Other remote", "the existing acknowledgement checkbox under the row; the Activity review is unchanged"],
      ["Rows", "owner/name · Visibility · Role, version and time at the right, Git match kept; New repository first, saying the address it creates from the project name"],
      ["Footer", "Cancel and one verb: Connect and review Pull · Create and connect"],
    ],
  },
  {
    key: "B", title: "B · The Repository band of project setup",
    blurb: "The setup page as it ships (Project, Storage) with the Repository band carrying the same field, the same answer rows and the page's own option rows. The project name above is the requested repository name, so New repository previews the address it makes. The single fingerprint match no longer moves the radio by itself: Git match is a badge, the choice is a click, and Finish waits for it.",
    changes: [
      ["One name", "the Project name is the requested cloud name; New repository previews {login}/{normalized name}, with -2 when the account already holds it"],
      ["No silent pick", "the one-match auto-select goes; Git match stays as the badge"],
      ["Blocked", "a refused address marks the band the way any blocked band is marked; Finish stays off until a row is chosen or an address resolves"],
    ],
  },
];

const PARAMS = new URLSearchParams(location.search);

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [scene, setScene] = useState<Scene>(() => (SCENES.some(([key]) => key === PARAMS.get("scene")) ? PARAMS.get("scene") as Scene : "member"));
  const only = PARAMS.get("only") as Level | null;
  const shown = only ? DIRECTIONS.filter((d) => d.key === only) : DIRECTIONS;
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        {!only && (
          <header className="mockup-head">
            <div>
              <h1>Mallard Cloud · connect a project by address</h1>
              <p>
                Linking a local project to a Mallard Cloud repository named by its website address instead of a
                bundle id, drawn with the app's own parts. The scene knob puts every panel in the same state so they compare.
              </p>
            </div>
            <button type="button" className="btn btn-ghost" onClick={() => { const next = !dark; setDark(next); document.documentElement.setAttribute("data-theme", next ? "dark" : "light"); }}>
              <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
            </button>
          </header>
        )}
        {!only && (
          <div className="mock-controls">
            <div className="mock-toggle">
              <span className="mock-aside">Scene</span>
              <div className="sync-segmented" role="radiogroup" aria-label="Scene">
                {SCENES.map(([key, text]) => (
                  <button key={key} type="button" role="radio" aria-checked={scene === key} className={scene === key ? "active" : undefined} onClick={() => setScene(key)}>{text}</button>
                ))}
              </div>
            </div>
          </div>
        )}
        {shown.map((dir) => (
          <section key={dir.key} className="mockup-panel" id={dir.key}>
            <h3>{dir.title}</h3>
            <p className="mock-blurb">{dir.blurb}</p>
            <div className="mockup-stage">
              {dir.key === "before" ? <div className="mock-body"><BeforeDialog /></div>
                : dir.key === "A" ? <div className="mock-body"><AfterDialog scene={scene} /></div>
                : <div className="mock-body"><SetupPage scene={scene} /></div>}
            </div>
            {dir.changes.length > 0 && <dl className="mock-spec">{dir.changes.map(([term, detail]) => <div key={term}><dt>{term}</dt><dd>{detail}</dd></div>)}</dl>}
          </section>
        ))}
      </div>
    </div>
  );
}

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
body { background: var(--app-shell); }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 30px; padding-top: 18px; border-top: 1px solid var(--border); }
.mockup-panel:first-of-type { margin-top: 12px; border-top: 0; }
.mockup-panel > h3 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mock-blurb { max-width: 96ch; margin: 0 0 12px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.5; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-body { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 18px; }
.mock-aside { margin: 0; color: var(--text-3); font-size: var(--type-xs); line-height: 1.5; }
.mock-controls { position: sticky; top: 0; z-index: 4; display: flex; flex-wrap: wrap; gap: 10px 22px; margin: 18px 0 0; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-card); box-shadow: var(--shadow-card); }
.mock-toggle { display: flex; align-items: center; gap: 10px; }
.mock-spec { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 8px 18px; margin: 10px 0 0; }
.mock-spec > div { display: grid; gap: 2px; }
.mock-spec dt { color: var(--text-3); font-size: var(--type-2xs); font-weight: var(--weight-semibold); letter-spacing: 0.6px; text-transform: uppercase; }
.mock-spec dd { margin: 0; color: var(--text-1); font-size: var(--type-xs); line-height: 1.5; }
/* An open, non-modal dialog laid out in flow so three panels can show at once. */
.mock-dialog { position: static; max-height: none; }
/* The two rules the change adds to App.css: the band's parts stack with the
   page's gap, and the option rows' second line is prose (an address, a role),
   not an id, so it takes the text face. */
.repo-connect { display: flex; flex-direction: column; gap: 10px; }
.repo-connect > .r2-step-list { margin-top: 0; }
.repo-rows .sync-bundle-choice-copy span, .repo-rows .sync-bundle-match-copy span { font-family: var(--font); }
/* In the 680px dialog the found row's facts take the line under the name;
   on the wide setup page the row keeps its one-line shape. */
.sync-bundle-connection-dialog .storage-found-row.with-verb { grid-template-columns: 26px minmax(0, 1fr) auto; row-gap: 6px; }
.sync-bundle-connection-dialog .storage-found-row.with-verb > .sync-storage-repository-icon { grid-row: 1; grid-column: 1; }
.sync-bundle-connection-dialog .storage-found-row.with-verb > .storage-found-text { grid-row: 1; grid-column: 2; }
.sync-bundle-connection-dialog .storage-found-row.with-verb > .storage-found-facts { grid-row: 2; grid-column: 2 / -1; justify-content: flex-start; }
.sync-bundle-connection-dialog .storage-found-row.with-verb > .storage-found-verb { grid-row: 1; grid-column: 3; }
`;

document.documentElement.setAttribute("data-theme", PARAMS.get("theme") === "dark" ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
