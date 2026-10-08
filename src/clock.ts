import { t, uiLocale } from "./i18n";
import { getPreference } from "./preferences";

/**
 * Hour-and-minute fields for every timestamp the app prints, so all surfaces
 * use one clock policy.
 *
 * With the 24-hour preference on (the default), the hour is `2-digit` under
 * `hourCycle: "h23"` — not `hour12: false`, which under en-US selects the h24
 * cycle on some ICU builds and renders midnight as `24:06`; h23 names the
 * cycle outright and reads the same everywhere, and the padded hour keeps
 * fixed-width columns fixed. Off leaves cycle and padding to the locale.
 */
export function clockOptions(withSeconds = false): Intl.DateTimeFormatOptions {
  const base: Intl.DateTimeFormatOptions = getPreference("clock24")
    ? { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }
    : { hour: "numeric", minute: "2-digit" };
  if (withSeconds) base.second = "2-digit";
  return base;
}

/**
 * Timestamps reach the frontend as epoch seconds from some surfaces and epoch
 * milliseconds from others. Anything past the year 2286 in seconds is really
 * milliseconds, which is the discriminator every caller here relies on.
 */
export function toEpochMilliseconds(value: number): number {
  return value > 10_000_000_000 ? value : value * 1_000;
}

/** Under a day, how long ago: `just now`, `5m ago`, `3h ago`; null past
 *  that, where callers print a date instead. */
export function formatElapsed(milliseconds: number): string | null {
  if (milliseconds < 60_000) return t("format:justNow");
  if (milliseconds < 3_600_000) return t("format:minutesAgo", { value: Math.floor(milliseconds / 60_000) });
  if (milliseconds < 86_400_000) return t("format:hoursAgo", { value: Math.floor(milliseconds / 3_600_000) });
  return null;
}

/** How long ago, coarsely: `just now`, `5m ago`, `3h ago`, then the date.
 *  Seconds or milliseconds, as `toEpochMilliseconds` reads them. */
export function formatRelativeTime(epoch?: number): string {
  if (!epoch) return t("format:unknown");
  return formatElapsed(Date.now() - toEpochMilliseconds(epoch))
    ?? new Date(toEpochMilliseconds(epoch)).toLocaleDateString(uiLocale());
}

export function formatDateTime(milliseconds: number): string {
  return new Date(milliseconds).toLocaleString(uiLocale(), {
    month: "short",
    day: "numeric",
    year: "numeric",
    ...clockOptions(),
  });
}

/* Built once per locale and clock setting: constructing an Intl formatter
   costs far more than formatting with one. */
const fullDateFormatters = new Map<string, Intl.DateTimeFormat>();
const fullClockFormatters = new Map<string, Intl.DateTimeFormat>();

export function formatFullDateTime(seconds: number): string {
  const when = new Date(seconds * 1_000);
  const locale = uiLocale();
  const dateKey = locale ?? "";
  let dateFormatter = fullDateFormatters.get(dateKey);
  if (!dateFormatter) {
    dateFormatter = new Intl.DateTimeFormat(locale, { month: "short", day: "numeric", year: "numeric" });
    fullDateFormatters.set(dateKey, dateFormatter);
  }
  const clockKey = `${dateKey}|${getPreference("clock24")}`;
  let clockFormatter = fullClockFormatters.get(clockKey);
  if (!clockFormatter) {
    clockFormatter = new Intl.DateTimeFormat(locale, clockOptions());
    fullClockFormatters.set(clockKey, clockFormatter);
  }
  return t("format:dateAtTime", { date: dateFormatter.format(when), time: clockFormatter.format(when) });
}

/**
 * A span at one significant unit: `45s`, `11m`, `1h 5m`, `2d 3h`. Null when
 * there is nothing measurable to show, so a zero-length span prints no chip
 * rather than "0s".
 */
export function formatDuration(from: number, to: number): string | null {
  const seconds = Math.max(
    0,
    Math.round((toEpochMilliseconds(to) - toEpochMilliseconds(from)) / 1_000),
  );
  if (seconds === 0) return null;
  if (seconds < 60) return t("format:seconds", { value: seconds });
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t("format:minutes", { value: minutes });
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  if (hours < 24) {
    return restMinutes
      ? t("format:hoursMinutes", { hours, minutes: restMinutes })
      : t("format:hours", { value: hours });
  }
  const days = Math.floor(hours / 24);
  const restHours = hours % 24;
  return restHours ? t("format:daysHours", { days, hours: restHours }) : t("format:days", { value: days });
}
