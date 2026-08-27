import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
  ProjectChatHistoryContent,
  syncHoldDetail,
} from "../../src/components/project-sync/ProjectActivityPage";
import type {
  ConversationBlocker,
  ConversationReviewAction,
  ThreadSyncDecision,
  ThreadSyncEntry,
} from "../../src/types";

const project = {
  local_project_id: "project-mallard",
  bundle_id: "bundle-mallard",
  display_name: "mallard",
  local_alias: "Mallard local",
  repository_fingerprint: "fingerprint",
  project_root: "/Users/test/projects/mallard",
  canonical_project_root: "/Users/test/projects/mallard",
  profile_ids: { codex: "profile-codex" },
  profile_names: ["Default Codex"],
  created_at: 1,
  updated_at: 2,
  revision: 1,
};

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
};

const WINDOW_START = 1_750_000_000;
const HEALTHY_ID = "019f7798-5437-7632-9dbc-5b589cf68bf0";
const BLOCKED_ID = "019f7798-5437-7632-9dbc-5b589cf68bf1";
const DIVERGED_ID = "019f7798-5437-7632-9dbc-5b589cf68bf2";
const STORED_ID = "019f7798-5437-7632-9dbc-5b589cf68bf3";
const OLDER_ID = "019f7798-5437-7632-9dbc-5b589cf68bf4";

function localSession(threadId: string, title: string) {
  return {
    resource_id: `codex:conversation:${threadId}`,
    provider: "codex",
    provider_display_name: "Codex",
    provider_session_id: threadId,
    logical_thread_id: threadId,
    source: "local_only" as const,
    access: "bound_ready" as const,
    title,
    preserved: null,
    summary: "",
    started_at: WINDOW_START + 1_000,
    ended_at: WINDOW_START + 2_000,
    working_directory: null,
    branch: "main",
    recorded_sha: "a".repeat(40),
    is_active: false,
    user_round_count: 2,
    agent_message_count: 3,
    tool_call_count: 4,
    total_tokens: 1_200,
    metrics_complete: true,
    actions: [
      {
        action_id: "codex.open-in-app",
        kind: "open_in_app" as const,
        label: "Open in Codex",
        enabled: true,
        disabled_reason: null,
      },
      {
        action_id: "codex.open-in-terminal",
        kind: "open_in_terminal" as const,
        label: "Open in Terminal",
        enabled: true,
        disabled_reason: null,
      },
    ],
  };
}

const activity = {
  project_id: project.local_project_id,
  sessions: [
    localSession(HEALTHY_ID, "Test task"),
    localSession(BLOCKED_ID, "Older codec task"),
    localSession(DIVERGED_ID, "Changed on both machines"),
  ],
  git: null,
  unmapped: [],
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" as const }],
  warnings: [],
  window_start: WINDOW_START,
  window_end: WINDOW_START + 3_000_000,
  next_before: null,
  storage_sync: [],
};

function decision(
  action: ConversationReviewAction,
  blocker: ConversationBlocker | null = null,
  selectable = false,
): ThreadSyncDecision {
  return { action, blocker, selectable };
}

function entry(overrides: Partial<ThreadSyncEntry> & Pick<ThreadSyncEntry, "thread_id">): ThreadSyncEntry {
  return {
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    conversation_resource_id: `codex:conversation:${overrides.thread_id}`,
    resource_id: `codex:conversation:${overrides.thread_id}`,
    display_name: overrides.thread_id,
    state: "local_only",
    local_present: true,
    storage_present: false,
    cloud_thread_id: overrides.thread_id,
    compared_against_storage: true,
    push_decision: decision("create_cloud", null, true),
    pull_decision: decision("report_local_only"),
    ...overrides,
  };
}

const healthyEntry = entry({
  thread_id: HEALTHY_ID,
  display_name: "Test task",
  local_updated_at: WINDOW_START + 2_000,
});
const blockedEntry = entry({
  thread_id: BLOCKED_ID,
  display_name: "Older codec task",
  local_updated_at: WINDOW_START + 2_000,
  push_decision: decision("blocked", "codec_unsupported"),
});
const divergedEntry = entry({
  thread_id: DIVERGED_ID,
  display_name: "Changed on both machines",
  state: "diverged",
  storage_present: true,
  local_updated_at: WINDOW_START + 2_000,
  storage_updated_at: WINDOW_START + 1_500,
  push_decision: decision("require_pull"),
  pull_decision: decision("keep_both", null, true),
});
const storedEntry = entry({
  thread_id: STORED_ID,
  display_name: "Improve cloud task titles",
  state: "storage_only",
  local_present: false,
  storage_present: true,
  storage_updated_at: WINDOW_START + 1_000,
  push_decision: decision("not_pushable"),
  pull_decision: decision("create_local", null, true),
});
const olderEntry = entry({
  thread_id: OLDER_ID,
  display_name: "Older unpublished task",
  local_updated_at: WINDOW_START - 5_000,
});

