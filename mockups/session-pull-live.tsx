import { useState } from "react";
import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import App from "../src/App";
import { projectSyncApi } from "../src/components/project-sync/api";
import type {
  AppLinkAnswer,
  AppLinkArrival,
  ConversationReviewRow,
  ProjectActivityRefreshEvent,
  ProjectContentInventory,
  ProjectDetail,
  ProjectSessionDetail,
  ProjectSessionSummary,
  PullReview,
  PullScope,
  RestoreResult,
  SessionTimelineEvent,
  ThreadSyncEntry,
} from "../src/types";

/**
 * Pull in Mallard, live: the shipped shell on a stand-in backend, taking a
 * session page's link the way it takes one from the system
 * (docs/PLAN_DEEP_LINK_SESSION_PULL.md). `npm run dev`, then
 * /mockups/session-pull-live.html. `?state=new|synced|newer|diverged|ahead|gone`
 * is where the linked session stands on this computer when the link arrives;
 * `?cold=<link>` starts the app from one. The backend keeps state: Pull makes
 * the session local, and a second link to it then finds it up to date.
 * `window.__calls` lists every backend call with what it was asked.
 */

const HOME = "/Users/you";
const HOST = "mallard-ai.com";
const ORIGIN = `https://${HOST}`;
const now = Math.floor(Date.now() / 1000);
const DAY = 86_400;
const query = new URLSearchParams(window.location.search);
type State = "new" | "synced" | "newer" | "diverged" | "ahead" | "gone";
const initial = (["new", "synced", "newer", "diverged", "ahead", "gone"] as const).find((state) => state === query.get("state")) ?? "new";
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

const PERSONAL = {
  storage_id: "storage-personal", name: "Mallard Cloud", provider: "managed_cloud", config_version: 1, provider_label: "Mallard Cloud", icon: "cloud", available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: "Rika321", is_path: false },
    { field_id: "service_origin", label: "Service", value: ORIGIN, is_path: false },
  ],
  settings: { service_origin: ORIGIN, account_id: "user_Rika321", account_login: "Rika321" },
  secrets: [{ secret_id: "account_token", present: true }],
};

/* ——— projects ———————————————————————————————————————————————————————— */

interface Project { id: string; name: string; root: string; git: boolean; bundle: string; provider: "codex" | "claude"; repository: string }
const HORROR: Project = { id: "project-horror", name: "HorrorGame", root: `${HOME}/code/HorrorGame`, git: true, bundle: "a".repeat(32), provider: "codex", repository: "repo_horror" };
const BIRDING: Project = { id: "project-birding", name: "Birding.Log", root: `${HOME}/Documents/Birding.Log`, git: false, bundle: "c".repeat(32), provider: "claude", repository: "repo_birding" };
const PROJECTS = [HORROR, BIRDING];
const byId = (id: string) => PROJECTS.find((project) => project.id === id);

const registration = (project: Project) => ({
  local_project_id: project.id, bundle_id: project.bundle, display_name: project.name, local_alias: null,
  repository_fingerprint: project.git ? "f".repeat(64) : null,
  recipe: { schema_version: 1, revision: 1, entries: {} }, revision: 1, created_at: now, updated_at: now,
});
const link = (project: Project) => ({
  managed_cloud_repository_id: project.repository, local_project_id: project.id, storage_id: PERSONAL.storage_id, bundle_id: project.bundle,
  recipe: null, project_content_preferences: { schema_version: 1, revision: 0, excluded_resource_ids: [] },
  conversation_checkpoints: {}, pinned: false, created_at: now,
});
const binding = (project: Project) => ({
  host_id: "00112233445566778899aabbccddeeff", replica_id: "replica-this-mac", local_project_id: project.id, bundle_id: project.bundle,
  project_root: project.root, canonical_project_root: project.root, profile_ids: { [project.provider]: `p-${project.provider}` },
  state: "active", revision: 3, updated_at: now - DAY,
});
const detailOf = (project: Project): ProjectDetail => ({
  project: registration(project), links: [link(project)], binding: binding(project), materializations: [],
} as unknown as ProjectDetail);
const CONFIG = { schema: 1, revision: 3, storages: [PERSONAL], projects: PROJECTS.map(registration), links: PROJECTS.map(link) };

