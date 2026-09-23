/**
 * A push onto the selected version that claimed it and never wrote its
 * record (CAS-free plan, section 13). The backend sends one of these two
 * sentences as the Push error, word for word, and the Push review and the
 * Library page then offer the one verb that finishes it: the record the
 * claim holds is written once, and nothing runs that by itself. A top-level
 * peer of `storageError.ts`, so both feature modules import it.
 */

export function unfinishedPushSentence(storageName: string): string {
  return `A push to ${storageName} did not finish. Finish it, then compare again.`;
}

export function ownUnfinishedPushSentence(storageName: string): string {
  return `This computer's earlier push to ${storageName} did not finish. Finish it, then compare again.`;
}

/** What the banner reads once the action wrote the record. */
export const PUSH_FINISHED = "Finished. Compare again.";

/** Whether `message` is one of the two sentences for `storageName`. */
export function finishPushOffered(
  message: string | null | undefined,
  storageName: string | null | undefined,
): boolean {
  if (!message || !storageName) return false;
  return message === unfinishedPushSentence(storageName)
    || message === ownUnfinishedPushSentence(storageName);
}
