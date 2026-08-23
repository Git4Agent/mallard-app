import { StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import { ProjectChatHistoryContent } from "./components/project-sync/ProjectActivityPage";
import type {
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionSummary,
} from "./types";
import "./App.css";
import "./RetroTheme.css";

/**
 * Specimen for the shipped Storage card: the markup ProjectLinksWorkspace now
 * renders, under the shipped stylesheets, sitting on the real activity
 * timeline. The card wears the Uncommitted card's treatment — same frame, same
 * 38px heading bar, same 26px rail inset — with the empty body laid out like
 * a session row: content against the left edge, action against the right.
 *
 * On a settled project (storage linked, no notices) the card renders nothing
 * at all — the identity lives in the path bar — so "linked" here shows the
 * card gone, which is the shipped behavior, not an omission.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

/* ───────────────────── stub data (the screenshot's) ───────────────────── */

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
  } as ProjectSessionSummary;
}

const PELICAN = session({
  resource_id: "codex:conversation:019f7798-5437-7632-9dbc-5b589cf68be0",
  title: "Can you create a pelican-riding-a-bicycle svg",
  ended_at: at(2026, 8, 18, 22, 1),
  started_at: at(2026, 8, 18, 22, 1) - 180,
  user_round_count: 20,
  agent_message_count: 5,
  tool_call_count: 376,
  total_tokens: 88_000,
});

const HORROR = session({
  resource_id: "codex:conversation:019f7798-5437-7632-9dbc-5b589cf68bf0",
  title: "Add horror story document",
  ended_at: at(2026, 8, 18, 10, 58),
  started_at: at(2026, 8, 18, 10, 58) - (59 * 3_600),
  user_round_count: 10,
  agent_message_count: 7,
  tool_call_count: 7,
  total_tokens: 365_000,
});

const DUCK = session({
  resource_id: "codex:conversation:019f7798-5437-7632-9dbc-5b589cf68bf1",
  title: "生成鹈鹕骑自行车动图",
  ended_at: at(2026, 8, 16, 22, 4),
  started_at: at(2026, 8, 16, 22, 4) - (82 * 3_600),
  user_round_count: 13,
  agent_message_count: 24,
  tool_call_count: 24,
  total_tokens: 1_260_000,
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

const UNCOMMITTED_ROWS = [PELICAN, HORROR, DUCK];

const ACTIVITY: ProjectActivity = {
  project_id: project.local_project_id,
  sessions: [...UNCOMMITTED_ROWS, COMMITTED],
  git: {
    selected_branch: "main",
    branches: [{ name: "main", is_current: true, available: true }],
    github_repository_url: "https://github.com/Git4Agent/mallard",
    commits: COMMITS,
    unique_session_count: 1,
  },
  unmapped: UNCOMMITTED_ROWS.map((row) => ({
    resource_id: row.resource_id,
    reason: "No commit has been made on this branch since the session's last activity",
  })),
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" }],
  warnings: [],
  window_start: at(2026, 7, 19, 0, 0),
  window_end: at(2026, 8, 18, 23, 59),
  next_before: at(2026, 7, 19, 0, 0),
  storage_sync: [],
} as unknown as ProjectActivity;

/* ──────────────── the shipped card, states hand-mirrored ──────────────── */

function ShippedStorageCard({ pickerOpen, onTogglePicker }: {
  pickerOpen: boolean;
  onTogglePicker: () => void;
}) {
  return (
    <section className="project-storage-card" aria-label="mallard-app storage">
      <div className="sync-history-section-heading">
        <h2><Icon name="link" size={14} />Storage</h2>
        <span className="project-storage-heading-actions">
          <button
            type="button"
            className={`sync-history-icon-action${pickerOpen ? " active" : ""}`}
            aria-expanded={pickerOpen}
            aria-controls="storage-link-picker-specimen"
            title="Link storage"
            aria-label="Link storage"
            onClick={onTogglePicker}
          >
            <Icon name="link" size={14} />
          </button>
          <button type="button" className="sync-history-icon-action" title="Add storage" aria-label="Add storage">
            <Icon name="plus" size={14} />
          </button>
        </span>
      </div>
      <div className="project-storage-empty">
        <Icon name="cloud" size={16} />
        <span>No storage linked yet</span>
        <button
          type="button"
          className="btn"
          aria-expanded={pickerOpen}
          aria-controls="storage-link-picker-specimen"
          onClick={onTogglePicker}
        >
          <Icon name="link" size={13} />Link storage
        </button>
      </div>
      {pickerOpen && (
        <div id="storage-link-picker-specimen" className="project-storage-card-footer">
          <div className="storage-link-picker">
            <span>Choose a storage destination</span>
            <button type="button">
              <Icon name="drive" size={15} />
              Local storage 1
              <Icon name="chevron-right" size={13} />
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

/* ─────────────────────────── harness ─────────────────────────── */

function Harness() {
  const [dark, setDark] = useState(
    () => document.documentElement.getAttribute("data-theme") === "dark",
  );
  const [linked, setLinked] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);

  const toggleTheme = () => {
    const next = !dark;
    setDark(next);
    document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
  };

  return (
    <div className="app mockup-root">
      <style>{SHELL_CSS}</style>
      <div className="mockup-shell">
        <div className="mockup-head">
          <div>
            <h1>Storage card &mdash; shipped</h1>
            <p>
              The shipped markup under the shipped stylesheets. The Storage card wears the
              Uncommitted card&rsquo;s treatment, and the empty body is a session-row layout:
              content left, action right. On a settled project the card renders nothing at
              all &mdash; the identity lives in the path bar &mdash; so &ldquo;Show linked&rdquo;
              showing no card is the shipped behavior, not an omission.
            </p>
          </div>
          <div className="mockup-controls">
            <button type="button" className="btn btn-ghost" onClick={toggleTheme}>{dark ? "Light" : "Dark"}</button>
            <button type="button" className="btn btn-ghost" onClick={() => setLinked(!linked)}>
              {linked ? "Show empty" : "Show linked"}
            </button>
          </div>
        </div>

        <div className="mockup-stage sync-history-page">
          {!linked && (
            <ShippedStorageCard
              pickerOpen={pickerOpen}
              onTogglePicker={() => setPickerOpen((current) => !current)}
            />
          )}
          <ProjectChatHistoryContent
            project={project}
            binding={binding}
            activity={ACTIVITY}
            loading={false}
            loadingMore={false}
            actionError={null}
            loadError={null}
            onRetryLoad={() => undefined}
            actionBusyResourceId={null}
            onLoadMore={() => undefined}
            onSessionAction={() => undefined}
            embedded
            comparison={null}
            activeStorageName={linked ? "Local storage 1" : null}
          />
        </div>
      </div>
    </div>
  );
}

const SHELL_CSS = `
/* Same unlock as the other specimens: the app height-locks the chain and
   scrolls inner panes, a specimen is a document and scrolls itself. */
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }

.mockup-shell { width: 100%; max-width: 960px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 18px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 74ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-controls { display: flex; flex: 0 0 auto; gap: 8px; }
.mockup-stage { padding: 18px 18px 22px; border: 1px solid var(--border); border-radius: 6px; background: var(--app-shell); }
body { background: var(--app-shell); }
`;

/* ?theme=dark so a headless capture can take the dark panel without clicking. */
const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
