//! Tauri command adapters for project sync.

use crate::event_sink::desktop_sink;
use mallard_application::activity_log::{emit_typed_log, ActivityLogScope};
use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::{
    agent_provider_descriptors, create_provider_profile_with_repository,
    create_storage_edit_draft_with_repository, create_storage_setup_draft_with_repository,
    discard_setup_draft_with_repository, discard_storage_setup_draft_with_repository,
    find_project_session_with_repository, get_bundle_inventory_with_repository,
    get_host_identity_status_with_repository, get_project_session_detail_with_repository,
    get_project_session_event_detail_with_repository,
    get_project_session_timeline_page_with_repository, get_project_with_repository,
    get_push_destination_status_with_repository, get_restore_readiness_with_repository,
    get_setup_draft_with_repository, get_storage_setup_draft_with_repository,
    get_stored_session_detail_with_repository, get_stored_session_event_detail_with_repository,
    get_stored_session_timeline_page_with_repository, inspect_project_files_with_repository,
    list_agent_folders_with_repository, list_local_projects_with_repository,
    list_project_bindings_with_repository, list_project_repository_kinds_with_repository,
    list_provider_profiles_with_repository, list_remote_bundle_snapshots_with_repository,
    list_setup_drafts_with_repository, list_storage_setup_drafts_with_repository,
    load_project_sync_config_with_repository, probe_project_folder_with_repository,
    probe_provider_profile_with_repository, read_project_markdown_image_with_repository,
    read_project_markdown_media_with_repository, recover_startup_state_with_repository,
    remove_local_project_with_repository, remove_project_link_with_repository,
    rename_local_project_with_repository, repository,
    resolve_project_markdown_path_with_repository, reveal_storage_secret_with_repository,
    save_project_binding_with_repository, save_project_link_with_repository, session_detail_error,
    sibling_bundle_status_with_repository, start_project_session_with_repository,
    storage_provider_descriptors, test_storage_setup_draft_with_repository,
    test_storage_with_repository, update_storage_setup_draft_with_repository, AgentFolderList,
    BundleReadiness, BundleSnapshotSummary, CommitStorageDraftResult, ConnectProjectBundleRequest,
    CreateSetupDraftResult, HostIdentityStatus, NewSessionSurface, ProjectActivity,
    ProjectActivityRefreshEvent, ProjectContentInventory, ProjectDetail, ProjectFolderProbe,
    ProjectMarkdownImage, ProjectOperationResult, ProjectSessionTimelineRequest,
    ProjectSessionsDeleteResult, ProjectSyncConfigView, ProjectSyncService, ProviderDescriptor,
    ProviderProfileProbe, ProviderProfileSummary, PushApproval, PushDestinationStatus,
    PushReviewContext, PushScope, ResourceInventory, RestoreResult, SaveProjectBindingRequest,
    SaveProjectLinkRequest, SessionDetailError, SessionDetailErrorCode, SessionTimelineEventKind,
    SessionToolCategory, SetupDraftInspection, SetupDraftList, SiblingBundleStatus,
    StorageDataClearResult, StorageProviderDescriptor, StorageSetupDraftList,
    StorageSetupDraftSummary, TestStorageDraftResult, UpdateStorageSetupDraftRequest,
    VerifyStorageResult,
};
use mallard_domain::{
    ActionId, BundleId, BundleRecipe, LocalProjectId, LocalProjectRegistration, ProjectBinding,
    ProjectSetupDraft, ProjectStorageLink, ProviderKey, ProviderProfile, PullReview, PullScope,
    ResourceId, SetupDraftId, StorageId, StorageProviderKey, StorageSetupDraftId,
};
use mallard_infrastructure::activity_log::ActivityLogType;
use std::collections::BTreeMap;
use std::path::Path;

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("project-sync worker failed: {}", error))?
}

async fn run_session_detail_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, SessionDetailError> + Send + 'static,
) -> Result<T, SessionDetailError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| {
            session_detail_error(
                SessionDetailErrorCode::ProviderError,
                "The session-detail worker stopped unexpectedly.",
            )
        })?
}

#[tauri::command]
pub async fn get_project_sync_config() -> Result<ProjectSyncConfigView, String> {
    let repository = repository()?;
    run_blocking(move || load_project_sync_config_with_repository(&repository)).await
}

