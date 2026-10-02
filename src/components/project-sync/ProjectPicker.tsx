import type { LocalProjectSummary, ProviderProfileSummary, StorageSummary } from "../../types";
import Icon from "../Icons";
import ProviderMark from "../sync-review/ProviderMark";
import LandingPicker from "./LandingPicker";
import { compactProjectPath, projectLabel, providerLabel } from "./model";
import ProjectsWelcome from "./ProjectsWelcome";

/**
 * The Projects landing: one door per registered project, built from the
 * registry alone — the label, the bound location, the bound providers (from
 * the binding's profile ids, never from a resource scan: `providers` on the
 * summary is only filled for the active project), and the linked storage's
 * name.
 *
 * Empty, it is the welcome instead: a mallard ride and the two ways Add
 * starts, from a folder or from Mallard Cloud.
 */
export default function ProjectPicker({ projects, profiles, storages, onAddProject, onAddCloudProject, onSelectProject }: {
  projects: LocalProjectSummary[];
  profiles: ProviderProfileSummary[];
  storages: StorageSummary[];
  onAddProject: () => void;
  onAddCloudProject: () => void;
  onSelectProject: (projectId: string) => void;
}) {
  if (projects.length === 0) {
    return <ProjectsWelcome onAddProject={onAddProject} onAddCloudProject={onAddCloudProject} />;
  }
  const storageNames = new Map(storages.map((storage) => [storage.storage_id, storage.name]));
  const profileNames = new Map(profiles.map((profile) => [profile.profile_id, profile.display_name]));
  return (
    <LandingPicker
      title="Projects"
      count={projects.length === 1 ? "1 on this machine" : `${projects.length} on this machine`}
      actionLabel="Add project…"
      onAction={onAddProject}
      cards={projects.map((project) => {
        const label = projectLabel(project);
        const boundProviders = Object.entries(project.profile_ids ?? {});
        const linkedNames = (project.linked_storage_ids ?? [])
          .map((storageId) => storageNames.get(storageId))
          .filter((name): name is string => Boolean(name));
        return {
          key: project.local_project_id,
          icon: project.is_git_repository === true ? "git-folder" : "folder",
          title: label,
          ariaLabel: project.is_git_repository === true ? `${label}, Git repository` : label,
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
          detail: (
            <>
              <Icon name="storage" size={13} />
              {linkedNames.length === 0 ? "No storage linked" : linkedNames.join(" · ")}
            </>
          ),
          detailMuted: linkedNames.length === 0,
          onOpen: () => onSelectProject(project.local_project_id),
        };
      })}
    />
  );
}
