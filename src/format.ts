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
 *
 * Three divergent copies of this used to exist. The one on the Library card
 * had no `B` tier at all and rounded up, so every resource under 1.5 KB
 * reported itself as "1 KB"; it also mixed a decimal threshold with a binary
 * divisor, so sizes just over a megabyte were understated.
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
