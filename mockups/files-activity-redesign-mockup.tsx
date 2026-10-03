import { StrictMode, useState } from "react";
import type { CSSProperties } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import Notice from "../src/components/notice/Notice";
import SyncReviewSelectionBar from "../src/components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../src/components/sync-review/ThreadStatusIcon";
import "../src/App.css";

/**
 * Mockup: files as TWO-ROW rows — the sessions grammar — across all three
 * surfaces at once: the activity face, the Push review, and the Pull review.
 *
 * One folder grammar everywhere: every directory is a framed tile with a
 * header strip ("Project root" included); in the reviews the strip also
 * carries the header's own selection cluster, replacing the floating bands.
 * Rows collapse their borders inside the tile, the commit-section way.
 *
 * Static data, shipped CSS, real leaf components. Dev-server only.
 */

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1160px; margin: 0 auto; padding: 24px 22px 70px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 16px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; align-items: center; margin: 0 0 20px; flex-wrap: wrap; }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 5px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--bg-active); color: var(--text-0); border-color: var(--border-mid); }
  .mk-variant { margin: 0 0 30px; }
  .mk-variant > h3 { margin: 0 0 2px; color: var(--text-0); font-size: var(--type-md); }
  .mk-variant > p { margin: 0 0 10px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-frame { width: 100%; max-width: 980px; padding: 3px; border: 1px dashed var(--border-mid); border-radius: 6px; background: var(--retro-frame-gutter); }
  .mk-frame > .mk-pane { border-radius: 4px; overflow: hidden; background: var(--bg-0); }
  .mk-pathbar { display: flex; align-items: center; gap: 8px; min-height: 52px; padding: 0 16px; border-bottom: 1px solid var(--border); background: var(--bg-0); }
  .mk-pathbar > svg { color: var(--text-2); flex: 0 0 auto; }
  .mk-pathbar > strong { font-size: var(--type-md); font-weight: var(--weight-semibold); color: var(--text-0); }
  .mk-pathbar > .sep { color: var(--text-3); font-family: var(--font-mono); }
  .mk-pathbar > .leaf { color: var(--text-0); font-family: var(--font-mono); font-size: var(--type-sm); }
  .mk-body { padding: 14px 16px 22px; display: flex; flex-direction: column; gap: 12px; }
  .mk-footer { display: flex; align-items: center; justify-content: flex-end; gap: 16px; min-height: 64px; padding: 10px 16px; border-top: 1px solid var(--border); background: var(--surface-footer); }

  .fa-filter { display: flex; align-items: center; gap: 10px; }
  .fa-filter input { min-width: 0; height: 30px; flex: 1; padding: 0 10px; border: 1px solid var(--control-border); border-radius: 7px; color: var(--text-0); background: var(--bg-1); outline: none; }
  .fa-filter > span { color: var(--text-3); font-size: var(--type-2xs); white-space: nowrap; }

  /* The framed folder tile, shared by all three surfaces. Two-row card rows
     collapse their borders inside it, the commit-section way. */
  .fa-tile { overflow: hidden; border: 1px solid var(--border); border-radius: 6px; background: var(--surface-row); }
  .fa-tile-head { display: flex; align-items: center; gap: 8px; min-height: 38px; padding: 2px 12px; border-bottom: 1px solid var(--border-mid); background: var(--surface-subtle); }
  .fa-tile-head > svg { flex: 0 0 auto; color: var(--text-3); }
  .fa-tile-head > strong { color: var(--text-0); font-size: var(--type-xs); font-weight: var(--weight-semibold); }
  .fa-tile-head > small { color: var(--text-3); font-size: var(--type-2xs); }
  .fa-tile.cloud { border-color: color-mix(in srgb, var(--thread-status-cloud) 35%, var(--border)); }
  .fa-tile.cloud > .fa-tile-head { background: color-mix(in srgb, var(--thread-status-cloud) 7%, var(--surface-subtle)); }
  .fa-tile.cloud > .fa-tile-head > svg { color: var(--thread-status-cloud); }
  .fa-tile > .sync-history-thread-list { gap: 0; }
  .fa-tile .sync-history-thread-card { border: 0; border-radius: 0; background: transparent; }
  .fa-tile .sync-history-thread-card + .sync-history-thread-card { border-top: 1px solid var(--border-mid); }
`;

/* ---------------------------------------------------------------- data --- */

type Tone = "synced" | "local" | "storage" | "diverged" | "unavailable" | "conflict";

interface MockEntry {
  name: string;
  directory: string;
  tone: Tone;
  icon: ThreadStatusIconName;
  sentence: string;
  kind: string;
  size: string;
  date: string;
  warning?: string;
  blocked?: string;
  cloudOnly?: boolean;
  storagePresent?: boolean;
  /** Pull only. */
  op?: "add" | "replace" | "delete";
  preservedCopy?: string;
}

const STORAGE = "Local storage 1";

const ACTIVITY_ENTRIES: MockEntry[] = [
  { name: "ghost_story.docx", directory: "", tone: "synced", icon: "synced", sentence: `Up to date here and in ${STORAGE}.`, kind: "Word", size: "36 KB", date: "Aug 30, 2026, 00:11", storagePresent: true },
  { name: "notes.txt", directory: "", tone: "local", icon: "local-outstanding", sentence: `Only on this computer. Push to save it in ${STORAGE}.`, kind: "Text", size: "4.1 KB", date: "Aug 30, 2026, 09:40" },
  { name: "render.sh", directory: "", tone: "diverged", icon: "divergent", sentence: `Changed here and in ${STORAGE}. Review before syncing.`, kind: "Shell", size: "2.1 KB", date: "Aug 29, 2026, 18:55", warning: "Executable" },
  { name: "test-design.md", directory: "doc", tone: "synced", icon: "synced", sentence: `Up to date here and in ${STORAGE}.`, kind: "Markdown", size: "2.1 KB", date: "Aug 30, 2026, 01:39", storagePresent: true },
  { name: "spec.md", directory: "doc", tone: "storage", icon: "cloud-outstanding", sentence: `Newer in ${STORAGE}. Pull to update this computer.`, kind: "Markdown", size: "9.8 KB", date: "Aug 29, 2026, 08:15", storagePresent: true },
  { name: "cold-joke.md", directory: "", tone: "storage", icon: "cloud-outstanding", sentence: `Only in ${STORAGE}. Pull to download it here.`, kind: "Markdown", size: "3.9 KB", date: "Aug 30, 2026, 00:19", cloudOnly: true },
  { name: "theme.wav", directory: "assets/audio", tone: "storage", icon: "cloud-outstanding", sentence: `Only in ${STORAGE}. Pull to download it here.`, kind: "WAV audio", size: "2.4 MB", date: "Aug 28, 2026, 19:03", cloudOnly: true },
];

const PUSH_ENTRIES: MockEntry[] = [
  { name: "notes.txt", directory: "", tone: "local", icon: "local-outstanding", sentence: `Only on this computer. Push to save it in ${STORAGE}.`, kind: "Text", size: "4.1 KB", date: "Aug 30, 2026, 09:40" },
  { name: "ghost_story.docx", directory: "", tone: "synced", icon: "synced", sentence: `Up to date here and in ${STORAGE}.`, kind: "Word", size: "36 KB", date: "Aug 30, 2026, 00:11", storagePresent: true },
  { name: "poster.psd", directory: "", tone: "unavailable", icon: "unavailable", sentence: "Unavailable for sync.", kind: "Photoshop", size: "48 MB", date: "Aug 18, 2026, 13:37", blocked: "File exceeds the 16 MiB project-content limit" },
  { name: "api_key.txt", directory: "doc", tone: "local", icon: "local-outstanding", sentence: `Only on this computer. Push to save it in ${STORAGE}.`, kind: "Text", size: "431 B", date: "Aug 28, 2026, 11:41", warning: "Possible secret" },
  { name: "spec.md", directory: "doc", tone: "storage", icon: "cloud-outstanding", sentence: `Newer in ${STORAGE}. Pull to update this computer.`, kind: "Markdown", size: "9.8 KB", date: "Aug 29, 2026, 08:15", storagePresent: true },
];

const PULL_ENTRIES: MockEntry[] = [
  { name: "soundtrack.mp3", directory: "", tone: "storage", icon: "cloud-outstanding", sentence: `Only in ${STORAGE}. Applying downloads it here.`, kind: "MP3 audio", size: "8.2 MB", date: "Aug 20, 2026, 20:14", op: "add" },
  { name: "ghost_story.docx", directory: "", tone: "diverged", icon: "divergent", sentence: `Changed here and in ${STORAGE}. Applying keeps the local version as ghost_story-local-copy.docx, then writes the storage version.`, kind: "Word", size: "36 KB", date: "Aug 30, 2026, 11:58", op: "replace", preservedCopy: "ghost_story-local-copy.docx" },
  { name: "cold-joke.md", directory: "", tone: "conflict", icon: "conflict", sentence: `Removed in ${STORAGE}. Applying deletes the local file (a backup is kept).`, kind: "Markdown", size: "3.9 KB", date: "Aug 30, 2026, 12:02", op: "delete" },
  { name: "spec.md", directory: "doc", tone: "storage", icon: "cloud-outstanding", sentence: `Newer in ${STORAGE}. Applying updates this computer.`, kind: "Markdown", size: "9.8 KB", date: "Aug 29, 2026, 08:15", op: "replace" },
  { name: "theme.wav", directory: "assets/audio", tone: "storage", icon: "cloud-outstanding", sentence: `Only in ${STORAGE}. Applying downloads it here.`, kind: "WAV audio", size: "2.4 MB", date: "Aug 28, 2026, 19:03", op: "add" },
];

const OP_CHIP: Record<NonNullable<MockEntry["op"]>, string> = {
  add: "Add file",
  replace: "Replace file",
  delete: "Delete file",
};

interface Group { directory: string; entries: MockEntry[] }

function groupByDirectory(entries: MockEntry[]): Group[] {
  const byDirectory = new Map<string, MockEntry[]>();
  for (const entry of entries) {
    if (!byDirectory.has(entry.directory)) byDirectory.set(entry.directory, []);
    byDirectory.get(entry.directory)!.push(entry);
  }
  return [...byDirectory.keys()]
    .sort((a, b) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)))
    .map((directory) => ({ directory, entries: byDirectory.get(directory)! }));
}

function groupFacts(entries: MockEntry[]): string {
  return `${entries.length} file${entries.length === 1 ? "" : "s"}`;
}

/* ------------------------------------------------------------- pieces --- */

function StatusMark({ entry }: { entry: MockEntry }) {
  return (
    <span className={`sync-thread-sync-indicator ${entry.tone}`} data-tooltip={entry.sentence} aria-label={entry.sentence} tabIndex={0}>
      <ThreadStatusIcon status={entry.icon} size={18} />
    </span>
  );
}

/** The two-row file row, the sessions grammar: title line, then the readout
 * line — kind and size chips, the rule, the ⓘ, and the stamp at the edge. */
function FileRow({ entry, mode, state }: {
  entry: MockEntry;
  mode: "activity" | "push" | "pull";
  state?: {
    selected: Set<string>;
    acked: Set<string>;
    removed: Set<string>;
    toggle: (name: string) => void;
    ack: (name: string) => void;
    remove: (name: string) => void;
  };
}) {
  const removed = state?.removed.has(entry.name) ?? false;
  const selectable = mode !== "activity" && !entry.blocked && entry.tone !== "storage" || mode === "pull";
  const selected = (state?.selected.has(entry.name) ?? false) && !removed;
  const opLabel = entry.op
    ? entry.op === "replace" && entry.preservedCopy
      ? "Replace file · keeps local copy"
      : OP_CHIP[entry.op]
    : null;
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {mode !== "activity" && selectable && !entry.blocked && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : mode === "pull" ? "Apply" : "Include"} ${entry.name}`}>
              <input type="checkbox" checked={selected} onChange={() => state?.toggle(entry.name)} />
            </label>
          )}
          <StatusMark entry={entry} />
          <strong title={entry.name}>{entry.name}</strong>
          {mode === "pull" && opLabel && (
            <span className="sync-history-branch-state history-only" title={entry.sentence}>{opLabel}</span>
          )}
          {entry.blocked && (
            <span className="sync-history-branch-state history-only" title={entry.blocked}>Blocked</span>
          )}
          {removed && <span className="sync-history-branch-state history-only">Removes from storage</span>}
          {entry.warning && (
            <span className="sync-project-file-warning"><Icon name="alert-triangle" size={13} /> {entry.warning}</span>
          )}
        </div>
        <div className="sync-history-thread-actions">
          {mode === "push" && entry.storagePresent && (
            <button type="button" className="btn btn-ghost sync-history-launch-action" title={removed ? "Keep in storage" : "Remove from storage"} onClick={() => state?.remove(entry.name)}>
              <Icon name={removed ? "refresh" : "trash"} size={14} />
            </button>
          )}
          <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={entry.cloudOnly || (mode === "pull" && entry.op === "add")} title={entry.cloudOnly || (mode === "pull" && entry.op === "add") ? "Not on this Mac" : `Reveal ${entry.name} in Finder`}>
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${entry.name} details`}>
            <span className="sync-history-thread-metric" data-tooltip={`Kind · ${entry.kind}`} tabIndex={0}><Icon name="file" size={12} /><b>{entry.kind}</b></span>
            <span className="sync-history-thread-metric" data-tooltip={`On disk · ${entry.size}`} tabIndex={0}><Icon name="drive" size={12} /><b>{entry.size}</b></span>
            {entry.warning
              ? <span className="sync-history-thread-metric sync-history-partial" data-tooltip={entry.warning} tabIndex={0}><Icon name="alert-triangle" size={12} /><b>1</b></span>
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <button type="button" className="btn btn-ghost sync-history-session-detail-action" title={`Details for ${entry.name}`}>
              <Icon name="info" size={13} />
            </button>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`Modified · ${entry.date}`} tabIndex={0}>
            <time>{entry.date}</time>
          </span>
        </div>
      </div>
      {mode === "push" && entry.warning && selected && (
        <label className="catalog-card-ack">
          <span className="sync-sync-row-choice">
            <input type="checkbox" checked={state?.acked.has(entry.name) ?? false} onChange={() => state?.ack(entry.name)} />
          </span>
          <Icon name="alert-triangle" size={12} />
          <span className="catalog-card-ack-copy"><strong>Credential-shaped content</strong> — doc/{entry.name}</span>
          <small className={state?.acked.has(entry.name) ? "done" : undefined}>{state?.acked.has(entry.name) ? "Reviewed" : "Mark reviewed"}</small>
        </label>
      )}
      {mode === "push" && entry.tone === "storage" && (
        <Notice
          severity="warning"
          timing="inline"
          className="sync-project-file-conflict"
          action={<button type="button" className="btn btn-ghost">Open Pull review</button>}
        >
          Pull required
        </Notice>
      )}
    </article>
  );
}

/** The tile head's selection cluster, review surfaces only. */
function TileCluster({ group, state }: {
  group: Group;
  state: { selected: Set<string>; toggleGroup: (group: Group, on: boolean) => void; removed: Set<string> };
}) {
  const selectable = group.entries.filter((entry) => !entry.blocked && !state.removed.has(entry.name));
  const picked = selectable.filter((entry) => state.selected.has(entry.name)).length;
  return (
    <div className="sync-push-resource-actions">
      <span
        className="sync-sync-review-selected"
        style={{ "--review-count-digits": String(selectable.length).length } as CSSProperties}
        title={`${picked} of ${selectable.length} selected`}
      >
        <b>{picked}</b>/{selectable.length}
      </span>
      <button type="button" className="btn btn-ghost sync-sync-review-icon-action" disabled={picked === selectable.length} title="Select all files in this folder" onClick={() => state.toggleGroup(group, true)}>
        <Icon name="check-circle" size={15} />
      </button>
      <button type="button" className="btn btn-ghost sync-sync-review-icon-action" disabled={picked === 0} title="Clear this folder's selection" onClick={() => state.toggleGroup(group, false)}>
        <Icon name="trash" size={15} />
      </button>
    </div>
  );
}

function useSelection(initial: string[]) {
  const defaults = initial;
  const [selected, setSelected] = useState<Set<string>>(() => new Set(initial));
  const [acked, setAcked] = useState<Set<string>>(() => new Set());
  const [removed, setRemoved] = useState<Set<string>>(() => new Set());
  const flip = (setter: typeof setSelected) => (name: string) => setter((current) => {
    const next = new Set(current);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    return next;
  });
  return {
    selected,
    acked,
    removed,
    toggle: flip(setSelected),
    ack: flip(setAcked),
    remove: flip(setRemoved),
    useRecommended: () => setSelected(new Set(defaults)),
    clearAll: () => setSelected(new Set()),
    toggleGroup: (group: Group, on: boolean) => setSelected((current) => {
      const next = new Set(current);
      for (const entry of group.entries) {
        if (entry.blocked || removed.has(entry.name)) continue;
        if (on) next.add(entry.name);
        else next.delete(entry.name);
      }
      return next;
    }),
  };
}

function Tile({ group, cloud = false, head, children }: {
  group: Group;
  cloud?: boolean;
  head?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className={`fa-tile${cloud ? " cloud" : ""}`}>
      <div className="fa-tile-head">
        <Icon name={cloud ? "cloud" : group.directory === "" ? "git-folder" : "folder"} size={14} />
        <strong>{cloud ? <>Only in <span className="sync-history-storage-name">{STORAGE}</span></> : group.directory === "" ? "Project root" : `${group.directory}/`}</strong>
        {!cloud && <small>{groupFacts(group.entries)}</small>}
        {head}
      </div>
      <div className="sync-history-thread-list">{children}</div>
    </div>
  );
}

function FilterLine({ count }: { count: number }) {
  return (
    <div className="fa-filter">
      <input type="search" placeholder="Search files…" aria-label="Search project file paths" />
      <span>{count} entries</span>
    </div>
  );
}

function PathBar({ leaf }: { leaf: string }) {
  return (
    <div className="mk-pathbar">
      <Icon name="drive" size={15} />
      <strong>{STORAGE}</strong>
      <span className="sep">/</span>
      <Icon name="folder" size={15} />
      <strong>testProjectNonGit</strong>
      <span className="sep">/</span>
      <span className="leaf">{leaf}</span>
    </div>
  );
}

/* --------------------------------------------------------------- panes --- */

function ActivityPane() {
  const cloud: Group = { directory: "", entries: ACTIVITY_ENTRIES.filter((entry) => entry.cloudOnly) };
  const localGroups = groupByDirectory(ACTIVITY_ENTRIES.filter((entry) => !entry.cloudOnly));
  return (
    <div className="mk-pane">
      <PathBar leaf="Files" />
      <div className="mk-body">
        <FilterLine count={ACTIVITY_ENTRIES.length} />
        <Tile group={cloud} cloud>
          {cloud.entries.map((entry) => <FileRow key={entry.name} entry={entry} mode="activity" />)}
        </Tile>
        {localGroups.map((group) => (
          <Tile key={group.directory || "(root)"} group={group}>
            {group.entries.map((entry) => <FileRow key={entry.name} entry={entry} mode="activity" />)}
          </Tile>
        ))}
      </div>
    </div>
  );
}

function PushPane() {
  const state = useSelection(["notes.txt", "ghost_story.docx", "api_key.txt"]);
  const groups = groupByDirectory(PUSH_ENTRIES);
  const selectable = PUSH_ENTRIES.filter((entry) => !entry.blocked && entry.tone !== "storage" && !state.removed.has(entry.name));
  const picked = selectable.filter((entry) => state.selected.has(entry.name)).length;
  return (
    <div className="mk-pane">
      <PathBar leaf="Push review · Project files" />
      {/* The global bar survives untouched: its check-circle is still "use
          the recommended selection" for the whole step, while a tile head's
          cluster only bulk-selects its own folder. */}
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title"><h2>Push review</h2></div>
        <SyncReviewSelectionBar
          count={picked}
          total={selectable.length}
          recommendedCount={selectable.length}
          recommendedTitle="Use the recommended project file selection"
          recommendedDisabled={false}
          onRecommended={state.useRecommended}
          clearDisabled={picked === 0}
          onClear={state.clearAll}
          close={{ ariaLabel: "Close push review", onClose: () => undefined }}
        />
      </header>
      <div className="mk-body">
        <FilterLine count={PUSH_ENTRIES.length} />
        {groups.map((group) => (
          <Tile
            key={group.directory || "(root)"}
            group={group}
            head={<TileCluster group={group} state={state} />}
          >
            {group.entries.map((entry) => <FileRow key={entry.name} entry={entry} mode="push" state={state} />)}
          </Tile>
        ))}
      </div>
      <div className="mk-footer">
        <button type="button" className="btn btn-ghost"><Icon name="chevron-left" size={14} />Sessions</button>
        <button type="button" className="btn btn-primary sync-pull-apply-button">
          <ThreadStatusIcon status="local-outstanding" size={16} />
          Push
        </button>
      </div>
    </div>
  );
}

function PullPane() {
  const state = useSelection(["soundtrack.mp3", "spec.md", "theme.wav"]);
  const groups = groupByDirectory(PULL_ENTRIES);
  const picked = PULL_ENTRIES.filter((entry) => state.selected.has(entry.name)).length;
  return (
    <div className="mk-pane">
      <PathBar leaf="Pull review · Project files" />
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title"><h2>Pull review</h2></div>
        <SyncReviewSelectionBar
          count={picked}
          total={PULL_ENTRIES.length}
          recommendedCount={PULL_ENTRIES.length}
          recommendedTitle="Use the recommended selection"
          recommendedDisabled={false}
          onRecommended={state.useRecommended}
          clearDisabled={picked === 0}
          onClear={state.clearAll}
          close={{ ariaLabel: "Close pull review", onClose: () => undefined }}
        />
      </header>
      <div className="mk-body">
        <div className="fa-filter">
          <input type="search" placeholder="Search files…" aria-label="Search project file paths" />
          <span>{PULL_ENTRIES.length} entries</span>
          <button type="button" className="btn btn-ghost"><Icon name="check-circle" size={14} /> Select safe additions</button>
        </div>
        {groups.map((group) => (
          <Tile
            key={group.directory || "(root)"}
            group={group}
            head={<TileCluster group={group} state={state} />}
          >
            {group.entries.map((entry) => <FileRow key={entry.name} entry={entry} mode="pull" state={state} />)}
          </Tile>
        ))}
      </div>
      <div className="mk-footer">
        <button type="button" className="btn btn-ghost"><Icon name="chevron-left" size={14} />Sessions</button>
        <button type="button" className="btn btn-primary sync-pull-apply-button">
          <ThreadStatusIcon status="cloud-outstanding" size={16} />
          Apply
        </button>
      </div>
    </div>
  );
}

/* ---------------------------------------------------------------- page --- */

function Page() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  document.documentElement.dataset.theme = theme;
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Files, two-row grammar — activity · Push · Pull</h1>
      <p className="mk-lede">
        Every file is a two-row card in the sessions grammar: title line (status mark,
        name, chips, row actions), then the readout line (kind and size chips, the rule,
        ⓘ, and the stamp at the edge). One folder grammar across all three surfaces:
        framed tiles with a header strip — the reviews carry the selection cluster in the
        strip instead of floating bands, and rows collapse their borders inside the tile.
        Everything is interactive: ticks, clusters, acks, removal.
      </p>
      <div className="mk-bar" role="group" aria-label="Theme">
        <button type="button" className={theme === "light" ? "on" : undefined} onClick={() => setTheme("light")}>Light</button>
        <button type="button" className={theme === "dark" ? "on" : undefined} onClick={() => setTheme("dark")}>Dark</button>
      </div>

      <div className="mk-variant">
        <h3>1 · Activity face</h3>
        <p>Read-only: no ticks, no acks. The cloud tile leads; kind and size ride line two the way a session&apos;s messages and tokens do.</p>
        <div className="mk-frame"><ActivityPane /></div>
      </div>

      <div className="mk-variant">
        <h3>2 · Push review</h3>
        <p>Same rows plus the review controls: checkbox in the title line, tile-head cluster over the folder&apos;s files, ack chips inside the card that raised them, trash for stored files, and the storage-ahead row held with its Pull-required strip.</p>
        <div className="mk-frame"><PushPane /></div>
      </div>

      <div className="mk-variant">
        <h3>3 · Pull review</h3>
        <p>Operation chips on the title line — &quot;Replace file · keeps local copy&quot; on the diverged row — with adds default-selected, delete unticked, and reveal disabled on files not on this Mac.</p>
        <div className="mk-frame"><PullPane /></div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
