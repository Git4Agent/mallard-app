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
import { openPath } from "@tauri-apps/plugin-opener";
import type {
  BundleFetchProgress,
  BundlePage,
  CodexConversationPathAudit,
  LocalProjectSummary,
  ProjectBinding,
  ProjectStorageLink,
  ProviderProfileSummary,
  RemoteBundleSummary,
  StorageConfig,
} from "../../types";
import Icon from "../Icons";
import ProjectActivityPage, { BranchPicker, type ActivityHeaderModel } from "./ProjectActivityPage";
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
import SyncOutcomeNotice from "../sync-review/SyncOutcomeNotice";
import { syncOutcomeView } from "./syncOutcome";
import { isIrrecoverableSyncError } from "./syncError";
import type { SyncOutcome } from "./syncOutcome";
import { newStorage, storageConfigReady, StorageEditor } from "./StorageSettings";
import { projectSyncApi } from "./api";
import {
  compactProjectPath,
  formatRelativeTime,
  projectLabel,
  providerLabel,
  storageAccessErrorMessage,
} from "./model";

type LinkKey = { projectId: string; storageId: string };
export type StorageReviewKind = "pull" | "push";
type StorageEditorRequest =
  | { mode: "open"; storageId: string; settingsExpanded: boolean; requestId: number }
  | { mode: "create"; storageKind: "local" | "s3"; requestId: number }
  | { mode: "close"; requestId: number };
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
  storages: StorageConfig[];
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
  onConfigureBinding: (projectId: string) => Promise<void> | void;
  onRepairConversationPaths: (projectId: string) => Promise<void> | void;
  onRenameProject: (projectId: string, alias: string | null) => Promise<boolean> | boolean;
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
  ) => Promise<boolean | void> | boolean | void;
  storageEditorRequest?: StorageEditorRequest | null;
  onStorageEditorRequestHandled?: () => void;
  onStorageEditorChange?: (storageId: string | null) => void;
  onStorageIssueChange?: (storageId: string, issue: string | null) => void;
  newProjectSetup?: ReactNode;
  historyRefreshEpoch?: number;
  inlineStorageReview?: InlineStorageReview | null;
  storageNotice?: StorageNotice | null;
  onStorageNoticeAction?: (outcome: SyncOutcome) => void;
  onDismissStorageNotice?: () => void;
}

const SessionDetailPage = lazy(() => import("./session-detail/SessionDetailPage"));

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

const MAX_AGENT_SETUP_STORAGE_IDS = 512;

export type AgentSetupMarker = {
  draftFingerprint: string;
  editingDraftId: string;
  requestedName: string;
  storageIds: readonly string[];
  storageIdsComplete: boolean;
};

export type AgentSetupReconciliation =
  | { kind: "adopt"; storage: StorageConfig }
  | { kind: "preserve"; reason: "draft_changed" | "no_match" | "ambiguous" };

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
  storages: readonly StorageConfig[],
  editingDraftId: string,
): AgentSetupMarker {
  return {
    draftFingerprint: storageDraftFingerprint(draft),
    editingDraftId,
    requestedName: normalizedStorageName(draft.name),
    storageIds: storages.slice(0, MAX_AGENT_SETUP_STORAGE_IDS).map((storage) => storage.id),
    storageIdsComplete: storages.length <= MAX_AGENT_SETUP_STORAGE_IDS,
  };
}

/** Decide whether a CLI-created storage safely completes the open setup. */
export function reconcileAgentSetup(
  marker: AgentSetupMarker,
  draft: StorageConfig,
  editingDraftId: string,
  storages: readonly StorageConfig[],
): AgentSetupReconciliation {
  if (
    marker.editingDraftId !== editingDraftId
    || marker.draftFingerprint !== storageDraftFingerprint(draft)
  ) {
    return { kind: "preserve", reason: "draft_changed" };
  }
  if (!marker.storageIdsComplete) return { kind: "preserve", reason: "ambiguous" };

  const priorIds = new Set(marker.storageIds);
  const candidates = storages.filter((storage) => (
    !priorIds.has(storage.id)
    && storage.kind === "s3"
    && normalizedStorageName(storage.name) === marker.requestedName
  ));
  if (candidates.length === 0) return { kind: "preserve", reason: "no_match" };
  if (candidates.length > 1) return { kind: "preserve", reason: "ambiguous" };
  return { kind: "adopt", storage: candidates[0] };
}

