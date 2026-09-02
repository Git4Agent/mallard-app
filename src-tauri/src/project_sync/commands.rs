//! Tauri command adapters for project sync.

use super::service::*;
use crate::activity_log::{emit_typed_log, ActivityLogScope};
use mallard_domain::{
    now_secs, ActionId, BundleId, BundleRecipe, LocalProjectId, LocalProjectRegistration, PlanId,
    ProjectBinding, ProjectSetupDraft, ProjectStorageLink, ProviderKey, ProviderProfile,
    ResourceId, RestorePlan, SetupDraftId, StorageConfig, StorageId, StorageSetupDraftId,
    SyncConfig,
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
pub async fn get_project_sync_config() -> Result<SyncConfig, String> {
    let repository = repository()?;
    run_blocking(move || load_project_sync_config_with_repository(&repository)).await
}

#[tauri::command]
pub async fn upsert_r2_storage(
    app: tauri::AppHandle,
    request: UpsertR2StorageRequest,
) -> Result<UpsertR2StorageResult, String> {
    let repository = repository()?;
    let runtime = tokio::runtime::Handle::current();
    let existing_storage_id = request.storage_id.clone();
    let storage_name = request.name.clone();
    let result =
        run_blocking(move || upsert_r2_storage_with_repository(&repository, runtime, request))
            .await;

    match &result {
        Ok(outcome) => {
            let (event, message) =
                r2_upsert_success_summary(existing_storage_id.is_some(), &storage_name);
            ActivityLogScope::new(ActivityLogType::Storage)
                .storage(&outcome.storage_id, Some(&storage_name))
                .emit(&app, "ok", event, &message);
        }
        Err(_) => {
            let log = match existing_storage_id.as_ref() {
                Some(storage_id) => ActivityLogScope::new(ActivityLogType::Storage)
                    .storage(storage_id, Some(&storage_name)),
                None => ActivityLogScope::new(ActivityLogType::Storage),
            };
            // Keep the activity record fixed and credential-free even if the
            // worker itself failed rather than returning a typed setup error.
            log.emit(
                &app,
                "error",
                "storage.settings_update_failed",
                "R2 storage settings could not be saved.",
            );
        }
    }

    result
}

#[tauri::command]
pub async fn test_r2_storage(
    request: UpsertR2StorageRequest,
) -> Result<TestR2StorageResult, String> {
    let repository = repository()?;
    let runtime = tokio::runtime::Handle::current();
    run_blocking(move || test_r2_storage_with_repository(&repository, runtime, request)).await
}

#[tauri::command]
pub async fn list_storage_setup_drafts() -> Result<StorageSetupDraftList, String> {
    let repository = repository()?;
    run_blocking(move || list_storage_setup_drafts_with_repository(&repository)).await
}

#[tauri::command]
pub async fn create_storage_setup_draft(
    storage: StorageSetupDraftPayload,
) -> Result<StorageSetupDraftView, String> {
    let repository = repository()?;
    run_blocking(move || {
        create_storage_setup_draft_with_repository(&repository, storage)
            .map(redact_storage_setup_draft)
    })
    .await
}

#[tauri::command]
pub async fn get_storage_setup_draft(
    draft_id: StorageSetupDraftId,
) -> Result<Option<StorageSetupDraftView>, String> {
    let repository = repository()?;
    run_blocking(move || get_storage_setup_draft_with_repository(&repository, &draft_id)).await
}

#[tauri::command]
pub async fn update_storage_setup_draft(
    request: UpdateStorageSetupDraftRequest,
) -> Result<StorageSetupDraftView, String> {
    let repository = repository()?;
    run_blocking(move || {
        update_storage_setup_draft_with_repository(&repository, request)
            .map(redact_storage_setup_draft)
    })
    .await
}

#[tauri::command]
pub async fn discard_storage_setup_draft(draft_id: StorageSetupDraftId) -> Result<bool, String> {
    let repository = repository()?;
    run_blocking(move || discard_storage_setup_draft_with_repository(&repository, &draft_id)).await
}

#[tauri::command]
pub fn get_mallard_cli_path() -> Result<String, String> {
    let current = std::env::current_exe()
        .map_err(|_| "Mallard cannot resolve its packaged CLI location.".to_string())?;
    packaged_cli_path(&current)?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "Mallard cannot represent its packaged CLI path.".to_string())
}

