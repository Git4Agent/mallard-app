import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import SyncReviewTabs from "./components/sync-review/SyncReviewTabs";
import ThreadStatusIcon, { type ThreadStatusIconName } from "./components/sync-review/ThreadStatusIcon";
import "./App.css";

/**
 * Mockup: the non-git Push review as a two-step flow.
 *
 * Section A — where the "Continue to files" control lives (three placements).
 * Section B — every file is a standard Catalog-grammar card; the variants
 * differ only in what the FOLDER is: a heading band with a select-all
 * checkbox, a framed container, or a divider with text actions. Cloud-only
 * (Pull-required) rows are out of scope for this Push mock.
 *
 * Static data, shipped CSS, real leaf components. Not part of the app
 * bundle: the build has one entry, and this file is reachable only from the
 * dev server.
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
  .mk-section { margin: 34px 0 10px; }
  .mk-section > h2 { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-lg); }
  .mk-section > p { margin: 0 0 14px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-variant { margin: 0 0 26px; }
  .mk-variant > h3 { margin: 0 0 2px; color: var(--text-0); font-size: var(--type-md); }
  .mk-variant > p { margin: 0 0 10px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-frame { width: 100%; max-width: 980px; padding: 3px; border: 1px dashed var(--border-mid); border-radius: 6px; background: var(--retro-frame-gutter); }
  .mk-frame > section { border-radius: 4px; overflow: hidden; }
  /* Mock-only: the tab-strip CTA sits at the strip's right edge. The zone
     carries the strip's own bottom rule so the line runs unbroken. */
  .mk-tabrow { display: flex; align-items: stretch; }
  .mk-tabrow > .mk-tabwrap { flex: 1; min-width: 0; display: flex; }
  .mk-tab-cta-zone { display: flex; align-items: center; padding: 0 12px; border-bottom: 1px solid var(--border); }
  /* Mock-only: folder group treatments. */
  .mk-group-heading { display: flex; align-items: center; gap: 8px; min-height: 34px; margin: 14px 2px 6px; padding: 0 6px; color: var(--text-1); }
  .mk-group-heading > svg { color: var(--text-3); flex: 0 0 auto; }
  .mk-group-heading > strong { font-size: var(--type-sm); font-weight: var(--weight-semibold); color: var(--text-0); }
  .mk-group-heading > small { color: var(--text-3); font-size: var(--type-2xs); }
  .mk-folder-card { margin-top: 14px; border: 1px solid var(--border); border-radius: 4px; background: var(--surface-row); }
  .mk-folder-card > .mk-group-heading { margin: 0; min-height: 40px; padding: 4px 12px; border-bottom: 1px solid var(--border-mid); }
  .mk-folder-card.collapsed > .mk-group-heading { border-bottom: 0; }
  .mk-folder-card > .sync-history-thread-list { gap: 0; }
  .mk-folder-card .catalog-card { border: 0; border-radius: 0; background: transparent; }
  .mk-folder-card .catalog-card + .catalog-card { border-top: 1px solid var(--border-mid); }
  .mk-folder-card .mk-group-collapse { margin-left: 2px; }
  .mk-folder-card .mk-folder-card { margin: 8px 10px 10px; background: var(--bg-0); }
  .mk-group-divider { display: flex; align-items: center; gap: 10px; margin: 18px 2px 6px; padding: 0 2px; }
  .mk-group-divider > svg { color: var(--text-3); flex: 0 0 auto; }
  .mk-group-divider > strong { font-size: var(--type-sm); font-weight: var(--weight-semibold); color: var(--text-0); white-space: nowrap; }
  .mk-group-divider > small { color: var(--text-3); font-size: var(--type-2xs); white-space: nowrap; }
  .mk-group-divider > .mk-rule { flex: 1; height: 1px; background: var(--border); }
  .mk-group-divider > .btn { padding: 2px 9px; font-size: var(--type-2xs); }
  /* Mock-only: the activity frame's workspace path bar, after the shipped
     52px bar — project crumb, then the leaf mirroring the active tab. */
  .mk-pathbar { display: flex; align-items: center; gap: 8px; min-height: 52px; padding: 0 16px; border-bottom: 1px solid var(--border); background: var(--bg-0); }
  .mk-pathbar > svg { color: var(--text-2); flex: 0 0 auto; }
  .mk-pathbar > strong { font-size: var(--type-md); font-weight: var(--weight-semibold); color: var(--text-0); }
  .mk-pathbar > .mk-pathbar-sep { color: var(--text-3); font-family: var(--font-mono); }
  .mk-pathbar-leaf { margin: 0; padding: 2px 4px; border: 0; border-radius: 4px; background: none; color: var(--text-0); font-family: var(--font-mono); font-size: var(--type-sm); letter-spacing: 0.02em; cursor: pointer; }
  .mk-pathbar-leaf:hover { background: var(--bg-active); text-decoration: underline dotted; text-underline-offset: 3px; }
  .mk-pathbar > .mk-pathbar-action { margin-left: auto; }
  /* Mock-only: the "Only in <storage>" partition, at the top of activity
     lists the way the sessions page leads with its storage-only group. The
     block owns the dashed rule that parts it from the local list below. */
  .mk-cloud-block { padding-bottom: 14px; margin-bottom: 6px; border-bottom: 1px dashed var(--border-mid); }
  .mk-cloud-heading { display: flex; align-items: center; gap: 8px; min-height: 34px; margin: 2px 2px 6px; padding: 0 6px; color: var(--text-1); }
  .mk-cloud-heading > svg { color: var(--thread-status-cloud); flex: 0 0 auto; }
  .mk-cloud-heading > strong { font-size: var(--type-sm); font-weight: var(--weight-semibold); color: var(--text-0); }
  .mk-cloud-heading > small { color: var(--text-3); font-size: var(--type-2xs); }
