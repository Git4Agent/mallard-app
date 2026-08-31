import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import SyncPathBar from "./components/sync-review/SyncPathBar";
import SyncReviewTabs from "./components/sync-review/SyncReviewTabs";
import "./App.css";

/**
 * Mockup: the non-git project's top bar, where Sessions ⇄ Files lives.
 *
 * Today the switch is a `.sync-segmented` form control dropped into the
 * trail's `content` slot (ProjectLinksWorkspace.tsx:2249) — a muted tray with
 * a border, the same control the storage cards and Settings use. It sits
 * after a "/" separator, so a two-button pill occupies a position that every
 * other element of the trail uses to say where you are.
 *
 * Each variant below keeps the shipped bar exactly — crumb markup, menus,
 * refresh, Pull/Push, ⋯ — and changes only what the switch is.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1160px; margin: 0 auto; padding: 24px 22px 90px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 16px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; align-items: center; margin: 0 0 22px; flex-wrap: wrap; }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 5px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--bg-active); color: var(--text-0); border-color: var(--border-mid); }
  .mk-bar > span { color: var(--text-3); font-size: var(--type-xs); }
  .mk-variant { margin: 0 0 30px; }
  .mk-variant > h3 { margin: 0 0 2px; color: var(--text-0); font-size: var(--type-md); }
  .mk-variant > p { margin: 0 0 10px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-variant > .mk-note { color: var(--text-3); font-size: var(--type-xs); }
  .mk-frame { padding: 3px; border: 1px dashed var(--border-mid); border-radius: 6px; background: var(--retro-frame-gutter); }
  .mk-frame > .mk-window { border-radius: 4px; overflow: visible; background: var(--bg-0); }
  /* The page body under the bar, just enough to show the bar in context. */
  .mk-body { min-height: 86px; padding: 14px 16px; border-top: 0; color: var(--text-3); font-size: var(--type-xs); }
  .mk-body > strong { display: block; margin-bottom: 3px; color: var(--text-1); font-size: var(--type-sm); font-weight: var(--weight-semibold); }

  /* ---- variant-only styling -------------------------------------------- */

  /* P2/P4: a switch built from the bar's own chip vocabulary — the branch
     pill's border, height and chip ground — instead of the settings-form
     tray that .sync-segmented paints. */
  .mk-scope-switch { display: inline-flex; align-items: center; flex: 0 0 auto; height: 26px; border: 1px solid var(--border); border-radius: 5px; background: var(--chip-bg); overflow: hidden; }
  .mk-scope-switch > button { height: 100%; padding: 0 9px; border: 0; background: transparent; color: var(--text-2); font: inherit; font-size: var(--type-2xs); cursor: pointer; transition: background 120ms ease, color 120ms ease; }
  .mk-scope-switch > button + button { border-left: 1px solid var(--border); }
  .mk-scope-switch > button:hover:not(.active) { color: var(--text-0); background: var(--button-bg-hover); }
  .mk-scope-switch > button.active { color: var(--text-0); background: var(--bg-active); font-weight: var(--weight-semibold); }
  .mk-scope-switch > button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  /* Riding a crumb as a qualifier: no separator before it, a hair of air after
     the name it narrows — the gap the branch pill keeps. */
  .mk-crumb-scope { margin-left: 7px; }
  /* P4: in the actions run, before the divider. */
  .mk-actions-switch { margin-right: 2px; }
  /* The retired .sync-path-crumb-content, kept locally so the "Today" frame
     still shows what shipped before this change. */
  .mk-legacy-crumb-content { min-width: 0; display: inline-flex; align-items: center; }
