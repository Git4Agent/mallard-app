import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import PushResourceWorkspace, {
  pushBudgetTotal,
  pushReviewDisabledReason,
  recommendedPushSelection,
  sanitizePushSelection,
} from "../../src/components/project-sync/PushResourceWorkspace";
import { recipeWithSelection } from "../../src/components/project-sync/model";
import {
  ProjectChatHistoryContent,
} from "../../src/components/project-sync/ProjectActivityPage";
import type {
  InventoryResource,
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectContentInventory,
  ProjectSessionSummary,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../src/types";

/* Automatic selection fills the budget in newest-first order.
   Every eligible session remains available for manual selection. */

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
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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

test("all eligible sessions stay selectable regardless of age or list position", () => {
  const html = renderReviewList(new Set([fits.resource_id, waits.resource_id]));

  // Both rows are on screen and selectable in one list.
  assert.match(html, /Refine sync review UX/);
  assert.match(html, /Catalog profile launch buttons/);
  assert.doesNotMatch(html, /Older activity|sync-history-older|Load earlier/);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
  assert.doesNotMatch(html, />Later<\/span>/);
  assert.equal((html.match(/sync-thread-sync-indicator local"/g) ?? []).length, 2);
  assert.match(html, /data-icon="drive"[^>]*>(?:<[^>]*>)*<\/svg><b>96 MB<\/b>/);
  assert.match(html, /<b>118 MB<\/b>/);
});

function conversationResource(threadEntry: ThreadSyncEntry): InventoryResource {
  return {
    resource_id: threadEntry.resource_id,
    kind: { key: "conversation", version: 1 },
    provider: "codex",
    provider_resource_type: "session",
    display_name: threadEntry.display_name,
    provenance: { kind: "conversation", version: 1, fields: {} },
    apply_policy: { key: "safe_file", version: 1 },
    codec_version: 1,
    attributes: {},
    category: "conversations",
    kind_label: "Conversation",
    apply_policy_label: "Safe file",
    selectable: true,
    default_selected: true,
  };
}

const projectFile: InventoryResource = {
  resource_id: "project:file:assets/recording.mov",
  kind: { key: "project_content_file", version: 1 },
  provider: null,
  provider_resource_type: null,
  display_name: "recording.mov",
  provenance: { kind: "project_content", version: 1, fields: {} },
  apply_policy: { key: "explicit_review", version: 1 },
  codec_version: 1,
  attributes: {},
  category: "project_files",
  kind_label: "Project file",
  apply_policy_label: "Explicit review",
  selectable: true,
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

test("automatic selection takes the newest sessions that fit, even when every session was saved", () => {
  const entries = Array.from({ length: 394 }, (_, index) => entry(`session-${index}`, {
    local_updated_at: NOW - index * 60,
    capture_bytes: 8 * MB,
  }));
  const resources = [...entries].reverse().map(conversationResource);
  const reviewed = comparison({ entries: [...entries].reverse() });
  const selected = recommendedPushSelection(new Set(resources.map((resource) => resource.resource_id)), resources, reviewed);

  assert.deepEqual([...selected], entries.slice(0, 63).map((row) => row.resource_id));
  assert.equal(pushBudgetTotal(selected, reviewed)?.total, 507 * MB);
  assert.equal(pushReviewDisabledReason({ selected, threadComparison: reviewed, deletionApproved: true }), null);
});

function renderWorkspace(selected: Set<string>, inventory: ProjectContentInventory | null = null) {
  return renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
      resources={[conversationResource(fits), conversationResource(waits), ...(inventory ? [projectFile] : [])]}
      selected={selected}
      projectDefaults={new Set([fits.resource_id])}
      busy={false}
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

test("manual selection can use an older session", () => {
  const html = renderWorkspace(new Set([waits.resource_id]));
  assert.doesNotMatch(html, /Too much to push at once|push again after this one|>Later</i);
  assert.match(html, /Catalog profile launch buttons/);
  assert.match(html, /This push · 121 MB of 512 MB/);
});

test("the meter is drawn in both pinned zones and sums sessions, requirements, and project files", () => {
  const within = renderWorkspace(new Set([fits.resource_id]));
  // The header glance and the footer gauge are the review's two zones that do
  // not scroll away, and they carry the same figure.
  assert.equal(within.match(/class="sync-push-budget-figure"/g)?.length, 2);
  assert.match(within, /class="sync-push-budget-glance" title="Sessions 96 MB · Requirements 3\.0 MB · Project files 0 B"/);
  assert.match(within, /class="sync-push-budget-gauge"[^>]*role="status"/);
  // Only the footer's announces. Two live regions would read every tick twice.
  assert.equal(within.match(/class="sync-push-budget[^"]*"[^>]*role="status"/g)?.length, 1);
  assert.match(within, /role="meter"[^>]*aria-valuemax="536870912"[^>]*aria-valuenow="103809024"/);
  assert.match(within, /aria-valuetext="This push · 99 MB of 512 MB"/);
  assert.match(within, /<b>99 MB<\/b> \/ 512 MB/);
  // While it fits, the track's right end IS the limit — no tick, no refusal.
  assert.doesNotMatch(within, /class="sync-push-budget-track over"/);
  assert.doesNotMatch(within, /sync-push-budget-sentence/);
  const pushWithin = within.slice(within.lastIndexOf("<button", within.indexOf("sync-pull-apply-button")));
  assert.doesNotMatch(pushWithin.slice(0, pushWithin.indexOf(">")), /disabled=""/);

  // A 500 MB project file carries the total past the limit: both figures turn
  // to the overshoot, the track grows a tick the fill runs past, the refusal
  // takes its own row, and Push refuses with the same sentence as its title.
  const over = renderWorkspace(new Set([fits.resource_id, projectFile.resource_id]), projectFiles);
  assert.match(over, /class="sync-push-budget-gauge over"/);
  assert.match(over, /class="sync-push-budget-track over"/);
  // The tick sits where the limit fell on a track scaled to the total, so the
  // fill visibly runs past it: 512 of 599 MB is 85.4% along.
  assert.match(over, /class="sync-push-budget-track over"[^>]*>.*?<b style="left:85\.4/);
  assert.equal(over.match(/class="sync-push-budget-figure over"/g)?.length, 2);
  assert.match(over, /<b>\+87 MB<\/b>/);
  assert.equal(
    over.match(/87 MB over the 512 MB push limit — exclude sessions or files to push\./g)?.length,
    // Once as the refusal row, once as the Push button's title, and once in
    // each figure's aria-valuetext.
    4,
  );
  assert.match(over, /class="sync-push-budget-sentence">87 MB over the 512 MB push limit/);
  const pushOver = over.slice(over.lastIndexOf("<button", over.indexOf("sync-pull-apply-button")));
  const pushOverTag = pushOver.slice(0, pushOver.indexOf(">"));
  assert.match(pushOverTag, /disabled=""/);
  assert.match(pushOverTag, /title="87 MB over the 512 MB push limit — exclude sessions or files to push\."/);
});

test("one rule refuses the push, and it is the one both push buttons read", () => {
  // Regression: the path bar's Push publishes the open review, and its own
  // disable rule knew nothing about the capture budget or the project-file
  // deletion approval — so a review the footer refused could still be pushed
  // from the bar above it.
  const overBudget = new Set([fits.resource_id, projectFile.resource_id]);
  assert.equal(
    pushReviewDisabledReason({
      selected: overBudget,
      threadComparison: comparison(),
      projectContentInventory: projectFiles,
      deletionApproved: true,
    }),
    "87 MB over the 512 MB push limit — exclude sessions or files to push.",
  );
  // Unticked deletions refuse too, and that state lives with the selection.
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      projectContentInventory: projectFiles,
      projectContentRemovals: new Set([projectFile.resource_id]),
      deletionApproved: false,
    }),
    "Approve the selected project file deletions",
  );
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      projectContentInventory: projectFiles,
      projectContentRemovals: new Set([projectFile.resource_id]),
      deletionApproved: true,
    }),
    null,
  );
  // An empty review, and one still scanning, refuse as they always did.
  assert.equal(
    pushReviewDisabledReason({ selected: new Set(), deletionApproved: true }),
    "Nothing is selected",
  );
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      deletionApproved: true,
      projectContentLoading: true,
    }),
    "Project files are still being scanned",
  );
  // And a review that fits, with nothing held, may push.
  assert.equal(
    pushReviewDisabledReason({
      selected: new Set([fits.resource_id]),
      threadComparison: comparison(),
      deletionApproved: true,
    }),
    null,
  );
});

