// @ts-nocheck
/*
 * Invented demo data for the README screenshots. Nothing here comes from a
 * real machine: one Git project "tidewatch" on a Mallard Cloud storage owned
 * by the demo account "mallard-demo". Every time is relative to now, so the
 * cards read "2 hours ago", not a fixed date.
 */

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const KB = 1024;
const MB = 1024 * KB;

export const NOW = Math.floor(Date.now() / 1000);
const ago = (seconds: number) => NOW - seconds;

export const PROJECT_ID = "project-tidewatch";
export const BUNDLE_ID = "bundle-tidewatch";
export const STORAGE_ID = "storage-mallard-cloud";
export const REPOSITORY_ID = "repo-tidewatch";
const PROJECT_ROOT = "/Users/demo/Code/tidewatch";
const HOST_ID = "5f0c2a7e91d84b3c8e6a1d2f40b7c913";
const FINGERPRINT = "git-tidewatch-fingerprint";

const recipe = { schema_version: 1, revision: 4, entries: {} };

export const registration = {
  local_project_id: PROJECT_ID,
  bundle_id: BUNDLE_ID,
  display_name: "tidewatch",
  local_alias: null,
  repository_fingerprint: FINGERPRINT,
  recipe,
  revision: 3,
  created_at: ago(24 * DAY),
  updated_at: ago(2 * HOUR),
};

export const binding = {
  host_id: HOST_ID,
  replica_id: "replica-demo-macbook",
  local_project_id: PROJECT_ID,
  bundle_id: BUNDLE_ID,
  project_root: PROJECT_ROOT,
  canonical_project_root: PROJECT_ROOT,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  state: "active",
  revision: 2,
  updated_at: ago(20 * DAY),
};

const scopeBase = (scope: string) => ({
  scope,
  scope_revision: 11,
  scope_sha256: "c".repeat(64),
  reviewed_in: { revision: 14, commit_id: "commit-14", manifest_sha256: "d".repeat(64), manifest_size: 18_204 },
  binding_revision: 2,
  last_pull_at: ago(26 * HOUR),
  last_push_at: ago(25 * HOUR),
});

export const link = {
  managed_cloud_repository_id: REPOSITORY_ID,
  local_project_id: PROJECT_ID,
  storage_id: STORAGE_ID,
  bundle_id: BUNDLE_ID,
  recipe,
  scope_bases: {
    project: scopeBase("project"),
    "provider:codex": scopeBase("provider:codex"),
    "provider:claude": scopeBase("provider:claude"),
  },
  project_content_preferences: { schema_version: 1, revision: 1, excluded_resource_ids: [] },
  conversation_checkpoints: {},
  pinned: false,
  created_at: ago(20 * DAY),
};

export const storage = {
  storage_id: STORAGE_ID,
  name: "Mallard Cloud",
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: "mallard-demo", is_path: false },
    { field_id: "service_origin", label: "Service", value: "https://mallard-ai.com", is_path: false },
  ],
  settings: {
    service_origin: "https://mallard-ai.com",
    github_user_id: "100000001",
    account_login: "mallard-demo",
  },
  secrets: [{ secret_id: "token", present: true }],
  connection: {
    state: "connected",
    code: "connected",
    message: "Connected",
    observed_at: ago(2 * MINUTE),
    evidence: null,
  },
};

export const config = {
  schema: 1,
  revision: 27,
  storages: [storage],
  projects: [registration],
  links: [link],
};

export const projectDetail = {
  project: registration,
  links: [link],
  binding,
  materializations: [],
};

export const hostIdentity = {
  host_id: HOST_ID,
  revision: 1,
  local_display_name: "Demo MacBook",
  created_at: ago(30 * DAY),
  updated_at: ago(30 * DAY),
};

export const agentProviders = [
  {
    key: "codex",
    display_name: "Codex",
    display_rank: 10,
    profile_label: ".codex profile",
    default_profile_directory: ".codex",
    setup_default: true,
    has_cli: true,
    maintenance_actions: [],
  },
  {
    key: "claude",
    display_name: "Claude",
    display_rank: 20,
    profile_label: ".claude profile",
    default_profile_directory: ".claude",
    setup_default: true,
    has_cli: true,
    maintenance_actions: [],
  },
];

