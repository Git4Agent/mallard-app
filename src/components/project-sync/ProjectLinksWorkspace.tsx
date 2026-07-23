import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import type {
  BundlePage,
  CodexConversationPathAudit,
  LocalProjectSummary,
  ProjectBinding,
  ProjectStorageLink,
  ProviderProfileSummary,
  RemoteBundleSummary,
  StorageConfigV3,
} from "../../types";
import Icon from "../Icons";
import ProjectChatHistoryPage from "./ProjectChatHistoryPage";
import SkillsPluginStatusPage from "./SkillsPluginStatusPage";
import ConversationPathRepairNotice from "./ConversationPathRepairNotice";
import { newStorage, storageConfigReady, StorageEditor } from "./StorageSettingsV3";
import { projectSyncApi } from "./api";
import {
  compactProjectPath,
  configuredProjectProvider,
  errorMessage,
  formatRelativeTime,
  PROJECT_PROVIDERS,
  projectLabel,
  providerLabel,
} from "./model";

type LinkKey = { projectId: string; storageId: string };
export type ProjectWorkspaceTab = "history" | "skills" | "plugins";
export type StorageReviewKind = "pull" | "push";
type StorageEditorRequest =
  | { mode: "toggle"; storageId: string; requestId: number }
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
};

interface Props {
  projects: LocalProjectSummary[];
  activeProjectId: string | null;
  activeStorageId: string | null;
  bindings: ProjectBinding[];
  profiles: ProviderProfileSummary[];
  storages: StorageConfigV3[];
  links: ProjectStorageLink[];
  loading: boolean;
  busy: boolean;
  error: string | null;
  conversationPathAudits: Record<string, CodexConversationPathAudit>;
  conversationPathAuditErrors: Record<string, string>;
  conversationPathAuditLoading: boolean;
  onSelectStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onLinkStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onUnlinkStorage: (projectId: string, storageId: string) => Promise<void> | void;
  onPush: (projectId: string, storageId: string) => Promise<void> | void;
  onPull: (projectId: string, storageId: string) => Promise<void> | void;
  onConfigureBinding: (projectId: string) => Promise<void> | void;
  onRepairConversationPaths: (projectId: string) => Promise<void> | void;
  onRenameProject: (projectId: string, alias: string | null) => Promise<boolean> | boolean;
  onRefresh: () => void;
  onAddProject: () => void;
  onOpenStorageSettings: (storageId?: string) => void;
  onSaveStorage: (storage: StorageConfigV3) => Promise<boolean | void> | boolean | void;
  storageEditorRequest?: StorageEditorRequest | null;
  onStorageEditorRequestHandled?: () => void;
  onStorageEditorChange?: (storageId: string | null) => void;
  newProjectSetup?: ReactNode;
  historyRefreshEpoch?: number;
  projectTabResetEpoch?: number;
  inlineStorageReview?: InlineStorageReview | null;
}

function storageSubtitle(storage: StorageConfigV3): string {
  if (storage.kind === "local") return compactProjectPath(storage.local_dir || "Folder not configured");
  if (storage.bucket) return `Bucket · ${storage.bucket}`;
  return storage.s3_endpoint ? "S3 endpoint configured" : "S3 storage not configured";
}

export function ProjectWorkspaceTabs({
  activeTab,
  isGitRepository,
  onChange,
}: {
  activeTab: ProjectWorkspaceTab;
  isGitRepository: boolean;
  onChange: (tab: ProjectWorkspaceTab) => void;
}) {
  const tabOrder: ProjectWorkspaceTab[] = ["history", "skills", "plugins"];
  const selectFromKeyboard = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    tab: ProjectWorkspaceTab,
  ) => {
    event.preventDefault();
    const tabList = event.currentTarget.parentElement;
    onChange(tab);
    window.requestAnimationFrame(() => {
      tabList
        ?.querySelector<HTMLButtonElement>(`[data-project-tab="${tab}"]`)
        ?.focus();
    });
  };
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    const currentTab = event.currentTarget.dataset.projectTab as ProjectWorkspaceTab;
    const currentIndex = tabOrder.indexOf(currentTab);
    if (event.key === "Home") return selectFromKeyboard(event, tabOrder[0]);
    if (event.key === "End") return selectFromKeyboard(event, tabOrder[tabOrder.length - 1]);
    if (event.key === "ArrowLeft") {
      return selectFromKeyboard(event, tabOrder[(currentIndex - 1 + tabOrder.length) % tabOrder.length]);
    }
    if (event.key === "ArrowRight") {
      selectFromKeyboard(event, tabOrder[(currentIndex + 1) % tabOrder.length]);
    }
  };

  return (
    <div className="v3-project-tabs" role="tablist" aria-label="Project information">
      <button
        type="button"
        id="project-history-tab"
        data-project-tab="history"
        role="tab"
        aria-selected={activeTab === "history"}
        aria-controls="project-history-panel"
        tabIndex={activeTab === "history" ? 0 : -1}
        className={activeTab === "history" ? "active" : undefined}
        onClick={() => onChange("history")}
        onKeyDown={handleKeyDown}
      >
        <Icon name={isGitRepository ? "git-branch" : "openai"} size={14} />
        {isGitRepository ? "Git & sessions" : "Sessions"}
      </button>
      <button
        type="button"
        id="project-skills-tab"
        data-project-tab="skills"
        role="tab"
        aria-selected={activeTab === "skills"}
        aria-controls="project-skills-panel"
        tabIndex={activeTab === "skills" ? 0 : -1}
        className={activeTab === "skills" ? "active" : undefined}
        onClick={() => onChange("skills")}
        onKeyDown={handleKeyDown}
      >
        <Icon name="folder" size={14} />
        Skills
      </button>
      <button
        type="button"
        id="project-plugins-tab"
        data-project-tab="plugins"
        role="tab"
        aria-selected={activeTab === "plugins"}
        aria-controls="project-plugins-panel"
        tabIndex={activeTab === "plugins" ? 0 : -1}
        className={activeTab === "plugins" ? "active" : undefined}
        onClick={() => onChange("plugins")}
        onKeyDown={handleKeyDown}
      >
        <Icon name="link" size={14} />
        Plugins
      </button>
    </div>
  );
}

