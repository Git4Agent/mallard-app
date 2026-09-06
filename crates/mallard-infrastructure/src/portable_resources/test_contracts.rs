//! Test-only portable contracts proving the four axes extend by
//! registration alone: a resource kind, an apply policy, a provenance kind,
//! and a tombstone target that no production build knows.

use super::registry::PortableRegistrations;
use super::traits::{
    ApplyPolicyAdapter, ProvenanceCodec, ResourceCategory, ResourceKindAdapter,
    TombstoneTargetCodec,
};
use mallard_domain::{
    ApplyPolicyCoordinate, ApplyPolicyPrimitive, BoundedPortableObject, ProvenanceEnvelope,
    ProvenanceKindKey, ProvenancePrimitive, ResourceId, ResourceKindCoordinate,
    ResourceKindPrimitive, TombstoneTargetEnvelope, TombstoneTargetKindKey,
    TombstoneTargetPrimitive,
};
use serde::Deserialize;
use std::sync::Arc;

pub const TEST_RESOURCE_KIND: &str = "testprov.note";
pub const TEST_APPLY_POLICY: &str = "testprov.review_twice";
pub const TEST_PROVENANCE_KIND: &str = "testprov.imported";
pub const TEST_TOMBSTONE_TARGET: &str = "testprov.note_target";

pub fn note_kind() -> ResourceKindCoordinate {
    ResourceKindCoordinate::parse(TEST_RESOURCE_KIND, 1).unwrap()
}

pub fn review_twice_policy() -> ApplyPolicyCoordinate {
    ApplyPolicyCoordinate::parse(TEST_APPLY_POLICY, 1).unwrap()
}

pub fn imported_provenance(source: &str) -> ProvenanceEnvelope {
    ProvenanceEnvelope {
        kind: ProvenanceKindKey::parse(TEST_PROVENANCE_KIND).unwrap(),
        version: 1,
        fields: BoundedPortableObject::from_json(&serde_json::json!({ "source": source })).unwrap(),
    }
}

pub fn note_tombstone(resource_id: &ResourceId) -> TombstoneTargetEnvelope {
    TombstoneTargetEnvelope {
        kind: TombstoneTargetKindKey::parse(TEST_TOMBSTONE_TARGET).unwrap(),
        version: 1,
        identity: BoundedPortableObject::from_json(&serde_json::json!({
            "note_id": resource_id.as_str(),
        }))
        .unwrap(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct NoteAttributesV1 {
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    content_sha256: Option<String>,
}

struct NoteKind;

impl ResourceKindAdapter for NoteKind {
    fn coordinate(&self) -> ResourceKindCoordinate {
        note_kind()
    }

    fn primitive(&self) -> ResourceKindPrimitive {
        ResourceKindPrimitive::Requirement
    }

    fn label(&self) -> &'static str {
        "Test note"
    }

    fn category(&self) -> ResourceCategory {
        ResourceCategory::Tools
    }

    fn validate_attributes(&self, attributes: &BoundedPortableObject) -> Result<(), String> {
        let _typed: NoteAttributesV1 = attributes.decode()?;
        Ok(())
    }
}

struct ReviewTwicePolicy;

impl ApplyPolicyAdapter for ReviewTwicePolicy {
    fn coordinate(&self) -> ApplyPolicyCoordinate {
        review_twice_policy()
    }

    fn primitive(&self) -> ApplyPolicyPrimitive {
        ApplyPolicyPrimitive::ExplicitReview
    }

    fn label(&self) -> &'static str {
        "Review twice"
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportedProvenanceV1 {
    source: String,
}

struct ImportedProvenance;

impl ProvenanceCodec for ImportedProvenance {
    fn kind(&self) -> ProvenanceKindKey {
        ProvenanceKindKey::parse(TEST_PROVENANCE_KIND).unwrap()
    }

    fn version(&self) -> u32 {
        1
    }

    fn decode(&self, fields: &BoundedPortableObject) -> Result<ProvenancePrimitive, String> {
        let typed: ImportedProvenanceV1 = fields.decode()?;
        if typed.source.is_empty() {
            return Err("imported provenance needs a source".to_string());
        }
        Ok(ProvenancePrimitive::Unknown)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteIdentityV1 {
    note_id: String,
}

struct NoteTombstone;

impl TombstoneTargetCodec for NoteTombstone {
    fn kind(&self) -> TombstoneTargetKindKey {
        TombstoneTargetKindKey::parse(TEST_TOMBSTONE_TARGET).unwrap()
    }

    fn version(&self) -> u32 {
        1
    }

    fn decode(&self, identity: &BoundedPortableObject) -> Result<TombstoneTargetPrimitive, String> {
        let typed: NoteIdentityV1 = identity.decode()?;
        Ok(TombstoneTargetPrimitive::Resource {
            resource_id: ResourceId::parse(typed.note_id)?,
        })
    }
}

pub fn register(registrations: &mut PortableRegistrations) {
    registrations.resource_kinds.push(Arc::new(NoteKind));
    registrations
        .apply_policies
        .push(Arc::new(ReviewTwicePolicy));
    registrations
        .provenance_kinds
        .push(Arc::new(ImportedProvenance));
    registrations
        .tombstone_targets
        .push(Arc::new(NoteTombstone));
}
