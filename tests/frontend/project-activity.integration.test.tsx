import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  activityHeaderCounts,
  BranchPicker,
  ProjectChatHistoryContent,
  pullOperationChip,
  SessionMetrics,
  sessionDebugCopyText,
  sessionSyncPresentation,
  unresolvedSyncPresentation,
} from "../../src/components/project-sync/ProjectActivityPage";
import ProjectSidebar from "../../src/components/project-sync/ProjectSidebar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../../src/components/sync-review/ThreadStatusIcon";
import { createSingleFlight } from "../../src/components/project-sync/api";
import type {
  ConversationReviewAction,
  LocalProjectSummary,
  ProjectBinding,
  ThreadSyncComparison,
  ThreadSyncDecision,
  ThreadSyncEntry,
} from "../../src/types";

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

const CODEX_SESSION_ID = "019f7798-5437-7632-9dbc-5b589cf68bf0";
const codexResourceId = `codex:conversation:${CODEX_SESSION_ID}`;
const CODEX_TRANSCRIPT_PATH = `/Users/test/.codex/sessions/2025/07/18/rollout-${CODEX_SESSION_ID}.jsonl`;
const CLAUDE_TRANSCRIPT_PATH = `/Users/test/.claude/projects/-Users-test-projects-mallard/${CODEX_SESSION_ID}.jsonl`;

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
  preserved: null,
  summary: "Map project-owned sessions onto the first-parent rail.",
  started_at: 1_752_800_000,
  ended_at: 1_752_803_600,
  transcript_path: CODEX_TRANSCRIPT_PATH,
  working_directory: "/Users/test/projects/mallard/packages/desktop",
  branch: "main",
  recorded_sha: "a".repeat(40),
  is_active: false,
  user_round_count: 3,
  agent_message_count: 5,
  // Keep this distinct from 3 prompts + 5 replies so the mock contract
  // catches tools being folded into the visible message total.
  tool_call_count: 13,
  total_input_tokens: 20000,
  total_output_tokens: 4800,
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
  },
  unmapped: [],
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" as const }],
  warnings: [],
  window_start: 1_750_000_000,
  window_end: 1_753_000_000,
  older: { newest_at: 1_749_000_000, commits: 41, sessions: 6 },
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
  loadError: null,
  onRetryLoad: () => undefined,
  actionBusyResourceId: null,
  onLoadMore: () => undefined,
  onShowLess: () => undefined,
  canShowLess: false,
  onSessionAction: () => undefined,
} satisfies Omit<ComponentProps<typeof ProjectChatHistoryContent>, "activity">;

function reviewDecision(
  action: ConversationReviewAction,
  selectable = false,
): ThreadSyncDecision {
  return { action, selectable };
}

