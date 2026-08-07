import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import type {
  CodexForkCheckpoint,
  CodexThreadDetailsPage,
  CodexThreadSummary,
  GitBranchSummary,
  LocalProjectSummary,
  ProjectBinding,
  ProjectChatHistory,
  RestoreResult,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../types";
import Icon, { type IconName } from "../Icons";
import { projectSyncApi } from "./api";
import { compactProjectPath, errorMessage, projectLabel } from "./model";
import { sessionName } from "./syncOutcome";
import ThreadStatusIcon, { type ThreadStatusIconName } from "./ThreadStatusIcon";

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
  onReviewPull?: () => void;
  onUnlinkStorage?: () => void;
  continuityActionBusy?: boolean;
  comparisonOverride?: ThreadSyncComparison | null;
  /** Per-row conflict resolution control, rendered under the row title. */
  rowResolution?: (resourceId: string) => ReactNode;
  /**
   * Fires with whether the review list is trustworthy: the timeline needs
   * `history` and the storage rows ("Only in …") need the comparison, both
   * fetched here asynchronously. A hosting review gates its primary action
   * on this so nothing is applied sight unseen.
   */
  onReviewListReadyChange?: (ready: boolean) => void;
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
  onFork?: (threadId: string) => void;
  forkBusyThreadId?: string | null;
  forkPanel?: ReactNode;
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
  onReviewPull?: () => void;
  onUnlinkStorage?: () => void;
  continuityActionBusy?: boolean;
  /** Per-row conflict resolution control, rendered under the row title. */
  rowResolution?: (resourceId: string) => ReactNode;
}

const THREAD_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const CHAT_HISTORY_PAGE_SIZE = 10;

export interface ForkCompletionFeedback {
  message: string;
  success: boolean;
  /** Present when the result named the fork it created — the banner then
   *  draws the thread structurally instead of printing the sentence. */
  fork?: {
    name: string;
    providerThreadId: string;
    /** Seconds; stamped at receipt, which is when the fork finished. */
    createdAt: number;
  } | null;
}

export function ForkCompletionBanner({ feedback, onDismiss }: {
  feedback: ForkCompletionFeedback;
  onDismiss: () => void;
}) {
  const fork = feedback.success ? feedback.fork : null;
  if (!fork) {
    return (
      <div className={`sync-callout${feedback.success ? "" : " error"}`}
        role={feedback.success ? "status" : "alert"}>
        <Icon name={feedback.success ? "git-fork" : "alert-triangle"} size={15} />
        <span>{feedback.message}</span>
      </div>
    );
  }
  // A fork is a timeline event — the thread it created is sitting in the
  // Uncommitted group below — so the notice takes the cards' shape and the
  // group headings' grammar: identity glyph, pixel-face phrase, the name in
  // the clean face ("Only in {storage}" exactly), with line 2 in the row
  // grammar (whole id left, stamp at the rail). The glyph lives in the
  // heading like the cloud and pencil do, not out on the rail — no line
  // passes the banner, so a rail mark would float in empty space.
  return (
    <section className="sync-history-fork-event" role="status">
      <div className="sync-history-fork-line">
        <Icon name="git-fork" size={13} />
        <span className="sync-history-fork-label">Created the Codex fork</span>
        <strong>{sessionName(fork.name).name}</strong>
        <button type="button" className="sync-notice-dismiss" title="Dismiss"
          aria-label="Dismiss fork notice" onClick={onDismiss}>
          <Icon name="x" size={12} />
        </button>
      </div>
      <span className="sync-history-fork-meta">
        <span className="sync-history-stored-thread-id"><code>{fork.providerThreadId}</code></span>
        <time dateTime={new Date(fork.createdAt * 1_000).toISOString()}>
          {formatDate(fork.createdAt)}
        </time>
      </span>
    </section>
  );
}

interface ForkActivityPanelProps {
  feedback: ForkCompletionFeedback | null;
  checkpoint: CodexForkCheckpoint | null;
  selectionMode?: "push" | "pull";
  resolving: boolean;
  onRetry: () => void;
  onDismissFeedback: () => void;
}

export function ForkActivityPanel({
  feedback,
  checkpoint,
  selectionMode,
  resolving,
  onRetry,
  onDismissFeedback,
}: ForkActivityPanelProps) {
  const retryable = checkpoint?.status === "fork_unacknowledged";
  if (!feedback && !checkpoint) return null;
  return (
    <>
      {feedback && <ForkCompletionBanner feedback={feedback} onDismiss={onDismissFeedback} />}
      {checkpoint && !selectionMode && (
        <section className="sync-fork-checkpoint" aria-live="polite">
          <h4>
            <Icon name="git-branch" size={15} />
            {retryable ? "Fork not acknowledged" : "Fork recovery needs attention"}
          </h4>
          <p>{checkpoint.message}</p>
          {retryable && (
            <p>A fork may already exist in Codex. Retrying may create another fork.</p>
          )}
          <dl>
            <div><dt>Source thread</dt><dd>{checkpoint.source_provider_thread_id}</dd></div>
            <div><dt>Suggested name</dt><dd>{checkpoint.suggested_fork_name}</dd></div>
          </dl>
          {retryable && (
            <div className="sync-fork-checkpoint-actions">
              <button type="button" className="btn btn-primary" disabled={resolving}
                onClick={onRetry}>
                {resolving ? "Retrying…" : "Retry fork"}
              </button>
            </div>
          )}
        </section>
      )}
    </>
  );
}

export async function refreshActivityAndHandleForkFeedback(
  refreshHistory: () => Promise<void>,
  refreshComparison: () => Promise<void>,
  dismiss: () => void,
  dismissForkFeedback: boolean,
) {
  await Promise.all([refreshHistory(), refreshComparison()]);
  if (dismissForkFeedback) dismiss();
}

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
    <div ref={rootRef} className={`sync-history-branch-select${open ? " open" : ""}`}>
      <Icon name="git-branch" size={15} />
      <button
        ref={triggerRef}
        type="button"
        className="sync-history-branch-trigger"
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
        {/* The trigger is just the branch name: it always shows the selected
            branch, so "Current" belongs in the open menu, not here. */}
        <span className="sync-history-branch-name" title={selected.name}>{selected.name}</span>
        {!selected.available && <span className="sync-history-branch-state history-only">History only</span>}
        <Icon name="chevron-down" size={12} className="sync-history-branch-chevron" />
      </button>
      <div
        ref={menuRef}
        id={menuId}
        className="sync-history-branch-menu"
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
        <div className="sync-history-branch-menu-heading">
          <span>Switch branch</span>
          <small>{branches.length}</small>
        </div>
        <div className="sync-history-branch-options">
          {branches.map((branch) => (
            <button
              key={branch.name}
              type="button"
              role="menuitemradio"
              aria-checked={branch.name === selectedBranch}
              className="sync-history-branch-option"
              data-branch={branch.name}
              title={!branch.available ? "This branch is no longer available locally; recorded session history can still be shown." : branch.name}
              onClick={() => {
                closeMenu(true);
                if (branch.name !== selectedBranch) onChange(branch.name);
              }}
            >
              <span className="sync-history-branch-check" aria-hidden="true">
                {branch.name === selectedBranch && <Icon name="check-circle" size={12} />}
              </span>
              {/* The badges already say what the second line used to; the full
                  explanation stays on the row's title. */}
              <span className="sync-history-branch-option-copy">
                <strong>{branch.name}</strong>
              </span>
              {branch.is_current && <span className="sync-history-branch-state current">Current</span>}
              {!branch.available && <span className="sync-history-branch-state history-only">History only</span>}
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

function formatCalendarDate(value: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short", day: "numeric", year: "numeric",
  }).format(new Date(value * 1_000));
}

function formatClockTime(value: number): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric", minute: "2-digit",
  }).format(new Date(value * 1_000));
}

function formatFullDateTime(value: number): string {
  return `${formatCalendarDate(value)} at ${formatClockTime(value)}`;
}

/**
 * How long the session ran, at one significant unit. The absolute start stamp
 * plus this covers the same ground the old start→end pair did, in far less
 * room. Returns null when the session has no measurable duration.
 */
function formatDuration(startedAt: number, endedAt: number): string | null {
  const seconds = Math.max(0, Math.round(endedAt - startedAt));
  if (seconds === 0) return null;
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  if (hours < 24) return restMinutes ? `${hours}h ${restMinutes}m` : `${hours}h`;
  const days = Math.floor(hours / 24);
  const restHours = hours % 24;
  return restHours ? `${days}d ${restHours}h` : `${days}d`;
}

function formatCount(value?: number | null): string {
  if (value == null) return "Not reported";
  return new Intl.NumberFormat(undefined, { notation: value >= 1_000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(value);
}

export function compactThreadTitle(value: string): string {
  return value.replace(
    /\s*·\s*\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}(?::\d{2})?Z?)?\s*$/,
    "",
  );
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
  status: ThreadStatusIconName;
  className: string;
  label: string;
};

