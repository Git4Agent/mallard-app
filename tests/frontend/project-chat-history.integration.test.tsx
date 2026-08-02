import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import Icon from "../../src/components/Icons";
import ProjectChatHistoryPage, {
  BranchPicker,
  ProjectChatHistoryContent,
  ThreadMetrics,
  threadDebugCopyText,
  threadSyncPresentation,
} from "../../src/components/project-sync/ProjectChatHistoryPage";
import ProjectSidebar from "../../src/components/project-sync/ProjectSidebar";
import { pushSelectableResourceIds } from "../../src/components/project-sync/PushResourceWorkspace";
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

const history = {
  project_id: project.local_project_id,
  threads: [{
    thread_id: "019f7798-5437-7632-9dbc-5b589cf68bf0",
    working_directory: "/Users/test/projects/mallard/packages/desktop",
    title: "Add Git history mapping",
    summary: "Map project-owned Codex sessions onto the first-parent rail.",
    started_at: 1_752_800_000,
    ended_at: 1_752_803_600,
    branch: "main",
    recorded_sha: "a".repeat(40),
    user_round_count: 3,
    agent_message_count: 5,
    tool_call_count: 8,
    total_tokens: 24800,
    metrics_complete: true,
    commit_occurrence_count: 2,
  }],
  git: {
    selected_branch: "main",
    branches: [{ name: "main", is_current: true, available: true }],
    github_repository_url: "https://github.com/Git4Agent/mallard",
    commits: [{
      sha: "b".repeat(40),
      short_sha: "bbbbbbb",
      committed_at: 1_752_802_000,
      subject: "Add project-scoped history",
      thread_refs: [{ thread_id: "019f7798-5437-7632-9dbc-5b589cf68bf0" }],
    }],
    unique_thread_count: 1,
    reference_count: 1,
    next_cursor: null,
  },
  unmapped: [],
  warnings: [],
  window_start: 1_750_000_000,
  window_end: 1_753_000_000,
  next_before: 1_750_000_000,
  codex_home: "/Users/test/config/mallard/.codex",
  storage_sync: [{
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    last_pull_at: 1_752_700_000,
    last_push_at: 1_752_800_000,
  }],
};

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
  assert.match(html, />Current</);
  assert.match(html, /Recorded session history only/);
  assert.match(html, />History only</);
  assert.doesNotMatch(html, /<select/);
});

