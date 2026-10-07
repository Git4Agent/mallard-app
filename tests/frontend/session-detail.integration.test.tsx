import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  ProjectSessionDetail,
  ProjectSessionTimelinePage,
  SessionTimelineEvent,
} from "../../src/types";
import { SessionDetailMarkdown } from "../../src/components/project-sync/session-detail/SessionDetailEvent";
import { mermaidSourceProblem, readMermaidResult } from "../../src/components/mermaid/mermaidModel";
import type { SessionBackTarget } from "../../src/components/project-sync/session-detail/SessionDetailBack";
import SessionDetailHeader from "../../src/components/project-sync/session-detail/SessionDetailHeader";
import SessionDetailPage from "../../src/components/project-sync/session-detail/SessionDetailPage";
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
  formatSessionLengthDetail,
  formatSessionStart,
  groupSessionEventsByDay,
  includedSessionEventKinds,
  mergeSessionTimelinePages,
  projectMarkdownDisplayPath,
  readableToolPayload,
  sessionDetailFilterToggles,
  sessionEventCoverage,
  sessionPushFacts,
  sessionToolDescriptor,
  sessionTurnDurationLabel,
  sessionTurnSpans,
  toolCommandLabel,
} from "../../src/components/project-sync/session-detail/sessionDetailModel";
import { safeMarkdownUrl, safeRelativeMarkdownPath } from "../../src/components/markdown/markdownUrl";

import { diagram as suppliedFlowchart, fenced } from "../../poc/mermaid-flowcharts/fixtures";

const exact = (value: number) => ({ value, quality: "exact" as const });

test("session Markdown renders TeX and dollar-delimited inline and display math", () => {
  for (const text of [String.raw`Time \(\Delta\tau\).`, String.raw`Time $\Delta\tau$.`]) {
    const html = renderToStaticMarkup(<SessionDetailMarkdown text={text} />);
    assert.match(html, /class="katex"/);
    assert.match(html, /<math xmlns="http:\/\/www.w3.org\/1998\/Math\/MathML"/);
    assert.doesNotMatch(html, /katex-display|katex-error/);
  }
  const formula = String.raw`\beta=\frac vc,\qquad \gamma=\frac{1}{\sqrt{1-\beta^2}}`;
  for (const text of [
    `\\[ ${formula} \\]`,
    `\\[\n${formula}\n\\]`,
    `$$\n${formula}\n$$`,
    `\`\`\`math\n${formula}\n\`\`\``,
  ]) {
    const html = renderToStaticMarkup(<SessionDetailMarkdown text={text} />);
    assert.match(html, /class="katex-display"/);
    assert.match(html, /<mfrac>/);
    assert.match(html, /<msqrt>/);
    assert.doesNotMatch(html, /katex-error|<pre>|<em>/);
  }
});

test("session Markdown protects underbraces and multiline math in lists and quotes", () => {
  const formula = String.raw`\underbrace{\Delta t=\gamma\,\Delta\tau}_{\text{time dilation}}, \qquad \underbrace{L=\frac{L_0}{\gamma}}_{\text{length contraction}}`;
  for (const text of [
    `\\[ ${formula} \\]`,
    `> \\[\n> ${formula}\n> \\]`,
    `- \\[\n  ${formula}\n  \\]`,
    `> - \\[\n>   ${formula}\n>   \\]`,
  ]) {
    const html = renderToStaticMarkup(<SessionDetailMarkdown text={text} />);
    assert.match(html, /class="katex-display"/);
    assert.match(html, /<munder>/);
    assert.match(html, /time dilation/);
    assert.match(html, /length contraction/);
    assert.doesNotMatch(html, /katex-error|<em>/);
  }
  const aligned = String.raw`\[
\begin{aligned}
\Delta x' &= \gamma(\Delta x-v\Delta t) \\
\Delta t' &= \gamma(\Delta t-v\Delta x/c^2)
\end{aligned}
\]`;
  const html = renderToStaticMarkup(<SessionDetailMarkdown text={aligned.replace(/\n/g, "\r\n")} />);
  assert.match(html, /<mtable/);
  assert.doesNotMatch(html, /katex-error/);
});