#[tauri::command]
pub fn list_storage_providers() -> Vec<StorageProviderDescriptor> {
    storage_provider_descriptors()
}

#[tauri::command]
pub async fn list_storage_setup_drafts() -> Result<StorageSetupDraftList, String> {
    let repository = repository()?;
    run_blocking(move || list_storage_setup_drafts_with_repository(&repository)).await
}

#[tauri::command]
pub async fn create_storage_setup_draft(
    provider: StorageProviderKey,
) -> Result<StorageSetupDraftSummary, String> {
    let repository = repository()?;
    run_blocking(move || create_storage_setup_draft_with_repository(&repository, &provider)).await
}

#[tauri::command]
pub async fn create_storage_edit_draft(
    storage_id: StorageId,
) -> Result<StorageSetupDraftSummary, String> {
    let repository = repository()?;
    run_blocking(move || create_storage_edit_draft_with_repository(&repository, &storage_id)).await
}

#[tauri::command]
pub async fn get_storage_setup_draft(
    draft_id: StorageSetupDraftId,
) -> Result<Option<StorageSetupDraftSummary>, String> {
    let repository = repository()?;
    run_blocking(move || get_storage_setup_draft_with_repository(&repository, &draft_id)).await
}

#[tauri::command]
pub async fn update_storage_setup_draft(
    request: UpdateStorageSetupDraftRequest,
) -> Result<StorageSetupDraftSummary, String> {
    let repository = repository()?;
    run_blocking(move || update_storage_setup_draft_with_repository(&repository, request)).await
}

#[tauri::command]
pub async fn discard_storage_setup_draft(draft_id: StorageSetupDraftId) -> Result<bool, String> {
    let repository = repository()?;
    run_blocking(move || discard_storage_setup_draft_with_repository(&repository, &draft_id)).await
}

#[tauri::command]
pub async fn test_storage_setup_draft(
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
) -> Result<TestStorageDraftResult, String> {
    let repository = repository()?;
    let runtime = tokio::runtime::Handle::current();
    run_blocking(move || {
        test_storage_setup_draft_with_repository(&repository, runtime, &draft_id, expected_revision)
    })
    .await
}

#[tauri::command]
pub async fn commit_storage_setup_draft(
    app: tauri::AppHandle,
    draft_id: StorageSetupDraftId,
    expected_draft_revision: u64,
) -> Result<CommitStorageDraftResult, String> {
    let repository = repository()?;
    let runtime = tokio::runtime::Handle::current();
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::commit_storage_setup_draft(
            &repository,
            sink.as_ref(),
            runtime,
            &draft_id,
            expected_draft_revision,
        )
    })
    .await
}

#[tauri::command]
pub async fn reveal_storage_secret(
    app: tauri::AppHandle,
    storage_id: StorageId,
    secret_id: mallard_domain::SecretKey,
) -> Result<String, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        // The log names the storage and the secret's label, never its value.
        let config = load_project_sync_config_with_repository(&repository)?;
        let storage = config
            .storages
            .iter()
            .find(|storage| storage.storage_id == storage_id);
        let secret_label = storage
            .and_then(|storage| {
                storage_provider_descriptors()
                    .into_iter()
                    .find(|descriptor| {
                        descriptor.key == storage.provider
                            && descriptor.config_version == storage.config_version
                    })
            })
            .and_then(|descriptor| {
                descriptor
                    .secrets
                    .into_iter()
                    .find(|secret| secret.secret_id == secret_id)
                    .map(|secret| secret.label)
            })
            .unwrap_or_else(|| secret_id.as_str().to_string());
        let storage_name = storage
            .map(|storage| storage.name.clone())
            .unwrap_or_default();
        let result = reveal_storage_secret_with_repository(&repository, &storage_id, &secret_id);
        if result.is_ok() {
            ActivityLogScope::new(ActivityLogType::Storage)
                .storage(&storage_id, Some(&storage_name))
                .emit(
                    sink.as_ref(),
                    "ok",
                    "storage.secret_revealed",
                    &format!("Secret revealed — {storage_name} · {secret_label}"),
                );
        }
        result
    })
    .await
}

#[tauri::command]
pub async fn test_storage(storage_id: StorageId) -> Result<VerifyStorageResult, String> {
    let repository = repository()?;
    let runtime = tokio::runtime::Handle::current();
    run_blocking(move || test_storage_with_repository(&repository, runtime, &storage_id)).await
}

