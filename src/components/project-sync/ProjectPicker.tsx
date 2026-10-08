import { t, useTranslation } from "../../i18n";
import type { LocalProjectSummary, ProviderProfileSummary, StorageSummary } from "../../types";
import Icon, { storageIconName } from "../Icons";
import { NoticeChip } from "../notice/Notice";
import ProviderMark from "../sync-review/ProviderMark";
import LandingPicker from "./LandingPicker";
import { compactProjectPath, projectLabel, providerLabel } from "./model";
import ProjectsWelcome from "./ProjectsWelcome";
import { storageConnectionIssue } from "./StorageSettings";

/** Tiles a card stacks before the count alone carries the rest. */
const STACKED_TILES = 4;

/**
 * The Projects landing: one door per registered project, built from the
 * registry alone — the label, the bound location, the bound providers (from
 * the binding's profile ids, never from a resource scan: `providers` on the
 * summary is only filled for the active project), and the linked storages.
 *
 * Empty, it is the welcome instead: a mallard ride and the two ways Add
 * starts, from a folder or from Mallard Cloud.
 */
export default function ProjectPicker({
  projects,
  profiles,
  storages,
  rememberedStorageByProject,
  onAddProject,
  onAddCloudProject,
  onSelectProject,
}: {
  projects: LocalProjectSummary[];
  profiles: ProviderProfileSummary[];
  storages: StorageSummary[];
  /** The storage each project last opened on. A project with none, or whose
      remembered storage is no longer linked, opens on its first link. */
  rememberedStorageByProject: Record<string, string>;
  onAddProject: () => void;
  onAddCloudProject: () => void;
  onSelectProject: (projectId: string) => void;
}) {
  const { t } = useTranslation("shell");
  if (projects.length === 0) {
    return <ProjectsWelcome onAddProject={onAddProject} onAddCloudProject={onAddCloudProject} />;
  }
  const storageById = new Map(storages.map((storage) => [storage.storage_id, storage]));
  const profileNames = new Map(profiles.map((profile) => [profile.profile_id, profile.display_name]));
  return (
    <LandingPicker
      title={t("roots.projects")}
      count={t("landing.count", { count: projects.length })}
      actionLabel={t("landing.addProject")}
      onAction={onAddProject}
      cards={projects.map((project) => {
        const label = projectLabel(project);
        const boundProviders = Object.entries(project.profile_ids ?? {});
        const linked = (project.linked_storage_ids ?? [])
          .map((storageId) => storageById.get(storageId))
          .filter((storage): storage is StorageSummary => Boolean(storage));
        // The storage the project opens on leads; the rest keep link order.
        const opensOn = linked.findIndex((storage) => (
          storage.storage_id === rememberedStorageByProject[project.local_project_id]
        ));
        if (opensOn > 0) linked.unshift(...linked.splice(opensOn, 1));
        return {
          key: project.local_project_id,
          icon: project.is_git_repository === true ? "git-folder" : "folder",
          title: label,
          // The stack shows one name; the door's name carries every one.
          ariaLabel: t(project.is_git_repository === true ? "landing.cardGit" : "landing.card", {
            name: label,
            storages: spokenStorages(linked),
          }),
          marks: boundProviders.length > 0
            ? boundProviders.map(([provider, profileId]) => (
              <ProviderMark
                key={provider}
                provider={provider}
                displayName={profileNames.get(profileId) ?? providerLabel(provider)}
              />
            ))
            : undefined,
          location: compactProjectPath(project.project_root),
          detail: <StorageStack linked={linked} />,
          detailMuted: linked.length === 0,
          onOpen: () => onSelectProject(project.local_project_id),
        };
      })}
    />
  );
}

/** The one word a storage needing attention carries: its tile's colour
 *  names the transport, never its state. */
function attentionWord(storage: StorageSummary): string | null {
  if (!storage.available) return t("shell:landing.unavailable");
  return storageConnectionIssue(storage) ? t("shell:landing.reconnect") : null;
}

function storageName(storage: StorageSummary): string {
  return storage.name || t("shell:fallback.unnamed");
}

function spokenStorages(linked: StorageSummary[]): string {
  if (linked.length === 0) return t("shell:landing.noStorage");
  const spoken = linked.map((storage, index) => [
    storageName(storage),
    index === 0 && linked.length > 1 ? t("shell:landing.opensHere") : null,
    attentionWord(storage),
  ].filter(Boolean).join(t("shell:landing.partSeparator")));
  return t("shell:landing.storages", { count: linked.length, list: spoken.join(t("shell:landing.listSeparator")) });
}

/**
 * One line whatever the count, so every card keeps one height: the linked
 * storages' marks overlapping on the rail's tile, the storage the project
 * opens on by name, and a count for the rest. A hidden storage's trouble is
 * counted, never pinned to the name shown.
 */
function StorageStack({ linked }: { linked: StorageSummary[] }) {
  const { t } = useTranslation("shell");
  if (linked.length === 0) {
    return (
      <span className="landing-picker-storages none">
        <Icon name="link-off" size={12} />
        <span>{t("landing.noStorage")}</span>
      </span>
    );
  }
  const [shown, ...others] = linked;
  const shownWord = attentionWord(shown);
  const unavailable = others.filter((storage) => !storage.available).length;
  const reconnect = others.filter((storage) => storage.available && storageConnectionIssue(storage)).length;
  const othersWord = unavailable > 0
    ? t("landing.othersUnavailable", { number: unavailable })
    : reconnect > 0 ? t("landing.othersReconnect", { number: reconnect }) : null;
  return (
    <span className="landing-picker-storages" title={linked.map(storageName).join("\n")}>
      <span className="landing-picker-storages-tiles">
        {linked.slice(0, STACKED_TILES).map((storage) => (
          <Icon key={storage.storage_id} name={storageIconName(storage.provider, storage.icon)} size={14} />
        ))}
      </span>
      <span className="landing-picker-storages-name">{storageName(shown)}</span>
      {shownWord && <NoticeChip severity="warning">{shownWord}</NoticeChip>}
      {others.length > 0 && <span className="landing-picker-storages-more">+{others.length}</span>}
      {othersWord && <NoticeChip severity="warning">{othersWord}</NoticeChip>}
    </span>
  );
}
