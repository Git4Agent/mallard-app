//! The provider CLIs as this machine resolves them: what discovery finds for
//! each provider, and a `--version` test of that executable. The saved
//! selection itself travels in app settings.

use super::service::provider_registry;
use mallard_domain::ProviderKey;
use mallard_infrastructure::app_settings::load_app_settings;
use mallard_infrastructure::cli_discovery::CliSelectionSource;
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

/// Discovery may run the user's shell once; call this off any interactive
/// thread.
pub fn inspect(
    repository: &LocalStateRepository,
    provider: &ProviderKey,
) -> Result<ProviderCliStatus, String> {
    let settings = load_app_settings(repository)?;
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

/// `--version` of the resolved executable, isolated from the user's homes
/// and credentials. Returns the version alone.
pub fn test(repository: &LocalStateRepository, provider: &ProviderKey) -> Result<String, String> {
    let settings = load_app_settings(repository)?;
    let cli = provider_cli(provider)?;
    let resolved = cli.resolve(settings.cli_path(provider))?;
    cli.probe_version(&resolved)
}
