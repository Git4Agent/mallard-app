use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::{
    list_remote_bundle_snapshots_with_repository, remove_project_link_with_repository,
    save_project_link_with_repository, ConnectProjectBundleRequest, SaveProjectLinkRequest,
};
use mallard_domain::BundleId;

use crate::args::{LinkAddArgs, LinkArgs, LinkCommand, LinkConnectArgs, LinkRemoveArgs};
use crate::commands::project::detail;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::CommandOutput;

pub(crate) fn run(context: &CliContext, args: LinkArgs) -> Result<CommandOutput, CliError> {
    let project = context.resolve_project(args.project.project.as_deref())?;
    match args.command {
        LinkCommand::List => {
            let detail = detail(context, &project)?;
            let config = context.load_config()?;
            let mut text = String::new();
            if detail.links.is_empty() {
                text.push_str(
                    "No storage links. Run `mallard-cli link add --storage <id|name>`.\n",
                );
            }
            for link in &detail.links {
                let name = config
                    .storages
                    .iter()
                    .find(|storage| storage.id == link.storage_id)
                    .map(|storage| storage.name.as_str())
                    .unwrap_or("?");
                text.push_str(&format!(
                    "{} [{}] — bundle {}{}\n",
                    name,
                    link.storage_id,
                    link.bundle_id,
                    if link.pinned { " (pinned)" } else { "" }
                ));
            }
            CommandOutput::new("ok", "links", &detail.links, text)
        }
        LinkCommand::Add(args) => add(context, &project, args),
        LinkCommand::Connect(args) => connect(context, &project, args),
        LinkCommand::Remove(args) => remove(context, &project, args),
    }
}

fn add(
    context: &CliContext,
    project: &mallard_domain::LocalProjectId,
    args: LinkAddArgs,
) -> Result<CommandOutput, CliError> {
    let storage = context.resolve_storage(None, Some(&args.storage))?;
    let request = SaveProjectLinkRequest {
        local_project_id: project.clone(),
        storage_id: storage,
        pinned: args.pinned,
    };
    let link = context
        .service(move |repository, _| save_project_link_with_repository(repository, request))?;
    CommandOutput::new(
        "linked",
        "link",
        &link,
        format!(
            "Linked to storage {}; the first Push publishes bundle {}.",
            link.storage_id, link.bundle_id
        ),
    )
}

fn connect(
    context: &CliContext,
    project: &mallard_domain::LocalProjectId,
    args: LinkConnectArgs,
) -> Result<CommandOutput, CliError> {
    let storage = context.resolve_storage(None, Some(&args.storage))?;
    let bundle_id = BundleId::parse(args.bundle_id.as_str()).map_err(|_| {
        CliError::invalid_code(
            "bundle_id_invalid",
            "That is not a bundle id. Run `mallard-cli storage snapshots <storage>`.",
        )
    })?;
    let current = detail(context, project)?;
    let snapshot_storage = storage.clone();
    let snapshots = context.service(move |repository, _| {
        list_remote_bundle_snapshots_with_repository(repository, &snapshot_storage)
    })?;
    if !snapshots
        .iter()
        .any(|snapshot| snapshot.bundle_id == bundle_id)
    {
        return Err(CliError::invalid_with(
            "bundle_not_found",
            "The storage holds no bundle with that id. Run `mallard-cli storage snapshots <storage>`.",
        ));
    }
    let request = ConnectProjectBundleRequest {
        local_project_id: project.clone(),
        storage_id: storage,
        bundle_id,
        expected_bundle_id: current.project.bundle_id.clone(),
        pinned: args.pinned,
        allow_repository_mismatch: args.allow_repository_mismatch,
    };
    let detail = context.service(move |repository, sink| {
        operations::connect_project_to_remote_bundle(repository, sink, request)
    })?;
    CommandOutput::new(
        "connected",
        "project",
        &detail,
        format!(
            "Connected {} to bundle {}. Run `mallard-cli pull review` next.",
            detail.project.display_name, detail.project.bundle_id
        ),
    )
}

fn remove(
    context: &CliContext,
    project: &mallard_domain::LocalProjectId,
    args: LinkRemoveArgs,
) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Removing a link forgets this computer's reviewed state for that storage. Pass --confirm.",
        ));
    }
    let storage = context.resolve_storage(None, Some(&args.storage))?;
    let project = project.clone();
    let removed = context.service(move |repository, _| {
        remove_project_link_with_repository(repository, &project, &storage)
    })?;
    CommandOutput::new(
        if removed { "removed" } else { "unchanged" },
        "removed",
        &removed,
        if removed {
            "Link removed. The bundle stays in the storage.".to_string()
        } else {
            "No such link.".to_string()
        },
    )
}
