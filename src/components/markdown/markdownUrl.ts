/**
 * Where a Markdown link or image may point. Text that reaches a page — a
 * session transcript, a skill's SKILL.md — is never trusted to navigate the
 * web view: only http(s) and mailto leave the app, and only clean relative
 * descendants stay as paths the page may resolve itself. Every other
 * destination becomes "" and renders as plain text.
 */

const EXTERNAL_MARKDOWN_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

const MARKDOWN_URL_SCHEME = /^[a-z][a-z\d+.-]*:/i;

/**
 * A clean relative descendant path, decoded, or null: absolute paths, URL
 * schemes, and parent traversal stay plain text. A page may resolve its
 * own directory context before this shared check.
 */
export function safeRelativeMarkdownPath(url: string): string | null {
  if (!url || url.startsWith("#") || url.startsWith("/") || url.startsWith("\\")
      || MARKDOWN_URL_SCHEME.test(url)) {
    return null;
  }
  let decoded: string;
  try {
    decoded = decodeURIComponent(url).replace(/\\/g, "/");
  } catch {
    return null;
  }
  if (decoded.startsWith("/") || MARKDOWN_URL_SCHEME.test(decoded)) return null;
  const parts: string[] = [];
  for (const part of decoded.split("/")) {
    if (!part || part === ".") continue;
    if (part === ".." || part.includes("\0")) return null;
    parts.push(part);
  }
  return parts.length > 0 ? parts.join("/") : null;
}

export function safeMarkdownUrl(url: string): string {
  if (url.startsWith("#")) return url;
  try {
    const parsed = new URL(url);
    if (EXTERNAL_MARKDOWN_PROTOCOLS.has(parsed.protocol)) return parsed.toString();
  } catch {
    // Relative Markdown destinations are handled as relative paths below.
  }
  return safeRelativeMarkdownPath(url) ?? "";
}

export function isSafeExternalMarkdownUrl(url: string): boolean {
  if (url.startsWith("#")) return false;
  try {
    return EXTERNAL_MARKDOWN_PROTOCOLS.has(new URL(url).protocol);
  } catch {
    return false;
  }
}
