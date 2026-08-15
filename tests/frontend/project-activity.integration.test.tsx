import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import Icon from "../../src/components/Icons";
import ProjectActivityPage, {
  activityHeaderCounts,
  BranchPicker,
  ProjectChatHistoryContent,
  SessionMetrics,
  sessionDebugCopyText,
  sessionSyncPresentation,
  unresolvedSyncPresentation,
} from "../../src/components/project-sync/ProjectActivityPage";
import ProjectSidebar from "../../src/components/project-sync/ProjectSidebar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../../src/components/project-sync/ThreadStatusIcon";
import { createSingleFlight } from "../../src/components/project-sync/api";

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

const CODEX_SESSION_ID = "019f7798-5437-7632-9dbc-5b589cf68bf0";
const codexResourceId = `codex:conversation:${CODEX_SESSION_ID}`;

function codexActions(enabled = true) {
  const disabledReason = enabled ? null : "This session is not materialized locally.";
  return [
    {
      action_id: "codex.open_in_app",
      kind: "open_in_app" as const,
      label: "Open in Codex",
      enabled,
      disabled_reason: disabledReason,
    },
    {
      action_id: "codex.open_in_terminal",
      kind: "open_in_terminal" as const,
      label: "Open in Terminal",
      enabled,
      disabled_reason: disabledReason,
    },
  ];
}

const codexSession = {
  resource_id: codexResourceId,
  provider: "codex",
  provider_display_name: "Codex",
  provider_session_id: CODEX_SESSION_ID,
  logical_thread_id: CODEX_SESSION_ID,
  source: "local_only" as const,
  access: "bound_ready" as const,
  title: "Add Git history mapping",
  is_preserved_local: false,
  summary: "Map project-owned sessions onto the first-parent rail.",
  started_at: 1_752_800_000,
  ended_at: 1_752_803_600,
  working_directory: "/Users/test/projects/mallard/packages/desktop",
  relative_cwd: "packages/desktop",
  branch: "main",
  recorded_sha: "a".repeat(40),
  is_active: false,
  user_round_count: 3,
  agent_message_count: 5,
  tool_call_count: 8,
  total_tokens: 24800,
  metrics_complete: true,
  actions: codexActions(),
};

const activity = {
  project_id: project.local_project_id,
  sessions: [codexSession],
  git: {
    selected_branch: "main",
    branches: [{ name: "main", is_current: true, available: true }],
    github_repository_url: "https://github.com/Git4Agent/mallard",
    commits: [{
      sha: "b".repeat(40),
      short_sha: "bbbbbbb",
      committed_at: 1_752_802_000,
      subject: "Add project-scoped history",
      session_refs: [{ resource_id: codexResourceId }],
    }],
    unique_session_count: 1,
    next_cursor: null,
  },
  unmapped: [],
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" as const }],
  warnings: [],
  window_start: 1_750_000_000,
  window_end: 1_753_000_000,
  next_before: 1_750_000_000,
  storage_sync: [{
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    last_pull_at: 1_752_700_000,
    last_push_at: 1_752_800_000,
  }],
};

const contentDefaults = {
  project,
  binding,
  loading: false,
  loadingMore: false,
  actionError: null,
  actionBusyResourceId: null,
  onBranchChange: () => undefined,
  onRefresh: () => undefined,
  onLoadMore: () => undefined,
  onSessionAction: () => undefined,
};

function emptyComparison(overrides: Record<string, unknown> = {}) {
  return {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    compared_at: 1_752_804_000,
    entries: [],
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    remote_continuity: { state: "in_sync", revision: 1 },
    ...overrides,
  };
}

test("the branch picker exposes branch state without using the native select menu", () => {
  const html = renderToStaticMarkup(
    <BranchPicker
      branches={[
        { name: "main", is_current: true, available: true },
        { name: "release/0.1", is_current: false, available: true },
        { name: "removed-experiment", is_current: false, available: false },
      ]}
      selectedBranch="main"
      disabled={false}
      onChange={() => undefined}
    />,
  );

  assert.match(html, /aria-haspopup="menu"/);
  assert.match(html, /aria-label="Branch: main, current branch"/);
  // The trigger is the branch name alone: "Current" belongs to the open menu,
  // and the row subtitles are carried by the badges and the row title instead.
  assert.equal(html.match(/>Current</g)?.length, 1);
  assert.doesNotMatch(html, /Checked out locally|Local branch|Recorded session history only/);
  assert.match(html, />History only</);
  assert.match(html, /This branch is no longer available locally/);
  assert.doesNotMatch(html, /<select/);
});

