import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { readClaudeWidget, isClaudeWidgetTool, CLAUDE_WIDGET_TOOL, type ClaudeWidgetTool } from "../../src/components/widgets/claudeWidget";
import SessionDetailEvent from "../../src/components/project-sync/session-detail/SessionDetailEvent";
import ClaudeSvgWidget from "../../src/components/widgets/ClaudeSvgWidget";
import { i18n } from "../../src/i18n";
import type { SessionTimelineEvent } from "../../src/types";

const svg = '<svg viewBox="0 0 200 80"><text x="10" y="40">Portable 🦆</text></svg>';
const tool: ClaudeWidgetTool = {
  native_name: CLAUDE_WIDGET_TOOL,
  argument_preview: JSON.stringify({ title: "Portable diagram", widget_code: svg }),
  argument_truncated: false,
  status: "completed",
};

test("recognizes only the recorded Claude widget tool and reads complete source", () => {
  assert.equal(isClaudeWidgetTool(tool), true);
  assert.equal(isClaudeWidgetTool({ native_name: "Write" }), false);
  assert.equal(isClaudeWidgetTool({ native_name: "mcp__other__show_widget" }), false);
  assert.deepEqual(readClaudeWidget(tool), { title: "Portable diagram", code: svg, error: null });
});

test("refuses incomplete, oversized, malformed, failed, and HTML widget input", () => {
  assert.match(readClaudeWidget({ ...tool, argument_truncated: true }).error!, /incomplete/);
  assert.match(readClaudeWidget({ ...tool, argument_preview: "x".repeat(1024 * 1024 + 1) }).error!, /limit/);
  assert.match(readClaudeWidget({ ...tool, argument_preview: "{" }).error!, /JSON/);
  assert.match(readClaudeWidget({ ...tool, argument_preview: "null" }).error!, /source/);
  assert.match(readClaudeWidget({ ...tool, status: "failed" }).error!, /failed/);
  const html = '<div>counter</div><script>window.attack=true</script>';
  const unsupported = readClaudeWidget({ ...tool, argument_preview: JSON.stringify({ widget_code: html }) });
  assert.match(unsupported.error!, /Only SVG/);
  assert.equal(unsupported.code, html, "unsupported source remains available for inspection");
});

test("shows the widget in a closed-tool timeline and keeps arbitrary tools as tool rows", () => {
  const event: SessionTimelineEvent = {
    event_id: "widget-1", ordinal: 1, kind: "tool_call", quality: "exact",
    tool: { ...tool, status: "completed", provider_call_id: "call-1", category: "other", summary: "Portable diagram", result_truncated: false, affected_paths: [] },
  };
  const render = (provider: string, nativeName = CLAUDE_WIDGET_TOOL) => renderToStaticMarkup(
    <SessionDetailEvent event={{ ...event, tool: { ...event.tool!, native_name: nativeName } }}
      provider={provider} providerDisplayName={provider} turnDuration={null} expandedToolIds={new Set()}
      toolDetails={{}} onToolExpandedChange={() => undefined} onOpenProjectPath={() => undefined} projectRoot="" />,
  );
  const widget = render("claude");
  assert.match(widget, /class="claude-widget"/);
  assert.match(widget, /Portable diagram/);
  assert.match(widget, /<summary>Source<\/summary>/);
  assert.doesNotMatch(widget, /session-detail-tool-call|<svg[^>]*>.*Portable|<iframe/);
  assert.doesNotMatch(render("claude", "Write"), /claude-widget/);
  assert.doesNotMatch(render("codex"), /claude-widget/);
});

test("in Chinese the widget's frame and refusals speak Chinese; the recorded title and source stay as given", () => {
  void i18n.changeLanguage("zh-CN");
  try {
    assert.equal(readClaudeWidget({ ...tool, argument_truncated: true }).error, "小组件源代码不完整。");
    assert.equal(readClaudeWidget({ ...tool, argument_preview: "{" }).title, "Claude 小组件");
    const html = renderToStaticMarkup(<ClaudeSvgWidget tool={tool} onCopy={async () => undefined} />);
    assert.match(html, /aria-label="Portable diagram"/);
    assert.match(html, /<span>小组件<\/span><strong>Portable diagram<\/strong>/);
    assert.match(html, /<summary>源代码<\/summary>/);
    const refused = renderToStaticMarkup(<ClaudeSvgWidget tool={{ ...tool, argument_preview: JSON.stringify({ title: "Counter", widget_code: "<div>counter</div>" }) }} onCopy={async () => undefined} />);
    assert.match(refused, /只能显示 SVG 小组件。打开“源代码”可查看此小组件。/);
  } finally {
    void i18n.changeLanguage("en");
  }
});
