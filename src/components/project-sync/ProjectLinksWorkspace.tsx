import { projectOutcomeAction } from "./syncOutcome";
import type { ManagedCloudPublicationStatus, ManagedCloudStatus } from "../managed-cloud/types";
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
  ProjectContentInventory,
  ProjectStorageLink,
  ProviderProfileSummary,
  PullScope,
  PushScope,
  RemoteBundleSummary,
  SiblingBundleStatus,
  StorageProviderDescriptor,
  StorageSetupDraft,
  StorageSummary,
  StorageProviderKey,
  ThreadSyncComparison,
} from "../../types";
import Icon, { storageIconName } from "../Icons";
import { NoticeChip, NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { formatDateTime, formatRelativeTime, toEpochMilliseconds } from "../../clock";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import ProjectActivityPage, { BranchPicker, type ActivityHeaderModel, type PushSessionFacts } from "./ProjectActivityPage";
import { ProjectFilesActivityView, projectFileSyncCounts, useProjectFilesScan } from "./ProjectFilesActivityPage";
import ProjectViewSwitch, { PROJECT_VIEW_PANEL_ID, projectViewTabId, type ProjectView } from "./ProjectViewSwitch";
import type {
  ActiveProjectSubview,
  OpenSessionDetailRequest,
} from "./session-detail/sessionDetailModel";
import { sessionPushDisabledReason, sessionPushGate } from "./session-detail/sessionDetailModel";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import ProviderMark from "../sync-review/ProviderMark";
import ProjectPicker from "./ProjectPicker";
import SessionDetailBack, { type SessionBackTarget } from "./session-detail/SessionDetailBack";
import type { SessionHeaderModel } from "./session-detail/SessionDetailPage";
import type { PullRequiredCopy } from "./PushResourceWorkspace";
import SyncOutcomeNotice from "../sync-review/SyncOutcomeNotice";
import SiblingRepoNotice from "./SiblingRepoNotice";
import StorageLinkDialog from "./StorageLinkDialog";
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
  StorageLockedForm,
  StorageReconnectBand,
  settingStrings,
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
import { R2SetupLane, useR2Setup, type R2SetupMode } from "./R2Setup";
import { DropboxSetupLane, useDropboxSetup } from "./DropboxSetup";
import { FeishuSetupLane, useFeishuSetup } from "./FeishuSetup";
import { ManagedCloudSetupLane, useManagedCloudSetup } from "./ManagedCloudSetup";
import { canPublishVersion, cloudRepositoryGone, cloudStatusNotice, cloudSyncBlocked, cloudVerbs, discardCloudDraft } from "../managed-cloud/managedCloudLink";
import ManagedCloudChip from "../managed-cloud/ManagedCloudChip";
import PublishVersionSheet, { type PublishVersionFact } from "../managed-cloud/PublishVersionSheet";
import ManagedCloudAddress from "../managed-cloud/ManagedCloudAddress";
import { isManagedCloudStorage, managedCloudAccount } from "./RepositoryAddressPicker";
import { openExternalUrl } from "./externalLink";
import type { StorageSetupPort } from "./StorageSetupStep";
import {
  compactProjectPath,
  projectLabel,
  providerLabel,
  type RenameResult,
} from "./model";

type LinkKey = { projectId: string; storageId: string };
export type StorageReviewKind = "pull" | "push";
type StorageEditorRequest =
  | { mode: "open"; storageId: string; requestId: number }
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
  /** The verb the review's primary wears; the card button shows the same. */
  continueLabel?: string | null;
  /** The review is still waiting on its rows; the bar button spins with the
   *  review's own primary. */
  continuePending?: boolean;
  /** Not the review yet: its frame while it is prepared. `busy` then means
   *  preparing, never publishing or applying. */
  preparing?: boolean;
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
  | { kind: "action_failed"; action: "push"; scope: PushScope; session?: PushSessionFacts; message: string }
  /** `scope`: what the Pull was of, so Retry asks for the same. */
  | { kind: "action_failed"; action: "pull"; scope: PullScope; message: string }
  /** A cleared storage this computer synced with before: nothing to pull. */
  | { kind: "nothing_to_pull"; message: string }
  /** A finished Push or Pull; the flow has already closed behind it. */
  | { kind: "outcome"; outcome: SyncOutcome }
);