function emptyComparison(overrides: Partial<ThreadSyncComparison> = {}): ThreadSyncComparison {
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
  const decision = (action: string, blocker: string | null = null) => ({
    action,
    blocker,
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
    conversation_resource_id: `codex:conversation:${state}-${JSON.stringify(overrides)}`,
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
    entries: [entry("local_ahead", { push_decision: { action: "blocked", selectable: false } })],
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
  const bar = readFileSync("src/components/sync-review/SyncPathBar.tsx", "utf8");
  const sync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  // The branch scopes the section, so it rides that crumb rather than standing
  // as a fourth one — and a session replaces the crumb, taking the chip with it.
  assert.match(workspace, /id: "section",[\s\S]*?scope: activityHeader && activityHeader\.branches\.length > 0 \? \(\s*<BranchPicker/);
  assert.match(bar, /\{crumb\.scope\}/);
  // A non-Git project's section crumb names the view on screen and offers the
  // other one in its own menu — the crumb grammar every other step of this bar
  // uses. The segmented pill this replaced sat in the leaf's position showing
  // both faces at once while naming neither, in the form vocabulary the storage
  // cards use rather than the bar's.
  assert.match(workspace, /else if \(workspaceProject\.is_git_repository\) \{/);
  assert.match(workspace, /label: workspaceSectionLabel,\s*icon: viewIcon\(activityView\),\s*tone: "section",/);
  assert.match(workspace, /menuLabel: "Switch view",/);
  assert.match(workspace, /menuHeader: \{\s*label: workspaceSectionLabel,/);
  assert.match(workspace, /label: "Switch to",[\s\S]*?onSelect: \(\) => setActivityView\(otherView\),/);
  assert.doesNotMatch(workspace, /sync-path-view-switch|activityViewSwitch/);
  assert.match(bar, /index > 0 && <span className="sync-path-separator"[\s\S]*?>\/<\/span>/);
  // The leaf is the page whether or not it opens a menu, so every branch that
  // can render a leaf marks it.
  assert.equal((bar.match(/aria-current=\{leaf \? "page" : undefined\}/g) ?? []).length, 3);
  // The caller-owned `content` slot existed only to park that pill in the
  // trail; with the crumb saying its own name, nothing renders through it.
  assert.doesNotMatch(bar, /crumb\.content|sync-path-crumb-content/);

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
      blocker: "busy",
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
      blocker: "codec_unsupported",
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
      blocker: "codec_unsupported",
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
});

test("thread status badges use the outlined Default 24px pixel family", () => {
  const badge = "M9 1h6v1h3v1h1v1h1v1h1v1h1v3h1v6h-1v3h-1v1h-1v1h-1v1h-1v1h-3v1h-6v-1h-3v-1h-1v-1h-1v-1h-1v-1h-1v-3h-1v-6h1v-3h1v-1h1v-1h1v-1h1v-1h3ZM11 2v1h-3v1h-2v1h-1v1h-1v2h-1v3h-1v2h1v3h1v2h1v1h1v1h2v1h3v1h2v-1h3v-1h2v-1h1v-1h1v-2h1v-3h1v-2h-1v-3h-1v-2h-1v-1h-1v-1h-2v-1h-3v-1Z";
  // The disc the filled variant paints: the ring's outer boundary alone, which
  // is everything before its second subpath.
  const disc = badge.slice(0, badge.indexOf("ZM") + 1);
  // Colours resolve per theme now. One hex could not serve both papers, and
  // three of the old seven failed outright on cream — amber `pending` measured
  // 2.22:1, on a 1px hairline.
  const expected: Record<
    ThreadStatusIconName,
    { light: string; dark: string; glyph: string; filled?: true }
  > = {
    // Cream forces the green to full saturation instead of lightness; dark
    // keeps the hue and lifts it a step so it clears the active row.
    synced: {
      light: "#006b32",
      dark: "#4dbf7e",
      glyph: "M16 9h2v2h-1v1h-1v2h-1v1h-1v1h-1v1h-2v-1h-1v-1h-1v-1h-1v-2h2v1h1v1h2v-1h1v-2h1v-1h1Z",
    },
    "cloud-outstanding": {
      light: "#256c90",
      dark: "#8ccff3",
      glyph: "M9 7h5v1h-5ZM9 8v2h-1v-2ZM14 8h1v2h-1ZM8 10v1h-2v-1ZM15 10h2v1h-2ZM6 11v3h-1v-3ZM17 11h1v5h-12v-2h1v1h10Z",
    },
    // VS Code's shipped gitDecoration.modified (light unaltered, dark lifted a
    // step for the active row): amber is what a file list means by "local work
    // not sent anywhere yet".
    "local-outstanding": {
      light: "#895503",
      dark: "#e6c896",
      glyph: "M11 8h3v1h1v1h1v1h1v2h-2v-1h-1v-1h-1v7h-2v-6h-1v1h-2v-2h1v-1h1v-1h1Z",
    },
    divergent: {
      light: "#6d529d",
      dark: "#cbb4f3",
      glyph: "M6 7h3v1h-3ZM7 8h3v1h-3ZM8 9h3v1h-3ZM9 10h3v1h-3ZM15 7h3v1h-3ZM14 8h3v1h-3ZM13 9h3v1h-3ZM12 10h3v1h-3ZM10 11h4v1h-4ZM10 12h4v6h-4Z",
      filled: true,
    },
    pending: {
      light: "#6f6159",
      dark: "#a79e90",
      glyph: "M11 7h2v5h4v2h-6Z",
    },
    conflict: {
      light: "#b53420",
      dark: "#f4917f",
      glyph: "M11 6h3v9h-3ZM11 16h3v3h-3Z",
      filled: true,
    },
    unavailable: {
      light: "#6f6159",
      dark: "#a79e90",
      glyph: "M7 11h10v3h-10Z",
    },
    // The three neutrals say "no verdict yet", so they take --text-3 rather
    // than a signal hue: a colour here would claim an answer the comparison has
    // not given, and on an unlinked project one of these is on every row.
    untracked: {
      light: "var(--text-3)",
      dark: "var(--text-3)",
      glyph: "M8 8h8v8H8ZM10 14h4v-4h-4Z",
    },
    checking: {
      light: "var(--text-3)",
      dark: "var(--text-3)",
      glyph: "M6 11h3v3H6Z",
    },
    unknown: {
      light: "var(--text-3)",
      dark: "var(--text-3)",
      glyph: "M9 6h6v2H9ZM13 8h2v3h-2ZM11 11h4v2h-4ZM11 13h2v2h-2ZM11 16h2v2h-2Z",
    },
  };
  const statuses = Object.keys(expected) as ThreadStatusIconName[];

  for (const status of statuses) {
    const html = renderToStaticMarkup(<ThreadStatusIcon status={status} />);
    assert.match(html, new RegExp(`data-thread-status="${status}"`));
    assert.match(html, /thread-status-icon-badge/);
    assert.ok(html.includes(expected[status].glyph), `${status} glyph geometry`);
    assert.match(html, /width="24"/);
    assert.match(html, /height="24"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    // No gradient, filter or stroke anywhere: the mark is flat fill on the
    // pixel grid, and a stroke would land on half-pixels at 18px.
    assert.doesNotMatch(html, /(?:linearGradient|filter|drop-shadow|stroke=)/);
    assert.doesNotMatch(html, /--thread-status-glyph/);

    if (expected[status].filled) {
      // Solid disc with the glyph punched out by fill-rule, so the two states a
      // button cannot resolve are found pre-attentively — and found in
      // greyscale, where hue tells the reader nothing (WCAG 1.4.1). No mask,
      // so no generated id has to survive server rendering.
      assert.ok(html.includes(disc), `${status} must paint the solid disc`);
      assert.match(html, /fill-rule="evenodd"/);
      assert.doesNotMatch(html, /<mask|url\(#/);
      // The knockout inverts stroke and gap, so overlapping subpaths would be
      // crossed an even number of times and fill back in.
      const rects = [...expected[status].glyph.matchAll(/M(\d+) (\d+)h(\d+)v(\d+)h-\d+Z/g)]
        .map(([, x, y, w, h]) => [+x, +y, +w, +h] as const);
      assert.equal(rects.length, expected[status].glyph.split("Z").length - 1,
        `${status} glyph must be whole rectangles, or overlap cannot be checked`);
      for (let i = 0; i < rects.length; i += 1) {
        for (let j = i + 1; j < rects.length; j += 1) {
          const [ax, ay, aw, ah] = rects[i];
          const [bx, by, bw, bh] = rects[j];
          const overlaps = ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah;
          assert.ok(!overlaps, `${status} rects ${i} and ${j} overlap; evenodd would refill the gap`);
        }
      }
    } else {
      assert.ok(html.includes(badge), `${status} must use the shared ring badge`);
      assert.match(html, /thread-status-icon-badge" fill="currentColor"/);
      assert.doesNotMatch(html, /fill-rule/);
    }
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
  // Both halves of every pair must stay in the shared palette; one value
  // cannot serve both papers.
  const palette = css;
  for (const { light, dark } of Object.values(expected)) {
    assert.ok(palette.includes(light), `light value ${light} missing from the palette`);
    assert.ok(palette.includes(dark), `dark value ${dark} missing from the palette`);
  }
  // None of the seven may be declared outside a theme block again.
  const invariant = palette.slice(0, palette.indexOf('data-theme="dark"'));
  assert.doesNotMatch(invariant, /--thread-status-(synced|local):/);
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
  const decision = (action: ConversationReviewAction, selectable: boolean) => ({
    action,
    selectable,
  });
  const comparison = emptyComparison({
    counts: { synced: 0, local: 1, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    entries: [{
      provider: "codex",
      provider_display_name: "Codex",
      access: "bound_ready",
      thread_id: CODEX_SESSION_ID,
      conversation_resource_id: syncResourceId,
      resource_id: syncResourceId,
      display_name: codexSession.title,
      state: "local_only",
      local_present: true,
      storage_present: false,
      cloud_thread_id: CODEX_SESSION_ID,
      compared_against_storage: true,
      provider_thread_id: CODEX_SESSION_ID,
      push_decision: decision("create_cloud", true),
      pull_decision: decision("report_local_only", false),
    }],
  });
  const browseHtml = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
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

test("pull selection rows wear the operation chips in the files grammar", () => {
  const chipEntry = (overrides: Record<string, unknown>) => ({
    state: "storage_ahead",
    compared_against_storage: true,
    pull_decision: reviewDecision("materialize_cloud", true),
    ...overrides,
  }) as Parameters<typeof pullOperationChip>[0];
  assert.equal(pullOperationChip(undefined), null);
  assert.equal(pullOperationChip(chipEntry({})), "Update session");
  assert.equal(
    pullOperationChip(chipEntry({ pull_decision: reviewDecision("keep_both", true) })),
    "Update session · keeps local copy",
  );
  assert.equal(
    pullOperationChip(chipEntry({ pull_decision: reviewDecision("create_local", true) })),
    "Add session",
  );
  assert.equal(
    pullOperationChip(chipEntry({ pull_decision: reviewDecision("adopt_cloud_metadata", true) })),
    "Mark synced",
  );
  assert.equal(
    pullOperationChip(chipEntry({ pull_decision: reviewDecision("report_local_only") })),
    null,
    "rows the pull would not touch carry no chip",
  );
  assert.equal(
    pullOperationChip(chipEntry({ compared_against_storage: false, state: "storage_only" })),
    "Add session",
  );
  assert.equal(
    pullOperationChip(chipEntry({ compared_against_storage: false, state: "diverged" })),
    "Update session · keeps local copy",
  );

  const comparison = emptyComparison({
    counts: { synced: 0, local: 0, storage: 0, diverged: 1, unavailable: 0, unknown: 0 },
    entries: [{
      provider: "codex",
      provider_display_name: "Codex",
      access: "bound_ready",
      thread_id: CODEX_SESSION_ID,
      conversation_resource_id: codexResourceId,
      resource_id: codexResourceId,
      display_name: codexSession.title,
      state: "diverged",
      local_present: true,
      storage_present: true,
      cloud_thread_id: CODEX_SESSION_ID,
      compared_against_storage: true,
      provider_thread_id: CODEX_SESSION_ID,
      push_decision: reviewDecision("require_pull"),
      pull_decision: reviewDecision("keep_both", true),
    }],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null }}
      comparison={comparison}
      activeStorageName="Local storage 1"
      selectionMode="pull"
      selectedResourceIds={new Set([codexResourceId])}
      selectableResourceIds={new Set([codexResourceId])}
      onToggleResource={() => undefined}
    />,
  );
  assert.match(html, />Update session · keeps local copy</);
});

test("Pull review shows only cloud-outstanding and divergent sessions", () => {
  const scenarios: Array<{
    key: string;
    title: string;
    state: ThreadSyncEntry["state"];
    source: "local_only" | "remote_only";
    pullAction: ConversationReviewAction;
    visible: boolean;
  }> = [
    { key: "stored", title: "Stored session", state: "storage_only", source: "remote_only", pullAction: "create_local", visible: true },
    { key: "newer-stored", title: "Newer stored session", state: "storage_ahead", source: "local_only", pullAction: "materialize_cloud", visible: true },
    { key: "diverged", title: "Diverged session", state: "diverged", source: "local_only", pullAction: "keep_both", visible: true },
    { key: "local", title: "Local session", state: "local_ahead", source: "local_only", pullAction: "report_push_eligible", visible: false },
    { key: "synced", title: "Synced session", state: "synced", source: "local_only", pullAction: "no_changes", visible: false },
    { key: "blocked", title: "Unavailable session", state: "unavailable", source: "local_only", pullAction: "report_local_only", visible: false },
  ];
  const sessionRows = scenarios.map((scenario, index) => {
    const nativeId = `019f7798-5437-7632-9dbc-${String(index + 1).padStart(12, "0")}`;
    return {
      ...codexSession,
      resource_id: `codex:conversation:${nativeId}`,
      provider_session_id: scenario.source === "remote_only" ? null : nativeId,
      logical_thread_id: nativeId,
      source: scenario.source,
      title: scenario.title,
      started_at: codexSession.started_at + index,
      ended_at: codexSession.ended_at + index,
      transcript_path: scenario.source === "remote_only" ? null : CODEX_TRANSCRIPT_PATH,
      working_directory: scenario.source === "remote_only" ? null : codexSession.working_directory,
      actions: scenario.source === "remote_only" ? [] : codexActions(),
    };
  });
  const comparison = emptyComparison({
    entries: scenarios.map((scenario, index): ThreadSyncEntry => {
      const session = sessionRows[index];
      return {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: session.logical_thread_id,
        conversation_resource_id: session.resource_id,
        resource_id: session.resource_id,
        display_name: scenario.title,
        state: scenario.state,
        local_present: scenario.source !== "remote_only",
        storage_present: true,
        cloud_thread_id: session.logical_thread_id,
        compared_against_storage: true,
        provider_thread_id: session.provider_session_id,
        push_decision: reviewDecision("no_changes"),
        pull_decision: reviewDecision(scenario.pullAction, scenario.visible),
      };
    }),
  });
  const commits = scenarios.flatMap((scenario, index) => {
    if (scenario.source === "remote_only") return [];
    return [{
      sha: String(index + 1).repeat(40),
      short_sha: String(index + 1).repeat(7),
      committed_at: activity.window_end - index,
      subject: `Commit ${scenario.title}`,
      session_refs: [{ resource_id: sessionRows[index].resource_id }],
    }];
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{
        ...activity,
        sessions: sessionRows,
        git: { ...activity.git, commits, unique_session_count: sessionRows.length },
        unmapped: [{ resource_id: sessionRows[0].resource_id, reason: "No local commit" }],
        older: null,
      }}
      comparison={comparison}
      activeStorageName="Local storage 1"
      selectionMode="pull"
      selectableResourceIds={new Set(sessionRows.map((session) => session.resource_id))}
    />,
  );

  assert.equal((html.match(/data-thread-status="cloud-outstanding"/g) ?? []).length, 2);
  assert.equal((html.match(/data-thread-status="divergent"/g) ?? []).length, 1);
  assert.doesNotMatch(html, /data-thread-status="(?:synced|local-outstanding|pending|conflict|unavailable|unknown)"/);
  for (const scenario of scenarios) {
    if (scenario.visible) assert.match(html, new RegExp(scenario.title));
    else {
      assert.doesNotMatch(html, new RegExp(scenario.title));
      assert.doesNotMatch(html, new RegExp(`Commit ${scenario.title}`));
    }
  }
});

test("Push review shows only ready local-outstanding sessions", () => {
  const scenarios: Array<{
    title: string;
    state: ThreadSyncEntry["state"];
    source: "local_only" | "remote_only";
    pushAction: ConversationReviewAction;
    selectable: boolean;
    visible: boolean;
  }> = [
    { title: "New local session", state: "local_only", source: "local_only", pushAction: "create_cloud", selectable: true, visible: true },
    { title: "Newer local session", state: "local_ahead", source: "local_only", pushAction: "publish", selectable: true, visible: true },
    { title: "Held local session", state: "local_ahead", source: "local_only", pushAction: "require_pull", selectable: false, visible: false },
    { title: "Stored session", state: "storage_only", source: "remote_only", pushAction: "not_pushable", selectable: false, visible: false },
    { title: "Diverged session", state: "diverged", source: "local_only", pushAction: "require_pull", selectable: false, visible: false },
    { title: "Synced session", state: "synced", source: "local_only", pushAction: "no_changes", selectable: false, visible: false },
    { title: "Blocked local session", state: "local_only", source: "local_only", pushAction: "blocked", selectable: false, visible: false },
  ];
  const sessionRows = scenarios.map((scenario, index) => {
    const nativeId = `019f7798-5437-7632-9dbc-${String(index + 20).padStart(12, "0")}`;
    return {
      ...codexSession,
      resource_id: `codex:conversation:${nativeId}`,
      provider_session_id: scenario.source === "remote_only" ? null : nativeId,
      logical_thread_id: nativeId,
      source: scenario.source,
      title: scenario.title,
      started_at: codexSession.started_at + index,
      ended_at: codexSession.ended_at + index,
      transcript_path: scenario.source === "remote_only" ? null : CODEX_TRANSCRIPT_PATH,
      working_directory: scenario.source === "remote_only" ? null : codexSession.working_directory,
      actions: scenario.source === "remote_only" ? [] : codexActions(),
    };
  });
  const comparison = emptyComparison({
    entries: scenarios.map((scenario, index): ThreadSyncEntry => {
      const session = sessionRows[index];
      return {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: session.logical_thread_id,
        conversation_resource_id: session.resource_id,
        resource_id: session.resource_id,
        display_name: scenario.title,
        state: scenario.state,
        local_present: scenario.source !== "remote_only",
        storage_present: scenario.state !== "local_only",
        cloud_thread_id: scenario.source === "remote_only" ? session.logical_thread_id : null,
        compared_against_storage: true,
        provider_thread_id: session.provider_session_id,
        push_decision: reviewDecision(scenario.pushAction, scenario.selectable),
        pull_decision: reviewDecision("no_changes"),
      };
    }),
  });
  const commits = scenarios.flatMap((scenario, index) => {
    if (scenario.source === "remote_only" || index === 0) return [];
    return [{
      sha: String(index + 1).repeat(40),
      short_sha: String(index + 1).repeat(7),
      committed_at: activity.window_end - index,
      subject: `Commit ${scenario.title}`,
      session_refs: [{ resource_id: sessionRows[index].resource_id }],
    }];
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{
        ...activity,
        sessions: sessionRows,
        git: { ...activity.git, commits, unique_session_count: sessionRows.length },
        unmapped: [{ resource_id: sessionRows[0].resource_id, reason: "No local commit" }],
        older: null,
      }}
      comparison={comparison}
      activeStorageName="Local storage 1"
      selectionMode="push"
      selectableResourceIds={new Set(sessionRows.map((session) => session.resource_id))}
    />,
  );

  assert.equal((html.match(/data-thread-status="local-outstanding"/g) ?? []).length, 2);
  assert.doesNotMatch(html, /data-thread-status="(?:synced|cloud-outstanding|divergent|pending|conflict|unavailable|unknown)"/);
  for (const scenario of scenarios) {
    if (scenario.visible) assert.match(html, new RegExp(scenario.title));
    else {
      assert.doesNotMatch(html, new RegExp(scenario.title));
      assert.doesNotMatch(html, new RegExp(`Commit ${scenario.title}`));
    }
  }
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
  // Half the loading recipe's 1.8s glint clock: every spinner shares one
  // metronome.
  assert.match(css, /\.icon-spin-frame\.spinning\s*\{[^}]*animation:\s*icon-spin 0\.9s linear infinite;/s);
});

test("sync tooltip hover keeps the sibling icon rail geometry stable", () => {
  const css = readFileSync("src/App.css", "utf8");
  const tooltipRule = css.match(/\.sync-thread-sync-indicator::after,[\s\S]+?\n\}/)?.[0] ?? "";

  assert.match(css, /\.sync-history-thread-title\s*\{[^}]*align-items:\s*center/s);
  assert.doesNotMatch(tooltipRule, /transform:/);
  // The metric tooltips need the same treatment: animating a transform promotes
  // them to their own compositing layer, which re-rasterises the text around
  // them and reads as the row — and the panel — twitching on hover.
  // Guarded on [data-tooltip]: a metric without one (the outcome list's
  // stamp) must not paint an empty tooltip box or invite a help cursor.
  // The selector list also carries the context items, which share the pattern.
  const metricTooltipRule = css.match(/\.sync-history-thread-metric\[data-tooltip\]::after[^{]*\{[^}]+\}/s)?.[0] ?? "";
  const metricHoverRule = css.match(/\.sync-history-thread-metric\[data-tooltip\]:hover::after,[\s\S]+?\n\}/)?.[0] ?? "";
  assert.doesNotMatch(css, /\.sync-history-thread-metric::after\s*\{/, "the unguarded tooltip selector must not come back");
  assert.match(metricTooltipRule, /position:\s*absolute/);
  assert.doesNotMatch(metricTooltipRule, /transform:/);
  assert.doesNotMatch(metricHoverRule, /transform:/);
  // The stamp's tooltip carries two lines separated by a newline; nowrap
  // collapsed them into one run-on line, and pre-line soft-wrapped mid-sentence.
  assert.match(metricTooltipRule, /white-space: pre;/);
  // Menu and panel reveals stay disabled so opening them does not re-rasterise
  // adjacent pixel text. Chevrons rotate instantly.
  const revealsOff = css.match(/\.sync-history-branch-menu,[\s\S]*?\{[^}]*animation: none;[^}]*\}/)?.[0] ?? "";
  for (const selector of ["\\.storage-link-menu", "\\.storage-link-actions-menu", "\\.sync-project-tab-panel"]) {
    assert.match(revealsOff, new RegExp(selector), `${selector} reveal must be disabled`);
  }
  const chevrons = css.match(/\.sync-history-branch-chevron,[\s\S]*?\{[^}]*\}/)?.[0] ?? "";
  assert.match(chevrons, /transition: color 120ms ease;/);
  assert.doesNotMatch(chevrons, /transition:[^;]*\btransform\b/);

  // A full-size button shifts one pixel while held. On the timeline's
  // 22-26px icon buttons that reads as the icon jogging against its
  // neighbours on every click, so they keep the colour feedback and drop the
  // displacement. Full-size buttons still get the press.
  assert.match(css, /\.btn:active:not\(:disabled\)\s*\{[^}]*transform: translate\(1px, 1px\);/s);
  const pressExempt = css.match(/\.sync-history-thread-actions \.sync-history-launch-action:active[\s\S]*?\{[^}]*\}/)?.[0] ?? "";
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
  for (const selector of [
    "\\.sync-history-session-details",
    "\\.sync-history-session-summary",
    "\\.sync-history-thread-meta",
  ]) {
    const rule = css.match(new RegExp(`\\n${selector} \\{[^}]*\\}`))?.[0] ?? "";
    assert.doesNotMatch(rule, /overflow:\s*(hidden|clip|auto|scroll)/, `${selector} must not clip the metric tooltip`);
  }
  assert.match(css, /\.sync-history-thread-time time\s*\{[^}]*overflow: hidden;[^}]*text-overflow: ellipsis;/s);

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
    /\.sync-thread-sync-indicator:hover,\s*\n\.sync-thread-sync-indicator:focus-visible/,
  );
});

test("Push and Pull reviews use only the outer workspace scroll", () => {
  const css = readFileSync("src/App.css", "utf8");
  const workspaceRule = css.match(/\.sync-main\.sync-sync-review-page\s*\{[^}]+\}/s)?.[0] ?? "";
  const reviewScrollRule = css.match(/\.sync-sync-review-page \.sync-sync-review-scroll\s*\{[^}]+\}/s)?.[0] ?? "";
  /* Anchored: scoped overrides of the zone (a review that owns its own top
     gutter, say) also end in `.sync-review-banners {`, and an unanchored
     match picked up whichever came first in the file rather than the zone's
     own definition. */
  const bannerZoneRule = css.match(/^\.sync-review-banners\s*\{[^}]+\}/ms)?.[0] ?? "";
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

/* The Storage card and Uncommitted card are the same object drawn in two
   subtrees, so their frames must stay aligned. */
test("the storage card wears the uncommitted card's frame", () => {
  const css = readFileSync("src/App.css", "utf8");
  const declaration = (rule: string, property: string) =>
    rule.match(new RegExp(`[^-]${property}:\\s*([^;]+);`))?.[1];

  const cardRule = css.match(/\.project-storage-card\s*\{[^}]+\}/s)?.[0] ?? "";
  const uncommittedRule = css.match(/\.sync-history-uncommitted\s*\{[^}]+\}/s)?.[0] ?? "";
  for (const property of ["border", "border-radius", "background"]) {
    assert.equal(
      declaration(cardRule, property),
      declaration(uncommittedRule, property),
      `storage card ${property} must match the uncommitted card's`,
    );
  }
  assert.match(cardRule, /margin:[^;]*var\(--sync-history-card-offset\)/,
    "the storage card must share the timeline's rail inset, not approximate it");

  // The inset and icon metrics only resolve if the card joins the timeline's
  // token scope — it lives in a different subtree, so the vars don't reach it
  // on their own and every var() would silently fall back to nothing.
  const tokenScope = css.match(/\.sync-history-timeline,[^{]*\{/s)?.[0] ?? "";
  assert.match(tokenScope, /\.project-storage-card/);

  const barRule = css.match(/\.project-storage-card > \.sync-history-section-heading\s*\{[^}]+\}/s)?.[0] ?? "";
  const uncommittedBarRule = css.match(/\.sync-history-uncommitted \.sync-history-section-heading\s*\{[^}]+\}/s)?.[0] ?? "";
  for (const property of ["min-height", "padding", "border-bottom"]) {
    assert.equal(
      declaration(barRule, property),
      declaration(uncommittedBarRule, property),
      `storage heading bar ${property} must match the uncommitted bar's`,
    );
  }
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

test("activity content renders commit and session rows without repeating project context", () => {
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
  // The provider-neutral page no longer prints a provider home path; identity
  // and provenance live in the per-row debug panel instead.
  assert.doesNotMatch(html, /Codex configuration:/);
  assert.doesNotMatch(html, /sync-history-project-context|Project directory:|Repository: mallard/);
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
  assert.match(html, /aria-label="Messages: 8\. Prompts: 3\. Replies: 5"/);
  assert.match(html, /data-tooltip="Total input · 20K\nTotal output · 4,800"/);
  assert.match(html, /aria-label="Total tokens: 25K\. Total input: 20K\. Total output: 4,800"/);
  assert.match(html, /class="sync-history-session-title-button"/);
  assert.match(html, /aria-label="View session details"/);
  assert.doesNotMatch(html, /during session|after session|started from/);
  assert.doesNotMatch(html, /Map project-owned sessions onto/);
  // Row actions come from the backend descriptors, labelled by the adapter.
  assert.match(html, /aria-label="Open in Codex"/);
  assert.match(html, /aria-label="Open in Terminal"/);
  // The action icon is keyed off the descriptor kind, not off a provider mark.
  assert.match(html, /aria-label="Show session debug info"/);
  assert.match(html, /aria-haspopup="dialog"/);
  assert.match(html, /aria-label="Session debug info"/);
  assert.match(html, /aria-label="Copy resource id"/);
  assert.match(html, /aria-label="Copy mallard thread id"/);
  assert.match(html, /aria-label="Copy native session id"/);
  assert.match(html, /aria-label="Copy transcript file"/);
  assert.match(html, /aria-label="Copy working directory"/);
  assert.doesNotMatch(html, /aria-label="Copy codex home"/);
  assert.ok(html.includes(CODEX_TRANSCRIPT_PATH));
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
  assert.match(html, /data-tooltip="Total input · 20K\nTotal output · 4,800"/);
  // Turns are one chip now — the sum on the face, each half named on its own
  // tooltip line — and the user glyph left the row with its readout.
  assert.match(html, /data-tooltip="Prompts · 3\nReplies · 5"[\s\S]*?data-icon="message"[\s\S]*?<b>8<\/b>/);
  assert.doesNotMatch(html, /sync-history-thread-metric[^>]*>[^<]*<svg[^>]*data-icon="user"/);
  assert.match(html, /data-tooltip="Tool calls · 13"/);
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
  const metaRule = css.match(/\.sync-history-session-summary \.sync-history-thread-meta\s*\{[^}]+\}/s)?.[0] ?? "";
  const launchRule = css.match(/\.sync-history-thread-actions \.sync-history-launch-action\s*\{[^}]+\}/s)?.[0] ?? "";
  const debugMenuRule = css.match(/\.sync-history-debug-menu\s*\{[^}]+\}/s)?.[0] ?? "";
  const debugPopoverRule = css.match(/\.sync-history-debug-popover\s*\{[^}]+\}/s)?.[0] ?? "";

  assert.match(metaRule, /flex:\s*0 1 auto/);
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
    css,
    /\.sync-history-thread-actions \.sync-history-launch-action,\s*\n\.sync-history-thread-actions \.sync-history-provider-inert,\s*\n\.session-detail-launch\s*\{[^}]*width:\s*22px;/s,
  );
  assert.match(css, /\.sync-history-session-summary > \.sync-history-thread-time-end\s*\{[^}]*margin-left: auto;/s);
  assert.doesNotMatch(css, /\.sync-history-session-summary \.sync-history-thread-meta\s*\{[^}]*flex:\s*1/s, "the readouts must not stretch into the clickables");
  // The menu is deliberately static so the popover anchors to the row instead
  // of to the 22px trigger — anchored to the trigger it was pushed outside the
  // card and clipped by the workspace scroll container.
  // The panel is portalled onto body and pinned to the window, so it carries no
  // static geometry at all: position, width and cap are measured as it opens.
  assert.match(debugPopoverRule, /position:\s*fixed/);
  assert.doesNotMatch(debugPopoverRule, /^\s*(top|left|right|bottom|width):/m);
  assert.doesNotMatch(debugPopoverRule, /100vw/);
  // Nothing anchors to the menu once the panel has left its subtree, and the
  // flip class went with it — placement is computed, never a CSS variant.
  assert.doesNotMatch(debugMenuRule, /position:/);
  assert.doesNotMatch(css, /\.sync-history-debug-popover\.drop-up/);
  // A row can sit where the panel fits neither above nor below. It is capped at
  // the room it lands in and scrolls inside that cap, so nothing is ever cut
  // off unreachably: header and footer hold, the value list takes the rest.
  assert.match(debugPopoverRule, /display:\s*flex/);
  assert.match(debugPopoverRule, /flex-direction:\s*column/);
  assert.match(css, /\.sync-history-debug-popover > header,\s*\n\.sync-history-debug-popover > footer\s*\{[^}]*flex: 0 0 auto;/s);
  assert.match(css, /\.sync-history-debug-values\s*\{[^}]*min-height: 0;[^}]*overflow-y: auto;/s);
  // Declaring display on the class outranks the UA sheet's [hidden] rule, so
  // the attribute has to be restated or the panel never closes.
  assert.match(css, /\.sync-history-debug-popover\[hidden\]\s*\{[^}]*display: none;/s);

  const placementSource = readFileSync("src/components/sync-review/debugPopoverPlacement.ts", "utf8");
  // The height is measured off the panel, never estimated: the row count varies
  // by provider, and the constant this replaced stood at 300 under a five-row
  // panel measuring 317, so it sent the panel into a gap it did not fit.
  // Ceiling of the fractional height, not offsetHeight's rounded integer: a
  // panel measuring 263.4 capped at 263 falls a hair short of its own content
  // and grows a scrollbar it does not need.
  assert.match(placementSource, /Math\.ceil\(panel\.getBoundingClientRect\(\)\.height\)/);
  assert.doesNotMatch(placementSource, /panel\.offsetHeight/);
  assert.doesNotMatch(placementSource, /= 300\b/);
  // Measured in layout, so the uncapped panel never paints and the scroller's
  // scrollable height never grows for a frame.
  assert.match(placementSource, /useLayoutEffect\(\(\) => \{\s*\n\s*if \(!open\) \{\s*\n\s*setPlacement\(null\);/s);
  // Measured against the window, not the scroller — that is the whole point of
  // lifting it out of the list, and .sync-main must not creep back in.
  assert.match(placementSource, /window\.innerHeight - anchor\.bottom/);
  assert.doesNotMatch(placementSource, /closest[^;]*sync-main/);
  // A fixed panel does not travel with its row, so it is dismissed rather than
  // left stranded beside the wrong session. Capture, or an inner scroll misses.
  assert.match(placementSource, /window\.addEventListener\("scroll", dismiss, true\)/);
  assert.match(placementSource, /window\.addEventListener\("resize", dismiss\)/);
  assert.match(
    placementSource,
    /panelRef\.current\s*\?\.querySelector<HTMLElement>\("button:not\(\[disabled\]\)"\)\s*\?\.focus\(\)/,
  );
  // Portalled, so "outside the menu" has to mean outside the panel too, or the
  // first click on a Copy button dismisses the thing being clicked.
  const topLayer = readFileSync("src/components/sync-review/TopLayer.tsx", "utf8");
  assert.match(topLayer, /typeof document === "undefined"/);
  // Both panels share the markup and the stylesheet, so they share the
  // placement — the catalog's copy had no placement logic at all.
  for (const page of [
    "src/components/project-sync/ProjectActivityPage.tsx",
    "src/components/project-sync/ProjectFileDetailsMenu.tsx",
    "src/components/storage-catalog/StorageCatalogPage.tsx",
  ]) {
    const source = readFileSync(page, "utf8");
    assert.match(source, /useDebugPopoverPlacement\(rootRef, panelRef, open, /, page);
    assert.match(source, /<TopLayer>/, page);
    assert.match(source, /maxHeight: `\$\{placement\.maxHeight\}px`/, page);
    assert.doesNotMatch(source, /SESSION_DEBUG_POPOVER_HEIGHT/, page);
    assert.match(source, /panelRef\.current\?\.contains\(target\)/, page);
  }
});

test("session debug copy text keeps complete local diagnostics", () => {
  const mallardThreadId = "thread-16c2fcc97307015b08a053ec0c8423e9";
  const resourceId = `codex:conversation:${mallardThreadId}`;
  assert.equal(
    sessionDebugCopyText(
      resourceId,
      mallardThreadId,
      CODEX_SESSION_ID,
      CODEX_TRANSCRIPT_PATH,
      codexSession.working_directory,
    ),
    [
      `Resource ID: ${resourceId}`,
      `Mallard thread ID: ${mallardThreadId}`,
      `Native session ID: ${CODEX_SESSION_ID}`,
      `Transcript file: ${CODEX_TRANSCRIPT_PATH}`,
      `Working directory: ${codexSession.working_directory}`,
    ].join("\n"),
  );
  // A remote-only row can lack both a provider-native id and a local
  // transcript; the copy says so rather than printing blanks.
  assert.equal(
    sessionDebugCopyText(
      "claude:conversation:thread-1",
      "thread-1",
      null,
      null,
      "/Users/test/projects/mallard",
    ),
    [
      "Resource ID: claude:conversation:thread-1",
      "Mallard thread ID: thread-1",
      "Native session ID: Not recorded",
      "Transcript file: Not recorded",
      "Working directory: /Users/test/projects/mallard",
    ].join("\n"),
  );
});

test("browse-mode session titles navigate without retaining the inline preview", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null }}
      onOpenSessionDetail={() => undefined}
    />,
  );

  assert.match(html, /class="sync-history-session-title-button"[^>]*>Add Git history mapping</);
  assert.match(html, /aria-label="View session details"/);
  assert.match(html, /data-tooltip="Prompts · 3\nReplies · 5"/);
  assert.doesNotMatch(html, /sync-history-chat-details|Load 10 older messages|Nested conversation preview/);
  assert.doesNotMatch(html, /aria-expanded="(?:true|false)"[^>]*View session details/);
});

