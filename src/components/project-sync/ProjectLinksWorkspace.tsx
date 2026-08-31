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
  CodexConversationPathAudit,
  LocalProjectSummary,
  ProjectBinding,
  ProjectStorageLink,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfileSummary,
  RemoteBundleSummary,
  SiblingBundleStatus,
  StorageConfig,
  StorageSetupDraft,
  StorageSetupDraftPayload,
} from "../../types";
import Icon from "../Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { formatDateTime, toEpochMilliseconds } from "../../clock";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import ProjectActivityPage, { BranchPicker, type ActivityHeaderModel } from "./ProjectActivityPage";
import ProjectFilesActivityPage, { type FilesActivityHeaderModel } from "./ProjectFilesActivityPage";
import type {
  ActiveProjectSubview,
  OpenSessionDetailRequest,
} from "./session-detail/sessionDetailModel";
import ConversationPathRepairNotice, {
  shouldShowConversationPathRepairNotice,
} from "./ConversationPathRepairNotice";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import ProviderMark from "../sync-review/ProviderMark";
import type { SessionHeaderModel } from "./session-detail/SessionDetailPage";
import type { PullRequiredCopy } from "./PushResourceWorkspace";
import SyncOutcomeNotice from "./SyncOutcomeNotice";
import { writeClipboardText } from "../sync-review/clipboard";
import SiblingRepoNotice from "./SiblingRepoNotice";
import { isIrrecoverableSyncError } from "./syncError";
import type { SyncOutcome } from "./syncOutcome";
import { newStorage, r2FieldsSet, storageConfigReady, StorageEditor } from "./StorageSettings";
import { storageCatalogApi } from "../storage-catalog/api";
import { catalogKindMeta } from "../storage-catalog/model";
import type { CatalogStorageRepository } from "../storage-catalog/types";
import { projectSyncApi } from "./api";
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
  | { mode: "create"; storageKind: "local" | "s3"; requestId: number }
  | { mode: "resume-draft"; draft: StorageSetupDraft; requestId: number }
  | { mode: "close"; requestId: number };

/** Autosave cadence for the New-storage form, matching project setup. */
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
  /** Registered provider descriptors. The agent-setup cluster derives its
   *  run buttons from these; it never holds a fixed provider list. */
  providers: ProviderDescriptor[];
  storages: StorageConfig[];
  /** Redacted machine-local drafts, refreshed when the CLI updates one. */
  storageSetupDrafts: StorageSetupDraft[];
  links: ProjectStorageLink[];
  loading: boolean;
  busy: boolean;
  error: string | null;
  conversationPathAudits: Record<string, CodexConversationPathAudit>;
  conversationPathAuditErrors: Record<string, string>;
  conversationPathAuditLoading: boolean;
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
  onRepairConversationPaths: (projectId: string) => Promise<void> | void;
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
  /** Open a storage's page; `settingsExpanded` lands with the editor open. */
  onOpenStorage: (storageId: string, settingsExpanded: boolean) => void;
  /** The Storages crumb every storage page is rooted at. */
  onOpenStorageIndex: () => void;
  onClearStorage: (storageId: string) => void;
  onRemoveStorage: (storageId: string) => void;
  onSaveStorage: (
    storage: StorageConfig,
    expectedBaseline: StorageConfig | null,
    setupDraftId?: string | null,
    setupDraftRevision?: number | null,
    setupDraftStorageId?: string | null,
  ) => Promise<boolean | void> | boolean | void;
  /** Commit-free probe of the New-storage form against R2. Resolves true on
   *  a verified connection; failures surface through the shared error zone. */
  onTestStorage?: (
    storage: StorageConfig,
    setupDraftId?: string | null,
    setupDraftRevision?: number | null,
    setupDraftStorageId?: string | null,
  ) => Promise<boolean>;
  /** The persisted storage-draft list changed (created, updated, discarded). */
  onStorageDraftsChanged?: () => void;
  storageEditorRequest?: StorageEditorRequest | null;
  onStorageEditorRequestHandled?: () => void;
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
  onStorageNoticeAction?: (outcome: SyncOutcome) => void;
  onDismissStorageNotice?: () => void;
}

const SessionDetailPage = lazy(() => import("./session-detail/SessionDetailPage"));

/** The client-editable draft fields. The storage id stays server-owned: the
 *  backend mints it and returns it only as metadata that Test/Finish echoes
 *  separately as `draft_storage_id` to pin the exact saved snapshot. */
export function storageDraftPayload(storage: StorageConfig): StorageSetupDraftPayload {
  const { id: _id, supports_conditional_writes: _writes, ...payload } = storage;
  return payload;
}

export function storageSubtitle(storage: StorageConfig): string {
  if (storage.kind === "local") return compactProjectPath(storage.local_dir || "Folder not configured");
  if (storage.bucket) return `Bucket · ${storage.bucket}`;
  return storage.s3_endpoint ? "S3 endpoint configured" : "S3 storage not configured";
}

// Only the local subtitle is a path; the rest are phrases. The menu says which
// one it is handing over so the stylesheet reads a class instead of guessing.
export function storageSubtitleIsPath(storage: StorageConfig): boolean {
  return storage.kind === "local";
}

function sameStorage(left: StorageConfig, right: StorageConfig): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

export type StorageDraftReconciliation =
  | { kind: "unchanged" }
  | { kind: "adopt"; storage: StorageConfig }
  | { kind: "close" }
  | { kind: "conflict"; reason: "changed" | "removed" };

/** Pure decision used when the config watcher supplies a newer snapshot. */
export function reconcileStorageDraft(
  baseline: StorageConfig,
  draft: StorageConfig,
  incoming: StorageConfig | null,
): StorageDraftReconciliation {
  const incomingChanged = !incoming || !sameStorage(incoming, baseline);
  if (!incomingChanged) return { kind: "unchanged" };
  if (incoming && sameStorage(draft, incoming)) {
    return { kind: "adopt", storage: incoming };
  }
  if (!sameStorage(draft, baseline)) {
    return { kind: "conflict", reason: incoming ? "changed" : "removed" };
  }
  return incoming ? { kind: "adopt", storage: incoming } : { kind: "close" };
}

export type AgentSetupMarker = {
  draftFingerprint: string;
  editingDraftId: string;
  setupDraftId: string;
  setupDraftRevision: number;
  setupStorageId: string;
  requestedName: string;
};

export type AgentSetupReconciliation =
  | { kind: "adopt"; draft: StorageSetupDraft }
  | { kind: "preserve"; reason: "draft_changed" | "no_update" | "invalid" };

