use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::{
    list_remote_bundle_snapshots_with_repository, storage_provider_descriptors,
    test_storage_with_repository,
};
use mallard_application::storage_setup::{list_storages, StorageSummary};
use mallard_infrastructure::storage_providers::StorageProviderRegistry;

use crate::args::{StorageCommand, StorageRef, StorageRemoveArgs, StorageRenameArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::{human_time, CommandOutput};

pub(crate) fn run(
    context: &CliContext,
    command: StorageCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        StorageCommand::List => list(context),
        StorageCommand::Add(args) => super::storage_setup::add(context, args.command),
        StorageCommand::Reconnect(storage) => super::storage_setup::reconnect(context, storage),
        StorageCommand::Draft(args) => super::storage_setup::draft(context, args.command),
        StorageCommand::Show(storage) => show(context, storage),
        StorageCommand::Verify(storage) => verify(context, storage),
        StorageCommand::Rename(args) => rename(context, args),
        StorageCommand::Remove(args) => remove(context, args),
        StorageCommand::Providers => providers(context),
        StorageCommand::Snapshots(storage) => snapshots(context, storage),
        StorageCommand::ClearData(args) => clear_data(context, args),
        StorageCommand::R2(_) => unreachable!("R2 verbs run through the setup backend"),
    }
}

pub(crate) fn summaries(context: &CliContext) -> Result<Vec<StorageSummary>, CliError> {
    context.blocking(|repository, _| {
        list_storages(repository, StorageProviderRegistry::production())
            .map_err(CliError::from_setup)
    })
}

pub(crate) fn summary_line(storage: &StorageSummary) -> String {
    format!(
        "{} [{}] — {}{}",
        storage.name,
        storage.storage_id,
        storage.provider_label,
        if storage.available {
            ""
        } else {
            " (unavailable in this build)"
        }
    )
}

fn list(context: &CliContext) -> Result<CommandOutput, CliError> {
    let storages = summaries(context)?;
    let mut text = String::new();
    if storages.is_empty() {
        text.push_str("No Mallard storages configured.\n");
    }
    for storage in &storages {
        text.push_str(&summary_line(storage));
        text.push('\n');
    }
    CommandOutput::new("ok", "storages", &storages, text)
}

fn show(context: &CliContext, storage: StorageRef) -> Result<CommandOutput, CliError> {
    let id = context.resolve_storage(None, Some(&storage.storage))?;
    let summary = summaries(context)?
        .into_iter()
        .find(|storage| storage.storage_id == id)
        .ok_or_else(|| {
            CliError::invalid_with("storage_not_found", "No storage has that id or name.")
        })?;
    let mut text = summary_line(&summary);
    text.push('\n');
    for field in &summary.summary {
        text.push_str(&format!("  {}: {}\n", field.label, field.value));
    }
    for secret in &summary.secrets {
        text.push_str(&format!(
            "  secret {}: {}\n",
            secret.secret_id,
            if secret.present { "saved" } else { "missing" }
        ));
    }
    CommandOutput::new("ok", "storage", &summary, text)
}

fn verify(context: &CliContext, storage: StorageRef) -> Result<CommandOutput, CliError> {
    let id = context.resolve_storage(None, Some(&storage.storage))?;
    let result = context.service(move |repository, _| {
        test_storage_with_repository(repository, tokio::runtime::Handle::current(), &id)
    })?;
    let text = format!(
        "Verified storage {}.\nMallard config version: {}\nRemote capability probe: {}",
        result.storage_id,
        result.config_revision,
        if result.verified { "passed" } else { "failed" }
    );
    CommandOutput::new("verified", "verification", &result, text)
}

fn rename(context: &CliContext, args: StorageRenameArgs) -> Result<CommandOutput, CliError> {
    let id = context.resolve_storage(None, Some(&args.storage.storage))?;
    let name = args.name;
    let expected_revision = args.expected_revision;
    let config = context.service(move |repository, sink| {
        operations::rename_storage(repository, sink, &id, &name, expected_revision)
    })?;
    CommandOutput::new(
        "renamed",
        "config",
        &config,
        format!(
            "Storage renamed (configuration revision {}).",
            config.revision
        ),
    )
}

fn remove(context: &CliContext, args: StorageRemoveArgs) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Removing a storage forgets it and its links on this computer. Pass --confirm.",
        ));
    }
    let id = context.resolve_storage(None, Some(&args.storage.storage))?;
    let expected_revision = args.expected_revision;
    let config = context.service(move |repository, sink| {
        operations::remove_storage(repository, sink, &id, expected_revision)
    })?;
    CommandOutput::new(
        "removed",
        "config",
        &config,
        format!(
            "Storage removed (configuration revision {}).",
            config.revision
        ),
    )
}

fn providers(_context: &CliContext) -> Result<CommandOutput, CliError> {
    let providers = storage_provider_descriptors();
    let mut text = String::new();
    for provider in &providers {
        text.push_str(&format!(
            "{} ({}) — {}\n",
            provider.display_name, provider.key, provider.description
        ));
        for field in &provider.fields {
            text.push_str(&format!(
                "  field {} — {}{}\n",
                field.field_id,
                field.label,
                if field.required { " (required)" } else { "" }
            ));
        }
    }
    CommandOutput::new("ok", "providers", &providers, text)
}

fn snapshots(context: &CliContext, storage: StorageRef) -> Result<CommandOutput, CliError> {
    let id = context.resolve_storage(None, Some(&storage.storage))?;
    let snapshots = context.service(move |repository, _| {
        list_remote_bundle_snapshots_with_repository(repository, &id)
    })?;
    let mut text = String::new();
    if snapshots.is_empty() {
        text.push_str("The storage holds no bundles.\n");
    }
    for snapshot in &snapshots {
        text.push_str(&format!(
            "{} [{}] — version {}, {} resources, updated {}{}\n",
            snapshot.display_name,
            snapshot.bundle_id,
            snapshot.revision,
            snapshot.resource_count,
            human_time(snapshot.updated_at),
            if snapshot.registration_only {
                " (registered, never pushed)"
            } else {
                ""
            }
        ));
    }
    CommandOutput::new("ok", "snapshots", &snapshots, text)
}

fn clear_data(context: &CliContext, args: StorageRemoveArgs) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Clearing a storage deletes every Mallard-managed object in it. Pass --confirm.",
        ));
    }
    let id = context.resolve_storage(None, Some(&args.storage.storage))?;
    let expected_revision = args.expected_revision;
    let result = context.service(move |repository, sink| {
        operations::clear_storage_data(repository, sink, &id, expected_revision)
    })?;
    CommandOutput::new(
        "cleared",
        "result",
        &result,
        format!(
            "Cleared {} repository objects and {} Library objects; {} project links and {} Library links reset.",
            result.repository_objects_deleted,
            result.library_objects_deleted,
            result.project_links_reset,
            result.catalog_links_reset
        ),
    )
}