const profile = (provider: string, name: string, path: string) => ({
  profile_id: `profile-${provider}`,
  provider,
  display_name: name,
  path,
  canonical_path: path,
  revision: 1,
  created_at: ago(30 * DAY),
  updated_at: ago(30 * DAY),
  available: true,
  readable: true,
  writable: true,
  used_by_projects: [PROJECT_ID],
  error: null,
});

export const profiles = [
  profile("codex", ".codex", "/Users/demo/.codex"),
  profile("claude", ".claude", "/Users/demo/.claude"),
];

/* `list_storage_providers` exactly as the production registry answers it:
   every compiled provider in display-rank order, Feishu included. The Add
   storage picker (StorageProviderPicker.tsx) is what leaves Feishu out. */
const R2_SETUP_DOC = "https://developers.cloudflare.com/r2/get-started/s3/";
const R2_BUCKET_DOC = "https://developers.cloudflare.com/r2/buckets/create-buckets/";
const R2_CREDENTIALS_DOC = "https://developers.cloudflare.com/r2/api/tokens/";
const CLOUDFLARE_ACCOUNT_ID_DOC = "https://developers.cloudflare.com/fundamentals/account/find-account-and-zone-ids/";
const R2_API_URL_PATTERN = "^https://([0-9a-fA-F]{32})(?:\\.(eu|us|fedramp))?\\.r2\\.cloudflarestorage\\.com/([^/?#%]+)$";
const DROPBOX_GUIDE = "https://www.dropbox.com/developers/reference/auth-types";
const FEISHU_GUIDE = "https://github.com/Git4Agent/mallard-app/blob/main/docs/FEISHU_APP_SETUP_GUIDE.md";
const wholeSetting = (source: string, template: string) => ({ source, pattern: "^\\s*(\\S.*?)\\s*$", group: 1, template });

