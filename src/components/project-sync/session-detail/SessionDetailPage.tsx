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
import { NoticeZone, type NoticeItem } from "../../notice/Notice";
import { projectSyncApi } from "../api";
import LoadingScaffold from "../../LoadingScaffold";
import SessionDetailBack, { type SessionBackTarget } from "./SessionDetailBack";
import SessionDetailHeader from "./SessionDetailHeader";
import SessionDetailTimeline from "./SessionDetailTimeline";
import SessionDetailToolbar from "./SessionDetailToolbar";
import type { SessionToolDetailState } from "./SessionDetailToolCall";
import {
  ALL_SESSION_TOOL_CATEGORIES,
  createDefaultSessionDetailFilters,
  includedSessionEventKinds,
  mergeSessionTimelinePages,
  normalizeSessionDetailFailure,
  type ProjectMarkdownImageState,
  sessionDetailFilterFingerprint,
  sessionDetailFilterToggles,
  type SessionDetailFilters,
  type SessionPushGate,
  sessionPushFacts,
} from "./sessionDetailModel";
import type { PushSessionFacts } from "../ProjectActivityPage";

const TIMELINE_PAGE_SIZE = 50;

/**
 * What the workspace path bar needs to stand in as this page's header: the
 * leaf crumb renders the title behind the provider's mark (with a dot when the
 * session is live), and the bar's single refresh points here instead of at the
 * activity comparison the page is covering.
 */
export type SessionHeaderModel =
  | {
    state: "ready";
    title: string;
    provider: string;
    providerDisplayName: string;
    refreshing: boolean;
    onRefresh: () => void;
    /** What the row of this session's own Push review prints. */
    pushFacts: PushSessionFacts;
    /** Set on an agent's page: the parent session the crumb trail returns to. */
    parent?: { resourceId: string; title: string } | null;
  }
  /* A failed load is reported, not left as null: null means "still opening",
     and a crumb reading "Opening session…" forever beside a body that says
     the detail is unavailable contradicts itself. */
  | { state: "failed" };

interface Props {
  localProjectId: string;
  projectRoot: string;
  resourceId: string;
  occurrenceKey: string;
  /** The page's back button, in every state the page has. */
  back: SessionBackTarget;
  /**
   * Reports the loaded session's identity and refresh handle for the path bar.
   * Called with null while the detail is still loading or has failed, and once
   * more on unmount so the crumb never outlives the page.
   */
  onHeaderChange: (header: SessionHeaderModel | null) => void;
  /** Opens another session page in this workspace — an agent's transcript,
   *  or the parent an agent page came from. */
  onOpenSession?: (resourceId: string) => void;
  /** A hold on this session's Push that the user clears, from the workspace's
   *  comparison. */
  pushGate: SessionPushGate | null;
}

function mergeWarnings(...groups: SessionDetailWarning[][]): SessionDetailWarning[] {
  const warnings = new Map<string, SessionDetailWarning>();
  for (const warning of groups.flat()) warnings.set(`${warning.code}:${warning.message}`, warning);
  return [...warnings.values()];
}

type SessionDetailEntryStateProps =
  | { state: "loading"; back: SessionBackTarget }
  | { state: "failed"; back: SessionBackTarget; error: string; onRetry: () => void };

function SessionDetailEntryState(props: SessionDetailEntryStateProps) {
  if (props.state === "loading") {
    // Match the loaded page geometry so detail lands without a jump. The
    // scaffold is a quiet named region.
    return (
      <section className="session-detail-page session-detail-entry-state" aria-label="Loading session detail">
        <SessionDetailBack target={props.back} />
        <LoadingScaffold shape="detail" label="Loading session detail" />
      </section>
    );
  }
  return (
    <section className="session-detail-page session-detail-entry-state" aria-labelledby="session-detail-entry-heading">
      <SessionDetailBack target={props.back} />
      <div className="session-detail-state error" role="alert">
        <Icon name="alert-triangle" size={24} />
        <h1 id="session-detail-entry-heading">Session detail unavailable</h1>
        <span>{props.error}</span>
        <button type="button" className="btn btn-ghost" onClick={props.onRetry}>Try again</button>
      </div>
    </section>
  );
}

