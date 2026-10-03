import { StrictMode, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import { NoticeChip } from "../src/components/notice/Notice";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../src/components/sync-review/ThreadStatusIcon";
import { nameMatchesSearch } from "../src/components/sync-review/nameSearch";
import ProjectViewSwitch, { type ProjectView } from "../src/components/project-sync/ProjectViewSwitch";
import {
  ProjectChatHistoryContent,
  SessionMetrics,
  compactThreadTitle,
  sessionSyncPresentation,
} from "../src/components/project-sync/ProjectActivityPage";
import { ProjectFilesActivityView } from "../src/components/project-sync/ProjectFilesActivityPage";
import SessionRowIcon from "../src/components/project-sync/SessionRowIcon";
import { fileKindLabel, projectFileWarningLabel } from "../src/components/project-sync/model";
import { formatBytes } from "../src/format";
import { formatFullDateTime } from "../src/clock";
import type {
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectContentEntry,
  ProjectContentInventory,
  ProjectSessionSummary,
  ResourceSyncState,
  SessionActionDescriptor,
  SessionSourceState,
  ThreadSyncComparison,
  ThreadSyncEntry,
  ThreadSyncState,
} from "../src/types";
import "../src/App.css";

/**
 * Mockup: the two faces of a non-Git project — Sessions and Files — side by
 * side, today and proposed, so their shared chrome can be compared directly.
 *
 * "Today" renders the shipped components (`ProjectChatHistoryContent`,
 * `ProjectFilesActivityView`) on static data. "Proposed" composes the shipped
 * CSS and leaf components into one grammar for both faces: one search row,
 * one refresh, one info glyph, one readout glyph family, one stamp, one tile.
 *
 * `npm run dev`, then /mockups/project-faces-mockup.html
 * (`?theme=light|dark&layout=stacked|side&cloud=tile|inline&marks=on|off`).
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1480px; margin: 0 auto; padding: 24px 22px 90px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 14px; max-width: 100ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { position: sticky; top: 0; z-index: 30; display: flex; gap: 8px; align-items: center; margin: 0 0 22px; padding: 8px 0; flex-wrap: wrap; background: var(--retro-frame-gutter); }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 5px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--bg-active); color: var(--text-0); border-color: var(--border-mid); }
  .mk-bar > span { color: var(--text-3); font-size: var(--type-xs); }
  .mk-variant { margin: 0 0 40px; }
  .mk-variant > h2 { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-lg); }
  .mk-variant > p { margin: 0 0 10px; max-width: 100ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-tag { display: inline-block; margin-left: 8px; padding: 1px 7px; border-radius: var(--r-pill); background: var(--surface-row-active); color: var(--text-0); font-size: var(--type-2xs); font-weight: var(--weight-semibold); vertical-align: 3px; }

  .mk-changes { margin: 0 0 14px; padding: 0; list-style: none; display: grid; gap: 6px; max-width: 120ch; }
  .mk-changes li { display: grid; grid-template-columns: 22px 1fr; gap: 8px; color: var(--text-2); font-size: var(--type-xs); line-height: var(--leading-prose); }
  .mk-changes li b { color: var(--text-0); }
  .mk-changes li a { color: var(--text-2); }
  .mk-num { display: inline-grid; place-items: center; width: 18px; height: 18px; border-radius: 50%; background: var(--blue); color: #fff; font-size: 10px; font-weight: 700; font-family: var(--font-mono); }
  .mk-issues li .mk-num { background: var(--red); }

  .mk-faces { display: grid; gap: 18px; }
  .mk-faces.side { grid-template-columns: 1fr 1fr; align-items: start; }
  .mk-face > h3 { margin: 0 0 6px; color: var(--text-1); font-size: var(--type-sm); font-weight: var(--weight-semibold); }
  .mk-frame { padding: 3px; border: 1px dashed var(--border-mid); border-radius: 10px; background: var(--retro-frame-gutter); }

  /* The workspace, just enough of it: the app root for its retro svg and
     input rules, the main column, the path bar, and the tab panel. */
  .mk-frame > .sync-app { display: block; height: auto; overflow: visible; padding: 0; background: transparent; }
  .mk-frame .sync-main { height: auto; min-height: 0; overflow: visible; padding: 0 18px 8px; border-radius: 8px; background: var(--bg-0); }
  .mk-frame .sync-project-tab-panel { padding-top: 14px; }

  /* Proposed: the search row is the page's first gap, not a second one. */
  .mk-proposed .sync-project-files-page > .sync-name-search { margin-bottom: 0; }

  /* Change markers. */
  .mk-marks .mk-chg { position: relative; outline: 2px dashed color-mix(in srgb, var(--blue) 70%, transparent); outline-offset: 2px; border-radius: 4px; }
  .mk-marks .mk-chg::after { content: attr(data-n); position: absolute; top: -9px; right: -9px; z-index: 5; display: grid; place-items: center; width: 16px; height: 16px; border-radius: 50%; background: var(--blue); color: #fff; font: 700 9px/1 var(--font-mono); pointer-events: none; }

  /* Row markers, by selector on the first row of each list — the shipped
     components take no mock classes, and a wrapper would change the row's
     grid. Red on Today (the issue list), blue on Proposed (the change list). */
  .mk-marks :is(.mk-today, .mk-proposed) :is(
    .sync-name-search-input,
    .sync-project-files-search > input,
    .sync-project-files-search > .btn,
    .sync-project-file-tile:not(.cloud) > .sync-project-file-tile-head > svg,
    .sync-history-section-heading > h2,
    .sync-project-file-tile-head > strong
  ),
  .mk-marks :is(.mk-today, .mk-proposed) .sync-history-thread-list > .sync-history-thread-card:first-child :is(
    .sync-history-debug-trigger,
    .sync-history-thread-meta,
    .sync-history-thread-time-end,
    .sync-history-thread-actions .sync-history-launch-action:first-child
  ) { position: relative; outline: 2px dashed var(--mk-mark); outline-offset: 2px; border-radius: 4px; }
  .mk-marks .mk-today { --mk-mark: color-mix(in srgb, var(--red) 75%, transparent); --mk-mark-solid: var(--red); }
  .mk-marks .mk-proposed { --mk-mark: color-mix(in srgb, var(--blue) 70%, transparent); --mk-mark-solid: var(--blue); }
  /* Proposed marks only what changed: its search, tiles, and root glyph
     carry their own .mk-chg, and its session readouts did not change. */
  .mk-marks .mk-proposed :is(.sync-project-file-tile-head > strong, .sync-project-file-tile > .sync-project-file-tile-head > svg),
  .mk-marks .mk-proposed .sync-history-content .sync-history-thread-card :is(.sync-history-thread-meta, .sync-history-thread-time-end, .sync-history-launch-action) { outline: 0 !important; }
  .mk-marks :is(.mk-today, .mk-proposed) .sync-history-thread-list > .sync-history-thread-card:first-child :is(.sync-history-debug-trigger, .sync-history-thread-meta, .sync-history-thread-time-end, .sync-history-thread-actions .sync-history-launch-action:first-child)::after,
  .mk-marks .mk-today :is(.sync-name-search-input, .sync-project-files-search > .btn, .sync-history-section-heading > h2, .sync-project-file-tile-head > strong)::after { position: absolute; top: -9px; right: -9px; z-index: 5; display: grid; place-items: center; width: 16px; height: 16px; border-radius: 50%; background: var(--mk-mark-solid); color: #fff; font: 700 9px/1 var(--font-mono); pointer-events: none; }
  .mk-marks .mk-proposed .sync-history-content .sync-history-thread-card :is(.sync-history-thread-meta, .sync-history-thread-time-end, .sync-history-launch-action)::after { content: none !important; }
  .mk-marks .mk-today .sync-name-search-input::after { content: "1"; }
  .mk-marks .mk-today .sync-project-files-search > .btn::after { content: "2"; }
  .mk-marks :is(.mk-today, .mk-proposed) .sync-history-thread-card:first-child .sync-history-debug-trigger::after { content: "3"; }
  .mk-marks :is(.mk-today, .mk-proposed) .sync-history-thread-card:first-child .sync-history-thread-meta::after { content: "4"; }
  .mk-marks :is(.mk-today, .mk-proposed) .sync-history-thread-card:first-child .sync-history-thread-time-end::after { content: "5"; }
  .mk-marks :is(.mk-today, .mk-proposed) .sync-history-thread-card:first-child .sync-history-thread-actions .sync-history-launch-action:first-child::after { content: "7"; }
  .mk-marks .mk-today :is(.sync-history-section-heading > h2, .sync-project-file-tile-head > strong)::after { content: "8"; }
  /* Session readouts, stamps, and provider marks are the reference, not an
     issue: Today marks them on the Files face only. */
  .mk-marks .mk-today .sync-history-content .sync-history-thread-card :is(.sync-history-thread-meta, .sync-history-thread-time-end, .sync-history-launch-action) { outline: 0 !important; }
  .mk-marks .mk-today .sync-history-content .sync-history-thread-card :is(.sync-history-thread-meta, .sync-history-thread-time-end, .sync-history-launch-action)::after { content: none !important; }
`;

/* -------------------------------------------------------------- data --- */

const STORAGE = "Physics R2";
const PROJECT = "physics-notes";
const ROOT = "/Users/me/Documents/physics-notes";
const HOME = "/Users/me";
const NOW = 1_790_000_000;

type ProviderId = "codex" | "claude";

const project: LocalProjectSummary = {
  local_project_id: "project-physics",
  bundle_id: "bundle-physics",
  display_name: PROJECT,
  local_alias: null,
  repository_fingerprint: null,
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  profile_names: ["Default Codex", "Default Claude"],
  revision: 1,
  is_git_repository: false,
};

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica",
  local_project_id: project.local_project_id,
  bundle_id: project.bundle_id,
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  state: "active",
  revision: 1,
  updated_at: 2,
};

function sessionActions(provider: ProviderId, name: string): SessionActionDescriptor[] {
  return [
    { action_id: `${provider}.open_in_app`, kind: "open_in_app", label: `Open in ${name}`, enabled: true, disabled_reason: null },
    { action_id: `${provider}.open_in_terminal`, kind: "open_in_terminal", label: "Open in Terminal", enabled: true, disabled_reason: null },
  ];
}

interface SessionSeed {
  id: string;
  provider: ProviderId;
  title: string;
  source: SessionSourceState;
  state: ThreadSyncState;
  endedAgo: number;
  minutes: number;
  prompts: number;
  replies: number;
  tools: number;
  tokens: number;
}

const SESSION_SEEDS: SessionSeed[] = [
  { id: "019f8a10-1111-7000-8000-000000000001", provider: "claude", title: "Typeset problem set 2 solutions in LaTeX", source: "local_only", state: "local_only", endedAgo: 3_600, minutes: 42, prompts: 6, replies: 9, tools: 31, tokens: 182_400 },
  { id: "019f8a10-2222-7000-8000-000000000002", provider: "codex", title: "Derive the Lorentz transformation from the light-clock postulates", source: "local_and_remote", state: "synced", endedAgo: 86_400, minutes: 25, prompts: 4, replies: 6, tools: 12, tokens: 64_900 },
  { id: "019f8a10-3333-7000-8000-000000000003", provider: "codex", title: "Check 4-vector notation across the discussion notes", source: "local_and_remote", state: "synced", endedAgo: 3 * 86_400, minutes: 18, prompts: 3, replies: 4, tools: 7, tokens: 41_200 },
  { id: "019f8a10-4444-7000-8000-000000000004", provider: "claude", title: "Summarize the unit 2 solutions for review", source: "remote_only", state: "storage_only", endedAgo: 2 * 86_400, minutes: 11, prompts: 2, replies: 3, tools: 4, tokens: 22_700 },
];

function buildSession(seed: SessionSeed): ProjectSessionSummary {
  const name = seed.provider === "codex" ? "Codex" : "Claude";
  const local = seed.source !== "remote_only";
  const ended = NOW - seed.endedAgo;
  const input = Math.round(seed.tokens * 0.82);
  return {
    resource_id: `${seed.provider}:conversation:${seed.id}`,
    provider: seed.provider,
    provider_display_name: name,
    provider_session_id: seed.id,
    logical_thread_id: seed.id,
    source: seed.source,
    access: "bound_ready",
    title: seed.title,
    preserved: null,
    summary: null,
    started_at: ended - seed.minutes * 60,
    ended_at: ended,
    last_prompt_at: ended - 90,
    transcript_path: local ? `${HOME}/.${seed.provider}/sessions/${seed.id}.jsonl` : null,
    working_directory: local ? ROOT : null,
    is_active: false,
    user_round_count: seed.prompts,
    agent_message_count: seed.replies,
    tool_call_count: seed.tools,
    total_input_tokens: input,
    total_output_tokens: seed.tokens - input,
    total_tokens: seed.tokens,
    metrics_complete: true,
    actions: local ? sessionActions(seed.provider, name) : [],
  };
}

function buildEntry(session: ProjectSessionSummary, state: ThreadSyncState): ThreadSyncEntry {
  const pushes = state === "local_only" || state === "local_ahead";
  const pulls = state === "storage_only" || state === "storage_ahead";
  return {
    provider: session.provider,
    provider_display_name: session.provider_display_name,
    access: "bound_ready",
    thread_id: session.provider_session_id ?? session.resource_id,
    conversation_resource_id: session.resource_id,
    resource_id: session.resource_id,
    display_name: session.title,
    state,
    local_present: state !== "storage_only",
    storage_present: state !== "local_only",
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: session.provider_session_id,
    push_decision: { action: state === "local_only" ? "create_cloud" : pushes ? "publish" : "no_changes", selectable: pushes },
    pull_decision: { action: pulls ? "create_local" : "no_changes", selectable: pulls },
  };
}

const SESSIONS = SESSION_SEEDS.map(buildSession).sort((a, b) => b.ended_at - a.ended_at);
const ENTRIES = new Map(SESSION_SEEDS.map((seed) => {
  const session = buildSession(seed);
  return [session.resource_id, buildEntry(session, seed.state)];
}));

const activity: ProjectActivity = {
  project_id: project.local_project_id,
  sessions: SESSIONS,
  git: null,
  unmapped: [],
  providers: [
    { provider: "codex", display_name: "Codex", access: "bound_ready" },
    { provider: "claude", display_name: "Claude", access: "bound_ready" },
  ],
  warnings: [],
  window_start: NOW - 30 * 86_400,
  window_end: NOW,
  older: null,
  storage_sync: [],
};

const comparison: ThreadSyncComparison = {
  project_id: project.local_project_id,
  storage_id: "storage-physics",
  storage_name: STORAGE,
  compared_at: NOW,
  entries: [...ENTRIES.values()],
  counts: { synced: 2, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
  scopes: [],
  provider_errors: {},
  warnings: [],
  remote_continuity: { state: "in_sync", revision: 1 },
};

const JUL_27_2021_2055 = 1_627_415_700;

function fileEntry(path: string, state: ResourceSyncState, size: number, mtime: number, overrides: Partial<ProjectContentEntry> = {}): ProjectContentEntry {
  const local = state !== "storage_only";
  return {
    descriptor: {
      resource_id: `project_content_file:${path}`,
      kind: { key: "project_content_file", version: 1 },
      provider: null,
      display_name: path.split("/").slice(-1)[0],
      provenance: { kind: "project_local", version: 1, fields: { relative_path: path } },
      apply_policy: { key: "explicit_review", version: 1 },
      codec_version: 1,
      attributes: {},
    },
    entry_type: "file",
    relative_path: path,
    logical_path: `project/${path}`,
    size,
    mode: 0o644,
    source_mtime: mtime,
    state,
    local_present: local,
    storage_present: state !== "local_only",
    base_present: state !== "local_only",
    local_digest: local ? "a".repeat(64) : null,
    storage_digest: state === "local_only" ? null : "b".repeat(64),
    base_digest: null,
    review_digest: "a".repeat(64),
    selected_in_recipe: true,
    newly_discovered: false,
    undecided: false,
    blocked_reason: null,
    warning_code: null,
    warning_digest: null,
    ...overrides,
  };
}

const FILES: ProjectContentEntry[] = [
  fileEntry("1_Discussion_Reference_Frames.pdf", "synced", 590_848, JUL_27_2021_2055),
  fileEntry("2_Discussion_The Lorentz Transformation.pdf", "synced", 2_202_009, JUL_27_2021_2055 + 360),
  fileEntry("3_Discussion_Interval__Causality__and_Proper_Time.pdf", "local_ahead", 524_288, JUL_27_2021_2055 + 120),
  fileEntry("4_Discussion_4vectors__Tilted_Axes__and_the_Expanding_Universe.pdf", "synced", 545_792, JUL_27_2021_2055 + 240),
  fileEntry("unit_1_sol.pdf", "synced", 272_384, JUL_27_2021_2055 + 1_260),
  fileEntry("notes/lorentz-derivation.md", "local_only", 4_301, NOW - 5_400),
  fileEntry("notes/problem-set-2.tex", "diverged", 11_264, NOW - 3_700),
  fileEntry("notes/unit_2_sol.pdf", "storage_only", 308_224, NOW - 2 * 86_400),
];

const inventory: ProjectContentInventory = {
  local_project_id: project.local_project_id,
  storage_id: "storage-physics",
  project_root: ROOT,
  eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  review_token: null,
  storage_revision: null,
  preference_revision: 0,
  excluded_resource_ids: [],
  entries: FILES,
  ignored_count: 2,
  blocked_count: 0,
  warnings: [],
  scanned_at: NOW,
};

/* ------------------------------------------------------- shared chrome --- */

function storageCrumb(): PathBarCrumb {
  return {
    id: "storage",
    label: STORAGE,
    icon: "cloud",
    tone: "root",
    menuLabel: "Storage",
    menuHeader: { label: STORAGE, detail: "Cloudflare R2", icon: "cloud" },
    menu: [{ items: [{ id: "storage-settings", label: "Storage settings…", icon: "settings", onSelect: () => undefined }] }],
  };
}

function projectCrumb(): PathBarCrumb {
  return {
    id: "project",
    label: PROJECT,
    icon: "folder",
    menuLabel: "Project",
    menuHeader: { label: PROJECT, detail: "~/Documents/physics-notes", detailIsPath: true, icon: "folder" },
    menu: [{ items: [{ id: "project-settings", label: "Project setup…", icon: "settings", onSelect: () => undefined }] }],
  };
}

/** The workspace path bar as it ships: the one refresh glyph follows the
 *  face — "Refresh activity" on Sessions, "Rescan project files" on Files. */
function Bar({ view, mark }: { view: ProjectView; mark?: ReactNode }) {
  const refreshLabel = view === "files" ? "Rescan project files" : "Refresh activity";
  return (
    <SyncPathBar
      crumbs={[storageCrumb(), projectCrumb()]}
      overflow={[{ items: [{ id: "app-settings", label: "Settings…", icon: "settings", onSelect: () => undefined }] }]}
      overflowLabel="App actions"
      actions={(
        <>
          <span className={mark ? "mk-chg" : undefined} data-n={mark ?? undefined}>
            <button type="button" className="sync-path-icon-action" title={refreshLabel} aria-label={refreshLabel}>
              <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
            </button>
          </span>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className="sync-path-sync">
              <Icon name="download" size={14} />
              <span className="sync-path-sync-label">Pull</span>
              <span className="sync-path-count" aria-hidden="true">{view === "files" ? 1 : 1}</span>
            </button>
            <button type="button" className="sync-path-sync">
              <Icon name="upload" size={14} />
              <span className="sync-path-sync-label">Push</span>
              <span className="sync-path-count" aria-hidden="true">{view === "files" ? 2 : 1}</span>
            </button>
          </div>
        </>
      )}
    />
  );
}

/** One face in the workspace frame: the app root, the main column, the path
 *  bar, and the tab panel the switch controls. */
function Workspace({ view, mark, children }: { view: ProjectView; mark?: ReactNode; children: ReactNode }) {
  return (
    <div className="mk-frame">
      <div className="sync-app">
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          <Bar view={view} mark={mark} />
          <div className="sync-project-tab-panel" role="tabpanel">{children}</div>
        </main>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------ today --- */

function TodaySessions({ onView }: { onView: (view: ProjectView) => void }) {
  return (
    <ProjectChatHistoryContent
      project={project}
      binding={binding}
      activity={activity}
      loading={false}
      loadingMore={false}
      loadError={null}
      onRetryLoad={() => undefined}
      actionBusyResourceId={null}
      onLoadMore={() => undefined}
      onShowLess={() => undefined}
      canShowLess={false}
      onSessionAction={() => undefined}
      onDeleteSession={() => undefined}
      onOpenSessionDetail={() => undefined}
      comparison={comparison}
      activeStorageName={STORAGE}
      viewSwitch={<ProjectViewSwitch view="sessions" onChange={onView} />}
    />
  );
}

function TodayFiles({ onView }: { onView: (view: ProjectView) => void }) {
  return (
    <ProjectFilesActivityView
      inventory={inventory}
      storageName={STORAGE}
      loading={false}
      error={null}
      onOpenFile={() => undefined}
      onRevealPath={() => undefined}
      viewSwitch={<ProjectViewSwitch view="files" onChange={onView} />}
    />
  );
}

/* --------------------------------------------------------- proposed --- */

/** Row glyphs in the session-row family: 14px, round caps, the readout
 *  weight. `file` and `size` join message/tool/token instead of 12px UI
 *  icons. Info is SessionRowIcon's own square i, on both faces. */
const ROW_GLYPHS = {
  file: ["M14 3v4a1 1 0 0 0 1 1h4", "M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z"],
  size: ["M4 6a8 3 0 1 0 16 0a8 3 0 1 0 -16 0", "M4 6v6a8 3 0 0 0 16 0v-6", "M4 12v6a8 3 0 0 0 16 0v-6"],
} as const;

function RowGlyph({ name }: { name: keyof typeof ROW_GLYPHS }) {
  return (
    <svg
      aria-hidden="true"
      data-icon={name}
      className="session-row-icon"
      focusable="false"
      width={14}
      height={14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.5}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {ROW_GLYPHS[name].map((d) => <path key={d} d={d} />)}
    </svg>
  );
}

/** The shared search row, as both faces would render it: NameSearchField's
 *  shape — magnifier, clear, Escape — with a count that names its noun
 *  ("8 files", "2 of 8 files") instead of "items" or "entries". */
function SearchRow({ query, onChange, label, matches, total, noun, leading, n }: {
  query: string;
  onChange: (query: string) => void;
  label: string;
  matches: number;
  total: number;
  noun: [string, string];
  leading: ReactNode;
  n: number;
}) {
  const input = useRef<HTMLInputElement>(null);
  const unit = (count: number) => (count === 1 ? noun[0] : noun[1]);
  return (
    <div className="sync-name-search">
      {leading}
      <div className="sync-name-search-input mk-chg" data-n={n}>
        <Icon name="search" size={14} />
        <input
          ref={input}
          type="search"
          value={query}
          placeholder={`${label}…`}
          aria-label={label}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== "Escape" || !query) return;
            event.preventDefault();
            onChange("");
          }}
        />
        {query && (
          <button type="button" className="sync-name-search-clear" aria-label="Clear search" title="Clear search"
            onClick={() => { onChange(""); input.current?.focus(); }}>
            <Icon name="x" size={13} />
          </button>
        )}
      </div>
      <div className="sync-name-search-results" aria-live="polite" aria-atomic="true">
        <span className="sync-name-search-count">
          {query.trim()
            ? <><b>{matches}</b><span> of {total} {unit(total)}</span></>
            : <>{total} {unit(total)}</>}
        </span>
      </div>
    </div>
  );
}

/** The one framed tile both faces group rows in. */
function Tile({ icon, title, facts, cloud = false, children, n }: {
  icon: IconName;
  title: ReactNode;
  facts?: string;
  cloud?: boolean;
  children: ReactNode;
  n?: number;
}) {
  return (
    <div className={`sync-project-file-tile${cloud ? " cloud" : ""}`}>
      <div className="sync-project-file-tile-head">
        {n ? <span className="mk-chg" data-n={n} style={{ display: "inline-flex" }}><Icon name={icon} size={14} /></span> : <Icon name={icon} size={14} />}
        <strong>{title}</strong>
        {facts && <small>{facts}</small>}
      </div>
      <div className="sync-history-thread-list">{children}</div>
    </div>
  );
}

function StatusMark({ status, tone, label }: { status: ThreadStatusIconName; tone: string; label: string }) {
  return (
    <span className={`sync-thread-sync-indicator ${tone}`} data-tooltip={label} aria-label={label} tabIndex={0}>
      <ThreadStatusIcon status={status} size={18} />
    </span>
  );
}

function InfoTrigger({ label }: { label: string }) {
  return (
    <div className="sync-history-debug-menu">
      <button type="button" className="btn btn-ghost sync-history-debug-trigger" title={label} aria-label={label}>
        <SessionRowIcon name="info" />
      </button>
    </div>
  );
}

/** Both faces close the row with the same formatter, so the stamps align as
 *  one column across the switch. */
function Stamp({ seconds, tooltip }: { seconds: number; tooltip: string }) {
  return (
    <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={tooltip} tabIndex={0}>
      <time dateTime={new Date(seconds * 1_000).toISOString()}>{formatFullDateTime(seconds)}</time>
    </span>
  );
}

function ProposedSessionCard({ session }: { session: ProjectSessionSummary }) {
  const entry = ENTRIES.get(session.resource_id)!;
  const presentation = sessionSyncPresentation(entry, STORAGE);
  const title = compactThreadTitle(session.title);
  const local = session.source !== "remote_only";
  const open = session.actions.find((action) => action.kind === "open_in_app");
  return (
    <article className="sync-history-thread-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <StatusMark status={presentation.status} tone={presentation.className} label={presentation.label} />
          {local
            ? <button type="button" className="sync-history-session-title-button" title="View session details">{title}</button>
            : <strong>{title}</strong>}
        </div>
        <div className="sync-history-thread-actions">
          {open ? (
            <button type="button" className="btn btn-ghost sync-history-launch-action" title={open.label} aria-label={open.label}>
              <ProviderMark provider={session.provider} displayName={session.provider_display_name} />
            </button>
          ) : (
            <span className="sync-history-launch-action sync-history-provider-inert" title={`${session.provider_display_name} session`}>
              <ProviderMark provider={session.provider} displayName={session.provider_display_name} />
            </span>
          )}
          {local && (
            <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Terminal" aria-label="Open in Terminal">
              <Icon name="terminal" size={14} />
            </button>
          )}
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <SessionMetrics session={session} showTimestamp={false} />
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            {local && (
              <button type="button" className="btn btn-ghost sync-history-session-detail-action" aria-label="Delete local session" title="Delete local session and its subagents">
                <Icon name="trash" size={14} />
              </button>
            )}
            <InfoTrigger label="Session info" />
            {local && (
              <button type="button" className="btn btn-ghost sync-history-session-detail-action" aria-label="View session details" title="View session details">
                <SessionRowIcon name="chevron-right" />
              </button>
            )}
          </div>
          <Stamp seconds={session.ended_at} tooltip={`Ended ${formatFullDateTime(session.ended_at)}`} />
        </div>
      </div>
    </article>
  );
}

/** The real face's status sentences, against the storage by name. */
function filePresentation(entry: ProjectContentEntry): { icon: ThreadStatusIconName; tone: string; sentence: string } {
  switch (entry.state) {
    case "synced": return { icon: "synced", tone: "synced", sentence: `Up to date here and in ${STORAGE}.` };
    case "local_only": return { icon: "local-outstanding", tone: "local", sentence: `Only on this computer. Push to save it in ${STORAGE}.` };
    case "local_ahead": return { icon: "local-outstanding", tone: "local", sentence: `Newer on this computer. Push to update ${STORAGE}.` };
    case "storage_only": return { icon: "cloud-outstanding", tone: "storage", sentence: `Only in ${STORAGE}. Pull to download it here.` };
    case "storage_ahead": return { icon: "cloud-outstanding", tone: "storage", sentence: `Newer in ${STORAGE}. Pull to update this computer.` };
    case "diverged": return { icon: "divergent", tone: "diverged", sentence: `Changed here and in ${STORAGE}. Review before syncing.` };
    default: return { icon: "unknown", tone: "unknown", sentence: "State unknown. Rescan to compare." };
  }
}

function ProposedFileCard({ entry, withPathPrefix = false }: { entry: ProjectContentEntry; withPathPrefix?: boolean }) {
  const presentation = filePresentation(entry);
  const directory = entry.relative_path.includes("/") ? entry.relative_path.slice(0, entry.relative_path.lastIndexOf("/")) : "";
  const name = entry.relative_path.split("/").slice(-1)[0];
  const kind = fileKindLabel(entry.relative_path);
  const warningLabel = entry.warning_code ? projectFileWarningLabel(entry.warning_code) : null;
  const where = entry.local_present ? "Modified" : `Updated in ${STORAGE}`;
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <StatusMark status={presentation.icon} tone={presentation.tone} label={presentation.sentence} />
          {entry.local_present ? (
            <button type="button" className="sync-history-session-title-button" title={`Open ${entry.relative_path}`}>
              {withPathPrefix && directory !== "" && <span>{directory}/</span>}
              {name}
            </button>
          ) : (
            <strong title={entry.relative_path}>
              {withPathPrefix && directory !== "" && <span>{directory}/</span>}
              {name}
            </strong>
          )}
          {warningLabel && <NoticeChip severity="warning">{warningLabel}</NoticeChip>}
        </div>
        <div className="sync-history-thread-actions">
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!entry.local_present}
            title={entry.local_present ? "Show in Finder" : "Not on this Mac"}
            aria-label={entry.local_present ? `Show ${entry.relative_path} in Finder` : `Cannot show ${entry.relative_path}: not on this Mac`}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${entry.relative_path} details`}>
            <span className="sync-history-thread-metric" data-tooltip={`Kind · ${kind}`} tabIndex={0}>
              <RowGlyph name="file" /><b>{kind}</b>
            </span>
            {entry.size != null && (
              <span className="sync-history-thread-metric" data-tooltip={`${entry.local_present ? "On disk" : `In ${STORAGE}`} · ${formatBytes(entry.size)}`} tabIndex={0}>
                <RowGlyph name="size" /><b>{formatBytes(entry.size)}</b>
              </span>
            )}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <InfoTrigger label="File info" />
          </div>
          {entry.source_mtime != null && (
            <Stamp seconds={entry.source_mtime} tooltip={`${where} · ${formatFullDateTime(entry.source_mtime)}`} />
          )}
        </div>
      </div>
    </article>
  );
}