#[tauri::command]
pub async fn rename_storage(
    app: tauri::AppHandle,
    storage_id: StorageId,
    name: String,
    expected_config_revision: u64,
) -> Result<ProjectSyncConfigView, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::rename_storage(
            &repository,
            sink.as_ref(),
            &storage_id,
            &name,
            expected_config_revision,
        )
    })
    .await
}

#[tauri::command]
pub async fn remove_storage(
    app: tauri::AppHandle,
    storage_id: StorageId,
    expected_config_revision: u64,
) -> Result<ProjectSyncConfigView, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::remove_storage(
            &repository,
            sink.as_ref(),
            &storage_id,
            expected_config_revision,
        )
    })
    .await
}

#[tauri::command]
pub async fn get_r2_setup_accounts(
    control: tauri::State<'_, super::r2_setup::R2SetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    reauthenticate: bool,
    on_progress: tauri::ipc::Channel<super::r2_setup::R2SetupProgress>,
) -> Result<Vec<super::r2_setup::R2Account>, super::r2_setup::R2SetupFailure> {
    let repository = repository().map_err(|_| super::r2_setup::R2SetupFailure::failed())?;
    super::r2_setup::accounts(
        &control,
        repository,
        draft_id,
        expected_revision,
        reauthenticate,
        on_progress,
    )
    .await
}

#[tauri::command]
pub async fn setup_r2_storage(
    control: tauri::State<'_, super::r2_setup::R2SetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    account_id: String,
    jurisdiction: String,
    bucket: String,
    on_progress: tauri::ipc::Channel<super::r2_setup::R2SetupProgress>,
) -> Result<(), super::r2_setup::R2SetupFailure> {
    let repository = repository().map_err(|_| super::r2_setup::R2SetupFailure::failed())?;
    super::r2_setup::setup(
        &control,
        repository,
        draft_id,
        expected_revision,
        account_id,
        jurisdiction,
        bucket,
        on_progress,
    )
    .await
}

/// A readable name for the Bucket line to propose before a bucket is made.
#[tauri::command]
pub fn propose_r2_bucket_name() -> Result<String, super::r2_setup::R2SetupFailure> {
    super::r2_setup::propose()
}

#[tauri::command]
pub async fn discover_r2_storage(
    control: tauri::State<'_, super::r2_setup::R2SetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    account_id: String,
    on_progress: tauri::ipc::Channel<super::r2_setup::R2SetupProgress>,
) -> Result<super::r2_setup::DiscoveryResult, super::r2_setup::R2SetupFailure> {
    let repository = repository().map_err(|_| super::r2_setup::R2SetupFailure::failed())?;
    super::r2_setup::discover(
        &control,
        repository,
        draft_id,
        expected_revision,
        account_id,
        on_progress,
    )
    .await
}

#[tauri::command]
pub async fn prepare_existing_r2_storage(
    control: tauri::State<'_, super::r2_setup::R2SetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    scan_id: String,
    candidate_id: String,
    on_progress: tauri::ipc::Channel<super::r2_setup::R2SetupProgress>,
) -> Result<(), super::r2_setup::R2SetupFailure> {
    let repository = repository().map_err(|_| super::r2_setup::R2SetupFailure::failed())?;
    super::r2_setup::prepare_existing(
        &control,
        repository,
        draft_id,
        expected_revision,
        scan_id,
        candidate_id,
        on_progress,
    )
    .await
}

#[tauri::command]
pub fn cancel_r2_storage_setup(
    control: tauri::State<'_, super::r2_setup::R2SetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
) -> Result<(), super::r2_setup::R2SetupFailure> {
    control.cancel(draft_id, expected_revision)
}

#[tauri::command]
pub async fn get_host_identity_status() -> Result<HostIdentityStatus, String> {
    let repository = repository()?;
    run_blocking(move || get_host_identity_status_with_repository(&repository)).await
}

#[tauri::command]
pub async fn list_local_projects() -> Result<Vec<LocalProjectRegistration>, String> {
    let repository = repository()?;
    run_blocking(move || list_local_projects_with_repository(&repository)).await
}

