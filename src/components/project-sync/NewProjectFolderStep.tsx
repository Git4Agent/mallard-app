import { useEffect, useRef, useState } from "react";
import type {
  AgentFolder,
  LocalProjectSummary,
  ProjectDetail,
  ProviderDescriptor,
  ProviderProfileSummary,
  StorageSummary,
} from "../../types";
import Icon from "../Icons";
import ProviderMark from "../sync-review/ProviderMark";
import { projectSyncApi } from "./api";
import CloudProjectSource, { type CloneFallback } from "./CloudProjectSource";
import ProjectFolderField from "./ProjectFolderField";
import { ModeDoor } from "./StorageSettings";
import {
  compactFolderPath,
  folderChoice,
  folderVerdict,
  formatRecentStamp,
  isAppScratchFolder,
  readFolderInput,
  settledProbe,
  useProjectFolderProbe,
  type FolderChoice,
  type FolderInput,
} from "./projectFolder";
import { useHomePath } from "./ProjectSetupWorkspace";

/** Where a new project comes from: a folder this computer has, or a
 *  Mallard Cloud repository that gets its folder here. */
export type NewProjectSource = "folder" | "cloud";

interface Props {
  workflowEpoch: number;
  isWorkflowCurrent: (workflowEpoch: number) => boolean;
  projects: LocalProjectSummary[];
  providers: ProviderDescriptor[];
  profiles: ProviderProfileSummary[];
  storages: StorageSummary[];
  busy: boolean;
  /** The door open at first, and for From Mallard Cloud the account and
   *  the repository the page was opened for. */
  initialSource?: NewProjectSource;
  initialStorageId?: string | null;
  initialAddress?: string | null;
  onCancel: () => void;
  /** Make the draft for this folder, or resume the one already there. */
  onChoose: (projectRoot: string) => void;
  onOpenProject: (projectId: string) => void;
  /** From Mallard Cloud with no account yet: set one up. */
  onAddCloudAccount: () => void;
  onCloudProjectDone: (detail: ProjectDetail, fallback: CloneFallback | null, workflowEpoch: number) => void;
}

/** Rows while the list is folded: it never becomes a scroller of its own. */
const FIRST_FOLDERS = 6;

const folderName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/** What choosing a recent folder does; nothing when it cannot be used. */
function rowChoice(folder: AgentFolder, projects: LocalProjectSummary[]): FolderChoice | null {
  return folder.probe.kind === "folder"
    ? folderChoice(folder.probe, folder.probe.canonical_path, projects)
    : null;
}

/** Rows the field's text still matches: a path keeps the folders under it,
 *  anything else is a search of names and locations. */
function matchingFolders(
  folders: AgentFolder[],
  input: FolderInput,
  compact: (path: string) => string,
): AgentFolder[] {
  if (input.kind === "empty") return folders;
  if (input.kind === "path") {
    const prefix = input.path.toLowerCase();
    return folders.filter((folder) => folder.path.toLowerCase().startsWith(prefix));
  }
  const query = input.query.toLowerCase();
  return folders.filter((folder) => (
    folderName(folder.path).toLowerCase().includes(query)
    || compact(folder.path).toLowerCase().includes(query)
  ));
}

/**
 * The setup page before a draft exists, with two doors. On this computer:
 * one Folder band — the field, and the folders recent sessions ran in drawn
 * as the Repository band draws its choices. Pick a row or type a path, then
 * Use this folder; the draft is made (or resumed) and the rest of the page
 * follows. Nothing is written while the field is typed. From Mallard Cloud:
 * `CloudProjectSource`, a repository that gets its folder here.
 */
