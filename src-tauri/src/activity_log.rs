//! Activity-log commands: the drawer's reads, its retention policy, and the
//! folder. Emission is `mallard_application::activity_log` through the
//! desktop's `event_sink`.

use mallard_infrastructure::activity_log::{
    ActivityLogCleanupRequest, ActivityLogCleanupResult, ActivityLogPage, ActivityLogPolicy,
    ActivityLogQuery, ActivityLogStats, ActivityLogStore,
};
use mallard_infrastructure::persistence::LocalStateRepository;

/// Resolve the activity log store below the machine-global metadata directory.
fn activity_log_store() -> Result<ActivityLogStore, String> {
    let repository = LocalStateRepository::from_user_home()?;
    ActivityLogStore::from_repository(&repository)
}

#[tauri::command]
pub async fn query_activity_logs(query: ActivityLogQuery) -> Result<ActivityLogPage, String> {
    let store = activity_log_store()?;
    // Appends enforce retention. Keep this read path free of a second cleanup scan.
    run_log_io(move || store.query(&query)).await
}

#[tauri::command]
pub async fn get_activity_log_stats() -> Result<ActivityLogStats, String> {
    let store = activity_log_store()?;
    run_log_io(move || store.stats()).await
}

#[tauri::command]
pub async fn update_activity_log_policy(
    policy: ActivityLogPolicy,
) -> Result<ActivityLogStats, String> {
    let store = activity_log_store()?;
    run_log_io(move || {
        store.save_policy(&policy)?;
        store.stats()
    })
    .await
}

#[tauri::command]
pub async fn cleanup_activity_logs(
    request: ActivityLogCleanupRequest,
) -> Result<ActivityLogCleanupResult, String> {
    let store = activity_log_store()?;
    run_log_io(move || store.cleanup(&request)).await
}

#[tauri::command]
pub fn get_activity_log_folder() -> Result<String, String> {
    let store = activity_log_store()?;
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