`;

/* ---------------------------------------------------------------- data --- */

type PullOp = "add" | "replace" | "delete";

interface MockFile {
  id: string;
  name: string;
  /** Folder group, or null for a root-level file. */
  group: string | null;
  status: "local" | "synced" | "blocked" | "storage" | "diverged" | "delete";
  size: string;
  stamp: string;
  tip: string;
  /** Pull rows carry the plan's operation; its presence marks a pull row. */
  op?: PullOp;
  /** A diverged replace pins this sibling: the local version is copied
   *  there before the storage version is written. Implemented behavior —
   *  every diverged pull preserves; there is no overwrite-only choice. */
  preservedCopy?: string;
  warning?: { id: string; lead: string; body: string };
  storagePresent?: boolean;
  blockedReason?: string;
}

const CARD_STATUS: Record<MockFile["status"], ThreadStatusIconName> = {
  local: "local-outstanding",
  synced: "synced",
  blocked: "unavailable",
  storage: "cloud-outstanding",
  diverged: "divergent",
  delete: "conflict",
};

const CARD_TONE: Record<MockFile["status"], string> = {
  local: "local",
  synced: "synced",
  blocked: "unavailable",
  storage: "storage",
  diverged: "diverged",
  delete: "conflict",
};

const OP_CHIP: Record<PullOp, string> = {
  add: "Add file",
  replace: "Replace file",
  delete: "Delete file",
};

const FILES: MockFile[] = [
  { id: "f1", name: "cold-joke.md", group: null, status: "local", size: "3.9 KB", stamp: "Aug 29, 2026 at 22:41", tip: "Only on this computer. Push to save it in Attic R2." },
  { id: "f2", name: "ghost_story.docx", group: null, status: "local", size: "36 KB", stamp: "Aug 30, 2026 at 09:12", tip: "Only on this computer. Push to save it in Attic R2." },
  { id: "f3", name: "old_draft.md", group: null, status: "synced", size: "9.4 KB", stamp: "Aug 12, 2026 at 16:20", tip: "Up to date here and in Attic R2.", storagePresent: true },
  { id: "f4", name: "poster.psd", group: null, status: "blocked", size: "48 MB", stamp: "Aug 18, 2026 at 13:37", tip: "Unavailable for sync.", blockedReason: "File exceeds the 16 MiB project-content limit" },
  { id: "n1", name: "outline.md", group: "notes", status: "synced", size: "12 KB", stamp: "Aug 27, 2026 at 18:05", tip: "Up to date here and in Attic R2.", storagePresent: true },
  { id: "n2", name: "render.sh", group: "notes", status: "local", size: "2.1 KB", stamp: "Aug 28, 2026 at 11:33", tip: "Only on this computer. Push to save it in Attic R2.", warning: { id: "w-exec", lead: "Executable content", body: "notes/render.sh" } },
  { id: "n3", name: "api_key.txt", group: "notes", status: "local", size: "431 B", stamp: "Aug 28, 2026 at 11:41", tip: "Only on this computer. Push to save it in Attic R2.", warning: { id: "w-secret", lead: "Credential-shaped content", body: "notes/api_key.txt" } },
  { id: "d1", name: "chapter1.md", group: "notes/drafts", status: "local", size: "5.6 KB", stamp: "Aug 26, 2026 at 10:12", tip: "Only on this computer. Push to save it in Attic R2." },
  { id: "d2", name: "chapter2.md", group: "notes/drafts", status: "local", size: "7.8 KB", stamp: "Aug 26, 2026 at 10:44", tip: "Only on this computer. Push to save it in Attic R2." },
  { id: "a1", name: "title-page.png", group: "assets", status: "local", size: "310 KB", stamp: "Aug 25, 2026 at 15:02", tip: "Only on this computer. Push to save it in Attic R2." },
  { id: "a2", name: "cover_sketch.png", group: "assets", status: "local", size: "1.2 MB", stamp: "Aug 25, 2026 at 15:07", tip: "Only on this computer. Push to save it in Attic R2." },
];

/** One review's file list plus its folder shape and defaults. */
interface ReviewModel {
  files: MockFile[];
  /** DFS order for the flat treatments; nesting for the container one. */
  flatGroups: string[];
  topGroups: string[];
  childGroups: Record<string, string[]>;
  /** Subtree facts — the cluster covers descendants, so the figures do too. */
  facts: Record<string, string>;
  /** Folders that do not exist locally yet; applying creates them. */
  newGroups: ReadonlySet<string>;
  defaults: ReadonlySet<string>;
}

const PUSH_MODEL: ReviewModel = {
  files: FILES,
  flatGroups: ["notes", "notes/drafts", "assets"],
  topGroups: ["notes", "assets"],
  childGroups: { notes: ["notes/drafts"], "notes/drafts": [], assets: [] },
  facts: {
    notes: "5 files · 27.9 KB",
    "notes/drafts": "2 files · 13.4 KB",
    assets: "2 files · 1.5 MB",
  },
  newGroups: new Set(),
  defaults: new Set(["f1", "f2", "f3", "n1", "n2", "n3", "d1", "d2", "a1", "a2"]),
};

/** The pull plan: one row per operation the storage side wants applied.
 * Diverged and delete rows start unticked — applying them needs an explicit
 * choice, and an unticked row simply keeps the local file. */
const PULL_FILES: MockFile[] = [
  { id: "q1", name: "soundtrack.mp3", group: null, status: "storage", op: "add", size: "8.2 MB", stamp: "Aug 20, 2026 at 20:14", tip: "Only in Attic R2. Pull to download it here." },
  { id: "q2", name: "ghost_story.docx", group: null, status: "diverged", op: "replace", size: "36 KB", stamp: "Aug 30, 2026 at 11:58", tip: "Changed here and in Attic R2. Applying keeps the local version as ghost_story-local-copy.docx, then writes the storage version.", preservedCopy: "ghost_story-local-copy.docx" },
  { id: "q3", name: "cold-joke.md", group: null, status: "delete", op: "delete", size: "3.9 KB", stamp: "Aug 30, 2026 at 12:02", tip: "Removed in Attic R2. Applying deletes the local file (a backup is kept)." },
  { id: "q4", name: "outline.md", group: "notes", status: "storage", op: "replace", size: "12 KB", stamp: "Aug 29, 2026 at 08:15", tip: "Newer in Attic R2. Pull to update this computer." },
  { id: "q5", name: "chapter3.md", group: "notes/drafts", status: "storage", op: "add", size: "6.1 KB", stamp: "Aug 29, 2026 at 08:20", tip: "Only in Attic R2. Pull to download it here." },
  { id: "q6", name: "theme.wav", group: "assets/audio", status: "storage", op: "add", size: "2.4 MB", stamp: "Aug 28, 2026 at 19:03", tip: "Only in Attic R2. Pull to download it here." },
];

const PULL_MODEL: ReviewModel = {
  files: PULL_FILES,
  flatGroups: ["notes", "notes/drafts", "assets/audio"],
  topGroups: ["notes", "assets/audio"],
  childGroups: { notes: ["notes/drafts"], "notes/drafts": [], "assets/audio": [] },
  facts: {
    notes: "2 files · 18.1 KB",
    "notes/drafts": "1 file · 6.1 KB",
    "assets/audio": "1 file · 2.4 MB",
  },
  newGroups: new Set(["assets/audio"]),
  defaults: new Set(["q1", "q4", "q5", "q6"]),
};

type ReviewMode = "push" | "pull";
const MODELS: Record<ReviewMode, ReviewModel> = { push: PUSH_MODEL, pull: PULL_MODEL };

/** A file sits under a group when the group is its folder or an ancestor. */
function inSubtree(file: MockFile, group: string): boolean {
  return file.group === group || (file.group != null && file.group.startsWith(`${group}/`));
}

interface MockSession {
  id: string;
  title: string;
  indicator: "local" | "synced" | "storage";
  chips: { messages: string; tools?: string; tokens: string };
  when: string;
  length: string;
}

const SESSION_TIPS: Record<MockSession["indicator"], string> = {
  local: "Only on this computer. Push to save it in Attic R2.",
  synced: "Up to date here and in Attic R2.",
  storage: "Only in Attic R2. Pull to download it here.",
};

const SESSIONS: MockSession[] = [
  { id: "s1", title: "Draft the ghost story ending", indicator: "local", chips: { messages: "24", tools: "9", tokens: "182k" }, when: "Aug 30, 2026 · 09:02", length: "38m" },
  { id: "s2", title: "Punch up the cold-joke timing", indicator: "local", chips: { messages: "11", tokens: "64k" }, when: "Aug 29, 2026 · 22:18", length: "17m" },
  { id: "session-3", title: "Outline chapter beats", indicator: "synced", chips: { messages: "31", tools: "4", tokens: "240k" }, when: "Aug 27, 2026 · 17:40", length: "1h 05m" },
];

const PULL_SESSIONS: MockSession[] = [
  { id: "s4", title: "Fix the cold open pacing", indicator: "storage", chips: { messages: "18", tools: "2", tokens: "121k" }, when: "Aug 30, 2026 · 11:31", length: "26m" },
  { id: "s5", title: "Draft the ghost story ending", indicator: "storage", chips: { messages: "29", tools: "9", tokens: "203k" }, when: "Aug 30, 2026 · 11:52", length: "44m" },
  { id: "s6", title: "Outline chapter beats", indicator: "synced", chips: { messages: "31", tools: "4", tokens: "240k" }, when: "Aug 27, 2026 · 17:40", length: "1h 05m" },
];

const MODE_SESSIONS: Record<ReviewMode, MockSession[]> = { push: SESSIONS, pull: PULL_SESSIONS };
const MODE_SESSION_DEFAULTS: Record<ReviewMode, string[]> = { push: ["s1", "s2"], pull: ["s4", "s5"] };

/* ------------------------------------------------- activity page data ---- */

type ActivityStatus = "new" | "local_ahead" | "synced" | "storage_ahead" | "diverged" | "blocked" | "cloud_only";

const ACTIVITY_STATUS: Record<ActivityStatus, { icon: ThreadStatusIconName; tone: string; tip: string }> = {
  new: { icon: "local-outstanding", tone: "local", tip: "Only on this computer. Push to save it in Attic R2." },
  local_ahead: { icon: "local-outstanding", tone: "local", tip: "Newer on this computer. Push to update Attic R2." },
  synced: { icon: "synced", tone: "synced", tip: "Up to date here and in Attic R2." },
  storage_ahead: { icon: "cloud-outstanding", tone: "storage", tip: "Newer in Attic R2. Pull to update this computer." },
  diverged: { icon: "divergent", tone: "diverged", tip: "Changed here and in Attic R2. Review before syncing." },
  blocked: { icon: "unavailable", tone: "unavailable", tip: "Unavailable for sync." },
  cloud_only: { icon: "cloud-outstanding", tone: "storage", tip: "Only in Attic R2. Pull to download it here." },
};

interface ActivityFile {
  id: string;
  name: string;
  group: string | null;
  status: ActivityStatus;
  size: string;
  stamp: string;
  warning?: { lead: string; body: string };
  blockedReason?: string;
}

/** The local walk: every state a scanned file can be in. */
const ACTIVITY_FILES: ActivityFile[] = [
  { id: "v1", name: "cold-joke.md", group: null, status: "new", size: "3.9 KB", stamp: "Aug 29, 2026 at 22:41" },
  { id: "v2", name: "ghost_story.docx", group: null, status: "local_ahead", size: "36 KB", stamp: "Aug 30, 2026 at 09:12" },
  { id: "v3", name: "old_draft.md", group: null, status: "synced", size: "9.4 KB", stamp: "Aug 12, 2026 at 16:20" },
  { id: "v4", name: "soundtrack.mp3", group: null, status: "storage_ahead", size: "8.2 MB", stamp: "Aug 20, 2026 at 20:14" },
  { id: "v5", name: "cover-blurb.md", group: null, status: "diverged", size: "1.8 KB", stamp: "Aug 29, 2026 at 18:55" },
  { id: "v6", name: "poster.psd", group: null, status: "blocked", size: "48 MB", stamp: "Aug 18, 2026 at 13:37", blockedReason: "File exceeds the 16 MiB project-content limit" },
  { id: "v7", name: "outline.md", group: "notes", status: "synced", size: "12 KB", stamp: "Aug 27, 2026 at 18:05" },
  { id: "v8", name: "render.sh", group: "notes", status: "new", size: "2.1 KB", stamp: "Aug 28, 2026 at 11:33", warning: { lead: "Executable content", body: "notes/render.sh" } },
  { id: "v9", name: "api_key.txt", group: "notes", status: "new", size: "431 B", stamp: "Aug 28, 2026 at 11:41", warning: { lead: "Credential-shaped content", body: "notes/api_key.txt" } },
  { id: "v10", name: "chapter1.md", group: "notes/drafts", status: "new", size: "5.6 KB", stamp: "Aug 26, 2026 at 10:12" },
  { id: "v11", name: "title-page.png", group: "assets", status: "synced", size: "310 KB", stamp: "Aug 25, 2026 at 15:02" },
];

/** The storage side this Mac does not hold: a loose file and a whole folder. */
const ACTIVITY_CLOUD_FILES: ActivityFile[] = [
  { id: "c1", name: "press-kit.pdf", group: null, status: "cloud_only", size: "2.4 MB", stamp: "Aug 22, 2026 at 12:30" },
  { id: "c2", name: "interview.md", group: "archive", status: "cloud_only", size: "18 KB", stamp: "Aug 15, 2026 at 09:40" },
  { id: "c3", name: "old_cover.png", group: "archive", status: "cloud_only", size: "840 KB", stamp: "Aug 15, 2026 at 09:41" },
];

const ACTIVITY_GROUPS = ["notes", "notes/drafts", "assets"];
const ACTIVITY_GROUP_FACTS: Record<string, string> = {
  notes: "4 files · 20.1 KB",
  "notes/drafts": "1 file · 5.6 KB",
  assets: "1 file · 310 KB",
  archive: "2 files · 858 KB",
};

interface ActivitySession {
  id: string;
  title: string;
  status: ActivityStatus;
  chips: { messages: string; tools?: string; tokens: string };
  when: string;
  length: string;
}

const ACTIVITY_SESSIONS: ActivitySession[] = [
  { id: "as1", title: "Draft the ghost story ending", status: "new", chips: { messages: "24", tools: "9", tokens: "182k" }, when: "Aug 30, 2026 · 09:02", length: "38m" },
  { id: "as2", title: "Punch up the cold-joke timing", status: "local_ahead", chips: { messages: "11", tokens: "64k" }, when: "Aug 29, 2026 · 22:18", length: "17m" },
  { id: "as3", title: "Outline chapter beats", status: "synced", chips: { messages: "31", tools: "4", tokens: "240k" }, when: "Aug 27, 2026 · 17:40", length: "1h 05m" },
];

const ACTIVITY_CLOUD_SESSIONS: ActivitySession[] = [
  { id: "as4", title: "Brainstorm sequel hooks", status: "cloud_only", chips: { messages: "18", tokens: "97k" }, when: "Aug 21, 2026 · 20:10", length: "26m" },
];

/* ------------------------------------------------------------- helpers --- */

function toggleIn(set: Set<string>, id: string): Set<string> {
  const next = new Set(set);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

function useFilesState(model: ReviewModel = PUSH_MODEL) {
  const [selected, setSelected] = useState<Set<string>>(() => new Set(model.defaults));
  const [acked, setAcked] = useState<Set<string>>(() => new Set());
  const [removals, setRemovals] = useState<Set<string>>(() => new Set());
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const selectable = (file: MockFile) => file.status !== "blocked" && !removals.has(file.id);
  return {
    model,
    selected,
    acked,
    removals,
    collapsed,
    selectable,
    isSelected: (file: MockFile) => selectable(file) && selected.has(file.id),
    toggleSelected: (id: string) => setSelected((current) => toggleIn(current, id)),
    toggleAcked: (id: string) => setAcked((current) => toggleIn(current, id)),
    toggleRemoval: (id: string) => setRemovals((current) => toggleIn(current, id)),
    toggleCollapsed: (id: string) => setCollapsed((current) => toggleIn(current, id)),
    /* The folder cluster's two verbs: select the whole subtree, or clear it. */
    setGroup: (group: string, shouldSelect: boolean) => {
      const ids = model.files.filter((file) => inSubtree(file, group) && selectable(file)).map((file) => file.id);
      setSelected((current) => {
        const next = new Set(current);
        ids.forEach((id) => { if (shouldSelect) next.add(id); else next.delete(id); });
        return next;
      });
    },
    /* The pull toolbar's verb: additions bring nothing local into question. */
    selectSafeAdditions: () => {
      const ids = model.files.filter((file) => file.op === "add").map((file) => file.id);
      setSelected((current) => new Set([...current, ...ids]));
    },
  };
}

type FilesStateApi = ReturnType<typeof useFilesState>;

function groupCounts(state: FilesStateApi, group: string): { picked: number; total: number } {
  const ids = state.model.files.filter((file) => inSubtree(file, group) && state.selectable(file)).map((file) => file.id);
  return { picked: ids.filter((id) => state.selected.has(id)).length, total: ids.length };
}

/* ------------------------------------------------- sessions step (ctx) --- */

function SessionCardMock({ session, selected, onToggle }: {
  session: MockSession;
  selected: boolean;
  onToggle: () => void;
}) {
  return (
    <article className={`sync-history-thread-card sync-sync-selectable-row${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${session.title}`}>
            <input type="checkbox" checked={selected} onChange={onToggle} />
          </label>
          <span
            className={`sync-thread-sync-indicator ${session.indicator}`}
            data-tooltip={SESSION_TIPS[session.indicator]}
            tabIndex={0}
          >
            <ThreadStatusIcon
              status={session.indicator === "local" ? "local-outstanding" : session.indicator === "storage" ? "cloud-outstanding" : "synced"}
              size={16}
            />
          </span>
          <strong>{session.title}</strong>
        </div>
        <div className="sync-history-thread-actions">
          <span className="sync-history-launch-action sync-history-provider-inert" title="Claude session" aria-label="Claude session">
            <ProviderMark provider="claude" displayName="Claude" />
          </span>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label="Session details">
            <span className="sync-history-thread-metric" data-tooltip={`Messages · ${session.chips.messages}`} tabIndex={0}><Icon name="message" size={12} /><b>{session.chips.messages}</b></span>
            {session.chips.tools && <span className="sync-history-thread-metric" data-tooltip={`Tool calls · ${session.chips.tools}`} tabIndex={0}><Icon name="tool" size={12} /><b>{session.chips.tools}</b></span>}
            <span className="sync-history-thread-metric" data-tooltip={`Total tokens · ${session.chips.tokens}`} tabIndex={0}><Icon name="token" size={12} /><b>{session.chips.tokens}</b></span>
            <span className="sync-history-thread-metric sync-history-thread-duration" data-tooltip={`Length · ${session.length}`} tabIndex={0}><Icon name="clock" size={12} /><b>{session.length}</b></span>
            <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`Started · ${session.when}`} tabIndex={0}>{session.when}</span>
          </div>
        </div>
      </div>
    </article>
  );
}

/* ---------------------------------------------------- the file card ------ */

function AckChipMock({ lead, body, checked, onToggle }: {
  lead: string;
  body: string;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <label className="catalog-card-ack">
      <span className="sync-sync-row-choice">
        <input type="checkbox" checked={checked} aria-label={`Reviewed: ${lead} — ${body}`} onChange={onToggle} />
      </span>
      <Icon name="alert-triangle" size={12} />
      <span className="catalog-card-ack-copy"><strong>{lead}</strong> — {body}</span>
      <small className={checked ? "done" : undefined}>{checked ? "Reviewed" : "Mark reviewed"}</small>
    </label>
  );
}

/** Every file — root or grouped, push or pull — is this same standard card. */
function FileCard({ file, state }: {
  file: MockFile;
  state: FilesStateApi;
}) {
  const removed = state.removals.has(file.id);
  const selectable = state.selectable(file);
  const selected = state.isSelected(file);
  const pullRow = file.op != null;
  const onDisk = file.op !== "add";
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectable && (
            <label className="sync-sync-row-choice" title={`${selected ? (pullRow ? "Keep local" : "Exclude") : (pullRow ? "Apply" : "Include")} ${file.name}`}>
              <input
                type="checkbox"
                checked={selected}
                aria-label={`${selected ? (pullRow ? "Keep local" : "Exclude") : (pullRow ? "Apply" : "Include")} ${file.name}`}
                onChange={() => state.toggleSelected(file.id)}
              />
            </label>
          )}
          <span className={`sync-thread-sync-indicator ${CARD_TONE[file.status]}`} data-tooltip={file.tip} aria-label={file.tip} tabIndex={0}>
            <ThreadStatusIcon status={CARD_STATUS[file.status]} size={18} />
          </span>
          <strong title={file.name}>{file.name}</strong>
          {file.op && (
            <span className="sync-history-branch-state history-only" title={file.tip}>
              {file.preservedCopy ? "Replace file · keeps local copy" : OP_CHIP[file.op]}
            </span>
          )}
          {file.blockedReason && (
            <span className="sync-history-branch-state history-only" title={file.blockedReason}>Blocked</span>
          )}
          {removed && (
            <span className="sync-history-branch-state history-only">Removes from storage</span>
          )}
        </div>
        <div className="sync-history-thread-actions">
          {file.storagePresent && !pullRow && (
            <button
              type="button"
              className="btn btn-ghost sync-history-launch-action"
              title={removed ? "Keep in storage" : "Remove from storage"}
              aria-label={removed ? `Keep ${file.name} in storage` : `Remove ${file.name} from storage`}
              onClick={() => state.toggleRemoval(file.id)}
            >
              <Icon name={removed ? "refresh" : "trash"} size={14} />
            </button>
          )}
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!onDisk}
            title={onDisk ? `Reveal ${file.name} in Finder` : "Not on this Mac"}
            aria-label={onDisk ? `Reveal ${file.name} in Finder` : `Cannot reveal ${file.name}: not on this Mac`}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${file.name} details`}>
            <span className="sync-history-thread-metric" data-tooltip={`${onDisk ? "On disk" : "In storage"} · ${file.size}`} tabIndex={0}><Icon name="drive" size={12} /><b>{file.size}</b></span>
            {file.warning
              ? <span className="sync-history-thread-metric sync-history-partial" data-tooltip={`${file.warning.lead} — ${file.warning.body}`} tabIndex={0}><Icon name="alert-triangle" size={12} /><b>1</b></span>
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <button
              type="button"
              className="btn btn-ghost sync-history-session-detail-action"
              title={file.blockedReason ?? `Path, mode, and digests for ${file.name}`}
              aria-label={`Details for ${file.name}`}
            >
              <Icon name="info" size={13} />
            </button>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`${pullRow ? "Updated in Attic R2" : "Modified"} · ${file.stamp}`} tabIndex={0}>
            <time>{file.stamp}</time>
          </span>
        </div>
      </div>
      {selected && file.warning && (() => {
        const warning = file.warning;
        return (
          <AckChipMock
            lead={warning.lead}
            body={warning.body}
            checked={state.acked.has(warning.id)}
            onToggle={() => state.toggleAcked(warning.id)}
          />
        );
      })()}
    </article>
  );
}

