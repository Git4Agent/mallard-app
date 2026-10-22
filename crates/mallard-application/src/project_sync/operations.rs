//! The reviewed operations with their activity-log narration, for every
//! adapter. The desktop runs each one on a blocking thread and forwards its
//! entries to the window; the command line runs it on its own thread and
//! prints them. Event names, messages, and their order are the product's
//! activity log and are the same whichever adapter ran the operation.
//!
//! Every function here is synchronous and may block on storage; call it off
//! any interactive or async-executor thread.

use std::time::Instant;

use mallard_domain::{
    now_secs, ActionId, BundleId, LocalProjectId, ProjectBinding, PullReview, PullScope,
    RestorePlan, SetupDraftId, StorageId, StorageSetupDraftId,
};
use mallard_infrastructure::activity_log::ActivityLogType;
use mallard_infrastructure::bundle_engine::FetchProgress;
use mallard_infrastructure::persistence::LocalStateRepository;
use tokio::runtime::Handle;

use super::maintenance::{MaintenanceActionRequest, MaintenanceActionResult, MaintenanceRegistry};
use super::service::{
    claim_history_scan_log_at, commit_storage_setup_draft_with_repository,
    connect_project_to_remote_bundle_with_repository, current_binding_for_restore_plan,
    delete_local_project_sessions_with_repository, execute_project_session_action_with_repository,
    format_fetch_started_message, format_mb, get_project_activity_with_repository,
    get_storage_setup_draft_with_repository, project_open_commands, project_storage_log_context,
    pull_action_resource_label, pull_plan_action_counts, pull_review_resource_labels,
    refresh_project_activity_with_progress, remove_storage_with_repository,
    rename_storage_with_repository, should_log_history_scan, storage_log_label,
    CommitStorageDraftResult, ConnectProjectBundleRequest, ProjectActivity,
    ProjectActivityRefreshEvent, ProjectDetail, ProjectOperationResult,
    ProjectSessionsDeleteResult, ProjectSyncConfigView, ProjectSyncService, PushApproval,
    PushReviewContext, PushScope, RestoreResult, StorageDataClearResult, HISTORY_SCAN_LOGGED_AT,
};
use crate::activity_log::ActivityLogScope;
use crate::events::{FetchProgressEvent, SyncEventSink};

/// Narrates one bundle transfer: the start line, coalesced progress events
/// (at most one every 150 ms, and always the last), and the completion line
/// with its rate.
pub struct FetchProgressEmitter<'a> {
    sink: &'a dyn SyncEventSink,
    log: ActivityLogScope,
    phase: &'static str,
    storage_id: StorageId,
    storage_name: String,
    bundle_id: BundleId,
    started: Option<Instant>,
    last_event: Option<Instant>,
}

impl<'a> FetchProgressEmitter<'a> {
    pub fn new(
        sink: &'a dyn SyncEventSink,
        log: ActivityLogScope,
        phase: &'static str,
        storage_id: StorageId,
        storage_name: String,
        bundle_id: BundleId,
    ) -> Self {
        Self {
            sink,
            log,
            phase,
            storage_id,
            storage_name,
            bundle_id,
            started: None,
            last_event: None,
        }
    }