test("thread indicators use the quiet desktop sync vocabulary", () => {
  const presentation = (state: Parameters<typeof threadSyncPresentation>[0]["state"]) => (
    threadSyncPresentation({ state } as Parameters<typeof threadSyncPresentation>[0], "Storage")
  );

  assert.deepEqual(
    ["synced", "local_only", "local_ahead", "storage_only", "storage_ahead", "diverged", "diverged_forked", "unavailable", "unknown"].map((state) => {
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
      { status: "divergent-forked", className: "diverged-forked" },
      { status: "unavailable", className: "unavailable" },
      { status: "conflict", className: "conflict" },
    ],
  );

  const unknown = threadSyncPresentation({
    state: "unknown",
    status_detail: "Conversation ancestry could not be proven safely.",
  } as Parameters<typeof threadSyncPresentation>[0], "Storage");
  assert.match(unknown.label, /ancestry could not be proven safely/i);
  assert.doesNotMatch(unknown.label, /baseline/i);

  const pending = threadSyncPresentation({
    state: "synced",
    status_detail: "A fork checkpoint is pending.",
    push_decision: {
      action: "blocked",
      reason_code: "resolution_pending",
    },
  } as Parameters<typeof threadSyncPresentation>[0], "Storage");
  assert.deepEqual(
    { status: pending.status, className: pending.className },
    { status: "pending", className: "pending" },
  );

  const unavailable = threadSyncPresentation({
    state: "local_only",
    push_decision: {
      action: "blocked",
      reason_code: "codec_unsupported",
    },
  } as Parameters<typeof threadSyncPresentation>[0], "Storage");
  assert.deepEqual(
    { status: unavailable.status, className: unavailable.className },
    { status: "unavailable", className: "unavailable" },
  );

  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-thread-sync-indicator\.synced \{ color: var\(--thread-status-synced\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.local \{ color: var\(--thread-status-local\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.storage \{ color: var\(--thread-status-cloud\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.diverged \{ color: var\(--thread-status-divergent\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.diverged-forked \{ color: var\(--thread-status-forked\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.pending \{ color: var\(--thread-status-pending\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.conflict \{ color: var\(--thread-status-conflict\); \}/);
  assert.match(css, /\.sync-thread-sync-indicator\.unavailable \{ color: var\(--thread-status-unavailable\); \}/);
  assert.match(css, /\.sync-thread-sync-error \{ color: var\(--thread-status-conflict\); \}/);
});

test("thread status badges use the selected solid pixel family", () => {
  const expected: Record<ThreadStatusIconName, { color: string; glyph: string }> = {
    synced: {
      color: "#32b86b",
      glyph: "M5 12H8V15H10V13H12V11H14V9H16V7H19V10H17V12H15V14H13V16H11V18H8V16H7V15H5Z",
    },
    "cloud-outstanding": {
      color: "#55749a",
      glyph: "M11 14H13V17H15L12 20L9 17H11Z",
    },
    "local-outstanding": {
      color: "#20a6c8",
      glyph: "M11 17H13V9H16L12 5L8 9H11Z",
    },
    divergent: {
      color: "#7657cc",
      glyph: "M5 7H14V5L19 8L14 11V9H5Z",
    },
    "divergent-forked": {
      color: "#5548b8",
      glyph: "M5 4H9V8H8V10H16V8H15V4H19V8H18V11H17V12H8V17H9V21H5V17H6V8H5Z",
    },
    pending: {
      color: "#e9a11d",
      glyph: "M11 7H13V12H17V14H11Z",
    },
    conflict: {
      color: "#ea514e",
      glyph: "M10 5H14V14H10ZM10 17H14V21H10Z",
    },
    unavailable: {
      color: "#7f828a",
      glyph: "M6 11H18V14H6Z",
    },
  };
  const statuses = Object.keys(expected) as ThreadStatusIconName[];

  for (const status of statuses) {
    const html = renderToStaticMarkup(<ThreadStatusIcon status={status} size={16} />);
    assert.match(html, new RegExp(`data-thread-status="${status}"`));
    assert.match(html, /thread-status-icon-badge/);
    assert.match(html, /M6 3H18V4H20V6H21V8H22V16H21V18H20V20H18V21H6V20H4V18H3V16H2V8H3V6H4V4H6Z/);
    assert.ok(html.includes(expected[status].glyph));
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /(?:linearGradient|filter|drop-shadow)/);
  }

  const css = readFileSync("src/App.css", "utf8");
  for (const status of statuses) {
    assert.match(css, new RegExp(`\\.thread-status-icon\\[data-thread-status="${status}"\\]`));
  }
  assert.match(css, /--thread-status-glyph: #fffaf1/);
  for (const { color } of Object.values(expected)) {
    assert.ok(css.includes(color));
  }
});

test("refreshing project activity uses the centered pixel refresh icon", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      project={project}
      binding={{
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
      }}
      history={history}
      loading
      loadingMore={false}
      actionError={null}
      actionBusyThreadId={null}
      comparisonLoading
      activeStorageName="R2 storage 1"
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onOpenCodex={() => undefined}
      onOpenTerminal={() => undefined}
    />,
  );

  assert.equal(html.match(/icon-spin-frame spinning/g)?.length, 1);
  assert.doesNotMatch(html, /sync-thread-sync-loading/);
  assert.match(html, /sync-history-refresh loading/);
  assert.match(html, /aria-busy="true"/);
  assert.match(html, /role="status">Comparing threads with R2 storage 1/);
  assert.match(html, /data-icon="refresh"/);

  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-history-icon-action\.loading:disabled\s*\{[^}]*color:\s*var\(--blue\);[^}]*opacity:\s*1;/s);
  assert.match(css, /\.icon-spin-frame\s*\{[^}]*width:\s*16px;[^}]*height:\s*16px;[^}]*transform-origin:\s*8px 8px;/s);
  assert.match(css, /\.icon-spin-frame\.spinning\s*\{[^}]*animation:\s*icon-spin 0\.8s linear infinite;/s);
});

test("sync tooltip hover keeps the sibling icon rail geometry stable", () => {
  const css = readFileSync("src/App.css", "utf8");
  const titleRule = css.match(/\.sync-history-thread-title\s*\{[^}]+\}/s)?.[0] ?? "";
  const tooltipRule = css.match(/\.sync-thread-sync-indicator::after,[\s\S]+?\n\}/)?.[0] ?? "";

  assert.match(titleRule, /align-items:\s*center/);
  assert.doesNotMatch(tooltipRule, /transform:/);
  assert.doesNotMatch(
    css,
    /\.sync-thread-sync-indicator:hover,\s*\n\.sync-thread-sync-indicator:focus-visible,\s*\n\.sync-thread-sync-error:hover/,
  );
});

test("Push and Pull reviews use only the outer workspace scroll", () => {
  const css = readFileSync("src/App.css", "utf8");
  const workspaceRule = css.match(/\.sync-main\.sync-sync-review-page\s*\{[^}]+\}/s)?.[0] ?? "";
  const reviewScrollRule = css.match(/\.sync-sync-review-page \.sync-sync-review-scroll\s*\{[^}]+\}/s)?.[0] ?? "";
  const reviewSummaryRule = css.match(/\.sync-sync-review-summary\s*\{[^}]+\}/s)?.[0] ?? "";
  const workspaceSource = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const pushSource = readFileSync("src/components/project-sync/PushResourceWorkspace.tsx", "utf8");
  const pullSource = readFileSync("src/components/project-sync/RestorePlanView.tsx", "utf8");
  const tabsSource = readFileSync("src/components/project-sync/SyncReviewTabs.tsx", "utf8");

  assert.match(workspaceRule, /overflow-y:\s*auto/);
  assert.doesNotMatch(workspaceRule, /overflow:\s*hidden/);
  assert.match(reviewScrollRule, /max-height:\s*none/);
  assert.match(reviewScrollRule, /overflow:\s*visible/);
  assert.doesNotMatch(reviewScrollRule, /overflow-y:\s*(auto|scroll)/);
  assert.match(reviewSummaryRule, /margin:\s*0;/);
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

test("history requests share concurrent work but not completed results", async () => {
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

  const first = singleFlight("project-history", start);
  const duplicate = singleFlight("project-history", start);
  assert.strictEqual(first, duplicate);
  assert.equal(starts, 1);

  resolveRequest("complete");
  assert.deepEqual(await Promise.all([first, duplicate]), ["complete", "complete"]);
  assert.equal(await singleFlight("project-history", async () => {
    starts += 1;
    return "refreshed";
  }), "refreshed");
  assert.equal(starts, 2);
});

test("history content uses the local alias and renders commit/thread actions", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      project={project}
      binding={{
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
      }}
      history={history}
      loading={false}
      loadingMore={false}
      actionError={null}
      actionBusyThreadId={null}
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onOpenCodex={() => undefined}
      onOpenTerminal={() => undefined}
      onToggleDetails={() => undefined}
    />,
  );
  const commitHeading = html.match(/<div class="sync-history-commit-heading">([\s\S]*?)<\/div>/)?.[1] ?? "";
  assert.doesNotMatch(html, /Project Name/);
  assert.match(html, /Mallard local/);
  assert.match(html, /sync-history-project-context/);
  assert.match(html, /title="Codex configuration:/);
  assert.match(html, /\/Users\/test\/config\/mallard\/\.codex/);
  assert.match(html, /Local storage 1/);
  assert.match(html, /aria-label="Last pull:/);
  assert.match(html, /aria-label="Last push:/);
  assert.match(html, /Repository: mallard/);
  assert.match(html, />Add project-scoped history</);
  assert.doesNotMatch(html, /sync-history-commit-metrics/);
  assert.match(commitHeading, /data-icon="git-commit"/);
  assert.match(commitHeading, /class="sync-history-commit-title"/);
  assert.match(commitHeading, /class="sync-history-commit-meta"/);
  assert.match(commitHeading, /class="sync-history-commit-actions"/);
  assert.match(commitHeading, /<strong title="Add project-scoped history">Add project-scoped history<\/strong>/);
  assert.ok(commitHeading.indexOf('data-icon="git-commit"') < commitHeading.indexOf("<strong"));
  assert.match(commitHeading, /<code>bbbbbbb<\/code>/);
  assert.match(commitHeading, /<time dateTime=/);
  assert.match(commitHeading, /sync-history-commit-toggle/);
  assert.doesNotMatch(commitHeading, /sync-history-commit-metrics/);
  assert.match(html, /aria-label="Collapse commit bbbbbbb"/);
  assert.match(html, /<time dateTime=[^>]+>[^<]+<\/time><a class="sync-history-commit-link"[^>]+><code>bbbbbbb<\/code><\/a>/);
  assert.match(html, new RegExp(`href="https://github\\.com/Git4Agent/mallard/commit/${"b".repeat(40)}"`));
  assert.match(html, /aria-label="Open commit bbbbbbb on GitHub"/);
  assert.doesNotMatch(html, /sync-history-thread-updated/);
  assert.doesNotMatch(html, /aria-label="Updated [^"]+"/);
  assert.doesNotMatch(html, /aria-label="Show session details"/);
  assert.doesNotMatch(html, /aria-label="Hide session details"/);
  assert.match(html, /aria-label="Started [^"]+"/);
  assert.match(html, /aria-label="Ended [^"]+"/);
  assert.match(html, /aria-label="User turns: 3"/);
  assert.match(html, /data-tooltip="Total tokens · 24\.8K"/);
  assert.match(html, /Appears under 2 commits/);
  assert.match(html, /aria-label="Load chat history"/);
  assert.doesNotMatch(html, /during session|after session|started from/);
  assert.doesNotMatch(html, /Map project-owned Codex sessions onto/);
  assert.doesNotMatch(html, /aria-label="Show conversation details"/);
  assert.match(html, /aria-label="Open in Codex"/);
  assert.match(html, /aria-label="Open in Terminal"/);
  assert.match(html, /aria-label="Show thread debug info"/);
  assert.match(html, /aria-haspopup="dialog"/);
  assert.match(html, /aria-label="Thread debug info"/);
  assert.match(html, /aria-label="Copy thread id"/);
  assert.match(html, /aria-label="Copy working directory"/);
  assert.match(html, /aria-label="Copy codex home"/);
  assert.match(html, /\/Users\/test\/projects\/mallard\/packages\/desktop/);
  assert.match(html, />Copy all</);
  assert.ok(
    html.indexOf('data-tooltip="Tool calls · 8"')
      < html.indexOf('aria-label="Show thread debug info"'),
  );
  assert.match(html, /sync-openai-icon/);
  assert.doesNotMatch(html, /aria-label="1 sessions"/);
  assert.doesNotMatch(html, />Open in Codex</);
  assert.doesNotMatch(html, /> Open in Terminal</);
  assert.doesNotMatch(html, />Load chat history</);
  assert.doesNotMatch(html, /Show chat details/);
});

test("changed storage history explains the recovery path and exposes only unlink", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={history}
      comparison={{
        project_id: project.local_project_id,
        storage_id: "storage-1",
        storage_name: "Local storage 1",
        generation: 2,
        base_generation: 1,
        compared_at: 3,
        entries: [],
        counts: {
          synced: 0,
          local: 0,
          storage: 0,
          diverged: 0,
          unavailable: 0,
          unknown: 0,
        },
        warnings: [],
        remote_continuity: {
          state: "replaced",
          base_generation: 1,
          remote_generation: 2,
        },
      }}
      loading={false}
      loadingMore={false}
      actionError={null}
      actionBusyThreadId={null}
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onOpenCodex={() => undefined}
      onOpenTerminal={() => undefined}
      onUnlinkStorage={() => undefined}
    />,
  );

  assert.match(html, />Storage history changed</);
  assert.match(html, /sync history in Local storage 1 is different from the one Mallard last synced for this project \(generation 1\)/);
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
      project={project}
      binding={{
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
      }}
      history={{ ...history, git: { ...history.git, github_repository_url: null } }}
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

  assert.match(html, new RegExp(`<code title="${"b".repeat(40)}">bbbbbbb</code>`));
  assert.doesNotMatch(html, /aria-label="Open commit .* on GitHub"/);
  assert.doesNotMatch(html, /\/commit\/b{40}/);
});

test("permanent session summaries label every metric icon", () => {
  const html = renderToStaticMarkup(<ThreadMetrics thread={history.threads[0]} />);
  const startedTime = html.match(/sync-history-thread-time-start[\s\S]*?<time[^>]*>([^<]+)<\/time>/)?.[1] ?? "";
  const endedTime = html.match(/sync-history-thread-time-end[\s\S]*?<time[^>]*>([^<]+)<\/time>/)?.[1] ?? "";

  assert.match(html, /data-icon="play"/);
  assert.match(html, /data-icon="check-circle"/);
  assert.match(html, /aria-label="Started [^"]+"/);
  assert.match(html, /aria-label="Ended [^"]+"/);
  assert.match(html, new RegExp(`<time dateTime="${new Date(history.threads[0].started_at * 1_000).toISOString()}"`));
  assert.match(html, new RegExp(`<time dateTime="${new Date(history.threads[0].ended_at * 1_000).toISOString()}"`));
  assert.match(startedTime, / at /);
  assert.match(startedTime, /\d{4}/);
  assert.match(startedTime, /\d{1,2}:\d{2}/);
  assert.doesNotMatch(startedTime, /\d{1,2}:\d{2}:\d{2}/);
  // The pair reads as one interval, so a same-day end drops the date the two
  // stamps share and keeps only the clock time. Full precision stays in the
  // accessible label and the tooltip, which are asserted above.
  assert.match(endedTime, /^\d{1,2}:\d{2}\s?[AP]M$/i);
  assert.doesNotMatch(endedTime, / at /);
  assert.doesNotMatch(endedTime, /\d{1,2}:\d{2}:\d{2}/);
  assert.match(html, /class="sync-history-range-arrow"/);
  assert.match(html, /data-tooltip="User turns · 3"/);
  assert.match(html, /data-tooltip="Total tokens · 24\.8K"/);
  assert.match(html, /data-tooltip="Agent messages · 5"/);
  assert.match(html, /data-tooltip="Tool calls · 8"/);
});

test("a session that starts and ends inside one minute reports elapsed time, not the same clock twice", () => {
  const startedAt = Math.floor(new Date(2026, 7, 1, 18, 53, 0).getTime() / 1_000);
  const html = renderToStaticMarkup(
    <ThreadMetrics thread={{ ...history.threads[0], started_at: startedAt, ended_at: startedAt + 41 }} />,
  );
  const started = html.match(/sync-history-thread-time-start[\s\S]*?<time[^>]*>([^<]+)<\/time>/)?.[1] ?? "";

  assert.match(started, / at /);
  assert.match(html, /class="sync-history-thread-elapsed">\+41s</);
  // An elapsed reading is not a datetime, so it is not marked up as one.
  assert.doesNotMatch(html, new RegExp(`<time dateTime="${new Date((startedAt + 41) * 1_000).toISOString()}"`));
  // The full ending timestamp still reaches assistive tech and the tooltip.
  assert.match(html, /aria-label="Ended [^"]+"/);
});

test("a session with no measurable duration renders a single timestamp", () => {
  const startedAt = Math.floor(new Date(2026, 7, 1, 18, 53, 0).getTime() / 1_000);
  const html = renderToStaticMarkup(
    <ThreadMetrics thread={{ ...history.threads[0], started_at: startedAt, ended_at: startedAt }} />,
  );

  assert.match(html, /sync-history-thread-time-start/);
  assert.doesNotMatch(html, /sync-history-thread-time-end/);
  assert.doesNotMatch(html, /sync-history-range-arrow/);
});

test("session summaries keep the full ending date when the session crosses midnight", () => {
  const startedAt = Math.floor(new Date(2026, 6, 30, 23, 55).getTime() / 1_000);
  const endedAt = Math.floor(new Date(2026, 6, 31, 0, 5).getTime() / 1_000);
  const html = renderToStaticMarkup(
    <ThreadMetrics thread={{ ...history.threads[0], started_at: startedAt, ended_at: endedAt }} />,
  );
  const endedTime = html.match(/sync-history-thread-time-end[\s\S]*?<time[^>]*>([^<]+)<\/time>/)?.[1] ?? "";

  assert.match(endedTime, / at /);
  assert.match(endedTime, /\d{4}/);
});

test("thread icon actions stay compact and the chat action follows the metrics", () => {
  const css = readFileSync("src/App.css", "utf8");
  const metaRule = css.match(/\.sync-history-session-summary \.sync-history-thread-meta\s*\{[^}]+\}/s)?.[0] ?? "";
  const chatRule = css.match(/\.sync-history-chat-toggle\s*\{[^}]+\}/s)?.[0] ?? "";
  const launchRule = css.match(/\.sync-history-thread-actions \.sync-history-launch-action\s*\{[^}]+\}/s)?.[0] ?? "";
  const debugMenuRule = css.match(/\.sync-history-debug-menu\s*\{[^}]+\}/s)?.[0] ?? "";
  const debugPopoverRule = css.match(/\.sync-history-debug-popover\s*\{[^}]+\}/s)?.[0] ?? "";

  assert.match(metaRule, /flex:\s*0 1 auto/);
  assert.match(chatRule, /width:\s*24px/);
  assert.match(chatRule, /padding:\s*0/);
  assert.match(launchRule, /width:\s*28px/);
  assert.match(launchRule, /padding:\s*0/);
  assert.match(debugMenuRule, /position:\s*relative/);
  assert.match(debugPopoverRule, /position:\s*absolute/);
});

