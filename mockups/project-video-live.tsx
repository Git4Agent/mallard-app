import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import App from "../src/App";
import { projectSyncApi } from "../src/components/project-sync/api";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import type {
  AgentFolder,
  CloudProjectFolder,
  LocalProjectRegistration,
  ManagedCloudRepository,
  ProjectActivityRefreshEvent,
  ProjectDetail,
  ProjectFolderProbe,
  ProjectSessionSummary,
  ProjectSetupDraft,
  StorageProviderDescriptor,
  StorageSummary,
  ThreadSyncEntry,
} from "../src/types";

/**
 * Add a project, live: the shipped shell on a stand-in backend, for the docs
 * tutorial video (/docs/add-a-project). `npm run dev`, then
 * /project-video-live.html. `?scene=mate` is the teammate's computer, which
 * adds the same project From Mallard Cloud. `window.__calls` lists every
 * backend call; anything unanswered is logged as `unanswered`.
 */

const HOME = "/Users/you";
const ORIGIN = "https://mallard-ai.com";
const now = Math.floor(Date.now() / 1000);
const scene = new URLSearchParams(window.location.search).get("scene") === "mate" ? "mate" : "main";
const LOGIN = scene === "mate" ? "hen" : "octocat";
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

/* ——— storages ——————————————————————————————————————————————————————— */

const CLOUD_ID = "storage-cloud";
const R2_ID = "storage-r2";
const CLOUD: StorageSummary = {
  storage_id: CLOUD_ID, name: `Mallard Cloud ${LOGIN}`, provider: "managed_cloud", config_version: 1, provider_label: "Mallard Cloud",
  icon: "cloud", available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: LOGIN, is_path: false },
    { field_id: "service_origin", label: "Service", value: ORIGIN, is_path: false },
  ],
  settings: { service_origin: ORIGIN, account_id: `user_${LOGIN}`, account_login: LOGIN },
  secrets: [{ secret_id: "account_token", present: true }],
} as StorageSummary;
const R2: StorageSummary = {
  storage_id: R2_ID, name: "R2 mallard-otter", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true,
  summary: [{ field_id: "bucket", label: "Bucket name", value: "mallard-otter", is_path: false }],
  settings: { bucket: "mallard-otter" }, secrets: [{ secret_id: "secret_access_key", present: true }],
} as StorageSummary;
const STORAGES = scene === "mate" ? [CLOUD] : [CLOUD, R2];

const DESCRIPTORS = [
  { key: "managed_cloud", config_version: 1, display_name: "Mallard Cloud", description: "Private snapshots with team sharing and web access.", default_name: "Mallard Cloud", display_rank: 5, icon: "cloud", clear_mode: "unavailable", fields: [], secrets: [], managed_secret_id: "account_token", local_setup: "managed_cloud" },
  { key: "r2", config_version: 1, display_name: "Cloudflare R2", description: "Store snapshots in your R2 bucket.", default_name: "R2 storage", display_rank: 10, icon: "cloud", clear_mode: "unconditional", fields: [], secrets: [], managed_secret_id: null, local_setup: "wrangler" },
] as unknown as StorageProviderDescriptor[];

/* ——— agents —————————————————————————————————————————————————————————— */

const AGENTS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, has_cli: true, maintenance_actions: [] },
];
const profile = (profileId: string, provider: string, dir: string) => ({
  profile_id: profileId, provider, display_name: `~/${dir}`, path: `${HOME}/${dir}`, canonical_path: `${HOME}/${dir}`,
  revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
});
const PROFILES = [profile("p-codex", "codex", ".codex"), profile("p-claude", "claude", ".claude")];
const PROFILE_IDS = { codex: "p-codex", claude: "p-claude" };

/* ——— projects ———————————————————————————————————————————————————————— */

const FINGERPRINT = { tidewatch: "a1".repeat(32), portal: "b2".repeat(32) };
const TIDEWATCH_BUNDLE = "7".repeat(32);

