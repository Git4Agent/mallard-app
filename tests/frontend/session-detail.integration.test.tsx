import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  ProjectSessionDetail,
  ProjectSessionTimelinePage,
  SessionTimelineEvent,
} from "../../src/types";
import SessionDetailEvent, {
  SessionDetailMarkdown,
} from "../../src/components/project-sync/session-detail/SessionDetailEvent";
import SessionDetailHeader from "../../src/components/project-sync/session-detail/SessionDetailHeader";
import SessionDetailInspector from "../../src/components/project-sync/session-detail/SessionDetailInspector";
import SessionDetailTimeline from "../../src/components/project-sync/session-detail/SessionDetailTimeline";
import SessionDetailToolCall from "../../src/components/project-sync/session-detail/SessionDetailToolCall";
import {
  createDefaultSessionDetailFilters,
  eventAnchorId,
  formatSessionCount,
  includedSessionEventKinds,
  includedSessionToolCategories,
  mergeSessionTimelinePages,
  safeMarkdownUrl,
  sessionToolParentState,
  toggleSessionToolParent,
} from "../../src/components/project-sync/session-detail/sessionDetailModel";

const exact = (value: number) => ({ value, quality: "exact" as const });

const detail: ProjectSessionDetail = {
  resource_id: "codex:conversation:session-1",
  provider: "codex",
  provider_display_name: "Codex",
  provider_session_id: "session-1",
  title: "Build provider-neutral session detail",
  summary: "Expose the durable conversation and its bounded tool evidence.",
  started_at: 1_786_000_000,
  ended_at: 1_786_000_600,
  is_active: false,
  branch: "main",
  recorded_sha: "a".repeat(40),
  tokens: { value: 18_200, quality: "lower_bound", explanation: "Some usage records were absent." },
  event_counts: {
    prompts: exact(2),
    responses: exact(3),
    intermediate_steps: { value: 1, quality: "lower_bound", explanation: "One legacy phase may be missing." },
    checkpoints: { value: 1, quality: "inferred", explanation: "Matched to the successor commit." },
    tool_calls: exact(4),
    file_edits: exact(1),
    shell_calls: exact(2),
    reads: { quality: "unsupported", explanation: "This transcript cannot distinguish direct reads." },
    other_tools: exact(1),
  },
  capabilities: {
    intermediate_steps: true,
    checkpoints: true,
    tool_calls: true,
    event_detail: true,
  },
  actions: [{
    action_id: "codex.open_in_app",
    kind: "open_in_app",
    label: "Open in Codex",
    enabled: true,
  }],
  checkpoint_jumps: [{
    event_id: "checkpoint-1",
    checkpoint: {
      sha: "b".repeat(40),
      short_sha: "bbbbbbb",
      subject: "Add session detail timeline",
      committed_at: 1_786_000_500,
      branch: "main",
    },
  }],
  snapshot_id: "snapshot-1",
  warnings: [],
};

const toolEvent: SessionTimelineEvent = {
  event_id: "tool-1",
  ordinal: 3,
  timestamp: 1_786_000_200,
  kind: "tool_call",
  quality: "exact",
  turn_id: "turn-1",
  parent_event_id: "response-1",
  tool: {
    provider_call_id: "call-1",
    native_name: "Read",
    category: "read",
    status: "completed",
    summary: "src/main.tsx",
    argument_truncated: false,
    result_truncated: false,
    affected_paths: ["src/main.tsx"],
  },
};

const events: SessionTimelineEvent[] = [
  {
    event_id: "prompt-1",
    ordinal: 1,
    timestamp: 1_786_000_000,
    kind: "prompt",
    quality: "exact",
    turn_id: "turn-1",
    text: "Please **inspect** the current flow.",
    turn_tool_summary: { turn_id: "turn-1", tool_calls: exact(2) },
  },
  {
    event_id: "response-1",
    ordinal: 2,
    timestamp: 1_786_000_100,
    kind: "response",
    quality: "exact",
    turn_id: "turn-1",
    text: "I found the relevant boundary and kept the change scoped.",
  },
  toolEvent,
  {
    event_id: "intermediate-1",
    ordinal: 4,
    timestamp: 1_786_000_300,
    kind: "intermediate_step",
    quality: "inferred",
    turn_id: "turn-1",
    text: "Running the focused checks now.",
  },
  {
    event_id: "checkpoint-1",
    ordinal: 5,
    timestamp: 1_786_000_500,
    kind: "checkpoint",
    quality: "inferred",
    checkpoint: detail.checkpoint_jumps[0].checkpoint,
  },
];

