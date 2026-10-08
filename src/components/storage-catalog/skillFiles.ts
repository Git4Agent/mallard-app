/*
 * A skill's files as its page opens them. A relative link in SKILL.md, or in
 * a Markdown file opened beside it, names one of the skill's own files; the
 * Mallard Cloud page resolves links by the same rule.
 */

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;
const CONTROL = /[\u0000-\u001f\u007f]/;

/** The file a relative link names: resolved against the linking file's own
 *  folder, `..` allowed but never above the skill's root, and only when it is
 *  one of the skill's files. Schemes, absolute paths, anchors and queries
 *  stay outside the skill. */
export function resolveSkillLink(from: string, href: string, listed: ReadonlySet<string>): string | null {
  if (!href || href.length > 4096 || /[?#]/.test(href) || CONTROL.test(href) || SCHEME.test(href)) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(href).replace(/\\/g, "/");
  } catch {
    return null;
  }
  if (decoded.startsWith("/") || SCHEME.test(decoded) || CONTROL.test(decoded)) return null;
  const parts = from.split("/").slice(0, -1);
  for (const part of decoded.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length === 0) return null;
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  const path = parts.join("/");
  return path && listed.has(path) ? path : null;
}

export const isMarkdownPath = (path: string) => /\.(md|markdown)$/i.test(path);

export const baseName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/** Lines a text has, a trailing newline closing the last one. */
export const lineCount = (text: string) => (text === "" ? 0 : text.replace(/\n$/, "").split("\n").length);

/** The agentskills.io frontmatter, split off SKILL.md as Mallard Cloud's
 *  skill page does: top-level `key: value` lines, indented continuations
 *  folded in (block scalars included), nested maps and lists skipped. Without
 *  a leading `---` and a closing one, the whole file is the body. */
export function splitSkillFrontmatter(text: string): { fields: Record<string, string>; body: string } {
  const lines = text.split(/\r?\n/);
  const close = lines[0] === "---" ? lines.findIndex((line, index) => index > 0 && line.trimEnd() === "---") : -1;
  if (close === -1) return { fields: {}, body: text };
  const fields: Record<string, string> = {};
  for (let index = 1; index < close; index++) {
    const match = /^([A-Za-z0-9_-]+):(.*)$/.exec(lines[index]!);
    if (!match) continue;
    const continuation: string[] = [];
    while (index + 1 < close && /^(\s|$)/.test(lines[index + 1]!)) {
      const line = lines[++index]!.trim();
      if (line) continuation.push(line);
    }
    const value = match[2]!.trim();
    if (/^[|>][+-]?$/.test(value)) fields[match[1]!] = continuation.join(value.startsWith("|") ? "\n" : " ");
    else if (value) fields[match[1]!] = unquote([value, ...continuation].join(" "));
  }
  return { fields, body: lines.slice(close + 1).join("\n") };
}

function unquote(value: string): string {
  const quote = value[0];
  if (value.length < 2 || (quote !== '"' && quote !== "'") || !value.endsWith(quote)) return value;
  const inner = value.slice(1, -1);
  return quote === "'" ? inner.replace(/''/g, "'") : inner.replace(/\\(["\\])/g, "$1");
}
