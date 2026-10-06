import { StrictMode, useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import type { StatusScope } from "../src/components/sync-review/statusScopes";
import ProjectViewSwitch, { type ProjectView } from "../src/components/project-sync/ProjectViewSwitch";
import { ProjectChatHistoryContent } from "../src/components/project-sync/ProjectActivityPage";
import { ProjectFilesActivityView, projectFileSyncCounts } from "../src/components/project-sync/ProjectFilesActivityPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "../src/components/project-sync/ProjectFilesReviewPage";
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
 * Mockup: the status scope bar on the Files face of a non-Git project.
 *
 * The Sessions face got the scope bar on 2026-10-03 (C); the Files face was
 * left out of that change on purpose and had only its name search.
 *
 * Picked 2026-10-06 and shipped: F1 — the same bar on Files, folder tiles
 * kept — and a file deleted here (`missing`) is a file to pull, on the strip
 * and on the path bar's Pull count alike. Every frame is the shipped page on
 * static data.
 *
 *   Files    the Files face in every state
 *   Spacing  the list-head gap fixed in App.css, shown on the review step
 *
 * `npm run dev`, then /mockups/files-filter-mockup.html
 * (`?theme=light|dark&only=files|spacing|refs`; `only` draws one section,
 * for screenshots).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Theme = "light" | "dark";

function readKnobs(): { theme: Theme; only: string | null } {
  const query = new URLSearchParams(window.location.search);
  return {
    theme: query.get("theme") === "dark" ? "dark" : "light",
    only: query.get("only"),
  };
}

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { display: block; background: var(--retro-frame-gutter); }
  .mk { max-width: 1480px; margin: 0 auto; padding: 24px 22px 90px; font-family: var(--font-sans); color: var(--text-0); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 14px; max-width: 110ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-lede code, .mk-points code, .mk-table code, .mk-variant > p code { font-family: var(--font-mono); font-size: 0.92em; }
  .mk-bar { position: sticky; top: 0; z-index: 30; display: flex; gap: 8px; align-items: center; margin: 0 0 22px; padding: 8px 0; flex-wrap: wrap; background: var(--retro-frame-gutter); }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 5px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--bg-active); color: var(--text-0); border-color: var(--border-mid); }
  .mk-bar > span { color: var(--text-3); font-size: var(--type-xs); }
  .mk-bar a { color: var(--text-2); font-size: var(--type-xs); }
  .mk-variant { margin: 0 0 44px; scroll-margin-top: 56px; }
  .mk-variant > h2 { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-lg); }
  .mk-variant > p { margin: 0 0 10px; max-width: 110ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-tag { display: inline-block; margin-left: 8px; padding: 1px 7px; border-radius: var(--r-pill); background: var(--surface-row-active); color: var(--text-0); font-size: var(--type-2xs); font-weight: var(--weight-semibold); vertical-align: 3px; }
  .mk-points { margin: 0 0 14px; padding: 0 0 0 18px; display: grid; gap: 5px; max-width: 120ch; color: var(--text-2); font-size: var(--type-xs); line-height: var(--leading-prose); }
  .mk-points b { color: var(--text-0); }
  .mk-points a, .mk-table a { color: var(--text-2); }

  .mk-cells { display: grid; grid-template-columns: repeat(auto-fill, minmax(640px, 1fr)); gap: 20px 18px; align-items: start; }
  .mk-cell > h3 { margin: 0 0 6px; color: var(--text-1); font-size: var(--type-sm); font-weight: var(--weight-semibold); }
  .mk-cell > h3 small { margin-left: 6px; color: var(--text-3); font-weight: var(--weight-regular); }
  .mk-cell.narrow { width: 580px; }
  .mk-clip { max-height: var(--clip, 460px); overflow: hidden; -webkit-mask-image: linear-gradient(#000 calc(100% - 40px), transparent); mask-image: linear-gradient(#000 calc(100% - 40px), transparent); }
  .mk-note { margin: 6px 2px 0; color: var(--text-2); font-size: var(--type-2xs); line-height: var(--leading-prose); }
  .mk-note b { color: var(--text-0); }

  .mk-frame { padding: 3px; border: 1px dashed var(--border-mid); border-radius: 10px; background: var(--retro-frame-gutter); }
  .mk-frame > .sync-app { display: block; height: auto; overflow: visible; padding: 0; background: transparent; }
  .mk-frame .sync-main { height: auto; min-height: 0; overflow: visible; padding: 0 18px 8px; border-radius: 8px; background: var(--bg-0); }
  .mk-frame .sync-project-tab-panel { padding-top: 14px; }
  /* The review frame: the files step's page inside the review column. */
  .mk-review { border-radius: 8px; background: var(--bg-0); }

  .mk-table { width: 100%; max-width: 1240px; border-collapse: collapse; font-size: var(--type-xs); color: var(--text-1); }
  .mk-table th, .mk-table td { padding: 6px 12px 6px 0; border-top: 1px solid var(--border); text-align: left; vertical-align: top; line-height: var(--leading-prose); }
  .mk-table thead th { border-top: 0; color: var(--text-2); font-weight: var(--weight-semibold); }
  .mk-table td:first-child { white-space: nowrap; font-weight: var(--weight-semibold); color: var(--text-0); }
  .mk-measure { font-family: var(--font-mono); font-size: var(--type-2xs); color: var(--text-2); }

`;

const link = (href: string, label: string) => <a href={href} target="_blank" rel="noreferrer">{label}</a>;

/* ——— data ————————————————————————————————————————————————————————— */

const STORAGE = "Physics R2";
const PROJECT = "physics-notes";
const ROOT = "/Users/me/Documents/physics-notes";
const HOME = "/Users/me";
const NOW = 1_790_000_000;
const DAY = 86_400;

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
  { id: "019f8a10-2222-7000-8000-000000000002", provider: "codex", title: "Derive the Lorentz transformation from the light-clock postulates", source: "local_and_remote", state: "synced", endedAgo: DAY, minutes: 25, prompts: 4, replies: 6, tools: 12, tokens: 64_900 },
  { id: "019f8a10-3333-7000-8000-000000000003", provider: "codex", title: "Check 4-vector notation across the discussion notes", source: "local_and_remote", state: "synced", endedAgo: 3 * DAY, minutes: 18, prompts: 3, replies: 4, tools: 7, tokens: 41_200 },
  { id: "019f8a10-4444-7000-8000-000000000004", provider: "claude", title: "Summarize the unit 2 solutions for review", source: "remote_only", state: "storage_only", endedAgo: 2 * DAY, minutes: 11, prompts: 2, replies: 3, tools: 4, tokens: 22_700 },
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
const SESSION_ENTRIES = SESSION_SEEDS.map((seed) => buildEntry(buildSession(seed), seed.state));

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
  window_start: NOW - 30 * DAY,
  window_end: NOW,
  older: null,
  storage_sync: [],
};

const comparison: ThreadSyncComparison = {
  project_id: project.local_project_id,
  storage_id: "storage-physics",
  storage_name: STORAGE,
  compared_at: NOW,
  entries: SESSION_ENTRIES,
  counts: { synced: 2, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
  scopes: [],
  provider_errors: {},
  warnings: [],
  remote_continuity: { state: "in_sync", revision: 1 },
};

/** The bar's thread tallies for this comparison: one to push, one to pull,
 *  nothing for the attention chip. */
const SESSION_COUNTS = { push: 1, pull: 1 };

function fileEntry(
  path: string,
  state: ResourceSyncState,
  size: number,
  mtime: number,
  overrides: Partial<ProjectContentEntry> = {},
): ProjectContentEntry {
  const local = state !== "storage_only" && state !== "missing";
  const stored = state !== "local_only";
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
    storage_present: stored,
    base_present: state !== "local_only" && state !== "storage_only",
    local_digest: local ? "a".repeat(64) : null,
    storage_digest: stored ? "b".repeat(64) : null,
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

const JUL_27_2021 = 1_627_415_700;

/** Fourteen files: four to push, three to pull (one of them deleted here),
 *  two for attention (a file changed on both sides, a symlink), five up to
 *  date. */
const FILES: ProjectContentEntry[] = [
  fileEntry("1_Discussion_Reference_Frames.pdf", "synced", 590_848, JUL_27_2021),
  fileEntry("2_Discussion_The_Lorentz_Transformation.pdf", "synced", 2_202_009, JUL_27_2021 + 360),
  fileEntry("3_Discussion_Interval_Causality_and_Proper_Time.pdf", "local_ahead", 524_288, NOW - 2 * 3_600),
  fileEntry("4_Discussion_4vectors_and_the_Expanding_Universe.pdf", "synced", 545_792, JUL_27_2021 + 240),
  fileEntry("syllabus.md", "synced", 3_210, JUL_27_2021 - 9 * DAY),
  fileEntry("unit_1_sol.pdf", "missing", 272_384, JUL_27_2021 + 1_260),
  fileEntry("figures/light-clock.svg", "local_only", 18_432, NOW - 5_400),
  fileEntry("figures/render.sh", "local_only", 1_148, NOW - 5_100, { warning_code: "executable" }),
  fileEntry("figures/spacetime-diagram.png", "storage_ahead", 412_160, NOW - DAY),
  fileEntry("figures/dataset", "blocked", 0, NOW - 4 * DAY, {
    entry_type: "blocked",
    size: null,
    storage_present: false,
    base_present: false,
    storage_digest: null,
    blocked_reason: "A symbolic link. Mallard copies regular files only.",
  }),
  fileEntry("notes/glossary.md", "synced", 2_048, NOW - 8 * DAY),
  fileEntry("notes/lorentz-derivation.md", "local_only", 4_301, NOW - 3_700),
  fileEntry("notes/problem-set-2.tex", "diverged", 11_264, NOW - 3_300),
  fileEntry("notes/unit_2_sol.pdf", "storage_only", 308_224, NOW - 2 * DAY),
];

/** The scope a file lands in after a Push: its four pushes are up to date. */
const FILES_AFTER_PUSH: ProjectContentEntry[] = FILES.map((entry) => (
  entry.state === "local_only" || entry.state === "local_ahead"
    ? { ...entry, state: "synced", storage_present: true, base_present: true, storage_digest: entry.local_digest }
    : entry
));

const inventory = (entries: ProjectContentEntry[]): ProjectContentInventory => ({
  local_project_id: project.local_project_id,
  storage_id: "storage-physics",
  project_root: ROOT,
  eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  review_token: null,
  storage_revision: null,
  preference_revision: 0,
  excluded_resource_ids: [],
  entries,
  ignored_count: 2,
  blocked_count: 1,
  warnings: [],
  scanned_at: NOW,
});

/* ——— chrome ———————————————————————————————————————————————————————— */

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

/** The workspace path bar: its counts add the session tallies to the file
 *  tallies, as `ProjectLinksWorkspace` does. No attention chip — the chip
 *  counts sessions only, and none of these sessions needs attention. */
function Bar({ view, pull, push }: { view: ProjectView; pull: number; push: number }) {
  const refreshLabel = view === "files" ? "Rescan project files" : "Refresh activity";
  return (
    <SyncPathBar
      crumbs={[storageCrumb(), projectCrumb()]}
      overflow={[{ items: [{ id: "app-settings", label: "Settings…", icon: "settings", onSelect: () => undefined }] }]}
      overflowLabel="App actions"
      actions={(
        <>
          <button type="button" className="sync-path-icon-action" title={refreshLabel} aria-label={refreshLabel}>
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
      )}
    />
  );
}

/** The bar counts files through the shipped `projectFileSyncCounts`. */
const fileCounts = (entries: ProjectContentEntry[]) => projectFileSyncCounts(inventory(entries));

function Workspace({ view, entries, children }: { view: ProjectView; entries: ProjectContentEntry[]; children: ReactNode }) {
  const files = fileCounts(entries);
  return (
    <div className="mk-frame">
      <div className="sync-app">
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          <Bar view={view} pull={SESSION_COUNTS.pull + files.pull} push={SESSION_COUNTS.push + files.push} />
          <div className="sync-project-tab-panel" role="tabpanel">{children}</div>
        </main>
      </div>
    </div>
  );
}

/** The shipped Sessions face, its scope held here so the frame stays live. */
function SessionsFace({ onView }: { onView: (view: ProjectView) => void }) {
  const [scope, setScope] = useState<StatusScope | null>(null);
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
      statusScope={scope}
      onStatusScopeChange={setScope}
      viewSwitch={<ProjectViewSwitch view="sessions" onChange={onView} />}
    />
  );
}

/** The shipped Files face, opened on a scope and still live. */
function FilesFace({ entries = FILES, initialScope = null, viewSwitch }: {
  entries?: ProjectContentEntry[];
  initialScope?: StatusScope | null;
  viewSwitch: ReactNode;
}) {
  const [scope, setScope] = useState<StatusScope | null>(initialScope);
  return (
    <ProjectFilesActivityView
      inventory={inventory(entries)}
      storageName={STORAGE}
      loading={false}
      error={null}
      onOpenFile={() => undefined}
      onRevealPath={() => undefined}
      viewSwitch={viewSwitch}
      statusScope={scope}
      onStatusScopeChange={setScope}
    />
  );
}

/** One frame with a working Sessions/Files switch, wired as the workspace
 *  does: Files unmounts on a switch (its search and scope reset); Sessions
 *  stays mounted under `hidden`. */
function LiveFaces() {
  const [view, setView] = useState<ProjectView>("files");
  return (
    <Workspace view={view} entries={FILES}>
      {view === "files" && (
        <ProjectFilesActivityView
          inventory={inventory(FILES)}
          storageName={STORAGE}
          loading={false}
          error={null}
          viewSwitch={<ProjectViewSwitch view="files" onChange={setView} />}
        />
      )}
      <div hidden={view === "files"}>
        <SessionsFace onView={setView} />
      </div>
    </Workspace>
  );
}

/** The fixtures as the Push review's files step sees them. */
const REVIEW_ROWS: ProjectFileReviewRow[] = FILES.map((entry) => ({
  resourceId: entry.descriptor.resource_id,
  relativePath: entry.relative_path,
  entryType: entry.entry_type === "blocked" ? "blocked" : entry.entry_type === "directory" ? "directory" : "file",
  state: entry.state,
  size: entry.size,
  mode: entry.mode,
  sourceMtime: entry.source_mtime,
  localPresent: entry.local_present,
  storagePresent: entry.storage_present,
  blockedReason: entry.blocked_reason,
  warningCode: entry.warning_code,
  warningDigest: entry.warning_digest,
}));

/* ——— page ————————————————————————————————————————————————————————— */

function Cell({ title, note, clip, narrow, children }: {
  title: ReactNode;
  note?: ReactNode;
  clip?: number;
  narrow?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={`mk-cell${narrow ? " narrow" : ""}`}>
      <h3>{title}</h3>
      <div className={clip ? "mk-clip" : undefined} style={clip ? ({ "--clip": `${clip}px` } as CSSProperties) : undefined}>
        {children}
      </div>
      {note && <p className="mk-note">{note}</p>}
    </div>
  );
}

function Page() {
  const initial = readKnobs();
  const [theme, setTheme] = useState<Theme>(initial.theme);
  const [measured, setMeasured] = useState<Record<string, string>>({});
  document.documentElement.dataset.theme = theme;
  /* Gaps are measured, not asserted: after the pixel font lands, from the
     rule a head draws to the next thing the page shows. */
  useEffect(() => {
    let cancelled = false;
    void document.fonts.ready.then(() => {
      if (cancelled) return;
      const px = (value: number) => `${Math.round(value)}px`;
      const nextShown = (element: Element | null | undefined) => {
        let next = element?.nextElementSibling ?? null;
        while (next && next.getBoundingClientRect().height === 0) next = next.nextElementSibling;
        return next;
      };
      const frame = (name: string) => document.querySelector(`[data-frame="${name}"]`);
      const gap = (above: Element | null | undefined, below: Element | null | undefined) => (
        above && below ? px(below.getBoundingClientRect().top - above.getBoundingClientRect().bottom) : "?"
      );
      const head = (name: string) => frame(name)?.querySelector(".sync-project-files-page > .sync-list-head");
      const strip = frame("files-all")?.querySelector(".sync-project-files-page > .sync-status-scopes");
      setMeasured({
        "files-strip": gap(head("files-all"), strip),
        "files-list": gap(strip, nextShown(strip)),
        "review": gap(head("review"), nextShown(head("review"))),
      });
    });
    return () => { cancelled = true; };
  }, [theme]);
  const shown = (id: string) => initial.only === null || initial.only === id;
  const measure = (name: string) => <span className="mk-measure">{measured[name] ?? "…"}</span>;
  const knob = <T extends string>(value: T, current: T, set: (value: T) => void, label: string) => (
    <button type="button" className={value === current ? "on" : undefined} onClick={() => set(value)}>{label}</button>
  );
  const fileSwitch = <ProjectViewSwitch view="files" onChange={() => undefined} />;
  const counts = fileCounts(FILES);

  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Files face: filter by status</h1>
      <p className="mk-lede">
        One non-Git project (<b>{PROJECT}</b>, linked to <b>{STORAGE}</b>): fourteen files and four
        sessions. Picked 2026-10-06 and now shipped: <b>F1</b>, the Sessions face’s scope bar on Files with the
        folder tiles kept, and <b>a file deleted here is a file to pull</b> — on the strip and on the bar’s Pull
        count. Every frame is the shipped page and every frame is live: the switch, the scopes, and the search work.
      </p>
      <div className="mk-bar" role="group" aria-label="Mock controls">
        <a href="#files">Files face</a><a href="#spacing">Spacing fix</a><a href="#refs">Decisions and references</a>
        <span>·</span>
        {knob("light", theme, setTheme, "Light")}
        {knob("dark", theme, setTheme, "Dark")}
      </div>

      {shown("files") && (
        <section className="mk-variant" id="files">
          <h2>Files face · F1, deleted files to pull<span className="mk-tag">shipped</span></h2>
          <ul className="mk-points">
            <li><b>The Sessions face’s strip, word for word:</b> All, To push, To pull, Attention, Up to date (Not compared only when a file has no verdict). One scope at a time; All is the way back.</li>
            <li><b>Folders stay,</b> as under the name search: a tile with no file in the scope leaves, the cloud tile still leads, and each tile’s facts count the files it shows.</li>
            <li><b>A file deleted here is a file to pull.</b> <code>unit_1_sol.pdf</code> keeps its Pull arrow and its place in the cloud tile, joins To pull, and the bar counts it: Pull {SESSION_COUNTS.pull + counts.pull} ({SESSION_COUNTS.pull} session + {counts.pull} files). Attention keeps the file changed on both sides and the symlink.</li>
            <li><b>The counts cover every file</b> — the scan is the whole inventory — so the scope applies before the 500-card budget, and To push / To pull equal the bar’s file tallies exactly.</li>
            <li><b>The scope belongs to the face.</b> Files unmounts on a switch, so its scope and search reset when you come back; Sessions keeps its own.</li>
          </ul>
          <div className="mk-cells">
            <Cell title={<>All <small>live — switch faces, choose scopes, search</small></>} clip={720} note={<>Space under the search rule: {measure("files-strip")} to the strip’s top, {measure("files-list")} from the strip to the first tile.</>}>
              <div data-frame="files-all"><LiveFaces /></div>
            </Cell>
            <Cell title={<>To pull <small>the deleted file joins it</small></>} clip={560}>
              <Workspace view="files" entries={FILES}><FilesFace initialScope="pull" viewSwitch={fileSwitch} /></Workspace>
            </Cell>
            <Cell title="To push" clip={560}>
              <Workspace view="files" entries={FILES}><FilesFace initialScope="push" viewSwitch={fileSwitch} /></Workspace>
            </Cell>
            <Cell title="Attention" clip={460}>
              <Workspace view="files" entries={FILES}><FilesFace initialScope="attention" viewSwitch={fileSwitch} /></Workspace>
            </Cell>
            <Cell title={<>To push, after a Push <small>the scope stays at 0</small></>} note={<>The scope you chose stays, at 0, until you release it; <i>Clear filter</i> appears because releasing it brings files back.</>} clip={300}>
              <Workspace view="files" entries={FILES_AFTER_PUSH}><FilesFace entries={FILES_AFTER_PUSH} initialScope="push" viewSwitch={fileSwitch} /></Workspace>
            </Cell>
            <Cell title={<>580px <small>the narrowest workspace</small></>} narrow clip={300}>
              <Workspace view="files" entries={FILES}><FilesFace viewSwitch={fileSwitch} /></Workspace>
            </Cell>
          </div>
        </section>
      )}

      {shown("spacing") && (
        <section className="mk-variant" id="spacing">
          <h2>Spacing fix<span className="mk-tag">shipped</span></h2>
          <p>
            The Files page spaces its children with its own 12px gap and already strips the search field’s
            margin (<code>.sync-project-files-page &gt; .sync-name-search</code>). When the field rides a list
            head — the view switch on the face, the Rescan button in the review — the head kept its own 12px
            margin on top of the gap, so the first tile sat 24px under the rule. The same rule now covers the
            list head. The review below is the shipped files step.
          </p>
          <div className="mk-cells">
            <Cell title={<>Push review, files step <small>first tile {measure("review")} under the rule</small></>} clip={260}>
              <div data-frame="review" className="mk-frame">
                <div className="sync-app">
                  <div className="mk-review">
                    <ProjectFilesReviewPage
                      mode="push"
                      projectRoot={ROOT}
                      eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
                      rows={REVIEW_ROWS}
                      selectedIds={new Set()}
                      onScan={() => undefined}
                      onToggle={() => undefined}
                    />
                  </div>
                </div>
              </div>
            </Cell>
          </div>
        </section>
      )}

      {shown("refs") && (
        <section className="mk-variant" id="refs">
          <h2>Decisions and references</h2>
          <ul className="mk-points">
            <li><b>Picked 2026-10-06:</b> F1 (folders kept) and a deleted-here file counted under To pull and on the bar’s Pull.</li>
            <li><b>Declined:</b> F2 (one merged list while narrowed), F3 (one scope across both faces), and keeping deleted files under Attention or giving them a glyph of their own.</li>
          </ul>
          <p>
            The control itself was chosen for this page on 2026-10-03 from the references below; that research is
            in <code>docs/PLAN_SESSION_STATUS_FILTER.md</code>.
          </p>
          <table className="mk-table">
            <thead><tr><th>Reference</th><th>What it asks</th><th>How the Files face meets it</th></tr></thead>
            <tbody>
              <tr><td>{link("https://developer.apple.com/design/human-interface-guidelines/search-fields", "Apple HIG, search fields")}</td><td>A scope bar narrows among clearly defined categories, opening on the broad one.</td><td>Five fixed parts; All first and in force on arrival.</td></tr>
              <tr><td>{link("https://primer.style/product/scenario-patterns/filter/", "Primer, filter pattern")}</td><td>The control sits at the top of what it governs; each option pairs a count with a label; an empty result has a way back; the new count is announced.</td><td>Under the search row; “To push 4”; <i>Clear filter</i>; the row reads “4 of 14 files” through its live region.</td></tr>
              <tr><td>{link("https://carbondesignsystem.com/patterns/filtering/", "Carbon, filtering")}</td><td>Apply at once for a single category; while a filter is in force, show it and clear it in one step.</td><td>No Apply; All releases it.</td></tr>
              <tr><td>{link("https://www.nngroup.com/articles/applying-filters/", "NN/g, applying filters")}</td><td>Instant filtering suits a list that answers within a second; options say how many results they hold.</td><td>The inventory is in memory; every scope carries its count.</td></tr>
              <tr><td>{link("https://www.w3.org/WAI/ARIA/apg/patterns/radio/", "ARIA APG, radio group")}</td><td>One checked at a time; arrows move the choice with focus; one tab stop.</td><td><code>StatusScopeBar</code> as it ships.</td></tr>
            </tbody>
          </table>
        </section>
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
