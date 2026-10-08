import i18next, { type FormatterModule } from "i18next";
import { initReactI18next } from "react-i18next";

/* Components take these from here, never from react-i18next directly, so
   whatever imports them also initializes the catalogs — a test bundle
   included. */
export { Trans, useTranslation } from "react-i18next";
// Tauri's own door, not `src/ipc.ts`: that module's copy is translated, so
// it imports this one, and the menu report runs while this module loads.
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  getLanguageChoice,
  subscribePreferences,
  type LanguageChoice,
  type UiLanguage,
} from "../preferences";
import { en } from "./en";
import { zhCN } from "./zh-CN";

/**
 * The interface language. Catalogs are bundled TypeScript modules, so
 * i18next initializes synchronously when this module loads — the first
 * render, every test bundle and every mock harness get the same strings
 * without setup. Components translate with `useTranslation`; helpers that
 * run during render call `t` from here.
 *
 * English is the source and the fallback. What the user typed, product
 * names and backend sentences never pass through a catalog.
 */

/* A language is named in its own language, whatever the interface speaks, so
   a reader lost in the current one still finds theirs. */
export const LANGUAGE_NAMES: Readonly<Record<UiLanguage, string>> = {
  // i18n-ignore: each language names itself
  en: "English",
  "zh-CN": "简体中文",
};

/** Every language a catalog exists for, offered or not. */
export const UI_LANGUAGES: readonly UiLanguage[] = ["en", "zh-CN"];

/** Languages Settings offers and System may resolve to. A language joins
 *  when every area of the app is translated, so no build follows the system
 *  into half a language. */
export const OFFERED_LANGUAGES: readonly UiLanguage[] = ["en", "zh-CN"];

const TRADITIONAL = /^zh-(hant|tw|hk|mo)(-|$)/;

/** System is the first English or Chinese entry of the browser's preferred
 *  languages. Traditional Chinese follows English: Simplified text reads as
 *  foreign there, and 简体中文 is one pick away in Settings. */
export function resolveLanguage(
  choice: LanguageChoice,
  preferred: readonly string[] = systemLanguages(),
): UiLanguage {
  if (choice !== "system") return choice;
  for (const tag of preferred) {
    const lower = tag.toLowerCase();
    if (lower === "en" || lower.startsWith("en-")) return "en";
    if (lower === "zh" || lower.startsWith("zh-")) {
      return !TRADITIONAL.test(lower) && OFFERED_LANGUAGES.includes("zh-CN") ? "zh-CN" : "en";
    }
  }
  return "en";
}

function systemLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  return navigator.languages?.length ? navigator.languages : [navigator.language];
}

/*
 * Spacing around values in Chinese. A value set into a Chinese sentence may
 * be a Chinese name or an English one, and the catalog cannot know which:
 * `返回 {{label}}` must read 返回项目 and 返回 Mallard alike. Values are
 * marked as the sentence is built, and the marks then settle each edge —
 * one space between Han and a Latin letter or digit, none between Han and
 * Han —
 * before they go. A value that starts with a digit keeps the catalog's own
 * spacing in front (`{{value}}分钟前` stays tight, `{{count}} 个会话` keeps
 * its space); behind it, a Han ending still drops a space before Han
 * (`将 3 个会话上传到`). A number passed as a number is never marked.
 * English is never marked, so its output is exactly what the catalog says.
 */
const VALUE_OPEN = "\uE000";
const VALUE_CLOSE = "\uE001";
const NUMBER_OPEN = "\uE002";
const NUMBER_CLOSE = "\uE003";
const HAN = "\\u3400-\\u9fff";

export function settleValueSpacing(text: string): string {
  if (!text.includes(VALUE_OPEN) && !text.includes(NUMBER_OPEN)) return text;
  return text
    .replace(new RegExp(`([${HAN}])${NUMBER_CLOSE} (?=[${HAN}])`, "g"), `$1${NUMBER_CLOSE}`)
    .replace(new RegExp(`([A-Za-z])${NUMBER_CLOSE}(?=[${HAN}])`, "g"), `$1${NUMBER_CLOSE} `)
    .replace(new RegExp(`([${HAN}]) ${VALUE_OPEN}(?=[${HAN}])`, "g"), `$1${VALUE_OPEN}`)
    .replace(new RegExp(`([${HAN}])${VALUE_OPEN}(?=[A-Za-z0-9])`, "g"), `$1 ${VALUE_OPEN}`)
    .replace(new RegExp(`([${HAN}])${VALUE_CLOSE} (?=[${HAN}])`, "g"), `$1${VALUE_CLOSE}`)
    .replace(new RegExp(`([A-Za-z0-9])${VALUE_CLOSE}(?=[${HAN}])`, "g"), `$1${VALUE_CLOSE} `)
    .replace(new RegExp(`[${VALUE_OPEN}${VALUE_CLOSE}${NUMBER_OPEN}${NUMBER_CLOSE}]`, "g"), "");
}

/* Every interpolated value passes through here (`alwaysFormat`); the app
   uses no named formats. */
const valueMarker: FormatterModule = {
  type: "formatter",
  init: () => undefined,
  add: () => undefined,
  addCached: () => undefined,
  format: (value: unknown, _format, lng) => {
    if (lng !== "zh-CN" || typeof value !== "string" || value === "") return value as string;
    return /^\d/.test(value) ? `${NUMBER_OPEN}${value}${NUMBER_CLOSE}` : `${VALUE_OPEN}${value}${VALUE_CLOSE}`;
  },
};

void i18next.use(initReactI18next).use(valueMarker).use({
  type: "postProcessor",
  name: "valueSpacing",
  process: (value: string) => settleValueSpacing(value),
}).init({
  resources: { en, "zh-CN": zhCN },
  lng: resolveLanguage(getLanguageChoice()),
  fallbackLng: "en",
  supportedLngs: [...UI_LANGUAGES],
  ns: Object.keys(en),
  defaultNS: "common",
  initAsync: false,
  returnNull: false,
  // React escapes text; i18next escaping would print `&amp;` in English.
  interpolation: { escapeValue: false, alwaysFormat: true },
  postProcess: ["valueSpacing"],
  react: { useSuspense: false },
});

export const i18n = i18next;
export const t = i18next.t.bind(i18next);

export function uiLanguage(): UiLanguage {
  return i18next.language === "zh-CN" ? "zh-CN" : "en";
}

/** The locale `Intl` formats with. English keeps the system's own locale —
 *  what every formatter used before there was a language setting — so an
 *  English interface on a British Mac still prints British dates. */
export function uiLocale(): string | undefined {
  return uiLanguage() === "zh-CN" ? "zh-CN" : undefined;
}

let menuLanguage: UiLanguage | null = null;

/** Switches to what storage holds now and labels the document with it, so
 *  `:lang(zh)` type rules and the webview's own text handling follow; the
 *  native menu bar is rebuilt in the same language. */
export function applyUiLanguage() {
  const language = resolveLanguage(getLanguageChoice());
  if (i18next.language !== language) void i18next.changeLanguage(language);
  if (typeof document !== "undefined") document.documentElement.lang = language;
  // Outside the desktop shell there is no menu bar; nothing to report.
  if (menuLanguage !== language && isTauri()) {
    menuLanguage = language;
    void invoke<void>("set_ui_language", { language }).catch(() => undefined);
  }
}

applyUiLanguage();
subscribePreferences(applyUiLanguage);
// System follows the OS when its language list changes under a running app.
if (typeof window !== "undefined") window.addEventListener("languagechange", applyUiLanguage);