interface Project { id: string; name: string; root: string; git: boolean; bundle: string; fingerprint: string | null; storageId: string | null }
const projects: Project[] = scene === "mate"
  ? [{ id: "project-dotfiles", name: "dotfiles", root: `${HOME}/code/dotfiles`, git: true, bundle: "5".repeat(32), fingerprint: "c3".repeat(32), storageId: CLOUD_ID }]
  : [{ id: "project-portal", name: "portal", root: `${HOME}/code/portal`, git: true, bundle: "6".repeat(32), fingerprint: FINGERPRINT.portal, storageId: CLOUD_ID }];

const registration = (project: Project): LocalProjectRegistration => ({
  local_project_id: project.id, bundle_id: project.bundle, display_name: project.name, local_alias: null,
  repository_fingerprint: project.fingerprint, recipe: { schema_version: 1, revision: 1, entries: {} }, revision: 1, created_at: now, updated_at: now,
} as LocalProjectRegistration);
const link = (project: Project) => ({
  managed_cloud_repository_id: `repo_${project.name}`, local_project_id: project.id, storage_id: project.storageId!, bundle_id: project.bundle,
  recipe: null, project_content_preferences: { schema_version: 1, revision: 0, excluded_resource_ids: [] },
  conversation_checkpoints: {}, pinned: false, created_at: now,
});
const binding = (project: Project) => ({
  host_id: "host-1", replica_id: "replica-1", local_project_id: project.id, bundle_id: project.bundle, project_root: project.root,
  canonical_project_root: project.root, profile_ids: PROFILE_IDS, state: "active", revision: 1, updated_at: now,
});
const detailOf = (project: Project): ProjectDetail => ({
  project: registration(project), links: project.storageId ? [link(project)] : [], binding: binding(project), materializations: [],
} as unknown as ProjectDetail);
const config = () => ({
  schema: 1, revision: configRevision, storages: STORAGES,
  projects: projects.map(registration), links: projects.filter((project) => project.storageId).map(link),
});
let configRevision = 1;
const byId = (id: string) => projects.find((project) => project.id === id);

/* ——— the folders recent sessions ran in ————————————————————————————— */

const probeOf = (path: string): ProjectFolderProbe => {
  const project = projects.find((candidate) => candidate.root === path);
  const name = path.split("/").pop() ?? "";
  return {
    kind: "folder", canonical_path: path, is_git_repository: name !== "field-notes",
    existing_draft_id: null, existing_project_ids: project ? [project.id] : [],
  };
};
const FOLDERS: AgentFolder[] = (scene === "mate" ? [
  { path: `${HOME}/code/garden`, providers: ["claude"], last_active: now - 40 * 60 },
  { path: `${HOME}/code/dotfiles`, providers: ["codex"], last_active: now - 2 * 86_400 },
] : [
  { path: `${HOME}/code/tidewatch`, providers: ["codex", "claude"], last_active: now - 12 * 60 },
  { path: `${HOME}/notes/field-notes`, providers: ["claude"], last_active: now - 3 * 3600 },
  { path: `${HOME}/code/portal`, providers: ["codex"], last_active: now - 26 * 3600 },
]).map((folder) => ({ ...folder, probe: probeOf(folder.path) })) as AgentFolder[];

/* ——— project setup ——————————————————————————————————————————————————— */