interface Props {
  projects: LocalProjectSummary[];
  activeProjectId: string | null;
  activeStorageId: string | null;
  /** The storage each project last opened on, for the Projects landing. */
  rememberedStorageByProject: Record<string, string>;
  activeProjectSubview: ActiveProjectSubview;
  bindings: ProjectBinding[];
  profiles: ProviderProfileSummary[];
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
  /** A failed session action in the Activity list, or null to clear it;
      the shell draws it under the path bar. */
  onSessionActionError: (message: string | null) => void;
  /** A finished action's one receipt; the shell draws it under the path bar. */
  onNotify: (message: string) => void;
  onCloseSessionDetail: () => void;
  onLinkStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onUnlinkStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onPush: (projectId: string, storageId: string, scope: PushScope, session?: PushSessionFacts) => Promise<void> | void;
  onPull: (projectId: string, storageId: string, scope?: PullScope) => Promise<void> | void;
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
  onDeleteProjectSessions: (projectId: string) => Promise<void> | void;
  onRefresh: () => void;
  onAddProject: () => void;
  /** Add project with Mallard Cloud already chosen as the source. */
  onAddCloudProject: () => void;
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
  /** Open a storage's page: its settings card over its repositories. */
  onOpenStorage: (storageId: string) => void;
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
  /** Probe a settled storage as it is saved, without a draft. */
  onTestStorage: (storageId: string) => Promise<boolean>;
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
  /** How many project files of the open non-Git project are still only in
   *  its storage: a session's page says so, since a session comes alone. */
  onProjectFilesToPullChange?: (count: number) => void;
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
          <span className="sync-storage-repo-chip"><span>Version</span><b>{bundle.revision ?? "—"}</b></span>
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
            <span className="sync-storage-repo-chip"><span>Version</span><b>{repo.revision ?? "—"}</b></span>
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

/** What a Pull or Push badge stands for, in the units the verb moves: the
 *  comparison's threads and, for a non-Git project, its scanned files. The
 *  badge is decorative, so the button's name has to say this. */
export function projectSyncCountPhrase(threads: number, files: number): string {
  const phrases = [
    threads > 0 ? `${threads} newer thread${threads === 1 ? "" : "s"}` : null,
    files > 0 ? `${files} changed file${files === 1 ? "" : "s"}` : null,
  ].filter((phrase): phrase is string => phrase !== null);
  return phrases.join(" and ");
}

/** What the Publish sheet says the version holds: the draft as the pair's
 *  comparison and file scan read it, and how far it is past the latest
 *  version. A read that is missing leaves its fact out. */
export function publishVersionHolds(
  comparison: ThreadSyncComparison | null | undefined,
  inventory: ProjectContentInventory | null | undefined,
  publication: ManagedCloudPublicationStatus | null | undefined,
): PublishVersionFact[] {
  const count = (total: number, one: string, many: string) => `${total} ${total === 1 ? one : many}`;
  const facts: PublishVersionFact[] = [];
  if (comparison) {
    facts.push({ icon: "message", word: count(comparison.entries.filter((entry) => entry.storage_present).length, "session", "sessions") });
  }
  if (inventory?.eligibility.state === "eligible") {
    const files = inventory.entries.filter((entry) => entry.entry_type === "file" && entry.storage_present).length;
    facts.push({ icon: "file", word: count(files, "file", "files") });
  }
  if (publication?.latest_version != null && publication.draft_ahead) {
    facts.push({ icon: "upload", word: `${count(publication.draft_ahead, "push", "pushes")} since version ${publication.latest_version}` });
  }
  return facts;
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
  rememberedStorageByProject,
  activeProjectSubview,
  bindings,
  profiles,
  storageProviders,
  storages,
  storageSetupDrafts,
  links,
  loading,
  busy,
  fetchProgress,
  onSelectStorage,
  onOpenSessionDetail,
  onSessionActionError,
  onNotify,
  onCloseSessionDetail,
  onLinkStorage,
  onUnlinkStorage,
  onPush,
  onPull,
  siblingNotice,
  onConnectSibling,
  onConfigureBinding,
  onRenameProject,
  onDeleteProjectSessions,
  onRefresh,
  onAddProject,
  onAddCloudProject,
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
  onTestStorage,
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
  onProjectFilesToPullChange,
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
    feishu.dismiss();
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
  /* When this visit last saw the settled storage's connection verified; a
     probe that succeeds forgets the transport's recorded health, so the
     page keeps its own receipt. */
  const [storageVerifiedAt, setStorageVerifiedAt] = useState<number | null>(null);
  /* Bumped by ⋯ → Rename storage… to put the caret in the name field. */
  const [nameFocusSeq, setNameFocusSeq] = useState(0);
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
  const [activityView, setActivityView] = useState<ProjectView>("sessions");
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
    setStorageVerifiedAt(null);
    setNameFocusSeq(0);
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
  const setupPort: StorageSetupPort = {
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
          ? storageNameForSettings(descriptor, formFromDraft(descriptor, saved), settingStrings(prepared.settings), storages.map((storage) => storage.name))
          : null;
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
  const feishu = useFeishuSetup({
    enabled: !!storageSession && findStorageProvider(storageProviders, storageSession.draft.provider, storageSession.draft.config_version)?.local_setup === "feishu",
    draftId: storageSession?.draft.draft_id ?? null,
    mode: r2Mode,
    port: setupPort,
  });
  const managedCloud = useManagedCloudSetup({
    enabled: !!storageSession && findStorageProvider(storageProviders, storageSession.draft.provider, storageSession.draft.config_version)?.local_setup === "managed_cloud",
    draftId: storageSession?.draft.draft_id ?? null,
    mode: r2Mode === "reconnect" ? "reconnect" : "create",
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
    if (r2.isRunning() || dropbox.isRunning() || feishu.isRunning() || managedCloud.isRunning()) return;
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
    if (r2.isRunning() || dropbox.isRunning() || feishu.isRunning() || managedCloud.isRunning()) return false;
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

  const openStorageDetails = async (storage: StorageSummary) => {
    const switchingStorage = editingStorage?.storageId !== storage.storage_id;
    if (switchingStorage) {
      if (!(await saveStorageDraftBeforeNavigation())) return;
      clearStorageListing();
      setEditingStorage({ projectId: "", storageId: storage.storage_id });
    }
    if (switchingStorage) await loadStorageBundles(storage);
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
      return;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editingStorage?.storageId, storages]);

  const activeProject = projects.find((project) => project.local_project_id === activeProjectId) ?? null;
  const workspaceProject = newProjectSetup ? null : activeProject;
  const activeLocalProjectId = activeProject?.local_project_id ?? null;
  useEffect(() => { setActivityView("sessions"); }, [activeLocalProjectId]);
  /* A non-Git project's files travel with its Pull and Push, so their scan
     feeds the bar's badges on either face, and the Files face renders the
     same scan. The scan holds the host-operation lock a review needs, so it
     stands down while one is open and runs again once it closes. */
  const projectFilesScan = useProjectFilesScan(
    activeProject && !activeProject.is_git_repository && !newProjectSetup && !inlineStorageReview
      ? activeProject.local_project_id
      : null,
    activeStorageId,
    historyRefreshEpoch,
  );
  const projectFileCounts = useMemo(
    () => projectFileSyncCounts(projectFilesScan.inventory),
    [projectFilesScan.inventory],
  );
  useEffect(() => {
    onProjectFilesToPullChange?.(projectFileCounts.pull);
  }, [onProjectFilesToPullChange, projectFileCounts.pull]);
  const filesFaceShown = activityView === "files" && !!activeProject && !activeProject.is_git_repository;
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
  /* The website bridge for a project on Mallard Cloud: the repository's page
     and its release state, read from the service when the pair opens, when
     the window comes back, and after every action ends. */
  const managedCloudRepositoryId = workspaceLinks.find((link) => link.storage_id === workspaceStorage?.storage_id)?.managed_cloud_repository_id;
  const managedCloudKey = workspaceStorage?.provider === "managed_cloud" && managedCloudRepositoryId ? workspaceActionPrefix : null;
  const managedCloudProjectId = managedCloudKey ? workspaceProject?.local_project_id ?? null : null;
  const managedCloudStorageId = managedCloudKey ? workspaceStorage?.storage_id ?? null : null;
  const [cloudObservation, setCloudObservation] = useState<{ key: string; status: ManagedCloudStatus } | null>(null);
  const [cloudActionError, setCloudActionError] = useState<{ key: string; message: string } | null>(null);
  useEffect(() => {
    if (managedCloudKey && activityHeader?.projectId === managedCloudProjectId && activityHeader?.storageId === managedCloudStorageId && activityHeader.managedCloudStatus && activityHeader.managedCloudStatus.repository_id === managedCloudRepositoryId) {
      setCloudObservation({ key: managedCloudKey, status: activityHeader.managedCloudStatus });
    }
  }, [managedCloudKey, managedCloudProjectId, managedCloudStorageId, activityHeader?.projectId, activityHeader?.storageId, activityHeader?.managedCloudStatus, managedCloudRepositoryId]);
  const cloudStatus: ManagedCloudStatus | null = managedCloudKey && activityHeader?.projectId === managedCloudProjectId && activityHeader?.storageId === managedCloudStorageId && activityHeader.comparisonError
    /* A comparison that failed closes Push and Pull whatever the repository
       answered: only a comparison that succeeded reads a gone one as empty. */
    ? { repository_id: managedCloudRepositoryId!, state: !activityHeader.managedCloudStatus || activityHeader.managedCloudStatus.state === "ready" || cloudRepositoryGone(activityHeader.managedCloudStatus) ? "unavailable" : activityHeader.managedCloudStatus.state, link: null, message: activityHeader.comparisonError }
    : managedCloudKey && cloudObservation?.key === managedCloudKey && cloudObservation.status.repository_id === managedCloudRepositoryId ? cloudObservation.status : null;
  const currentManagedCloudLink = cloudStatus?.state === "ready" ? cloudStatus.link : null;
  /* "Checking…" only while this pair's read is in flight. A comparison may
     come back without a status (the store had none to give), and then the
     bar shows no chip rather than a check that never ends. */
  const cloudChecking = !cloudStatus
    && !!managedCloudKey
    && activityHeader?.projectId === managedCloudProjectId
    && activityHeader?.storageId === managedCloudStorageId
    && !!activityHeader.refreshing;
  const cloudBlocksSync = !!managedCloudKey && cloudSyncBlocked(cloudStatus);
  const cloudRefreshRef = useRef(activityHeader?.onRefresh);
  cloudRefreshRef.current = activityHeader?.projectId === managedCloudProjectId && activityHeader?.storageId === managedCloudStorageId ? activityHeader.onRefresh : undefined;
  useEffect(() => {
    if (!managedCloudKey || runningAction !== null) return;
    const refresh = () => cloudRefreshRef.current?.();
    // Initial selection is loaded by the activity page. Focus and completed actions reuse that read.
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [managedCloudKey, runningAction]);
  /* The repository's verbs live in the Cloud chip's panel, beside the state
     they act on (docs/PLAN_CLOUD_STATUS_CHIP.md). The panel decides which it
     offers from the status (cloudVerbs); these are what Publish version and
     the discard do. Both name the repository by the link, so they wait while
     another repository is read in place of the link's. */
  const [publishSheetKey, setPublishSheetKey] = useState<string | null>(null);
  /* The sheet is this pair's, and open only while Publish is: the number is
     the one the status gives. A status that stops offering Publish lets the
     sheet go, so it never comes back by itself. */
  const publishVersionNumber = publishSheetKey !== null && publishSheetKey === managedCloudKey && canPublishVersion(cloudStatus)
    ? cloudVerbs(cloudStatus).publishVersion
    : null;
  useEffect(() => {
    if (publishSheetKey !== null && publishVersionNumber === null) setPublishSheetKey(null);
  }, [publishSheetKey, publishVersionNumber]);
  const cloudActivity = activityHeader?.projectId === managedCloudProjectId && activityHeader?.storageId === managedCloudStorageId
    ? activityHeader
    : null;
  const cloudFileScan = projectFilesScan.inventory?.local_project_id === managedCloudProjectId && projectFilesScan.inventory?.storage_id === managedCloudStorageId
    ? projectFilesScan.inventory
    : null;
  const publishCloudVersion = (name: string | null) => {
    if (!managedCloudKey || !managedCloudProjectId || !managedCloudStorageId || !canPublishVersion(cloudStatus)) return;
    const linkKey = managedCloudKey;
    const projectId = managedCloudProjectId;
    const storageId = managedCloudStorageId;
    void run(`publish:${linkKey}`, async () => {
      try {
        projectSyncApi.invalidateProjectActivity(projectId, storageId);
        const published = await projectSyncApi.publishManagedCloudVersion(projectId, storageId, name);
        setCloudActionError(null);
        onNotify(`Published version ${published.version}.`);
      } catch (reason) {
        // The sheet closes itself once answered; a refusal is read on the page.
        setCloudActionError({ key: linkKey, message: reason instanceof Error ? reason.message : String(reason) });
      }
      cloudRefreshRef.current?.();
    });
  };
  const discardCloudDraftNow = () => {
    if (!managedCloudKey || !managedCloudProjectId || !managedCloudStorageId) return;
    const linkKey = managedCloudKey;
    const projectId = managedCloudProjectId;
    const storageId = managedCloudStorageId;
    void run(`discard:${linkKey}`, () => discardCloudDraft(cloudStatus, {
      confirm,
      discard: (expectedRepositoryRevision) => {
        projectSyncApi.invalidateProjectActivity(projectId, storageId);
        return projectSyncApi.discardManagedCloudDraft(projectId, storageId, expectedRepositoryRevision);
      },
      report: (refusal) => setCloudActionError(refusal === null ? null : { key: linkKey, message: refusal }),
      refresh: () => cloudRefreshRef.current?.(),
    }));
  };
  const workspaceReviewOpen = !!workspaceActionPrefix
    && inlineStorageReview?.projectId === workspaceProject?.local_project_id
    && inlineStorageReview?.storageId === workspaceStorage?.storage_id;
  const workspaceReviewKind = workspaceReviewOpen ? inlineStorageReview?.kind ?? null : null;
  const workspacePushReviewOpen = workspaceReviewOpen && inlineStorageReview?.kind === "push";
  const workspacePullReviewOpen = workspaceReviewOpen && inlineStorageReview?.kind === "pull";
  const workspacePreparingReview = workspaceReviewOpen && !!inlineStorageReview?.preparing;
  const workspacePushPreparing = runningAction === `push:${workspaceActionPrefix}`;
  const workspacePushPublishing = workspacePushReviewOpen && busy && !workspacePreparingReview;
  const workspacePullPreparing = runningAction === `pull:${workspaceActionPrefix}`;
  const workspacePullApplying = workspacePullReviewOpen && busy && !workspacePreparingReview;
  const workspacePullProgress = workspacePullPreparing || workspacePullApplying;
  const workspacePushProgress = workspacePushPreparing || workspacePushPublishing;
  /* A review still waiting on its rows is progress too: the bar's button
     spins with the review's own primary, under the same label. */
  const workspacePullSpinning = workspacePullProgress
    || (workspacePullReviewOpen && !!inlineStorageReview?.continuePending);
  const workspacePushSpinning = workspacePushProgress
    || (workspacePushReviewOpen && !!inlineStorageReview?.continuePending);
  /* The prepare's own progress is the card's caption, in words; the bar
     carries a count only for the transfer the user asked for. */
  const workspacePushReadout = workspacePushPublishing && fetchProgress?.phase === "push"
    ? `${fetchProgress.index}/${fetchProgress.total_objects} · ${(fetchProgress.fetched_bytes / 1_000_000).toFixed(1)} MB…`
    : null;
  const workspaceControlsLocked = busy
    || !!runningAction
    || storageActionLockedForReview(workspaceReviewKind, "storage");

  // Name the destination when returning from a session to its project.
  const workspaceSectionLabel = workspaceProject?.is_git_repository
    ? "Git history"
    : activityView === "files" ? "Files" : "Sessions";
  // One switch for a non-Git project's two faces. Each face seats it at the
  // start of its own search row, so the trail ends at the project and the
  // choice sits on the page it changes.
  const projectViewSwitch = workspaceProject && !workspaceProject.is_git_repository
    ? <ProjectViewSwitch view={activityView} onChange={setActivityView} />
    : undefined;
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
    const storageLinkCount = links.filter((link) => link.storage_id === pageStorageId).length;
    const pathFact = savedEditedStorage?.summary.find((field) => field.is_path) ?? null;
    /* A Mallard Cloud storage leads its card with the website it lives on,
       and the ⋯ menu opens it, as a folder's Reveal in Finder does. */
    const cloudAccount = savedEditedStorage && isManagedCloudStorage(savedEditedStorage)
      ? managedCloudAccount(savedEditedStorage)
      : null;
    const formLocked = busy || !!runningAction || r2.state.running || dropbox.state.running || feishu.state.running;
    const storageActionBlocked = formLocked || !storageReady || destinationConflict !== null;

    /* Creation's Finish setup, and a reconnect's Save credential: the same
       verified commit of the exact draft the user reviewed. */
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
      // the settled storage; a reconnect leaves the credential verified.
      const reconnected = !creatingStorage;
      resetStorageSession();
      setStorageVerifiedAt(reconnected ? Date.now() : null);
      onOpenStorage(result.storage_id);
    });

    /* The settled storage's own probe: no draft, the saved configuration as
       it is. Success is this visit's receipt; refusal reaches the shell's
       error zone like every other action. */
    const testSavedStorage = () => run(`test-storage:${pageStorageId}`, async () => {
      setStorageVerifiedAt(null);
      const verified = await onTestStorage(pageStorageId);
      if (verified) setStorageVerifiedAt(Date.now());
    });

    /* Reconnect opens the storage's edit draft — the one draft a settled
       storage still has, and the backend holds its settings and name fixed —
       so the band under the guide row can take a replacement credential. */
    const beginReconnect = () => {
      if (savedEditedStorage) void ensureStorageEditSession(savedEditedStorage);
    };

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
      const wasNew = !session.draft.edits_existing;
      // A reconnect's Cancel is its own answer; a setup draft is asked, as
      // it may hold an afternoon of setup.
      if (wasNew) {
        const approved = await confirm(
          `Discard the storage draft for ${session.form.name || "(unnamed)"}?\n\nThis removes only Mallard’s local setup draft; external storage resources are not changed.`,
          { title: "Discard storage draft" },
        );
        if (!approved) return;
      }
      try {
        await storageDraftSaveQueue.enqueue(
          () => projectSyncApi.discardStorageSetupDraft(session.draft.draft_id),
        );
      } catch (reason) {
        setStorageDraftActionError(`Mallard could not discard the storage draft: ${errorMessage(reason)}`);
        return;
      }
      resetStorageSession();
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
      onStorageDraftsChanged?.();
      onOpenStorage(storage.storage_id);
    });