test("thread debug copy text keeps complete local identifiers", () => {
  assert.equal(
    threadDebugCopyText(
      history.threads[0].thread_id,
      history.threads[0].working_directory,
      history.codex_home,
    ),
    [
      `Thread ID: ${history.threads[0].thread_id}`,
      `Working directory: ${history.threads[0].working_directory}`,
      `Codex home: ${history.codex_home}`,
    ].join("\n"),
  );
});

test("chat history remains independently collapsible under the permanent summary", () => {
  const thread = history.threads[0];
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={{ ...history, git: null }}
      loading={false}
      loadingMore={false}
      actionError={null}
      actionBusyThreadId={null}
      detailsByThread={{
        [thread.thread_id]: {
          loading: false,
          error: null,
          page: {
            thread_id: thread.thread_id,
            turns: [{ ordinal: 1, role: "user", timestamp: thread.started_at, preview: "Nested conversation preview" }],
            next_cursor: 10,
          },
        },
      }}
      openDetailOccurrences={new Set([thread.thread_id])}
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onOpenCodex={() => undefined}
      onOpenTerminal={() => undefined}
      onToggleDetails={() => undefined}
    />,
  );

  assert.doesNotMatch(html, /aria-label="Hide session details"/);
  assert.doesNotMatch(html, /conversation details/);
  assert.match(html, /aria-label="Hide chat history"/);
  assert.match(html, /data-tooltip="User turns · 3"/);
  assert.match(html, /Load 10 older messages/);
  assert.ok(html.indexOf("Load 10 older messages") < html.indexOf("Nested conversation preview"));
  assert.match(html, /Nested conversation preview/);
});

