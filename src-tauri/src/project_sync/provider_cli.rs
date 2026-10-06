//! Settings commands for the provider CLIs: what discovery finds for each
//! provider on this machine, a `--version` test of that executable, and a
//! fresh shell probe. The saved selection itself travels in app settings.

use super::service::provider_registry;
use mallard_domain::ProviderKey;
use mallard_infrastructure::app_settings::load_app_settings;
use mallard_infrastructure::cli_discovery::{refresh_shell_path, CliSelectionSource};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::providers::ProviderCli;
use serde::Serialize;

/// One provider's CLI as this machine resolves it right now.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderCliStatus {
    pub provider: ProviderKey,
    /// The path saved in Settings; absent means automatic discovery.
    pub selected_path: Option<String>,
    /// The launcher discovery chose, as found; absent on failure.
    pub launcher: Option<String>,
    /// The executable that launcher runs.
    pub executable: Option<String>,
    pub source: Option<CliSelectionSource>,
    /// Why nothing resolved: short, without paths or environment contents.
    pub error: Option<String>,
}

fn provider_cli(provider: &ProviderKey) -> Result<&'static dyn ProviderCli, String> {
    provider_registry()
        .require(provider)?
        .cli()
        .ok_or_else(|| format!("{provider} has no command-line tool"))
}

fn inspect(provider: &ProviderKey) -> Result<ProviderCliStatus, String> {
    let settings = load_app_settings(&LocalStateRepository::from_user_home()?)?;
    let selected_path = settings.cli_path(provider);
    let mut status = ProviderCliStatus {
        provider: provider.clone(),
        selected_path: selected_path.map(|path| path.to_string_lossy().into_owned()),
        launcher: None,
        executable: None,
        source: None,
        error: None,
    };
    match provider_cli(provider)?.resolve(selected_path) {
        Ok(cli) => {
            status.launcher = Some(cli.launcher.to_string_lossy().into_owned());
            status.executable = Some(cli.executable.to_string_lossy().into_owned());
            status.source = Some(cli.source);
        }
        Err(error) => status.error = Some(error),
    }
    Ok(status)
}

fn test(provider: &ProviderKey) -> Result<String, String> {
    let settings = load_app_settings(&LocalStateRepository::from_user_home()?)?;
    let cli = provider_cli(provider)?;
    let resolved = cli.resolve(settings.cli_path(provider))?;
    cli.probe_version(&resolved)
}

/// Discovery may run the user's shell once; it belongs off the main thread.
#[tauri::command]
pub async fn inspect_provider_cli(provider: ProviderKey) -> Result<ProviderCliStatus, String> {
    tauri::async_runtime::spawn_blocking(move || inspect(&provider))
        .await
        .map_err(|error| format!("CLI inspection worker failed: {error}"))?
}

/// `--version` of the resolved executable, isolated from the user's homes
/// and credentials. Returns the version alone.
#[tauri::command]
pub async fn test_provider_cli(provider: ProviderKey) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || test(&provider))
        .await
        .map_err(|error| format!("CLI test worker failed: {error}"))?
}

/// Forget the probed shell `PATH`; the next inspection runs the shell again.
#[tauri::command]
pub fn refresh_cli_detection() {
    refresh_shell_path();
}
