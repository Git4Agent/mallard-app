//! Tauri surface for scope-rowed Library resources.
//!
//! Commands own catalog scopes (one menu row per provider profile and
//! resource kind), registrations, links, and per-link operations. They read
//! the neutral configured-storage list but never load a project registration,
//! project binding, thread, or repository bundle.

use bytes::Bytes;
use mallard_domain::{StorageConfig, StorageId};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_core::ConfiguredObjectStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::adapters::{capability_matrix, NativeProcessRunner, PluginProcessRunner};
use super::apply::{
    apply_resource_fork, execute_resource_pull_action, recover_catalog_operations,
    PullApplyEnvironment,
};
use super::domain::{
    provider_owns_kind, scope_owns, CatalogActionResult, CatalogCapturedWith, CatalogClaim,
    CatalogClaimPin, CatalogLinkId, CatalogLinkState, CatalogPlanActionId, CatalogPlanId,
    CatalogRegistrationId, CatalogResource, CatalogResourceForkPlan, CatalogResourceId,
    CatalogResourceKindTag, CatalogResourceLink, CatalogResourcePlanDocument,
    CatalogResourcePullPlan, CatalogResourcePushPlan, CatalogResourceRegistration,
    CatalogResourceRepositoryId, CatalogResourcesDocument, CatalogScope, CatalogScopeId,
    CodexPetFormat, LibraryHeadCoordinate, LibraryResourceTag, LocalCatalogProfile,
    LocalCatalogProfileId, LocalCatalogTargetId, ProviderKey, CATALOG_SCHEMA,
};
use super::inventory::{
    inventory_local, CatalogInventoryFilter, CatalogLocalInventory, LocalResourceRecord,
};
use super::persistence::CatalogStateRepository;
use super::profiles::{resolve_profile_context, validate_new_profile_home, ResolvedCatalogContext};
use super::resource_engine::{
    head_coordinate, storage_identity_for_config, verify_library_continuity, LibraryResourceEngine,
    ObservedLibraryHead, PublishLibraryResourceRequest, RemoteLibraryResource,
};
use super::resource_planning::{
    build_resource_fork_plan, build_resource_pull_plan, build_resource_push_plan,
    derive_link_state, target_for_resource, validate_resource_pull_selection,
    ResourceForkPlanInputs, ResourcePullPlanInputs, ResourcePushPlanInputs,
};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn pull_outcome_accepts_remote(outcome: super::domain::CatalogActionOutcome) -> bool {
    !matches!(
        outcome,
        super::domain::CatalogActionOutcome::Failed | super::domain::CatalogActionOutcome::Partial
    )
}

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("catalog resource worker failed: {}", error))?
}

fn state_repository() -> Result<LocalStateRepository, String> {
    LocalStateRepository::from_user_home()
}

fn catalog_repository() -> Result<CatalogStateRepository, String> {
    CatalogStateRepository::from_user_home()
}

fn os_user_home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "could not resolve the user home directory".to_string())
}

fn storage_for(
    state: &LocalStateRepository,
    storage_id: &StorageId,
) -> Result<StorageConfig, String> {
    state
        .load_config()?
        .storages
        .into_iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(|| format!("unknown storage '{}'", storage_id))
}

fn engine_for(
    storage: &StorageConfig,
) -> Result<LibraryResourceEngine<ConfiguredObjectStore>, String> {
    storage.validate()?;
    Ok(LibraryResourceEngine::new(
        ConfiguredObjectStore::from_config(storage)?,
    ))
}

