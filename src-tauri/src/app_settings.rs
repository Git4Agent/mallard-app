//! Tauri-facing tuning-settings commands. The document, validation, and the
//! installed process-wide snapshot live in
//! `mallard_infrastructure::app_settings`; this module adds the command
//! surface and the cache consequences the infrastructure crate cannot see:
//! observation and inventory caches bake the old limits into their contents
//! and a settings change fires no filesystem event, so an update drops them
//! explicitly.

use crate::project_sync::session_watch_service::session_watch_service;
use mallard_infrastructure::app_settings::{
    install_app_settings, load_app_settings, save_app_settings, AppSettings,
};
use mallard_infrastructure::persistence::LocalStateRepository;

async fn run_settings_io<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("settings worker failed: {error}"))?
}

/// Non-Tauri core, unit-testable: persist, make live, return the read-back.
fn update_app_settings_core(
    repository: &LocalStateRepository,
    settings: &AppSettings,
) -> Result<AppSettings, String> {
    save_app_settings(repository, settings)?;
    install_app_settings(settings);
    load_app_settings(repository)
}

#[tauri::command]
pub async fn get_app_settings() -> Result<AppSettings, String> {
    run_settings_io(|| load_app_settings(&LocalStateRepository::from_user_home()?)).await
}

#[tauri::command]
pub async fn update_app_settings(settings: AppSettings) -> Result<AppSettings, String> {
    run_settings_io(move || {
        let repository = LocalStateRepository::from_user_home()?;
        let saved = update_app_settings_core(&repository, &settings)?;
        session_watch_service().reset_all_caches_and_purge_disk(repository.root())?;
        Ok(saved)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::update_app_settings_core;
    use mallard_infrastructure::app_settings::AppSettings;
    use mallard_infrastructure::persistence::LocalStateRepository;

    /// Default values only: the install half writes process-wide atomics and
    /// this binary runs its tests concurrently.
    #[test]
    fn update_core_persists_installs_and_reads_back() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let settings = AppSettings::default();

        let saved = update_app_settings_core(&repository, &settings).unwrap();
        assert_eq!(saved, settings);
        assert!(temp.path().join(".mallard").join("settings.json").is_file());

        let invalid = AppSettings {
            comparison_workers: 0,
            ..settings
        };
        assert!(update_app_settings_core(&repository, &invalid).is_err());
    }
}
