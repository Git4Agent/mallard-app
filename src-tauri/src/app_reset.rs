//! App-scoped local-data reset: measure and delete the whole `~/.mallard`
//! metadata root. The deletion itself lives on
//! `LocalStateRepository::wipe_local_state`; this module adds the
//! catalog-state guard the infrastructure crate cannot see, re-arms the two
//! long-lived backend holders of pre-wipe state, and exposes the Tauri
//! surface. The process deliberately stays alive — relaunching the raw
//! binary under `tauri dev` orphans the frontend from its dev server (the
//! CLI kills Vite when the original process exits, and the relaunched app
//! then loads `devUrl` from a dead port: a white window). The frontend
//! finishes the cutover with `window.location.reload()` instead.

use crate::activity_log::{ActivityLogScope, ActivityLogType};
use crate::project_sync::config_watcher::ConfigWatcher;
use crate::project_sync::session_watch_service::session_watch_service;
use crate::storage_catalog::persistence::CatalogStateRepository;
use mallard_infrastructure::persistence::{
    LocalStateRepository, LocalStateSummary, LocalStateWipeResult,
};
use tauri::Manager as _;

async fn run_reset_io<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("local-data reset worker failed: {error}"))?
}

/// Non-Tauri core, unit-testable. Lock order is catalog-state → host
/// operation → persistence — the order `clear_storage_data` already
/// establishes; inverting it risks a flock deadlock between two commands.
fn reset_local_state_with_repositories(
    repository: &LocalStateRepository,
    catalog: &CatalogStateRepository,
) -> Result<LocalStateWipeResult, String> {
    let _catalog_guard = catalog.acquire_state_guard()?;
    repository.wipe_local_state()
}

#[tauri::command]
pub async fn get_local_state_summary(_app: tauri::AppHandle) -> Result<LocalStateSummary, String> {
    run_reset_io(|| LocalStateRepository::from_user_home()?.local_state_summary()).await
}

#[tauri::command]
pub async fn reset_local_state(app: tauri::AppHandle) -> Result<LocalStateWipeResult, String> {
    // The started breadcrumb reaches the live drawer even though its persisted
    // copy is about to be deleted, and explains a partial state if the wipe
    // dies midway. A failure is re-logged durably (the log store recreates the
    // root). Success deliberately logs nothing: a persisted success entry
    // would recreate `~/.mallard/logs` before the restart, and the restarted
    // app already logs `system.app_started`.
    let scope = ActivityLogScope::new(ActivityLogType::System);
    scope.emit(
        &app,
        "warning",
        "system.local_state_reset_started",
        "Deleting all Mallard local data…",
    );
    let outcome = run_reset_io(|| {
        let repository = LocalStateRepository::from_user_home()?;
        let catalog = CatalogStateRepository::from_user_home()?;
        reset_local_state_with_repositories(&repository, &catalog)
    })
    .await;
    if let Err(error) = &outcome {
        scope.emit(
            &app,
            "error",
            "system.local_state_reset_failed",
            &format!("Local data reset failed: {error}"),
        );
    }
    if outcome.is_ok() {
        // Everything else backend-side is constructed per call, and the two
        // process-wide content caches are keyed by content coordinates, so
        // three things hold pre-wipe state: the installed tuning snapshot
        // (settings.json is gone, so live values return to defaults), the
        // session watch service (caches keyed by registrations that no longer
        // exist), and the config watcher (its directory watch died with the
        // deleted root). Re-arm all of them before the promise resolves and
        // the frontend reloads.
        mallard_infrastructure::app_settings::install_app_settings(
            &mallard_infrastructure::app_settings::AppSettings::default(),
        );
        session_watch_service().reset_all_caches();
        // Starting a fresh watcher recreates the (empty) root — accepted; the
        // boot log entry recreates it on a real fresh launch too.
        match ConfigWatcher::start(app.clone()) {
            Ok(next) => {
                if let Ok(mut watcher) = app.state::<std::sync::Mutex<ConfigWatcher>>().lock() {
                    *watcher = next;
                }
            }
            Err(_) => {
                // Live config refresh stays degraded until the next launch;
                // the reset itself succeeded and the reload still lands.
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::reset_local_state_with_repositories;
    use crate::storage_catalog::persistence::CatalogStateRepository;
    use mallard_infrastructure::persistence::LocalStateRepository;

    #[test]
    fn reset_with_repositories_clears_catalog_state_too() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(temp.path()).unwrap();
        std::fs::write(catalog.root().join("catalog_scopes.json"), b"{}").unwrap();

        let result = reset_local_state_with_repositories(&repository, &catalog).unwrap();
        assert!(result.existed);
        assert!(result.files_removed > 0);
        assert!(!temp.path().join(".mallard").exists());

        // Both repositories reconstruct cleanly over the fresh root.
        LocalStateRepository::from_home_dir(temp.path()).unwrap();
        CatalogStateRepository::from_home_dir(temp.path()).unwrap();
    }
}