let draft: ProjectSetupDraft | null = null;
const inspection = (current: ProjectSetupDraft) => ({
  draft: current,
  sections: [
    { section: "project", state: "ready" },
    { section: "profiles", state: "ready" },
    { section: "storage", state: "ready" },
    { section: "repository", state: "ready" },
  ],
  can_finalize: true,
});
function newDraft(projectRoot: string): ProjectSetupDraft {
  const name = projectRoot.split("/").pop() ?? "project";
  return {
    schema: 1, draft_id: "draft-1", local_project_id: `project-${name}`, new_bundle_id: TIDEWATCH_BUNDLE, project_root: projectRoot,
    canonical_project_root: projectRoot, display_name: name,
    repository_fingerprint: name === "tidewatch" ? FINGERPRINT.tidewatch : null,
    profiles: { codex: { kind: "existing", profile_id: "p-codex" }, claude: { kind: "existing", profile_id: "p-claude" } },
    storage: null, repository: { kind: "new" }, revision: 1, created_at: now, updated_at: now,
  } as ProjectSetupDraft;
}
function finalize(current: ProjectSetupDraft): ProjectDetail {
  const project: Project = {
    id: current.local_project_id, name: current.display_name, root: current.canonical_project_root, git: true,
    bundle: current.repository.kind === "existing" ? current.repository.bundle_id : current.new_bundle_id,
    fingerprint: current.repository_fingerprint ?? null, storageId: current.storage?.storage_id ?? null,
  };
  projects.push(project);
  draft = null;
  configRevision += 1;
  void emit("project-sync-config-changed", { revision: configRevision });
  return detailOf(project);
}

/* ——— Mallard Cloud ——————————————————————————————————————————————————— */

const repo = (name: string, owner: string, role: string, fingerprint: string, bundle: string, revision: number): ManagedCloudRepository => ({
  repository_id: `repo_${name}`, owner_login: owner, name, address: `${owner}/${name}`, display_name: name, website_url: `${ORIGIN}/${owner}/${name}`,
  bundle_id: bundle, repository_class: "git_project", visibility: "private", role, can_push: true, repository_fingerprint: fingerprint,
  git_remote: "unknown", github_url: `https://github.com/${owner}/${name}`, revision, updated_at: now - 5 * 60, resources: 3,
  registration_only: false, description: null, star_count: 0, viewer_starred: false,
});
/* The teammate sees what the video's first half made: octocat/tidewatch, as a Member. */
const YOURS = scene === "mate"
  ? [repo("tidewatch", "octocat", "member", FINGERPRINT.tidewatch, TIDEWATCH_BUNDLE, 1), repo("dotfiles", "hen", "owner", "c3".repeat(32), "5".repeat(32), 4)]
  : [repo("portal", "octocat", "owner", FINGERPRINT.portal, "6".repeat(32), 9)];
projectSyncApi.listManagedCloudRepositories = async () => { await wait(120); return YOURS; };
projectSyncApi.listManagedCloudStarred = async () => ({ rows: [], next_cursor: null });
projectSyncApi.listManagedCloudPublic = async () => ({ rows: [], next_cursor: null });
projectSyncApi.probeCloudProjectFolder = async () => ({ folder: { kind: "new" } as CloudProjectFolder, can_clone: true });
projectSyncApi.getCloudProjectFolder = async (request, onProgress) => {
  for (const percent of [null, 24, 61, 93]) {
    onProgress({ stage: "cloning", percent });
    if (holdClone && percent === 61) await new Promise<void>((resolve) => { releaseClone = resolve; });
    else await wait(300);
  }
  return { kind: "cloned", canonical_path: request.destination };
};
/* `window.__holdClone = true` keeps the clone at 61% for a still; false lets it finish. */
let holdClone = false;
let releaseClone: (() => void) | null = null;
Object.defineProperty(window, "__holdClone", {
  set: (value: boolean) => {
    holdClone = value;
    if (!value) { releaseClone?.(); releaseClone = null; }
  },
});