export const storageProviders = [
  {
    key: "managed_cloud",
    config_version: 1,
    display_name: "Mallard Cloud",
    description: "Private snapshots with team sharing and web access.",
    default_name: "Mallard Cloud",
    name_suggestion: wholeSetting("account_login", "Mallard Cloud {1}"),
    display_rank: 5,
    icon: "cloud",
    clear_mode: "unavailable",
    fields: [],
    secrets: [{
      secret_id: "token",
      label: "Sign-in token",
      required: true,
      placeholder: "Issued by the browser sign-in",
      help: "Issued at sign-in and valid 90 days. Reconnect to renew it.",
    }],
    managed_secret_id: "token",
    local_setup: "managed_cloud",
    setup_guide_url: null,
    manual_setup_hint: null,
    setup_doors: null,
    discovery: null,
  },
  {
    key: "r2",
    config_version: 1,
    display_name: "Cloudflare R2",
    description: "Store snapshots in your R2 bucket.",
    default_name: "R2 storage",
    name_suggestion: wholeSetting("bucket", "R2 {1}"),
    display_rank: 10,
    icon: "cloud",
    clear_mode: "unconditional",
    fields: [
      { field_id: "api_url", label: "S3 API URL", kind: "url", required: true, placeholder: "Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent", help: "How to find the Cloudflare R2 S3 API URL", help_url: R2_SETUP_DOC, derive: null },
      { field_id: "bucket", label: "Bucket name", kind: "text", required: true, placeholder: "Example: codex-sync-backups", help: "How to create a Cloudflare R2 bucket", help_url: R2_BUCKET_DOC, derive: { source: "api_url", pattern: R2_API_URL_PATTERN, group: 3 } },
      { field_id: "account_id", label: "Account ID", kind: "text", required: true, placeholder: "Example: 023e105f4ecef8ad9ca31a8372d0c353", help: "Where to find your Cloudflare Account ID", help_url: CLOUDFLARE_ACCOUNT_ID_DOC, derive: { source: "api_url", pattern: R2_API_URL_PATTERN, group: 1 } },
      { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true, placeholder: "Example: 4a2b…91ef", help: "How to create an R2 Access Key ID", help_url: R2_CREDENTIALS_DOC, derive: null },
    ],
    secrets: [{ secret_id: "secret_access_key", label: "Secret Access Key", required: true, placeholder: "Example: a1B2…x9Y0", help: "How to create an R2 Secret Access Key", help_url: R2_CREDENTIALS_DOC }],
    managed_secret_id: null,
    local_setup: "wrangler",
    setup_guide_url: R2_SETUP_DOC,
    manual_setup_hint: "Enter an existing R2 connection in the fields below.",
    setup_doors: {
      create_hint: "A new bucket in your Cloudflare account.",
      existing_hint: "A bucket Mallard already uses.",
      existing_help: { label: "Set up team access", url: "https://developers.cloudflare.com/fundamentals/manage-members/manage/" },
    },
    discovery: {
      unit: "bucket",
      unit_plural: "buckets",
      place: "your Cloudflare account",
      find_label: "Find in my account",
      access_hint: "Uses your Wrangler sign-in. Read-only: lists the buckets and reads one marker file in each.",
      scanning_hint: "Looking through your Cloudflare account…",
      prepare_hint: "Cloudflare issues an access key for this bucket only.",
    },
  },
  {
    key: "dropbox",
    config_version: 1,
    display_name: "Dropbox",
    description: "A Dropbox folder. Up to 150 MiB per object.",
    default_name: "Dropbox storage",
    name_suggestion: wholeSetting("folder_name", "Dropbox {1}"),
    display_rank: 20,
    icon: "cloud",
    clear_mode: "revision_checked",
    fields: [{ field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true, placeholder: "Shared or team namespace ID", help: "Use the same namespace ID on each machine. This value cannot change after setup.", help_url: DROPBOX_GUIDE, derive: null }],
    secrets: [
      { secret_id: "access_token", label: "Access token", required: true, placeholder: "User-linked Dropbox access token", help: "For manual setup, supply account_info.read, files.metadata.read, files.content.read, and files.content.write. Replace this token when it expires. Browser setup renews access automatically.", help_url: DROPBOX_GUIDE },
      { secret_id: "refresh_token", label: "Refresh token", required: true, placeholder: "Dropbox refresh token", help: "Browser setup saves this token to renew access automatically. Show it to review the value, or enter a replacement issued for this Mallard app and Dropbox account.", help_url: DROPBOX_GUIDE },
    ],
    managed_secret_id: "refresh_token",
    local_setup: "dropbox",
    setup_guide_url: DROPBOX_GUIDE,
    manual_setup_hint: "Enter an existing shared or team namespace and a user-linked token. Mallard stores data in .mallard within that namespace.",
    setup_doors: {
      create_hint: "A new shared folder in your Dropbox.",
      existing_hint: "A shared folder Mallard already uses.",
      existing_help: null,
    },
    discovery: {
      unit: "shared folder",
      unit_plural: "shared folders",
      place: "your Dropbox",
      find_label: "Find in my Dropbox",
      access_hint: "Discovery lists accessible folders and reads their Mallard markers without changing them.",
      scanning_hint: "Approve access in your browser, then Mallard looks through your shared folders…",
      prepare_hint: "This is the connection Mallard keeps, renewed automatically.",
    },
  },
  {
    key: "feishu",
    config_version: 1,
    display_name: "Feishu",
    description: "A Feishu Drive folder. Up to 100 MiB per object.",
    default_name: "Feishu storage",
    name_suggestion: wholeSetting("folder_name", "Feishu {1}"),
    display_rank: 25,
    icon: "cloud",
    clear_mode: "unavailable",
    fields: [{ field_id: "folder_token", label: "Folder token", kind: "text", required: true, placeholder: "The Drive folder's token", help: "The Drive folder this storage lives in. Fixed after setup; share it in Feishu with each user who connects.", help_url: FEISHU_GUIDE, derive: null }],
    secrets: [],
    managed_secret_id: null,
    local_setup: "feishu",
    setup_guide_url: FEISHU_GUIDE,
    manual_setup_hint: null,
    setup_doors: {
      create_hint: "A new folder in your My Space.",
      existing_hint: "A folder shared with you in Feishu.",
      existing_help: null,
    },
    discovery: null,
  },
  {
    key: "local",
    config_version: 1,
    display_name: "Local folder",
    description: "A local folder or mounted drive.",
    default_name: "Local storage",
    name_suggestion: { source: "directory", pattern: "([^/\\\\]+)[/\\\\]*$", group: 1, template: "{1}" },
    display_rank: 30,
    icon: "drive",
    clear_mode: "revision_checked",
    fields: [{ field_id: "directory", label: "Location path", kind: "directory", required: true, placeholder: "e.g. /Volumes/backup/mallard-storage", help: null, help_url: null, derive: null }],
    secrets: [],
    managed_secret_id: null,
    local_setup: null,
    setup_guide_url: null,
    manual_setup_hint: null,
    setup_doors: null,
    discovery: null,
  },
];

