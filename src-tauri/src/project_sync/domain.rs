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
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
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
    generation: u64,
    commit_id: &str,
    manifest_sha256: &str,
) -> Result<(), String> {
    validate_named_id("bundle head pin commit", commit_id)?;
    validate_sha256("bundle head pin manifest", manifest_sha256)?;
    let is_absent = generation == 0;
    let uses_absent_values =
        commit_id == ABSENT_BUNDLE_COMMIT_ID && manifest_sha256.bytes().all(|byte| byte == b'0');
    if is_absent != uses_absent_values {
        return Err("bundle head pin has inconsistent absent coordinates".to_string());
    }
    Ok(())
}

macro_rules! named_id {
    ($name:ident, $label:literal) => {
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_named_id($label, &value)?;
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
named_id!(ThreadId, "thread id");
named_id!(PlanId, "plan id");
named_id!(ActionId, "action id");
named_id!(MaterializationId, "materialization id");
named_id!(ConversationOperationId, "conversation operation id");
named_id!(LocalProviderProfileId, "local provider profile id");
named_id!(SetupDraftId, "setup draft id");

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

fn validate_codex_thread_name(label: &str, value: &str) -> Result<(), String> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > 180
        || value.chars().any(char::is_control)
        || value.split_whitespace().collect::<Vec<_>>().join(" ") != value
    {
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Claude,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
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
    #[serde(default)]
    pub included_default_exclusions: Vec<String>,
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
        if self.included_default_exclusions.len() > 1_024 {
            return Err(format!("storage '{}' has too many exclusions", self.id));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    CodexConversation,
    ClaudeConversation,
    ProjectContentFile,
    ProjectContentDirectory,
    ProjectMemory,
    Requirement,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceScope {
    Project,
    ProviderState,
    Requirement,
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
    pub provider: Option<Provider>,
    pub scope: ResourceScope,
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

    pub fn selected_ids(&self) -> BTreeSet<ResourceId> {
        self.entries.keys().cloned().collect()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RecipeBase {
    pub generation: u64,
    pub manifest_sha256: String,
    /// Immutable commit that established this reviewed base. It lets status
    /// views reload the exact historical manifest after the storage head
    /// advances, and names the manifest the continuity probe checks for
    /// lineage membership — which is why it is required, not optional.
    pub commit_id: String,
    pub recipe_revision: u64,
    /// The machine binding that established this reviewed remote base. Older
    /// experimental bases deserialize as `None` and cannot authorize Push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pull_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_push_at: Option<u64>,
}

impl RecipeBase {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("recipe base manifest", &self.manifest_sha256)?;
        validate_named_id("recipe base commit", &self.commit_id)?;
        Ok(())
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
    RemoteMissing { base_generation: u64 },
    /// Remote history exists but this replica has never reviewed a base.
    Unreviewed { remote_generation: u64 },
    /// The head names exactly the manifest this replica last synced.
    InSync { generation: u64 },
    /// The recorded base is still part of the remote history; a pull
    /// fast-forwards onto it.
    Advanced {
        base_generation: u64,
        remote_generation: u64,
    },
    /// The remote no longer contains the recorded base: the history this
    /// replica synced was replaced, not extended.
    Replaced {
        base_generation: u64,
        remote_generation: u64,
    },
    /// The probe could not reach the store; no verdict is claimed.
    Unknown { message: String },
}

/// Classify remote history continuity from content witnesses alone.
///
/// The probe runs only in the generation-ahead arm and must read the store —
/// never a local cache, whose contents witness nothing about the remote —
/// reporting whether the base manifest is still present with exactly the
/// recorded digest. Every other row is decided for free: equal generations
/// with different digests cannot share a lineage (a lineage commits each
/// generation exactly once), and generations never regress within one.
pub fn classify_remote_continuity(
    observed_head: Option<&BundleHead>,
    base: Option<&RecipeBase>,
    probe: &mut dyn FnMut(&RecipeBase) -> Result<bool, String>,
) -> RemoteContinuity {
    match (observed_head, base) {
        (None, None) => RemoteContinuity::Fresh,
        (None, Some(base)) => RemoteContinuity::RemoteMissing {
            base_generation: base.generation,
        },
        (Some(head), None) => RemoteContinuity::Unreviewed {
            remote_generation: head.generation,
        },
        (Some(head), Some(base)) => {
            if head.generation == base.generation {
                if head.manifest_sha256 == base.manifest_sha256 {
                    RemoteContinuity::InSync {
                        generation: head.generation,
                    }
                } else {
                    RemoteContinuity::Replaced {
                        base_generation: base.generation,
                        remote_generation: head.generation,
                    }
                }
            } else if head.generation < base.generation {
                RemoteContinuity::Replaced {
                    base_generation: base.generation,
                    remote_generation: head.generation,
                }
            } else {
                match probe(base) {
                    Ok(true) => RemoteContinuity::Advanced {
                        base_generation: base.generation,
                        remote_generation: head.generation,
                    },
                    Ok(false) => RemoteContinuity::Replaced {
                        base_generation: base.generation,
                        remote_generation: head.generation,
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
    #[serde(default)]
    pub recipe_bases: BTreeMap<StorageId, RecipeBase>,
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
        self.recipe.validate()?;
        for base in self.recipe_bases.values() {
            base.validate()?;
        }
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
    /// Machine-local, destination-specific decisions for generic project
    /// content. These preferences are never serialized into a remote bundle.
    #[serde(default)]
    pub project_content_preferences: ProjectContentPreferences,
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
    if descriptor.scope != ResourceScope::Project
        || descriptor.apply_policy != ApplyPolicy::ExplicitReview
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
                recipe.validate()?;
            }
            link.project_content_preferences.validate()?;
            if !cells.insert((link.local_project_id.clone(), link.storage_id.clone())) {
                return Err(format!(
                    "duplicate project/storage link '{}'/ '{}'",
                    link.local_project_id, link.storage_id
                ));
            }
        }
        for project in &self.projects {
            for storage_id in project.recipe_bases.keys() {
                if !storage_ids.contains(storage_id) {
                    return Err(format!(
                        "project '{}' has a recipe base for unknown storage '{}'",
                        project.local_project_id, storage_id
                    ));
                }
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
    pub provider: Provider,
    pub display_name: String,
    /// The exact CODEX_HOME or CLAUDE_CONFIG_DIR spelling selected locally.
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
    #[serde(default)]
    pub profile_ids: BTreeMap<Provider, LocalProviderProfileId>,
    /// Runtime-only resolved provider homes. Machine state persists profile
    /// IDs; commands populate these fields after revalidating the catalog.
    #[serde(skip)]
    pub codex_home: Option<String>,
    #[serde(skip)]
    pub claude_home: Option<String>,
    pub state: BindingState,
    pub revision: u64,
    pub updated_at: u64,
}

impl ProjectBinding {
    pub fn validate_structure(&self) -> Result<(), String> {
        validate_absolute_clean_path("project root", &self.project_root)?;
        validate_absolute_clean_path("canonical project root", &self.canonical_project_root)?;
        if let Some(path) = &self.codex_home {
            validate_absolute_clean_path("Codex home", path)?;
        }
        if let Some(path) = &self.claude_home {
            validate_absolute_clean_path("Claude home", path)?;
        }
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

/// Witness that this thread's branch was preserved by a Mallard-driven fork:
/// the source chain's digest pair at fork time plus the adopted fork's
/// identity. Written only after fork verification, overwritten by the next
/// adoption, never cleared — it is consulted solely under a `Diverged`
/// classification, where digest inequality self-invalidates a stale stamp.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ForkStamp {
    pub fork_thread_id: ThreadId,
    pub fork_provider_thread_id: String,
    pub lineage_digest: String,
    pub semantic_digest: String,
    pub message_count: u64,
    pub forked_at: u64,
}

impl ForkStamp {
    fn validate(&self, source_thread_id: &ThreadId) -> Result<(), String> {
        if &self.fork_thread_id == source_thread_id {
            return Err("conversation fork stamp cannot name its own thread".to_string());
        }
        validate_provider_thread_id(
            "conversation fork stamp provider thread",
            &self.fork_provider_thread_id,
        )?;
        validate_sha256("conversation fork stamp lineage", &self.lineage_digest)?;
        validate_sha256("conversation fork stamp semantics", &self.semantic_digest)?;
        if self.message_count == 0 {
            return Err("conversation fork stamp requires a nonzero message count".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalConversationBinding {
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub thread_id: ThreadId,
    /// Immediate portable parent for a locally preserved conflict branch.
    /// This survives a Pull fork until a later Push publishes the branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_stamp: Option<ForkStamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    pub materialization_state: ConversationMaterializationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_file_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_head_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_lineage_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_semantic_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_manifest_sha256: Option<String>,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl LocalConversationBinding {
    pub fn validate(&self) -> Result<(), String> {
        if self.forked_from.as_ref() == Some(&self.thread_id) {
            return Err(format!(
                "local conversation thread '{}' cannot fork from itself",
                self.thread_id
            ));
        }
        if let Some(stamp) = &self.fork_stamp {
            stamp.validate(&self.thread_id)?;
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
        if self
            .last_observed_head_message_id
            .as_ref()
            .is_some_and(|message_id| {
                message_id.is_empty()
                    || message_id.len() > 256
                    || message_id.chars().any(char::is_control)
            })
        {
            return Err("conversation binding has an invalid head message ID".to_string());
        }
        for (label, digest) in [
            (
                "conversation local file",
                self.last_observed_file_sha256.as_deref(),
            ),
            (
                "conversation lineage",
                self.last_observed_lineage_digest.as_deref(),
            ),
            (
                "conversation semantic",
                self.last_observed_semantic_digest.as_deref(),
            ),
            (
                "conversation source manifest",
                self.source_manifest_sha256.as_deref(),
            ),
        ] {
            if let Some(digest) = digest {
                validate_sha256(label, digest)?;
            }
        }
        if self.source_generation.is_some() != self.source_manifest_sha256.is_some() {
            return Err(
                "conversation binding source generation and manifest must be pinned together"
                    .to_string(),
            );
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

    pub fn validate(
        &self,
        identity: &HostIdentity,
        machine: &MachineProjectState,
        config: &SyncConfig,
    ) -> Result<(), String> {
        if self.schema != CONVERSATION_BINDINGS_SCHEMA
            || self.bindings.len() > MAX_CONVERSATION_BINDINGS
            || self.origin_aliases.len() > MAX_ORIGIN_ALIASES
        {
            return Err("invalid conversation bindings document".to_string());
        }
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
        let mut binding_keys = HashSet::new();
        let mut provider_owners = HashSet::new();
        for binding in &self.bindings {
            binding.validate()?;
            if binding.host_id != self.host_id {
                return Err("conversation binding belongs to a different host".to_string());
            }
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
}

pub fn fallback_origin_host_label(token: &OriginHostToken) -> String {
    format!("Host {}", token.as_str()[..4].to_ascii_uppercase())
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationSyncPhase {
    Prepared,
    ReplacementStaged,
    ForkAttemptPending,
    AwaitingManualFork,
    ForkVerified,
    ForkNamingRecorded,
    LocalForkBound,
    ProviderCommitted,
    BindingsCommitted,
    OriginalNamingRecorded,
    CloudPublished,
    LocalCommitted,
    RecoveryRequired,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexForkAttemptStatus {
    Pending,
    DefinitiveNoFork,
    Verified,
    Ambiguous,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexForkAttempt {
    pub executor: CodexForkExecutor,
    pub status: CodexForkAttemptStatus,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationNameStatus {
    Applied,
    UserOverride,
    Unchanged,
    Deferred,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexInventoryRefreshStatus {
    NotAttempted,
    Deferred,
    Refreshed,
    Failed,
}

/// Which entry point created a fork-carrying divergence operation. The
/// executor ladder, verification, and checkpoints are identical for both;
/// origin decides what follows fork adoption — a Pull restores the cloud
/// branch into the source, a manual fork stops with the source untouched.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForkOrigin {
    PullPreserve,
    Manual,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexDivergenceOperation {
    pub origin: ForkOrigin,
    pub resolution: ConversationDivergenceResolution,
    pub available_resolutions: Vec<ConversationDivergenceResolution>,
    pub recommended_resolution: ConversationDivergenceResolution,
    pub local_branch_preserved: bool,
    pub preserved_thread_id: ThreadId,
    pub source_provider_thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_provider_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_transcript_sha256: Option<String>,
    pub local_lineage_digest: String,
    pub local_semantic_digest: String,
    pub local_message_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_local_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_cloud_title: Option<String>,
    pub desired_fork_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_original_name: Option<String>,
    pub codex_executable: String,
    pub codex_executable_sha256: String,
    pub available_executors: Vec<CodexForkExecutor>,
    pub pre_existing_children: Vec<CodexNativeChildPin>,
    pub attempts: Vec<CodexForkAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_executor: Option<CodexForkExecutor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successful_executor: Option<CodexForkExecutor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_name_status: Option<ConversationNameStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name_status: Option<ConversationNameStatus>,
    pub inventory_refresh_status: CodexInventoryRefreshStatus,
}

impl CodexDivergenceOperation {
    fn validate(
        &self,
        source_thread_id: &ThreadId,
        phase: ConversationSyncPhase,
    ) -> Result<(), String> {
        let plan = ConversationDivergencePlan {
            available_resolutions: self.available_resolutions.clone(),
            recommended_resolution: self.recommended_resolution,
            local_branch_preserved: self.local_branch_preserved,
            preserved_thread_id: self.preserved_thread_id.clone(),
            source_provider_thread_id: self.source_provider_thread_id.clone(),
            local_lineage_digest: self.local_lineage_digest.clone(),
            local_semantic_digest: self.local_semantic_digest.clone(),
            local_message_count: self.local_message_count,
            pinned_local_title: self.pinned_local_title.clone(),
            pinned_cloud_title: self.pinned_cloud_title.clone(),
            desired_fork_name: self.desired_fork_name.clone(),
            desired_original_name: self.desired_original_name.clone(),
            codex_executable: self.codex_executable.clone(),
            codex_executable_sha256: self.codex_executable_sha256.clone(),
            available_executors: self.available_executors.clone(),
            pre_existing_children: self.pre_existing_children.clone(),
        };
        plan.validate(source_thread_id)?;
        if self.origin == ForkOrigin::Manual {
            if self.resolution != ConversationDivergenceResolution::PreserveLocalWithNativeFork {
                return Err("manual fork operation must preserve the local branch".to_string());
            }
            if matches!(
                phase,
                ConversationSyncPhase::ProviderCommitted | ConversationSyncPhase::BindingsCommitted
            ) {
                return Err("manual fork operation has no provider or binding commit".to_string());
            }
            if self.desired_original_name.is_some() {
                return Err("manual fork operation cannot rename the source thread".to_string());
            }
        }
        if self.resolution == ConversationDivergenceResolution::UseCloudOnly
            && (self.fork_provider_thread_id.is_some()
                || self.fork_transcript_path.is_some()
                || self.fork_transcript_sha256.is_some()
                || !self.attempts.is_empty()
                || self.current_executor.is_some()
                || self.successful_executor.is_some()
                || self.fork_name_status.is_some())
        {
            return Err("cloud-only divergence operation carries native-fork state".to_string());
        }
        if self.fork_provider_thread_id.is_some() != self.fork_transcript_path.is_some()
            || self.fork_provider_thread_id.is_some() != self.fork_transcript_sha256.is_some()
        {
            return Err(
                "Codex divergence fork identity and transcript pins must be paired".to_string(),
            );
        }
        if let Some(thread_id) = &self.fork_provider_thread_id {
            validate_provider_thread_id("Codex divergence fork provider thread", thread_id)?;
            if thread_id == &self.source_provider_thread_id {
                return Err("Codex divergence fork reuses the source provider thread".to_string());
            }
        }
        if let Some(path) = &self.fork_transcript_path {
            validate_absolute_clean_path("Codex divergence fork transcript", path)?;
        }
        if let Some(digest) = &self.fork_transcript_sha256 {
            validate_sha256("Codex divergence fork transcript", digest)?;
        }
        if self.attempts.len() > 3 {
            return Err("Codex divergence has too many executor attempts".to_string());
        }
        for (index, attempt) in self.attempts.iter().enumerate() {
            if self.available_executors.get(index) != Some(&attempt.executor)
                || (index + 1 < self.attempts.len()
                    && attempt.status != CodexForkAttemptStatus::DefinitiveNoFork)
            {
                return Err(
                    "Codex divergence executor attempts are not an ordered prefix".to_string(),
                );
            }
        }
        let expected_current = self.attempts.last().and_then(|attempt| {
            matches!(
                attempt.status,
                CodexForkAttemptStatus::Pending | CodexForkAttemptStatus::Ambiguous
            )
            .then_some(attempt.executor)
        });
        let expected_success = self.attempts.last().and_then(|attempt| {
            (attempt.status == CodexForkAttemptStatus::Verified).then_some(attempt.executor)
        });
        if self.current_executor != expected_current || self.successful_executor != expected_success
        {
            return Err("Codex divergence executor outcome is inconsistent".to_string());
        }
        let has_verified_fork = self.fork_provider_thread_id.is_some();
        if has_verified_fork != self.successful_executor.is_some()
            || has_verified_fork != self.fork_name_status.is_some()
        {
            return Err("Codex divergence verified fork state is incomplete".to_string());
        }
        match self.resolution {
            ConversationDivergenceResolution::UseCloudOnly => {
                if !matches!(
                    phase,
                    ConversationSyncPhase::Prepared
                        | ConversationSyncPhase::ReplacementStaged
                        | ConversationSyncPhase::ProviderCommitted
                        | ConversationSyncPhase::BindingsCommitted
                        | ConversationSyncPhase::OriginalNamingRecorded
                        | ConversationSyncPhase::LocalCommitted
                        | ConversationSyncPhase::RecoveryRequired
                ) {
                    return Err("cloud-only divergence has an invalid phase".to_string());
                }
            }
            ConversationDivergenceResolution::PreserveLocalWithNativeFork => {
                if matches!(phase, ConversationSyncPhase::CloudPublished) {
                    return Err("preserved divergence has an invalid phase".to_string());
                }
                let phase_requires_verified_fork = matches!(
                    phase,
                    ConversationSyncPhase::ForkVerified
                        | ConversationSyncPhase::ForkNamingRecorded
                        | ConversationSyncPhase::LocalForkBound
                        | ConversationSyncPhase::ProviderCommitted
                        | ConversationSyncPhase::BindingsCommitted
                        | ConversationSyncPhase::OriginalNamingRecorded
                        | ConversationSyncPhase::LocalCommitted
                );
                if phase_requires_verified_fork != has_verified_fork {
                    return Err(
                        "Codex divergence phase disagrees with verified fork state".to_string()
                    );
                }
                if phase == ConversationSyncPhase::AwaitingManualFork
                    && (self.current_executor != Some(CodexForkExecutor::Manual)
                        || self.attempts.last().is_none_or(|attempt| {
                            attempt.executor != CodexForkExecutor::Manual
                                || attempt.status != CodexForkAttemptStatus::Pending
                        }))
                {
                    return Err("manual fork checkpoint has invalid executor state".to_string());
                }
                if matches!(
                    phase,
                    ConversationSyncPhase::Prepared | ConversationSyncPhase::ReplacementStaged
                ) && (!self.attempts.is_empty()
                    || self.current_executor.is_some()
                    || self.successful_executor.is_some())
                {
                    return Err(
                        "Codex divergence has executor state before fork execution".to_string()
                    );
                }
                if phase == ConversationSyncPhase::RecoveryRequired
                    && !has_verified_fork
                    && self
                        .attempts
                        .last()
                        .is_none_or(|attempt| attempt.status != CodexForkAttemptStatus::Ambiguous)
                {
                    return Err(
                        "pre-verification Codex recovery lacks an ambiguous executor".to_string(),
                    );
                }
            }
        }
        let original_name_may_be_recorded = matches!(
            phase,
            ConversationSyncPhase::BindingsCommitted
                | ConversationSyncPhase::OriginalNamingRecorded
                | ConversationSyncPhase::LocalCommitted
                | ConversationSyncPhase::RecoveryRequired
        );
        if self.original_name_status.is_some() && !original_name_may_be_recorded {
            return Err("Codex original name was recorded before provider commit".to_string());
        }
        if matches!(
            phase,
            ConversationSyncPhase::OriginalNamingRecorded | ConversationSyncPhase::LocalCommitted
        ) && self.original_name_status.is_none()
        {
            return Err("Codex original naming phase lacks a recorded result".to_string());
        }
        let refresh_must_be_recorded = matches!(
            phase,
            ConversationSyncPhase::OriginalNamingRecorded | ConversationSyncPhase::LocalCommitted
        );
        if refresh_must_be_recorded
            != (self.inventory_refresh_status != CodexInventoryRefreshStatus::NotAttempted)
        {
            return Err("Codex inventory refresh status disagrees with its phase".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", content = "details", rename_all = "snake_case")]
pub enum ConversationSyncOperationKind {
    CloudPublication,
    ProviderMaterialization,
    CodexDivergence(CodexDivergenceOperation),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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
        self.role.validate()?;
        Ok(())
    }
}

/// The cloud commit a sync operation belongs to. Present on every push and
/// pull operation; absent only on a manual-fork operation, which touches no
/// storage and must not fabricate one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationCloudAnchor {
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub generation: u64,
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
pub struct ConversationSyncOperation {
    pub operation_id: ConversationOperationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<PlanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_action_id: Option<ActionId>,
    pub direction: ConversationPlanDirection,
    pub phase: ConversationSyncPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<ConversationCloudAnchor>,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub provider: Provider,
    pub thread_id: ThreadId,
    pub operation: ConversationSyncOperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_forked_from: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    pub materialization_state: ConversationMaterializationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_digest: Option<String>,
    pub expected_host_identity_revision: u64,
    pub expected_bindings_document_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_binding_revision: Option<u64>,
    pub expected_profile_revision: u64,
    pub expected_project_binding_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_local_digest: Option<String>,
    #[serde(default)]
    pub targets: Vec<ConversationJournalTarget>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl ConversationSyncOperation {
    pub fn binding_target_thread_id(&self) -> &ThreadId {
        &self.thread_id
    }

    pub fn divergence(&self) -> Option<&CodexDivergenceOperation> {
        match &self.operation {
            ConversationSyncOperationKind::CodexDivergence(divergence) => Some(divergence),
            _ => None,
        }
    }

    pub fn fork_origin(&self) -> Option<ForkOrigin> {
        self.divergence().map(|divergence| divergence.origin)
    }

    pub fn cloud_anchor(&self) -> Result<&ConversationCloudAnchor, String> {
        self.cloud
            .as_ref()
            .ok_or_else(|| "conversation operation lacks a cloud anchor".to_string())
    }
}

impl ConversationSyncOperation {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(anchor) = &self.cloud {
            anchor.validate()?;
        }
        if self.cloud.is_none() && self.fork_origin() != Some(ForkOrigin::Manual) {
            return Err("only a manual fork operation may omit its cloud anchor".to_string());
        }
        if self.plan_action_id.is_some() && self.plan_id.is_none() {
            return Err("conversation journal action provenance requires a plan id".to_string());
        }
        if let Some(digest) = &self.expected_local_digest {
            validate_sha256("conversation journal local target", digest)?;
        }
        for (label, digest) in [
            (
                "conversation journal lineage",
                self.lineage_digest.as_deref(),
            ),
            (
                "conversation journal semantic",
                self.semantic_digest.as_deref(),
            ),
        ] {
            if let Some(digest) = digest {
                validate_sha256(label, digest)?;
            }
        }
        if self.lineage_digest.is_some() != self.semantic_digest.is_some() {
            return Err("conversation journal lineage and semantics must be paired".to_string());
        }
        if self.binding_forked_from.as_ref() == Some(self.binding_target_thread_id()) {
            return Err("conversation journal binding cannot parent itself".to_string());
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
            ConversationSyncOperationKind::CloudPublication => {
                if self.direction != ConversationPlanDirection::Push
                    || !self.targets.is_empty()
                    || !matches!(
                        self.phase,
                        ConversationSyncPhase::Prepared
                            | ConversationSyncPhase::CloudPublished
                            | ConversationSyncPhase::BindingsCommitted
                            | ConversationSyncPhase::LocalCommitted
                            | ConversationSyncPhase::RecoveryRequired
                    )
                {
                    return Err("invalid conversation cloud-publication journal".to_string());
                }
            }
            ConversationSyncOperationKind::ProviderMaterialization => {
                if self.direction != ConversationPlanDirection::Pull
                    || !matches!(
                        self.phase,
                        ConversationSyncPhase::Prepared
                            | ConversationSyncPhase::ReplacementStaged
                            | ConversationSyncPhase::ProviderCommitted
                            | ConversationSyncPhase::BindingsCommitted
                            | ConversationSyncPhase::LocalCommitted
                            | ConversationSyncPhase::RecoveryRequired
                    )
                {
                    return Err("invalid conversation materialization journal".to_string());
                }
            }
            ConversationSyncOperationKind::CodexDivergence(divergence) => {
                if self.direction != ConversationPlanDirection::Pull
                    || self.provider != Provider::Codex
                {
                    return Err("invalid Codex divergence journal scope".to_string());
                }
                match divergence.origin {
                    ForkOrigin::PullPreserve => {
                        if self.cloud.is_none() {
                            return Err(
                                "pull divergence operation requires a cloud anchor".to_string()
                            );
                        }
                    }
                    ForkOrigin::Manual => {
                        if self.cloud.is_some() || !self.targets.is_empty() {
                            return Err(
                                "manual fork operation carries no cloud anchor or targets"
                                    .to_string(),
                            );
                        }
                    }
                }
                divergence.validate(&self.thread_id, self.phase)?;
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
        for operation in &self.operations {
            operation.validate()?;
            if !ids.insert(operation.operation_id.clone()) {
                return Err(format!(
                    "duplicate conversation journal operation '{}'",
                    operation.operation_id
                ));
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

/// A resumable, machine-local project setup draft.  Drafts hold selections
/// and preallocated identities only — never discovered file contents, remote
/// listings, or resource payloads; those are rescanned on resume.
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
    #[serde(default)]
    pub profiles: BTreeMap<Provider, DraftProfileSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<DraftStorageSelection>,
    pub repository: DraftRepositoryChoice,
    /// Resource IDs checked in the setup inventory.
    #[serde(default)]
    pub selected_resource_ids: Vec<ResourceId>,
    /// Digest over the discovered candidate IDs at selection time.  A changed
    /// signature flags the saved selection for re-review, never silent reuse.
    #[serde(default)]
    pub discovery_signature: String,
    /// Digest over the provider home paths the discovery signature was taken
    /// against.  Repointing a profile changes what is discoverable at all, so
    /// that is a re-derive, not the drift `discovery_signature` reports.
    #[serde(default)]
    pub discovery_profile_signature: String,
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
        if self.selected_resource_ids.len() > MAX_RESOURCES {
            return Err(format!("draft exceeds {} resources", MAX_RESOURCES));
        }
        if !self.discovery_signature.is_empty() {
            validate_sha256("draft discovery signature", &self.discovery_signature)?;
        }
        if !self.discovery_profile_signature.is_empty() {
            validate_sha256(
                "draft discovery profile signature",
                &self.discovery_profile_signature,
            )?;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_version: Option<String>,
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationThread {
    pub thread_id: ThreadId,
    pub provider: Provider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
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
    /// Provenance of a fork only. Never consulted by the resolver, and the
    /// referenced thread may have been tombstoned or pruned since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ThreadId>,
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
        validate_display_text("conversation lineage codec", &self.lineage_codec, false)?;
        let expected_resource_id = conversation_resource_id(self.provider, &self.thread_id)?;
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
        let canonical_transcript = conversation_transcript_path(self.provider, &self.thread_id)?;
        let canonical_prefix = format!(
            "state/conversations/{}/{}/",
            self.provider.as_str(),
            self.thread_id
        );
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
        if self.forked_from.as_ref() == Some(&self.thread_id) {
            return Err(format!(
                "conversation thread '{}' cannot fork from itself",
                self.thread_id
            ));
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
pub enum ConversationRelation {
    Identical,
    CloudPrefixOfLocal,
    LocalPrefixOfCloud,
    Diverged,
    Unknown,
    LocalMissing,
    CloudMissing,
    BothMissing,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanDirection {
    Push,
    Pull,
}

/// Typed reconciliation outcomes. Push and Pull intentionally share one enum
/// so every relation can be represented in review DTOs without interpreting a
/// localized message. Divergence is directional: Push requires Pull, while
/// Pull offers one typed resolution action with two reviewed outcomes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanActionType {
    NoOp,
    FastForwardLocal,
    FastForwardCloud,
    KeepLocal,
    RequirePull,
    ResolveDivergence,
    CreateCloud,
    CreateLocal,
    StoreOnly,
    ReplaceLocal,
    ReplaceCloud,
    TombstoneThread,
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationDivergenceResolution {
    PreserveLocalWithNativeFork,
    UseCloudOnly,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationResolutionSelection {
    pub action_id: ActionId,
    pub resolution: ConversationDivergenceResolution,
    pub destructive_approved: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CodexForkExecutor {
    AppServerStdio,
    NativePty,
    Manual,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexNativeChildPin {
    pub provider_thread_id: String,
    pub transcript_sha256: String,
}

impl CodexNativeChildPin {
    fn validate(&self) -> Result<(), String> {
        validate_provider_thread_id(
            "Codex native child provider thread",
            &self.provider_thread_id,
        )?;
        validate_sha256("Codex native child transcript", &self.transcript_sha256)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationDivergencePlan {
    pub available_resolutions: Vec<ConversationDivergenceResolution>,
    /// Advisory review default. `UseCloudOnly` exactly when
    /// `local_branch_preserved` witnessed at plan time; apply re-proves the
    /// witness before any waived-confirmation overwrite.
    pub recommended_resolution: ConversationDivergenceResolution,
    pub local_branch_preserved: bool,
    pub preserved_thread_id: ThreadId,
    pub source_provider_thread_id: String,
    pub local_lineage_digest: String,
    pub local_semantic_digest: String,
    pub local_message_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_local_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_cloud_title: Option<String>,
    pub desired_fork_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_original_name: Option<String>,
    pub codex_executable: String,
    pub codex_executable_sha256: String,
    pub available_executors: Vec<CodexForkExecutor>,
    pub pre_existing_children: Vec<CodexNativeChildPin>,
}

impl ConversationDivergencePlan {
    fn validate(&self, source_thread_id: &ThreadId) -> Result<(), String> {
        if self.available_resolutions
            != [
                ConversationDivergenceResolution::PreserveLocalWithNativeFork,
                ConversationDivergenceResolution::UseCloudOnly,
            ]
        {
            return Err(
                "conversation divergence must advertise exactly keep-both and cloud-only"
                    .to_string(),
            );
        }
        let expected_recommended = if self.local_branch_preserved {
            ConversationDivergenceResolution::UseCloudOnly
        } else {
            ConversationDivergenceResolution::PreserveLocalWithNativeFork
        };
        if self.recommended_resolution != expected_recommended {
            return Err(
                "conversation divergence recommendation disagrees with its preservation witness"
                    .to_string(),
            );
        }
        if &self.preserved_thread_id == source_thread_id {
            return Err(
                "conversation divergence must mint a distinct preserved thread".to_string(),
            );
        }
        validate_provider_thread_id(
            "Codex divergence source provider thread",
            &self.source_provider_thread_id,
        )?;
        validate_sha256(
            "conversation divergence local lineage",
            &self.local_lineage_digest,
        )?;
        if self.local_message_count == 0 {
            return Err("conversation divergence requires a nonzero local chain".to_string());
        }
        validate_sha256(
            "conversation divergence local semantics",
            &self.local_semantic_digest,
        )?;
        for (label, title) in [
            (
                "conversation divergence local title",
                self.pinned_local_title.as_deref(),
            ),
            (
                "conversation divergence cloud title",
                self.pinned_cloud_title.as_deref(),
            ),
            (
                "conversation divergence original title",
                self.desired_original_name.as_deref(),
            ),
        ] {
            if let Some(title) = title {
                validate_codex_thread_name(label, title)?;
            }
        }
        validate_codex_thread_name(
            "conversation divergence preserved title",
            &self.desired_fork_name,
        )?;
        validate_absolute_clean_path(
            "conversation divergence Codex executable",
            &self.codex_executable,
        )?;
        validate_sha256(
            "conversation divergence Codex executable",
            &self.codex_executable_sha256,
        )?;
        let expected = [
            CodexForkExecutor::AppServerStdio,
            CodexForkExecutor::NativePty,
            CodexForkExecutor::Manual,
        ];
        if self.available_executors != expected {
            return Err(
                "conversation divergence executors must be app-server, native PTY, then manual"
                    .to_string(),
            );
        }
        if self.pre_existing_children.len() > 1_024 {
            return Err("conversation divergence has too many existing children".to_string());
        }
        let mut child_ids = BTreeSet::new();
        for child in &self.pre_existing_children {
            child.validate()?;
            if child.provider_thread_id == self.source_provider_thread_id
                || !child_ids.insert(child.provider_thread_id.clone())
            {
                return Err("conversation divergence has invalid existing child pins".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationReasonCode {
    Identical,
    StructuralRewriteOnly,
    CloudPrefixOfLocal,
    LocalPrefixOfCloud,
    Diverged,
    UnknownLineage,
    LocalMissing,
    CloudMissing,
    BothMissing,
    ProviderNotQuiescent,
    CodecUnsupported,
    AmbiguousBinding,
    MissingBinding,
    TombstoneUnchanged,
    TombstoneModified,
    StalePlan,
    ResolutionPending,
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
pub struct ConversationPlanAction {
    pub action_id: ActionId,
    pub direction: ConversationPlanDirection,
    pub action: ConversationPlanActionType,
    pub reason_code: ConversationReasonCode,
    pub relation: ConversationRelation,
    pub provider: Provider,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divergence: Option<ConversationDivergencePlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_resource_id: Option<ResourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_provider_thread_id: Option<String>,
    pub required_codec: String,
    pub pinned_generation: u64,
    pub pinned_commit_id: String,
    pub pinned_manifest_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_lineage_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_semantic_digest: Option<String>,
    pub expected_local_target: TargetExpectation,
    #[serde(default)]
    pub expected_provider_state: Vec<ProviderStateExpectation>,
    pub expected_profile_revision: u64,
    pub expected_project_binding_revision: u64,
    pub expected_host_identity_revision: u64,
    pub expected_bindings_document_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_binding_revision: Option<u64>,
    pub expires_at: u64,
}

impl ConversationPlanAction {
    pub fn validate(&self) -> Result<(), String> {
        validate_bundle_head_pin(
            self.pinned_generation,
            &self.pinned_commit_id,
            &self.pinned_manifest_sha256,
        )?;
        validate_display_text("conversation plan codec", &self.required_codec, false)?;
        for (label, digest) in [
            (
                "conversation plan lineage",
                self.pinned_lineage_digest.as_deref(),
            ),
            (
                "conversation plan semantic",
                self.pinned_semantic_digest.as_deref(),
            ),
        ] {
            if let Some(digest) = digest {
                validate_sha256(label, digest)?;
            }
        }
        if self.pinned_lineage_digest.is_some() != self.pinned_semantic_digest.is_some() {
            return Err("conversation plan must pin lineage and semantics together".to_string());
        }
        self.expected_local_target.validate()?;
        if self.expected_provider_state.len() > 64 {
            return Err("conversation plan has too many provider targets".to_string());
        }
        let mut paths = BTreeSet::new();
        for target in &self.expected_provider_state {
            target.validate()?;
            if !paths.insert(target.target_path.to_lowercase()) {
                return Err("conversation plan repeats a provider target".to_string());
            }
        }
        if self
            .proposed_provider_thread_id
            .as_ref()
            .is_some_and(|thread_id| {
                validate_provider_thread_id("conversation plan provider thread", thread_id).is_err()
            })
        {
            return Err("conversation plan has an invalid provider thread id".to_string());
        }
        if (self.direction == ConversationPlanDirection::Pull
            && matches!(
                self.action,
                ConversationPlanActionType::FastForwardLocal
                    | ConversationPlanActionType::CreateLocal
                    | ConversationPlanActionType::ReplaceLocal
            ))
            && self.proposed_provider_thread_id.is_none()
        {
            return Err(
                "conversation materialization action lacks a provider thread id".to_string(),
            );
        }
        if self.direction == ConversationPlanDirection::Push
            && matches!(
                self.action,
                ConversationPlanActionType::FastForwardLocal
                    | ConversationPlanActionType::StoreOnly
                    | ConversationPlanActionType::CreateLocal
                    | ConversationPlanActionType::KeepLocal
                    | ConversationPlanActionType::ReplaceLocal
            )
        {
            return Err("conversation Push plan contains a Pull-only action".to_string());
        }
        if self.direction == ConversationPlanDirection::Pull
            && matches!(
                self.action,
                ConversationPlanActionType::FastForwardCloud
                    | ConversationPlanActionType::RequirePull
                    | ConversationPlanActionType::CreateCloud
                    | ConversationPlanActionType::ReplaceCloud
                    | ConversationPlanActionType::TombstoneThread
            )
        {
            return Err("conversation Pull plan contains a Push-only action".to_string());
        }
        if self.direction == ConversationPlanDirection::Push
            && self.action == ConversationPlanActionType::ResolveDivergence
        {
            return Err("conversation Push plan contains a Pull-only action".to_string());
        }
        match (&self.action, &self.divergence) {
            (ConversationPlanActionType::ResolveDivergence, Some(divergence)) => {
                if self.provider != Provider::Codex
                    || self.relation != ConversationRelation::Diverged
                    || self.proposed_provider_thread_id.is_some()
                {
                    return Err(
                        "conversation divergence plan has an invalid action scope".to_string()
                    );
                }
                divergence.validate(&self.thread_id)?;
            }
            (ConversationPlanActionType::ResolveDivergence, None) => {
                return Err("conversation divergence action lacks its reviewed plan".to_string());
            }
            (_, Some(_)) => {
                return Err(
                    "non-divergence conversation action carries divergence state".to_string(),
                );
            }
            (_, None) => {}
        }
        if self.action == ConversationPlanActionType::NoOp
            && (self.proposed_provider_thread_id.is_some()
                || !self.expected_provider_state.is_empty())
        {
            return Err("conversation no-op reserves mutation targets".to_string());
        }
        if matches!(
            self.action,
            ConversationPlanActionType::KeepLocal
                | ConversationPlanActionType::RequirePull
                | ConversationPlanActionType::Blocked
        ) && (self.proposed_provider_thread_id.is_some()
            || !self.expected_provider_state.is_empty())
        {
            return Err("informational conversation action reserves mutation targets".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationPushPlan {
    pub schema_version: u32,
    pub plan_id: PlanId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub expected_host_identity_revision: u64,
    pub expected_project_binding_revision: u64,
    pub expected_bindings_document_revision: u64,
    pub expected_profile_revision: u64,
    pub generation: u64,
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
        validate_bundle_head_pin(self.generation, &self.commit_id, &self.manifest_sha256)?;
        if self.created_at > self.expires_at || self.actions.len() > MAX_ACTIONS {
            return Err("invalid conversation push-plan lifetime or action count".to_string());
        }
        let mut action_ids = BTreeSet::new();
        for action in &self.actions {
            action.validate()?;
            if action.direction != ConversationPlanDirection::Push
                || action.host_id != self.host_id
                || action.replica_id != self.replica_id
                || action.local_project_id != self.local_project_id
                || action.provider_profile_id != self.provider_profile_id
                || action.pinned_generation != self.generation
                || action.pinned_commit_id != self.commit_id
                || action.pinned_manifest_sha256 != self.manifest_sha256
                || action.expected_host_identity_revision != self.expected_host_identity_revision
                || action.expected_bindings_document_revision
                    != self.expected_bindings_document_revision
                || action.expected_profile_revision != self.expected_profile_revision
                || action.expected_project_binding_revision
                    != self.expected_project_binding_revision
                || action.expires_at != self.expires_at
            {
                return Err("conversation push-plan action pins disagree with the plan".to_string());
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

/// Chain-level relation oracle. Production classification runs entirely on
/// [`classify_thread_refresh`] (manifest metadata, local chain, pin); this
/// full-chain twin remains as the test suite's independent witness that the
/// metadata table agrees with chain-level truth.
#[cfg(test)]
pub fn classify_conversation_relation(
    local: Option<&[CanonicalConversationMessage]>,
    cloud: Option<&[CanonicalConversationMessage]>,
) -> ConversationRelation {
    match (local, cloud) {
        (None, None) => return ConversationRelation::BothMissing,
        (None, Some(_)) => return ConversationRelation::LocalMissing,
        (Some(_), None) => return ConversationRelation::CloudMissing,
        (Some(local), Some(cloud)) => {
            if validate_conversation_chain(local).is_err()
                || validate_conversation_chain(cloud).is_err()
            {
                return ConversationRelation::Unknown;
            }
            let common = local.len().min(cloud.len());
            for index in 0..common {
                if local[index].message_id != cloud[index].message_id {
                    return ConversationRelation::Diverged;
                }
                if local[index].parent_message_id != cloud[index].parent_message_id
                    || local[index].semantic_fingerprint != cloud[index].semantic_fingerprint
                {
                    return ConversationRelation::Unknown;
                }
            }
            match local.len().cmp(&cloud.len()) {
                std::cmp::Ordering::Equal => ConversationRelation::Identical,
                std::cmp::Ordering::Greater => ConversationRelation::CloudPrefixOfLocal,
                std::cmp::Ordering::Less => ConversationRelation::LocalPrefixOfCloud,
            }
        }
    }
}

/// The local side of a manifest-only refresh classification.
#[derive(Clone, Copy, Debug)]
pub enum RefreshLocalState<'a> {
    /// No local conversation exists for this thread.
    Missing,
    /// A local conversation exists but its canonical chain is unavailable
    /// (unreadable transcript, incomplete codec lineage).
    Uncapturable,
    /// The captured canonical chain.
    Chain(&'a [CanonicalConversationMessage]),
}

/// The cloud side of a manifest-only refresh classification: the manifest's
/// per-thread metadata, never transcript content.
#[derive(Clone, Copy, Debug)]
pub struct RefreshCloudMeta<'a> {
    pub message_count: u64,
    pub lineage_digest: &'a str,
    pub semantic_digest: &'a str,
}

/// Classify a thread's sync state from manifest metadata, the local chain,
/// and the binding's last-synced digest pin — no cloud payload reads.  See
/// docs/DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md for the decision table and
/// its soundness argument.  Every branch is witnessed by content digests;
/// anything unwitnessed classifies as `Diverged` (review) or `Unknown`,
/// never as a fast-forward.
pub fn classify_thread_refresh(
    local: RefreshLocalState<'_>,
    cloud: Option<RefreshCloudMeta<'_>>,
    last_synced: Option<(&str, &str)>,
) -> ConversationRelation {
    let (chain, cloud) = match (local, cloud) {
        (RefreshLocalState::Missing, None) => return ConversationRelation::BothMissing,
        (RefreshLocalState::Missing, Some(_)) => return ConversationRelation::LocalMissing,
        (_, None) => return ConversationRelation::CloudMissing,
        (RefreshLocalState::Uncapturable, Some(_)) => return ConversationRelation::Unknown,
        (RefreshLocalState::Chain(chain), Some(cloud)) => (chain, cloud),
    };
    let Ok((local_lineage, local_semantic)) = conversation_chain_digests(chain) else {
        return ConversationRelation::Unknown;
    };
    if (local_lineage.as_str(), local_semantic.as_str())
        == (cloud.lineage_digest, cloud.semantic_digest)
    {
        return ConversationRelation::Identical;
    }
    let local_count = chain.len() as u64;
    match local_count.cmp(&cloud.message_count) {
        std::cmp::Ordering::Greater => {
            // The prefix is recomputed locally; a match proves the cloud
            // thread is an ancestor of the local one.
            let Ok((prefix_lineage, prefix_semantic)) =
                conversation_chain_digests(&chain[..cloud.message_count as usize])
            else {
                return ConversationRelation::Unknown;
            };
            if (prefix_lineage.as_str(), prefix_semantic.as_str())
                == (cloud.lineage_digest, cloud.semantic_digest)
            {
                ConversationRelation::CloudPrefixOfLocal
            } else {
                ConversationRelation::Diverged
            }
        }
        std::cmp::Ordering::Less => {
            // Equality with the last-synced pin proves nothing local is
            // unsaved, which is exactly what a fast-forward pull needs.  A
            // position or timestamp cannot witness this; only content can.
            if last_synced == Some((local_lineage.as_str(), local_semantic.as_str())) {
                ConversationRelation::LocalPrefixOfCloud
            } else {
                ConversationRelation::Diverged
            }
        }
        // Equal counts with different digests cannot fast-forward either way.
        std::cmp::Ordering::Equal => ConversationRelation::Diverged,
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

/// Derive the first portable identity for a provider-native conversation.
/// Fork identities remain random and are persisted in their reviewed plans;
/// this derivation only makes repeated reviews of the same previously-unbound
/// local conversation stable before its first publication commits a binding.
pub fn initial_conversation_thread_id(
    bundle_id: &BundleId,
    provider: Provider,
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

pub fn conversation_resource_id(
    provider: Provider,
    thread_id: &ThreadId,
) -> Result<ResourceId, String> {
    ResourceId::parse(format!("{}:conversation:{}", provider.as_str(), thread_id))
}

pub fn conversation_transcript_path(
    provider: Provider,
    thread_id: &ThreadId,
) -> Result<LogicalPath, String> {
    LogicalPath::parse(format!(
        "state/conversations/{}/{}/transcript.jsonl",
        provider.as_str(),
        thread_id
    ))
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
        if self.size > MAX_FILE_BYTES {
            return Err(format!("bundle file exceeds {} bytes", MAX_FILE_BYTES));
        }
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
        thread_id: ThreadId,
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
    pub generation: u64,
    pub commit_id: String,
    pub manifest_key: String,
    pub manifest_sha256: String,
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub generation: u64,
    pub commit_id: String,
    pub updated_at: u64,
    pub bundle: BundleIdentity,
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub published_by_origin: PortableOrigin,
    #[serde(default)]
    pub conversations: BTreeMap<ThreadId, ConversationThread>,
    #[serde(default)]
    pub resources: BTreeMap<ResourceId, ResourceDescriptor>,
    #[serde(default)]
    pub files: BTreeMap<LogicalPath, BundleFileEntry>,
    #[serde(default)]
    pub directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
    #[serde(default)]
    pub tombstones: BTreeMap<String, Tombstone>,
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
        self.recipe.validate()?;
        if self.resources.len() > MAX_RESOURCES
            || self.files.len() > MAX_FILES
            || self.directories.len() > MAX_FILES
            || self.tombstones.len() > MAX_FILES
        {
            return Err("bundle manifest exceeds collection limits".to_string());
        }
        for (id, descriptor) in &self.resources {
            if id != &descriptor.resource_id {
                return Err(format!("resource key '{}' does not match descriptor", id));
            }
            descriptor.validate()?;
        }
        self.validate_conversations()?;
        let recipe_ids = self.recipe.selected_ids();
        let resource_ids: BTreeSet<_> = self.resources.keys().cloned().collect();
        if recipe_ids != resource_ids {
            return Err("manifest recipe and live resources differ".to_string());
        }
        let mut folded_paths = BTreeMap::<String, (&LogicalPath, &'static str)>::new();
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
            let folded = path.as_str().to_lowercase();
            if let Some((previous, previous_kind)) =
                folded_paths.insert(folded, (path, "directory"))
            {
                return Err(format!(
                    "portable path collision between {} '{}' and directory '{}'",
                    previous_kind, previous, path
                ));
            }
        }
        for (path, entry) in &self.files {
            entry.validate()?;
            let descriptor = self.resources.get(&entry.resource_id).ok_or_else(|| {
                format!(
                    "file '{}' references missing resource '{}'",
                    path, entry.resource_id
                )
            })?;
            *file_owners.entry(entry.resource_id.clone()).or_default() += 1;
            let folded = path.as_str().to_lowercase();
            if let Some((previous, previous_kind)) = folded_paths.insert(folded, (path, "file")) {
                return Err(format!(
                    "portable path collision between {} '{}' and file '{}'",
                    previous_kind, previous, path
                ));
            }
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
        for path in self.files.keys() {
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
            }
        }
        Ok(())
    }

    fn validate_conversations(&self) -> Result<(), String> {
        if self.conversations.len() > MAX_RESOURCES {
            return Err("bundle has too many conversation threads".to_string());
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
            let expected_kind = match thread.provider {
                Provider::Codex => ResourceKind::CodexConversation,
                Provider::Claude => ResourceKind::ClaudeConversation,
            };
            if descriptor.kind != expected_kind || descriptor.provider != Some(thread.provider) {
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
            if self
                .directories
                .values()
                .any(|entry| entry.resource_id == thread.resource_id)
            {
                return Err(format!(
                    "conversation resource '{}' cannot own a directory entry",
                    thread.resource_id
                ));
            }
        }
        for (resource_id, descriptor) in &self.resources {
            if descriptor.kind == ResourceKind::CodexConversation
                && !thread_resources.contains(resource_id)
            {
                return Err(format!(
                    "conversation resource '{}' belongs to no live thread",
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
        Ok(())
    }

    pub fn validate_against_head(&self, head: &BundleHead) -> Result<(), String> {
        self.validate()?;
        head.validate()?;
        if self.schema_version != head.schema_version
            || self.bundle.bundle_id != head.bundle_id
            || self.bundle.kind != head.kind
            || self.generation != head.generation
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
        provider: Provider,
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
    pub generation: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub binding_revision: u64,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub actions: Vec<RestoreAction>,
    #[serde(default)]
    pub conversation_actions: Vec<ConversationPlanAction>,
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
            if action.direction != ConversationPlanDirection::Pull
                || action.host_id != self.host_id
                || action.replica_id != self.replica_id
                || action.pinned_generation != self.generation
                || action.pinned_commit_id != self.commit_id
                || action.pinned_manifest_sha256 != self.manifest_sha256
                || action.expected_host_identity_revision != self.expected_host_identity_revision
                || action.expires_at != self.expires_at
            {
                return Err("conversation Pull action pins disagree with the plan".to_string());
            }
            if !action_ids.insert(action.action_id.clone()) {
                return Err(format!("duplicate restore action '{}'", action.action_id));
            }
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
pub struct CodexDivergenceReceipt {
    pub resolution: ConversationDivergenceResolution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_provider_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<CodexForkExecutor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_fork_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_name_status: Option<ConversationNameStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_original_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name_status: Option<ConversationNameStatus>,
    pub inventory_refresh_status: CodexInventoryRefreshStatus,
}

impl CodexDivergenceReceipt {
    fn validate(&self) -> Result<(), String> {
        match self.resolution {
            ConversationDivergenceResolution::PreserveLocalWithNativeFork => {
                if self.preserved_thread_id.is_none()
                    || self.preserved_provider_thread_id.is_none()
                    || self.executor.is_none()
                    || self.desired_fork_name.is_none()
                    || self.fork_name_status.is_none()
                {
                    return Err("preserved divergence receipt is incomplete".to_string());
                }
            }
            ConversationDivergenceResolution::UseCloudOnly => {
                if self.preserved_thread_id.is_some()
                    || self.preserved_provider_thread_id.is_some()
                    || self.executor.is_some()
                    || self.desired_fork_name.is_some()
                    || self.fork_name_status.is_some()
                {
                    return Err("cloud-only divergence receipt carries fork state".to_string());
                }
            }
        }
        if let Some(thread_id) = &self.preserved_provider_thread_id {
            validate_provider_thread_id("preserved provider thread", thread_id)?;
        }
        if self.original_name_status.is_none() {
            return Err("Codex divergence receipt lacks an original naming result".to_string());
        }
        for (label, name) in [
            ("preserved fork name", self.desired_fork_name.as_deref()),
            (
                "restored original name",
                self.desired_original_name.as_deref(),
            ),
        ] {
            if let Some(name) = name {
                validate_codex_thread_name(label, name)?;
            }
        }
        if self.inventory_refresh_status == CodexInventoryRefreshStatus::NotAttempted {
            return Err("Codex divergence receipt lacks an inventory refresh result".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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
    pub cloud_generation: u64,
    pub cloud_commit_id: String,
    pub cloud_manifest_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_origin_host_token: Option<OriginHostToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_origin_replica_token: Option<OriginReplicaToken>,
    pub journal_operation_id: ConversationOperationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divergence: Option<CodexDivergenceReceipt>,
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
        if let Some(divergence) = &self.divergence {
            divergence.validate()?;
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
    pub generation: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_base_defaults_sync_activity_timestamps() {
        let base: RecipeBase = serde_json::from_str(
            r#"{"generation":1,"manifest_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","commit_id":"commit-1","recipe_revision":2}"#,
        )
        .unwrap();
        assert_eq!(base.last_pull_at, None);
        assert_eq!(base.last_push_at, None);
    }

    /// The commit ID names the manifest the continuity probe checks, so a
    /// base without one is invalid rather than quietly unprobeable
    /// (DESIGN_REMOTE_HISTORY_CONTINUITY.md cutover step 1).
    #[test]
    fn recipe_base_requires_an_immutable_commit_id() {
        assert!(serde_json::from_str::<RecipeBase>(
            r#"{"generation":1,"manifest_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","recipe_revision":2}"#,
        )
        .is_err());
    }

    fn bundle_id() -> BundleId {
        BundleId::parse("0123456789abcdef0123456789abcdef").unwrap()
    }

    fn resource_id(value: &str) -> ResourceId {
        ResourceId::parse(value).unwrap()
    }

    fn portable_origin() -> PortableOrigin {
        PortableOrigin {
            origin_host_token: OriginHostToken::parse("a".repeat(64)).unwrap(),
            origin_replica_token: OriginReplicaToken::parse("b".repeat(64)).unwrap(),
            shared_label: None,
        }
    }

    fn project_content_descriptor(entry_type: &str, relative_path: &str) -> ResourceDescriptor {
        let logical_path = LogicalPath::parse(format!("project/{}", relative_path)).unwrap();
        let (_, resource_id) = project_content_identity(entry_type, &logical_path).unwrap();
        ResourceDescriptor {
            resource_id,
            kind: match entry_type {
                "file" => ResourceKind::ProjectContentFile,
                "dir" => ResourceKind::ProjectContentDirectory,
                _ => panic!("unsupported test entry type"),
            },
            provider: None,
            scope: ResourceScope::Project,
            display_name: relative_path.to_string(),
            provenance: Provenance::ProjectLocal {
                relative_path: relative_path.to_string(),
            },
            apply_policy: ApplyPolicy::ExplicitReview,
            relative_cwd: None,
            codec_version: 1,
            metadata: BTreeMap::new(),
        }
    }

    fn project_content_manifest() -> BundleManifest {
        let mut manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            generation: 1,
            commit_id: "commit-project-content".to_string(),
            updated_at: 1,
            bundle: BundleIdentity {
                bundle_id: bundle_id(),
                display_name: "Project".to_string(),
                kind: BundleKind::Project,
                repository_fingerprint: None,
            },
            recipe: BundleRecipe::default(),
            captured_with: CapturedWith::default(),
            published_by_origin: portable_origin(),
            conversations: BTreeMap::new(),
            resources: BTreeMap::new(),
            files: BTreeMap::new(),
            directories: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        };
        for relative_path in ["docs", "docs/specs", "docs/empty"] {
            let descriptor = project_content_descriptor("dir", relative_path);
            let resource_id = descriptor.resource_id.clone();
            manifest.recipe.entries.insert(
                resource_id.clone(),
                RecipeEntry {
                    resource_id: resource_id.clone(),
                    apply_policy: ApplyPolicy::ExplicitReview,
                    required: relative_path != "docs/empty",
                },
            );
            manifest.resources.insert(resource_id.clone(), descriptor);
            manifest.directories.insert(
                LogicalPath::parse(format!("project/{}", relative_path)).unwrap(),
                BundleDirectoryEntry {
                    resource_id,
                    mode: Some(0o755),
                    source_mtime: 1,
                },
            );
        }
        let descriptor = project_content_descriptor("file", "docs/specs/a.md");
        let resource_id = descriptor.resource_id.clone();
        manifest.recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::ExplicitReview,
                required: false,
            },
        );
        manifest.resources.insert(resource_id.clone(), descriptor);
        manifest.files.insert(
            LogicalPath::parse("project/docs/specs/a.md").unwrap(),
            BundleFileEntry {
                resource_id,
                sha256: "a".repeat(64),
                size: 1,
                source_mtime: 1,
                object_key: "_uploads/upload-1/files/project/docs/specs/a.md".to_string(),
                mode: Some(0o644),
            },
        );
        manifest
    }

    #[test]
    fn ids_fail_closed_during_deserialization() {
        assert!(BundleId::parse("../escape").is_err());
        assert!(BundleId::parse("ABCDEF0123456789ABCDEF0123456789").is_err());
        assert!(ResourceId::parse("skill:release").is_ok());
        assert!(ResourceId::parse("Skill/Release").is_err());
        assert!(serde_json::from_str::<BundleId>("\"../../etc/passwd\"").is_err());
    }

    #[test]
    fn initial_conversation_ids_are_stable_and_bundle_scoped() {
        let first = initial_conversation_thread_id(
            &bundle_id(),
            Provider::Codex,
            "019f742a-a206-7932-876c-9db8d8ce575a",
        )
        .unwrap();
        assert_eq!(
            first,
            initial_conversation_thread_id(
                &bundle_id(),
                Provider::Codex,
                "019f742a-a206-7932-876c-9db8d8ce575a",
            )
            .unwrap()
        );
        assert_ne!(
            first,
            initial_conversation_thread_id(
                &BundleId::parse("11111111111111111111111111111111").unwrap(),
                Provider::Codex,
                "019f742a-a206-7932-876c-9db8d8ce575a",
            )
            .unwrap()
        );
    }

    #[test]
    fn host_identity_and_scoped_origin_tokens_have_fixed_validated_encodings() {
        assert!(HostId::parse("00112233445566778899aabbccddeeff").is_ok());
        assert!(HostId::parse("00112233445566778899AABBCCDDEEFF").is_err());
        assert!(HostId::parse("short").is_err());

        let host_id = HostId::parse("00112233445566778899aabbccddeeff").unwrap();
        let bundle = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let replica = ReplicaId::parse("replica-a").unwrap();
        let host_token = derive_origin_host_token(&host_id, &bundle).unwrap();
        let replica_token = derive_origin_replica_token(&host_id, &bundle, &replica).unwrap();

        // Golden vectors freeze the domain-separated u64-length-prefixed
        // encoding used by every portable-bundle writer.
        assert_eq!(
            host_token.as_str(),
            "a4ab94f1dfb193b63b7b1862aa6c9cabe17ff1c463287f2958281527c75bc62d"
        );
        assert_eq!(
            replica_token.as_str(),
            "0399716f9fe42796b64a32c5763e91334d58017c93f0c6dd56590320530edd07"
        );
        assert_eq!(
            derive_origin_host_token(&host_id, &bundle).unwrap(),
            host_token
        );
        assert_ne!(
            derive_origin_host_token(
                &host_id,
                &BundleId::parse("11111111111111111111111111111111").unwrap()
            )
            .unwrap(),
            host_token
        );
        assert_ne!(
            derive_origin_replica_token(&host_id, &bundle, &ReplicaId::parse("replica-b").unwrap())
                .unwrap(),
            replica_token
        );
        assert_ne!(
            derive_origin_host_token(
                &HostId::parse("ffeeddccbbaa99887766554433221100").unwrap(),
                &bundle
            )
            .unwrap(),
            host_token
        );
    }

    #[test]
    fn origin_labels_are_bounded_local_presentation_only() {
        let token = OriginHostToken::parse("7f3a".repeat(16)).unwrap();
        assert_eq!(fallback_origin_host_label(&token), "Host 7F3A");
        let valid = PortableOrigin {
            origin_host_token: token,
            origin_replica_token: OriginReplicaToken::parse("ab".repeat(32)).unwrap(),
            shared_label: Some("Work laptop".to_string()),
        };
        valid.validate().unwrap();
        let mut invalid = valid;
        invalid.shared_label = Some("x".repeat(129));
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn logical_paths_are_portable_and_namespace_bounded() {
        assert!(LogicalPath::parse("project/.agents/skills/release/SKILL.md").is_ok());
        assert!(LogicalPath::parse("state/claude/projects/root/session.jsonl").is_ok());
        for path in [
            "/project/a",
            "project/../secret",
            "project/a\\b",
            "project/CON/file",
            "unknown/file",
            "state",
        ] {
            assert!(LogicalPath::parse(path).is_err(), "{}", path);
        }
    }

    #[test]
    fn config_rejects_split_bundle_identity_and_duplicate_cells() {
        let storage = StorageConfig {
            id: StorageId::parse("personal").unwrap(),
            name: "Personal".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: "/tmp/project-sync-store".to_string(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        };
        let project = LocalProjectRegistration {
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            bundle_id: bundle_id(),
            display_name: "Project A".to_string(),
            local_alias: None,
            repository_fingerprint: None,
            recipe: BundleRecipe::default(),
            recipe_bases: BTreeMap::new(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        };
        let mut config = SyncConfig {
            schema: LOCAL_SCHEMA,
            revision: 0,
            storages: vec![storage],
            projects: vec![project.clone()],
            links: vec![ProjectStorageLink {
                local_project_id: project.local_project_id.clone(),
                storage_id: StorageId::parse("personal").unwrap(),
                bundle_id: BundleId::parse("11111111111111111111111111111111").unwrap(),
                recipe: None,
                project_content_preferences: Default::default(),
                pinned: false,
                created_at: 1,
            }],
        };
        assert!(config.validate().unwrap_err().contains("differs"));
        config.links[0].bundle_id = bundle_id();
        config.links.push(config.links[0].clone());
        assert!(config.validate().unwrap_err().contains("duplicate"));
    }

    #[test]
    fn every_mallard_contract_is_reset_to_schema_one() {
        for (name, schema) in [
            ("local config", LOCAL_SCHEMA),
            ("machine projects", MACHINE_PROJECT_SCHEMA),
            ("host identity", HOST_IDENTITY_SCHEMA),
            ("conversation bindings", CONVERSATION_BINDINGS_SCHEMA),
            (
                "conversation sync journal",
                CONVERSATION_SYNC_JOURNAL_SCHEMA,
            ),
            ("conversation push plan", PUSH_PLAN_SCHEMA),
            ("bundle", BUNDLE_SCHEMA),
            ("recipe", RECIPE_SCHEMA),
            ("restore plan", RESTORE_PLAN_SCHEMA),
            ("setup draft", SETUP_DRAFT_SCHEMA),
            ("setup transaction", SETUP_TRANSACTION_SCHEMA),
            (
                "project content preferences",
                PROJECT_CONTENT_PREFERENCES_SCHEMA,
            ),
        ] {
            assert_eq!(schema, 1, "{name} must start at schema 1");
        }

        let current = SyncConfig::default();
        current.validate().unwrap();

        for unsupported_schema in [2, 3] {
            let mut unsupported = current.clone();
            unsupported.schema = unsupported_schema;
            let error = unsupported.validate().unwrap_err();
            assert!(
                error.contains("unsupported project-sync schema"),
                "{unsupported_schema}: {error}"
            );
        }
    }

    #[test]
    fn active_bindings_share_a_checkout_only_through_distinct_provider_configs() {
        let project = |id: &str, bundle: &str| LocalProjectRegistration {
            local_project_id: LocalProjectId::parse(id).unwrap(),
            bundle_id: BundleId::parse(bundle).unwrap(),
            display_name: id.to_string(),
            local_alias: None,
            repository_fingerprint: None,
            recipe: BundleRecipe::default(),
            recipe_bases: BTreeMap::new(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        };
        let config = SyncConfig {
            schema: LOCAL_SCHEMA,
            revision: 0,
            storages: Vec::new(),
            projects: vec![
                project("project-a", "0123456789abcdef0123456789abcdef"),
                project("project-b", "11111111111111111111111111111111"),
            ],
            links: Vec::new(),
        };
        let profile = |id: &str, path: &str| ProviderProfile {
            profile_id: LocalProviderProfileId::parse(id).unwrap(),
            provider: Provider::Codex,
            display_name: id.to_string(),
            path: path.to_string(),
            canonical_path: path.to_string(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        };
        let binding = |replica: &str, project: &str, bundle: &str, profile: &str| ProjectBinding {
            host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
            replica_id: ReplicaId::parse(replica).unwrap(),
            local_project_id: LocalProjectId::parse(project).unwrap(),
            bundle_id: BundleId::parse(bundle).unwrap(),
            project_root: "/home/user/projects/health-game".to_string(),
            canonical_project_root: "/home/user/projects/health-game".to_string(),
            profile_ids: BTreeMap::from([(
                Provider::Codex,
                LocalProviderProfileId::parse(profile).unwrap(),
            )]),
            codex_home: None,
            claude_home: None,
            state: BindingState::Active,
            revision: 0,
            updated_at: 1,
        };
        let mut machine = MachineProjectState {
            schema: MACHINE_PROJECT_SCHEMA,
            revision: 0,
            profiles: vec![
                profile("profile-main", "/home/user/.codex"),
                profile("profile-alt", "/home/user/conf2/.codex"),
            ],
            bindings: vec![
                binding(
                    "replica-a",
                    "project-a",
                    "0123456789abcdef0123456789abcdef",
                    "profile-main",
                ),
                binding(
                    "replica-b",
                    "project-b",
                    "11111111111111111111111111111111",
                    "profile-alt",
                ),
            ],
        };
        machine.validate(&config).unwrap();
        machine.bindings[1].profile_ids = BTreeMap::from([(
            Provider::Codex,
            LocalProviderProfileId::parse("profile-main").unwrap(),
        )]);
        let error = machine.validate(&config).unwrap_err();
        assert!(error.contains("same provider config"), "{}", error);
    }

    #[test]
    fn recipe_key_is_identity_and_digest_is_only_version_metadata() {
        let id = resource_id("memo:release");
        let mut recipe = BundleRecipe::default();
        recipe.entries.insert(
            id.clone(),
            RecipeEntry {
                resource_id: id,
                apply_policy: ApplyPolicy::ExplicitReview,
                required: false,
            },
        );
        assert!(recipe.validate().is_ok());
        let encoded = serde_json::to_string(&recipe).unwrap();
        assert!(encoded.contains("memo:release"));
        assert!(!encoded.contains("source_digest"));
    }

    #[test]
    fn manifest_requires_recipe_resource_and_file_referential_integrity() {
        let id = resource_id("memory:claude");
        let path = LogicalPath::parse("state/claude/memory/MEMORY.md").unwrap();
        let mut recipe = BundleRecipe::default();
        recipe.entries.insert(
            id.clone(),
            RecipeEntry {
                resource_id: id.clone(),
                apply_policy: ApplyPolicy::Merge,
                required: false,
            },
        );
        let mut resources = BTreeMap::new();
        resources.insert(
            id.clone(),
            ResourceDescriptor {
                resource_id: id.clone(),
                kind: ResourceKind::ProjectMemory,
                provider: Some(Provider::Claude),
                scope: ResourceScope::ProviderState,
                display_name: "Claude memory".to_string(),
                provenance: Provenance::Unknown,
                apply_policy: ApplyPolicy::Merge,
                relative_cwd: None,
                codec_version: 1,
                metadata: BTreeMap::new(),
            },
        );
        let mut files = BTreeMap::new();
        files.insert(
            path,
            BundleFileEntry {
                resource_id: id,
                sha256: "a".repeat(64),
                size: 1,
                source_mtime: 1,
                object_key: "_uploads/upload-1/files/state/claude/memory/MEMORY.md".to_string(),
                mode: Some(0o644),
            },
        );
        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            generation: 1,
            commit_id: "commit-1".to_string(),
            updated_at: 1,
            bundle: BundleIdentity {
                bundle_id: bundle_id(),
                display_name: "Project".to_string(),
                kind: BundleKind::Project,
                repository_fingerprint: None,
            },
            recipe,
            captured_with: CapturedWith::default(),
            published_by_origin: portable_origin(),
            conversations: BTreeMap::new(),
            resources,
            files,
            directories: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        };
        assert!(manifest.validate().is_ok());
        let mut broken = manifest.clone();
        broken.resources.clear();
        assert!(broken.validate().is_err());
    }

    #[test]
    fn project_content_manifest_validates_exact_tree_identity() {
        let manifest = project_content_manifest();
        manifest.validate().unwrap();

        let mut unsupported = manifest.clone();
        unsupported.schema_version = BUNDLE_SCHEMA + 1;
        assert!(unsupported.validate().unwrap_err().contains("unsupported"));

        let mut missing_ancestor = manifest.clone();
        let docs = LogicalPath::parse("project/docs").unwrap();
        let docs_resource = missing_ancestor
            .directories
            .remove(&docs)
            .unwrap()
            .resource_id;
        missing_ancestor.resources.remove(&docs_resource);
        missing_ancestor.recipe.entries.remove(&docs_resource);
        assert!(missing_ancestor
            .validate()
            .unwrap_err()
            .contains("lacks directory entry"));

        let mut orphan = manifest.clone();
        orphan
            .files
            .remove(&LogicalPath::parse("project/docs/specs/a.md").unwrap());
        assert!(orphan
            .validate()
            .unwrap_err()
            .contains("must own exactly one file"));

        let mut forged = manifest.clone();
        let file_id = forged.files.values().next().unwrap().resource_id.clone();
        forged.resources.get_mut(&file_id).unwrap().provenance = Provenance::ProjectLocal {
            relative_path: "docs/specs/other.md".to_string(),
        };
        assert!(forged
            .validate()
            .unwrap_err()
            .contains("mismatched path provenance"));
    }

    #[test]
    fn project_content_manifest_rejects_path_collision_and_oversized_file() {
        let mut collision = project_content_manifest();
        let descriptor = project_content_descriptor("file", "docs/empty");
        let resource_id = descriptor.resource_id.clone();
        collision.recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::ExplicitReview,
                required: false,
            },
        );
        collision.resources.insert(resource_id.clone(), descriptor);
        collision.files.insert(
            LogicalPath::parse("project/docs/empty").unwrap(),
            BundleFileEntry {
                resource_id,
                sha256: "b".repeat(64),
                size: 1,
                source_mtime: 1,
                object_key: "_uploads/upload-1/files/project/docs/empty".to_string(),
                mode: Some(0o644),
            },
        );
        assert!(collision.validate().unwrap_err().contains("path collision"));

        let mut oversized = project_content_manifest();
        oversized.files.values_mut().next().unwrap().size = MAX_PROJECT_CONTENT_FILE_BYTES + 1;
        assert!(oversized.validate().unwrap_err().contains("exceeds"));
    }

    #[test]
    fn project_content_tombstones_are_typed_and_path_bound() {
        let mut manifest = project_content_manifest();
        let path = LogicalPath::parse("project/docs/specs/a.md").unwrap();
        let entry = manifest.files.remove(&path).unwrap();
        manifest.resources.remove(&entry.resource_id);
        manifest.recipe.entries.remove(&entry.resource_id);
        let tombstone = Tombstone {
            target: TombstoneTarget::ProjectContentFile {
                resource_id: entry.resource_id,
                logical_path: path,
            },
            last_sha256: Some(entry.sha256),
            deleted_at: 2,
        };
        manifest
            .tombstones
            .insert(tombstone.canonical_key(), tombstone);
        manifest.validate().unwrap();

        let key = manifest.tombstones.keys().next().unwrap().clone();
        let tombstone = manifest.tombstones.get_mut(&key).unwrap();
        if let TombstoneTarget::ProjectContentFile { resource_id, .. } = &mut tombstone.target {
            *resource_id = project_content_descriptor("file", "docs/specs/other.md").resource_id;
        }
        let changed = manifest.tombstones.remove(&key).unwrap();
        manifest.tombstones.insert(changed.canonical_key(), changed);
        assert!(manifest.validate().unwrap_err().contains("does not match"));
    }

    #[test]
    fn executable_restore_actions_cannot_be_implicitly_approved() {
        let action = RestoreAction {
            action_id: ActionId::parse("action-1").unwrap(),
            resource_id: resource_id("project:file:docs-readme"),
            kind: RestoreActionKind::WriteProjectFile {
                logical_path: LogicalPath::parse("project/docs/README.md").unwrap(),
                mode: 0o644,
                source_mtime: 1,
            },
            target_path: None,
            source_sha256: None,
            expected_target_sha256: None,
            expected_target_mode: None,
            materialized_sha256: None,
            requires_explicit_approval: false,
        };
        assert!(action.validate().unwrap_err().contains("explicit approval"));
    }

    fn canonical_message(
        id: &str,
        parent: Option<&str>,
        semantic_byte: char,
    ) -> CanonicalConversationMessage {
        CanonicalConversationMessage {
            message_id: MessageId::parse(id).unwrap(),
            parent_message_id: parent.map(|value| MessageId::parse(value).unwrap()),
            semantic_fingerprint: semantic_byte.to_string().repeat(64),
        }
    }

    #[test]
    fn conversation_relation_matrix_uses_ids_parentage_and_semantics() {
        let base = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
        ];
        let local = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
            canonical_message("m-local", Some("m-b"), 'c'),
        ];
        let cloud = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
            canonical_message("m-cloud", Some("m-b"), 'd'),
        ];

        assert_eq!(
            classify_conversation_relation(Some(&base), Some(&base)),
            ConversationRelation::Identical
        );
        assert_eq!(
            classify_conversation_relation(Some(&local), Some(&base)),
            ConversationRelation::CloudPrefixOfLocal
        );
        assert_eq!(
            classify_conversation_relation(Some(&base), Some(&cloud)),
            ConversationRelation::LocalPrefixOfCloud
        );
        assert_eq!(
            classify_conversation_relation(Some(&local), Some(&cloud)),
            ConversationRelation::Diverged
        );
        assert_eq!(
            classify_conversation_relation(None, Some(&cloud)),
            ConversationRelation::LocalMissing
        );
        assert_eq!(
            classify_conversation_relation(Some(&local), None),
            ConversationRelation::CloudMissing
        );
        assert_eq!(
            classify_conversation_relation(None, None),
            ConversationRelation::BothMissing
        );

        let same_text_different_id = vec![canonical_message("other", None, 'a')];
        assert_eq!(
            classify_conversation_relation(Some(&base[..1]), Some(&same_text_different_id),),
            ConversationRelation::Diverged
        );

        let mut reused_id = base.clone();
        reused_id[1].semantic_fingerprint = "e".repeat(64);
        assert_eq!(
            classify_conversation_relation(Some(&base), Some(&reused_id)),
            ConversationRelation::Unknown
        );

        let mut broken_parent = base.clone();
        broken_parent[1].parent_message_id = None;
        assert_eq!(
            classify_conversation_relation(Some(&base), Some(&broken_parent)),
            ConversationRelation::Unknown
        );
    }

    #[test]
    fn conversation_digests_separate_lineage_from_semantics() {
        let first = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
        ];
        let mut semantic_change = first.clone();
        semantic_change[1].semantic_fingerprint = "c".repeat(64);
        let different_lineage = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-c", Some("m-a"), 'b'),
        ];

        let first_digests = conversation_chain_digests(&first).unwrap();
        let semantic_digests = conversation_chain_digests(&semantic_change).unwrap();
        let lineage_digests = conversation_chain_digests(&different_lineage).unwrap();
        assert_eq!(first_digests.0, semantic_digests.0);
        assert_ne!(first_digests.1, semantic_digests.1);
        assert_ne!(first_digests.0, lineage_digests.0);
        assert_ne!(first_digests.1, lineage_digests.1);
    }

    fn refresh_meta(chain: &[CanonicalConversationMessage]) -> ((String, String), u64) {
        (
            conversation_chain_digests(chain).unwrap(),
            chain.len() as u64,
        )
    }

    fn cloud_meta<'a>(pair: &'a (String, String), count: u64) -> RefreshCloudMeta<'a> {
        RefreshCloudMeta {
            message_count: count,
            lineage_digest: &pair.0,
            semantic_digest: &pair.1,
        }
    }

    /// Every row of the decision table in
    /// docs/DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md, straight down the table.
    /// Row 1 (tombstones) and row 5's binding-only variant live in the
    /// review layer; everything else is the classifier's contract.
    #[test]
    fn manifest_only_refresh_covers_every_decision_row() {
        let shared = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
        ];
        let extended = {
            let mut chain = shared.clone();
            chain.push(canonical_message("m-c", Some("m-b"), 'c'));
            chain
        };
        let (shared_pair, shared_count) = refresh_meta(&shared);
        let (extended_pair, extended_count) = refresh_meta(&extended);
        let branched = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-x", Some("m-a"), '4'),
            canonical_message("m-y", Some("m-x"), '5'),
        ];
        let (branched_pair, _) = refresh_meta(&branched);

        // Rows 2-4: presence.
        assert_eq!(
            classify_thread_refresh(RefreshLocalState::Missing, None, None),
            ConversationRelation::BothMissing
        );
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Missing,
                Some(cloud_meta(&shared_pair, shared_count)),
                None
            ),
            ConversationRelation::LocalMissing
        );
        assert_eq!(
            classify_thread_refresh(RefreshLocalState::Chain(&shared), None, None),
            ConversationRelation::CloudMissing
        );
        // Row 5: bound but uncapturable holds the row.
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Uncapturable,
                Some(cloud_meta(&shared_pair, shared_count)),
                None
            ),
            ConversationRelation::Unknown
        );
        // Row 6: digest-pair equality is sync.
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&shared),
                Some(cloud_meta(&shared_pair, shared_count)),
                None
            ),
            ConversationRelation::Identical
        );
        // Rows 7-8: longer local proves ancestry with a prefix pair or goes
        // to review.
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&extended),
                Some(cloud_meta(&shared_pair, shared_count)),
                None
            ),
            ConversationRelation::CloudPrefixOfLocal
        );
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&branched),
                Some(cloud_meta(&shared_pair, shared_count)),
                None
            ),
            ConversationRelation::Diverged
        );
        // Rows 9-10: shorter local fast-forwards only when the pin proves
        // nothing local is unsaved.
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&shared),
                Some(cloud_meta(&extended_pair, extended_count)),
                Some((&shared_pair.0, &shared_pair.1))
            ),
            ConversationRelation::LocalPrefixOfCloud
        );
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&shared),
                Some(cloud_meta(&extended_pair, extended_count)),
                Some((&branched_pair.0, &branched_pair.1))
            ),
            ConversationRelation::Diverged
        );
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&shared),
                Some(cloud_meta(&extended_pair, extended_count)),
                None
            ),
            ConversationRelation::Diverged
        );
        // Row 11: equal counts with different content cannot fast-forward.
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&branched),
                Some(cloud_meta(&extended_pair, extended_count)),
                None
            ),
            ConversationRelation::Diverged
        );
    }

    /// The loss scenarios that defeated the timestamp- and position-based
    /// refresh designs, kept as regressions so they cannot return.  Each
    /// comment names the design the scenario broke.
    #[test]
    fn manifest_only_refresh_defeats_the_recorded_loss_scenarios() {
        let synced = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
            canonical_message("m-c", Some("m-b"), 'c'),
        ];
        let (synced_pair, _) = refresh_meta(&synced);
        let cloud_extended = {
            let mut chain = synced.clone();
            chain.push(canonical_message("m-d", Some("m-c"), 'd'));
            chain.push(canonical_message("m-e", Some("m-d"), 'e'));
            chain
        };
        let (cloud_pair, cloud_count) = refresh_meta(&cloud_extended);

        // Two-machine ping-pong: A pushed (pin moved), B pushed on top.  A's
        // refresh must read cloud-outstanding, not divergent — the pull-time
        // timestamp design classified this happy path as divergence.
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&synced),
                Some(cloud_meta(&cloud_pair, cloud_count)),
                Some((&synced_pair.0, &synced_pair.1))
            ),
            ConversationRelation::LocalPrefixOfCloud
        );

        // Edit-and-resubmit that regrows to the synced length: the
        // position-pin design read pin == length as clean and clobbered the
        // local branch with a pull.
        let regrown = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
            canonical_message("m-r", Some("m-b"), '7'),
        ];
        assert_eq!(refresh_meta(&regrown).1, synced.len() as u64);
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&regrown),
                Some(cloud_meta(&cloud_pair, cloud_count)),
                Some((&synced_pair.0, &synced_pair.1))
            ),
            ConversationRelation::Diverged
        );

        // Truncation below the last-synced state: unspecified in both
        // rejected designs, drifting toward a destructive pull.
        let truncated = &synced[..2];
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(truncated),
                Some(cloud_meta(&cloud_pair, cloud_count)),
                Some((&synced_pair.0, &synced_pair.1))
            ),
            ConversationRelation::Diverged
        );

        // Never synced on this replica: the 'max' timestamp sentinel
        // auto-classified this as cloud-outstanding and replaced local
        // content that had never been saved anywhere.
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&synced),
                Some(cloud_meta(&cloud_pair, cloud_count)),
                None
            ),
            ConversationRelation::Diverged
        );

        // Equal-length force-replace over a pin-clean local: review, never a
        // silent replace.
        let replaced = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
            canonical_message("m-z", Some("m-b"), '9'),
        ];
        let (replaced_pair, replaced_count) = refresh_meta(&replaced);
        assert_eq!(
            classify_thread_refresh(
                RefreshLocalState::Chain(&synced),
                Some(cloud_meta(&replaced_pair, replaced_count)),
                Some((&synced_pair.0, &synced_pair.1))
            ),
            ConversationRelation::Diverged
        );
    }

    fn conversation_manifest() -> BundleManifest {
        let bundle = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let thread_id = ThreadId::parse("thread-0123456789abcdef0123456789abcdef").unwrap();
        let resource_id = conversation_resource_id(Provider::Codex, &thread_id).unwrap();
        let transcript_path = conversation_transcript_path(Provider::Codex, &thread_id).unwrap();
        let chain = vec![
            canonical_message("m-a", None, 'a'),
            canonical_message("m-b", Some("m-a"), 'b'),
        ];
        let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain).unwrap();
        let thread = ConversationThread {
            thread_id: thread_id.clone(),
            provider: Provider::Codex,
            display_title: Some("Conversation".to_string()),
            resource_id: resource_id.clone(),
            files: vec![ConversationFileRef {
                logical_path: transcript_path.clone(),
                role: ConversationFileRole::Transcript,
            }],
            lineage_status: ConversationLineageStatus::Complete,
            lineage_codec: "codex-lineage-v1".to_string(),
            head_message_id: Some(MessageId::parse("m-b").unwrap()),
            message_count: 2,
            lineage_digest,
            semantic_digest,
            forked_from: None,
            source_thread_id: Some("thread-a".to_string()),
            created_by_origin: None,
            last_updated_by_origin: None,
            created_at: 1,
            updated_at: 1,
        };
        let descriptor = ResourceDescriptor {
            resource_id: resource_id.clone(),
            kind: ResourceKind::CodexConversation,
            provider: Some(Provider::Codex),
            scope: ResourceScope::ProviderState,
            display_name: "Conversation".to_string(),
            provenance: Provenance::Unknown,
            apply_policy: ApplyPolicy::SafeFile,
            relative_cwd: Some(".".to_string()),
            codec_version: 1,
            metadata: BTreeMap::new(),
        };
        BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            generation: 2,
            commit_id: "commit-conversation".to_string(),
            updated_at: 2,
            bundle: BundleIdentity {
                bundle_id: bundle,
                display_name: "Project".to_string(),
                kind: BundleKind::Project,
                repository_fingerprint: None,
            },
            recipe: BundleRecipe {
                schema_version: RECIPE_SCHEMA,
                revision: 1,
                entries: BTreeMap::from([(
                    resource_id.clone(),
                    RecipeEntry {
                        resource_id: resource_id.clone(),
                        apply_policy: ApplyPolicy::SafeFile,
                        required: false,
                    },
                )]),
            },
            captured_with: CapturedWith {
                app_version: "test".to_string(),
                codex_version: Some("test".to_string()),
                claude_version: None,
                codec_versions: BTreeMap::from([("codex-lineage-v1".to_string(), 1)]),
            },
            published_by_origin: PortableOrigin {
                origin_host_token: OriginHostToken::parse("a".repeat(64)).unwrap(),
                origin_replica_token: OriginReplicaToken::parse("b".repeat(64)).unwrap(),
                shared_label: None,
            },
            conversations: BTreeMap::from([(thread_id.clone(), thread)]),
            resources: BTreeMap::from([(resource_id.clone(), descriptor)]),
            files: BTreeMap::from([(
                transcript_path,
                BundleFileEntry {
                    resource_id,
                    sha256: "c".repeat(64),
                    size: 10,
                    source_mtime: 1,
                    object_key: "_uploads/upload-1/files/transcript.jsonl".to_string(),
                    mode: Some(0o600),
                },
            )]),
            directories: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        }
    }

    /// Manifest-only refresh classifies from the count and digest pair, so a
    /// conversation entry without valid metadata may not exist at all.
    #[test]
    fn manifest_rejects_conversations_without_refresh_metadata() {
        let manifest = conversation_manifest();
        manifest.validate().unwrap();

        let mut unproven = manifest.clone();
        unproven
            .conversations
            .values_mut()
            .next()
            .unwrap()
            .lineage_status = ConversationLineageStatus::Unknown {
            reason_code: "missing-parent".to_string(),
        };
        assert!(unproven
            .validate()
            .unwrap_err()
            .contains("unproven lineage"));

        let mut empty = manifest.clone();
        empty
            .conversations
            .values_mut()
            .next()
            .unwrap()
            .message_count = 0;
        assert!(empty
            .validate()
            .unwrap_err()
            .contains("incomplete lineage metadata"));

        let mut malformed = manifest;
        malformed
            .conversations
            .values_mut()
            .next()
            .unwrap()
            .lineage_digest = "not-a-digest".to_string();
        assert!(malformed
            .validate()
            .unwrap_err()
            .contains("not a lowercase SHA-256"));
    }

    #[test]
    fn manifest_enforces_conversation_referential_integrity() {
        let manifest = conversation_manifest();
        manifest.validate().unwrap();

        let mut missing_file = manifest.clone();
        missing_file.files.clear();
        assert!(missing_file
            .validate()
            .unwrap_err()
            .contains("missing file"));

        let mut wrong_provider = manifest.clone();
        wrong_provider
            .conversations
            .values_mut()
            .next()
            .unwrap()
            .provider = Provider::Claude;
        assert!(wrong_provider
            .validate()
            .unwrap_err()
            .contains("non-canonical resource id"));

        let mut duplicate_transcript = manifest.clone();
        let thread = duplicate_transcript
            .conversations
            .values_mut()
            .next()
            .unwrap();
        let duplicate = thread.files[0].clone();
        thread.files.push(duplicate);
        assert!(duplicate_transcript
            .validate()
            .unwrap_err()
            .contains("repeats file"));

        let mut non_canonical_transcript = manifest.clone();
        non_canonical_transcript
            .conversations
            .values_mut()
            .next()
            .unwrap()
            .files[0]
            .logical_path = LogicalPath::parse("state/codex/sessions/thread-a.jsonl").unwrap();
        assert!(non_canonical_transcript
            .validate()
            .unwrap_err()
            .contains("non-canonical transcript path"));

        let mut self_fork = manifest.clone();
        {
            let thread = self_fork.conversations.values_mut().next().unwrap();
            thread.forked_from = Some(thread.thread_id.clone());
        }
        assert!(self_fork
            .validate()
            .unwrap_err()
            .contains("cannot fork from itself"));

        let mut cyclic_forks = manifest.clone();
        let first_id = cyclic_forks.conversations.keys().next().unwrap().clone();
        let second_id = ThreadId::parse("thread-11111111111111111111111111111111").unwrap();
        let second_resource_id = conversation_resource_id(Provider::Codex, &second_id).unwrap();
        let second_path = conversation_transcript_path(Provider::Codex, &second_id).unwrap();
        let mut second_thread = cyclic_forks.conversations.get(&first_id).unwrap().clone();
        second_thread.thread_id = second_id.clone();
        second_thread.resource_id = second_resource_id.clone();
        second_thread.files[0].logical_path = second_path.clone();
        second_thread.forked_from = Some(first_id.clone());
        cyclic_forks
            .conversations
            .get_mut(&first_id)
            .unwrap()
            .forked_from = Some(second_id.clone());
        let mut second_descriptor = cyclic_forks.resources.values().next().unwrap().clone();
        second_descriptor.resource_id = second_resource_id.clone();
        let mut second_file = cyclic_forks.files.values().next().unwrap().clone();
        second_file.resource_id = second_resource_id.clone();
        cyclic_forks.conversations.insert(second_id, second_thread);
        cyclic_forks
            .resources
            .insert(second_resource_id.clone(), second_descriptor);
        cyclic_forks.files.insert(second_path, second_file);
        cyclic_forks.recipe.entries.insert(
            second_resource_id.clone(),
            RecipeEntry {
                resource_id: second_resource_id,
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
        assert!(cyclic_forks
            .validate()
            .unwrap_err()
            .contains("ancestry contains a cycle"));

        let mut unsupported = manifest;
        unsupported.schema_version = BUNDLE_SCHEMA + 1;
        assert!(unsupported.validate().unwrap_err().contains("unsupported"));
    }
}
