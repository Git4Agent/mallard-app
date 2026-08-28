import { getPreference } from "./preferences";

/**
 * Hour-and-minute fields for every timestamp the app prints, so the clock is
 * decided in one place. Before this, four surfaces disagreed: the session
 * timeline was 24-hour by design while logs, the catalog, and the activity
 * page each followed the locale their own way.
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

export function formatDateTime(milliseconds: number): string {
  return new Date(milliseconds).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    ...clockOptions(),
  });
}

let fullDateFormatter: Intl.DateTimeFormat | null = null;
let fullClockFormatter: Intl.DateTimeFormat | null = null;
let fullClockFormatterIs24Hour: boolean | null = null;

export function formatFullDateTime(seconds: number): string {
  const when = new Date(seconds * 1_000);
  fullDateFormatter ??= new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  const clock24 = getPreference("clock24");
  if (fullClockFormatter === null || fullClockFormatterIs24Hour !== clock24) {
    fullClockFormatter = new Intl.DateTimeFormat(undefined, clockOptions());
    fullClockFormatterIs24Hour = clock24;
  }
  return `${fullDateFormatter.format(when)} at ${fullClockFormatter.format(when)}`;
}

/**
 * A span at one significant unit: `45s`, `11m`, `1h 5m`, `2d 3h`. Null when
 * there is nothing measurable to show, so a zero-length span prints no chip
 * rather than "0s".
 *
 * Two copies of this used to disagree. The session timeline's normalised its
 * units but stopped at hours, so a two-day session read "51h"; the activity
 * page's had the day tier but assumed seconds, so a millisecond timestamp came
 * out a thousand times too long. This has both.
 */
export function formatDuration(from: number, to: number): string | null {
  const seconds = Math.max(
    0,
    Math.round((toEpochMilliseconds(to) - toEpochMilliseconds(from)) / 1_000),
  );
  if (seconds === 0) return null;
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  if (hours < 24) return restMinutes ? `${hours}h ${restMinutes}m` : `${hours}h`;
  const days = Math.floor(hours / 24);
  const restHours = hours % 24;
  return restHours ? `${days}d ${restHours}h` : `${days}d`;
}
