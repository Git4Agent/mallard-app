use super::harness::codex;
use mallard_domain::{
    BindingState, BundleId, BundleRecipe, LocalProjectId, LocalProjectRegistration,
    LocalProviderProfileId, ProjectBinding, ProviderProfile, ReplicaId,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub(super) struct InstalledScope {
    pub(super) host_id: mallard_domain::HostId,
    pub(super) host_identity_revision: u64,
    pub(super) replica_id: ReplicaId,
    pub(super) project_id: LocalProjectId,
    pub(super) profile_id: LocalProviderProfileId,
    pub(super) project_root: PathBuf,
    pub(super) provider_root: PathBuf,
}

pub(super) fn install_scope(
    repository: &LocalStateRepository,
    bundle_id: &BundleId,
    suffix: &str,
    replica: &str,
) -> InstalledScope {
    let provider = codex();
    let identity = repository.load_or_create_host_identity().unwrap();
    let home = repository.root().parent().unwrap();
    let project_root = home.join(format!("project-{suffix}"));
    let provider_root = home.join(format!("codex-{suffix}"));
    fs::create_dir(&project_root).unwrap();
    fs::create_dir(&provider_root).unwrap();
    let project_root = fs::canonicalize(project_root).unwrap();
    let provider_root = fs::canonicalize(provider_root).unwrap();
    let project_id = LocalProjectId::parse(format!("project-{suffix}")).unwrap();
    let profile_id = LocalProviderProfileId::parse(format!("profile-{suffix}")).unwrap();
    let replica_id = ReplicaId::parse(replica).unwrap();
    repository
        .mutate_config(|config| {
            config.projects.push(LocalProjectRegistration {
                local_project_id: project_id.clone(),
                bundle_id: bundle_id.clone(),
                display_name: format!("Project {suffix}"),
                local_alias: None,
                repository_fingerprint: None,
                recipe: BundleRecipe::default(),
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
    repository
        .mutate_bindings(|_, machine| {
            machine.profiles.push(ProviderProfile {
                profile_id: profile_id.clone(),
                provider: provider.clone(),
                display_name: format!("{} {suffix}", provider.as_str()),
                path: provider_root.to_string_lossy().into_owned(),
                canonical_path: provider_root.to_string_lossy().into_owned(),
                provider_config: mallard_domain::AgentProviderConfig::version_1(),
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            machine.bindings.push(ProjectBinding {
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: project_id.clone(),
                bundle_id: bundle_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                canonical_project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(provider, profile_id.clone())]),
                state: BindingState::Active,
                revision: 0,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
    InstalledScope {
        host_id: identity.host_id,
        host_identity_revision: identity.revision,
        replica_id,
        project_id,
        profile_id,
        project_root,
        provider_root,
    }
}
