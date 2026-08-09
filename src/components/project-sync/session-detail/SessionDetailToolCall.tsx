import type {
  SessionEventDetail,
  SessionTimelineEvent,
} from "../../../types";
import Icon from "../../Icons";
import {
  formatSessionDetailTimestamp,
  sessionDetailDateTime,
  SESSION_TOOL_CATEGORY_LABELS,
} from "./sessionDetailModel";

export interface SessionToolDetailState {
  loading: boolean;
  error: string | null;
  detail: SessionEventDetail | null;
}

interface Props {
  event: SessionTimelineEvent;
  expanded: boolean;
  detailState?: SessionToolDetailState;
  compact?: boolean;
  onExpandedChange: (eventId: string, expanded: boolean) => void;
}

function PayloadBlock({
  label,
  value,
  truncated,
}: {
  label: string;
  value?: string | null;
  truncated: boolean;
}) {
  if (!value) return null;
  return (
    <section className="session-detail-tool-payload" aria-label={label}>
      <h4>{label}</h4>
      <pre><code>{value}</code></pre>
      {truncated && <p className="session-detail-truncation">Bounded preview · additional content omitted</p>}
    </section>
  );
}

export default function SessionDetailToolCall({
  event,
  expanded,
  detailState,
  compact = false,
  onExpandedChange,
}: Props) {
  const tool = event.tool;
  if (!tool) {
    return (
      <div className="session-detail-tool-malformed" role="note">
        <Icon name="alert-triangle" size={13} />
        This tool call could not be projected safely.
      </div>
    );
  }

  const expandedDetail = detailState?.detail;
  const affectedPaths = expandedDetail?.affected_paths.length
    ? expandedDetail.affected_paths
    : tool.affected_paths;
  const argumentsValue = expandedDetail?.arguments ?? tool.argument_preview;
  const resultValue = expandedDetail?.result ?? tool.result_preview;
  const argumentsTruncated = expandedDetail?.arguments_truncated ?? tool.argument_truncated;
  const resultTruncated = expandedDetail?.result_truncated ?? tool.result_truncated;

  return (
    <details
      className={`session-detail-tool-call status-${tool.status}${compact ? " compact" : ""}`}
      open={expanded}
      onToggle={(toggleEvent) => {
        const open = toggleEvent.currentTarget.open;
        if (open !== expanded) onExpandedChange(event.event_id, open);
      }}
    >
      <summary>
        <span className="session-detail-tool-disclosure" aria-hidden="true">
          <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
        </span>
        <span className="session-detail-tool-kind">{SESSION_TOOL_CATEGORY_LABELS[tool.category]}</span>
        <strong>{tool.native_name}</strong>
        <span className="session-detail-tool-summary">{tool.summary}</span>
        <span className={`session-detail-tool-status status-${tool.status}`}>{tool.status}</span>
        {event.timestamp != null && (
          <time dateTime={sessionDetailDateTime(event.timestamp)}>
            {formatSessionDetailTimestamp(event.timestamp)}
          </time>
        )}
      </summary>

      <div className="session-detail-tool-body" aria-busy={detailState?.loading || undefined}>
        {detailState?.loading && (
          <div className="session-detail-local-state" role="status">
            <span className="status-loader" /> Loading bounded tool detail…
          </div>
        )}
        {detailState?.error && (
          <div className="session-detail-local-state error" role="alert">
            <Icon name="alert-triangle" size={13} />
            {detailState.error}
          </div>
        )}
        {!detailState?.loading && !detailState?.error && (
          <>
            <PayloadBlock
              label="Arguments"
              value={argumentsValue}
              truncated={argumentsTruncated}
            />
            <PayloadBlock
              label="Result"
              value={resultValue}
              truncated={resultTruncated}
            />
            {!argumentsValue && !resultValue && (
              <p className="session-detail-tool-empty">No text argument or result is available for this call.</p>
            )}
            {affectedPaths.length > 0 && (
              <section className="session-detail-tool-paths" aria-label="Affected project paths">
                <h4>Affected paths</h4>
                <ul>{affectedPaths.map((path) => <li key={path}><code>{path}</code></li>)}</ul>
              </section>
            )}
          </>
        )}
      </div>
    </details>
  );
}