type CloudRows = "tile" | "inline";

function ProposedSessions({ onView, cloudRows }: { onView: (view: ProjectView) => void; cloudRows: CloudRows }) {
  const [query, setQuery] = useState("");
  const matching = SESSIONS.filter((session) => nameMatchesSearch(session.title, query));
  const cloud = cloudRows === "tile" ? matching.filter((session) => session.source === "remote_only") : [];
  const rest = cloudRows === "tile" ? matching.filter((session) => session.source !== "remote_only") : matching;
  return (
    <div className="sync-history-page sync-history-embedded">
      <section className="sync-history-content" aria-label="Sessions">
        <SearchRow
          query={query}
          onChange={setQuery}
          label="Search loaded sessions"
          matches={matching.length}
          total={SESSIONS.length}
          noun={["session", "sessions"]}
          leading={<ProjectViewSwitch view="sessions" onChange={onView} />}
          n={1}
        />
        {matching.length === 0 ? (
          <div className="sync-history-state">No matching loaded sessions.</div>
        ) : (
          <div style={{ display: "grid", gap: 12 }}>
            {cloud.length > 0 && (
              <Tile icon="cloud" cloud title={<>Only in <span className="sync-history-storage-name">{STORAGE}</span></>} n={8}>
                {cloud.map((session) => <ProposedSessionCard key={session.resource_id} session={session} />)}
              </Tile>
            )}
            {rest.length > 0 && (
              <Tile icon="message" title="Sessions" facts={`${rest.length} session${rest.length === 1 ? "" : "s"}`} n={8}>
                {rest.map((session) => <ProposedSessionCard key={session.resource_id} session={session} />)}
              </Tile>
            )}
          </div>
        )}
      </section>
    </div>
  );
}

