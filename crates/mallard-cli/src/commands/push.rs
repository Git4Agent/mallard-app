use std::collections::BTreeSet;

use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::{
    get_push_destination_status_with_repository, inspect_project_files_with_repository,
    sibling_bundle_status_with_repository, ProjectContentEntryType, ProjectContentInventory,
    PushApproval, PushDestinationStatus, PushReviewContext, PushScope,
};
use mallard_domain::{
    LocalProjectId, ProjectFileSyncEligibilityState, RecipeEntry, ResourceId, StorageId,
};

use crate::args::{PushApplyArgs, PushArgs, PushCommand, PushReviewArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::{human_bytes, CommandOutput};

pub(crate) fn run(context: &CliContext, args: PushArgs) -> Result<CommandOutput, CliError> {
    let project = context.resolve_project(args.target.project.project.as_deref())?;
    let storage = context.resolve_storage(Some(&project), args.target.storage.as_deref())?;
    match args.command {
        PushCommand::Review(args) => review(context, project, storage, args),
        PushCommand::Apply(args) => apply(context, project, storage, args),
        PushCommand::Finish => finish(context, project, storage),
        PushCommand::Status => status(context, project, storage),
    }
}

pub(crate) fn resource_id(value: &str) -> Result<ResourceId, CliError> {
    ResourceId::parse(value).map_err(|_| {
        CliError::invalid_with(
            "resource_id_invalid",
            "That is not a resource id. Resource ids appear in `session list` and `project inventory`.",
        )
    })
}

fn scope(session: Option<&str>) -> Result<PushScope, CliError> {
    Ok(match session {
        Some(session) => PushScope::Session {
            resource_id: resource_id(session)?,
        },
        None => PushScope::Project,
    })
}

fn prepare(
    context: &CliContext,
    project: &LocalProjectId,
    storage: &StorageId,
    scope: PushScope,
) -> Result<PushReviewContext, CliError> {
    let (project, storage) = (project.clone(), storage.clone());
    context.service(move |repository, sink| {
        operations::prepare_push_review(repository, sink, &project, &storage, scope)
    })
}

fn destination_line(status: &PushDestinationStatus) -> String {
    match status {
        PushDestinationStatus::Ready { revision } => match revision {
            Some(revision) => format!("Destination ready at version {revision}."),
            None => "Destination ready; nothing pushed yet.".to_string(),
        },
        PushDestinationStatus::RequiresPull {
            remote_revision,
            blocking_scopes,
            ..
        } => format!(
            "Pull first: the storage is at version {remote_revision} and this computer has not reviewed it (scopes: {}).",
            blocking_scopes.join(", ")
        ),
        PushDestinationStatus::Unavailable { message } => {
            format!("Destination unavailable: {message}")
        }
    }
}

fn review_text(review: &PushReviewContext, storage: &str) -> String {
    let mut text = format!(
        "Push review — {} → {}\n{}\n",
        review.project.project.display_name,
        storage,
        destination_line(&review.destination_status)
    );
    match &review.thread_comparison {
        Some(comparison) => {
            text.push_str(&format!(
                "Sessions: {} synced, {} local only, {} storage only, {} diverged, {} unavailable\n",
                comparison.counts.synced,
                comparison.counts.local,
                comparison.counts.storage,
                comparison.counts.diverged,
                comparison.counts.unavailable
            ));
            for entry in &comparison.entries {
                text.push_str(&format!(
                    "  {} {} — {} [{}]{}\n",
                    if entry.push_decision.selectable {
                        "[x]"
                    } else {
                        "[ ]"
                    },
                    entry.display_name,
                    serde_json::to_value(entry.state)
                        .ok()
                        .and_then(|value| value.as_str().map(ToOwned::to_owned))
                        .unwrap_or_default(),
                    entry.resource_id,
                    entry
                        .push_decision
                        .blocked_reason
                        .as_deref()
                        .map(|reason| format!(" · {reason}"))
                        .unwrap_or_default()
                ));
            }
            for (provider, error) in &comparison.provider_errors {
                text.push_str(&format!("warning: {provider}: {error}\n"));
            }
        }
        None => text.push_str("Sessions: not compared\n"),
    }
    if let Some(error) = &review.thread_comparison_error {
        text.push_str(&format!("warning: comparison failed: {error}\n"));
    }
    let mut files_to_push = false;
    if let Some(files) = &review.project_files {
        if matches!(
            files.eligibility.state,
            ProjectFileSyncEligibilityState::Eligible
        ) {
            files_to_push = !files.entries.is_empty();
            text.push_str(&format!(
                "Project files: {} entries ({} ignored, {} blocked); `push apply --files` includes them\n",
                files.entries.len(),
                files.ignored_count,
                files.blocked_count
            ));
            text.push_str(&super::project::files_entry_lines(files));
        } else {
            text.push_str(&format!("Project files: {}\n", files.eligibility.reason));
        }
    }
    if let Some(error) = &review.project_files_error {
        text.push_str(&format!("warning: project files: {error}\n"));
    }
    text.push_str(if files_to_push {
        "Nothing was pushed. Run `mallard-cli push apply --files` to push the recommended sessions with the project files, or `mallard-cli push apply` for the sessions alone.\n"
    } else {
        "Nothing was pushed. Run `mallard-cli push apply` to push the recommended selection.\n"
    });
    text
}

fn review(
    context: &CliContext,
    project: LocalProjectId,
    storage: StorageId,
    args: PushReviewArgs,
) -> Result<CommandOutput, CliError> {
    let mut review = prepare(context, &project, &storage, scope(args.session.as_deref())?)?;
    if review.project_files.is_none() {
        // The desktop scans a project's files in its own Files step; the
        // review here carries that scan, so an agent sees what --files pushes.
        let (files_project, files_storage) = (project.clone(), storage.clone());
        match context.service(move |repository, _| {
            Ok(inspect_project_files_with_repository(
                repository,
                &files_project,
                &files_storage,
            ))
        })? {
            Ok(inventory) => review.project_files = Some(inventory),
            Err(error) => review.project_files_error = Some(error),
        }
    }
    let storage_name = context
        .load_config()?
        .storages
        .into_iter()
        .find(|candidate| candidate.id == storage)
        .map(|candidate| candidate.name)
        .unwrap_or_else(|| storage.to_string());
    let text = review_text(&review, &storage_name);
    CommandOutput::new("reviewed", "review", &review, text)
}

/// The project files a review ticks by itself: the files already in the
/// recipe, and every new file the scan found nothing against. A new folder
/// comes along only when it is empty; one that holds entries follows the
/// files ticked inside it. The desktop's Files step applies the same rule.
fn offered_files(inventory: &ProjectContentInventory) -> BTreeSet<ResourceId> {
    let mut holds_entries = BTreeSet::new();
    for entry in &inventory.entries {
        let parts: Vec<&str> = entry.relative_path.split('/').collect();
        for end in 1..parts.len() {
            holds_entries.insert(parts[..end].join("/"));
        }
    }
    inventory
        .entries
        .iter()
        .filter(|entry| {
            entry.selected_in_recipe
                || (entry.undecided
                    && entry.warning_code.is_none()
                    && (entry.entry_type != ProjectContentEntryType::Directory
                        || !holds_entries.contains(&entry.relative_path)))
        })
        .map(|entry| entry.descriptor.resource_id.clone())
        .collect()
}

/// Every folder above a ticked entry, so a file never travels without its
/// folders.
fn required_directories(
    inventory: &ProjectContentInventory,
    selected: &BTreeSet<ResourceId>,
) -> BTreeSet<ResourceId> {
    let directories: std::collections::BTreeMap<&str, &ResourceId> = inventory
        .entries
        .iter()
        .filter(|entry| entry.entry_type == ProjectContentEntryType::Directory)
        .map(|entry| (entry.relative_path.as_str(), &entry.descriptor.resource_id))
        .collect();
    let mut required = BTreeSet::new();
    for entry in &inventory.entries {
        if !selected.contains(&entry.descriptor.resource_id) {
            continue;
        }
        let parts: Vec<&str> = entry.relative_path.split('/').collect();
        for end in 1..parts.len() {
            if let Some(directory) = directories.get(parts[..end].join("/").as_str()) {
                required.insert((*directory).clone());
            }
        }
    }
    required
}

fn apply(
    context: &CliContext,
    project: LocalProjectId,
    storage: StorageId,
    args: PushApplyArgs,
) -> Result<CommandOutput, CliError> {
    let review = prepare(context, &project, &storage, scope(args.session.as_deref())?)?;
    let mut approval = PushApproval::recommended(&review, &storage);

    for value in &args.select {
        let id = resource_id(value)?;
        let resource = review
            .inventory
            .resources
            .iter()
            .find(|resource| resource.descriptor.resource_id == id)
            .ok_or_else(|| {
                CliError::invalid_with(
                    "resource_not_in_review",
                    format!("'{value}' is not in the review's inventory."),
                )
            })?;
        if !resource.selectable || resource.blocked_reason.is_some() {
            return Err(CliError::invalid_with(
                "resource_not_selectable",
                format!(
                    "'{value}' cannot be pushed{}.",
                    resource
                        .blocked_reason
                        .as_deref()
                        .map(|reason| format!(": {reason}"))
                        .unwrap_or_default()
                ),
            ));
        }
        approval.recipe.entries.insert(
            id.clone(),
            RecipeEntry {
                resource_id: id,
                apply_policy: resource.descriptor.apply_policy.clone(),
                required: false,
            },
        );
    }
    for value in &args.deselect {
        let id = resource_id(value)?;
        if approval
            .recipe
            .entries
            .get(&id)
            .is_some_and(|entry| entry.required)
        {
            return Err(CliError::invalid_with(
                "resource_required",
                format!("'{value}' is required by the push and cannot be left out."),
            ));
        }
        approval.recipe.entries.remove(&id);
    }

    let mut acknowledge_all = false;
    let mut acknowledged = BTreeSet::new();
    for value in &args.acknowledge_warnings {
        if value == "all" {
            acknowledge_all = true;
        } else {
            acknowledged.insert(value.clone());
        }
    }

    if args.files || !args.remove_file.is_empty() {
        let (files_project, files_storage) = (project.clone(), storage.clone());
        let inventory = context.service(move |repository, _| {
            inspect_project_files_with_repository(repository, &files_project, &files_storage)
        })?;
        if inventory.eligibility.state != ProjectFileSyncEligibilityState::Eligible {
            return Err(CliError::invalid_with(
                "project_files_not_eligible",
                format!(
                    "This project's files do not sync through Mallard: {}",
                    inventory.eligibility.reason
                ),
            ));
        }
        let mut selected = if args.files {
            offered_files(&inventory)
        } else {
            BTreeSet::new()
        };
        for value in &args.select {
            let id = resource_id(value)?;
            if inventory
                .entries
                .iter()
                .any(|entry| entry.descriptor.resource_id == id)
            {
                selected.insert(id);
            }
        }
        let removals: BTreeSet<ResourceId> = args
            .remove_file
            .iter()
            .map(|value| resource_id(value))
            .collect::<Result<_, _>>()?;
        for removal in &removals {
            if !inventory
                .entries
                .iter()
                .any(|entry| &entry.descriptor.resource_id == removal)
            {
                return Err(CliError::invalid_with(
                    "project_file_not_found",
                    format!("'{removal}' is not a project file of this project."),
                ));
            }
            selected.remove(removal);
            approval.recipe.entries.remove(removal);
        }
        selected.extend(required_directories(&inventory, &selected));
        for entry in &inventory.entries {
            let id = &entry.descriptor.resource_id;
            if !selected.contains(id) || entry.entry_type == ProjectContentEntryType::Blocked {
                continue;
            }
            if let Some(digest) = &entry.warning_digest {
                if acknowledge_all {
                    acknowledged.insert(digest.clone());
                }
            }
            approval.recipe.entries.insert(
                id.clone(),
                RecipeEntry {
                    resource_id: id.clone(),
                    apply_policy: entry.descriptor.apply_policy.clone(),
                    required: false,
                },
            );
        }
        approval.project_content_review_token = inventory.review_token.clone();
        approval.project_files_shown = args.files;
        approval.project_content_removal_ids = removals.into_iter().collect();
    }
    approval.acknowledged_warning_digests = acknowledged.into_iter().collect();

    let selected_count = approval.recipe.entries.len();
    let result = context.service(move |repository, sink| {
        operations::push_bundle(repository, sink, &project, &storage, approval)
    })?;
    let mut text = format!(
        "{} {} ({} resources selected)\n",
        if result.success {
            "Pushed."
        } else {
            "Push did not complete."
        },
        result.message,
        selected_count
    );
    for resource in &result.results {
        text.push_str(&format!(
            "  {} {}{}\n",
            resource.state,
            resource.resource_id,
            resource
                .message
                .as_deref()
                .map(|message| format!(" — {message}"))
                .unwrap_or_default()
        ));
    }
    CommandOutput::new(
        if result.success {
            "pushed"
        } else {
            "incomplete"
        },
        "result",
        &result,
        text,
    )
}

fn finish(
    context: &CliContext,
    project: LocalProjectId,
    storage: StorageId,
) -> Result<CommandOutput, CliError> {
    context.service(move |repository, sink| {
        operations::finish_unfinished_push(repository, sink, &project, &storage)
    })?;
    CommandOutput::new(
        "finished",
        "finished",
        &true,
        "The unfinished push's record was written.".to_string(),
    )
}

fn status(
    context: &CliContext,
    project: LocalProjectId,
    storage: StorageId,
) -> Result<CommandOutput, CliError> {
    let (destination, sibling) = context.service(move |repository, _| {
        Ok((
            get_push_destination_status_with_repository(repository, &project, &storage)?,
            sibling_bundle_status_with_repository(repository, &project, &storage)?,
        ))
    })?;
    let text = format!(
        "{}\nSiblings: {}",
        destination_line(&destination),
        serde_json::to_string(&sibling).unwrap_or_default()
    );
    CommandOutput::merged(
        "ok",
        &serde_json::json!({ "destination": destination, "sibling": sibling }),
        text,
    )
}

#[allow(dead_code)]
fn bytes(value: u64) -> String {
    human_bytes(value)
}
