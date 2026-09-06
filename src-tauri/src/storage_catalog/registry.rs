//! The compile-time Library registry: provider adapters keyed by provider,
//! and capabilities keyed by the exact (provider, resource kind, format,
//! format version) coordinate. Each capability joins a provider adapter and a
//! format codec and declares its targets, capture protocol, and apply
//! maturity.
//!
//! Registries are code, not state: every process builds this from compiled
//! registrations, duplicate registrations are startup errors, and common
//! catalog code dispatches only through lookups here. Unknown coordinates
//! pass structural validation elsewhere; they cannot pass registry
//! validation and cannot execute.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde::Serialize;

use super::adapters::{
    CatalogProviderAdapter, CatalogProviderDescriptor, ClaudeCatalogAdapter, CodexCatalogAdapter,
};
use super::agent_skill::AgentSkillCodec;
use super::codec::CatalogFormatCodec;
use super::codex_pet::CodexPetCodec;
use super::domain::{
    CatalogCoordinate, CatalogCoordinateLabels, CatalogFormatKey, CatalogResourceKindKey,
    CatalogResourceRepositoryId, CatalogTargetKey, ProviderKey,
};
use super::plugin_intent::PluginInstallIntentCodec;

/// How the common workflow discovers and captures resources of a
/// capability. A protocol shape, never a provider, kind, or format identity.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSupport {
    /// Every child directory of each target root is one candidate the format
    /// codec captures.
    DirectoryTree,
    /// The provider adapter inventories native state; nothing is read from a
    /// directory.
    ProviderInventory,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplySupport {
    Unsupported,
    Manual,
    Automatic,
}

/// Icon and label hints the frontend renders without holding its own
/// provider, kind, or format tables.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogPresentation {
    pub icon: String,
    pub kind_label: String,
    pub format_label: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogCapabilityDescriptor {
    pub provider: ProviderKey,
    pub provider_label: String,
    pub resource_kind: CatalogResourceKindKey,
    pub format: CatalogFormatKey,
    pub format_version: u32,
    pub label: String,
    pub singular_label: String,
    pub presentation: CatalogPresentation,
    pub capture: CaptureSupport,
    pub apply: ApplySupport,
    pub targets: Vec<CatalogTargetKey>,
}

impl CatalogCapabilityDescriptor {
    pub fn coordinate(&self) -> CatalogCoordinate {
        CatalogCoordinate {
            provider: self.provider.clone(),
            resource_kind: self.resource_kind.clone(),
            format: self.format.clone(),
            format_version: self.format_version,
        }
    }

    pub fn labels(&self) -> CatalogCoordinateLabels {
        CatalogCoordinateLabels {
            provider_label: self.provider_label.clone(),
            kind_label: self.presentation.kind_label.clone(),
            format_label: self.presentation.format_label.clone(),
        }
    }
}

pub struct CatalogCapabilityRegistration {
    pub coordinate: CatalogCoordinate,
    pub label: String,
    pub singular_label: String,
    pub presentation: CatalogPresentation,
    pub capture: CaptureSupport,
    pub apply: ApplySupport,
    pub targets: Vec<CatalogTargetKey>,
    pub codec: Arc<dyn CatalogFormatCodec>,
}

pub struct CatalogCapability {
    pub descriptor: CatalogCapabilityDescriptor,
    pub codec: Arc<dyn CatalogFormatCodec>,
    pub adapter: Arc<dyn CatalogProviderAdapter>,
}

impl CatalogCapability {
    pub fn coordinate(&self) -> CatalogCoordinate {
        self.descriptor.coordinate()
    }

    pub fn labels(&self) -> CatalogCoordinateLabels {
        self.descriptor.labels()
    }

    pub fn repository_id(&self) -> CatalogResourceRepositoryId {
        CatalogResourceRepositoryId::for_coordinate(&self.coordinate())
    }

    pub fn provider(&self) -> &ProviderKey {
        &self.descriptor.provider
    }

    pub fn declares_target(&self, target: &CatalogTargetKey) -> bool {
        self.descriptor.targets.contains(target)
    }
}

