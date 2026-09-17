import { useEffect, useState, type ReactNode } from "react";
import type { StorageSetupDraft } from "../../types";
import Icon from "../Icons";

/** What the workspace lends every setup lane's hook: the draft to run on,
 *  and the seat for the draft the lane prepares. */
export interface StorageSetupPort {
  /** Something else holds the page: no run may start. */
  blocked: () => boolean;
  /** Pin the open draft for one run: a save, and whether it still stands. */
  beginRun: () => { isCurrent: () => boolean; persist: () => Promise<StorageSetupDraft | null> };
  onRunStarted: () => void;
  /** Load and seat the prepared draft; false when the page moved on first. */
  adoptPrepared: (saved: StorageSetupDraft, isCurrent: () => boolean) => Promise<boolean>;
}

/** Seconds left on a browser step, from the moment its page opened. */
export function useCountdown(key: string | null, seconds: number): string {
  const [left, setLeft] = useState(seconds);
  useEffect(() => {
    setLeft(seconds);
    if (key === null) return undefined;
    const started = Date.now();
    const id = window.setInterval(() => {
      setLeft(Math.max(0, seconds - Math.floor((Date.now() - started) / 1000)));
    }, 250);
    return () => window.clearInterval(id);
  }, [key, seconds]);
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

export type StepStatus = "todo" | "doing" | "done" | "failed" | "stopped";
export interface StepView { key: string; title: string; status: StepStatus; fact?: ReactNode; controls?: ReactNode; body?: ReactNode }

export function Step({ step }: { step: StepView }) {
  const mark = step.status === "done" ? <Icon name="check-circle" size={12} />
    : step.status === "doing" ? <span className="status-loader" />
    : step.status === "failed" ? <Icon name="alert-triangle" size={12} />
    : step.status === "stopped" ? <Icon name="ban" size={12} />
    : <span className="r2-step-dot" />;
  return (
    <li className="r2-step" data-status={step.status}>
      <span className="r2-step-mark">{mark}</span>
      <div className="r2-step-body">
        <span className="r2-step-title">{step.title}</span>
        {step.fact && <span className="r2-step-fact" role={step.status === "failed" ? "alert" : undefined}>{step.fact}</span>}
        {step.controls && <span className="r2-step-controls">{step.controls}</span>}
        {step.body}
      </div>
    </li>
  );
}
