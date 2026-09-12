import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";

/**
 * Mockup: a Local-folder storage pointed at a folder that already holds
 * another Mallard storage's data.
 *
 * Today the settings drawer of a settled storage has two verbs, Test
 * connection and Save storage. The folder probe only asks "exists, is a
 * directory, writable", so a folder that already belongs to another storage
 * passes it; the Repositories list below silently becomes that folder's
 * contents; and because every edit autosaves into a persisted draft, closing
 * the page leaves a "Draft · Unsaved Local folder settings" row behind. There
 * is no Revert, no Cancel, and no sentence that says the folder is taken.
 *
 * Four directions, ordered by how much they move. `npm run dev`, then
 * /storage-collision-mockup.html.
 *
 * Landed 2026-09-10: D. `findDestinationConflict` in StorageSettings.tsx
 * reads the typed folder against every connected storage's folder fact, the
 * editor draws the band with "Open <storage>" and "Choose another…", the
 * workspace disables Test/Save/Create while it stands, and the backend's
 * `storage_destination_conflict` names the owning storage. A, B and C stay
 * here as the record of what was weighed.
 */

/* ——— data ——————————————————————————————————————————— */

const SAVED_PATH = "/Users/example/storage/personal";
const TAKEN_PATH = "/Users/example/storage/shared";

type Repo = { name: string; git: boolean; catalog?: boolean; revision: number; updated: string };
const OWN_REPOS: Repo[] = [
  { name: "HorrorGame", git: true, revision: 1, updated: "16h ago" },
  { name: "Skills · Codex", git: false, catalog: true, revision: 1, updated: "7h ago" },
];
const TAKEN_REPOS: Repo[] = [
  { name: "demo", git: false, revision: 3, updated: "2d ago" },
  { name: "mallard-app", git: true, revision: 12, updated: "40m ago" },
];

/* ——— shared parts —————————————————————————————————————— */

function Card({ children }: { children: ReactNode }) {
  return <article className="sync-storage-card">{children}</article>;
}

function CardHeader() {
  return (
    <header>
      <span className="sync-storage-icon"><Icon name="drive" size={18} /></span>
      <h2 className="sync-storage-editor-title">Storage settings</h2>
      <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
        <button type="button" role="radio" aria-checked={false} disabled>Cloudflare R2</button>
        <button type="button" role="radio" aria-checked className="active" disabled>Local folder</button>
        <button type="button" role="radio" aria-checked={false} disabled>Dropbox</button>
      </div>
    </header>
  );
}

function PathField({ value, onChange, readOnly, trailing }: {
  value: string;
  onChange?: (next: string) => void;
  readOnly?: boolean;
  trailing?: ReactNode;
}) {
  return (
    <label className="sync-folder-field compact">
      <span className="sync-field-name">Location path</span>
      <div>
        <input
          type="text"
          value={value}
          readOnly={readOnly}
          onChange={(event) => onChange?.(event.target.value)}
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
        />
        {trailing ?? <button type="button" className="btn"><Icon name="folder" size={13} /> Browse…</button>}
      </div>
    </label>
  );
}

function RepoRow({ repo }: { repo: Repo }) {
  return (
    <div className="sync-storage-repository-item">
      <div className="sync-storage-repository-row">
        <span className="sync-storage-repository-icon">
          <Icon name={repo.catalog ? "lightbulb" : repo.git ? "git-folder" : "folder"} size={14} />
        </span>
        <strong className="sync-storage-repository-name">{repo.name}</strong>
        <span className="sync-storage-repository-chips">
          <span className="sync-storage-repo-chip"><span>Revision</span><b>{repo.revision}</b></span>
          <span className="sync-storage-repo-chip"><span>Updated</span><b>{repo.updated}</b></span>
        </span>
        <button type="button" className="sync-history-commit-toggle" aria-label={`Show details for ${repo.name}`}>
          <Icon name="chevron-down" size={12} />
        </button>
      </div>
    </div>
  );
}

function Repositories({ repos, heading, hint }: { repos: Repo[]; heading?: ReactNode; hint?: ReactNode }) {
  return (
    <section className="sync-storage-repositories" aria-label="Repositories">
      <div className="sync-storage-repositories-heading">
        <h2>{heading ?? "Repositories"}</h2>
        {hint}
      </div>
      <div className="sync-storage-repository-list">
        {repos.map((repo) => <RepoRow key={repo.name} repo={repo} />)}
      </div>
    </section>
  );
}

function Footer({ children }: { children: ReactNode }) {
  return <footer className="sync-storage-editor-footer">{children}</footer>;
}

const takenFolder = (path: string) => path === TAKEN_PATH;

