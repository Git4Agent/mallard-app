//! Ephemeral reviewed-plan store.
//!
//! A review (Push, Pull, or Fork) pins what the user saw so apply can refuse
//! when anything moved underneath it. That snapshot only needs to outlive the
//! open review dialog: the frontend keeps the plan in component state and has
//! no way to resume one after a reload, so plans live in one bounded
//! in-memory map per catalog root and die with the process. Replacement,
//! expiry, and a hard cap bound each map; apply re-verifies everything
//! against live state, so a discarded or evicted plan can only ever fail
//! closed ("review again"), never publish something unreviewed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use super::domain::{CatalogPlanId, CatalogResourcePlanDocument};

/// More concurrent reviews than one person can have open. Eviction beyond
/// the cap forces a fresh review, never a wrong apply.
const MAX_PLANS_PER_CATALOG: usize = 16;

type PlanMap = BTreeMap<CatalogPlanId, CatalogResourcePlanDocument>;

/// One map per catalog root, so isolated machine homes (tests model several
/// in one process) never see each other's reviews.
fn stores() -> MutexGuard<'static, BTreeMap<PathBuf, PlanMap>> {
    static STORES: OnceLock<Mutex<BTreeMap<PathBuf, PlanMap>>> = OnceLock::new();
    STORES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        // A panic while holding the lock cannot leave a map half-written in a
        // way apply would trust: every entry is re-verified on use.
        .unwrap_or_else(PoisonError::into_inner)
}

/// Validate and remember one freshly built review. Expired plans for the same
/// root are swept, and the oldest reviews are evicted beyond the cap.
pub fn save(root: &Path, plan: CatalogResourcePlanDocument, now: u64) -> Result<(), String> {
    plan.validate()?;
    let mut stores = stores();
    let plans = stores.entry(root.to_path_buf()).or_default();
    plans.retain(|_, existing| now <= existing.expires_at());
    while plans.len() >= MAX_PLANS_PER_CATALOG {
        let oldest = plans
            .iter()
            .min_by_key(|(plan_id, existing)| (existing.created_at(), (*plan_id).clone()))
            .map(|(plan_id, _)| plan_id.clone())
            .expect("a non-empty map has a minimum");
        plans.remove(&oldest);
    }
    plans.insert(plan.plan_id().clone(), plan);
    Ok(())
}

pub fn load(root: &Path, plan_id: &CatalogPlanId) -> Result<CatalogResourcePlanDocument, String> {
    stores()
        .get(root)
        .and_then(|plans| plans.get(plan_id))
        .cloned()
        .ok_or_else(|| "this review is no longer available; review again".to_string())
}

pub fn discard(root: &Path, plan_id: &CatalogPlanId) -> bool {
    stores()
        .get_mut(root)
        .is_some_and(|plans| plans.remove(plan_id).is_some())
}

/// Drop every plan the predicate matches — supersession when a new review
/// opens, and the sweeps that follow an unlink or a storage data clear.
pub fn discard_where(root: &Path, mut matches: impl FnMut(&CatalogResourcePlanDocument) -> bool) {
    if let Some(plans) = stores().get_mut(root) {
        plans.retain(|_, plan| !matches(plan));
    }
}

/// Drop every pending review for one catalog root — the local-data reset,
/// which deletes the root while the process stays alive.
pub fn discard_all(root: &Path) {
    stores().remove(root);
}

#[cfg(test)]
mod tests {
    use super::super::domain::{
        CatalogCoordinate, CatalogLinkId, CatalogResourceRepositoryId, CatalogScopeId,
        CatalogScopePushPlan,
    };
    use super::*;
    use mallard_domain::StorageId;

    fn push_plan(created_at: u64, expires_at: u64) -> CatalogResourcePlanDocument {
        CatalogResourcePlanDocument::Push(CatalogScopePushPlan {
            plan_id: CatalogPlanId::generate().unwrap(),
            scope_id: CatalogScopeId::generate().unwrap(),
            link_id: CatalogLinkId::generate().unwrap(),
            link_revision: 1,
            storage_id: StorageId::parse("storage-a").unwrap(),
            repository_id: CatalogResourceRepositoryId::for_coordinate(
                &CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap(),
            ),
            storage_identity_sha256: "a".repeat(64),
            profile_pins: Vec::new(),
            target_pins: Vec::new(),
            observed_head: None,
            observed_items: BTreeMap::new(),
            rows: Vec::new(),
            warning_digests: Vec::new(),
            created_at,
            expires_at,
        })
    }

    fn root(temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join(".mallard")
    }

    #[test]
    fn plans_round_trip_and_discard_per_root() {
        let temp = tempfile::tempdir().unwrap();
        let sibling = tempfile::tempdir().unwrap();
        let plan = push_plan(1, 100);
        let plan_id = plan.plan_id().clone();
        save(&root(&temp), plan.clone(), 1).unwrap();

        assert_eq!(load(&root(&temp), &plan_id).unwrap(), plan);
        // Another machine home never sees this root's reviews.
        let missing = load(&root(&sibling), &plan_id).unwrap_err();
        assert!(missing.contains("review again"), "{missing}");
        assert!(!discard(&root(&sibling), &plan_id));

        assert!(discard(&root(&temp), &plan_id));
        assert!(!discard(&root(&temp), &plan_id));
        assert!(load(&root(&temp), &plan_id).is_err());
    }

    #[test]
    fn saving_sweeps_expired_plans_and_replacement_discards_by_predicate() {
        let temp = tempfile::tempdir().unwrap();
        let expired = push_plan(1, 5);
        let expired_id = expired.plan_id().clone();
        save(&root(&temp), expired, 1).unwrap();

        let superseded = push_plan(6, 100);
        let superseded_id = superseded.plan_id().clone();
        save(&root(&temp), superseded, 6).unwrap();
        assert!(load(&root(&temp), &expired_id).is_err());

        let scope_id = match load(&root(&temp), &superseded_id).unwrap() {
            CatalogResourcePlanDocument::Push(plan) => plan.scope_id,
            _ => unreachable!("the fixture is a Push plan"),
        };
        discard_where(
            &root(&temp),
            |plan| matches!(plan, CatalogResourcePlanDocument::Push(plan) if plan.scope_id == scope_id),
        );
        assert!(load(&root(&temp), &superseded_id).is_err());
    }

    #[test]
    fn the_cap_evicts_the_oldest_review_first() {
        let temp = tempfile::tempdir().unwrap();
        let oldest = push_plan(1, 1_000);
        let oldest_id = oldest.plan_id().clone();
        save(&root(&temp), oldest, 1).unwrap();
        let mut newer = Vec::new();
        for offset in 0..MAX_PLANS_PER_CATALOG as u64 {
            let plan = push_plan(2 + offset, 1_000);
            newer.push(plan.plan_id().clone());
            save(&root(&temp), plan, 2 + offset).unwrap();
        }

        assert!(load(&root(&temp), &oldest_id).is_err());
        for plan_id in &newer {
            load(&root(&temp), plan_id).unwrap();
        }
    }

    #[test]
    fn discard_all_forgets_a_whole_root() {
        let temp = tempfile::tempdir().unwrap();
        let plan = push_plan(1, 100);
        let plan_id = plan.plan_id().clone();
        save(&root(&temp), plan, 1).unwrap();
        discard_all(&root(&temp));
        assert!(load(&root(&temp), &plan_id).is_err());
    }
}
