import type { ThreadSyncDecision, ThreadSyncEntry } from "../../types";
import { backendText } from "../../i18n/backend";

/*
 * A sync row's detail and a decision's reason are the backend's English.
 * A sentence the service words itself carries a code and the value it names
 * (a provider, a read error), and is translated by that code under
 * `syncDetail`; a blocker's own sentence is translated by the blocker. An
 * error passed through has no code and stays as written.
 */

export function syncStatusDetail(
  entry: Pick<ThreadSyncEntry, "status_detail" | "status_detail_code" | "status_detail_value">,
): string | null | undefined {
  const detail = entry.status_detail;
  return detail && entry.status_detail_code
    ? backendText(`syncDetail.${entry.status_detail_code}`, detail, { value: entry.status_detail_value ?? "" })
    : detail;
}

export function syncDecisionReason(
  decision: Pick<ThreadSyncDecision, "blocker" | "blocked_reason" | "reason_code" | "reason_value">,
): string | null | undefined {
  const reason = decision.blocked_reason;
  if (reason == null) return reason;
  if (decision.reason_code) {
    return backendText(`syncDetail.${decision.reason_code}`, reason, { value: decision.reason_value ?? "" });
  }
  return decision.blocker ? backendText(`blocker.${decision.blocker}`, reason) : reason;
}
