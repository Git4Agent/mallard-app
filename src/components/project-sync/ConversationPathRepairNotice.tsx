import type { CodexConversationPathAudit, NoticeSeverity } from "../../types";
import Icon from "../Icons";
import Notice from "../notice/Notice";
import { compactProjectPath } from "./model";

interface Props {
  audit?: CodexConversationPathAudit;
  auditError?: string;
  projectName: string;
  profileName: string;
  profilePath?: string | null;
  showScope?: boolean;
  busy: boolean;
  onRepair: () => Promise<void> | void;
}

export function shouldShowConversationPathRepairNotice(
  audit?: CodexConversationPathAudit,
  auditError?: string,
): boolean {
  return !!auditError || (!!audit && !audit.ready);
}

export default function ConversationPathRepairNotice({
  audit,
  auditError,
  projectName,
  profileName,
  profilePath,
  showScope = true,
  busy,
  onRepair,
}: Props) {
  if (!shouldShowConversationPathRepairNotice(audit, auditError)) return null;

  const issueCount = audit?.issues.length ?? 0;
  const recordedPaths = [...new Set(audit?.issues.map((issue) => issue.recorded_cwd) ?? [])];
  const sourceDescription = recordedPaths.length === 1
    ? compactProjectPath(recordedPaths[0])
    : `${recordedPaths.length} different project paths`;
  const configurationLabel = [
    projectName,
    profileName,
    profilePath ? compactProjectPath(profilePath) : null,
  ].filter(Boolean).join(" · ");
  const blocked = !!auditError || !!audit?.blockers.length;
  const detail = auditError
    ? `Mallard could not verify Codex conversation ownership: ${auditError}`
    : blocked
      ? audit?.blockers[0] ?? "Conversation ownership needs manual review."
      : `${issueCount} ${issueCount === 1 ? "conversation points" : "conversations point"} to ${sourceDescription}. Push and Pull are paused.`;

  /* `danger` is reserved for a thing that failed, and when `auditError` is set
   * the failed thing is Mallard's own audit: it could not read the ownership it
   * needs, so it cannot say whether anything is wrong. Paths it *can* see are a
   * warning — nothing broke, the reader is simply being asked to act before
   * Push and Pull will run again. */
  const severity: NoticeSeverity = auditError ? "danger" : "warning";

  /* The region names the count, not the topic: "3 conversation paths require
   * repair" is what a reader jumping between landmarks needs to hear, and the
   * generic heading it replaced said nothing the card had not already said.
   * The sentence itself is visible now rather than parked in a `title` — a
   * tooltip is not a text equivalent, and the ⓘ that carried it is gone.
   *
   * The audit is loaded with the project — `loadConversationPathAudits` runs as
   * part of the shell load, and this notice renders off the record it left
   * behind — so the sentence is on screen the moment the card first paints.
   * Standing content is not a status message (WCAG 2.2 SC 4.1.3), which is what
   * made the old role="alert" wrong: it interrupted the reader to announce a
   * condition they had never not had. Repairing does not re-time it either; the
   * button reports its own progress in its label. */
  return (
    <Notice
      severity={severity}
      timing="standing"
      label={blocked
        ? "Conversation ownership needs review"
        : `${issueCount} conversation path${issueCount === 1 ? "" : "s"} require${issueCount === 1 ? "s" : ""} repair`}
      className={`conversation-path-repair-notice${blocked ? " blocked" : " repairable"}`}
      title={blocked ? "Conversation paths need review" : undefined}
      action={blocked ? undefined : (
        <button
          type="button"
          className="btn btn-ghost conversation-path-repair-button"
          disabled={busy}
          onClick={() => void onRepair()}
          aria-label={`Repair ${issueCount} conversation path${issueCount === 1 ? "" : "s"}`}
        >
          <Icon
            name={busy ? "refresh" : "alert-triangle"}
            size={14}
            className={`conversation-path-repair-button-icon${busy ? " icon-spin" : ""}`}
          />
          {busy ? "Repairing…" : "Repair"}
        </button>
      )}
    >
      {/* The scope stays a qualifier on the blocked branch only. A repairable
          notice sits inside the configuration it belongs to and never repeated
          the label, and the repair button beside it says which one it means. */}
      {blocked && showScope && (
        <>
          <span className="conversation-path-repair-scope">{configurationLabel}</span>{" "}
        </>
      )}
      {detail}
    </Notice>
  );
}