/* ——— Today ——————————————————————————————————————————— */

function Today() {
  const [path, setPath] = useState(TAKEN_PATH);
  const taken = takenFolder(path);
  return (
    <div className="mock-body">
      <Card>
        <CardHeader />
        <PathField value={path} onChange={setPath} />
        <Footer>
          <button type="button" className="btn">Test connection</button>
          <button type="button" className="btn btn-primary">Save storage</button>
        </Footer>
      </Card>
      <Repositories repos={taken ? TAKEN_REPOS : OWN_REPOS} />
      <p className="mock-aside">
        The list above is already the other folder's contents, and nothing says so. Type the
        saved path back by hand to see it change — that is the only way back today.
        <button type="button" className="mock-link" onClick={() => setPath(SAVED_PATH)}>reset</button>
      </p>
    </div>
  );
}

/* ——— A · Name the collision, add the way back ————————————— */

function DirectionA() {
  const [path, setPath] = useState(TAKEN_PATH);
  const dirty = path !== SAVED_PATH;
  const taken = takenFolder(path);
  return (
    <div className="mock-body">
      <Card>
        <CardHeader />
        <PathField value={path} onChange={setPath} />
        {taken && (
          <div className="storage-setup-guide" role="alert">
            <Icon name="alert-triangle" size={13} />
            <span className="storage-setup-guide-error">
              This folder already holds a Mallard storage — 2 repositories, last written 40m ago.
              Saving points <strong>myconfstorage</strong> at that data instead of its own.
            </span>
          </div>
        )}
        <Footer>
          {dirty && (
            <button type="button" className="btn btn-ghost" onClick={() => setPath(SAVED_PATH)}>
              Discard changes
            </button>
          )}
          <button type="button" className="btn">Test connection</button>
          <button type="button" className="btn btn-primary">Save storage</button>
        </Footer>
      </Card>
      <Repositories
        repos={taken ? TAKEN_REPOS : OWN_REPOS}
        hint={dirty && <span className="storage-repos-qualifier">in the folder as typed — not saved</span>}
      />
    </div>
  );
}

/* ——— B · A decision band with three doors ———————————————— */

type Choice = null | "adopt" | "keep";

