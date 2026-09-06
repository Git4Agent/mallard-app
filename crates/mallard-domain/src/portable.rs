//! Open Project Sync portable contracts.
//!
//! The four portable extension axes — resource kind, apply policy,
//! provenance kind, and tombstone target — are versioned coordinates and
//! bounded envelopes, never closed enums. An older build parses a newer
//! bundle, preserves every record, and treats unknown coordinates as inert.
//!
//! Opening the name does not open the behavior. A registered adapter or codec
//! resolves an exact coordinate to one of the closed *primitives* below, and
//! the common engine executes only those primitives. Common code dispatches
//! by resolver lookup; it never matches on a contract key.

use crate::bounded::BoundedPortableObject;
use crate::keys::{ApplyPolicyKey, ProvenanceKindKey, ResourceKindKey, TombstoneTargetKindKey};
use crate::{CloudThreadId, LogicalPath, ResourceId};
use serde::{Deserialize, Serialize};
use std::fmt;

fn validate_contract_version(label: &str, version: u32) -> Result<(), String> {
    if version == 0 {
        return Err(format!("{} version must be positive", label));
    }
    Ok(())
}

/// One exact resource-kind contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct ResourceKindCoordinate {
    pub key: ResourceKindKey,
    pub version: u32,
}

impl ResourceKindCoordinate {
    pub fn new(key: ResourceKindKey, version: u32) -> Self {
        Self { key, version }
    }

    pub fn parse(key: &str, version: u32) -> Result<Self, String> {
        let coordinate = Self::new(ResourceKindKey::parse(key)?, version);
        coordinate.validate()?;
        Ok(coordinate)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_contract_version("resource kind", self.version)
    }
}

impl fmt::Display for ResourceKindCoordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.key, self.version)
    }
}

/// One exact apply-policy contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct ApplyPolicyCoordinate {
    pub key: ApplyPolicyKey,
    pub version: u32,
}

impl ApplyPolicyCoordinate {
    pub fn new(key: ApplyPolicyKey, version: u32) -> Self {
        Self { key, version }
    }

    pub fn parse(key: &str, version: u32) -> Result<Self, String> {
        let coordinate = Self::new(ApplyPolicyKey::parse(key)?, version);
        coordinate.validate()?;
        Ok(coordinate)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_contract_version("apply policy", self.version)
    }
}

impl fmt::Display for ApplyPolicyCoordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.key, self.version)
    }
}

/// Provenance as a versioned kind plus bounded, codec-typed fields.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceEnvelope {
    pub kind: ProvenanceKindKey,
    pub version: u32,
    #[serde(default)]
    pub fields: BoundedPortableObject,
}

impl ProvenanceEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        validate_contract_version("provenance kind", self.version)
    }
}

/// A tombstone target as a versioned kind plus a bounded identity object.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TombstoneTargetEnvelope {
    pub kind: TombstoneTargetKindKey,
    pub version: u32,
    pub identity: BoundedPortableObject,
}

impl TombstoneTargetEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        validate_contract_version("tombstone target kind", self.version)?;
        if self.identity.is_empty() {
            return Err(format!(
                "tombstone target '{}' has an empty identity",
                self.kind
            ));
        }
        Ok(())
    }

    /// The load-time map key of a tombstone: its kind, version, and the
    /// canonical JSON of its identity. It is derived, never persisted, so a
    /// later codec can change only the in-memory key.
    pub fn structural_key(&self) -> String {
        format!(
            "{}@{}:{}",
            self.kind,
            self.version,
            String::from_utf8_lossy(&self.identity.canonical_json())
        )
    }
}

/// The closed set of resource-kind behaviors the common engine implements.
/// A registered resource-kind adapter maps its coordinate to exactly one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKindPrimitive {
    Conversation,
    ProviderState,
    ProjectContentFile,
    ProjectContentDirectory,
    Requirement,
}

impl ResourceKindPrimitive {
    /// Provider-owned kinds live in their provider's scope; everything else
    /// in the project scope.
    pub fn requires_provider(self) -> bool {
        matches!(self, Self::Conversation | Self::ProviderState)
    }
}

/// The closed set of apply behaviors the common engine implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ApplyPolicyPrimitive {
    SafeFile,
    Merge,
    ExplicitReview,
    ManualOnly,
    Never,
}

/// Typed provenance the common engine understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenancePrimitive {
    ProjectLocal { relative_path: String },
    Unknown,
}

/// Typed tombstone targets the common engine can acknowledge or delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TombstoneTargetPrimitive {
    Resource {
        resource_id: ResourceId,
    },
    File {
        resource_id: ResourceId,
        logical_path: LogicalPath,
    },
    ProjectContentFile {
        resource_id: ResourceId,
        logical_path: LogicalPath,
    },
    ProjectContentDirectory {
        resource_id: ResourceId,
        logical_path: LogicalPath,
    },
    ConversationThread {
        thread_id: CloudThreadId,
    },
}

