import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import PushResourceWorkspace, { pushBudgetTotal } from "../../src/components/project-sync/PushResourceWorkspace";
import {
  ProjectChatHistoryContent,
  reviewWindowDays,
  syncHoldDetail,
} from "../../src/components/project-sync/ProjectActivityPage";
import type {
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectContentInventory,
  ProjectResourceDescriptor,
  ProjectSessionSummary,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../src/types";

/* The Push review when the backlog is bigger than one push
   (docs/PLAN_PUSH_CAPTURE_BUDGET.md): rows that do not fit stay in the one
   list, unselectable, wearing "Later"; the footer meter sums what this push
   carries; nothing counts rounds. */

const MB = 1024 * 1024;
const LIMIT = 512 * MB;
/* 2026-09-01T00:00:00Z. */
const NOW = 1_788_220_800;

const PROJECT_ROOT = "/Users/test/projects/mallard";

const project: LocalProjectSummary = {
  local_project_id: "project-mallard",
  bundle_id: "bundle-mallard",
  display_name: "mallard",
  local_alias: "Mallard local",
  repository_fingerprint: "fingerprint",
  project_root: PROJECT_ROOT,
  canonical_project_root: PROJECT_ROOT,
  profile_ids: { codex: "profile-codex" },
  profile_names: ["Default Codex"],
  revision: 1,
};

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica",
  local_project_id: project.local_project_id,
  bundle_id: project.bundle_id,
  project_root: PROJECT_ROOT,
  canonical_project_root: PROJECT_ROOT,
  profile_ids: { codex: "profile-codex" },
  state: "active",
  revision: 1,
  updated_at: 2,
};

const FITS_ID = "019f7798-5437-7632-9dbc-5b589cf68b10";
const WAITS_ID = "019f7798-5437-7632-9dbc-5b589cf68b11";

function entry(threadId: string, overrides: Partial<ThreadSyncEntry> = {}): ThreadSyncEntry {
  return {
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    thread_id: threadId,
    conversation_resource_id: `codex:conversation:${threadId}`,
    resource_id: `codex:conversation:${threadId}`,
    display_name: threadId,
    state: "local_only",
    local_present: true,
    storage_present: false,
    compared_against_storage: true,
    local_updated_at: NOW - 3_600,
    push_decision: { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
    capture_bytes: 96 * MB,
    ...overrides,
  };
}

const fits = entry(FITS_ID, { display_name: "Refine sync review UX", capture_bytes: 96 * MB });
const waits = entry(WAITS_ID, {
  display_name: "Catalog profile launch buttons",
  capture_bytes: 118 * MB,
  local_updated_at: NOW - 5 * 86_400,
  push_decision: {
    action: "blocked",
    blocker: "capture_budget",
    selectable: false,
    blocked_reason: "This push is full with newer conversations, so this one does not fit.",
  },
});

function comparison(overrides: Partial<ThreadSyncComparison> = {}): ThreadSyncComparison {
  return {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Attic R2",
    compared_at: NOW,
    entries: [fits, waits],
    counts: { synced: 0, local: 2, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    capture_budget: {
      limit_bytes: LIMIT,
      reserved_bytes: 3 * MB,
      admitted_bytes: 96 * MB,
      held_count: 1,
      held_bytes: 118 * MB,
    },
    remote_continuity: { state: "in_sync", revision: 1 },
    ...overrides,
  };
}

function session(threadId: string, title: string, endedAt: number): ProjectSessionSummary {
  return {
    resource_id: `codex:conversation:${threadId}`,
    provider: "codex",
    provider_display_name: "Codex",
    provider_session_id: threadId,
    logical_thread_id: threadId,
    source: "local_only",
    access: "bound_ready",
    title,
    preserved: null,
    summary: "",
    started_at: endedAt - 1_000,
    ended_at: endedAt,
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
        kind: "open_in_app",
        label: "Open in Codex",
        enabled: true,
        disabled_reason: null,
      },
    ],
  };
}

const activity: ProjectActivity = {
  project_id: project.local_project_id,
  sessions: [
    session(FITS_ID, "Refine sync review UX", NOW - 3_600),
    session(WAITS_ID, "Catalog profile launch buttons", NOW - 5 * 86_400),
  ],
  git: null,
  unmapped: [],
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" }],
  warnings: [],
  window_start: NOW - 400 * 86_400,
  window_end: NOW,
  // The backend still reports what lies before the window; a review ignores it.
  older: { newest_at: NOW - 401 * 86_400, commits: 3, sessions: 2 },
  storage_sync: [],
};

