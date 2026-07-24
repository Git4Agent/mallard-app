import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import type {
  CodexThreadDetailsPage,
  CodexThreadSummary,
  GitBranchSummary,
  LocalProjectSummary,
  ProjectBinding,
  ProjectChatHistory,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../types";
import Icon from "../Icons";
import { projectSyncApi } from "./api";
import { compactProjectPath, errorMessage, projectLabel } from "./model";
import ThreadVariantReview from "./ThreadVariantReview";

interface PageProps {
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  refreshEpoch: number;
  embedded?: boolean;
  activeStorageId?: string | null;
  activeStorageName?: string | null;
  selectionMode?: "push" | "pull";
  selectedResourceIds?: ReadonlySet<string>;
  selectableResourceIds?: ReadonlySet<string>;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  comparisonOverride?: ThreadSyncComparison | null;
}

interface ThreadDetailsState {
  loading: boolean;
  error: string | null;
  page: CodexThreadDetailsPage | null;
}

interface ContentProps {
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  history: ProjectChatHistory | null;
  loading: boolean;
  loadingMore: boolean;
  actionError: string | null;
  actionBusyThreadId: string | null;
  detailsByThread?: Record<string, ThreadDetailsState>;
  openDetailOccurrences?: ReadonlySet<string>;
  onBranchChange: (branch: string) => void;
  onRefresh: () => void;
  onLoadMore: () => void;
  onOpenCodex: (threadId: string) => void;
  onOpenTerminal: (threadId: string) => void;
  onToggleDetails?: (threadId: string, occurrenceKey: string) => void;
  onLoadMoreDetails?: (threadId: string) => void;
  embedded?: boolean;
  comparison?: ThreadSyncComparison | null;
  comparisonLoading?: boolean;
  comparisonError?: string | null;
  activeStorageName?: string | null;
  selectionMode?: "push" | "pull";
  selectedResourceIds?: ReadonlySet<string>;
  selectableResourceIds?: ReadonlySet<string>;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
}

const THREAD_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const CHAT_HISTORY_PAGE_SIZE = 10;

interface BranchPickerProps {
  branches: GitBranchSummary[];
  selectedBranch: string;
  disabled: boolean;
  onChange: (branch: string) => void;
}

export function BranchPicker({ branches, selectedBranch, disabled, onChange }: BranchPickerProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = `branch-picker-${useId().replace(/:/g, "")}`;
  const selected = branches.find((branch) => branch.name === selectedBranch) ?? branches[0];

  const focusOption = (target: "selected" | "first" | "last") => {
    window.requestAnimationFrame(() => {
      const options = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitemradio"]',
      ) ?? []);
      const option = target === "first"
        ? options[0]
        : target === "last"
          ? options[options.length - 1]
          : options.find((candidate) => candidate.dataset.branch === selectedBranch) ?? options[0];
      option?.focus();
    });
  };

  const openMenu = (focusTarget: "selected" | "first" | "last" = "selected") => {
    if (disabled) return;
    setOpen(true);
    focusOption(focusTarget);
  };

  const closeMenu = (restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) window.requestAnimationFrame(() => triggerRef.current?.focus());
  };

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !rootRef.current?.contains(event.target)) closeMenu();
    };
    const handleFocusIn = (event: FocusEvent) => {
      if (event.target instanceof Node && !rootRef.current?.contains(event.target)) closeMenu();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      closeMenu(true);
    };
    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("focusin", handleFocusIn);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("focusin", handleFocusIn);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  if (!selected) return null;

  return (
    <div ref={rootRef} className={`v3-history-branch-select${open ? " open" : ""}`}>
      <Icon name="git-branch" size={15} />
      <button
        ref={triggerRef}
        type="button"
        className="v3-history-branch-trigger"
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={menuId}
        aria-label={`Branch: ${selected.name}${selected.is_current ? ", current branch" : ""}${!selected.available ? ", history only" : ""}`}
        title="Choose a branch"
        onClick={() => open ? closeMenu() : openMenu()}
        onKeyDown={(event) => {
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
          event.preventDefault();
          openMenu(event.key === "ArrowDown" ? "first" : "last");
        }}
      >
        <span className="v3-history-branch-name" title={selected.name}>{selected.name}</span>
        {selected.is_current && <span className="v3-history-branch-state current">Current</span>}
        {!selected.available && <span className="v3-history-branch-state history-only">History only</span>}
        <Icon name="chevron-down" size={12} className="v3-history-branch-chevron" />
      </button>
      <div
        ref={menuRef}
        id={menuId}
        className="v3-history-branch-menu"
        role="menu"
        aria-label="Branches"
        hidden={!open}
        onKeyDown={(event) => {
          const options = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(
            '[role="menuitemradio"]',
          ));
          const currentIndex = options.indexOf(document.activeElement as HTMLButtonElement);
          let nextIndex: number | null = null;
          if (event.key === "ArrowDown") nextIndex = currentIndex < 0 ? 0 : (currentIndex + 1) % options.length;
          if (event.key === "ArrowUp") nextIndex = currentIndex < 0
            ? options.length - 1
            : (currentIndex - 1 + options.length) % options.length;
          if (event.key === "Home") nextIndex = 0;
          if (event.key === "End") nextIndex = options.length - 1;
          if (nextIndex === null) return;
          event.preventDefault();
          options[nextIndex]?.focus();
        }}
      >
        <div className="v3-history-branch-menu-heading">
          <span>Switch branch</span>
          <small>{branches.length}</small>
        </div>
        <div className="v3-history-branch-options">
          {branches.map((branch) => (
            <button
              key={branch.name}
              type="button"
              role="menuitemradio"
              aria-checked={branch.name === selectedBranch}
              className="v3-history-branch-option"
              data-branch={branch.name}
              title={!branch.available ? "This branch is no longer available locally; recorded session history can still be shown." : branch.name}
              onClick={() => {
                closeMenu(true);
                if (branch.name !== selectedBranch) onChange(branch.name);
              }}
            >
              <span className="v3-history-branch-check" aria-hidden="true">
                {branch.name === selectedBranch ? "✓" : ""}
              </span>
              <span className="v3-history-branch-option-copy">
                <strong>{branch.name}</strong>
                <span>
                  {branch.is_current ? "Checked out locally" : branch.available ? "Local branch" : "Recorded session history only"}
                </span>
              </span>
              {branch.is_current && <span className="v3-history-branch-state current">Current</span>}
              {!branch.available && <span className="v3-history-branch-state history-only">History only</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function formatDate(value?: number | null): string {
  if (!value) return "Not recorded";
  return new Intl.DateTimeFormat(undefined, {
    month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit",
  }).format(new Date(value * 1_000));
}

function formatCount(value?: number | null): string {
  if (value == null) return "Not reported";
  return new Intl.NumberFormat(undefined, { notation: value >= 1_000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(value);
}

async function openExternalUrl(url: string) {
  try {
    await openUrl(url);
  } catch {
    if (typeof window !== "undefined") window.open(url, "_blank", "noopener,noreferrer");
  }
}

function githubCommitUrl(repositoryUrl: string | null | undefined, sha: string): string | null {
  if (!repositoryUrl) return null;
  return `${repositoryUrl.replace(/\/+$/, "")}/commit/${sha}`;
}

type ThreadSyncPresentation = {
  icon: "badge-check" | "cloud-off" | "download" | "git-compare-arrows" | "octagon-x" | "upload";
  className: string;
  label: string;
};

export function threadSyncPresentation(entry: ThreadSyncEntry, storageName: string): ThreadSyncPresentation {
  switch (entry.state) {
    case "synced":
      return { icon: "badge-check", className: "synced", label: `Up to date here and in ${storageName}.` };
    case "local_only":
      return { icon: "upload", className: "local", label: `Only on this computer. Push to save it in ${storageName}.` };
    case "local_ahead":
      return { icon: "upload", className: "local", label: `Newer on this computer. Push to update ${storageName}.` };
    case "storage_only":
      return { icon: "download", className: "storage", label: `Only in ${storageName}. Pull to download it here.` };
    case "storage_ahead":
      return { icon: "download", className: "storage", label: `Newer in ${storageName}. Pull to update this computer.` };
    case "diverged":
      return { icon: "git-compare-arrows", className: "diverged", label: `Changed here and in ${storageName}. Review before syncing.` };
    case "unavailable":
      return {
        icon: "octagon-x",
        className: "unavailable",
        label: `Unavailable for sync. ${entry.status_detail ?? "Mallard could not safely read this session."}`,
      };
    default:
      return { icon: "cloud-off", className: "needs-baseline", label: `Comparison baseline needed in ${storageName}. Pull or Push to establish one.` };
  }
}

function ThreadSyncIndicator({ entry, storageName }: { entry: ThreadSyncEntry; storageName: string }) {
  const presentation = threadSyncPresentation(entry, storageName);
  return (
    <span
      className={`v3-thread-sync-indicator ${presentation.className}`}
      data-tooltip={presentation.label}
      aria-label={presentation.label}
      tabIndex={0}
    >
      <Icon name={presentation.icon} size={13} />
    </span>
  );
}

export function ThreadMetrics({ thread, id }: { thread: CodexThreadSummary; id?: string }) {
  const startedLabel = `Started ${formatDate(thread.started_at)}`;
  const endedLabel = `${thread.is_active ? "Last activity" : "Ended"} ${formatDate(thread.ended_at)}${thread.is_active ? ", active" : ""}`;

  return <div id={id} className="v3-history-thread-meta" aria-label="Session details">
    <span className="v3-history-thread-metric" data-tooltip={startedLabel} aria-label={startedLabel} tabIndex={0}><Icon name="play" size={12} /><time>{formatDate(thread.started_at)}</time></span>
    <span className="v3-history-thread-metric" data-tooltip={endedLabel} aria-label={endedLabel} tabIndex={0}><Icon name={thread.is_active ? "activity" : "check-circle"} size={12} /><time>{formatDate(thread.ended_at)}</time></span>
    <span className="v3-history-thread-metric" data-tooltip={`User turns · ${thread.user_round_count}`} aria-label={`User turns: ${thread.user_round_count}`} tabIndex={0}><Icon name="user" size={12} /><b>{thread.user_round_count}</b></span>
    <span className="v3-history-thread-metric" data-tooltip={`Total tokens · ${formatCount(thread.total_tokens)}`} aria-label={`Total tokens: ${formatCount(thread.total_tokens)}`} tabIndex={0}><Icon name="token" size={12} /><b>{formatCount(thread.total_tokens)}</b></span>
    <span className="v3-history-thread-metric" data-tooltip={`Agent messages · ${thread.agent_message_count}`} aria-label={`Agent messages: ${thread.agent_message_count}`} tabIndex={0}><Icon name="message" size={12} /><b>{thread.agent_message_count}</b></span>
    <span className="v3-history-thread-metric" data-tooltip={`Tool calls · ${thread.tool_call_count}`} aria-label={`Tool calls: ${thread.tool_call_count}`} tabIndex={0}><Icon name="tool" size={12} /><b>{thread.tool_call_count}</b></span>
    {thread.commit_occurrence_count > 1 && (
      <span className="v3-history-thread-metric" data-tooltip={`Commit appearances · ${thread.commit_occurrence_count}`} aria-label={`Appears under ${thread.commit_occurrence_count} commits`} tabIndex={0}><Icon name="git-branch" size={12} /><b>{thread.commit_occurrence_count}</b></span>
    )}
    {!thread.metrics_complete && <span className="v3-history-thread-metric v3-history-partial" data-tooltip="Some session metrics are unavailable" aria-label="Some session metrics are unavailable" tabIndex={0}><Icon name="alert-triangle" size={12} /></span>}
  </div>;
}

interface ThreadCardProps {
  thread: CodexThreadSummary;
  occurrenceKey: string;
  busy?: boolean;
  details?: ThreadDetailsState;
  detailsOpen?: boolean;
  onOpenCodex: (threadId: string) => void;
  onOpenTerminal: (threadId: string) => void;
  onToggleDetails?: (threadId: string, occurrenceKey: string) => void;
  onLoadMoreDetails?: (threadId: string) => void;
  syncEntry?: ThreadSyncEntry;
  storageName?: string | null;
  resourceId: string;
  selectionMode?: "push" | "pull";
  selected?: boolean;
  selectable?: boolean;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
}

function syncReviewStateLabel(
  mode: "push" | "pull",
  entry: ThreadSyncEntry | undefined,
  selected: boolean,
): string | null {
  if (!entry) return null;
  if (entry.state === "unavailable") return "Unavailable for sync";
  if (mode === "push") {
    if (entry.state === "storage_only" || entry.state === "storage_ahead" || entry.state === "diverged") {
      return "Pull required";
    }
    return entry.state === "unknown" ? "Review status" : null;
  }
  if (entry.state === "diverged") return selected ? "Use storage version" : "Keep local";
  if (entry.state === "storage_only" || entry.state === "storage_ahead") {
    return selected ? "Restore from storage" : "Keep local";
  }
  if (entry.state === "local_only" || entry.state === "local_ahead") return "Keep local";
  return entry.state === "synced" ? null : "Review status";
}

function ThreadCard({
  thread, occurrenceKey, busy = false, details, detailsOpen = false, onOpenCodex, onOpenTerminal, onToggleDetails, onLoadMoreDetails,
  syncEntry, storageName, resourceId, selectionMode, selected = false, selectable = false,
  selectionDisabled = false, onToggleResource,
}: ThreadCardProps) {
  const launchable = THREAD_UUID.test(thread.thread_id);
  const chatDetailsId = `thread-chat-${occurrenceKey.replace(/[^a-z0-9_-]/gi, "-")}`;
  const chatDetailsLabel = detailsOpen
    ? "Hide chat history"
    : details?.page
      ? "Show chat history"
      : "Load chat history";
  const reviewStateLabel = selectionMode ? syncReviewStateLabel(selectionMode, syncEntry, selected) : null;
  return (
    <article className={`v3-history-thread-card${selectionMode ? " v3-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="v3-history-thread-topline">
        {selectionMode && (
          <label
            className="v3-sync-row-choice"
            title={selectable ? `${selected ? "Exclude" : "Include"} ${thread.title || "this session"}` : reviewStateLabel ?? "No sync action available"}
          >
            <input
              type="checkbox"
              checked={selected}
              disabled={selectionDisabled || !selectable}
              aria-label={`${selected ? "Exclude" : "Include"} ${thread.title || "Untitled Codex session"}`}
              onChange={() => onToggleResource?.(resourceId)}
            />
          </label>
        )}
        <div className="v3-history-thread-title">
          {syncEntry && storageName && <ThreadSyncIndicator entry={syncEntry} storageName={storageName} />}
          <strong>{thread.title || "Untitled Codex session"}</strong>
          {reviewStateLabel && (
            <span className={`v3-sync-review-state state-${syncEntry?.state ?? "local_only"}`}>{reviewStateLabel}</span>
          )}
        </div>
        <div className="v3-history-thread-actions">
          <button type="button" className="btn btn-ghost v3-history-launch-action" disabled={!launchable || busy}
            onClick={() => onOpenCodex(thread.thread_id)} title={busy ? "Opening in Codex…" : "Open in Codex"}
            aria-label={busy ? "Opening in Codex" : "Open in Codex"}>
            <Icon name={busy ? "refresh" : "openai"} size={15} className={busy ? "icon-spin" : "v3-openai-icon"} />
          </button>
          <button type="button" className="btn btn-ghost v3-history-launch-action" disabled={!launchable || busy}
            onClick={() => onOpenTerminal(thread.thread_id)} title="Open in Terminal" aria-label="Open in Terminal">
            <Icon name="terminal" size={14} />
          </button>
        </div>
      </div>

      <div className="v3-history-session-details">
        <div className="v3-history-session-summary">
          <ThreadMetrics thread={thread} />
          {onToggleDetails && (
            <button type="button" className={`btn btn-ghost v3-history-chat-toggle${detailsOpen ? " active" : ""}`}
              aria-label={chatDetailsLabel} title={chatDetailsLabel} aria-expanded={detailsOpen} aria-controls={chatDetailsId}
              onClick={() => onToggleDetails(thread.thread_id, occurrenceKey)}>
              <Icon name="message" size={13} />
            </button>
          )}
        </div>
        {onToggleDetails && detailsOpen && (
          <div id={chatDetailsId} className="v3-history-chat-details" aria-live="polite">
            {details?.loading && !details.page ? <div className="v3-history-detail-state"><span className="status-loader" /> Loading chat history…</div> : null}
            {details?.error && <div className="v3-history-detail-state error" role="alert">{details.error}</div>}
            {details?.page?.next_cursor != null && (
              <button type="button" className="btn btn-ghost v3-history-load-older" disabled={details.loading}
                onClick={() => onLoadMoreDetails?.(thread.thread_id)}>
                {details.loading ? "Loading older messages…" : `Load ${CHAT_HISTORY_PAGE_SIZE} older messages`}
              </button>
            )}
            {details?.page?.turns.map((turn) => (
              <div key={`${thread.thread_id}:${turn.ordinal}`} className="v3-history-turn">
                <span>{turn.role === "user" ? "User" : "Codex"}</span>
                <time>{formatDate(turn.timestamp)}</time>
                <p title={turn.preview}>{turn.preview}</p>
              </div>
            ))}
          </div>
        )}
      </div>
    </article>
  );
}

function StoredThreadCard({
  entry,
  storageName,
  selectionMode,
  selected = false,
  selectable = false,
  selectionDisabled = false,
  onToggleResource,
}: {
  entry: ThreadSyncEntry;
  storageName: string;
  selectionMode?: "push" | "pull";
  selected?: boolean;
  selectable?: boolean;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
}) {
  const shortId = entry.thread_id.length > 12 ? entry.thread_id.slice(0, 8) : entry.thread_id;
  const title = entry.display_name && entry.display_name !== entry.thread_id
    ? entry.display_name
    : `Stored thread ${shortId}`;
  const reviewStateLabel = selectionMode ? syncReviewStateLabel(selectionMode, entry, selected) : null;
  return (
    <article className={`v3-history-thread-card v3-history-stored-thread${selectionMode ? " v3-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="v3-history-thread-topline">
        {selectionMode && (
          <label className="v3-sync-row-choice" title={selectable ? `${selected ? "Exclude" : "Include"} ${title}` : reviewStateLabel ?? "No sync action available"}>
            <input
              type="checkbox"
              checked={selected}
              disabled={selectionDisabled || !selectable}
              aria-label={`${selected ? "Exclude" : "Include"} ${title}`}
              onChange={() => onToggleResource?.(entry.resource_id)}
            />
          </label>
        )}
        <div className="v3-history-thread-title" title={entry.thread_id}>
          <ThreadSyncIndicator entry={entry} storageName={storageName} />
          <strong>{title}</strong>
          {reviewStateLabel && <span className={`v3-sync-review-state state-${entry.state}`}>{reviewStateLabel}</span>}
        </div>
      </div>
    </article>
  );
}

export function ProjectChatHistoryContent({
  project, binding, history, loading, loadingMore, actionError, actionBusyThreadId,
  detailsByThread = {}, openDetailOccurrences = new Set(), onBranchChange, onRefresh, onLoadMore, onOpenCodex,
  onOpenTerminal, onToggleDetails, onLoadMoreDetails, embedded = false, comparison = null,
  comparisonLoading = false, comparisonError = null, activeStorageName = null,
  selectionMode, selectedResourceIds = new Set(), selectableResourceIds,
  selectionDisabled = false, onToggleResource,
}: ContentProps) {
  const label = projectLabel(project);
  const aliased = label !== project.display_name;
  const threads = useMemo(() => new Map((history?.threads ?? []).map((thread) => [thread.thread_id, thread])), [history?.threads]);
  const syncByThread = useMemo(() => new Map((comparison?.entries ?? []).map((entry) => [entry.thread_id, entry])), [comparison?.entries]);
  const comparisonStorageName = comparison?.storage_name ?? activeStorageName ?? "selected storage";
  const threadEntries = useMemo(
    () => (comparison?.entries ?? []).filter((entry) => entry.cloud_thread_id),
    [comparison?.entries],
  );
  const compactSelectionReview = embedded && selectionMode != null;
  const storedOnlyEntries = useMemo(() => (comparison?.entries ?? [])
    .filter((entry) => !entry.cloud_thread_id)
    .filter((entry) => !entry.local_present && entry.storage_present && !threads.has(entry.thread_id))
    .filter((entry) => !history || entry.storage_updated_at == null || entry.storage_updated_at >= history.window_start),
  [comparison?.entries, history, threads]);
  const hasCodexProfile = !!binding?.profile_ids?.codex;
  const embeddedTitle = history?.git ? "Git history" : history ? "Codex threads" : "Activity";
  const embeddedThreadCount = history
    ? new Set([...history.threads.map((thread) => thread.thread_id), ...storedOnlyEntries.map((entry) => entry.thread_id)]).size
    : null;
  const visibleComparisonCounts = useMemo(() => {
    const visibleThreadIds = new Set(threads.keys());
    storedOnlyEntries.forEach((entry) => visibleThreadIds.add(entry.thread_id));
    const counts = { local: 0, storage: 0, diverged: 0, unavailable: 0, needsBaseline: 0 };

    for (const entry of comparison?.entries ?? []) {
      if (!visibleThreadIds.has(entry.thread_id)) continue;
      if (entry.state === "local_only" || entry.state === "local_ahead") counts.local += 1;
      else if (entry.state === "storage_only" || entry.state === "storage_ahead") counts.storage += 1;
      else if (entry.state === "diverged") counts.diverged += 1;
      else if (entry.state === "unavailable") counts.unavailable += 1;
      else if (entry.state === "unknown") counts.needsBaseline += 1;
    }

    return counts;
  }, [comparison?.entries, storedOnlyEntries, threads]);
  const visibleComparisonChangeCount = visibleComparisonCounts.local
    + visibleComparisonCounts.storage
    + visibleComparisonCounts.diverged
    + visibleComparisonCounts.unavailable
    + visibleComparisonCounts.needsBaseline;
  const showEmbeddedThreadCount = embedded && !compactSelectionReview && !history?.git && embeddedThreadCount !== null;
  const showComparisonSummary = comparison != null && visibleComparisonChangeCount > 0;
  const renderThread = (thread: CodexThreadSummary, key: string) => {
    const entry = syncByThread.get(thread.thread_id);
    const resourceId = entry?.resource_id ?? `codex:session:${thread.thread_id}`;
    return (
    <ThreadCard key={key} thread={thread} occurrenceKey={key} busy={actionBusyThreadId === thread.thread_id}
      details={detailsByThread[thread.thread_id]} detailsOpen={openDetailOccurrences.has(key)} onOpenCodex={onOpenCodex} onOpenTerminal={onOpenTerminal}
      onToggleDetails={onToggleDetails} onLoadMoreDetails={onLoadMoreDetails}
      syncEntry={entry} storageName={comparison ? comparisonStorageName : null}
      resourceId={resourceId} selectionMode={selectionMode} selected={selectedResourceIds.has(resourceId)}
      selectable={selectableResourceIds?.has(resourceId) ?? !!selectionMode}
      selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} />
    );
  };
  const orderedReferences = (references: { thread_id: string }[]) => [...references].sort((left, right) => {
    const leftThread = threads.get(left.thread_id);
    const rightThread = threads.get(right.thread_id);
    if (!leftThread || !rightThread) return left.thread_id.localeCompare(right.thread_id);
    return rightThread.ended_at - leftThread.ended_at
      || leftThread.started_at - rightThread.started_at
      || left.thread_id.localeCompare(right.thread_id);
  });
  const flatThreadRows: Array<
    | { kind: "local"; id: string; updatedAt: number; thread: CodexThreadSummary }
    | { kind: "stored"; id: string; updatedAt: number; entry: ThreadSyncEntry }
  > = [
    ...(history?.threads ?? []).map((thread) => ({
      kind: "local" as const,
      id: thread.thread_id,
      updatedAt: thread.ended_at,
      thread,
    })),
    ...storedOnlyEntries.map((entry) => ({
      kind: "stored" as const,
      id: entry.thread_id,
      updatedAt: entry.storage_updated_at ?? entry.local_updated_at ?? 0,
      entry,
    })),
  ].sort((left, right) => right.updatedAt - left.updatedAt || left.id.localeCompare(right.id));

  const content = (
      <section
        className={embedded ? "v3-history-content" : "profile-links-section"}
        aria-labelledby={compactSelectionReview ? undefined : "project-activity-heading"}
        aria-label={compactSelectionReview ? embeddedTitle : undefined}
      >
        {!compactSelectionReview && <header className="profile-links-heading v3-history-header">
          <div className="profile-links-copy">
            {embedded ? (
              <h2 id="project-activity-heading" className="v3-history-embedded-title">
                <Icon
                  name={history?.git ? "git-branch" : history ? "openai" : "activity"}
                  size={15}
                  className={!history?.git && history ? "v3-openai-icon" : undefined}
                />
                {embeddedTitle}
              </h2>
            ) : (
              <h1 id="project-activity-heading" className="settings-section-title">{label}</h1>
            )}
          </div>
          <div className="v3-history-toolbar">
            {activeStorageName && !compactSelectionReview && (
              <span
                className="v3-history-storage-lens"
                title={`Comparing threads with ${activeStorageName}`}
                aria-label={`Comparing threads with ${activeStorageName}`}
              >
                <Icon name="link" size={12} />
                <span>{activeStorageName}</span>
              </span>
            )}
            {history?.git && (
              <BranchPicker
                branches={history.git.branches}
                selectedBranch={history.git.selected_branch}
                disabled={loading}
                onChange={onBranchChange}
              />
            )}
            {(showEmbeddedThreadCount || showComparisonSummary) && (
              <span className="v3-history-toolbar-stats">
                {showEmbeddedThreadCount && (
                  <span
                    className="v3-history-heading-count v3-history-toolbar-count"
                    title={`${embeddedThreadCount} thread${embeddedThreadCount === 1 ? "" : "s"} shown`}
                    aria-label={`${embeddedThreadCount} thread${embeddedThreadCount === 1 ? "" : "s"}`}
                  >
                    <Icon name="message" size={12} />{embeddedThreadCount}
                  </span>
                )}
                {showComparisonSummary && (
                  <span className="v3-thread-sync-summary" aria-label={`Visible thread comparison with ${comparisonStorageName}`}>
                    {visibleComparisonCounts.local > 0 && <span className="local" title={`${visibleComparisonCounts.local} local thread change${visibleComparisonCounts.local === 1 ? "" : "s"}`}><Icon name="upload" size={12} />{visibleComparisonCounts.local}</span>}
                    {visibleComparisonCounts.storage > 0 && <span className="storage" title={`${visibleComparisonCounts.storage} thread change${visibleComparisonCounts.storage === 1 ? "" : "s"} in ${comparisonStorageName}`}><Icon name="download" size={12} />{visibleComparisonCounts.storage}</span>}
                    {visibleComparisonCounts.diverged > 0 && <span className="diverged" title={`${visibleComparisonCounts.diverged} diverged thread${visibleComparisonCounts.diverged === 1 ? "" : "s"}`}><Icon name="git-compare-arrows" size={12} />{visibleComparisonCounts.diverged}</span>}
                    {visibleComparisonCounts.unavailable > 0 && <span className="unavailable" title={`${visibleComparisonCounts.unavailable} thread${visibleComparisonCounts.unavailable === 1 ? " is" : "s are"} unavailable for sync`}><Icon name="octagon-x" size={12} />{visibleComparisonCounts.unavailable}</span>}
                    {visibleComparisonCounts.needsBaseline > 0 && <span className="needs-baseline" title={`${visibleComparisonCounts.needsBaseline} thread${visibleComparisonCounts.needsBaseline === 1 ? " needs" : "s need"} a comparison baseline`}><Icon name="cloud-off" size={12} />{visibleComparisonCounts.needsBaseline}</span>}
                  </span>
                )}
              </span>
            )}
            {comparisonLoading && <span className="v3-thread-sync-loading" role="status" aria-label={`Comparing threads with ${activeStorageName ?? "storage"}`}><Icon name="refresh" size={13} className="icon-spin" /></span>}
            {comparisonError && <span className="v3-thread-sync-error" tabIndex={0} data-tooltip={comparisonError} aria-label={`Storage comparison failed: ${comparisonError}`}><Icon name="octagon-x" size={13} /></span>}
            <button type="button" className="v3-history-icon-action v3-history-refresh" onClick={onRefresh} disabled={loading || comparisonLoading}
              title="Refresh activity" aria-label="Refresh activity"><Icon name="refresh" size={15} className={loading || comparisonLoading ? "icon-spin" : undefined} /></button>
          </div>
        </header>}

        {actionError && <div className="v3-callout error v3-history-error" role="alert"><Icon name="alert-triangle" size={15} /><span>{actionError}</span></div>}
        {!hasCodexProfile ? (
          <div className="v3-history-state v3-history-profile-state"><Icon name="alert-triangle" size={18} /><div><strong>No Codex profile is connected.</strong><span>Remove and add the project again to choose a Codex profile.</span></div></div>
        ) : loading && !history ? <div className="v3-history-state" role="status" aria-live="polite"><span className="status-loader" /> Loading project activity…</div> : !history ? null : (
          <>
            {!embedded && (
              <section className="v3-history-project-context" aria-label="Project information">
                <span className="v3-history-context-item" title={`Project directory: ${binding?.canonical_project_root ?? binding?.project_root ?? project.project_root ?? "Not configured"}`}>
                  <Icon name="folder" size={14} /><span>{compactProjectPath(binding?.canonical_project_root ?? binding?.project_root ?? project.project_root ?? "Not configured")}</span>
                </span>
                <span className="v3-history-context-item" title={`Codex configuration: ${history.codex_home}`}>
                  <Icon name="terminal" size={14} /><span>{compactProjectPath(history.codex_home)}</span>
                </span>
                {aliased && <span className="v3-history-context-item" title={`Repository: ${project.display_name}`}><Icon name="git-branch" size={14} /><span>{project.display_name}</span></span>}
                {history.storage_sync.length ? history.storage_sync.map((storage) => (
                  <span key={storage.storage_id} className="v3-history-context-item v3-history-storage-item" title={`Storage: ${storage.storage_name}`}>
                    <Icon name="cloud" size={14} /><strong>{storage.storage_name}</strong>
                    <span className={storage.last_pull_at ? "recorded" : undefined} title={`Last pull: ${formatDate(storage.last_pull_at)}`} aria-label={`Last pull: ${formatDate(storage.last_pull_at)}`}><Icon name="download" size={13} /></span>
                    <span className={storage.last_push_at ? "recorded" : undefined} title={`Last push: ${formatDate(storage.last_push_at)}`} aria-label={`Last push: ${formatDate(storage.last_push_at)}`}><Icon name="upload" size={13} /></span>
                  </span>
                )) : <span className="v3-history-context-item muted" title="No storage linked"><Icon name="cloud" size={14} /><span>No storage</span></span>}
              </section>
            )}

            {threadEntries.length > 0 && (
              <ThreadVariantReview
                entries={threadEntries}
                selectedResourceIds={selectedResourceIds}
                disabled={selectionDisabled}
                onToggleResource={onToggleResource}
                onOpenProviderThread={onOpenCodex}
              />
            )}

            {history.git ? (
              <>
                {storedOnlyEntries.length > 0 && (
                  <section className="v3-history-storage-only" aria-labelledby="storage-only-heading">
                    <div className="v3-history-section-heading">
                      <h2 id="storage-only-heading">Only in {comparisonStorageName}</h2>
                      <span className="v3-history-heading-count" title="Threads not available on this machine"><Icon name="download" size={12} />{storedOnlyEntries.length}</span>
                    </div>
                    <div className="v3-history-thread-list flat">
                      {storedOnlyEntries.map((entry) => <StoredThreadCard key={`stored:${entry.resource_id}`} entry={entry} storageName={comparisonStorageName}
                        selectionMode={selectionMode} selected={selectedResourceIds.has(entry.resource_id)}
                        selectable={selectableResourceIds?.has(entry.resource_id) ?? !!selectionMode}
                        selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} />)}
                    </div>
                  </section>
                )}
                {history.unmapped.length > 0 && (
                  <section className="v3-history-uncommitted" aria-labelledby="uncommitted-heading">
                    <div className="v3-history-section-heading"><h2 id="uncommitted-heading">Uncommitted</h2></div>
                    <div className="v3-history-thread-list flat">{history.unmapped.map((reference) => { const thread = threads.get(reference.thread_id); return thread ? renderThread(thread, `uncommitted:${thread.thread_id}`) : null; })}</div>
                  </section>
                )}
                <section className="v3-history-commit-section" aria-label={`First-parent commits on ${history.git.selected_branch}`}>
                  {!embedded && <div className="v3-history-section-heading"><h2>Commit history</h2><span className="v3-history-heading-count" title={`${history.git.reference_count} commit occurrences`} aria-label={`${history.git.reference_count} commit occurrences`}><Icon name="git-branch" size={12} />{history.git.reference_count}</span></div>}
                  {history.git.commits.length === 0 ? <div className="v3-history-state">No commits are available in this 30-day window.</div> : (
                    <ol className="v3-history-commit-rail">{history.git.commits.map((commit) => {
                      const commitUrl = githubCommitUrl(history.git?.github_repository_url, commit.sha);
                      return (
                        <li key={commit.sha} className="v3-history-commit"><span className="v3-history-commit-node" aria-hidden="true" />
                          <div className="v3-history-commit-heading">
                            {commitUrl ? (
                              <a className="v3-history-commit-link" href={commitUrl} target="_blank" rel="noopener noreferrer"
                                title={`Open ${commit.sha} on GitHub`} aria-label={`Open commit ${commit.short_sha} on GitHub`}
                                onClick={(event) => { event.preventDefault(); void openExternalUrl(commitUrl); }}>
                                <code>{commit.short_sha}</code>
                              </a>
                            ) : <code title={commit.sha}>{commit.short_sha}</code>}
                            <time dateTime={new Date(commit.committed_at * 1_000).toISOString()}>{formatDate(commit.committed_at)}</time><strong>{commit.subject}</strong>
                          </div>
                          {commit.thread_refs.length > 0 && <div className="v3-history-thread-list">{orderedReferences(commit.thread_refs).map((reference) => { const thread = threads.get(reference.thread_id); return thread ? renderThread(thread, `${commit.sha}:${thread.thread_id}`) : null; })}</div>}
                        </li>
                      );
                    })}</ol>
                  )}
                </section>
              </>
            ) : (
              <section
                aria-labelledby={embedded && !compactSelectionReview ? "project-activity-heading" : !embedded ? "codex-sessions-heading" : undefined}
                aria-label={compactSelectionReview ? embeddedTitle : undefined}
              >
                {!embedded && <div className="v3-history-section-heading"><h2 id="codex-sessions-heading" className="v3-history-codex-heading"><Icon name="openai" size={14} className="v3-openai-icon" />Codex threads</h2><span className="v3-history-heading-count" title={`${history.threads.length} threads`}><Icon name="message" size={12} />{history.threads.length}</span></div>}
                <div className="v3-history-thread-list flat">{flatThreadRows.length ? flatThreadRows.map((row) => row.kind === "local"
                  ? renderThread(row.thread, row.thread.thread_id)
                  : <StoredThreadCard key={`stored:${row.entry.resource_id}`} entry={row.entry} storageName={comparisonStorageName}
                    selectionMode={selectionMode} selected={selectedResourceIds.has(row.entry.resource_id)}
                    selectable={selectableResourceIds?.has(row.entry.resource_id) ?? !!selectionMode}
                    selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} />)
                  : <div className="v3-history-state">No project-owned Codex sessions were found in this 30-day window.</div>}</div>
              </section>
            )}
            {history.next_before != null && <button type="button" className="btn v3-history-load-more" disabled={loadingMore} onClick={onLoadMore}>{loadingMore ? "Loading…" : "Load previous 30 days"}</button>}
          </>
        )}
      </section>
  );

  if (embedded) {
    return <div className={`v3-history-page v3-history-embedded${compactSelectionReview ? " v3-history-selection-review" : ""}`}>{content}</div>;
  }
  return <main className="v3-main v3-project-links-page v3-git-info-page v3-history-page">{content}</main>;
}

function mergePages(previous: ProjectChatHistory | null, next: ProjectChatHistory): ProjectChatHistory {
  if (!previous || previous.git?.selected_branch !== next.git?.selected_branch) return next;
  const threads = new Map(previous.threads.map((thread) => [thread.thread_id, thread]));
  next.threads.forEach((thread) => threads.set(thread.thread_id, thread));
  const unmapped = new Map(previous.unmapped.map((item) => [item.thread_id, item]));
  next.unmapped.forEach((item) => unmapped.set(item.thread_id, item));
  if (!previous.git || !next.git) return { ...next, threads: [...threads.values()], unmapped: [...unmapped.values()] };
  const commits = new Map(previous.git.commits.map((commit) => [commit.sha, commit]));
  next.git.commits.forEach((commit) => {
    const existing = commits.get(commit.sha);
    if (!existing) { commits.set(commit.sha, commit); return; }
    const references = new Map(existing.thread_refs.map((reference) => [reference.thread_id, reference]));
    commit.thread_refs.forEach((reference) => references.set(reference.thread_id, reference));
    commits.set(commit.sha, { ...commit, thread_refs: [...references.values()] });
  });
  const mergedCommits = [...commits.values()].sort((left, right) => right.committed_at - left.committed_at);
  const references = mergedCommits.reduce((count, commit) => count + commit.thread_refs.length, 0);
  const unique = new Set(mergedCommits.flatMap((commit) => commit.thread_refs.map((reference) => reference.thread_id))).size;
  const occurrences = new Map<string, number>();
  mergedCommits.forEach((commit) => commit.thread_refs.forEach((reference) => occurrences.set(reference.thread_id, (occurrences.get(reference.thread_id) ?? 0) + 1)));
  const mergedThreads = [...threads.values()].map((thread) => ({ ...thread, commit_occurrence_count: occurrences.get(thread.thread_id) ?? 0 }));
  return { ...next, threads: mergedThreads, unmapped: [...unmapped.values()], git: { ...next.git, commits: mergedCommits, reference_count: references, unique_thread_count: unique } };
}

export default function ProjectChatHistoryPage({
  project,
  binding,
  refreshEpoch,
  embedded = false,
  activeStorageId = null,
  activeStorageName = null,
  selectionMode,
  selectedResourceIds,
  selectableResourceIds,
  selectionDisabled = false,
  onToggleResource,
  comparisonOverride,
}: PageProps) {
  const [history, setHistory] = useState<ProjectChatHistory | null>(null);
  const [comparison, setComparison] = useState<ThreadSyncComparison | null>(null);
  const [comparisonLoading, setComparisonLoading] = useState(false);
  const [comparisonError, setComparisonError] = useState<string | null>(null);
  const [branch, setBranch] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionBusyThreadId, setActionBusyThreadId] = useState<string | null>(null);
  const [detailsByThread, setDetailsByThread] = useState<Record<string, ThreadDetailsState>>({});
  const [openDetailOccurrences, setOpenDetailOccurrences] = useState<Set<string>>(new Set());
  const requestRef = useRef(0);
  const comparisonRequestRef = useRef(0);
  const detailRequestRef = useRef<Record<string, number>>({});
  const detailSequenceRef = useRef(0);
  const contextKey = `${project.local_project_id}:${binding?.revision ?? "none"}:${binding?.profile_ids?.codex ?? "none"}`;
  const contextKeyRef = useRef(contextKey);
  contextKeyRef.current = contextKey;

  const load = async (beforeTime?: number | null, requestedBranch = branch, forceRevalidate = false) => {
    if (!binding?.profile_ids?.codex) return;
    const requestId = ++requestRef.current;
    if (beforeTime) setLoadingMore(true); else setLoading(true);
    setActionError(null);
    try {
      const next = await projectSyncApi.getProjectChatHistory(project.local_project_id, requestedBranch, beforeTime, 30, forceRevalidate);
      if (requestId !== requestRef.current) return;
      setHistory((current) => beforeTime ? mergePages(current, next) : next);
      setBranch(next.git?.selected_branch ?? null);
    } catch (reason) {
      if (requestId === requestRef.current) setActionError(errorMessage(reason));
    } finally {
      if (requestId === requestRef.current) { setLoading(false); setLoadingMore(false); }
    }
  };

  const loadComparison = async () => {
    if (comparisonOverride !== undefined) {
      comparisonRequestRef.current += 1;
      setComparison(comparisonOverride);
      setComparisonLoading(false);
      setComparisonError(null);
      return;
    }
    if (!binding?.profile_ids?.codex || !activeStorageId) {
      comparisonRequestRef.current += 1;
      setComparison(null);
      setComparisonLoading(false);
      setComparisonError(null);
      return;
    }
    const requestId = ++comparisonRequestRef.current;
    setComparisonLoading(true);
    setComparisonError(null);
    try {
      const next = await projectSyncApi.getThreadSyncComparison(project.local_project_id, activeStorageId);
      if (requestId !== comparisonRequestRef.current) return;
      setComparison(next);
    } catch (reason) {
      if (requestId !== comparisonRequestRef.current) return;
      setComparison(null);
      setComparisonError(errorMessage(reason));
    } finally {
      if (requestId === comparisonRequestRef.current) setComparisonLoading(false);
    }
  };

  useEffect(() => {
    detailRequestRef.current = {};
    setHistory(null); setBranch(null); setDetailsByThread({}); setOpenDetailOccurrences(new Set()); void load(null, null);
    return () => { requestRef.current += 1; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.local_project_id, binding?.revision, refreshEpoch]);

  useEffect(() => {
    setComparison(null);
    void loadComparison();
    return () => { comparisonRequestRef.current += 1; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.local_project_id, binding?.revision, activeStorageId, refreshEpoch, comparisonOverride]);

  const changeBranch = (nextBranch: string) => { setBranch(nextBranch); setHistory(null); setDetailsByThread({}); setOpenDetailOccurrences(new Set()); void load(null, nextBranch); };
  const openCodex = async (threadId: string) => {
    setActionError(null);
    if (!THREAD_UUID.test(threadId)) { setActionError("This session has an invalid ID and cannot be opened."); return; }
    setActionBusyThreadId(threadId);
    try { await projectSyncApi.openCodexThreadInApp(project.local_project_id, threadId); }
    catch (reason) { setActionError(errorMessage(reason)); }
    finally { setActionBusyThreadId(null); }
  };
  const openTerminal = async (threadId: string) => {
    setActionError(null); setActionBusyThreadId(threadId);
    try { await projectSyncApi.openCodexThreadInTerminal(project.local_project_id, threadId); }
    catch (reason) { setActionError(errorMessage(reason)); }
    finally { setActionBusyThreadId(null); }
  };
  const loadDetails = async (threadId: string, cursor?: number | null) => {
    const projectId = project.local_project_id;
    const requestContext = contextKey;
    const requestId = ++detailSequenceRef.current;
    detailRequestRef.current[threadId] = requestId;
    setDetailsByThread((current) => ({ ...current, [threadId]: { loading: true, error: null, page: current[threadId]?.page ?? null } }));
    try {
      const next = await projectSyncApi.getProjectChatThreadDetails(projectId, threadId, cursor, CHAT_HISTORY_PAGE_SIZE);
      if (contextKeyRef.current !== requestContext || detailRequestRef.current[threadId] !== requestId) return;
      setDetailsByThread((current) => {
        const previous = current[threadId]?.page;
        const turns = cursor != null && previous ? [...next.turns, ...previous.turns] : next.turns;
        return { ...current, [threadId]: { loading: false, error: null, page: { ...next, turns } } };
      });
    } catch (reason) {
      if (contextKeyRef.current !== requestContext || detailRequestRef.current[threadId] !== requestId) return;
      setDetailsByThread((current) => ({ ...current, [threadId]: { loading: false, error: errorMessage(reason), page: current[threadId]?.page ?? null } }));
    }
  };
  const toggleDetails = (threadId: string, occurrenceKey: string) => {
    const wasOpen = openDetailOccurrences.has(occurrenceKey);
    setOpenDetailOccurrences((current) => {
      const next = new Set(current);
      if (wasOpen) next.delete(occurrenceKey); else next.add(occurrenceKey);
      return next;
    });
    if (!wasOpen && !detailsByThread[threadId]?.page && !detailsByThread[threadId]?.loading) void loadDetails(threadId);
  };

  return <ProjectChatHistoryContent project={project} binding={binding} history={history} loading={loading} loadingMore={loadingMore} actionError={actionError} actionBusyThreadId={actionBusyThreadId} detailsByThread={detailsByThread} openDetailOccurrences={openDetailOccurrences} onBranchChange={changeBranch} onRefresh={() => { void load(null, branch, true); void loadComparison(); }} onLoadMore={() => void load(history?.next_before ?? null, branch)} onOpenCodex={(id) => void openCodex(id)} onOpenTerminal={(id) => void openTerminal(id)} onToggleDetails={toggleDetails} onLoadMoreDetails={(id) => void loadDetails(id, detailsByThread[id]?.page?.next_cursor)} embedded={embedded} comparison={comparison} comparisonLoading={comparisonLoading} comparisonError={comparisonError} activeStorageName={activeStorageName} selectionMode={selectionMode} selectedResourceIds={selectedResourceIds} selectableResourceIds={selectableResourceIds} selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} />;
}
