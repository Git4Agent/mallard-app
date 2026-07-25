import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import Icon from "../../src/components/Icons";
import ProjectChatHistoryPage, {
  BranchPicker,
  ProjectChatHistoryContent,
  ThreadMetrics,
  threadSyncPresentation,
} from "../../src/components/project-sync/ProjectChatHistoryPage";
import ProjectSidebar from "../../src/components/project-sync/ProjectSidebar";
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

test("V3 thread indicators use the quiet desktop sync vocabulary", () => {
  const presentation = (state: Parameters<typeof threadSyncPresentation>[0]["state"]) => (
    threadSyncPresentation({ state } as Parameters<typeof threadSyncPresentation>[0], "Storage")
  );

  assert.deepEqual(
    ["synced", "local_only", "local_ahead", "storage_only", "storage_ahead", "diverged", "unavailable", "unknown"].map((state) => {
      const { icon, className } = presentation(state as Parameters<typeof presentation>[0]);
      return { icon, className };
    }),
    [
      { icon: "badge-check", className: "synced" },
      { icon: "upload", className: "local" },
      { icon: "upload", className: "local" },
      { icon: "download", className: "storage" },
      { icon: "download", className: "storage" },
      { icon: "git-compare-arrows", className: "diverged" },
      { icon: "octagon-x", className: "unavailable" },
      { icon: "cloud-off", className: "needs-baseline" },
    ],
  );

  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.v3-thread-sync-indicator\.synced \{ color: var\(--text-2\); \}/);
  assert.match(css, /\.v3-thread-sync-indicator\.local,\s*\n\.v3-thread-sync-indicator\.storage \{ color: var\(--blue\); \}/);
  assert.match(css, /\.v3-thread-sync-indicator\.diverged \{ color: var\(--yellow\); \}/);
  assert.match(css, /\.v3-thread-sync-indicator\.unavailable,\s*\n\.v3-thread-sync-error \{ color: var\(--red\); \}/);
  assert.match(css, /\.v3-thread-sync-indicator\.needs-baseline \{ color: var\(--text-2\); \}/);
  assert.match(css, /\.v3-thread-sync-loading\s*\{[^}]*color:\s*var\(--blue\);/s);
});

test("sync tooltip hover keeps the sibling icon rail geometry stable", () => {
  const css = readFileSync("src/App.css", "utf8");
  const titleRule = css.match(/\.v3-history-thread-title\s*\{[^}]+\}/s)?.[0] ?? "";
  const tooltipRule = css.match(/\.v3-thread-sync-indicator::after,[\s\S]+?\n\}/)?.[0] ?? "";

  assert.match(titleRule, /align-items:\s*center/);
  assert.doesNotMatch(tooltipRule, /transform:/);
  assert.doesNotMatch(
    css,
    /\.v3-thread-sync-indicator:hover,\s*\n\.v3-thread-sync-indicator:focus-visible,\s*\n\.v3-thread-sync-error:hover/,
  );
});

test("Push and Pull reviews use only the outer workspace scroll", () => {
  const css = readFileSync("src/App.css", "utf8");
  const workspaceRule = css.match(/\.v3-main\.v3-sync-review-page\s*\{[^}]+\}/s)?.[0] ?? "";
  const reviewScrollRule = css.match(/\.v3-sync-review-page \.v3-sync-review-scroll\s*\{[^}]+\}/s)?.[0] ?? "";
  const reviewSummaryRule = css.match(/\.v3-sync-review-summary\s*\{[^}]+\}/s)?.[0] ?? "";
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
  assert.match(workspaceSource, /v3-sync-review-page/);
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

