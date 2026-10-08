//! Reviewed local removal. Storage is never opened by these commands.
use super::*;
use crate::storage_catalog::apply::{journal_entry, journal_phase, journal_upsert};
use crate::storage_catalog::domain::{
    CatalogOperationId, CatalogOperationKind, CatalogOperationPhase, CatalogResourceRemovalPlan,
};
use crate::storage_catalog::removal_files;

#[derive(Serialize, Debug)]
pub struct CatalogRemovalResult {
    pub removed: bool,
    /// English; a finished removal is translated by `removed`, with
    /// `display_name`. A failed one's message is an error.
    pub message: String,
    pub display_name: String,
}

fn native_fingerprint(inventory: &ProviderInventory) -> String {
    // These local observations include installation paths, versions and enablement;
    // the portable intent digest deliberately does not. Never emit this input.
    let mut facts = inventory
        .plugins
        .iter()
        .map(|p| format!("{p:?}"))
        .collect::<Vec<_>>();
    facts.extend(inventory.marketplaces.iter().map(|m| format!("{m:?}")));
    facts.sort();
    sha256_hex(facts.join("\n").as_bytes())
}

impl StorageCatalogService {
    pub fn prepare_removal(
        &self,
        scope_id: &CatalogScopeId,
        resource_id: &CatalogResourceId,
    ) -> Result<CatalogResourceRemovalPlan, String> {
        let ResolvedScope { scope, .. } = resolve_scope(&self.catalog, scope_id)?;
        let capability = require_capability(&self.registry, &scope)?;
        let context = resolve_profile_context(
            &self.registry,
            &self.catalog.load_profiles()?,
            &scope.profile_id,
            &scope.provider,
            &self.user_home,
        )
        .map_err(|e| e.to_string())?;
        let inventory =
            inventory_local(&self.registry, &capability, &context).map_err(|e| e.to_string())?;
        let record = inventory
            .resources
            .get(resource_id)
            .ok_or("This item is not available locally. Refresh and try again.")?;
        let (entries, native_state_sha256, argv) = if let Some(path) = &record.target_path {
            (removal_files::snapshot(path)?, None, Vec::new())
        } else {
            let native = inventory
                .native
                .as_ref()
                .ok_or("Plugin inventory is unavailable")?;
            (
                BTreeMap::new(),
                Some(native_fingerprint(native)),
                capability.adapter.plan_native_remove(&record.resource)?,
            )
        };
        let now = now_secs();
        let plan = CatalogResourceRemovalPlan {
            plan_id: CatalogPlanId::generate()?,
            scope_id: scope_id.clone(),
            resource_id: resource_id.clone(),
            display_name: capability.codec.display_name(&record.resource)?,
            uninstall: native_state_sha256.is_some(),
            file_count: entries.values().filter(|e| e.sha256.is_some()).count(),
            created_at: now,
            expires_at: now + 300,
            scope_revision: scope.revision,
            profile_pins: context.profile_pins(),
            target_pins: context.target_pins(),
            target: record.target.clone(),
            resource: record.resource.clone(),
            entries,
            native_state_sha256,
            argv,
        };
        plan_store::discard_where(
            self.catalog.root(),
            |p| matches!(p, CatalogResourcePlanDocument::Removal(p) if p.scope_id == *scope_id && p.resource_id == *resource_id),
        );
        plan_store::save(
            self.catalog.root(),
            CatalogResourcePlanDocument::Removal(plan.clone()),
            now,
        )?;
        Ok(plan)
    }