#[tauri::command]
pub fn launch_r2_agent_setup(provider: String, instruction: String) -> Result<(), String> {
    let provider = ProviderKey::parse(provider)?;
    let descriptor = provider_descriptor(&provider)?;
    let launcher = descriptor
        .cli_launcher
        .ok_or_else(|| format!("{} has no launchable agent CLI.", descriptor.display_name))?;
    let script = agent_launch_script(&launcher.program, &launcher.display_name, &instruction);
    open_agent_launch_script_in_terminal(&script)
}

#[tauri::command]
pub async fn get_host_identity_status() -> Result<HostIdentityStatus, String> {
    let repository = repository()?;
    run_blocking(move || get_host_identity_status_with_repository(&repository)).await
}

#[tauri::command]
pub async fn save_project_sync_config(
    app: tauri::AppHandle,
    config: SyncConfig,
) -> Result<SyncConfig, String> {
    let repository = repository()?;
    let (storage_change, result) = run_blocking(move || {
        let storage_change = project_sync_config_storage_change(&repository, &config);
        let result = save_project_sync_config_with_repository(&repository, config);
        Ok((storage_change, result))
    })
    .await?;
    if let Some((event, message, storage_id, storage_name)) = storage_change {
        let log = ActivityLogScope::new(ActivityLogType::Storage)
            .storage(&storage_id, Some(&storage_name));
        match &result {
            Ok(_) => log.emit(&app, "ok", event, &message),
            Err(error) => log.emit(
                &app,
                "error",
                "storage.settings_update_failed",
                &format!("Storage settings could not be saved: {error}"),
            ),
        }
    }
    result.map(redact_storage_secrets)
}

#[tauri::command]
pub async fn list_local_projects() -> Result<Vec<LocalProjectRegistration>, String> {
    let repository = repository()?;
    run_blocking(move || list_local_projects_with_repository(&repository)).await
}