export function threadSyncPresentation(entry: ThreadSyncEntry, storageName: string): ThreadSyncPresentation {
  type ReasonCode = ThreadSyncEntry["push_decision"]["reason_code"];
  const decisionWithReason = (reasons: readonly ReasonCode[]) => {
    if (entry.push_decision && reasons.includes(entry.push_decision.reason_code)) {
      return { mode: "push" as const };
    }
    if (entry.pull_decision && reasons.includes(entry.pull_decision.reason_code)) {
      return { mode: "pull" as const };
    }
    return null;
  };
  const pending = decisionWithReason(["resolution_pending", "provider_not_quiescent", "stale_plan"]);
  if (pending) {
    return {
      status: "pending",
      className: "pending",
      label: syncHoldDetail(pending.mode, entry)
        ?? entry.status_detail
        ?? "Sync is waiting for this conversation to become ready.",
    };
  }
  const conflict = decisionWithReason(["unknown_lineage", "ambiguous_binding", "missing_binding"]);
  if (conflict) {
    return {
      status: "conflict",
      className: "conflict",
      label: syncHoldDetail(conflict.mode, entry)
        ?? entry.status_detail
        ?? "Mallard could not safely match the local and stored conversation.",
    };
  }
  const blocked = (mode: "push" | "pull"): ThreadSyncPresentation => ({
    status: "unavailable",
    className: "unavailable",
    label: syncHoldDetail(mode, entry) ?? "This thread can't be synced safely right now.",
  });
  switch (entry.state) {
    case "synced":
      return { status: "synced", className: "synced", label: `Up to date here and in ${storageName}.` };
    case "local_only":
      if (entry.push_decision?.action === "blocked") return blocked("push");
      return { status: "local-outstanding", className: "local", label: `Only on this computer. Push to save it in ${storageName}.` };
    case "local_ahead":
      if (entry.push_decision?.action === "blocked") return blocked("push");
      return { status: "local-outstanding", className: "local", label: `Newer on this computer. Push to update ${storageName}.` };
    case "storage_only":
      if (entry.pull_decision?.action === "blocked") return blocked("pull");
      return { status: "cloud-outstanding", className: "storage", label: `Only in ${storageName}. Pull to download it here.` };
    case "storage_ahead":
      if (entry.pull_decision?.action === "blocked") return blocked("pull");
      return { status: "cloud-outstanding", className: "storage", label: `Newer in ${storageName}. Pull to update this computer.` };
    case "diverged":
      return { status: "divergent", className: "diverged", label: `Changed here and in ${storageName}. Review before syncing.` };
    case "diverged_forked": {
      const forkName = entry.fork_preservation?.fork_display_name;
      return {
        status: "divergent-forked",
        className: "diverged-forked",
        label: `Changed here and in ${storageName}; your local branch is already preserved in ${forkName ? `the fork “${forkName}”` : "a fork"}. Pull to adopt the ${storageName} version.`,
      };
    }
    case "unavailable":
      return {
        status: "unavailable",
        className: "unavailable",
        label: `Unavailable for sync. ${entry.status_detail ?? "Mallard could not safely read this session."}`,
      };
    case "unknown":
    default:
      return {
        status: "conflict",
        className: "conflict",
        label: `Comparison needs attention. ${entry.status_detail ?? "Mallard could not prove this thread's ancestry safely."}`,
      };
  }
}

function ThreadSyncIndicator({ entry, storageName }: { entry: ThreadSyncEntry; storageName: string }) {
  const presentation = threadSyncPresentation(entry, storageName);
  return (
    <span
      className={`sync-thread-sync-indicator ${presentation.className}`}
      data-tooltip={presentation.label}
      aria-label={presentation.label}
      tabIndex={0}
    >
      <ThreadStatusIcon status={presentation.status} size={18} />
    </span>
  );
}

/* One absolute stamp, pushed to the far edge so it forms a column down the
   list. It anchors on the last activity because that is the more useful end of
   the interval; the start is that minus the duration sitting beside the other
   readouts. Both ends are spelled out in full in the label. */
function ThreadTimestamp({ thread }: { thread: CodexThreadSummary }) {
  const startedLabel = `Started ${formatFullDateTime(thread.started_at)}`;
  const endedLabel = `${thread.is_active ? "Last activity" : "Ended"} ${formatFullDateTime(thread.ended_at)}${thread.is_active ? ", active" : ""}`;
  return (
    <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`${startedLabel}\n${endedLabel}`} aria-label={`${startedLabel}. ${endedLabel}`} tabIndex={0}>
      <time dateTime={new Date(thread.ended_at * 1_000).toISOString()}>{formatFullDateTime(thread.ended_at)}</time>
    </span>
  );
}

/** Readouts only — everything in here is hovered, nothing is clicked. The
 *  card renders the stamp separately so it can trail the row's buttons; the
 *  standalone renders keep it attached. */
export function ThreadMetrics({
  thread,
  id,
  showTimestamp = true,
}: {
  thread: CodexThreadSummary;
  id?: string;
  showTimestamp?: boolean;
}) {
  const durationText = formatDuration(thread.started_at, thread.ended_at);
  const durationLabel = durationText
    ? `${thread.is_active ? "Active for" : "Ran for"} ${durationText}`
    : "";

  return <div id={id} className="sync-history-thread-meta" aria-label="Session details">
    <span className="sync-history-thread-metric" data-tooltip={`User turns · ${thread.user_round_count}`} aria-label={`User turns: ${thread.user_round_count}`} tabIndex={0}><Icon name="user" size={12} /><b>{thread.user_round_count}</b></span>
    <span className="sync-history-thread-metric" data-tooltip={`Total tokens · ${formatCount(thread.total_tokens)}`} aria-label={`Total tokens: ${formatCount(thread.total_tokens)}`} tabIndex={0}><Icon name="token" size={12} /><b>{formatCount(thread.total_tokens)}</b></span>
    <span className="sync-history-thread-metric" data-tooltip={`Agent messages · ${thread.agent_message_count}`} aria-label={`Agent messages: ${thread.agent_message_count}`} tabIndex={0}><Icon name="message" size={12} /><b>{thread.agent_message_count}</b></span>
    {thread.tool_call_count > 0 && <span className="sync-history-thread-metric" data-tooltip={`Tool calls · ${thread.tool_call_count}`} aria-label={`Tool calls: ${thread.tool_call_count}`} tabIndex={0}><Icon name="tool" size={12} /><b>{thread.tool_call_count}</b></span>}
    {!thread.metrics_complete && <span className="sync-history-thread-metric sync-history-partial" data-tooltip="Some session metrics are unavailable" aria-label="Some session metrics are unavailable" tabIndex={0}><Icon name="alert-triangle" size={12} /></span>}
    {durationText && (
      <span className="sync-history-thread-metric sync-history-thread-duration" data-tooltip={durationLabel} aria-label={durationLabel} tabIndex={0}>
        <Icon name={thread.is_active ? "activity" : "clock"} size={12} /><b>{durationText}</b>
      </span>
    )}
    {showTimestamp && <ThreadTimestamp thread={thread} />}
  </div>;
}

interface ThreadCardProps {
  thread: CodexThreadSummary;
  occurrenceKey: string;
  workingDirectory: string;
  codexHome: string;
  busy?: boolean;
  details?: ThreadDetailsState;
  detailsOpen?: boolean;
  onOpenCodex: (threadId: string) => void;
  onOpenTerminal: (threadId: string) => void;
  onFork?: (threadId: string) => void;
  forkBusy?: boolean;
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
  onReviewPull?: () => void;
  rowResolution?: (resourceId: string) => ReactNode;
}

type ThreadDebugCopyTarget = "thread-id" | "working-directory" | "codex-home" | "all";

interface ThreadDebugMenuProps {
  threadId: string;
  workingDirectory: string;
  codexHome: string;
}

export function threadDebugCopyText(
  threadId: string,
  workingDirectory: string,
  codexHome: string,
): string {
  return [
    `Thread ID: ${threadId}`,
    `Working directory: ${workingDirectory}`,
    `Codex home: ${codexHome}`,
  ].join("\n");
}

async function writeClipboardText(value: string, fallbackRoot: HTMLElement | null): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // Some WebViews expose Clipboard but still reject it. Use the local
      // document fallback while this user gesture is active.
    }
  }
  if (typeof document === "undefined") throw new Error("Clipboard access is unavailable");
  const field = document.createElement("textarea");
  field.value = value;
  field.readOnly = true;
  field.style.position = "fixed";
  field.style.opacity = "0";
  field.style.pointerEvents = "none";
  const previousFocus = document.activeElement instanceof HTMLElement
    ? document.activeElement
    : null;
  (fallbackRoot ?? document.body).append(field);
  let copied = false;
  try {
    field.select();
    copied = document.execCommand("copy");
  } finally {
    field.remove();
    previousFocus?.focus();
  }
  if (!copied) throw new Error("Clipboard access is unavailable");
}