test("embedded history follows the project workspace without repeating project controls", () => {
  const historyWithUncommitted = {
    ...history,
    threads: [{
      ...history.threads[0],
      title: "Test conversation — fork · 2026-08-01 09:00Z",
    }],
    unmapped: [{
      thread_id: history.threads[0].thread_id,
      reason: "No subsequent commit",
    }],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={historyWithUncommitted}
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

  assert.match(html, /sync-history-embedded/);
  assert.match(html, /<h2[^>]*sync-history-embedded-title[^>]*>.*Git history/s);
  assert.match(html, /class="sync-history-timeline has-uncommitted"/);
  assert.match(html, /<h2 id="uncommitted-heading"><svg[^>]*data-icon="edit"[^>]*>.*<\/svg>Uncommitted<\/h2>/s);
  assert.match(html, /class="sync-history-thread-kind uncommitted"/);
  assert.match(html, /class="sync-history-thread-kind uncommitted"[^>]*>.*data-icon="edit"/s);
  assert.match(html, /<strong title="Test conversation — fork · 2026-08-01 09:00Z">Test conversation — fork<\/strong>/);
  assert.match(html, /aria-label="1 session not linked to a commit"/);
  // The status legend is gone; the per-row tooltips carry the same meanings.
  assert.doesNotMatch(html, /sync-history-status-legend/);
  assert.doesNotMatch(html, /Cloud outstanding/);
  assert.doesNotMatch(html, />Commit history</);
  assert.doesNotMatch(html, /aria-label="1 thread"/);
  assert.doesNotMatch(html, /Sessions not linked to a commit/);
  assert.doesNotMatch(html, /<main/);
  assert.doesNotMatch(html, /aria-label="Project settings"/);
  assert.doesNotMatch(html, /sync-history-project-context/);
  assert.doesNotMatch(html, /Local storage 1/);
});

test("embedded non-Git history uses one Codex threads heading", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={{ ...history, git: null }}
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

  assert.equal(html.match(/Codex threads/g)?.length, 1);
  assert.doesNotMatch(html, />Activity</);
  assert.match(html, /aria-label="1 thread"/);
});

