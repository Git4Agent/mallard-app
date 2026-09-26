import Icon from "../Icons";
import type { NoticeItem } from "../notice/Notice";
import { PUSH_FINISHED, finishPushOffered } from "../../unfinishedPush";

/**
 * A Push the user just started that did not happen, as the shell zone under
 * the path bar reads it. The review scrolls as one page with its list, so a
 * box inside it was off screen from wherever the list was left when Push was
 * pressed; the shell zone is not. It offers the one verb that finishes an
 * unfinished push onto this version, reads in the done tone once that verb
 * wrote the record, and closes like every other message: the zone draws the
 * close control, and `onDismiss` clears the review's copy of the failure.
 */
export function pushFailureNotice(
  error: string,
  storageName: string,
  { busy, onFinishPush, onDismiss }: {
    busy: boolean;
    onFinishPush: () => void;
    onDismiss: () => void;
  },
): NoticeItem {
  const finished = error === PUSH_FINISHED;
  return {
    key: "push-failed",
    severity: finished ? "success" : "danger",
    message: error,
    action: finishPushOffered(error, storageName) ? (
      <button type="button" className="btn btn-ghost" onClick={onFinishPush} disabled={busy}>
        <Icon name="upload" size={13} />
        Finish that push
      </button>
    ) : undefined,
    onDismiss,
  };
}
