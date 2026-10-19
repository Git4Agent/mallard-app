//! Setting up a storage from the terminal: the draft the desktop's setup page
//! would hold, filled by a folder or by a browser sign-in, then probed and
//! saved in the same process. Credentials go straight from the provider into
//! Mallard's local state; none is printed.
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use mallard_application::events::SyncEventSink;
use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::{
    create_storage_edit_draft_with_repository, create_storage_setup_draft_with_repository,
    discard_storage_setup_draft_with_repository, get_storage_setup_draft_with_repository,
    list_storage_setup_drafts_with_repository, test_storage_setup_draft_with_repository,
    update_storage_setup_draft_with_repository, StorageSetupDraftSummary,
    UpdateStorageSetupDraftRequest,
};
use mallard_application::storage_setup::{
    begin_storage_draft_operation, CommitStorageDraftResult, StorageSetupError,
};
use mallard_domain::{BoundedConfigObject, StorageSetupDraftId};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::{self, StorageProviderRegistry};
use mallard_infrastructure::{
    dropbox_auth, dropbox_setup, feishu_auth, feishu_connections, loopback_browser, managed_cloud,
};
use tokio::runtime::Handle;

use crate::args::{
    StorageAddCloudArgs, StorageAddCommand, StorageAddDropboxArgs, StorageAddFeishuArgs,
    StorageAddLocalArgs, StorageAddR2Args, StorageDraftCommand, StorageRef,
};
use crate::backend::{ExistingBucket, ProductionBackend, R2Target, SetupInput, StorageBackend};
use crate::commands::storage::{summaries, summary_line};
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::CommandOutput;

const LINK_EXPIRES_IN: u32 = 180;

pub(crate) fn add(
    context: &CliContext,
    command: StorageAddCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        StorageAddCommand::Local(args) => add_local(context, args),
        StorageAddCommand::R2(args) => add_r2(context, args),
        StorageAddCommand::Dropbox(args) => add_dropbox(context, args),
        StorageAddCommand::Feishu(args) => add_feishu(context, args),
        StorageAddCommand::Cloud(args) => add_cloud(context, args),
    }
}

fn saved(
    context: &CliContext,
    committed: &CommitStorageDraftResult,
    status: &'static str,
) -> Result<CommandOutput, CliError> {
    let summary = summaries(context)?
        .into_iter()
        .find(|storage| storage.storage_id == committed.storage_id)
        .ok_or_else(|| {
            CliError::persistence("The storage was saved but could not be read back.")
        })?;
    let text = format!(
        "{}\n{} (configuration revision {}).",
        summary_line(&summary),
        if committed.replaced {
            "Storage settings saved"
        } else {
            "Storage added"
        },
        committed.config_revision
    );
    CommandOutput::new(status, "storage", &summary, text)
}

fn setup_error(error: StorageSetupError) -> CliError {
    CliError::from_setup(error)
}

fn draft_id(value: &str) -> Result<StorageSetupDraftId, CliError> {
    StorageSetupDraftId::parse(value).map_err(|_| {
        CliError::invalid_code(
            "storage_setup_draft_id_invalid",
            "That is not a storage setup draft id. Run `mallard-cli storage draft list`.",
        )
    })
}

/// Save a prepared draft: the capability probe and the atomic write the
/// desktop's Finish setup performs.
fn commit(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    draft_id: &StorageSetupDraftId,
    revision: u64,
) -> Result<CommitStorageDraftResult, CliError> {
    operations::commit_storage_setup_draft(repository, sink, Handle::current(), draft_id, revision)
        .map_err(CliError::from_service)
}

/// Run a sign-in sequence on a blocking thread: the loopback receiver blocks
/// the thread, the exchanges run on the runtime.
fn block_on<T>(
    future: impl std::future::Future<Output = Result<T, CliError>>,
) -> Result<T, CliError> {
    Handle::current().block_on(future)
}

fn new_draft(
    repository: &LocalStateRepository,
    provider: &mallard_domain::StorageProviderKey,
    name: Option<&str>,
) -> Result<StorageSetupDraftSummary, CliError> {
    let draft = create_storage_setup_draft_with_repository(repository, provider)
        .map_err(CliError::from_service)?;
    match name {
        Some(name) if name != draft.name => update_storage_setup_draft_with_repository(
            repository,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: draft.revision,
                name: name.to_string(),
                settings: draft.settings.clone(),
                secrets: Default::default(),
            },
        )
        .map_err(CliError::from_service),
        _ => Ok(draft),
    }
}