test("session Markdown keeps literal code, escaped delimiters, and currency readable", () => {
  for (const text of [
    "`\\(x_0\\)` and `\\[x_0\\]` and `$x_0$`",
    "```text\n\\[x_0\\]\n$x_0$\n```",
    "> ```latex\n> \\[x_0\\]\n> ```",
    "    \\[x_0\\]",
    String.raw`\\(x_0\\) and \\[x_0\\]`,
    String.raw`Costs \$5 and \$10.`,
    "Costs $5.",
    String.raw`Incomplete \(x_0 or \[x_0`,
  ]) {
    const html = renderToStaticMarkup(<SessionDetailMarkdown text={text} />);
    assert.doesNotMatch(html, /class="katex|<math/);
  }
  const html = renderToStaticMarkup(<SessionDetailMarkdown text={'[Formula](https://example.com/\\(x\\))'} />);
  assert.match(html, /href="https:\/\/example.com\/\(x\)"/);
  assert.doesNotMatch(html, /class="katex/);
});

test("session math errors remain local and untrusted commands cannot create links or images", () => {
  for (const formula of [
    String.raw`\frac{1}{`,
    String.raw`\def\loop{\loop}\loop`,
  ]) {
    const html = renderToStaticMarkup(<SessionDetailMarkdown text={`Before \\(${formula}\\) after **still readable**.`} />);
    assert.match(html, /class="katex-error"/);
    assert.match(html, /Before /);
    assert.match(html, /after <strong>still readable<\/strong>/);
  }
  const html = renderToStaticMarkup(<SessionDetailMarkdown text={String.raw`\(\href{https://example.com}{link}\) \(\includegraphics{https://example.com/image.png}\)`} />);
  assert.doesNotMatch(html, /<a\b|<img\b|<script\b|<iframe\b/);
  const bounded = renderToStaticMarkup(<SessionDetailMarkdown text={String.raw`\(\rule{500em}{500em}\)`} />);
  assert.doesNotMatch(bounded, /(?:width|height|border-[^:]*):500em/);
  assert.match(bounded, /25em/);
});

test("session Markdown recognizes closed Mermaid fences in quotes and lists", () => {
  for (const text of [
    "```mermaid\nflowchart TD\nA-->B\n```",
    "~~~~mermaid\ngraph LR\nA-->B\n~~~~~",
    "> ```mermaid\n> graph LR\n> A-->B\n>```",
    "- ```mermaid\n  flowchart TD\n  A-->B\n  ```",
    "> - ```mermaid\n>   flowchart LR\n>   A-->B\n>   ```",
    "```mermaid\r\nflowchart TD\r\nA-->B\r\n```",
  ]) {
    const html = renderToStaticMarkup(<SessionDetailMarkdown text={text} />);
    assert.match(html, /class="mermaid-flowchart"/);
    assert.match(html, /data-status="loading"/);
    assert.match(html, /<summary>Source<\/summary>/);
    assert.doesNotMatch(html, /incomplete|<svg|<iframe|<script/);
  }
});

test("session Markdown retains source controls for incomplete and unsupported diagrams", () => {
  for (const text of [
    "```mermaid\nflowchart TD\nA-->B",
    "````mermaid\nflowchart TD\nA-->B\n```",
    "```mermaid\nflowchart TD\nA-->B\n    ```",
    "  ```mermaid\nflowchart TD\nA-->B\n    ```",
    "> ```mermaid\n> flowchart TD\n> A-->B",
    "- ```mermaid\n  flowchart TD\n  A-->B",
  ]) {
    const html = renderToStaticMarkup(<SessionDetailMarkdown text={text} />);
    assert.match(html, /data-status="error"/);
    assert.match(html, /fence is incomplete/);
    assert.match(html, /<summary>Source<\/summary>/);
  }
  const unsupported = renderToStaticMarkup(<SessionDetailMarkdown text={"```mermaid\nsequenceDiagram\nA->>B: message\n```"} />);
  assert.match(unsupported, /Only Mermaid flowcharts are supported/);
  const ordinary = renderToStaticMarkup(<SessionDetailMarkdown text={"```javascript\nconsole.log('code');\n```"} />);
  assert.match(ordinary, /language-javascript/);
  assert.doesNotMatch(ordinary, /mermaid-flowchart/);
});

test("the supplied six-node flowchart has a complete fence; css stays code", () => {
  const html = renderToStaticMarkup(<SessionDetailMarkdown text={fenced(suppliedFlowchart)} />);
  assert.match(html, /data-status="loading"/);
  assert.doesNotMatch(html, /incomplete/);
  const css = renderToStaticMarkup(<SessionDetailMarkdown text={fenced(suppliedFlowchart, "css")} />);
  assert.match(css, /language-css/);
  assert.doesNotMatch(css, /mermaid-flowchart/);
});

test("Mermaid source bounds use UTF-8 bytes and parser replies must keep the isolation settings", () => {
  assert.equal(mermaidSourceProblem("flowchart TD\nA-->B"), null);
  assert.equal(mermaidSourceProblem("graph LR\nA-->B"), null);
  assert.match(mermaidSourceProblem("flowchart TD\n%% " + "🦆".repeat(8192))!, /32 KiB/);
  const result = { svg: '<svg xmlns="http://www.w3.org/2000/svg"/>', strict: true, htmlLabels: false, tauriPresent: false };
  assert.deepEqual(readMermaidResult(result), { svg: result.svg });
  for (const unsafe of [null, {}, { ...result, tauriPresent: true }, { ...result, strict: false }, { ...result, htmlLabels: true }, { ...result, svg: "x".repeat(1024 * 1024 + 1) }]) {
    assert.equal(readMermaidResult(unsafe).svg, undefined);
    assert.ok(readMermaidResult(unsafe).error);
  }
});

// 2026-08-06T07:06:40Z and onward, so the day divider and the card clock have
// a real day boundary to work with.
const START = 1_786_000_000;

// Where a session's page returns to: the project's page, in the trail's word.
const back: SessionBackTarget = { kind: "section", label: "Git history", onActivate: () => undefined };

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
    agent_calls: exact(1),
  },
  actions: [{
    action_id: "codex.open_in_app",
    kind: "open_in_app",
    label: "Open in Codex",
    enabled: true,
  }],
  snapshot_id: "snapshot-1",
  warnings: [],
};

const toolEvent: SessionTimelineEvent = {
  event_id: "tool-1",
  ordinal: 2,
  timestamp: START + 150,
  kind: "tool_call",
  quality: "exact",
  turn_id: "turn-1",
  parent_event_id: "prompt-1",
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
  ordinal: 3,
  timestamp: START + 200,
  tool: { ...toolEvent.tool!, provider_call_id: "call-2", status: "failed", summary: "python3 render.py" },
};

// Chronological, the way the feed now renders: the call sits between the
// prompt that caused it and the response that follows it.
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
  toolEvent,
  {
    event_id: "response-1",
    ordinal: 4,
    timestamp: START + 300,
    kind: "response",
    quality: "exact",
    turn_id: "turn-1",
    text: "I found the relevant boundary and kept the change scoped.",
  },
  {
    event_id: "intermediate-1",
    ordinal: 5,
    timestamp: START + 330,
    kind: "intermediate_step",
    quality: "inferred",
    turn_id: "turn-1",
    text: "Running the focused checks now.",
  },
];

const checkpointEvent: SessionTimelineEvent = {
  event_id: "checkpoint-1",
  ordinal: 6,
  timestamp: START + 400,
  kind: "checkpoint",
  quality: "exact",
  turn_id: "turn-1",
  checkpoint: {
    sha: "7350274" + "b".repeat(33),
    short_sha: "7350274",
    subject: "Add pelican GIF render pipeline",
    committed_at: START + 400,
    branch: "main",
  },
};

test("Codex and Claude session prompts, replies, and steps use the same math renderer", () => {
  for (const provider of ["codex", "claude"]) {
    const mathEvents: SessionTimelineEvent[] = (["prompt", "response", "intermediate_step"] as const).map((kind, index) => ({
      event_id: `${provider}-math-${index}`,
      ordinal: index,
      timestamp: START + index,
      kind,
      quality: "exact",
      text: String.raw`\[\Delta t=\gamma\Delta\tau\]`,
    }));
    const html = renderTimeline(mathEvents, { provider, providerDisplayName: provider });
    assert.equal((html.match(/class="katex-display"/g) ?? []).length, 3);
    assert.doesNotMatch(html, /katex-error/);
  }
});

function renderTimeline(
  timelineEvents: SessionTimelineEvent[],
  overrides?: { provider?: string; providerDisplayName?: string; repositoryUrl?: string | null },
): string {
  return renderToStaticMarkup(
    <SessionDetailTimeline
      events={timelineEvents}
      provider={overrides?.provider ?? "codex"}
      providerDisplayName={overrides?.providerDisplayName ?? "Codex"}
      loading={false}
      loadingMore={false}
      nextCursor={null}
      error={null}
      expandedToolIds={new Set()}
      toolDetails={{}}
      onRetry={() => undefined}
      onLoadMore={() => undefined}
      onToolExpandedChange={() => undefined}
      onOpenProjectPath={() => undefined}
      projectImages={{}}
      onLoadProjectImage={() => undefined}
      projectRoot="/Users/example/HorrorGame"
      repositoryUrl={overrides?.repositoryUrl ?? null}
      hasFilterToggles
    />,
  );
}