test("activity follows the project workspace without repeating project controls", () => {
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
      activity={activityWithUncommitted}
    />,
  );

  assert.match(html, /sync-history-embedded/);
  // The workspace path bar's section crumb is the visible name for this page,
  // so the activity region drops a repeated title and keeps its accessible name.
  assert.doesNotMatch(html, /sync-history-embedded-title/);
  assert.match(html, /aria-label="Git history"/);
  assert.match(html, /class="sync-history-timeline has-uncommitted"/);
  assert.match(html, /<h2 id="uncommitted-heading"><svg[^>]*data-icon="edit"[^>]*>.*<\/svg>Uncommitted<\/h2>/s);
  // Local-only unmapped rows never conjure the storage-only group.
  assert.doesNotMatch(html, /storage-only-heading/);
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

test("storage-only sessions leave Uncommitted for their own group above the rail", () => {
  const storedId = "019f7798-5437-7632-9dbc-5b589cf68bf1";
  const storedResourceId = `codex:conversation:${storedId}`;
  const storedSession = {
    ...codexSession,
    resource_id: storedResourceId,
    provider_session_id: null,
    logical_thread_id: storedId,
    transcript_path: null,
    source: "remote_only" as const,
    title: "Add horror story document",
    working_directory: null,
    recorded_sha: null,
    actions: [],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activeStorageName="Local storage 1"
      activity={{
        ...activity,
        sessions: [codexSession, storedSession],
        unmapped: [
          { resource_id: codexResourceId, reason: "No subsequent commit" },
          { resource_id: storedResourceId, reason: "No subsequent commit" },
        ],
      }}
    />,
  );

  // The heading names the storage the way the row badges do, and the group
  // sits above the timeline — it is not on the rail.
  assert.match(html, /<h2 id="storage-only-heading"><svg[^>]*data-icon="cloud"[^>]*>.*?<\/svg>Only in <span class="sync-history-storage-name">Local storage 1<\/span><\/h2>/s);
  assert.ok(html.indexOf('id="storage-only-heading"') < html.indexOf("sync-history-timeline"));
  assert.match(html, /class="sync-history-heading-count"[^>]*aria-label="1 session not on this computer"/);
  assert.match(html, /class="sync-history-spine-fold"[^>]*aria-label="Collapse 1 session only in Local storage 1"[^>]*aria-expanded="true"[^>]*aria-controls="storage-only-list"/);
  // Each list holds only its own rows; the local draft keeps Uncommitted and
  // its fold count follows the split.
  const storageList = html.slice(html.indexOf('id="storage-only-list"'), html.indexOf('id="uncommitted-list"'));
  assert.match(storageList, /Add horror story document/);
  assert.doesNotMatch(storageList, /Add Git history mapping/);
  const uncommittedList = html.slice(html.indexOf('id="uncommitted-list"'));
  assert.match(uncommittedList, /Add Git history mapping/);
  assert.doesNotMatch(uncommittedList, /Add horror story document/);
  assert.match(html, /aria-label="Collapse 1 session not linked to a commit"/);
});

test("a page whose unmapped rows are all storage-only renders no Uncommitted group", () => {
  const storedId = "019f7798-5437-7632-9dbc-5b589cf68bf1";
  const storedResourceId = `codex:conversation:${storedId}`;
  const storedSession = {
    ...codexSession,
    resource_id: storedResourceId,
    provider_session_id: null,
    logical_thread_id: storedId,
    transcript_path: null,
    source: "remote_only" as const,
    title: "Add horror story document",
    working_directory: null,
    recorded_sha: null,
    actions: [],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activeStorageName="Local storage 1"
      activity={{
        ...activity,
        sessions: [codexSession, storedSession],
        unmapped: [{ resource_id: storedResourceId, reason: "No subsequent commit" }],
      }}
    />,
  );

  assert.match(html, /id="storage-only-heading"/);
  // No local unmapped rows: no Uncommitted frame, and the timeline drops the
  // padding that exists only to clear it.
  assert.doesNotMatch(html, /uncommitted-heading|has-uncommitted/);
  assert.match(html, /class="sync-history-timeline"/);
  // The committed local session still renders on the rail as provenance.
  assert.match(html.slice(html.indexOf("sync-history-commit-rail")), /Add Git history mapping/);
});

test("Activity badges the provenance-carrying preserved side of a divergent Pull", () => {
  const markerTitle = "Respond to greeting (Preserved 2 · Machine A) · 2026-08-02 07:54Z";
  const renderSession = (
    preserved: { preserved_from: string; ordinal: number } | null,
  ) => renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{
        ...activity,
        git: null,
        sessions: [{
          ...codexSession,
          title: markerTitle,
          preserved,
        }],
      }}
    />,
  );

  const preserved = renderSession({
    preserved_from: "019fac21-4aba-7fc2-bb7d-df711c950987",
    ordinal: 2,
  });
  assert.match(preserved, /class="sync-history-preserved-badge"/);
  assert.match(preserved, /title="Local branch preserved during a divergent Pull of thread 019fac21\."/);
  assert.match(preserved, />Preserved 2<\/span>/);
  // The compacted title drops only the trailing timestamp; the marker stays.
  assert.match(preserved, /Respond to greeting \(Preserved 2 · Machine A\)/);

  const titleOnly = renderSession(null);
  assert.doesNotMatch(titleOnly, /sync-history-preserved-badge|>Preserved 2<\/span>/);
});