/* ——— the stored sessions, and where each stands on this computer ————————— */

interface Session {
  project: Project;
  cloudThreadId: string;
  /** The local thread, once this computer holds the session. */
  localId: string;
  title: string;
  bytes: number;
  endedAt: number;
  state: State;
}
const START = now - 3 * DAY;
const LINKED: Session = {
  project: HORROR, cloudThreadId: "cloudthread-7f3a9c0e2b1d4f6a8c9e", localId: "019fb002-0000-7000-8000-00000000aa01",
  title: "Improve game interaction and collision", bytes: 184 * 1024, endedAt: START + 16 * 60, state: initial,
};
const OTHER: Session = {
  project: HORROR, cloudThreadId: "cloudthread-1b2c3d4e5f6a7b8c9d0e", localId: "019fb002-0000-7000-8000-00000000aa02",
  title: "Sketch the inventory UI", bytes: 96 * 1024, endedAt: START - DAY, state: "new",
};
const FOLDER: Session = {
  project: BIRDING, cloudThreadId: "cloudthread-2c4e6a8b0d1f3a5c7e9b", localId: "5b1e0c7a-2d4f-4c6e-9a8b-0d1f3a5c7e9b",
  title: "Tidy the species list", bytes: 61 * 1024, endedAt: START + 8 * 3600, state: "new",
};
const SESSIONS = [LINKED, OTHER, FOLDER];
const providerName = (session: Session) => (session.project.provider === "codex" ? "Codex" : "Claude");
const cloudId = (session: Session) => `${session.project.provider}:conversation:${session.cloudThreadId}`;
const pageId = (session: Session) => `${session.project.provider}:conversation:${session.localId}`;
const here = (session: Session) => session.state !== "new" && session.state !== "gone";
/** The row's id in a review: the local capture's when the session is here. */
const rowId = (session: Session) => (here(session) ? `${session.project.provider}:session:${session.localId}` : cloudId(session));
const pullAction = (session: Session) => (
  { new: "create_local", newer: "materialize_cloud", diverged: "keep_both", synced: "no_changes", ahead: "report_push_eligible", gone: "no_changes" } as const
)[session.state];

function reviewRow(session: Session): ConversationReviewRow {
  const action = pullAction(session);
  return {
    stored_without_credential_redaction: false, provider: session.project.provider,
    thread_id: here(session) ? session.localId : null, cloud_thread_id: session.cloudThreadId, resource_id: rowId(session),
    action, action_id: action === "report_push_eligible" ? null : `selection-${session.cloudThreadId}`,
    blocker: null, blocked_reason: null, display_title: session.title,
    source_size: action === "report_push_eligible" ? null : session.bytes, stored_ended_at: session.endedAt,
  };
}

function planFor(project: Project, scope: PullScope): PullReview {
  const stored = SESSIONS.filter((session) => session.project === project && session.state !== "gone");
  const wanted = scope.kind === "session" ? stored.filter((session) => cloudId(session) === scope.resource_id) : stored;
  if (scope.kind === "session" && wanted.length === 0) throw "That session isn't in Mallard Cloud any more.";
  const bound = binding(project);
  return {
    apply_limit_bytes: 512 * 1024 * 1024, scope, schema_version: 1,
    plan_id: `plan-${project.id}-${scope.kind}-${Date.now()}`, storage_id: PERSONAL.storage_id, bundle_id: project.bundle,
    host_id: bound.host_id, replica_id: bound.replica_id, expected_host_identity_revision: 0, revision: 7, commit_id: "commit-7",
    manifest_sha256: "a".repeat(64), binding_revision: bound.revision, created_at: now, expires_at: now + DAY, actions: [],
    conversation_review: wanted.filter((session) => pullAction(session) !== "no_changes").map(reviewRow),
    // A session's Pull has no Files step: the eligibility that offers one is absent.
    ...(scope.kind === "project" ? { project_content_eligibility: { state: project.git ? "git_managed" : "eligible", reason: project.git ? "Git manages files in this project folder" : "Project folder is not a Git repository" } } : {}),
    remote_continuity: { state: "in_sync", revision: 7 },
  } as PullReview;
}