pub struct CatalogRegistry {
    providers: BTreeMap<ProviderKey, Arc<dyn CatalogProviderAdapter>>,
    /// Display order: provider rank first, then registration order.
    capabilities: Vec<Arc<CatalogCapability>>,
    index: BTreeMap<CatalogCoordinate, usize>,
}

fn target(key: &str) -> CatalogTargetKey {
    CatalogTargetKey::parse(key).expect("static target keys are valid")
}

fn presentation(icon: &str, kind_label: &str, format_label: &str) -> CatalogPresentation {
    CatalogPresentation {
        icon: icon.to_string(),
        kind_label: kind_label.to_string(),
        format_label: format_label.to_string(),
    }
}

impl CatalogRegistry {
    /// Build a registry, refusing a repeated provider, a repeated coordinate,
    /// a capability whose provider is not registered, an empty target list,
    /// or a zero format version.
    pub fn from_registrations(
        adapters: Vec<Arc<dyn CatalogProviderAdapter>>,
        registrations: Vec<CatalogCapabilityRegistration>,
    ) -> Result<Self, String> {
        let mut providers: BTreeMap<ProviderKey, Arc<dyn CatalogProviderAdapter>> = BTreeMap::new();
        for adapter in adapters {
            let descriptor = adapter.descriptor();
            if providers.insert(descriptor.key.clone(), adapter).is_some() {
                return Err(format!(
                    "Library provider '{}' is registered twice",
                    descriptor.key
                ));
            }
        }
        let mut capabilities = Vec::new();
        let mut index = BTreeMap::new();
        for (order, registration) in registrations.into_iter().enumerate() {
            registration.coordinate.validate()?;
            let adapter = providers
                .get(&registration.coordinate.provider)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "Library capability {} names an unregistered provider",
                        registration.coordinate
                    )
                })?;
            if registration.targets.is_empty() {
                return Err(format!(
                    "Library capability {} declares no target",
                    registration.coordinate
                ));
            }
            if index.contains_key(&registration.coordinate) {
                return Err(format!(
                    "Library capability {} is registered twice",
                    registration.coordinate
                ));
            }
            let provider = adapter.descriptor();
            let capability = Arc::new(CatalogCapability {
                descriptor: CatalogCapabilityDescriptor {
                    provider: registration.coordinate.provider.clone(),
                    provider_label: provider.label.clone(),
                    resource_kind: registration.coordinate.resource_kind.clone(),
                    format: registration.coordinate.format.clone(),
                    format_version: registration.coordinate.format_version,
                    label: registration.label,
                    singular_label: registration.singular_label,
                    presentation: registration.presentation,
                    capture: registration.capture,
                    apply: registration.apply,
                    targets: registration.targets,
                },
                codec: registration.codec,
                adapter,
            });
            index.insert(registration.coordinate, capabilities.len());
            capabilities.push((provider.display_rank, order, capability));
        }
        capabilities.sort_by_key(|(rank, order, _)| (*rank, *order));
        let capabilities: Vec<Arc<CatalogCapability>> = capabilities
            .into_iter()
            .map(|(_, _, capability)| capability)
            .collect();
        let index = capabilities
            .iter()
            .enumerate()
            .map(|(position, capability)| (capability.coordinate(), position))
            .collect();
        Ok(Self {
            providers,
            capabilities,
            index,
        })
    }

    fn production_adapters() -> Vec<Arc<dyn CatalogProviderAdapter>> {
        vec![
            Arc::new(CodexCatalogAdapter),
            Arc::new(ClaudeCatalogAdapter),
        ]
    }

    /// The version-1 registrations: Codex and Claude skills, the shared
    /// plug-in intent format, and the two Codex pet format versions.
    fn production_registrations() -> Vec<CatalogCapabilityRegistration> {
        let skill_codec: Arc<dyn CatalogFormatCodec> = Arc::new(AgentSkillCodec);
        let plugin_codec: Arc<dyn CatalogFormatCodec> = Arc::new(PluginInstallIntentCodec);
        let skills =
            |provider: &str, targets: Vec<CatalogTargetKey>| CatalogCapabilityRegistration {
                coordinate: CatalogCoordinate::new(
                    provider,
                    super::agent_skill::KIND_KEY,
                    super::agent_skill::FORMAT_KEY,
                    1,
                )
                .expect("static coordinate"),
                label: "Skills".to_string(),
                singular_label: "Skill".to_string(),
                presentation: presentation("skill", "Skills", "Agent Skill"),
                capture: CaptureSupport::DirectoryTree,
                apply: ApplySupport::Automatic,
                targets,
                codec: skill_codec.clone(),
            };
        let plugins = |provider: &str| CatalogCapabilityRegistration {
            coordinate: CatalogCoordinate::new(
                provider,
                super::plugin_intent::KIND_KEY,
                super::plugin_intent::FORMAT_KEY,
                super::plugin_intent::FORMAT_VERSION,
            )
            .expect("static coordinate"),
            label: "Plugins".to_string(),
            singular_label: "Plugin".to_string(),
            presentation: presentation("plugin", "Plugins", "Plugin install intent"),
            capture: CaptureSupport::ProviderInventory,
            apply: ApplySupport::Automatic,
            targets: vec![target("user.plugins")],
            codec: plugin_codec.clone(),
        };
        let pets = |version: u32| CatalogCapabilityRegistration {
            coordinate: CatalogCoordinate::new(
                "codex",
                super::codex_pet::KIND_KEY,
                super::codex_pet::FORMAT_KEY,
                version,
            )
            .expect("static coordinate"),
            label: format!("Pets v{version}"),
            singular_label: format!("Pet v{version}"),
            // Two formats of one kind are two rows in every list, so each
            // version past the first wears its own hint and the frontend can
            // tell them apart without reading the label.
            presentation: presentation(
                &if version == 1 {
                    "pet".to_string()
                } else {
                    format!("pet-v{version}")
                },
                "Pets",
                &format!("Codex pet v{version}"),
            ),
            capture: CaptureSupport::DirectoryTree,
            apply: ApplySupport::Automatic,
            targets: vec![target("desktop.pets")],
            codec: Arc::new(CodexPetCodec::for_version(version).expect("registered pet versions")),
        };
        vec![
            skills(
                "codex",
                vec![target("provider-home.skills"), target("shared-user.skills")],
            ),
            plugins("codex"),
            pets(1),
            pets(2),
            skills("claude", vec![target("provider-home.skills")]),
            plugins("claude"),
        ]
    }

    pub fn production() -> Arc<CatalogRegistry> {
        static REGISTRY: OnceLock<Arc<CatalogRegistry>> = OnceLock::new();
        REGISTRY
            .get_or_init(|| {
                Arc::new(
                    CatalogRegistry::from_registrations(
                        Self::production_adapters(),
                        Self::production_registrations(),
                    )
                    .expect("production Library registry has no duplicate registrations"),
                )
            })
            .clone()
    }

    /// A registry that additionally exposes the test provider, kind, format,
    /// version, and target: the proof that an addition needs no common-code
    /// change.
    #[cfg(test)]
    pub fn with_test_capability() -> Arc<CatalogRegistry> {
        let mut adapters = Self::production_adapters();
        adapters.push(Arc::new(test_support::TestCatalogAdapter));
        let mut registrations = Self::production_registrations();
        registrations.push(test_support::registration());
        Arc::new(
            CatalogRegistry::from_registrations(adapters, registrations)
                .expect("test Library registry has no duplicate registrations"),
        )
    }

    pub fn provider(&self, key: &ProviderKey) -> Option<&dyn CatalogProviderAdapter> {
        self.providers.get(key).map(Arc::as_ref)
    }

    pub fn require_provider(
        &self,
        key: &ProviderKey,
    ) -> Result<&dyn CatalogProviderAdapter, String> {
        self.provider(key)
            .ok_or_else(|| format!("no Library provider is registered for '{}'", key))
    }

    /// Provider descriptors in display order: rank first, then key.
    pub fn providers(&self) -> Vec<CatalogProviderDescriptor> {
        let mut descriptors: Vec<CatalogProviderDescriptor> = self
            .providers
            .values()
            .map(|adapter| adapter.descriptor())
            .collect();
        descriptors.sort_by(|left, right| {
            left.display_rank
                .cmp(&right.display_rank)
                .then_with(|| left.key.cmp(&right.key))
        });
        descriptors
    }

    pub fn capability(&self, coordinate: &CatalogCoordinate) -> Option<Arc<CatalogCapability>> {
        self.index
            .get(coordinate)
            .map(|position| self.capabilities[*position].clone())
    }

    /// Registry validation: the exact coordinate must be compiled in.
    pub fn require_capability(
        &self,
        coordinate: &CatalogCoordinate,
    ) -> Result<Arc<CatalogCapability>, String> {
        self.capability(coordinate).ok_or_else(|| {
            if self.providers.contains_key(&coordinate.provider) {
                format!(
                    "Library coordinate {} is not supported by this build",
                    coordinate
                )
            } else {
                format!(
                    "no Library provider is registered for '{}'",
                    coordinate.provider
                )
            }
        })
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &Arc<CatalogCapability>> {
        self.capabilities.iter()
    }

    pub fn descriptors(&self) -> Vec<CatalogCapabilityDescriptor> {
        self.capabilities
            .iter()
            .map(|capability| capability.descriptor.clone())
            .collect()
    }

    /// The other registered versions of one coordinate's format that share a
    /// target with it — the codecs a directory in that target may belong to.
    pub fn siblings(&self, capability: &CatalogCapability) -> Vec<Arc<CatalogCapability>> {
        let coordinate = capability.coordinate();
        self.capabilities
            .iter()
            .filter(|candidate| {
                let other = candidate.coordinate();
                other != coordinate
                    && other.shares_format(&coordinate)
                    && candidate
                        .descriptor
                        .targets
                        .iter()
                        .any(|target| capability.declares_target(target))
            })
            .cloned()
            .collect()
    }

    /// Labels for any coordinate: the registered presentation when it is
    /// compiled in, the raw keys otherwise so an unavailable record still
    /// names itself.
    pub fn labels_for(&self, coordinate: &CatalogCoordinate) -> CatalogCoordinateLabels {
        match self.capability(coordinate) {
            Some(capability) => capability.labels(),
            None => CatalogCoordinateLabels {
                provider_label: self
                    .provider(&coordinate.provider)
                    .map(|adapter| adapter.descriptor().label)
                    .unwrap_or_else(|| coordinate.provider.to_string()),
                kind_label: coordinate.resource_kind.to_string(),
                format_label: format!("{}@{}", coordinate.format, coordinate.format_version),
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A provider, kind, format, version, and target that exist only in the
    //! test registry. Nothing in common catalog code names any of them.

    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;

    use serde::{Deserialize, Serialize};

    use super::super::adapters::{
        CatalogProviderAdapter, CatalogProviderDescriptor, PluginProcessRunner, ProviderInventory,
        ResolvedCatalogTarget,
    };
    use super::super::codec::{
        content_warnings, read_directory_tree, CapturedResource, CatalogFormatCodec, TreeLimits,
    };
    use super::super::domain::{
        validate_component_name, BoundedPortableObject, CatalogBlocker, CatalogCoordinate,
        CatalogResourceEnvelope, CatalogTargetKey, NativePluginPlan, PluginApplyMode, ProviderKey,
    };
    use super::super::profiles::prospective_canonical;
    use super::{presentation, ApplySupport, CaptureSupport, CatalogCapabilityRegistration};

    pub const PROVIDER: &str = "testprov";
    pub const KIND: &str = "test_note";
    pub const FORMAT: &str = "mallard.test-note";
    pub const VERSION: u32 = 1;
    pub const TARGET: &str = "provider-home.notes";
    pub const DEFAULT_DIRECTORY: &str = ".testprov";

    pub fn coordinate() -> CatalogCoordinate {
        CatalogCoordinate::new(PROVIDER, KIND, FORMAT, VERSION).unwrap()
    }

    pub fn provider_key() -> ProviderKey {
        ProviderKey::parse(PROVIDER).unwrap()
    }

    pub struct TestCatalogAdapter;

    impl CatalogProviderAdapter for TestCatalogAdapter {
        fn descriptor(&self) -> CatalogProviderDescriptor {
            CatalogProviderDescriptor {
                key: provider_key(),
                label: "Test Provider".to_string(),
                default_profile_directory: Some(DEFAULT_DIRECTORY.to_string()),
                display_rank: 90,
            }
        }

        fn resolve_targets(
            &self,
            canonical_home: &Path,
            _os_user_home: &Path,
        ) -> Result<Vec<ResolvedCatalogTarget>, CatalogBlocker> {
            Ok(vec![ResolvedCatalogTarget {
                target: CatalogTargetKey::parse(TARGET).unwrap(),
                canonical_root: prospective_canonical(&canonical_home.join("notes"))
                    .map_err(|detail| CatalogBlocker::ProfileUnavailable { detail })?,
                root_class: "provider-home".to_string(),
                summary: "Test notes".to_string(),
                exclude_provider_owned: false,
            }])
        }

        fn launch_home_app(&self, _home: &Path, _os_user_home: &Path) -> Result<(), String> {
            Err("the test provider has no desktop app".to_string())
        }

        fn launch_home_terminal(&self, _home: &Path) -> Result<(), String> {
            Err("the test provider has no terminal launcher".to_string())
        }

        fn discover_native(
            &self,
            _home: &Path,
            _runner: &dyn PluginProcessRunner,
        ) -> Result<ProviderInventory, CatalogBlocker> {
            Ok(ProviderInventory::default())
        }

        fn native_resources(&self, _inventory: &ProviderInventory) -> Vec<CatalogResourceEnvelope> {
            Vec::new()
        }

        fn native_state(
            &self,
            _envelope: &CatalogResourceEnvelope,
            _inventory: &ProviderInventory,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn plan_native_apply(
            &self,
            _envelope: &CatalogResourceEnvelope,
            _inventory: &ProviderInventory,
            _mode: PluginApplyMode,
        ) -> Result<NativePluginPlan, CatalogBlocker> {
            Err(CatalogBlocker::NativeContractUnverified {
                provider: provider_key(),
                operation: "native apply".to_string(),
                detail: "the test provider has no native resources".to_string(),
            })
        }
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct NoteIdentity {
        name: String,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct NoteDescriptor {
        title: String,
    }

    /// A directory holding `NOTE.md`; the first line is its title.
    pub struct TestNoteCodec;

    const LIMITS: TreeLimits = TreeLimits {
        max_files: 16,
        max_file_bytes: 64 * 1024,
        max_total_bytes: 256 * 1024,
    };

    fn decode(
        envelope: &CatalogResourceEnvelope,
    ) -> Result<(NoteIdentity, NoteDescriptor), String> {
        let identity: NoteIdentity = envelope.identity.decode()?;
        let descriptor: NoteDescriptor = envelope.descriptor.decode()?;
        validate_component_name("note name", &identity.name, 64)?;
        Ok((identity, descriptor))
    }

    fn capture(
        name: &str,
        files: BTreeMap<String, super::super::codec::CapturedFile>,
    ) -> Result<CapturedResource, String> {
        validate_component_name("note name", name, 64)?;
        let note = files.get("NOTE.md").ok_or("no NOTE.md; not a test note")?;
        let title = String::from_utf8_lossy(&note.bytes)
            .lines()
            .next()
            .unwrap_or("")
            .trim_start_matches('#')
            .trim()
            .to_string();
        let title = if title.is_empty() {
            name.to_string()
        } else {
            title
        };
        let warnings = content_warnings(&files);
        Ok(CapturedResource {
            identity: BoundedPortableObject::encode(&NoteIdentity {
                name: name.to_string(),
            })?,
            descriptor: BoundedPortableObject::encode(&NoteDescriptor {
                title: title.clone(),
            })?,
            files,
            display_name: title,
            warnings,
            notes: Vec::new(),
        })
    }

    impl CatalogFormatCodec for TestNoteCodec {
        fn validate(&self, envelope: &CatalogResourceEnvelope) -> Result<(), String> {
            decode(envelope)?;
            if envelope.file("NOTE.md").is_none() {
                return Err("test note has no NOTE.md".to_string());
            }
            Ok(())
        }

        fn display_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
            Ok(decode(envelope)?.1.title)
        }

        fn member_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
            Ok(decode(envelope)?.0.name)
        }

        fn capture_directory(
            &self,
            directory: &Path,
            _root_class: &str,
        ) -> Result<CapturedResource, String> {
            let name = directory
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("note directory name is not UTF-8")?;
            capture(name, read_directory_tree(directory, &LIMITS)?)
        }

        fn rename(
            &self,
            captured: &CapturedResource,
            new_name: &str,
        ) -> Result<CapturedResource, String> {
            capture(new_name, captured.files.clone())
        }
    }

    pub fn registration() -> CatalogCapabilityRegistration {
        CatalogCapabilityRegistration {
            coordinate: coordinate(),
            label: "Notes".to_string(),
            singular_label: "Note".to_string(),
            presentation: presentation("note", "Notes", "Test note"),
            capture: CaptureSupport::DirectoryTree,
            apply: ApplySupport::Automatic,
            targets: vec![CatalogTargetKey::parse(TARGET).unwrap()],
            codec: Arc::new(TestNoteCodec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinate(provider: &str, kind: &str, format: &str, version: u32) -> CatalogCoordinate {
        CatalogCoordinate::new(provider, kind, format, version).unwrap()
    }

    #[test]
    fn production_registers_the_version_1_capabilities_in_display_order() {
        let registry = CatalogRegistry::production();
        let listed: Vec<String> = registry
            .descriptors()
            .iter()
            .map(|descriptor| {
                format!(
                    "{}/{}/{}@{} -> {}",
                    descriptor.provider,
                    descriptor.resource_kind,
                    descriptor.format,
                    descriptor.format_version,
                    descriptor
                        .targets
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect();
        assert_eq!(
            listed,
            vec![
                "codex/agent_skill/agentskills.io.skill@1 -> provider-home.skills,shared-user.skills",
                "codex/plugin_install_intent/mallard.plugin-install-intent@1 -> user.plugins",
                "codex/desktop_pet/openai.codex-pet@1 -> desktop.pets",
                "codex/desktop_pet/openai.codex-pet@2 -> desktop.pets",
                "claude/agent_skill/agentskills.io.skill@1 -> provider-home.skills",
                "claude/plugin_install_intent/mallard.plugin-install-intent@1 -> user.plugins",
            ]
        );
        let descriptors = registry.descriptors();
        assert_eq!(descriptors[0].provider_label, "Codex");
        assert_eq!(descriptors[0].presentation.kind_label, "Skills");
        assert_eq!(descriptors[0].singular_label, "Skill");
        assert_eq!(descriptors[1].capture, CaptureSupport::ProviderInventory);
        assert_eq!(descriptors[2].label, "Pets v1");
        assert_eq!(descriptors[3].apply, ApplySupport::Automatic);
        assert_eq!(
            registry
                .providers()
                .iter()
                .map(|provider| provider.key.to_string())
                .collect::<Vec<_>>(),
            vec!["codex", "claude"]
        );
    }

    #[test]
    fn duplicate_registrations_are_rejected() {
        let mut registrations = CatalogRegistry::production_registrations();
        registrations.push(CatalogRegistry::production_registrations().remove(0));
        let error = CatalogRegistry::from_registrations(
            CatalogRegistry::production_adapters(),
            registrations,
        )
        .err()
        .expect("duplicate registration is rejected");
        assert_eq!(
            error,
            "Library capability codex/agent_skill/agentskills.io.skill@1 is registered twice"
        );
        let mut adapters = CatalogRegistry::production_adapters();
        adapters.push(Arc::new(CodexCatalogAdapter));
        assert_eq!(
            CatalogRegistry::from_registrations(adapters, Vec::new())
                .err()
                .as_deref(),
            Some("Library provider 'codex' is registered twice")
        );
    }

    #[test]
    fn registrations_need_a_registered_provider_a_target_and_a_positive_version() {
        let mut orphan = CatalogRegistry::production_registrations().remove(0);
        orphan.coordinate = coordinate("cursor", "agent_skill", "agentskills.io.skill", 1);
        assert!(CatalogRegistry::from_registrations(
            CatalogRegistry::production_adapters(),
            vec![orphan]
        )
        .err()
        .unwrap()
        .contains("unregistered provider"));
        let mut targetless = CatalogRegistry::production_registrations().remove(0);
        targetless.targets.clear();
        assert!(CatalogRegistry::from_registrations(
            CatalogRegistry::production_adapters(),
            vec![targetless]
        )
        .err()
        .unwrap()
        .contains("declares no target"));
        let mut zero = CatalogRegistry::production_registrations().remove(0);
        zero.coordinate.format_version = 0;
        assert!(CatalogRegistry::from_registrations(
            CatalogRegistry::production_adapters(),
            vec![zero]
        )
        .err()
        .unwrap()
        .contains("format version 0"));
    }

    #[test]
    fn invalid_combinations_and_unknown_coordinates_fail_registry_validation() {
        let registry = CatalogRegistry::production();
        // Pets are Codex-native: Claude cannot host the coordinate even
        // though every key in it is registered somewhere.
        let claude_pets = coordinate("claude", "desktop_pet", "openai.codex-pet", 1);
        assert_eq!(
            registry.require_capability(&claude_pets).err().as_deref(),
            Some("Library coordinate claude/desktop_pet/openai.codex-pet@1 is not supported by this build")
        );
        // A skill kind under the pet format, an unregistered version, and an
        // unregistered provider each fail distinctly.
        assert!(registry
            .require_capability(&coordinate("codex", "agent_skill", "openai.codex-pet", 1))
            .is_err());
        assert!(registry
            .require_capability(&coordinate("codex", "desktop_pet", "openai.codex-pet", 3))
            .is_err());
        assert_eq!(
            registry
                .require_capability(&coordinate("cursor", "agent_skill", "cursor.skill", 1))
                .err()
                .as_deref(),
            Some("no Library provider is registered for 'cursor'")
        );
        assert!(registry
            .require_provider(&ProviderKey::parse("cursor").unwrap())
            .is_err());
        // Unknown coordinates still get labels so an unavailable record can
        // name itself.
        let labels =
            registry.labels_for(&coordinate("codex", "desktop_pet", "openai.codex-pet", 3));
        assert_eq!(labels.provider_label, "Codex");
        assert_eq!(labels.kind_label, "desktop_pet");
        assert_eq!(labels.format_label, "openai.codex-pet@3");
        let labels = registry.labels_for(&coordinate("cursor", "agent_skill", "cursor.skill", 1));
        assert_eq!(labels.provider_label, "cursor");
    }

    #[test]
    fn siblings_are_the_other_versions_sharing_a_target() {
        let registry = CatalogRegistry::production();
        let v1 = registry
            .capability(&coordinate("codex", "desktop_pet", "openai.codex-pet", 1))
            .unwrap();
        let siblings: Vec<u32> = registry
            .siblings(&v1)
            .iter()
            .map(|capability| capability.descriptor.format_version)
            .collect();
        assert_eq!(siblings, vec![2]);
        let skills = registry
            .capability(&coordinate(
                "codex",
                "agent_skill",
                "agentskills.io.skill",
                1,
            ))
            .unwrap();
        assert!(registry.siblings(&skills).is_empty());
    }

    #[test]
    fn the_test_capability_registers_beside_production_without_common_changes() {
        let registry = CatalogRegistry::with_test_capability();
        let capability = registry
            .require_capability(&test_support::coordinate())
            .unwrap();
        assert_eq!(capability.descriptor.provider_label, "Test Provider");
        assert_eq!(
            capability.descriptor.targets[0].as_str(),
            test_support::TARGET
        );
        // It lists last, after every production provider, by display rank.
        assert_eq!(
            registry.descriptors().last().unwrap().provider.as_str(),
            test_support::PROVIDER
        );
        assert!(CatalogRegistry::production()
            .capability(&test_support::coordinate())
            .is_none());
    }
}
