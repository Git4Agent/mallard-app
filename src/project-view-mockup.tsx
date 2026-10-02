import { StrictMode, useState, type KeyboardEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "./components/Icons";
import NameSearchField from "./components/sync-review/NameSearchField";
import { fileNoun, sessionNoun } from "./components/project-sync/model";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb } from "./components/sync-review/SyncPathBar";
import "./App.css";

/**
 * Mockup: the Sessions ⇄ Files switch of a non-Git project.
 *
 * What ships today (ProjectLinksWorkspace.tsx) is two controls for one piece
 * of state: the section crumb's "Switch view" menu, and a `.sync-segmented`
 * pill on the page whose `align-self: flex-start` never takes, so it spans
 * the panel. The Pull/Push badges come from the Sessions page's header model
 * alone, so they vanish on Files.
 *
 *   Before  what ships
 *   A       tab view above the panel — the review's underline tabs, with counts
 *   B       A, with search sharing the tab row
 *   C       toolbar segmented control riding the project crumb
 *   D       minimal fix: the same pill, hugged, on the search row
 *
 * `npm run dev`, then /project-view-mockup.html
 * (`?pending=both|sessions|files|none&width=wide|narrow&theme=light|dark`).
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1120px; margin: 0 auto; padding: 24px 22px 90px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 14px; max-width: 94ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; align-items: center; margin: 0 0 22px; flex-wrap: wrap; }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 5px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--bg-active); color: var(--text-0); border-color: var(--border-mid); }
  .mk-bar > span { color: var(--text-3); font-size: var(--type-xs); }
  .mk-variant { margin: 0 0 34px; }
  .mk-variant > h3 { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-md); }
  .mk-variant > p { margin: 0 0 6px; max-width: 94ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-variant > ul { margin: 0 0 10px; padding-left: 18px; max-width: 94ch; color: var(--text-2); font-size: var(--type-xs); line-height: var(--leading-prose); }
  .mk-variant > ul b { color: var(--text-0); }
  .mk-tag { display: inline-block; margin-left: 8px; padding: 1px 7px; border-radius: var(--r-pill); background: var(--surface-row-active); color: var(--text-0); font-size: var(--type-2xs); font-weight: var(--weight-semibold); vertical-align: 2px; }
  .mk-frame { display: grid; gap: 12px; padding: 14px; border: 1px dashed var(--border-mid); border-radius: 10px; background: var(--bg-0); }

  /* The page under the bar: the workspace panel, just enough to show the
     switch in the place it lives. */
  .mk-page { min-height: 300px; padding: 22px 26px 28px; border: var(--box-border); border-radius: 22px; background: var(--surface-card); box-shadow: var(--shadow-control); }
  .mk-page > * + * { margin-top: 14px; }
  .mk-tile-head { display: flex; align-items: center; gap: 8px; color: var(--text-1); font-size: var(--type-sm); }
  .mk-tile-head strong { font-family: var(--font-display); font-weight: var(--weight-heavy); }
  .mk-list { border: var(--box-border); border-radius: 14px; overflow: hidden; background: var(--surface-row); box-shadow: var(--shadow-control); }
  .mk-row { display: grid; grid-template-columns: 22px 1fr auto; gap: 4px 10px; align-items: center; padding: 11px 14px; }
  .mk-row + .mk-row { border-top: 1px solid var(--border-mid); }
  .mk-row > strong { color: var(--text-0); font-family: var(--font-display); font-size: var(--type-sm); font-weight: var(--weight-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mk-row > .mk-meta { grid-column: 2; display: flex; gap: 12px; color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); }
  .mk-row > .mk-meta span { display: inline-flex; gap: 4px; align-items: center; }
  .mk-row > time { grid-column: 3; color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); text-align: right; }
  .mk-row > .mk-state { color: var(--text-2); }
  .mk-row > .mk-state.pull, .mk-row > .mk-state.push { color: var(--warning-text); }

  /* Today: the pill as it renders — a block flex box, so it fills the row. */

  /* A/B: the review's underline strip, seated on the page. The review pads it
     for its own header; on the page it starts at the content edge. */
  .mk-view-tabs.sync-sync-review-tabs { padding: 0; gap: 20px; }
  .mk-view-tabs > button { padding: 0 2px; }
  .mk-view-tabs > button > .mk-dot { width: 7px; height: 7px; margin-left: -2px; border-radius: 50%; background: var(--yellow); box-shadow: 0 0 0 1.5px var(--ink-line); }
  .mk-view-tabs > button > small.mk-wait { min-width: 18px; color: transparent; background: var(--surface-muted); }

  /* B: one band — the tab row carries the panel's search at its end, the way
     a VS Code panel carries its filter beside its tabs. */
  .mk-tabrow { display: flex; align-items: flex-end; gap: 16px; border-bottom: 1px solid var(--border-mid); }
  .mk-tabrow > .mk-view-tabs { width: auto; border-bottom: 0; }
  .mk-tabrow-search { margin-left: auto; margin-bottom: 7px; display: flex; align-items: center; gap: 7px; width: min(300px, 45%); height: 30px; padding: 0 10px; border: 1px solid var(--border-mid); border-radius: var(--r-pill); background: var(--surface-row); color: var(--text-2); }
  .mk-tabrow-search input { min-width: 0; flex: 1; border: 0; outline: 0; background: transparent; color: var(--text-0); font: inherit; font-size: var(--type-xs); }
  .mk-tabrow-search small { font-family: var(--font-mono); font-size: var(--type-3xs); white-space: nowrap; }

  /* C: a toolbar segmented control in the bar's chip vocabulary (the branch
     pill's height and ground), riding the project crumb as its qualifier. */
  .mk-scope-switch { display: inline-flex; align-items: center; flex: 0 0 auto; height: 28px; margin-left: 8px; padding: 2px; border: var(--box-border); border-radius: var(--r-pill); background: var(--button-bg); box-shadow: var(--shadow-control); }
  .mk-scope-switch > button { position: relative; height: 100%; display: inline-flex; align-items: center; gap: 5px; padding: 0 10px; border: 0; border-radius: var(--r-pill); background: transparent; color: var(--text-2); font: inherit; font-family: var(--font-display); font-size: var(--type-2xs); font-weight: var(--weight-medium); cursor: pointer; }
  .mk-scope-switch > button.active { color: var(--text-0); background: var(--surface-row-active); }
  .mk-scope-switch .mk-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--yellow); box-shadow: 0 0 0 1.5px var(--ink-line); }
  .mk-narrow .mk-scope-switch > button > span.mk-scope-label { display: none; }

  /* D: the shipped pill, hugged, on the search row. */
  .mk-searchrow { display: flex; align-items: center; gap: 14px; }
  .mk-searchrow > .sync-segmented { flex: 0 0 auto; }
  .mk-searchrow > .sync-segmented button { display: inline-flex; align-items: center; gap: 6px; }
  .mk-searchrow > .sync-name-search { flex: 1; min-width: 0; }
