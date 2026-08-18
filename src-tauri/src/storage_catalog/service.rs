//! Non-Tauri application boundary for storage-scoped Library resources.
//!
//! Desktop commands and external integration tests use this service so
//! profile setup, discovery, linking, reviewed Push publication, and reviewed
//! Pull application all cross the same production orchestration path. Tests
//! inject an isolated machine home and a bounded provider process runner; no
//! operation needs to redirect the process-wide user home.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mallard_domain::StorageId;
use mallard_infrastructure::persistence::LocalStateRepository;

use super::adapters::{NativeProcessRunner, PluginProcessRunner};
use super::domain::{
    CatalogLinkId, CatalogPlanActionId, CatalogPlanId, CatalogRegistrationId, CatalogResource,
    CatalogResourceKindTag, CatalogResourcePullPlan, CatalogResourcePushPlan,
    CatalogResourceRepositoryId, CatalogScopeId, LibraryHeadCoordinate, LibraryResourceTag,
    LocalCatalogProfile, LocalCatalogProfileId, ProviderKey,
};
use super::persistence::CatalogStateRepository;
use super::resource_commands::{
    self, CatalogResourceLinkInspectionDto, CatalogResourcePullOutcomeDto,
    CatalogResourcePushOutcomeDto, CatalogScopeDto, CatalogScopeLinkOutcomeDto,
    CatalogScopeResourcesDto, LinkCatalogScopeResourceRequest,
};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// The complete approval captured by a reviewed Library Push.
#[derive(Clone, Debug)]
pub struct CatalogPublishApproval {
    pub plan_id: CatalogPlanId,
    pub include: bool,
    pub acknowledged_warning_digests: Vec<String>,
}

impl CatalogPublishApproval {
    /// Select the resource exactly as recommended by the review. Warning
    /// acknowledgement is deliberately empty: callers must copy the digests
    /// they actually displayed and received approval for.
    pub fn recommended(plan: &CatalogResourcePushPlan) -> Self {
        Self {
            plan_id: plan.plan_id.clone(),
            include: plan.row.selectable,
            acknowledged_warning_digests: Vec::new(),
        }
    }
}

/// The exact action approved from a reviewed Library Pull.
#[derive(Clone, Debug)]
pub struct CatalogPullApproval {
    pub plan_id: CatalogPlanId,
    pub approved_action_id: Option<CatalogPlanActionId>,
    pub destructive_acknowledged: bool,
}

impl CatalogPullApproval {
    /// Approve the plan's recommended action, or Skip when the plan has no
    /// recommended mutation.
    pub fn recommended(plan: &CatalogResourcePullPlan) -> Self {
        Self {
            plan_id: plan.plan_id.clone(),
            approved_action_id: plan.item.default_action_id.clone(),
            destructive_acknowledged: false,
        }
    }
}

/// Verified current state of one independent Library repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRepositoryInspectionDto {
    pub storage_id: StorageId,
    pub repository_id: CatalogResourceRepositoryId,
    pub tag: Option<LibraryResourceTag>,
    pub head: Option<LibraryHeadCoordinate>,
    pub resource: Option<CatalogResource>,
}

/// Production Library use cases with explicitly selected machine state,
/// provider-home resolution, and provider process execution.
pub struct StorageCatalogService {
    state: LocalStateRepository,
    catalog: CatalogStateRepository,
    runner: Box<dyn PluginProcessRunner>,
    user_home: PathBuf,
}

impl StorageCatalogService {
    /// Resolve production repositories and provider defaults from the actual
    /// user home. Tauri wrappers use this constructor.
    pub fn from_user_home() -> Result<Self, String> {
        let user_home = dirs::home_dir()
            .ok_or_else(|| "could not resolve the user home directory".to_string())?;
        Self::from_home_dirs(user_home.clone(), user_home, Box::new(NativeProcessRunner))
    }