    pub fn apply_removal(&self, plan_id: &CatalogPlanId) -> Result<CatalogRemovalResult, String> {
        let CatalogResourcePlanDocument::Removal(plan) =
            plan_store::load(self.catalog.root(), plan_id)?
        else {
            return Err("This is not a local removal review".to_string());
        };
        let identity = plan.target.decode()?;
        let _lock = self
            .catalog
            .acquire_target_lock(Path::new(&identity.canonical_root))?;
        // Consume under the same lock as Pull and Fork. Repeated clicks cannot replay.
        if !plan_store::discard(self.catalog.root(), plan_id) {
            return Err("Review removal again".to_string());
        }
        if now_secs() > plan.expires_at {
            return Err("This removal review expired. Review again.".to_string());
        }
        let ResolvedScope { scope, .. } = resolve_scope(&self.catalog, &plan.scope_id)?;
        if scope.revision != plan.scope_revision {
            return Err("The Catalog row changed. Review again.".to_string());
        }
        let capability = require_capability(&self.registry, &scope)?;
        let context = resolve_profile_context(
            &self.registry,
            &self.catalog.load_profiles()?,
            &scope.profile_id,
            &scope.provider,
            &self.user_home,
        )
        .map_err(|e| e.to_string())?;
        context.verify_pins(&plan.profile_pins, &plan.target_pins)?;
        let inventory =
            inventory_local(&self.registry, &capability, &context).map_err(|e| e.to_string())?;
        let record = inventory
            .resources
            .get(&plan.resource_id)
            .ok_or("The local item changed. Refresh and review again.")?;
        if record.target != plan.target || record.resource != plan.resource {
            return Err("The local item changed. Review again.".to_string());
        }
        if plan.uninstall {
            if inventory.native.as_ref().map(native_fingerprint) != plan.native_state_sha256
                || capability.adapter.plan_native_remove(&record.resource)? != plan.argv
            {
                return Err("Plugin state changed. Review removal again.".to_string());
            }
        } else if removal_files::snapshot(
            record
                .target_path
                .as_ref()
                .ok_or("Local folder is unavailable")?,
        )? != plan.entries
        {
            return Err("Local files changed. Review removal again.".to_string());
        }
        let operation_id = CatalogOperationId::generate()?;
        let mut entry = journal_entry(
            operation_id.clone(),
            CatalogOperationKind::LocalRemoval,
            scope.coordinate(),
            plan.target.clone(),
            now_secs(),
        );
        entry.plan_id = Some(plan_id.clone());
        entry.expected_target_sha256 = Some(plan.resource.content_sha256.clone());
        entry.target_path = record
            .target_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        journal_upsert(&self.catalog, entry)?;
        // Include aliases and other storages that pin this same physical root.
        plan_store::discard_where(self.catalog.root(), |pending| match pending {
            CatalogResourcePlanDocument::Push(p) => p
                .target_pins
                .iter()
                .any(|pin| pin.canonical_root == identity.canonical_root),
            CatalogResourcePlanDocument::Pull(p) => p
                .target_pins
                .iter()
                .any(|pin| pin.canonical_root == identity.canonical_root),
            CatalogResourcePlanDocument::Fork(p) => p.source_target == plan.target,
            CatalogResourcePlanDocument::Removal(p) => p.target == plan.target,
        });
        let attempt = (|| -> Result<(), String> {
            if plan.uninstall {
                let output = self.runner.run(
                    &scope.provider,
                    &context.profile.canonical_home,
                    &plan.argv,
                )?;
                let after = capability
                    .adapter
                    .discover_local(&context.profile.canonical_home)
                    .map_err(|e| e.to_string())?;
                if capability
                    .adapter
                    .native_state(&plan.resource, &after)?
                    .is_some()
                {
                    return Err("The provider still reports this plugin installed. Refresh and review again.".to_string());
                }
                if !output.success {
                    return Err("The provider reported an uninstall failure. Check the refreshed inventory.".to_string());
                }
            } else {
                removal_files::remove(
                    record
                        .target_path
                        .as_ref()
                        .ok_or("Local folder is unavailable")?,
                    &plan.entries,
                )?;
            }
            self.catalog.mutate_materializations(|doc| {
                doc.claims.retain(|claim| claim.target != plan.target);
                Ok(())
            })?;
            self.catalog.mutate_preservations(|doc| {
                doc.witnesses
                    .retain(|witness| witness.preserved_target != plan.target);
                Ok(())
            })?;
            Ok(())
        })();
        let (removed, message) = match attempt {
            Ok(()) => (
                true,
                format!(
                    "{} removed from this provider home. Copies in storage are kept.",
                    plan.display_name
                ),
            ),
            Err(error) => (
                false,
                format!("Removal did not complete. Some local changes may have been made. {error}"),
            ),
        };
        journal_phase(
            &self.catalog,
            &operation_id,
            if removed {
                CatalogOperationPhase::Completed
            } else {
                CatalogOperationPhase::Failed
            },
            Some(if removed {
                CatalogActionOutcome::Removed
            } else {
                CatalogActionOutcome::Partial
            }),
            Some(message.clone()),
            now_secs(),
        )?;
        Ok(CatalogRemovalResult {
            removed,
            message,
            display_name: plan.display_name.clone(),
        })
    }
}