test("path bar counts tally the whole comparison, split by the button that clears them", () => {
  const decision = (action: string, reason_code = "ready") => ({
    action,
    reason_code,
    selectable: true,
  });
  const entry = (
    state: string,
    overrides: Record<string, unknown> = {},
  ) => ({
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    thread_id: `thread-${state}-${JSON.stringify(overrides)}`,
    resource_id: `codex:conversation:${state}-${JSON.stringify(overrides)}`,
    display_name: state,
    state,
    local_present: true,
    storage_present: true,
    push_decision: decision("noop"),
    pull_decision: decision("noop"),
    ...overrides,
  }) as Parameters<typeof sessionSyncPresentation>[0];

  const counts = activityHeaderCounts({
    entries: [
      entry("local_only"),
      entry("local_ahead"),
      entry("storage_ahead"),
      entry("diverged"),
      entry("synced"),
      entry("unavailable"),
      entry("unknown"),
      // Waiting on a provider that is still writing: work, but nobody's to do.
      entry("local_ahead", { push_decision: decision("hold", "busy") }),
      // A ghost of a thread removed on both sides belongs to no one.
      entry("local_only", { local_present: false, storage_present: false }),
    ],
  } as Parameters<typeof activityHeaderCounts>[0], "Local storage 1");

  // Two states push, one pulls; nothing else is claimed by either verb.
  assert.equal(counts.push, 2);
  assert.equal(counts.pull, 1);
  // diverged + pending + unavailable + unknown-as-conflict.
  assert.equal(counts.attention, 4);
  // A conflict or an unsyncable thread is louder than a review or a hold.
  assert.equal(counts.severity, "danger");
  assert.match(counts.detail, /1 thread changed on both sides/);
  assert.match(counts.detail, /1 thread waiting to sync/);
  assert.match(counts.detail, /1 thread Mallard could not match/);
  assert.match(counts.detail, /1 thread that can't be synced/);

  // A blocked push is not pushable, however the state reads on its own.
  const blocked = activityHeaderCounts({
    entries: [entry("local_ahead", { push_decision: { action: "blocked", reason_code: "ready", selectable: false } })],
  } as Parameters<typeof activityHeaderCounts>[0], "Local storage 1");
  assert.equal(blocked.push, 0);
  assert.equal(blocked.attention, 1);

  // No comparison yet is not "nothing to do" — it is no answer, and the bar
  // draws no badge rather than a confident zero.
  const empty = activityHeaderCounts(null, "Local storage 1");
  assert.deepEqual(
    { push: empty.push, pull: empty.pull, attention: empty.attention },
    { push: 0, pull: 0, attention: 0 },
  );
  assert.equal(empty.detail, "");
});

test("the path bar hangs the branch on the section crumb and keeps one overflow", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const bar = readFileSync("src/components/project-sync/SyncPathBar.tsx", "utf8");
  const sync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  // The branch scopes the section, so it rides that crumb rather than standing
  // as a fourth one — and a session replaces the crumb, taking the chip with it.
  assert.match(workspace, /id: "section",[\s\S]*?scope: activityHeader && activityHeader\.branches\.length > 0 \? \(\s*<BranchPicker/);
  assert.match(bar, /\{crumb\.scope\}/);

  // Counts ride the buttons that clear them, and stand down while that button
  // is mid-review or mid-transfer and its label is the readout.
  assert.match(workspace, /const showPullCount = pullCount > 0 && !workspacePullReviewOpen && !workspacePullProgress;/);
  assert.match(workspace, /const showPushCount = pushCount > 0 && !workspacePushReviewOpen && !workspacePushProgress;/);
  assert.match(workspace, /\{showPullCount && <span className="sync-path-count" aria-hidden="true">\{pullCount\}<\/span>\}/);
  assert.match(workspace, /\{showPushCount && <span className="sync-path-count" aria-hidden="true">\{pushCount\}<\/span>\}/);
  // The number is decorative only because the button's own name already says
  // it — never let the badge become the only place the count is spoken.
  assert.match(workspace, /`Pull \$\{pullCount\} newer thread\$\{pullCount === 1 \? "" : "s"\} from /);
  assert.match(workspace, /`Push \$\{pushCount\} newer thread\$\{pushCount === 1 \? "" : "s"\} to /);

  // One overflow, at the end of the row. Two `⋯` in one bar was the reason a
  // command could only be found by opening both.
  assert.doesNotMatch(bar, /windowActions/);
  assert.doesNotMatch(workspace, /windowActions/);
  assert.match(sync, /appActions: PathBarMenuGroup\[\] = \[\{\s*label: "Mallard",/);
  // And one refresh glyph: the app-wide reload cannot wear the same mark as
  // the project refresh sitting a few pixels from it.
  assert.match(sync, /label: "Reload app data",[\s\S]*?icon: "computer" as const,/);
});

test("session indicators use the quiet desktop sync vocabulary", () => {
  const presentation = (state: Parameters<typeof sessionSyncPresentation>[0]["state"]) => (
    sessionSyncPresentation({ state } as Parameters<typeof sessionSyncPresentation>[0], "Storage")
  );

  assert.deepEqual(
    ["synced", "local_only", "local_ahead", "storage_only", "storage_ahead", "diverged", "unavailable", "unknown"].map((state) => {
      const { status, className } = presentation(state as Parameters<typeof presentation>[0]);
      return { status, className };
    }),
    [
      { status: "synced", className: "synced" },
      { status: "local-outstanding", className: "local" },
      { status: "local-outstanding", className: "local" },
      { status: "cloud-outstanding", className: "storage" },
      { status: "cloud-outstanding", className: "storage" },
      { status: "divergent", className: "diverged" },
      { status: "unavailable", className: "unavailable" },
      { status: "conflict", className: "conflict" },
    ],
  );

  const unknown = sessionSyncPresentation({
    state: "unknown",
    status_detail: "Conversation ancestry could not be proven safely.",
  } as Parameters<typeof sessionSyncPresentation>[0], "Storage");
  assert.match(unknown.label, /ancestry could not be proven safely/i);
  assert.doesNotMatch(unknown.label, /baseline/i);

  const pending = sessionSyncPresentation({
    state: "synced",
    status_detail: "The provider is still writing this task.",
    push_decision: {
      action: "blocked",
      reason_code: "provider_not_quiescent",
    },
  } as Parameters<typeof sessionSyncPresentation>[0], "Storage");
  assert.deepEqual(
    { status: pending.status, className: pending.className },
    { status: "pending", className: "pending" },
  );

  const unavailable = sessionSyncPresentation({
    state: "local_only",
    push_decision: {
      action: "blocked",
      reason_code: "codec_unsupported",
    },
  } as Parameters<typeof sessionSyncPresentation>[0], "Storage");
  assert.deepEqual(
    { status: unavailable.status, className: unavailable.className },
    { status: "unavailable", className: "unavailable" },
  );

  // Provider naming in hold copy comes from the entry, never from a
  // hard-coded provider branch: a Claude entry speaks Claude.
  const claudeBlocked = sessionSyncPresentation({
    state: "local_only",
    provider_display_name: "Claude",
    push_decision: {
      action: "blocked",
      reason_code: "codec_unsupported",
    },
  } as Parameters<typeof sessionSyncPresentation>[0], "Storage");
  assert.match(claudeBlocked.label, /can't safely capture this Claude session/i);

  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-thread-sync-indicator\.synced \{ color: var\(--thread-status-synced\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.local \{ color: var\(--thread-status-local\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.storage \{ color: var\(--thread-status-cloud\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.diverged \{ color: var\(--thread-status-divergent\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.pending \{ color: var\(--thread-status-pending\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.conflict \{ color: var\(--thread-status-conflict\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.unavailable \{ color: var\(--thread-status-unavailable\); \}/);
  assert.match(css, /\.sync-thread-sync-error \{ color: var\(--thread-status-conflict\); \}/);
});

test("thread status badges use the outlined Default 24px pixel family", () => {
  const badge = "M9 1h6v1h3v1h1v1h1v1h1v1h1v3h1v6h-1v3h-1v1h-1v1h-1v1h-1v1h-3v1h-6v-1h-3v-1h-1v-1h-1v-1h-1v-1h-1v-3h-1v-6h1v-3h1v-1h1v-1h1v-1h1v-1h3ZM11 2v1h-3v1h-2v1h-1v1h-1v2h-1v3h-1v2h1v3h1v2h1v1h1v1h2v1h3v1h2v-1h3v-1h2v-1h1v-1h1v-2h1v-3h1v-2h-1v-3h-1v-2h-1v-1h-1v-1h-2v-1h-3v-1Z";
  const expected: Record<ThreadStatusIconName, { color: string; glyph: string }> = {
    synced: {
      color: "#31ae6b",
      glyph: "M16 9h2v2h-1v1h-1v2h-1v1h-1v1h-1v1h-2v-1h-1v-1h-1v-1h-1v-2h2v1h1v1h2v-1h1v-2h1v-1h1Z",
    },
    "cloud-outstanding": {
      color: "#397dcc",
      glyph: "M9 7h5v1h-5ZM9 8v2h-1v-2ZM14 8h1v2h-1ZM8 10v1h-2v-1ZM15 10h2v1h-2ZM6 11v3h-1v-3ZM17 11h1v5h-12v-2h1v1h10Z",
    },
    "local-outstanding": {
      color: "#16a7c9",
      glyph: "M11 8h3v1h1v1h1v1h1v2h-2v-1h-1v-1h-1v7h-2v-6h-1v1h-2v-2h1v-1h1v-1h1Z",
    },
    divergent: {
      color: "#7655d6",
      glyph: "M6 6h4v5h1v1h1v2h1v-1h1v-2h1v-1h-1v-4h4v4h-1v2h-1v2h-1v1h-1v1h-1v2h-2v-3h-1v-2h-1v-1h-1v-2h-2ZM7 7v2h2v-2ZM15 7v2h2v-2Z",
    },
    pending: {
      color: "#e59b1b",
      glyph: "M11 7h2v5h4v2h-6Z",
    },
    conflict: {
      color: "#e44649",
      glyph: "M11 6h3v9h-3ZM11 16h3v3h-3Z",
    },
    unavailable: {
      color: "#85878e",
      glyph: "M7 11h10v3h-10Z",
    },
    // The three neutrals say "no verdict yet", so they take --text-3 rather
    // than a signal hue: a colour here would claim an answer the comparison has
    // not given, and on an unlinked project one of these is on every row.
    untracked: {
      color: "var(--text-3)",
      glyph: "M8 8h8v8H8ZM10 14h4v-4h-4Z",
    },
    checking: {
      color: "var(--text-3)",
      glyph: "M6 11h3v3H6Z",
    },
    unknown: {
      color: "var(--text-3)",
      glyph: "M9 6h6v2H9ZM13 8h2v3h-2ZM11 11h4v2h-4ZM11 13h2v2h-2ZM11 16h2v2h-2Z",
    },
  };
  const statuses = Object.keys(expected) as ThreadStatusIconName[];

  for (const status of statuses) {
    const html = renderToStaticMarkup(<ThreadStatusIcon status={status} />);
    assert.match(html, new RegExp(`data-thread-status="${status}"`));
    assert.match(html, /thread-status-icon-badge/);
    assert.ok(html.includes(badge), `${status} must use the shared ring badge`);
    assert.ok(html.includes(expected[status].glyph), `${status} glyph geometry`);
    assert.match(html, /width="24"/);
    assert.match(html, /height="24"/);
    assert.match(html, /thread-status-icon-badge" fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    // ring and glyph both inherit the status colour, so nothing is knocked out
    assert.doesNotMatch(html, /(?:linearGradient|filter|drop-shadow|stroke=)/);
    assert.doesNotMatch(html, /--thread-status-glyph/);
  }

  // Three dots, animated in turn: one glyph pulsing as a whole reads as an
  // alarm, a cycle reads as work in progress.
  const checkingHtml = renderToStaticMarkup(<ThreadStatusIcon status="checking" />);
  assert.match(checkingHtml, /class="thread-status-icon-dots"/);
  assert.equal((checkingHtml.match(/<path fill="currentColor"/g) ?? []).length, 3);

  const css = readFileSync("src/App.css", "utf8");
  for (const status of statuses) {
    assert.match(css, new RegExp(`\\.thread-status-icon\\[data-thread-status="${status}"\\]`));
  }
  for (const { color } of Object.values(expected)) {
    assert.ok(css.includes(color));
  }
  assert.match(css, /\.thread-status-icon-dots > path \{[^}]*animation: thread-status-dot/s);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s*\.thread-status-icon-dots > path \{ animation: none;/s);
});

test("a row with no comparison entry names why, instead of drawing nothing", () => {
  const base = {
    comparisonLoaded: true,
    comparisonLoading: false,
    comparisonError: null,
    storageName: "R2 storage 1",
    hasBoundProvider: true,
  };

  // Loading wins: a refresh nulls the comparison, and the badges used to blink
  // out and back on every one of them.
  assert.deepEqual(
    unresolvedSyncPresentation({ ...base, comparisonLoading: true }),
    { status: "checking", className: "checking", label: "Comparing with R2 storage 1…" },
  );
  const failed = unresolvedSyncPresentation({ ...base, comparisonError: "NoSuchBucket: mallard-prod" });
  assert.equal(failed.status, "unknown");
  assert.match(failed.label, /NoSuchBucket: mallard-prod/);

  // No storage and no bound profile are both "untracked", but they are not the
  // same sentence — one is fixed by linking, the other by connecting.
  const noStorage = unresolvedSyncPresentation({ ...base, storageName: null, comparisonLoaded: false });
  assert.equal(noStorage.status, "untracked");
  assert.match(noStorage.label, /Link a storage/);
  const noProfile = unresolvedSyncPresentation({ ...base, hasBoundProvider: false, comparisonLoaded: false });
  assert.equal(noProfile.status, "untracked");
  assert.match(noProfile.label, /Connect an agent profile/);

  // A comparison exists and this row is simply not in it — view-only, or an
  // adapter this Mallard version does not have.
  const uncovered = unresolvedSyncPresentation(base);
  assert.equal(uncovered.status, "unknown");
  assert.match(uncovered.label, /cannot read this session well enough to match it/);

  // Every branch resolves; none can return null.
  for (const status of ["checking", "unknown", "untracked"] as const) {
    assert.ok(status.length > 0);
  }
});

test("every row leads with exactly one glyph, so the titles form a column", () => {
  // The bug this guards: the badge was conditional, and the empty "uncommitted"
  // placeholder that stood in for it was later rendered *beside* it instead of
  // instead of it. Either way some rows got a different number of leading
  // boxes, and their titles sat 26px off every other row's. Measured in a
  // browser at the time: titleX was [64, 38] before the fix and [38] after.
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} />,
  );
  const titles = html.match(/<div class="sync-history-thread-title">[\s\S]*?<(?:button|strong)/g) ?? [];
  assert.ok(titles.length > 0, "no rows rendered");
  for (const lead of titles) {
    const badges = (lead.match(/class="sync-thread-sync-indicator/g) ?? []).length;
    assert.equal(badges, 1, `exactly one status badge before the title: ${lead}`);
    // Nothing else may claim a leading 18px box on a plain browse row.
    assert.doesNotMatch(lead, /sync-history-thread-kind|sync-history-branch-state current/);
  }
});

test("fresh storage matches portable Activity rows to conversation comparisons", () => {
  const syncResourceId = `codex:conversation:${CODEX_SESSION_ID}`;
  const decision = (action: string, selectable: boolean) => ({
    action,
    reason_code: "missing_cloud",
    selectable,
  });
  const comparison = emptyComparison({
    counts: { synced: 0, local: 1, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    entries: [{
      provider: "codex",
      provider_display_name: "Codex",
      access: "bound_ready",
      thread_id: CODEX_SESSION_ID,
      resource_id: syncResourceId,
      display_name: codexSession.title,
      state: "local_only",
      local_present: true,
      storage_present: false,
      cloud_thread_id: CODEX_SESSION_ID,
      provider_thread_id: CODEX_SESSION_ID,
      push_decision: decision("create_cloud", true),
      pull_decision: decision("report_local_only", false),
    }],
  });
  const browseHtml = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null }}
      comparison={comparison}
      activeStorageName="Local storage 1"
    />,
  );

  assert.match(browseHtml, /aria-label="Only on this computer\. Push to save it in Local storage 1\."/);
  assert.doesNotMatch(browseHtml, /cannot read this session well enough to match it/);

  const reviewHtml = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null }}
      comparison={comparison}
      activeStorageName="Local storage 1"
      selectionMode="push"
      selectedResourceIds={new Set([syncResourceId])}
      selectableResourceIds={new Set([syncResourceId])}
      onToggleResource={() => undefined}
    />,
  );

  assert.match(reviewHtml, /aria-label="Exclude Add Git history mapping"/);
  assert.match(reviewHtml, /<input[^>]*checked=""/);
});

test("an unlinked project still draws a badge on every row", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} />,
  );
  const rows = html.match(/<article class="sync-history-thread-card[^"]*">/g) ?? [];
  assert.ok(rows.length > 0);
  assert.equal(
    (html.match(/class="sync-thread-sync-indicator untracked"/g) ?? []).length,
    rows.length,
    "one untracked badge per row when there is no storage to compare against",
  );
  assert.match(html, /aria-label="Only on this Mac\. Link a storage to compare and sync it\."/);
});

test("refreshing project activity spins in the path bar, and announces from the list", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={activity}
      loading
      comparisonLoading
      activeStorageName="R2 storage 1"
    />,
  );

  // The button moved to the path bar with the rest of the header. What has to
  // stay with the list is the announcement, because it describes this list's
  // work — and it is the only thing a screen reader had from that row.
  assert.doesNotMatch(html, /data-icon="refresh"/);
  assert.doesNotMatch(html, /icon-spin-frame/);
  assert.match(html, /role="status">Comparing threads with R2 storage 1/);

  // One glyph, scoped to the view: it points at the open session when there is
  // one, and at the activity comparison otherwise.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /className=\{`sync-path-icon-action\$\{viewRefresh\.refreshing \? " loading" : ""\}`\}/);
  assert.match(workspace, /aria-busy=\{viewRefresh\.refreshing\}/);
  assert.match(workspace, /className=\{`icon-spin-frame\$\{viewRefresh\.refreshing \? " spinning" : ""\}`\}/);
  assert.match(workspace, /"Refreshing session" : "Refresh session"/);
  assert.match(workspace, /"Refreshing activity" : "Refresh activity"/);

  const css = readFileSync("src/App.css", "utf8");
  // Turning is work in progress, not a disabled control: the one place a
  // disabled button is allowed to keep full opacity.
  assert.match(css, /\.sync-path-icon-action\.loading:disabled\s*\{[^}]*opacity:\s*1;/s);
  assert.match(css, /\.icon-spin-frame\s*\{[^}]*width:\s*16px;[^}]*height:\s*16px;[^}]*transform-origin:\s*8px 8px;/s);
  assert.match(css, /\.icon-spin-frame\.spinning\s*\{[^}]*animation:\s*icon-spin 0\.8s linear infinite;/s);
});

test("sync tooltip hover keeps the sibling icon rail geometry stable", () => {
  const css = readFileSync("src/App.css", "utf8");
  const titleRule = css.match(/\.sync-history-thread-title\s*\{[^}]+\}/s)?.[0] ?? "";
  const tooltipRule = css.match(/\.sync-thread-sync-indicator::after,[\s\S]+?\n\}/)?.[0] ?? "";

  assert.match(titleRule, /align-items:\s*center/);
  assert.doesNotMatch(tooltipRule, /transform:/);
  // The metric tooltips need the same treatment: animating a transform promotes
  // them to their own compositing layer, which re-rasterises the text around
  // them and reads as the row — and the panel — twitching on hover.
  // Guarded on [data-tooltip]: a metric without one (the outcome list's
  // stamp) must not paint an empty tooltip box or invite a help cursor.
  const metricTooltipRule = css.match(/\.sync-history-thread-metric\[data-tooltip\]::after\s*\{[^}]+\}/s)?.[0] ?? "";
  const metricHoverRule = css.match(/\.sync-history-thread-metric\[data-tooltip\]:hover::after,[\s\S]+?\n\}/)?.[0] ?? "";
  assert.doesNotMatch(css, /\.sync-history-thread-metric::after\s*\{/, "the unguarded tooltip selector must not come back");
  assert.match(metricTooltipRule, /position:\s*absolute/);
  assert.doesNotMatch(metricTooltipRule, /transform:/);
  assert.doesNotMatch(metricHoverRule, /transform:/);
  // The stamp's tooltip carries two lines separated by a newline; nowrap
  // collapsed them into one run-on line, and pre-line soft-wrapped mid-sentence.
  assert.match(metricTooltipRule, /white-space: pre;/);
  // Menus and panels in the base sheet open with a keyframe that slides and
  // scales them in. Animating a transform promotes the element to its own
  // compositing layer, and creating that layer re-rasterises the icons and
  // text beside it — the twitch that shows when a menu opens. The skin drops
  // the reveals and lets chevrons rotate instantly.
  const retroCssMotion = readFileSync("src/RetroTheme.css", "utf8");
  const revealsOff = retroCssMotion.match(/\.sync-history-branch-menu,[\s\S]*?\{[^}]*animation: none;[^}]*\}/)?.[0] ?? "";
  for (const selector of ["\\.storage-link-menu", "\\.storage-link-actions-menu", "\\.sync-capability-details", "\\.sync-project-tab-panel"]) {
    assert.match(revealsOff, new RegExp(selector), `${selector} reveal must be disabled`);
  }
  const chevrons = retroCssMotion.match(/\.sync-history-branch-chevron,[\s\S]*?\{[^}]*\}/)?.[0] ?? "";
  assert.match(chevrons, /transition: color 120ms ease;/);
  assert.doesNotMatch(chevrons, /transition:[^;]*\btransform\b/);

  // The retro press shifts a button a pixel while held. On the timeline's
  // 22-26px icon buttons that reads as the icon jogging against its
  // neighbours on every click, so they keep the colour feedback and drop the
  // displacement. Full-size buttons still get the press.
  const retroCssPress = readFileSync("src/RetroTheme.css", "utf8");
  assert.match(retroCssPress, /\.btn:active:not\(:disabled\)\s*\{[^}]*transform: translate\(1px, 1px\);/s);
  const pressExempt = retroCssPress.match(/\.sync-history-thread-actions \.sync-history-launch-action:active[\s\S]*?\{[^}]*\}/)?.[0] ?? "";
  assert.match(pressExempt, /transform: none;/);
  for (const selector of [
    "\\.sync-history-debug-trigger:active",
    "\\.sync-history-commit-toggle:active",
    "\\.sync-history-icon-action:active",
    // Icon-only clickers elsewhere in the app, found by cross-checking every
    // \`.btn\` for a fixed square size or zero padding.
    "\\.sync-inline-action-close:active",
    "\\.log-panel-actions \\.btn:active",
    "\\.sync-modal-header \\.btn:active",
  ]) {
    assert.match(pressExempt, new RegExp(selector), `${selector} must be exempt from the press displacement`);
  }

  // It is pinned to the row's right edge, so its tooltip must open inward.
  // The stamp is a direct child of the summary row now (it trails the
  // clickable pair), so the inward-opening tooltip targets it there.
  assert.match(css, /\.sync-history-session-summary > \.sync-history-thread-time-end::after\s*\{[^}]*right: -6px;[^}]*left: auto;/s);

  // The tooltip is a pseudo-element sitting above its metric, so any clipping
  // ancestor renders it invisible — all that shows is a sliver at the clip
  // edge. The row is held to one line by truncating the timestamp instead.
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");
  for (const selector of [
    "\\.sync-history-session-details",
    "\\.sync-history-session-summary",
    "\\.sync-history-thread-meta",
  ]) {
    const rule = retroCss.match(new RegExp(`\\n${selector} \\{[^}]*\\}`))?.[0] ?? "";
    assert.doesNotMatch(rule, /overflow:\s*(hidden|clip|auto|scroll)/, `${selector} must not clip the metric tooltip`);
  }
  assert.match(retroCss, /\.sync-history-thread-time time\s*\{[^}]*overflow: hidden;[^}]*text-overflow: ellipsis;/s);

  // Neither tooltip animates. An opacity or transform transition promotes it to
  // its own compositing layer, and text re-rasterised against a layer origin can
  // land a pixel off the page grid — which the unsmoothed pixel face shows as a
  // twitch even though no geometry changes.
  assert.doesNotMatch(tooltipRule, /transition:/);
  assert.doesNotMatch(metricTooltipRule, /transition:/);
  // A scrollbar appearing must not push the workspace sideways.
  assert.match(css, /\.sync-main\s*\{[^}]*overflow: auto;[^}]*scrollbar-gutter: stable;/s);
  assert.doesNotMatch(
    css,
    /\.sync-thread-sync-indicator:hover,\s*\n\.sync-thread-sync-indicator:focus-visible,\s*\n\.sync-thread-sync-error:hover/,
  );
});