/* -------------------------------------------------- folder treatments --- */

/** The review header's own cluster — count, select-all, clear — reused at
 * folder scale over the folder's subtree. */
function GroupSelectionCluster({ group, state }: { group: string; state: FilesStateApi }) {
  const { picked, total } = groupCounts(state, group);
  return (
    <div className="sync-push-resource-actions">
      <span
        className="sync-sync-review-selected"
        style={{ "--review-count-digits": String(total).length } as React.CSSProperties}
        aria-live="polite"
        aria-label={`${picked} of ${total} selected in ${group}`}
        title={`${picked} of ${total} selected in ${group}/`}
      >
        <b>{picked}</b>/{total}
      </span>
      <button
        type="button"
        className="btn btn-ghost sync-sync-review-icon-action"
        disabled={picked === total}
        title={`Select all files in ${group}/`}
        aria-label={`Select all files in ${group}`}
        onClick={() => state.setGroup(group, true)}
      >
        <Icon name="check-circle" size={15} />
      </button>
      <button
        type="button"
        className="btn btn-ghost sync-sync-review-icon-action"
        disabled={picked === 0}
        title={`Clear the ${group}/ selection`}
        aria-label={`Clear the ${group} selection`}
        onClick={() => state.setGroup(group, false)}
      >
        <Icon name="trash" size={15} />
      </button>
    </div>
  );
}

