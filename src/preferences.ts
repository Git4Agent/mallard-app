import { useSyncExternalStore } from "react";
import type { AppTheme } from "./types";

/**
 * Global preferences: one typed, localStorage-backed store, read and written
 * only through here. Each preference is one key under the `mallard.*`
 * namespace, every read carries a hardcoded fallback, and every write
 * notifies subscribers — components stay live through the hooks below.
 *
 * Theme lives here too (this module absorbed theme.ts). The legacy
 * `agent-sync-theme` key is abandoned, not migrated: absence now means
 * System, so a fresh install — and the first launch after this cutover —
 * follows the OS instead of forcing dark.
 */

export type ThemeChoice = "system" | AppTheme;

export type BooleanPreferenceName =
  | "openLogsOnSync"
  | "refreshOnFocus"
  | "clock24";

const THEME_KEY = "mallard.theme";

/** All three default on: each preserves the behavior that was hardcoded
    before it became a preference. */
const BOOLEAN_KEYS: Record<BooleanPreferenceName, string> = {
  openLogsOnSync: "mallard.open-logs-on-sync",
  refreshOnFocus: "mallard.refresh-on-focus",
  clock24: "mallard.clock-24h",
};

const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) listener();
}

/** A locked-down webview can reject storage; the defaults still apply. */
function readKey(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeKey(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // The in-memory value still notified; only persistence is lost.
  }
}

export function getThemeChoice(): ThemeChoice {
  const stored = readKey(THEME_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

export function systemTheme(): AppTheme {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } catch {
    return "dark";
  }
}

export function resolveTheme(choice: ThemeChoice): AppTheme {
  return choice === "system" ? systemTheme() : choice;
}

export function applyResolvedTheme(theme: AppTheme) {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
}

/** Boot-time paint, before React mounts, so light mode does not flash dark. */
export function applyThemeChoice(choice: ThemeChoice) {
  applyResolvedTheme(resolveTheme(choice));
}

export function setThemeChoice(choice: ThemeChoice) {
  writeKey(THEME_KEY, choice);
  applyThemeChoice(choice);
  notify();
}

export function getPreference(name: BooleanPreferenceName): boolean {
  return readKey(BOOLEAN_KEYS[name]) !== "off";
}

export function setPreference(name: BooleanPreferenceName, value: boolean) {
  writeKey(BOOLEAN_KEYS[name], value ? "on" : "off");
  notify();
}

export function subscribePreferences(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

// The third getter serves server-style renders — the frontend test harness
// renders pages with renderToStaticMarkup, which requires it.
export function usePreference(name: BooleanPreferenceName): boolean {
  return useSyncExternalStore(subscribePreferences, () => getPreference(name), () => getPreference(name));
}

export function useThemeChoice(): ThemeChoice {
  return useSyncExternalStore(subscribePreferences, getThemeChoice, getThemeChoice);
}