#[tauri::command]
pub async fn recover_startup_state(app: tauri::AppHandle) -> Result<(), String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        for warning in recover_startup_state_with_repository(&repository) {
            emit_typed_log(
                sink.as_ref(),
                ActivityLogType::Configuration,
                "warning",
                &warning,
            );
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn get_project(
    local_project_id: LocalProjectId,
) -> Result<Option<ProjectDetail>, String> {
    let repository = repository()?;
    run_blocking(move || get_project_with_repository(&repository, &local_project_id)).await
}

#[tauri::command]
pub async fn list_project_repository_kinds() -> Result<BTreeMap<String, bool>, String> {
    let repository = repository()?;
    run_blocking(move || list_project_repository_kinds_with_repository(&repository)).await
}

#[tauri::command]
pub async fn list_agent_providers() -> Result<Vec<ProviderDescriptor>, String> {
    Ok(agent_provider_descriptors())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_activity(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: Option<StorageId>,
    branch: Option<String>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    review: bool,
) -> Result<ProjectActivity, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::get_project_activity(
            &repository,
            sink.as_ref(),
            &local_project_id,
            storage_id.as_ref(),
            branch.as_deref(),
            before_time,
            window_days,
            review,
        )
    })
    .await
}

#[tauri::command]
pub async fn refresh_project_activity(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    branch: Option<String>,
    window_days: Option<u64>,
    review: bool,
    on_progress: tauri::ipc::Channel<ProjectActivityRefreshEvent>,
) -> Result<(), String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::refresh_project_activity(
            &repository,
            sink.as_ref(),
            &local_project_id,
            &storage_id,
            branch.as_deref(),
            window_days,
            review,
            &mut |event| {
                on_progress
                    .send(event)
                    .map_err(|error| format!("send activity refresh: {error}"))
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn get_project_session_detail(
    local_project_id: LocalProjectId,
    resource_id: String,
    // Set for a stored row (`…:cloudthread-…`): the storage its page reads
    // from. A local row refuses one and a stored row needs one; the two
    // reads below take it the same way.
    storage_id: Option<StorageId>,
) -> Result<tauri::ipc::Response, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    let encoded = run_session_detail_blocking(move || {
        let detail = match &storage_id {
            Some(storage_id) => get_stored_session_detail_with_repository(
                &repository,
                &local_project_id,
                storage_id,
                &resource_id,
            )?,
            None => get_project_session_detail_with_repository(
                &repository,
                &local_project_id,
                &resource_id,
            )?,
        };
        mallard_infrastructure::session_detail::encode_project_session_detail_response(&detail)
    })
    .await?;
    Ok(tauri::ipc::Response::new(encoded))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_session_timeline_page(
    local_project_id: LocalProjectId,
    resource_id: String,
    snapshot_id: String,
    cursor: Option<String>,
    limit: Option<usize>,
    included_event_kinds: Vec<SessionTimelineEventKind>,
    included_tool_categories: Vec<SessionToolCategory>,
    turn_id: Option<String>,
    storage_id: Option<StorageId>,
) -> Result<tauri::ipc::Response, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    let encoded = run_session_detail_blocking(move || {
        let request = ProjectSessionTimelineRequest {
            snapshot_id,
            cursor,
            limit,
            included_event_kinds: included_event_kinds.into_iter().collect(),
            included_tool_categories: included_tool_categories.into_iter().collect(),
            turn_id,
        };
        let page = match &storage_id {
            Some(storage_id) => get_stored_session_timeline_page_with_repository(
                &repository,
                &local_project_id,
                storage_id,
                &resource_id,
                request,
            )?,
            None => get_project_session_timeline_page_with_repository(
                &repository,
                &local_project_id,
                &resource_id,
                request,
            )?,
        };
        mallard_infrastructure::session_detail::encode_project_session_timeline_page_response(&page)
    })
    .await?;
    Ok(tauri::ipc::Response::new(encoded))
}

#[tauri::command]
pub async fn get_project_session_event_detail(
    local_project_id: LocalProjectId,
    resource_id: String,
    snapshot_id: String,
    event_id: String,
    storage_id: Option<StorageId>,
) -> Result<tauri::ipc::Response, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    let encoded = run_session_detail_blocking(move || {
        let detail = match &storage_id {
            Some(storage_id) => get_stored_session_event_detail_with_repository(
                &repository,
                &local_project_id,
                storage_id,
                &resource_id,
                &snapshot_id,
                &event_id,
            )?,
            None => get_project_session_event_detail_with_repository(
                &repository,
                &local_project_id,
                &resource_id,
                &snapshot_id,
                &event_id,
            )?,
        };
        mallard_infrastructure::session_detail::encode_session_event_detail_response(&detail)
    })
    .await?;
    Ok(tauri::ipc::Response::new(encoded))
}