`;

const FILE_COUNT = 14;
const SESSION_COUNT = 12;

type View = "sessions" | "files";

/* ------------------------------------------------------------ bar parts --- */

function StorageCrumb() {
  return (
    <li className="sync-path-item">
      <button type="button" className="sync-path-crumb root" aria-haspopup="menu" title="Local storage 1">
        <Icon name="drive" size={14} />
        <span>Local storage 1</span>
        <Icon name="chevron-down" size={11} className="sync-path-chevron" />
      </button>
    </li>
  );
}

function ProjectCrumb({ leaf = false, scope }: { leaf?: boolean; scope?: React.ReactNode }) {
  return (
    <li className={leaf ? "sync-path-item leaf-item" : "sync-path-item"}>
      <span className="sync-path-separator" aria-hidden="true">/</span>
      <span className={`sync-path-crumb split${leaf ? " leaf" : ""}`}>
        <button type="button" className="sync-path-crumb-link" title="testProjectNonGit (2)">
          <Icon name="folder" size={14} />
          <span>testProjectNonGit (2)</span>
        </button>
        <button type="button" className="sync-path-crumb-toggle" aria-haspopup="menu" title="Switch project" aria-label="Switch project">
          <Icon name="chevron-down" size={11} className="sync-path-chevron" />
        </button>
      </span>
      {scope}
    </li>
  );
}

function ScopeSwitch({ view, onChange, className }: {
  view: View;
  onChange: (view: View) => void;
  className?: string;
}) {
  return (
    <span className={`mk-scope-switch${className ? ` ${className}` : ""}`} role="group" aria-label="Project view">
      <button
        type="button"
        className={view === "sessions" ? "active" : undefined}
        aria-pressed={view === "sessions"}
        onClick={() => onChange("sessions")}
      >
        Sessions
      </button>
      <button
        type="button"
        className={view === "files" ? "active" : undefined}
        aria-pressed={view === "files"}
        onClick={() => onChange("files")}
      >
        Files
      </button>
    </span>
  );
}

/** What the workspace hands SyncPathBar as `actions`: the bar itself owns the
 * wrapper, the divider and the ⋯. */
function PathActionsInner() {
  return (
    <>
      <button type="button" className="sync-path-icon-action" title="Rescan files" aria-label="Rescan files">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync">
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
        </button>
        <button type="button" className="sync-path-sync">
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
          <span className="sync-path-count" aria-hidden="true">3</span>
        </button>
      </div>
    </>
  );
}

/** The same run, for the variants that hand-roll the bar around a shape
 * SyncPathBar has no slot for. */
function PathActions({ children }: { children?: React.ReactNode }) {
  return (
    <div className="sync-path-actions">
      {children}
      <PathActionsInner />
      <span className="sync-path-divider" aria-hidden="true" />
      <div className="sync-path-overflow">
        <button type="button" className="sync-path-more" title="App actions" aria-label="App actions">
          <Icon name="more" size={16} />
        </button>
      </div>
    </div>
  );
}

function Body({ view }: { view: View }) {
  return (
    <div className="mk-body">
      <strong>{view === "sessions" ? "Sessions" : "Project files"}</strong>
      {view === "sessions"
        ? `${SESSION_COUNT} sessions · the activity list renders here`
        : `${FILE_COUNT} entries · the file list renders here`}
    </div>
  );
}

/* ------------------------------------------------------------ variants --- */

/** Today: `.sync-segmented` in the trail's content slot, after a separator. */
function BarToday({ view, onChange }: { view: View; onChange: (view: View) => void }) {
  return (
    <header className="sync-pathbar sync-titlebar">
      <ol className="sync-path" aria-label="Workspace path">
        <StorageCrumb />
        <ProjectCrumb />
        <li className="sync-path-item leaf-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <span className="mk-legacy-crumb-content" aria-current="page">
            <span className="sync-segmented sync-path-view-switch" role="group" aria-label="Project view">
              <button type="button" className={view === "sessions" ? "active" : undefined} aria-pressed={view === "sessions"} onClick={() => onChange("sessions")}>Sessions</button>
              <button type="button" className={view === "files" ? "active" : undefined} aria-pressed={view === "files"} onClick={() => onChange("files")}>Files</button>
            </span>
          </span>
        </li>
      </ol>
      <PathActions />
    </header>
  );
}

/**
 * P1 — the leaf is an ordinary crumb with an ordinary menu.
 *
 * This variant drives the real SyncPathBar with the crumb config
 * ProjectLinksWorkspace now ships, so it renders the shipped code path rather
 * than an imitation of it.
 */
function BarLeafMenu({ view, onChange }: { view: View; onChange: (view: View) => void }) {
  const otherView: View = view === "files" ? "sessions" : "files";
  const viewIcon = (v: View) => (v === "files" ? "file" as const : "message" as const);
  const viewLabel = (v: View) => (v === "files" ? "Files" : "Sessions");
  const viewDetail = (v: View) => (v === "files" ? "Files outside Git" : "Agent sessions");
  return (
    <SyncPathBar
      crumbs={[
        {
          id: "storage",
          label: "Local storage 1",
          icon: "drive",
          tone: "root",
          menuLabel: "Storage",
          menuHeader: { label: "Local storage 1", detail: "~/mallard/storage-1", detailIsPath: true, icon: "drive" },
          menu: [{ items: [{ id: "storage-settings", label: "Storage settings…", icon: "settings", onSelect: () => undefined }] }],
        },
        {
          id: "project",
          label: "testProjectNonGit (2)",
          icon: "folder",
          menuLabel: "Project",
          onActivate: () => undefined,
          menuHeader: { label: "testProjectNonGit (2)", detail: "~/Desktop/testProjectNonGit", detailIsPath: true, icon: "folder" },
          menu: [{ items: [{ id: "project-settings", label: "Project setup…", icon: "settings", onSelect: () => undefined }] }],
        },
        {
          id: "section",
          label: viewLabel(view),
          icon: viewIcon(view),
          tone: "section",
          menuLabel: "Switch view",
          menuHeader: { label: viewLabel(view), detail: viewDetail(view), icon: viewIcon(view) },
          menu: [{
            label: "Switch to",
            items: [{
              id: `view-${otherView}`,
              label: viewLabel(otherView),
              detail: viewDetail(otherView),
              icon: viewIcon(otherView),
              onSelect: () => onChange(otherView),
            }],
          }],
        },
      ]}
      overflow={[{ items: [{ id: "app-settings", label: "Settings…", icon: "settings", onSelect: () => undefined }] }]}
      overflowLabel="App actions"
      actions={<PathActionsInner />}
    />
  );
}

/** P2 — the switch rides the project crumb as a scope, like the branch pill. */
function BarCrumbScope({ view, onChange }: { view: View; onChange: (view: View) => void }) {
  return (
    <header className="sync-pathbar sync-titlebar">
      <ol className="sync-path" aria-label="Workspace path">
        <StorageCrumb />
        <ProjectCrumb leaf scope={<ScopeSwitch view={view} onChange={onChange} className="mk-crumb-scope" />} />
      </ol>
      <PathActions />
    </header>
  );
}

/** P3 — the trail says where; the review tab strip says which list. */
function BarWithTabs({ view, onChange }: { view: View; onChange: (view: View) => void }) {
  return (
    <>
      <header className="sync-pathbar sync-titlebar">
        <ol className="sync-path" aria-label="Workspace path">
          <StorageCrumb />
          <ProjectCrumb leaf />
        </ol>
        <PathActions />
      </header>
      <SyncReviewTabs
        tabs={[
          { id: "sessions", label: "Sessions", icon: "message", count: SESSION_COUNT },
          { id: "files", label: "Files", icon: "file", count: FILE_COUNT },
        ]}
        activeId={view}
        onChange={(id) => onChange(id as View)}
      />
    </>
  );
}

/** P4 — the trail stays pure; the switch joins the actions run. */
function BarActionsSwitch({ view, onChange }: { view: View; onChange: (view: View) => void }) {
  return (
    <header className="sync-pathbar sync-titlebar">
      <ol className="sync-path" aria-label="Workspace path">
        <StorageCrumb />
        <ProjectCrumb leaf />
      </ol>
      <PathActions>
        <ScopeSwitch view={view} onChange={onChange} className="mk-actions-switch" />
        <span className="sync-path-divider" aria-hidden="true" />
      </PathActions>
    </header>
  );
}

/* ---------------------------------------------------------------- page --- */

const VARIANTS: Array<{
  id: string;
  title: string;
  copy: string;
  note?: string;
  render: (view: View, onChange: (view: View) => void) => React.ReactNode;
}> = [
  {
    id: "today",
    title: "Today — segmented control in the trail",
    copy: "The shipped bar, for comparison. A settings-style pill (muted tray, own border, filled active half) sits after a separator, in the position the trail uses to say where you are — so the last step of the path is a control showing two places at once rather than naming the one you are in.",
    render: (view, onChange) => <BarToday view={view} onChange={onChange} />,
  },
  {
    id: "p1",
    title: "P1 · Leaf crumb with a menu — SHIPPED",
    copy: "The leaf is an ordinary crumb: icon, name, chevron — identical in build to the storage and project crumbs, opening the bar's own menu (current view as the header, the other as a switch row under \"Switch to\", no radio state). The trail reads as a path again: storage / project / Files. This frame drives the real SyncPathBar with the crumb config ProjectLinksWorkspace now ships, so it is the shipped code path rather than an imitation of it.",
    note: "One click to see the choice, two to switch. Click the leaf to open the menu.",
    render: (view, onChange) => <BarLeafMenu view={view} onChange={onChange} />,
  },
  {
    id: "p2",
    title: "P2 · Qualifier riding the project crumb",
    copy: "The switch moves into the crumb's scope slot — the same slot the Git branch picker uses, whose contract is \"a control that narrows this crumb rather than descending from it\". No separator before it, so it reads as a qualifier on the project, not a step down the path. Restyled to the bar's chip vocabulary (chip ground, 26px, hairline divide) instead of the form-control tray.",
    note: "One click to switch. Mirrors the Git project's bar exactly: crumb + qualifier.",
    render: (view, onChange) => <BarCrumbScope view={view} onChange={onChange} />,
  },
  {
    id: "p3",
    title: "P3 · Trail says where, tabs say which",
    copy: "The trail ends at the project, and the view switch becomes the review pages' own tab strip beneath the bar — with counts and the arrow-key contract already built in. The activity page then reads exactly like the Push and Pull reviews it leads into.",
    note: "One click to switch; costs a 42px row under the bar.",
    render: (view, onChange) => <BarWithTabs view={view} onChange={onChange} />,
  },
  {
    id: "p4",
    title: "P4 · Switch in the actions run",
    copy: "The trail stays purely locational and the switch joins the right-hand run, parted from Pull and Push by the bar's existing divider — the same chip vocabulary as P2.",
    note: "One click to switch, but it puts a view control in the zone the bar reserves for verbs.",
    render: (view, onChange) => <BarActionsSwitch view={view} onChange={onChange} />,
  },
];

function Page() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [narrow, setNarrow] = useState(false);
  const [views, setViews] = useState<Record<string, View>>(() => (
    Object.fromEntries(VARIANTS.map((variant) => [variant.id, "files" as View]))
  ));
  document.documentElement.dataset.theme = theme;
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Non-git top bar — where Sessions ⇄ Files lives</h1>
      <p className="mk-lede">
        Every variant keeps the shipped bar untouched — crumb markup and menus, the
        view-scoped refresh, Pull and Push, the ⋯ — and changes only what the switch is and
        where it sits. All are interactive; the width toggle squeezes the frame to 620px so
        you can see each one under pressure.
      </p>
      <div className="mk-bar" role="group" aria-label="Mock controls">
        <button type="button" className={theme === "light" ? "on" : undefined} onClick={() => setTheme("light")}>Light</button>
        <button type="button" className={theme === "dark" ? "on" : undefined} onClick={() => setTheme("dark")}>Dark</button>
        <span>·</span>
        <button type="button" className={!narrow ? "on" : undefined} onClick={() => setNarrow(false)}>Wide</button>
        <button type="button" className={narrow ? "on" : undefined} onClick={() => setNarrow(true)}>Narrow</button>
      </div>

      {VARIANTS.map((variant) => (
        <div key={variant.id} className="mk-variant">
          <h3>{variant.title}</h3>
          <p>{variant.copy}</p>
          {variant.note && <p className="mk-note">{variant.note}</p>}
          <div className="mk-frame" style={{ maxWidth: narrow ? 620 : 980 }}>
            <div className="mk-window">
              {variant.render(views[variant.id], (next) => setViews((current) => ({ ...current, [variant.id]: next })))}
              <Body view={views[variant.id]} />
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