test("non-Git activity is named once, and not on screen", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
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
  const flatSessionRule = css.match(/\.sync-history-thread-list\.flat > \.sync-history-thread-card\s*\{[^}]+\}/s)?.[0] ?? "";
  // 8px, not 0. The compact review was zeroed while a section heading with its
  // own top margin led the body; that heading is suppressed, so a
  // timeline leads with a card and a flat list with a row — neither carries a
  // margin, and both sat their first border on the header's hairline.
  assert.match(compactReviewRule, /padding-top:\s*8px/);
  assert.match(compactReviewRule, /padding-right:\s*8px/);
  assert.match(compactHeadingRule, /margin-top:\s*8px/);
  assert.doesNotMatch(flatSessionRule, /background:/, "flat-list chrome must not override selected-row tint");
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
  const decision = (action: ConversationReviewAction, selectable: boolean) => ({
    action,
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
        conversation_resource_id: syncedSyncResourceId,
        resource_id: syncedSyncResourceId,
        display_name: codexSession.title,
        state: "synced",
        local_present: true,
        storage_present: true,
        cloud_thread_id: CODEX_SESSION_ID,
        compared_against_storage: true,
        provider_thread_id: CODEX_SESSION_ID,
        push_decision: decision("no_changes", false),
        pull_decision: decision("no_changes", false),
      },
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: localAheadId,
        conversation_resource_id: localAheadResourceId,
        resource_id: localAheadSyncResourceId,
        display_name: localAheadSession.title,
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        cloud_thread_id: localAheadId,
        compared_against_storage: true,
        provider_thread_id: localAheadId,
        push_decision: decision("publish", true),
        pull_decision: decision("report_push_eligible", false),
      },
    ],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
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
    transcript_path: null,
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
        conversation_resource_id: codexResourceId,
        resource_id: localSyncResourceId,
        display_name: codexSession.title,
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        cloud_thread_id: CODEX_SESSION_ID,
        compared_against_storage: true,
        provider_thread_id: CODEX_SESSION_ID,
        local_updated_at: codexSession.ended_at,
        storage_updated_at: codexSession.ended_at - 60,
        push_decision: reviewDecision("publish", true),
        pull_decision: reviewDecision("report_push_eligible"),
      },
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: remoteId,
        conversation_resource_id: remoteResourceId,
        resource_id: remoteSyncResourceId,
        display_name: "Improve cloud thread titles",
        state: "storage_only",
        local_present: false,
        storage_present: true,
        cloud_thread_id: remoteId,
        compared_against_storage: true,
        storage_updated_at: 1_752_802_500,
        push_decision: reviewDecision("no_changes"),
        pull_decision: reviewDecision("create_local", true),
      },
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: olderId,
        conversation_resource_id: `codex:conversation:${olderId}`,
        resource_id: `codex:conversation:${olderId}`,
        display_name: olderId,
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        cloud_thread_id: olderId,
        compared_against_storage: true,
        provider_thread_id: olderId,
        local_updated_at: activity.window_start - 60,
        storage_updated_at: activity.window_start - 120,
        push_decision: reviewDecision("publish", true),
        pull_decision: reviewDecision("report_push_eligible"),
      },
    ],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
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
      conversation_resource_id: codexResourceId,
      resource_id: `codex:conversation:${CODEX_SESSION_ID}`,
      display_name: CODEX_SESSION_ID,
      state: "unavailable",
      local_present: true,
      storage_present: false,
      compared_against_storage: true,
      provider_thread_id: CODEX_SESSION_ID,
      status_detail: "Session file exceeds the 100 MiB per-file sync limit.",
      local_updated_at: codexSession.ended_at,
      push_decision: reviewDecision("not_pushable"),
      pull_decision: reviewDecision("report_local_only"),
    }],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null }}
      comparison={comparison}
      activeStorageName="Local storage 1"
    />,
  );

  assert.match(html, /aria-label="Unavailable for sync\. Session file exceeds the 100 MiB per-file sync limit\."/);
  assert.doesNotMatch(html, /can.t tell which copy is newer/);
});

