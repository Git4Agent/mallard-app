import { StrictMode, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon, { type IconName } from "../src/components/Icons";
import ManagedCloudChip from "../src/components/managed-cloud/ManagedCloudChip";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import Notice, { NoticeZone, type NoticeItem } from "../src/components/notice/Notice";
import NewProjectFolderStep from "../src/components/project-sync/NewProjectFolderStep";
import ProjectSidebar from "../src/components/project-sync/ProjectSidebar";
import RestorePlanView from "../src/components/project-sync/RestorePlanView";
import SessionDetailPage from "../src/components/project-sync/session-detail/SessionDetailPage";
import { projectSyncApi, type ActivityRefreshRequest } from "../src/components/project-sync/api";
import AddCatalogScopeWorkspace from "../src/components/storage-catalog/AddCatalogScopeWorkspace";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import { catalogPresentation } from "../src/components/storage-catalog/model";
import type {
  CatalogCapabilityDescriptor,
  CatalogCloudRepository,
  CatalogProfile,
  CatalogScopeView,
} from "../src/components/storage-catalog/types";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { PathBarBelow, type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "../src/components/sync-review/SyncReviewSelectionBar";
import type {
  ConversationReviewRow,
  LocalProjectSummary,
  ManagedCloudRepository,
  ManagedCloudRepositoryResolution,
  ProjectActivity,
  ProjectActivityRefreshEvent,
  ProjectBinding,
  ProjectSessionDetail,
  ProjectSessionSummary,
  ProviderDescriptor,
  ProviderProfileSummary,
  PullReview,
  SessionTimelineEvent,
  StorageSummary,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../src/types";
import "../src/App.css";

/**
 * Specimen: Pull this session — the link from the website's session page,
 * in the app (docs/PLAN_DEEP_LINK_SESSION_PULL.md).
 *
 * Every frame is the shipped shell: the sidebar, the path bar, the connect
 * dialog's frame as the sheet, the Pull review (`RestorePlanView`) on a
 * one-row plan, the session page (`SessionDetailPage`) on a stand-in
 * backend, and the shipped New project and Add Catalog resource pages. What
 * the plan adds is dotted: the sheet's second line, the review's header, the
 * pending-files line on a non-Git project's session page.
 *
 * `npm run dev`, then /mockups/session-pull-mockup.html
 * (`?scene=session-new|session-synced|session-newer|session-diverged|nongit|no-project|no-project-folder|pet-saved|skill-new|waiting|no-account|bad-link`,
 * `?dir=b|a` (sheet first, or straight to the review), `?stage=before|arrival|review|landed`,
 * `?theme=dark`, `?shot=on`).
 */

/* ——— data ————————————————————————————————————————————————————————— */

const HOME = "/Users/you";
const HOST = "mallard-ai.com";
const ORIGIN = `https://${HOST}`;
const now = Math.floor(Date.now() / 1000);
const DAY = 86_400;
const KB = 1024;
const MB = 1024 * KB;
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));
const noop = () => {};

const cloud = (id: string, name: string, login: string): StorageSummary => ({
  storage_id: id,
  name,
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: login, is_path: false },
    { field_id: "service_origin", label: "Service", value: ORIGIN, is_path: false },
  ],
  settings: { service_origin: ORIGIN, account_id: `user_${login}`, account_login: login },
  secrets: [{ secret_id: "account_token", present: true }],
});
const PERSONAL = cloud("storage-personal", "Mallard Cloud", "Rika321");

const project = (name: string, rel: string, git: boolean, provider: string) => ({
  local_project_id: `project-${name}`,
  bundle_id: `bundle-${name}`,
  display_name: name,
  revision: 1,
  project_root: `${HOME}/${rel}`,
  canonical_project_root: `${HOME}/${rel}`,
  is_git_repository: git,
  profile_ids: { [provider]: `p-${provider}` },
}) as LocalProjectSummary;
const HORROR = project("HorrorGame", "code/HorrorGame", true, "codex");
const BIRDING = project("Birding.Log", "Documents/Birding.Log", false, "claude");
const API_SERVER = project("api-server", "code/api-server", true, "codex");
const PROJECTS = [HORROR, BIRDING, API_SERVER];

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const agentProfile = (profileId: string, provider: string, dir: string): ProviderProfileSummary => ({
  profile_id: profileId, provider, display_name: `~/${dir}`, path: `${HOME}/${dir}`, canonical_path: `${HOME}/${dir}`,
  revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
});
const PROFILES = [agentProfile("p-codex", "codex", ".codex"), agentProfile("p-claude", "claude", ".claude")];

const binding = (summary: LocalProjectSummary): ProjectBinding => ({
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica-this-mac",
  local_project_id: summary.local_project_id,
  bundle_id: summary.bundle_id,
  project_root: summary.project_root ?? `${HOME}/${summary.display_name}`,
  canonical_project_root: summary.canonical_project_root ?? `${HOME}/${summary.display_name}`,
  profile_ids: summary.profile_ids ?? {},
  state: "active",
  revision: 3,
  updated_at: now - DAY,
});

const PET_COORDINATE = { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 };
const PETS: CatalogCapabilityDescriptor = {
  ...PET_COORDINATE, provider_label: "Codex", label: "Pets v2", singular_label: "Pet v2",
  presentation: { icon: "pet-v2", kind_label: "Pets v2", format_label: "Codex pet v2" }, capture: "directory_tree", apply: "automatic", targets: [],
};
const SKILLS: CatalogCapabilityDescriptor = {
  provider: "codex", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1, provider_label: "Codex", label: "Skills", singular_label: "Skill",
  presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" }, capture: "directory_tree", apply: "automatic", targets: [],
};
const SKILL_MARK: IconName = catalogPresentation(SKILLS).icon;
const PET_MARK: IconName = catalogPresentation(PETS).icon;
const CATALOG_PROFILE: CatalogProfile = {
  profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Default Codex", configured_home: "~/.codex", revision: 1, created_at: 0, updated_at: 0,
};
const scopeView = (id: string, capability: CatalogCapabilityDescriptor): CatalogScopeView => ({
  scope: { scope_id: id, profile_id: CATALOG_PROFILE.profile_id, provider: capability.provider, resource_kind: capability.resource_kind, format: capability.format, format_version: capability.format_version, revision: 1, created_at: 0, updated_at: 0 },
  profile: CATALOG_PROFILE, label: capability.label, available: true, provider_label: "Codex", kind_label: capability.label, format_label: capability.format, capability,
});
const PETS_SCOPE = "scope-pets-v2";
const SKILLS_SCOPE = "scope-skills";
const SCOPES = [scopeView(PETS_SCOPE, PETS), scopeView(SKILLS_SCOPE, SKILLS)];

/* ——— the sessions the pages show ———————————————————————————————————— */

const exact = (value: number) => ({ value, quality: "exact" as const });
/* 2026-09-12T07:00:00Z */
const START = 1_789_196_400;

interface SessionFixture {
  detail: ProjectSessionDetail;
  events: SessionTimelineEvent[];
  provider: "codex" | "claude";
  providerName: string;
  cloudThreadId: string;
  bytes: number;
}