function apply(review: PullReview, approved: string[]): RestoreResult {
  for (const session of SESSIONS) {
    if (approved.includes(`selection-${session.cloudThreadId}`)) session.state = "synced";
  }
  return {
    success: true, message: `Applied ${approved.length} restore actions`,
    readiness: { bundle_id: review.bundle_id, state: "ready", issues: [] }, readiness_error: null, plan_id: review.plan_id,
    // A session's Pull moves no reviewed base: the field is absent.
    ...(review.scope.kind === "project" ? { reviewed_base_advanced: true } : {}),
    applied_action_ids: approved, failed_actions: [], preserved_threads: [], after_pull_actions: [], not_pulled_action_ids: [],
  };
}

/* ——— what the project's page reads ——————————————————————————————————— */

function summary(session: Session): ProjectSessionSummary {
  const local = here(session);
  return {
    resource_id: local ? pageId(session) : cloudId(session), provider: session.project.provider, provider_display_name: providerName(session),
    provider_session_id: local ? session.localId : null, logical_thread_id: local ? session.localId : session.cloudThreadId,
    source: local ? "local_and_remote" : "remote_only", access: "bound_ready", title: session.title, preserved: null, summary: null,
    started_at: session.endedAt - 16 * 60, ended_at: session.endedAt, last_prompt_at: session.endedAt - 60,
    transcript_path: local ? `${HOME}/.${session.project.provider}/sessions/${session.localId}.jsonl` : null,
    working_directory: local ? session.project.root : null, recorded_sha: null, is_active: false,
    user_round_count: 2, agent_message_count: 2, tool_call_count: 2, total_tokens: 48_200, metrics_complete: true,
    actions: local ? [{ action_id: "open_in_app", kind: "open_in_app", label: `Open in ${providerName(session)}`, enabled: true }] : [],
  } as ProjectSessionSummary;
}
function entry(session: Session): ThreadSyncEntry {
  const local = here(session);
  const action = pullAction(session);
  return {
    provider: session.project.provider, provider_display_name: providerName(session), access: "bound_ready",
    thread_id: local ? session.localId : session.cloudThreadId, conversation_resource_id: local ? pageId(session) : cloudId(session),
    resource_id: rowId(session), display_name: session.title,
    state: ({ new: "storage_only", newer: "storage_ahead", diverged: "diverged", synced: "synced", ahead: "local_ahead", gone: "synced" } as const)[session.state],
    local_present: local, storage_present: true, storage_updated_at: session.endedAt, local_updated_at: local ? session.endedAt - 3600 : null,
    cloud_thread_id: session.cloudThreadId, compared_against_storage: true,
    subagent_selection: { wire_children: "off", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: local ? session.localId : null, capture_bytes: local ? session.bytes : null, pull_bytes: session.bytes,
    push_decision: session.state === "ahead" ? { action: "publish", selectable: true } : { action: "no_changes", selectable: false },
    pull_decision: { action, selectable: action === "create_local" || action === "materialize_cloud" || action === "keep_both" },
  } as ThreadSyncEntry;
}
const listed = (project: Project) => SESSIONS.filter((session) => session.project === project && session.state !== "gone");

function activityOf(project: Project) {
  return {
    project_id: project.id, sessions: listed(project).map(summary), git: null, unmapped: [],
    providers: [{ provider: project.provider, display_name: project.provider === "codex" ? "Codex" : "Claude", access: "bound_ready" }],
    warnings: [], window_start: now - 30 * DAY, window_end: now, older: null,
    storage_sync: [{ storage_id: PERSONAL.storage_id, storage_name: PERSONAL.name, last_pull_at: now - DAY, last_push_at: now - 2 * DAY }],
  };
}
function comparisonOf(project: Project) {
  const entries = listed(project).map(entry);
  const count = (...states: string[]) => entries.filter((candidate) => states.includes(candidate.state)).length;
  return {
    managed_cloud_status: {
      repository_id: project.repository, state: "ready", message: null,
      link: {
        visibility: "private", can_pull: true, repository_id: project.repository, website_url: `${ORIGIN}/Rika321/${project.name}`,
        owner_login: "Rika321", name: project.name, release: { kind: "private" }, repository_revision: 7, can_request_release: true, can_publish_version: true, role: "owner", can_push: true,
      },
      publication: { draft: "pushed", latest_version: null, next_version: 1, draft_ahead: null, public_version: null, review_version: null },
    },
    project_id: project.id, storage_id: PERSONAL.storage_id, storage_name: PERSONAL.name, revision: 7, base_revision: 6, compared_at: now, entries,
    counts: { synced: count("synced"), local: count("local_ahead"), storage: count("storage_only", "storage_ahead"), diverged: count("diverged"), unavailable: 0, unknown: 0 },
    scopes: [], provider_errors: {}, warnings: [], capture_budget: { limit_bytes: 512 * 1024 * 1024, reserved_bytes: 0 },
    remote_continuity: { state: "in_sync", revision: 7 },
  };
}

projectSyncApi.getProjectActivity = (async (localProjectId: string) => {
  await wait(120);
  return activityOf(byId(localProjectId)!);
}) as unknown as typeof projectSyncApi.getProjectActivity;
projectSyncApi.refreshProjectActivity = Object.assign(
  async (request: { localProjectId: string }, onEvent: (event: ProjectActivityRefreshEvent) => void) => {
    const project = byId(request.localProjectId);
    if (!project) return;
    onEvent({ stage: "activity", storage_pending: true, activity: activityOf(project) } as unknown as ProjectActivityRefreshEvent);
    await wait(200);
    onEvent({ stage: "comparison", comparison: comparisonOf(project) } as unknown as ProjectActivityRefreshEvent);
  },
  { invalidate: () => {} },
) as unknown as typeof projectSyncApi.refreshProjectActivity;

/* The folder's files: one is still only in Mallard Cloud. */
const filesOf = (project: Project): ProjectContentInventory => ({
  local_project_id: project.id, storage_id: PERSONAL.storage_id, project_root: project.root,
  eligibility: { state: "eligible", reason: "Project folder is not a Git repository" }, review_token: null, storage_revision: 7,
  preference_revision: 0, excluded_resource_ids: [],
  entries: [{
    descriptor: { resource_id: "project:file:species.csv", kind: "project_file", display_name: "species.csv" },
    relative_path: "species.csv", entry_type: "file", state: "storage_only", local_present: false, storage_present: true,
    size: 18_400, undecided: false, blocked_reason: null, warnings: [],
  }],
  ignored_count: 0, blocked_count: 0, warnings: [], scanned_at: now,
} as unknown as ProjectContentInventory);

/* ——— a session's own page ———————————————————————————————————————————— */

const exact = (value: number) => ({ value, quality: "exact" as const });
const sessionOfPage = (resourceId: string) => SESSIONS.find((session) => pageId(session) === resourceId && here(session));
const detailOfSession = (session: Session): ProjectSessionDetail => ({
  resource_id: pageId(session), provider: session.project.provider, provider_display_name: providerName(session),
  provider_session_id: session.localId, title: session.title, summary: null, started_at: session.endedAt - 16 * 60, ended_at: session.endedAt,
  is_active: false, branch: session.project.git ? "main" : null, recorded_sha: null, tokens: exact(48_200),
  event_counts: {
    prompts: exact(1), responses: exact(1), intermediate_steps: exact(0), checkpoints: exact(0), tool_calls: exact(0),
    file_edits: exact(0), shell_calls: exact(0), reads: exact(0), other_tools: exact(0), agent_calls: exact(0),
  },
  actions: [{ action_id: "open_in_app", kind: "open_in_app", label: `Open in ${providerName(session)}`, enabled: true }],
  snapshot_id: `snapshot-${session.localId}`, warnings: [],
} as unknown as ProjectSessionDetail);
const eventsOf = (session: Session): SessionTimelineEvent[] => ([
  { event_id: "p1", ordinal: 1, timestamp: session.endedAt - 16 * 60, kind: "prompt", quality: "exact", turn_id: "t1", text: `${session.title}.` },
  { event_id: "r1", ordinal: 2, timestamp: session.endedAt, kind: "response", quality: "exact", turn_id: "t1", text: "Done." },
] as unknown as SessionTimelineEvent[]);
projectSyncApi.getProjectSessionDetail = (async (_projectId: string, resourceId: string) => {
  await wait(150);
  const session = sessionOfPage(resourceId);
  if (!session) throw new Error(`no such session: ${resourceId}`);
  return detailOfSession(session);
}) as typeof projectSyncApi.getProjectSessionDetail;
projectSyncApi.getProjectSessionTimelinePage = (async (_projectId: string, resourceId: string, page: { snapshot_id: string; included_event_kinds: string[] }) => {
  const session = sessionOfPage(resourceId);
  if (!session) throw new Error(`no such session: ${resourceId}`);
  const kinds = new Set(page.included_event_kinds);
  return { resource_id: resourceId, snapshot_id: page.snapshot_id, events: eventsOf(session).filter((event) => kinds.has(event.kind)), next_cursor: null, warnings: [] };
}) as unknown as typeof projectSyncApi.getProjectSessionTimelinePage;

/* ——— links ——————————————————————————————————————————————————————————— */

const sessionLink = (address: string, session: Session): AppLinkArrival => ({
  kind: "session", host: HOST, address, provider: session.project.provider, cloud_thread_id: session.cloudThreadId, resource_id: cloudId(session),
});
const LINKS_TO_SEND: Record<string, AppLinkArrival> = {
  session: sessionLink("Rika321/HorrorGame", LINKED),
  "session-folder": sessionLink("Rika321/Birding.Log", FOLDER),
  "session-away": sessionLink("maya/portal", LINKED),
  // A `sessions/{id}` the website never writes is refused in Rust.
  forged: { kind: "invalid" },
  repository: { kind: "repository", host: HOST, address: "Rika321/HorrorGame" },
};
const SIGHTS: Record<string, AppLinkAnswer["accounts"][number]["sight"]> = {
  "rika321/horrorgame": { kind: "found", repository: { repository_id: HORROR.repository, address: "Rika321/HorrorGame", repository_class: "git_project", bundle_id: HORROR.bundle, catalog: null, visibility: "private", role: "owner" } },
  "rika321/birding.log": { kind: "found", repository: { repository_id: BIRDING.repository, address: "Rika321/Birding.Log", repository_class: "non_git_project", bundle_id: BIRDING.bundle, catalog: null, visibility: "private", role: "owner" } },
  "maya/portal": { kind: "found", repository: { repository_id: "repo_maya_portal", address: "maya/portal", repository_class: "git_project", bundle_id: "b".repeat(32), catalog: null, visibility: "public", role: null } },
};

/* ——— the stand-in backend ————————————————————————————————————————————— */

const calls: string[] = [];
(window as unknown as { __calls: string[] }).__calls = calls;
let render = () => {};
const note = (line: string) => {
  calls.push(line);
  render();
};
let waiting: AppLinkArrival | null = LINKS_TO_SEND[query.get("cold") ?? ""] ?? null;
const QUIET = new Set(["plugin:event|listen", "plugin:event|unlisten", "plugin:event|emit", "plugin:path|resolve_directory"]);

(window as unknown as { isTauri: boolean }).isTauri = true;
mockWindows("main");
mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  if (!QUIET.has(command)) note(command === "plan_bundle_restore" ? `${command} ${JSON.stringify(args.scope)}` : command);
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "get_host_identity_status": return { host_id: "host-1", revision: 1, created_at: now, updated_at: now };
    case "get_project_sync_config": return CONFIG;
    case "list_local_projects": return PROJECTS.map((project) => ({
      ...registration(project), project_root: project.root, canonical_project_root: project.root,
      profile_ids: { [project.provider]: `p-${project.provider}` }, providers: [project.provider],
      linked_storage_ids: [PERSONAL.storage_id], is_git_repository: project.git,
    }));
    case "get_project": { const project = byId(String(args.localProjectId)); return project ? detailOf(project) : null; }
    case "list_project_bindings": return PROJECTS.map(binding);
    case "list_project_repository_kinds": return Object.fromEntries(PROJECTS.map((project) => [project.id, project.git]));
    case "list_setup_drafts": return { drafts: [] };
    case "list_storage_setup_drafts": return { drafts: [], warnings: [] };
    case "list_storage_providers": return [];
    case "list_catalog_scopes":
    case "list_catalog_capabilities":
    case "list_catalog_resource_profiles": return [];
    case "list_agent_providers": return [
      { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
      { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, maintenance_actions: [] },
    ];
    case "list_provider_profiles": return (["codex", "claude"] as const).map((provider) => ({
      profile_id: `p-${provider}`, provider, display_name: `~/.${provider}`, path: `${HOME}/.${provider}`, canonical_path: `${HOME}/.${provider}`,
      revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
    }));
    case "list_agent_folders": return { folders: [] };
    case "list_managed_cloud_repositories": return [];
    case "list_managed_cloud_starred":
    case "list_managed_cloud_public": return { rows: [], next_cursor: null };
    case "recover_startup_state": return null;
    case "prepare_app_update": return { current_version: "0.1.7", ready_version: null };
    case "get_app_settings": return {
      push_codex_children: true, push_claude_children: true, credential_redaction_enabled: true, update_check_enabled: false, schema: 1,
      conversation_file_limit_bytes: 64 * 1024 * 1024, timeline_preview_limit_bytes: 4 * 1024 * 1024, comparison_workers: 4, cloud_transfer_concurrency: 4,
    };
    case "inspect_project_files": { const project = byId(String(args.localProjectId)); return project ? filesOf(project) : null; }
    case "take_app_link": { const taken = waiting; waiting = null; return taken; }
    case "resolve_app_link": {
      await wait(400);
      const address = String(args.address);
      const sight = SIGHTS[address.toLowerCase()] ?? { kind: "not_found" };
      return {
        host: String(args.host), address, can_set_up: true,
        accounts: [{ storage_id: PERSONAL.storage_id, storage_name: PERSONAL.name, account_login: "Rika321", sight }],
      } satisfies AppLinkAnswer;
    }
    case "plan_bundle_restore": {
      await wait(500);
      const project = PROJECTS.find((candidate) => candidate.bundle === args.bundleId)!;
      return planFor(project, args.scope as PullScope);
    }
    case "apply_bundle_restore": {
      await wait(700);
      return apply(args.review as PullReview, args.approvedActionIds as string[]);
    }
    case "get_restore_readiness": return { bundle_id: byId(String(args.localProjectId))!.bundle, state: "ready", issues: [] };
    case "find_project_session": {
      const session = SESSIONS.find((candidate) => cloudId(candidate) === args.resourceId && candidate.project.id === args.localProjectId);
      return session && here(session) ? pageId(session) : null;
    }
    default:
      return null;
  }
}, { shouldMockEvents: true });

/** What the system does when a link is opened: the backend keeps it and says so. */
async function send(name: string) {
  waiting = LINKS_TO_SEND[name];
  note(`— link: ${name}`);
  await emit("app-link");
}

function Harness() {
  const [, setTick] = useState(0);
  render = () => setTick((tick) => tick + 1);
  return (
    <div className="harness">
      <div className="harness-controls">
        <strong>Open a link</strong>
        {Object.keys(LINKS_TO_SEND).map((name) => (
          <button key={name} type="button" data-link={name} onClick={() => void send(name)}>{name}</button>
        ))}
        <span>state={LINKED.state}</span>
        <button type="button" onClick={() => { calls.length = 0; render(); }}>Clear log</button>
      </div>
      <div className="harness-frame"><App /></div>
      <pre className="harness-log" aria-live="polite">{calls.length ? calls.join("\n") : "Backend calls appear here."}</pre>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Harness />);
