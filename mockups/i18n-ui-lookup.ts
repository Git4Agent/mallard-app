import { i18n, uiLanguage } from "../src/i18n";
import { en } from "../src/i18n/en";

/**
 * The live harnesses drive the app by the English its controls carry
 * ("Storages", "tidewatch. Project menu", "Push"). With the app's own
 * catalogs in Chinese those controls read Chinese, so a lookup asks here what
 * an English label reads in the interface language now: the English itself,
 * plus every catalog sentence that renders it, rendered again in Chinese with
 * the same values (the app's own `t`, so spacing and plurals match).
 */

type Tree = { readonly [key: string]: string | Tree };
interface Entry { key: string; english: string; pattern: RegExp | null; names: string[] }

const PLACEHOLDER = /\{\{\s*(\w+)\s*\}\}/g;
const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

function entries(tree: Tree, namespace: string, prefix = ""): Entry[] {
  return Object.entries(tree).flatMap(([name, value]) => {
    if (typeof value !== "string") return entries(value, namespace, `${prefix}${name}.`);
    // Sentences with markup render through Trans; the harness never looks them up.
    if (/<\/?\w+\s*\/?>/.test(value)) return [];
    const key = `${namespace}:${prefix}${name.replace(/_(one|other)$/, "")}`;
    const names = [...value.matchAll(PLACEHOLDER)].map((match) => match[1]);
    const pattern = names.length === 0
      ? null
      : new RegExp(`^${escape(value).replace(/\\\{\\\{\s*\w+\s*\\\}\\\}/g, "(.+?)")}$`);
    return [{ key, english: value, pattern, names }];
  });
}

const ENTRIES: Entry[] = Object.entries(en).flatMap(([namespace, tree]) => entries(tree as Tree, namespace));
const translate = i18n.t.bind(i18n) as unknown as (key: string, values?: Record<string, unknown>) => string;
const cache = new Map<string, string[]>();

/** What `english` may read as now. */
export function inUi(english: string): string[] {
  if (uiLanguage() !== "zh-CN") return [english];
  const known = cache.get(english);
  if (known) return known;
  const out = new Set<string>([english]);
  for (const entry of ENTRIES) {
    if (entry.pattern === null) {
      if (entry.english === english) out.add(translate(entry.key));
      continue;
    }
    const match = entry.pattern.exec(english);
    if (!match) continue;
    const values = Object.fromEntries(entry.names.map((name, index) => {
      const value = match[index + 1];
      return [name, name === "count" && /^\d+$/.test(value) ? Number(value) : value];
    }));
    out.add(translate(entry.key, values));
  }
  const result = [...out];
  cache.set(english, result);
  return result;
}

/** Whether `text` starts with `prefix` as the interface reads it: the whole
 *  prefix translated, or its first word translated and the rest as given. */
export function startsInUi(text: string, prefix: string): boolean {
  if (text.startsWith(prefix)) return true;
  if (inUi(prefix).some((candidate) => text.startsWith(candidate))) return true;
  const [first, ...rest] = prefix.split(" ");
  if (rest.length === 0) return false;
  const tail = rest.join(" ");
  return inUi(first).some((word) => text.startsWith(`${word} ${tail}`) || text.startsWith(`${word}${tail}`));
}