/// Registry validation for portable contracts: an exact coordinate resolves
/// to a primitive or to nothing. `Err` means a registered codec rejected the
/// record's fields; `Ok(None)` means the coordinate is not registered in
/// this build and the record must stay preserved and inert.
pub trait PortableContractResolver {
    fn resolve_resource_kind(
        &self,
        coordinate: &ResourceKindCoordinate,
        attributes: &BoundedPortableObject,
    ) -> Result<Option<ResourceKindPrimitive>, String>;
    /// The wire rules one agent provider's conversations must satisfy, when
    /// this build registers that provider.
    fn conversation_wire_rules(
        &self,
        _provider: &crate::ProviderKey,
    ) -> Option<crate::ConversationWireRules> {
        None
    }
    fn apply_policy(&self, coordinate: &ApplyPolicyCoordinate) -> Option<ApplyPolicyPrimitive>;
    fn provenance(
        &self,
        envelope: &ProvenanceEnvelope,
    ) -> Result<Option<ProvenancePrimitive>, String>;
    fn tombstone_target(
        &self,
        envelope: &TombstoneTargetEnvelope,
    ) -> Result<Option<TombstoneTargetPrimitive>, String>;
}

/// The version-1 keys of the initial contracts, with constructors for the
/// records current adapters emit. Registries register against these keys;
/// common validation and planning never compare against them.
pub mod contracts {
    use super::*;

    pub const CONTRACT_VERSION_1: u32 = 1;

    pub const CONVERSATION: &str = "conversation";
    pub const PROVIDER_STATE: &str = "provider_state";
    pub const PROJECT_CONTENT_FILE: &str = "project_content_file";
    pub const PROJECT_CONTENT_DIRECTORY: &str = "project_content_directory";
    pub const REQUIREMENT: &str = "requirement";

    pub const SAFE_FILE: &str = "safe_file";
    pub const MERGE: &str = "merge";
    pub const EXPLICIT_REVIEW: &str = "explicit_review";
    pub const MANUAL_ONLY: &str = "manual_only";
    pub const NEVER: &str = "never";

    pub const PROJECT_LOCAL: &str = "project_local";
    pub const UNKNOWN: &str = "unknown";

    pub const RESOURCE: &str = "resource";
    pub const FILE: &str = "file";
    pub const CONVERSATION_THREAD: &str = "conversation_thread";

    fn kind(key: &str) -> ResourceKindCoordinate {
        ResourceKindCoordinate::parse(key, CONTRACT_VERSION_1).expect("static resource kind")
    }

    fn policy(key: &str) -> ApplyPolicyCoordinate {
        ApplyPolicyCoordinate::parse(key, CONTRACT_VERSION_1).expect("static apply policy")
    }

    pub fn conversation() -> ResourceKindCoordinate {
        kind(CONVERSATION)
    }

    pub fn provider_state() -> ResourceKindCoordinate {
        kind(PROVIDER_STATE)
    }

    pub fn project_content_file() -> ResourceKindCoordinate {
        kind(PROJECT_CONTENT_FILE)
    }

    pub fn project_content_directory() -> ResourceKindCoordinate {
        kind(PROJECT_CONTENT_DIRECTORY)
    }

    pub fn requirement() -> ResourceKindCoordinate {
        kind(REQUIREMENT)
    }

    pub fn safe_file() -> ApplyPolicyCoordinate {
        policy(SAFE_FILE)
    }

    pub fn merge() -> ApplyPolicyCoordinate {
        policy(MERGE)
    }

    pub fn explicit_review() -> ApplyPolicyCoordinate {
        policy(EXPLICIT_REVIEW)
    }

    pub fn manual_only() -> ApplyPolicyCoordinate {
        policy(MANUAL_ONLY)
    }

    pub fn never() -> ApplyPolicyCoordinate {
        policy(NEVER)
    }

    pub fn provenance_unknown() -> ProvenanceEnvelope {
        ProvenanceEnvelope {
            kind: ProvenanceKindKey::parse(UNKNOWN).expect("static provenance kind"),
            version: CONTRACT_VERSION_1,
            fields: BoundedPortableObject::empty(),
        }
    }

    pub fn provenance_project_local(relative_path: &str) -> Result<ProvenanceEnvelope, String> {
        Ok(ProvenanceEnvelope {
            kind: ProvenanceKindKey::parse(PROJECT_LOCAL).expect("static provenance kind"),
            version: CONTRACT_VERSION_1,
            fields: BoundedPortableObject::from_json(&serde_json::json!({
                "relative_path": relative_path,
            }))?,
        })
    }

