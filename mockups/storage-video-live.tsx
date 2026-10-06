import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import App from "../src/App";
import { projectSyncApi } from "../src/components/project-sync/api";
import type { ManagedCloudSetupLogin, StorageProviderDescriptor, StorageSetupDraft, StorageSummary } from "../src/types";

/**
 * Add a storage, live: the shipped shell on a stand-in backend, for the
 * docs tutorial video (/docs/add-a-storage). `npm run dev`, then
 * /storage-video-live.html. The Mallard Cloud and Dropbox sign-ins wait on
 * `window.__advance()` so its in-browser step holds still for a capture;
 * `window.__calls` lists every backend call. Descriptors are the Rust
 * adapters' own (storage_providers/*.rs, as of 2026-10-03).
 */

const HOME = "/Users/you";
const ORIGIN = "https://mallard-ai.com";
const now = Math.floor(Date.now() / 1000);
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
const FEISHU = {
  key: "feishu", config_version: 1, display_name: "Feishu", description: "A Feishu Drive folder. Up to 100 MiB per object.",
  default_name: "Feishu storage", name_suggestion: whole("folder_name", "Feishu {1}"),
  display_rank: 25, icon: "cloud", clear_mode: "unavailable", fields: [], secrets: [], managed_secret_id: null, local_setup: "feishu",
} as unknown as StorageProviderDescriptor;
const LOCAL: StorageProviderDescriptor = {
  key: "local", config_version: 1, display_name: "Local folder", description: "A local folder or mounted drive.",
  default_name: "Local storage", name_suggestion: { source: "directory", pattern: "([^/\\\\]+)[/\\\\]*$", group: 1, template: "{1}" },
  display_rank: 30, icon: "drive", clear_mode: "revision_checked",
  fields: [{ field_id: "directory", label: "Location path", kind: "directory", required: true, placeholder: "e.g. /Volumes/backup/mallard-storage", help: null, help_url: null }],
  secrets: [], managed_secret_id: null, local_setup: null, setup_guide_url: null, manual_setup_hint: null, setup_doors: null, discovery: null,
} as StorageProviderDescriptor;
const PROVIDERS = [MALLARD, R2, DROPBOX, FEISHU, LOCAL];
const byKey = (key: string) => PROVIDERS.find((provider) => provider.key === key)!;

/* ——— state the stand-in backend keeps ——————————————————————————————— */

const config = { schema: 1, revision: 1, storages: [] as StorageSummary[], projects: [] as unknown[], links: [] as unknown[] };
const drafts = new Map<string, StorageSetupDraft>();
let draftCount = 0;

const calls: string[] = [];
const QUIET = new Set(["plugin:event|listen", "plugin:event|unlisten", "plugin:event|emit", "plugin:path|resolve_directory"]);
(window as unknown as { __calls: string[] }).__calls = calls;

function newDraft(provider: string): StorageSetupDraft {
  const descriptor = byKey(provider);
  draftCount += 1;
  const open = [...drafts.values()].filter((draft) => draft.provider === provider).length + 1;
  const draft: StorageSetupDraft = {
    draft_id: `draft-${draftCount}`, storage_id: `storage-${draftCount}`, name: `${descriptor.default_name} draft ${open}`,
    provider, config_version: 1, provider_label: descriptor.display_name, available: true, settings: {},
    secrets: descriptor.secrets.map((secret) => ({ secret_id: secret.secret_id, present: false })),
    edits_existing: false, revision: 1, created_at: now, updated_at: now,
  };
  drafts.set(draft.draft_id, draft);
  return draft;
}

function summaryOf(draft: StorageSetupDraft): StorageSummary {
  const settings = draft.settings as Record<string, string>;
  const summary = draft.provider === "managed_cloud"
    ? [
        { field_id: "account_login", label: "Account", value: settings.account_login, is_path: false },
        { field_id: "service_origin", label: "Service", value: settings.service_origin, is_path: false },
      ]
    : draft.provider === "dropbox"
      ? [
          { field_id: "folder_name", label: "Folder", value: settings.folder_name, is_path: false },
          { field_id: "namespace_id", label: "Namespace ID", value: settings.namespace_id, is_path: false },
        ]
    : draft.provider === "local"
      ? [{ field_id: "directory", label: "Location path", value: settings.directory, is_path: true }]
      : [];
  return {
    storage_id: draft.storage_id, name: draft.name, provider: draft.provider, config_version: 1, provider_label: draft.provider_label,
    icon: byKey(draft.provider).icon, available: true, summary, settings: draft.settings,
    secrets: draft.secrets.map((secret) => ({ secret_id: secret.secret_id, present: secret.present })), connection: null,
  } as StorageSummary;
}

/* `?seed=cloud` starts with the Mallard Cloud storage the video just made. */
if (new URLSearchParams(window.location.search).get("seed") === "cloud") {
  const seed = newDraft("managed_cloud");
  drafts.delete(seed.draft_id);
  config.storages = [summaryOf({
    ...seed, name: "Mallard Cloud octocat",
    settings: { service_origin: ORIGIN, github_user_id: "123", account_login: "octocat" },
    secrets: [{ secret_id: "account_token", present: true }],
  })];
}

