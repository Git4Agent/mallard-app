use mallard_infrastructure::app_settings::{load_app_settings, AppSettings};

use crate::args::{SettingsCommand, SettingsSetArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::CommandOutput;

pub(crate) fn run(
    context: &CliContext,
    command: SettingsCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        SettingsCommand::Get => {
            let settings = context.service(|repository, _| load_app_settings(repository))?;
            CommandOutput::new("ok", "settings", &settings, render(&settings)?)
        }
        SettingsCommand::Set(args) => set(context, args),
    }
}

fn render(settings: &AppSettings) -> Result<String, CliError> {
    let value = serde_json::to_value(settings)
        .map_err(|_| CliError::persistence("Mallard CLI could not encode the settings."))?;
    let mut text = String::new();
    if let serde_json::Value::Object(fields) = value {
        for (key, value) in fields {
            if key == "schema" {
                continue;
            }
            text.push_str(&format!("{key} = {value}\n"));
        }
    }
    Ok(text)
}

fn set(context: &CliContext, args: SettingsSetArgs) -> Result<CommandOutput, CliError> {
    let mut document =
        serde_json::to_value(context.service(|repository, _| load_app_settings(repository))?)
            .map_err(|_| CliError::persistence("Mallard CLI could not encode the settings."))?;
    let fields = document
        .as_object_mut()
        .ok_or_else(|| CliError::persistence("Mallard CLI could not encode the settings."))?;
    for assignment in &args.assignments {
        let Some((key, raw)) = assignment.split_once('=') else {
            return Err(CliError::invalid_code(
                "settings_assignment_invalid",
                "Each setting is written key=value.",
            ));
        };
        if key == "schema" || !fields.contains_key(key) {
            return Err(CliError::invalid_with(
                "settings_key_unknown",
                format!("Unknown setting '{key}'. Run `mallard-cli settings get` for the keys."),
            ));
        }
        // A JSON value when it parses as one (`true`, `4`, `"text"`, `null`),
        // the plain text otherwise.
        let value = serde_json::from_str::<serde_json::Value>(raw)
            .unwrap_or_else(|_| serde_json::Value::String(raw.to_string()));
        fields.insert(key.to_string(), value);
    }
    let settings: AppSettings = serde_json::from_value(document).map_err(|error| {
        CliError::invalid_with(
            "settings_value_invalid",
            format!("A setting has the wrong type: {error}"),
        )
    })?;
    let saved = context.service(move |repository, _| {
        mallard_application::app_settings::update_app_settings(repository, &settings)
    })?;
    CommandOutput::new("saved", "settings", &saved, render(&saved)?)
}