    pub fn observe(&mut self, progress: FetchProgress) {
        let now = Instant::now();
        let upload = self.phase == "push";
        if self.started.is_none() {
            self.started = Some(now);
            self.log.emit(
                self.sink,
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
            self.sink.fetch_progress(&FetchProgressEvent {
                phase: self.phase,
                storage_id: self.storage_id.to_string(),
                bundle_id: self.bundle_id.to_string(),
                index: progress.index,
                total_objects: progress.total_objects,
                fetched_bytes: progress.fetched_bytes,
                total_bytes: progress.total_bytes,
            });
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
                self.sink,
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

pub fn commit_storage_setup_draft(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    runtime: Handle,
    draft_id: &StorageSetupDraftId,
    expected_draft_revision: u64,
) -> Result<CommitStorageDraftResult, String> {
    let storage_name = get_storage_setup_draft_with_repository(repository, draft_id)?
        .map(|draft| draft.name)
        .unwrap_or_default();
    let result = commit_storage_setup_draft_with_repository(
        repository,
        runtime,
        draft_id,
        expected_draft_revision,
    );
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
                .emit(sink, "ok", event, &message);
        }
        Err(_) => {
            // Keep the activity record fixed and credential-free even if the
            // worker itself failed rather than returning a typed setup error.
            ActivityLogScope::new(ActivityLogType::Storage).emit(
                sink,
                "error",
                "storage.settings_update_failed",
                "Storage settings could not be saved.",
            );
        }
    }
    result
}

pub fn rename_storage(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    storage_id: &StorageId,
    name: &str,
    expected_config_revision: u64,
) -> Result<ProjectSyncConfigView, String> {
    let result =
        rename_storage_with_repository(repository, storage_id, name, expected_config_revision);
    let log = ActivityLogScope::new(ActivityLogType::Storage).storage(storage_id, Some(name));
    match &result {
        Ok(_) => log.emit(
            sink,
            "ok",
            "storage.settings_updated",
            &format!("Storage settings updated — {name}"),
        ),
        Err(error) => log.emit(
            sink,
            "error",
            "storage.settings_update_failed",
            &format!("Storage settings could not be saved: {error}"),
        ),
    }
    result
}

pub fn remove_storage(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    storage_id: &StorageId,
    expected_config_revision: u64,
) -> Result<ProjectSyncConfigView, String> {
    let storage_name = storage_log_label(repository, storage_id);
    let result = remove_storage_with_repository(repository, storage_id, expected_config_revision);
    let log =
        ActivityLogScope::new(ActivityLogType::Storage).storage(storage_id, Some(&storage_name));
    match &result {
        Ok(_) => log.emit(
            sink,
            "ok",
            "storage.removed",
            &format!("Storage removed — {storage_name}"),
        ),
        Err(error) => log.emit(
            sink,
            "error",
            "storage.settings_update_failed",
            &format!("Storage could not be removed: {error}"),
        ),
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub fn get_project_activity(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    local_project_id: &LocalProjectId,
    storage_id: Option<&StorageId>,
    branch: Option<&str>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    review: bool,
) -> Result<ProjectActivity, String> {
    let log = ActivityLogScope::new(ActivityLogType::History).project(local_project_id, None);
    let log_routine_scan = should_log_history_scan(local_project_id);
    if log_routine_scan {
        log.emit(
            sink,
            "info",
            "history.scan_started",
            &format!("Scanning agent session activity for project {local_project_id}…"),
        );
    }
    let result = get_project_activity_with_repository(
        repository,
        local_project_id,
        storage_id,
        branch,
        before_time,
        window_days,
        review,
    );
    match &result {
        Ok(activity) => {
            for warning in &activity.warnings {
                log.emit(sink, "warning", "history.scan_warning", warning);
            }
            if log_routine_scan {
                log.emit(
                    sink,
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
            sink,
            "error",
            "history.scan_failed",
            &format!("Session activity scan failed: {error}"),
        ),
    }
    result
}

/// Stages reach `on_event` as they complete; local rows remain usable if the
/// storage or the comparison fails.
#[allow(clippy::too_many_arguments)]
pub fn refresh_project_activity(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    branch: Option<&str>,
    window_days: Option<u64>,
    review: bool,
    on_event: &mut dyn FnMut(ProjectActivityRefreshEvent) -> Result<(), String>,
) -> Result<(), String> {
    let log = ActivityLogScope::new(ActivityLogType::History).project(local_project_id, None);
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
    let total_started = Instant::now();
    let result = refresh_project_activity_with_progress(
        repository,
        local_project_id,
        storage_id,
        branch,
        window_days,
        review,
        &mut |event| {
            match &event {
                ProjectActivityRefreshEvent::ActivityError { message, .. }
                | ProjectActivityRefreshEvent::ComparisonError { message, .. } => {
                    log.emit(sink, "warning", "history.refresh_warning", message)
                }
                _ => {}
            }
            on_event(event)
        },
        &mut |phase, elapsed| {
            if log_routine_compare {
                log.emit(
                    sink,
                    "info",
                    "history.compare_phase_timing",
                    &format!(
                        "Storage comparison phase — {phase}: {:.3}s",
                        elapsed.as_secs_f64()
                    ),
                );
            }
        },
    );
    if let Err(error) = &result {
        log.emit(sink, "error", "history.refresh_failed", error);
    } else if log_routine_compare {
        log.emit(
            sink,
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

/// Run one of a session's actions; the line that runs goes to the activity log.
pub fn execute_project_session_action(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    action_id: &str,
) -> Result<(), String> {
    let log = ActivityLogScope::new(ActivityLogType::History).project(local_project_id, None);
    execute_project_session_action_with_repository(
        repository,
        local_project_id,
        resource_id,
        action_id,
        &mut |command| {
            log.emit(
                sink,
                "info",
                "history.session_action",
                &format!("$ {command}"),
            )
        },
    )
}

pub fn delete_local_project_sessions(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    local_project_id: &LocalProjectId,
    expected_project_revision: u64,
) -> Result<ProjectSessionsDeleteResult, String> {
    let log = ActivityLogScope::new(ActivityLogType::Repair).project(local_project_id, None);
    log.emit(
        sink,
        "warning",
        "repair.project_sessions_delete_started",
        "Deleting this project's local sessions…",
    );
    let result = delete_local_project_sessions_with_repository(
        repository,
        local_project_id,
        expected_project_revision,
    );
    match &result {
        Ok(summary) => {
            let metadata = if summary.metadata_reset.is_some() {
                "completed"
            } else if summary.metadata_error.is_none() {
                "skipped; existing metadata kept"
            } else {
                "failed"
            };
            log.emit(
                sink,
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
            sink,
            "error",
            "repair.project_sessions_delete_failed",
            "Project session deletion could not start.",
        ),
    }
    result
}

pub fn clear_storage_data(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    storage_id: &StorageId,
    expected_config_revision: u64,
) -> Result<StorageDataClearResult, String> {
    let storage_name = repository
        .load_config()?
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .map(|storage| storage.name.clone())
        .ok_or_else(|| format!("unknown storage '{}'", storage_id))?;
    let log =
        ActivityLogScope::new(ActivityLogType::Storage).storage(storage_id, Some(&storage_name));
    log.emit(
        sink,
        "warning",
        "storage.data_clear_started",
        "Clearing all Mallard-managed data from storage…",
    );
    let result = ProjectSyncService::new(repository)
        .clear_storage_data(storage_id, expected_config_revision);
    match &result {
        Ok(summary) => log.emit(
            sink,
            "ok",
            "storage.data_clear_completed",
            &format!(
                "Cleared {} repository objects and {} Library objects",
                summary.repository_objects_deleted, summary.library_objects_deleted
            ),
        ),
        Err(error) => log.emit(
            sink,
            "error",
            "storage.data_clear_failed",
            &format!("Storage data could not be cleared: {error}"),
        ),
    }
    result
}

pub fn connect_project_to_remote_bundle(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    request: ConnectProjectBundleRequest,
) -> Result<ProjectDetail, String> {
    let storage = storage_log_label(repository, &request.storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Configuration)
        .project(&request.local_project_id, None)
        .storage(&request.storage_id, Some(&storage));
    log.emit(
        sink,
        "info",
        "configuration.remote_connected_started",
        &format!(
            "Connecting project to bundle {} in {}…",
            request.bundle_id, storage
        ),
    );
    let result = connect_project_to_remote_bundle_with_repository(repository, request);
    match &result {
        Ok(detail) => log.emit(
            sink,
            "ok",
            "configuration.remote_connected",
            &format!(
                "Connected {} to remote bundle {}",
                detail.project.display_name, detail.project.bundle_id
            ),
        ),
        Err(error) => log.emit(
            sink,
            "error",
            "configuration.remote_connect_failed",
            &format!("Failed to connect remote bundle: {}", error),
        ),
    }
    result
}

pub fn execute_provider_maintenance_action(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    request: MaintenanceActionRequest,
) -> Result<MaintenanceActionResult, String> {
    let label = format!("{} {}", request.provider, request.action_id);
    let read_only = MaintenanceRegistry::production()
        .descriptor(&request.provider, &request.action_id)
        .is_some_and(|descriptor| descriptor.read_only);
    let log = ActivityLogScope::new(ActivityLogType::Pull);
    if !read_only {
        log.emit(
            sink,
            "info",
            "maintenance.action_started",
            &format!("Running {label}…"),
        );
    }
    let result = MaintenanceRegistry::production().execute(repository, &request);
    if !read_only {
        match &result {
            Ok(report) => log.emit(
                sink,
                if report.ready { "ok" } else { "warning" },
                "maintenance.action_completed",
                &report.summary,
            ),
            Err(error) => log.emit(
                sink,
                "error",
                "maintenance.action_failed",
                &format!("{label} failed: {error}"),
            ),
        }
    }
    result
}

/// *Finish that push*: write the record that the claim of the selected
/// version holds, once, and report. The user's verb, from the Push review.
pub fn finish_unfinished_push(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<(), String> {
    let (project, storage, _) =
        project_storage_log_context(repository, local_project_id, storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(local_project_id, Some(&project))
        .storage(storage_id, Some(&storage));
    let result =
        ProjectSyncService::new(repository).finish_unfinished_push(local_project_id, storage_id);
    match &result {
        Ok(()) => log.emit(
            sink,
            "ok",
            "push.finished",
            &format!("↑  Finished a push to {storage} that did not finish"),
        ),
        Err(error) => log.emit(
            sink,
            "error",
            "push.finish_failed",
            &format!("↑  Finish that push to {storage} failed — {error}"),
        ),
    }
    result
}

pub fn push_bundle(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    approval: PushApproval,
) -> Result<ProjectOperationResult, String> {
    let (project, storage, bundle_id) =
        project_storage_log_context(repository, local_project_id, storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(local_project_id, Some(&project))
        .storage(storage_id, Some(&storage));
    log.emit(
        sink,
        "info",
        "push.started",
        &format!("↑  Push started — {} → {}", project, storage),
    );
    let selected_count = approval.recipe.entries.len();
    let mut upload_progress = bundle_id.map(|bundle_id| {
        FetchProgressEmitter::new(
            sink,
            log.clone(),
            "push",
            storage_id.clone(),
            storage.clone(),
            bundle_id,
        )
    });
    let total_started = Instant::now();
    log.emit(
        sink,
        "info",
        "push.scan_started",
        &format!(
            "   Scanning {} resources selected for this storage…",
            selected_count
        ),
    );
    let result = ProjectSyncService::new(repository).push_reviewed_with_progress(
        local_project_id,
        storage_id,
        approval,
        &mut |update| {
            if let Some(emitter) = upload_progress.as_mut() {
                emitter.observe(update);
            }
        },
        &mut |phase, elapsed| {
            log.emit(
                sink,
                "info",
                "push.phase_timing",
                &format!("   Push phase — {phase}: {:.3}s", elapsed.as_secs_f64()),
            );
        },
    );
    log.emit(
        sink,
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
                    sink,
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
                sink,
                level,
                "push.completed",
                &format!("{glyph}  Push complete — {}", operation.message),
            );
        }
        Err(error) => log.emit(
            sink,
            "error",
            "push.failed",
            &format!("✗  Push failed: {}", error),
        ),
    }
    result
}

/// Compare the storage's version with this computer and build the Pull
/// review. Nothing is applied.
pub fn plan_bundle_restore(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
    scope: &PullScope,
) -> Result<PullReview, String> {
    let (project, storage, _) =
        project_storage_log_context(repository, &binding.local_project_id, storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Pull)
        .project(&binding.local_project_id, Some(&project))
        .storage(storage_id, Some(&storage));
    log.emit(
        sink,
        "info",
        "pull.review_started",
        &format!("↓  Preparing Pull review — {} ← {}", project, storage),
    );
    let mut progress = FetchProgressEmitter::new(
        sink,
        log.clone(),
        "plan",
        storage_id.clone(),
        storage.clone(),
        bundle_id.clone(),
    );
    let total_started = Instant::now();
    let result = ProjectSyncService::new(repository).plan_bundle_restore_with_progress(
        storage_id,
        bundle_id,
        binding,
        scope,
        &mut |update| progress.observe(update),
        &mut |phase, elapsed| {
            log.emit(
                sink,
                "info",
                "pull.phase_timing",
                &format!(
                    "   Pull review phase — {phase}: {:.3}s",
                    elapsed.as_secs_f64()
                ),
            );
        },
    );
    log.emit(
        sink,
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
                    sink,
                    "info",
                    "pull.resource_review",
                    &format!("↓  Review {resource}"),
                );
            }
            let (ordinary_actions, conversation_actions) = pull_plan_action_counts(plan);
            let applicable_actions = ordinary_actions.saturating_add(conversation_actions);
            log.emit(
                sink,
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
                sink,
                "info",
                "pull.awaiting_approval",
                "   Nothing has been applied yet — use the Apply button in the Pull review.",
            );
        }
        Err(error) => log.emit(
            sink,
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

/// Apply the approved actions of a Pull review. The review's pins are
/// verified against the storage and this computer before the first write.
pub fn apply_bundle_restore(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    review: PullReview,
    approved_action_ids: Vec<ActionId>,
) -> Result<RestoreResult, String> {
    let review = RestorePlan::from(review);
    let storage_for_log = storage_log_label(repository, &review.storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Pull);
    log.emit(
        sink,
        "info",
        "pull.apply_started",
        &format!(
            "↓  Applying {} approved Pull actions…",
            approved_action_ids.len()
        ),
    );
    let mut fetch_progress = Some(FetchProgressEmitter::new(
        sink,
        log.clone()
            .storage(&review.storage_id, Some(&storage_for_log)),
        "apply",
        review.storage_id.clone(),
        storage_for_log,
        review.bundle_id.clone(),
    ));
    let total_started = Instant::now();
    let result = ProjectSyncService::new(repository).apply_bundle_restore_with_progress(
        &review,
        approved_action_ids,
        &mut |update| {
            if let Some(emitter) = fetch_progress.as_mut() {
                emitter.observe(update);
            }
        },
        &mut |phase, elapsed| {
            log.emit(
                sink,
                "info",
                "pull.phase_timing",
                &format!(
                    "   Pull apply phase — {phase}: {:.3}s",
                    elapsed.as_secs_f64()
                ),
            );
        },
    );
    log.emit(
        sink,
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
                let resource = pull_action_resource_label(&review, action_id)
                    .unwrap_or_else(|| action_id.to_string());
                log.emit(
                    sink,
                    "ok",
                    "pull.resource_applied",
                    &format!("↓  {}", resource),
                );
            }
            for failure in &operation.failed_actions {
                log.emit(
                    sink,
                    "error",
                    "pull.resource_failed",
                    &format!("✗  {}: {}", failure.action_id, failure.message),
                );
            }
            log.emit(
                sink,
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
                // Guidance is informational: an unreadable settings file
                // hides a saved CLI path, never the lines.
                match current_binding_for_restore_plan(repository, &review) {
                    Ok(binding) => {
                        let settings =
                            mallard_infrastructure::app_settings::load_app_settings(repository)
                                .unwrap_or_default();
                        log.emit(
                            sink,
                            "info",
                            "pull.open_project_hint",
                            "   Open this project after Pull:",
                        );
                        for (label, command) in project_open_commands(&binding, &settings) {
                            log.emit(
                                sink,
                                "info",
                                "pull.open_project_command",
                                &format!("   {}: {}", label, command),
                            );
                        }
                    }
                    Err(error) => log.emit(
                        sink,
                        "info",
                        "pull.open_project_unavailable",
                        &format!("   Open command unavailable: {}", error),
                    ),
                }
            }
        }
        Err(error) => log.emit(
            sink,
            "error",
            "pull.failed",
            &format!("✗  Pull failed: {}", error),
        ),
    }
    result
}

pub fn prepare_push_review(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    scope: PushScope,
) -> Result<PushReviewContext, String> {
    let (project, storage, _) =
        project_storage_log_context(repository, local_project_id, storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(local_project_id, Some(&project))
        .storage(storage_id, Some(&storage));
    log.emit(
        sink,
        "info",
        "push.review_started",
        &format!("↑  Preparing Push review — {} → {}", project, storage),
    );
    let total_started = Instant::now();
    let result = ProjectSyncService::new(repository).prepare_push_review_with_phases(
        local_project_id,
        storage_id,
        scope,
        &mut |phase, elapsed| {
            log.emit(
                sink,
                "info",
                "push.phase_timing",
                &format!(
                    "   Push review phase — {phase}: {:.3}s",
                    elapsed.as_secs_f64()
                ),
            );
        },
    );
    log.emit(
        sink,
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
                sink,
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
                    sink,
                    "warning",
                    "push.comparison_failed",
                    &format!("⚠  Conversation comparison failed: {}", error),
                );
            }
            if let Some(error) = &context.project_files_error {
                log.emit(
                    sink,
                    "warning",
                    "push.project_files_failed",
                    &format!("⚠  Project files inventory failed: {}", error),
                );
            }
        }
        Err(error) => log.emit(
            sink,
            "error",
            "push.review_failed",
            &format!("✗  Push review failed: {}", error),
        ),
    }
    result
}

pub fn finalize_project_setup(
    repository: &LocalStateRepository,
    sink: &dyn SyncEventSink,
    draft_id: &SetupDraftId,
    expected_revision: u64,
) -> Result<ProjectDetail, String> {
    let log = ActivityLogScope::new(ActivityLogType::Configuration);
    log.emit(
        sink,
        "info",
        "configuration.project_setup_started",
        "Finalizing project setup…",
    );
    let result =
        ProjectSyncService::new(repository).finalize_project_setup(draft_id, expected_revision);
    match &result {
        Ok(detail) => log.emit(
            sink,
            "ok",
            "configuration.project_setup_completed",
            &format!("Project {} is set up", detail.project.display_name),
        ),
        Err(error) => log.emit(
            sink,
            "error",
            "configuration.project_setup_failed",
            &format!("Project setup failed: {}", error),
        ),
    }
    result
}
