import { useCallback, useDeferredValue, useEffect, useMemo, useState, type ReactNode } from "react";
import type {
  ProjectContentEntry,
  ProjectContentInventory,
} from "../../types";
import Icon from "../Icons";
import LoadingScaffold from "../LoadingScaffold";
import Notice, { NoticeChip } from "../notice/Notice";
import ListTile from "../sync-review/ListTile";
import NameSearchField from "../sync-review/NameSearchField";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../sync-review/ThreadStatusIcon";
import { nameMatchesSearch } from "../sync-review/nameSearch";
import { formatBytes } from "../../format";
import { formatFullDateTime } from "../../clock";
import { errorMessage } from "../../storageError";
import { PROJECT_FILE_CARD_RENDER_LIMIT, fileKindLabel, fileNoun, projectFileWarningLabel } from "./model";
import { projectSyncApi } from "./api";
import { ProjectFilesLockedNotice } from "./ProjectFilesReviewPage";
import ProjectFileDetailsMenu from "./ProjectFileDetailsMenu";
import SessionRowIcon from "./SessionRowIcon";

/**
 * The Files face of a non-git project's activity page. Read-only on purpose:
 * selection, acknowledgement, and removal are review vocabulary, so this page
 * carries none of it — statuses, reveals, and readouts only, the same
 * standing-list contract as the Sessions face beside it.
 *
 * Every file is a two-line session-grammar card — Kind and Size ride the
 * readout line in the session rows' glyphs, the way a session's messages and
 * tokens do — and every folder is a `ListTile`, the storage-only partition
 * leading as the cloud-tinted tile. Hierarchy is containment, never floating
 * headings. The search row is the Sessions face's own field, and the path
 * bar's refresh is this face's rescan, so the two faces share their chrome.
 */

/** One scan of a non-Git project's files against its storage. */
export interface ProjectFilesScan {
  inventory: ProjectContentInventory | null;
  loading: boolean;
  error: string | null;
  rescan: () => void;
}

/**
 * The workspace holds the scan, not the Files face: the path bar's Pull and
 * Push badges count these files on both faces, and the face renders this
 * same scan instead of running a second one. A null project stands the scan
 * down and drops its result, so a later scan starts clean. A rescan of the
 * same pair keeps the last inventory on screen until the new one lands.
 */
export function useProjectFilesScan(
  localProjectId: string | null,
  storageId: string | null,
  refreshEpoch: number,
): ProjectFilesScan {
  const key = localProjectId && storageId ? `${localProjectId}\n${storageId}` : null;
  const [scanned, setScanned] = useState<{
    key: string;
    inventory: ProjectContentInventory | null;
    error: string | null;
  } | null>(null);
  const [scanningKey, setScanningKey] = useState<string | null>(null);
  const [scanEpoch, setScanEpoch] = useState(0);

  useEffect(() => {
    if (!key || !localProjectId || !storageId) {
      setScanned(null);
      return;
    }
    let stale = false;
    setScanningKey(key);
    projectSyncApi.inspectProjectFiles(localProjectId, storageId)
      .then((inventory) => { if (!stale) setScanned({ key, inventory, error: null }); })
      .catch((reason) => { if (!stale) setScanned({ key, inventory: null, error: errorMessage(reason) }); })
      .finally(() => { if (!stale) setScanningKey(null); });
    return () => { stale = true; };
  }, [key, localProjectId, storageId, refreshEpoch, scanEpoch]);

  const rescan = useCallback(() => setScanEpoch((epoch) => epoch + 1), []);
  const current = scanned?.key === key ? scanned : null;
  return {
    inventory: current?.inventory ?? null,
    loading: key !== null && scanningKey === key,
    error: current?.error ?? null,
    rescan,
  };
}

/** Files a scan says Pull or Push would move. */
export interface ProjectFileSyncCounts {
  pull: number;
  push: number;
}

/**
 * Tallied in the rows' own vocabulary: a file only in storage or newer there
 * is one Pull brings here; a file only here or newer here is one Push sends.
 * Directory rows are not drawn, a blocked file moves with neither verb, and a
 * diverged or missing file waits on the reader's choice, so none is counted.
 */
export function projectFileSyncCounts(inventory: ProjectContentInventory | null): ProjectFileSyncCounts {
  const counts = { pull: 0, push: 0 };
  if (!inventory || inventory.eligibility.state !== "eligible") return counts;
  for (const entry of inventory.entries) {
    if (entry.entry_type === "directory") continue;
    switch (entry.state) {
      case "storage_only":
      case "storage_ahead":
        counts.pull += 1;
        break;
      case "local_only":
      case "local_ahead":
        counts.push += 1;
        break;
      default:
        break;
    }
  }
  return counts;
}