test("Push review hides assignment-conflicted sessions", () => {
  const reason = "Codex Desktop assigns this thread here, but its authoritative home belongs to another live Codex Desktop project.";
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{
        ...activity,
        git: null,
        sessions: [{ ...codexSession, sync_blocked_reason: reason }],
      }}
      comparison={emptyComparison()}
      activeStorageName="Local storage 1"
      selectionMode="push"
      selectedResourceIds={new Set()}
      selectableResourceIds={new Set([codexResourceId])}
      onToggleResource={() => undefined}
    />,
  );

  assert.doesNotMatch(html, /Add Git history mapping/);
  assert.doesNotMatch(html, new RegExp(reason.replace(/\./g, "\\.")));
  assert.doesNotMatch(html, /sync-sync-row-choice/);
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
    transcript_path: CLAUDE_TRANSCRIPT_PATH,
    title: "Claude twin",
    actions: [
      {
        action_id: "claude.open_in_app",
        kind: "open_in_app" as const,
        label: "Open in Claude",
        enabled: true,
        disabled_reason: null,
      },
      {
        action_id: "claude.open_in_terminal",
        kind: "open_in_terminal" as const,
        label: "Open in Terminal",
        enabled: true,
        disabled_reason: null,
      },
    ],
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
  const decision: ThreadSyncDecision = {
    action: "create_cloud",
    selectable: true,
  };
  const comparison = emptyComparison({
    entries: [
      {
        provider: "codex",
        provider_display_name: "Codex",
        access: "bound_ready",
        thread_id: nativeId,
        conversation_resource_id: codexTwin.resource_id,
        resource_id: codexSyncResourceId,
        display_name: codexTwin.title,
        state: "local_only",
        local_present: true,
        storage_present: false,
        cloud_thread_id: nativeId,
        compared_against_storage: true,
        provider_thread_id: nativeId,
        push_decision: decision,
        pull_decision: { ...decision, action: "report_push_eligible", selectable: false },
      },
      {
        provider: "claude",
        provider_display_name: "Claude",
        access: "bound_ready",
        thread_id: nativeId,
        conversation_resource_id: claudeTwin.resource_id,
        resource_id: claudeSyncResourceId,
        display_name: claudeTwin.title,
        state: "local_only",
        local_present: true,
        storage_present: false,
        cloud_thread_id: nativeId,
        compared_against_storage: true,
        provider_thread_id: nativeId,
        push_decision: decision,
        pull_decision: { ...decision, action: "report_push_eligible", selectable: false },
      },
    ],
  });
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
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
  // Each row wears its own provider's mark, and both provider marks launch the
  // corresponding desktop app.
  assert.match(html, /aria-label="Open in Codex"><svg[^>]*data-icon="openai"/);
  assert.match(html, /aria-label="Open in Claude"><svg[^>]*data-icon="claude"/);
  assert.ok(html.includes(CODEX_TRANSCRIPT_PATH));
  assert.ok(html.includes(CLAUDE_TRANSCRIPT_PATH));
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
  // Selection is keyed by resource id, so ticking the Claude twin cannot
  // leak onto the Codex twin that shares its native id.
  assert.match(html, /<input[^>]*aria-label="Exclude Claude twin"[^>]*checked=""/);
  const codexInput = html.match(/<input[^>]*aria-label="Include Codex twin"[^>]*>/)?.[0] ?? "";
  assert.notEqual(codexInput, "");
  assert.doesNotMatch(codexInput, /checked=""/);
});

