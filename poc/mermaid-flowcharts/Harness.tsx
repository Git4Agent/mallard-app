import { useState, type ComponentType } from "react";
import { fixture } from "./fixtures";
import type { ClaudeWidgetTool } from "../../src/components/widgets/claudeWidget";
import "./poc.css";

export default function Harness({ surface, Markdown, Widget, widgetCode }: {
  surface: "Local" | "Cloud";
  Markdown: ComponentType<{ text: string; flowcharts?: boolean }>;
  Widget: ComponentType<{ tool: ClaudeWidgetTool; onCopy: (text: string) => Promise<void> }>;
  widgetCode: string;
}) {
  const [example, setExample] = useState(new URLSearchParams(location.search).get("case") ?? "diagram");
  const [theme, setTheme] = useState("light");
  return (
    <main className="poc-shell">
      <header className="poc-header"><div><span className="poc-eyebrow">PRODUCTION RENDERER CHECK</span><h1>{surface} session detail</h1></div>
        <nav><a href="desktop.html">Local</a><a href="cloud.html">Cloud</a></nav>
      </header>
      <div className="poc-controls"><label>Example <select value={example} onChange={(event) => setExample(event.target.value)}>{["diagram", "widget", "multiple", "malformed", "empty", "unclosed", "oversized", "ordinary", "directives", "frontmatter", "resources", "labels", "nodes", "edges", "quoted"].map((name) => <option key={name}>{name}</option>)}</select></label>
        <label>Theme <select value={theme} onChange={(event) => { setTheme(event.target.value); document.documentElement.dataset.theme = event.target.value; }}><option>light</option><option>dark</option></select></label>
        <span>Production components. Synthetic session source.</span></div>
      <article className="poc-message session-detail-event-copy"><div className="poc-speaker">Assistant · sample session message</div>
        {example === "widget" ? <Widget tool={{ native_name: "mcp__visualize__show_widget", argument_preview: JSON.stringify({ title: "child_transcripts_held_by_each_side", widget_code: widgetCode }), argument_truncated: false, status: "completed" }} onCopy={(text) => navigator.clipboard.writeText(text)} /> : <Markdown text={fixture(example)} flowcharts />}
      </article>
      <footer>Diagrams are images. The original source stays in the session.</footer>
    </main>
  );
}