function groupFacts(entries: ProjectContentEntry[]): string {
  const bytes = entries.reduce((sum, entry) => sum + (entry.size ?? 0), 0);
  return `${entries.length} file${entries.length === 1 ? "" : "s"} · ${formatBytes(bytes)}`;
}

function ProposedFiles({ onView, cloudRows }: { onView: (view: ProjectView) => void; cloudRows: CloudRows }) {
  const [query, setQuery] = useState("");
  const matching = FILES.filter((entry) => nameMatchesSearch(entry.relative_path, query));
  const cloud = cloudRows === "tile" ? matching.filter((entry) => !entry.local_present) : [];
  const placed = cloudRows === "tile" ? matching.filter((entry) => entry.local_present) : matching;
  const groups = new Map<string, ProjectContentEntry[]>();
  for (const entry of placed) {
    const directory = entry.relative_path.includes("/") ? entry.relative_path.slice(0, entry.relative_path.lastIndexOf("/")) : "";
    groups.set(directory, [...(groups.get(directory) ?? []), entry]);
  }
  const ordered = [...groups.entries()].sort(([a], [b]) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)));
  return (
    <section className="sync-project-files-page" aria-label="Project files">
      <SearchRow
        query={query}
        onChange={setQuery}
        label="Search files"
        matches={matching.length}
        total={FILES.length}
        noun={["file", "files"]}
        leading={<ProjectViewSwitch view="files" onChange={onView} />}
        n={1}
      />
      {matching.length === 0 ? (
        <div className="sync-history-state mk-chg" data-n={9}>No matching files.</div>
      ) : (
        <>
          {cloud.length > 0 && (
            <Tile icon="cloud" cloud title={<>Only in <span className="sync-history-storage-name">{STORAGE}</span></>} n={8}>
              {cloud.map((entry) => <ProposedFileCard key={entry.relative_path} entry={entry} withPathPrefix />)}
            </Tile>
          )}
          {ordered.map(([directory, entries]) => (
            <Tile
              key={directory || "(root)"}
              icon="folder"
              title={directory === "" ? "Project root" : <span>{directory}/</span>}
              facts={groupFacts(entries)}
              n={directory === "" ? 6 : undefined}
            >
              {entries.map((entry) => <ProposedFileCard key={entry.relative_path} entry={entry} />)}
            </Tile>
          ))}
        </>
      )}
      <details className="sync-project-files-scan-summary">
        <summary>{inventory.ignored_count} ignored · {inventory.blocked_count} blocked · {inventory.warnings.length} scan warnings</summary>
      </details>
    </section>
  );
}