    fn tombstone(
        key: &str,
        identity: serde_json::Value,
    ) -> Result<TombstoneTargetEnvelope, String> {
        Ok(TombstoneTargetEnvelope {
            kind: TombstoneTargetKindKey::parse(key)?,
            version: CONTRACT_VERSION_1,
            identity: BoundedPortableObject::from_json(&identity)?,
        })
    }

    pub fn tombstone_resource(resource_id: &ResourceId) -> Result<TombstoneTargetEnvelope, String> {
        tombstone(
            RESOURCE,
            serde_json::json!({ "resource_id": resource_id.as_str() }),
        )
    }

    pub fn tombstone_file(
        resource_id: &ResourceId,
        logical_path: &LogicalPath,
    ) -> Result<TombstoneTargetEnvelope, String> {
        tombstone(
            FILE,
            serde_json::json!({
                "resource_id": resource_id.as_str(),
                "logical_path": logical_path.as_str(),
            }),
        )
    }

    pub fn tombstone_project_content_file(
        resource_id: &ResourceId,
        logical_path: &LogicalPath,
    ) -> Result<TombstoneTargetEnvelope, String> {
        tombstone(
            PROJECT_CONTENT_FILE,
            serde_json::json!({
                "resource_id": resource_id.as_str(),
                "logical_path": logical_path.as_str(),
            }),
        )
    }

    pub fn tombstone_project_content_directory(
        resource_id: &ResourceId,
        logical_path: &LogicalPath,
    ) -> Result<TombstoneTargetEnvelope, String> {
        tombstone(
            PROJECT_CONTENT_DIRECTORY,
            serde_json::json!({
                "resource_id": resource_id.as_str(),
                "logical_path": logical_path.as_str(),
            }),
        )
    }