#[tauri::command]
pub async fn open_project_markdown_path(
    local_project_id: LocalProjectId,
    relative_path: String,
) -> Result<(), SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    run_session_detail_blocking(move || {
        let path = resolve_project_markdown_path_with_repository(
            &repository,
            &local_project_id,
            &relative_path,
        )?;
        tauri_plugin_opener::open_path(path, None::<&str>).map_err(|_| {
            session_detail_error(
                SessionDetailErrorCode::ProviderError,
                "Mallard could not open the linked project path.",
            )
        })
    })
    .await
}

#[tauri::command]
pub async fn read_project_markdown_image(
    local_project_id: LocalProjectId,
    relative_path: String,
) -> Result<ProjectMarkdownImage, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    run_session_detail_blocking(move || {
        read_project_markdown_image_with_repository(&repository, &local_project_id, &relative_path)
    })
    .await
}

/// A project track or clip for an inline player, as raw bytes: the webview
/// wraps them in a `blob:` URL rather than a base64 `data:` string.
#[tauri::command]
pub async fn read_project_markdown_media(
    local_project_id: LocalProjectId,
    relative_path: String,
) -> Result<tauri::ipc::Response, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    let bytes = run_session_detail_blocking(move || {
        read_project_markdown_media_with_repository(&repository, &local_project_id, &relative_path)
    })
    .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn delete_local_project_session(
    local_project_id: LocalProjectId,
    resource_id: String,
) -> Result<mallard_infrastructure::project_activity::LocalSessionDeleteResult, String> {
    let repository = repository()?;
    run_blocking(move || {
        mallard_application::project_sync::service::delete_local_project_session_with_repository(
            &repository,
            &local_project_id,
            &resource_id,
        )
    })
    .await
}

#[tauri::command]
pub async fn execute_project_session_action(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    resource_id: String,
    action_id: String,
) -> Result<(), String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::execute_project_session_action(
            &repository,
            sink.as_ref(),
            &local_project_id,
            &resource_id,
            &action_id,
        )
    })
    .await
}

/// Start a new session of one bound provider in the project. The line that
/// runs goes to the activity log, as a session action's does.
#[tauri::command]
pub async fn start_project_session(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    provider: ProviderKey,
    surface: NewSessionSurface,
) -> Result<(), String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    let log = ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    run_blocking(move || {
        start_project_session_with_repository(
            &repository,
            &local_project_id,
            &provider,
            surface,
            &mut |command| {
                log.emit(
                    sink.as_ref(),
                    "info",
                    "history.session_start",
                    &format!("$ {command}"),
                )
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn remove_local_project(local_project_id: LocalProjectId) -> Result<bool, String> {
    let repository = repository()?;
    run_blocking(move || remove_local_project_with_repository(&repository, &local_project_id)).await
}

#[tauri::command]
pub async fn delete_local_project_sessions(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    expected_project_revision: u64,
) -> Result<ProjectSessionsDeleteResult, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::delete_local_project_sessions(
            &repository,
            sink.as_ref(),
            &local_project_id,
            expected_project_revision,
        )
    })
    .await
}

#[tauri::command]
pub async fn clear_storage_data(
    app: tauri::AppHandle,
    storage_id: StorageId,
    expected_config_revision: u64,
) -> Result<StorageDataClearResult, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::clear_storage_data(
            &repository,
            sink.as_ref(),
            &storage_id,
            expected_config_revision,
        )
    })
    .await
}

#[tauri::command]
pub async fn rename_local_project(
    local_project_id: LocalProjectId,
    local_alias: Option<String>,
    expected_revision: u64,
) -> Result<LocalProjectRegistration, String> {
    let repository = repository()?;
    run_blocking(move || {
        rename_local_project_with_repository(
            &repository,
            &local_project_id,
            local_alias,
            expected_revision,
        )
    })
    .await
}

#[tauri::command]
pub async fn save_project_link(
    request: SaveProjectLinkRequest,
) -> Result<ProjectStorageLink, String> {
    let repository = repository()?;
    run_blocking(move || save_project_link_with_repository(&repository, request)).await
}