/* The Mallard Cloud sign-in: the link arrives at once; the account lands
   when the harness says the browser step is over. */
let advance: () => void = () => {};
(window as unknown as { __advance: () => void }).__advance = () => advance();
projectSyncApi.signInManagedCloudStorage = async (draftId, _revision, _mode, onProgress) => {
  calls.push("sign_in_managed_cloud_storage");
  onProgress({ status: "authorization_link", url: `${ORIGIN}/v1/app/sign-in?state=Q2hc7v`, expires_in: 180 });
  await new Promise<void>((resolve) => { advance = resolve; });
  const draft = drafts.get(draftId)!;
  const next: StorageSetupDraft = {
    ...draft,
    settings: { service_origin: ORIGIN, github_user_id: "123", account_login: "octocat" },
    secrets: [{ secret_id: "account_token", present: true }],
    revision: draft.revision + 1, updated_at: now,
  };
  drafts.set(draftId, next);
  const login: ManagedCloudSetupLogin = { account: { github_user_id: "123", login: "octocat" }, service_origin: ORIGIN };
  return login;
};
projectSyncApi.cancelManagedCloudSetup = async () => {};

/* Dropbox, the same way: the link at once, the account when the harness
   says the browser step is over, then the folder Mallard creates. */
projectSyncApi.authorizeDropboxSetup = async (_draftId, _revision, _mode, _switchAccount, onProgress) => {
  calls.push("authorize_dropbox_setup");
  onProgress({ status: "authorization_link", url: "https://www.dropbox.com/oauth2/authorize?client_id=mallard", expires_in: 180 });
  await new Promise<void>((resolve) => { advance = resolve; });
  return { session_id: "dbx-session-1", account: { id: "dbid:octocat", name: "Octo Cat", email: "octocat@example.com" } };
};
projectSyncApi.proposeDropboxFolderName = async () => "Mallard otter";
projectSyncApi.prepareDropboxStorage = async (draftId, _revision, _session, folderName) => {
  calls.push("prepare_dropbox_storage");
  const draft = drafts.get(draftId)!;
  drafts.set(draftId, {
    ...draft,
    settings: { namespace_id: "4823017", folder_name: folderName ?? "Mallard otter" },
    secrets: [{ secret_id: "access_token", present: true }, { secret_id: "refresh_token", present: true }],
    revision: draft.revision + 1, updated_at: now,
  });
};
projectSyncApi.cancelDropboxSetup = async () => {};

(window as unknown as { isTauri: boolean }).isTauri = true;
mockWindows("main");
mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  if (!QUIET.has(command)) calls.push(command);
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "plugin:dialog|open": return "/Volumes/Backup/mallard-storage";
    case "get_host_identity_status": return { host_id: "host-1", revision: 1, created_at: now, updated_at: now };
    case "get_project_sync_config": return config;
    case "list_local_projects": return [];
    case "list_setup_drafts": return { drafts: [] };
    case "list_storage_setup_drafts": return { drafts: [...drafts.values()], warnings: [] };
    case "list_storage_providers": return PROVIDERS;
    case "list_catalog_scopes": return [];
    case "list_catalog_capabilities": return [];
    case "list_catalog_resource_profiles": return [];
    case "list_project_repository_kinds": return {};
    case "list_agent_providers": return [
      { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
    ];
    case "list_provider_profiles": return [];
    case "list_project_bindings": return [];
    case "list_agent_folders": return { folders: [] };
    case "list_managed_cloud_repositories": return [];
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
    case "create_storage_setup_draft": return newDraft(String(args.provider));
    case "get_storage_setup_draft": return drafts.get(String(args.draftId)) ?? null;
    case "update_storage_setup_draft": {
      const request = args.request as { draft_id: string; name: string; settings: Record<string, unknown> };
      const draft = drafts.get(request.draft_id)!;
      const next = { ...draft, name: request.name, settings: { ...draft.settings, ...request.settings }, revision: draft.revision + 1, updated_at: now };
      drafts.set(draft.draft_id, next as StorageSetupDraft);
      return next;
    }
    case "discard_storage_setup_draft": return drafts.delete(String(args.draftId));
    case "test_storage_setup_draft": {
      const draft = drafts.get(String(args.draftId))!;
      return { draft_id: draft.draft_id, draft_revision: draft.revision, verified: true };
    }
    case "commit_storage_setup_draft": {
      const draft = drafts.get(String(args.draftId))!;
      drafts.delete(draft.draft_id);
      config.storages = [...config.storages, summaryOf(draft)];
      config.revision += 1;
      void emit("project-sync-config-changed", { revision: config.revision });
      return { storage_id: draft.storage_id, config_revision: config.revision, replaced: false };
    }
    case "test_storage": return { storage_id: String(args.storageId), verified: true };
    default:
      console.warn("unanswered", command, args);
      return null;
  }
}, { shouldMockEvents: true });

createRoot(document.getElementById("root")!).render(<div className="harness-frame"><App /></div>);
