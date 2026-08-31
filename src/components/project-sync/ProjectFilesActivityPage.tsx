import { useEffect, useMemo, useState } from "react";
import type {
  LocalProjectSummary,
  ProjectContentEntry,
  ProjectContentInventory,
} from "../../types";
import Icon from "../Icons";
import LoadingScaffold from "../LoadingScaffold";
import Notice from "../notice/Notice";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../sync-review/ThreadStatusIcon";
import { formatBytes } from "../../format";
import { formatDateTime } from "../../clock";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import { ProjectFilesLockedNotice } from "./ProjectFilesReviewPage";
import ProjectFileDetailsMenu from "./ProjectFileDetailsMenu";

/**
 * The Files face of a non-git project's activity page. Read-only on purpose:
 * selection, acknowledgement, and removal are review vocabulary, so this page
 * carries none of it — statuses, reveals, and readouts only, the same
 * standing-list contract as the Sessions face beside it.
 */

/** What this page hands the workspace path bar: its refresh, nothing else. */
export interface FilesActivityHeaderModel {
  refreshing: boolean;
  onRefresh: () => void;
}

interface PageProps {
  project: LocalProjectSummary;
  activeStorageId?: string | null;
  activeStorageName?: string | null;
  refreshEpoch: number;
  onHeaderModelChange?: (model: FilesActivityHeaderModel | null) => void;
  onRevealPath?: (path: string) => void;
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
      return { icon: "unavailable", tone: "unavailable", sentence: "In the sync recipe but missing from this folder." };
    default:
      return { icon: "unknown", tone: "unknown", sentence: "State unknown. Rescan to compare." };
  }
}

const WARNING_LABELS: Record<string, string> = {
  possible_secret: "Possible secret",
  executable: "Executable",
};

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

/** One directory band's worth of rows: the files directly inside it. */
interface FileGroup {
  directory: string;
  entries: ProjectContentEntry[];
}

/** Root files first, then each directory that directly holds files, in path
 * order — the flat full-path banding the review's folder treatment uses. */
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

function groupFacts(group: FileGroup): string {
  const bytes = group.entries.reduce((sum, entry) => sum + (entry.size ?? 0), 0);
  const count = `${group.entries.length} file${group.entries.length === 1 ? "" : "s"}`;
  return bytes > 0 ? `${count} · ${formatBytes(bytes)}` : count;
}

function FileGroupHeading({ group }: { group: FileGroup }) {
  return (
    <div className="sync-project-files-group-heading">
      <Icon name="folder" size={14} />
      <strong>{group.directory}/</strong>
      <small>{groupFacts(group)}</small>
    </div>
  );
}