/* ------------------------------------------------------------- page --- */

type Theme = "light" | "dark";
type Layout = "stacked" | "side";

function readKnobs() {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    theme: pick("theme", ["light", "dark"] as const, "light"),
    layout: pick("layout", ["stacked", "side"] as const, "side"),
    cloud: pick("cloud", ["tile", "inline"] as const, "tile"),
    marks: pick("marks", ["on", "off"] as const, "on"),
  };
}

const ISSUES: ReactNode[] = [
  <><b>Two search fields.</b> Sessions uses <code>NameSearchField</code>, an underlined row with a magnifier, a clear button, and a live “m / n” count. Files uses a bare pill input with no magnifier and no clear button, and counts “entries” whether or not a filter is active.</>,
  <><b>Two refresh controls on Files.</b> The path-bar refresh already rescans files on this face, and the search row adds a labelled “Rescan” button. Sessions has only the path-bar refresh.</>,
  <><b>Two info glyphs.</b> Sessions uses the square ⓘ (<code>SessionRowIcon</code>, 1.75 stroke). Files uses a circled ⓘ (<code>Icon</code>, 2 stroke) instead.</>,
  <><b>Two readout glyph families.</b> Messages, tools, and tokens use 14px row glyphs at the readout weight. Kind and Size use 12px UI icons at a thinner weight.</>,
  <><b>Two stamp formatters.</b> Sessions uses <code>formatFullDateTime</code>, Files uses <code>formatDateTime</code>. The two can render differently by locale.</>,
  <><b>A Git glyph on a non-Git project.</b> “Project root” uses <code>git-folder</code> (folder with &lt;/&gt;), which the rest of the app shows only for Git repositories.</>,
  <><b>“Reveal … in Finder”.</b> Apple’s own apps say “Show in Finder”.</>,
  <><b>Two ways of grouping.</b> Sessions groups under plain headings (“Only in …”, “Sessions”). Files groups in framed tiles.</>,
];

