import { useMemo, useState } from "react";
import { useTranslation } from "../../i18n";
import { readClaudeWidget, svgWidgetImage, type ClaudeWidgetTool } from "./claudeWidget";
import { useSessionTheme } from "../sessionTheme";
import "./ClaudeSvgWidget.css";

export default function ClaudeSvgWidget({ tool, onCopy }: {
  tool: ClaudeWidgetTool;
  onCopy: (text: string) => Promise<void>;
}) {
  const { t, i18n } = useTranslation("shell");
  const theme = useSessionTheme();
  // Both read words (a fallback title, an error): they follow the language.
  const widget = useMemo(() => readClaudeWidget(tool), [tool, i18n.language]);
  const image = useMemo(() => widget.error ? { src: null, error: widget.error } : svgWidgetImage(widget.code, theme), [widget, theme, i18n.language]);
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const [sourceOpen, setSourceOpen] = useState(false);
  const error = image.error ?? (image.src && failedSrc === image.src ? t("widget.notDisplayed") : null);
  return (
    <section className="claude-widget" aria-label={widget.title}>
      <div className="claude-widget-head"><span>{t("widget.head")}</span><strong>{widget.title}</strong></div>
      {error ? <p className="claude-widget-note" role="note">{error}</p> : image.src ? (
        <div className="claude-widget-image">
          <img src={image.src} alt={widget.title} loading="lazy" decoding="async" onError={() => setFailedSrc(image.src)} />
        </div>
      ) : <p className="claude-widget-note" role="status">{t("widget.loading")}</p>}
      <details className="claude-widget-source" open={sourceOpen} onToggle={(event) => setSourceOpen(event.currentTarget.open)}>
        <summary>{t("diagram.source")}</summary>
        {sourceOpen && <div>
          <button type="button" onClick={() => void onCopy(widget.code)}>{t("diagram.copySource")}</button>
          <pre><code>{widget.code}</code></pre>
        </div>}
      </details>
    </section>
  );
}
