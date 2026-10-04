import { useEffect, useId, useRef } from "react";
import Icon, { type IconName } from "../Icons";
import type { LinkStop, LinkVerdict } from "./appLink";

/**
 * Open from Mallard Cloud: the sheet every `mallard://` link opens. It says
 * what the link found and where that stands on this computer, and offers one
 * verb. Nothing moves until the verb is pressed; closing the sheet drops the
 * link. The connect dialog's frame around the pickers' found row.
 */

export type AppLinkSheetState =
  | { phase: "looking" }
  | { phase: "answered"; verdict: LinkVerdict };

const STOP_ICON: Record<LinkStop["severity"], IconName> = {
  info: "storage-mallard",
  warning: "alert-triangle",
  danger: "octagon-x",
};

const STOP_VERB: Record<NonNullable<LinkStop["action"]>, string> = {
  set_up: "Set up Mallard Cloud",
  retry: "Try again",
};

export default function AppLinkSheet({ subject, state, onVerb, onClose }: {
  /** `owner/name` as the link spelled it; null for a link that could not be read. */
  subject: string | null;
  state: AppLinkSheetState;
  /** The verb was pressed: go where the verdict says. */
  onVerb: (verdict: LinkVerdict) => void;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => { dialogRef.current?.showModal(); }, []);
  /* `close()` hands focus back; the link is let go here rather than on the
     dialog's own close event, which also covers Esc and may arrive late. */
  const close = () => {
    dialogRef.current?.close();
    onClose();
  };

  const verdict = state.phase === "answered" ? state.verdict : null;
  const verb = !verdict ? null : verdict.kind === "landing" ? verdict.verb : verdict.action ? STOP_VERB[verdict.action] : null;

  return (
    <dialog
      ref={dialogRef}
      className="sync-modal sync-bundle-connection-dialog app-link-sheet"
      aria-labelledby={titleId}
      onClose={onClose}
    >
      <header className="sync-modal-header">
        <div>
          <h1 id={titleId}>Open from Mallard Cloud</h1>
        </div>
        <button type="button" className="btn btn-ghost" onClick={close} aria-label="Close without opening the link">
          <Icon name="x" size={17} />
        </button>
      </header>
      <div className="sync-modal-body repo-connect">
        {!verdict ? (
          <small className="sync-setup-inline-status" role="status">
            <span className="status-loader" /> Looking up {subject}…
          </small>
        ) : verdict.kind === "landing" ? (
          <>
            <div className="r2-step-list">
              <div className="storage-found-list" aria-label="Repository at this address">
                <div className="storage-found-row with-verb">
                  <span className="sync-storage-repository-icon"><Icon name={verdict.found.mark} size={14} /></span>
                  <span className="storage-found-text">
                    <span className="storage-found-name">{verdict.found.address}</span>
                    <span className="storage-found-verdict ok">
                      <Icon name="check-circle" size={12} /> {verdict.found.standing}
                    </span>
                    {verdict.found.sub && (
                      <span className="storage-found-verdict">
                        <Icon name="message" size={12} /> {verdict.found.sub}
                      </span>
                    )}
                  </span>
                </div>
              </div>
            </div>
            <small className="repo-answer-line" role="status">
              <Icon name="computer" size={13} /> {verdict.found.here}
            </small>
          </>
        ) : (
          <div className="r2-step-list">
            <div className="storage-found-empty" role={verdict.severity === "danger" ? "alert" : "status"}>
              <Icon name={STOP_ICON[verdict.severity]} size={24} />
              <strong>{verdict.message}</strong>
            </div>
          </div>
        )}
      </div>
      <footer className="sync-modal-footer">
        <div>
          <button type="button" className="btn" onClick={close}>{verb ? "Cancel" : "Close"}</button>
          {verdict && verb && (
            <button type="button" className="btn btn-primary" onClick={() => onVerb(verdict)}>{verb}</button>
          )}
        </div>
      </footer>
    </dialog>
  );
}