export type AgentSetupEditorAdoption = {
  editingStorage: LinkKey;
  storageDraft: StorageConfig;
  storageBaseline: StorageConfig;
  storageDetailsExpanded: true;
  status: string;
};

/** State transition that keeps the setup surface open with committed values. */
export function agentSetupEditorAdoption(storage: StorageConfig): AgentSetupEditorAdoption {
  return {
    editingStorage: { projectId: "", storageId: storage.id },
    storageDraft: { ...storage },
    storageBaseline: { ...storage },
    storageDetailsExpanded: true,
    status: `Connected ${storage.name || "Cloudflare R2"}. Settings filled in automatically.`,
  };
}

export function buildR2AgentSetupInstruction({
  storageName,
  cliPath,
}: {
  storageName: string;
  cliPath: string;
}): string {
  return [
    `Set up the Cloudflare R2 storage “${storageName.trim() || "Cloudflare R2"}” in Mallard.`,
    "",
    "Use this Mallard CLI:",
    cliPath,
    "",
    "Treat that as a literal executable path and quote it for the current shell. Run its R2 help first, then either connect an existing bucket or provision a new one. Finish through the CLI; do not edit Mallard’s open setup form or JSON, put a secret in an argument or environment variable, or print credentials. Let me enter the Secret Access Key or Cloudflare bootstrap token through the CLI's hidden prompt. Mallard is listening for the committed storage and will automatically populate the open setup page when the CLI finishes. Finish by verifying the storage and confirming it appears in the running app.",
  ].join("\n");
}

async function writeClipboardText(value: string): Promise<void> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // Some desktop WebViews expose Clipboard but reject it. Continue under
      // the same user gesture with a document-local copy operation.
    }
  }
  if (typeof document === "undefined") throw new Error("Clipboard access is unavailable");
  const field = document.createElement("textarea");
  field.value = value;
  field.readOnly = true;
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.append(field);
  field.select();
  const copied = document.execCommand("copy");
  field.remove();
  if (!copied) throw new Error("Clipboard access is unavailable");
}

