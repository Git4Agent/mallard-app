import { useMemo, useState } from "react";
import { readClaudeWidget, svgWidgetImage, type ClaudeWidgetTool } from "./claudeWidget";
import { useSessionTheme } from "../sessionTheme";
import "./ClaudeSvgWidget.css";

export default function ClaudeSvgWidget({ tool, onCopy }: {
  tool: ClaudeWidgetTool;
  onCopy: (text: string) => Promise<void>;
}) {
  const theme = useSessionTheme();
  const widget = useMemo(() => readClaudeWidget(tool), [tool]);
  const image = useMemo(() => widget.error ? { src: null, error: widget.error } : svgWidgetImage(widget.code, theme), [widget, theme]);
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const [sourceOpen, setSourceOpen] = useState(false);
  const error = image.error ?? (image.src && failedSrc === image.src ? "The SVG image could not be displayed." : null);
  return (
    <section className="claude-widget" aria-label={widget.title}>
      <div className="claude-widget-head"><span>Widget</span><strong>{widget.title}</strong></div>
      {error ? <p className="claude-widget-note" role="note">{error}</p> : image.src ? (
        <div className="claude-widget-image">
          <img src={image.src} alt={widget.title} loading="lazy" decoding="async" onError={() => setFailedSrc(image.src)} />
        </div>
      ) : <p className="claude-widget-note" role="status">Loading widget…</p>}
      <details className="claude-widget-source" open={sourceOpen} onToggle={(event) => setSourceOpen(event.currentTarget.open)}>
        <summary>Source</summary>
        {sourceOpen && <div>
          <button type="button" onClick={() => void onCopy(widget.code)}>Copy source</button>
          <pre><code>{widget.code}</code></pre>
        </div>}
      </details>
    </section>
  );
}
