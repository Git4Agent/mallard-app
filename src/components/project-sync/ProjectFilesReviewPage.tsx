import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import type { ProjectFileSyncEligibility } from "../../types";
import Icon from "../Icons";
import LoadingScaffold from "../LoadingScaffold";
import Notice from "../notice/Notice";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../sync-review/ThreadStatusIcon";
import { formatDateTime } from "../../clock";
import { formatBytes } from "../../format";
import { STORAGE_BLOCKING_STATES } from "./model";
import ProjectFileDetailsMenu from "./ProjectFileDetailsMenu";

export type ProjectFileReviewEntryType = "file" | "directory" | "blocked";

export interface ProjectFileReviewRow {
  resourceId: string;
  relativePath: string;
  entryType: ProjectFileReviewEntryType;
  state: string;
  size?: number | null;
  mode?: number | null;
  sourceMtime?: number | null;
  localPresent: boolean;
  storagePresent: boolean;
  newlyDiscovered?: boolean;
  selectedAfterScan?: boolean;
  blockedReason?: string | null;
  warningCode?: string | null;
  warningDigest?: string | null;
  operation?: "add" | "replace" | "create_directory" | "delete_file" | "delete_directory";
  /** A diverged replace copies the local version here first (Pull only). */
  preservedCopyPath?: string;
}

interface Props {
  mode: "push" | "pull";
  eligibility: ProjectFileSyncEligibility;
  rows: ProjectFileReviewRow[];
  selectedIds: ReadonlySet<string>;
  requiredIds?: ReadonlySet<string>;
  removalIds?: ReadonlySet<string>;
  acknowledgedWarningDigests?: ReadonlySet<string>;
  scanned?: boolean;
  loading?: boolean;
  ignoredCount?: number;
  blockedCount?: number;
  warnings?: string[];
  disabled?: boolean;
  onScan?: () => void;
  onToggle: (resourceId: string) => void;
  onBulkToggle?: (resourceIds: string[], selected: boolean) => void;
  onToggleRemoval?: (resourceId: string) => void;
  onToggleWarning?: (warningDigest: string) => void;
  onReviewPull?: () => void;
}

const MAX_RENDERED_ROWS = 500;

function parentPath(path: string): string {
  const separator = path.lastIndexOf("/");
  return separator < 0 ? "" : path.slice(0, separator);
}

function fileName(path: string): string {
  return path.split("/").slice(-1)[0];
}

/** The pull row's operation chip. Push rows carry no state chip at all —
 * their tick and status mark already say it. */
function pullOperationLabel(row: ProjectFileReviewRow): string {
  if (row.operation === "delete_file") return "Delete file";
  if (row.operation === "delete_directory") return "Delete folder";
  if (row.operation === "create_directory") return "Create folder";
  if (row.operation === "add") return "Add file";
  if (row.operation === "replace") {
    return row.preservedCopyPath ? "Replace file · keeps local copy" : "Replace file";
  }
  return row.state.replace(/_/g, " ");
}

/** The card's status mark: the same tone-and-sentence vocabulary the session
 * rows and the Files activity face speak, extended with the pull operations. */
function rowPresentation(
  row: ProjectFileReviewRow,
  mode: Props["mode"],
): { icon: ThreadStatusIconName; tone: string; sentence: string } {
  if (row.blockedReason || row.entryType === "blocked" || row.state === "blocked") {
    return {
      icon: "unavailable",
      tone: "unavailable",
      sentence: `Unavailable for sync. ${row.blockedReason ?? ""}`.trim(),
    };
  }
  if (mode === "pull") {
    switch (row.operation) {
      case "add":
        return { icon: "cloud-outstanding", tone: "storage", sentence: "Only in storage. Applying downloads it here." };
      case "create_directory":
        return { icon: "cloud-outstanding", tone: "storage", sentence: "Applying creates this folder." };
      case "replace":
        return row.preservedCopyPath
          ? {
            icon: "divergent",
            tone: "diverged",
            sentence: `Changed here and in storage. Applying keeps the local version as ${row.preservedCopyPath}, then writes the storage version.`,
          }
          : { icon: "cloud-outstanding", tone: "storage", sentence: "Newer in storage. Applying updates this computer." };
      case "delete_file":
        return { icon: "conflict", tone: "conflict", sentence: "Removed in storage. Applying deletes the local file (a backup is kept)." };
      case "delete_directory":
        return { icon: "conflict", tone: "conflict", sentence: "Removed in storage. Applying deletes the empty local folder." };
      default:
        break;
    }
  }
  switch (row.state) {
    case "synced":
      return { icon: "synced", tone: "synced", sentence: "Up to date here and in storage." };
    case "local_only":
      return { icon: "local-outstanding", tone: "local", sentence: "Only on this computer. Push to save it in storage." };
    case "local_ahead":
      return { icon: "local-outstanding", tone: "local", sentence: "Newer on this computer. Push to update storage." };
    case "storage_only":
      return { icon: "cloud-outstanding", tone: "storage", sentence: "Only in storage. Pull to download it here." };
    case "storage_ahead":
      return { icon: "cloud-outstanding", tone: "storage", sentence: "Newer in storage. Pull to update this computer." };
    case "diverged":
      return { icon: "divergent", tone: "diverged", sentence: "Changed here and in storage. Review before syncing." };
    case "missing":
      return { icon: "unavailable", tone: "unavailable", sentence: "In the sync recipe but missing from this folder." };
    default:
      return { icon: "unknown", tone: "unknown", sentence: "State unknown. Rescan to compare." };
  }
}

