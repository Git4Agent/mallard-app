import type { SessionDetailWarning } from "../../../types";
import Icon from "../../Icons";
import { highestSeverity, SEVERITY_ICON, severityLabel } from "../../notice/Notice";
import { PathBarPopover } from "../../sync-review/SyncPathBar";
import { useTranslation } from "../../../i18n";
import { backendText } from "../../../i18n/backend";

/* Tool calls are off by default, so the note about calls that never recorded
   a result offers to show them. */
const TOOL_CALL_NOTE = "incomplete_tool_call";

/** A note's sentence: the backend's English, translated by its code. */
const noteText = (note: SessionDetailWarning) => backendText(`sessionWarning.${note.code}`, note.message);

/**
 * How the transcript was read, as one chip at the end of the facts: the
 * loudest rung's glyph and a count, opening the notes in a panel. A caveat is
 * a label on the session, not a message, so it never closes (chips are
 * labels; docs/PLAN_SESSION_DETAIL_NOTICES.md).
 */
export default function SessionDetailNotes({
  notes,
  onShowToolCalls,
}: {
  notes: SessionDetailWarning[];
  /** Null while tool calls are already shown. */
  onShowToolCalls: (() => void) | null;
}) {
  const { t } = useTranslation("session");
  const rung = highestSeverity(notes.map((note) => note.severity));
  if (rung === null) return null;
  const count = notes.length;
  return (
    <>
      <PathBarPopover
        id="session-notes"
        label={t("notes.title")}
        className={`session-detail-chip session-detail-notes-chip ${rung}`}
        ariaLabel={t("notes.onSession", { count })}
        face={<><Icon name={SEVERITY_ICON[rung]} size={12} />{t("notes.count", { count })}</>}
      >
        {(close) => (
          <div className="session-detail-notes">
            <p className="session-detail-notes-title">{t("notes.title")}</p>
            {notes.map((note) => (
              <p key={`${note.code}:${note.message}`} className={`session-detail-note ${note.severity}`}>
                <Icon name={SEVERITY_ICON[note.severity]} size={12} />
                <span>
                  <b>{severityLabel(note.severity)} ·</b> {noteText(note)}
                  {note.code === TOOL_CALL_NOTE && onShowToolCalls && (
                    <button
                      type="button"
                      className="btn-link session-detail-note-action"
                      onClick={() => {
                        onShowToolCalls();
                        close(true);
                      }}
                    >{t("notes.showToolCalls")}</button>
                  )}
                </span>
              </p>
            ))}
          </div>
        )}
      </PathBarPopover>
      {/* The panel exists only while open, at the end of the document, and
          focus enters it only when it holds a button. The sentences are also
          printed here in flow, hidden visually, so a screen reader meets them
          where the chip stands without opening anything. */}
      <span className="sync-visually-hidden">
        {notes.map((note) => t("notes.spoken", { severity: severityLabel(note.severity), message: noteText(note) })).join(" ")}
      </span>
    </>
  );
}