function GroupLabel({ group, state, leaf = false }: { group: string; state: FilesStateApi; leaf?: boolean }) {
  return (
    <>
      <Icon name="folder" size={14} />
      <strong>{leaf ? `${group.split("/").slice(-1)[0]}/` : `${group}/`}</strong>
      <small>{state.model.facts[group]}</small>
      {state.model.newGroups.has(group) && (
        <span className="sync-history-branch-state history-only" title="This folder does not exist locally yet; applying creates it.">New folder</span>
      )}
    </>
  );
}

/** V1 — a slim heading band; the cards below stay full-width standard cards. */
function GroupHeadingBand({ group, state }: { group: string; state: FilesStateApi }) {
  return (
    <div className="mk-group-heading">
      <GroupLabel group={group} state={state} />
      <GroupSelectionCluster group={group} state={state} />
    </div>
  );
}

/** V2 — the folder frames its files; rows join borders inside the frame.
 * A nested folder nests its own frame, selective-sync style. */
function FolderContainer({ group, state }: {
  group: string;
  state: FilesStateApi;
}) {
  const files = state.model.files.filter((file) => file.group === group);
  const childGroups = state.model.childGroups[group] ?? [];
  const folded = state.collapsed.has(group);
  return (
    <div className={`mk-folder-card${folded ? " collapsed" : ""}`}>
      <div className="mk-group-heading">
        <GroupLabel group={group} state={state} leaf />
        <GroupSelectionCluster group={group} state={state} />
        <button
          type="button"
          className="btn btn-ghost sync-history-launch-action mk-group-collapse"
          title={folded ? `Show files in ${group}/` : `Hide files in ${group}/`}
          aria-expanded={!folded}
          onClick={() => state.toggleCollapsed(group)}
        >
          <Icon name={folded ? "chevron-down" : "chevron-up"} size={14} />
        </button>
      </div>
      {!folded && (
        <>
          <div className="sync-history-thread-list">
            {files.map((file) => <FileCard key={file.id} file={file} state={state} />)}
          </div>
          {childGroups.map((child) => <FolderContainer key={child} group={child} state={state} />)}
        </>
      )}
    </div>
  );
}