    pub fn tombstone_conversation_thread(
        thread_id: &CloudThreadId,
    ) -> Result<TombstoneTargetEnvelope, String> {
        tombstone(
            CONVERSATION_THREAD,
            serde_json::json!({ "thread_id": thread_id.as_str() }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_require_positive_versions_and_print_stably() {
        let kind = ResourceKindCoordinate::parse("conversation", 1).unwrap();
        assert_eq!(kind.to_string(), "conversation@1");
        assert!(ResourceKindCoordinate::parse("conversation", 0).is_err());
        assert!(ApplyPolicyCoordinate::parse("Bad", 1).is_err());
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "{\"key\":\"conversation\",\"version\":1}");
        assert!(serde_json::from_str::<ResourceKindCoordinate>(
            "{\"key\":\"conversation\",\"version\":1,\"extra\":true}"
        )
        .is_err());
    }

    #[test]
    fn tombstone_keys_derive_from_kind_version_and_canonical_identity() {
        let resource_id = ResourceId::parse("resource-a").unwrap();
        let path = LogicalPath::parse("project/README.md").unwrap();
        let target = contracts::tombstone_file(&resource_id, &path).unwrap();
        assert_eq!(
            target.structural_key(),
            "file@1:{\"logical_path\":\"project/README.md\",\"resource_id\":\"resource-a\"}"
        );
        let unregistered = TombstoneTargetEnvelope {
            kind: TombstoneTargetKindKey::parse("future_target").unwrap(),
            version: 3,
            identity: BoundedPortableObject::from_json(&serde_json::json!({"b": 1, "a": 2}))
                .unwrap(),
        };
        unregistered.validate().unwrap();
        assert_eq!(
            unregistered.structural_key(),
            "future_target@3:{\"a\":2,\"b\":1}"
        );
        assert!(TombstoneTargetEnvelope {
            identity: BoundedPortableObject::empty(),
            ..unregistered
        }
        .validate()
        .is_err());
    }

    #[test]
    fn provenance_envelopes_carry_bounded_portable_fields() {
        let local = contracts::provenance_project_local("docs/notes.md").unwrap();
        assert_eq!(local.fields.get_str("relative_path"), Some("docs/notes.md"));
        assert!(contracts::provenance_project_local("/abs/path").is_err());
        let json = serde_json::to_string(&contracts::provenance_unknown()).unwrap();
        assert_eq!(json, "{\"kind\":\"unknown\",\"version\":1,\"fields\":{}}");
    }
}

/// A registry-free resolver for the version-1 contracts, for tests of the
/// domain's registry-validation stage. Production resolution lives in the
/// infrastructure registry.
#[cfg(any(test, feature = "test-support"))]
pub mod test_resolver {
    use super::contracts;
    use super::*;

    pub struct Version1Resolver;

    impl PortableContractResolver for Version1Resolver {
        fn conversation_wire_rules(
            &self,
            provider: &crate::ProviderKey,
        ) -> Option<crate::ConversationWireRules> {
            use crate::{ConversationWireRules, PathMapDeclaration, TailWireKind};
            match provider.as_str() {
                "codex" => Some(ConversationWireRules {
                    requires_redaction_stamp: true,
                    requires_portable_tail: true,
                    path_map: PathMapDeclaration::InBand,
                    tail_wire: TailWireKind::CodexRollout,
                }),
                "claude" => Some(ConversationWireRules {
                    requires_redaction_stamp: true,
                    requires_portable_tail: true,
                    path_map: PathMapDeclaration::OutOfBand,
                    tail_wire: TailWireKind::ClaudeJsonl,
                }),
                _ => None,
            }
        }

        fn resolve_resource_kind(
            &self,
            coordinate: &ResourceKindCoordinate,
            _attributes: &BoundedPortableObject,
        ) -> Result<Option<ResourceKindPrimitive>, String> {
            if coordinate.version != contracts::CONTRACT_VERSION_1 {
                return Ok(None);
            }
            Ok(match coordinate.key.as_str() {
                contracts::CONVERSATION => Some(ResourceKindPrimitive::Conversation),
                contracts::PROVIDER_STATE => Some(ResourceKindPrimitive::ProviderState),
                contracts::PROJECT_CONTENT_FILE => Some(ResourceKindPrimitive::ProjectContentFile),
                contracts::PROJECT_CONTENT_DIRECTORY => {
                    Some(ResourceKindPrimitive::ProjectContentDirectory)
                }
                contracts::REQUIREMENT => Some(ResourceKindPrimitive::Requirement),
                _ => None,
            })
        }

        fn apply_policy(&self, coordinate: &ApplyPolicyCoordinate) -> Option<ApplyPolicyPrimitive> {
            if coordinate.version != contracts::CONTRACT_VERSION_1 {
                return None;
            }
            match coordinate.key.as_str() {
                contracts::SAFE_FILE => Some(ApplyPolicyPrimitive::SafeFile),
                contracts::MERGE => Some(ApplyPolicyPrimitive::Merge),
                contracts::EXPLICIT_REVIEW => Some(ApplyPolicyPrimitive::ExplicitReview),
                contracts::MANUAL_ONLY => Some(ApplyPolicyPrimitive::ManualOnly),
                contracts::NEVER => Some(ApplyPolicyPrimitive::Never),
                _ => None,
            }
        }

        fn provenance(
            &self,
            envelope: &ProvenanceEnvelope,
        ) -> Result<Option<ProvenancePrimitive>, String> {
            if envelope.version != contracts::CONTRACT_VERSION_1 {
                return Ok(None);
            }
            match envelope.kind.as_str() {
                contracts::PROJECT_LOCAL => {
                    let relative_path = envelope
                        .fields
                        .get_str("relative_path")
                        .ok_or_else(|| "project-local provenance lacks a path".to_string())?;
                    if relative_path.is_empty() || relative_path.split('/').any(|part| part == "..")
                    {
                        return Err("invalid project-local provenance path".to_string());
                    }
                    Ok(Some(ProvenancePrimitive::ProjectLocal {
                        relative_path: relative_path.to_string(),
                    }))
                }
                contracts::UNKNOWN => Ok(Some(ProvenancePrimitive::Unknown)),
                _ => Ok(None),
            }
        }

        fn tombstone_target(
            &self,
            envelope: &TombstoneTargetEnvelope,
        ) -> Result<Option<TombstoneTargetPrimitive>, String> {
            if envelope.version != contracts::CONTRACT_VERSION_1 {
                return Ok(None);
            }
            let field = |name: &str| -> Result<String, String> {
                envelope
                    .identity
                    .get_str(name)
                    .map(str::to_string)
                    .ok_or_else(|| format!("tombstone identity lacks '{}'", name))
            };
            let resource_id = || ResourceId::parse(field("resource_id")?);
            let logical_path = || LogicalPath::parse(field("logical_path")?);
            Ok(Some(match envelope.kind.as_str() {
                contracts::RESOURCE => TombstoneTargetPrimitive::Resource {
                    resource_id: resource_id()?,
                },
                contracts::FILE => TombstoneTargetPrimitive::File {
                    resource_id: resource_id()?,
                    logical_path: logical_path()?,
                },
                contracts::PROJECT_CONTENT_FILE => TombstoneTargetPrimitive::ProjectContentFile {
                    resource_id: resource_id()?,
                    logical_path: logical_path()?,
                },
                contracts::PROJECT_CONTENT_DIRECTORY => {
                    TombstoneTargetPrimitive::ProjectContentDirectory {
                        resource_id: resource_id()?,
                        logical_path: logical_path()?,
                    }
                }
                contracts::CONVERSATION_THREAD => TombstoneTargetPrimitive::ConversationThread {
                    thread_id: CloudThreadId::parse(field("thread_id")?)?,
                },
                _ => return Ok(None),
            }))
        }
    }
}
