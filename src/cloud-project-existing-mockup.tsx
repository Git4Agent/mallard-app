import { StrictMode, createContext, useContext, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "./components/Icons";
import { Acknowledgement } from "./components/notice/Notice";
import ProviderMark from "./components/sync-review/ProviderMark";
import type { ProviderDescriptor } from "./types";
import "./App.css";

/**
 * Mockup: New project › From Mallard Cloud with a folder that is already on
 * this computer, used as it is — no clone, no new folder. Today the Folder
 * line refuses a folder that holds files. A: a link on that line. B: a New
 * folder / Existing folder switch heading the field (Browse follows it).
 * C: B, opened on a checkout Mallard found in agent history (later layer).
 *
 * `npm run dev` (or `tauri dev`), then /cloud-project-existing-mockup.html
 * (`?show=all|today|a|b|c|<frame id>&switch=pills|segmented&theme=dark`).
 */

/* ——— data ——————————————————————————————————————————————————————— */

type RepoKind = "git" | "plain";
interface Repo { address: string; name: string; git: boolean; source: string | null; role: string; when: string }
const REPOS: Record<RepoKind, Repo> = {
  git: {
    address: "rikaqu0223-arch/HorrorGame",
    name: "HorrorGame",
    git: true,
    source: "github.com/rikaqu0223-arch/HorrorGame",
    role: "Owner",
    when: "not pushed yet",
  },
  plain: {
    address: "rikaqu0223-arch/field-notes",
    name: "field-notes",
    git: false,
    source: null,
    role: "Owner",
    when: "2h ago",
  },
};

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const PROFILE: Record<string, string> = { codex: "~/.codex", claude: "~/.claude" };

(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC((command) => (command === "plugin:path|resolve_directory" ? "/Users/you" : null));

/* ——— frames ———————————————————————————————————————————————————— */

type Direction = "today" | "a" | "b" | "c";
type Mode = "new" | "existing";
interface Line { tone: "ok" | "warn" | "bad"; text: string; links?: string[] }
interface Frame {
  id: string;
  direction: Direction;
  title: string;
  note: string;
  repo: RepoKind;
  /** The switch's position; none for Today and A, which have no switch. */
  mode?: Mode;
  folder: string;
  line: Line;
  /** Setup's Git-remote tick, when the folder can't be verified. */
  ack?: boolean;
  verb: string;
  enabled: boolean;
  status?: string;
  /** The whole page rather than the Project band and the footer. */
  full?: boolean;
  /** C: the recent folders, the found checkout first. */
  recent?: boolean;
}

const GIT = REPOS.git;
const cloneLine: Line = { tone: "ok", text: `New folder. Clones ${GIT.source}; empty if that fails.` };
const holdsLinks = ["Use it as it is", "Use ~/Desktop/project/demo-2"];
const noRemote: Line = { tone: "warn", text: "No Git remote to check." };
const checkout: Line = { tone: "ok", text: "Already its checkout. Nothing to clone." };

const FRAMES: Frame[] = [
  {
    id: "dead-end", direction: "today", title: "Today", full: true, repo: "git",
    note: "A folder that holds files is refused; the only way on is a new folder beside it.",
    folder: "~/Desktop/project/demo",
    line: { tone: "warn", text: "Holds files; needs a new or empty folder.", links: ["Use ~/Desktop/project/demo-2"] },
    verb: "Clone and connect", enabled: false,
  },

  {
    id: "a-holds", direction: "a", title: "Holds files", repo: "git",
    note: "The line gains a second way on. No new control.",
    folder: "~/Desktop/project/demo",
    line: { tone: "warn", text: "Holds files.", links: holdsLinks },
    verb: "Clone and connect", enabled: false,
  },
  {
    id: "a-as-is", direction: "a", title: "Used as it is", repo: "git",
    note: "Git repository, no remote in the folder: setup’s tick. Editing the path undoes it.",
    folder: "~/Desktop/project/demo",
    line: { ...noRemote, links: ["Use a new folder"] }, ack: true,
    verb: "Connect", enabled: false,
  },
  {
    id: "a-browse", direction: "a", title: "The Browse trap", repo: "git",
    note: "Browse picks the parent, so picking demo gives demo/HorrorGame. A folder with another name must be typed.",
    folder: "~/Desktop/project/demo/HorrorGame",
    line: cloneLine,
    verb: "Clone and connect", enabled: true,
  },

  {
    id: "b-open", direction: "b", title: "Open", full: true, repo: "git", mode: "new",
    note: "New folder first: today’s page, with the switch heading the field.",
    folder: "~/Desktop/project/HorrorGame",
    line: cloneLine,
    verb: "Clone and connect", enabled: true,
  },
  {
    id: "b-holds", direction: "b", title: "New folder, holds files", repo: "git", mode: "new",
    note: "Use it as it is flips the switch to Existing folder.",
    folder: "~/Desktop/project/demo",
    line: { tone: "warn", text: "Holds files.", links: holdsLinks },
    verb: "Clone and connect", enabled: false,
  },
  {
    id: "b-no-remote", direction: "b", title: "Existing: no Git remote", repo: "git", mode: "existing",
    note: "The screenshot. HorrorGame is Git and demo has no remote: one tick, setup’s rule.",
    folder: "~/Desktop/project/demo",
    line: noRemote, ack: true,
    verb: "Connect", enabled: false,
  },
  {
    id: "b-checkout", direction: "b", title: "Existing: its checkout", repo: "git", mode: "existing",
    note: "Remote matches by fingerprint. Browse picked the folder itself.",
    folder: "~/src/HorrorGame",
    line: checkout,
    verb: "Connect", enabled: true,
  },
  {
    id: "b-other-remote", direction: "b", title: "Existing: another remote", repo: "git", mode: "existing",
    note: "A fork, or another spelling off GitHub.",
    folder: "~/src/horror-fork",
    line: { tone: "warn", text: "Git remote: github.com/maya/HorrorGame." }, ack: true,
    verb: "Connect", enabled: false,
  },
  {
    id: "b-plain", direction: "b", title: "Existing: no Git either side", repo: "plain", mode: "existing",
    note: "Nothing to verify. A later Pull is reviewed, and a file that differs keeps a local copy.",
    folder: "~/Documents/field-notes",
    line: { tone: "ok", text: "Used as it is." },
    verb: "Connect", enabled: true,
  },
  {
    id: "b-plain-git", direction: "b", title: "Existing: Git folder, repository without", repo: "plain", mode: "existing",
    note: "Fingerprints differ (none vs this remote): the tick.",
    folder: "~/code/field-notes",
    line: { tone: "warn", text: "A Git checkout; field-notes has no Git." }, ack: true,
    verb: "Connect", enabled: false,
  },
  {
    id: "b-empty", direction: "b", title: "Existing: empty", repo: "git", mode: "existing",
    note: "Existing never clones. New folder clones into it.",
    folder: "~/Desktop/project/empty",
    line: { tone: "ok", text: "Empty. Nothing is cloned." },
    verb: "Connect", enabled: true,
  },
  {
    id: "b-missing", direction: "b", title: "Existing: nothing there", repo: "git", mode: "existing",
    note: "Switching keeps the text; the suggested new path isn’t there yet.",
    folder: "~/Desktop/project/HorrorGame",
    line: { tone: "bad", text: "No folder here." },
    verb: "Connect", enabled: false,
  },
  {
    id: "b-project", direction: "b", title: "Existing: already a project", repo: "git", mode: "existing",
    note: "Open it goes to that project.",
    folder: "~/code/api-server",
    line: { tone: "bad", text: "Already a project here.", links: ["Open it"] },
    verb: "Connect", enabled: false,
  },
  {
    id: "b-connecting", direction: "b", title: "Connecting", repo: "git", mode: "existing",
    note: "Setup’s finish, as today. Nothing is pulled; it opens on Activity.",
    folder: "~/src/HorrorGame",
    line: checkout,
    verb: "Connecting…", enabled: false, status: `Connecting to ${GIT.address}…`,
  },

  {
    id: "c-found", direction: "c", title: "Found in agent history", repo: "git", mode: "existing", recent: true,
    note: "Later layer. A recent folder whose remote matches opens Existing on it.",
    folder: "~/src/HorrorGame",
    line: checkout,
    verb: "Connect", enabled: true,
  },
];

const SECTIONS: { direction: Direction; title: string; sub: string }[] = [
  { direction: "today", title: "Today", sub: "shipped" },
  { direction: "a", title: "A · A link on the Folder line", sub: "smallest change; Browse unchanged" },
  { direction: "b", title: "B · New folder | Existing folder", sub: "recommended; Browse follows the switch" },
  { direction: "c", title: "C · B, opened on a checkout Mallard found", sub: "later layer" },
];

/** `show`: all, one direction (today|a|b|c), or one frame id. `look`: the
 *  switch as the Storage band's pills or as the shipped segmented pill. */
type Look = "pills" | "segmented";
interface Knobs { show: string; look: Look; theme: "light" | "dark" }
const LookContext = createContext<Look>("pills");
const shows = (show: string, frame: Frame) => show === "all" || show === frame.direction || show === frame.id;
function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const show = query.get("show");
  return {
    show: show && FRAMES.some((frame) => shows(show, frame)) ? show : "all",
    look: query.get("switch") === "segmented" ? "segmented" : "pills",
    theme: query.get("theme") === "dark" ? "dark" : "light",
  };
}

/* ——— bands, in the setup page's own markup ——————————————————————— */

function Band({ title, className, children }: { title: string; className?: string; children: ReactNode }) {
  return (
    <section className={`sync-setup-section${className ? ` ${className}` : ""}`}>
      <div className="sync-card-heading"><div><strong>{title}</strong></div></div>
      {children}
    </section>
  );
}

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

function StorageBand({ locked }: { locked: boolean }) {
  return (
    <Band title="Storage">
      <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
        <button type="button" role="radio" aria-checked className="setup-pill active" disabled={locked} aria-label="Mallard Cloud 2 · Mallard Cloud">
          <Icon name="storage-mallard" size={12} />
          <span className="setup-pill-label">Mallard Cloud 2</span>
          <span className="setup-pill-check" aria-hidden="true"><Icon name="check" size={12} /></span>
        </button>
      </div>
    </Band>
  );
}

/** The Repository band as the screenshot has it: the address, the code, and
 *  the chosen row in the account's list. */
function RepositoryBand({ repo, locked }: { repo: Repo; locked: boolean }) {
  return (
    <Band title="Repository" className="repo-connect cloud-project-repository">
      <label className="sync-folder-field compact">
        <span>Repository address</span>
        <div>
          <input value={repo.address} onChange={() => undefined} disabled={locked} spellCheck={false} placeholder="mallard-ai.com/owner/name" />
          <button type="button" className="btn" disabled={locked}><Icon name="search" size={12} /> Look up</button>
        </div>
      </label>
      <label className="sync-folder-field compact">
        <span>Invitation code (optional)</span>
        <div>
          <input value="" onChange={() => undefined} disabled={locked} spellCheck={false} placeholder="XXXXX-XXXXX-XXXXX-XXXXX" />
          <button type="button" className="btn" disabled><Icon name="lock" size={12} /> Join</button>
        </div>
      </label>
      <div className="sync-bundle-match-options repo-rows" role="radiogroup" aria-label="Repositories you belong to">
        <button type="button" role="radio" aria-checked className="sync-bundle-match-option active" disabled={locked}>
          <span className="sync-bundle-radio"><span /></span>
          <span className="setup-choice-mark" aria-hidden="true"><Icon name={repo.git ? "git-folder" : "folder"} size={24} /></span>
          <span className="sync-bundle-match-copy">
            <strong>{repo.address}</strong>
            <span>{repo.role}</span>
          </span>
          <span className="sync-bundle-match-meta">{repo.when}</span>
        </button>
      </div>
    </Band>
  );
}

/** The two positions of the switch; Browse picks the parent under New
 *  folder and the folder itself under Existing folder. */
function FolderSwitch({ mode, locked }: { mode: Mode; locked: boolean }) {
  const look = useContext(LookContext);
  if (look === "pills") {
    return (
      <div className="setup-pill-row" role="radiogroup" aria-label="Folder">
        {(["new", "existing"] as const).map((option) => (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={mode === option}
            className={`setup-pill${mode === option ? " active" : ""}`}
            disabled={locked}
          >
            <Icon name={option === "new" ? "plus" : "folder"} size={12} />
            <span className="setup-pill-label">{option === "new" ? "New folder" : "Existing folder"}</span>
            <span className="setup-pill-check" aria-hidden="true">{mode === option && <Icon name="check" size={12} />}</span>
          </button>
        ))}
      </div>
    );
  }
  return (
    <div className="sync-segmented" role="radiogroup" aria-label="Folder">
      {(["new", "existing"] as const).map((option) => (
        <button
          key={option}
          type="button"
          role="radio"
          aria-checked={mode === option}
          className={mode === option ? "active" : undefined}
          disabled={locked}
        >
          {option === "new" ? "New folder" : "Existing folder"}
        </button>
      ))}
    </div>
  );
}

/** The shipped Folder field's markup (`ProjectFolderField`), with the switch
 *  in its label's place and room for two ways on in the line. */
function FolderField({ frame, locked }: { frame: Frame; locked: boolean }) {
  const { line } = frame;
  return (
    <div className="sync-project-folder-field">
      {frame.mode ? (
        <>
          <label className="sync-visually-hidden" htmlFor={`${frame.id}-folder`}>Folder</label>
          <FolderSwitch mode={frame.mode} locked={locked} />
        </>
      ) : (
        <label htmlFor={`${frame.id}-folder`}>Folder</label>
      )}
      <div className="sync-profile-select-row">
        <input
          id={`${frame.id}-folder`}
          value={frame.folder}
          onChange={() => undefined}
          disabled={locked}
          placeholder="~/code/my-app"
          spellCheck={false}
          aria-invalid={line.tone === "bad" || undefined}
        />
        <button type="button" className="btn" disabled={locked}>
          <Icon name="folder" size={13} /> Browse…
        </button>
      </div>
      <p className={`sync-setup-path-hint ${line.tone}`} aria-live="polite">
        {line.text}
        {line.links?.map((label, index) => (
          <span key={label} style={{ display: "contents" }}>
            {index > 0 && <span aria-hidden="true">·</span>}
            <button type="button" className="btn-link" disabled={locked}>{label}</button>
          </span>
        ))}
      </p>
    </div>
  );
}

function RecentFolders() {
  const rows = [
    { name: "HorrorGame", path: "~/src/HorrorGame", when: "2d ago", git: true, badge: "Its checkout", providers: ["codex", "claude"], active: true },
    { name: "demo", path: "~/Desktop/project/demo", when: "Sep 20", git: false, badge: null, providers: ["codex"], active: false },
    { name: "horror-fork", path: "~/src/horror-fork", when: "Sep 12", git: true, badge: null, providers: ["claude"], active: false },
  ];
  return (
    <div className="sync-folder-choices">
      <div className="sync-folder-choices-head">Recent<small>{rows.length}</small></div>
      <div className="sync-bundle-match-options" role="radiogroup" aria-label="Recent folders">
        {rows.map((row) => (
          <button key={row.path} type="button" role="radio" aria-checked={row.active} className={`sync-bundle-match-option${row.active ? " active" : ""}`}>
            <span className="sync-bundle-radio"><span /></span>
            <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.git ? "git-folder" : "folder"} size={24} /></span>
            <span className="sync-bundle-match-copy">
              <strong>{row.name}{row.badge && <small className="sync-bundle-recommended">{row.badge}</small>}</strong>
              <span>{row.path}</span>
            </span>
            <span className="sync-bundle-match-meta">
              {row.when}
              <span className="sync-folder-choice-marks">
                {row.providers.map((provider) => (
                  <ProviderMark key={provider} provider={provider} displayName={provider === "codex" ? "Codex" : "Claude"} size={12} />
                ))}
              </span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function ProjectBand({ frame, locked }: { frame: Frame; locked: boolean }) {
  const repo = REPOS[frame.repo];
  return (
    <Band title="Project">
      <div className="sync-field-grid sync-setup-project-fields">
        <label>
          <span>Name</span>
          <input value={repo.name} readOnly disabled />
        </label>
        <FolderField frame={frame} locked={locked} />
        {frame.recent && <RecentFolders />}
      </div>
      {/* Outside the field grid, whose label and input rules would restyle
          the box's own label and checkbox — as the Repository band seats it. */}
      {frame.ack && (
        <Acknowledgement checked={false} disabled={locked} onChange={() => undefined}>
          Can’t verify this is {repo.name}. Connect it anyway.
        </Acknowledgement>
      )}
    </Band>
  );
}

function AgentBand({ locked }: { locked: boolean }) {
  return (
    <Band title="Agent">
      <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
        {PROVIDERS.map((descriptor) => (
          <div key={descriptor.key} className="sync-provider-card enabled">
            <label className="sync-provider-toggle setup-marked-toggle">
              <input type="checkbox" checked readOnly disabled={locked} aria-label={`Use ${descriptor.display_name} in this project`} />
              <ProviderMark provider={descriptor.key} displayName={descriptor.display_name} size={12} />
              <strong>{descriptor.display_name}</strong>
              <span>{descriptor.profile_label}</span>
            </label>
            <div className="sync-agent-profile-list">
              <div className="sync-agent-profile-row">
                <span className="sync-select-shell">
                  <select value={PROFILE[descriptor.key]} disabled={locked} aria-label={`${descriptor.display_name} profile`} onChange={() => undefined}>
                    <option value="">Choose profile</option>
                    <option value={PROFILE[descriptor.key]}>{PROFILE[descriptor.key]}</option>
                  </select>
                </span>
              </div>
            </div>
          </div>
        ))}
      </div>
    </Band>
  );
}

function Page({ frame }: { frame: Frame }) {
  const repo = REPOS[frame.repo];
  const locked = !!frame.status;
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section sync-project-overview">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="New project">
          {frame.full && (
            <header className="sync-modal-header">
              <div><h1>New project</h1></div>
              <button type="button" className="btn btn-ghost" aria-label="Close; nothing has been saved"><Icon name="x" size={17} /></button>
            </header>
          )}
          <div className="sync-modal-body">
            {frame.full && <Doors disabled={locked} />}
            {frame.full && <StorageBand locked={locked} />}
            {frame.full && <RepositoryBand repo={repo} locked={locked} />}
            <ProjectBand frame={frame} locked={locked} />
            {frame.full && <AgentBand locked={locked} />}
          </div>
          <footer className="sync-modal-footer sync-setup-footer">
            {frame.status && <span className="sync-setup-save-state" aria-live="polite">{frame.status}</span>}
            <div>
              <button type="button" className="btn btn-ghost" disabled={locked}>Cancel</button>
              <button type="button" className="btn btn-primary" disabled={!frame.enabled}>{frame.verb}</button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = (next: Knobs) => {
    setKnobs(next);
    const query = new URLSearchParams({ show: next.show, switch: next.look, theme: next.theme });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const shown = FRAMES.filter((frame) => shows(knobs.show, frame));
  let number = 0;
  return (
    <LookContext.Provider value={knobs.look}>
    <div className="spec">
      <h1 className="spec-title">New project › From Mallard Cloud · an existing folder</h1>

      <div className="spec-controls">
        <div className="spec-group">
          <span>Show</span>
          <select value={knobs.show} onChange={(event) => set({ ...knobs, show: event.target.value })} aria-label="Frame">
            <option value="all">All frames</option>
            {SECTIONS.map((section) => <option key={section.direction} value={section.direction}>{section.title}</option>)}
            {FRAMES.map((frame) => <option key={frame.id} value={frame.id}>{frame.direction.toUpperCase()} · {frame.title}</option>)}
          </select>
        </div>
        <div className="spec-group">
          <span>Switch</span>
          {(["pills", "segmented"] as const).map((look) => (
            <button key={look} type="button" className="spec-seg" aria-pressed={knobs.look === look} onClick={() => set({ ...knobs, look })}>
              {look === "pills" ? "Pills (Storage’s)" : "Segmented"}
            </button>
          ))}
        </div>
        <div className="spec-group">
          <span>Theme</span>
          {(["light", "dark"] as const).map((theme) => (
            <button key={theme} type="button" className="spec-seg" aria-pressed={knobs.theme === theme} onClick={() => set({ ...knobs, theme })}>
              {theme === "light" ? "Light" : "Dark"}
            </button>
          ))}
        </div>
      </div>

      <div className="spec-panel">
        <table>
          <thead>
            <tr><th>Folder</th><th>Browse picks</th><th>The verb</th><th>Gets</th></tr>
          </thead>
          <tbody>
            <tr><td><b>New folder</b></td><td>the parent; the name stays</td><td>Clone and connect · Create folder and connect</td><td>gh → git → empty folder (today)</td></tr>
            <tr><td><b>Existing folder</b></td><td>the folder itself</td><td>Connect</td><td>the folder as it is: nothing cloned, made or pulled</td></tr>
          </tbody>
        </table>
        <ul>
          <li>Existing folder keeps setup’s Git-remote rule: a folder Mallard can’t verify as this repository needs the tick. Only a new or empty folder skips it, as today.</li>
          <li>Still refused: a file, Mallard’s data, a folder that’s already a project.</li>
        </ul>
      </div>

      {SECTIONS.map((section) => {
        const frames = shown.filter((frame) => frame.direction === section.direction);
        if (frames.length === 0) return null;
        return (
          <div key={section.direction} style={{ display: "contents" }}>
            <h2 className="spec-h2">{section.title}<small>{section.sub}</small></h2>
            <div className="spec-row">
              {frames.map((frame) => {
                number += 1;
                return (
                  <div key={frame.id} data-frame={frame.id}>
                    <h3 className="spec-col-title"><small>{number}</small>{frame.title}</h3>
                    <p className="spec-col-note">{frame.note}</p>
                    <div className="spec-frame"><Page frame={frame} /></div>
                  </div>
                );
              })}
            </div>
          </div>
        );
      })}
    </div>
    </LookContext.Provider>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
