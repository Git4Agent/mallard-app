//! Tauri command adapters for storage-scoped Library resources. Every
//! provider, kind, format, and target crosses this boundary as a key or a
//! coordinate object with backend-supplied labels; never a closed union.

use std::collections::BTreeMap;

use mallard_domain::StorageId;

use super::domain::{
    CatalogActionResult, CatalogFormatKey, CatalogLinkId, CatalogPlanActionId, CatalogPlanId,
    CatalogResourceForkPlan, CatalogResourceId, CatalogResourceKindKey, CatalogScopeId,
    CatalogScopePullPlan, CatalogScopePushPlan, LocalCatalogProfileId, ProviderKey,
};
use super::registry::CatalogCapabilityDescriptor;
use super::service::cloud::{CatalogCloudPlace, CatalogCloudRepository, CatalogCloudResolution};
use super::service::{
    CatalogProfileDto, CatalogProfileInspectionDto, CatalogProfileProbeDto, CatalogPullApproval,
    CatalogPushApproval, CatalogScopeDto, CatalogScopeLinkOutcomeDto, CatalogScopePullOutcomeDto,
    CatalogScopePushOutcomeDto, CatalogScopeResourcesDto, CatalogStorageRepositoryDto,
    StorageCatalogService,
};

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("catalog resource worker failed: {}", error))?
}

#[tauri::command]
pub async fn list_catalog_capabilities() -> Result<Vec<CatalogCapabilityDescriptor>, String> {
    run_blocking(move || Ok(StorageCatalogService::from_user_home()?.capabilities())).await
}

#[tauri::command]
pub async fn list_catalog_resource_profiles() -> Result<Vec<CatalogProfileDto>, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.list_profiles()).await
}

#[tauri::command]
pub async fn save_catalog_resource_profile(
    provider: ProviderKey,
    path: String,
) -> Result<CatalogProfileDto, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.save_profile(provider, path))
        .await
}

#[tauri::command]
pub async fn rename_catalog_resource_profile(
    profile_id: LocalCatalogProfileId,
    display_name: String,
    expected_revision: u64,
) -> Result<CatalogProfileDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.rename_profile(
            &profile_id,
            display_name,
            expected_revision,
        )
    })
    .await
}

#[tauri::command]
pub async fn probe_catalog_resource_profile(
    provider: ProviderKey,
    path: String,
) -> Result<CatalogProfileProbeDto, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.probe_profile(provider, path))
        .await
}

#[tauri::command]
pub async fn inspect_catalog_resource_profile(
    profile_id: LocalCatalogProfileId,
) -> Result<CatalogProfileInspectionDto, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.inspect_profile(&profile_id))
        .await
}

#[tauri::command]
pub async fn open_catalog_profile_in_app(profile_id: LocalCatalogProfileId) -> Result<(), String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.open_profile_in_app(&profile_id))
        .await
}

#[tauri::command]
pub async fn open_catalog_profile_in_terminal(
    profile_id: LocalCatalogProfileId,
) -> Result<(), String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.open_profile_in_terminal(&profile_id)
    })
    .await
}

#[tauri::command]
pub async fn list_catalog_scopes() -> Result<Vec<CatalogScopeDto>, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.list_scopes()).await
}

#[tauri::command]
pub async fn create_catalog_scope(
    provider: ProviderKey,
    profile_id: LocalCatalogProfileId,
    resource_kind: CatalogResourceKindKey,
    format: CatalogFormatKey,
    format_version: u32,
) -> Result<CatalogScopeDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.create_scope(
            provider,
            profile_id,
            resource_kind,
            format,
            format_version,
        )
    })
    .await
}

#[tauri::command]
pub async fn delete_catalog_scope(
    scope_id: CatalogScopeId,
    expected_revision: u64,
) -> Result<bool, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.delete_scope(&scope_id, expected_revision)
    })
    .await
}

