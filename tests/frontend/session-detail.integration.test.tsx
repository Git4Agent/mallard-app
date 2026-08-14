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
import SessionDetailTimeline from "../../src/components/project-sync/session-detail/SessionDetailTimeline";
import SessionDetailToolbar from "../../src/components/project-sync/session-detail/SessionDetailToolbar";
import SessionDetailToolCall from "../../src/components/project-sync/session-detail/SessionDetailToolCall";
import {
  createDefaultSessionDetailFilters,
  eventAnchorId,
  formatCompactCount,
  formatSessionCount,
  formatSessionDuration,
  formatSessionGap,
  formatSessionRange,
  groupSessionEventsByDay,
  includedSessionEventKinds,
  mergeSessionTimelinePages,
  readableToolPayload,
  safeMarkdownUrl,
  sessionDetailFilterToggles,
  sessionEventCoverage,
  toolCommandLabel,
} from "../../src/components/project-sync/session-detail/sessionDetailModel";

const exact = (value: number) => ({ value, quality: "exact" as const });

// 2026-08-13T11:10:00Z and onward, so the day divider and the gutter clock have
// a real day boundary to work with.
const START = 1_786_000_000;

const detail: ProjectSessionDetail = {
  resource_id: "codex:conversation:session-1",
  provider: "codex",
  provider_display_name: "Codex",
  provider_session_id: "session-1",
  title: "Build provider-neutral session detail",
  summary: "Expose the durable conversation and its bounded tool evidence.",
  started_at: START,
  ended_at: START + 660,
  is_active: false,
  branch: "main",
  recorded_sha: "a".repeat(40),
  tokens: { value: 1_063_252, quality: "lower_bound", explanation: "Some usage records were absent." },
  event_counts: {
    prompts: exact(2),
    responses: exact(3),
    intermediate_steps: { value: 1, quality: "lower_bound", explanation: "One legacy phase may be missing." },
    checkpoints: exact(0),
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
  checkpoint_jumps: [],
  snapshot_id: "snapshot-1",
  warnings: [],
};

const toolEvent: SessionTimelineEvent = {
  event_id: "tool-1",
  ordinal: 3,
  timestamp: START + 200,
  kind: "tool_call",
  quality: "exact",
  turn_id: "turn-1",
  parent_event_id: "response-1",
  tool: {
    provider_call_id: "call-1",
    native_name: "exec",
    category: "read",
    status: "completed",
    summary: "sed -n '241,520p' skills/imagegen/SKILL.md",
    argument_truncated: false,
    result_truncated: false,
    affected_paths: ["src/main.tsx"],
  },
};

const failedToolEvent: SessionTimelineEvent = {
  ...toolEvent,
  event_id: "tool-2",
  ordinal: 4,
  tool: { ...toolEvent.tool!, provider_call_id: "call-2", status: "failed", summary: "python3 render.py" },
};

const events: SessionTimelineEvent[] = [
  {
    event_id: "prompt-1",
    ordinal: 1,
    timestamp: START,
    kind: "prompt",
    quality: "exact",
    turn_id: "turn-1",
    text: "Please **inspect** the current flow.",
    turn_tool_summary: { turn_id: "turn-1", tool_calls: exact(2) },
  },
  {
    event_id: "response-1",
    ordinal: 2,
    timestamp: START + 300,
    kind: "response",
    quality: "exact",
    turn_id: "turn-1",
    text: "I found the relevant boundary and kept the change scoped.",
  },
  toolEvent,
  {
    event_id: "intermediate-1",
    ordinal: 5,
    timestamp: START + 400,
    kind: "intermediate_step",
    quality: "inferred",
    turn_id: "turn-1",
    text: "Running the focused checks now.",
  },
];

test("filters carry event kinds only — tool categories left with the inspector", () => {
  const filters = createDefaultSessionDetailFilters(detail.event_counts);
  assert.deepEqual(includedSessionEventKinds(filters), ["prompt", "response", "checkpoint"]);
  assert.equal(filters.toolCalls, false);
  assert.equal("toolCategories" in filters, false);

  // The query still names every category, because sorting by category is gone
  // from the UI, not from the command.
  const pageSource = readFileSync(
    "src/components/project-sync/session-detail/SessionDetailPage.tsx",
    "utf8",
  );
  assert.match(pageSource, /included_tool_categories: \[\.\.\.ALL_SESSION_TOOL_CATEGORIES\]/);
  assert.doesNotMatch(pageSource, /SessionDetailInspector|selectedCheckpointId|toggleAllLoadedTools/);
});

test("a toggle appears only for a kind this session actually holds", () => {
  const toggles = sessionDetailFilterToggles(detail.event_counts);
  // checkpoints is exactly 0 here and `reads` is unsupported: neither earns a
  // control. The old panel rendered both, one of them disabled.
  assert.deepEqual(toggles.map((toggle) => toggle.key), [
    "prompts",
    "responses",
    "intermediateSteps",
    "toolCalls",
  ]);
  assert.deepEqual(toggles.map((toggle) => toggle.label), [
    "Prompts",
    "Replies",
    "Steps",
    "Tool calls",
  ]);

  const coverage = sessionEventCoverage(detail.event_counts);
  assert.equal(coverage.total, 2 + 3 + 1 + 0 + 4);
  // intermediate_steps is a lower bound, so the denominator refuses to claim
  // precision it does not have.
  assert.equal(coverage.approximate, true);
});

test("the toolbar states one count, with the denominator the page never had", () => {
  const filters = createDefaultSessionDetailFilters(detail.event_counts);
  const html = renderToStaticMarkup(
    <SessionDetailToolbar
      counts={detail.event_counts}
      filters={filters}
      loadedCount={5}
      onFiltersChange={() => undefined}
    />,
  );

  assert.match(html, /class="session-detail-toolbar"/);
  assert.match(html, /5 of 10\+ events/);
  assert.match(html, /aria-pressed="true"[^>]*>[^<]*<span class="session-detail-filter-box"[^>]*><\/span><span class="session-detail-filter-name">Prompts/);
  assert.match(html, /<span class="session-detail-filter-name">Tool calls<\/span><span class="session-detail-filter-count quality-exact">4<\/span>/);
  // No checkpoint control and no second panel of any kind.
  assert.doesNotMatch(html, /Checkpoints|No checkpoints|Expand loaded tool calls|<fieldset|<select/);
});

test("the header prints the title once, states duration, and marks the sha as a commit", () => {
  const html = renderToStaticMarkup(
    <SessionDetailHeader
      detail={{ ...detail, summary: detail.title }}
      headingRef={{ current: null }}
      actionBusyId={null}
      actionError={null}
      onRefresh={() => undefined}
      onAction={() => undefined}
    />,
  );

  assert.doesNotMatch(html, /Back to Activity|session-detail-breadcrumbs/);
  assert.match(html, /<h1 id="session-detail-heading" tabindex="-1">Build provider-neutral session detail<\/h1>/);
  // The summary repeated the title, so it is not printed a second time.
  const titleCount = html.split("Build provider-neutral session detail").length - 1;
  assert.equal(titleCount, 1);
  assert.match(html, /Open in Codex/);
  assert.match(html, /aria-label="Refresh session detail"/);
  assert.match(html, /#aaaaaaa/);
  assert.match(html, />11m</);
  assert.match(html, />1\.06M\+</);
  // `Started` and `Ended` spelled the same date twice; one range replaces both.
  assert.doesNotMatch(html, />Started<|>Ended</);

  const distinct = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      headingRef={{ current: null }}
      actionBusyId={null}
      actionError={null}
      onRefresh={() => undefined}
      onAction={() => undefined}
    />,
  );
  assert.match(distinct, /Expose the durable conversation/);
});

test("the timeline states the day once and gives every turn a clock and a rail", () => {
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

  // One divider for four same-day events, where each row used to carry a date.
  assert.equal(html.split("session-detail-day-divider").length - 1, 1);
  assert.equal(html.split("session-detail-event-gutter").length - 1, 4);
  assert.equal(html.split("session-detail-event-rail").length - 1, 4);
  assert.match(html, /class="session-detail-event-copy session-detail-prompt-card"/);
  assert.match(html, /class="session-detail-event-copy session-detail-response-prose"/);
  assert.match(html, /2 tool calls/);
  assert.match(html, new RegExp(`id="${eventAnchorId("tool-1")}"`));

  // Who spoke, not what the schema calls the row.
  assert.match(html, /<h3>You<\/h3>/);
  assert.match(html, /<h3>Codex<\/h3>/);
  assert.doesNotMatch(html, /<h3>Prompt<\/h3>|<h3>Response<\/h3>/);

  // The eyebrow that restated the heading, and the count that contradicted the
  // filter panel, are both gone.
  assert.doesNotMatch(html, /Chronological record|session-detail-section-heading|loaded<\/code>/);
});

test("the gap between turns is shown, and only when it is worth showing", () => {
  assert.equal(formatSessionGap(START, START + 30), null);
  assert.equal(formatSessionGap(START, START + 300), "+5m");
  assert.equal(formatSessionGap(START, START + 7_200), "+2h");
  assert.equal(formatSessionGap(null, START), null);

  const html = renderToStaticMarkup(
    <SessionDetailEvent
      event={events[1]}
      providerDisplayName="Codex"
      previousTimestamp={START}
      expandedToolIds={new Set()}
      toolDetails={{}}
      onToolExpandedChange={() => undefined}
      onTurnToolExpandedChange={() => undefined}
      onLoadMoreTurnTools={() => undefined}
    />,
  );
  assert.match(html, /<span class="session-detail-event-gap">\+5m<\/span>/);
  assert.equal(formatSessionDuration(START, START + 660), "11m");
  assert.equal(formatSessionDuration(START, START + 45), "45s");
  assert.equal(formatSessionDuration(START, START + 3_900), "1h 5m");
  assert.equal(formatSessionDuration(START, null), null);
  assert.match(formatSessionRange(START, START + 660) ?? "", /·.*→/);
});

test("events split into day groups, carrying undated rows with the group they arrive in", () => {
  const nextDay: SessionTimelineEvent = { ...events[0], event_id: "prompt-2", ordinal: 9, timestamp: START + 90_000 };
  const undated: SessionTimelineEvent = { ...events[1], event_id: "response-2", ordinal: 10, timestamp: null };
  const groups = groupSessionEventsByDay([...events, nextDay, undated]);
  assert.equal(groups.length, 2);
  assert.deepEqual(groups[0].events.map((event) => event.event_id), [
    "prompt-1", "response-1", "tool-1", "intermediate-1",
  ]);
  assert.deepEqual(groups[1].events.map((event) => event.event_id), ["prompt-2", "response-2"]);
});

test("a tool row leads with the command and shows status only when it is not ok", () => {
  const ok = renderToStaticMarkup(
    <SessionDetailToolCall event={toolEvent} expanded={false} onExpandedChange={() => undefined} />,
  );
  assert.match(ok, /<strong>exec<\/strong>/);
  assert.match(ok, /sed -n &#x27;241,520p&#x27; skills\/imagegen\/SKILL\.md/);
  // `completed` on every row said nothing 24 times over, and the category chip
  // said `Other` just as often.
  assert.doesNotMatch(ok, /session-detail-tool-status|session-detail-tool-kind|>Read</);
  // The per-row timestamp repeated the minute already printed on the turn.
  assert.doesNotMatch(ok, /<time/);
  assert.doesNotMatch(ok, /<details[^>]* open/);

  const failed = renderToStaticMarkup(
    <SessionDetailToolCall event={failedToolEvent} expanded={false} onExpandedChange={() => undefined} />,
  );
  assert.match(failed, /<span class="session-detail-tool-status status-failed">failed<\/span>/);
});

test("a row shows the command, not the wrapper the agent wrote around it", () => {
  // What Codex actually stores. Printed verbatim every row opened with the
  // same 34 characters and truncation ate the part that told them apart.
  assert.equal(
    toolCommandLabel(`const r = await tools.exec_command({cmd:"sed -n '1,240p' <provider-home>/skills/SKILL.md","workdir":".","yield_time_ms":10000});\ntext(r.output)`),
    "sed -n '1,240p' <provider-home>/skills/SKILL.md",
  );
  // Escapes inside the command survive the lift.
  assert.equal(
    toolCommandLabel(`await tools.exec_command({cmd:"grep -n \\"needle\\" src/app.ts"})`),
    'grep -n "needle" src/app.ts',
  );
  // A bare JSON argument object yields whatever the call is about.
  assert.equal(
    toolCommandLabel('{"prompt":"frame 03 · pelican side view","size":"1024x1024"}'),
    "frame 03 · pelican side view",
  );
  assert.equal(toolCommandLabel('{"file_path":"src/main.tsx","limit":40}'), "src/main.tsx");
  // Anything unrecognised is left exactly as it is, minus trailing lines.
  assert.equal(toolCommandLabel("npm run build"), "npm run build");
  assert.equal(toolCommandLabel("first line\nsecond line"), "first line");
  assert.equal(toolCommandLabel('{"unknown_key":"value"}'), '{"unknown_key":"value"}');
  assert.equal(toolCommandLabel(""), "");

  const html = renderToStaticMarkup(
    <SessionDetailToolCall
      event={{
        ...toolEvent,
        tool: {
          ...toolEvent.tool!,
          summary: `const r = await tools.exec_command({cmd:"python3 render.py","workdir":"."});`,
        },
      }}
      expanded={false}
      onExpandedChange={() => undefined}
    />,
  );
  assert.match(html, /class="session-detail-tool-summary" title="[^"]*exec_command[^"]*">python3 render\.py</);
  // The original is still reachable — as the row's tooltip, and in Arguments.
  assert.doesNotMatch(html, />const r = await/);
});

test("payloads share one bounded tabbed pane and stay escaped", () => {
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
          arguments_truncated: false,
          result: "Read 42 lines",
          result_truncated: true,
          affected_paths: ["src/main.tsx"],
        },
      }}
      onExpandedChange={() => undefined}
    />,
  );
  assert.match(expanded, /<details[^>]* open=""/);
  assert.match(expanded, /role="tablist"/);
  // Output leads, because the result is what you opened the row to read.
  assert.match(expanded, /is-on" aria-selected="true">Output</);
  assert.match(expanded, />Arguments</);
  assert.match(expanded, />Paths</);
  assert.match(expanded, /class="session-detail-payload-body"><code>Read 42 lines<\/code>/);
  assert.doesNotMatch(expanded, /<script>/);
  assert.match(expanded, /aria-label="Copy to clipboard"/);
  // The notice belongs to the pane on screen: Output is the truncated one here.
  assert.match(expanded, /Bounded preview · additional content omitted/);

  const whole = renderToStaticMarkup(
    <SessionDetailToolCall
      event={toolEvent}
      expanded
      detailState={{
        loading: false,
        error: null,
        detail: {
          event_id: toolEvent.event_id,
          arguments: null,
          arguments_truncated: false,
          result: "Read 42 lines",
          result_truncated: false,
          affected_paths: [],
        },
      }}
      onExpandedChange={() => undefined}
    />,
  );
  assert.doesNotMatch(whole, /Bounded preview/);
  // No arguments, so no Arguments tab. Paths survives: an empty list on the
  // detail falls back to the preview's own paths.
  assert.doesNotMatch(whole, />Arguments</);
  assert.match(whole, />Paths</);
});

