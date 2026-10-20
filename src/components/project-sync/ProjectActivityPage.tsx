import type { ManagedCloudStatus } from "../managed-cloud/types";
import { confirm } from "@tauri-apps/plugin-dialog";
import type { ParseKeys } from "i18next";
import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import type {
  GitBranchSummary,
  GitHistoryOutgoing,
  LocalProjectSummary,
  NewSessionSurface,
  NoticeSeverity,
  OlderActivity,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionSummary,
  ProviderActivityState,
  ProviderKey,
  SessionActionDescriptor,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../types";
import Icon, { type IconName } from "../Icons";
import SessionRowIcon from "./SessionRowIcon";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { formatDuration, formatFullDateTime } from "../../clock";
import { formatBytes, formatCompactNumber, joinAnd } from "../../format";
import { t, Trans, uiLocale, useTranslation } from "../../i18n";
import { backendText } from "../../i18n/backend";
import { syncDecisionReason, syncStatusDetail } from "./syncDetailText";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import { githubCommitUrl, openExternalUrl } from "./externalLink";
import LoadingScaffold from "../LoadingScaffold";
import ListTile from "../sync-review/ListTile";
import NameSearchField from "../sync-review/NameSearchField";
import StatusScopeBar from "../sync-review/StatusScopeBar";
import { STATUS_SCOPES, type StatusScope } from "../sync-review/statusScopes";
import { nameMatchesSearch, type NameSearchResults } from "../sync-review/nameSearch";
import { localSessionDeleteError, projectLabel, sessionNoun } from "./model";
import { clearedStorageNotice } from "./clearedStorageNotice";
import { clearStorageWarningDismissal, dismissStorageWarning, useDismissedStorageWarning } from "../../preferences";
import ProviderMark from "../sync-review/ProviderMark";
import NewSessionMenu, { newSessionProviders } from "./NewSessionMenu";
import { writeClipboardText } from "../sync-review/clipboard";
import TopLayer from "../sync-review/TopLayer";
import { useDebugPopoverPlacement } from "../sync-review/debugPopoverPlacement";
import {
  isPullReviewRow,
  isPushReviewRow,
} from "../sync-review/pullReviewStatus";
import { formatSessionDetailTimestamp, type OpenSessionDetailRequest } from "./session-detail/sessionDetailModel";
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
  projectId: string;
  storageId: string | null;
  managedCloudStatus: ManagedCloudStatus | null;
  branches: GitBranchSummary[];
  selectedBranch: string;
  onSelectBranch: (branch: string) => void;
  counts: ActivityHeaderCounts;
  /** The whole comparison failed, as against one provider's failing. */
  comparisonError: string | null;
  comparison: ThreadSyncComparison | null;
  refreshing: boolean;
  onRefresh: () => void;
}

interface SessionSearchProps {
  searchQuery?: string;
  onSearchQueryChange?: (query: string) => void;
  onSearchResultsChange?: (results: NameSearchResults) => void;
}

interface PageProps extends SessionSearchProps {
  comparisonErrorHandled?: boolean;
  /**
   * Reports the latest row-action failure (open, resume, delete), or null to
   * clear it. The shell draws it under the path bar: this list scrolls as one
   * page, so a box at its top was off screen from any row further down.
   */
  onActionError: (message: string | null) => void;
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  refreshEpoch: number;
  activeStorageId?: string | null;
  activeStorageName?: string | null;
  selectionMode?: "push" | "pull";
  pushGitHistory?: PushGitHistory;
  selectedResourceIds?: ReadonlySet<string>;
  selectableResourceIds?: ReadonlySet<string>;
  /** A review of these rows alone (a session's own Pull): every other row
   *  the comparison holds is left out of the list. */
  reviewResourceIds?: ReadonlySet<string>;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  /** Drop selected rows hidden by Show less because they cannot be reviewed. */
  onDropSelection?: (resourceIds: string[]) => void;
  onReviewPull?: () => void;
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
   * nothing is applied sight unseen, and says why when a read failed.
   */
  onReviewListStateChange?: (state: ReviewListState) => void;
  /** What the hosting review already knows about the list it is waiting
   *  for, so the placeholders land at the list's own height. */
  reviewScaffold?: ReviewScaffold;
  /**
   * Hands the workspace path bar what it needs to speak for this page, and
   * null when there is nothing to say. Only the workspace passes it; the two
   * review hosts render this page inside their own chrome.
   */
  onHeaderModelChange?: (model: ActivityHeaderModel | null) => void;
  /** A non-Git project's Sessions/Files switch; only the workspace passes it. */
  viewSwitch?: ReactNode;
}

interface ContentProps extends SessionSearchProps {
  /** The status scope the standing list is narrowed to; null is All. Like
   *  the query, the list holds it unless its host does. */
  statusScope?: StatusScope | null;
  onStatusScopeChange?: (scope: StatusScope | null) => void;
  comparisonErrorHandled?: boolean;
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  activity: ProjectActivity | null;
  loading: boolean;
  loadingMore: boolean;
  /** Page-level load failure. A failed row action is never one of these: the
      page reports it to the shell, under the path bar. */
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
  onDeleteSession?: (session: ProjectSessionSummary) => void;
  onOpenSessionDetail?: (
    request: OpenSessionDetailRequest,
    trigger: HTMLButtonElement,
  ) => void;
  comparison?: ThreadSyncComparison | null;
  comparisonLoading?: boolean;
  storageLoading?: boolean;
  comparisonError?: string | null;
  activeStorageName?: string | null;
  /** The storage whose stored rows this list shows. A stored row opens its
   *  page through it; without one, a stored row's title stays plain. */
  storageId?: string | null;
  selectionMode?: "push" | "pull";
  pushGitHistory?: PushGitHistory;
  selectedResourceIds?: ReadonlySet<string>;
  selectableResourceIds?: ReadonlySet<string>;
  reviewResourceIds?: ReadonlySet<string>;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  onReviewPull?: () => void;
  onChooseAgentProfile?: () => void;
  continuityActionBusy?: boolean;
  reviewScaffold?: ReviewScaffold;
  viewSwitch?: ReactNode;
  /** Starts a new session of a bound agent in the project. Only the
   *  standing list offers it; a review never does. */
  onStartSession?: (provider: ProviderKey, surface: NewSessionSurface) => void;
  /** A start is running. */
  startSessionBusy?: boolean;
}

/** A Push review's Git history against the history storage holds. */
export interface PushGitHistory {
  /** Commits storage lacks. The rail marks them ↑ and keeps them even when
   *  no session sits under them. Empty when storage shares no commit. */
  outgoing: ReadonlySet<string>;
  /** Storage's head left this branch, so the marks take the warning tone. */
  rewritten: boolean;
  /** The Push sends Git history even with no session change. */
  changed: boolean;
}

/** A review list is loading until both of its reads have settled. */
export type ReviewListState = "loading" | "ready" | "failed";

/** The rows a review will land, as its plan or context counts them. */
export interface ReviewScaffold {
  count: number;
  /** How many of them land under "Only in …". */
  storageOnly: number;
}

interface BranchPickerProps {
  branches: GitBranchSummary[];
  selectedBranch: string;
  disabled: boolean;
  onChange: (branch: string) => void;
}

export function BranchPicker({ branches, selectedBranch, disabled, onChange }: BranchPickerProps) {
  const { t } = useTranslation("projects");
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
  const label = selected.is_current
    ? t("branch.labelCurrent", { name: selected.name })
    : t("branch.label", { name: selected.name });
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
        title={t("branch.choose")}
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
        aria-label={t("branch.menu")}
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
          <span>{t("branch.heading")}</span>
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
              {branch.is_current && <span className="sync-history-branch-state current">{t("branch.current")}</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function formatDate(value?: number | null): string {
  if (!value) return t("projects:notRecorded");
  // Shared with the session detail page, so a row and the page it opens
  // stamp the same moment the same way.
  return formatSessionDetailTimestamp(value);
}

function formatCount(value?: number | null): string {
  if (value == null) return t("projects:notReported");
  return formatCompactNumber(value);
}

/* Day precision, for the two places that name a boundary rather than a moment:
   the window a page covers, and the day a step backwards lands on. A clock
   time on either would claim the edge falls at that exact second. */
const dayFormatters = new Map<string, Intl.DateTimeFormat>();
function dayFormatter(): Intl.DateTimeFormat {
  const locale = uiLocale();
  let formatter = dayFormatters.get(locale ?? "");
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, {
      month: "short",
      day: "numeric",
      year: "numeric",
    });
    dayFormatters.set(locale ?? "", formatter);
  }
  return formatter;
}

function formatDay(seconds: number): string {
  return dayFormatter().format(new Date(seconds * 1_000));
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
  const formatter = dayFormatter() as RangeCapableFormat;
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
  if (older.commits > 0) parts.push(t("projects:rail.commits", { count: older.commits }));
  if (older.sessions > 0) parts.push(t("projects:rail.sessions", { count: older.sessions }));
  const day = formatDay(older.newest_at);
  return parts.length
    ? t("projects:rail.loadEarlierTitleCounts", { day, counts: joinAnd(parts) })
    : t("projects:rail.loadEarlierTitle", { day });
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
  const { t } = useTranslation(["projects", "common"]);
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
          title={t("rail.showLessTitle")}
          onClick={onShowLess}
        >
          <strong>{t("rail.showLess")}</strong>
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
          <strong>{busy ? t("common:loading") : t("rail.loadEarlier")}</strong>
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
        ?? syncStatusDetail(entry)
        ?? t("projects:status.waiting"),
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
        ?? syncStatusDetail(entry)
        ?? t("projects:status.unmatched"),
    };
  }
  const blocked = (mode: "push" | "pull"): ComparisonPresentation => ({
    status: "unavailable",
    className: "unavailable",
    label: syncHoldDetail(mode, entry) ?? t("projects:status.threadBlocked"),
  });
  switch (entry.state) {
    case "synced":
      /* The message tail is current, but metadata or export settings await
         publication. The next Push updates the stored copy. */
      if (entry.push_decision?.action === "publish") {
        return {
          status: "local-outstanding",
          className: "local",
          label: t(entry.status_detail_code === "local_metadata_changed"
            ? "projects:status.metadataPending"
            : "projects:status.republish", { storage: storageName }),
        };
      }
      return { status: "synced", className: "synced", label: syncStatusDetail(entry) ?? t("projects:status.synced", { storage: storageName }) };
    case "local_only":
      if (entry.push_decision?.action === "blocked") return blocked("push");
      return { status: "local-outstanding", className: "local", label: syncStatusDetail(entry) ?? t("projects:status.localOnly", { storage: storageName }) };
    case "local_ahead":
      if (entry.push_decision?.action === "blocked") return blocked("push");
      return { status: "local-outstanding", className: "local", label: syncStatusDetail(entry) ?? t("projects:status.localAhead", { storage: storageName }) };
    case "storage_only":
      if (entry.pull_decision?.action === "blocked") return blocked("pull");
      return { status: "cloud-outstanding", className: "storage", label: syncStatusDetail(entry) ?? t("projects:status.storageOnly", { storage: storageName }) };
    case "storage_ahead":
      if (entry.pull_decision?.action === "blocked") return blocked("pull");
      return { status: "cloud-outstanding", className: "storage", label: syncStatusDetail(entry) ?? t("projects:status.storageAhead", { storage: storageName }) };
    case "diverged":
      return { status: "divergent", className: "diverged", label: t("projects:status.diverged", { storage: storageName }) };
    case "unavailable": {
      const name = entry.display_name.trim() || entry.provider_thread_id || entry.thread_id;
      return {
        status: "unavailable",
        className: "unavailable",
        label: t("projects:status.cannotSync", { name, detail: syncStatusDetail(entry) ?? t("projects:status.unreadable") }),
      };
    }
    case "unknown":
    default:
      return {
        status: "conflict",
        className: "conflict",
        label: t("projects:status.needsAttention", { detail: syncStatusDetail(entry) ?? t("projects:status.ancestryUnproven") }),
      };
  }
}

