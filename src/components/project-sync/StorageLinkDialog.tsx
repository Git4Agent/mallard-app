import { useEffect, useRef } from "react";
import type { StorageSummary } from "../../types";
import Icon, { iconNameFromHint } from "../Icons";
import { storageConnectionIssue, storageSubtitle, storageSubtitleIsPath } from "./StorageSettings";

/**
 * Which storage a project is linked to, asked as the app asks every other
 * question of this size: a native `<dialog>` over an inert page.
 *
 * The rows carry what you choose with — the storage's own identifier, which
 * is what tells two R2 buckets or two Dropbox folders apart — and nothing
 * that costs a scan: name, provider, `summary[0]`, and the recorded
 * connection issue, all registry-level facts a listing already holds.
 *
 * Linking is one click and reversible, so a row commits on activation; there
 * is no selection to confirm. Add storage lives in the footer because the
 * card's own Add button is behind the backdrop while this is open.
 */
interface Props {
  projectName: string;
  /** Storages this build can open that the project is not linked to yet. */
  storages: StorageSummary[];
  busy: boolean;
  onLink: (storageId: string) => void;
  onAddStorage: () => void;
  onCancel: () => void;
}

export default function StorageLinkDialog({
  projectName,
  storages,
  busy,
  onLink,
  onAddStorage,
  onCancel,
}: Props) {
  // Native <dialog> + showModal() supplies focus trap, Escape, inert
  // background, and focus restoration.
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialogRef.current;
    if (!element) return;
    element.showModal();
    /* showModal() focuses the first focusable descendant, which is the close
       ✕ in the header — it opens a chooser with the keyboard already on
       "dismiss". The first destination is what this dialog is for, so focus
       goes there instead. */
    element.querySelector<HTMLButtonElement>(".sync-storage-link-list button")?.focus();
  }, []);
  /* Every way out goes through close(): the platform restores focus to the
     opener on the close steps, and unmounting the element instead only takes
     it out of the top layer — Escape would return focus and the buttons
     would drop it on the floor. */
  const dismiss = () => {
    const element = dialogRef.current;
    if (element?.open) element.close();
    else onCancel();
  };
  return (
    <dialog
      ref={dialogRef}
      className="sync-modal sync-storage-link-dialog"
      aria-labelledby="sync-storage-link-title"
      onCancel={(event) => { if (busy) event.preventDefault(); }}
      onClose={onCancel}
    >
      <header className="sync-modal-header">
        <div>
          <span className="sync-eyebrow">{projectName}</span>
          <h1 id="sync-storage-link-title">Link a storage</h1>
          <p>
            Where this project&apos;s sessions are pushed and pulled. A project can
            be linked to more than one.
          </p>
        </div>
        <button
          type="button"
          className="btn btn-ghost"
          onClick={dismiss}
          disabled={busy}
          aria-label="Close storage chooser"
        >
          <Icon name="x" size={17} />
        </button>
      </header>

      <div className="sync-modal-body">
        <ul className="sync-storage-link-list">
          {storages.map((storage) => {
            const name = storage.name || "(unnamed)";
            const subtitle = storageSubtitle(storage);
            const reconnect = storageConnectionIssue(storage);
            return (
              <li key={storage.storage_id}>
                <button
                  type="button"
                  className="sync-storage-index-row"
                  disabled={busy}
                  aria-label={[
                    `Link ${name}`,
                    `${storage.provider_label}, ${subtitle}`,
                    reconnect ? `Storage issue: ${reconnect}` : null,
                  ].filter(Boolean).join(". ")}
                  onClick={() => onLink(storage.storage_id)}
                >
                  <span className="sync-storage-link-mark" aria-hidden="true">
                    <Icon name={iconNameFromHint(storage.icon, "drive")} size={13} />
                  </span>
                  <span className="sync-storage-index-copy">
                    <strong>{name}</strong>
                    {/* A path keeps the mono face; any other fact is prose. */}
                    <small className={storageSubtitleIsPath(storage) ? "path" : undefined}>
                      {storage.provider_label} · {subtitle}
                    </small>
                  </span>
                  {reconnect ? (
                    <span className="sync-storage-index-warning">
                      <Icon name="alert-triangle" size={12} aria-hidden="true" />
                      <span>Reconnect</span>
                    </span>
                  ) : <span />}
                  <span />
                  <Icon name="chevron-right" size={12} className="sync-storage-index-open" aria-hidden="true" />
                </button>
              </li>
            );
          })}
        </ul>
      </div>

      <footer className="sync-modal-footer">
        <span>{storages.length} available</span>
        <div>
          <button type="button" className="btn btn-ghost" onClick={onAddStorage} disabled={busy}>
            <Icon name="plus" size={13} />
            Add storage
          </button>
          <button type="button" className="btn" onClick={dismiss} disabled={busy}>Cancel</button>
        </div>
      </footer>
    </dialog>
  );
}