#[tauri::command]
pub async fn list_catalog_scope_resources(
    scope_id: CatalogScopeId,
    storage_id: Option<StorageId>,
) -> Result<CatalogScopeResourcesDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?
            .list_scope_resources(&scope_id, storage_id.as_ref())
    })
    .await
}

#[tauri::command]
pub async fn get_catalog_resource_preview(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
) -> Result<String, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.resource_preview(
            &scope_id,
            &storage_id,
            &resource_id,
        )
    })
    .await
}

#[tauri::command]
pub async fn list_catalog_storage_repositories(
    storage_id: StorageId,
) -> Result<Vec<CatalogStorageRepositoryDto>, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.list_storage_repositories(&storage_id)
    })
    .await
}

#[tauri::command]
pub async fn link_catalog_scope_storage(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
) -> Result<CatalogScopeLinkOutcomeDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.link_scope_storage(&scope_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn unlink_catalog_scope_storage(
    link_id: CatalogLinkId,
    expected_revision: u64,
) -> Result<bool, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.unlink_scope_storage(&link_id, expected_revision)
    })
    .await
}

#[tauri::command]
pub async fn prepare_catalog_scope_push(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
) -> Result<CatalogScopePushPlan, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.prepare_push(&scope_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn apply_catalog_scope_push(
    plan_id: CatalogPlanId,
    selected_resource_ids: Vec<CatalogResourceId>,
    acknowledged_warning_digests: Vec<String>,
    on_progress: tauri::ipc::Channel<super::service::CatalogSyncProgress>,
) -> Result<CatalogScopePushOutcomeDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?
            .with_progress(move |progress| {
                let _ = on_progress.send(progress);
            })
            .apply_push(CatalogPushApproval {
                plan_id,
                selected_resource_ids,
                acknowledged_warning_digests,
            })
    })
    .await
}

/// *Finish that push*: write the record that the claim of the selected
/// version holds, once, and report. The user's verb, from the Library page.
#[tauri::command]
pub async fn finish_catalog_unfinished_push(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
) -> Result<(), String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.finish_unfinished_push(&scope_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn prepare_catalog_scope_pull(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    requested_copy_names: Option<BTreeMap<CatalogResourceId, String>>,
) -> Result<CatalogScopePullPlan, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.prepare_pull(
            &scope_id,
            &storage_id,
            requested_copy_names.unwrap_or_default(),
        )
    })
    .await
}

#[tauri::command]
pub async fn apply_catalog_scope_pull(
    plan_id: CatalogPlanId,
    choices: BTreeMap<CatalogResourceId, CatalogPlanActionId>,
    destructive_acknowledged: Vec<CatalogResourceId>,
    on_progress: tauri::ipc::Channel<super::service::CatalogSyncProgress>,
) -> Result<CatalogScopePullOutcomeDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?
            .with_progress(move |progress| {
                let _ = on_progress.send(progress);
            })
            .apply_pull(CatalogPullApproval {
                plan_id,
                choices,
                destructive_acknowledged,
            })
    })
    .await
}

#[tauri::command]
pub async fn prepare_catalog_resource_fork(
    scope_id: CatalogScopeId,
    resource_id: CatalogResourceId,
    requested_name: String,
) -> Result<CatalogResourceForkPlan, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.prepare_fork(
            &scope_id,
            &resource_id,
            &requested_name,
        )
    })
    .await
}

#[tauri::command]
pub async fn apply_catalog_resource_fork(
    scope_id: CatalogScopeId,
    plan_id: CatalogPlanId,
) -> Result<CatalogActionResult, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.apply_fork(&scope_id, &plan_id))
        .await
}

