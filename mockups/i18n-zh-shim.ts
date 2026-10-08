import { ZH_EXACT, ZH_RULES } from "./i18n-zh-dictionary";

/**
 * Specimen-only: rewrites the shipped shell's English into Chinese after React
 * renders, to show the layout before any catalog exists. It only ever sets a
 * text node's value or an attribute — never adds or removes nodes — so React
 * keeps reconciling the same DOM. The real change puts copy in a catalog.
 */

const ATTRIBUTES = ["title", "aria-label", "placeholder", "alt", "data-tooltip"] as const;
/* Machine text and the user's own words stay as they are. */
/* A count drawn in a <code> (a session's tokens) is still a number to format. */
const SKIP = "code:not([class^=quality-]), pre, kbd, samp, .sync-mono, [data-i18n-skip], textarea, input, .harness-controls, .harness-log";
/* A transcript's words: the user's prompts, the agent's replies, tool
   commands and output, commit subjects, an agent's label, brief and report.
   Kept as recorded, text and attributes alike, except a control drawn inside
   them (a Copy button, a media placeholder). */
const WORDS = ".markdown-copy, .session-detail-tool-summary, .session-detail-agent-label, .session-detail-agent-task, .session-detail-agent-pill:not(.mode), .session-detail-checkpoint-subject, .session-detail-payload-body";
const UI_IN_WORDS = "button, .markdown-image-placeholder, .session-detail-event-empty";
/* English that stays English in a Chinese UI: names and protocols. */
const KEEP = /^(v|Mallard Cloud octocat( · \w+)?|Mallard|Mallard Cloud|Codex|Claude|Git|GitHub|R2|Cloudflare R2|S3|Dropbox|main|tidewatch|portal|field-notes|octocat|MB|KB|GB|English|Baloo 2|[A-Z]?[a-z0-9-]+\.[a-z]{2,4}|~\/\S+|\/\S+|[0-9a-f]{7,}|[0-9a-f]{8}…[0-9a-f]{8}|EU|US|FedRAMP|S3 API URL|Wrangler|[\d\s.,:·/%+×—–-]*)$/;

const untranslated = new Set<string>();
(window as unknown as { __untranslated: Set<string> }).__untranslated = untranslated;

function translateCore(core: string): string | null {
  const exact = ZH_EXACT.get(core);
  if (exact !== undefined) return exact;
  for (const [pattern, replacement] of ZH_RULES) {
    if (!pattern.test(core)) continue;
    return typeof replacement === "string" ? core.replace(pattern, replacement) : core.replace(pattern, replacement as (...groups: string[]) => string);
  }
  return null;
}

/** Leading and trailing spaces are layout; keep them unless a CJK edge makes them noise. */
export function translate(value: string): string | null {
  const core = value.trim();
  if (!core || !/[A-Za-z]/.test(core)) return null;
  /* Han in a string is the user's name inside English ("Fork 潮汐笔记 — …")
     or copy already translated: the English rules still try it, and only
     an all-English miss is reported. */
  const zh = translateCore(core);
  if (zh === null) {
    if (!/[\u3400-\u9fff]/.test(core) && !KEEP.test(core)) untranslated.add(core);
    return null;
  }
  const lead = value.slice(0, value.indexOf(core));
  const trail = value.slice(value.indexOf(core) + core.length);
  return lead + zh + trail;
}

function inWords(element: Element): boolean {
  const words = element.closest(WORDS);
  if (!words) return false;
  const control = element.closest(UI_IN_WORDS);
  return !control || control.contains(words);
}

function skipped(node: Node): boolean {
  const parent = node.parentElement;
  return !parent || !!parent.closest(SKIP) || inWords(parent);
}

function translateTextNode(node: Text) {
  if (skipped(node)) return;
  const next = translate(node.nodeValue ?? "");
  if (next !== null && next !== node.nodeValue) {
    const owner = node.parentElement;
    if (owner) owner.setAttribute("data-i18n-en", owner.textContent?.trim() ?? "");
    node.nodeValue = next;
  }
}

function translateElement(element: Element) {
  if (element.closest("[data-i18n-skip]") || inWords(element)) return;
  for (const name of ATTRIBUTES) {
    const value = element.getAttribute(name);
    if (!value) continue;
    const next = translate(value);
    if (next !== null && next !== value) {
      element.setAttribute(`data-i18n-en-${name}`, value);
      element.setAttribute(name, next);
    }
  }
}

function sweep(root: Node) {
  if (root.nodeType === Node.TEXT_NODE) {
    translateTextNode(root as Text);
    return;
  }
  if (!(root instanceof Element)) return;
  translateElement(root);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeType === Node.TEXT_NODE) translateTextNode(node as Text);
    else translateElement(node as Element);
  }
}

export function installChineseShim(root: Element) {
  const observer = new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === "characterData") translateTextNode(record.target as Text);
      else if (record.type === "attributes") translateElement(record.target as Element);
      else record.addedNodes.forEach((node) => sweep(node));
    }
  });
  observer.observe(document.body, {
    subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: [...ATTRIBUTES],
  });
  sweep(root);
  /* Portalled menus and dialogs mount on body. */
  sweep(document.body);
}

/**
 * The CJK stack under `:lang(zh)`. Baloo 2 keeps Latin and digits; Han falls
 * through to the platform's Simplified Chinese face. Steps under 12px are
 * raised there: Han strokes need the pixels Latin does without.
 */
export const CJK_FONT_CSS = `
:root:lang(zh) {
  --font-cjk: "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei UI", "Microsoft YaHei", "Noto Sans CJK SC", "Noto Sans SC", "Source Han Sans SC";
  --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", var(--font-cjk), system-ui, sans-serif;
  --font-display: "Baloo 2", var(--font-cjk), "Arial Rounded MT Bold", sans-serif;
  --type-3xs: 0.6875rem;
  --type-2xs: 0.6875rem;
  --type-xs: 0.75rem;
}
:root:lang(zh) * { letter-spacing: normal !important; }
/* Found by the specimen: the timeline's gap pill sits on a 24px rail and
   "+1分钟" breaks between Han characters where "+1m" cannot. The real change
   sets nowrap on it for every language. */
.session-detail-event-gap > span { white-space: nowrap; }
`;
