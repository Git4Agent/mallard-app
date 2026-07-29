import { useMemo, useState } from "react";
import type { ProjectFileSyncEligibility } from "../../types";
import Icon from "../Icons";

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
const STORAGE_BLOCKING_STATES = new Set(["storage_only", "storage_ahead", "diverged", "unknown"]);

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

function formatBytes(value?: number | null): string | null {
  if (value == null) return null;
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(value < 10 * 1024 ? 1 : 0)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function formatMode(mode?: number | null): string | null {
  return mode == null ? null : `0${mode.toString(8).padStart(3, "0")}`;
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
        {!eligible && (
          <div className="sync-callout error" role="alert"><Icon name="lock" size={15} /> <span><strong>Project files are locked.</strong>{eligibility.reason}</span></div>
        )}
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

      {!eligible && (
        <div className="sync-callout error sync-project-files-locked" role="alert">
          <Icon name="lock" size={15} />
          <span><strong>Project files are locked.</strong>{eligibility.reason}{eligibility.detected_root ? ` Git root: ${eligibility.detected_root}` : ""}</span>
        </div>
      )}

      {rows.length > 0 && (
        <div className="sync-project-files-search">
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search files…"
            aria-label="Search project file paths"
          />
          <span>{rows.length} entr{rows.length === 1 ? "y" : "ies"}</span>
        </div>
      )}

      {rows.length === 0 ? (
        <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No changes</strong></span></div>
      ) : (
        <div className="sync-project-file-tree" role="tree" aria-label="Project files">
          {shownRows.map((row) => {
            const isDirectory = row.entryType === "directory";
            const expanded = expandedDirectories.has(row.relativePath) || Boolean(normalizedSearch);
            const selected = selectedIds.has(row.resourceId) && !removalIds.has(row.resourceId);
            const required = requiredIds.has(row.resourceId);
            const removed = removalIds.has(row.resourceId);
            const selectable = rowIsSelectable(row, mode, eligible) && !removed;
            const detailsOpen = details.has(row.resourceId);
            const childCount = isDirectory ? rows.filter((candidate) => candidate.relativePath.startsWith(`${row.relativePath}/`)).length : 0;
            const warningAcknowledged = !!row.warningDigest && acknowledgedWarningDigests.has(row.warningDigest);
            return (
              <div
                key={row.resourceId}
                className={`sync-project-file-row${selected ? " selected" : ""}${required ? " required" : ""}${removed ? " removal" : ""}${row.blockedReason ? " blocked" : ""}`}
                role="treeitem"
                aria-level={depth(row.relativePath) + 1}
                aria-expanded={isDirectory ? expanded : undefined}
                style={{ "--project-file-indent": `${depth(row.relativePath) * 16}px` } as React.CSSProperties}
              >
                <div className="sync-project-file-row-main">
                  {isDirectory ? (
                    <button type="button" className="sync-project-file-expand" onClick={() => toggleDirectory(row)} aria-label={`${expanded ? "Collapse" : "Expand"} ${row.relativePath}`}>
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
                  {formatBytes(row.size) && <span className="sync-project-file-size">{formatBytes(row.size)}</span>}
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
                    {row.sourceMtime != null && <><span>Modified</span><code>{new Date(row.sourceMtime * 1000).toLocaleString()}</code></>}
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
                  <div className="sync-project-file-conflict"><span>Pull required</span><button type="button" className="btn btn-ghost" onClick={onReviewPull}>Review Pull</button></div>
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