/** V3 — a hairline divider carrying the same cluster. */
function GroupDivider({ group, state }: { group: string; state: FilesStateApi }) {
  return (
    <div className="mk-group-divider">
      <GroupLabel group={group} state={state} />
      <span className="mk-rule" aria-hidden="true" />
      <GroupSelectionCluster group={group} state={state} />
    </div>
  );
}

/* ------------------------------------------------- activity page mock ---- */

/** An activity session row: no selection — status, launches, and readouts. */
function ActivitySessionCard({ session }: { session: ActivitySession }) {
  const presentation = ACTIVITY_STATUS[session.status];
  const remote = session.status === "cloud_only";
  return (
    <article className="sync-history-thread-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <span className={`sync-thread-sync-indicator ${presentation.tone}`} data-tooltip={presentation.tip} aria-label={presentation.tip} tabIndex={0}>
            <ThreadStatusIcon status={presentation.icon} size={16} />
          </span>
          <strong>{session.title}</strong>
        </div>
        <div className="sync-history-thread-actions">
          {remote ? (
            <span className="sync-history-launch-action sync-history-provider-inert" title="Claude session — not on this Mac" aria-label="Claude session, not on this Mac">
              <ProviderMark provider="claude" displayName="Claude" />
            </span>
          ) : (
            <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Claude" aria-label="Open in Claude">
              <ProviderMark provider="claude" displayName="Claude" />
            </button>
          )}
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label="Session details">
            <span className="sync-history-thread-metric" data-tooltip={`Messages · ${session.chips.messages}`} tabIndex={0}><Icon name="message" size={12} /><b>{session.chips.messages}</b></span>
            {session.chips.tools && <span className="sync-history-thread-metric" data-tooltip={`Tool calls · ${session.chips.tools}`} tabIndex={0}><Icon name="tool" size={12} /><b>{session.chips.tools}</b></span>}
            <span className="sync-history-thread-metric" data-tooltip={`Total tokens · ${session.chips.tokens}`} tabIndex={0}><Icon name="token" size={12} /><b>{session.chips.tokens}</b></span>
            <span className="sync-history-thread-metric sync-history-thread-duration" data-tooltip={`Length · ${session.length}`} tabIndex={0}><Icon name="clock" size={12} /><b>{session.length}</b></span>
            <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`Started · ${session.when}`} tabIndex={0}>{session.when}</span>
          </div>
        </div>
      </div>
    </article>
  );
}

