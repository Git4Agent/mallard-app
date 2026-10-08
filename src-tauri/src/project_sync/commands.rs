//! Tauri command adapters for project sync.

use super::service::{
    agent_provider_descriptors, claim_history_scan_log_at,
    commit_storage_setup_draft_with_repository, connect_project_to_remote_bundle_with_repository,
    create_provider_profile_with_repository, create_storage_edit_draft_with_repository,
    create_storage_setup_draft_with_repository, current_binding_for_restore_plan,
    delete_local_project_sessions_with_repository, discard_setup_draft_with_repository,
    discard_storage_setup_draft_with_repository, execute_project_session_action_with_repository,
    find_project_session_with_repository, format_fetch_started_message, format_mb,
    get_bundle_inventory_with_repository, get_host_identity_status_with_repository,
    get_project_activity_with_repository, get_project_session_detail_with_repository,
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
    probe_provider_profile_with_repository, project_open_commands, project_storage_log_context,
    pull_action_resource_label, pull_plan_action_counts, pull_review_resource_labels,
    read_project_markdown_image_with_repository, read_project_markdown_media_with_repository,
    recover_startup_state_with_repository, refresh_project_activity_with_progress,
    remove_local_project_with_repository, remove_project_link_with_repository,
    remove_storage_with_repository, rename_local_project_with_repository,
    rename_storage_with_repository, repository, resolve_project_markdown_path_with_repository,
    reveal_storage_secret_with_repository, save_project_binding_with_repository,
    save_project_link_with_repository, session_detail_error, should_log_history_scan,
    sibling_bundle_status_with_repository, start_project_session_with_repository,
    storage_log_label, storage_provider_descriptors, test_storage_setup_draft_with_repository,
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
    VerifyStorageResult, HISTORY_SCAN_LOGGED_AT,
};
use crate::activity_log::{emit_typed_log, ActivityLogScope};
use mallard_domain::{
    now_secs, ActionId, BundleId, BundleRecipe, LocalProjectId, LocalProjectRegistration,
    ProjectBinding, ProjectSetupDraft, ProjectStorageLink, ProviderKey, ProviderProfile,
    PullReview, PullScope, ResourceId, RestorePlan, SetupDraftId, StorageId, StorageProviderKey,
    StorageSetupDraftId,
};
use mallard_infrastructure::activity_log::ActivityLogType;
use mallard_infrastructure::bundle_engine::FetchProgress;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;
use tauri::Emitter as _;

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

#[derive(Serialize, Clone)]
struct FetchProgressEventPayload {
    phase: &'static str,
    storage_id: String,
    bundle_id: String,
    index: usize,
    total_objects: usize,
    fetched_bytes: u64,
    total_bytes: u64,
}

struct FetchProgressEmitter {
    app: tauri::AppHandle,
    log: ActivityLogScope,
    phase: &'static str,
    storage_id: StorageId,
    storage_name: String,
    bundle_id: BundleId,
    started: Option<Instant>,
    last_event: Option<Instant>,
}

impl FetchProgressEmitter {
    fn new(
        app: tauri::AppHandle,
        log: ActivityLogScope,
        phase: &'static str,
        storage_id: StorageId,
        storage_name: String,
        bundle_id: BundleId,
    ) -> Self {
        Self {
            app,
            log,
            phase,
            storage_id,
            storage_name,
            bundle_id,
            started: None,
            last_event: None,
        }
    }

