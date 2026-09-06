/**
 * Contradictory durable metadata cannot be repaired by retrying the same
 * reviewed Push or Pull. These errors offer the explicit project-scoped
 * local-state reset; transient and ordinary stale-review errors keep Retry.
 */
export function isIrrecoverableSyncError(message: string | null | undefined): boolean {
  if (!message) return false;
  const normalized = message.toLowerCase();
  return normalized.includes("stale at semanticdigest")
    || normalized.includes("stale at lineagedigest")
    || normalized.includes("changed lineage or semantic content");
}
