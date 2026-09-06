//! Portable resource contract roles.
//!
//! A resource-kind adapter owns typed attribute decoding, safe presentation,
//! and the mapping to one engine primitive. An apply-policy adapter selects
//! one engine primitive and its review rules. A provenance codec validates
//! and types provenance fields; a tombstone codec validates and types a
//! target identity. None of them performs I/O.

use mallard_domain::{
    ApplyPolicyCoordinate, ApplyPolicyPrimitive, BoundedPortableObject, ProvenanceKindKey,
    ProvenancePrimitive, ResourceKindCoordinate, ResourceKindPrimitive, TombstoneTargetKindKey,
    TombstoneTargetPrimitive,
};
use serde::{Deserialize, Serialize};

/// Presentation grouping for resource kinds. A closed presentation vocabulary
/// the frontend already renders; it is not a contract identity.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceCategory {
    Conversations,
    ProjectFiles,
    Tools,
}

impl ResourceCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversations => "conversations",
            Self::ProjectFiles => "project_files",
            Self::Tools => "tools",
        }
    }
}

/// Safe, serializable facts about one registered resource kind.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResourceKindDescriptor {
    pub key: String,
    pub version: u32,
    pub label: String,
    pub category: ResourceCategory,
}

pub trait ResourceKindAdapter: Send + Sync {
    fn coordinate(&self) -> ResourceKindCoordinate;
    fn primitive(&self) -> ResourceKindPrimitive;
    fn label(&self) -> &'static str;
    fn category(&self) -> ResourceCategory;
    /// Typed decoding of the bounded attributes for this exact contract.
    /// Unknown fields are rejected.
    fn validate_attributes(&self, attributes: &BoundedPortableObject) -> Result<(), String>;

    fn descriptor(&self) -> ResourceKindDescriptor {
        let coordinate = self.coordinate();
        ResourceKindDescriptor {
            key: coordinate.key.as_str().to_string(),
            version: coordinate.version,
            label: self.label().to_string(),
            category: self.category(),
        }
    }
}

pub trait ApplyPolicyAdapter: Send + Sync {
    fn coordinate(&self) -> ApplyPolicyCoordinate;
    fn primitive(&self) -> ApplyPolicyPrimitive;
    fn label(&self) -> &'static str;
}

pub trait ProvenanceCodec: Send + Sync {
    fn kind(&self) -> ProvenanceKindKey;
    fn version(&self) -> u32;
    /// Decode the bounded fields into the typed primitive, rejecting unknown
    /// or malformed fields.
    fn decode(&self, fields: &BoundedPortableObject) -> Result<ProvenancePrimitive, String>;
}

pub trait TombstoneTargetCodec: Send + Sync {
    fn kind(&self) -> TombstoneTargetKindKey;
    fn version(&self) -> u32;
    /// Decode the bounded identity into the typed primitive, rejecting
    /// unknown or malformed fields.
    fn decode(&self, identity: &BoundedPortableObject) -> Result<TombstoneTargetPrimitive, String>;
}