test("Push and Pull reviews use only the outer workspace scroll", () => {
  const css = readFileSync("src/App.css", "utf8");
  const workspaceRule = css.match(/\.sync-main\.sync-sync-review-page\s*\{[^}]+\}/s)?.[0] ?? "";
  const reviewScrollRule = css.match(/\.sync-sync-review-page \.sync-sync-review-scroll\s*\{[^}]+\}/s)?.[0] ?? "";
  const bannerZoneRule = css.match(/\.sync-review-banners\s*\{[^}]+\}/s)?.[0] ?? "";
  const workspaceSource = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const pushSource = readFileSync("src/components/project-sync/PushResourceWorkspace.tsx", "utf8");
  const pullSource = readFileSync("src/components/project-sync/RestorePlanView.tsx", "utf8");
  const tabsSource = readFileSync("src/components/sync-review/SyncReviewTabs.tsx", "utf8");

  assert.match(workspaceRule, /overflow-y:\s*auto/);
  assert.doesNotMatch(workspaceRule, /overflow:\s*hidden/);
  assert.match(reviewScrollRule, /max-height:\s*none/);
  assert.match(reviewScrollRule, /overflow:\s*visible/);
  assert.doesNotMatch(reviewScrollRule, /overflow-y:\s*(auto|scroll)/);
  assert.match(bannerZoneRule, /display:\s*grid/, "the banner zone owns spacing at the top of the scroll");
  assert.match(workspaceSource, /sync-sync-review-page/);
  assert.doesNotMatch(pushSource, /useSyncReviewScroll|onScroll=|ref=\{scrollRef\}/);
  assert.doesNotMatch(pullSource, /useSyncReviewScroll|onScroll=|ref=\{scrollRef\}/);
  assert.doesNotMatch(tabsSource, /useSyncReviewScroll/);

  const pullReviewHandler = workspaceSource.slice(
    workspaceSource.indexOf("if (pullReviewOpen)"),
    workspaceSource.indexOf("setStoragePickerProjectId(null)", workspaceSource.indexOf("if (pullReviewOpen)")),
  );
  const pushReviewHandler = workspaceSource.slice(
    workspaceSource.indexOf("if (pushReviewOpen)"),
    workspaceSource.indexOf("setStoragePickerProjectId(null)", workspaceSource.indexOf("if (pushReviewOpen)")),
  );
  assert.doesNotMatch(pullReviewHandler, /scrollIntoView/);
  assert.doesNotMatch(pushReviewHandler, /scrollIntoView/);
});

test("the combined project header stays aligned with Storage and shares its divider", () => {
  const css = readFileSync("src/App.css", "utf8");
  const combinedHeadingRule = css.match(/\.sync-project-combined-page \.sync-combined-project-heading\s*\{[^}]+\}/s)?.[0] ?? "";
  const storageGroupRule = css.match(/\.sync-project-combined-page \.project-storage-group\s*\{[^}]+\}/s)?.[0] ?? "";

  assert.match(combinedHeadingRule, /align-items:\s*stretch/);
  assert.match(combinedHeadingRule, /margin-bottom:\s*0/);
  assert.match(combinedHeadingRule, /border-bottom:\s*0/);
  assert.match(storageGroupRule, /border-top:\s*1px/);
});

test("activity requests share concurrent work but not completed results", async () => {
  const singleFlight = createSingleFlight();
  let starts = 0;
  let resolveRequest!: (value: string) => void;
  const pending = new Promise<string>((resolve) => {
    resolveRequest = resolve;
  });
  const start = () => {
    starts += 1;
    return pending;
  };

  const first = singleFlight("project-activity", start);
  const duplicate = singleFlight("project-activity", start);
  assert.strictEqual(first, duplicate);
  assert.equal(starts, 1);

  resolveRequest("complete");
  assert.deepEqual(await Promise.all([first, duplicate]), ["complete", "complete"]);
  assert.equal(await singleFlight("project-activity", async () => {
    starts += 1;
    return "refreshed";
  }), "refreshed");
  assert.equal(starts, 2);
});

