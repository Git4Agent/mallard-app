use mallard_application::project_sync::maintenance::MaintenanceActionRequest;
use mallard_application::project_sync::operations;
use mallard_application::project_sync::provider_cli;
use mallard_application::project_sync::service::agent_provider_descriptors;
use mallard_domain::{ActionKey, BoundedConfigObject, ProviderKey};
use mallard_infrastructure::cli_discovery::refresh_shell_path;

use crate::args::{MaintenanceArgs, ProviderCliCommand, ProviderCommand, ProviderRef};
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::CommandOutput;

pub(crate) fn run(
    context: &CliContext,
    command: ProviderCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        ProviderCommand::List => list(),
        ProviderCommand::Cli(args) => match args.command {
            ProviderCliCommand::Inspect(provider) => inspect(context, provider),
            ProviderCliCommand::Test(provider) => test(context, provider),
            ProviderCliCommand::Refresh => {
                refresh_shell_path();
                CommandOutput::new(
                    "refreshed",
                    "refreshed",
                    &true,
                    "The shell PATH will be probed again on the next inspection.".to_string(),
                )
            }
        },
        ProviderCommand::Maintenance(args) => maintenance(context, args),
    }
}

pub(crate) fn provider_key(value: &str) -> Result<ProviderKey, CliError> {
    let key = ProviderKey::parse(value).map_err(|_| {
        CliError::invalid_with(
            "provider_unknown",
            "That is not a provider key. Run `mallard-cli provider list`.",
        )
    })?;
    if !agent_provider_descriptors()
        .iter()
        .any(|descriptor| descriptor.key == key)
    {
        return Err(CliError::invalid_with(
            "provider_unknown",
            "No provider has that key. Run `mallard-cli provider list`.",
        ));
    }
    Ok(key)
}

fn list() -> Result<CommandOutput, CliError> {
    let providers = agent_provider_descriptors();
    let mut text = String::new();
    for provider in &providers {
        text.push_str(&format!(
            "{} ({}){}\n",
            provider.display_name,
            provider.key,
            if provider.has_cli {
                " — has a CLI"
            } else {
                ""
            }
        ));
        for action in &provider.maintenance_actions {
            text.push_str(&format!(
                "  action {} — {}{}{}\n",
                action.action_id,
                action.label,
                if action.read_only { " (read-only)" } else { "" },
                if action.requires_confirmation {
                    " (needs --confirm)"
                } else {
                    ""
                }
            ));
        }
    }
    CommandOutput::new("ok", "providers", &providers, text)
}

fn inspect(context: &CliContext, provider: ProviderRef) -> Result<CommandOutput, CliError> {
    let key = provider_key(&provider.provider)?;
    let status = context.service(move |repository, _| provider_cli::inspect(repository, &key))?;
    let text = match (&status.executable, &status.error) {
        (Some(executable), _) => format!(
            "{}: {}{}",
            status.provider,
            executable,
            status
                .source
                .as_ref()
                .map(|source| format!(" ({source:?})"))
                .unwrap_or_default()
        ),
        (None, Some(error)) => format!("{}: not found — {error}", status.provider),
        (None, None) => format!("{}: not found", status.provider),
    };
    CommandOutput::new("ok", "cli", &status, text)
}

fn test(context: &CliContext, provider: ProviderRef) -> Result<CommandOutput, CliError> {
    let key = provider_key(&provider.provider)?;
    let version = context.service(move |repository, _| provider_cli::test(repository, &key))?;
    CommandOutput::new("ok", "version", &version, version.clone())
}

fn maintenance(context: &CliContext, args: MaintenanceArgs) -> Result<CommandOutput, CliError> {
    let provider = provider_key(&args.provider.provider)?;
    let action_id = ActionKey::parse(args.action.as_str()).map_err(|_| {
        CliError::invalid_with(
            "maintenance_action_unknown",
            "That is not an action id. Run `mallard-cli provider list`.",
        )
    })?;
    let arguments = match &args.arguments {
        Some(text) => BoundedConfigObject::from_json_str(text).map_err(|_| {
            CliError::invalid_code(
                "maintenance_arguments_invalid",
                "--arguments must be one small JSON object.",
            )
        })?,
        None => BoundedConfigObject::empty(),
    };
    let request = MaintenanceActionRequest {
        provider,
        action_id,
        arguments,
        confirmed: args.confirm,
    };
    let result = context.service(move |repository, sink| {
        operations::execute_provider_maintenance_action(repository, sink, request)
    })?;
    let text = format!(
        "{} {}: {}{}",
        result.provider,
        result.action_id,
        result.summary,
        if result.ready { "" } else { " (not ready)" }
    );
    CommandOutput::new(
        if result.ready { "ok" } else { "not_ready" },
        "result",
        &result,
        text,
    )
}
