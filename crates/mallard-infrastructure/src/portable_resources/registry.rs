use super::traits::{
    ApplyPolicyAdapter, ProvenanceCodec, ResourceKindAdapter, ResourceKindDescriptor,
    TombstoneTargetCodec,
};
use mallard_domain::{
    ApplyPolicyCoordinate, ApplyPolicyPrimitive, BoundedPortableObject, PortableContractResolver,
    ProvenanceEnvelope, ProvenanceKindKey, ProvenancePrimitive, ResourceKindCoordinate,
    ResourceKindPrimitive, TombstoneTargetEnvelope, TombstoneTargetKindKey,
    TombstoneTargetPrimitive,
};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// Everything one build registers for the four portable axes.
#[derive(Default)]
pub struct PortableRegistrations {
    pub resource_kinds: Vec<Arc<dyn ResourceKindAdapter>>,
    pub apply_policies: Vec<Arc<dyn ApplyPolicyAdapter>>,
    pub provenance_kinds: Vec<Arc<dyn ProvenanceCodec>>,
    pub tombstone_targets: Vec<Arc<dyn TombstoneTargetCodec>>,
}

#[derive(Clone)]
pub struct PortableResourceRegistry {
    resource_kinds: BTreeMap<ResourceKindCoordinate, Arc<dyn ResourceKindAdapter>>,
    apply_policies: BTreeMap<ApplyPolicyCoordinate, Arc<dyn ApplyPolicyAdapter>>,
    provenance_kinds: BTreeMap<(ProvenanceKindKey, u32), Arc<dyn ProvenanceCodec>>,
    tombstone_targets: BTreeMap<(TombstoneTargetKindKey, u32), Arc<dyn TombstoneTargetCodec>>,
}

impl PortableResourceRegistry {
    /// Build a registry, refusing any repeated coordinate on any axis.
    pub fn from_registrations(registrations: PortableRegistrations) -> Result<Self, String> {
        let mut resource_kinds = BTreeMap::new();
        for adapter in registrations.resource_kinds {
            let coordinate = adapter.coordinate();
            coordinate.validate()?;
            if resource_kinds.insert(coordinate.clone(), adapter).is_some() {
                return Err(format!(
                    "resource kind '{}' is registered twice",
                    coordinate
                ));
            }
        }
        let mut apply_policies = BTreeMap::new();
        for adapter in registrations.apply_policies {
            let coordinate = adapter.coordinate();
            coordinate.validate()?;
            if apply_policies.insert(coordinate.clone(), adapter).is_some() {
                return Err(format!("apply policy '{}' is registered twice", coordinate));
            }
        }
        let mut provenance_kinds = BTreeMap::new();
        for codec in registrations.provenance_kinds {
            let key = (codec.kind(), codec.version());
            if key.1 == 0 {
                return Err(format!("provenance kind '{}' declares version 0", key.0));
            }
            if provenance_kinds.insert(key.clone(), codec).is_some() {
                return Err(format!(
                    "provenance kind '{}@{}' is registered twice",
                    key.0, key.1
                ));
            }
        }
        let mut tombstone_targets = BTreeMap::new();
        for codec in registrations.tombstone_targets {
            let key = (codec.kind(), codec.version());
            if key.1 == 0 {
                return Err(format!("tombstone target '{}' declares version 0", key.0));
            }
            if tombstone_targets.insert(key.clone(), codec).is_some() {
                return Err(format!(
                    "tombstone target '{}@{}' is registered twice",
                    key.0, key.1
                ));
            }
        }
        Ok(Self {
            resource_kinds,
            apply_policies,
            provenance_kinds,
            tombstone_targets,
        })
    }