test("default filters keep conversation and checkpoints visible while individual tools stay off", () => {
  const filters = createDefaultSessionDetailFilters(detail.event_counts);
  assert.deepEqual(includedSessionEventKinds(filters), ["prompt", "response", "checkpoint"]);
  assert.deepEqual(includedSessionToolCategories(filters), ["file_edit", "shell", "other"]);
  assert.equal(filters.toolCalls, false);
  assert.equal(filters.toolCategories.file_edit, true);
  assert.equal(filters.toolCategories.shell, true);
  assert.equal(filters.toolCategories.read, false);
  assert.equal(filters.toolCategories.other, true);

  const initialParent = sessionToolParentState(filters, detail.event_counts);
  assert.equal(initialParent.checked, false);
  assert.equal(initialParent.indeterminate, false);
  const enabled = toggleSessionToolParent(filters, detail.event_counts);
  assert.equal(enabled.toolCalls, true);
  assert.equal(sessionToolParentState(enabled, detail.event_counts).checked, true);
});

test("the inspector shows stable full-session counts and selected-but-disabled tool categories", () => {
  const filters = createDefaultSessionDetailFilters(detail.event_counts);
  const html = renderToStaticMarkup(
    <SessionDetailInspector
      detail={detail}
      filters={filters}
      loadedToolCount={0}
      allLoadedToolsExpanded={false}
      selectedCheckpointId=""
      onFiltersChange={() => undefined}
      onCheckpointJump={() => undefined}
      onToggleAllLoadedTools={() => undefined}
    />,
  );

  assert.match(html, /<legend>Filters<\/legend>/);
  assert.match(html, />Prompts<\/span><span class="session-detail-filter-count quality-exact"[^>]*>2<\/span>/);
  assert.match(html, />Intermediate steps<\/span><span class="session-detail-filter-count quality-lower_bound"[^>]*>1\+<\/span>/);
  assert.match(html, />Read<\/span><span class="session-detail-filter-count quality-unsupported"[^>]*>Unavailable<\/span>/);
  assert.match(html, /aria-label="Tool call categories"/);
  assert.match(html, /type="checkbox" disabled="" checked=""/);
  assert.match(html, /Expand loaded tool calls/);
  assert.match(html, /disabled=""[^>]*>[^<]*<svg[^>]*data-icon="chevron-down"/);
  assert.match(html, /value="checkpoint-1">bbbbbbb · Add session detail timeline/);
});

test("timeline hierarchy contains prompts, open responses, progress, tools, and checkpoint anchors", () => {
  const html = renderToStaticMarkup(
    <SessionDetailTimeline
      events={events}
      providerDisplayName="Codex"
      loading={false}
      loadingMore={false}
      nextCursor={null}
      error={null}
      expandedToolIds={new Set()}
      toolDetails={{}}
      turnToolStates={{}}
      onRetry={() => undefined}
      onLoadMore={() => undefined}
      onToolExpandedChange={() => undefined}
      onTurnToolExpandedChange={() => undefined}
      onLoadMoreTurnTools={() => undefined}
    />,
  );

  assert.match(html, /class="session-detail-event-copy session-detail-prompt-card"/);
  assert.match(html, /class="session-detail-event-copy session-detail-response-prose"/);
  assert.match(html, /class="session-detail-event-copy session-detail-intermediate-prose"/);
  assert.match(html, /2 tool calls/);
  assert.match(html, /data-event-kind="tool_call"/);
  assert.match(html, /data-event-kind="checkpoint"/);
  assert.match(html, new RegExp(`id="${eventAnchorId("checkpoint-1")}"`));
  assert.match(html, /bbbbbbb/);
  assert.match(html, />main<\/span>/);

  const response = html.match(/<article[^>]*event-response[\s\S]*?<\/article>/)?.[0] ?? "";
  assert.match(response, /session-detail-response-prose/);
  assert.doesNotMatch(response, /session-detail-prompt-card/);
});

test("tool payloads render only inside an expanded escaped disclosure", () => {
  const collapsed = renderToStaticMarkup(
    <SessionDetailToolCall
      event={toolEvent}
      expanded={false}
      onExpandedChange={() => undefined}
    />,
  );
  assert.doesNotMatch(collapsed, /<details[^>]* open/);
  assert.doesNotMatch(collapsed, /<h4>Arguments<\/h4>/);

  const expanded = renderToStaticMarkup(
    <SessionDetailToolCall
      event={toolEvent}
      expanded
      detailState={{
        loading: false,
        error: null,
        detail: {
          event_id: toolEvent.event_id,
          arguments: "<script>bad()</script>\npath = src/main.tsx",
          arguments_truncated: true,
          result: "Read 42 lines",
          result_truncated: false,
          affected_paths: ["src/main.tsx"],
        },
      }}
      onExpandedChange={() => undefined}
    />,
  );
  assert.match(expanded, /<details[^>]* open=""/);
  assert.match(expanded, /<h4>Arguments<\/h4><pre><code>&lt;script&gt;bad\(\)&lt;\/script&gt;/);
  assert.doesNotMatch(expanded, /<script>/);
  assert.match(expanded, /Bounded preview · additional content omitted/);
  assert.match(expanded, /<h4>Result<\/h4>/);
});

