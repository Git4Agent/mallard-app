import type { StorageConfig } from "./types";
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
 * R2 transport failures arrive as AWS SDK prose — "S3 conditional put
 * 'bundles/…': dispatch failure: io error" — which names no fix. When the
 * storage is S3-backed and the shape matches, say what to check instead.
 */
export function storageAccessErrorMessage(
  storage: Pick<StorageConfig, "kind"> | null | undefined,
  error: unknown,
): string {
  const message = errorMessage(error);
  const looksLikeR2ConnectionFailure = /^S3 (?:get|body|conditional put)/i.test(message)
    && /dispatch failure|service error|timed out|access denied|credentials?|invalid access key|signature|no such bucket/i.test(message);
  if (storage?.kind !== "s3" || !looksLikeR2ConnectionFailure) return message;
  return "Couldn’t connect to Cloudflare R2. Check the S3 API URL, bucket name, Account ID, and credentials, then try again.";
}