test("activity content uses the local alias and renders commit and session rows", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={activity}
      onOpenSessionDetail={() => undefined}
    />,
  );
  const commitHeading = html.match(/<div class="sync-history-commit-heading">([\s\S]*?)<\/div>/)?.[1] ?? "";
  assert.doesNotMatch(html, /Project Name/);
  // The alias is the path bar's to say. The list used to print it again as an
  // <h1> directly under the crumb that already carried it.
  assert.doesNotMatch(html, /Mallard local/);
  assert.match(html, /sync-history-project-context/);
  // The provider-neutral page no longer prints a provider home path; identity
  // and provenance live in the per-row debug panel instead.
  assert.doesNotMatch(html, /Codex configuration:/);
  assert.match(html, /Local storage 1/);
  assert.match(html, /aria-label="Last pull:/);
  assert.match(html, /aria-label="Last push:/);
  assert.match(html, /Repository: mallard/);
  assert.match(html, />Add project-scoped history</);
  assert.doesNotMatch(html, /sync-history-commit-metrics/);
  assert.match(commitHeading, /data-icon="git-commit"/);
  assert.match(commitHeading, /class="sync-history-commit-title"/);
  assert.match(commitHeading, /class="sync-history-commit-meta"/);
  assert.match(commitHeading, /<strong title="Add project-scoped history">Add project-scoped history<\/strong>/);
  assert.ok(commitHeading.indexOf('data-icon="git-commit"') < commitHeading.indexOf("<strong"));
  assert.match(commitHeading, /<code>bbbbbbb<\/code>/);
  assert.match(commitHeading, /<time dateTime=/);
  // The fold lives in the rail, before the heading in the DOM, not in a
  // trailing actions column — the heading carries no buttons at all.
  assert.doesNotMatch(commitHeading, /sync-history-commit-actions|sync-history-commit-toggle/);
  assert.match(html, /class="sync-history-spine-fold"[^>]*aria-label="Collapse commit bbbbbbb"[^>]*aria-expanded="true"[^>]*aria-controls="commit-sessions-bbbbbbb"/);
  assert.match(html, /class="sync-history-spine-fold"[^>]*>\s*<svg[^>]*data-icon="chevron-down"/);
  assert.ok(html.indexOf("sync-history-spine-fold") < html.indexOf('class="sync-history-commit-heading"'), "the fold precedes the content it controls");
  // Identifier before stamp: the linked sha chip leads with the external-link
  // glyph, and the timestamp trails the row to join the session stamp column.
  assert.match(html, /<a class="sync-history-commit-link"[^>]+><svg[^>]*data-icon="external-link"[\s\S]*?<code>bbbbbbb<\/code><\/a><time dateTime=/);
  assert.match(html, new RegExp(`href="https://github\\.com/Git4Agent/mallard/commit/${"b".repeat(40)}"`));
  assert.match(html, /aria-label="Open commit bbbbbbb on GitHub"/);
  // Provider identity is the mark on the button that opens that provider, not a
  // text pill: a pill is as wide as the name it holds, so with more than one
  // provider in the list the titles stopped lining up row to row.
  assert.doesNotMatch(html, /sync-history-branch-state current" title="[^"]*session"/);
  assert.match(html, /aria-label="Open in Codex"><svg[^>]*data-icon="openai"/);
  // No location line. "On this Mac" was on every row of an unlinked project and
  // never varied; on a linked one the status badge already encodes it.
  assert.doesNotMatch(html, /sync-history-stored-thread-location/);
  assert.doesNotMatch(html, /On this Mac/);
  assert.doesNotMatch(html, /sync-history-thread-updated/);
  assert.doesNotMatch(html, /aria-label="Updated [^"]+"/);
  assert.doesNotMatch(html, /aria-label="Show session details"/);
  // Both endpoints are announced together on the single visible stamp.
  assert.match(html, /aria-label="Started [^"]+\. Ended [^"]+"/);
  assert.match(html, /aria-label="User turns: 3"/);
  assert.match(html, /data-tooltip="Total tokens · 24\.8K"/);
  assert.match(html, /aria-label="1 session linked to commits"/);
  assert.match(html, /class="sync-history-session-title-button"/);
  assert.match(html, /aria-label="View session details"/);
  assert.doesNotMatch(html, /during session|after session|started from/);
  assert.doesNotMatch(html, /Map project-owned sessions onto/);
  // Row actions come from the backend descriptors, labelled by the adapter.
  assert.match(html, /aria-label="Open in Codex"/);
  assert.match(html, /aria-label="Open in Terminal"/);
  // The action icon is keyed off the descriptor kind, not off a provider mark.
  // The knot is back on the open button, where it sat before the row went
  // provider-neutral — but keyed off the provider now, not hardcoded, and the
  // legacy hand-tuned class is gone.
  assert.doesNotMatch(html, /sync-openai-icon/);
  assert.match(html, /aria-label="Show session debug info"/);
  assert.match(html, /aria-haspopup="dialog"/);
  assert.match(html, /aria-label="Session debug info"/);
  assert.match(html, /aria-label="Copy resource id"/);
  assert.match(html, /aria-label="Copy provider session id"/);
  assert.match(html, /aria-label="Copy working directory"/);
  assert.doesNotMatch(html, /aria-label="Copy codex home"/);
  assert.match(html, /\/Users\/test\/projects\/mallard\/packages\/desktop/);
  assert.match(html, new RegExp(`<code>${codexResourceId}</code>`));
  assert.match(html, />Copy all</);
  // Line 1 carries the backend actions only — the ways to open this session
  // somewhere else. The two row-level clickables belong to line 2.
  {
    const cluster = html.slice(
      html.indexOf('class="sync-history-thread-actions"'),
      html.indexOf('class="sync-history-session-summary"'),
    );
    const positions = [
      cluster.indexOf('aria-label="Open in Codex"'),
      cluster.indexOf('aria-label="Open in Terminal"'),
    ];
    assert.ok(positions.every((position) => position >= 0), `cluster pieces missing: ${positions}`);
    assert.deepEqual(positions, [...positions].sort((a, b) => a - b), "cluster order is open, then terminal");
    assert.doesNotMatch(cluster, /Show session debug info|View session details/);
  }
  // Line 2 reads readouts │ debug detail ……… stamp: a rule parts the glyphs you
  // can only hover from the two you can click, and the stamp still closes the
  // line at the card edge so it forms one column with every other stamp.
  {
    const row = html.slice(
      html.indexOf('class="sync-history-session-summary"'),
      html.indexOf("</article>", html.indexOf('class="sync-history-session-summary"')),
    );
    const order = [
      row.indexOf('class="sync-history-thread-meta"'),
      row.indexOf("sync-history-actions-divider"),
      row.indexOf('aria-label="Show session debug info"'),
      row.indexOf('aria-label="View session details"'),
      row.indexOf("sync-history-thread-time-end"),
    ];
    assert.ok(order.every((position) => position >= 0), `line 2 pieces missing: ${order}`);
    assert.deepEqual(order, [...order].sort((a, b) => a - b), "readouts, rule, debug, detail, stamp last");
  }
  // The rule is decorative — it must not be announced as content.
  assert.match(html, /<span class="sync-history-actions-divider" aria-hidden="true"><\/span>/);
  assert.match(html, /aria-label="View session details"[^>]*>\s*<svg[^>]*data-icon="chevron-right"/);
  assert.doesNotMatch(html, /aria-label="View session details"[^>]*aria-expanded=/);
  assert.doesNotMatch(html, /aria-label="1 sessions"/);
  assert.doesNotMatch(html, />Open in Codex</);
  assert.doesNotMatch(html, /> Open in Terminal</);
  assert.doesNotMatch(html, /Load chat history|sync-history-chat-details/);
});

test("changed storage history explains the recovery path and exposes only unlink", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={activity}
      comparison={emptyComparison({
        revision: 2,
        base_revision: 1,
        remote_continuity: {
          state: "replaced",
          base_revision: 1,
          remote_revision: 2,
        },
      })}
      onUnlinkStorage={() => undefined}
    />,
  );

  assert.match(html, />Storage history changed</);
  assert.match(html, /sync history in Local storage 1 is different from the one Mallard last synced for this project \(revision 1\)/);
  assert.match(html, /sync data in that storage was deleted and created again/);
  assert.match(html, /session statuses below use the replacement/);
  assert.match(html, /Unlinking does not delete files/);
  assert.match(html, /saved only in the previous storage history cannot be recovered from this machine/);
  assert.doesNotMatch(html, />Pull and review<\/button>/);
  assert.match(html, />Unlink storage<\/button>/);
  assert.doesNotMatch(html, /Remote history was replaced/);
});

test("commit hashes remain plain text without a GitHub remote", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: { ...activity.git, github_repository_url: null } }}
    />,
  );

  assert.match(html, new RegExp(`<code title="${"b".repeat(40)}">bbbbbbb</code>`));
  assert.doesNotMatch(html, /aria-label="Open commit .* on GitHub"/);
  assert.doesNotMatch(html, /\/commit\/b{40}/);
});

test("permanent session summaries label every metric icon", () => {
  const html = renderToStaticMarkup(<SessionMetrics session={codexSession} />);
  const endedTime = html.match(/sync-history-thread-time-end[\s\S]*?<time[^>]*>([^<]+)<\/time>/)?.[1] ?? "";

  // The row anchors on the last activity — the more useful end of the interval
  // — plus how long it ran. Start is that minus the duration, and both ends are
  // spelled out in full in the accessible label and tooltip.
  assert.match(html, new RegExp(`<time dateTime="${new Date(codexSession.ended_at * 1_000).toISOString()}"`));
  assert.doesNotMatch(html, new RegExp(`<time dateTime="${new Date(codexSession.started_at * 1_000).toISOString()}"`));
  assert.match(html, /aria-label="Started [^"]+\. (?:Ended|Last activity) [^"]+"/);
  assert.match(endedTime, / at /);
  assert.match(endedTime, /\d{4}/);
  assert.match(endedTime, /\d{1,2}:\d{2}/);
  assert.doesNotMatch(endedTime, /\d{1,2}:\d{2}:\d{2}/);
  assert.match(html, /class="sync-history-thread-metric sync-history-thread-duration"[^>]*aria-label="Ran for 1h"/);
  assert.match(html, /data-icon="clock"/);
  // Only one stamp, and none of the old paired-range markup.
  assert.doesNotMatch(html, /sync-history-thread-time-start|sync-history-range-arrow/);
  assert.match(html, /data-tooltip="User turns · 3"/);
  assert.match(html, /data-tooltip="Total tokens · 24\.8K"/);
  assert.match(html, /data-tooltip="Agent messages · 5"/);
  assert.match(html, /data-tooltip="Tool calls · 8"/);
});

test("a sub-minute session still reports how long it ran", () => {
  const startedAt = Math.floor(new Date(2026, 7, 1, 18, 53, 0).getTime() / 1_000);
  const html = renderToStaticMarkup(
    <SessionMetrics session={{ ...codexSession, started_at: startedAt, ended_at: startedAt + 41 }} />,
  );
  const ended = html.match(/sync-history-thread-time-end[\s\S]*?<time[^>]*>([^<]+)<\/time>/)?.[1] ?? "";

  assert.match(ended, / at /);
  // Seconds, not a rounded-to-zero minute reading.
  assert.match(html, /sync-history-thread-duration[^>]*aria-label="Ran for 41s"/);
  assert.match(html, /<b>41s<\/b>/);
  // The full ending timestamp still reaches assistive tech and the tooltip.
  assert.match(html, /aria-label="Started [^"]+\. Ended [^"]+"/);
});

test("a session with no measurable duration renders a single timestamp", () => {
  const startedAt = Math.floor(new Date(2026, 7, 1, 18, 53, 0).getTime() / 1_000);
  const html = renderToStaticMarkup(
    <SessionMetrics session={{ ...codexSession, started_at: startedAt, ended_at: startedAt }} />,
  );

  assert.match(html, /sync-history-thread-time-end/);
  assert.doesNotMatch(html, /sync-history-thread-time-start/);
  assert.doesNotMatch(html, /sync-history-range-arrow/);
  assert.doesNotMatch(html, /sync-history-thread-duration/);
});

test("a session that crosses midnight still resolves to an unambiguous end", () => {
  const startedAt = Math.floor(new Date(2026, 6, 30, 23, 55).getTime() / 1_000);
  const endedAt = Math.floor(new Date(2026, 6, 31, 0, 5).getTime() / 1_000);
  const html = renderToStaticMarkup(
    <SessionMetrics session={{ ...codexSession, started_at: startedAt, ended_at: endedAt }} />,
  );
  const endedTime = html.match(/sync-history-thread-time-end[\s\S]*?<time[^>]*>([^<]+)<\/time>/)?.[1] ?? "";

  // The visible stamp is the far side of the boundary; the duration walks back
  // to the start, so the interval stays unambiguous with one date on screen.
  assert.match(endedTime, /Jul 31, 2026/);
  assert.match(endedTime, / at /);
  assert.match(html, /sync-history-thread-duration[^>]*aria-label="Ran for 10m"/);
  // The full ending date is still spelled out for assistive tech and the tooltip.
  assert.match(html, /aria-label="Started [^"]+\. Ended [^"]+2026[^"]*"/);
});