export const appSettings = {
  codex_cli_path: null,
  claude_cli_path: null,
  push_codex_children: true,
  push_claude_children: true,
  credential_redaction_enabled: true,
  update_check_enabled: false,
  schema: 1,
  timeline_preview_limit_bytes: 2 * MB,
  comparison_workers: 4,
  cloud_transfer_concurrency: 4,
};

/* ------------------------------------------------------------------ */
/* Sessions and commits                                               */
/* ------------------------------------------------------------------ */

type Sync = "synced" | "local_only" | "local_ahead" | "storage_ahead" | "storage_only";

interface DemoSession {
  key: string;
  provider: "codex" | "claude";
  id: string;
  title: string;
  startedAgo: number;
  minutes: number;
  rounds: number;
  replies: number;
  tools: number;
  input: number;
  output: number;
  sync: Sync;
  bytes: number;
}

const sessions: DemoSession[] = [
  {
    key: "s1", provider: "codex", id: "019fa3c1-7d42-7a10-9b3e-2c8f5e6d0a11",
    title: "Retry tide uploads with exponential backoff",
    startedAgo: 2 * HOUR + 40 * MINUTE, minutes: 52, rounds: 6, replies: 14, tools: 38,
    input: 1_284_500, output: 31_900, sync: "local_ahead", bytes: 2_310 * KB,
  },
  {
    key: "s2", provider: "claude", id: "8c41d2e6-3b7a-4f19-a2d8-6e0b9c5f1a74",
    title: "Investigate dropped uploads on hotel Wi-Fi",
    startedAgo: 4 * HOUR + 5 * MINUTE, minutes: 47, rounds: 9, replies: 21, tools: 44,
    input: 912_400, output: 27_300, sync: "local_only", bytes: 1_640 * KB,
  },
  {
    key: "s3", provider: "codex", id: "019fa1b8-2e93-7c55-8f01-4d7a3b9e6c22",
    title: "Queue tide readings while offline",
    startedAgo: 1 * DAY + 3 * HOUR, minutes: 74, rounds: 8, replies: 19, tools: 61,
    input: 2_105_800, output: 48_200, sync: "synced", bytes: 3_020 * KB,
  },
  {
    key: "s4", provider: "claude", id: "2f7e9a10-c4b5-4d63-9e8a-71b0d3c5f2e8",
    title: "Review offline queue edge cases",
    startedAgo: 1 * DAY + 1 * HOUR, minutes: 38, rounds: 5, replies: 12, tools: 23,
    input: 604_100, output: 15_800, sync: "storage_only", bytes: 1_180 * KB,
  },
  {
    key: "s5", provider: "codex", id: "019fa1d4-6a17-7e02-b3c9-8e5f1a2d7b33",
    title: "Write tests for the reading queue",
    startedAgo: 22 * HOUR, minutes: 29, rounds: 4, replies: 9, tools: 27,
    input: 538_900, output: 12_400, sync: "local_ahead", bytes: 890 * KB,
  },
  {
    key: "s6", provider: "claude", id: "a93d5c27-81e4-4b0f-b6d2-0f4e8a1c7d59",
    title: "Tide chart shows the wrong hour after DST",
    startedAgo: 3 * DAY + 2 * HOUR, minutes: 41, rounds: 7, replies: 16, tools: 32,
    input: 776_300, output: 19_600, sync: "synced", bytes: 1_420 * KB,
  },
  {
    key: "s7", provider: "codex", id: "019f9c4e-1b28-7d90-a4e6-5c3b8f2a9d44",
    title: "Add moon phase next to the tide table",
    startedAgo: 5 * DAY + 4 * HOUR, minutes: 63, rounds: 7, replies: 17, tools: 49,
    input: 1_633_200, output: 36_700, sync: "synced", bytes: 2_540 * KB,
  },
  {
    key: "s8", provider: "claude", id: "d6b1f8a3-5e02-4c97-8a1d-3f9e7b2c6a05",
    title: "Smooth tide predictions between stations",
    startedAgo: 5 * DAY + 1 * HOUR, minutes: 35, rounds: 6, replies: 13, tools: 25,
    input: 688_000, output: 17_100, sync: "storage_only", bytes: 1_260 * KB,
  },
  {
    key: "s9", provider: "codex", id: "019f97a2-4c61-7b13-9d58-2a6e1f4b8c55",
    title: "Scaffold Expo app with TypeScript",
    startedAgo: 9 * DAY + 3 * HOUR, minutes: 46, rounds: 5, replies: 11, tools: 34,
    input: 942_700, output: 22_800, sync: "synced", bytes: 1_730 * KB,
  },
];