export function StorageRepositoryRow({ bundle }: { bundle: RemoteBundleSummary }) {
  const name = bundle.display_name || "Unnamed repository";
  return (
    <details className="v3-storage-repository-row" name="storage-repository-details">
      <summary aria-label={`Show details for ${name}`}>
        <span className="v3-storage-repository-icon"><Icon name="folder" size={16} /></span>
        <strong className="v3-storage-repository-name">{name}</strong>
        <span className="v3-storage-repository-details-icon" title="Repository details">
          <Icon name="info" size={15} />
        </span>
      </summary>
      <dl className="v3-storage-repository-details">
        <div>
          <dt>Repository ID</dt>
          <dd><code>{bundle.bundle_id}</code></dd>
        </div>
        <div>
          <dt>Generation</dt>
          <dd>{bundle.generation ?? "—"}</dd>
        </div>
        <div>
          <dt>Resources</dt>
          <dd>{bundle.resource_count ?? 0}</dd>
        </div>
        <div>
          <dt>Updated</dt>
          <dd>{formatRelativeTime(bundle.updated_at)}</dd>
        </div>
      </dl>
    </details>
  );
}

export function StorageSettingsMeta({
  storage,
  creating = false,
}: {
  storage: StorageConfigV3;
  creating?: boolean;
}) {
  const kindLabel = storage.kind === "local" ? "Local folder" : "Cloudflare R2";
  const label = creating ? kindLabel : storage.name || "Unnamed storage";

  return (
    <div className="v3-storage-settings-meta" title={`${kindLabel}: ${label}`}>
      <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={12} />
      <span>{label}</span>
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
  reviewOpen,
  preparing,
  publishing,
}: {
  reviewOpen: boolean;
  preparing: boolean;
  publishing: boolean;
}): string {
  if (publishing) return "Pushing…";
  if (preparing) return "Preparing…";
  if (reviewOpen) return "Continue push";
  return "Push";
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
  onSelectStorage,
  onLinkStorage,
  onUnlinkStorage,
  onPush,
  onPull,
  onConfigureBinding,
  onRepairConversationPaths,
  onRenameProject,
  onRefresh,
  onAddProject,
  onOpenStorageSettings,
  onSaveStorage,
  storageEditorRequest,
  onStorageEditorRequestHandled,
  onStorageEditorChange,
  newProjectSetup,
  historyRefreshEpoch = 0,
  projectTabResetEpoch = 0,
  inlineStorageReview,
}: Props) {
  const [linkingProjectId, setLinkingProjectId] = useState<string | null>(null);
  const [runningAction, setRunningAction] = useState<string | null>(null);
  const [editingStorage, setEditingStorage] = useState<LinkKey | null>(null);
  const [storageDraft, setStorageDraft] = useState<StorageConfigV3 | null>(null);
  const [storageDetailsExpanded, setStorageDetailsExpanded] = useState(false);
  const [bundlePage, setBundlePage] = useState<BundlePage | null>(null);
  const [bundleLoading, setBundleLoading] = useState(false);
  const [bundleError, setBundleError] = useState<string | null>(null);
  const [renamingProjectId, setRenamingProjectId] = useState<string | null>(null);
  const [projectAliasDraft, setProjectAliasDraft] = useState("");
  const [activeProjectTab, setActiveProjectTab] = useState<ProjectWorkspaceTab>("history");
  const [storagePickerProjectId, setStoragePickerProjectId] = useState<string | null>(null);
  const bundleRequestRef = useRef(0);
  const storageSettingsRef = useRef<HTMLDivElement>(null);
  const storagePickerRef = useRef<HTMLDivElement>(null);
  const storagePickerMenuRef = useRef<HTMLDivElement>(null);
  const storagePickerTriggerRef = useRef<HTMLButtonElement>(null);

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
    setStoragePickerProjectId(null);
  }, [activeProjectId]);

  useEffect(() => {
    setActiveProjectTab("history");
  }, [projectTabResetEpoch]);

  useEffect(() => {
    if (!inlineStorageReview) return;
    setStoragePickerProjectId(null);
    setLinkingProjectId(null);
  }, [inlineStorageReview?.kind, inlineStorageReview?.projectId, inlineStorageReview?.storageId]);

  useEffect(() => {
    if (!storagePickerProjectId) return;
    const pickerContains = (target: EventTarget | null) => target instanceof Node && (
      storagePickerRef.current?.contains(target) || storagePickerMenuRef.current?.contains(target)
    );
    const handlePointerDown = (event: PointerEvent) => {
      if (!pickerContains(event.target)) {
        setStoragePickerProjectId(null);
      }
    };
    const handleFocusIn = (event: FocusEvent) => {
      if (!pickerContains(event.target)) setStoragePickerProjectId(null);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setStoragePickerProjectId(null);
      window.requestAnimationFrame(() => storagePickerTriggerRef.current?.focus());
    };
    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("focusin", handleFocusIn);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("focusin", handleFocusIn);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [storagePickerProjectId]);

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
  const creatingStorage = !!editingStorage && !savedEditedStorage;
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

  const loadStorageBundles = async (storageId: string) => {
    const requestId = ++bundleRequestRef.current;
    setBundlePage(null);
    setBundleError(null);
    setBundleLoading(true);
    try {
      const bundles = await projectSyncApi.listRemoteBundleSnapshots(storageId);
      if (bundleRequestRef.current === requestId) setBundlePage({ bundles });
    } catch (reason) {
      if (bundleRequestRef.current === requestId) {
        setBundleError(errorMessage(reason));
      }
    } finally {
      if (bundleRequestRef.current === requestId) setBundleLoading(false);
    }
  };

  useEffect(() => {
    if (!storageEditorRequest) return;
    if (storageEditorRequest.mode === "close") {
      bundleRequestRef.current += 1;
      setEditingStorage(null);
      setStorageDraft(null);
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
      setStorageDetailsExpanded(true);
      setBundlePage(null);
      setBundleError(null);
      setBundleLoading(false);
      onStorageEditorRequestHandled?.();
      return;
    }
    const storage = storages.find((candidate) => candidate.id === storageEditorRequest.storageId);
    if (!storage) return;
    if (editingStorage?.storageId === storage.id) {
      bundleRequestRef.current += 1;
      setEditingStorage(null);
      setStorageDraft(null);
      setStorageDetailsExpanded(false);
      setBundlePage(null);
      setBundleError(null);
      setBundleLoading(false);
      onStorageEditorRequestHandled?.();
      return;
    }

    setEditingStorage({ projectId: "", storageId: storage.id });
    setStorageDraft({ ...storage });
    setStorageDetailsExpanded(!storageConfigReady(storage));
    void loadStorageBundles(storage.id);
    onStorageEditorRequestHandled?.();
  }, [storageEditorRequest?.requestId]);

  const toggleStorageEditor = async (storage: StorageConfigV3) => {
    if (editingStorage?.storageId === storage.id) {
      setEditingStorage(null);
      setStorageDraft(null);
      setStorageDetailsExpanded(false);
      return;
    }

    setEditingStorage({ projectId: "", storageId: storage.id });
    setStorageDraft({ ...storage });
    setStorageDetailsExpanded(!storageConfigReady(storage));
    await loadStorageBundles(storage.id);
  };

  const activeProject = projects.find((project) => project.local_project_id === activeProjectId) ?? null;
  const workspaceProject = newProjectSetup ? null : activeProject;
  const workspaceBinding = workspaceProject
    ? bindingByProject.get(workspaceProject.local_project_id)
    : undefined;
  const workspaceProviders = PROJECT_PROVIDERS.filter((provider) => (
    workspaceBinding?.profile_ids?.[provider]
  ));
  const workspaceHasMultipleProviders = workspaceProviders.length > 1;
  const workspaceProvider = configuredProjectProvider(workspaceBinding?.profile_ids);
  const workspaceProfileId = workspaceProvider
    ? workspaceBinding?.profile_ids?.[workspaceProvider]
    : null;
  const workspaceProfile = profiles.find((candidate) => candidate.profile_id === workspaceProfileId);
  const workspaceProfileReady = !workspaceHasMultipleProviders
    && !!workspaceProfile?.available
    && workspaceProfile.readable;
  const workspaceProfileLabel = workspaceHasMultipleProviders
    ? "Choose one agent"
    : workspaceProfile?.display_name ?? "No agent configured";
  const workspaceProfileTitle = workspaceHasMultipleProviders
    ? "Codex and Claude are both assigned"
    : workspaceProfile
      ? `${providerLabel(workspaceProvider ?? "codex")} agent home: ${workspaceProfile.path}${workspaceProfileReady ? "" : " (unavailable)"}`
      : "No agent profile is assigned";
  const proposedProjectAlias = workspaceProject
    ? projectAliasDraft.trim() && projectAliasDraft.trim() !== workspaceProject.display_name
      ? projectAliasDraft.trim()
      : null
    : null;
  const projectNameChanged = !!workspaceProject
    && proposedProjectAlias !== (workspaceProject.local_alias ?? null);

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

  if (editingStorage && storageDraft && editedStorageConfig) {
    const storageReady = storageConfigReady(storageDraft);
    const showStorageDetails = creatingStorage || storageDetailsExpanded;
    const storageDetailsId = `storage-${storageDraft.id}-settings`;
    return (
      <main className="v3-main v3-project-links-page v3-storage-settings-page">
        <section className="profile-links-section" aria-labelledby="storage-settings-heading">
          <div className="profile-links-heading">
            <div className="profile-links-copy">
              <h1 id="storage-settings-heading" className="settings-section-title">
                {creatingStorage ? "New storage" : "Storage settings"}
              </h1>
              <StorageSettingsMeta storage={storageDraft} creating={creatingStorage} />
            </div>
            <div className="v3-storage-settings-actions">
              {!creatingStorage && (
                <button
                  type="button"
                  className="btn btn-ghost v3-storage-details-toggle"
                  onClick={() => setStorageDetailsExpanded((current) => !current)}
                  disabled={busy}
                  aria-controls={storageDetailsId}
                  aria-expanded={storageDetailsExpanded}
                >
                  {storageDetailsExpanded ? "Hide settings" : "Edit settings"}
                  <Icon name="chevron-down" size={12} className="v3-storage-details-chevron" />
                </button>
              )}
              <button
                type="button"
                className="btn btn-ghost"
                onClick={() => {
                  setEditingStorage(null);
                  setStorageDraft(null);
                  setStorageDetailsExpanded(false);
                }}
                aria-label="Close storage settings"
              >
                <Icon name="x" size={14} />
              </button>
            </div>
          </div>

          <div
            ref={storageSettingsRef}
            className={`v3-storage-settings-below v3-storage-settings-dedicated${showStorageDetails ? " settings-expanded" : " settings-collapsed"}`}
          >
            <div
              id={storageDetailsId}
              className="v3-storage-details-panel"
              hidden={!showStorageDetails}
            >
              {showStorageDetails && (
                <StorageEditor
                  storage={storageDraft}
                  disabled={busy}
                  onChange={setStorageDraft}
                  primaryAction={(
                    <button
                      type="button"
                      className="btn"
                      disabled={busy || !storageReady || (!creatingStorage && JSON.stringify(storageDraft) === JSON.stringify(editedStorageConfig))}
                      onClick={() => void run(`storage:${editedStorageConfig.id}`, async () => {
                        const saved = await onSaveStorage(storageDraft);
                        if (saved === false) return;
                        setStorageDetailsExpanded(false);
                        await loadStorageBundles(editedStorageConfig.id);
                      })}
                    >
                      {runningAction === `storage:${editedStorageConfig.id}` ? "Saving…" : creatingStorage ? "Create storage" : "Save storage"}
                    </button>
                  )}
                />
              )}
            </div>

            {!creatingStorage && (
              <section className="v3-storage-repositories" aria-labelledby="storage-repositories-heading">
                <div className="v3-storage-repositories-heading">
                  <div>
                    <h2 id="storage-repositories-heading">Repositories</h2>
                    <span>{orderedRemoteBundles.length} repo{orderedRemoteBundles.length === 1 ? "" : "s"} in this storage</span>
                  </div>
                  <button
                    type="button"
                    className="btn btn-ghost"
                    onClick={() => void loadStorageBundles(editedStorageConfig.id)}
                    disabled={busy || bundleLoading}
                    title="Refresh repositories"
                    aria-label={`Refresh repositories in ${editedStorageConfig.name || "storage"}`}
                  >
                    <Icon name="refresh" size={14} className={bundleLoading ? "icon-spin" : undefined} />
                  </button>
                </div>

                {bundleLoading ? (
                  <div className="v3-storage-repository-state"><span className="status-loader" /> Loading repositories…</div>
                ) : bundleError ? (
                  <div className="v3-callout error"><Icon name="alert-triangle" size={15} /> {bundleError}</div>
                ) : orderedRemoteBundles.length === 0 ? (
                  <div className="v3-storage-repository-state">No repositories in this storage.</div>
                ) : (
                  <div className="v3-storage-repository-list">
                    {orderedRemoteBundles.map((bundle) => (
                      <StorageRepositoryRow key={bundle.bundle_id} bundle={bundle} />
                    ))}
                  </div>
                )}
              </section>
            )}

            {error && <div className="v3-callout error"><Icon name="alert-triangle" size={15} /> {error}</div>}
          </div>
        </section>
      </main>
    );
  }

  if (!workspaceProject && !editingStorage && !newProjectSetup) {
    return (
      <main className="v3-main v3-project-links-page v3-git-info-page">
        <section className="profile-links-section" aria-labelledby="git-info-heading">
          <div className="profile-links-heading">
            <div className="profile-links-copy">
              <h1 id="git-info-heading" className="settings-section-title">Project history</h1>
              <div className="profile-links-subtitle">Select a completed project to view its Codex history.</div>
            </div>
          </div>
        </section>
      </main>
    );
  }

  return (
    <main className={`v3-main v3-project-links-page${workspaceProject ? " v3-project-workspace-page v3-project-combined-page" : ""}${newProjectSetup ? " v3-project-setup-page" : ""}${inlineStorageReview ? " v3-sync-review-page" : ""}`}>
      <section
        className="profile-links-section"
        aria-label={newProjectSetup
          ? "Project setup"
          : workspaceProject
            ? projectLabel(workspaceProject)
            : undefined}
        aria-labelledby={newProjectSetup || workspaceProject ? undefined : "project-links-heading"}
      >
        {!newProjectSetup && (
          <div className="profile-links-heading v3-combined-project-heading">
            <div className="profile-links-copy">
              {workspaceProject ? (
                <div className="v3-project-heading-identity">
                  {renamingProjectId === workspaceProject.local_project_id ? (
                    <form
                      className="v3-project-heading-rename"
                      onSubmit={(event) => {
                        event.preventDefault();
                        void saveProjectName();
                      }}
                    >
                      <input
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
                    </form>
                  ) : (
                    <div className="v3-project-heading-name">
                      <h1>{projectLabel(workspaceProject)}</h1>
                      <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={busy}
                        onClick={() => {
                          setProjectAliasDraft(workspaceProject.local_alias ?? workspaceProject.display_name);
                          setRenamingProjectId(workspaceProject.local_project_id);
                        }}
                      >
                        Rename
                      </button>
                    </div>
                  )}
                  <div className="v3-project-heading-meta">
                    {workspaceProject.local_alias && (
                      <span
                        className="v3-project-heading-meta-item"
                        title={`Repository: ${workspaceProject.display_name}`}
                        aria-label={`Repository ${workspaceProject.display_name}`}
                      >
                        <Icon name={workspaceProject.is_git_repository ? "git-folder" : "folder"} size={12} />
                        <span>{workspaceProject.display_name}</span>
                      </span>
                    )}
                    <span
                      className="v3-project-heading-meta-item"
                      title={workspaceProject.project_root ?? undefined}
                      aria-label={`Project folder ${workspaceProject.project_root ?? "not configured"}`}
                    >
                      <Icon name="folder" size={12} />
                      <span>{compactProjectPath(workspaceProject.project_root)}</span>
                    </span>
                    <span
                      className={`v3-project-heading-meta-item v3-project-heading-agent${workspaceProfileReady ? "" : " warning"}`}
                      title={workspaceProfileTitle}
                    >
                      <Icon name="terminal" size={12} />
                      <span>{workspaceProfileLabel}</span>
                    </span>
                  </div>
                  {renamingProjectId === workspaceProject.local_project_id && error && (
                    <div className="v3-project-heading-error"><Icon name="alert-triangle" size={13} /> {error}</div>
                  )}
                </div>
              ) : (
                <>
                  <h1 id="project-links-heading" className="settings-section-title">Project links</h1>
                  <div className="profile-links-subtitle">Choose where each project repo syncs.</div>
                </>
              )}
            </div>
            {!workspaceProject && (
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
            )}
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
          <div id="project-configuration-panel" className="profile-links-list v3-combined-settings-panel">
            {[workspaceProject].map((project) => {
              const projectLinks = linkedByProject.get(project.local_project_id) ?? [];
              const linkedStorages = projectLinks
                .map((link) => storages.find((storage) => storage.id === link.storage_id))
                .filter((storage): storage is StorageConfigV3 => !!storage);
              const selectedStorage = linkedStorages.find((storage) => (
                project.local_project_id === activeProjectId && storage.id === activeStorageId
              )) ?? linkedStorages[0] ?? null;
              const storagePickerOpen = storagePickerProjectId === project.local_project_id;
              const availableStorages = storages.filter((storage) => (
                !projectLinks.some((link) => link.storage_id === storage.id)
              ));
              const projectBinding = bindingByProject.get(project.local_project_id);
              const bindingActive = projectBinding?.state === "active";
              const configuredProviders = PROJECT_PROVIDERS.filter((provider) => (
                projectBinding?.profile_ids?.[provider]
              ));
              const hasMultipleProviders = configuredProviders.length > 1;
              const projectProvider = configuredProjectProvider(projectBinding?.profile_ids);
              const projectProfileId = projectProvider
                ? projectBinding?.profile_ids?.[projectProvider]
                : null;
              const projectProfile = profiles.find((candidate) => candidate.profile_id === projectProfileId);
              const profilesReadable = !hasMultipleProviders
                && !!projectProfile?.available
                && projectProfile.readable;
              const profilesWritable = profilesReadable && !!projectProfile?.writable;
              const canSync = bindingActive && profilesReadable;
              const canRestore = bindingActive && profilesWritable;
              const codexConfigured = bindingActive && projectProvider === "codex" && !hasMultipleProviders;
              const conversationPathAudit = conversationPathAudits[project.local_project_id];
              const conversationPathAuditError = conversationPathAuditErrors[project.local_project_id];
              const conversationPathNoticeBlocked = !!conversationPathAuditError
                || !!conversationPathAudit?.blockers.length;
              const conversationPathBlocked = conversationPathsBlockSync(
                codexConfigured,
                conversationPathAudit,
                conversationPathAuditError,
                conversationPathAuditLoading,
              );
              const conversationPathTitle = codexConfigured
                ? conversationPathAuditLoading
                  ? "Checking Codex conversation paths"
                  : conversationPathAuditError
                    ? `Codex conversation paths could not be verified: ${conversationPathAuditError}`
                    : conversationPathAudit && !conversationPathAudit.ready
                      ? "Repair Codex conversation paths before Push or Pull"
                      : null
                : null;
              const profileIssue = hasMultipleProviders
                ? "Choose one project agent before syncing"
                : bindingActive && !profilesReadable
                  ? "The selected agent profile is unavailable"
                  : null;
              const conversationPathNotice = codexConfigured && !conversationPathAuditLoading ? (
                <ConversationPathRepairNotice
                  audit={conversationPathAudit}
                  auditError={conversationPathAuditError}
                  projectName={projectLabel(project)}
                  profileName={projectProfile?.display_name ?? "Codex"}
                  profilePath={projectProfile?.path ?? conversationPathAudit?.profile_path}
                  showScope={false}
                  busy={busy || runningAction === `repair-paths:${project.local_project_id}`}
                  onRepair={() => run(
                    `repair-paths:${project.local_project_id}`,
                    () => onRepairConversationPaths(project.local_project_id),
                  )}
                />
              ) : null;

              return (
                <article
                  key={project.local_project_id}
                  className="profile-link-card v3-project-link-card"
                >
                  <div className="profile-link-connections">
                    <section
                      className="project-profile-group project-storage-group"
                      aria-label={`${projectLabel(project)} storage`}
                    >
                      {conversationPathNoticeBlocked && conversationPathNotice}

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
                          {availableStorages.length > 0 && (
                            <button
                              type="button"
                              className={`profile-link-another${linkingProjectId === project.local_project_id ? " active" : ""}`}
                              disabled={busy || !!inlineStorageReview}
                              aria-expanded={linkingProjectId === project.local_project_id}
                              onClick={() => {
                                setStoragePickerProjectId(null);
                                setLinkingProjectId((current) => current === project.local_project_id ? null : project.local_project_id);
                              }}
                            >
                              <Icon name="link" size={13} />
                              Link storage
                            </button>
                          )}
                          <button
                            type="button"
                            className="profile-link-another"
                            disabled={busy || !!inlineStorageReview}
                            onClick={() => {
                              setStoragePickerProjectId(null);
                              setLinkingProjectId(null);
                              onOpenStorageSettings();
                            }}
                          >
                            <Icon name="plus" size={14} />
                            Add storage
                          </button>
                        </span>
                      </div>
                    {projectLinks.length === 0 && <div className="profile-link-no-storage">No storage linked yet.</div>}

                    <div className="project-profile-storage-list">
                    {selectedStorage && (() => {
                      const storage = selectedStorage;
                      const reviewOpen = inlineStorageReview?.projectId === project.local_project_id
                        && inlineStorageReview.storageId === storage.id;
                      const actionPrefix = `${project.local_project_id}:${storage.id}`;
                      const reviewPanelId = `storage-review-${actionPrefix.replace(/[^a-z0-9_-]/gi, "-")}`;
                      const activeReviewKind = reviewOpen ? inlineStorageReview?.kind ?? null : null;
                      const pushReviewOpen = reviewOpen && inlineStorageReview?.kind === "push";
                      const pullReviewOpen = reviewOpen && inlineStorageReview?.kind === "pull";
                      const pushPreparing = runningAction === `push:${actionPrefix}`;
                      const pushPublishing = pushReviewOpen && busy;
                      const pushLabel = projectPushActionLabel({
                        reviewOpen: pushReviewOpen,
                        preparing: pushPreparing,
                        publishing: pushPublishing,
                      });
                      const pushProgress = pushPreparing || pushPublishing;
                      const storageControlsLocked = busy
                        || !!runningAction
                        || storageActionLockedForReview(activeReviewKind, "storage");
                      const focusStorageOption = (position: "first" | "last") => {
                        window.requestAnimationFrame(() => {
                          const options = Array.from(storagePickerMenuRef.current?.querySelectorAll<HTMLButtonElement>(
                            '[role="menuitem"]',
                          ) ?? []);
                          if (options.length === 0) return;
                          if (position === "first") options[0]?.focus();
                          else options[options.length - 1]?.focus();
                        });
                      };

                      return (
                        <div
                          key={storage.id}
                          className={`storage-link-block selected${reviewOpen ? " v3-review-open" : ""}${storagePickerOpen ? " storage-picker-open" : ""}`}
                          aria-busy={reviewOpen && busy}
                        >
                          <div className="storage-link-row">
                            <div className="storage-link-storage-section">
                              <div ref={storagePickerRef} className="storage-link-main">
                                {linkedStorages.length > 1 ? (
                                  <button
                                    ref={storagePickerTriggerRef}
                                    type="button"
                                    className={`storage-link-identity storage-link-switcher${storagePickerOpen ? " open" : ""}`}
                                    disabled={storageControlsLocked}
                                    aria-haspopup="menu"
                                    aria-expanded={storagePickerOpen}
                                    aria-controls={`storage-picker-${project.local_project_id}`}
                                    title="Choose the storage used to compare, pull, and push"
                                    onClick={() => {
                                      setLinkingProjectId(null);
                                      setStoragePickerProjectId((current) => (
                                        current === project.local_project_id ? null : project.local_project_id
                                      ));
                                    }}
                                    onKeyDown={(event) => {
                                      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                                      event.preventDefault();
                                      setStoragePickerProjectId(project.local_project_id);
                                      focusStorageOption(event.key === "ArrowDown" ? "first" : "last");
                                    }}
                                    aria-label={`Active storage: ${storage.name || "unnamed"}. Choose another storage`}
                                  >
                                    <span className="storage-link-icon" aria-hidden="true">
                                      <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={23} />
                                    </span>
                                    <span className="storage-link-copy">
                                      <strong>{storage.name || "(unnamed)"}</strong>
                                      <span title={storageSubtitle(storage)}>{storageSubtitle(storage)}</span>
                                    </span>
                                    <span className="storage-link-switcher-affordance" aria-hidden="true">
                                      <Icon name="chevron-down" size={13} />
                                    </span>
                                  </button>
                                ) : (
                                  <div className="storage-link-identity">
                                    <span className="storage-link-icon">
                                      <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={23} />
                                    </span>
                                    <span className="storage-link-copy">
                                      <strong>{storage.name || "(unnamed)"}</strong>
                                      <span title={storageSubtitle(storage)}>{storageSubtitle(storage)}</span>
                                    </span>
                                  </div>
                                )}
                              </div>
                            </div>

                            <div className="storage-link-actions">
                              <div className="storage-link-row-controls" role="group" aria-label="Storage settings">
                                <button
                                  type="button"
                                  className={`storage-link-configure${editingStorage?.storageId === storage.id ? " active" : ""}`}
                                  disabled={storageControlsLocked}
                                  onClick={() => void toggleStorageEditor(storage)}
                                  title={`Configure ${storage.name || "storage"}`}
                                  aria-label={`Configure ${storage.name || "storage"}`}
                                  aria-expanded={editingStorage?.storageId === storage.id}
                                >
                                  <Icon name="settings" size={16} />
                                </button>
                                <button
                                  type="button"
                                  className="storage-link-unlink"
                                  disabled={storageControlsLocked}
                                  onClick={() => void run(
                                    `unlink:${actionPrefix}`,
                                    () => onUnlinkStorage(project.local_project_id, storage.id),
                                  )}
                                  title={`Unlink ${storage.name || "storage"} from this project`}
                                  aria-label={`Unlink ${storage.name || "storage"} from ${projectLabel(project)}`}
                                >
                                  <Icon
                                    name={runningAction === `unlink:${actionPrefix}` ? "refresh" : "x"}
                                    size={14}
                                    className={runningAction === `unlink:${actionPrefix}` ? "icon-spin" : undefined}
                                  />
                                </button>
                              </div>

                              <div className="storage-link-action-group" role="group" aria-label="Sync actions">
                                <button
                                  type="button"
                                  className={`storage-link-sync storage-link-sync-secondary${pullReviewOpen ? " active" : ""}`}
                                  disabled={busy
                                    || !!runningAction
                                    || conversationPathBlocked
                                    || (bindingActive && !canRestore)
                                    || storageActionLockedForReview(activeReviewKind, "pull")}
                                  onClick={() => {
                                    if (pullReviewOpen) {
                                      document
                                        .getElementById(reviewPanelId)
                                        ?.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')
                                        ?.focus({ preventScroll: true });
                                      return;
                                    }
                                    setStoragePickerProjectId(null);
                                    setLinkingProjectId(null);
                                    void run(`pull:${actionPrefix}`, () => onPull(project.local_project_id, storage.id));
                                  }}
                                  title={conversationPathTitle
                                    ?? profileIssue
                                    ?? (!bindingActive
                                      ? "Confirm project setup before pulling"
                                      : pullReviewOpen
                                      ? "Return to the open Pull review"
                                      : !profilesWritable
                                        ? "The selected agent profile is read only"
                                        : "Review the Pull actions before applying them")}
                                  aria-label={!bindingActive
                                    ? "Set up project before Pull"
                                    : pullReviewOpen
                                    ? "Return to Pull review"
                                    : `Pull from ${storage.name || "storage"}`}
                                  aria-expanded={pullReviewOpen}
                                >
                                  <Icon name="download" size={15} />
                                  {runningAction === `pull:${actionPrefix}`
                                    ? "Reviewing…"
                                    : pullReviewOpen
                                      ? "View review"
                                      : bindingActive
                                        ? "Pull"
                                        : "Set up"}
                                </button>
                                <button
                                  type="button"
                                  className={`storage-link-sync storage-link-sync-commit${pushReviewOpen ? " active" : ""}`}
                                  disabled={busy
                                    || !!runningAction
                                    || conversationPathBlocked
                                    || !canSync
                                    || storageActionLockedForReview(activeReviewKind, "push")
                                    || (pushReviewOpen && inlineStorageReview?.continueDisabled)}
                                  onClick={() => {
                                    if (pushReviewOpen) {
                                      inlineStorageReview?.onContinue?.();
                                      return;
                                    }
                                    setStoragePickerProjectId(null);
                                    setLinkingProjectId(null);
                                    void run(`push:${actionPrefix}`, () => onPush(project.local_project_id, storage.id));
                                  }}
                                  title={conversationPathTitle
                                    ?? profileIssue
                                    ?? (pushReviewOpen
                                      ? "Continue reviewing resources for this push"
                                      : "Choose resources, then push them to this storage")}
                                  aria-label={pushReviewOpen
                                    ? "Continue push"
                                    : `Push to ${storage.name || "storage"}`}
                                  aria-expanded={pushReviewOpen}
                                  aria-controls={pushReviewOpen ? reviewPanelId : undefined}
                                >
                                  <Icon
                                    key={pushProgress ? "push-progress" : "push-idle"}
                                    name={pushProgress ? "refresh" : "upload"}
                                    size={15}
                                    className={pushProgress ? "icon-spin" : undefined}
                                  />
                                  {pushLabel}
                                </button>
                              </div>
                            </div>
                          </div>

                          {linkedStorages.length > 1 && (
                            <div
                              ref={storagePickerMenuRef}
                              id={`storage-picker-${project.local_project_id}`}
                              className="storage-link-menu"
                              role="menu"
                              aria-label="Switch storage"
                              hidden={!storagePickerOpen}
                              onKeyDown={(event) => {
                                const options = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(
                                  '[role="menuitem"]',
                                ));
                                if (options.length === 0) return;
                                const currentIndex = options.indexOf(document.activeElement as HTMLButtonElement);
                                let nextIndex: number | null = null;
                                if (event.key === "ArrowDown") {
                                  nextIndex = currentIndex < 0 ? 0 : (currentIndex + 1) % options.length;
                                }
                                if (event.key === "ArrowUp") {
                                  nextIndex = currentIndex < 0
                                    ? options.length - 1
                                    : (currentIndex - 1 + options.length) % options.length;
                                }
                                if (event.key === "Home") nextIndex = 0;
                                if (event.key === "End") nextIndex = options.length - 1;
                                if (nextIndex === null) return;
                                event.preventDefault();
                                options[nextIndex]?.focus();
                              }}
                            >
                              {linkedStorages.filter((linkedStorage) => linkedStorage.id !== storage.id).map((linkedStorage) => (
                                <button
                                  key={linkedStorage.id}
                                  type="button"
                                  role="menuitem"
                                  className="storage-link-menu-option"
                                  disabled={storageControlsLocked}
                                  aria-label={`Switch to ${linkedStorage.name || "unnamed storage"}`}
                                  onClick={() => {
                                    setStoragePickerProjectId(null);
                                    if (reviewOpen) inlineStorageReview?.onClose();
                                    void onSelectStorage(project.local_project_id, linkedStorage.id);
                                    window.requestAnimationFrame(() => storagePickerTriggerRef.current?.focus());
                                  }}
                                >
                                  <span className="storage-link-menu-icon" aria-hidden="true">
                                    <Icon name={linkedStorage.kind === "local" ? "drive" : "cloud"} size={20} />
                                  </span>
                                  <span className="storage-link-menu-copy">
                                    <strong>{linkedStorage.name || "(unnamed)"}</strong>
                                    <span title={storageSubtitle(linkedStorage)}>{storageSubtitle(linkedStorage)}</span>
                                  </span>
                                  <Icon className="storage-link-menu-arrow" name="chevron-right" size={13} aria-hidden="true" />
                                </button>
                              ))}
                            </div>
                          )}

                          {reviewOpen && (
                            <div id={reviewPanelId} className="v3-storage-inline-review">
                              {inlineStorageReview?.content}
                            </div>
                          )}
                        </div>
                      );
                    })()}
                    </div>

                    {linkingProjectId === project.local_project_id && (
                      <div className="project-profile-group-footer">
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
          <ProjectWorkspaceTabs
            activeTab={activeProjectTab}
            isGitRepository={!!activeProject.is_git_repository}
            onChange={setActiveProjectTab}
          />
        )}

        {activeProject && !newProjectSetup && !inlineStorageReview && activeProjectTab === "history" && (
          <div
            id="project-history-panel"
            className="v3-project-tab-panel"
            role="tabpanel"
            aria-labelledby="project-history-tab"
          >
            <ProjectChatHistoryPage
              embedded
              project={activeProject}
              binding={activeBindingByProject.get(activeProject.local_project_id) ?? null}
              refreshEpoch={historyRefreshEpoch}
              activeStorageId={activeStorageId}
              activeStorageName={storages.find((storage) => storage.id === activeStorageId)?.name ?? null}
            />
          </div>
        )}

        {activeProject && !newProjectSetup && !inlineStorageReview && (
          activeProjectTab === "skills" || activeProjectTab === "plugins"
        ) && (
          <div
            id={`project-${activeProjectTab}-panel`}
            className="v3-project-tab-panel"
            role="tabpanel"
            aria-labelledby={`project-${activeProjectTab}-tab`}
          >
            <SkillsPluginStatusPage
              view={activeProjectTab}
              project={activeProject}
              binding={activeBindingByProject.get(activeProject.local_project_id) ?? null}
              refreshEpoch={historyRefreshEpoch}
              activeStorageId={activeStorageId}
              activeStorageName={storages.find((storage) => storage.id === activeStorageId)?.name ?? null}
              onOpenProjectSettings={() => {
                document.getElementById("project-configuration-panel")?.scrollIntoView({
                  block: "start",
                  behavior: "smooth",
                });
              }}
            />
          </div>
        )}
      </section>

    </main>
  );
}
