import { StrictMode, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import SyncReviewTabs from "./components/sync-review/SyncReviewTabs";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import "./App.css";

/**
 * Mockup: the files review's selection controls, redesigned.
 *
 * What ships today spends one icon on three meanings — the check-circle is
 * "use the recommended selection" in the review header, "select every file in
 * this folder" on a band, and "Select safe additions" in the toolbar — and
 * pairs each with a trash that means "deselect" on a screen where a card's
 * trash means "remove from storage". Three counters count three scopes.
 *
 * Both options below rest on one move: selection state is shown and driven by
 * the same checkbox the rows already wear — a tri-state box on each folder
 * band (all / some / none, click to toggle) — and every smart selection is
 * NAMED in words. They differ only in what the review header carries.
 *
 * Everything is live: tick rows, tick bands, use the header controls. Static
 * data, shipped classes, real leaf components — dev server only.
 */

const STORAGE = "Vault";

interface MockFile {
  id: string;
  path: string;
  directory: string;
  op: "Add file" | "Replace file · keeps local copy" | "Delete file";
  size: string;
  stamp: string;
}

/* The screenshot's shape: four pending files, two of them safe additions. */
const FILES: MockFile[] = [
  { id: "readme", path: "README.md", directory: "", op: "Replace file · keeps local copy", size: "4.1 KB", stamp: "Aug 30, 2026 at 00:19" },
  { id: "notes", path: "docs/notes.md", directory: "docs", op: "Add file", size: "12 KB", stamp: "Aug 30, 2026 at 00:11" },
  { id: "rollout", path: "docs/rollout.md", directory: "docs", op: "Add file", size: "6.1 KB", stamp: "Aug 30, 2026 at 01:39" },
  { id: "deploy", path: "scripts/deploy.sh", directory: "scripts", op: "Replace file · keeps local copy", size: "2.4 KB", stamp: "Aug 28, 2026 at 08:03" },
];

const SAFE_IDS = new Set(FILES.filter((file) => file.op === "Add file").map((file) => file.id));
const GROUPS = ["", "docs", "scripts"].map((directory) => ({
  directory,
  files: FILES.filter((file) => file.directory === directory),
}));

const fileName = (path: string) => path.split("/").slice(-1)[0];

function useSelection(initial: string[]) {
  const [selected, setSelected] = useState<Set<string>>(() => new Set(initial));
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
  const setMany = (ids: string[], on: boolean) => setSelected((current) => {
    const next = new Set(current);
    ids.forEach((id) => (on ? next.add(id) : next.delete(id)));
    return next;
  });
  return { selected, setSelected, toggle, setMany };
}

// ---------------------------------------------------------------------------
// Shipped fragments: the file card and the review chrome
// ---------------------------------------------------------------------------

function FileCard({ file, checked, onToggle }: { file: MockFile; checked: boolean; onToggle: () => void }) {
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <label className="sync-sync-row-choice" title={`${checked ? "Exclude" : "Apply"} ${file.path}`}>
            <input type="checkbox" checked={checked} aria-label={`Apply ${file.path}`} onChange={onToggle} />
          </label>
          <span className="sync-thread-sync-indicator storage" data-tooltip={`Only in ${STORAGE}. Pull to download it here.`} tabIndex={0}>
            <ThreadStatusIcon status="cloud-outstanding" size={18} />
          </span>
          <strong title={file.path}>{fileName(file.path)}</strong>
          <span className="sync-history-branch-state history-only">{file.op}</span>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${file.path} details`}>
            <span className="sync-history-thread-metric" data-tooltip={`In ${STORAGE} · ${file.size}`} tabIndex={0}>
              <Icon name="drive" size={12} /><b>{file.size}</b>
            </span>
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <div className="sync-history-debug-menu">
              <button type="button" className="btn btn-ghost sync-history-session-detail-action" title={`Details for ${file.path}`}>
                <Icon name="info" size={13} />
              </button>
            </div>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`Stored ${file.stamp}`} tabIndex={0}>
            <time>{file.stamp}</time>
          </span>
        </div>
      </div>
    </article>
  );
}

function ReviewShell({ headerCluster, toolbarExtra, children }: {
  headerCluster: ReactNode;
  toolbarExtra?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace mock-review">
      <header className="sync-inline-action-header sync-sync-review-header">
        <div className="sync-sync-review-title"><h2>Pull review</h2></div>
        {headerCluster}
      </header>
      <SyncReviewTabs
        tabs={[
          { id: "history", label: "Sessions", icon: "message", count: 2 },
          { id: "project_files", label: "Project files", icon: "file", count: 4 },
        ]}
        activeId="project_files"
        onChange={() => undefined}
      />
      <div className="sync-inline-action-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <section className="sync-project-files-page" aria-label="Project files">
            <div className="sync-project-files-search">
              <input type="search" placeholder="Search files…" aria-label="Search project file paths" readOnly />
              <span>4 entries</span>
              {toolbarExtra}
            </div>
            {children}
          </section>
        </div>
      </div>
      <footer className="sync-inline-action-footer sync-sync-review-footer">
        <button type="button" className="btn btn-ghost">‹ Sessions</button>
        <button type="button" className="btn btn-primary sync-pull-apply-button">
          <ThreadStatusIcon status="cloud-outstanding" size={16} />
          Pull
        </button>
      </footer>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Panel 1 — what ships today
// ---------------------------------------------------------------------------

function TodayBand({ directory, picked, total, onAll, onNone }: {
  directory: string; picked: number; total: number; onAll: () => void; onNone: () => void;
}) {
  return (
    <div className="sync-project-files-group-heading">
      <Icon name="folder" size={14} />
      <strong>{directory}/</strong>
      <small>{total} file{total === 1 ? "" : "s"}</small>
      <div className="sync-push-resource-actions">
        <span className="sync-sync-review-selected" style={{ "--review-count-digits": 1 } as CSSProperties}>
          <b>{picked}</b>/{total}
        </span>
        <button type="button" className="btn btn-ghost sync-sync-review-icon-action" title={`Select all files in ${directory}/`} disabled={picked === total} onClick={onAll}>
          <Icon name="check-circle" size={15} />
        </button>
        <button type="button" className="btn btn-ghost sync-sync-review-icon-action" title={`Clear the ${directory}/ selection`} disabled={picked === 0} onClick={onNone}>
          <Icon name="trash" size={15} />
        </button>
      </div>
    </div>
  );
}

function PanelToday() {
  const state = useSelection(["notes"]);
  return (
    <ReviewShell
      headerCluster={(
        <div className="sync-push-resource-actions">
          <span className="sync-sync-review-selected" style={{ "--review-count-digits": 1 } as CSSProperties}>
            <b>{state.selected.size}</b>/4
          </span>
          <button type="button" className="btn btn-ghost sync-sync-review-icon-action" title="Use the recommended selection">
            <Icon name="check-circle" size={15} />
          </button>
          <button
            type="button"
            className="btn btn-ghost sync-sync-review-icon-action"
            title="Clear current selection"
            disabled={state.selected.size === 0}
            onClick={() => state.setSelected(new Set())}
          >
            <Icon name="trash" size={15} />
          </button>
          <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label="Close pull review">
            <Icon name="x" size={15} />
          </button>
        </div>
      )}
      toolbarExtra={(
        <button
          type="button"
          className="btn btn-ghost"
          onClick={() => state.setMany([...SAFE_IDS], true)}
        >
          <Icon name="check-circle" size={14} /> Select safe additions
        </button>
      )}
    >
      {GROUPS.map((group) => {
        const picked = group.files.filter((file) => state.selected.has(file.id)).length;
        return (
          <div key={group.directory || "(root)"}>
            {group.directory !== "" && (
              <TodayBand
                directory={group.directory}
                picked={picked}
                total={group.files.length}
                onAll={() => state.setMany(group.files.map((file) => file.id), true)}
                onNone={() => state.setMany(group.files.map((file) => file.id), false)}
              />
            )}
            <div className="sync-history-thread-list flat">
              {group.files.map((file) => (
                <FileCard key={file.id} file={file} checked={state.selected.has(file.id)} onToggle={() => state.toggle(file.id)} />
              ))}
            </div>
          </div>
        );
      })}
    </ReviewShell>
  );
}

// ---------------------------------------------------------------------------
// Shared by both options: the tri-state folder checkbox
// ---------------------------------------------------------------------------

function BandChoice({ directory, picked, total, onToggle }: {
  directory: string; picked: number; total: number; onToggle: (on: boolean) => void;
}) {
  const all = picked === total && total > 0;
  const some = picked > 0 && picked < total;
  return (
    <label
      className="sync-sync-row-choice"
      title={all ? `Exclude every file in ${directory}/` : `Apply every file in ${directory}/`}
    >
      <input
        type="checkbox"
        checked={all}
        ref={(node) => { if (node) node.indeterminate = some; }}
        aria-label={`Apply every file in ${directory}`}
        onChange={() => onToggle(!all)}
      />
    </label>
  );
}

function OptionBand({ directory, files, selected, setMany }: {
  directory: string;
  files: MockFile[];
  selected: Set<string>;
  setMany: (ids: string[], on: boolean) => void;
}) {
  const picked = files.filter((file) => selected.has(file.id)).length;
  return (
    <div className="sync-project-files-group-heading">
      <BandChoice
        directory={directory}
        picked={picked}
        total={files.length}
        onToggle={(on) => setMany(files.map((file) => file.id), on)}
      />
      <Icon name="folder" size={14} />
      <strong>{directory}/</strong>
      <small>{files.length} file{files.length === 1 ? "" : "s"}</small>
      {/* The shipped counter chip, pushed to the band's far edge — the same
          readout the header wears, at the same weight. */}
      <span
        className="sync-sync-review-selected"
        style={{ "--review-count-digits": 1 } as CSSProperties}
        aria-label={`${picked} of ${files.length} selected in ${directory}`}
        title={`${picked} of ${files.length} selected in ${directory}/`}
      >
        <b>{picked}</b>/{files.length}
      </span>
    </div>
  );
}

function OptionGroups({ selected, toggle, setMany }: {
  selected: Set<string>;
  toggle: (id: string) => void;
  setMany: (ids: string[], on: boolean) => void;
}) {
  return (
    <>
      {GROUPS.map((group) => (
        <div key={group.directory || "(root)"}>
          {group.directory !== "" && (
            <OptionBand directory={group.directory} files={group.files} selected={selected} setMany={setMany} />
          )}
          <div className="sync-history-thread-list flat">
            {group.files.map((file) => (
              <FileCard key={file.id} file={file} checked={selected.has(file.id)} onToggle={() => toggle(file.id)} />
            ))}
          </div>
        </div>
      ))}
    </>
  );
}

// ---------------------------------------------------------------------------
// Option A — checkboxes select, words command
// ---------------------------------------------------------------------------

function PanelOptionA() {
  const state = useSelection(["notes"]);
  return (
    <ReviewShell
      headerCluster={(
        <div className="sync-push-resource-actions">
          <span className="sync-sync-review-selected" style={{ "--review-count-digits": 1 } as CSSProperties}>
            <b>{state.selected.size}</b>/4
          </span>
          <button
            type="button"
            className="btn btn-ghost"
            title="Select the safe additions — files that only add, never replace"
            onClick={() => state.setMany([...SAFE_IDS], true)}
          >
            <Icon name="check-circle" size={14} /> Recommended
          </button>
          <button
            type="button"
            className="btn btn-ghost"
            disabled={state.selected.size === 0}
            title="Clear the selection"
            onClick={() => state.setSelected(new Set())}
          >
            None
          </button>
          <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label="Close pull review">
            <Icon name="x" size={15} />
          </button>
        </div>
      )}
    >
      <OptionGroups selected={state.selected} toggle={state.toggle} setMany={state.setMany} />
    </ReviewShell>
  );
}

// ---------------------------------------------------------------------------
// Option C — keep the header, fix the folders
// ---------------------------------------------------------------------------

function PanelOptionC() {
  const state = useSelection(["notes"]);
  return (
    <ReviewShell
      headerCluster={(
        <div className="sync-push-resource-actions">
          <span className="sync-sync-review-selected" style={{ "--review-count-digits": 1 } as CSSProperties}>
            <b>{state.selected.size}</b>/4
          </span>
          <button
            type="button"
            className="btn btn-ghost sync-sync-review-icon-action"
            title={`Restore the recommended safe selection (${SAFE_IDS.size})`}
            aria-label={`Use recommended selection (${SAFE_IDS.size})`}
            onClick={() => state.setMany([...SAFE_IDS], true)}
          >
            <Icon name="check-circle" size={15} />
          </button>
          <button
            type="button"
            className="btn btn-ghost sync-sync-review-icon-action"
            title="Clear current selection"
            aria-label="Clear current selection"
            disabled={state.selected.size === 0}
            onClick={() => state.setSelected(new Set())}
          >
            <Icon name="trash" size={15} />
          </button>
          <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label="Close pull review">
            <Icon name="x" size={15} />
          </button>
        </div>
      )}
    >
      <OptionGroups selected={state.selected} toggle={state.toggle} setMany={state.setMany} />
    </ReviewShell>
  );
}

// ---------------------------------------------------------------------------
// Option B — one selection menu
// ---------------------------------------------------------------------------

function PanelOptionB() {
  const state = useSelection(["notes"]);
  const [open, setOpen] = useState(true);
  const matches = (ids: Set<string>) => state.selected.size === ids.size && [...ids].every((id) => state.selected.has(id));
  const everything = new Set(FILES.map((file) => file.id));
  const choose = (ids: Set<string>) => { state.setSelected(new Set(ids)); setOpen(false); };
  return (
    <ReviewShell
      headerCluster={(
        <div className="sync-push-resource-actions">
          <span className="mock-selection-menu-anchor">
            <button
              type="button"
              className="btn btn-ghost"
              aria-haspopup="menu"
              aria-expanded={open}
              title="Change what is selected"
              onClick={() => setOpen(!open)}
            >
              <span className="sync-sync-review-selected" style={{ "--review-count-digits": 1 } as CSSProperties}>
                <b>{state.selected.size}</b>/4
              </span>
              selected
              <Icon name={open ? "chevron-up" : "chevron-down"} size={11} />
            </button>
            {open && (
              <div className="sync-path-menu end" role="menu" aria-label="Change the selection">
                <div className="sync-path-menu-group">
                  <button type="button" role="menuitemradio" aria-checked={matches(SAFE_IDS)} className="sync-path-menu-item" onClick={() => choose(SAFE_IDS)}>
                    <span className="sync-path-menu-copy">
                      <span>Safe additions · {SAFE_IDS.size}</span>
                      <small className="phrase">Files that only add — nothing local is replaced</small>
                    </span>
                    {matches(SAFE_IDS) && <Icon name="check" size={13} className="sync-path-menu-check" />}
                  </button>
                  <button type="button" role="menuitemradio" aria-checked={matches(everything)} className="sync-path-menu-item" onClick={() => choose(everything)}>
                    <span className="sync-path-menu-copy">
                      <span>Everything · {FILES.length}</span>
                      <small className="phrase">Including files that replace this Mac’s copies</small>
                    </span>
                    {matches(everything) && <Icon name="check" size={13} className="sync-path-menu-check" />}
                  </button>
                  <button type="button" role="menuitemradio" aria-checked={state.selected.size === 0} className="sync-path-menu-item" onClick={() => choose(new Set())}>
                    <span className="sync-path-menu-copy">
                      <span>None</span>
                      <small className="phrase">Start from an empty selection</small>
                    </span>
                    {state.selected.size === 0 && <Icon name="check" size={13} className="sync-path-menu-check" />}
                  </button>
                </div>
              </div>
            )}
          </span>
          <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label="Close pull review">
            <Icon name="x" size={15} />
          </button>
        </div>
      )}
    >
      <OptionGroups selected={state.selected} toggle={state.toggle} setMany={state.setMany} />
    </ReviewShell>
  );
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

function Panel({ title, note, children }: { title: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p>{note}</p>
      <div className="mockup-stage tight">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>The files review’s selection controls</h1>
            <p>
              Today one icon carries three meanings — the check-circle is “use the recommended
              selection” in the header, “select every file in this folder” on a band, and
              “Select safe additions” in the toolbar — and each pairs with a trash that means
              “deselect” on a screen where a card’s trash means “remove from storage”. Three
              counters count three scopes. Both options below make one move: selection is shown
              and driven by the same checkbox the rows already wear — a tri-state box on each
              folder band — and every smart selection is named in words. They differ only in
              what the header carries. Everything here is live; try the bands.
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
            <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel
          title="Today — three clusters, one icon, three meanings"
          note={<>
            The shipped controls, live. The header’s ⊘ and 🗑 are unlabeled; each folder band
            repeats the same pair at folder scope; “Select safe additions” floats in the toolbar
            as a third smart-select — and on the files step the header’s ⊘ actually recommends
            nothing, because files are excluded from the step’s recommended set.
          </>}
        >
          <PanelToday />
        </Panel>

        <Panel
          title="Option C — keep the header, fix the folders (SHIPPED)"
          note={<>
            Shipped: the review header stays exactly the cluster every review
            wears — count, ⊘ recommended, 🗑 clear, ✕ — untouched. What changes is below it.
            Each folder band trades its icon pair for the tri-state checkbox its rows already
            wear, keeping the shipped counter chip at the band’s edge. And the floating
            “Select safe additions” toolbar button folds into the header’s ⊘, which on the
            files step today recommends nothing — now it restores the safe additions its
            tooltip already promises. One icon pair on the whole screen instead of one per
            folder, and the toolbar goes back to being a search bar.
          </>}
        >
          <PanelOptionC />
        </Panel>

        <Panel
          title="Option A — checkboxes select, words command"
          note={<>
            Each folder band leads with the same checkbox its rows wear — ticked, unticked, or a
            dash while the folder is partly selected; one click applies or clears the whole
            folder, and “n of m” becomes a quiet readout instead of a counter chip. The header
            keeps the count and spends words where icons were: <em>Recommended</em> — which for
            the files step now means the safe additions, so the floating toolbar button
            disappears — and <em>None</em>. No trash means “deselect” anywhere again.
          </>}
        >
          <PanelOptionA />
        </Panel>

        <Panel
          title="Option B — one selection menu"
          note={<>
            The same folder checkboxes, and the header collapses to a single self-describing
            control: the count is the button, and it opens the app’s own menu of named
            selections — <em>Safe additions · 2</em>, <em>Everything · 4</em>, <em>None</em> —
            each with a sentence saying what it includes, the current match ticked. One control
            instead of three, and it scales if a step ever grows another smart selection. Shown
            open; pick an entry to see the bands follow.
          </>}
        >
          <PanelOptionB />
        </Panel>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); }
.mockup-stage.tight { padding: 0; overflow: hidden; }
body { background: var(--app-shell); }
.mock-review .sync-sync-review-scroll { max-height: none; overflow: visible; scrollbar-gutter: auto; }
`;

const PROPOSED_CSS = `
/* Option C shipped: the tri-state checkbox, the band's checkbox column and
   the counter chip's edge placement all live in App.css now, which this
   harness imports. What remains here serves only the declined variants. */

/* Option B only: the menu needs an anchor; the button itself is the app's
   own ghost button and the menu the app's own popover. */
.mock-selection-menu-anchor { position: relative; display: inline-flex; }
.mock-selection-menu-anchor > .btn { display: inline-flex; align-items: center; gap: 6px; }
`;

const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS;
document.head.appendChild(style);
/* One root across hot reloads. */
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
