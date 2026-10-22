//! Tauri command adapters for the Mallard Cloud website bridge. The reads
//! and the two repository actions live in
//! `mallard_application::project_sync::managed_cloud_link`.
use mallard_application::project_sync::managed_cloud_link::{
    accept_repository_invitation, discard_draft, list_public_projects, list_repositories,
    list_starred_projects, publish_version, resolve_repository, storage_client,
    valid_local_fingerprint, ManagedCloudPublishedVersion, ManagedCloudRepository,
    ManagedCloudRepositoryResolution,
};
use mallard_application::project_sync::service::repository;
use mallard_domain::{LocalProjectId, StorageId};
use mallard_infrastructure::managed_cloud::client::{
    DiscardedDraft, PublicSort, RepositoryListing,
};

/// The project repositories the account belongs to on a Mallard Cloud
/// storage, each compared with the local project's Git remote.
#[tauri::command]
pub async fn list_managed_cloud_repositories(
    storage_id: StorageId,
    local_repository_fingerprint: Option<String>,
) -> Result<Vec<ManagedCloudRepository>, String> {
    let local = valid_local_fingerprint(&local_repository_fingerprint)?;
    let (client, origin) = storage_client(&storage_id)?;
    list_repositories(&client, &origin, local).await
}

/// Starred: the project repositories the account starred on the website.
/// A row is picked by resolving its address, which compares the remote.
#[tauri::command]
pub async fn list_managed_cloud_starred(
    storage_id: StorageId,
    cursor: Option<String>,
) -> Result<RepositoryListing, String> {
    let (client, _) = storage_client(&storage_id)?;
    list_starred_projects(&client, cursor.as_deref()).await
}

/// Public: the released projects on the service, `newest` or by `stars`.
#[tauri::command]
pub async fn list_managed_cloud_public(
    storage_id: StorageId,
    sort: String,
    cursor: Option<String>,
) -> Result<RepositoryListing, String> {
    let sort = PublicSort::parse(&sort).ok_or("sort must be newest or stars")?;
    let (client, _) = storage_client(&storage_id)?;
    list_public_projects(&client, sort, cursor.as_deref()).await
}

/// One pasted address, resolved against a Mallard Cloud storage: what is
/// there, whether the account belongs to it, and how its Git remote
/// compares with the local project's.
#[tauri::command]
pub async fn resolve_managed_cloud_repository(
    storage_id: StorageId,
    address: String,
    local_repository_fingerprint: Option<String>,
) -> Result<ManagedCloudRepositoryResolution, String> {
    let local = valid_local_fingerprint(&local_repository_fingerprint)?;
    let (client, origin) = storage_client(&storage_id)?;
    resolve_repository(&client, &origin, &address, local).await
}

/// One invitation code entered against a Mallard Cloud storage: the
/// account joins the private repository the code belongs to, which comes
/// back the way `resolve_managed_cloud_repository` describes one.
#[tauri::command]
pub async fn accept_managed_cloud_invitation(
    storage_id: StorageId,
    code: String,
    local_repository_fingerprint: Option<String>,
) -> Result<ManagedCloudRepositoryResolution, String> {
    let local = valid_local_fingerprint(&local_repository_fingerprint)?;
    let (client, origin) = storage_client(&storage_id)?;
    accept_repository_invitation(&client, &origin, &code, local).await
}

/// Publish version, from the Cloud chip's panel. Names the repository by
/// the link's pin.
#[tauri::command]
pub async fn publish_managed_cloud_version(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    name: Option<String>,
) -> Result<ManagedCloudPublishedVersion, String> {
    publish_version(
        &repository()?,
        &local_project_id,
        &storage_id,
        name.as_deref(),
    )
    .await
}

/// The Owner's discard of a draft past the latest version, confirmed in the
/// workspace. Names the repository by the link's pin and the repository
/// revision the status showed.
#[tauri::command]
pub async fn discard_managed_cloud_draft(
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    expected_repository_revision: u64,
) -> Result<DiscardedDraft, String> {
    discard_draft(
        &repository()?,
        &local_project_id,
        &storage_id,
        expected_repository_revision,
    )
    .await
}
