import { useMemo, useState } from "react";
import type { ProjectFileSyncEligibility } from "../../types";
import Icon from "../Icons";
import Notice from "../notice/Notice";
import { formatDateTime } from "../../clock";
import { formatBytes } from "../../format";
import { STORAGE_BLOCKING_STATES } from "./model";

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
  onSelectSafe?: () => void;
  onToggle: (resourceId: string) => void;
  onBulkToggle?: (resourceIds: string[], selected: boolean) => void;
  onToggleRemoval?: (resourceId: string) => void;
  onToggleWarning?: (warningDigest: string) => void;
  onReviewPull?: () => void;
}

const MAX_RENDERED_ROWS = 500;

function parentPath(path: string): string | null {
  const separator = path.lastIndexOf("/");
  return separator < 0 ? null : path.slice(0, separator);
}

function depth(path: string): number {
  return path.split("/").length - 1;
}

function displayState(row: ProjectFileReviewRow, mode: Props["mode"]): string {
  if (row.blockedReason) return "Blocked";
  if (row.operation === "delete_file") return "Delete file";
  if (row.operation === "delete_directory") return "Delete folder";
  if (row.operation === "create_directory") return "Create folder";
  if (row.operation === "add") return "Add file";
  if (row.operation === "replace") return "Replace file";
  if (row.selectedAfterScan) return "New · selected after scan";
  const labels: Record<string, string> = {
    synced: "Synced",
    local_only: mode === "push" ? "New" : "Local only",
    local_ahead: "Local ahead",
    storage_only: "Storage only",
    storage_ahead: "Storage ahead",
    diverged: "Diverged",
    missing: "Missing locally",
    blocked: "Blocked",
    unknown: "Unknown",
  };
  return labels[row.state] ?? row.state.replace(/_/g, " ");
}

