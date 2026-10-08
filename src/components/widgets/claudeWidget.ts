import { t } from "../../i18n";
import type { SessionTheme } from "../sessionTheme";

export const CLAUDE_WIDGET_TOOL = "mcp__visualize__show_widget";
const MAX_WIDGET_BYTES = 1024 * 1024;

export interface ClaudeWidgetTool {
  native_name: string;
  argument_preview?: string | null;
  argument_truncated: boolean;
  status: string;
}

export function isClaudeWidgetTool(tool: { native_name: string } | null | undefined): boolean {
  return tool?.native_name === CLAUDE_WIDGET_TOOL;
}

export function readClaudeWidget(tool: ClaudeWidgetTool): { title: string; code: string; error: string | null } {
  const fallback = { title: t("shell:widget.title"), code: tool.argument_preview ?? "" };
  if (tool.argument_truncated) return { ...fallback, error: t("shell:widget.incomplete") };
  if (new TextEncoder().encode(fallback.code).length > MAX_WIDGET_BYTES) {
    return { ...fallback, error: t("shell:widget.tooLarge") };
  }
  let input: unknown;
  try {
    input = JSON.parse(fallback.code);
  } catch {
    return { ...fallback, error: t("shell:widget.invalidJson") };
  }
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    return { ...fallback, error: t("shell:widget.noSource") };
  }
  const fields = input as Record<string, unknown>;
  const title = typeof fields.title === "string" && fields.title.trim()
    ? fields.title.trim().slice(0, 256) : fallback.title;
  const code = typeof fields.widget_code === "string" ? fields.widget_code : "";
  if (!code.trim()) return { title, code: fallback.code, error: t("shell:widget.noSource") };
  if (tool.status === "failed") return { title, code, error: t("shell:widget.failed") };
  if (!/^\s*(?:(?:<\?xml\s[^?]*\?>|<!--[\s\S]*?-->)\s*)*<svg(?:\s|\/?>)/.test(code)) {
    return { title, code, error: t("shell:widget.onlySvg") };
  }
  return { title, code, error: null };
}

// Resolve Claude host classes when the image is prepared. Keep the recorded source unchanged.
const SVG_PALETTES: Record<SessionTheme, string> = {
  light: `
  :root{--color-text-primary:#33271b;--color-text-secondary:#74624a;--color-text-tertiary:#9a825f;--color-background-primary:#fffdf7;--color-background-secondary:#faf1dc;--color-border-primary:#9a825f;--color-border-secondary:#b6a17a;--color-border-tertiary:#decfaf;--font-sans:system-ui,sans-serif;--font-mono:monospace}
  .c-gray{--fill:#eeece5;--stroke:#92988c;--heading:#41483b;--label:#626959}
  .c-teal{--fill:#e1f4ee;--stroke:#489b84;--heading:#155b49;--label:#347c65}
  .c-blue{--fill:#e4f0fa;--stroke:#508fb7;--heading:#234f70;--label:#417ca3}
  .c-purple{--fill:#eee8fa;--stroke:#8972b5;--heading:#503b79;--label:#786297}
  .c-green{--fill:#e8f3e2;--stroke:#77a563;--heading:#3e622d;--label:#62884d}
  .c-coral,.c-red{--fill:#fbe8e3;--stroke:#c97b6b;--heading:#8d3f32;--label:#af6354}
  .c-pink{--fill:#f8e5f0;--stroke:#b66b95;--heading:#803b63;--label:#a15b82}
  .c-amber,.c-orange,.c-yellow{--fill:#faf0d9;--stroke:#b18c45;--heading:#77561f;--label:#957436}
  `,
  dark: `
  :root{--color-text-primary:#fbf3dd;--color-text-secondary:#c8b998;--color-text-tertiary:#a99670;--color-background-primary:#2d261b;--color-background-secondary:#332b1e;--color-border-primary:#a99670;--color-border-secondary:#998765;--color-border-tertiary:#8b7b59;--font-sans:system-ui,sans-serif;--font-mono:monospace}
  .c-gray{--fill:#454744;--stroke:#a2a79d;--heading:#e0dfd7;--label:#c0bfb7}
  .c-teal{--fill:#075e4f;--stroke:#68baa2;--heading:#a1ded0;--label:#70c4ac}
  .c-blue{--fill:#12466c;--stroke:#6aa9d1;--heading:#b3d8f1;--label:#87bddd}
  .c-purple{--fill:#463477;--stroke:#a491ce;--heading:#d2c7ef;--label:#b6a6dc}
  .c-green{--fill:#315726;--stroke:#87b775;--heading:#c0dcb4;--label:#a0ca90}
  .c-coral,.c-red{--fill:#74342d;--stroke:#d39185;--heading:#efc4bb;--label:#dfa698}
  .c-pink{--fill:#6f2d50;--stroke:#ca88af;--heading:#edbfd8;--label:#dba0c2}
  .c-amber,.c-orange,.c-yellow{--fill:#64481b;--stroke:#c6a366;--heading:#e7d0a5;--label:#d6ba88}
  `,
};
const SVG_STYLE = `
  text{font-family:var(--font-sans);fill:var(--color-text-primary)}
  .t{font-size:14px}.th{font-size:16px;font-weight:600}.ts{font-size:13px;fill:var(--color-text-secondary)}
  .box{fill:none;stroke:var(--color-border-tertiary)}.arr{fill:none;stroke:var(--color-text-secondary);stroke-width:1.5}
  [class*="c-"] rect,[class*="c-"] circle,[class*="c-"] ellipse,rect[class*="c-"],circle[class*="c-"],ellipse[class*="c-"]{fill:var(--fill);stroke:var(--stroke)}
  [class*="c-"] .th{fill:var(--heading)}[class*="c-"] .ts{fill:var(--label)}
`;

export function svgWidgetImage(code: string, theme: SessionTheme): { src: string | null; error: string | null } {
  // Node rendering has no XML parser. The browser prepares the image on mount.
  if (typeof DOMParser === "undefined") return { src: null, error: null };
  if (/<!DOCTYPE|<!ENTITY/i.test(code)) return { src: null, error: t("shell:widget.declarations") };
  try {
    const documentCode = code.replace(/<svg\b([^>]*)>/, (root, attributes: string) => (
      // i18n-ignore: SVG markup, not copy
      /\bxmlns\s*=/.test(attributes) ? root : `<svg xmlns="http://www.w3.org/2000/svg"${attributes}>`
    ));
    const parsed = new DOMParser().parseFromString(documentCode, "image/svg+xml");
    if (parsed.querySelector("parsererror") || parsed.documentElement.localName !== "svg"
        || parsed.documentElement.namespaceURI !== "http://www.w3.org/2000/svg") {
      return { src: null, error: t("shell:widget.notSvg") };
    }
    const style = parsed.createElementNS("http://www.w3.org/2000/svg", "style");
    style.textContent = SVG_PALETTES[theme] + SVG_STYLE;
    parsed.documentElement.prepend(style);
    // Image mode is the execution boundary. Never insert this XML into the page.
    const xml = new XMLSerializer().serializeToString(parsed);
    return { src: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(xml)}`, error: null };
  } catch {
    return { src: null, error: t("shell:widget.notSvg") };
  }
}