test("completed Push and Pull operations reset the project workspace to its first tab", () => {
  const workspaceSource = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const syncSource = readFileSync("src/components/project-sync/ProjectSyncV3.tsx", "utf8");
  const pushHandler = syncSource.slice(
    syncSource.indexOf("const publishProject = async"),
    syncSource.indexOf("const publishPendingPush = async"),
  );
  const pullHandler = syncSource.slice(
    syncSource.indexOf("const applyReview = async"),
    syncSource.indexOf("const openNewStorageConfiguration"),
  );

  assert.match(workspaceSource, /projectTabResetEpoch/);
  assert.match(workspaceSource, /setActiveProjectTab\("history"\)/);
  assert.match(pushHandler, /setProjectTabResetEpoch/);
  assert.match(pullHandler, /setProjectTabResetEpoch/);
  assert.match(pullHandler, /!result\.error && pullMutationCompleted/);
});

test("the combined project header stays aligned with Storage and shares its divider", () => {
  const css = readFileSync("src/App.css", "utf8");
  const combinedHeadingRule = css.match(/\.v3-project-combined-page \.v3-combined-project-heading\s*\{[^}]+\}/s)?.[0] ?? "";
  const storageGroupRule = css.match(/\.v3-project-combined-page \.project-storage-group\s*\{[^}]+\}/s)?.[0] ?? "";

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
  assert.doesNotMatch(html, /Project Name/);
  assert.match(html, /Mallard local/);
  assert.match(html, /v3-history-project-context/);
  assert.match(html, /title="Codex configuration:/);
  assert.match(html, /\/Users\/test\/config\/mallard\/\.codex/);
  assert.match(html, /Local storage 1/);
  assert.match(html, /aria-label="Last pull:/);
  assert.match(html, /aria-label="Last push:/);
  assert.match(html, /Repository: mallard/);
  assert.match(html, /Add project-scoped history/);
  assert.match(html, new RegExp(`href="https://github\\.com/Git4Agent/mallard/commit/${"b".repeat(40)}"`));
  assert.match(html, /aria-label="Open commit bbbbbbb on GitHub"/);
  assert.doesNotMatch(html, /v3-history-thread-updated/);
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
  assert.match(html, /v3-openai-icon/);
  assert.doesNotMatch(html, /aria-label="1 sessions"/);
  assert.doesNotMatch(html, />Open in Codex</);
  assert.doesNotMatch(html, /> Open in Terminal</);
  assert.doesNotMatch(html, />Load chat history</);
  assert.doesNotMatch(html, /Show chat details/);
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

  assert.match(html, /data-tooltip="User turns · 3"/);
  assert.match(html, /data-tooltip="Total tokens · 24\.8K"/);
  assert.match(html, /data-tooltip="Agent messages · 5"/);
  assert.match(html, /data-tooltip="Tool calls · 8"/);
});

test("thread icon actions stay compact and the chat action follows the metrics", () => {
  const css = readFileSync("src/App.css", "utf8");
  const metaRule = css.match(/\.v3-history-session-summary \.v3-history-thread-meta\s*\{[^}]+\}/s)?.[0] ?? "";
  const chatRule = css.match(/\.v3-history-chat-toggle\s*\{[^}]+\}/s)?.[0] ?? "";
  const launchRule = css.match(/\.v3-history-thread-actions \.v3-history-launch-action\s*\{[^}]+\}/s)?.[0] ?? "";

  assert.match(metaRule, /flex:\s*0 1 auto/);
  assert.match(chatRule, /width:\s*24px/);
  assert.match(chatRule, /padding:\s*0/);
  assert.match(launchRule, /width:\s*28px/);
  assert.match(launchRule, /padding:\s*0/);
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

  assert.match(html, /v3-history-embedded/);
  assert.match(html, /<h2[^>]*v3-history-embedded-title[^>]*>.*Git history/s);
  assert.match(html, /<h2 id="uncommitted-heading">Uncommitted<\/h2>/);
  assert.doesNotMatch(html, />Commit history</);
  assert.doesNotMatch(html, /aria-label="1 thread"/);
  assert.doesNotMatch(html, /Sessions not linked to a commit/);
  assert.doesNotMatch(html, /<main/);
  assert.doesNotMatch(html, /aria-label="Project settings"/);
  assert.doesNotMatch(html, /v3-history-project-context/);
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

  assert.match(html, /v3-history-selection-review/);
  assert.match(html, /aria-label="Git history"/);
  assert.doesNotMatch(html, /v3-history-header/);
  assert.doesNotMatch(html, />Local session</);
  assert.doesNotMatch(html, /v3-history-storage-lens/);
  assert.doesNotMatch(html, /v3-history-toolbar-count/);
  assert.doesNotMatch(html, /aria-label="Branch"/);
  assert.doesNotMatch(html, /aria-label="Refresh activity"/);

  const css = readFileSync("src/App.css", "utf8");
  const compactReviewRule = css.match(/\.v3-sync-review-scroll > \.v3-history-selection-review\s*\{[^}]+\}/s)?.[0] ?? "";
  const compactHeadingRule = css.match(/\.v3-history-selection-review \.v3-history-section-heading\s*\{[^}]+\}/s)?.[0] ?? "";
  assert.match(compactReviewRule, /padding-top:\s*0/);
  assert.match(compactReviewRule, /padding-right:\s*8px/);
  assert.match(compactHeadingRule, /margin-top:\s*8px/);
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
        display_name: "019f7798-5437-7632-9dbc-5b589cf68bf1",
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
  assert.match(html, /v3-thread-sync-indicator local[^>]+aria-label="Newer on this computer\.[^"]+"[^>]*><svg[^>]+>/);
  assert.match(html, /v3-thread-sync-indicator storage[^>]+aria-label="Only in Local storage 1\.[^"]+"[^>]*><svg[^>]+>/);
  assert.doesNotMatch(html, /v3-thread-sync-glyph/);
  assert.match(html, /Stored thread 019f7798/);
  assert.match(html, /aria-label="2 threads"/);
  assert.ok(
    html.indexOf('aria-label="Newer on this computer. Push to update Local storage 1."')
      < html.indexOf("Add Git history mapping"),
  );
  assert.ok(
    html.indexOf('aria-label="Only in Local storage 1. Pull to download it here."')
      < html.indexOf("Stored thread 019f7798"),
  );
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
  const openAiIconIndex = html.indexOf("v3-openai-icon", headingIndex);

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