/** The row's status vocabulary, said against the storage by name — the same
 * sentences the session rows speak, so the two faces read as one page. */
function filePresentation(
  entry: ProjectContentEntry,
  storageName: string,
): { icon: ThreadStatusIconName; tone: string; sentence: string } {
  if (entry.blocked_reason || entry.state === "blocked") {
    return { icon: "unavailable", tone: "unavailable", sentence: `Unavailable for sync. ${entry.blocked_reason ?? ""}`.trim() };
  }
  switch (entry.state) {
    case "synced":
      return { icon: "synced", tone: "synced", sentence: `Up to date here and in ${storageName}.` };
    case "local_only":
      return { icon: "local-outstanding", tone: "local", sentence: `Only on this computer. Push to save it in ${storageName}.` };
    case "local_ahead":
      return { icon: "local-outstanding", tone: "local", sentence: `Newer on this computer. Push to update ${storageName}.` };
    case "storage_only":
      return { icon: "cloud-outstanding", tone: "storage", sentence: `Only in ${storageName}. Pull to download it here.` };
    case "storage_ahead":
      return { icon: "cloud-outstanding", tone: "storage", sentence: `Newer in ${storageName}. Pull to update this computer.` };
    case "diverged":
      return { icon: "divergent", tone: "diverged", sentence: `Changed here and in ${storageName}. Review before syncing.` };
    case "missing":
      return {
        icon: "cloud-outstanding",
        tone: "storage",
        sentence: `Not in this folder. It remains in ${storageName}. Pull to download it here, or remove it during Push.`,
      };
    default:
      return { icon: "unknown", tone: "unknown", sentence: "State unknown. Rescan to compare." };
  }
}

function parentPath(path: string): string {
  const separator = path.lastIndexOf("/");
  return separator < 0 ? "" : path.slice(0, separator);
}

function fileName(path: string): string {
  return path.split("/").slice(-1)[0];
}

function formatMode(mode?: number | null): string | null {
  return mode == null ? null : `0${mode.toString(8).padStart(3, "0")}`;
}

/** One framed tile's worth of rows: the files directly inside a directory. */
interface FileGroup {
  directory: string;
  entries: ProjectContentEntry[];
}

/** Root files first, then each directory that directly holds files, in path
 * order — the flat full-path grouping the review's folder treatment uses. */
function groupByDirectory(entries: ProjectContentEntry[]): FileGroup[] {
  const byDirectory = new Map<string, ProjectContentEntry[]>();
  for (const entry of entries) {
    const directory = parentPath(entry.relative_path);
    if (!byDirectory.has(directory)) byDirectory.set(directory, []);
    byDirectory.get(directory)!.push(entry);
  }
  return [...byDirectory.keys()]
    .sort((a, b) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)))
    .map((directory) => ({ directory, entries: byDirectory.get(directory)! }));
}

/** Search the complete file inventory — by path, word for word, the way the
 * Sessions face matches titles — then apply one global card budget before
 * the cloud/local partition and directory grouping. */
export function buildProjectFilesActivityProjection(
  fileEntries: ProjectContentEntry[],
  query: string,
) {
  const matchingEntries = query.trim()
    ? fileEntries.filter((entry) => nameMatchesSearch(entry.relative_path, query))
    : fileEntries;
  const visibleEntries = matchingEntries.slice(0, PROJECT_FILE_CARD_RENDER_LIMIT);
  const cloudEntries = visibleEntries.filter((entry) => (
    !entry.local_present && entry.storage_present
  ));
  const localEntries = visibleEntries.filter((entry) => (
    entry.local_present || !entry.storage_present
  ));
  return {
    matchingCount: matchingEntries.length,
    cloudEntries,
    localGroups: groupByDirectory(localEntries),
  };
}

function groupFacts(entries: ProjectContentEntry[]): string {
  const bytes = entries.reduce((sum, entry) => sum + (entry.size ?? 0), 0);
  const count = `${entries.length} ${fileNoun(entries.length)}`;
  return bytes > 0 ? `${count} · ${formatBytes(bytes)}` : count;
}

/** One file, the two-line session-grammar card: title line (status mark,
 * name, chips, reveal), then the readout line — Kind and Size chips where a
 * session carries its messages and tokens, the rule, the details popover,
 * and the stamp closing the row. Rows in the merged cloud tile keep their
 * directory as a quiet prefix, so a nested storage-only file still says
 * where it lands. */