test("filters carry event kinds only, and the conversation is what opens by default", () => {
  const filters = createDefaultSessionDetailFilters(detail.event_counts);
  // Steps, tool calls, and agents require selection.
  assert.deepEqual(includedSessionEventKinds(filters), ["prompt", "response", "checkpoint"]);
  assert.equal(filters.agents, false);
  assert.equal(filters.toolCalls, false);
  assert.equal(filters.intermediateSteps, false);
  assert.equal("toolCategories" in filters, false);

  // The query still names every category, because sorting by category is gone
  // from the UI, not from the command — and nothing on the page asks for a
  // single turn's calls any more.
  const pageSource = readFileSync(
    "src/components/project-sync/session-detail/SessionDetailPage.tsx",
    "utf8",
  );
  assert.match(pageSource, /included_tool_categories: \[\.\.\.ALL_SESSION_TOOL_CATEGORIES\]/);
  assert.doesNotMatch(pageSource, /SessionDetailInspector|selectedCheckpointId|toggleAllLoadedTools/);
  assert.doesNotMatch(pageSource, /turnToolStates|loadTurnTools|SessionTurnToolState/);
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
    "agents",
  ]);
  assert.deepEqual(toggles.map((toggle) => toggle.label), [
    "Prompts",
    "Replies",
    "Steps",
    "Tool calls",
    "Agents",
  ]);

  const coverage = sessionEventCoverage(detail.event_counts);
  assert.equal(coverage.total, 2 + 3 + 1 + 0 + 4 + 1);
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
  assert.match(html, /5 of 11\+ events/);
  assert.match(html, /aria-pressed="true"[^>]*>[^<]*<span class="session-detail-filter-box"[^>]*><\/span><span class="session-detail-filter-name">Prompts/);
  assert.match(html, /<span class="session-detail-filter-name">Tool calls<\/span><span class="session-detail-filter-count quality-exact">4<\/span>/);
  // No checkpoint control and no second panel of any kind.
  assert.doesNotMatch(html, /Checkpoints|No checkpoints|Expand loaded tool calls|<fieldset|<select/);
});

test("the header is one strip: hidden h1, the way back, facts as chips, verbs on the right", () => {
  const html = renderToStaticMarkup(
    <SessionDetailHeader
      detail={{ ...detail, summary: detail.title }}
      back={back}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );

  assert.doesNotMatch(html, /session-detail-breadcrumbs/);
  // The visible title is the path bar's leaf crumb; the h1 stays for structure
  // and as the focus landing, screen-reader-only.
  assert.match(html, /<h1 id="session-detail-heading" class="sync-visually-hidden" tabindex="-1">Build provider-neutral session detail<\/h1>/);
  const titleCount = html.split("Build provider-neutral session detail").length - 1;
  assert.equal(titleCount, 1);

  // Facts state themselves: glyph chips, no caption row, full values in
  // tooltips — the sha bare and 7 chars, the way git prints it.
  assert.match(html, /class="session-detail-chip" title="Recorded branch main"/);
  assert.match(html, /data-icon="git-branch"/);
  assert.match(html, /data-icon="git-commit"/);
  assert.match(html, new RegExp(`title="${"a".repeat(40)}"`));
  assert.match(html, />aaaaaaa</);
  assert.match(html, />1\.06M\+<\/code> tokens/);
  assert.doesNotMatch(html, />Started<|>Ended<|>Branch<|>Commit<|>Ran<|>Tokens</);

  // The stamp is two chips: when it began, and how long it took. Neither the
  // end time nor the arrow that pointed at it appears on the strip.
  assert.match(html, /<span class="session-detail-chip"><time [^>]*="2026-08-06T[\d:.]+Z">\w+ \d+, 2026 · \d{2}:\d{2}<\/time><\/span>/);
  assert.match(html, /class="session-detail-chip" title="Ran 11m, ending \d{2}:\d{2}"><svg[^>]*data-icon="clock"[\s\S]*?<b>11m<\/b>/);
  assert.doesNotMatch(html, /→/);

  // Provider identity is the mark on the leaf crumb and on the launch button
  // itself; the text chip, the Active chip and the second refresh glyph are
  // gone. The launch pair speaks the activity rows' icon vocabulary — the
  // provider's own mark for open-in-app, the label demoted to tooltip and
  // accessible name, no button text.
  assert.match(html, /session-detail-launch[^>]*title="Open in Codex"[^>]*aria-label="Open in Codex"/);
  assert.match(html, /session-detail-launch[^>]*>.*data-icon="openai"/);
  assert.doesNotMatch(html, />Open in Codex</);
  assert.doesNotMatch(html, /session-detail-provider-badge|session-detail-active-badge|session-detail-refresh|Refresh session detail/);

  const distinct = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      back={back}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );
  // Transcript-derived summaries belong in the timeline, not above it.
  assert.doesNotMatch(distinct, /session-detail-summary|Expose the durable conversation/);
});

// What Push cannot do and how the transcript was read are labels on the
// strip, not boxes: a hold the user clears leads the verbs, under Push, and
// the transcript's notes are one chip at the end of the facts.
test("a Push hold and the transcript notes are chips, never boxes", () => {
  const html = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      back={back}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={{ tone: "info", label: "Pull first", detail: "Storage has changes. Review Pull before pushing this session." }}
      notes={[
        { code: "incomplete_tool_call", severity: "info", message: "Tool calls marked incomplete never recorded a result. They were most likely interrupted." },
        { code: "malformed_record", severity: "warning", message: "Some lines of the session file couldn't be read and are left out." },
      ]}
      onShowToolCalls={() => undefined}
    />,
  );
  assert.doesNotMatch(html, /class="notice/);
  assert.doesNotMatch(html, /Dismiss/);
  assert.match(
    html,
    /<div class="session-detail-header-actions" aria-label="Session actions"><span class="session-detail-chip session-detail-push-gate info" title="Storage has changes\. Review Pull before pushing this session\."><svg[^>]*data-icon="download"[\s\S]*?<\/svg>Pull first<span class="sync-visually-hidden"> Storage has changes\. Review Pull before pushing this session\.<\/span><\/span>/,
  );
  // The loudest note sets the chip's rung; the count is the face, and the
  // panel is a dialog that is not in the page until it opens.
  assert.match(
    html,
    /<span class="sync-path-popover"><button type="button" class="session-detail-chip session-detail-notes-chip warning" aria-haspopup="dialog" aria-expanded="false" aria-label="2 notes on this session"><svg[^>]*data-icon="alert-triangle"[\s\S]*?<\/svg>2 notes<\/button><\/span>/,
  );
  assert.doesNotMatch(html, /session-detail-notes"|Show tool calls/);
  // Closed, the sentences still stand beside the chip for a screen reader.
  assert.match(html, /<span class="sync-visually-hidden">Note: Tool calls marked incomplete never recorded a result\.[^<]*Warning: Some lines of the session file/);
  // The notes end the facts.
  assert.match(html, /2 notes<\/button><\/span><span class="sync-visually-hidden">[^<]*<\/span><\/div><\/div><div class="session-detail-header-actions"/);

  const quiet = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      back={back}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );
  assert.doesNotMatch(quiet, /session-detail-push-gate|session-detail-notes-chip|sync-path-popover/);

  // Up to date, checking and ready draw nothing on the page: the workspace's
  // Push status box is gone, and the page's own box holds only events.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.doesNotMatch(workspace, /Session Push status/);
  assert.match(workspace, /pushGate=\{workspaceReviewOpen \? null : sessionPushHold\}/);
  assert.match(workspace, /sessionPushGate\(sessionPushEntry, managedCloudKey \? null : sessionComparison\.comparisonError\)/);
  const page = readFileSync("src/components/project-sync/session-detail/SessionDetailPage.tsx", "utf8");
  assert.match(page, /className="session-detail-notices"\s*timing="blocking"/);
});

