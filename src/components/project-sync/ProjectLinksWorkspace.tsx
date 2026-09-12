import {

  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type {
  BundleFetchProgress,
  BundleSnapshotSummary,
  CommitStorageDraftResult,
  LocalProjectSummary,
  ProjectBinding,
  ProjectStorageLink,
  ProviderDescriptor,
  ProviderProfileSummary,
  RemoteBundleSummary,
  SiblingBundleStatus,
  StorageProviderDescriptor,
  StorageSetupDraft,
  StorageSummary,
  StorageProviderKey,
} from "../../types";
import Icon, { iconNameFromHint } from "../Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { formatDateTime, toEpochMilliseconds } from "../../clock";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import ProjectActivityPage, { BranchPicker, type ActivityHeaderModel } from "./ProjectActivityPage";
import ProjectFilesActivityPage, { type FilesActivityHeaderModel } from "./ProjectFilesActivityPage";
import type {
  ActiveProjectSubview,
  OpenSessionDetailRequest,
} from "./session-detail/sessionDetailModel";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import ProviderMark from "../sync-review/ProviderMark";
import ProjectPicker from "./ProjectPicker";
import type { SessionHeaderModel } from "./session-detail/SessionDetailPage";
import type { PullRequiredCopy } from "./PushResourceWorkspace";
import SyncOutcomeNotice from "./SyncOutcomeNotice";
import SiblingRepoNotice from "./SiblingRepoNotice";
import { isIrrecoverableSyncError } from "./syncError";
import type { SyncOutcome, SyncOutcomeAction } from "./syncOutcome";
import {
  connectionDetail,
  connectionSuspends,
  draftSettingsFromForm,
  fieldsSet,
  findDestinationConflict,
  findStorageProvider,
  formFromDraft,
  secretOperationsFromForm,
  secretPresenceOf,
  storageDraftReady,
  storageSubtitle,
  storageSubtitleIsPath,
  StorageEditor,
  storageNameIsAuto,
  storageNameForSettings,
  type StorageEditorForm,
  type StorageSetupMode,
} from "./StorageSettings";
import { storageCatalogApi } from "../storage-catalog/api";
import { catalogPresentation, catalogStorageRepositoryViews } from "../storage-catalog/model";
import type { CatalogStorageRepositoryView } from "../storage-catalog/types";
import { projectSyncApi } from "./api";
import { StorageDraftSaveQueue } from "./storageDraftSaveQueue";
import { R2SetupLane, useR2Setup, type R2SetupMode, type R2SetupPort } from "./R2Setup";
import { DropboxSetupLane, useDropboxSetup } from "./DropboxSetup";
import {
  compactProjectPath,
  formatRelativeTime,
  projectLabel,
  providerLabel,
  type RenameResult,
} from "./model";

type LinkKey = { projectId: string; storageId: string };
export type StorageReviewKind = "pull" | "push";
type StorageEditorRequest =
  | { mode: "open"; storageId: string; settingsExpanded: boolean; requestId: number }
  | { mode: "resume-draft"; draft: StorageSetupDraft; requestId: number }
  | { mode: "close"; requestId: number };

/** Autosave cadence for the storage form, matching project setup. */
const STORAGE_DRAFT_AUTOSAVE_DELAY_MS = 700;
type InlineStorageReview = {
  kind: StorageReviewKind;
  projectId: string;
  storageId: string;
  content: ReactNode;
  onClose: () => void;
  onContinue?: () => void;
  continueDisabled?: boolean;
  /** Why onContinue is withheld right now — surfaced as the button's title. */
  continueTitle?: string | null;
};

/**
 * A gate the push or pull flow cannot get past never opens a review; it lands
 * here instead, as a banner inside the storage card with the way forward on
 * it. Cleared by ProjectSync when a review opens or the workspace resets.
 */
export type StorageNotice = {
  projectId: string;
  storageId: string;
} & (
  | { kind: "requires_pull"; copy: PullRequiredCopy }
  | { kind: "remote_missing"; baseRevision: number | null }
  | { kind: "remote_replaced"; baseRevision: number | null }
  | { kind: "action_failed"; action: "push" | "pull"; message: string }
  /** A finished Push or Pull; the flow has already closed behind it. */
  | { kind: "outcome"; outcome: SyncOutcome }
);

interface Props {
  projects: LocalProjectSummary[];
  activeProjectId: string | null;
  activeStorageId: string | null;
  activeProjectSubview: ActiveProjectSubview;
  bindings: ProjectBinding[];
  profiles: ProviderProfileSummary[];
  /** Registered agent provider descriptors. The agent-setup cluster derives
   *  its run buttons from these; it never holds a fixed provider list. */
  providers: ProviderDescriptor[];
  /** Compiled storage provider descriptors: every field, secret, and
   *  assistant the storage form shows comes from one of these. */
  storageProviders: StorageProviderDescriptor[];
  storages: StorageSummary[];
  /** Redacted machine-local drafts, refreshed when the CLI updates one. */
  storageSetupDrafts: StorageSetupDraft[];
  links: ProjectStorageLink[];
  loading: boolean;
  busy: boolean;
  fetchProgress?: BundleFetchProgress | null;
  onSelectStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onOpenSessionDetail: (projectId: string, request: OpenSessionDetailRequest) => void;
  onCloseSessionDetail: () => void;
  onLinkStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onUnlinkStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onPush: (projectId: string, storageId: string) => Promise<void> | void;
  onPull: (projectId: string, storageId: string) => Promise<void> | void;
  /** Sibling detection for the active pair: another repo in this storage
      carries the same repository. */
  siblingNotice?: {
    projectId: string;
    storageId: string;
    status: SiblingBundleStatus;
  } | null;
  onConnectSibling?: (
    projectId: string,
    storageId: string,
    matches: BundleSnapshotSummary[],
  ) => void;
  onConfigureBinding: (projectId: string) => Promise<void> | void;
  onRenameProject: (projectId: string, alias: string | null) => Promise<RenameResult> | RenameResult;
  onResetProjectMetadata: (projectId: string) => Promise<void> | void;
  onRefresh: () => void;
  onAddProject: () => void;
  onSelectProject: (projectId: string) => void;
  /**
   * Theme and app-wide commands. The path bar doubles as the window titlebar
   * here, but it carries one overflow menu, not two — a second `⋯` 90px from
   * the first, in the same glyph and at the same weight, made the only way to
   * find a command opening both. These join the end of that one menu.
   */
  appActions: PathBarMenuGroup[];
  onAddStorage: () => void;
  /** Mint a draft on a provider and open its form; the type switch uses it
   *  to restart a new draft, carrying a typed name along. */
  onCreateStorageDraft: (provider: StorageProviderKey, name?: string) => Promise<void>;
  /** Open a storage's page; `settingsExpanded` lands with the editor open. */
  onOpenStorage: (storageId: string, settingsExpanded: boolean) => void;
  /** The Storages crumb every storage page is rooted at. */
  onOpenStorageIndex: () => void;
  onClearStorage: (storageId: string) => void;
  onRemoveStorage: (storageId: string) => void;
  /** Rename a settled storage in place. Resolves false when the rename was
   *  refused; the shell has already shown why. */
  onRenameStorage: (storageId: string, name: string) => Promise<boolean>;
  /** Verify and commit the exact draft revision the user reviewed. Resolves
   *  null on refusal; the shell has already shown why. */
  onCommitStorageDraft: (
    draftId: string,
    expectedDraftRevision: number,
  ) => Promise<CommitStorageDraftResult | null>;
  /** Commit-free probe of the exact draft revision. Resolves true on a
   *  verified connection; failures surface through the shared error zone. */
  onTestStorageDraft: (draftId: string, expectedRevision: number) => Promise<boolean>;
  /** The persisted storage-draft list changed (created, updated, discarded). */
  onStorageDraftsChanged?: () => void;
  storageDraftSaveQueue: StorageDraftSaveQueue;
  storageEditorRequest?: StorageEditorRequest | null;
  onStorageEditorRequestHandled?: () => void;
  onStorageNavigationGuardChange?: (guard: (() => Promise<boolean>) | null) => void;
  onStorageEditorChange?: (storageId: string | null) => void;
  onStorageIssueChange?: (storageId: string, issue: string | null) => void;
  newProjectSetup?: ReactNode;
  historyRefreshEpoch?: number;
  /** Bumped when a storage's contents changed underneath the open page —
   *  clearing its data, for one — so the repository list reloads instead of
   *  showing objects that no longer exist. */
  storageRefreshEpoch?: number;
  inlineStorageReview?: InlineStorageReview | null;
  storageNotice?: StorageNotice | null;
  onStorageNoticeAction?: (outcome: SyncOutcome, action: SyncOutcomeAction) => void;
  onDismissStorageNotice?: () => void;
}

const SessionDetailPage = lazy(() => import("./session-detail/SessionDetailPage"));

/** The open storage form: the backend draft it edits — the last revision the
 *  backend returned, with secret presence — and the local edits on top. */
export type StorageDraftSession = {
  draft: StorageSetupDraft;
  form: StorageEditorForm;
};

type StorageDraftSaveSnapshot = {
  session: number;
  editVersion: number;
  draftId: string;
  name: string;
  settings: StorageSetupDraft["settings"];
  typedSecrets: Record<string, string>;
};

/** The exact saved draft passed to setup. Contains no credentials. */
export type StorageSetupMarker = {
  draftId: string;
  revision: number;
  storageId: string;
  requestedName: string;
};

export type StorageSetupReconciliation =
  | { kind: "adopt"; draft: StorageSetupDraft }
  | { kind: "preserve"; reason: "no_update" | "invalid" };

export function createStorageSetupMarker(
  saved: Pick<StorageSetupDraft, "draft_id" | "revision" | "storage_id">,
  requestedName: string,
): StorageSetupMarker {
  return {
    draftId: saved.draft_id,
    revision: saved.revision,
    storageId: saved.storage_id,
    requestedName: requestedName.trim(),
  };
}

/** Decide whether the CLI advanced the exact persisted draft the open form
 *  handed it. Name matching is deliberately insufficient: another draft can
 *  have the same label, and an older refresh must not look like CLI success.
 *  An edit in the form kills the marker before this is ever consulted. */
export function reconcileStorageSetup(
  marker: StorageSetupMarker,
  descriptor: Pick<StorageProviderDescriptor, "key" | "fields" | "secrets" | "managed_secret_id">,
  setupDrafts: readonly StorageSetupDraft[],
): StorageSetupReconciliation {
  const candidate = setupDrafts.find(({ draft_id }) => draft_id === marker.draftId);
  if (!candidate || candidate.revision <= marker.revision) {
    return { kind: "preserve", reason: "no_update" };
  }
  if (
    candidate.provider !== descriptor.key
    || candidate.storage_id !== marker.storageId
    || candidate.name.trim() !== marker.requestedName
    || !storageDraftReady(descriptor, formFromDraft(descriptor, candidate), secretPresenceOf(candidate))
  ) {
    return { kind: "preserve", reason: "invalid" };
  }
  return { kind: "adopt", draft: candidate };
}

/** After the R2 helper reports the draft ready, load it at once — a fast CLI
 *  must not depend on a later watcher event — and adopt it only if it is the
 *  same draft, newer, and complete. */
export async function loadPreparedR2Draft(
  saved: StorageSetupDraft,
  descriptor: StorageProviderDescriptor,
  isCurrent: () => boolean,
  api: Pick<typeof projectSyncApi, "getStorageSetupDraft"> = projectSyncApi,
): Promise<StorageSetupDraft | null> {
  if (!isCurrent()) return null;
  const prepared = await api.getStorageSetupDraft(saved.draft_id);
  if (!isCurrent()) return null;
  const result = reconcileStorageSetup(createStorageSetupMarker(saved, saved.name), descriptor, prepared ? [prepared] : []);
  if (result.kind !== "adopt") throw { code: "draft_changed" };
  return result.draft;
}

/** What the guide row's agent slot is saying: an agent CLI was opened in a
 *  terminal, the copy landed and the page is listening, the CLI's draft was
 *  adopted for review, or the hand-off failed. */
/** The fold: everything the storage listing knows about a repository beyond
 * its chips. Absent facts don't render — no dash placeholders. */
export function StorageRepositoryDetails({ bundle, id }: { bundle: RemoteBundleSummary; id?: string }) {
  return (
    <dl className="sync-storage-repository-details" id={id}>
      <dt>Repository ID</dt>
      <dd><code>{bundle.bundle_id}</code></dd>
      {typeof bundle.updated_at === "number" && (
        <>
          <dt>Updated</dt>
          <dd>{formatDateTime(toEpochMilliseconds(bundle.updated_at))}</dd>
        </>
      )}
      {typeof bundle.resource_count === "number" && (
        <>
          <dt>Resources</dt>
          <dd>{bundle.resource_count}</dd>
        </>
      )}
      {!!bundle.providers?.length && (
        <>
          <dt>Agents</dt>
          <dd>{bundle.providers.map((provider) => providerLabel(provider)).join(" · ")}</dd>
        </>
      )}
      {bundle.repository_fingerprint && (
        <>
          <dt>Fingerprint</dt>
          <dd><code>{bundle.repository_fingerprint}</code></dd>
        </>
      )}
    </dl>
  );
}

export function StorageRepositoryRow({ bundle }: { bundle: RemoteBundleSummary }) {
  const name = bundle.display_name || "Unnamed repository";
  const [open, setOpen] = useState(false);
  const detailsId = `repo-details-${bundle.bundle_id}`;
  return (
    <div className="sync-storage-repository-item">
      <div className="sync-storage-repository-row">
        {/* A fingerprint only exists when the bundle came from a git remote,
            so it is also the signal for the git flavour of the folder icon. */}
        <span className="sync-storage-repository-icon">
          <Icon name={bundle.repository_fingerprint ? "git-folder" : "folder"} size={14} />
        </span>
        <strong className="sync-storage-repository-name">{name}</strong>
        <span className="sync-storage-repository-chips">
          <span className="sync-storage-repo-chip"><span>Revision</span><b>{bundle.revision ?? "—"}</b></span>
          <span className="sync-storage-repo-chip"><span>Updated</span><b>{formatRelativeTime(bundle.updated_at)}</b></span>
        </span>
        <button
          type="button"
          className="sync-history-commit-toggle"
          aria-expanded={open}
          aria-controls={detailsId}
          aria-label={`${open ? "Hide" : "Show"} details for ${name}`}
          title={`${open ? "Hide" : "Show"} details for ${name}`}
          onClick={() => setOpen((value) => !value)}
        >
          <Icon name={open ? "chevron-up" : "chevron-down"} size={12} />
        </button>
      </div>
      {open && <StorageRepositoryDetails bundle={bundle} id={detailsId} />}
    </div>
  );
}

/** A Catalog kind repository in the project row's exact grammar: the kind
 * icon as the identity tile, the scope-label name one level up ("Skills ·
 * Codex"), the same chips and fold. When the derived id answers but cannot
 * be read, the row states the refusal where the chips would be — on a
 * listing, a silent hole reads as "nothing stored". */
export function CatalogStorageRepositoryRow({ repo }: { repo: CatalogStorageRepositoryView }) {
  const [open, setOpen] = useState(false);
  const glyph = catalogPresentation(repo.capability);
  /* The capability's own label when this build registers the coordinate;
     the backend's raw labels for one it does not. */
  const name = `${repo.capability?.label ?? repo.kind_label} · ${repo.provider_label}`;
  const detailsId = `catalog-repo-details-${repo.repository_id}`;
  const unreadable = repo.error != null;
  const shownNames = repo.resource_names.slice(0, 12);
  const moreNames = repo.resource_names.length - shownNames.length;
  return (
    <div className="sync-storage-repository-item">
      <div className="sync-storage-repository-row">
        <span className={`sync-storage-repository-icon${unreadable ? " danger" : ""}`}>
          <Icon name={unreadable ? "alert-triangle" : glyph.icon} size={14} />
        </span>
        <strong className="sync-storage-repository-name">{name}</strong>
        {unreadable ? (
          <span className="sync-storage-repo-unreadable">Can’t be read</span>
        ) : (
          <span className="sync-storage-repository-chips">
            <span className="sync-storage-repo-chip"><span>Revision</span><b>{repo.revision ?? "—"}</b></span>
            <span className="sync-storage-repo-chip"><span>Updated</span><b>{formatRelativeTime(repo.updated_at ?? undefined)}</b></span>
          </span>
        )}
        <button
          type="button"
          className="sync-history-commit-toggle"
          aria-expanded={open}
          aria-controls={detailsId}
          aria-label={`${open ? "Hide" : "Show"} details for ${name}`}
          title={`${open ? "Hide" : "Show"} details for ${name}`}
          onClick={() => setOpen((value) => !value)}
        >
          <Icon name={open ? "chevron-up" : "chevron-down"} size={12} />
        </button>
      </div>
      {open && (
        <dl className="sync-storage-repository-details" id={detailsId}>
          <dt>Repository ID</dt>
          <dd><code>{repo.repository_id}</code></dd>
          {typeof repo.updated_at === "number" && (
            <>
              <dt>Updated</dt>
              <dd>{formatDateTime(toEpochMilliseconds(repo.updated_at))}</dd>
            </>
          )}
          {repo.resource_names.length > 0 && (
            <>
              <dt>Resources</dt>
              <dd>{repo.resource_names.length}</dd>
              <dt>Contents</dt>
              <dd>{shownNames.join(" · ")}{moreNames > 0 ? ` · +${moreNames} more` : ""}</dd>
            </>
          )}
          {repo.error && (
            <>
              <dt>Problem</dt>
              <dd className="sync-storage-repo-problem">{repo.error}</dd>
            </>
          )}
        </dl>
      )}
    </div>
  );
}

export function projectPushActionLabel({
  preparing,
  publishing,
}: {
  preparing: boolean;
  publishing: boolean;
}): string {
  // The first click opens review; the button on that review publishes.
  if (publishing) return "Pushing…";
  if (preparing) return "Preparing…";
  return "Push";
}

export function projectPullActionLabel({
  preparing,
  applying,
}: {
  preparing: boolean;
  applying: boolean;
}): string {
  // Same shape as Push: the card button on an open review IS the pull — the
  // first click opens the review, the second applies it. Progress lives on
  // the button, not in a review banner.
  if (applying) return "Pulling…";
  if (preparing) return "Preparing…";
  return "Pull";
}

export function storageActionLockedForReview(
  reviewKind: StorageReviewKind | null,
  action: StorageReviewKind | "storage",
): boolean {
  return reviewKind !== null && (action === "storage" || action !== reviewKind);
}

export default function ProjectLinksWorkspace({
  projects,
  activeProjectId,
  activeStorageId,
  activeProjectSubview,
  bindings,
  profiles,
  providers,
  storageProviders,
  storages,
  storageSetupDrafts,
  links,
  loading,
  busy,
  fetchProgress,
  onSelectStorage,
  onOpenSessionDetail,
  onCloseSessionDetail,
  onLinkStorage,
  onUnlinkStorage,
  onPush,
  onPull,
  siblingNotice,
  onConnectSibling,
  onConfigureBinding,
  onRenameProject,
  onResetProjectMetadata,
  onRefresh,
  onAddProject,
  onSelectProject,
  appActions,
  onAddStorage,
  onCreateStorageDraft,
  onOpenStorage,
  onOpenStorageIndex,
  onStorageDraftsChanged,
  onClearStorage,
  onRemoveStorage,
  onRenameStorage,
  onCommitStorageDraft,
  onTestStorageDraft,
  storageEditorRequest,
  onStorageEditorRequestHandled,
  storageDraftSaveQueue,
  onStorageNavigationGuardChange,
  onStorageEditorChange,
  onStorageIssueChange,
  newProjectSetup,
  historyRefreshEpoch = 0,
  storageRefreshEpoch = 0,
  inlineStorageReview,
  storageNotice,
  onStorageNoticeAction,
  onDismissStorageNotice,
}: Props) {
  const [linkingProjectId, setLinkingProjectId] = useState<string | null>(null);
  const [runningAction, setRunningAction] = useState<string | null>(null);
  /* The storage page: the settled storage it shows, or — for a draft that
     never became one — the storage id the draft was minted with. */
  const [editingStorage, setEditingStorage] = useState<LinkKey | null>(null);
  /* The open form. Every draft is a backend object: the session holds the
     last revision the backend returned plus the edits typed on top, and the
     ref mirrors it synchronously so queued saves read the latest form. */
  const [storageSession, setStorageSession] = useState<StorageDraftSession | null>(null);
  const storageSessionRef = useRef<StorageDraftSession | null>(null);
  const [storageDetailsExpanded, setStorageDetailsExpanded] = useState(false);
  /* Opening a settled storage's settings asks the backend for an edit draft
     first; the panel shows its loading line until that draft arrives. */
  const [storageSessionLoading, setStorageSessionLoading] = useState(false);
  const [storageTestVerified, setStorageTestVerified] = useState(false);
  const [storageDraftActionError, setStorageDraftActionError] = useState<string | null>(null);
  /* The New-storage question for a provider that can look in its account:
     which door was taken, whether the fields were unfolded for a pasted
     connection, what the look found, and which row is chosen. All of it
     belongs to the open draft and goes with it; the request token retires a
     look the user cancelled or that outlived its draft. */
  const [storageSetupMode, setStorageSetupMode] = useState<StorageSetupMode | null>(null);
  const [manualEntry, setManualEntry] = useState(false);
  /* The draft's server revision is bookkeeping read only at the head of the
     save queue; the session counter invalidates async draft work when the
     form's identity changes — the same job bundleRequestRef does for
     listings. */
  const draftRevisionRef = useRef(0);
  const draftDirtyRef = useRef(false);
  const saveBeforeNavigationRef = useRef<() => Promise<boolean>>(async () => true);
  const draftEditVersionRef = useRef(0);
  const draftSessionRef = useRef(0);
  const committingRef = useRef(false);
  const draftListedRef = useRef(false);
  const editSessionRequestRef = useRef(0);
  const editingStorageRef = useRef<LinkKey | null>(null);
  editingStorageRef.current = editingStorage;
  const commitStorageSession = (next: StorageDraftSession | null) => {
    storageSessionRef.current = next;
    setStorageSession(next);
  };
  const updateStorageSession = (mutate: (current: StorageDraftSession) => StorageDraftSession) => {
    const current = storageSessionRef.current;
    if (current) commitStorageSession(mutate(current));
  };
  const resetStorageSession = () => {
    draftSessionRef.current += 1;
    draftEditVersionRef.current += 1;
    editSessionRequestRef.current += 1;
    draftDirtyRef.current = false;
    draftRevisionRef.current = 0;
    committingRef.current = false;
    draftListedRef.current = false;
    commitStorageSession(null);
    setStorageTestVerified(false);
    setStorageDraftActionError(null);
    setStorageSessionLoading(false);
    setStorageSetupMode(null);
    setManualEntry(false);
    r2.dismiss();
    dropbox.dismiss();
  };
  /** Seat a backend draft in the form. The descriptor shapes the form; a
   *  draft this build has no descriptor for opens empty and uneditable. */
  const openStorageSession = (draft: StorageSetupDraft, descriptor: StorageProviderDescriptor | null) => {
    resetStorageSession();
    draftRevisionRef.current = draft.revision;
    commitStorageSession({
      draft,
      form: descriptor
        ? formFromDraft(descriptor, draft)
        : { name: draft.name, settings: {}, typedSecrets: {} },
    });
  };
  const [renamingStorageId, setRenamingStorageId] = useState<string | null>(null);
  const [storageNameDraft, setStorageNameDraft] = useState("");
  const [remoteBundles, setRemoteBundles] = useState<BundleSnapshotSummary[] | null>(null);
  const [bundleLoading, setBundleLoading] = useState(false);
  const [bundleError, setBundleError] = useState<string | null>(null);
  /* The Catalog's kind repositories in the same storage, listed beside the
     project ones. Their failure has its own surface — a callout under the
     list — so one broken half never hides the other. */
  const [catalogRepos, setCatalogRepos] = useState<CatalogStorageRepositoryView[] | null>(null);
  const [catalogRepoError, setCatalogRepoError] = useState<string | null>(null);
  /* Revealing is `reveal_item_in_dir`, not `open_path`: the capability grants
     open_path only for the log folder, so a project root or a storage folder
     is refused there. The rejection is surfaced, never dropped — a folder that
     was moved or deleted since the config was written is the ordinary case. */
  const [renameError, setRenameError] = useState<string | null>(null);
  const [revealError, setRevealError] = useState<string | null>(null);
  const revealPath = (path: string) => {
    setRevealError(null);
    void revealItemInDir(path).catch((reason) => setRevealError(errorMessage(reason)));
  };
  const openProjectFile = (localProjectId: string, relativePath: string) => {
    setRevealError(null);
    void projectSyncApi.openProjectMarkdownPath(localProjectId, relativePath)
      .catch((reason) => setRevealError(errorMessage(reason)));
  };
  const [renamingProjectId, setRenamingProjectId] = useState<string | null>(null);
  const [projectAliasDraft, setProjectAliasDraft] = useState("");
  // The activity list speaks through the path bar: it owns the branches and the
  // storage comparison, the bar owns the row they belong in. The callback has
  // to be stable — the list reports through an effect, and a new identity every
  // render would put the two of them in a loop.
  const [activityHeader, setActivityHeader] = useState<ActivityHeaderModel | null>(null);
  const handleActivityHeaderChange = useCallback(
    (model: ActivityHeaderModel | null) => setActivityHeader(model),
    [],
  );
  /* Which face of the activity page a non-git project shows. The path bar's
     leaf crumb is the switch; a git project has no Files face, so the state
     resets whenever the project changes. */
  const [activityView, setActivityView] = useState<"sessions" | "files">("sessions");
  const [filesActivityHeader, setFilesActivityHeader] = useState<FilesActivityHeaderModel | null>(null);
  const handleFilesActivityHeaderChange = useCallback(
    (model: FilesActivityHeaderModel | null) => setFilesActivityHeader(model),
    [],
  );
  const bundleRequestRef = useRef(0);
  const storageSettingsRef = useRef<HTMLDivElement>(null);
  const workspaceScrollRef = useRef<HTMLElement>(null);
  const overviewScrollPositionRef = useRef(0);
  const sessionDetailOriginRef = useRef<HTMLButtonElement | null>(null);
  const openSessionDetail = (
    projectId: string,
    request: OpenSessionDetailRequest,
    trigger: HTMLButtonElement,
  ) => {
    overviewScrollPositionRef.current = workspaceScrollRef.current?.scrollTop ?? 0;
    sessionDetailOriginRef.current = trigger;
    onOpenSessionDetail(projectId, request);
  };

  const closeSessionDetail = () => {
    const scrollTop = overviewScrollPositionRef.current;
    const origin = sessionDetailOriginRef.current;
    onCloseSessionDetail();
    window.requestAnimationFrame(() => {
      workspaceScrollRef.current?.scrollTo({ top: scrollTop, left: 0 });
      origin?.focus({ preventScroll: true });
    });
  };

  useEffect(() => {
    if (!editingStorage) return;
    const frame = window.requestAnimationFrame(() => {
      storageSettingsRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [editingStorage]);

  /* Symmetric: unmounting is leaving the storage page, so the report has to be
     withdrawn too. Without the cleanup the sidebar kept the storage row lit
     after the user navigated to Settings, showing three active items at once
     and never letting "Projects" render as inactive. Mirrors the pattern
     ProjectActivityPage uses for onHeaderModelChange. */
  useEffect(() => {
    onStorageEditorChange?.(editingStorage?.storageId ?? null);
    return () => onStorageEditorChange?.(null);
  }, [editingStorage?.storageId, onStorageEditorChange]);

  const savedEditedStorage = editingStorage
    ? storages.find((storage) => storage.storage_id === editingStorage.storageId) ?? null
    : null;

  useEffect(() => {
    setRenamingStorageId(null);
    setStorageNameDraft(savedEditedStorage?.name ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editingStorage?.storageId]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      workspaceScrollRef.current?.scrollTo({ top: 0, left: 0 });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [activeProjectId, activeStorageId]);

  useEffect(() => {
    if (activeProjectSubview.kind !== "session_detail") return;
    const frame = window.requestAnimationFrame(() => {
      workspaceScrollRef.current?.scrollTo({ top: 0, left: 0 });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [activeProjectSubview.kind, activeProjectSubview.kind === "session_detail"
    ? activeProjectSubview.resourceId
    : null]);

  useEffect(() => {
    if (!inlineStorageReview) return;
    setLinkingProjectId(null);
  }, [inlineStorageReview?.kind, inlineStorageReview?.projectId, inlineStorageReview?.storageId]);

  const linkedByProject = useMemo(() => new Map(projects.map((project) => [
    project.local_project_id,
    links.filter((link) => link.local_project_id === project.local_project_id),
  ])), [links, projects]);
  const bindingByProject = useMemo(() => new Map(bindings
    .map((candidate) => [candidate.local_project_id, candidate])), [bindings]);
  const activeBindingByProject = useMemo(() => new Map(bindings
    .filter((candidate) => candidate.state === "active")
    .map((candidate) => [candidate.local_project_id, candidate])), [bindings]);

  /* A draft whose storage id is not in the config is a storage being
     created; its page is the New-storage form. An edit draft shares its
     storage's id and rides that storage's page instead. */
  const creatingStorage = !!editingStorage && savedEditedStorage === null;
  /* The R2 hand-off. The hook runs the packaged helper one step at a time
     on the draft this page holds; the port below lends it the draft — saved
     before every run, pinned by session and edit version — and the seat for
     the draft the helper prepares. */
  const r2Mode: R2SetupMode = !creatingStorage ? "reconnect" : storageSetupMode === "existing" ? "existing" : "create";
  const setupPort: R2SetupPort = {
      blocked: () => busy || !!runningAction,
      beginRun: () => {
        const session = draftSessionRef.current;
        const editVersion = draftEditVersionRef.current;
        return {
          isCurrent: () => draftSessionRef.current === session && draftEditVersionRef.current === editVersion,
          persist: () => persistStorageSetupDraft(session, editVersion),
        };
      },
      onRunStarted: () => {
        setStorageTestVerified(false);
        setStorageDraftActionError(null);
      },
      adoptPrepared: async (saved, isCurrent) => {
        const descriptor = descriptorFor(saved);
        if (!descriptor) throw { code: "draft_changed" };
        const prepared = await loadPreparedR2Draft(saved, descriptor, isCurrent);
        if (!prepared) return false;
        const suggestedName = !saved.edits_existing
          ? storageNameForSettings(descriptor, formFromDraft(descriptor, saved), formFromDraft(descriptor, prepared).settings) : null;
        openStorageSession(prepared, descriptor);
        if (suggestedName && suggestedName !== prepared.name) {
          // Use the normal draft autosave; close, Test, and Finish also flush it.
          draftEditVersionRef.current += 1;
          draftDirtyRef.current = true;
          updateStorageSession((current) => ({ ...current, form: { ...current.form, name: suggestedName } }));
        }
        onStorageDraftsChanged?.();
        return true;
      },
    };
  const r2 = useR2Setup({
    enabled: !!storageSession
      && findStorageProvider(storageProviders, storageSession.draft.provider, storageSession.draft.config_version)?.local_setup === "wrangler",
    draftId: storageSession?.draft.draft_id ?? null,
    mode: r2Mode,
    port: setupPort,
  });
  const dropbox = useDropboxSetup({
    enabled: !!storageSession && findStorageProvider(storageProviders, storageSession.draft.provider, storageSession.draft.config_version)?.local_setup === "dropbox",
    draftId: storageSession?.draft.draft_id ?? null,
    mode: r2Mode,
    port: setupPort,
  });
  const orderedRemoteBundles = [...(remoteBundles ?? [])].sort((left, right) => {
    return (right.updated_at ?? 0) - (left.updated_at ?? 0);
  });
  /* Catalog sets follow the projects, newest first among themselves; a
     refusal row carries no timestamp and sinks to the end. */
  const orderedCatalogRepos = [...(catalogRepos ?? [])].sort((left, right) => {
    return (right.updated_at ?? 0) - (left.updated_at ?? 0);
  });
  const run = async (key: string, action: () => Promise<void> | void) => {
    if ((r2.isRunning() || dropbox.isRunning())) return;
    setRunningAction(key);
    try {
      await action();
    } finally {
      setRunningAction(null);
    }
  };

  const loadStorageBundles = async (storage: StorageSummary) => {
    const storageId = storage.storage_id;
    const requestId = ++bundleRequestRef.current;
    setRemoteBundles(null);
    setBundleError(null);
    setCatalogRepos(null);
    setCatalogRepoError(null);
    if (!storage.available) {
      // Nothing to ask: this build cannot open the store. The page says so
      // in the repositories card; it is not a storage issue to flag.
      setRemoteBundles([]);
      setCatalogRepos([]);
      setBundleLoading(false);
      onStorageIssueChange?.(storageId, null);
      return;
    }
    setBundleLoading(true);
    // Both reads start together under the one loading gate.
    const bundlesRead = projectSyncApi.listRemoteBundleSnapshots(storageId);
    const catalogRead = Promise.all([
      storageCatalogApi.listStorageRepositories(storageId),
      storageCatalogApi.listCapabilities(),
    ]);
    try {
      const bundles = await bundlesRead;
      if (bundleRequestRef.current === requestId) {
        setRemoteBundles(bundles);
        onStorageIssueChange?.(storageId, null);
      }
    } catch (reason) {
      if (bundleRequestRef.current === requestId) {
        const issue = storageAccessErrorMessage(storage, reason);
        setBundleError(issue);
        onStorageIssueChange?.(storageId, issue);
      }
    }
    try {
      const [repositories, capabilities] = await catalogRead;
      if (bundleRequestRef.current === requestId) {
        setCatalogRepos(catalogStorageRepositoryViews(repositories, capabilities));
      }
    } catch (reason) {
      if (bundleRequestRef.current === requestId) {
        setCatalogRepoError(storageAccessErrorMessage(storage, reason));
      }
    } finally {
      if (bundleRequestRef.current === requestId) setBundleLoading(false);
    }
  };

  useEffect(() => {
    if (!storageRefreshEpoch || !savedEditedStorage) return;
    void loadStorageBundles(savedEditedStorage);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storageRefreshEpoch]);

  const clearStorageListing = () => {
    bundleRequestRef.current += 1;
    setRemoteBundles(null);
    setBundleError(null);
    setCatalogRepos(null);
    setCatalogRepoError(null);
    setBundleLoading(false);
  };

  const descriptorFor = (draft: Pick<StorageSetupDraft, "provider" | "config_version">) => (
    findStorageProvider(storageProviders, draft.provider, draft.config_version)
  );

  const captureStorageDraftSave = (
    session: number,
    editVersion: number,
  ): StorageDraftSaveSnapshot | null => {
    const current = storageSessionRef.current;
    if (draftSessionRef.current !== session || !current) return null;
    const descriptor = descriptorFor(current.draft);
    return {
      session,
      editVersion,
      draftId: current.draft.draft_id,
      name: current.form.name,
      settings: descriptor
        ? draftSettingsFromForm(descriptor, current.form, current.draft.settings)
        : current.draft.settings,
      typedSecrets: { ...current.form.typedSecrets },
    };
  };

  const saveStorageDraftSnapshot = async (
    snapshot: StorageDraftSaveSnapshot,
  ): Promise<StorageSetupDraft | null> => {
    const current = storageSessionRef.current;
    if (
      draftSessionRef.current !== snapshot.session
      || current?.draft.draft_id !== snapshot.draftId
    ) return null;
    try {
      const descriptor = descriptorFor(current.draft);
      const secrets = descriptor
        ? secretOperationsFromForm(
          {
            typedSecrets: snapshot.typedSecrets,
            persistedSecrets: current.form.persistedSecrets,
          },
          secretPresenceOf(current.draft),
        )
        : {};
      const saved = await projectSyncApi.updateStorageSetupDraft({
        draft_id: snapshot.draftId,
        revision: draftRevisionRef.current,
        name: snapshot.name,
        settings: snapshot.settings,
        secrets,
      });
      if (
        draftSessionRef.current !== snapshot.session
        || storageSessionRef.current?.draft.draft_id !== snapshot.draftId
      ) return null;
      draftRevisionRef.current = saved.revision;
      updateStorageSession((latest) => {
        const persistedSecrets = { ...latest.form.persistedSecrets };
        for (const [secretId, value] of Object.entries(snapshot.typedSecrets)) {
          if (value && latest.form.typedSecrets[secretId] === value) {
            persistedSecrets[secretId] = value;
          }
        }
        return { draft: saved, form: { ...latest.form, persistedSecrets } };
      });
      if (draftEditVersionRef.current === snapshot.editVersion) {
        draftDirtyRef.current = false;
        setStorageDraftActionError(null);
      }
      onStorageDraftsChanged?.();
      return saved;
    } catch (reason) {
      if (draftSessionRef.current === snapshot.session) draftDirtyRef.current = true;
      throw reason;
    }
  };

  const persistStorageSetupDraft = (
    session: number,
    editVersion = draftEditVersionRef.current,
  ): Promise<StorageSetupDraft | null> => {
    const snapshot = captureStorageDraftSave(session, editVersion);
    if (!snapshot) return Promise.resolve(null);
    return storageDraftSaveQueue.enqueue(() => saveStorageDraftSnapshot(snapshot));
  };

  /** Test and Finish must act on the exact snapshot the user reviewed. Put
   *  that snapshot behind any autosave already in flight, then reject the
   *  action if the form changed while the guarded save was pending. */
  const persistStorageSetupDraftForAction = async (
    session: number,
    editVersion: number,
    action: "testing the connection" | "finishing setup" | "preparing the connection",
  ): Promise<StorageSetupDraft | null> => {
    try {
      const saved = await persistStorageSetupDraft(session, editVersion);
      if (!saved || draftSessionRef.current !== session) return null;
      if (draftEditVersionRef.current !== editVersion) {
        setStorageDraftActionError(
          `The storage form changed while Mallard was preparing it for ${action}. Review the latest values and try again.`,
        );
        return null;
      }
      setStorageDraftActionError(null);
      return saved;
    } catch (reason) {
      if (draftSessionRef.current === session && draftEditVersionRef.current === editVersion) {
        // Preserve the autosave safety net after a failed action save. A
        // later edit or another explicit action will retry this snapshot.
        draftDirtyRef.current = true;
        setStorageDraftActionError(
          `Mallard could not save the current storage draft before ${action}: ${errorMessage(reason)}`,
        );
      }
      return null;
    }
  };

  const saveStorageDraftBeforeNavigation = async (): Promise<boolean> => {
    if ((r2.isRunning() || dropbox.isRunning())) return false;
    const session = draftSessionRef.current;
    const current = storageSessionRef.current;
    if (!current) return true;
    const editVersion = draftEditVersionRef.current;
    const snapshot = captureStorageDraftSave(session, editVersion);
    if (!snapshot) return false;
    try {
      await storageDraftSaveQueue.beforeNavigation(
        async () => {
          const saved = await saveStorageDraftSnapshot(snapshot);
          if (
            !saved
            || draftSessionRef.current !== session
            || draftEditVersionRef.current !== editVersion
          ) throw new Error("storage form changed while saving");
          return saved;
        },
        resetStorageSession,
      );
      return true;
    } catch (reason) {
      if (draftSessionRef.current === session) {
        draftDirtyRef.current = true;
        setStorageDraftActionError(draftEditVersionRef.current !== editVersion
          ? "The storage form changed while Mallard was saving it. Review the latest values and try again."
          : `Mallard could not save the current storage draft before leaving: ${errorMessage(reason)}`);
      }
      return false;
    }
  };
  saveBeforeNavigationRef.current = saveStorageDraftBeforeNavigation;

  useEffect(() => {
    const guard = () => saveBeforeNavigationRef.current();
    onStorageNavigationGuardChange?.(guard);
    return () => onStorageNavigationGuardChange?.(null);
  }, [onStorageNavigationGuardChange]);

  /** Open the edit draft for a settled storage: the one already listed for
   *  it, else a fresh copy of its configuration minted by the backend. The
   *  draft carries the saved secrets server-side and reports them present. */
  const ensureStorageEditSession = async (storage: StorageSummary) => {
    if (!storage.available || storageSessionRef.current) return;
    const requestId = ++editSessionRequestRef.current;
    setStorageSessionLoading(true);
    setStorageDraftActionError(null);
    try {
      const listed = storageSetupDrafts.find((draft) => (
        draft.edits_existing && draft.storage_id === storage.storage_id
      ));
      const resumed = listed ? await projectSyncApi.getStorageSetupDraft(listed.draft_id) : null;
      const draft = resumed ?? await storageDraftSaveQueue.enqueue(
        () => projectSyncApi.createStorageEditDraft(storage.storage_id),
      );
      if (
        requestId !== editSessionRequestRef.current
        || editingStorageRef.current?.storageId !== storage.storage_id
      ) return;
      openStorageSession(draft, descriptorFor(draft));
      if (!resumed) onStorageDraftsChanged?.();
    } catch (reason) {
      if (requestId !== editSessionRequestRef.current) return;
      setStorageSessionLoading(false);
      setStorageDraftActionError(`Mallard could not open the storage settings: ${errorMessage(reason)}`);
    }
  };

  useEffect(() => {
    if (!storageEditorRequest) return;
    if (storageEditorRequest.mode === "close") {
      resetStorageSession();
      clearStorageListing();
      setEditingStorage(null);
      setStorageDetailsExpanded(false);
      onStorageEditorRequestHandled?.();
      return;
    }
    if (storageEditorRequest.mode === "resume-draft") {
      const draft = storageEditorRequest.draft;
      clearStorageListing();
      // An edit draft rides its storage's page with the settings open; a
      // draft for a storage that does not exist yet is the New-storage page.
      const settled = draft.edits_existing
        ? storages.find((candidate) => candidate.storage_id === draft.storage_id) ?? null
        : null;
      setEditingStorage({ projectId: "", storageId: draft.storage_id });
      openStorageSession(draft, descriptorFor(draft));
      setStorageDetailsExpanded(true);
      if (settled) void loadStorageBundles(settled);
      onStorageEditorRequestHandled?.();
      return;
    }
    const storage = storages.find((candidate) => candidate.storage_id === storageEditorRequest.storageId);
    if (!storage) {
      onStorageEditorRequestHandled?.();
      return;
    }
    if (editingStorage?.storageId !== storage.storage_id) {
      resetStorageSession();
      setEditingStorage({ projectId: "", storageId: storage.storage_id });
    }
    // Every open is a navigation onto the page, so the listing reloads —
    // a commit that just replaced the connection settings lands here too.
    void loadStorageBundles(storage);
    setStorageDetailsExpanded(storageEditorRequest.settingsExpanded);
    if (storageEditorRequest.settingsExpanded) void ensureStorageEditSession(storage);
    onStorageEditorRequestHandled?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storageEditorRequest?.requestId]);

  useEffect(() => {
    if (!storageSession || !draftDirtyRef.current) return;
    const session = draftSessionRef.current;
    const editVersion = draftEditVersionRef.current;
    const timer = window.setTimeout(() => {
      void (async () => {
        if (draftSessionRef.current !== session || !draftDirtyRef.current) return;
        try {
          await persistStorageSetupDraft(session, editVersion);
        } catch (reason) {
          if (draftSessionRef.current === session) {
            setStorageDraftActionError(`Mallard could not save the storage draft: ${errorMessage(reason)}`);
          }
        }
      })();
    }, STORAGE_DRAFT_AUTOSAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storageSession]);

  const openStorageEditorWithSettings = async (storage: StorageSummary) => {
    const switchingStorage = editingStorage?.storageId !== storage.storage_id;
    if (switchingStorage) {
      if (!(await saveStorageDraftBeforeNavigation())) return;
      clearStorageListing();
      setEditingStorage({ projectId: "", storageId: storage.storage_id });
    }
    setStorageDetailsExpanded(true);
    if (switchingStorage) await loadStorageBundles(storage);
    void ensureStorageEditSession(storage);
  };

  /* The rail can discard the draft this form is editing. Its disappearance
     from the list is the signal — once the list has carried it at all, so a
     listing that started before the draft existed cannot be mistaken for a
     discard — and a commit in flight consumes it on purpose. The check runs
     again when a session opens, because the list may already hold its draft
     and not change again before the discard. */
  const openDraftId = storageSession?.draft.draft_id ?? null;
  useEffect(() => {
    const session = storageSessionRef.current;
    if (!session || committingRef.current) return;
    if (storageSetupDrafts.some((draft) => draft.draft_id === session.draft.draft_id)) {
      draftListedRef.current = true;
      return;
    }
    if (!draftListedRef.current) return;
    const wasNew = !session.draft.edits_existing;
    resetStorageSession();
    setStorageDetailsExpanded(false);
    if (wasNew) {
      clearStorageListing();
      setEditingStorage(null);
      onOpenStorageIndex();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storageSetupDrafts, openDraftId]);

  /* A removed settled storage leaves the page and its edit draft invalid. */
  useEffect(() => {
    if (!editingStorage) return;
    const session = storageSessionRef.current;
    if (session && !session.draft.edits_existing) return;
    const incoming = storages.find((candidate) => candidate.storage_id === editingStorage.storageId) ?? null;
    if (!incoming) {
      resetStorageSession();
      clearStorageListing();
      setEditingStorage(null);
      setStorageDetailsExpanded(false);
      return;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editingStorage?.storageId, storages]);

  const activeProject = projects.find((project) => project.local_project_id === activeProjectId) ?? null;
  const workspaceProject = newProjectSetup ? null : activeProject;
  const activeLocalProjectId = activeProject?.local_project_id ?? null;
  useEffect(() => { setActivityView("sessions"); }, [activeLocalProjectId]);
  const sessionDetailSubview = activeProjectSubview.kind === "session_detail"
    && workspaceProject?.local_project_id === activeProjectSubview.localProjectId
    ? activeProjectSubview
    : null;
  const workspaceBinding = workspaceProject
    ? bindingByProject.get(workspaceProject.local_project_id)
    : undefined;
  // The header summarizes every bound provider profile; a project may bind
  // one provider or several, and none of them is "the" project agent.
  const workspaceProfiles = Object.entries(workspaceBinding?.profile_ids ?? {}).map(
    ([provider, profileId]) => ({
      provider,
      profile: profiles.find((candidate) => candidate.profile_id === profileId),
    }),
  );
  const workspaceProfileReady = workspaceProfiles.length > 0
    && workspaceProfiles.every(({ profile }) => !!profile?.available && profile.readable);
  const workspaceProfileLabel = workspaceProfiles.length === 0
    ? "No agent configured"
    : workspaceProfiles
      .map(({ provider, profile }) => profile?.display_name ?? providerLabel(provider))
      .join(" · ");
  const workspaceProfileTitle = workspaceProfiles.length === 0
    ? "No agent profile is assigned"
    : workspaceProfiles
      .map(({ provider, profile }) => (profile
        ? `${providerLabel(provider)} agent home: ${profile.path}${profile.available && profile.readable ? "" : " (unavailable)"}`
        : `${providerLabel(provider)} profile is missing on this machine`))
      .join("\n");
  const proposedProjectAlias = workspaceProject
    ? projectAliasDraft.trim() && projectAliasDraft.trim() !== workspaceProject.display_name
      ? projectAliasDraft.trim()
      : null
    : null;
  const projectNameChanged = !!workspaceProject
    && proposedProjectAlias !== (workspaceProject.local_alias ?? null);

  // ---- the workspace path bar -------------------------------------------
  // Derive the path-bar state once because the bar renders above the
  // scrolling page.
  const workspaceLinks = workspaceProject
    ? linkedByProject.get(workspaceProject.local_project_id) ?? []
    : [];
  const workspaceLinkedStorages = workspaceLinks
    .map((link) => storages.find((storage) => storage.storage_id === link.storage_id))
    .filter((storage): storage is StorageSummary => !!storage);
  const workspaceStorage = workspaceLinkedStorages.find((storage) => (
    workspaceProject?.local_project_id === activeProjectId && storage.storage_id === activeStorageId
  )) ?? workspaceLinkedStorages[0] ?? null;
  // Only storages this build can open are offered for linking.
  const workspaceAvailableStorages = storages.filter((storage) => (
    storage.available && !workspaceLinks.some((link) => link.storage_id === storage.storage_id)
  ));
  const workspaceBindingActive = workspaceBinding?.state === "active";
  const workspaceReadableProfiles = workspaceProfiles.filter(({ profile }) => (
    !!profile?.available && profile.readable
  ));
  const workspaceProfilesReadable = workspaceReadableProfiles.length > 0;
  const workspaceProfilesWritable = workspaceReadableProfiles.some(({ profile }) => !!profile?.writable);
  const workspaceCanSync = workspaceBindingActive && workspaceProfilesReadable;
  const workspaceCanRestore = workspaceBindingActive && workspaceProfilesWritable;
  const workspaceProfileIssue = workspaceBindingActive && !workspaceProfilesReadable
    ? "No selected agent profile is available on this machine"
    : null;
  // Push and Pull stand down on a storage this build has no adapter for.
  const workspaceStorageUnavailable = !!workspaceStorage && !workspaceStorage.available;
  const workspaceStorageIssue = workspaceStorage && workspaceStorageUnavailable
    ? `This build has no ${workspaceStorage.provider_label} adapter, so ${workspaceStorage.name || "this storage"} cannot sync`
    : null;

  const workspaceActionPrefix = workspaceProject && workspaceStorage
    ? `${workspaceProject.local_project_id}:${workspaceStorage.storage_id}`
    : null;
  const workspaceReviewOpen = !!workspaceActionPrefix
    && inlineStorageReview?.projectId === workspaceProject?.local_project_id
    && inlineStorageReview?.storageId === workspaceStorage?.storage_id;
  const workspaceReviewKind = workspaceReviewOpen ? inlineStorageReview?.kind ?? null : null;
  const workspacePushReviewOpen = workspaceReviewOpen && inlineStorageReview?.kind === "push";
  const workspacePullReviewOpen = workspaceReviewOpen && inlineStorageReview?.kind === "pull";
  const workspacePushPreparing = runningAction === `push:${workspaceActionPrefix}`;
  const workspacePushPublishing = workspacePushReviewOpen && busy;
  const workspacePullPreparing = runningAction === `pull:${workspaceActionPrefix}`;
  const workspacePullApplying = workspacePullReviewOpen && busy;
  const workspacePullProgress = workspacePullPreparing || workspacePullApplying;
  const workspacePushProgress = workspacePushPreparing || workspacePushPublishing;
  const workspacePullReadout = workspacePullPreparing && fetchProgress?.phase === "plan"
    ? `${fetchProgress.index}/${fetchProgress.total_objects} · ${(fetchProgress.fetched_bytes / 1_000_000).toFixed(1)} MB…`
    : null;
  const workspacePushReadout = workspacePushPublishing && fetchProgress?.phase === "push"
    ? `${fetchProgress.index}/${fetchProgress.total_objects} · ${(fetchProgress.fetched_bytes / 1_000_000).toFixed(1)} MB…`
    : null;
  const workspaceControlsLocked = busy
    || !!runningAction
    || storageActionLockedForReview(workspaceReviewKind, "storage");

  // The section the page is showing. The activity page picks Git history for a
  // git repo; a non-git project has two faces — Sessions and Files — and the
  // crumb names the one on screen. It reads the same facts off the project and
  // the view state so the trail is right before the page has finished loading.
  const workspaceSectionLabel = workspaceProject?.is_git_repository
    ? "Git history"
    : activityView === "files" ? "Files" : "Sessions";
  const [sessionHeader, setSessionHeader] = useState<SessionHeaderModel | null>(null);
  const handleSessionHeaderChange = useCallback((header: SessionHeaderModel | null) => {
    setSessionHeader(header);
  }, []);

  const saveProjectName = async () => {
    if (!workspaceProject || !projectNameChanged) return;
    let result: RenameResult = { ok: false, message: null };
    setRenameError(null);
    await run(
      `rename:${workspaceProject.local_project_id}`,
      async () => {
        result = await onRenameProject(workspaceProject.local_project_id, proposedProjectAlias);
      },
    );
    if (result.ok) {
      setRenamingProjectId(null);
      return;
    }
    setRenameError(result.message);
  };

  const saveStorageName = async () => {
    const storage = savedEditedStorage;
    if (!storage || renamingStorageId !== storage.storage_id) return;
    const name = storageNameDraft.trim();
    if (!name || name === storage.name) return;
    await run(`rename-storage:${storage.storage_id}`, async () => {
      const renamed = await onRenameStorage(storage.storage_id, name);
      if (!renamed) return;
      // An open edit draft would write the old name back on Finish, so it
      // follows the rename; the autosave carries it.
      const session = storageSessionRef.current;
      if (session?.draft.edits_existing && session.form.name !== name) {
        draftDirtyRef.current = true;
        updateStorageSession((current) => ({ ...current, form: { ...current.form, name } }));
      }
      setStorageNameDraft(name);
      setRenamingStorageId(null);
    });
  };

  if (editingStorage && (savedEditedStorage || storageSession)) {
    const pageStorageId = editingStorage.storageId;
    const descriptor = storageSession
      ? descriptorFor(storageSession.draft)
      : savedEditedStorage
        ? findStorageProvider(storageProviders, savedEditedStorage.provider, savedEditedStorage.config_version)
        : null;
    const secretPresence = storageSession ? secretPresenceOf(storageSession.draft) : {};
    const storageReady = !!storageSession && !!descriptor
      && storageDraftReady(descriptor, storageSession.form, secretPresence);
    /* A folder another storage already occupies is refused here, before a
       verb is offered: the backend refuses the same folder on Test and Save
       (its check also sees through symlinks), but by then the page would
       have listed the other storage's repositories as this one's. */
    const destinationConflict = storageSession && descriptor
      ? findDestinationConflict(descriptor, storageSession.form, storages, pageStorageId)
      : null;
    const progress = storageSession && descriptor
      ? fieldsSet(descriptor, storageSession.form, secretPresence)
      : null;
    /* Nothing yet to name a draft that is being created; an edit draft's
       storage is named by the settled record. */
    const storageName = creatingStorage
      ? storageSession?.form.name || "(unnamed)"
      : savedEditedStorage?.name || "(unnamed)";
    const providerLabelForPage = savedEditedStorage?.provider_label
      ?? storageSession?.draft.provider_label
      ?? "";
    const storageAvailable = savedEditedStorage?.available ?? storageSession?.draft.available ?? false;
    const supportsManagedClear = storageAvailable && (descriptor?.clear_mode === "revision_checked" || descriptor?.clear_mode === "unconditional");
    const showStorageDetails = creatingStorage
      || (storageDetailsExpanded && (!!storageSession || storageSessionLoading));
    const storageDetailsId = `storage-${pageStorageId}-settings`;
    const storageNameChanged = !!storageNameDraft.trim() && storageNameDraft.trim() !== savedEditedStorage?.name;
    const storageLinkCount = links.filter((link) => link.storage_id === pageStorageId).length;
    const renamingStorage = renamingStorageId === pageStorageId;
    const pathFact = savedEditedStorage?.summary.find((field) => field.is_path) ?? null;
    const formLocked = busy || !!runningAction || r2.state.running || dropbox.state.running;
    const storageActionBlocked = formLocked || !storageReady || destinationConflict !== null;

    const saveEditedStorage = () => run(`storage:${pageStorageId}`, async () => {
      if (!storageSession) return;
      const session = draftSessionRef.current;
      const editVersion = draftEditVersionRef.current;
      const savedDraft = await persistStorageSetupDraftForAction(
        session,
        editVersion,
        "finishing setup",
      );
      if (!savedDraft) return;
      setStorageTestVerified(false);
      committingRef.current = true;
      const result = await onCommitStorageDraft(savedDraft.draft_id, savedDraft.revision);
      committingRef.current = false;
      if (!result || draftSessionRef.current !== session) return;
      // A successful commit consumes the draft. Only the page remains, now on
      // the settled storage, whether it was created or replaced.
      resetStorageSession();
      setStorageDetailsExpanded(false);
      onOpenStorage(result.storage_id, false);
    });

    /* The advisory twin of Finish: same gate, same probe, no commit. The
       verdict is only as fresh as the fields it tested, so every edit, save,
       and close clears it. */
    const testStorageConnection = () => run(`test-storage:${pageStorageId}`, async () => {
      if (!storageSession) return;
      setStorageTestVerified(false);
      const session = draftSessionRef.current;
      const editVersion = draftEditVersionRef.current;
      const savedDraft = await persistStorageSetupDraftForAction(
        session,
        editVersion,
        "testing the connection",
      );
      if (!savedDraft) return;
      const verified = await onTestStorageDraft(savedDraft.draft_id, savedDraft.revision);
      if (verified && draftSessionRef.current === session) setStorageTestVerified(true);
    });

    const discardStorageDraftAndClose = () => run(`discard-draft:${pageStorageId}`, async () => {
      const session = storageSessionRef.current;
      if (!session) return;
      const approved = await confirm(
        `Discard the storage draft for ${session.form.name || "(unnamed)"}?\n\nThis removes only Mallard’s local setup draft; external storage resources are not changed.`,
        { title: "Discard storage draft" },
      );
      if (!approved) return;
      const wasNew = !session.draft.edits_existing;
      try {
        await storageDraftSaveQueue.enqueue(
          () => projectSyncApi.discardStorageSetupDraft(session.draft.draft_id),
        );
      } catch (reason) {
        setStorageDraftActionError(`Mallard could not discard the storage draft: ${errorMessage(reason)}`);
        return;
      }
      resetStorageSession();
      setStorageDetailsExpanded(false);
      onStorageDraftsChanged?.();
      if (wasNew) onOpenStorageIndex();
    });

    /* The door out of a taken folder: the draft that named it is dropped —
       a fresh one has nothing worth keeping, an edit draft is reverted — and
       the storage that owns the folder opens in its place. No confirmation:
       the user chose the door over the field. */
    const openConflictingStorage = (storage: StorageSummary) => run(`open-conflict:${pageStorageId}`, async () => {
      const session = storageSessionRef.current;
      if (!session) return;
      try {
        await storageDraftSaveQueue.enqueue(
          () => projectSyncApi.discardStorageSetupDraft(session.draft.draft_id),
        );
      } catch (reason) {
        setStorageDraftActionError(`Mallard could not discard the storage draft: ${errorMessage(reason)}`);
        return;
      }
      resetStorageSession();
      setStorageDetailsExpanded(false);
      onStorageDraftsChanged?.();
      onOpenStorage(storage.storage_id, false);
    });

    /* The header's type switch restarts a new draft on another provider:
       the current draft goes, a typed name comes along, a default one is
       re-minted for the new type. */
    const switchStorageProvider = (provider: StorageProviderDescriptor) => run(`switch-provider:${pageStorageId}`, async () => {
      const session = storageSessionRef.current;
      if (!session || !descriptor) return;
      if (provider.key === descriptor.key && provider.config_version === descriptor.config_version) return;
      const keptName = storageNameIsAuto(descriptor, session.form) ? undefined : session.form.name;
      try {
        await storageDraftSaveQueue.enqueue(
          () => projectSyncApi.discardStorageSetupDraft(session.draft.draft_id),
        );
      } catch (reason) {
        setStorageDraftActionError(`Mallard could not discard the storage draft: ${errorMessage(reason)}`);
        return;
      }
      resetStorageSession();
      setStorageDetailsExpanded(false);
      onStorageDraftsChanged?.();
      await onCreateStorageDraft(provider.key, keptName);
    });

    /* The New-storage doors. Create new is today's card; Use existing is a
       read-only look at the account for the draft's provider, then one row
       chosen and prepared. Taking a door drops whatever the other found. */
    const chooseSetupMode = (mode: StorageSetupMode) => {
      if (mode === storageSetupMode) return;
      if (formLocked || (r2.isRunning() || dropbox.isRunning())) return;
      r2.dismiss();
    dropbox.dismiss();
      setStorageSetupMode(mode);
      setManualEntry(false);
    };
    /* The doors are asked of a blank new draft and stay as a switch once
       taken; a draft that already holds values — resumed, or prepared by
       the agent or the look — is past the question. */
    const r2Ready = (r2.state.phase === "review" || dropbox.state.phase === "review");
    const askingSetupMode = creatingStorage && !!descriptor?.discovery
      && (storageSetupMode !== null || (progress?.set ?? 0) === 0) && !r2Ready;
    /* Hide keeps the draft: it is autosaved and listed on the rail, and the
       page's own Discard is the way to drop it. */
    const toggleStorageDetails = async () => {
      if (storageDetailsExpanded && storageSession) {
        if (!(await saveStorageDraftBeforeNavigation())) return;
        setStorageDetailsExpanded(false);
        return;
      }
      setStorageDetailsExpanded(true);
      if (savedEditedStorage && !storageSession) void ensureStorageEditSession(savedEditedStorage);
    };

    // The trail starts at the Storages index the crumb leads back to; the
    // storage itself carries the switcher; the section names the page. A
    // draft being created has no section yet and nothing to switch to, so
    // its trail is just the root and the leaf that says what it will be.
    const storagePathCrumbs: PathBarCrumb[] = [
      {
        id: "storages",
        label: "Storages",
        icon: "storage",
        tone: "root",
        title: "All storages",
        ariaLabel: "All storages",
        onActivate: onOpenStorageIndex,
        disabled: busy,
      },
      creatingStorage || !savedEditedStorage
        ? { id: "storage", label: "New storage" }
        : {
            id: "storage",
            label: storageName,
            title: `${storageName} · ${storageSubtitle(savedEditedStorage)}`,
            ariaLabel: `${storageName}. Storage menu`,
            // The page below is this storage, so its verbs stay in the page's
            // overflow; the crumb menu is the header naming where you are and
            // the ways to be somewhere else.
            menuHeader: {
              label: storageName,
              detail: storageSubtitle(savedEditedStorage),
              detailIsPath: storageSubtitleIsPath(savedEditedStorage),
              icon: iconNameFromHint(savedEditedStorage.icon, "drive"),
            },
            menu: [
              ...(storages.some((candidate) => candidate.storage_id !== pageStorageId) ? [{
                label: "Switch to",
                items: storages
                  .filter((candidate) => candidate.storage_id !== pageStorageId)
                  .map((candidate) => ({
                    id: candidate.storage_id,
                    label: candidate.name || "(unnamed)",
                    detail: storageSubtitle(candidate),
                    detailIsPath: storageSubtitleIsPath(candidate),
                    icon: iconNameFromHint(candidate.icon, "drive"),
                    disabled: busy,
                    onSelect: () => onOpenStorage(candidate.storage_id, false),
                  })),
              }] : []),
              {
                items: [{
                  id: "add-storage",
                  label: "Add storage…",
                  icon: "plus" as const,
                  disabled: busy,
                  onSelect: () => onAddStorage(),
                }],
              },
            ],
            menuLabel: "Storage",
            disabled: busy,
          },
      ...(creatingStorage ? [] : [{
        id: "section",
        label: "Repositories",
        tone: "section" as const,
      }]),
    ];

    // Storage commands plus the sidebar's storage shortcuts. A draft receives
    // only app-level commands. A storage this build cannot open keeps the
    // record's verbs — rename, remove — and loses the adapter's: settings,
    // clearing its data.
    const storagePathOverflow: PathBarMenuGroup[] = creatingStorage ? appActions : [
      {
        label: "Storage",
        items: [
          ...(pathFact ? [{
            id: "reveal",
            label: "Reveal in Finder",
            detail: compactProjectPath(pathFact.value),
            icon: "folder" as const,
            onSelect: () => revealPath(pathFact.value),
          }] : []),
          ...(storageAvailable ? [{
            id: "storage-settings",
            label: storageDetailsExpanded && storageSession ? "Hide storage settings" : "Storage settings",
            icon: "settings" as const,
            disabled: busy,
            onSelect: toggleStorageDetails,
          }] : []),
          {
            id: "rename",
            label: "Rename storage…",
            icon: "edit" as const,
            disabled: busy,
            onSelect: () => {
              setStorageNameDraft(savedEditedStorage?.name ?? "");
              setRenamingStorageId(pageStorageId);
            },
          },
          ...(supportsManagedClear ? [{
            id: "clear",
            label: "Clear storage data…",
            icon: "trash" as const,
            danger: true,
            disabled: busy,
            onSelect: () => onClearStorage(pageStorageId),
          }] : []),
          {
            id: "remove",
            label: "Remove storage…",
            icon: "x" as const,
            danger: true,
            disabled: busy || storageLinkCount > 0,
            disabledReason: storageLinkCount > 0
              ? `Linked to ${storageLinkCount} project${storageLinkCount === 1 ? "" : "s"} — unlink first`
              : undefined,
            title: storageLinkCount > 0
              ? `${storageName} is linked to ${storageLinkCount} project${storageLinkCount === 1 ? "" : "s"}; unlink it before removing`
              : `Remove ${storageName}; synced files stay in storage`,
            onSelect: () => onRemoveStorage(pageStorageId),
          },
        ],
      },
      ...appActions,
    ];

    // Creation uses the form's commit bar. A settled storage exposes Refresh
    // in the path bar.
    const storagePathActions = creatingStorage || !savedEditedStorage ? undefined : (
      <button
        type="button"
        className="sync-path-sync"
        disabled={busy || bundleLoading || !storageAvailable}
        title={`Refresh repositories in ${storageName}`}
        aria-label={`Refresh repositories in ${storageName}`}
        onClick={() => void loadStorageBundles(savedEditedStorage)}
      >
        <Icon name="refresh" size={14} className={bundleLoading ? "icon-spin" : undefined} />
        <span className="sync-path-sync-label">Refresh</span>
      </button>
    );

    const storageNotices: NoticeItem[] = [];
    if (storageDraftActionError) {
      storageNotices.push({
        key: "storage-draft-save-failed",
        severity: "danger",
        message: storageDraftActionError,
      });
    }
    if (storageSession && !descriptor) {
      storageNotices.push({
        key: "storage-provider-unavailable",
        severity: "warning",
        message: `This build has no ${providerLabelForPage} adapter (configuration version ${storageSession.draft.config_version}), so this draft cannot be edited, tested, or finished here.`,
      });
    }
    /* A refused credential is the one standing fact about this storage:
       every read fails the same way until it changes, so the repository
       listing's failure says nothing more. The action opens the settings,
       where the same namespace is kept and only the credential is asked
       for again; once open, the editor's guide row carries it. */
    const storageConnection = savedEditedStorage?.connection ?? null;
    const connectionRefused = !creatingStorage && connectionSuspends(storageConnection);
    if (connectionRefused && storageConnection) {
      storageNotices.push({
        key: "storage-connection",
        severity: "danger",
        message: connectionDetail(storageConnection),
        action: showStorageDetails ? undefined : (
          <button
            type="button"
            className="btn btn-ghost"
            disabled={formLocked}
            onClick={() => void toggleStorageDetails()}
          >
            Reconnect {providerLabelForPage}
          </button>
        ),
      });
    }
    if (!creatingStorage && !bundleLoading && bundleError && !connectionRefused) {
      storageNotices.push({ key: "repository-list-failed", severity: "danger", message: bundleError });
    }
    if (!creatingStorage && !bundleLoading && !bundleError && catalogRepoError) {
      storageNotices.push({ key: "catalog-repositories-failed", severity: "danger", message: catalogRepoError });
    }
    if (revealError) {
      storageNotices.push({ key: "reveal-failed", severity: "danger", message: revealError });
    }
    /* Initial read failures are standing notices. Action failures announce. */
    const storageNoticeTiming: NoticeTiming = storageDraftActionError
      || revealError
      ? { timing: "blocking" }
      : { timing: "standing", label: "Problems with this storage" };

    /* On an existing storage the agent's job is regaining access to the
       same destination, so its verb says so whether or not access has
       been refused yet. */
    /* Reset setup, for either lane: this draft and all of its progress go,
       a fresh draft opens on the doors. A bucket or folder already created
       at the provider and any connected storage are kept. */
    const resetR2Setup = () => {
      if (formLocked || (r2.isRunning() || dropbox.isRunning()) || !descriptor) return;
      void run(`reset-r2:${pageStorageId}`, async () => {
        const session = storageSessionRef.current;
        if (!session) return;
        try {
          await storageDraftSaveQueue.enqueue(
            () => projectSyncApi.discardStorageSetupDraft(session.draft.draft_id),
          );
        } catch {
          setStorageDraftActionError("Mallard could not reset setup. Select Reset setup to try again.");
          return;
        }
        resetStorageSession();
        setStorageDetailsExpanded(false);
        onStorageDraftsChanged?.();
        await onCreateStorageDraft(descriptor.key, descriptor.default_name ?? undefined);
      });
    };
    const setupSlot = storageSession && descriptor?.local_setup === "wrangler"
      ? <R2SetupLane
          mode={r2Mode}
          flow={r2}
          disabled={formLocked}
          descriptor={descriptor}
          storages={storages}
          bucketName={storageSession.form.settings.bucket}
          onOpenStorage={(storageId) => onOpenStorage(storageId, false)}
          onCreateInstead={() => chooseSetupMode("create")}
          onReset={resetR2Setup}
        />
      : storageSession && descriptor?.local_setup === "dropbox"
        ? <DropboxSetupLane flow={dropbox} mode={r2Mode} disabled={formLocked} descriptor={descriptor} storages={storages}
            onOpenStorage={(id) => onOpenStorage(id, false)} onCreateInstead={() => chooseSetupMode("create")}
            onNewSetup={() => void run("new-dropbox-account", async () => {
              // Preserve the original draft and its journal for recovery.
              if (!(await saveStorageDraftBeforeNavigation())) return;
              resetStorageSession();
              await onCreateStorageDraft(descriptor.key);
              onStorageDraftsChanged?.();
            })}
            onReset={resetR2Setup} />
        : undefined;

    return (
      <>
      <SyncPathBar
        crumbs={storagePathCrumbs}
        overflow={storagePathOverflow}
        overflowLabel={creatingStorage ? "App actions" : "Storage and app actions"}
        overflowDisabled={busy}
        actions={storagePathActions}
        ariaLabel={`${creatingStorage ? "New storage" : storageName} path`}
      />
      <main className="sync-main sync-project-links-page sync-storage-settings-page">
        <section
          className="profile-links-section"
          aria-label={creatingStorage ? "New storage" : storageName}
        >
          {/* Renaming opens from the ⋯ as a row under the trail, the same
              once-in-a-while surface the project rename uses. */}
          {renamingStorage && (
            <form
              className="sync-path-rename"
              onSubmit={(event) => {
                event.preventDefault();
                void saveStorageName();
              }}
            >
              <label htmlFor="sync-storage-rename-input">Storage name</label>
              <input
                id="sync-storage-rename-input"
                value={storageNameDraft}
                onChange={(event) => setStorageNameDraft(event.target.value)}
                aria-label="Storage name"
                disabled={busy}
                autoFocus
              />
              <button type="submit" className="btn btn-primary" disabled={busy || !storageNameChanged}>
                {runningAction === `rename-storage:${pageStorageId}` ? "Saving…" : "Save"}
              </button>
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => {
                  setStorageNameDraft(savedEditedStorage?.name ?? "");
                  setRenamingStorageId(null);
                }}
              >
                Cancel
              </button>
            </form>
          )}

          <div
            ref={storageSettingsRef}
            className={`sync-storage-settings-below sync-storage-settings-dedicated${showStorageDetails ? " settings-expanded" : " settings-collapsed"}`}
          >
            {/* The column's one box, at the head of the column it speaks for,
                the way the session page's zone sits under its header. */}
            <NoticeZone
              items={storageNotices}
              subject="problems with this storage"
              {...storageNoticeTiming}
            />


            <div
              id={storageDetailsId}
              className="sync-storage-details-panel"
              hidden={!showStorageDetails}
            >
              {showStorageDetails && storageSessionLoading && (
                <div className="sync-storage-repository-state"><span className="status-loader" /> Opening storage settings…</div>
              )}
              {showStorageDetails && storageSession && descriptor && (
                <StorageEditor
                  /* Identity, so every per-draft control state inside the
                     editor — which secrets are revealed — resets when the
                     draft does. */
                  key={storageSession.draft.draft_id}
                  descriptor={descriptor}
                  providers={storageProviders}
                  form={storageSession.form}
                  secretPresence={secretPresence}
                  disabled={formLocked}
                  editsExisting={!creatingStorage}
                  connection={creatingStorage ? null : storageConnection}
                  destinationConflict={destinationConflict
                    ? { storage: destinationConflict, onOpen: () => void openConflictingStorage(destinationConflict) }
                    : null}
                  setupMode={askingSetupMode ? { mode: storageSetupMode, onChoose: chooseSetupMode } : undefined}
                  manualEntry={manualEntry}
                  setupDone={r2Ready}
                  onSwitchProvider={creatingStorage ? switchStorageProvider : undefined}
                  onRemove={!creatingStorage && storageLinkCount === 0 ? () => onRemoveStorage(pageStorageId) : undefined}
                  // A draft's create is the page's verb and lives in the
                  // commit bar below; a settled storage's save belongs to the
                  // drawer it commits.
                  primaryAction={creatingStorage ? undefined : (
                    <>
                      <button
                        type="button"
                        className="btn"
                        disabled={storageActionBlocked}
                        onClick={() => void testStorageConnection()}
                      >
                        {runningAction === `test-storage:${pageStorageId}` ? "Testing…" : "Test connection"}
                      </button>
                      <button
                        type="button"
                        className="btn btn-primary"
                        disabled={storageActionBlocked}
                        onClick={() => void saveEditedStorage()}
                      >
                        {runningAction === `storage:${pageStorageId}` ? "Saving…" : "Save storage"}
                      </button>
                    </>
                  )}
                  onChange={(nextForm) => {
                    // An edit invalidates the current setup result.
                    draftEditVersionRef.current += 1;
                    r2.dismiss();
                    dropbox.dismiss();
                    // The same edit stales a Test verdict and arms the
                    // draft autosave.
                    setStorageTestVerified(false);
                    setStorageDraftActionError(null);
                    draftDirtyRef.current = true;
                    updateStorageSession((current) => ({ ...current, form: nextForm }));
                  }}
                  setupSlot={setupSlot}
                />
              )}
            </div>

            {creatingStorage && showStorageDetails && storageSession && (
              <footer className="sync-storage-setup-footer">
                {progress && progress.total > 0 && (descriptor?.secrets.length ?? 0) > 0 && (
                  <span className="sync-storage-editor-progress" aria-live="polite">
                    {progress.set} of {progress.total} set
                  </span>
                )}
                <div>
                  {/* The verdict rides with the verbs rather than sitting in
                      the caption slot beside the tally: it is the fact that
                      unlocks Finish setup, so it belongs where the eye is on
                      its way there, and it wears a chip so it carries at a
                      glance rather than reading as more chrome. */}
                  {storageTestVerified && (
                    <span className="storage-setup-verified" role="status">
                      <Icon name="check-circle" size={13} /> Connection verified
                    </span>
                  )}
                  <button
                    type="button"
                    className="btn btn-ghost"
                    disabled={formLocked}
                    onClick={() => void discardStorageDraftAndClose()}
                  >
                    Discard draft…
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost"
                    disabled={formLocked}
                    onClick={onOpenStorageIndex}
                    aria-label="Close setup; the draft is kept"
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn"
                    disabled={storageActionBlocked}
                    onClick={() => void testStorageConnection()}
                  >
                    {runningAction === `test-storage:${pageStorageId}` ? "Testing…" : "Test connection"}
                  </button>

                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={storageActionBlocked}
                      onClick={() => void saveEditedStorage()}
                    >
                      {descriptor && (descriptor.secrets.length > 0 || descriptor.local_setup)
                        ? runningAction === `storage:${pageStorageId}` ? "Finishing…" : "Finish setup"
                        : runningAction === `storage:${pageStorageId}` ? "Creating…" : "Create storage"}
                    </button>

                </div>
              </footer>
            )}

            {!creatingStorage && (
              <section className="sync-storage-repositories" aria-labelledby="storage-repositories-heading">
                <div className="sync-storage-repositories-heading">
                  <h2 id="storage-repositories-heading">Repositories</h2>
                </div>

                {bundleLoading ? (
                  <div className="sync-storage-repository-state"><span className="status-loader" /> Loading repositories…</div>
                ) : !storageAvailable ? (
                  <div className="sync-storage-repository-state">
                    {`This build has no ${providerLabelForPage} adapter, so the repositories in this storage cannot be listed.`}
                  </div>
                ) : orderedRemoteBundles.length === 0 && orderedCatalogRepos.length === 0 ? (
                  /* Emptiness is tested before failure, because the two lists
                     are read independently: a project-bundle listing that
                     refuses says nothing about the Catalog repositories that
                     loaded beside it, and hiding the rows because its sibling
                     failed left the card showing a heading and nothing else.
                     A failure only suppresses the line below, which is
                     reserved for a storage that really does hold nothing —
                     the refusal itself is already a rung of the column's one
                     box above, so the card does not say it twice. */
                  bundleError || catalogRepoError ? null : (
                    <div className="sync-storage-repository-state">No repositories in this storage.</div>
                  )
                ) : (
                  <div className="sync-storage-repository-list">
                    {orderedRemoteBundles.map((bundle) => (
                      <StorageRepositoryRow key={bundle.bundle_id} bundle={bundle} />
                    ))}
                    {orderedCatalogRepos.map((repo) => (
                      <CatalogStorageRepositoryRow key={repo.repository_id} repo={repo} />
                    ))}
                  </div>
                )}
              </section>
            )}
          </div>
        </section>
      </main>
      </>
    );
  }

  if (!workspaceProject && !editingStorage && !newProjectSetup) {
    return (
      <main className="sync-main sync-project-links-page sync-landing-page">
        <ProjectPicker
          projects={projects}
          profiles={profiles}
          providers={providers}
          storages={storages}
          onAddProject={onAddProject}
          onSelectProject={onSelectProject}
        />
      </main>
    );
  }

  // Storage leads the trail because it is the outermost scope and the thing
  // Push and Pull act on; the repo sits inside it; the section is the page.
  // Drilling into a session appends a fourth crumb and turns the section into
  // the way back — the trail grows, it never swaps.
  const pathCrumbs: PathBarCrumb[] = [];
  if (workspaceProject) {
    if (workspaceStorage) {
      // The menu speaks for the current storage: a header, settings and unlink,
      // other storages as switch commands, then the ways to add one. The header
      // identifies the current storage, so switching rows need no radio state.
      //
      // While the workspace is locked the menu still opens; one banner says
      // why every row is inert, instead of a tooltip per greyed row.
      const storageMenuBanner = workspaceControlsLocked
        ? workspacePushProgress || workspacePushReviewOpen
          ? "Push in progress — storage can't change until it finishes."
          : workspacePullProgress || workspacePullReviewOpen
            ? "Pull in progress — storage can't change until it finishes."
            : "Working — storage can't change right now."
        : undefined;
      const otherStorages = workspaceLinkedStorages.filter((storage) => (
        storage.storage_id !== workspaceStorage.storage_id
      ));
      const storageMenu: PathBarMenuGroup[] = [
        {
          items: [
            {
              id: "storage-settings",
              label: "Storage settings…",
              icon: "settings" as const,
              disabled: workspaceControlsLocked,
              onSelect: () => { void openStorageEditorWithSettings(workspaceStorage); },
            },
            {
              id: "unlink",
              label: `Unlink from ${projectLabel(workspaceProject)}…`,
              icon: "link-off" as const,
              danger: true,
              disabled: workspaceControlsLocked,
              title: "Stop syncing this pairing; files already in storage are kept",
              onSelect: () => void run(
                `unlink:${workspaceActionPrefix}`,
                () => onUnlinkStorage(workspaceProject.local_project_id, workspaceStorage.storage_id),
              ),
            },
          ],
        },
        ...(otherStorages.length > 0 ? [{
          label: "Switch to",
          items: otherStorages.map((storage) => ({
            id: storage.storage_id,
            label: storage.name || "(unnamed)",
            detail: storageSubtitle(storage),
            detailIsPath: storageSubtitleIsPath(storage),
            icon: iconNameFromHint(storage.icon, "drive"),
            disabled: workspaceControlsLocked,
            onSelect: () => {
              if (workspaceReviewOpen) inlineStorageReview?.onClose();
              void onSelectStorage(workspaceProject.local_project_id, storage.storage_id);
            },
          })),
        }] : []),
        {
          items: [
            // Stays put at zero candidates and says why, instead of vanishing
            // from the DOM for a reason the user cannot see.
            {
              id: "link-storage",
              label: "Link storage…",
              icon: "link" as const,
              disabled: workspaceControlsLocked || workspaceAvailableStorages.length === 0,
              disabledReason: workspaceAvailableStorages.length === 0
                ? "Everything you've added is already linked"
                : undefined,
              onSelect: () => {
                setLinkingProjectId((current) => (
                  current === workspaceProject.local_project_id
                    ? null
                    : workspaceProject.local_project_id
                ));
              },
            },
            {
              id: "add-storage",
              label: "Add storage…",
              icon: "plus" as const,
              disabled: workspaceControlsLocked,
              onSelect: () => {
                setLinkingProjectId(null);
                onAddStorage();
              },
            },
          ],
        },
      ];
      pathCrumbs.push({
        id: "storage",
        label: workspaceStorage.name || "(unnamed)",
        icon: iconNameFromHint(workspaceStorage.icon, "drive"),
        tone: "root",
        title: `${workspaceStorage.name || "Storage"} · ${storageSubtitle(workspaceStorage)}`,
        ariaLabel: `${workspaceStorage.name || "Unnamed storage"}. Storage menu`,
        menuHeader: {
          label: workspaceStorage.name || "(unnamed)",
          detail: storageSubtitle(workspaceStorage),
          detailIsPath: storageSubtitleIsPath(workspaceStorage),
          icon: iconNameFromHint(workspaceStorage.icon, "drive"),
        },
        menuBanner: storageMenuBanner,
        menu: storageMenu,
        menuLabel: "Storage",
      });
    }

    // Same grammar as the storage crumb: header, project commands, other
    // projects as switch commands, then Add.
    const otherProjects = projects.filter((candidate) => (
      candidate.local_project_id !== workspaceProject.local_project_id
    ));
    pathCrumbs.push({
      id: "project",
      label: projectLabel(workspaceProject),
      icon: workspaceProject.is_git_repository ? "git-folder" : "folder",
      warning: !workspaceProfileReady,
      title: [
        workspaceProject.project_root ?? "Folder not configured",
        workspaceProfileTitle,
      ].filter(Boolean).join("\n"),
      ariaLabel: sessionDetailSubview
        ? `Back to ${workspaceSectionLabel} for ${projectLabel(workspaceProject)}`
        : `${projectLabel(workspaceProject)}. Project menu`,
      // Inside a session the repo is the ancestor you came from, so its label
      // walks back out to the project's page; the menu stays on its chevron.
      onActivate: sessionDetailSubview ? closeSessionDetail : undefined,
      menuHeader: {
        label: projectLabel(workspaceProject),
        detail: workspaceProject.project_root
          ? compactProjectPath(workspaceProject.project_root)
          : "Folder not configured",
        detailIsPath: !!workspaceProject.project_root,
        icon: workspaceProject.is_git_repository ? "git-folder" : "folder",
      },
      menu: [
        {
          items: [
            ...(workspaceProject.project_root ? [{
              id: "reveal",
              label: "Reveal in Finder",
              icon: "folder" as const,
              onSelect: () => revealPath(workspaceProject.project_root as string),
            }] : []),
            {
              id: "agent",
              label: "Agent profile…",
              detail: workspaceProfileLabel,
              detailIsPath: false,
              icon: "terminal" as const,
              title: workspaceProfileTitle,
              onSelect: () => void onConfigureBinding(workspaceProject.local_project_id),
            },
            {
              id: "rename",
              label: "Rename project…",
              icon: "edit" as const,
              disabled: busy,
              onSelect: () => {
                setProjectAliasDraft(workspaceProject.local_alias ?? workspaceProject.display_name);
                setRenamingProjectId(workspaceProject.local_project_id);
              },
            },
            {
              id: "reset-sync-metadata",
              label: "Reset local sync state…",
              icon: "trash" as const,
              danger: true,
              disabled: busy,
              title: "Forget local sync bases, conversation mappings, and results; files and cloud data are kept",
              onSelect: () => void onResetProjectMetadata(workspaceProject.local_project_id),
            },
          ],
        },
        ...(otherProjects.length > 0 ? [{
          label: "Switch to",
          items: otherProjects.map((candidate) => ({
            id: candidate.local_project_id,
            label: projectLabel(candidate),
            detail: compactProjectPath(candidate.project_root),
            detailIsPath: true,
            icon: candidate.is_git_repository ? ("git-folder" as const) : ("folder" as const),
            disabled: busy || !!inlineStorageReview,
            onSelect: () => onSelectProject(candidate.local_project_id),
          })),
        }] : []),
        {
          items: [{
            id: "add-project",
            label: "Add project…",
            icon: "plus" as const,
            disabled: busy,
            onSelect: () => onAddProject(),
          }],
        },
      ],
      menuLabel: "Project",
      disabled: busy || !!inlineStorageReview,
    });

    // A session is not inside Git history — it is the thing the history lists.
    // So the section crumb names the page you are on and steps aside once you
    // open a session, giving the title the width it needs.
    if (sessionDetailSubview) {
      // An agent's page sits under the session that delegated to it; the
      // parent is the crumb before the title, and the way back.
      if (sessionHeader?.state === "ready" && sessionHeader.parent) {
        const parent = sessionHeader.parent;
        pathCrumbs.push({
          id: "session-parent",
          label: parent.title,
          title: `Back to ${parent.title}`,
          onActivate: () => onOpenSessionDetail(sessionDetailSubview.localProjectId, {
            resourceId: parent.resourceId,
            occurrenceKey: parent.resourceId,
          }),
        });
      }
      // The leaf is the page's one visible title, so it also carries the
      // page's identity: the provider's mark, and a dot when the session is
      // live. The words stay in the tooltip and in hidden text for readers.
      pathCrumbs.push({
        id: "session",
        label: sessionHeader?.state === "ready"
          ? sessionHeader.title
          : sessionHeader?.state === "failed"
            ? "Session unavailable"
            : "Opening session…",
        title: sessionHeader?.state === "ready"
          ? [sessionHeader.title, sessionHeader.providerDisplayName, sessionHeader.active ? "Active" : null]
            .filter(Boolean).join(" · ")
          : undefined,
        leading: sessionHeader?.state === "ready" ? (
          <span className={`sync-path-leaf-mark${sessionHeader.active ? " active" : ""}`}>
            <ProviderMark
              provider={sessionHeader.provider}
              displayName={sessionHeader.providerDisplayName}
            />
            {sessionHeader.active && <span className="sync-visually-hidden">Active</span>}
          </span>
        ) : undefined,
      });
    } else if (workspaceProject.is_git_repository) {
      // Git history keeps its section and branch in the trail. For a non-Git
      // project, the compact Sessions/Files switch replaces this crumb.
      pathCrumbs.push({
        id: "section",
        label: workspaceSectionLabel,
        tone: "section",
        // Which branch the history is showing is part of where you are, so it
        // rides the crumb that names the section rather than a row of its own.
        // A session replaces this crumb outright. The branch selects the Git
        // commit rail; sessions remain project-wide.
        scope: activityHeader && activityHeader.branches.length > 0 ? (
          <BranchPicker
            branches={activityHeader.branches}
            selectedBranch={activityHeader.selectedBranch}
            disabled={activityHeader.refreshing}
            onChange={activityHeader.onSelectBranch}
          />
        ) : undefined,
      });
    } else if (!inlineStorageReview) {
      // A non-Git project has two faces, and the crumb names the one on
      // screen — the same shape the Git project's section crumb has, so both
      // kinds of project read as one trail. The other face is the crumb's own
      // menu rather than a control parked in the leaf's position: every crumb
      // in this bar offers its alternatives that way, and a two-button pill
      // showed both faces at once while naming neither. The current view is
      // the menu's header, so no row carries a tick.
      const otherView = activityView === "files" ? "sessions" : "files";
      const viewIcon = (view: typeof activityView) => (view === "files" ? "file" as const : "message" as const);
      const viewLabel = (view: typeof activityView) => (view === "files" ? "Files" : "Sessions");
      const viewDetail = (view: typeof activityView) => (
        view === "files" ? "Files outside Git" : "Agent sessions"
      );
      pathCrumbs.push({
        id: "section",
        label: workspaceSectionLabel,
        icon: viewIcon(activityView),
        tone: "section",
        menuLabel: "Switch view",
        menuHeader: {
          label: workspaceSectionLabel,
          detail: viewDetail(activityView),
          icon: viewIcon(activityView),
        },
        menu: [{
          label: "Switch to",
          items: [{
            id: `view-${otherView}`,
            label: viewLabel(otherView),
            detail: viewDetail(otherView),
            icon: viewIcon(otherView),
            onSelect: () => setActivityView(otherView),
          }],
        }],
      });
    }
  }

  // One home per scope: the storage crumb holds the storage verbs, the project
  // crumb holds the project verbs, and the ⋯ is left with exactly what has no
  // crumb of its own — Mallard itself.
  const pathOverflow: PathBarMenuGroup[] = workspaceProject ? appActions : [];

  // A badge on the button that clears it, rather than a tally beside it: "3 to
  // push" is what Push does, said once. The counts stand down while that
  // button is mid-review or mid-transfer, where its own label is the readout.
  const pullCount = activityHeader?.counts.pull ?? 0;
  const pushCount = activityHeader?.counts.push ?? 0;
  const showPullCount = pullCount > 0 && !workspacePullReviewOpen && !workspacePullProgress;
  const showPushCount = pushCount > 0 && !workspacePushReviewOpen && !workspacePushProgress;
  const attention = activityHeader?.counts.attention ?? 0;

  // One unlabelled refresh, scoped to the view: the open session when there is
  // one, the activity comparison otherwise. Pull and Push keep their names and
  // their project scope — only the bare glyph follows what you are looking at.
  const viewRefresh = sessionDetailSubview
    ? sessionHeader?.state === "ready" && {
      refreshing: sessionHeader.refreshing,
      onRefresh: sessionHeader.onRefresh,
      label: sessionHeader.refreshing ? "Refreshing session" : "Refresh session",
    }
    : activityView === "files"
      ? filesActivityHeader && {
        refreshing: filesActivityHeader.refreshing,
        onRefresh: filesActivityHeader.onRefresh,
        label: filesActivityHeader.refreshing ? "Rescanning project files" : "Rescan project files",
      }
      : activityHeader && {
        refreshing: activityHeader.refreshing,
        onRefresh: activityHeader.onRefresh,
        label: activityHeader.refreshing ? "Refreshing activity" : "Refresh activity",
      };

  const syncPathActions = workspaceProject && workspaceStorage ? (
    <>
      {/* One chip summarizes issues that neither action can clear. */}
      {attention > 0 && (
        /* Not role="status": this is a standing count, not an event, and a
           live region re-announces it on every unrelated re-render. The label
           carries what the number means. */
        <span
          className={`sync-path-attention ${activityHeader?.counts.severity ?? "warning"}`}
          tabIndex={0}
          data-tooltip={activityHeader?.counts.detail}
          aria-label={`${attention} thread${attention === 1 ? "" : "s"} need attention: ${activityHeader?.counts.detail}`}
        >
          <Icon name="alert-triangle" size={13} />
          <b>{attention}</b>
        </span>
      )}
      {viewRefresh && (
        <button
          type="button"
          className={`sync-path-icon-action${viewRefresh.refreshing ? " loading" : ""}`}
          onClick={viewRefresh.onRefresh}
          disabled={viewRefresh.refreshing}
          aria-busy={viewRefresh.refreshing}
          title={viewRefresh.label}
          aria-label={viewRefresh.label}
        >
          <span
            aria-hidden="true"
            className={`icon-spin-frame${viewRefresh.refreshing ? " spinning" : ""}`}
          >
            <Icon name="refresh" size={16} />
          </span>
        </button>
      )}
    <div className="sync-path-action-group" role="group" aria-label="Sync actions">
      <button
        type="button"
        className={`sync-path-sync${workspacePullReviewOpen ? " active" : ""}`}
        disabled={busy
          || !!runningAction
          || workspaceStorageUnavailable
          || (workspaceBindingActive && !workspaceCanRestore)
          || storageActionLockedForReview(workspaceReviewKind, "pull")
          || (workspacePullReviewOpen && inlineStorageReview?.continueDisabled)}
        onClick={() => {
          if (workspacePullReviewOpen) {
            // The bar button IS the review's primary: it applies the current
            // selection — an empty one included, which is how a
            // revision-only change is acknowledged.
            inlineStorageReview?.onContinue?.();
            return;
          }
          setLinkingProjectId(null);
          void run(
            `pull:${workspaceActionPrefix}`,
            () => onPull(workspaceProject.local_project_id, workspaceStorage.storage_id),
          );
        }}
        title={workspaceStorageIssue
          ?? workspaceProfileIssue
          ?? (!workspaceBindingActive
            ? "Confirm project setup before pulling"
            : workspacePullReviewOpen
            ? (inlineStorageReview?.continueTitle ?? "Pull the selected changes from this storage")
            : !workspaceProfilesWritable
              ? "No selected agent profile is writable"
              : "Review the Pull actions before applying them")}
        aria-label={!workspaceBindingActive
          ? "Set up project before Pull"
          : workspacePullReviewOpen
          ? `Pull selected changes from ${workspaceStorage.name || "storage"}`
          : showPullCount
            ? `Pull ${pullCount} newer thread${pullCount === 1 ? "" : "s"} from ${workspaceStorage.name || "storage"}`
            : `Pull from ${workspaceStorage.name || "storage"}`}
        aria-expanded={workspacePullReviewOpen}
      >
        <Icon
          key={workspacePullProgress ? "pull-progress" : "pull-idle"}
          name={workspacePullProgress ? "refresh" : "download"}
          size={14}
          className={workspacePullProgress ? "icon-spin" : undefined}
        />
        {workspacePullReadout
          ? <span className="sync-btn-readout">{workspacePullReadout}</span>
          : <span className="sync-path-sync-label">{projectPullActionLabel({ preparing: workspacePullPreparing, applying: workspacePullApplying })}</span>}
        {showPullCount && <span className="sync-path-count" aria-hidden="true">{pullCount}</span>}
      </button>
      <button
        type="button"
        className={`sync-path-sync${workspacePushReviewOpen ? " active" : ""}`}
        disabled={busy
          || !!runningAction
          || workspaceStorageUnavailable
          || !workspaceCanSync
          || storageActionLockedForReview(workspaceReviewKind, "push")
          || (workspacePushReviewOpen && inlineStorageReview?.continueDisabled)}
        onClick={() => {
          if (workspacePushReviewOpen) {
            inlineStorageReview?.onContinue?.();
            return;
          }
          setLinkingProjectId(null);
          void run(
            `push:${workspaceActionPrefix}`,
            () => onPush(workspaceProject.local_project_id, workspaceStorage.storage_id),
          );
        }}
        title={workspaceStorageIssue
          ?? workspaceProfileIssue
          ?? (workspacePushReviewOpen
            ? (inlineStorageReview?.continueTitle ?? "Push the selected resources to this storage")
            : "Choose resources, then push them to this storage")}
        aria-label={workspacePushReviewOpen
          ? `Push selected resources to ${workspaceStorage.name || "storage"}`
          : showPushCount
            ? `Push ${pushCount} newer thread${pushCount === 1 ? "" : "s"} to ${workspaceStorage.name || "storage"}`
            : `Push to ${workspaceStorage.name || "storage"}`}
        aria-expanded={workspacePushReviewOpen}
      >
        <Icon
          key={workspacePushProgress ? "push-progress" : "push-idle"}
          name={workspacePushProgress ? "refresh" : "upload"}
          size={14}
          className={workspacePushProgress ? "icon-spin" : undefined}
        />
        {workspacePushReadout
          ? <span className="sync-btn-readout">{workspacePushReadout}</span>
          : <span className="sync-path-sync-label">{projectPushActionLabel({ preparing: workspacePushPreparing, publishing: workspacePushPublishing })}</span>}
        {showPushCount && <span className="sync-path-count" aria-hidden="true">{pushCount}</span>}
      </button>
    </div>
    </>
  ) : null;
  const pathActions = syncPathActions;

  // A failed reveal is an event and starts a new notice timer.
  const workspaceNotices: NoticeItem[] = [];
  if (revealError) {
    workspaceNotices.push({ key: "reveal-failed", severity: "danger", message: revealError });
  }
  const workspaceNoticeTiming: NoticeTiming = revealError
    ? { timing: "blocking" }
    : { timing: "standing", label: "Notes about this workspace" };

  return (
    <>
    {workspaceProject && (
      <SyncPathBar
        crumbs={pathCrumbs}
        overflow={pathOverflow}
        overflowLabel="App actions"
        actions={pathActions}
        ariaLabel={`${projectLabel(workspaceProject)} path`}
      />
    )}
    <main ref={workspaceScrollRef} className={`sync-main sync-project-links-page${workspaceProject ? " sync-project-workspace-page sync-project-combined-page" : ""}${newProjectSetup ? " sync-project-setup-page" : ""}${inlineStorageReview ? " sync-sync-review-page" : ""}`}>
      <section
        className={`profile-links-section sync-project-overview${sessionDetailSubview ? " session-detail-hidden" : ""}`}
        hidden={sessionDetailSubview != null}
        inert={sessionDetailSubview != null}
        aria-hidden={sessionDetailSubview ? true : undefined}
        aria-label={newProjectSetup
          ? "Project setup"
          : workspaceProject
            ? projectLabel(workspaceProject)
            : undefined}
        aria-labelledby={newProjectSetup || workspaceProject ? undefined : "project-links-heading"}
      >
        {!newProjectSetup && !workspaceProject && (
          <div className="profile-links-heading sync-combined-project-heading">
            <div className="profile-links-copy">
              <h1 id="project-links-heading" className="settings-section-title">Project links</h1>
              <div className="profile-links-subtitle">Choose where each project repo syncs.</div>
            </div>
            <div className="profile-links-heading-actions">
              <div className="profile-links-counts">
                {projects.length} projects <span>·</span> {storages.length} storage <span>·</span> {links.length} links
              </div>
              <div className="profile-links-primary-actions">
                <button type="button" className="btn profile-refresh-linkage" onClick={onRefresh} disabled={loading || busy}>
                  <Icon name="refresh" size={16} className={loading ? "icon-spin" : undefined} />
                  {loading ? "Refreshing…" : "Refresh"}
                </button>
                <button type="button" className="btn profile-add-profile" onClick={onAddProject} disabled={busy}>
                  <Icon name="plus" size={16} /> Add project
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Renaming is a once-in-a-project act, so it opens from the path bar's
            ··· menu as a row under the trail rather than living in the header. */}
        {workspaceProject && renamingProjectId === workspaceProject.local_project_id && (
          <form
            className="sync-path-rename"
            onSubmit={(event) => {
              event.preventDefault();
              void saveProjectName();
            }}
          >
            <label htmlFor="sync-path-rename-input">Project name</label>
            <input
              id="sync-path-rename-input"
              value={projectAliasDraft}
              onChange={(event) => setProjectAliasDraft(event.target.value)}
              aria-label="Project display name"
              disabled={busy}
              autoFocus
            />
            <button type="submit" className="btn btn-primary" disabled={busy || !projectNameChanged}>
              {runningAction === `rename:${workspaceProject.local_project_id}` ? "Saving…" : "Save"}
            </button>
            <button
              type="button"
              className="btn btn-ghost"
              disabled={busy}
              onClick={() => {
                setProjectAliasDraft(workspaceProject.local_alias ?? workspaceProject.display_name);
                setRenamingProjectId(null);
              }}
            >
              Cancel
            </button>
            {workspaceProject.local_alias && (
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => void run(
                  `rename:${workspaceProject.local_project_id}`,
                  async () => {
                    setRenameError(null);
                    const result = await onRenameProject(workspaceProject.local_project_id, null);
                    if (!result.ok) {
                      setRenameError(result.message);
                      return;
                    }
                    setProjectAliasDraft(workspaceProject.display_name);
                    setRenamingProjectId(null);
                  },
                )}
              >
                Use repo name
              </button>
            )}
            {renameError && (
              <div className="sync-path-rename-error" role="alert">
                <Icon name="alert-triangle" size={13} /> {renameError}
              </div>
            )}
          </form>
        )}

        <NoticeZone
          items={workspaceNotices}
          subject="notes about this workspace"
          {...workspaceNoticeTiming}
        />

        {newProjectSetup}

        {!newProjectSetup && (projects.length === 0 ? (
          <div className="profile-links-empty">
            <Icon name="folder" size={24} />
            <span>Add a project to choose which resources and storage belong to it.</span>
            <button type="button" className="btn" onClick={onAddProject} disabled={busy}>
              <Icon name="plus" size={15} /> Add project
            </button>
          </div>
        ) : workspaceProject ? (
          <div id="project-configuration-panel" className="profile-links-list sync-combined-settings-panel">
            {[workspaceProject].map((project) => {
              const projectLinks = linkedByProject.get(project.local_project_id) ?? [];
              const linkedStorages = projectLinks
                .map((link) => storages.find((storage) => storage.storage_id === link.storage_id))
                .filter((storage): storage is StorageSummary => !!storage);
              const selectedStorage = linkedStorages.find((storage) => (
                project.local_project_id === activeProjectId && storage.storage_id === activeStorageId
              )) ?? linkedStorages[0] ?? null;
              const availableStorages = storages.filter((storage) => (
                storage.available && !projectLinks.some((link) => link.storage_id === storage.storage_id)
              ));
              const projectBinding = bindingByProject.get(project.local_project_id);
              const bindingActive = projectBinding?.state === "active";
              const renderStorageConnectionActions = () => (
                <>
                  {availableStorages.length > 0 && (
                    <button
                      type="button"
                      className={`sync-history-icon-action${linkingProjectId === project.local_project_id ? " active" : ""}`}
                      disabled={busy || !!inlineStorageReview}
                      aria-expanded={linkingProjectId === project.local_project_id}
                      aria-controls={`storage-link-picker-${project.local_project_id}`}
                      title="Link storage"
                      aria-label="Link storage"
                      onClick={() => {
                                    setLinkingProjectId((current) => current === project.local_project_id ? null : project.local_project_id);
                      }}
                    >
                      <Icon name="link" size={14} />
                    </button>
                  )}
                  <button
                    type="button"
                    className="sync-history-icon-action"
                    disabled={busy || !!inlineStorageReview}
                    title="Add storage"
                    aria-label="Add storage"
                    onClick={() => {
                      setLinkingProjectId(null);
                      onAddStorage();
                    }}
                  >
                    <Icon name="plus" size={14} />
                  </button>
                </>
              );
              // The bar is the activity page's own section heading — same h2,
              // count chip and icon-button vocabulary as branch-unmapped below it,
              // so the two card headings cannot drift apart.
              const storageHeading = (
                <div className="sync-history-section-heading">
                  <h2><Icon name="link" size={14} />Storage</h2>
                  <span className="project-storage-heading-actions">
                    {projectLinks.length > 0 && (
                      <span
                        className="sync-history-heading-count"
                        title={`${projectLinks.length} linked storage location${projectLinks.length === 1 ? "" : "s"}`}
                        aria-label={`${projectLinks.length} linked storage location${projectLinks.length === 1 ? "" : "s"}`}
                      >
                        <Icon name="cloud" size={12} />{projectLinks.length}
                      </span>
                    )}
                    {!bindingActive && !selectedStorage && (
                      <button
                        type="button"
                        className="project-storage-setup-action"
                        disabled={busy || !!inlineStorageReview}
                        onClick={() => void onConfigureBinding(project.local_project_id)}
                      >
                        <Icon name="folder" size={13} />
                        {projectBinding ? "Confirm setup" : "Set up project"}
                      </button>
                    )}
                    {renderStorageConnectionActions()}
                  </span>
                </div>
              );

              // Storage identity and actions live in the path bar. Do not draw
              // an empty frame when this group has no notice or review.
              const storageBlockShown = !!selectedStorage
                && (!!(storageNotice
                  && storageNotice.projectId === project.local_project_id
                  && storageNotice.storageId === selectedStorage.storage_id)
                  || !!(siblingNotice
                    && siblingNotice.projectId === project.local_project_id
                    && siblingNotice.storageId === selectedStorage.storage_id
                    && siblingNotice.status.state !== "none")
                  || (inlineStorageReview?.projectId === project.local_project_id
                    && inlineStorageReview.storageId === selectedStorage.storage_id));
              const storageGroupEmpty = !!selectedStorage
                && projectLinks.length > 0
                && !storageBlockShown
                && linkingProjectId !== project.local_project_id;
              if (storageGroupEmpty) return null;

              return (
                <article
                  key={project.local_project_id}
                  className="profile-link-card sync-project-link-card"
                >
                  <div className="profile-link-connections">
                    <section
                      className={`project-storage-card${project.is_git_repository ? "" : " project-storage-card-non-git"}`}
                      aria-label={`${projectLabel(project)} storage`}
                    >
                      {!selectedStorage && storageHeading}
                    {projectLinks.length === 0 && (
                      <div className="project-storage-empty">
                        <Icon name="cloud" size={16} />
                        <span>No storage linked yet</span>
                        <button
                          type="button"
                          className="btn"
                          disabled={busy || !!inlineStorageReview}
                          aria-expanded={availableStorages.length > 0
                            ? linkingProjectId === project.local_project_id
                            : undefined}
                          aria-controls={availableStorages.length > 0
                            ? `storage-link-picker-${project.local_project_id}`
                            : undefined}
                          onClick={() => {
                            if (availableStorages.length === 0) {
                              setLinkingProjectId(null);
                              onAddStorage();
                              return;
                            }
                            setLinkingProjectId((current) => current === project.local_project_id ? null : project.local_project_id);
                          }}
                        >
                          <Icon name={availableStorages.length > 0 ? "link" : "plus"} size={13} />
                          {availableStorages.length > 0 ? "Link storage" : "Add storage"}
                        </button>
                      </div>
                    )}

                    {/* The block below returns null unless it has a notice or an
                        open review. Drop the wrapper with it: an empty list is
                        still an element, and it would keep the card footer off
                        `:first-child`, leaving the footer's rule stacked 1px
                        under the card frame as a doubled hairline. */}
                    {storageBlockShown && (
                    <div className="project-profile-storage-list">
                    {selectedStorage && (() => {
                      const storage = selectedStorage;
                      const reviewOpen = inlineStorageReview?.projectId === project.local_project_id
                        && inlineStorageReview.storageId === storage.storage_id;
                      const actionPrefix = `${project.local_project_id}:${storage.storage_id}`;
                      const reviewPanelId = `storage-review-${actionPrefix.replace(/[^a-z0-9_-]/gi, "-")}`;
                      const activeNotice = storageNotice
                        && storageNotice.projectId === project.local_project_id
                        && storageNotice.storageId === storage.storage_id
                        ? storageNotice
                        : null;
                      const noticeActionsDisabled = busy || !!runningAction;
                      const activeSibling = siblingNotice
                        && siblingNotice.projectId === project.local_project_id
                        && siblingNotice.storageId === storage.storage_id
                        && siblingNotice.status.state !== "none"
                        ? siblingNotice.status
                        : null;
                      // This card contains banners and the open review. With
                      // neither, it has nothing to frame.
                      if (!activeNotice && !reviewOpen && !activeSibling) return null;

                      /* The four gates form a discriminated union, so at most
                       * one is present in the shared notice box.
                       *
                       * All four are events without exception: ProjectSync
                       * initialises the notice at null and only ever sets it
                       * inside the Push and Pull handlers, so none of them can
                       * be on screen when the card first renders. Each one is
                       * a Push or a Pull that did not start, so the box stops
                       * the flow whatever rung it reads at. */
                      const gateNotices: NoticeItem[] = [];
                      if (activeNotice?.kind === "requires_pull") {
                        gateNotices.push({
                          key: "requires-pull",
                          /* Waiting on Pull is an informational step, not a
                             failure: one action clears it. */
                          severity: "info",
                          title: activeNotice.copy.title,
                          message: (
                            <>
                              {activeNotice.copy.body}
                              <small className="sync-sync-review-pull-detail">{activeNotice.copy.detail}</small>
                            </>
                          ),
                          action: (
                            <div className="sync-sync-review-recovery-actions">
                              <button
                                type="button"
                                className="btn btn-ghost"
                                disabled={noticeActionsDisabled}
                                onClick={() => void run(`pull:${actionPrefix}`, () => onPull(project.local_project_id, storage.storage_id))}
                              >
                                <Icon name="download" size={13} />
                                Open Pull review
                              </button>
                            </div>
                          ),
                        });
                      }
                      if (activeNotice?.kind === "remote_missing") {
                        gateNotices.push({
                          key: "remote-missing",
                          /* Nothing failed or was lost locally. Push is held
                             until the user resolves the missing remote history. */
                          severity: "warning",
                          title: "Remote history is missing",
                          message: `${storage.name || "This storage"} no longer holds this project's sync history, so Push is paused. If this is expected, unlink and link the storage again.`,
                          action: (
                            <div className="sync-sync-review-recovery-actions">
                              <button
                                type="button"
                                className="btn btn-ghost"
                                disabled={noticeActionsDisabled}
                                onClick={() => onOpenStorage(storage.storage_id, true)}
                              >
                                Storage settings
                              </button>
                              <button
                                type="button"
                                className="btn btn-ghost danger"
                                disabled={noticeActionsDisabled}
                                onClick={() => void run(`unlink:${actionPrefix}`, () => onUnlinkStorage(project.local_project_id, storage.storage_id))}
                              >
                                Unlink storage…
                              </button>
                            </div>
                          ),
                        });
                      }
                      if (activeNotice?.kind === "remote_replaced") {
                        gateNotices.push({
                          key: "remote-replaced",
                          severity: "warning",
                          title: "Storage history changed",
                          message: `${storage.name || "This storage"} holds a different sync history than this project last synced${activeNotice.baseRevision != null ? ` (revision ${activeNotice.baseRevision})` : ""}. Review & adopt pulls the new history; unlinking doesn't delete files.`,
                          action: (
                            <div className="sync-sync-review-recovery-actions">
                              <button
                                type="button"
                                className="btn btn-ghost danger"
                                disabled={noticeActionsDisabled}
                                onClick={() => void run(`unlink:${actionPrefix}`, () => onUnlinkStorage(project.local_project_id, storage.storage_id))}
                              >
                                Unlink storage…
                              </button>
                              <button
                                type="button"
                                className="btn btn-ghost"
                                disabled={noticeActionsDisabled}
                                onClick={() => void run(`pull:${actionPrefix}`, () => onPull(project.local_project_id, storage.storage_id))}
                              >
                                <Icon name="download" size={13} />
                                Review &amp; adopt
                              </button>
                            </div>
                          ),
                        });
                      }
                      if (activeNotice?.kind === "action_failed") {
                        const failedAction = activeNotice.action;
                        const failedMessage = activeNotice.message;
                        gateNotices.push({
                          key: "action-failed",
                          severity: "danger",
                          title: failedAction === "push" ? "Push couldn't start" : "Pull couldn't start",
                          message: failedMessage,
                          action: (
                            <div className="sync-sync-review-recovery-actions">
                              {connectionSuspends(storage.connection) ? (
                                /* A refused credential fails the same way
                                   on every retry; the way forward is the
                                   storage's own settings, same namespace,
                                   credential asked for again. */
                                <button
                                  type="button"
                                  className="btn btn-ghost"
                                  disabled={noticeActionsDisabled}
                                  onClick={() => onOpenStorage(storage.storage_id, true)}
                                >
                                  Reconnect {storage.provider_label}
                                </button>
                              ) : isIrrecoverableSyncError(failedMessage) ? (
                                <button
                                  type="button"
                                  className="btn btn-ghost danger"
                                  disabled={noticeActionsDisabled}
                                  onClick={() => void run(
                                    `reset-sync-metadata:${project.local_project_id}`,
                                    () => onResetProjectMetadata(project.local_project_id),
                                  )}
                                >
                                  <Icon name="trash" size={13} />
                                  Reset local sync state
                                </button>
                              ) : (
                                <button
                                  type="button"
                                  className="btn btn-ghost"
                                  disabled={noticeActionsDisabled}
                                  onClick={() => void run(
                                    `${failedAction}:${actionPrefix}`,
                                    () => (failedAction === "push" ? onPush : onPull)(project.local_project_id, storage.storage_id),
                                  )}
                                >
                                  Retry
                                </button>
                              )}
                            </div>
                          ),
                        });
                      }

                      return (
                        <div
                          key={storage.storage_id}
                          className={`storage-link-block selected${reviewOpen ? " sync-review-open" : ""}`}
                          aria-busy={reviewOpen && busy}
                        >
                          {activeSibling && (
                            <SiblingRepoNotice
                              status={activeSibling}
                              busy={noticeActionsDisabled}
                              onConnect={(matches) => onConnectSibling?.(
                                project.local_project_id,
                                storage.storage_id,
                                matches,
                              )}
                            />
                          )}
                          {activeNotice && activeNotice.kind === "outcome" && (
                            <SyncOutcomeNotice
                              outcome={activeNotice.outcome}
                              busy={noticeActionsDisabled}
                              onAction={(action) => onStorageNoticeAction?.(activeNotice.outcome, action)}
                              onDismiss={() => onDismissStorageNotice?.()}
                            />
                          )}
                          <NoticeZone
                            items={gateNotices}
                            subject="reasons this storage is held"
                            className="sync-storage-notice"
                            timing="blocking"
                          />
                          {reviewOpen && (
                            <div id={reviewPanelId} className="sync-storage-inline-review" tabIndex={-1}>
                              {inlineStorageReview?.content}
                            </div>
                          )}
                        </div>
                      );
                    })()}
                    </div>
                    )}

                    {linkingProjectId === project.local_project_id && (
                      <div
                        id={`storage-link-picker-${project.local_project_id}`}
                        className="project-storage-card-footer"
                      >
                        <div className="storage-link-picker-heading">
                          <span>Choose a storage destination</span>
                          <button
                            type="button"
                            className="sync-history-icon-action"
                            disabled={busy}
                            title="Cancel linking storage"
                            aria-label="Cancel linking storage"
                            onClick={() => setLinkingProjectId(null)}
                          >
                            <Icon name="x" size={13} />
                          </button>
                        </div>
                        <div className="storage-link-picker">
                          {availableStorages.map((storage) => (
                            <button
                              key={storage.storage_id}
                              type="button"
                              disabled={busy}
                              onClick={() => void run(`link:${project.local_project_id}:${storage.storage_id}`, async () => {
                                await onLinkStorage(project.local_project_id, storage.storage_id);
                                setLinkingProjectId(null);
                              })}
                            >
                              <Icon name={iconNameFromHint(storage.icon, "drive")} size={15} />
                              {storage.name || "(unnamed)"}
                              <Icon name="chevron-right" size={13} />
                            </button>
                          ))}
                        </div>
                      </div>
                    )}
                    </section>
                  </div>

                </article>
              );
            })}
          </div>
        ) : (
          <div className="sync-storage-index-list" aria-label="Projects">
            {projects.map((project) => {
              const projectBinding = bindingByProject.get(project.local_project_id);
              const profileMissing = Object.keys(projectBinding?.profile_ids ?? {}).length === 0;
              return (
                <button
                  key={project.local_project_id}
                  type="button"
                  className="sync-storage-index-row"
                  disabled={busy}
                  onClick={() => onSelectProject(project.local_project_id)}
                  aria-label={`Open ${projectLabel(project)}`}
                >
                  <span className="sync-storage-repository-icon">
                    <Icon name={project.is_git_repository ? "git-folder" : "folder"} size={14} />
                  </span>
                  <span className="sync-storage-index-copy">
                    <strong>{projectLabel(project)}</strong>
                    <small>{compactProjectPath(project.project_root)}</small>
                  </span>
                  {profileMissing && (
                    <span className="sync-storage-index-warning">
                      <Icon name="alert-triangle" size={12} /> Agent profile needed
                    </span>
                  )}
                  <Icon name="chevron-right" size={12} className="sync-storage-index-open" aria-hidden="true" />
                </button>
              );
            })}
          </div>
        ))}

        {activeProject && !newProjectSetup && !inlineStorageReview && (
          <div id="project-history-panel" className="sync-project-tab-panel">
            {activityView === "files" && !activeProject.is_git_repository ? (
              <ProjectFilesActivityPage
                project={activeProject}
                activeStorageId={activeStorageId}
                activeStorageName={storages.find((storage) => storage.storage_id === activeStorageId)?.name ?? null}
                refreshEpoch={historyRefreshEpoch}
                onHeaderModelChange={handleFilesActivityHeaderChange}
                onOpenFile={(relativePath) => openProjectFile(activeProject.local_project_id, relativePath)}
                onRevealPath={revealPath}
              />
            ) : (
              <ProjectActivityPage
                project={activeProject}
                binding={activeBindingByProject.get(activeProject.local_project_id) ?? null}
                refreshEpoch={historyRefreshEpoch}
                activeStorageId={activeStorageId}
                activeStorageName={storages.find((storage) => storage.storage_id === activeStorageId)?.name ?? null}
                onUnlinkStorage={activeStorageId ? () => {
                  void run(
                    `unlink:${activeProject.local_project_id}:${activeStorageId}`,
                    () => onUnlinkStorage(activeProject.local_project_id, activeStorageId),
                  );
                } : undefined}
                continuityActionBusy={busy || !!runningAction}
                onHeaderModelChange={handleActivityHeaderChange}
                onChooseAgentProfile={() => {
                  void onConfigureBinding(activeProject.local_project_id);
                }}
                onOpenSessionDetail={(request, trigger) => (
                  openSessionDetail(activeProject.local_project_id, request, trigger)
                )}
              />
            )}
          </div>
        )}
      </section>

      {sessionDetailSubview && workspaceProject && (
        <Suspense fallback={(
          /* Not role="status": the region and its sentence mount in the same
             commit, so there is no change for a live region to announce — the
             text is simply what this view renders with while the chunk
             loads. */
          <section className="session-detail-page session-detail-entry-state">
            <span className="status-loader" />
            <strong>Opening session detail…</strong>
          </section>
        )}>
          <SessionDetailPage
            localProjectId={sessionDetailSubview.localProjectId}
            projectRoot={activeBindingByProject.get(sessionDetailSubview.localProjectId)?.canonical_project_root
              ?? workspaceProject.canonical_project_root
              ?? workspaceProject.project_root
              ?? ""}
            resourceId={sessionDetailSubview.resourceId}
            occurrenceKey={sessionDetailSubview.occurrenceKey}
            onHeaderChange={handleSessionHeaderChange}
            onOpenSession={(resourceId) => onOpenSessionDetail(sessionDetailSubview.localProjectId, {
              resourceId,
              occurrenceKey: resourceId,
            })}
          />
        </Suspense>
      )}

    </main>
    </>
  );
}
