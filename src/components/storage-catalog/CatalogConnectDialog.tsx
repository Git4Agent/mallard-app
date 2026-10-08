import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "../../i18n";
import Icon from "../Icons";
import Notice from "../notice/Notice";
import { errorMessage } from "../../storageError";
import { storageCatalogApi } from "./api";
import { useCloudRepositoryChoice, type CloudChoiceTarget } from "./CloudRepositoryChoice";
import type { CatalogNouns } from "./model";
import type { CatalogCapabilityDescriptor, CatalogCoordinate } from "./types";

/**
 * A Catalog item's repository, chosen the way a project's is: the project's
 * connect dialog with fewer words. Three reasons open it:
 *
 * - `connect`: an item with no repository; a new private one is offered
 *   first, and Connect on it creates it now, as a project's Link does.
 * - `several`: two repositories the account can see hold the item.
 * - `add`: someone else's repository of this row's kind, shared with you or
 *   by address or invitation code.
 *
 * Whichever it is, the repository is fixed once saved, and the footer says
 * so before anything is. Nothing is pulled.
 */

export type CatalogConnectReason =
  | { kind: "connect"; resourceId: string; name: string }
  | { kind: "several"; resourceId: string; name: string }
  | { kind: "add" };

export interface CatalogConnected {
  resourceId: string;
  name: string;
  address: string;
}

export default function CatalogConnectDialog({
  reason, scopeId, storageId, accountLogin, coordinate, nouns, rowLabel, listed, capabilities, onClose, onConnected,
}: {
  reason: CatalogConnectReason;
  scopeId: string;
  storageId: string;
  accountLogin: string;
  coordinate: CatalogCoordinate;
  /** The kind's words. */
  nouns: CatalogNouns;
  rowLabel: string;
  /** The items the row lists, which Add leaves out. */
  listed: ReadonlySet<string>;
  /** For the words a misfit names another kind by. */
  capabilities: readonly CatalogCapabilityDescriptor[];
  onClose: () => void;
  onConnected: (connected: CatalogConnected) => void;
}) {
  const { t } = useTranslation(["library", "common"]);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => { dialogRef.current?.showModal(); }, []);
  const [working, setWorking] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const target: CloudChoiceTarget = reason.kind === "add"
    ? { kind: "add", coordinate, nouns, rowLabel, listed }
    : { kind: "item", coordinate, nouns, resourceId: reason.resourceId, name: reason.name, offerNew: reason.kind === "connect" };
  const choice = useCloudRepositoryChoice({
    storageId,
    scopeId,
    accountLogin,
    target,
    working,
    capabilities,
    list: () => (reason.kind === "add"
      ? storageCatalogApi.listCloudShared(storageId)
      : storageCatalogApi.listCloudRepositories(scopeId, storageId, reason.resourceId)),
    listKey: `${scopeId}:${storageId}:${reason.kind === "add" ? "shared" : reason.resourceId}`,
  });
  const { selection, chosen, ready } = choice;
  const adding = reason.kind === "add";

  const confirm = async () => {
    if (!selection || working) return;
    setWorking(true);
    setFailure(null);
    try {
      // Connecting needs the row's one pairing with the storage; linking is
      // configuration only, and is already there after any Push or Pull.
      const link = await storageCatalogApi.linkScopeStorage(scopeId, storageId);
      if (selection === "new") {
        if (reason.kind === "add") return;
        const created = await storageCatalogApi.createCloudRepository(scopeId, storageId, reason.resourceId, link.revision);
        if (mounted.current) onConnected({ resourceId: reason.resourceId, name: reason.name, address: created.address });
        return;
      }
      await storageCatalogApi.connectCloudRepository(scopeId, storageId, selection.resource_id, selection.repository_id, link.revision);
      if (mounted.current) {
        onConnected({
          resourceId: selection.resource_id,
          name: reason.kind === "add" ? selection.display_name : reason.name,
          address: selection.address,
        });
      }
    } catch (cause) {
      if (mounted.current) setFailure(errorMessage(cause));
    } finally {
      if (mounted.current) setWorking(false);
    }
  };

  const close = () => {
    if (!working) dialogRef.current?.close();
  };
  const title = reason.kind === "add"
    ? t("connect.addTitle")
    : reason.kind === "several" ? t("connect.chooseTitle") : t("connect.connectTitle", { name: reason.name });
  const verb = adding ? t("add.add") : t("connect.connect");
  /* Said before the choice is made, whichever it is. */
  const note = selection && (selection !== "new" || ready) ? t("add.fixed") : null;

  return (
    <dialog
      ref={dialogRef}
      className="sync-modal sync-bundle-connection-dialog"
      aria-labelledby={titleId}
      onCancel={(event) => { if (working) event.preventDefault(); }}
      onClose={onClose}
    >
      <header className="sync-modal-header">
        <div>
          <h1 id={titleId}>{title}</h1>
          {reason.kind === "several" && (
            <p>{ready
              ? t("connect.inCount", { name: reason.name, count: choice.rows.length })
              : t("connect.inSeveral", { name: reason.name })}</p>
          )}
        </div>
        <button type="button" className="btn btn-ghost" onClick={close} disabled={working} aria-label={t("connect.close")}>
          <Icon name="x" size={17} />
        </button>
      </header>
      <div className="sync-modal-body repo-connect">
        {choice.field}
        {choice.found}
        {choice.list}
        {failure && <Notice severity="danger" timing="blocking">{failure}</Notice>}
      </div>
      <footer className="sync-modal-footer">
        {note ? <span>{note}</span> : null}
        <div>
          <button type="button" className="btn" onClick={close} disabled={working}>{t("common:cancel")}</button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={working || !selection || (selection === "new" && !ready) || (adding && !chosen)}
            onClick={() => void confirm()}
          >
            {working ? (adding ? t("add.adding") : t("connect.connecting")) : verb}
          </button>
        </div>
      </footer>
    </dialog>
  );
}
