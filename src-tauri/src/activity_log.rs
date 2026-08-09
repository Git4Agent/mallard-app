//! Tauri-facing activity logging: frontend event emission and the
//! activity-log commands. The JSONL store itself lives in
//! `mallard_infrastructure::activity_log` and is re-exported here so
//! existing `crate::activity_log::` paths keep working.

use mallard_domain::generated_named_id;
use mallard_infrastructure::persistence::LocalStateRepository;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

#[cfg(not(test))]
use mallard_infrastructure::activity_log::day_for_timestamp;
use mallard_infrastructure::activity_log::normalized_level_name;
pub use mallard_infrastructure::activity_log::*;

static PERSISTENCE_WARNING_EMITTED: AtomicBool = AtomicBool::new(false);

/// Resolve the activity log store below the machine-global metadata
/// directory. `ActivityLogStore::from_app` lived on the store before the
/// workspace reorganization moved it out of the Tauri crate.
fn store_from_app<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
) -> Result<ActivityLogStore, String> {
    let repository = LocalStateRepository::from_user_home()?;
    ActivityLogStore::from_repository(&repository)
}

#[derive(Clone, Debug)]
pub struct ActivityLogScope {
    log_type: ActivityLogType,
    run_id: Option<String>,
    context: ActivityLogContext,
}

impl ActivityLogScope {
    pub fn new(log_type: ActivityLogType) -> Self {
        Self {
            log_type,
            run_id: generated_named_id(log_type.as_str()).ok(),
            context: ActivityLogContext::default(),
        }
    }

    pub fn project(mut self, project_id: impl ToString, project_name: Option<&str>) -> Self {
        self.context.project_id = Some(project_id.to_string());
        self.context.project_name = project_name.map(ToOwned::to_owned);
        self
    }

    pub fn storage(mut self, storage_id: impl ToString, storage_name: Option<&str>) -> Self {
        self.context.storage_id = Some(storage_id.to_string());
        self.context.storage_name = storage_name.map(ToOwned::to_owned);
        self
    }

    pub fn emit<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        level: &str,
        event: &str,
        message: &str,
    ) {
        emit_activity_log(
            app,
            self.log_type,
            level,
            event,
            message,
            self.run_id.clone(),
            self.context.clone(),
        );
    }
}

pub fn emit_typed_log<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    log_type: ActivityLogType,
    level: &str,
    message: &str,
) {
    let event = format!("{}.{}", log_type.as_str(), normalized_level_name(level));
    emit_activity_log(
        app,
        log_type,
        level,
        &event,
        message,
        None,
        ActivityLogContext::default(),
    );
}

fn emit_activity_log<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    log_type: ActivityLogType,
    level: &str,
    event: &str,
    message: &str,
    run_id: Option<String>,
    context: ActivityLogContext,
) {
    let entry = ActivityLogEntry::new(log_type, level, event, message, run_id, context);
    #[cfg(not(test))]
    let persisted = store_from_app(app).and_then(|store| {
        store.append(&entry)?;
        let day = day_for_timestamp(entry.ts)?;
        store.maybe_cleanup(&day);
        Ok(())
    });
    #[cfg(test)]
    let persisted: Result<(), String> = Ok(());
    let _ = app.emit("sync-log", &entry);
    if persisted.is_err() && !PERSISTENCE_WARNING_EMITTED.swap(true, Ordering::SeqCst) {
        let warning = ActivityLogEntry::new(
            ActivityLogType::System,
            "warning",
            "system.log_persistence_unavailable",
            "Activity log persistence is unavailable; current entries remain visible until Mallard closes.",
            None,
            ActivityLogContext::default(),
        );
        let _ = app.emit("sync-log", warning);
    }
}

#[tauri::command]
pub async fn query_activity_logs(
    app: tauri::AppHandle,
    query: ActivityLogQuery,
) -> Result<ActivityLogPage, String> {
    let store = store_from_app(&app)?;
    run_log_io(move || {
        let _ = store.cleanup(&ActivityLogCleanupRequest::default());
        store.query(&query)
    })
    .await
}

#[tauri::command]
pub async fn get_activity_log_stats(app: tauri::AppHandle) -> Result<ActivityLogStats, String> {
    let store = store_from_app(&app)?;
    run_log_io(move || store.stats()).await
}

#[tauri::command]
pub async fn update_activity_log_policy(
    app: tauri::AppHandle,
    policy: ActivityLogPolicy,
) -> Result<ActivityLogStats, String> {
    let store = store_from_app(&app)?;
    run_log_io(move || {
        store.save_policy(&policy)?;
        store.stats()
    })
    .await
}

#[tauri::command]
pub async fn cleanup_activity_logs(
    app: tauri::AppHandle,
    request: ActivityLogCleanupRequest,
) -> Result<ActivityLogCleanupResult, String> {
    let store = store_from_app(&app)?;
    run_log_io(move || store.cleanup(&request)).await
}

#[tauri::command]
pub fn get_activity_log_folder(app: tauri::AppHandle) -> Result<String, String> {
    let store = store_from_app(&app)?;
    store
        .root()
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "activity log folder is not valid UTF-8".to_string())
}

async fn run_log_io<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("activity log worker failed: {error}"))?
}
