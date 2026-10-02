import { useEffect, useState } from "react";
import type { ManagedCloudStatus } from "../managed-cloud/types";
import { errorMessage } from "../../storageError";
import { storageCatalogApi } from "./api";

/**
 * One item's Mallard Cloud status, read once when its page opens and again
 * after each action (`readKey`), as a project's is. A status that cannot be
 * read is the status "unavailable", so the chip and the note offer Retry.
 * Nothing is read for an item with no repository, and the list reads none.
 */
export function useItemCloudStatus({ scopeId, storageId, resourceId, repositoryId, readKey }: {
  scopeId: string;
  storageId: string | null;
  resourceId: string | null;
  repositoryId: string | null;
  readKey: unknown;
}) {
  const [status, setStatus] = useState<ManagedCloudStatus | null>(null);
  const [retries, setRetries] = useState(0);
  useEffect(() => {
    setStatus(null);
    if (!storageId || !resourceId || !repositoryId) return;
    let current = true;
    void storageCatalogApi.getCloudStatus(scopeId, storageId, resourceId, repositoryId)
      .then((value) => { if (current) setStatus(value); })
      .catch((reason) => {
        if (current) setStatus({ repository_id: repositoryId, state: "unavailable", link: null, message: errorMessage(reason) });
      });
    return () => { current = false; };
  }, [scopeId, storageId, resourceId, repositoryId, readKey, retries]);
  return { status, retry: () => setRetries((count) => count + 1) };
}