#[tauri::command]
pub async fn list_catalog_cloud_repositories(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
) -> Result<Vec<CatalogCloudRepository>, String> {
    StorageCatalogService::from_user_home()?
        .list_cloud_repositories(&scope_id, &storage_id, &resource_id)
        .await
}
#[tauri::command]
pub async fn list_catalog_cloud_shared(
    storage_id: StorageId,
) -> Result<Vec<CatalogCloudRepository>, String> {
    StorageCatalogService::from_user_home()?
        .list_cloud_shared(&storage_id)
        .await
}
#[tauri::command]
pub async fn list_catalog_cloud_starred(
    scope_id: Option<CatalogScopeId>,
    storage_id: StorageId,
    cursor: Option<String>,
) -> Result<mallard_infrastructure::managed_cloud::client::RepositoryListing, String> {
    StorageCatalogService::from_user_home()?
        .list_cloud_starred(scope_id.as_ref(), &storage_id, cursor.as_deref())
        .await
}
#[tauri::command]
pub async fn list_catalog_cloud_public(
    scope_id: Option<CatalogScopeId>,
    storage_id: StorageId,
    sort: String,
    cursor: Option<String>,
) -> Result<mallard_infrastructure::managed_cloud::client::RepositoryListing, String> {
    let sort = mallard_infrastructure::managed_cloud::client::PublicSort::parse(&sort)
        .ok_or("sort must be newest or stars")?;
    StorageCatalogService::from_user_home()?
        .list_cloud_public(scope_id.as_ref(), &storage_id, sort, cursor.as_deref())
        .await
}
#[tauri::command]
pub async fn resolve_catalog_cloud_repository(
    storage_id: StorageId,
    address: String,
) -> Result<CatalogCloudResolution, String> {
    StorageCatalogService::from_user_home()?
        .resolve_cloud_repository(&storage_id, &address)
        .await
}
/// The row on a storage that saved a repository, for a link that names it.
#[tauri::command]
pub async fn find_catalog_cloud_repository(
    storage_id: StorageId,
    repository_id: String,
) -> Result<Option<CatalogCloudPlace>, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.find_cloud_repository(&storage_id, &repository_id)
    })
    .await
}
#[tauri::command]
pub async fn join_catalog_cloud_repository(
    storage_id: StorageId,
    code: String,
) -> Result<CatalogCloudResolution, String> {
    StorageCatalogService::from_user_home()?
        .join_cloud_repository(&storage_id, &code)
        .await
}
#[tauri::command]
pub async fn connect_catalog_cloud_repository(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
    repository_id: String,
    expected_link_revision: u64,
) -> Result<(), String> {
    StorageCatalogService::from_user_home()?
        .connect_cloud_repository(
            &scope_id,
            &storage_id,
            &resource_id,
            &repository_id,
            expected_link_revision,
        )
        .await
}
#[tauri::command]
pub async fn create_catalog_cloud_repository(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
    expected_link_revision: u64,
) -> Result<CatalogCloudRepository, String> {
    StorageCatalogService::from_user_home()?
        .create_cloud_repository(&scope_id, &storage_id, &resource_id, expected_link_revision)
        .await
}
#[tauri::command]
pub async fn get_catalog_cloud_status(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
    repository_id: String,
) -> Result<mallard_infrastructure::managed_cloud::client::ManagedCloudStatus, String> {
    StorageCatalogService::from_user_home()?
        .cloud_status(&scope_id, &storage_id, &resource_id, &repository_id)
        .await
}
#[tauri::command]
pub async fn request_catalog_cloud_release(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
    repository_id: String,
    expected_revision: u64,
) -> Result<(), String> {
    StorageCatalogService::from_user_home()?
        .request_cloud_release(
            &scope_id,
            &storage_id,
            &resource_id,
            &repository_id,
            expected_revision,
        )
        .await
}
#[tauri::command]
pub async fn discard_catalog_cloud_draft(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
    repository_id: String,
    expected_revision: u64,
) -> Result<mallard_infrastructure::managed_cloud::client::DiscardedDraft, String> {
    StorageCatalogService::from_user_home()?
        .discard_cloud_draft(
            &scope_id,
            &storage_id,
            &resource_id,
            &repository_id,
            expected_revision,
        )
        .await
}
