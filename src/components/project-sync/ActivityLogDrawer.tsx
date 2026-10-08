import { useCallback, useEffect, useRef, useState } from "react";
import type {
  MouseEvent as ReactMouseEvent,
  Ref,
} from "react";
import type {
  ActivityLogLevel,
  ActivityLogType,
  LogLine,
} from "../../types";
import LogPanel, { ACTIVITY_LOG_TYPES } from "../LogPanel";
import { useTranslation } from "../../i18n";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";

const MAX_LOG_LINES = 2_000;

function logMatchesFilters(
  line: LogLine,
  types: readonly ActivityLogType[],
  level: ActivityLogLevel | "all",
  search = "",
): boolean {
  const needle = search.trim().toLocaleLowerCase();
  return types.includes(line.type ?? "system")
    && (level === "all" || line.level === level)
    && (!needle
      || line.message.toLocaleLowerCase().includes(needle)
      || !!line.event?.toLocaleLowerCase().includes(needle));
}

function mergeLogLines(...groups: LogLine[][]): LogLine[] {
  const entries = new Map<string, LogLine>();
  for (const line of groups.flat()) {
    const key = line.id ?? `${line.ts}:${line.level}:${line.type ?? "system"}:${line.message}`;
    entries.set(key, line);
  }
  return [...entries.values()].sort((left, right) => (
    left.ts - right.ts || (left.id ?? "").localeCompare(right.id ?? "")
  ));
}

interface Props {
  open: boolean;
  height: number;
  refreshEpoch: number;
  scrollToBottomEpoch: number;
  drawerRef: Ref<HTMLDivElement>;
  onStartResize: (event: ReactMouseEvent<HTMLDivElement>) => void;
  onResetHeight: () => void;
  onManage: () => void;
  onClose: () => void;
  readLiveLines: () => readonly LogLine[];
  subscribeToLiveLines: (subscriber: (line: LogLine) => void) => () => void;
}

