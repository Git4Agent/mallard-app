//! Mallard Cloud from the terminal: the account's repositories, a website
//! address resolved, versions, and a project's folder got from a repository.
//! Every read goes to the storage's own service with the storage's token;
//! nothing here writes Mallard state except the clone's folder on disk.
use std::sync::Arc;

use mallard_application::events::SyncEventSink;
use mallard_application::project_sync::app_link::{self, AppLinkArrival};
use mallard_application::project_sync::cloud_project::{
    clone_tools, get_folder, github_source, CloudProjectFolderResult,
};
use mallard_application::project_sync::managed_cloud_link::{
    accept_repository_invitation, discard_draft, list_public_projects, list_repositories,
    list_starred_projects, publish_version, resolve_repository, storage_client,
    ManagedCloudRepository, ManagedCloudRepositoryResolution,
};
use mallard_domain::StorageId;
use mallard_infrastructure::managed_cloud::client::PublicSort;
use mallard_infrastructure::storage_providers::managed_cloud::provider_key;
use tokio::runtime::Handle;

use crate::args::{
    CloudAddressArgs, CloudCloneArgs, CloudCodeArgs, CloudCommand, CloudCursorArgs,
    CloudDiscardArgs, CloudLinkArgs, CloudListArgs, CloudPublicArgs, CloudPublishArgs,
    CloudStorageArgs,
};
use crate::commands::project::detail;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::CommandOutput;

pub(crate) fn run(context: &CliContext, command: CloudCommand) -> Result<CommandOutput, CliError> {
    match command {
        CloudCommand::Repositories(args) => repositories(context, args),
        CloudCommand::Starred(args) => starred(context, args),
        CloudCommand::Public(args) => public(context, args),
        CloudCommand::Resolve(args) => resolve(context, args),
        CloudCommand::AcceptInvite(args) => accept_invite(context, args),
        CloudCommand::PublishVersion(args) => publish(context, args),
        CloudCommand::DiscardDraft(args) => discard(context, args),
        CloudCommand::Clone(args) => clone(context, args),
        CloudCommand::Link(args) => link(context, args),
    }
}

/// A Mallard Cloud storage by id or name; any other provider is refused.
fn cloud_storage(context: &CliContext, selector: &str) -> Result<StorageId, CliError> {
    let id = context.resolve_storage(None, Some(selector))?;
    let is_cloud = context
        .load_config()?
        .storages
        .iter()
        .any(|storage| storage.id == id && storage.provider == provider_key());
    if !is_cloud {
        return Err(CliError::invalid_with(
            "storage_not_cloud",
            "That storage is not Mallard Cloud.",
        ));
    }
    Ok(id)
}

/// The Git fingerprint the listings compare remotes with: the named
/// project's, else the current folder's project's, else none.
fn local_fingerprint(
    context: &CliContext,
    project: Option<&str>,
) -> Result<Option<String>, CliError> {
    match context.resolve_project(project) {
        Ok(id) => Ok(detail(context, &id)?.project.repository_fingerprint),
        Err(error) if project.is_none() && error.code() == "project_not_found" => Ok(None),
        Err(error) => Err(error),
    }
}

fn block_on<T>(future: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
    Handle::current().block_on(future)
}

fn repository_line(repository: &ManagedCloudRepository) -> String {
    format!(
        "{} [{}] — {} · {} · version {}{}{}\n",
        repository.address,
        repository.repository_id,
        repository.repository_class,
        repository.visibility,
        repository.revision,
        if repository.can_push {
            " · can push"
        } else {
            ""
        },
        repository
            .bundle_id
            .as_deref()
            .map(|bundle| format!(" · bundle {bundle}"))
            .unwrap_or_default()
    )
}

fn repositories(context: &CliContext, args: CloudListArgs) -> Result<CommandOutput, CliError> {
    let storage = cloud_storage(context, &args.target.storage)?;
    let fingerprint = local_fingerprint(context, args.target.project.as_deref())?;
    let repositories = context.service(move |_, _| {
        let (client, origin) = storage_client(&storage)?;
        block_on(list_repositories(&client, &origin, fingerprint.as_deref()))
    })?;
    let mut text = String::new();
    if repositories.is_empty() {
        text.push_str("The account belongs to no project repository.\n");
    }
    for repository in &repositories {
        text.push_str(&repository_line(repository));
    }
    CommandOutput::new("ok", "repositories", &repositories, text)
}

fn starred(context: &CliContext, args: CloudCursorArgs) -> Result<CommandOutput, CliError> {
    let storage = cloud_storage(context, &args.storage)?;
    let cursor = args.cursor;
    let listing = context.service(move |_, _| {
        let (client, _) = storage_client(&storage)?;
        block_on(list_starred_projects(&client, cursor.as_deref()))
    })?;
    let text = serde_json::to_string_pretty(&listing).unwrap_or_default();
    CommandOutput::new("ok", "listing", &listing, text)
}

