import { useMemo, useState } from "react";
import type { SessionEventDetail, SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import Notice from "../../notice/Notice";
import { writeClipboardText } from "../../sync-review/clipboard";
import { t as translate, useTranslation } from "../../../i18n";
import { readableToolPayload, sessionToolDescriptor, toolCommandLabel } from "./sessionDetailModel";

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

/** What a payload box holds; it names the box and its copy key. */
export type PayloadKind = "arguments" | "result" | "brief" | "report";

/**
 * One labelled, bounded box per payload. Each keeps its own raw toggle and copy
 * key so what you copy is always what the box you are looking at shows.
 * Agent rows reuse it for the brief and the report.
 */
export function PayloadSection({
  kind,
  value,
  truncated,
}: {
  kind: PayloadKind;
  value: string;
  truncated: boolean;
}) {
  const { t } = useTranslation("session");
  const view = useMemo(() => readableToolPayload(value), [value]);
  const [showRaw, setShowRaw] = useState(false);
  const body = showRaw && view.transformed ? view.raw : view.text;

  return (
    <section className="session-detail-payload-section">
      <div className="session-detail-payload-head">
        <span className="session-detail-payload-label">{payloadLabel(kind)}</span>
        <span className="session-detail-payload-tools">
          {/* Deliberately not `.btn`: that class carries the rounded ghost
              treatment, and these sit in a squared-off panel. */}
          {view.transformed && (
            <button
              type="button"
              className={`session-detail-payload-action${showRaw ? " is-on" : ""}`}
              aria-pressed={showRaw}
              title={showRaw ? t("payload.showReadable") : t("payload.showStored")}
              onClick={() => setShowRaw((raw) => !raw)}
            >
              <Icon name="code" size={12} />
            </button>
          )}
          <button
            type="button"
            className="session-detail-payload-action"
            aria-label={payloadCopyLabel(kind)}
            title={t("payload.copy")}
            onClick={() => void writeClipboardText(body)}
          >
            <Icon name="copy" size={12} />
          </button>
        </span>
      </div>
      <pre className="session-detail-payload-body"><code>{body}</code></pre>
      {truncated && (
        <p className="session-detail-truncation">{t("payload.truncated")}</p>
      )}
    </section>
  );
}

/** A payload box's name, and the head of a section that holds a note instead. */
export function payloadLabel(kind: PayloadKind): string {
  switch (kind) {
    case "arguments": return translate("session:payload.arguments");
    case "result": return translate("session:payload.result");
    case "brief": return translate("session:payload.brief");
    case "report": return translate("session:payload.report");
  }
}

function payloadCopyLabel(kind: PayloadKind): string {
  switch (kind) {
    case "arguments": return translate("session:payload.copyArguments");
    case "result": return translate("session:payload.copyResult");
    case "brief": return translate("session:payload.copyBrief");
    case "report": return translate("session:payload.copyReport");
  }
}

/**
 * Stacked, not tabbed: arguments sit beside the result they produced, which is
 * what the pane is opened to compare. Every box is bounded, so the stack still
 * cannot push the rest of the conversation off the screen.
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
  const { t } = useTranslation("session");
  if (!argumentsValue && !resultValue && affectedPaths.length === 0) {
    return <p className="session-detail-tool-empty">{t("tool.empty")}</p>;
  }

  return (
    <div className="session-detail-tool-payload">
      {argumentsValue && (
        <PayloadSection kind="arguments" value={argumentsValue} truncated={argumentsTruncated} />
      )}
      {resultValue && (
        <PayloadSection kind="result" value={resultValue} truncated={resultTruncated} />
      )}
      {affectedPaths.length > 0 && (
        <section className="session-detail-payload-section">
          <div className="session-detail-payload-head">
            <span className="session-detail-payload-label">{t("payload.paths")}</span>
          </div>
          <ul className="session-detail-tool-paths">
            {affectedPaths.map((path) => <li key={path}><code>{path}</code></li>)}
          </ul>
        </section>
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
  const { t } = useTranslation("session");
  const tool = event.tool;
  if (!tool) {
    return (
      <div className="session-detail-tool-malformed" role="note">
        <Icon name="alert-triangle" size={12} />
        {t("tool.malformed")}
      </div>
    );
  }

  const descriptor = sessionToolDescriptor(tool);
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
      {/* The action, then the command — `Bash`/`Read`/`Edit` derived from the
          wrapped function or the recorded name, tinted by the category that
          derivation carries. Status is rendered only when it is not
          `completed` — an exception worth seeing, not a label worth 24
          repetitions. The chevron closes the row at the far edge. */}
      <summary>
        <span className={`session-detail-tool-name cat-${descriptor.category}`}>
          {descriptor.label}
        </span>
        <span className="session-detail-tool-summary" title={tool.summary}>
          {toolCommandLabel(tool.summary)}
        </span>
        {tool.status !== "completed" && (
          <span className={`session-detail-tool-status status-${tool.status}`}>{t(`toolStatus.${tool.status}`)}</span>
        )}
        <span className="session-detail-tool-chevron" aria-hidden="true">
          <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
        </span>
      </summary>

      {/* Only an open row builds its payload. `<details>` hides but still mounts
          a closed body, so mounting every payload can retain megabytes of
          invisible text. Summary and native keyboard behavior stay outside
          this guard. */}
      {expanded && (
      <div className="session-detail-tool-body" aria-busy={detailState?.loading || undefined}>
        {detailState?.loading && (
          <div className="session-detail-local-state" role="status">
            <span className="status-loader" /> {t("tool.loading")}
          </div>
        )}
        {/* The fetch this reports is started by the row being expanded, so the
            failure is an event and not something the pane renders with — and
            it leaves the pane with no payload at all, which is blocking rather
            than advisory. The box prints the severity instead of relying on a
            class and glyph alone. */}
        {detailState?.error && (
          <Notice severity="danger" timing="blocking">{detailState.error}</Notice>
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
      )}
    </details>
  );
}