/** Roughly the panel's rendered height; only used to pick which way it opens. */
const THREAD_DEBUG_POPOVER_HEIGHT = 220;

function ThreadDebugMenu({ threadId, workingDirectory, codexHome }: ThreadDebugMenuProps) {
  const [open, setOpen] = useState(false);
  const [dropUp, setDropUp] = useState(false);
  const [copied, setCopied] = useState<ThreadDebugCopyTarget | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuId = `thread-debug-${useId().replace(/:/g, "")}`;
  const rows: Array<{ key: Exclude<ThreadDebugCopyTarget, "all">; label: string; value: string }> = [
    { key: "thread-id", label: "Thread ID", value: threadId },
    { key: "working-directory", label: "Working directory", value: workingDirectory },
    { key: "codex-home", label: "Codex home", value: codexHome },
  ];
  const copiedLabel = copied === "thread-id"
    ? "Thread ID copied."
    : copied === "working-directory"
      ? "Working directory copied."
      : copied === "codex-home"
        ? "Codex home copied."
        : copied === "all"
          ? "Debug info copied."
          : "";

  const closeMenu = (restoreFocus = false) => {
    setOpen(false);
    setCopied(null);
    setCopyError(null);
    if (restoreFocus) window.requestAnimationFrame(() => triggerRef.current?.focus());
  };

  // Hanging the panel below a row near the end of the list grows the
  // workspace's scrollable height, which makes the scrollbar jump the moment
  // it opens. Overflow past the top of a scroller is not scrollable, so those
  // rows open upward instead; rows with room below keep opening down.
  useEffect(() => {
    if (!open) {
      setDropUp(false);
      return;
    }
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const scroller = trigger.closest<HTMLElement>(".sync-main")?.getBoundingClientRect();
    const roomBelow = (scroller?.bottom ?? window.innerHeight) - rect.bottom;
    const roomAbove = rect.top - (scroller?.top ?? 0);
    // Only flip when down does not fit *and* up is the roomier side, so a row
    // that fits neither way still opens toward whichever clips it less.
    setDropUp(roomBelow < THREAD_DEBUG_POPOVER_HEIGHT && roomAbove > roomBelow);
  }, [open]);

  const copyValue = async (target: ThreadDebugCopyTarget, value: string) => {
    try {
      await writeClipboardText(value, rootRef.current);
      setCopyError(null);
      setCopied(target);
    } catch (reason) {
      setCopied(null);
      setCopyError(`Could not copy: ${errorMessage(reason)}`);
    }
  };

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

  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(null), 2_000);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  return (
    <div ref={rootRef} className="sync-history-debug-menu">
      <button
        ref={triggerRef}
        type="button"
        className="btn btn-ghost sync-history-debug-trigger"
        title="Thread debug info"
        aria-label="Show thread debug info"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={menuId}
        onClick={() => open ? closeMenu() : setOpen(true)}
      >
        <Icon name="info" size={14} />
      </button>
      <div
        id={menuId}
        className={`sync-history-debug-popover${dropUp ? " drop-up" : ""}`}
        role="dialog"
        aria-label="Thread debug info"
        hidden={!open}
      >
        <header>
          <strong>Thread debug info</strong>
          <span>Click a value to copy it.</span>
        </header>
        <div className="sync-history-debug-values">
          {rows.map((row) => (
            <button
              key={row.key}
              type="button"
              className="sync-history-debug-copy"
              aria-label={`Copy ${row.label.toLowerCase()}`}
              title={`Copy ${row.label.toLowerCase()}`}
              onClick={() => void copyValue(row.key, row.value)}
            >
              <span>{row.label}</span>
              <code>{row.value}</code>
              <small className={copied === row.key ? "copied" : undefined}>
                {copied === row.key ? "Copied" : "Copy"}
              </small>
            </button>
          ))}
        </div>
        <footer>
          <span
            className={`sync-history-debug-status${copyError ? " error" : copied ? " success" : ""}`}
            role="status"
            aria-live="polite"
          >
            {copyError ?? copiedLabel}
          </span>
          <button
            type="button"
            className="btn btn-ghost sync-history-debug-copy-all"
            onClick={() => void copyValue(
              "all",
              threadDebugCopyText(threadId, workingDirectory, codexHome),
            )}
          >
            {copied === "all" ? "Copied all" : "Copy all"}
          </button>
        </footer>
      </div>
    </div>
  );
}

/**
 * Per-row "Open Pull review" affordance for push rows held behind a Pull
 * (diverged or behind storage). The hold applies to this one thread; the
 * rest of the Push proceeds without it.
 */
function ReviewPullRowAction({
  mode,
  entry,
  disabled,
  onReviewPull,
}: {
  mode: "push" | "pull" | undefined;
  entry: ThreadSyncEntry | undefined;
  disabled: boolean;
  onReviewPull?: () => void;
}) {
  if (mode !== "push" || !entry || !onReviewPull) return null;
  if (entry.push_decision?.action !== "require_pull") return null;
  const detail = syncHoldDetail("push", entry)
    ?? "Pull this conversation before pushing it.";
  return (
    <button
      type="button"
      className="btn btn-ghost sync-sync-review-pull-action"
      disabled={disabled}
      title={detail}
      aria-label={`Open Pull review — ${detail}`}
      onClick={onReviewPull}
    >
      Open Pull review
    </button>
  );
}

function syncReviewStateLabel(
  mode: "push" | "pull",
  entry: ThreadSyncEntry | undefined,
  selected: boolean,
): string | null {
  if (!entry) return null;
  if (entry.state === "unavailable") return "Unavailable for sync";
  if (entry.cloud_thread_id) {
    const decision = mode === "push" ? entry.push_decision : entry.pull_decision;
    switch (decision.action) {
      case "no_op": return null;
      case "fast_forward_cloud": return selected ? "Update cloud" : "Not selected";
      case "fast_forward_local": return selected ? "Update this thread" : "Keep local";
      case "keep_local": return "Keep local";
      case "require_pull": return "Pull required";
      case "resolve_divergence":
        if (!selected) return "Not selected";
        // The same words the row's own resolution control uses: a branch
        // already preserved in a fork is simply overwritten, a fresh one is
        // forked first. One vocabulary for the phrase and the buttons.
        return entry.state === "diverged_forked" ? "Overwrite" : "Fork and overwrite";
      case "create_cloud": return selected ? "Publish new thread" : "Not selected";
      case "create_local": return selected ? "Restore new conversation" : "Not selected";
      case "store_only": return "Backup only";
      case "replace_local": return selected ? "Replace local branch" : "Needs approval";
      case "replace_cloud": return selected ? "Replace cloud branch" : "Needs approval";
      case "tombstone_thread": return selected ? "Delete thread" : "Needs approval";
      case "blocked": return "Can't sync";
    }
  }
  if (mode === "push") {
    if (entry.state === "storage_only" || entry.state === "storage_ahead" || entry.state === "diverged" || entry.state === "diverged_forked") {
      return "Pull required";
    }
    return entry.state === "unknown" ? "Review status" : null;
  }
  if (entry.state === "diverged" || entry.state === "diverged_forked") return selected ? "Use storage version" : "Keep local";
  if (entry.state === "storage_only" || entry.state === "storage_ahead") {
    return selected ? "Restore from storage" : "Keep local";
  }
  if (entry.state === "local_only" || entry.state === "local_ahead") return "Keep local";
  return entry.state === "synced" ? null : "Review status";
}

/**
 * Fine print for rows that cannot proceed in the active direction. Speaks
 * user vocabulary only — sync verdicts never gate opening a thread.
 */
export function syncHoldDetail(mode: "push" | "pull", entry: ThreadSyncEntry): string | null {
  if (entry.state === "unavailable") {
    return entry.status_detail ?? "Mallard could not safely read this session.";
  }
  const decision = mode === "push" ? entry.push_decision : entry.pull_decision;
  if (!decision) return null;
  if (decision.action === "blocked") {
    switch (decision.reason_code) {
      case "codec_unsupported":
        return mode === "push"
          ? "Can't sync — this session was saved by an older Codex version. It still opens on this computer."
          : "Can't sync — this stored copy can't be restored safely on this computer.";
      case "ambiguous_binding":
        return "Can't sync — more than one stored thread could match this session.";
      case "missing_binding":
        return "Can't sync — this session isn't linked to a stored thread yet.";
      case "provider_not_quiescent":
        return "Can't sync right now — Codex may still be writing this session.";
      case "stale_plan":
        return "The comparison is out of date. Refresh and review again.";
      case "resolution_pending":
        return "A Codex fork checkpoint from an earlier Pull is pending. Retry it in the Pull review.";
      default:
        return "Can't sync — this session can't be changed safely right now.";
    }
  }
  if (decision.action === "store_only") {
    return decision.reason_code === "provider_not_quiescent"
      ? entry.status_detail
        ?? "Conflict resolution is unavailable right now. Finish any active Codex turn, then refresh."
      : "Pulling saves a backup copy that can't be opened on this computer.";
  }
  if (mode === "push" && decision.action === "require_pull") {
    if (entry.state === "diverged_forked") {
      return "Local and stored history have diverged, but the local branch is already preserved in a fork. Pull to adopt the stored version.";
    }
    return entry.state === "diverged"
      ? "Local and stored history have diverged. Pull and choose how to preserve the local branch before pushing."
      : "The stored thread has newer messages. Pull them before pushing.";
  }
  return null;
}