/** An activity file row: the review card, read-only — no checkbox, no acks. */
function ActivityFileCard({ file }: { file: ActivityFile }) {
  const presentation = ACTIVITY_STATUS[file.status];
  const onDisk = file.status !== "cloud_only";
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <span className={`sync-thread-sync-indicator ${presentation.tone}`} data-tooltip={presentation.tip} aria-label={presentation.tip} tabIndex={0}>
            <ThreadStatusIcon status={presentation.icon} size={18} />
          </span>
          <strong title={file.name}>{file.name}</strong>
          {file.blockedReason && (
            <span className="sync-history-branch-state history-only" title={file.blockedReason}>Blocked</span>
          )}
        </div>
        <div className="sync-history-thread-actions">
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!onDisk}
            title={onDisk ? `Reveal ${file.name} in Finder` : "Not on this Mac"}
            aria-label={onDisk ? `Reveal ${file.name} in Finder` : `Cannot reveal ${file.name}: not on this Mac`}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${file.name} details`}>
            <span className="sync-history-thread-metric" data-tooltip={`${onDisk ? "On disk" : "In storage"} · ${file.size}`} tabIndex={0}><Icon name="drive" size={12} /><b>{file.size}</b></span>
            {file.warning
              ? <span className="sync-history-thread-metric sync-history-partial" data-tooltip={`${file.warning.lead} — ${file.warning.body}`} tabIndex={0}><Icon name="alert-triangle" size={12} /><b>1</b></span>
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <button
              type="button"
              className="btn btn-ghost sync-history-session-detail-action"
              title={file.blockedReason ?? `Path, mode, and digests for ${file.name}`}
              aria-label={`Details for ${file.name}`}
            >
              <Icon name="info" size={13} />
            </button>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`${onDisk ? "Modified" : "Updated in Attic R2"} · ${file.stamp}`} tabIndex={0}>
            <time>{file.stamp}</time>
          </span>
        </div>
      </div>
    </article>
  );
}

/** The activity list's folder band: label and facts, no selection cluster. */
function ActivityGroupHeading({ group }: { group: string }) {
  return (
    <div className="mk-group-heading">
      <Icon name="folder" size={14} />
      <strong>{group}/</strong>
      <small>{ACTIVITY_GROUP_FACTS[group]}</small>
    </div>
  );
}

/** The storage-only partition, named after the storage the way the sessions
 * activity names its group. */
function CloudHeading({ count }: { count: number }) {
  return (
    <div className="mk-cloud-heading">
      <Icon name="cloud" size={14} />
      <strong>Only in Attic R2</strong>
      <small>{count} item{count === 1 ? "" : "s"} · Pull to download</small>
    </div>
  );
}

function ActivityFilesPanel() {
  const rootFiles = ACTIVITY_FILES.filter((file) => file.group === null);
  const cloudRoot = ACTIVITY_CLOUD_FILES.filter((file) => file.group === null);
  return (
    <section className="sync-project-files-page" aria-label="Project files">
      <div className="sync-project-files-search">
        <input type="search" placeholder="Search files…" aria-label="Search project file paths" />
        <span>{ACTIVITY_FILES.length + ACTIVITY_CLOUD_FILES.length} entries</span>
      </div>
      <div className="mk-cloud-block">
        <CloudHeading count={ACTIVITY_CLOUD_FILES.length} />
        <div className="sync-history-thread-list flat">
          {cloudRoot.map((file) => <ActivityFileCard key={file.id} file={file} />)}
        </div>
        <ActivityGroupHeading group="archive" />
        <div className="sync-history-thread-list flat">
          {ACTIVITY_CLOUD_FILES.filter((file) => file.group === "archive").map((file) => (
            <ActivityFileCard key={file.id} file={file} />
          ))}
        </div>
      </div>
      <div className="sync-history-thread-list flat">
        {rootFiles.map((file) => <ActivityFileCard key={file.id} file={file} />)}
      </div>
      {ACTIVITY_GROUPS.map((group) => (
        <div key={group}>
          <ActivityGroupHeading group={group} />
          <div className="sync-history-thread-list flat">
            {ACTIVITY_FILES.filter((file) => file.group === group).map((file) => (
              <ActivityFileCard key={file.id} file={file} />
            ))}
          </div>
        </div>
      ))}
      <details className="sync-project-files-scan-summary">
        <summary>1 ignored · 1 blocked · 2 scan warnings</summary>
        <p>Ignore file '.mallardignore' matched 'drafts/'.</p>
      </details>
    </section>
  );
}

function ActivitySessionsPanel() {
  return (
    <>
      <div className="mk-cloud-block">
        <CloudHeading count={ACTIVITY_CLOUD_SESSIONS.length} />
        <div className="sync-history-thread-list flat">
          {ACTIVITY_CLOUD_SESSIONS.map((session) => <ActivitySessionCard key={session.id} session={session} />)}
        </div>
      </div>
      <div className="sync-history-thread-list flat">
        {ACTIVITY_SESSIONS.map((session) => <ActivitySessionCard key={session.id} session={session} />)}
      </div>
    </>
  );
}

/** The non-git project's activity page. No tab strip and no panel title:
 * the path bar's leaf crumb IS the switch — click "Sessions" to see Files,
 * click "Files" to go back — and the bar carries the view-scoped Rescan.
 * Both lists lead with the "Only in Attic R2" partition, the way the
 * sessions activity page puts storage-only work on top. */
function MockActivity() {
  const [tab, setTab] = useState<"sessions" | "files">("sessions");
  const other = tab === "sessions" ? "Files" : "Sessions";
  return (
    <section className="sync-inline-action-review sync-sync-review-workspace" aria-label="Activity page mock">
      <div className="mk-pathbar">
        <Icon name="folder" size={15} />
        <strong>testProjectNonGit</strong>
        <Icon name="chevron-down" size={12} />
        <span className="mk-pathbar-sep">/</span>
        <button
          type="button"
          className="mk-pathbar-leaf"
          title={`Switch to ${other}`}
          aria-label={`${tab === "sessions" ? "Sessions" : "Files"} — switch to ${other}`}
          onClick={() => setTab(tab === "sessions" ? "files" : "sessions")}
        >
          {tab === "sessions" ? "Sessions" : "Files"}
        </button>
        <button
          type="button"
          className="btn btn-ghost sync-history-launch-action mk-pathbar-action"
          title={tab === "files" ? "Rescan project files" : "Refresh sessions"}
          aria-label={tab === "files" ? "Rescan project files" : "Refresh sessions"}
        >
          <Icon name="refresh" size={14} />
        </button>
      </div>
      <div className="sync-inline-action-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          {tab === "sessions" ? <ActivitySessionsPanel /> : <ActivityFilesPanel />}
        </div>
      </div>
    </section>
  );
}

/* -------------------------------------------------------- review shell --- */

type FlowVariant = "footer" | "tabs" | "both";
type FolderVariant = "heading" | "container" | "divider";

function FilesPanel({ variant, state, mode = "push" }: {
  variant: FolderVariant;
  state: FilesStateApi;
  mode?: ReviewMode;
}) {
  const rootFiles = state.model.files.filter((file) => file.group === null);
  return (
    <section className="sync-project-files-page" aria-label="Project files">
      {/* One line: the tab already says "Project files", so the bar spends
          its row on the controls — search, the tally, and the step's verb. */}
      <div className="sync-project-files-search">
        <input type="search" placeholder="Search files…" aria-label="Search project file paths" />
        <span>{state.model.files.length} entries</span>
        {mode === "push"
          ? <button type="button" className="btn btn-ghost"><Icon name="refresh" size={14} /> Rescan</button>
          : (
            <button type="button" className="btn btn-ghost" onClick={() => state.selectSafeAdditions()}>
              <Icon name="check-circle" size={14} /> Select safe additions
            </button>
          )}
      </div>
      <div className="sync-history-thread-list flat">
        {rootFiles.map((file) => <FileCard key={file.id} file={file} state={state} />)}
      </div>
      {variant === "container"
        ? state.model.topGroups.map((group) => (
          <FolderContainer key={group} group={group} state={state} />
        ))
        : state.model.flatGroups.map((group) => (
          <div key={group}>
            {variant === "heading"
              ? <GroupHeadingBand group={group} state={state} />
              : <GroupDivider group={group} state={state} />}
            <div className="sync-history-thread-list flat">
              {state.model.files.filter((file) => file.group === group).map((file) => (
                <FileCard key={file.id} file={file} state={state} />
              ))}
            </div>
          </div>
        ))}
      {mode === "push" && (
        <details className="sync-project-files-scan-summary">
          <summary>1 ignored · 1 blocked · 2 scan warnings</summary>
          <p>Ignore file '.mallardignore' matched 'drafts/'.</p>
        </details>
      )}
    </section>
  );
}

function ContinueButton({ step, onContinue }: {
  step: "history" | "project_files";
  onContinue: () => void;
}) {
  const forward = step === "history";
  return (
    <button
      type="button"
      className="btn btn-ghost"
      onClick={onContinue}
      title={forward ? "Review project files next" : "Back to session selection"}
    >
      {!forward && <Icon name="chevron-left" size={14} />}
      {forward ? "Project files" : "Sessions"}
      {forward && <Icon name="chevron-right" size={14} />}
    </button>
  );
}

function MockReview({ flow, folders, mode = "push", initialStep = "history" }: {
  flow: FlowVariant;
  folders: FolderVariant;
  mode?: ReviewMode;
  initialStep?: "history" | "project_files";
}) {
  const model = MODELS[mode];
  const sessions = MODE_SESSIONS[mode];
  const [step, setStep] = useState<"history" | "project_files">(initialStep);
  const [sessionSelection, setSessionSelection] = useState<Set<string>>(() => new Set(MODE_SESSION_DEFAULTS[mode]));
  const filesState = useFilesState(model);
  const selectedFiles = model.files.filter((file) => filesState.isSelected(file));
  const counts = { history: sessionSelection.size, project_files: selectedFiles.length };
  const totals = { history: sessions.length, project_files: model.files.length };
  const unacked = mode === "push" && selectedFiles.some((file) => file.warning && !filesState.acked.has(file.warning.id));
  const jump = () => setStep(step === "history" ? "project_files" : "history");
  const title = mode === "push" ? "Push review" : "Pull review";
  return (
    <section className={`sync-inline-action-review ${mode === "push" ? "sync-inline-push-review" : "sync-inline-pull-review"} sync-sync-review-workspace`} aria-label={`${title} mock`}>
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title"><h2>{title}</h2></div>
        <SyncReviewSelectionBar
          count={counts[step]}
          total={totals[step]}
          recommendedCount={totals[step]}
          recommendedTitle="Use the recommended selection"
          recommendedDisabled={false}
          onRecommended={() => undefined}
          clearDisabled={counts[step] === 0}
          onClear={() => undefined}
          close={{ ariaLabel: `Close ${title.toLowerCase()}`, onClose: () => undefined }}
        />
      </header>
      <div className="mk-tabrow">
        <div className="mk-tabwrap">
          <SyncReviewTabs
            tabs={[
              { id: "history", label: "Sessions", icon: "message", count: counts.history },
              { id: "project_files", label: "Project files", icon: "file", count: counts.project_files },
            ]}
            activeId={step}
            onChange={(id) => setStep(id as "history" | "project_files")}
          />
        </div>
        {(flow === "tabs" || flow === "both") && step === "history" && (
          <div className="mk-tab-cta-zone">
            <ContinueButton step={step} onContinue={jump} />
          </div>
        )}
      </div>
      <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          {step === "history"
            ? (
              <div className="sync-history-thread-list flat">
                {sessions.map((session) => (
                  <SessionCardMock
                    key={session.id}
                    session={session}
                    selected={sessionSelection.has(session.id)}
                    onToggle={() => setSessionSelection((current) => toggleIn(current, session.id))}
                  />
                ))}
              </div>
            )
            : <FilesPanel variant={folders} state={filesState} mode={mode} />}
        </div>
      </div>
      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {step === "project_files" && unacked && (
          <span><small>Acknowledge the warnings above before pushing.</small></span>
        )}
        {(flow === "footer" || flow === "both") && <ContinueButton step={step} onContinue={jump} />}
        <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={step === "project_files" && unacked}>
          <ThreadStatusIcon status={mode === "push" ? "local-outstanding" : "cloud-outstanding"} size={16} />
          {mode === "push" ? "Push" : "Apply"}
        </button>
      </footer>
    </section>
  );
}

/* ---------------------------------------------------------------- page --- */

function Page() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  document.documentElement.dataset.theme = theme;
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Non-git Push review — sessions first, then files</h1>
      <p className="mk-lede">
        Section A places the step control that carries the reader from session selection to
        file selection. Section B: every file — root or inside a folder — is the same
        standard Catalog-grammar card with its own checkbox, warning chip, and actions; the
        variants differ only in what the folder itself is. Cloud-only rows are out of scope
        for Push here. Every frame is interactive.
      </p>
      <div className="mk-bar" role="group" aria-label="Theme">
        <button type="button" className={theme === "light" ? "on" : undefined} onClick={() => setTheme("light")}>Light</button>
        <button type="button" className={theme === "dark" ? "on" : undefined} onClick={() => setTheme("dark")}>Dark</button>
      </div>

      <div className="mk-section">
        <h2>A — Where &quot;Continue to files&quot; lives</h2>
        <p>
          In every option Push stays visible and enabled on both steps — the control is a
          navigator, not a gate. It flips to &quot;‹ Sessions&quot; on the files step.
        </p>
        <div className="mk-variant">
          <h3>A1 · Footer companion</h3>
          <p>A ghost button beside Push. The footer reads as &quot;what happens next&quot;: continue reviewing, or publish now.</p>
          <div className="mk-frame"><MockReview flow="footer" folders="heading" /></div>
        </div>
        <div className="mk-variant">
          <h3>A2 · Tab-strip CTA (top)</h3>
          <p>The strip&apos;s right edge carries the forward affordance; the footer keeps a single primary action. Disappears once you are on the files step — the tabs themselves take over.</p>
          <div className="mk-frame"><MockReview flow="tabs" folders="heading" /></div>
        </div>
        <div className="mk-variant">
          <h3>A3 · Both ends</h3>
          <p>Same button at top and bottom, for long session lists where the footer is far away.</p>
          <div className="mk-frame"><MockReview flow="both" folders="heading" /></div>
        </div>
      </div>

      <div className="mk-section">
        <h2>B — The folder, three ways</h2>
        <p>
          Same data in all three: four root files (one synced, one blocked), a
          &quot;notes&quot; folder holding both warnings plus a nested
          &quot;notes/drafts&quot; folder, and an &quot;assets&quot; folder. Files behave
          identically everywhere — own checkbox, own &quot;Mark reviewed&quot; chip, own
          remove control. Each folder reuses the review header&apos;s own cluster —
          count, select-all, clear — over its whole subtree, so a partial pick reads as
          &quot;3/5&quot;. Only the folder&apos;s presence changes.
        </p>
        <div className="mk-variant">
          <h3>B1 · Heading band</h3>
          <p>A slim band above the cards, one per directory that holds files, labeled with the full path and closed by the cluster — nesting reads as &quot;notes/drafts/&quot; without indentation, the way VS Code&apos;s flat source-control view groups by folder. The cards stay full-width.</p>
          <div className="mk-frame"><MockReview flow="footer" folders="heading" initialStep="project_files" /></div>
        </div>
        <div className="mk-variant">
          <h3>B2 · Folder container (tree)</h3>
          <p>The folder frames its files, and a nested folder nests its own frame — the selective-sync / IDE-commit-dialog standard. Strongest containment and the clearest nesting; costs a border level per depth (the scan caps depth at 32, so deep trees need the fold control).</p>
          <div className="mk-frame"><MockReview flow="footer" folders="container" initialStep="project_files" /></div>
        </div>
        <div className="mk-variant">
          <h3>B3 · Hairline divider</h3>
          <p>The lightest structure: a rule between the folder label and its cluster. Nested folders read as full-path dividers, like B1.</p>
          <div className="mk-frame"><MockReview flow="footer" folders="divider" initialStep="project_files" /></div>
        </div>
      </div>

      <div className="mk-section">
        <h2>C — The activity page: Sessions ⇄ Files</h2>
        <p>
          The non-git project&apos;s standing activity page. No tab strip and no panel
          title: the path bar&apos;s leaf crumb IS the switch — click
          &quot;Sessions&quot; to see Files, click &quot;Files&quot; to come back — and the
          bar&apos;s refresh is view-scoped (Rescan on Files). Both lists are read-only:
          no checkboxes, no ack chips — statuses, launches, and readouts only. Like the
          sessions activity page, the &quot;Only in Attic R2&quot; partition leads each
          list: a cloud-only file and a whole cloud-only folder on Files (reveal
          disabled: not on this Mac), a cloud-only session on Sessions. Below it, the
          local list covers every state: new, newer here, synced, newer in storage,
          diverged, blocked.
        </p>
        <div className="mk-variant">
          <h3>C1 · Files activity, all states</h3>
          <p>Opens on Sessions, like the app. Click the leaf crumb to switch. Folder bands carry facts only — selection clusters belong to reviews.</p>
          <div className="mk-frame"><MockActivity /></div>
        </div>
      </div>

      <div className="mk-section">
        <h2>D — Pull: the same grammar over the pull cases</h2>
        <p>
          One plan covers every pull operation: an added file (only in storage), a
          replacement (storage newer), a diverged file (changed both sides), a deletion
          (removed in storage), and a new &quot;assets/audio&quot; folder whose creation
          rides its first file. The toolbar verb becomes &quot;Select safe additions&quot;,
          the footer verb becomes Apply, and an unticked row simply keeps the local file —
          skipping is a valid review outcome. Reveal is disabled on added files: not on
          this Mac yet.
        </p>
        <div className="mk-variant">
          <h3>D1 · Checkbox is the whole choice — diverged rows always keep a copy</h3>
          <p>
            The implemented backend behavior: ticked means apply what storage has,
            unticked means keep local, and a diverged replacement is never lossy — before
            the storage version is written, the local version is copied to a pinned
            sibling (here &quot;ghost_story-local-copy.docx&quot;), catalog-style. The
            chip reads &quot;Replace file · keeps local copy&quot;; the copy then shows up
            in the next Push scan as an ordinary new file. There is no per-row action
            menu — an unwanted copy is one delete away. Deletes still start unticked and
            back up before removing.
          </p>
          <div className="mk-frame"><MockReview flow="footer" folders="heading" mode="pull" initialStep="project_files" /></div>
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
