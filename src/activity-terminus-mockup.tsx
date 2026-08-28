import { StrictMode, useEffect, useState, type ReactNode } from "react";
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
 * Specimen for the shipped rail terminus: the real component, the shipped
 * stylesheet, stub data in each of the five states the control has to hold.
 *
 * The paging control now ends the rail it pages, in the rail's own column,
 * instead of floating at the bottom of the page below the Older activity fold.
 * It names the day one press lands on, because the cursor tracks the newest
 * thing below the window rather than stepping a fixed thirty days into
 * whatever happens to be there.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SHELL_CSS = `
  html, body { height: auto; overflow: auto; }
  #root { height: auto; min-height: 100%; overflow: auto; }
  .mk-page { max-width: 1180px; margin: 0 auto; padding: 28px 24px 96px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 22px; padding-right: 96px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-panel { margin: 0 0 20px; border: 1px solid var(--border-mid); border-radius: 6px; background: var(--surface-card); }
  .mk-panel > header { display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
    padding: 9px 16px; border-bottom: 1px solid var(--border-mid); background: var(--surface-subtle); }
  .mk-panel > header h2 { margin: 0; color: var(--text-0); font-size: var(--type-md); font-weight: var(--weight-bold); }
  .mk-panel > header p { margin: 0 0 0 auto; color: var(--text-2); font-size: var(--type-sm); }
  .mk-tag { padding: 1px 7px; border: 1px solid var(--border-mid); border-radius: 999px;
    color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); text-transform: uppercase; letter-spacing: 0.06em; }
  .mk-stage { padding: 18px 20px 22px; background: var(--bg-0); }
  .mk-theme { position: fixed; z-index: 20; top: 18px; right: 20px;
    display: inline-flex; align-items: center; gap: 7px;
    padding: 6px 12px; border: 1px solid var(--border-mid); border-radius: 999px;
    background: var(--surface-card); color: var(--text-1);
    font-family: var(--font-mono); font-size: var(--type-2xs); text-transform: uppercase; letter-spacing: 0.06em;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.16); cursor: pointer; }
  .mk-theme:hover, .mk-theme:focus-visible { border-color: var(--border); background: var(--bg-hover); color: var(--text-0); }
  .mk-theme > svg { color: var(--text-2); }
`;

const project: LocalProjectSummary = {
  local_project_id: "project-mallard",
  bundle_id: "bundle-mallard",
  display_name: "mallard",
  project_root: "/Users/test/projects/mallard",
  created_at: 1,
  updated_at: 2,
  revision: 1,
} as LocalProjectSummary;

const binding: ProjectBinding = {
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
} as ProjectBinding;

const SESSION_ID = "019f7798-5437-7632-9dbc-5b589cf68bf0";
const RESOURCE_ID = `codex:conversation:${SESSION_ID}`;

const session = {
  resource_id: RESOURCE_ID,
  provider: "codex",
  provider_display_name: "Codex",
  provider_session_id: SESSION_ID,
  logical_thread_id: SESSION_ID,
  source: "local_only",
  access: "bound_ready",
  title: "Reconcile the codex thread allocator",
  preserved: null,
  summary: "Map project-owned sessions onto the first-parent rail.",
  started_at: 1_752_800_000,
  ended_at: 1_752_803_600,
  transcript_path: `/Users/test/.codex/sessions/rollout-${SESSION_ID}.jsonl`,
  working_directory: "/Users/test/projects/mallard",
  branch: "main",
  recorded_sha: "a".repeat(40),
  is_active: false,
  user_round_count: 3,
  agent_message_count: 5,
  tool_call_count: 13,
  total_input_tokens: 20000,
  total_output_tokens: 4800,
  total_tokens: 24800,
  metrics_complete: true,
  actions: [],
} as unknown as ProjectSessionSummary;

const activity = {
  project_id: project.local_project_id,
  sessions: [session],
  git: {
    selected_branch: "main",
    branches: [{ name: "main", is_current: true, available: true }],
    github_repository_url: "https://github.com/Git4Agent/mallard",
    commits: [{
      sha: "b".repeat(40),
      short_sha: "bbbbbbb",
      committed_at: 1_752_802_000,
      subject: "Trim low-value frontend assertions",
      session_refs: [{ resource_id: RESOURCE_ID }],
    }],
    unique_session_count: 1,
  },
  unmapped: [],
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" }],
  warnings: [],
  window_start: 1_750_000_000,
  window_end: 1_753_000_000,
  older: { newest_at: 1_741_000_000, commits: 41, sessions: 6 },
  storage_sync: [],
} as unknown as ProjectActivity;

