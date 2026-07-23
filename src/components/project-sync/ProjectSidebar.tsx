import { useEffect, useRef, useState } from "react";
import type {
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import type { LocalProjectSummary, SetupDraftSummary, StorageConfigV3 } from "../../types";
import Icon from "../Icons";
import { projectLabel } from "./model";

const SIDEBAR_SECTION_SPLIT_KEY = "mallard.sidebar-section-split";
const DEFAULT_PROJECT_SHARE = 0.56;
const MIN_SECTION_SHARE = 0.2;

function clampSectionShare(value: number): number {
  return Math.min(1 - MIN_SECTION_SHARE, Math.max(MIN_SECTION_SHARE, value));
}

function storedProjectShare(): number {
  if (typeof window === "undefined") return DEFAULT_PROJECT_SHARE;
  try {
    const value = Number.parseFloat(window.localStorage.getItem(SIDEBAR_SECTION_SPLIT_KEY) ?? "");
    return Number.isFinite(value) ? clampSectionShare(value) : DEFAULT_PROJECT_SHARE;
  } catch {
    return DEFAULT_PROJECT_SHARE;
  }
}

interface Props {
  projects: LocalProjectSummary[];
  drafts: SetupDraftSummary[];
  activeDraftId: string | null;
  storages: StorageConfigV3[];
  storageUsage: Record<string, number>;
  activeProjectId: string | null;
  activeStorageId: string | null;
  loading: boolean;
  busy: boolean;
  activityOpen: boolean;
  unreadLogs: number;
  onSelectProject: (projectId: string) => void;
  onRemoveProject: (projectId: string) => void;
  onSelectDraft: (draftId: string) => void;
  onDiscardDraft: (draftId: string) => void;
  onToggleActivity: () => void;
  onAddProject: () => void;
  onRefresh: () => void;
  onOpenStorage: (storageId: string, settingsExpanded: boolean) => void;
  onRemoveStorage: (storageId: string) => void;
  onAddStorage: () => void;
}

export default function ProjectSidebar({
  projects,
  drafts,
  activeDraftId,
  storages,
  storageUsage,
  activeProjectId,
  activeStorageId,
  loading,
  busy,
  activityOpen,
  unreadLogs,
  onSelectProject,
  onRemoveProject,
  onSelectDraft,
  onDiscardDraft,
  onToggleActivity,
  onAddProject,
  onRefresh,
  onOpenStorage,
  onRemoveStorage,
  onAddStorage,
}: Props) {
  const [projectShare, setProjectShare] = useState(storedProjectShare);
  const [resizingSections, setResizingSections] = useState(false);
  const sectionsRef = useRef<HTMLDivElement>(null);
  const sectionResizeRef = useRef<{ pointerId: number; top: number; height: number } | null>(null);

  useEffect(() => {
    try {
      window.localStorage.setItem(SIDEBAR_SECTION_SPLIT_KEY, String(projectShare));
    } catch {
      // The splitter still works for this session when browser storage is unavailable.
    }
  }, [projectShare]);

  const startSectionResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const bounds = sectionsRef.current?.getBoundingClientRect();
    if (!bounds || bounds.height <= 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    sectionResizeRef.current = {
      pointerId: event.pointerId,
      top: bounds.top,
      height: bounds.height,
    };
    setResizingSections(true);
  };

  const continueSectionResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const resize = sectionResizeRef.current;
    if (!resize || resize.pointerId !== event.pointerId) return;
    setProjectShare(clampSectionShare((event.clientY - resize.top) / resize.height));
  };

  const finishSectionResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (sectionResizeRef.current?.pointerId !== event.pointerId) return;
    sectionResizeRef.current = null;
    setResizingSections(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const resizeSectionsWithKeyboard = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    let nextShare: number | null = null;
    if (event.key === "ArrowUp") nextShare = projectShare - 0.05;
    if (event.key === "ArrowDown") nextShare = projectShare + 0.05;
    if (event.key === "Home") nextShare = MIN_SECTION_SHARE;
    if (event.key === "End") nextShare = 1 - MIN_SECTION_SHARE;
    if (nextShare === null) return;
    event.preventDefault();
    setProjectShare(clampSectionShare(nextShare));
  };

  return (
    <aside id="project-sidebar" className="v3-sidebar">
      <div className="v3-sidebar-drag" data-tauri-drag-region />
      <div className="v3-sidebar-brand" data-tauri-drag-region>
        <img className="v3-brand-logo" src="/mallard-logo.svg" alt="" data-tauri-drag-region />
        <strong>Mallard</strong>
      </div>

      <nav className="v3-primary-nav" aria-label="Project sync navigation">
        <button
          type="button"
          className="active"
          onClick={() => activeProjectId && onSelectProject(activeProjectId)}
          disabled={busy}
        >
          <Icon name="folder" size={15} /> Projects
        </button>
        <button
          type="button"
          className={activityOpen ? "active" : undefined}
          onClick={onToggleActivity}
          aria-expanded={activityOpen}
        >
          <Icon name="activity" size={15} /> Log
          {unreadLogs > 0 && !activityOpen && (
            <span className="sidebar-nav-badge v3-activity-badge">{unreadLogs}</span>
          )}
        </button>
      </nav>

      <div
        ref={sectionsRef}
        className={`v3-sidebar-sections sidebar-settings-sections${resizingSections ? " resizing" : ""}`}
        style={{
          gridTemplateRows: `minmax(96px, ${projectShare}fr) 9px minmax(96px, ${1 - projectShare}fr)`,
        }}
      >
        <section className="sidebar-split-pane sidebar-project-pane" aria-label="Projects">
          <div className="sidebar-section-heading">
            <div className="sidebar-section-label">
              <span className="sidebar-section-title">Projects</span>
            </div>
            <div className="sidebar-heading-actions">
              <button
                type="button"
                className="sidebar-section-action"
                onClick={onRefresh}
                disabled={loading || busy}
                title="Refresh projects"
                aria-label="Refresh projects"
              >
                <Icon name="refresh" size={15} className={loading ? "icon-spin" : undefined} />
              </button>
              <button
                type="button"
                className="sidebar-section-action sidebar-add-action"
                onClick={onAddProject}
                disabled={busy}
                title="Add project"
                aria-label="Add project"
              >
                <Icon name="plus" size={16} />
              </button>
            </div>
          </div>

          <div className="sidebar-profile-list">
            {drafts.map((draft) => (
              <div
                key={draft.draft_id}
                className={`sidebar-profile-item v3-sidebar-draft${activeDraftId === draft.draft_id ? " active" : ""}`}
              >
                <button
                  type="button"
                  className="sidebar-profile-main"
                  onClick={() => onSelectDraft(draft.draft_id)}
                  disabled={busy}
                  title={draft.last_error
                    ? `${draft.project_root} — last attempt failed: ${draft.last_error}`
                    : `${draft.project_root} — resumable setup draft`}
                >
                  <Icon name="folder" size={15} />
                  <span>{draft.display_name}</span>
                  <span className={`v3-draft-badge${draft.status === "attention" ? " attention" : ""}`}>Draft</span>
                </button>
                <div className="sidebar-profile-actions">
                  <button
                    type="button"
                    onClick={() => onSelectDraft(draft.draft_id)}
                    disabled={busy}
                    title={`Continue setup for ${draft.display_name}`}
                    aria-label={`Continue setup for ${draft.display_name}`}
                  >
                    <Icon name="settings" size={13} />
                  </button>
                  <button
                    type="button"
                    className="sidebar-profile-remove"
                    onClick={() => onDiscardDraft(draft.draft_id)}
                    disabled={busy}
                    title={`Discard the setup draft for ${draft.display_name}; no project files are touched`}
                    aria-label={`Discard the setup draft for ${draft.display_name}`}
                  >
                    <Icon name="trash" size={13} />
                  </button>
                </div>
              </div>
            ))}
            {loading && projects.length === 0 && drafts.length === 0 ? (
              <div className="sidebar-msg">Loading projects…</div>
            ) : projects.length === 0 && drafts.length === 0 ? (
              <button type="button" className="v3-sidebar-empty" onClick={onAddProject} disabled={busy}>
                <Icon name="plus" size={15} /> Add your first project
              </button>
            ) : projects.map((project) => {
              const profileRequired = Object.keys(project.profile_ids ?? {}).length !== 1;
              const label = projectLabel(project);
              const aliased = label !== project.display_name;
              return (
                <div
                  key={project.local_project_id}
                  className={`sidebar-profile-item${!activeDraftId && !activeStorageId && activeProjectId === project.local_project_id ? " active" : ""}${profileRequired ? " needs-profile" : ""}`}
                >
                  <button
                    type="button"
                    className="sidebar-profile-main"
                    onClick={() => onSelectProject(project.local_project_id)}
                    disabled={busy}
                    aria-label={project.is_git_repository === true ? `${label}, Git repository` : label}
                    title={[
                      project.is_git_repository === true ? "Git repository" : null,
                      aliased ? `Repo: ${project.display_name}` : null,
                      project.project_root ?? null,
                    ].filter(Boolean).join("\n") || label}
                  >
                    <Icon
                      name={project.is_git_repository === true ? "git-folder" : "folder"}
                      size={16}
                      className={project.is_git_repository === true ? "v3-project-git-icon" : "v3-project-folder-icon"}
                    />
                    <span>{label}</span>
                    {profileRequired && <Icon name="alert-triangle" size={12} className="v3-sidebar-profile-warning" />}
                  </button>
                  <div className="sidebar-profile-actions">
                    <button
                      type="button"
                      className="sidebar-profile-remove"
                      onClick={() => onRemoveProject(project.local_project_id)}
                      disabled={busy}
                      title={`Remove ${label} from Mallard; project files stay on disk`}
                      aria-label={`Remove ${label} from Mallard`}
                    >
                      <Icon name="trash" size={13} />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </section>

        <div
          className="sidebar-section-divider"
          role="separator"
          aria-label="Resize Projects and Storage sections"
          aria-orientation="horizontal"
          aria-valuemin={Math.round(MIN_SECTION_SHARE * 100)}
          aria-valuemax={Math.round((1 - MIN_SECTION_SHARE) * 100)}
          aria-valuenow={Math.round(projectShare * 100)}
          tabIndex={0}
          onPointerDown={startSectionResize}
          onPointerMove={continueSectionResize}
          onPointerUp={finishSectionResize}
          onPointerCancel={finishSectionResize}
          onKeyDown={resizeSectionsWithKeyboard}
          onDoubleClick={() => setProjectShare(DEFAULT_PROJECT_SHARE)}
        />

        <section className="sidebar-split-pane sidebar-storage-pane" aria-label="Storage">
          <div className="sidebar-section-heading">
            <div className="sidebar-section-label">
              <span className="sidebar-section-title">Storage</span>
            </div>
            <button
              type="button"
              className="sidebar-section-action sidebar-add-action"
              onClick={onAddStorage}
              disabled={busy}
              title="Add storage"
              aria-label="Add storage"
            >
              <Icon name="plus" size={16} />
            </button>
          </div>
          <div className="sidebar-links-list">
            {storages.map((storage) => {
              const usage = storageUsage[storage.id] ?? 0;
              const storageName = storage.name || "storage";
              return (
                <div
                  key={storage.id}
                  className={`sidebar-profile-item sidebar-storage-item${activeStorageId === storage.id ? " active" : ""}`}
                >
                  <button
                    type="button"
                    className="sidebar-profile-main"
                    onClick={() => onOpenStorage(storage.id, false)}
                    disabled={busy}
                    title={`Open ${storageName}`}
                    aria-label={`Open ${storageName}`}
                  >
                    <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={16} />
                    <span>{storage.name || "(unnamed)"}</span>
                  </button>
                  <div className="sidebar-profile-actions">
                    <button
                      type="button"
                      onClick={() => onOpenStorage(storage.id, true)}
                      disabled={busy}
                      title={`Storage settings for ${storageName}`}
                      aria-label={`Storage settings for ${storageName}`}
                    >
                      <Icon name="settings" size={13} />
                    </button>
                    <button
                      type="button"
                      className="sidebar-profile-remove"
                      onClick={() => onRemoveStorage(storage.id)}
                      disabled={busy || usage > 0}
                      title={usage > 0
                        ? `${storageName} is linked to ${usage} project${usage === 1 ? "" : "s"}; unlink it before removing`
                        : `Remove ${storageName}; synced files stay in storage`}
                      aria-label={usage > 0
                        ? `Cannot remove ${storageName} while it is linked to a project`
                        : `Remove ${storageName}`}
                    >
                      <Icon name="trash" size={13} />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </section>
      </div>

    </aside>
  );
}