    /// Build the full production service over an isolated machine home.
    ///
    /// `machine_home` owns Mallard's machine-local `.mallard` documents.
    /// `user_home` is used only when resolving provider default homes and
    /// shared skill roots. Both must be explicit absolute paths, which lets
    /// tests model two machines without touching real user state.
    pub fn from_home_dirs(
        machine_home: impl Into<PathBuf>,
        user_home: impl Into<PathBuf>,
        runner: Box<dyn PluginProcessRunner>,
    ) -> Result<Self, String> {
        let machine_home = machine_home.into();
        let user_home = user_home.into();
        if !machine_home.is_absolute() {
            return Err(format!(
                "machine home must be absolute: '{}'",
                machine_home.display()
            ));
        }
        if !user_home.is_absolute() {
            return Err(format!(
                "provider user home must be absolute: '{}'",
                user_home.display()
            ));
        }
        Ok(Self {
            state: LocalStateRepository::from_home_dir(machine_home.clone())?,
            catalog: CatalogStateRepository::from_home_dir(machine_home)?,
            runner,
            user_home,
        })
    }

    pub fn state_repository(&self) -> &LocalStateRepository {
        &self.state
    }

    pub fn user_home(&self) -> &Path {
        &self.user_home
    }

    pub fn save_profile(
        &self,
        provider: ProviderKey,
        display_name: String,
        configured_home: String,
    ) -> Result<LocalCatalogProfile, String> {
        resource_commands::save_catalog_resource_profile_with_repository(
            &self.catalog,
            provider,
            display_name,
            configured_home,
            now_secs(),
        )
    }

    pub fn create_scope(
        &self,
        provider: ProviderKey,
        profile_id: LocalCatalogProfileId,
        resource_kind: CatalogResourceKindTag,
    ) -> Result<CatalogScopeDto, String> {
        resource_commands::create_catalog_scope_with_repositories(
            &self.catalog,
            provider,
            profile_id,
            resource_kind,
            now_secs(),
        )
    }