fn add_local(context: &CliContext, args: StorageAddLocalArgs) -> Result<CommandOutput, CliError> {
    let path = std::path::PathBuf::from(&args.path);
    if !path.is_absolute() {
        return Err(CliError::invalid_code(
            "storage_directory_invalid",
            "--path must be an absolute path.",
        ));
    }
    std::fs::create_dir_all(&path).map_err(|_| {
        CliError::invalid_code(
            "storage_directory_invalid",
            "The folder does not exist and could not be created.",
        )
    })?;
    let name = args.name.clone().or_else(|| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });
    let committed = context.blocking(move |repository, sink| {
        let draft = new_draft(
            repository,
            &storage_providers::local::provider_key(),
            name.as_deref(),
        )?;
        let draft = update_storage_setup_draft_with_repository(
            repository,
            UpdateStorageSetupDraftRequest {
                draft_id: draft.draft_id.clone(),
                revision: draft.revision,
                name: draft.name.clone(),
                settings: BoundedConfigObject::from_json(&serde_json::json!({
                    "directory": path.to_string_lossy(),
                }))
                .map_err(|_| {
                    CliError::invalid_code(
                        "storage_directory_invalid",
                        "The folder path is not valid.",
                    )
                })?,
                secrets: Default::default(),
            },
        )
        .map_err(CliError::from_service)?;
        commit(repository, sink, &draft.draft_id, draft.revision)
    })?;
    saved(context, &committed, "added")
}

/// R2 goes through the Wrangler-driven backend the desktop also spawns: the
/// draft is created here, filled there, and saved here.
fn add_r2(context: &CliContext, args: StorageAddR2Args) -> Result<CommandOutput, CliError> {
    let jurisdiction = storage_providers::r2::R2Jurisdiction::parse(&args.jurisdiction)
        .ok_or_else(|| {
            CliError::invalid_code(
                "r2_jurisdiction_invalid",
                "The R2 jurisdiction must be default, eu, us, or fedramp.",
            )
        })?;
    if let Some(bucket) = &args.bucket {
        if !storage_providers::r2::valid_bucket_name(bucket) {
            return Err(CliError::invalid_code(
                "r2_bucket_name_invalid",
                "The R2 bucket name must use 3 to 63 lowercase letters, numbers, or hyphens and begin and end with a letter or number.",
            ));
        }
    }
    let name = args.name.clone();
    let draft = context.service(move |repository, _| {
        new_draft(
            repository,
            &storage_providers::r2::provider_key(),
            name.as_deref(),
        )
        .map_err(|error| error.message().to_string())
    })?;
    let backend = ProductionBackend::from_user_home()?;
    let target = match args.existing_bucket.zip(args.creation_date) {
        Some((name, creation_date)) => R2Target::Existing(ExistingBucket {
            name,
            creation_date,
        }),
        None => R2Target::New {
            bucket: args.bucket,
        },
    };
    backend.setup(
        SetupInput {
            name: draft.name.clone(),
            draft_id: draft.draft_id.to_string(),
            expected_revision: draft.revision,
        },
        args.login,
        args.account_id,
        jurisdiction,
        target,
    )?;
    finish_prepared(context, &draft.draft_id, "added")
}

/// The draft a sign-in filled: read its revision back and save it.
fn finish_prepared(
    context: &CliContext,
    draft_id: &StorageSetupDraftId,
    status: &'static str,
) -> Result<CommandOutput, CliError> {
    let id = draft_id.clone();
    let committed = context.blocking(move |repository, sink| {
        let draft = get_storage_setup_draft_with_repository(repository, &id)
            .map_err(CliError::from_service)?
            .ok_or_else(|| {
                CliError::persistence("The prepared draft disappeared before it was saved.")
            })?;
        commit(repository, sink, &id, draft.revision)
    })?;
    saved(context, &committed, status)
}

fn callback_error(error: loopback_browser::CallbackError) -> CliError {
    CliError::remote(error.code(), error.message())
}