// The crumb that walks back reads as a place, so the page carries a button
// that reads as one: ahead of the facts, and in the two states that have no
// strip yet. It names where it lands, and the workspace decides where that is.
test("the page's way back leads the strip and both entry states, one level up the trail", () => {
  const header = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      back={back}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );
  assert.match(
    header,
    /<div class="session-detail-strip"><div class="session-detail-lead"><button type="button" class="btn session-detail-back" title="Back to Git history" aria-label="Back to Git history"><svg[^>]*data-icon="chevron-left"[\s\S]*?<\/svg><span>Git history<\/span><\/button><div class="session-detail-chips" aria-label="Session facts">/,
  );
  assert.doesNotMatch(header, /session-detail-back[^>]*disabled/);

  // It waits where the project crumb waits, and from an agent's page it names
  // the parent session: a title, so it leaves the app's display face.
  const waiting = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      back={{ ...back, disabled: true }}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );
  assert.match(waiting, /class="btn session-detail-back" disabled=""/);
  const child = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      back={{ kind: "session", label: "Plan the release", onActivate: () => undefined }}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );
  assert.match(child, /class="btn session-detail-back to-session" title="Back to Plan the release" aria-label="Back to Plan the release"/);

  // A page that is still opening renders the button and nothing else yet: the
  // scaffold holds back for its first 150ms.
  const opening = renderToStaticMarkup(
    <SessionDetailPage
      localProjectId="project-1"
      projectRoot="/tmp/project"
      resourceId={detail.resource_id}
      occurrenceKey={detail.resource_id}
      back={back}
      onHeaderChange={() => undefined}
      pushGate={null}
    />,
  );
  assert.match(
    opening,
    /^<section class="session-detail-page session-detail-entry-state" aria-label="Loading session detail"><button type="button" class="btn session-detail-back" title="Back to Git history"/,
  );
  const pageSource = readFileSync("src/components/project-sync/session-detail/SessionDetailPage.tsx", "utf8");
  assert.equal(pageSource.split("<SessionDetailBack target={props.back} />").length - 1, 2);

  // The workspace hands the page one target: the parent session from an
  // agent's page, otherwise the project's page through the close that restores
  // the list's scroll position and focus. The chunk's own fallback has it too.
  const workspaceSource = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspaceSource, /kind: "session",\s*label: sessionParent\.title,\s*onActivate: \(\) => onOpenSessionDetail\(/);
  assert.match(workspaceSource, /kind: "section",\s*label: workspaceSectionLabel,\s*onActivate: closeSessionDetail,\s*disabled: busy \|\| !!inlineStorageReview,/);
  assert.match(workspaceSource, /<section className="session-detail-page session-detail-entry-state">\s*<SessionDetailBack target=\{sessionBack\} \/>/);
  assert.match(workspaceSource, /back=\{sessionBack\}/);

  // 26px clears the 24px minimum target; the margins give the 4px back, so
  // the strip keeps the chips' height.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.btn\.session-detail-back \{\s*min-height: 26px;\s*flex: 0 0 auto;\s*gap: 3px;\s*margin: -2px 0;/);
  assert.match(css, /\.session-detail-entry-state > \.btn\.session-detail-back \{\s*justify-self: start;\s*margin: -1px 0;/);
});

const agentEvent: SessionTimelineEvent = {
  event_id: "agent-1",
  ordinal: 2,
  timestamp: START + 100,
  kind: "agent_call",
  quality: "exact",
  turn_id: "turn-1",
  parent_event_id: "prompt-1",
  tool: {
    provider_call_id: "task-1",
    native_name: "Task",
    category: "other",
    status: "completed",
    summary: "Research term marking",
    argument_truncated: false,
    result_truncated: false,
    affected_paths: [],
  },
  agent: {
    provider_agent_id: "a1b2c3",
    label: "Research term marking",
    role: "general-purpose",
    model: "claude-opus-5",
    mode: "async",
    status: "completed",
    started_at: START + 100,
    ended_at: START + 931,
    brief: "Research how reading apps mark a target word.",
    brief_truncated: false,
    report: "I have enough. Here are the findings.",
    report_truncated: false,
    transcript_resource_id: "codex:conversation:session-1/agent/a1b2c3",
  },
};