test("the panel's footer button refuses on exactly the shared rule", () => {
  // The footer stopped carrying a rule of its own; this pins that it still
  // refuses where the shared one does, with that reason as its title.
  const over = renderWorkspace(new Set([fits.resource_id, projectFile.resource_id]), projectFiles);
  const reason = pushReviewDisabledReason({
    selected: new Set([fits.resource_id, projectFile.resource_id]),
    threadComparison: comparison(),
    projectContentInventory: projectFiles,
    deletionApproved: true,
  });
  assert.equal(reason, "87 MB over the 512 MB push limit — exclude sessions or files to push.");
  const push = over.slice(over.lastIndexOf("<button", over.indexOf("sync-pull-apply-button")));
  const tag = push.slice(0, push.indexOf(">"));
  assert.match(tag, /disabled=""/);
  assert.match(tag, new RegExp(`title="${reason!.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`));
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

test("five stored 96 MiB sessions do not expand a selected 40 MiB capture", () => {
  const selectedEntry = entry("selected", { capture_bytes: 40 * MB });
  const syncedEntries = Array.from({ length: 5 }, (_, index) => entry(`synced-${index}`, {
    state: "synced",
    storage_present: true,
    push_decision: { action: "no_changes", selectable: false },
  }));
  const resources = [...syncedEntries, selectedEntry].map(conversationResource);
  const reviewed = comparison({
    entries: [...syncedEntries, selectedEntry],
    capture_budget: { limit_bytes: LIMIT, reserved_bytes: 0 },
  });
  const savedRecipe = recipeWithSelection(
    { schema_version: 1, revision: 1, entries: {} },
    resources,
    new Set(syncedEntries.map((row) => row.resource_id)),
  );
  const selected = sanitizePushSelection(new Set([selectedEntry.resource_id]), resources, reviewed);
  const captureRecipe = recipeWithSelection(savedRecipe, resources, selected);

  assert.deepEqual(Object.keys(captureRecipe.entries), [selectedEntry.resource_id]);
  assert.equal(pushBudgetTotal(selected, reviewed)?.total, 40 * MB);
  assert.equal(
    pushBudgetTotal(new Set(Object.keys(captureRecipe.entries)), reviewed)?.total,
    40 * MB,
    "the meter and backend recipe use the same capture set",
  );
  assert.equal(pushReviewDisabledReason({ selected, threadComparison: reviewed, deletionApproved: true }), null);
});


test("Push review shows the selected redaction mode and the Off warning", () => {
  for (const enabled of [true, false]) {
    const html = renderToStaticMarkup(
      <PushResourceWorkspace credentialRedactionEnabled={enabled} resources={[]} selected={new Set()} projectDefaults={new Set()} busy={false} onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined} />,
    );
    assert.match(html, new RegExp(`Credential redaction: ${enabled ? "On" : "Off"}`));
    assert.equal(html.includes("Exported content can include API keys, tokens, and passwords."), !enabled);
  }
});


test("current sessions remain selectable for a reviewed mode change", () => {
  const changed = entry("mode-change", { state: "synced", storage_present: true, status_detail: "Export settings changed", push_decision: { action: "publish", selectable: true } });
  const html = renderToStaticMarkup(
    <PushResourceWorkspace credentialRedactionEnabled={false} resources={[conversationResource(changed)]}
      selected={new Set([changed.resource_id])} projectDefaults={new Set([changed.resource_id])}
      busy={false} threadComparison={comparison({ entries: [changed] })}
      onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined} />,
  );
  assert.match(html, /1 of 1 selected/);
  assert.doesNotMatch(html, /Nothing to push/);
});
