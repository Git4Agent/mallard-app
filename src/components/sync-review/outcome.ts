import type { IconName } from "../Icons";
/** A finished operation rendered on the storage card after its review closes. */
export type SyncOutcomeRowStatus = "synced" | "held" | "preserved" | "failed" | "not_pulled" | "not_attempted" | "skipped";

export interface SyncOutcomeRow {
  key: string;
  title: string;
  status: SyncOutcomeRowStatus;
  reason?: string | null;
  threadId?: string | null;
  updatedAt?: number | null;
}

export interface SyncOutcomeStat {
  icon: IconName;
  value?: string;
  label: string;
  /** Renders "Version 6" rather than "6 version". */
  lead?: boolean;
}


export interface OutcomeAction {
  kind: string;
  label: string;
}
export interface OutcomeActionPresentation {
  key: string;
  icon: IconName;
  title?: string;
  available: boolean;
}
export interface SyncOutcomeView<A extends OutcomeAction = OutcomeAction> {
  tone: "success" | "warning";
  title: string;
  message?: string;
  stats: SyncOutcomeStat[];
  rows: SyncOutcomeRow[];
  actions: A[];
  defaultOpen: boolean;
  technical: Array<{ label: string; value: string }>;
}
