//! Settings commands for the provider CLIs: what discovery finds for each
//! provider on this machine, a `--version` test of that executable, and a
//! fresh shell probe. The resolution is
//! `mallard_application::project_sync::provider_cli`.

pub use mallard_application::project_sync::provider_cli::ProviderCliStatus;
use mallard_domain::ProviderKey;
use mallard_infrastructure::cli_discovery::refresh_shell_path;
use mallard_infrastructure::persistence::LocalStateRepository;

/// Discovery may run the user's shell once; it belongs off the main thread.
#[tauri::command]
pub async fn inspect_provider_cli(provider: ProviderKey) -> Result<ProviderCliStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        mallard_application::project_sync::provider_cli::inspect(
            &LocalStateRepository::from_user_home()?,
            &provider,
        )
    })
    .await
    .map_err(|error| format!("CLI inspection worker failed: {error}"))?
}

/// `--version` of the resolved executable, isolated from the user's homes
/// and credentials. Returns the version alone.
#[tauri::command]
pub async fn test_provider_cli(provider: ProviderKey) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        mallard_application::project_sync::provider_cli::test(
            &LocalStateRepository::from_user_home()?,
            &provider,
        )
    })
    .await
    .map_err(|error| format!("CLI test worker failed: {error}"))?
}

/// Forget the probed shell `PATH`; the next inspection runs the shell again.
#[tauri::command]
pub fn refresh_cli_detection() {
    refresh_shell_path();
}