test("selection review removes repeated history and storage labels", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={history}
      loading={false}
      loadingMore={false}
      actionError={null}
      actionBusyThreadId={null}
      activeStorageName="R2 storage 2"
      selectionMode="push"
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onOpenCodex={() => undefined}
      onOpenTerminal={() => undefined}
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

test("Push review removes selection controls and gutters from non-uploadable sessions", () => {
  const syncedThread = history.threads[0];
  const localAheadThread = {
    ...syncedThread,
    thread_id: "019f7798-5437-7632-9dbc-5b589cf68bf1",
    title: "Continue local work",
    started_at: syncedThread.started_at + 1,
    ended_at: syncedThread.ended_at + 1,
  };
  const syncedResourceId = `codex:session:${syncedThread.thread_id}`;
  const localAheadResourceId = `codex:session:${localAheadThread.thread_id}`;
  const resources = [
    {
      resource_id: syncedResourceId,
      kind: "conversation",
      provider: "codex",
      scope: "provider_state",
      display_name: syncedThread.title,
      provenance: { kind: "conversation" as const },
      apply_policy: "safe_file" as const,
      codec_version: 1,
      metadata: {},
      category: "conversations",
    },
    {
      resource_id: localAheadResourceId,
      kind: "conversation",
      provider: "codex",
      scope: "provider_state",
      display_name: localAheadThread.title,
      provenance: { kind: "conversation" as const },
      apply_policy: "safe_file" as const,
      codec_version: 1,
      metadata: {},
      category: "conversations",
    },
  ];
  const comparison = {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    compared_at: 1_752_804_000,
    counts: { synced: 1, local: 1, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: syncedThread.thread_id,
        resource_id: syncedResourceId,
        display_name: syncedThread.title,
        state: "synced" as const,
        local_present: true,
        storage_present: true,
        push_decision: {
          action: "no_op" as const,
          reason_code: "identical" as const,
          selectable: false,
          selected_by_default: false,
          requires_explicit_approval: false,
        },
        pull_decision: {
          action: "no_op" as const,
          reason_code: "identical" as const,
          selectable: false,
          selected_by_default: false,
          requires_explicit_approval: false,
        },
      },
      {
        thread_id: localAheadThread.thread_id,
        resource_id: localAheadResourceId,
        display_name: localAheadThread.title,
        state: "local_ahead" as const,
        local_present: true,
        storage_present: true,
        push_decision: {
          action: "fast_forward_cloud" as const,
          reason_code: "cloud_prefix_of_local" as const,
          selectable: true,
          selected_by_default: true,
          requires_explicit_approval: false,
        },
        pull_decision: {
          action: "keep_local" as const,
          reason_code: "cloud_prefix_of_local" as const,
          selectable: false,
          selected_by_default: false,
          requires_explicit_approval: false,
        },
      },
    ],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={{ ...history, git: null, threads: [syncedThread, localAheadThread] }}
      comparison={comparison}
      activeStorageName="Local storage 1"
      selectionMode="push"
      selectableResourceIds={pushSelectableResourceIds(resources, comparison)}
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

  const syncedInput = html.match(/<input[^>]*aria-label="Include Add Git history mapping"[^>]*>/)?.[0] ?? "";
  const localAheadInput = html.match(/<input[^>]*aria-label="Include Continue local work"[^>]*>/)?.[0] ?? "";
  assert.equal(syncedInput, "");
  assert.notEqual(localAheadInput, "");
  assert.doesNotMatch(localAheadInput, /disabled=""/);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 1);
  assert.equal((html.match(/sync-sync-selectable-row/g) ?? []).length, 1);
  assert.doesNotMatch(html, /sync-selection-placeholder/);
});

