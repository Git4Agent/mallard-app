//! Stable, validated key newtypes for every extension axis.
//!
//! A key is identity only: it is never a path, URL, command, executable
//! name, or authorization grant. Every key shares one grammar so a persisted
//! key is safe as a single logical-namespace component and compares exactly,
//! case-sensitively, with a registered coordinate.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The shared key grammar: 1–64 bytes of lowercase ASCII; the first byte is
/// a letter, the last a letter or digit, interior bytes are `[a-z0-9._-]`,
/// and `..` never appears.
pub fn validate_bounded_key(label: &str, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let valid = (1..=64).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(byte)
        })
        && !value.contains("..");
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{} must be 1-64 lowercase ASCII key characters: '{}'",
            label,
            value.escape_debug()
        ))
    }
}

macro_rules! bounded_key {
    ($(#[$meta:meta])* $name:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_bounded_key($label, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

bounded_key!(
    /// Validated agent-provider identity. Providers are registered keys, not
    /// a closed enum: structural validation accepts any well-formed key so
    /// unknown remote provider scopes stay readable and preservable, while
    /// commands that mutate local provider state must additionally resolve
    /// the key through the compiled adapter registry.
    ProviderKey,
    "provider key"
);

bounded_key!(
    /// Presentation-oriented provider resource subtype such as `session`,
    /// `memory`, `todo`, or `file-history`. Core behavior keys off the
    /// registered resource kind, ownership, and adapter capabilities — never
    /// off this string.
    ResourceTypeKey,
    "provider resource type"
);

bounded_key!(
    /// Validated storage-provider identity (`local`, `r2`, …). The key selects
    /// a compiled storage adapter; the record stays structurally readable
    /// when no adapter is registered.
    StorageProviderKey,
    "storage provider key"
);

bounded_key!(
    /// A Library resource kind (`agent_skill`, `desktop_pet`, …).
    CatalogResourceKindKey,
    "catalog resource kind"
);

bounded_key!(
    /// A Library portable format (`agentskills.io.skill`, …). Its
    /// `format_version` versions the portable resource shape.
    CatalogFormatKey,
    "catalog format"
);

bounded_key!(
    /// A machine-local Library apply target (`provider-home.skills`, …).
    CatalogTargetKey,
    "catalog target"
);

bounded_key!(
    /// A Project Sync portable resource kind (`conversation`, …).
    ResourceKindKey,
    "resource kind"
);

bounded_key!(
    /// A Project Sync apply policy (`safe_file`, `explicit_review`, …).
    ApplyPolicyKey,
    "apply policy"
);

bounded_key!(
    /// A Project Sync provenance kind (`project_local`, `unknown`, …).
    ProvenanceKindKey,
    "provenance kind"
);

bounded_key!(
    /// A Project Sync tombstone target kind (`file`, `conversation_thread`, …).
    TombstoneTargetKindKey,
    "tombstone target kind"
);

bounded_key!(
    /// The identifier of one secret inside a provider configuration.
    SecretKey,
    "secret key"
);

bounded_key!(
    /// The identifier of one safe configuration field a provider descriptor
    /// declares.
    FieldKey,
    "field key"
);

bounded_key!(
    /// A provider-declared action identifier (`codex.app.restart`, …).
    ActionKey,
    "action key"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_share_one_grammar() {
        for valid in [
            "a",
            "codex",
            "agentskills.io.skill",
            "provider-home.skills",
            "r2",
        ] {
            StorageProviderKey::parse(valid).unwrap();
            CatalogFormatKey::parse(valid).unwrap();
            TombstoneTargetKindKey::parse(valid).unwrap();
        }
        for invalid in [
            "",
            "A",
            "1a",
            "a.",
            "a..b",
            "a/b",
            "a b",
            "ä",
            &"a".repeat(65),
        ] {
            assert!(ResourceKindKey::parse(invalid).is_err(), "{invalid:?}");
        }
        assert!(ApplyPolicyKey::parse("a".repeat(64)).is_ok());
    }

    #[test]
    fn keys_serialize_as_validated_strings() {
        let key: ProvenanceKindKey = serde_json::from_str("\"project_local\"").unwrap();
        assert_eq!(key.as_str(), "project_local");
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"project_local\"");
        assert!(serde_json::from_str::<ProvenanceKindKey>("\"Project\"").is_err());
        assert!(serde_json::from_str::<ProvenanceKindKey>("1").is_err());
    }
}