// The row leads with an icon for how this thread stands against storage. It is
// keyed off the entry state, never off the selection: an icon that swapped
// glyphs when its own checkbox was ticked changed under the cursor doing the
// ticking. What the selection would *do* is the phrase, and the phrase lives
// in the tooltip and the accessibility label, where it costs no layout.
const REVIEW_STATE_ICONS: Record<string, IconName> = {
  synced: "check-circle",
  local_only: "upload",
  local_ahead: "upload",
  storage_only: "download",
  storage_ahead: "download",
  diverged: "git-compare-arrows",
  diverged_forked: "git-compare-arrows",
  unavailable: "alert-triangle",
  unknown: "help-circle",
};

const INDICATOR_STATE_CLASS: Record<string, string> = {
  synced: "synced",
  local_only: "local",
  local_ahead: "local",
  storage_only: "storage",
  storage_ahead: "storage",
  diverged: "diverged",
  diverged_forked: "diverged-forked",
  unavailable: "unavailable",
  unknown: "pending",
};

function SyncDecisionBadge({ label, state, detail, blocked = false }: {
  label: string;
  state: string | undefined;
  detail: string | null;
  blocked?: boolean;
}) {
  return (
    <span
      className={`sync-thread-sync-indicator ${INDICATOR_STATE_CLASS[state ?? "unknown"] ?? "pending"}`}
      data-tooltip={detail ? `${label}\n${detail}` : label}
      aria-label={detail ? `${label}. ${detail}` : label}
      tabIndex={0}
    >
      <Icon
        name={blocked ? "octagon-x" : REVIEW_STATE_ICONS[state ?? "unknown"] ?? "help-circle"}
        size={13}
      />
    </span>
  );
}

function ReviewRequiredBadge() {
  return (
    <span
      className="sync-thread-sync-error"
      data-tooltip={"Review required\nThis change needs your explicit approval before it applies."}
      aria-label="Review required. This change needs your explicit approval before it applies."
      tabIndex={0}
    >
      <Icon name="alert-triangle" size={12} />
    </span>
  );
}