function comparisonOf(entries: ThreadSyncEntry[]) {
  return {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    compared_at: WINDOW_START + 4_000,
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    remote_continuity: { state: "in_sync" as const, revision: 1 },
    entries,
  };
}

function render(props: {
  selectionMode?: "push" | "pull";
  entries: ThreadSyncEntry[];
  selectableResourceIds?: ReadonlySet<string>;
  selectedResourceIds?: ReadonlySet<string>;
}) {
  return renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={binding}
      activity={activity}
      comparison={comparisonOf(props.entries)}
      activeStorageName="Local storage 1"
      selectionMode={props.selectionMode}
      selectableResourceIds={props.selectableResourceIds}
      selectedResourceIds={props.selectedResourceIds}
      loading={false}
      loadingMore={false}
      actionError={null}
      actionBusyResourceId={null}
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onSessionAction={() => undefined}
    />,
  );
}

test("Push selection folds the new decisions into existing task cards", () => {
  const html = render({
    selectionMode: "push",
    entries: [healthyEntry, blockedEntry, divergedEntry],
    selectableResourceIds: new Set([healthyEntry.resource_id]),
    selectedResourceIds: new Set([healthyEntry.resource_id]),
  });

  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 1);
  assert.match(html, /aria-label="Exclude Test task"/);
  assert.doesNotMatch(html, /aria-label="(Include|Exclude) Older codec task"/);
  assert.match(html, /data-thread-status="unavailable"/);
  assert.match(html, /Mallard can&#x27;t safely capture this Codex session\. It still opens on this computer\./);
  assert.match(html, /Pull required/);
  assert.match(html, /Pull will keep both histories as independent sessions/);
  assert.equal((html.match(/aria-label="Open in Codex"/g) ?? []).length, 3);
});

test("Pull selection presents divergence as the single keep-both action", () => {
  const html = render({
    selectionMode: "pull",
    entries: [divergedEntry],
    selectableResourceIds: new Set([divergedEntry.resource_id]),
    selectedResourceIds: new Set([divergedEntry.resource_id]),
  });

  assert.match(html, /aria-label="Exclude Changed on both machines"/);
  assert.match(html, /Keep both/);
  assert.doesNotMatch(html, /Fork in Codex|Fork and overwrite|radiogroup/);
});

test("browse mode shows sync state but no selection decisions", () => {
  const html = render({ entries: [healthyEntry, blockedEntry, divergedEntry] });

  assert.doesNotMatch(html, /type="checkbox"/);
  assert.equal((html.match(/aria-label="Open in Codex"/g) ?? []).length, 3);
  assert.doesNotMatch(html, /Fork in Codex/);
});

test("blocked tasks use the unavailable indicator instead of the upload arrow", () => {
  const html = render({ entries: [healthyEntry, blockedEntry] });

  assert.match(
    html,
    /sync-thread-sync-indicator unavailable[^>]+aria-label="Can&#x27;t sync — Mallard can&#x27;t safely capture this Codex session\. It still opens on this computer\."/,
  );
  assert.match(html, /sync-thread-sync-indicator local/);
  assert.equal((html.match(/sync-thread-sync-indicator unavailable/g) ?? []).length, 1);
});

test("storage-only tasks offer the reviewed create-local action", () => {
  const html = render({
    selectionMode: "pull",
    entries: [storedEntry],
    selectableResourceIds: new Set([storedEntry.resource_id]),
  });

  assert.match(html, /class="sync-history-stored-thread-id"><code>019f7798-5437-7632-9dbc-5b589cf68bf3<\/code><\/span>/);
  assert.match(html, /aria-label="Include Improve cloud task titles, Codex thread 019f7798-5437-7632-9dbc-5b589cf68bf3"/);
  assert.doesNotMatch(html, /sync-history-stored-thread-location/);
});

test("out-of-window tasks remain discoverable in Older activity", () => {
  const html = render({
    selectionMode: "push",
    entries: [healthyEntry, olderEntry],
    selectableResourceIds: new Set([healthyEntry.resource_id, olderEntry.resource_id]),
  });

  assert.match(html, /Older activity/);
  assert.match(html, /Older unpublished task/);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
});

test("hold details explain metadata pulls, divergence, and unsupported codecs", () => {
  assert.match(syncHoldDetail("push", blockedEntry) ?? "", /can't safely capture this Codex session/i);
  assert.equal(syncHoldDetail("push", healthyEntry), null);
  assert.match(syncHoldDetail("push", divergedEntry) ?? "", /keep both histories as independent sessions/);
  assert.match(
    syncHoldDetail("push", entry({
      thread_id: HEALTHY_ID,
      state: "synced",
      storage_present: true,
      push_decision: decision("require_pull_metadata"),
    })) ?? "",
    /storage has newer sync metadata/,
  );
  assert.match(
    syncHoldDetail("push", entry({
      thread_id: HEALTHY_ID,
      state: "unavailable",
      status_detail: "Session file exceeds the 100 MiB per-file sync limit.",
    })) ?? "",
    /100 MiB per-file sync limit/,
  );
});
