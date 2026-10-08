import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import App from "../src/App";
import { projectSyncApi } from "../src/components/project-sync/api";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import type {
  AgentFolder,
  InventoryResource,
  ProjectFolderProbe,
  ProjectSetupDraft,
  StorageSetupDraft,
  LocalProjectRegistration,
  ProjectActivityRefreshEvent,
  ProjectDetail,
  ProjectSessionDetail,
  ProjectSessionSummary,
  ProjectSessionTimelineQuery,
  SessionEventDetail,
  SessionTimelineEvent,
  StorageProviderDescriptor,
  StorageSummary,
  ThreadSyncEntry,
} from "../src/types";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import type {
  CatalogCapabilityDescriptor,
  CatalogLinkState,
  CatalogProfile,
  CatalogPullAction,
  CatalogPullDisposition,
  CatalogResourceRelation,
  CatalogScopeDto,
  CatalogScopePullItem,
  CatalogScopePushRow,
  CatalogScopeRow,
  LibraryHeadCoordinate,
} from "../src/components/storage-catalog/types";
import { installChineseShim, CJK_FONT_CSS } from "./i18n-zh-shim";

/**
 * Chinese UI, live: the shipped shell on a stand-in backend, with the
 * specimen-only shim from i18n-zh-shim.ts rewriting what is on screen. The
 * shim only sets text and attributes after React renders; nothing in src/
 * knows about it. Mounted in frames by i18n-zh-mockup.html.
 *
 *   ?screen=sessions|push|settings|error|storages|storage-add|storage-cloud|storage-cloud-signin|
 *           storage-r2|storage-r2-create|storage-r2-signin|storage-r2-account|storage-r2-bucket|
 *           storage-r2-authorize|storage-r2-existing|storage-dropbox-signin|storage-dropbox-folder|
 *           storage-local|storage-local-ready|project-add|project-setup|logs|library|
 *           cloud-private|cloud-draft|cloud-public|cloud-review|cloud-signin|cloud-publish|
 *           storage-page|project-menu|project-rename|project-binding|link-storage|session-info|
 *           new-session-menu|confirm-delete-session|confirm-remove-project|confirm-unlink|update|startup|
 *           session-detail|session-detail-tool|session-detail-agent|session-detail-en|session-detail-child|
 *           library-scope|library-push|library-pushed|library-pull|library-pull-overwrite|library-pulled|
 *           library-fork|library-fork-refused|library-cloud-push|library-cloud-pull|
 *           delete-session-in-use|delete-close-codex|delete-codex-closed|delete-all-stopped
 *   ?lang=en|zh   ?font=default|cjk
 *   ?pref=system|en|zh (the Language row, direction A)   ?theme=light|dark
 *
 * Owner rules (2026-10-07): what the user typed is never translated, so the
 * sample data is the same mixed Chinese and English in both languages; and
 * backend errors stay English.
 *
 * `window.__untranslated` lists English the shim met and could not place.
 */

const query = new URLSearchParams(window.location.search);
const screen = query.get("screen") ?? "sessions";
const zh = query.get("lang") === "zh";
const cjkFont = query.get("font") === "cjk";
const pref = (["system", "en", "zh"] as const).find((value) => value === query.get("pref")) ?? "system";
const theme = query.get("theme") === "dark" ? "dark" : "light";

try {
  window.localStorage.setItem("mallard.theme", theme);
  window.localStorage.setItem("mallard.active-storage-by-project", "{}");
} catch { /* a refused write leaves the default theme */ }

const HOME = "/Users/you";
const ORIGIN = "https://mallard-ai.com";
const LOGIN = "octocat";
const now = Math.floor(Date.now() / 1000);
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));
const KB = 1024;
const MB = 1024 * KB;

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
const R2_STORAGE: StorageSummary = {
  storage_id: R2_ID, name: "R2 mallard-otter", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true,
  summary: [{ field_id: "bucket", label: "Bucket name", value: "mallard-otter", is_path: false }],
  settings: { bucket: "mallard-otter" }, secrets: [{ secret_id: "secret_access_key", present: true }],
} as StorageSummary;
const STORAGES = [CLOUD, R2_STORAGE];
/* Descriptors are the Rust adapters' own, copied from storage-video-live.tsx. */
const whole = (source: string, template: string) => ({ source, pattern: "^\\s*(\\S.*?)\\s*$", group: 1, template });

const MALLARD: StorageProviderDescriptor = {
  key: "managed_cloud", config_version: 1, display_name: "Mallard Cloud",
  description: "Private snapshots with team sharing and web access.",
  default_name: "Mallard Cloud", name_suggestion: whole("account_login", "Mallard Cloud {1}"),
  display_rank: 5, icon: "cloud", clear_mode: "unavailable", fields: [],
  secrets: [{ secret_id: "account_token", label: "Sign-in token", required: true, placeholder: "Issued by the browser sign-in", help: "Issued at sign-in and valid 90 days. Reconnect to renew it." }],
  managed_secret_id: "account_token", local_setup: "managed_cloud", setup_guide_url: null, manual_setup_hint: null, setup_doors: null, discovery: null,
} as StorageProviderDescriptor;
const R2_DOC = "https://developers.cloudflare.com/r2/";
const R2: StorageProviderDescriptor = {
  key: "r2", config_version: 1, display_name: "Cloudflare R2", description: "Store snapshots in your R2 bucket.",
  default_name: "R2 storage", name_suggestion: whole("bucket", "R2 {1}"),
  display_rank: 10, icon: "cloud", clear_mode: "unconditional",
  fields: [
    { field_id: "api_url", label: "S3 API URL", kind: "url", required: true, placeholder: "Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent", help: "How to find the Cloudflare R2 S3 API URL", help_url: R2_DOC },
    { field_id: "bucket", label: "Bucket name", kind: "text", required: true, placeholder: "Example: codex-sync-backups", help: "How to create a Cloudflare R2 bucket", help_url: R2_DOC },
    { field_id: "account_id", label: "Account ID", kind: "text", required: true, placeholder: "Example: 023e105f4ecef8ad9ca31a8372d0c353", help: "Where to find your Cloudflare Account ID", help_url: R2_DOC },
    { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true, placeholder: "Example: 4a2b…91ef", help: "How to create an R2 Access Key ID", help_url: R2_DOC },
  ],
  secrets: [{ secret_id: "secret_access_key", label: "Secret Access Key", required: true, placeholder: "Example: a1B2…x9Y0", help: "How to create an R2 Secret Access Key", help_url: R2_DOC }],
  managed_secret_id: null, local_setup: "wrangler", setup_guide_url: R2_DOC,
  manual_setup_hint: "Enter an existing R2 connection in the fields below.",
  setup_doors: {
    create_hint: "A new bucket in your Cloudflare account.",
    existing_hint: "A bucket Mallard already uses.",
    existing_help: { label: "Set up team access", url: "https://developers.cloudflare.com/fundamentals/manage-members/manage/" },
  },
  discovery: {
    unit: "bucket", unit_plural: "buckets", place: "your Cloudflare account", find_label: "Find in my account",
    access_hint: "Uses your Wrangler sign-in. Read-only: lists the buckets and reads one marker file in each.",
    scanning_hint: "Looking through your Cloudflare account…",
    prepare_hint: "Cloudflare issues an access key for this bucket only.",
  },
} as StorageProviderDescriptor;
const DROPBOX_GUIDE = "https://www.dropbox.com/developers/reference/auth-types";
const DROPBOX: StorageProviderDescriptor = {
  key: "dropbox", config_version: 1, display_name: "Dropbox", description: "A Dropbox folder. Up to 150 MiB per object.",
  default_name: "Dropbox storage", name_suggestion: whole("folder_name", "Dropbox {1}"),
  display_rank: 20, icon: "cloud", clear_mode: "revision_checked",
  fields: [{ field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true, placeholder: "Shared or team namespace ID", help: "Use the same namespace ID on each machine. This value cannot change after setup.", help_url: DROPBOX_GUIDE }],
  secrets: [
    { secret_id: "access_token", label: "Access token", required: true, placeholder: "User-linked Dropbox access token", help: "For manual setup, supply account_info.read, files.metadata.read, files.content.read, and files.content.write. Replace this token when it expires. Browser setup renews access automatically.", help_url: DROPBOX_GUIDE },
    { secret_id: "refresh_token", label: "Refresh token", required: true, placeholder: "Dropbox refresh token", help: "Browser setup saves this token to renew access automatically. Show it to review the value, or enter a replacement issued for this Mallard app and Dropbox account.", help_url: DROPBOX_GUIDE },
  ],
  managed_secret_id: "refresh_token", local_setup: "dropbox", setup_guide_url: DROPBOX_GUIDE,
  manual_setup_hint: "Enter an existing shared or team namespace and a user-linked token. Mallard stores data in .mallard within that namespace.",
  setup_doors: { create_hint: "A new shared folder in your Dropbox.", existing_hint: "A shared folder Mallard already uses.", existing_help: null },
  discovery: {
    unit: "shared folder", unit_plural: "shared folders", place: "your Dropbox", find_label: "Find in my Dropbox",
    access_hint: "Discovery lists accessible folders and reads their Mallard markers without changing them.",
    scanning_hint: "Approve access in your browser, then Mallard looks through your shared folders…",
    prepare_hint: "This is the connection Mallard keeps, renewed automatically.",
  },
} as StorageProviderDescriptor;
const FEISHU_GUIDE = "https://open.feishu.cn/document/";
const FEISHU: StorageProviderDescriptor = {
  key: "feishu", config_version: 1, display_name: "Feishu", description: "A Feishu Drive folder. Up to 100 MiB per object.",
  default_name: "Feishu storage", name_suggestion: whole("folder_name", "Feishu {1}"),
  display_rank: 25, icon: "cloud", clear_mode: "unavailable",
  fields: [{ field_id: "folder_token", label: "Folder token", kind: "text", required: true, placeholder: "The Drive folder's token", help: "The Drive folder this storage lives in. Fixed after setup; share it in Feishu with each user who connects.", help_url: FEISHU_GUIDE }],
  secrets: [], managed_secret_id: null, local_setup: "feishu", setup_guide_url: FEISHU_GUIDE, manual_setup_hint: null,
  setup_doors: { create_hint: "A new folder in your My Space.", existing_hint: "A folder shared with you in Feishu.", existing_help: null },
  discovery: null,
} as StorageProviderDescriptor;
const LOCAL: StorageProviderDescriptor = {
  key: "local", config_version: 1, display_name: "Local folder", description: "A local folder or mounted drive.",
  default_name: "Local storage", name_suggestion: { source: "directory", pattern: "([^/\\\\]+)[/\\\\]*$", group: 1, template: "{1}" },
  display_rank: 30, icon: "drive", clear_mode: "revision_checked",
  fields: [{ field_id: "directory", label: "Location path", kind: "directory", required: true, placeholder: "e.g. /Volumes/backup/mallard-storage", help: null, help_url: null }],
  secrets: [], managed_secret_id: null, local_setup: null, setup_guide_url: null, manual_setup_hint: null, setup_doors: null, discovery: null,
} as StorageProviderDescriptor;
const PROVIDERS = [MALLARD, R2, DROPBOX, FEISHU, LOCAL];
const byKey = (key: string) => PROVIDERS.find((provider) => provider.key === key)!;

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