    /// The process-wide production registry: every version-1 contract.
    pub fn production() -> &'static PortableResourceRegistry {
        static REGISTRY: OnceLock<PortableResourceRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            PortableResourceRegistry::from_registrations(super::contracts::version_1())
                .expect("production portable registry has no duplicate registrations")
        })
    }

    /// The production contracts plus the test-only additions.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_contracts() -> PortableResourceRegistry {
        let mut registrations = super::contracts::version_1();
        super::test_contracts::register(&mut registrations);
        PortableResourceRegistry::from_registrations(registrations)
            .expect("test portable registry has no duplicate registrations")
    }

    pub fn resource_kind_adapter(
        &self,
        coordinate: &ResourceKindCoordinate,
    ) -> Option<&dyn ResourceKindAdapter> {
        self.resource_kinds.get(coordinate).map(Arc::as_ref)
    }

    pub fn apply_policy_adapter(
        &self,
        coordinate: &ApplyPolicyCoordinate,
    ) -> Option<&dyn ApplyPolicyAdapter> {
        self.apply_policies.get(coordinate).map(Arc::as_ref)
    }

    pub fn require_resource_kind(
        &self,
        coordinate: &ResourceKindCoordinate,
    ) -> Result<&dyn ResourceKindAdapter, String> {
        self.resource_kind_adapter(coordinate).ok_or_else(|| {
            format!(
                "resource kind '{}' is not registered in this build",
                coordinate
            )
        })
    }

    /// The registered coordinate for one engine primitive, when exactly one
    /// registration maps to it at the newest version. Used by adapters that
    /// must emit a coordinate for a primitive they produce.
    pub fn resource_kind_for(
        &self,
        primitive: ResourceKindPrimitive,
    ) -> Option<ResourceKindCoordinate> {
        self.resource_kinds
            .iter()
            .filter(|(_, adapter)| adapter.primitive() == primitive)
            .map(|(coordinate, _)| coordinate.clone())
            .max_by_key(|coordinate| coordinate.version)
    }

    pub fn apply_policy_for(
        &self,
        primitive: ApplyPolicyPrimitive,
    ) -> Option<ApplyPolicyCoordinate> {
        self.apply_policies
            .iter()
            .filter(|(_, adapter)| adapter.primitive() == primitive)
            .map(|(coordinate, _)| coordinate.clone())
            .max_by_key(|coordinate| coordinate.version)
    }

    /// Descriptors in ascending key order.
    pub fn resource_kind_descriptors(&self) -> Vec<ResourceKindDescriptor> {
        self.resource_kinds
            .values()
            .map(|adapter| adapter.descriptor())
            .collect()
    }

    /// Safe presentation for one kind coordinate: the registered label, or
    /// the raw coordinate when this build has no adapter for it.
    pub fn resource_kind_label(&self, coordinate: &ResourceKindCoordinate) -> String {
        self.resource_kind_adapter(coordinate)
            .map(|adapter| adapter.label().to_string())
            .unwrap_or_else(|| coordinate.to_string())
    }

    pub fn apply_policy_label(&self, coordinate: &ApplyPolicyCoordinate) -> String {
        self.apply_policy_adapter(coordinate)
            .map(|adapter| adapter.label().to_string())
            .unwrap_or_else(|| coordinate.to_string())
    }
}

impl PortableContractResolver for PortableResourceRegistry {
    fn conversation_wire_rules(
        &self,
        provider: &mallard_domain::ProviderKey,
    ) -> Option<mallard_domain::ConversationWireRules> {
        crate::providers::AgentProviderRegistry::production()
            .get(provider)
            .and_then(|adapter| adapter.conversation_wire_rules())
    }

    fn resolve_resource_kind(
        &self,
        coordinate: &ResourceKindCoordinate,
        attributes: &BoundedPortableObject,
    ) -> Result<Option<ResourceKindPrimitive>, String> {
        let Some(adapter) = self.resource_kind_adapter(coordinate) else {
            return Ok(None);
        };
        adapter.validate_attributes(attributes)?;
        Ok(Some(adapter.primitive()))
    }

    fn apply_policy(&self, coordinate: &ApplyPolicyCoordinate) -> Option<ApplyPolicyPrimitive> {
        self.apply_policy_adapter(coordinate)
            .map(|adapter| adapter.primitive())
    }

    fn provenance(
        &self,
        envelope: &ProvenanceEnvelope,
    ) -> Result<Option<ProvenancePrimitive>, String> {
        match self
            .provenance_kinds
            .get(&(envelope.kind.clone(), envelope.version))
        {
            Some(codec) => codec.decode(&envelope.fields).map(Some),
            None => Ok(None),
        }
    }