#[tauri::command]
pub async fn recover_startup_state(app: tauri::AppHandle) -> Result<(), String> {
    let repository = repository()?;
    // Both passes are blocking filesystem work, so they belong on the blocking
    // pool rather than a runtime worker.  Logging stays out here: `run_blocking`
    // turns a panic in the worker into a command error, whereas panicking
    // inline would leave the caller's promise unresolved forever.
    let (warnings, recovery) =
        run_blocking(move || Ok(recover_startup_state_with_repository(&repository))).await?;
    for warning in warnings {
        emit_typed_log(&app, ActivityLogType::Configuration, "warning", &warning);
    }
    match recovery {
        Ok(report) if !report.recovery_required.is_empty() => emit_typed_log(
            &app,
            ActivityLogType::Repair,
            "warning",
            "One or more conversation sync operations need manual recovery.",
        ),
        Ok(_) => {}
        Err(error) => emit_typed_log(
            &app,
            ActivityLogType::Repair,
            "warning",
            &format!("Conversation recovery is unavailable: {error}"),
        ),
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
pub async fn get_project_thread_sync_comparison(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<ThreadSyncComparison, String> {
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
        get_project_thread_sync_comparison_with_progress(
            &repository,
            &local_project_id,
            &storage_id,
            &mut |phase, elapsed| {
                if log_routine_compare {
                    phase_log.emit(
                        &phase_app,
                        "info",
                        "history.compare_phase_timing",
                        &format!(
                            "   Storage comparison phase — {phase}: {:.3}s",
                            elapsed.as_secs_f64()
                        ),
                    );
                }
            },
        )
    })
    .await;
    match &result {
        Ok(comparison) => {
            if log_routine_compare {
                log.emit(
                    &app,
                    "info",
                    "history.compare_completed",
                    &format!(
                        "Storage comparison complete: {} threads in {:.3}s",
                        comparison.entries.len(),
                        total_started.elapsed().as_secs_f64()
                    ),
                );
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "history.compare_failed",
            &format!("Storage comparison failed: {error}"),
        ),
    }
    result
}

#[tauri::command]
pub async fn get_project_session_detail(
    local_project_id: LocalProjectId,
    resource_id: String,
) -> Result<tauri::ipc::Response, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    let encoded = run_session_detail_blocking(move || {
        let detail = get_project_session_detail_with_repository(
            &repository,
            &local_project_id,
            &resource_id,
        )?;
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
) -> Result<tauri::ipc::Response, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    let encoded = run_session_detail_blocking(move || {
        let page = get_project_session_timeline_page_with_repository(
            &repository,
            &local_project_id,
            &resource_id,
            ProjectSessionTimelineRequest {
                snapshot_id,
                cursor,
                limit,
                included_event_kinds: included_event_kinds.into_iter().collect(),
                included_tool_categories: included_tool_categories.into_iter().collect(),
                turn_id,
            },
        )?;
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
) -> Result<tauri::ipc::Response, SessionDetailError> {
    let repository = repository().map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not open its local state repository.",
        )
    })?;
    let encoded = run_session_detail_blocking(move || {
        let detail = get_project_session_event_detail_with_repository(
            &repository,
            &local_project_id,
            &resource_id,
            &snapshot_id,
            &event_id,
        )?;
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

#[tauri::command]
pub async fn remove_local_project(local_project_id: LocalProjectId) -> Result<bool, String> {
    let repository = repository()?;
    run_blocking(move || remove_local_project_with_repository(&repository, &local_project_id)).await
}

#[tauri::command]
pub async fn reset_project_sync_metadata(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    expected_project_revision: u64,
) -> Result<ProjectSyncMetadataResetResult, String> {
    let repository = repository()?;
    let project_for_work = local_project_id.clone();
    let log = ActivityLogScope::new(ActivityLogType::Repair).project(&local_project_id, None);
    log.emit(
        &app,
        "warning",
        "repair.project_sync_metadata_reset_started",
        "Resetting this project's local sync metadata…",
    );
    let result = run_blocking(move || {
        reset_project_sync_metadata_with_repository(
            &repository,
            &project_for_work,
            expected_project_revision,
        )
    })
    .await;
    match &result {
        Ok(summary) => log.emit(
            &app,
            "ok",
            "repair.project_sync_metadata_reset_completed",
            &format!(
                "Reset local sync metadata: {} conversation mappings, {} checkpoints, and {} saved results removed",
                summary.conversation_bindings_removed,
                summary.conversation_checkpoints_removed,
                summary.materializations_removed,
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "repair.project_sync_metadata_reset_failed",
            &format!("Local sync metadata reset failed: {error}"),
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
pub async fn audit_codex_conversation_paths(
    local_project_id: LocalProjectId,
) -> Result<CodexConversationPathAudit, String> {
    let repository = repository()?;
    run_blocking(move || {
        audit_codex_conversation_paths_with_repository(&repository, &local_project_id)
    })
    .await
}

#[tauri::command]
pub async fn repair_codex_conversation_paths(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<CodexConversationPathRepairResult, String> {
    let repository = repository()?;
    let log = ActivityLogScope::new(ActivityLogType::Repair).project(&local_project_id, None);
    log.emit(
        &app,
        "info",
        "repair.conversation_paths_started",
        "Repairing Codex conversation paths…",
    );
    let result = run_blocking(move || {
        repair_codex_conversation_paths_with_repository(&repository, &local_project_id)
    })
    .await;
    match &result {
        Ok(report) => log.emit(
            &app,
            "ok",
            "repair.conversation_paths_completed",
            &format!(
                "Repaired {} conversation path{}",
                report.repaired_thread_ids.len(),
                if report.repaired_thread_ids.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "repair.conversation_paths_failed",
            &format!("Conversation path repair failed: {error}"),
        ),
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

#[tauri::command]
// The argument list is the frozen Tauri command contract.
#[allow(clippy::too_many_arguments)]
pub async fn push_bundle(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    recipe: BundleRecipe,
    project_content_review_token: Option<String>,
    project_content_removal_ids: Option<Vec<ResourceId>>,
    acknowledged_warning_digests: Option<Vec<String>>,
    conversation_push_plan_id: Option<PlanId>,
    approved_conversation_action_ids: Option<Vec<ActionId>>,
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
                recipe,
                project_content_review_token,
                project_content_removal_ids: project_content_removal_ids.unwrap_or_default(),
                acknowledged_warning_digests: acknowledged_warning_digests.unwrap_or_default(),
                conversation_push_plan_id,
                approved_conversation_action_ids: approved_conversation_action_ids
                    .unwrap_or_default(),
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
                log.emit(
                    &app,
                    if resource.state == "synced" {
                        "ok"
                    } else {
                        "info"
                    },
                    if resource.state == "synced" {
                        "push.resource_synced"
                    } else {
                        "push.resource_checked"
                    },
                    &format!("↑  {} ({})", resource.resource_id, resource.state),
                );
            }
            log.emit(
                &app,
                "ok",
                "push.completed",
                &format!("✓  Push complete — {}", operation.message),
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
) -> Result<RestorePlan, String> {
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
                "pull.plan_ready",
                &format!(
                    "✓  Pull plan ready — revision {}, {} applicable actions ({} resources, {} conversations)",
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
            "pull.planning_failed",
            &format!("✗  Pull planning failed: {}", error),
        ),
    }
    result
}

#[tauri::command]
pub async fn apply_bundle_restore(
    app: tauri::AppHandle,
    plan_id: PlanId,
    approved_action_ids: Vec<ActionId>,
) -> Result<RestoreResult, String> {
    let repository = repository()?;
    let log_context_repository = repository.clone();
    let log_plan_id = plan_id.clone();
    let (plan_for_log, storage_for_log) = run_blocking(move || {
        let plan = log_context_repository.load_restore_plan(&log_plan_id).ok();
        let storage = plan
            .as_ref()
            .map(|plan| storage_log_label(&log_context_repository, &plan.storage_id));
        Ok((plan, storage))
    })
    .await?;
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
    let mut fetch_progress = plan_for_log.as_ref().map(|plan| {
        let storage = storage_for_log
            .clone()
            .unwrap_or_else(|| plan.storage_id.to_string());
        FetchProgressEmitter::new(
            app.clone(),
            log.clone().storage(&plan.storage_id, Some(&storage)),
            "apply",
            plan.storage_id.clone(),
            storage,
            plan.bundle_id.clone(),
        )
    });
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository).apply_bundle_restore_with_progress(
            &plan_id,
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
                let resource = plan_for_log
                    .as_ref()
                    .and_then(|plan| pull_action_resource_label(plan, action_id))
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
                let plan_for_binding = plan_for_log.clone();
                let binding = run_blocking(move || {
                    let plan = plan_for_binding
                        .as_ref()
                        .ok_or_else(|| "restore plan is unavailable".to_string())?;
                    current_binding_for_restore_plan(&binding_repository, plan)
                })
                .await;
                match binding {
                    Ok(binding) => {
                        log.emit(
                            &app,
                            "info",
                            "pull.open_project_hint",
                            "   Open this project after Pull:",
                        );
                        for (label, command) in project_open_commands(&binding) {
                            log.emit(
                                &app,
                                "info",
                                "pull.open_project_command",
                                &format!("   {}: {}", label, command),
                            );
                        }
                        // Resuming a thread in the app takes two sequenced deep
                        // links, so it has no pasteable form; point at the
                        // button that performs them instead.
                        #[cfg(target_os = "macos")]
                        if binding.homes.contains_key(&codex_key()) {
                            log.emit(
                                &app,
                                "info",
                                "pull.open_project_command",
                                "   Codex app — resume: use Open in Codex (one deep link cannot carry both project and chat)",
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

#[tauri::command]
pub async fn restart_codex_app_after_pull(
    request: RestartCodexAppAfterPullRequest,
) -> Result<CodexAppRestartResult, String> {
    let repository = repository()?;
    run_blocking(move || restart_codex_app_after_pull_with_repository(&repository, request)).await
}

#[tauri::command]
pub async fn get_restore_readiness(restore_plan_id: PlanId) -> Result<BundleReadiness, String> {
    let repository = repository()?;
    run_blocking(move || get_restore_readiness_with_repository(&repository, &restore_plan_id)).await
}

#[tauri::command]
pub async fn list_pending_storage_bundle_snapshots(
    storage: StorageConfig,
) -> Result<Vec<BundleSnapshotSummary>, String> {
    run_blocking(move || list_pending_storage_bundle_snapshots_with_config(&storage)).await
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
            log.emit(
                &app,
                "ok",
                "push.review_ready",
                &format!(
                    "✓  Push review ready — {} threads compared, {} pushable conversations",
                    threads,
                    context.conversation_action_ids.len()
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