/* ——— projects: tidewatch has three sessions waiting to go up ——————————— */

interface Project { id: string; name: string; root: string; git: boolean; bundle: string; fingerprint: string | null; storageId: string }
const projects: Project[] = [
  { id: "project-tidewatch", name: "tidewatch", root: `${HOME}/code/tidewatch`, git: true, bundle: "7".repeat(32), fingerprint: "a1".repeat(32), storageId: CLOUD_ID },
  { id: "project-portal", name: "portal", root: `${HOME}/code/portal`, git: true, bundle: "6".repeat(32), fingerprint: "b2".repeat(32), storageId: CLOUD_ID },
  { id: "project-notes", name: "野外笔记", root: `${HOME}/notes/野外笔记`, git: false, bundle: "5".repeat(32), fingerprint: null, storageId: R2_ID },
];
const byId = (id: string) => projects.find((project) => project.id === id);

const registration = (project: Project): LocalProjectRegistration => ({
  local_project_id: project.id, bundle_id: project.bundle, display_name: project.name, local_alias: null,
  repository_fingerprint: project.fingerprint, recipe: { schema_version: 1, revision: 1, entries: {} }, revision: 1, created_at: now, updated_at: now,
} as LocalProjectRegistration);
const link = (project: Project) => ({
  ...(project.storageId === CLOUD_ID ? { managed_cloud_repository_id: `repo_${project.name}` } : {}),
  local_project_id: project.id, storage_id: project.storageId, bundle_id: project.bundle,
  recipe: null, project_content_preferences: { schema_version: 1, revision: 0, excluded_resource_ids: [] },
  conversation_checkpoints: {}, pinned: false, created_at: now,
});
const binding = (project: Project) => ({
  host_id: "host-1", replica_id: "replica-1", local_project_id: project.id, bundle_id: project.bundle, project_root: project.root,
  canonical_project_root: project.root, profile_ids: PROFILE_IDS, state: "active", revision: 1, updated_at: now,
});
const detailOf = (project: Project, withBinding = true): ProjectDetail => ({
  project: registration(project), links: [link(project)], binding: withBinding ? binding(project) : null, materializations: [],
} as unknown as ProjectDetail);
const config = () => ({ schema: 1, revision: 1, storages: STORAGES, projects: projects.map(registration), links: projects.map(link) });

/* ——— sessions: titles are the user's own words, kept as typed in either UI language ——— */

interface Session { id: string; provider: "codex" | "claude"; title: string; at: number; rounds: number; bytes: number }
const SESSIONS: Record<string, Session[]> = {
  tidewatch: [
    { id: "019fb002-1c4e-7a31-9d0b-3f6a2c8e5d41", provider: "codex", title: "添加低潮位提醒", at: now - 12 * 60, rounds: 9, bytes: 1.4 * MB },
    { id: "5b0e6f1a-8c2d-4e7b-a913-27d4c5f08e62", provider: "claude", title: "Fix the chart's time zone", at: now - 95 * 60, rounds: 6, bytes: 415 * KB },
    { id: "019fa7c4-02bd-7e15-8c6a-91d3e4b7f203", provider: "codex", title: "规划测站选择器", at: now - 26 * 3600, rounds: 14, bytes: 2.2 * MB },
  ],
  portal: [
    { id: "019f9e11-7d2a-7c40-b5e8-6a1f0c3d9e74", provider: "codex", title: "Sign-in page polish", at: now - 30 * 3600, rounds: 5, bytes: 300 * KB },
  ],
  "野外笔记": [
    { id: "8c1d2e3f-4a5b-4c6d-9e7f-0a1b2c3d4e5f", provider: "claude", title: "总结三月的调查", at: now - 3 * 3600, rounds: 4, bytes: 210 * KB },
  ],
};
const sessionsOf = (project: Project) => SESSIONS[project.name] ?? [];
const titleOf = (session: Session) => session.title;
const pushedHere = (project: Project) => project.name !== "tidewatch";

