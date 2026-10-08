import { memo, useCallback, useMemo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import type { PluggableList } from "unified";
import "katex/dist/katex.min.css";
import Icon from "../Icons";
import { makeMermaidPre } from "../mermaid/MermaidFlowchart";
import { remarkMermaidFences } from "../mermaid/remarkMermaidFences";
import { writeClipboardText } from "../sync-review/clipboard";
import { isSafeExternalMarkdownUrl, safeMarkdownUrl } from "./markdownUrl";
import { remarkLatexMath } from "./remarkLatexMath";

/**
 * The app's one Markdown renderer: GitHub-flavoured Markdown, math, and
 * Mermaid flowcharts, with raw HTML skipped and every destination passed
 * through `safeMarkdownUrl`. A page styles it with `.markdown-copy` and may
 * replace how images and links render; by default an image is a chip that is
 * never fetched, and a link opens outside the app only when it is http(s) or
 * mailto and the page says how.
 */

/** Alt text (or the path) inside a bordered chip: the still-loading and
 * cannot-embed states share one calm shape instead of the webview's
 * broken-image glyph. A track or a clip names itself, and a page that knows
 * why a file did not show prints the reason in place of "not shown". */
export function MarkdownImagePlaceholder({
  label,
  detail,
  loading,
  kind = "image",
  showDetail = false,
}: {
  label: string;
  detail?: string;
  loading?: boolean;
  kind?: "image" | "audio" | "video";
  showDetail?: boolean;
}) {
  /* A tint is not a text alternative, and a tooltip on a non-interactive span
     is not keyboard-reachable. Print the state as a word and repeat it in the
     accessible name because `role="img"` replaces the subtree. */
  const reason = showDetail && !loading && detail ? detail : null;
  const state = loading ? "loading…" : reason ?? "not shown";
  const accessibleName = reason
    ? `${label} — ${reason}`
    : detail ? `${label} — ${kind} ${state}. ${detail}` : `${label} — ${kind} ${state}`;
  return (
    <span
      className={`markdown-image-placeholder${loading ? " is-loading" : ""}`}
      role="img"
      aria-label={accessibleName}
      {...(detail ? { title: detail } : {})}
    >
      <Icon name="file" size={12} />
      <span className="markdown-image-placeholder-label">{label}</span>
      <span className="markdown-image-placeholder-state">{state}</span>
    </span>
  );
}

/* A fresh array literal every render is a changed prop to ReactMarkdown, so
   these live at module scope. */
const REMARK_PLUGINS = [remarkGfm, remarkMermaidFences, remarkMath, remarkLatexMath];
const REHYPE_PLUGINS: PluggableList = [[rehypeKatex, {
  trust: false,
  maxExpand: 1000,
  maxSize: 25,
}]];

/** A relative link the page can open itself: `resolve` names the file a
 *  link's href points at, or nothing, and `open` shows it. */
export interface MarkdownFileLink {
  resolve: (href: string) => string | null;
  open: (path: string) => void;
}

/* Parsing is the most expensive thing a Markdown page does, and its page
   re-renders for reasons that never change the text. Memoized on its own
   inputs, each body is parsed once; a caller keeps `components` and
   `fileLink` stable. */
const MarkdownText = memo(function MarkdownText({
  text,
  components,
  onOpenExternal,
  fileLink,
}: {
  text: string;
  /** Replaces the default renderers by tag, e.g. a page's own `img` or `a`. */
  components?: Components;
  /** Opens an http(s) or mailto link outside the app; without it a link is text. */
  onOpenExternal?: (url: string) => void;
  /** Opens a relative link to one of the page's own files; without it such a link is text. */
  fileLink?: MarkdownFileLink;
}) {
  /* `safeMarkdownUrl` keeps no `..`, so an href the page resolves to its own
     file is kept as written and only ever reaches `fileLink.open`. */
  const urlTransform = useCallback(
    (url: string) => (fileLink?.resolve(url) ? url : safeMarkdownUrl(url)),
    [fileLink],
  );
  const merged = useMemo<Components>(() => ({
    pre: makeMermaidPre(writeClipboardText),
    img: ({ alt }) => <MarkdownImagePlaceholder label={alt || "image"} />,
    // `node` is react-markdown's syntax tree, not an attribute.
    a: ({ href, children, node: _node, ...props }) => {
      const file = href && fileLink ? fileLink.resolve(href) : null;
      if (file && fileLink) {
        return (
          <a
            href={file}
            title={props.title}
            onClick={(event) => {
              event.preventDefault();
              fileLink.open(file);
            }}
          >
            {children}
          </a>
        );
      }
      const safeHref = href ? safeMarkdownUrl(href) : "";
      if (!onOpenExternal || !isSafeExternalMarkdownUrl(safeHref)) return <span>{children}</span>;
      return (
        <a
          {...props}
          href={safeHref}
          rel="noopener noreferrer"
          onClick={(event) => {
            event.preventDefault();
            onOpenExternal(safeHref);
          }}
        >
          {children}
        </a>
      );
    },
    ...components,
  }), [components, onOpenExternal, fileLink]);
  return (
    <ReactMarkdown
      remarkPlugins={REMARK_PLUGINS}
      rehypePlugins={REHYPE_PLUGINS}
      skipHtml
      urlTransform={urlTransform}
      components={merged}
    >
      {text}
    </ReactMarkdown>
  );
});

export default MarkdownText;