export default function ActivityLogDrawer({
  open,
  height,
  refreshEpoch,
  scrollToBottomEpoch,
  drawerRef,
  onStartResize,
  onResetHeight,
  onManage,
  onClose,
  readLiveLines,
  subscribeToLiveLines,
}: Props) {
  const { t } = useTranslation("logs");
  const [logLines, setLogLines] = useState<LogLine[]>([]);
  const [logTypeFilters, setLogTypeFilters] = useState<ActivityLogType[]>(() => [...ACTIVITY_LOG_TYPES]);
  const [logLevelFilter, setLogLevelFilter] = useState<ActivityLogLevel | "all">("all");
  const [logSearchInput, setLogSearchInput] = useState("");
  const [logSearch, setLogSearch] = useState("");
  const [logCursor, setLogCursor] = useState<string | null>(null);
  const [logLoading, setLogLoading] = useState(true);
  const [logLoadingOlder, setLogLoadingOlder] = useState(false);
  const [logLoadError, setLogLoadError] = useState<string | null>(null);
  const openRef = useRef(open);
  const logFiltersRef = useRef({ types: logTypeFilters, level: logLevelFilter, search: logSearch });
  const logRequestRef = useRef(0);
  const pendingLinesRef = useRef<LogLine[]>([]);
  const flushFrameRef = useRef<number | null>(null);

  openRef.current = open;
  logFiltersRef.current = { types: logTypeFilters, level: logLevelFilter, search: logSearch };

  useEffect(() => {
    const timer = window.setTimeout(() => setLogSearch(logSearchInput), 180);
    return () => window.clearTimeout(timer);
  }, [logSearchInput]);

  const flushPendingLines = useCallback(() => {
    if (flushFrameRef.current !== null) return;
    flushFrameRef.current = window.requestAnimationFrame(() => {
      flushFrameRef.current = null;
      const filters = logFiltersRef.current;
      const pending = pendingLinesRef.current.filter((line) => (
        logMatchesFilters(line, filters.types, filters.level, filters.search)
      ));
      pendingLinesRef.current = [];
      if (pending.length === 0) return;
      setLogLines((current) => mergeLogLines(current, pending).slice(-MAX_LOG_LINES));
    });
  }, []);

  const loadActivityLogs = useCallback(async (cursor: string | null = null) => {
    const requestId = ++logRequestRef.current;
    const startedAt = Date.now();
    const loadingOlder = cursor !== null;
    if (loadingOlder) {
      setLogLoadingOlder(true);
    } else {
      setLogLoading(true);
      setLogLines(mergeLogLines(readLiveLines().filter((line) => (
        logMatchesFilters(line, logTypeFilters, logLevelFilter, logSearch)
      ))).slice(-MAX_LOG_LINES));
    }
    setLogLoadError(null);
    if (logTypeFilters.length === 0) {
      setLogCursor(null);
      setLogLines([]);
      setLogLoading(false);
      setLogLoadingOlder(false);
      return;
    }
    try {
      const page = await projectSyncApi.queryActivityLogs({
        types: logTypeFilters,
        levels: logLevelFilter === "all" ? [] : [logLevelFilter],
        search: logSearch.trim() || null,
        cursor,
        limit: 500,
      });
      if (requestId !== logRequestRef.current) return;
      setLogCursor(page.next_cursor ?? null);
      setLogLines((current) => {
        const next = loadingOlder
          ? mergeLogLines(page.entries, current)
          : mergeLogLines(
            page.entries,
            readLiveLines().filter((line) => line.ts >= startedAt
              && logMatchesFilters(line, logTypeFilters, logLevelFilter, logSearch)),
          );
        return next.slice(-MAX_LOG_LINES);
      });
    } catch (reason) {
      if (requestId === logRequestRef.current) setLogLoadError(errorMessage(reason));
    } finally {
      if (requestId === logRequestRef.current) {
        setLogLoading(false);
        setLogLoadingOlder(false);
      }
    }
  }, [logLevelFilter, logSearch, logTypeFilters, readLiveLines]);

  useEffect(() => {
    if (open) void loadActivityLogs();
  }, [loadActivityLogs, open, refreshEpoch]);

  useEffect(() => {
    const unsubscribe = subscribeToLiveLines((line) => {
      const filters = logFiltersRef.current;
      if (openRef.current && logMatchesFilters(line, filters.types, filters.level, filters.search)) {
        pendingLinesRef.current.push(line);
        if (pendingLinesRef.current.length > MAX_LOG_LINES) {
          pendingLinesRef.current.splice(0, pendingLinesRef.current.length - MAX_LOG_LINES);
        }
        flushPendingLines();
      }
    });
    return () => {
      if (flushFrameRef.current !== null) {
        window.cancelAnimationFrame(flushFrameRef.current);
        flushFrameRef.current = null;
      }
      unsubscribe();
    };
  }, [flushPendingLines, subscribeToLiveLines]);

  return (
    <div
      ref={drawerRef}
      id="activity-log-drawer"
      className={`log-drawer sync-log-drawer${open ? " open" : ""}`}
      style={{ height: open ? height : undefined }}
      role="region"
      aria-label={t("drawer.label")}
      tabIndex={-1}
    >
      {open && (
        <div
          className="log-resizer"
          role="separator"
          aria-label={t("drawer.resize")}
          aria-orientation="horizontal"
          onMouseDown={onStartResize}
          onDoubleClick={onResetHeight}
        />
      )}
      {open && (
        <LogPanel
          lines={logLines}
          typeFilters={logTypeFilters}
          levelFilter={logLevelFilter}
          search={logSearchInput}
          loading={logLoading}
          loadingOlder={logLoadingOlder}
          hasOlder={!!logCursor}
          error={logLoadError}
          scrollToBottomEpoch={scrollToBottomEpoch}
          onTypeFiltersChange={setLogTypeFilters}
          onLevelFilterChange={setLogLevelFilter}
          onSearchChange={setLogSearchInput}
          onLoadOlder={() => logCursor && void loadActivityLogs(logCursor)}
          onManage={onManage}
          onClose={onClose}
        />
      )}
    </div>
  );
}
