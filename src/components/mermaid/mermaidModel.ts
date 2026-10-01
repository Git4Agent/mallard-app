export const MAX_MERMAID_SOURCE_BYTES = 32 * 1024;
export const MAX_MERMAID_SVG_BYTES = 1024 * 1024;

export interface MermaidResult {
  svg?: string;
  error?: string;
}

export function mermaidSourceProblem(code: string): string | null {
  if (new TextEncoder().encode(code).length > MAX_MERMAID_SOURCE_BYTES) {
    return "This flowchart exceeds the 32 KiB source limit.";
  }
  if (!/^\s*(?:---[\s\S]*?---\s*)?(?:%%[^\n]*\n\s*)*(?:flowchart|graph)(?:\s|$)/.test(code)) {
    return "Only Mermaid flowcharts are supported.";
  }
  return null;
}

/** Treat the parser's message as untrusted, even though its scripts are bundled. */
export function readMermaidResult(value: unknown): MermaidResult {
  if (!value || typeof value !== "object") return { error: "The flowchart parser returned an invalid result." };
  const result = value as Record<string, unknown>;
  if (result.tauriPresent !== false || result.strict !== true || result.htmlLabels !== false) {
    return { error: "The flowchart parser could not run safely." };
  }
  if (typeof result.error === "string" && result.error.length <= 200) return { error: result.error };
  if (typeof result.svg !== "string" || new TextEncoder().encode(result.svg).length > MAX_MERMAID_SVG_BYTES) {
    return { error: "The flowchart parser returned an invalid image." };
  }
  return { svg: result.svg };
}

export function mermaidImageSource(svg: string): string {
  if (new TextEncoder().encode(svg).length > MAX_MERMAID_SVG_BYTES || /<!DOCTYPE|<!ENTITY/i.test(svg)) {
    throw new Error("Unsupported SVG size or declaration");
  }
  const parsed = new DOMParser().parseFromString(svg, "image/svg+xml");
  if (parsed.querySelector("parsererror,script,foreignObject,image,iframe,object,embed,a")
      || parsed.documentElement.namespaceURI !== "http://www.w3.org/2000/svg"
      || parsed.documentElement.localName !== "svg") throw new Error("Unsupported SVG elements");
  for (const element of parsed.querySelectorAll("*")) {
    for (const attribute of element.attributes) {
      if (/^on/i.test(attribute.name) || (/href$/i.test(attribute.name) && !attribute.value.startsWith("#"))) {
        throw new Error("Unsupported SVG attribute");
      }
    }
  }
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
