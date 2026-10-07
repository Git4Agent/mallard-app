//! The `mallard.plugin-install-intent` format codec, version 1: the shared
//! portable shape of a Codex or Claude plug-in install intent — a
//! marketplace source, a canonical plug-in id, and a resolution policy.
//!
//! It is Mallard-defined because neither provider publishes a portable
//! intent format. Machine-local observations (installed version, install
//! root, exported skills, enablement) never enter it, so provider
//! auto-updates do not create catalog conflicts.

use serde::{Deserialize, Serialize};

use super::codec::{CapturedResource, CatalogFormatCodec, CatalogResourceDetail, PayloadReader};
use super::domain::{
    validate_display_text, BoundedPortableObject, CatalogCoordinate, CatalogResourceEnvelope,
    ProviderKey,
};

pub const FORMAT_KEY: &str = "mallard.plugin-install-intent";
pub const KIND_KEY: &str = "plugin_install_intent";
pub const FORMAT_VERSION: u32 = 1;

pub fn validate_plugin_namespace(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(format!("invalid {}: '{}'", label, value.escape_debug()));
    }
    Ok(())
}

/// A canonical plugin ID is the provider-native qualified form
/// `<plugin>@<namespace>`; the namespace participates in identity.
pub fn validate_canonical_plugin_id(label: &str, value: &str) -> Result<(), String> {
    let invalid = || format!("invalid {}: '{}'", label, value.escape_debug());
    if value.len() > 512 || value.contains("..") {
        return Err(invalid());
    }
    let Some((name, namespace)) = value.split_once('@') else {
        return Err(invalid());
    };
    if name.is_empty()
        || name.len() > 256
        || namespace.contains('@')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || !name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(invalid());
    }
    validate_plugin_namespace(label, namespace)
}

fn contains_url_credentials(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority_end = remainder.find('/').unwrap_or(remainder.len());
    remainder[..authority_end].contains('@')
}

/// A credential-free, normalized public Git location: an `https`/`git` URL or
/// an `owner/repo` shorthand. Absolute paths, embedded credentials, query
/// tokens, and shell fragments are rejected.
pub fn validate_git_location(label: &str, value: &str) -> Result<(), String> {
    let invalid = || format!("invalid {}: '{}'", label, value.escape_debug());
    if value.is_empty()
        || value.len() > 1024
        || value.chars().any(char::is_control)
        || value.contains(' ')
        || value.contains('?')
        || value.contains('#')
        || value.starts_with('/')
        || value.starts_with('.')
        || value.starts_with('~')
        || value.contains('\\')
        || contains_url_credentials(value)
    {
        return Err(invalid());
    }
    if let Some((scheme, _)) = value.split_once("://") {
        if !matches!(scheme, "https" | "git") {
            return Err(invalid());
        }
        return Ok(());
    }
    // owner/repo shorthand: exactly one separator, safe components.
    let parts: Vec<&str> = value.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(invalid());
    }
    Ok(())
}

