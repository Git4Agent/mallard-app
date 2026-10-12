import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import packageMetadata from "../../../package.json";
import { useTranslation } from "../../i18n";
import type {
  LocalProjectSummary,
  SetupDraftSummary,
  StorageProviderDescriptor,
  StorageProviderKey,
  StorageSetupDraft,
  StorageSummary,
} from "../../types";
import Icon, { storageIconName, type IconName } from "../Icons";
import { catalogDoors, catalogScopeIcon, catalogScopeLabel } from "../storage-catalog/model";
import type { CatalogCapabilityDescriptor, CatalogScopeView } from "../storage-catalog/types";
import { projectLabel, storageDoors } from "./model";
import { storageProviderName } from "./storageText";
import { findStorageProvider, storageConnectionIssue } from "./StorageSettings";

/* One rail, one list. The head says the five destinations once, as a strip of
   glyphs; below it the rail is whichever root the workspace is standing in, at
   whatever height is left over. Nothing in here is drag-sized or remembered
   between launches, so the sidebar has the same shape in an 800x560 window as
   in a full-screen one — the list is the only thing that grows.

   It replaces a three-pane split whose panes floored at 96px each. At the
   800x560 window floor — the tauri.conf.json minimum — the head alone asked
   for 314px of the 544 available and the three floors for 312 more, so the
   rail scrolled and Storages dropped off the bottom. The head is a fixed
   98px now (48 brand bar, 46 tab strip, 4 border), plus a 30px strip for the
   traffic lights on macOS, and the list takes the remaining 356 there. The
   brand bar is the same on every platform; the sidebar toggle rides its far
   end. The roots were also being named twice over, once
   in the nav and once as a section heading; the strip says each of them
   once. */

/** The three roots that own the rail, in the order they sit in the strip;
 *  each is named by `shell:roots.<id>`. */
export type SectionId = "projects" | "catalog" | "storages";

/** An empty rail's rows: one dashed tile per way Add can start — the glyph
 * and a plus, no words — so the count of tiles is the count of choices, and
 * a tile opens Add with that choice already made. */
function SidebarDoors({ doors, disabled }: {
  doors: Array<{ key: string; icon: IconName; label: string; onOpen: () => void }>;
  disabled: boolean;
}) {
  return (
    <div className="sidebar-doors">
      {doors.map((door) => (
        <button
          key={door.key}
          type="button"
          className="sidebar-door"
          title={door.label}
          aria-label={door.label}
          disabled={disabled}
          onClick={door.onOpen}
        >
          <Icon name={door.icon} size={16} />
          <Icon name="plus" size={12} />
        </button>
      ))}
    </div>
  );
}

/* The heading says which root the rail is showing and how many rows it holds.
   With one list there is nothing to disclose, so the chevron and the whole
   collapsed-only branch go with it: the count is a plain fact about the list
   and is always drawn, and a flagged row is left to draw its own triangle
   because no closed pane can hide it any more. */
function RailHeading({ title, count, children }: {
  title: string;
  count: number;
  children: ReactNode;
}) {
  return (
    <div className="sidebar-section-heading">
      <span className="sidebar-section-label">
        <span className="sidebar-section-title">{title}</span>
        <span className="sidebar-section-count">{count}</span>
      </span>
      <div className="sidebar-heading-actions">{children}</div>
    </div>
  );
}

interface Props {
  projects: LocalProjectSummary[];
  drafts: SetupDraftSummary[];
  activeDraftId: string | null;
  storages: StorageSummary[];
  /** Persisted New-storage forms and unsaved settings edits; rows the same
      way project drafts are. */
  storageDrafts: StorageSetupDraft[];
  /** Compiled provider descriptors: a draft row borrows its provider's icon. */
  storageProviders: StorageProviderDescriptor[];
  storageUsage: Record<string, number>;
  storageIssues?: Record<string, string>;
  activeProjectId: string | null;
  activeStorageId: string | null;
  catalogScopes: CatalogScopeView[];
  /** The registry the Add chooser offers: an empty Catalog rail draws one
      door per shape it registers. */
  catalogCapabilities?: CatalogCapabilityDescriptor[];
  activeCatalogScopeId: string | null;
  /** A root's add page is open: New project, Add Catalog resource, or the
      storage type picker. A draft's own page is not one. */
  addingProject: boolean;
  addingCatalogScope: boolean;
  addingStorage: boolean;
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
  onOpenStorage: (storageId: string) => void;
  onOpenStorageDraft: (draftId: string) => void;
  onDiscardStorageDraft: (draftId: string) => void;
  onRemoveStorage: (storageId: string) => void;
  /** Add storage, with the provider already chosen when a door names one. */
  onAddStorage: (provider?: StorageProviderKey) => void;
  onOpenCatalogIndex: () => void;
  onOpenCatalogScope: (scopeId: string) => void;
  /** Add a Catalog row, with Kind already chosen when a door names one. */
  onAddCatalogScope: (choiceKey?: string) => void;
  onRemoveCatalogScope: (scopeId: string) => void;
  onOpenStorageIndex: () => void;
  onOpenSettings: () => void;
  /** The version mark's one action, at its far end (the update pill). */
  versionAction?: ReactNode;
}