fn add_dropbox(
    context: &CliContext,
    args: StorageAddDropboxArgs,
) -> Result<CommandOutput, CliError> {
    if let Some(folder) = &args.folder_name {
        if !dropbox_setup::valid_folder_name(folder) {
            return Err(CliError::invalid_code(
                "dropbox_folder_name_invalid",
                "The Dropbox folder name is not valid.",
            ));
        }
    }
    let committed = context.blocking(move |repository, sink| {
        let draft = new_draft(
            repository,
            &storage_providers::dropbox::provider_key(),
            args.name.as_deref(),
        )?;
        let scopes = if args.existing.is_some() {
            dropbox_auth::DISCOVERY_SCOPES
        } else {
            dropbox_auth::SETUP_SCOPES
        };
        let (client, grant) = dropbox_sign_in(sink, scopes, args.switch_account)?;
        let account = block_on(async {
            dropbox_setup::discovery::account(&client, &grant.access.token, &grant.account_id)
                .await
                .map_err(|error| CliError::remote(error.code(), error.message()))
        })?;
        let (namespace, selected_name, folder_name) = match &args.existing {
            Some(wanted) => {
                let scan = block_on(async {
                    dropbox_setup::discovery::discover(&client, &grant.access.token, &account.id)
                        .await
                        .map_err(|error| CliError::remote(error.code(), error.message()))
                })?;
                let folder = scan
                    .folders
                    .iter()
                    .find(|folder| {
                        &folder.name == wanted
                            && folder.marker
                                == mallard_infrastructure::storage_core::MarkerState::Storage
                    })
                    .ok_or_else(|| {
                        CliError::invalid_with(
                            "dropbox_folder_not_found",
                            format!(
                                "No shared folder named '{wanted}' holds a Mallard storage{}.",
                                if scan.partial {
                                    " (the scan was partial)"
                                } else {
                                    ""
                                }
                            ),
                        )
                    })?;
                (
                    Some(folder.namespace.clone()),
                    Some(folder.name.clone()),
                    None,
                )
            }
            None => {
                let name = match args.folder_name.clone() {
                    Some(name) => name,
                    None => dropbox_setup::propose_folder_name()
                        .map_err(|error| CliError::remote(error.code(), error.message()))?,
                };
                (None, None, Some(name))
            }
        };
        let operation =
            begin_storage_draft_operation(repository, &draft.draft_id, draft.revision, &draft.name)
                .map_err(setup_error)?;
        let prepared = block_on(async {
            mallard_application::dropbox_setup::prepare(
                repository,
                StorageProviderRegistry::production(),
                &operation,
                &draft.draft_id,
                &client,
                &grant,
                namespace.as_deref(),
                selected_name.as_deref(),
                folder_name.as_deref(),
            )
            .await
            .map_err(setup_error)
        })?;
        drop(operation);
        commit(
            repository,
            sink,
            &prepared.draft_id,
            prepared.draft_revision,
        )
    })?;
    saved(context, &committed, "added")
}

/// Dropbox's browser sign-in: the link goes to the sink, the callback comes
/// back on the fixed loopback port, and the code is exchanged for a grant.
fn dropbox_sign_in(
    sink: &dyn SyncEventSink,
    scopes: &str,
    switch_account: bool,
) -> Result<
    (
        dropbox_auth::DropboxClient,
        dropbox_auth::AuthorizationGrant,
    ),
    CliError,
> {
    let request = dropbox_auth::AuthorizationRequest::for_login(scopes, switch_account)
        .map_err(|error| CliError::remote(error.code(), error.message()))?;
    let cancel = AtomicBool::new(false);
    let code = loopback_browser::authorize(
        &dropbox_auth::DROPBOX_CALLBACK,
        &request.url,
        &request.state,
        dropbox_auth::CALLBACK_PORT,
        |url| {
            sink.authorization_link(url.as_str(), LINK_EXPIRES_IN);
            Ok(())
        },
        &cancel,
    )
    .map_err(callback_error)?;
    let client = dropbox_auth::DropboxClient::new()
        .map_err(|error| CliError::remote(error.code(), error.message()))?;
    let grant = block_on(async {
        client
            .exchange(&code, &request.verifier, scopes)
            .await
            .map_err(|error| CliError::remote(error.code(), error.message()))
    })?;
    Ok((client, grant))
}

