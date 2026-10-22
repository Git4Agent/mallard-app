use std::collections::BTreeMap;
use std::path::Path;

use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::{
    agent_provider_descriptors, create_setup_draft_with_repository,
    get_bundle_inventory_with_repository, get_project_with_repository,
    inspect_project_files_with_repository, inspect_setup_draft_with_repository,
    list_remote_bundle_snapshots_with_repository, load_project_sync_config_with_repository,
    probe_project_folder_with_repository, remove_local_project_with_repository,
    rename_local_project_with_repository, update_setup_draft_with_repository, ProjectDetail,
    ProjectFolderProbe,
};
use mallard_domain::{
    BundleId, DraftProfileSelection, DraftRepositoryChoice, DraftStorageSelection, LocalProjectId,
    LocalProviderProfileId, ProviderKey,
};
use mallard_infrastructure::persistence::user_home_dir;

use crate::args::{
    ProjectAddArgs, ProjectCommand, ProjectRemoveArgs, ProjectRenameArgs, ProjectSelector,
    ProjectStorageSelector,
};
use crate::commands::provider::provider_key;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::{human_time, CommandOutput};

pub(crate) fn run(
    context: &CliContext,
    command: ProjectCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        ProjectCommand::List => list(context),
        ProjectCommand::Add(args) => add(context, args),
        ProjectCommand::Show(selector) => show(context, selector),
        ProjectCommand::Files(selector) => files(context, selector),
        ProjectCommand::Rename(args) => rename(context, args),
        ProjectCommand::Remove(args) => remove(context, args),
        ProjectCommand::Inventory(selector) => inventory(context, selector),
    }
}

fn list(context: &CliContext) -> Result<CommandOutput, CliError> {
    let config =
        context.service(|repository, _| load_project_sync_config_with_repository(repository))?;
    let mut text = String::new();
    if config.projects.is_empty() {
        text.push_str("No projects registered. Run `mallard-cli project add <path>`.\n");
    }
    for project in &config.projects {
        let storages: Vec<String> = config
            .links
            .iter()
            .filter(|link| link.local_project_id == project.local_project_id)
            .map(|link| {
                config
                    .storages
                    .iter()
                    .find(|storage| storage.storage_id == link.storage_id)
                    .map(|storage| storage.name.clone())
                    .unwrap_or_else(|| link.storage_id.to_string())
            })
            .collect();
        text.push_str(&format!(
            "{} [{}] — revision {}; storages: {}\n",
            project
                .local_alias
                .as_deref()
                .unwrap_or(&project.display_name),
            project.local_project_id,
            project.revision,
            if storages.is_empty() {
                "none".to_string()
            } else {
                storages.join(", ")
            }
        ));
    }
    CommandOutput::new("ok", "projects", &config.projects, text)
}

pub(crate) fn detail(
    context: &CliContext,
    local_project_id: &LocalProjectId,
) -> Result<ProjectDetail, CliError> {
    let id = local_project_id.clone();
    context
        .service(move |repository, _| get_project_with_repository(repository, &id))?
        .ok_or_else(|| {
            CliError::invalid_with(
                "project_not_found",
                "No project has that id. Run `mallard-cli project list`.",
            )
        })
}

fn show(context: &CliContext, selector: ProjectSelector) -> Result<CommandOutput, CliError> {
    let id = context.resolve_project(selector.project.as_deref())?;
    let detail = detail(context, &id)?;
    let config = context.load_config()?;
    let mut text = format!(
        "{} [{}]\n  bundle {}\n  revision {} · updated {}\n",
        detail.project.display_name,
        detail.project.local_project_id,
        detail.project.bundle_id,
        detail.project.revision,
        human_time(detail.project.updated_at)
    );
    if let Some(alias) = &detail.project.local_alias {
        text.push_str(&format!("  nickname {alias}\n"));
    }
    if let Some(fingerprint) = &detail.project.repository_fingerprint {
        text.push_str(&format!("  git repository {fingerprint}\n"));
    }
    match &detail.binding {
        Some(binding) => {
            text.push_str(&format!("  folder {}\n", binding.project_root));
            for (provider, profile) in &binding.profile_ids {
                text.push_str(&format!("  provider {provider} → profile {profile}\n"));
            }
        }
        None => text.push_str("  folder: not bound on this computer\n"),
    }
    if detail.links.is_empty() {
        text.push_str("  storages: none\n");
    }
    for link in &detail.links {
        let name = config
            .storages
            .iter()
            .find(|storage| storage.id == link.storage_id)
            .map(|storage| storage.name.as_str())
            .unwrap_or("?");
        text.push_str(&format!(
            "  storage {} [{}]{}\n",
            name,
            link.storage_id,
            if link.pinned { " (pinned)" } else { "" }
        ));
    }
    CommandOutput::new("ok", "project", &detail, text)
}