/** The acknowledgement chip's lead, the Catalog's vocabulary for the same
 * scanner findings. An unknown code keeps a plain generic lead. */
const WARNING_LEADS: Record<string, string> = {
  executable: "Executable content",
  possible_secret: "Credential-shaped content",
};

function warningLead(code: string): string {
  return WARNING_LEADS[code] ?? "Scan warning";
}

function formatMode(mode?: number | null): string | null {
  return mode == null ? null : `0${mode.toString(8).padStart(3, "0")}`;
}

/* The same refusal is printed on both faces of this page — the unscanned empty
   state and the scanned list — so it is written once here. `eligibility` is a
   prop: the box is on screen the moment the page renders, which makes it
   standing content and a named region rather than a live one (WCAG 2.2 SC
   4.1.3). Nothing failed while the user watched; the folder was never eligible.
   The detected Git root is the one thing that differs between the two sites —
   so it stays opt-in. */
export function ProjectFilesLockedNotice({
  eligibility,
  withDetectedRoot = false,
}: {
  eligibility: ProjectFileSyncEligibility;
  withDetectedRoot?: boolean;
}) {
  return (
    <Notice
      severity="danger"
      timing="standing"
      label="Why project files are locked"
      title="Project files are locked."
      className="sync-project-files-locked"
    >
      {eligibility.reason}
      {withDetectedRoot && eligibility.detected_root ? ` Git root: ${eligibility.detected_root}` : ""}
    </Notice>
  );
}

function rowIsSelectable(
  row: ProjectFileReviewRow,
  mode: Props["mode"],
  eligible: boolean,
): boolean {
  if (!eligible || row.entryType === "blocked" || row.blockedReason) return false;
  if (mode === "push" && STORAGE_BLOCKING_STATES.has(row.state)) return false;
  return true;
}

/** One directory band's worth of cards: the file rows directly inside it. */
interface ReviewFileGroup {
  directory: string;
  cards: ProjectFileReviewRow[];
}