const commits = [
  { sha: "7c1e94b2d0a35f68e9b14c7a2d5f3e8b61a09c47", ago: 1 * HOUR + 35 * MINUTE, subject: "Retry uploads on flaky Wi-Fi", refs: ["s1", "s2"] },
  { sha: "e42b8f1c9a6d07e35b2c4f8a1d9e6b3c70f5a218", ago: 20 * HOUR, subject: "Add offline queue for tide readings", refs: ["s3", "s4", "s5"] },
  { sha: "3a9d6e2f8b14c75a0e3d9f6b2c8a4e1d57b93f60", ago: 3 * DAY, subject: "Fix timezone in tide chart", refs: ["s6"] },
  { sha: "b5f2a8c4e1d93b07f6a2e8c5d1b4f9a36e07c812", ago: 4 * DAY + 20 * HOUR, subject: "Show moon phase next to tide table", refs: ["s7", "s8"] },
  { sha: "0d8e3b6a9c2f41e7b5d0a3c8f6e2b9d14a7c5e93", ago: 9 * DAY, subject: "Set up Expo project and CI", refs: ["s9"] },
];

const conversationId = (session: DemoSession) => `${session.provider}:conversation:${session.id}`;
const captureId = (session: DemoSession) => `${session.provider}:session:${session.id}`;
const providerName = (provider: string) => (provider === "codex" ? "Codex" : "Claude");
const endedAt = (session: DemoSession) => ago(session.startedAgo) + session.minutes * MINUTE;
const localHere = (session: DemoSession) => session.sync !== "storage_only";

function sessionActions(session: DemoSession) {
  const enabled = localHere(session);
  const reason = enabled ? null : "This session is not on this computer yet.";
  return [
    {
      action_id: `${session.provider}.open_in_app`,
      kind: "open_in_app",
      label: `Open in ${providerName(session.provider)}`,
      enabled,
      disabled_reason: reason,
    },
    {
      action_id: `${session.provider}.open_in_terminal`,
      kind: "open_in_terminal",
      label: "Open in Terminal",
      enabled,
      disabled_reason: reason,
    },
  ];
}