export default function ProjectSidebar({
  projects,
  drafts,
  activeDraftId,
  storages,
  storageDrafts,
  storageProviders,
  storageUsage,
  storageIssues = {},
  activeProjectId,
  activeStorageId,
  catalogScopes,
  catalogCapabilities = [],
  activeCatalogScopeId,
  addingProject,
  addingCatalogScope,
  addingStorage,
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
  versionAction,
}: Props) {
  const { t } = useTranslation(["shell", "common"]);
  /* The root the workspace is standing in. Three of the five destinations own
     a list; Settings is a leaf with none, so it reports null rather than
     falling through to Projects. Logs is a drawer over whatever is already
     open and never enters into it. */
  const openRoot: SectionId | null = activeStorageId || storageIndexOpen
    ? "storages"
    : activeCatalogScopeId || addingCatalogScope || catalogIndexOpen
      ? "catalog"
      : settingsOpen
        ? null
        : "projects";

  /* Behind the settings page the rail keeps the root it was showing. Snapping
     back to Projects would rearrange the sidebar on the way to a page that has
     nothing to do with it. */
  const [lastRoot, setLastRoot] = useState<SectionId>(openRoot ?? "projects");
  useEffect(() => {
    if (openRoot) setLastRoot(openRoot);
  }, [openRoot]);
  const railRoot = openRoot ?? lastRoot;

  const tabClass = (active: boolean) => `sidebar-root-tab${active ? " active" : ""}`;

  const railCount = railRoot === "projects"
    ? drafts.length + projects.length
    : railRoot === "catalog"
      ? catalogScopes.length
      : storages.length + storageDrafts.length;

  /* One add action, named for the root the rail is showing. While that
     root's add page is open it is held off, doors and all: pressing it again
     would only reopen the page, and drop a choice already made there. */
  const railAdd = railRoot === "projects"
    ? { onClick: onAddProject, title: t("sidebar.addProject"), label: t("sidebar.addProject"), open: addingProject, openTitle: t("sidebar.addingProject") }
    : railRoot === "catalog"
      ? {
        onClick: () => onAddCatalogScope(),
        title: t("sidebar.addCatalogTitle"),
        label: t("pages.addCatalog"),
        open: addingCatalogScope,
        openTitle: t("sidebar.addingCatalog"),
      }
      : { onClick: () => onAddStorage(), title: t("sidebar.addStorage"), label: t("sidebar.addStorage"), open: addingStorage, openTitle: t("sidebar.addingStorage") };
  const addHeld = busy || railAdd.open;

  const projectRows = () => (
    <>
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
              ? t("sidebar.draftFailedTitle", { path: draft.project_root, error: draft.last_error })
              : t("sidebar.draftTitle", { path: draft.project_root })}
            aria-label={draft.last_error
              ? t("sidebar.draftFailedLabel", { name: draft.display_name, error: draft.last_error })
              : t("sidebar.draftLabel", { name: draft.display_name })}
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
              {t("sidebar.draft")}
            </span>
          </button>
          <div className="sidebar-profile-actions">
            <button
              type="button"
              onClick={() => onSelectDraft(draft.draft_id)}
              disabled={busy}
              title={t("sidebar.continueSetup", { name: draft.display_name })}
              aria-label={t("sidebar.continueSetup", { name: draft.display_name })}
            >
              <Icon name="settings" size={13} />
            </button>
            <button
              type="button"
              className="sidebar-profile-remove"
              onClick={() => onDiscardDraft(draft.draft_id)}
              disabled={busy}
              title={t("sidebar.discardDraftTitle", { name: draft.display_name })}
              aria-label={t("sidebar.discardDraft", { name: draft.display_name })}
            >
              <Icon name="trash" size={13} />
            </button>
          </div>
        </div>
      ))}
      {loading && projects.length === 0 && drafts.length === 0 ? (
        <div className="sidebar-msg">{t("sidebar.loadingProjects")}</div>
      ) : projects.length === 0 && drafts.length === 0 ? (
        /* One door: Add project opens setup at its Folder field. */
        <SidebarDoors
          disabled={addHeld}
          doors={[{ key: "folder", icon: "folder", label: t("sidebar.addProject"), onOpen: onAddProject }]}
        />
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
              aria-label={project.is_git_repository === true
                ? t(profileRequired ? "sidebar.projectGitIssue" : "sidebar.projectGit", { name: label })
                : profileRequired ? t("sidebar.projectIssue", { name: label }) : label}
              title={[
                project.is_git_repository === true ? t("sidebar.gitRepository") : null,
                aliased ? t("sidebar.repoName", { name: project.display_name }) : null,
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
                title={t("sidebar.removeProjectTitle", { name: label })}
                aria-label={t("sidebar.removeFromMallard", { name: label })}
              >
                <Icon name="trash" size={13} />
              </button>
            </div>
          </div>
        );
      })}
    </>
  );

  const catalogRows = () => (
    catalogScopes.length === 0 ? (
      /* One door per shape the registry offers; before it answers, the
         chooser itself. */
      <SidebarDoors
        disabled={addHeld}
        doors={catalogCapabilities.length > 0
          ? catalogDoors(catalogCapabilities).map((door) => ({
            key: door.key,
            icon: door.icon,
            label: t("sidebar.addDoor", { label: door.label }),
            onOpen: () => onAddCatalogScope(door.key),
          }))
          : [{ key: "any", icon: "catalog", label: t("sidebar.addResource"), onOpen: () => onAddCatalogScope() }]}
      />
    ) : catalogScopes.map((scope) => {
      const label = catalogScopeLabel(scope);
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
            <Icon name={catalogScopeIcon(scope)} size={16} />
            <span>{label}</span>
          </button>
          <div className="sidebar-profile-actions">
            <button
              type="button"
              className="sidebar-profile-remove"
              onClick={() => onRemoveCatalogScope(scope.scope.scope_id)}
              disabled={busy}
              title={t("sidebar.removeRowTitle", { label })}
              aria-label={t("sidebar.removeRow", { label })}
            >
              <Icon name="trash" size={13} />
            </button>
          </div>
        </div>
      );
    })
  );

  const storageRows = () => (
    <>
      {storages.length === 0 && storageDrafts.length === 0 ? (
        /* One door per provider the build ships; before the descriptors
           arrive, the picker itself. */
        <SidebarDoors
          disabled={addHeld}
          doors={storageProviders.length > 0
            ? storageDoors(storageProviders).map((door) => ({
              key: door.key,
              icon: door.icon,
              label: t("sidebar.addStorageDoor", { label: door.label }),
              onOpen: () => onAddStorage(door.key),
            }))
            : [{ key: "any", icon: "storage", label: t("sidebar.addStorage"), onOpen: () => onAddStorage() }]}
        />
      ) : storages.map((storage) => {
        const usage = storageUsage[storage.storage_id] ?? 0;
        const storageName = storage.name || t("fallback.storage");
        /* A storage this build cannot open is flagged the way a broken
           one is: the row stays, the triangle says why in its name. A
           refused credential comes first and opens straight onto the
           settings, where the same namespace is kept and only the
           credential is asked for again. */
        const storageIssue = storageConnectionIssue(storage)
          ?? storageIssues[storage.storage_id]
          ?? (storage.available ? null : t("sidebar.noAdapter", { provider: storageProviderName(storage) }));
        return (
          <div
            key={storage.storage_id}
            className={`sidebar-profile-item${activeStorageId === storage.storage_id ? " active" : ""}`}
          >
            <button
              type="button"
              className="sidebar-profile-main"
              onClick={() => onOpenStorage(storage.storage_id)}
              disabled={busy}
              title={storageIssue
                ? t("sidebar.openStorageIssueTitle", { name: storageName, issue: storageIssue })
                : t("sidebar.openStorage", { name: storageName })}
              aria-label={storageIssue
                ? t("sidebar.openStorageIssue", { name: storageName, issue: storageIssue })
                : t("sidebar.openStorage", { name: storageName })}
            >
              <Icon name={storageIconName(storage.provider, storage.icon)} size={16} />
              <span>{storage.name || t("fallback.unnamed")}</span>
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
                onClick={() => onRemoveStorage(storage.storage_id)}
                disabled={busy || usage > 0}
                title={usage > 0
                  ? t("sidebar.storageLinked", { name: storageName, count: usage })
                  : t("sidebar.removeStorageTitle", { name: storageName })}
                aria-label={t("sidebar.removeFromMallard", { name: storageName })}
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
        const draftName = draft.name || t("fallback.unnamed");
        const draftIcon = findStorageProvider(storageProviders, draft.provider, draft.config_version)?.icon;
        return (
          <div
            key={draft.draft_id}
            className={`sidebar-profile-item sync-sidebar-draft${activeStorageId === draft.storage_id ? " active" : ""}`}
          >
            <button
              type="button"
              className="sidebar-profile-main"
              onClick={() => onOpenStorageDraft(draft.draft_id)}
              disabled={busy}
              title={t("sidebar.continueStorage", { name: draftName })}
              aria-label={t("sidebar.continueStorageLabel", { name: draftName })}
            >
              <Icon name={storageIconName(draft.provider, draftIcon)} size={16} />
              <span>{draftName}</span>
              <span className="sync-draft-badge">{t("sidebar.draft")}</span>
            </button>
            <div className="sidebar-profile-actions">
              <button
                type="button"
                className="sidebar-profile-remove"
                onClick={() => onDiscardStorageDraft(draft.draft_id)}
                disabled={busy}
                title={t("sidebar.discardStorageDraftTitle", { name: draftName })}
                aria-label={t("sidebar.discardStorageDraft", { name: draftName })}
              >
                <Icon name="trash" size={13} />
              </button>
            </div>
          </div>
        );
      })}
    </>
  );

  return (
    <aside id="project-sidebar" className="sync-sidebar">
      <div className="sync-sidebar-head">
        <div className="sync-sidebar-drag" data-tauri-drag-region />
        <div className="sync-sidebar-brand" data-tauri-drag-region>
          <img className="sync-brand-logo" src="/mallard-logo-duck.svg" alt="" data-tauri-drag-region />
          <strong>Mallard</strong>
        </div>

        {/* Five destinations, one glyph each. They carry no text, so the
            active one is named with aria-current rather than only drawn, and
            every tab states its destination in its own accessible name. */}
        <nav className="sidebar-root-tabs" aria-label={t("roots.navigation")}>
          <button
            type="button"
            className={tabClass(openRoot === "projects")}
            aria-current={openRoot === "projects" ? "page" : undefined}
            aria-label={t("roots.projects")}
            title={t("roots.projects")}
            onClick={onOpenProjects}
            disabled={busy}
          >
            <Icon name="folder" size={19} />
          </button>
          <button
            type="button"
            className={tabClass(openRoot === "catalog")}
            aria-current={openRoot === "catalog" ? "page" : undefined}
            aria-label={t("roots.catalog")}
            title={t("roots.catalog")}
            onClick={onOpenCatalogIndex}
            disabled={busy}
          >
            <Icon name="catalog" size={19} />
          </button>
          <button
            type="button"
            className={tabClass(openRoot === "storages")}
            aria-current={openRoot === "storages" ? "page" : undefined}
            aria-label={t("roots.storages")}
            title={t("roots.storages")}
            onClick={onOpenStorageIndex}
            disabled={busy}
          >
            <Icon name="storage" size={19} />
          </button>
          <button
            type="button"
            className={tabClass(settingsOpen)}
            aria-current={settingsOpen ? "page" : undefined}
            aria-label={t("common:settings")}
            title={t("common:settings")}
            onClick={onOpenSettings}
            disabled={busy}
          >
            <Icon name="settings" size={19} />
          </button>
          <button
            id="activity-log-trigger"
            type="button"
            className={tabClass(activityOpen)}
            /* The unread count is drawn as a badge with no text beside it to
               be read with, so it belongs in the button's own name. */
            aria-label={unreadLogs > 0 && !activityOpen ? t("roots.logsUnread", { unread: unreadLogs }) : t("roots.logs")}
            title={t("roots.logs")}
            onClick={onToggleActivity}
            aria-controls="activity-log-drawer"
            aria-expanded={activityOpen}
          >
            <Icon name="activity" size={19} />
            {unreadLogs > 0 && !activityOpen && (
              <span className="sync-activity-badge">{unreadLogs}</span>
            )}
          </button>
        </nav>
      </div>

      <section className="sync-sidebar-rail" aria-label={t(`roots.${railRoot}`)}>
        <RailHeading title={t(`roots.${railRoot}`)} count={railCount}>
          <button
            type="button"
            className="sidebar-section-action sidebar-add-action"
            onClick={railAdd.onClick}
            disabled={addHeld}
            title={railAdd.open ? railAdd.openTitle : railAdd.title}
            aria-label={railAdd.label}
          >
            <Icon name="plus" size={16} />
          </button>
        </RailHeading>
        <div className="sidebar-rail-list">
          {railRoot === "projects" && projectRows()}
          {railRoot === "catalog" && catalogRows()}
          {railRoot === "storages" && storageRows()}
        </div>
      </section>

      <footer className="sync-sidebar-product-mark" aria-label={t("sidebar.version", { version: packageMetadata.version })}>
        <img src="/mallard-logo-duck.svg" alt="" />
        <span>v{packageMetadata.version}</span>
        {versionAction}
      </footer>
    </aside>
  );
}
