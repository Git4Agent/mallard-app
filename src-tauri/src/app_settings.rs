//! Tauri-facing tuning-settings commands. Saving, making live, and the cache
//! consequence are `mallard_application::app_settings`.

use mallard_infrastructure::app_settings::{load_app_settings, AppSettings};
use mallard_infrastructure::persistence::LocalStateRepository;

async fn run_settings_io<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("settings worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_app_settings() -> Result<AppSettings, String> {
    run_settings_io(|| load_app_settings(&LocalStateRepository::from_user_home()?)).await
}

#[tauri::command]
pub async fn update_app_settings(settings: AppSettings) -> Result<AppSettings, String> {
    run_settings_io(move || {
        let repository = LocalStateRepository::from_user_home()?;
        mallard_application::app_settings::update_app_settings(&repository, &settings)
    })
    .await
}