    /* The header's type switch restarts a new draft on another provider:
       the current draft goes, a typed name comes along, a default one is
       re-minted for the new type. */
    const switchStorageProvider = (provider: StorageProviderDescriptor) => run(`switch-provider:${pageStorageId}`, async () => {
      const session = storageSessionRef.current;
      if (!session || !descriptor) return;
      if (provider.key === descriptor.key && provider.config_version === descriptor.config_version) return;
      const keptName = storageNameIsAuto(descriptor, session.form, settingStrings(session.draft.settings)) ? undefined : session.form.name;
      try {
        await storageDraftSaveQueue.enqueue(
          () => projectSyncApi.discardStorageSetupDraft(session.draft.draft_id),
        );
      } catch (reason) {
        setStorageDraftActionError(`Mallard could not discard the storage draft: ${errorMessage(reason)}`);
        return;
      }
      resetStorageSession();
      onStorageDraftsChanged?.();
      await onCreateStorageDraft(provider.key, keptName);
    });

    /* The New-storage doors. Create new is today's card; Use existing is a
       read-only look at the account for the draft's provider, then one row
       chosen and prepared. Taking a door drops whatever the other found. */
    const chooseSetupMode = (mode: StorageSetupMode) => {
      if (mode === storageSetupMode) return;
      if (formLocked || (r2.isRunning() || dropbox.isRunning() || feishu.isRunning())) return;
      r2.dismiss();
      dropbox.dismiss();
      feishu.dismiss();
      setStorageSetupMode(mode);
      setManualEntry(false);
    };
    /* The doors are asked of a blank new draft and stay as a switch once
       taken; a draft that already holds values — resumed, or prepared by
       the agent or the look — is past the question. */
    const r2Ready = (r2.state.phase === "review" || dropbox.state.phase === "review" || feishu.state.phase === "review");
    const askingSetupMode = creatingStorage && !!descriptor?.setup_doors
      && (storageSetupMode !== null || (progress?.set ?? 0) === 0) && !r2Ready;
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
              icon: storageIconName(savedEditedStorage.provider, savedEditedStorage.icon),
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
                    icon: storageIconName(candidate.provider, candidate.icon),
                    disabled: busy,
                    onSelect: () => onOpenStorage(candidate.storage_id),
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
          ...(cloudAccount ? [{
            id: "open-website",
            label: "Open website",
            detail: cloudAccount.host,
            icon: "external-link" as const,
            onSelect: () => void openExternalUrl(cloudAccount.origin),
          }] : []),
          {
            id: "rename",
            label: "Rename storage…",
            icon: "edit" as const,
            disabled: busy,
            // The name is the settings card's one live field.
            onSelect: () => setNameFocusSeq((seq) => seq + 1),
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
        // The settings card's guide row carries Reconnect…; the shared
        // banner states the refusal.
        message: connectionDetail(storageConnection),
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
      if (formLocked || r2.isRunning() || dropbox.isRunning() || feishu.isRunning() || managedCloud.isRunning() || !descriptor) return;
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
        onStorageDraftsChanged?.();
        await onCreateStorageDraft(descriptor.key, descriptor.default_name ?? undefined);
      });
    };
    /* Every edit to an open draft — a setup form or a reconnect band —
       invalidates the current setup result, stales a Test verdict, and arms
       the draft autosave. */
    const onStorageFormChange = (nextForm: StorageEditorForm) => {
      draftEditVersionRef.current += 1;
      r2.dismiss();
      dropbox.dismiss();
      feishu.dismiss();
      managedCloud.dismiss();
      setStorageTestVerified(false);
      setStorageDraftActionError(null);
      draftDirtyRef.current = true;
      updateStorageSession((current) => ({ ...current, form: nextForm }));
    };
    const setupSlot = storageSession && descriptor?.local_setup === "wrangler"
      ? <R2SetupLane
          mode={r2Mode}
          flow={r2}
          disabled={formLocked}
          descriptor={descriptor}
          storages={storages}
          bucketName={storageSession.form.settings.bucket}
          onOpenStorage={(storageId) => onOpenStorage(storageId)}
          onCreateInstead={() => chooseSetupMode("create")}
          onReset={resetR2Setup}
        />
      : storageSession && descriptor?.local_setup === "dropbox"
        ? <DropboxSetupLane flow={dropbox} mode={r2Mode} disabled={formLocked} descriptor={descriptor} storages={storages}
            onOpenStorage={(id) => onOpenStorage(id)} onCreateInstead={() => chooseSetupMode("create")}
            onNewSetup={() => void run("new-dropbox-account", async () => {
              // Preserve the original draft and its journal for recovery.
              if (!(await saveStorageDraftBeforeNavigation())) return;
              resetStorageSession();
              await onCreateStorageDraft(descriptor.key);
              onStorageDraftsChanged?.();
            })}
            onReset={resetR2Setup} />
      : storageSession && descriptor?.local_setup === "feishu"
        ? <FeishuSetupLane flow={feishu} mode={r2Mode} disabled={formLocked} descriptor={descriptor}
            onReset={resetR2Setup} />
      : storageSession && descriptor?.local_setup === "managed_cloud"
        ? <ManagedCloudSetupLane flow={managedCloud} mode={r2Mode === "reconnect" ? "reconnect" : "create"} disabled={formLocked}
            storages={storages} onOpenStorage={(id) => onOpenStorage(id)} onReset={resetR2Setup} />
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
        notices={{
          items: storageNotices.map((item) => ({ ...item, key: `storage:${pageStorageId}:${item.key}` })),
          ...storageNoticeTiming,
        }}
      />
      <main className="sync-main sync-project-links-page sync-storage-settings-page">
        <section
          className="profile-links-section"
          aria-label={creatingStorage ? "New storage" : storageName}
        >
          <div
            ref={storageSettingsRef}
            className="sync-storage-settings-below sync-storage-settings-dedicated"
          >
            <div className="sync-storage-details-panel">
              {creatingStorage && storageSession && descriptor && (
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
                  destinationConflict={destinationConflict
                    ? { storage: destinationConflict, onOpen: () => void openConflictingStorage(destinationConflict) }
                    : null}
                  setupMode={askingSetupMode ? { mode: storageSetupMode, onChoose: chooseSetupMode } : undefined}
                  manualEntry={manualEntry}
                  setupDone={r2Ready}
                  takenNames={storages.map((storage) => storage.name)}
                  onSwitchProvider={switchStorageProvider}
                  onChange={onStorageFormChange}
                  setupSlot={setupSlot}
                />
              )}
              {!creatingStorage && savedEditedStorage && descriptor && (
                <StorageLockedForm
                  key={pageStorageId}
                  storage={savedEditedStorage}
                  descriptor={descriptor}
                  disabled={busy || !!runningAction}
                  onRename={(name) => onRenameStorage(pageStorageId, name)}
                  focusNameSeq={nameFocusSeq}
                  onRevealSecret={(secretId) => projectSyncApi.revealStorageSecret(pageStorageId, secretId)}
                  onTest={() => void testSavedStorage()}
                  testing={runningAction === `test-storage:${pageStorageId}`}
                  verifiedAt={storageVerifiedAt}
                  reconnect={{
                    open: !!storageSession || storageSessionLoading,
                    onOpen: beginReconnect,
                    band: storageSession ? (
                      <StorageReconnectBand
                        key={storageSession.draft.draft_id}
                        descriptor={descriptor}
                        form={storageSession.form}
                        secretPresence={secretPresence}
                        disabled={formLocked}
                        connection={storageConnection}
                        onChange={onStorageFormChange}
                        setupSlot={setupSlot}
                        verbs={(
                          <>
                            {storageTestVerified && (
                              <span className="storage-setup-verified" role="status">
                                <Icon name="check-circle" size={13} /> Connection verified
                              </span>
                            )}
                            <button type="button" className="btn btn-ghost" disabled={formLocked} onClick={() => void discardStorageDraftAndClose()}>
                              Cancel
                            </button>
                            <button type="button" className="btn" disabled={storageActionBlocked} onClick={() => void testStorageConnection()}>
                              {runningAction === `test-storage:${pageStorageId}` ? "Testing…" : "Test connection"}
                            </button>
                            <button type="button" className="btn btn-primary" disabled={storageActionBlocked} onClick={() => void saveEditedStorage()}>
                              {runningAction === `storage:${pageStorageId}` ? "Saving…" : "Save credential"}
                            </button>
                          </>
                        )}
                      />
                    ) : (
                      <div className="sync-storage-repository-state"><span className="status-loader" /> Opening the reconnect…</div>
                    ),
                  }}
                  lead={cloudAccount ? {
                    band: <ManagedCloudAddress account={cloudAccount} onOpenUrl={(url) => void openExternalUrl(url)} />,
                    // The two facts managedCloudAccount reads.
                    facts: ["account_login", "service_origin"],
                  } : undefined}
                />
              )}
            </div>

            {creatingStorage && storageSession && (
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
          storages={storages}
          rememberedStorageByProject={rememberedStorageByProject}
          onAddProject={onAddProject}
          onAddCloudProject={onAddCloudProject}
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
              id: "storage-details",
              label: "Storage details…",
              icon: "settings" as const,
              disabled: workspaceControlsLocked,
              onSelect: () => { void openStorageDetails(workspaceStorage); },
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
            icon: storageIconName(storage.provider, storage.icon),
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
              // Opens the chooser. Not a toggle: the menu closes on select, so
              // a second visit to this row can only mean "open it again".
              onSelect: () => setLinkingProjectId(workspaceProject.local_project_id),
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
        icon: storageIconName(workspaceStorage.provider, workspaceStorage.icon),
        tone: "root",
        title: `${workspaceStorage.name || "Storage"} · ${storageSubtitle(workspaceStorage)}`,
        ariaLabel: `${workspaceStorage.name || "Unnamed storage"}. Storage menu`,
        menuHeader: {
          label: workspaceStorage.name || "(unnamed)",
          detail: storageSubtitle(workspaceStorage),
          detailIsPath: storageSubtitleIsPath(workspaceStorage),
          icon: storageIconName(workspaceStorage.provider, workspaceStorage.icon),
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
      scope: !sessionDetailSubview && workspaceProject.is_git_repository
        && activityHeader && activityHeader.branches.length > 0 ? (
          <>
            <span className="sync-path-separator" aria-hidden="true">/</span>
            <BranchPicker
              branches={activityHeader.branches}
              selectedBranch={activityHeader.selectedBranch}
              disabled={activityHeader.refreshing}
              onChange={activityHeader.onSelectBranch}
            />
          </>
        ) : undefined,
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
              id: "delete-project-sessions",
              label: "Delete all local sessions…",
              icon: "trash" as const,
              danger: true,
              disabled: busy,
              title: "Delete this project’s local sessions and subagents, and clear its session bindings and local sync state",
              onSelect: () => void onDeleteProjectSessions(workspaceProject.local_project_id),
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

    // An open session follows its project in the trail.
    if (sessionDetailSubview) {
      // An agent's page sits under the session that delegated to it; the
      // parent is the crumb before the title, and the way back.
      if (sessionHeader?.state === "ready" && sessionHeader.parent) {
        const parent = sessionHeader.parent;
        pathCrumbs.push({
          id: "session-parent",
          label: parent.title,
          title: `Back to ${parent.title}`,
          leading: (
            <span className="sync-path-leaf-mark">
              <ProviderMark
                provider={sessionHeader.provider}
                displayName={sessionHeader.providerDisplayName}
              />
            </span>
          ),
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
          ? [sessionHeader.title, sessionHeader.providerDisplayName]
            .filter(Boolean).join(" · ")
          : undefined,
        leading: sessionHeader?.state === "ready" ? (
          <span className="sync-path-leaf-mark">
            <ProviderMark
              provider={sessionHeader.provider}
              displayName={sessionHeader.providerDisplayName}
            />
          </span>
        ) : undefined,
      });
    }
  }

  // One home per scope: the storage crumb holds the storage verbs, the project
  // crumb holds the project verbs, and the ⋯ is left with exactly what has no
  // crumb of its own — Mallard itself.
  const pathOverflow: PathBarMenuGroup[] = workspaceProject ? appActions : [];

  // The session page's own back button walks one level up the trail, as the
  // crumbs do: an agent's page returns to the session that delegated to it,
  // and a session returns to the project's page, waiting where its crumb waits.
  const sessionParent = sessionHeader?.state === "ready" ? sessionHeader.parent ?? null : null;
  const sessionBack: SessionBackTarget | null = !sessionDetailSubview ? null
    : sessionParent
      ? {
        kind: "session",
        label: sessionParent.title,
        onActivate: () => onOpenSessionDetail(sessionDetailSubview.localProjectId, {
          resourceId: sessionParent.resourceId,
          occurrenceKey: sessionParent.resourceId,
        }),
      }
      : {
        kind: "section",
        label: workspaceSectionLabel,
        onActivate: closeSessionDetail,
        disabled: busy || !!inlineStorageReview,
      };

  // A badge on the button that clears it, rather than a tally beside it: "3 to
  // push" is what Push does, said once. The counts stand down while that
  // button is mid-review or mid-transfer, where its own label is the readout.
  const sessionPushEntry = sessionDetailSubview
    ? activityHeader?.comparison?.entries.find(
      (entry) => entry.conversation_resource_id === sessionDetailSubview.resourceId,
    )
    : undefined;
  // The comparison this session's Push reads, once it is current for this
  // project and storage.
  const sessionComparison = sessionDetailSubview
    && activityHeader
    && activityHeader.projectId === workspaceProject?.local_project_id
    && activityHeader.storageId === workspaceStorage?.storage_id
    && !activityHeader.refreshing
    ? activityHeader
    : null;
  const sessionPushIssue = !sessionDetailSubview ? null
    : !sessionComparison
      ? "Checking session…"
      : sessionComparison.comparisonError
        ?? sessionPushDisabledReason(sessionPushEntry);
  // What the page says under Push: only a hold the user clears. Up to date,
  // checking and ready are Push's own state, tooltip and count; a failed
  // comparison the Cloud chip already reports is not said twice.
  const sessionPushHold = sessionComparison
    ? sessionPushGate(sessionPushEntry, managedCloudKey ? null : sessionComparison.comparisonError)
    : null;
  const pullThreadCount = activityHeader?.counts.pull ?? 0;
  const pushThreadCount = sessionDetailSubview
    ? sessionPushIssue === null ? 1 : 0
    : activityHeader?.counts.push ?? 0;
  const pushFileCount = sessionDetailSubview ? 0 : projectFileCounts.push;
  const pullCount = pullThreadCount + projectFileCounts.pull;
  const pushCount = pushThreadCount + pushFileCount;
  // The file scan holds the host-operation lock both verbs take, so a click
  // during it would only be refused; they wait it out, and say why.
  const projectFilesScanIssue = projectFilesScan.loading ? "Scanning project files…" : null;
  const showPullCount = pullCount > 0 && !workspacePullReviewOpen && !workspacePullProgress;
  const showPushCount = pushCount > 0 && !workspacePushReviewOpen && !workspacePushProgress;
  const attention = activityHeader?.counts.attention ?? 0;

  // One unlabelled refresh, scoped to the view: the open session when there is
  // one, the activity comparison otherwise. Refresh both reads in session
  // detail so its Push eligibility is also current.
  const viewRefresh = sessionDetailSubview
    ? sessionHeader?.state === "ready" && {
      refreshing: sessionHeader.refreshing || !!activityHeader?.refreshing,
      onRefresh: () => { sessionHeader.onRefresh(); activityHeader?.onRefresh(); },
      label: sessionHeader.refreshing || activityHeader?.refreshing ? "Refreshing session" : "Refresh session",
    }
    : filesFaceShown
      ? {
        refreshing: projectFilesScan.loading,
        onRefresh: projectFilesScan.rescan,
        label: projectFilesScan.loading ? "Rescanning project files" : "Rescan project files",
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
          aria-label={activityHeader?.counts.detail}
        >
          <Icon name={activityHeader?.counts.severity === "danger" ? "octagon-x" : "alert-triangle"} size={13} />
          <b>{attention}</b>
        </span>
      )}
      {/* Every answer the service gives shows here, the ones the notice below
          also carries included; while the first answer is on its way it
          checks, inert. */}
      {managedCloudKey && (cloudStatus || cloudChecking) && (
        <ManagedCloudChip
          status={cloudStatus}
          storageName={workspaceStorage.name || "Mallard Cloud"}
          busy={busy || !!runningAction}
          refreshing={!!activityHeader?.refreshing}
          onOpenUrl={(url) => void openExternalUrl(url)}
          onPublishVersion={() => setPublishSheetKey(managedCloudKey)}
          onDiscardDraft={discardCloudDraftNow}
          onRetry={() => cloudRefreshRef.current?.()}
          onStorageSettings={() => {
            if (managedCloudStorageId) onOpenStorage(managedCloudStorageId);
          }}
        />
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
          || projectFilesScan.loading
          || workspaceStorageUnavailable
          || cloudBlocksSync
          || (currentManagedCloudLink !== null && !currentManagedCloudLink.can_pull)
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
          ?? projectFilesScanIssue
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
          ? `${inlineStorageReview?.continueLabel ?? "Pull selected changes"} — ${workspaceStorage.name || "storage"}`
          : showPullCount
            ? `Pull ${projectSyncCountPhrase(pullThreadCount, projectFileCounts.pull)} from ${workspaceStorage.name || "storage"}`
            : `Pull from ${workspaceStorage.name || "storage"}`}
        aria-expanded={workspacePullReviewOpen}
      >
        <Icon
          key={workspacePullSpinning ? "pull-progress" : "pull-idle"}
          name={workspacePullSpinning ? "refresh" : "download"}
          size={14}
          className={workspacePullSpinning ? "icon-spin" : undefined}
        />
        <span className="sync-path-sync-label">{
          workspacePullReviewOpen && !workspacePullProgress && inlineStorageReview?.continueLabel
            ? inlineStorageReview.continueLabel
            : projectPullActionLabel({ preparing: workspacePullPreparing, applying: workspacePullApplying })
        }</span>
        {showPullCount && <span className="sync-path-count" aria-hidden="true">{pullCount}</span>}
      </button>
      <button
        type="button"
        className={`sync-path-sync${workspacePushReviewOpen ? " active" : ""}`}
        disabled={busy
          || !!runningAction
          || projectFilesScan.loading
          || workspaceStorageUnavailable
          || cloudBlocksSync
          || (currentManagedCloudLink !== null && !currentManagedCloudLink.can_push)
          || !workspaceCanSync
          || (!workspacePushReviewOpen && sessionPushIssue !== null)
          || storageActionLockedForReview(workspaceReviewKind, "push")
          || (workspacePushReviewOpen && inlineStorageReview?.continueDisabled)}
        onClick={() => {
          if (workspacePushReviewOpen) {
            inlineStorageReview?.onContinue?.();
            return;
          }
          if (sessionDetailSubview && !sessionPushEntry) return;
          setLinkingProjectId(null);
          void run(
            `push:${workspaceActionPrefix}`,
            () => onPush(workspaceProject.local_project_id, workspaceStorage.storage_id,
              sessionDetailSubview && sessionPushEntry
                ? { kind: "session", resource_id: sessionPushEntry.resource_id }
                : { kind: "project" },
              sessionDetailSubview && sessionHeader?.state === "ready" ? sessionHeader.pushFacts : undefined),
          );
        }}
        title={workspaceStorageIssue
          ?? workspaceProfileIssue
          ?? projectFilesScanIssue
          ?? (workspacePushReviewOpen
            ? (inlineStorageReview?.continueTitle ?? "Push the selected resources to this storage")
            : sessionPushIssue ?? (sessionDetailSubview
              ? "Review and push this session to this storage"
              : "Choose resources, then push them to this storage"))}
        aria-label={workspacePushReviewOpen
          ? `Push selected resources to ${workspaceStorage.name || "storage"}`
          : showPushCount
            ? `Push ${projectSyncCountPhrase(pushThreadCount, pushFileCount)} to ${workspaceStorage.name || "storage"}`
            : `Push to ${workspaceStorage.name || "storage"}`}
        aria-expanded={workspacePushReviewOpen}
      >
        <Icon
          key={workspacePushSpinning ? "push-progress" : "push-idle"}
          name={workspacePushSpinning ? "refresh" : "upload"}
          size={14}
          className={workspacePushSpinning ? "icon-spin" : undefined}
        />
        {workspacePushReadout
          ? <span className="sync-btn-readout">{workspacePushReadout}</span>
          : <span className="sync-path-sync-label">{
            workspacePushReviewOpen && !workspacePushProgress && inlineStorageReview?.continueLabel
              ? inlineStorageReview.continueLabel
              : projectPushActionLabel({ preparing: workspacePushPreparing, publishing: workspacePushPublishing })
          }</span>}
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
  const cloudNotice = managedCloudKey ? cloudStatusNotice(cloudStatus) : null;
  if (cloudNotice) {
    workspaceNotices.push({
      key: "managed-cloud-status", severity: "warning",
      title: cloudNotice.title,
      message: cloudNotice.message,
      action: cloudNotice.action === null ? undefined : (
        <button
          type="button"
          className="btn btn-ghost"
          disabled={activityHeader?.refreshing}
          onClick={() => cloudNotice.action === "storage_settings" && managedCloudStorageId ? onOpenStorage(managedCloudStorageId) : cloudRefreshRef.current?.()}
        >
          {cloudNotice.action === "storage_settings" ? "Storage settings" : "Retry"}
        </button>
      ),
    });
  }
  if (cloudActionError?.key === workspaceActionPrefix) {
    workspaceNotices.push({ key: "managed-cloud-action", severity: "warning", message: cloudActionError.message });
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
              const projectBinding = bindingByProject.get(project.local_project_id);
              const bindingActive = projectBinding?.state === "active";
              const renderStorageConnectionActions = () => (
                <>
                  {workspaceAvailableStorages.length > 0 && (
                    <button
                      type="button"
                      className="sync-history-icon-action"
                      disabled={busy || !!inlineStorageReview}
                      title="Link storage"
                      aria-label="Link storage"
                      aria-haspopup="dialog"
                      onClick={() => setLinkingProjectId(project.local_project_id)}
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
                && !storageBlockShown;
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
                          aria-haspopup={workspaceAvailableStorages.length > 0 ? "dialog" : undefined}
                          onClick={() => {
                            if (workspaceAvailableStorages.length === 0) {
                              setLinkingProjectId(null);
                              onAddStorage();
                              return;
                            }
                            setLinkingProjectId(project.local_project_id);
                          }}
                        >
                          <Icon name={workspaceAvailableStorages.length > 0 ? "link" : "plus"} size={13} />
                          {workspaceAvailableStorages.length > 0 ? "Link storage" : "Add storage"}
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

                      /* The gates form a discriminated union, so at most one
                       * is present in the shared notice box. A storage that
                       * another computer cleared is not one of them: it stops
                       * no Push and no Pull, and the activity list below
                       * carries its standing note.
                       *
                       * All of them are events without exception: ProjectSync
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
                      if (activeNotice?.kind === "nothing_to_pull") {
                        /* A cleared storage this computer synced with
                           before. There is nothing to pull, and the standing
                           note in the activity list says to push again. */
                        gateNotices.push({
                          key: "nothing-to-pull",
                          severity: "info",
                          message: activeNotice.message,
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
                                  onClick={() => onOpenStorage(storage.storage_id)}
                                >
                                  Reconnect {storage.provider_label}
                                </button>
                              ) : (
                                <button
                                  type="button"
                                  className="btn btn-ghost"
                                  disabled={noticeActionsDisabled}
                                  onClick={() => void run(
                                    `${failedAction}:${actionPrefix}`,
                                    () => activeNotice.action === "push"
                                      ? onPush(project.local_project_id, storage.storage_id, activeNotice.scope, activeNotice.session)
                                      : onPull(project.local_project_id, storage.storage_id, activeNotice.scope),
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
                            <SyncOutcomeNotice presentAction={projectOutcomeAction}
                              outcome={activeNotice.outcome}
                              busy={noticeActionsDisabled}
                              onAction={(action) => onStorageNoticeAction?.(activeNotice.outcome, action)}
                              onDismiss={() => onDismissStorageNotice?.()}
                            />
                          )}
                          <NoticeZone
                            items={gateNotices}
                            subject="reasons this storage is held"
                            className="sync-storage-notice notice-strip"
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
                    <NoticeChip severity="warning">Agent profile needed</NoticeChip>
                  )}
                  <Icon name="chevron-right" size={12} className="sync-storage-index-open" aria-hidden="true" />
                </button>
              );
            })}
          </div>
        ))}

        {activeProject && !newProjectSetup && !inlineStorageReview && (
          <div
            id={PROJECT_VIEW_PANEL_ID}
            className="sync-project-tab-panel"
            role={activeProject.is_git_repository ? undefined : "tabpanel"}
            aria-labelledby={activeProject.is_git_repository ? undefined : projectViewTabId(activityView)}
          >
            {filesFaceShown && (
              <ProjectFilesActivityView
                inventory={projectFilesScan.inventory}
                storageName={storages.find((storage) => storage.storage_id === activeStorageId)?.name.trim() || "storage"}
                loading={projectFilesScan.loading}
                error={projectFilesScan.error}
                onOpenFile={(relativePath) => openProjectFile(activeProject.local_project_id, relativePath)}
                onRevealPath={revealPath}
                viewSwitch={projectViewSwitch}
              />
            )}
            {/* The Sessions face stays mounted under the Files face: its
                comparison is what the bar's thread counts, attention chip and
                Cloud status read, and those speak for the project on both
                faces. Only the face on screen seats the switch, so its tab
                ids stay unique for the focus hand-off. */}
            <div hidden={filesFaceShown}>
              <ProjectActivityPage
                comparisonErrorHandled={!!managedCloudKey}
                project={activeProject}
                binding={activeBindingByProject.get(activeProject.local_project_id) ?? null}
                refreshEpoch={historyRefreshEpoch}
                activeStorageId={activeStorageId}
                activeStorageName={storages.find((storage) => storage.storage_id === activeStorageId)?.name ?? null}
                continuityActionBusy={busy || !!runningAction}
                onHeaderModelChange={handleActivityHeaderChange}
                onChooseAgentProfile={() => {
                  void onConfigureBinding(activeProject.local_project_id);
                }}
                onActionError={onSessionActionError}
                onOpenSessionDetail={(request, trigger) => (
                  openSessionDetail(activeProject.local_project_id, request, trigger)
                )}
                viewSwitch={filesFaceShown ? undefined : projectViewSwitch}
              />
            </div>
          </div>
        )}
      </section>

      {sessionDetailSubview && sessionBack && workspaceProject && (
        <Suspense fallback={(
          /* Not role="status": the region and its sentence mount in the same
             commit, so there is no change for a live region to announce — the
             text is simply what this view renders with while the chunk
             loads. */
          <section className="session-detail-page session-detail-entry-state">
            <SessionDetailBack target={sessionBack} />
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
            back={sessionBack}
            onHeaderChange={handleSessionHeaderChange}
            pushGate={workspaceReviewOpen ? null : sessionPushHold}
            onOpenSession={(resourceId) => onOpenSessionDetail(sessionDetailSubview.localProjectId, {
              resourceId,
              occurrenceKey: resourceId,
            })}
          />
        </Suspense>
      )}

    </main>

    {/* Page level, not inside the storage card: the card returns null once the
        project is settled, and the chooser must outlive it. A modal <dialog>
        is promoted to the top layer, so where it sits in the tree only decides
        what unmounts it. */}
    {workspaceProject
      && linkingProjectId === workspaceProject.local_project_id
      && workspaceAvailableStorages.length > 0 && (
      <StorageLinkDialog
        projectName={projectLabel(workspaceProject)}
        storages={workspaceAvailableStorages}
        busy={busy || !!runningAction}
        onLink={(storageId) => void run(
          `link:${workspaceProject.local_project_id}:${storageId}`,
          async () => {
            await onLinkStorage(workspaceProject.local_project_id, storageId);
            setLinkingProjectId(null);
          },
        )}
        onAddStorage={() => {
          setLinkingProjectId(null);
          onAddStorage();
        }}
        onCancel={() => setLinkingProjectId(null)}
      />
    )}
    {publishVersionNumber !== null && (
      <PublishVersionSheet
        version={publishVersionNumber}
        holds={publishVersionHolds(cloudActivity?.comparison, cloudFileScan, cloudStatus?.publication)}
        sessionsToPush={cloudActivity?.counts.push ?? 0}
        busy={runningAction === `publish:${managedCloudKey}`}
        onPublish={publishCloudVersion}
        onClose={() => setPublishSheetKey(null)}
      />
    )}
    </>
  );
}