const CHANGES: ReactNode[] = [
  <><b>One search row:</b> both faces use the same field, with a magnifier, a clear button, Escape to clear, and a live count that says what it counts (“8 files”, “2 of 8 files”, “4 sessions”). <a href="https://developer.apple.com/design/human-interface-guidelines/search-fields">HIG search fields</a> · <a href="https://primer.style/product/scenario-patterns/filter/">Primer filter</a> · <a href="https://www.nngroup.com/articles/consistency-and-standards/">NN/g consistency</a></>,
  <><b>One refresh:</b> the path-bar glyph, which is already scoped to the open face. The in-page “Rescan” button goes.</>,
  <><b>One info glyph:</b> the square ⓘ Sessions already uses (<code>SessionRowIcon</code>), now on Files too, at the same size and weight. It opens a popover and never navigates. <a href="https://developer.apple.com/design/human-interface-guidelines/lists-and-tables">HIG lists</a></>,
  <><b>One glyph family:</b> Kind and Size become row glyphs next to messages, tools, and tokens, at one size and one stroke. <a href="https://developer.apple.com/design/human-interface-guidelines/icons">HIG icons</a></>,
  <><b>One stamp:</b> <code>formatFullDateTime</code> on both faces, so the right-edge column reads the same.</>,
  <><b>Plain folder for the non-Git root:</b> the same rule the picker, sidebar, and storage index already follow.</>,
  <><b>“Show in Finder”</b> as the tooltip and accessible name.</>,
  <><b>One grouping:</b> framed tiles on both faces. Use the knob to compare storage-only rows in their own cloud tile (today’s rule) with rows left in place under the cloud status glyph (how Dropbox, iCloud Drive, OneDrive, and Google Drive show them). <a href="https://support.apple.com/guide/mac-help/check-icloud-drive-file-folder-status-mac-mchlc994344b/mac">iCloud status</a> · <a href="https://help.dropbox.com/sync/macos-sync-icons">Dropbox icons</a></>,
  <><b>No-match state:</b> Files says “No matching files.” in the same row Sessions uses.</>,
];