test("session actions stay compact and trail the readouts", () => {
  const css = readFileSync("src/App.css", "utf8");
  const retro = readFileSync("src/RetroTheme.css", "utf8");
  const metaRule = css.match(/\.sync-history-session-summary \.sync-history-thread-meta\s*\{[^}]+\}/s)?.[0] ?? "";
  const launchRule = css.match(/\.sync-history-thread-actions \.sync-history-launch-action\s*\{[^}]+\}/s)?.[0] ?? "";
  const debugMenuRule = css.match(/\.sync-history-debug-menu\s*\{[^}]+\}/s)?.[0] ?? "";
  const debugPopoverRule = css.match(/\.sync-history-debug-popover\s*\{[^}]+\}/s)?.[0] ?? "";

  assert.match(metaRule, /flex:\s*0 1 auto/);
  assert.match(launchRule, /width:\s*28px/);
  assert.match(launchRule, /padding:\s*0/);
  // The debug panel and the detail chevron sit on line 2 after the readouts, so
  // a hairline has to part what you hover from what you click. It is a plain
  // 1px rule, never a glyph that could read as another readout.
  assert.match(css, /\.sync-history-actions-divider\s*\{[^}]*width: 1px;[^}]*height: 14px;[^}]*flex: 0 0 auto;[^}]*background: var\(--border-mid\);/s);
  assert.match(css, /\.sync-history-row-clickables\s*\{[^}]*display: flex;[^}]*flex: 0 0 auto;/s);
  // The inert provider mark takes a launch button's box, so the cluster is the
  // same width whether or not the row can be opened — and the session header's
  // launch pair shares the group, so both surfaces move together.
  assert.match(
    retro,
    /\.sync-history-thread-actions \.sync-history-launch-action,\s*\n\.sync-history-thread-actions \.sync-history-provider-inert,\s*\n\.session-detail-launch\s*\{[^}]*width:\s*22px;/s,
  );
  assert.match(retro, /\.sync-history-session-summary > \.sync-history-thread-time-end\s*\{[^}]*margin-left: auto;/s);
  assert.doesNotMatch(retro, /\.sync-history-session-summary \.sync-history-thread-meta\s*\{[^}]*flex:\s*1/s, "the readouts must not stretch into the clickables");
  // The menu is deliberately static so the popover anchors to the row instead
  // of to the 22px trigger — anchored to the trigger it was pushed outside the
  // card and clipped by the workspace scroll container.
  assert.match(debugMenuRule, /position:\s*static/);
  assert.match(debugPopoverRule, /position:\s*absolute/);
  assert.match(debugPopoverRule, /width: min\(380px, calc\(100% - 24px\)\)/);
  assert.doesNotMatch(debugPopoverRule, /100vw/);
});

test("session debug copy text keeps complete portable identifiers", () => {
  assert.equal(
    sessionDebugCopyText(
      codexResourceId,
      CODEX_SESSION_ID,
      codexSession.working_directory,
    ),
    [
      `Resource ID: ${codexResourceId}`,
      `Provider session ID: ${CODEX_SESSION_ID}`,
      `Working directory: ${codexSession.working_directory}`,
    ].join("\n"),
  );
  // A remote-only row can lack a provider-native id; the copy says so rather
  // than printing a blank.
  assert.equal(
    sessionDebugCopyText("claude:conversation:thread-1", null, "/Users/test/projects/mallard"),
    [
      "Resource ID: claude:conversation:thread-1",
      "Provider session ID: Not recorded",
      "Working directory: /Users/test/projects/mallard",
    ].join("\n"),
  );
});

test("browse-mode session titles navigate without retaining the inline preview", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null }}
      onOpenSessionDetail={() => undefined}
    />,
  );

  assert.match(html, /class="sync-history-session-title-button"[^>]*>Add Git history mapping</);
  assert.match(html, /aria-label="View session details"/);
  assert.match(html, /data-tooltip="User turns · 3"/);
  assert.doesNotMatch(html, /sync-history-chat-details|Load 10 older messages|Nested conversation preview/);
  assert.doesNotMatch(html, /aria-expanded="(?:true|false)"[^>]*View session details/);
});

test("embedded activity follows the project workspace without repeating project controls", () => {
  const activityWithUncommitted = {
    ...activity,
    sessions: [{
      ...codexSession,
      title: "Test conversation — fork · 2026-08-01 09:00Z",
    }],
    unmapped: [{
      resource_id: codexResourceId,
      reason: "No subsequent commit",
    }],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={activityWithUncommitted}
    />,
  );

  assert.match(html, /sync-history-embedded/);
  // The workspace path bar's section crumb is the visible name for this page,
  // so the embedded header keeps its controls and drops its title; the region
  // still carries an accessible name.
  assert.doesNotMatch(html, /sync-history-embedded-title/);
  assert.match(html, /aria-label="Git history"/);
  assert.match(html, /class="sync-history-timeline has-uncommitted"/);
  assert.match(html, /<h2 id="uncommitted-heading"><svg[^>]*data-icon="edit"[^>]*>.*<\/svg>Uncommitted<\/h2>/s);
  // The unmapped reason is bookkeeping, not something the reader needs on screen.
  assert.doesNotMatch(html, /No subsequent commit/);
  // The empty placeholder is gone. It existed to reserve the 18px box while
  // the status badge could be absent; the badge is never absent now, and a
  // second glyph beside it pushed these rows' titles 26px right of every other
  // row's — measured, not guessed. The group heading still carries the pencil.
  assert.doesNotMatch(html, /sync-history-thread-kind/);
  const uncommittedRow = html.slice(html.indexOf('id="uncommitted-list"'));
  assert.match(uncommittedRow, /class="sync-thread-sync-indicator untracked"/);
  assert.match(html, /<strong title="Test conversation — fork · 2026-08-01 09:00Z">Test conversation — fork<\/strong>/);
  // The count badge is gone from the heading; the fold control carries the
  // count so the group is still announced with its size.
  assert.match(html, /aria-label="Collapse 1 session not linked to a commit"/);
  assert.match(html, /class="sync-history-spine-fold"[^>]*aria-label="Collapse 1 session not linked to a commit"[^>]*aria-expanded="true"[^>]*aria-controls="uncommitted-list"/);
  // The status legend is gone; the per-row tooltips carry the same meanings.
  assert.doesNotMatch(html, /sync-history-status-legend/);
  assert.doesNotMatch(html, /Cloud outstanding/);
  assert.doesNotMatch(html, />Commit history</);
  assert.doesNotMatch(html, /Sessions not linked to a commit/);
  assert.doesNotMatch(html, /<main/);
  assert.doesNotMatch(html, /aria-label="Project settings"/);
  assert.doesNotMatch(html, /sync-history-project-context/);
  assert.doesNotMatch(html, /Local storage 1/);
});

test("Activity badges only the receipt-proven preserved side of a divergent Pull", () => {
  const mintedTitle = "Respond to greeting — local before Pull · 2026-08-02 07:54Z";
  const renderSession = (isPreservedLocal: boolean) => renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{
        ...activity,
        git: null,
        sessions: [{
          ...codexSession,
          title: mintedTitle,
          is_preserved_local: isPreservedLocal,
        }],
      }}
    />,
  );

  const preserved = renderSession(true);
  assert.match(preserved, /class="sync-history-preserved-badge"/);
  assert.match(preserved, /title="Local branch preserved during a divergent Pull\."/);
  assert.match(preserved, />Preserved<\/span>/);
  assert.match(preserved, /Respond to greeting — local before Pull/);

  const titleOnly = renderSession(false);
  assert.doesNotMatch(titleOnly, /sync-history-preserved-badge|>Preserved<\/span>/);
});

test("embedded non-Git activity is named once, and not on screen", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null }}
    />,
  );

  // The path bar says "Sessions" once. Nothing here repeats it as a heading,
  // and only the outer region carries it as an accessible name — the list
  // inside would otherwise announce the same word a second time.
  assert.doesNotMatch(html, />Sessions</);
  assert.equal(html.match(/aria-label="Sessions"/g)?.length, 1);
  assert.doesNotMatch(html, />Activity</);
  assert.doesNotMatch(html, /Codex threads/);
  // The tally went with the header. It counted the rows on screen, which is
  // not what Push and Pull act on; the bar counts the whole comparison.
  assert.doesNotMatch(html, /aria-label="1 session"/);
});

test("selection review removes repeated history and storage labels", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={activity}
      activeStorageName="R2 storage 2"
      selectionMode="push"
    />,
  );

  assert.match(html, /sync-history-selection-review/);
  assert.match(html, /aria-label="Git history"/);
  assert.doesNotMatch(html, /sync-history-header/);
  assert.doesNotMatch(html, />Local session</);
  assert.doesNotMatch(html, /sync-history-storage-lens/);
  assert.doesNotMatch(html, /sync-history-toolbar-count/);
  assert.doesNotMatch(html, /aria-label="Branch"/);
  assert.doesNotMatch(html, /aria-label="Refresh activity"/);

  const css = readFileSync("src/App.css", "utf8");
  const compactReviewRule = css.match(/\.sync-sync-review-scroll > \.sync-history-selection-review\s*\{[^}]+\}/s)?.[0] ?? "";
  const compactHeadingRule = css.match(/\.sync-history-selection-review \.sync-history-section-heading\s*\{[^}]+\}/s)?.[0] ?? "";
  assert.match(compactReviewRule, /padding-top:\s*0/);
  assert.match(compactReviewRule, /padding-right:\s*8px/);
  assert.match(compactHeadingRule, /margin-top:\s*8px/);
});

test("Push review renders checkboxes only for selectable bound-ready rows", () => {
  const localAheadId = "019f7798-5437-7632-9dbc-5b589cf68bf1";
  const localAheadResourceId = `codex:conversation:${localAheadId}`;
  const syncedSyncResourceId = `codex:conversation:${CODEX_SESSION_ID}`;
  const localAheadSyncResourceId = `codex:conversation:${localAheadId}`;
  const localAheadSession = {
    ...codexSession,
    resource_id: localAheadResourceId,
    provider_session_id: localAheadId,
    logical_thread_id: localAheadId,
    title: "Continue local work",
    started_at: codexSession.started_at + 1,
    ended_at: codexSession.ended_at + 1,
  };
  const decision = (action: string, selectable: boolean) => ({
    action,
    reason_code: "sync_current",
    selectable,
  });
  const comparison = emptyComparison({
    counts: { synced: 1, local: 1, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    entries: [
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: CODEX_SESSION_ID,
        resource_id: syncedSyncResourceId,
        display_name: codexSession.title,
        state: "synced",
        local_present: true,
        storage_present: true,
        cloud_thread_id: CODEX_SESSION_ID,
        provider_thread_id: CODEX_SESSION_ID,
        push_decision: decision("no_changes", false),
        pull_decision: decision("no_changes", false),
      },
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: localAheadId,
        resource_id: localAheadSyncResourceId,
        display_name: localAheadSession.title,
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        cloud_thread_id: localAheadId,
        provider_thread_id: localAheadId,
        push_decision: { ...decision("publish", true), reason_code: "local_outstanding" },
        pull_decision: { ...decision("report_push_eligible", false), reason_code: "local_outstanding" },
      },
    ],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null, sessions: [codexSession, localAheadSession] }}
      comparison={comparison}
      activeStorageName="Local storage 1"
      selectionMode="push"
      selectableResourceIds={new Set([localAheadSyncResourceId])}
    />,
  );

  const syncedInput = html.match(/<input[^>]*aria-label="Include Add Git history mapping"[^>]*>/)?.[0] ?? "";
  const localAheadInput = html.match(/<input[^>]*aria-label="Include Continue local work"[^>]*>/)?.[0] ?? "";
  assert.equal(syncedInput, "");
  assert.notEqual(localAheadInput, "");
  assert.doesNotMatch(localAheadInput, /disabled=""/);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 1);
  assert.equal((html.match(/sync-sync-selectable-row/g) ?? []).length, 1);
  assert.doesNotMatch(html, /sync-selection-placeholder/);
  // The fork maintenance action never renders inside a review selection.
  assert.doesNotMatch(html, /aria-label="Fork in Codex"/);
});

