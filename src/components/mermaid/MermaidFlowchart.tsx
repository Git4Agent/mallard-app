import { useEffect, useRef, useState } from "react";
import type { Components } from "react-markdown";
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
  const target = useRef<HTMLElement>(null);
  const theme = useSessionTheme();
  const [active, setActive] = useState(false);
  const [state, setState] = useState<{ code: string; theme: SessionTheme; src?: string; error?: string } | null>(null);
  const [sourceOpen, setSourceOpen] = useState(false);
  const [copied, setCopied] = useState<{ code: string; message: string } | null>(null);
  const inputError = complete ? mermaidSourceProblem(code) : "The Mermaid fence is incomplete. The source is available below.";
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
        setState(result.svg ? { code, theme, src: mermaidImageSource(result.svg) } : { code, theme, error: result.error });
      } catch {
        setState({ code, theme, error: "This flowchart contains unsupported image content. The source is available below." });
      }
    }).catch(() => {
      if (!cancelled) setState({ code, theme, error: "The flowchart could not be rendered. The source is available below." });
    });
    return () => { cancelled = true; };
  }, [active, code, inputError, theme]);
  const current = state?.code === code && state.theme === theme ? state : null;
  const error = inputError ?? current?.error;
  return (
    <section ref={target} className="mermaid-flowchart" aria-label="Mermaid flowchart" data-status={error ? "error" : current?.src ? "ready" : "loading"}>
      <div className="mermaid-flowchart-head">Flowchart</div>
      {error ? <p className="mermaid-flowchart-note" role="note">{error}</p> : current?.src ? (
        <div className="mermaid-flowchart-image">
          <img src={current.src} alt="Flowchart from the session message" decoding="async" onError={() => setState({ code, theme, error: "The flowchart image could not be displayed. The source is available below." })} />
        </div>
      ) : <p className="mermaid-flowchart-note" role="status">Loading flowchart…</p>}
      <details className="mermaid-flowchart-source" open={sourceOpen} onToggle={(event) => setSourceOpen(event.currentTarget.open)}>
        <summary>Source</summary>
        {sourceOpen && <div>
          <button type="button" onClick={() => {
            void onCopy(code).then(() => setCopied({ code, message: "Copied." })).catch(() => setCopied({ code, message: "Select the source to copy it." }));
          }}>Copy source</button>
          <span role="status">{copied?.code === code ? copied.message : ""}</span>
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
