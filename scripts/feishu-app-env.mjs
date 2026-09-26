// Mallard's Feishu app for local builds. `npm run tauri dev` and
// `npm run tauri build` take MALLARD_FEISHU_APP_ID and
// MALLARD_FEISHU_APP_SECRET from `.env.feishu` at the repository root
// (git-ignored, `NAME=value` lines) when the environment leaves them unset.
// The name starts with `.env.` so Vite's dev server refuses to serve it.
// Rust reads the two names with `option_env!`, and Cargo recompiles the
// crate whenever either value changes (docs/FEISHU_APP_SETUP_GUIDE.md).
import { readFileSync } from "node:fs";

export const FEISHU_APP_ENV_FILE = ".env.feishu";
export const FEISHU_APP_ENV_NAMES = ["MALLARD_FEISHU_APP_ID", "MALLARD_FEISHU_APP_SECRET"];

/** The two names from `NAME=value` lines; blank lines, `#` lines, and empty values are skipped. */
export function parseFeishuAppEnv(text) {
  const values = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const separator = line.indexOf("=");
    if (separator < 0) continue;
    const name = line.slice(0, separator).trim();
    const value = line.slice(separator + 1).trim();
    if (FEISHU_APP_ENV_NAMES.includes(name) && value) values[name] = value;
  }
  return values;
}

/** Fills the names `env` leaves unset or empty from the file; a missing file changes nothing. */
export function applyFeishuAppEnv(env, path = FEISHU_APP_ENV_FILE) {
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch {
    return;
  }
  for (const [name, value] of Object.entries(parseFeishuAppEnv(text))) {
    if (!env[name]) env[name] = value;
  }
}