test("selected storage adds directional indicators and remote-only session rows", () => {
  const remoteId = "019f7798-5437-7632-9dbc-5b589cf68bf1";
  const remoteResourceId = `codex:conversation:${remoteId}`;
  const localSyncResourceId = `codex:conversation:${CODEX_SESSION_ID}`;
  const remoteSyncResourceId = `codex:conversation:${remoteId}`;
  const olderId = "019f7798-5437-7632-9dbc-5b589cf68bf2";
  const remoteSession = {
    ...codexSession,
    resource_id: remoteResourceId,
    provider_session_id: null,
    logical_thread_id: remoteId,
    source: "remote_only" as const,
    title: "Improve cloud thread titles",
    started_at: 1_752_802_000,
    ended_at: 1_752_802_500,
    working_directory: null,
    actions: [],
  };
  const comparison = emptyComparison({
    revision: 4,
    base_revision: 3,
    counts: { synced: 0, local: 2, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    entries: [
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: CODEX_SESSION_ID,
        resource_id: localSyncResourceId,
        display_name: codexSession.title,
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        cloud_thread_id: CODEX_SESSION_ID,
        provider_thread_id: CODEX_SESSION_ID,
        local_updated_at: codexSession.ended_at,
        storage_updated_at: codexSession.ended_at - 60,
      },
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: remoteId,
        resource_id: remoteSyncResourceId,
        display_name: "Improve cloud thread titles",
        state: "storage_only",
        local_present: false,
        storage_present: true,
        cloud_thread_id: remoteId,
        storage_updated_at: 1_752_802_500,
      },
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: olderId,
        resource_id: `codex:conversation:${olderId}`,
        display_name: olderId,
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        cloud_thread_id: olderId,
        provider_thread_id: olderId,
        local_updated_at: activity.window_start - 60,
        storage_updated_at: activity.window_start - 120,
      },
    ],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null, sessions: [codexSession, remoteSession] }}
      comparison={comparison}
      activeStorageName="Local storage 1"
    />,
  );

  // The header is gone entirely: the path bar's root crumb names the storage
  // being compared against, and the tallies ride Push and Pull.
  assert.doesNotMatch(html, /sync-history-storage-lens/);
  assert.doesNotMatch(html, /Visible thread comparison/);
  assert.doesNotMatch(html, /sync-thread-sync-summary/);
  assert.match(html, /aria-label="Newer on this computer\. Push to update Local storage 1\."/);
  assert.match(html, /aria-label="Only in Local storage 1\. Pull to download it here\."/);
  assert.match(html, /sync-thread-sync-indicator local[^>]+aria-label="Newer on this computer\.[^"]+"[^>]*><svg[^>]+>/);
  assert.match(html, /sync-thread-sync-indicator storage[^>]+aria-label="Only in Local storage 1\.[^"]+"[^>]*><svg[^>]+>/);
  assert.doesNotMatch(html, /sync-thread-sync-glyph/);
  assert.match(html, /Improve cloud thread titles/);
  // The remote-only row says where it lives through its badge, not a location
  // line: "Only in Local storage 1. Pull to download it here." is asserted
  // above, and the row keeps its provider mark inert because it has no open
  // action to hang it on.
  assert.doesNotMatch(html, /sync-history-stored-thread-location/);
  assert.match(html, /class="sync-history-launch-action sync-history-provider-inert"/);
  assert.ok(
    html.indexOf('aria-label="Newer on this computer. Push to update Local storage 1."')
      < html.indexOf("Add Git history mapping"),
  );
  assert.ok(
    html.indexOf('aria-label="Only in Local storage 1. Pull to download it here."')
      < html.indexOf("Improve cloud thread titles"),
  );

  // The title/location stack is gone with the location line, so the title is a
  // direct flex child of the row again and needs no wrapper.
  const css = readFileSync("src/App.css", "utf8");
  assert.doesNotMatch(css, /sync-history-stored-thread-copy/);
});

test("unavailable sessions explain why they cannot be synced", () => {
  const comparison = emptyComparison({
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 1, unknown: 0 },
    entries: [{
      provider: "codex",
      provider_display_name: "Codex",
      access: "bound_ready",
      thread_id: CODEX_SESSION_ID,
      resource_id: `codex:conversation:${CODEX_SESSION_ID}`,
      display_name: CODEX_SESSION_ID,
      state: "unavailable",
      local_present: true,
      storage_present: false,
      provider_thread_id: CODEX_SESSION_ID,
      status_detail: "Session file exceeds the 100 MiB per-file sync limit.",
      local_updated_at: codexSession.ended_at,
    }],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null }}
      comparison={comparison}
      activeStorageName="Local storage 1"
    />,
  );

  assert.match(html, /aria-label="Unavailable for sync\. Session file exceeds the 100 MiB per-file sync limit\."/);
  assert.doesNotMatch(html, /can.t tell which copy is newer/);
});

test("mixed-provider rows sort deterministically and never collide on native ids", () => {
  // Both providers use the very same provider-native session id: only the
  // resource id — provider-qualified — may key the rows.
  const nativeId = "0000aaaa-1111-2222-3333-444455556666";
  const codexSyncResourceId = `codex:conversation:${nativeId}`;
  const claudeSyncResourceId = `claude:session:${nativeId}`;
  const codexTwin = {
    ...codexSession,
    resource_id: `codex:conversation:${nativeId}`,
    provider_session_id: nativeId,
    logical_thread_id: nativeId,
    title: "Codex twin",
  };
  const claudeTwin = {
    ...codexSession,
    resource_id: `claude:conversation:${nativeId}`,
    provider: "claude",
    provider_display_name: "Claude",
    provider_session_id: nativeId,
    logical_thread_id: nativeId,
    title: "Claude twin",
    actions: [{
      action_id: "claude.open_in_terminal",
      kind: "open_in_terminal" as const,
      label: "Open in Terminal",
      enabled: true,
      disabled_reason: null,
    }],
  };
  const mixedActivity = {
    ...activity,
    git: null,
    // Deliberately unsorted: the page re-applies the deterministic order
    // (latest first, then provider key, then resource id).
    sessions: [codexTwin, claudeTwin],
    providers: [
      { provider: "codex", display_name: "Codex", access: "bound_ready" as const },
      { provider: "claude", display_name: "Claude", access: "bound_ready" as const },
    ],
  };
  const decision = {
    action: "create_cloud",
    reason_code: "missing_cloud",
    selectable: true,
  };
  const comparison = emptyComparison({
    entries: [
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: nativeId,
        resource_id: codexSyncResourceId,
        display_name: codexTwin.title,
        state: "local_only",
        local_present: true,
        storage_present: false,
        cloud_thread_id: nativeId,
        provider_thread_id: nativeId,
        push_decision: decision,
        pull_decision: { ...decision, action: "report_push_eligible", selectable: false },
      },
      {
        provider: "claude",
        provider_display_name: "Claude",
        access: "bound_ready",
        thread_id: nativeId,
        resource_id: claudeSyncResourceId,
        display_name: claudeTwin.title,
        state: "local_only",
        local_present: true,
        storage_present: false,
        cloud_thread_id: nativeId,
        provider_thread_id: nativeId,
        push_decision: decision,
        pull_decision: { ...decision, action: "report_push_eligible", selectable: false },
      },
    ],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={mixedActivity}
      comparison={comparison}
      activeStorageName="Local storage 1"
      selectionMode="push"
      selectableResourceIds={new Set([codexSyncResourceId, claudeSyncResourceId])}
      selectedResourceIds={new Set([claudeSyncResourceId])}
    />,
  );

  // Same ended_at, so the provider key breaks the tie: claude before codex.
  assert.ok(html.indexOf("Claude twin") < html.indexOf("Codex twin"), "provider key orders equal timestamps");
  // Each row wears its own provider's mark. Codex reuses the ChatGPT knot
  // already in the sheet; Claude Code has no canonical crisp-edge geometry yet
  // and ships a placeholder burst. The Claude twin offers no open action, so
  // its mark renders inert rather than vanishing — identity does not disable.
  assert.match(html, /aria-label="Open in Codex"><svg[^>]*data-icon="openai"/);
  assert.match(
    html,
    /class="sync-history-launch-action sync-history-provider-inert"[^>]*title="Claude session"[^>]*><svg[^>]*data-icon="claude"/,
  );
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
  // Selection is keyed by resource id, so ticking the Claude twin cannot
  // leak onto the Codex twin that shares its native id.
  assert.match(html, /<input[^>]*aria-label="Exclude Claude twin"[^>]*checked=""/);
  const codexInput = html.match(/<input[^>]*aria-label="Include Codex twin"[^>]*>/)?.[0] ?? "";
  assert.notEqual(codexInput, "");
  assert.doesNotMatch(codexInput, /checked=""/);
});

test("view-only rows keep their badge and copy but never a checkbox", () => {
  const claudeRemote = {
    ...codexSession,
    resource_id: "claude:conversation:claude-thread-1",
    provider: "claude",
    provider_display_name: "Claude",
    provider_session_id: null,
    logical_thread_id: "claude-thread-1",
    source: "remote_only" as const,
    access: "view_only" as const,
    title: "Claude remote session",
    working_directory: null,
    actions: [],
  };
  const unsupportedRemote = {
    ...claudeRemote,
    resource_id: "mystery:conversation:thread-9",
    provider: "mystery",
    provider_display_name: "mystery",
    logical_thread_id: "thread-9",
    access: "unsupported" as const,
    title: "Unknown provider session",
    ended_at: claudeRemote.ended_at - 10,
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{
        ...activity,
        git: null,
        sessions: [claudeRemote, unsupportedRemote],
        providers: [
          { provider: "codex", display_name: "Codex", access: "bound_ready" as const },
          { provider: "claude", display_name: "Claude", access: "view_only" as const, detail: "Sessions were published from another machine." },
        ],
      }}
      activeStorageName="Local storage 1"
      selectionMode="pull"
      // Even a listed resource id must not produce a checkbox without
      // bound-ready access: visibility is not permission.
      selectableResourceIds={new Set([claudeRemote.resource_id, unsupportedRemote.resource_id])}
    />,
  );

  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 0);
  assert.doesNotMatch(html, /sync-sync-selectable-row/);
  assert.match(html, /class="sync-history-branch-state history-only"[^>]*>View only</);
  assert.match(html, /View only — connect a Claude profile to sync this session\./);
  assert.match(html, /View only — this Mallard version has no adapter for mystery sessions\./);
  // The per-provider notice strip explains the access state once, with the
  // backend's detail carried along.
  assert.match(html, /sync-history-provider-notice/);
  assert.match(html, /<strong>Claude<\/strong> — View only — connect a Claude profile to sync these sessions\. Sessions were published from another machine\./);
  // Review rows and storage-only rows never navigate or download implicitly.
  assert.doesNotMatch(html, /sync-history-session-title-button|aria-label="View session details"/);
});