test("selected storage adds directional indicators and storage-only threads", () => {
  const comparison = {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    generation: 4,
    base_generation: 3,
    compared_at: 1_752_804_000,
    counts: { synced: 0, local: 2, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: history.threads[0].thread_id,
        resource_id: `codex:session:${history.threads[0].thread_id}`,
        display_name: history.threads[0].thread_id,
        state: "local_ahead" as const,
        local_present: true,
        storage_present: true,
        local_updated_at: history.threads[0].ended_at,
        storage_updated_at: history.threads[0].ended_at - 60,
      },
      {
        thread_id: "019f7798-5437-7632-9dbc-5b589cf68bf1",
        resource_id: "codex:session:019f7798-5437-7632-9dbc-5b589cf68bf1",
        display_name: "Improve cloud thread titles",
        state: "storage_only" as const,
        local_present: false,
        storage_present: true,
        storage_updated_at: 1_752_802_500,
      },
      {
        thread_id: "019f7798-5437-7632-9dbc-5b589cf68bf2",
        resource_id: "codex:session:019f7798-5437-7632-9dbc-5b589cf68bf2",
        display_name: "019f7798-5437-7632-9dbc-5b589cf68bf2",
        state: "local_ahead" as const,
        local_present: true,
        storage_present: true,
        local_updated_at: history.window_start - 60,
        storage_updated_at: history.window_start - 120,
      },
    ],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={{ ...history, git: null }}
      comparison={comparison}
      activeStorageName="Local storage 1"
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

  assert.match(html, /aria-label="Comparing threads with Local storage 1"/);
  assert.match(html, /aria-label="Visible thread comparison with Local storage 1"/);
  assert.match(html, /title="1 local thread change"/);
  assert.doesNotMatch(html, /title="2 local thread changes"/);
  assert.match(html, /aria-label="Newer on this computer\. Push to update Local storage 1\."/);
  assert.match(html, /aria-label="Only in Local storage 1\. Pull to download it here\."/);
  assert.match(html, /sync-thread-sync-indicator local[^>]+aria-label="Newer on this computer\.[^"]+"[^>]*><svg[^>]+>/);
  assert.match(html, /sync-thread-sync-indicator storage[^>]+aria-label="Only in Local storage 1\.[^"]+"[^>]*><svg[^>]+>/);
  assert.doesNotMatch(html, /sync-thread-sync-glyph/);
  assert.match(html, /Improve cloud thread titles/);
  assert.match(html, /sync-history-stored-thread-meta/);
  // The id is a chip and the timestamp a <time>, both right-aligned into the
  // meta gutter, so the "Updated"/"Codex thread" prose labels are gone.
  assert.match(html, /<code title="019f7798-[^"]*">019f7798<\/code>/);
  assert.match(html, /sync-history-stored-thread-meta[^>]*>[\s\S]*?<time[^>]*>[^<]+<\/time>/);
  assert.doesNotMatch(html, /Stored thread 019f7798/);
  assert.match(html, /aria-label="2 threads"/);
  assert.ok(
    html.indexOf('aria-label="Newer on this computer. Push to update Local storage 1."')
      < html.indexOf("Add Git history mapping"),
  );
  assert.ok(
    html.indexOf('aria-label="Only in Local storage 1. Pull to download it here."')
      < html.indexOf("Improve cloud thread titles"),
  );

  const css = readFileSync("src/App.css", "utf8");
  const storedCopyRule = css.match(/\.sync-history-stored-thread-copy\s*\{[^}]+\}/s)?.[0] ?? "";
  assert.match(storedCopyRule, /flex-direction:\s*column/);
});