const COMMITS: Record<string, { subject: string; at: number; session?: number }[]> = {
  tidewatch: [
    { subject: "潮位低于设定值时提醒", at: now - 9 * 60, session: 0 },
    { subject: "Show chart times in the station's zone", at: now - 92 * 60, session: 1 },
    { subject: "测站选择器草图", at: now - 25 * 3600, session: 2 },
    { subject: "Initial commit", at: now - 3 * 86_400 },
  ],
  portal: [{ subject: "Tidy the sign-in page", at: now - 29 * 3600, session: 0 }, { subject: "Initial commit", at: now - 6 * 86_400 }],
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

const providerName = (provider: string) => (provider === "codex" ? "Codex" : "Claude");
function summary(project: Project, session: Session): ProjectSessionSummary {
  const name = providerName(session.provider);
  return {
    resource_id: `${session.provider}:conversation:${session.id}`, provider: session.provider, provider_display_name: name,
    provider_session_id: session.id, logical_thread_id: session.id,
    source: pushedHere(project) ? "local_and_remote" : "local_only",
    access: "bound_ready", title: titleOf(session), summary: null, started_at: session.at - 40 * 60, ended_at: session.at,
    last_prompt_at: session.at - 60,
    transcript_path: `${HOME}/.${session.provider}/sessions/${session.id}.jsonl`,
    working_directory: project.root, sync_blocked_reason: null, recorded_sha: null, is_active: false,
    user_round_count: session.rounds, agent_message_count: session.rounds * 2, tool_call_count: session.rounds * 3,
    total_input_tokens: null, total_output_tokens: null, total_tokens: session.rounds * 21_000, metrics_complete: true,
    actions: [
      { action_id: "open_in_app", kind: "open_in_app", label: `Open in ${name}`, enabled: true },
      { action_id: "open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true },
    ],
  } as ProjectSessionSummary;
}
function entry(project: Project, session: Session): ThreadSyncEntry {
  const pushed = pushedHere(project);
  return {
    provider: session.provider, provider_display_name: providerName(session.provider), access: "bound_ready",
    thread_id: session.id, conversation_resource_id: `${session.provider}:conversation:${session.id}`,
    resource_id: `${session.provider}:session:${session.id}`, display_name: titleOf(session),
    state: pushed ? "synced" : "local_only", local_present: true, storage_present: pushed, compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    local_updated_at: session.at,
    push_decision: pushed ? { action: "no_changes", selectable: false } : { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
    capture_bytes: session.bytes,
  } as ThreadSyncEntry;
}
function resource(session: Session): InventoryResource {
  return {
    resource_id: `${session.provider}:session:${session.id}`, kind: { key: "conversation", version: 1 }, provider: session.provider,
    display_name: titleOf(session), provenance: { kind: "provider_home", version: 1, fields: {} },
    apply_policy: { key: "explicit_review", version: 1 }, codec_version: 1, attributes: {}, category: "conversations",
    kind_label: "Session", apply_policy_label: "Explicit review", selectable: true, default_selected: true, logical_paths: [],
  };
}

/* portal is pushed; `?screen=cloud-<state>` picks where its repository stands. */
const CLOUD_STATE = screen === "cloud-publish" ? "draft" : screen.startsWith("cloud-") ? screen.slice("cloud-".length) : "private";
const cloudStatus = (project: Project): ManagedCloudStatus => {
  const pushed = pushedHere(project);
  if (pushed && CLOUD_STATE === "signin") {
    return { repository_id: `repo_${project.name}`, state: "sign_in_required", link: null, publication: null, message: null };
  }
  const visibility = pushed && CLOUD_STATE !== "private" ? "public" : "private";
  const release = !pushed ? { kind: "not_pushed" as const }
    : CLOUD_STATE === "review" ? { kind: "waiting_super_admin" as const }
      : visibility === "public" ? { kind: "public" as const } : { kind: "private" as const };
  const ahead = CLOUD_STATE === "public" ? 0 : 2;
  return {
    repository_id: `repo_${project.name}`, state: "ready", message: null,
    link: {
      visibility, can_pull: true, repository_id: `repo_${project.name}`, website_url: `${ORIGIN}/octocat/${project.name}`,
      owner_login: LOGIN, name: project.name, release, repository_revision: pushed ? 7 : 0,
      can_request_release: pushed && CLOUD_STATE !== "review", can_publish_version: pushed && ahead > 0, can_discard_draft: pushed && ahead > 0,
      role: "owner", can_push: true,
    },
    publication: pushed
      ? { draft: "pushed", latest_version: 2, next_version: 3, draft_ahead: ahead, public_version: visibility === "public" ? 2 : null, review_version: null }
      : { draft: "none", latest_version: null, next_version: 1, draft_ahead: null, public_version: null, review_version: null },
  } as ManagedCloudStatus;
};

projectSyncApi.refreshProjectActivity = Object.assign(
  async (request: { localProjectId: string; storageId: string; windowDays: number }, onEvent: (event: ProjectActivityRefreshEvent) => void) => {
    const project = byId(request.localProjectId);
    if (!project) return;
    const sessions = sessionsOf(project);
    onEvent({
      stage: "activity", storage_pending: true,
      activity: {
        project_id: project.id, sessions: sessions.map((session) => summary(project, session)), git: project.git ? gitPage(project) : null, unmapped: [],
        providers: AGENTS.map((agent) => ({ provider: agent.key, display_name: agent.display_name, access: "bound_ready" })),
        warnings: [], window_start: now - request.windowDays * 86_400, window_end: now, older: null,
        storage_sync: [{ storage_id: project.storageId, storage_name: project.storageId === CLOUD_ID ? CLOUD.name : R2_STORAGE.name, last_pull_at: null, last_push_at: null }],
      },
    } as ProjectActivityRefreshEvent);
    await wait(120);
    const entries = sessions.map((session) => entry(project, session));
    const pushed = pushedHere(project);
    onEvent({
      stage: "comparison",
      comparison: {
        ...(project.storageId === CLOUD_ID ? { managed_cloud_status: cloudStatus(project) } : {}),
        project_id: project.id, storage_id: project.storageId, storage_name: project.storageId === CLOUD_ID ? CLOUD.name : R2_STORAGE.name,
        revision: pushed ? 1 : 0, base_revision: pushed ? 1 : 0, compared_at: now, entries,
        counts: { synced: pushed ? entries.length : 0, local: pushed ? 0 : entries.length, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
        scopes: [], provider_errors: {}, warnings: [],
      },
    } as unknown as ProjectActivityRefreshEvent);
  },
  { invalidate: () => {} },
) as unknown as typeof projectSyncApi.refreshProjectActivity;

/* ——— a session's own page: the transcript is the user's and the agent's
   words, kept as recorded in either UI language; tool output and the
   backend's notes stay English. ——————————————————————————————————————— */

const exact = (value: number) => ({ value, quality: "exact" as const });
type Turn = Omit<SessionTimelineEvent, "ordinal" | "quality"> & { quality?: SessionTimelineEvent["quality"] };
const tool = (
  id: string, at: number, turn: string, native: string, category: "read" | "shell" | "file_edit" | "other", summary: string,
  extra: { status?: "completed" | "failed"; paths?: string[]; args?: string; result?: string } = {},
): Turn => ({
  event_id: id, timestamp: at, kind: "tool_call", turn_id: turn,
  tool: {
    provider_call_id: `call-${id}`, native_name: native, category, status: extra.status ?? "completed", summary,
    argument_preview: extra.args ?? null, argument_truncated: false, result_preview: extra.result ?? null, result_truncated: false,
    affected_paths: extra.paths ?? [],
  },
});
interface Transcript { detail: Omit<ProjectSessionDetail, "resource_id" | "snapshot_id">; events: Turn[]; details: Record<string, Omit<SessionEventDetail, "event_id">> }

/* 添加低潮位提醒 (Codex): written in Chinese, answered in Chinese; the second
   turn switches to English, as people do. */
function codexTranscript(session: Session): Transcript {
  const t0 = session.at - 40 * 60;
  const commit = sha(`tidewatch${COMMITS.tidewatch[0].subject}`);
  return {
    detail: {
      provider: "codex", provider_display_name: "Codex", provider_session_id: session.id, title: session.title,
      summary: null, started_at: t0, ended_at: session.at, is_active: false, branch: "main", recorded_sha: commit,
      tokens: { value: 412_880, quality: "lower_bound", explanation: "Some usage records were absent." },
      event_counts: {
        prompts: exact(2), responses: exact(2),
        intermediate_steps: { value: 1, quality: "lower_bound", explanation: "One legacy phase may be missing." },
        checkpoints: exact(1), tool_calls: exact(7), file_edits: exact(2), shell_calls: exact(2), reads: exact(1), other_tools: exact(1), agent_calls: exact(1),
      },
      actions: [
        { action_id: "open_in_app", kind: "open_in_app", label: "Open in Codex", enabled: true },
        { action_id: "open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true },
      ],
      github_repository_url: "https://github.com/octocat/tidewatch",
      warnings: [{ code: "usage_partial", message: "Token usage is missing for 1 turn; the total is a lower bound.", severity: "info" }],
      settings: { model: "gpt-5.5-codex", effort: "high", source: "turn" },
    },
    events: [
      { event_id: "p1", timestamp: t0, kind: "prompt", turn_id: "t1", turn_tool_summary: { turn_id: "t1", tool_calls: exact(6) },
        settings: { model: "gpt-5.5-codex", effort: "high", source: "turn" },
        text: "潮位低于 **0.3 m** 时给我发提醒。阈值放在设置里，默认 0.3；同一次低潮只提醒一次。" },
      { event_id: "s1", timestamp: t0 + 40, kind: "intermediate_step", turn_id: "t1", text: "先看一下现有的提醒规则是怎么存的，再决定阈值放在哪里。" },
      tool("c1", t0 + 55, "t1", "exec", "read", "rg -n \"threshold\" src/alerts src/settings", { paths: ["src/alerts/rules.ts"],
        args: "rg -n \"threshold\" src/alerts src/settings", result: "src/alerts/rules.ts:14:  threshold: number;\nsrc/settings/defaults.ts:22:  rainThreshold: 12," }),
      tool("c2", t0 + 3 * 60, "t1", "apply_patch", "file_edit", "src/alerts/lowTide.ts, src/settings/defaults.ts", {
        paths: ["src/alerts/lowTide.ts", "src/settings/defaults.ts"],
        args: "*** Begin Patch\n*** Add File: src/alerts/lowTide.ts\n+export function lowTide(level: number, threshold = 0.3) {\n+  return level < threshold;\n+}\n*** End Patch" }),
      tool("c3", t0 + 6 * 60, "t1", "exec", "shell", "npm test -- alerts", { status: "failed",
        args: "npm test -- alerts", result: "FAIL src/alerts/lowTide.test.ts\n  ✕ fires once per low tide across a DST change (14 ms)\n\nTests: 1 failed, 11 passed, 12 total" }),
      tool("c4", t0 + 9 * 60, "t1", "apply_patch", "file_edit", "src/alerts/lowTide.ts", { paths: ["src/alerts/lowTide.ts"] }),
      tool("c5", t0 + 10 * 60, "t1", "exec", "shell", "npm test -- alerts", { args: "npm test -- alerts", result: "Tests: 12 passed, 12 total\nTime:  1.84 s" }),
      { event_id: "a1", timestamp: t0 + 11 * 60, kind: "agent_call", turn_id: "t1",
        tool: { provider_call_id: "spawn-1", native_name: "spawn_agent", category: "other", status: "completed", summary: "Review the alert copy", argument_truncated: false, result_truncated: false, affected_paths: [] },
        agent: {
          provider_agent_id: "019fb003-77aa", label: "Review the alert copy", role: "reviewer", model: "gpt-5.5-codex", mode: "spawn", status: "completed",
          started_at: t0 + 11 * 60, ended_at: t0 + 17 * 60, tool_calls: 4, tokens: 38_400,
          brief: "检查提醒文案：要短，带上测站名和当前潮位，不要用感叹号。", brief_truncated: false,
          report: "文案没问题。建议把“潮位过低”改成“低于 0.3 m”，读者能直接看到阈值。", report_truncated: false,
          transcript_resource_id: `codex:conversation:${session.id}/agent/019fb003-77aa`,
        } },
      tool("c6", t0 + 18 * 60, "t1", "web_search", "other", "NOAA tide predictions datum MLLW", { result: "Tide predictions are relative to Mean Lower Low Water (MLLW)…" }),
      { event_id: "k1", timestamp: t0 + 26 * 60, kind: "checkpoint", turn_id: "t1",
        checkpoint: { sha: commit, short_sha: commit.slice(0, 7), subject: COMMITS.tidewatch[0].subject, committed_at: t0 + 26 * 60, branch: "main" } },
      { event_id: "r1", timestamp: t0 + 27 * 60, kind: "response", turn_id: "t1", settings: { model: "gpt-5.5-codex", effort: "high", source: "turn" },
        text: "已添加低潮位提醒：\n\n- 新规则在 `src/alerts/lowTide.ts`：潮位低于阈值时提醒一次，回升到阈值以上后才会再次提醒。\n- 阈值在 **设置 › 提醒** 里，默认 `0.3 m`。\n\n`npm test -- alerts` 已通过。第一次失败是夏令时那天的用例重复提醒，已修正。" },
      { event_id: "p2", timestamp: t0 + 35 * 60, kind: "prompt", turn_id: "t2", turn_tool_summary: { turn_id: "t2", tool_calls: exact(1) },
        settings: { model: "gpt-5.5-codex", effort: "medium", source: "turn" },
        text: "Also draw the threshold on the tide chart." },
      tool("c7", t0 + 37 * 60, "t2", "apply_patch", "file_edit", "src/chart/TideChart.tsx", { paths: ["src/chart/TideChart.tsx"] }),
      { event_id: "r2", timestamp: session.at, kind: "response", turn_id: "t2", settings: { model: "gpt-5.5-codex", effort: "medium", source: "turn" },
        text: "Done. The chart draws a dashed line at the threshold, labelled with its value." },
    ],
    details: {
      c1: { arguments: "{\"cmd\":[\"rg\",\"-n\",\"threshold\",\"src/alerts\",\"src/settings\"]}", arguments_truncated: false,
        result: "src/alerts/rules.ts:14:  threshold: number;\nsrc/settings/defaults.ts:22:  rainThreshold: 12,", result_truncated: false, affected_paths: ["src/alerts/rules.ts"] },
      c3: { arguments: "{\"cmd\":[\"npm\",\"test\",\"--\",\"alerts\"]}", arguments_truncated: false,
        result: "FAIL src/alerts/lowTide.test.ts\n  ✕ fires once per low tide across a DST change (14 ms)\n\n    Expected: 1 alert\n    Received: 2 alerts\n\nTests: 1 failed, 11 passed, 12 total", result_truncated: false, affected_paths: [] },
    },
  };
}

/* Fix the chart's time zone (Claude): an English transcript stays English in
   the Chinese UI, word for word. */
function claudeTranscript(session: Session): Transcript {
  const t0 = session.at - 40 * 60;
  const commit = sha(`tidewatch${COMMITS.tidewatch[1].subject}`);
  return {
    detail: {
      provider: "claude", provider_display_name: "Claude", provider_session_id: session.id, title: session.title,
      summary: null, started_at: t0, ended_at: session.at, is_active: false, branch: "main", recorded_sha: commit,
      tokens: exact(126_300),
      event_counts: {
        prompts: exact(1), responses: exact(1), intermediate_steps: exact(0), checkpoints: exact(1), tool_calls: exact(3),
        file_edits: exact(1), shell_calls: exact(1), reads: exact(1), other_tools: exact(0), agent_calls: exact(1),
      },
      actions: [
        { action_id: "open_in_app", kind: "open_in_app", label: "Open in Claude", enabled: true },
        { action_id: "open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true },
      ],
      warnings: [], settings: { model: "claude-opus-5-5", effort: "high", source: "reply" },
    },
    events: [
      { event_id: "p1", timestamp: t0, kind: "prompt", turn_id: "t1", turn_tool_summary: { turn_id: "t1", tool_calls: exact(3) },
        settings: { model: "claude-opus-5-5", effort: "high", source: "linked" },
        text: "The tide chart shows times in UTC. Show them in the station's own time zone." },
      tool("c1", t0 + 30, "t1", "Read", "read", "src/chart/axis.ts", { paths: ["src/chart/axis.ts"] }),
      { event_id: "a1", timestamp: t0 + 2 * 60, kind: "agent_call", turn_id: "t1",
        tool: { provider_call_id: "task-1", native_name: "Task", category: "other", status: "completed", summary: "Check DST edge cases", argument_truncated: false, result_truncated: false, affected_paths: [] },
        agent: {
          provider_agent_id: "a1b2c3", label: "Check DST edge cases", role: "general-purpose", model: "claude-sonnet-5-5", mode: "async", status: "completed",
          started_at: t0 + 2 * 60, ended_at: t0 + 14 * 60, tool_calls: 9, tokens: 52_100,
          brief: "Find the days where a station's offset changes and list what the axis would get wrong.", brief_truncated: false,
          report: "Two cases matter: the 23-hour day in March and the 25-hour day in November. Format with the station's zone, not a fixed offset.", report_truncated: false,
          transcript_resource_id: null,
        } },
      tool("c2", t0 + 15 * 60, "t1", "Edit", "file_edit", "src/chart/axis.ts", { paths: ["src/chart/axis.ts"] }),
      tool("c3", t0 + 20 * 60, "t1", "Bash", "shell", "npm test -- chart", { result: "Tests: 8 passed, 8 total" }),
      { event_id: "k1", timestamp: t0 + 30 * 60, kind: "checkpoint", turn_id: "t1",
        checkpoint: { sha: commit, short_sha: commit.slice(0, 7), subject: COMMITS.tidewatch[1].subject, committed_at: t0 + 30 * 60, branch: "main" } },
      { event_id: "r1", timestamp: session.at, kind: "response", turn_id: "t1", settings: { model: "claude-opus-5-5", effort: "high", source: "reply" },
        text: "Done. Chart times now use the station's zone (`America/Los_Angeles` for Monterey), and the axis label names it. Both DST days are covered by new tests." },
    ],
    details: {},
  };
}

/* The reviewer the Codex session spawned, opened as its own page. */
function childTranscript(parent: Session): Transcript {
  const t0 = parent.at - 29 * 60;
  return {
    detail: {
      provider: "codex", provider_display_name: "Codex", provider_session_id: "019fb003-77aa", title: "Review the alert copy",
      summary: null, started_at: t0, ended_at: t0 + 6 * 60, is_active: false, branch: "main", recorded_sha: null,
      tokens: exact(38_400),
      event_counts: {
        prompts: exact(1), responses: exact(1), intermediate_steps: exact(0), checkpoints: exact(0), tool_calls: exact(2),
        file_edits: exact(0), shell_calls: exact(1), reads: exact(1), other_tools: exact(0), agent_calls: exact(0),
      },
      actions: [], warnings: [],
      agent: {
        parent_resource_id: `codex:conversation:${parent.id}`, parent_title: parent.title,
        preview: { label: "Review the alert copy", role: "reviewer", model: "gpt-5.5-codex", mode: "spawn", status: "completed", brief_truncated: false, report_truncated: false },
      },
    },
    events: [
      { event_id: "p1", timestamp: t0, kind: "prompt", turn_id: "t1", text: "检查提醒文案：要短，带上测站名和当前潮位，不要用感叹号。" },
      tool("c1", t0 + 40, "t1", "exec", "read", "sed -n '1,80p' src/alerts/copy.ts", { paths: ["src/alerts/copy.ts"] }),
      tool("c2", t0 + 3 * 60, "t1", "exec", "shell", "npm run lint -- src/alerts"),
      { event_id: "r1", timestamp: t0 + 6 * 60, kind: "response", turn_id: "t1",
        text: "文案没问题。建议把“潮位过低”改成“低于 0.3 m”，读者能直接看到阈值。" },
    ],
    details: {},
  };
}

function transcriptOf(resourceId: string): Transcript | null {
  const [tideA, tideB] = SESSIONS.tidewatch;
  if (resourceId === `codex:conversation:${tideA.id}`) return codexTranscript(tideA);
  if (resourceId === `claude:conversation:${tideB.id}`) return claudeTranscript(tideB);
  if (resourceId.endsWith("/agent/019fb003-77aa")) return childTranscript(tideA);
  return null;
}
projectSyncApi.getProjectSessionDetail = (async (_projectId: string, resourceId: string) => {
  await wait(120);
  const transcript = transcriptOf(resourceId);
  if (!transcript) throw new Error(`session ${resourceId} is not in this project's provider home`);
  return { ...transcript.detail, resource_id: resourceId, snapshot_id: `snapshot-${resourceId}` } as ProjectSessionDetail;
}) as typeof projectSyncApi.getProjectSessionDetail;
projectSyncApi.getProjectSessionTimelinePage = (async (_projectId: string, resourceId: string, page: ProjectSessionTimelineQuery) => {
  await wait(80);
  const kinds = new Set(page.included_event_kinds);
  const categories = new Set(page.included_tool_categories);
  const events = (transcriptOf(resourceId)?.events ?? [])
    .map((event, index) => ({ quality: "exact" as const, ...event, ordinal: index + 1 }) as SessionTimelineEvent)
    .filter((event) => kinds.has(event.kind)
      && (event.kind !== "tool_call" || !event.tool || categories.has(event.tool.category))
      && (!page.turn_id || event.turn_id === page.turn_id));
  return { resource_id: resourceId, snapshot_id: page.snapshot_id, events, next_cursor: null, warnings: [] };
}) as typeof projectSyncApi.getProjectSessionTimelinePage;
projectSyncApi.getProjectSessionEventDetail = (async (_projectId: string, resourceId: string, _snapshot: string, eventId: string) => {
  await wait(60);
  const transcript = transcriptOf(resourceId);
  const event = transcript?.events.find((candidate) => candidate.event_id === eventId);
  const known = transcript?.details[eventId];
  return {
    event_id: eventId,
    arguments: known?.arguments ?? event?.tool?.argument_preview ?? null, arguments_truncated: false,
    result: known?.result ?? event?.tool?.result_preview ?? null, result_truncated: false,
    affected_paths: known?.affected_paths ?? event?.tool?.affected_paths ?? [],
  };
}) as typeof projectSyncApi.getProjectSessionEventDetail;

/* ——— the Library: Claude skills on R2 (one repository for the row) or on
   Mallard Cloud (one repository per skill). Names and descriptions are the
   skill authors' words (a name is ASCII by the backend's rule, a description
   is free text); backend refusals stay English. Only `library-*`
   screens list rows, so the other frames keep the empty Library. ——————— */

const LIBRARY = screen.startsWith("library-");
const LIBRARY_CLOUD = screen.startsWith("library-cloud");
const LIBRARY_STORAGE = LIBRARY_CLOUD ? CLOUD_ID : R2_ID;
const libraryProfile = (provider: "claude" | "codex"): CatalogProfile => ({
  profile_id: `p-${provider}`, provider, provider_label: providerName(provider), display_name: providerName(provider),
  configured_home: `${HOME}/.${provider}`, revision: 1, created_at: now - 9 * 86_400, updated_at: now - 9 * 86_400,
});
const SKILLS: CatalogCapabilityDescriptor = {
  provider: "claude", provider_label: "Claude", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1,
  label: "Skills", singular_label: "Skill", presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
  capture: "directory_tree", apply: "automatic", targets: ["user.skills"],
};
const PLUGINS: CatalogCapabilityDescriptor = {
  provider: "claude", provider_label: "Claude", resource_kind: "plugin_install_intent", format: "mallard.plugin-install-intent", format_version: 1,
  label: "Plugins", singular_label: "Plugin", presentation: { icon: "plugin", kind_label: "Plugins", format_label: "Plugin install intent" },
  capture: "provider_inventory", apply: "automatic", targets: ["user.plugins"],
};
const PETS: CatalogCapabilityDescriptor = {
  provider: "codex", provider_label: "Codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2,
  label: "Pets v2", singular_label: "Pet v2", presentation: { icon: "pet-v2", kind_label: "Pets", format_label: "Codex pet v2" },
  capture: "directory_tree", apply: "automatic", targets: ["desktop.pets"],
};
const libraryScope = (key: string, capability: CatalogCapabilityDescriptor): CatalogScopeDto => {
  const profile = libraryProfile(capability.provider as "claude" | "codex");
  return {
    scope: {
      scope_id: `scope-${key}`, profile_id: profile.profile_id, provider: capability.provider, resource_kind: capability.resource_kind,
      format: capability.format, format_version: capability.format_version, revision: 1, created_at: now, updated_at: now,
    },
    profile, label: capability.label, available: true,
    provider_label: capability.provider_label, kind_label: capability.presentation.kind_label, format_label: capability.presentation.format_label,
  };
};
const SKILL_SCOPE = libraryScope("skills", SKILLS);
const coordinate = {
  provider: SKILLS.provider, resource_kind: SKILLS.resource_kind, format: SKILLS.format, format_version: SKILLS.format_version,
  provider_label: "Claude", kind_label: "Skills", format_label: "Agent Skill",
};

interface Skill { name: string; state: CatalogLinkState; local: boolean; remote: boolean; description: string | null; blocker?: string; executable?: boolean }
const LIBRARY_SKILLS: Skill[] = [
  { name: "tide-csv-clean", state: "local_changed", local: true, remote: true, executable: true, description: "清洗测站导出的潮位 CSV：去重、补齐缺失的时间点、统一到 UTC。" },
  { name: "station-report", state: "new", local: true, remote: false, description: "Writes the weekly station report from the alert log." },
  { name: "commit-message", state: "storage_changed", local: true, remote: true, description: "Writes a commit message from the staged diff, in the house style." },
  { name: "pdf-tables", state: "diverged", local: true, remote: true, description: "Pulls the tables out of a PDF into CSV files." },
  { name: "translate-polish", state: "local_missing", local: false, remote: true, description: "把英文文档翻译成中文并润色，保留术语表里的词。" },
  { name: "moon-phases", state: "local_missing", local: false, remote: true, description: "Adds the moon phase to each day of a tide table." },
  { name: "invoice-ocr", state: "storage_changed", local: true, remote: true, description: "Reads scanned invoices into the expense sheet." },
  { name: "release-notes", state: "in_sync", local: true, remote: true, description: "Drafts release notes from the commits since the last tag." },
  /* A skill's name is its folder's, and the backend's rule is ASCII: a folder
     named in Chinese is refused, in the backend's words. */
  { name: "潮汐笔记", state: "blocked", local: true, remote: false, description: null, blocker: "invalid skill name: '潮汐笔记'" },
];
const skillId = (name: string) => `skill:${name}`;
const digest = (seed: string) => sha(seed).repeat(2).slice(0, 64);
const EXEC_WARNING = { code: "executable_content", message: "scripts/clean.py is executable.", digest: digest("exec-warning") };
const repositoryOf = (skill: Skill) => `repo_${digest(skill.name).slice(0, 12)}`;
const addressOf = (skill: Skill) => `${skill.name === "commit-message" ? "maya" : LOGIN}/${skill.name}`;
const libraryRow = (skill: Skill): CatalogScopeRow => ({
  row_id: `row-${skill.name}`, display_name: skill.name, resource_id: skillId(skill.name), state: skill.state,
  local_present: skill.local, local_path: skill.local ? `${HOME}/.claude/skills/${skill.name}` : null,
  local_modified_at: skill.local ? now - 5 * 3600 : null, local_files: skill.local ? 4 : null, local_bytes: skill.local ? 12_400 : null,
  remote_present: skill.remote, remote_updated_at: skill.remote ? now - 2 * 86_400 : null, content_sha256: digest(skill.name),
  preview_png: null, description: skill.description, warnings: skill.executable ? [EXEC_WARNING] : [], blocker: skill.blocker ?? null,
});
const skillNamed = (name: string) => LIBRARY_SKILLS.find((skill) => skill.name === name)!;
const head = (skill: Skill | null, revision: number): LibraryHeadCoordinate => ({
  repository_id: skill ? repositoryOf(skill) : "repo_library_skills", revision, commit_id: digest(`commit${skill?.name ?? ""}${revision}`),
  manifest_sha256: digest(`manifest${revision}`), manifest_size: 2_048, storage_identity_sha256: digest(LIBRARY_STORAGE),
});

if (LIBRARY) {
  storageCatalogApi.listScopes = async () => [SKILL_SCOPE, libraryScope("plugins", PLUGINS), libraryScope("pets", PETS)];
  storageCatalogApi.listCapabilities = async () => [SKILLS, PLUGINS, PETS];
  storageCatalogApi.listProfiles = async () => [libraryProfile("claude"), libraryProfile("codex")];
  storageCatalogApi.listScopeResources = async (scopeId, storageId) => {
    await wait(120);
    const skills = scopeId === SKILL_SCOPE.scope.scope_id ? LIBRARY_SKILLS : [];
    const chosen = storageId ?? LIBRARY_STORAGE;
    const cloud = chosen === CLOUD_ID;
    const stored = Object.fromEntries(skills.filter((skill) => skill.remote).map((skill) => [skillId(skill.name), repositoryOf(skill)]));
    return {
      ...coordinate, scope: SKILL_SCOPE.scope, profile: SKILL_SCOPE.profile, storage_id: chosen, storage_name: cloud ? CLOUD.name : R2_STORAGE.name,
      link: { link_id: "link-skills", revision: 3 }, storage_revision: cloud ? null : 7, rows: skills.map(libraryRow), warnings: [], recovery: [],
      cloud_repositories: cloud ? stored : {}, cloud_connections: cloud ? stored : {}, cloud_status: null,
    };
  };
  storageCatalogApi.getPreview = async () => "";

  /* Push: two to send, one held for Pull, one refused, one already matching. */
  const pushRow = (name: string, relation: CatalogResourceRelation, extra: Partial<CatalogScopePushRow["row"]> = {}): CatalogScopePushRow => ({
    row: {
      ...coordinate, resource_id: skillId(name), display_name: name, relation, selectable: false, selected_by_default: false, held_for_pull: false,
      local_content_sha256: digest(name), storage_content_sha256: relation === "local_only" ? null : digest(`stored${name}`), warnings: [], ...extra,
    },
  });
  storageCatalogApi.preparePush = async (scopeId, storageId) => {
    await wait(200);
    const sending = ["tide-csv-clean", "station-report"].map(skillNamed);
    return {
      plan_id: "plan-push-skills", scope_id: scopeId, link_id: "link-skills", link_revision: 3, storage_id: storageId,
      repository_id: "repo_library_skills", storage_identity_sha256: digest(storageId),
      ...(LIBRARY_CLOUD
        ? {
          observed_items: { [skillId("tide-csv-clean")]: head(skillNamed("tide-csv-clean"), 4) },
          destinations: Object.fromEntries(sending.map((skill) => [skillId(skill.name), {
            repository_id: skill.remote ? repositoryOf(skill) : null, address: addressOf(skill), create: !skill.remote,
          }])),
        }
        : { observed_head: head(null, 7) }),
      rows: [
        pushRow("tide-csv-clean", "local_changed", { selectable: true, selected_by_default: true, warnings: [EXEC_WARNING] }),
        pushRow("station-report", "local_only", { selectable: true, selected_by_default: true }),
        pushRow("pdf-tables", "changed_both", { held_for_pull: true, detail: "Changed here and in storage since the last Pull. Pull it first." }),
        pushRow("潮汐笔记", "blocked", { blocked_reason: "invalid skill name: '潮汐笔记'" }),
        pushRow("release-notes", "matching"),
      ],
      warning_digests: [EXEC_WARNING.digest], created_at: now, expires_at: now + 900,
    };
  };
  storageCatalogApi.applyPush = async (planId, selected) => {
    await wait(300);
    const failing = "station-report";
    return {
      plan_id: planId, published: true,
      results: selected.map((resourceId) => {
        const name = resourceId.slice("skill:".length);
        return name === failing
          ? { resource_id: resourceId, display_name: name, state: "failed" as const, message: "R2 returned 503 Service Unavailable while writing the record.", revision: null }
          : { resource_id: resourceId, display_name: name, state: "published" as const, message: null, revision: LIBRARY_CLOUD ? 5 : 8 };
      }),
      stopped: "R2 returned 503 Service Unavailable while writing the record.", revision: LIBRARY_CLOUD ? null : 8,
      message: "Pushed 1 of 2 skills.",
    };
  };

  /* Pull: an install, an update, a both-sides change with a copy kept or an
     overwrite, a manual step, and a refusal; one target is claimed elsewhere. */
  const action = (name: string, kind: CatalogPullAction["kind"], destructive = false): CatalogPullAction => ({
    action_id: `action-${name}-${kind.kind}`, resource_id: skillId(name), kind, requires_destructive_approval: destructive,
    expected_target_state: kind.kind === "install" ? { state: "absent" } : { state: "sha256", sha256: digest(name) },
    incoming_content_sha256: digest(`stored${name}`),
  });
  const pullItem = (name: string, relation: CatalogResourceRelation, disposition: CatalogPullDisposition, actions: CatalogPullAction[], claims: string[] = []): CatalogScopePullItem => ({
    item: { ...coordinate, resource_id: skillId(name), display_name: name, relation, disposition, actions, default_action_id: actions[0]?.action_id ?? null },
    conflicting_claims: claims.map((storageId) => ({ storage_id: storageId, content_sha256: digest(`claim${name}`) })),
  });
  const keepCopy = action("pdf-tables", {
    kind: "preserve_as_copy_and_overwrite",
    copy: {
      copy_name: "pdf-tables-local", copy_resource_id: skillId("pdf-tables-local"), copy_target_path: `${HOME}/.claude/skills/pdf-tables-local`,
      transformed_content_sha256: digest("copy"), original_content_sha256: digest("pdf-tables"), storage_content_sha256: digest("storedpdf-tables"), reusable_witness: false,
    },
  });
  storageCatalogApi.preparePull = async (scopeId, storageId) => {
    await wait(200);
    const reading = ["translate-polish", "commit-message", "pdf-tables", "moon-phases", "invoice-ocr"].map(skillNamed);
    return {
      plan_id: "plan-pull-skills", scope_id: scopeId, link_id: "link-skills", link_revision: 3, storage_id: storageId,
      repository_id: "repo_library_skills", storage_identity_sha256: digest(storageId),
      ...(LIBRARY_CLOUD
        ? {
          item_heads: Object.fromEntries(reading.map((skill, index) => [skillId(skill.name), head(skill, 3 + index)])),
          sources: Object.fromEntries(reading.map((skill, index) => [skillId(skill.name), {
            repository_id: repositoryOf(skill), address: addressOf(skill), reads: skill.name === "commit-message" ? "public" as const : "draft" as const, revision: 3 + index,
          }])),
        }
        : { head: head(null, 7) }),
      items: [
        pullItem("translate-polish", "storage_only", { kind: "executable" }, [action("translate-polish", { kind: "install" })]),
        pullItem("commit-message", "storage_changed", { kind: "executable" }, [action("commit-message", { kind: "update" })]),
        pullItem("pdf-tables", "changed_both", { kind: "executable" }, [keepCopy, action("pdf-tables", { kind: "overwrite_with_storage" }, true)], [LIBRARY_CLOUD ? R2_ID : CLOUD_ID]),
        pullItem("moon-phases", "storage_only", { kind: "manual", message: "This skill needs Claude 2.4 or later. Update Claude, then pull again." }, []),
        pullItem("invoice-ocr", "storage_changed", { kind: "blocked", reason: "The stored copy failed its SHA-256 check." }, []),
        pullItem("release-notes", "matching", { kind: "already_matching" }, []),
        pullItem("tide-csv-clean", "local_changed", { kind: "kept_local" }, []),
      ],
      created_at: now, expires_at: now + 900,
    };
  };
  storageCatalogApi.applyPull = async (planId, choices) => {
    await wait(300);
    return {
      plan_id: planId, remote_accepted: false, stopped: null,
      results: Object.entries(choices).map(([resourceId, actionId]) => {
        const name = resourceId.slice("skill:".length);
        if (name === "commit-message") return { resource_id: resourceId, action_id: actionId, display_name: name, outcome: "failed" as const, message: "The local folder changed after this review was prepared. Review again." };
        if (name === "pdf-tables") return { resource_id: resourceId, action_id: actionId, display_name: name, outcome: "preserved_and_updated" as const, message: "Kept this Mac's copy as pdf-tables-local." };
        return { resource_id: resourceId, action_id: actionId, display_name: name, outcome: "installed" as const, message: null };
      }),
    };
  };

  /* Fork: the copy's name is the user's; a refusal is the backend's sentence. */
  storageCatalogApi.prepareFork = async (scopeId, resourceId, requestedName) => {
    await wait(150);
    if (screen === "library-fork-refused") throw new Error(`A skill named ${requestedName} already exists in ~/.claude/skills.`);
    return {
      plan_id: "plan-fork", scope_id: scopeId, source_coordinate: SKILLS, source_resource_id: resourceId, source_display_name: resourceId.slice(6),
      source_content_sha256: digest(resourceId), copy_name: requestedName, copy_resource_id: skillId(requestedName),
      copy_target_path: `${HOME}/.claude/skills/${requestedName}`, transformed_content_sha256: digest(requestedName), created_at: now, expires_at: now + 600,
    };
  };
  storageCatalogApi.applyFork = async (_scopeId, planId) => ({ resource_id: planId, display_name: "fork", outcome: "forked", message: null });
}

/* ——— the stand-in backend ——————————————————————————————————————————— */

/* What the backend says when a Cloud repository moved on mid-review: the Rust
   sentence, verbatim. Owner rule: backend errors stay English. */
const BACKEND_ERROR = "Mallard Cloud refused the push: the repository changed after this review was prepared. Review again to see the new snapshot.";

const QUIET = new Set(["plugin:event|listen", "plugin:event|unlisten", "plugin:event|emit", "plugin:path|resolve_directory"]);
const tidewatch = projects[0];

/* ——— setup flows: recent agent folders, a project draft, a storage draft ——— */

const probeOf = (path: string): ProjectFolderProbe => {
  const project = projects.find((candidate) => candidate.root === path);
  return { kind: "folder", canonical_path: path, is_git_repository: !path.includes("草稿"), existing_draft_id: null, existing_project_ids: project ? [project.id] : [] } as ProjectFolderProbe;
};
const FOLDERS: AgentFolder[] = [
  { path: `${HOME}/code/harbor`, providers: ["codex", "claude"], last_active: now - 8 * 60 },
  { path: `${HOME}/notes/草稿箱`, providers: ["claude"], last_active: now - 3 * 3600 },
  { path: `${HOME}/code/tidewatch`, providers: ["codex"], last_active: now - 26 * 3600 },
].map((folder) => ({ ...folder, probe: probeOf(folder.path) })) as AgentFolder[];

let projectDraft: ProjectSetupDraft | null = null;
const newProjectDraft = (root: string): ProjectSetupDraft => ({
  schema: 1, draft_id: "draft-project-1", local_project_id: "project-harbor", new_bundle_id: "8".repeat(32), project_root: root,
  canonical_project_root: root, display_name: root.split("/").pop() ?? "project", repository_fingerprint: "c3".repeat(32),
  profiles: { codex: { kind: "existing", profile_id: "p-codex" }, claude: { kind: "existing", profile_id: "p-claude" } },
  storage: null, repository: { kind: "new" }, revision: 1, created_at: now, updated_at: now,
} as ProjectSetupDraft);
const inspection = (draft: ProjectSetupDraft) => ({
  draft,
  sections: [{ section: "project", state: "ready" }, { section: "profiles", state: "ready" }, { section: "storage", state: "ready" }, { section: "repository", state: "ready" }],
  can_finalize: true,
});

const storageDrafts = new Map<string, StorageSetupDraft>();
function newStorageDraft(provider: string): StorageSetupDraft {
  const descriptor = byKey(provider);
  const draft: StorageSetupDraft = {
    draft_id: `draft-${provider}`, storage_id: `storage-${provider}-new`, name: descriptor.default_name ?? descriptor.display_name,
    provider, config_version: 1, provider_label: descriptor.display_name, available: true, settings: {},
    secrets: descriptor.secrets.map((secret) => ({ secret_id: secret.secret_id, present: false })),
    edits_existing: false, revision: 1, created_at: now, updated_at: now,
  };
  storageDrafts.set(draft.draft_id, draft);
  return draft;
}

/* ——— the setup lanes: each answers, or holds at the step the screen shows ——— */

const hold = () => new Promise<never>(() => {});
const LINK = {
  cloud: `${ORIGIN}/v1/app/sign-in?state=Q2hc7v`,
  r2: "https://dash.cloudflare.com/oauth2/auth?response_type=code&client_id=54d11594-84e4-41aa-b438-e81b8fa78ee7",
  dropbox: "https://www.dropbox.com/oauth2/authorize?client_id=mallard&response_type=code",
  feishu: "https://accounts.feishu.cn/open-apis/authen/v1/authorize?client_id=cli_a1b2c3d4e5&response_type=code",
};
/* Account and bucket names are the user's own; they stay as Cloudflare has them. */
const R2_ACCOUNTS = [{ id: "023e105f4ecef8ad9ca31a8372d0c353", name: "octocat" }, { id: "7f3c0a9e5b1d4c2f8e6a7b9c0d1e2f3a", name: "海港实验室" }];
projectSyncApi.getR2SetupAccounts = async (_draft, _revision, _login, onProgress) => {
  onProgress?.({ status: "wrangler_ready", version: "4.42.0", path: "/opt/homebrew/bin/wrangler" });
  if (screen === "storage-r2-signin") {
    onProgress?.({ status: "authorization_link", stage: "login", url: LINK.r2 });
    return hold();
  }
  await wait(120);
  return R2_ACCOUNTS;
};
projectSyncApi.proposeR2BucketName = async () => "mallard-heron-4821";
projectSyncApi.setupR2Storage = async (_draft, _revision, _account, _jurisdiction, _bucket, onProgress) => {
  onProgress?.({ status: "bucket_ready", bucket: "mallard-heron-4821" });
  onProgress?.({ status: "authorization_link", stage: "bucket", url: LINK.r2 });
  return hold();
};
projectSyncApi.discoverR2Storage = async () => {
  await wait(200);
  return {
    scan_id: "scan-1", scope: "octocat · Default and EU", status: "complete", issues: [],
    candidates: [
      { candidate_id: "c1", name: "mallard-otter", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Jurisdiction", value: "Default" }, { label: "Created", value: "Sep 12, 2026" }], cautions: [], connected_storage_id: "storage-r2" },
      { candidate_id: "c2", name: "team-sessions", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Jurisdiction", value: "EU" }, { label: "Created", value: "Aug 30, 2026" }], cautions: [], connected_storage_id: null },
      { candidate_id: "c3", name: "website-assets", marker: "missing", marker_detail: null, facts: [{ label: "Jurisdiction", value: "Default" }, { label: "Created", value: "Mar 3, 2026" }], cautions: [], connected_storage_id: null },
    ],
  };
};
projectSyncApi.prepareExistingR2Storage = async () => hold();
projectSyncApi.cancelR2StorageSetup = async () => {};
projectSyncApi.signInManagedCloudStorage = async (_draft, _revision, _mode, onProgress) => {
  onProgress({ status: "authorization_link", url: LINK.cloud, expires_in: 180 });
  return hold();
};
projectSyncApi.cancelManagedCloudSetup = async () => {};
projectSyncApi.authorizeDropboxSetup = async (_draft, _revision, _mode, _switch, onProgress) => {
  onProgress({ status: "authorization_link", url: LINK.dropbox, expires_in: 180 } as never);
  if (screen === "storage-dropbox-signin") return hold();
  await wait(120);
  return { session_id: "dbx-session-1", account: { id: "dbid:octocat", name: "Octo Cat", email: "octocat@example.com" } };
};
projectSyncApi.proposeDropboxFolderName = async () => "Mallard otter";
projectSyncApi.prepareDropboxStorage = async () => hold();
projectSyncApi.cancelDropboxSetup = async () => {};
projectSyncApi.authorizeFeishuSetup = async (_draft, _revision, _mode, onProgress) => {
  onProgress({ status: "authorization_link", url: LINK.feishu, expires_in: 180 } as never);
  if (screen === "storage-feishu-signin") return hold();
  await wait(120);
  return { session_id: "feishu-session-1", account: { id: "ou_octocat", name: "鸭鸭" } };
};
projectSyncApi.proposeFeishuFolderName = async () => "Mallard fox";
projectSyncApi.prepareFeishuStorage = async () => hold();
projectSyncApi.cancelFeishuSetup = async () => {};

/* A delete Codex refuses: which confirms each screen answers Yes to before
   the dialog it shows (sessionDeletion.ts). */
const APPROVED_DIALOGS: Record<string, string[]> = {
  "delete-session-in-use": ["Delete local session?"],
  "delete-close-codex": ["Delete local session?"],
  "delete-codex-closed": ["Delete local session?", "Close Codex before deleting"],
  "delete-all-stopped": ["Delete all local sessions?"],
};

/* ——— native dialogs: macOS draws them outside the page, so the specimen
   draws what the app asked for (title, message, buttons) as an alert. ——— */
function drawDialog(args: Record<string, unknown>) {
  const buttons = args.buttons;
  const labels = typeof buttons === "string"
    ? (buttons === "YesNo" ? ["No", "Yes"] : buttons === "Ok" ? ["Ok"] : ["Cancel", "Ok"])
    : buttons && typeof buttons === "object"
      ? Object.values(buttons as Record<string, unknown>).flatMap((value) => (Array.isArray(value) ? value.map(String).reverse() : [String(value)]))
      : ["Ok"];
  const overlay = document.createElement("div");
  overlay.className = "zh-native-dialog";
  overlay.innerHTML = `<div class="zh-native-alert" role="alertdialog">
    <div class="zh-native-icon">🦆</div>
    <strong></strong><p></p>
    <div class="zh-native-buttons">${labels.map((_label, index) => `<button type="button" class="${index === labels.length - 1 ? "default" : ""}"></button>`).join("")}</div>
  </div>`;
  overlay.querySelector("strong")!.textContent = String(args.title ?? "Mallard");
  overlay.querySelector("p")!.textContent = String(args.message ?? "");
  overlay.querySelectorAll("button").forEach((button, index) => { button.textContent = labels[index]; });
  document.body.append(overlay);
}
const DIALOG_CSS = `
.zh-native-dialog { position: fixed; inset: 0; z-index: 9999; display: grid; place-items: start center; padding-top: 90px; background: rgba(0,0,0,.12); }
.zh-native-alert { width: 290px; padding: 18px 16px 14px; border-radius: 12px; background: #ececec; color: #1d1d1f; text-align: center;
  box-shadow: 0 10px 40px rgba(0,0,0,.35), 0 0 0 .5px rgba(0,0,0,.2); font: 13px/1.4 -apple-system, "PingFang SC", system-ui, sans-serif; }
.zh-native-icon { font-size: 40px; line-height: 1; margin-bottom: 8px; }
.zh-native-alert strong { display: block; margin-bottom: 6px; font-size: 13px; }
.zh-native-alert p { margin: 0 0 14px; font-size: 11px; white-space: pre-wrap; }
.zh-native-buttons { display: grid; gap: 6px; }
.zh-native-buttons button { padding: 4px 0; border: 0; border-radius: 6px; background: #fff; color: #1d1d1f; font: 13px -apple-system, "PingFang SC", system-ui; box-shadow: 0 0 0 .5px rgba(0,0,0,.15); }
.zh-native-buttons button.default { background: #0a84ff; color: #fff; }
`;
{
  const style = document.createElement("style");
  style.textContent = DIALOG_CSS;
  document.head.append(style);
}

(window as unknown as { isTauri: boolean }).isTauri = true;
mockWindows("main");
mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "plugin:app|version": return "0.1.7";
    case "plugin:dialog|open": return "/Volumes/Backup/mallard-storage";
    case "plugin:dialog|message": {
      /* A blocked delete: the confirms before the asked-for dialog are approved. */
      const buttons = args.buttons as { OkCancelCustom?: [string, string] } | string | undefined;
      if ((APPROVED_DIALOGS[screen] ?? []).includes(String(args.title))) {
        return typeof buttons === "object" && buttons.OkCancelCustom ? buttons.OkCancelCustom[0] : "Ok";
      }
      drawDialog(args);
      return hold();
    }
    case "delete_local_project_session": return {
      blocked_by: screen === "delete-session-in-use" ? "codex_session_in_use" : "codex_app_running", provider_error: null, metadata_error: null,
    };
    case "delete_local_project_sessions": return {
      blocked_by: "codex_app_running", sessions_deleted: 2, sessions_failed: 0, provider_errors: [], metadata_reset: null, metadata_error: null,
    };
    case "execute_provider_maintenance_action": {
      await wait(150);
      return { provider: "codex", action_id: "codex.app.close", ready: true, summary: "Codex is closed.", facts: {} };
    }
    case "get_host_identity_status":
      if (screen === "startup") return hold();
      return { host_id: "host-1", revision: 1, created_at: now, updated_at: now };
    case "get_project_sync_config": return config();
    case "list_local_projects": return projects.map((project) => ({
      ...registration(project), project_root: project.root, canonical_project_root: project.root, profile_ids: PROFILE_IDS,
      providers: ["codex", "claude"], linked_storage_ids: [project.storageId], is_git_repository: project.git,
    }));
    case "get_project": { const project = byId(String(args.localProjectId)); return project ? detailOf(project) : null; }
    case "list_project_bindings": return projects.map(binding);
    case "list_project_repository_kinds": return Object.fromEntries(projects.map((project) => [project.id, project.git]));
    case "list_setup_drafts": return { drafts: [] };
    case "list_storage_setup_drafts": return { drafts: [], warnings: [] };
    case "list_storage_providers": return PROVIDERS;
    case "list_catalog_scopes":
    case "list_catalog_capabilities":
    case "list_catalog_resource_profiles":
    case "list_catalog_cloud_shared": return [];
    case "list_remote_bundle_snapshots": return [
      { bundle_id: "7".repeat(32), display_name: "tidewatch", kind: "project", revision: 7, updated_at: now - 3600, resource_count: 3, providers: ["codex", "claude"], storage_id: String(args.storageId ?? CLOUD_ID) },
      { bundle_id: "6".repeat(32), display_name: "portal", kind: "project", revision: 4, updated_at: now - 30 * 3600, resource_count: 1, providers: ["codex"], storage_id: String(args.storageId ?? CLOUD_ID) },
      { bundle_id: "9".repeat(32), display_name: "海港", kind: "project", revision: 0, registration_only: true, storage_id: String(args.storageId ?? CLOUD_ID) },
    ];
    case "list_catalog_storage_repositories": return [];
    case "list_agent_providers": return AGENTS;
    case "list_provider_profiles": return PROFILES;
    case "list_agent_folders": return { folders: FOLDERS };
    case "probe_project_folder": return probeOf(String(args.projectRoot));
    case "create_setup_draft": projectDraft = newProjectDraft(String(args.projectRoot)); return { draft: projectDraft, resumed: false };
    case "get_setup_draft": return projectDraft;
    case "update_setup_draft": { const next = args.draft as ProjectSetupDraft; projectDraft = { ...next, revision: next.revision + 1 }; return projectDraft; }
    case "inspect_setup_draft": return projectDraft ? inspection(projectDraft) : null;
    case "discard_setup_draft": projectDraft = null; return true;
    case "create_storage_setup_draft": return newStorageDraft(String(args.provider));
    case "get_storage_setup_draft": return storageDrafts.get(String(args.draftId)) ?? null;
    case "update_storage_setup_draft": {
      const request = args.request as { draft_id: string; name: string; settings: Record<string, unknown> };
      const draft = storageDrafts.get(request.draft_id)!;
      const next = { ...draft, name: request.name, settings: { ...draft.settings, ...request.settings }, revision: draft.revision + 1 } as StorageSetupDraft;
      storageDrafts.set(draft.draft_id, next);
      return next;
    }
    case "discard_storage_setup_draft": return storageDrafts.delete(String(args.draftId));
    case "test_storage": return { storage_id: String(args.storageId), verified: true };
    case "test_storage_setup_draft": { await wait(200); const draft = storageDrafts.get(String(args.draftId)); return { draft_id: String(args.draftId), draft_revision: draft?.revision ?? 1, verified: true }; }
    case "list_managed_cloud_repositories": return [];
    case "list_managed_cloud_starred":
    case "list_managed_cloud_public":
    case "list_catalog_cloud_starred":
    case "list_catalog_cloud_public": return { rows: [], next_cursor: null };
    case "take_app_link":
    case "recover_startup_state":
    case "hold_project_git_watch":
    case "release_project_git_watch":
    case "watch_project_git":
    case "unwatch_project_git": return null;
    case "prepare_app_update": return { current_version: "0.1.7", ready_version: screen === "update" ? "0.1.8" : null };
    case "get_app_settings": return {
      push_codex_children: true, push_claude_children: true, credential_redaction_enabled: false, update_check_enabled: true, schema: 1,
      codex_cli_path: null, claude_cli_path: null,
      conversation_file_limit_bytes: 64 * MB, timeline_preview_limit_bytes: 2 * MB, comparison_workers: 4, cloud_transfer_concurrency: 4,
    };
    case "inspect_provider_cli": {
      const provider = String(args.provider);
      const path = `/opt/homebrew/bin/${provider}`;
      return { provider, selected_path: null, launcher: path, executable: path, source: "shell_path", error: null };
    }
    case "get_activity_log_stats": return {
      total_bytes: 3.2 * MB, file_count: 6, oldest_ts: (now - 12 * 86_400) * 1000, newest_ts: now * 1000,
      policy: { schema: 1, retention_days: 30, max_total_bytes: 100 * MB },
    };
    /* Log lines are backend text: they stay English (owner rule). */
    case "query_activity_logs": return { entries: [
      { id: "l4", level: "success", type: "push", event: "push.done", message: "Pushed 3 sessions to Mallard Cloud octocat (revision 1).", ts: (now - 60) * 1000 },
      { id: "l3", level: "info", type: "push", event: "push.capture", message: "Captured 3 sessions, 4.0 MB, credential redaction off.", ts: (now - 75) * 1000 },
      { id: "l2", level: "warning", type: "history", event: "history.compare", message: "Skipped 1 transcript above the 64 MB thread size limit.", ts: (now - 3600) * 1000 },
      { id: "l1", level: "info", type: "configuration", event: "config.storage", message: "Linked tidewatch to Mallard Cloud octocat.", ts: (now - 7200) * 1000 },
    ], next_cursor: null };
    case "get_local_state_summary": return { root_path: `${HOME}/.mallard`, file_count: 41, total_bytes: 18.6 * MB, backups_bytes: 4.1 * MB };
    case "get_push_destination_status": return { state: "ready", revision: 0 };
    case "prepare_push_review": {
      await wait(150);
      const sessions = sessionsOf(tidewatch);
      return {
        scope: { kind: "project" },
        credential_redaction_enabled: false,
        project: detailOf(tidewatch, false),
        inventory: {
          project: tidewatch.name, bundle_id: tidewatch.bundle,
          resources: sessions.map(resource), recipe: { schema_version: 1, revision: 1, entries: {} },
        },
        destination_status: { state: "ready", revision: 0 },
        thread_comparison: {
          project_id: tidewatch.id, storage_id: CLOUD_ID, storage_name: CLOUD.name, revision: 0, base_revision: 0, compared_at: now,
          entries: sessions.map((session) => entry(tidewatch, session)),
          counts: { synced: 0, local: sessions.length, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
          scopes: [], provider_errors: {}, warnings: [],
          capture_budget: { limit_bytes: 512 * MB, reserved_bytes: 0 },
          remote_continuity: { state: "in_sync", revision: 0 },
        },
        thread_comparison_error: null,
        project_files: null, project_files_error: null,
        git_history: null, git_history_review_token: null, git_history_changed: false, git_history_outgoing: null,
        prepared_at: now,
      };
    }
    case "push_bundle": {
      await wait(200);
      throw BACKEND_ERROR;
    }
    default:
      if (!QUIET.has(command)) console.warn("unanswered", command, args);
      return null;
  }
}, { shouldMockEvents: true });

/* ——— drive the shell to the screen asked for ———————————————————————— */

const settle = (ms = 60) => wait(ms);
async function until<T>(find: () => T | null | undefined, timeout = 4000): Promise<T> {
  const started = Date.now();
  for (;;) {
    const found = find();
    if (found) return found;
    if (Date.now() - started > timeout) throw new Error("harness: element never appeared");
    await settle();
  }
}
/* Buttons are found by their English label before the shim rewrites it, or by
   the label the shim remembers it had. */
const englishOf = (element: Element) => element.getAttribute("data-i18n-en") ?? element.textContent?.trim() ?? "";
const labelOf = (element: Element) => element.getAttribute("data-i18n-en-aria-label") ?? element.getAttribute("aria-label") ?? "";
const buttonNamed = (name: string, scope: ParentNode = document) =>
  [...scope.querySelectorAll("button, [role=menuitem], a")].find((element) => englishOf(element) === name || labelOf(element) === name) as HTMLElement | undefined;

/* Which provider each storage screen opens, by its prefix. */
const STORAGE_TYPE: Array<[string, string]> = [
  ["storage-r2", "Set up Cloudflare R2 storage"],
  ["storage-cloud", "Set up Mallard Cloud storage"],
  ["storage-local", "Set up Local folder storage"],
  ["storage-dropbox", "Set up Dropbox storage"],
];

/* An element by the English it had before the shim, its own text, or its label. */
function byEnglish(text: string): HTMLElement | undefined {
  const hit = [...document.querySelectorAll<HTMLElement>("body *")].find((element) =>
    element.getAttribute("data-i18n-en") === text
    || element.getAttribute("data-i18n-en-aria-label") === text
    || element.getAttribute("aria-label") === text
    || ((element.children.length === 0 || element.matches("button, [role=radio], a")) && element.textContent?.trim() === text));
  return (hit?.closest("button, [role=radio], a") as HTMLElement | null) ?? hit;
}
async function press(text: string, pause = 300) {
  (await until(() => byEnglish(text))).click();
  await settle(pause);
}
/* A React-controlled select takes a value through the native setter. */
function choose(select: HTMLSelectElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, value);
  select.dispatchEvent(new Event("change", { bubbles: true }));
}
async function chooseAccount() {
  const select = await until(() => [...document.querySelectorAll("select")].find((candidate) => [...candidate.options].some((option) => option.value === R2_ACCOUNTS[0].id)));
  choose(select, R2_ACCOUNTS[0].id);
  await settle(120);
  await press("Continue", 400);
}

async function openProject() {
  const projectRow = await until(() => [...document.querySelectorAll(".sync-sidebar button, nav button, [role=treeitem], [role=option]")]
    .find((element) => (element.textContent ?? "").includes("tidewatch")) as HTMLElement | undefined);
  projectRow.click();
  await settle(400);
}

async function drive() {
  if (screen === "sessions" || screen === "push" || screen === "error" || screen === "settings") await openProject();
  if (screen.startsWith("cloud-")) {
    const portal = await until(() => [...document.querySelectorAll(".sync-sidebar button, nav button, [role=treeitem], [role=option]")]
      .find((element) => (element.textContent ?? "").includes("portal")) as HTMLElement | undefined);
    portal.click();
    await settle(600);
    const chip = await until(() => [...document.querySelectorAll<HTMLButtonElement>(".sync-path-cloud-status")]
      .find((element) => !element.disabled && !element.classList.contains("checking")));
    chip.click();
    await settle(400);
    if (screen === "cloud-publish") await press("Publish version 3…", 600);
  }
  if (screen === "push" || screen === "error") {
    (await until(() => [...document.querySelectorAll(".sync-path-sync")].find((element) => labelOf(element).startsWith("Push")) as HTMLElement | undefined)).click();
    await settle(500);
    if (screen === "error") {
      const commit = await until(() => [...document.querySelectorAll("button")]
        .find((element) => englishOf(element) === "Push" && !element.classList.contains("sync-path-sync")) as HTMLElement | undefined);
      commit.click();
      await settle(500);
    }
  }
  if (screen === "settings") {
    (await until(() => buttonNamed("App actions"))).click();
    await settle(120);
    (await until(() => buttonNamed("Settings"))).click();
    await settle(400);
    addLanguageRow();
  }
  if (screen.startsWith("storage")) {
    (await until(() => buttonNamed("Storages"))).click();
    await settle(300);
    if (screen !== "storages") {
      (await until(() => buttonNamed("Add storage…"))).click();
      await settle(300);
      const type = STORAGE_TYPE.find(([prefix]) => screen.startsWith(prefix));
      if (type) await press(type[1], 400);
    }
    if (screen.startsWith("storage-r2-")) {
      const existing = screen === "storage-r2-existing";
      await press(existing ? "Use existing" : "Create new");
      if (screen !== "storage-r2-create") {
        await press(existing ? "Find in my account" : "Set up with Wrangler", 500);
        if (screen === "storage-r2-bucket" || screen === "storage-r2-authorize" || existing) await chooseAccount();
        if (screen === "storage-r2-authorize") await press("Create & authorize", 500);
      }
    }
    if (screen === "storage-cloud-signin") await press("Sign in with GitHub", 500);
    if (screen.startsWith("storage-dropbox-")) {
      await press("Create new");
      await press("Connect Dropbox", 600);
      if (screen === "storage-dropbox-folder") await press("Continue", 500);
    }
    if (screen === "storage-local-ready") {
      await press("Browse…", 400);
      await press("Test connection", 500);
    }
  }
  if (screen.startsWith("project-")) {
    (await until(() => buttonNamed("Add project"))).click();
    await settle(400);
    if (screen === "project-setup") {
      (await until(() => [...document.querySelectorAll("[role=radio]")].find((element) => labelOf(element).startsWith("harbor")) as HTMLElement | undefined)).click();
      await settle(120);
      (await until(() => buttonNamed("Use this folder"))).click();
      await settle(500);
    }
  }
  if (screen === "storage-page") {
    (await until(() => buttonNamed("Storages"))).click();
    await settle(300);
    await press("Open Mallard Cloud octocat", 700);
  }
  const projectMenu = ["project-menu", "project-rename", "project-binding"];
  if (projectMenu.includes(screen) || ["link-storage", "session-info", "new-session-menu", "confirm-delete-session", "confirm-unlink"].includes(screen)) await openProject();
  if (projectMenu.includes(screen)) {
    await press("tidewatch. Project menu", 300);
    if (screen === "project-rename") await press("Rename project…", 400);
    if (screen === "project-binding") await press("Agent profile…", 500);
  }
  if (screen === "link-storage" || screen === "confirm-unlink") {
    await press("Mallard Cloud octocat. Storage menu", 300);
    await press(screen === "link-storage" ? "Link storage…" : "Unlink from tidewatch…", 500);
  }
  if (screen === "session-info") {
    /* Headless capture focuses the page, and the panel closes on focus that
       lands outside it; hold it open for the still. */
    document.addEventListener("focusin", (event) => event.stopImmediatePropagation(), true);
    await press("Show session info", 400);
  }
  if (screen === "new-session-menu") await press("New session", 400);
  if (screen === "confirm-delete-session") await press("Delete local session", 500);
  if (screen.startsWith("delete-")) {
    await openProject();
    if (screen === "delete-all-stopped") {
      await press("tidewatch. Project menu", 300);
      await press("Delete all local sessions…", 500);
    } else {
      await press("Delete local session", 500);
    }
    await until(() => document.querySelector(".zh-native-dialog"));
    await settle(200);
  }
  if (screen === "confirm-remove-project") await press("Remove portal from Mallard", 500);
  if (screen === "update") {
    await settle(300);
    window.dispatchEvent(new Event("focus"));
    await settle(800);
  }
  if (screen.startsWith("session-detail")) {
    await openProject();
    const title = screen === "session-detail-en" ? "Fix the chart's time zone" : "添加低潮位提醒";
    (await until(() => [...document.querySelectorAll<HTMLButtonElement>(".sync-history-session-title-button")]
      .find((element) => (element.textContent ?? "").includes(title)))).click();
    await until(() => (document.querySelectorAll(".session-detail-event").length >= 3 ? true : null));
    await settle(300);
    /* Every kind on: steps, tool calls, and agents join the default view. */
    for (const toggle of document.querySelectorAll<HTMLButtonElement>(".session-detail-filter-toggle[aria-pressed=false]")) {
      toggle.click();
      await settle(250);
    }
    await until(() => document.querySelector(".session-detail-tool-call"));
    if (screen === "session-detail-tool") {
      (await until(() => document.querySelector<HTMLElement>(".session-detail-tool-call.status-failed > summary"))).click();
      await until(() => document.querySelector(".session-detail-tool-call.status-failed .session-detail-payload-body"));
      await settle(300);
    }
    if (screen === "session-detail-agent" || screen === "session-detail-en" || screen === "session-detail-child") {
      (await until(() => document.querySelector<HTMLElement>(".session-detail-agent > summary"))).click();
      await settle(300);
    }
    if (screen === "session-detail-child") {
      (await until(() => document.querySelector<HTMLButtonElement>(".session-detail-agent-open:not(:disabled)"))).click();
      await until(() => ([...document.querySelectorAll(".session-detail-event")].some((element) => element.textContent?.includes("文案没问题")) ? true : null));
      await settle(300);
    }
    await until(() => document.querySelector(".session-detail-event"));
  }
  if (LIBRARY) {
    (await until(() => buttonNamed("Catalog"))).click();
    await settle(300);
    (await until(() => [...document.querySelectorAll<HTMLButtonElement>(".sidebar-profile-main:not(:disabled)")]
      .find((element) => (element.getAttribute("data-i18n-en-title") ?? element.title).endsWith(`${HOME}/.claude`)))).click();
    const syncButtons = () => [...document.querySelectorAll<HTMLButtonElement>(".sync-path-action-group .sync-path-sync")];
    await until(() => (syncButtons().length === 2 && syncButtons().every((button) => !button.disabled) ? true : null));
    await settle(300);
    const [pull, push] = syncButtons();
    if (screen.includes("push")) {
      push.click();
      await until(() => document.getElementById("catalog-push-title"));
      await settle(300);
      if (screen === "library-pushed") {
        document.querySelectorAll<HTMLInputElement>(".notice-ack input[type=checkbox]").forEach((box) => { if (!box.checked) box.click(); });
        await settle(200);
        (await until(() => document.querySelector<HTMLButtonElement>(".sync-sync-review-footer .btn-primary:not(:disabled)"))).click();
        await until(() => document.querySelector(".sync-outcome-notice"));
        await settle(300);
      }
    }
    if (screen.includes("pull")) {
      pull.click();
      await until(() => document.getElementById("catalog-pull-title"));
      await settle(300);
      if (screen === "library-pull-overwrite") {
        (await until(() => document.querySelector<HTMLButtonElement>(".catalog-row-action.is-choice"))).click();
        await settle(300);
        (await until(() => document.querySelector<HTMLButtonElement>("[role=menu] .sync-path-menu-item.danger"))).click();
        await until(() => document.querySelector(".notice-ack"));
        await settle(300);
      }
      if (screen === "library-pulled") {
        (await until(() => document.querySelector<HTMLButtonElement>(".sync-pull-apply-button:not(:disabled)"))).click();
        await until(() => document.querySelector(".sync-outcome-notice"));
        await settle(300);
      }
    }
    if (screen.startsWith("library-fork")) {
      (await until(() => [...document.querySelectorAll<HTMLButtonElement>("button")]
        .find((element) => labelOf(element).startsWith("Fork tide-csv-clean")))).click();
      const drawer = await until(() => document.querySelector<HTMLFormElement>(".catalog-fork-drawer"));
      await settle(300);
      if (screen === "library-fork-refused") {
        drawer.querySelector<HTMLButtonElement>("button[type=submit]")!.click();
        await until(() => drawer.querySelector(".catalog-fork-note.danger"));
        await settle(200);
      }
    }
  }
  if (screen === "logs") {
    (await until(() => buttonNamed("Logs"))).click();
    await settle(400);
  }
  if (screen === "library") {
    (await until(() => buttonNamed("Catalog"))).click();
    await settle(400);
  }
  document.documentElement.dataset.ready = "1";
}

/* ——— direction A: a Language row under Theme, drawn with SettingsRow's markup ———
   Specimen-only: appended beside React's rows (never in place of them). The
   simulated Mac prefers Chinese, so System shows Chinese. Picking reloads the
   frame in that language: the app applies the choice at once. */
function addLanguageRow() {
  const card = document.getElementById("settings-appearance")?.closest("section")?.querySelector(".sync-settings-card");
  if (!card || card.querySelector("[data-language-row]")) return;
  const row = document.createElement("div");
  row.className = "sync-settings-row";
  row.dataset.languageRow = "";
  row.dataset.i18nSkip = "";
  const label = zh ? "语言" : "Language";
  const hint = zh ? "“跟随系统”使用 macOS 的语言。" : "System uses your macOS language.";
  const options: Array<[typeof pref, string, string]> = [["system", zh ? "跟随系统" : "System", zh ? "zh-CN" : "en"], ["en", "English", "en"], ["zh", "简体中文", "zh-CN"]];
  row.innerHTML = `<div class="sync-settings-copy"><strong>${label}</strong><p>${hint}</p></div>
    <div class="sync-settings-control"><div class="sync-settings-segment" role="radiogroup" aria-label="${label}">${options.map(([value, text, lang]) =>
      `<button type="button" role="radio" lang="${lang}" data-pref="${value}" aria-checked="${pref === value}"${pref === value ? ' class="active"' : ""}>${text}</button>`).join("")}</div></div>`;
  row.querySelectorAll<HTMLButtonElement>("button[data-pref]").forEach((button) => button.addEventListener("click", () => {
    const next = new URLSearchParams(window.location.search);
    const chosen = button.dataset.pref!;
    next.set("pref", chosen);
    next.set("lang", chosen === "en" ? "en" : "zh");
    window.location.search = `?${next}`;
  }));
  card.append(row);
}

if (zh) {
  document.documentElement.lang = "zh-CN";
  installChineseShim(document.getElementById("root")!);
}
if (cjkFont) {
  const style = document.createElement("style");
  style.textContent = CJK_FONT_CSS;
  document.head.append(style);
}

createRoot(document.getElementById("root")!).render(<App />);
void drive().catch((error) => { document.documentElement.dataset.ready = `failed: ${String(error)}`; });