test("session actions render from backend descriptors with disabled reasons", () => {
  const heldSession = {
    ...codexSession,
    resource_id: "codex:conversation:held-thread",
    provider_session_id: "held-thread",
    logical_thread_id: "held-thread",
    title: "Held session",
    ended_at: codexSession.ended_at - 100,
    actions: codexActions(false),
  };
  const localSession = {
    ...codexSession,
    actions: codexActions(),
  };
  const browseHtml = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null, sessions: [localSession, heldSession] }}
    />,
  );

  // Enabled actions carry their adapter label. Open wears the provider's own
  // mark — it is the row's identity as well as its button; the other kinds are
  // the action, not the agent, so they keep a generic glyph.
  const openButton = browseHtml.match(/<button[^>]*aria-label="Open in Codex"[^>]*>[\s\S]*?<\/button>/g) ?? [];
  assert.equal(openButton.length, 2);
  assert.match(openButton[0] ?? "", /data-icon="openai"/);
  assert.doesNotMatch(browseHtml, /data-icon="play"/);
  assert.doesNotMatch(openButton[0] ?? "", /disabled=""/);
  assert.match(openButton[1] ?? "", /disabled=""/);
  // A disabled action explains itself in its title.
  assert.match(openButton[1] ?? "", /title="This session is not materialized locally\."/);
  const terminalButtons = browseHtml.match(/<button[^>]*aria-label="Open in Terminal"[^>]*>[\s\S]*?<\/button>/g) ?? [];
  assert.match(terminalButtons[0] ?? "", /data-icon="terminal"/);
  // Selection reviews retain safe launch actions so a held row can still be
  // inspected in its own app or terminal.
  const reviewHtml = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{ ...activity, git: null, sessions: [localSession, heldSession] }}
      selectionMode="push"
      selectableResourceIds={new Set()}
    />,
  );
  assert.equal((reviewHtml.match(/aria-label="Open in Codex"/g) ?? []).length, 2);
  assert.equal((reviewHtml.match(/aria-label="Open in Terminal"/g) ?? []).length, 2);
});

test("provider comparison failures render isolated warnings", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      embedded
      activity={{
        ...activity,
        git: null,
        providers: [
          { provider: "codex", display_name: "Codex", access: "bound_ready" as const },
          { provider: "claude", display_name: "Claude", access: "bound_ready" as const },
        ],
      }}
      comparison={emptyComparison({
        provider_errors: { claude: "The Claude profile is locked by another process." },
      })}
      activeStorageName="Local storage 1"
    />,
  );

  assert.match(html, /sync-history-provider-error/);
  assert.match(html, /role="alert"/);
  assert.match(html, /<strong>Claude<\/strong> comparison failed: The Claude profile is locked by another process\./);
  // The failure is isolated: the healthy provider's rows still render.
  assert.match(html, /Add Git history mapping/);
  assert.doesNotMatch(html, /<strong>Codex<\/strong> comparison failed/);
});

test("non-Git activity renders a flat session list", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      project={{ ...project, local_alias: null }}
      activity={{ ...activity, git: null }}
    />,
  );
  const headingIndex = html.indexOf('id="project-sessions-heading"');
  const headingEndIndex = html.indexOf("</h2>", headingIndex);
  const messageIconIndex = html.indexOf('data-icon="message"', headingIndex);

  assert.match(html, />Sessions</);
  assert.doesNotMatch(html, /Codex threads/);
  assert.ok(headingIndex >= 0);
  assert.ok(messageIconIndex > headingIndex);
  assert.ok(messageIconIndex < headingEndIndex);
  assert.doesNotMatch(html, /Branch/);
});

test("a project without a bound provider profile stays in the workspace", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      binding={null}
      activity={null}
      actionError="Provider profile path changed."
    />,
  );
  assert.match(html, /role="alert"/);
  assert.match(html, /No agent profile is connected\./);
  assert.doesNotMatch(html, /No Codex profile is connected/);
  assert.doesNotMatch(html, /Open Project Settings|Project settings/);
});