function ThreadCard({
  thread, occurrenceKey, workingDirectory, codexHome, busy = false, details, detailsOpen = false, onOpenCodex, onOpenTerminal, onFork, forkBusy = false, onToggleDetails, onLoadMoreDetails,
  syncEntry, storageName, resourceId, selectionMode, selected = false, selectable = false,
  selectionDisabled = false, onToggleResource, onReviewPull, rowResolution,
}: ThreadCardProps) {
  const launchable = THREAD_UUID.test(thread.thread_id);
  const fullTitle = thread.title || "Untitled Codex session";
  const visibleTitle = compactThreadTitle(fullTitle);
  const chatDetailsId = `thread-chat-${occurrenceKey.replace(/[^a-z0-9_-]/gi, "-")}`;
  const chatDetailsLabel = detailsOpen
    ? "Hide chat history"
    : details?.page
      ? "Show chat history"
      : "Load chat history";
  const reviewStateLabel = selectionMode ? syncReviewStateLabel(selectionMode, syncEntry, selected) : null;
  const activeDecision = selectionMode && syncEntry
    ? (selectionMode === "push" ? syncEntry.push_decision : syncEntry.pull_decision)
    : null;
  const holdDetail = selectionMode && syncEntry ? syncHoldDetail(selectionMode, syncEntry) : null;
  const uncommitted = occurrenceKey.startsWith("uncommitted:");
  return (
    <article className={`sync-history-thread-card${selectionMode && selectable ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectionMode && selectable && (
            <label
              className="sync-sync-row-choice"
              title={`${selected ? "Exclude" : "Include"} ${thread.title || "this session"}`}
            >
              <input
                type="checkbox"
                checked={selected}
                disabled={selectionDisabled}
                aria-label={`${selected ? "Exclude" : "Include"} ${thread.title || "Untitled Codex session"}`}
                onChange={() => onToggleResource?.(resourceId)}
              />
            </label>
          )}
          {selectionMode && !selectable && activeDecision?.action === "blocked" && (
            <span
              className="sync-sync-row-choice sync-sync-row-blocked"
              title={holdDetail ?? "Can't sync"}
              aria-label={holdDetail ?? "Can't sync"}
            >
              <Icon name="octagon-x" size={14} />
            </span>
          )}
          {/* The blocked octagon in the choice slot already carries the whole
              story; rows it marks render no second status icon. Rows with a
              review decision lead with it; everything else keeps the plain
              sync indicator (or the uncommitted mark). */}
          {selectionMode && !selectable && activeDecision?.action === "blocked" ? null
            : selectionMode && reviewStateLabel ? (
              <SyncDecisionBadge
                label={reviewStateLabel}
                state={syncEntry?.state}
                detail={holdDetail}
                blocked={activeDecision?.action === "blocked"}
              />
            ) : syncEntry && storageName ? (
              <ThreadSyncIndicator entry={syncEntry} storageName={storageName} />
            ) : uncommitted ? (
              // The slot without the glyph: the status badge that loads in
              // must land where the title already sits, and the group heading
              // carries the pencil, so the row-level copy of it said nothing.
              <span className="sync-history-thread-kind uncommitted" aria-hidden="true" />
            ) : null}
          {activeDecision?.requires_explicit_approval && <ReviewRequiredBadge />}
          <strong title={visibleTitle === fullTitle ? undefined : fullTitle}>{visibleTitle}</strong>
          <ReviewPullRowAction
            mode={selectionMode}
            entry={syncEntry}
            disabled={selectionDisabled || busy}
            onReviewPull={onReviewPull}
          />
        </div>
        <div className="sync-history-thread-actions">
          {!selectionMode && onFork && (
            <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!launchable || busy || forkBusy}
              onClick={() => onFork(thread.thread_id)} title={forkBusy ? "Forking in Codex…" : "Fork in Codex"}
              aria-label={forkBusy ? "Forking in Codex" : "Fork in Codex"}>
              <Icon name={forkBusy ? "refresh" : "git-fork"} size={14} className={forkBusy ? "icon-spin" : undefined} />
            </button>
          )}
          <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!launchable || busy}
            onClick={() => onOpenCodex(thread.thread_id)} title={busy ? "Opening in Codex…" : "Open in Codex"}
            aria-label={busy ? "Opening in Codex" : "Open in Codex"}>
            <Icon name={busy ? "refresh" : "openai"} size={14} className={busy ? "icon-spin" : "sync-openai-icon"} />
          </button>
          <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!launchable || busy}
            onClick={() => onOpenTerminal(thread.thread_id)} title="Open in Terminal" aria-label="Open in Terminal">
            <Icon name="terminal" size={14} />
          </button>
        </div>
      </div>

      {rowResolution?.(resourceId)}

      {/* Line 2 reads left to right: readouts you hover, a hairline rule, then
          the two clickables — debug info and the transcript — as one group,
          with the stamp closing the line at the card edge so it forms one
          column with every other stamp on the page. */}
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <ThreadMetrics thread={thread} showTimestamp={false} />
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <ThreadDebugMenu
            threadId={thread.thread_id}
            workingDirectory={workingDirectory}
            codexHome={codexHome}
          />
          {onToggleDetails && (
            <button type="button" className={`btn btn-ghost sync-history-chat-toggle${detailsOpen ? " active" : ""}`}
              aria-label={chatDetailsLabel} title={chatDetailsLabel} aria-expanded={detailsOpen} aria-controls={chatDetailsId}
              onClick={() => onToggleDetails(thread.thread_id, occurrenceKey)}>
              <Icon name={detailsOpen ? "chevron-up" : "chevron-down"} size={13} />
            </button>
          )}
          <ThreadTimestamp thread={thread} />
        </div>
        {onToggleDetails && detailsOpen && (
          <div id={chatDetailsId} className="sync-history-chat-details" aria-live="polite">
            {details?.loading && !details.page ? <div className="sync-history-detail-state"><span className="status-loader" /> Loading chat history…</div> : null}
            {details?.error && <div className="sync-history-detail-state error" role="alert">{details.error}</div>}
            {details?.page?.next_cursor != null && (
              <button type="button" className="btn btn-ghost sync-history-load-older" disabled={details.loading}
                onClick={() => onLoadMoreDetails?.(thread.thread_id)}>
                {details.loading ? "Loading older messages…" : `Load ${CHAT_HISTORY_PAGE_SIZE} older messages`}
              </button>
            )}
            {details?.page?.turns.map((turn) => (
              <div key={`${thread.thread_id}:${turn.ordinal}`} className="sync-history-turn">
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

function SyncEntryCard({
  entry,
  storageName,
  scopeStated = false,
  selectionMode,
  selected = false,
  selectable = false,
  selectionDisabled = false,
  onToggleResource,
  onReviewPull,
}: {
  entry: ThreadSyncEntry;
  storageName: string;
  /** Set when the surrounding section heading already names the storage, so the
   *  row does not repeat it on every card. */
  scopeStated?: boolean;
  selectionMode?: "push" | "pull";
  selected?: boolean;
  selectable?: boolean;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  onReviewPull?: () => void;
}) {
  const fullTitle = entry.display_name.trim() && entry.display_name !== entry.thread_id
    ? entry.display_name
    : "Untitled thread";
  // Local threads already drop the provenance timestamp their title carries;
  // stored threads get the same treatment so both lists read alike.
  const title = compactThreadTitle(fullTitle);
  const identityLabel = `${title}, Codex thread ${entry.thread_id}`;
  const location = entry.local_present && entry.storage_present
    ? `On this Mac · In ${storageName}`
    : entry.local_present
      ? "On this Mac"
      : `In ${storageName} · Pull to open`;
  const updatedAt = entry.storage_updated_at ?? entry.local_updated_at;
  // Where the section heading already names the storage, repeating it on every
  // card says nothing; elsewhere the row is the only thing that can say where
  // the thread lives and how to get it, so it keeps the full phrase.
  const visibleLocation = scopeStated ? null : location;
  const reviewStateLabel = selectionMode ? syncReviewStateLabel(selectionMode, entry, selected) : null;
  const activeDecision = selectionMode
    ? (selectionMode === "push" ? entry.push_decision : entry.pull_decision)
    : null;
  const holdDetail = selectionMode ? syncHoldDetail(selectionMode, entry) : null;
  return (
    <article className={`sync-history-thread-card sync-history-stored-thread${selectionMode && selectable ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        {/* No native title here: the status badge inside carries its own CSS
            tooltip, and a title on the wrapper fires a second, overlapping
            OS tooltip on the same hover. The id lives on the sha chip. */}
        <div className="sync-history-thread-title">
          {selectionMode && selectable && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${identityLabel}`}>
              <input
                type="checkbox"
                checked={selected}
                disabled={selectionDisabled}
                aria-label={`${selected ? "Exclude" : "Include"} ${identityLabel}`}
                onChange={() => onToggleResource?.(entry.resource_id)}
              />
            </label>
          )}
          {selectionMode && !selectable && activeDecision?.action === "blocked" && (
            <span
              className="sync-sync-row-choice sync-sync-row-blocked"
              title={holdDetail ?? "Can't sync"}
              aria-label={holdDetail ?? "Can't sync"}
            >
              <Icon name="octagon-x" size={14} />
            </span>
          )}
          {selectionMode && !selectable && activeDecision?.action === "blocked" ? null
            : selectionMode && reviewStateLabel ? (
              <SyncDecisionBadge
                label={reviewStateLabel}
                state={entry.state}
                detail={holdDetail}
                blocked={activeDecision?.action === "blocked"}
              />
            ) : (
              <ThreadSyncIndicator entry={entry} storageName={storageName} />
            )}
          {activeDecision?.requires_explicit_approval && <ReviewRequiredBadge />}
          <div className="sync-history-stored-thread-copy">
            <strong title={title === fullTitle ? undefined : fullTitle}>{title}</strong>
            {visibleLocation && <span className="sync-history-stored-thread-location">{visibleLocation}</span>}
          </div>
          <ReviewPullRowAction
            mode={selectionMode}
            entry={entry}
            disabled={selectionDisabled}
            onReviewPull={onReviewPull}
          />
        </div>
      </div>
      {/* Line 2 matches the completed sync's rows: the whole thread id, never
          a stub, with the stamp at the far edge. The id is inline inside the
          wrapper because a flex item is blockified and a blockified element
          serialises with a trailing newline — one click on the inline code
          takes all 36 characters whole. */}
      <span className="sync-history-stored-thread-meta">
        <span className="sync-history-stored-thread-id"><code>{entry.thread_id}</code></span>
        {updatedAt != null && (
          <time dateTime={new Date(updatedAt * 1_000).toISOString()}>{formatDate(updatedAt)}</time>
        )}
      </span>
    </article>
  );
}

/** The fold control lives in the timeline rail, where the pixel node used to
 *  sit — the node became the button. Every fold on the page shares the rail's
 *  column, and because the button is outside the card, neither a review's
 *  checkbox nor an outcome banner can move it. It precedes the heading and
 *  the list in the DOM, so the trigger reads before the content it controls.
 *  Leading-position rotation: right when collapsed, down when open. */
function SectionFold({ collapsed, controls, label, disabled = false, onToggle }: {
  collapsed: boolean;
  controls: string;
  label: string;
  disabled?: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className="sync-history-spine-fold"
      aria-label={label}
      title={label}
      aria-expanded={!collapsed}
      aria-controls={controls}
      disabled={disabled}
      onClick={onToggle}
    >
      <Icon name={collapsed ? "chevron-right" : "chevron-down"} size={12} />
    </button>
  );
}

export function ProjectChatHistoryContent({
  project, binding, history, loading, loadingMore, actionError, actionBusyThreadId,
  detailsByThread = {}, openDetailOccurrences = new Set(), onBranchChange, onRefresh, onLoadMore, onOpenCodex,
  onOpenTerminal, onFork, forkBusyThreadId = null, forkPanel = null, onToggleDetails, onLoadMoreDetails, embedded = false, comparison = null,
  comparisonLoading = false, comparisonError = null, activeStorageName = null,
  selectionMode, selectedResourceIds = new Set(), selectableResourceIds,
  selectionDisabled = false, onToggleResource, onReviewPull, onUnlinkStorage,
  continuityActionBusy = false, rowResolution,
}: ContentProps) {
  const [collapsedCommitShas, setCollapsedCommitShas] = useState<Set<string>>(new Set());
  // Threads that exist only in storage cannot be pushed, so in a Push review the
  // group is context rather than something to act on — it starts folded there and
  // open everywhere else. Null means "follow that default"; toggling pins it.
  const [storageOnlyOverride, setStorageOnlyOverride] = useState<boolean | null>(null);
  const storageOnlyCollapsed = storageOnlyOverride ?? selectionMode === "push";
  const [uncommittedCollapsed, setUncommittedCollapsed] = useState(false);
  const label = projectLabel(project);
  const aliased = label !== project.display_name;
  const threads = useMemo(() => new Map((history?.threads ?? []).map((thread) => [thread.thread_id, thread])), [history?.threads]);
  const syncByThread = useMemo(() => new Map((comparison?.entries ?? []).map((entry) => [entry.thread_id, entry])), [comparison?.entries]);
  const comparisonStorageName = comparison?.storage_name ?? activeStorageName ?? "selected storage";
  const compactSelectionReview = embedded && selectionMode != null;
  const storedOnlyEntries = useMemo(() => (comparison?.entries ?? [])
    .filter((entry) => !entry.local_present && entry.storage_present && !threads.has(entry.thread_id))
    .filter((entry) => !history || entry.storage_updated_at == null || entry.storage_updated_at >= history.window_start),
  [comparison?.entries, history, threads]);
  const olderEntries = useMemo(() => {
    if (!selectionMode) return [];
    const visibleResourceIds = new Set(storedOnlyEntries.map((entry) => entry.resource_id));
    return (comparison?.entries ?? []).filter((entry) => (
      (entry.local_present || entry.storage_present)
      && !threads.has(entry.thread_id)
      && !visibleResourceIds.has(entry.resource_id)
    ));
  }, [comparison?.entries, selectionMode, storedOnlyEntries, threads]);
  const olderDecisionOf = (entry: ThreadSyncEntry) => (
    selectionMode === "pull" ? entry.pull_decision : entry.push_decision
  );
  const olderActionable = olderEntries.filter((entry) => olderDecisionOf(entry)?.selectable);
  const olderInert = olderEntries.filter((entry) => !olderDecisionOf(entry)?.selectable);
  const hasCodexProfile = !!binding?.profile_ids?.codex;
  const embeddedTitle = history?.git ? "Git history" : history ? "Codex threads" : "Activity";
  const embeddedThreadCount = history
    ? new Set([...history.threads.map((thread) => thread.thread_id), ...storedOnlyEntries.map((entry) => entry.thread_id)]).size
    : null;
  const visibleComparisonCounts = useMemo(() => {
    const visibleThreadIds = new Set(threads.keys());
    storedOnlyEntries.forEach((entry) => visibleThreadIds.add(entry.thread_id));
    const counts: Record<ThreadStatusIconName, number> = {
      synced: 0,
      "cloud-outstanding": 0,
      "local-outstanding": 0,
      divergent: 0,
      "divergent-forked": 0,
      pending: 0,
      conflict: 0,
      unavailable: 0,
    };

    for (const entry of comparison?.entries ?? []) {
      if (!visibleThreadIds.has(entry.thread_id)) continue;
      counts[threadSyncPresentation(entry, comparisonStorageName).status] += 1;
    }

    return counts;
  }, [comparison?.entries, comparisonStorageName, storedOnlyEntries, threads]);
  const visibleComparisonChangeCount = visibleComparisonCounts["local-outstanding"]
    + visibleComparisonCounts["cloud-outstanding"]
    + visibleComparisonCounts.divergent
    + visibleComparisonCounts["divergent-forked"]
    + visibleComparisonCounts.pending
    + visibleComparisonCounts.conflict
    + visibleComparisonCounts.unavailable;
  const refreshing = loading || comparisonLoading;
  const refreshStatus = comparisonLoading
    ? `Comparing threads with ${activeStorageName ?? "storage"}`
    : loading
      ? "Refreshing project activity"
      : null;
  const showEmbeddedThreadCount = embedded && !compactSelectionReview && !history?.git && embeddedThreadCount !== null;
  const showComparisonSummary = comparison != null && visibleComparisonChangeCount > 0;
  const renderThread = (thread: CodexThreadSummary, key: string) => {
    const entry = syncByThread.get(thread.thread_id);
    const resourceId = entry?.resource_id ?? `codex:session:${thread.thread_id}`;
    const workingDirectory = thread.working_directory
      || binding?.canonical_project_root
      || binding?.project_root
      || project.project_root
      || "Not configured";
    return (
    <ThreadCard key={key} thread={thread} occurrenceKey={key} workingDirectory={workingDirectory}
      codexHome={history?.codex_home ?? "Not configured"} busy={actionBusyThreadId === thread.thread_id}
      details={detailsByThread[thread.thread_id]} detailsOpen={openDetailOccurrences.has(key)} onOpenCodex={onOpenCodex} onOpenTerminal={onOpenTerminal}
      onFork={onFork} forkBusy={forkBusyThreadId === thread.thread_id}
      onToggleDetails={onToggleDetails} onLoadMoreDetails={onLoadMoreDetails}
      syncEntry={entry} storageName={comparison ? comparisonStorageName : null}
      resourceId={resourceId} selectionMode={selectionMode} selected={selectedResourceIds.has(resourceId)}
      selectable={selectableResourceIds?.has(resourceId) ?? !!selectionMode}
      selectionDisabled={selectionDisabled} onToggleResource={onToggleResource}
      onReviewPull={onReviewPull} rowResolution={rowResolution} />
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
  const toggleCommit = (sha: string) => {
    setCollapsedCommitShas((current) => {
      const next = new Set(current);
      if (next.has(sha)) next.delete(sha);
      else next.add(sha);
      return next;
    });
  };
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
        className={embedded ? "sync-history-content" : "profile-links-section"}
        aria-labelledby={compactSelectionReview ? undefined : "project-activity-heading"}
        aria-label={compactSelectionReview ? embeddedTitle : undefined}
      >
        {!compactSelectionReview && <header className="profile-links-heading sync-history-header">
          <div className="profile-links-copy">
            {embedded ? (
              <h2 id="project-activity-heading" className="sync-history-embedded-title">
                <Icon
                  name={history?.git ? "git-branch" : history ? "openai" : "activity"}
                  size={15}
                  className={!history?.git && history ? "sync-openai-icon" : undefined}
                />
                {embeddedTitle}
              </h2>
            ) : (
              <h1 id="project-activity-heading" className="settings-section-title">{label}</h1>
            )}
          </div>
          <div className="sync-history-toolbar">
            {activeStorageName && !compactSelectionReview && (
              <span
                className="sync-history-storage-lens"
                title={`Comparing threads with ${activeStorageName}`}
                aria-label={`Comparing threads with ${activeStorageName}`}
              >
                <Icon name="drive" size={13} />
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
              <span className="sync-history-toolbar-stats">
                {showEmbeddedThreadCount && (
                  <span
                    className="sync-history-heading-count sync-history-toolbar-count"
                    title={`${embeddedThreadCount} thread${embeddedThreadCount === 1 ? "" : "s"} shown`}
                    aria-label={`${embeddedThreadCount} thread${embeddedThreadCount === 1 ? "" : "s"}`}
                  >
                    <Icon name="message" size={12} />{embeddedThreadCount}
                  </span>
                )}
                {showComparisonSummary && (
                  <span className="sync-thread-sync-summary" aria-label={`Visible thread comparison with ${comparisonStorageName}`}>
                    {visibleComparisonCounts["local-outstanding"] > 0 && <span className="local" title={`${visibleComparisonCounts["local-outstanding"]} local thread change${visibleComparisonCounts["local-outstanding"] === 1 ? "" : "s"}`}><ThreadStatusIcon status="local-outstanding" size={12} />{visibleComparisonCounts["local-outstanding"]}</span>}
                    {visibleComparisonCounts["cloud-outstanding"] > 0 && <span className="storage" title={`${visibleComparisonCounts["cloud-outstanding"]} thread change${visibleComparisonCounts["cloud-outstanding"] === 1 ? "" : "s"} in ${comparisonStorageName}`}><ThreadStatusIcon status="cloud-outstanding" size={12} />{visibleComparisonCounts["cloud-outstanding"]}</span>}
                    {visibleComparisonCounts.divergent > 0 && <span className="diverged" title={`${visibleComparisonCounts.divergent} divergent thread${visibleComparisonCounts.divergent === 1 ? "" : "s"}`}><ThreadStatusIcon status="divergent" size={12} />{visibleComparisonCounts.divergent}</span>}
                    {visibleComparisonCounts["divergent-forked"] > 0 && <span className="diverged-forked" title={`${visibleComparisonCounts["divergent-forked"]} divergent thread${visibleComparisonCounts["divergent-forked"] === 1 ? "" : "s"} already preserved as a fork`}><ThreadStatusIcon status="divergent-forked" size={12} />{visibleComparisonCounts["divergent-forked"]}</span>}
                    {visibleComparisonCounts.pending > 0 && <span className="pending" title={`${visibleComparisonCounts.pending} thread${visibleComparisonCounts.pending === 1 ? " is" : "s are"} waiting to sync`}><ThreadStatusIcon status="pending" size={12} />{visibleComparisonCounts.pending}</span>}
                    {visibleComparisonCounts.conflict > 0 && <span className="conflict" title={`${visibleComparisonCounts.conflict} thread conflict${visibleComparisonCounts.conflict === 1 ? "" : "s"} need attention`}><ThreadStatusIcon status="conflict" size={12} />{visibleComparisonCounts.conflict}</span>}
                    {visibleComparisonCounts.unavailable > 0 && <span className="unavailable" title={`${visibleComparisonCounts.unavailable} thread${visibleComparisonCounts.unavailable === 1 ? " is" : "s are"} unavailable for sync`}><ThreadStatusIcon status="unavailable" size={12} />{visibleComparisonCounts.unavailable}</span>}
                  </span>
                )}
              </span>
            )}
            {comparisonError && <span className="sync-thread-sync-error" tabIndex={0} data-tooltip={comparisonError} aria-label={`Storage comparison failed: ${comparisonError}`}><ThreadStatusIcon status="conflict" size={24} /></span>}
            <button
              type="button"
              className={`sync-history-icon-action sync-history-refresh${refreshing ? " loading" : ""}`}
              onClick={onRefresh}
              disabled={refreshing}
              aria-busy={refreshing}
              title={refreshing ? "Refreshing activity" : "Refresh activity"}
              aria-label={refreshing ? "Refreshing activity" : "Refresh activity"}
            >
              <span
                aria-hidden="true"
                className={`icon-spin-frame${refreshing ? " spinning" : ""}`}
              >
                <Icon name="refresh" size={16} />
              </span>
            </button>
            {refreshStatus && <span className="sync-visually-hidden" role="status">{refreshStatus}</span>}
          </div>
        </header>}

        {actionError && <div className="sync-callout error sync-history-error" role="alert"><Icon name="alert-triangle" size={15} /><span>{actionError}</span></div>}
        {forkPanel}
        {comparison?.remote_continuity?.state === "replaced" && (
          <div className="sync-callout warning sync-history-continuity" role="status">
            <Icon name="alert-triangle" size={15} />
            <div className="sync-history-continuity-copy">
              <strong>Storage history changed</strong>
              <span>
                {`The sync history in ${comparisonStorageName} is different from the one Mallard last synced for this project (revision ${comparison.remote_continuity.base_revision}). This usually means Mallard's sync data in that storage was deleted and created again. The session statuses below use the replacement. Unlink this storage and link it again to start fresh. Unlinking does not delete files, but anything saved only in the previous storage history cannot be recovered from this machine.`}
              </span>
            </div>
            {onUnlinkStorage && (
              <div className="sync-sync-review-recovery-actions">
                <button type="button" className="btn btn-ghost danger" onClick={onUnlinkStorage} disabled={continuityActionBusy}>
                  Unlink storage
                </button>
              </div>
            )}
          </div>
        )}
        {!hasCodexProfile ? (
          <div className="sync-history-state sync-history-profile-state"><Icon name="alert-triangle" size={18} /><div><strong>No Codex profile is connected.</strong><span>Remove and add the project again to choose a Codex profile.</span></div></div>
        ) : loading && !history ? <div className="sync-history-state" role="status" aria-live="polite"><span className="status-loader" /> Loading project activity…</div> : !history ? null : (
          <>
            {!embedded && (
              <section className="sync-history-project-context" aria-label="Project information">
                <span className="sync-history-context-item" title={`Project directory: ${binding?.canonical_project_root ?? binding?.project_root ?? project.project_root ?? "Not configured"}`}>
                  <Icon name="folder" size={14} /><span>{compactProjectPath(binding?.canonical_project_root ?? binding?.project_root ?? project.project_root ?? "Not configured")}</span>
                </span>
                <span className="sync-history-context-item" title={`Codex configuration: ${history.codex_home}`}>
                  <Icon name="terminal" size={14} /><span>{compactProjectPath(history.codex_home)}</span>
                </span>
                {aliased && <span className="sync-history-context-item" title={`Repository: ${project.display_name}`}><Icon name="git-branch" size={14} /><span className="sync-history-context-name">{project.display_name}</span></span>}
                {history.storage_sync.length ? history.storage_sync.map((storage) => (
                  <span key={storage.storage_id} className="sync-history-context-item sync-history-storage-item" title={`Storage: ${storage.storage_name}`}>
                    <Icon name="cloud" size={14} /><strong>{storage.storage_name}</strong>
                    <span className={storage.last_pull_at ? "recorded" : undefined} title={`Last pull: ${formatDate(storage.last_pull_at)}`} aria-label={`Last pull: ${formatDate(storage.last_pull_at)}`}><Icon name="download" size={13} /></span>
                    <span className={storage.last_push_at ? "recorded" : undefined} title={`Last push: ${formatDate(storage.last_push_at)}`} aria-label={`Last push: ${formatDate(storage.last_push_at)}`}><Icon name="upload" size={13} /></span>
                  </span>
                )) : <span className="sync-history-context-item muted" title="No storage linked"><Icon name="cloud" size={14} /><span>No storage</span></span>}
              </section>
            )}

            {history.git ? (
              <>
                {storedOnlyEntries.length > 0 && (
                  <section className={`sync-history-storage-only${storageOnlyCollapsed ? " collapsed" : ""}`} aria-labelledby="storage-only-heading">
                    <SectionFold
                      collapsed={storageOnlyCollapsed}
                      controls="storage-only-list"
                      label={`${storageOnlyCollapsed ? "Expand" : "Collapse"} ${storedOnlyEntries.length} thread${storedOnlyEntries.length === 1 ? "" : "s"} only in ${comparisonStorageName}`}
                      onToggle={() => setStorageOnlyOverride(!storageOnlyCollapsed)}
                    />
                    <div className="sync-history-section-heading">
                      <h2 id="storage-only-heading"><Icon name="cloud" size={13} />Only in <span className="sync-history-storage-name">{comparisonStorageName}</span></h2>
                    </div>
                    {!storageOnlyCollapsed && (
                      <div id="storage-only-list" className="sync-history-thread-list flat">
                        {storedOnlyEntries.map((entry) => <SyncEntryCard key={`stored:${entry.resource_id}`} entry={entry} storageName={comparisonStorageName} scopeStated
                          selectionMode={selectionMode} selected={selectedResourceIds.has(entry.resource_id)}
                          selectable={selectableResourceIds?.has(entry.resource_id) ?? !!selectionMode}
                          selectionDisabled={selectionDisabled} onToggleResource={onToggleResource}
                          onReviewPull={onReviewPull} />)}
                      </div>
                    )}
                  </section>
                )}
                <div className={`sync-history-timeline${history.unmapped.length > 0 ? " has-uncommitted" : ""}`}>
                  {history.unmapped.length > 0 && (
                    <section className={`sync-history-uncommitted${uncommittedCollapsed ? " collapsed" : ""}`} aria-labelledby="uncommitted-heading">
                      <SectionFold
                        collapsed={uncommittedCollapsed}
                        controls="uncommitted-list"
                        label={`${uncommittedCollapsed ? "Expand" : "Collapse"} ${history.unmapped.length} session${history.unmapped.length === 1 ? "" : "s"} not linked to a commit`}
                        onToggle={() => setUncommittedCollapsed(!uncommittedCollapsed)}
                      />
                      <div className="sync-history-section-heading">
                        <h2 id="uncommitted-heading"><Icon name="edit" size={13} />Uncommitted</h2>
                      </div>
                      {!uncommittedCollapsed && (
                        <div id="uncommitted-list" className="sync-history-thread-list flat">{history.unmapped.map((reference) => { const thread = threads.get(reference.thread_id); return thread ? renderThread(thread, `uncommitted:${thread.thread_id}`) : null; })}</div>
                      )}
                    </section>
                  )}
                  <section className="sync-history-commit-section" aria-label={`First-parent commits on ${history.git.selected_branch}`}>
                    {!embedded && <div className="sync-history-section-heading"><h2>Commit history</h2><span className="sync-history-heading-count" title={`${history.git.unique_thread_count} session${history.git.unique_thread_count === 1 ? "" : "s"} linked to commits`} aria-label={`${history.git.unique_thread_count} session${history.git.unique_thread_count === 1 ? "" : "s"} linked to commits`}><Icon name="message" size={12} />{history.git.unique_thread_count}</span></div>}
                    {history.git.commits.length === 0 ? <div className="sync-history-state">No commits are available in this 30-day window.</div> : (
                      <ol className="sync-history-commit-rail">{history.git.commits.map((commit) => {
                        const commitUrl = githubCommitUrl(history.git?.github_repository_url, commit.sha);
                        const references = orderedReferences(commit.thread_refs);
                        const collapsed = collapsedCommitShas.has(commit.sha);
                        const displaySha = commit.short_sha.slice(0, 7);
                        const threadListId = `commit-threads-${commit.short_sha}`;
                        return (
                          <li key={commit.sha} className={`sync-history-commit${collapsed ? " collapsed" : ""}`}>
                            <SectionFold
                              collapsed={collapsed}
                              controls={threadListId}
                              label={`${collapsed ? "Expand" : "Collapse"} commit ${displaySha}`}
                              disabled={references.length === 0}
                              onToggle={() => toggleCommit(commit.sha)}
                            />
                            <div className="sync-history-commit-heading">
                              <span className="sync-history-commit-title">
                                <span className="sync-history-commit-icon" aria-hidden="true">
                                  <Icon name="git-commit" size={14} />
                                </span>
                                <strong title={commit.subject}>{commit.subject}</strong>
                              </span>
                              {/* Identifier first, stamp last: the stamp closes the row
                                  at the card edge and forms one column with the session
                                  stamps below. The external-link glyph marks the chip as
                                  an off-app target, so it exists only when the remote
                                  does — an unlinked sha is bare mono, not a chip. */}
                              <span className="sync-history-commit-meta">
                                {commitUrl ? (
                                  <a className="sync-history-commit-link" href={commitUrl} target="_blank" rel="noopener noreferrer"
                                    title={`Open ${commit.sha} on GitHub`} aria-label={`Open commit ${displaySha} on GitHub`}
                                    onClick={(event) => { event.preventDefault(); void openExternalUrl(commitUrl); }}>
                                    <Icon name="external-link" size={12} />
                                    <code>{displaySha}</code>
                                  </a>
                                ) : <code title={commit.sha}>{displaySha}</code>}
                                <time dateTime={new Date(commit.committed_at * 1_000).toISOString()}>{formatDate(commit.committed_at)}</time>
                              </span>
                            </div>
                            {!collapsed && references.length > 0 && (
                              <div id={threadListId} className="sync-history-thread-list">
                                {references.map((reference) => {
                                  const thread = threads.get(reference.thread_id);
                                  return thread ? renderThread(thread, `${commit.sha}:${thread.thread_id}`) : null;
                                })}
                              </div>
                            )}
                          </li>
                        );
                      })}</ol>
                    )}
                  </section>
                </div>
              </>
            ) : (
              <section
                aria-labelledby={embedded && !compactSelectionReview ? "project-activity-heading" : !embedded ? "codex-sessions-heading" : undefined}
                aria-label={compactSelectionReview ? embeddedTitle : undefined}
              >
                {!embedded && <div className="sync-history-section-heading"><h2 id="codex-sessions-heading" className="sync-history-codex-heading"><Icon name="openai" size={14} className="sync-openai-icon" />Codex threads</h2><span className="sync-history-heading-count" title={`${history.threads.length} threads`}><Icon name="message" size={12} />{history.threads.length}</span></div>}
                <div className="sync-history-thread-list flat">{flatThreadRows.length ? flatThreadRows.map((row) => row.kind === "local"
                  ? renderThread(row.thread, row.thread.thread_id)
                  : <SyncEntryCard key={`stored:${row.entry.resource_id}`} entry={row.entry} storageName={comparisonStorageName}
                    selectionMode={selectionMode} selected={selectedResourceIds.has(row.entry.resource_id)}
                    selectable={selectableResourceIds?.has(row.entry.resource_id) ?? !!selectionMode}
                    selectionDisabled={selectionDisabled} onToggleResource={onToggleResource}
                    onReviewPull={onReviewPull} />)
                  : <div className="sync-history-state">No project-owned Codex sessions were found in this 30-day window.</div>}</div>
              </section>
            )}
            {selectionMode && olderEntries.length > 0 && (
              <section className="sync-history-older" aria-label="Older activity">
                <details className="sync-history-older-group">
                  <summary>
                    <span className="sync-history-older-title">Older activity</span>
                    <span
                      className="sync-history-heading-count"
                      title={`${olderEntries.length} thread${olderEntries.length === 1 ? "" : "s"} outside this 30-day window`}
                    >
                      <Icon name="message" size={12} />{olderEntries.length}
                    </span>
                  </summary>
                  <div className="sync-history-thread-list flat">
                    {olderActionable.map((entry) => (
                      <SyncEntryCard key={`older:${entry.resource_id}`} entry={entry} storageName={comparisonStorageName}
                        selectionMode={selectionMode} selected={selectedResourceIds.has(entry.resource_id)}
                        selectable={selectableResourceIds?.has(entry.resource_id) ?? !!selectionMode}
                        selectionDisabled={selectionDisabled} onToggleResource={onToggleResource}
                        onReviewPull={onReviewPull} />
                    ))}
                    {olderInert.length > 0 && (
                      <details className="sync-history-older-inert">
                        <summary>
                          {olderInert.length === 1
                            ? "1 thread needs no changes or can't sync"
                            : `${olderInert.length} threads need no changes or can't sync`}
                        </summary>
                        <div className="sync-history-thread-list flat">
                          {olderInert.map((entry) => (
                            <SyncEntryCard key={`older:${entry.resource_id}`} entry={entry} storageName={comparisonStorageName}
                              selectionMode={selectionMode} onReviewPull={onReviewPull} />
                          ))}
                        </div>
                      </details>
                    )}
                  </div>
                </details>
              </section>
            )}
            {history.next_before != null && <button type="button" className="btn sync-history-load-more" disabled={loadingMore} onClick={onLoadMore}>{loadingMore ? "Loading…" : "Load previous 30 days"}</button>}
          </>
        )}
      </section>
  );

  if (embedded) {
    return <div className={`sync-history-page sync-history-embedded${compactSelectionReview ? " sync-history-selection-review" : ""}`}>{content}</div>;
  }
  return <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">{content}</main>;
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
  const unique = new Set(mergedCommits.flatMap((commit) => commit.thread_refs.map((reference) => reference.thread_id))).size;
  return { ...next, threads: [...threads.values()], unmapped: [...unmapped.values()], git: { ...next.git, commits: mergedCommits, unique_thread_count: unique } };
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
  onReviewPull,
  onUnlinkStorage,
  continuityActionBusy = false,
  comparisonOverride,
  rowResolution,
  onReviewListReadyChange,
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
  const [forkBusyThreadId, setForkBusyThreadId] = useState<string | null>(null);
  const [forkCheckpoint, setForkCheckpoint] = useState<CodexForkCheckpoint | null>(null);
  const [forkNotice, setForkNotice] = useState<ForkCompletionFeedback | null>(null);
  const [forkResolving, setForkResolving] = useState(false);
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

  const reviewListReady = history !== null && !loading && comparison !== null && !comparisonLoading;
  useEffect(() => {
    onReviewListReadyChange?.(reviewListReady);
  }, [reviewListReady, onReviewListReadyChange]);

  // Rehydrate a pending Activity-page fork checkpoint (for example after an app
  // restart mid-fork). Pull-origin checkpoints stay with the Pull review.
  useEffect(() => {
    let cancelled = false;
    setForkCheckpoint(null);
    setForkNotice(null);
    if (!binding?.profile_ids?.codex) return;
    void (async () => {
      try {
        const pending = await projectSyncApi.getPendingCodexDivergence(project.local_project_id);
        if (cancelled) return;
        const activityCheckpoint = pending?.codex_fork_checkpoints.find(
          (checkpoint) => checkpoint.origin === "manual",
        ) ?? null;
        setForkCheckpoint(activityCheckpoint);
      } catch {
        // Rehydration is best-effort; the checkpoint re-surfaces on the next
        // fork action or refresh.
      }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.local_project_id, binding?.revision, refreshEpoch]);

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
  const refreshActivity = async (dismissForkFeedback: boolean) => {
    await refreshActivityAndHandleForkFeedback(
      () => load(null, branch, true),
      loadComparison,
      () => setForkNotice(null),
      dismissForkFeedback,
    );
  };
  const consumeForkResult = (result: RestoreResult) => {
    const activityCheckpoint = result.codex_fork_checkpoints.find(
      (checkpoint) => checkpoint.origin === "manual",
    ) ?? null;
    setForkCheckpoint(result.status === "complete" ? null : activityCheckpoint);
    setForkNotice({
      message: result.message,
      success: result.success,
      fork: result.codex_manual_fork
        ? {
          name: result.codex_manual_fork.fork_name,
          providerThreadId: result.codex_manual_fork.provider_thread_id,
          createdAt: Math.floor(Date.now() / 1_000),
        }
        : null,
    });
    return result.status === "complete" || activityCheckpoint == null;
  };
  const forkThread = async (threadId: string) => {
    setActionError(null);
    setForkNotice(null);
    setForkBusyThreadId(threadId);
    try {
      const result = await projectSyncApi.forkCodexThread(project.local_project_id, threadId);
      if (consumeForkResult(result)) await refreshActivity(false);
    } catch (reason) { setActionError(errorMessage(reason)); }
    finally { setForkBusyThreadId(null); }
  };
  const retryForkCheckpoint = async () => {
    if (!forkCheckpoint) return;
    setActionError(null);
    setForkNotice(null);
    setForkResolving(true);
    try {
      const result = await projectSyncApi.retryCodexDivergence(forkCheckpoint.operation_id);
      if (consumeForkResult(result)) await refreshActivity(false);
    } catch (reason) { setActionError(errorMessage(reason)); }
    finally { setForkResolving(false); }
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

  const forkPanel = (
    <ForkActivityPanel
      feedback={forkNotice}
      checkpoint={forkCheckpoint}
      selectionMode={selectionMode}
      resolving={forkResolving}
      onRetry={() => void retryForkCheckpoint()}
      onDismissFeedback={() => setForkNotice(null)}
    />
  );
  return <ProjectChatHistoryContent project={project} binding={binding} history={history} loading={loading} loadingMore={loadingMore} actionError={actionError} actionBusyThreadId={actionBusyThreadId} detailsByThread={detailsByThread} openDetailOccurrences={openDetailOccurrences} onBranchChange={changeBranch} onRefresh={() => { void refreshActivity(true); }} onLoadMore={() => void load(history?.next_before ?? null, branch)} onOpenCodex={(id) => void openCodex(id)} onOpenTerminal={(id) => void openTerminal(id)} onFork={(id) => void forkThread(id)} forkBusyThreadId={forkBusyThreadId} forkPanel={forkPanel} onToggleDetails={toggleDetails} onLoadMoreDetails={(id) => void loadDetails(id, detailsByThread[id]?.page?.next_cursor)} embedded={embedded} comparison={comparison} comparisonLoading={comparisonLoading} comparisonError={comparisonError} activeStorageName={activeStorageName} selectionMode={selectionMode} selectedResourceIds={selectedResourceIds} selectableResourceIds={selectableResourceIds} selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} onReviewPull={onReviewPull} onUnlinkStorage={onUnlinkStorage} continuityActionBusy={continuityActionBusy} rowResolution={rowResolution} />;
}
