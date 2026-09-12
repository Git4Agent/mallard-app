//! The storage-provider adapter contract.
//!
//! An adapter owns its safe descriptor and configuration fields, typed
//! configuration decoding, settings and secret validation, redacted
//! summaries, continuity coordinates, allowed local roots, connection
//! probing, and construction of an object store. Everything else — storage
//! IDs and names, drafts and commits, object keys, publication, digests,
//! listings, conditional deletion, and the managed clear — belongs to the
//! common storage layer.

use crate::storage_core::ObjectStore;
use mallard_domain::{
    BoundedConfigObject, FieldKey, SecretKey, StorageProviderConfig, StorageProviderKey,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;

/// How the setup form should render one safe field.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageFieldKind {
    Text,
    Url,
    Directory,
}

/// One safe, editable configuration field a provider declares.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageFieldDescriptor {
    pub field_id: FieldKey,
    pub label: String,
    pub immutable_on_edit: bool,
    pub kind: StorageFieldKind,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Documentation the field's help mark opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
    /// A value the form fills in from another field as it is typed. The
    /// adapter still validates the persisted value; this is convenience only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derive: Option<StorageFieldDerivation>,
}

/// A suggested storage name: capture group `group` of `pattern` matched
/// against the `source` field.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageNameSuggestion {
    pub source: FieldKey,
    pub pattern: String,
    pub group: u32,
}

/// One declarative extraction: when `source` matches `pattern` (a regular
/// expression), capture group `group` becomes this field's value; when it
/// does not match, the field clears.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageFieldDerivation {
    pub source: FieldKey,
    pub pattern: String,
    pub group: u32,
}

/// One secret a provider accepts. Its value never leaves the backend.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageSecretDescriptor {
    pub secret_id: SecretKey,
    pub label: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageLocalSetup {
    Wrangler,
    Dropbox,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageDiscoveryHelp {
    pub label: String,
    pub url: String,
}

/// A provider that can look in the signed-in account for destinations
/// Mallard already uses. Every sentence the New-storage card's Use-existing
/// door and lane print comes from here, so the page never names a provider.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageDiscoveryDescriptor {
    /// What one candidate is called: "bucket", "shared folder".
    pub unit: String,
    pub unit_plural: String,
    /// Where the look happens, for prose: "your Cloudflare account".
    pub place: String,
    /// The Create-new door's sentence.
    pub create_hint: String,
    /// The Use-existing door's sentence.
    pub existing_hint: String,
    /// Optional access guide beside the Use-existing choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_help: Option<StorageDiscoveryHelp>,
    /// The lane's verb: "Find in my account".
    pub find_label: String,
    /// What the look signs in with, and that it only reads.
    pub access_hint: String,
    /// Printed while the scan runs.
    pub scanning_hint: String,
    /// Printed after "Approve access to <name> in your browser." while the
    /// chosen destination's credential is being issued.
    pub prepare_hint: String,
}

/// Storage-wide clear is separate from normal revision-checked deletion.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageClearMode {
    RevisionChecked,
    Unconditional,
    Unavailable,
}

/// Serializable provider facts for setup and listing. The frontend renders
/// these; it never holds a fixed provider array.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageProviderDescriptor {
    pub key: StorageProviderKey,
    pub config_version: u32,
    pub display_name: String,
    pub description: String,
    /// The stem of the name a new draft starts with (`{stem} {n}`), so the
    /// form opens ready to save once its fields are in.
    pub default_name: String,
    /// Where a suggested storage name comes from while the form is filled:
    /// the form applies it only while the name is still the default or the
    /// previous suggestion, so a typed name always wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_suggestion: Option<StorageNameSuggestion>,
    /// Display order: lower ranks first, then key order. Never a provider
    /// name branch.
    pub display_rank: u32,
    /// A generic presentation hint (`cloud`, `drive`).
    pub icon: String,
    /// Deletion semantics for the storage-wide clear action.
    pub clear_mode: StorageClearMode,
    pub fields: Vec<StorageFieldDescriptor>,
    pub secrets: Vec<StorageSecretDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_secret_id: Option<SecretKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_setup: Option<StorageLocalSetup>,
    /// The provider's own setup documentation, offered beside the manual
    /// fields as a "Setup guide" link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_guide_url: Option<String>,
    /// The sentence introducing the manual fields; the form falls back to a
    /// generic one naming the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_setup_hint: Option<String>,
    /// Absent for a provider that cannot look in an account: the New-storage
    /// card then has no Use-existing door.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<StorageDiscoveryDescriptor>,
}