function FileActivityCard({ entry, storageName, projectRoot, onRevealPath }: {
  entry: ProjectContentEntry;
  storageName: string;
  projectRoot: string;
  onRevealPath?: (path: string) => void;
}) {
  const presentation = filePresentation(entry, storageName);
  const warningLabel = entry.warning_code
    ? WARNING_LABELS[entry.warning_code] ?? entry.warning_code.replace(/_/g, " ")
    : null;
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
          <strong title={entry.relative_path}>{fileName(entry.relative_path)}</strong>
          {entry.blocked_reason && (
            <span className="sync-history-branch-state history-only" title={entry.blocked_reason}>Blocked</span>
          )}
          {warningLabel && (
            <span className="sync-project-file-warning"><Icon name="alert-triangle" size={13} /> {warningLabel}</span>
          )}
        </div>
        <div className="sync-history-thread-actions">
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!entry.local_present || !onRevealPath}
            title={entry.local_present ? `Reveal ${entry.relative_path} in Finder` : "Not on this Mac"}
            aria-label={entry.local_present
              ? `Reveal ${entry.relative_path} in Finder`
              : `Cannot reveal ${entry.relative_path}: not on this Mac`}
            onClick={() => onRevealPath?.(`${projectRoot}/${entry.relative_path}`)}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${entry.relative_path} details`}>
            {entry.size != null
              ? (
                <span
                  className="sync-history-thread-metric"
                  data-tooltip={`${entry.local_present ? "On disk" : `In ${storageName}`} · ${formatBytes(entry.size)}`}
                  tabIndex={0}
                >
                  <Icon name="drive" size={12} /><b>{formatBytes(entry.size)}</b>
                </span>
              )
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <ProjectFileDetailsMenu
              relativePath={entry.relative_path}
              entryTypeLabel={entry.entry_type === "blocked" ? "Blocked entry" : "Regular file"}
              mode={formatMode(entry.mode)}
              modified={entry.source_mtime == null ? null : formatDateTime(entry.source_mtime * 1_000)}
              blockedReason={entry.blocked_reason}
            />
          </div>
          {entry.source_mtime != null && (
            <span
              className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end"
              data-tooltip={`${entry.local_present ? "Modified" : `Updated in ${storageName}`} · ${formatDateTime(entry.source_mtime * 1_000)}`}
              tabIndex={0}
            >
              <time>{formatDateTime(entry.source_mtime * 1_000)}</time>
            </span>
          )}
        </div>
      </div>
    </article>
  );
}

/** The list itself, a pure function of the scan — exported for the DOM-less
 * integration tests, which render markup rather than run effects. */
export function ProjectFilesActivityView({
  inventory,
  storageName,
  loading,
  error,
  onRevealPath,
}: {
  inventory: ProjectContentInventory | null;
  storageName: string;
  loading: boolean;
  error: string | null;
  onRevealPath?: (path: string) => void;
}) {
  const [search, setSearch] = useState("");
  const normalizedSearch = search.trim().toLocaleLowerCase();
  /* Directory rows carry no bytes and no per-file identity, and the bands
     already speak for the folders — files (and blocked candidates) are the
     cards. */
  const fileEntries = useMemo(
    () => (inventory?.entries ?? []).filter((entry) => entry.entry_type !== "directory"),
    [inventory],
  );
  const shownEntries = useMemo(
    () => (normalizedSearch
      ? fileEntries.filter((entry) => entry.relative_path.toLocaleLowerCase().includes(normalizedSearch))
      : fileEntries),
    [fileEntries, normalizedSearch],
  );
  /* The storage side this Mac does not hold leads the list, the way the
     sessions face leads with its storage-only group. */
  const cloudGroups = groupByDirectory(shownEntries.filter((entry) => !entry.local_present && entry.storage_present));
  const localGroups = groupByDirectory(shownEntries.filter((entry) => entry.local_present || !entry.storage_present));
  const cloudCount = cloudGroups.reduce((sum, group) => sum + group.entries.length, 0);

  if (error) {
    return (
      <section className="sync-project-files-page" aria-label="Project files">
        {/* The scan the reader asked for did not happen; the box interrupts. */}
        <Notice severity="danger" timing="blocking">{error}</Notice>
      </section>
    );
  }
  if (!inventory) {
    return (
      <section className="sync-project-files-page" aria-label="Project files">
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
        <ProjectFilesLockedNotice eligibility={inventory.eligibility} withDetectedRoot />
      </section>
    );
  }

  const renderGroups = (groups: FileGroup[]) => groups.map((group) => (
    <div key={group.directory || "(root)"}>
      {group.directory !== "" && <FileGroupHeading group={group} />}
      <div className="sync-history-thread-list flat">
        {group.entries.map((entry) => (
          <FileActivityCard
            key={entry.descriptor.resource_id}
            entry={entry}
            storageName={storageName}
            projectRoot={inventory.project_root}
            onRevealPath={onRevealPath}
          />
        ))}
      </div>
    </div>
  ));

  return (
    <section className="sync-project-files-page" aria-label="Project files">
      {fileEntries.length > 0 && (
        <div className="sync-project-files-search">
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Escape" || !search) return;
              event.preventDefault();
              event.stopPropagation();
              setSearch("");
            }}
            placeholder="Search files…"
            aria-label="Search project file paths"
          />
          <span>{fileEntries.length} entr{fileEntries.length === 1 ? "y" : "ies"}</span>
        </div>
      )}
      {fileEntries.length === 0 ? (
        <div className="sync-pull-empty">
          <Icon name="check-circle" size={16} />
          <span><strong>No files</strong><small>Nothing in this folder syncs yet.</small></span>
        </div>
      ) : (
        <>
          {/* The catalog page's own partition grammar: the shared section
              heading, storage named in the running face, no counts and no
              phrase — each row's badge already says "Pull to download it
              here" against the storage by name. The local half takes a
              heading only while the partition exists; alone, the leaf crumb
              already names it. */}
          {cloudCount > 0 && (
            <div className="sync-history-section-heading">
              <h2>
                <Icon name="cloud" size={14} />
                Only in <span className="sync-history-storage-name">{storageName}</span>
              </h2>
            </div>
          )}
          {cloudCount > 0 && renderGroups(cloudGroups)}
          {cloudCount > 0 && localGroups.length > 0 && (
            <div className="sync-history-section-heading">
              <h2><Icon name="file" size={14} />Project files</h2>
            </div>
          )}
          {renderGroups(localGroups)}
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

export default function ProjectFilesActivityPage({
  project,
  activeStorageId = null,
  activeStorageName = null,
  refreshEpoch,
  onHeaderModelChange,
  onRevealPath,
}: PageProps) {
  const [inventory, setInventory] = useState<ProjectContentInventory | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [scanEpoch, setScanEpoch] = useState(0);

  useEffect(() => {
    if (!activeStorageId) {
      setInventory(null);
      setError(null);
      return;
    }
    let stale = false;
    setLoading(true);
    setError(null);
    projectSyncApi.inspectProjectFiles(project.local_project_id, activeStorageId)
      .then((inspected) => { if (!stale) setInventory(inspected); })
      .catch((reason) => { if (!stale) setError(errorMessage(reason)); })
      .finally(() => { if (!stale) setLoading(false); });
    return () => { stale = true; };
  }, [project.local_project_id, activeStorageId, refreshEpoch, scanEpoch]);

  /* The bar's one refresh follows the view; this page's is the rescan. */
  useEffect(() => {
    onHeaderModelChange?.({
      refreshing: loading,
      onRefresh: () => setScanEpoch((epoch) => epoch + 1),
    });
  }, [loading, onHeaderModelChange]);
  useEffect(() => () => { onHeaderModelChange?.(null); }, [onHeaderModelChange]);

  return (
    <ProjectFilesActivityView
      inventory={inventory}
      storageName={activeStorageName?.trim() || "storage"}
      loading={loading}
      error={error}
      onRevealPath={onRevealPath}
    />
  );
}
