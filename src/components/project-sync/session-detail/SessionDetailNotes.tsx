import type { SessionDetailWarning } from "../../../types";
import Icon from "../../Icons";
import { highestSeverity, SEVERITY_ICON, SEVERITY_LABEL } from "../../notice/Notice";
import { PathBarPopover } from "../../sync-review/SyncPathBar";

/* Tool calls are off by default, so the note about calls that never recorded
   a result offers to show them. */
const TOOL_CALL_NOTE = "incomplete_tool_call";

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
  const rung = highestSeverity(notes.map((note) => note.severity));
  if (rung === null) return null;
  const count = `${notes.length} ${notes.length === 1 ? "note" : "notes"}`;
  return (
    <>
      <PathBarPopover
        id="session-notes"
        label="Notes on this session"
        className={`session-detail-chip session-detail-notes-chip ${rung}`}
        ariaLabel={`${count} on this session`}
        face={<><Icon name={SEVERITY_ICON[rung]} size={12} />{count}</>}
      >
        {(close) => (
          <div className="session-detail-notes">
            <p className="session-detail-notes-title">Notes on this session</p>
            {notes.map((note) => (
              <p key={`${note.code}:${note.message}`} className={`session-detail-note ${note.severity}`}>
                <Icon name={SEVERITY_ICON[note.severity]} size={12} />
                <span>
                  <b>{SEVERITY_LABEL[note.severity]} ·</b> {note.message}
                  {note.code === TOOL_CALL_NOTE && onShowToolCalls && (
                    <button
                      type="button"
                      className="btn-link session-detail-note-action"
                      onClick={() => {
                        onShowToolCalls();
                        close(true);
                      }}
                    >Show tool calls</button>
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
        {notes.map((note) => `${SEVERITY_LABEL[note.severity]}: ${note.message}`).join(" ")}
      </span>
    </>
  );
}