function FileActivityCard({ entry, storageName, projectRoot, onOpenFile, onRevealPath, withPathPrefix = false }: {
  entry: ProjectContentEntry;
  storageName: string;
  projectRoot: string;
  onOpenFile?: (relativePath: string) => void;
  onRevealPath?: (path: string) => void;
  withPathPrefix?: boolean;
}) {
  const presentation = filePresentation(entry, storageName);
  const warningLabel = entry.warning_code ? projectFileWarningLabel(entry.warning_code) : null;
  const directory = parentPath(entry.relative_path);
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <span
            className={`sync-thread-sync-indicator ${presentation.tone}`}
            data-tooltip={presentation.sentence}
            aria-label={presentation.sentence}
            tabIndex={0}
          >
            <ThreadStatusIcon status={presentation.icon} size={18} />
          </span>
          {entry.local_present && onOpenFile ? (
            <button
              type="button"
              className="sync-history-session-title-button"
              title={`Open ${entry.relative_path}`}
              onClick={() => onOpenFile(entry.relative_path)}
            >
              {withPathPrefix && directory !== "" && <span>{directory}/</span>}
              {fileName(entry.relative_path)}
            </button>
          ) : (
            <strong title={entry.relative_path}>
              {withPathPrefix && directory !== "" && <span>{directory}/</span>}
              {fileName(entry.relative_path)}
            </strong>
          )}
          {entry.blocked_reason && (
            <span className="sync-history-branch-state history-only" title={entry.blocked_reason}>Blocked</span>
          )}
          {warningLabel && (
            <NoticeChip severity="warning">{warningLabel}</NoticeChip>
          )}
        </div>
        <div className="sync-history-thread-actions">
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!entry.local_present || !onRevealPath}
            title={entry.local_present ? `Show ${entry.relative_path} in Finder` : "Not on this Mac"}
            aria-label={entry.local_present
              ? `Show ${entry.relative_path} in Finder`
              : `Cannot show ${entry.relative_path}: not on this Mac`}
            onClick={() => onRevealPath?.(`${projectRoot}/${entry.relative_path}`)}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${entry.relative_path} details`}>
            <span
              className="sync-history-thread-metric"
              data-tooltip={`Kind · ${fileKindLabel(entry.relative_path)}`}
              tabIndex={0}
            >
              <SessionRowIcon name="file" /><b>{fileKindLabel(entry.relative_path)}</b>
            </span>
            {entry.size != null
              ? (
                <span
                  className="sync-history-thread-metric"
                  data-tooltip={`${entry.local_present ? "On disk" : `In ${storageName}`} · ${formatBytes(entry.size)}`}
                  tabIndex={0}
                >
                  <SessionRowIcon name="size" /><b>{formatBytes(entry.size)}</b>
                </span>
              )
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <ProjectFileDetailsMenu
              relativePath={entry.relative_path}
              projectRoot={projectRoot}
              entryTypeLabel={entry.entry_type === "blocked" ? "Blocked entry" : "Regular file"}
              mode={formatMode(entry.mode)}
              modified={entry.source_mtime == null ? null : formatFullDateTime(entry.source_mtime)}
              blockedReason={entry.blocked_reason}
            />
          </div>
          {entry.source_mtime != null && (
            <span
              className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end"
              data-tooltip={`${entry.local_present ? "Modified" : `Updated in ${storageName}`} · ${formatFullDateTime(entry.source_mtime)}`}
              tabIndex={0}
            >
              <time dateTime={new Date(entry.source_mtime * 1_000).toISOString()}>{formatFullDateTime(entry.source_mtime)}</time>
            </span>
          )}
        </div>
      </div>
    </article>
  );
}

/** The list itself, a pure function of the scan the workspace holds. */
export function ProjectFilesActivityView({
  inventory,
  storageName,
  loading,
  error,
  onOpenFile,
  onRevealPath,
  viewSwitch,
}: {
  inventory: ProjectContentInventory | null;
  storageName: string;
  loading: boolean;
  error: string | null;
  onOpenFile?: (relativePath: string) => void;
  onRevealPath?: (path: string) => void;
  viewSwitch?: ReactNode;
}) {
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  /* Directory rows carry no bytes and no per-file identity, and the tiles
     already speak for the folders — files (and blocked candidates) are the
     rows. */
  const fileEntries = useMemo(
    () => (inventory?.entries ?? []).filter((entry) => entry.entry_type !== "directory"),
    [inventory],
  );
  const { matchingCount, cloudEntries, localGroups } = useMemo(
    () => buildProjectFilesActivityProjection(fileEntries, deferredSearch),
    [fileEntries, deferredSearch],
  );
  /* The switch rides the search row when there is one, and stands alone at
     the top of every other state, so it never moves between the two faces. */
  const switchRow = viewSwitch ? <div className="sync-project-view-row">{viewSwitch}</div> : null;

  if (error) {
    return (
      <section className="sync-project-files-page" aria-label="Project files">
        {switchRow}
        {/* The scan the reader asked for did not happen; the box interrupts. */}
        <Notice severity="danger" timing="blocking">{error}</Notice>
      </section>
    );
  }
  if (!inventory) {
    return (
      <section className="sync-project-files-page" aria-label="Project files">
        {switchRow}
        {loading
          ? <LoadingScaffold shape="rows" label="Scanning project files" />
          : (
            /* Standing, not an event: there was no storage to compare against
               the moment the page rendered. */
            <Notice severity="info" timing="standing" label="Why file states are unavailable">
              Link a storage to scan this folder and compare its files.
            </Notice>
          )}
      </section>
    );
  }
  if (inventory.eligibility.state !== "eligible") {
    return (
      <section className="sync-project-files-page" aria-label="Project files">
        {switchRow}
        <ProjectFilesLockedNotice eligibility={inventory.eligibility} withDetectedRoot />
      </section>
    );
  }

  return (
    <section className="sync-project-files-page" aria-label="Project files">
      {fileEntries.length > 0 ? (
        /* The Sessions face's own search row: the switch leads it, and the
           path bar's refresh is this face's rescan, so neither face carries
           a second one. */
        <NameSearchField
          query={search}
          onChange={setSearch}
          label="Search files"
          matches={matchingCount}
          total={fileEntries.length}
          noun={fileNoun}
          leading={viewSwitch}
        />
      ) : switchRow}
      {fileEntries.length === 0 ? (
        <div className="sync-pull-empty">
          <Icon name="check-circle" size={16} />
          <span><strong>No files</strong><small>Nothing in this folder syncs yet.</small></span>
        </div>
      ) : matchingCount === 0 ? (
        <div className="sync-history-state">No matching files.</div>
      ) : (
        <>
          {cloudEntries.length > 0 && (
            /* No counts and no phrase in the head — each row's badge already
               says "Pull to download it here" against the storage by name. */
            <ListTile icon="cloud" cloud label={<>Only in <span className="sync-history-storage-name">{storageName}</span></>}>
              {cloudEntries.map((entry) => (
                <FileActivityCard
                  key={entry.descriptor.resource_id}
                  entry={entry}
                  storageName={storageName}
                  projectRoot={inventory.project_root}
                  onOpenFile={onOpenFile}
                  onRevealPath={onRevealPath}
                  withPathPrefix
                />
              ))}
            </ListTile>
          )}
          {localGroups.map((group) => (
            /* "Project root" is chrome and keeps the pixel face; a path is
               the user's data and reads in the normal face, via the head's
               span carve-out. A plain folder: this face exists only for
               projects Git does not manage. */
            <ListTile
              key={group.directory || "(root)"}
              icon="folder"
              label={group.directory === "" ? "Project root" : <span>{group.directory}/</span>}
              facts={groupFacts(group.entries)}
            >
              {group.entries.map((entry) => (
                <FileActivityCard
                  key={entry.descriptor.resource_id}
                  entry={entry}
                  storageName={storageName}
                  projectRoot={inventory.project_root}
                  onOpenFile={onOpenFile}
                  onRevealPath={onRevealPath}
                />
              ))}
            </ListTile>
          ))}
          {matchingCount > PROJECT_FILE_CARD_RENDER_LIMIT && (
            <div className="sync-project-file-limit">
              First {PROJECT_FILE_CARD_RENDER_LIMIT} of {matchingCount} files shown. Search to narrow the list.
            </div>
          )}
        </>
      )}
      {(inventory.ignored_count > 0 || inventory.blocked_count > 0 || inventory.warnings.length > 0) && (
        <details className="sync-project-files-scan-summary">
          <summary>
            {inventory.ignored_count} ignored · {inventory.blocked_count} blocked · {inventory.warnings.length} scan warning{inventory.warnings.length === 1 ? "" : "s"}
          </summary>
          {inventory.warnings.map((warning) => <p key={warning}>{warning}</p>)}
        </details>
      )}
    </section>
  );
}