test("the Git project icon uses the bold hollow-folder design with an oversized branch", () => {
  const html = renderToStaticMarkup(<Icon name="git-folder" size={16} />);
  assert.match(html, /icon-git-folder-shell/);
  assert.match(html, /icon-git-folder-mark/);
  assert.match(html, /cx="10.1" cy="10.6" r="1.3"/);
  assert.match(html, /cx="8.5" cy="16.4" r="1.3"/);
  assert.doesNotMatch(html, /fill="currentColor"/);
  assert.doesNotMatch(html, /d="M3 10h18"/);
});

test("standalone Git branch icons keep the shared blue mark", () => {
  const html = renderToStaticMarkup(<Icon name="git-branch" size={16} />);
  const css = readFileSync("src/App.css", "utf8");

  assert.match(html, /icon-git-branch-mark/);
  assert.match(html, /class="icon-git-branch-mark" stroke="currentColor"/);
  assert.match(
    css,
    /\.icon-git-branch-mark,\s*\.icon-git-folder-mark\s*\{\s*color:\s*var\(--blue\);\s*\}/,
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
  assert.match(html, /src="\/mallard-logo\.svg"/);
  assert.doesNotMatch(html, /mallard-logo\.png/);
  assert.doesNotMatch(html, /aria-label="View history for Mallard local"/);
  assert.equal(html.match(/v3-project-git-icon/g)?.length, 1);
  assert.equal(html.match(/icon-git-folder-mark/g)?.length, 1);
  assert.equal(html.match(/v3-project-folder-icon/g)?.length, 1);
  assert.match(html, /aria-label="Mallard local, Git repository"/);
  assert.doesNotMatch(html, /v3-repository-kind|>git<\/span>/);
  assert.doesNotMatch(html, /Default Codex|myconf3 · Codex/);
  assert.match(html, /role="separator" aria-label="Resize Projects and Storage sections" aria-orientation="horizontal"/);
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
  assert.doesNotMatch(html, /Legacy profiles|Open legacy/);
});

void ProjectChatHistoryPage;
