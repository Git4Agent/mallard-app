import { StrictMode, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type {
  InventoryResource,
  LocalProjectRegistration,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionSummary,
  ProjectStorageLink,
  ProviderDescriptor,
  ProviderProfileSummary,
  StorageProviderDescriptor,
  StorageSummary,
  SyncConfig,
  ThreadSyncEntry,
  ThreadSyncState,
} from "./types";
import type {
  CatalogCapabilityDescriptor,
  CatalogProfile,
  CatalogScopeDto,
  CatalogScopeResources,
  CatalogScopeRow,
} from "./components/storage-catalog/types";
import { projectSyncApi } from "./components/project-sync/api";
import { storageCatalogApi } from "./components/storage-catalog/api";
import { setThemeChoice } from "./preferences";
import App from "./App";
import "./picture-book-mockup.css";

/**
 * Direction C of the Mallard Cloud style mockup ("the picture book") put on
 * the desktop app itself: the shipped `App`, mounted whole against a
 * stand-in backend, with `picture-book-mockup.css` layered over `App.css`.
 * Nothing shipped is edited; the style is one attribute on <html>.
 *
 * `npm run dev`, then /picture-book-mockup.html
 *   ?style=book|today      the picture book, or the app as shipped
 *   ?palette=honey|sepia   the website's Honey & Espresso, or the app's own
 *   ?theme=light|dark
 *   ?screen=home|history|catalog|skills|storages|settings
 *   ?chrome=0              hide the knob bar (for screenshots)
 */

const params = new URLSearchParams(window.location.search);
const style = params.get("style") === "today" ? "today" : "book";
const palette = params.get("palette") === "sepia" ? "sepia" : "honey";
const theme = params.get("theme") === "dark" ? "dark" : "light";
const screen = params.get("screen") ?? "home";
const chrome = params.get("chrome") !== "0";

document.documentElement.dataset.style = style;
document.documentElement.dataset.palette = style === "book" ? palette : "sepia";
setThemeChoice(theme);

// ---------------------------------------------------------------------------
// Fixtures: the same pond as the website mockup.
// ---------------------------------------------------------------------------

const NOW = Math.floor(Date.now() / 1000);
const HOUR = 3600;
const DAY = 86_400;
const HOME = "/Users/rika";

const STORAGE_PROVIDERS: StorageProviderDescriptor[] = [
  {
    key: "managed_cloud", config_version: 1, display_name: "Mallard Cloud",
    description: "Repositories on mallard-ai.com, signed in with GitHub.", display_rank: 5,
    icon: "cloud", clear_mode: "unavailable", fields: [],
    secrets: [{ secret_id: "account_token", label: "Account token", required: true }],
    managed_secret_id: "account_token", local_setup: "managed_cloud",
  },
  {
    key: "r2", config_version: 1, display_name: "Cloudflare R2",
    description: "An R2 bucket reached through its S3 API.", default_name: "R2 storage", display_rank: 10,
    icon: "cloud", clear_mode: "unconditional",
    fields: [{ field_id: "bucket", label: "Bucket name", kind: "text", required: true }],
    secrets: [{ secret_id: "secret_access_key", label: "Secret Access Key", required: true }],
  },
  {
    key: "local", config_version: 1, display_name: "Local folder",
    description: "A folder on this Mac or on a mounted drive.", display_rank: 20,
    icon: "drive", clear_mode: "revision_checked",
    fields: [{ field_id: "directory", label: "Location path", kind: "directory", required: true }],
    secrets: [],
  },
];

const STORAGES: StorageSummary[] = [
  {
    storage_id: "storage-cloud", name: "Mallard Cloud", provider: "managed_cloud", config_version: 1,
    provider_label: "Mallard Cloud", icon: "cloud", available: true,
    summary: [
      { field_id: "account_login", label: "Account", value: "Rika321", is_path: false },
      { field_id: "service_origin", label: "Service", value: "https://mallard-ai.com", is_path: false },
    ],
    settings: { service_origin: "https://mallard-ai.com", account_id: "user_1", account_login: "Rika321" },
    secrets: [{ secret_id: "account_token", present: true }],
    connection: { state: "connected", code: "ok", message: "Connected", observed_at: NOW - 120 },
  },
  {
    storage_id: "storage-r2", name: "Attic R2", provider: "r2", config_version: 1,
    provider_label: "Cloudflare R2", icon: "cloud", available: true,
    summary: [{ field_id: "bucket", label: "Bucket", value: "mallard-otter", is_path: false }],
    settings: { bucket: "mallard-otter" },
    secrets: [{ secret_id: "secret_access_key", present: true }],
  },
  {
    storage_id: "storage-local", name: "Backup disk", provider: "local", config_version: 1,
    provider_label: "Local folder", icon: "drive", available: true,
    summary: [{ field_id: "directory", label: "Location", value: "/Volumes/Backup/mallard", is_path: true }],
    settings: { directory: "/Volumes/Backup/mallard" },
    secrets: [],
  },
];

const RECIPE = { schema_version: 1 as const, revision: 1, entries: {} };

function registration(id: string, name: string, git: boolean): LocalProjectRegistration {
  return {
    local_project_id: id, bundle_id: `bundle-${id}`, display_name: name, local_alias: null,
    repository_fingerprint: git ? id.padEnd(64, "0") : null,
    recipe: RECIPE, revision: 3, created_at: NOW - 40 * DAY, updated_at: NOW - 2 * HOUR,
  };
}

const PROJECTS: LocalProjectRegistration[] = [
  registration("tidewatch", "tidewatch", true),
  registration("horrorgame", "HorrorGame", true),
  registration("birding-log", "Birding-Log", false),
  registration("ledger-api", "ledger-api", true),
];

function link(projectId: string, storageId: string): ProjectStorageLink {
  return {
    local_project_id: projectId, storage_id: storageId, bundle_id: `bundle-${projectId}`,
    recipe: null, scope_bases: {},
    project_content_preferences: { schema_version: 1, revision: 1, excluded_resource_ids: [] },
    conversation_checkpoints: {}, pinned: true, created_at: NOW - 30 * DAY,
  };
}

const LINKS: ProjectStorageLink[] = [
  link("tidewatch", "storage-cloud"),
  link("tidewatch", "storage-r2"),
  link("horrorgame", "storage-cloud"),
  link("birding-log", "storage-local"),
  link("ledger-api", "storage-r2"),
];

const CONFIG: SyncConfig = { schema: 1, revision: 12, storages: STORAGES, projects: PROJECTS, links: LINKS };

const BINDINGS: ProjectBinding[] = PROJECTS.map((project) => ({
  host_id: "00112233445566778899aabbccddeeff", replica_id: "replica-1",
  local_project_id: project.local_project_id, bundle_id: project.bundle_id,
  project_root: `${HOME}/code/${project.display_name}`, canonical_project_root: `${HOME}/code/${project.display_name}`,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  state: "active", revision: 2, updated_at: NOW - 10 * DAY,
}));

const AGENT_PROVIDERS: ProviderDescriptor[] = [
  { key: "claude", display_name: "Claude", display_rank: 10, profile_label: "Claude home", setup_default: true,
    cli_launcher: { program: "claude", display_name: "Claude Code" }, maintenance_actions: [] },
  { key: "codex", display_name: "Codex", display_rank: 20, profile_label: "Codex home", setup_default: true,
    cli_launcher: { program: "codex", display_name: "Codex CLI" }, maintenance_actions: [] },
];

const PROFILES: ProviderProfileSummary[] = [
  { profile_id: "profile-claude", provider: "claude", display_name: "Default Claude", path: `${HOME}/.claude`,
    canonical_path: `${HOME}/.claude`, revision: 1, created_at: NOW - 60 * DAY, updated_at: NOW - 60 * DAY,
    available: true, readable: true, writable: true, used_by_projects: PROJECTS.map((p) => p.local_project_id) },
  { profile_id: "profile-codex", provider: "codex", display_name: "Default Codex", path: `${HOME}/.codex`,
    canonical_path: `${HOME}/.codex`, revision: 1, created_at: NOW - 60 * DAY, updated_at: NOW - 60 * DAY,
    available: true, readable: true, writable: true, used_by_projects: PROJECTS.map((p) => p.local_project_id) },
];

// Sessions, newest first. `state` is what the storage comparison says.
const SESSIONS: Array<{ id: string; provider: "claude" | "codex"; title: string; ago: number; rounds: number; tools: number; state: ThreadSyncState }> = [
  { id: "s01", provider: "claude", title: "Resolve stations by name with fuzzy matching", ago: 2 * HOUR, rounds: 9, tools: 41, state: "local_ahead" },
  { id: "s02", provider: "codex", title: "Print predictions in the station's own timezone", ago: 7 * HOUR, rounds: 6, tools: 23, state: "local_only" },
  { id: "s03", provider: "claude", title: "Cache NOAA responses for an hour", ago: 1 * DAY + 3 * HOUR, rounds: 4, tools: 17, state: "synced" },
  { id: "s04", provider: "codex", title: "Add --json output and a snapshot test", ago: 2 * DAY, rounds: 5, tools: 28, state: "synced" },
  { id: "s05", provider: "claude", title: "Retry 429s with jittered backoff", ago: 3 * DAY + 5 * HOUR, rounds: 3, tools: 12, state: "storage_ahead" },
  { id: "s06", provider: "codex", title: "Split the client into its own crate", ago: 4 * DAY, rounds: 11, tools: 64, state: "synced" },
  { id: "s07", provider: "claude", title: "Write the README quick start", ago: 6 * DAY, rounds: 2, tools: 6, state: "synced" },
  { id: "s08", provider: "codex", title: "Parse high and low tide events", ago: 8 * DAY, rounds: 7, tools: 31, state: "diverged" },
];

const resourceId = (s: (typeof SESSIONS)[number]) => `${s.provider}:conversation:${s.id}`;
const providerLabel = (key: string) => (key === "claude" ? "Claude" : "Codex");

function session(s: (typeof SESSIONS)[number]): ProjectSessionSummary {
  return {
    resource_id: resourceId(s), provider: s.provider, provider_display_name: providerLabel(s.provider),
    provider_session_id: s.id, logical_thread_id: s.id,
    source: s.state === "storage_only" ? "remote_only" : s.state === "local_only" ? "local_only" : "local_and_remote",
    access: "bound_ready", title: s.title, preserved: null, summary: "",
    started_at: NOW - s.ago - 40 * 60, ended_at: NOW - s.ago,
    working_directory: `${HOME}/code/tidewatch`, recorded_sha: "a".repeat(40),
    is_active: s.ago < 3 * HOUR, user_round_count: s.rounds, agent_message_count: s.rounds * 2,
    tool_call_count: s.tools, total_tokens: s.tools * 2100, metrics_complete: true,
    actions: [{ action_id: `${s.provider}.open-in-app`, kind: "open_in_app", label: `Open in ${providerLabel(s.provider)}`, enabled: true }],
  };
}

const COMMITS = [
  { subject: "Resolve stations by name", ago: 90 * 60, refs: ["s01"] },
  { subject: "Show local station time in every table", ago: 6 * HOUR, refs: ["s02"] },
  { subject: "Cache NOAA responses", ago: 1 * DAY + 2 * HOUR, refs: ["s03"] },
  { subject: "Add --json output", ago: 2 * DAY - HOUR, refs: ["s04"] },
  { subject: "Bump reqwest to 0.12", ago: 2 * DAY + 4 * HOUR, refs: [] },
  { subject: "Retry rate-limited requests", ago: 3 * DAY + 4 * HOUR, refs: ["s05"] },
  { subject: "Move the NOAA client into tidewatch-noaa", ago: 4 * DAY - 2 * HOUR, refs: ["s06"] },
  { subject: "Quick start in the README", ago: 6 * DAY - HOUR, refs: ["s07"] },
  { subject: "Parse hi/lo events", ago: 8 * DAY - HOUR, refs: ["s08"] },
];

function activityFor(projectId: string): ProjectActivity {
  const tidewatch = projectId === "tidewatch";
  const sessions = (tidewatch ? SESSIONS : SESSIONS.slice(2, 6)).map(session);
  return {
    project_id: projectId,
    sessions,
    git: projectId === "birding-log" ? null : {
      selected_branch: "main",
      branches: [{ name: "main", is_current: true }, { name: "station-search", is_current: false }],
      commits: (tidewatch ? COMMITS : COMMITS.slice(2, 6)).map((c, i) => {
        const sha = (i + 1).toString(16).padStart(2, "0").repeat(20);
        return {
          sha, short_sha: sha.slice(0, 7), committed_at: NOW - c.ago, subject: c.subject,
          session_refs: c.refs.map((id) => ({ resource_id: resourceId(SESSIONS.find((s) => s.id === id)!) })),
        };
      }),
      github_repository_url: `https://github.com/Rika321/${projectId}`,
      unique_session_count: sessions.length,
    },
    unmapped: [],
    providers: AGENT_PROVIDERS.map((p) => ({ provider: p.key, display_name: p.display_name, access: "bound_ready" })),
    warnings: [],
    window_start: NOW - 30 * DAY,
    window_end: NOW,
    older: null,
    storage_sync: [{ storage_id: "storage-cloud", storage_name: "Mallard Cloud", last_pull_at: NOW - DAY, last_push_at: NOW - 3 * HOUR }],
  };
}

function entry(s: (typeof SESSIONS)[number]): ThreadSyncEntry {
  const local = s.state !== "storage_only";
  const remote = s.state !== "local_only";
  return {
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider: s.provider, provider_display_name: providerLabel(s.provider), access: "bound_ready",
    thread_id: s.id, conversation_resource_id: resourceId(s), resource_id: resourceId(s),
    display_name: s.title, state: s.state, local_present: local, storage_present: remote,
    local_updated_at: NOW - s.ago, storage_updated_at: remote ? NOW - s.ago - HOUR : null,
    cloud_thread_id: remote ? s.id : null, compared_against_storage: true,
    capture_bytes: s.tools * 180_000,
    push_decision: { action: s.state === "synced" ? "no_changes" : "publish", selectable: s.state !== "synced" },
    pull_decision: { action: s.state === "storage_ahead" ? "materialize_cloud" : "no_changes", selectable: s.state === "storage_ahead" },
  };
}

function inventoryResource(s: (typeof SESSIONS)[number]): InventoryResource {
  return {
    resource_id: resourceId(s), kind: { key: "conversation", version: 1 }, provider: s.provider,
    provider_resource_type: "session", display_name: s.title,
    provenance: { kind: "conversation", version: 1, fields: {} },
    apply_policy: { key: "safe_file", version: 1 }, codec_version: 1, attributes: {},
    category: "conversations", kind_label: "Conversation", apply_policy_label: "Safe file",
    selectable: true, default_selected: true,
  };
}

// Catalog.
function capability(provider: string, kind: "agent_skill" | "plugin_install_intent" | "desktop_pet"): CatalogCapabilityDescriptor {
  const label = providerLabel(provider);
  if (kind === "agent_skill") {
    return { provider, provider_label: label, resource_kind: kind, format: "agentskills.io.skill", format_version: 1,
      label: "Skills", singular_label: "Skill", presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
      capture: "directory_tree", apply: "automatic", targets: ["provider-home.skills"] };
  }
  if (kind === "plugin_install_intent") {
    return { provider, provider_label: label, resource_kind: kind, format: "mallard.plugin-install-intent", format_version: 1,
      label: "Plugins", singular_label: "Plugin", presentation: { icon: "plugin", kind_label: "Plugins", format_label: "Plugin install intent" },
      capture: "provider_inventory", apply: "automatic", targets: ["user.plugins"] };
  }
  return { provider, provider_label: label, resource_kind: kind, format: "openai.codex-pet", format_version: 1,
    label: "Pets", singular_label: "Pet", presentation: { icon: "pet", kind_label: "Pets", format_label: "Codex pet" },
    capture: "directory_tree", apply: "automatic", targets: ["desktop.pets"] };
}

const CAPABILITIES = [
  capability("claude", "agent_skill"),
  capability("claude", "plugin_install_intent"),
  capability("codex", "agent_skill"),
  capability("codex", "plugin_install_intent"),
  capability("codex", "desktop_pet"),
];

const CATALOG_PROFILES: CatalogProfile[] = PROFILES.map((p) => ({
  profile_id: `cat-${p.profile_id}`, provider: p.provider, provider_label: providerLabel(p.provider),
  display_name: p.display_name, configured_home: p.path, revision: 1, created_at: 1, updated_at: 1,
}));

const SCOPES: CatalogScopeDto[] = [CAPABILITIES[0], CAPABILITIES[2], CAPABILITIES[1], CAPABILITIES[4]].map((cap, i) => ({
  scope: { scope_id: `catscope-${i + 1}`, provider: cap.provider, profile_id: `cat-profile-${cap.provider}`,
    resource_kind: cap.resource_kind, format: cap.format, format_version: cap.format_version, revision: 1, created_at: 1, updated_at: 1 },
  profile: CATALOG_PROFILES.find((p) => p.provider === cap.provider)!,
  provider_label: cap.provider_label, kind_label: cap.presentation.kind_label, format_label: cap.presentation.format_label,
  label: cap.label, available: true,
}));

const SKILL_ROWS: Array<[string, string, CatalogScopeRow["state"]]> = [
  ["release-notes", "Turns a range of commits into user-facing release notes, grouped by area.", "in_sync"],
  ["pond-review", "Reviews a diff the way the pond team does: tests first, naming second.", "local_changed"],
  ["tide-tables", "Formats tide predictions as a Markdown table in the station's timezone.", "in_sync"],
  ["changelog-lint", "Checks CHANGELOG.md against Keep a Changelog before a release.", "new"],
  ["birding-checklist", "Builds a seasonal checklist from an eBird export.", "storage_changed"],
  ["rust-errors", "Explains a rustc error and proposes the smallest fix.", "in_sync"],
];

function scopeResources(scopeId: string, storageId: string | null | undefined): CatalogScopeResources {
  const dto = SCOPES.find((s) => s.scope.scope_id === scopeId) ?? SCOPES[0];
  const storage = STORAGES.find((s) => s.storage_id === (storageId ?? "storage-cloud")) ?? STORAGES[0];
  return {
    scope: dto.scope, profile: dto.profile, provider_label: dto.provider_label, kind_label: dto.kind_label,
    format_label: dto.format_label, storage_id: storage.storage_id, storage_name: storage.name,
    link: { link_id: `link-${scopeId}`, revision: 1 }, storage_revision: 14,
    rows: SKILL_ROWS.map(([name, description, state], i) => ({
      row_id: `row-${i}`, resource_id: `skill-${name}`, display_name: name, description, state,
      local_present: state !== "storage_changed", local_path: `${dto.profile.configured_home}/skills/${name}`,
      local_modified_at: NOW - (i + 1) * DAY, local_files: 3 + i, local_bytes: 4096 * (i + 2),
      remote_present: state !== "new", remote_updated_at: NOW - (i + 2) * DAY,
      warnings: [],
    } as CatalogScopeRow)),
    warnings: [],
    recovery: [],
  };
}

// ---------------------------------------------------------------------------
// The stand-in backend.
// ---------------------------------------------------------------------------

const unmocked = new Set<string>();
function stubAll(api: Record<string, unknown>, label: string) {
  for (const key of Object.keys(api)) {
    api[key] = (...args: unknown[]) => {
      if (!unmocked.has(`${label}.${key}`)) {
        unmocked.add(`${label}.${key}`);
        console.warn(`[picture-book] unmocked ${label}.${key}`, args);
      }
      return Promise.reject(new Error(`${key} is not part of this mock.`));
    };
  }
}
stubAll(projectSyncApi as unknown as Record<string, unknown>, "projectSyncApi");
stubAll(storageCatalogApi as unknown as Record<string, unknown>, "storageCatalogApi");

const api = projectSyncApi as unknown as Record<string, (...args: never[]) => unknown>;
const cat = storageCatalogApi as unknown as Record<string, (...args: never[]) => unknown>;
const ok = <T,>(value: T) => async () => value;
const detail = (id: string) => {
  const project = PROJECTS.find((p) => p.local_project_id === id) ?? null;
  if (!project) return null;
  return {
    project,
    links: LINKS.filter((l) => l.local_project_id === id),
    binding: BINDINGS.find((b) => b.local_project_id === id) ?? null,
    materializations: [],
  };
};

Object.assign(api, {
  getHostIdentityStatus: ok({ host_id: "00112233445566778899aabbccddeeff", revision: 1, local_display_name: "Rika's MacBook Pro", created_at: NOW - 90 * DAY, updated_at: NOW - 90 * DAY }),
  getConfig: ok(CONFIG),
  recoverStartupState: ok(undefined),
  listProjects: ok(PROJECTS),
  listProjectRepositoryKinds: ok({ tidewatch: true, horrorgame: true, "birding-log": false, "ledger-api": true }),
  listSetupDrafts: ok({ drafts: [], warnings: [] }),
  listStorageSetupDrafts: ok({ drafts: [], warnings: [] }),
  listAgentProviders: ok(AGENT_PROVIDERS),
  listStorageProviders: ok(STORAGE_PROVIDERS),
  listProviderProfiles: ok(PROFILES),
  listBindings: ok(BINDINGS),
  getProject: async (id: string) => detail(id),
  getInventory: async (id: string) => ({ project: id, bundle_id: `bundle-${id}`, resources: SESSIONS.map(inventoryResource), recipe: RECIPE, generated_at: NOW, warnings: [] }),
  getSiblingBundleStatus: ok({ state: "none" }),
  getPushDestinationStatus: ok({ state: "ready", revision: 14 }),
  listRemoteBundleSnapshots: ok([]),
  getManagedCloudLink: async (projectId: string, storageId: string) => storageId !== "storage-cloud" ? null : ({
    repository_id: `repo-${projectId}`, website_url: `https://mallard-ai.com/Rika321/${projectId}`,
    owner_login: "Rika321", name: projectId, release: { kind: "public" }, repository_revision: 14,
    can_request_release: false, role: "owner", can_push: true,
  }),
  getProjectActivity: async (id: string) => activityFor(id),
  refreshProjectActivity: async (request: { localProjectId: string; storageId: string }, emit: (event: unknown) => void) => {
    const activity = activityFor(request.localProjectId);
    emit({ stage: "activity", activity });
    const entries = SESSIONS.map(entry);
    const storage = STORAGES.find((s) => s.storage_id === request.storageId) ?? STORAGES[0];
    emit({
      stage: "comparison",
      comparison: {
        project_id: request.localProjectId, storage_id: storage.storage_id, storage_name: storage.name,
        revision: 14, base_revision: 14, compared_at: NOW, entries,
        counts: {
          synced: entries.filter((e) => e.state === "synced").length,
          local: entries.filter((e) => e.state === "local_only" || e.state === "local_ahead").length,
          storage: entries.filter((e) => e.state === "storage_ahead" || e.state === "storage_only").length,
          diverged: entries.filter((e) => e.state === "diverged").length,
          unavailable: 0, unknown: 0,
        },
        scopes: [], provider_errors: {}, warnings: [],
        capture_budget: { limit_bytes: 512 * 1024 * 1024, reserved_bytes: 3 * 1024 * 1024 },
        remote_continuity: { state: "in_sync", revision: 14 },
      },
    });
  },
  queryActivityLogs: ok({ entries: [], next_cursor: null }),
  getActivityLogStats: ok({ total_bytes: 1_840_000, file_count: 14, oldest_ts: NOW - 20 * DAY, newest_ts: NOW, policy: { schema: 1, retention_days: 30, max_total_bytes: 50_000_000 } }),
  getActivityLogFolder: ok(`${HOME}/.mallard/logs`),
  getLocalStateSummary: ok({ root_path: `${HOME}/.mallard`, file_count: 312, total_bytes: 48_200_000, backups_bytes: 6_100_000 }),
  getAppSettings: ok({ push_codex_children: true, push_claude_children: true, credential_redaction_enabled: true, schema: 1,
    conversation_file_limit_bytes: 64 * 1024 * 1024, timeline_preview_limit_bytes: 256 * 1024, comparison_workers: 4, cloud_transfer_concurrency: 6 }),
});

Object.assign(cat, {
  listCapabilities: ok(CAPABILITIES),
  listScopes: ok(SCOPES),
  listProfiles: ok(CATALOG_PROFILES),
  listScopeResources: async (scopeId: string, storageId?: string | null) => scopeResources(scopeId, storageId),
  listStorageRepositories: ok([]),
  getPreview: async () => { throw new Error("no preview"); },
});

// Window, event and dialog plumbing that the shell reaches through Tauri itself.
(window as unknown as { isTauri: boolean }).isTauri = true;
mockWindows("main");
mockIPC((cmd, payload) => {
  console.info(`[picture-book] ipc ${cmd}`, payload);
  return null;
}, { shouldMockEvents: true });

// ---------------------------------------------------------------------------
// The knob bar, outside the app's own root.
// ---------------------------------------------------------------------------

function Knobs() {
  const set = (key: string, value: string) => {
    const next = new URLSearchParams(window.location.search);
    next.set(key, value);
    window.location.search = next.toString();
  };
  const group = (key: string, current: string, values: string[]) => (
    <span className="pb-knob-group">
      {values.map((value) => (
        <button key={value} type="button" className={value === current ? "is-on" : ""} onClick={() => set(key, value)}>{value}</button>
      ))}
    </span>
  );
  return (
    <div className="pb-knobs" role="toolbar" aria-label="Mock controls">
      {group("style", style, ["today", "book"])}
      {style === "book" && group("palette", palette, ["honey", "sepia"])}
      {group("theme", theme, ["light", "dark"])}
      {group("screen", screen, ["home", "history", "catalog", "skills", "storages", "settings"])}
    </div>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

if (chrome) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  createRoot(host).render(<Knobs />);
}

// Walk to the requested screen once the shell is up.
const clickWhenReady = (find: () => HTMLElement | null, then?: () => void, tries = 60) => {
  const target = find();
  if (target) {
    target.click();
    then?.();
  } else if (tries > 0) {
    window.setTimeout(() => clickWhenReady(find, then, tries - 1), 100);
  }
};
const byLabel = (label: string) => () =>
  Array.from(document.querySelectorAll<HTMLElement>("button, a"))
    .find((el) => el.getAttribute("aria-label") === label || el.getAttribute("title") === label || el.textContent?.trim() === label) ?? null;

if (screen === "home") clickWhenReady(byLabel("Projects"));
if (screen === "catalog") clickWhenReady(byLabel("Catalog"));
if (screen === "skills") clickWhenReady(byLabel("Catalog"), () => clickWhenReady(() => document.querySelector<HTMLElement>(".sync-sidebar-rail[aria-label=\"Catalog\"] .sidebar-profile-main")));
if (screen === "storages") clickWhenReady(byLabel("Storages"));
if (screen === "settings") clickWhenReady(byLabel("Settings"));

// ---------------------------------------------------------------------------
// The pond hero: the website's picture-book plate, set above the Projects
// page's cards. Mounted into the shipped page from outside, so no component
// changes; it follows the page in and out.
// ---------------------------------------------------------------------------

const DUCK = (id: string) => (
  <defs>
    <path id={`tail-${id}`} d="M196 192c-9-6-15-19-8-25 9 2 23 13 29 23Z" />
    <path id={`body-${id}`} d="M200 206c0-28 32-44 78-44 34 0 58 8 72 22 10 10 14 22 10 34-6 18-36 26-82 26-46 0-78-12-78-38Z" />
    <path id={`neck-${id}`} d="M318 190c0-18 4-32 12-44l32 6c-4 12-6 26-4 40Z" />
    <circle id={`head-${id}`} cx="350" cy="134" r="25" />
  </defs>
);

const star = (x: number, y: number, r: number) =>
  `M${x} ${y}q${r * 0.25} ${r * 0.75} ${r} ${r}q${-r * 0.75} ${r * 0.25} ${-r} ${r}q${-r * 0.25} ${-r * 0.75} ${-r} ${-r}q${r * 0.75} ${-r * 0.25} ${r} ${-r}Z`;

function Sky({ night }: { night: boolean }) {
  if (night) {
    const stars: Array<[number, number, number]> = [[40, 30, 6], [140, 106, 4], [196, 53, 5], [292, 19, 7], [360, 88, 4], [540, 31, 5], [530, 116, 4], [96, 147, 3], [240, 127, 3]];
    return (
      <>
        {stars.map(([x, y, r]) => <path key={`${x}-${y}`} className="vk-star" d={star(x, y, r)} />)}
        <circle className="vk-sun" cx="452" cy="84" r="34" />
        <circle className="vk-crater" cx="440" cy="76" r="7" />
        <circle className="vk-crater" cx="464" cy="96" r="4.5" />
      </>
    );
  }
  const rays: ReactNode[] = [];
  for (let i = 0; i < 8; i += 1) {
    const a = (i * Math.PI) / 4;
    const x1 = 452 + Math.cos(a) * 50; const y1 = 84 + Math.sin(a) * 50;
    const x2 = 452 + Math.cos(a) * 64; const y2 = 84 + Math.sin(a) * 64;
    rays.push(<path key={i} className="vk-ray" d={`M${x1.toFixed(1)} ${y1.toFixed(1)}L${x2.toFixed(1)} ${y2.toFixed(1)}`} />);
  }
  return <>{rays}<circle className="vk-sun" cx="452" cy="84" r="34" /></>;
}

function Pond({ night }: { night: boolean }) {
  const id = night ? "pb-night" : "pb-day";
  const cloud = "M0 30c0-11 9-19 20-18 3-11 13-18 25-18 13 0 23 9 25 21 9-1 18 6 18 15Z";
  return (
    <svg className="pb-scene" viewBox="0 0 580 260" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
      {DUCK(id)}
      <rect className="vk-sky" width="580" height="260" />
      <rect className="vk-sky2" y="126" width="580" height="80" />
      <Sky night={night} />
      <path className="vk-cloud" transform="translate(52 58) scale(1.1)" d={cloud} />
      <path className="vk-cloud" transform="translate(236 34) scale(0.8)" d={cloud} />
      <path className="vk-hill" d="M-10 178c60-40 120-46 190-20 50-26 110-30 170 0 60-34 130-38 250 4v58H-10Z" />
      <path className="vk-bank" d="M-10 198c40-22 100-24 150 6v40H-10Z" />
      <rect className="vk-water" x="-10" y="200" width="600" height="70" />
      <path className="vk-reflect" d="M430 212h44M438 224h28" />
      <g className="vk-outline"><use href={`#tail-${id}`} /><use href={`#body-${id}`} /><use href={`#neck-${id}`} /><use href={`#head-${id}`} /></g>
      <g><use className="vk-rump" href={`#tail-${id}`} /><use className="vk-body" href={`#body-${id}`} /><use className="vk-head" href={`#neck-${id}`} /><use className="vk-head" href={`#head-${id}`} /></g>
      <path className="vk-breast" d="M312 170c30 2 50 14 50 34 0 10-8 20-24 24-12-18-24-38-26-58Z" />
      <path className="vk-rump-in" d="M204 196c2-10 10-18 20-22 4 14 4 30 0 46-12-4-20-14-20-24Z" />
      <path className="vk-wing" d="M232 194c26-18 76-20 104-4-18 20-68 28-104 16Z" />
      <path className="vk-spec" d="M296 196l24-2c-2 7-9 11-17 13l-9-2Z" />
      <path className="vk-collar" d="M322 168c11 3 25 4 38 1" />
      <path className="vk-bill" d="M370 126c15-2 30 2 38 11-8 8-23 10-38 6Z" />
      <circle className="vk-eye" cx="356" cy="126" r="4" /><circle className="vk-gleam" cx="357.5" cy="124.5" r="1.4" />
      <path className="vk-front" d="M-10 228c30-6 60 6 90 0s60-6 90 0 60 6 90 0 60-6 90 0 60 6 90 0 60-6 90 0 60 6 90 0v50H-10Z" />
      <path className="vk-wave" d="M34 246q12-8 24 0t24 0M180 252q12-8 24 0M380 248q12-8 24 0t24 0" />
      <path className="vk-stem" d="M516 270V150M532 270V132M548 270V162M504 270V186" />
      <rect className="vk-cattail" x="510" y="146" width="12" height="36" rx="6" />
      <rect className="vk-cattail" x="526" y="126" width="12" height="38" rx="6" />
      <rect className="vk-cattail" x="542" y="158" width="12" height="30" rx="6" />
      <path className="vk-leaf" d="M524 270c-6-40 4-70 24-96-10 30-14 60-12 96Z" />
    </svg>
  );
}

type Landing = "Projects" | "Catalog" | "Storages";

const HERO: Record<Landing, { lede: string; readout: Array<[number, string]> }> = {
  Projects: {
    lede: "Every conversation and commit from the projects on this Mac, pushed to the storages you pick.",
    readout: [[PROJECTS.length, "projects"], [SESSIONS.length, "sessions"], [STORAGES.length, "storages"], [AGENT_PROVIDERS.length, "agents"]],
  },
  Catalog: {
    lede: "Skills, plugins and desktop pets from each agent home, kept in step across your Macs.",
    readout: [[SCOPES.length, "rows"], [SKILL_ROWS.length, "skills"], [CATALOG_PROFILES.length, "homes"], [AGENT_PROVIDERS.length, "agents"]],
  },
  Storages: {
    lede: "Where projects and the catalog are kept: Mallard Cloud, a bucket of your own, or a folder.",
    readout: [[STORAGES.length, "storages"], [LINKS.length, "links"], [PROJECTS.length, "projects"], [1, "on the web"]],
  },
};

function PondHero({ page }: { page: Landing }) {
  const { lede, readout } = HERO[page];
  return (
    <section className="pb-hero" aria-label={page}>
      <div className="pb-hero-text">
        <h1 className="pb-title">{page}</h1>
        <p className="pb-lede">{lede}</p>
        <dl className="pb-readout">
          {readout.map(([value, label], i) => (
            <div key={label} className={`cell${i === 0 ? " is-lead" : ""}`}><dd>{value}</dd><dt>{label}</dt></div>
          ))}
        </dl>
      </div>
      <div className="pb-plate"><Pond night={theme === "dark"} /></div>
    </section>
  );
}

if (style === "book") {
  const roots = new WeakMap<HTMLElement, { root: Root; page: string }>();
  const mountHero = () => {
    const page = document.querySelector<HTMLElement>(".landing-picker");
    const title = page?.querySelector(".landing-picker-head > h1")?.textContent?.trim() as Landing | undefined;
    if (!page || !title || !(title in HERO)) return;
    let host = page.querySelector<HTMLElement>(":scope > .pb-hero-host");
    if (!host) {
      host = document.createElement("div");
      host.className = "pb-hero-host";
      page.insertBefore(host, page.firstChild);
    }
    const mounted = roots.get(host);
    if (mounted?.page === title) return;
    const root = mounted?.root ?? createRoot(host);
    roots.set(host, { root, page: title });
    root.render(<PondHero page={title} />);
  };
  new MutationObserver(mountHero).observe(document.body, { childList: true, subtree: true, characterData: true });
}