function renderReviewList(selectable: Set<string>) {
  return renderToStaticMarkup(
    <ProjectChatHistoryContent
      project={project}
      binding={binding}
      activity={activity}
      loading={false}
      loadingMore={false}
      actionError={null}
      loadError={null}
      onRetryLoad={() => undefined}
      actionBusyResourceId={null}
      onShowLess={() => undefined}
      canShowLess={false}
      onLoadMore={() => undefined}
      onSessionAction={() => undefined}
      comparison={comparison()}
      selectionMode="push"
      selectedResourceIds={selectable}
      selectableResourceIds={selectable}
      onToggleResource={() => undefined}
    />,
  );
}

test("a session that doesn't fit stays in the list, unselectable, wearing Later", () => {
  const html = renderReviewList(new Set([fits.resource_id]));

  // Both rows are on screen, in the one list: no fold, no paging control.
  assert.match(html, /Refine sync review UX/);
  assert.match(html, /Catalog profile launch buttons/);
  assert.doesNotMatch(html, /Older activity|sync-history-older|Load earlier/);
  // Only the row that fits has a checkbox; the other wears the chip instead.
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 1);
  assert.equal((html.match(/>Later<\/span>/g) ?? []).length, 1);
  // The held row keeps its real status glyph — it is still local work that
  // will go — and its sentence says the push is full and to push again.
  assert.equal((html.match(/sync-thread-sync-indicator local"/g) ?? []).length, 2);
  assert.match(
    html,
    /Not in this push\. Doesn(&#x27;|')t fit in this push — the 512 MB limit is full with newer sessions\. Push again after this one\./,
  );
  assert.doesNotMatch(html, /round/i);
  // Review rows carry their push size after the duration.
  assert.match(html, /data-icon="drive"[^>]*>(?:<[^>]*>)*<\/svg><b>96 MB<\/b>/);
  assert.match(html, /<b>118 MB<\/b>/);
});

test("hold copy names the limit when the review knows it, and a too-large session is unsyncable", () => {
  assert.equal(
    syncHoldDetail("push", waits, LIMIT),
    "Doesn't fit in this push — the 512 MB limit is full with newer sessions. Push again after this one.",
  );
  assert.equal(
    syncHoldDetail("push", waits),
    "Doesn't fit in this push — the limit is full with newer sessions. Push again after this one.",
  );
  assert.equal(
    syncHoldDetail("pull", { ...waits, pull_decision: waits.push_decision }, LIMIT),
    "Doesn't fit in this pull — the 512 MB limit is full with newer sessions. Pull again after this one.",
  );
  const tooLarge = entry(WAITS_ID, {
    push_decision: { action: "blocked", blocker: "too_large", selectable: false },
  });
  assert.equal(syncHoldDetail("push", tooLarge), "Can't sync — this session is larger than one push carries.");
});

test("a review asks for the whole span back to the oldest outstanding session", () => {
  // No comparison to measure by: ten years, so nothing is left behind a page.
  assert.equal(reviewWindowDays(null, NOW), 3_650);
  assert.equal(reviewWindowDays(comparison({ entries: [] }), NOW), 3_650);
  // Measured: back to the oldest entry plus a day of slack…
  const hundredDays = entry("old", { local_updated_at: NOW - 100 * 86_400 });
  assert.equal(reviewWindowDays(comparison({ entries: [fits, hundredDays] }), NOW), 101);
  // …and never less than the standing page.
  assert.equal(reviewWindowDays(comparison({ entries: [fits] }), NOW), 30);
});

function conversationResource(threadEntry: ThreadSyncEntry): ProjectResourceDescriptor {
  return {
    resource_id: threadEntry.resource_id,
    kind: "conversation",
    provider: "codex",
    provider_resource_type: "session",
    display_name: threadEntry.display_name,
    provenance: { kind: "conversation" },
    apply_policy: "safe_file",
    codec_version: 1,
    metadata: {},
    category: "conversations",
    default_selected: true,
  };
}

const projectFile: ProjectResourceDescriptor = {
  resource_id: "project:file:assets/recording.mov",
  kind: "project_content_file",
  provider: null,
  provider_resource_type: null,
  display_name: "recording.mov",
  provenance: { kind: "project_content" },
  apply_policy: "explicit_review",
  codec_version: 1,
  metadata: {},
  category: "project_files",
  logical_paths: ["project/assets/recording.mov"],
  default_selected: true,
};

const projectFiles: ProjectContentInventory = {
  local_project_id: project.local_project_id,
  storage_id: "storage-1",
  project_root: PROJECT_ROOT,
  eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  review_token: "review-token",
  storage_revision: null,
  preference_revision: 0,
  excluded_resource_ids: [],
  entries: [{
    descriptor: projectFile,
    entry_type: "file",
    relative_path: "assets/recording.mov",
    logical_path: "project/assets/recording.mov",
    size: 500 * MB,
    mode: 0o644,
    source_mtime: NOW - 60,
    state: "local_only",
    local_present: true,
    storage_present: false,
    base_present: false,
    selected_in_recipe: false,
    newly_discovered: true,
    selected_after_scan: true,
    blocked_reason: null,
    warning_code: null,
    warning_digest: null,
  }],
  ignored_count: 0,
  blocked_count: 0,
  warnings: [],
  scanned_at: NOW,
};

function renderWorkspace(selected: Set<string>, inventory: ProjectContentInventory | null = null) {
  return renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[conversationResource(fits), conversationResource(waits), ...(inventory ? [projectFile] : [])]}
      selected={selected}
      projectDefaults={new Set([fits.resource_id])}
      busy={false}
      error={null}
      threadComparison={comparison()}
      projectContentInventory={inventory}
      projectContentScanned={inventory != null}
      showProjectFiles={inventory != null}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
}

test("the review banner says what this push carries and that the rest waits — no rounds", () => {
  const html = renderWorkspace(new Set([fits.resource_id]));
  assert.match(html, /Too much to push at once/);
  assert.match(
    html,
    /This push carries the newest 1 session \(96 MB\), back to [A-Z][a-z]{2} \d{1,2}\. The 1 older session \(118 MB\) doesn(&#x27;|')t fit — push again after this one\./,
  );
  assert.doesNotMatch(html, /round|wait for the next/i);
  // The held row is listed in the fallback inventory too, unselectable.
  assert.match(html, /Catalog profile launch buttons/);
});

test("the footer meter sums sessions, requirements, and project files against the limit", () => {
  const within = renderWorkspace(new Set([fits.resource_id]));
  assert.match(within, /class="sync-push-budget"[^>]*role="status"/);
  assert.match(within, /This push · 99 MB of 512 MB/);
  assert.match(within, /role="meter"[^>]*aria-valuemax="536870912"[^>]*aria-valuenow="103809024"/);
  assert.match(within, /title="Sessions 96 MB · Requirements 3\.0 MB · Project files 0 B"/);
  const pushWithin = within.slice(within.lastIndexOf("<button", within.indexOf("sync-pull-apply-button")));
  assert.doesNotMatch(pushWithin.slice(0, pushWithin.indexOf(">")), /disabled=""/);

  // A 500 MB project file carries the total past the limit: the sentence
  // turns, the bar turns, and Push refuses with the sentence as its title.
  const over = renderWorkspace(new Set([fits.resource_id, projectFile.resource_id]), projectFiles);
  assert.match(over, /class="sync-push-budget over"/);
  assert.match(over, /87 MB over the 512 MB push limit — exclude sessions or files to push\./);
  const pushOver = over.slice(over.lastIndexOf("<button", over.indexOf("sync-pull-apply-button")));
  const pushOverTag = pushOver.slice(0, pushOver.indexOf(">"));
  assert.match(pushOverTag, /disabled=""/);
  assert.match(pushOverTag, /title="87 MB over the 512 MB push limit — exclude sessions or files to push\."/);
});

test("pushBudgetTotal is null without a budget and otherwise sums what the selection would push", () => {
  assert.equal(pushBudgetTotal(new Set([fits.resource_id]), comparison({ capture_budget: null })), null);
  const total = pushBudgetTotal(
    new Set([fits.resource_id, waits.resource_id, projectFile.resource_id]),
    comparison(),
    projectFiles,
  );
  assert.deepEqual(total, {
    sessions: (96 + 118) * MB,
    reserved: 3 * MB,
    projectFiles: 500 * MB,
    total: (96 + 118 + 3 + 500) * MB,
    limit: LIMIT,
  });
});