/** The row's size is optional; everything else is the shared formatter. */
function formatOptionalBytes(value?: number | null): string | null {
  return value == null ? null : formatBytes(value);
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
function ProjectFilesLockedNotice({
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
  onSelectSafe,
  onToggle,
  onBulkToggle,
  onToggleRemoval,
  onToggleWarning,
  onReviewPull,
}: Props) {
  const [search, setSearch] = useState("");
  const [expandedDirectories, setExpandedDirectories] = useState<Set<string>>(() => new Set());
  const [details, setDetails] = useState<Set<string>>(() => new Set());
  const eligible = eligibility.state === "eligible";
  const normalizedSearch = search.trim().toLocaleLowerCase();
  const directoryPaths = useMemo(
    () => new Set(rows.filter((row) => row.entryType === "directory").map((row) => row.relativePath)),
    [rows],
  );
  /* Count descendants once per row change in O(rows x depth). Scanning every
     row for each directory would be quadratic in the project file count. */
  const descendantCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const row of rows) {
      for (let parent = parentPath(row.relativePath); parent; parent = parentPath(parent)) {
        counts.set(parent, (counts.get(parent) ?? 0) + 1);
      }
    }
    return counts;
  }, [rows]);
  const shownRows = useMemo(() => {
    if (normalizedSearch) {
      const matchingPaths = new Set(rows
        .filter((row) => row.relativePath.toLocaleLowerCase().includes(normalizedSearch))
        .map((row) => row.relativePath));
      for (const path of [...matchingPaths]) {
        let parent = parentPath(path);
        while (parent) {
          if (directoryPaths.has(parent)) matchingPaths.add(parent);
          parent = parentPath(parent);
        }
      }
      return rows.filter((row) => matchingPaths.has(row.relativePath)).slice(0, MAX_RENDERED_ROWS);
    }
    return rows.filter((row) => {
      let parent = parentPath(row.relativePath);
      while (parent) {
        if (directoryPaths.has(parent) && !expandedDirectories.has(parent)) return false;
        parent = parentPath(parent);
      }
      return true;
    }).slice(0, MAX_RENDERED_ROWS);
  }, [directoryPaths, expandedDirectories, normalizedSearch, rows]);
  const selectedRows = rows.filter((row) => selectedIds.has(row.resourceId) && !removalIds.has(row.resourceId));
  const keptLocal = mode === "pull" ? rows.length - selectedRows.length : 0;

  const toggleDirectory = (row: ProjectFileReviewRow) => {
    setExpandedDirectories((current) => {
      const next = new Set(current);
      if (next.has(row.relativePath)) next.delete(row.relativePath);
      else next.add(row.relativePath);
      return next;
    });
  };
  const toggleDirectorySelection = (row: ProjectFileReviewRow) => {
    const descendants = rows.filter((candidate) => (
      candidate.relativePath === row.relativePath
      || candidate.relativePath.startsWith(`${row.relativePath}/`)
    ) && rowIsSelectable(candidate, mode, eligible) && !removalIds.has(candidate.resourceId));
    const shouldSelect = descendants.some((candidate) => !selectedIds.has(candidate.resourceId));
    if (onBulkToggle) {
      onBulkToggle(descendants.map((candidate) => candidate.resourceId), shouldSelect);
      return;
    }
    for (const candidate of descendants) {
      if (selectedIds.has(candidate.resourceId) !== shouldSelect) onToggle(candidate.resourceId);
    }
  };

  if (!scanned && mode === "push") {
    return (
      <section className="sync-project-files-page sync-project-files-unscanned" aria-labelledby="sync-project-files-title">
        <div className="sync-project-files-empty">
          <span className="sync-project-files-empty-icon" aria-hidden="true"><Icon name="folder" size={17} /></span>
          <div className="sync-project-files-empty-copy">
            <h3 id="sync-project-files-title">Files outside Git</h3>
          </div>
          <button type="button" className="btn btn-primary" onClick={onScan} disabled={disabled || loading || !eligible}>
            <Icon name={loading ? "refresh" : "folder"} size={14} className={loading ? "icon-spin" : undefined} />
            {loading ? "Scanning…" : "Scan files"}
          </button>
        </div>
        {!eligible && <ProjectFilesLockedNotice eligibility={eligibility} />}
      </section>
    );
  }

  return (
    <section className="sync-project-files-page" aria-labelledby="sync-project-files-title">
      <div className="sync-project-files-toolbar">
        <div className="sync-project-files-heading">
          <h3 id="sync-project-files-title">Project files</h3>
          {eligible && mode === "push" && (
            <span
              className="sync-project-files-note"
              role="img"
              aria-label="Review paths and warnings before Push; scanning cannot detect every secret."
              title="Review paths and warnings before Push; scanning cannot detect every secret."
            >
              <Icon name="info" size={13} />
            </span>
          )}
        </div>
        {mode === "push" && onScan && (
          <button type="button" className="btn btn-ghost" onClick={onScan} disabled={disabled || loading || !eligible}>
            <Icon name="refresh" size={14} className={loading ? "icon-spin" : undefined} /> {loading ? "Scanning…" : "Rescan"}
          </button>
        )}
        {mode === "pull" && onSelectSafe && (
          <button type="button" className="btn btn-ghost" onClick={onSelectSafe} disabled={disabled || loading || !eligible}>
            <Icon name="check-circle" size={14} /> Select safe additions
          </button>
        )}
      </div>

      {!eligible && <ProjectFilesLockedNotice eligibility={eligibility} withDetectedRoot />}

      {rows.length > 0 && (
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
          />
          <span>{rows.length} entr{rows.length === 1 ? "y" : "ies"}</span>
        </div>
      )}

      {rows.length === 0 ? (
        <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No changes</strong></span></div>
      ) : (
        /* A list, not an ARIA tree: rows contain focusable controls and provide
           no treeitem or arrow-key contract. Tab walks the controls. */
        <div className="sync-project-file-tree" role="list" aria-label="Project files">
          {shownRows.map((row) => {
            const isDirectory = row.entryType === "directory";
            const expanded = expandedDirectories.has(row.relativePath) || Boolean(normalizedSearch);
            const selected = selectedIds.has(row.resourceId) && !removalIds.has(row.resourceId);
            const required = requiredIds.has(row.resourceId);
            const removed = removalIds.has(row.resourceId);
            const selectable = rowIsSelectable(row, mode, eligible) && !removed;
            const detailsOpen = details.has(row.resourceId);
            const childCount = isDirectory ? descendantCounts.get(row.relativePath) ?? 0 : 0;
            const warningAcknowledged = !!row.warningDigest && acknowledgedWarningDigests.has(row.warningDigest);
            return (
              <div
                key={row.resourceId}
                className={`sync-project-file-row${selected ? " selected" : ""}${required ? " required" : ""}${removed ? " removal" : ""}${row.blockedReason ? " blocked" : ""}`}
                role="listitem"
                style={{ "--project-file-indent": `${depth(row.relativePath) * 16}px` } as React.CSSProperties}
              >
                <div className="sync-project-file-row-main">
                  {isDirectory ? (
                    <button type="button" className="sync-project-file-expand" aria-expanded={expanded} onClick={() => toggleDirectory(row)} aria-label={`${expanded ? "Collapse" : "Expand"} ${row.relativePath}`}>
                      <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
                    </button>
                  ) : <span className="sync-project-file-expand" />}
                  {selectable && !required ? (
                    <input
                      type="checkbox"
                      aria-label={`${mode === "pull" ? "Apply" : "Include"} ${row.relativePath}`}
                      checked={selected}
                      disabled={disabled}
                      onChange={() => isDirectory ? toggleDirectorySelection(row) : onToggle(row.resourceId)}
                    />
                  ) : null}
                  <Icon name={isDirectory ? "folder" : row.entryType === "blocked" ? "ban" : "file"} size={14} />
                  <button type="button" className="sync-project-file-name" onClick={() => setDetails((current) => {
                    const next = new Set(current);
                    if (next.has(row.resourceId)) next.delete(row.resourceId);
                    else next.add(row.resourceId);
                    return next;
                  })} aria-expanded={detailsOpen}>
                    <strong>{row.relativePath.split("/").slice(-1)[0]}</strong>
                    <small>{displayState(row, mode)}{required ? " · required directory" : ""}{childCount > 0 ? ` · ${childCount} descendant${childCount === 1 ? "" : "s"}` : ""}</small>
                  </button>
                  {formatOptionalBytes(row.size) && <span className="sync-project-file-size">{formatOptionalBytes(row.size)}</span>}
                  {row.warningCode && <span className="sync-project-file-warning"><Icon name="alert-triangle" size={13} /> Warning</span>}
                  {mode === "push" && row.storagePresent && onToggleRemoval && (
                    <button type="button" className={`btn btn-ghost sync-project-file-remove${removed ? " active" : ""}`} disabled={disabled || !eligible || required} onClick={() => onToggleRemoval(row.resourceId)}>
                      {removed ? "Keep in storage" : "Remove from storage"}
                    </button>
                  )}
                </div>
                {detailsOpen && (
                  <div className="sync-project-file-details">
                    <span>Path</span><code>{row.relativePath}</code>
                    <span>Type</span><code>{isDirectory ? "Directory" : row.entryType === "blocked" ? "Blocked entry" : "Regular file"}</code>
                    {formatMode(row.mode) && <><span>Mode</span><code>{formatMode(row.mode)}</code></>}
                    {row.sourceMtime != null && <><span>Modified</span><code>{formatDateTime(row.sourceMtime * 1_000)}</code></>}
                    {row.blockedReason && <><span>Blocked</span><code>{row.blockedReason}</code></>}
                  </div>
                )}
                {row.warningDigest && onToggleWarning && (
                  <label className="sync-project-file-ack">
                    <input type="checkbox" checked={warningAcknowledged} disabled={disabled || !selected} onChange={() => onToggleWarning(row.warningDigest!)} />
                    Reviewed this warning.
                  </label>
                )}
                {mode === "push" && STORAGE_BLOCKING_STATES.has(row.state) && onReviewPull && (
                  /* Warning, not error: nothing has failed — the push has not
                     run — the row simply cannot be included until a Pull
                     settles the storage side, which is action the reader may
                     need to take. `inline` rather than `standing` because this
                     marker repeats per row: a blocked scan draws dozens, and a
                     landmark each would hand a screen-reader user dozens of
                     regions to step through. The row already names the file. */
                  <Notice
                    severity="warning"
                    timing="inline"
                    className="sync-project-file-conflict"
                    action={<button type="button" className="btn btn-ghost" onClick={onReviewPull}>Open Pull review</button>}
                  >
                    Pull required
                  </Notice>
                )}
              </div>
            );
          })}
          {shownRows.length >= MAX_RENDERED_ROWS && rows.length > shownRows.length && (
            <div className="sync-project-file-limit">First {MAX_RENDERED_ROWS} shown. Search or collapse folders.</div>
          )}
        </div>
      )}

      {(ignoredCount > 0 || blockedCount > 0 || warnings.length > 0) && (
        <details className="sync-project-files-scan-summary">
          <summary>{ignoredCount} ignored · {blockedCount} blocked · {warnings.length} scan warning{warnings.length === 1 ? "" : "s"}</summary>
          {warnings.map((warning) => <p key={warning}>{warning}</p>)}
        </details>
      )}

      {rows.length > 0 && (
        <div className="sync-project-files-counts" aria-label={`${selectedRows.length} entries selected`}>
          <strong>{selectedRows.length} selected</strong>
          {mode === "pull" && keptLocal > 0 && <small>{keptLocal} local</small>}
          {mode === "push" && removalIds.size > 0 && <small>{removalIds.size} removal{removalIds.size === 1 ? "" : "s"}</small>}
        </div>
      )}
    </section>
  );
}