test("a stored transport payload is unwrapped, with the raw value one click away", () => {
  const envelope = JSON.stringify([
    { type: "input_text", text: "Script completed\nWall time 0.1 seconds" },
    { type: "input_text", text: "## Examples\nUse case: product-mockup" },
  ]);
  const lifted = readableToolPayload(envelope);
  assert.equal(lifted.text, "Script completed\nWall time 0.1 seconds\n\n## Examples\nUse case: product-mockup");
  assert.equal(lifted.raw, envelope);
  assert.equal(lifted.transformed, true);
  assert.doesNotMatch(lifted.text, /\\n/);

  // A bare string with escaped newlines is unescaped in place.
  const escaped = readableToolPayload("line one\\nline two\\ttabbed");
  assert.equal(escaped.text, "line one\nline two\ttabbed");
  assert.equal(escaped.transformed, true);

  // Plain text is left exactly as it is, and the raw toggle stays away.
  const plain = readableToolPayload("already readable");
  assert.equal(plain.text, "already readable");
  assert.equal(plain.transformed, false);

  // A JSON object is pretty-printed rather than dumped on one line.
  const object = readableToolPayload('{"cmd":"ls","workdir":"."}');
  assert.match(object.text, /^\{\n {2}"cmd": "ls",\n {2}"workdir": "\."\n\}$/);
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

test("timeline page merging is chronological, de-duplicated, and counts keep their quality", () => {
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
  const merged = mergeSessionTimelinePages(mergeSessionTimelinePages([], first), second);
  assert.deepEqual(merged.map((event) => event.event_id), ["prompt-1", "response-1", "intermediate-1"]);
  assert.equal(formatSessionCount({ value: 4, quality: "lower_bound" }), "4+");
  assert.equal(formatSessionCount({ quality: "unsupported" }), "Unavailable");
  assert.equal(formatCompactCount({ value: 1_063_252, quality: "exact" }), "1.06M");
  assert.equal(formatCompactCount({ value: 18_200, quality: "lower_bound" }), "18K+");
  assert.equal(formatCompactCount({ value: 940, quality: "exact" }), "940");
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

test("every icon on the page sits on a size the 24-unit grid divides evenly", () => {
  // crispEdges rounds rather than antialiases, so a stem holds one width only
  // when size x DPR / 24 is a whole number — 12 or 24 on a Retina display.
  // 13/14/17/18 render the same shape with mixed 1px and 2px stems.
  const CLEAN = new Set([12, 24]);
  const files = [
    "SessionDetailEvent.tsx",
    "SessionDetailHeader.tsx",
    "SessionDetailPage.tsx",
    "SessionDetailTimeline.tsx",
    "SessionDetailToolCall.tsx",
    "SessionDetailToolbar.tsx",
  ];
  const offGrid: string[] = [];
  for (const file of files) {
    const source = readFileSync(`src/components/project-sync/session-detail/${file}`, "utf8");
    for (const match of source.matchAll(/size=\{(\d+)\}/g)) {
      if (!CLEAN.has(Number(match[1]))) offGrid.push(`${file}: size={${match[1]}}`);
    }
  }
  assert.deepEqual(offGrid, []);

  const icons = readFileSync("src/components/Icons.tsx", "utf8");
  assert.match(icons, /viewBox="0 0 24 24"/);
  assert.match(icons, /shapeRendering="crispEdges"/);

  // A declared size only holds if flex cannot shrink the mark: without this a
  // 12px chevron rendered at 5.4px inside the tool-call summary.
  assert.match(
    readFileSync("src/App.css", "utf8"),
    /\.session-detail-page svg \{\s*flex: 0 0 auto;\s*\}/,
  );
  // Only the brand mark is exempt from the pixel-fill convention.
  const nonPixel = [...icons.matchAll(/\{name === "([a-z-]+)" && \(([\s\S]*?)\n {6}\)\}/g)]
    .filter(([, , body]) => !body.includes("icon-pixel-fill"))
    .map(([, name]) => name);
  assert.deepEqual(nonPixel, ["claude"]);
});

test("the stylesheets encode one column, a measured turn grid, and a bounded pane", () => {
  const baseCss = readFileSync("src/App.css", "utf8");
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");

  // The two-column grid and every inspector rule left with the rail.
  assert.doesNotMatch(baseCss, /grid-template-columns: minmax\(0, 1fr\) minmax\(240px, 284px\)/);
  assert.doesNotMatch(baseCss, /session-detail-inspector|session-detail-filter-row|session-detail-checkpoint-jump/);
  assert.doesNotMatch(retroCss, /session-detail-inspector|session-detail-filter-row/);

  assert.match(baseCss, /\.session-detail-event \{[^}]*grid-template-columns: 58px 3px minmax\(0, 1fr\)/s);
  assert.match(baseCss, /\.session-detail-event-copy \{[^}]*max-width: 68ch/s);
  assert.match(baseCss, /\.session-detail-payload-body \{[^}]*max-height: 240px/s);

  // The payload block speaks the app's own vocabulary: a tab is a tile joined
  // to the panel, not an underline indicator, and its buttons are square
  // keycaps rather than rounded ghosts.
  assert.match(baseCss, /\.session-detail-payload-tabs \{[^}]*margin-bottom: -1px/s);
  assert.match(baseCss, /\.session-detail-payload-tab \{[^}]*border-bottom: 0;[^}]*border-radius: 0/s);
  assert.match(baseCss, /\.session-detail-payload-tab\.is-on \{[^}]*border-color: var\(--border-mid\)/s);
  // 24px is the minimum target size (WCAG 2.2 AA 2.5.8) and a whole multiple
  // of the 24-unit icon grid; --border was too faint to read as an edge.
  assert.match(baseCss, /\.session-detail-payload-action \{[^}]*width: 24px;\s*height: 24px;/s);
  assert.match(baseCss, /\.session-detail-payload-action \{[^}]*border: 1px solid var\(--button-border\)/s);
  assert.match(baseCss, /\.session-detail-payload-action \{[^}]*border-radius: 0/s);
  assert.match(baseCss, /\.session-detail-payload-tab \{[^}]*min-height: 24px/s);
  // Symmetric: the panel lines up with the rows instead of hanging inside them.
  assert.match(baseCss, /\.session-detail-tool-body \{[^}]*padding: 7px 9px 9px;/s);
  assert.match(baseCss, /\.session-detail-payload-action:active \{\s*transform: translate\(1px, 1px\);/);
  assert.doesNotMatch(baseCss, /\.session-detail-payload-tab\.is-on \{[^}]*border-bottom-color/s);
  // The gap has to be zero or the tab cannot sit on the panel it fronts.
  assert.match(baseCss, /\.session-detail-tool-payload \{[^}]*gap: 0;/s);

  // The page-scoped `font: inherit` reset outranked the pixel-face rule on
  // specificity, so a tab label came out in the system sans. The global reset
  // already covers these controls.
  assert.doesNotMatch(baseCss, /\.session-detail-page button,\s*\.session-detail-page select/);
  // The surviving reset is an element selector, so a class always outranks it.
  assert.match(retroCss, /\nbutton,\ninput,\nselect,\ntextarea,\nsummary \{\n  font-family: inherit;/);
  assert.match(retroCss, /\.session-detail-payload-tab \{[\s\S]{0,200}?font-family: var\(--font-pixel\)/);
  assert.match(baseCss, /@container session-detail-layout \(max-width: 720px\)/);
  assert.match(baseCss, /@container session-detail-layout \(max-width: 500px\)/);
  assert.match(baseCss, /\.session-detail-prompt-card[\s\S]*background: var\(--surface-card\)/);
  assert.match(baseCss, /\.session-detail-response-prose[\s\S]*background: transparent/);

  // The speaker label mixes the body colour into the accent so a 10px label
  // clears 4.5:1 on light paper; the rail beside it keeps the pure hue.
  assert.match(baseCss, /\.event-prompt \.session-detail-event-heading h3 \{\s*color: color-mix\(in srgb, var\(--red\) 85%, var\(--text-0\)\);/);
  assert.match(baseCss, /\.event-prompt \.session-detail-event-rail \{ background: var\(--red\); \}/);

  // Pixel chrome, mono evidence.
  assert.match(retroCss, /\.session-detail-day-divider > span,[\s\S]*?font-family: var\(--font-pixel\)/);
  assert.match(retroCss, /\.session-detail-payload-body,[\s\S]*?font-family: var\(--font-mono\)/);
});
