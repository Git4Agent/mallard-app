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
import { createActivityRefresh, createSingleFlight } from "../../src/components/project-sync/api";
import { dismissStorageWarning, clearStorageWarningDismissal } from "../../src/preferences";
import type {
  ConversationReviewAction,
  ProjectActivityRefreshEvent,
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
    branches: [{ name: "main", is_current: true }],
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

test("the branch picker lists local branches without using the native select menu", () => {
  const html = renderToStaticMarkup(
    <BranchPicker
      branches={[
        { name: "main", is_current: true },
        { name: "release/0.1", is_current: false },
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
  assert.doesNotMatch(html, />History only</);
  assert.doesNotMatch(html, /<select/);
});

test("switching the Git rail does not clear Push or Pull selection", () => {
  const source = readFileSync(
    "src/components/project-sync/ProjectActivityPage.tsx",
    "utf8",
  );
  const changeBranch = source.match(
    /const changeBranch = useCallback\(\(nextBranch: string\) => \{([\s\S]*?)\n  \}, \[\]\);/,
  )?.[1] ?? "";

  assert.match(changeBranch, /setBranch\(nextBranch\)/);
  assert.match(changeBranch, /setPages\(\[\]\)/);
  assert.match(changeBranch, /load\(null, nextBranch\)/);
  assert.doesNotMatch(changeBranch, /onDropSelection|setComparison|loadComparison/);
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
  // The switch is also a control on the page, not only a crumb menu: the
  // app's segmented pill as a tablist, arrows moving between its two tabs.
  assert.match(workspace, /className="sync-segmented sync-project-view-tabs" role="tablist" aria-label="Project view"/);
  assert.match(workspace, /role="tab"\s*aria-selected=\{active\}\s*aria-controls="project-history-panel"\s*tabIndex=\{active \? 0 : -1\}/);
  assert.match(workspace, /event\.key === "ArrowRight" \|\| event\.key === "ArrowDown"/);
  assert.match(workspace, /role=\{activeProject\.is_git_repository \? undefined : "tabpanel"\}/);
  // A popover that closes on outside focus must move focus in when it opens,
  // or Tab closes it before reaching a Copy button. All three share the recipe.
  for (const file of [
    "src/components/storage-catalog/StorageCatalogPage.tsx",
    "src/components/project-sync/ProjectFileDetailsMenu.tsx",
    "src/components/project-sync/ProjectActivityPage.tsx",
  ]) {
    const source = readFileSync(file, "utf8");
    assert.match(source, /focusedForOpen\.current = true;[\s\S]*?panelRef\.current\?\.querySelector<HTMLElement>\("button"\)\?\.focus\(\);/, file);
  }
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
      glyph: "M6 6h4v4h-4ZM14 6h4v4h-4ZM7 10h2v1h-2ZM15 10h2v1h-2ZM7 11h3v1h-3ZM14 11h3v1h-3ZM8 12h8v1h-8ZM9 13h6v1h-6ZM11 14h2v2h-2ZM10 16h4v3h-4Z",
      filled: true,
    },
    pending: {
      light: "#725f48",
      dark: "#c8b998",
      glyph: "M11 7h2v5h4v2h-6Z",
    },
    conflict: {
      light: "#b53420",
      dark: "#f4917f",
      glyph: "M11 6h3v9h-3ZM11 16h3v3h-3Z",
      filled: true,
    },
    unavailable: {
      light: "#725f48",
      dark: "#c8b998",
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
  // The bug this guards: the badge was conditional, and the empty branch-unmapped
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
      subagent_selection: { wire_children: "on", keys: [], omitted: true, estimated_bytes: 0 },
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
  assert.doesNotMatch(browseHtml, /Child rollouts excluded/);
  assert.match(reviewHtml, /tabindex="0" aria-label="Child rollouts excluded" aria-describedby="[^"]+"/);
  assert.match(reviewHtml, /role="tooltip"/);
  assert.match(reviewHtml, /Child conversations are excluded from this Push\. Pull restores the children stored in the cloud\./);
  const pullHtml = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={{ ...activity, git: null }} comparison={comparison} selectionMode="pull" />,
  );
  assert.doesNotMatch(pullHtml, /Child rollouts excluded/);
});

test("pull selection rows wear the operation chips in the files grammar", () => {
  const chipEntry = (overrides: Record<string, unknown>) => ({
    provider: "codex", provider_display_name: "Codex", access: "bound_ready",
    thread_id: "test", conversation_resource_id: "codex:conversation:test", resource_id: "codex:conversation:test",
    display_name: "Test", local_present: true, storage_present: true,
    push_decision: reviewDecision("no_changes"),
    state: "storage_ahead",
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
      subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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

test("Pull review includes missing-child recovery and excludes unrelated unavailable sessions", () => {
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
    { key: "recovery", title: "Missing child recovery", state: "unknown", source: "local_only", pullAction: "keep_both", visible: true },
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
  assert.doesNotMatch(html, /data-thread-status="(?:synced|local-outstanding|pending)"/);
  assert.equal((html.match(/data-thread-status="conflict"/g) ?? []).length, 1);
  const withoutActivity = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, sessions: [], git: null, unmapped: [] }}
      comparison={comparison}
      selectionMode="pull"
      selectableResourceIds={new Set(sessionRows.map((session) => session.resource_id))}
    />,
  );
  assert.match(withoutActivity, /Include Missing child recovery/);
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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

  // A full-size button sinks two pixels, the depth of its shadow, while held.
  // On the timeline's 22-26px icon buttons that reads as the icon jogging
  // against its neighbours on every click, so they keep the colour feedback
  // and drop the displacement. Full-size buttons still get the press.
  assert.match(css, /\.btn:active:not\(:disabled\)\s*\{[^}]*transform: translate\(2px, 2px\);/s);
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

/* The Storage card and branch-unmapped card are the same object drawn in two
   subtrees, so their frames must stay aligned. */
test("the storage card wears the branch-unmapped card's frame", () => {
  const css = readFileSync("src/App.css", "utf8");
  const declaration = (rule: string, property: string) =>
    rule.match(new RegExp(`[^-]${property}:\\s*([^;]+);`))?.[1];

  const cardRule = css.match(/\.project-storage-card\s*\{[^}]+\}/s)?.[0] ?? "";
  const branchUnmappedRule = css.match(/\.sync-history-branch-unmapped\s*\{[^}]+\}/s)?.[0] ?? "";
  for (const property of ["border", "border-radius", "background"]) {
    assert.equal(
      declaration(cardRule, property),
      declaration(branchUnmappedRule, property),
      `storage card ${property} must match the branch-unmapped card's`,
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
  const branchUnmappedBarRule = css.match(/\.sync-history-branch-unmapped \.sync-history-section-heading\s*\{[^}]+\}/s)?.[0] ?? "";
  for (const property of ["min-height", "padding", "border-bottom"]) {
    assert.equal(
      declaration(barRule, property),
      declaration(branchUnmappedBarRule, property),
      `storage heading bar ${property} must match the branch-unmapped bar's`,
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
  assert.match(html, /aria-label="Show session info"/);
  assert.match(html, /aria-haspopup="dialog"/);
  assert.match(html, /aria-label="Session info"/);
  // Each row is named from its own content — the label, then the value — so a
  // screen reader hears the path it is about to copy. An aria-label would
  // replace all of it with the verb, which is the one part already visible in
  // the row's own `Copy`. The mouse tooltip keeps the verb; the last assertion
  // is what stops an aria-label being helpfully added back.
  assert.match(html, /title="Copy resource id"/);
  assert.match(html, /title="Copy mallard thread id"/);
  assert.match(html, /title="Copy native session id"/);
  assert.match(html, /title="Copy transcript file"/);
  assert.match(html, /title="Copy working directory"/);
  assert.doesNotMatch(html, /title="Copy codex home"/);
  assert.doesNotMatch(html, /class="sync-history-debug-copy"[^>]*aria-label=/);
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
    assert.doesNotMatch(cluster, /Show session info|View session details/);
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
      row.indexOf('aria-label="Show session info"'),
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

test("a cleared-storage dismissal survives reopening only for the same project, storage, and condition", () => {
  const values = new Map<string, string>();
  let refuse = false;
  const scope = globalThis as { window?: unknown };
  const originalWindow = scope.window;
  scope.window = { localStorage: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      if (refuse) throw new Error("Storage unavailable");
      values.set(key, value);
    },
    removeItem: (key: string) => values.delete(key),
  } };
  const comparison = emptyComparison({
    remote_continuity: { state: "remote_missing", base_revision: 2 },
  });
  const render = (currentProject = project, currentComparison = comparison) => renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} project={currentProject} activity={activity} comparison={currentComparison} />,
  );
  const condition = JSON.stringify([
    project.local_project_id, comparison.storage_id, comparison.remote_continuity,
    "Another computer cleared Local storage 1.",
  ]);
  try {
    assert.match(render(), /Another computer cleared/);
    assert.equal(dismissStorageWarning(project.local_project_id, comparison.storage_id, condition), true);
    // Each render mounts a fresh component, as reopening does.
    assert.doesNotMatch(render(), /Another computer cleared/);
    assert.match(render({ ...project, local_project_id: "another-project" }), /Another computer cleared/);
    assert.match(render(project, { ...comparison, storage_id: "another-storage" }), /Another computer cleared/);
    assert.match(render(project, {
      ...comparison, remote_continuity: { state: "remote_missing", base_revision: 3 },
    }), /Another computer cleared/);
    clearStorageWarningDismissal(project.local_project_id, comparison.storage_id);
    assert.match(render(), /Another computer cleared/);
    refuse = true;
    assert.equal(dismissStorageWarning(project.local_project_id, comparison.storage_id, condition), false);
    assert.match(render(), /Another computer cleared/);
  } finally {
    if (originalWindow === undefined) delete scope.window;
    else scope.window = originalWindow;
  }
});

test("a storage another computer cleared shows one standing note with no unlink and no reset", () => {
  const writable = (scope: string, continuity: ThreadSyncComparison["scopes"][number]["continuity"]) => (
    { scope, access: "bound_ready" as const, continuity }
  );
  const render = (comparison: ThreadSyncComparison, selectionMode?: "push" | "pull") => renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={activity}
      comparison={comparison}
      selectionMode={selectionMode}
    />,
  );

  // Story 1: nobody pushed since the clear, so every area this computer
  // writes is a first publication target and Push is open.
  const cleared = emptyComparison({
    base_revision: 2,
    remote_continuity: { state: "remote_missing", base_revision: 2 },
    scopes: [writable("project", { state: "fresh" }), writable("provider:codex", { state: "fresh" })],
  });
  const openHtml = render(cleared);
  assert.match(openHtml, /Another computer cleared Local storage 1\./);
  assert.doesNotMatch(openHtml, /Push again\./);
  assert.match(openHtml, /aria-label="Dismiss: Warning\. Another computer cleared Local storage 1\./);
  // Standing content in the warning tone: a named region, never an alert.
  assert.match(openHtml, /class="notice warning[^"]*" role="region" aria-label="Notes about this project activity"/);
  // The Push review shows the same note above its list.
  assert.match(render(cleared, "push"), /Another computer cleared Local storage 1\./);

  // Story 2: another computer pushed into an area this computer writes, so a
  // Pull comes first. An area this computer only views never decides that.
  const recreated = emptyComparison({
    revision: 1,
    base_revision: 2,
    remote_continuity: { state: "replaced", base_revision: 2, remote_revision: 1 },
    scopes: [
      writable("project", { state: "fresh" }),
      writable("provider:codex", { state: "unreviewed", remote_revision: 1 }),
    ],
  });
  assert.match(render(recreated), /Another computer cleared Local storage 1\. Pull, then push again\./);
  const elsewhere = emptyComparison({
    revision: 1,
    base_revision: 2,
    remote_continuity: { state: "replaced", base_revision: 2, remote_revision: 1 },
    scopes: [
      writable("project", { state: "fresh" }),
      writable("provider:codex", { state: "in_sync", revision: 1 }),
      { scope: "provider:claude", provider: "claude", access: "view_only", continuity: { state: "unreviewed", remote_revision: 1 } },
    ],
  });
  assert.match(render(elsewhere), /Another computer cleared Local storage 1\./);

  // The Pull review is an ordinary first Pull: no note.
  assert.doesNotMatch(render(recreated, "pull"), /Another computer cleared/);
  // A storage whose history this computer is on has nothing to say.
  assert.doesNotMatch(render(emptyComparison()), /Another computer cleared/);

  // R3 and Q4: another computer's push displaced the publication this
  // computer last pushed, or last pulled. Nothing is rolled back; the note
  // says which, and asks for a comparison. The area that holds the displaced
  // content reads as never reviewed here, so a Pull comes first.
  const displaced = (saved_by: "push" | "pull") => emptyComparison({
    revision: 3,
    base_revision: 3,
    remote_continuity: { state: "displaced", base_revision: 3, remote_revision: 3, saved_by },
    scopes: [
      writable("project", { state: "in_sync", revision: 3 }),
      writable("provider:codex", { state: "unreviewed", remote_revision: 3 }),
    ],
  });
  const displacedPush = render(displaced("push"));
  assert.match(
    displacedPush,
    /Another computer pushed to Local storage 1 first\. This computer&#x27;s last push is no longer part of its history\. Compare again\./,
  );
  assert.match(
    render(displaced("pull")),
    /Another computer pushed to Local storage 1 first\. This computer&#x27;s last pull is no longer part of its history\. Compare again\./,
  );
  assert.match(displacedPush, /class="notice warning[^"]*" role="region"/);
  assert.match(render(displaced("push"), "push"), /Another computer pushed to Local storage 1 first/);
  assert.doesNotMatch(render(displaced("push"), "pull"), /Another computer pushed/);
  assert.doesNotMatch(displacedPush, /Another computer cleared|Push again\./);

  // No state asks for an unlink or a reset, and the note has no action.
  for (const html of [openHtml, render(recreated), displacedPush]) {
    assert.doesNotMatch(html, /Unlink storage|Storage history changed|Reset local sync|unlink/i);
    assert.doesNotMatch(html, /Another computer (cleared|pushed)[^<]*<\/p>\s*<button/);
  }
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
  const activityWithBranchUnmapped = {
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
      activity={activityWithBranchUnmapped}
    />,
  );

  assert.match(html, /sync-history-embedded/);
  // The workspace path bar's section crumb is the visible name for this page,
  // so the activity region drops a repeated title and keeps its accessible name.
  assert.doesNotMatch(html, /sync-history-embedded-title/);
  assert.match(html, /aria-label="Git history"/);
  assert.match(html, /class="sync-history-timeline has-branch-unmapped"/);
  assert.match(html, /<h2 id="branch-unmapped-heading"><svg[^>]*data-icon="edit"[^>]*>.*<\/svg>Uncommitted<\/h2>/s);
  // Local-only unmapped rows never conjure the storage-only group.
  assert.doesNotMatch(html, /storage-only-heading/);
  // The unmapped reason is bookkeeping, not something the reader needs on screen.
  assert.doesNotMatch(html, /No subsequent commit/);
  // The empty placeholder is gone. It existed to reserve the 18px box while
  // the status badge could be absent; the badge is never absent now, and a
  // second glyph beside it pushed these rows' titles 26px right of every other
  // row's — measured, not guessed. The group heading still carries the pencil.
  assert.doesNotMatch(html, /sync-history-thread-kind/);
  const branchUnmappedRow = html.slice(html.indexOf('id="branch-unmapped-list"'));
  assert.match(branchUnmappedRow, /class="sync-thread-sync-indicator untracked"/);
  assert.match(html, /<strong title="Test conversation — fork · 2026-08-01 09:00Z">Test conversation — fork<\/strong>/);
  // The count badge is gone from the heading; the fold control carries the
  // count so the group is still announced with its size.
  assert.match(html, /aria-label="Collapse 1 uncommitted session"/);
  assert.match(html, /class="sync-history-spine-fold"[^>]*aria-label="Collapse 1 uncommitted session"[^>]*aria-expanded="true"[^>]*aria-controls="branch-unmapped-list"/);
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

test("storage-only sessions leave branch-unmapped rows in their own group above the rail", () => {
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
  // Each list holds only its own rows; the local session stays in the
  // branch-unmapped group and its fold count follows the split.
  const storageList = html.slice(html.indexOf('id="storage-only-list"'), html.indexOf('id="branch-unmapped-list"'));
  assert.match(storageList, /Add horror story document/);
  assert.doesNotMatch(storageList, /Add Git history mapping/);
  const branchUnmappedList = html.slice(html.indexOf('id="branch-unmapped-list"'));
  assert.match(branchUnmappedList, /Add Git history mapping/);
  assert.doesNotMatch(branchUnmappedList, /Add horror story document/);
  assert.match(html, /aria-label="Collapse 1 uncommitted session"/);
});

test("a page whose unmapped rows are all storage-only renders no branch-unmapped group", () => {
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
  // No local unmapped rows: no branch-unmapped frame, and the timeline drops
  // the padding that exists only to clear it.
  assert.doesNotMatch(html, /branch-unmapped-heading|has-branch-unmapped/);
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
      subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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

test("Push review hides sessions blocked by provider discovery", () => {
  const reason = "The provider could not verify this session for sync.";
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
        subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
  assert.match(html, /aria-label="Open in Claude"><svg[^>]*data-icon="claude-mark"/);
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
  assert.doesNotMatch(html, /title="Copy transcript file"/);
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
          subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
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
        storageProviders={[]}
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
  // From inside the opening tag, so the nav's own label is not one of the tabs.
  const stripStart = html.indexOf('<nav class="sidebar-root-tabs"');
  const strip = html.slice(html.indexOf(">", stripStart) + 1, html.indexOf("</nav>"));
  const tabs = [...strip.matchAll(/aria-label="([^"]+)"/g)].map((match) => match[1]);
  // Each root is named once, here, and nowhere else in the sidebar: Catalog is
  // a destination like Projects and Storages, Settings sits after them, and
  // Logs stays last because it toggles a drawer instead of navigating.
  assert.deepEqual(tabs, ["Projects", "Catalog", "Storages", "Settings", "Logs"]);
  // Icon-only, so the open destination is stated and not only drawn.
  assert.match(strip, /aria-current="page" aria-label="Projects"/);
  assert.equal((strip.match(/aria-current="page"/g) ?? []).length, 1);
  assert.doesNotMatch(strip, /> Projects<|> Storages<|> Catalog</);

  // One rail below it, holding the root the workspace is standing in — with
  // no page open that is Projects — and its count.
  const rail = html.slice(
    html.indexOf('<section class="sync-sidebar-rail"'),
    html.indexOf("</section>") + "</section>".length,
  );
  assert.match(rail, /aria-label="Projects"/);
  // One draft and two projects: the count is the rows, drafts included.
  assert.match(rail, /class="sidebar-section-count">3</);
  assert.match(rail, /<span>Mallard local<\/span>/);
  assert.doesNotMatch(rail, /<span>Local storage 1<\/span>/);
  // Nothing between the tiles is a handle any more: the head is fixed, the
  // rail takes the rest, and neither is drag-sized.
  assert.doesNotMatch(html, /role="separator" aria-orientation="horizontal"/);
  // The heading is a label now, not a disclosure — aria-expanded belongs to
  // the Logs tab alone, which owns a drawer.
  assert.doesNotMatch(html, /sidebar-section-divider|sidebar-section-toggle/);
  assert.equal((html.match(/aria-expanded=/g) ?? []).length, 1);
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
        storageProviders={[]}
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

test("the Catalog rail starts empty and lists one row per added scope", () => {
  // Rows in the backend's shape: a registered capability — the coordinate,
  // its labels, its presentation hint — and the scope at that coordinate
  // joined to it, the way the shell joins them.
  const shapes = {
    Skills: { resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1, icon: "skill", kind_label: "Skills", singular_label: "Skill", format_label: "Agent Skill" },
    "Pets v1": { resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 1, icon: "pet", kind_label: "Pets", singular_label: "Pet v1", format_label: "Codex pet v1" },
    "Pets v2": { resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2, icon: "pet-v2", kind_label: "Pets", singular_label: "Pet v2", format_label: "Codex pet v2" },
  } as const;
  const scopeView = (
    scopeId: string,
    label: keyof typeof shapes,
    displayName: string,
  ): ComponentProps<typeof ProjectSidebar>["catalogScopes"][number] => {
    const shape = shapes[label];
    const capability = {
      provider: "codex",
      provider_label: "Codex",
      resource_kind: shape.resource_kind,
      format: shape.format,
      format_version: shape.format_version,
      label,
      singular_label: shape.singular_label,
      presentation: { icon: shape.icon, kind_label: shape.kind_label, format_label: shape.format_label },
      capture: "directory_tree" as const,
      apply: "automatic" as const,
      targets: [],
    };
    return {
      scope: {
        scope_id: scopeId,
        provider: capability.provider,
        profile_id: `catprofile-${displayName}`,
        resource_kind: capability.resource_kind,
        format: capability.format,
        format_version: capability.format_version,
        revision: 1,
        created_at: 1,
        updated_at: 1,
      },
      profile: {
        profile_id: `catprofile-${displayName}`,
        provider: "codex",
        provider_label: "Codex",
        display_name: displayName,
        configured_home: `/Users/user/${displayName}`,
        revision: 1,
        created_at: 1,
        updated_at: 1,
      },
      provider_label: capability.provider_label,
      kind_label: capability.presentation.kind_label,
      format_label: capability.presentation.format_label,
      label,
      available: true,
      capability,
    };
  };
  const render = (
    scopes: ReturnType<typeof scopeView>[],
    activeScopeId: string | null,
    capabilities: NonNullable<ReturnType<typeof scopeView>["capability"]>[] = [],
  ) =>
    renderToStaticMarkup(
      <ProjectSidebar
        projects={[project]}
        drafts={[]}
        activeDraftId={null}
        storages={[]}
        storageUsage={{}}
        storageDrafts={[]}
        storageProviders={[]}
        onOpenStorageDraft={() => undefined}
        onDiscardStorageDraft={() => undefined}
        activeProjectId={null}
        activeStorageId={null}
        catalogScopes={scopes}
        catalogCapabilities={capabilities}
        activeCatalogScopeId={activeScopeId}
        addingCatalogScope={false}
        /* The rail shows the root the workspace is standing in: a selected
           row puts Catalog there, and with none selected the index does. */
        catalogIndexOpen={activeScopeId === null}
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
  // The rail, not the strip: the Catalog tab carries the same name.
  const catalogPane = (html: string) => {
    const start = html.indexOf('<section class="sync-sidebar-rail" aria-label="Catalog"');
    return html.slice(start, html.indexOf("</section>", start));
  };

  // Fresh state: no permanent Skills/Plugins/Pets rows. One dashed tile per
  // shape the registry offers — glyph and plus, no words — each opening Add
  // with that Kind already chosen; the heading keeps the bare verb.
  const registered = (label: Parameters<typeof scopeView>[1]) => {
    const { capability } = scopeView("catscope-registry", label, "Codex home");
    assert.ok(capability);
    return capability;
  };
  const emptyHtml = render([], null, [registered("Skills"), registered("Pets v1")]);
  const emptyPane = catalogPane(emptyHtml);
  assert.match(emptyPane, /aria-label="Add Catalog resource"/);
  const tiles = [...emptyPane.matchAll(/class="sidebar-door" title="([^"]+)"/g)].map((match) => match[1]);
  assert.deepEqual(tiles, ["Add Skills", "Add Pets v1"]);
  assert.match(emptyPane, /aria-label="Add Skills"[^]*?data-icon="lightbulb"[^]*?data-icon="plus"/);
  assert.match(emptyPane, /aria-label="Add Pets v1"[^]*?data-icon="paw"/);
  assert.doesNotMatch(emptyPane, /Add your first|sync-sidebar-empty|<span>Skills<\/span>|<span>Plugins<\/span>|<span>Pets<\/span>/);
  // Before the registry answers, the one tile is the chooser itself.
  assert.match(catalogPane(render([], null)), /class="sidebar-door" title="Add resource"[^]*?data-icon="catalog"/);

  // Two Codex configs give the same kind two distinct rows, labelled by
  // config name; pet v1 and v2 from one config are two separate rows.
  const html = render([
    scopeView("catscope-1", "Skills", "Codex home"),
    scopeView("catscope-2", "Skills", "conf2"),
    scopeView("catscope-3", "Pets v1", "Codex home"),
    scopeView("catscope-4", "Pets v2", "Codex home"),
  ], "catscope-2");
  const pane = catalogPane(html);
  // Each row wears the glyph its capability's presentation hint names.
  assert.match(pane, /data-icon="lightbulb"[^]*?Skills · Codex home/);
  assert.match(pane, /data-icon="paw"[^]*?Pets v1 · Codex home/);
  assert.match(pane, /data-icon="cat"[^]*?Pets v2 · Codex home/);
  assert.match(pane, /<span>Skills · Codex home<\/span>/);
  assert.match(pane, /<span>Skills · conf2<\/span>/);
  assert.match(pane, /<span>Pets v1 · Codex home<\/span>/);
  assert.match(pane, /<span>Pets v2 · Codex home<\/span>/);
  assert.match(pane, /sidebar-profile-item active[^]*?Skills · conf2/);
  assert.match(pane, /aria-label="Remove the Skills · conf2 row"/);
  assert.doesNotMatch(pane, /sidebar-door/);
});

test("an empty Storages rail prompts the way Projects and Catalog do", () => {
  const providers = [
    { key: "local", config_version: 1, display_name: "Local folder", description: "A folder.", icon: "drive", fields: [] },
    { key: "cloud-a", config_version: 1, display_name: "Cloud A", description: "A bucket.", icon: "cloud", fields: [] },
    // A second config version of a provider is the same door.
    { key: "cloud-a", config_version: 2, display_name: "Cloud A", description: "A bucket.", icon: "cloud", fields: [] },
  ] as unknown as ComponentProps<typeof ProjectSidebar>["storageProviders"];
  const render = (
    storages: ComponentProps<typeof ProjectSidebar>["storages"],
    storageProviders: ComponentProps<typeof ProjectSidebar>["storageProviders"] = providers,
  ) =>
    renderToStaticMarkup(
      <ProjectSidebar
        projects={[project]}
        drafts={[]}
        activeDraftId={null}
        storages={storages}
        storageUsage={{}}
        storageDrafts={[]}
        storageProviders={storageProviders}
        onOpenStorageDraft={() => undefined}
        onDiscardStorageDraft={() => undefined}
        activeProjectId={null}
        activeStorageId={null}
        catalogScopes={[]}
        activeCatalogScopeId={null}
        addingCatalogScope={false}
        catalogIndexOpen={false}
        /* Storages owns the rail while its index is the page. */
        storageIndexOpen={true}
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

  // The rail, not the strip: the Storages tab carries the same name.
  const storagePane = (html: string) => {
    const start = html.indexOf('<section class="sync-sidebar-rail" aria-label="Storages"');
    return html.slice(start, html.indexOf("</section>", start));
  };

  // All three roots say the same thing when they are empty: dashed tiles,
  // one per way Add can start — here one per provider the build ships.
  const empty = storagePane(render([]));
  const tiles = [...empty.matchAll(/class="sidebar-door" title="([^"]+)"/g)].map((match) => match[1]);
  assert.deepEqual(tiles, ["Add Local folder storage", "Add Cloud A storage"]);
  assert.match(empty, /aria-label="Add Local folder storage"[^]*?data-icon="storage-local"[^]*?data-icon="plus"/);
  assert.match(empty, /aria-label="Add Cloud A storage"[^]*?data-icon="cloud"/);
  assert.doesNotMatch(empty, /Add your first|sync-sidebar-empty/);
  // Before the descriptors arrive, the one tile is the picker itself.
  assert.match(storagePane(render([], [])), /class="sidebar-door" title="Add storage"[^]*?data-icon="storage"/);

  // And it is a first-run prompt, not a permanent Add row.
  const settled = storagePane(render([{
    storage_id: "storage-1",
    name: "Local storage 1",
    provider: "local",
    config_version: 1,
    provider_label: "Local folder",
    icon: "drive",
    available: true,
    summary: [{ field_id: "directory", label: "Folder", value: "/tmp/storage", is_path: true }],
    settings: {},
    secrets: [],
  }]));
  assert.match(settled, /<span>Local storage 1<\/span>/);
  assert.doesNotMatch(settled, /sidebar-door/);
});

test("the sidebar locks project and storage navigation during a sync workflow", () => {
  // One rail shows one root, so the lock is read on each of the two it can
  // lock: the project rows on Projects, the storage rows on Storages.
  const render = (storageIndexOpen: boolean) => renderToStaticMarkup(
    <ProjectSidebar
      projects={[project]}
      drafts={[]}
      activeDraftId={null}
      storages={[{
        storage_id: "storage-1",
        name: "Local storage 1",
        provider: "local",
        config_version: 1,
        provider_label: "Local folder",
        icon: "drive",
        available: true,
        summary: [{ field_id: "directory", label: "Folder", value: "/tmp/storage", is_path: true }],
        settings: {},
        secrets: [],
      }]}
      storageUsage={{ "storage-1": 1 }}
      storageDrafts={[]}
        storageProviders={[]}
      onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined}
      storageIssues={{ "storage-1": "Storage folder unavailable." }}
      activeProjectId={project.local_project_id}
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      catalogIndexOpen={false}
      storageIndexOpen={storageIndexOpen}
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

  const html = render(false);
  const storageHtml = render(true);
  const buttonAt = (source: string, marker: number) =>
    source.slice(source.lastIndexOf("<button", marker), source.indexOf(">", source.lastIndexOf("<button", marker)) + 1);

  const projectButton = buttonAt(html, html.indexOf("<span>Mallard local</span>"));
  const storageButton = buttonAt(storageHtml, storageHtml.indexOf("<span>Local storage 1</span>"));
  const addStorageButton = buttonAt(storageHtml, storageHtml.indexOf('aria-label="Add storage"'));
  const removeStorageMarker = storageHtml.indexOf('aria-label="Remove Local storage 1 from Mallard"');
  const removeStorageButton = storageHtml.slice(
    storageHtml.lastIndexOf("<button", removeStorageMarker),
    storageHtml.indexOf("</button>", removeStorageMarker) + "</button>".length,
  );

  assert.match(projectButton, /disabled=""/);
  assert.match(storageButton, /disabled=""/);
  assert.match(addStorageButton, /disabled=""/);
  assert.match(removeStorageButton, /class="sidebar-profile-remove"/);
  assert.match(removeStorageButton, /disabled=""/);
  assert.match(removeStorageButton, /data-icon="trash"/);
  assert.doesNotMatch(html, /data-icon="more"/);
  assert.match(storageHtml, /sync-sidebar-storage-warning/);
  assert.match(storageButton, /Storage issue: Storage folder unavailable\./);
  // Busy locks the strip as well: no destination can be left mid-workflow.
  // Logs is the exception — a drawer over the page, not a way off it.
  assert.equal((html.match(/class="sidebar-root-tab[^"]*"[^>]*disabled=""/g) ?? []).length, 4);
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

});

test("a project with no repository gets the same control, minus the rail", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={{ ...activity, git: null }} />,
  );
  assert.match(html, /class="sync-history-rail-foot no-rail"/);
  assert.match(html, /<strong>Load earlier<\/strong>/);
});

test("a review shows the whole span at once: no paging, no fold, unlisted entries as rows", () => {
  // A review loads every outstanding session in one request, so it never
  // pages and never needs a catch-all box. What the comparison knows and the
  // activity scan does not is offered as rows of the same list — dropping it
  // silently would drop candidates.
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
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    push_decision: { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
  };
  const comparison = emptyComparison({ entries: [entry] });

  // Even with `older` reporting more history, a review draws no foot: it
  // asked for the whole span, so the rail simply ends.
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults} activity={activity} selectionMode="push" comparison={comparison} />,
  );
  assert.doesNotMatch(html, /sync-history-rail-step/);
  assert.doesNotMatch(html, /Load earlier/);
  // The remainder is rows of the same list, not a box.
  assert.doesNotMatch(html, /Older activity|sync-history-older/);
  assert.match(html, /aria-label="Sessions without an activity row"/);
  assert.match(html, /A thread no session row reaches/);
  assert.doesNotMatch(html, /nothing earlier/);
  const changed = { ...entry, state: "synced" as const, storage_present: true,
    status_detail: "Export settings changed", push_decision: { action: "publish" as const, selectable: true } };
  const modeChange = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} selectionMode="push"
      comparison={emptyComparison({ entries: [changed] })} selectableResourceIds={new Set([entry.resource_id])} />,
  );
  assert.match(modeChange, /A thread no session row reaches/);
  assert.match(modeChange, />Export settings changed<\/span>/);
  assert.match(modeChange, /type="checkbox"/);
  // A current session exported under earlier settings is Push work, not a
  // synced check: it carries the local glyph and joins the Push count.
  const changedPresentation = sessionSyncPresentation(changed, "Storage");
  assert.equal(changedPresentation.status, "local-outstanding");
  assert.equal(changedPresentation.className, "local");
  assert.match(changedPresentation.label, /Pushed with your previous export settings\. Push to update Storage\./);
  assert.equal(sessionSyncPresentation({ ...changed, push_decision: { action: "no_changes" as const, selectable: false } }, "Storage").status, "synced");
  // The standing page still pages.
  const standing = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} />,
  );
  assert.match(standing, /<strong>Load earlier<\/strong>/);
});

test("a Push candidate missing from the selected branch projection remains visible", () => {
  const nativeId = "claude-off-rail";
  const sessionResourceId = `claude:conversation:${nativeId}`;
  const reviewResourceId = `claude:session:${nativeId}`;
  const claudeSession = {
    ...codexSession,
    resource_id: sessionResourceId,
    provider: "claude",
    provider_display_name: "Claude",
    provider_session_id: nativeId,
    logical_thread_id: nativeId,
    transcript_path: CLAUDE_TRANSCRIPT_PATH,
    title: "Claude session absent from this commit rail",
    actions: [],
  };
  const entry: ThreadSyncEntry = {
    provider: "claude",
    provider_display_name: "Claude",
    access: "bound_ready",
    thread_id: nativeId,
    conversation_resource_id: sessionResourceId,
    resource_id: reviewResourceId,
    display_name: claudeSession.title,
    state: "local_only",
    local_present: true,
    storage_present: false,
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: nativeId,
    push_decision: { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{
        ...activity,
        sessions: [claudeSession],
        git: {
          ...activity.git,
          commits: [{ ...activity.git.commits[0], session_refs: [] }],
          unique_session_count: 0,
        },
        // A defensive Push/Pull invariant: even if an Activity projection is
        // incomplete, the comparison entry must not disappear with it.
        unmapped: [],
      }}
      comparison={emptyComparison({ entries: [entry] })}
      activeStorageName="Local storage 1"
      selectionMode="push"
      selectedResourceIds={new Set([reviewResourceId])}
      selectableResourceIds={new Set([reviewResourceId])}
    />,
  );

  assert.match(html, /aria-label="Sessions without an activity row"/);
  assert.match(html, /<strong>Claude session absent from this commit rail<\/strong>/);
  assert.match(html, /aria-label="Claude thread"/);
  assert.match(html, /aria-label="Exclude Claude session absent from this commit rail, Claude thread claude-off-rail"/);
});

test("an empty flat list names its window only when paging could fill it", () => {
  // The rail's rule, applied to the list a non-git project draws instead: a
  // project whose homes hold no session at all has no "Load earlier" step,
  // so its sentence must not blame a 30-day window it cannot page past.
  const nothingEver = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null, sessions: [], older: null }}
    />,
  );
  assert.match(nothingEver, /class="sync-history-state">No sessions yet\.</);
  assert.doesNotMatch(nothingEver, /No sessions in |sync-history-rail-step/);

  const olderCommitsOnly = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null, sessions: [], older: { newest_at: 1_749_000_000, commits: 3, sessions: 0 } }}
    />,
  );
  assert.match(olderCommitsOnly, /No sessions yet\./);

  const emptyWindow = renderToStaticMarkup(
    <ProjectChatHistoryContent
      {...contentDefaults}
      activity={{ ...activity, git: null, sessions: [] }}
    />,
  );
  assert.match(emptyWindow, /No sessions in [^<]+\./);
  assert.doesNotMatch(emptyWindow, /No sessions yet/);
  assert.match(emptyWindow, /<strong>Load earlier<\/strong>/);
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

test("the empty sentence in a flat list keeps all four inner corners", () => {
  // The frame's only child: its generic 5px radius painted the hatched fill
  // square over the frame's 12px inner curve, and the top-left, top-right and
  // bottom-left corners read as missing — the bottom-right hid under the drop
  // shadow (2026-09-05). Source gate, as for the lone card below.
  const css = readFileSync("src/App.css", "utf8");
  const rule = css.match(/\.sync-history-thread-list\.flat > \.sync-history-state \{([^}]*)\}/)?.[1];
  assert.ok(rule, "flat empty-state rule exists");
  assert.match(rule, /border-radius: calc\(var\(--retro-panel-radius\) - 2px\)/);
});

test("a lone card in a flat list keeps all four inner corners", () => {
  // `:first-child` and `:last-child` both match a lone card, and each rule
  // owns two corners. They have to be written as longhands: a `border-radius`
  // shorthand in either wipes the other's corners, and a non-git project with
  // one session then paints its hover tint square into the list's rounded
  // top — the "missing corners" of 2026-09-04. A source gate is the only kind
  // a node test can hold here; the cascade itself was measured in a probe.
  const css = readFileSync("src/App.css", "utf8");
  const rule = (pseudo: string) => css.match(
    new RegExp(`\\.sync-history-thread-list\\.flat > \\.sync-history-thread-card:${pseudo} \\{([^}]*)\\}`),
  )?.[1];
  const first = rule("first-child");
  const last = rule("last-child");
  assert.ok(first && last, "both corner rules exist");
  assert.doesNotMatch(first, /\bborder-radius:/, "first-child sets its two corners as longhands");
  assert.doesNotMatch(last, /\bborder-radius:/, "last-child sets its two corners as longhands");
  const inner = String.raw`calc\(var\(--retro-panel-radius\) - 2px\);`;
  assert.match(first, new RegExp(`border-top-left-radius: ${inner}`));
  assert.match(first, new RegExp(`border-top-right-radius: ${inner}`));
  assert.match(last, new RegExp(`border-bottom-left-radius: ${inner}`));
  assert.match(last, new RegExp(`border-bottom-right-radius: ${inner}`));
});


test("activity refresh joins in-flight requests, replays rows, and clears completed requests", async () => {
  let starts = 0;
  let emit!: (event: ProjectActivityRefreshEvent) => void;
  let finish!: () => void;
  const refresh = createActivityRefresh(async (_request, onEvent) => {
    starts += 1;
    emit = onEvent;
    await new Promise<void>((resolve) => { finish = resolve; });
  });
  const request = { localProjectId: "project-a", storageId: "store-a", branch: null, windowDays: 30, review: false, bindingRevision: 1, refreshEpoch: 0 };
  const first: ProjectActivityRefreshEvent[] = [];
  const second: ProjectActivityRefreshEvent[] = [];
  const pending = refresh(request, (event) => first.push(event));
  await Promise.resolve();
  const local: ProjectActivityRefreshEvent = { stage: "activity", activity, storage_pending: true };
  emit(local);
  const joined = refresh(request, (event) => second.push(event));
  assert.equal(joined, pending);
  assert.deepEqual(second, [local]);
  assert.equal(starts, 1);
  emit({ stage: "comparison_error", message: "Storage unavailable" });
  assert.deepEqual(second, first);
  finish();
  await pending;
  const next = refresh(request, () => {});
  await Promise.resolve();
  assert.equal(starts, 2, "a later refresh must make a fresh read");
  finish();
  await next;
});

test("activity refresh isolates project, storage, and branch requests and clears failures", async () => {
  let starts = 0;
  const refresh = createActivityRefresh(async () => { starts += 1; throw new Error("offline"); });
  const request = { localProjectId: "project-a", storageId: "store-a", branch: null, windowDays: 30, review: false, bindingRevision: 1, refreshEpoch: 0 };
  await Promise.all([
    request,
    { ...request, localProjectId: "project-b" },
    { ...request, storageId: "store-b" },
    { ...request, branch: "main" },
    { ...request, review: true },
    { ...request, bindingRevision: 2 },
    { ...request, refreshEpoch: 1 },
  ].map((input) => assert.rejects(refresh(input, () => {}), /offline/)));
  assert.equal(starts, 7);
  await assert.rejects(refresh(request, () => {}), /offline/);
  assert.equal(starts, 8);
});


test("name search matches literal words in any order and ignores case and whitespace", async () => {
  const { nameMatchesSearch } = await import("../../src/components/sync-review/nameSearch");
  for (const query of ["api fix", "  FIX   api  ", "", "   ", "timeout"]) {
    assert.equal(nameMatchesSearch("Fix API timeout", query), true, query);
  }
  assert.equal(nameMatchesSearch("Fix API timeout", "api cache"), false);
  assert.equal(nameMatchesSearch("Fix API timeout", ".*"), false);
  assert.equal(nameMatchesSearch("Fix [API] timeout", "[api]"), true);
  assert.equal(nameMatchesSearch("修复登录超时", "登录 修复"), true);
});

test("session search filters names and empty Git groups, while retaining history paging", () => {
  const render = (query: string) => renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} searchQuery={query} />,
  );
  const matched = render("mapping git");
  assert.match(matched, /Add Git history mapping/);
  assert.match(matched, /Add project-scoped history/);
  assert.match(matched, /Search loaded sessions/);
  for (const query of ["nonexistent", "first-parent", "project-scoped"]) {
    const html = render(query);
    assert.match(html, /No matching loaded sessions/);
    assert.match(html, /Load earlier/);
    assert.doesNotMatch(html, /Add Git history mapping|Add project-scoped history|No commits yet|No sessions yet/);
  }
});