fn add_feishu(context: &CliContext, args: StorageAddFeishuArgs) -> Result<CommandOutput, CliError> {
    if let Some(folder) = &args.folder_name {
        if !dropbox_setup::valid_folder_name(folder) {
            return Err(CliError::invalid_code(
                "feishu_folder_name_invalid",
                "The Feishu folder name is not valid.",
            ));
        }
    }
    let committed = context.blocking(move |repository, sink| {
        let draft = new_draft(
            repository,
            &storage_providers::feishu::provider_key(),
            args.name.as_deref(),
        )?;
        let (client, grant, account) = feishu_sign_in(sink)?;
        let (folder_token, folder_name) = match (&args.folder_name, &args.folder_link) {
            (_, Some(link)) => {
                let token = feishu_auth::folder_token_from_link(link).ok_or_else(|| {
                    CliError::invalid_code(
                        "feishu_folder_link_invalid",
                        "That is not a Feishu Drive folder link.",
                    )
                })?;
                block_on(async {
                    client
                        .folder_reachable(&grant.access.token, &token)
                        .await
                        .map_err(|failure| {
                            CliError::remote(failure.error.code(), failure.error.message())
                        })
                })?;
                (token, None)
            }
            (name, None) => {
                let name = match name.clone() {
                    Some(name) => name,
                    None => dropbox_setup::propose_folder_name()
                        .map_err(|error| CliError::remote(error.code(), error.message()))?,
                };
                let token = block_on(async {
                    client
                        .create_root_folder(&grant.access.token, &name)
                        .await
                        .map_err(|failure| {
                            CliError::remote(failure.error.code(), failure.error.message())
                        })
                })?;
                (token, Some(name))
            }
        };
        let connections = feishu_connections::installed().ok_or_else(|| {
            CliError::persistence(
                "Mallard could not open its Feishu sign-in store on this computer.",
            )
        })?;
        let operation =
            begin_storage_draft_operation(repository, &draft.draft_id, draft.revision, &draft.name)
                .map_err(setup_error)?;
        let prepared = mallard_application::feishu_setup::prepare(
            repository,
            StorageProviderRegistry::production(),
            &operation,
            &draft.draft_id,
            &connections,
            mallard_application::feishu_setup::FeishuPrepared {
                grant: &grant,
                account_name: &account.name,
                folder_token: Some(&folder_token),
                folder_name: folder_name.as_deref(),
            },
        )
        .map_err(setup_error)?;
        drop(operation);
        commit(
            repository,
            sink,
            &prepared.draft_id,
            prepared.draft_revision,
        )
    })?;
    saved(context, &committed, "added")
}

fn feishu_sign_in(
    sink: &dyn SyncEventSink,
) -> Result<
    (
        feishu_auth::FeishuClient,
        feishu_auth::AuthorizationGrant,
        feishu_auth::Account,
    ),
    CliError,
> {
    let app = feishu_auth::FeishuApp::built_in().ok_or_else(|| {
        CliError::remote(
            "feishu_app_unavailable",
            "This build carries no Feishu app registration.",
        )
    })?;
    let request = feishu_auth::AuthorizationRequest::new(&app.app_id)
        .map_err(|error| CliError::remote(error.code(), error.message()))?;
    let cancel = AtomicBool::new(false);
    let code = loopback_browser::authorize(
        &feishu_auth::FEISHU_CALLBACK,
        &request.url,
        &request.state,
        feishu_auth::CALLBACK_PORT,
        |url| {
            sink.authorization_link(url.as_str(), LINK_EXPIRES_IN);
            Ok(())
        },
        &cancel,
    )
    .map_err(callback_error)?;
    let client = feishu_auth::FeishuClient::new()
        .map_err(|error| CliError::remote(error.code(), error.message()))?;
    let (grant, account) = block_on(async {
        let grant = client
            .exchange(&app, &code, &request.verifier)
            .await
            .map_err(|failure| CliError::remote(failure.error.code(), failure.error.message()))?;
        let account = client
            .user_info(&grant.access.token)
            .await
            .map_err(|failure| CliError::remote(failure.error.code(), failure.error.message()))?;
        Ok((grant, account))
    })?;
    Ok((client, grant, account))
}