function absoluteTime(epoch: number): string {
  // Same second/millisecond tolerance as formatRelativeTime, since the fold's
  // absolute stamp expands the very value the chip shows relatively.
  const milliseconds = epoch > 10_000_000_000 ? epoch : epoch * 1000;
  return new Date(milliseconds).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
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
          <dd>{absoluteTime(bundle.updated_at)}</dd>
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
  // With the wizard gone, the card button on an open review IS the push: the
  // first click opens the review, the second publishes the selection.
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
  storages,
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
  onClearStorage,
  onRemoveStorage,
  onSaveStorage,
  storageEditorRequest,
  onStorageEditorRequestHandled,
  onStorageEditorChange,
  onStorageIssueChange,
  newProjectSetup,
  historyRefreshEpoch = 0,
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
  const [agentCopyStatus, setAgentCopyStatus] = useState<string | null>(null);
  const [storageDetailsExpanded, setStorageDetailsExpanded] = useState(false);
  const [renamingStorageId, setRenamingStorageId] = useState<string | null>(null);
  const [storageNameDraft, setStorageNameDraft] = useState("");
  const [bundlePage, setBundlePage] = useState<BundlePage | null>(null);
  const [bundleLoading, setBundleLoading] = useState(false);
  const [bundleError, setBundleError] = useState<string | null>(null);
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
  const bundleRequestRef = useRef(0);
  const storageSettingsRef = useRef<HTMLDivElement>(null);
  const workspaceScrollRef = useRef<HTMLElement>(null);
  const overviewScrollPositionRef = useRef(0);
  const sessionDetailOriginRef = useRef<HTMLButtonElement | null>(null);
  const agentSetupMarkerRef = useRef<AgentSetupMarker | null>(null);

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

  useEffect(() => {
    onStorageEditorChange?.(editingStorage?.storageId ?? null);
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
  const orderedRemoteBundles = [...(bundlePage?.bundles ?? [])].sort((left, right) => {
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
    setBundlePage(null);
    setBundleError(null);
    setBundleLoading(true);
    try {
      const bundles = await projectSyncApi.listRemoteBundleSnapshots(storageId);
      if (bundleRequestRef.current === requestId) {
        setBundlePage({ bundles });
        onStorageIssueChange?.(storageId, null);
      }
    } catch (reason) {
      if (bundleRequestRef.current === requestId) {
        const issue = storageAccessErrorMessage(storage, reason);
        setBundleError(issue);
        onStorageIssueChange?.(storageId, issue);
      }
    } finally {
      if (bundleRequestRef.current === requestId) setBundleLoading(false);
    }
  };

  useEffect(() => {
    if (!storageEditorRequest) return;
    agentSetupMarkerRef.current = null;
    if (storageEditorRequest.mode === "close") {
      bundleRequestRef.current += 1;
      setEditingStorage(null);
      setStorageDraft(null);
      setStorageBaseline(null);
      setStorageConflict(null);
      setAgentCopyStatus(null);
      setStorageDetailsExpanded(false);
      setBundlePage(null);
      setBundleError(null);
      setBundleLoading(false);
      onStorageEditorRequestHandled?.();
      return;
    }
    if (storageEditorRequest.mode === "create") {
      const storage = newStorage(storageEditorRequest.storageKind, storages.length + 1);
      bundleRequestRef.current += 1;
      setEditingStorage({ projectId: "", storageId: storage.id });
      setStorageDraft(storage);
      setStorageBaseline(null);
      setStorageConflict(null);
      setAgentCopyStatus(null);
      setStorageDetailsExpanded(true);
      setBundlePage(null);
      setBundleError(null);
      setBundleLoading(false);
      onStorageEditorRequestHandled?.();
      return;
    }
    const storage = storages.find((candidate) => candidate.id === storageEditorRequest.storageId);
    if (!storage) {
      onStorageEditorRequestHandled?.();
      return;
    }

    if (editingStorage?.storageId !== storage.id) {
      setEditingStorage({ projectId: "", storageId: storage.id });
      setStorageDraft({ ...storage });
      setStorageBaseline({ ...storage });
      setStorageConflict(null);
      setAgentCopyStatus(null);
      void loadStorageBundles(storage);
    }
    setStorageDetailsExpanded(storageEditorRequest.settingsExpanded);
    onStorageEditorRequestHandled?.();
  }, [storageEditorRequest?.requestId]);

  const openStorageEditorWithSettings = async (storage: StorageConfig) => {
    const switchingStorage = editingStorage?.storageId !== storage.id;
    if (switchingStorage) {
      agentSetupMarkerRef.current = null;
      setEditingStorage({ projectId: "", storageId: storage.id });
      setStorageDraft({ ...storage });
      setStorageBaseline({ ...storage });
      setStorageConflict(null);
      setAgentCopyStatus(null);
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
      storages,
    );
    if (reconciliation.kind === "preserve") {
      if (reconciliation.reason === "draft_changed" || reconciliation.reason === "ambiguous") {
        agentSetupMarkerRef.current = null;
      }
      return;
    }

    agentSetupMarkerRef.current = null;
    const adoption = agentSetupEditorAdoption(reconciliation.storage);
    setEditingStorage(adoption.editingStorage);
    setStorageDraft(adoption.storageDraft);
    setStorageBaseline(adoption.storageBaseline);
    setStorageConflict(null);
    setStorageDetailsExpanded(adoption.storageDetailsExpanded);
    setBundlePage(null);
    setBundleError(null);
    setAgentCopyStatus(adoption.status);
    void loadStorageBundles(reconciliation.storage);
  }, [storages]);

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

  const copyAgentSetupInstruction = async () => {
    if (!storageDraft || storageDraft.kind !== "s3") return;
    setAgentCopyStatus(null);
    try {
      const cliPath = await projectSyncApi.getMallardCliPath();
      const instruction = buildR2AgentSetupInstruction({
        storageName: storageDraft.name,
        cliPath,
      });
      await writeClipboardText(instruction);
      agentSetupMarkerRef.current = createAgentSetupMarker(
        storageDraft,
        storages,
        storageDraft.id,
      );
      setAgentCopyStatus("Agent setup instruction copied.");
    } catch (reason) {
      setAgentCopyStatus(`Could not copy agent setup instructions: ${String(reason)}`);
    }
  };

  const activeProject = projects.find((project) => project.local_project_id === activeProjectId) ?? null;
  const workspaceProject = newProjectSetup ? null : activeProject;
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
  // Everything the header used to spread across an h1, a meta row and a
  // storage card is derived once here, because the bar renders above the
  // scrolling page rather than inside it.
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
  // git repo and Sessions otherwise; the crumb reads the same fact off the
  // project so the trail is right before the page has finished loading.
  const workspaceSectionLabel = workspaceProject?.is_git_repository ? "Git history" : "Sessions";
  const [sessionHeader, setSessionHeader] = useState<SessionHeaderModel | null>(null);
  const handleSessionHeaderChange = useCallback((header: SessionHeaderModel | null) => {
    setSessionHeader(header);
  }, []);

  const saveProjectName = async () => {
    if (!workspaceProject || !projectNameChanged) return;
    let saved = false;
    await run(
      `rename:${workspaceProject.local_project_id}`,
      async () => {
        saved = await onRenameProject(workspaceProject.local_project_id, proposedProjectAlias);
      },
    );
    if (saved) setRenamingProjectId(null);
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
      setStorageNameDraft(name);
      setRenamingStorageId(null);
    });
  };

  if (editingStorage && storageDraft && editedStorageConfig) {
    const storageReady = storageConfigReady(storageDraft);
    const showStorageDetails = creatingStorage || storageDetailsExpanded;
    const storageDetailsId = `storage-${storageDraft.id}-settings`;
    const storageNameChanged = !!storageNameDraft.trim() && storageNameDraft.trim() !== storageDraft.name;
    const storageName = storageDraft.name || "(unnamed)";
    const storageLinkCount = links.filter((link) => link.storage_id === editedStorageConfig.id).length;
    const renamingStorage = renamingStorageId === storageDraft.id;
    const saveEditedStorage = () => run(`storage:${editedStorageConfig.id}`, async () => {
      const wasCreatingR2 = creatingStorage && storageDraft.kind === "s3";
      agentSetupMarkerRef.current = null;
      setAgentCopyStatus(null);
      const saved = await onSaveStorage(storageDraft, storageBaseline);
      if (saved === false) return;
      setStorageConflict(null);
      if (wasCreatingR2) {
        setStorageDetailsExpanded(false);
        setEditingStorage(null);
        setStorageDraft(null);
        setStorageBaseline(null);
        return;
      }
      setStorageDetailsExpanded(false);
      await loadStorageBundles(storageDraft);
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

    // Everything the old header held as its own button — the gear, the
    // pencil, the ✕ — plus the storage verbs the sidebar keeps as shortcuts.
    // A draft has none of these; it gets the app scope alone.
    const storagePathOverflow: PathBarMenuGroup[] = creatingStorage ? appActions : [
      {
        label: "Storage",
        items: [
          ...(editedStorageConfig.kind === "local" && editedStorageConfig.local_dir ? [{
            id: "reveal",
            label: "Reveal in Finder",
            detail: compactProjectPath(editedStorageConfig.local_dir),
            icon: "folder" as const,
            onSelect: () => { void openPath(editedStorageConfig.local_dir as string); },
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

    // Creating, the page's one verb is the save; settled, it is the refresh
    // that the repositories card no longer duplicates.
    const storagePathActions = creatingStorage ? (
      <button
        type="button"
        className="sync-path-sync"
        disabled={busy || !!storageConflict || !storageReady}
        onClick={() => void saveEditedStorage()}
      >
        <Icon name="check-circle" size={14} />
        <span className="sync-path-sync-label">
          {runningAction === `storage:${editedStorageConfig.id}` ? "Creating…" : "Create storage"}
        </span>
      </button>
    ) : (
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
            <div
              id={storageDetailsId}
              className="sync-storage-details-panel"
              hidden={!showStorageDetails}
            >
              {showStorageDetails && (
                <>
                  {storageConflict && (
                    <div className="sync-callout warning" role="alert">
                      <Icon name="alert-triangle" size={14} />
                      <span>
                        This storage was {storageConflict === "removed" ? "removed" : "changed"} outside this editor, so this draft can no longer be saved. Loading the current configuration discards the draft.
                      </span>
                      <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => void reloadExternalStorage()}>
                        Load current
                      </button>
                    </div>
                  )}
                  {creatingStorage && storageDraft.kind === "s3" && (
                    <div className="sync-callout" role="status">
                      <Icon name="terminal" size={14} />
                      <span>Let a terminal coding agent connect R2 through Mallard’s packaged CLI.</span>
                      <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={busy}
                        onClick={() => void copyAgentSetupInstruction()}
                      >
                        Set up with an agent
                      </button>
                      {agentCopyStatus && <span>{agentCopyStatus}</span>}
                    </div>
                  )}
                  {!creatingStorage && agentCopyStatus && (
                    <div className="sync-callout" role="status">
                      <Icon name="check-circle" size={14} />
                      <span>{agentCopyStatus}</span>
                    </div>
                  )}
                  <StorageEditor
                    storage={storageDraft}
                    disabled={busy || !!storageConflict}
                    onChange={(nextDraft) => {
                      agentSetupMarkerRef.current = null;
                      setStorageDraft(nextDraft);
                    }}
                    showNameField={creatingStorage}
                    // A draft's save is the page's verb and lives in the path
                    // bar; a settled storage's save belongs to the drawer it
                    // commits.
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
                </>
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
                ) : bundleError ? (
                  <div className="sync-callout error" role="alert">
                    <Icon name="alert-triangle" size={15} />
                    <span>{bundleError}</span>
                  </div>
                ) : orderedRemoteBundles.length === 0 ? (
                  <div className="sync-storage-repository-state">No repositories in this storage.</div>
                ) : (
                  <div className="sync-storage-repository-list">
                    {orderedRemoteBundles.map((bundle) => (
                      <StorageRepositoryRow key={bundle.bundle_id} bundle={bundle} />
                    ))}
                  </div>
                )}
              </section>
            )}

            {error && !renamingStorage && (
              <div className="sync-callout error"><Icon name="alert-triangle" size={15} /> {error}</div>
            )}
          </div>
        </section>
      </main>
      </>
    );
  }

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
          {agentCopyStatus && (
            <div className="sync-callout" role="status">
              <Icon name="check-circle" size={14} />
              <span>{agentCopyStatus}</span>
            </div>
          )}
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
      // The menu speaks for the current storage: a header naming it, its two
      // verbs right beneath — settings and the unlink that used to hide at the
      // far end of the bar — then the other storages as plain switch commands,
      // then the ways to get more. No radio state: the header is the "which
      // one am I on", so switching rows carry no checkmark.
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

    // Same grammar as the storage crumb: header, this project's verbs — the
    // four that used to live at the other end of the bar — then the other
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
              onSelect: () => { void openPath(workspaceProject.project_root as string); },
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
    } else {
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
    : activityHeader && {
      refreshing: activityHeader.refreshing,
      onRefresh: activityHeader.onRefresh,
      label: activityHeader.refreshing ? "Refreshing activity" : "Refresh activity",
    };

  const pathActions = workspaceProject && workspaceStorage ? (
    <>
      {/* One chip for everything neither button can clear. Four coloured
          tallies said the same thing in a vocabulary nothing else on the page
          used, and none of them was actionable. */}
      {attention > 0 && (
        <span
          className={`sync-path-attention ${activityHeader?.counts.severity ?? "warning"}`}
          tabIndex={0}
          role="status"
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
                    const saved = await onRenameProject(workspaceProject.local_project_id, null);
                    if (!saved) return;
                    setProjectAliasDraft(workspaceProject.display_name);
                    setRenamingProjectId(null);
                  },
                )}
              >
                Use repo name
              </button>
            )}
            {error && (
              <div className="sync-path-rename-error" role="alert">
                <Icon name="alert-triangle" size={13} /> {error}
              </div>
            )}
          </form>
        )}

        {agentCopyStatus && (
          <div className="sync-callout" role="status">
            <Icon name="check-circle" size={14} />
            <span>{agentCopyStatus}</span>
          </div>
        )}

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
              const renderStorageConnectionActions = (buttonClassName: string) => (
                <>
                  {availableStorages.length > 0 && (
                    <button
                      type="button"
                      className={`${buttonClassName}${linkingProjectId === project.local_project_id ? " active" : ""}`}
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
                    className={buttonClassName}
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
              const storageHeading = (
                <div
                  className="project-profile-storage-heading"
                  aria-label={`${projectLinks.length} linked storage location${projectLinks.length === 1 ? "" : "s"}`}
                >
                  <Icon name="link" size={14} className="project-profile-storage-icon" />
                  <span>Storage</span>
                  <small>{projectLinks.length}</small>
                  {!conversationPathNoticeBlocked && conversationPathNotice}
                  <span className="project-profile-storage-actions">
                    {!bindingActive && !selectedStorage && (
                      <button
                        type="button"
                        className="profile-link-another"
                        disabled={busy || !!inlineStorageReview}
                        onClick={() => void onConfigureBinding(project.local_project_id)}
                      >
                        <Icon name="folder" size={13} />
                        {projectBinding ? "Confirm setup" : "Set up project"}
                      </button>
                    )}
                    {renderStorageConnectionActions("profile-link-another profile-storage-icon-action")}
                  </span>
                </div>
              );

              // The group is a framed box. Its contents used to be permanent —
              // the storage card was always there — but the identity and the
              // buttons moved to the path bar, so on a settled project it can
              // now hold nothing at all, and an empty frame draws as a rule
              // above the history.
              const storageGroupNotice = (conversationPathNoticeBlocked || selectedStorage)
                && conversationPathNotice;
              const storageBlockShown = !!selectedStorage
                && (!!(storageNotice
                  && storageNotice.projectId === project.local_project_id
                  && storageNotice.storageId === selectedStorage.id)
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
                      className="project-profile-group project-storage-group"
                      aria-label={`${projectLabel(project)} storage`}
                    >
                      {storageGroupNotice}

                      {!selectedStorage && storageHeading}
                    {projectLinks.length === 0 && <div className="profile-link-no-storage">No storage linked yet.</div>}

                    {/* The block below returns null unless it has a notice or an
                        open review. Drop the wrapper with it: an empty list is
                        still an element, and it would keep the group footer off
                        `:first-child`, leaving the footer's rule stacked 1px
                        under the group frame as a doubled hairline. */}
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
                      // The card used to be the storage's identity and its two
                      // buttons; both live in the path bar now, so what is left
                      // is a container for banners and the open review. With
                      // neither it has nothing to frame, and an empty frame
                      // reads as a gap above the history.
                      if (!activeNotice && !reviewOpen) return null;

                      return (
                        <div
                          key={storage.id}
                          className={`storage-link-block selected${reviewOpen ? " sync-review-open" : ""}`}
                          aria-busy={reviewOpen && busy}
                        >
                          {activeNotice && activeNotice.kind === "outcome" && (
                            <SyncOutcomeNotice
                              view={syncOutcomeView(activeNotice.outcome)}
                              busy={noticeActionsDisabled}
                              onAction={() => onStorageNoticeAction?.(activeNotice.outcome)}
                              onDismiss={() => onDismissStorageNotice?.()}
                            />
                          )}
                          {activeNotice && activeNotice.kind === "requires_pull" && (
                            <div className="sync-callout sync-storage-notice sync-sync-review-pull-required" role="status">
                              <Icon name="git-compare-arrows" size={15} />
                              <span>
                                <strong>{activeNotice.copy.title}</strong>
                                {activeNotice.copy.body}
                                <small className="sync-sync-review-pull-detail">{activeNotice.copy.detail}</small>
                              </span>
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
                            </div>
                          )}
                          {activeNotice && activeNotice.kind === "remote_missing" && (
                            <div className="sync-callout sync-storage-notice sync-sync-review-remote-missing" role="alert">
                              <Icon name="alert-triangle" size={15} />
                              <span>
                                <strong>Remote history is missing</strong>
                                {`${storage.name || "This storage"} no longer holds this project's sync history, so Push is paused. If this is expected, unlink and link the storage again.`}
                              </span>
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
                            </div>
                          )}
                          {activeNotice && activeNotice.kind === "remote_replaced" && (
                            <div className="sync-callout sync-storage-notice sync-sync-review-remote-missing" role="alert">
                              <Icon name="alert-triangle" size={15} />
                              <span>
                                <strong>Storage history changed</strong>
                                {`${storage.name || "This storage"} holds a different sync history than this project last synced${activeNotice.baseRevision != null ? ` (revision ${activeNotice.baseRevision})` : ""}. Review & adopt pulls the new history; unlinking doesn't delete files.`}
                              </span>
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
                            </div>
                          )}
                          {activeNotice && activeNotice.kind === "action_failed" && (
                            <div className="sync-callout sync-storage-notice error" role="alert">
                              <Icon name="alert-triangle" size={15} />
                              <span>
                                <strong>{activeNotice.action === "push" ? "Push couldn't start" : "Pull couldn't start"}</strong>
                                {activeNotice.message}
                              </span>
                              <div className="sync-sync-review-recovery-actions">
                                {isIrrecoverableSyncError(activeNotice.message) ? (
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
                                      `${activeNotice.action}:${actionPrefix}`,
                                      () => (activeNotice.action === "push" ? onPush : onPull)(project.local_project_id, storage.id),
                                    )}
                                  >
                                    Retry
                                  </button>
                                )}
                              </div>
                            </div>
                          )}
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
                        className="project-profile-group-footer"
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
        ) : null)}

        {activeProject && !newProjectSetup && !inlineStorageReview && (
          <div id="project-history-panel" className="sync-project-tab-panel">
            <ProjectActivityPage
              embedded
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
              onOpenSessionDetail={(request, trigger) => (
                openSessionDetail(activeProject.local_project_id, request, trigger)
              )}
            />
          </div>
        )}
      </section>

      {sessionDetailSubview && workspaceProject && (
        <Suspense fallback={(
          <section className="session-detail-page session-detail-entry-state" role="status">
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
            initialCheckpointId={sessionDetailSubview.initialCheckpointId}
            onHeaderChange={handleSessionHeaderChange}
          />
        </Suspense>
      )}

    </main>
    </>
  );
}