function sessionSummary(session: DemoSession) {
  const local = localHere(session);
  const source = session.sync === "storage_only"
    ? "remote_only"
    : session.sync === "local_only" ? "local_only" : "local_and_remote";
  const ended = endedAt(session);
  return {
    resource_id: conversationId(session),
    provider: session.provider,
    provider_display_name: providerName(session.provider),
    provider_session_id: session.id,
    logical_thread_id: session.id,
    source,
    access: "bound_ready",
    title: session.title,
    preserved: null,
    summary: null,
    started_at: ago(session.startedAgo),
    ended_at: ended,
    last_prompt_at: ended - 4 * MINUTE,
    transcript_path: local
      ? session.provider === "codex"
        ? `/Users/demo/.codex/sessions/rollout-${session.id}.jsonl`
        : `/Users/demo/.claude/projects/-Users-demo-Code-tidewatch/${session.id}.jsonl`
      : null,
    working_directory: PROJECT_ROOT,
    sync_blocked_reason: null,
    recorded_sha: null,
    is_active: false,
    user_round_count: session.rounds,
    agent_message_count: session.replies,
    tool_call_count: session.tools,
    total_input_tokens: session.input,
    total_output_tokens: session.output,
    total_tokens: session.input + session.output,
    metrics_complete: true,
    actions: sessionActions(session),
  };
}

function decisions(sync: Sync) {
  switch (sync) {
    case "synced":
      return { push: { action: "no_changes", selectable: false }, pull: { action: "no_changes", selectable: false } };
    case "local_only":
      return { push: { action: "create_cloud", selectable: true }, pull: { action: "report_local_only", selectable: false } };
    case "local_ahead":
      return { push: { action: "publish", selectable: true }, pull: { action: "report_push_eligible", selectable: false } };
    case "storage_ahead":
      return { push: { action: "require_pull", selectable: false }, pull: { action: "materialize_cloud", selectable: true } };
    case "storage_only":
      return { push: { action: "not_pushable", selectable: false }, pull: { action: "create_local", selectable: true } };
  }
}

function comparisonEntry(session: DemoSession) {
  const local = localHere(session);
  const stored = session.sync !== "local_only";
  const { push, pull } = decisions(session.sync);
  const ended = endedAt(session);
  return {
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider: session.provider,
    provider_display_name: providerName(session.provider),
    access: "bound_ready",
    thread_id: session.id,
    conversation_resource_id: conversationId(session),
    resource_id: local ? captureId(session) : conversationId(session),
    display_name: session.title,
    state: session.sync,
    local_present: local,
    storage_present: stored,
    status_detail: null,
    status_detail_code: null,
    status_detail_value: null,
    local_updated_at: local ? (session.sync === "storage_ahead" ? ended - 50 * MINUTE : ended) : null,
    storage_updated_at: stored ? (session.sync === "local_ahead" ? ended - 35 * MINUTE : ended) : null,
    cloud_thread_id: stored ? `cloud-${session.id}` : null,
    forked_from: null,
    compared_against_storage: true,
    provider_thread_id: session.id,
    capture_bytes: local ? session.bytes : null,
    pull_bytes: stored ? session.bytes : null,
    push_decision: push,
    pull_decision: pull,
  };
}

export const managedCloudStatus = {
  repository_id: REPOSITORY_ID,
  state: "ready",
  link: {
    visibility: "private",
    can_pull: true,
    repository_id: REPOSITORY_ID,
    website_url: "https://mallard-ai.com/mallard-demo/tidewatch",
    owner_login: "mallard-demo",
    name: "tidewatch",
    release: { kind: "private" },
    repository_revision: 14,
    can_request_release: true,
    can_publish_version: true,
    can_discard_draft: true,
    role: "owner",
    can_push: true,
  },
  publication: {
    draft: "pushed",
    latest_version: 3,
    next_version: 4,
    draft_ahead: 2,
    public_version: null,
    review_version: null,
  },
  message: null,
};

const counts = sessions.reduce((tally, session) => {
  if (session.sync === "synced") tally.synced += 1;
  else if (session.sync === "local_only" || session.sync === "local_ahead") tally.local += 1;
  else tally.storage += 1;
  return tally;
}, { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 });

