import { settleValueSpacing, uiLanguage } from "./index";
import { zhBackend } from "./zh-CN/backend";

/**
 * Fixed labels the backend sends beside a stable key: storage provider
 * fields, summary labels, resource kinds, capability names, agent and
 * session actions, conversation blockers, warning codes. The backend's
 * English stays the one English source — the CLI and the activity log print
 * it too — and Chinese is looked up here by the label's key. A label this
 * build has no Chinese for shows the backend's English, as an error does.
 *
 * Keys are dotted paths into `zh-CN/backend.ts`, built from the backend's
 * own identifiers: `storageProvider.r2.field.bucket.label`,
 * `resourceKind.conversation`, `blocker.missing_binding`.
 */
export function backendText(
  key: string,
  english: string,
  values: Readonly<Record<string, string | number>> = {},
): string {
  if (uiLanguage() !== "zh-CN") return english;
  const chinese = BACKEND_ZH.get(key);
  if (chinese === undefined) return english;
  // Values settle their spacing against the Han around them, as catalog
  // values do (see `settleValueSpacing`); a number keeps the template's.
  return settleValueSpacing(chinese.replace(/\{\{\s*(\w+)\s*\}\}/g, (match, name: string) => {
    if (!(name in values)) return match;
    const value = String(values[name]);
    if (value === "") return value;
    return /^\d/.test(value) ? `\uE002${value}\uE003` : `\uE000${value}\uE001`;
  }));
}

type Tree = { readonly [key: string]: string | Tree };

function flatten(tree: Tree, prefix = "", out = new Map<string, string>()): Map<string, string> {
  for (const [key, value] of Object.entries(tree)) {
    if (typeof value === "string") out.set(prefix + key, value);
    else flatten(value, `${prefix}${key}.`, out);
  }
  return out;
}

/** Every backend label with a Chinese translation, by dotted key. */
export const BACKEND_ZH: ReadonlyMap<string, string> = flatten(zhBackend);
