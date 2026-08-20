//! Local project-sync state and portable bundle model.
//!
//! This module intentionally contains no Tauri or filesystem code.  All
//! strings which can become cloud namespace components are validated on
//! deserialization, so persistence and transport code cannot accidentally
//! turn untrusted JSON into an unchecked path or identifier.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::path::{Component, Path};

pub const LOCAL_SCHEMA: u32 = 1;
pub const MACHINE_PROJECT_SCHEMA: u32 = 1;
pub const HOST_IDENTITY_SCHEMA: u32 = 1;
pub const CONVERSATION_BINDINGS_SCHEMA: u32 = 1;
pub const CONVERSATION_SYNC_JOURNAL_SCHEMA: u32 = 1;
pub const PUSH_PLAN_SCHEMA: u32 = 1;
pub const BUNDLE_SCHEMA: u32 = 1;
pub const RECIPE_SCHEMA: u32 = 1;
pub const RESTORE_PLAN_SCHEMA: u32 = 1;
pub const SETUP_DRAFT_SCHEMA: u32 = 1;
pub const SETUP_TRANSACTION_SCHEMA: u32 = 1;
pub const MAX_SETUP_DRAFTS: usize = 64;
/// Bound on provider scopes in one manifest (the project scope is separate).
pub const MAX_PROVIDER_SCOPES: usize = 64;

pub const MAX_PROJECTS: usize = 1_024;
pub const MAX_STORAGES: usize = 128;
pub const MAX_LINKS: usize = 4_096;
pub const MAX_BINDINGS: usize = 4_096;
pub const MAX_PROVIDER_PROFILES: usize = 1_024;
pub const MAX_RESOURCES: usize = 20_000;
pub const MAX_FILES: usize = 100_000;
pub const MAX_ACTIONS: usize = 100_000;
pub const MAX_CONVERSATION_BINDINGS: usize = 100_000;
pub const MAX_ORIGIN_ALIASES: usize = 20_000;
pub const MAX_CONVERSATION_JOURNAL_OPERATIONS: usize = 10_000;
pub const MAX_LOGICAL_PATH_BYTES: usize = 1_024;
pub const MAX_CONVERSATION_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_PROJECT_CONTENT_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_PROJECT_CONTENT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const ABSENT_BUNDLE_COMMIT_ID: &str = "absent";

const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn valid_named_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._:".contains(&byte)
        })
        && !value
            .chars()
            .last()
            .is_some_and(|character| matches!(character, '-' | '.' | '_' | ':'))
        && !value.contains("..")
}

fn validate_named_id(label: &str, value: &str) -> Result<(), String> {
    if valid_named_id(value) {
        Ok(())
    } else {
        Err(format!(
            "{} must be 1-128 lowercase ASCII identifier characters: '{}'",
            label,
            value.escape_debug()
        ))
    }
}

pub fn absent_bundle_head_pin() -> (u64, String, String) {
    (0, ABSENT_BUNDLE_COMMIT_ID.to_string(), "0".repeat(64))
}

fn validate_bundle_head_pin(
    revision: u64,
    commit_id: &str,
    manifest_sha256: &str,
) -> Result<(), String> {
    validate_named_id("bundle head pin commit", commit_id)?;
    validate_sha256("bundle head pin manifest", manifest_sha256)?;
    let is_absent = revision == 0;
    let uses_absent_values =
        commit_id == ABSENT_BUNDLE_COMMIT_ID && manifest_sha256.bytes().all(|byte| byte == b'0');
    if is_absent != uses_absent_values {
        return Err("bundle head pin has inconsistent absent coordinates".to_string());
    }
    Ok(())
}

const CLOUD_THREAD_ID_PREFIX: &str = "cloudthread-";

fn no_extra_named_id_rule(_label: &str, _value: &str) -> Result<(), String> {
    Ok(())
}

/// Local and cloud thread ids are structurally disjoint namespaces: a local
/// `ThreadId` may never use the `cloudthread-` prefix and a `CloudThreadId`
/// must, so no manifest, plan, or binding document can smuggle one space's
/// id into the other's fields.
fn reject_cloud_thread_namespace(label: &str, value: &str) -> Result<(), String> {
    if value.starts_with(CLOUD_THREAD_ID_PREFIX) {
        return Err(format!(
            "{} must not use the '{}' cloud namespace: '{}'",
            label,
            CLOUD_THREAD_ID_PREFIX,
            value.escape_debug()
        ));
    }
    Ok(())
}

fn require_cloud_thread_namespace(label: &str, value: &str) -> Result<(), String> {
    if !value.starts_with(CLOUD_THREAD_ID_PREFIX) {
        return Err(format!(
            "{} must start with '{}': '{}'",
            label,
            CLOUD_THREAD_ID_PREFIX,
            value.escape_debug()
        ));
    }
    Ok(())
}

macro_rules! named_id {
    ($name:ident, $label:literal) => {
        named_id!($name, $label, extra = no_extra_named_id_rule);
    };
    ($name:ident, $label:literal, extra = $extra:path) => {
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_named_id($label, &value)?;
                $extra($label, &value)?;
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

named_id!(ResourceId, "resource id");
named_id!(LocalProjectId, "local project id");
named_id!(StorageId, "storage id");
named_id!(ReplicaId, "replica id");
named_id!(ThreadId, "thread id", extra = reject_cloud_thread_namespace);
named_id!(
    CloudThreadId,
    "cloud thread id",
    extra = require_cloud_thread_namespace
);
named_id!(PlanId, "plan id");
named_id!(ActionId, "action id");
named_id!(MaterializationId, "materialization id");
named_id!(ConversationOperationId, "conversation operation id");
named_id!(LocalProviderProfileId, "local provider profile id");
named_id!(SetupDraftId, "setup draft id");

impl CloudThreadId {
    /// Mint a fresh cloud thread identity. Called only at push apply — plans
    /// carry intent, never minted identities, and recovery installs journaled
    /// documents verbatim without re-minting.
    pub fn generate() -> Result<Self, String> {
        Self::parse(generated_named_id("cloudthread")?)
    }
}

/// Presence-requiring nullable deserializer: attaching `deserialize_with`
/// disables serde's implicit missing-key tolerance for `Option` fields, so
/// the key must be present on the wire (`null` for `None`). Fields using it
/// must also serialize unconditionally (no `skip_serializing_if`), making
/// pre-cutover documents that lack the key fail closed instead of silently
/// loading as `None`.
fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

macro_rules! fixed_hex_id {
    ($name:ident, $label:literal, $bytes:literal) => {
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                if value.len() != $bytes * 2
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(format!(
                        "{} must be exactly {} lowercase hexadecimal characters: '{}'",
                        $label,
                        $bytes * 2,
                        value.escape_debug()
                    ));
                }
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

fixed_hex_id!(HostId, "host id", 16);
fixed_hex_id!(OriginHostToken, "origin host token", 32);
fixed_hex_id!(OriginReplicaToken, "origin replica token", 32);

/// Provider message identifiers are portable ancestry evidence, but unlike
/// Mallard-owned IDs their alphabet is provider-defined. Keep them bounded
/// and free of control characters without imposing `named_id!` syntax.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct MessageId(String);

impl MessageId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err("message id must be 1-256 non-control characters".to_string());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MessageId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<MessageId> for String {
    fn from(value: MessageId) -> Self {
        value.0
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl HostId {
    pub fn generate() -> Result<Self, String> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| format!("random host id: {}", error))?;
        Self::parse(hex_lower(&bytes))
    }
}

/// Derive one bundle-scoped provenance token from the private `host_id`.
/// The raw `host_id` never leaves the machine; bundles only ever carry these
/// HMAC outputs, so two bundles cannot be linked to one installation.
fn derive_origin_token(host_id: &HostId, fields: &[&[u8]]) -> Result<String, String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(host_id.as_str().as_bytes())
        .map_err(|_| "initialize origin-token HMAC".to_string())?;
    for field in fields {
        let length = u64::try_from(field.len())
            .map_err(|_| "origin-token field exceeds supported length".to_string())?;
        mac.update(&length.to_be_bytes());
        mac.update(field);
    }
    Ok(hex_lower(&mac.finalize().into_bytes()))
}

pub fn derive_origin_host_token(
    host_id: &HostId,
    bundle_id: &BundleId,
) -> Result<OriginHostToken, String> {
    OriginHostToken::parse(derive_origin_token(
        host_id,
        &[b"mallard-origin-host-v2", bundle_id.as_str().as_bytes()],
    )?)
}

pub fn derive_origin_replica_token(
    host_id: &HostId,
    bundle_id: &BundleId,
    replica_id: &ReplicaId,
) -> Result<OriginReplicaToken, String> {
    OriginReplicaToken::parse(derive_origin_token(
        host_id,
        &[
            b"mallard-origin-replica-v2",
            bundle_id.as_str().as_bytes(),
            replica_id.as_str().as_bytes(),
        ],
    )?)
}

/// A bundle ID is an opaque, generated 128-bit value.  Requiring its exact
/// lowercase hexadecimal representation makes it safe as one cloud key
/// component and prevents paths or user labels from becoming identity.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct BundleId(String);

impl BundleId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!(
                "bundle id must be exactly 32 lowercase hexadecimal characters: '{}'",
                value.escape_debug()
            ));
        }
        Ok(Self(value))
    }

    pub fn generate() -> Result<Self, String> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| format!("random bundle id: {}", error))?;
        Self::parse(hex_lower(&bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BundleId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<BundleId> for String {
    fn from(value: BundleId) -> String {
        value.0
    }
}

impl fmt::Display for BundleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A random RFC 4122 version-4 UUID in lowercase hyphenated form. Provider
/// CLIs treat session identifiers as UUIDs (Claude Code's `--session-id`
/// requires one), so forked provider sessions must mint IDs in the
/// provider-native format rather than Mallard's internal named-ID format.
pub fn generated_uuid_v4() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| format!("random uuid: {}", error))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex_lower(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

pub fn generated_named_id(prefix: &str) -> Result<String, String> {
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(format!("invalid generated id prefix '{}'", prefix));
    }
    let mut bytes = [0_u8; 12];
    getrandom::fill(&mut bytes).map_err(|error| format!("random id: {}", error))?;
    Ok(format!("{}-{}", prefix, hex_lower(&bytes)))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

/// A portable path inside one bundle.  It is never joined directly to a
/// machine path; provider adapters own that mapping.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct LogicalPath(String);

impl LogicalPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_logical_path(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LogicalPath {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<LogicalPath> for String {
    fn from(value: LogicalPath) -> String {
        value.0
    }
}

impl fmt::Display for LogicalPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate_logical_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > MAX_LOGICAL_PATH_BYTES
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return Err(format!("unsafe logical path '{}'", path.escape_debug()));
    }
    let mut components = path.split('/');
    let root = components.next().unwrap_or_default();
    if !matches!(root, "project" | "state" | "dependencies" | "requirements") {
        return Err(format!("unknown logical namespace '{}'", root));
    }
    let mut saw_child = false;
    for component in components {
        saw_child = true;
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > 255
            || component
                .chars()
                .last()
                .is_some_and(|character| matches!(character, ' ' | '.'))
        {
            return Err(format!("unsafe logical path '{}'", path.escape_debug()));
        }
        let stem = component.split('.').next().unwrap_or_default();
        if WINDOWS_RESERVED_NAMES
            .iter()
            .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        {
            return Err(format!("reserved device name in logical path '{}'", path));
        }
    }
    if !saw_child {
        return Err(format!("logical namespace '{}' is not a file path", root));
    }
    Ok(())
}

pub fn validate_sha256(label: &str, digest: &str) -> Result<(), String> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("{} is not a lowercase SHA-256 digest", label))
    }
}

fn validate_display_text(label: &str, value: &str, allow_empty: bool) -> Result<(), String> {
    if (!allow_empty && value.trim().is_empty())
        || value.len() > 1_024
        || value.chars().any(char::is_control)
    {
        Err(format!("invalid {}", label))
    } else {
        Ok(())
    }
}

fn validate_provider_thread_id(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(format!("invalid {}", label))
    } else {
        Ok(())
    }
}

pub fn validate_absolute_clean_path(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4_096 || value.chars().any(char::is_control) {
        return Err(format!("invalid {}", label));
    }
    let path = Path::new(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(format!(
            "{} must be an absolute clean path: '{}'",
            label, value
        ));
    }
    Ok(())
}

/// Validated agent-provider identity. Providers are registered keys, not a
/// closed enum: structural validation accepts any well-formed key so unknown
/// remote provider scopes stay readable and preservable, while commands that
/// mutate local provider state must additionally resolve the key through the
/// compile-time adapter registry.
///
/// Keys use a bounded lowercase ASCII namespace: the first character is a
/// letter, later characters are `[a-z0-9._-]`, the final character is a
/// letter or digit, and `..` never appears. This keeps every key safe as one
/// logical-namespace component before the normal logical-path validation
/// runs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct ProviderKey(String);

fn validate_bounded_key(label: &str, value: &str) -> Result<(), String> {
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

impl ProviderKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_bounded_key("provider key", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProviderKey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ProviderKey> for String {
    fn from(value: ProviderKey) -> String {
        value.0
    }
}

impl fmt::Display for ProviderKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Presentation-oriented provider resource subtype such as `session`,
/// `memory`, `todo`, or `file-history`. Core behavior keys off
/// [`ResourceKind`], ownership, and adapter capabilities — never off this
/// string.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceTypeKey(String);

impl ResourceTypeKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_bounded_key("provider resource type", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ResourceTypeKey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ResourceTypeKey> for String {
    fn from(value: ResourceTypeKey) -> String {
        value.0
    }
}

impl fmt::Display for ResourceTypeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One independently revisioned portion of a portable repository: the shared
/// project slice or one provider's slice.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SyncScopeKey {
    Project,
    Provider(ProviderKey),
}

impl SyncScopeKey {
    pub fn provider(key: ProviderKey) -> Self {
        Self::Provider(key)
    }

    pub fn provider_key(&self) -> Option<&ProviderKey> {
        match self {
            Self::Project => None,
            Self::Provider(key) => Some(key),
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        if value == "project" {
            return Ok(Self::Project);
        }
        if let Some(key) = value.strip_prefix("provider:") {
            return Ok(Self::Provider(ProviderKey::parse(key)?));
        }
        Err(format!("unknown sync scope '{}'", value.escape_debug()))
    }

    pub fn canonical(&self) -> String {
        match self {
            Self::Project => "project".to_string(),
            Self::Provider(key) => format!("provider:{}", key),
        }
    }
}

impl fmt::Display for SyncScopeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

impl Serialize for SyncScopeKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> Deserialize<'de> for SyncScopeKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundleKind {
    Project,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    S3,
    Local,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub id: StorageId,
    pub name: String,
    pub kind: StorageKind,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub access_key_id: String,
    #[serde(default)]
    pub secret_access_key: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub s3_endpoint: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub local_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_conditional_writes: Option<bool>,
}

impl StorageConfig {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("storage name", &self.name, true)?;
        match self.kind {
            StorageKind::Local => {
                if self.local_dir.is_empty() {
                    return Err(format!("storage '{}' needs a local directory", self.id));
                }
                validate_absolute_clean_path("local storage directory", &self.local_dir)?;
            }
            StorageKind::S3 => {
                if self.bucket.trim().is_empty() {
                    return Err(format!("storage '{}' needs a bucket", self.id));
                }
            }
        }
        Ok(())
    }
}

/// Generic resource classes. Provider identity lives in
/// [`ResourceDescriptor::provider`], never in the kind, so registering a new
/// provider adds no variant here.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Conversation,
    ProviderState,
    ProjectContentFile,
    ProjectContentDirectory,
    Requirement,
}

