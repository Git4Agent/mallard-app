import { useCallback, useEffect, useRef } from "react";
import type { ActivityLogLevel, ActivityLogType, LogLine } from "../types";
import { userFacingRepoTerms } from "../terminology";
import Icon from "./Icons";

interface Props {
  lines: LogLine[];
  typeFilters?: ActivityLogType[];
  levelFilter?: ActivityLogLevel | "all";
  search?: string;
  loading?: boolean;
  loadingOlder?: boolean;
  hasOlder?: boolean;
  error?: string | null;
  scrollToBottomEpoch?: number;
  onTypeFiltersChange?: (value: ActivityLogType[]) => void;
  onLevelFilterChange?: (value: ActivityLogLevel | "all") => void;
  onSearchChange?: (value: string) => void;
  onLoadOlder?: () => void;
  onManage?: () => void;
  onClear?: () => void;
  onClose: () => void;
}

const LOG_TYPES: Array<{ value: ActivityLogType; label: string }> = [
  { value: "push", label: "Push" },
  { value: "pull", label: "Pull" },
  { value: "repair", label: "Repair" },
  { value: "storage", label: "Storage" },
  { value: "configuration", label: "Configuration" },
  { value: "history", label: "History" },
  { value: "system", label: "System" },
];

export const ACTIVITY_LOG_TYPES = LOG_TYPES.map((option) => option.value);

const LOG_LEVELS: Array<{ value: ActivityLogLevel; label: string }> = [
  { value: "info", label: "Info" },
  { value: "success", label: "Success" },
  { value: "warning", label: "Warning" },
  { value: "error", label: "Error" },
];

/** A little over one line: a tail that lags by one arrival is still a tail. */
const TAIL_SLACK_PX = 24;

export interface TailMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  /** Where the last pin left the viewport, read back after the browser clamped it. */
  pinnedScrollTop: number;
}

/**
 * Whether the log keeps following its tail once a scroll event lands.
 *
 * Pinning to the tail queues a scroll event of our own making, delivered a frame or
 * more later — by which time newer lines have grown the list past the offset that
 * pin landed on, so the event reports a distance of a line or more rather than zero.
 * Reading that as "the reader scrolled away" is what stranded the panel mid-list
 * while sync bursts streamed in. While the tail is engaged the viewport has only
 * truly moved if it left the offset the last pin put it at.
 */
export function nextFollowTail(following: boolean, metrics: TailMetrics): boolean {
  if (following && Math.abs(metrics.scrollTop - metrics.pinnedScrollTop) < 1) return true;

  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= TAIL_SLACK_PX;
}

