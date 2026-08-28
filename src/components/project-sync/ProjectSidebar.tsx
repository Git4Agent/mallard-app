import { useEffect, useRef, useState } from "react";
import type {
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import packageMetadata from "../../../package.json";
import type {
  LocalProjectSummary,
  SetupDraftSummary,
  StorageConfig,
  StorageSetupDraft,
} from "../../types";
import Icon from "../Icons";
import { catalogKindMeta, catalogScopeLabel } from "../storage-catalog/model";
import type { CatalogScopeView } from "../storage-catalog/types";
import { projectLabel } from "./model";

/* Both gutters down the sections box drag — they are the same 3px of frame
   colour, so a fixed bottom row read as a handle and answered to nothing.
   Projects and Catalog divide the elastic space by share. Storages does not:
   it holds an unbounded list of configured destinations, so it is sized to
   its contents up to a 40% cap until the lower handle pins it, and the list
   scrolls past either limit rather than squeezing the panes above onto their
   floors. Its floor rides on the pane rather than the track: fit-content()
   is a track size and cannot go inside minmax(), and a content-sized track
   takes its base from the item's minimum contribution, which the split pane
   zeroes — so this was the one track a short box could reclaim entirely,
   leaving two pixels of clipped heading where a section should be. */
type SectionLayout = {
  headHeight: number | null;
  projectShare: number;
  storageHeight: number | null;
};
/* 0 is the head over Projects, 1 Projects over Catalog, 2 Catalog over
   Storages — top to bottom, the order they appear down the rail. */
type SectionHandle = 0 | 1 | 2;

const SIDEBAR_SECTION_SPLIT_KEY = "mallard.sidebar-section-split";
const DEFAULT_PROJECT_SHARE = 0.56;
const MIN_SECTION_SHARE = 0.2;
export const MIN_PANE_HEIGHT = 96;
const SECTION_KEYBOARD_STEP = 12;

function clampSectionShare(value: number): number {
  return Math.min(1 - MIN_SECTION_SHARE, Math.max(MIN_SECTION_SHARE, value));
}

/* Storages floors at the same MIN_PANE_HEIGHT as the panes above it — pinned
   under its contents it scrolls like they do, rather than refusing to move. The
   ceiling stops once Catalog, the pane this handle actually borders, is down to
   its own floor, so the handle never starts eating Projects instead. */
export function storageHeightRange(boxHeight: number, projectsHeight: number): [number, number] {
  return [MIN_PANE_HEIGHT, Math.max(MIN_PANE_HEIGHT, boxHeight - 6 - projectsHeight - MIN_PANE_HEIGHT)];
}

function clampStorageHeight(value: number, boxHeight: number, projectsHeight: number): number {
  const [min, max] = storageHeightRange(boxHeight, projectsHeight);
  return Math.min(max, Math.max(min, value));
}

/* Projects and Catalog split what Storages leaves behind, so pinning Storages
   changes the meaning of a fixed share and drags the gutter above it along.
   Re-solving the share against the new elastic space keeps Projects on exactly
   the pixels it had, which is what makes this handle move one gutter only. */
export function shareHolding(projectsHeight: number, boxHeight: number, storageHeight: number): number {
  const elastic = boxHeight - 6 - storageHeight;
  return elastic > 0 ? clampSectionShare(projectsHeight / elastic) : DEFAULT_PROJECT_SHARE;
}

/* The head's handle changes the size of the whole box beneath it, which moves
   both gutters below unless the share is re-solved. Holding Catalog on its
   pixels leaves Projects — the pane this handle borders — to absorb the change,
   and Storages is sized in its own right so it never moves either way. */
export function shareHoldingCatalog(
  catalogHeight: number,
  boxHeight: number,
  storageHeight: number,
): number {
  const elastic = boxHeight - 6 - storageHeight;
  return elastic > 0 ? clampSectionShare(1 - catalogHeight / elastic) : DEFAULT_PROJECT_SHARE;
}

/* The head can shed height down to its wordmark plus one nav row — below that a
   row would be unreachable — and gains nothing past its natural content height,
   which is only empty tile. The nav scrolls in between. */
function headHeightRange(head: HTMLElement): [number, number] {
  const rowOf = (selector: string) => head.querySelector<HTMLElement>(selector)?.offsetHeight ?? 0;
  const nav = head.querySelector<HTMLElement>(".sync-primary-nav");
  const navStyle = nav ? getComputedStyle(nav) : null;
  const navChrome = navStyle
    ? Number.parseFloat(navStyle.paddingTop) + Number.parseFloat(navStyle.paddingBottom)
    : 0;
  // Everything above the nav, plus the tile's own borders. Both ends are then
  // measured off the nav's scroll height rather than the head's — the head
  // reports whatever the pin already gave it, so a shortened head would keep
  // lowering its own ceiling and could never be dragged back open.
  const above = rowOf(".sync-sidebar-drag") + rowOf(".sync-sidebar-brand")
    + head.offsetHeight - head.clientHeight;
  const min = above + navChrome + (nav?.querySelector<HTMLElement>("button")?.offsetHeight ?? 0);
  return [min, Math.max(min, above + (nav?.scrollHeight ?? 0))];
}

function storedSectionLayout(): SectionLayout {
  const fallback: SectionLayout = {
    headHeight: null,
    projectShare: DEFAULT_PROJECT_SHARE,
    storageHeight: null,
  };
  if (typeof window === "undefined") return fallback;
  let stored = "";
  try {
    stored = window.localStorage.getItem(SIDEBAR_SECTION_SPLIT_KEY) ?? "";
  } catch {
    return fallback;
  }
  const [share, height, head] = stored.split(",");
  const parsedShare = Number.parseFloat(share);
  if (!Number.isFinite(parsedShare)) return fallback;
  const parsedHeight = Number.parseFloat(height);
  const parsedHead = Number.parseFloat(head);
  return {
    headHeight: Number.isFinite(parsedHead) ? parsedHead : null,
    projectShare: clampSectionShare(parsedShare),
    storageHeight: Number.isFinite(parsedHeight) ? Math.max(MIN_PANE_HEIGHT, parsedHeight) : null,
  };
}

/* Whole fr factors, not the shares themselves. Grid gives a track whose flex
   factor is below 1 only that fraction of the leftover space — so once a pane
   bottoms out on its 96px floor and the factors still in play add up to less
   than 1, the rows stop filling the box and leave dead space under the last
   pane. Scaled to 100 the remaining factor is always well over 1. The first
   track is Projects and the second is Catalog. */
function elasticRows(projectShare: number): string {
  const projects = projectShare * 100;
  return `minmax(${MIN_PANE_HEIGHT}px, ${projects}fr) 3px minmax(${MIN_PANE_HEIGHT}px, ${100 - projects}fr)`;
}

interface Props {
  projects: LocalProjectSummary[];
  drafts: SetupDraftSummary[];
  activeDraftId: string | null;
  storages: StorageConfig[];
  /** Persisted New-storage forms; rows the same way project drafts are. */
  storageDrafts: StorageSetupDraft[];
  storageUsage: Record<string, number>;
  storageIssues?: Record<string, string>;
  activeProjectId: string | null;
  activeStorageId: string | null;
  catalogScopes: CatalogScopeView[];
  activeCatalogScopeId: string | null;
  addingCatalogScope: boolean;
  settingsOpen: boolean;
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
  onOpenStorage: (storageId: string, settingsExpanded: boolean) => void;
  onOpenStorageDraft: (draftId: string) => void;
  onDiscardStorageDraft: (draftId: string) => void;
  onRemoveStorage: (storageId: string) => void;
  onAddStorage: () => void;
  onOpenCatalogMenu: () => void;
  onOpenCatalogScope: (scopeId: string) => void;
  onAddCatalogScope: () => void;
  onRemoveCatalogScope: (scopeId: string) => void;
  onOpenSettings: () => void;
}

export default function ProjectSidebar({
  projects,
  drafts,
  activeDraftId,
  storages,
  storageDrafts,
  storageUsage,
  storageIssues = {},
  activeProjectId,
  activeStorageId,
  catalogScopes,
  activeCatalogScopeId,
  addingCatalogScope,
  settingsOpen,
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
  onOpenStorage,
  onOpenStorageDraft,
  onDiscardStorageDraft,
  onRemoveStorage,
  onAddStorage,
  onOpenCatalogMenu,
  onOpenCatalogScope,
  onAddCatalogScope,
  onRemoveCatalogScope,
  onOpenSettings,
}: Props) {
  const [sectionLayout, setSectionLayout] = useState<SectionLayout>(storedSectionLayout);
  const [resizingSections, setResizingSections] = useState(false);
  const sectionsRef = useRef<HTMLDivElement>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const sectionResizeRef = useRef<
    {
      pointerId: number;
      handle: SectionHandle;
      top: number;
      bottom: number;
      height: number;
      projects: number;
      catalog: number;
      storage: number;
      headTop: number;
      headStart: number;
      headRange: [number, number];
    } | null
  >(null);

  /* The list is fixed, so its height is the one number the pin must respect. It
     is read off the laid-out rows rather than the pane, which reports whatever
     the pin already gave it. */
  const paneHeight = (selector: string) => sectionsRef.current
    ?.querySelector<HTMLElement>(selector)
    ?.getBoundingClientRect().height ?? MIN_PANE_HEIGHT;

  const storageContentHeight = () => {
    const pane = sectionsRef.current?.querySelector<HTMLElement>(".sidebar-storage-pane");
    if (!pane) return MIN_PANE_HEIGHT;
    const heading = pane.querySelector<HTMLElement>(".sidebar-section-heading");
    const list = pane.querySelector<HTMLElement>(".sidebar-links-list");
    const style = getComputedStyle(pane);
    // The tile's own border and padding sit outside the rows it holds, so they
    // count toward the floor as well — a pin measured without them shaves the
    // bottom row by a hair.
    const chrome = pane.offsetHeight - pane.clientHeight
      + Number.parseFloat(style.paddingTop) + Number.parseFloat(style.paddingBottom);
    return (heading?.offsetHeight ?? 0) + (list?.scrollHeight ?? 0) + chrome;
  };

  useEffect(() => {
    try {
      const pixels = (value: number | null) => (value === null ? "auto" : String(Math.round(value)));
      window.localStorage.setItem(
        SIDEBAR_SECTION_SPLIT_KEY,
        `${sectionLayout.projectShare},${pixels(sectionLayout.storageHeight)},${pixels(sectionLayout.headHeight)}`,
      );
    } catch {
      // The splitter still works for this session when browser storage is unavailable.
    }
  }, [sectionLayout]);

  const startSectionResize = (event: ReactPointerEvent<HTMLDivElement>, handle: SectionHandle) => {
    if (event.button !== 0) return;
    const bounds = sectionsRef.current?.getBoundingClientRect();
    if (!bounds || bounds.height <= 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    sectionResizeRef.current = {
      pointerId: event.pointerId,
      handle,
      top: bounds.top,
      bottom: bounds.bottom,
      height: bounds.height,
      projects: paneHeight(".sidebar-project-pane"),
      catalog: paneHeight(".sidebar-catalog-pane"),
      storage: paneHeight(".sidebar-storage-pane"),
      headTop: headRef.current?.getBoundingClientRect().top ?? 0,
      headStart: headRef.current?.getBoundingClientRect().height ?? 0,
      headRange: headRef.current ? headHeightRange(headRef.current) : [0, 0],
    };
    setResizingSections(true);
  };

  const continueSectionResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const resize = sectionResizeRef.current;
    if (!resize || resize.pointerId !== event.pointerId) return;
    if (resize.handle === 0) {
      // The head is a flex tile above the box, so this handle sets its height
      // directly. The rail is a fixed column, so whatever the head takes the box
      // below loses — re-solve the share against that new box so Catalog keeps
      // its pixels and only Projects, the pane this handle borders, gives way.
      const [min, max] = resize.headRange;
      const height = Math.min(max, Math.max(min, event.clientY - resize.headTop));
      const box = resize.height - (height - resize.headStart);
      setSectionLayout((current) => ({
        ...current,
        headHeight: height,
        projectShare: shareHoldingCatalog(resize.catalog, box, resize.storage),
      }));
      return;
    }
    if (resize.handle === 1) {
      const share = clampSectionShare((event.clientY - resize.top) / resize.height);
      setSectionLayout((current) => ({ ...current, projectShare: share }));
      return;
    }
    // The lowest handle sets the Storages pane's own height, measured up from the
    // bottom of the box, and re-solves the share so Projects keeps the pixels it
    // started with — only Catalog, the pane this handle borders, gives way.
    const height = clampStorageHeight(resize.bottom - event.clientY, resize.height, resize.projects);
    setSectionLayout((current) => ({
      ...current,
      projectShare: shareHolding(resize.projects, resize.height, height),
      storageHeight: height,
    }));
  };

  const finishSectionResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (sectionResizeRef.current?.pointerId !== event.pointerId) return;
    sectionResizeRef.current = null;
    setResizingSections(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const resizeSectionsWithKeyboard = (
    event: ReactKeyboardEvent<HTMLDivElement>,
    handle: SectionHandle,
  ) => {
    if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    if (handle === 0) {
      const head = headRef.current;
      if (!head) return;
      const [min, max] = headHeightRange(head);
      const from = sectionLayout.headHeight ?? head.getBoundingClientRect().height;
      const next = event.key === "ArrowUp" ? from - SECTION_KEYBOARD_STEP
        : event.key === "ArrowDown" ? from + SECTION_KEYBOARD_STEP
        : event.key === "Home" ? min
        : max;
      setSectionLayout((current) => ({ ...current, headHeight: Math.min(max, Math.max(min, next)) }));
      return;
    }
    if (handle === 1) {
      const share = sectionLayout.projectShare;
      const next = event.key === "ArrowUp" ? share - 0.05
        : event.key === "ArrowDown" ? share + 0.05
        : event.key === "Home" ? MIN_SECTION_SHARE
        : 1 - MIN_SECTION_SHARE;
      setSectionLayout((current) => ({ ...current, projectShare: clampSectionShare(next) }));
      return;
    }
    const box = sectionsRef.current?.getBoundingClientRect().height ?? 0;
    const projects = paneHeight(".sidebar-project-pane");
    const [min, max] = storageHeightRange(box, projects);
    const current = sectionLayout.storageHeight ?? storageContentHeight();
    const next = event.key === "ArrowUp" ? current + SECTION_KEYBOARD_STEP
      : event.key === "ArrowDown" ? current - SECTION_KEYBOARD_STEP
      : event.key === "Home" ? max
      : min;
    const height = clampStorageHeight(next, box, projects);
    setSectionLayout((layout) => ({
      ...layout,
      projectShare: shareHolding(projects, box, height),
      storageHeight: height,
    }));
  };

  const resetSection = (handle: SectionHandle) => {
    if (handle === 0) {
      // Back to the height its own rows ask for.
      setSectionLayout((current) => ({ ...current, headHeight: null }));
      return;
    }
    if (handle === 1) {
      setSectionLayout((current) => ({ ...current, projectShare: DEFAULT_PROJECT_SHARE }));
      return;
    }
    // Back to fitting its contents exactly — the state a fixed list wants — and
    // holding Projects where it is, the same as a drag of this handle.
    const box = sectionsRef.current?.getBoundingClientRect().height ?? 0;
    const projects = sectionsRef.current
      ?.querySelector<HTMLElement>(".sidebar-project-pane")
      ?.getBoundingClientRect().height ?? MIN_PANE_HEIGHT;
    setSectionLayout((current) => ({
      ...current,
      projectShare: shareHolding(projects, box, storageContentHeight()),
      storageHeight: null,
    }));
  };

  /* Every gutter wears the same handle; only what it resizes differs. */
  const dividerProps = (handle: SectionHandle, label: string) => ({
    className: "sidebar-section-divider",
    role: "separator",
    "aria-label": label,
    "aria-orientation": "horizontal" as const,
    tabIndex: 0,
    onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => startSectionResize(event, handle),
    onPointerMove: continueSectionResize,
    onPointerUp: finishSectionResize,
    onPointerCancel: finishSectionResize,
    onKeyDown: (event: ReactKeyboardEvent<HTMLDivElement>) => resizeSectionsWithKeyboard(event, handle),
    onDoubleClick: () => resetSection(handle),
  });

  const primaryStorageId = activeStorageId && storages.some((storage) => storage.id === activeStorageId)
    ? activeStorageId
    : storages[0]?.id ?? null;

  return (
    <aside id="project-sidebar" className="sync-sidebar">
      <div
        ref={headRef}
        className="sync-sidebar-head"
        style={sectionLayout.headHeight === null
          ? undefined
          : { height: `${sectionLayout.headHeight}px`, flexBasis: `${sectionLayout.headHeight}px` }}
      >
        <div className="sync-sidebar-drag" data-tauri-drag-region />
        <div className="sync-sidebar-brand" data-tauri-drag-region>
          <img className="sync-brand-logo" src="/mallard-logo-duck.svg" alt="" data-tauri-drag-region />
          <strong>Mallard</strong>
        </div>

        <nav className="sync-primary-nav" aria-label="Project sync navigation">
          <button
            type="button"
            className={!activeStorageId && !activeCatalogScopeId && !addingCatalogScope && !settingsOpen ? "active" : undefined}
            onClick={() => activeProjectId && onSelectProject(activeProjectId)}
            disabled={busy}
          >
            <Icon name="folder" size={15} /> Projects
          </button>
          <button
            type="button"
            className={activeCatalogScopeId || addingCatalogScope ? "active" : undefined}
            onClick={onOpenCatalogMenu}
            disabled={busy}
          >
            <Icon name="catalog" size={15} /> Catalog
          </button>
          <button
            type="button"
            className={activeStorageId ? "active" : undefined}
            onClick={() => primaryStorageId && onOpenStorage(primaryStorageId, false)}
            disabled={busy || !primaryStorageId}
          >
            <Icon name="drive" size={15} /> Storages
          </button>
          <button
            type="button"
            className={settingsOpen ? "active" : undefined}
            onClick={onOpenSettings}
            disabled={busy}
          >
            <Icon name="settings" size={15} /> Settings
          </button>
          <button
            type="button"
            className={activityOpen ? "active" : undefined}
            onClick={onToggleActivity}
            aria-expanded={activityOpen}
          >
            <Icon name="activity" size={15} /> Logs
            {unreadLogs > 0 && !activityOpen && (
              <span className="sidebar-nav-badge sync-activity-badge">{unreadLogs}</span>
            )}
          </button>
        </nav>
      </div>

      <div
        {...dividerProps(0, "Resize navigation and Projects sections")}
        className="sidebar-section-divider sidebar-head-divider"
        aria-valuenow={sectionLayout.headHeight === null ? undefined : Math.round(sectionLayout.headHeight)}
        aria-valuetext={sectionLayout.headHeight === null
          ? "Navigation height: fits contents"
          : `Navigation height: ${Math.round(sectionLayout.headHeight)} pixels`}
      />

      <div
        ref={sectionsRef}
        className={`sync-sidebar-sections sidebar-settings-sections${resizingSections ? " resizing" : ""}`}
        style={{
          gridTemplateRows: `${elasticRows(sectionLayout.projectShare)} 3px ${sectionLayout.storageHeight === null ? "fit-content(40%)" : `${sectionLayout.storageHeight}px`}`,
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
                className={`sidebar-profile-item sync-sidebar-draft${activeDraftId === draft.draft_id ? " active" : ""}`}
              >
                <button
                  type="button"
                  className="sidebar-profile-main"
                  onClick={() => onSelectDraft(draft.draft_id)}
                  disabled={busy}
                  title={draft.last_error
                    ? `${draft.project_root} — last attempt failed: ${draft.last_error}`
                    : `${draft.project_root} — resumable setup draft`}
                  aria-label={draft.last_error
                    ? `${draft.display_name} draft — last attempt failed: ${draft.last_error}`
                    : `${draft.display_name} — resumable setup draft`}
                >
                  <Icon name="folder" size={15} />
                  <span>{draft.display_name}</span>
                  {/* Attention is the draft that points at a storage or a
                      profile the machine no longer has, and the only thing
                      saying so was the badge's tint — the same word, in a
                      different colour, which is nothing at all to a reader who
                      cannot separate the two (WCAG 1.4.1). The triangle is the
                      second channel, the shape the two warnings further down
                      this rail already use, so the row reads as flagged in
                      greyscale. The word stays "Draft": it is what marks the
                      row as a draft at all, and swapping it for the state
                      would cost more than it bought. */}
                  <span className={`sync-draft-badge${draft.status === "attention" ? " attention" : ""}`}>
                    {draft.status === "attention" && (
                      <Icon name="alert-triangle" size={12} aria-hidden="true" />
                    )}
                    Draft
                  </span>
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
              <button type="button" className="sync-sidebar-empty" onClick={onAddProject} disabled={busy}>
                <Icon name="plus" size={15} /> Add your first project
              </button>
            ) : projects.map((project) => {
              const profileRequired = Object.keys(project.profile_ids ?? {}).length === 0;
              const label = projectLabel(project);
              const aliased = label !== project.display_name;
              return (
                <div
                  key={project.local_project_id}
                  className={`sidebar-profile-item${!activeDraftId && !activeStorageId && !activeCatalogScopeId && !addingCatalogScope && !settingsOpen && activeProjectId === project.local_project_id ? " active" : ""}${profileRequired ? " needs-profile" : ""}`}
                >
                  <button
                    type="button"
                    className="sidebar-profile-main"
                    onClick={() => onSelectProject(project.local_project_id)}
                    disabled={busy}
                    /* The triangle below is decorative, and the button was
                       named by the project label alone, so the warning was
                       drawn and never spoken — a reader who does not see the
                       rail learned nothing was wrong. The storage rows further
                       down already name their issue on the button; this says
                       the same thing the same way. */
                    aria-label={[
                      project.is_git_repository === true ? `${label}, Git repository` : label,
                      profileRequired ? "Project issue: No agent profile is assigned" : null,
                    ].filter(Boolean).join(". ")}
                    title={[
                      project.is_git_repository === true ? "Git repository" : null,
                      aliased ? `Repo: ${project.display_name}` : null,
                      project.project_root ?? null,
                    ].filter(Boolean).join("\n") || label}
                  >
                    <Icon
                      name={project.is_git_repository === true ? "git-folder" : "folder"}
                      size={16}
                      className={project.is_git_repository === true ? "sync-project-git-icon" : "sync-project-folder-icon"}
                    />
                    <span>{label}</span>
                    {profileRequired && (
                      <Icon
                        name="alert-triangle"
                        size={12}
                        className="sync-sidebar-profile-warning"
                        aria-hidden="true"
                      />
                    )}
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
          {...dividerProps(1, "Resize Projects and Catalog sections")}
          aria-valuemin={Math.round(MIN_SECTION_SHARE * 100)}
          aria-valuemax={Math.round((1 - MIN_SECTION_SHARE) * 100)}
          aria-valuenow={Math.round(sectionLayout.projectShare * 100)}
        />

        <section className="sidebar-split-pane sidebar-catalog-pane" aria-label="Catalog">
          <div className="sidebar-section-heading">
            <div className="sidebar-section-label">
              <span className="sidebar-section-title">Catalog</span>
            </div>
            <button
              type="button"
              className="sidebar-section-action sidebar-add-action"
              onClick={onAddCatalogScope}
              disabled={busy}
              title="Add a resource: one kind from one agent profile"
              aria-label="Add Catalog resource"
            >
              <Icon name="plus" size={16} />
            </button>
          </div>
          <div className="sidebar-profile-list">
            {catalogScopes.length === 0 ? (
              <button
                type="button"
                className="sync-sidebar-empty"
                onClick={onAddCatalogScope}
                disabled={busy}
              >
                <Icon name="plus" size={15} /> Add your first resource
              </button>
            ) : catalogScopes.map((scope) => {
              const label = catalogScopeLabel(scope.scope, scope.profile);
              return (
                <div
                  key={scope.scope.scope_id}
                  className={`sidebar-profile-item${activeCatalogScopeId === scope.scope.scope_id ? " active" : ""}`}
                >
                  <button
                    type="button"
                    className="sidebar-profile-main"
                    onClick={() => onOpenCatalogScope(scope.scope.scope_id)}
                    disabled={busy}
                    title={`${label} — ${scope.profile.configured_home}`}
                  >
                    <Icon name={catalogKindMeta(scope.scope.resource_kind).icon} size={16} />
                    <span>{label}</span>
                  </button>
                  <div className="sidebar-profile-actions">
                    <button
                      type="button"
                      className="sidebar-profile-remove"
                      onClick={() => onRemoveCatalogScope(scope.scope.scope_id)}
                      disabled={busy}
                      title={`Remove the ${label} row; no provider or storage content is touched`}
                      aria-label={`Remove the ${label} row`}
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
          {...dividerProps(2, "Resize Catalog and Storages sections")}
          aria-valuemin={MIN_PANE_HEIGHT}
          aria-valuenow={sectionLayout.storageHeight === null ? undefined : Math.round(sectionLayout.storageHeight)}
          aria-valuetext={sectionLayout.storageHeight === null
            ? "Storage height: fits contents"
            : `Storage height: ${Math.round(sectionLayout.storageHeight)} pixels`}
        />

        <section
          className="sidebar-split-pane sidebar-storage-pane"
          aria-label="Storages"
          style={{ minHeight: `${MIN_PANE_HEIGHT}px` }}
        >
          <div className="sidebar-section-heading">
            <div className="sidebar-section-label">
              <span className="sidebar-section-title">Storages</span>
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
            {storages.length === 0 && storageDrafts.length === 0 ? (
              <button type="button" className="sync-sidebar-empty" onClick={onAddStorage} disabled={busy}>
                <Icon name="plus" size={15} /> Add your first storage
              </button>
            ) : storages.map((storage) => {
              const usage = storageUsage[storage.id] ?? 0;
              const storageName = storage.name || "storage";
              const storageIssue = storageIssues[storage.id] ?? null;
              return (
                <div
                  key={storage.id}
                  className={`sidebar-profile-item${activeStorageId === storage.id ? " active" : ""}`}
                >
                  <button
                    type="button"
                    className="sidebar-profile-main"
                    onClick={() => onOpenStorage(storage.id, false)}
                    disabled={busy}
                    title={storageIssue ? `Open ${storageName} — ${storageIssue}` : `Open ${storageName}`}
                    aria-label={storageIssue
                      ? `Open ${storageName}. Storage issue: ${storageIssue}`
                      : `Open ${storageName}`}
                  >
                    <Icon name={storage.kind === "local" ? "drive" : "cloud"} size={16} />
                    <span>{storage.name || "(unnamed)"}</span>
                    {storageIssue && (
                      <Icon
                        name="alert-triangle"
                        size={12}
                        className="sync-sidebar-storage-warning"
                        aria-hidden="true"
                      />
                    )}
                  </button>
                  <div className="sidebar-profile-actions">
                    <button
                      type="button"
                      className="sidebar-profile-remove"
                      onClick={() => onRemoveStorage(storage.id)}
                      disabled={busy || usage > 0}
                      title={usage > 0
                        ? `${storageName} is linked to ${usage} project${usage === 1 ? "" : "s"}; unlink it before removing`
                        : `Remove ${storageName}; synced files stay in storage`}
                      aria-label={`Remove ${storageName} from Mallard`}
                    >
                      <Icon name="trash" size={13} />
                    </button>
                  </div>
                </div>
              );
            })}
            {/* A storage that was never created rides the same rail as the
                ones that were, the way project drafts ride the Projects pane:
                open resumes the form, the trash discards only the draft. The
                workspace reports the draft's storage id as the open editor,
                so the active marker needs no extra state. */}
            {storageDrafts.map((draft) => {
              const draftName = draft.storage.name || "(unnamed)";
              return (
                <div
                  key={draft.draft_id}
                  className={`sidebar-profile-item sync-sidebar-draft${activeStorageId === draft.storage.id ? " active" : ""}`}
                >
                  <button
                    type="button"
                    className="sidebar-profile-main"
                    onClick={() => onOpenStorageDraft(draft.draft_id)}
                    disabled={busy}
                    title={`Continue setting up ${draftName}`}
                    aria-label={`Continue setting up ${draftName} — resumable storage draft`}
                  >
                    <Icon name={draft.storage.kind === "local" ? "drive" : "cloud"} size={16} />
                    <span>{draftName}</span>
                    <span className="sync-draft-badge">Draft</span>
                  </button>
                  <div className="sidebar-profile-actions">
                    <button
                      type="button"
                      className="sidebar-profile-remove"
                      onClick={() => onDiscardStorageDraft(draft.draft_id)}
                      disabled={busy}
                      title={draft.storage.kind === "s3"
                        ? `Discard the local draft for ${draftName}; Cloudflare bucket and access-key resources are kept`
                        : `Discard the local draft for ${draftName}; the selected folder and its contents are kept`}
                      aria-label={`Discard the storage draft for ${draftName}`}
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

      <footer className="sync-sidebar-product-mark" aria-label={`Mallard version ${packageMetadata.version}`}>
        <img src="/mallard-logo-duck.svg" alt="" />
        <span>v{packageMetadata.version}</span>
      </footer>
    </aside>
  );
}
