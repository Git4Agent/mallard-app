import { t } from "./i18n";
import { backendText } from "./i18n/backend";
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
 * Object-store transport failures arrive as AWS SDK prose — "S3 put
 * 'bundles/…': dispatch failure: io error" — which names no fix. The
 * prose can sit behind a caller's prefix ("register the repo in 'X': S3 …"),
 * so it is matched anywhere in the message. When the shape matches, say the
 * cause and the next step instead, naming the storage by the label its
 * provider descriptor gave it — in Chinese, looked up by the provider key
 * when the caller has it. The raw text stays in Logs, where the backend
 * already records it.
 */
export function storageAccessErrorMessage(
  storage: (Pick<StorageSummary, "provider_label"> & { provider?: string; available?: boolean }) | null | undefined,
  error: unknown,
): string {
  const message = errorMessage(error);
  const kind = storageFailureKind(message);
  if (!storage || !kind) return message;
  // A storage with no adapter carries its raw key, which stays.
  const label = storage.provider && storage.available !== false
    ? backendText(`storageProvider.${storage.provider}.displayName`, storage.provider_label)
    : storage.provider_label;
  switch (kind) {
    case "unreachable":
      return t("shell:storageError.unreachable", { label });
    case "credentials":
      return t("shell:storageError.credentials", { label });
    case "missing_bucket":
      return t("shell:storageError.missingBucket", { label });
    case "service":
      return t("shell:storageError.service", { label });
  }
}

export type StorageFailureKind = "unreachable" | "credentials" | "missing_bucket" | "service";

/** The cause an S3-shaped failure names, or null when the text is not one.
 *  It reads the backend's English, which stays English in every language. */
export function storageFailureKind(message: string): StorageFailureKind | null {
  if (!/\bS3 (?:get|body|put|record put|list|delete|head)\b/i.test(message)) return null;
  if (/no such bucket|nosuchbucket/i.test(message)) return "missing_bucket";
  if (/access denied|accessdenied|credentials?|invalid access key|invalidaccesskeyid|signature|forbidden|unauthorized/i.test(message)) {
    return "credentials";
  }
  if (/dispatch failure|timed out|connection|io error|dns/i.test(message)) return "unreachable";
  if (/service error/i.test(message)) return "service";
  return null;
}
