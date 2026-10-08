import { useCallback, useEffect, useReducer, useRef } from "react";
import { t } from "../../i18n";

/**
 * Receipts for the Settings page. Every row applies the moment it changes, so
 * the page owes proof that the change landed: a stamp beside the control that
 * changed, and one status line in the bar beside Done. Nothing here asks the
 * user to save — there is nothing left to save.
 *
 * The state is a pure reducer so it can be tested without a DOM; the hook owns
 * the timers. A write still running after SLOW_MS shows "Saving…" (a normal
 * write takes tens of milliseconds and goes straight to "Saved"); a receipt
 * stands for RECEIPT_MS, then dims for FADE_MS and goes. The skin tweens
 * nothing, so both steps land at once.
 */

export const SLOW_MS = 400;
export const RECEIPT_MS = 2_400;
export const FADE_MS = 300;

export interface ReceiptsState {
  /** Writes still running after SLOW_MS, by setting key. */
  saving: ReadonlySet<string>;
  /** Standing receipts. `seq` ties the fade and expiry timers to the save that set them. */
  saved: ReadonlyMap<string, { seq: number; fading: boolean }>;
  /** Settings whose latest write failed. */
  failed: ReadonlySet<string>;
  /** When this visit last saved anything; null until the first save. */
  savedAt: number | null;
  /** What the status region says. `seq` keeps a repeated sentence a new announcement. */
  announcement: { text: string; seq: number } | null;
}

export const EMPTY_RECEIPTS: ReceiptsState = {
  saving: new Set(),
  saved: new Map(),
  failed: new Set(),
  savedAt: null,
  announcement: null,
};

export type ReceiptsAction =
  /** A write started. `store` is every key whose failure it clears, the way the section's error clears when the next apply starts. */
  | { type: "begin"; store: readonly string[] }
  | { type: "slow"; key: string }
  | { type: "settled"; key: string; ok: boolean; at: number; seq: number; announce: string | null }
  | { type: "fade"; key: string; seq: number }
  | { type: "expire"; key: string; seq: number };

export function receiptsReducer(state: ReceiptsState, action: ReceiptsAction): ReceiptsState {
  switch (action.type) {
    case "begin": {
      if (!action.store.some((key) => state.failed.has(key))) return state;
      const failed = new Set(state.failed);
      for (const key of action.store) failed.delete(key);
      return { ...state, failed };
    }
    case "slow":
      return state.saving.has(action.key) ? state : { ...state, saving: new Set(state.saving).add(action.key) };
    case "settled": {
      const saving = new Set(state.saving);
      saving.delete(action.key);
      const saved = new Map(state.saved);
      const failed = new Set(state.failed);
      if (action.ok) {
        saved.set(action.key, { seq: action.seq, fading: false });
        failed.delete(action.key);
      } else {
        saved.delete(action.key);
        failed.add(action.key);
      }
      return {
        saving,
        saved,
        failed,
        savedAt: action.ok ? action.at : state.savedAt,
        announcement: action.announce === null ? state.announcement : { text: action.announce, seq: action.seq },
      };
    }
    case "fade": {
      const entry = state.saved.get(action.key);
      if (!entry || entry.seq !== action.seq || entry.fading) return state;
      return { ...state, saved: new Map(state.saved).set(action.key, { seq: entry.seq, fading: true }) };
    }
    case "expire": {
      const entry = state.saved.get(action.key);
      if (!entry || entry.seq !== action.seq) return state;
      const saved = new Map(state.saved);
      saved.delete(action.key);
      return { ...state, saved };
    }
  }
}

export type RowReceipt =
  | { kind: "saving" }
  | { kind: "saved"; fading: boolean }
  | { kind: "failed" };

/** The stamp for one row, or null when the row has nothing to report. */
export function rowReceipt(state: ReceiptsState, key: string): RowReceipt | null {
  if (state.saving.has(key)) return { kind: "saving" };
  if (state.failed.has(key)) return { kind: "failed" };
  const saved = state.saved.get(key);
  return saved ? { kind: "saved", fading: saved.fading } : null;
}

export interface StatusLine {
  tone: "rule" | "saving" | "saved" | "settled" | "failed";
  text: string;
  /** When the visit last saved, for the settled line only. */
  at: number | null;
}

/**
 * The bar's line. Until something changes it states the rule — a result on a
 * page nobody touched would be a small lie. A failure outranks everything and
 * stands until that setting saves; the count while stamps are up is the
 * stamps you can see. Worded when the page renders, in its language.
 */
export function statusLine(state: ReceiptsState): StatusLine {
  const failed = state.failed.size;
  if (failed) return { tone: "failed", text: t("settings:status.notSaved", { count: failed }), at: null };
  if (state.saving.size) return { tone: "saving", text: t("common:saving"), at: null };
  const saved = state.saved.size;
  if (saved) return { tone: "saved", text: t("settings:status.saved", { count: saved }), at: null };
  if (state.savedAt !== null) return { tone: "settled", text: t("settings:status.allSaved"), at: state.savedAt };
  return { tone: "rule", text: t("settings:status.rule"), at: null };
}

export interface SettingsReceipts {
  state: ReceiptsState;
  /** A write started; `store` lists the keys whose failures it clears. */
  begin: (key: string, store: readonly string[]) => void;
  /** A write finished. `announce` is what the status region says; null stays silent. */
  settle: (key: string, ok: boolean, announce: string | null) => void;
}

export function useSettingsReceipts(): SettingsReceipts {
  const [state, dispatch] = useReducer(receiptsReducer, EMPTY_RECEIPTS);
  const timers = useRef(new Set<number>());
  const slowTimers = useRef(new Map<string, number>());
  const seq = useRef(0);

  // The page unmounts on Done and on every navigation; no timer outlives it.
  useEffect(() => {
    const standing = timers.current;
    const slow = slowTimers.current;
    return () => {
      for (const timer of standing) window.clearTimeout(timer);
      for (const timer of slow.values()) window.clearTimeout(timer);
      standing.clear();
      slow.clear();
    };
  }, []);

  const after = useCallback((ms: number, run: () => void) => {
    const timer = window.setTimeout(() => {
      timers.current.delete(timer);
      run();
    }, ms);
    timers.current.add(timer);
  }, []);

  const clearSlow = useCallback((key: string) => {
    const pending = slowTimers.current.get(key);
    if (pending === undefined) return;
    window.clearTimeout(pending);
    slowTimers.current.delete(key);
  }, []);

  const begin = useCallback((key: string, store: readonly string[]) => {
    dispatch({ type: "begin", store });
    clearSlow(key);
    slowTimers.current.set(key, window.setTimeout(() => {
      slowTimers.current.delete(key);
      dispatch({ type: "slow", key });
    }, SLOW_MS));
  }, [clearSlow]);

  const settle = useCallback((key: string, ok: boolean, announce: string | null) => {
    clearSlow(key);
    seq.current += 1;
    const next = seq.current;
    dispatch({ type: "settled", key, ok, at: Date.now(), seq: next, announce });
    if (!ok) return;
    after(RECEIPT_MS, () => dispatch({ type: "fade", key, seq: next }));
    after(RECEIPT_MS + FADE_MS, () => dispatch({ type: "expire", key, seq: next }));
  }, [after, clearSlow]);

  return { state, begin, settle };
}