export default function ProjectFilesReviewPage({
  mode,
  eligibility,
  rows,
  selectedIds,
  requiredIds = new Set(),
  removalIds = new Set(),
  acknowledgedWarningDigests = new Set(),
  scanned = true,
  loading = false,
  ignoredCount = 0,
  blockedCount = 0,
  warnings = [],
  disabled = false,
  onScan,
  onToggle,
  onBulkToggle,
  onToggleRemoval,
  onToggleWarning,
  onReviewPull,
}: Props) {
  const [search, setSearch] = useState("");
  const eligible = eligibility.state === "eligible";
  const normalizedSearch = search.trim().toLocaleLowerCase();

  /* Which rows draw a card. Files always do. A directory does only when
     nothing else speaks for it — no descendant rows at all (an empty folder
     still has to be pushable) — or when its own deletion is the decision on
     the table. Every other directory is represented by its band and rides
     the required-ancestor rule. */
  const cardRows = useMemo(() => {
    const hasDescendants = new Set<string>();
    for (const row of rows) {
      for (let parent = parentPath(row.relativePath); parent !== ""; parent = parentPath(parent)) {
        hasDescendants.add(parent);
      }
    }
    return rows.filter((row) => (
      row.entryType !== "directory"
      || row.operation === "delete_directory"
      || !hasDescendants.has(row.relativePath)
    ));
  }, [rows]);

  const shownCards = useMemo(
    () => (normalizedSearch
      ? cardRows.filter((row) => row.relativePath.toLocaleLowerCase().includes(normalizedSearch))
      : cardRows
    ).slice(0, MAX_RENDERED_ROWS),
    [cardRows, normalizedSearch],
  );

  const groups = useMemo<ReviewFileGroup[]>(() => {
    const byDirectory = new Map<string, ProjectFileReviewRow[]>();
    for (const row of shownCards) {
      const directory = parentPath(row.relativePath);
      if (!byDirectory.has(directory)) byDirectory.set(directory, []);
      byDirectory.get(directory)!.push(row);
    }
    return [...byDirectory.keys()]
      .sort((a, b) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)))
      .map((directory) => ({ directory, cards: byDirectory.get(directory)! }));
  }, [shownCards]);

  /* The band's cluster covers the whole subtree — files and the directory
     resources riding along — the way the review header's cluster covers the
     step. Counts speak for the selectable files; directories select
     implicitly with them. */
  const subtreeRows = (directory: string) => rows.filter((row) => (
    row.relativePath === directory || row.relativePath.startsWith(`${directory}/`)
  ));
  const subtreeSelection = (directory: string) => {
    const files = subtreeRows(directory).filter((row) => (
      row.entryType !== "directory" && rowIsSelectable(row, mode, eligible) && !removalIds.has(row.resourceId)
    ));
    return {
      picked: files.filter((row) => selectedIds.has(row.resourceId)).length,
      total: files.length,
    };
  };
  const setSubtree = (directory: string, shouldSelect: boolean) => {
    const targets = subtreeRows(directory).filter((row) => (
      rowIsSelectable(row, mode, eligible) && !removalIds.has(row.resourceId)
    ));
    const ids = targets.map((row) => row.resourceId);
    if (onBulkToggle) {
      onBulkToggle(ids, shouldSelect);
      return;
    }
    for (const row of targets) {
      if (selectedIds.has(row.resourceId) !== shouldSelect) onToggle(row.resourceId);
    }
  };

  const groupFacts = (directory: string): string => {
    const files = subtreeRows(directory).filter((row) => row.entryType !== "directory");
    const bytes = files.reduce((sum, row) => sum + (row.size ?? 0), 0);
    const count = `${files.length} file${files.length === 1 ? "" : "s"}`;
    return bytes > 0 ? `${count} · ${formatBytes(bytes)}` : count;
  };

  /* Opening the step is the opt-in: the scan starts itself, once per mount.
     A failed scan leaves `scanned` false with the parent's error notice on
     screen, and the toolbar's Rescan is the retry — the guard is what keeps
     a failing walk from re-launching in a loop. */
  const autoScanRequested = useRef(false);
  useEffect(() => {
    if (mode !== "push" || scanned || loading || disabled || !eligible || !onScan) return;
    if (autoScanRequested.current) return;
    autoScanRequested.current = true;
    onScan();
  }, [mode, scanned, loading, disabled, eligible, onScan]);

  const renderCard = (row: ProjectFileReviewRow) => {
    const isDirectory = row.entryType === "directory";
    const selected = selectedIds.has(row.resourceId) && !removalIds.has(row.resourceId);
    const required = requiredIds.has(row.resourceId);
    const removed = removalIds.has(row.resourceId);
    const selectable = rowIsSelectable(row, mode, eligible) && !removed;
    const presentation = rowPresentation(row, mode);
    const warningAcknowledged = !!row.warningDigest && acknowledgedWarningDigests.has(row.warningDigest);
    const size = row.size == null ? null : formatBytes(row.size);
    const displayMode = formatMode(row.mode);
    return (
      <article
        key={row.resourceId}
        className={`sync-history-thread-card catalog-card${removed ? " removal" : ""}`}
      >
        <div className="sync-history-thread-topline">
          <div className="sync-history-thread-title">
            {selectable && !required && (
              <label
                className="sync-sync-row-choice"
                title={`${selected ? "Exclude" : mode === "pull" ? "Apply" : "Include"} ${row.relativePath}`}
              >
                <input
                  type="checkbox"
                  aria-label={`${mode === "pull" ? "Apply" : "Include"} ${row.relativePath}`}
                  checked={selected}
                  disabled={disabled}
                  onChange={() => onToggle(row.resourceId)}
                />
              </label>
            )}
            <span
              className={`sync-thread-sync-indicator ${presentation.tone}`}
              data-tooltip={presentation.sentence}
              aria-label={presentation.sentence}
              tabIndex={0}
            >
              <ThreadStatusIcon status={presentation.icon} size={18} />
            </span>
            {isDirectory && <Icon name="folder" size={14} aria-hidden="true" />}
            <strong title={row.relativePath}>{fileName(row.relativePath)}</strong>
            {/* Pull rows name their operation; push rows say nothing here —
                the tick and the status mark already carry the state, so a
                chip would just restate them. Blockers and required folders
                are the two facts neither control speaks for. */}
            {(row.blockedReason || mode === "pull" || (mode === "push" && required)) && (
              <span className="sync-history-branch-state history-only" title={presentation.sentence}>
                {row.blockedReason
                  ? "Blocked"
                  : mode === "pull"
                    ? pullOperationLabel(row)
                    : "Required folder"}
              </span>
            )}
            {removed && (
              <span className="sync-history-branch-state history-only">Removes from storage</span>
            )}
            {row.warningCode && (
              <span className="sync-project-file-warning">
                <Icon name="alert-triangle" size={13} /> Warning
              </span>
            )}
          </div>
          <div className="sync-history-thread-actions">
            {mode === "push" && row.storagePresent && onToggleRemoval && (
              <button
                type="button"
                className="btn btn-ghost sync-history-launch-action"
                disabled={disabled || !eligible || required}
                title={removed ? "Keep in storage" : "Remove from storage"}
                aria-label={removed ? `Keep ${row.relativePath} in storage` : `Remove ${row.relativePath} from storage`}
                onClick={() => onToggleRemoval(row.resourceId)}
              >
                <Icon name={removed ? "refresh" : "trash"} size={14} />
              </button>
            )}
          </div>
        </div>
        <div className="sync-history-session-details">
          <div className="sync-history-session-summary">
            <div className="sync-history-thread-meta" aria-label={`${row.relativePath} details`}>
              {size != null
                ? (
                  <span
                    className="sync-history-thread-metric"
                    data-tooltip={`${row.localPresent ? "On disk" : "In storage"} · ${size}`}
                    tabIndex={0}
                  >
                    <Icon name="drive" size={12} /><b>{size}</b>
                  </span>
                )
                : <span className="sync-history-thread-metric" aria-hidden="true" />}
            </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <ProjectFileDetailsMenu
              relativePath={row.relativePath}
              entryTypeLabel={isDirectory ? "Directory" : row.entryType === "blocked" ? "Blocked entry" : "Regular file"}
              mode={displayMode}
              modified={row.sourceMtime == null ? null : formatDateTime(row.sourceMtime * 1_000)}
              preservedCopyPath={row.preservedCopyPath}
              blockedReason={row.blockedReason}
            />
          </div>
            {row.sourceMtime != null && (
              <span
                className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end"
                data-tooltip={`${row.localPresent ? "Modified" : "Updated in storage"} · ${formatDateTime(row.sourceMtime * 1_000)}`}
                tabIndex={0}
              >
                <time>{formatDateTime(row.sourceMtime * 1_000)}</time>
              </span>
            )}
          </div>
        </div>
        {row.warningDigest && row.warningCode && onToggleWarning && (
          <label className="catalog-card-ack">
            <span className="sync-sync-row-choice">
              <input
                type="checkbox"
                checked={warningAcknowledged}
                disabled={disabled || !selected}
                aria-label={`Reviewed: ${warningLead(row.warningCode)} — ${row.relativePath}`}
                onChange={() => onToggleWarning(row.warningDigest!)}
              />
            </span>
            <Icon name="alert-triangle" size={12} />
            <span className="catalog-card-ack-copy">
              <strong>{warningLead(row.warningCode)}</strong> — {row.relativePath}
            </span>
            <small className={warningAcknowledged ? "done" : undefined}>
              {warningAcknowledged ? "Reviewed" : "Mark reviewed"}
            </small>
          </label>
        )}
        {mode === "push" && STORAGE_BLOCKING_STATES.has(row.state) && onReviewPull && (
          /* Warning, not error: nothing has failed — the push has not run —
             the row simply cannot be included until a Pull settles the
             storage side. `inline` rather than `standing` because this marker
             repeats per row, and a landmark each would hand a screen-reader
             user dozens of regions to step through. */
          <Notice
            severity="warning"
            timing="inline"
            className="sync-project-file-conflict"
            action={<button type="button" className="btn btn-ghost" onClick={onReviewPull}>Open Pull review</button>}
          >
            Pull required
          </Notice>
        )}
      </article>
    );
  };

  const renderBand = (directory: string) => {
    const { picked, total } = subtreeSelection(directory);
    const all = total > 0 && picked === total;
    const verb = all ? "Exclude" : mode === "pull" ? "Apply" : "Include";
    return (
      <div className="sync-project-files-group-heading">
        {/* The folder's own row control: the same checkbox its rows wear —
            ticked, unticked, or a dash while the folder is partly selected —
            and one click applies or clears the subtree. The icon pair this
            replaces spent a check-circle the header already means something
            else by, and a trash that reads as deletion on a page where a
            card's trash really deletes from storage. The counter chip stays,
            moved to the band's far edge. */}
        <label className="sync-sync-row-choice" title={`${verb} every file in ${directory}/`}>
          <input
            type="checkbox"
            checked={all}
            ref={(node) => { if (node) node.indeterminate = picked > 0 && picked < total; }}
            disabled={disabled || !eligible || total === 0}
            aria-label={`${verb} every file in ${directory}`}
            onChange={() => setSubtree(directory, !all)}
          />
        </label>
        <Icon name="folder" size={14} />
        <strong>{directory}/</strong>
        <small>{groupFacts(directory)}</small>
        <span
          className="sync-sync-review-selected"
          style={{ "--review-count-digits": String(total).length } as CSSProperties}
          aria-label={`${picked} of ${total} selected in ${directory}`}
          title={`${picked} of ${total} selected in ${directory}/`}
        >
          <b>{picked}</b>/{total}
        </span>
      </div>
    );
  };

  return (
    <section className="sync-project-files-page" aria-label="Project files">
      {!eligible && <ProjectFilesLockedNotice eligibility={eligibility} withDetectedRoot />}

      {/* One line: the tab already says "Project files", so the bar spends
          its row on the controls — search, the tally, and the step's verb. */}
      <div className="sync-project-files-search">
        <input
          type="search"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          /* Escape clears the filter, the same as any search field. Without
             this it reaches the review's own window listener and throws the
             whole prepared push away. An empty field has nothing to clear,
             so it still closes the review. */
          onKeyDown={(event) => {
            if (event.key !== "Escape" || !search) return;
            event.preventDefault();
            event.stopPropagation();
            setSearch("");
          }}
          placeholder="Search files…"
          aria-label="Search project file paths"
          disabled={rows.length === 0}
        />
        {rows.length > 0 && <span>{cardRows.length} entr{cardRows.length === 1 ? "y" : "ies"}</span>}
        {mode === "push" && onScan && (
          <button type="button" className="btn btn-ghost" onClick={onScan} disabled={disabled || loading || !eligible}>
            <Icon name="refresh" size={14} className={loading ? "icon-spin" : undefined} /> {loading ? "Scanning…" : "Rescan"}
          </button>
        )}
      </div>

      {rows.length === 0 ? (
        mode === "push" && !scanned && eligible ? (
          <LoadingScaffold shape="rows" label="Scanning project files" />
        ) : (
          <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No changes</strong></span></div>
        )
      ) : (
        <>
          {groups.map((group) => (
            <div key={group.directory || "(root)"}>
              {group.directory !== "" && renderBand(group.directory)}
              <div className="sync-history-thread-list flat">
                {group.cards.map(renderCard)}
              </div>
            </div>
          ))}
          {shownCards.length >= MAX_RENDERED_ROWS && cardRows.length > shownCards.length && (
            <div className="sync-project-file-limit">First {MAX_RENDERED_ROWS} shown. Search to narrow the list.</div>
          )}
        </>
      )}

      {(ignoredCount > 0 || blockedCount > 0 || warnings.length > 0) && (
        <details className="sync-project-files-scan-summary">
          <summary>{ignoredCount} ignored · {blockedCount} blocked · {warnings.length} scan warning{warnings.length === 1 ? "" : "s"}</summary>
          {warnings.map((warning) => <p key={warning}>{warning}</p>)}
        </details>
      )}
    </section>
  );
}
