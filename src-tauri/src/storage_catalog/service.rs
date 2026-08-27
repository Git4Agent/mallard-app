//! Non-Tauri application boundary for storage-scoped Library resources.
//!
//! Desktop commands and external integration tests use this service so
//! profile setup, discovery, scope linking, reviewed Push publication, and
//! reviewed Pull application all cross the same production orchestration
//! path. Tests inject an isolated machine home and a bounded provider process
//! runner; no operation needs to redirect the process-wide user home.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use mallard_domain::StorageId;
use mallard_infrastructure::persistence::LocalStateRepository;

use super::adapters::{NativeProcessRunner, PluginProcessRunner};
use super::domain::{
    CatalogPlanActionId, CatalogPlanId, CatalogResourceId, CatalogResourceKindTag, CatalogScopeId,
    CatalogScopePullPlan, CatalogScopePushPlan, LocalCatalogProfile, LocalCatalogProfileId,
    ProviderKey,
};
use super::persistence::CatalogStateRepository;
use super::resource_commands::{
    self, CatalogScopeDto, CatalogScopeLinkOutcomeDto, CatalogScopePullOutcomeDto,
    CatalogScopePushOutcomeDto, CatalogScopeResourcesDto,
};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// The complete approval captured by a reviewed scope Push.
#[derive(Clone, Debug)]
pub struct CatalogPushApproval {
    pub plan_id: CatalogPlanId,
    pub selected_resource_ids: Vec<CatalogResourceId>,
    pub acknowledged_warning_digests: Vec<String>,
}

impl CatalogPushApproval {
    /// Select every row exactly as recommended by the review. Warning
    /// acknowledgement is deliberately empty: callers must copy the digests
    /// they actually displayed and received approval for.
    pub fn recommended(plan: &CatalogScopePushPlan) -> Self {
        Self {
            plan_id: plan.plan_id.clone(),
            selected_resource_ids: plan
                .rows
                .iter()
                .filter(|row| row.row.selected_by_default)
                .map(|row| row.row.resource_id.clone())
                .collect(),
            acknowledged_warning_digests: Vec::new(),
        }
    }
}

/// The exact per-item actions approved from a reviewed scope Pull.
#[derive(Clone, Debug)]
pub struct CatalogPullApproval {
    pub plan_id: CatalogPlanId,
    pub choices: BTreeMap<CatalogResourceId, CatalogPlanActionId>,
    pub destructive_acknowledged: Vec<CatalogResourceId>,
}

impl CatalogPullApproval {
    /// Approve every item's recommended action; items with no recommendation
    /// are explicit Skips.
    pub fn recommended(plan: &CatalogScopePullPlan) -> Self {
        Self {
            plan_id: plan.plan_id.clone(),
            choices: plan
                .items
                .iter()
                .filter_map(|item| {
                    item.item
                        .default_action_id
                        .clone()
                        .map(|action_id| (item.item.resource_id.clone(), action_id))
                })
                .collect(),
            destructive_acknowledged: Vec::new(),
        }
    }
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

    pub fn save_profile(
        &self,
        provider: ProviderKey,
        configured_home: String,
    ) -> Result<LocalCatalogProfile, String> {
        resource_commands::save_catalog_resource_profile_with_repository(
            &self.catalog,
            provider,
            configured_home,
            now_secs(),
        )
    }

    pub fn rename_profile(
        &self,
        profile_id: &LocalCatalogProfileId,
        display_name: String,
        expected_revision: u64,
    ) -> Result<LocalCatalogProfile, String> {
        resource_commands::rename_catalog_resource_profile_with_repository(
            &self.catalog,
            profile_id,
            display_name,
            expected_revision,
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

    pub fn list_storage_repositories(
        &self,
        storage_id: &StorageId,
    ) -> Result<Vec<resource_commands::CatalogStorageRepositoryDto>, String> {
        resource_commands::list_catalog_storage_repositories_with_repository(
            &self.state,
            storage_id,
        )
    }

    pub fn pet_preview(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
    ) -> Result<String, String> {
        resource_commands::get_catalog_pet_preview_with_repositories(
            &self.state,
            &self.catalog,
            scope_id,
            storage_id,
            resource_id,
            &self.user_home,
        )
    }

    pub fn link_scope_storage(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Result<CatalogScopeLinkOutcomeDto, String> {
        resource_commands::link_catalog_scope_storage_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn prepare_push(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Result<CatalogScopePushPlan, String> {
        resource_commands::prepare_push_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn apply_push(
        &self,
        approval: CatalogPushApproval,
    ) -> Result<CatalogScopePushOutcomeDto, String> {
        resource_commands::apply_push_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            approval.plan_id,
            approval.selected_resource_ids,
            approval.acknowledged_warning_digests,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn prepare_pull(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        requested_copy_names: BTreeMap<CatalogResourceId, String>,
    ) -> Result<CatalogScopePullPlan, String> {
        resource_commands::prepare_pull_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            requested_copy_names,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn apply_pull(
        &self,
        approval: CatalogPullApproval,
    ) -> Result<CatalogScopePullOutcomeDto, String> {
        resource_commands::apply_pull_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            approval.plan_id,
            approval.choices,
            approval.destructive_acknowledged,
            &self.user_home,
            now_secs(),
        )
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
            .save_profile(ProviderKey::Codex, codex_home.to_string_lossy().to_string())
            .expect("save profile");
        assert_eq!(profile.display_name, "myconf1");
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

        assert_eq!(resources.rows.len(), 1);
        assert_eq!(resources.rows[0].display_name, "mallard-alpha");
        assert!(resources.rows[0].local_present);
        assert!(resources.link.is_none());
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
