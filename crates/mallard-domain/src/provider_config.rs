//! Versioned provider-configuration envelopes.
//!
//! Each provider configuration carries its own positive local version. The
//! owner of one coordinate can add version 2 without changing any other
//! coordinate or a top-level document schema.

use crate::bounded::BoundedConfigObject;
use crate::secrets::SecretMap;
use serde::{Deserialize, Serialize};

fn validate_version(label: &str, version: u32) -> Result<(), String> {
    if version == 0 {
        return Err(format!("{} config version must be positive", label));
    }
    Ok(())
}

/// One storage provider's saved configuration: safe settings the frontend may
/// read and edit, and machine-local secrets it never receives.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageProviderConfig {
    pub version: u32,
    pub settings: BoundedConfigObject,
    pub secrets: SecretMap,
}

impl StorageProviderConfig {
    pub fn new(version: u32, settings: BoundedConfigObject, secrets: SecretMap) -> Self {
        Self {
            version,
            settings,
            secrets,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_version("storage provider", self.version)
    }
}

/// One agent provider's profile-local configuration. Authentication stays in
/// the configured provider home; this envelope never holds credentials.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentProviderConfig {
    pub version: u32,
    pub settings: BoundedConfigObject,
}

impl AgentProviderConfig {
    /// The initial envelope every current provider uses: version 1 with no
    /// settings.
    pub fn version_1() -> Self {
        Self {
            version: 1,
            settings: BoundedConfigObject::empty(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_version("agent provider", self.version)
    }
}

/// One Library catalog profile's provider-local configuration. The Library
/// keeps its own profile record; this envelope is shared so it needs no
/// Project Sync DTO.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogProviderConfig {
    pub version: u32,
    pub settings: BoundedConfigObject,
}

impl CatalogProviderConfig {
    pub fn version_1() -> Self {
        Self {
            version: 1,
            settings: BoundedConfigObject::empty(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_version("catalog provider", self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_require_positive_versions_and_reject_unknown_fields() {
        assert!(AgentProviderConfig::version_1().validate().is_ok());
        assert!(AgentProviderConfig {
            version: 0,
            settings: BoundedConfigObject::empty()
        }
        .validate()
        .is_err());
        assert!(serde_json::from_str::<StorageProviderConfig>(
            "{\"version\":1,\"settings\":{},\"secrets\":{},\"extra\":1}"
        )
        .is_err());
        let config: StorageProviderConfig = serde_json::from_str(
            "{\"version\":1,\"settings\":{\"a\":1},\"secrets\":{\"t\":\"v\"}}",
        )
        .unwrap();
        assert!(config.validate().is_ok());
        assert!(!format!("{config:?}").contains("\"v\""));
    }
}