fn public(context: &CliContext, args: CloudPublicArgs) -> Result<CommandOutput, CliError> {
    let storage = cloud_storage(context, &args.listing.storage)?;
    let sort = PublicSort::parse(&args.sort).ok_or_else(|| {
        CliError::invalid_code("cloud_sort_invalid", "--sort must be newest or stars.")
    })?;
    let cursor = args.listing.cursor;
    let listing = context.service(move |_, _| {
        let (client, _) = storage_client(&storage)?;
        block_on(list_public_projects(&client, sort, cursor.as_deref()))
    })?;
    let text = serde_json::to_string_pretty(&listing).unwrap_or_default();
    CommandOutput::new("ok", "listing", &listing, text)
}

fn resolution_text(resolution: &ManagedCloudRepositoryResolution) -> String {
    match resolution {
        ManagedCloudRepositoryResolution::Collaborator { repository } => {
            format!("Collaborator: {}", repository_line(repository))
        }
        ManagedCloudRepositoryResolution::Public { repository } => {
            format!("Public: {}", repository_line(repository))
        }
        ManagedCloudRepositoryResolution::NotFound { address } => {
            format!("Nothing at {address} this account may see.\n")
        }
        ManagedCloudRepositoryResolution::NotAProject { repository } => format!(
            "{} is a Library item, not a project; see `mallard-cli catalog cloud resolve`.\n",
            repository.address
        ),
        ManagedCloudRepositoryResolution::WrongService { host, expected } => {
            format!("The address names {host}; this storage signs in to {expected}.\n")
        }
        ManagedCloudRepositoryResolution::GitHub => {
            "That is a GitHub address, not a Mallard Cloud one.\n".to_string()
        }
        ManagedCloudRepositoryResolution::Invalid => "That is not an address.\n".to_string(),
    }
}

fn resolve_address(
    context: &CliContext,
    target: &CloudStorageArgs,
    address: String,
) -> Result<ManagedCloudRepositoryResolution, CliError> {
    let storage = cloud_storage(context, &target.storage)?;
    let fingerprint = local_fingerprint(context, target.project.as_deref())?;
    context.service(move |_, _| {
        let (client, origin) = storage_client(&storage)?;
        block_on(resolve_repository(
            &client,
            &origin,
            &address,
            fingerprint.as_deref(),
        ))
    })
}

fn resolve(context: &CliContext, args: CloudAddressArgs) -> Result<CommandOutput, CliError> {
    let resolution = resolve_address(context, &args.target, args.address)?;
    let text = resolution_text(&resolution);
    CommandOutput::new("ok", "resolution", &resolution, text)
}

fn accept_invite(context: &CliContext, args: CloudCodeArgs) -> Result<CommandOutput, CliError> {
    let storage = cloud_storage(context, &args.target.storage)?;
    let fingerprint = local_fingerprint(context, args.target.project.as_deref())?;
    let code = args.code;
    let resolution = context.service(move |_, _| {
        let (client, origin) = storage_client(&storage)?;
        block_on(accept_repository_invitation(
            &client,
            &origin,
            &code,
            fingerprint.as_deref(),
        ))
    })?;
    let text = resolution_text(&resolution);
    CommandOutput::new("joined", "resolution", &resolution, text)
}

fn publish(context: &CliContext, args: CloudPublishArgs) -> Result<CommandOutput, CliError> {
    let project = context.resolve_project(args.target.project.project.as_deref())?;
    let storage = context.resolve_storage(Some(&project), args.target.storage.as_deref())?;
    let name = args.name;
    let published = context.service(move |repository, _| {
        block_on(publish_version(
            repository,
            &project,
            &storage,
            name.as_deref(),
        ))
    })?;
    CommandOutput::new(
        "published",
        "version",
        &published,
        format!(
            "Published version {} of repository {}.",
            published.version, published.repository_id
        ),
    )
}

fn discard(context: &CliContext, args: CloudDiscardArgs) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Discarding the draft removes every push after the latest version from Mallard Cloud. Pass --confirm.",
        ));
    }
    let project = context.resolve_project(args.target.project.project.as_deref())?;
    let storage = context.resolve_storage(Some(&project), args.target.storage.as_deref())?;
    let revision = args.expected_repository_revision;
    let discarded = context.service(move |repository, _| {
        block_on(discard_draft(repository, &project, &storage, revision))
    })?;
    let text = format!(
        "Draft discarded ({}); the repository is at version {}.",
        discarded.state,
        discarded
            .kept_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    CommandOutput::new("discarded", "discarded", &discarded, text)
}