test("unavailable sessions explain why they cannot be synced", () => {
  const thread = history.threads[0];
  const comparison = {
    project_id: project.local_project_id,
    storage_id: "storage-1",
    storage_name: "Local storage 1",
    compared_at: 1_752_804_000,
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 1, unknown: 0 },
    warnings: [],
    entries: [{
      thread_id: thread.thread_id,
      resource_id: `codex:session:${thread.thread_id}`,
      display_name: thread.thread_id,
      state: "unavailable" as const,
      local_present: true,
      storage_present: false,
      status_detail: "Session file exceeds the 100 MiB per-file sync limit.",
      local_updated_at: thread.ended_at,
    }],
  };
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      embedded
      project={project}
      binding={{
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
      }}
      history={{ ...history, git: null }}
      comparison={comparison}
      activeStorageName="Local storage 1"
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

  assert.match(html, /aria-label="Unavailable for sync\. Session file exceeds the 100 MiB per-file sync limit\."/);
  assert.match(html, /title="1 thread is unavailable for sync"/);
  assert.doesNotMatch(html, /can.t tell which copy is newer/);
});

test("non-Git history renders a flat Codex thread list", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      project={{ ...project, local_alias: null }}
      binding={{
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
      }}
      history={{ ...history, git: null }}
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
  const headingIndex = html.indexOf('id="codex-sessions-heading"');
  const headingEndIndex = html.indexOf("</h2>", headingIndex);
  const openAiIconIndex = html.indexOf("sync-openai-icon", headingIndex);

  assert.match(html, /Codex threads/);
  assert.ok(headingIndex >= 0);
  assert.ok(openAiIconIndex > headingIndex);
  assert.ok(openAiIconIndex < headingEndIndex);
  assert.doesNotMatch(html, /Branch/);
});