function DirectionB() {
  const [path, setPath] = useState(TAKEN_PATH);
  const [choice, setChoice] = useState<Choice>(null);
  const taken = takenFolder(path) && choice !== "keep";
  const blocked = taken && choice === null;
  return (
    <div className="mock-body">
      <Card>
        <CardHeader />
        <PathField value={path} onChange={(next) => { setPath(next); setChoice(null); }} />
        {taken && (
          <div className={`notice ${choice === "adopt" ? "info" : "warning"} storage-collision-band`} role="alert">
            <Icon name={choice === "adopt" ? "info" : "alert-triangle"} size={13} className="notice-mark" />
            <div className="notice-body">
              <p className="notice-line">
                <strong className="notice-type">{choice === "adopt" ? "Note ·" : "Warning ·"}</strong>{" "}
                <strong className="notice-lead">Folder already in use.</strong>{" "}
                {choice === "adopt"
                  ? <>myconfstorage will take over the 2 repositories already in this folder. Its own 2 repositories stay in the old folder, unlinked.</>
                  : <>Another Mallard storage wrote 2 repositories here, most recently 40m ago. Choose what myconfstorage should do.</>}
              </p>
            </div>
            <div className="storage-collision-verbs">
              <button type="button" className={`btn${choice === "adopt" ? " btn-primary" : ""}`} onClick={() => setChoice("adopt")}>
                Use this folder's data
              </button>
              <button type="button" className="btn"><Icon name="folder" size={13} /> Choose another…</button>
              <button type="button" className="btn btn-ghost" onClick={() => { setPath(SAVED_PATH); setChoice("keep"); }}>
                Keep saved path
              </button>
            </div>
          </div>
        )}
        <Footer>
          <button type="button" className="btn" disabled={blocked}>Test connection</button>
          <button type="button" className="btn btn-primary" disabled={blocked}>Save storage</button>
        </Footer>
      </Card>
      <Repositories
        repos={taken ? TAKEN_REPOS : OWN_REPOS}
        heading={taken ? <>Already in <code className="storage-repos-path">{TAKEN_PATH.replace(/^.*\//, "")}</code></> : undefined}
        hint={taken && <span className="storage-repos-qualifier">another storage's — not myconfstorage's until saved</span>}
      />
    </div>
  );
}

/* ——— C · A local storage is its folder ————————————————————— */

function DirectionC() {
  const [moving, setMoving] = useState(false);
  return (
    <div className="mock-body">
      <Card>
        <CardHeader />
        <PathField
          value={SAVED_PATH}
          readOnly
          trailing={(
            <button type="button" className="btn" onClick={() => setMoving(true)}>
              <Icon name="folder" size={13} /> Change folder…
            </button>
          )}
        />
        <div className="storage-setup-guide" role="note">
          <Icon name="info" size={13} />
          <span className="storage-setup-guide-copy">
            A local storage is its folder. Changing the folder starts a new storage draft, which
            you can test, finish, or discard; this one stays as it is until then.
          </span>
        </div>
        {moving && (
          <div className="notice info storage-collision-band" role="status">
            <Icon name="info" size={13} className="notice-mark" />
            <div className="notice-body">
              <p className="notice-line">
                <strong className="notice-type">Note ·</strong>{" "}
                <strong className="notice-lead">New storage draft started</strong> from these settings —
                pick its folder on the setup page. It carries Discard draft and Cancel like every draft.
              </p>
            </div>
            <div className="storage-collision-verbs">
              <button type="button" className="btn btn-primary">Open draft</button>
              <button type="button" className="btn btn-ghost" onClick={() => setMoving(false)}>Never mind</button>
            </div>
          </div>
        )}
        <Footer>
          <button type="button" className="btn">Test connection</button>
          <button type="button" className="btn btn-primary" disabled>Save storage</button>
        </Footer>
      </Card>
      <Repositories repos={OWN_REPOS} />
    </div>
  );
}

/* ——— D · Refuse and redirect ———————————————————————————— */

function DirectionD() {
  const [path, setPath] = useState(SAVED_PATH);
  const connected = path === SAVED_PATH;
  return (
    <div className="mock-body">
      <Card>
        <header>
          <span className="sync-storage-icon"><Icon name="drive" size={18} /></span>
          <input value="Backup drive" readOnly aria-label="Storage name" />
          <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
            <button type="button" role="radio" aria-checked={false} disabled>Cloudflare R2</button>
            <button type="button" role="radio" aria-checked className="active" disabled>Local folder</button>
            <button type="button" role="radio" aria-checked={false} disabled>Dropbox</button>
          </div>
        </header>
        <PathField value={path} onChange={setPath} />
        {connected && (
          <div className="notice danger storage-collision-band" role="alert">
            <Icon name="alert-triangle" size={13} className="notice-mark" />
            <div className="notice-body">
              <p className="notice-line">
                <strong className="notice-type">Error ·</strong>{" "}
                <strong className="notice-lead">This folder is already connected as myconfstorage.</strong>{" "}
                A folder can be one storage. Open that storage, or choose another folder for this one.
              </p>
            </div>
            <div className="storage-collision-verbs">
              <button type="button" className="btn btn-primary">Open myconfstorage</button>
              <button type="button" className="btn"><Icon name="folder" size={13} /> Choose another…</button>
            </div>
          </div>
        )}
      </Card>
      <footer className="sync-storage-setup-footer">
        <div>
          <button type="button" className="btn btn-ghost">Discard draft…</button>
          <button type="button" className="btn btn-ghost">Cancel</button>
          <button type="button" className="btn" disabled={connected}>Test connection</button>
          <button type="button" className="btn btn-primary" disabled={connected}>Create storage</button>
        </div>
      </footer>
      <p className="mock-aside">
        Shown on the New-storage form; the settled drawer wears the same band with Discard changes
        beside it. Type any other path to clear it.
        <button type="button" className="mock-link" onClick={() => setPath(SAVED_PATH)}>reset</button>
      </p>
    </div>
  );
}

/* ——— harness ————————————————————————————————————————— */

function Panel({ title, cost, note, children }: { title: string; cost: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title} <span className="mockup-cost">{cost}</span></h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

const NOTES = {
  today: <>
    As it ships. You opened the settings of <strong>myconfstorage</strong> and changed its
    Location path to a folder that another Mallard storage already writes to. The folder
    probe only asks "exists, is a directory, writable", so Test connection passes. The
    Repositories list below has already become the other folder's contents, with nothing to
    say so. Every keystroke autosaved into a persisted edit draft, so leaving the page
    leaves a <em>Draft · Unsaved Local folder settings</em> row behind, and the drawer has
    no Discard, no Cancel, no revert: the only way back is retyping the old path.
  </>,
  a: <>
    Nothing moves; two things are added. A guide row under the field names the collision
    the moment the typed folder holds another storage's data — how many repositories, how
    recently written, and what Save would do. The footer gains <strong>Discard changes</strong>,
    shown only while the draft differs from the saved settings: it throws the edit draft
    away and restores the saved path, the same verb the create form already has as
    "Discard draft…". The Repositories heading takes a qualifier while unsaved, so the list
    stops passing another folder's contents off as this storage's. Save still goes
    through: the sentence says what it will do, and the user decides.
  </>,
  b: <>
    The collision becomes a decision the page refuses to save past. The band under the
    field states the fact and offers three doors — <strong>Use this folder's data</strong>,
    which restates the band as a note of what will happen; <strong>Choose another…</strong>,
    the same Browse; and <strong>Keep saved path</strong>, the revert. Until one is taken,
    Test and Save are disabled, so the taken folder can never be saved by accident. The
    Repositories section is retitled with the folder's name and says whose data it lists.
    Costs a gate and a folder-ownership probe: the backend has to look for another
    storage's marker in the folder, which today's Local adapter does not do.
  </>,
  c: <>
    Removes the collision from settings entirely. A local storage <em>is</em> its folder, so
    the settled drawer shows the path read-only, and the only verb on it is{" "}
    <strong>Change folder…</strong>, which starts a fresh storage draft from these settings
    and hands over to the setup page — where Discard draft, Cancel, Test connection and
    Finish setup already exist, and where direction B's band can live if the new folder
    is taken. Save storage on this drawer is left for a rename. Costs one descriptor flag
    (<code>immutable_on_edit</code> on the directory field) and one verb; the edit draft
    for a local storage then never carries a path change that could need reverting.
  </>,
};

const NOTE_D = <>
  The rule the backend already holds, moved to where the folder is chosen. Today{" "}
  <code>ensure_destination_available</code> refuses a second storage on the same canonical
  folder — but only inside Save, after the probe, with the sentence "That storage
  destination is already connected." and no name, and Test connection passes. Here the check
  runs the moment the path settles: the page compares the typed folder with every connected
  local storage's folder fact, the backend confirms it on Test and Save with the existing
  storage's id and name in the error, and the band names the storage and opens it. Test and
  Create stay disabled while the band stands, so the page in your screenshot is never reached.
  Nothing new for the user to learn: one folder, one storage.
</>;

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>A local storage pointed at a folder that is already taken</h1>
            <p>
              The settled storage drawer autosaves every edit into a draft and offers only
              Test connection and Save storage. When the new folder already holds another
              storage's data nothing names the collision, the Repositories list quietly
              switches to it, and there is no verb that goes back. Three directions below,
              ordered by how much they move: say it and add the way back; make it a decision;
              take the folder out of settings. Each panel keeps its own state.
            </p>
          </div>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              const next = !dark;
              setDark(next);
              document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
            }}
          >
            <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel title="Today" cost="ships now" note={NOTES.today}><Today /></Panel>
        <Panel title="A · Name it, add the way back" cost="one verb, one sentence" note={NOTES.a}><DirectionA /></Panel>
        <Panel title="B · Three doors" cost="one gate + a folder probe" note={NOTES.b}><DirectionB /></Panel>
        <Panel title="C · The folder is the storage" cost="one flag, one verb" note={NOTES.c}><DirectionC /></Panel>
        <Panel title="D · Refuse and redirect" cost="one check moved earlier, one door" note={NOTE_D}><DirectionD /></Panel>
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
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mockup-cost { margin-left: 8px; padding: 2px 7px; border-radius: var(--r-pill); color: var(--text-2); background: var(--chip-bg); font-family: var(--font-mono); font-size: var(--type-2xs); }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-panel > p code { font-family: var(--font-mono); font-size: var(--type-xs); }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-body { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 18px; }
.mock-aside { margin: 0; color: var(--text-3); font-size: var(--type-xs); line-height: 1.5; }
.mock-link { margin-left: 6px; padding: 0; border: 0; color: var(--text-2); background: none; font: inherit; text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
`;

/* Everything a cutover would lift into App.css. Every other class on this
   page is the app's own. */
const PROPOSED_CSS = `
/* A collision band is the notice box the app already draws, plus a row of
   doors under the sentence. The doors wrap as a group, so a narrow drawer
   drops them to their own line rather than squeezing the sentence. */
.storage-collision-band { flex-wrap: wrap; }
.storage-collision-verbs { display: flex; flex-wrap: wrap; gap: 6px; flex: 1 0 100%; justify-content: flex-end; }

/* A heading's qualifier: the list is real, its ownership is not settled. */
.sync-storage-repositories-heading { display: flex; align-items: baseline; gap: 10px; }
.storage-repos-qualifier { color: var(--text-3); font-size: var(--type-2xs); font-weight: var(--weight-semibold); letter-spacing: 0.06em; text-transform: uppercase; }
.storage-repos-path { font-family: var(--font-mono); font-size: 0.9em; font-weight: var(--weight-regular); }
`;

const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
