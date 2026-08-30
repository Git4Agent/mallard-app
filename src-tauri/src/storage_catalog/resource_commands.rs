//! Tauri command adapters for storage-scoped Library resources.

use std::collections::BTreeMap;

use mallard_domain::StorageId;

use super::domain::{
    CatalogActionResult, CatalogLinkId, CatalogPlanActionId, CatalogPlanId,
    CatalogResourceForkPlan, CatalogResourceId, CatalogResourceKindTag, CatalogScopeId,
    CatalogScopePullPlan, CatalogScopePushPlan, LocalCatalogProfile, LocalCatalogProfileId,
    ProviderKey,
};
use super::service::{
    CatalogCapabilityDto, CatalogPullApproval, CatalogPushApproval, CatalogScopeDto,
    CatalogScopeLinkOutcomeDto, CatalogScopePullOutcomeDto, CatalogScopePushOutcomeDto,
    CatalogScopeResourcesDto, CatalogStorageRepositoryDto, StorageCatalogService,
};

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("catalog resource worker failed: {}", error))?
}

#[tauri::command]
pub async fn list_catalog_capabilities() -> Result<Vec<CatalogCapabilityDto>, String> {
    Ok(StorageCatalogService::capabilities())
}

#[tauri::command]
pub async fn list_catalog_resource_profiles() -> Result<Vec<LocalCatalogProfile>, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.list_profiles()).await
}

#[tauri::command]
pub async fn save_catalog_resource_profile(
    provider: ProviderKey,
    path: String,
) -> Result<LocalCatalogProfile, String> {
    run_blocking(move || StorageCatalogService::from_user_home()?.save_profile(provider, path))
        .await
}

#[tauri::command]
pub async fn rename_catalog_resource_profile(
    profile_id: LocalCatalogProfileId,
    display_name: String,
    expected_revision: u64,
) -> Result<LocalCatalogProfile, String> {
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
    resource_kind: CatalogResourceKindTag,
) -> Result<CatalogScopeDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.create_scope(provider, profile_id, resource_kind)
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
pub async fn get_catalog_pet_preview(
    scope_id: CatalogScopeId,
    storage_id: StorageId,
    resource_id: CatalogResourceId,
) -> Result<String, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.pet_preview(&scope_id, &storage_id, &resource_id)
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
) -> Result<CatalogScopePushOutcomeDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.apply_push(CatalogPushApproval {
            plan_id,
            selected_resource_ids,
            acknowledged_warning_digests,
        })
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
) -> Result<CatalogScopePullOutcomeDto, String> {
    run_blocking(move || {
        StorageCatalogService::from_user_home()?.apply_pull(CatalogPullApproval {
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
