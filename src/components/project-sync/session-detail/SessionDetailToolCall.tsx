import { useMemo, useState } from "react";
import type { SessionEventDetail, SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import { readableToolPayload, toolCommandLabel } from "./sessionDetailModel";

export interface SessionToolDetailState {
  loading: boolean;
  error: string | null;
  detail: SessionEventDetail | null;
}

interface Props {
  event: SessionTimelineEvent;
  expanded: boolean;
  detailState?: SessionToolDetailState;
  onExpandedChange: (eventId: string, expanded: boolean) => void;
}

type PayloadTabId = "output" | "arguments" | "paths";

interface PayloadTab {
  id: PayloadTabId;
  label: string;
}

async function copyText(value: string): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // Some WebViews expose Clipboard but still reject it. Fall through.
    }
  }
  if (typeof document === "undefined") return;
  const field = document.createElement("textarea");
  field.value = value;
  field.readOnly = true;
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.appendChild(field);
  field.select();
  try {
    document.execCommand("copy");
  } finally {
    field.remove();
  }
}

/**
 * Arguments, output and paths share one bounded pane instead of stacking three
 * unbounded blocks. Stacked, a single call could insert several screens between
 * one turn and the next; tabbed and capped, the transcript stays put.
 */
function ToolPayload({
  argumentsValue,
  argumentsTruncated,
  resultValue,
  resultTruncated,
  affectedPaths,
}: {
  argumentsValue: string | null;
  argumentsTruncated: boolean;
  resultValue: string | null;
  resultTruncated: boolean;
  affectedPaths: string[];
}) {
  const tabs: PayloadTab[] = [
    ...(resultValue ? [{ id: "output" as const, label: "Output" }] : []),
    ...(argumentsValue ? [{ id: "arguments" as const, label: "Arguments" }] : []),
    ...(affectedPaths.length > 0 ? [{ id: "paths" as const, label: "Paths" }] : []),
  ];
  const [active, setActive] = useState<PayloadTabId | null>(null);
  const [showRaw, setShowRaw] = useState(false);

  const current = active && tabs.some((tab) => tab.id === active) ? active : tabs[0]?.id ?? null;
  const source = current === "arguments" ? argumentsValue : current === "output" ? resultValue : null;
  const view = useMemo(() => (source ? readableToolPayload(source) : null), [source]);
  const truncated = current === "arguments" ? argumentsTruncated : current === "output" ? resultTruncated : false;

  if (tabs.length === 0) {
    return <p className="session-detail-tool-empty">No text argument or result is available for this call.</p>;
  }

  const body = view ? (showRaw && view.transformed ? view.raw : view.text) : "";

  return (
    <div className="session-detail-tool-payload">
      <div className="session-detail-payload-tabs" role="tablist" aria-label="Tool call payload">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            className={`session-detail-payload-tab${tab.id === current ? " is-on" : ""}`}
            aria-selected={tab.id === current}
            onClick={() => { setActive(tab.id); setShowRaw(false); }}
          >
            {tab.label}
          </button>
        ))}
        <span className="session-detail-payload-tools">
          {/* Deliberately not `.btn`: that class carries the rounded ghost
              treatment, and these sit in a squared-off panel. */}
          {view?.transformed && (
            <button
              type="button"
              className={`session-detail-payload-action${showRaw ? " is-on" : ""}`}
              aria-pressed={showRaw}
              title={showRaw ? "Show readable text" : "Show the stored value"}
              onClick={() => setShowRaw((raw) => !raw)}
            >
              <Icon name="code" size={12} />
            </button>
          )}
          {current !== "paths" && (
            <button
              type="button"
              className="session-detail-payload-action"
              aria-label="Copy to clipboard"
              title="Copy to clipboard"
              onClick={() => void copyText(body)}
            >
              <Icon name="copy" size={12} />
            </button>
          )}
        </span>
      </div>

      {current === "paths" ? (
        <ul className="session-detail-tool-paths">
          {affectedPaths.map((path) => <li key={path}><code>{path}</code></li>)}
        </ul>
      ) : (
        <pre className="session-detail-payload-body"><code>{body}</code></pre>
      )}

      {truncated && current !== "paths" && (
        <p className="session-detail-truncation">Bounded preview · additional content omitted</p>
      )}
    </div>
  );
}

export default function SessionDetailToolCall({
  event,
  expanded,
  detailState,
  onExpandedChange,
}: Props) {
  const tool = event.tool;
  if (!tool) {
    return (
      <div className="session-detail-tool-malformed" role="note">
        <Icon name="alert-triangle" size={12} />
        This tool call could not be projected safely.
      </div>
    );
  }

  const expandedDetail = detailState?.detail;
  const affectedPaths = expandedDetail?.affected_paths.length
    ? expandedDetail.affected_paths
    : tool.affected_paths;
  const argumentsValue = expandedDetail?.arguments ?? tool.argument_preview ?? null;
  const resultValue = expandedDetail?.result ?? tool.result_preview ?? null;

  return (
    <details
      className={`session-detail-tool-call status-${tool.status}`}
      open={expanded}
      onToggle={(toggleEvent) => {
        const open = toggleEvent.currentTarget.open;
        if (open !== expanded) onExpandedChange(event.event_id, open);
      }}
    >
      {/* Name, then the command. The category chip and the per-row timestamp are
          gone: both were constant down the whole list, and the timestamp
          repeated the minute already printed on the turn above. Status is
          rendered only when it is not `completed` — an exception worth seeing,
          not a label worth 24 repetitions. */}
      <summary>
        <span className="session-detail-tool-disclosure" aria-hidden="true">
          <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
        </span>
        <strong>{tool.native_name}</strong>
        <span className="session-detail-tool-summary" title={tool.summary}>
          {toolCommandLabel(tool.summary)}
        </span>
        {tool.status !== "completed" && (
          <span className={`session-detail-tool-status status-${tool.status}`}>{tool.status}</span>
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
            <Icon name="alert-triangle" size={12} />
            {detailState.error}
          </div>
        )}
        {!detailState?.loading && !detailState?.error && (
          <ToolPayload
            argumentsValue={argumentsValue}
            argumentsTruncated={expandedDetail?.arguments_truncated ?? tool.argument_truncated}
            resultValue={resultValue}
            resultTruncated={expandedDetail?.result_truncated ?? tool.result_truncated}
            affectedPaths={affectedPaths}
          />
        )}
      </div>
    </details>
  );
}
