//! The version-1 portable contracts: five resource kinds, five apply
//! policies, two provenance kinds, and five tombstone targets. Each maps its
//! coordinate to one engine primitive and types its bounded payload.

use super::registry::PortableRegistrations;
use super::traits::{
    ApplyPolicyAdapter, ProvenanceCodec, ResourceCategory, ResourceKindAdapter,
    TombstoneTargetCodec,
};
use mallard_domain::portable::contracts;
use mallard_domain::{
    validate_sha256, ApplyPolicyCoordinate, ApplyPolicyPrimitive, BoundedPortableObject,
    BoundedValue, CloudThreadId, LogicalPath, ProvenanceKindKey, ProvenancePrimitive, ResourceId,
    ResourceKindCoordinate, ResourceKindPrimitive, TombstoneTargetKindKey,
    TombstoneTargetPrimitive,
};
use serde::Deserialize;
use std::path::{Component, Path};
use std::sync::Arc;

/// Credential material is excluded from portable data by policy: every
/// string value runs through the shared credential-shape detector. A match
/// rejects the record. This is a warning aid, not proof of safety.
pub fn reject_credential_shapes(object: &BoundedPortableObject) -> Result<(), String> {
    fn visit(value: &BoundedValue) -> Result<(), String> {
        match value {
            BoundedValue::String(text) => {
                if crate::credential_redaction::shared()
                    .redact_str(text)
                    .is_some()
                {
                    return Err("portable object contains credential-shaped text".to_string());
                }
                Ok(())
            }
            BoundedValue::Array(items) => items.iter().try_for_each(visit),
            BoundedValue::Object(fields) => fields.values().try_for_each(visit),
            BoundedValue::Null | BoundedValue::Bool(_) | BoundedValue::Number(_) => Ok(()),
        }
    }
    object.iter().try_for_each(|(_, value)| visit(value))
}

