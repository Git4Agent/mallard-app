import { useEffect, useRef, useState } from "react";
import type {
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
  ReactNode,
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
   zeroes. Put the floor on the pane so a short box cannot reclaim this track
   entirely. */
type SectionLayout = {
  headHeight: number | null;
  projectShare: number;
  storageHeight: number | null;
};
/* 0 is the head over Projects, 1 Projects over Catalog, 2 Catalog over
   Storages — top to bottom, the order they appear down the rail. */
type SectionHandle = 0 | 1 | 2;
/* What a drag on a gutter means: the head's own height, the Projects/Catalog
   share, or the Storages pin. Which operation a handle carries depends on
   which panes are open — see gutterOperation. */
type GutterOperation = "head" | "share" | "pin";

/* The three collapsible panes, in the same rail order. The head never
   collapses — its rows are destinations, not a list to put away. */
export type SectionId = "projects" | "catalog" | "storages";
const SECTION_IDS: readonly SectionId[] = ["projects", "catalog", "storages"];

const SIDEBAR_SECTION_SPLIT_KEY = "mallard.sidebar-section-split";
const SIDEBAR_SECTION_COLLAPSED_KEY = "mallard.sidebar-section-collapsed";
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

/* The share drag tracks the pointer pixel for pixel on the elastic scale: the
   share is Projects' fraction of the two share-sized panes, so it re-derives
   from the panes as measured at grab time plus the pointer's travel. Mapping
   against the whole box would include the Storages row and make the split
   jump when the handle is grabbed. */
export function shareDragged(
  projectsHeight: number,
  catalogHeight: number,
  deltaY: number,
): number {
  const elastic = projectsHeight + catalogHeight;
  return elastic > 0
    ? clampSectionShare((projectsHeight + deltaY) / elastic)
    : DEFAULT_PROJECT_SHARE;
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

function storedCollapsedSections(): ReadonlySet<SectionId> {
  if (typeof window === "undefined") return new Set();
  let stored = "";
  try {
    stored = window.localStorage.getItem(SIDEBAR_SECTION_COLLAPSED_KEY) ?? "";
  } catch {
    return new Set();
  }
  return new Set(
    stored.split(",").filter((id): id is SectionId => (SECTION_IDS as readonly string[]).includes(id)),
  );
}

/* Whole fr factors, not the shares themselves. Grid gives a track whose flex
   factor is below 1 only that fraction of the leftover space — so once a pane
   bottoms out on its 96px floor and the factors still in play add up to less
   than 1, the rows stop filling the box and leave dead space under the last
   pane. Scaled to 100 the remaining factor is always well over 1. The rows are
   Projects, Catalog, and Storages, parted by the two 3px handle tracks.

   A collapsed pane's row drops to min-content — its heading; the 96px floor is
   for lists, and flooring a closed tile would hold it open on empty space. The
   share and the Storages pin ride through a collapse untouched, so reopening
   restores the exact prior split. When both share-sized panes are closed the
   Storages row takes the elastic factor instead of its content size: the
   leftover space always belongs to someone. */
export function sectionGridRows(
  projectShare: number,
  storageHeight: number | null,
  collapsed: ReadonlySet<SectionId>,
): string {
  const projects = projectShare * 100;
  const projectsRow = collapsed.has("projects")
    ? "min-content"
    : `minmax(${MIN_PANE_HEIGHT}px, ${projects}fr)`;
  const catalogRow = collapsed.has("catalog")
    ? "min-content"
    : `minmax(${MIN_PANE_HEIGHT}px, ${100 - projects}fr)`;
  const storagesRow = collapsed.has("storages")
    ? "min-content"
    : collapsed.has("projects") && collapsed.has("catalog")
      ? `minmax(${MIN_PANE_HEIGHT}px, 1fr)`
      : storageHeight === null
        ? "fit-content(40%)"
        : `${storageHeight}px`;
  return `${projectsRow} 3px ${catalogRow} 3px ${storagesRow}`;
}

const SECTION_TITLES: Record<SectionId, string> = {
  projects: "Projects",
  catalog: "Catalog",
  storages: "Storages",
};

/* One disclosure recipe for all three headings (ARIA disclosure pattern): the
   label half of the row is the button, and the add action stays a sibling so
   no button nests inside another. A closed heading still reports its list —
   the row count, and the warning triangle when a row inside is flagged —
   because putting a section away must not put its attention state away with
   it; the triangle is also named on the button, not only drawn, the same
   WCAG 1.4.1 rule the rows themselves follow. */
function SectionHeading({
  section,
  collapsed,
  count,
  flagged,
  onToggle,
  children,
}: {
  section: SectionId;
  collapsed: boolean;
  count: number;
  flagged: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  const title = SECTION_TITLES[section];
  return (
    <div className="sidebar-section-heading">
      {/* Never disabled on busy: collapsing is pure view state, and a control
          that only rearranges the rail should not wait on the backend. */}
      <button
        type="button"
        className="sidebar-section-toggle"
        aria-expanded={!collapsed}
        aria-controls={`sidebar-${section}-list`}
        aria-label={collapsed && flagged
          ? `${title}, ${count} row${count === 1 ? "" : "s"}, contains a flagged row`
          : undefined}
        onClick={onToggle}
      >
        <Icon name={collapsed ? "chevron-right" : "chevron-down"} size={13} aria-hidden="true" />
        <span className="sidebar-section-title">{title}</span>
        {collapsed && flagged && (
          <Icon name="alert-triangle" size={13} className="sidebar-section-alert" aria-hidden="true" />
        )}
        {collapsed && <span className="sidebar-section-count">{count}</span>}
      </button>
      <div className="sidebar-heading-actions">{children}</div>
    </div>
  );
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
  catalogIndexOpen: boolean;
  storageIndexOpen: boolean;
  settingsOpen: boolean;
  loading: boolean;
  busy: boolean;
  activityOpen: boolean;
  unreadLogs: number;
  onOpenProjects: () => void;
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
  onOpenCatalogIndex: () => void;
  onOpenCatalogScope: (scopeId: string) => void;
  onAddCatalogScope: () => void;
  onRemoveCatalogScope: (scopeId: string) => void;
  onOpenStorageIndex: () => void;
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
  catalogIndexOpen,
  storageIndexOpen,
  settingsOpen,
  loading,
  busy,
  activityOpen,
  unreadLogs,
  onOpenProjects,
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
  onOpenCatalogIndex,
  onOpenCatalogScope,
  onAddCatalogScope,
  onRemoveCatalogScope,
  onOpenStorageIndex,
  onOpenSettings,
}: Props) {
  const [sectionLayout, setSectionLayout] = useState<SectionLayout>(storedSectionLayout);
  const [collapsedSections, setCollapsedSections] = useState<ReadonlySet<SectionId>>(storedCollapsedSections);
  const [resizingSections, setResizingSections] = useState(false);
  const sectionsRef = useRef<HTMLDivElement>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const sectionResizeRef = useRef<
    {
      pointerId: number;
      handle: SectionHandle;
      startY: number;
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

  useEffect(() => {
    try {
      window.localStorage.setItem(SIDEBAR_SECTION_COLLAPSED_KEY, [...collapsedSections].join(","));
    } catch {
      // Collapsing still works for this session when browser storage is unavailable.
    }
  }, [collapsedSections]);

  const toggleSection = (section: SectionId) =>
    setCollapsedSections((current) => {
      const next = new Set(current);
      if (!next.delete(section)) next.add(section);
      return next;
    });

  /* The add actions put a row inside their pane, so a collapsed pane reopens
     before the flow starts — otherwise the new row would land out of sight.
     Nothing else reopens a pane: navigating to a destination that lives in a
     closed section respects that the user closed it. */
  const revealSection = (section: SectionId) =>
    setCollapsedSections((current) => {
      if (!current.has(section)) return current;
      const next = new Set(current);
      next.delete(section);
      return next;
    });

  /* With either share-sized pane closed, the share has no rendered meaning —
     min-content rows ignore their fr factor — but it is still what reopening
     restores. Re-solving it against a closed pane's heading-height would
     corrupt that stored split, so the head handle then only moves the box
     edge and the open pane absorbs the change on its own. */
  const shareLocked = collapsedSections.has("projects") || collapsedSections.has("catalog");

  /* What a drag on this gutter can mean right now. The share needs both
     share-sized panes open, and the Storages pin needs Storages open plus an
     open pane above it to absorb the change. A gutter beside a closed Catalog
     passes the pin through — the drag re-pins Storages across the closed
     tile, the way a sash resizes across a collapsed view in VS Code — and a
     gutter with no open pane on one side means nothing at all, so it goes
     inert. */
  const gutterOperation = (handle: SectionHandle): GutterOperation | null => {
    if (handle === 0) return "head";
    const projectsOpen = !collapsedSections.has("projects");
    const catalogOpen = !collapsedSections.has("catalog");
    const storagesOpen = !collapsedSections.has("storages");
    if (handle === 1) {
      if (projectsOpen && catalogOpen) return "share";
      return projectsOpen && storagesOpen ? "pin" : null;
    }
    if (!storagesOpen) return null;
    return projectsOpen || catalogOpen ? "pin" : null;
  };

  const startSectionResize = (event: ReactPointerEvent<HTMLDivElement>, handle: SectionHandle) => {
    if (event.button !== 0) return;
    const bounds = sectionsRef.current?.getBoundingClientRect();
    if (!bounds || bounds.height <= 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    sectionResizeRef.current = {
      pointerId: event.pointerId,
      handle,
      startY: event.clientY,
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
    // Collapse state cannot change mid-drag, so the operation the gutter
    // carried at pointerdown is the one derived here.
    const operation = gutterOperation(resize.handle);
    if (operation === "head") {
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
        projectShare: shareLocked
          ? current.projectShare
          : shareHoldingCatalog(resize.catalog, box, resize.storage),
      }));
      return;
    }
    if (operation === "share") {
      setSectionLayout((current) => ({
        ...current,
        projectShare: shareDragged(resize.projects, resize.catalog, event.clientY - resize.startY),
      }));
      return;
    }
    if (operation !== "pin") return;
    // The pin moves with the pointer relative to the grab point — measured off
    // the box bottom it would snap Storages to the pointer's absolute height,
    // which for the gutter passing the pin through a closed Catalog means a
    // jump by the closed tile's whole span the moment it is grabbed. With
    // Catalog open it absorbs the change and the share is re-solved so
    // Projects keeps the pixels it started with; with Catalog closed Projects
    // absorbs on its own and the stored share must ride through untouched.
    // The clamp holds the fixed pane's pixels and stops once the absorber is
    // down on its own floor.
    const catalogClosed = collapsedSections.has("catalog");
    const held = catalogClosed ? resize.catalog : resize.projects;
    const height = clampStorageHeight(
      resize.storage + (resize.startY - event.clientY),
      resize.height,
      held,
    );
    setSectionLayout((current) => ({
      ...current,
      projectShare: catalogClosed
        ? current.projectShare
        : shareHolding(resize.projects, resize.height, height),
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
    const operation = gutterOperation(handle);
    if (operation === "head") {
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
    if (operation === "share") {
      const share = sectionLayout.projectShare;
      const next = event.key === "ArrowUp" ? share - 0.05
        : event.key === "ArrowDown" ? share + 0.05
        : event.key === "Home" ? MIN_SECTION_SHARE
        : 1 - MIN_SECTION_SHARE;
      setSectionLayout((current) => ({ ...current, projectShare: clampSectionShare(next) }));
      return;
    }
    if (operation !== "pin") return;
    const box = sectionsRef.current?.getBoundingClientRect().height ?? 0;
    const catalogClosed = collapsedSections.has("catalog");
    const projects = paneHeight(".sidebar-project-pane");
    const held = catalogClosed ? paneHeight(".sidebar-catalog-pane") : projects;
    const [min, max] = storageHeightRange(box, held);
    const current = sectionLayout.storageHeight ?? storageContentHeight();
    const next = event.key === "ArrowUp" ? current + SECTION_KEYBOARD_STEP
      : event.key === "ArrowDown" ? current - SECTION_KEYBOARD_STEP
      : event.key === "Home" ? max
      : min;
    const height = clampStorageHeight(next, box, held);
    setSectionLayout((layout) => ({
      ...layout,
      projectShare: catalogClosed ? layout.projectShare : shareHolding(projects, box, height),
      storageHeight: height,
    }));
  };

  const resetSection = (handle: SectionHandle) => {
    const operation = gutterOperation(handle);
    if (operation === "head") {
      // Back to the height its own rows ask for.
      setSectionLayout((current) => ({ ...current, headHeight: null }));
      return;
    }
    if (operation === "share") {
      setSectionLayout((current) => ({ ...current, projectShare: DEFAULT_PROJECT_SHARE }));
      return;
    }
    if (operation !== "pin") return;
    // Back to fitting its contents exactly — the state a fixed list wants — and
    // holding Projects where it is, the same as a drag of this gutter; through
    // a closed Catalog the stored share rides untouched instead.
    const catalogClosed = collapsedSections.has("catalog");
    const box = sectionsRef.current?.getBoundingClientRect().height ?? 0;
    const projects = paneHeight(".sidebar-project-pane");
    setSectionLayout((current) => ({
      ...current,
      projectShare: catalogClosed
        ? current.projectShare
        : shareHolding(projects, box, storageContentHeight()),
      storageHeight: null,
    }));
  };

  /* The name says what the drag trades right now, so a gutter that passes the
     pin through a closed Catalog announces Projects and Storages — the panes
     it actually moves. An inert gutter keeps the geometric name of the seam
     it draws. */
  const dividerLabel = (handle: SectionHandle, operation: GutterOperation | null): string => {
    if (handle === 0) return "Resize navigation and Projects sections";
    if (operation === "pin" && collapsedSections.has("catalog")) {
      return "Resize Projects and Storages sections";
    }
    return handle === 1
      ? "Resize Projects and Catalog sections"
      : "Resize Catalog and Storages sections";
  };

  /* The separator reports the value of whatever it operates: the head its
     height, the share its percentage, the pin the Storages height. */
  const dividerValues = (operation: GutterOperation | null) => {
    if (operation === "head") {
      return {
        "aria-valuenow": sectionLayout.headHeight === null
          ? undefined
          : Math.round(sectionLayout.headHeight),
        "aria-valuetext": sectionLayout.headHeight === null
          ? "Navigation height: fits contents"
          : `Navigation height: ${Math.round(sectionLayout.headHeight)} pixels`,
      };
    }
    if (operation === "share") {
      return {
        "aria-valuemin": Math.round(MIN_SECTION_SHARE * 100),
        "aria-valuemax": Math.round((1 - MIN_SECTION_SHARE) * 100),
        "aria-valuenow": Math.round(sectionLayout.projectShare * 100),
      };
    }
    if (operation === "pin") {
      return {
        "aria-valuemin": MIN_PANE_HEIGHT,
        "aria-valuenow": sectionLayout.storageHeight === null
          ? undefined
          : Math.round(sectionLayout.storageHeight),
        "aria-valuetext": sectionLayout.storageHeight === null
          ? "Storage height: fits contents"
          : `Storage height: ${Math.round(sectionLayout.storageHeight)} pixels`,
      };
    }
    return {};
  };

  /* Every gutter wears the same handle; only what it resizes differs. A gutter
     whose operation has gone with a collapsed pane is inert: the 3px seam
     stays, but the drag affordance, the keyboard stop, and the double-click
     reset all leave with it. */
  const dividerProps = (handle: SectionHandle) => {
    const operation = gutterOperation(handle);
    const inert = operation === null;
    return {
      className: `sidebar-section-divider${inert ? " inert" : ""}`,
      role: "separator",
      "aria-label": dividerLabel(handle, operation),
      "aria-orientation": "horizontal" as const,
      "aria-disabled": inert || undefined,
      tabIndex: inert ? -1 : 0,
      ...dividerValues(operation),
      ...(inert ? {} : {
        onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => startSectionResize(event, handle),
        onPointerMove: continueSectionResize,
        onPointerUp: finishSectionResize,
        onPointerCancel: finishSectionResize,
        onKeyDown: (event: ReactKeyboardEvent<HTMLDivElement>) => resizeSectionsWithKeyboard(event, handle),
        onDoubleClick: () => resetSection(handle),
      }),
    };
  };

  /* What a closed heading must still say: the attention draft and the
     profile-less project are the two flags the Projects rows draw, and the
     storage rows draw theirs from the issue map. Catalog rows have no flagged
     state to carry. */
  const projectsFlagged = drafts.some((draft) => draft.status === "attention")
    || projects.some((item) => Object.keys(item.profile_ids ?? {}).length === 0);
  const storagesFlagged = storages.some((storage) => Boolean(storageIssues[storage.id]));

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
            className={!activeStorageId && !activeCatalogScopeId && !addingCatalogScope && !catalogIndexOpen && !storageIndexOpen && !settingsOpen ? "active" : undefined}
            onClick={onOpenProjects}
            disabled={busy}
          >
            <Icon name="folder" size={15} /> Projects
          </button>
          <button
            type="button"
            className={activeCatalogScopeId || addingCatalogScope || catalogIndexOpen ? "active" : undefined}
            onClick={onOpenCatalogIndex}
            disabled={busy}
          >
            <Icon name="catalog" size={15} /> Catalog
          </button>
          <button
            type="button"
            className={activeStorageId || storageIndexOpen ? "active" : undefined}
            onClick={onOpenStorageIndex}
            disabled={busy}
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
            id="activity-log-trigger"
            type="button"
            className={activityOpen ? "active" : undefined}
            onClick={onToggleActivity}
            aria-controls="activity-log-drawer"
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
        {...dividerProps(0)}
        className="sidebar-section-divider sidebar-head-divider"
      />

      <div
        ref={sectionsRef}
        className={`sync-sidebar-sections sidebar-settings-sections${resizingSections ? " resizing" : ""}`}
        style={{
          gridTemplateRows: sectionGridRows(sectionLayout.projectShare, sectionLayout.storageHeight, collapsedSections),
        }}
      >
        <section
          className={`sidebar-split-pane sidebar-project-pane${collapsedSections.has("projects") ? " collapsed" : ""}`}
          aria-label="Projects"
        >
          <SectionHeading
            section="projects"
            collapsed={collapsedSections.has("projects")}
            count={drafts.length + projects.length}
            flagged={projectsFlagged}
            onToggle={() => toggleSection("projects")}
          >
            <button
              type="button"
              className="sidebar-section-action sidebar-add-action"
              onClick={() => {
                revealSection("projects");
                onAddProject();
              }}
              disabled={busy}
              title="Add project"
              aria-label="Add project"
            >
              <Icon name="plus" size={16} />
            </button>
          </SectionHeading>

          <div
            id="sidebar-projects-list"
            className="sidebar-profile-list"
            hidden={collapsedSections.has("projects")}
          >
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
                  {/* An attention draft points at a missing storage or profile.
                      The triangle provides a second channel beyond tint
                      (WCAG 1.4.1), while the word keeps the row identified as a
                      draft. */}
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
                    /* The triangle is decorative, so the button's accessible
                       name must include the project issue. */
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

        <div {...dividerProps(1)} />

        <section
          className={`sidebar-split-pane sidebar-catalog-pane${collapsedSections.has("catalog") ? " collapsed" : ""}`}
          aria-label="Catalog"
        >
          <SectionHeading
            section="catalog"
            collapsed={collapsedSections.has("catalog")}
            count={catalogScopes.length}
            flagged={false}
            onToggle={() => toggleSection("catalog")}
          >
            <button
              type="button"
              className="sidebar-section-action sidebar-add-action"
              onClick={() => {
                revealSection("catalog");
                onAddCatalogScope();
              }}
              disabled={busy}
              title="Add a resource: one kind from one agent profile"
              aria-label="Add Catalog resource"
            >
              <Icon name="plus" size={16} />
            </button>
          </SectionHeading>
          <div
            id="sidebar-catalog-list"
            className="sidebar-profile-list"
            hidden={collapsedSections.has("catalog")}
          >
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

        <div {...dividerProps(2)} />

        <section
          className={`sidebar-split-pane sidebar-storage-pane${collapsedSections.has("storages") ? " collapsed" : ""}`}
          aria-label="Storages"
          /* The floor rides the pane rather than the track (see the header
             comment) — but a closed tile floored at 96px would be held open
             on empty space, so the floor leaves with the list. */
          style={{ minHeight: collapsedSections.has("storages") ? undefined : `${MIN_PANE_HEIGHT}px` }}
        >
          <SectionHeading
            section="storages"
            collapsed={collapsedSections.has("storages")}
            count={storages.length + storageDrafts.length}
            flagged={storagesFlagged}
            onToggle={() => toggleSection("storages")}
          >
            <button
              type="button"
              className="sidebar-section-action sidebar-add-action"
              onClick={() => {
                revealSection("storages");
                onAddStorage();
              }}
              disabled={busy}
              title="Add storage"
              aria-label="Add storage"
            >
              <Icon name="plus" size={16} />
            </button>
          </SectionHeading>
          <div
            id="sidebar-storages-list"
            className="sidebar-links-list"
            hidden={collapsedSections.has("storages")}
          >
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
            {/* Storage drafts share the configured-storage rail. Open resumes
                the form; trash discards only the draft. The active editor's
                storage id supplies the active marker. */}
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