function formatTs(ts: number): string {
  return new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function typeSelectionLabel(selected: ActivityLogType[]): string {
  if (selected.length === LOG_TYPES.length) return "All";
  if (selected.length === 0) return "None";
  if (selected.length === 1) {
    return LOG_TYPES.find((option) => option.value === selected[0])?.label ?? "1 selected";
  }
  return `${selected.length} selected`;
}

export default function LogPanel({
  lines,
  typeFilters = ACTIVITY_LOG_TYPES,
  levelFilter = "all",
  search = "",
  loading = false,
  loadingOlder = false,
  hasOlder = false,
  error = null,
  scrollToBottomEpoch = 0,
  onTypeFiltersChange,
  onLevelFilterChange,
  onSearchChange,
  onLoadOlder,
  onManage,
  onClear,
  onClose,
}: Props) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const followTailRef = useRef(true);
  const pinnedScrollTopRef = useRef(-1);
  const typeFilterRef = useRef<HTMLDetailsElement>(null);

  const pinToTail = useCallback(() => {
    const body = bodyRef.current;
    if (!body) return;

    body.scrollTop = body.scrollHeight;
    pinnedScrollTopRef.current = body.scrollTop; // read back: the assignment is clamped
  }, []);

  useEffect(() => {
    const closeOnOutsidePointer = (event: PointerEvent) => {
      const filter = typeFilterRef.current;
      if (filter?.open && event.target instanceof Node && !filter.contains(event.target)) {
        filter.open = false;
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      const filter = typeFilterRef.current;
      if (event.key !== "Escape" || !filter?.open) return;
      event.preventDefault();
      filter.open = false;
      filter.querySelector("summary")?.focus();
    };
    document.addEventListener("pointerdown", closeOnOutsidePointer);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeOnOutsidePointer);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, []);

  useEffect(() => {
    if (followTailRef.current) pinToTail();
  }, [lines, pinToTail]);

  useEffect(() => {
    followTailRef.current = true;
    pinToTail();

    // The drawer animates open, so the first pin lands against a body that is still
    // growing; pin again once it has settled.
    const frame = window.requestAnimationFrame(pinToTail);
    return () => window.cancelAnimationFrame(frame);
  }, [scrollToBottomEpoch, pinToTail]);

  useEffect(() => {
    const body = bodyRef.current;
    if (!body || typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(() => {
      if (followTailRef.current) pinToTail();
    });
    observer.observe(body);
    return () => observer.disconnect();
  }, [pinToTail]);

  const handleScroll = () => {
    const body = bodyRef.current;
    if (!body) return;

    followTailRef.current = nextFollowTail(followTailRef.current, {
      scrollTop: body.scrollTop,
      scrollHeight: body.scrollHeight,
      clientHeight: body.clientHeight,
      pinnedScrollTop: pinnedScrollTopRef.current,
    });
  };

  return (
    <div className="log-panel">
      <div className="log-panel-header">
        <div className="log-panel-heading">
          <span className="log-panel-title">log</span>
          {onTypeFiltersChange && (
            <details className="log-type-filter" ref={typeFilterRef}>
              <summary aria-label={`Filter log by types: ${typeSelectionLabel(typeFilters)}`}>
                <span className="log-type-filter-key">Type</span>
                <span className="log-type-filter-value">{typeSelectionLabel(typeFilters)}</span>
                <Icon name="chevron-down" size={12} />
              </summary>
              <div className="log-type-filter-menu" role="group" aria-label="Log types">
                <div className="log-type-filter-bulk-actions">
                  <button
                    type="button"
                    onClick={() => onTypeFiltersChange([...ACTIVITY_LOG_TYPES])}
                    disabled={typeFilters.length === ACTIVITY_LOG_TYPES.length}
                  >
                    Select all
                  </button>
                  <button
                    type="button"
                    onClick={() => onTypeFiltersChange([])}
                    disabled={typeFilters.length === 0}
                  >
                    Deselect all
                  </button>
                </div>
                {LOG_TYPES.map((option) => (
                  <label className="log-type-filter-option" key={option.value}>
                    <input
                      type="checkbox"
                      checked={typeFilters.includes(option.value)}
                      onChange={() => {
                        const next = new Set(typeFilters);
                        if (next.has(option.value)) next.delete(option.value);
                        else next.add(option.value);
                        onTypeFiltersChange(ACTIVITY_LOG_TYPES.filter((value) => next.has(value)));
                      }}
                    />
                    <span>{option.label}</span>
                  </label>
                ))}
              </div>
            </details>
          )}
          {onLevelFilterChange && (
            <label className="log-filter-control">
              <span>Level</span>
              <select
                aria-label="Filter log by level"
                value={levelFilter}
                onChange={(event) => onLevelFilterChange(event.target.value as ActivityLogLevel | "all")}
              >
                <option value="all">All</option>
                {LOG_LEVELS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
              </select>
            </label>
          )}
          {onSearchChange && (
            <label className="log-search-control">
              <input
                type="search"
                value={search}
                placeholder="Search logs"
                aria-label="Search log"
                onChange={(event) => onSearchChange(event.target.value)}
              />
            </label>
          )}
        </div>
        <div className="log-panel-actions">
          {onManage && (
            <button className="btn btn-ghost log-manage-btn" onClick={onManage}>
              Manage logs
            </button>
          )}
          {!onManage && onClear && (
            <button className="btn btn-ghost log-clear-btn" onClick={onClear} aria-label="Clear log">
              clear
            </button>
          )}
          <button className="btn btn-ghost" onClick={onClose} title="Close" aria-label="Close log">
            <Icon name="x" size={13} />
          </button>
        </div>
      </div>
      <div className="log-panel-body" ref={bodyRef} onScroll={handleScroll}>
        {hasOlder && onLoadOlder && (
          <button
            type="button"
            className="log-load-older"
            onClick={() => {
              followTailRef.current = false;
              onLoadOlder();
            }}
            disabled={loadingOlder}
          >
            {loadingOlder ? "Loading…" : "Load older logs"}
          </button>
        )}
        {error && <div className="log-empty log-load-error">Could not load retained logs: {error}</div>}
        {loading && lines.length === 0 ? (
          <div className="log-empty"><span className="status-loader" /> Loading retained logs…</div>
        ) : lines.length === 0 ? (
          <div className="log-empty">No entries match these filters.</div>
        ) : (
          lines.map((line, i) => (
            <div key={line.id ?? `${line.ts}:${i}`} className={`log-line log-${line.level}`}>
              <span className="log-ts">{formatTs(line.ts)}</span>
              <span className={`log-type log-type-${line.type ?? "system"}`}>{line.type ?? "system"}</span>
              <span className="log-msg">{userFacingRepoTerms(line.message)}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
