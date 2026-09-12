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
 * put 'bundles/…': dispatch failure: io error" — which names no fix. The
 * prose can sit behind a caller's prefix ("register the repo in 'X': S3 …"),
 * so it is matched anywhere in the message. When the shape matches, say the
 * cause and the next step instead, naming the storage by the label its
 * provider descriptor gave it. The raw text stays in Logs, where the
 * backend already records it.
 */
export function storageAccessErrorMessage(
  storage: Pick<StorageSummary, "provider_label"> | null | undefined,
  error: unknown,
): string {
  const message = errorMessage(error);
  const kind = storageFailureKind(message);
  if (!storage || !kind) return message;
  const label = storage.provider_label;
  switch (kind) {
    case "unreachable":
      return `Couldn’t reach ${label}. Check the network and try again.`;
    case "credentials":
      return `${label} rejected the credentials. Reconnect the storage with a current key, then try again.`;
    case "missing_bucket":
      return `The ${label} bucket can’t be found. Check the bucket name in the storage settings.`;
    case "service":
      return `Couldn’t connect to ${label}. Check the connection settings and credentials, then try again.`;
  }
}

export type StorageFailureKind = "unreachable" | "credentials" | "missing_bucket" | "service";

/** The cause an S3-shaped failure names, or null when the text is not one. */
export function storageFailureKind(message: string): StorageFailureKind | null {
  if (!/\bS3 (?:get|body|conditional put|list|delete|head)\b/i.test(message)) return null;
  if (/no such bucket|nosuchbucket/i.test(message)) return "missing_bucket";
  if (/access denied|accessdenied|credentials?|invalid access key|invalidaccesskeyid|signature|forbidden|unauthorized/i.test(message)) {
    return "credentials";
  }
  if (/dispatch failure|timed out|connection|io error|dns/i.test(message)) return "unreachable";
  if (/service error/i.test(message)) return "service";
  return null;
}