impl ResourceKind {
    /// The sync scope this kind of resource belongs to: provider-owned kinds
    /// live in their provider's scope, everything else in the project scope.
    pub fn requires_provider(self) -> bool {
        matches!(self, Self::Conversation | Self::ProviderState)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPolicy {
    SafeFile,
    Merge,
    ExplicitReview,
    ManualOnly,
    Never,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    ProjectLocal { relative_path: String },
    Unknown,
}

impl Provenance {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::ProjectLocal { relative_path } => {
                if relative_path.is_empty()
                    || Path::new(relative_path).is_absolute()
                    || Path::new(relative_path).components().any(|component| {
                        matches!(component, Component::CurDir | Component::ParentDir)
                    })
                {
                    return Err("invalid project-local provenance path".to_string());
                }
            }
            Self::Unknown => {}
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResourceDescriptor {
    pub resource_id: ResourceId,
    pub kind: ResourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKey>,
    /// Presentation-only provider subtype (`session`, `memory`, `todo`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_resource_type: Option<ResourceTypeKey>,
    pub display_name: String,
    pub provenance: Provenance,
    pub apply_policy: ApplyPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_cwd: Option<String>,
    pub codec_version: u32,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl ResourceDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("resource display name", &self.display_name, false)?;
        self.provenance.validate()?;
        if self.kind.requires_provider() != self.provider.is_some() {
            return Err(format!(
                "resource '{}' provider does not match its kind",
                self.resource_id
            ));
        }
        if self.provider.is_none() && self.provider_resource_type.is_some() {
            return Err(format!(
                "resource '{}' has a provider subtype without a provider",
                self.resource_id
            ));
        }
        if self.codec_version == 0 {
            return Err(format!(
                "resource '{}' has codec version 0",
                self.resource_id
            ));
        }
        if let Some(relative) = &self.relative_cwd {
            // Provider adapters use `.` as the sole canonical spelling for
            // the bound project root. Embedded `.` and every `..` remain
            // forbidden.
            let invalid_component = relative != "."
                && Path::new(relative)
                    .components()
                    .any(|component| matches!(component, Component::CurDir | Component::ParentDir));
            if relative.is_empty() || Path::new(relative).is_absolute() || invalid_component {
                return Err(format!(
                    "resource '{}' has invalid relative cwd",
                    self.resource_id
                ));
            }
        }
        if self.metadata.len() > 256
            || self.metadata.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 256
                    || value.len() > 4_096
                    || key.chars().any(char::is_control)
                    || value.chars().any(char::is_control)
            })
        {
            return Err(format!(
                "resource '{}' has invalid metadata",
                self.resource_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RecipeEntry {
    pub resource_id: ResourceId,
    pub apply_policy: ApplyPolicy,
    #[serde(default)]
    pub required: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleRecipe {
    pub schema_version: u32,
    pub revision: u64,
    #[serde(default)]
    pub entries: BTreeMap<ResourceId, RecipeEntry>,
}

impl Default for BundleRecipe {
    fn default() -> Self {
        Self {
            schema_version: RECIPE_SCHEMA,
            revision: 0,
            entries: BTreeMap::new(),
        }
    }
}

impl BundleRecipe {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != RECIPE_SCHEMA {
            return Err(format!(
                "unsupported recipe schema {} (expected {})",
                self.schema_version, RECIPE_SCHEMA
            ));
        }
        if self.entries.len() > MAX_RESOURCES {
            return Err(format!("recipe exceeds {} resources", MAX_RESOURCES));
        }
        for (id, entry) in &self.entries {
            if id != &entry.resource_id {
                return Err(format!(
                    "recipe key '{}' does not match entry '{}'",
                    id, entry.resource_id
                ));
            }
            if entry.apply_policy == ApplyPolicy::Never {
                return Err(format!("recipe cannot select Never resource '{}'", id));
            }
        }
        Ok(())
    }

    /// Validate a recipe used as machine-local selection state. Portable
    /// conversation identities are valid inside bundle scope recipes, but a
    /// local project or storage-link recipe selects provider capture resources
    /// instead. Conversation publication is authorized separately through a
    /// reviewed conversation plan.
    pub fn validate_machine_local(&self) -> Result<(), String> {
        self.validate()?;
        if let Some(resource_id) = self
            .entries
            .keys()
            .find(|resource_id| is_conversation_resource_id(resource_id.as_str()))
        {
            return Err(format!(
                "machine-local recipe contains portable conversation resource '{}'; conversations must be reviewed through a conversation plan",
                resource_id
            ));
        }
        Ok(())
    }

    pub fn selected_ids(&self) -> BTreeSet<ResourceId> {
        self.entries.keys().cloned().collect()
    }
}

/// The repository-level manifest coordinates a reviewed scope base was
/// established against. It lets status views reload the exact historical
/// manifest after the storage head advances, and names the manifest the
/// continuity probe checks for lineage membership.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RepositoryBase {
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
}

impl RepositoryBase {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("repository base manifest", &self.manifest_sha256)?;
        if self.manifest_size == 0 {
            return Err("repository base manifest size must be nonzero".to_string());
        }
        validate_named_id("repository base commit", &self.commit_id)?;
        Ok(())
    }
}

/// The machine-local last reviewed revision and content digest for one sync
/// scope. Continuity, Push authority, and Pull fast-forwarding are decided
/// per scope so unrelated providers never block each other.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ScopeBase {
    pub scope: SyncScopeKey,
    pub scope_revision: u64,
    pub scope_sha256: String,
    pub reviewed_in: RepositoryBase,
    /// The machine binding revision that established this reviewed base.
    pub binding_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pull_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_push_at: Option<u64>,
}

impl ScopeBase {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("scope base content", &self.scope_sha256)?;
        if self.scope_revision == 0 {
            return Err("scope base revision must be nonzero".to_string());
        }
        self.reviewed_in.validate()
    }
}

/// Scope-level continuity between one reviewed scope base and the verified
/// current manifest. No probe is needed: global repository continuity has
/// already established lineage, and scope revisions are internal to it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ScopeContinuity {
    /// No remote scope and no reviewed base: first publication target.
    Fresh,
    /// A base is recorded but the current manifest has no such scope.
    RemoteMissing { base_revision: u64 },
    /// The remote scope exists but this replica never reviewed it.
    Unreviewed { remote_revision: u64 },
    /// The remote scope is exactly the reviewed base.
    InSync { revision: u64 },
    /// The remote scope advanced past the reviewed base.
    Advanced {
        base_revision: u64,
        remote_revision: u64,
    },
    /// The remote scope no longer extends the reviewed base.
    Replaced {
        base_revision: u64,
        remote_revision: u64,
    },
}

/// Classify one scope's continuity from the verified current manifest's
/// scope coordinates and the recorded scope base.
pub fn classify_scope_continuity(
    remote: Option<(u64, &str)>,
    base: Option<&ScopeBase>,
) -> ScopeContinuity {
    match (remote, base) {
        (None, None) => ScopeContinuity::Fresh,
        (None, Some(base)) => ScopeContinuity::RemoteMissing {
            base_revision: base.scope_revision,
        },
        (Some((remote_revision, _)), None) => ScopeContinuity::Unreviewed { remote_revision },
        (Some((remote_revision, remote_sha256)), Some(base)) => {
            if remote_revision == base.scope_revision {
                if remote_sha256 == base.scope_sha256 {
                    ScopeContinuity::InSync {
                        revision: remote_revision,
                    }
                } else {
                    ScopeContinuity::Replaced {
                        base_revision: base.scope_revision,
                        remote_revision,
                    }
                }
            } else if remote_revision < base.scope_revision {
                ScopeContinuity::Replaced {
                    base_revision: base.scope_revision,
                    remote_revision,
                }
            } else {
                ScopeContinuity::Advanced {
                    base_revision: base.scope_revision,
                    remote_revision,
                }
            }
        }
    }
}

/// Repository-level continuity between this replica's recorded sync base and
/// the observed bundle head — rows 1–8 of DESIGN_REMOTE_HISTORY_CONTINUITY.md.
/// Computed by every plan phase and surfaced by Refresh, Pull review, and
/// Push review. Advisory: it gates and explains, never mutates.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RemoteContinuity {
    /// No remote history and no recorded base: a first publication target.
    Fresh,
    /// A base is recorded but the remote holds no history at all.
    RemoteMissing { base_revision: u64 },
    /// Remote history exists but this replica has never reviewed a base.
    Unreviewed { remote_revision: u64 },
    /// The head names exactly the manifest this replica last synced.
    InSync { revision: u64 },
    /// The recorded base is still part of the remote history; a pull
    /// fast-forwards onto it.
    Advanced {
        base_revision: u64,
        remote_revision: u64,
    },
    /// The remote no longer contains the recorded base: the history this
    /// replica synced was replaced, not extended.
    Replaced {
        base_revision: u64,
        remote_revision: u64,
    },
    /// The probe could not reach the store; no verdict is claimed.
    Unknown { message: String },
}