test("loading another history page makes its names searchable", () => {
  const older = { ...codexSession, resource_id: "codex:conversation:older", title: "Earlier API repair" };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} searchQuery="repair earlier" activity={{
      ...activity, git: null, sessions: [...activity.sessions, older],
    }} />,
  );
  assert.match(html, /Earlier API repair/);
  assert.doesNotMatch(html, /Add Git history mapping/);
  assert.match(html, /Load earlier/);
});

test("review search does not reintroduce a hidden activity row through its comparison name", () => {
  const entry: ThreadSyncEntry = {
    provider: "codex", provider_display_name: "Codex", access: "bound_ready",
    thread_id: CODEX_SESSION_ID, conversation_resource_id: codexResourceId,
    resource_id: codexResourceId, display_name: "Comparison alias",
    state: "local_only", local_present: true, storage_present: false,
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    push_decision: { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
  };
  const orphan = { ...entry, thread_id: "orphan", resource_id: "orphan", conversation_resource_id: "orphan", display_name: "Fix remote API" };
  const props = {
    ...contentDefaults, activity, selectionMode: "push" as const,
    comparison: emptyComparison({ entries: [entry, orphan] }),
    selectedResourceIds: new Set([codexResourceId, "orphan"]),
  };
  const hidden = renderToStaticMarkup(<ProjectChatHistoryContent {...props} searchQuery="alias" />);
  assert.match(hidden, /No matching sessions/);
  assert.match(hidden, /2 selected hidden by search/);
  assert.doesNotMatch(hidden, /Comparison alias|Nothing to push|Add Git history mapping|Fix remote API/);
  const matched = renderToStaticMarkup(<ProjectChatHistoryContent {...props} searchQuery="api fix" />);
  assert.match(matched, /Fix remote API/);
  assert.match(matched, /1 selected hidden by search/);
  assert.match(matched, /aria-label="Sessions without an activity row"/);
  assert.doesNotMatch(matched, /Add Git history mapping/);
  assert.equal(props.selectedResourceIds.size, 2);
});


test("a shared comparison replays recovered Cloud status without its earlier error", async () => {
  let emit!: (event: ProjectActivityRefreshEvent) => void;
  let finish!: () => void;
  const refresh = createActivityRefresh(async (_request, onEvent) => {
    emit = onEvent;
    await new Promise<void>((resolve) => { finish = resolve; });
  });
  const request = { localProjectId: "project-cloud", storageId: "storage-cloud", branch: null, windowDays: 30, review: false, bindingRevision: 1, refreshEpoch: 0 };
  const pending = refresh(request, () => {});
  await Promise.resolve();
  emit({ stage: "comparison_error", message: "Access lost", managed_cloud_status: { repository_id: "repo_fixture", state: "inaccessible", link: null, message: "Access lost" } });
  const recovered = { stage: "comparison", comparison: { project_id: "project-cloud" } } as ProjectActivityRefreshEvent;
  emit(recovered);
  const replayed: ProjectActivityRefreshEvent[] = [];
  assert.equal(refresh(request, (event) => replayed.push(event)), pending);
  assert.deepEqual(replayed, [recovered]);
  finish();
  await pending;
});


test("a refresh after a release discards older events and keeps the newer refresh shared", async () => {
  const runs: Array<{ emit: (event: ProjectActivityRefreshEvent) => void; finish: () => void }> = [];
  const refresh = createActivityRefresh(async (_request, emit) => {
    await new Promise<void>((finish) => { runs.push({ emit, finish }); });
  });
  const request = { localProjectId: "cloud-project", storageId: "cloud-storage", branch: null, windowDays: 30, review: false, bindingRevision: 1, refreshEpoch: 0 };
  const seen: ProjectActivityRefreshEvent[] = [];
  const old = refresh(request, (event) => seen.push(event));
  await Promise.resolve();
  refresh.invalidate(request.localProjectId, request.storageId);
  const fresh = refresh(request, (event) => seen.push(event));
  await Promise.resolve();
  assert.equal(runs.length, 2);
  const stale = { stage: "comparison", comparison: { project_id: "old-rejection" } } as ProjectActivityRefreshEvent;
  const pending = { stage: "comparison", comparison: { project_id: "new-pending" } } as ProjectActivityRefreshEvent;
  runs[0].emit(stale);
  assert.deepEqual(seen, []);
  runs[0].finish();
  await old;
  assert.equal(refresh(request, () => {}), fresh);
  runs[1].emit(pending);
  assert.deepEqual(seen, [pending]);
  runs[1].finish();
  await fresh;
});

test("draft sync labels do not describe unpublished content as local only", () => {
  const synced = { state: "synced", status_detail: "Synced to the Cloud draft.", push_decision: { action: "no_changes", selectable: false } } as Parameters<typeof sessionSyncPresentation>[0];
  assert.equal(sessionSyncPresentation(synced, "Mallard Cloud").status, "synced");
  assert.equal(sessionSyncPresentation(synced, "Mallard Cloud").label, "Synced to the Cloud draft.");
  const missing = { ...synced, state: "local_only", status_detail: "Not in the Cloud draft. Push to save it there." } as Parameters<typeof sessionSyncPresentation>[0];
  assert.match(sessionSyncPresentation(missing, "Mallard Cloud").label, /^Not in the Cloud draft/);
  assert.doesNotMatch(sessionSyncPresentation(missing, "Mallard Cloud").label, /Only on this computer/);
  const settingsChanged = { ...synced, push_decision: { action: "publish", selectable: true } } as Parameters<typeof sessionSyncPresentation>[0];
  assert.equal(sessionSyncPresentation(settingsChanged, "Mallard Cloud").status, "local-outstanding");
});


test("cloud loading is announced without a status panel and does not report a final empty list", () => {
  const empty = { ...activity, sessions: [], unmapped: [], git: { ...activity.git, commits: [] } };
  const pending = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={empty} storageLoading comparisonLoading />,
  );
  assert.match(pending, /role="status"/);
  assert.match(pending, /Loading cloud sessions…/);
  assert.doesNotMatch(pending, /0 items|No commits|No sessions|Load earlier/);

  const withLocalRows = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={activity} storageLoading comparisonLoading />,
  );
  assert.match(withLocalRows, /Loading cloud sessions…/);
  assert.match(withLocalRows, /class="sync-visually-hidden" role="status"/);
  assert.doesNotMatch(withLocalRows, /class="sync-history-state" role="status"/);
  assert.match(withLocalRows, /Add project-scoped history/);

  const failed = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} activity={empty}
      comparisonError="Storage unavailable" />,
  );
  assert.doesNotMatch(failed, /Loading cloud sessions/);
  assert.match(failed, /Storage unavailable/);
});