/// How a plugin restore resolves content: through the provider's own
/// install/update operation, or manually when the source is machine-local or
/// otherwise unenforceable.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginResolutionPolicy {
    ProviderResolved,
    ManualOnly,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GitMarketplaceSource {
    pub location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

/// The portable plugin source forms of this format version. A
/// provider-managed marketplace identity always carries its namespace; a
/// credential-free Git location is present when the marketplace itself is
/// portable. A `None` Git source is a machine-local marketplace: the intent
/// stays portable but restores manually.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortablePluginSource {
    ProviderMarketplace {
        namespace: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git: Option<GitMarketplaceSource>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginInstallIntentIdentity {
    pub namespace: String,
    pub plugin_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginInstallIntentDescriptor {
    pub source: PortablePluginSource,
    pub resolution: PluginResolutionPolicy,
}

/// The typed pair one envelope of this format decodes into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInstallIntent {
    pub identity: PluginInstallIntentIdentity,
    pub descriptor: PluginInstallIntentDescriptor,
}

impl PluginInstallIntent {
    pub fn validate(&self) -> Result<(), String> {
        let identity = &self.identity;
        validate_plugin_namespace("plugin namespace", &identity.namespace)?;
        validate_canonical_plugin_id("plugin id", &identity.plugin_id)?;
        let Some((_, namespace)) = identity.plugin_id.split_once('@') else {
            return Err(format!(
                "plugin id '{}' has no namespace qualifier",
                identity.plugin_id
            ));
        };
        if namespace != identity.namespace {
            return Err(format!(
                "plugin id '{}' does not match its namespace '{}'",
                identity.plugin_id, identity.namespace
            ));
        }
        let PortablePluginSource::ProviderMarketplace {
            namespace: source_namespace,
            git,
        } = &self.descriptor.source;
        if source_namespace != &identity.namespace {
            return Err(format!(
                "plugin '{}' source namespace differs from its identity namespace",
                identity.plugin_id
            ));
        }
        match git {
            Some(git) => {
                validate_git_location("plugin marketplace source", &git.location)?;
                if let Some(git_ref) = &git.git_ref {
                    validate_display_text("plugin marketplace ref", git_ref, 256)?;
                }
                if self.descriptor.resolution != PluginResolutionPolicy::ProviderResolved {
                    return Err(format!(
                        "plugin '{}' with a portable marketplace source must be provider-resolved",
                        identity.plugin_id
                    ));
                }
            }
            None => {
                if self.descriptor.resolution != PluginResolutionPolicy::ManualOnly {
                    return Err(format!(
                        "plugin '{}' without a portable marketplace source must be manual-only",
                        identity.plugin_id
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn coordinate(provider: &ProviderKey) -> CatalogCoordinate {
        CatalogCoordinate::new(provider.as_str(), KIND_KEY, FORMAT_KEY, FORMAT_VERSION)
            .expect("the plugin intent coordinate keys are static")
    }

    /// The validated envelope of this intent under one provider.
    pub fn envelope(&self, provider: &ProviderKey) -> Result<CatalogResourceEnvelope, String> {
        self.validate()?;
        Ok(CatalogResourceEnvelope::new(
            &Self::coordinate(provider),
            BoundedPortableObject::encode(&self.identity)?,
            BoundedPortableObject::encode(&self.descriptor)?,
            Vec::new(),
        ))
    }

    pub fn decode(envelope: &CatalogResourceEnvelope) -> Result<Self, String> {
        let intent = Self {
            identity: envelope
                .identity
                .decode()
                .map_err(|error| format!("plugin intent identity: {}", error))?,
            descriptor: envelope
                .descriptor
                .decode()
                .map_err(|error| format!("plugin intent descriptor: {}", error))?,
        };
        intent.validate()?;
        Ok(intent)
    }
}

pub struct PluginInstallIntentCodec;

impl CatalogFormatCodec for PluginInstallIntentCodec {
    fn validate(&self, envelope: &CatalogResourceEnvelope) -> Result<(), String> {
        let intent = PluginInstallIntent::decode(envelope)?;
        if !envelope.files.is_empty() {
            return Err(format!(
                "plugin intent '{}' owns no payload files",
                intent.identity.plugin_id
            ));
        }
        Ok(())
    }

    fn display_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
        Ok(PluginInstallIntent::decode(envelope)?.identity.plugin_id)
    }

    fn member_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
        Ok(PluginInstallIntent::decode(envelope)?.identity.plugin_id)
    }

    fn collision_key(&self, _captured: &CapturedResource) -> Option<String> {
        None
    }

    /// The intent's page: the plugin and the marketplace it installs from.
    /// What the installed copy says about itself is this Mac's to add.
    fn detail(
        &self,
        envelope: &CatalogResourceEnvelope,
        _read_payload: &PayloadReader<'_>,
    ) -> Result<CatalogResourceDetail, String> {
        let intent = PluginInstallIntent::decode(envelope)?;
        let PortablePluginSource::ProviderMarketplace { git, .. } = intent.descriptor.source;
        Ok(CatalogResourceDetail::Install {
            plugin_id: intent.identity.plugin_id,
            marketplace: intent.identity.namespace,
            location: git.as_ref().map(|git| git.location.clone()),
            git_ref: git.and_then(|git| git.git_ref),
            install: Vec::new(),
            installed_version: None,
            listing: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(
        git: Option<GitMarketplaceSource>,
        resolution: PluginResolutionPolicy,
    ) -> PluginInstallIntent {
        PluginInstallIntent {
            identity: PluginInstallIntentIdentity {
                namespace: "acme".to_string(),
                plugin_id: "tools@acme".to_string(),
            },
            descriptor: PluginInstallIntentDescriptor {
                source: PortablePluginSource::ProviderMarketplace {
                    namespace: "acme".to_string(),
                    git,
                },
                resolution,
            },
        }
    }

    #[test]
    fn intents_round_trip_as_typed_envelopes_and_ignore_observations() {
        let provider = ProviderKey::parse("codex").unwrap();
        let resolved = intent(
            Some(GitMarketplaceSource {
                location: "https://example.com/acme/tools".to_string(),
                git_ref: None,
            }),
            PluginResolutionPolicy::ProviderResolved,
        );
        let envelope = resolved.envelope(&provider).unwrap();
        envelope.validate().unwrap();
        PluginInstallIntentCodec.validate(&envelope).unwrap();
        assert_eq!(
            PluginInstallIntentCodec.display_name(&envelope).unwrap(),
            "tools@acme"
        );
        assert_eq!(PluginInstallIntent::decode(&envelope).unwrap(), resolved);
        assert!(envelope.files.is_empty());

        // A source change alters semantic intent; the identity stays.
        let changed = intent(
            Some(GitMarketplaceSource {
                location: "https://example.com/acme/tools-next".to_string(),
                git_ref: None,
            }),
            PluginResolutionPolicy::ProviderResolved,
        )
        .envelope(&provider)
        .unwrap();
        assert_eq!(changed.resource_id, envelope.resource_id);
        assert_ne!(changed.content_sha256, envelope.content_sha256);

        // Its page names the plugin and where it installs from.
        assert_eq!(
            PluginInstallIntentCodec
                .detail(&envelope, &|_, _| Err("no payloads".to_string()))
                .unwrap(),
            CatalogResourceDetail::Install {
                plugin_id: "tools@acme".to_string(),
                marketplace: "acme".to_string(),
                location: Some("https://example.com/acme/tools".to_string()),
                git_ref: None,
                install: Vec::new(),
                installed_version: None,
                listing: None,
            }
        );

        // The same intent under another provider is another resource.
        let claude = resolved
            .envelope(&ProviderKey::parse("claude").unwrap())
            .unwrap();
        assert_ne!(claude.resource_id, envelope.resource_id);

        // Exact requirements are not part of the supported portable shape.
        let mut with_requirement = envelope.clone();
        with_requirement.descriptor = BoundedPortableObject::from_json(&serde_json::json!({
            "source": { "kind": "provider_marketplace", "namespace": "acme" },
            "resolution": "manual_only",
            "requirement": { "exact": "1.2.3" }
        }))
        .unwrap();
        assert!(PluginInstallIntentCodec
            .validate(&with_requirement)
            .is_err());
    }

    #[test]
    fn resolution_must_agree_with_the_portable_source() {
        assert!(intent(None, PluginResolutionPolicy::ProviderResolved)
            .validate()
            .unwrap_err()
            .contains("manual-only"));
        intent(None, PluginResolutionPolicy::ManualOnly)
            .validate()
            .unwrap();
        assert!(intent(
            Some(GitMarketplaceSource {
                location: "https://user:token@example.com/acme".to_string(),
                git_ref: None,
            }),
            PluginResolutionPolicy::ProviderResolved,
        )
        .validate()
        .is_err());
        let mut mismatched = intent(None, PluginResolutionPolicy::ManualOnly);
        mismatched.identity.plugin_id = "tools@other".to_string();
        assert!(mismatched
            .validate()
            .unwrap_err()
            .contains("does not match its namespace"));
    }

    #[test]
    fn plugin_namespaces_distinguish_identity() {
        let provider = ProviderKey::parse("claude").unwrap();
        let acme = intent(None, PluginResolutionPolicy::ManualOnly)
            .envelope(&provider)
            .unwrap();
        let mut other = intent(None, PluginResolutionPolicy::ManualOnly);
        other.identity.namespace = "other".to_string();
        other.identity.plugin_id = "tools@other".to_string();
        other.descriptor.source = PortablePluginSource::ProviderMarketplace {
            namespace: "other".to_string(),
            git: None,
        };
        assert_ne!(
            other.envelope(&provider).unwrap().resource_id,
            acme.resource_id
        );
    }

    #[test]
    fn git_locations_are_credential_free_and_public() {
        for valid in [
            "https://example.com/acme/market",
            "acme/market",
            "git://host/x",
        ] {
            validate_git_location("source", valid).unwrap();
        }
        for invalid in [
            "/tmp/local",
            "~/market",
            "ssh://host/x",
            "acme/market?token=1",
            "a b",
            "https://u:p@host/x",
        ] {
            assert!(
                validate_git_location("source", invalid).is_err(),
                "{invalid}"
            );
        }
    }
}