const HORROR_SESSION: SessionFixture = {
  provider: "codex",
  providerName: "Codex",
  cloudThreadId: "cloudthread-7f3a9c0e2b1d4f6a8c9e",
  bytes: 184 * KB,
  detail: {
    resource_id: "codex:conversation:019fb002-0000-7000-8000-00000000aa01",
    provider: "codex",
    provider_display_name: "Codex",
    provider_session_id: "019fb002-0000-7000-8000-00000000aa01",
    title: "Improve game interaction and collision",
    summary: "Sprint tunneling fixed with movement substeps; the interact prompt is gated on the interactable group and fades.",
    started_at: START,
    ended_at: START + 16 * 60,
    is_active: false,
    branch: "main",
    recorded_sha: "9f3c2d1" + "a".repeat(33),
    tokens: exact(48_200),
    event_counts: {
      prompts: exact(2), responses: exact(2), intermediate_steps: exact(0), checkpoints: exact(1), tool_calls: exact(2),
      file_edits: exact(1), shell_calls: exact(1), reads: exact(0), other_tools: exact(0), agent_calls: exact(0),
    },
    actions: [
      { action_id: "codex.open_in_app", kind: "open_in_app", label: "Open in Codex", enabled: true },
      { action_id: "codex.open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true },
    ],
    github_repository_url: "https://github.com/Rika321/HorrorGame",
    snapshot_id: "snapshot-horror-1",
    warnings: [],
  },
  events: [
    { event_id: "p1", ordinal: 1, timestamp: START, kind: "prompt", quality: "exact", turn_id: "t1", turn_tool_summary: { turn_id: "t1", tool_calls: exact(2) },
      text: "The player clips through the cabin walls when sprinting. Fix the collision so sprinting can't tunnel, and make the interaction prompt only appear when we're actually facing something interactable." },
    { event_id: "c1", ordinal: 2, timestamp: START + 60, kind: "tool_call", quality: "exact", turn_id: "t1", parent_event_id: "p1",
      tool: { provider_call_id: "call-1", native_name: "exec", category: "shell", status: "completed", summary: "godot --headless --script res://tools/collision_audit.gd", argument_truncated: false, result_preview: "3 bodies tunnel at sprint speed", result_truncated: false, affected_paths: [] } },
    { event_id: "c2", ordinal: 3, timestamp: START + 240, kind: "tool_call", quality: "exact", turn_id: "t1", parent_event_id: "p1",
      tool: { provider_call_id: "call-2", native_name: "edit", category: "file_edit", status: "completed", summary: "player/Player.gd (+18 −4)", argument_truncated: false, result_truncated: false, affected_paths: ["player/Player.gd"] } },
    { event_id: "k1", ordinal: 4, timestamp: START + 9 * 60, kind: "checkpoint", quality: "exact", turn_id: "t1",
      checkpoint: { sha: "9f3c2d1" + "a".repeat(33), short_sha: "9f3c2d1", subject: "Fix sprint tunneling and gate the interact prompt", committed_at: START + 9 * 60, branch: "main" } },
    { event_id: "r1", ordinal: 5, timestamp: START + 9 * 60 + 20, kind: "response", quality: "exact", turn_id: "t1",
      text: "Done. Sprinting now moves in four substeps per physics frame, so no step is longer than the thinnest wall, and the interact prompt only shows when the raycast hits something in the `interactable` group." },
    { event_id: "p2", ordinal: 6, timestamp: START + 15 * 60, kind: "prompt", quality: "exact", turn_id: "t2", text: "Also make the prompt fade out instead of popping." },
    { event_id: "r2", ordinal: 7, timestamp: START + 16 * 60, kind: "response", quality: "exact", turn_id: "t2",
      text: "The prompt now tweens its alpha to 0 over 150 ms when the target is lost, and back to 1 over 100 ms when a new interactable is hit." },
  ],
};

const BIRDING_START = START + 8 * DAY + 11 * 3600;
const BIRDING_SESSION: SessionFixture = {
  provider: "claude",
  providerName: "Claude",
  cloudThreadId: "cloudthread-2c4e6a8b0d1f3a5c7e9b",
  bytes: 61 * KB,
  detail: {
    resource_id: "claude:conversation:5b1e0c7a-2d4f-4c6e-9a8b-0d1f3a5c7e9b",
    provider: "claude",
    provider_display_name: "Claude",
    provider_session_id: "5b1e0c7a-2d4f-4c6e-9a8b-0d1f3a5c7e9b",
    title: "Tidy the species list",
    summary: "Merged duplicate species rows and normalised the dates in the log.",
    started_at: BIRDING_START,
    ended_at: BIRDING_START + 9 * 60,
    is_active: false,
    branch: null,
    recorded_sha: null,
    tokens: exact(12_400),
    event_counts: {
      prompts: exact(2), responses: exact(2), intermediate_steps: exact(0), checkpoints: exact(0), tool_calls: exact(1),
      file_edits: exact(1), shell_calls: exact(0), reads: exact(0), other_tools: exact(0), agent_calls: exact(0),
    },
    actions: [
      { action_id: "claude.open_in_app", kind: "open_in_app", label: "Open in Claude", enabled: true },
      { action_id: "claude.open_in_terminal", kind: "open_in_terminal", label: "Open in Terminal", enabled: true },
    ],
    snapshot_id: "snapshot-birding-1",
    warnings: [],
  },
  events: [
    { event_id: "p1", ordinal: 1, timestamp: BIRDING_START, kind: "prompt", quality: "exact", turn_id: "t1", turn_tool_summary: { turn_id: "t1", tool_calls: exact(1) },
      text: "The species list has duplicates with different capitalisation. Merge them and fix the dates." },
    { event_id: "c1", ordinal: 2, timestamp: BIRDING_START + 120, kind: "tool_call", quality: "exact", turn_id: "t1", parent_event_id: "p1",
      tool: { provider_call_id: "call-1", native_name: "Edit", category: "file_edit", status: "completed", summary: "species.csv (+7 −21)", argument_truncated: false, result_truncated: false, affected_paths: ["species.csv"] } },
    { event_id: "r1", ordinal: 3, timestamp: BIRDING_START + 6 * 60, kind: "response", quality: "exact", turn_id: "t1",
      text: "Merged 14 duplicate rows into 7 and rewrote 31 dates as ISO 8601. The log now sorts by date without surprises." },
    { event_id: "p2", ordinal: 4, timestamp: BIRDING_START + 8 * 60, kind: "prompt", quality: "exact", turn_id: "t2", text: "Keep the common names capitalised as the field guide does." },
    { event_id: "r2", ordinal: 5, timestamp: BIRDING_START + 9 * 60, kind: "response", quality: "exact", turn_id: "t2",
      text: "Done: sentence case for common names, as in the guide, with the two hyphenated names left as printed." },
  ],
};
const SESSIONS: Record<string, SessionFixture> = {
  [HORROR_SESSION.detail.resource_id]: HORROR_SESSION,
  [BIRDING_SESSION.detail.resource_id]: BIRDING_SESSION,
};

/* ——— the Pull review's plan: one row, no files ——————————————————————— */

type RowAction = "create_local" | "materialize_cloud" | "keep_both";

function sessionRow(session: SessionFixture, action: RowAction, key: string): ConversationReviewRow {
  const local = action !== "create_local";
  return {
    stored_without_credential_redaction: false,
    provider: session.provider,
    thread_id: local ? session.detail.provider_session_id : null,
    cloud_thread_id: session.cloudThreadId,
    resource_id: local ? session.detail.resource_id : `${session.provider}:conversation:${session.cloudThreadId}`,
    action,
    action_id: `selection-${key}`,
    blocker: null,
    blocked_reason: null,
    display_title: session.detail.title,
    source_size: session.bytes,
    stored_ended_at: session.detail.ended_at,
  };
}

function plan(summary: LocalProjectSummary, rows: ConversationReviewRow[]): PullReview {
  const bound = binding(summary);
  return {
    apply_limit_bytes: 512 * MB,
    schema_version: 1,
    plan_id: `plan-${summary.local_project_id}-${rows.map((row) => row.action_id).join("-")}`,
    storage_id: PERSONAL.storage_id,
    bundle_id: bound.bundle_id,
    host_id: bound.host_id,
    replica_id: bound.replica_id,
    expected_host_identity_revision: 0,
    revision: 7,
    commit_id: "commit-7",
    manifest_sha256: "a".repeat(64),
    binding_revision: bound.revision,
    created_at: now,
    expires_at: now + DAY,
    actions: [],
    conversation_review: rows,
  };
}

/** Another stored session of the project, for the project's own review (scene 10). */
function otherSession(title: string, key: string, bytes: number): ReviewedRow {
  const session: SessionFixture = {
    ...HORROR_SESSION,
    cloudThreadId: `cloudthread-${key}`,
    bytes,
    detail: { ...HORROR_SESSION.detail, resource_id: `codex:conversation:cloudthread-${key}`, provider_session_id: `cloudthread-${key}`, title, ended_at: START - 3 * DAY },
  };
  const row: ConversationReviewRow = {
    ...sessionRow(session, "create_local", key),
    display_title: title,
    source_size: bytes,
    stored_ended_at: START - 3 * DAY,
  };
  return { session, action: "create_local", row };
}

/* ——— the stand-in backend the shipped pages call ————————————————————— */

const runtime = window as unknown as { isTauri: boolean };
runtime.isTauri = true;
mockIPC((command) => (command === "plugin:path|resolve_directory" ? HOME : null));

const cloudProject = (overrides: Partial<ManagedCloudRepository>): ManagedCloudRepository => ({
  repository_id: "repo_maya_portal", owner_login: "maya", name: "portal", address: "maya/portal",
  website_url: `${ORIGIN}/maya/portal`, bundle_id: "b".repeat(32), repository_class: "git_project", visibility: "public",
  role: null, can_push: true, repository_fingerprint: "f".repeat(64), git_remote: "differs",
  github_url: "https://github.com/maya/portal", revision: 6, updated_at: now - 9 * DAY, resources: 14, registration_only: false,
  description: "Customer portal, built with Codex.", star_count: 7, viewer_starred: true,
  ...overrides,
});
const MAYA_PORTAL = cloudProject({});
const MAYA_FIELD_NOTES = cloudProject({
  repository_id: "repo_maya_field_notes", name: "field-notes", address: "maya/field-notes", website_url: `${ORIGIN}/maya/field-notes`,
  bundle_id: "c".repeat(32), repository_class: "non_git_project", repository_fingerprint: null, github_url: null, resources: 9,
  description: "Field notes and photo logs, kept with Claude.",
});

/** `owner/name` out of whatever the field was handed. */
const addressOf = (text: string) => text.trim().replace(/^https?:\/\/[^/]+\//i, "").replace(/\/+$/, "").toLowerCase();

projectSyncApi.listAgentFolders = async () => ({ folders: [] });
projectSyncApi.listManagedCloudRepositories = async () => {
  await wait(250);
  return [];
};
projectSyncApi.listManagedCloudStarred = async () => ({ rows: [], next_cursor: null });
projectSyncApi.listManagedCloudPublic = async () => ({ rows: [], next_cursor: null });
projectSyncApi.resolveManagedCloudRepository = async (_storageId, address): Promise<ManagedCloudRepositoryResolution> => {
  await wait(350);
  const wanted = addressOf(address);
  if (wanted === MAYA_PORTAL.address) return { kind: "public", repository: MAYA_PORTAL };
  if (wanted === MAYA_FIELD_NOTES.address) return { kind: "public", repository: MAYA_FIELD_NOTES };
  return { kind: "not_found", address: wanted };
};
projectSyncApi.probeCloudProjectFolder = async () => ({ folder: { kind: "new" }, can_clone: true });

projectSyncApi.getProjectSessionDetail = async (_projectId, resourceId) => {
  await wait(250);
  const session = SESSIONS[resourceId];
  if (!session) throw new Error(`no such session: ${resourceId}`);
  return session.detail;
};
projectSyncApi.getProjectSessionTimelinePage = async (_projectId, resourceId, query) => {
  await wait(200);
  const session = SESSIONS[resourceId];
  if (!session) throw new Error(`no such session: ${resourceId}`);
  const kinds = new Set(query.included_event_kinds);
  return {
    resource_id: resourceId,
    snapshot_id: query.snapshot_id,
    events: session.events.filter((event) => kinds.has(event.kind)),
    next_cursor: null,
    warnings: [],
  };
};
projectSyncApi.getProjectSessionEventDetail = async (_projectId, _resourceId, _snapshotId, eventId) => ({
  event_id: eventId,
  arguments: null,
  arguments_truncated: false,
  result: null,
  result_truncated: false,
  affected_paths: [],
});

/* The review draws its rows through the shipped Activity list, which reads
   the project's activity and its storage comparison. Both are answered for
   the review on screen: the link's session, and the project's other stored
   sessions when the project's own review is open. */
interface ReviewedRow { session: SessionFixture; action: RowAction; row: ConversationReviewRow }
let currentReview: { summary: LocalProjectSummary; rows: ReviewedRow[] } | null = null;

function sessionSummary({ session, action, row }: ReviewedRow): ProjectSessionSummary {
  const local = action !== "create_local";
  return {
    resource_id: row.resource_id,
    provider: session.provider,
    provider_display_name: session.providerName,
    provider_session_id: local ? session.detail.provider_session_id : null,
    logical_thread_id: local ? session.detail.provider_session_id : session.cloudThreadId,
    source: action === "create_local" ? "remote_only" : "local_and_remote",
    access: "bound_ready",
    title: row.display_title ?? session.detail.title,
    preserved: null,
    summary: session.detail.summary ?? null,
    started_at: (row.stored_ended_at ?? session.detail.ended_at ?? START) - 16 * 60,
    ended_at: row.stored_ended_at ?? session.detail.ended_at ?? START,
    last_prompt_at: (row.stored_ended_at ?? session.detail.ended_at ?? START) - 60,
    transcript_path: local ? `${HOME}/.${session.provider}/sessions/${session.detail.provider_session_id}.jsonl` : null,
    working_directory: local ? HORROR.project_root : null,
    recorded_sha: session.detail.recorded_sha ?? null,
    is_active: false,
    user_round_count: 2,
    agent_message_count: 2,
    tool_call_count: 2,
    total_tokens: session.detail.tokens.value ?? null,
    metrics_complete: true,
    actions: local ? session.detail.actions : [],
  };
}

function comparisonEntry({ session, action, row }: ReviewedRow): ThreadSyncEntry {
  const local = action !== "create_local";
  return {
    provider: session.provider,
    provider_display_name: session.providerName,
    access: "bound_ready",
    thread_id: local ? session.detail.provider_session_id : session.cloudThreadId,
    conversation_resource_id: row.resource_id,
    resource_id: row.resource_id,
    display_name: row.display_title ?? session.detail.title,
    state: action === "create_local" ? "storage_only" : action === "materialize_cloud" ? "storage_ahead" : "diverged",
    local_present: local,
    storage_present: true,
    storage_updated_at: row.stored_ended_at ?? null,
    local_updated_at: local ? (row.stored_ended_at ?? START) - 3600 : null,
    cloud_thread_id: row.cloud_thread_id,
    compared_against_storage: true,
    subagent_selection: { wire_children: "off", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: local ? session.detail.provider_session_id : null,
    pull_bytes: row.source_size ?? session.bytes,
    push_decision: { action: "no_changes", selectable: false },
    pull_decision: { action, selectable: true },
  };
}

function activityFor(summary: LocalProjectSummary, rows: ReviewedRow[]): ProjectActivity {
  const provider = rows[0]?.session.provider ?? "codex";
  const providerName = rows[0]?.session.providerName ?? "Codex";
  return {
    project_id: summary.local_project_id,
    sessions: rows.map(sessionSummary),
    git: null,
    unmapped: [],
    providers: [{ provider, display_name: providerName, access: "bound_ready" }],
    warnings: [],
    window_start: START - 30 * DAY,
    window_end: now,
    older: null,
    storage_sync: [{ storage_id: PERSONAL.storage_id, storage_name: PERSONAL.name, last_pull_at: now - DAY, last_push_at: now - 2 * DAY }],
  };
}

function comparisonFor(summary: LocalProjectSummary, rows: ReviewedRow[]): ThreadSyncComparison {
  const entries = rows.map(comparisonEntry);
  return {
    project_id: summary.local_project_id,
    storage_id: PERSONAL.storage_id,
    storage_name: PERSONAL.name,
    revision: 7,
    base_revision: 6,
    compared_at: now,
    entries,
    counts: {
      synced: 0,
      local: 0,
      storage: entries.filter((entry) => entry.state !== "diverged").length,
      diverged: entries.filter((entry) => entry.state === "diverged").length,
      unavailable: 0,
      unknown: 0,
    },
    scopes: [],
    provider_errors: {},
    warnings: [],
    capture_budget: { limit_bytes: 512 * MB, reserved_bytes: 0 },
    remote_continuity: { state: "in_sync", revision: 7 },
  };
}

projectSyncApi.getProjectActivity = async () => {
  await wait(150);
  if (!currentReview) throw new Error("no review on screen");
  return activityFor(currentReview.summary, currentReview.rows);
};
projectSyncApi.refreshProjectActivity = Object.assign(
  async (_request: ActivityRefreshRequest, onEvent: (event: ProjectActivityRefreshEvent) => void) => {
    if (!currentReview) return;
    const { summary, rows } = currentReview;
    onEvent({ stage: "activity", activity: activityFor(summary, rows), storage_pending: false });
    await wait(200);
    onEvent({ stage: "comparison", comparison: comparisonFor(summary, rows) });
  },
  { invalidate: noop },
);
projectSyncApi.invalidateProjectActivity = noop;
projectSyncApi.holdProjectGitWatch = () => noop;
projectSyncApi.onProjectGitChanged = async () => noop;

const MOCHI: CatalogCloudRepository = {
  ...PET_COORDINATE, provider_label: "Codex", kind_label: "Pets v2", format_label: "Codex pet v2",
  repository_id: "repo_maya_mochi", owner_login: "maya", name: "Mochi", address: "maya/Mochi",
  description: "A round duck that naps on the dock and wakes for new tasks.", star_count: 12, viewer_starred: true,
  website_url: `${ORIGIN}/maya/Mochi`, visibility: "public", role: null, can_push: false, revision: 5, updated_at: now - 6 * DAY,
  registration_only: false, resource_id: "mochi", display_name: "Mochi", available: true,
};
const PDF_TOOLS: CatalogCloudRepository = {
  provider: "codex", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1,
  provider_label: "Codex", kind_label: "Skills", format_label: "Agent Skill",
  repository_id: "repo_kai_pdf_tools", owner_login: "kai", name: "pdf-tools", address: "kai/pdf-tools",
  description: "Split, merge and OCR PDFs from a session.", star_count: 31, viewer_starred: false,
  website_url: `${ORIGIN}/kai/pdf-tools`, visibility: "public", role: null, can_push: false, revision: 2, updated_at: now - 3 * DAY,
  registration_only: false, resource_id: "pdf-tools", display_name: "pdf-tools", available: true,
};
storageCatalogApi.listProfiles = async () => [CATALOG_PROFILE];
storageCatalogApi.listCapabilities = async () => [PETS, SKILLS];
storageCatalogApi.listCloudShared = async () => {
  await wait(250);
  return [];
};
storageCatalogApi.listCloudStarred = async () => ({ rows: [], next_cursor: null });
storageCatalogApi.listCloudPublic = async () => ({ rows: [], next_cursor: null });
storageCatalogApi.resolveCloudRepository = async (_storageId, address) => {
  await wait(350);
  const wanted = addressOf(address);
  if (wanted === MOCHI.address.toLowerCase()) return { kind: "found", repository: MOCHI };
  if (wanted === PDF_TOOLS.address.toLowerCase()) return { kind: "found", repository: PDF_TOOLS };
  return { kind: "not_found", address: wanted };
};
storageCatalogApi.linkScopeStorage = async () => ({ link_id: "link-cloud", revision: 1, completion: "open" });
storageCatalogApi.connectCloudRepository = async () => { await wait(300); };

const MOCHI_STATUS: ManagedCloudStatus = {
  repository_id: MOCHI.repository_id,
  state: "ready",
  link: {
    visibility: "public", can_pull: true, can_push: false, repository_id: MOCHI.repository_id, website_url: MOCHI.website_url,
    owner_login: "maya", name: "Mochi", release: { kind: "public" }, repository_revision: 5, can_request_release: false, role: null,
  },
  publication: { state: "matches_published", private_revision: 5, public_revision: 5 },
  message: null,
};

/* ——— the scenes ——————————————————————————————————————————————————— */

type Dir = "a" | "b";
type Stage = "before" | "opening" | "arrival" | "review" | "pulling" | "landed" | "setup";

/** What the service answered, as the sheet says it. */
interface Found {
  mark: IconName;
  /** The found row's second line, in the pickers' words. */
  verdict: string;
  /** The row's third line: what the link names beyond the repository. New. */
  sub: string | null;
  /** Where it stands on this computer. */
  here: string;
  verb: string;
}

interface Stop {
  severity: "info" | "warning";
  message: string;
  action?: string;
}

type Review =
  /** The one-session Pull review, with the row's action. */
  | { kind: "session"; action: RowAction }
  /** Already here and up to date: no review, the session's page. */
  | { kind: "synced" }
  /** The row is not in the storage's current version. */
  | { kind: "gone" }
  /** New project › From Mallard Cloud. */
  | { kind: "setup" }
  /** Add Catalog resource › From Mallard Cloud, then the item's review. */
  | { kind: "add" }
  /** The item's page with its one-item Pull review open. */
  | { kind: "catalog" }
  /** Nothing opens. */
  | { kind: "none" };

interface Scene {
  id: string;
  n: string;
  label: string;
  group: "sessions" | "catalog" | "stops";
  /** What the browser hands over. */
  link: string;
  /** `owner/name` as the link spelled it. */
  subject: string;
  /** The website's page, and its button. */
  page: string;
  storages: StorageSummary[];
  /** The project open when the link arrives, and the one it lands in. */
  project: LocalProjectSummary;
  session: SessionFixture;
  /** A Pull review of the project is open when the link arrives. */
  reviewOpen: boolean;
  found: Found | null;
  stop: Stop | null;
  review: Review;
  shows: string;
  fresh: string;
}

const appLink = (page: string) => `mallard://open?address=${encodeURIComponent(page)}`;
const HORROR_PAGE = `${ORIGIN}/Rika321/HorrorGame/sessions/codex:conversation:${HORROR_SESSION.cloudThreadId}?commit=9f3c2d1&scope=private`;
const BIRDING_PAGE = `${ORIGIN}/Rika321/Birding.Log/sessions/claude%3Aconversation%3A${BIRDING_SESSION.cloudThreadId}?commit=4`;

const HERE_HORROR = `On this computer: ${HORROR.display_name}`;
const ONE_SESSION = "One session of this repository";
const SESSION_FOUND = (verb = "Pull session…"): Found => ({ mark: "git-folder", verdict: "Yours · Owner", sub: ONE_SESSION, here: HERE_HORROR, verb });

const SCENES: Scene[] = [
  {
    id: "session-new", n: "1", label: "Not here", group: "sessions",
    link: appLink(HORROR_PAGE), subject: "Rika321/HorrorGame", page: "session page", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: SESSION_FOUND(), stop: null, review: { kind: "session", action: "create_local" },
    shows: "The common case. The project is connected; the session is only in Mallard Cloud. The sheet's verb opens the shipped Pull review on a plan of one row, already ticked; Pull 1 brings it here and opens its page.",
    fresh: "The sheet's third line. The review's header and its ticked row. The landing on the session's page.",
  },
  {
    id: "session-synced", n: "2", label: "Up to date", group: "sessions",
    link: appLink(HORROR_PAGE), subject: "Rika321/HorrorGame", page: "session page", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: SESSION_FOUND(), stop: null, review: { kind: "synced" },
    shows: "The session is already here and matches storage. The scoped plan comes back synced, so no review opens: the session's page opens with one line, and nothing is pulled.",
    fresh: "The skipped review and its line (decision 7).",
  },
  {
    id: "session-newer", n: "3", label: "Newer in Cloud", group: "sessions",
    link: appLink(HORROR_PAGE), subject: "Rika321/HorrorGame", page: "session page", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: SESSION_FOUND(), stop: null, review: { kind: "session", action: "materialize_cloud" },
    shows: "The session is here, and another computer pushed more of it. The one row is the review's Update row; Pull 1 brings the reviewed storage history into this task.",
    fresh: "Nothing beyond scene 1: the row's words are the review's own.",
  },
  {
    id: "session-diverged", n: "4", label: "Changed on both", group: "sessions",
    link: appLink(HORROR_PAGE), subject: "Rika321/HorrorGame", page: "session page", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: SESSION_FOUND(), stop: null, review: { kind: "session", action: "keep_both" },
    shows: "Changed here and in Mallard Cloud. The one row is the review's Keep both row: the local history stays as an independent task and the storage history comes into this one.",
    fresh: "Nothing beyond scene 1.",
  },
  {
    id: "nongit", n: "5", label: "Non-Git project", group: "sessions",
    link: appLink(BIRDING_PAGE), subject: "Rika321/Birding.Log", page: "session page", storages: [PERSONAL], project: BIRDING, session: BIRDING_SESSION, reviewOpen: false,
    found: { mark: "folder", verdict: "Yours · Owner", sub: ONE_SESSION, here: `On this computer: ${BIRDING.display_name}`, verb: "Pull session…" }, stop: null,
    review: { kind: "session", action: "create_local" },
    shows: "A non-Git project: the same link and route. The session is pulled alone, as in scene 1; the folder's files are not part of a session Pull. The session's page then says they are still in Mallard Cloud, and the project's Pull badge counts them.",
    fresh: "The pending-files line under the bar on the session's page (decision 9).",
  },
  {
    id: "no-project", n: "6", label: "Project not here", group: "sessions",
    link: appLink(`${ORIGIN}/maya/portal/sessions/codex:conversation:${HORROR_SESSION.cloudThreadId}`), subject: "maya/portal", page: "session page", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: { mark: "git-folder", verdict: "Public · anyone signed in can push", sub: ONE_SESSION, here: "Not on this computer. Set up the project, then use the link again.", verb: "Set up project…" }, stop: null,
    review: { kind: "setup" },
    shows: "The repository is not connected to any project on this computer. The sheet offers Set up project…, as a repository link does: the shipped New project › From Mallard Cloud, account ticked, address looked up. The session is not carried through setup in this layer.",
    fresh: "The sentence under the row (decision 8).",
  },
  {
    id: "no-project-folder", n: "7", label: "Non-Git, not here", group: "sessions",
    link: appLink(`${ORIGIN}/maya/field-notes/sessions/claude:conversation:${BIRDING_SESSION.cloudThreadId}`), subject: "maya/field-notes", page: "session page", storages: [PERSONAL], project: BIRDING, session: BIRDING_SESSION, reviewOpen: false,
    found: { mark: "folder", verdict: "Public · anyone signed in can push", sub: ONE_SESSION, here: "Not on this computer. Set up the project, then use the link again.", verb: "Set up project…" }, stop: null,
    review: { kind: "setup" },
    shows: "The same, for a non-Git repository: the shipped page's verb reads Create folder and connect. Nothing is cloned and nothing is pulled; the folder's files and sessions arrive through the project's first Pull.",
    fresh: "Nothing on the page: it is the shipped page after a paste and Look up.",
  },
  {
    id: "pet-saved", n: "8", label: "Pet, in the Catalog", group: "catalog",
    link: appLink(`${ORIGIN}/maya/Mochi`), subject: "maya/Mochi", page: "pet page (the repository page)", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: { mark: PET_MARK, verdict: "Public · Contributor · v5", sub: null, here: "In your Catalog: Pets v2 · Default Codex", verb: "Pull pet…" }, stop: null,
    review: { kind: "catalog" },
    shows: "A pet's page is its repository page, so the link is the repository link. The pet is saved in a Catalog row; the sheet's verb is Pull pet…, which opens the pet's page with its own one-item Pull review — the review its Pull button opens today.",
    fresh: "The sheet's verb. The review opening on arrival. The pet page's body is a stand-in.",
  },
  {
    id: "skill-new", n: "9", label: "Skill, not in Catalog", group: "catalog",
    link: appLink(`${ORIGIN}/kai/pdf-tools`), subject: "kai/pdf-tools", page: "skill page (the repository page)", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: { mark: SKILL_MARK, verdict: "Public · Contributor · v2", sub: null, here: "Not in your Catalog.", verb: "Add to Catalog…" }, stop: null,
    review: { kind: "add" },
    shows: "A skill in no Catalog row. The sheet offers Add to Catalog…, as today: the shipped Add Catalog resource › From Mallard Cloud, item found, row chosen. Press Add: the binding is saved, and because the button said Pull, the skill's page opens with its Pull review (decision 11).",
    fresh: "The review opening after Add. Everything before it is the shipped Add flow.",
  },
  {
    id: "waiting", n: "10", label: "A review is open", group: "stops",
    link: appLink(HORROR_PAGE), subject: "Rika321/HorrorGame", page: "session page", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: true,
    found: SESSION_FOUND(), stop: null, review: { kind: "session", action: "create_local" },
    shows: "The project's own Pull review is open when the link arrives. A link never closes a review: it waits under the bar as Link · … is waiting, with Open. Open closes the review and shows the sheet.",
    fresh: "Nothing: the waiting line is the shipped one.",
  },
  {
    id: "no-account", n: "11", label: "No account", group: "stops",
    link: appLink(HORROR_PAGE), subject: "Rika321/HorrorGame", page: "session page", storages: [], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: null, stop: { severity: "info", message: "Sign in to Mallard Cloud to open Rika321/HorrorGame.", action: "Set up Mallard Cloud" },
    review: { kind: "none" },
    shows: "No storage signs in to the link's host, so nothing can be looked up. This build's service → Set up Mallard Cloud; the link waits for the account. Any other host → No account here for {host}.",
    fresh: "Nothing: the repository link's stop.",
  },
  {
    id: "bad-link", n: "12", label: "Bad link", group: "stops",
    link: appLink(`${ORIGIN}/Rika321/HorrorGame/sessions/not-a-session-id`), subject: "", page: "session page", storages: [PERSONAL], project: HORROR, session: HORROR_SESSION, reviewOpen: false,
    found: null, stop: { severity: "warning", message: "Mallard can’t open that link." },
    review: { kind: "none" },
    shows: "A sessions tail whose id is not a cloud conversation resource id is not a link: the website never sends one. Refused in the usual words, none of its text shown. Other tails (/release, a trailing slash) still open the repository.",
    fresh: "The refusal of a malformed session id.",
  },
];

/* ——— the app window ——————————————————————————————————————————————— */

function Sidebar({ storages, activeProjectId, activeCatalogScopeId, addingProject, addingCatalogScope, addingStorage }: {
  storages: StorageSummary[];
  activeProjectId: string | null;
  activeCatalogScopeId: string | null;
  addingProject: boolean;
  addingCatalogScope: boolean;
  addingStorage: boolean;
}) {
  return (
    <ProjectSidebar
      projects={PROJECTS} drafts={[]} activeDraftId={null} storages={storages} storageDrafts={[]} storageProviders={[]}
      storageUsage={Object.fromEntries(storages.map((storage) => [storage.storage_id, storage === PERSONAL ? 2 : 0]))}
      activeProjectId={activeProjectId} activeStorageId={null} catalogScopes={SCOPES} catalogCapabilities={[PETS, SKILLS]}
      activeCatalogScopeId={activeCatalogScopeId} addingProject={addingProject} addingCatalogScope={addingCatalogScope} addingStorage={addingStorage}
      catalogIndexOpen={false} storageIndexOpen={false} settingsOpen={false} loading={false} busy={false} activityOpen={false} unreadLogs={0}
      onOpenProjects={noop} onSelectProject={noop} onRemoveProject={noop} onSelectDraft={noop} onDiscardDraft={noop}
      onToggleActivity={noop} onAddProject={noop} onOpenStorage={noop} onOpenStorageDraft={noop} onDiscardStorageDraft={noop}
      onRemoveStorage={noop} onAddStorage={noop} onOpenCatalogIndex={noop} onOpenCatalogScope={noop} onAddCatalogScope={noop}
      onRemoveCatalogScope={noop} onOpenStorageIndex={noop} onOpenSettings={noop}
    />
  );
}

/** The shell's zone under the bar, as the app places it. */
function Zone({ items }: { items: NoticeItem[] }) {
  return <NoticeZone items={items} subject="messages from Mallard" className="sync-backend-banner" timing="progress" />;
}

const STORAGE_CRUMB: PathBarCrumb = { id: "storage", label: "Mallard Cloud", icon: "storage-mallard", tone: "root" };
const projectCrumb = (summary: LocalProjectSummary): PathBarCrumb => ({
  id: "project", label: summary.display_name, icon: summary.is_git_repository ? "git-folder" : "folder",
});
const sessionCrumb = (session: SessionFixture): PathBarCrumb => ({
  id: "session",
  label: session.detail.title,
  title: `${session.detail.title} · ${session.providerName}`,
  leading: (
    <span className="sync-path-leaf-mark">
      <ProviderMark provider={session.provider} displayName={session.providerName} />
    </span>
  ),
});

/** The project bar's verbs: refresh, then Pull and Push with their counts. */
function ProjectActions({ pull, push, pullActive, pushDisabled }: { pull: number; push: number; pullActive?: boolean; pushDisabled?: boolean }) {
  return (
    <>
      <button type="button" className="sync-path-icon-action" aria-label="Refresh" title="Refresh">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className={`sync-path-sync${pullActive ? " active" : ""}`} aria-expanded={pullActive}>
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
          {pull > 0 && <span className="sync-path-count" aria-hidden="true">{pull}</span>}
        </button>
        <button type="button" className="sync-path-sync" disabled={pushDisabled}>
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
          {push > 0 && <span className="sync-path-count" aria-hidden="true">{push}</span>}
        </button>
      </div>
    </>
  );
}

function Titlebar({ icon, title }: { icon: IconName; title: string }) {
  return (
    <header className="sync-titlebar">
      <div className="sync-titlebar-context">
        <Icon name={icon} size={16} />
        <strong>{title}</strong>
      </div>
    </header>
  );
}

/** The project's Activity, where the link arrives: its bar is real, its body is not drawn. */
function ProjectPage({ zone, summary, pull }: { zone: ReactNode; summary: LocalProjectSummary; pull: number }) {
  return (
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar crumbs={[STORAGE_CRUMB, projectCrumb(summary)]} actions={<ProjectActions pull={pull} push={0} />} ariaLabel={`${summary.display_name} path`} />
      <main className="sync-main">
        <div className="spec-standin">
          <strong>{summary.display_name}</strong>
          {summary.is_git_repository ? "Git history with its sessions" : "The folder’s sessions"}, and the <em>Only in Mallard Cloud</em> rows. A stand-in for the shipped Activity.
        </div>
      </main>
    </PathBarBelow.Provider>
  );
}

/**
 * Does to the shipped review what the scope does: ticks its one row and names
 * the header after the session. Both are the plan's own changes to
 * `RestorePlanView`; here they are done to the shipped component from outside,
 * and the header is dotted.
 */
function DrivenReview({ title, children }: { title: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let live = true;
    let ticked = false;
    void (async () => {
      while (live) {
        const root = ref.current;
        if (root) {
          const box = root.querySelector<HTMLInputElement>('.sync-sync-row-choice input[type="checkbox"], .sync-pull-item-select input[type="checkbox"]');
          if (box && !box.checked && !ticked) {
            ticked = true;
            box.click();
          }
          const heading = root.querySelector<HTMLElement>("#sync-pull-review-title");
          if (heading && heading.textContent !== `Pull session · ${title}`) {
            heading.textContent = `Pull session · ${title}`;
            heading.classList.add("spec-new");
          }
        }
        await wait(120);
      }
    })();
    return () => {
      live = false;
    };
  }, [title]);
  return <div ref={ref} style={{ display: "contents" }}>{children}</div>;
}

/** The shipped Pull review in the project's workspace: the storage card opens into it. */
function ReviewPage({ zone, summary, review, scoped, pulling, onApply, onBack }: {
  zone: ReactNode;
  summary: LocalProjectSummary;
  review: PullReview;
  /** A session Pull: the row ticked, the header named. */
  scoped: SessionFixture | null;
  pulling: boolean;
  onApply: () => void;
  onBack: () => void;
}) {
  const count = review.conversation_review.length;
  const body = (
    <RestorePlanView
      onSessionActionError={noop}
      project={summary}
      storageName={PERSONAL.name}
      projectName={summary.display_name}
      plan={review}
      binding={binding(summary)}
      phase={pulling ? "restoring" : "idle"}
      supportLoading={false}
      busy={pulling}
      error={null}
      onDismissError={noop}
      onApply={onApply}
      onBack={onBack}
    />
  );
  return (
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar crumbs={[STORAGE_CRUMB, projectCrumb(summary)]} actions={<ProjectActions pull={count} push={0} pullActive />} ariaLabel={`${summary.display_name} path`} />
      <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page sync-sync-review-page">
        <section className="profile-links-section sync-project-overview" aria-label={summary.display_name}>
          <div className="profile-links-list sync-combined-settings-panel">
            <article className="profile-link-card sync-project-link-card">
              <div className="profile-link-connections">
                <section className="project-storage-card" aria-label={`${summary.display_name} storage`}>
                  <div className="project-profile-storage-list">
                    <div className="storage-link-block selected sync-review-open">
                      <div className="sync-storage-inline-review" tabIndex={-1}>
                        {scoped ? <DrivenReview title={scoped.detail.title}>{body}</DrivenReview> : body}
                      </div>
                    </div>
                  </div>
                </section>
              </div>
            </article>
          </div>
        </section>
      </main>
    </PathBarBelow.Provider>
  );
}

/** The session's page after the Pull: the shipped page on the stand-in backend. */
function SessionPage({ zone, summary, session }: { zone: ReactNode; summary: LocalProjectSummary; session: SessionFixture }) {
  return (
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar
        crumbs={[STORAGE_CRUMB, projectCrumb(summary), sessionCrumb(session)]}
        actions={<ProjectActions pull={0} push={0} pushDisabled />}
        ariaLabel={`${summary.display_name} path`}
      />
      <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
        <SessionDetailPage
          localProjectId={summary.local_project_id}
          projectRoot={summary.project_root ?? ""}
          resourceId={session.detail.resource_id}
          occurrenceKey={session.detail.resource_id}
          back={{ kind: "section", label: summary.is_git_repository ? "Git history" : "Sessions", onActivate: noop }}
          onHeaderChange={noop}
          pushGate={null}
        />
      </main>
    </PathBarBelow.Provider>
  );
}

function typeInto(input: HTMLInputElement, text: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Does to the shipped page what the link does: opens From Mallard Cloud,
 *  puts the address in the field, and looks it up. */
function Driven({ address, door, children }: { address: string; door?: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let live = true;
    let filled: HTMLInputElement | null = null;
    void (async () => {
      while (live) {
        const root = ref.current;
        if (root) {
          if (door) {
            const cloudDoor = [...root.querySelectorAll<HTMLButtonElement>(".storage-mode-door")].find((candidate) => candidate.textContent?.includes("From Mallard Cloud"));
            if (cloudDoor && cloudDoor.getAttribute("aria-checked") !== "true") cloudDoor.click();
          }
          const input = root.querySelector<HTMLInputElement>(".cloud-project-repository input");
          if (input && input !== filled && !input.value && !input.disabled) {
            filled = input;
            typeInto(input, address);
            await wait(300);
            if (!live) return;
            root.querySelector<HTMLButtonElement>(".cloud-project-repository .repo-verb")?.click();
          }
        }
        await wait(120);
      }
    })();
    return () => {
      live = false;
    };
  }, [address, door]);
  return <div ref={ref} style={{ display: "contents" }}>{children}</div>;
}

function NewProjectPage({ zone, storages, subject }: { zone: ReactNode; storages: StorageSummary[]; subject: string }) {
  return (
    <>
      <Titlebar icon="folder" title="New project" />
      {zone}
      <main className="sync-main sync-project-links-page sync-project-setup-page">
        <section className="profile-links-section sync-project-overview" aria-label="Project setup">
          <Driven address={subject}>
            <NewProjectFolderStep
              workflowEpoch={1}
              isWorkflowCurrent={() => true}
              projects={PROJECTS}
              providers={PROVIDERS}
              profiles={PROFILES}
              storages={storages}
              busy={false}
              initialSource="cloud"
              initialStorageId={storages[0]?.storage_id ?? null}
              onCancel={noop}
              onChoose={noop}
              onOpenProject={noop}
              onAddCloudAccount={noop}
              onCloudProjectDone={noop}
            />
          </Driven>
        </section>
      </main>
    </>
  );
}

function AddCatalogPage({ zone, subject, onAdded }: { zone: ReactNode; subject: string; onAdded: () => void }) {
  return (
    <>
      <Titlebar icon="catalog" title="Add Catalog resource" />
      {zone}
      <Driven address={subject} door>
        <AddCatalogScopeWorkspace
          workflowEpoch={1}
          isWorkflowCurrent={() => true}
          busy={false}
          initialChoiceKey={null}
          scopes={SCOPES}
          storages={[]}
          cloudAccounts={[{ id: PERSONAL.storage_id, name: PERSONAL.name, account_login: "Rika321" }]}
          onSetUpCloud={noop}
          onClose={noop}
          onCreated={noop}
          onAdded={onAdded}
        />
      </Driven>
    </>
  );
}

const REVIEW_CLASS = "sync-inline-action-review sync-inline-push-review sync-sync-review-workspace";
const HEADER_CLASS = "sync-inline-action-header sync-push-resource-header sync-sync-review-header";
const CONTENT_CLASS = "sync-inline-action-content sync-push-resource-content sync-sync-review-content";

/** The Catalog item's page: its bar with the Cloud chip, and its one-item Pull review, drawn in the review's own classes. */
function CatalogItemPage({ zone, item, reviewOpen, pulling, onApply, onBack }: {
  zone: ReactNode;
  item: CatalogCloudRepository;
  reviewOpen: boolean;
  pulling: boolean;
  onApply: () => void;
  onBack: () => void;
}) {
  const pet = item.resource_kind === "desktop_pet";
  const row: PathBarCrumb = { id: "row", label: pet ? "Pets v2 · Default Codex" : "Skills · Default Codex", icon: pet ? PET_MARK : SKILL_MARK };
  const leaf: PathBarCrumb = { id: "item", label: item.display_name };
  const actions = (
    <>
      <ManagedCloudChip status={{ ...MOCHI_STATUS, repository_id: item.repository_id, link: { ...MOCHI_STATUS.link!, repository_id: item.repository_id, website_url: item.website_url, owner_login: item.owner_login, name: item.name, repository_revision: item.revision } }} storageName={PERSONAL.name} busy={false} refreshing={false} onOpenUrl={noop} onRequestRelease={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
      <ProjectActions pull={reviewOpen ? 1 : 0} push={0} pullActive={reviewOpen} pushDisabled />
    </>
  );
  return (
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar crumbs={[STORAGE_CRUMB, row, leaf]} actions={actions} ariaLabel={`${item.display_name} path`} />
      <main className={`sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page${reviewOpen ? " sync-sync-review-page" : ""}`}>
        {reviewOpen ? (
          <section className="profile-links-section sync-project-overview" aria-label={item.display_name}>
            <div className="profile-links-list sync-combined-settings-panel">
              <article className="profile-link-card sync-project-link-card">
                <div className="profile-link-connections">
                  <section className="project-storage-card" aria-label={`${item.display_name} storage`}>
                    <div className="project-profile-storage-list">
                      <div className="storage-link-block selected sync-review-open">
                        <div className="sync-storage-inline-review" tabIndex={-1}>
                          <section className={REVIEW_CLASS} aria-label="Pull review">
                            <header className={HEADER_CLASS}>
                              <div className="sync-sync-review-title"><h2>Pull review</h2></div>
                              <SyncReviewSelectionBar
                                count={1} total={1} busy={pulling}
                                recommendedTitle="Use the recommended selection (1)" recommendedCount={1} recommendedDisabled onRecommended={noop}
                                clearDisabled={pulling} onClear={noop}
                                close={{ ariaLabel: "Close Pull review", onClose: onBack }}
                              />
                            </header>
                            <div className={CONTENT_CLASS}>
                              <div className="sync-inline-action-scroll sync-sync-review-scroll">
                                <div className="sync-pull-items" role="list">
                                  <div className="sync-pull-item sync-pull-item-selectable selected" role="listitem">
                                    <label className="sync-pull-item-select">
                                      <input type="checkbox" aria-label={`Select ${item.display_name}`} checked readOnly disabled={pulling} />
                                    </label>
                                    <button type="button" className="sync-pull-item-main" aria-expanded={false}>
                                      <span className="sync-pull-item-toggle"><Icon name="chevron-right" size={12} /></span>
                                      <span className="sync-pull-item-icon"><Icon name={pet ? PET_MARK : SKILL_MARK} size={14} /></span>
                                      <span className="sync-pull-item-copy">
                                        <strong>{item.display_name}</strong>
                                        <span>{pet ? "Pet v2 · Codex · 2.1 MB" : "Skill · Codex · 48 KB"} · from {item.address} (public version)</span>
                                      </span>
                                      <span className="sync-history-branch-state history-only">{pet ? "Update to v5" : "Install v2"}</span>
                                    </button>
                                  </div>
                                </div>
                              </div>
                            </div>
                            <footer className="sync-pull-apply-bar sync-inline-action-footer sync-sync-review-footer">
                              {pulling && <p className="sync-review-footer-note" role="status">Pull cannot be cancelled while changes are being applied.</p>}
                              <button type="button" className="btn btn-ghost" disabled={pulling} onClick={onBack}>Back</button>
                              <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={pulling} onClick={onApply}>{pulling ? "Pulling…" : "Pull 1"}</button>
                            </footer>
                          </section>
                        </div>
                      </div>
                    </div>
                  </section>
                </div>
              </article>
            </div>
          </section>
        ) : (
          <div className="spec-standin">
            <strong>{item.display_name} · v{item.revision}</strong>
            Up to date here and in Mallard Cloud. The item’s card, its launch row and ⓘ menu: a stand-in for the shipped page.
          </div>
        )}
      </main>
    </PathBarBelow.Provider>
  );
}

/** The sheet: the connect dialog's frame around the picker's found row, with one line added. */
function Sheet({ scene, looking, onVerb, onClose }: { scene: Scene; looking: boolean; onVerb: () => void; onClose: () => void }) {
  const { found, stop } = scene;
  const verb = looking ? null : found?.verb ?? stop?.action ?? null;
  return (
    <div className="spec-scrim">
      <dialog open className="sync-modal sync-bundle-connection-dialog spec-sheet" aria-label="Open from Mallard Cloud">
        <header className="sync-modal-header">
          <div><h1>Open from Mallard Cloud</h1></div>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close"><Icon name="x" size={17} /></button>
        </header>
        <div className="sync-modal-body repo-connect">
          {looking ? (
            <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Looking up {scene.subject}…</small>
          ) : found ? (
            <>
              <div className="r2-step-list">
                <div className="storage-found-list" aria-label="Repository at this address">
                  <div className="storage-found-row with-verb">
                    <span className="sync-storage-repository-icon"><Icon name={found.mark} size={14} /></span>
                    <span className="storage-found-text">
                      <span className="storage-found-name">{scene.subject}</span>
                      <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {found.verdict}</span>
                      {found.sub && <span className="storage-found-sub spec-new"><Icon name="message" size={12} /> {found.sub}</span>}
                    </span>
                  </div>
                </div>
              </div>
              <small className="repo-answer-line" role="status"><Icon name="computer" size={13} /> {found.here}</small>
            </>
          ) : stop ? (
            <Notice severity={stop.severity} timing="inline">{stop.message}</Notice>
          ) : null}
        </div>
        <footer className="sync-modal-footer">
          <span />
          <div>
            <button type="button" className="btn" onClick={onClose}>{verb ? "Cancel" : "Close"}</button>
            {verb && <button type="button" className="btn btn-primary" onClick={onVerb}>{verb}</button>}
          </div>
        </footer>
      </dialog>
    </div>
  );
}

/** One app window, played from the link. */
function Window({ scene, dir, run, jump, onStage }: { scene: Scene; dir: Dir; run: number; jump: Stage | null; onStage: (stage: Stage) => void }) {
  const [stage, setStageState] = useState<Stage>("before");
  const [sheetOpen, setSheetOpen] = useState(false);
  const [addedItem, setAddedItem] = useState(false);
  const playRef = useRef(0);
  const setStage = (next: Stage) => { setStageState(next); onStage(next); };

  useEffect(() => {
    const play = ++playRef.current;
    const live = () => playRef.current === play;
    setAddedItem(false);
    setSheetOpen(false);
    if (jump) {
      setStage(jump);
      if (jump === "arrival" && !scene.reviewOpen) setSheetOpen(true);
      return;
    }
    setStage("before");
    void (async () => {
      await wait(700);
      if (!live()) return;
      if (scene.reviewOpen) {
        setStage("arrival");
        return;
      }
      setStage("opening");
      setSheetOpen(true);
      await wait(900);
      if (!live()) return;
      if (!scene.stop && dir === "a" && scene.review.kind !== "none") {
        setSheetOpen(false);
        setStage(scene.review.kind === "synced" ? "landed" : "review");
        return;
      }
      setStage("arrival");
    })();
    return () => {
      playRef.current += 1;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, dir, run, jump]);

  /* The sheet's verb. */
  const goOn = () => {
    setSheetOpen(false);
    if (scene.stop?.action) { setStage("setup"); return; }
    switch (scene.review.kind) {
      case "synced": setStage("landed"); return;
      case "gone": setStage("before"); return;
      case "none": setStage("before"); return;
      default: setStage("review");
    }
  };
  const dismiss = () => { setSheetOpen(false); if (stage === "opening" || stage === "arrival") setStage("before"); };
  const openWaiting = () => { setStage("opening"); setSheetOpen(true); void wait(900).then(() => setStage("arrival")); };
  const apply = () => {
    setStage("pulling");
    void wait(1100).then(() => setStage("landed"));
  };

  const items: NoticeItem[] = [];
  const waiting = scene.reviewOpen && stage === "arrival" && !sheetOpen;
  if (waiting) {
    items.push({
      key: "app-link-waiting", severity: "info", title: "Link",
      message: `${scene.session.detail.title} is waiting.`,
      action: <button type="button" className="btn" onClick={openWaiting}>Open</button>,
      dismissalId: String(run), onDismiss: () => setStage("before"),
    });
  }
  if (stage === "landed") {
    const synced = scene.review.kind === "synced";
    if (scene.review.kind === "catalog" || scene.review.kind === "add") {
      items.push({ key: "receipt", severity: "success", message: addedItem ? `${PDF_TOOLS.display_name} added from ${PDF_TOOLS.address}. Pulled v2.` : `Pulled ${MOCHI.display_name} · v5.`, dismissalId: String(run) });
    } else {
      items.push({ key: "receipt", severity: synced ? "info" : "success", message: synced ? "Already here and up to date." : `Pulled “${scene.session.detail.title}”.`, dismissalId: String(run) });
      if (!synced && !scene.project.is_git_repository) {
        items.push({
          key: "files-waiting", severity: "info",
          message: <span className="spec-new">Project files for this folder are still in Mallard Cloud. Pull the project to bring them.</span>,
          action: <button type="button" className="btn">Pull project</button>,
          dismissalId: String(run),
        });
      }
    }
  }
  const zone = <Zone items={items} />;

  const catalog = scene.review.kind === "catalog" || scene.review.kind === "add";
  const item = scene.review.kind === "add" ? PDF_TOOLS : MOCHI;
  const sidebar = {
    storages: scene.storages.length === 0 && stage === "setup" ? [] : scene.storages,
    activeProjectId: catalog && stage !== "before" && stage !== "opening" && stage !== "arrival" ? null : scene.project.local_project_id,
    activeCatalogScopeId: catalog && (stage === "review" || stage === "pulling" || stage === "landed") ? (item === MOCHI ? PETS_SCOPE : SKILLS_SCOPE) : null,
    addingProject: stage === "review" && scene.review.kind === "setup",
    addingCatalogScope: stage === "review" && scene.review.kind === "add" && !addedItem,
    addingStorage: stage === "setup",
  };

  const linkRow: ReviewedRow | null = scene.review.kind === "session"
    ? { session: scene.session, action: scene.review.action, row: sessionRow(scene.session, scene.review.action, "link") }
    : null;
  const projectRows: ReviewedRow[] = [
    { session: scene.session, action: "create_local", row: sessionRow(scene.session, "create_local", "link") },
    otherSession("Add dash cooldown UI", "b2", 96 * KB),
    otherSession("Rework the save menu", "c3", 220 * KB),
  ];
  const over = stage === "before" || stage === "opening" || stage === "arrival";
  currentReview = over && scene.reviewOpen
    ? { summary: scene.project, rows: projectRows }
    : linkRow ? { summary: scene.project, rows: [linkRow] } : null;
  const scopedPlan = linkRow ? plan(scene.project, [linkRow.row]) : null;
  const projectPlan = plan(scene.project, projectRows.map((reviewed) => reviewed.row));

  let page: ReactNode;
  if (over && scene.reviewOpen) {
    page = <ReviewPage zone={zone} summary={scene.project} review={projectPlan} scoped={null} pulling={false} onApply={noop} onBack={noop} />;
  } else if (over) {
    page = <ProjectPage zone={zone} summary={scene.project} pull={scene.review.kind === "synced" ? 0 : 1} />;
  } else if (stage === "setup") {
    page = (
      <>
        <Titlebar icon="storage-mallard" title="New storage" />
        {zone}
        <main className="sync-main">
          <div className="spec-standin">
            <strong>Mallard Cloud sign-in</strong>
            The shipped lane: Sign in, Account, Access, Review. A stand-in; the link waits under the bar until the account is added.
          </div>
        </main>
      </>
    );
  } else if (scene.review.kind === "setup") {
    page = <NewProjectPage zone={zone} storages={scene.storages} subject={scene.subject} />;
  } else if (scene.review.kind === "add" && !addedItem) {
    page = <AddCatalogPage zone={zone} subject={scene.subject} onAdded={() => setAddedItem(true)} />;
  } else if (catalog) {
    page = <CatalogItemPage zone={zone} item={item} reviewOpen={stage !== "landed"} pulling={stage === "pulling"} onApply={apply} onBack={() => setStage("before")} />;
  } else if (stage === "landed") {
    page = <SessionPage zone={zone} summary={scene.project} session={scene.session} />;
  } else if (scopedPlan) {
    page = <ReviewPage zone={zone} summary={scene.project} review={scopedPlan} scoped={scene.session} pulling={stage === "pulling"} onApply={apply} onBack={() => setStage("before")} />;
  } else {
    page = <ProjectPage zone={zone} summary={scene.project} pull={1} />;
  }

  return (
    <div className="spec-frame">
      <div className="sync-app" style={{ "--sync-sidebar-width": "248px" } as CSSProperties}>
        <Sidebar {...sidebar} />
        <div className="sync-workspace">{page}</div>
      </div>
      {sheetOpen && <Sheet scene={scene} looking={stage === "opening"} onVerb={goOn} onClose={dismiss} />}
    </div>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

interface Knobs { scene: string; dir: Dir; stage: Stage | null; theme: "light" | "dark"; shot: boolean }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const stage = query.get("stage");
  return {
    scene: query.get("scene") ?? "session-new",
    dir: query.get("dir") === "a" ? "a" : "b",
    stage: stage === "before" || stage === "arrival" || stage === "review" || stage === "landed" ? stage : null,
    theme: query.get("theme") === "dark" ? "dark" : "light",
    shot: query.get("shot") === "on",
  };
}

const STAGE_WORDS: Record<Stage, string> = {
  before: "Where you were when the link arrived.",
  opening: "The sheet is looking the repository up.",
  arrival: "The sheet has its answer; nothing has moved.",
  review: "The verb was pressed: the review is open, nothing pulled yet.",
  pulling: "Pull 1 is applying.",
  landed: "Landed.",
  setup: "Mallard Cloud is being set up; the link waits.",
};

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const [run, setRun] = useState(0);
  const [stage, setStage] = useState<Stage>("before");
  const scene = SCENES.find((candidate) => candidate.id === knobs.scene) ?? SCENES[0];
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    setKnobs((current) => {
      const next = { ...current, [key]: value };
      const query = new URLSearchParams();
      query.set("scene", next.scene);
      if (next.dir !== "b") query.set("dir", next.dir);
      if (next.stage) query.set("stage", next.stage);
      if (next.theme === "dark") query.set("theme", "dark");
      if (next.shot) query.set("shot", "on");
      window.history.replaceState(null, "", `?${query}`);
      return next;
    });
  };
  useEffect(() => {
    document.documentElement.dataset.theme = knobs.theme;
  }, [knobs.theme]);

  const seg = <T extends string>(key: "scene" | "dir" | "theme", value: T, label: ReactNode, pressed: boolean) => (
    <button key={value} type="button" className="spec-seg" aria-pressed={pressed} onClick={() => { set(key, value as never); setRun((count) => count + 1); }}>{label}</button>
  );

  return (
    <div className="spec">
      {!knobs.shot && (
        <>
          <h1 className="spec-title">Pull this session<small>the link from the website, in the app · docs/PLAN_DEEP_LINK_SESSION_PULL.md</small></h1>
          <div className="spec-controls" role="toolbar" aria-label="Specimen controls">
            <div className="spec-group"><span>Scene</span>{SCENES.map((candidate) => seg("scene", candidate.id, <><b>{candidate.n}</b>{candidate.label}</>, candidate.id === scene.id))}</div>
            <div className="spec-group"><span>Sheet</span>{seg("dir", "b", "B · Sheet first", knobs.dir === "b")}{seg("dir", "a", "A · Straight to the review", knobs.dir === "a")}</div>
            <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light", knobs.theme === "light")}{seg("theme", "dark", "Dark", knobs.theme === "dark")}</div>
            <div className="spec-group"><span>Stage</span>{(["before", "arrival", "review", "landed"] as const).map((candidate) => (
              <button key={candidate} type="button" className="spec-seg" aria-pressed={knobs.stage === candidate} onClick={() => { set("stage", knobs.stage === candidate ? null : candidate); setRun((count) => count + 1); }}>{candidate}</button>
            ))}<button type="button" className="spec-seg" onClick={() => { set("stage", null); setRun((count) => count + 1); }}>▶ Play</button></div>
          </div>
          <div className="spec-browser">
            <span>Website · {scene.page}</span>
            <code title={scene.link}>{scene.link}</code>
            <button type="button" className="btn btn-primary" onClick={() => { set("stage", null); setRun((count) => count + 1); }}>{scene.group === "catalog" ? "Pull in Mallard" : "Pull in Mallard"}</button>
          </div>
          <div className="spec-card">
            <p><span>Shows</span>{scene.shows}</p>
            <p><span>New</span>{scene.fresh}</p>
            <p><span>Stage</span><em className="spec-stage">{STAGE_WORDS[stage]}</em></p>
          </div>
        </>
      )}
      <Window key={`${scene.id}-${knobs.dir}-${run}`} scene={scene} dir={knobs.dir} run={run} jump={knobs.stage} onStage={setStage} />
      {!knobs.shot && (
        <div className="spec-notes">
          <div className="spec-card">
            <strong>What is shipped, what is drawn</strong>
            <ul>
              <li><b>Shipped:</b> the sidebar, the path bar and its verbs, the connect dialog's frame as the sheet, the Pull review on a one-row plan, the session page, New project and Add Catalog resource, the notice zone.</li>
              <li><b>Dotted, the plan's additions:</b> the sheet's third line, the review's header, the ticked row, the pending-files line on a non-Git project's session page.</li>
              <li><b>Stand-ins:</b> the project's Activity behind the sheet, the Mallard Cloud sign-in lane, the Catalog item's page body. The Catalog's one-item review is drawn in the review's own classes.</li>
            </ul>
          </div>
          <div className="spec-card">
            <strong>The rule, in order</strong>
            <ul>
              <li>Read the link in Rust: <code>open</code>, one <code>address</code>, a tail of exactly <code>sessions/{"{id}"}</code>.</li>
              <li>Find the accounts on the host and ask the service for the repository, as a repository link does.</li>
              <li>The project is here → the sheet names the repository and the session, and offers <b>Pull session…</b>. Not here → <b>Set up project…</b>.</li>
              <li>The verb opens the shipped Pull with a scope: one row, no files. Synced → no review, the session's page.</li>
              <li><b>Pull 1</b> applies the one action; the reviewed base and the cloud pin do not move; the session's page opens.</li>
            </ul>
          </div>
        </div>
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