export default function SessionDetailPage({
  localProjectId,
  projectRoot,
  resourceId,
  occurrenceKey,
  back,
  onHeaderChange,
  onOpenSession,
  pushGate,
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
  }, [localProjectId, resourceId, occurrenceKey, loadDetail]);

  // The refresh handle stays one stable function so the bar's button never
  // remounts mid-spin; the loader it calls is read through a ref.
  const loadDetailRef = useRef(loadDetail);
  loadDetailRef.current = loadDetail;
  const refreshSession = useCallback(() => {
    void loadDetailRef.current(true);
  }, []);

  useEffect(() => {
    onHeaderChange(detail ? {
      state: "ready",
      title: detail.title || "Untitled session",
      provider: detail.provider,
      providerDisplayName: detail.provider_display_name,
      refreshing: detailLoading,
      onRefresh: refreshSession,
      pushFacts: sessionPushFacts(detail),
      parent: detail.agent
        ? { resourceId: detail.agent.parent_resource_id, title: detail.agent.parent_title }
        : null,
    } : detailError && !detailLoading ? { state: "failed" } : null);
  }, [detail, detailError, detailLoading, onHeaderChange, refreshSession]);

  useEffect(() => () => onHeaderChange(null), [onHeaderChange]);

  useEffect(() => {
    if (!detail || focusedHeadingRef.current) return;
    focusedHeadingRef.current = true;
    const frame = window.requestAnimationFrame(() => headingRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [detail]);

  const requestTimelinePage = useCallback(async ({
    sessionDetail,
    activeFilters,
    cursor = null,
    append = false,
  }: {
    sessionDetail: ProjectSessionDetail;
    activeFilters: SessionDetailFilters;
    cursor?: string | null;
    append?: boolean;
  }) => {
    const requestId = ++timelineRequestRef.current;
    if (append) setTimelineLoadingMore(true);
    else {
      setTimelineLoading(true);
      // A cursor belongs to its filter set and snapshot. Clear it before a new
      // query so Load more cannot submit a stale cursor.
      setNextCursor(null);
    }
    setTimelineError(null);
    try {
      const page = await projectSyncApi.getProjectSessionTimelinePage(
        localProjectId,
        resourceId,
        {
          snapshot_id: sessionDetail.snapshot_id,
          cursor,
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
  }, [localProjectId, resourceId]);

  const filterFingerprint = filters ? sessionDetailFilterFingerprint(filters) : "";
  useEffect(() => {
    if (!detail || !filters) return;
    setExpandedToolIds(new Set());
    void requestTimelinePage({
      sessionDetail: detail,
      activeFilters: filters,
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

  const handleToolExpandedChange = useCallback((eventId: string, expanded: boolean) => {
    setExpandedToolIds((current) => {
      const next = new Set(current);
      if (expanded) next.add(eventId);
      else next.delete(eventId);
      return next;
    });
    if (expanded) void requestToolDetail(eventId);
  }, [requestToolDetail]);

  const retryTimeline = useCallback(() => {
    setTimelineReloadEpoch((epoch) => epoch + 1);
  }, []);

  /* "Load 50 more" is only rendered once the session, its filters and a cursor
     all exist; the guard keeps that a fact rather than an assumption. */
  const loadMoreEvents = useCallback(() => {
    // The timeline block, and so the button, renders only under `filters`.
    if (!detail || !filters || !nextCursor) return;
    void requestTimelinePage({
      sessionDetail: detail,
      activeFilters: filters,
      cursor: nextCursor,
      append: true,
    });
  }, [detail, filters, nextCursor, requestTimelinePage]);

  const openProjectPath = useCallback(async (relativePath: string) => {
    setActionError(null);
    try {
      await projectSyncApi.openProjectMarkdownPath(localProjectId, relativePath);
    } catch (reason) {
      setActionError(normalizeSessionDetailFailure(reason).message);
    }
  }, [localProjectId]);

  // Markdown-referenced project images, embedded as bounded data: URIs.
  // The cache is settled outcomes only; the ref dedupes in-flight loads so
  // one path is read at most once per page, whatever re-renders happen.
  const [projectImages, setProjectImages] = useState<
    Readonly<Record<string, ProjectMarkdownImageState>>
  >({});
  const requestedProjectImages = useRef(new Set<string>());
  useEffect(() => {
    requestedProjectImages.current = new Set();
    setProjectImages({});
  }, [localProjectId]);
  const loadProjectImage = useCallback(async (relativePath: string) => {
    if (requestedProjectImages.current.has(relativePath)) return;
    requestedProjectImages.current.add(relativePath);
    let state: ProjectMarkdownImageState;
    try {
      const image = await projectSyncApi.readProjectMarkdownImage(localProjectId, relativePath);
      state = {
        status: "loaded",
        src: `data:${image.mime_type};base64,${image.base64_data}`,
      };
    } catch (reason) {
      state = { status: "failed", message: normalizeSessionDetailFailure(reason).message };
    }
    setProjectImages((previous) => ({ ...previous, [relativePath]: state }));
  }, [localProjectId]);

  /* The timeline hands these straight down to every event body, and the
     memoized markdown only skips a re-parse while its props hold still. Fresh
     arrows at the JSX site would change identity on every render and make the
     memo dead weight. */
  const handleOpenProjectPath = useCallback((relativePath: string) => {
    void openProjectPath(relativePath);
  }, [openProjectPath]);

  const handleLoadProjectImage = useCallback((relativePath: string) => {
    void loadProjectImage(relativePath);
  }, [loadProjectImage]);

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
    return detailError ? (
      <SessionDetailEntryState
        state="failed"
        back={back}
        error={detailError}
        onRetry={() => void loadDetail(false)}
      />
    ) : <SessionDetailEntryState state="loading" back={back} />;
  }

  const notes = mergeWarnings(detail.warnings, timelineWarnings);

  /* The box holds only what happened while the page was open: a failed
     action, or a snapshot that moved under a page request. Both arrive after
     the page has painted, so the box announces — the snapshot banner
     especially, since its Refresh button is the only way out of a session
     whose pages would otherwise mix across snapshots. */
  const notices: NoticeItem[] = [];
  if (actionError) {
    notices.push({ key: "action-failed", severity: "danger", message: actionError });
  }
  if (snapshotChanged) {
    notices.push({
      key: "snapshot-changed",
      severity: "warning",
      title: "This session changed.",
      message: "Refresh before continuing so pages are not mixed across snapshots.",
      action: (
        <button type="button" className="btn btn-ghost" onClick={() => void loadDetail(true)}>Refresh</button>
      ),
    });
  }

  return (
    <section
      className="session-detail-page"
      data-resource-id={resourceId}
      data-occurrence-key={occurrenceKey}
      aria-labelledby="session-detail-heading"
    >
      <SessionDetailHeader
        detail={detail}
        back={back}
        headingRef={headingRef}
        actionBusyId={actionBusyId}
        onAction={(action) => void runAction(action)}
        pushGate={pushGate}
        notes={notes}
        onShowToolCalls={filters && !filters.toolCalls
          ? () => setFilters((current) => current && { ...current, toolCalls: true })
          : null}
      />

      <NoticeZone
        items={notices}
        subject="notes about this session"
        className="session-detail-notices"
        timing="blocking"
      />

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
            onRetry={retryTimeline}
            onLoadMore={loadMoreEvents}
            onToolExpandedChange={handleToolExpandedChange}
            onOpenProjectPath={handleOpenProjectPath}
            projectImages={projectImages}
            onLoadProjectImage={handleLoadProjectImage}
            projectRoot={projectRoot}
            repositoryUrl={detail.github_repository_url}
            hasFilterToggles={sessionDetailFilterToggles(detail.event_counts).length > 0}
            onOpenSession={onOpenSession}
          />
        </div>
      )}
    </section>
  );
}