fn add_cloud(context: &CliContext, args: StorageAddCloudArgs) -> Result<CommandOutput, CliError> {
    let committed = context.blocking(move |repository, sink| {
        let draft = new_draft(
            repository,
            &storage_providers::managed_cloud::provider_key(),
            args.name.as_deref(),
        )?;
        let origin = managed_cloud::default_origin();
        let signed = cloud_sign_in(sink, &origin)?;
        let operation =
            begin_storage_draft_operation(repository, &draft.draft_id, draft.revision, &draft.name)
                .map_err(setup_error)?;
        let prepared = mallard_application::managed_cloud_setup::prepare(
            repository,
            StorageProviderRegistry::production(),
            &operation,
            &draft.draft_id,
            mallard_application::managed_cloud_setup::SignedInAccount {
                service_origin: &origin,
                github_user_id: &signed.user.github_user_id,
                account_login: &signed.user.github_login,
                token: &signed.token,
            },
        )
        .map_err(setup_error)?;
        drop(operation);
        commit(
            repository,
            sink,
            &prepared.draft_id,
            prepared.draft_revision,
        )
    })?;
    saved(context, &committed, "added")
}

fn cloud_sign_in(
    sink: &dyn SyncEventSink,
    origin: &str,
) -> Result<managed_cloud::SignedIn, CliError> {
    // The sign-in hands the link over from the runtime; the sink is shared
    // with that task for the one call.
    let (link_sender, link_receiver) = std::sync::mpsc::channel::<String>();
    let cancel = Arc::new(AtomicBool::new(false));
    let signed = block_on(async {
        let sign_in = managed_cloud::sign_in(
            origin,
            move |url| link_sender.send(url.to_string()).map_err(|_| ()),
            cancel,
        );
        let forward = async {
            // Poll the link once it is handed over; the sign-in itself keeps
            // waiting for the browser.
            loop {
                match link_receiver.try_recv() {
                    Ok(url) => {
                        sink.authorization_link(&url, LINK_EXPIRES_IN);
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        };
        let (signed, _) = tokio::join!(sign_in, forward);
        signed.map_err(|error| {
            CliError::remote(error.code(), "The Mallard Cloud sign-in did not complete.")
        })
    })?;
    Ok(signed)
}

/// Sign in again for a saved storage: an edit draft carries the saved
/// destination, the provider's sign-in refreshes the credential, and the
/// draft is saved over the storage.
pub(crate) fn reconnect(
    context: &CliContext,
    storage: StorageRef,
) -> Result<CommandOutput, CliError> {
    let id = context.resolve_storage(None, Some(&storage.storage))?;
    let provider = context
        .load_config()?
        .storages
        .into_iter()
        .find(|storage| storage.id == id)
        .map(|storage| storage.provider)
        .ok_or_else(|| {
            CliError::invalid_with("storage_not_found", "No storage has that id or name.")
        })?;
    let draft_storage = id.clone();
    let draft = context.service(move |repository, _| {
        create_storage_edit_draft_with_repository(repository, &draft_storage)
    })?;
    if provider == storage_providers::r2::provider_key() {
        let backend = ProductionBackend::from_user_home()?;
        backend.setup(
            SetupInput {
                name: draft.name.clone(),
                draft_id: draft.draft_id.to_string(),
                expected_revision: draft.revision,
            },
            false,
            None,
            storage_providers::r2::R2Jurisdiction::parse("default").expect("a known jurisdiction"),
            R2Target::New { bucket: None },
        )?;
        return finish_prepared(context, &draft.draft_id, "reconnected");
    }
    let committed = context.blocking(move |repository, sink| {
        let operation = begin_storage_draft_operation(repository, &draft.draft_id, draft.revision, &draft.name)
            .map_err(setup_error)?;
        let prepared = if provider == storage_providers::dropbox::provider_key() {
            let (client, grant) = dropbox_sign_in(sink, dropbox_auth::STORAGE_SCOPES, false)?;
            block_on(async {
                mallard_application::dropbox_setup::prepare(
                    repository,
                    StorageProviderRegistry::production(),
                    &operation,
                    &draft.draft_id,
                    &client,
                    &grant,
                    None,
                    None,
                    None,
                )
                .await
                .map_err(setup_error)
            })?
        } else if provider == storage_providers::feishu::provider_key() {
            let (_, grant, account) = feishu_sign_in(sink)?;
            let connections = feishu_connections::installed().ok_or_else(|| {
                CliError::persistence("Mallard could not open its Feishu sign-in store on this computer.")
            })?;
            mallard_application::feishu_setup::prepare(
                repository,
                StorageProviderRegistry::production(),
                &operation,
                &draft.draft_id,
                &connections,
                mallard_application::feishu_setup::FeishuPrepared {
                    grant: &grant,
                    account_name: &account.name,
                    folder_token: None,
                    folder_name: None,
                },
            )
            .map_err(setup_error)?
        } else if provider == storage_providers::managed_cloud::provider_key() {
            let origin = draft
                .settings
                .get_str("service_origin")
                .filter(|origin| !origin.is_empty() && managed_cloud::origin_allowed(origin))
                .map(str::to_owned)
                .unwrap_or_else(managed_cloud::default_origin);
            let signed = cloud_sign_in(sink, &origin)?;
            mallard_application::managed_cloud_setup::prepare(
                repository,
                StorageProviderRegistry::production(),
                &operation,
                &draft.draft_id,
                mallard_application::managed_cloud_setup::SignedInAccount {
                    service_origin: &origin,
                    github_user_id: &signed.user.github_user_id,
                    account_login: &signed.user.github_login,
                    token: &signed.token,
                },
            )
            .map_err(setup_error)?
        } else {
            drop(operation);
            let _ = discard_storage_setup_draft_with_repository(repository, &draft.draft_id);
            return Err(CliError::invalid_code(
                "storage_reconnect_unsupported",
                "This storage keeps no credential to renew. Edit its settings in the desktop app instead.",
            ));
        };
        drop(operation);
        commit(repository, sink, &prepared.draft_id, prepared.draft_revision)
    })?;
    saved(context, &committed, "reconnected")
}

pub(crate) fn draft(
    context: &CliContext,
    command: StorageDraftCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        StorageDraftCommand::List => {
            let list = context
                .service(|repository, _| list_storage_setup_drafts_with_repository(repository))?;
            let mut text = String::new();
            if list.drafts.is_empty() {
                text.push_str("No open storage setup drafts.\n");
            }
            for draft in &list.drafts {
                text.push_str(&draft_line(draft));
            }
            for warning in &list.warnings {
                text.push_str(&format!("warning: {warning}\n"));
            }
            CommandOutput::new("ok", "drafts", &list.drafts, text)
        }
        StorageDraftCommand::Show(draft) => {
            let id = draft_id(&draft.draft_id)?;
            let summary = context
                .service(move |repository, _| {
                    get_storage_setup_draft_with_repository(repository, &id)
                })?
                .ok_or_else(|| {
                    CliError::invalid_with(
                        "storage_setup_draft_not_found",
                        "No open draft has that id.",
                    )
                })?;
            let text = draft_line(&summary);
            CommandOutput::new("ok", "draft", &summary, text)
        }
        StorageDraftCommand::Test(draft) => {
            let id = draft_id(&draft.draft_id)?;
            let probe_id = id.clone();
            let revision = context
                .service(move |repository, _| {
                    get_storage_setup_draft_with_repository(repository, &probe_id)
                })?
                .ok_or_else(|| {
                    CliError::invalid_with(
                        "storage_setup_draft_not_found",
                        "No open draft has that id.",
                    )
                })?
                .revision;
            let result = context.service(move |repository, _| {
                test_storage_setup_draft_with_repository(
                    repository,
                    Handle::current(),
                    &id,
                    revision,
                )
            })?;
            let text = serde_json::to_string(&result).unwrap_or_default();
            CommandOutput::new("tested", "test", &result, text)
        }
        StorageDraftCommand::Commit(args) => {
            let id = draft_id(&args.draft.draft_id)?;
            let revision = args.expected_revision;
            let committed = context
                .blocking(move |repository, sink| commit(repository, sink, &id, revision))?;
            saved(context, &committed, "committed")
        }
        StorageDraftCommand::Discard(draft) => {
            let id = draft_id(&draft.draft_id)?;
            let discarded = context.service(move |repository, _| {
                discard_storage_setup_draft_with_repository(repository, &id)
            })?;
            CommandOutput::new(
                if discarded { "discarded" } else { "unchanged" },
                "discarded",
                &discarded,
                if discarded {
                    "Draft discarded. Nothing remote was revoked or deleted.".to_string()
                } else {
                    "No open draft has that id.".to_string()
                },
            )
        }
    }
}

fn draft_line(draft: &StorageSetupDraftSummary) -> String {
    format!(
        "{} [{}] — {}{}, revision {}\n",
        draft.name,
        draft.draft_id,
        draft.provider_label,
        if draft.edits_existing {
            format!(" (edits storage {})", draft.storage_id)
        } else {
            String::new()
        },
        draft.revision
    )
}