const emptyGit = { ...activity.git!, commits: [], unique_session_count: 0 };

const defaults = {
  project,
  binding,
  loading: false,
  loadingMore: false,
  actionError: null,
  loadError: null,
  actionBusyResourceId: null,
  onRetryLoad: () => undefined,
  onLoadMore: () => undefined,
  onShowLess: () => undefined,
  canShowLess: false,
  onSessionAction: () => undefined,
  embedded: false,
  comparison: null,
  comparisonLoading: false,
  comparisonError: null,
  activeStorageName: null,
  selectionMode: undefined,
  selectedResourceIds: new Set<string>(),
  onToggleResource: () => undefined,
  onReviewPull: () => undefined,
};

function Panel({ tag, title, words, children }: {
  tag: string;
  title: string;
  words?: string;
  children: ReactNode;
}) {
  return (
    <section className="mk-panel">
      <header>
        <span className="mk-tag">{tag}</span>
        <h2>{title}</h2>
        {words && <p>{words}</p>}
      </header>
      <div className="mk-stage">{children}</div>
    </section>
  );
}

function Mockup() {
  const [theme, setTheme] = useState<"light" | "dark">(() => {
    const requested = new URLSearchParams(location.search).get("theme");
    if (requested === "light" || requested === "dark") return requested;
    return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  });
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    const url = new URL(location.href);
    url.searchParams.set("theme", theme);
    history.replaceState(null, "", url);
  }, [theme]);

  return (
    <div className="sync-main mk-page">
      <style>{SHELL_CSS}</style>

      <button
        type="button"
        className="mk-theme"
        aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
        onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} size={12} />
        {theme === "dark" ? "Light" : "Dark"}
      </button>

      <h1 className="mk-title">Rail foot · landed</h1>

      <Panel tag="normal" title="A page with commits" words="the foot ends the rail">
        <ProjectChatHistoryContent {...defaults} activity={activity} embedded />
      </Panel>

      <Panel tag="paged" title="After a Load earlier" words="two steps, one node">
        <ProjectChatHistoryContent {...defaults} activity={activity} canShowLess embedded />
      </Panel>

      <Panel tag="paged · spent" title="Paged to the end" words="nothing to load, still something to pop">
        <ProjectChatHistoryContent {...defaults} activity={{ ...activity, older: null }} canShowLess embedded />
      </Panel>

      <Panel tag="empty window" title="No commits in range" words="the gap is a card on the rail">
        <ProjectChatHistoryContent {...defaults} activity={{ ...activity, git: emptyGit }} embedded />
      </Panel>

      <Panel tag="loading" title="A fetch in flight" words="only the node moves">
        <ProjectChatHistoryContent {...defaults} activity={{ ...activity, git: emptyGit }} loadingMore embedded />
      </Panel>

      <Panel tag="floor" title="Nothing older" words="no control, no rail">
        <ProjectChatHistoryContent {...defaults} activity={{ ...activity, older: null }} embedded />
      </Panel>

      <Panel tag="no repo" title="Flat Sessions list" words="same control, no rail to hang on">
        <ProjectChatHistoryContent {...defaults} activity={{ ...activity, git: null }} embedded />
      </Panel>

      {/* The review shape: the embedded page inside the review's own scroll
          chrome, which is where the top gutter is decided. */}
      <Panel tag="review" title="Inside a Push review" words="header hairline to first card">
        <div className="sync-inline-action-content sync-sync-review-content">
          <div className="sync-inline-action-scroll sync-sync-review-scroll">
            <ProjectChatHistoryContent
              {...defaults}
              activity={{ ...activity, git: emptyGit }}
              embedded
              selectionMode="push"
              comparison={{ entries: [], storage_name: "Local storage 1" } as never}
            />
          </div>
        </div>
      </Panel>

      <Panel tag="branch" title="History-only branch" words="the window is not the problem">
        <ProjectChatHistoryContent
          {...defaults}
          activity={{ ...activity, git: { ...emptyGit, branches: [{ name: "feature/tail", is_current: false, available: false }], selected_branch: "feature/tail" } }}
          embedded
        />
      </Panel>
    </div>
  );
}

declare global {
  interface Window { __terminusMockupRoot?: Root }
}
const container = document.getElementById("root") as HTMLElement;
(window.__terminusMockupRoot ??= createRoot(container)).render(
  <StrictMode><Mockup /></StrictMode>,
);
