import { useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import App from "../src/App";
import type {
  InventoryResource,
  ProjectContentEntry,
  ProjectContentInventory,
  ProjectResourceDescriptor,
  ThreadSyncEntry,
} from "../src/types";

/**
 * Non-Git Push, live: the shipped shell on a stand-in backend that holds one
 * non-Git project, a local-folder storage, two sessions and a folder of
 * files. It answers the scan and the push the way the backend does — a push
 * without the scan's token leaves stored files alone, and only a shown list
 * remembers the files left out — and logs every push it receives.
 * `npm run dev`, then /mockups/push-files-live.html (`?scan=slow|fail`).
 */

const HOME = "/Users/you";
const ROOT = `${HOME}/notes/physics-225`;
const now = Math.floor(Date.now() / 1000);
const query = new URLSearchParams(window.location.search);
const scanMode = query.get("scan") ?? "fast";

const PROJECT_ID = "project-physics";
const STORAGE_ID = "storage-drive";
const BUNDLE_ID = "c".repeat(32);
const KB = 1024;
const MB = 1024 * KB;

const STORAGE = {
  storage_id: STORAGE_ID, name: "Backup drive", provider: "local", config_version: 1, provider_label: "Local folder", icon: "drive",
  available: true, summary: [{ field_id: "path", label: "Folder", value: "/Volumes/Backup/mallard", is_path: true }],
  settings: { path: "/Volumes/Backup/mallard" }, secrets: [],
};

/* ——— the folder, storage, and what this link remembers ——————————————— */

interface File { path: string; size: number; version: number; warning?: "executable" | "possible_secret" }
const folder: File[] = [
  { path: "README.md", size: 2.1 * KB, version: 1 },
  { path: ".env", size: 0.4 * KB, version: 1, warning: "possible_secret" },
  { path: "notes/plan.md", size: 6.4 * KB, version: 1 },
  { path: "notes/todo.md", size: 1.8 * KB, version: 1 },
  { path: "assets/cover.png", size: 48 * KB, version: 1 },
  { path: "data/sample.csv", size: 31 * KB, version: 1 },
  { path: "scripts/run.sh", size: 1.1 * KB, version: 1, warning: "executable" },
];
const stored = new Map<string, number>();
const recipe = new Set<string>();
const leftOut = new Set<string>();
let scans = 0;
let revision = 1;

const fileId = (path: string) => `project:file:${path}`;
const dirId = (path: string) => `project:directory:${path}`;
const digest = (file: File) => `${file.path}@${file.version}`.padEnd(64, "0");

function descriptor(resourceId: string, type: "file" | "directory", path: string): ProjectResourceDescriptor {
  return {
    resource_id: resourceId,
    kind: { key: type === "file" ? "project_content_file" : "project_content_directory", version: 1 },
    provider: null,
    display_name: path.split("/").slice(-1)[0],
    provenance: { kind: "project_local", version: 1, fields: { relative_path: path } },
    apply_policy: { key: "explicit_review", version: 1 },
    codec_version: 1,
    attributes: {},
  };
}

function entryFor(resourceId: string, type: "file" | "directory", path: string, file?: File): ProjectContentEntry {
  const inStorage = stored.has(resourceId);
  const local = file ? digest(file) : path.padEnd(64, "d");
  const remote = inStorage ? (file ? `${file.path}@${stored.get(resourceId)}`.padEnd(64, "0") : local) : null;
  const inRecipe = recipe.has(resourceId);
  return {
    descriptor: descriptor(resourceId, type, path),
    entry_type: type,
    relative_path: path,
    logical_path: `project/${path}`,
    size: file?.size ?? null,
    mode: file?.warning === "executable" ? 0o755 : type === "directory" ? 0o755 : 0o644,
    source_mtime: now - 3600,
    state: !inStorage ? "local_only" : remote === local ? "synced" : "local_ahead",
    local_present: true,
    storage_present: inStorage,
    base_present: inStorage,
    local_digest: local,
    storage_digest: remote,
    base_digest: remote,
    review_digest: local,
    selected_in_recipe: inRecipe,
    newly_discovered: !inStorage && !inRecipe,
    undecided: !inStorage && !inRecipe && !leftOut.has(resourceId),
    blocked_reason: null,
    warning_code: file?.warning ?? null,
    warning_digest: file?.warning ? `warning:${digest(file)}` : null,
  };
}

function scan(): ProjectContentInventory {
  scans += 1;
  const directories = [...new Set(folder.flatMap((file) => {
    const parts = file.path.split("/").slice(0, -1);
    return parts.map((_, index) => parts.slice(0, index + 1).join("/"));
  }))];
  const entries = [
    ...directories.map((path) => entryFor(dirId(path), "directory", path)),
    ...folder.map((file) => entryFor(fileId(file.path), "file", file.path, file)),
  ].sort((left, right) => left.relative_path.localeCompare(right.relative_path));
  return {
    local_project_id: PROJECT_ID,
    storage_id: STORAGE_ID,
    project_root: ROOT,
    eligibility: { state: "eligible", reason: "This project folder is not inside a Git work tree." },
    review_token: `scan-${scans}:${folder.map((file) => file.version).join("")}`,
    storage_revision: revision,
    preference_revision: leftOut.size,
    excluded_resource_ids: [...leftOut],
    entries,
    ignored_count: 3,
    blocked_count: 0,
    warnings: [],
    scanned_at: now,
  };
}

/* ——— sessions ——————————————————————————————————————————————————————— */

const SESSIONS = [
  { id: "019f7798-5437-7632-9dbc-5b589cf68b10", title: "Locate mallard-release folder", bytes: 1.4 * MB, at: now - 240 },
  { id: "019f7798-5437-7632-9dbc-5b589cf68b11", title: "Locate and move Feishu env", bytes: 415 * KB, at: now - 660 },
];
const pushedSessions = new Set<string>();
const sessionResourceId = (id: string) => `codex:session:${id}`;

function sessionEntry(session: (typeof SESSIONS)[number]): ThreadSyncEntry {
  const pushed = pushedSessions.has(session.id);
  return {
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    thread_id: session.id,
    conversation_resource_id: `codex:conversation:${session.id}`,
    resource_id: sessionResourceId(session.id),
    display_name: session.title,
    state: pushed ? "synced" : "local_only",
    local_present: true,
    storage_present: pushed,
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    local_updated_at: session.at,
    push_decision: pushed ? { action: "no_changes", selectable: false } : { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
    capture_bytes: session.bytes,
  };
}

function sessionResource(session: (typeof SESSIONS)[number]): InventoryResource {
  return {
    resource_id: sessionResourceId(session.id),
    kind: { key: "conversation", version: 1 },
    provider: "codex",
    display_name: session.title,
    provenance: { kind: "provider_home", version: 1, fields: {} },
    apply_policy: { key: "explicit_review", version: 1 },
    codec_version: 1,
    attributes: {},
    category: "conversations",
    kind_label: "Session",
    apply_policy_label: "Explicit review",
    selectable: true,
    default_selected: true,
    logical_paths: [],
  };
}

/* ——— the stand-in backend ————————————————————————————————————————————— */

const registration = {
  local_project_id: PROJECT_ID, bundle_id: BUNDLE_ID, display_name: "physics 225", local_alias: null, repository_fingerprint: null,
  recipe: { schema_version: 1, revision: 1, entries: {} }, revision: 1, created_at: now, updated_at: now,
};
const link = () => ({
  local_project_id: PROJECT_ID, storage_id: STORAGE_ID, bundle_id: BUNDLE_ID,
  recipe: recipe.size > 0 ? {
    schema_version: 1,
    revision,
    entries: Object.fromEntries([...recipe].map((id) => [id, { resource_id: id, apply_policy: { key: "explicit_review", version: 1 }, required: false }])),
  } : null,
  project_content_preferences: { schema_version: 1, revision: leftOut.size, excluded_resource_ids: [...leftOut] },
  conversation_checkpoints: {}, pinned: false, created_at: now,
});
const config = () => ({ schema: 1, revision, storages: [STORAGE], projects: [registration], links: [link()] });
const BINDING = {
  host_id: "host-1", replica_id: "replica-1", local_project_id: PROJECT_ID, bundle_id: BUNDLE_ID, project_root: ROOT,
  canonical_project_root: ROOT, profile_ids: { codex: "p-codex" }, state: "active", revision: 1, updated_at: now,
};
const detail = () => ({ project: registration, links: [link()], binding: BINDING, materializations: [] });
// The review's own copy carries no binding: it then lists sessions from its
// inventory, and this backend has no activity history to serve.
const reviewDetail = () => ({ ...detail(), binding: null });

const log: string[] = [];
let render = () => {};
const note = (line: string) => {
  log.push(line);
  render();
};
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));
const QUIET = new Set(["plugin:event|listen", "plugin:event|unlisten", "plugin:event|emit", "plugin:path|resolve_directory"]);