/// Classify remote history continuity from content witnesses alone.
///
/// The probe runs only in the revision-ahead arm and must read the store —
/// never a local cache, whose contents witness nothing about the remote —
/// reporting whether the base manifest is still present with exactly the
/// recorded digest. Every other row is decided for free: equal revisions
/// with different digests cannot share a lineage (a lineage commits each
/// revision exactly once), and revisions never regress within one.
pub fn classify_remote_continuity(
    observed_head: Option<&BundleHead>,
    base: Option<&RepositoryBase>,
    probe: &mut dyn FnMut(&RepositoryBase) -> Result<bool, String>,
) -> RemoteContinuity {
    match (observed_head, base) {
        (None, None) => RemoteContinuity::Fresh,
        (None, Some(base)) => RemoteContinuity::RemoteMissing {
            base_revision: base.revision,
        },
        (Some(head), None) => RemoteContinuity::Unreviewed {
            remote_revision: head.revision,
        },
        (Some(head), Some(base)) => {
            if head.revision == base.revision {
                if head.manifest_sha256 == base.manifest_sha256 {
                    RemoteContinuity::InSync {
                        revision: head.revision,
                    }
                } else {
                    RemoteContinuity::Replaced {
                        base_revision: base.revision,
                        remote_revision: head.revision,
                    }
                }
            } else if head.revision < base.revision {
                RemoteContinuity::Replaced {
                    base_revision: base.revision,
                    remote_revision: head.revision,
                }
            } else {
                match probe(base) {
                    Ok(true) => RemoteContinuity::Advanced {
                        base_revision: base.revision,
                        remote_revision: head.revision,
                    },
                    Ok(false) => RemoteContinuity::Replaced {
                        base_revision: base.revision,
                        remote_revision: head.revision,
                    },
                    Err(message) => RemoteContinuity::Unknown { message },
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalProjectRegistration {
    pub local_project_id: LocalProjectId,
    pub bundle_id: BundleId,
    pub display_name: String,
    /// Machine-local nickname shown instead of `display_name`. Never pushed:
    /// the remote bundle keeps `display_name`, so two checkouts of the same
    /// repo can be told apart without renaming it for every replica.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
    pub recipe: BundleRecipe,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl LocalProjectRegistration {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("project display name", &self.display_name, false)?;
        if let Some(alias) = &self.local_alias {
            validate_display_text("project local alias", alias, false)?;
        }
        if let Some(fingerprint) = &self.repository_fingerprint {
            validate_sha256("repository fingerprint", fingerprint)?;
        }
        self.recipe.validate_machine_local()?;
        if self.created_at > self.updated_at {
            return Err(format!(
                "project '{}' has invalid timestamps",
                self.local_project_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectStorageLink {
    pub local_project_id: LocalProjectId,
    pub storage_id: StorageId,
    /// Must equal the registration's bundle ID.  Keeping it explicit makes
    /// remote requests self-contained while validation prevents split identity.
    pub bundle_id: BundleId,
    /// Destination-specific selection from the last explicit Push. Links that
    /// have never pushed fall back to the project's default recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<BundleRecipe>,
    /// Machine-local reviewed bases, one per sync scope. A scope that was
    /// merely viewed has no entry; only an approved Push, Pull, or explicit
    /// provider-scoped acknowledgment may advance one.
    #[serde(default)]
    pub scope_bases: BTreeMap<SyncScopeKey, ScopeBase>,
    /// Machine-local, destination-specific decisions for generic project
    /// content. These preferences are never serialized into a remote bundle.
    #[serde(default)]
    pub project_content_preferences: ProjectContentPreferences,
    /// Destination-scoped conversation state, keyed by provider scope and
    /// thread. Two links may have accepted different versions of one alias;
    /// nothing here is ever serialized into a remote bundle. Deliberately not
    /// defaulted: a pre-cutover link document fails closed instead of being
    /// silently upgraded.
    pub conversation_checkpoints:
        BTreeMap<SyncScopeKey, BTreeMap<ThreadId, ConversationCheckpoint>>,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: u64,
}

pub const PROJECT_CONTENT_PREFERENCES_SCHEMA: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectContentPreferences {
    pub schema_version: u32,
    pub revision: u64,
    #[serde(default)]
    pub excluded_resource_ids: BTreeSet<ResourceId>,
}

impl Default for ProjectContentPreferences {
    fn default() -> Self {
        Self {
            schema_version: PROJECT_CONTENT_PREFERENCES_SCHEMA,
            revision: 0,
            excluded_resource_ids: BTreeSet::new(),
        }
    }
}

impl ProjectContentPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PROJECT_CONTENT_PREFERENCES_SCHEMA {
            return Err(format!(
                "unsupported project-content preferences schema {}",
                self.schema_version
            ));
        }
        if self.excluded_resource_ids.len() > MAX_RESOURCES {
            return Err(format!(
                "project-content preferences exceed {} exclusions",
                MAX_RESOURCES
            ));
        }
        if let Some(resource_id) = self
            .excluded_resource_ids
            .iter()
            .find(|resource_id| !is_project_content_resource_id(resource_id))
        {
            return Err(format!(
                "project-content preferences contain non-content resource '{}'",
                resource_id
            ));
        }
        Ok(())
    }
}

pub fn is_project_content_resource_id(resource_id: &ResourceId) -> bool {
    resource_id.as_str().starts_with("project:content-file:")
        || resource_id.as_str().starts_with("project:content-dir:")
}

fn project_content_identity(
    entry_type: &str,
    logical_path: &LogicalPath,
) -> Result<(String, ResourceId), String> {
    let relative_path = logical_path
        .as_str()
        .strip_prefix("project/")
        .filter(|relative| !relative.is_empty())
        .ok_or_else(|| {
            format!(
                "project-content path '{}' is outside the project namespace",
                logical_path
            )
        })?;
    let digest = Sha256::digest(format!("{}\0{}", entry_type, relative_path).as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{:02x}", byte);
    }
    let prefix = match entry_type {
        "file" => "project:content-file:",
        "dir" => "project:content-dir:",
        _ => {
            return Err(format!(
                "unsupported project-content entry type '{}'",
                entry_type
            ))
        }
    };
    Ok((
        relative_path.to_string(),
        ResourceId::parse(format!("{}{}", prefix, hex))?,
    ))
}

fn validate_project_content_descriptor_identity(
    descriptor: &ResourceDescriptor,
    logical_path: &LogicalPath,
    entry_type: &str,
) -> Result<(), String> {
    let (relative_path, expected_id) = project_content_identity(entry_type, logical_path)?;
    if descriptor.resource_id != expected_id {
        return Err(format!(
            "project-content resource '{}' does not match {} path '{}'",
            descriptor.resource_id, entry_type, logical_path
        ));
    }
    match &descriptor.provenance {
        Provenance::ProjectLocal {
            relative_path: provenance,
        } if provenance == &relative_path => {}
        _ => {
            return Err(format!(
                "project-content resource '{}' has mismatched path provenance",
                descriptor.resource_id
            ));
        }
    }
    if descriptor.apply_policy != ApplyPolicy::ExplicitReview
        || descriptor.codec_version != 1
        || descriptor.provider.is_some()
        || descriptor.relative_cwd.is_some()
    {
        return Err(format!(
            "project-content resource '{}' has invalid descriptor policy",
            descriptor.resource_id
        ));
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyncConfig {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub storages: Vec<StorageConfig>,
    #[serde(default)]
    pub projects: Vec<LocalProjectRegistration>,
    #[serde(default)]
    pub links: Vec<ProjectStorageLink>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            schema: LOCAL_SCHEMA,
            revision: 0,
            storages: Vec::new(),
            projects: Vec::new(),
            links: Vec::new(),
        }
    }
}

impl SyncConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != LOCAL_SCHEMA {
            return Err(format!(
                "unsupported project-sync schema {} (expected {})",
                self.schema, LOCAL_SCHEMA
            ));
        }
        if self.storages.len() > MAX_STORAGES
            || self.projects.len() > MAX_PROJECTS
            || self.links.len() > MAX_LINKS
        {
            return Err("project-sync config exceeds collection limits".to_string());
        }
        let mut storage_ids = HashSet::new();
        for storage in &self.storages {
            storage.validate()?;
            if !storage_ids.insert(storage.id.clone()) {
                return Err(format!("duplicate storage id '{}'", storage.id));
            }
        }
        let mut local_ids = HashSet::new();
        let mut projects = BTreeMap::new();
        for project in &self.projects {
            project.validate()?;
            if !local_ids.insert(project.local_project_id.clone()) {
                return Err(format!(
                    "duplicate local project id '{}'",
                    project.local_project_id
                ));
            }
            projects.insert(project.local_project_id.clone(), project);
        }
        let mut cells = HashSet::new();
        for link in &self.links {
            let project = projects.get(&link.local_project_id).ok_or_else(|| {
                format!(
                    "link references unknown project '{}'",
                    link.local_project_id
                )
            })?;
            if !storage_ids.contains(&link.storage_id) {
                return Err(format!(
                    "link references unknown storage '{}'",
                    link.storage_id
                ));
            }
            if link.bundle_id != project.bundle_id {
                return Err(format!(
                    "link bundle '{}' differs from project bundle '{}'",
                    link.bundle_id, project.bundle_id
                ));
            }
            if let Some(recipe) = &link.recipe {
                recipe.validate_machine_local()?;
            }
            if link.scope_bases.len() > MAX_PROVIDER_SCOPES + 1 {
                return Err(format!(
                    "link '{}'/'{}' has too many scope bases",
                    link.local_project_id, link.storage_id
                ));
            }
            for (scope, base) in &link.scope_bases {
                if scope != &base.scope {
                    return Err(format!(
                        "link scope base key '{}' does not match its scope '{}'",
                        scope, base.scope
                    ));
                }
                base.validate()?;
            }
            link.project_content_preferences.validate()?;
            let mut checkpoint_count = 0_usize;
            for (scope, checkpoints) in &link.conversation_checkpoints {
                if scope.provider_key().is_none() {
                    return Err(format!(
                        "link '{}'/'{}' keeps conversation checkpoints outside a provider scope",
                        link.local_project_id, link.storage_id
                    ));
                }
                checkpoint_count += checkpoints.len();
                for checkpoint in checkpoints.values() {
                    checkpoint.validate()?;
                }
            }
            if checkpoint_count > MAX_CONVERSATION_BINDINGS {
                return Err(format!(
                    "link '{}'/'{}' has too many conversation checkpoints",
                    link.local_project_id, link.storage_id
                ));
            }
            if !cells.insert((link.local_project_id.clone(), link.storage_id.clone())) {
                return Err(format!(
                    "duplicate project/storage link '{}'/ '{}'",
                    link.local_project_id, link.storage_id
                ));
            }
        }
        Ok(())
    }

    pub fn project(&self, id: &LocalProjectId) -> Option<&LocalProjectRegistration> {
        self.projects
            .iter()
            .find(|project| &project.local_project_id == id)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    Active,
    Detached,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HostIdentity {
    pub schema: u32,
    pub revision: u64,
    pub host_id: HostId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_display_name: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl HostIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != HOST_IDENTITY_SCHEMA {
            return Err(format!(
                "unsupported host identity schema {} (expected {})",
                self.schema, HOST_IDENTITY_SCHEMA
            ));
        }
        if let Some(name) = &self.local_display_name {
            validate_host_label("local host display name", name)?;
        }
        if self.created_at > self.updated_at {
            return Err("host identity has invalid timestamps".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PortableOrigin {
    pub origin_host_token: OriginHostToken,
    pub origin_replica_token: OriginReplicaToken,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_label: Option<String>,
}

impl PortableOrigin {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(label) = &self.shared_label {
            validate_host_label("shared origin label", label)?;
        }
        Ok(())
    }
}

fn validate_host_label(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(format!("invalid {}", label))
    } else {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderProfile {
    pub profile_id: LocalProviderProfileId,
    pub provider: ProviderKey,
    pub display_name: String,
    /// The exact provider-home spelling selected locally (for example a
    /// `CODEX_HOME` or `CLAUDE_CONFIG_DIR` directory).
    pub path: String,
    /// Canonical path captured when the profile was created.
    pub canonical_path: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl ProviderProfile {
    pub fn validate_structure(&self) -> Result<(), String> {
        validate_display_text("provider profile name", &self.display_name, false)?;
        validate_absolute_clean_path("provider profile path", &self.path)?;
        validate_absolute_clean_path("canonical provider profile path", &self.canonical_path)?;
        if self.created_at > self.updated_at {
            return Err(format!(
                "provider profile '{}' has invalid timestamps",
                self.profile_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectBinding {
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub bundle_id: BundleId,
    /// User-selected spelling, used for provider cwd behavior.
    pub project_root: String,
    /// Canonical path captured when the binding was validated.
    pub canonical_project_root: String,
    /// The bound providers. Machine state persists profile IDs only; runtime
    /// provider homes are resolved through the profile catalog into an
    /// operation context, never stored on the binding.
    #[serde(default)]
    pub profile_ids: BTreeMap<ProviderKey, LocalProviderProfileId>,
    pub state: BindingState,
    pub revision: u64,
    pub updated_at: u64,
}

impl ProjectBinding {
    pub fn validate_structure(&self) -> Result<(), String> {
        validate_absolute_clean_path("project root", &self.project_root)?;
        validate_absolute_clean_path("canonical project root", &self.canonical_project_root)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MachineProjectState {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub profiles: Vec<ProviderProfile>,
    #[serde(default)]
    pub bindings: Vec<ProjectBinding>,
}

impl Default for MachineProjectState {
    fn default() -> Self {
        Self {
            schema: MACHINE_PROJECT_SCHEMA,
            revision: 0,
            profiles: Vec::new(),
            bindings: Vec::new(),
        }
    }
}

impl MachineProjectState {
    pub fn validate(&self, config: &SyncConfig) -> Result<(), String> {
        if self.schema != MACHINE_PROJECT_SCHEMA
            || self.bindings.len() > MAX_BINDINGS
            || self.profiles.len() > MAX_PROVIDER_PROFILES
        {
            return Err("invalid machine project state document".to_string());
        }
        let mut profile_ids = HashSet::new();
        let mut profile_paths: Vec<&ProviderProfile> = Vec::new();
        let mut profiles = BTreeMap::new();
        for profile in &self.profiles {
            profile.validate_structure()?;
            if !profile_ids.insert(profile.profile_id.clone()) {
                return Err(format!(
                    "duplicate provider profile id '{}'",
                    profile.profile_id
                ));
            }
            let path = Path::new(&profile.canonical_path);
            if let Some(other) = profile_paths.iter().find(|other| {
                let other_path = Path::new(&other.canonical_path);
                path.starts_with(other_path) || other_path.starts_with(path)
            }) {
                return Err(format!(
                    "provider profiles '{}' and '{}' overlap",
                    other.profile_id, profile.profile_id
                ));
            }
            profile_paths.push(profile);
            profiles.insert(profile.profile_id.clone(), profile);
        }
        let mut replicas = HashSet::new();
        let mut host_id: Option<&HostId> = None;
        let mut active_projects = HashSet::new();
        let mut active_root_profiles: BTreeMap<(String, LocalProviderProfileId), &LocalProjectId> =
            BTreeMap::new();
        for binding in &self.bindings {
            binding.validate_structure()?;
            if let Some(expected) = host_id {
                if expected != &binding.host_id {
                    return Err("machine project bindings contain multiple host IDs".to_string());
                }
            } else {
                host_id = Some(&binding.host_id);
            }
            if !replicas.insert(binding.replica_id.clone()) {
                return Err("machine project bindings contain a duplicate replica ID".to_string());
            }
            let project = config.project(&binding.local_project_id);
            if binding.state == BindingState::Active && project.is_none() {
                return Err(format!(
                    "active binding references unknown project '{}'",
                    binding.local_project_id
                ));
            }
            if let Some(project) = project {
                if binding.bundle_id != project.bundle_id {
                    return Err(format!(
                        "binding bundle '{}' differs from project bundle '{}'",
                        binding.bundle_id, project.bundle_id
                    ));
                }
            }
            if binding.state == BindingState::Active {
                if binding.profile_ids.is_empty() {
                    return Err(format!(
                        "project '{}' has no provider profile",
                        binding.local_project_id
                    ));
                }
                for (provider, profile_id) in &binding.profile_ids {
                    let profile = profiles.get(profile_id).ok_or_else(|| {
                        format!(
                            "project '{}' references unknown provider profile '{}'",
                            binding.local_project_id, profile_id
                        )
                    })?;
                    if &profile.provider != provider {
                        return Err(format!(
                            "project '{}' assigns {:?} to a {:?} profile",
                            binding.local_project_id, provider, profile.provider
                        ));
                    }
                }
                if !active_projects.insert(binding.local_project_id.clone()) {
                    return Err(format!(
                        "project '{}' has multiple active bindings",
                        binding.local_project_id
                    ));
                }
                // Project identity is (checkout, provider config): one folder
                // may host several active projects as long as no two of them
                // use the same provider profile.
                let folded = binding.canonical_project_root.to_lowercase();
                for profile_id in binding.profile_ids.values() {
                    if let Some(other) = active_root_profiles.insert(
                        (folded.clone(), profile_id.clone()),
                        &binding.local_project_id,
                    ) {
                        return Err(format!(
                            "projects '{}' and '{}' use the same provider config for one checkout",
                            other, binding.local_project_id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn active_for(&self, id: &LocalProjectId) -> Option<&ProjectBinding> {
        self.bindings.iter().find(|binding| {
            &binding.local_project_id == id && binding.state == BindingState::Active
        })
    }

    pub fn for_project(&self, id: &LocalProjectId) -> Option<&ProjectBinding> {
        self.active_for(id).or_else(|| {
            self.bindings
                .iter()
                .find(|binding| &binding.local_project_id == id)
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMaterializationState {
    Pending,
    Resumable,
    StoredOnly,
}

/// Read-side character budget for provider index titles: composed preserved
/// marker titles must fit it whole so the marker suffix survives truncating
/// readers.
pub const PRESERVED_TITLE_MAX_CHARS: usize = 180;

const PRESERVED_MARKER_PREFIX: &str = "Preserved ";
const PRESERVED_MARKER_SEPARATOR: &str = " · ";

/// Portable provenance of a divergent keep-both preservation: the origin
/// thread the preserved copy was split from — always named in the cloud id
/// space, on cloud rows and binding rows alike — and the 1-based per-origin
/// ordinal minted at apply. Display-only — never consulted by the resolver,
/// and the origin thread may have been tombstoned or pruned since. Ordinals
/// are pinned per device; concurrent keep-boths of the same origin on
/// different devices may mint the same ordinal (accepted — the device label
/// in the marker title disambiguates).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreservedProvenance {
    pub preserved_from: CloudThreadId,
    pub ordinal: u64,
}

impl PreservedProvenance {
    pub fn validate(&self) -> Result<(), String> {
        if self.ordinal == 0 {
            return Err("preservation provenance has a zero ordinal".to_string());
        }
        Ok(())
    }

    /// Cloud rows share the provenance id space with their own key, so a
    /// self-preserve is expressible there and rejected here. Binding rows
    /// carry a local-space key, where the mismatch is a type error instead.
    pub fn validate_for_cloud_row(&self, carrier: &CloudThreadId) -> Result<(), String> {
        self.validate()?;
        if &self.preserved_from == carrier {
            return Err(format!(
                "conversation thread '{}' cannot be preserved from itself",
                carrier
            ));
        }
        Ok(())
    }
}

/// True when `text` is exactly `Preserved <digits> · <label>` with a
/// non-empty label.
fn is_preserved_marker(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(PRESERVED_MARKER_PREFIX) else {
        return false;
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    rest[digits..]
        .strip_prefix(PRESERVED_MARKER_SEPARATOR)
        .is_some_and(|label| !label.is_empty())
}

/// Removes a trailing keep-both marker — `"{base} (Preserved {N} · {label})"`
/// or the bare `"Preserved {N} · {label}"` — returning the base title (empty
/// when the title was only a marker). Titles without a marker come back
/// unchanged.
pub fn strip_preserved_marker(title: &str) -> String {
    let trimmed = title.trim_end();
    if is_preserved_marker(trimmed) {
        return String::new();
    }
    if let Some(without_paren) = trimmed.strip_suffix(')') {
        if let Some(open) = without_paren.rfind('(') {
            if is_preserved_marker(&without_paren[open + 1..]) {
                return without_paren[..open].trim_end().to_string();
            }
        }
    }
    title.to_string()
}

/// Composes the display title of a preserved copy: `"{base} (Preserved {N} ·
/// {label})"`, or the bare marker when there is no usable base. Any existing
/// marker is stripped from the base first (a preserved copy that is itself
/// preserved never nests markers), and the base is char-truncated so the
/// whole title stays within [`PRESERVED_TITLE_MAX_CHARS`].
pub fn compose_preserved_title(base: Option<&str>, ordinal: u64, device_label: &str) -> String {
    let marker = format!(
        "{}{}{}{}",
        PRESERVED_MARKER_PREFIX, ordinal, PRESERVED_MARKER_SEPARATOR, device_label
    );
    let mut base = base.unwrap_or_default().trim().to_string();
    loop {
        let stripped = strip_preserved_marker(&base).trim().to_string();
        if stripped == base {
            break;
        }
        base = stripped;
    }
    // The suffix costs the marker plus " (" and ")".
    let budget = PRESERVED_TITLE_MAX_CHARS.saturating_sub(marker.chars().count() + 3);
    if base.is_empty() || budget == 0 {
        return marker;
    }
    if base.chars().count() > budget {
        base = base
            .chars()
            .take(budget)
            .collect::<String>()
            .trim_end()
            .to_string();
        if base.is_empty() {
            return marker;
        }
    }
    format!("{} ({})", base, marker)
}

/// One machine-local conversation registration. `thread_id` is this
/// machine's local identity; `cloud_thread_id` is the machine-global half of
/// the `(local, cloud)` pairing — the only place the mapping lives. The cloud
/// stores no mapping and no local identity with linkage semantics.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalConversationBinding {
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub thread_id: ThreadId,
    /// The cloud thread this local thread is paired with; `None` for a
    /// local-only thread (never published, or deliberately detached by
    /// keep-both). Present on the wire even when `None` so pre-cutover rows
    /// fail closed on the missing key.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub cloud_thread_id: Option<CloudThreadId>,
    /// Immediate portable parent for an imported conversation branch, in the
    /// cloud id space (copied verbatim from the pulled cloud row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<CloudThreadId>,
    /// Present when this thread is the local side retained by a divergent
    /// keep-both Pull. Portable: published with the thread on Push and
    /// restored from the cloud record on other machines' pulls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved: Option<PreservedProvenance>,
    /// The immutable portable partition of the bound root task, seeded once
    /// at first full parse.  A bound Codex task without a layout is
    /// corruption: recover it from an exact journal/checkpoint/cloud witness
    /// or fail closed — never re-seed at `WholeThread@0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_tail_layout: Option<RootTailLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    pub materialization_state: ConversationMaterializationState,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl LocalConversationBinding {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(preserved) = &self.preserved {
            preserved.validate()?;
        }
        if let Some(layout) = &self.root_tail_layout {
            layout.validate()?;
        }
        let requires_provider_id = matches!(
            self.materialization_state,
            ConversationMaterializationState::Pending | ConversationMaterializationState::Resumable
        );
        if requires_provider_id != self.provider_thread_id.is_some() {
            return Err(format!(
                "conversation thread '{}' has an invalid provider-thread binding",
                self.thread_id
            ));
        }
        if self.provider_thread_id.as_ref().is_some_and(|thread_id| {
            thread_id.is_empty() || thread_id.len() > 128 || thread_id.chars().any(char::is_control)
        }) {
            return Err("conversation binding has an invalid provider thread ID".to_string());
        }
        if self.created_at > self.updated_at {
            return Err("conversation binding has invalid timestamps".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OriginHostAlias {
    pub bundle_id: BundleId,
    pub origin_host_token: OriginHostToken,
    pub display_name: String,
    pub revision: u64,
    pub updated_at: u64,
}

impl OriginHostAlias {
    pub fn validate(&self) -> Result<(), String> {
        validate_host_label("origin host alias", &self.display_name)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationBindings {
    pub schema: u32,
    pub revision: u64,
    pub host_id: HostId,
    #[serde(default)]
    pub bindings: Vec<LocalConversationBinding>,
    #[serde(default)]
    pub origin_aliases: Vec<OriginHostAlias>,
}

impl ConversationBindings {
    pub fn empty(host_id: HostId) -> Self {
        Self {
            schema: CONVERSATION_BINDINGS_SCHEMA,
            revision: 0,
            host_id,
            bindings: Vec::new(),
            origin_aliases: Vec::new(),
        }
    }

    /// Context-free document invariants: bounds, per-binding validity, and
    /// key uniqueness.  Journaled document snapshots validate through this;
    /// the full `validate` adds host/machine/config cross-checks.
    pub fn validate_structure(&self) -> Result<(), String> {
        if self.schema != CONVERSATION_BINDINGS_SCHEMA
            || self.bindings.len() > MAX_CONVERSATION_BINDINGS
            || self.origin_aliases.len() > MAX_ORIGIN_ALIASES
        {
            return Err("invalid conversation bindings document".to_string());
        }
        let mut binding_keys = HashSet::new();
        let mut provider_owners = HashSet::new();
        let mut cloud_claims = HashSet::new();
        for binding in &self.bindings {
            binding.validate()?;
            if binding.host_id != self.host_id {
                return Err("conversation binding belongs to a different host".to_string());
            }
            let key = (
                binding.host_id.clone(),
                binding.replica_id.clone(),
                binding.local_project_id.clone(),
                binding.provider_profile_id.clone(),
                binding.thread_id.clone(),
            );
            if !binding_keys.insert(key) {
                return Err("duplicate local conversation binding".to_string());
            }
            if let Some(cloud_thread_id) = &binding.cloud_thread_id {
                let claim = (
                    binding.host_id.clone(),
                    binding.replica_id.clone(),
                    binding.local_project_id.clone(),
                    binding.provider_profile_id.clone(),
                    cloud_thread_id.clone(),
                );
                if !cloud_claims.insert(claim) {
                    return Err("cloud thread id is claimed by multiple conversations".to_string());
                }
            }
            if let Some(thread_id) = &binding.provider_thread_id {
                let owner = (
                    binding.host_id.clone(),
                    binding.provider_profile_id.clone(),
                    thread_id.clone(),
                );
                if !provider_owners.insert(owner) {
                    return Err("provider thread ID is owned by multiple conversations".to_string());
                }
            }
        }
        let mut aliases = HashSet::new();
        for alias in &self.origin_aliases {
            alias.validate()?;
            if !aliases.insert((alias.bundle_id.clone(), alias.origin_host_token.clone())) {
                return Err("duplicate origin host alias".to_string());
            }
        }
        Ok(())
    }

    pub fn validate(
        &self,
        identity: &HostIdentity,
        machine: &MachineProjectState,
        config: &SyncConfig,
    ) -> Result<(), String> {
        self.validate_structure()?;
        if self.host_id != identity.host_id {
            return Err("conversation bindings belong to a different host".to_string());
        }

        let profiles = machine
            .profiles
            .iter()
            .map(|profile| (profile.profile_id.clone(), profile))
            .collect::<BTreeMap<_, _>>();
        let replicas = machine
            .bindings
            .iter()
            .map(|binding| (binding.replica_id.clone(), binding))
            .collect::<BTreeMap<_, _>>();
        for binding in &self.bindings {
            let project_binding = replicas
                .get(&binding.replica_id)
                .ok_or_else(|| "conversation binding references an unknown replica".to_string())?;
            if project_binding.host_id != self.host_id
                || project_binding.local_project_id != binding.local_project_id
                || !project_binding
                    .profile_ids
                    .values()
                    .any(|profile_id| profile_id == &binding.provider_profile_id)
                || (project_binding.state == BindingState::Active
                    && config.project(&binding.local_project_id).is_none())
                || !profiles.contains_key(&binding.provider_profile_id)
            {
                return Err("conversation binding project/profile scope is stale".to_string());
            }
            // A materialized cursor-provider task must carry its immutable
            // portable partition under that provider's local codec; a missing
            // or foreign layout is corruption, never a default.
            if binding.provider_thread_id.is_some() {
                let expected_layout_codec =
                    profiles
                        .get(&binding.provider_profile_id)
                        .and_then(|profile| match profile.provider.as_str() {
                            "codex" => Some(PORTABLE_TAIL_CODEC_V1),
                            "claude" => Some(CLAUDE_PORTABLE_TAIL_CODEC_V1),
                            _ => None,
                        });
                if let Some(expected_codec) = expected_layout_codec {
                    if binding
                        .root_tail_layout
                        .as_ref()
                        .is_none_or(|layout| layout.codec != expected_codec)
                    {
                        return Err(format!(
                            "conversation thread '{}' is bound without its provider's root tail \
                             layout",
                            binding.thread_id
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn fallback_origin_host_label(token: &OriginHostToken) -> String {
    format!("Host {}", token.as_str()[..4].to_ascii_uppercase())
}

/// Durable phase of a publishing Push operation.  `RecoveryBlockedCorruption`
/// gates the touched resources for explicit repair; it neither retries nor
/// releases.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PushJournalPhase {
    CandidateUploaded,
    HeadAdvanced,
    LocalCommitted,
    Complete,
    CompleteWithWarnings,
    AbortedCasLost,
    RecoveryBlockedCorruption,
}

impl PushJournalPhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::CompleteWithWarnings | Self::AbortedCasLost
        )
    }

    pub fn is_blocked(self) -> bool {
        self == Self::RecoveryBlockedCorruption
    }
}

/// Durable phase of a materializing Pull operation (including keep-both).
/// Metadata-only Pull actions are journal-free atomic `SyncConfig`
/// replacements and never appear here.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PullJournalPhase {
    TargetStaged,
    ProviderCommitted,
    LocalCommitted,
    Complete,
    CompleteWithWarnings,
    AbortedPrecommit,
    AbortedProviderCollision,
    RecoveryBlockedCorruption,
}

impl PullJournalPhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete
                | Self::CompleteWithWarnings
                | Self::AbortedPrecommit
                | Self::AbortedProviderCollision
        )
    }

    pub fn is_blocked(self) -> bool {
        self == Self::RecoveryBlockedCorruption
    }
}

/// A publishing Push: one immutable candidate, one head CAS, and the exact
/// local after-state to install.  The whole-document before/after states make
/// recovery a deterministic installation, never a recomputation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPublicationOperation {
    pub phase: PushJournalPhase,
    pub candidate_commit_id: String,
    pub candidate_manifest_sha256: String,
    pub artifact_sha256: String,
    /// The accepted tail before this operation; absent exactly for
    /// `CreateCloud`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_before: Option<PortableConversationTail>,
    pub tail_after: PortableConversationTail,
    pub planned_relation: TailClassification,
    pub sync_config_before: SyncConfig,
    pub sync_config_after: SyncConfig,
    /// Publishing also commits the binding after-state: a first `CreateCloud`
    /// creates the binding row with its immutable layout.
    pub bindings_before: ConversationBindings,
    pub bindings_after: ConversationBindings,
}

impl ConversationPublicationOperation {
    fn validate(&self) -> Result<(), String> {
        validate_named_id("conversation candidate commit", &self.candidate_commit_id)?;
        validate_sha256(
            "conversation candidate manifest",
            &self.candidate_manifest_sha256,
        )?;
        validate_sha256("conversation candidate artifact", &self.artifact_sha256)?;
        self.tail_after.validate()?;
        match (self.planned_relation, &self.tail_before) {
            (TailClassification::LocalOutstanding, Some(before)) => {
                before.validate()?;
                if !before.layout().matches(&self.tail_after) {
                    return Err(
                        "conversation publication moves the portable tail layout".to_string()
                    );
                }
            }
            (TailClassification::MissingCloud, None) => {}
            _ => {
                return Err(
                    "conversation publication relation and accepted-before state disagree"
                        .to_string(),
                );
            }
        }
        self.sync_config_before.validate()?;
        self.sync_config_after.validate()?;
        self.bindings_before.validate_structure()?;
        self.bindings_after.validate_structure()
    }
}

/// The identities of a divergent keep-both binding move, minted at apply and
/// journaled before any visible mutation.  The full document mutations live
/// in the operation's before/after states; these fields let recovery assert
/// it is resuming the same move, and receipts read minted keep-both facts
/// from here — never from a plan.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeepBothJournal {
    pub preserved_thread_id: ThreadId,
    pub remote_provider_thread_id: String,
    pub preserved: PreservedProvenance,
    pub preserved_display_title: String,
}

/// A materializing Pull: verified cloud objects reconstructed into staged
/// targets, installed no-clobber, and committed as exact journaled
/// after-states for bindings and `SyncConfig`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationMaterializationOperation {
    pub phase: PullJournalPhase,
    pub planned_relation: TailClassification,
    /// The verified cloud tail this operation accepts.
    pub accepted_tail_after: PortableConversationTail,
    /// The cloud manifest's whole-chain digests: staged transcript bytes must
    /// re-analyze to exactly these before any visible mutation.
    pub pinned_lineage_digest: String,
    pub pinned_semantic_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_both: Option<KeepBothJournal>,
    pub sync_config_before: SyncConfig,
    pub sync_config_after: SyncConfig,
    pub bindings_before: ConversationBindings,
    pub bindings_after: ConversationBindings,
}

impl ConversationMaterializationOperation {
    fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.planned_relation,
            TailClassification::CloudOutstanding
                | TailClassification::Divergent
                | TailClassification::MissingLocal
        ) {
            return Err(
                "conversation materialization has a non-materializing relation".to_string(),
            );
        }
        self.accepted_tail_after.validate()?;
        validate_sha256(
            "conversation materialization lineage pin",
            &self.pinned_lineage_digest,
        )?;
        validate_sha256(
            "conversation materialization semantic pin",
            &self.pinned_semantic_digest,
        )?;
        if (self.planned_relation == TailClassification::Divergent) != self.keep_both.is_some() {
            return Err(
                "conversation keep-both state disagrees with its planned relation".to_string(),
            );
        }
        if let Some(keep_both) = &self.keep_both {
            validate_provider_thread_id(
                "conversation keep-both remote provider thread",
                &keep_both.remote_provider_thread_id,
            )?;
            keep_both.preserved.validate()?;
            validate_display_text(
                "conversation keep-both preserved title",
                &keep_both.preserved_display_title,
                false,
            )?;
        }
        self.sync_config_before.validate()?;
        self.sync_config_after.validate()?;
        self.bindings_before.validate_structure()?;
        self.bindings_after.validate_structure()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", content = "details", rename_all = "snake_case")]
// The operation payloads dwarf a marker enum by design; boxing them would
// change every construction site for no serialized-contract gain.
#[allow(clippy::large_enum_variant)]
pub enum ConversationSyncOperationKind {
    CloudPublication(ConversationPublicationOperation),
    ProviderMaterialization(ConversationMaterializationOperation),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationJournalTarget {
    pub target_path: String,
    pub role: ConversationFileRole,
    pub expected: TargetExpectation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_sha256: Option<String>,
    /// The provider index row ids this target rewrites — exactly the ids the
    /// fragment verifier must find. Non-empty exactly for
    /// `ProviderIndexFragment` targets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub index_rows: Vec<String>,
    #[serde(default)]
    pub commit_started: bool,
    #[serde(default)]
    pub committed: bool,
}

impl ConversationJournalTarget {
    fn validate(&self) -> Result<(), String> {
        validate_absolute_clean_path("conversation journal target", &self.target_path)?;
        self.expected.validate()?;
        if let Some(path) = &self.staged_path {
            validate_absolute_clean_path("conversation journal staging path", path)?;
        }
        if let Some(path) = &self.backup_path {
            validate_absolute_clean_path("conversation journal backup path", path)?;
        }
        if let Some(digest) = &self.staged_sha256 {
            validate_sha256("conversation journal staged target", digest)?;
        }
        if self.staged_path.is_some() != self.staged_sha256.is_some() {
            return Err("conversation journal staging path and digest must be paired".to_string());
        }
        if self.committed && !self.commit_started {
            return Err(
                "conversation journal target cannot be committed before commit starts".to_string(),
            );
        }
        if (self.role == ConversationFileRole::ProviderIndexFragment) == self.index_rows.is_empty()
        {
            return Err(
                "conversation journal index rows must pair with the index-fragment role"
                    .to_string(),
            );
        }
        if self.index_rows.len() > 8 {
            return Err("conversation journal target has too many index rows".to_string());
        }
        let mut row_ids = BTreeSet::new();
        for row_id in &self.index_rows {
            validate_provider_thread_id("conversation journal index row", row_id)?;
            if !row_ids.insert(row_id) {
                return Err("conversation journal target repeats an index row".to_string());
            }
        }
        self.role.validate()?;
        Ok(())
    }
}

/// The reviewed cloud coordinates a sync operation was planned against.
/// Present on every operation: metadata-only actions are journal-free, so
/// every journaled operation touches storage.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationCloudAnchor {
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
}

impl ConversationCloudAnchor {
    fn validate(&self) -> Result<(), String> {
        validate_named_id("conversation journal commit", &self.commit_id)?;
        validate_sha256("conversation journal manifest", &self.manifest_sha256)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSyncOperation {
    pub operation_id: ConversationOperationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<PlanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_action_id: Option<ActionId>,
    pub cloud: ConversationCloudAnchor,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub provider: ProviderKey,
    pub thread_id: ThreadId,
    /// The cloud identity this operation publishes or materializes. Required
    /// and complete: operations are constructed at apply, after minting, so
    /// the journal is the durable record of every identity pairing.
    pub cloud_thread_id: CloudThreadId,
    pub operation: ConversationSyncOperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    pub materialization_state: ConversationMaterializationState,
    pub expected_host_identity_revision: u64,
    pub expected_bindings_document_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_binding_revision: Option<u64>,
    pub expected_profile_revision: u64,
    pub expected_project_binding_revision: u64,
    #[serde(default)]
    pub targets: Vec<ConversationJournalTarget>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl ConversationSyncOperation {
    pub fn direction(&self) -> ConversationPlanDirection {
        match &self.operation {
            ConversationSyncOperationKind::CloudPublication(_) => ConversationPlanDirection::Push,
            ConversationSyncOperationKind::ProviderMaterialization(_) => {
                ConversationPlanDirection::Pull
            }
        }
    }

    pub fn is_terminal(&self) -> bool {
        match &self.operation {
            ConversationSyncOperationKind::CloudPublication(operation) => {
                operation.phase.is_terminal()
            }
            ConversationSyncOperationKind::ProviderMaterialization(operation) => {
                operation.phase.is_terminal()
            }
        }
    }

    pub fn is_blocked(&self) -> bool {
        match &self.operation {
            ConversationSyncOperationKind::CloudPublication(operation) => {
                operation.phase.is_blocked()
            }
            ConversationSyncOperationKind::ProviderMaterialization(operation) => {
                operation.phase.is_blocked()
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.cloud.validate()?;
        if self.plan_action_id.is_some() && self.plan_id.is_none() {
            return Err("conversation journal action provenance requires a plan id".to_string());
        }
        if self.created_at > self.updated_at || self.targets.len() > 64 {
            return Err("invalid conversation journal timestamps or target count".to_string());
        }
        if self.provider_thread_id.as_ref().is_some_and(|thread_id| {
            thread_id.is_empty() || thread_id.len() > 128 || thread_id.chars().any(char::is_control)
        }) {
            return Err("conversation journal has an invalid provider thread id".to_string());
        }
        let requires_provider = matches!(
            self.materialization_state,
            ConversationMaterializationState::Pending | ConversationMaterializationState::Resumable
        );
        if requires_provider != self.provider_thread_id.is_some() {
            return Err("conversation journal has invalid provider materialization".to_string());
        }
        match &self.operation {
            ConversationSyncOperationKind::CloudPublication(operation) => {
                if !self.targets.is_empty() {
                    return Err(
                        "conversation publication journal reserves no provider targets".to_string(),
                    );
                }
                operation.validate()?;
            }
            ConversationSyncOperationKind::ProviderMaterialization(operation) => {
                operation.validate()?;
                if let Some(keep_both) = &operation.keep_both {
                    if keep_both.preserved_thread_id == self.thread_id {
                        return Err("conversation keep-both cannot preserve into its own thread"
                            .to_string());
                    }
                    if keep_both.preserved.preserved_from != self.cloud_thread_id {
                        return Err(
                            "conversation keep-both provenance must name the operation's cloud thread"
                                .to_string(),
                        );
                    }
                }
            }
        }
        let mut targets = BTreeSet::new();
        for target in &self.targets {
            target.validate()?;
            if !targets.insert(target.target_path.to_lowercase()) {
                return Err("conversation journal repeats a target".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSyncJournal {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub operations: Vec<ConversationSyncOperation>,
}

impl ConversationSyncJournal {
    pub fn empty() -> Self {
        Self {
            schema: CONVERSATION_SYNC_JOURNAL_SCHEMA,
            revision: 0,
            operations: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CONVERSATION_SYNC_JOURNAL_SCHEMA
            || self.operations.len() > MAX_CONVERSATION_JOURNAL_OPERATIONS
        {
            return Err("invalid conversation sync journal".to_string());
        }
        let mut ids = BTreeSet::new();
        let mut local_keys = BTreeSet::new();
        let mut cloud_keys = BTreeSet::new();
        let mut plan_actions = BTreeSet::new();
        for operation in &self.operations {
            operation.validate()?;
            if !ids.insert(operation.operation_id.clone()) {
                return Err(format!(
                    "duplicate conversation journal operation '{}'",
                    operation.operation_id
                ));
            }
            // Pending-operation uniqueness, per identity space. The cloud key
            // is deliberately not provider-qualified: cloud ids are
            // bundle-global. Together with the plan-action key this closes
            // the interrupted-retry hole apply-time minting opens — a retried
            // action mints fresh local identities, but shares its cloud id
            // and plan action with the journaled first attempt.
            if !local_keys.insert((
                operation.local_project_id.clone(),
                operation.provider.clone(),
                operation.thread_id.clone(),
            )) {
                return Err(format!(
                    "conversation journal holds two operations for thread '{}'",
                    operation.thread_id
                ));
            }
            if !cloud_keys.insert((
                operation.local_project_id.clone(),
                operation.cloud_thread_id.clone(),
            )) {
                return Err(format!(
                    "conversation journal holds two operations for cloud thread '{}'",
                    operation.cloud_thread_id
                ));
            }
            if let (Some(plan_id), Some(action_id)) =
                (&operation.plan_id, &operation.plan_action_id)
            {
                if !plan_actions.insert((plan_id.clone(), action_id.clone())) {
                    return Err(
                        "conversation journal holds two operations for one plan action".to_string(),
                    );
                }
            }
        }
        Ok(())
    }
}

/// Which local provider profile a setup draft intends to use.  A pending
/// selection stays draft-only; the profile record is created during
/// finalization so an abandoned draft never pollutes the profile catalog.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DraftProfileSelection {
    Existing {
        profile_id: LocalProviderProfileId,
    },
    Pending {
        path: String,
        #[serde(default)]
        display_name: String,
    },
}

/// Which storage a setup draft intends to link.  Pending storage carries the
/// full (possibly still incomplete) configuration, including credentials, so
/// drafts require the same private file permissions as the config itself.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
// A pending storage necessarily carries the whole connection form; boxing
// the draft-only payload buys nothing.
#[allow(clippy::large_enum_variant)]
pub enum DraftStorageSelection {
    Existing { storage_id: StorageId },
    Pending { storage: StorageConfig },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DraftRepositoryChoice {
    /// Publish a new remote repo under the draft's preallocated bundle ID.
    New,
    /// Connect to an existing remote bundle.  A fingerprint mismatch between
    /// checkout and remote must be explicitly acknowledged before finalize.
    Existing {
        storage_id: StorageId,
        bundle_id: BundleId,
        #[serde(default)]
        display_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repository_fingerprint: Option<String>,
        #[serde(default)]
        mismatch_acknowledged: bool,
    },
}

/// A resumable, machine-local project setup draft. Drafts hold configuration
/// choices and preallocated identities only — never discovered resources,
/// remote listings, or payloads.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectSetupDraft {
    pub schema: u32,
    pub draft_id: SetupDraftId,
    /// Preallocated so every finalize retry reconciles to the same records
    /// instead of creating duplicates.
    pub local_project_id: LocalProjectId,
    /// Bundle identity used when `repository` is `New`.
    pub new_bundle_id: BundleId,
    pub project_root: String,
    pub canonical_project_root: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
    /// One entry per enabled provider. An absent key means the provider is
    /// disabled for this project; selecting or editing one provider never
    /// removes another entry.
    #[serde(default)]
    pub profiles: BTreeMap<ProviderKey, DraftProfileSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<DraftStorageSelection>,
    pub repository: DraftRepositoryChoice,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl ProjectSetupDraft {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SETUP_DRAFT_SCHEMA {
            return Err(format!(
                "unsupported setup draft schema {} (expected {})",
                self.schema, SETUP_DRAFT_SCHEMA
            ));
        }
        validate_display_text("draft display name", &self.display_name, false)?;
        validate_absolute_clean_path("draft project root", &self.project_root)?;
        validate_absolute_clean_path("draft canonical project root", &self.canonical_project_root)?;
        if let Some(fingerprint) = &self.repository_fingerprint {
            validate_sha256("draft repository fingerprint", fingerprint)?;
        }
        for selection in self.profiles.values() {
            if let DraftProfileSelection::Pending { path, display_name } = selection {
                validate_absolute_clean_path("draft profile path", path)?;
                validate_display_text("draft profile name", display_name, true)?;
            }
        }
        if let Some(DraftStorageSelection::Pending { storage }) = &self.storage {
            // A draft may hold a half-edited storage; bound the text instead
            // of demanding completeness.  Finalize runs the strict validation.
            validate_display_text("draft storage name", &storage.name, true)?;
            for value in [
                &storage.bucket,
                &storage.account_id,
                &storage.s3_endpoint,
                &storage.region,
                &storage.local_dir,
            ] {
                if value.len() > 4_096 || value.chars().any(char::is_control) {
                    return Err("invalid draft storage field".to_string());
                }
            }
        }
        if let DraftRepositoryChoice::Existing {
            display_name,
            repository_fingerprint,
            ..
        } = &self.repository
        {
            validate_display_text("draft repository name", display_name, true)?;
            if let Some(fingerprint) = repository_fingerprint {
                validate_sha256("draft remote repository fingerprint", fingerprint)?;
            }
        }
        if let Some(error) = &self.last_error {
            if error.len() > 4_096 || error.chars().any(char::is_control) {
                return Err("invalid draft error text".to_string());
            }
        }
        if self.created_at > self.updated_at {
            return Err(format!("draft '{}' has invalid timestamps", self.draft_id));
        }
        Ok(())
    }
}

/// The deterministic records one finalize attempt will create.  Written
/// before the first document mutation so an interrupted finalization can be
/// completed (or safely discarded when nothing was applied) on recovery.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SetupTransaction {
    pub schema: u32,
    pub draft_id: SetupDraftId,
    pub draft_revision: u64,
    pub created_at: u64,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    #[serde(default)]
    pub profiles: Vec<ProviderProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageConfig>,
    pub project: LocalProjectRegistration,
    #[serde(default)]
    pub links: Vec<ProjectStorageLink>,
    pub binding: ProjectBinding,
}

impl SetupTransaction {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SETUP_TRANSACTION_SCHEMA {
            return Err(format!(
                "unsupported setup transaction schema {} (expected {})",
                self.schema, SETUP_TRANSACTION_SCHEMA
            ));
        }
        for profile in &self.profiles {
            profile.validate_structure()?;
        }
        if let Some(storage) = &self.storage {
            storage.validate()?;
        }
        self.project.validate()?;
        for link in &self.links {
            if link.local_project_id != self.project.local_project_id
                || link.bundle_id != self.project.bundle_id
            {
                return Err("setup transaction link does not match its project".to_string());
            }
        }
        self.binding.validate_structure()?;
        if self.binding.host_id != self.host_id
            || self.binding.replica_id != self.replica_id
            || self.binding.local_project_id != self.project.local_project_id
            || self.binding.bundle_id != self.project.bundle_id
        {
            return Err("setup transaction binding does not match its project".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleIdentity {
    pub bundle_id: BundleId,
    pub display_name: String,
    pub kind: BundleKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
}

impl BundleIdentity {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("bundle display name", &self.display_name, false)?;
        if let Some(fingerprint) = &self.repository_fingerprint {
            validate_sha256("repository fingerprint", fingerprint)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct CapturedWith {
    pub app_version: String,
    #[serde(default)]
    pub provider_versions: BTreeMap<ProviderKey, String>,
    #[serde(default)]
    pub codec_versions: BTreeMap<String, u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConversationLineageStatus {
    Complete,
    Unknown { reason_code: String },
}

impl ConversationLineageStatus {
    fn validate(&self) -> Result<(), String> {
        if let Self::Unknown { reason_code } = self {
            validate_display_text("conversation lineage reason", reason_code, false)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationFileRole {
    Transcript,
    ProviderIndexFragment,
    Sidecar { sidecar_kind: String },
}

impl ConversationFileRole {
    fn validate(&self) -> Result<(), String> {
        if let Self::Sidecar { sidecar_kind } = self {
            validate_display_text("conversation sidecar kind", sidecar_kind, false)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationFileRef {
    pub logical_path: LogicalPath,
    pub role: ConversationFileRole,
}

/// Proven, bounded session activity counters. `complete` is false whenever a
/// bounded parse could not account for every record.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionMetrics {
    #[serde(default)]
    pub user_rounds: u64,
    #[serde(default)]
    pub agent_messages: u64,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub complete: bool,
}

/// Bounded provider-neutral presentation metadata stored with each portable
/// conversation so remote sessions render in Activity without payload reads.
/// Never a chat preview: transcript content requires an explicit, verified,
/// bounded fetch.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionPresentation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_sha: Option<String>,
    #[serde(default)]
    pub metrics: SessionMetrics,
}

impl SessionPresentation {
    fn validate(&self) -> Result<(), String> {
        if let Some(branch) = &self.branch {
            validate_display_text("session branch", branch, false)?;
        }
        if let Some(sha) = &self.recorded_sha {
            if sha.is_empty()
                || sha.len() > 64
                || !sha
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err("session recorded sha must be bounded lowercase hex".to_string());
            }
        }
        if let Some(relative) = &self.relative_cwd {
            let invalid_component = relative != "."
                && Path::new(relative)
                    .components()
                    .any(|component| matches!(component, Component::CurDir | Component::ParentDir));
            if relative.is_empty()
                || relative.len() > 1_024
                || Path::new(relative).is_absolute()
                || invalid_component
            {
                return Err("session presentation has an invalid relative cwd".to_string());
            }
        }
        Ok(())
    }
}

/// One cloud conversation row. Keyed entirely in the cloud id space
/// (`cloudthread-…`): the id is minted at push apply, independent of any
/// machine's local thread id, and stays the thread's one portable identity
/// across all storages. The cloud stores no local identity with linkage
/// semantics.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationThread {
    pub thread_id: CloudThreadId,
    pub provider: ProviderKey,
    /// Presentation-only provider title, or a bounded first-user fallback
    /// when the provider has no explicit title. Never used as identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    pub presentation: SessionPresentation,
    pub resource_id: ResourceId,
    #[serde(default)]
    pub files: Vec<ConversationFileRef>,
    pub lineage_status: ConversationLineageStatus,
    pub lineage_codec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_message_id: Option<MessageId>,
    /// Canonical chain length. Required: manifest-only refresh classifies
    /// threads from this and the digest pair without reading transcripts, so
    /// a thread without them cannot be published at all.
    pub message_count: u64,
    pub lineage_digest: String,
    pub semantic_digest: String,
    /// The portable wire value: the sole conversation-content comparison
    /// input. Mandatory when `provider == codex`; optional for providers that
    /// have not adopted the codec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable_tail: Option<PortableConversationTail>,
    /// Provenance of a fork only. Never consulted by the resolver, and the
    /// referenced thread may have been tombstoned or pruned since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<CloudThreadId>,
    /// Keep-both preservation provenance. Like `forked_from`, display-only:
    /// never consulted by the resolver, and the origin thread may have been
    /// tombstoned or pruned since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved: Option<PreservedProvenance>,
    /// A content fact, never identity: the provider session id embedded in
    /// the stored transcript bytes (the pusher's local session id). Consumed
    /// only as content/context — builder self-verification, adapter renaming
    /// hints, display fallbacks — and determines nothing about matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_origin: Option<PortableOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated_by_origin: Option<PortableOrigin>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl ConversationThread {
    fn validate(&self) -> Result<(), String> {
        self.lineage_status.validate()?;
        if let Some(title) = &self.display_title {
            validate_display_text("conversation title", title, false)?;
        }
        self.presentation.validate()?;
        validate_display_text("conversation lineage codec", &self.lineage_codec, false)?;
        let expected_resource_id = cloud_conversation_resource_id(&self.provider, &self.thread_id)?;
        if self.resource_id != expected_resource_id {
            return Err(format!(
                "conversation thread '{}' has non-canonical resource id '{}'",
                self.thread_id, self.resource_id
            ));
        }
        if self.files.is_empty() || self.files.len() > MAX_FILES {
            return Err(format!(
                "conversation thread '{}' has an invalid file inventory",
                self.thread_id
            ));
        }
        let mut paths = BTreeSet::new();
        let mut transcript_count = 0_usize;
        let canonical_transcript =
            cloud_conversation_transcript_path(&self.provider, &self.thread_id)?;
        let canonical_prefix = format!(
            "state/conversations/{}/{}/",
            self.provider.as_str(),
            self.thread_id
        );
        let sidecar_prefix = format!("{}sidecars/", canonical_prefix);
        for file in &self.files {
            file.role.validate()?;
            if file.role == ConversationFileRole::Transcript {
                transcript_count += 1;
                if file.logical_path != canonical_transcript {
                    return Err(format!(
                        "conversation thread '{}' has non-canonical transcript path '{}'",
                        self.thread_id, file.logical_path
                    ));
                }
            }
            if let ConversationFileRole::Sidecar { sidecar_kind } = &file.role {
                let expected_prefix = format!("{}{}/", sidecar_prefix, sidecar_kind);
                if !file.logical_path.as_str().starts_with(&expected_prefix) {
                    return Err(format!(
                        "conversation thread '{}' sidecar '{}' is outside its sidecar namespace",
                        self.thread_id, file.logical_path
                    ));
                }
            }
            if !file.logical_path.as_str().starts_with(&canonical_prefix) {
                return Err(format!(
                    "conversation thread '{}' has file outside its canonical namespace: '{}'",
                    self.thread_id, file.logical_path
                ));
            }
            if !paths.insert(file.logical_path.clone()) {
                return Err(format!(
                    "conversation thread '{}' repeats file '{}'",
                    self.thread_id, file.logical_path
                ));
            }
        }
        if transcript_count != 1 {
            return Err(format!(
                "conversation thread '{}' must own exactly one transcript",
                self.thread_id
            ));
        }
        // Only proven-lineage conversations may be published: manifest-only
        // refresh classifies from the count and digest pair, so an entry
        // whose chain could not be canonicalized has no valid metadata to
        // publish and is rejected outright.
        if let ConversationLineageStatus::Unknown { .. } = self.lineage_status {
            return Err(format!(
                "conversation thread '{}' has unproven lineage and cannot be published",
                self.thread_id
            ));
        }
        if self.head_message_id.is_none()
            || self.message_count == 0
            || self.message_count > MAX_FILES as u64
        {
            return Err(format!(
                "conversation thread '{}' has incomplete lineage metadata",
                self.thread_id
            ));
        }
        validate_sha256("conversation lineage", &self.lineage_digest)?;
        validate_sha256("conversation semantic", &self.semantic_digest)?;
        match &self.portable_tail {
            Some(tail) => {
                if self.provider.as_str() == "codex" {
                    tail.validate_codex_wire()?;
                } else if self.provider.as_str() == "claude" {
                    tail.validate_claude_wire()?;
                } else {
                    tail.validate()?;
                }
                // The accepted tail always ends at the complete canonical
                // root end.
                if tail.accepted_end_record_count != self.message_count {
                    return Err(format!(
                        "conversation thread '{}' has a portable tail that does not end at its message count",
                        self.thread_id
                    ));
                }
            }
            None => {
                if matches!(self.provider.as_str(), "codex" | "claude") {
                    return Err(format!(
                        "conversation thread '{}' requires a portable tail for {}",
                        self.thread_id, self.provider
                    ));
                }
            }
        }
        if self.forked_from.as_ref() == Some(&self.thread_id) {
            return Err(format!(
                "conversation thread '{}' cannot fork from itself",
                self.thread_id
            ));
        }
        if let Some(preserved) = &self.preserved {
            preserved.validate_for_cloud_row(&self.thread_id)?;
        }
        if let Some(thread_id) = &self.source_thread_id {
            if thread_id.is_empty()
                || thread_id.len() > 256
                || thread_id.chars().any(char::is_control)
            {
                return Err("conversation source thread id is invalid".to_string());
            }
        }
        for origin in [
            self.created_by_origin.as_ref(),
            self.last_updated_by_origin.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            origin.validate()?;
        }
        if self.created_at > self.updated_at {
            return Err(format!(
                "conversation thread '{}' has invalid timestamps",
                self.thread_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CanonicalConversationMessage {
    pub message_id: MessageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<MessageId>,
    pub semantic_fingerprint: String,
}

impl CanonicalConversationMessage {
    fn validate(&self) -> Result<(), String> {
        validate_sha256(
            "conversation message semantic fingerprint",
            &self.semantic_fingerprint,
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanDirection {
    Push,
    Pull,
}

/// Presentation outcomes for the compact conversation review row. These are
/// never persisted as executable actions: they explain both mutations and
/// informational/blocked states to the UI.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationReviewAction {
    // Push
    Publish,
    CreateCloud,
    NoChanges,
    RequirePullMetadata,
    RequirePull,
    NotPushable,
    // Pull
    AdoptCloudMetadata,
    ReportPushEligible,
    MaterializeCloud,
    KeepBoth,
    CreateLocal,
    ReportLocalOnly,
    NotFound,
    // Shared
    Blocked,
}

/// The only conversation operations that an approved plan can execute.
/// Informational review outcomes deliberately cannot inhabit this type.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanActionType {
    Publish,
    CreateCloud,
    AdoptCloudMetadata,
    MaterializeCloud,
    KeepBoth,
    CreateLocal,
}

/// A capability or evidence failure that blocks a reviewed direction. Normal
/// reason text is derived from the classified relation and review action;
/// only genuine blockers need an independent value.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationBlocker {
    UnknownEvidence,
    DetachedRequiresPull,
    CodecUnsupported,
    AmbiguousBinding,
    MissingBinding,
    Busy,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", content = "sha256", rename_all = "snake_case")]
pub enum TargetExpectation {
    Absent,
    Sha256(String),
}

impl TargetExpectation {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Sha256(digest) = self {
            validate_sha256("conversation target expectation", digest)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderStateExpectation {
    pub target_path: String,
    pub expectation: TargetExpectation,
}

impl ProviderStateExpectation {
    fn validate(&self) -> Result<(), String> {
        validate_absolute_clean_path("conversation provider target", &self.target_path)?;
        self.expectation.validate()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationContextPins {
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub host_identity_revision: u64,
    pub profile_revision: u64,
    pub project_binding_revision: u64,
    pub bindings_document_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_revision: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationRevisionPins {
    pub bundle_revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub scope_revision: u64,
    pub scope_sha256: String,
}

impl ConversationRevisionPins {
    fn validate(&self) -> Result<(), String> {
        validate_bundle_head_pin(self.bundle_revision, &self.commit_id, &self.manifest_sha256)?;
        validate_sha256("conversation plan scope digest", &self.scope_sha256)?;
        if (self.scope_revision == 0) != self.scope_sha256.bytes().all(|byte| byte == b'0') {
            return Err(
                "conversation plan scope pin has inconsistent absent coordinates".to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationTailPins {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<PortableConversationTail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_local: Option<PortableConversationTail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<PortableConversationTail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<PortableConversationTail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_range: Option<RawRangePin>,
}

impl ConversationTailPins {
    pub fn classification(&self) -> Result<TailClassification, String> {
        let side = |tail: &Option<PortableConversationTail>| match tail {
            Some(tail) => TailSideEvidence::Present(tail.clone()),
            None => TailSideEvidence::ConfirmedAbsent,
        };
        let evidence = VerifiedTailEvidence::new(
            side(&self.accepted),
            side(&self.accepted_local),
            side(&self.local),
            side(&self.cloud),
        )
        .map_err(|blocker| format!("conversation plan tail evidence is invalid: {blocker:?}"))?;
        Ok(compare_conversation_tail(&evidence))
    }

    fn validate(&self) -> Result<(), String> {
        self.classification()?;
        for (label, digest) in [
            ("conversation plan lineage", self.lineage_digest.as_deref()),
            (
                "conversation plan semantic",
                self.semantic_digest.as_deref(),
            ),
        ] {
            if let Some(digest) = digest {
                validate_sha256(label, digest)?;
            }
        }
        if self.lineage_digest.is_some() != self.semantic_digest.is_some() {
            return Err("conversation plan must pin lineage and semantics together".to_string());
        }
        if let Some(pin) = &self.raw_range {
            pin.validate()?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationTargetPins {
    pub local: TargetExpectation,
    #[serde(default)]
    pub provider_state: Vec<ProviderStateExpectation>,
}

impl ConversationTargetPins {
    fn validate(&self) -> Result<(), String> {
        self.local.validate()?;
        if self.provider_state.len() > 64 {
            return Err("conversation plan has too many provider targets".to_string());
        }
        let mut paths = BTreeSet::new();
        for target in &self.provider_state {
            target.validate()?;
            if !paths.insert(target.target_path.to_lowercase()) {
                return Err("conversation plan repeats a provider target".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationActionPins {
    pub context: ConversationContextPins,
    pub revision: ConversationRevisionPins,
    pub tail: ConversationTailPins,
    pub target: ConversationTargetPins,
    pub codec: String,
    pub expires_at: u64,
}

impl ConversationActionPins {
    fn validate(&self) -> Result<(), String> {
        self.revision.validate()?;
        self.tail.validate()?;
        self.target.validate()?;
        validate_display_text("conversation plan codec", &self.codec, false)
    }
}

/// One reviewed conversation action. Plans are pure intent: review classifies
/// relations and pins evidence, but assigns no new identity of any kind — no
/// cloud thread id, no local thread id, no provider session id, no keep-both
/// preserved identity or marker title. Every fresh identity is minted at
/// apply, immediately before journaling; any conflict there fails the action.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPlanAction {
    pub action_id: ActionId,
    pub action: ConversationPlanActionType,
    pub provider: ProviderKey,
    /// The local thread this action operates on; `None` exactly for
    /// `CreateLocal`, whose local identity is minted at apply. Present on
    /// the wire even when `None` so pre-cutover actions fail closed.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub thread_id: Option<ThreadId>,
    /// The cloud row this action was reviewed against; `None` exactly for an
    /// unmapped `CreateCloud`, whose cloud identity is minted at apply.
    /// Present on the wire even when `None` so pre-cutover actions fail
    /// closed.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub cloud_thread_id: Option<CloudThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_resource_id: Option<ResourceId>,
    pub pins: ConversationActionPins,
}

impl ConversationPlanAction {
    pub fn validate(&self) -> Result<(), String> {
        self.pins.validate()?;
        let relation = self.pins.tail.classification()?;
        if !self.action.matches_relation(relation) {
            return Err("conversation plan action disagrees with its tail evidence".to_string());
        }
        // Identity-presence matrix: the local id is absent exactly when apply
        // mints it (CreateLocal); the cloud id is absent only when apply
        // mints it (unmapped CreateCloud). Both-absent is unrepresentable.
        if self.thread_id.is_none() != (self.action == ConversationPlanActionType::CreateLocal) {
            return Err(
                "conversation plan action local identity disagrees with its action".to_string(),
            );
        }
        if self.cloud_thread_id.is_none() && self.action != ConversationPlanActionType::CreateCloud
        {
            return Err(
                "conversation plan action requires its reviewed cloud thread id".to_string(),
            );
        }

        let publishes = matches!(
            self.action,
            ConversationPlanActionType::Publish | ConversationPlanActionType::CreateCloud
        );
        let materializes = matches!(
            self.action,
            ConversationPlanActionType::MaterializeCloud
                | ConversationPlanActionType::KeepBoth
                | ConversationPlanActionType::CreateLocal
        );
        if publishes != self.pins.tail.raw_range.is_some() {
            return Err("conversation plan raw range pin disagrees with its action".to_string());
        }
        if materializes && self.pins.tail.lineage_digest.is_none() {
            return Err("conversation materializing action lacks manifest digest pins".to_string());
        }
        if !materializes && !self.pins.target.provider_state.is_empty() {
            return Err(
                "non-materializing conversation action reserves provider targets".to_string(),
            );
        }
        Ok(())
    }
}

/// Small immutable Pull-review row. `action_id` is present exactly when the
/// row projects an executable action stored in `conversation_actions`. The
/// id pair mirrors the action's: `thread_id` is `None` for cloud-only rows,
/// `cloud_thread_id` is `None` for local-only rows; at least one is present.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationReviewRow {
    pub provider: ProviderKey,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub thread_id: Option<ThreadId>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub cloud_thread_id: Option<CloudThreadId>,
    pub resource_id: ResourceId,
    pub action: ConversationReviewAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<ConversationBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<ActionId>,
}

impl ConversationReviewRow {
    fn validate(&self) -> Result<(), String> {
        if self.thread_id.is_none() && self.cloud_thread_id.is_none() {
            return Err("conversation review row names no identity".to_string());
        }
        if (self.action == ConversationReviewAction::Blocked) != self.blocker.is_some() {
            return Err("conversation review blocker disagrees with its action".to_string());
        }
        if self.action.planned_action().is_some() != self.action_id.is_some() {
            return Err("conversation review action id disagrees with its action".to_string());
        }
        if let Some(reason) = &self.blocked_reason {
            validate_display_text("conversation review blocker", reason, false)?;
        }
        Ok(())
    }
}
/// A reviewed multi-provider Push plan. Provider and profile pins live on
/// every action, never at the top level, so one plan can carry actions for
/// several providers with independent profile revisions.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPushPlan {
    pub schema_version: u32,
    pub plan_id: PlanId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub expected_host_identity_revision: u64,
    pub expected_project_binding_revision: u64,
    pub expected_bindings_document_revision: u64,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub actions: Vec<ConversationPlanAction>,
}

impl ConversationPushPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PUSH_PLAN_SCHEMA {
            return Err(format!(
                "unsupported conversation push-plan schema {}",
                self.schema_version
            ));
        }
        validate_bundle_head_pin(self.revision, &self.commit_id, &self.manifest_sha256)?;
        if self.created_at > self.expires_at || self.actions.len() > MAX_ACTIONS {
            return Err("invalid conversation push-plan lifetime or action count".to_string());
        }
        let mut action_ids = BTreeSet::new();
        let mut claimed_cloud_ids = BTreeSet::new();
        let mut provider_scope_pins: BTreeMap<ProviderKey, (u64, String)> = BTreeMap::new();
        for action in &self.actions {
            action.validate()?;
            if !matches!(
                action.action,
                ConversationPlanActionType::Publish | ConversationPlanActionType::CreateCloud
            ) || action.pins.context.host_id != self.host_id
                || action.pins.context.replica_id != self.replica_id
                || action.pins.context.local_project_id != self.local_project_id
                || action.pins.revision.bundle_revision != self.revision
                || action.pins.revision.commit_id != self.commit_id
                || action.pins.revision.manifest_sha256 != self.manifest_sha256
                || action.pins.context.host_identity_revision
                    != self.expected_host_identity_revision
                || action.pins.context.bindings_document_revision
                    != self.expected_bindings_document_revision
                || action.pins.context.project_binding_revision
                    != self.expected_project_binding_revision
                || action.pins.expires_at != self.expires_at
            {
                return Err("conversation push-plan action pins disagree with the plan".to_string());
            }
            // Push actions always operate on a registered local thread; the
            // action-level matrix leaves `None` to CreateLocal only, so this
            // is a defensive restatement for the push document.
            if action.thread_id.is_none() {
                return Err("conversation push-plan action lacks its local thread id".to_string());
            }
            if let Some(cloud_thread_id) = &action.cloud_thread_id {
                if !claimed_cloud_ids.insert((action.provider.clone(), cloud_thread_id.clone())) {
                    return Err("conversation push-plan claims one cloud thread twice".to_string());
                }
            }
            let scope_pin = (
                action.pins.revision.scope_revision,
                action.pins.revision.scope_sha256.clone(),
            );
            if let Some(previous) = provider_scope_pins.insert(action.provider.clone(), scope_pin) {
                if previous
                    != (
                        action.pins.revision.scope_revision,
                        action.pins.revision.scope_sha256.clone(),
                    )
                {
                    return Err(format!(
                        "conversation push-plan pins two scope states for provider '{}'",
                        action.provider
                    ));
                }
            }
            if !action_ids.insert(action.action_id.clone()) {
                return Err(format!(
                    "duplicate conversation push action '{}'",
                    action.action_id
                ));
            }
        }
        Ok(())
    }
}

pub fn validate_conversation_chain(chain: &[CanonicalConversationMessage]) -> Result<(), String> {
    if chain.is_empty() || chain.len() > MAX_FILES {
        return Err("conversation chain is empty or exceeds its record limit".to_string());
    }
    let mut ids = BTreeSet::new();
    for (index, message) in chain.iter().enumerate() {
        message.validate()?;
        if !ids.insert(message.message_id.clone()) {
            return Err("conversation chain contains a duplicate message id".to_string());
        }
        let expected_parent = index
            .checked_sub(1)
            .and_then(|previous| chain.get(previous))
            .map(|previous| &previous.message_id);
        if message.parent_message_id.as_ref() != expected_parent {
            return Err("conversation chain is not one linear path".to_string());
        }
    }
    Ok(())
}

pub fn conversation_chain_digests(
    chain: &[CanonicalConversationMessage],
) -> Result<(String, String), String> {
    validate_conversation_chain(chain)?;
    let mut lineage = Sha256::new();
    let mut semantic = Sha256::new();
    update_length_prefixed(&mut lineage, b"mallard-conversation-lineage-v1");
    update_length_prefixed(&mut semantic, b"mallard-conversation-semantic-v1");
    for message in chain {
        update_length_prefixed(&mut lineage, message.message_id.as_str().as_bytes());
        update_optional_length_prefixed(
            &mut lineage,
            message.parent_message_id.as_ref().map(MessageId::as_str),
        );
        update_length_prefixed(&mut semantic, message.message_id.as_str().as_bytes());
        update_optional_length_prefixed(
            &mut semantic,
            message.parent_message_id.as_ref().map(MessageId::as_str),
        );
        update_length_prefixed(&mut semantic, message.semantic_fingerprint.as_bytes());
    }
    Ok((
        hex_lower(&lineage.finalize()),
        hex_lower(&semantic.finalize()),
    ))
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn update_optional_length_prefixed(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            update_length_prefixed(hasher, value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

fn stable_conversation_id(prefix: &str, domain: &str, fields: &[&[u8]]) -> Result<String, String> {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, domain.as_bytes());
    for field in fields {
        update_length_prefixed(&mut hasher, field);
    }
    // Conversation IDs use the first 128 digest bits. This matches Mallard's
    // other opaque identities while leaving enough room for the canonical
    // `<provider>:conversation:<thread>` resource ID inside the existing
    // 128-byte ResourceId bound. The domain and length-prefixed encoding keep
    // distinct identity namespaces independent.
    let digest = hasher.finalize();
    let value = format!("{}-{}", prefix, hex_lower(&digest[..16]));
    validate_named_id("stable conversation id", &value)?;
    Ok(value)
}

/// Derive the first machine-local identity for a provider-native
/// conversation. Strictly machine-local: it makes repeated reviews of the
/// same previously-unbound local conversation stable before its first
/// publication commits a binding, and lets an orphaned materialized file
/// whose binding is lost re-derive the same id at rediscovery. It never
/// determines any cloud identity — the cloud id is minted independently at
/// push apply.
pub fn initial_conversation_thread_id(
    bundle_id: &BundleId,
    provider: &ProviderKey,
    provider_thread_id: &str,
) -> Result<ThreadId, String> {
    if provider_thread_id.is_empty()
        || provider_thread_id.len() > 512
        || provider_thread_id.chars().any(char::is_control)
    {
        return Err("provider thread id must be 1-512 non-control characters".to_string());
    }
    ThreadId::parse(stable_conversation_id(
        "thread",
        "mallard-initial-conversation-thread-v1",
        &[
            bundle_id.as_str().as_bytes(),
            provider.as_str().as_bytes(),
            provider_thread_id.as_bytes(),
        ],
    )?)
}

/// Derive the stable local identity used when copy-on-write retains a
/// provider-native conversation after its portable thread moves to a fresh
/// provider alias. While the retained bytes remain unchanged this identity
/// stays hidden as accepted physical history; if the provider app later
/// appends to those bytes, the same identity makes that work reappear as a
/// distinct local branch instead of colliding with the moved thread.
pub fn retained_conversation_thread_id(
    bundle_id: &BundleId,
    provider: &ProviderKey,
    displaced_thread_id: &ThreadId,
    provider_thread_id: &str,
) -> Result<ThreadId, String> {
    if provider_thread_id.is_empty()
        || provider_thread_id.len() > 512
        || provider_thread_id.chars().any(char::is_control)
    {
        return Err("provider thread id must be 1-512 non-control characters".to_string());
    }
    ThreadId::parse(stable_conversation_id(
        "thread",
        "mallard-retained-conversation-thread-v1",
        &[
            bundle_id.as_str().as_bytes(),
            provider.as_str().as_bytes(),
            displaced_thread_id.as_str().as_bytes(),
            provider_thread_id.as_bytes(),
        ],
    )?)
}

pub fn conversation_resource_id(
    provider: &ProviderKey,
    thread_id: &ThreadId,
) -> Result<ResourceId, String> {
    ResourceId::parse(format!("{}:conversation:{}", provider.as_str(), thread_id))
}

/// The cloud-space twin of [`conversation_resource_id`], keying manifest
/// rows. The two id namespaces are prefix-disjoint, so the two resource-id
/// forms can never collide.
pub fn cloud_conversation_resource_id(
    provider: &ProviderKey,
    thread_id: &CloudThreadId,
) -> Result<ResourceId, String> {
    ResourceId::parse(format!("{}:conversation:{}", provider.as_str(), thread_id))
}

/// Parse the canonical portable conversation resource identity
/// (`<provider>:conversation:<thread>`). Provider-native capture identities
/// such as `<provider>:session:<id>` deliberately do not match.
pub fn parse_conversation_resource_id(
    resource_id: &str,
) -> Result<(ProviderKey, ThreadId), String> {
    let mut parts = resource_id.splitn(3, ':');
    let provider = parts
        .next()
        .ok_or_else(|| "resource id is empty".to_string())?;
    let class = parts.next();
    let thread = parts.next();
    if class != Some("conversation") {
        return Err(format!(
            "'{}' is not a conversation resource id",
            resource_id
        ));
    }
    let thread = thread.ok_or_else(|| format!("'{}' has no thread id", resource_id))?;
    Ok((ProviderKey::parse(provider)?, ThreadId::parse(thread)?))
}

/// Parse the canonical cloud conversation resource identity
/// (`<provider>:conversation:cloudthread-…`). Local-space resource ids
/// deliberately do not match, exactly as cloud-space ids fail
/// [`parse_conversation_resource_id`].
pub fn parse_cloud_conversation_resource_id(
    resource_id: &str,
) -> Result<(ProviderKey, CloudThreadId), String> {
    let mut parts = resource_id.splitn(3, ':');
    let provider = parts
        .next()
        .ok_or_else(|| "resource id is empty".to_string())?;
    let class = parts.next();
    let thread = parts.next();
    if class != Some("conversation") {
        return Err(format!(
            "'{}' is not a conversation resource id",
            resource_id
        ));
    }
    let thread = thread.ok_or_else(|| format!("'{}' has no thread id", resource_id))?;
    Ok((ProviderKey::parse(provider)?, CloudThreadId::parse(thread)?))
}

/// True when the resource id names a conversation in either id space
/// (`<provider>:conversation:<local or cloud thread>`). Machine-local
/// selection state excludes both: local conversation identities key reviews
/// and receipts, cloud identities key manifest rows.
pub fn is_conversation_resource_id(resource_id: &str) -> bool {
    parse_conversation_resource_id(resource_id).is_ok()
        || parse_cloud_conversation_resource_id(resource_id).is_ok()
}

pub fn conversation_transcript_path(
    provider: &ProviderKey,
    thread_id: &ThreadId,
) -> Result<LogicalPath, String> {
    LogicalPath::parse(format!(
        "state/conversations/{}/{}/transcript.jsonl",
        provider.as_str(),
        thread_id
    ))
}

pub fn cloud_conversation_transcript_path(
    provider: &ProviderKey,
    thread_id: &CloudThreadId,
) -> Result<LogicalPath, String> {
    LogicalPath::parse(format!(
        "state/conversations/{}/{}/transcript.jsonl",
        provider.as_str(),
        thread_id
    ))
}

pub fn conversation_sidecar_path(
    provider: &ProviderKey,
    thread_id: &ThreadId,
    sidecar_kind: &str,
    file_name: &str,
) -> Result<LogicalPath, String> {
    LogicalPath::parse(format!(
        "state/conversations/{}/{}/sidecars/{}/{}",
        provider.as_str(),
        thread_id,
        sidecar_kind,
        file_name
    ))
}

pub fn cloud_conversation_sidecar_path(
    provider: &ProviderKey,
    thread_id: &CloudThreadId,
    sidecar_kind: &str,
    file_name: &str,
) -> Result<LogicalPath, String> {
    LogicalPath::parse(format!(
        "state/conversations/{}/{}/sidecars/{}/{}",
        provider.as_str(),
        thread_id,
        sidecar_kind,
        file_name
    ))
}

/// The bounded namespace for provider state that is not owned by one
/// conversation, such as provider memory.
pub fn provider_state_path_prefix(provider: &ProviderKey) -> String {
    format!("state/providers/{}/", provider.as_str())
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleFileEntry {
    pub resource_id: ResourceId,
    pub sha256: String,
    pub size: u64,
    pub source_mtime: u64,
    pub object_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleDirectoryEntry {
    pub resource_id: ResourceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    pub source_mtime: u64,
}

impl BundleDirectoryEntry {
    pub fn validate(&self) -> Result<(), String> {
        if self.mode.is_some_and(|mode| mode & !0o777 != 0) {
            return Err("bundle directory mode contains unsafe bits".to_string());
        }
        Ok(())
    }
}

impl BundleFileEntry {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("bundle file", &self.sha256)?;
        if self.object_key.is_empty()
            || self.object_key.len() > 2_048
            || self.object_key.starts_with('/')
            || self.object_key.contains('\\')
            || self
                .object_key
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(format!("unsafe bundle object key '{}'", self.object_key));
        }
        if self.mode.is_some_and(|mode| mode & !0o777 != 0) {
            return Err("bundle file mode contains unsafe bits".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TombstoneTarget {
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Tombstone {
    pub target: TombstoneTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sha256: Option<String>,
    pub deleted_at: u64,
}

impl Tombstone {
    pub fn canonical_key(&self) -> String {
        match &self.target {
            TombstoneTarget::Resource { resource_id } => format!("resource:{}", resource_id),
            TombstoneTarget::File {
                resource_id,
                logical_path,
            } => format!("file:{}:{}", resource_id, logical_path),
            TombstoneTarget::ProjectContentFile {
                resource_id,
                logical_path,
            } => format!("project-content-file:{}:{}", resource_id, logical_path),
            TombstoneTarget::ProjectContentDirectory {
                resource_id,
                logical_path,
            } => format!("project-content-dir:{}:{}", resource_id, logical_path),
            TombstoneTarget::ConversationThread { thread_id } => {
                format!("conversation-thread:{}", thread_id)
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(digest) = &self.last_sha256 {
            validate_sha256("tombstone digest", digest)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleHead {
    pub schema_version: u32,
    pub bundle_id: BundleId,
    pub kind: BundleKind,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_key: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub updated_at: u64,
}

impl BundleHead {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != BUNDLE_SCHEMA {
            return Err(format!(
                "unsupported bundle head schema {}",
                self.schema_version
            ));
        }
        validate_named_id("commit id", &self.commit_id)?;
        validate_sha256("head manifest", &self.manifest_sha256)?;
        if self.manifest_size == 0 {
            return Err("head manifest size must be nonzero".to_string());
        }
        if self.manifest_key.is_empty()
            || self.manifest_key.starts_with('/')
            || self.manifest_key.contains("..")
            || self.manifest_key.contains('\\')
        {
            return Err("unsafe manifest key".to_string());
        }
        Ok(())
    }
}

/// One independently revisioned slice of a portable repository. Every
/// reference inside a scope must stay inside that scope: conversations may
/// only name files of their own provider scope, provider resources cannot
/// point into another provider, and project content lives only in the
/// project scope.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleScope {
    pub revision: u64,
    /// Canonical content digest of this scope. Computed by
    /// [`BundleScope::compute_content_sha256`]; excludes itself and covers
    /// the scope key, revision, and all live and tombstoned content.
    pub content_sha256: String,
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    #[serde(default)]
    pub conversations: BTreeMap<CloudThreadId, ConversationThread>,
    #[serde(default)]
    pub resources: BTreeMap<ResourceId, ResourceDescriptor>,
    #[serde(default)]
    pub files: BTreeMap<LogicalPath, BundleFileEntry>,
    #[serde(default)]
    pub directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
    #[serde(default)]
    pub tombstones: BTreeMap<String, Tombstone>,
}

/// The canonical encoding hashed into a scope's `content_sha256`. Field
/// order is fixed by this struct and every collection is an ordered map, so
/// `serde_json` serialization is deterministic. Changing this layout changes
/// every scope digest and is a contract change.
#[derive(Serialize)]
struct BundleScopeDigestView<'a> {
    scope: String,
    revision: u64,
    recipe: &'a BundleRecipe,
    captured_with: &'a CapturedWith,
    conversations: &'a BTreeMap<CloudThreadId, ConversationThread>,
    resources: &'a BTreeMap<ResourceId, ResourceDescriptor>,
    files: &'a BTreeMap<LogicalPath, BundleFileEntry>,
    directories: &'a BTreeMap<LogicalPath, BundleDirectoryEntry>,
    tombstones: &'a BTreeMap<String, Tombstone>,
}

impl BundleScope {
    /// Build a scope and seal its canonical content digest.
    #[allow(clippy::too_many_arguments)]
    pub fn sealed(
        scope: &SyncScopeKey,
        revision: u64,
        recipe: BundleRecipe,
        captured_with: CapturedWith,
        conversations: BTreeMap<CloudThreadId, ConversationThread>,
        resources: BTreeMap<ResourceId, ResourceDescriptor>,
        files: BTreeMap<LogicalPath, BundleFileEntry>,
        directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
        tombstones: BTreeMap<String, Tombstone>,
    ) -> Result<Self, String> {
        let mut built = Self {
            revision,
            content_sha256: "0".repeat(64),
            recipe,
            captured_with,
            conversations,
            resources,
            files,
            directories,
            tombstones,
        };
        built.content_sha256 = built.compute_content_sha256(scope)?;
        Ok(built)
    }

    pub fn compute_content_sha256(&self, scope: &SyncScopeKey) -> Result<String, String> {
        let view = BundleScopeDigestView {
            scope: scope.canonical(),
            revision: self.revision,
            recipe: &self.recipe,
            captured_with: &self.captured_with,
            conversations: &self.conversations,
            resources: &self.resources,
            files: &self.files,
            directories: &self.directories,
            tombstones: &self.tombstones,
        };
        let bytes = serde_json::to_vec(&view)
            .map_err(|error| format!("encode scope '{}' digest view: {}", scope, error))?;
        Ok(hex_lower(&Sha256::digest(&bytes)))
    }

    fn validate(&self, scope: &SyncScopeKey) -> Result<(), String> {
        if self.revision == 0 {
            return Err(format!("scope '{}' has a zero revision", scope));
        }
        validate_sha256("scope content", &self.content_sha256)?;
        let computed = self.compute_content_sha256(scope)?;
        if computed != self.content_sha256 {
            return Err(format!("scope '{}' content digest mismatch", scope));
        }
        self.recipe.validate()?;
        if self.resources.len() > MAX_RESOURCES
            || self.files.len() > MAX_FILES
            || self.directories.len() > MAX_FILES
            || self.tombstones.len() > MAX_FILES
            || self.conversations.len() > MAX_RESOURCES
        {
            return Err(format!("scope '{}' exceeds collection limits", scope));
        }
        for (id, descriptor) in &self.resources {
            if id != &descriptor.resource_id {
                return Err(format!("resource key '{}' does not match descriptor", id));
            }
            descriptor.validate()?;
        }
        let recipe_ids = self.recipe.selected_ids();
        let resource_ids: BTreeSet<_> = self.resources.keys().cloned().collect();
        if recipe_ids != resource_ids {
            return Err(format!(
                "scope '{}' recipe and live resources differ",
                scope
            ));
        }
        match scope {
            SyncScopeKey::Project => self.validate_project_scope(),
            SyncScopeKey::Provider(provider) => self.validate_provider_scope(provider),
        }
    }

    fn validate_project_scope(&self) -> Result<(), String> {
        if !self.conversations.is_empty() {
            return Err("project scope cannot contain conversations".to_string());
        }
        for descriptor in self.resources.values() {
            if !matches!(
                descriptor.kind,
                ResourceKind::ProjectContentFile
                    | ResourceKind::ProjectContentDirectory
                    | ResourceKind::Requirement
            ) {
                return Err(format!(
                    "project scope cannot hold {:?} resource '{}'",
                    descriptor.kind, descriptor.resource_id
                ));
            }
        }
        let mut file_owners = BTreeMap::<ResourceId, usize>::new();
        let mut directory_owners = BTreeMap::<ResourceId, usize>::new();
        let mut project_content_bytes = 0_u64;
        for (path, entry) in &self.directories {
            entry.validate()?;
            let descriptor = self.resources.get(&entry.resource_id).ok_or_else(|| {
                format!(
                    "directory '{}' references missing resource '{}'",
                    path, entry.resource_id
                )
            })?;
            if descriptor.kind != ResourceKind::ProjectContentDirectory {
                return Err(format!(
                    "directory '{}' is owned by non-directory resource '{}'",
                    path, entry.resource_id
                ));
            }
            validate_project_content_descriptor_identity(descriptor, path, "dir")?;
            *directory_owners
                .entry(entry.resource_id.clone())
                .or_default() += 1;
            let components = path.as_str().split('/').collect::<Vec<_>>();
            for end in 2..components.len() {
                let ancestor = LogicalPath::parse(components[..end].join("/"))?;
                if !self.directories.contains_key(&ancestor) {
                    return Err(format!(
                        "project-content directory '{}' lacks directory entry '{}'",
                        path, ancestor
                    ));
                }
            }
        }
        for (path, entry) in &self.files {
            entry.validate()?;
            if path.as_str().starts_with("state/") {
                return Err(format!(
                    "project scope cannot hold provider-state file '{}'",
                    path
                ));
            }
            let descriptor = self.resources.get(&entry.resource_id).ok_or_else(|| {
                format!(
                    "file '{}' references missing resource '{}'",
                    path, entry.resource_id
                )
            })?;
            *file_owners.entry(entry.resource_id.clone()).or_default() += 1;
            if descriptor.kind == ResourceKind::ProjectContentFile {
                validate_project_content_descriptor_identity(descriptor, path, "file")?;
                if entry.size > MAX_PROJECT_CONTENT_FILE_BYTES {
                    return Err(format!(
                        "project-content file '{}' exceeds {} bytes",
                        path, MAX_PROJECT_CONTENT_FILE_BYTES
                    ));
                }
                project_content_bytes = project_content_bytes
                    .checked_add(entry.size)
                    .ok_or_else(|| "project-content byte total overflow".to_string())?;
                if project_content_bytes > MAX_PROJECT_CONTENT_TOTAL_BYTES {
                    return Err(format!(
                        "project content exceeds {} bytes",
                        MAX_PROJECT_CONTENT_TOTAL_BYTES
                    ));
                }
                let components = path.as_str().split('/').collect::<Vec<_>>();
                for end in 2..components.len() {
                    let ancestor = LogicalPath::parse(components[..end].join("/"))?;
                    if !self.directories.contains_key(&ancestor) {
                        return Err(format!(
                            "project-content file '{}' lacks directory entry '{}'",
                            path, ancestor
                        ));
                    }
                }
            }
        }
        for (resource_id, descriptor) in &self.resources {
            let file_count = file_owners.get(resource_id).copied().unwrap_or_default();
            let directory_count = directory_owners
                .get(resource_id)
                .copied()
                .unwrap_or_default();
            match descriptor.kind {
                ResourceKind::ProjectContentFile if file_count != 1 || directory_count != 0 => {
                    return Err(format!(
                        "project-content file resource '{}' must own exactly one file",
                        resource_id
                    ));
                }
                ResourceKind::ProjectContentDirectory
                    if directory_count != 1 || file_count != 0 =>
                {
                    return Err(format!(
                        "project-content directory resource '{}' must own exactly one directory",
                        resource_id
                    ));
                }
                _ => {}
            }
        }
        for (key, tombstone) in &self.tombstones {
            tombstone.validate()?;
            if key != &tombstone.canonical_key() {
                return Err(format!("non-canonical tombstone key '{}'", key));
            }
            match &tombstone.target {
                TombstoneTarget::Resource { resource_id } => {
                    if self.resources.contains_key(resource_id) {
                        return Err(format!(
                            "live resource '{}' also has a tombstone",
                            resource_id
                        ));
                    }
                }
                TombstoneTarget::File {
                    resource_id,
                    logical_path,
                } => {
                    if self.files.contains_key(logical_path) {
                        return Err(format!("live file '{}' also has a tombstone", logical_path));
                    }
                    if logical_path.as_str().starts_with("state/") {
                        return Err(format!(
                            "project scope cannot tombstone provider file '{}'",
                            logical_path
                        ));
                    }
                    if self.resources.contains_key(resource_id)
                        && !self.recipe.entries.contains_key(resource_id)
                    {
                        return Err(format!("invalid file tombstone resource '{}'", resource_id));
                    }
                }
                TombstoneTarget::ProjectContentFile {
                    resource_id,
                    logical_path,
                } => {
                    if self.files.contains_key(logical_path) {
                        return Err(format!(
                            "live project-content file '{}' also has a tombstone",
                            logical_path
                        ));
                    }
                    let (_, expected_id) = project_content_identity("file", logical_path)?;
                    if resource_id != &expected_id {
                        return Err(format!(
                            "project-content file tombstone resource '{}' does not match '{}'",
                            resource_id, logical_path
                        ));
                    }
                    if self.resources.contains_key(resource_id) {
                        return Err(format!(
                            "live resource '{}' also has a project-content tombstone",
                            resource_id
                        ));
                    }
                    if tombstone.last_sha256.is_none() {
                        return Err(format!(
                            "project-content file tombstone '{}' lacks its last digest",
                            logical_path
                        ));
                    }
                }
                TombstoneTarget::ProjectContentDirectory {
                    resource_id,
                    logical_path,
                } => {
                    if self.directories.contains_key(logical_path) {
                        return Err(format!(
                            "live project-content directory '{}' also has a tombstone",
                            logical_path
                        ));
                    }
                    let (_, expected_id) = project_content_identity("dir", logical_path)?;
                    if resource_id != &expected_id {
                        return Err(format!(
                            "project-content directory tombstone resource '{}' does not match '{}'",
                            resource_id, logical_path
                        ));
                    }
                    if self.resources.contains_key(resource_id) {
                        return Err(format!(
                            "live resource '{}' also has a project-content tombstone",
                            resource_id
                        ));
                    }
                    if tombstone.last_sha256.is_some() {
                        return Err(format!(
                            "project-content directory tombstone '{}' cannot have a digest",
                            logical_path
                        ));
                    }
                }
                TombstoneTarget::ConversationThread { thread_id } => {
                    return Err(format!(
                        "project scope cannot tombstone conversation thread '{}'",
                        thread_id
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_provider_scope(&self, provider: &ProviderKey) -> Result<(), String> {
        if !self.directories.is_empty() {
            return Err(format!(
                "provider scope '{}' cannot contain directory entries",
                provider
            ));
        }
        let conversation_prefix = format!("state/conversations/{}/", provider);
        let state_prefix = provider_state_path_prefix(provider);
        for descriptor in self.resources.values() {
            if descriptor.provider.as_ref() != Some(provider) {
                return Err(format!(
                    "provider scope '{}' cannot hold resource '{}' of another provider",
                    provider, descriptor.resource_id
                ));
            }
            if !matches!(
                descriptor.kind,
                ResourceKind::Conversation | ResourceKind::ProviderState
            ) {
                return Err(format!(
                    "provider scope '{}' cannot hold {:?} resource '{}'",
                    provider, descriptor.kind, descriptor.resource_id
                ));
            }
        }
        let mut thread_resources = BTreeSet::new();
        let mut referenced_files = BTreeMap::<LogicalPath, ResourceId>::new();
        for (thread_id, thread) in &self.conversations {
            if thread_id != &thread.thread_id {
                return Err(format!(
                    "conversation thread key '{}' does not match its embedded id",
                    thread_id
                ));
            }
            thread.validate()?;
            if &thread.provider != provider {
                return Err(format!(
                    "conversation thread '{}' belongs to provider '{}', not scope '{}'",
                    thread_id, thread.provider, provider
                ));
            }
            if !thread_resources.insert(thread.resource_id.clone()) {
                return Err(format!(
                    "conversation resource '{}' belongs to multiple threads",
                    thread.resource_id
                ));
            }
            let descriptor = self.resources.get(&thread.resource_id).ok_or_else(|| {
                format!(
                    "conversation thread '{}' references missing resource '{}'",
                    thread_id, thread.resource_id
                )
            })?;
            if descriptor.kind != ResourceKind::Conversation
                || descriptor.provider.as_ref() != Some(provider)
            {
                return Err(format!(
                    "conversation thread '{}' disagrees with its resource provider",
                    thread_id
                ));
            }
            if !self
                .captured_with
                .codec_versions
                .contains_key(&thread.lineage_codec)
            {
                return Err(format!(
                    "conversation thread '{}' references unknown codec '{}'",
                    thread_id, thread.lineage_codec
                ));
            }
            let declared_paths = thread
                .files
                .iter()
                .map(|file| file.logical_path.clone())
                .collect::<BTreeSet<_>>();
            for file in &thread.files {
                let entry = self.files.get(&file.logical_path).ok_or_else(|| {
                    format!(
                        "conversation thread '{}' references missing file '{}'",
                        thread_id, file.logical_path
                    )
                })?;
                if entry.resource_id != thread.resource_id {
                    return Err(format!(
                        "conversation file '{}' has the wrong resource owner",
                        file.logical_path
                    ));
                }
                if let Some(previous) =
                    referenced_files.insert(file.logical_path.clone(), thread.resource_id.clone())
                {
                    return Err(format!(
                        "conversation file '{}' is shared by '{}' and '{}'",
                        file.logical_path, previous, thread.resource_id
                    ));
                }
            }
            for (path, entry) in &self.files {
                if entry.resource_id == thread.resource_id && !declared_paths.contains(path) {
                    return Err(format!(
                        "conversation resource '{}' owns undeclared file '{}'",
                        thread.resource_id, path
                    ));
                }
            }
        }
        for (resource_id, descriptor) in &self.resources {
            if descriptor.kind == ResourceKind::Conversation
                && !thread_resources.contains(resource_id)
            {
                return Err(format!(
                    "conversation resource '{}' belongs to no live thread",
                    resource_id
                ));
            }
        }
        for (path, entry) in &self.files {
            entry.validate()?;
            let in_conversations = path.as_str().starts_with(&conversation_prefix);
            let in_state = path.as_str().starts_with(&state_prefix);
            if !in_conversations && !in_state {
                return Err(format!(
                    "provider scope '{}' cannot hold file '{}' outside its namespaces",
                    provider, path
                ));
            }
            let descriptor = self.resources.get(&entry.resource_id).ok_or_else(|| {
                format!(
                    "file '{}' references missing resource '{}'",
                    path, entry.resource_id
                )
            })?;
            if in_state && descriptor.kind != ResourceKind::ProviderState {
                return Err(format!(
                    "provider-state file '{}' must be owned by a provider-state resource",
                    path
                ));
            }
            if in_conversations && descriptor.kind != ResourceKind::Conversation {
                return Err(format!(
                    "conversation-namespace file '{}' must be owned by a conversation resource",
                    path
                ));
            }
        }
        for (resource_id, descriptor) in &self.resources {
            if descriptor.kind == ResourceKind::ProviderState
                && !self
                    .files
                    .values()
                    .any(|entry| &entry.resource_id == resource_id)
            {
                return Err(format!(
                    "provider-state resource '{}' owns no files",
                    resource_id
                ));
            }
        }
        for start in self.conversations.keys() {
            let mut path = BTreeSet::new();
            let mut cursor = start;
            while let Some(parent) = self
                .conversations
                .get(cursor)
                .and_then(|thread| thread.forked_from.as_ref())
                .filter(|parent| self.conversations.contains_key(*parent))
            {
                if !path.insert(cursor.clone()) || path.contains(parent) {
                    return Err(format!(
                        "conversation fork ancestry contains a cycle at '{}'",
                        parent
                    ));
                }
                cursor = parent;
            }
        }
        for (key, tombstone) in &self.tombstones {
            tombstone.validate()?;
            if key != &tombstone.canonical_key() {
                return Err(format!("non-canonical tombstone key '{}'", key));
            }
            match &tombstone.target {
                TombstoneTarget::Resource { resource_id } => {
                    if self.resources.contains_key(resource_id) {
                        return Err(format!(
                            "live resource '{}' also has a tombstone",
                            resource_id
                        ));
                    }
                }
                TombstoneTarget::File {
                    resource_id,
                    logical_path,
                } => {
                    if self.files.contains_key(logical_path) {
                        return Err(format!("live file '{}' also has a tombstone", logical_path));
                    }
                    if !logical_path.as_str().starts_with(&conversation_prefix)
                        && !logical_path.as_str().starts_with(&state_prefix)
                    {
                        return Err(format!(
                            "provider scope '{}' cannot tombstone file '{}' outside its namespaces",
                            provider, logical_path
                        ));
                    }
                    if self.resources.contains_key(resource_id)
                        && !self.recipe.entries.contains_key(resource_id)
                    {
                        return Err(format!("invalid file tombstone resource '{}'", resource_id));
                    }
                }
                TombstoneTarget::ConversationThread { thread_id } => {
                    if self.conversations.contains_key(thread_id) {
                        return Err(format!(
                            "live conversation thread '{}' also has a tombstone",
                            thread_id
                        ));
                    }
                    if tombstone.last_sha256.is_none() {
                        return Err(format!(
                            "conversation thread tombstone '{}' lacks its last digest",
                            thread_id
                        ));
                    }
                }
                TombstoneTarget::ProjectContentFile { logical_path, .. }
                | TombstoneTarget::ProjectContentDirectory { logical_path, .. } => {
                    return Err(format!(
                        "provider scope '{}' cannot tombstone project content '{}'",
                        provider, logical_path
                    ));
                }
            }
        }
        Ok(())
    }
}

/// A portable repository manifest composed of independently revisioned sync
/// scopes. The manifest-level revision and commit identify one global CAS
/// publication; each scope carries its own revision and content digest so
/// unrelated providers never block each other.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub revision: u64,
    pub commit_id: String,
    pub updated_at: u64,
    pub bundle: BundleIdentity,
    pub published_by_origin: PortableOrigin,
    #[serde(default)]
    pub scopes: BTreeMap<SyncScopeKey, BundleScope>,
}

impl BundleManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != BUNDLE_SCHEMA {
            return Err(format!(
                "unsupported bundle manifest schema {}",
                self.schema_version
            ));
        }
        self.published_by_origin.validate()?;
        validate_named_id("commit id", &self.commit_id)?;
        self.bundle.validate()?;
        let provider_scope_count = self
            .scopes
            .keys()
            .filter(|scope| matches!(scope, SyncScopeKey::Provider(_)))
            .count();
        if provider_scope_count > MAX_PROVIDER_SCOPES {
            return Err(format!(
                "bundle manifest exceeds {} provider scopes",
                MAX_PROVIDER_SCOPES
            ));
        }
        let mut total_resources = 0_usize;
        let mut total_files = 0_usize;
        let mut total_directories = 0_usize;
        let mut total_tombstones = 0_usize;
        for (scope_key, scope) in &self.scopes {
            scope.validate(scope_key)?;
            total_resources = total_resources.saturating_add(scope.resources.len());
            total_files = total_files.saturating_add(scope.files.len());
            total_directories = total_directories.saturating_add(scope.directories.len());
            total_tombstones = total_tombstones.saturating_add(scope.tombstones.len());
        }
        if total_resources > MAX_RESOURCES
            || total_files > MAX_FILES
            || total_directories > MAX_FILES
            || total_tombstones > MAX_FILES
        {
            return Err("bundle manifest exceeds collection limits".to_string());
        }
        let mut folded_paths = BTreeMap::<String, (LogicalPath, &'static str)>::new();
        let mut thread_ids = BTreeSet::new();
        let mut resource_ids = BTreeSet::new();
        for scope in self.scopes.values() {
            for thread_id in scope.conversations.keys() {
                if !thread_ids.insert(thread_id.clone()) {
                    return Err(format!(
                        "conversation thread '{}' appears in multiple scopes",
                        thread_id
                    ));
                }
            }
            for resource_id in scope.resources.keys() {
                if !resource_ids.insert(resource_id.clone()) {
                    return Err(format!(
                        "resource '{}' appears in multiple scopes",
                        resource_id
                    ));
                }
            }
            for path in scope.directories.keys() {
                let folded = path.as_str().to_lowercase();
                if let Some((previous, previous_kind)) =
                    folded_paths.insert(folded, (path.clone(), "directory"))
                {
                    return Err(format!(
                        "portable path collision between {} '{}' and directory '{}'",
                        previous_kind, previous, path
                    ));
                }
            }
            for path in scope.files.keys() {
                let folded = path.as_str().to_lowercase();
                if let Some((previous, previous_kind)) =
                    folded_paths.insert(folded, (path.clone(), "file"))
                {
                    return Err(format!(
                        "portable path collision between {} '{}' and file '{}'",
                        previous_kind, previous, path
                    ));
                }
            }
        }
        for scope in self.scopes.values() {
            for path in scope.files.keys() {
                let components: Vec<_> = path.as_str().split('/').collect();
                for end in 1..components.len() {
                    let ancestor = components[..end].join("/").to_lowercase();
                    if let Some((existing, kind)) = folded_paths.get(&ancestor) {
                        if *kind == "directory" {
                            continue;
                        }
                        return Err(format!(
                            "manifest file '{}' is ancestor of '{}'",
                            existing, path
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn scope(&self, key: &SyncScopeKey) -> Option<&BundleScope> {
        self.scopes.get(key)
    }

    pub fn provider_scope(&self, provider: &ProviderKey) -> Option<&BundleScope> {
        self.scopes.get(&SyncScopeKey::Provider(provider.clone()))
    }

    pub fn provider_scopes(&self) -> impl Iterator<Item = (&ProviderKey, &BundleScope)> {
        self.scopes.iter().filter_map(|(key, scope)| match key {
            SyncScopeKey::Provider(provider) => Some((provider, scope)),
            SyncScopeKey::Project => None,
        })
    }

    pub fn all_resources(&self) -> impl Iterator<Item = (&ResourceId, &ResourceDescriptor)> {
        self.scopes
            .values()
            .flat_map(|scope| scope.resources.iter())
    }

    pub fn all_files(&self) -> impl Iterator<Item = (&LogicalPath, &BundleFileEntry)> {
        self.scopes.values().flat_map(|scope| scope.files.iter())
    }

    pub fn all_directories(&self) -> impl Iterator<Item = (&LogicalPath, &BundleDirectoryEntry)> {
        self.scopes
            .values()
            .flat_map(|scope| scope.directories.iter())
    }

    pub fn all_conversations(
        &self,
    ) -> impl Iterator<Item = (&SyncScopeKey, &CloudThreadId, &ConversationThread)> {
        self.scopes.iter().flat_map(|(key, scope)| {
            scope
                .conversations
                .iter()
                .map(move |(thread_id, thread)| (key, thread_id, thread))
        })
    }

    pub fn all_tombstones(&self) -> impl Iterator<Item = (&SyncScopeKey, &String, &Tombstone)> {
        self.scopes.iter().flat_map(|(key, scope)| {
            scope
                .tombstones
                .iter()
                .map(move |(tombstone_key, tombstone)| (key, tombstone_key, tombstone))
        })
    }

    pub fn find_conversation(
        &self,
        thread_id: &CloudThreadId,
    ) -> Option<(&SyncScopeKey, &ConversationThread)> {
        self.scopes.iter().find_map(|(key, scope)| {
            scope
                .conversations
                .get(thread_id)
                .map(|thread| (key, thread))
        })
    }

    pub fn find_resource(
        &self,
        resource_id: &ResourceId,
    ) -> Option<(&SyncScopeKey, &ResourceDescriptor)> {
        self.scopes.iter().find_map(|(key, scope)| {
            scope
                .resources
                .get(resource_id)
                .map(|descriptor| (key, descriptor))
        })
    }

    pub fn find_file(&self, path: &LogicalPath) -> Option<(&SyncScopeKey, &BundleFileEntry)> {
        self.scopes
            .iter()
            .find_map(|(key, scope)| scope.files.get(path).map(|entry| (key, entry)))
    }

    /// The scope coordinates the continuity and mutation-firewall layers
    /// compare: `(revision, content_sha256)` per present scope.
    pub fn scope_coordinates(&self) -> BTreeMap<SyncScopeKey, (u64, String)> {
        self.scopes
            .iter()
            .map(|(key, scope)| (key.clone(), (scope.revision, scope.content_sha256.clone())))
            .collect()
    }

    pub fn validate_against_head(&self, head: &BundleHead) -> Result<(), String> {
        self.validate()?;
        head.validate()?;
        if self.schema_version != head.schema_version
            || self.bundle.bundle_id != head.bundle_id
            || self.bundle.kind != head.kind
            || self.revision != head.revision
            || self.commit_id != head.commit_id
        {
            return Err("bundle head and manifest identity differ".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleSnapshot {
    pub storage_id: StorageId,
    pub head: BundleHead,
    pub manifest: BundleManifest,
    pub fetched_at: u64,
}

impl BundleSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        self.manifest.validate_against_head(&self.head)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreActionType {
    WriteFile,
    WriteProjectFile,
    EnsureProjectDirectory,
    DeleteProjectFile,
    DeleteProjectDirectory,
    MergeFile,
    MaterializeConversation,
    PublishConversation,
    Manual,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RestoreActionKind {
    WriteFile {
        logical_path: LogicalPath,
    },
    WriteProjectFile {
        logical_path: LogicalPath,
        mode: u32,
        source_mtime: u64,
    },
    EnsureProjectDirectory {
        logical_path: LogicalPath,
        mode: u32,
        source_mtime: u64,
    },
    DeleteProjectFile {
        logical_path: LogicalPath,
        last_sha256: String,
    },
    DeleteProjectDirectory {
        logical_path: LogicalPath,
    },
    MergeFile {
        logical_path: LogicalPath,
    },
    MaterializeConversation {
        provider: ProviderKey,
        logical_path: LogicalPath,
    },
    Manual {
        message: String,
    },
}

impl RestoreActionKind {
    pub fn action_type(&self) -> RestoreActionType {
        match self {
            Self::WriteFile { .. } => RestoreActionType::WriteFile,
            Self::WriteProjectFile { .. } => RestoreActionType::WriteProjectFile,
            Self::EnsureProjectDirectory { .. } => RestoreActionType::EnsureProjectDirectory,
            Self::DeleteProjectFile { .. } => RestoreActionType::DeleteProjectFile,
            Self::DeleteProjectDirectory { .. } => RestoreActionType::DeleteProjectDirectory,
            Self::MergeFile { .. } => RestoreActionType::MergeFile,
            Self::MaterializeConversation { .. } => RestoreActionType::MaterializeConversation,
            Self::Manual { .. } => RestoreActionType::Manual,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::WriteProjectFile { mode, .. } | Self::EnsureProjectDirectory { mode, .. } => {
                if mode & !0o777 != 0 {
                    Err("project-content mode contains unsafe bits".to_string())
                } else {
                    Ok(())
                }
            }
            Self::DeleteProjectFile { last_sha256, .. } => {
                validate_sha256("project-content deletion", last_sha256)
            }
            Self::Manual { message } => validate_display_text("manual action", message, false),
            _ => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RestoreAction {
    pub action_id: ActionId,
    pub resource_id: ResourceId,
    pub kind: RestoreActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_target_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_target_mode: Option<u32>,
    /// Digest of the exact bytes apply would write to `target_path`. When the
    /// live target already matches, apply can complete the action without
    /// downloading the payload. `None` means it was not derivable at plan
    /// time; apply falls back to fetching the object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialized_sha256: Option<String>,
    pub requires_explicit_approval: bool,
}

impl RestoreAction {
    pub fn validate(&self) -> Result<(), String> {
        self.kind.validate()?;
        if let Some(path) = &self.target_path {
            validate_absolute_clean_path("restore target", path)?;
        }
        if let Some(digest) = &self.source_sha256 {
            validate_sha256("restore source", digest)?;
        }
        if let Some(digest) = &self.expected_target_sha256 {
            validate_sha256("restore target", digest)?;
        }
        if let Some(digest) = &self.materialized_sha256 {
            validate_sha256("restore materialized", digest)?;
        }
        if self
            .expected_target_mode
            .is_some_and(|mode| mode & !0o777 != 0)
        {
            return Err("restore target mode contains unsafe bits".to_string());
        }
        if matches!(
            self.kind.action_type(),
            RestoreActionType::WriteProjectFile
                | RestoreActionType::EnsureProjectDirectory
                | RestoreActionType::DeleteProjectFile
                | RestoreActionType::DeleteProjectDirectory
        ) && !self.requires_explicit_approval
        {
            return Err(format!(
                "action '{}' requires explicit approval",
                self.action_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    pub schema_version: u32,
    pub plan_id: PlanId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub expected_host_identity_revision: u64,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub binding_revision: u64,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub actions: Vec<RestoreAction>,
    #[serde(default)]
    pub conversation_actions: Vec<ConversationPlanAction>,
    #[serde(default)]
    pub conversation_review: Vec<ConversationReviewRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_content_eligibility: Option<ProjectFileSyncEligibility>,
    /// Advisory repository-level continuity computed at plan time, so the
    /// pull review can explain a replaced history before anything applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_continuity: Option<RemoteContinuity>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectFileSyncEligibilityState {
    Eligible,
    GitManaged,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectFileSyncEligibility {
    pub state: ProjectFileSyncEligibilityState,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_root: Option<String>,
}

impl ProjectFileSyncEligibility {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("project-file eligibility reason", &self.reason, false)?;
        if let Some(root) = &self.detected_root {
            validate_absolute_clean_path("detected Git root", root)?;
        }
        Ok(())
    }
}

impl RestorePlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != RESTORE_PLAN_SCHEMA {
            return Err(format!(
                "unsupported restore-plan schema {}",
                self.schema_version
            ));
        }
        validate_named_id("commit id", &self.commit_id)?;
        validate_sha256("restore-plan manifest", &self.manifest_sha256)?;
        if self.created_at > self.expires_at
            || self.actions.len() > MAX_ACTIONS
            || self.conversation_actions.len() > MAX_ACTIONS
            || self.conversation_review.len() > MAX_ACTIONS
            || self
                .actions
                .len()
                .saturating_add(self.conversation_actions.len())
                > MAX_ACTIONS
        {
            return Err("invalid restore plan lifetime or action count".to_string());
        }
        let mut action_ids = HashSet::new();
        if let Some(eligibility) = &self.project_content_eligibility {
            eligibility.validate()?;
        }
        for action in &self.actions {
            action.validate()?;
            if !action_ids.insert(action.action_id.clone()) {
                return Err(format!("duplicate restore action '{}'", action.action_id));
            }
        }
        for action in &self.conversation_actions {
            action.validate()?;
            if !matches!(
                action.action,
                ConversationPlanActionType::AdoptCloudMetadata
                    | ConversationPlanActionType::MaterializeCloud
                    | ConversationPlanActionType::KeepBoth
                    | ConversationPlanActionType::CreateLocal
            ) || action.pins.context.host_id != self.host_id
                || action.pins.context.replica_id != self.replica_id
                || action.pins.revision.bundle_revision != self.revision
                || action.pins.revision.commit_id != self.commit_id
                || action.pins.revision.manifest_sha256 != self.manifest_sha256
                || action.pins.context.host_identity_revision
                    != self.expected_host_identity_revision
                || action.pins.expires_at != self.expires_at
            {
                return Err("conversation Pull action pins disagree with the plan".to_string());
            }
            if !action_ids.insert(action.action_id.clone()) {
                return Err(format!("duplicate restore action '{}'", action.action_id));
            }
        }
        let conversation_action_ids = self
            .conversation_actions
            .iter()
            .map(|action| &action.action_id)
            .collect::<BTreeSet<_>>();
        let mut reviewed_action_ids = BTreeSet::new();
        for row in &self.conversation_review {
            row.validate()?;
            if let Some(action_id) = &row.action_id {
                let action = self
                    .conversation_actions
                    .iter()
                    .find(|action| action.action_id == *action_id)
                    .ok_or_else(|| {
                        "conversation review row references an unknown action".to_string()
                    })?;
                if !reviewed_action_ids.insert(action_id) {
                    return Err(
                        "conversation review rows reference one action more than once".to_string(),
                    );
                }
                if row.action.planned_action() != Some(action.action)
                    || row.provider != action.provider
                    || row.thread_id != action.thread_id
                    || row.cloud_thread_id != action.cloud_thread_id
                    || action
                        .source_resource_id
                        .as_ref()
                        .is_some_and(|resource_id| resource_id != &row.resource_id)
                {
                    return Err(
                        "conversation review row disagrees with its executable action".to_string(),
                    );
                }
            }
        }
        if reviewed_action_ids != conversation_action_ids {
            return Err("conversation Pull action is missing its review row".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Applied,
    Skipped,
    Failed,
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationApplyReceipt {
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub thread_id: ThreadId,
    pub provider_profile_id: LocalProviderProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    pub materialization_state: ConversationMaterializationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_file_sha256_after: Option<String>,
    pub cloud_revision: u64,
    pub cloud_commit_id: String,
    pub cloud_manifest_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_origin_host_token: Option<OriginHostToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_origin_replica_token: Option<OriginReplicaToken>,
    pub journal_operation_id: ConversationOperationId,
    /// The accepted portable tail after this operation, when it moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_tail: Option<PortableConversationTail>,
    /// The minted logical thread that preserved the local branch of a
    /// divergent keep-both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_thread_id: Option<ThreadId>,
    /// The preserved copy's marker title, read from the journaled keep-both
    /// record (identities are minted at apply; plans carry none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_display_title: Option<String>,
    /// Recorded for every consumed provider source; old sources are never
    /// moved or deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_disposition: Option<ConversationSourceDisposition>,
}

/// The only source disposition this resolver records. Cleanup is a separate
/// future operation, so the vocabulary is deliberately closed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationSourceDisposition {
    Retained,
}

impl ConversationApplyReceipt {
    pub fn validate(&self) -> Result<(), String> {
        validate_named_id("conversation receipt commit", &self.cloud_commit_id)?;
        validate_sha256("conversation receipt manifest", &self.cloud_manifest_sha256)?;
        for (label, digest) in [
            (
                "conversation receipt lineage",
                self.lineage_digest.as_deref(),
            ),
            (
                "conversation receipt semantic",
                self.semantic_digest.as_deref(),
            ),
            (
                "conversation receipt local file",
                self.local_file_sha256_after.as_deref(),
            ),
        ] {
            if let Some(digest) = digest {
                validate_sha256(label, digest)?;
            }
        }
        if self.lineage_digest.is_some() != self.semantic_digest.is_some() {
            return Err("conversation receipt lineage and semantics must be paired".to_string());
        }
        let requires_provider = matches!(
            self.materialization_state,
            ConversationMaterializationState::Pending | ConversationMaterializationState::Resumable
        );
        if requires_provider != self.provider_thread_id.is_some() {
            return Err("conversation receipt has invalid provider materialization".to_string());
        }
        if self.provider_thread_id.as_ref().is_some_and(|thread_id| {
            thread_id.is_empty() || thread_id.len() > 128 || thread_id.chars().any(char::is_control)
        }) {
            return Err("conversation receipt has an invalid provider thread id".to_string());
        }
        if let Some(tail) = &self.accepted_tail {
            tail.validate()?;
        }
        if self.preserved_thread_id.as_ref() == Some(&self.thread_id) {
            return Err("conversation receipt cannot preserve into its own thread".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ApplyReceipt {
    pub action_id: ActionId,
    pub resource_id: ResourceId,
    pub action_type: RestoreActionType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_path: Option<LogicalPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_sha256_after: Option<String>,
    pub status: ActionStatus,
    pub applied_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ConversationApplyReceipt>,
}

impl ApplyReceipt {
    pub fn validate(&self) -> Result<(), String> {
        for (label, digest) in [
            ("receipt source", self.source_sha256.as_deref()),
            ("receipt target", self.target_sha256_after.as_deref()),
        ] {
            if let Some(digest) = digest {
                validate_sha256(label, digest)?;
            }
        }
        if let Some(path) = &self.target_path {
            validate_absolute_clean_path("receipt target", path)?;
        }
        if self.error.as_ref().is_some_and(|error| {
            error.len() > 8_192 || error.chars().any(|character| character == '\0')
        }) {
            return Err("invalid receipt error".to_string());
        }
        if let Some(conversation) = &self.conversation {
            conversation.validate()?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaterializationStatus {
    Partial,
    Complete,
    Detached,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MaterializationRecord {
    pub materialization_id: MaterializationId,
    pub plan_id: PlanId,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub binding_revision: u64,
    pub status: MaterializationStatus,
    pub applied_at: u64,
    #[serde(default)]
    pub receipts: Vec<ApplyReceipt>,
}

impl MaterializationRecord {
    pub fn validate(&self) -> Result<(), String> {
        validate_named_id("commit id", &self.commit_id)?;
        validate_sha256("materialization manifest", &self.manifest_sha256)?;
        if self.receipts.len() > MAX_ACTIONS {
            return Err("materialization has too many receipts".to_string());
        }
        let mut actions = HashSet::new();
        for receipt in &self.receipts {
            receipt.validate()?;
            if !actions.insert(receipt.action_id.clone()) {
                return Err(format!("duplicate apply receipt '{}'", receipt.action_id));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Materializations {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub records: Vec<MaterializationRecord>,
}

impl Default for Materializations {
    fn default() -> Self {
        Self {
            schema: LOCAL_SCHEMA,
            revision: 0,
            records: Vec::new(),
        }
    }
}

impl Materializations {
    pub fn validate(&self, config: &SyncConfig) -> Result<(), String> {
        if self.schema != LOCAL_SCHEMA || self.records.len() > 100_000 {
            return Err("invalid materializations document".to_string());
        }
        let mut ids = HashSet::new();
        for record in &self.records {
            record.validate()?;
            if !ids.insert(record.materialization_id.clone()) {
                return Err(format!(
                    "duplicate materialization id '{}'",
                    record.materialization_id
                ));
            }
            let project = config.project(&record.local_project_id);
            if record.status != MaterializationStatus::Detached && project.is_none() {
                return Err(format!(
                    "live materialization references unknown project '{}'",
                    record.local_project_id
                ));
            }
            if let Some(project) = project {
                if project.bundle_id != record.bundle_id {
                    return Err("materialization/project bundle mismatch".to_string());
                }
            }
        }
        Ok(())
    }
}

mod conversation_reconciliation;
mod conversation_tail;
mod conversation_tail_relation;

pub use conversation_reconciliation::{
    default_conversation_decision, BindingResolution, ClassifiedThread, ConversationCapabilities,
    ConversationDecision, ConversationOperationState, ReviewPurpose,
};
pub use conversation_tail::{
    compute_conversation_tail_state, compute_portable_conversation_tail,
    compute_portable_conversation_tail_with_codec, conversation_prefix_witness,
    AcceptedConversationVersion, ConversationCheckpoint, ConversationPrefixWitness,
    PhysicalMappingState, PortableConversationTail, RawRangePin, RootPhysicalMapping,
    RootTailLayout, TailAccumulator, TailPartitionKind, CLAUDE_PORTABLE_TAIL_CODEC_V1,
    CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1, PORTABLE_TAIL_CODEC_V1, PORTABLE_TAIL_CODEC_WIRE_V1,
};
pub use conversation_tail_relation::{
    compare_conversation_tail, SyncCurrency, TailClassification, TailEvidenceBlocker,
    TailSideEvidence, VerifiedTailEvidence,
};

#[cfg(test)]
mod tests;
