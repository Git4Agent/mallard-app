import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { ProjectChatHistoryContent, syncHoldDetail } from "../../src/components/project-sync/ProjectChatHistoryPage";
import type {
  ConversationPlanActionType,
  ConversationReasonCode,
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
const LEGACY_ID = "019f7798-5437-7632-9dbc-5b589cf68bf1";
const REPLACE_ID = "019f7798-5437-7632-9dbc-5b589cf68bf2";
const STORED_ID = "019f7798-5437-7632-9dbc-5b589cf68bf3";
const OLDER_ACTIONABLE_ID = "019f7798-5437-7632-9dbc-5b589cf68bf4";
const OLDER_BLOCKED_ID = "019f7798-5437-7632-9dbc-5b589cf68bf5";
const OLDER_SYNCED_ID = "019f7798-5437-7632-9dbc-5b589cf68bf6";

function localThread(threadId: string, title: string) {
  return {
    thread_id: threadId,
    title,
    summary: "",
    started_at: WINDOW_START + 1_000,
    ended_at: WINDOW_START + 2_000,
    branch: "main",
    recorded_sha: "a".repeat(40),
    user_round_count: 2,
    agent_message_count: 3,
    tool_call_count: 4,
    total_tokens: 1_200,
    metrics_complete: true,
    commit_occurrence_count: 0,
  };
}

const history = {
  project_id: project.local_project_id,
  threads: [
    localThread(HEALTHY_ID, "Test task"),
    localThread(LEGACY_ID, "Legacy pre-0.146 session"),
    localThread(REPLACE_ID, "Force-replace candidate"),
  ],
  git: null,
  unmapped: [],
  warnings: [],
  window_start: WINDOW_START,
  window_end: WINDOW_START + 3_000_000,
  next_before: null,
  codex_home: "/Users/test/config/mallard/.codex",
  storage_sync: [],
};

function decision(
  action: ConversationPlanActionType,
  reason_code: ConversationReasonCode,
  overrides: Partial<ThreadSyncDecision> = {},
): ThreadSyncDecision {
  const explicit = ["replace_local", "replace_cloud", "tombstone_thread"].includes(action);
  const selectable = [
    "fast_forward_local",
    "fast_forward_cloud",
    "resolve_divergence",
    "create_cloud",
    "create_local",
    "replace_local",
    "replace_cloud",
    "tombstone_thread",
  ].includes(action);
  return {
    action,
    reason_code,
    selectable,
    selected_by_default: selectable && !explicit,
    requires_explicit_approval: explicit,
    ...overrides,
  };
}

function entry(overrides: Partial<ThreadSyncEntry> & Pick<ThreadSyncEntry, "thread_id">): ThreadSyncEntry {
  return {
    resource_id: `codex:session:${overrides.thread_id}`,
    display_name: overrides.thread_id,
    state: "local_only",
    local_present: true,
    storage_present: false,
    cloud_thread_id: overrides.thread_id,
    push_decision: decision("create_cloud", "cloud_missing"),
    pull_decision: decision("keep_local", "cloud_missing"),
    ...overrides,
  };
}

const healthyEntry = entry({
  thread_id: HEALTHY_ID,
  display_name: "Test task",
  local_updated_at: WINDOW_START + 2_000,
});
const legacyEntry = entry({
  thread_id: LEGACY_ID,
  display_name: "Legacy pre-0.146 session",
  local_updated_at: WINDOW_START + 2_000,
  push_decision: decision("blocked", "codec_unsupported", {
    blocked_reason: "This conversation cannot be transformed safely by the active codec.",
  }),
});
const replaceEntry = entry({
  thread_id: REPLACE_ID,
  display_name: "Force-replace candidate",
  state: "diverged",
  storage_present: true,
  local_updated_at: WINDOW_START + 2_000,
  storage_updated_at: WINDOW_START + 1_500,
  push_decision: decision("replace_cloud", "diverged"),
});
const storedEntry = entry({
  thread_id: STORED_ID,
  display_name: "Improve cloud thread titles",
  state: "storage_only",
  local_present: false,
  storage_present: true,
  storage_updated_at: WINDOW_START + 1_000,
  push_decision: decision("no_op", "local_missing"),
  pull_decision: decision("create_local", "local_missing"),
});
const olderActionableEntry = entry({
  thread_id: OLDER_ACTIONABLE_ID,
  display_name: "Older unpublished session",
  local_updated_at: WINDOW_START - 5_000,
});
const olderBlockedEntry = entry({
  thread_id: OLDER_BLOCKED_ID,
  display_name: "Older legacy session",
  local_updated_at: WINDOW_START - 6_000,
  push_decision: decision("blocked", "codec_unsupported"),
});
const olderSyncedEntry = entry({
  thread_id: OLDER_SYNCED_ID,
  display_name: "Older synced session",
  state: "synced",
  storage_present: true,
  local_updated_at: WINDOW_START - 7_000,
  storage_updated_at: WINDOW_START - 7_000,
  push_decision: decision("no_op", "identical"),
});

function comparisonOf(entries: ThreadSyncEntry[]) {
  return {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    compared_at: WINDOW_START + 4_000,
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
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
      history={history}
      comparison={comparisonOf(props.entries)}
      activeStorageName="Local storage 1"
      selectionMode={props.selectionMode}
      selectableResourceIds={props.selectableResourceIds}
      selectedResourceIds={props.selectedResourceIds}
      loading={false}
      loadingMore={false}
      actionError={null}
      actionBusyThreadId={null}
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onOpenCodex={() => undefined}
      onOpenTerminal={() => undefined}
    />,
  );
}

const selectable = new Set([healthyEntry.resource_id, replaceEntry.resource_id, olderActionableEntry.resource_id]);

test("selection mode folds decisions into the existing thread cards", () => {
  const html = render({
    selectionMode: "push",
    entries: [healthyEntry, legacyEntry, replaceEntry],
    selectableResourceIds: selectable,
    selectedResourceIds: new Set([healthyEntry.resource_id]),
  });

  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2, "only selectable rows get a checkbox");
  assert.equal((html.match(/>Test task<\/strong>/g) ?? []).length, 1, "each thread renders exactly once");
  assert.match(html, /aria-label="Exclude Test task"/);
  assert.doesNotMatch(html, /aria-label="(Include|Exclude) Legacy pre-0\.146 session"/);

  assert.match(html, />Can&#x27;t sync</);
  assert.match(html, /sync-sync-review-state[^"]*action-blocked/);
  assert.match(html, /sync-sync-row-blocked/, "blocked rows reuse the shared gutter marker");
  assert.match(html, /saved by an older Codex version\. It still opens on this computer\./);
  assert.doesNotMatch(html, /sync-history-sync-hold/, "the reason lives in the marker tooltip, not visible text");
  assert.doesNotMatch(html, /cannot be transformed safely by the active codec/);
  assert.match(html, />Review required</);

  assert.equal((html.match(/aria-label="Open in Codex"/g) ?? []).length, 3, "blocked threads stay openable");
  assert.doesNotMatch(html, /Stored only|Not materialized|Resumable|materialization/);
  assert.doesNotMatch(html, /sync-thread-variant/);
});

test("browse mode shows history only, with no decision content", () => {
  const html = render({ entries: [healthyEntry, legacyEntry, replaceEntry, olderBlockedEntry] });

  assert.doesNotMatch(html, /type="checkbox"/);
  assert.doesNotMatch(html, /Review required|Older activity/);
  assert.doesNotMatch(html, />Can&#x27;t sync</, "no review-state labels outside selection mode");
  assert.doesNotMatch(html, /sync-sync-row-blocked/);
  assert.equal((html.match(/aria-label="Open in Codex"/g) ?? []).length, 3);
});

test("blocked threads use the block indicator instead of the upload arrow", () => {
  const html = render({ entries: [healthyEntry, legacyEntry] });

  assert.match(
    html,
    /sync-thread-sync-indicator unavailable[^>]+aria-label="Can&#x27;t sync — this session was saved by an older Codex version\. It still opens on this computer\."/,
  );
  assert.match(html, /sync-thread-sync-indicator local/, "pushable threads keep the upload indicator");
  assert.equal((html.match(/sync-thread-sync-indicator unavailable/g) ?? []).length, 1);
  assert.match(html, /title="1 thread is unavailable for sync"/);
  assert.match(html, /title="1 local thread change"/);
});

test("out-of-window threads live in a collapsed Older activity group with a non-actionable count", () => {
  const html = render({
    selectionMode: "push",
    entries: [healthyEntry, olderActionableEntry, olderBlockedEntry, olderSyncedEntry],
    selectableResourceIds: new Set([healthyEntry.resource_id, olderActionableEntry.resource_id]),
  });

  assert.match(html, /Older activity/);
  assert.match(html, /title="3 threads outside this 30-day window"/);
  assert.match(html, /aria-label="Include Older unpublished session, Codex thread 019f7798-5437-7632-9dbc-5b589cf68bf4"/);
  assert.match(html, />2 threads need no changes or can&#x27;t sync</);
  assert.match(html, /Older legacy session/, "blocked threads stay discoverable");
  assert.match(html, /Older synced session/);
  assert.match(html, /On this Mac/);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2, "healthy in-window plus older actionable");
});

test("storage-only rows speak location and offer Pull to open", () => {
  const html = render({
    selectionMode: "pull",
    entries: [storedEntry],
    selectableResourceIds: new Set([storedEntry.resource_id]),
  });

  assert.match(html, /In Local storage 1 · Pull to open/);
  assert.match(html, /Updated [^<]+ · Codex thread 019f7798/);
  assert.match(html, /aria-label="Include Improve cloud thread titles, Codex thread 019f7798-5437-7632-9dbc-5b589cf68bf3"/);
});

test("hold details use user vocabulary for every non-actionable direction", () => {
  assert.match(
    syncHoldDetail("push", legacyEntry) ?? "",
    /older Codex version.*still opens on this computer/,
  );
  assert.equal(syncHoldDetail("push", healthyEntry), null);
  assert.match(
    syncHoldDetail("pull", entry({
      thread_id: STORED_ID,
      pull_decision: decision("store_only", "codec_unsupported"),
    })) ?? "",
    /backup copy that can't be opened/,
  );
  assert.match(
    syncHoldDetail("push", entry({
      thread_id: HEALTHY_ID,
      state: "storage_ahead",
      storage_present: true,
      push_decision: decision("require_pull", "local_prefix_of_cloud"),
    })) ?? "",
    /Pull them before pushing/,
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

test("the retired review-list vocabulary stays out of the sources", () => {
  const page = readFileSync("src/components/project-sync/ProjectChatHistoryPage.tsx", "utf8");
  const css = readFileSync("src/App.css", "utf8");
  const types = readFileSync("src/types.ts", "utf8");

  assert.doesNotMatch(page, /ThreadVariantReview|materialization/);
  assert.doesNotMatch(css, /sync-thread-variant/);
  assert.doesNotMatch(types, /ConversationMaterializationState|materialization_state/);
  assert.doesNotMatch(types, /resumable\??:/);
});
