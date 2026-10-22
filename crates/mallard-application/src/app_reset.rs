//! App-scoped local-data reset: delete the whole `~/.mallard` metadata root.
//! The deletion itself lives on `LocalStateRepository::wipe_local_state`;
//! this adds the catalog-state guard the infrastructure crate cannot see and
//! forgets the pending Library reviews the process keeps in memory.

use crate::storage_catalog::persistence::CatalogStateRepository;
use crate::storage_catalog::plan_store;
use mallard_infrastructure::persistence::{LocalStateRepository, LocalStateWipeResult};

/// Lock order is catalog-state → host operation → persistence — the order
/// `clear_storage_data` already establishes; inverting it risks a flock
/// deadlock between two operations.
pub fn reset_local_state(
    repository: &LocalStateRepository,
    catalog: &CatalogStateRepository,
) -> Result<LocalStateWipeResult, String> {
    let _catalog_guard = catalog.acquire_state_guard()?;
    let result = repository.wipe_local_state()?;
    // Pending Library reviews are process-wide in-memory state; the process
    // survives the wipe, so forget them with the root they described.
    plan_store::discard_all(catalog.root());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::reset_local_state;
    use crate::storage_catalog::domain::{
        CatalogCoordinate, CatalogLinkId, CatalogPlanId, CatalogResourcePlanDocument,
        CatalogResourceRepositoryId, CatalogScopeId, CatalogScopePushPlan,
    };
    use crate::storage_catalog::persistence::CatalogStateRepository;
    use crate::storage_catalog::plan_store;
    use mallard_infrastructure::persistence::LocalStateRepository;

    #[test]
    fn reset_clears_catalog_state_too() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(temp.path()).unwrap();
        std::fs::write(catalog.root().join("catalog_scopes.json"), b"{}").unwrap();
        let plan = CatalogResourcePlanDocument::Push(CatalogScopePushPlan {
            plan_id: CatalogPlanId::generate().unwrap(),
            scope_id: CatalogScopeId::generate().unwrap(),
            link_id: CatalogLinkId::generate().unwrap(),
            link_revision: 1,
            storage_id: mallard_domain::StorageId::parse("storage-a").unwrap(),
            repository_id: CatalogResourceRepositoryId::for_coordinate(
                &CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap(),
            ),
            storage_identity_sha256: "a".repeat(64),
            profile_pins: Vec::new(),
            target_pins: Vec::new(),
            observed_head: None,
            observed_items: std::collections::BTreeMap::new(),
            destinations: std::collections::BTreeMap::new(),
            rows: Vec::new(),
            warning_digests: Vec::new(),
            created_at: 1,
            expires_at: 2,
        });
        let plan_id = plan.plan_id().clone();
        plan_store::save(catalog.root(), plan, 1).unwrap();

        let result = reset_local_state(&repository, &catalog).unwrap();
        assert!(result.existed);
        assert!(result.files_removed > 0);
        assert!(!temp.path().join(".mallard").exists());
        // The process survives the reset, so its pending reviews must not.
        assert!(plan_store::load(catalog.root(), &plan_id).is_err());

        // Both repositories reconstruct cleanly over the fresh root.
        LocalStateRepository::from_home_dir(temp.path()).unwrap();
        CatalogStateRepository::from_home_dir(temp.path()).unwrap();
    }
}