fn scope_label(resource: &CatalogResource) -> Option<String> {
    match resource {
        CatalogResource::AgentSkill(skill) => {
            Some(skill.descriptor.scope.display_label().to_string())
        }
        CatalogResource::CodexPetV1(_) => Some("v1".to_string()),
        CatalogResource::CodexPetV2(_) => Some("v2".to_string()),
        CatalogResource::PluginInstallIntent(_) => None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{:02x}", byte);
    }
    output
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogCapabilityDto {
    pub provider: ProviderKey,
    pub kinds: Vec<CatalogResourceKindTag>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogLocalCandidateDto {
    pub resource_id: CatalogResourceId,
    pub kind: CatalogResourceKindTag,
    pub provider: ProviderKey,
    pub display_name: String,
    pub scope_label: Option<String>,
    pub content_sha256: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogRemoteCandidateDto {
    pub repository_id: CatalogResourceRepositoryId,
    pub provider: ProviderKey,
    pub kind: CatalogResourceKindTag,
    pub resource_id: CatalogResourceId,
    pub display_name: String,
    pub scope_label: Option<String>,
    pub head_revision: u64,
    pub content_sha256: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogResourceLinkDto {
    pub link: CatalogResourceLink,
    pub storage_name: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogResourceDto {
    pub registration: CatalogResourceRegistration,
    pub scope_label: Option<String>,
    pub pet_format: Option<CodexPetFormat>,
    pub links: Vec<CatalogResourceLinkDto>,
}

/// One Catalog menu row plus the profile its label derives from.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeDto {
    pub scope: CatalogScope,
    pub profile: LocalCatalogProfile,
}

/// One registered resource inside a scope, with its local presence reported
/// honestly rather than inferred.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeResourceRowDto {
    pub resource: CatalogResourceDto,
    pub local_present: bool,
}

/// The complete content of one scope page: registered rows, locally
/// discovered but unregistered candidates, and — when a storage is being
/// browsed — verified stored resources not yet linked into this scope.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeResourcesDto {
    pub scope: CatalogScope,
    pub profile: LocalCatalogProfile,
    pub registered: Vec<CatalogScopeResourceRowDto>,
    pub discovered: Vec<CatalogLocalCandidateDto>,
    pub remote_candidates: Vec<CatalogRemoteCandidateDto>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeLinkOutcomeDto {
    pub resource: CatalogResourceDto,
    pub completion: String,
    pub link_id: CatalogLinkId,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogResourceLinkInspectionDto {
    pub registration_id: super::domain::CatalogRegistrationId,
    pub link_id: CatalogLinkId,
    pub storage_id: StorageId,
    pub repository_id: CatalogResourceRepositoryId,
    pub state: CatalogLinkState,
    pub local_present: bool,
    pub remote_present: bool,
    pub local_content_sha256: Option<String>,
    pub remote_content_sha256: Option<String>,
    pub remote_revision: Option<u64>,
    pub blocker: Option<String>,
    pub generated_at: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogResourcePushOutcomeDto {
    pub plan_id: CatalogPlanId,
    pub published: bool,
    pub revision: Option<u64>,
    pub message: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogResourcePullOutcomeDto {
    pub plan_id: CatalogPlanId,
    pub remote_accepted: bool,
    pub result: CatalogActionResult,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogResourceStatusDto {
    pub registration_id: super::domain::CatalogRegistrationId,
    pub link_id: Option<CatalogLinkId>,
    pub active_plan_ids: Vec<CatalogPlanId>,
    pub recovery_messages: Vec<String>,
}

/// One Link action inside a scope page. Exactly one of `local_resource_id`
/// (a discovered local resource, published as a new stored resource) and
/// `repository_id` (an existing stored resource on the storage, adopted for
/// review) identifies the resource.
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LinkCatalogScopeResourceRequest {
    pub scope_id: CatalogScopeId,
    pub storage_id: StorageId,
    pub local_resource_id: Option<CatalogResourceId>,
    pub repository_id: Option<CatalogResourceRepositoryId>,
}

fn local_candidates(inventory: &CatalogLocalInventory) -> Vec<CatalogLocalCandidateDto> {
    let mut candidates: Vec<_> = inventory
        .resources
        .values()
        .map(|record| CatalogLocalCandidateDto {
            resource_id: record.resource.resource_id(),
            kind: record.resource.kind_tag(),
            provider: record.resource.provider(),
            display_name: record.resource.display_name(),
            scope_label: scope_label(&record.resource),
            content_sha256: record.resource.content_sha256().to_string(),
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.resource_id.cmp(&right.resource_id))
    });
    candidates
}

struct ResolvedScope {
    scope: CatalogScope,
    profile: LocalCatalogProfile,
}

fn resolve_scope(
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
) -> Result<ResolvedScope, String> {
    let scopes = catalog.load_scopes()?;
    let scope = scopes
        .scope(scope_id)
        .ok_or_else(|| format!("unknown catalog scope '{}'", scope_id))?
        .clone();
    let profiles = catalog.load_profiles()?;
    let profile = profiles
        .profile(&scope.profile_id)
        .ok_or("this Catalog row lost its provider profile")?
        .clone();
    if profile.provider_key != scope.provider {
        return Err("this Catalog row's profile changed its provider".to_string());
    }
    Ok(ResolvedScope { scope, profile })
}

fn verified_remote_choices(
    storage: &StorageConfig,
    kind: CatalogResourceKindTag,
    provider: ProviderKey,
) -> Result<Vec<(RemoteLibraryResource, CatalogResource)>, String> {
    let engine = engine_for(storage)?;
    let mut choices = Vec::new();
    for listed in engine.list_resources()? {
        if listed.tag.provider != provider || listed.tag.resource_kind != kind {
            continue;
        }
        let Some((observed, manifest)) = engine.inspect(&listed.tag.repository_id)? else {
            return Err(format!(
                "stored resource '{}' has a tag but no head",
                listed.tag.display_name
            ));
        };
        if listed.head.as_ref() != Some(&observed.head)
            || manifest.resource.provider() != provider
            || manifest.resource.kind_tag() != listed.tag.resource_kind
            || manifest.resource.resource_id() != listed.tag.resource_id
            || manifest.resource.identity_sha256() != listed.tag.stable_identity_sha256
        {
            return Err(format!(
                "stored resource '{}' has inconsistent discovery metadata",
                listed.tag.display_name
            ));
        }
        choices.push((listed, manifest.resource));
    }
    choices.sort_by(|left, right| {
        left.0
            .tag
            .display_name
            .to_lowercase()
            .cmp(&right.0.tag.display_name.to_lowercase())
            .then_with(|| left.0.tag.repository_id.cmp(&right.0.tag.repository_id))
    });
    Ok(choices)
}

pub fn list_catalog_capabilities_core() -> Vec<CatalogCapabilityDto> {
    capability_matrix()
        .into_iter()
        .map(|(provider, kinds)| CatalogCapabilityDto { provider, kinds })
        .collect()
}

#[tauri::command]
pub async fn list_catalog_capabilities(
    _app: tauri::AppHandle,
) -> Result<Vec<CatalogCapabilityDto>, String> {
    Ok(list_catalog_capabilities_core())
}

#[tauri::command]
pub async fn list_catalog_resource_profiles(
    _app: tauri::AppHandle,
) -> Result<Vec<LocalCatalogProfile>, String> {
    run_blocking(move || Ok(catalog_repository()?.load_profiles()?.profiles)).await
}

pub fn save_catalog_resource_profile_with_repository(
    catalog: &CatalogStateRepository,
    provider: ProviderKey,
    display_name: String,
    path: String,
    now: u64,
) -> Result<LocalCatalogProfile, String> {
    let canonical = validate_new_profile_home(catalog.root(), &path)?;
    catalog.mutate_profiles(|document| {
        if document
            .profiles
            .iter()
            .any(|profile| profile.provider_key == provider && profile.configured_home == canonical)
        {
            return Err("a catalog resource profile for this home already exists".to_string());
        }
        let profile = LocalCatalogProfile {
            profile_id: LocalCatalogProfileId::generate()?,
            provider_key: provider,
            display_name,
            configured_home: canonical.to_string_lossy().to_string(),
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        profile.validate()?;
        document.profiles.push(profile.clone());
        Ok(profile)
    })
}

#[tauri::command]
pub async fn save_catalog_resource_profile(
    _app: tauri::AppHandle,
    provider: ProviderKey,
    display_name: String,
    path: String,
) -> Result<LocalCatalogProfile, String> {
    run_blocking(move || {
        save_catalog_resource_profile_with_repository(
            &catalog_repository()?,
            provider,
            display_name,
            path,
            now_secs(),
        )
    })
    .await
}

pub fn delete_catalog_resource_profile_with_repositories(
    catalog: &CatalogStateRepository,
    profile_id: &LocalCatalogProfileId,
    expected_revision: u64,
) -> Result<bool, String> {
    if catalog
        .load_resources()?
        .registrations
        .iter()
        .any(|registration| &registration.profile_id == profile_id)
    {
        return Err("this profile is used by a registered Library resource".to_string());
    }
    if catalog
        .load_scopes()?
        .scopes
        .iter()
        .any(|scope| &scope.profile_id == profile_id)
    {
        return Err("this profile is used by a Catalog menu row".to_string());
    }
    if catalog.list_resource_plans()?.iter().any(|plan| {
        let pins = match plan {
            CatalogResourcePlanDocument::Push(plan) => &plan.profile_pins,
            CatalogResourcePlanDocument::Pull(plan) => &plan.profile_pins,
            CatalogResourcePlanDocument::Fork(plan) => &plan.profile_pins,
        };
        pins.iter().any(|pin| &pin.profile_id == profile_id)
    }) {
        return Err("this profile is used by a Library review plan".to_string());
    }
    catalog.mutate_profiles(|document| {
        let Some(index) = document
            .profiles
            .iter()
            .position(|profile| &profile.profile_id == profile_id)
        else {
            return Ok(false);
        };
        if document.profiles[index].revision != expected_revision {
            return Err("the catalog resource profile changed; refresh first".to_string());
        }
        document.profiles.remove(index);
        Ok(true)
    })
}

#[tauri::command]
pub async fn delete_catalog_resource_profile(
    _app: tauri::AppHandle,
    profile_id: LocalCatalogProfileId,
    expected_revision: u64,
) -> Result<bool, String> {
    run_blocking(move || {
        delete_catalog_resource_profile_with_repositories(
            &catalog_repository()?,
            &profile_id,
            expected_revision,
        )
    })
    .await
}

fn resource_dto(
    state: &LocalStateRepository,
    document: &CatalogResourcesDocument,
    registration: &CatalogResourceRegistration,
) -> Result<CatalogResourceDto, String> {
    let config = state.load_config()?;
    let links = document
        .links_for(&registration.registration_id)
        .map(|link| {
            let name = config
                .storages
                .iter()
                .find(|storage| storage.id == link.storage_id)
                .map(|storage| storage.name.clone())
                .unwrap_or_else(|| "Missing storage".to_string());
            CatalogResourceLinkDto {
                link: link.clone(),
                storage_name: name,
            }
        })
        .collect();
    let (scope, pet_format) = match &registration.local_target {
        LocalCatalogTargetId::SkillDirectory { scope, .. } => {
            (Some(scope.display_label().to_string()), None)
        }
        LocalCatalogTargetId::PetDirectory { .. } => (
            Some(
                match registration.resource_kind {
                    CatalogResourceKindTag::CodexPetV1 => "v1",
                    CatalogResourceKindTag::CodexPetV2 => "v2",
                    _ => "pet",
                }
                .to_string(),
            ),
            registration.resource_kind.pet_format(),
        ),
        LocalCatalogTargetId::Plugin { .. } => (None, None),
    };
    Ok(CatalogResourceDto {
        registration: registration.clone(),
        scope_label: scope,
        pet_format,
        links,
    })
}

#[tauri::command]
pub async fn list_catalog_scopes(_app: tauri::AppHandle) -> Result<Vec<CatalogScopeDto>, String> {
    run_blocking(move || {
        let catalog = catalog_repository()?;
        let profiles = catalog.load_profiles()?;
        let mut scopes = catalog.load_scopes()?.scopes;
        scopes.sort_by_key(|scope| (scope.created_at, scope.scope_id.clone()));
        scopes
            .into_iter()
            .map(|scope| {
                let profile = profiles
                    .profile(&scope.profile_id)
                    .ok_or("a Catalog row lost its provider profile")?
                    .clone();
                Ok(CatalogScopeDto { scope, profile })
            })
            .collect()
    })
    .await
}

pub fn create_catalog_scope_with_repositories(
    catalog: &CatalogStateRepository,
    provider: ProviderKey,
    profile_id: LocalCatalogProfileId,
    resource_kind: CatalogResourceKindTag,
    now: u64,
) -> Result<CatalogScopeDto, String> {
    if !provider_owns_kind(provider, resource_kind) {
        return Err("this provider cannot host that resource kind".to_string());
    }
    let profiles = catalog.load_profiles()?;
    let profile = profiles
        .profile(&profile_id)
        .ok_or_else(|| format!("unknown catalog profile '{}'", profile_id))?
        .clone();
    if profile.provider_key != provider {
        return Err(format!(
            "catalog profile '{}' is a {} profile, not {}",
            profile_id,
            profile.provider_key.as_str(),
            provider.as_str()
        ));
    }
    let scope = catalog.mutate_scopes(|document| {
        if document
            .scopes
            .iter()
            .any(|scope| scope.profile_id == profile_id && scope.resource_kind == resource_kind)
        {
            return Err("this Catalog row already exists".to_string());
        }
        let scope = CatalogScope {
            scope_id: CatalogScopeId::generate()?,
            provider,
            profile_id: profile_id.clone(),
            resource_kind,
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        scope.validate()?;
        document.scopes.push(scope.clone());
        Ok(scope)
    })?;
    Ok(CatalogScopeDto { scope, profile })
}

/// Adding a row records a scope and nothing else: no inventory, no provider
/// CLI, no storage traffic.
#[tauri::command]
pub async fn create_catalog_scope(
    _app: tauri::AppHandle,
    provider: ProviderKey,
    profile_id: LocalCatalogProfileId,
    resource_kind: CatalogResourceKindTag,
) -> Result<CatalogScopeDto, String> {
    run_blocking(move || {
        create_catalog_scope_with_repositories(
            &catalog_repository()?,
            provider,
            profile_id,
            resource_kind,
            now_secs(),
        )
    })
    .await
}

pub fn delete_catalog_scope_with_repositories(
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    expected_revision: u64,
) -> Result<bool, String> {
    let scopes = catalog.load_scopes()?;
    let Some(scope) = scopes.scope(scope_id) else {
        return Ok(false);
    };
    if catalog
        .load_resources()?
        .registrations
        .iter()
        .any(|registration| scope_owns(scope, registration))
    {
        return Err("this row still owns registered resources; remove them first".to_string());
    }
    catalog.mutate_scopes(|document| {
        let Some(index) = document
            .scopes
            .iter()
            .position(|scope| &scope.scope_id == scope_id)
        else {
            return Ok(false);
        };
        if document.scopes[index].revision != expected_revision {
            return Err("this Catalog row changed; refresh first".to_string());
        }
        document.scopes.remove(index);
        Ok(true)
    })
}

#[tauri::command]
pub async fn delete_catalog_scope(
    _app: tauri::AppHandle,
    scope_id: CatalogScopeId,
    expected_revision: u64,
) -> Result<bool, String> {
    run_blocking(move || {
        delete_catalog_scope_with_repositories(&catalog_repository()?, &scope_id, expected_revision)
    })
    .await
}

pub fn list_catalog_scope_resources_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    scope_id: &CatalogScopeId,
    storage_id: Option<&StorageId>,
    user_home: &Path,
) -> Result<CatalogScopeResourcesDto, String> {
    let ResolvedScope { scope, profile } = resolve_scope(catalog, scope_id)?;
    let context = resolve_profile_context(
        &catalog.load_profiles()?,
        &scope.profile_id,
        scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory = inventory_local(
        &context,
        CatalogInventoryFilter::Kind(scope.resource_kind),
        runner,
    )
    .map_err(|blocker| blocker.to_string())?;

    let document = catalog.load_resources()?;
    let mut registered = Vec::new();
    let mut registered_resource_ids = BTreeSet::new();
    let mut linked_repositories = BTreeSet::new();
    for registration in &document.registrations {
        if !scope_owns(&scope, registration) {
            continue;
        }
        registered_resource_ids.insert(registration.resource_id.clone());
        for link in document.links_for(&registration.registration_id) {
            linked_repositories.insert((link.storage_id.clone(), link.repository_id.clone()));
        }
        registered.push(CatalogScopeResourceRowDto {
            resource: resource_dto(state, &document, registration)?,
            local_present: inventory
                .resources
                .get(&registration.resource_id)
                .is_some_and(|record| record.target == registration.local_target),
        });
    }
    registered.sort_by(|left, right| {
        left.resource
            .registration
            .display_name
            .to_lowercase()
            .cmp(&right.resource.registration.display_name.to_lowercase())
            .then_with(|| {
                left.resource
                    .registration
                    .registration_id
                    .cmp(&right.resource.registration.registration_id)
            })
    });

    let discovered = local_candidates(&inventory)
        .into_iter()
        .filter(|candidate| !registered_resource_ids.contains(&candidate.resource_id))
        .collect();

    let remote_candidates = match storage_id {
        Some(storage_id) => {
            let storage = storage_for(state, storage_id)?;
            verified_remote_choices(&storage, scope.resource_kind, scope.provider)?
                .into_iter()
                .filter(|(listed, _)| {
                    !linked_repositories
                        .contains(&(storage.id.clone(), listed.tag.repository_id.clone()))
                })
                .map(|(listed, resource)| CatalogRemoteCandidateDto {
                    repository_id: listed.tag.repository_id,
                    provider: resource.provider(),
                    kind: resource.kind_tag(),
                    resource_id: resource.resource_id(),
                    display_name: resource.display_name(),
                    scope_label: scope_label(&resource),
                    head_revision: listed.head.map(|head| head.revision).unwrap_or(0),
                    content_sha256: resource.content_sha256().to_string(),
                })
                .collect()
        }
        None => Vec::new(),
    };

    Ok(CatalogScopeResourcesDto {
        scope,
        profile,
        registered,
        discovered,
        remote_candidates,
    })
}

#[tauri::command]
pub async fn list_catalog_scope_resources(
    _app: tauri::AppHandle,
    scope_id: CatalogScopeId,
    storage_id: Option<StorageId>,
) -> Result<CatalogScopeResourcesDto, String> {
    run_blocking(move || {
        list_catalog_scope_resources_with_repositories(
            &state_repository()?,
            &catalog_repository()?,
            &NativeProcessRunner,
            &scope_id,
            storage_id.as_ref(),
            &os_user_home()?,
        )
    })
    .await
}

pub fn link_catalog_scope_resource_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    request: LinkCatalogScopeResourceRequest,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopeLinkOutcomeDto, String> {
    let ResolvedScope { scope, .. } = resolve_scope(catalog, &request.scope_id)?;
    let context = resolve_profile_context(
        &catalog.load_profiles()?,
        &scope.profile_id,
        scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory = inventory_local(
        &context,
        CatalogInventoryFilter::Kind(scope.resource_kind),
        runner,
    )
    .map_err(|blocker| blocker.to_string())?;
    let storage = storage_for(state, &request.storage_id)?;

    let adopted_remote = matches!(
        (&request.local_resource_id, &request.repository_id),
        (None, Some(_))
    );
    let (repository_id, local_record, remote, accepted_head, accepted_content) =
        match (&request.local_resource_id, &request.repository_id) {
            (Some(resource_id), None) => {
                let record = inventory
                    .resources
                    .get(resource_id)
                    .ok_or("the selected local resource is no longer available")?;
                (
                    CatalogResourceRepositoryId::generate()?,
                    Some(record),
                    None,
                    None,
                    None,
                )
            }
            (None, Some(repository_id)) => {
                let engine = engine_for(&storage)?;
                let (tag, _) = engine
                    .read_tag(repository_id)?
                    .ok_or("the selected stored resource disappeared")?;
                let (observed, manifest) = engine
                    .inspect(repository_id)?
                    .ok_or("the selected stored resource disappeared")?;
                if tag.provider != scope.provider
                    || tag.resource_kind != scope.resource_kind
                    || manifest.resource.provider() != scope.provider
                    || manifest.resource.kind_tag() != scope.resource_kind
                    || manifest.resource.resource_id() != tag.resource_id
                {
                    return Err("the selected stored resource is incompatible".to_string());
                }
                let coordinate =
                    head_coordinate(&observed.head, &storage_identity_for_config(&storage));
                let local = inventory.resources.get(&manifest.resource.resource_id());
                let exact_match = local.is_some_and(|record| {
                    record.resource.content_sha256() == manifest.resource.content_sha256()
                });
                (
                    repository_id.clone(),
                    local,
                    Some(manifest.resource.clone()),
                    exact_match.then_some(coordinate),
                    exact_match.then(|| manifest.resource.content_sha256().to_string()),
                )
            }
            _ => return Err("choose exactly one local or stored resource".to_string()),
        };

    let identity_resource = local_record
        .map(|record| record.resource.clone())
        .or(remote)
        .ok_or("link setup has neither a local nor stored resource")?;
    if identity_resource.kind_tag() != scope.resource_kind
        || identity_resource.provider() != scope.provider
    {
        return Err("the selected resource is incompatible with this row".to_string());
    }
    let (target, _) = target_for_resource(&context, &identity_resource)?;
    if let Some(local) = local_record {
        if local.target != target {
            return Err("the selected local resource resolves to a different target".to_string());
        }
    }

    // Registration reuse plus the new link land in one mutation under one
    // lock, so no journal is needed to make the pair atomic.
    let profile_id = scope.profile_id.clone();
    let resource_kind = scope.resource_kind;
    let registration = catalog.mutate_resources(|document| {
        let existing = document
            .registrations
            .iter()
            .find(|registration| {
                registration.profile_id == profile_id
                    && registration.resource_kind == resource_kind
                    && registration.local_target == target
            })
            .cloned();
        let registration = match existing {
            Some(existing) => {
                if existing.resource_id != identity_resource.resource_id()
                    || existing.provider != identity_resource.provider()
                {
                    return Err("this target is registered to a different resource".to_string());
                }
                existing
            }
            None => {
                let registration = CatalogResourceRegistration {
                    registration_id: CatalogRegistrationId::generate()?,
                    provider: identity_resource.provider(),
                    profile_id: profile_id.clone(),
                    resource_kind,
                    resource_id: identity_resource.resource_id(),
                    local_target: target.clone(),
                    display_name: identity_resource.display_name(),
                    revision: 1,
                    created_at: now,
                    updated_at: now,
                };
                document.registrations.push(registration.clone());
                registration
            }
        };
        document.links.push(CatalogResourceLink {
            link_id: CatalogLinkId::generate()?,
            registration_id: registration.registration_id.clone(),
            storage_id: storage.id.clone(),
            repository_id: repository_id.clone(),
            accepted_head: accepted_head.clone(),
            accepted_content_sha256: accepted_content.clone(),
            revision: 1,
            created_at: now,
            updated_at: now,
        });
        Ok(registration)
    })?;

    let document = catalog.load_resources()?;
    let link_id = document
        .links_for(&registration.registration_id)
        .find(|link| link.storage_id == storage.id && link.repository_id == repository_id)
        .map(|link| link.link_id.clone())
        .ok_or("the new catalog link disappeared")?;
    let resource = resource_dto(
        state,
        &document,
        document
            .registration(&registration.registration_id)
            .ok_or("the new catalog registration disappeared")?,
    )?;
    Ok(CatalogScopeLinkOutcomeDto {
        completion: if accepted_head.is_none() && adopted_remote {
            "review_pull".to_string()
        } else {
            "open".to_string()
        },
        link_id,
        resource,
    })
}

#[tauri::command]
pub async fn link_catalog_scope_resource(
    _app: tauri::AppHandle,
    request: LinkCatalogScopeResourceRequest,
) -> Result<CatalogScopeLinkOutcomeDto, String> {
    run_blocking(move || {
        link_catalog_scope_resource_with_repositories(
            &state_repository()?,
            &catalog_repository()?,
            &NativeProcessRunner,
            request,
            &os_user_home()?,
            now_secs(),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_catalog_resource(
    _app: tauri::AppHandle,
    registration_id: super::domain::CatalogRegistrationId,
) -> Result<CatalogResourceDto, String> {
    run_blocking(move || {
        let state = state_repository()?;
        let catalog = catalog_repository()?;
        let document = catalog.load_resources()?;
        let registration = document
            .registration(&registration_id)
            .ok_or_else(|| format!("unknown catalog registration '{}'", registration_id))?;
        resource_dto(&state, &document, registration)
    })
    .await
}

fn discard_plans_for(
    catalog: &CatalogStateRepository,
    registration_id: &super::domain::CatalogRegistrationId,
    link_id: Option<&CatalogLinkId>,
) -> Result<(), String> {
    for plan in catalog.list_resource_plans()? {
        let matches = match &plan {
            CatalogResourcePlanDocument::Push(plan) => {
                &plan.registration_id == registration_id
                    && link_id.is_none_or(|link_id| &plan.link_id == link_id)
            }
            CatalogResourcePlanDocument::Pull(plan) => {
                &plan.registration_id == registration_id
                    && link_id.is_none_or(|link_id| &plan.link_id == link_id)
            }
            CatalogResourcePlanDocument::Fork(plan) => {
                &plan.registration_id == registration_id && link_id.is_none()
            }
        };
        if matches {
            let _ = catalog.discard_plan(plan.plan_id());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn unlink_catalog_resource(
    _app: tauri::AppHandle,
    registration_id: super::domain::CatalogRegistrationId,
    link_id: CatalogLinkId,
    expected_revision: u64,
) -> Result<bool, String> {
    run_blocking(move || {
        let catalog = catalog_repository()?;
        let document = catalog.load_resources()?;
        let link = document
            .link(&link_id)
            .ok_or_else(|| format!("unknown catalog link '{}'", link_id))?;
        if link.registration_id != registration_id {
            return Err("the link belongs to another resource".to_string());
        }
        if link.revision != expected_revision {
            return Err("the catalog link changed; refresh before unlinking".to_string());
        }
        let registration = document
            .registration(&registration_id)
            .ok_or("the link lost its Library registration")?;
        let active_operation = catalog
            .load_operations()?
            .operations
            .into_iter()
            .any(|operation| {
                !operation.phase.is_terminal()
                    && operation.storage_id.as_ref() == Some(&link.storage_id)
                    && operation.target == registration.local_target
            });
        if active_operation {
            return Err("an operation for this link is still in progress".to_string());
        }
        let storage_id = link.storage_id.clone();
        let local_target = registration.local_target.clone();
        discard_plans_for(&catalog, &registration_id, Some(&link_id))?;
        catalog.mutate_materializations(|claims| {
            claims
                .claims
                .retain(|claim| claim.storage_id != storage_id || claim.target != local_target);
            Ok(())
        })?;
        catalog.mutate_preservations(|preservations| {
            preservations.witnesses.retain(|witness| {
                witness.storage_id != storage_id || witness.source_target != local_target
            });
            Ok(())
        })?;
        catalog.mutate_operations(|operations| {
            operations.operations.retain(|operation| {
                !(operation.phase.is_terminal()
                    && operation.storage_id.as_ref() == Some(&storage_id)
                    && operation.target == local_target)
            });
            Ok(())
        })?;
        catalog.mutate_resources(|document| {
            let before = document.links.len();
            document.links.retain(|link| link.link_id != link_id);
            Ok(document.links.len() != before)
        })
    })
    .await
}

#[tauri::command]
pub async fn remove_catalog_resource(
    _app: tauri::AppHandle,
    registration_id: super::domain::CatalogRegistrationId,
    expected_revision: u64,
) -> Result<bool, String> {
    run_blocking(move || {
        let catalog = catalog_repository()?;
        let document = catalog.load_resources()?;
        let registration = document
            .registration(&registration_id)
            .ok_or_else(|| format!("unknown catalog registration '{}'", registration_id))?;
        if registration.revision != expected_revision {
            return Err("the catalog resource changed; refresh before removing".to_string());
        }
        if document.links_for(&registration_id).next().is_some() {
            return Err("unlink every storage before removing this resource".to_string());
        }
        let active_operation = catalog
            .load_operations()?
            .operations
            .into_iter()
            .any(|operation| {
                !operation.phase.is_terminal() && operation.target == registration.local_target
            });
        if active_operation {
            return Err("an operation for this resource is still in progress".to_string());
        }
        let local_target = registration.local_target.clone();
        discard_plans_for(&catalog, &registration_id, None)?;
        catalog.mutate_operations(|operations| {
            operations.operations.retain(|operation| {
                !(operation.phase.is_terminal() && operation.target == local_target)
            });
            Ok(())
        })?;
        catalog.mutate_resources(|document| {
            let before = document.registrations.len();
            document
                .registrations
                .retain(|registration| registration.registration_id != registration_id);
            Ok(document.registrations.len() != before)
        })
    })
    .await
}

struct ResolvedResourceWorkspace {
    registration: CatalogResourceRegistration,
    link: CatalogResourceLink,
    storage: StorageConfig,
    context: ResolvedCatalogContext,
    inventory: CatalogLocalInventory,
    observed: Option<ObservedLibraryHead>,
    remote_resource: Option<CatalogResource>,
    state: CatalogLinkState,
}

fn resolve_resource_workspace(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    link_id: &CatalogLinkId,
    user_home: &Path,
) -> Result<ResolvedResourceWorkspace, String> {
    let document = catalog.load_resources()?;
    let link = document
        .link(link_id)
        .ok_or_else(|| format!("unknown catalog link '{}'", link_id))?
        .clone();
    let registration = document
        .registration(&link.registration_id)
        .ok_or("the catalog link lost its registration")?
        .clone();
    let storage = storage_for(state, &link.storage_id)?;
    let profiles = catalog.load_profiles()?;
    let context = resolve_profile_context(
        &profiles,
        &registration.profile_id,
        registration.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory = inventory_local(&context, CatalogInventoryFilter::Everything, runner)
        .map_err(|blocker| blocker.to_string())?;
    let local = inventory.resources.get(&registration.resource_id);
    if local.is_some_and(|record| record.target != registration.local_target) {
        return Err(
            "the linked local resource target no longer matches its registration".to_string(),
        );
    }
    let engine = engine_for(&storage)?;
    let observed = engine.read_head(&link.repository_id)?;
    verify_library_continuity(
        link.accepted_head.as_ref(),
        observed.as_ref().map(|observed| &observed.head),
        &link.repository_id,
        &storage_identity_for_config(&storage),
    )?;
    let remote_resource = match &observed {
        Some(observed) => {
            let (tag, _) = engine
                .read_tag(&link.repository_id)?
                .ok_or("the linked resource repository has no discovery tag")?;
            let manifest = engine.read_manifest(&observed.head)?;
            if tag.provider != registration.provider
                || tag.resource_kind != registration.resource_kind
                || tag.resource_id != registration.resource_id
                || manifest.resource.resource_id() != registration.resource_id
                || manifest.resource.kind_tag() != registration.resource_kind
                || manifest.resource.provider() != registration.provider
            {
                return Err(
                    "the linked repository identity does not match its registration".to_string(),
                );
            }
            Some(manifest.resource)
        }
        None => None,
    };
    let state_value = derive_link_state(
        link.accepted_content_sha256.as_deref(),
        local.map(|record| record.resource.content_sha256()),
        remote_resource
            .as_ref()
            .map(CatalogResource::content_sha256),
        link.accepted_head.is_some(),
    );
    Ok(ResolvedResourceWorkspace {
        registration,
        link,
        storage,
        context,
        inventory,
        observed,
        remote_resource,
        state: state_value,
    })
}

fn inspect_link_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    link_id: &CatalogLinkId,
    user_home: &Path,
    now: u64,
) -> Result<CatalogResourceLinkInspectionDto, String> {
    match resolve_resource_workspace(state, catalog, runner, link_id, user_home) {
        Ok(workspace) => {
            let local = workspace
                .inventory
                .resources
                .get(&workspace.registration.resource_id);
            Ok(CatalogResourceLinkInspectionDto {
                registration_id: workspace.registration.registration_id,
                link_id: workspace.link.link_id,
                storage_id: workspace.link.storage_id,
                repository_id: workspace.link.repository_id,
                state: workspace.state,
                local_present: local.is_some(),
                remote_present: workspace.remote_resource.is_some(),
                local_content_sha256: local
                    .map(|record| record.resource.content_sha256().to_string()),
                remote_content_sha256: workspace
                    .remote_resource
                    .as_ref()
                    .map(|resource| resource.content_sha256().to_string()),
                remote_revision: workspace.observed.map(|observed| observed.head.revision),
                blocker: None,
                generated_at: now,
            })
        }
        Err(error) => {
            let document = catalog.load_resources()?;
            let link = document
                .link(link_id)
                .ok_or_else(|| format!("unknown catalog link '{}'", link_id))?;
            Ok(CatalogResourceLinkInspectionDto {
                registration_id: link.registration_id.clone(),
                link_id: link.link_id.clone(),
                storage_id: link.storage_id.clone(),
                repository_id: link.repository_id.clone(),
                state: CatalogLinkState::Blocked,
                local_present: false,
                remote_present: false,
                local_content_sha256: None,
                remote_content_sha256: None,
                remote_revision: None,
                blocker: Some(error),
                generated_at: now,
            })
        }
    }
}

#[tauri::command]
pub async fn inspect_catalog_resource_link(
    _app: tauri::AppHandle,
    link_id: CatalogLinkId,
    force_revalidate: bool,
) -> Result<CatalogResourceLinkInspectionDto, String> {
    run_blocking(move || {
        let state = state_repository()?;
        let catalog = catalog_repository()?;
        if force_revalidate {
            let document = catalog.load_resources()?;
            if let Some(link) = document.link(&link_id) {
                discard_plans_for(&catalog, &link.registration_id, Some(&link_id))?;
            }
        }
        inspect_link_with_repositories(
            &state,
            &catalog,
            &NativeProcessRunner,
            &link_id,
            &os_user_home()?,
            now_secs(),
        )
    })
    .await
}

fn discard_link_plans(
    catalog: &CatalogStateRepository,
    link_id: &CatalogLinkId,
    except: Option<&CatalogPlanId>,
) -> Result<(), String> {
    for plan in catalog.list_resource_plans()? {
        let plan_link = match &plan {
            CatalogResourcePlanDocument::Push(plan) => Some(&plan.link_id),
            CatalogResourcePlanDocument::Pull(plan) => Some(&plan.link_id),
            CatalogResourcePlanDocument::Fork(_) => None,
        };
        if plan_link == Some(link_id) && except != Some(plan.plan_id()) {
            let _ = catalog.discard_plan(plan.plan_id());
        }
    }
    Ok(())
}

fn prepare_push_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    link_id: &CatalogLinkId,
    user_home: &Path,
    now: u64,
) -> Result<CatalogResourcePushPlan, String> {
    let workspace = resolve_resource_workspace(state, catalog, runner, link_id, user_home)?;
    let plan = build_resource_push_plan(ResourcePushPlanInputs {
        registration: &workspace.registration,
        link: &workspace.link,
        storage_kind: workspace.storage.kind,
        storage_identity_sha256: storage_identity_for_config(&workspace.storage),
        context: &workspace.context,
        inventory: &workspace.inventory,
        state: workspace.state,
        observed_head: workspace.observed.as_ref().map(|observed| {
            head_coordinate(
                &observed.head,
                &storage_identity_for_config(&workspace.storage),
            )
        }),
        now,
    })?;
    discard_link_plans(catalog, link_id, None)?;
    catalog.save_resource_plan(&CatalogResourcePlanDocument::Push(plan.clone()))?;
    Ok(plan)
}

#[tauri::command]
pub async fn prepare_catalog_resource_push(
    _app: tauri::AppHandle,
    link_id: CatalogLinkId,
) -> Result<CatalogResourcePushPlan, String> {
    run_blocking(move || {
        prepare_push_with_repositories(
            &state_repository()?,
            &catalog_repository()?,
            &NativeProcessRunner,
            &link_id,
            &os_user_home()?,
            now_secs(),
        )
    })
    .await
}

fn payloads_for_record(record: &LocalResourceRecord) -> BTreeMap<String, Bytes> {
    let mut payloads = BTreeMap::new();
    if let Some(tree) = &record.skill_tree {
        for file in tree.files.values() {
            payloads.insert(sha256_hex(&file.bytes), Bytes::from(file.bytes.clone()));
        }
    }
    if let Some(pet) = &record.pet {
        payloads.insert(
            sha256_hex(&pet.atlas_bytes),
            Bytes::from(pet.atlas_bytes.clone()),
        );
    }
    payloads
}

fn observed_matches(
    pinned: Option<&LibraryHeadCoordinate>,
    observed: Option<&ObservedLibraryHead>,
    storage_identity: &str,
) -> bool {
    match (pinned, observed) {
        (None, None) => true,
        (Some(pinned), Some(observed)) => {
            &head_coordinate(&observed.head, storage_identity) == pinned
        }
        _ => false,
    }
}

#[tauri::command]
pub async fn publish_catalog_resource(
    _app: tauri::AppHandle,
    link_id: CatalogLinkId,
    plan_id: CatalogPlanId,
    include: bool,
    acknowledged_warning_digests: Vec<String>,
) -> Result<CatalogResourcePushOutcomeDto, String> {
    run_blocking(move || {
        let state = state_repository()?;
        let catalog = catalog_repository()?;
        let CatalogResourcePlanDocument::Push(plan) = catalog.load_resource_plan(&plan_id)? else {
            return Err("the referenced plan is not a Push review".to_string());
        };
        if plan.link_id != link_id {
            return Err("the Push review belongs to another link".to_string());
        }
        let now = now_secs();
        if now < plan.created_at || now > plan.expires_at {
            return Err("the Push review expired; review again".to_string());
        }
        if !include {
            catalog.discard_plan(&plan_id)?;
            return Ok(CatalogResourcePushOutcomeDto {
                plan_id,
                published: false,
                revision: plan.observed_head.map(|head| head.revision),
                message: "Nothing was pushed".to_string(),
            });
        }
        if !plan.row.selectable {
            return Err(plan
                .row
                .blocked_reason
                .clone()
                .unwrap_or_else(|| "this resource has nothing safe to Push".to_string()));
        }
        let acknowledged: BTreeSet<_> = acknowledged_warning_digests.into_iter().collect();
        for warning in &plan.row.warnings {
            if !acknowledged.contains(&warning.digest) {
                return Err(format!("unacknowledged warning: {}", warning.message));
            }
        }
        let workspace = resolve_resource_workspace(
            &state,
            &catalog,
            &NativeProcessRunner,
            &link_id,
            &os_user_home()?,
        )?;
        if workspace.registration.revision != plan.registration_revision
            || workspace.link.revision != plan.link_revision
        {
            return Err("the resource or link changed after review".to_string());
        }
        let storage_identity = storage_identity_for_config(&workspace.storage);
        if storage_identity != plan.storage_identity_sha256
            || !observed_matches(
                plan.observed_head.as_ref(),
                workspace.observed.as_ref(),
                &storage_identity,
            )
        {
            return Err("the storage head changed after review".to_string());
        }
        workspace
            .context
            .verify_pins(&plan.profile_pins, &plan.skill_root_pins)?;
        if workspace.inventory.source_stamp != plan.source_stamp {
            return Err("the local provider inventory changed after review".to_string());
        }
        let record = workspace
            .inventory
            .resources
            .get(&workspace.registration.resource_id)
            .ok_or("the local resource disappeared after review")?;
        if record.resource != plan.resource {
            return Err("the local resource changed after review".to_string());
        }
        let engine = engine_for(&workspace.storage)?;
        let tag = match engine.read_tag(&workspace.link.repository_id)? {
            Some((tag, _)) => tag,
            None => LibraryResourceTag {
                schema_version: CATALOG_SCHEMA,
                repository_id: workspace.link.repository_id.clone(),
                provider: record.resource.provider(),
                resource_kind: record.resource.kind_tag(),
                resource_id: record.resource.resource_id(),
                stable_identity_sha256: record.resource.identity_sha256(),
                display_name: record.resource.display_name(),
                scope_label: scope_label(&record.resource),
                created_at: workspace.link.created_at,
            },
        };
        let host = state.load_or_create_host_identity()?;
        let published = engine.publish(PublishLibraryResourceRequest {
            repository_id: workspace.link.repository_id.clone(),
            tag,
            observed: workspace.observed,
            resource: record.resource.clone(),
            payloads: payloads_for_record(record),
            captured_with: CatalogCapturedWith {
                app: "mallard".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            published_by_origin: super::domain::derive_library_origin(
                host.host_id.as_str(),
                workspace.link.link_id.as_str(),
                &workspace.link.repository_id,
            )?,
            updated_at: now,
        })?;
        let coordinate = head_coordinate(&published.head, &storage_identity);
        catalog.mutate_resources(|document| {
            let link = document
                .links
                .iter_mut()
                .find(|link| link.link_id == link_id)
                .ok_or("the catalog link disappeared while publishing")?;
            if link.revision != plan.link_revision {
                return Err("the catalog link changed while publishing".to_string());
            }
            link.accepted_head = Some(coordinate);
            link.accepted_content_sha256 =
                Some(published.manifest.resource.content_sha256().to_string());
            link.revision = link.revision.saturating_add(1);
            link.updated_at = now;
            Ok(())
        })?;
        catalog.discard_plan(&plan_id)?;
        Ok(CatalogResourcePushOutcomeDto {
            plan_id,
            published: published.published,
            revision: Some(published.head.revision),
            message: if published.published {
                format!("Published resource revision {}", published.head.revision)
            } else {
                "The linked storage already had this content".to_string()
            },
        })
    })
    .await
}

fn name_taken_index<'a>(
    inventory: &'a CatalogLocalInventory,
) -> impl Fn(&LocalCatalogTargetId, &str) -> bool + 'a {
    move |target, candidate| {
        let folded = candidate.to_lowercase();
        let inventoried =
            inventory
                .resources
                .values()
                .any(|record| match (&record.target, target) {
                    (
                        LocalCatalogTargetId::SkillDirectory {
                            canonical_root: left_root,
                            install_directory,
                            ..
                        },
                        LocalCatalogTargetId::SkillDirectory {
                            canonical_root: right_root,
                            ..
                        },
                    ) => left_root == right_root && install_directory.to_lowercase() == folded,
                    (
                        LocalCatalogTargetId::PetDirectory {
                            canonical_home: left_home,
                            pet_id,
                        },
                        LocalCatalogTargetId::PetDirectory {
                            canonical_home: right_home,
                            ..
                        },
                    ) => left_home == right_home && pet_id.to_lowercase() == folded,
                    _ => false,
                });
        if inventoried {
            return true;
        }
        let parent = match target {
            LocalCatalogTargetId::SkillDirectory { canonical_root, .. } => {
                Path::new(canonical_root).to_path_buf()
            }
            LocalCatalogTargetId::PetDirectory { canonical_home, .. } => {
                Path::new(canonical_home).join("pets")
            }
            LocalCatalogTargetId::Plugin { .. } => return true,
        };
        match fs::read_dir(parent) {
            Ok(entries) => entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.to_lowercase() == folded)
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true,
        }
    }
}

fn claim_pins_for_target(
    claims: &[CatalogClaim],
    current_storage_id: &StorageId,
    target: &LocalCatalogTargetId,
    incoming_content_sha256: &str,
) -> Vec<CatalogClaimPin> {
    let mut pins: Vec<_> = claims
        .iter()
        .filter(|claim| {
            claim.storage_id != *current_storage_id
                && claim.target == *target
                && claim.content_sha256 != incoming_content_sha256
        })
        .map(|claim| CatalogClaimPin {
            storage_id: claim.storage_id.clone(),
            content_sha256: claim.content_sha256.clone(),
        })
        .collect();
    pins.sort();
    pins
}

fn conflicting_claim_pins(
    catalog: &CatalogStateRepository,
    inventory: &CatalogLocalInventory,
    storage_id: &StorageId,
    target: &LocalCatalogTargetId,
    incoming_content_sha256: &str,
) -> Result<Vec<CatalogClaimPin>, String> {
    let target_is_occupied = inventory
        .resources
        .values()
        .any(|record| record.target == *target);
    if !target_is_occupied {
        return Ok(Vec::new());
    }
    Ok(claim_pins_for_target(
        &catalog.load_materializations()?.claims,
        storage_id,
        target,
        incoming_content_sha256,
    ))
}

fn prepare_pull_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    link_id: &CatalogLinkId,
    requested_copy_name: Option<&str>,
    user_home: &Path,
    now: u64,
) -> Result<CatalogResourcePullPlan, String> {
    let workspace = resolve_resource_workspace(state, catalog, runner, link_id, user_home)?;
    let remote = workspace
        .remote_resource
        .as_ref()
        .ok_or("the linked repository has no resource to Pull")?;
    let observed = workspace
        .observed
        .as_ref()
        .ok_or("the linked repository has no head")?;
    let storage_identity = storage_identity_for_config(&workspace.storage);
    let name_taken = name_taken_index(&workspace.inventory);
    let (target, _) = target_for_resource(&workspace.context, remote)?;
    let conflicting_claims = conflicting_claim_pins(
        catalog,
        &workspace.inventory,
        &workspace.link.storage_id,
        &target,
        remote.content_sha256(),
    )?;
    let state =
        if !conflicting_claims.is_empty() && workspace.state == CatalogLinkState::StorageChanged {
            CatalogLinkState::Diverged
        } else {
            workspace.state
        };
    let plan = build_resource_pull_plan(ResourcePullPlanInputs {
        registration: &workspace.registration,
        link: &workspace.link,
        storage_identity_sha256: storage_identity.clone(),
        context: &workspace.context,
        inventory: &workspace.inventory,
        remote_resource: remote,
        head: head_coordinate(&observed.head, &storage_identity),
        state,
        conflicting_claims,
        requested_copy_name,
        name_taken: &name_taken,
        now,
    })?;
    discard_link_plans(catalog, link_id, None)?;
    catalog.save_resource_plan(&CatalogResourcePlanDocument::Pull(plan.clone()))?;
    Ok(plan)
}

#[tauri::command]
pub async fn prepare_catalog_resource_pull(
    _app: tauri::AppHandle,
    link_id: CatalogLinkId,
    requested_copy_name: Option<String>,
) -> Result<CatalogResourcePullPlan, String> {
    run_blocking(move || {
        prepare_pull_with_repositories(
            &state_repository()?,
            &catalog_repository()?,
            &NativeProcessRunner,
            &link_id,
            requested_copy_name.as_deref(),
            &os_user_home()?,
            now_secs(),
        )
    })
    .await
}

#[tauri::command]
pub async fn apply_catalog_resource_pull(
    _app: tauri::AppHandle,
    link_id: CatalogLinkId,
    plan_id: CatalogPlanId,
    approved_action_id: Option<CatalogPlanActionId>,
    destructive_acknowledged: bool,
) -> Result<CatalogResourcePullOutcomeDto, String> {
    run_blocking(move || {
        let state = state_repository()?;
        let catalog = catalog_repository()?;
        let CatalogResourcePlanDocument::Pull(plan) = catalog.load_resource_plan(&plan_id)? else {
            return Err("the referenced plan is not a Pull review".to_string());
        };
        if plan.link_id != link_id {
            return Err("the Pull review belongs to another link".to_string());
        }
        let now = now_secs();
        if now < plan.created_at || now > plan.expires_at {
            return Err("the Pull review expired; review again".to_string());
        }
        let workspace = resolve_resource_workspace(
            &state,
            &catalog,
            &NativeProcessRunner,
            &link_id,
            &os_user_home()?,
        )?;
        if workspace.registration.revision != plan.registration_revision
            || workspace.link.revision != plan.link_revision
            || storage_identity_for_config(&workspace.storage) != plan.storage_identity_sha256
        {
            return Err("the resource, link, or storage changed after review".to_string());
        }
        workspace
            .context
            .verify_pins(&plan.profile_pins, &plan.skill_root_pins)?;
        let observed = workspace
            .observed
            .as_ref()
            .ok_or("the linked repository head disappeared")?;
        if head_coordinate(&observed.head, &plan.storage_identity_sha256) != plan.head
            || workspace.remote_resource.as_ref() != Some(&plan.resource)
        {
            return Err("the stored resource changed after review".to_string());
        }
        let approved = validate_resource_pull_selection(
            &plan,
            approved_action_id.as_ref(),
            destructive_acknowledged,
        )?;
        if let Some(action) = approved.first().copied() {
            let current_claims = conflicting_claim_pins(
                &catalog,
                &workspace.inventory,
                &workspace.link.storage_id,
                &action.target,
                &action.incoming_content_sha256,
            )?;
            if current_claims != plan.conflicting_claims {
                return Err(
                    "cross-storage claims changed after review; refresh and review Pull again"
                        .to_string(),
                );
            }
            if !current_claims.is_empty()
                && action.kind.preservation().is_none()
                && !action.kind.is_destructive_overwrite()
            {
                return Err(
                    "this target is claimed by another storage at a different version; choose preservation, acknowledged overwrite, or Skip"
                        .to_string(),
                );
            }
        }
        let engine = engine_for(&workspace.storage)?;
        let fetch = |sha256: &str, size: u64| {
            engine.fetch_payload(&plan.repository_id, sha256, size)
        };
        let environment = PullApplyEnvironment {
            repository: &catalog,
            runner: &NativeProcessRunner,
            fetch_payload: &fetch,
        };
        let result = execute_resource_pull_action(
            &environment,
            &plan,
            approved.first().copied(),
            now,
        );
        let remote_accepted = pull_outcome_accepts_remote(result.outcome);
        if remote_accepted {
            catalog.mutate_resources(|document| {
                let link = document
                    .links
                    .iter_mut()
                    .find(|link| link.link_id == link_id)
                    .ok_or("the catalog link disappeared while accepting Pull")?;
                if link.revision != plan.link_revision {
                    return Err("the catalog link changed while accepting Pull".to_string());
                }
                link.accepted_head = Some(plan.head.clone());
                link.accepted_content_sha256 = Some(plan.resource.content_sha256().to_string());
                link.revision = link.revision.saturating_add(1);
                link.updated_at = now;
                Ok(())
            })?;
        }
        catalog.discard_plan(&plan_id)?;
        Ok(CatalogResourcePullOutcomeDto {
            plan_id,
            remote_accepted,
            result,
        })
    })
    .await
}

fn prepare_fork_with_repositories(
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    registration_id: &super::domain::CatalogRegistrationId,
    requested_name: &str,
    user_home: &Path,
    now: u64,
) -> Result<CatalogResourceForkPlan, String> {
    let document = catalog.load_resources()?;
    let registration = document
        .registration(registration_id)
        .ok_or_else(|| format!("unknown catalog registration '{}'", registration_id))?
        .clone();
    let context = resolve_profile_context(
        &catalog.load_profiles()?,
        &registration.profile_id,
        registration.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory = inventory_local(&context, CatalogInventoryFilter::Everything, runner)
        .map_err(|blocker| blocker.to_string())?;
    let name_taken = name_taken_index(&inventory);
    let plan = build_resource_fork_plan(ResourceForkPlanInputs {
        registration: &registration,
        context: &context,
        inventory: &inventory,
        requested_name,
        name_taken: &name_taken,
        now,
    })?;
    for existing in catalog.list_resource_plans()? {
        if matches!(
            &existing,
            CatalogResourcePlanDocument::Fork(existing)
                if existing.registration_id == *registration_id
        ) {
            let _ = catalog.discard_plan(existing.plan_id());
        }
    }
    catalog.save_resource_plan(&CatalogResourcePlanDocument::Fork(plan.clone()))?;
    Ok(plan)
}

#[tauri::command]
pub async fn prepare_catalog_resource_fork(
    _app: tauri::AppHandle,
    registration_id: super::domain::CatalogRegistrationId,
    requested_name: String,
) -> Result<CatalogResourceForkPlan, String> {
    run_blocking(move || {
        prepare_fork_with_repositories(
            &catalog_repository()?,
            &NativeProcessRunner,
            &registration_id,
            &requested_name,
            &os_user_home()?,
            now_secs(),
        )
    })
    .await
}

#[tauri::command]
pub async fn apply_catalog_resource_fork(
    _app: tauri::AppHandle,
    registration_id: super::domain::CatalogRegistrationId,
    plan_id: CatalogPlanId,
) -> Result<CatalogActionResult, String> {
    run_blocking(move || {
        let catalog = catalog_repository()?;
        let CatalogResourcePlanDocument::Fork(plan) = catalog.load_resource_plan(&plan_id)? else {
            return Err("the referenced plan is not a Fork review".to_string());
        };
        if plan.registration_id != registration_id {
            return Err("the Fork review belongs to another resource".to_string());
        }
        let document = catalog.load_resources()?;
        let registration = document
            .registration(&registration_id)
            .ok_or("the Fork registration disappeared")?;
        if registration.revision != plan.registration_revision {
            return Err("the resource changed after Fork review".to_string());
        }
        let context = resolve_profile_context(
            &catalog.load_profiles()?,
            &registration.profile_id,
            registration.provider,
            &os_user_home()?,
        )
        .map_err(|blocker| blocker.to_string())?;
        context.verify_pins(&plan.profile_pins, &plan.skill_root_pins)?;
        let result = apply_resource_fork(&catalog, &plan, now_secs())?;
        catalog.discard_plan(&plan_id)?;
        Ok(result)
    })
    .await
}

#[tauri::command]
pub async fn get_catalog_resource_status(
    _app: tauri::AppHandle,
    registration_id: super::domain::CatalogRegistrationId,
    link_id: Option<CatalogLinkId>,
) -> Result<CatalogResourceStatusDto, String> {
    run_blocking(move || {
        let catalog = catalog_repository()?;
        let document = catalog.load_resources()?;
        if document.registration(&registration_id).is_none() {
            return Err(format!(
                "unknown catalog registration '{}'",
                registration_id
            ));
        }
        if let Some(link_id) = &link_id {
            let link = document
                .link(link_id)
                .ok_or_else(|| format!("unknown catalog link '{}'", link_id))?;
            if link.registration_id != registration_id {
                return Err("the link belongs to another resource".to_string());
            }
        }
        let active_plan_ids = catalog
            .list_resource_plans()?
            .into_iter()
            .filter(|plan| match plan {
                CatalogResourcePlanDocument::Push(plan) => {
                    plan.registration_id == registration_id
                        && link_id
                            .as_ref()
                            .is_none_or(|link_id| &plan.link_id == link_id)
                }
                CatalogResourcePlanDocument::Pull(plan) => {
                    plan.registration_id == registration_id
                        && link_id
                            .as_ref()
                            .is_none_or(|link_id| &plan.link_id == link_id)
                }
                CatalogResourcePlanDocument::Fork(plan) => {
                    plan.registration_id == registration_id && link_id.is_none()
                }
            })
            .map(|plan| plan.plan_id().clone())
            .collect();
        Ok(CatalogResourceStatusDto {
            registration_id,
            link_id,
            active_plan_ids,
            recovery_messages: recover_catalog_operations(&catalog, now_secs())?,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::domain::{AgentSkillScope, CatalogActionOutcome};
    use super::*;
    use mallard_domain::{StorageKind, SyncConfig};

    struct EmptyCodexRunner;

    impl PluginProcessRunner for EmptyCodexRunner {
        fn run(
            &self,
            provider: ProviderKey,
            _home: &Path,
            argv: &[String],
        ) -> Result<super::super::adapters::CmdOutput, String> {
            assert_eq!(provider, ProviderKey::Codex);
            let stdout = match argv.join(" ").as_str() {
                "plugin marketplace list --json" => r#"{"marketplaces":[]}"#,
                "plugin list --json" => r#"{"installed":[]}"#,
                other => panic!("unexpected provider command: {other}"),
            };
            Ok(super::super::adapters::CmdOutput {
                success: true,
                stdout: stdout.to_string(),
                stderr: String::new(),
            })
        }
    }

    fn storage(value: &str) -> StorageId {
        StorageId::parse(value).unwrap()
    }

    fn claim(storage_id: &str, digest_byte: char) -> CatalogClaim {
        CatalogClaim {
            target: LocalCatalogTargetId::SkillDirectory {
                scope: AgentSkillScope::CodexHome,
                canonical_root: "/tmp/codex/skills".to_string(),
                install_directory: "review".to_string(),
            },
            storage_id: storage(storage_id),
            resource_id: CatalogResourceId::parse(format!("resource-{}", "a".repeat(64))).unwrap(),
            content_sha256: digest_byte.to_string().repeat(64),
            outcome: CatalogActionOutcome::Updated,
            recorded_at: 1,
            plan_id: None,
        }
    }

    #[test]
    fn cross_storage_claim_pins_exclude_the_current_storage_and_matching_content() {
        let target = claim("storage-a", 'a').target;
        let claims = vec![
            claim("storage-current", 'b'),
            claim("storage-a", 'a'),
            claim("storage-b", 'b'),
        ];
        let pins = claim_pins_for_target(
            &claims,
            &storage("storage-current"),
            &target,
            &"b".repeat(64),
        );
        assert_eq!(
            pins,
            vec![CatalogClaimPin {
                storage_id: storage("storage-a"),
                content_sha256: "a".repeat(64),
            }]
        );
    }

    #[test]
    fn cross_storage_claim_pins_are_stably_sorted() {
        let target = claim("storage-b", 'b').target;
        let pins = claim_pins_for_target(
            &[claim("storage-b", 'b'), claim("storage-a", 'a')],
            &storage("storage-current"),
            &target,
            &"c".repeat(64),
        );
        assert_eq!(
            pins.iter()
                .map(|pin| pin.storage_id.as_str())
                .collect::<Vec<_>>(),
            vec!["storage-a", "storage-b"]
        );
    }

    #[test]
    fn failed_or_partial_pull_never_advances_the_comparison_base() {
        assert!(!pull_outcome_accepts_remote(CatalogActionOutcome::Failed));
        assert!(!pull_outcome_accepts_remote(CatalogActionOutcome::Partial));
        assert!(pull_outcome_accepts_remote(CatalogActionOutcome::Updated));
        assert!(pull_outcome_accepts_remote(CatalogActionOutcome::Skipped));
    }

    /// Panics on any provider CLI invocation, so "this path does no inventory
    /// work" is asserted by construction rather than by timing.
    struct PanicRunner;

    impl PluginProcessRunner for PanicRunner {
        fn run(
            &self,
            _provider: ProviderKey,
            _home: &Path,
            argv: &[String],
        ) -> Result<super::super::adapters::CmdOutput, String> {
            panic!("provider inventory ran unexpectedly: {}", argv.join(" "));
        }
    }

    struct LibraryFixture {
        _temporary: tempfile::TempDir,
        user_home: PathBuf,
        skill_path: PathBuf,
        storage_path: PathBuf,
        state: LocalStateRepository,
        catalog: CatalogStateRepository,
        profile: LocalCatalogProfile,
        storage: StorageConfig,
    }

    fn library_fixture() -> LibraryFixture {
        let temporary = tempfile::tempdir().unwrap();
        let user_home = temporary.path().join("user");
        let codex_home = user_home.join("codex-home");
        let skill_path = codex_home.join("skills/review");
        let storage_path = temporary.path().join("storage");
        fs::create_dir_all(&skill_path).unwrap();
        fs::create_dir_all(&storage_path).unwrap();
        fs::write(
            skill_path.join("SKILL.md"),
            b"---\nname: review\n---\nDraft-linked skill.\n",
        )
        .unwrap();

        let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();
        let profile = save_catalog_resource_profile_with_repository(
            &catalog,
            ProviderKey::Codex,
            "Codex test home".to_string(),
            codex_home.to_string_lossy().to_string(),
            1,
        )
        .unwrap();
        let storage = StorageConfig {
            id: storage("library-test"),
            name: "Library test storage".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: storage_path.to_string_lossy().to_string(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        };
        let mut config = SyncConfig::default();
        config.storages.push(storage.clone());
        state.save_config(config).unwrap();

        LibraryFixture {
            _temporary: temporary,
            user_home,
            skill_path,
            storage_path,
            state,
            catalog,
            profile,
            storage,
        }
    }

    fn review_resource_id() -> CatalogResourceId {
        super::super::domain::skill_resource_id(AgentSkillScope::CodexHome, "review", "review")
    }

    fn second_storage(fixture: &LibraryFixture) -> StorageConfig {
        let path = fixture.user_home.join("second-storage");
        fs::create_dir_all(&path).unwrap();
        let storage = StorageConfig {
            id: storage("library-second"),
            name: "Second library storage".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: path.to_string_lossy().to_string(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        };
        fixture
            .state
            .mutate_config(|config| {
                config.storages.push(storage.clone());
                Ok(())
            })
            .unwrap();
        storage
    }

    fn skill_scope(fixture: &LibraryFixture) -> CatalogScopeDto {
        create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::AgentSkill,
            2,
        )
        .unwrap()
    }

    #[test]
    fn creating_a_scope_is_metadata_only_and_unique_per_profile_and_kind() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        assert_eq!(row.scope.resource_kind, CatalogResourceKindTag::AgentSkill);
        assert_eq!(row.profile.profile_id, fixture.profile.profile_id);

        let duplicate = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::AgentSkill,
            3,
        )
        .unwrap_err();
        assert_eq!(duplicate, "this Catalog row already exists");

        // Pet v1 and v2 are distinct kinds and therefore distinct rows.
        for kind in [
            CatalogResourceKindTag::CodexPetV1,
            CatalogResourceKindTag::CodexPetV2,
        ] {
            create_catalog_scope_with_repositories(
                &fixture.catalog,
                ProviderKey::Codex,
                fixture.profile.profile_id.clone(),
                kind,
                4,
            )
            .unwrap();
        }
        assert_eq!(fixture.catalog.load_scopes().unwrap().scopes.len(), 3);

        let rejected = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Claude,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::CodexPetV1,
            5,
        )
        .unwrap_err();
        assert_eq!(rejected, "this provider cannot host that resource kind");
    }

    #[test]
    fn linking_a_scope_resource_links_without_syncing_content() {
        let fixture = library_fixture();
        let original = fs::read(fixture.skill_path.join("SKILL.md")).unwrap();
        let row = skill_scope(&fixture);

        let outcome = link_catalog_scope_resource_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            LinkCatalogScopeResourceRequest {
                scope_id: row.scope.scope_id.clone(),
                storage_id: fixture.storage.id.clone(),
                local_resource_id: Some(review_resource_id()),
                repository_id: None,
            },
            &fixture.user_home,
            3,
        )
        .unwrap();

        assert_eq!(outcome.completion, "open");
        assert_eq!(outcome.resource.links.len(), 1);
        assert!(outcome.resource.links[0].link.accepted_head.is_none());
        assert_eq!(
            fs::read(fixture.skill_path.join("SKILL.md")).unwrap(),
            original
        );
        assert!(!fixture.storage_path.join(".mallard/v1/library").exists());

        // The scope page reports it as registered, locally present, and no
        // longer a discoverable candidate.
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(page.registered.len(), 1);
        assert!(page.registered[0].local_present);
        assert!(page.discovered.is_empty());
    }

    #[test]
    fn linking_a_second_storage_reuses_the_registration() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let second = second_storage(&fixture);

        let link = |storage_id: &StorageId, now| {
            link_catalog_scope_resource_with_repositories(
                &fixture.state,
                &fixture.catalog,
                &EmptyCodexRunner,
                LinkCatalogScopeResourceRequest {
                    scope_id: row.scope.scope_id.clone(),
                    storage_id: storage_id.clone(),
                    local_resource_id: Some(review_resource_id()),
                    repository_id: None,
                },
                &fixture.user_home,
                now,
            )
        };
        let first = link(&fixture.storage.id, 3).unwrap();
        let again = link(&second.id, 4).unwrap();
        assert_eq!(
            first.resource.registration.registration_id,
            again.resource.registration.registration_id
        );
        assert_eq!(again.resource.links.len(), 2);

        let document = fixture.catalog.load_resources().unwrap();
        assert_eq!(document.registrations.len(), 1);
        assert_eq!(document.links.len(), 2);
    }

    #[test]
    fn a_pets_scope_lists_without_provider_clis() {
        let fixture = library_fixture();
        let codex_home = fixture.user_home.join("codex-home");
        let atlas = super::super::codex_pet::fixtures::webp_atlas(CodexPetFormat::V1);
        fs::create_dir_all(codex_home.join("pets/duck")).unwrap();
        fs::write(
            codex_home.join("pets/duck/pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"A duck.","spritesheetPath":"spritesheet.webp"}"#,
        )
        .unwrap();
        fs::write(codex_home.join("pets/duck/spritesheet.webp"), &atlas).unwrap();

        let row = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::CodexPetV1,
            2,
        )
        .unwrap();
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &PanicRunner,
            &row.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(page.discovered.len(), 1);
        assert_eq!(page.discovered[0].display_name, "Duck");
        assert_eq!(page.discovered[0].kind, CatalogResourceKindTag::CodexPetV1);
    }

    #[test]
    fn deleting_a_scope_refuses_while_it_owns_registrations() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        link_catalog_scope_resource_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            LinkCatalogScopeResourceRequest {
                scope_id: row.scope.scope_id.clone(),
                storage_id: fixture.storage.id.clone(),
                local_resource_id: Some(review_resource_id()),
                repository_id: None,
            },
            &fixture.user_home,
            3,
        )
        .unwrap();
        let refused =
            delete_catalog_scope_with_repositories(&fixture.catalog, &row.scope.scope_id, 1)
                .unwrap_err();
        assert!(
            refused.contains("still owns registered resources"),
            "{refused}"
        );
    }

    #[test]
    fn deleting_a_profile_refuses_while_a_scope_names_it() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let refused = delete_catalog_resource_profile_with_repositories(
            &fixture.catalog,
            &fixture.profile.profile_id,
            fixture.profile.revision,
        )
        .unwrap_err();
        assert_eq!(refused, "this profile is used by a Catalog menu row");

        // Removing the row unblocks the profile.
        assert!(
            delete_catalog_scope_with_repositories(&fixture.catalog, &row.scope.scope_id, 1)
                .unwrap()
        );
        assert!(delete_catalog_resource_profile_with_repositories(
            &fixture.catalog,
            &fixture.profile.profile_id,
            fixture.profile.revision,
        )
        .unwrap());
    }
}