test("Markdown drops raw HTML and unsafe link protocols", () => {
  const html = renderToStaticMarkup(
    <SessionDetailMarkdown text={'<script>alert("x")</script>\n\n[bad](javascript:alert(1)) [good](https://example.com/docs)'} />,
  );
  assert.doesNotMatch(html, /<script>|javascript:/i);
  assert.match(html, />bad<\/span>/);
  assert.match(html, /href="https:\/\/example\.com\/docs"/);
  assert.equal(safeMarkdownUrl("file:///tmp/secret"), "");
  assert.equal(safeMarkdownUrl("javascript:alert(1)"), "");
  assert.equal(safeMarkdownUrl("https://example.com/docs"), "https://example.com/docs");
});

test("header and filters expose the expected navigation and fieldset semantics", () => {
  const html = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      headingRef={{ current: null }}
      actionBusyId={null}
      actionError={null}
      onRefresh={() => undefined}
      onAction={() => undefined}
    />,
  );
  // The workspace path bar carries the trail and the way back, so the header
  // no longer draws a breadcrumb or a Back button of its own.
  assert.doesNotMatch(html, /Back to Activity/);
  assert.doesNotMatch(html, /session-detail-breadcrumbs/);
  assert.match(html, /^<header class="session-detail-header"><div class="session-detail-heading-row">/);
  assert.match(html, /<h1 id="session-detail-heading" tabindex="-1">/);
  assert.match(html, /Open in Codex/);
  assert.match(html, />18,200\+<\/code>/);
});

test("timeline page merging is chronological, de-duplicated, and count labels preserve quality", () => {
  const first: ProjectSessionTimelinePage = {
    resource_id: detail.resource_id,
    snapshot_id: detail.snapshot_id,
    events: [events[1], events[0]],
    next_cursor: "next",
    warnings: [],
  };
  const second: ProjectSessionTimelinePage = {
    ...first,
    events: [events[1], events[3]],
    next_cursor: null,
  };
  const merged = mergeSessionTimelinePages(
    mergeSessionTimelinePages([], first),
    second,
  );
  assert.deepEqual(merged.map((event) => event.event_id), ["prompt-1", "response-1", "intermediate-1"]);
  assert.equal(formatSessionCount({ value: 4, quality: "lower_bound" }), "4+");
  assert.equal(formatSessionCount({ quality: "unsupported" }), "Unavailable");
});

test("workspace navigation lazy-loads detail and preserves the mounted Activity controller", () => {
  const workspaceSource = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const syncSource = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const activitySource = readFileSync("src/components/project-sync/ProjectActivityPage.tsx", "utf8");
  const apiSource = readFileSync("src/components/project-sync/api.ts", "utf8");

  assert.match(workspaceSource, /lazy\(\(\) => import\("\.\/session-detail\/SessionDetailPage"\)\)/);
  assert.match(workspaceSource, /hidden=\{sessionDetailSubview != null\}/);
  assert.match(workspaceSource, /inert=\{sessionDetailSubview != null\}/);
  assert.match(workspaceSource, /overviewScrollPositionRef/);
  assert.match(workspaceSource, /origin\?\.focus\(\{ preventScroll: true \}\)/);
  assert.match(syncSource, /type ActiveProjectSubview|ActiveProjectSubview/);
  assert.match(syncSource, /kind: "session_detail"/);
  assert.match(activitySource, /sync-history-session-title-button/);
  assert.match(activitySource, /session\.source !== "remote_only"/);
  assert.match(activitySource, /!selectionMode/);
  assert.doesNotMatch(activitySource, /sync-history-chat-details|getProjectSessionDetails/);

  assert.match(apiSource, /"get_project_session_detail"/);
  assert.match(apiSource, /"get_project_session_timeline_page"/);
  assert.match(apiSource, /"get_project_session_event_detail"/);
  assert.doesNotMatch(apiSource, /"get_project_session_details"/);
});

test("base and RetroTheme selectors encode wide, medium, narrow, and asymmetric message layouts", () => {
  const baseCss = readFileSync("src/App.css", "utf8");
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");
  assert.match(baseCss, /grid-template-columns: minmax\(0, 1fr\) minmax\(240px, 284px\)/);
  assert.match(baseCss, /@container session-detail-layout \(max-width: 980px\)/);
  assert.match(baseCss, /@container session-detail-layout \(max-width: 720px\)/);
  assert.match(baseCss, /\.session-detail-prompt-card[\s\S]*background: var\(--surface-card\)/);
  assert.match(baseCss, /\.session-detail-response-prose[\s\S]*background: transparent/);
  assert.match(retroCss, /\.session-detail-prompt-card,[\s\S]*var\(--surface-card\)/);
  assert.match(retroCss, /\.session-detail-response-prose[\s\S]*background: transparent/);
});