/** The one mapping from a row's badge to its scope. The path bar's tallies and
 *  the list's scope bar both read it, so they cannot sort a row differently. */
export function sessionStatusScope(status: ThreadStatusIconName): StatusScope {
  switch (status) {
    case "local-outstanding": return "push";
    case "cloud-outstanding": return "pull";
    case "divergent":
    case "pending":
    case "conflict":
    case "unavailable": return "attention";
    case "synced": return "synced";
    case "untracked":
    case "checking":
    case "unknown": return "unknown";
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


export function activityHeaderCounts(
  comparison: ThreadSyncComparison | null,
  storageName: string,
): ActivityHeaderCounts {
  let push = 0;
  let pull = 0;
  let attention = 0;
  let review = 0;
  let waiting = 0;
  let conflict = 0;
  let unsyncable = 0;
  const unsyncableDetails: string[] = [];

  for (const entry of comparison?.entries ?? []) {
    // An entry present on neither side is a ghost of a thread that has been
    // removed from both; it is nobody's work.
    if (!entry.local_present && !entry.storage_present) continue;
    const presentation = sessionSyncPresentation(entry, storageName);
    const scope = sessionStatusScope(presentation.status);
    if (scope === "push") push += 1;
    else if (scope === "pull") pull += 1;
    else if (scope === "attention") {
      attention += 1;
      // The breakdown the chip's tooltip spells out.
      switch (presentation.status) {
        case "divergent": review += 1; break;
        case "pending": waiting += 1; break;
        case "conflict": conflict += 1; break;
        case "unavailable":
          unsyncable += 1;
          if (unsyncableDetails.length < 3) unsyncableDetails.push(presentation.label);
          break;
        default: break;
      }
    }
  }

  const summary = ([
    review > 0 ? t("projects:header.changedBoth", { count: review }) : null,
    waiting > 0 ? t("projects:header.waiting", { count: waiting }) : null,
    conflict > 0 ? t("projects:header.unmatched", { count: conflict }) : null,
  ] as (string | null)[]).filter((phrase): phrase is string => phrase !== null).join(t("projects:header.listSeparator"));
  const detail = ([
    summary ? t("projects:header.summary", { summary }) : null,
    ...unsyncableDetails,
    unsyncable > unsyncableDetails.length
      ? t("projects:header.andMore", { more: unsyncable - unsyncableDetails.length })
      : null,
  ] as (string | null)[]).filter((phrase): phrase is string => phrase !== null).join(" ");

  return {
    push,
    pull,
    attention,
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

/** The badge a standing row draws: a machine-local sync block first, then the
 *  comparison's verdict, then the page's own no-verdict state. The scope bar
 *  sorts rows through this same function, so a row's scope is the badge it
 *  wears. */
function sessionRowPresentation(
  session: ProjectSessionSummary,
  entry: ThreadSyncEntry | undefined,
  storageName: string | null | undefined,
  fallback: SessionSyncPresentation,
): SessionSyncPresentation {
  if (session.sync_blocked_reason) {
    return {
      status: "unavailable",
      className: "unavailable",
      label: t("projects:status.unavailableWithReason", { reason: session.sync_blocked_reason }),
    };
  }
  return entry && storageName ? sessionSyncPresentation(entry, storageName) : fallback;
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
      label: storageName != null
        ? t("projects:status.comparing", { storage: storageName })
        : t("projects:status.comparingUnnamed"),
    };
  }
  if (comparisonError) {
    return {
      status: "unknown",
      className: "unknown",
      label: t("projects:status.notCompared", { error: comparisonError }),
    };
  }
  if (!storageName) {
    return {
      status: "untracked",
      className: "untracked",
      label: t("projects:status.noStorage"),
    };
  }
  if (!hasBoundProvider) {
    return {
      status: "untracked",
      className: "untracked",
      label: t("projects:status.noProfile", { storage: storageName }),
    };
  }
  // A comparison exists and this row is not in it — a view-only session, or one
  // this Mallard version has no adapter for.
  return {
    status: "unknown",
    className: "unknown",
    label: comparisonLoaded
      ? t("projects:status.notComparable", { storage: storageName })
      : t("projects:status.notComparedYet", { storage: storageName }),
  };
}

/* One absolute stamp, pushed to the far edge so it forms a column down the
   list. It anchors on the last activity because that is the more useful end of
   the interval; the start is that minus the duration sitting beside the other
   readouts. Both ends, and the last input between them — the time that places
   the session on its commit — are spelled out in full in the label. */
function SessionTimestamp({ session }: { session: SessionRowMetrics }) {
  const { t } = useTranslation("projects");
  const started = formatFullDateTime(session.started_at);
  const ended = formatFullDateTime(session.ended_at);
  const input = session.last_prompt_at == null ? null : formatFullDateTime(session.last_prompt_at);
  const tooltip = input == null
    ? t("metrics.timesTooltip", { started, ended })
    : t("metrics.timesTooltipInput", { started, input, ended });
  const label = input == null
    ? t("metrics.timesLabel", { started, ended })
    : t("metrics.timesLabelInput", { started, input, ended });
  return (
    <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={tooltip} aria-label={label} tabIndex={0}>
      <time dateTime={new Date(session.ended_at * 1_000).toISOString()}>{formatFullDateTime(session.ended_at)}</time>
    </span>
  );
}

/** Readouts only — everything in here is hovered, nothing is clicked. The
 *  card renders the stamp separately so it can trail the row's buttons; the
 *  standalone renders keep it attached. */
/** The size a review row prints beside its metrics: what a push of the
 *  session captures, or what a pull of it installs. Drawn in a review only,
 *  where the limit makes the number mean something. */
export interface ReviewSizeReadout {
  bytes: number;
  direction: "push" | "pull";
}

function ReviewSizeMetric({ readout }: { readout: ReviewSizeReadout }) {
  const { t } = useTranslation("projects");
  const size = formatBytes(readout.bytes);
  const tooltip = readout.direction === "push" ? t("metrics.pushSizeTooltip", { size }) : t("metrics.pullSizeTooltip", { size });
  const label = readout.direction === "push" ? t("metrics.pushSizeLabel", { size }) : t("metrics.pullSizeLabel", { size });
  return (
    <span className="sync-history-thread-metric" data-tooltip={tooltip} aria-label={label} tabIndex={0}>
      <Icon name="drive" size={12} /><b>{formatBytes(readout.bytes)}</b>
    </span>
  );
}

/** The readouts a session row prints, as the summary fields that hold them.
 *  Activity rows pass the whole summary; a session's own page reports the
 *  same fields for the row of its Push review. */
export type SessionRowMetrics = Pick<
  ProjectSessionSummary,
  | "started_at"
  | "ended_at"
  | "last_prompt_at"
  | "user_round_count"
  | "agent_message_count"
  | "tool_call_count"
  | "total_input_tokens"
  | "total_output_tokens"
  | "total_tokens"
  | "metrics_complete"
>;

export function reviewSizeReadout(
  mode: "push" | "pull" | undefined,
  entry: ThreadSyncEntry | undefined,
): ReviewSizeReadout | null {
  if (mode === "push" && entry?.capture_bytes != null) return { bytes: entry.capture_bytes, direction: "push" };
  if (mode === "pull" && entry?.pull_bytes != null) return { bytes: entry.pull_bytes, direction: "pull" };
  return null;
}

export function SessionMetrics({
  session,
  id,
  showTimestamp = true,
  sizeReadout = null,
  subagents = null,
}: {
  session: SessionRowMetrics;
  id?: string;
  showTimestamp?: boolean;
  sizeReadout?: ReviewSizeReadout | null;
  /** The subagent transcripts a Push of this session carries with it. */
  subagents?: { count: number; bytes: number } | null;
}) {
  const { t } = useTranslation("projects");
  const durationText = formatDuration(session.started_at, session.ended_at);
  const durationLabel = durationText
    ? t("metrics.ranFor", { duration: durationText })
    : "";
  const totalTokens = formatCount(session.total_tokens);
  const totalInputTokens = formatCount(session.total_input_tokens);
  const totalOutputTokens = formatCount(session.total_output_tokens);
  /* A total with neither part beside it prints alone: "Not reported" twice
     would speak for a split the row was never given. */
  const tokenSplit = session.total_input_tokens != null || session.total_output_tokens != null;

  const prompts = session.user_round_count;
  const replies = session.agent_message_count;
  return <div id={id} className="sync-history-thread-meta" aria-label={t("metrics.label")}>
    {/* Ordered by what the row is about: the exchange first, then the work it
        did, then what it cost, then how long it took. */}
    {/* One message count sums prompts and final replies. The tooltip shows the
        split, one value per line. */}
    <span className="sync-history-thread-metric" data-tooltip={t("metrics.messagesTooltip", { prompts, replies })} aria-label={t("metrics.messagesLabel", { messages: prompts + replies, prompts, replies })} tabIndex={0}><SessionRowIcon name="message" /><b>{prompts + replies}</b></span>
    {session.tool_call_count > 0 && <span className="sync-history-thread-metric" data-tooltip={t("metrics.toolCallsTooltip", { calls: session.tool_call_count })} aria-label={t("metrics.toolCallsLabel", { calls: session.tool_call_count })} tabIndex={0}><SessionRowIcon name="tool" /><b>{session.tool_call_count}</b></span>}
    <span
      className="sync-history-thread-metric"
      data-tooltip={tokenSplit
        ? t("metrics.tokensSplitTooltip", { input: totalInputTokens, output: totalOutputTokens })
        : t("metrics.tokensTooltip", { total: totalTokens })}
      aria-label={tokenSplit
        ? t("metrics.tokensSplitLabel", { total: totalTokens, input: totalInputTokens, output: totalOutputTokens })
        : t("metrics.tokensLabel", { total: totalTokens })}
      tabIndex={0}
    ><SessionRowIcon name="token" /><b>{totalTokens}</b></span>
    {!session.metrics_complete && <span className="sync-history-thread-metric sync-history-partial" data-tooltip={t("metrics.partial")} aria-label={t("metrics.partial")} tabIndex={0}><Icon name="alert-triangle" size={12} /></span>}
    {durationText && (
      <span className="sync-history-thread-metric sync-history-thread-duration" data-tooltip={durationLabel} aria-label={durationLabel} tabIndex={0}>
        <SessionRowIcon name="clock" /><b>{durationText}</b>
      </span>
    )}
    {subagents && subagents.count > 0 && (
      <span className="sync-history-thread-metric" data-tooltip={t("metrics.subagentsTooltip", { subagents: subagents.count, size: formatBytes(subagents.bytes) })} aria-label={t("metrics.subagentsLabel", { subagents: subagents.count, size: formatBytes(subagents.bytes) })} tabIndex={0}>
        <Icon name="git-fork" size={12} /><b>{subagents.count}</b>
      </span>
    )}
    {sizeReadout && <ReviewSizeMetric readout={sizeReadout} />}
    {showTimestamp && <SessionTimestamp session={session} />}
  </div>;
}

/** Copy for rows the local machine can see but must not change. */
function sessionAccessNote(session: ProjectSessionSummary): { label: string; detail: string } | null {
  switch (session.access) {
    case "view_only":
      return {
        label: t("projects:access.viewOnly"),
        detail: t("projects:access.viewOnlyDetail", { provider: session.provider_display_name }),
      };
    case "unsupported":
      return {
        label: t("projects:access.viewOnly"),
        detail: t("projects:access.unsupportedDetail", { provider: session.provider_display_name }),
      };
    case "bound_unavailable":
      return {
        label: t("projects:access.unavailable"),
        detail: t("projects:access.unavailableDetail", { provider: session.provider_display_name }),
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
  onDeleteSession?: (session: ProjectSessionSummary) => void;
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
  /** The storage a stored row opens its page through. */
  storageId?: string | null;
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
  const notRecorded = t("projects:notRecorded");
  const line = (label: string, value: string) => t("projects:debug.line", { label, value });
  return [
    line(t("projects:debug.resourceId"), resourceId),
    line(t("projects:debug.threadId"), logicalThreadId ?? notRecorded),
    line(t("projects:debug.sessionId"), providerSessionId ?? notRecorded),
    line(t("projects:debug.transcript"), transcriptPath ?? notRecorded),
    line(t("projects:debug.workingDirectory"), workingDirectory),
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

/** Each copyable field's name, its button's title and its receipt. */
const SESSION_DEBUG_FIELDS: Record<Exclude<SessionDebugCopyTarget, "all">, {
  label: ParseKeys<"projects">;
  copy: ParseKeys<"projects">;
  copied: ParseKeys<"projects">;
}> = {
  "resource-id": { label: "debug.resourceId", copy: "debug.copyResourceId", copied: "debug.resourceIdCopied" },
  "logical-thread-id": { label: "debug.threadId", copy: "debug.copyThreadId", copied: "debug.threadIdCopied" },
  "provider-session-id": { label: "debug.sessionId", copy: "debug.copySessionId", copied: "debug.sessionIdCopied" },
  "transcript-path": { label: "debug.transcript", copy: "debug.copyTranscript", copied: "debug.transcriptCopied" },
  "working-directory": { label: "debug.workingDirectory", copy: "debug.copyWorkingDirectory", copied: "debug.workingDirectoryCopied" },
};

function SessionDebugMenu({
  resourceId,
  logicalThreadId,
  providerSessionId,
  transcriptPath,
  workingDirectory,
}: SessionDebugMenuProps) {
  const { t } = useTranslation(["projects", "common"]);
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState<SessionDebugCopyTarget | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const placement = useDebugPopoverPlacement(rootRef, panelRef, open, () => closeMenu());
  const menuId = `session-debug-${useId().replace(/:/g, "")}`;
  const rows: Array<{ key: Exclude<SessionDebugCopyTarget, "all">; value: string }> = [
    { key: "resource-id", value: resourceId },
    ...(logicalThreadId != null
      ? [{ key: "logical-thread-id" as const, value: logicalThreadId }]
      : []),
    ...(providerSessionId != null
      ? [{ key: "provider-session-id" as const, value: providerSessionId }]
      : []),
    ...(transcriptPath != null
      ? [{ key: "transcript-path" as const, value: transcriptPath }]
      : []),
    { key: "working-directory", value: workingDirectory },
  ];
  const copiedLabel = copied === "all"
    ? t("debug.allCopied")
    : copied
      ? t(SESSION_DEBUG_FIELDS[copied].copied)
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
      setCopyError(t("debug.copyFailed", { error: errorMessage(reason) }));
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
        title={t("debug.title")}
        aria-label={t("debug.show")}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={menuId}
        onClick={() => open ? closeMenu() : setOpen(true)}
      >
        <SessionRowIcon name="info" />
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
          aria-label={t("debug.title")}
          hidden={!open}
        >
          <header>
            <strong>{t("debug.title")}</strong>
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
                title={t(SESSION_DEBUG_FIELDS[row.key].copy)}
                onClick={() => void copyValue(row.key, row.value)}
              >
                <span>{t(SESSION_DEBUG_FIELDS[row.key].label)}</span>
                <code>{row.value}</code>
                <small className={copied === row.key ? "copied" : undefined}>
                  {copied === row.key ? t("common:copied") : t("common:copy")}
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
              {copied === "all" ? t("copiedAll") : t("copyAll")}
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
    ?? t("projects:pullFirst.detail");
  return (
    <button
      type="button"
      className="btn btn-ghost sync-sync-review-pull-action"
      disabled={disabled}
      title={detail}
      aria-label={t("projects:pullFirst.openWithDetail", { detail })}
      onClick={onReviewPull}
    >
      {t("projects:pullFirst.open")}
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
      case "materialize_cloud": return t("projects:pullChip.update");
      case "keep_both": return t("projects:pullChip.updateKeep");
      case "create_local": return t("projects:pullChip.add");
      case "adopt_cloud_metadata": return t("projects:pullChip.markSynced");
      default: return null;
    }
  }
  switch (entry.state) {
    case "storage_only": return t("projects:pullChip.add");
    case "storage_ahead": return t("projects:pullChip.update");
    case "diverged": return t("projects:pullChip.updateKeep");
    default: return null;
  }
}

function syncReviewStateLabel(
  mode: "push" | "pull",
  entry: ThreadSyncEntry | undefined,
  selected: boolean,
): string | null {
  if (!entry) return null;
  const notSelected = t("projects:reviewState.notSelected");
  if (entry.state === "unavailable") return t("projects:reviewState.unavailable");
  if (entry.compared_against_storage) {
    const decision = mode === "push" ? entry.push_decision : entry.pull_decision;
    switch (decision.action) {
      case "no_changes": return null;
      case "publish": return selected ? t("projects:reviewState.updateStorage") : notSelected;
      case "create_cloud": return selected ? t("projects:reviewState.publishNew") : notSelected;
      case "require_pull_metadata": return t("projects:reviewState.pullMetadata");
      case "require_pull": return t("projects:reviewState.pullRequired");
      case "not_pushable": return t("projects:reviewState.notHere");
      case "adopt_cloud_metadata": return selected ? t("projects:reviewState.markSynced") : notSelected;
      case "report_push_eligible": return t("projects:reviewState.readyToPush");
      case "materialize_cloud": return selected ? t("projects:reviewState.updateTask") : t("projects:reviewState.keepLocal");
      case "keep_both": return selected ? t("projects:reviewState.keepBoth") : notSelected;
      case "create_local": return selected ? t("projects:reviewState.createLocal") : notSelected;
      case "report_local_only": return t("projects:reviewState.keepLocal");
      case "not_found": return t("projects:reviewState.notFound");
      case "blocked": return t("projects:reviewState.cantSync");
    }
  }
  if (mode === "push") {
    if (entry.state === "storage_only" || entry.state === "storage_ahead" || entry.state === "diverged") {
      return t("projects:reviewState.pullRequired");
    }
    return entry.state === "unknown" ? t("projects:reviewState.reviewStatus") : null;
  }
  if (entry.state === "diverged") return selected ? t("projects:reviewState.keepBoth") : notSelected;
  if (entry.state === "storage_only" || entry.state === "storage_ahead") {
    return selected ? t("projects:reviewState.restore") : t("projects:reviewState.keepLocal");
  }
  if (entry.state === "local_only" || entry.state === "local_ahead") return t("projects:reviewState.keepLocal");
  return entry.state === "synced" ? null : t("projects:reviewState.reviewStatus");
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
  const provider = entry.provider_display_name;
  if (entry.state === "unavailable") {
    return syncStatusDetail(entry) ?? t("projects:status.unreadable");
  }
  const decision = mode === "push" ? entry.push_decision : entry.pull_decision;
  if (!decision) return null;
  if (mode === "push" && entry.state === "synced" && decision.action === "publish") {
    return syncStatusDetail(entry) ?? t("projects:hold.exportChanged");
  }
  if (mode === "pull" && decision.action === "keep_both" && decision.blocked_reason) {
    return syncDecisionReason(decision) ?? null;
  }
  if (decision.action === "blocked") {
    switch (decision.blocker) {
      case "codec_unsupported":
        return mode === "push"
          ? t("projects:hold.captureUnsupported", { provider })
          : t("projects:hold.restoreUnsupported");
      case "ambiguous_binding":
        return t("projects:hold.ambiguous");
      case "missing_binding":
        return t("projects:hold.unlinked");
      case "busy":
        return t("projects:hold.busy", { provider });
      case "detached_requires_pull":
        return t("projects:hold.detached");
      case "unknown_evidence":
        return t("projects:hold.unknownEvidence");
      default:
        return t("projects:hold.blocked");
    }
  }
  if (mode === "push" && decision.action === "require_pull_metadata") {
    return t("projects:hold.metadataNewer");
  }
  if (mode === "push" && decision.action === "require_pull") {
    return entry.state === "diverged"
      ? t("projects:hold.diverged")
      : t("projects:hold.storageNewer");
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
    label: detail ? t("projects:reviewState.withDetail", { label, detail }) : label,
  }} />;
}

/** A session action's label in the interface language, by its id. */
function actionLabel(action: SessionActionDescriptor): string {
  return backendText(`sessionAction.${action.action_id}`, action.label);
}

/** Disabled reason wins over the busy phrase, which wins over the plain label. */
function actionTitle(action: SessionActionDescriptor, busy = false): string {
  if (!action.enabled && action.disabled_reason != null) {
    return backendText(`sessionAction.${action.action_id}.disabledReason`, action.disabled_reason);
  }
  if (!action.enabled) return actionLabel(action);
  return busy ? `${actionLabel(action)}…` : actionLabel(action);
}

function ChildOmissionIcon() {
  const { t } = useTranslation("projects");
  const tooltipId = useId();
  return (
    <span className="sync-child-omission" tabIndex={0} aria-label={t("child.label")} aria-describedby={tooltipId}>
      <Icon name="info" size={14} />
      <span id={tooltipId} role="tooltip" className="sync-child-omission-tooltip">
        {t("child.detail")}
      </span>
    </span>
  );
}

function SessionCard({
  session, occurrenceKey, workingDirectory, busy = false, busyActionId = null,
  onSessionAction, onDeleteSession, onOpenSessionDetail,
  syncEntry, reviewResourceId, storageName, storageId = null, syncFallback, selectionMode, selected = false, selectable = false,
  selectionDisabled = false, onToggleResource, onReviewPull,
}: SessionCardProps) {
  const { t } = useTranslation("projects");
  const fullTitle = session.title || t("untitledSession");
  const visibleTitle = compactThreadTitle(fullTitle);
  const reviewStateLabel = selectionMode ? syncReviewStateLabel(selectionMode, syncEntry, selected) : null;
  const holdDetail = selectionMode && syncEntry ? syncHoldDetail(selectionMode, syncEntry) : null;
  // The status badge owns row state; the branch-unmapped heading owns its pencil.
  const accessNote = sessionAccessNote(session);
  // A stored row opens through the storage the list shows: reading its copy
  // needs an adapter, not a provider profile. A local row needs its profile.
  const stored = session.source === "remote_only";
  const canOpenDetails = !selectionMode
    && onOpenSessionDetail != null
    && (stored
      ? storageId != null && session.access !== "unsupported"
      : session.access === "bound_ready");
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
      ...(stored && storageId ? { storageId } : {}),
    }, trigger);
  };
  return (
    <article className={`sync-history-thread-card${selectionMode && selectable ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectionMode && selectable && (
            <label
              className="sync-sync-row-choice"
              title={session.title
                ? t(selected ? "select.exclude" : "select.include", { name: session.title })
                : t(selected ? "select.excludeThis" : "select.includeThis")}
            >
              <input
                type="checkbox"
                checked={selected}
                disabled={selectionDisabled}
                aria-label={t(selected ? "select.exclude" : "select.include", { name: session.title || t("untitledSession") })}
                onChange={() => onToggleResource?.(reviewResourceId)}
              />
            </label>
          )}
          {!session.sync_blocked_reason && selectionMode && reviewStateLabel && syncEntry ? (
            <ReviewSyncIndicator
              entry={syncEntry}
              storageName={storageName ?? t("storageFallback")}
              label={reviewStateLabel}
              detail={holdDetail}
            />
          ) : (
            <SessionSyncIndicator
              presentation={sessionRowPresentation(session, syncEntry, storageName, syncFallback)}
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
              title={visibleTitle === fullTitle ? t("row.viewDetails") : t("row.viewDetailsTitled", { title: fullTitle })}
              onClick={(clickEvent) => openSessionDetail(clickEvent.currentTarget)}
            >
              {visibleTitle}
            </button>
          ) : (
            <strong title={visibleTitle === fullTitle ? undefined : fullTitle}>{visibleTitle}</strong>
          )}
          {session.sync_blocked_reason && (
            <span className="sync-history-branch-state history-only" title={session.sync_blocked_reason}>
              {t("reviewState.unavailable")}
            </span>
          )}
          {selectionMode === "push" && syncEntry?.subagent_selection?.omitted && <ChildOmissionIcon />}
          {session.preserved && (
            <span
              className="sync-history-preserved-badge"
              title={t("row.preservedTitle", { thread: session.preserved.preserved_from.slice(0, 8) })}
            >
              {t("row.preserved", { ordinal: session.preserved.ordinal })}
            </span>
          )}
          {selectionMode === "push" && syncEntry?.state === "synced" && syncEntry.push_decision.action === "publish" && (
            <span className="sync-history-branch-state history-only">{syncStatusDetail(syncEntry) ?? t("hold.exportChanged")}</span>
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
              aria-label={actionLabel(openAction)}
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
              title={t("row.providerSession", { provider: session.provider_display_name })}
              aria-label={t("row.providerSession", { provider: session.provider_display_name })}
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
                aria-label={actionLabel(action)}
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
          {session.sync_blocked_reason ? (
            <span className="sync-history-thread-metric sync-history-partial">{t("metrics.unavailable")}</span>
          ) : (
            <SessionMetrics
              session={session}
              showTimestamp={false}
              sizeReadout={reviewSizeReadout(selectionMode, syncEntry)}
            />
          )}
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            {!selectionMode && session.source !== "remote_only" && session.access === "bound_ready"
              && (session.provider === "codex" || session.provider === "claude")
              && session.provider_session_id && session.transcript_path && onDeleteSession && (
              <button
                type="button"
                className="btn btn-ghost sync-history-session-detail-action"
                aria-label={t("row.delete")}
                title={t("row.deleteTitle")}
                disabled={busy}
                onClick={() => onDeleteSession(session)}
              >
                <Icon name={busyActionId === "delete" && busy ? "refresh" : "trash"} size={14}
                  className={busyActionId === "delete" && busy ? "icon-spin" : undefined} />
              </button>
            )}
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
                aria-label={t("row.viewDetails")}
                title={t("row.viewDetails")}
                onClick={(clickEvent) => openSessionDetail(clickEvent.currentTarget)}
              >
                <SessionRowIcon name="chevron-right" />
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
  const { t } = useTranslation("projects");
  const fullTitle = entry.display_name.trim() && entry.display_name !== entry.thread_id
    ? entry.display_name
    : t("untitledThread");
  // Local threads already drop the provenance timestamp their title carries;
  // stored threads get the same treatment so both lists read alike.
  const title = compactThreadTitle(fullTitle);
  const identityLabel = t("row.threadIdentity", { title, provider: entry.provider_display_name, id: entry.thread_id });
  const updatedAt = entry.storage_updated_at ?? entry.local_updated_at;
  const sizeReadout = reviewSizeReadout(selectionMode, entry);
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
            <label className="sync-sync-row-choice" title={t(selected ? "select.exclude" : "select.include", { name: identityLabel })}>
              <input
                type="checkbox"
                checked={selected}
                disabled={selectionDisabled}
                aria-label={t(selected ? "select.exclude" : "select.include", { name: identityLabel })}
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
            title={t("row.providerThread", { provider: entry.provider_display_name })}
            aria-label={t("row.providerThread", { provider: entry.provider_display_name })}
          >
            <ProviderMark provider={entry.provider} displayName={entry.provider_display_name} />
          </span>
          <strong title={title === fullTitle ? undefined : fullTitle}>{title}</strong>
          {selectionMode === "push" && entry.state === "synced" && entry.push_decision.action === "publish" && (
            <span className="sync-history-branch-state history-only">{syncStatusDetail(entry) ?? t("hold.exportChanged")}</span>
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
        {sizeReadout && <ReviewSizeMetric readout={sizeReadout} />}
        {updatedAt != null && (
          <time dateTime={new Date(updatedAt * 1_000).toISOString()}>{formatDate(updatedAt)}</time>
        )}
      </span>
    </article>
  );
}

/** What a session's own page knows about it, for the row of its Push review.
 *  The review's comparison carries the rest: status, provider, size. */
export interface PushSessionFacts {
  title: string;
  metrics: SessionRowMetrics;
}

/** The Git history a session's Push to Mallard Cloud carries: the branch, and
 *  what it adds to the history storage holds. */
export interface SessionPushHistory {
  branch: string;
  outgoing: GitHistoryOutgoing;
}

/** The ↑N pill's words: the rail's commit chip sentence, with the count and
 *  the branch, since the commits themselves are not on this review. */
export function sessionPushHistoryTitle(storageName: string, history: SessionPushHistory): string {
  const head = t("projects:pushHistory.head", {
    count: history.outgoing.new_commits,
    storage: storageName,
    branch: history.branch,
  });
  return history.outgoing.rewritten
    ? `${head}\n${t("projects:pushHistory.rewritten", { storage: storageName })}`
    : head;
}

/** The one row of a session's own Push review: the review's session row with
 *  nothing to select and nothing to open. `facts` is null when the session's
 *  page had not loaded; the comparison entry alone still names and sizes it.
 *  `history` marks the commits the Push adds to the Cloud's Git history with
 *  the rail's ↑ chip, holding its count; none when it adds none, or when
 *  storage holds no history this branch shares, as on the rail. */
export function SessionPushRow({ entry, storageName, facts, history = null }: {
  entry: ThreadSyncEntry;
  storageName: string;
  facts: PushSessionFacts | null;
  history?: SessionPushHistory | null;
}) {
  const { t } = useTranslation("projects");
  const historyTitle = history && history.outgoing.new_commits > 0
    ? sessionPushHistoryTitle(storageName, history)
    : null;
  const fullTitle = facts?.title
    ?? (entry.display_name.trim() && entry.display_name !== entry.thread_id ? entry.display_name : t("untitledSession"));
  const title = compactThreadTitle(fullTitle);
  const reviewStateLabel = syncReviewStateLabel("push", entry, true);
  const sizeReadout = reviewSizeReadout("push", entry);
  const subagents = {
    count: entry.subagent_selection.keys.length,
    bytes: entry.subagent_selection.estimated_bytes,
  };
  return (
    <article className="sync-history-thread-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {reviewStateLabel ? (
            <ReviewSyncIndicator
              entry={entry}
              storageName={storageName}
              label={reviewStateLabel}
              detail={syncHoldDetail("push", entry)}
            />
          ) : (
            <SessionSyncIndicator presentation={sessionSyncPresentation(entry, storageName)} />
          )}
          <strong title={title === fullTitle ? undefined : fullTitle}>{title}</strong>
          {historyTitle && history && (
            <span
              className={`sync-history-branch-state sync-history-outgoing${history.outgoing.rewritten ? " history-only" : ""}`}
              title={historyTitle}
              aria-label={historyTitle}
              tabIndex={0}
            >↑{history.outgoing.new_commits}</span>
          )}
          {entry.subagent_selection.omitted && <ChildOmissionIcon />}
          {entry.state === "synced" && entry.push_decision.action === "publish" && (
            <span className="sync-history-branch-state history-only">{syncStatusDetail(entry) ?? t("hold.exportChanged")}</span>
          )}
        </div>
        {/* The session's page already opens it elsewhere, so the mark is the
            row's provider identity and nothing more. */}
        <div className="sync-history-thread-actions">
          <span
            className="sync-history-launch-action sync-history-provider-inert"
            title={t("row.providerSession", { provider: entry.provider_display_name })}
            aria-label={t("row.providerSession", { provider: entry.provider_display_name })}
          >
            <ProviderMark provider={entry.provider} displayName={entry.provider_display_name} />
          </span>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          {facts ? (
            <>
              <SessionMetrics session={facts.metrics} showTimestamp={false} sizeReadout={sizeReadout} subagents={subagents} />
              <SessionTimestamp session={facts.metrics} />
            </>
          ) : (
            <>
              <div className="sync-history-thread-meta" aria-label={t("metrics.label")}>
                {sizeReadout && <ReviewSizeMetric readout={sizeReadout} />}
              </div>
              {entry.local_updated_at != null && (
                <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end">
                  <time dateTime={new Date(entry.local_updated_at * 1_000).toISOString()}>{formatFullDateTime(entry.local_updated_at)}</time>
                </span>
              )}
            </>
          )}
        </div>
      </div>
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
      return t("projects:notice.viewOnly", { provider: state.display_name });
    case "unsupported":
      return t("projects:access.unsupportedDetail", { provider: state.display_name });
    case "bound_unavailable":
      return t("projects:notice.unreadable", { provider: state.display_name });
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
  project, binding, activity, loading, loadingMore, loadError, onRetryLoad, actionBusyResourceId, actionBusyActionId = null, onShowLess, canShowLess,
  onLoadMore,
  onSessionAction, onDeleteSession, onOpenSessionDetail, comparison = null, storageId = null,
  storageLoading = false, comparisonLoading = false, comparisonError = null, comparisonErrorHandled = false, activeStorageName = null,
  selectionMode, pushGitHistory, selectedResourceIds = new Set(), selectableResourceIds, reviewResourceIds,
  selectionDisabled = false, onToggleResource, onReviewPull,
  onChooseAgentProfile,
  continuityActionBusy = false,
  searchQuery, onSearchQueryChange, onSearchResultsChange,
  statusScope: hostStatusScope, onStatusScopeChange,
  reviewScaffold,
  viewSwitch,
  onStartSession,
  startSessionBusy = false,
}: ContentProps) {
  const { t, i18n: { language } } = useTranslation(["projects", "common"]);
  const [localQuery, setLocalQuery] = useState("");
  const [localStatusScope, setLocalStatusScope] = useState<StatusScope | null>(null);
  const listRef = useRef<HTMLElement>(null);
  /* A review's list is keyed on settled reads, not on request flags: every
     flag starts false, so a flag-keyed branch painted nothing on the first
     commit, and the rows are filtered on the comparison, so the list was
     empty between the two reads. Each caption names the read still out. */
  const reviewLoadingLabel = !selectionMode
    ? null
    : activity === null && loadError === null
      ? t("list.loadingSessions")
      : comparison === null && comparisonError === null
        ? t("list.comparingComputer")
        : null;
  const comparisonStorageId = comparison?.storage_id ?? "";
  const dismissedClearedNotice = useDismissedStorageWarning(project.local_project_id, comparisonStorageId);
  const query = searchQuery ?? localQuery;
  const searching = query.trim().length > 0;
  const [collapsedCommitShas, setCollapsedCommitShas] = useState<Set<string>>(new Set());
  const [branchUnmappedFolded, setBranchUnmappedCollapsed] = useState(false);
  const [storageOnlyFolded, setStorageOnlyCollapsed] = useState(false);
  const label = projectLabel(project);
  const allSessions = useMemo(
    () => [...(activity?.sessions ?? [])].sort(compareSessions),
    [activity?.sessions],
  );
  const syncIndex = useMemo(
    () => indexSessionSyncEntries(comparison?.entries ?? []),
    [comparison?.entries],
  );
  const reviewSessions = useMemo(() => {
    if (!selectionMode) return allSessions;
    return allSessions.filter((session) => {
      const entry = syncEntryForSession(session, syncIndex);
      if (!entry || reviewResourceIds?.has(entry.resource_id) === false) return false;
      const status = sessionSyncPresentation(entry, "storage").status;
      return selectionMode === "pull"
        ? isPullReviewRow(status, entry.pull_decision)
        : isPushReviewRow(status, entry.push_decision);
    });
  }, [allSessions, reviewResourceIds, selectionMode, syncIndex]);
  const comparisonStorageName = comparison?.storage_name ?? activeStorageName ?? t("selectedStorage");
  /* What every row's badge is resolved against, settled before the list is
     narrowed: the scope bar sorts rows by the badge they draw. */
  const rowStorageName = comparison ? comparisonStorageName : activeStorageName;
  const hasBoundProvider = Object.keys(binding?.profile_ids ?? {}).length > 0;
  // Resolved once for the page: every row without a comparison entry draws this
  // instead of drawing nothing.
  const syncFallback = useMemo(() => unresolvedSyncPresentation({
    comparisonLoaded: comparison != null,
    comparisonLoading,
    comparisonError,
    storageName: comparison?.storage_name ?? activeStorageName,
    hasBoundProvider,
  }), [comparison, comparisonLoading, comparisonError, activeStorageName, hasBoundProvider, language]);
  const scopeBySession = useMemo(() => new Map(reviewSessions.map((session) => [
    session.resource_id,
    sessionStatusScope(sessionRowPresentation(
      session, syncEntryForSession(session, syncIndex), rowStorageName, syncFallback,
    ).status),
  ])), [reviewSessions, syncIndex, rowStorageName, syncFallback]);
  const scopeCounts = useMemo(() => {
    const counts = new Map<StatusScope, number>();
    for (const scope of scopeBySession.values()) counts.set(scope, (counts.get(scope) ?? 0) + 1);
    return counts;
  }, [scopeBySession]);
  const chosenStatusScope = hostStatusScope === undefined ? localStatusScope : hostStatusScope;
  const chooseStatusScope = onStatusScopeChange ?? setLocalStatusScope;
  /* The scope bar is drawn when it can narrow something: on the standing
     list, over a settled comparison, with rows in two scopes — or while a
     scope is chosen, so it can be released. A review's rows are one verb's by
     construction. A scope chosen before the bar went away is not applied
     behind the reader's back. */
  const scopeBarShown = !selectionMode && comparison != null
    && (scopeCounts.size > 1 || chosenStatusScope !== null);
  const statusScope = scopeBarShown ? chosenStatusScope : null;
  /* A name and a scope narrow the list alike: folds open, a commit without a
     matching session leaves the rail, and the count reads "3 of 12". */
  const narrowing = searching || statusScope !== null;
  const branchUnmappedCollapsed = !narrowing && branchUnmappedFolded;
  const storageOnlyCollapsed = !narrowing && storageOnlyFolded;
  const sessions = useMemo(
    () => reviewSessions.filter((session) => (
      (statusScope === null || scopeBySession.get(session.resource_id) === statusScope)
      && nameMatchesSearch(session.title || t("untitledSession"), query)
    )),
    [reviewSessions, query, statusScope, scopeBySession, t, language],
  );
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
  // Resolved the same way the rows' status badges resolve it, so the group
  // heading and the tooltips under it can never name two different storages.
  const storageOnlyName = rowStorageName ?? t("storageFallback");
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
  const renderedSyncEntryIds = useMemo(() => {
    // Use the unfiltered projection here. Search must not turn an excluded
    // activity row into a comparison-only row with another name.
    const representedIds = activity?.git ? new Set([
      ...activity.git.commits.flatMap((commit) => commit.session_refs.map((ref) => ref.resource_id)),
      ...(activity.unmapped ?? []).map((ref) => ref.resource_id),
    ]) : new Set(reviewSessions.map((session) => session.resource_id));
    return new Set(reviewSessions.flatMap((session) => {
      const entry = syncEntryForSession(session, syncIndex);
      return representedIds.has(session.resource_id) && entry ? [entry.resource_id] : [];
    }));
  }, [activity, reviewSessions, syncIndex]);
  /* The non-git flat list's own split of the same fact, session-typed rather
     than reference-typed because that list renders sessions directly. */
  const cloudSessions = sessions.filter((session) => session.source === "remote_only");
  const localSessions = sessions.filter((session) => session.source !== "remote_only");
  const compactSelectionReview = selectionMode != null;
  /* Review rows the activity scan has no session row for. A review loads the
     whole span at once, so this is only what no activity row can ever reach;
     a review must still offer it or it would drop candidates silently. It
     renders as plain rows at the foot of the same list — never a box. */
  const allUnlistedEntries = useMemo(() => {
    if (!selectionMode) return [];
    return (comparison?.entries ?? []).filter((entry) => (
      (entry.local_present || entry.storage_present)
      && reviewResourceIds?.has(entry.resource_id) !== false
      && (selectionMode === "pull"
        ? isPullReviewRow(sessionSyncPresentation(entry, "storage").status, entry.pull_decision)
        : isPushReviewRow(sessionSyncPresentation(entry, "storage").status, entry.push_decision))
      && !renderedSyncEntryIds.has(entry.resource_id)
    ));
  }, [comparison?.entries, reviewResourceIds, selectionMode, renderedSyncEntryIds]);
  const unlistedEntries = useMemo(() => allUnlistedEntries.filter((entry) => (
    nameMatchesSearch(entry.display_name.trim() && entry.display_name !== entry.thread_id
      ? entry.display_name : t("untitledThread"), query)
  )), [allUnlistedEntries, query, t, language]);
  const searchResults = useMemo<NameSearchResults>(() => ({
    query,
    resourceIds: new Set([
      ...sessions.map((session) => syncEntryForSession(session, syncIndex)?.resource_id ?? session.resource_id),
      ...unlistedEntries.map((entry) => entry.resource_id),
    ]),
  }), [query, sessions, syncIndex, unlistedEntries]);
  useEffect(() => { onSearchResultsChange?.(searchResults); }, [onSearchResultsChange, searchResults]);
  const searchableIds = new Set([
    ...reviewSessions.map((session) => syncEntryForSession(session, syncIndex)?.resource_id ?? session.resource_id),
    ...allUnlistedEntries.map((entry) => entry.resource_id),
  ]);
  const hiddenSelected = searching ? [...searchableIds].filter((id) => (
    selectedResourceIds.has(id) && !searchResults.resourceIds.has(id)
  )).length : 0;
  /* Releasing the scope brings rows back only if the name alone matches some. */
  const scopeEmptiedList = statusScope !== null && searchResults.resourceIds.size === 0
    && reviewSessions.some((session) => nameMatchesSearch(session.title || t("untitledSession"), query));
  const sectionTitle = activity?.git ? t("list.gitHistory") : activity ? t("list.sessions") : t("list.activity");
  const windowRange = activity ? formatDayRange(activity.window_start, activity.window_end) : "";
  /* An empty rail either belongs to an unborn branch or to a loaded range
     without commits. Branch choices come only from the local repository. */
  const emptyRail = useMemo(() => {
    if ((activity?.older?.commits ?? 0) === 0) {
      return { icon: "git-commit" as IconName, title: t("list.noCommitsYet"), meta: null };
    }
    return { icon: "range" as IconName, title: t("list.noCommits"), meta: windowRange };
  }, [activity?.older?.commits, windowRange, t, language]);
  const refreshStatus = storageLoading
    ? t("list.loadingCloudStatus")
    : comparisonLoading
      ? (activeStorageName != null
        ? t("list.comparingThreads", { storage: activeStorageName })
        : t("list.comparingThreadsUnnamed"))
      : loading
        ? t("list.refreshing")
        : null;
  const renderSession = (session: ProjectSessionSummary, key: string) => {
    const entry = syncBySessionResource.get(session.resource_id);
    const reviewResourceId = entry?.resource_id ?? session.resource_id;
    const workingDirectory = session.working_directory
      || binding?.canonical_project_root
      || binding?.project_root
      || project.project_root
      || t("notConfigured");
    const selectable = !!selectionMode
      && session.access === "bound_ready"
      && !session.sync_blocked_reason
      && (selectableResourceIds?.has(reviewResourceId) ?? true);
    return (
    <SessionCard key={key} session={session} occurrenceKey={key} workingDirectory={workingDirectory}
      busy={actionBusyResourceId === session.resource_id} busyActionId={actionBusyActionId}
      onSessionAction={onSessionAction} onDeleteSession={onDeleteSession} onOpenSessionDetail={onOpenSessionDetail}
      syncEntry={entry} reviewResourceId={reviewResourceId}
      storageName={rowStorageName} storageId={storageId} syncFallback={syncFallback}
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
  const outgoingCommits = pushGitHistory?.outgoing;
  const outgoingTitle = pushGitHistory?.rewritten
    ? (activeStorageName != null
      ? t("list.outgoingRewritten", { storage: activeStorageName })
      : t("list.outgoingRewrittenUnnamed"))
    : (activeStorageName != null
      ? t("list.outgoing", { storage: activeStorageName })
      : t("list.outgoingUnnamed"));
  const commits = useMemo(() => {
    const candidates = activity?.git?.commits ?? [];
    if (!selectionMode && !narrowing) return candidates;
    /* A commit Push adds stays on the review rail with no session under it;
       a search or a scope narrows to sessions, so it drops the bare ones. */
    return candidates.filter((commit) => (
      commit.session_refs.some((reference) => sessionsByResource.has(reference.resource_id))
      || (!narrowing && outgoingCommits?.has(commit.sha) === true)
    ));
  }, [activity?.git?.commits, selectionMode, narrowing, sessionsByResource, outgoingCommits]);
  // A review holds the whole span at once, so an empty one is truly empty.
  const selectionReviewEmpty = !searching && selectionMode != null
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
   * provider, refresh, continuity, and comparison messages in source order.
   * A failed row action is not one of them: this zone scrolls away with the
   * list, so the shell draws that failure under the path bar instead. */
  const notices: NoticeItem[] = [];
  /* A refresh that failed on top of an already-loaded page: the stale page
     stays useful, so the message owns the retry instead of replacing the list.
     A first load that failed has no list to keep and stays a whole-page state
     below. */
  const refreshFailed = Boolean(loadError) && activity != null;
  if (refreshFailed) {
    notices.push({
      key: "refresh-failed",
      severity: "danger",
      message: t("notice.refreshFailed", { error: loadError ?? "" }),
      action: (
        <button type="button" className="btn btn-ghost" onClick={onRetryLoad} disabled={loading}>
          {t("common:tryAgain")}
        </button>
      ),
    });
  }
  /* A Pull of a cleared storage is an ordinary first Pull, so its review says
     nothing about the clear. */
  const storageNotice = clearedStorageNotice(comparison, comparisonStorageName);
  const clearedNotice = selectionMode === "pull" ? null : storageNotice;
  /* The dismissal names the condition, never its sentence: a sentence is in
     whichever language was on, and the same condition must stay dismissed
     across a switch. The condition is what the sentence reads — the storage
     and its name, its continuity, the state of each area this computer
     writes (whether a Pull comes first), and on Mallard Cloud the repository
     and the version a discard returned to — so a changed instruction shows
     again. */
  const clearedCloud = comparison?.managed_cloud_status ?? null;
  const clearedNoticeId = comparison && clearedNotice?.key === "storage-cleared"
    ? JSON.stringify([
      project.local_project_id,
      comparison.storage_id,
      comparison.remote_continuity,
      comparison.storage_name,
      comparison.scopes
        .filter((scope) => scope.access === "bound_ready" || scope.access === "bound_unavailable")
        .map((scope) => [scope.scope, scope.continuity.state]),
      clearedCloud && [
        clearedCloud.repository_id,
        clearedCloud.link?.repository_id ?? null,
        clearedCloud.link?.owner_login ?? null,
        clearedCloud.link?.name ?? null,
        clearedCloud.publication?.latest_version ?? null,
        clearedCloud.publication?.draft_ahead ?? null,
      ],
    ])
    : null;
  const continuityState = comparison?.remote_continuity?.state;
  const storageWarningResolved = !comparisonLoading && !comparisonError && (
    continuityState === "fresh" || continuityState === "unreviewed"
    || continuityState === "in_sync" || continuityState === "advanced"
  );
  useEffect(() => {
    if (storageWarningResolved && dismissedClearedNotice !== null) {
      clearStorageWarningDismissal(project.local_project_id, comparisonStorageId);
    }
  }, [storageWarningResolved, project.local_project_id, comparisonStorageId, dismissedClearedNotice]);
  if (clearedNotice && (clearedNoticeId === null || clearedNoticeId !== dismissedClearedNotice)) {
    notices.push({
      ...clearedNotice,
      dismissalId: clearedNoticeId ?? undefined,
      onDismiss: clearedNoticeId === null || !comparison ? undefined : () => {
        dismissStorageWarning(project.local_project_id, comparison.storage_id, clearedNoticeId);
      },
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
      message: state.detail
        ? t("notice.messageDetail", { message: providerNoticeCopy(state), detail: state.detail })
        : t("notice.message", { message: providerNoticeCopy(state) }),
    });
  }
  /* A whole-comparison failure belongs in the shared notice zone, separate
     from provider-scoped failures. A review draws it in its list instead:
     its rows are the comparison's, so there is no row below to fall back. */
  if (comparisonError && !comparisonErrorHandled && !selectionMode) {
    notices.push({
      key: "comparison-failed",
      severity: "warning",
      title: t("notice.comparisonFailedTitle", { storage: comparisonStorageName }),
      message: t("notice.comparisonFailed", { error: comparisonError }),
    });
  }
  /* Comparison failures are isolated per provider: one failed provider warns
     here while every other provider's rows keep their statuses. */
  for (const [provider, message] of providerErrors) {
    notices.push({
      key: `provider-error:${provider}`,
      severity: "warning",
      title: providerNames.get(provider) ?? provider,
      message: t("notice.providerFailed", { error: message }),
    });
  }
  /* Only one of those five is an event. A `loadError` sitting over a loaded
   * page can only have come from the refresh or the load-more the user asked
   * for; it HAPPENED and did not finish, so it re-times the whole box to
   * assertive. Everything else here is standing content: the
   * continuity state, the provider access states and both comparison failures
   * all arrive with the data this view is first rendered from, and WCAG 2.2
   * SC 4.1.3 says content already present when the view renders is not a
   * status message. It gets a named region instead. */
  const noticeTiming: NoticeTiming = refreshFailed
    ? { timing: "blocking" }
    : { timing: "standing", label: t("notice.label") };

  /* "New session" closes the standing list's search row. A review shows
     what one verb will move, not where work begins, so it has none. An
     empty flat list carries it in its box instead, so the page offers it
     once. */
  const startProviders = !selectionMode && onStartSession && activity
    ? newSessionProviders(activity.providers)
    : [];
  const startInBox = activity !== null && !activity.git && sessions.length === 0 && !narrowing;
  const newSession = (inBox: boolean) => (onStartSession && startProviders.length > 0 ? (
    <NewSessionMenu
      providers={startProviders}
      busy={startSessionBusy}
      primary={inBox}
      align={inBox ? "start" : "end"}
      onStart={onStartSession}
    />
  ) : null);

  /* A review's field lands with its rows: until then the caption stands in
     its slot, and a list that failed has nothing to search. */
  const searchField = hasBoundProvider && !(storageLoading && searchableIds.size === 0)
    && !(selectionMode && (activity === null || comparison === null))
    && !(selectionMode === "push" && selectionReviewEmpty)
    ? (
      <NameSearchField query={query} onChange={onSearchQueryChange ?? setLocalQuery}
        label={selectionMode ? t("list.searchSessions") : t("list.searchLoaded")}
        matches={searchResults.resourceIds.size} total={searchableIds.size} noun={sessionNoun} hiddenSelected={hiddenSelected}
        narrowed={statusScope !== null} leading={viewSwitch}
        trailing={startInBox ? undefined : newSession(false) ?? undefined} />
    )
    : null;
  /* The bar rides under the field it narrows with. A scope no loaded row
     falls in has no place on it — unless it is the one in force, which
     stays, at zero, so it can be released. */
  const scopeBar = searchField && scopeBarShown ? (
    <StatusScopeBar
      label={t("list.scopeLabel")}
      noun={sessionNoun}
      total={searchableIds.size}
      active={statusScope}
      onChange={chooseStatusScope}
      scopes={STATUS_SCOPES
        .filter((scope) => (scopeCounts.get(scope.id) ?? 0) > 0 || scope.id === statusScope)
        .map((scope) => ({ ...scope, count: scopeCounts.get(scope.id) ?? 0 }))}
    />
  ) : null;
  /* The button leaves with the box it sits in, so focus goes to the field
     first, as the field's own clear does. */
  const clearStatusScope = () => {
    listRef.current?.querySelector<HTMLInputElement>(".sync-name-search input")?.focus();
    chooseStatusScope(null);
  };

  const content = (
      <section
        ref={listRef}
        className="sync-history-content"
        aria-label={sectionTitle}
      >
        {/* The workspace path bar owns branch, counts, and refresh. */}
        {refreshStatus && (
          <span className="sync-visually-hidden" role="status">
            {refreshStatus}
          </span>
        )}

        {/* A non-Git project's view switch leads the page in every state,
            so it holds its place across the two faces and the notes below
            never push it down; the search rides its row whenever there is
            one. */}
        {viewSwitch && (searchField ?? <div className="sync-project-view-row">{viewSwitch}</div>)}
        {viewSwitch && scopeBar}
        <NoticeZone
          key={JSON.stringify([project.local_project_id, comparisonStorageId])}
          items={notices}
          subject={t("notice.subject")}
          className="sync-history-notices"
          {...noticeTiming}
        />
        {!viewSwitch && searchField}
        {!viewSwitch && scopeBar}
        {!hasBoundProvider ? (
          <div className="sync-history-state sync-history-profile-state">
            <Icon name="alert-triangle" size={18} />
            <div>
              <strong>{t("list.noProfile")}</strong>
              <span>{t("list.noProfileHint")}</span>
            </div>
            {onChooseAgentProfile && (
              <button
                type="button"
                className="btn"
                disabled={continuityActionBusy}
                onClick={onChooseAgentProfile}
              >
                {t("list.chooseProfile")}
              </button>
            )}
          </div>
        ) : reviewLoadingLabel ? (
          /* The review's own shape, ungated: its host already waited 150ms
             before opening the review, and this is what the review is drawn
             with from its first frame until both reads have settled. */
          <LoadingScaffold
            shape="sessions"
            label={reviewLoadingLabel}
            count={reviewScaffold?.count}
            storageOnly={reviewScaffold?.storageOnly}
          />
        ) : selectionMode && activity && comparison === null ? (
          /* A review whose comparison failed has no rows to show — they are
             the comparison's — so the failure takes the list's place with its
             own retry, never a blank and never an endless load. It announces:
             it replaces the scaffold after the read settles. */
          <div className="sync-history-state sync-history-profile-state" role="alert">
            <Icon name="alert-triangle" size={18} />
            <div>
              <strong>{t("list.compareFailed", { storage: comparisonStorageName })}</strong>
              <span>{comparisonError}</span>
            </div>
            <button type="button" className="btn" onClick={onRetryLoad} disabled={loading || comparisonLoading}>
              <Icon name="refresh" size={13} /> {t("common:tryAgain")}
            </button>
          </div>
        ) : (loading && !activity) || (storageLoading && allSessions.length === 0) ? (
          /* Not a status message: the scaffold is what the view renders with
             while its first read is in flight — the page's own row geometry
             in bones, so the list lands without a jump. On the one path that
             reaches it from a user action — picking a branch, which drops
             the list — the visually hidden line above is already announcing
             the refresh politely. Claiming a live region here only made a
             screen reader say it twice. */
          <LoadingScaffold shape="rows" label={storageLoading ? t("list.loadingCloud") : t("list.loadingActivity")} />
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
                <strong>{t("list.loadFailed", { project: label })}</strong>
                <span>{loadError}</span>
              </div>
              <button type="button" className="btn" onClick={onRetryLoad} disabled={loading}>
                <Icon name="refresh" size={13} /> {t("common:tryAgain")}
              </button>
            </div>
          ) : null
        ) : (
          <>
            {narrowing && searchResults.resourceIds.size === 0 && (
              <div className="sync-history-state">
                {selectionMode ? t("list.noMatches") : t("list.noMatchesLoaded")}
                {scopeEmptiedList && (
                  <button type="button" className="btn btn-ghost" onClick={clearStatusScope}>{t("list.clearFilter")}</button>
                )}
              </div>
            )}
            {selectionReviewEmpty && (selectionMode === "push" ? (
              /* One line: when the Git history still travels, its commits sit
                 on the rail below, so the title only has to say the sessions
                 are not part of it. */
              <div className="sync-pull-empty compact">
                <Icon name="check-circle" size={16} />
                <span><strong>{pushGitHistory?.changed ? t("list.noSessionChanges") : t("list.nothingToPush")}</strong></span>
              </div>
            ) : (
              <div className="sync-pull-empty">
                <Icon name="check-circle" size={16} />
                <span>
                  <strong>{t("list.noSessionChanges")}</strong>
                  <small>{t("list.alreadyMatch")}</small>
                </span>
              </div>
            ))}
            {!selectionReviewEmpty && activity.git && storageOnlyReferences.length > 0 && (
              /* Above the timeline, not on it: these rows have no commit and no
                 local file, so the group draws the timeline's frame treatment
                 but no rail segment. The rows are the ordinary session card —
                 the backend hands a stored session no launch actions, and its
                 title opens the stored copy's page through this storage. */
              <section className={`sync-history-storage-only${storageOnlyCollapsed ? " collapsed" : ""}`} aria-labelledby="storage-only-heading">
                <SectionFold
                  collapsed={storageOnlyCollapsed}
                  controls="storage-only-list"
                  disabled={narrowing}
                  label={t(storageOnlyCollapsed ? "list.expandStorageOnly" : "list.collapseStorageOnly", {
                    count: storageOnlyReferences.length,
                    storage: storageOnlyName,
                  })}
                  onToggle={() => setStorageOnlyCollapsed(!storageOnlyCollapsed)}
                />
                <div className="sync-history-section-heading">
                  <h2 id="storage-only-heading"><Icon name="cloud" size={13} /><Trans t={t} i18nKey="list.onlyIn" values={{ storage: storageOnlyName }} components={{ name: <span className="sync-history-storage-name" /> }} /></h2>
                  <span className="sync-history-heading-count" title={t("list.storageOnlyCount", { count: storageOnlyReferences.length })} aria-label={t("list.storageOnlyCount", { count: storageOnlyReferences.length })}><Icon name="message" size={12} />{storageOnlyReferences.length}</span>
                </div>
                {!storageOnlyCollapsed && (
                  <div id="storage-only-list" className="sync-history-thread-list flat">{storageOnlyReferences.map((reference) => { const session = sessionsByResource.get(reference.resource_id); return session ? renderSession(session, `storage-only:${session.resource_id}`) : null; })}</div>
                )}
              </section>
            )}
            {/* An empty Push review still draws the rail when it carries
                commits: they are what the Push sends. */}
            {(!selectionReviewEmpty || commits.length > 0) && (activity.git ? (showGitTimeline ? (
              <div className={`sync-history-timeline${branchUnmappedReferences.length > 0 ? " has-branch-unmapped" : ""}`}>
                {branchUnmappedReferences.length > 0 && (
                  <section className={`sync-history-branch-unmapped${branchUnmappedCollapsed ? " collapsed" : ""}`} aria-labelledby="branch-unmapped-heading">
                    <SectionFold
                      collapsed={branchUnmappedCollapsed}
                      controls="branch-unmapped-list"
                      disabled={narrowing}
                      label={t(branchUnmappedCollapsed ? "list.expandUncommitted" : "list.collapseUncommitted", {
                        count: branchUnmappedReferences.length,
                      })}
                      onToggle={() => setBranchUnmappedCollapsed(!branchUnmappedCollapsed)}
                    />
                    <div className="sync-history-section-heading">
                      <h2 id="branch-unmapped-heading"><Icon name="edit" size={13} />{t("list.uncommitted")}</h2>
                    </div>
                    {!branchUnmappedCollapsed && (
                      <div id="branch-unmapped-list" className="sync-history-thread-list flat">{branchUnmappedReferences.map((reference) => { const session = sessionsByResource.get(reference.resource_id); return session ? renderSession(session, `branch-unmapped:${session.resource_id}`) : null; })}</div>
                    )}
                  </section>
                )}
                {((!narrowing && selectionMode !== "pull") || commits.length > 0) && (
                  <section className="sync-history-commit-section" aria-label={t("list.commitsOn", { branch: activity.git.selected_branch })}>
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
                      const collapsed = !narrowing && collapsedCommitShas.has(commit.sha);
                      const displaySha = commit.short_sha.slice(0, 7);
                      const sessionListId = `commit-sessions-${commit.short_sha}`;
                      return (
                        <li key={commit.sha} className={`sync-history-commit${collapsed ? " collapsed" : ""}`}>
                          <SectionFold
                            collapsed={collapsed}
                            controls={sessionListId}
                            label={t(collapsed ? "list.expandCommit" : "list.collapseCommit", { sha: displaySha })}
                            disabled={narrowing || references.length === 0}
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
                              {outgoingCommits?.has(commit.sha) && (
                                <span
                                  className={`sync-history-branch-state sync-history-outgoing${pushGitHistory?.rewritten ? " history-only" : ""}`}
                                  title={outgoingTitle}
                                  aria-label={outgoingTitle}
                                >↑</span>
                              )}
                              {commitUrl ? (
                                <a className="sync-history-commit-link" href={commitUrl} target="_blank" rel="noopener noreferrer"
                                  title={t("list.openOnGitHub", { sha: commit.sha })} aria-label={t("list.openCommitOnGitHub", { sha: displaySha })}
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
              <section className="sync-history-flat-groups">
                {/* The flat list groups the way the Files face does:
                    stored-only sessions lead in the cloud-tinted tile — no
                    counts, each row's badge already says "Pull to download it
                    here" — and the local half takes a tile headed "Sessions"
                    only while the partition exists. Alone, the switch already
                    names it, so it stays the bare framed list. */}
                {sessions.length === 0 ? (narrowing ? null : (
                  <div className="sync-history-thread-list flat">
                    {/* Same rule as the empty rail: the window is named only
                        when paging past it could still turn something up. A
                        project whose homes hold no session at all gets no
                        "Load earlier" control, so blaming its window would
                        promise a step the page cannot offer. */}
                    <div className={`sync-history-state${startProviders.length > 0 ? " with-start" : ""}`}>
                      {(activity.older?.sessions ?? 0) > 0 ? t("list.noSessionsIn", { range: windowRange }) : t("list.noSessionsYet")}
                      {newSession(true)}
                    </div>
                  </div>
                )) : (
                  <>
                    {cloudSessions.length > 0 && (
                      <ListTile icon="cloud" cloud label={(
                        <Trans t={t} i18nKey="list.onlyIn" values={{ storage: storageOnlyName }}
                          components={{ name: <span className="sync-history-storage-name" /> }} />
                      )}>
                        {cloudSessions.map((session) => renderSession(session, `storage-only:${session.resource_id}`))}
                      </ListTile>
                    )}
                    {localSessions.length > 0 && (cloudSessions.length > 0 ? (
                      <ListTile icon="message" label={t("list.sessions")}>
                        {localSessions.map((session) => renderSession(session, session.resource_id))}
                      </ListTile>
                    ) : (
                      <div className="sync-history-thread-list flat">
                        {localSessions.map((session) => renderSession(session, session.resource_id))}
                      </div>
                    ))}
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
              <div className="sync-history-thread-list flat" aria-label={t("list.unlisted")}>
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

/** The branch a pick pins. Picking the checked-out branch pins nothing, so a
 *  refresh or a ref change keeps following whatever is checked out next. */
export function branchPin(name: string, branches: GitBranchSummary[]): string | null {
  return branches.some((branch) => branch.name === name && branch.is_current) ? null : name;
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
  comparisonErrorHandled = false,
  project,
  binding,
  refreshEpoch,
  activeStorageId = null,
  activeStorageName = null,
  selectionMode,
  pushGitHistory,
  selectedResourceIds,
  selectableResourceIds,
  reviewResourceIds,
  selectionDisabled = false,
  onToggleResource,
  onDropSelection,
  onReviewPull,
  onChooseAgentProfile,
  continuityActionBusy = false,
  comparisonOverride,
  onReviewListStateChange,
  reviewScaffold,
  onHeaderModelChange,
  onOpenSessionDetail,
  onActionError,
  searchQuery, onSearchQueryChange, onSearchResultsChange,
  viewSwitch,
}: PageProps) {
  const { i18n: { language } } = useTranslation();
  /* Keep loaded windows as a stack so Show less is a pop instead of a refetch. */
  const [pages, setPages] = useState<ProjectActivity[]>([]);
  const activity = useMemo(
    () => pages.reduce<ProjectActivity | null>((merged, page) => mergePages(merged, page), null),
    [pages],
  );
  const [comparison, setComparison] = useState<ThreadSyncComparison | null>(null);
  const [comparisonLoading, setComparisonLoading] = useState(false);
  const [storageLoading, setStorageLoading] = useState(false);
  const [comparisonError, setComparisonError] = useState<string | null>(null);
  const [managedCloudStatus, setManagedCloudStatus] = useState<ManagedCloudStatus | null>(null);
  /* The branch the viewer pinned in the picker. Null follows the checked-out
     branch, so every reload re-resolves HEAD and re-reads its history. */
  const [branch, setBranch] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const actionInFlightRef = useRef(false);
  /* The shell outlives this list, so only the page still showing the project
     an action ran for may report on it. Leaving the page or switching project
     clears the message, and a failure that lands after that is dropped
     instead of surfacing over whatever screen came next. */
  const shownProjectRef = useRef<string | null>(project.local_project_id);
  useEffect(() => {
    shownProjectRef.current = project.local_project_id;
    return () => {
      shownProjectRef.current = null;
      onActionError(null);
    };
  }, [project.local_project_id, onActionError]);
  const reportActionError = (message: string | null) => {
    if (shownProjectRef.current === project.local_project_id) onActionError(message);
  };
  const [actionBusyResourceId, setActionBusyResourceId] = useState<string | null>(null);
  const [actionBusyActionId, setActionBusyActionId] = useState<string | null>(null);
  const [startSessionBusy, setStartSessionBusy] = useState(false);
  const requestRef = useRef(0);
  const comparisonRequestRef = useRef(0);
  const hasBoundProvider = Object.keys(binding?.profile_ids ?? {}).length > 0;

  const load = async (beforeTime?: number | null, requestedBranch = branch, fresh = false) => {
    if (!hasBoundProvider) return false;
    const requestId = ++requestRef.current;
    if (beforeTime) setLoadingMore(true); else setLoading(true);
    setStorageLoading(activeStorageId !== null);
    reportActionError(null);
    setLoadError(null);
    try {
      const next = await projectSyncApi.getProjectActivity(
        project.local_project_id,
        activeStorageId,
        requestedBranch,
        beforeTime,
        30,
        selectionMode != null,
        fresh,
      );
      if (requestId !== requestRef.current) return false;
      setPages((current) => beforeTime ? [...current, next] : [next]);
    } catch (reason) {
      // Page-level failures and row-action failures are different facts;
      // keeping them in one state made them indistinguishable on screen.
      if (requestId === requestRef.current) setLoadError(errorMessage(reason));
    } finally {
      if (requestId === requestRef.current) { setLoading(false); setLoadingMore(false); setStorageLoading(false); }
    }
    return requestId === requestRef.current;
  };

  const loadWorkspace = async (requestedBranch = branch) => {
    if (comparisonOverride !== undefined || activeStorageId === null) {
      comparisonRequestRef.current += 1;
      setComparison(comparisonOverride ?? null);
      setManagedCloudStatus(comparisonOverride?.managed_cloud_status ?? null);
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
    setStorageLoading(true);
    setLoadError(null);
    setComparisonError(null);
    reportActionError(null);
    let receivedActivity = false;
    try {
      await projectSyncApi.refreshProjectActivity({
        localProjectId: project.local_project_id,
        storageId: activeStorageId,
        branch: requestedBranch,
        windowDays: 30,
        review: selectionMode != null,
        bindingRevision: binding?.revision ?? null,
        refreshEpoch,
      }, (event) => {
        if (event.stage === "activity" || event.stage === "activity_error") {
          if (requestId !== requestRef.current) return;
          setStorageLoading(event.storage_pending);
          if (event.stage === "activity") {
            receivedActivity = true;
            setPages([event.activity]);
            setLoadError(null);
          } else {
            setLoadError(event.message);
          }
          setLoading(false);
        } else {
          if (comparisonRequestId !== comparisonRequestRef.current) return;
          if (event.stage === "comparison") {
            setComparison(event.comparison);
            setManagedCloudStatus(event.comparison.managed_cloud_status ?? null);
            setComparisonError(null);
          } else {
            setComparison(null);
            setComparisonError(event.message);
            setManagedCloudStatus(event.managed_cloud_status ?? null);
          }
          setComparisonLoading(false);
        }
      });
    } catch (reason) {
      if (requestId === requestRef.current && !receivedActivity) setLoadError(errorMessage(reason));
      if (comparisonRequestId === comparisonRequestRef.current) {
        setComparison(null);
        setComparisonError(errorMessage(reason));
        setManagedCloudStatus(null);
      }
    } finally {
      if (requestId === requestRef.current) { setLoading(false); setStorageLoading(false); }
      if (comparisonRequestId === comparisonRequestRef.current) setComparisonLoading(false);
    }
  };

  useEffect(() => {
    // The whole per-project slate resets, not just the data. Leaving the error
    // and loading flags set carried the previous project's failure banner onto
    // the next one, and a switch made mid-flight stranded the spinners on
    // because their owning request had already been superseded.
    setPages([]); setBranch(null); setComparison(null); setManagedCloudStatus(null);
    setLoadError(null); setComparisonError(null); reportActionError(null);
    setLoading(false); setComparisonLoading(false); setLoadingMore(false); setStorageLoading(false);
    void loadWorkspace(null);
    return () => { requestRef.current += 1; comparisonRequestRef.current += 1; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.local_project_id, binding?.revision, activeStorageId, refreshEpoch, comparisonOverride, selectionMode]);

  /* Failed only while a read that never landed has an error: a load-more that
     fails over a loaded list leaves that list trustworthy. With no agent
     profile connected nothing is ever read, and the list says so itself. */
  const reviewListState: ReviewListState = !hasBoundProvider
    || (activity === null && loadError !== null)
    || (comparison === null && comparisonError !== null)
    ? "failed"
    : activity !== null && !loading && !storageLoading && comparison !== null && !comparisonLoading
      ? "ready"
      : "loading";
  useEffect(() => {
    onReviewListStateChange?.(reviewListState);
  }, [reviewListState, onReviewListStateChange]);

  // The path bar holds onto whatever model it was last handed, so a model that
  // is a new object every render would drive it into a re-render loop. These
  // two handlers are the only mutable part of it; a latest-ref keeps their
  // identity fixed while they still see this render's `load`, `branch`, and
  // `branches`.
  const branches = activity?.git?.branches;
  const latest = useRef({ load, loadWorkspace, branch, branches });
  latest.current = { load, loadWorkspace, branch, branches };

  /* A checkout or commit made outside Mallard moves the Git refs; reload so
     the mapping is rebuilt from the branch's current history. The watch is
     held only while this page shows a Git project. */
  const hasGit = activity?.git != null;
  useEffect(() => {
    if (!hasGit) return;
    const localProjectId = project.local_project_id;
    const release = projectSyncApi.holdProjectGitWatch(localProjectId);
    const unlisten = projectSyncApi.onProjectGitChanged((event) => {
      if (event.local_project_id !== localProjectId) return;
      void latest.current.load(null, latest.current.branch, true);
    }).catch(() => null);
    return () => {
      release();
      void unlisten.then((stop) => stop?.());
    };
  }, [project.local_project_id, hasGit]);

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
    setBranch(branchPin(nextBranch, latest.current.branches ?? []));
    setPages([]);
    void latest.current.load(null, nextBranch);
  }, []);
  const refreshActivity = useCallback(() => {
    // An explicit refresh must not replay a request started before a mutation.
    projectSyncApi.invalidateProjectActivity(project.local_project_id, activeStorageId);
    void latest.current.loadWorkspace(latest.current.branch);
  }, [project.local_project_id, activeStorageId]);

  // The chip's breakdown is copy, so a language switch recounts it.
  const headerCounts = useMemo(
    () => activityHeaderCounts(
      comparison,
      comparison?.storage_name ?? activeStorageName ?? t("projects:selectedStorage"),
    ),
    [comparison, activeStorageName, language],
  );
  const headerModel = useMemo<ActivityHeaderModel>(() => ({
    projectId: project.local_project_id,
    storageId: activeStorageId,
    managedCloudStatus,
    // A project with no repository has no branches to offer; the bar simply
    // draws no chip, and the counts and refresh still stand.
    branches: activity?.git?.branches ?? [],
    selectedBranch: activity?.git?.selected_branch ?? "",
    onSelectBranch: changeBranch,
    counts: headerCounts,
    comparisonError,
    comparison,
    refreshing: loading || comparisonLoading,
    onRefresh: refreshActivity,
  }), [
    activity?.git?.branches,
    activity?.git?.selected_branch,
    changeBranch,
    headerCounts,
    project.local_project_id,
    activeStorageId,
    managedCloudStatus,
    comparisonError,
    comparison,
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

  const deleteSession = async (session: ProjectSessionSummary) => {
    if (actionInFlightRef.current) return;
    actionInFlightRef.current = true;
    try {
      const approved = await confirm(
        t("projects:deleteConfirm.message", { title: session.title || t("projects:untitledSession") }),
        {
          title: t("projects:deleteConfirm.title"),
          kind: "warning",
          okLabel: t("common:delete"),
          cancelLabel: t("common:cancel"),
        },
      );
      if (!approved) return;
      reportActionError(null);
      setActionBusyResourceId(session.resource_id);
      setActionBusyActionId("delete");
      let failure: string | null = null;
      try {
        const result = await projectSyncApi.deleteLocalProjectSession(project.local_project_id, session.resource_id);
        failure = localSessionDeleteError(session.provider, result);
      } catch (reason) { failure = errorMessage(reason); }
      await loadWorkspace(branch);
      reportActionError(failure);
    } catch (reason) { reportActionError(errorMessage(reason)); }
    finally {
      setActionBusyResourceId(null);
      setActionBusyActionId(null);
      actionInFlightRef.current = false;
    }
  };

  const runSessionAction = async (session: ProjectSessionSummary, action: SessionActionDescriptor) => {
    if (!action.enabled || actionInFlightRef.current) return;
    actionInFlightRef.current = true;
    reportActionError(null);
    setActionBusyResourceId(session.resource_id);
    setActionBusyActionId(action.action_id);
    try {
      await projectSyncApi.executeProjectSessionAction(project.local_project_id, session.resource_id, action.action_id);
    } catch (reason) { reportActionError(errorMessage(reason)); }
    finally {
      setActionBusyResourceId(null);
      setActionBusyActionId(null);
      actionInFlightRef.current = false;
    }
  };
  /* One launch at a time, shared with the row actions. Nothing is added to
     the list here: the session is listed once its agent writes it and the
     list is read again. */
  const startSession = async (provider: ProviderKey, surface: NewSessionSurface) => {
    if (actionInFlightRef.current) return;
    actionInFlightRef.current = true;
    reportActionError(null);
    setStartSessionBusy(true);
    try {
      await projectSyncApi.startProjectSession(project.local_project_id, provider, surface);
    } catch (reason) { reportActionError(errorMessage(reason)); }
    finally {
      setStartSessionBusy(false);
      actionInFlightRef.current = false;
    }
  };
  return <ProjectChatHistoryContent key={`${project.local_project_id}:${activeStorageId}:${selectionMode}`} searchQuery={searchQuery} onSearchQueryChange={onSearchQueryChange} onSearchResultsChange={onSearchResultsChange} project={project} binding={binding} activity={activity} loading={loading} loadingMore={loadingMore} loadError={loadError} onRetryLoad={() => void loadWorkspace(branch)} actionBusyResourceId={actionBusyResourceId} actionBusyActionId={actionBusyActionId} onLoadMore={() => void load(activity?.older ? activity.older.newest_at + 1 : null, activity?.git?.selected_branch ?? branch)} onShowLess={showLess} canShowLess={pages.length > 1} onSessionAction={(session, action) => void runSessionAction(session, action)} onDeleteSession={(session) => void deleteSession(session)} onOpenSessionDetail={onOpenSessionDetail} comparison={comparison} storageLoading={storageLoading} comparisonLoading={comparisonLoading} comparisonError={comparisonError} comparisonErrorHandled={comparisonErrorHandled} activeStorageName={activeStorageName} storageId={activeStorageId} selectionMode={selectionMode} pushGitHistory={pushGitHistory} selectedResourceIds={selectedResourceIds} selectableResourceIds={selectableResourceIds} reviewResourceIds={reviewResourceIds} selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} onReviewPull={onReviewPull} onChooseAgentProfile={onChooseAgentProfile} continuityActionBusy={continuityActionBusy} reviewScaffold={reviewScaffold} viewSwitch={viewSwitch} onStartSession={selectionMode ? undefined : (provider, surface) => void startSession(provider, surface)} startSessionBusy={startSessionBusy} />;
}