function storageDraftFingerprint(storage: StorageConfig): string {
  const {
    access_key_id,
    secret_access_key,
    ...nonCredentialFields
  } = storage;
  const redactedSnapshot = JSON.stringify({
    ...nonCredentialFields,
    has_access_key_id: access_key_id.length > 0,
    has_secret_access_key: secret_access_key.length > 0,
  });
  // Keep the marker fixed-size and credential-free. Draft edits invalidate
  // the live marker immediately; this redacted fingerprint additionally makes
  // the pure reconciliation decision independent of credential contents.
  let hash = 0xcbf29ce484222325n;
  for (const character of redactedSnapshot) {
    hash ^= BigInt(character.codePointAt(0) ?? 0);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return hash.toString(16).padStart(16, "0");
}

function normalizedStorageName(name: string): string {
  return name.trim();
}

export function createAgentSetupMarker(
  draft: StorageConfig,
  setupDraftId: string,
  setupDraftRevision: number,
  editingDraftId: string,
  setupStorageId: string,
): AgentSetupMarker {
  return {
    draftFingerprint: storageDraftFingerprint(draft),
    editingDraftId,
    setupDraftId,
    setupDraftRevision,
    setupStorageId,
    requestedName: normalizedStorageName(draft.name),
  };
}

/** Decide whether the CLI updated the exact persisted draft the open form
 *  handed it. Name matching is deliberately insufficient: another draft can
 *  have the same label, and an older refresh must not look like CLI success. */
export function reconcileAgentSetup(
  marker: AgentSetupMarker,
  draft: StorageConfig,
  editingDraftId: string,
  setupDrafts: readonly StorageSetupDraft[],
): AgentSetupReconciliation {
  if (
    marker.editingDraftId !== editingDraftId
    || marker.draftFingerprint !== storageDraftFingerprint(draft)
  ) {
    return { kind: "preserve", reason: "draft_changed" };
  }
  const candidate = setupDrafts.find(({ draft_id }) => draft_id === marker.setupDraftId);
  if (!candidate || candidate.revision <= marker.setupDraftRevision) {
    return { kind: "preserve", reason: "no_update" };
  }
  if (
    candidate.revision !== marker.setupDraftRevision + 1
    || candidate.storage.id !== marker.setupStorageId
    || candidate.storage.kind !== "s3"
    || normalizedStorageName(candidate.storage.name) !== marker.requestedName
    || !storageConfigReady(candidate.storage, candidate.secret_present)
  ) {
    return { kind: "preserve", reason: "invalid" };
  }
  return { kind: "adopt", draft: candidate };
}

/** What the guide row's agent slot is saying: an agent CLI was opened in a
 *  terminal, the copy landed and the page is listening, the CLI's draft was
 *  adopted for review, or the hand-off failed. */
export type AgentSetupNotice =
  | { kind: "listening" }
  | { kind: "launched"; agent: string }
  | { kind: "ready"; message: string }
  | { kind: "error"; message: string };

export type AgentSetupEditorAdoption = {
  editingStorage: LinkKey;
  storageDraft: StorageConfig;
  storageBaseline: null;
  storageSetupDraftId: string;
  storageSetupSecretPresent: boolean;
  storageSetupDraftRevision: number;
  storageDetailsExpanded: true;
  status: string;
};

/** State transition that keeps the editable New-storage form open. Nothing is
 *  committed until the user presses Finish setup. */
export function agentSetupEditorAdoption(draft: StorageSetupDraft): AgentSetupEditorAdoption {
  return {
    editingStorage: { projectId: "", storageId: draft.storage.id },
    storageDraft: { ...draft.storage },
    storageBaseline: null,
    storageSetupDraftId: draft.draft_id,
    storageSetupSecretPresent: draft.secret_present,
    storageSetupDraftRevision: draft.revision,
    storageDetailsExpanded: true,
    status: "R2 settings filled in. Review or edit them, optionally test the connection, then finish setup yourself.",
  };
}

export function buildR2AgentSetupInstruction({
  storageName,
  cliPath,
  draftId,
  expectedRevision,
}: {
  storageName: string;
  cliPath: string;
  draftId: string;
  expectedRevision: number;
}): string {
  const requestedName = storageName.trim() || "Cloudflare R2";
  return [
    `Prepare the Cloudflare R2 storage “${requestedName}” draft through the Mallard CLI for my later review. Do not finish the setup for me. Do not invoke Computer Use for any purpose, including read-only verification.`,
    "",
    "Use this Mallard CLI:",
    cliPath,
    "",
    "Treat that as a literal executable path and quote it for the current shell; do not interpret it as shell syntax.",
    "",
    "First check the required runtime dependency without installing anything: run `wrangler --version` and confirm that `wrangler` is available on PATH.",
    "",
    "If Wrangler is missing or `wrangler --version` fails, stop before invoking Mallard setup. Tell me that Mallard requires a working `wrangler` command on PATH, ask me to approve installing it, and show me this exact command: `npm install --global wrangler@latest`. Do not run that installation unless I explicitly approve it. Before an approved installation, run `npm --version`; if npm is missing, report that separately and ask how I want to install Node.js and npm—do not choose or install a package manager on your own. After an approved installation, rerun `wrangler --version` and continue only if it succeeds. Mallard invokes `wrangler` directly, so do not substitute `npx wrangler` or a project-local-only installation.",
    "",
    "Once the Wrangler dependency check succeeds, run the Mallard CLI’s `storage r2 --help` command.",
    "",
    "Before invoking setup, check whether your command-execution environment restricts writing to the current user’s ~/.mallard directory or launching the system browser. If it does, ask me for one-time approval to run the exact, fully quoted Mallard setup command described next with permission to write to the current user’s ~/.mallard directory and launch the system browser. Request approval only for that exact setup command, not for a shell or a broader command prefix. Do not invoke setup until I explicitly approve it; if I decline, stop.",
    "",
    `Then run its \`storage r2 setup\` command with all three required options: \`--name\` with the exact storage name shown above as one literal argument, \`--draft-id\` with the exact literal value \`${draftId}\`, and \`--expected-revision\` with the exact decimal value \`${expectedRevision}\`. Quote the executable path, storage name, and draft ID correctly for the current shell; do not interpret any value as shell syntax.`,
    "",
    "The Mallard CLI uses the existing Wrangler login and may open the system browser. I—not the agent—will review and complete Cloudflare login, MFA, and consent. Do not control the browser or inspect its contents. Wait for the CLI to finish; the agent should observe only its success or failure result.",
    "",
    "Complete only the draft-preparation handoff through the CLI. Do not work around an execution restriction by changing HOME, using temporary or project-local Mallard state, using sudo, editing ~/.mallard, or editing Mallard’s open setup form. Do not request or handle credentials, put a secret in an argument or environment variable, or print credentials.",
    "",
    "The setup command only prepares the exact schema-1 draft named above; it does not test the connection or commit the storage. Mallard is listening for that draft and will automatically fill the existing editable setup form when the CLI finishes. The CLI success or failure result is the end of the agent’s task. Do not use screenshots, browser automation, UI automation, or direct inspection of Mallard’s window or form to confirm the update. Report only the CLI success or failure result, then stop. I will confirm that the fields appeared, review or edit them, optionally test the connection, and personally click Finish setup when I am ready.",
  ].join("\n");
}

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
export function CatalogStorageRepositoryRow({ repo }: { repo: CatalogStorageRepository }) {
  const [open, setOpen] = useState(false);
  const meta = catalogKindMeta(repo.resource_kind);
  const name = `${meta.plural} · ${providerLabel(repo.provider)}`;
  const detailsId = `catalog-repo-details-${repo.repository_id}`;
  const unreadable = repo.error != null;
  /* The fold names the whole set until it stops being a glance. */
  const shownNames = repo.resource_names.slice(0, 12);
  const moreNames = repo.resource_names.length - shownNames.length;
  return (
    <div className="sync-storage-repository-item">
      <div className="sync-storage-repository-row">
        <span className={`sync-storage-repository-icon${unreadable ? " danger" : ""}`}>
          <Icon name={unreadable ? "alert-triangle" : meta.icon} size={14} />
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

export function conversationPathsBlockSync(
  hasBinding: boolean,
  audit: CodexConversationPathAudit | undefined,
  auditError: string | undefined,
  auditLoading: boolean,
): boolean {
  return hasBinding && (auditLoading || !!auditError || !audit || !audit.ready);
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
  storages,
  storageSetupDrafts,
  links,
  loading,
  busy,
  error,
  conversationPathAudits,
  conversationPathAuditErrors,
  conversationPathAuditLoading,
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
  onRepairConversationPaths,
  onRenameProject,
  onResetProjectMetadata,
  onRefresh,
  onAddProject,
  onSelectProject,
  appActions,
  onAddStorage,
  onOpenStorage,
  onOpenStorageIndex,
  onTestStorage,
  onStorageDraftsChanged,
  onClearStorage,
  onRemoveStorage,
  onSaveStorage,
  storageEditorRequest,
  onStorageEditorRequestHandled,
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
  const [editingStorage, setEditingStorage] = useState<LinkKey | null>(null);
  const [storageDraft, setStorageDraft] = useState<StorageConfig | null>(null);
  const [storageBaseline, setStorageBaseline] = useState<StorageConfig | null>(null);
  const [storageConflict, setStorageConflict] = useState<"changed" | "removed" | null>(null);
  const [agentNotice, setAgentNotice] = useState<AgentSetupNotice | null>(null);
  const [agentMenuOpen, setAgentMenuOpen] = useState(false);
  /* Which run button faces the split cluster: the last agent picked this
     session, else the first registered launcher. */
  const [preferredLauncher, setPreferredLauncher] = useState<ProviderKey | null>(null);
  const [copyAgainFlashed, setCopyAgainFlashed] = useState(false);
  const [storageDetailsExpanded, setStorageDetailsExpanded] = useState(false);
  /* The persisted setup draft behind the New-storage form. The id and the
     secret flag render; the revision is server-owned bookkeeping and the
     session counter invalidates async draft work when the form's identity
     changes — the same job bundleRequestRef does for listings. */
  const [storageSetupDraftId, setStorageSetupDraftId] = useState<string | null>(null);
  const [storageSetupSecretPresent, setStorageSetupSecretPresent] = useState(false);
  const [storageTestVerified, setStorageTestVerified] = useState(false);
  const [storageDraftActionError, setStorageDraftActionError] = useState<string | null>(null);
  const storageSetupDraftIdRef = useRef<string | null>(null);
  const storageSetupDraftRevisionRef = useRef(0);
  const draftDirtyRef = useRef(false);
  const draftSavingRef = useRef(false);
  const draftSaveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const draftEditVersionRef = useRef(0);
  const draftSessionRef = useRef(0);
  const resetStorageSetupDraftState = () => {
    draftSessionRef.current += 1;
    draftEditVersionRef.current += 1;
    draftDirtyRef.current = false;
    storageSetupDraftIdRef.current = null;
    storageSetupDraftRevisionRef.current = 0;
    setStorageSetupDraftId(null);
    setStorageSetupSecretPresent(false);
    setStorageTestVerified(false);
    setStorageDraftActionError(null);
  };
  const [renamingStorageId, setRenamingStorageId] = useState<string | null>(null);
  const [storageNameDraft, setStorageNameDraft] = useState("");
  const [remoteBundles, setRemoteBundles] = useState<BundleSnapshotSummary[] | null>(null);
  const [bundleLoading, setBundleLoading] = useState(false);
  const [bundleError, setBundleError] = useState<string | null>(null);
  /* The Catalog's kind repositories in the same storage, listed beside the
     project ones. Their failure has its own surface — a callout under the
     list — so one broken half never hides the other. */
  const [catalogRepos, setCatalogRepos] = useState<CatalogStorageRepository[] | null>(null);
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
  const agentSetupMarkerRef = useRef<AgentSetupMarker | null>(null);
  const agentSetupBusyRef = useRef(false);
  /* The exact instruction of the live hand-off, so Copy again re-copies the
     same text the marker was armed for. */
  const agentInstructionRef = useRef<string | null>(null);
  const copyAgainTimerRef = useRef<number | null>(null);
  const agentSplitRef = useRef<HTMLDivElement | null>(null);

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

  useEffect(() => {
    setRenamingStorageId(null);
    setStorageNameDraft(storageDraft?.name ?? "");
  }, [storageDraft?.id]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      workspaceScrollRef.current?.scrollTo({ top: 0, left: 0 });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [activeProjectId, activeStorageId]);

  useEffect(() => {
    if (activeProjectSubview.kind !== "session_detail") return;
    agentSetupMarkerRef.current = null;
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

  const savedEditedStorage = editingStorage
    ? storages.find((storage) => storage.id === editingStorage.storageId) ?? null
    : null;
  const editedStorageConfig = savedEditedStorage
    ?? (storageDraft?.id === editingStorage?.storageId ? storageDraft : null);
  const creatingStorage = !!editingStorage && storageBaseline === null;
  const orderedRemoteBundles = [...(remoteBundles ?? [])].sort((left, right) => {
    return (right.updated_at ?? 0) - (left.updated_at ?? 0);
  });
  /* Catalog sets follow the projects, newest first among themselves; a
     refusal row carries no timestamp and sinks to the end. */
  const orderedCatalogRepos = [...(catalogRepos ?? [])].sort((left, right) => {
    return (right.updated_at ?? 0) - (left.updated_at ?? 0);
  });
  const run = async (key: string, action: () => Promise<void> | void) => {
    setRunningAction(key);
    try {
      await action();
    } finally {
      setRunningAction(null);
    }
  };

  const loadStorageBundles = async (storage: StorageConfig) => {
    const storageId = storage.id;
    const requestId = ++bundleRequestRef.current;
    setRemoteBundles(null);
    setBundleError(null);
    setCatalogRepos(null);
    setCatalogRepoError(null);
    setBundleLoading(true);
    // Both reads start together under the one loading gate.
    const bundlesRead = projectSyncApi.listRemoteBundleSnapshots(storageId);
    const catalogRead = storageCatalogApi.listStorageRepositories(storageId);
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
      const repositories = await catalogRead;
      if (bundleRequestRef.current === requestId) setCatalogRepos(repositories);
    } catch (reason) {
      if (bundleRequestRef.current === requestId) {
        setCatalogRepoError(storageAccessErrorMessage(storage, reason));
      }
    } finally {
      if (bundleRequestRef.current === requestId) setBundleLoading(false);
    }
  };

  useEffect(() => {
    if (!storageRefreshEpoch || !editedStorageConfig) return;
    void loadStorageBundles(editedStorageConfig);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storageRefreshEpoch]);

  /* Every navigation away funnels through the close request, so this is
     where the persisted draft gets its last best-effort save — the debounced
     autosave usually landed already, and a save already in flight carries
     the same edits, so neither case flushes twice. The draft file itself
     survives the close; only the form state resets. */
  const flushStorageSetupDraft = () => {
    if (
      !draftDirtyRef.current
      || draftSavingRef.current
      || !storageSetupDraftIdRef.current
      || !storageDraft
    ) return;
    draftDirtyRef.current = false;
    void projectSyncApi
      .updateStorageSetupDraft(
        storageSetupDraftIdRef.current,
        storageSetupDraftRevisionRef.current,
        storageDraftPayload(storageDraft),
      )
      .then(() => onStorageDraftsChanged?.())
      .catch(() => {
        // Best-effort: at most the last debounce window of typing is lost,
        // the same guarantee project setup gives.
      });
  };

  /** Serialize saves of the open draft. The agent handoff may be clicked
   *  while the debounce save is still in flight; reading the id and revision
   *  refs only when this operation reaches the head of the queue makes the
   *  second write update the draft just created by the first one. */
  const persistStorageSetupDraft = (
    storage: StorageConfig,
    session: number,
  ): Promise<StorageSetupDraft | null> => {
    const operation = draftSaveQueueRef.current.then(async () => {
      if (draftSessionRef.current !== session) return null;
      draftSavingRef.current = true;
      try {
        const draftId = storageSetupDraftIdRef.current;
        const saved = draftId
          ? await projectSyncApi.updateStorageSetupDraft(
              draftId,
              storageSetupDraftRevisionRef.current,
              storageDraftPayload(storage),
            )
          : await projectSyncApi.createStorageSetupDraft(storageDraftPayload(storage));
        if (draftSessionRef.current !== session) return null;
        storageSetupDraftIdRef.current = saved.draft_id;
        storageSetupDraftRevisionRef.current = saved.revision;
        setStorageSetupDraftId(saved.draft_id);
        setStorageSetupSecretPresent(saved.secret_present);
        onStorageDraftsChanged?.();
        return saved;
      } finally {
        draftSavingRef.current = false;
      }
    });
    draftSaveQueueRef.current = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation;
  };

  /** Test and Finish must act on the exact snapshot the user reviewed. Put
   *  that snapshot behind any autosave already in flight, then reject the
   *  action if the form changed while the guarded save was pending. */
  const persistStorageSetupDraftForAction = async (
    storage: StorageConfig,
    session: number,
    editVersion: number,
    action: "testing the connection" | "finishing setup",
  ): Promise<StorageSetupDraft | null> => {
    draftDirtyRef.current = false;
    try {
      const saved = await persistStorageSetupDraft(storage, session);
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

  useEffect(() => {
    if (!storageEditorRequest) return;
    agentSetupMarkerRef.current = null;
    if (storageEditorRequest.mode === "close") {
      bundleRequestRef.current += 1;
      flushStorageSetupDraft();
      resetStorageSetupDraftState();
      setEditingStorage(null);
      setStorageDraft(null);
      setStorageBaseline(null);
      setStorageConflict(null);
      setAgentNotice(null);
      setStorageDetailsExpanded(false);
      setRemoteBundles(null);
      setBundleError(null);
      setCatalogRepos(null);
      setCatalogRepoError(null);
      setBundleLoading(false);
      onStorageEditorRequestHandled?.();
      return;
    }
    if (storageEditorRequest.mode === "create") {
      const storage = newStorage(storageEditorRequest.storageKind, storages.length + 1);
      bundleRequestRef.current += 1;
      resetStorageSetupDraftState();
      setEditingStorage({ projectId: "", storageId: storage.id });
      setStorageDraft(storage);
      setStorageBaseline(null);
      setStorageConflict(null);
      setAgentNotice(null);
      setStorageDetailsExpanded(true);
      setRemoteBundles(null);
      setBundleError(null);
      setCatalogRepos(null);
      setCatalogRepoError(null);
      setBundleLoading(false);
      onStorageEditorRequestHandled?.();
      return;
    }
    if (storageEditorRequest.mode === "resume-draft") {
      const draft = storageEditorRequest.draft;
      bundleRequestRef.current += 1;
      resetStorageSetupDraftState();
      setEditingStorage({ projectId: "", storageId: draft.storage.id });
      // Opening one exact draft is the narrow IPC path that includes its
      // secret, so the existing password field can reveal it for review.
      // Background draft listings remain redacted.
      setStorageDraft({ ...draft.storage });
      setStorageBaseline(null);
      setStorageConflict(null);
      setAgentNotice(null);
      setStorageDetailsExpanded(true);
      setRemoteBundles(null);
      setBundleError(null);
      setCatalogRepos(null);
      setCatalogRepoError(null);
      setBundleLoading(false);
      setStorageSetupDraftId(draft.draft_id);
      storageSetupDraftIdRef.current = draft.draft_id;
      setStorageSetupSecretPresent(draft.secret_present);
      storageSetupDraftRevisionRef.current = draft.revision;
      onStorageEditorRequestHandled?.();
      return;
    }
    const storage = storages.find((candidate) => candidate.id === storageEditorRequest.storageId);
    if (!storage) {
      onStorageEditorRequestHandled?.();
      return;
    }

    if (editingStorage?.storageId !== storage.id) {
      flushStorageSetupDraft();
      resetStorageSetupDraftState();
      setEditingStorage({ projectId: "", storageId: storage.id });
      setStorageDraft({ ...storage });
      setStorageBaseline({ ...storage });
      setStorageConflict(null);
      setAgentNotice(null);
      void loadStorageBundles(storage);
    }
    setStorageDetailsExpanded(storageEditorRequest.settingsExpanded);
    onStorageEditorRequestHandled?.();
  }, [storageEditorRequest?.requestId]);

  /* Debounced autosave of the New-storage form, mirroring the project setup
     recipe: the draft file is created lazily on the first edit, updated on
     later ones, and a change of form identity cancels the pending timer —
     the close funnel does its own flush. Only server-owned fields are
     adopted back; the editor is keyed by the storage id, so the view's
     server-minted id would remount the form mid-typing. */
  useEffect(() => {
    if (
      !creatingStorage
      || !storageDraft
      || !draftDirtyRef.current
    ) return;
    const session = draftSessionRef.current;
    const timer = window.setTimeout(() => {
      void (async () => {
        if (
          draftSessionRef.current !== session
          || !draftDirtyRef.current
        ) return;
        draftDirtyRef.current = false;
        try {
          await persistStorageSetupDraft(storageDraft, session);
        } catch {
          // The form stays the source of truth while open; the next edit
          // retries. A draft is a safety net, not a gate.
        }
      })();
    }, STORAGE_DRAFT_AUTOSAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [storageDraft, storageSetupDraftId, creatingStorage]);

  const openStorageEditorWithSettings = async (storage: StorageConfig) => {
    const switchingStorage = editingStorage?.storageId !== storage.id;
    if (switchingStorage) {
      agentSetupMarkerRef.current = null;
      setEditingStorage({ projectId: "", storageId: storage.id });
      setStorageDraft({ ...storage });
      setStorageBaseline({ ...storage });
      setStorageConflict(null);
      setAgentNotice(null);
    }

    setStorageDetailsExpanded(true);
    if (switchingStorage) await loadStorageBundles(storage);
  };

  useEffect(() => {
    const marker = agentSetupMarkerRef.current;
    if (!marker || !editingStorage || !storageDraft || storageBaseline) return;
    const reconciliation = reconcileAgentSetup(
      marker,
      storageDraft,
      editingStorage.storageId,
      storageSetupDrafts,
    );
    if (reconciliation.kind === "preserve") {
      if (reconciliation.reason === "draft_changed" || reconciliation.reason === "invalid") {
        agentSetupMarkerRef.current = null;
        if (reconciliation.reason === "invalid") {
          setAgentNotice({
            kind: "error",
            message: "Mallard received an incomplete R2 draft. Review the CLI result, then try the agent setup again.",
          });
        }
      }
      return;
    }

    // Draft listings stay redacted because the watcher refreshes them in the
    // background. Reveal only the exact, pinned draft the CLI advanced, then
    // revalidate every identity and form-session guard after the IPC read.
    const session = draftSessionRef.current;
    const editVersion = draftEditVersionRef.current;
    const editingDraftId = editingStorage.storageId;
    const formSnapshot = { ...storageDraft };
    let cancelled = false;
    void (async () => {
      try {
        const revealed = await projectSyncApi.getStorageSetupDraft(marker.setupDraftId);
        if (
          cancelled
          || agentSetupMarkerRef.current !== marker
          || draftSessionRef.current !== session
          || draftEditVersionRef.current !== editVersion
        ) return;
        const revealedReconciliation = revealed
          ? reconcileAgentSetup(marker, formSnapshot, editingDraftId, [revealed])
          : { kind: "preserve", reason: "invalid" } as const;
        if (
          !revealed
          || revealedReconciliation.kind !== "adopt"
          || !revealed.secret_present
          || !revealed.storage.secret_access_key
        ) {
          agentSetupMarkerRef.current = null;
          setAgentNotice({
            kind: "error",
            message: "Mallard could not load the complete R2 values prepared by the CLI. Copy fresh agent setup instructions and try again.",
          });
          return;
        }

        agentSetupMarkerRef.current = null;
        // Invalidate any pending debounce from before the CLI update. The
        // exact-get response includes the secret so the existing editable
        // password field can show it on request; the form remains uncommitted.
        draftSessionRef.current += 1;
        draftEditVersionRef.current += 1;
        draftDirtyRef.current = false;
        const adoption = agentSetupEditorAdoption(revealedReconciliation.draft);
        storageSetupDraftIdRef.current = adoption.storageSetupDraftId;
        storageSetupDraftRevisionRef.current = adoption.storageSetupDraftRevision;
        setEditingStorage(adoption.editingStorage);
        setStorageDraft(adoption.storageDraft);
        setStorageBaseline(adoption.storageBaseline);
        setStorageSetupDraftId(adoption.storageSetupDraftId);
        setStorageSetupSecretPresent(adoption.storageSetupSecretPresent);
        setStorageTestVerified(false);
        setStorageConflict(null);
        setStorageDetailsExpanded(adoption.storageDetailsExpanded);
        setRemoteBundles(null);
        setBundleError(null);
        setStorageDraftActionError(null);
        setAgentNotice({ kind: "ready", message: adoption.status });
      } catch {
        if (
          cancelled
          || agentSetupMarkerRef.current !== marker
          || draftSessionRef.current !== session
          || draftEditVersionRef.current !== editVersion
        ) return;
        agentSetupMarkerRef.current = null;
        setAgentNotice({
          kind: "error",
          message: "Mallard could not load the R2 values prepared by the CLI. Copy fresh agent setup instructions and try again.",
        });
      }
    })();
    return () => { cancelled = true; };
  }, [storageSetupDrafts]);

  useEffect(() => {
    if (!editingStorage || !storageDraft) return;
    const incoming = storages.find((candidate) => candidate.id === editingStorage.storageId) ?? null;
    if (!storageBaseline) {
      if (incoming && sameStorage(incoming, storageDraft)) {
        setStorageBaseline({ ...incoming });
        setStorageDraft({ ...incoming });
      }
      return;
    }
    const reconciliation = reconcileStorageDraft(storageBaseline, storageDraft, incoming);
    if (reconciliation.kind === "adopt") {
      setStorageBaseline({ ...reconciliation.storage });
      setStorageDraft({ ...reconciliation.storage });
      setStorageConflict(null);
    } else if (reconciliation.kind === "close") {
      setEditingStorage(null);
      setStorageDraft(null);
      setStorageBaseline(null);
      setStorageConflict(null);
      setStorageDetailsExpanded(false);
    } else if (reconciliation.kind === "conflict") {
      setStorageConflict(reconciliation.reason);
    }
  }, [editingStorage?.storageId, storageBaseline, storageDraft, storages]);

  const reloadExternalStorage = async () => {
    if (!editingStorage) return;
    // The draft can hold values shown exactly once (an R2 secret key), so it
    // is never dropped silently: loading the current configuration is the
    // only action offered, and it asks first when it would lose edits.
    if (storageDraft && (!storageBaseline || !sameStorage(storageDraft, storageBaseline))) {
      const approved = await confirm(
        "Discard your unsaved draft and load the current configuration?\n\nAnything typed here that was never saved — including a secret access key — cannot be recovered.",
        { title: "Discard draft" },
      );
      if (!approved) return;
    }
    const incoming = storages.find((candidate) => candidate.id === editingStorage.storageId) ?? null;
    if (!incoming) {
      setEditingStorage(null);
      setStorageDraft(null);
      setStorageBaseline(null);
      setStorageConflict(null);
      setStorageDetailsExpanded(false);
      return;
    }
    setStorageBaseline({ ...incoming });
    setStorageDraft({ ...incoming });
    setStorageConflict(null);
  };

  const clearCopyAgainFlash = () => {
    if (copyAgainTimerRef.current !== null) {
      window.clearTimeout(copyAgainTimerRef.current);
      copyAgainTimerRef.current = null;
    }
    setCopyAgainFlashed(false);
  };
  useEffect(() => () => {
    if (copyAgainTimerRef.current !== null) window.clearTimeout(copyAgainTimerRef.current);
  }, []);

  /* The split cluster's menu closes like the path bar's: outside press or
     Escape, without disturbing any review listening on window. */
  useEffect(() => {
    if (!agentMenuOpen) return;
    const closeOnOutside = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Node && agentSplitRef.current?.contains(target)) return;
      setAgentMenuOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      setAgentMenuOpen(false);
    };
    document.addEventListener("mousedown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [agentMenuOpen]);

  /** Persist the exact form snapshot, build the guarded instruction, run one
   *  hand-off verb with it, then arm the listener. Both verbs share every
   *  fence: the pending debounce is cancelled, the CLI receives the persisted
   *  id and revision, and a form that changed mid-flight refuses the marker. */
  const runAgentSetupHandoff = async (
    action: (instruction: string) => Promise<void>,
    failure: (reason: unknown) => string,
    notice: AgentSetupNotice,
  ) => {
    if (!storageDraft || storageDraft.kind !== "s3" || agentSetupBusyRef.current) return;
    agentSetupBusyRef.current = true;
    setAgentMenuOpen(false);
    clearCopyAgainFlash();
    setAgentNotice(null);
    const session = draftSessionRef.current;
    const editVersion = draftEditVersionRef.current;
    const draftSnapshot = { ...storageDraft };
    try {
      draftDirtyRef.current = false;
      const saved = await persistStorageSetupDraft(draftSnapshot, session);
      if (
        !saved
        || draftSessionRef.current !== session
        || draftEditVersionRef.current !== editVersion
      ) {
        throw new Error("the storage form changed before the agent handoff was ready");
      }
      const cliPath = await projectSyncApi.getMallardCliPath();
      const instruction = buildR2AgentSetupInstruction({
        storageName: draftSnapshot.name,
        cliPath,
        draftId: saved.draft_id,
        expectedRevision: saved.revision,
      });
      await action(instruction);
      if (
        draftSessionRef.current !== session
        || draftEditVersionRef.current !== editVersion
      ) {
        throw new Error("the storage form changed during the agent handoff");
      }
      agentInstructionRef.current = instruction;
      agentSetupMarkerRef.current = createAgentSetupMarker(
        draftSnapshot,
        saved.draft_id,
        saved.revision,
        draftSnapshot.id,
        saved.storage.id,
      );
      setAgentNotice(notice);
    } catch (reason) {
      setAgentNotice({ kind: "error", message: failure(reason) });
    } finally {
      agentSetupBusyRef.current = false;
    }
  };

  const copyAgentSetupInstruction = () => runAgentSetupHandoff(
    (instruction) => writeClipboardText(instruction),
    (reason) => `Could not copy agent setup instructions: ${String(reason)}`,
    { kind: "listening" },
  );

  const launchAgentSetup = (provider: ProviderKey, agent: string) => runAgentSetupHandoff(
    (instruction) => projectSyncApi.launchR2AgentSetup(provider, instruction),
    (reason) => `Could not open ${agent}: ${String(reason)}`,
    { kind: "launched", agent },
  );

  /** Re-copy the exact instruction the live marker was armed for. */
  const copyAgentInstructionAgain = async () => {
    const instruction = agentInstructionRef.current;
    if (!instruction) return;
    try {
      await writeClipboardText(instruction);
      clearCopyAgainFlash();
      setCopyAgainFlashed(true);
      copyAgainTimerRef.current = window.setTimeout(() => {
        copyAgainTimerRef.current = null;
        setCopyAgainFlashed(false);
      }, 1100);
    } catch (reason) {
      setAgentNotice({ kind: "error", message: `Could not copy agent setup instructions: ${String(reason)}` });
    }
  };

  /** The user backs out of the hand-off: the marker dies, the offer returns. */
  const cancelAgentSetup = () => {
    agentSetupMarkerRef.current = null;
    agentInstructionRef.current = null;
    clearCopyAgainFlash();
    setAgentNotice(null);
  };

  /* The run buttons come from the registry: every provider shipping a CLI
     launcher gets one, and Copy covers every other agent. */
  const agentLaunchers = providers.flatMap((descriptor) => (
    descriptor.cli_launcher
      ? [{ key: descriptor.key, launcher: descriptor.cli_launcher }]
      : []
  ));
  const defaultAgentLauncher = agentLaunchers.find(({ key }) => key === preferredLauncher)
    ?? agentLaunchers[0]
    ?? null;

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
    .map((link) => storages.find((storage) => storage.id === link.storage_id))
    .filter((storage): storage is StorageConfig => !!storage);
  const workspaceStorage = workspaceLinkedStorages.find((storage) => (
    workspaceProject?.local_project_id === activeProjectId && storage.id === activeStorageId
  )) ?? workspaceLinkedStorages[0] ?? null;
  const workspaceAvailableStorages = storages.filter((storage) => (
    !workspaceLinks.some((link) => link.storage_id === storage.id)
  ));
  const workspaceBindingActive = workspaceBinding?.state === "active";
  const workspaceReadableProfiles = workspaceProfiles.filter(({ profile }) => (
    !!profile?.available && profile.readable
  ));
  const workspaceProfilesReadable = workspaceReadableProfiles.length > 0;
  const workspaceProfilesWritable = workspaceReadableProfiles.some(({ profile }) => !!profile?.writable);
  const workspaceCanSync = workspaceBindingActive && workspaceProfilesReadable;
  const workspaceCanRestore = workspaceBindingActive && workspaceProfilesWritable;
  const workspaceCodexProfile = workspaceProfiles.find(({ provider }) => provider === "codex")?.profile;
  const workspaceCodexConfigured = workspaceBindingActive && !!workspaceCodexProfile;
  const workspacePathAudit = workspaceProject
    ? conversationPathAudits[workspaceProject.local_project_id]
    : undefined;
  const workspacePathAuditError = workspaceProject
    ? conversationPathAuditErrors[workspaceProject.local_project_id]
    : undefined;
  const workspacePathBlocked = conversationPathsBlockSync(
    workspaceCodexConfigured,
    workspacePathAudit,
    workspacePathAuditError,
    conversationPathAuditLoading,
  );
  const workspacePathTitle = workspaceCodexConfigured
    ? conversationPathAuditLoading
      ? "Checking Codex conversation paths"
      : workspacePathAuditError
        ? `Codex conversation paths could not be verified: ${workspacePathAuditError}`
        : workspacePathAudit && !workspacePathAudit.ready
          ? "Repair Codex conversation paths before Push or Pull"
          : null
    : null;
  const workspaceProfileIssue = workspaceBindingActive && !workspaceProfilesReadable
    ? "No selected agent profile is available on this machine"
    : null;

  const workspaceActionPrefix = workspaceProject && workspaceStorage
    ? `${workspaceProject.local_project_id}:${workspaceStorage.id}`
    : null;
  const workspaceReviewOpen = !!workspaceActionPrefix
    && inlineStorageReview?.projectId === workspaceProject?.local_project_id
    && inlineStorageReview?.storageId === workspaceStorage?.id;
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
    if (!storageDraft || !editedStorageConfig || renamingStorageId !== storageDraft.id) return;
    const name = storageNameDraft.trim();
    if (!name || name === storageDraft.name) return;
    agentSetupMarkerRef.current = null;
    await run(`rename-storage:${storageDraft.id}`, async () => {
      const renamedStorage = { ...editedStorageConfig, name };
      const saved = await onSaveStorage(renamedStorage, editedStorageConfig);
      if (saved === false) return;
      setStorageDraft((current) => current?.id === renamedStorage.id ? { ...current, name } : current);
      // The rename saved only the persisted snapshot, not the editor's other
      // controlled values. Advance the baseline to that renamed snapshot so a
      // watcher echo is not mistaken for an external change; the draft update
      // above changes only its name and therefore keeps any typed R2 secret.
      setStorageBaseline({ ...renamedStorage });
      setStorageNameDraft(name);
      setRenamingStorageId(null);
    });
  };

  if (editingStorage && storageDraft && editedStorageConfig) {
    // A storage that is not a fresh draft is already saved, so the backend
    // holds its secret even though the redacted config renders the field
    // blank. A resumed setup draft holds one the same server-side way.
    const secretStored = !creatingStorage || storageSetupSecretPresent;
    const storageReady = storageConfigReady(storageDraft, secretStored);
    const showStorageDetails = creatingStorage || storageDetailsExpanded;
    const storageDetailsId = `storage-${storageDraft.id}-settings`;
    const storageNameChanged = !!storageNameDraft.trim() && storageNameDraft.trim() !== storageDraft.name;
    const storageName = storageDraft.name || "(unnamed)";
    const storageLinkCount = links.filter((link) => link.storage_id === editedStorageConfig.id).length;
    const renamingStorage = renamingStorageId === storageDraft.id;
    const saveEditedStorage = () => run(`storage:${editedStorageConfig.id}`, async () => {
      const storageSnapshot = { ...storageDraft };
      const baselineSnapshot = storageBaseline ? { ...storageBaseline } : null;
      const wasCreatingR2 = creatingStorage && storageSnapshot.kind === "s3";
      let setupDraftId = creatingStorage ? storageSetupDraftId : null;
      let setupDraftRevision: number | null = null;
      let setupDraftStorageId: string | null = null;
      agentSetupMarkerRef.current = null;
      setAgentNotice(null);
      if (wasCreatingR2) {
        const session = draftSessionRef.current;
        const editVersion = draftEditVersionRef.current;
        const savedDraft = await persistStorageSetupDraftForAction(
          storageSnapshot,
          session,
          editVersion,
          "finishing setup",
        );
        if (!savedDraft) return;
        setupDraftId = savedDraft.draft_id;
        setupDraftRevision = savedDraft.revision;
        setupDraftStorageId = savedDraft.storage.id;
      }
      setStorageTestVerified(false);
      const saved = await onSaveStorage(
        storageSnapshot,
        baselineSnapshot,
        setupDraftId,
        setupDraftRevision,
        setupDraftStorageId,
      );
      if (saved === false) return;
      // A successful create consumes the draft. Only form state remains.
      if (creatingStorage) resetStorageSetupDraftState();
      setStorageConflict(null);
      if (wasCreatingR2) {
        setStorageDetailsExpanded(false);
        setEditingStorage(null);
        setStorageDraft(null);
        setStorageBaseline(null);
        return;
      }
      const savedStorage = storageSnapshot.kind === "s3"
        ? { ...storageSnapshot, secret_access_key: "" }
        : storageSnapshot;
      if (storageSnapshot.kind === "s3") {
        // The backend retains the submitted secret. Drop the one-time draft
        // value immediately after success; a failed save returns above and
        // deliberately leaves it available for correction and retry.
        setStorageDraft({ ...savedStorage });
        setStorageBaseline({ ...savedStorage });
      }
      setStorageDetailsExpanded(false);
      await loadStorageBundles(savedStorage);
    });

    /* The advisory twin of Create: same gate, same probe, no commit. The
       verdict is only as fresh as the fields it tested, so every edit, save,
       and close clears it. */
    const testStorageConnection = () => run(`test-storage:${editedStorageConfig.id}`, async () => {
      if (!onTestStorage) return;
      agentSetupMarkerRef.current = null;
      setAgentNotice(null);
      setStorageTestVerified(false);
      const storageSnapshot = { ...storageDraft };
      const session = draftSessionRef.current;
      const editVersion = draftEditVersionRef.current;
      const savedDraft = await persistStorageSetupDraftForAction(
        storageSnapshot,
        session,
        editVersion,
        "testing the connection",
      );
      if (!savedDraft) return;
      const verified = await onTestStorage(
        storageSnapshot,
        savedDraft.draft_id,
        savedDraft.revision,
        savedDraft.storage.id,
      );
      if (verified) setStorageTestVerified(true);
    });

    const discardStorageDraftAndClose = () => run(`discard-draft:${editedStorageConfig.id}`, async () => {
      if (!storageSetupDraftId) return;
      const discardImpact = storageDraft.kind === "s3"
        ? "This removes only Mallard’s local setup draft. It does not delete any Cloudflare bucket or access key that may have been created during preparation."
        : "This removes only Mallard’s local setup draft. The selected folder and its contents are not changed.";
      const approved = await confirm(
        `Discard the storage draft for ${storageDraft.name || "(unnamed)"}?\n\n${discardImpact}`,
        { title: "Discard storage draft" },
      );
      if (!approved) return;
      try {
        await projectSyncApi.discardStorageSetupDraft(storageSetupDraftId);
      } catch {
        // The draft row, if it survives, can be discarded from its page.
      }
      resetStorageSetupDraftState();
      onStorageDraftsChanged?.();
      onOpenStorageIndex();
    });

    // The trail starts at the Storages index the crumb leads back to; the
    // storage itself carries the switcher; the section names the page. A
    // draft being created has no section yet and nothing to switch to, so
    // its trail is just the root and the leaf that says what it will be.
    const storagePathCrumbs: PathBarCrumb[] = [
      {
        id: "storages",
        label: "Storages",
        icon: "drive",
        tone: "root",
        title: "All storages",
        ariaLabel: "All storages",
        onActivate: onOpenStorageIndex,
        disabled: busy,
      },
      creatingStorage
        ? { id: "storage", label: "New storage" }
        : {
            id: "storage",
            label: storageName,
            title: `${storageName} · ${storageSubtitle(editedStorageConfig)}`,
            ariaLabel: `${storageName}. Storage menu`,
            // The page below is this storage, so its verbs stay in the page's
            // overflow; the crumb menu is the header naming where you are and
            // the ways to be somewhere else.
            menuHeader: {
              label: storageName,
              detail: storageSubtitle(editedStorageConfig),
              detailIsPath: storageSubtitleIsPath(editedStorageConfig),
              icon: editedStorageConfig.kind === "local" ? "drive" : "cloud",
            },
            menu: [
              ...(storages.some((candidate) => candidate.id !== editedStorageConfig.id) ? [{
                label: "Switch to",
                items: storages
                  .filter((candidate) => candidate.id !== editedStorageConfig.id)
                  .map((candidate) => ({
                    id: candidate.id,
                    label: candidate.name || "(unnamed)",
                    detail: storageSubtitle(candidate),
                    detailIsPath: storageSubtitleIsPath(candidate),
                    icon: candidate.kind === "local" ? ("drive" as const) : ("cloud" as const),
                    disabled: busy,
                    onSelect: () => onOpenStorage(candidate.id, false),
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
    // only app-level commands.
    const storagePathOverflow: PathBarMenuGroup[] = creatingStorage ? appActions : [
      {
        label: "Storage",
        items: [
          ...(editedStorageConfig.kind === "local" && editedStorageConfig.local_dir ? [{
            id: "reveal",
            label: "Reveal in Finder",
            detail: compactProjectPath(editedStorageConfig.local_dir),
            icon: "folder" as const,
            onSelect: () => revealPath(editedStorageConfig.local_dir as string),
          }] : []),
          {
            id: "storage-settings",
            label: storageDetailsExpanded ? "Hide storage settings" : "Storage settings",
            icon: "settings" as const,
            disabled: busy,
            onSelect: () => setStorageDetailsExpanded((current) => !current),
          },
          {
            id: "rename",
            label: "Rename storage…",
            icon: "edit" as const,
            disabled: busy,
            onSelect: () => {
              setStorageNameDraft(storageDraft.name);
              setRenamingStorageId(storageDraft.id);
            },
          },
          {
            id: "clear",
            label: "Clear storage data…",
            icon: "trash" as const,
            danger: true,
            disabled: busy,
            onSelect: () => onClearStorage(editedStorageConfig.id),
          },
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
            onSelect: () => onRemoveStorage(editedStorageConfig.id),
          },
        ],
      },
      ...appActions,
    ];

    // Creation uses the form's commit bar. A settled storage exposes Refresh
    // in the path bar.
    const storagePathActions = creatingStorage ? undefined : (
      <button
        type="button"
        className="sync-path-sync"
        disabled={busy || bundleLoading}
        title={`Refresh repositories in ${storageName}`}
        aria-label={`Refresh repositories in ${storageName}`}
        onClick={() => void loadStorageBundles(editedStorageConfig)}
      >
        <Icon name="refresh" size={14} className={bundleLoading ? "icon-spin" : undefined} />
        <span className="sync-path-sync-label">Refresh</span>
      </button>
    );

    /* One box takes the loudest storage-editor, repository, or conflict
     * notice. The conflict appears only inside the expanded panel. */
    const storageNotices: NoticeItem[] = [];
    if (showStorageDetails && storageConflict) {
      storageNotices.push({
        key: "storage-changed-externally",
        severity: "warning",
        message: `This storage was ${storageConflict === "removed" ? "removed" : "changed"} outside this editor, so this draft can no longer be saved. Loading the current configuration discards the draft.`,
        action: (
          <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => void reloadExternalStorage()}>
            Load current
          </button>
        ),
      });
    }
    if (storageDraftActionError) {
      storageNotices.push({
        key: "storage-draft-save-failed",
        severity: "danger",
        message: storageDraftActionError,
      });
    }
    if (!creatingStorage && !bundleLoading && bundleError) {
      storageNotices.push({ key: "repository-list-failed", severity: "danger", message: bundleError });
    }
    if (!creatingStorage && !bundleLoading && !bundleError && catalogRepoError) {
      storageNotices.push({ key: "catalog-repositories-failed", severity: "danger", message: catalogRepoError });
    }
    if (revealError) {
      storageNotices.push({ key: "reveal-failed", severity: "danger", message: revealError });
    }
    if (error && !renamingStorage) {
      storageNotices.push({ key: "storage-action-failed", severity: "danger", message: error });
    }
    /* Standing by default: both read failures are set by the load this page
     * starts as it opens, and `error` arrives as a prop, so all three are on
     * screen the moment the view renders and must not announce. The two that
     * ARE events re-time the whole box — a reveal the user asked for came
     * back refused, and the conflict lands from the config watcher while the
     * draft is open and stops it from ever being saved. */
    const storageNoticeTiming: NoticeTiming = storageDraftActionError
      || revealError
      || (showStorageDetails && storageConflict)
      ? { timing: "blocking" }
      : { timing: "standing", label: "Problems with this storage" };

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
              <button type="submit" className="btn btn-primary" disabled={busy || !storageNameChanged || !!storageConflict}>
                {runningAction === `rename-storage:${storageDraft.id}` ? "Saving…" : "Save"}
              </button>
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => {
                  setStorageNameDraft(storageDraft.name);
                  setRenamingStorageId(null);
                }}
              >
                Cancel
              </button>
              {error && (
                <div className="sync-path-rename-error" role="alert">
                  <Icon name="alert-triangle" size={13} /> {error}
                </div>
              )}
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
              {showStorageDetails && (
                <StorageEditor
                  /* Identity, so every per-storage ref inside the editor —
                     the R2 jurisdiction, the S3 API URL draft and its
                     editing flag — resets when the storage does. Without it
                     a new R2 draft inherited the previous storage's
                     jurisdiction until the endpoint happened to parse. */
                  key={storageDraft.id}
                  storage={storageDraft}
                  disabled={busy || !!runningAction || !!storageConflict}
                  onChange={(nextDraft) => {
                    // An edit orphans the copied instruction, so the marker
                    // dies — and with it the row's claim to be listening.
                    draftEditVersionRef.current += 1;
                    agentSetupMarkerRef.current = null;
                    setAgentNotice((current) => (
                      current?.kind === "listening" || current?.kind === "launched" || current?.kind === "ready"
                        ? null
                        : current
                    ));
                    // The same edit stales a Test verdict and, on the setup
                    // page, arms the draft autosave.
                    setStorageTestVerified(false);
                    setStorageDraftActionError(null);
                    if (creatingStorage) draftDirtyRef.current = true;
                    setStorageDraft(nextDraft);
                  }}
                  secretStored={secretStored}
                  showNameField={creatingStorage}
                  showProgress={!creatingStorage}
                  agentSlot={storageDraft.kind !== "s3" ? undefined
                    : agentNotice?.kind === "launched" ? (
                      <>
                        <span className="sync-r2-guide-wait" role="status">
                          <span className="status-loader" /> {agentNotice.agent} is running the setup — finish Cloudflare sign-in when the browser opens…
                        </span>
                        <button type="button" className="btn-link" onClick={cancelAgentSetup}>Cancel</button>
                      </>
                    )
                    : agentNotice?.kind === "listening" ? (
                      <>
                        <span className="sync-r2-guide-connected" role="status">
                          <Icon name="check-circle" size={13} /> Copied.
                        </span>
                        <span className="sync-r2-guide-wait" role="status">
                          <span className="status-loader" /> Listening for your agent…
                        </span>
                        <button
                          type="button"
                          className={copyAgainFlashed
                            ? "btn sync-r2-guide-btn sync-r2-copied-flash"
                            : "btn sync-r2-guide-btn"}
                          onClick={() => void copyAgentInstructionAgain()}
                        >
                          <Icon name={copyAgainFlashed ? "check" : "copy"} size={13} /> {copyAgainFlashed ? "Copied" : "Copy again"}
                        </button>
                        <button type="button" className="btn-link" onClick={cancelAgentSetup}>Cancel</button>
                      </>
                    )
                    : agentNotice?.kind === "ready" ? (
                      <span className="sync-r2-guide-connected" role="status">
                        <Icon name="check-circle" size={13} /> {agentNotice.message}
                      </span>
                    )
                    : agentNotice?.kind === "error" ? (
                      <>
                        <span className="sync-r2-guide-error" role="alert">{agentNotice.message}</span>
                        {creatingStorage && (
                          <>
                            <button
                              type="button"
                              className="btn sync-r2-guide-btn"
                              disabled={busy || !storageDraft.name.trim()}
                              onClick={() => void copyAgentSetupInstruction()}
                            >
                              <Icon name="copy" size={13} /> Copy agent instruction
                            </button>
                            <button type="button" className="btn-link" onClick={cancelAgentSetup}>Try again</button>
                          </>
                        )}
                      </>
                    )
                    : creatingStorage ? (
                      defaultAgentLauncher ? (
                        <div className="sync-r2-split" ref={agentSplitRef}>
                          <button
                            type="button"
                            className="btn sync-r2-guide-btn sync-r2-agent-cta"
                            disabled={busy || !storageDraft.name.trim()}
                            title={`Opens ${defaultAgentLauncher.launcher.display_name} in a terminal window with the setup instruction as its prompt`}
                            onClick={() => void launchAgentSetup(defaultAgentLauncher.key, defaultAgentLauncher.launcher.display_name)}
                          >
                            <ProviderMark
                              provider={defaultAgentLauncher.key}
                              displayName={defaultAgentLauncher.launcher.display_name}
                              size={13}
                            /> Run with {defaultAgentLauncher.launcher.display_name}
                          </button>
                          <button
                            type="button"
                            className="btn sync-r2-guide-btn sync-r2-agent-cta sync-r2-split-arrow"
                            disabled={busy || !storageDraft.name.trim()}
                            aria-haspopup="menu"
                            aria-expanded={agentMenuOpen}
                            aria-label="More agent setup routes"
                            onClick={() => setAgentMenuOpen((open) => !open)}
                          >
                            <Icon name="chevron-down" size={11} />
                          </button>
                          {agentMenuOpen && (
                            <div className="sync-path-menu end" role="menu">
                              {agentLaunchers
                                .filter(({ key }) => key !== defaultAgentLauncher.key)
                                .map(({ key, launcher }) => (
                                  <button
                                    key={key}
                                    type="button"
                                    className="sync-path-menu-item"
                                    role="menuitem"
                                    onClick={() => {
                                      setPreferredLauncher(key);
                                      void launchAgentSetup(key, launcher.display_name);
                                    }}
                                  >
                                    <ProviderMark provider={key} displayName={launcher.display_name} size={13} />
                                    <span className="sync-r2-menu-text">
                                      Run with {launcher.display_name}
                                      <small>Opens a terminal window with the instruction as its prompt</small>
                                    </span>
                                  </button>
                                ))}
                              <button
                                type="button"
                                className="sync-path-menu-item"
                                role="menuitem"
                                onClick={() => void copyAgentSetupInstruction()}
                              >
                                <Icon name="copy" size={13} />
                                <span className="sync-r2-menu-text">
                                  Copy agent instruction
                                  <small>Paste into any other coding agent yourself</small>
                                </span>
                              </button>
                            </div>
                          )}
                        </div>
                      ) : (
                        <button
                          type="button"
                          className="btn sync-r2-guide-btn sync-r2-agent-cta"
                          disabled={busy || !storageDraft.name.trim()}
                          onClick={() => void copyAgentSetupInstruction()}
                        >
                          <Icon name="copy" size={13} /> Copy agent instruction
                        </button>
                      )
                    ) : undefined}
                  // A draft's create is the page's verb and lives in the
                  // commit bar below; a settled storage's save belongs to the
                  // drawer it commits.
                  primaryAction={creatingStorage ? undefined : (
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={busy || !!storageConflict || !storageReady || (!!storageBaseline && sameStorage(storageDraft, storageBaseline))}
                      onClick={() => void saveEditedStorage()}
                    >
                      {runningAction === `storage:${editedStorageConfig.id}` ? "Saving…" : "Save storage"}
                    </button>
                  )}
                />
              )}
            </div>

            {!creatingStorage && (
              <section className="sync-storage-repositories" aria-labelledby="storage-repositories-heading">
                {/* Refresh is the page's verb and lives in the path bar; the
                    card heading only names what it frames. */}
                <div className="sync-storage-repositories-heading">
                  <h2 id="storage-repositories-heading">Repositories</h2>
                </div>

                {bundleLoading ? (
                  <div className="sync-storage-repository-state"><span className="status-loader" /> Loading repositories…</div>
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

            {/* The draft's verbs close its column, under the fields they act
                on. Cancel is the crumb's way back said plainly beside the
                commit; anything typed keeps living in the autosaved draft,
                which waits on the Storages index until finished or
                discarded. */}
            {creatingStorage && (
              <footer className="sync-storage-setup-footer">
                {storageDraft.kind !== "local" && (
                  <span className="sync-storage-editor-progress" aria-live="polite">
                    {r2FieldsSet(storageDraft, secretStored)} of 5 set
                  </span>
                )}
                {storageTestVerified && (
                  <span className="sync-r2-guide-connected" role="status">
                    <Icon name="check-circle" size={13} /> Connection verified
                  </span>
                )}
                <div>
                  {storageSetupDraftId && (
                    <button
                      type="button"
                      className="btn btn-ghost"
                      disabled={busy}
                      onClick={() => void discardStorageDraftAndClose()}
                    >
                      Discard draft…
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn-ghost"
                    disabled={busy}
                    onClick={onOpenStorageIndex}
                    aria-label={storageSetupDraftId ? "Close setup; the draft is kept" : "Close setup"}
                  >
                    Cancel
                  </button>
                  {storageDraft.kind === "s3" && (
                    <button
                      type="button"
                      className="btn"
                      disabled={busy || !!storageConflict || !storageReady || !!runningAction}
                      onClick={() => void testStorageConnection()}
                    >
                      {runningAction === `test-storage:${editedStorageConfig.id}` ? "Testing…" : "Test connection"}
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={busy || !!storageConflict || !storageReady || !!runningAction}
                    onClick={() => void saveEditedStorage()}
                  >
                    {storageDraft.kind === "s3"
                      ? runningAction === `storage:${editedStorageConfig.id}` ? "Finishing…" : "Finish setup"
                      : runningAction === `storage:${editedStorageConfig.id}` ? "Creating…" : "Create storage"}
                  </button>
                </div>
              </footer>
            )}
          </div>
        </section>
      </main>
      </>
    );
  }

  /* What the R2 agent handover left behind, said once on each of the two
   * screens that outlive the editor. Both surviving kinds are receipts — the
   * CLI's draft is ready for user review, or the copy itself failed — so the
   * non-error one is `success`, which is also the check-circle the row draws.
   * Standing, not an event: neither screen can set `agentNotice`, because the
   * copy button and the adoption effect both need the storage editor open and
   * both of these render only once it is closed. By the time it is read here
   * it is content the view already came with, so it must not announce. */
  const agentSetupNotices: NoticeItem[] = agentNotice
    && agentNotice.kind !== "listening"
    && agentNotice.kind !== "launched"
    ? [{
        key: "agent-setup",
        severity: agentNotice.kind === "error" ? "danger" : "success",
        message: agentNotice.message,
      }]
    : [];

  if (!workspaceProject && !editingStorage && !newProjectSetup) {
    return (
      <main className="sync-main sync-project-links-page sync-git-info-page">
        <section className="profile-links-section" aria-labelledby="git-info-heading">
          <div className="profile-links-heading">
            <div className="profile-links-copy">
              <h1 id="git-info-heading" className="settings-section-title">Project history</h1>
              <div className="profile-links-subtitle">Select a completed project to view its Codex history.</div>
            </div>
          </div>
          <NoticeZone
            items={agentSetupNotices}
            subject="notes about storage setup"
            timing="standing"
            label="Notes about storage setup"
          />
        </section>
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
        storage.id !== workspaceStorage.id
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
                () => onUnlinkStorage(workspaceProject.local_project_id, workspaceStorage.id),
              ),
            },
          ],
        },
        ...(otherStorages.length > 0 ? [{
          label: "Switch to",
          items: otherStorages.map((storage) => ({
            id: storage.id,
            label: storage.name || "(unnamed)",
            detail: storageSubtitle(storage),
            detailIsPath: storageSubtitleIsPath(storage),
            icon: storage.kind === "local" ? ("drive" as const) : ("cloud" as const),
            disabled: workspaceControlsLocked,
            onSelect: () => {
              if (workspaceReviewOpen) inlineStorageReview?.onClose();
              void onSelectStorage(workspaceProject.local_project_id, storage.id);
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
                setLinkingProjectId(workspaceProject.local_project_id);
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
        icon: workspaceStorage.kind === "local" ? "drive" : "cloud",
        tone: "root",
        title: `${workspaceStorage.name || "Storage"} · ${storageSubtitle(workspaceStorage)}`,
        ariaLabel: `${workspaceStorage.name || "Unnamed storage"}. Storage menu`,
        menuHeader: {
          label: workspaceStorage.name || "(unnamed)",
          detail: storageSubtitle(workspaceStorage),
          detailIsPath: storageSubtitleIsPath(workspaceStorage),
          icon: workspaceStorage.kind === "local" ? "drive" : "cloud",
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
              title: "Forget local sync bases, conversation mappings, recovery records, and saved plans; files and cloud data are kept",
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
        // A session replaces this crumb outright, and the branch goes with it:
        // the branch scopes the list, and inside a session there is no list.
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
          || workspacePathBlocked
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
            () => onPull(workspaceProject.local_project_id, workspaceStorage.id),
          );
        }}
        title={workspacePathTitle
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
          || workspacePathBlocked
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
            () => onPush(workspaceProject.local_project_id, workspaceStorage.id),
          );
        }}
        title={workspacePathTitle
          ?? workspaceProfileIssue
          ?? (workspacePushReviewOpen
            ? "Push the selected resources to this storage"
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

  /* The workspace's own two messages, in one box above the page. A refused
   * reveal is an event — the user picked it out of a crumb menu a moment ago —
   * and it re-times the box; the setup receipt beside it is standing, for the
   * reason spelled out where `agentSetupNotices` is built. */
  const workspaceNotices: NoticeItem[] = [];
  if (revealError) {
    workspaceNotices.push({ key: "reveal-failed", severity: "danger", message: revealError });
  }
  workspaceNotices.push(...agentSetupNotices);
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
                .map((link) => storages.find((storage) => storage.id === link.storage_id))
                .filter((storage): storage is StorageConfig => !!storage);
              const selectedStorage = linkedStorages.find((storage) => (
                project.local_project_id === activeProjectId && storage.id === activeStorageId
              )) ?? linkedStorages[0] ?? null;
              const availableStorages = storages.filter((storage) => (
                !projectLinks.some((link) => link.storage_id === storage.id)
              ));
              const projectBinding = bindingByProject.get(project.local_project_id);
              const bindingActive = projectBinding?.state === "active";
              const projectProfiles = Object.entries(projectBinding?.profile_ids ?? {}).map(
                ([provider, profileId]) => ({
                  provider,
                  profile: profiles.find((candidate) => candidate.profile_id === profileId),
                }),
              );
              // Codex conversation-path repair is provider-specific
              // maintenance; it appears exactly when a Codex profile is bound.
              const codexProfile = projectProfiles.find(({ provider }) => provider === "codex")?.profile;
              const codexConfigured = bindingActive && !!codexProfile;
              const conversationPathAudit = conversationPathAudits[project.local_project_id];
              const conversationPathAuditError = conversationPathAuditErrors[project.local_project_id];
              const conversationPathNoticeBlocked = !!conversationPathAuditError
                || !!conversationPathAudit?.blockers.length;
              const conversationPathNotice = codexConfigured
                && !conversationPathAuditLoading
                && shouldShowConversationPathRepairNotice(
                  conversationPathAudit,
                  conversationPathAuditError,
                ) ? (
                <ConversationPathRepairNotice
                  audit={conversationPathAudit}
                  auditError={conversationPathAuditError}
                  projectName={projectLabel(project)}
                  profileName={codexProfile?.display_name ?? "Codex"}
                  profilePath={codexProfile?.path ?? conversationPathAudit?.profile_path}
                  showScope={false}
                  busy={busy || runningAction === `repair-paths:${project.local_project_id}`}
                  onRepair={() => run(
                    `repair-paths:${project.local_project_id}`,
                    () => onRepairConversationPaths(project.local_project_id),
                  )}
                />
              ) : null;
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
              // count chip and icon-button vocabulary as Uncommitted below it,
              // so the two card headings cannot drift apart.
              const storageHeading = (
                <div className="sync-history-section-heading">
                  <h2><Icon name="link" size={14} />Storage</h2>
                  {!conversationPathNoticeBlocked && conversationPathNotice}
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
              const storageGroupNotice = (conversationPathNoticeBlocked || selectedStorage)
                && conversationPathNotice;
              const storageBlockShown = !!selectedStorage
                && (!!(storageNotice
                  && storageNotice.projectId === project.local_project_id
                  && storageNotice.storageId === selectedStorage.id)
                  || !!(siblingNotice
                    && siblingNotice.projectId === project.local_project_id
                    && siblingNotice.storageId === selectedStorage.id
                    && siblingNotice.status.state !== "none")
                  || (inlineStorageReview?.projectId === project.local_project_id
                    && inlineStorageReview.storageId === selectedStorage.id));
              const storageGroupEmpty = !storageGroupNotice
                && !!selectedStorage
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
                      {storageGroupNotice}

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
                        && inlineStorageReview.storageId === storage.id;
                      const actionPrefix = `${project.local_project_id}:${storage.id}`;
                      const reviewPanelId = `storage-review-${actionPrefix.replace(/[^a-z0-9_-]/gi, "-")}`;
                      const activeNotice = storageNotice
                        && storageNotice.projectId === project.local_project_id
                        && storageNotice.storageId === storage.id
                        ? storageNotice
                        : null;
                      const noticeActionsDisabled = busy || !!runningAction;
                      const activeSibling = siblingNotice
                        && siblingNotice.projectId === project.local_project_id
                        && siblingNotice.storageId === storage.id
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
                                onClick={() => void run(`pull:${actionPrefix}`, () => onPull(project.local_project_id, storage.id))}
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
                                onClick={() => onOpenStorage(storage.id, true)}
                              >
                                Storage settings
                              </button>
                              <button
                                type="button"
                                className="btn btn-ghost danger"
                                disabled={noticeActionsDisabled}
                                onClick={() => void run(`unlink:${actionPrefix}`, () => onUnlinkStorage(project.local_project_id, storage.id))}
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
                                onClick={() => void run(`unlink:${actionPrefix}`, () => onUnlinkStorage(project.local_project_id, storage.id))}
                              >
                                Unlink storage…
                              </button>
                              <button
                                type="button"
                                className="btn btn-ghost"
                                disabled={noticeActionsDisabled}
                                onClick={() => void run(`pull:${actionPrefix}`, () => onPull(project.local_project_id, storage.id))}
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
                              {isIrrecoverableSyncError(failedMessage) ? (
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
                                    () => (failedAction === "push" ? onPush : onPull)(project.local_project_id, storage.id),
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
                          key={storage.id}
                          className={`storage-link-block selected${reviewOpen ? " sync-review-open" : ""}`}
                          aria-busy={reviewOpen && busy}
                        >
                          {activeSibling && (
                            <SiblingRepoNotice
                              status={activeSibling}
                              busy={noticeActionsDisabled}
                              onConnect={(matches) => onConnectSibling?.(
                                project.local_project_id,
                                storage.id,
                                matches,
                              )}
                            />
                          )}
                          {activeNotice && activeNotice.kind === "outcome" && (
                            <SyncOutcomeNotice
                              outcome={activeNotice.outcome}
                              busy={noticeActionsDisabled}
                              onAction={() => onStorageNoticeAction?.(activeNotice.outcome)}
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
                        <div className="storage-link-picker">
                          <span>Choose a storage destination</span>
                          {availableStorages.map((storage) => (
                            <button
                              key={storage.id}
                              type="button"
                              disabled={busy}
                              onClick={() => void run(`link:${project.local_project_id}:${storage.id}`, async () => {
                                await onLinkStorage(project.local_project_id, storage.id);
                                setLinkingProjectId(null);
                              })}
                            >
                              <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={15} />
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
                activeStorageName={storages.find((storage) => storage.id === activeStorageId)?.name ?? null}
                refreshEpoch={historyRefreshEpoch}
                onHeaderModelChange={handleFilesActivityHeaderChange}
                onRevealPath={revealPath}
              />
            ) : (
              <ProjectActivityPage
                project={activeProject}
                binding={activeBindingByProject.get(activeProject.local_project_id) ?? null}
                refreshEpoch={historyRefreshEpoch}
                activeStorageId={activeStorageId}
                activeStorageName={storages.find((storage) => storage.id === activeStorageId)?.name ?? null}
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
          />
        </Suspense>
      )}

    </main>
    </>
  );
}