export default function NewProjectFolderStep({
  workflowEpoch,
  isWorkflowCurrent,
  projects,
  providers,
  profiles,
  storages,
  busy,
  initialSource = "folder",
  initialStorageId = null,
  initialAddress = null,
  onCancel,
  onChoose,
  onOpenProject,
  onAddCloudAccount,
  onCloudProjectDone,
}: Props) {
  const [source, setSource] = useState<NewProjectSource>(initialSource);
  const homePath = useHomePath();
  const [text, setText] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [folders, setFolders] = useState<AgentFolder[] | null>(null);
  const [showAll, setShowAll] = useState(false);
  const mountedRef = useRef(true);
  const isActive = () => mountedRef.current && isWorkflowCurrent(workflowEpoch);
  const compact = (path: string) => compactFolderPath(path, homePath);
  const input = readFolderInput(text, homePath);
  const probeState = useProjectFolderProbe(input.kind === "path" ? input.path : null, isActive);

  useEffect(() => {
    mountedRef.current = true;
    void projectSyncApi.listAgentFolders()
      .then((list) => { if (isActive()) setFolders(list.folders); })
      // The list is a convenience; the field works without it.
      .catch(() => { if (isActive()) setFolders([]); });
    return () => { mountedRef.current = false; };
  }, []);

  /* The list offers only what can be chosen, silently. App scratch folders
     wait for the field: a search or a path that matches one brings it in. */
  const filtered = input.kind !== "empty";
  const offered = (folder: AgentFolder) => (
    rowChoice(folder, projects) !== null && (filtered || !isAppScratchFolder(folder.path))
  );
  const listed = matchingFolders(folders ?? [], input, compact).filter(offered);
  const pickedFolder = picked ? listed.find((folder) => folder.path === picked) ?? null : null;
  /* Folded, the list is its first rows — plus the picked one when it sits
     further down, so folding never hides what the button will act on. */
  const firstRows = listed.slice(0, FIRST_FOLDERS);
  const visible = filtered || showAll
    ? listed
    : pickedFolder && !firstRows.includes(pickedFolder) ? [...firstRows, pickedFolder] : firstRows;

  const probe = settledProbe(input, probeState);
  const typedChoice = input.kind === "path" ? folderChoice(probe, input.path, projects) : null;
  // A search that leaves exactly one row is a pick.
  const soleChoice = input.kind === "search" && listed.length === 1 ? rowChoice(listed[0], projects) : null;
  const choice = pickedFolder ? rowChoice(pickedFolder, projects) : typedChoice ?? soleChoice;
  const verdict = folderVerdict(input, probeState, { compact, projects });
  // A folder that is already a project can still take another agent profile.
  const anotherProfilePath = probe?.kind === "folder" ? probe.canonical_path : null;

  const commit = (target: FolderChoice | null) => {
    if (!target || busy) return;
    if (target.kind === "open") onOpenProject(target.projectId);
    else onChoose(target.path);
  };

  /* A Browse result is a choice already made: a folder setup accepts goes
     straight on; anything else stays in the field with its verdict. */
  const browsed = async (path: string) => {
    setText(compact(path));
    setPicked(null);
    try {
      const picks = folderChoice(await projectSyncApi.probeProjectFolder(path), path, projects);
      if (isActive() && picks?.kind === "setup") onChoose(path);
    } catch {
      // The field's own probe reports the failure.
    }
  };

  const primaryLabel = choice?.kind === "open"
    ? `Open ${choice.name}`
    : choice?.kind === "setup" && choice.resumes
      ? "Resume setup"
      : "Use this folder";
  const providerName = (key: string) => (
    providers.find((descriptor) => descriptor.key === key)?.display_name ?? key
  );
  const doors = (
    <div className="storage-mode-doors" role="radiogroup" aria-label="Where the project comes from">
      <ModeDoor
        icon="folder"
        title="On this computer"
        detail="A folder you have."
        checked={source === "folder"}
        disabled={busy}
        onChoose={() => setSource("folder")}
      />
      <ModeDoor
        icon="storage-mallard"
        title="From Mallard Cloud"
        detail="A repository you can open."
        checked={source === "cloud"}
        disabled={busy}
        onChoose={() => setSource("cloud")}
      />
    </div>
  );
  return (
    <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-labelledby="sync-new-project-title">
      <header className="sync-modal-header">
        <div>
          <h1 id="sync-new-project-title">New project</h1>
        </div>
        <button type="button" className="btn btn-ghost" onClick={onCancel} aria-label="Close; nothing has been saved">
          <Icon name="x" size={17} />
        </button>
      </header>

      {source === "cloud" ? (
        <CloudProjectSource
          doors={doors}
          storages={storages}
          initialStorageId={initialStorageId}
          initialAddress={initialAddress}
          providers={providers}
          profiles={profiles}
          projects={projects}
          busy={busy}
          workflowEpoch={workflowEpoch}
          isWorkflowCurrent={isWorkflowCurrent}
          onCancel={onCancel}
          onAddAccount={onAddCloudAccount}
          onOpenProject={onOpenProject}
          onDone={onCloudProjectDone}
        />
      ) : (
        <>
          <div className="sync-modal-body">
            {doors}
            <section className="sync-setup-section">
              <div className="sync-card-heading">
                <div><strong>Folder</strong></div>
              </div>
              <ProjectFolderField
                value={text}
                verdict={verdict}
                disabled={busy}
                autoFocus
                labelHidden
                placeholder={folders && folders.length > 0 ? "Path or search" : "Path, such as ~/code/app"}
                onChange={(next) => {
                  setText(next);
                  setPicked(null);
                }}
                onEnter={() => commit(choice)}
                onBrowsed={(path) => void browsed(path)}
                onRun={(run) => {
                  if (run === "another-profile" && anotherProfilePath) onChoose(anotherProfilePath);
                }}
              />
              {folders === null ? (
                <div className="sync-folder-choices-state">
                  <span className="status-loader" /> Finding recent folders…
                </div>
              ) : folders.length > 0 && (
                /* A machine with no sessions yet has nothing to offer: the field alone. */
                <div className="sync-folder-choices">
                  <div className="sync-folder-choices-head" id="sync-folder-choices-title">
                    Recent
                    <small>{listed.length}</small>
                  </div>
                  {listed.length === 0 ? (
                    <div className="sync-folder-choices-state">No recent folder matches.</div>
                  ) : (
                    <div className="sync-bundle-match-options" role="radiogroup" aria-labelledby="sync-folder-choices-title">
                      {visible.map((folder) => {
                        const rowTarget = rowChoice(folder, projects);
                        const active = pickedFolder?.path === folder.path;
                        const name = folderName(folder.path);
                        const stamp = formatRecentStamp(folder.last_active);
                        const isGit = folder.probe.kind === "folder" && folder.probe.is_git_repository;
                        const badge = rowTarget?.kind === "open"
                          ? "Added"
                          : rowTarget?.kind === "setup" && rowTarget.resumes ? "Draft" : null;
                        return (
                          <button
                            key={folder.path}
                            type="button"
                            role="radio"
                            aria-checked={active}
                            aria-label={[name, badge, compact(folder.path), stamp].filter(Boolean).join(", ")}
                            className={`sync-bundle-match-option${active ? " active" : ""}`}
                            disabled={busy}
                            onClick={() => setPicked(folder.path)}
                            onDoubleClick={() => commit(rowTarget)}
                          >
                            <span className="sync-bundle-radio"><span /></span>
                            <span className="setup-choice-mark" aria-hidden="true">
                              <Icon name={isGit ? "git-folder" : "folder"} size={24} />
                            </span>
                            <span className="sync-bundle-match-copy">
                              <strong>{name}{badge && <small className="sync-bundle-recommended">{badge}</small>}</strong>
                              <span>{compact(folder.path)}</span>
                            </span>
                            <span className="sync-bundle-match-meta">
                              {stamp}
                              <span className="sync-folder-choice-marks">
                                {folder.providers.map((provider) => (
                                  <ProviderMark key={provider} provider={provider} displayName={providerName(provider)} size={12} />
                                ))}
                              </span>
                            </span>
                          </button>
                        );
                      })}
                      {!filtered && listed.length > FIRST_FOLDERS && (
                        <button
                          type="button"
                          className="sync-folder-choices-more"
                          aria-expanded={showAll}
                          onClick={() => setShowAll((open) => !open)}
                          disabled={busy}
                        >
                          <Icon name={showAll ? "chevron-up" : "chevron-down"} size={12} />
                          {showAll ? "Show less" : `Show all ${listed.length}`}
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}
            </section>
          </div>

          <footer className="sync-modal-footer sync-setup-footer">
            <div>
              <button type="button" className="btn btn-ghost" onClick={onCancel}>
                Cancel
              </button>
              <button type="button" className="btn btn-primary" disabled={!choice || busy} onClick={() => commit(choice)}>
                {primaryLabel}
              </button>
            </div>
          </footer>
        </>
      )}
    </section>
  );
}