    fn observe(&mut self, progress: FetchProgress) {
        let now = Instant::now();
        let upload = self.phase == "push";
        if self.started.is_none() {
            self.started = Some(now);
            self.log.emit(
                &self.app,
                "info",
                if upload {
                    "push.upload_started"
                } else {
                    "pull.fetch_started"
                },
                &format_fetch_started_message(
                    upload,
                    progress.total_objects,
                    progress.total_bytes,
                    &self.storage_name,
                ),
            );
        }
        let last = progress.index == progress.total_objects;
        let due = self
            .last_event
            .is_none_or(|previous| now.duration_since(previous).as_millis() >= 150);
        if last || due {
            self.last_event = Some(now);
            let _ = self.app.emit(
                "project-sync-fetch-progress",
                &FetchProgressEventPayload {
                    phase: self.phase,
                    storage_id: self.storage_id.to_string(),
                    bundle_id: self.bundle_id.to_string(),
                    index: progress.index,
                    total_objects: progress.total_objects,
                    fetched_bytes: progress.fetched_bytes,
                    total_bytes: progress.total_bytes,
                },
            );
        }
        if last {
            let elapsed = self
                .started
                .map(|start| now.duration_since(start).as_secs_f64())
                .unwrap_or_default();
            let rate = if elapsed > 0.0 {
                progress.total_bytes as f64 / 1_000_000.0 / elapsed
            } else {
                0.0
            };
            self.log.emit(
                &self.app,
                "ok",
                if upload {
                    "push.upload_completed"
                } else {
                    "pull.fetch_completed"
                },
                &format!(
                    "{}  {} {} in {:.1}s ({:.1} MB/s)",
                    if upload { "↑" } else { "↓" },
                    if upload { "Uploaded" } else { "Downloaded" },
                    format_mb(progress.total_bytes),
                    elapsed,
                    rate
                ),
            );
        }
    }
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
    let label_repository = repository.clone();
    let label_draft_id = draft_id.clone();
    let storage_name = run_blocking(move || {
        Ok(
            get_storage_setup_draft_with_repository(&label_repository, &label_draft_id)?
                .map(|draft| draft.name)
                .unwrap_or_default(),
        )
    })
    .await?;
    let result = run_blocking(move || {
        commit_storage_setup_draft_with_repository(
            &repository,
            runtime,
            &draft_id,
            expected_draft_revision,
        )
    })
    .await;
    match &result {
        Ok(outcome) => {
            let (event, message) = if outcome.replaced {
                (
                    "storage.settings_updated",
                    format!("Storage settings updated — {storage_name}"),
                )
            } else {
                ("storage.added", format!("Storage added — {storage_name}"))
            };
            ActivityLogScope::new(ActivityLogType::Storage)
                .storage(&outcome.storage_id, Some(&storage_name))
                .emit(&app, "ok", event, &message);
        }
        Err(_) => {
            // Keep the activity record fixed and credential-free even if the
            // worker itself failed rather than returning a typed setup error.
            ActivityLogScope::new(ActivityLogType::Storage).emit(
                &app,
                "error",
                "storage.settings_update_failed",
                "Storage settings could not be saved.",
            );
        }
    }
    result
}

#[tauri::command]
pub async fn reveal_storage_secret(
    app: tauri::AppHandle,
    storage_id: StorageId,
    secret_id: mallard_domain::SecretKey,
) -> Result<String, String> {
    let repository = repository()?;
    let log_storage_id = storage_id.clone();
    let log_repository = repository.clone();
    let log_secret_id = secret_id.clone();
    // The log names the storage and the secret's label, never its value.
    let (storage_name, secret_label) = run_blocking(move || {
        let config = load_project_sync_config_with_repository(&log_repository)?;
        let storage = config
            .storages
            .iter()
            .find(|storage| storage.storage_id == log_storage_id);
        let label = storage
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
                    .find(|secret| secret.secret_id == log_secret_id)
                    .map(|secret| secret.label)
            })
            .unwrap_or_else(|| log_secret_id.as_str().to_string());
        Ok((
            storage
                .map(|storage| storage.name.clone())
                .unwrap_or_default(),
            label,
        ))
    })
    .await?;
    let log_storage_id = storage_id.clone();
    let result = run_blocking(move || {
        reveal_storage_secret_with_repository(&repository, &storage_id, &secret_id)
    })
    .await;
    if result.is_ok() {
        ActivityLogScope::new(ActivityLogType::Storage)
            .storage(&log_storage_id, Some(&storage_name))
            .emit(
                &app,
                "ok",
                "storage.secret_revealed",
                &format!("Secret revealed — {storage_name} · {secret_label}"),
            );
    }
    result
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
    let log_storage_id = storage_id.clone();
    let log_name = name.clone();
    let result = run_blocking(move || {
        rename_storage_with_repository(&repository, &storage_id, &name, expected_config_revision)
    })
    .await;
    let log =
        ActivityLogScope::new(ActivityLogType::Storage).storage(&log_storage_id, Some(&log_name));
    match &result {
        Ok(_) => log.emit(
            &app,
            "ok",
            "storage.settings_updated",
            &format!("Storage settings updated — {log_name}"),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "storage.settings_update_failed",
            &format!("Storage settings could not be saved: {error}"),
        ),
    }
    result
}

