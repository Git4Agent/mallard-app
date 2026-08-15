import {
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import type {
  ProjectSessionDetail,
  SessionActionDescriptor,
  SessionDetailWarning,
  SessionTimelineEvent,
} from "../../../types";
import Icon from "../../Icons";
import { projectSyncApi } from "../api";
import SessionDetailHeader from "./SessionDetailHeader";
import SessionDetailTimeline from "./SessionDetailTimeline";
import SessionDetailToolbar from "./SessionDetailToolbar";
import type { SessionToolDetailState } from "./SessionDetailToolCall";
import {
  ALL_SESSION_TOOL_CATEGORIES,
  createDefaultSessionDetailFilters,
  eventAnchorId,
  findCheckpointJump,
  includedSessionEventKinds,
  mergeSessionTimelinePages,
  normalizeSessionDetailFailure,
  sessionDetailFilterFingerprint,
  type SessionDetailFilters,
} from "./sessionDetailModel";

const TIMELINE_PAGE_SIZE = 50;

/**
 * What the workspace path bar needs to stand in as this page's header: the
 * leaf crumb renders the title behind the provider's mark (with a dot when the
 * session is live), and the bar's single refresh points here instead of at the
 * activity comparison the page is covering.
 */
export interface SessionHeaderModel {
  title: string;
  provider: string;
  providerDisplayName: string;
  active: boolean;
  refreshing: boolean;
  onRefresh: () => void;
}

interface Props {
  localProjectId: string;
  resourceId: string;
  occurrenceKey: string;
  initialCheckpointId?: string;
  /**
   * Reports the loaded session's identity and refresh handle for the path bar.
   * Called with null while the detail is still loading or has failed, and once
   * more on unmount so the crumb never outlives the page.
   */
  onHeaderChange: (header: SessionHeaderModel | null) => void;
}

function mergeWarnings(...groups: SessionDetailWarning[][]): SessionDetailWarning[] {
  const warnings = new Map<string, SessionDetailWarning>();
  for (const warning of groups.flat()) warnings.set(`${warning.code}:${warning.message}`, warning);
  return [...warnings.values()];
}

function SessionDetailEntryState({
  loading,
  error,
  onRetry,
}: {
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}) {
  return (
    <section className="session-detail-page session-detail-entry-state" aria-labelledby="session-detail-entry-heading">
      <div className={`session-detail-state${error ? " error" : ""}`} role={error ? "alert" : "status"}>
        {loading ? <span className="status-loader" /> : <Icon name="alert-triangle" size={24} />}
        <h1 id="session-detail-entry-heading">{loading ? "Loading session detail…" : "Session detail unavailable"}</h1>
        <span>{error ?? "Preparing the local, bounded timeline."}</span>
        {error && <button type="button" className="btn btn-ghost" onClick={onRetry}>Try again</button>}
      </div>
    </section>
  );
}

export default function SessionDetailPage({
  localProjectId,
  resourceId,
  occurrenceKey,
  initialCheckpointId,
  onHeaderChange,
}: Props) {
  const [detail, setDetail] = useState<ProjectSessionDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(true);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [filters, setFilters] = useState<SessionDetailFilters | null>(null);
  const [events, setEvents] = useState<SessionTimelineEvent[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineLoadingMore, setTimelineLoadingMore] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [snapshotChanged, setSnapshotChanged] = useState(false);
  const [timelineWarnings, setTimelineWarnings] = useState<SessionDetailWarning[]>([]);
  const [timelineReloadEpoch, setTimelineReloadEpoch] = useState(0);
  const [toolDetails, setToolDetails] = useState<Record<string, SessionToolDetailState>>({});
  const [expandedToolIds, setExpandedToolIds] = useState<Set<string>>(new Set());
  const [actionBusyId, setActionBusyId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const detailRequestRef = useRef(0);
  const timelineRequestRef = useRef(0);
  const eventDetailRequestRef = useRef<Record<string, number>>({});
  const sequenceRef = useRef(0);
  const pendingAnchorRef = useRef<string | null>(null);
  const focusAnchorRef = useRef<string | null>(null);
  const initialCheckpointRef = useRef(initialCheckpointId ?? null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const focusedHeadingRef = useRef(false);

  const loadDetail = useCallback(async (preserveFilters = false) => {
    const requestId = ++detailRequestRef.current;
    setDetailLoading(true);
    setDetailError(null);
    setActionError(null);
    try {
      const next = await projectSyncApi.getProjectSessionDetail(localProjectId, resourceId);
      if (requestId !== detailRequestRef.current) return;
      const initialCheckpoint = initialCheckpointRef.current
        ? findCheckpointJump(next.checkpoint_jumps, initialCheckpointRef.current)
        : null;
      initialCheckpointRef.current = null;
      pendingAnchorRef.current = initialCheckpoint?.event_id ?? null;
      focusAnchorRef.current = initialCheckpoint?.event_id ?? null;
      setSnapshotChanged(false);
      setTimelineError(null);
      setTimelineWarnings([]);
      setToolDetails({});
      setExpandedToolIds(new Set());
      setDetail(next);
      setFilters((current) => preserveFilters && current
        ? current
        : createDefaultSessionDetailFilters(next.event_counts));
    } catch (reason) {
      if (requestId !== detailRequestRef.current) return;
      setDetailError(normalizeSessionDetailFailure(reason).message);
      setDetail(null);
      setFilters(null);
    } finally {
      if (requestId === detailRequestRef.current) setDetailLoading(false);
    }
  }, [localProjectId, resourceId]);

  useEffect(() => {
    initialCheckpointRef.current = initialCheckpointId ?? null;
    focusedHeadingRef.current = false;
    setDetail(null);
    setFilters(null);
    setEvents([]);
    setNextCursor(null);
    void loadDetail(false);
    return () => {
      detailRequestRef.current += 1;
      timelineRequestRef.current += 1;
      eventDetailRequestRef.current = {};
    };
  }, [localProjectId, resourceId, occurrenceKey, initialCheckpointId, loadDetail]);

  // The refresh handle stays one stable function so the bar's button never
  // remounts mid-spin; the loader it calls is read through a ref.
  const loadDetailRef = useRef(loadDetail);
  loadDetailRef.current = loadDetail;
  const refreshSession = useCallback(() => {
    void loadDetailRef.current(true);
  }, []);

  useEffect(() => {
    onHeaderChange(detail ? {
      title: detail.title || "Untitled session",
      provider: detail.provider,
      providerDisplayName: detail.provider_display_name,
      active: detail.is_active,
      refreshing: detailLoading,
      onRefresh: refreshSession,
    } : null);
  }, [detail, detailLoading, onHeaderChange, refreshSession]);

  useEffect(() => () => onHeaderChange(null), [onHeaderChange]);

  useEffect(() => {
    if (!detail || focusedHeadingRef.current) return;
    focusedHeadingRef.current = true;
    const frame = window.requestAnimationFrame(() => headingRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [detail]);

  const focusTimelineAnchor = useCallback((eventId: string) => {
    const frame = window.requestAnimationFrame(() => {
      const target = document.getElementById(eventAnchorId(eventId));
      target?.scrollIntoView({ block: "center" });
      target?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(frame);
  }, []);

  const requestTimelinePage = useCallback(async ({
    sessionDetail,
    activeFilters,
    cursor = null,
    anchorEventId = null,
    append = false,
  }: {
    sessionDetail: ProjectSessionDetail;
    activeFilters: SessionDetailFilters;
    cursor?: string | null;
    anchorEventId?: string | null;
    append?: boolean;
  }) => {
    const requestId = ++timelineRequestRef.current;
    if (append) setTimelineLoadingMore(true);
    else setTimelineLoading(true);
    setTimelineError(null);
    try {
      const page = await projectSyncApi.getProjectSessionTimelinePage(
        localProjectId,
        resourceId,
        {
          snapshot_id: sessionDetail.snapshot_id,
          cursor,
          anchor_event_id: anchorEventId,
          limit: TIMELINE_PAGE_SIZE,
          included_event_kinds: includedSessionEventKinds(activeFilters),
          included_tool_categories: [...ALL_SESSION_TOOL_CATEGORIES],
          turn_id: null,
        },
      );
      if (requestId !== timelineRequestRef.current) return;
      setSnapshotChanged(false);
      setEvents((current) => mergeSessionTimelinePages(current, page, !append));
      setNextCursor(page.next_cursor ?? null);
      setTimelineWarnings((current) => append
        ? mergeWarnings(current, page.warnings)
        : page.warnings);
      const anchorToFocus = focusAnchorRef.current;
      if (anchorToFocus) {
        focusAnchorRef.current = null;
        focusTimelineAnchor(anchorToFocus);
      }
    } catch (reason) {
      if (requestId !== timelineRequestRef.current) return;
      const failure = normalizeSessionDetailFailure(reason);
      setSnapshotChanged(failure.code === "snapshot_changed");
      setTimelineError(failure.message);
      if (!append) {
        setEvents([]);
        setNextCursor(null);
      }
    } finally {
      if (requestId === timelineRequestRef.current) {
        setTimelineLoading(false);
        setTimelineLoadingMore(false);
      }
    }
  }, [focusTimelineAnchor, localProjectId, resourceId]);

  const filterFingerprint = filters ? sessionDetailFilterFingerprint(filters) : "";
  useEffect(() => {
    if (!detail || !filters) return;
    const anchorEventId = pendingAnchorRef.current;
    pendingAnchorRef.current = null;
    setExpandedToolIds(new Set());
    void requestTimelinePage({
      sessionDetail: detail,
      activeFilters: filters,
      anchorEventId,
    });
  }, [detail, filterFingerprint, timelineReloadEpoch, requestTimelinePage]);

  const requestToolDetail = useCallback(async (eventId: string) => {
    if (!detail) return;
    const existing = toolDetails[eventId];
    if (existing?.loading || existing?.detail) return;
    const requestId = ++sequenceRef.current;
    eventDetailRequestRef.current[eventId] = requestId;
    setToolDetails((current) => ({
      ...current,
      [eventId]: { loading: true, error: null, detail: current[eventId]?.detail ?? null },
    }));
    try {
      const next = await projectSyncApi.getProjectSessionEventDetail(
        localProjectId,
        resourceId,
        detail.snapshot_id,
        eventId,
      );
      if (eventDetailRequestRef.current[eventId] !== requestId) return;
      setToolDetails((current) => ({
        ...current,
        [eventId]: { loading: false, error: null, detail: next },
      }));
    } catch (reason) {
      if (eventDetailRequestRef.current[eventId] !== requestId) return;
      const failure = normalizeSessionDetailFailure(reason);
      if (failure.code === "snapshot_changed") setSnapshotChanged(true);
      setToolDetails((current) => ({
        ...current,
        [eventId]: {
          loading: false,
          error: failure.message,
          detail: current[eventId]?.detail ?? null,
        },
      }));
    }
  }, [detail, localProjectId, resourceId, toolDetails]);

  const handleToolExpandedChange = (eventId: string, expanded: boolean) => {
    setExpandedToolIds((current) => {
      const next = new Set(current);
      if (expanded) next.add(eventId);
      else next.delete(eventId);
      return next;
    });
    if (expanded) void requestToolDetail(eventId);
  };

  const runAction = async (action: SessionActionDescriptor) => {
    if (!action.enabled || actionBusyId) return;
    setActionBusyId(action.action_id);
    setActionError(null);
    try {
      await projectSyncApi.executeProjectSessionAction(localProjectId, resourceId, action.action_id);
    } catch (reason) {
      setActionError(normalizeSessionDetailFailure(reason).message);
    } finally {
      setActionBusyId(null);
    }
  };

  if (!detail) {
    return (
      <SessionDetailEntryState
        loading={detailLoading}
        error={detailError}
        onRetry={() => void loadDetail(false)}
      />
    );
  }

  const warnings = mergeWarnings(detail.warnings, timelineWarnings);

  return (
    <section
      className="session-detail-page"
      data-resource-id={resourceId}
      data-occurrence-key={occurrenceKey}
      aria-labelledby="session-detail-heading"
    >
      <SessionDetailHeader
        detail={detail}
        headingRef={headingRef}
        actionBusyId={actionBusyId}
        actionError={actionError}
        onAction={(action) => void runAction(action)}
      />

      {snapshotChanged && (
        <div className="sync-callout warning session-detail-stale" role="alert">
          <Icon name="refresh" size={12} />
          <span><strong>This session changed.</strong> Refresh before continuing so pages are not mixed across snapshots.</span>
          <button type="button" className="btn btn-ghost" onClick={() => void loadDetail(true)}>Refresh</button>
        </div>
      )}

      {warnings.length > 0 && (
        <section className="session-detail-warnings" aria-label="Session detail warnings">
          {warnings.map((warning) => (
            <div key={`${warning.code}:${warning.message}`} className="sync-callout warning" role="status">
              <Icon name="alert-triangle" size={12} />
              <span>{warning.message}</span>
            </div>
          ))}
        </section>
      )}

      {filters && (
        <div className="session-detail-layout">
          <SessionDetailToolbar
            counts={detail.event_counts}
            filters={filters}
            loadedCount={events.length}
            onFiltersChange={setFilters}
          />
          <SessionDetailTimeline
            events={events}
            provider={detail.provider}
            providerDisplayName={detail.provider_display_name}
            loading={timelineLoading}
            loadingMore={timelineLoadingMore}
            nextCursor={nextCursor}
            error={timelineError}
            expandedToolIds={expandedToolIds}
            toolDetails={toolDetails}
            onRetry={() => setTimelineReloadEpoch((epoch) => epoch + 1)}
            onLoadMore={() => void requestTimelinePage({
              sessionDetail: detail,
              activeFilters: filters,
              cursor: nextCursor,
              append: true,
            })}
            onToolExpandedChange={handleToolExpandedChange}
          />
        </div>
      )}
    </section>
  );
}