test("an invalid persisted Codex profile stays in the project workspace", () => {
  const html = renderToStaticMarkup(
    <ProjectChatHistoryContent
      project={project}
      binding={{
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
      }}
      history={null}
      loading={false}
      loadingMore={false}
      actionError="Codex profile path changed."
      actionBusyThreadId={null}
      onBranchChange={() => undefined}
      onRefresh={() => undefined}
      onLoadMore={() => undefined}
      onOpenCodex={() => undefined}
      onOpenTerminal={() => undefined}
    />,
  );
  assert.match(html, /role="alert"/);
  assert.doesNotMatch(html, /Open Project Settings|Project settings/);
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

test("sidebar utility controls use matching Pixelarticons and a pixel count badge", () => {
  for (const name of ["plus", "settings", "trash"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={16} />);
    assert.match(html, /icon-pixelarticons/, `${name} must match the sidebar utility icon family`);
    assert.match(html, /fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }

  const css = readFileSync("src/RetroTheme.css", "utf8");
  const badgeRule = css.match(
    /\.sidebar-nav-badge,\s*\n\.sync-primary-nav button > \.sync-activity-badge\s*\{[^}]+\}/s,
  )?.[0] ?? "";
  assert.match(badgeRule, /font-family:\s*var\(--font-pixel\)/);
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

test("history launch actions use matching Pixelarticons silhouettes", () => {
  for (const name of ["git-fork", "terminal"] as const) {
    const html = renderToStaticMarkup(<Icon name={name} size={14} />);
    assert.match(html, /icon-pixelarticons/, `${name} must match the history launch action family`);
    assert.match(html, /fill="currentColor"/);
    assert.match(html, /shape-rendering="crispEdges"/);
    assert.doesNotMatch(html, /<(?:circle|ellipse|line|polyline)\b/);
  }
});

test("ChatGPT actions use the recognizable knot mark with crisp pixel rendering", () => {
  const html = renderToStaticMarkup(<Icon name="openai" size={14} />);
  assert.match(html, /icon-chatgpt-mark/);
  assert.match(html, /transform="translate\(1 1\) scale\(0\.06875\)"/);
  assert.match(html, /fill="currentColor"/);
  assert.match(html, /shape-rendering="crispEdges"/);
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
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
    />,
  );
  assert.match(html, /src="\/mallard-logo-pixel-ring\.png"/);
  assert.match(html, /src="\/mallard-logo-pixel-duck\.png"/);
  assert.doesNotMatch(html, /src="\/mallard-logo-pixel\.png"/);
  assert.doesNotMatch(html, /src="\/mallard-logo\.png"/);
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
  const logsNavIndex = primaryNav.indexOf(" Logs</button>");
  assert.ok(projectsNavIndex >= 0);
  assert.ok(storagesNavIndex > projectsNavIndex);
  assert.ok(logsNavIndex > storagesNavIndex);
  assert.match(html, /aria-label="Storages"/);
  assert.match(html, />Storages<\/span>/);
  assert.match(html, /role="separator" aria-label="Resize Projects and Storages sections" aria-orientation="horizontal"/);
  assert.match(html, /aria-valuenow="56"/);
  assert.doesNotMatch(html, /sidebar-section-count/);
  assert.doesNotMatch(html, /Git Based|Non-Git Based/);
  assert.doesNotMatch(html, /Project settings for Mallard local/);
  assert.doesNotMatch(html, /View history for draft repo/);
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
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
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

void ProjectChatHistoryPage;
