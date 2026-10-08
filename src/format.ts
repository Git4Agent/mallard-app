import { uiLanguage, uiLocale } from "./i18n";

/**
 * Formatting shared by both feature modules. A top-level peer of `clock.ts`,
 * so `storage-catalog/` can use it without importing from `project-sync/` —
 * the boundary those two keep deliberately.
 */

/**
 * Binary units, because every producer of these numbers is a filesystem or an
 * object store. One decimal below ten units and none above it, so a size reads
 * precisely where precision matters and stays short where it does not:
 * `640 B`, `1.5 KB`, `45 KB`, `2.3 MB`, `118 MB`.
 */
export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "0 B";
  if (value < 1024) return `${Math.round(value)} B`;
  const kilobytes = value / 1024;
  if (kilobytes < 1024) return `${kilobytes.toFixed(kilobytes < 10 ? 1 : 0)} KB`;
  const megabytes = kilobytes / 1024;
  if (megabytes < 1024) return `${megabytes.toFixed(megabytes < 10 ? 1 : 0)} MB`;
  const gigabytes = megabytes / 1024;
  return `${gigabytes.toFixed(gigabytes < 10 ? 1 : 0)} GB`;
}

/** A count with the interface language's grouping: `9,432`. */
export function formatNumber(value: number): string {
  return new Intl.NumberFormat(uiLocale()).format(value);
}

/**
 * The one compaction rule for counts of tokens and turns, everywhere they
 * appear: exact below 10K, whole K to 1M, hundredths of M above. The activity
 * row a session is launched from and the page it opens must agree on the
 * number — two thresholds made 9,432 read "9.4K" in one and "9,432" in the
 * other. Chinese counts in 万 and 亿, one decimal, past the same 10,000.
 */
export function formatCompactNumber(value: number): string {
  if (value < 10_000) return formatNumber(value);
  if (uiLanguage() === "zh-CN") {
    return new Intl.NumberFormat("zh-CN", { notation: "compact", maximumFractionDigits: 1 }).format(value);
  }
  return value >= 1_000_000 ? `${(value / 1_000_000).toFixed(2)}M` : `${Math.round(value / 1_000)}K`;
}

/** "A and B and C" as English has always joined them here; Chinese lists
 *  read A、B和C. */
export function joinAnd(parts: readonly string[]): string {
  if (uiLanguage() !== "zh-CN") return parts.join(" and ");
  if (parts.length < 2) return parts.join("");
  // 和 is a Han word: it takes a space against a Latin word or a digit.
  const head = parts.slice(0, -1).join("、");
  const tail = parts[parts.length - 1];
  const before = /[A-Za-z0-9]$/.test(head) ? " " : "";
  const after = /^[A-Za-z0-9]/.test(tail) ? " " : "";
  return `${head}${before}和${after}${tail}`;
}