    pub fn list_scope_resources(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: Option<&StorageId>,
    ) -> Result<CatalogScopeResourcesDto, String> {
        resource_commands::list_catalog_scope_resources_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            &self.user_home,
        )
    }

    pub fn link_scope_resource(
        &self,
        request: LinkCatalogScopeResourceRequest,
    ) -> Result<CatalogScopeLinkOutcomeDto, String> {
        resource_commands::link_catalog_scope_resource_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            request,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn inspect_links(
        &self,
        registration_id: &CatalogRegistrationId,
    ) -> Result<Vec<CatalogResourceLinkInspectionDto>, String> {
        resource_commands::inspect_links_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            registration_id,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn prepare_push(&self, link_id: &CatalogLinkId) -> Result<CatalogResourcePushPlan, String> {
        resource_commands::prepare_push_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            link_id,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn publish_reviewed(
        &self,
        link_id: &CatalogLinkId,
        approval: CatalogPublishApproval,
    ) -> Result<CatalogResourcePushOutcomeDto, String> {
        resource_commands::publish_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            link_id,
            approval.plan_id,
            approval.include,
            approval.acknowledged_warning_digests,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn prepare_pull(
        &self,
        link_id: &CatalogLinkId,
        requested_copy_name: Option<&str>,
    ) -> Result<CatalogResourcePullPlan, String> {
        resource_commands::prepare_pull_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            link_id,
            requested_copy_name,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn apply_pull(
        &self,
        link_id: &CatalogLinkId,
        approval: CatalogPullApproval,
    ) -> Result<CatalogResourcePullOutcomeDto, String> {
        resource_commands::apply_pull_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            link_id,
            approval.plan_id,
            approval.approved_action_id,
            approval.destructive_acknowledged,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn inspect_repository(
        &self,
        storage_id: &StorageId,
        repository_id: &CatalogResourceRepositoryId,
    ) -> Result<CatalogRepositoryInspectionDto, String> {
        let storage = resource_commands::storage_for(&self.state, storage_id)?;
        let engine = resource_commands::engine_for(&storage)?;
        let tag = engine.read_tag(repository_id)?.map(|(tag, _)| tag);
        let inspected = engine.inspect(repository_id)?;
        let (head, resource) = match inspected {
            Some((observed, manifest)) => {
                let tag = tag
                    .as_ref()
                    .ok_or("the Library repository has a head but no discovery tag")?;
                if tag.repository_id != *repository_id
                    || tag.resource_id != manifest.resource.resource_id()
                    || tag.resource_kind != manifest.resource.kind_tag()
                    || tag.provider != manifest.resource.provider()
                    || tag.stable_identity_sha256 != manifest.resource.identity_sha256()
                {
                    return Err(
                        "the Library repository has inconsistent discovery metadata".to_string()
                    );
                }
                (
                    Some(super::resource_engine::head_coordinate(
                        &observed.head,
                        &super::resource_engine::storage_identity_for_config(&storage),
                    )),
                    Some(manifest.resource),
                )
            }
            None => {
                if tag.is_some() {
                    return Err("the Library repository has a tag but no head".to_string());
                }
                (None, None)
            }
        };
        Ok(CatalogRepositoryInspectionDto {
            storage_id: storage_id.clone(),
            repository_id: repository_id.clone(),
            tag,
            head,
            resource,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::storage_catalog::adapters::{CmdOutput, StubRunner};

    fn empty_codex_runner() -> Box<dyn PluginProcessRunner> {
        let ok = |stdout: &str| CmdOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        };
        Box::new(StubRunner::new(vec![
            (
                "plugin marketplace list --json",
                ok(r#"{"marketplaces":[]}"#),
            ),
            ("plugin list --json", ok(r#"{"installed":[]}"#)),
        ]))
    }

    #[test]
    fn isolated_service_discovers_a_profile_skill_through_the_production_path() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let machine_home = temporary.path().join("machine");
        let user_home = temporary.path().join("user");
        let codex_home = temporary.path().join("myconf1");
        let skill = codex_home.join("skills").join("mallard-alpha");
        fs::create_dir_all(&skill).expect("create test skill");
        fs::create_dir_all(&machine_home).expect("create machine home");
        fs::create_dir_all(&user_home).expect("create user home");
        fs::write(
            skill.join("SKILL.md"),
            b"---\nname: mallard-alpha\ndescription: isolated test skill\n---\nBody\n",
        )
        .expect("write test skill");

        let service =
            StorageCatalogService::from_home_dirs(&machine_home, &user_home, empty_codex_runner())
                .expect("construct isolated service");
        let profile = service
            .save_profile(
                ProviderKey::Codex,
                "Codex one".to_string(),
                codex_home.to_string_lossy().to_string(),
            )
            .expect("save profile");
        let scope = service
            .create_scope(
                ProviderKey::Codex,
                profile.profile_id,
                CatalogResourceKindTag::AgentSkill,
            )
            .expect("create scope");
        let resources = service
            .list_scope_resources(&scope.scope.scope_id, None)
            .expect("list resources");

        assert_eq!(
            service.state_repository().root(),
            machine_home.join(".mallard")
        );
        assert_eq!(resources.rows.len(), 1);
        assert_eq!(resources.rows[0].display_name, "mallard-alpha");
        assert!(resources.rows[0].local_present);
        assert!(!resources.rows[0].linked);
        assert!(!resources.rows[0].remote_present);
    }

    #[test]
    fn isolated_service_rejects_relative_homes() {
        assert!(StorageCatalogService::from_home_dirs(
            "relative-machine",
            "/absolute-user",
            empty_codex_runner(),
        )
        .is_err());
        assert!(StorageCatalogService::from_home_dirs(
            "/absolute-machine",
            "relative-user",
            empty_codex_runner(),
        )
        .is_err());
    }
}