const cloudStatus = (project: Project): ManagedCloudStatus => {
  const pushed = pushedHere(project) || onlyInStorage(project);
  return {
    repository_id: `repo_${project.name}`, state: "ready", message: null,
    link: {
      visibility: "private", can_pull: true, repository_id: `repo_${project.name}`, website_url: `${ORIGIN}/octocat/${project.name}`,
      owner_login: project.name === "dotfiles" ? "hen" : "octocat", name: project.name,
      release: pushed ? { kind: "private" } : { kind: "not_pushed" }, repository_revision: pushed ? 1 : 0,
      can_request_release: pushed, can_publish_version: pushed, can_discard_draft: pushed, role: scene === "mate" && project.name === "tidewatch" ? "member" : "owner", can_push: true,
    },
    publication: { draft: pushed ? "pushed" : "none", latest_version: null, next_version: 1, draft_ahead: null, public_version: null, review_version: null },
  };
};

/* ——— a project's sessions ————————————————————————————————————————————— */

const SESSIONS: Record<string, { id: string; provider: "codex" | "claude"; title: string; at: number; rounds: number }[]> = {
  tidewatch: [
    { id: "019fb002-1c4e-7a31-9d0b-3f6a2c8e5d41", provider: "codex", title: "Add tide alerts for low water", at: now - 12 * 60, rounds: 9 },
    { id: "5b0e6f1a-8c2d-4e7b-a913-27d4c5f08e62", provider: "claude", title: "Fix the chart's time zone", at: now - 95 * 60, rounds: 6 },
    { id: "019fa7c4-02bd-7e15-8c6a-91d3e4b7f203", provider: "codex", title: "Plan the station picker", at: now - 26 * 3600, rounds: 14 },
  ],
  portal: [
    { id: "019f9e11-7d2a-7c40-b5e8-6a1f0c3d9e74", provider: "codex", title: "Sign-in page polish", at: now - 30 * 3600, rounds: 5 },
  ],
  dotfiles: [],
};
const sessionsOf = (project: Project) => SESSIONS[project.name] ?? [];
/* Each session lands on the commit it made; the oldest commit has none. */
const COMMITS: Record<string, { subject: string; at: number; session?: number }[]> = {
  tidewatch: [
    { subject: "Alert when the tide drops below a set level", at: now - 9 * 60, session: 0 },
    { subject: "Show chart times in the station's zone", at: now - 92 * 60, session: 1 },
    { subject: "Sketch the station picker", at: now - 25 * 3600, session: 2 },
    { subject: "Initial commit", at: now - 3 * 86_400 },
  ],
  portal: [{ subject: "Tidy the sign-in page", at: now - 29 * 3600, session: 0 }, { subject: "Initial commit", at: now - 6 * 86_400 }],
  dotfiles: [{ subject: "Initial commit", at: now - 9 * 86_400 }],
};
const sha = (seed: string) => [...seed].reduce((hash, char) => (hash * 31 + char.charCodeAt(0)) >>> 0, 7).toString(16).padStart(8, "0").repeat(5);
function gitPage(project: Project) {
  const sessions = sessionsOf(project);
  return {
    selected_branch: "main", branches: [{ name: "main", is_current: true }],
    github_repository_url: `https://github.com/octocat/${project.name}`, unique_session_count: sessions.length,
    commits: (COMMITS[project.name] ?? []).map((commit) => {
      const full = sha(project.name + commit.subject);
      const session = commit.session === undefined ? null : sessions[commit.session];
      return {
        sha: full, short_sha: full.slice(0, 7), committed_at: commit.at, subject: commit.subject,
        session_refs: session ? [{ resource_id: `${session.provider}:conversation:${session.id}` }] : [],
      };
    }),
  };
}
/* octocat has pushed nothing of tidewatch yet; on the teammate's computer
   it is all in storage, waiting for Pull. */
const pushedHere = (project: Project) => project.name !== "tidewatch";
const onlyInStorage = (project: Project) => scene === "mate" && project.name === "tidewatch";