#[tauri::command]
pub async fn connect_project_to_remote_bundle(
    app: tauri::AppHandle,
    request: ConnectProjectBundleRequest,
) -> Result<ProjectDetail, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::connect_project_to_remote_bundle(&repository, sink.as_ref(), request)
    })
    .await
}

#[tauri::command]
pub async fn remove_project_link(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<bool, String> {
    let repository = repository()?;
    run_blocking(move || {
        remove_project_link_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn list_provider_profiles() -> Result<Vec<ProviderProfileSummary>, String> {
    let repository = repository()?;
    run_blocking(move || list_provider_profiles_with_repository(&repository)).await
}

#[tauri::command]
pub async fn probe_provider_profile(
    provider: ProviderKey,
    path: String,
) -> Result<ProviderProfileProbe, String> {
    let repository = repository()?;
    run_blocking(move || probe_provider_profile_with_repository(&repository, &provider, &path))
        .await
}

#[tauri::command]
pub async fn create_provider_profile(
    provider: ProviderKey,
    display_name: String,
    path: String,
) -> Result<ProviderProfile, String> {
    let repository = repository()?;
    run_blocking(move || {
        create_provider_profile_with_repository(&repository, &provider, &display_name, &path)
    })
    .await
}

#[tauri::command]
pub async fn list_project_bindings() -> Result<Vec<ProjectBinding>, String> {
    let repository = repository()?;
    run_blocking(move || list_project_bindings_with_repository(&repository)).await
}

#[tauri::command]
pub async fn execute_provider_maintenance_action(
    app: tauri::AppHandle,
    request: mallard_application::project_sync::maintenance::MaintenanceActionRequest,
) -> Result<mallard_application::project_sync::maintenance::MaintenanceActionResult, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::execute_provider_maintenance_action(&repository, sink.as_ref(), request)
    })
    .await
}

#[tauri::command]
pub async fn save_project_binding(
    request: SaveProjectBindingRequest,
) -> Result<ProjectBinding, String> {
    let repository = repository()?;
    run_blocking(move || save_project_binding_with_repository(&repository, request)).await
}

#[tauri::command]
pub async fn get_bundle_inventory(
    local_project_id: LocalProjectId,
) -> Result<ResourceInventory, String> {
    let repository = repository()?;
    run_blocking(move || get_bundle_inventory_with_repository(&repository, &local_project_id)).await
}

#[tauri::command]
pub async fn inspect_project_files(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<ProjectContentInventory, String> {
    let repository = repository()?;
    run_blocking(move || {
        inspect_project_files_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn list_remote_bundle_snapshots(
    storage_id: StorageId,
) -> Result<Vec<BundleSnapshotSummary>, String> {
    let repository = repository()?;
    run_blocking(move || list_remote_bundle_snapshots_with_repository(&repository, &storage_id))
        .await
}

/// *Finish that push*: write the record that the claim of the selected
/// version holds, once, and report. The user's verb, from the Push review.
#[tauri::command]
pub async fn finish_unfinished_push(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<(), String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::finish_unfinished_push(
            &repository,
            sink.as_ref(),
            &local_project_id,
            &storage_id,
        )
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn push_bundle(
    credential_redaction_enabled: bool,
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    recipe: BundleRecipe,
    scope: PushScope,
    subagent_selections: std::collections::BTreeMap<ResourceId, mallard_domain::SubagentSelection>,
    project_content_review_token: Option<String>,
    project_files_shown: bool,
    git_history_review_token: Option<String>,
    project_content_removal_ids: Option<Vec<ResourceId>>,
    acknowledged_warning_digests: Option<Vec<String>>,
) -> Result<ProjectOperationResult, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::push_bundle(
            &repository,
            sink.as_ref(),
            &local_project_id,
            &storage_id,
            PushApproval {
                scope,
                credential_redaction_enabled,
                subagent_selections,
                recipe,
                project_content_review_token,
                project_files_shown,
                git_history_review_token,
                project_content_removal_ids: project_content_removal_ids.unwrap_or_default(),
                acknowledged_warning_digests: acknowledged_warning_digests.unwrap_or_default(),
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn plan_bundle_restore(
    app: tauri::AppHandle,
    storage_id: StorageId,
    bundle_id: BundleId,
    binding: ProjectBinding,
    scope: PullScope,
) -> Result<PullReview, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::plan_bundle_restore(
            &repository,
            sink.as_ref(),
            &storage_id,
            &bundle_id,
            &binding,
            &scope,
        )
    })
    .await
}

#[tauri::command]
pub async fn apply_bundle_restore(
    app: tauri::AppHandle,
    review: PullReview,
    approved_action_ids: Vec<ActionId>,
) -> Result<RestoreResult, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::apply_bundle_restore(&repository, sink.as_ref(), review, approved_action_ids)
    })
    .await
}

/// Where the session a stored row names is in a project: the id its page is
/// opened by, or none when the project does not hold it. Reads only.
#[tauri::command]
pub async fn find_project_session(
    local_project_id: LocalProjectId,
    resource_id: ResourceId,
) -> Result<Option<ResourceId>, String> {
    let repository = repository()?;
    run_blocking(move || {
        find_project_session_with_repository(&repository, &local_project_id, &resource_id)
    })
    .await
}

#[tauri::command]
pub async fn get_restore_readiness(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<BundleReadiness, String> {
    let repository = repository()?;
    run_blocking(move || {
        get_restore_readiness_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn get_sibling_bundle_status(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<SiblingBundleStatus, String> {
    let repository = repository()?;
    run_blocking(move || {
        sibling_bundle_status_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn prepare_push_review(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    scope: PushScope,
) -> Result<PushReviewContext, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::prepare_push_review(
            &repository,
            sink.as_ref(),
            &local_project_id,
            &storage_id,
            scope,
        )
    })
    .await
}

#[tauri::command]
pub async fn get_push_destination_status(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<PushDestinationStatus, String> {
    let repository = repository()?;
    run_blocking(move || {
        get_push_destination_status_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn list_setup_drafts() -> Result<SetupDraftList, String> {
    let repository = repository()?;
    run_blocking(move || list_setup_drafts_with_repository(&repository)).await
}

/// The Folder field's verdict for a typed path. Reads only.
#[tauri::command]
pub async fn probe_project_folder(project_root: String) -> Result<ProjectFolderProbe, String> {
    let repository = repository()?;
    run_blocking(move || probe_project_folder_with_repository(&repository, &project_root)).await
}

/// Folders the newest sessions of every registered profile ran in. Reads
/// only, and a bounded number of session files per profile.
#[tauri::command]
pub async fn list_agent_folders() -> Result<AgentFolderList, String> {
    let repository = repository()?;
    run_blocking(move || list_agent_folders_with_repository(&repository)).await
}

#[tauri::command]
pub async fn create_setup_draft(project_root: String) -> Result<CreateSetupDraftResult, String> {
    let repository = repository()?;
    run_blocking(move || {
        ProjectSyncService::new(&repository).create_setup_draft(Path::new(&project_root))
    })
    .await
}

#[tauri::command]
pub async fn get_setup_draft(draft_id: SetupDraftId) -> Result<Option<ProjectSetupDraft>, String> {
    let repository = repository()?;
    run_blocking(move || get_setup_draft_with_repository(&repository, &draft_id)).await
}

#[tauri::command]
pub async fn update_setup_draft(draft: ProjectSetupDraft) -> Result<ProjectSetupDraft, String> {
    let repository = repository()?;
    run_blocking(move || ProjectSyncService::new(&repository).update_setup_draft(draft)).await
}

#[tauri::command]
pub async fn discard_setup_draft(draft_id: SetupDraftId) -> Result<bool, String> {
    let repository = repository()?;
    run_blocking(move || discard_setup_draft_with_repository(&repository, &draft_id)).await
}

#[tauri::command]
pub async fn inspect_setup_draft(draft_id: SetupDraftId) -> Result<SetupDraftInspection, String> {
    let repository = repository()?;
    run_blocking(move || ProjectSyncService::new(&repository).inspect_setup_draft(&draft_id)).await
}

#[tauri::command]
pub async fn finalize_project_setup(
    app: tauri::AppHandle,
    draft_id: SetupDraftId,
    expected_revision: u64,
) -> Result<ProjectDetail, String> {
    let repository = repository()?;
    let sink = desktop_sink(&app);
    run_blocking(move || {
        operations::finalize_project_setup(&repository, sink.as_ref(), &draft_id, expected_revision)
    })
    .await
}