test("Pull review hides view-only rows but keeps their provider notice", () => {
  const claudeRemote = {
    ...codexSession,
    resource_id: "claude:conversation:claude-thread-1",
    provider: "claude",
    provider_display_name: "Claude",
    provider_session_id: null,
    logical_thread_id: "claude-thread-1",
    transcript_path: null,
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
      // A listed resource ID cannot override the Pull status filter.
      selectableResourceIds={new Set([claudeRemote.resource_id, unsupportedRemote.resource_id])}
    />,
  );

  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 0);
  assert.doesNotMatch(html, /sync-sync-selectable-row/);
  assert.doesNotMatch(html, /Claude remote session|Unknown provider session/);
  assert.doesNotMatch(html, /aria-label="Copy transcript file"/);
  // The per-provider notice explains the access state once, with the backend's
  // detail carried along — now as one item in the page's single notice zone
  // rather than a box of its own.
  assert.match(html, /class="notice [a-z]+ sync-history-notices"/);
  assert.match(html, /<strong class="notice-lead">Claude<\/strong> — View only — connect a Claude profile to sync these sessions\. Sessions were published from another machine\./);
  // Review rows and storage-only rows never navigate or download implicitly.
  assert.doesNotMatch(html, /sync-history-session-title-button|aria-label="View session details"/);
});

test("stored-only sessions lead the non-git flat list under an Only-in-storage heading", () => {
  // The Catalog page's partition grammar, applied to the flat Sessions face:
  // stored-only rows lift into a plain "Only in <storage>" section — no
  // frame, no fold, no count, unlike the git face's framed group — and the
  // local half is headed "Sessions" only while the partition exists.
  const storedOnly = {
    ...codexSession,
    resource_id: "codex:conversation:stored-only-1",
    provider_session_id: null,
    logical_thread_id: "stored-only-1",
    transcript_path: null,
    source: "remote_only" as const,
    title: "Stored-only session",
    working_directory: null,
    actions: [],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null, sessions: [storedOnly, codexSession] }}
      activeStorageName="Local storage 1"
    />,
  );
  const cloudHeading = html.indexOf('sync-history-storage-name">Local storage 1');
  const storedTitle = html.indexOf("Stored-only session");
  const localHeading = html.indexOf("Sessions</h2>");
  const localTitle = html.indexOf("Add Git history mapping");
  assert.ok(cloudHeading >= 0, "the partition is named after the storage");
  assert.ok(storedTitle > cloudHeading, "stored-only sessions render inside the partition");
  assert.ok(localHeading > storedTitle, "the local half is headed while the partition exists");
  assert.ok(localTitle > localHeading, "local sessions follow their heading");
  assert.doesNotMatch(html, /sync-history-storage-only|sync-history-spine-fold|sync-history-heading-count/);

  // Alone, the leaf crumb already names the page: no partition, no headings.
  const plain = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null, sessions: [codexSession] }}
      activeStorageName="Local storage 1"
    />,
  );
  assert.doesNotMatch(plain, /sync-history-section-heading|Sessions<\/h2>/);
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
      activity={{ ...activity, git: null, sessions: [localSession, heldSession] }}
    />,
  );

  // Enabled actions carry their adapter label. Open wears the provider's own
  // mark — it is the row's identity as well as its button; the other kinds are
  // the action, not the agent, so they keep a generic glyph.
  const openButton = browseHtml.match(/<button[^>]*aria-label="Open in Codex"[^>]*>[\s\S]*?<\/button>/g) ?? [];
  assert.equal(openButton.length, 2);
  assert.match(openButton[0] ?? "", /data-icon="openai"/);
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
      activity={{ ...activity, git: null, sessions: [localSession, heldSession] }}
      comparison={emptyComparison({
        entries: [localSession, heldSession].map((session): ThreadSyncEntry => ({
          provider: "codex",
          provider_display_name: "Codex",
          access: "bound_ready",
          thread_id: session.logical_thread_id,
          conversation_resource_id: session.resource_id,
          resource_id: session.resource_id,
          display_name: session.title,
          state: "local_only",
          local_present: true,
          storage_present: false,
          compared_against_storage: true,
          provider_thread_id: session.provider_session_id,
          push_decision: reviewDecision("create_cloud", true),
          pull_decision: reviewDecision("report_local_only"),
        })),
      })}
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

  // A comparison that failed while the page loaded is standing content: it is
  // there on first paint, so the zone is a named region. `role="alert"` here
  // announced a condition the reader had never not had.
  assert.match(html, /class="notice warning sync-history-notices" role="region" aria-label="Notes about this project activity"/);
  assert.doesNotMatch(html, /role="alert"/);
  assert.match(html, /<strong class="notice-lead">Claude<\/strong> comparison failed: The Claude profile is locked by another process\./);
  // The failure is isolated: the healthy provider's rows still render.
  assert.match(html, /Add Git history mapping/);
  assert.doesNotMatch(html, /<strong>Codex<\/strong> comparison failed/);
});