function summary(project: Project, session: (typeof SESSIONS)["tidewatch"][number]): ProjectSessionSummary {
  const name = session.provider === "codex" ? "Codex" : "Claude";
  return {
    resource_id: `${session.provider}:conversation:${session.id}`, provider: session.provider, provider_display_name: name,
    provider_session_id: session.id, logical_thread_id: session.id,
    source: onlyInStorage(project) ? "remote_only" : pushedHere(project) ? "local_and_remote" : "local_only",
    access: "bound_ready", title: session.title, summary: null, started_at: session.at - 40 * 60, ended_at: session.at,
    last_prompt_at: session.at - 60,
    transcript_path: onlyInStorage(project) ? null : `${HOME}/.${session.provider}/sessions/${session.id}.jsonl`,
    working_directory: project.root, sync_blocked_reason: null, recorded_sha: null, is_active: false,
    user_round_count: session.rounds, agent_message_count: session.rounds * 2, tool_call_count: session.rounds * 3,
    total_input_tokens: null, total_output_tokens: null, total_tokens: session.rounds * 21_000, metrics_complete: true,
    actions: [
      { action_id: "open_in_app", kind: "open_in_app", label: `Open in ${name}`, enabled: true },
      { action_id: "open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true },
    ],
  } as ProjectSessionSummary;
}
function entry(project: Project, session: (typeof SESSIONS)["tidewatch"][number]): ThreadSyncEntry {
  const pushed = pushedHere(project);
  if (onlyInStorage(project)) {
    return {
      provider: session.provider, provider_display_name: session.provider === "codex" ? "Codex" : "Claude", access: "bound_ready",
      thread_id: session.id, conversation_resource_id: `${session.provider}:conversation:${session.id}`,
      resource_id: `${session.provider}:session:${session.id}`, display_name: session.title,
      state: "storage_only", local_present: false, storage_present: true, compared_against_storage: true,
      subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
      local_updated_at: null, push_decision: { action: "no_changes", selectable: false },
      pull_decision: { action: "create_local", selectable: true }, capture_bytes: 380_000,
    } as unknown as ThreadSyncEntry;
  }
  return {
    provider: session.provider, provider_display_name: session.provider === "codex" ? "Codex" : "Claude", access: "bound_ready",
    thread_id: session.id, conversation_resource_id: `${session.provider}:conversation:${session.id}`,
    resource_id: `${session.provider}:session:${session.id}`, display_name: session.title,
    state: pushed ? "synced" : "local_only", local_present: true, storage_present: pushed, compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    local_updated_at: session.at,
    push_decision: pushed ? { action: "no_changes", selectable: false } : { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
    capture_bytes: 380_000,
  } as ThreadSyncEntry;
}

projectSyncApi.refreshProjectActivity = Object.assign(
  async (request: { localProjectId: string; storageId: string; windowDays: number }, onEvent: (event: ProjectActivityRefreshEvent) => void) => {
    calls.push(`refresh_project_activity ${request.localProjectId}`);
    const project = byId(request.localProjectId);
    if (!project) return;
    const sessions = sessionsOf(project);
    onEvent({
      stage: "activity", storage_pending: true,
      activity: {
        project_id: project.id, sessions: sessions.map((session) => summary(project, session)), git: project.git ? gitPage(project) : null, unmapped: [],
        providers: AGENTS.map((agent) => ({ provider: agent.key, display_name: agent.display_name, access: "bound_ready" })),
        warnings: [], window_start: now - request.windowDays * 86_400, window_end: now, older: null,
        storage_sync: [{ storage_id: CLOUD_ID, storage_name: CLOUD.name, last_pull_at: null, last_push_at: null }],
      },
    } as ProjectActivityRefreshEvent);
    await wait(150);
    const entries = sessions.map((session) => entry(project, session));
    const pushed = pushedHere(project) || onlyInStorage(project);
    onEvent({
      stage: "comparison",
      comparison: {
        managed_cloud_status: cloudStatus(project), project_id: project.id, storage_id: CLOUD_ID, storage_name: CLOUD.name,
        revision: pushed ? 1 : 0, base_revision: pushed ? 1 : 0, compared_at: now, entries,
        counts: onlyInStorage(project)
          ? { synced: 0, local: 0, storage: entries.length, diverged: 0, unavailable: 0, unknown: 0 }
          : { synced: pushed ? entries.length : 0, local: pushed ? 0 : entries.length, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
        scopes: [], provider_errors: {}, warnings: [],
      },
    } as unknown as ProjectActivityRefreshEvent);
  },
  { invalidate: () => {} },
) as unknown as typeof projectSyncApi.refreshProjectActivity;

/* ——— the stand-in backend ——————————————————————————————————————————— */

const calls: string[] = [];
(window as unknown as { __calls: string[] }).__calls = calls;
const QUIET = new Set(["plugin:event|listen", "plugin:event|unlisten", "plugin:event|emit", "plugin:path|resolve_directory"]);

(window as unknown as { isTauri: boolean }).isTauri = true;
mockWindows("main");
mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  if (!QUIET.has(command)) calls.push(command);
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "get_host_identity_status": return { host_id: "host-1", revision: 1, created_at: now, updated_at: now };
    case "get_project_sync_config": return config();
    case "list_local_projects": return projects.map((project) => ({
      ...registration(project), project_root: project.root, canonical_project_root: project.root, profile_ids: PROFILE_IDS,
      providers: ["codex", "claude"], linked_storage_ids: project.storageId ? [project.storageId] : [], is_git_repository: project.git,
    }));
    case "get_project": { const project = byId(String(args.localProjectId)); return project ? detailOf(project) : null; }
    case "list_project_bindings": return projects.map(binding);
    case "list_project_repository_kinds": return Object.fromEntries(projects.map((project) => [project.id, project.git]));
    case "list_setup_drafts": return { drafts: [] };
    case "list_storage_setup_drafts": return { drafts: [], warnings: [] };
    case "list_storage_providers": return DESCRIPTORS;
    case "list_catalog_scopes": return [];
    case "list_catalog_capabilities": return [];
    case "list_catalog_resource_profiles": return [];
    case "list_agent_providers": return AGENTS;
    case "list_provider_profiles": return PROFILES;
    case "list_agent_folders": await wait(80); return { folders: FOLDERS };
    case "probe_project_folder": return probeOf(String(args.projectRoot));
    case "list_managed_cloud_repositories": return YOURS;
    case "list_managed_cloud_starred":
    case "list_managed_cloud_public":
    case "list_catalog_cloud_starred":
    case "list_catalog_cloud_public": return { rows: [], next_cursor: null };
    case "list_catalog_cloud_shared": return [];
    case "list_remote_bundle_snapshots": return [];
    case "list_catalog_storage_repositories": return [];
    case "take_app_link": return null;
    case "recover_startup_state": return null;
    case "prepare_app_update": return { current_version: "0.1.6", ready_version: null };
    case "get_app_settings": return {
      push_codex_children: true, push_claude_children: true, credential_redaction_enabled: false, update_check_enabled: true, schema: 1,
      conversation_file_limit_bytes: 64 * 1024 * 1024, timeline_preview_limit_bytes: 4 * 1024 * 1024, comparison_workers: 4, cloud_transfer_concurrency: 4,
    };
    case "create_setup_draft": draft = newDraft(String(args.projectRoot)); return { draft, resumed: false };
    case "get_setup_draft": return draft;
    case "update_setup_draft": {
      const next = args.draft as ProjectSetupDraft;
      draft = { ...next, revision: next.revision + 1, updated_at: now };
      return draft;
    }
    case "inspect_setup_draft": return draft ? inspection(draft) : null;
    case "discard_setup_draft": draft = null; return true;
    case "finalize_project_setup": await wait(250); return finalize(draft!);
    case "hold_project_git_watch":
    case "release_project_git_watch": return null;
    default:
      console.warn("unanswered", command, args);
      return null;
  }
}, { shouldMockEvents: true });

createRoot(document.getElementById("root")!).render(<div className="harness-frame"><App /></div>);
