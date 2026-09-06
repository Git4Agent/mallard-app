import type { StorageSummary } from "./types";
import { userFacingRepoTerms } from "./terminology";

/**
 * Turning a backend failure into something a person can act on. A top-level
 * peer of `clock.ts` and `format.ts`, so both feature modules import it
 * without `storage-catalog/` depending on `project-sync/`.
 */

/** Every backend error reaches the screen through here, so the internal
 *  protocol term is translated exactly once. */
export function errorMessage(error: unknown): string {
  return userFacingRepoTerms(error instanceof Error ? error.message : String(error));
}

/**
 * Object-store transport failures arrive as AWS SDK prose — "S3 conditional
 * put 'bundles/…': dispatch failure: io error" — which names no fix. When the
 * shape matches, say what to check instead, naming the storage by the label
 * its provider descriptor gave it.
 */
export function storageAccessErrorMessage(
  storage: Pick<StorageSummary, "provider_label"> | null | undefined,
  error: unknown,
): string {
  const message = errorMessage(error);
  const looksLikeConnectionFailure = /^S3 (?:get|body|conditional put)/i.test(message)
    && /dispatch failure|service error|timed out|access denied|credentials?|invalid access key|signature|no such bucket/i.test(message);
  if (!storage || !looksLikeConnectionFailure) return message;
  return `Couldn’t connect to ${storage.provider_label}. Check the connection settings and credentials, then try again.`;
}
