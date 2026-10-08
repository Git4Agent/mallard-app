import { useEffect, useRef, useState } from "react";
import type { Components } from "react-markdown";
import { useTranslation } from "../../i18n";
import { renderMermaid } from "./mermaidClient";
import { mermaidImageSource, mermaidSourceProblem } from "./mermaidModel";
import { useSessionTheme, type SessionTheme } from "../sessionTheme";
import "./MermaidFlowchart.css";

type CopySource = (source: string) => Promise<void>;

export default function MermaidFlowchart({ code, complete, onCopy }: {
  code: string;
  complete: boolean;
  onCopy: CopySource;
}) {
  const { t, i18n } = useTranslation("shell");
  const language = i18n.language;
  const target = useRef<HTMLElement>(null);
  const theme = useSessionTheme();
  const [active, setActive] = useState(false);
  /* A drawn result belongs to its source, theme and language: an error is
     worded when the render settles. */
  const [state, setState] = useState<{ code: string; theme: SessionTheme; language: string; src?: string; error?: string } | null>(null);
  const [sourceOpen, setSourceOpen] = useState(false);
  const [copied, setCopied] = useState<{ code: string; ok: boolean } | null>(null);
  const inputError = complete ? mermaidSourceProblem(code) : t("mermaid.incomplete");
  useEffect(() => {
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) setActive(true);
    }, { rootMargin: "200px" });
    if (target.current) observer.observe(target.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!active || inputError) return;
    let cancelled = false;
    void renderMermaid(code, theme).then((result) => {
      if (cancelled) return;
      try {
        setState(result.svg ? { code, theme, language, src: mermaidImageSource(result.svg) } : { code, theme, language, error: result.error });
      } catch {
        setState({ code, theme, language, error: t("mermaid.unsupportedImage") });
      }
    }).catch(() => {
      if (!cancelled) setState({ code, theme, language, error: t("mermaid.notRendered") });
    });
    return () => { cancelled = true; };
  }, [active, code, inputError, theme, language, t]);
  const current = state?.code === code && state.theme === theme && state.language === language ? state : null;
  const error = inputError ?? current?.error;
  return (
    <section ref={target} className="mermaid-flowchart" aria-label={t("mermaid.label")} data-status={error ? "error" : current?.src ? "ready" : "loading"}>
      <div className="mermaid-flowchart-head">{t("mermaid.head")}</div>
      {error ? <p className="mermaid-flowchart-note" role="note">{error}</p> : current?.src ? (
        <div className="mermaid-flowchart-image">
          <img src={current.src} alt={t("mermaid.alt")} decoding="async" onError={() => setState({ code, theme, language, error: t("mermaid.notDisplayed") })} />
        </div>
      ) : <p className="mermaid-flowchart-note" role="status">{t("mermaid.loading")}</p>}
      <details className="mermaid-flowchart-source" open={sourceOpen} onToggle={(event) => setSourceOpen(event.currentTarget.open)}>
        <summary>{t("diagram.source")}</summary>
        {sourceOpen && <div>
          <button type="button" onClick={() => {
            void onCopy(code).then(() => setCopied({ code, ok: true })).catch(() => setCopied({ code, ok: false }));
          }}>{t("diagram.copySource")}</button>
          <span role="status">{copied?.code === code ? t(copied.ok ? "diagram.copied" : "diagram.selectToCopy") : ""}</span>
          <pre><code>{code}</code></pre>
        </div>}
      </details>
    </section>
  );
}

export function makeMermaidPre(onCopy: CopySource): NonNullable<Components["pre"]> {
  return function MermaidPre({ node, children, ...props }) {
    const code = node?.children.length === 1 ? node.children[0] : null;
    if (!code || code.type !== "element" || code.tagName !== "code"
        || !Array.isArray(code.properties.className) || !code.properties.className.includes("language-mermaid")) {
      return <pre {...props}>{children}</pre>;
    }
    const source = code.children.map((child) => child.type === "text" ? child.value : "").join("");
    return <MermaidFlowchart code={source} complete={code.properties["data-mermaid-complete"] === true} onCopy={onCopy} />;
  };
}
