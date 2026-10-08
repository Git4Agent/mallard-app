import { useSyncExternalStore } from "react";
import type { AppTheme } from "./types";

/**
 * Global preferences: one typed, localStorage-backed store, read and written
 * only through here. Each preference is one key under the `mallard.*`
 * namespace, every read carries a hardcoded fallback, and every write
 * notifies subscribers — components stay live through the hooks below.
 *
 * Theme lives here too. Absence means System, so a fresh install follows the
 * OS instead of forcing dark.
 */

export type ThemeChoice = "system" | AppTheme;

/** The interface language: the system's, or one picked in Settings. */
export type UiLanguage = "en" | "zh-CN";
export type LanguageChoice = "system" | UiLanguage;

export type BooleanPreferenceName =
  | "openLogsOnSync"
  | "refreshOnFocus"
  | "clock24";

const THEME_KEY = "mallard.theme";
const LANGUAGE_KEY = "mallard.language";

/** Sync preferences default off; the 24-hour clock defaults on. */
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

/**
 * Whether the value persisted. Every reader goes back to storage, so a
 * rejected write leaves the previous value in force — the Settings page says
 * "Not saved" on the strength of this answer.
 */
function writeKey(key: string, value: string): boolean {
  try {
    window.localStorage.setItem(key, value);
    return true;
  } catch {
    return false;
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

/**
 * Returns whether the choice was saved. The page paints from what storage
 * holds afterwards, not from the request: a rejected write would otherwise
 * leave the window in a theme the control does not show.
 */
export function setThemeChoice(choice: ThemeChoice): boolean {
  const saved = writeKey(THEME_KEY, choice);
  applyThemeChoice(saved ? choice : getThemeChoice());
  notify();
  return saved;
}

export function getLanguageChoice(): LanguageChoice {
  const stored = readKey(LANGUAGE_KEY);
  return stored === "en" || stored === "zh-CN" ? stored : "system";
}

/** Returns whether the choice was saved; `src/i18n` applies what storage
 *  then holds, the way the theme does. */
export function setLanguageChoice(choice: LanguageChoice): boolean {
  const saved = writeKey(LANGUAGE_KEY, choice);
  notify();
  return saved;
}

export function getPreference(name: BooleanPreferenceName): boolean {
  const stored = readKey(BOOLEAN_KEYS[name]);
  return name === "clock24" ? stored !== "off" : stored === "on";
}

/** Returns whether the value was saved; subscribers re-read either way. */
export function setPreference(name: BooleanPreferenceName, value: boolean): boolean {
  const saved = writeKey(BOOLEAN_KEYS[name], value ? "on" : "off");
  notify();
  return saved;
}

function warningDismissalKey(projectId: string, storageId: string): string {
  return `mallard.dismissed-storage-warning.${JSON.stringify([projectId, storageId])}`;
}

/** Save only the UI acknowledgement. Sync records are unchanged. */
export function dismissStorageWarning(projectId: string, storageId: string, condition: string): boolean {
  const saved = writeKey(warningDismissalKey(projectId, storageId), condition);
  notify();
  return saved;
}

export function useDismissedStorageWarning(projectId: string, storageId: string): string | null {
  const read = () => readKey(warningDismissalKey(projectId, storageId));
  return useSyncExternalStore(subscribePreferences, read, read);
}

export function clearStorageWarningDismissal(projectId: string, storageId: string) {
  try {
    window.localStorage.removeItem(warningDismissalKey(projectId, storageId));
  } catch {
    return;
  }
  notify();
}

/**
 * Factory reset: drop every `mallard.*` key — a prefix sweep, not a list,
 * because keys live in three modules and future ones must be covered without
 * remembering this function exists.
 * Called by the Settings page after a successful `~/.mallard` wipe, right
 * before the app restarts.
 */
export function clearStoredUiState() {
  try {
    const doomed: string[] = [];
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (key?.startsWith("mallard.")) doomed.push(key);
    }
    for (const key of doomed) window.localStorage.removeItem(key);
  } catch {
    // A locked-down webview can reject storage; the restart still lands on
    // defaults-or-stale, which the app tolerates either way.
  }
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

export function useLanguageChoice(): LanguageChoice {
  return useSyncExternalStore(subscribePreferences, getLanguageChoice, getLanguageChoice);
}