fn rename(context: &CliContext, args: ProjectRenameArgs) -> Result<CommandOutput, CliError> {
    let id = context.resolve_project(args.selector.project.as_deref())?;
    if args.name.is_none() && !args.clear {
        return Err(CliError::invalid_code(
            "rename_requires_name",
            "Pass --name <nickname> or --clear.",
        ));
    }
    let alias = args.name;
    let expected_revision = args.expected_revision;
    let project = context.service(move |repository, _| {
        rename_local_project_with_repository(repository, &id, alias, expected_revision)
    })?;
    let text = match &project.local_alias {
        Some(alias) => format!(
            "{} is now shown as {} (revision {}).",
            project.display_name, alias, project.revision
        ),
        None => format!(
            "{} is shown by its own name again (revision {}).",
            project.display_name, project.revision
        ),
    };
    CommandOutput::new("renamed", "project", &project, text)
}

fn remove(context: &CliContext, args: ProjectRemoveArgs) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Removing a project forgets it on this computer. Pass --confirm.",
        ));
    }
    let id = context.resolve_project(args.selector.project.as_deref())?;
    let detail = detail(context, &id)?;
    if detail.project.revision != args.expected_revision {
        return Err(CliError::conflict(
            "revision_conflict",
            format!(
                "The project is at revision {}, not {}. Read it again before removing it.",
                detail.project.revision, args.expected_revision
            ),
        ));
    }
    let removed = context
        .service(move |repository, _| remove_local_project_with_repository(repository, &id))?;
    CommandOutput::new(
        if removed { "removed" } else { "unchanged" },
        "removed",
        &removed,
        if removed {
            format!("Forgot {} on this computer.", detail.project.display_name)
        } else {
            "Nothing was removed.".to_string()
        },
    )
}

fn inventory(context: &CliContext, selector: ProjectSelector) -> Result<CommandOutput, CliError> {
    let id = context.resolve_project(selector.project.as_deref())?;
    let inventory = context
        .service(move |repository, _| get_bundle_inventory_with_repository(repository, &id))?;
    let mut text = format!(
        "{} — {} resources, recipe revision {}\n",
        inventory.project,
        inventory.resources.len(),
        inventory.recipe.revision
    );
    for resource in &inventory.resources {
        text.push_str(&format!(
            "  {} {} — {} · {}{}{}\n",
            if resource.default_selected {
                "[x]"
            } else {
                "[ ]"
            },
            resource.descriptor.resource_id,
            resource.kind_label,
            resource.apply_policy_label,
            if resource.selectable {
                ""
            } else {
                " · not selectable"
            },
            resource
                .blocked_reason
                .as_deref()
                .map(|reason| format!(" · {reason}"))
                .unwrap_or_default()
        ));
    }
    for warning in &inventory.warnings {
        text.push_str(&format!("warning: {warning}\n"));
    }
    CommandOutput::new("ok", "inventory", &inventory, text)
}