    fn tombstone_target(
        &self,
        envelope: &TombstoneTargetEnvelope,
    ) -> Result<Option<TombstoneTargetPrimitive>, String> {
        match self
            .tombstone_targets
            .get(&(envelope.kind.clone(), envelope.version))
        {
            Some(codec) => codec.decode(&envelope.identity).map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::portable::contracts;
    use mallard_domain::{
        BoundedPortableObject, BundleRecipe, BundleScope, CapturedWith, RecipeEntry,
        ResourceDescriptor, ResourceId, SyncScopeKey,
    };

    #[test]
    fn production_registers_every_version_1_contract() {
        let registry = PortableResourceRegistry::production();
        for coordinate in [
            contracts::conversation(),
            contracts::provider_state(),
            contracts::project_content_file(),
            contracts::project_content_directory(),
            contracts::requirement(),
        ] {
            assert!(
                registry
                    .resolve_resource_kind(&coordinate, &BoundedPortableObject::empty())
                    .unwrap()
                    .is_some(),
                "{coordinate}"
            );
        }
        for coordinate in [
            contracts::safe_file(),
            contracts::merge(),
            contracts::explicit_review(),
            contracts::manual_only(),
            contracts::never(),
        ] {
            assert!(registry.apply_policy(&coordinate).is_some(), "{coordinate}");
        }
        assert_eq!(
            registry
                .provenance(&contracts::provenance_unknown())
                .unwrap(),
            Some(ProvenancePrimitive::Unknown)
        );
        let unknown_kind = ResourceKindCoordinate::parse("future_kind", 1).unwrap();
        assert!(registry
            .resolve_resource_kind(&unknown_kind, &BoundedPortableObject::empty())
            .unwrap()
            .is_none());
        let unknown_version = ResourceKindCoordinate::parse("conversation", 2).unwrap();
        assert!(registry
            .resolve_resource_kind(&unknown_version, &BoundedPortableObject::empty())
            .unwrap()
            .is_none());
        assert_eq!(
            registry.resource_kind_for(ResourceKindPrimitive::Conversation),
            Some(contracts::conversation())
        );
        assert_eq!(
            registry.apply_policy_for(ApplyPolicyPrimitive::ExplicitReview),
            Some(contracts::explicit_review())
        );
    }

    #[test]
    fn duplicate_registrations_are_rejected_on_every_axis() {
        let mut registrations = super::super::contracts::version_1();
        registrations
            .resource_kinds
            .push(registrations.resource_kinds[0].clone());
        assert!(PortableResourceRegistry::from_registrations(registrations)
            .err()
            .unwrap()
            .contains("registered twice"));
        let mut registrations = super::super::contracts::version_1();
        registrations
            .tombstone_targets
            .push(registrations.tombstone_targets[0].clone());
        assert!(PortableResourceRegistry::from_registrations(registrations)
            .err()
            .unwrap()
            .contains("registered twice"));
    }

    #[test]
    fn scope_resolution_rejects_attributes_the_registered_kind_does_not_accept() {
        let resource_id = ResourceId::parse("requirement:invalid-attributes").unwrap();
        let policy = contracts::manual_only();
        let scope_key = SyncScopeKey::Project;
        let scope = BundleScope::sealed(
            &scope_key,
            1,
            BundleRecipe {
                schema_version: 1,
                revision: 1,
                entries: BTreeMap::from([(
                    resource_id.clone(),
                    RecipeEntry {
                        resource_id: resource_id.clone(),
                        apply_policy: policy.clone(),
                        required: false,
                    },
                )]),
            },
            CapturedWith::default(),
            BTreeMap::new(),
            BTreeMap::from([(
                resource_id.clone(),
                ResourceDescriptor {
                    resource_id,
                    kind: contracts::requirement(),
                    provider: None,
                    provider_resource_type: None,
                    display_name: "Invalid attributes".to_string(),
                    provenance: contracts::provenance_unknown(),
                    apply_policy: policy,
                    codec_version: 1,
                    attributes: BoundedPortableObject::from_json(&serde_json::json!({
                        "unexpected": "value"
                    }))
                    .unwrap(),
                },
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        let error = scope
            .resolve(&scope_key, PortableResourceRegistry::production())
            .unwrap_err();
        assert!(error.contains("attributes"), "{error}");
    }

    #[test]
    fn descriptors_follow_key_order_and_test_contracts_add_without_common_changes() {
        let keys = PortableResourceRegistry::with_test_contracts()
            .resource_kind_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.key)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "conversation",
                "project_content_directory",
                "project_content_file",
                "provider_state",
                "requirement",
                "testprov.note",
            ]
        );
    }
}
