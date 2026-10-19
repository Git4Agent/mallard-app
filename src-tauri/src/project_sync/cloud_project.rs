//! Tauri commands for getting a project's folder from a Mallard Cloud
//! repository: the probe, the clone or creation with progress, and Cancel.
//! The work is `mallard_application::project_sync::cloud_project`.
pub use mallard_application::project_sync::cloud_project::CloudProjectControl;
use mallard_application::project_sync::cloud_project::{
    clone_tools, get_folder, github_source, probe_folder, valid_fingerprint, valid_operation_id,
    CloudProjectFolderProbe, CloudProjectFolderProgress, CloudProjectFolderRequest,
    CloudProjectFolderResult,
};
use mallard_application::project_sync::service::repository;

/// The Folder field's verdict for a new project's destination. Reads only.
#[tauri::command]
pub async fn probe_cloud_project_folder(
    destination: String,
    repository_fingerprint: Option<String>,
) -> Result<CloudProjectFolderProbe, String> {
    let fingerprint = valid_fingerprint(repository_fingerprint)?;
    let repository = repository()?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(CloudProjectFolderProbe {
            folder: probe_folder(&repository, &destination, fingerprint.as_deref())?,
            can_clone: clone_tools().git.is_some(),
        })
    })
    .await
    .map_err(|error| format!("project-sync worker failed: {error}"))?
}

/// Makes the new project's folder while preserving its repository class.
#[tauri::command]
pub async fn get_cloud_project_folder(
    control: tauri::State<'_, CloudProjectControl>,
    request: CloudProjectFolderRequest,
    on_progress: tauri::ipc::Channel<CloudProjectFolderProgress>,
) -> Result<CloudProjectFolderResult, String> {
    valid_operation_id(&request.operation_id)?;
    let fingerprint = valid_fingerprint(request.repository_fingerprint)?;
    let source = request
        .github_url
        .as_deref()
        .map(|url| github_source(url, fingerprint.as_deref()))
        .transpose()?;
    let run = control.begin(&request.operation_id)?;
    let repository = repository()?;
    let tools = tauri::async_runtime::spawn_blocking(clone_tools)
        .await
        .map_err(|error| format!("project-sync worker failed: {error}"))?;
    get_folder(
        repository,
        tools,
        request.destination,
        fingerprint,
        request.is_git_project,
        source,
        run.cancellation.clone(),
        &|progress| {
            let _ = on_progress.send(progress);
        },
    )
    .await
}

/// Stops the clone of that run; git removes what it wrote.
#[tauri::command]
pub fn cancel_cloud_project_folder(
    control: tauri::State<'_, CloudProjectControl>,
    operation_id: String,
) -> Result<(), String> {
    valid_operation_id(&operation_id)?;
    control.cancel(&operation_id);
    Ok(())
}