fn clone(context: &CliContext, args: CloudCloneArgs) -> Result<CommandOutput, CliError> {
    let storage = cloud_storage(context, &args.storage)?;
    let destination = std::path::PathBuf::from(&args.into);
    if !destination.is_absolute() {
        return Err(CliError::invalid_code(
            "clone_destination_invalid",
            "--into must be an absolute path.",
        ));
    }
    let target = CloudStorageArgs {
        storage: args.storage.clone(),
        project: None,
    };
    let repository = match resolve_address(context, &target, args.address)? {
        ManagedCloudRepositoryResolution::Collaborator { repository }
        | ManagedCloudRepositoryResolution::Public { repository } => repository,
        other => {
            return Err(CliError::invalid_with(
                "cloud_repository_unavailable",
                resolution_text(&other).trim().to_string(),
            ))
        }
    };
    let is_git_project = repository.repository_class == "git_project";
    let source = match (&repository.github_url, is_git_project) {
        (Some(url), true) => Some(
            github_source(url, repository.repository_fingerprint.as_deref())
                .map_err(|message| CliError::invalid_with("cloud_source_invalid", message))?,
        ),
        _ => None,
    };
    let sink: Arc<dyn SyncEventSink> = context.sink();
    let fingerprint = repository.repository_fingerprint.clone();
    let destination_text = destination.to_string_lossy().into_owned();
    let result = context.service(move |state, _| {
        let tools = clone_tools();
        let (_, cancellation) = tokio::sync::watch::channel(false);
        block_on(get_folder(
            state.clone(),
            tools,
            destination_text,
            fingerprint,
            is_git_project,
            source,
            cancellation,
            &move |progress| sink.clone_progress(&progress),
        ))
    })?;
    let (canonical_path, what) = match &result {
        CloudProjectFolderResult::Cloned { canonical_path } => (canonical_path, "Cloned into"),
        CloudProjectFolderResult::Created { canonical_path } => (canonical_path, "Created"),
        CloudProjectFolderResult::Initialized { canonical_path, .. } => {
            (canonical_path, "Initialized an empty Git repository at")
        }
        CloudProjectFolderResult::Existing { canonical_path } => {
            (canonical_path, "Already a checkout at")
        }
    };
    let mut text = format!("{what} {canonical_path}.\n");
    if let CloudProjectFolderResult::Initialized { failure, .. } = &result {
        text.push_str(&format!(
            "The clone did not complete ({:?}){}\n",
            failure.reason,
            failure
                .command
                .as_deref()
                .map(|command| format!("; fetch later with: {command}"))
                .unwrap_or_default()
        ));
    }
    if let Some(bundle) = &repository.bundle_id {
        text.push_str(&format!(
            "Next: mallard-cli project add {canonical_path} --storage {storage} --connect {bundle}\n"
        ));
    }
    CommandOutput::merged(
        "ok",
        &serde_json::json!({ "folder": result, "repository": repository }),
        text,
    )
}

fn link(context: &CliContext, args: CloudLinkArgs) -> Result<CommandOutput, CliError> {
    let raw = if args.link.starts_with("mallard://") {
        args.link.clone()
    } else {
        format!(
            "mallard://open?address={}",
            url::form_urlencoded::byte_serialize(args.link.as_bytes()).collect::<String>()
        )
    };
    let arrival = app_link::read_link(&raw);
    let (host, address) = match &arrival {
        AppLinkArrival::Repository { host, address }
        | AppLinkArrival::Session { host, address, .. } => (host.clone(), address.clone()),
        AppLinkArrival::Listing { host, .. } => {
            return CommandOutput::merged(
                "ok",
                &serde_json::json!({ "arrival": arrival, "answer": serde_json::Value::Null }),
                format!("The link names the listing on {host}; nothing is asked of the service."),
            )
        }
        AppLinkArrival::Invalid => {
            return Err(CliError::invalid_code(
                "app_link_invalid",
                "Mallard can't open that link.",
            ))
        }
    };
    let answer = context
        .service(move |repository, _| block_on(app_link::resolve(repository, &host, &address)))?;
    let mut text = format!("{} on {}\n", answer.address, answer.host);
    if answer.accounts.is_empty() {
        text.push_str(if answer.can_set_up {
            "No storage signs in to this host yet: run `mallard-cli storage add cloud`.\n"
        } else {
            "No storage signs in to this host.\n"
        });
    }
    for account in &answer.accounts {
        text.push_str(&format!(
            "  {} ({}): {}\n",
            account.storage_name,
            account.account_login,
            serde_json::to_string(&account.sight).unwrap_or_default()
        ));
    }
    CommandOutput::merged(
        "ok",
        &serde_json::json!({ "arrival": arrival, "answer": answer }),
        text,
    )
}