export function comparison() {
  return {
    managed_cloud_status: managedCloudStatus,
    project_id: PROJECT_ID,
    storage_id: STORAGE_ID,
    storage_name: storage.name,
    revision: 14,
    base_revision: 14,
    compared_at: ago(30),
    entries: sessions.map(comparisonEntry),
    counts,
    scopes: [],
    provider_errors: {},
    warnings: [],
    capture_budget: { limit_bytes: 512 * MB, reserved_bytes: 96 * KB },
    remote_continuity: { state: "in_sync", revision: 14 },
  };
}

const sessionByKey = new Map(sessions.map((session) => [session.key, session]));

export function activity() {
  return {
    project_id: PROJECT_ID,
    sessions: sessions.map(sessionSummary),
    git: {
      selected_branch: "main",
      branches: [
        { name: "main", is_current: true },
        { name: "feature/tide-alerts", is_current: false },
      ],
      github_repository_url: null,
      commits: commits.map((commit) => ({
        sha: commit.sha,
        short_sha: commit.sha.slice(0, 7),
        committed_at: ago(commit.ago),
        subject: commit.subject,
        session_refs: commit.refs.map((key) => ({ resource_id: conversationId(sessionByKey.get(key)) })),
      })),
      unique_session_count: sessions.length,
    },
    unmapped: [],
    providers: [
      { provider: "codex", display_name: "Codex", access: "bound_ready", detail: null },
      { provider: "claude", display_name: "Claude", access: "bound_ready", detail: null },
    ],
    warnings: [],
    window_start: ago(30 * DAY),
    window_end: NOW,
    older: null,
    storage_sync: [{
      storage_id: STORAGE_ID,
      storage_name: storage.name,
      last_pull_at: ago(26 * HOUR),
      last_push_at: ago(25 * HOUR),
    }],
  };
}

function conversationResource(session: DemoSession) {
  return {
    resource_id: captureId(session),
    kind: { key: "conversation", version: 1 },
    provider: session.provider,
    provider_resource_type: "session",
    display_name: session.title,
    provenance: { kind: "conversation", version: 1, fields: {} },
    apply_policy: { key: "safe_file", version: 1 },
    codec_version: 1,
    attributes: {},
    category: "conversations",
    kind_label: "Conversation",
    apply_policy_label: "Safe file",
    selectable: true,
    default_selected: false,
    blocked_reason: null,
  };
}

export function inventory() {
  return {
    project: PROJECT_ID,
    bundle_id: BUNDLE_ID,
    resources: sessions.filter(localHere).map(conversationResource),
    recipe,
    generated_at: NOW,
    warnings: [],
  };
}

export function pushReviewContext(scope) {
  const history = activity().git;
  return {
    scope,
    credential_redaction_enabled: true,
    project: projectDetail,
    inventory: inventory(),
    destination_status: { state: "ready", revision: 14 },
    thread_comparison: comparison(),
    thread_comparison_error: null,
    project_files: null,
    project_files_error: null,
    git_history: {
      branch: "main",
      head_sha: commits[0].sha,
      commits: history.commits.map((commit) => ({
        sha: commit.sha,
        committed_at: commit.committed_at,
        subject: commit.subject,
        session_refs: commit.session_refs.map((ref) => ref.resource_id),
      })),
      uncommitted_session_refs: [],
      outside_history_session_refs: [],
      truncated: false,
    },
    git_history_review_token: "git-review-token-demo",
    git_history_changed: true,
    git_history_outgoing: { new_commits: 1, rewritten: false },
    prepared_at: NOW,
  };
}

function reviewRow(session: DemoSession, index: number) {
  const { pull } = decisions(session.sync);
  const actionable = pull.selectable;
  return {
    stored_without_credential_redaction: false,
    provider: session.provider,
    thread_id: localHere(session) ? session.id : null,
    cloud_thread_id: session.sync === "local_only" ? null : `cloud-${session.id}`,
    resource_id: comparisonEntry(session).resource_id,
    action: pull.action,
    blocker: null,
    blocked_reason: null,
    action_id: actionable ? `pull-action-${index}` : null,
    display_title: session.title,
    source_size: actionable ? session.bytes : null,
    stored_ended_at: session.sync === "local_only" ? null : endedAt(session),
  };
}