test("a project without a bound provider profile stays in the workspace", () => {
  let choseProfile = false;
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      binding={null}
      activity={null}
      actionError="Provider profile path changed."
      onChooseAgentProfile={() => { choseProfile = true; }}
    />,
  );
  assert.match(html, /role="alert"/);
  assert.match(html, /No agent profile is connected\./);
  assert.match(html, /Choose an Agent profile to load this project’s sessions\./);
  assert.match(html, /<button[^>]*>Choose Agent profile<\/button>/);
  assert.doesNotMatch(html, /No Codex profile is connected/);
  assert.doesNotMatch(html, /Remove and add the project again/);
  assert.doesNotMatch(html, /Open Project Settings|Project settings/);
  assert.equal(choseProfile, false, "server rendering must not run the action");

  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /onChooseAgentProfile=\{\(\) => \{\s*void onConfigureBinding\(activeProject\.local_project_id\);/);
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
      storageDrafts={[]}
      onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined}
      activeProjectId={project.local_project_id}
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      catalogIndexOpen={false}
      storageIndexOpen={false}
      settingsOpen={false}
      loading={false}
      busy={false}
      activityOpen={false}
      unreadLogs={0}
      onOpenProjects={() => undefined}
      onSelectProject={() => undefined}
      onRemoveProject={() => undefined}
      onSelectDraft={() => undefined}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => undefined}
      onOpenSettings={() => undefined}
      onOpenStorage={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogIndex={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
      onOpenStorageIndex={() => undefined}
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
  const settingsNavIndex = primaryNav.indexOf(" Settings</button>");
  const logsNavIndex = primaryNav.indexOf(" Logs</button>");
  assert.ok(projectsNavIndex >= 0);
  // Catalog is a destination like Projects and Storages, so it joins them in
  // the nav; Settings sits above Logs, and Logs stays last because it toggles
  // a drawer instead of navigating.
  assert.ok(catalogNavIndex > projectsNavIndex);
  assert.ok(storagesNavIndex > catalogNavIndex);
  assert.ok(settingsNavIndex > storagesNavIndex);
  assert.ok(logsNavIndex > settingsNavIndex);
  assert.match(html, /aria-label="Storages"/);
  assert.match(html, />Storages<\/span>/);
  const catalogPaneIndex = html.indexOf('<section class="sidebar-split-pane sidebar-catalog-pane"');
  const storagePaneIndex = html.indexOf('<section class="sidebar-split-pane sidebar-storage-pane"');
  assert.ok(catalogPaneIndex >= 0 && storagePaneIndex > catalogPaneIndex);
  // Every seam down the rail is a handle — each one is the same 3px of frame
  // colour, so any that did nothing read as a handle that was simply broken.
  assert.match(html, /role="separator" aria-label="Resize navigation and Projects sections" aria-orientation="horizontal"/);
  assert.match(html, /aria-valuetext="Navigation height: fits contents"/);
  assert.match(html, /role="separator" aria-label="Resize Projects and Catalog sections" aria-orientation="horizontal"/);
  assert.match(html, /role="separator" aria-label="Resize Catalog and Storages sections" aria-orientation="horizontal"/);
  assert.match(html, /aria-valuenow="56"/);
  // Storages is content-sized (capped at 40% of the box), so it reports fitting
  // its contents until the handle pins it to a height rather than carrying a
  // share of the box.
  assert.match(html, /aria-valuetext="Storage height: fits contents"/);
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
      storageDrafts={[]}
      onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined}
      activeProjectId="project-multiple-profiles"
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      catalogIndexOpen={false}
      storageIndexOpen={false}
      settingsOpen={false}
      loading={false}
      busy={false}
      activityOpen={false}
      unreadLogs={0}
      onOpenProjects={() => undefined}
      onSelectProject={() => undefined}
      onRemoveProject={() => undefined}
      onSelectDraft={() => undefined}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => undefined}
      onOpenSettings={() => undefined}
      onOpenStorage={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogIndex={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
      onOpenStorageIndex={() => undefined}
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

/* One prop bundle for the collapse tests; the fixtures vary per test. */
const sidebarChrome = {
  activeDraftId: null,
  storageUsage: {},
  onOpenStorageDraft: () => undefined,
  onDiscardStorageDraft: () => undefined,
  activeProjectId: null,
  activeStorageId: null,
  activeCatalogScopeId: null,
  addingCatalogScope: false,
  settingsOpen: false,
  catalogIndexOpen: false,
  storageIndexOpen: false,
  loading: false,
  busy: false,
  activityOpen: false,
  unreadLogs: 0,
  onOpenProjects: () => undefined,
  onSelectProject: () => undefined,
  onRemoveProject: () => undefined,
  onSelectDraft: () => undefined,
  onDiscardDraft: () => undefined,
  onToggleActivity: () => undefined,
  onAddProject: () => undefined,
  onOpenSettings: () => undefined,
  onOpenStorage: () => undefined,
  onRemoveStorage: () => undefined,
  onAddStorage: () => undefined,
  onOpenCatalogIndex: () => undefined,
  onOpenCatalogScope: () => undefined,
  onAddCatalogScope: () => undefined,
  onRemoveCatalogScope: () => undefined,
  onOpenStorageIndex: () => undefined,
} satisfies Partial<ComponentProps<typeof ProjectSidebar>>;

test("every section heading is a disclosure and a fresh render opens all three", () => {
  const html = renderToStaticMarkup(
    <ProjectSidebar
      {...sidebarChrome}
      projects={[project]}
      drafts={[]}
      storages={[]}
      storageDrafts={[]}
      catalogScopes={[]}
    />,
  );
  // The label half of each heading is the toggle — a real button, so the add
  // action must be its sibling, never its child.
  assert.equal((html.match(/sidebar-section-toggle/g) ?? []).length, 3);
  assert.equal((html.match(/aria-expanded="true" aria-controls="sidebar-[a-z]+-list"/g) ?? []).length, 3);
  for (const section of ["projects", "catalog", "storages"]) {
    assert.match(html, new RegExp(`aria-controls="sidebar-${section}-list"`));
    assert.match(html, new RegExp(`id="sidebar-${section}-list"`));
  }
  // Open panes hide nothing and carry no collapsed dressing: no hidden lists,
  // no closed-tile class, no count chips (gated above too), live handles only.
  assert.doesNotMatch(html, /hidden=""/);
  assert.doesNotMatch(html, /sidebar-split-pane sidebar-[a-z]+-pane collapsed/);
  assert.doesNotMatch(html, /sidebar-section-divider inert/);
  assert.doesNotMatch(html, /aria-disabled="true" tabindex="-1"/);
});

test("collapsed sections keep their signals and quiet their handles", () => {
  // The collapse set is read from browser storage at render time, and these
  // renders are the one place a stubbed window stands in for it.
  const globals = globalThis as { window?: unknown };
  globals.window = {
    localStorage: {
      getItem: (key: string) => (key === "mallard.sidebar-section-collapsed" ? "projects,storages" : null),
    },
  };
  try {
    const html = renderToStaticMarkup(
      <ProjectSidebar
        {...sidebarChrome}
        projects={[project, { ...project, local_project_id: "project-no-profile", local_alias: "No profile", profile_ids: {} }]}
        drafts={[]}
        storages={[{
          id: "storage-local",
          name: "Local storage 1",
          kind: "local",
          bucket: "",
          access_key_id: "",
          secret_access_key: "",
          account_id: "",
          s3_endpoint: "",
          region: "",
          local_dir: "/tmp/storage",
        }]}
        storageIssues={{ "storage-local": "The storage folder is missing." }}
        storageDrafts={[]}
        catalogScopes={[]}
      />,
    );
    // Closed panes: toggle reports the state, the list is hidden, the tile
    // wears the collapsed class; Catalog stays open between them. (Scoped to
    // the toggles — the Logs nav button also carries aria-expanded.)
    assert.equal((html.match(/class="sidebar-section-toggle" aria-expanded="false"/g) ?? []).length, 2);
    assert.equal((html.match(/class="sidebar-section-toggle" aria-expanded="true"/g) ?? []).length, 1);
    assert.equal((html.match(/hidden=""/g) ?? []).length, 2);
    assert.match(html, /sidebar-split-pane sidebar-project-pane collapsed/);
    assert.match(html, /sidebar-split-pane sidebar-storage-pane collapsed/);
    assert.match(html, /sidebar-split-pane sidebar-catalog-pane"/);
    // A closed heading still reports its list: the count chip, and the flag
    // named on the button rather than only drawn (both panes carry one here).
    assert.equal((html.match(/sidebar-section-count/g) ?? []).length, 2);
    assert.match(html, /aria-label="Projects, 2 rows, contains a flagged row"/);
    assert.match(html, /aria-label="Storages, 1 row, contains a flagged row"/);
    assert.equal((html.match(/sidebar-section-alert/g) ?? []).length, 2);
    // Both handles border a closed pane, so both are inert: seam kept, drag
    // affordance and keyboard stop gone.
    assert.equal((html.match(/sidebar-section-divider inert/g) ?? []).length, 2);
    assert.equal((html.match(/aria-disabled="true" tabindex="-1"/g) ?? []).length, 2);
    // Closed rows are min-content; the open Catalog keeps its elastic factor
    // between them. (The factor's digits are floating-point noise; the row
    // shape is what this pins.)
    assert.match(html, /grid-template-rows:min-content 8px minmax\(96px, [0-9.]+fr\) 8px min-content/);
    // The closed Storages tile sheds its 96px floor with its list.
    assert.doesNotMatch(html, /aria-label="Storages"[^>]*style="min-height:96px"/);
  } finally {
    delete globals.window;
  }
});

test("a closed Catalog passes the Storages pin through both gutters", () => {
  const globals = globalThis as { window?: unknown };
  globals.window = {
    localStorage: {
      getItem: (key: string) => (key === "mallard.sidebar-section-collapsed" ? "catalog" : null),
    },
  };
  try {
    const html = renderToStaticMarkup(
      <ProjectSidebar
        {...sidebarChrome}
        projects={[project]}
        drafts={[]}
        storages={[]}
        storageDrafts={[]}
        catalogScopes={[]}
      />,
    );
    // Neither gutter dies with the closed tile between them: both re-pin
    // Storages across it, named for the panes the drag actually moves, the
    // way a sash resizes across a collapsed view in VS Code.
    assert.doesNotMatch(html, /sidebar-section-divider inert/);
    assert.equal(
      (html.match(/aria-label="Resize Projects and Storages sections"/g) ?? []).length,
      2,
    );
    // Both carry the pin's value, not the share's percentage.
    assert.equal((html.match(/aria-valuetext="Storage height: fits contents"/g) ?? []).length, 2);
    assert.doesNotMatch(html, /aria-valuenow="56"/);
  } finally {
    delete globals.window;
  }
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
        storageDrafts={[]}
        onOpenStorageDraft={() => undefined}
        onDiscardStorageDraft={() => undefined}
        activeProjectId={null}
        activeStorageId={null}
        catalogScopes={scopes}
        activeCatalogScopeId={activeScopeId}
        addingCatalogScope={false}
        catalogIndexOpen={false}
        storageIndexOpen={false}
        settingsOpen={false}
        loading={false}
        busy={false}
        activityOpen={false}
        unreadLogs={0}
        onOpenProjects={() => undefined}
        onSelectProject={() => undefined}
        onRemoveProject={() => undefined}
        onSelectDraft={() => undefined}
        onDiscardDraft={() => undefined}
        onToggleActivity={() => undefined}
        onAddProject={() => undefined}
        onOpenSettings={() => undefined}
        onOpenStorage={() => undefined}
        onRemoveStorage={() => undefined}
        onAddStorage={() => undefined}
        onOpenCatalogIndex={() => undefined}
        onOpenCatalogScope={() => undefined}
        onAddCatalogScope={() => undefined}
        onRemoveCatalogScope={() => undefined}
        onOpenStorageIndex={() => undefined}
      />,
    );
  const catalogPane = (html: string) => {
    const start = html.indexOf('aria-label="Catalog"');
    return html.slice(start, html.indexOf("</section>", start));
  };

  // Fresh state: no permanent Skills/Plugins/Pets rows, just the Add prompt.
  const emptyHtml = render([], null);
  const emptyPane = catalogPane(emptyHtml);
  assert.match(emptyPane, /aria-label="Add Catalog resource"/);
  assert.match(emptyPane, /Add your first resource/);
  assert.doesNotMatch(emptyPane, /<span>Skills<\/span>|<span>Plugins<\/span>|<span>Pets<\/span>/);

  // Two Codex configs give the same kind two distinct rows, labelled by
  // config name; pet v1 and v2 from one config are two separate rows.
  const html = render([
    scopeView("catscope-1", "agent_skill", "Codex home"),
    scopeView("catscope-2", "agent_skill", "conf2"),
    scopeView("catscope-3", "codex_pet_v1", "Codex home"),
    scopeView("catscope-4", "codex_pet_v2", "Codex home"),
  ], "catscope-2");
  const pane = catalogPane(html);
  assert.match(pane, /<span>Skills · Codex home<\/span>/);
  assert.match(pane, /<span>Skills · conf2<\/span>/);
  assert.match(pane, /<span>Pets v1 · Codex home<\/span>/);
  assert.match(pane, /<span>Pets v2 · Codex home<\/span>/);
  assert.match(pane, /sidebar-profile-item active[^]*?Skills · conf2/);
  assert.match(pane, /aria-label="Remove the Skills · conf2 row"/);
  assert.doesNotMatch(pane, /Add your first resource/);
});

test("an empty Storages pane prompts the way Projects and Catalog do", () => {
  const render = (storages: ComponentProps<typeof ProjectSidebar>["storages"]) =>
    renderToStaticMarkup(
      <ProjectSidebar
        projects={[project]}
        drafts={[]}
        activeDraftId={null}
        storages={storages}
        storageUsage={{}}
        storageDrafts={[]}
        onOpenStorageDraft={() => undefined}
        onDiscardStorageDraft={() => undefined}
        activeProjectId={null}
        activeStorageId={null}
        catalogScopes={[]}
        activeCatalogScopeId={null}
        addingCatalogScope={false}
        catalogIndexOpen={false}
        storageIndexOpen={false}
        settingsOpen={false}
        loading={false}
        busy={false}
        activityOpen={false}
        unreadLogs={0}
        onOpenProjects={() => undefined}
        onSelectProject={() => undefined}
        onRemoveProject={() => undefined}
        onSelectDraft={() => undefined}
        onDiscardDraft={() => undefined}
        onToggleActivity={() => undefined}
        onAddProject={() => undefined}
        onOpenSettings={() => undefined}
        onOpenStorage={() => undefined}
        onRemoveStorage={() => undefined}
        onAddStorage={() => undefined}
        onOpenCatalogIndex={() => undefined}
        onOpenCatalogScope={() => undefined}
        onAddCatalogScope={() => undefined}
        onRemoveCatalogScope={() => undefined}
        onOpenStorageIndex={() => undefined}
      />,
    );

  const storagePane = (html: string) => {
    const start = html.indexOf('aria-label="Storages"');
    return html.slice(start, html.indexOf("</section>", start));
  };

  // The three panes say the same thing when they are empty, in the same row.
  const empty = storagePane(render([]));
  assert.match(empty, /class="sync-sidebar-empty"/);
  assert.match(empty, /Add your first storage/);

  // And it is a first-run prompt, not a permanent Add row.
  const settled = storagePane(render([{
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
  }]));
  assert.match(settled, /<span>Local storage 1<\/span>/);
  assert.doesNotMatch(settled, /Add your first storage/);
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
      }]}
      storageUsage={{ "storage-1": 1 }}
      storageDrafts={[]}
      onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined}
      storageIssues={{ "storage-1": "Storage folder unavailable." }}
      activeProjectId={project.local_project_id}
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      catalogIndexOpen={false}
      storageIndexOpen={false}
      settingsOpen={false}
      loading={false}
      busy
      activityOpen={false}
      unreadLogs={0}
      onOpenProjects={() => undefined}
      onSelectProject={() => undefined}
      onRemoveProject={() => undefined}
      onSelectDraft={() => undefined}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => undefined}
      onOpenSettings={() => undefined}
      onOpenStorage={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogIndex={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
      onOpenStorageIndex={() => undefined}
    />,
  );

  const projectMarker = html.indexOf("<span>Mallard local</span>");
  const projectButton = html.slice(html.lastIndexOf("<button", projectMarker), html.indexOf(">", html.lastIndexOf("<button", projectMarker)) + 1);
  const storageMarker = html.indexOf("<span>Local storage 1</span>");
  const storageButton = html.slice(html.lastIndexOf("<button", storageMarker), html.indexOf(">", html.lastIndexOf("<button", storageMarker)) + 1);
  const addStorageMarker = html.indexOf('aria-label="Add storage"');
  const addStorageButton = html.slice(html.lastIndexOf("<button", addStorageMarker), html.indexOf(">", addStorageMarker) + 1);
  const removeStorageMarker = html.indexOf('aria-label="Remove Local storage 1 from Mallard"');
  const removeStorageButton = html.slice(
    html.lastIndexOf("<button", removeStorageMarker),
    html.indexOf("</button>", removeStorageMarker) + "</button>".length,
  );

  assert.match(projectButton, /disabled=""/);
  assert.match(storageButton, /disabled=""/);
  assert.match(addStorageButton, /disabled=""/);
  assert.match(removeStorageButton, /class="sidebar-profile-remove"/);
  assert.match(removeStorageButton, /disabled=""/);
  assert.match(removeStorageButton, /data-icon="trash"/);
  assert.doesNotMatch(html, /data-icon="more"/);
  assert.match(html, /sync-sidebar-storage-warning/);
  assert.match(storageButton, /Storage issue: Storage folder unavailable\./);
});

