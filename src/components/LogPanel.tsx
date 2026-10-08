import { memo, useCallback, useEffect, useMemo, useRef } from "react";
import type { ParseKeys } from "i18next";
import type { ActivityLogLevel, ActivityLogType, LogLine } from "../types";
import { clockOptions } from "../clock";
import { t, uiLocale, useTranslation } from "../i18n";
import { usePreference } from "../preferences";
import { userFacingRepoTerms } from "../terminology";
import Icon, { type IconName } from "./Icons";
import Notice from "./notice/Notice";

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

/* Labels are read when the filter renders, in the language of that moment. */
const LOG_TYPES: Array<{ value: ActivityLogType; label: () => string }> = [
  { value: "push", label: () => t("common:push") },
  { value: "pull", label: () => t("common:pull") },
  { value: "repair", label: () => t("logs:types.repair") },
  { value: "storage", label: () => t("logs:types.storage") },
  { value: "configuration", label: () => t("logs:types.configuration") },
  { value: "history", label: () => t("logs:types.history") },
  { value: "system", label: () => t("logs:types.system") },
];

export const ACTIVITY_LOG_TYPES = LOG_TYPES.map((option) => option.value);

const LOG_LEVELS: Array<{ value: ActivityLogLevel; label: ParseKeys<"logs"> }> = [
  { value: "info", label: "levels.info" },
  { value: "success", label: "levels.success" },
  { value: "warning", label: "levels.warning" },
  { value: "error", label: "levels.error" },
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


function typeSelectionLabel(selected: ActivityLogType[]): string {
  if (selected.length === LOG_TYPES.length) return t("logs:panel.allTypes");
  if (selected.length === 0) return t("logs:panel.noTypes");
  if (selected.length === 1) {
    return LOG_TYPES.find((option) => option.value === selected[0])?.label()
      ?? t("logs:panel.typesSelected", { count: 1 });
  }
  return t("logs:panel.typesSelected", { count: selected.length });
}

/* A log level needs a shape as well as colour (WCAG 1.4.1). Error takes the
   octagon rather than the
   triangle a notice would draw, because a notice separates the two by printing
   "Error ·" beside the glyph and a log row has no room for that word: here the
   silhouettes have to do it alone.

   `info` is absent on purpose. It is the baseline face — `.log-info .log-msg`
   paints the same ink as an unclassed message — so it carries no colour to
   duplicate, and a glyph on every ordinary row would be noise over the whole
   log. Its slot is still rendered, empty, so the columns beside it stay
   straight whichever level a row happens to be. */
const LEVEL_MARKS: Partial<Record<ActivityLogLevel, { icon: IconName; label: ParseKeys<"logs"> }>> = {
  success: { icon: "check-circle", label: "levels.success" },
  warning: { icon: "alert-triangle", label: "levels.warning" },
  error: { icon: "octagon-x", label: "levels.error" },
};

/* Memoized so an incoming line reconciles one new row instead of every
   retained one. Both props are stable: the formatter is memoized on the clock
   preference and the language, and each line object is immutable once
   emitted. The row's own hook re-renders it when the language changes. */
const LogRow = memo(function LogRow({
  line,
  formatter,
}: {
  line: LogLine;
  formatter: Intl.DateTimeFormat;
}) {
  const { t } = useTranslation("logs");
  const mark = LEVEL_MARKS[line.level];
  const type = line.type ?? "system";
  return (
    <div className={`log-line log-${line.level}`}>
      <span className="log-ts">{formatter.format(line.ts)}</span>
      <span className="log-level-mark">
        {mark && (
          <>
            <Icon name={mark.icon} size={12} />
            {/* The glyph is decorative; the hidden word carries the level. */}
            <span className="sync-visually-hidden">{t(mark.label)}</span>
          </>
        )}
      </span>
      {/* The type's word, by its key; a type this build has no word for
          prints as the backend named it. */}
      <span className={`log-type log-type-${type}`}>{t(`tag.${type}`, { defaultValue: type })}</span>
      <span className="log-msg">{userFacingRepoTerms(line.message)}</span>
    </div>
  );
});

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
  const { t, i18n } = useTranslation(["logs", "common"]);
  const bodyRef = useRef<HTMLDivElement>(null);
  const followTailRef = useRef(true);
  const pinnedScrollTopRef = useRef(-1);
  const typeFilterRef = useRef<HTMLDetailsElement>(null);

  // Subscribed, not just read: the drawer can be open while the Settings page
  // flips the clock preference, and the stamps must follow without waiting
  // for the next log event.
  const clock24 = usePreference("clock24");
  const language = i18n.language;
  // One formatter for the whole list, not one per line per render.
  // `toLocaleTimeString` builds and throws away an Intl.DateTimeFormat on every
  // call, and this list re-renders on every incoming log event while open.
  // Rebuilt when the clock preference or the interface language changes.
  const formatter = useMemo(() => {
    void clock24;
    void language;
    return new Intl.DateTimeFormat(uiLocale(), clockOptions(true));
  }, [clock24, language]);

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
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();

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
          <span className="log-panel-title">{t("panel.title")}</span>
          {onTypeFiltersChange && (
            <details className="log-type-filter" ref={typeFilterRef}>
              <summary aria-label={t("panel.typeFilter", { selection: typeSelectionLabel(typeFilters) })}>
                <span className="log-type-filter-key">{t("panel.type")}</span>
                <span className="log-type-filter-value">{typeSelectionLabel(typeFilters)}</span>
                <Icon name="chevron-down" size={12} />
              </summary>
              <div className="log-type-filter-menu" role="group" aria-label={t("panel.types")}>
                <div className="log-type-filter-bulk-actions">
                  <button
                    type="button"
                    onClick={() => onTypeFiltersChange([...ACTIVITY_LOG_TYPES])}
                    disabled={typeFilters.length === ACTIVITY_LOG_TYPES.length}
                  >
                    {t("panel.selectAll")}
                  </button>
                  <button
                    type="button"
                    onClick={() => onTypeFiltersChange([])}
                    disabled={typeFilters.length === 0}
                  >
                    {t("panel.deselectAll")}
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
                    <span>{option.label()}</span>
                  </label>
                ))}
              </div>
            </details>
          )}
          {onLevelFilterChange && (
            <label className="log-filter-control">
              <span>{t("panel.level")}</span>
              <select
                aria-label={t("panel.levelFilter")}
                value={levelFilter}
                onChange={(event) => onLevelFilterChange(event.target.value as ActivityLogLevel | "all")}
              >
                <option value="all">{t("panel.allLevels")}</option>
                {LOG_LEVELS.map((option) => <option key={option.value} value={option.value}>{t(option.label)}</option>)}
              </select>
            </label>
          )}
          {onSearchChange && (
            <label className="log-search-control">
              <input
                type="search"
                value={search}
                placeholder={t("panel.searchPlaceholder")}
                aria-label={t("panel.search")}
                onChange={(event) => onSearchChange(event.target.value)}
                /* The drawer can sit over an open Push/Pull review, whose own
                   Escape listener would discard it. Clearing the field is what
                   Escape means here. */
                onKeyDown={(event) => {
                  if (event.key !== "Escape" || !search) return;
                  event.preventDefault();
                  event.stopPropagation();
                  onSearchChange("");
                }}
              />
            </label>
          )}
        </div>
        <div className="log-panel-actions">
          {onManage && (
            <button className="btn btn-ghost log-manage-btn" onClick={onManage}>
              {t("panel.manage")}
            </button>
          )}
          {!onManage && onClear && (
            <button className="btn btn-ghost log-clear-btn" onClick={onClear} aria-label={t("panel.clearLabel")}>
              {t("panel.clear")}
            </button>
          )}
          <button className="btn btn-ghost" onClick={onClose} title={t("common:close")} aria-label={t("panel.closeLabel")}>
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
            {loadingOlder ? t("common:loading") : t("panel.loadOlder")}
          </button>
        )}
        {error && (
          /* The box can appear after a read fails. It announces instead of
             sitting as a named region. Polite, not assertive: a failed read
             does not block the rest of the application. */
          <Notice severity="danger" timing="progress" className="log-load-error">
            {t("panel.loadFailed", { error })}
          </Notice>
        )}
        {loading && lines.length === 0 ? (
          <div className="log-empty"><span className="status-loader" /> {t("panel.loadingRetained")}</div>
        ) : lines.length === 0 ? (
          <div className="log-empty">{t("panel.noMatches")}</div>
        ) : (
          lines.map((line, i) => (
            <LogRow key={line.id ?? `${line.ts}:${i}`} line={line} formatter={formatter} />
          ))
        )}
      </div>
    </div>
  );
}
