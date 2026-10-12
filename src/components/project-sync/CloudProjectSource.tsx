import { useEffect, useRef, useState, type ReactNode } from "react";
import type {
  CloneFailure,
  CloneFailureReason,
  CloudProjectFolder,
  CloudProjectFolderProbe,
  CloudProjectFolderProgress,
  DraftProfileSelection,
  LocalProjectSummary,
  ManagedCloudRepository,
  ProjectDetail,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfileSummary,
  StorageSummary,
} from "../../types";
import Icon from "../Icons";
import { t as translate, useTranslation } from "../../i18n";
import { backendText } from "../../i18n/backend";
import Notice, { Acknowledgement, FieldMessage } from "../notice/Notice";
import ProviderMark from "../sync-review/ProviderMark";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import { repositoryFingerprintsRequireAcknowledgement } from "./model";
import { storageProviderName } from "./storageText";
import ProjectFolderField from "./ProjectFolderField";
import {
  compactFolderPath,
  readFolderInput,
  type FolderInput,
  type FolderRun,
  type FolderVerdict,
} from "./projectFolder";
import { useHomePath, useProfilePathProbes } from "./ProjectSetupWorkspace";
import RepositoryAddressPicker, {
  isManagedCloudStorage,
  useManagedCloudRepositories,
  type RepositorySelection,
} from "./RepositoryAddressPicker";

/** A Git repository whose source code is not downloaded. */
export interface CloneFallback {
  projectName: string;
  failure: CloneFailure;
}

/** Why the folder is empty, in a few words. */
export function cloneFailureText(reason: CloneFailureReason): string {
  switch (reason) {
    case "missing_source": return translate("projectSetup:clone.missingSource");
    case "refused": return translate("projectSetup:clone.refused");
    case "unreachable": return translate("projectSetup:clone.unreachable");
    case "failed": return translate("projectSetup:clone.failed");
  }
}

/** New folder: cloned or made here. Existing folder: one this computer
 *  already has, connected as it is. */
export type FolderMode = "new" | "existing";

interface Props {
  /** New project's two ways in, drawn at the top of the body. */
  doors: ReactNode;
  storages: StorageSummary[];
  /** The account to start on, such as the one Finish setup just added. */
  initialStorageId: string | null;
  /** The repository the page was opened for (a link from the website),
   *  looked up as a pasted address is once an account is chosen. */
  initialAddress?: string | null;
  providers: ProviderDescriptor[];
  profiles: ProviderProfileSummary[];
  projects: LocalProjectSummary[];
  busy: boolean;
  workflowEpoch: number;
  isWorkflowCurrent: (workflowEpoch: number) => boolean;
  onCancel: () => void;
  /** No Mallard Cloud account yet: set one up. */
  onAddAccount: () => void;
  /** The folder is already a project: go to it instead. */
  onOpenProject: (projectId: string) => void;
  onDone: (detail: ProjectDetail, fallback: CloneFallback | null, workflowEpoch: number) => void;
}

const PROBE_DELAY_MS = 320;

type ProbeState =
  | { status: "idle" }
  | { status: "checking"; key: string }
  | { status: "done"; key: string; probe: CloudProjectFolderProbe }
  | { status: "error"; key: string; message: string };

type Run =
  | { phase: "idle" }
  | { phase: "folder"; progress: CloudProjectFolderProgress | null }
  | { phase: "connect" }
  /** What failed, said when it is drawn; the backend's sentence, or none
   *  when the folder only changed under the line. */
  | { phase: "failed"; failure: "changed" | "connect" | "folder"; message: string | null };

const probeKey = (path: string, fingerprint: string | null, round: number) => `${path}\n${fingerprint ?? ""}\n${round}`;