test("a delegation is a tile and a row: AGENT stamp, label, role, length, status only when not done", () => {
  const html = renderTimeline([events[0], agentEvent]);
  // Its own row class, between the punch runs and the cards, and the fork
  // mark on a 24px tile named for the agent.
  assert.match(html, /class="session-detail-row-agent"/);
  assert.match(html, /event-agent-call agent-completed/);
  assert.match(html, /aria-label="Agent · Research term marking · \d{2}:\d{2}"/);
  assert.match(html, /data-icon="git-fork"/);
  // The row: stamp, label, role pill, the background mode, the length.
  assert.match(html, /session-detail-tool-name session-detail-agent-stamp">Agent</);
  assert.match(html, /class="session-detail-agent-label" title="Research term marking">Research term marking</);
  assert.match(html, /class="session-detail-agent-pill">general-purpose</);
  assert.match(html, /class="session-detail-agent-pill mode">async</);
  // Length and chevron share one end cluster, so the length sits against the
  // chevron on every row rather than splitting the free space with it.
  assert.match(html, /class="session-detail-agent-end"><span class="session-detail-agent-meta"><span class="session-detail-agent-length">14m<\/span><\/span><span class="session-detail-tool-chevron"/);
  // Done says nothing; the brief and report wait behind the row.
  assert.doesNotMatch(html, /session-detail-agent-status/);
  assert.doesNotMatch(html, /Open transcript|Here are the findings/);
  // Codex's nickname rows carry the task path beside the name.
  const codexHtml = renderTimeline([
    { ...agentEvent, agent: { ...agentEvent.agent!, label: "Hilbert", task: "inspect_poc", mode: "spawn", role: "explorer" } },
  ]);
  assert.match(codexHtml, /session-detail-agent-label" title="Hilbert">Hilbert<\/span><span class="session-detail-agent-task" title="inspect_poc">inspect_poc</);
  assert.doesNotMatch(codexHtml, /session-detail-agent-pill mode/);
});

test("an agent row gives its free space to one end cluster, and its stamp is as wide as its word", () => {
  // Two auto margins (meta and chevron) split the free space and floated the
  // length mid-row; the stamp had inherited the tool column's 88px.
  const css = readFileSync("src/App.css", "utf8");
  const rule = (selector: string) => css.match(new RegExp(`\\n${selector} \\{([^}]*)\\}`))?.[1] ?? "";
  assert.match(rule("\\.session-detail-agent-end"), /margin-left: auto;/);
  assert.match(rule("\\.session-detail-agent-end > \\.session-detail-tool-chevron"), /margin-left: 0;/);
  // Found before it is searched: a renamed rule must fail here, not pass empty.
  const meta = rule("\\.session-detail-agent-meta");
  assert.match(meta, /font-variant-numeric: tabular-nums;/);
  assert.doesNotMatch(meta, /margin-left/);
  assert.match(rule("\\.session-detail-agent-stamp"), /flex: 0 0 auto;/);
});

test("a run that is not simply done prints its status, and the tile takes its colour", () => {
  for (const [status, label, tone] of [
    ["failed", "failed", "agent-failed"],
    ["running", "running", "agent-running"],
    ["interrupted", "interrupted", "agent-interrupted"],
    ["incomplete", "incomplete", "agent-incomplete"],
    ["unavailable", "no transcript", "agent-unavailable"],
  ] as const) {
    const html = renderTimeline([
      { ...agentEvent, agent: { ...agentEvent.agent!, status, transcript_resource_id: null } },
    ]);
    assert.match(html, new RegExp(`event-agent-call ${tone}`));
    assert.match(html, new RegExp(`class="session-detail-agent-status status-${status}">${label}</span>`));
  }
});

test("agents open with the conversation and count on the header", () => {
  const header = renderToStaticMarkup(
    <SessionDetailHeader
      detail={detail}
      back={back}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );
  assert.match(header, /data-icon="git-fork"[^]*?<code class="quality-exact">1<\/code>agent<\/span>/);
  assert.doesNotMatch(header, /\/agent\//);

  // A child page states what it was, and offers no verbs of its own.
  const childHeader = renderToStaticMarkup(
    <SessionDetailHeader
      detail={{
        ...detail,
        actions: [],
        event_counts: { ...detail.event_counts, agent_calls: exact(0) },
        agent: {
          parent_resource_id: "codex:conversation:session-1",
          parent_title: "Build provider-neutral session detail",
          preview: agentEvent.agent!,
        },
      }}
      back={back}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
      pushGate={null}
      notes={[]}
      onShowToolCalls={null}
    />,
  );
  assert.match(childHeader, /<code>general-purpose<\/code>/);
  assert.match(childHeader, /<code>claude-opus-5<\/code>/);
  assert.match(childHeader, /title="Run async">async</);
  assert.doesNotMatch(childHeader, /agents<\/span>|session-detail-launch/);
});

test("the timeline is a column of tiles for turns and punches for calls, no speaker headings", () => {
  const html = renderTimeline(events);

  // One divider for four same-day events, where each row used to carry a date.
  assert.equal(html.split("session-detail-day-divider").length - 1, 1);
  // Three tiles (prompt, response, step) and one punch (the call).
  assert.equal(html.split("session-detail-event-node\"").length - 1, 3);
  assert.equal(html.split("session-detail-event-tick").length - 1, 1);
  assert.match(html, /class="session-detail-row-tool"/);
  assert.match(html, new RegExp(`id="${eventAnchorId("tool-1")}"`));

  // The tile is the speaker: name · clock on its label, no headings anywhere.
  assert.match(html, /aria-label="You · \d{2}:\d{2}"/);
  assert.match(html, /aria-label="Codex · \d{2}:\d{2}"/);
  assert.match(html, /data-icon="openai"/);
  assert.match(html, /sync-provider-mark codex/);
  assert.doesNotMatch(html, /session-detail-event-heading|<h3>You<\/h3>|<h3>Codex<\/h3>/);

  // The clock gutter and its per-event rail left with the headings.
  assert.doesNotMatch(html, /session-detail-event-gutter|session-detail-event-rail/);

  // You ask in a card: markdown body plus a clock · duration · calls meta line.
  assert.match(html, /class="session-detail-card session-detail-card-prompt"/);
  // The stamp leads the card: clock · duration · calls, then the copy.
  assert.match(html, /class="session-detail-card-head"><time[^>]*>\d{2}:\d{2}<\/time><span>6m<\/span><span>2 calls<\/span><\/div><div class="session-detail-event-copy markdown-copy"/);
  // The reply is a card too, stamped with its clock alone.
  assert.match(html, /class="session-detail-card session-detail-card-reply"><div class="session-detail-card-head"><time[^>]*>\d{2}:\d{2}<\/time><\/div><div class="session-detail-event-copy markdown-copy"/);

  // The eyebrow that restated the heading, and the count that contradicted the
  // filter panel, are both gone.
  assert.doesNotMatch(html, /Chronological record|session-detail-section-heading|loaded<\/code>/);
});

test("the wait between turns is a chip in the node column, and ticks never carry one", () => {
  assert.equal(formatSessionGap(START, START + 30), null);
  assert.equal(formatSessionGap(START, START + 300), "+5m");
  assert.equal(formatSessionGap(START, START + 7_200), "+2h");
  assert.equal(formatSessionGap(null, START), null);

  // response-1 follows tool-1 by 150s; the step follows by 30s and stays quiet.
  const html = renderTimeline(events);
  assert.equal(html.split("session-detail-event-gap").length - 1, 1);
  assert.match(html, /class="session-detail-event-gap"><span>\+3m<\/span><\/div>/);

  // A slow tool call shows no chip — the wait reads as the call's own runtime.
  const slowTool: SessionTimelineEvent = { ...toolEvent, timestamp: START + 600 };
  const toolOnly = renderTimeline([events[0], slowTool]);
  assert.doesNotMatch(toolOnly, /session-detail-event-gap/);

  assert.equal(formatSessionDuration(START, START + 660), "11m");
  assert.equal(formatSessionDuration(START, START + 45), "45s");
  assert.equal(formatSessionDuration(START, START + 3_900), "1h 5m");
  assert.equal(formatSessionDuration(START, null), null);
  // The stamp is a start, and only a start — no end, no arrow. The year is
  // always spelled out, so the chip's shape never depends on today's date.
  assert.equal(formatSessionStart(null), null);
  assert.doesNotMatch(formatSessionStart(START) ?? "", /→/);
  assert.match(formatSessionStart(START) ?? "", /^\w+ \d+, 2026 · \d{2}:\d{2}$/);

  // The end time is off the chip faces, so the length tooltip is where it
  // survives — the start needs none, having nothing left to abbreviate.
  assert.match(formatSessionLengthDetail(START, START + 660) ?? "", /^Ran 11m, ending \d{2}:\d{2}$/);
  // A day boundary in the middle makes the end's own date load-bearing.
  assert.match(formatSessionLengthDetail(START, START + 86_400) ?? "", /ending \w+ \d+, 2026 · \d{2}:\d{2}$/);
  // Running: a distance so far, never an end.
  assert.equal(formatSessionLengthDetail(START, null), null);
});

test("a turn's duration is measured from its loaded extent, never promised", () => {
  // Only the prompt has loaded: the turn has no extent yet.
  const partial = sessionTurnSpans([events[0]]);
  assert.equal(sessionTurnDurationLabel(partial.get("turn-1")), null);

  // The span grows as later pages land.
  const spans = sessionTurnSpans(events);
  assert.deepEqual(spans.get("turn-1"), { started: START, ended: START + 330 });
  assert.equal(sessionTurnDurationLabel(spans.get("turn-1")), "6m");
  assert.equal(sessionTurnDurationLabel(undefined), null);

  // Undated events and events outside any turn contribute nothing.
  const sparse = sessionTurnSpans([
    { ...events[0], timestamp: null },
    { ...events[2], turn_id: null },
  ]);
  assert.equal(sparse.size, 0);

  // A single loaded call still pluralises honestly.
  const single = renderTimeline([{
    ...events[0],
    turn_tool_summary: { turn_id: "turn-1", tool_calls: exact(1) },
  }]);
  assert.match(single, /<span>1 call<\/span>/);
});

test("providers wear their own mark on the tile, with a monogram fallback", () => {
  const known = renderTimeline([events[2]]);
  assert.match(known, /data-icon="openai"/);

  // An unknown provider still renders a speaker, not an empty slot.
  const unknown = renderTimeline([events[2]], { provider: "gemini", providerDisplayName: "Gemini" });
  assert.doesNotMatch(unknown, /data-icon="openai"/);
  assert.match(unknown, /sync-provider-monogram/);
  assert.match(unknown, />G</);
  assert.match(unknown, /aria-label="Gemini · \d{2}:\d{2}"/);
});

test("a checkpoint is a card of recorded fields — sha, subject, branch, clock", () => {
  const html = renderTimeline([checkpointEvent]);
  assert.match(html, /class="session-detail-checkpoint-card"/);
  // The full sha stays reachable as the chip's tooltip.
  assert.match(html, /class="session-detail-checkpoint-sha" title="7350274b+">7350274</);
  assert.match(html, /class="session-detail-checkpoint-subject"[^>]*>Add pelican GIF render pipeline</);
  assert.match(html, /class="session-detail-checkpoint-branch">main</);
  assert.match(html, /class="session-detail-checkpoint-end">.*<time[^>]*>\d{2}:\d{2}<\/time>/);
  assert.match(html, /aria-label="Checkpoint · \d{2}:\d{2}"/);
  // No fabricated diff stats: the provider does not record them.
  assert.doesNotMatch(html, /\+\d+ \/ -\d+/);
});

test("a checkpoint sha opens the commit only where a remote can resolve it", () => {
  const linked = renderTimeline([checkpointEvent], {
    repositoryUrl: "https://github.com/Git4Agent/mallard",
  });
  // Same shape the activity list's commit heading uses: glyph first, then the
  // chip, and the anchor holds the full sha in its title.
  assert.match(
    linked,
    /<a class="session-detail-checkpoint-link" href="https:\/\/github\.com\/Git4Agent\/mallard\/commit\/7350274b+"[^>]*title="Open 7350274b+ on GitHub"[^>]*><svg[^>]*data-icon="external-link"[\s\S]*?<code class="session-detail-checkpoint-sha">7350274<\/code><\/a>/,
  );
  assert.match(linked, /rel="noopener noreferrer"/);

  // No remote, no link: the chip stays exactly as it was, full sha on hover.
  const bare = renderTimeline([checkpointEvent]);
  assert.doesNotMatch(bare, /session-detail-checkpoint-link/);
  assert.match(bare, /<code class="session-detail-checkpoint-sha" title="7350274b+">7350274<\/code>/);
});

test("a matched checkpoint flags itself on the row, not on a line above it", () => {
  // Every checkpoint is matched from Git rather than read out of a transcript,
  // so this flag is the one that always shows. Stacked above the card it made
  // the row 62px where a plain commit reads in 36px.
  const html = renderTimeline([{ ...checkpointEvent, quality: "inferred" }]);
  assert.doesNotMatch(html, /session-detail-event-flags/);
  assert.match(
    html,
    /class="session-detail-checkpoint-end"><span class="session-detail-quality quality-inferred" title="Matched from Git[^"]*">Inferred<span class="sync-visually-hidden"> Matched from Git[^"]*<\/span><\/span><span class="session-detail-checkpoint-branch">main</,
  );

  // Every other kind still flags above its body: their rows have nowhere to
  // put one.
  const response = renderTimeline([{ ...events[2], quality: "inferred" }]);
  assert.match(response, /class="session-detail-event-flags"><span class="session-detail-quality quality-inferred">Inferred</);
});

test("events split into day groups, carrying undated rows with the group they arrive in", () => {
  const nextDay: SessionTimelineEvent = { ...events[0], event_id: "prompt-2", ordinal: 9, timestamp: START + 90_000 };
  const undated: SessionTimelineEvent = { ...events[2], event_id: "response-2", ordinal: 10, timestamp: null };
  const groups = groupSessionEventsByDay([...events, nextDay, undated]);
  assert.equal(groups.length, 2);
  assert.deepEqual(groups[0].events.map((event) => event.event_id), [
    "prompt-1", "tool-1", "response-1", "intermediate-1",
  ]);
  assert.deepEqual(groups[1].events.map((event) => event.event_id), ["prompt-2", "response-2"]);
});

test("a tool row leads with its tinted action, and shows status only when it is not ok", () => {
  const ok = renderToStaticMarkup(
    <SessionDetailToolCall event={toolEvent} expanded={false} onExpandedChange={() => undefined} />,
  );
  // `exec` is not a name anyone reads; the action it performs is.
  assert.match(ok, /<span class="session-detail-tool-name cat-shell">Bash<\/span>/);
  assert.match(ok, /sed -n &#x27;241,520p&#x27; skills\/imagegen\/SKILL\.md/);
  // The chevron closes the row at the far edge, where every disclosure points.
  assert.match(ok, /session-detail-tool-summary[\s\S]*session-detail-tool-chevron/);
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

  // The failure is visible from the node column: the punch takes the failure colour.
  const timeline = renderTimeline([events[0], failedToolEvent]);
  assert.match(timeline, /event-tool-call tool-failed/);
});

test("the action label derives from the wrapped function first, then the recorded name", () => {
  // Codex records `exec` and buckets nearly everything as `other`; the JS
  // wrapper inside the summary is the better witness when it is present.
  assert.deepEqual(
    sessionToolDescriptor({ native_name: "exec", category: "other", summary: "sed -n '1,4p' a.md" }),
    { label: "Bash", category: "shell" },
  );
  assert.deepEqual(
    sessionToolDescriptor({ native_name: "exec", category: "other", summary: "const r = await tools.read_file({path:'src/a.rs'});" }),
    { label: "Read", category: "read" },
  );
  assert.deepEqual(
    sessionToolDescriptor({ native_name: "exec", category: "other", summary: 'await tools.apply_patch({file_path:"src/a.rs"})' }),
    { label: "Edit", category: "file_edit" },
  );
  // A name with no mapping passes through with its recorded category.
  assert.deepEqual(
    sessionToolDescriptor({ native_name: "image_gen", category: "other", summary: '{"prompt":"frame 03"}' }),
    { label: "image_gen", category: "other" },
  );
  // Providers that already record readable names keep them, tint included.
  assert.deepEqual(
    sessionToolDescriptor({ native_name: "Grep", category: "read", summary: "-rn needle src/" }),
    { label: "Grep", category: "read" },
  );
  assert.deepEqual(
    sessionToolDescriptor({ native_name: "Bash", category: "shell", summary: "npm run build" }),
    { label: "Bash", category: "shell" },
  );
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
  // Wrapper arguments are JS, not JSON: single-quoted values lift too — these
  // were the rows that still opened with `const r = await tools.…`.
  assert.equal(
    toolCommandLabel("const r = await tools.read_file({path:'src/a.rs'});"),
    "src/a.rs",
  );
  // A wrapper with no primary argument still sheds the harness noise.
  assert.equal(toolCommandLabel("const r = await tools.list_dir({});"), "list_dir({});");
  // Rows end early rather than running to the chevron: a hard 64-point cap,
  // counted in code points so CJK cannot be split mid-character. The full
  // command survives as the row's tooltip and in Arguments.
  const long = `git -C /Users/peytonmontei/Documents/entire/devenv/entireio/ log --oneline --graph --all`;
  const capped = toolCommandLabel(`const r = await tools.exec_command({cmd:${JSON.stringify(long)}});`);
  assert.equal([...capped].length, 65);
  assert.equal(capped, [...long].slice(0, 64).join("").trimEnd() + "…");
  const cjk = "鹈".repeat(80);
  assert.equal([...toolCommandLabel(cjk)].length, 65);
  assert.match(toolCommandLabel(cjk), /…$/);
  // A bare JSON argument object yields whatever the call is about.
  assert.equal(
    toolCommandLabel('{"prompt":"frame 03 · pelican side view","size":"1024x1024"}'),
    "frame 03 · pelican side view",
  );
  assert.equal(toolCommandLabel('{"file_path":"src/main.tsx","limit":40}'), "src/main.tsx");
  // Anything unrecognised is left exactly as it is; a body that goes on —
  // more lines, or past the cap — says so with an ellipsis.
  assert.equal(toolCommandLabel("npm run build"), "npm run build");
  assert.equal(toolCommandLabel("first line\nsecond line"), "first line…");
  assert.equal(toolCommandLabel('{"unknown_key":"value"}'), '{"unknown_key":"value"}');
  assert.equal(toolCommandLabel(""), "");
  // Template-literal commands lift too — these rows used to show nothing but
  // `exec_command({`.
  assert.equal(
    toolCommandLabel("const r = await tools.exec_command({cmd:`python3 -c 'print(1)'`});"),
    "python3 -c 'print(1)'",
  );
  assert.equal(
    toolCommandLabel("await tools.exec_command({cmd:`make build\nmake test`})"),
    "make build…",
  );

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

test("payloads stack labelled bounded sections, arguments beside their result", () => {
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
  // No tabs: three sections in reading order, arguments before the result they
  // produced, paths last.
  assert.doesNotMatch(expanded, /role="tablist"|session-detail-payload-tab/);
  assert.equal(expanded.split("session-detail-payload-section").length - 1, 3);
  assert.match(expanded, /Arguments<[\s\S]*Result<[\s\S]*Paths</);
  // Each section keeps its own copy key, so what you copy is what you see.
  assert.match(expanded, /aria-label="Copy arguments to clipboard"/);
  assert.match(expanded, /aria-label="Copy result to clipboard"/);
  assert.match(expanded, /class="session-detail-payload-body"><code>Read 42 lines<\/code>/);
  assert.doesNotMatch(expanded, /<script>/);
  // The notice sits inside the section it bounds: the result is the truncated
  // one here, and it renders exactly once.
  assert.equal(expanded.split("Bounded preview · additional content omitted").length - 1, 1);
  // Neither payload here differs from its stored form, so no raw toggle.
  assert.doesNotMatch(expanded, /title="Show the stored value"/);

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
  // No arguments, so no Arguments section. Paths survives: an empty list on
  // the detail falls back to the preview's own paths.
  assert.doesNotMatch(whole, />Arguments</);
  assert.match(whole, />Paths</);

  // A stored transport envelope earns the raw toggle on its own section.
  const envelope = renderToStaticMarkup(
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
          result: JSON.stringify([{ type: "input_text", text: "Script completed" }]),
          result_truncated: false,
          affected_paths: [],
        },
      }}
      onExpandedChange={() => undefined}
    />,
  );
  assert.match(envelope, /title="Show the stored value"/);
  assert.match(envelope, /<code>Script completed<\/code>/);
});

test("path-bearing tool detail keeps external paths visible but out of the Paths list", () => {
  const inProjectPaths = [
    "generated/session-paths/inside-alpha.txt",
    "generated/session-paths/nested/inside-beta.json",
  ];
  const externalPaths = [
    "/private/tmp/mallard-live-123/project1_external/outside-alpha.txt",
    "/private/tmp/mallard-live-123/project1_external/nested/outside-beta.json",
  ];
  const argumentsValue = JSON.stringify({
    alpha: `./${inProjectPaths[0]}`,
    outside_alpha: externalPaths[0],
  });
  const resultValue = [
    `Created ./${inProjectPaths[1]}`,
    `Preserved ${externalPaths[1]}`,
  ].join("\n");

  const html = renderToStaticMarkup(
    <SessionDetailToolCall
      event={{
        ...toolEvent,
        event_id: "tool-path-remap",
        tool: {
          ...toolEvent.tool!,
          provider_call_id: "call-path-remap",
          category: "file_edit",
          summary: "Create the bounded path fixtures",
          argument_preview: argumentsValue,
          result_preview: resultValue,
          affected_paths: inProjectPaths,
        },
      }}
      expanded
      detailState={{
        loading: false,
        error: null,
        detail: {
          event_id: "tool-path-remap",
          arguments: argumentsValue,
          arguments_truncated: false,
          result: resultValue,
          result_truncated: false,
          affected_paths: inProjectPaths,
        },
      }}
      onExpandedChange={() => undefined}
    />,
  );

  // Session Details normalizes the active checkout to `.` while preserving
  // unrelated absolute paths in the bounded provider-native payload.
  assert.ok(html.includes(`./${inProjectPaths[0]}`));
  assert.ok(html.includes(`./${inProjectPaths[1]}`));
  for (const externalPath of externalPaths) {
    assert.ok(html.includes(externalPath));
  }

  // The dedicated Paths section remains project-scoped: it contains the two
  // safe relative paths and never promotes either external absolute path.
  const pathsSection = html.match(
    /<span class="session-detail-payload-label">Paths<\/span>[\s\S]*?<ul class="session-detail-tool-paths">([\s\S]*?)<\/ul>/,
  )?.[1];
  assert.ok(pathsSection, "expected the dedicated Paths list to render");
  for (const inProjectPath of inProjectPaths) assert.ok(pathsSection.includes(inProjectPath));
  for (const externalPath of externalPaths) assert.ok(!pathsSection.includes(externalPath));
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

test("Markdown project images embed through the validated loader only", () => {
  // A loaded cache entry renders the bounded data: URI; a failed entry and
  // a still-loading entry render the quiet chip, never a broken <img>.
  const loaded = renderToStaticMarkup(
    <SessionDetailMarkdown
      text={"![GIF](./animation.gif)"}
      onLoadProjectImage={() => undefined}
      projectImages={{
        "animation.gif": { status: "loaded", src: "data:image/gif;base64,R0lGODlh" },
      }}
    />,
  );
  assert.match(
    loaded,
    /<img[^>]*class="session-detail-markdown-image"[^>]*src="data:image\/gif;base64,R0lGODlh"[^>]*alt="GIF"/,
  );

  const failed = renderToStaticMarkup(
    <SessionDetailMarkdown
      text={"![GIF](./animation.gif)"}
      onLoadProjectImage={() => undefined}
      projectImages={{
        "animation.gif": { status: "failed", message: "too large" },
      }}
    />,
  );
  assert.doesNotMatch(failed, /<img/);
  assert.match(failed, /markdown-image-placeholder/);
  assert.match(failed, /title="too large"/);

  const pending = renderToStaticMarkup(
    <SessionDetailMarkdown
      text={"![GIF](./animation.gif)"}
      onLoadProjectImage={() => undefined}
      projectImages={{}}
    />,
  );
  assert.doesNotMatch(pending, /<img/);
  assert.match(pending, /markdown-image-placeholder is-loading/);
});

test("Markdown images outside the project never reach the webview as requests", () => {
  // Absolute paths, traversal, and foreign schemes fall to the chip: no
  // <img> element exists for them, so the webview issues no request and the
  // validated command is the only local read path. External https images
  // keep loading exactly as before.
  const html = renderToStaticMarkup(
    <SessionDetailMarkdown
      text={[
        "![Escape](/tmp/animation.gif)",
        "![Parent](../animation.gif)",
        "![Scheme](file:///tmp/animation.gif)",
        "![Remote](https://example.com/chart.png)",
      ].join(" ")}
      onLoadProjectImage={() => undefined}
      projectImages={{}}
    />,
  );
  const images = html.match(/<img[^>]*>/g) ?? [];
  assert.equal(images.length, 1);
  assert.match(images[0], /src="https:\/\/example\.com\/chart\.png"/);
  assert.match(images[0], /alt="Remote"/);
  // The chip names the file AND says what happened to it: a bare alt text left
  // "not shown" to the 30%-opacity tint, which no screen reader reports.
  for (const alt of ["Escape", "Parent", "Scheme"]) {
    assert.match(html, new RegExp(`markdown-image-placeholder[^>]*aria-label="${alt} — image not shown`));
  }
  // Without a loader wired (plain markdown contexts), project images also
  // stay chips instead of firing requests.
  const unwired = renderToStaticMarkup(
    <SessionDetailMarkdown text={"![GIF](./animation.gif)"} />,
  );
  assert.doesNotMatch(unwired, /<img/);
  assert.match(unwired, /markdown-image-placeholder/);
});

test("Markdown links clean project-relative paths and rejects every escape form", () => {
  const html = renderToStaticMarkup(
    <SessionDetailMarkdown
      text={"Created [story.md](./story.md). [Outside](/tmp/story.md) [Parent](../secret.md)"}
      onOpenProjectPath={() => undefined}
      projectRoot="/Users/example/HorrorGame"
    />,
  );
  assert.match(html, /href="story\.md"[^>]*>story\.md<\/a>/);
  assert.match(html, /data-tooltip="\/Users\/example\/HorrorGame\/story\.md"/);
  assert.match(html, /aria-label="Open \/Users\/example\/HorrorGame\/story\.md"/);
  assert.match(html, />Outside<\/span>/);
  assert.match(html, />Parent<\/span>/);
  assert.equal(safeRelativeMarkdownPath("./docs/story%20one.md"), "docs/story one.md");
  assert.equal(safeRelativeMarkdownPath("../secret.md"), null);
  assert.equal(safeRelativeMarkdownPath("%2e%2e/secret.md"), null);
  assert.equal(safeRelativeMarkdownPath("/tmp/story.md"), null);
  assert.equal(safeRelativeMarkdownPath("%2ftmp/story.md"), null);
  assert.equal(safeRelativeMarkdownPath("file:///tmp/story.md"), null);
  assert.equal(safeRelativeMarkdownPath("file%3a///tmp/story.md"), null);
  assert.equal(safeMarkdownUrl("docs/story.md"), "docs/story.md");
  assert.equal(
    projectMarkdownDisplayPath("C:\\project\\HorrorGame\\", "docs/story.md"),
    "C:\\project\\HorrorGame\\docs\\story.md",
  );
});

test("a file link's path card is not clipped by the card it sits in", () => {
  // The path card opens above the link. On a card's first line that is over
  // the stamp band and past the card's top edge, and an overflow clip on the
  // card cut its top off and fused what was left into the band. The band
  // rounds its own corners instead, at the card's 8px less its 2px edge.
  const css = readFileSync("src/App.css", "utf8");
  const rule = (selector: string) => css.match(new RegExp(`\\n${selector} \\{([^}]*)\\}`))?.[1] ?? "";
  const card = rule("\\.session-detail-card");
  assert.match(card, /border-radius: 8px;/);
  assert.doesNotMatch(card, /overflow/);
  assert.match(rule("\\.session-detail-card-head"), /border-radius: 6px 6px 0 0;/);
  assert.match(rule("\\.session-detail-project-link\\[data-tooltip\\]::after"), /bottom: calc\(100% \+ 8px\);/);
});

test("timeline page merging is chronological, de-duplicated, and counts keep their quality", () => {
  const first: ProjectSessionTimelinePage = {
    resource_id: detail.resource_id,
    snapshot_id: detail.snapshot_id,
    events: [events[2], events[0]],
    next_cursor: "next",
    warnings: [],
  };
  const second: ProjectSessionTimelinePage = {
    ...first,
    events: [events[2], events[3]],
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
  assert.match(apiSource, /"open_project_markdown_path"/);
  assert.doesNotMatch(apiSource, /"get_project_session_details"/);
});

// The row of a session's own Push review prints what its page counted. The
// page has a token total and no input/output split, and a lower bound on any
// readout marks the row's readouts partial.
test("the session page reports its counts in the fields its Push row prints", () => {
  assert.deepEqual(sessionPushFacts(detail), {
    title: "Build provider-neutral session detail",
    metrics: {
      started_at: START,
      ended_at: START + 660,
      user_round_count: 2,
      agent_message_count: 3,
      tool_call_count: 4,
      total_tokens: 1_063_252,
      metrics_complete: false,
    },
  });
  const unstamped = sessionPushFacts({
    ...detail,
    title: "",
    started_at: null,
    ended_at: null,
    tokens: { quality: "unsupported" },
  });
  assert.equal(unstamped.title, "Untitled session");
  assert.deepEqual(
    [unstamped.metrics.started_at, unstamped.metrics.ended_at, unstamped.metrics.total_tokens, unstamped.metrics.metrics_complete],
    [0, 0, null, true],
  );
});
