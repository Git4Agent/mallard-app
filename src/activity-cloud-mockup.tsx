import { StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import { ProjectChatHistoryContent } from "./components/project-sync/ProjectActivityPage";
import type {
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionSummary,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "./types";
import "./App.css";
import "./RetroTheme.css";

/**
 * Specimen for the shipped storage-only group: the real component, the shipped
 * stylesheet, stub data shaped like the screenshot that started this. Storage
 * -only sessions that no commit claims render in their own "Only in <storage>"
 * group above the timeline instead of under Uncommitted. Not part of the app
 * bundle: the build has one entry, and this file is reachable only from the
 * dev server.
 */

const STORAGE_NAME = "Local storage 1";

/** Local time, so the stamps read the way the screenshot's do. */
const at = (y: number, m: number, d: number, h: number, min: number) =>
  Math.floor(new Date(y, m - 1, d, h, min).getTime() / 1000);

function session(overrides: Partial<ProjectSessionSummary> & {
  resource_id: string;
  title: string;
  ended_at: number;
}): ProjectSessionSummary {
  return {
    provider: "codex",
    provider_display_name: "Codex",
    provider_session_id: overrides.resource_id.split(":").pop() ?? null,
    logical_thread_id: overrides.resource_id.split(":").pop() ?? null,
    source: "local_only",
    access: "bound_ready",
    preserved: null,
    summary: null,
    started_at: overrides.ended_at - 3_600,
    working_directory: "/Users/hequ/Desktop/project/mallard-app",
    relative_cwd: null,
    branch: "main",
    recorded_sha: null,
    is_active: false,
    user_round_count: 4,
    agent_message_count: 6,
    tool_call_count: 9,
    total_tokens: 48_000,
    metrics_complete: true,
    actions: [
      { action_id: "codex.open_in_app", kind: "open_in_app", label: "Open in Codex", enabled: true, disabled_reason: null },
      { action_id: "codex.open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true, disabled_reason: null },
    ],
    ...overrides,
  };
}

/** A session that exists in the bundle and nowhere on this machine: no local
 *  file, so the backend hands it no actions and the detail view is closed. */
function storedSession(overrides: Partial<ProjectSessionSummary> & {
  resource_id: string;
  title: string;
  ended_at: number;
}): ProjectSessionSummary {
  return session({
    ...overrides,
    source: "remote_only",
    working_directory: null,
    recorded_sha: null,
    actions: [],
  });
}

const HORROR = storedSession({
  resource_id: "codex:conversation:019f7798-5437-7632-9dbc-5b589cf68bf0",
  title: "Add horror story document",
  ended_at: at(2026, 8, 17, 21, 11),
  started_at: at(2026, 8, 17, 21, 11) - (45 * 3_600),
  user_round_count: 5,
  agent_message_count: 5,
  tool_call_count: 7,
  total_tokens: 365_000,
});

const DUCK = storedSession({
  resource_id: "codex:conversation:019f7798-5437-7632-9dbc-5b589cf68bf1",
  title: "生成鹈鹕骑自行车动图",
  ended_at: at(2026, 8, 16, 22, 4),
  started_at: at(2026, 8, 16, 22, 4) - (82 * 3_600),
  user_round_count: 5,
  agent_message_count: 8,
  tool_call_count: 24,
  total_tokens: 1_260_000,
});

/** Local work with no commit after it — what "Uncommitted" is actually for. */
const DRAFT = session({
  resource_id: "codex:conversation:019f7798-5437-7632-9dbc-5b589cf68bf2",
  title: "Split the storage-only rows out of Uncommitted",
  ended_at: at(2026, 8, 18, 14, 2),
  started_at: at(2026, 8, 18, 14, 2) - (2 * 3_600 + 40 * 60),
  user_round_count: 11,
  agent_message_count: 14,
  tool_call_count: 38,
  total_tokens: 214_000,
});

const COMMITTED = session({
  resource_id: "codex:conversation:019f7798-5437-7632-9dbc-5b589cf68bf3",
  title: "Enhance the game loop",
  ended_at: at(2026, 8, 1, 0, 40),
  started_at: at(2026, 8, 1, 0, 40) - 5_400,
  recorded_sha: "9f75e4c000000000000000000000000000000000",
  user_round_count: 7,
  agent_message_count: 9,
  tool_call_count: 16,
  total_tokens: 96_000,
});

const COMMITS = [
  { sha: "7350274000000000000000000000000000000000", short_sha: "7350274", committed_at: at(2026, 8, 2, 0, 11), subject: "Test Again", session_refs: [] },
  { sha: "9f75e4c000000000000000000000000000000000", short_sha: "9f75e4c", committed_at: at(2026, 8, 1, 0, 57), subject: "Enhance UI and GameLoop", session_refs: [{ resource_id: COMMITTED.resource_id }] },
  { sha: "d2f8387000000000000000000000000000000000", short_sha: "d2f8387", committed_at: at(2026, 7, 27, 22, 9), subject: "weiren", session_refs: [] },
];

const project: LocalProjectSummary = {
  local_project_id: "project-mallard",
  bundle_id: "bundle-mallard",
  display_name: "mallard-app",
  local_alias: null,
  repository_fingerprint: "fingerprint",
  project_root: "/Users/hequ/Desktop/project/mallard-app",
  canonical_project_root: "/Users/hequ/Desktop/project/mallard-app",
  profile_ids: { codex: "profile-codex" },
  profile_names: ["Default Codex"],
  created_at: 1,
  updated_at: 2,
  revision: 1,
} as LocalProjectSummary;

const binding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica",
  local_project_id: project.local_project_id,
  bundle_id: project.bundle_id,
  project_root: project.project_root,
  canonical_project_root: project.project_root,
  profile_ids: { codex: "profile-codex" },
  state: "active",
  revision: 1,
  updated_at: 2,
} as unknown as ProjectBinding;

function activityFor(sessions: ProjectSessionSummary[], unmapped: ProjectSessionSummary[]): ProjectActivity {
  return {
    project_id: project.local_project_id,
    sessions,
    git: {
      selected_branch: "main",
      branches: [{ name: "main", is_current: true, available: true }],
      github_repository_url: "https://github.com/Git4Agent/mallard",
      commits: COMMITS,
      unique_session_count: 1,
    },
    unmapped: unmapped.map((row) => ({ resource_id: row.resource_id, reason: "No commit has been made on this branch since the session's last activity" })),
    providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" }],
    warnings: [],
    window_start: at(2026, 7, 19, 0, 0),
    window_end: at(2026, 8, 18, 23, 59),
    next_before: at(2026, 7, 19, 0, 0),
    storage_sync: [{ storage_id: "storage-1", storage_name: STORAGE_NAME, last_pull_at: at(2026, 8, 15, 9, 0), last_push_at: at(2026, 8, 17, 18, 0) }],
  } as ProjectActivity;
}

function entry(row: ProjectSessionSummary, state: ThreadSyncEntry["state"]): ThreadSyncEntry {
  const threadId = row.resource_id.split(":").pop() ?? row.resource_id;
  return {
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    thread_id: threadId,
    conversation_resource_id: row.resource_id,
    resource_id: row.resource_id,
    display_name: row.title,
    state,
    local_present: state !== "storage_only",
    storage_present: state !== "local_only",
    status_detail: null,
    local_updated_at: row.ended_at,
    storage_updated_at: row.ended_at,
    cloud_thread_id: null,
    forked_from: null,
    provider_thread_id: threadId,
    push_decision: { action: state === "storage_only" ? "not_pushable" : "publish", selectable: state !== "storage_only" },
    pull_decision: { action: state === "storage_only" ? "materialize_cloud" : "no_changes", selectable: state === "storage_only" },
    origin_label: null,
  } as ThreadSyncEntry;
}

function comparisonFor(entries: ThreadSyncEntry[]): ThreadSyncComparison {
  return {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: STORAGE_NAME,
    revision: 12,
    base_revision: 12,
    compared_at: at(2026, 8, 18, 15, 0),
    entries,
    counts: {
      synced: entries.filter((row) => row.state === "synced").length,
      local: entries.filter((row) => row.state === "local_only").length,
      storage: entries.filter((row) => row.state === "storage_only").length,
      diverged: 0,
      unavailable: 0,
      unknown: 0,
    },
    scopes: [],
    provider_errors: {},
    warnings: [],
    remote_continuity: { state: "in_sync", revision: 12 },
  } as unknown as ThreadSyncComparison;
}

const STORED = [HORROR, DUCK];
const COMPARISON = comparisonFor([
  entry(HORROR, "storage_only"),
  entry(DUCK, "storage_only"),
  entry(DRAFT, "local_only"),
  entry(COMMITTED, "synced"),
]);

/** Your data: two stored rows, one local draft, three commits. The component
 *  now routes the stored rows into the group by itself. */
const MIXED = activityFor([DRAFT, COMMITTED, ...STORED], [DRAFT, ...STORED]);
/** The screenshot's actual shape: Uncommitted held nothing but the stored
 *  rows, so the split empties it and the heading disappears with it. */
const STORED_ONLY = activityFor([COMMITTED, ...STORED], STORED);

const timelineProps = {
  project,
  binding,
  loading: false,
  loadingMore: false,
  actionError: null,
  loadError: null,
  onRetryLoad: () => undefined,
  actionBusyResourceId: null,
  onLoadMore: () => undefined,
  onSessionAction: () => undefined,
  embedded: true,
  comparison: COMPARISON,
  activeStorageName: STORAGE_NAME,
};

function Timeline({ activity }: { activity: ProjectActivity }) {
  return <ProjectChatHistoryContent {...timelineProps} activity={activity} />;
}

function Panel({ title, note, children }: { title: string; note: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(START_DARK);
  return (
    <div className="app mockup-root">
      <style>{SHELL_CSS}</style>
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Storage-only sessions — shipped</h1>
            <p>
              The real component, no mockup JSX left: storage-only sessions that no commit claims
              render in <strong>Only in {STORAGE_NAME}</strong> above the timeline, and Uncommitted
              keeps only local work. The rows are the ordinary two-line card, untouched. A stored
              session that does map to a commit stays on the rail as provenance.
            </p>
          </div>
          <button type="button" className="btn" onClick={() => {
            const next = !dark;
            setDark(next);
            document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
          }}>
            <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel
          title="Stored rows and a local draft"
          note={<>Both lists exist: the group holds the two stored sessions, Uncommitted keeps the
            local draft, and each fold announces its own count.</>}
        >
          <Timeline activity={MIXED} />
        </Panel>

        <Panel
          title="Only stored rows unmapped — the screenshot's shape"
          note={<>Uncommitted held nothing but the stored rows, so the split empties it: the page
            opens on the group, then goes straight into the commit rail.</>}
        >
          <Timeline activity={STORED_ONLY} />
        </Panel>
      </div>
    </div>
  );
}

const SHELL_CSS = `
/* html, body, #root and .app are all height-locked with overflow:hidden, because
   in the real app the inner panes scroll and the window never does. A mockup is a
   document, so the whole chain is unlocked and the *document* scrolls — not an
   inner div. That distinction is the whole fix: a scroll container nested inside
   a locked document still answers scrollTop, so it looks fine to a script and is
   dead to a mouse wheel, the spacebar, Page Down and find-on-page.

   Specificity, not source order, does the work: #root needs an id to beat an id,
   and .app.mockup-root needs two classes to beat .app. */
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }

/* width:100% is load-bearing — .app is a column flex container, and an auto
   cross-axis margin without it makes the shell shrink to its content. */
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 74ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 78ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-stage { padding: 14px 16px 18px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); }
body { background: var(--app-shell); }
`;

/* ?theme=dark so a headless capture can take the dark panel without clicking. */
const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