test("the rail's foot names the day one press lands on", () => {
  // The control it replaced said "Load previous 30 days" and floated at the
  // very bottom of the page, below the Older activity fold — detached from the
  // rail it paged, and unable to say whether the next click landed on anything.
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} />,
  );
  const marker = html.indexOf("sync-history-rail-foot");
  assert.ok(marker > 0, "the foot renders");
  const foot = html.slice(html.lastIndexOf("<div", marker), html.indexOf("</div>", html.indexOf("</button>", marker)));
  assert.match(foot, /<strong>Load earlier<\/strong>/);
  // The counts stay available without being printed at everyone.
  assert.match(foot, /aria-label="Load earlier history — through [^"]*, 41 commits and 6 sessions"/);
  // The node is the foot's, not a button's: two controls cannot each own it.
  assert.match(foot, /class="sync-history-rail-foot-node"[^>]*>.*?data-icon="chevron-down"/s);
  assert.doesNotMatch(foot, /icon-spin/);
  // One window loaded, so there is nothing to pop.
  assert.doesNotMatch(foot, /Show less/);
  // It ends the timeline it pages, rather than trailing the whole page.
  assert.ok(
    html.indexOf("sync-history-rail-foot") > html.indexOf("sync-history-commit-rail"),
    "the foot comes after the rail",
  );
  assert.doesNotMatch(html, /sync-history-load-more|Load previous 30 days/);
});

test("Show less appears only once a window has been loaded onto the stack", () => {
  // Paging was a one-way door until this existed: the merged view had no way
  // back to the window it opened on.
  const one = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} />,
  );
  assert.doesNotMatch(one, /Show less/);

  const paged = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} canShowLess />,
  );
  assert.match(paged, /<strong>Show less<\/strong>/);
  assert.match(paged, /<strong>Load earlier<\/strong>/);
  // Both are steps in the same foot, so one node serves them.
  const nodes = paged.match(/class="sync-history-rail-foot-node"/g) ?? [];
  assert.equal(nodes.length, 1);

  // At the end of the history there is nothing to load, but still something to
  // pop — so the foot stays and the floor does not claim the last word.
  const spent = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={{ ...activity, older: null }} canShowLess />,
  );
  assert.match(spent, /<strong>Show less<\/strong>/);
  assert.doesNotMatch(spent, /Load earlier|sync-history-rail-floor/);
});

test("a fetch in flight moves the node and nothing else", () => {
  // The labels and the date hold their positions because the spinner lands in
  // the rail node, a 24px slot outside the row. The date holds because the
  // fetch is still running *through* that day.
  const ready = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} />,
  );
  const busy = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} loadingMore />,
  );
  assert.match(busy, /class="sync-history-rail-foot-node"[^>]*>.*?data-icon="refresh"[^>]*class="icon-spin"/s);
  assert.match(busy, /<strong>Loading…<\/strong>/);
  assert.match(busy, /disabled=""/);
  // Same stamp, in the same slot, either way.
  const stamp = (html: string) => html.match(/<time datetime="[^"]*">· ([^<]*)<\/time>/)?.[1];
  assert.equal(stamp(busy), stamp(ready));
});

test("nothing older draws no terminal footer or reload control", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={{ ...activity, older: null }} />,
  );
  assert.doesNotMatch(html, /sync-history-rail-step/);
  assert.doesNotMatch(html, /sync-history-rail-floor|nothing earlier/);
});

test("an empty rail is still the rail, and says which of three things happened", () => {
  // One sentence used to cover all three, blaming a 30-day window even for the
  // two the window has nothing to do with — where no amount of paging would
  // ever have produced a commit.
  const noCommits = { ...activity.git, commits: [], unique_session_count: 0 };
  const emptyWindow = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={{ ...activity, git: noCommits }} />,
  );
  assert.doesNotMatch(emptyWindow, /No commits are available in this 30-day window/);
  assert.match(emptyWindow, /class="sync-history-commit sync-history-commit-gap"/);
  assert.match(emptyWindow, /<strong>No commits<\/strong>/);
  // This is the one empty rail paging could still fill, so its mark says span,
  // not absence: `clock` said time, and was the heaviest stroke on the row.
  assert.match(emptyWindow, /data-icon="range"/);
  assert.doesNotMatch(emptyWindow, /data-icon="clock"/);
  // The gap sits on the rail, so the terminus below still has a spine to end.
  assert.ok(emptyWindow.indexOf("sync-history-commit-gap") > emptyWindow.indexOf("sync-history-commit-rail"));
  assert.match(emptyWindow, /sync-history-rail-step/);

  const unborn = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: noCommits, older: { newest_at: 1_749_000_000, commits: 0, sessions: 6 } }}
    />,
  );
  assert.match(unborn, /<strong>No commits yet<\/strong>/);

  const historyOnly = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{
        ...activity,
        git: { ...noCommits, branches: [{ name: "main", is_current: false, available: false }] },
      }}
    />,
  );
  assert.match(historyOnly, /<strong>Branch not checked out here<\/strong>/);
  assert.match(historyOnly, /data-icon="git-branch"/);
});

test("a project with no repository gets the same control, minus the rail", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={{ ...activity, git: null }} />,
  );
  assert.match(html, /class="sync-history-rail-foot no-rail"/);
  assert.match(html, /<strong>Load earlier<\/strong>/);
});

test("a review pages too, and the fold waits until paging is spent", () => {
  // Two doors into one room, otherwise: while anything is still loadable the
  // terminus is the route, so the fold stays out of the way. It cannot just be
  // deleted — the comparison is not windowed and the terminus only walks the
  // session timeline, so an entry with no session row at all is reachable by
  // no amount of paging.
  const entry: ThreadSyncEntry = {
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    thread_id: "orphan",
    conversation_resource_id: "codex:conversation:orphan",
    resource_id: "codex:conversation:orphan",
    display_name: "A thread no session row reaches",
    state: "local_only",
    local_present: true,
    storage_present: false,
    compared_against_storage: true,
    push_decision: { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
  };
  const comparison = emptyComparison({ entries: [entry] });

  // More to load: the terminus is the way back, and the fold holds its peace.
  const paging = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults} activity={activity} selectionMode="push" comparison={comparison} />,
  );
  assert.match(paging, /sync-history-rail-step/);
  assert.match(paging, /<strong>Load earlier<\/strong>/);
  assert.doesNotMatch(paging, /Older activity/);

  // Paging spent: whatever the comparison still holds is the true remainder,
  // and a review has to offer it or it drops candidates silently.
  const spent = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults} activity={{ ...activity, older: null }} selectionMode="push" comparison={comparison} />,
  );
  assert.doesNotMatch(spent, /sync-history-rail-step/);
  assert.match(spent, /Older activity/);
  assert.match(spent, /A thread no session row reaches/);
  // A spent review uses the remainder fold, never a terminal footer.
  assert.doesNotMatch(spent, /nothing earlier/);
});

test("non-git browse also omits the terminal footer", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null, older: null }}
    />,
  );
  assert.doesNotMatch(html, /sync-history-rail-floor|nothing earlier/);
});