function Page() {
  const initial = readKnobs();
  const [theme, setTheme] = useState<Theme>(initial.theme);
  const [layout, setLayout] = useState<Layout>(initial.layout);
  const [cloudRows, setCloudRows] = useState<CloudRows>(initial.cloud);
  const [marks, setMarks] = useState(initial.marks === "on");
  document.documentElement.dataset.theme = theme;
  const knob = <T extends string>(value: T, current: T, set: (value: T) => void, label: string) => (
    <button type="button" className={value === current ? "on" : undefined} onClick={() => set(value)}>{label}</button>
  );
  const noop = () => undefined;
  return (
    <div className={`mk${marks ? " mk-marks" : ""}`}>
      <style>{SHELL}</style>
      <h1 className="mk-title">Non-Git project: Sessions and Files together</h1>
      <p className="mk-lede">
        Each frame shows one face of the same project (<b>{PROJECT}</b>, linked to <b>{STORAGE}</b>).
        “Today” renders the shipped components on static data. “Proposed” uses the shipped CSS and
        row components, with one search row, one refresh, one info glyph, one glyph family, one
        stamp, and one kind of tile on both faces. Numbered outlines match the lists. The search
        fields work, and rows show their tooltips on hover.
      </p>
      <div className="mk-bar" role="group" aria-label="Mock controls">
        {knob("side", layout, setLayout, "Side by side")}
        {knob("stacked", layout, setLayout, "Stacked")}
        <span>·</span>
        <span>Storage-only rows</span>
        {knob("tile", cloudRows, setCloudRows, "Own tile")}
        {knob("inline", cloudRows, setCloudRows, "In place")}
        <span>·</span>
        <button type="button" className={marks ? "on" : undefined} onClick={() => setMarks((on) => !on)}>Marks</button>
        <span>·</span>
        {knob("light", theme, setTheme, "Light")}
        {knob("dark", theme, setTheme, "Dark")}
      </div>

      <section className="mk-variant mk-today">
        <h2>Today</h2>
        <p>The shipped faces. Red numbers mark where the two faces disagree.</p>
        <ol className="mk-changes mk-issues">
          {ISSUES.map((issue, index) => <li key={index}><span className="mk-num">{index + 1}</span><span>{issue}</span></li>)}
        </ol>
        <div className={`mk-faces${layout === "side" ? " side" : ""}`}>
          <div className="mk-face">
            <h3>Sessions</h3>
            <Workspace view="sessions"><TodaySessions onView={noop} /></Workspace>
          </div>
          <div className="mk-face">
            <h3>Files</h3>
            <Workspace view="files"><TodayFiles onView={noop} /></Workspace>
          </div>
        </div>
      </section>

      <section className="mk-variant mk-proposed">
        <h2>Proposed<span className="mk-tag">recommended</span></h2>
        <p>Both faces share one grammar. Blue numbers mark each change.</p>
        <ol className="mk-changes">
          {CHANGES.map((change, index) => <li key={index}><span className="mk-num">{index + 1}</span><span>{change}</span></li>)}
        </ol>
        <div className={`mk-faces${layout === "side" ? " side" : ""}`}>
          <div className="mk-face">
            <h3>Sessions</h3>
            <Workspace view="sessions" mark={2}><ProposedSessions onView={noop} cloudRows={cloudRows} /></Workspace>
          </div>
          <div className="mk-face">
            <h3>Files</h3>
            <Workspace view="files" mark={2}><ProposedFiles onView={noop} cloudRows={cloudRows} /></Workspace>
          </div>
        </div>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