`;

const STORAGE = "Mallard Cloud Rika321";
const PROJECT = "mallardNonGit";

type View = "sessions" | "files";
type Pending = "both" | "sessions" | "files" | "none";
type Width = "wide" | "narrow";
type Theme = "light" | "dark";

/* ---------------------------------------------------------------- data --- */

interface SessionRow { title: string; provider: "claude" | "codex"; messages: number; tools: number; tokens: string; duration: string; at: string }
interface FileRow { path: string; size: string; at: string; state: "pull" | "push" | "synced" }

const SESSIONS: SessionRow[] = [
  { title: "Pelican and mallard racing SVG", provider: "claude", messages: 7, tools: 42, tokens: "4.80M", duration: "2h 48m", at: "Oct 1, 2026 at 14:59" },
  { title: "Create pelican cycling GIF", provider: "codex", messages: 4, tools: 12, tokens: "863K", duration: "7m", at: "Oct 1, 2026 at 12:10" },
  { title: "Create pelican and mallard SVG", provider: "codex", messages: 4, tools: 14, tokens: "794K", duration: "6m", at: "Oct 1, 2026 at 12:07" },
];

const FILES: FileRow[] = [
  { path: "assets/pelican-race.svg", size: "48 KB", at: "Oct 1, 2026 at 14:58", state: "pull" },
  { path: "assets/pelican-cycle.gif", size: "1.2 MB", at: "Oct 1, 2026 at 12:09", state: "pull" },
  { path: "notes/ideas.md", size: "3 KB", at: "Oct 1, 2026 at 15:20", state: "push" },
  { path: "README.md", size: "2 KB", at: "Sep 29, 2026 at 09:41", state: "synced" },
  { path: "scripts/render.sh", size: "1 KB", at: "Sep 29, 2026 at 09:40", state: "synced" },
];

interface FaceCounts { items: number; pull: number; push: number }

function faceCounts(pending: Pending): Record<View, FaceCounts> {
  const sessionsPending = pending === "both" || pending === "sessions";
  const filesPending = pending === "both" || pending === "files";
  return {
    sessions: { items: SESSIONS.length, pull: sessionsPending ? 3 : 0, push: 0 },
    files: { items: FILES.length, pull: filesPending ? 2 : 0, push: filesPending ? 1 : 0 },
  };
}

const VIEW_ICON: Record<View, IconName> = { sessions: "message", files: "file" };
const VIEW_LABEL: Record<View, string> = { sessions: "Sessions", files: "Files" };
const VIEWS: readonly View[] = ["sessions", "files"];

function pendingPhrase(counts: FaceCounts): string | null {
  const parts = [
    counts.pull > 0 ? `${counts.pull} to pull` : null,
    counts.push > 0 ? `${counts.push} to push` : null,
  ].filter(Boolean);
  return parts.length > 0 ? parts.join(", ") : null;
}

/* ------------------------------------------------------------ bar parts --- */

function storageCrumb(): PathBarCrumb {
  return {
    id: "storage",
    label: STORAGE,
    icon: "cloud",
    tone: "root",
    menuLabel: "Storage",
    menuHeader: { label: STORAGE, detail: "Mallard Cloud", icon: "cloud" },
    menu: [{ items: [{ id: "storage-settings", label: "Storage settings…", icon: "settings", onSelect: () => undefined }] }],
  };
}

function projectCrumb(scope?: ReactNode): PathBarCrumb {
  return {
    id: "project",
    label: PROJECT,
    icon: "folder",
    menuLabel: "Project",
    menuHeader: { label: PROJECT, detail: "~/Desktop/mallardNonGit", detailIsPath: true, icon: "folder" },
    menu: [{ items: [{ id: "project-settings", label: "Project setup…", icon: "settings", onSelect: () => undefined }] }],
    scope,
  };
}

/** Today's section crumb: names the face, offers the other in its menu. */
function sectionCrumb(view: View, onChange: (view: View) => void): PathBarCrumb {
  const other: View = view === "files" ? "sessions" : "files";
  const detail = (v: View) => (v === "files" ? "Files outside Git" : "Agent sessions");
  return {
    id: "section",
    label: VIEW_LABEL[view],
    icon: VIEW_ICON[view],
    tone: "section",
    menuLabel: "Switch view",
    menuHeader: { label: VIEW_LABEL[view], detail: detail(view), icon: VIEW_ICON[view] },
    menu: [{
      label: "Switch to",
      items: [{ id: `view-${other}`, label: VIEW_LABEL[other], detail: detail(other), icon: VIEW_ICON[other], onSelect: () => onChange(other) }],
    }],
  };
}

function BarActions({ pull, push }: { pull: number; push: number }) {
  return (
    <>
      <button type="button" className="sync-path-icon-action" title="Mallard Cloud is connected" aria-label="Mallard Cloud is connected">
        <Icon name="cloud-check" size={16} />
      </button>
      <button type="button" className="sync-path-icon-action" title="Refresh" aria-label="Refresh">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync">
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
          {pull > 0 && <span className="sync-path-count" aria-hidden="true">{pull}</span>}
        </button>
        <button type="button" className="sync-path-sync">
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
          {push > 0 && <span className="sync-path-count" aria-hidden="true">{push}</span>}
        </button>
      </div>
    </>
  );
}

function Bar({ crumbs, pull, push }: { crumbs: PathBarCrumb[]; pull: number; push: number }) {
  return (
    <SyncPathBar
      crumbs={crumbs}
      overflow={[{ items: [{ id: "app-settings", label: "Settings…", icon: "settings", onSelect: () => undefined }] }]}
      overflowLabel="App actions"
      actions={<BarActions pull={pull} push={push} />}
    />
  );
}

/* ----------------------------------------------------------- switches --- */

/** The arrow/Home/End contract every tablist here keeps. */
function tabKeys(event: KeyboardEvent<HTMLButtonElement>, view: View, onChange: (view: View) => void) {
  const index = VIEWS.indexOf(view);
  const next = event.key === "ArrowRight" ? VIEWS[(index + 1) % VIEWS.length]
    : event.key === "ArrowLeft" ? VIEWS[(index - 1 + VIEWS.length) % VIEWS.length]
      : event.key === "Home" ? VIEWS[0]
        : event.key === "End" ? VIEWS[VIEWS.length - 1]
          : null;
  if (!next) return;
  event.preventDefault();
  const strip = event.currentTarget.parentElement;
  onChange(next);
  window.requestAnimationFrame(() => strip?.querySelector<HTMLButtonElement>(`[data-view="${next}"]`)?.focus());
}

/** Before: the shipped `.sync-segmented` pill, as it renders on the page. */
function ShippedPill({ view, onChange, hug = false }: { view: View; onChange: (view: View) => void; hug?: boolean }) {
  return (
    <div className={`sync-segmented${hug ? "" : " sync-project-view-tabs"}`} role="tablist" aria-label="Project view">
      {VIEWS.map((id) => (
        <button
          key={id}
          type="button"
          data-view={id}
          role="tab"
          aria-selected={view === id}
          tabIndex={view === id ? 0 : -1}
          className={view === id ? "active" : undefined}
          onClick={() => onChange(id)}
          onKeyDown={(event) => tabKeys(event, view, onChange)}
        >
          <Icon name={VIEW_ICON[id]} size={13} />
          <span>{VIEW_LABEL[id]}</span>
        </button>
      ))}
    </div>
  );
}

/** A/B: the review's underline tabs — item count, and a dot where changes wait. */
function ViewTabs({ view, onChange, counts, loadingFiles }: {
  view: View;
  onChange: (view: View) => void;
  counts: Record<View, FaceCounts>;
  loadingFiles: boolean;
}) {
  return (
    <div className="sync-project-tabs sync-sync-review-tabs mk-view-tabs" role="tablist" aria-label="Project view">
      {VIEWS.map((id) => {
        const pending = pendingPhrase(counts[id]);
        const loading = id === "files" && loadingFiles;
        return (
          <button
            key={id}
            type="button"
            data-view={id}
            role="tab"
            aria-selected={view === id}
            aria-label={`${VIEW_LABEL[id]}, ${loading ? "counting" : `${counts[id].items}`}${pending ? `, ${pending}` : ""}`}
            title={pending ?? undefined}
            tabIndex={view === id ? 0 : -1}
            className={view === id ? "active" : undefined}
            onClick={() => onChange(id)}
            onKeyDown={(event) => tabKeys(event, view, onChange)}
          >
            <Icon name={VIEW_ICON[id]} size={14} />
            <span>{VIEW_LABEL[id]}</span>
            {loading ? <small className="mk-wait">0</small> : <small>{counts[id].items}</small>}
            {pending && !loading && <span className="mk-dot" aria-hidden="true" />}
          </button>
        );
      })}
    </div>
  );
}

/** C: a toolbar segmented control, sized to the bar. */
function ScopeSwitch({ view, onChange, counts }: { view: View; onChange: (view: View) => void; counts: Record<View, FaceCounts> }) {
  return (
    <span className="mk-scope-switch" role="tablist" aria-label="Project view">
      {VIEWS.map((id) => {
        const pending = pendingPhrase(counts[id]);
        return (
          <button
            key={id}
            type="button"
            data-view={id}
            role="tab"
            aria-selected={view === id}
            aria-label={`${VIEW_LABEL[id]}${pending ? `, ${pending}` : ""}`}
            title={pending ? `${VIEW_LABEL[id]} — ${pending}` : VIEW_LABEL[id]}
            tabIndex={view === id ? 0 : -1}
            className={view === id ? "active" : undefined}
            onClick={() => onChange(id)}
            onKeyDown={(event) => tabKeys(event, view, onChange)}
          >
            <Icon name={VIEW_ICON[id]} size={13} />
            <span className="mk-scope-label">{VIEW_LABEL[id]}</span>
            {pending && <span className="mk-dot" aria-hidden="true" />}
          </button>
        );
      })}
    </span>
  );
}

/* ---------------------------------------------------------------- body --- */

function SessionsList({ pending }: { pending: boolean }) {
  return (
    <>
      <div className="mk-tile-head">
        <Icon name={pending ? "cloud" : "check-circle"} size={15} />
        {pending ? <span><strong>Only in</strong> {STORAGE}</span> : <strong>In both places</strong>}
      </div>
      <div className="mk-list">
        {SESSIONS.map((session) => (
          <div key={session.title} className="mk-row">
            <span className={`mk-state${pending ? " pull" : ""}`}><Icon name={pending ? "download" : "check-circle"} size={15} /></span>
            <strong>{session.title}</strong>
            <ProviderMark provider={session.provider} displayName={session.provider === "claude" ? "Claude" : "Codex"} size={16} />
            <span className="mk-meta">
              <span><Icon name="message" size={12} />{session.messages}</span>
              <span><Icon name="tool" size={12} />{session.tools}</span>
              <span><Icon name="token" size={12} />{session.tokens}</span>
              <span><Icon name="clock" size={12} />{session.duration}</span>
            </span>
            <time>{session.at}</time>
          </div>
        ))}
      </div>
    </>
  );
}

function FilesList({ pending }: { pending: boolean }) {
  const rows = FILES.map((file) => ({ ...file, state: pending ? file.state : "synced" as const }));
  return (
    <>
      <div className="mk-tile-head">
        <Icon name="file" size={15} />
        <strong>Files outside Git</strong>
      </div>
      <div className="mk-list">
        {rows.map((file) => (
          <div key={file.path} className="mk-row">
            <span className={`mk-state ${file.state}`}>
              <Icon name={file.state === "pull" ? "download" : file.state === "push" ? "upload" : "check-circle"} size={15} />
            </span>
            <strong>{file.path}</strong>
            <span />
            <span className="mk-meta"><span>{file.size}</span></span>
            <time>{file.at}</time>
          </div>
        ))}
      </div>
    </>
  );
}

function Body({ view, counts }: { view: View; counts: Record<View, FaceCounts> }) {
  const pending = counts[view].pull + counts[view].push > 0;
  return view === "sessions" ? <SessionsList pending={pending} /> : <FilesList pending={pending} />;
}

function Search({ view, counts }: { view: View; counts: Record<View, FaceCounts> }) {
  const [query, setQuery] = useState("");
  const total = counts[view].items;
  return (
    <NameSearchField noun={view === "sessions" ? sessionNoun : fileNoun}
      query={query}
      onChange={setQuery}
      label={view === "sessions" ? "Search loaded sessions" : "Search files"}
      matches={total}
      total={total}
    />
  );
}

/* ------------------------------------------------------------ variants --- */

interface FrameProps { view: View; onChange: (view: View) => void; counts: Record<View, FaceCounts>; loadingFiles: boolean }

function totals(counts: Record<View, FaceCounts>) {
  return {
    pull: counts.sessions.pull + counts.files.pull,
    push: counts.sessions.push + counts.files.push,
  };
}

function Before({ view, onChange, counts }: FrameProps) {
  // Today the badges come from the Sessions page alone and null on unmount.
  const pull = view === "sessions" ? counts.sessions.pull : 0;
  const push = view === "sessions" ? counts.sessions.push : 0;
  return (
    <>
      <Bar crumbs={[storageCrumb(), projectCrumb(), sectionCrumb(view, onChange)]} pull={pull} push={push} />
      <div className="mk-page">
        <ShippedPill view={view} onChange={onChange} />
        <Search view={view} counts={counts} />
        <Body view={view} counts={counts} />
      </div>
    </>
  );
}

function TabView({ view, onChange, counts, loadingFiles }: FrameProps) {
  const { pull, push } = totals(counts);
  return (
    <>
      <Bar crumbs={[storageCrumb(), projectCrumb()]} pull={pull} push={push} />
      <div className="mk-page">
        <ViewTabs view={view} onChange={onChange} counts={counts} loadingFiles={loadingFiles} />
        <Search view={view} counts={counts} />
        <Body view={view} counts={counts} />
      </div>
    </>
  );
}

function TabRowSearch({ view, onChange, counts, loadingFiles }: FrameProps) {
  const { pull, push } = totals(counts);
  return (
    <>
      <Bar crumbs={[storageCrumb(), projectCrumb()]} pull={pull} push={push} />
      <div className="mk-page">
        <div className="mk-tabrow">
          <ViewTabs view={view} onChange={onChange} counts={counts} loadingFiles={loadingFiles} />
          <label className="mk-tabrow-search">
            <Icon name="search" size={13} />
            <input type="search" placeholder={view === "sessions" ? "Search sessions…" : "Search files…"} aria-label={`Search ${view}`} />
          </label>
        </div>
        <Body view={view} counts={counts} />
      </div>
    </>
  );
}

function ToolbarSegmented({ view, onChange, counts }: FrameProps) {
  const { pull, push } = totals(counts);
  return (
    <>
      <Bar crumbs={[storageCrumb(), projectCrumb(<ScopeSwitch view={view} onChange={onChange} counts={counts} />)]} pull={pull} push={push} />
      <div className="mk-page">
        <Search view={view} counts={counts} />
        <Body view={view} counts={counts} />
      </div>
    </>
  );
}

function MinimalFix({ view, onChange, counts }: FrameProps) {
  const { pull, push } = totals(counts);
  const [query, setQuery] = useState("");
  return (
    <>
      <Bar crumbs={[storageCrumb(), projectCrumb()]} pull={pull} push={push} />
      <div className="mk-page">
        <div className="mk-searchrow">
          <ShippedPill view={view} onChange={onChange} hug />
          <NameSearchField noun={view === "sessions" ? sessionNoun : fileNoun}
            query={query}
            onChange={setQuery}
            label={view === "sessions" ? "Search loaded sessions" : "Search files"}
            matches={counts[view].items}
            total={counts[view].items}
          />
        </div>
        <Body view={view} counts={counts} />
      </div>
    </>
  );
}

const VARIANTS: Array<{
  id: string;
  title: string;
  tag?: string;
  copy: ReactNode;
  points: ReactNode[];
  render: (props: FrameProps) => ReactNode;
}> = [
  {
    id: "before",
    title: "Before: crumb menu and full-width pill",
    copy: "What ships. Two controls hold one piece of state, and the pill spans the panel with the same ink border and cast shadow as the cards. Switch to Files and the Pull/Push badges disappear, because only the Sessions page reports them.",
    points: [
      <><b>Redundant:</b> the trail’s “Sessions ⌄” menu and the pill switch the same view.</>,
      <><b>Wrong weight:</b> a full-width pill reads like a field or a card header, not a tab.</>,
      <><b>Blind:</b> nothing says which view holds the 3 to pull, and on Files the badges go away.</>,
    ],
    render: (props) => <Before {...props} />,
  },
  {
    id: "a",
    title: "A: Tabs above the panel",
    tag: "Recommended",
    copy: "The trail stops at the project, and the page opens with the underline tabs that Push and Pull review already use. Each tab shows how many items it holds, and a dot marks the view that has something to pull or push. Pull and Push stay project-wide and count both views.",
    points: [
      <><b>Matches the guides:</b> Apple HIG (macOS) says to use a tab view in the main window area, not a segmented control. Primer says to use UnderlinePanels when tabs swap content in place.</>,
      <><b>Consistent:</b> same strip, nouns, and counters as the review the user lands in after pressing Pull.</>,
      <><b>Costs:</b> one 44px row, which the pill already took. Counts for the hidden view must load too (shown here as a placeholder while it counts).</>,
    ],
    render: (props) => <TabView {...props} />,
  },
  {
    id: "b",
    title: "B: Tabs with search on the same row",
    copy: "A, plus the panel’s search moves to the right end of the tab row, the way a VS Code panel puts its filter next to its tabs. The tabs, search, and divider take one row instead of two.",
    points: [
      <><b>Denser:</b> the list starts about 50px higher.</>,
      <><b>Costs:</b> the search box shrinks and loses its “3 items” readout. Both pages have to share one search state, which means moving search out of the two pages and into the workspace.</>,
    ],
    render: (props) => <TabRowSearch {...props} />,
  },
  {
    id: "c",
    title: "C: Segmented control in the toolbar",
    copy: "A compact segmented control on the project crumb, in the same slot the Git branch picker uses. Apple HIG allows a segmented control for switching views in a toolbar. The page starts directly with search.",
    points: [
      <><b>Most compact:</b> no row is spent on the switch.</>,
      <><b>Costs:</b> no room for counts, only dots. It crowds the bar, and at narrow widths it drops to icons only. This is close to P2 in the earlier path-bar mockup.</>,
    ],
    render: (props) => <ToolbarSegmented {...props} />,
  },
  {
    id: "d",
    title: "D: Smallest change",
    copy: "Keep the pill, but make it only as wide as its two buttons and put it at the start of the search row. Remove the crumb menu. This fixes the redundancy and the stretching, but nothing else.",
    points: [
      <><b>Smallest diff:</b> no new primitive.</>,
      <><b>Costs:</b> it is still a segmented control in the main area (against HIG and Primer), it still shows no counts, and it looks different from the review tabs.</>,
    ],
    render: (props) => <MinimalFix {...props} />,
  },
];

/* ---------------------------------------------------------------- page --- */

function readKnobs(): { pending: Pending; width: Width; theme: Theme } {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    pending: pick("pending", ["both", "sessions", "files", "none"] as const, "both"),
    width: pick("width", ["wide", "narrow"] as const, "wide"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

function Page() {
  const initial = readKnobs();
  const [theme, setTheme] = useState<Theme>(initial.theme);
  const [width, setWidth] = useState<Width>(initial.width);
  const [pending, setPending] = useState<Pending>(initial.pending);
  const [loadingFiles, setLoadingFiles] = useState(false);
  const [views, setViews] = useState<Record<string, View>>(() => (
    Object.fromEntries(VARIANTS.map((variant) => [variant.id, "sessions" as View]))
  ));
  document.documentElement.dataset.theme = theme;
  const counts = faceCounts(pending);
  const knob = <T extends string>(value: T, current: T, set: (value: T) => void, label: string) => (
    <button type="button" className={value === current ? "on" : undefined} onClick={() => set(value)}>{label}</button>
  );
  return (
    <div className={`mk${width === "narrow" ? " mk-narrow" : ""}`}>
      <style>{SHELL}</style>
      <h1 className="mk-title">Sessions / Files: switching views in a non-Git project</h1>
      <p className="mk-lede">
        Each frame uses the real path bar, search field, and app styles. Only the switch and its
        position change between frames. Every frame is clickable, and the tabs support arrow keys.
        Use the controls below to change which view has pending changes and to check narrow
        widths and dark mode.
      </p>
      <div className="mk-bar" role="group" aria-label="Mock controls">
        <span>Changes in</span>
        {knob("both", pending, setPending, "Both")}
        {knob("sessions", pending, setPending, "Sessions only")}
        {knob("files", pending, setPending, "Files only")}
        {knob("none", pending, setPending, "Neither")}
        <span>·</span>
        <button type="button" className={loadingFiles ? "on" : undefined} onClick={() => setLoadingFiles((on) => !on)}>Files still counting</button>
        <span>·</span>
        {knob("wide", width, setWidth, "Wide")}
        {knob("narrow", width, setWidth, "Narrow")}
        <span>·</span>
        {knob("light", theme, setTheme, "Light")}
        {knob("dark", theme, setTheme, "Dark")}
      </div>

      {VARIANTS.map((variant) => (
        <section key={variant.id} className="mk-variant" id={`variant-${variant.id}`}>
          <h3>{variant.title}{variant.tag && <span className="mk-tag">{variant.tag}</span>}</h3>
          <p>{variant.copy}</p>
          <ul>{variant.points.map((point, index) => <li key={index}>{point}</li>)}</ul>
          <div className="mk-frame" style={{ maxWidth: width === "narrow" ? 640 : 1060 }}>
            {variant.render({
              view: views[variant.id],
              onChange: (next) => setViews((current) => ({ ...current, [variant.id]: next })),
              counts,
              loadingFiles,
            })}
          </div>
        </section>
      ))}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