test("the page speaks only the provider-neutral activity contracts", () => {
  const source = readFileSync("src/components/project-sync/ProjectActivityPage.tsx", "utf8");

  // New command surface only.
  assert.match(source, /getProjectActivity\(/);
  assert.match(source, /executeProjectSessionAction\(/);
  assert.match(source, /getSessionSyncComparison\(/);
  assert.match(source, /onOpenSessionDetail/);
  assert.doesNotMatch(source, /getProjectSessionDetails|getProjectChatHistory|getProjectChatThreadDetails|openCodexThreadInApp|openCodexThreadInTerminal|getThreadSyncComparison/);

  // Provider launches are reached through generic action descriptors, never
  // through a provider-key comparison in common flow.
  assert.doesNotMatch(source, /fork_session|forkCodexThread|retryCodexDivergence/);
  assert.doesNotMatch(source, /===\s*"codex"|"codex"\s*===/);
  assert.doesNotMatch(source, /provider_ids\?\.codex|profile_ids\?\.codex/);

  // Rows key and select by the provider-qualified resource id.
  assert.match(source, /session\.resource_id/);
  assert.doesNotMatch(source, /codex:conversation:/);
});

test("the Git project icon uses the downloaded pixel-folder language", () => {
  const html = renderToStaticMarkup(<Icon name="git-folder" size={16} />);
  assert.match(html, /icon-git-folder-shell/);
  assert.match(html, /icon-git-folder-mark/);
  assert.match(html, /icon-pixelarticons/);
  assert.match(html, /icon-pixel-fill/);
  assert.match(html, /fill="currentColor"/);
  assert.match(html, /shape-rendering="crispEdges"/);
  assert.doesNotMatch(html, /<circle/);
});

test("sidebar navigation icons use the restrained Pixelarticons silhouettes", () => {
  for (const name of ["folder", "drive", "activity", "git-folder"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={16} />);
    assert.match(html, /icon-pixelarticons/, `${name} must use the selected Pixelarticons geometry`);
    assert.match(html, /viewBox="0 0 24 24"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }
});

test("history metrics use matching sourced pixel silhouettes", () => {
  for (const name of ["user", "token", "message", "tool", "info"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={12} />);
    assert.match(html, /icon-(?:pixelarticons|hackernoon-pixel)/, `${name} must use the downloaded pixel icon family`);
    assert.match(html, /fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }
});

test("storage overflow and sync actions use matching Pixelarticons silhouettes", () => {
  for (const name of ["more", "download", "upload"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={16} />);
    assert.match(html, /icon-pixelarticons/, `${name} must match the storage action icon family`);
    assert.match(html, /fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }
});

test("sidebar utility controls use matching Pixelarticons and a monospace count badge", () => {
  for (const name of ["plus", "trash"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={16} />);
    assert.match(html, /icon-pixelarticons/, `${name} must match the sidebar utility icon family`);
    assert.match(html, /fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }

  // The settings cog is drawn in-house rather than sourced, so it carries no
  // provenance class — it still has to share the utility glyphs' construction.
  const settings = renderToStaticMarkup(<Icon name="settings" size={16} />);
  assert.match(settings, /icon-pixel-fill/);
  assert.doesNotMatch(settings, /icon-pixelarticons/);
  assert.match(settings, /fill="currentColor"/);
  assert.match(settings, /shape-rendering="crispEdges"/);
  assert.doesNotMatch(settings, /<(?:circle|ellipse|line|polyline)\b/);

  const css = readFileSync("src/RetroTheme.css", "utf8");
  const badgeRule = css.match(
    /\.sidebar-nav-badge,\s*\n\.sync-primary-nav button > \.sync-activity-badge\s*\{[^}]+\}/s,
  )?.[0] ?? "";
  assert.match(badgeRule, /font-family:\s*var\(--font-mono\)/);
  assert.match(badgeRule, /font-variant-numeric:\s*tabular-nums/);
});

test("theme controls use the compact Pixelarticons sun and moon pair", () => {
  for (const name of ["sun", "moon"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={18} />);
    assert.match(html, /icon-pixelarticons/);
    assert.match(html, /fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }
});

test("history launch actions share one pixel construction", () => {
  for (const name of ["git-fork", "terminal", "play"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={14} />);
    assert.match(html, /icon-pixel-fill/, `${name} must match the history launch action family`);
    assert.match(html, /fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }
  // git-fork is still Pixelarticons geometry; terminal was redrawn in-house
  // because the sourced one dissolved at 12-14px, so it carries no provenance.
  assert.match(renderToStaticMarkup(<Icon name="git-fork" size={14} />), /icon-pixelarticons/);
  assert.doesNotMatch(renderToStaticMarkup(<Icon name="terminal" size={14} />), /icon-pixelarticons/);
});

test("the terminal glyph survives the sizes it is rendered at", () => {
  const html = renderToStaticMarkup(<Icon name="terminal" size={12} />);
  const d = html.match(/ d="([^"]+)"/)?.[1] ?? "";
  assert.ok(d.length > 0, "terminal must draw a path");
  // A 24-unit grid at 12px is half a device pixel per unit, so every edge has
  // to land on an even coordinate or the feature is lost to rounding.
  const coords = [...d.matchAll(/-?\d+/g)].map((m) => Number(m[0]));
  assert.ok(coords.every((n) => n % 2 === 0), `odd coordinate in terminal path: ${d}`);
});

test("the external-link glyph survives the size the commit chip renders it at", () => {
  const html = renderToStaticMarkup(<Icon name="external-link" size={12} />);
  const d = html.match(/ d="([^"]+)"/)?.[1] ?? "";
  assert.ok(d.length > 0, "external-link must draw a path");
  // Same rule as the terminal glyph: 24 units at 12px is half a device pixel
  // each, so an odd edge paints grey instead of black. Rasterized, the sourced
  // glyph came out a grey box with a smeared diagonal.
  const coords = [...d.matchAll(/-?\d+/g)].map((m) => Number(m[0]));
  assert.ok(coords.every((n) => n % 2 === 0), `odd coordinate in external-link path: ${d}`);
  // Redrawn in-house, so it must not claim the sourced provenance.
  assert.doesNotMatch(html, /icon-pixelarticons/);
  assert.match(html, /icon-pixel-fill/);
  assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
});

test("the shared icon catalog renders as monochrome pixel geometry", () => {
  const pixelIconNames = [
    "activity",
    "alert-triangle",
    "badge-check",
    "ban",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "chevron-up",
    "clock",
    "cloud",
    "cloud-off",
    "computer",
    "check-circle",
    "download",
    "drive",
    "edit",
    "file",
    "folder",
    "external-link",
    "git-branch",
    "git-commit",
    "git-compare-arrows",
    "git-fork",
    "git-folder",
    "help-circle",
    "info",
    "link",
    "lock",
    "message",
    "moon",
    "more",
    "octagon-x",
    "openai",
    "pause",
    "play",
    "plus",
    "refresh",
    "settings",
    "sidebar-collapse",
    "sidebar-expand",
    "sun",
    "token",
    "tool",
    "trash",
    "terminal",
    "upload",
    "user",
    "x",
  ] as const;

  for (const name of pixelIconNames) {
    const html = renderToStaticMarkup(<Icon name={name} size={16} />);
    assert.match(html, /class="[^"]*icon-pixel-fill[^"]*"/, `${name} must use pixel fill geometry`);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
    assert.doesNotMatch(html, /\brx=|\bry=|stroke-width=/);
  }
});

test("standalone Git branch icons stay monochrome in the retro skin", () => {
  const html = renderToStaticMarkup(<Icon name="git-branch" size={16} />);
  const css = readFileSync("src/RetroTheme.css", "utf8");

  assert.match(html, /icon-git-branch-mark/);
  assert.match(html, /class="icon-git-branch-mark icon-pixel-fill"/);
  assert.match(html, /fill="currentColor" stroke="none"/);
  assert.match(
    css,
    /\.sync-app \.icon-git-branch-mark,\s*\.sync-app \.icon-git-folder-mark\s*\{\s*color:\s*var\(--text-1\);\s*\}/,
  );
});

test("completed projects use their main row for history and only mark Git repositories", () => {
  const html = renderToStaticMarkup(
    <ProjectSidebar
      projects={[
        { ...project, is_git_repository: true },
        {
          ...project,
          local_project_id: "project-folder",
          local_alias: "Plain folder",
          profile_names: ["myconf3 · Codex"],
          is_git_repository: false,
        },
      ]}
      drafts={[{
        draft_id: "draft-1",
        display_name: "draft repo",
        project_root: "/tmp/draft",
        updated_at: 1,
        revision: 1,
        status: "draft",
      }]}
      activeDraftId={null}
      storages={[]}
      storageUsage={{}}
      activeProjectId={project.local_project_id}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      loading={false}
      busy={false}
      activityOpen={false}
      unreadLogs={0}
      onSelectProject={() => undefined}
      onRemoveProject={() => undefined}
      onSelectDraft={() => undefined}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => undefined}
      onRefresh={() => undefined}
      onOpenStorage={() => undefined}
      onClearStorage={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogMenu={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
    />,
  );
  // The brand row and the product footer are the same mark now, so it appears
  // twice and no raster logo appears at all.
  assert.equal(html.match(/src="\/mallard-logo-duck\.svg"/g)?.length, 2);
  assert.doesNotMatch(html, /\.png"/);
  assert.doesNotMatch(html, /aria-label="View history for Mallard local"/);
  assert.equal(html.match(/sync-project-git-icon/g)?.length, 1);
  assert.equal(html.match(/icon-git-folder-mark/g)?.length, 1);
  assert.equal(html.match(/sync-project-folder-icon/g)?.length, 1);
  assert.match(html, /aria-label="Mallard local, Git repository"/);
  assert.doesNotMatch(html, /sync-repository-kind|>git<\/span>/);
  assert.doesNotMatch(html, /Default Codex|myconf3 · Codex/);
  const primaryNav = html.slice(
    html.indexOf('<nav class="sync-primary-nav"'),
    html.indexOf("</nav>") + "</nav>".length,
  );
  const projectsNavIndex = primaryNav.indexOf(" Projects</button>");
  const storagesNavIndex = primaryNav.indexOf(" Storages</button>");
  const catalogNavIndex = primaryNav.indexOf(" Catalog</button>");
  const logsNavIndex = primaryNav.indexOf(" Logs</button>");
  assert.ok(projectsNavIndex >= 0);
  // Catalog is a destination like Projects and Storages, so it joins them in
  // the nav; Logs stays last because it toggles a drawer instead of navigating.
  assert.ok(catalogNavIndex > projectsNavIndex);
  assert.ok(storagesNavIndex > catalogNavIndex);
  assert.ok(logsNavIndex > storagesNavIndex);
  assert.match(html, /aria-label="Storages"/);
  assert.match(html, />Storages<\/span>/);
  // Every seam down the rail is a handle — each one is the same 3px of frame
  // colour, so any that did nothing read as a handle that was simply broken.
  assert.match(html, /role="separator" aria-label="Resize navigation and Projects sections" aria-orientation="horizontal"/);
  assert.match(html, /aria-valuetext="Navigation height: fits contents"/);
  assert.match(html, /role="separator" aria-label="Resize Projects and Storages sections" aria-orientation="horizontal"/);
  assert.match(html, /role="separator" aria-label="Resize Storages and Catalog sections" aria-orientation="horizontal"/);
  assert.match(html, /aria-valuenow="56"/);
  // Catalog is content-sized (capped at 40% of the box), so it reports fitting
  // its contents until the handle pins it to a height rather than carrying a
  // share of the box.
  assert.match(html, /aria-valuetext="Catalog height: fits contents"/);
  assert.doesNotMatch(html, /sidebar-section-count/);
  assert.doesNotMatch(html, /Git Based|Non-Git Based/);
  assert.doesNotMatch(html, /Project settings for Mallard local/);
  assert.doesNotMatch(html, /View history for draft repo/);
});

test("the project menu accepts multiple bound provider profiles", () => {
  const html = renderToStaticMarkup(
    <ProjectSidebar
      projects={[
        {
          ...project,
          local_project_id: "project-multiple-profiles",
          local_alias: "Multiple profiles",
          profile_ids: {
            codex: "profile-codex",
            claude: "profile-claude",
          },
        },
        {
          ...project,
          local_project_id: "project-no-profile",
          local_alias: "No profile",
          profile_ids: {},
        },
      ]}
      drafts={[]}
      activeDraftId={null}
      storages={[]}
      storageUsage={{}}
      activeProjectId="project-multiple-profiles"
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      loading={false}
      busy={false}
      activityOpen={false}
      unreadLogs={0}
      onSelectProject={() => undefined}
      onRemoveProject={() => undefined}
      onSelectDraft={() => undefined}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => undefined}
      onOpenStorage={() => undefined}
      onClearStorage={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogMenu={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
    />,
  );

  const multipleProfilesRow = html.slice(
    html.lastIndexOf('<div class="sidebar-profile-item', html.indexOf("Multiple profiles")),
    html.indexOf("</div>", html.indexOf("Multiple profiles")),
  );
  const noProfileRow = html.slice(
    html.lastIndexOf('<div class="sidebar-profile-item', html.indexOf("No profile")),
    html.indexOf("</div>", html.indexOf("No profile")),
  );

  assert.doesNotMatch(multipleProfilesRow, /needs-profile|sync-sidebar-profile-warning/);
  assert.match(noProfileRow, /needs-profile/);
  assert.match(noProfileRow, /sync-sidebar-profile-warning/);
  assert.equal((html.match(/sync-sidebar-profile-warning/g) ?? []).length, 1);
});

test("the Catalog pane starts empty and lists one row per added scope", () => {
  const scopeView = (
    scopeId: string,
    resourceKind: "agent_skill" | "plugin_install_intent" | "codex_pet_v1" | "codex_pet_v2",
    displayName: string,
  ) => ({
    scope: {
      scope_id: scopeId,
      provider: "codex" as const,
      profile_id: `catprofile-${displayName}`,
      resource_kind: resourceKind,
      revision: 1,
      created_at: 1,
      updated_at: 1,
    },
    profile: {
      profile_id: `catprofile-${displayName}`,
      provider_key: "codex" as const,
      display_name: displayName,
      configured_home: `/Users/user/${displayName}`,
      revision: 1,
      created_at: 1,
      updated_at: 1,
    },
  });
  const render = (scopes: ReturnType<typeof scopeView>[], activeScopeId: string | null) =>
    renderToStaticMarkup(
      <ProjectSidebar
        projects={[project]}
        drafts={[]}
        activeDraftId={null}
        storages={[]}
        storageUsage={{}}
        activeProjectId={null}
        activeStorageId={null}
        catalogScopes={scopes}
        activeCatalogScopeId={activeScopeId}
        addingCatalogScope={false}
        loading={false}
        busy={false}
        activityOpen={false}
        unreadLogs={0}
        onSelectProject={() => undefined}
        onRemoveProject={() => undefined}
        onSelectDraft={() => undefined}
        onDiscardDraft={() => undefined}
        onToggleActivity={() => undefined}
        onAddProject={() => undefined}
        onOpenStorage={() => undefined}
        onClearStorage={() => undefined}
        onRemoveStorage={() => undefined}
        onAddStorage={() => undefined}
        onOpenCatalogMenu={() => undefined}
        onOpenCatalogScope={() => undefined}
        onAddCatalogScope={() => undefined}
        onRemoveCatalogScope={() => undefined}
      />,
    );

  // Fresh state: no permanent Skills/Plugins/Pets rows, just the Add prompt.
  const emptyHtml = render([], null);
  const emptyPane = emptyHtml.slice(emptyHtml.indexOf('aria-label="Catalog"'));
  assert.match(emptyPane, /aria-label="Add Catalog row"/);
  assert.match(emptyPane, /Add your first row/);
  assert.doesNotMatch(emptyPane, /<span>Skills<\/span>|<span>Plugins<\/span>|<span>Pets<\/span>/);

  // Two Codex configs give the same kind two distinct rows, labelled by
  // config name; pet v1 and v2 from one config are two separate rows.
  const html = render([
    scopeView("catscope-1", "agent_skill", "Codex home"),
    scopeView("catscope-2", "agent_skill", "conf2"),
    scopeView("catscope-3", "codex_pet_v1", "Codex home"),
    scopeView("catscope-4", "codex_pet_v2", "Codex home"),
  ], "catscope-2");
  const pane = html.slice(html.indexOf('aria-label="Catalog"'));
  assert.match(pane, /<span>Skills · Codex home<\/span>/);
  assert.match(pane, /<span>Skills · conf2<\/span>/);
  assert.match(pane, /<span>Pets v1 · Codex home<\/span>/);
  assert.match(pane, /<span>Pets v2 · Codex home<\/span>/);
  assert.match(pane, /sidebar-profile-item active[^]*?Skills · conf2/);
  assert.match(pane, /aria-label="Remove the Skills · conf2 row"/);
  assert.doesNotMatch(pane, /Add your first row/);
});

test("the sidebar locks project and storage navigation during a sync workflow", () => {
  const html = renderToStaticMarkup(
    <ProjectSidebar
      projects={[project]}
      drafts={[]}
      activeDraftId={null}
      storages={[{
        id: "storage-1",
        name: "Local storage 1",
        kind: "local",
        bucket: "",
        access_key_id: "",
        secret_access_key: "",
        account_id: "",
        s3_endpoint: "",
        region: "",
        local_dir: "/tmp/storage",
        included_default_exclusions: [],
      }]}
      storageUsage={{ "storage-1": 1 }}
      storageIssues={{ "storage-1": "Storage folder unavailable." }}
      activeProjectId={project.local_project_id}
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      loading={false}
      busy
      activityOpen={false}
      unreadLogs={0}
      onSelectProject={() => undefined}
      onRemoveProject={() => undefined}
      onSelectDraft={() => undefined}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => undefined}
      onRefresh={() => undefined}
      onOpenStorage={() => undefined}
      onClearStorage={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogMenu={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
    />,
  );

  const projectMarker = html.indexOf("<span>Mallard local</span>");
  const projectButton = html.slice(html.lastIndexOf("<button", projectMarker), html.indexOf(">", html.lastIndexOf("<button", projectMarker)) + 1);
  const storageMarker = html.indexOf("<span>Local storage 1</span>");
  const storageButton = html.slice(html.lastIndexOf("<button", storageMarker), html.indexOf(">", html.lastIndexOf("<button", storageMarker)) + 1);
  const addStorageMarker = html.indexOf('aria-label="Add storage"');
  const addStorageButton = html.slice(html.lastIndexOf("<button", addStorageMarker), html.indexOf(">", addStorageMarker) + 1);

  assert.match(projectButton, /disabled=""/);
  assert.match(storageButton, /disabled=""/);
  assert.match(addStorageButton, /disabled=""/);
  assert.match(html, /sync-sidebar-storage-warning/);
  assert.match(storageButton, /Storage issue: Storage folder unavailable\./);
  assert.doesNotMatch(html, /Legacy profiles|Open legacy/);
});

void ProjectActivityPage;
