import { useEffect, useId, useRef, useState } from "react";
import Icon, { type IconName } from "../Icons";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { useTranslation } from "../../i18n";

/**
 * The sheet Publish version asks in (docs/PLAN_CLOUD_VERSIONS.md), in the
 * frame of the link's sheet: what the version holds, the sessions on this
 * computer it will not hold, and an optional name. Publish freezes the draft
 * that is in Mallard Cloud, for the team; nothing becomes public, and nothing
 * here is sent. Sessions that wait to be pushed are said and do not stop it.
 * One verb; closing the sheet publishes nothing. Once the verb is pressed the
 * one request is on its way: the sheet holds until it is answered, then
 * closes itself, and the page says how it ended.
 */

/** A version's name, as the service bounds it. */
const NAME_CHARS = 80;

/** One thing the version holds, in a glyph and a word or two. */
export interface PublishVersionFact {
  icon: IconName;
  word: string;
}

export default function PublishVersionSheet({ version, holds, sessionsToPush, busy, onPublish, onClose }: {
  /** The number the version gets. */
  version: number;
  /** What the draft in Mallard Cloud holds. */
  holds: PublishVersionFact[];
  /** Sessions here that wait to be pushed: not in the draft, so not in the version. */
  sessionsToPush: number;
  /** The request is on its way: it cannot be called back, so the sheet stays. */
  busy: boolean;
  /** The verb was pressed, with the name as typed or null for none. */
  onPublish: (name: string | null) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation(["cloud", "common"]);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const [name, setName] = useState("");
  useEffect(() => {
    dialogRef.current?.showModal();
    // The one thing to type; Return in it publishes.
    nameRef.current?.focus();
  }, []);
  /* `close()` hands focus back; the sheet is let go here rather than on the
     dialog's own close event, which also covers Esc and may arrive late. */
  const close = () => {
    dialogRef.current?.close();
    onClose();
  };
  const publish = () => {
    if (!busy) onPublish(name.trim() || null);
  };
  // Answered: out through `close()`, so focus returns to where Publish was asked.
  const wasBusy = useRef(busy);
  useEffect(() => {
    if (wasBusy.current && !busy) close();
    wasBusy.current = busy;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [busy]);
  const verb = t("publish.title", { version });

  return (
    <dialog
      ref={dialogRef}
      className="sync-modal app-link-sheet"
      aria-labelledby={titleId}
      onCancel={(event) => { if (busy) event.preventDefault(); }}
      onClose={onClose}
    >
      <header className="sync-modal-header">
        <div><h1 id={titleId}>{verb}</h1></div>
        <button type="button" className="btn btn-ghost" onClick={close} disabled={busy} aria-label={t("publish.close")}>
          <Icon name="x" size={17} />
        </button>
      </header>
      <div className="sync-modal-body app-link-lines">
        {holds.length > 0 && (
          <>
            <span className="app-link-mark" aria-hidden="true"><Icon name="storage-mallard" size={14} /></span>
            <span className="app-link-facts">
              {holds.map((fact, index) => (
                <span key={index} className="app-link-fact"><Icon name={fact.icon} size={12} />{fact.word}</span>
              ))}
            </span>
          </>
        )}
        {sessionsToPush > 0 && (
          <>
            <span className="app-link-mark warning" aria-hidden="true"><ThreadStatusIcon status="local-outstanding" size={16} /></span>
            <span className="app-link-here">
              {t("publish.notPushed", { count: sessionsToPush })}
            </span>
          </>
        )}
        <span className="app-link-mark" aria-hidden="true"><Icon name="edit" size={14} /></span>
        <label className="publish-version-name">
          <span className="sync-visually-hidden">{t("publish.nameLabel")}</span>
          <input
            ref={nameRef}
            type="text"
            value={name}
            maxLength={NAME_CHARS}
            placeholder={t("publish.namePlaceholder")}
            spellCheck={false}
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              // Return that ends an input method's composition is not the verb.
              if (event.key === "Enter" && !event.nativeEvent.isComposing && event.keyCode !== 229) publish();
            }}
          />
        </label>
      </div>
      <footer className="sync-modal-footer">
        <div>
          <button type="button" className="btn" onClick={close} disabled={busy}>{t("common:cancel")}</button>
          <button type="button" className="btn btn-primary" disabled={busy} onClick={publish}>{busy ? t("publish.publishing") : verb}</button>
        </div>
      </footer>
    </dialog>
  );
}