interface PushArgs {
  recipe: { entries: Record<string, unknown> };
  projectContentReviewToken: string | null;
  projectFilesShown: boolean;
  acknowledgedWarningDigests: string[];
}

(window as unknown as { isTauri: boolean }).isTauri = true;
// The shell guards its window's close while a sync writes.
mockWindows("main");
mockIPC(async (command, payload) => {
  if (!QUIET.has(command)) note(command);
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "get_host_identity_status": return { host_id: "host-1", revision: 1, created_at: now, updated_at: now };
    case "get_project_sync_config": return config();
    case "list_local_projects": return [registration];
    case "get_project": return detail();
    case "list_setup_drafts": return { drafts: [] };
    case "list_storage_setup_drafts": return { drafts: [], warnings: [] };
    case "list_catalog_scopes": return [];
    case "list_catalog_capabilities": return [];
    case "list_catalog_resource_profiles": return [];
    case "list_project_repository_kinds": return { [PROJECT_ID]: false };
    case "list_agent_providers": return [
      { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
    ];
    case "list_storage_providers": return [];
    case "list_provider_profiles": return [{
      profile_id: "p-codex", provider: "codex", display_name: "~/.codex", path: `${HOME}/.codex`, canonical_path: `${HOME}/.codex`,
      revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
    }];
    case "list_project_bindings": return [BINDING];
    case "get_project_activity": return {
      project_id: PROJECT_ID, sessions: [], git: null, unmapped: [], providers: [], warnings: [],
      window_start: now - 30 * 86_400, window_end: now, older: null, storage_sync: [],
    };
    case "list_agent_folders": return { folders: [] };
    case "take_app_link": return null;
    case "prepare_push_review": {
      await wait(200);
      return {
        scope: { kind: "project" },
        credential_redaction_enabled: false,
        project: reviewDetail(),
        inventory: {
          project: "physics 225",
          bundle_id: BUNDLE_ID,
          resources: SESSIONS.map(sessionResource),
          recipe: { schema_version: 1, revision: 1, entries: {} },
        },
        destination_status: { state: "ready", revision },
        thread_comparison: {
          project_id: PROJECT_ID,
          storage_id: STORAGE_ID,
          storage_name: STORAGE.name,
          revision,
          base_revision: revision,
          compared_at: now,
          entries: SESSIONS.map(sessionEntry),
          counts: { synced: pushedSessions.size, local: SESSIONS.length - pushedSessions.size, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
          scopes: [],
          provider_errors: {},
          warnings: [],
          capture_budget: { limit_bytes: 512 * MB, reserved_bytes: 21 * KB },
          remote_continuity: { state: "in_sync", revision },
        },
        thread_comparison_error: null,
        project_files: null,
        project_files_error: null,
        git_history: null,
        git_history_review_token: null,
        git_history_changed: false,
        git_history_outgoing: null,
        prepared_at: now,
      };
    }
    case "inspect_project_files": {
      await wait(scanMode === "slow" ? 2500 : 250);
      if (scanMode === "fail") throw "walk '/Users/you/notes/physics-225': permission denied";
      return scan();
    }
    case "push_bundle": {
      const args = payload as unknown as PushArgs;
      const pushedIds = Object.keys(args.recipe.entries);
      const token = args.projectContentReviewToken;
      const carried: string[] = [];
      if (token) {
        // The backend walks the folder only with the scan's token.
        for (const file of folder) {
          const id = fileId(file.path);
          if (pushedIds.includes(id) || (recipe.has(id) && stored.has(id))) {
            if (stored.get(id) !== file.version) carried.push(file.path);
            stored.set(id, file.version);
            recipe.add(id);
            leftOut.delete(id);
          } else if (args.projectFilesShown) {
            leftOut.add(id);
          }
        }
        for (const id of pushedIds.filter((candidate) => candidate.startsWith("project:directory:"))) {
          stored.set(id, 1);
          recipe.add(id);
        }
      }
      SESSIONS.filter((session) => pushedIds.includes(sessionResourceId(session.id))).forEach((session) => pushedSessions.add(session.id));
      revision += 1;
      note(`— push: token ${token ? "sent" : "none"} · files shown ${args.projectFilesShown} · files captured ${carried.length ? carried.join(", ") : "none"} · left out ${leftOut.size}`);
      return { success: true, message: "Pushed", resources_changed: pushedIds.length, revision, results: pushedIds.map((id) => ({ resource_id: id, state: "synced" })) };
    }
    default:
      return null;
  }
}, { shouldMockEvents: true });

function Harness() {
  const [, setTick] = useState(0);
  render = () => setTick((tick) => tick + 1);
  return (
    <div className="harness">
      <div className="harness-controls">
        <strong>The folder</strong>
        <button type="button" data-edit onClick={() => { folder.find((file) => file.path === "notes/plan.md")!.version += 1; note("— edited notes/plan.md"); }}>Edit notes/plan.md</button>
        <button type="button" data-add onClick={() => { folder.push({ path: `notes/idea-${folder.length}.md`, size: 3 * KB, version: 1 }); note("— added a new file"); }}>Add a file</button>
        <span>scan={scanMode} · in storage {[...stored.keys()].filter((id) => id.startsWith("project:file:")).length} · left out {leftOut.size}</span>
        <button type="button" onClick={() => { log.length = 0; render(); }}>Clear log</button>
      </div>
      <div className="harness-frame"><App /></div>
      <pre className="harness-log" aria-live="polite">{log.length ? log.join("\n") : "Backend calls appear here."}</pre>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Harness />);