/// One `--provider` value: `codex`, `codex=<profile-id>`, or `codex=<path>`.
fn profile_selection(
    context: &CliContext,
    spec: &str,
) -> Result<(ProviderKey, DraftProfileSelection), CliError> {
    let (key, value) = match spec.split_once('=') {
        Some((key, value)) => (key, Some(value)),
        None => (spec, None),
    };
    let provider = provider_key(key)?;
    let machine = context.load_bindings()?;
    let profiles: Vec<_> = machine
        .profiles
        .iter()
        .filter(|profile| profile.provider == provider)
        .collect();
    let selection = match value {
        Some(value) => {
            if let Ok(profile_id) = LocalProviderProfileId::parse(value) {
                if profiles.iter().any(|profile| profile.profile_id == profile_id) {
                    return Ok((provider, DraftProfileSelection::Existing { profile_id }));
                }
            }
            if !Path::new(value).is_absolute() {
                return Err(CliError::invalid_with(
                    "provider_profile_invalid",
                    format!(
                        "'{value}' is neither a saved {provider} profile id nor an absolute path to a {provider} home."
                    ),
                ));
            }
            DraftProfileSelection::Pending {
                path: value.to_string(),
                display_name: Path::new(value)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            }
        }
        None => match profiles.as_slice() {
            [profile] => DraftProfileSelection::Existing {
                profile_id: profile.profile_id.clone(),
            },
            [] => default_home_selection(&provider).ok_or_else(|| {
                CliError::invalid_with(
                    "provider_profile_required",
                    format!(
                        "No saved {provider} profile and no default home on this computer. Pass --provider {provider}=<path to its home>."
                    ),
                )
            })?,
            many => {
                return Err(CliError::invalid_with(
                    "provider_profile_ambiguous",
                    format!(
                        "Several {provider} profiles are saved; pass --provider {provider}=<profile-id>: {}",
                        many.iter()
                            .map(|profile| format!("{} ({})", profile.profile_id, profile.display_name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ))
            }
        },
    };
    Ok((provider, selection))
}

/// The provider's default home under this user's home directory, when it
/// exists: what the desktop's New project preselects for a first project.
fn default_home_selection(provider: &ProviderKey) -> Option<DraftProfileSelection> {
    let descriptor = agent_provider_descriptors()
        .into_iter()
        .find(|descriptor| &descriptor.key == provider)?;
    let directory = descriptor.default_profile_directory?;
    let path = user_home_dir()?.join(&directory);
    path.is_dir().then(|| DraftProfileSelection::Pending {
        path: path.to_string_lossy().into_owned(),
        display_name: directory.trim_start_matches('.').to_string(),
    })
}

fn add(context: &CliContext, args: ProjectAddArgs) -> Result<CommandOutput, CliError> {
    let path = args.path.clone();
    let probe = context
        .service(move |repository, _| probe_project_folder_with_repository(repository, &path))?;
    let canonical = match probe {
        ProjectFolderProbe::Folder { canonical_path, .. } => canonical_path,
        ProjectFolderProbe::Invalid { message } => {
            return Err(CliError::invalid_with("project_folder_invalid", message))
        }
        ProjectFolderProbe::Missing => {
            return Err(CliError::invalid_code(
                "project_folder_missing",
                "Nothing exists at that path.",
            ))
        }
        ProjectFolderProbe::Unreadable { message } => {
            return Err(CliError::invalid_with("project_folder_unreadable", message))
        }
        ProjectFolderProbe::NotADirectory { .. } => {
            return Err(CliError::invalid_code(
                "project_folder_not_a_directory",
                "That path is a file; pass its folder.",
            ))
        }
        ProjectFolderProbe::MallardData { .. } => {
            return Err(CliError::invalid_code(
                "project_folder_mallard_data",
                "That folder is Mallard's own data folder, or holds it.",
            ))
        }
    };

    // Providers: the ones named, or every default provider the computer can
    // bind without asking.
    let mut profiles = BTreeMap::new();
    if args.provider.is_empty() {
        for descriptor in agent_provider_descriptors() {
            if !descriptor.setup_default {
                continue;
            }
            if let Ok((provider, selection)) = profile_selection(context, descriptor.key.as_str()) {
                profiles.insert(provider, selection);
            }
        }
        if profiles.is_empty() {
            return Err(CliError::invalid_code(
                "provider_profile_required",
                "No agent provider home was found on this computer. Pass --provider <key>=<path to its home>, for example --provider codex=$HOME/.codex.",
            ));
        }
    } else {
        for spec in &args.provider {
            let (provider, selection) = profile_selection(context, spec)?;
            profiles.insert(provider, selection);
        }
    }

    let storage = match args.storage.as_deref() {
        Some(selector) => Some(context.resolve_storage(None, Some(selector))?),
        None => None,
    };
    let repository_choice = match (&args.connect, &storage) {
        (Some(bundle), Some(storage_id)) => {
            let bundle_id = BundleId::parse(bundle.as_str()).map_err(|_| {
                CliError::invalid_code(
                    "bundle_id_invalid",
                    "That is not a bundle id. Run `mallard-cli storage snapshots <storage>`.",
                )
            })?;
            let snapshot_storage = storage_id.clone();
            let snapshot = context
                .service(move |repository, _| {
                    list_remote_bundle_snapshots_with_repository(repository, &snapshot_storage)
                })?
                .into_iter()
                .find(|snapshot| snapshot.bundle_id == bundle_id)
                .ok_or_else(|| {
                    CliError::invalid_with(
                        "bundle_not_found",
                        "The storage holds no bundle with that id. Run `mallard-cli storage snapshots <storage>`.",
                    )
                })?;
            DraftRepositoryChoice::Existing {
                storage_id: storage_id.clone(),
                bundle_id,
                display_name: snapshot.display_name,
                repository_fingerprint: snapshot.repository_fingerprint,
                mismatch_acknowledged: args.allow_repository_mismatch,
            }
        }
        _ => DraftRepositoryChoice::New,
    };

    let name = args.name.clone();
    let (draft_id, revision, inspection) = context.service(move |repository, _| {
        let created = create_setup_draft_with_repository(repository, &canonical)?;
        let mut draft = created.draft;
        draft.profiles = profiles;
        draft.storage = storage.map(|storage_id| DraftStorageSelection { storage_id });
        draft.repository = repository_choice;
        if let Some(name) = name {
            draft.display_name = name;
        }
        let draft = update_setup_draft_with_repository(repository, draft)?;
        let inspection = inspect_setup_draft_with_repository(repository, &draft.draft_id)?;
        Ok((draft.draft_id, draft.revision, inspection))
    })?;
    if !inspection.can_finalize {
        let blocked: Vec<String> = inspection
            .sections
            .iter()
            .filter(|section| section.state == "blocked")
            .map(|section| {
                format!(
                    "{}: {}",
                    section.section,
                    section.message.as_deref().unwrap_or("blocked")
                )
            })
            .collect();
        return Err(CliError::invalid_with(
            "project_setup_blocked",
            format!(
                "Setup cannot finish: {}. The draft {} is kept; fix the folder or providers and run `project add` again.",
                blocked.join("; "),
                draft_id
            ),
        ));
    }
    let mut detail = context.service(move |repository, sink| {
        operations::finalize_project_setup(repository, sink, &draft_id, revision)
    })?;
    // A stored bundle keeps the name it was published with; a name given
    // when connecting becomes the nickname this computer shows, as a
    // rename would make it.
    if let Some(name) = args
        .name
        .filter(|name| *name != detail.project.display_name)
    {
        let id = detail.project.local_project_id.clone();
        let expected_revision = detail.project.revision;
        detail = context.service(move |repository, _| {
            rename_local_project_with_repository(repository, &id, Some(name), expected_revision)?;
            get_project_with_repository(repository, &id)?
                .ok_or_else(|| "The project vanished while it was being named.".to_string())
        })?;
    }
    let mut text = format!(
        "Added {} [{}] — bundle {}{}\n",
        detail
            .project
            .local_alias
            .as_deref()
            .unwrap_or(&detail.project.display_name),
        detail.project.local_project_id,
        detail.project.bundle_id,
        if detail.project.local_alias.is_some() {
            format!(" (stored as {})", detail.project.display_name)
        } else {
            String::new()
        }
    );
    if let Some(binding) = &detail.binding {
        for (provider, profile) in &binding.profile_ids {
            text.push_str(&format!(
                "  provider {provider} → profile {profile}
"
            ));
        }
    }
    for link in &detail.links {
        text.push_str(&format!(
            "  storage {}
",
            link.storage_id
        ));
    }
    for section in &inspection.sections {
        if section.state == "attention" {
            if let Some(message) = &section.message {
                text.push_str(&format!(
                    "note: {message}
"
                ));
            }
        }
    }
    CommandOutput::new("added", "project", &detail, text)
}

fn files(
    context: &CliContext,
    selector: ProjectStorageSelector,
) -> Result<CommandOutput, CliError> {
    let id = context.resolve_project(selector.project.project.as_deref())?;
    let storage = context.resolve_storage(Some(&id), selector.storage.as_deref())?;
    let inventory = context.service(move |repository, _| {
        inspect_project_files_with_repository(repository, &id, &storage)
    })?;
    let mut text = format!(
        "{} — {} ({} entries, {} ignored, {} blocked)
",
        inventory.project_root,
        inventory.eligibility.reason,
        inventory.entries.len(),
        inventory.ignored_count,
        inventory.blocked_count
    );
    text.push_str(&files_entry_lines(&inventory));
    CommandOutput::new("ok", "files", &inventory, text)
}

/// One line per scanned entry, then the scan's warnings: what `project
/// files` lists and what the Push review repeats for a non-Git project.
pub(super) fn files_entry_lines(
    inventory: &mallard_application::project_sync::service::ProjectContentInventory,
) -> String {
    let mut text = String::new();
    for entry in &inventory.entries {
        text.push_str(&format!(
            "  {} {} — {}{} [{}]{}\n",
            if entry.selected_in_recipe {
                "[x]"
            } else {
                "[ ]"
            },
            entry.relative_path,
            entry.state,
            if entry.undecided { " · new" } else { "" },
            entry.descriptor.resource_id,
            entry
                .warning_code
                .as_deref()
                .map(|code| format!(" · warning {code}"))
                .unwrap_or_default()
        ));
    }
    for warning in &inventory.warnings {
        text.push_str(&format!("warning: {warning}\n"));
    }
    text
}