#[tauri::command]
pub async fn remove_storage(
    app: tauri::AppHandle,
    storage_id: StorageId,
    expected_config_revision: u64,
) -> Result<ProjectSyncConfigView, String> {
    let repository = repository()?;
    let storage_name = storage_log_label(&repository, &storage_id);
    let log_storage_id = storage_id.clone();
    let result = run_blocking(move || {
        remove_storage_with_repository(&repository, &storage_id, expected_config_revision)
    })
    .await;
    let log = ActivityLogScope::new(ActivityLogType::Storage)
        .storage(&log_storage_id, Some(&storage_name));
    match &result {
        Ok(_) => log.emit(
            &app,
            "ok",
            "storage.removed",
            &format!("Storage removed — {storage_name}"),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "storage.settings_update_failed",
            &format!("Storage could not be removed: {error}"),
        ),
    }
    result
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
    let warnings =
        run_blocking(move || Ok(recover_startup_state_with_repository(&repository))).await?;
    for warning in warnings {
        emit_typed_log(&app, ActivityLogType::Configuration, "warning", &warning);
    }
    Ok(())
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
    let log = ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    let log_routine_scan = should_log_history_scan(&local_project_id);
    if log_routine_scan {
        log.emit(
            &app,
            "info",
            "history.scan_started",
            &format!("Scanning agent session activity for project {local_project_id}…"),
        );
    }
    let result = run_blocking(move || {
        get_project_activity_with_repository(
            &repository,
            &local_project_id,
            storage_id.as_ref(),
            branch.as_deref(),
            before_time,
            window_days,
            review,
        )
    })
    .await;
    match &result {
        Ok(activity) => {
            for warning in &activity.warnings {
                log.emit(&app, "warning", "history.scan_warning", warning);
            }
            if log_routine_scan {
                log.emit(
                    &app,
                    "info",
                    "history.scan_completed",
                    &format!(
                        "Session activity scan complete: {} sessions in this {}-day window",
                        activity.sessions.len(),
                        activity.window_end.saturating_sub(activity.window_start) / (24 * 60 * 60)
                    ),
                );
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "history.scan_failed",
            &format!("Session activity scan failed: {error}"),
        ),
    }
    result
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
    let log = ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    let log_routine_compare = HISTORY_SCAN_LOGGED_AT
        .lock()
        .map(|mut logged_at| {
            claim_history_scan_log_at(
                &mut logged_at,
                &format!("compare:{local_project_id}"),
                now_secs(),
            )
        })
        .unwrap_or(true);
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        refresh_project_activity_with_progress(
            &repository,
            &local_project_id,
            &storage_id,
            branch.as_deref(),
            window_days,
            review,
            &mut |event| {
                match &event {
                    ProjectActivityRefreshEvent::ActivityError { message, .. }
                    | ProjectActivityRefreshEvent::ComparisonError { message, .. } => {
                        phase_log.emit(&phase_app, "warning", "history.refresh_warning", message)
                    }
                    _ => {}
                }
                on_progress
                    .send(event)
                    .map_err(|error| format!("send activity refresh: {error}"))
            },
            &mut |phase, elapsed| {
                if log_routine_compare {
                    phase_log.emit(
                        &phase_app,
                        "info",
                        "history.compare_phase_timing",
                        &format!(
                            "Storage comparison phase — {phase}: {:.3}s",
                            elapsed.as_secs_f64()
                        ),
                    );
                }
            },
        )
    })
    .await;
    if let Err(error) = &result {
        log.emit(&app, "error", "history.refresh_failed", error);
    } else if log_routine_compare {
        log.emit(
            &app,
            "info",
            "history.refresh_completed",
            &format!(
                "Activity refresh complete in {:.3}s",
                total_started.elapsed().as_secs_f64()
            ),
        );
    }
    result
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
        super::service::delete_local_project_session_with_repository(
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
    let worker_app = app.clone();
    let worker_log =
        ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    run_blocking(move || {
        execute_project_session_action_with_repository(
            &repository,
            &local_project_id,
            &resource_id,
            &action_id,
            &mut |command| {
                worker_log.emit(
                    &worker_app,
                    "info",
                    "history.session_action",
                    &format!("$ {command}"),
                )
            },
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
    let worker_app = app.clone();
    let worker_log =
        ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    run_blocking(move || {
        start_project_session_with_repository(
            &repository,
            &local_project_id,
            &provider,
            surface,
            &mut |command| {
                worker_log.emit(
                    &worker_app,
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
    let project_for_work = local_project_id.clone();
    let log = ActivityLogScope::new(ActivityLogType::Repair).project(&local_project_id, None);
    log.emit(
        &app,
        "warning",
        "repair.project_sessions_delete_started",
        "Deleting this project's local sessions…",
    );
    let result = run_blocking(move || {
        delete_local_project_sessions_with_repository(
            &repository,
            &project_for_work,
            expected_project_revision,
        )
    })
    .await;
    match &result {
        Ok(summary) => {
            let metadata = if summary.metadata_reset.is_some() {
                "completed"
            } else {
                "failed"
            };
            log.emit(
                &app,
                if summary.provider_errors.is_empty() && summary.metadata_error.is_none() {
                    "ok"
                } else {
                    "warning"
                },
                "repair.project_sessions_delete_completed",
                &format!(
                    "Session deletion: {} completed, {} failed; {} provider issues; metadata reset {metadata}",
                    summary.sessions_deleted,
                    summary.sessions_failed,
                    summary.provider_errors.len(),
                ),
            )
        }
        Err(_) => log.emit(
            &app,
            "error",
            "repair.project_sessions_delete_failed",
            "Project session deletion could not start.",
        ),
    }
    result
}

#[tauri::command]
pub async fn clear_storage_data(
    app: tauri::AppHandle,
    storage_id: StorageId,
    expected_config_revision: u64,
) -> Result<StorageDataClearResult, String> {
    let repository = repository()?;
    let label_repository = repository.clone();
    let label_storage_id = storage_id.clone();
    let storage_name = run_blocking(move || {
        label_repository
            .load_config()?
            .storages
            .iter()
            .find(|storage| storage.id == label_storage_id)
            .map(|storage| storage.name.clone())
            .ok_or_else(|| format!("unknown storage '{}'", label_storage_id))
    })
    .await?;
    let storage_for_work = storage_id.clone();
    let log =
        ActivityLogScope::new(ActivityLogType::Storage).storage(&storage_id, Some(&storage_name));
    log.emit(
        &app,
        "warning",
        "storage.data_clear_started",
        "Clearing all Mallard-managed data from storage…",
    );
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository)
            .clear_storage_data(&storage_for_work, expected_config_revision)
    })
    .await;
    match &result {
        Ok(summary) => log.emit(
            &app,
            "ok",
            "storage.data_clear_completed",
            &format!(
                "Cleared {} repository objects and {} Library objects",
                summary.repository_objects_deleted, summary.library_objects_deleted
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "storage.data_clear_failed",
            &format!("Storage data could not be cleared: {error}"),
        ),
    }
    result
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
    let label_repository = repository.clone();
    let label_storage_id = request.storage_id.clone();
    let storage =
        run_blocking(move || Ok(storage_log_label(&label_repository, &label_storage_id))).await?;
    let log = ActivityLogScope::new(ActivityLogType::Configuration)
        .project(&request.local_project_id, None)
        .storage(&request.storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "configuration.remote_connected_started",
        &format!(
            "Connecting project to bundle {} in {}…",
            request.bundle_id, storage
        ),
    );
    let result = run_blocking(move || {
        connect_project_to_remote_bundle_with_repository(&repository, request)
    })
    .await;
    match &result {
        Ok(detail) => log.emit(
            &app,
            "ok",
            "configuration.remote_connected",
            &format!(
                "Connected {} to remote bundle {}",
                detail.project.display_name, detail.project.bundle_id
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "configuration.remote_connect_failed",
            &format!("Failed to connect remote bundle: {}", error),
        ),
    }
    result
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
    request: super::maintenance::MaintenanceActionRequest,
) -> Result<super::maintenance::MaintenanceActionResult, String> {
    let repository = repository()?;
    let label = format!("{} {}", request.provider, request.action_id);
    let read_only = super::maintenance::MaintenanceRegistry::production()
        .descriptor(&request.provider, &request.action_id)
        .is_some_and(|descriptor| descriptor.read_only);
    let log = ActivityLogScope::new(ActivityLogType::Pull);
    if !read_only {
        log.emit(
            &app,
            "info",
            "maintenance.action_started",
            &format!("Running {label}…"),
        );
    }
    let result = run_blocking(move || {
        super::maintenance::MaintenanceRegistry::production().execute(&repository, &request)
    })
    .await;
    if !read_only {
        match &result {
            Ok(report) => log.emit(
                &app,
                if report.ready { "ok" } else { "warning" },
                "maintenance.action_completed",
                &report.summary,
            ),
            Err(error) => log.emit(
                &app,
                "error",
                "maintenance.action_failed",
                &format!("{label} failed: {error}"),
            ),
        }
    }
    result
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
    let context_repository = repository.clone();
    let context_project_id = local_project_id.clone();
    let context_storage_id = storage_id.clone();
    let (project, storage, _) = run_blocking(move || {
        Ok(project_storage_log_context(
            &context_repository,
            &context_project_id,
            &context_storage_id,
        ))
    })
    .await?;
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(&local_project_id, Some(&project))
        .storage(&storage_id, Some(&storage));
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository).finish_unfinished_push(&local_project_id, &storage_id)
    })
    .await;
    match &result {
        Ok(()) => log.emit(
            &app,
            "ok",
            "push.finished",
            &format!("↑  Finished a push to {storage} that did not finish"),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "push.finish_failed",
            &format!("↑  Finish that push to {storage} failed — {error}"),
        ),
    }
    result
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
    let context_repository = repository.clone();
    let context_project_id = local_project_id.clone();
    let context_storage_id = storage_id.clone();
    let (project, storage, bundle_id) = run_blocking(move || {
        Ok(project_storage_log_context(
            &context_repository,
            &context_project_id,
            &context_storage_id,
        ))
    })
    .await?;
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(&local_project_id, Some(&project))
        .storage(&storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "push.started",
        &format!("↑  Push started — {} → {}", project, storage),
    );
    let worker_app = app.clone();
    let worker_log = log.clone();
    let selected_count = recipe.entries.len();
    let mut upload_progress = bundle_id.map(|bundle_id| {
        FetchProgressEmitter::new(
            app.clone(),
            log.clone(),
            "push",
            storage_id.clone(),
            storage.clone(),
            bundle_id,
        )
    });
    let total_started = Instant::now();
    let result = run_blocking(move || {
        worker_log.emit(
            &worker_app,
            "info",
            "push.scan_started",
            &format!(
                "   Scanning {} resources selected for this storage…",
                selected_count
            ),
        );
        ProjectSyncService::new(&repository).push_reviewed_with_progress(
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
            &mut |update| {
                if let Some(emitter) = upload_progress.as_mut() {
                    emitter.observe(update);
                }
            },
            &mut |phase, elapsed| {
                worker_log.emit(
                    &worker_app,
                    "info",
                    "push.phase_timing",
                    &format!("   Push phase — {phase}: {:.3}s", elapsed.as_secs_f64()),
                );
            },
        )
    })
    .await;
    log.emit(
        &app,
        "info",
        "push.timing",
        &format!(
            "   Push total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
    match &result {
        Ok(operation) => {
            for resource in &operation.results {
                let (level, event) = match resource.state.as_str() {
                    "synced" => ("ok", "push.resource_synced"),
                    "failed" => ("error", "push.resource_failed"),
                    "held" => ("warning", "push.resource_held"),
                    _ => ("info", "push.resource_checked"),
                };
                let detail = resource
                    .message
                    .as_deref()
                    .map(|message| format!(" — {message}"))
                    .unwrap_or_default();
                log.emit(
                    &app,
                    level,
                    event,
                    &format!("↑  {} ({}){}", resource.resource_id, resource.state, detail),
                );
            }
            let (level, glyph) = if operation.success {
                ("ok", "✓")
            } else {
                ("warning", "⚠")
            };
            log.emit(
                &app,
                level,
                "push.completed",
                &format!("{glyph}  Push complete — {}", operation.message),
            );
        }
        Err(error) => log.emit(
            &app,
            "error",
            "push.failed",
            &format!("✗  Push failed: {}", error),
        ),
    }
    result
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
    let context_repository = repository.clone();
    let context_project_id = binding.local_project_id.clone();
    let context_storage_id = storage_id.clone();
    let (project, storage, _) = run_blocking(move || {
        Ok(project_storage_log_context(
            &context_repository,
            &context_project_id,
            &context_storage_id,
        ))
    })
    .await?;
    let log = ActivityLogScope::new(ActivityLogType::Pull)
        .project(&binding.local_project_id, Some(&project))
        .storage(&storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "pull.review_started",
        &format!("↓  Preparing Pull review — {} ← {}", project, storage),
    );
    let mut progress = FetchProgressEmitter::new(
        app.clone(),
        log.clone(),
        "plan",
        storage_id.clone(),
        storage.clone(),
        bundle_id.clone(),
    );
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository).plan_bundle_restore_with_progress(
            &storage_id,
            &bundle_id,
            &binding,
            &scope,
            &mut |update| progress.observe(update),
            &mut |phase, elapsed| {
                phase_log.emit(
                    &phase_app,
                    "info",
                    "pull.phase_timing",
                    &format!(
                        "   Pull review phase — {phase}: {:.3}s",
                        elapsed.as_secs_f64()
                    ),
                );
            },
        )
    })
    .await;
    log.emit(
        &app,
        "info",
        "pull.review_timing",
        &format!(
            "   Pull review total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
    match &result {
        Ok(plan) => {
            for resource in pull_review_resource_labels(plan) {
                log.emit(
                    &app,
                    "info",
                    "pull.resource_review",
                    &format!("↓  Review {resource}"),
                );
            }
            let (ordinary_actions, conversation_actions) = pull_plan_action_counts(plan);
            let applicable_actions = ordinary_actions.saturating_add(conversation_actions);
            log.emit(
                &app,
                "ok",
                "pull.review_ready",
                &format!(
                    "✓  Pull review ready — version {}, {} applicable actions ({} resources, {} conversations)",
                    plan.revision,
                    applicable_actions,
                    ordinary_actions,
                    conversation_actions
                ),
            );
            log.emit(
                &app,
                "info",
                "pull.awaiting_approval",
                "   Nothing has been applied yet — use the Apply button in the Pull review.",
            );
        }
        Err(error) => log.emit(
            &app,
            "error",
            "pull.comparison_failed",
            &format!("✗  Pull comparison failed: {}", error),
        ),
    }
    result.and_then(|plan| {
        PullReview::from_plan(
            plan,
            mallard_infrastructure::provider_capture::MAX_CAPTURE_BYTES,
        )
    })
}

#[tauri::command]
pub async fn apply_bundle_restore(
    app: tauri::AppHandle,
    review: PullReview,
    approved_action_ids: Vec<ActionId>,
) -> Result<RestoreResult, String> {
    let review = RestorePlan::from(review);
    let repository = repository()?;
    let storage_for_log = storage_log_label(&repository, &review.storage_id);
    let review_for_log = review.clone();
    let binding_repository = repository.clone();
    let log = ActivityLogScope::new(ActivityLogType::Pull);
    log.emit(
        &app,
        "info",
        "pull.apply_started",
        &format!(
            "↓  Applying {} approved Pull actions…",
            approved_action_ids.len()
        ),
    );
    let mut fetch_progress = Some(FetchProgressEmitter::new(
        app.clone(),
        log.clone()
            .storage(&review.storage_id, Some(&storage_for_log)),
        "apply",
        review.storage_id.clone(),
        storage_for_log,
        review.bundle_id.clone(),
    ));
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository).apply_bundle_restore_with_progress(
            &review,
            approved_action_ids,
            &mut |update| {
                if let Some(emitter) = fetch_progress.as_mut() {
                    emitter.observe(update);
                }
            },
            &mut |phase, elapsed| {
                phase_log.emit(
                    &phase_app,
                    "info",
                    "pull.phase_timing",
                    &format!(
                        "   Pull apply phase — {phase}: {:.3}s",
                        elapsed.as_secs_f64()
                    ),
                );
            },
        )
    })
    .await;
    log.emit(
        &app,
        "info",
        "pull.apply_timing",
        &format!(
            "   Pull apply total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
    match &result {
        Ok(operation) => {
            for action_id in &operation.applied_action_ids {
                let resource = pull_action_resource_label(&review_for_log, action_id)
                    .unwrap_or_else(|| action_id.to_string());
                log.emit(
                    &app,
                    "ok",
                    "pull.resource_applied",
                    &format!("↓  {}", resource),
                );
            }
            for failure in &operation.failed_actions {
                log.emit(
                    &app,
                    "error",
                    "pull.resource_failed",
                    &format!("✗  {}: {}", failure.action_id, failure.message),
                );
            }
            log.emit(
                &app,
                if operation.success { "ok" } else { "error" },
                if operation.success {
                    "pull.completed"
                } else {
                    "pull.completed_with_errors"
                },
                &format!(
                    "{}  Pull finished — {}",
                    if operation.success { "✓" } else { "✗" },
                    operation.message
                ),
            );
            if operation.success {
                let review_for_binding = review_for_log.clone();
                let binding = run_blocking(move || {
                    current_binding_for_restore_plan(&binding_repository, &review_for_binding).map(
                        |binding| {
                            // Guidance is informational: an unreadable settings
                            // file hides a saved CLI path, never the lines.
                            let settings = mallard_infrastructure::app_settings::load_app_settings(
                                &binding_repository,
                            )
                            .unwrap_or_default();
                            (binding, settings)
                        },
                    )
                })
                .await;
                match binding {
                    Ok((binding, settings)) => {
                        log.emit(
                            &app,
                            "info",
                            "pull.open_project_hint",
                            "   Open this project after Pull:",
                        );
                        for (label, command) in project_open_commands(&binding, &settings) {
                            log.emit(
                                &app,
                                "info",
                                "pull.open_project_command",
                                &format!("   {}: {}", label, command),
                            );
                        }
                    }
                    Err(error) => log.emit(
                        &app,
                        "info",
                        "pull.open_project_unavailable",
                        &format!("   Open command unavailable: {}", error),
                    ),
                }
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "pull.failed",
            &format!("✗  Pull failed: {}", error),
        ),
    }
    result
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
    let context_repository = repository.clone();
    let context_project_id = local_project_id.clone();
    let context_storage_id = storage_id.clone();
    let (project, storage, _) = run_blocking(move || {
        Ok(project_storage_log_context(
            &context_repository,
            &context_project_id,
            &context_storage_id,
        ))
    })
    .await?;
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(&local_project_id, Some(&project))
        .storage(&storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "push.review_started",
        &format!("↑  Preparing Push review — {} → {}", project, storage),
    );
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository).prepare_push_review_with_phases(
            &local_project_id,
            &storage_id,
            scope,
            &mut |phase, elapsed| {
                phase_log.emit(
                    &phase_app,
                    "info",
                    "push.phase_timing",
                    &format!(
                        "   Push review phase — {phase}: {:.3}s",
                        elapsed.as_secs_f64()
                    ),
                );
            },
        )
    })
    .await;
    log.emit(
        &app,
        "info",
        "push.review_timing",
        &format!(
            "   Push review total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
    match &result {
        Ok(context) => {
            let threads = context
                .thread_comparison
                .as_ref()
                .map(|comparison| comparison.entries.len())
                .unwrap_or(0);
            let selectable = context
                .thread_comparison
                .as_ref()
                .map(|comparison| {
                    comparison
                        .entries
                        .iter()
                        .filter(|entry| entry.push_decision.selectable)
                        .count()
                })
                .unwrap_or(0);
            log.emit(
                &app,
                "ok",
                "push.review_ready",
                &format!(
                    "✓  Push review ready — {} threads compared, {} pushable conversations",
                    threads, selectable
                ),
            );
            // A review that opens without its comparison is a review with
            // nothing selectable; the reason must reach the log, not only
            // the context the frontend may or may not render.
            if let Some(error) = &context.thread_comparison_error {
                log.emit(
                    &app,
                    "warning",
                    "push.comparison_failed",
                    &format!("⚠  Conversation comparison failed: {}", error),
                );
            }
            if let Some(error) = &context.project_files_error {
                log.emit(
                    &app,
                    "warning",
                    "push.project_files_failed",
                    &format!("⚠  Project files inventory failed: {}", error),
                );
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "push.review_failed",
            &format!("✗  Push review failed: {}", error),
        ),
    }
    result
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
    let log = ActivityLogScope::new(ActivityLogType::Configuration);
    log.emit(
        &app,
        "info",
        "configuration.project_setup_started",
        "Finalizing project setup…",
    );
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository).finalize_project_setup(&draft_id, expected_revision)
    })
    .await;
    match &result {
        Ok(detail) => log.emit(
            &app,
            "ok",
            "configuration.project_setup_completed",
            &format!("Project {} is set up", detail.project.display_name),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "configuration.project_setup_failed",
            &format!("Project setup failed: {}", error),
        ),
    }
    result
}
