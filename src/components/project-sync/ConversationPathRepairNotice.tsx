import type { CodexConversationPathAudit } from "../../types";
import Icon from "../Icons";
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

  return (
    <div
      className={`conversation-path-repair-notice${blocked ? " blocked" : " repairable"}`}
      role="alert"
      aria-label={blocked
        ? undefined
        : `${issueCount} conversation path${issueCount === 1 ? " requires" : "s require"} repair`}
    >
      {blocked ? (
        <>
          <span className="conversation-path-repair-icon">
            <Icon name="alert-triangle" size={15} />
          </span>
          <span className="conversation-path-repair-copy">
            <strong>Conversation paths need review</strong>
            {showScope && <span className="conversation-path-repair-scope">{configurationLabel}</span>}
            <span className="conversation-path-repair-detail">{detail}</span>
          </span>
        </>
      ) : (
        <>
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
          <span
            className="conversation-path-repair-help"
            title={detail}
            aria-label={detail}
            tabIndex={0}
          >
            <Icon name="help-circle" size={14} />
          </span>
        </>
      )}
    </div>
  );
}
