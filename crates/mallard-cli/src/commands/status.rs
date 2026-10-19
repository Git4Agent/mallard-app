use mallard_application::project_sync::service::{
    get_host_identity_status_with_repository, load_project_sync_config_with_repository,
};
use serde::Serialize;

use crate::args::StatusArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::CommandOutput;

#[derive(Serialize)]
struct CurrentProject {
    local_project_id: String,
    display_name: String,
    project_root: Option<String>,
    storages: Vec<String>,
}

pub(crate) fn status(context: &CliContext, args: StatusArgs) -> Result<CommandOutput, CliError> {
    let (host, config) = context.service(|repository, _| {
        Ok((
            get_host_identity_status_with_repository(repository)?,
            load_project_sync_config_with_repository(repository)?,
        ))
    })?;
    let current = match context.resolve_project(args.selector.project.as_deref()) {
        Ok(id) => {
            let bindings = context
                .repository()
                .load_bindings()
                .map_err(|_| CliError::persistence(crate::context::LOCAL_STATE_MESSAGE))?;
            config
                .projects
                .iter()
                .find(|project| project.local_project_id == id)
                .map(|project| CurrentProject {
                    local_project_id: id.to_string(),
                    display_name: project
                        .local_alias
                        .clone()
                        .unwrap_or_else(|| project.display_name.clone()),
                    project_root: bindings
                        .bindings
                        .iter()
                        .find(|binding| binding.local_project_id == id)
                        .map(|binding| binding.project_root.clone()),
                    storages: config
                        .links
                        .iter()
                        .filter(|link| link.local_project_id == id)
                        .map(|link| link.storage_id.to_string())
                        .collect(),
                })
        }
        // No project here is a plain fact of the status, not a failure.
        Err(error) if error.code() == "project_not_found" => None,
        Err(error) => return Err(error),
    };

    let mut text = format!(
        "Host {} (revision {})\nConfiguration revision {}\n",
        host.host_id, host.revision, config.revision
    );
    text.push_str(&format!("Storages: {}\n", config.storages.len()));
    for storage in &config.storages {
        text.push_str(&format!(
            "  {} [{}] — {}{}\n",
            storage.name,
            storage.storage_id,
            storage.provider_label,
            if storage.available {
                ""
            } else {
                " (unavailable in this build)"
            }
        ));
    }
    text.push_str(&format!("Projects: {}\n", config.projects.len()));
    for project in &config.projects {
        let links = config
            .links
            .iter()
            .filter(|link| link.local_project_id == project.local_project_id)
            .count();
        text.push_str(&format!(
            "  {} [{}] — {} link{}\n",
            project
                .local_alias
                .as_deref()
                .unwrap_or(&project.display_name),
            project.local_project_id,
            links,
            if links == 1 { "" } else { "s" }
        ));
    }
    let selected = args.selector.project.is_some();
    match &current {
        Some(project) if selected => text.push_str(&format!(
            "Project: {} [{}]\n",
            project.display_name, project.local_project_id
        )),
        Some(project) => text.push_str(&format!(
            "Current folder: project {} [{}]\n",
            project.display_name, project.local_project_id
        )),
        None => text.push_str("Current folder: not inside a registered project\n"),
    }

    CommandOutput::merged(
        "ok",
        &serde_json::json!({
            "host": host,
            "config": config,
            "current_project": current,
        }),
        text,
    )
}