test("old and recent cloud sessions share full rows in Pull review", () => {
  const sessions = [1, 60, 120].map((days) => ({
    ...codexSession,
    resource_id: `codex:cloud-conversation:cloudthread-${days}`,
    title: `Cloud session ${days}`,
    source: "remote_only" as const,
    provider_session_id: null,
    transcript_path: null,
    started_at: activity.window_end - days * 86_400 - 60,
    ended_at: activity.window_end - days * 86_400,
    actions: [],
  }));
  const entries = sessions.map((session): ThreadSyncEntry => ({
    provider: "codex", provider_display_name: "Codex", access: "bound_ready",
    thread_id: session.resource_id, conversation_resource_id: session.resource_id,
    resource_id: session.resource_id, display_name: session.title,
    state: "storage_only", local_present: false, storage_present: true,
    storage_updated_at: activity.window_end,
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    push_decision: { action: "no_changes", selectable: false },
    pull_decision: { action: "create_local", selectable: true },
  }));
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent {...contentDefaults} selectionMode="pull"
      activity={{ ...activity, sessions, git: { ...activity.git, commits: [] },
        unmapped: sessions.map((session) => ({ resource_id: session.resource_id, reason: "Storage only" })) }}
      comparison={emptyComparison({ entries })} activeStorageName="Fixture storage" />,
  );
  assert.equal((html.match(/id="storage-only-heading"/g) ?? []).length, 1);
  assert.equal((html.match(/aria-label="Session details"/g) ?? []).length, 3);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 3);
  assert.doesNotMatch(html, /Sessions without an activity row|sync-history-stored-thread-meta|Load earlier/);
  for (const session of sessions) {
    assert.ok(html.includes(new Date(session.ended_at * 1000).toISOString()));
  }
});

test("both activity request paths explicitly select review scope", () => {
  const page = readFileSync("src/components/project-sync/ProjectActivityPage.tsx", "utf8");
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.match(page, /getProjectActivity\([\s\S]*?30,\s*selectionMode != null,/);
  assert.match(page, /refreshProjectActivity\(\{[\s\S]*?review: selectionMode != null,/);
  assert.match(api, /invoke<void>\("refresh_project_activity", \{[^}]*review,/);
  assert.doesNotMatch(page, /reviewWindowDays/);
});