/// One safe, redacted fact about a configured storage.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageSummaryField {
    pub field_id: FieldKey,
    pub label: String,
    pub value: String,
    /// The value is a machine-local path the UI may reveal in Finder.
    pub is_path: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageAdapterErrorKind {
    Invalid,
    Remote,
    CleanupRequired,
    Internal,
}

/// A typed, sanitized adapter error. Messages are fixed text, at most
/// extended with allowlisted connection evidence (a route literal, a
/// status, a fixed error identifier, a request ID); settings, secrets,
/// destinations, SDK output, and filesystem values are never retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageAdapterError {
    kind: StorageAdapterErrorKind,
    code: &'static str,
    message: String,
}

impl StorageAdapterError {
    pub fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: StorageAdapterErrorKind::Invalid,
            code,
            message: message.into(),
        }
    }

    pub fn remote(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: StorageAdapterErrorKind::Remote,
            code,
            message: message.into(),
        }
    }

    pub fn cleanup_required(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: StorageAdapterErrorKind::CleanupRequired,
            code,
            message: message.into(),
        }
    }

    pub fn internal(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: StorageAdapterErrorKind::Internal,
            code,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> StorageAdapterErrorKind {
        self.kind
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StorageAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StorageAdapterError {}

/// The adapter must save recovery data before a remote mutation.
pub trait StorageProbeJournal {
    fn recovery(&self) -> &BoundedConfigObject;
    fn checkpoint(&mut self, recovery: BoundedConfigObject) -> Result<(), String>;
}

pub trait StorageProviderAdapter: Send + Sync {
    fn descriptor(&self) -> StorageProviderDescriptor;

    /// Structural validation of drafted settings with no I/O. Returns the
    /// canonical settings the draft should store.
    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> Result<BoundedConfigObject, StorageAdapterError>;

    /// Structural validation of a complete configuration: canonical settings
    /// plus every required secret. Still no I/O.
    fn validate_config(&self, config: &StorageProviderConfig) -> Result<(), StorageAdapterError>;

    /// Safe, redacted facts for listings and logs.
    fn summary(&self, config: &StorageProviderConfig) -> Result<Vec<StorageSummaryField>, String>;

    /// Canonical continuity coordinates. Never a credential or display name;
    /// the common layer hashes them together with the provider key.
    fn continuity_coordinates(&self, config: &StorageProviderConfig)
        -> Result<Vec<String>, String>;

    /// The machine-local directory this storage occupies, for isolation
    /// checks. Remote providers return `None`.
    fn local_root(&self, config: &StorageProviderConfig) -> Result<Option<PathBuf>, String>;

    /// Bounded, non-secret facts identifying the destination for the setup
    /// journal. Never a credential, token, HTTP material, or local path.
    fn journal_facts(&self, config: &StorageProviderConfig) -> Result<BoundedConfigObject, String>;

    /// Verify the connection and every capability the common layer relies
    /// on. Remote residue is scoped to `setup_id`.
    fn probe(
        &self,
        config: &StorageProviderConfig,
        setup_id: &str,
        runtime: &Handle,
        journal: &mut dyn StorageProbeJournal,
    ) -> Result<(), StorageAdapterError>;

    /// Remove the residue of one interrupted probe.
    fn reconcile_probe(
        &self,
        config: &StorageProviderConfig,
        setup_id: &str,
        runtime: &Handle,
        journal: &mut dyn StorageProbeJournal,
    ) -> Result<(), StorageAdapterError>;

    /// Render the declared setup assistant's instruction for one exact draft.


    /// Open the object store. Remote providers need the runtime handle.
    fn open(
        &self,
        config: &StorageProviderConfig,
        runtime: Option<Handle>,
    ) -> Result<Arc<dyn ObjectStore>, String>;
}
