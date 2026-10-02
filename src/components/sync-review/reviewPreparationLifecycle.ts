/**
 * The lifecycle of a Push or Pull review while it is still being prepared.
 *
 * A prepare shows its card only once it has run longer than the 150 ms the
 * loading scaffold waits for, so a warm read or an instant ending never
 * flashes one. Cancel settles the prepare's awaited call at once through a
 * race: the prepare returns, its own `finally` releases its locks, and the
 * backend's late answer goes nowhere. Once a write starts, Cancel is
 * withdrawn. One prepare at a time: beginning another supersedes the last.
 *
 * Framework-free so the timing is testable; the host mirrors the events into
 * its own state.
 */

export const PREPARING_CANCELLED = Symbol("preparing cancelled");

export const PREPARING_GATE_MS = 150;

/** The handle one prepare holds for its own card. */
export interface PreparingHandle {
  /** The call's value, or PREPARING_CANCELLED the moment Cancel is pressed;
   *  a late value is dropped, since the race has already settled. */
  race: <T>(work: Promise<T>) => Promise<T | typeof PREPARING_CANCELLED>;
  /** A write is starting: Cancel is withdrawn for the rest of this prepare. */
  writing: () => void;
  /** Closes the card. Idempotent; the host calls it in the same update that
   *  puts whatever replaces the card on screen. */
  finish: () => void;
}

export interface PreparingEvents {
  /** The prepare outlasted the gate: show its card. */
  shown: () => void;
  /** A write started: withdraw Cancel. */
  writing: () => void;
  /** The prepare ended, by any ending or by Cancel: close its card. */
  finished: () => void;
}

export interface PreparingTimers {
  set: (run: () => void, ms: number) => unknown;
  clear: (timer: unknown) => void;
}

const systemTimers: PreparingTimers = {
  set: (run, ms) => globalThis.setTimeout(run, ms),
  clear: (timer) => globalThis.clearTimeout(timer as ReturnType<typeof setTimeout>),
};

export function createReviewPreparing(events: PreparingEvents, timers: PreparingTimers = systemTimers) {
  let sequence = 0;
  let current: { id: number; cancel: () => void; writing: boolean } | null = null;
  return {
    /** `immediate`: a review already fills the card's slot, so there is no
     *  flash to avoid — waiting out the gate would drop back to the page
     *  under it for 150 ms. The card shows at once instead. */
    begin({ immediate = false }: { immediate?: boolean } = {}): PreparingHandle {
      const id = ++sequence;
      let cancel = () => {};
      const cancelled = new Promise<typeof PREPARING_CANCELLED>((resolve) => {
        cancel = () => resolve(PREPARING_CANCELLED);
      });
      current = { id, cancel, writing: false };
      const gate = immediate
        ? null
        : timers.set(() => {
          if (current?.id === id) events.shown();
        }, PREPARING_GATE_MS);
      if (immediate) events.shown();
      return {
        race: <T>(work: Promise<T>) => Promise.race([work, cancelled]),
        writing: () => {
          if (current?.id !== id) return;
          current.writing = true;
          events.writing();
        },
        finish: () => {
          if (gate !== null) timers.clear(gate);
          if (current?.id !== id) return;
          current = null;
          events.finished();
        },
      };
    },
    /** Stops waiting for the current prepare; false when there is none, or
     *  when it is already writing. */
    cancel(): boolean {
      if (!current || current.writing) return false;
      current.cancel();
      return true;
    },
  };
}