/** The Folder field's verdict for the destination, read without writing. */
function useCloudFolderProbe(
  path: string | null,
  fingerprint: string | null,
  round: number,
  isActive: () => boolean,
): ProbeState {
  const [state, setState] = useState<ProbeState>({ status: "idle" });
  const requestRef = useRef(0);
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  useEffect(() => {
    const requestId = ++requestRef.current;
    if (!path) {
      setState({ status: "idle" });
      return;
    }
    const key = probeKey(path, fingerprint, round);
    setState({ status: "checking", key });
    const timer = window.setTimeout(() => {
      void projectSyncApi.probeCloudProjectFolder(path, fingerprint).then(
        (probe) => {
          if (requestRef.current === requestId && isActiveRef.current()) setState({ status: "done", key, probe });
        },
        (reason) => {
          if (requestRef.current === requestId && isActiveRef.current()) {
            setState({ status: "error", key, message: errorMessage(reason) });
          }
        },
      );
    }, PROBE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [path, fingerprint, round]);
  return state;
}

/** The folder most projects here sit in; the home folder otherwise. No
 *  setting: a new project goes beside the ones already on this computer. */
export function defaultProjectParent(projects: LocalProjectSummary[], home: string | null): string | null {
  const counts = new Map<string, number>();
  for (const project of projects) {
    const root = project.canonical_project_root ?? project.project_root;
    const parent = root ? root.replace(/[\\/][^\\/]+[\\/]?$/, "") : "";
    if (parent && parent !== root) counts.set(parent, (counts.get(parent) ?? 0) + 1);
  }
  let best: string | null = null;
  let most = 0;
  for (const [parent, count] of counts) {
    if (count > most) {
      best = parent;
      most = count;
    }
  }
  return best ?? home;
}

export function joinFolder(parent: string, name: string): string {
  const separator = parent.includes("\\") && !parent.includes("/") ? "\\" : "/";
  return `${parent.replace(/[\\/]+$/, "")}${separator}${name}`;
}

/** What Browse picked, as the field's path: for a new folder the parent,
 *  keeping the repository's name; for an existing one the folder itself. */
export function browsedPath(mode: FolderMode, picked: string, name: string): string {
  return mode === "existing" ? picked : joinFolder(picked, name);
}

/** Whether the click will try to clone: a GitHub repository, with Git here. */
export function clonesFrom(repository: ManagedCloudRepository, canClone: boolean): string | null {
  return repository.repository_class === "git_project" && repository.github_url && canClone
    ? repository.github_url.replace(/^https:\/\//, "")
    : null;
}

/** Whether an existing folder connects only once the person ticks setup's
 *  acknowledgement: it holds files, and its remote isn't this repository's. */
export function needsTick(
  mode: FolderMode,
  folder: CloudProjectFolder | null,
  repository: ManagedCloudRepository,
): boolean {
  return mode === "existing" && folder?.kind === "occupied"
    && repositoryFingerprintsRequireAcknowledgement(repository.repository_fingerprint, folder.local_fingerprint);
}

/** What the Folder line says: where the project lands, and how. */
export function destinationVerdict(
  input: FolderInput,
  state: ProbeState,
  key: string | null,
  repository: ManagedCloudRepository,
  compact: (path: string) => string,
  mode: FolderMode,
): FolderVerdict | null {
  if (input.kind === "empty") return null;
  if (input.kind === "search") return { tone: "bad", message: translate("projectSetup:destination.fullPath") };
  if (state.status === "error" && state.key === key) return { tone: "bad", message: state.message };
  if (state.status !== "done" || state.key !== key) return null;
  const { folder, can_clone: canClone } = state.probe;
  const git = repository.repository_class === "git_project";
  switch (folder.kind) {
    case "refused":
      return { tone: "bad", message: folder.message };
    case "project":
      return {
        tone: "bad",
        message: translate("projectSetup:destination.alreadyProject"),
        actions: [{ label: translate("projectSetup:destination.openIt"), run: "open-project" }],
      };
    case "checkout":
      return { tone: "ok", message: translate("projectSetup:destination.checkout") };
  }
  if (mode === "existing") {
    switch (folder.kind) {
      case "new":
        return { tone: "bad", message: translate("projectSetup:destination.noFolder") };
      case "empty":
        return { tone: "ok", message: translate("projectSetup:destination.emptyNoClone") };
      case "occupied":
        if (!needsTick(mode, folder, repository)) return { tone: "ok", message: translate("projectSetup:destination.asIs") };
        if (!folder.local_fingerprint) return { tone: "warn", message: translate("projectSetup:destination.noRemote") };
        if (!git) return { tone: "warn", message: translate("projectSetup:destination.noGit", { name: repository.name }) };
        return {
          tone: "warn",
          message: folder.git_remote
            ? translate("projectSetup:destination.remote", { remote: folder.git_remote })
            : translate("projectSetup:destination.otherRemote"),
        };
    }
  }
  const source = clonesFrom(repository, canClone);
  switch (folder.kind) {
    case "occupied":
      return {
        tone: "warn",
        message: translate("projectSetup:destination.holdsFiles"),
        actions: [
          { label: translate("projectSetup:destination.useAsIs"), run: "use-as-is" },
          ...(folder.suggestion
            ? [{ label: translate("projectSetup:folder.use", { path: compact(folder.suggestion) }), value: compact(folder.suggestion) }]
            : []),
        ],
      };
    case "new":
    case "empty":
      if (source) return { tone: "ok", message: translate("projectSetup:destination.clones", { source }) };
      if (git && !canClone) return { tone: "bad", message: translate("projectSetup:destination.installGit") };
      if (git) return { tone: "warn", message: translate("projectSetup:destination.emptyGit") };
      return {
        tone: "ok",
        message: folder.kind === "new" ? translate("projectSetup:destination.newEmpty") : translate("projectSetup:destination.emptyFolder"),
      };
  }
}

/** Setup's Git-remote acknowledgement, from what the folder holds when it
 *  connects: an empty folder (made here or found) has nothing to verify, its
 *  checkout matches, and one that holds files is acknowledged only by the
 *  person's tick. */
export function mismatchAcknowledged(holds: "empty" | "checkout" | "occupied", ticked: boolean): boolean {
  return holds === "empty" || (holds === "occupied" && ticked);
}

export interface ConnectChoices {
  storageId: string;
  bundleId: string;
  repository: ManagedCloudRepository;
  profiles: Record<ProviderKey, DraftProfileSelection>;
}

/** Setup's finish on the folder: the draft (or the one already there), this
 *  page's choices, and Finish. A failure keeps the folder. */
export async function connectFolder(projectRoot: string, choices: ConnectChoices, acknowledged: boolean): Promise<ProjectDetail> {
  const { draft } = await projectSyncApi.createSetupDraft(projectRoot);
  const saved = await projectSyncApi.updateSetupDraft({
    ...draft,
    profiles: choices.profiles,
    storage: { storage_id: choices.storageId },
    repository: {
      kind: "existing",
      storage_id: choices.storageId,
      bundle_id: choices.bundleId,
      display_name: choices.repository.address,
      repository_fingerprint: choices.repository.repository_fingerprint,
      mismatch_acknowledged: acknowledged,
    },
  });
  return projectSyncApi.finalizeProjectSetup(saved.draft_id, saved.revision);
}

/** Existing folder: asks about it again — the line may be stale — and
 *  connects it as it is only while the answer is the one shown and the tick
 *  still covers it; null when it changed. Nothing in the folder is touched,
 *  and the path keeps the spelling it was typed in. */
export async function connectExistingFolder(
  path: string,
  shown: CloudProjectFolder,
  ticked: boolean,
  choices: ConnectChoices,
): Promise<ProjectDetail | null> {
  const { folder } = await projectSyncApi.probeCloudProjectFolder(path, choices.repository.repository_fingerprint);
  if (folder.kind !== shown.kind) return null;
  if (folder.kind !== "empty" && folder.kind !== "checkout" && folder.kind !== "occupied") return null;
  if (needsTick("existing", folder, choices.repository) && !ticked) return null;
  return connectFolder(path, choices, mismatchAcknowledged(folder.kind, ticked));
}

interface AgentChoices {
  enabled: Record<ProviderKey, boolean>;
  chosen: Record<ProviderKey, string>;
}

/** Prefer the resolved default home, then the sole usable profile.
 *  Explicit choices stay in place when the default-home probes finish. */
export function defaultAgents(
  providers: ProviderDescriptor[],
  profiles: ProviderProfileSummary[],
  defaultPaths: Record<ProviderKey, string>,
  choices: AgentChoices,
): AgentChoices {
  const enabled: Record<ProviderKey, boolean> = {};
  const chosen: Record<ProviderKey, string> = {};
  for (const descriptor of providers) {
    const options = usableProfiles(profiles, descriptor.key);
    enabled[descriptor.key] = descriptor.setup_default && options.length > 0;
    const preferred = options.find((profile) => profile.canonical_path === defaultPaths[descriptor.key]);
    chosen[descriptor.key] = preferred?.profile_id ?? (options.length === 1 ? options[0].profile_id : "");
  }
  return { enabled: { ...enabled, ...choices.enabled }, chosen: { ...chosen, ...choices.chosen } };
}

function usableProfiles(profiles: ProviderProfileSummary[], provider: ProviderKey) {
  return profiles.filter((profile) => profile.provider === provider && profile.available && profile.readable);
}

let operationSequence = 0;
function newOperationId(): string {
  operationSequence += 1;
  return `cloud-project-${Date.now().toString(36)}-${operationSequence}`;
}

/**
 * New project › From Mallard Cloud: a project whose repository is on
 * Mallard Cloud. Setup's own bands in the order this needs them — the
 * Mallard Cloud account first (Storage), the repository by its address
 * (Repository), its folder (Project), the agent profiles (Agent) — and one
 * verb that connects. The folder is a new one — a GitHub repository is
 * cloned, with gh and then git; anything else, or a clone that does not
 * work, gets an empty folder — or one this computer already has, connected
 * as it is. Nothing is pulled: the project opens on Activity like any
 * finished setup.
 */
export default function CloudProjectSource({
  doors,
  storages,
  initialStorageId,
  initialAddress = null,
  providers,
  profiles,
  projects,
  busy,
  workflowEpoch,
  isWorkflowCurrent,
  onCancel,
  onAddAccount,
  onOpenProject,
  onDone,
}: Props) {
  const { t } = useTranslation(["projectSetup", "common"]);
  const homePath = useHomePath();
  const compact = (path: string) => compactFolderPath(path, homePath);
  const accounts = storages.filter(isManagedCloudStorage);
  const [storageId, setStorageId] = useState<string | null>(() => (
    initialStorageId && accounts.some((storage) => storage.storage_id === initialStorageId)
      ? initialStorageId
      : accounts.length === 1 ? accounts[0].storage_id : null
  ));
  const storage = accounts.find((candidate) => candidate.storage_id === storageId) ?? null;
  const mountedRef = useRef(true);
  const isActive = () => mountedRef.current && isWorkflowCurrent(workflowEpoch);
  const { listing, refresh } = useManagedCloudRepositories(storage?.storage_id ?? null, null, isActive);
  const [selection, setSelection] = useState<RepositorySelection | null>(null);
  const [repository, setRepository] = useState<ManagedCloudRepository | null>(null);
  const [mode, setMode] = useState<FolderMode>("new");
  const [folderText, setFolderText] = useState("");
  const suggestedFolder = useRef("");
  // The tick belongs to one folder for one repository.
  const [tickedFor, setTickedFor] = useState<string | null>(null);
  const [agentChoices, setAgents] = useState<AgentChoices>({ enabled: {}, chosen: {} });
  const defaultProbes = useProfilePathProbes(providers.flatMap((descriptor) => (
    homePath && descriptor.setup_default && descriptor.default_profile_directory
      && usableProfiles(profiles, descriptor.key).length > 1
      ? [{ provider: descriptor.key, path: joinFolder(homePath, descriptor.default_profile_directory) }]
      : []
  )), isActive);
  const defaultPaths = Object.fromEntries(Object.entries(defaultProbes).flatMap(([provider, state]) => (
    state.status === "ok" && state.probe.readable ? [[provider, state.probe.canonical_path]] : []
  )));
  const agents = defaultAgents(providers, profiles, defaultPaths, agentChoices);
  const [run, setRun] = useState<Run>({ phase: "idle" });
  const [probeRound, setProbeRound] = useState(0);
  const operationRef = useRef<string | null>(null);
  // Cancel pressed during this run: its refusal is not a failure to show.
  const cancelledRef = useRef(false);

  // The only account is chosen as soon as it exists.
  useEffect(() => {
    if (!storageId && accounts.length === 1) setStorageId(accounts[0].storage_id);
  }, [accounts.length]);

  /* Leaving mid-clone stops the clone; git removes what it wrote. */
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (operationRef.current) void projectSyncApi.cancelCloudProjectFolder(operationRef.current).catch(() => {});
    };
  }, []);

  const input = readFolderInput(folderText, homePath);
  const destination = input.kind === "path" ? input.path : null;
  const fingerprint = repository?.repository_fingerprint ?? null;
  const probeState = useCloudFolderProbe(repository ? destination : null, fingerprint, probeRound, isActive);
  const key = destination ? probeKey(destination, fingerprint, probeRound) : null;
  const probe = probeState.status === "done" && probeState.key === key ? probeState.probe : null;
  const verdict = repository ? destinationVerdict(input, probeState, key, repository, compact, mode) : null;

  const running = run.phase === "folder" || run.phase === "connect";
  const working = busy || running;
  const existing = mode === "existing";
  const folderKind = probe?.folder.kind ?? null;
  const source = repository && probe ? clonesFrom(repository, probe.can_clone) : null;
  const willClone = !existing && !!source && (folderKind === "new" || folderKind === "empty");
  const tick = !!repository && needsTick(mode, probe?.folder ?? null, repository);
  const tickFor = destination && repository ? `${destination}\n${repository.repository_id}` : null;
  const ticked = tickFor !== null && tickedFor === tickFor;

  const enabledProviders = providers.filter((descriptor) => agents.enabled[descriptor.key]);
  const incomplete = enabledProviders.filter((descriptor) => !agents.chosen[descriptor.key]);
  const agentMessage = enabledProviders.length === 0
    ? t("agents.noneOn")
    : incomplete.length > 0
      ? t("agents.chooseFor", { providers: incomplete.map((descriptor) => descriptor.display_name).join(t("shared.separator")) })
      : null;

  // Named for what the click will do, once the folder's verdict says it.
  const verb = existing || !repository || !probe || folderKind === "checkout"
    ? t("cloudSource.connect")
    : willClone ? t("cloudSource.cloneConnect")
      : repository.repository_class === "git_project" ? t("cloudSource.createGit") : t("cloudSource.createFolder");
  const connectable = existing
    ? folderKind === "empty" || folderKind === "checkout" || (folderKind === "occupied" && (!tick || ticked))
    : folderKind === "new" || folderKind === "empty" || folderKind === "checkout";
  const ready = !!storage && !!repository?.bundle_id && !!destination && connectable && verdict?.tone !== "bad" && agentMessage === null;

  const chooseStorage = (next: string) => {
    if (next === storageId) return;
    // Repositories belong to one account: a new account starts over.
    setStorageId(next);
    setSelection(null);
    setRepository(null);
  };

  const chooseRepository = (next: RepositorySelection | null, chosen: ManagedCloudRepository | null) => {
    setSelection(next);
    const picked = next?.kind === "existing" ? chosen : null;
    setRepository(picked);
    if (!picked) return;
    // The folder follows the repository until it is edited.
    const parent = defaultProjectParent(projects, homePath);
    const suggestion = parent ? compact(joinFolder(parent, picked.name)) : "";
    if (!folderText || folderText === suggestedFolder.current) setFolderText(suggestion);
    suggestedFolder.current = suggestion;
  };

  const runFolderAction = (action: FolderRun) => {
    if (action === "use-as-is") setMode("existing");
    else if (action === "open-project" && probe?.folder.kind === "project") onOpenProject(probe.folder.project_id);
  };

  const start = async () => {
    if (!ready || working || !storage || !repository?.bundle_id || !destination || !probe) return;
    const chosenProfiles: Record<ProviderKey, DraftProfileSelection> = {};
    for (const descriptor of enabledProviders) {
      chosenProfiles[descriptor.key] = { kind: "existing", profile_id: agents.chosen[descriptor.key] };
    }
    const choices: ConnectChoices = {
      storageId: storage.storage_id,
      bundleId: repository.bundle_id,
      repository,
      profiles: chosenProfiles,
    };

    if (existing) {
      setRun({ phase: "connect" });
      try {
        const detail = await connectExistingFolder(destination, probe.folder, ticked, choices);
        if (!isActive()) return;
        if (!detail) {
          setRun({ phase: "failed", failure: "changed", message: null });
          setProbeRound((round) => round + 1);
          return;
        }
        setRun({ phase: "idle" });
        onDone(detail, null, workflowEpoch);
      } catch (reason) {
        if (!isActive()) return;
        setRun({ phase: "failed", failure: "connect", message: errorMessage(reason) });
        setProbeRound((round) => round + 1);
      }
      return;
    }

    const operationId = newOperationId();
    operationRef.current = operationId;
    cancelledRef.current = false;
    setRun({ phase: "folder", progress: null });
    let folder;
    try {
      folder = await projectSyncApi.getCloudProjectFolder({
        destination,
        is_git_project: repository.repository_class === "git_project",
        github_url: repository.repository_class === "git_project" ? repository.github_url : null,
        repository_fingerprint: repository.repository_fingerprint,
        operation_id: operationId,
      }, (progress) => {
        if (isActive() && operationRef.current === operationId) setRun({ phase: "folder", progress });
      });
    } catch (reason) {
      operationRef.current = null;
      if (!isActive()) return;
      setRun(cancelledRef.current
        ? { phase: "idle" }
        : { phase: "failed", failure: "folder", message: errorMessage(reason) });
      setProbeRound((round) => round + 1);
      return;
    }
    operationRef.current = null;
    if (!isActive()) return;
    setRun({ phase: "connect" });
    try {
      const holds = folder.kind === "created" || (folder.kind === "initialized" && !repository.github_url) ? "empty" : "checkout";
      const detail = await connectFolder(folder.canonical_path, choices, mismatchAcknowledged(holds, false));
      if (!isActive()) return;
      const fallback = folder.kind === "initialized"
        ? { projectName: detail.project.local_alias || detail.project.display_name, failure: folder.failure }
        : null;
      setRun({ phase: "idle" });
      onDone(detail, fallback, workflowEpoch);
    } catch (reason) {
      if (!isActive()) return;
      setRun({ phase: "failed", failure: "connect", message: errorMessage(reason) });
      setProbeRound((round) => round + 1);
    }
  };

  const cancel = () => {
    if (run.phase === "folder" && operationRef.current) {
      cancelledRef.current = true;
      void projectSyncApi.cancelCloudProjectFolder(operationRef.current).catch(() => {});
      return;
    }
    onCancel();
  };

  const cloning = source ?? repository?.address ?? "";
  const status = run.phase === "folder"
    ? run.progress?.stage === "creating"
      ? repository?.repository_class === "git_project" ? t("cloudSource.initializing") : t("cloudSource.making")
      : run.progress?.stage === "cloning" && run.progress.percent !== null
        ? t("cloudSource.cloningPercent", { source: cloning, percent: run.progress.percent })
        : t("cloudSource.cloning", { source: cloning })
    : run.phase === "connect" && repository ? t("cloudSource.connectingTo", { address: repository.address }) : null;
  const primaryLabel = run.phase === "folder"
    ? run.progress?.stage === "creating" || !willClone ? t("cloudSource.creating") : t("cloudSource.cloningShort")
    : run.phase === "connect" ? t("cloudSource.connecting") : run.phase === "failed" ? t("common:tryAgain") : verb;
  const failureTitle = run.phase !== "failed" ? null
    : run.failure === "changed" ? t("cloudSource.folderChanged")
      : run.failure === "connect" ? t("cloudSource.notConnected") : t("cloudSource.notMade");

  /* What kind of folder the field names; Browse follows it. */
  const folderSwitch = (
    <div className="setup-pill-row" role="radiogroup" aria-label={t("sections.folder")}>
      {(["new", "existing"] as const).map((option) => {
        const active = mode === option;
        return (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={active}
            className={`setup-pill${active ? " active" : ""}`}
            disabled={working}
            onClick={() => setMode(option)}
          >
            <Icon name={option === "new" ? "plus" : "folder"} size={12} />
            <span className="setup-pill-label">{option === "new" ? t("cloudSource.newFolder") : t("cloudSource.existingFolder")}</span>
            <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
          </button>
        );
      })}
    </div>
  );

  return (
    <>
      <div className="sync-modal-body">
        {doors}

        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>{t("sections.storage")}</strong></div></div>
          {accounts.length === 0 ? (
            <div className="r2-step-list">
              <div className="storage-found-empty" role="status">
                <Icon name="storage-mallard" size={24} />
                <strong>{t("cloudSource.noAccount")}</strong>
                <span className="storage-found-empty-verbs">
                  <button type="button" className="btn btn-primary" onClick={onAddAccount} disabled={working}>
                    <Icon name="link" size={13} /> {t("cloudSource.setUpCloud")}
                  </button>
                </span>
              </div>
            </div>
          ) : (
            <>
              <div className="setup-pill-row" role="radiogroup" aria-label={t("sections.storage")}>
                {accounts.map((account) => {
                  const active = account.storage_id === storageId;
                  const name = account.name || t("shared.unnamed");
                  return (
                    <button
                      key={account.storage_id}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      aria-label={`${name} · ${storageProviderName(account)}`}
                      title={`${name} · ${storageProviderName(account)}`}
                      className={`setup-pill${active ? " active" : ""}`}
                      disabled={working || !account.available}
                      onClick={() => chooseStorage(account.storage_id)}
                    >
                      <Icon name="storage-mallard" size={12} />
                      <span className="setup-pill-label">{account.available ? name : t("shared.unavailable", { name })}</span>
                      <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
                    </button>
                  );
                })}
              </div>
              {!storage && (
                <small className="sync-setup-hint">
                  {initialAddress ? t("cloudSource.chooseAccountFor", { address: initialAddress }) : t("cloudSource.chooseAccount")}
                </small>
              )}
            </>
          )}
        </section>

        {storage && (
          <section className="sync-setup-section repo-connect cloud-project-repository">
            <div className="sync-card-heading"><div><strong>{t("sections.repository")}</strong></div></div>
            <RepositoryAddressPicker
              storage={storage}
              projectName=""
              localFingerprint={null}
              listing={listing}
              onRefresh={refresh}
              selection={selection}
              rows="match-option"
              offerNew={false}
              mode="clone"
              disabled={working}
              initialAddress={initialAddress}
              onSelect={chooseRepository}
            />
          </section>
        )}

        {storage && repository && (
          <section className="sync-setup-section">
            <div className="sync-card-heading"><div><strong>{t("sections.project")}</strong></div></div>
            <div className="sync-field-grid sync-setup-project-fields">
              <label>
                <span>{t("sections.name")}</span>
                <input value={repository.name} readOnly disabled />
              </label>
              <ProjectFolderField
                value={folderText}
                verdict={verdict}
                disabled={working}
                heading={folderSwitch}
                placeholder="~/code/my-app"
                onChange={setFolderText}
                onEnter={() => void start()}
                onBrowsed={(picked) => setFolderText(compact(browsedPath(mode, picked, repository.name)))}
                onRun={runFolderAction}
              />
            </div>
            {/* Outside the field grid, whose label and input rules would
                restyle the box's own label and checkbox. */}
            {tick && (
              <Acknowledgement
                checked={ticked}
                disabled={working}
                onChange={(checked) => setTickedFor(checked ? tickFor : null)}
              >
                {t("cloudSource.cantVerify", { name: repository.name })}
              </Acknowledgement>
            )}
          </section>
        )}

        {storage && repository && (
          <section className="sync-setup-section">
            <div className="sync-card-heading"><div><strong>{t("sections.agent")}</strong></div></div>
            <div className="sync-provider-cards" role="group" aria-label={t("agents.group")}>
              {providers.map((descriptor) => {
                const options = usableProfiles(profiles, descriptor.key);
                const on = !!agents.enabled[descriptor.key];
                const value = agents.chosen[descriptor.key] ?? "";
                return (
                  <div key={descriptor.key} className={`sync-provider-card${on ? " enabled" : ""}`}>
                    <label className="sync-provider-toggle setup-marked-toggle">
                      <input
                        type="checkbox"
                        checked={on}
                        disabled={working || options.length === 0 || (on && enabledProviders.length === 1)}
                        aria-label={t("agents.use", { provider: descriptor.display_name })}
                        onChange={() => setAgents((current) => ({
                          ...current,
                          enabled: { ...current.enabled, [descriptor.key]: !on },
                        }))}
                      />
                      <ProviderMark provider={descriptor.key} displayName={descriptor.display_name} size={12} />
                      <strong>{descriptor.display_name}</strong>
                      <span>{backendText(`agentProfile.${descriptor.key}`, descriptor.profile_label)}</span>
                    </label>
                    {on ? (
                      <div className="sync-agent-profile-list">
                        <div className={`sync-agent-profile-row${value ? "" : " has-error"}`}>
                          <span className="sync-select-shell">
                            <select
                              value={value}
                              disabled={working}
                              aria-label={t("agents.profile", { provider: descriptor.display_name })}
                              aria-invalid={!value || undefined}
                              onChange={(event) => {
                                const next = event.target.value;
                                setAgents((current) => ({ ...current, chosen: { ...current.chosen, [descriptor.key]: next } }));
                              }}
                            >
                              <option value="">{t("agents.chooseProfile")}</option>
                              {options.map((option) => (
                                <option key={option.profile_id} value={option.profile_id}>{option.display_name}</option>
                              ))}
                            </select>
                          </span>
                        </div>
                      </div>
                    ) : (
                      <small>{t("agents.notUsed")}</small>
                    )}
                  </div>
                );
              })}
            </div>
            {agentMessage && <FieldMessage>{agentMessage}</FieldMessage>}
          </section>
        )}

        {run.phase === "failed" && (
          <Notice severity="danger" timing="blocking" title={failureTitle ?? undefined} className="sync-setup-system-error">
            {run.message ?? t("cloudSource.checkAgain")}
          </Notice>
        )}
      </div>

      <footer className="sync-modal-footer sync-setup-footer">
        {status && <span className="sync-setup-save-state" aria-live="polite">{status}</span>}
        <div>
          <button type="button" className="btn btn-ghost" onClick={cancel} disabled={run.phase === "connect"}>
            {t("common:cancel")}
          </button>
          <button type="button" className="btn btn-primary" disabled={!ready || working} onClick={() => void start()}>
            {primaryLabel}
          </button>
        </div>
      </footer>
    </>
  );
}