fn validate_optional_digest(digest: &Option<String>) -> Result<(), String> {
    if let Some(digest) = digest {
        validate_sha256("resource content digest", digest)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Resource kinds
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DigestOnlyAttributesV1 {
    #[serde(default)]
    content_sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectContentAttributesV1 {
    #[serde(default)]
    entry_type: Option<String>,
    #[serde(default)]
    content_sha256: Option<String>,
}

struct SimpleKind {
    key: &'static str,
    primitive: ResourceKindPrimitive,
    label: &'static str,
    category: ResourceCategory,
    entry_type: Option<&'static str>,
}

impl ResourceKindAdapter for SimpleKind {
    fn coordinate(&self) -> ResourceKindCoordinate {
        ResourceKindCoordinate::parse(self.key, contracts::CONTRACT_VERSION_1)
            .expect("static resource kind")
    }

    fn primitive(&self) -> ResourceKindPrimitive {
        self.primitive
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn category(&self) -> ResourceCategory {
        self.category
    }

    fn validate_attributes(&self, attributes: &BoundedPortableObject) -> Result<(), String> {
        reject_credential_shapes(attributes)?;
        match self.entry_type {
            Some(expected) => {
                let typed: ProjectContentAttributesV1 = attributes.decode()?;
                if typed
                    .entry_type
                    .as_deref()
                    .is_some_and(|actual| actual != expected)
                {
                    return Err(format!(
                        "{} attributes declare entry type '{}'",
                        self.label,
                        typed.entry_type.unwrap_or_default()
                    ));
                }
                validate_optional_digest(&typed.content_sha256)
            }
            None => {
                let typed: DigestOnlyAttributesV1 = attributes.decode()?;
                validate_optional_digest(&typed.content_sha256)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Apply policies
// ---------------------------------------------------------------------------

struct SimplePolicy {
    key: &'static str,
    primitive: ApplyPolicyPrimitive,
    label: &'static str,
}

impl ApplyPolicyAdapter for SimplePolicy {
    fn coordinate(&self) -> ApplyPolicyCoordinate {
        ApplyPolicyCoordinate::parse(self.key, contracts::CONTRACT_VERSION_1)
            .expect("static apply policy")
    }

    fn primitive(&self) -> ApplyPolicyPrimitive {
        self.primitive
    }

    fn label(&self) -> &'static str {
        self.label
    }
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectLocalProvenanceV1 {
    relative_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnknownProvenanceV1 {}

pub struct ProjectLocalProvenance;

impl ProvenanceCodec for ProjectLocalProvenance {
    fn kind(&self) -> ProvenanceKindKey {
        ProvenanceKindKey::parse(contracts::PROJECT_LOCAL).expect("static provenance kind")
    }

    fn version(&self) -> u32 {
        contracts::CONTRACT_VERSION_1
    }

    fn decode(&self, fields: &BoundedPortableObject) -> Result<ProvenancePrimitive, String> {
        reject_credential_shapes(fields)?;
        let typed: ProjectLocalProvenanceV1 = fields.decode()?;
        let path = Path::new(&typed.relative_path);
        if typed.relative_path.is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err("invalid project-local provenance path".to_string());
        }
        Ok(ProvenancePrimitive::ProjectLocal {
            relative_path: typed.relative_path,
        })
    }
}

pub struct UnknownProvenance;

impl ProvenanceCodec for UnknownProvenance {
    fn kind(&self) -> ProvenanceKindKey {
        ProvenanceKindKey::parse(contracts::UNKNOWN).expect("static provenance kind")
    }

    fn version(&self) -> u32 {
        contracts::CONTRACT_VERSION_1
    }

    fn decode(&self, fields: &BoundedPortableObject) -> Result<ProvenancePrimitive, String> {
        let UnknownProvenanceV1 {} = fields.decode()?;
        Ok(ProvenancePrimitive::Unknown)
    }
}

// ---------------------------------------------------------------------------
// Tombstone targets
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceIdentityV1 {
    resource_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathIdentityV1 {
    resource_id: String,
    logical_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadIdentityV1 {
    thread_id: String,
}

#[derive(Clone, Copy)]
enum PathTargetShape {
    File,
    ProjectContentFile,
    ProjectContentDirectory,
}

struct TombstoneCodec {
    key: &'static str,
    shape: Option<PathTargetShape>,
    thread: bool,
}

impl TombstoneTargetCodec for TombstoneCodec {
    fn kind(&self) -> TombstoneTargetKindKey {
        TombstoneTargetKindKey::parse(self.key).expect("static tombstone target kind")
    }

    fn version(&self) -> u32 {
        contracts::CONTRACT_VERSION_1
    }

    fn decode(&self, identity: &BoundedPortableObject) -> Result<TombstoneTargetPrimitive, String> {
        reject_credential_shapes(identity)?;
        if self.thread {
            let typed: ThreadIdentityV1 = identity.decode()?;
            return Ok(TombstoneTargetPrimitive::ConversationThread {
                thread_id: CloudThreadId::parse(typed.thread_id)?,
            });
        }
        match self.shape {
            None => {
                let typed: ResourceIdentityV1 = identity.decode()?;
                Ok(TombstoneTargetPrimitive::Resource {
                    resource_id: ResourceId::parse(typed.resource_id)?,
                })
            }
            Some(shape) => {
                let typed: PathIdentityV1 = identity.decode()?;
                let resource_id = ResourceId::parse(typed.resource_id)?;
                let logical_path = LogicalPath::parse(typed.logical_path)?;
                Ok(match shape {
                    PathTargetShape::File => TombstoneTargetPrimitive::File {
                        resource_id,
                        logical_path,
                    },
                    PathTargetShape::ProjectContentFile => {
                        TombstoneTargetPrimitive::ProjectContentFile {
                            resource_id,
                            logical_path,
                        }
                    }
                    PathTargetShape::ProjectContentDirectory => {
                        TombstoneTargetPrimitive::ProjectContentDirectory {
                            resource_id,
                            logical_path,
                        }
                    }
                })
            }
        }
    }
}

/// Every version-1 registration.
pub fn version_1() -> PortableRegistrations {
    PortableRegistrations {
        resource_kinds: vec![
            Arc::new(SimpleKind {
                key: contracts::CONVERSATION,
                primitive: ResourceKindPrimitive::Conversation,
                label: "Session",
                category: ResourceCategory::Conversations,
                entry_type: None,
            }),
            Arc::new(SimpleKind {
                key: contracts::PROVIDER_STATE,
                primitive: ResourceKindPrimitive::ProviderState,
                label: "Provider state",
                category: ResourceCategory::Conversations,
                entry_type: None,
            }),
            Arc::new(SimpleKind {
                key: contracts::PROJECT_CONTENT_FILE,
                primitive: ResourceKindPrimitive::ProjectContentFile,
                label: "Project file",
                category: ResourceCategory::ProjectFiles,
                entry_type: Some("file"),
            }),
            Arc::new(SimpleKind {
                key: contracts::PROJECT_CONTENT_DIRECTORY,
                primitive: ResourceKindPrimitive::ProjectContentDirectory,
                label: "Project folder",
                category: ResourceCategory::ProjectFiles,
                entry_type: Some("directory"),
            }),
            Arc::new(SimpleKind {
                key: contracts::REQUIREMENT,
                primitive: ResourceKindPrimitive::Requirement,
                label: "Requirement",
                category: ResourceCategory::Tools,
                entry_type: None,
            }),
        ],
        apply_policies: vec![
            Arc::new(SimplePolicy {
                key: contracts::SAFE_FILE,
                primitive: ApplyPolicyPrimitive::SafeFile,
                label: "Applied when safe",
            }),
            Arc::new(SimplePolicy {
                key: contracts::MERGE,
                primitive: ApplyPolicyPrimitive::Merge,
                label: "Merged on Pull",
            }),
            Arc::new(SimplePolicy {
                key: contracts::EXPLICIT_REVIEW,
                primitive: ApplyPolicyPrimitive::ExplicitReview,
                label: "Applied after review",
            }),
            Arc::new(SimplePolicy {
                key: contracts::MANUAL_ONLY,
                primitive: ApplyPolicyPrimitive::ManualOnly,
                label: "Manual apply only",
            }),
            Arc::new(SimplePolicy {
                key: contracts::NEVER,
                primitive: ApplyPolicyPrimitive::Never,
                label: "Never applied",
            }),
        ],
        provenance_kinds: vec![
            Arc::new(ProjectLocalProvenance),
            Arc::new(UnknownProvenance),
        ],
        tombstone_targets: vec![
            Arc::new(TombstoneCodec {
                key: contracts::RESOURCE,
                shape: None,
                thread: false,
            }),
            Arc::new(TombstoneCodec {
                key: contracts::FILE,
                shape: Some(PathTargetShape::File),
                thread: false,
            }),
            Arc::new(TombstoneCodec {
                key: contracts::PROJECT_CONTENT_FILE,
                shape: Some(PathTargetShape::ProjectContentFile),
                thread: false,
            }),
            Arc::new(TombstoneCodec {
                key: contracts::PROJECT_CONTENT_DIRECTORY,
                shape: Some(PathTargetShape::ProjectContentDirectory),
                thread: false,
            }),
            Arc::new(TombstoneCodec {
                key: contracts::CONVERSATION_THREAD,
                shape: None,
                thread: true,
            }),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::super::registry::PortableResourceRegistry;
    use super::*;
    use mallard_domain::PortableContractResolver;

    #[test]
    fn typed_attributes_reject_unknown_fields_and_credential_shapes() {
        let registry = PortableResourceRegistry::production();
        let conversation = registry
            .require_resource_kind(&contracts::conversation())
            .unwrap();
        conversation
            .validate_attributes(&BoundedPortableObject::empty())
            .unwrap();
        conversation
            .validate_attributes(
                &BoundedPortableObject::from_json(&serde_json::json!({
                    "content_sha256": "a".repeat(64)
                }))
                .unwrap(),
            )
            .unwrap();
        assert!(conversation
            .validate_attributes(
                &BoundedPortableObject::from_json(&serde_json::json!({ "cwd": "relative" }))
                    .unwrap()
            )
            .is_err());
        assert!(conversation
            .validate_attributes(
                &BoundedPortableObject::from_json(&serde_json::json!({
                    "content_sha256": "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
                }))
                .unwrap()
            )
            .is_err());
        let file = registry
            .require_resource_kind(&contracts::project_content_file())
            .unwrap();
        assert!(file
            .validate_attributes(
                &BoundedPortableObject::from_json(
                    &serde_json::json!({ "entry_type": "directory" })
                )
                .unwrap()
            )
            .is_err());
    }

    #[test]
    fn codecs_type_provenance_and_tombstone_identities() {
        let registry = PortableResourceRegistry::production();
        let local = contracts::provenance_project_local("docs/notes.md").unwrap();
        assert_eq!(
            registry.provenance(&local).unwrap(),
            Some(ProvenancePrimitive::ProjectLocal {
                relative_path: "docs/notes.md".to_string()
            })
        );
        let escaping = mallard_domain::ProvenanceEnvelope {
            fields: BoundedPortableObject::from_json(&serde_json::json!({"relative_path": "../x"}))
                .unwrap(),
            ..local.clone()
        };
        assert!(registry.provenance(&escaping).is_err());
        let extra = mallard_domain::ProvenanceEnvelope {
            fields: BoundedPortableObject::from_json(
                &serde_json::json!({"relative_path": "a", "extra": 1}),
            )
            .unwrap(),
            ..local
        };
        assert!(registry.provenance(&extra).is_err());

        let resource_id = ResourceId::parse("resource-a").unwrap();
        let path = LogicalPath::parse("project/README.md").unwrap();
        assert_eq!(
            registry
                .tombstone_target(
                    &contracts::tombstone_project_content_file(&resource_id, &path).unwrap()
                )
                .unwrap(),
            Some(TombstoneTargetPrimitive::ProjectContentFile {
                resource_id: resource_id.clone(),
                logical_path: path.clone()
            })
        );
        let thread = CloudThreadId::parse("cloudthread-v1").unwrap();
        assert_eq!(
            registry
                .tombstone_target(&contracts::tombstone_conversation_thread(&thread).unwrap())
                .unwrap(),
            Some(TombstoneTargetPrimitive::ConversationThread { thread_id: thread })
        );
        let future = mallard_domain::TombstoneTargetEnvelope {
            kind: TombstoneTargetKindKey::parse("future_target").unwrap(),
            version: 1,
            identity: BoundedPortableObject::from_json(&serde_json::json!({"id": "x"})).unwrap(),
        };
        assert_eq!(registry.tombstone_target(&future).unwrap(), None);
    }
}