export function pullReview(scope) {
  return {
    apply_limit_bytes: 512 * MB,
    scope,
    schema_version: 1,
    plan_id: "plan-tidewatch-demo",
    storage_id: STORAGE_ID,
    bundle_id: BUNDLE_ID,
    host_id: HOST_ID,
    replica_id: binding.replica_id,
    expected_host_identity_revision: 1,
    revision: 14,
    commit_id: "commit-14",
    manifest_sha256: "d".repeat(64),
    binding_revision: 2,
    created_at: NOW,
    expires_at: NOW + DAY,
    actions: [],
    conversation_review: sessions.map(reviewRow),
    project_content_eligibility: {
      state: "git_managed",
      reason: "Project files travel through Git.",
      detected_root: PROJECT_ROOT,
    },
    remote_continuity: { state: "in_sync", revision: 14 },
  };
}

export const readiness = {
  bundle_id: BUNDLE_ID,
  state: "ready",
  issues: [],
  generated_at: NOW,
};

/* ------------------------------------------------------------------ */
/* Add a project: the same machine before tidewatch is a project.     */
/* ------------------------------------------------------------------ */

export const freshConfig = { ...config, revision: 21, projects: [], links: [] };
export const freshProfiles = profiles.map((entry) => ({ ...entry, used_by_projects: [] }));

const folderProbe = (path: string, git: boolean, draftId: string | null = null) => ({
  kind: "folder",
  canonical_path: path,
  is_git_repository: git,
  existing_draft_id: draftId,
  existing_project_ids: [],
});

const agentFolder = (path: string, providers: string[], lastActiveAgo: number, git = true) => ({
  path,
  providers,
  last_active: ago(lastActiveAgo),
  probe: folderProbe(path, git),
});

export const agentFolders = {
  folders: [
    agentFolder(PROJECT_ROOT, ["codex", "claude"], 2 * HOUR + 10 * MINUTE),
    agentFolder("/Users/demo/Code/tidewatch-api", ["codex"], 26 * HOUR),
    agentFolder("/Users/demo/Code/harbor-charts", ["claude", "codex"], 3 * DAY),
    agentFolder("/Users/demo/Notes/sailing-log", ["claude"], 6 * DAY, false),
    agentFolder("/Users/demo/Code/dotfiles", ["codex"], 12 * DAY),
  ],
};

export function probeFolder(path: string, draftId: string | null) {
  const known = agentFolders.folders.find((folder) => folder.path === path);
  return folderProbe(path, known ? known.probe.is_git_repository : true, path === PROJECT_ROOT ? draftId : null);
}

export const DRAFT_ID = "draft-tidewatch";

export function setupDraft(revision = 0) {
  return {
    schema: 1,
    draft_id: DRAFT_ID,
    local_project_id: PROJECT_ID,
    new_bundle_id: BUNDLE_ID,
    project_root: PROJECT_ROOT,
    canonical_project_root: PROJECT_ROOT,
    display_name: "tidewatch",
    repository_fingerprint: FINGERPRINT,
    profiles: {
      codex: { kind: "existing", profile_id: "profile-codex" },
      claude: { kind: "existing", profile_id: "profile-claude" },
    },
    storage: { storage_id: STORAGE_ID },
    repository: { kind: "new" },
    revision,
    created_at: ago(MINUTE),
    updated_at: ago(MINUTE),
    last_error: null,
  };
}

export function draftSummary(draft) {
  return {
    draft_id: draft.draft_id,
    display_name: draft.display_name,
    project_root: draft.project_root,
    updated_at: draft.updated_at,
    revision: draft.revision,
    status: "draft",
    last_error: null,
  };
}

export function draftInspection(draft) {
  return {
    draft,
    sections: [
      { section: "project", state: "ready", message: null },
      { section: "profiles", state: "ready", message: null },
      { section: "storage", state: "ready", message: null },
      { section: "repository", state: "ready", message: null },
    ],
    can_finalize: true,
  };
}
