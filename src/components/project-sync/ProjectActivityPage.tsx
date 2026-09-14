import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type {
  GitBranchSummary,
  LocalProjectSummary,
  NoticeSeverity,
  OlderActivity,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionSummary,
  ProviderActivityState,
  SessionActionDescriptor,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../types";
import Icon, { type IconName } from "../Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { formatDuration, formatFullDateTime } from "../../clock";
import { formatBytes } from "../../format";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import { githubCommitUrl, openExternalUrl } from "./externalLink";
import LoadingScaffold from "../LoadingScaffold";
import { projectLabel } from "./model";
import ProviderMark from "../sync-review/ProviderMark";
import { writeClipboardText } from "../sync-review/clipboard";
import TopLayer from "../sync-review/TopLayer";
import { useDebugPopoverPlacement } from "../sync-review/debugPopoverPlacement";
import {
  isPullReviewRow,
  isPushReviewRow,
} from "../sync-review/pullReviewStatus";
import { formatCompactNumber, formatSessionDetailTimestamp, type OpenSessionDetailRequest } from "./session-detail/sessionDetailModel";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../sync-review/ThreadStatusIcon";

/**
 * What the workspace path bar renders on this page's behalf.
 *
 * The list owns the data — branches arrive with the activity page, counts with
 * the storage comparison — but the controls belong in the bar, beside the Push
 * and Pull buttons the numbers describe. One backend read projects both, and
 * the page hands the bar a model plus the handlers that drive it.
 */
export interface ActivityHeaderModel {
  branches: GitBranchSummary[];
  selectedBranch: string;
  onSelectBranch: (branch: string) => void;
  counts: ActivityHeaderCounts;
  /** The whole comparison failed, as against one provider's failing. */
  comparisonError: string | null;
  refreshing: boolean;
  onRefresh: () => void;
}

interface PageProps {
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  refreshEpoch: number;
  activeStorageId?: string | null;
  activeStorageName?: string | null;
  selectionMode?: "push" | "pull";
  selectedResourceIds?: ReadonlySet<string>;
  selectableResourceIds?: ReadonlySet<string>;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  /** Drop selected rows hidden by Show less because they cannot be reviewed. */
  onDropSelection?: (resourceIds: string[]) => void;
  onReviewPull?: () => void;
  onUnlinkStorage?: () => void;
  onChooseAgentProfile?: () => void;
  continuityActionBusy?: boolean;
  comparisonOverride?: ThreadSyncComparison | null;
  onOpenSessionDetail?: (
    request: OpenSessionDetailRequest,
    trigger: HTMLButtonElement,
  ) => void;
  /**
   * Fires with whether the review list is trustworthy: the timeline needs
   * `activity` and the storage rows need the comparison, projected from one
   * asynchronous backend read. A hosting review gates its primary action so
   * nothing is applied sight unseen.
   */
  onReviewListReadyChange?: (ready: boolean) => void;
  /**
   * Hands the workspace path bar what it needs to speak for this page, and
   * null when there is nothing to say. Only the workspace passes it; the two
   * review hosts render this page inside their own chrome.
   */
  onHeaderModelChange?: (model: ActivityHeaderModel | null) => void;
}

interface ContentProps {
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  activity: ProjectActivity | null;
  loading: boolean;
  loadingMore: boolean;
  actionError: string | null;
  /** Page-level load failure — kept apart from actionError so a failed row
      action is never mistaken for an empty page and vice versa. */
  loadError: string | null;
  onRetryLoad: () => void;
  actionBusyResourceId: string | null;
  actionBusyActionId?: string | null;
  onLoadMore: () => void;
  onShowLess: () => void;
  /** True once more than one window is loaded — there is something to pop. */
  canShowLess: boolean;
  /** Dispatches one backend-declared row action. */
  onSessionAction: (session: ProjectSessionSummary, action: SessionActionDescriptor) => void;
  onOpenSessionDetail?: (
    request: OpenSessionDetailRequest,
    trigger: HTMLButtonElement,
  ) => void;
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
  onChooseAgentProfile?: () => void;
  continuityActionBusy?: boolean;
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
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
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

  // The whole pill — icon, name, and chevron — is one path-bar target.
  const label = `Branch: ${selected.name}${selected.is_current ? ", current branch" : ""}`;
  const face = (
    <>
      <Icon name="git-branch" size={12} />
      {/* Just the branch name: the pill always shows the selected branch, so
          "Current" belongs in the open menu, not here. */}
      <span className="sync-history-branch-name" title={selected.name}>{selected.name}</span>
    </>
  );

  return (
    <div ref={rootRef} className={`sync-history-branch-select${open ? " open" : ""}`}>
      <button
        ref={triggerRef}
        type="button"
        className="sync-history-branch-trigger"
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={menuId}
        aria-label={label}
        title="Choose a branch"
        onClick={() => open ? closeMenu() : openMenu()}
        onKeyDown={(event) => {
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
          event.preventDefault();
          openMenu(event.key === "ArrowDown" ? "first" : "last");
        }}
      >
        {face}
        <Icon name="chevron-down" size={11} className="sync-history-branch-chevron" />
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
          <span>View branch</span>
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
              title={branch.name}
              onClick={() => {
                closeMenu(true);
                if (branch.name !== selectedBranch) onChange(branch.name);
              }}
            >
              <span className="sync-history-branch-check" aria-hidden="true">
                {branch.name === selectedBranch && <Icon name="check-circle" size={12} />}
              </span>
              <span className="sync-history-branch-option-copy">
                <strong>{branch.name}</strong>
              </span>
              {branch.is_current && <span className="sync-history-branch-state current">Current</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function formatDate(value?: number | null): string {
  if (!value) return "Not recorded";
  // Shared with the session detail page, so a row and the page it opens
  // stamp the same moment the same way.
  return formatSessionDetailTimestamp(value);
}

function formatCount(value?: number | null): string {
  if (value == null) return "Not reported";
  return formatCompactNumber(value);
}

/* Day precision, for the two places that name a boundary rather than a moment:
   the window a page covers, and the day a step backwards lands on. A clock
   time on either would claim the edge falls at that exact second. */
let dayFormatter: Intl.DateTimeFormat | null = null;
function formatDay(seconds: number): string {
  dayFormatter ??= new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  return dayFormatter.format(new Date(seconds * 1_000));
}

/* `formatRange` collapses the parts two dates share — "Jul 28 – Aug 27, 2026"
   rather than the year twice — and does it per locale, which hand-rolling
   cannot. It postdates this project's TS lib, so it is reached through a
   capability check with the spelled-out form as the fallback. */
interface RangeCapableFormat extends Intl.DateTimeFormat {
  formatRange?: (from: Date, to: Date) => string;
}

/** The window a page covers, as one range — "Jul 28 – Aug 27, 2026". */
function formatDayRange(fromSeconds: number, toSeconds: number): string {
  dayFormatter ??= new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  const formatter = dayFormatter as RangeCapableFormat;
  const from = new Date(fromSeconds * 1_000);
  // The window is half-open, so its last covered day is the second before it
  // ends; formatting `window_end` itself would name a day the page excludes.
  const to = new Date(Math.max(fromSeconds, toSeconds - 1) * 1_000);
  return formatter.formatRange
    ? formatter.formatRange(from, to)
    : `${formatter.format(from)} – ${formatter.format(to)}`;
}

/** "41 commits and 6 sessions", dropping whichever side is empty. */
function describeOlder(older: OlderActivity): string {
  const parts: string[] = [];
  if (older.commits > 0) parts.push(`${older.commits} commit${older.commits === 1 ? "" : "s"}`);
  if (older.sessions > 0) parts.push(`${older.sessions} session${older.sessions === 1 ? "" : "s"}`);
  const through = `through ${formatDay(older.newest_at)}`;
  return parts.length ? `Load earlier history — ${through}, ${parts.join(" and ")}` : `Load earlier history — ${through}`;
}

/**
 * The rail's foot: whichever of the two paging controls apply, under the rail
 * they page and in its own column.
 *
 * Load earlier states the day one press lands on, which is the point of a
 * cursor that tracks the data instead of stepping a fixed thirty days into
 * whatever is there. Show less pops the newest window back off.
 *
 * While a fetch runs only the node moves. It is a 24px slot outside the row,
 * so the labels and the date hold their positions; the date holds because the
 * fetch is still running *through* that day.
 */
function RailFoot({ older, canShowLess, busy, railed, onLoadMore, onShowLess }: {
  older: OlderActivity | null;
  canShowLess: boolean;
  busy: boolean;
  /** False in the flat Sessions list of a project with no repository, which
   *  has no rail to hang a node on. */
  railed: boolean;
  onLoadMore: () => void;
  onShowLess: () => void;
}) {
  if (!older && !canShowLess) return null;
  const loadLabel = older ? describeOlder(older) : "";
  return (
    <div className={`sync-history-rail-foot${railed ? "" : " no-rail"}`}>
      <span className="sync-history-rail-foot-node" aria-hidden="true">
        <Icon name={busy ? "refresh" : "chevron-down"} size={12} className={busy ? "icon-spin" : undefined} />
      </span>
      {canShowLess && (
        <button
          type="button"
          className="sync-history-rail-step"
          disabled={busy}
          title="Drop the most recently loaded window"
          onClick={onShowLess}
        >
          <strong>Show less</strong>
        </button>
      )}
      {older && (
        <button
          type="button"
          className="sync-history-rail-step"
          disabled={busy}
          aria-busy={busy || undefined}
          title={loadLabel}
          aria-label={loadLabel}
          onClick={onLoadMore}
        >
          <strong>{busy ? "Loading…" : "Load earlier"}</strong>
          <time dateTime={new Date(older.newest_at * 1_000).toISOString()}>· {formatDay(older.newest_at)}</time>
        </button>
      )}
    </div>
  );
}

export function compactThreadTitle(value: string): string {
  return value.replace(
    /\s*·\s*\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}(?::\d{2})?Z?)?\s*$/,
    "",
  );
}

type SessionSyncPresentation = {
  status: ThreadStatusIconName;
  className: string;
  label: string;
};

/** The verdicts a real comparison entry can reach. The three neutral badges are
 *  deliberately not among them: those say "no verdict", which is a statement
 *  about the comparison, not about the thread. Keeping them out of this union
 *  is what stops them being counted in the header tally. */
type ComparisonStatus = Exclude<ThreadStatusIconName, "untracked" | "checking" | "unknown">;
type ComparisonPresentation = SessionSyncPresentation & { status: ComparisonStatus };

type SessionSyncIndex = Map<string, ThreadSyncEntry>;

/** Activity and comparison expose the same portable conversation identity.
 *  The comparison's capture resource ID remains reserved for plan selection. */
function indexSessionSyncEntries(entries: readonly ThreadSyncEntry[]): SessionSyncIndex {
  const index: SessionSyncIndex = new Map();
  for (const entry of entries) {
    index.set(entry.conversation_resource_id, entry);
  }
  return index;
}

function syncEntryForSession(
  session: ProjectSessionSummary,
  index: SessionSyncIndex,
): ThreadSyncEntry | undefined {
  return index.get(session.resource_id);
}

export function sessionSyncPresentation(
  entry: ThreadSyncEntry,
  storageName: string,
): ComparisonPresentation {
  type Blocker = NonNullable<ThreadSyncEntry["push_decision"]["blocker"]>;
  const decisionWithBlocker = (blockers: readonly Blocker[]) => {
    if (entry.push_decision?.blocker && blockers.includes(entry.push_decision.blocker)) {
      return { mode: "push" as const };
    }
    if (entry.pull_decision?.blocker && blockers.includes(entry.pull_decision.blocker)) {
      return { mode: "pull" as const };
    }
    return null;
  };
  const pending = decisionWithBlocker(["busy"]);
  if (pending) {
    return {
      status: "pending",
      className: "pending",
      label: syncHoldDetail(pending.mode, entry)
        ?? entry.status_detail
        ?? "Sync is waiting for this session to become ready.",
    };
  }
  const conflict = decisionWithBlocker([
    "unknown_evidence",
    "ambiguous_binding",
    "missing_binding",
    "detached_requires_pull",
  ]);
  if (conflict) {
    return {
      status: "conflict",
      className: "conflict",
      label: syncHoldDetail(conflict.mode, entry)
        ?? entry.status_detail
        ?? "Mallard could not safely match the local and stored session.",
    };
  }
  const blocked = (mode: "push" | "pull"): ComparisonPresentation => ({
    status: "unavailable",
    className: "unavailable",
    label: syncHoldDetail(mode, entry) ?? "This thread can't be synced safely right now.",
  });
  switch (entry.state) {
    case "synced":
      /* Current on both sides, but exported under settings that have since
         changed (redaction, child sessions): the storage holds the earlier
         copy until the next Push replaces it, so this is Push work and
         counts as such — a synced check here would hide it. */
      if (entry.push_decision?.action === "publish") {
        return {
          status: "local-outstanding",
          className: "local",
          label: `Pushed with your previous export settings. Push to update ${storageName}.`,
        };
      }
      return { status: "synced", className: "synced", label: entry.status_detail ?? `Up to date here and in ${storageName}.` };
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

/** What the path bar says on the activity list's behalf: two numbers that ride
 *  the buttons resolving them, and one for everything neither button can.
 *
 *  The tallies run over the whole comparison, not the rows on screen. Push and
 *  Pull act on the project, so a badge scoped to the visible 30-day window
 *  would promise a number the very next click disproves. */
export interface ActivityHeaderCounts {
  /** Newer here than in storage — what Push sends. */
  push: number;
  /** Newer in storage than here — what Pull fetches. */
  pull: number;
  /** Divergent, held, conflicting or unsyncable: needs a person, not a verb. */
  attention: number;
  /** Conflicts and unsyncable threads read louder than a review or a hold. */
  severity: "warning" | "danger";
  /** The breakdown the single chip stands in for. */
  detail: string;
}

function countPhrase(count: number, singular: string, plural: string): string | null {
  if (count === 0) return null;
  return `${count} ${count === 1 ? singular : plural}`;
}

export function activityHeaderCounts(
  comparison: ThreadSyncComparison | null,
  storageName: string,
): ActivityHeaderCounts {
  let push = 0;
  let pull = 0;
  let review = 0;
  let waiting = 0;
  let conflict = 0;
  let unsyncable = 0;

  for (const entry of comparison?.entries ?? []) {
    // An entry present on neither side is a ghost of a thread that has been
    // removed from both; it is nobody's work.
    if (!entry.local_present && !entry.storage_present) continue;
    switch (sessionSyncPresentation(entry, storageName).status) {
      case "local-outstanding": push += 1; break;
      case "cloud-outstanding": pull += 1; break;
      case "divergent": review += 1; break;
      case "pending": waiting += 1; break;
      case "conflict": conflict += 1; break;
      case "unavailable": unsyncable += 1; break;
      default: break;
    }
  }

  const detail = [
    countPhrase(review, "thread changed on both sides", "threads changed on both sides"),
    countPhrase(waiting, "thread waiting to sync", "threads waiting to sync"),
    countPhrase(conflict, "thread Mallard could not match", "threads Mallard could not match"),
    countPhrase(unsyncable, "thread that can't be synced", "threads that can't be synced"),
  ].filter((phrase): phrase is string => phrase !== null).join(", ");

  return {
    push,
    pull,
    attention: review + waiting + conflict + unsyncable,
    severity: conflict > 0 || unsyncable > 0 ? "danger" : "warning",
    detail,
  };
}

/** The badge, from a presentation the caller has already resolved. Every row
 *  renders one: the slot is a grid column, and a column that sometimes
 *  holds nothing is a column that moves every title beside it. */
function SessionSyncIndicator({ presentation }: { presentation: SessionSyncPresentation }) {
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

/** Named badge state for every row without a comparison entry. */
export function unresolvedSyncPresentation({
  comparisonLoaded,
  comparisonLoading,
  comparisonError,
  storageName,
  hasBoundProvider,
}: {
  comparisonLoaded: boolean;
  comparisonLoading: boolean;
  comparisonError: string | null;
  storageName: string | null;
  hasBoundProvider: boolean;
}): SessionSyncPresentation {
  if (comparisonLoading) {
    return {
      status: "checking",
      className: "checking",
      label: `Comparing with ${storageName ?? "storage"}…`,
    };
  }
  if (comparisonError) {
    return {
      status: "unknown",
      className: "unknown",
      label: `Not compared. ${comparisonError}`,
    };
  }
  if (!storageName) {
    return {
      status: "untracked",
      className: "untracked",
      label: "Only on this Mac. Link a storage to compare and sync it.",
    };
  }
  if (!hasBoundProvider) {
    return {
      status: "untracked",
      className: "untracked",
      label: `Only on this Mac. Connect an agent profile to compare with ${storageName}.`,
    };
  }
  // A comparison exists and this row is not in it — a view-only session, or one
  // this Mallard version has no adapter for.
  return {
    status: "unknown",
    className: "unknown",
    label: comparisonLoaded
      ? `Not compared with ${storageName}. Mallard cannot read this session well enough to match it.`
      : `Not compared with ${storageName} yet.`,
  };
}

/* One absolute stamp, pushed to the far edge so it forms a column down the
   list. It anchors on the last activity because that is the more useful end of
   the interval; the start is that minus the duration sitting beside the other
   readouts. Both ends are spelled out in full in the label. */
function SessionTimestamp({ session }: { session: ProjectSessionSummary }) {
  const startedLabel = `Started ${formatFullDateTime(session.started_at)}`;
  const endedLabel = `${session.is_active ? "Last activity" : "Ended"} ${formatFullDateTime(session.ended_at)}${session.is_active ? ", active" : ""}`;
  return (
    <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`${startedLabel}\n${endedLabel}`} aria-label={`${startedLabel}. ${endedLabel}`} tabIndex={0}>
      <time dateTime={new Date(session.ended_at * 1_000).toISOString()}>{formatFullDateTime(session.ended_at)}</time>
    </span>
  );
}

/** Readouts only — everything in here is hovered, nothing is clicked. The
 *  card renders the stamp separately so it can trail the row's buttons; the
 *  standalone renders keep it attached. */
export function SessionMetrics({
  session,
  id,
  showTimestamp = true,
  captureBytes = null,
}: {
  session: ProjectSessionSummary;
  id?: string;
  showTimestamp?: boolean;
  /** What a push of this session weighs; drawn in Push review only, where
   *  the budget makes the number mean something. */
  captureBytes?: number | null;
}) {
  const durationText = formatDuration(session.started_at, session.ended_at);
  const durationLabel = durationText
    ? `${session.is_active ? "Active for" : "Ran for"} ${durationText}`
    : "";
  const totalTokens = formatCount(session.total_tokens);
  const totalInputTokens = formatCount(session.total_input_tokens);
  const totalOutputTokens = formatCount(session.total_output_tokens);

  return <div id={id} className="sync-history-thread-meta" aria-label="Session details">
    {/* Ordered by what the row is about: the exchange first, then the work it
        did, then what it cost, then how long it took. */}
    {/* One message count sums prompts and final replies. The tooltip shows the
        split, one value per line. */}
    <span className="sync-history-thread-metric" data-tooltip={`Prompts · ${session.user_round_count}\nReplies · ${session.agent_message_count}`} aria-label={`Messages: ${session.user_round_count + session.agent_message_count}. Prompts: ${session.user_round_count}. Replies: ${session.agent_message_count}`} tabIndex={0}><Icon name="message" size={12} /><b>{session.user_round_count + session.agent_message_count}</b></span>
    {session.tool_call_count > 0 && <span className="sync-history-thread-metric" data-tooltip={`Tool calls · ${session.tool_call_count}`} aria-label={`Tool calls: ${session.tool_call_count}`} tabIndex={0}><Icon name="tool" size={12} /><b>{session.tool_call_count}</b></span>}
    <span className="sync-history-thread-metric" data-tooltip={`Total input · ${totalInputTokens}\nTotal output · ${totalOutputTokens}`} aria-label={`Total tokens: ${totalTokens}. Total input: ${totalInputTokens}. Total output: ${totalOutputTokens}`} tabIndex={0}><Icon name="token" size={12} /><b>{totalTokens}</b></span>
    {!session.metrics_complete && <span className="sync-history-thread-metric sync-history-partial" data-tooltip="Some session metrics are unavailable" aria-label="Some session metrics are unavailable" tabIndex={0}><Icon name="alert-triangle" size={12} /></span>}
    {durationText && (
      <span className="sync-history-thread-metric sync-history-thread-duration" data-tooltip={durationLabel} aria-label={durationLabel} tabIndex={0}>
        <Icon name={session.is_active ? "activity" : "clock"} size={12} /><b>{durationText}</b>
      </span>
    )}
    {captureBytes != null && (
      <span className="sync-history-thread-metric" data-tooltip={`Push size · ${formatBytes(captureBytes)}`} aria-label={`Push size: ${formatBytes(captureBytes)}`} tabIndex={0}>
        <Icon name="drive" size={12} /><b>{formatBytes(captureBytes)}</b>
      </span>
    )}
    {showTimestamp && <SessionTimestamp session={session} />}
  </div>;
}

/** Copy for rows the local machine can see but must not change. */
function sessionAccessNote(session: ProjectSessionSummary): { label: string; detail: string } | null {
  switch (session.access) {
    case "view_only":
      return {
        label: "View only",
        detail: `View only — connect a ${session.provider_display_name} profile to sync this session.`,
      };
    case "unsupported":
      return {
        label: "View only",
        detail: `View only — this Mallard version has no adapter for ${session.provider_display_name} sessions.`,
      };
    case "bound_unavailable":
      return {
        label: "Unavailable",
        detail: `Unavailable — the ${session.provider_display_name} profile can't be read right now.`,
      };
    default:
      return null;
  }
}

/** Open carries the provider's own mark; Terminal shows the action itself. */
const SESSION_ACTION_ICONS: Record<Exclude<SessionActionDescriptor["kind"], "open_in_app">, IconName> = {
  open_in_terminal: "terminal",
};

interface SessionCardProps {
  session: ProjectSessionSummary;
  occurrenceKey: string;
  workingDirectory: string;
  busy?: boolean;
  busyActionId?: string | null;
  onSessionAction: (session: ProjectSessionSummary, action: SessionActionDescriptor) => void;
  onOpenSessionDetail?: (
    request: OpenSessionDetailRequest,
    trigger: HTMLButtonElement,
  ) => void;
  syncEntry?: ThreadSyncEntry;
  /** Capture/plan identity used by Push and Pull selection. This intentionally
   *  differs from `session.resource_id`, which addresses Activity and detail
   *  commands by portable conversation identity. */
  reviewResourceId: string;
  storageName?: string | null;
  /** Drawn when there is no comparison entry to read. Required, not optional:
   *  the badge slot is a grid column and must never resolve to nothing. */
  syncFallback: SessionSyncPresentation;
  selectionMode?: "push" | "pull";
  selected?: boolean;
  selectable?: boolean;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  onReviewPull?: () => void;
}

interface SessionDebugMenuProps {
  resourceId: string;
  logicalThreadId: string | null;
  providerSessionId: string | null;
  transcriptPath: string | null;
  workingDirectory: string;
}

export function sessionDebugCopyText(
  resourceId: string,
  logicalThreadId: string | null,
  providerSessionId: string | null,
  transcriptPath: string | null,
  workingDirectory: string,
): string {
  return [
    `Resource ID: ${resourceId}`,
    `Mallard thread ID: ${logicalThreadId ?? "Not recorded"}`,
    `Native session ID: ${providerSessionId ?? "Not recorded"}`,
    `Transcript file: ${transcriptPath ?? "Not recorded"}`,
    `Working directory: ${workingDirectory}`,
  ].join("\n");
}

/** Approximate rendered height for choosing the opening direction. */

type SessionDebugCopyTarget =
  | "resource-id"
  | "logical-thread-id"
  | "provider-session-id"
  | "transcript-path"
  | "working-directory"
  | "all";

function SessionDebugMenu({
  resourceId,
  logicalThreadId,
  providerSessionId,
  transcriptPath,
  workingDirectory,
}: SessionDebugMenuProps) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState<SessionDebugCopyTarget | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const placement = useDebugPopoverPlacement(rootRef, panelRef, open, () => closeMenu());
  const menuId = `session-debug-${useId().replace(/:/g, "")}`;
  const rows: Array<{ key: Exclude<SessionDebugCopyTarget, "all">; label: string; value: string }> = [
    { key: "resource-id", label: "Resource ID", value: resourceId },
    ...(logicalThreadId != null
      ? [{ key: "logical-thread-id" as const, label: "Mallard thread ID", value: logicalThreadId }]
      : []),
    ...(providerSessionId != null
      ? [{ key: "provider-session-id" as const, label: "Native session ID", value: providerSessionId }]
      : []),
    ...(transcriptPath != null
      ? [{ key: "transcript-path" as const, label: "Transcript file", value: transcriptPath }]
      : []),
    { key: "working-directory", label: "Working directory", value: workingDirectory },
  ];
  const copiedLabel = copied === "resource-id"
    ? "Resource ID copied."
    : copied === "logical-thread-id"
      ? "Mallard thread ID copied."
      : copied === "provider-session-id"
        ? "Native session ID copied."
        : copied === "transcript-path"
          ? "Transcript file copied."
          : copied === "working-directory"
            ? "Working directory copied."
            : copied === "all"
              ? "Debug info copied."
              : "";

  const closeMenu = (restoreFocus = false) => {
    setOpen(false);
    setCopied(null);
    setCopyError(null);
    if (restoreFocus) window.requestAnimationFrame(() => triggerRef.current?.focus());
  };


  const copyValue = async (target: SessionDebugCopyTarget, value: string) => {
    try {
      await writeClipboardText(value, rootRef.current);
      setCopyError(null);
      setCopied(target);
    } catch (reason) {
      setCopied(null);
      setCopyError(`Could not copy: ${errorMessage(reason)}`);
    }
  };

  /* A dialog that closes when focus leaves it is unreachable by keyboard
     unless opening it also moves focus in. Once the panel has been placed,
     the first Copy button takes focus; Escape hands it back to the trigger. */
  const focusedForOpen = useRef(false);
  useEffect(() => {
    if (!open) {
      focusedForOpen.current = false;
      return;
    }
    if (!placement || focusedForOpen.current) return;
    focusedForOpen.current = true;
    const frame = window.requestAnimationFrame(() => {
      panelRef.current?.querySelector<HTMLElement>("button")?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [open, placement]);
  useEffect(() => {
    if (!open) return;
    // The panel is portalled onto body, so it is not inside rootRef. Both have
    // to count as "inside" or the first click on a Copy button dismisses it.
    const inside = (target: EventTarget | null) => target instanceof Node
      && Boolean(rootRef.current?.contains(target) || panelRef.current?.contains(target));
    const handlePointerDown = (event: PointerEvent) => {
      if (!inside(event.target)) closeMenu();
    };
    const handleFocusIn = (event: FocusEvent) => {
      if (!inside(event.target)) closeMenu();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
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
        title="Session info"
        aria-label="Show session info"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={menuId}
        onClick={() => open ? closeMenu() : setOpen(true)}
      >
        <Icon name="info" size={14} />
      </button>
      {/* On body, not in the row. Nothing in the list can bound it there:
          not the row's card, not the commit rail's stacking context, and
          not .sync-main's scrollport. */}
      <TopLayer>
        <div
          ref={panelRef}
          id={menuId}
          className="sync-history-debug-popover"
          style={placement
            ? {
              top: `${placement.top}px`,
              left: `${placement.left}px`,
              width: `${placement.width}px`,
              maxHeight: `${placement.maxHeight}px`,
            }
            /* Measured on the first layout pass; kept out of the paint
               until then so it never shows at the window origin. */
            : { visibility: "hidden" }}
          role="dialog"
          aria-label="Session info"
          hidden={!open}
        >
          <header>
            <strong>Session info</strong>
          </header>
          <div className="sync-history-debug-values">
            {/* No aria-label: it would replace the row's content as the
                accessible name, and the value is the one thing worth hearing.
                Named from content instead, so the path or digest is announced
                and stays navigable with a reading cursor. */}
            {rows.map((row) => (
              <button
                key={row.key}
                type="button"
                className="sync-history-debug-copy"
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
                sessionDebugCopyText(
                  resourceId,
                  logicalThreadId,
                  providerSessionId,
                  transcriptPath,
                  workingDirectory,
                ),
              )}
            >
              {copied === "all" ? "Copied all" : "Copy all"}
            </button>
          </footer>
        </div>

      </TopLayer>
    </div>
  );
}

/**
 * Per-row "Open Pull review" affordance for push rows held behind a Pull
 * (diverged or behind storage). The hold applies to this one session; the
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
  if (!["require_pull", "require_pull_metadata"].includes(entry.push_decision?.action)) return null;
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

/**
 * The pull operation, in the files review's chip grammar — "Add session",
 * "Update session · keeps local copy" — so every pull review names its
 * operations in one voice. Rows the pull would not touch return null; the
 * tick-dependent phrasing stays with the indicator label beside it.
 */
export function pullOperationChip(entry: ThreadSyncEntry | undefined): string | null {
  if (!entry || entry.state === "unavailable") return null;
  if (entry.compared_against_storage) {
    switch (entry.pull_decision.action) {
      case "materialize_cloud": return "Update session";
      case "keep_both": return "Update session · keeps local copy";
      case "create_local": return "Add session";
      case "adopt_cloud_metadata": return "Mark synced";
      default: return null;
    }
  }
  switch (entry.state) {
    case "storage_only": return "Add session";
    case "storage_ahead": return "Update session";
    case "diverged": return "Update session · keeps local copy";
    default: return null;
  }
}

function syncReviewStateLabel(
  mode: "push" | "pull",
  entry: ThreadSyncEntry | undefined,
  selected: boolean,
): string | null {
  if (!entry) return null;
  if (entry.state === "unavailable") return "Unavailable for sync";
  if (entry.compared_against_storage) {
    const decision = mode === "push" ? entry.push_decision : entry.pull_decision;
    switch (decision.action) {
      case "no_changes": return null;
      case "publish": return selected ? "Update storage" : "Not selected";
      case "create_cloud": return selected ? "Publish new task" : "Not selected";
      case "require_pull_metadata": return "Pull sync metadata";
      case "require_pull": return "Pull required";
      case "not_pushable": return "Not on this computer";
      case "adopt_cloud_metadata": return selected ? "Mark synced" : "Not selected";
      case "report_push_eligible": return "Ready to push";
      case "materialize_cloud": return selected ? "Update this task" : "Keep local";
      case "keep_both": return selected ? "Keep both" : "Not selected";
      case "create_local": return selected ? "Create local task" : "Not selected";
      case "report_local_only": return "Keep local";
      case "not_found": return "Not found";
      case "blocked": return "Can't sync";
    }
  }
  if (mode === "push") {
    if (entry.state === "storage_only" || entry.state === "storage_ahead" || entry.state === "diverged") {
      return "Pull required";
    }
    return entry.state === "unknown" ? "Review status" : null;
  }
  if (entry.state === "diverged") return selected ? "Keep both" : "Not selected";
  if (entry.state === "storage_only" || entry.state === "storage_ahead") {
    return selected ? "Restore from storage" : "Keep local";
  }
  if (entry.state === "local_only" || entry.state === "local_ahead") return "Keep local";
  return entry.state === "synced" ? null : "Review status";
}

/**
 * Fine print for rows that cannot proceed in the active direction. Speaks
 * user vocabulary only — sync verdicts never gate opening a session. Provider
 * naming comes from the entry, never from a hard-coded provider branch.
 */
export function syncHoldDetail(
  mode: "push" | "pull",
  entry: ThreadSyncEntry,
): string | null {
  const providerName = entry.provider_display_name ?? "the provider";
  if (entry.state === "unavailable") {
    return entry.status_detail ?? "Mallard could not safely read this session.";
  }
  const decision = mode === "push" ? entry.push_decision : entry.pull_decision;
  if (!decision) return null;
  if (mode === "push" && entry.state === "synced" && decision.action === "publish") {
    return entry.status_detail ?? "Export settings changed";
  }
  if (mode === "pull" && decision.action === "keep_both" && decision.blocked_reason) {
    return decision.blocked_reason;
  }
  if (decision.action === "blocked") {
    switch (decision.blocker) {
      case "codec_unsupported":
        return mode === "push"
          ? `Can't sync — Mallard can't safely capture this ${providerName} session. It still opens on this computer.`
          : "Can't sync — this stored copy can't be restored safely on this computer.";
      case "ambiguous_binding":
        return "Can't sync — more than one stored session could match this one.";
      case "missing_binding":
        return "Can't sync — this session isn't linked to a stored copy yet.";
      case "busy":
        return `Another ${providerName} sync operation is still finishing. Wait, then refresh.`;
      case "detached_requires_pull":
        return "This session's local mapping changed. Pull to establish the current mapping.";
      case "unknown_evidence":
        return "Can't sync — Mallard could not prove the local and stored session state safely.";
      default:
        return "Can't sync — this session can't be changed safely right now.";
    }
  }
  if (mode === "push" && decision.action === "require_pull_metadata") {
    return "The session content matches, but storage has newer sync metadata. Pull once before pushing.";
  }
  if (mode === "push" && decision.action === "require_pull") {
    return entry.state === "diverged"
      ? "Local and stored history have diverged. Pull will keep both histories as independent sessions."
      : "The stored session has newer messages. Pull them before pushing.";
  }
  return null;
}

/** Review rows use the same status presentation and pixel badge as Activity.
 *  Selection changes only the explanatory phrase; it never swaps the status
 *  glyph under the checkbox the user just clicked. */
function ReviewSyncIndicator({ entry, storageName, label, detail }: {
  entry: ThreadSyncEntry;
  storageName: string;
  label: string;
  detail: string | null;
}) {
  const presentation = sessionSyncPresentation(entry, storageName);
  return <SessionSyncIndicator presentation={{
    ...presentation,
    label: detail ? `${label}. ${detail}` : label,
  }} />;
}

/** Disabled reason wins over the busy phrase, which wins over the plain label. */
function actionTitle(action: SessionActionDescriptor, busy = false): string {
  if (!action.enabled) return action.disabled_reason ?? action.label;
  return busy ? `${action.label}…` : action.label;
}

function ChildOmissionIcon() {
  const tooltipId = useId();
  return (
    <span className="sync-child-omission" tabIndex={0} aria-label="Child rollouts excluded" aria-describedby={tooltipId}>
      <Icon name="info" size={14} />
      <span id={tooltipId} role="tooltip" className="sync-child-omission-tooltip">
        Child conversations are excluded from this Push. Pull restores the children stored in the cloud.
      </span>
    </span>
  );
}

function SessionCard({
  session, occurrenceKey, workingDirectory, busy = false, busyActionId = null,
  onSessionAction, onOpenSessionDetail,
  syncEntry, reviewResourceId, storageName, syncFallback, selectionMode, selected = false, selectable = false,
  selectionDisabled = false, onToggleResource, onReviewPull,
}: SessionCardProps) {
  const fullTitle = session.title || "Untitled session";
  const visibleTitle = compactThreadTitle(fullTitle);
  const reviewStateLabel = selectionMode ? syncReviewStateLabel(selectionMode, syncEntry, selected) : null;
  const holdDetail = selectionMode && syncEntry ? syncHoldDetail(selectionMode, syncEntry) : null;
  const syncBlockedPresentation: SessionSyncPresentation | null = session.sync_blocked_reason
    ? {
        status: "unavailable",
        className: "unavailable",
        label: `Unavailable for sync. ${session.sync_blocked_reason}`,
      }
    : null;
  // The status badge owns row state; the branch-unmapped heading owns its pencil.
  const accessNote = sessionAccessNote(session);
  const canOpenDetails = !selectionMode
    && session.source !== "remote_only"
    && session.access === "bound_ready"
    && onOpenSessionDetail != null;
  const openAction = session.actions.find((action) => action.kind === "open_in_app") ?? null;
  const otherActions = session.actions.filter(
    (action): action is SessionActionDescriptor & { kind: keyof typeof SESSION_ACTION_ICONS } =>
      action.kind !== "open_in_app",
  );
  const openSessionDetail = (trigger: HTMLButtonElement) => {
    if (!canOpenDetails) return;
    onOpenSessionDetail?.({
      resourceId: session.resource_id,
      occurrenceKey,
    }, trigger);
  };
  return (
    <article className={`sync-history-thread-card${selectionMode && selectable ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectionMode && selectable && (
            <label
              className="sync-sync-row-choice"
              title={`${selected ? "Exclude" : "Include"} ${session.title || "this session"}`}
            >
              <input
                type="checkbox"
                checked={selected}
                disabled={selectionDisabled}
                aria-label={`${selected ? "Exclude" : "Include"} ${session.title || "Untitled session"}`}
                onChange={() => onToggleResource?.(reviewResourceId)}
              />
            </label>
          )}
          {syncBlockedPresentation ? (
            <SessionSyncIndicator presentation={syncBlockedPresentation} />
          ) : selectionMode && reviewStateLabel && syncEntry ? (
            <ReviewSyncIndicator
              entry={syncEntry}
              storageName={storageName ?? "storage"}
              label={reviewStateLabel}
              detail={holdDetail}
            />
          ) : (
            <SessionSyncIndicator
              presentation={syncEntry && storageName
                ? sessionSyncPresentation(syncEntry, storageName)
                : syncFallback}
            />
          )}
          {accessNote && (
            <span
              className="sync-history-branch-state history-only"
              title={accessNote.detail}
              aria-label={accessNote.detail}
            >
              {accessNote.label}
            </span>
          )}
          {canOpenDetails ? (
            <button
              type="button"
              className="sync-history-session-title-button"
              title={visibleTitle === fullTitle ? "View session details" : `${fullTitle} · View session details`}
              onClick={(clickEvent) => openSessionDetail(clickEvent.currentTarget)}
            >
              {visibleTitle}
            </button>
          ) : (
            <strong title={visibleTitle === fullTitle ? undefined : fullTitle}>{visibleTitle}</strong>
          )}
          {selectionMode === "push" && syncEntry?.subagent_selection?.omitted && <ChildOmissionIcon />}
          {session.preserved && (
            <span
              className="sync-history-preserved-badge"
              title={`Local branch preserved during a divergent Pull of thread ${session.preserved.preserved_from.slice(0, 8)}.`}
            >
              Preserved {session.preserved.ordinal}
            </span>
          )}
          {selectionMode === "push" && syncEntry?.state === "synced" && syncEntry.push_decision.action === "publish" && (
            <span className="sync-history-branch-state history-only">{syncEntry.status_detail ?? "Export settings changed"}</span>
          )}
          {selectionMode === "pull" && pullOperationChip(syncEntry) && (
            <span className="sync-history-branch-state history-only" title={holdDetail ?? reviewStateLabel ?? undefined}>
              {pullOperationChip(syncEntry)}
            </span>
          )}
          <ReviewPullRowAction
            mode={selectionMode}
            entry={syncEntry}
            disabled={selectionDisabled || busy}
            onReviewPull={onReviewPull}
          />
        </div>
        {/* Line 1 carries the backend actions only — the things that open this
            session somewhere else. The two row-level clickables live on line 2,
            parted from the readouts by a rule. */}
        <div className="sync-history-thread-actions">
          {openAction ? (
            <button
              type="button"
              className="btn btn-ghost sync-history-launch-action"
              disabled={!openAction.enabled || busy}
              onClick={() => onSessionAction(session, openAction)}
              title={actionTitle(openAction)}
              aria-label={openAction.label}
            >
              {busy && busyActionId === openAction.action_id
                ? <Icon name="refresh" size={14} className="icon-spin" />
                : <ProviderMark provider={session.provider} displayName={session.provider_display_name} />}
            </button>
          ) : (
            // Nothing to open — a stored-only session, or one no profile can
            // read. The mark stays, dimmed and inert, because it is the row's
            // provider identity as well as its button.
            <span
              className="sync-history-launch-action sync-history-provider-inert"
              title={`${session.provider_display_name} session`}
              aria-label={`${session.provider_display_name} session`}
            >
              <ProviderMark provider={session.provider} displayName={session.provider_display_name} />
            </span>
          )}
          {otherActions.map((action) => {
            const actionBusy = busy && busyActionId === action.action_id;
            return (
              <button
                key={action.action_id}
                type="button"
                className="btn btn-ghost sync-history-launch-action"
                disabled={!action.enabled || busy}
                onClick={() => onSessionAction(session, action)}
                title={actionTitle(action, actionBusy)}
                aria-label={action.label}
              >
                <Icon
                  name={actionBusy ? "refresh" : SESSION_ACTION_ICONS[action.kind]}
                  size={14}
                  className={actionBusy ? "icon-spin" : undefined}
                />
              </button>
            );
          })}
        </div>
      </div>


      {/* Line 2 reads: readouts │ debug detail ……… stamp. The rule is what keeps
          the two clickables from reading as a sixth and seventh readout — every
          glyph left of it is hover-only, everything right of it is a button. The
          stamp still closes the line so it forms one column down the list. */}
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <SessionMetrics
            session={session}
            showTimestamp={false}
            captureBytes={selectionMode === "push" ? syncEntry?.capture_bytes ?? null : null}
          />
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <SessionDebugMenu
              resourceId={session.resource_id}
              logicalThreadId={session.logical_thread_id ?? null}
              providerSessionId={session.provider_session_id ?? null}
              transcriptPath={session.transcript_path ?? null}
              workingDirectory={workingDirectory}
            />
            {canOpenDetails && (
              <button
                type="button"
                className="btn btn-ghost sync-history-session-detail-action"
                aria-label="View session details"
                title="View session details"
                onClick={(clickEvent) => openSessionDetail(clickEvent.currentTarget)}
              >
                <Icon name="chevron-right" size={13} />
              </button>
            )}
          </div>
          <SessionTimestamp session={session} />
        </div>
      </div>
    </article>
  );
}

function SyncEntryCard({
  entry,
  storageName,
  selectionMode,
  selected = false,
  selectable = false,
  selectionDisabled = false,
  onToggleResource,
  onReviewPull,
}: {
  entry: ThreadSyncEntry;
  storageName: string;
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
  const identityLabel = `${title}, ${entry.provider_display_name} thread ${entry.thread_id}`;
  const updatedAt = entry.storage_updated_at ?? entry.local_updated_at;
  const reviewStateLabel = selectionMode ? syncReviewStateLabel(selectionMode, entry, selected) : null;
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
          {selectionMode && reviewStateLabel ? (
            <ReviewSyncIndicator
              entry={entry}
              storageName={storageName}
              label={reviewStateLabel}
              detail={holdDetail}
            />
          ) : (
            <SessionSyncIndicator presentation={sessionSyncPresentation(entry, storageName)} />
          )}
          {/* Nothing to open — the thread is a comparison entry, not a session
              — so the mark is inert here by construction. It still identifies
              the provider, which is why it renders at all. */}
          <span
            className="sync-history-launch-action sync-history-provider-inert"
            title={`${entry.provider_display_name} thread`}
            aria-label={`${entry.provider_display_name} thread`}
          >
            <ProviderMark provider={entry.provider} displayName={entry.provider_display_name} />
          </span>
          <strong title={title === fullTitle ? undefined : fullTitle}>{title}</strong>
          {selectionMode === "push" && entry.state === "synced" && entry.push_decision.action === "publish" && (
            <span className="sync-history-branch-state history-only">{entry.status_detail ?? "Export settings changed"}</span>
          )}
          {selectionMode === "pull" && pullOperationChip(entry) && (
            <span className="sync-history-branch-state history-only" title={holdDetail ?? reviewStateLabel ?? undefined}>
              {pullOperationChip(entry)}
            </span>
          )}
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
        {selectionMode === "push" && entry.capture_bytes != null && (
          <span className="sync-history-thread-metric" data-tooltip={`Push size · ${formatBytes(entry.capture_bytes)}`} aria-label={`Push size: ${formatBytes(entry.capture_bytes)}`} tabIndex={0}>
            <Icon name="drive" size={12} /><b>{formatBytes(entry.capture_bytes)}</b>
          </span>
        )}
        {updatedAt != null && (
          <time dateTime={new Date(updatedAt * 1_000).toISOString()}>{formatDate(updatedAt)}</time>
        )}
      </span>
    </article>
  );
}

/** Every fold control shares the timeline rail's column. Because the button is
 *  outside the card, neither a review checkbox nor an outcome banner can move
 *  it. It precedes the heading and list in the DOM, so the trigger reads before
 *  the content it controls. It points right when collapsed and down when open. */
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

/** Deterministic activity order: latest activity first, then provider key,
 *  then resource ID — the same comparator the backend uses, applied again
 *  after client-side page merges so identical native IDs from different
 *  providers keep one stable order. */
function compareSessions(left: ProjectSessionSummary, right: ProjectSessionSummary): number {
  return right.ended_at - left.ended_at
    || (left.provider < right.provider ? -1 : left.provider > right.provider ? 1 : 0)
    || (left.resource_id < right.resource_id ? -1 : left.resource_id > right.resource_id ? 1 : 0);
}

/** Notice copy for a provider whose sessions cannot all be acted on here. */
function providerNoticeCopy(state: ProviderActivityState): string {
  switch (state.access) {
    case "view_only":
      return `View only — connect a ${state.display_name} profile to sync these sessions.`;
    case "unsupported":
      return `View only — this Mallard version has no adapter for ${state.display_name} sessions.`;
    case "bound_unavailable":
      return `The ${state.display_name} profile can't be read right now. Its sessions fail closed until it is reachable again.`;
    default:
      return "";
  }
}

/** How loudly one of those provider notices reads.
 *
 * A profile Mallard cannot reach means the reader is seeing less than the
 * whole truth —
 * those sessions fail closed until it comes back — which is exactly `warning`.
 * `view_only` and `unsupported` are facts about how the list was produced,
 * nothing is missing and nothing is broken, so they stay at the quiet rung.
 */
function providerNoticeSeverity(state: ProviderActivityState): NoticeSeverity {
  return state.access === "bound_unavailable" ? "warning" : "info";
}

export function ProjectChatHistoryContent({
  project, binding, activity, loading, loadingMore, actionError, loadError, onRetryLoad, actionBusyResourceId, actionBusyActionId = null, onShowLess, canShowLess,
  onLoadMore,
  onSessionAction, onOpenSessionDetail, comparison = null,
  comparisonLoading = false, comparisonError = null, activeStorageName = null,
  selectionMode, selectedResourceIds = new Set(), selectableResourceIds,
  selectionDisabled = false, onToggleResource, onReviewPull, onUnlinkStorage,
  onChooseAgentProfile,
  continuityActionBusy = false,
}: ContentProps) {
  const [collapsedCommitShas, setCollapsedCommitShas] = useState<Set<string>>(new Set());
  const [branchUnmappedCollapsed, setBranchUnmappedCollapsed] = useState(false);
  const [storageOnlyCollapsed, setStorageOnlyCollapsed] = useState(false);
  const label = projectLabel(project);
  const allSessions = useMemo(
    () => [...(activity?.sessions ?? [])].sort(compareSessions),
    [activity?.sessions],
  );
  const syncIndex = useMemo(
    () => indexSessionSyncEntries(comparison?.entries ?? []),
    [comparison?.entries],
  );
  const sessions = useMemo(() => {
    if (!selectionMode) return allSessions;
    return allSessions.filter((session) => {
      const entry = syncEntryForSession(session, syncIndex);
      if (!entry) return false;
      const status = sessionSyncPresentation(entry, "storage").status;
      return selectionMode === "pull"
        ? isPullReviewRow(status, entry.pull_decision)
        : isPushReviewRow(status, entry.push_decision);
    });
  }, [allSessions, selectionMode, syncIndex]);
  const sessionsByResource = useMemo(
    () => new Map(sessions.map((session) => [session.resource_id, session])),
    [sessions],
  );
  const syncBySessionResource = useMemo(() => {
    const matched = new Map<string, ThreadSyncEntry>();
    for (const session of sessions) {
      const entry = syncEntryForSession(session, syncIndex);
      if (entry) matched.set(session.resource_id, entry);
    }
    return matched;
  }, [sessions, syncIndex]);
  const providerNames = useMemo(
    () => new Map((activity?.providers ?? []).map((state) => [state.provider, state.display_name])),
    [activity?.providers],
  );
  const providerNotices = (activity?.providers ?? []).filter((state) => state.access !== "bound_ready");
  const providerErrors = useMemo(
    () => Object.entries(comparison?.provider_errors ?? {})
      .sort(([left], [right]) => left.localeCompare(right)),
    [comparison?.provider_errors],
  );
  const comparisonStorageName = comparison?.storage_name ?? activeStorageName ?? "selected storage";
  // Resolved the same way the rows' status badges resolve it, so the group
  // heading and the tooltips under it can never name two different storages.
  const storageOnlyName = (comparison ? comparisonStorageName : activeStorageName) ?? "storage";
  // Unmapped rows split by where the session lives. A storage-only session
  // stays in its storage group; a local session states only that this branch
  // has no matching commit.
  const unmappedReferences = (activity?.unmapped ?? [])
    .filter((reference) => sessionsByResource.has(reference.resource_id));
  const storageOnlyReferences = unmappedReferences.filter((reference) => sessionsByResource.get(reference.resource_id)?.source === "remote_only");
  const branchUnmappedReferences = unmappedReferences.filter((reference) => sessionsByResource.get(reference.resource_id)?.source !== "remote_only");
  /* A comparison row is represented only when the Activity projection will
     render its session. Presence in `activity.sessions` is not enough: a
     missing commit/unmapped reference would otherwise hide a planned Push or
     Pull row from both the Git rail and the fallback list. */
  const renderedSessionIds = useMemo(() => {
    if (!activity?.git) return new Set(sessions.map((session) => session.resource_id));
    return new Set([
      ...activity.git.commits.flatMap((commit) => (
        commit.session_refs.map((reference) => reference.resource_id)
      )),
      ...(activity.unmapped ?? []).map((reference) => reference.resource_id),
    ].filter((resourceId) => sessionsByResource.has(resourceId)));
  }, [activity, sessions, sessionsByResource]);
  const renderedSyncEntryIds = useMemo(() => {
    const entryIds = new Set<string>();
    for (const resourceId of renderedSessionIds) {
      const entry = syncBySessionResource.get(resourceId);
      if (entry) entryIds.add(entry.resource_id);
    }
    return entryIds;
  }, [renderedSessionIds, syncBySessionResource]);
  /* The non-git flat list's own split of the same fact, session-typed rather
     than reference-typed because that list renders sessions directly. */
  const cloudSessions = sessions.filter((session) => session.source === "remote_only");
  const localSessions = sessions.filter((session) => session.source !== "remote_only");
  const compactSelectionReview = selectionMode != null;
  /* Review rows the activity scan has no session row for. A review loads the
     whole span at once, so this is only what no activity row can ever reach;
     a review must still offer it or it would drop candidates silently. It
     renders as plain rows at the foot of the same list — never a box. */
  const unlistedEntries = useMemo(() => {
    if (!selectionMode) return [];
    return (comparison?.entries ?? []).filter((entry) => (
      (entry.local_present || entry.storage_present)
      && (selectionMode === "pull"
        ? isPullReviewRow(sessionSyncPresentation(entry, "storage").status, entry.pull_decision)
        : isPushReviewRow(sessionSyncPresentation(entry, "storage").status, entry.push_decision))
      && !renderedSyncEntryIds.has(entry.resource_id)
    ));
  }, [comparison?.entries, selectionMode, renderedSyncEntryIds]);
  const hasBoundProvider = Object.keys(binding?.profile_ids ?? {}).length > 0;
  // Resolved once for the page: every row without a comparison entry draws this
  // instead of drawing nothing.
  const syncFallback = useMemo(() => unresolvedSyncPresentation({
    comparisonLoaded: comparison != null,
    comparisonLoading,
    comparisonError,
    storageName: comparison?.storage_name ?? activeStorageName,
    hasBoundProvider,
  }), [comparison, comparisonLoading, comparisonError, activeStorageName, hasBoundProvider]);
  const sectionTitle = activity?.git ? "Git history" : activity ? "Sessions" : "Activity";
  const windowRange = activity ? formatDayRange(activity.window_start, activity.window_end) : "";
  /* An empty rail either belongs to an unborn branch or to a loaded range
     without commits. Branch choices come only from the local repository. */
  const emptyRail = useMemo(() => {
    if ((activity?.older?.commits ?? 0) === 0) {
      return { icon: "git-commit" as IconName, title: "No commits yet", meta: null };
    }
    return { icon: "range" as IconName, title: "No commits", meta: windowRange };
  }, [activity?.older?.commits, windowRange]);
  const refreshStatus = comparisonLoading
    ? `Comparing threads with ${activeStorageName ?? "storage"}`
    : loading
      ? "Refreshing project activity"
      : null;
  const renderSession = (session: ProjectSessionSummary, key: string) => {
    const entry = syncBySessionResource.get(session.resource_id);
    const reviewResourceId = entry?.resource_id ?? session.resource_id;
    const workingDirectory = session.working_directory
      || binding?.canonical_project_root
      || binding?.project_root
      || project.project_root
      || "Not configured";
    const selectable = !!selectionMode
      && session.access === "bound_ready"
      && !session.sync_blocked_reason
      && (selectableResourceIds?.has(reviewResourceId) ?? true);
    return (
    <SessionCard key={key} session={session} occurrenceKey={key} workingDirectory={workingDirectory}
      busy={actionBusyResourceId === session.resource_id} busyActionId={actionBusyActionId}
      onSessionAction={onSessionAction} onOpenSessionDetail={onOpenSessionDetail}
      syncEntry={entry} reviewResourceId={reviewResourceId}
      storageName={comparison ? comparisonStorageName : activeStorageName} syncFallback={syncFallback}
      selectionMode={selectionMode} selected={selectedResourceIds.has(reviewResourceId)}
      selectable={selectable}
      selectionDisabled={selectionDisabled} onToggleResource={onToggleResource}
      onReviewPull={onReviewPull} />
    );
  };
  const orderedReferences = (references: { resource_id: string }[]) => references
    .filter((reference) => sessionsByResource.has(reference.resource_id))
    .sort((left, right) => {
      const leftSession = sessionsByResource.get(left.resource_id);
      const rightSession = sessionsByResource.get(right.resource_id);
      if (!leftSession || !rightSession) return left.resource_id.localeCompare(right.resource_id);
      return rightSession.ended_at - leftSession.ended_at
        || leftSession.started_at - rightSession.started_at
        || left.resource_id.localeCompare(right.resource_id);
    });
  const commits = useMemo(() => {
    const candidates = activity?.git?.commits ?? [];
    if (!selectionMode) return candidates;
    return candidates.filter((commit) => (
      commit.session_refs.some((reference) => sessionsByResource.has(reference.resource_id))
    ));
  }, [activity?.git?.commits, selectionMode, sessionsByResource]);
  // A review holds the whole span at once, so an empty one is truly empty.
  const selectionReviewEmpty = selectionMode != null
    && comparison != null
    && sessions.length === 0
    && unlistedEntries.length === 0;
  const showGitTimeline = !selectionMode
    || branchUnmappedReferences.length > 0
    || commits.length > 0;
  const toggleCommit = (sha: string) => {
    setCollapsedCommitShas((current) => {
      const next = new Set(current);
      if (next.has(sha)) next.delete(sha);
      else next.add(sha);
      return next;
    });
  };

  /* One notice zone takes the loudest severity and lists the remaining page,
   * provider, refresh, continuity, and comparison messages in source order. */
  const notices: NoticeItem[] = [];
  if (actionError) {
    notices.push({ key: "action-failed", severity: "danger", message: actionError });
  }
  /* A refresh that failed on top of an already-loaded page: the stale page
     stays useful, so the message owns the retry instead of replacing the list.
     A first load that failed has no list to keep and stays a whole-page state
     below. */
  const refreshFailed = Boolean(loadError) && activity != null;
  if (refreshFailed) {
    notices.push({
      key: "refresh-failed",
      severity: "danger",
      message: `Couldn't refresh activity: ${loadError}`,
      action: (
        <button type="button" className="btn btn-ghost" onClick={onRetryLoad} disabled={loading}>
          Try again
        </button>
      ),
    });
  }
  if (comparison?.remote_continuity?.state === "replaced") {
    notices.push({
      key: "continuity-replaced",
      severity: "warning",
      title: "Storage history changed",
      message: `The sync history in ${comparisonStorageName} is different from the one Mallard last synced for this project (revision ${comparison.remote_continuity.base_revision}). This usually means Mallard's sync data in that storage was deleted and created again. The session statuses below use the replacement. Unlink this storage and link it again to start fresh. Unlinking does not delete files, but anything saved only in the previous storage history cannot be recovered from this machine.`,
      // The recovery is one verb, and only when the host offers it.
      action: onUnlinkStorage ? (
        <button type="button" className="btn btn-ghost danger" onClick={onUnlinkStorage} disabled={continuityActionBusy}>
          Unlink storage
        </button>
      ) : undefined,
    });
  }
  /* One note per provider whose rows are visible but not fully actionable,
     straight from the backend's provider states — the page never derives
     access from a provider name. */
  for (const state of providerNotices) {
    notices.push({
      key: `provider-notice:${state.provider}`,
      severity: providerNoticeSeverity(state),
      title: state.display_name,
      message: `— ${providerNoticeCopy(state)}${state.detail ? ` ${state.detail}` : ""}`,
    });
  }
  /* A whole-comparison failure belongs in the shared notice zone, separate
     from provider-scoped failures. */
  if (comparisonError) {
    notices.push({
      key: "comparison-failed",
      severity: "warning",
      title: `Comparison with ${comparisonStorageName} failed`,
      message: `— ${comparisonError} Every row below falls back to an unknown status.`,
    });
  }
  /* Comparison failures are isolated per provider: one failed provider warns
     here while every other provider's rows keep their statuses. */
  for (const [provider, message] of providerErrors) {
    notices.push({
      key: `provider-error:${provider}`,
      severity: "warning",
      title: providerNames.get(provider) ?? provider,
      message: `comparison failed: ${message}`,
    });
  }
  /* Only two of those six are events. `actionError` is set in the catch of a
   * row action the user just ran, and a `loadError` sitting over a loaded page
   * can only have come from the refresh or the load-more the user asked for;
   * both are things that HAPPENED and did not finish, so either one re-times
   * the whole box to assertive. Everything else here is standing content: the
   * continuity state, the provider access states and both comparison failures
   * all arrive with the data this view is first rendered from, and WCAG 2.2
   * SC 4.1.3 says content already present when the view renders is not a
   * status message. It gets a named region instead. */
  const noticeTiming: NoticeTiming = actionError || refreshFailed
    ? { timing: "blocking" }
    : { timing: "standing", label: "Notes about this project activity" };

  const content = (
      <section
        className="sync-history-content"
        aria-label={sectionTitle}
      >
        {/* The workspace path bar owns branch, counts, and refresh. */}
        {refreshStatus && <span className="sync-visually-hidden" role="status">{refreshStatus}</span>}

        <NoticeZone
          items={notices}
          subject="notes about this project activity"
          className="sync-history-notices"
          {...noticeTiming}
        />
        {!hasBoundProvider ? (
          <div className="sync-history-state sync-history-profile-state">
            <Icon name="alert-triangle" size={18} />
            <div>
              <strong>No agent profile is connected.</strong>
              <span>Choose an Agent profile to load this project’s sessions.</span>
            </div>
            {onChooseAgentProfile && (
              <button
                type="button"
                className="btn"
                disabled={continuityActionBusy}
                onClick={onChooseAgentProfile}
              >
                Choose Agent profile
              </button>
            )}
          </div>
        ) : loading && !activity ? (
          /* Not a status message: the scaffold is what the view renders with
             while its first read is in flight — the page's own row geometry
             in bones, so the list lands without a jump. On the one path that
             reaches it from a user action — picking a branch, which drops
             the list — the visually hidden line above is already announcing
             the refresh politely. Claiming a live region here only made a
             screen reader say it twice. */
          <LoadingScaffold shape="rows" label="Loading project activity" />
        ) : !activity ? (
          /* A failed first load is a page of its own, never a blank one: it
             names what failed and carries its own retry, because the trail's
             refresh control is not rendered for every project shape.
             It announces, because it is never there at first paint: the load
             runs from a mount effect and clears `loadError` on the way in, so
             this box replaces the loading line after the fetch settles — on
             the very first load as much as on a retry or a branch switch. A
             region here would leave the reader with the polite "Loading…" and
             then silence, unable to tell a finished retry from a failed one. */
          loadError ? (
            <div className="sync-history-state sync-history-profile-state" role="alert">
              <Icon name="alert-triangle" size={18} />
              <div>
                <strong>{`Couldn't load activity for ${label}.`}</strong>
                <span>{loadError}</span>
              </div>
              <button type="button" className="btn" onClick={onRetryLoad} disabled={loading}>
                <Icon name="refresh" size={13} /> Try again
              </button>
            </div>
          ) : null
        ) : (
          <>
            {selectionReviewEmpty && (
              <div className="sync-pull-empty">
                <Icon name="check-circle" size={16} />
                <span>
                  <strong>{selectionMode === "push" ? "Nothing to push" : "No session changes"}</strong>
                  <small>
                    {selectionMode === "push"
                      ? "No local session changes are ready to push."
                      : "Sessions already match this storage."}
                  </small>
                </span>
              </div>
            )}
            {!selectionReviewEmpty && activity.git && storageOnlyReferences.length > 0 && (
              /* Above the timeline, not on it: these rows have no commit and no
                 local file, so the group draws the timeline's frame treatment
                 but no rail segment. The rows are the ordinary session card —
                 the backend already hands a stored session no actions, and
                 details stay closed for `remote_only`, so nothing on the row
                 offers what a missing transcript cannot honor. */
              <section className={`sync-history-storage-only${storageOnlyCollapsed ? " collapsed" : ""}`} aria-labelledby="storage-only-heading">
                <SectionFold
                  collapsed={storageOnlyCollapsed}
                  controls="storage-only-list"
                  label={`${storageOnlyCollapsed ? "Expand" : "Collapse"} ${storageOnlyReferences.length} session${storageOnlyReferences.length === 1 ? "" : "s"} only in ${storageOnlyName}`}
                  onToggle={() => setStorageOnlyCollapsed(!storageOnlyCollapsed)}
                />
                <div className="sync-history-section-heading">
                  <h2 id="storage-only-heading"><Icon name="cloud" size={13} />Only in <span className="sync-history-storage-name">{storageOnlyName}</span></h2>
                  <span className="sync-history-heading-count" title={`${storageOnlyReferences.length} session${storageOnlyReferences.length === 1 ? "" : "s"} not on this computer`} aria-label={`${storageOnlyReferences.length} session${storageOnlyReferences.length === 1 ? "" : "s"} not on this computer`}><Icon name="message" size={12} />{storageOnlyReferences.length}</span>
                </div>
                {!storageOnlyCollapsed && (
                  <div id="storage-only-list" className="sync-history-thread-list flat">{storageOnlyReferences.map((reference) => { const session = sessionsByResource.get(reference.resource_id); return session ? renderSession(session, `storage-only:${session.resource_id}`) : null; })}</div>
                )}
              </section>
            )}
            {!selectionReviewEmpty && (activity.git ? (showGitTimeline ? (
              <div className={`sync-history-timeline${branchUnmappedReferences.length > 0 ? " has-branch-unmapped" : ""}`}>
                {branchUnmappedReferences.length > 0 && (
                  <section className={`sync-history-branch-unmapped${branchUnmappedCollapsed ? " collapsed" : ""}`} aria-labelledby="branch-unmapped-heading">
                    <SectionFold
                      collapsed={branchUnmappedCollapsed}
                      controls="branch-unmapped-list"
                      label={`${branchUnmappedCollapsed ? "Expand" : "Collapse"} ${branchUnmappedReferences.length} uncommitted session${branchUnmappedReferences.length === 1 ? "" : "s"}`}
                      onToggle={() => setBranchUnmappedCollapsed(!branchUnmappedCollapsed)}
                    />
                    <div className="sync-history-section-heading">
                      <h2 id="branch-unmapped-heading"><Icon name="edit" size={13} />Uncommitted</h2>
                    </div>
                    {!branchUnmappedCollapsed && (
                      <div id="branch-unmapped-list" className="sync-history-thread-list flat">{branchUnmappedReferences.map((reference) => { const session = sessionsByResource.get(reference.resource_id); return session ? renderSession(session, `branch-unmapped:${session.resource_id}`) : null; })}</div>
                    )}
                  </section>
                )}
                {(selectionMode !== "pull" || commits.length > 0) && (
                  <section className="sync-history-commit-section" aria-label={`First-parent commits on ${activity.git.selected_branch}`}>
                  {commits.length === 0 ? (
                    /* An empty rail is still the rail: the gap draws as a card
                       on it, so the page keeps the spine every other state is
                       read against, and the terminus below still has something
                       to hang from. The selected empty-state copy distinguishes
                       windowing from branches that contain no commits. */
                    <ol className="sync-history-commit-rail">
                      <li className="sync-history-commit sync-history-commit-gap">
                        {/* Not a fold: there is nothing here to expand, so the
                            rail column gets a marker rather than a control
                            that would do nothing when pressed. */}
                        <span className="sync-history-spine-fold inert" aria-hidden="true"><Icon name="chevron-down" size={12} /></span>
                        <div className="sync-history-commit-heading">
                          <span className="sync-history-commit-title">
                            <span className="sync-history-commit-icon" aria-hidden="true"><Icon name={emptyRail.icon} size={14} /></span>
                            <strong>{emptyRail.title}</strong>
                          </span>
                          {emptyRail.meta && (
                            <span className="sync-history-commit-meta">
                              <time dateTime={new Date(activity.window_start * 1_000).toISOString()}>{emptyRail.meta}</time>
                            </span>
                          )}
                        </div>
                      </li>
                    </ol>
                  ) : (
                    <ol className="sync-history-commit-rail">{commits.map((commit) => {
                      const commitUrl = githubCommitUrl(activity.git?.github_repository_url, commit.sha);
                      const references = orderedReferences(commit.session_refs);
                      const collapsed = collapsedCommitShas.has(commit.sha);
                      const displaySha = commit.short_sha.slice(0, 7);
                      const sessionListId = `commit-sessions-${commit.short_sha}`;
                      return (
                        <li key={commit.sha} className={`sync-history-commit${collapsed ? " collapsed" : ""}`}>
                          <SectionFold
                            collapsed={collapsed}
                            controls={sessionListId}
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
                            <div id={sessionListId} className="sync-history-thread-list">
                              {references.map((reference) => {
                                const session = sessionsByResource.get(reference.resource_id);
                                return session
                                  ? renderSession(session, `${commit.sha}:${session.resource_id}`)
                                  : null;
                              })}
                            </div>
                          )}
                        </li>
                      );
                    })}</ol>
                  )}
                  </section>
                )}
                {/* The paging control ends the rail it pages. A review never
                    pages: it loaded the whole span, so the rail simply ends. */}
                {!selectionMode && (
                  <RailFoot older={activity.older ?? null} canShowLess={canShowLess} busy={loadingMore}
                    railed onLoadMore={onLoadMore} onShowLess={onShowLess} />
                )}
              </div>
            ) : null) : (
              <section>
                {/* The flat list partitions the way the Catalog page and the
                    Files face do: stored-only sessions lead under the shared
                    plain section heading — no frame, no fold, no count — and
                    the local half takes a heading only while the partition
                    exists; alone, the leaf crumb already names it. */}
                {sessions.length === 0 ? (
                  <div className="sync-history-thread-list flat">
                    {/* Same rule as the empty rail: the window is named only
                        when paging past it could still turn something up. A
                        project whose homes hold no session at all gets no
                        "Load earlier" control, so blaming its window would
                        promise a step the page cannot offer. */}
                    <div className="sync-history-state">
                      {(activity.older?.sessions ?? 0) > 0 ? `No sessions in ${windowRange}.` : "No sessions yet."}
                    </div>
                  </div>
                ) : (
                  <>
                    {cloudSessions.length > 0 && (
                      <>
                        <div className="sync-history-section-heading">
                          <h2>
                            <Icon name="cloud" size={14} />
                            Only in <span className="sync-history-storage-name">{storageOnlyName}</span>
                          </h2>
                        </div>
                        <div className="sync-history-thread-list flat">
                          {cloudSessions.map((session) => renderSession(session, `storage-only:${session.resource_id}`))}
                        </div>
                      </>
                    )}
                    {localSessions.length > 0 && (
                      <>
                        {cloudSessions.length > 0 && (
                          <div className="sync-history-section-heading">
                            <h2><Icon name="message" size={14} />Sessions</h2>
                          </div>
                        )}
                        <div className="sync-history-thread-list flat">
                          {localSessions.map((session) => renderSession(session, session.resource_id))}
                        </div>
                      </>
                    )}
                  </>
                )}
                {/* Same control, no rail to hang it from: a project with no
                    repository draws no spine, so the node and its segment are
                    suppressed and the row keeps the list's own left edge. A
                    review never pages. */}
                {!selectionMode && (
                  <RailFoot older={activity.older ?? null} canShowLess={canShowLess} busy={loadingMore}
                    railed={false} onLoadMore={onLoadMore} onShowLess={onShowLess} />
                )}
              </section>
            ))}
            {/* Review rows no activity row reaches, as plain rows at the foot
                of the same list. The review loaded the whole span, so nothing
                here is "older": it is what the comparison knows and the
                activity scan does not, offered rather than silently dropped. */}
            {!selectionReviewEmpty && selectionMode && unlistedEntries.length > 0 && (
              <div className="sync-history-thread-list flat" aria-label="Sessions without an activity row">
                {unlistedEntries.map((entry) => (
                  <SyncEntryCard key={`unlisted:${entry.resource_id}`} entry={entry} storageName={comparisonStorageName}
                    selectionMode={selectionMode} selected={selectedResourceIds.has(entry.resource_id)}
                    selectable={entry.access === "bound_ready" && (selectableResourceIds?.has(entry.resource_id) ?? true)}
                    selectionDisabled={selectionDisabled} onToggleResource={onToggleResource}
                    onReviewPull={onReviewPull} />
                ))}
              </div>
            )}
          </>
        )}
      </section>
  );

  return <div className={`sync-history-page sync-history-embedded${compactSelectionReview ? " sync-history-selection-review" : ""}`}>{content}</div>;
}

/** How far back a review asks the activity to reach. A review shows every
 *  outstanding session at once, so it asks for the whole span back to the
 *  oldest one the comparison knows, plus a day of slack, instead of a 30-day
 *  page; with no comparison to measure by it asks for ten years. Never less
 *  than the standing page, so a young project reads the same either way. */
export function reviewWindowDays(
  comparison: ThreadSyncComparison | null | undefined,
  nowSeconds = Date.now() / 1_000,
): number {
  const oldest = (comparison?.entries ?? [])
    .map((entry) => entry.local_updated_at ?? entry.storage_updated_at ?? null)
    .filter((value): value is number => value != null)
    .reduce<number | null>((min, value) => (min == null || value < min ? value : min), null);
  if (oldest == null) return 3_650;
  return Math.max(30, Math.ceil((nowSeconds - oldest) / 86_400) + 1);
}

function mergePages(previous: ProjectActivity | null, next: ProjectActivity): ProjectActivity {
  if (!previous || previous.git?.selected_branch !== next.git?.selected_branch) return next;
  /* The merged page covers both windows, and everything between them: the
     cursor lands just past the newest thing below the previous window, so the
     span it skipped held nothing by construction. `older` comes from `next`,
     the deepest page — it is what still lies below all of this. */
  const window_start = Math.min(previous.window_start, next.window_start);
  const window_end = Math.max(previous.window_end, next.window_end);
  const sessions = new Map(previous.sessions.map((session) => [session.resource_id, session]));
  next.sessions.forEach((session) => sessions.set(session.resource_id, session));
  const mergedSessions = [...sessions.values()].sort(compareSessions);
  const unmapped = new Map(previous.unmapped.map((item) => [item.resource_id, item]));
  next.unmapped.forEach((item) => unmapped.set(item.resource_id, item));
  if (!previous.git || !next.git) return { ...next, window_start, window_end, sessions: mergedSessions, unmapped: [...unmapped.values()] };
  const commits = new Map(previous.git.commits.map((commit) => [commit.sha, commit]));
  next.git.commits.forEach((commit) => {
    const existing = commits.get(commit.sha);
    if (!existing) { commits.set(commit.sha, commit); return; }
    const references = new Map(existing.session_refs.map((reference) => [reference.resource_id, reference]));
    commit.session_refs.forEach((reference) => references.set(reference.resource_id, reference));
    commits.set(commit.sha, { ...commit, session_refs: [...references.values()] });
  });
  const mergedCommits = [...commits.values()].sort((left, right) => right.committed_at - left.committed_at);
  const unique = new Set(mergedCommits.flatMap((commit) => commit.session_refs.map((reference) => reference.resource_id))).size;
  return { ...next, window_start, window_end, sessions: mergedSessions, unmapped: [...unmapped.values()], git: { ...next.git, commits: mergedCommits, unique_session_count: unique } };
}

export default function ProjectActivityPage({
  project,
  binding,
  refreshEpoch,
  activeStorageId = null,
  activeStorageName = null,
  selectionMode,
  selectedResourceIds,
  selectableResourceIds,
  selectionDisabled = false,
  onToggleResource,
  onDropSelection,
  onReviewPull,
  onUnlinkStorage,
  onChooseAgentProfile,
  continuityActionBusy = false,
  comparisonOverride,
  onReviewListReadyChange,
  onHeaderModelChange,
  onOpenSessionDetail,
}: PageProps) {
  /* Keep loaded windows as a stack so Show less is a pop instead of a refetch. */
  const [pages, setPages] = useState<ProjectActivity[]>([]);
  const activity = useMemo(
    () => pages.reduce<ProjectActivity | null>((merged, page) => mergePages(merged, page), null),
    [pages],
  );
  const [comparison, setComparison] = useState<ThreadSyncComparison | null>(null);
  const [comparisonLoading, setComparisonLoading] = useState(false);
  const [comparisonError, setComparisonError] = useState<string | null>(null);
  const [branch, setBranch] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionBusyResourceId, setActionBusyResourceId] = useState<string | null>(null);
  const [actionBusyActionId, setActionBusyActionId] = useState<string | null>(null);
  const requestRef = useRef(0);
  const comparisonRequestRef = useRef(0);
  const hasBoundProvider = Object.keys(binding?.profile_ids ?? {}).length > 0;

  const load = async (beforeTime?: number | null, requestedBranch = branch) => {
    if (!hasBoundProvider) return false;
    const requestId = ++requestRef.current;
    if (beforeTime) setLoadingMore(true); else setLoading(true);
    setActionError(null);
    setLoadError(null);
    try {
      const next = await projectSyncApi.getProjectActivity(
        project.local_project_id,
        activeStorageId,
        requestedBranch,
        beforeTime,
        selectionMode ? reviewWindowDays(comparisonOverride) : 30,
      );
      if (requestId !== requestRef.current) return false;
      setPages((current) => beforeTime ? [...current, next] : [next]);
      setBranch(next.git?.selected_branch ?? null);
    } catch (reason) {
      // Page-level failures and row-action failures are different facts;
      // keeping them in one state made them indistinguishable on screen.
      if (requestId === requestRef.current) setLoadError(errorMessage(reason));
    } finally {
      if (requestId === requestRef.current) { setLoading(false); setLoadingMore(false); }
    }
    return requestId === requestRef.current;
  };

  const loadWorkspace = async (requestedBranch = branch) => {
    if (comparisonOverride !== undefined || activeStorageId === null) {
      comparisonRequestRef.current += 1;
      setComparison(comparisonOverride ?? null);
      setComparisonLoading(false);
      setComparisonError(null);
      await load(null, requestedBranch);
      return;
    }
    if (!hasBoundProvider) return;
    const requestId = ++requestRef.current;
    const comparisonRequestId = ++comparisonRequestRef.current;
    setLoading(true);
    setComparisonLoading(true);
    setLoadError(null);
    setComparisonError(null);
    setActionError(null);
    let receivedActivity = false;
    try {
      await projectSyncApi.refreshProjectActivity({
        localProjectId: project.local_project_id,
        storageId: activeStorageId,
        branch: requestedBranch,
        windowDays: 30,
        bindingRevision: binding?.revision ?? null,
        refreshEpoch,
      }, (event) => {
        if (event.stage === "activity" || event.stage === "activity_error") {
          if (requestId !== requestRef.current) return;
          if (event.stage === "activity") {
            receivedActivity = true;
            setPages([event.activity]);
            setBranch(event.activity.git?.selected_branch ?? null);
            setLoadError(null);
          } else {
            setLoadError(event.message);
          }
          setLoading(false);
        } else {
          if (comparisonRequestId !== comparisonRequestRef.current) return;
          if (event.stage === "comparison") {
            setComparison(event.comparison);
            setComparisonError(null);
          } else {
            setComparison(null);
            setComparisonError(event.message);
          }
          setComparisonLoading(false);
        }
      });
    } catch (reason) {
      if (requestId === requestRef.current && !receivedActivity) setLoadError(errorMessage(reason));
      if (comparisonRequestId === comparisonRequestRef.current) {
        setComparison(null);
        setComparisonError(errorMessage(reason));
      }
    } finally {
      if (requestId === requestRef.current) setLoading(false);
      if (comparisonRequestId === comparisonRequestRef.current) setComparisonLoading(false);
    }
  };

  useEffect(() => {
    // The whole per-project slate resets, not just the data. Leaving the error
    // and loading flags set carried the previous project's failure banner onto
    // the next one, and a switch made mid-flight stranded the spinners on
    // because their owning request had already been superseded.
    setPages([]); setBranch(null); setComparison(null);
    setLoadError(null); setComparisonError(null); setActionError(null);
    setLoading(false); setComparisonLoading(false); setLoadingMore(false);
    void loadWorkspace(null);
    return () => { requestRef.current += 1; comparisonRequestRef.current += 1; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.local_project_id, binding?.revision, activeStorageId, refreshEpoch, comparisonOverride]);

  const reviewListReady = activity !== null && !loading && comparison !== null && !comparisonLoading;
  useEffect(() => {
    onReviewListReadyChange?.(reviewListReady);
  }, [reviewListReady, onReviewListReadyChange]);

  // The path bar holds onto whatever model it was last handed, so a model that
  // is a new object every render would drive it into a re-render loop. These
  // two handlers are the only mutable part of it; a latest-ref keeps their
  // identity fixed while they still see this render's `load` and `branch`.
  const latest = useRef({ load, loadWorkspace, branch });
  latest.current = { load, loadWorkspace, branch };

  /* Show less pops the newest page off the stack. Every id a review can tick
     is either a session's own or its comparison entry's, so the set a row
     answers to is built the same way the row builds it — anything else would
     drop a project file or a tool, which no window ever governed.
     Drop only selected ids that leave the reachable set. */
  const visibleReviewIds = useCallback((page: ProjectActivity | null) => {
    const index = indexSessionSyncEntries(comparison?.entries ?? []);
    const ids = new Set<string>();
    for (const session of page?.sessions ?? []) {
      ids.add(session.resource_id);
      const entry = syncEntryForSession(session, index);
      if (entry) ids.add(entry.resource_id);
    }
    return ids;
  }, [comparison?.entries]);

  const showLess = useCallback(() => {
    if (pages.length <= 1) return;
    const next = pages.slice(0, -1);
    const before = visibleReviewIds(activity);
    const after = visibleReviewIds(
      next.reduce<ProjectActivity | null>((merged, page) => mergePages(merged, page), null),
    );
    const dropped = [...(selectedResourceIds ?? [])].filter((id) => before.has(id) && !after.has(id));
    setPages(next);
    if (dropped.length > 0) onDropSelection?.(dropped);
  }, [activity, onDropSelection, pages, selectedResourceIds, visibleReviewIds]);

  const changeBranch = useCallback((nextBranch: string) => {
    // The review parent owns selection. A branch changes only this page's Git
    // projection, so it must not clear selection or rebuild comparison.
    setBranch(nextBranch);
    setPages([]);
    void latest.current.load(null, nextBranch);
  }, []);
  const refreshActivity = useCallback(() => {
    void latest.current.loadWorkspace(latest.current.branch);
  }, []);

  const headerCounts = useMemo(
    () => activityHeaderCounts(
      comparison,
      comparison?.storage_name ?? activeStorageName ?? "selected storage",
    ),
    [comparison, activeStorageName],
  );
  const headerModel = useMemo<ActivityHeaderModel>(() => ({
    // A project with no repository has no branches to offer; the bar simply
    // draws no chip, and the counts and refresh still stand.
    branches: activity?.git?.branches ?? [],
    selectedBranch: activity?.git?.selected_branch ?? "",
    onSelectBranch: changeBranch,
    counts: headerCounts,
    comparisonError,
    refreshing: loading || comparisonLoading,
    onRefresh: refreshActivity,
  }), [
    activity?.git?.branches,
    activity?.git?.selected_branch,
    changeBranch,
    headerCounts,
    comparisonError,
    loading,
    comparisonLoading,
    refreshActivity,
  ]);
  useEffect(() => {
    onHeaderModelChange?.(headerModel);
  }, [headerModel, onHeaderModelChange]);
  // Leaving the page has to empty the bar, or its controls outlive the list
  // they speak for.
  useEffect(() => () => { onHeaderModelChange?.(null); }, [onHeaderModelChange]);

  const runSessionAction = async (session: ProjectSessionSummary, action: SessionActionDescriptor) => {
    if (!action.enabled) return;
    setActionError(null);
    setActionBusyResourceId(session.resource_id);
    setActionBusyActionId(action.action_id);
    try {
      await projectSyncApi.executeProjectSessionAction(project.local_project_id, session.resource_id, action.action_id);
    } catch (reason) { setActionError(errorMessage(reason)); }
    finally {
      setActionBusyResourceId(null);
      setActionBusyActionId(null);
    }
  };
  return <ProjectChatHistoryContent project={project} binding={binding} activity={activity} loading={loading} loadingMore={loadingMore} actionError={actionError} loadError={loadError} onRetryLoad={() => void loadWorkspace(branch)} actionBusyResourceId={actionBusyResourceId} actionBusyActionId={actionBusyActionId} onLoadMore={() => void load(activity?.older ? activity.older.newest_at + 1 : null, branch)} onShowLess={showLess} canShowLess={pages.length > 1} onSessionAction={(session, action) => void runSessionAction(session, action)} onOpenSessionDetail={onOpenSessionDetail} comparison={comparison} comparisonLoading={comparisonLoading} comparisonError={comparisonError} activeStorageName={activeStorageName} selectionMode={selectionMode} selectedResourceIds={selectedResourceIds} selectableResourceIds={selectableResourceIds} selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} onReviewPull={onReviewPull} onUnlinkStorage={onUnlinkStorage} onChooseAgentProfile={onChooseAgentProfile} continuityActionBusy={continuityActionBusy} />;
}
