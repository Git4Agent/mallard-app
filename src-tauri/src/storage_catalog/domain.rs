//! Scope-rowed Library resource domain: open catalog coordinates, the
//! portable resource envelope, schema-1 repositories with set manifests,
//! machine-local profiles, scopes, scope-storage links, apply targets,
//! claims, witnesses, the operations journal, immutable Push/Pull/Fork plans,
//! and their structural validation.
//!
//! Only structural validation lives here: key grammar, bounds, digests, and
//! document invariants. Nothing in this module resolves a coordinate to a
//! codec or adapter — that is registry validation in `registry` — so records
//! with coordinates this build does not register still load and stay inert.
//!
//! This module is deliberately independent of the project-sync domain. It
//! imports only storage-neutral identifiers, keys, bounded objects, and
//! shared digest validation from `mallard_domain`; it never uses bundle,
//! project, thread, or restore-plan types. Keep it free of Tauri and
//! filesystem concerns.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

pub(crate) use mallard_domain::sha256_hex;
use mallard_domain::{
    generated_named_id, hex_lower, update_length_prefixed, validate_absolute_clean_path,
    validate_bounded_key, validate_sha256, StorageId,
};
pub use mallard_domain::{
    BoundedConfigObject, BoundedPortableObject, CatalogFormatKey, CatalogProviderConfig,
    CatalogResourceKindKey, CatalogTargetKey, ProviderKey,
};

/// Portable catalog schema. Exactly 1; unknown versions block catalog work.
pub const CATALOG_SCHEMA: u32 = 1;

// Enforced limits applied consistently in validation, capture, storage reads,
// and tests. Format-specific limits live beside their codec.
pub const MAX_CATALOG_RESOURCES: usize = 512;
pub const MAX_CATALOG_FILES: usize = 8_192;
/// Structural per-file ceiling for every portable payload file; codecs
/// enforce their own tighter limits.
pub const MAX_CATALOG_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_REVISION_PAYLOAD_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_CATALOG_PROFILES: usize = 64;
pub const MAX_CATALOG_SCOPES: usize = 1_024;
pub const MAX_CATALOG_LINKS_PER_SCOPE: usize = 128;
pub const MAX_CATALOG_CLAIMS: usize = 4_096;
pub const MAX_CATALOG_WITNESSES: usize = 512;
pub const MAX_CATALOG_OPERATIONS: usize = 256;
pub const MAX_GENERATED_NAME_ATTEMPTS: u32 = 64;

fn generate_catalog_id(prefix: &str) -> Result<String, String> {
    generated_named_id(prefix).map_err(|error| {
        error
            .strip_prefix("random id: ")
            .map_or(error.clone(), |detail| {
                format!("gather randomness: {detail}")
            })
    })
}

fn length_prefixed_digest(domain_tag: &[u8], fields: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, domain_tag);
    for field in fields {
        update_length_prefixed(&mut hasher, field);
    }
    hex_lower(&hasher.finalize())
}

macro_rules! catalog_id {
    ($name:ident, $prefix:literal, $hex_len:literal, $label:literal) => {
        #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                let invalid = || format!("invalid {}: '{}'", $label, value.escape_debug());
                let Some(hex) = value.strip_prefix(concat!($prefix, "-")) else {
                    return Err(invalid());
                };
                if hex.len() != $hex_len
                    || !hex
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err(invalid());
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(value: String) -> Result<Self, String> {
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

catalog_id!(CatalogCommitId, "commit", 24, "catalog commit id");
catalog_id!(CatalogResourceId, "resource", 64, "catalog resource id");
catalog_id!(CatalogPlanId, "catplan", 24, "catalog plan id");
catalog_id!(CatalogPlanActionId, "action", 24, "catalog plan action id");
catalog_id!(CatalogOperationId, "catop", 24, "catalog operation id");
catalog_id!(
    CatalogResourceRepositoryId,
    "library",
    32,
    "catalog resource repository id"
);
catalog_id!(CatalogLinkId, "catlink", 24, "catalog link id");
catalog_id!(CatalogScopeId, "catscope", 24, "catalog scope id");
catalog_id!(
    LocalCatalogProfileId,
    "catprofile",
    24,
    "catalog profile id"
);

impl CatalogCommitId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(generate_catalog_id("commit")?)
    }
}

impl CatalogPlanId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(generate_catalog_id("catplan")?)
    }
}

impl CatalogPlanActionId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(generate_catalog_id("action")?)
    }
}

impl CatalogOperationId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(generate_catalog_id("catop")?)
    }
}

impl CatalogLinkId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(generate_catalog_id("catlink")?)
    }
}

impl CatalogScopeId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(generate_catalog_id("catscope")?)
    }
}

impl LocalCatalogProfileId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(generate_catalog_id("catprofile")?)
    }
}

// ---------------------------------------------------------------------------
// Open coordinates
// ---------------------------------------------------------------------------

/// The four independent Library axes that address one portable resource
/// shape. Every axis is an open key; `format_version` versions the portable
/// shape of `format`. Structurally any well-formed coordinate parses — only
/// the registry decides whether this build can operate on it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct CatalogCoordinate {
    pub provider: ProviderKey,
    pub resource_kind: CatalogResourceKindKey,
    pub format: CatalogFormatKey,
    pub format_version: u32,
}

impl CatalogCoordinate {
    pub fn new(
        provider: impl Into<String>,
        resource_kind: impl Into<String>,
        format: impl Into<String>,
        format_version: u32,
    ) -> Result<Self, String> {
        let coordinate = Self {
            provider: ProviderKey::parse(provider)?,
            resource_kind: CatalogResourceKindKey::parse(resource_kind)?,
            format: CatalogFormatKey::parse(format)?,
            format_version,
        };
        coordinate.validate()?;
        Ok(coordinate)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format_version == 0 {
            return Err(format!(
                "catalog coordinate {} declares format version 0",
                self
            ));
        }
        Ok(())
    }

    /// Same provider, kind, and format — only the version may differ.
    pub fn shares_format(&self, other: &Self) -> bool {
        self.provider == other.provider
            && self.resource_kind == other.resource_kind
            && self.format == other.format
    }
}

impl fmt::Display for CatalogCoordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}/{}@{}",
            self.provider, self.resource_kind, self.format, self.format_version
        )
    }
}

impl CatalogResourceRepositoryId {
    /// The one repository a coordinate maps to, on every machine. Deriving
    /// instead of minting is what makes two replicas that push resources of
    /// the same coordinate converge on the same repository without ever
    /// seeing each other first. The derivation takes exactly the provider,
    /// kind, format, and format version: never a profile, path, target,
    /// storage, or credential.
    pub fn for_coordinate(coordinate: &CatalogCoordinate) -> Self {
        let digest = length_prefixed_digest(
            b"mallard-library-repository-v1",
            &[
                coordinate.provider.as_str().as_bytes(),
                coordinate.resource_kind.as_str().as_bytes(),
                coordinate.format.as_str().as_bytes(),
                &coordinate.format_version.to_be_bytes(),
            ],
        );
        Self::parse(format!("library-{}", &digest[..32]))
            .expect("derived catalog repository ids are always valid")
    }

    /// The one item stream a resource maps to on a storage that holds one
    /// item per repository (Mallard Cloud): the coordinate's derivation
    /// extended by the resource id, under its own domain tag, so an item
    /// stream can never collide with a coordinate repository. The service
    /// derives the same id (`STORE_API.md` section 6).
    pub fn for_item(coordinate: &CatalogCoordinate, resource_id: &CatalogResourceId) -> Self {
        let digest = length_prefixed_digest(
            b"mallard-library-item-v1",
            &[
                coordinate.provider.as_str().as_bytes(),
                coordinate.resource_kind.as_str().as_bytes(),
                coordinate.format.as_str().as_bytes(),
                &coordinate.format_version.to_be_bytes(),
                resource_id.as_str().as_bytes(),
            ],
        );
        Self::parse(format!("library-{}", &digest[..32]))
            .expect("derived catalog repository ids are always valid")
    }
}

/// Backend-supplied labels for one coordinate, served beside the raw keys so
/// the frontend never holds a provider, kind, or format label table.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogCoordinateLabels {
    pub provider_label: String,
    pub kind_label: String,
    pub format_label: String,
}

// ---------------------------------------------------------------------------
// Validators
// ---------------------------------------------------------------------------

pub fn validate_display_text(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(format!("invalid {}: '{}'", label, value.escape_debug()));
    }
    Ok(())
}

/// Directory-member and effective-name rule: ASCII alphanumeric start,
/// `[A-Za-z0-9._-]` body, bounded, never a dot path, no trailing dot.
pub fn validate_component_name(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    let valid_start = value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric());
    if !valid_start
        || value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || value == "."
        || value == ".."
        || value.ends_with('.')
    {
        return Err(format!("invalid {}: '{}'", label, value.escape_debug()));
    }
    Ok(())
}

/// A portable root-class token inside a resource identity (`provider-home`,
/// `shared-user`): the stable key grammar.
pub fn validate_root_class(label: &str, value: &str) -> Result<(), String> {
    validate_bounded_key(label, value)
}

/// The member a resource occupies inside its target root: a directory name
/// for directory resources, a native identifier (`tools@acme`) for
/// provider-inventory resources. Bounded, never a path, never a dot path.
pub fn validate_target_member(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 512
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(format!("invalid {}: '{}'", label, value.escape_debug()));
    }
    Ok(())
}

/// A resource-relative payload path: bounded depth and length, no traversal,
/// no nested VCS metadata, portable component names.
pub fn validate_portable_relative_path(value: &str) -> Result<(), String> {
    let invalid = || format!("invalid portable payload path: '{}'", value.escape_debug());
    if value.is_empty()
        || value.len() > 1024
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    let components: Vec<&str> = value.split('/').collect();
    if components.len() > 32 {
        return Err(invalid());
    }
    for component in &components {
        if component.is_empty()
            || *component == "."
            || *component == ".."
            || component.len() > 255
            || component.eq_ignore_ascii_case(".git")
        {
            return Err(invalid());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Portable resources
// ---------------------------------------------------------------------------

/// One immutable payload file: its resource-relative path, exact size,
/// SHA-256, and safe portable mode. The object key is content-addressed and
/// derived, never stored.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogPortableFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub mode: u32,
}

impl CatalogPortableFile {
    pub fn object_key(&self) -> String {
        format!("_objects/{}", self.sha256)
    }

    pub fn validate(&self, label: &str, max_bytes: u64) -> Result<(), String> {
        validate_portable_relative_path(&self.path)?;
        validate_sha256(label, &self.sha256)?;
        if self.size > max_bytes {
            return Err(format!("{} exceeds its size limit", label));
        }
        if !matches!(self.mode, 0o644 | 0o755) {
            return Err(format!(
                "{} has unsupported portable mode {:o}",
                label, self.mode
            ));
        }
        Ok(())
    }
}

/// The open cloud resource envelope. Provider-specific meaning lives in the
/// bounded `identity` and `descriptor` objects, which a registered format
/// codec decodes into typed structures for its exact coordinate. Structural
/// validation proves ids, digests, bounds, and file entries; an envelope
/// whose coordinate this build does not register still parses, is
/// preserved, and is inert.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogResourceEnvelope {
    pub resource_id: CatalogResourceId,
    pub provider: ProviderKey,
    pub resource_kind: CatalogResourceKindKey,
    pub format: CatalogFormatKey,
    pub format_version: u32,
    pub identity: BoundedPortableObject,
    pub descriptor: BoundedPortableObject,
    pub content_sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<CatalogPortableFile>,
}

/// Resource identity is stable across content updates: it is derived from
/// the coordinate plus the canonical identity object, with every component
/// length-delimited and domain-tagged. The format version participates so
/// two versions of one format never share a resource, even in one directory.
pub fn derive_resource_id(
    coordinate: &CatalogCoordinate,
    identity: &BoundedPortableObject,
) -> CatalogResourceId {
    let digest = length_prefixed_digest(
        b"mallard-catalog-resource-v1",
        &[
            coordinate.provider.as_str().as_bytes(),
            coordinate.resource_kind.as_str().as_bytes(),
            coordinate.format.as_str().as_bytes(),
            &coordinate.format_version.to_be_bytes(),
            &identity.canonical_json(),
        ],
    );
    CatalogResourceId::parse(format!("resource-{}", digest))
        .expect("catalog resource ids derived from digests are always valid")
}

impl CatalogResourceEnvelope {
    /// Build an envelope with its derived id and content digest. Files are
    /// canonicalized to path order.
    pub fn new(
        coordinate: &CatalogCoordinate,
        identity: BoundedPortableObject,
        descriptor: BoundedPortableObject,
        mut files: Vec<CatalogPortableFile>,
    ) -> Self {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut envelope = Self {
            resource_id: derive_resource_id(coordinate, &identity),
            provider: coordinate.provider.clone(),
            resource_kind: coordinate.resource_kind.clone(),
            format: coordinate.format.clone(),
            format_version: coordinate.format_version,
            identity,
            descriptor,
            content_sha256: String::new(),
            files,
        };
        envelope.content_sha256 = envelope.compute_content_sha256();
        envelope
    }

    pub fn coordinate(&self) -> CatalogCoordinate {
        CatalogCoordinate {
            provider: self.provider.clone(),
            resource_kind: self.resource_kind.clone(),
            format: self.format.clone(),
            format_version: self.format_version,
        }
    }

    /// Content digest over the coordinate, identity, descriptor, and every
    /// file's path, digest, size, and mode. Machine-local observations never
    /// enter it.
    pub fn compute_content_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        update_length_prefixed(&mut hasher, b"mallard-catalog-content-v1");
        update_length_prefixed(&mut hasher, self.provider.as_str().as_bytes());
        update_length_prefixed(&mut hasher, self.resource_kind.as_str().as_bytes());
        update_length_prefixed(&mut hasher, self.format.as_str().as_bytes());
        hasher.update(self.format_version.to_be_bytes());
        update_length_prefixed(&mut hasher, &self.identity.canonical_json());
        update_length_prefixed(&mut hasher, &self.descriptor.canonical_json());
        for file in &self.files {
            update_length_prefixed(&mut hasher, file.path.as_bytes());
            update_length_prefixed(&mut hasher, file.sha256.as_bytes());
            hasher.update(file.size.to_be_bytes());
            hasher.update(file.mode.to_be_bytes());
        }
        hex_lower(&hasher.finalize())
    }

    /// Structural validation: coordinate grammar, non-empty identity, id and
    /// digest derivations, and bounded canonical file entries.
    pub fn validate(&self) -> Result<(), String> {
        self.coordinate().validate()?;
        if self.identity.is_empty() {
            return Err(format!(
                "catalog resource '{}' has an empty identity",
                self.resource_id
            ));
        }
        if self.resource_id != derive_resource_id(&self.coordinate(), &self.identity) {
            return Err(format!(
                "catalog resource '{}' id does not match its identity derivation",
                self.resource_id
            ));
        }
        if self.files.len() > MAX_CATALOG_FILES {
            return Err(format!(
                "catalog resource '{}' exceeds the {} file limit",
                self.resource_id, MAX_CATALOG_FILES
            ));
        }
        let mut folded = BTreeSet::new();
        let mut previous: Option<&str> = None;
        let mut total: u64 = 0;
        for file in &self.files {
            file.validate(
                &format!("catalog payload file '{}'", file.path),
                MAX_CATALOG_FILE_BYTES,
            )?;
            if previous.is_some_and(|previous| previous >= file.path.as_str()) {
                return Err(format!(
                    "catalog resource '{}' file entries are not in canonical order",
                    self.resource_id
                ));
            }
            previous = Some(file.path.as_str());
            if !folded.insert(file.path.to_lowercase()) {
                return Err(format!(
                    "catalog resource '{}' has a case-folded path collision at '{}'",
                    self.resource_id, file.path
                ));
            }
            total = total.saturating_add(file.size);
        }
        if total > MAX_REVISION_PAYLOAD_BYTES {
            return Err(format!(
                "catalog resource '{}' exceeds its payload limit",
                self.resource_id
            ));
        }
        validate_sha256("catalog resource content digest", &self.content_sha256)?;
        if self.content_sha256 != self.compute_content_sha256() {
            return Err(format!(
                "catalog resource '{}' content digest does not match its envelope",
                self.resource_id
            ));
        }
        Ok(())
    }

    pub fn file(&self, path: &str) -> Option<&CatalogPortableFile> {
        self.files.iter().find(|file| file.path == path)
    }

    /// Total portable payload bytes owned by this resource.
    pub fn payload_bytes(&self) -> u64 {
        self.files
            .iter()
            .fold(0_u64, |total, file| total.saturating_add(file.size))
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Every content-addressed payload this resource references, by digest.
    pub fn referenced_payloads(&self) -> BTreeMap<String, u64> {
        self.files
            .iter()
            .map(|file| (file.sha256.clone(), file.size))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Portable catalog documents
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogCapturedWith {
    pub app: String,
    pub version: String,
}

impl CatalogCapturedWith {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("captured-with app", &self.app, 128)?;
        validate_display_text("captured-with version", &self.version, 64)
    }
}

/// Privacy-preserving publication origin, HMAC-derived per Library resource
/// repository with domain separation so tokens never correlate with project
/// bundle origin tokens for the same host.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortableCatalogOrigin {
    pub origin_host_token: String,
    pub origin_replica_token: String,
}

impl PortableCatalogOrigin {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("catalog origin host token", &self.origin_host_token)?;
        validate_sha256("catalog origin replica token", &self.origin_replica_token)
    }
}

fn derive_catalog_token(host_id: &str, fields: &[&[u8]]) -> Result<String, String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(host_id.as_bytes())
        .map_err(|_| "derive catalog origin token".to_string())?;
    for field in fields {
        let length = u64::try_from(field.len())
            .map_err(|_| "catalog origin token field too long".to_string())?;
        mac.update(&length.to_be_bytes());
        mac.update(field);
    }
    Ok(hex_lower(&mac.finalize().into_bytes()))
}

pub fn derive_library_origin(
    host_id: &str,
    replica_id: &str,
    repository_id: &CatalogResourceRepositoryId,
) -> Result<PortableCatalogOrigin, String> {
    Ok(PortableCatalogOrigin {
        origin_host_token: derive_catalog_token(
            host_id,
            &[
                b"mallard-library-origin-host-v1",
                repository_id.as_str().as_bytes(),
            ],
        )?,
        origin_replica_token: derive_catalog_token(
            host_id,
            &[
                b"mallard-library-origin-replica-v1",
                repository_id.as_str().as_bytes(),
                replica_id.as_bytes(),
            ],
        )?,
    })
}

// ---------------------------------------------------------------------------
// Per-coordinate Library repositories
// ---------------------------------------------------------------------------

/// Immutable discovery metadata for one repository. It freezes exactly the
/// coordinate the repository id derives from and contains no local
/// profile/path/target/storage identity. It is never trusted in place of the
/// verified publication and manifest. Every field derives from the
/// coordinate, so every writer writes identical bytes and a plain write of
/// its key is safe.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryResourceTag {
    pub schema_version: u32,
    pub repository_id: CatalogResourceRepositoryId,
    pub provider: ProviderKey,
    pub resource_kind: CatalogResourceKindKey,
    pub format: CatalogFormatKey,
    pub format_version: u32,
    /// Present on an item stream (one item per repository): the resource
    /// the stream is derived from. Absent on a coordinate repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<CatalogResourceId>,
}

impl LibraryResourceTag {
    pub fn new(coordinate: &CatalogCoordinate, repository_id: CatalogResourceRepositoryId) -> Self {
        Self {
            schema_version: CATALOG_SCHEMA,
            repository_id,
            provider: coordinate.provider.clone(),
            resource_kind: coordinate.resource_kind.clone(),
            format: coordinate.format.clone(),
            format_version: coordinate.format_version,
            resource_id: None,
        }
    }

    /// The tag of one item's stream.
    pub fn for_item(coordinate: &CatalogCoordinate, resource_id: &CatalogResourceId) -> Self {
        Self {
            schema_version: CATALOG_SCHEMA,
            repository_id: CatalogResourceRepositoryId::for_item(coordinate, resource_id),
            provider: coordinate.provider.clone(),
            resource_kind: coordinate.resource_kind.clone(),
            format: coordinate.format.clone(),
            format_version: coordinate.format_version,
            resource_id: Some(resource_id.clone()),
        }
    }

    pub fn coordinate(&self) -> CatalogCoordinate {
        CatalogCoordinate {
            provider: self.provider.clone(),
            resource_kind: self.resource_kind.clone(),
            format: self.format.clone(),
            format_version: self.format_version,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported library tag schema {} (expected {})",
                self.schema_version, CATALOG_SCHEMA
            ));
        }
        self.coordinate().validate()?;
        let derived = match &self.resource_id {
            Some(resource_id) => {
                CatalogResourceRepositoryId::for_item(&self.coordinate(), resource_id)
            }
            None => CatalogResourceRepositoryId::for_coordinate(&self.coordinate()),
        };
        if self.repository_id != derived {
            return Err(
                "library repository id does not match its coordinate derivation".to_string(),
            );
        }
        Ok(())
    }
}

/// One resource inside a repository's published set.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublishedResourceEntry {
    pub resource: CatalogResourceEnvelope,
    pub updated_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryResourceManifest {
    pub schema_version: u32,
    pub repository_id: CatalogResourceRepositoryId,
    pub revision: u64,
    pub commit_id: CatalogCommitId,
    pub updated_at: u64,
    pub captured_with: CatalogCapturedWith,
    /// The last publisher; per-entry origins are deliberately not tracked.
    pub published_by_origin: PortableCatalogOrigin,
    pub resources: BTreeMap<CatalogResourceId, PublishedResourceEntry>,
}

impl LibraryResourceManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported library manifest schema {} (expected {})",
                self.schema_version, CATALOG_SCHEMA
            ));
        }
        if self.revision == 0 || self.updated_at == 0 {
            return Err("library manifest requires a version and timestamp".to_string());
        }
        self.captured_with.validate()?;
        self.published_by_origin.validate()?;
        if self.resources.len() > MAX_CATALOG_RESOURCES {
            return Err(format!(
                "library manifest exceeds the {} resource limit",
                MAX_CATALOG_RESOURCES
            ));
        }
        for (resource_id, entry) in &self.resources {
            entry.resource.validate()?;
            if resource_id != &entry.resource.resource_id {
                return Err("library manifest entry is keyed by a foreign resource id".to_string());
            }
            if entry.updated_at == 0 {
                return Err("library manifest entry requires a timestamp".to_string());
            }
            // Every entry must belong to the repository's own derivation, so
            // one manifest can never mix coordinates: the coordinate's, or,
            // for the one entry of an item stream, the item's.
            let coordinate = entry.resource.coordinate();
            let belongs = CatalogResourceRepositoryId::for_coordinate(&coordinate)
                == self.repository_id
                || (self.resources.len() == 1
                    && CatalogResourceRepositoryId::for_item(&coordinate, resource_id)
                        == self.repository_id);
            if !belongs {
                return Err("library manifest entry does not belong to this repository".to_string());
            }
        }
        Ok(())
    }

    /// The published set as plain resources.
    #[cfg(test)]
    pub fn resource_map(&self) -> BTreeMap<CatalogResourceId, CatalogResourceEnvelope> {
        self.resources
            .iter()
            .map(|(resource_id, entry)| (resource_id.clone(), entry.resource.clone()))
            .collect()
    }

    /// Whether this manifest is one item stream's: exactly one entry, on the
    /// repository derived from it.
    pub fn is_item_stream(&self) -> bool {
        self.resources.len() == 1
            && self.resources.iter().all(|(resource_id, entry)| {
                CatalogResourceRepositoryId::for_item(&entry.resource.coordinate(), resource_id)
                    == self.repository_id
            })
    }
}

/// The in-memory view of the selected publication record, field for field.
/// It is never a stored object.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryResourceHead {
    pub schema_version: u32,
    pub repository_id: CatalogResourceRepositoryId,
    pub revision: u64,
    pub commit_id: CatalogCommitId,
    pub manifest_key: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub updated_at: u64,
}

pub fn library_manifest_relative_key(revision: u64, commit_id: &CatalogCommitId) -> String {
    format!("_manifests/{}-{}.json", revision, commit_id)
}

impl LibraryResourceHead {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported library head schema {} (expected {})",
                self.schema_version, CATALOG_SCHEMA
            ));
        }
        if self.revision == 0 || self.updated_at == 0 {
            return Err("library head requires a version and timestamp".to_string());
        }
        validate_sha256("library head manifest digest", &self.manifest_sha256)?;
        if self.manifest_key != library_manifest_relative_key(self.revision, &self.commit_id) {
            return Err("library head manifest key does not match its coordinates".to_string());
        }
        Ok(())
    }
}

/// The publication record of one repository: written last, and the only
/// thing that publishes a set. Its object name repeats its graph facts and
/// carries the digest of these exact bytes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryResourceCommit {
    pub schema_version: u32,
    pub repository_id: CatalogResourceRepositoryId,
    pub revision: u64,
    pub commit_id: CatalogCommitId,
    /// The parent's token. Both halves are absent for a first publication
    /// and both present otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_commit_id: Option<CatalogCommitId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_record_sha256: Option<String>,
    /// UTC Unix milliseconds, taken when the record is built. It ranks
    /// siblings and proves nothing about arrival order.
    pub created_at_ms: u64,
    pub manifest_key: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub published_by_origin: PortableCatalogOrigin,
}

impl LibraryResourceCommit {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA || self.revision == 0 || self.manifest_size == 0 {
            return Err("invalid library commit coordinates".to_string());
        }
        if self.previous_commit_id.is_some() != self.previous_record_sha256.is_some()
            || self.previous_commit_id.is_some() != (self.revision > 1)
        {
            return Err("library commit names half a parent or the wrong depth".to_string());
        }
        if let Some(previous) = &self.previous_record_sha256 {
            validate_sha256("library commit parent record digest", previous)?;
        }
        validate_sha256("library commit manifest digest", &self.manifest_sha256)?;
        if self.manifest_key != library_manifest_relative_key(self.revision, &self.commit_id) {
            return Err("library commit manifest key does not match its coordinates".to_string());
        }
        self.published_by_origin.validate()
    }

    /// The head view every reader of the selected publication sees.
    pub fn head(&self) -> LibraryResourceHead {
        LibraryResourceHead {
            schema_version: self.schema_version,
            repository_id: self.repository_id.clone(),
            revision: self.revision,
            commit_id: self.commit_id.clone(),
            manifest_key: self.manifest_key.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            manifest_size: self.manifest_size,
            updated_at: (self.created_at_ms / 1000).max(1),
        }
    }
}

/// Machine-local accepted coordinate for one scope/storage link.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryHeadCoordinate {
    pub repository_id: CatalogResourceRepositoryId,
    pub revision: u64,
    pub commit_id: CatalogCommitId,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub storage_identity_sha256: String,
}

impl LibraryHeadCoordinate {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision == 0 {
            return Err("accepted library head requires a version".to_string());
        }
        validate_sha256("accepted library manifest", &self.manifest_sha256)?;
        validate_sha256(
            "accepted library storage identity",
            &self.storage_identity_sha256,
        )
    }
}

// ---------------------------------------------------------------------------
// Machine-local state
// ---------------------------------------------------------------------------

/// Machine-local identity of a storage connection, pinned into accepted heads
/// so a silently swapped destination cannot masquerade as catalog continuity.
pub fn storage_connection_identity_sha256(kind: &str, coordinates: &[&str]) -> String {
    let mut fields: Vec<&[u8]> = vec![kind.as_bytes()];
    for coordinate in coordinates {
        fields.push(coordinate.as_bytes());
    }
    length_prefixed_digest(b"mallard-catalog-storage-identity-v1", &fields)
}

/// Machine-local catalog profile. This is a catalog-owned record; it does not
/// widen or reuse the project-sync provider profile. The provider's own
/// versioned settings live in `provider_config`; current providers use
/// version 1 with no settings.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalCatalogProfile {
    pub profile_id: LocalCatalogProfileId,
    pub provider: ProviderKey,
    pub display_name: String,
    pub configured_home: String,
    pub provider_config: CatalogProviderConfig,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl LocalCatalogProfile {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("catalog profile name", &self.display_name, 128)?;
        validate_absolute_clean_path("catalog profile home", &self.configured_home)?;
        self.provider_config.validate()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogProfilesDocument {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub profiles: Vec<LocalCatalogProfile>,
}

impl Default for CatalogProfilesDocument {
    fn default() -> Self {
        Self {
            schema: CATALOG_SCHEMA,
            revision: 0,
            profiles: Vec::new(),
        }
    }
}

impl CatalogProfilesDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog profiles schema {} (expected {})",
                self.schema, CATALOG_SCHEMA
            ));
        }
        if self.profiles.len() > MAX_CATALOG_PROFILES {
            return Err(format!(
                "catalog profiles exceed the {} entry limit",
                MAX_CATALOG_PROFILES
            ));
        }
        let mut ids = BTreeSet::new();
        for profile in &self.profiles {
            profile.validate()?;
            if !ids.insert(profile.profile_id.clone()) {
                return Err(format!(
                    "duplicate catalog profile '{}'",
                    profile.profile_id
                ));
            }
        }
        Ok(())
    }

    pub fn profile(&self, profile_id: &LocalCatalogProfileId) -> Option<&LocalCatalogProfile> {
        self.profiles
            .iter()
            .find(|profile| &profile.profile_id == profile_id)
    }
}

/// One user-added Catalog menu row: every resource of one coordinate under
/// one provider profile. Carries no display name — the label always derives
/// from the registered capability plus the profile's current name — and no
/// storage, because the scope links to storages through `CatalogScopeLink`
/// records. A scope whose coordinate this build does not register loads,
/// lists as unavailable, and cannot be operated on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScope {
    pub scope_id: CatalogScopeId,
    pub provider: ProviderKey,
    pub profile_id: LocalCatalogProfileId,
    pub resource_kind: CatalogResourceKindKey,
    pub format: CatalogFormatKey,
    pub format_version: u32,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl CatalogScope {
    pub fn coordinate(&self) -> CatalogCoordinate {
        CatalogCoordinate {
            provider: self.provider.clone(),
            resource_kind: self.resource_kind.clone(),
            format: self.format.clone(),
            format_version: self.format_version,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.coordinate().validate()?;
        if self.revision == 0 || self.created_at == 0 || self.created_at > self.updated_at {
            return Err(format!(
                "catalog scope '{}' has invalid version or timestamps",
                self.scope_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScopesDocument {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub scopes: Vec<CatalogScope>,
}

impl Default for CatalogScopesDocument {
    fn default() -> Self {
        Self {
            schema: CATALOG_SCHEMA,
            revision: 0,
            scopes: Vec::new(),
        }
    }
}

impl CatalogScopesDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog scopes schema {} (expected {})",
                self.schema, CATALOG_SCHEMA
            ));
        }
        if self.scopes.len() > MAX_CATALOG_SCOPES {
            return Err(format!(
                "catalog scopes exceed the {} entry limit",
                MAX_CATALOG_SCOPES
            ));
        }
        let mut ids = BTreeSet::new();
        let mut rows = BTreeSet::new();
        for scope in &self.scopes {
            scope.validate()?;
            if !ids.insert(scope.scope_id.clone()) {
                return Err(format!("duplicate catalog scope '{}'", scope.scope_id));
            }
            if !rows.insert((scope.profile_id.clone(), scope.coordinate())) {
                return Err(format!(
                    "catalog scope '{}' duplicates another row for its profile and coordinate",
                    scope.scope_id
                ));
            }
        }
        Ok(())
    }

    pub fn scope(&self, scope_id: &CatalogScopeId) -> Option<&CatalogScope> {
        self.scopes.iter().find(|scope| &scope.scope_id == scope_id)
    }
}

// ---------------------------------------------------------------------------
// Scope-storage links
// ---------------------------------------------------------------------------

/// One scope's connection to one storage: the repository it reads, the
/// continuity pin for that repository's head, and the per-resource accepted
/// content bases the three-way comparison runs against.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScopeLink {
    pub link_id: CatalogLinkId,
    pub scope_id: CatalogScopeId,
    pub storage_id: StorageId,
    pub repository_id: CatalogResourceRepositoryId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_head: Option<LibraryHeadCoordinate>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accepted_resources: BTreeMap<CatalogResourceId, String>,
    /// On a storage that holds one item per repository (Mallard Cloud):
    /// the accepted head of each item's own stream, the per-item form of
    /// `accepted_head`. Empty on a storage with one repository per
    /// coordinate.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accepted_items: BTreeMap<CatalogResourceId, LibraryHeadCoordinate>,
    /// Exact Cloud repositories selected for these items. Machine-local only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cloud_repositories: BTreeMap<CatalogResourceId, String>,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl CatalogScopeLink {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(head) = &self.accepted_head {
            head.validate()?;
            if head.repository_id != self.repository_id {
                return Err(format!(
                    "catalog scope link '{}' accepted a different repository",
                    self.link_id
                ));
            }
        }
        if self.cloud_repositories.len() > MAX_CATALOG_RESOURCES {
            return Err("too many Cloud repository bindings".to_string());
        }
        for id in self.cloud_repositories.values() {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                return Err("invalid Cloud repository binding".to_string());
            }
        }
        if self.accepted_items.len() > MAX_CATALOG_RESOURCES {
            return Err(format!(
                "catalog scope link '{}' exceeds the {} accepted-item limit",
                self.link_id, MAX_CATALOG_RESOURCES
            ));
        }
        for head in self.accepted_items.values() {
            head.validate()?;
        }
        if self.accepted_resources.len() > MAX_CATALOG_RESOURCES {
            return Err(format!(
                "catalog scope link '{}' exceeds the {} accepted-resource limit",
                self.link_id, MAX_CATALOG_RESOURCES
            ));
        }
        for content in self.accepted_resources.values() {
            validate_sha256("accepted catalog resource content", content)?;
        }
        if self.revision == 0 || self.created_at == 0 || self.created_at > self.updated_at {
            return Err(format!(
                "catalog scope link '{}' has invalid version or timestamps",
                self.link_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScopeLinksDocument {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub links: Vec<CatalogScopeLink>,
}

impl Default for CatalogScopeLinksDocument {
    fn default() -> Self {
        Self {
            schema: CATALOG_SCHEMA,
            revision: 0,
            links: Vec::new(),
        }
    }
}

impl CatalogScopeLinksDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog scope links schema {} (expected {})",
                self.schema, CATALOG_SCHEMA
            ));
        }
        if self.links.len() > MAX_CATALOG_SCOPES * MAX_CATALOG_LINKS_PER_SCOPE {
            return Err("catalog scope links exceed their limit".to_string());
        }
        let mut link_ids = BTreeSet::new();
        let mut coordinates = BTreeSet::new();
        for link in &self.links {
            link.validate()?;
            if !link_ids.insert(link.link_id.clone()) {
                return Err(format!("duplicate catalog scope link '{}'", link.link_id));
            }
            if !coordinates.insert((link.scope_id.clone(), link.storage_id.clone())) {
                return Err(format!(
                    "catalog scope '{}' already has a link on storage '{}'",
                    link.scope_id, link.storage_id
                ));
            }
        }
        Ok(())
    }

    pub fn link(&self, link_id: &CatalogLinkId) -> Option<&CatalogScopeLink> {
        self.links.iter().find(|link| &link.link_id == link_id)
    }

    pub fn link_for(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Option<&CatalogScopeLink> {
        self.links
            .iter()
            .find(|link| &link.scope_id == scope_id && &link.storage_id == storage_id)
    }

    pub fn links_for<'a>(
        &'a self,
        scope_id: &'a CatalogScopeId,
    ) -> impl Iterator<Item = &'a CatalogScopeLink> + 'a {
        self.links
            .iter()
            .filter(move |link| &link.scope_id == scope_id)
    }
}

// ---------------------------------------------------------------------------
// Machine-local apply targets
// ---------------------------------------------------------------------------

/// The typed identity every target this build operates on carries: the
/// canonical directory the target root occupies (a skills root, a pets root,
/// or the provider home for native resources), the adapter-declared portable
/// class of that root, and the member the resource occupies inside it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogTargetIdentity {
    pub canonical_root: String,
    pub root_class: String,
    pub member: String,
}

impl CatalogTargetIdentity {
    pub fn validate(&self) -> Result<(), String> {
        validate_absolute_clean_path("catalog target root", &self.canonical_root)?;
        validate_root_class("catalog target root class", &self.root_class)?;
        validate_target_member("catalog target member", &self.member)
    }
}

/// Canonical machine-local target of one resource, used by claims, journals,
/// witnesses, plans, and locks. The identity is a bounded machine-local
/// object — it carries canonical absolute roots and must never enter cloud
/// data, frontend DTOs, logs, or receipts. Structural validation keeps a
/// target with a shape this build does not decode loadable and inert;
/// operating on it decodes the typed identity.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogTargetConfig {
    pub provider: ProviderKey,
    pub target: CatalogTargetKey,
    pub identity: BoundedConfigObject,
}

impl CatalogTargetConfig {
    pub fn new(
        provider: ProviderKey,
        target: CatalogTargetKey,
        identity: &CatalogTargetIdentity,
    ) -> Result<Self, String> {
        identity.validate()?;
        Ok(Self {
            provider,
            target,
            identity: BoundedConfigObject::encode(identity)?,
        })
    }

    /// Structural validation only: the keys and the bounded object are
    /// already validated by their types; an empty identity is the one
    /// unrepresentable shape.
    pub fn validate(&self) -> Result<(), String> {
        if self.identity.is_empty() {
            return Err(format!(
                "catalog target '{}' has an empty identity",
                self.target
            ));
        }
        Ok(())
    }

    /// The typed identity, for operating on this target.
    pub fn decode(&self) -> Result<CatalogTargetIdentity, String> {
        let identity: CatalogTargetIdentity = self
            .identity
            .decode()
            .map_err(|error| format!("catalog target '{}' identity: {}", self.target, error))?;
        identity.validate()?;
        Ok(identity)
    }

    /// The same root with another member: the target a preservation copy or
    /// Fork copy occupies.
    pub fn with_member(&self, member: &str) -> Result<Self, String> {
        let mut identity = self.decode()?;
        identity.member = member.to_string();
        Self::new(self.provider.clone(), self.target.clone(), &identity)
    }
}

/// One verified per-target receipt and cross-storage claim edge. The same
/// target and digest can share claims across storages; a different digest
/// requires explicit conflict resolution naming every claiming storage.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogClaim {
    pub target: CatalogTargetConfig,
    pub coordinate: CatalogCoordinate,
    pub storage_id: StorageId,
    pub resource_id: CatalogResourceId,
    pub content_sha256: String,
    pub outcome: CatalogActionOutcome,
    pub recorded_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<CatalogPlanId>,
}

/// The portion of a cross-storage target claim pinned into a Pull review.
/// A changed claim invalidates the review before any local mutation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct CatalogClaimPin {
    pub storage_id: StorageId,
    pub content_sha256: String,
}

impl CatalogClaimPin {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("catalog claim pin digest", &self.content_sha256)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogMaterializationsDocument {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub claims: Vec<CatalogClaim>,
}

impl Default for CatalogMaterializationsDocument {
    fn default() -> Self {
        Self {
            schema: CATALOG_SCHEMA,
            revision: 0,
            claims: Vec::new(),
        }
    }
}

impl CatalogMaterializationsDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog materializations schema {} (expected {})",
                self.schema, CATALOG_SCHEMA
            ));
        }
        if self.claims.len() > MAX_CATALOG_CLAIMS {
            return Err(format!(
                "catalog claims exceed the {} entry limit",
                MAX_CATALOG_CLAIMS
            ));
        }
        let mut edges = BTreeSet::new();
        for claim in &self.claims {
            claim.target.validate()?;
            claim.coordinate.validate()?;
            validate_sha256("catalog claim digest", &claim.content_sha256)?;
            if !edges.insert((
                claim.target.identity.canonical_json(),
                claim.storage_id.clone(),
            )) {
                return Err("duplicate catalog claim edge".to_string());
            }
        }
        Ok(())
    }
}

/// Machine-local recovery record for a completed preservation copy. Valid
/// only while the exact generated copy still exists with its recorded digest;
/// never portable fork lineage.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogPreservationWitness {
    pub storage_id: StorageId,
    pub coordinate: CatalogCoordinate,
    pub source_resource_id: CatalogResourceId,
    pub source_target: CatalogTargetConfig,
    pub local_source_sha256: String,
    pub storage_source_sha256: String,
    pub preserved_resource_id: CatalogResourceId,
    pub preserved_target: CatalogTargetConfig,
    pub preserved_content_sha256: String,
    pub operation_id: CatalogOperationId,
    pub created_at: u64,
}

impl CatalogPreservationWitness {
    pub fn validate(&self) -> Result<(), String> {
        self.coordinate.validate()?;
        self.source_target.validate()?;
        self.preserved_target.validate()?;
        validate_sha256("witness local source digest", &self.local_source_sha256)?;
        validate_sha256("witness storage source digest", &self.storage_source_sha256)?;
        validate_sha256("witness preserved digest", &self.preserved_content_sha256)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogPreservationsDocument {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub witnesses: Vec<CatalogPreservationWitness>,
}

impl Default for CatalogPreservationsDocument {
    fn default() -> Self {
        Self {
            schema: CATALOG_SCHEMA,
            revision: 0,
            witnesses: Vec::new(),
        }
    }
}

impl CatalogPreservationsDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog preservations schema {} (expected {})",
                self.schema, CATALOG_SCHEMA
            ));
        }
        if self.witnesses.len() > MAX_CATALOG_WITNESSES {
            return Err(format!(
                "catalog preservation witnesses exceed the {} entry limit",
                MAX_CATALOG_WITNESSES
            ));
        }
        for witness in &self.witnesses {
            witness.validate()?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Operations journal
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogOperationKind {
    DirectoryReplace,
    Preservation,
    Fork,
    NativeApply,
    LocalRemoval,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogOperationPhase {
    Journaled,
    CopyActivated,
    WitnessRecorded,
    OriginalBackedUp,
    Completed,
    RolledBack,
    Failed,
}

impl CatalogOperationPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            CatalogOperationPhase::Completed
                | CatalogOperationPhase::RolledBack
                | CatalogOperationPhase::Failed
        )
    }
}

/// One journaled catalog mutation. Recovery resumes the same operation and
/// name; a retry never mints a second copy. The coordinate names the codec
/// recovery verifies with.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogOperationJournalEntry {
    pub operation_id: CatalogOperationId,
    pub kind: CatalogOperationKind,
    pub coordinate: CatalogCoordinate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_id: Option<StorageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<CatalogPlanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<CatalogPlanActionId>,
    pub target: CatalogTargetConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_tree_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_resource_id: Option<CatalogResourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_target_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incoming_content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_content_sha256: Option<String>,
    pub phase: CatalogOperationPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CatalogActionOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl CatalogOperationJournalEntry {
    pub fn validate(&self) -> Result<(), String> {
        self.target.validate()?;
        self.coordinate.validate()?;
        if self.coordinate.provider != self.target.provider {
            return Err(format!(
                "catalog operation '{}' coordinate does not own its target",
                self.operation_id
            ));
        }
        if self.created_at == 0 || self.updated_at < self.created_at {
            return Err(format!(
                "catalog operation '{}' has invalid timestamps",
                self.operation_id
            ));
        }
        for (label, digest) in [
            ("expected target", &self.expected_target_sha256),
            ("incoming content", &self.incoming_content_sha256),
            ("copy content", &self.copy_content_sha256),
            ("backup tree", &self.backup_tree_sha256),
        ] {
            if let Some(digest) = digest {
                validate_sha256(&format!("catalog operation {label} digest"), digest)?;
            }
        }
        for (label, path) in [
            ("target", &self.target_path),
            ("backup", &self.backup_path),
            ("copy target", &self.copy_target_path),
        ] {
            if let Some(path) = path {
                validate_absolute_clean_path(&format!("catalog operation {label}"), path)?;
            }
        }
        if let Some(copy_name) = &self.copy_name {
            validate_target_member("catalog operation copy name", copy_name)?;
        }

        let has_storage_plan_action =
            self.storage_id.is_some() && self.plan_id.is_some() && self.action_id.is_some();
        let has_target = self.target_path.is_some();
        let any_copy = self.copy_target_path.is_some()
            || self.copy_name.is_some()
            || self.copy_resource_id.is_some();
        let has_copy = self.copy_target_path.is_some()
            && self.copy_name.is_some()
            && self.copy_resource_id.is_some();
        if any_copy != has_copy {
            return Err(format!(
                "catalog operation '{}' has an incomplete copy target",
                self.operation_id
            ));
        }
        match self.kind {
            CatalogOperationKind::LocalRemoval => {
                if self.storage_id.is_some()
                    || self.plan_id.is_none()
                    || self.action_id.is_some()
                    || any_copy
                    || self.expected_target_sha256.is_none()
                    || self.incoming_content_sha256.is_some()
                    || self.copy_content_sha256.is_some()
                    || self.backup_path.is_some()
                    || self.backup_tree_sha256.is_some()
                    || !matches!(
                        self.phase,
                        CatalogOperationPhase::Journaled
                            | CatalogOperationPhase::Completed
                            | CatalogOperationPhase::Failed
                    )
                {
                    return Err("invalid local removal journal".to_string());
                }
            }
            CatalogOperationKind::DirectoryReplace => {
                if !has_storage_plan_action
                    || !has_target
                    || self.incoming_content_sha256.is_none()
                    || any_copy
                    || self.copy_content_sha256.is_some()
                {
                    return Err(format!(
                        "catalog directory-replace operation '{}' has inconsistent fields",
                        self.operation_id
                    ));
                }
                if !matches!(
                    self.phase,
                    CatalogOperationPhase::Journaled
                        | CatalogOperationPhase::OriginalBackedUp
                        | CatalogOperationPhase::Completed
                        | CatalogOperationPhase::RolledBack
                        | CatalogOperationPhase::Failed
                ) {
                    return Err(format!(
                        "catalog directory-replace operation '{}' has an invalid phase",
                        self.operation_id
                    ));
                }
            }
            CatalogOperationKind::Preservation => {
                if !has_storage_plan_action
                    || !has_target
                    || !has_copy
                    || self.expected_target_sha256.is_none()
                    || self.incoming_content_sha256.is_none()
                    || self.copy_content_sha256.is_none()
                {
                    return Err(format!(
                        "catalog preservation operation '{}' has inconsistent fields",
                        self.operation_id
                    ));
                }
                if !matches!(
                    self.phase,
                    CatalogOperationPhase::Journaled
                        | CatalogOperationPhase::CopyActivated
                        | CatalogOperationPhase::WitnessRecorded
                        | CatalogOperationPhase::OriginalBackedUp
                        | CatalogOperationPhase::Completed
                        | CatalogOperationPhase::RolledBack
                        | CatalogOperationPhase::Failed
                ) {
                    return Err(format!(
                        "catalog preservation operation '{}' has an invalid phase",
                        self.operation_id
                    ));
                }
            }
            CatalogOperationKind::Fork => {
                if self.storage_id.is_some()
                    || self.plan_id.is_none()
                    || self.action_id.is_some()
                    || !has_target
                    || !has_copy
                    || self.expected_target_sha256.is_none()
                    || self.incoming_content_sha256.is_some()
                    || self.copy_content_sha256.is_none()
                    || self.backup_path.is_some()
                    || self.backup_tree_sha256.is_some()
                    || !matches!(
                        self.phase,
                        CatalogOperationPhase::Journaled
                            | CatalogOperationPhase::Completed
                            | CatalogOperationPhase::Failed
                    )
                {
                    return Err(format!(
                        "catalog fork operation '{}' has inconsistent fields or phase",
                        self.operation_id
                    ));
                }
            }
            CatalogOperationKind::NativeApply => {
                if !has_storage_plan_action
                    || has_target
                    || self.backup_path.is_some()
                    || self.backup_tree_sha256.is_some()
                    || any_copy
                    || self.expected_target_sha256.is_some()
                    || self.incoming_content_sha256.is_none()
                    || self.copy_content_sha256.is_some()
                    || !matches!(
                        self.phase,
                        CatalogOperationPhase::Journaled
                            | CatalogOperationPhase::Completed
                            | CatalogOperationPhase::Failed
                    )
                {
                    return Err(format!(
                        "catalog native operation '{}' has inconsistent fields or phase",
                        self.operation_id
                    ));
                }
            }
        }

        if self.phase == CatalogOperationPhase::OriginalBackedUp && self.backup_path.is_none() {
            return Err(format!(
                "catalog operation '{}' records a backup phase without a backup",
                self.operation_id
            ));
        }
        if self.backup_path.is_some()
            && self.kind == CatalogOperationKind::Preservation
            && matches!(
                self.phase,
                CatalogOperationPhase::Journaled | CatalogOperationPhase::CopyActivated
            )
        {
            return Err(format!(
                "catalog operation '{}' records a backup before its backup phase",
                self.operation_id
            ));
        }
        if self.backup_path.is_some()
            != (self.backup_tree_sha256.is_some() && self.expected_target_sha256.is_some())
        {
            return Err(format!(
                "catalog operation '{}' has an incomplete backup witness",
                self.operation_id
            ));
        }
        if self.phase == CatalogOperationPhase::Completed
            && (self.kind == CatalogOperationKind::Preservation
                || self.kind == CatalogOperationKind::DirectoryReplace
                    && self.expected_target_sha256.is_some())
            && self.backup_path.is_none()
        {
            return Err(format!(
                "catalog operation '{}' completed without its required backup",
                self.operation_id
            ));
        }
        if let Some(target_path) = &self.target_path {
            let identity = self.target.decode()?;
            let expected_target = Path::new(&identity.canonical_root).join(&identity.member);
            if Path::new(target_path) != expected_target {
                return Err(format!(
                    "catalog operation '{}' target path does not match its target identity",
                    self.operation_id
                ));
            }
            if let (Some(copy_path), Some(copy_name)) = (&self.copy_target_path, &self.copy_name) {
                let expected_copy = Path::new(&identity.canonical_root).join(copy_name);
                if Path::new(copy_path) != expected_copy {
                    return Err(format!(
                        "catalog operation '{}' copy path does not match its target identity",
                        self.operation_id
                    ));
                }
            }
        }
        if let (Some(target_path), Some(backup_path)) = (&self.target_path, &self.backup_path) {
            let target_path = Path::new(target_path);
            let target_name = target_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    format!(
                        "catalog operation '{}' target has no name",
                        self.operation_id
                    )
                })?;
            let local_name = format!(".mallard-backup-{}-{}", self.operation_id, target_name);
            let repository_name = format!("{}-{}", self.operation_id, target_name);
            let sibling_backup = target_path
                .parent()
                .ok_or_else(|| {
                    format!(
                        "catalog operation '{}' target has no parent",
                        self.operation_id
                    )
                })?
                .join(&local_name);
            let plan_id = self.plan_id.as_ref().ok_or_else(|| {
                format!(
                    "catalog operation '{}' backup has no plan",
                    self.operation_id
                )
            })?;
            let repository_suffix = Path::new("backups")
                .join("catalog_resources")
                .join(plan_id.as_str())
                .join(&repository_name);
            if Path::new(backup_path) != sibling_backup
                && !Path::new(backup_path).ends_with(&repository_suffix)
            {
                return Err(format!(
                    "catalog operation '{}' backup path is not bound to the operation",
                    self.operation_id
                ));
            }
        }
        if self.phase.is_terminal() != self.outcome.is_some() {
            return Err(format!(
                "catalog operation '{}' outcome does not match its phase",
                self.operation_id
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogOperationsDocument {
    pub schema: u32,
    pub revision: u64,
    #[serde(default)]
    pub operations: Vec<CatalogOperationJournalEntry>,
}

impl Default for CatalogOperationsDocument {
    fn default() -> Self {
        Self {
            schema: CATALOG_SCHEMA,
            revision: 0,
            operations: Vec::new(),
        }
    }
}

impl CatalogOperationsDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog operations schema {} (expected {})",
                self.schema, CATALOG_SCHEMA
            ));
        }
        if self.operations.len() > MAX_CATALOG_OPERATIONS {
            return Err(format!(
                "catalog operations exceed the {} entry limit",
                MAX_CATALOG_OPERATIONS
            ));
        }
        let mut ids = BTreeSet::new();
        for entry in &self.operations {
            entry.validate()?;
            if !ids.insert(entry.operation_id.clone()) {
                return Err(format!(
                    "duplicate catalog operation '{}'",
                    entry.operation_id
                ));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Reconciliation and plans
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogResourceRelation {
    LocalOnly,
    StorageOnly,
    Matching,
    LocalChanged,
    StorageChanged,
    ChangedBoth,
    UnavailableLocally,
    Blocked,
}

/// Derived state for one registration/storage/repository link. It is never
/// persisted independently from the accepted base, current local observation,
/// and current remote head.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogLinkState {
    New,
    InSync,
    LocalChanged,
    StorageChanged,
    Diverged,
    LocalMissing,
    RemoteMissing,
    UnreviewedRemote,
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogResourceWarning {
    pub code: String,
    pub message: String,
    pub digest: String,
}

/// The profile a review was built against; apply refuses when it moved.
/// Machine-local: never serialized to the frontend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogProfilePin {
    pub profile_id: LocalCatalogProfileId,
    pub provider: ProviderKey,
    pub revision: u64,
    pub canonical_home: String,
}

/// One resolved target root a review was built against. Machine-local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogTargetPin {
    pub target: CatalogTargetKey,
    pub canonical_root: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogPushRow {
    pub resource_id: CatalogResourceId,
    #[serde(flatten)]
    pub coordinate: CatalogCoordinate,
    #[serde(flatten)]
    pub labels: CatalogCoordinateLabels,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_label: Option<String>,
    pub relation: CatalogResourceRelation,
    pub selectable: bool,
    pub selected_by_default: bool,
    pub held_for_pull: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub warnings: Vec<CatalogResourceWarning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CatalogTargetExpectation {
    Absent,
    Sha256 { sha256: String },
}

/// Pinned identity for a reviewed preservation copy. Apply never chooses or
/// changes the name.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogPreservationPin {
    pub copy_name: String,
    pub copy_resource_id: CatalogResourceId,
    pub copy_target_path: String,
    pub transformed_content_sha256: String,
    pub original_content_sha256: String,
    pub storage_content_sha256: String,
    /// A prior verified witness this action may reuse instead of copying
    /// again after partial failure.
    pub reusable_witness: bool,
}

/// Structured native invocation pinned at plan time. Portable strings never
/// become a shell command; argv is revalidated before launch. Machine-local
/// review state: never serialized to the frontend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePluginPlan {
    pub provider: ProviderKey,
    pub argv: Vec<String>,
    /// Marketplace registration argv required before install, when the
    /// portable source proves it. Empty when the marketplace already exists.
    pub marketplace_argv: Vec<Vec<String>>,
    pub mode: PluginApplyMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginApplyMode {
    InstallMissing,
    OverwriteExisting,
}

/// The fixed protocol outcomes an executable Pull action can take. They
/// describe the common workflow, never a provider, kind, or format identity:
/// a native (provider-inventory) action carries its pinned invocation beside
/// the action, and a directory action carries its target path.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogPullActionKind {
    Install,
    Update,
    PreserveAsCopyAndOverwrite { copy: CatalogPreservationPin },
    OverwriteWithStorage,
}

impl CatalogPullActionKind {
    pub fn preservation(&self) -> Option<&CatalogPreservationPin> {
        match self {
            CatalogPullActionKind::PreserveAsCopyAndOverwrite { copy } => Some(copy),
            _ => None,
        }
    }

    pub fn is_destructive_overwrite(&self) -> bool {
        matches!(self, CatalogPullActionKind::OverwriteWithStorage)
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogPullAction {
    pub action_id: CatalogPlanActionId,
    pub resource_id: CatalogResourceId,
    pub kind: CatalogPullActionKind,
    pub requires_destructive_approval: bool,
    /// Machine-local target identity: review state only, never IPC.
    #[serde(skip_serializing)]
    pub target: CatalogTargetConfig,
    #[serde(skip_serializing)]
    pub target_path: Option<String>,
    /// The pinned native invocation for provider-inventory resources.
    #[serde(skip_serializing)]
    pub native: Option<NativePluginPlan>,
    pub expected_target_state: CatalogTargetExpectation,
    pub incoming_content_sha256: String,
}

impl CatalogPullAction {
    pub fn validate(&self) -> Result<(), String> {
        self.target.validate()?;
        validate_sha256("pull action incoming digest", &self.incoming_content_sha256)?;
        if let CatalogTargetExpectation::Sha256 { sha256 } = &self.expected_target_state {
            validate_sha256("pull action target digest", sha256)?;
        }
        if self.native.is_some()
            && (self.target_path.is_some() || self.kind.preservation().is_some())
        {
            return Err(format!(
                "pull action '{}' pairs a native invocation with a directory operation",
                self.action_id
            ));
        }
        if self.native.is_none() && self.target_path.is_none() {
            return Err(format!(
                "pull action '{}' has neither a target path nor a native invocation",
                self.action_id
            ));
        }
        if self.kind.is_destructive_overwrite() && !self.requires_destructive_approval {
            return Err(format!(
                "destructive pull action '{}' must require explicit approval",
                self.action_id
            ));
        }
        if let Some(copy) = self.kind.preservation() {
            validate_component_name("preservation copy name", &copy.copy_name, 128)?;
            validate_sha256(
                "preservation transformed digest",
                &copy.transformed_content_sha256,
            )?;
            validate_sha256(
                "preservation original digest",
                &copy.original_content_sha256,
            )?;
            validate_sha256("preservation storage digest", &copy.storage_content_sha256)?;
            validate_absolute_clean_path("preservation copy target", &copy.copy_target_path)?;
        }
        Ok(())
    }
}

/// Non-mutating review dispositions. `skip` is a review choice recorded as an
/// outcome, never an executor action.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogPullDisposition {
    Executable,
    AlreadyMatching,
    KeptLocal,
    Manual { message: String },
    Blocked { reason: String },
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogPullItem {
    pub resource_id: CatalogResourceId,
    #[serde(flatten)]
    pub coordinate: CatalogCoordinate,
    #[serde(flatten)]
    pub labels: CatalogCoordinateLabels,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_label: Option<String>,
    pub relation: CatalogResourceRelation,
    pub disposition: CatalogPullDisposition,
    #[serde(default)]
    pub actions: Vec<CatalogPullAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_action_id: Option<CatalogPlanActionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CatalogPullItem {
    pub fn validate(&self) -> Result<(), String> {
        match &self.disposition {
            CatalogPullDisposition::Executable => {
                if self.actions.is_empty() {
                    return Err(format!(
                        "executable pull item '{}' owns no actions",
                        self.resource_id
                    ));
                }
            }
            _ => {
                if !self.actions.is_empty() {
                    return Err(format!(
                        "non-mutating pull item '{}' cannot own executor actions",
                        self.resource_id
                    ));
                }
            }
        }
        let mut ids = BTreeSet::new();
        for action in &self.actions {
            action.validate()?;
            if action.resource_id != self.resource_id {
                return Err(format!(
                    "pull action '{}' does not belong to item '{}'",
                    action.action_id, self.resource_id
                ));
            }
            if !ids.insert(action.action_id.clone()) {
                return Err(format!("duplicate pull action '{}'", action.action_id));
            }
        }
        if let Some(default_action_id) = &self.default_action_id {
            if !ids.contains(default_action_id) {
                return Err(format!(
                    "pull item '{}' defaults to an unknown action",
                    self.resource_id
                ));
            }
        }
        Ok(())
    }
}

/// One row inside a scope Push review: the presentation row plus the exact
/// captured resource the publish would upsert. Storage-only rows carry no
/// resource and can never be selected. The capture stays backend-side —
/// apply verifies against it, and the review only ever renders `row`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogScopePushRow {
    pub row: CatalogPushRow,
    #[serde(skip_serializing)]
    pub resource: Option<CatalogResourceEnvelope>,
}

impl CatalogScopePushRow {
    pub fn validate(&self) -> Result<(), String> {
        match &self.resource {
            Some(resource) => {
                resource.validate()?;
                if resource.resource_id != self.row.resource_id
                    || resource.coordinate() != self.row.coordinate
                {
                    return Err("catalog Push row does not match its resource".to_string());
                }
                if self.row.local_content_sha256.as_deref() != Some(&resource.content_sha256) {
                    return Err(
                        "catalog Push row content pin does not match its capture".to_string()
                    );
                }
            }
            None => {
                if self.row.selectable {
                    return Err("a selectable catalog Push row must carry its resource".to_string());
                }
            }
        }
        Ok(())
    }
}

/// Where one item's Pull reads on Mallard Cloud. Display only: the plan's
/// item head is what the Pull applies.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogPullSource {
    pub repository_id: String,
    /// `{owner}/{name}`.
    pub address: String,
    /// `draft` for a collaborator; `public` on a public repository the
    /// account does not belong to.
    pub reads: String,
    /// The version the review read.
    pub revision: u64,
}

/// Where one item's Push goes on Mallard Cloud. Display only: the Push
/// resolves its repository again when it writes.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogPushDestination {
    /// The repository it pushes to; `None` when the Push creates it.
    pub repository_id: Option<String>,
    /// `{owner}/{name}`; for a new repository, the name the service is
    /// expected to give it.
    pub address: String,
    pub create: bool,
}

/// Immutable Push review for one scope link: one observed repository head and
/// one row per resource on either side. Reviews live only in the in-memory
/// plan store; serialization is the one-way trip to the review dialog.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogScopePushPlan {
    pub plan_id: CatalogPlanId,
    pub scope_id: CatalogScopeId,
    pub link_id: CatalogLinkId,
    pub link_revision: u64,
    pub storage_id: StorageId,
    pub repository_id: CatalogResourceRepositoryId,
    pub storage_identity_sha256: String,
    #[serde(skip_serializing)]
    pub profile_pins: Vec<CatalogProfilePin>,
    #[serde(skip_serializing)]
    pub target_pins: Vec<CatalogTargetPin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_head: Option<LibraryHeadCoordinate>,
    /// On a storage that holds one item per repository: the head of each
    /// item's stream as the review observed it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub observed_items: BTreeMap<CatalogResourceId, LibraryHeadCoordinate>,
    /// On Mallard Cloud: where each pushable item goes, named before the
    /// Push, including the repositories it will create.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub destinations: BTreeMap<CatalogResourceId, CatalogPushDestination>,
    pub rows: Vec<CatalogScopePushRow>,
    pub warning_digests: Vec<String>,
    pub created_at: u64,
    pub expires_at: u64,
}

impl CatalogScopePushPlan {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256(
            "catalog Push storage identity",
            &self.storage_identity_sha256,
        )?;
        if let Some(head) = &self.observed_head {
            head.validate()?;
            if head.repository_id != self.repository_id {
                return Err("catalog Push head belongs to another repository".to_string());
            }
        }
        for head in self.observed_items.values() {
            head.validate()?;
        }
        if self.rows.len() > MAX_CATALOG_RESOURCES {
            return Err("catalog Push review exceeds its row limit".to_string());
        }
        let mut resource_ids = BTreeSet::new();
        let mut warnings = BTreeSet::new();
        for row in &self.rows {
            row.validate()?;
            if !resource_ids.insert(row.row.resource_id.clone()) {
                return Err("catalog Push review repeats a resource".to_string());
            }
            for warning in &row.row.warnings {
                warnings.insert(&warning.digest);
            }
        }
        let warning_digests: BTreeSet<_> = self.warning_digests.iter().collect();
        if warning_digests.len() != self.warning_digests.len() || warning_digests != warnings {
            return Err("catalog Push warning pins are inconsistent".to_string());
        }
        if self.link_revision == 0 || self.expires_at <= self.created_at {
            return Err("catalog Push has an invalid version or lifetime".to_string());
        }
        Ok(())
    }

    pub fn push_row(&self, resource_id: &CatalogResourceId) -> Option<&CatalogScopePushRow> {
        self.rows
            .iter()
            .find(|row| &row.row.resource_id == resource_id)
    }
}

/// One item inside a scope Pull review: the presentation item, the stored
/// resource its actions would materialize, and its cross-storage claim pins.
/// The stored resource stays backend-side for apply; the review renders
/// `item` and its claim pins.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogScopePullItem {
    pub item: CatalogPullItem,
    #[serde(skip_serializing)]
    pub resource: Option<CatalogResourceEnvelope>,
    pub conflicting_claims: Vec<CatalogClaimPin>,
}

impl CatalogScopePullItem {
    pub fn validate(&self) -> Result<(), String> {
        self.item.validate()?;
        match &self.resource {
            Some(resource) => {
                resource.validate()?;
                if resource.resource_id != self.item.resource_id
                    || resource.coordinate() != self.item.coordinate
                {
                    return Err("catalog Pull item does not match its resource".to_string());
                }
                if self
                    .item
                    .actions
                    .iter()
                    .any(|action| action.incoming_content_sha256 != resource.content_sha256)
                {
                    return Err("catalog Pull action pins a different content digest".to_string());
                }
            }
            None => {
                if matches!(self.item.disposition, CatalogPullDisposition::Executable) {
                    return Err(
                        "an executable catalog Pull item must carry its resource".to_string()
                    );
                }
            }
        }
        if self.conflicting_claims.len() > MAX_CATALOG_CLAIMS {
            return Err(format!(
                "catalog Pull claim pins exceed the {} entry limit",
                MAX_CATALOG_CLAIMS
            ));
        }
        if !self.conflicting_claims.is_empty()
            && self.item.actions.iter().any(|action| {
                action.kind.preservation().is_none() && !action.kind.is_destructive_overwrite()
            })
        {
            return Err(
                "a cross-storage conflict may offer only preservation, overwrite, or Skip"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub fn action(&self, action_id: &CatalogPlanActionId) -> Option<&CatalogPullAction> {
        self.item
            .actions
            .iter()
            .find(|action| &action.action_id == action_id)
    }
}

/// Immutable Pull review for one scope link: one mandatory repository head
/// and one item per stored resource. Reviews live only in the in-memory plan
/// store; serialization is the one-way trip to the review dialog.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogScopePullPlan {
    pub plan_id: CatalogPlanId,
    pub scope_id: CatalogScopeId,
    pub link_id: CatalogLinkId,
    pub link_revision: u64,
    pub storage_id: StorageId,
    pub repository_id: CatalogResourceRepositoryId,
    pub storage_identity_sha256: String,
    #[serde(skip_serializing)]
    pub profile_pins: Vec<CatalogProfilePin>,
    #[serde(skip_serializing)]
    pub target_pins: Vec<CatalogTargetPin>,
    /// The reviewed version of the coordinate repository; absent on a
    /// storage that holds one item per repository, where `item_heads`
    /// carries each item's reviewed version instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<LibraryHeadCoordinate>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub item_heads: BTreeMap<CatalogResourceId, LibraryHeadCoordinate>,
    /// On Mallard Cloud: the repository each item reads, and which version.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<CatalogResourceId, CatalogPullSource>,
    pub items: Vec<CatalogScopePullItem>,
    pub created_at: u64,
    pub expires_at: u64,
}

impl CatalogScopePullPlan {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256(
            "catalog Pull storage identity",
            &self.storage_identity_sha256,
        )?;
        if let Some(head) = &self.head {
            head.validate()?;
            if head.repository_id != self.repository_id {
                return Err("catalog Pull head belongs to another repository".to_string());
            }
        }
        for head in self.item_heads.values() {
            head.validate()?;
        }
        if self.head.is_none() && self.item_heads.is_empty() && !self.items.is_empty() {
            return Err("catalog Pull review names no reviewed version".to_string());
        }
        if self.items.len() > MAX_CATALOG_RESOURCES {
            return Err("catalog Pull review exceeds its item limit".to_string());
        }
        let mut resource_ids = BTreeSet::new();
        for item in &self.items {
            item.validate()?;
            if !resource_ids.insert(item.item.resource_id.clone()) {
                return Err("catalog Pull review repeats a resource".to_string());
            }
            let mut claiming_storages = BTreeSet::new();
            for claim in &item.conflicting_claims {
                claim.validate()?;
                if claim.storage_id == self.storage_id {
                    return Err("catalog Pull cannot conflict with its own storage".to_string());
                }
                if !claiming_storages.insert(&claim.storage_id) {
                    return Err("catalog Pull has duplicate claim pins".to_string());
                }
            }
        }
        if self.link_revision == 0 || self.expires_at <= self.created_at {
            return Err("catalog Pull has an invalid version or lifetime".to_string());
        }
        Ok(())
    }

    pub fn pull_item(&self, resource_id: &CatalogResourceId) -> Option<&CatalogScopePullItem> {
        self.items
            .iter()
            .find(|item| &item.item.resource_id == resource_id)
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogResourceForkPlan {
    pub plan_id: CatalogPlanId,
    pub scope_id: CatalogScopeId,
    #[serde(skip_serializing)]
    pub profile_pins: Vec<CatalogProfilePin>,
    #[serde(skip_serializing)]
    pub target_pins: Vec<CatalogTargetPin>,
    pub source_coordinate: CatalogCoordinate,
    pub source_resource_id: CatalogResourceId,
    pub source_display_name: String,
    #[serde(skip_serializing)]
    pub source_target: CatalogTargetConfig,
    #[serde(skip_serializing)]
    pub source_path: String,
    pub source_content_sha256: String,
    pub copy_name: String,
    pub copy_resource_id: CatalogResourceId,
    pub copy_target_path: String,
    pub transformed_content_sha256: String,
    pub created_at: u64,
    pub expires_at: u64,
}

impl CatalogResourceForkPlan {
    pub fn validate(&self) -> Result<(), String> {
        self.source_coordinate.validate()?;
        self.source_target.validate()?;
        validate_component_name("Fork copy name", &self.copy_name, 128)?;
        validate_absolute_clean_path("Fork source path", &self.source_path)?;
        validate_absolute_clean_path("Fork copy target", &self.copy_target_path)?;
        validate_sha256("Fork source digest", &self.source_content_sha256)?;
        validate_sha256("Fork transformed digest", &self.transformed_content_sha256)?;
        if self.expires_at <= self.created_at {
            return Err("catalog resource Fork plan lifetime is empty".to_string());
        }
        Ok(())
    }
}

/// One complete local removal review. Paths and fingerprints stay on this machine.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogResourceRemovalPlan {
    pub plan_id: CatalogPlanId,
    pub scope_id: CatalogScopeId,
    pub resource_id: CatalogResourceId,
    pub display_name: String,
    pub uninstall: bool,
    pub file_count: usize,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(skip_serializing)]
    pub scope_revision: u64,
    #[serde(skip_serializing)]
    pub profile_pins: Vec<CatalogProfilePin>,
    #[serde(skip_serializing)]
    pub target_pins: Vec<CatalogTargetPin>,
    #[serde(skip_serializing)]
    pub target: CatalogTargetConfig,
    #[serde(skip_serializing)]
    pub resource: CatalogResourceEnvelope,
    #[serde(skip_serializing)]
    pub entries: BTreeMap<String, CatalogRemovalEntry>,
    #[serde(skip_serializing)]
    pub native_state_sha256: Option<String>,
    #[serde(skip_serializing)]
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRemovalEntry {
    /// None is an exact directory, including empty directories.
    pub sha256: Option<String>,
}

impl CatalogResourceRemovalPlan {
    pub fn validate(&self) -> Result<(), String> {
        self.target.validate()?;
        self.resource.validate()?;
        if self.expires_at <= self.created_at || self.resource_id != self.resource.resource_id {
            return Err("invalid local removal review".to_string());
        }
        if self.uninstall != self.native_state_sha256.is_some()
            || self.uninstall != !self.argv.is_empty()
            || self.uninstall && !self.entries.is_empty()
            || !self.uninstall && !self.entries.contains_key("")
        {
            return Err("incomplete local removal review".to_string());
        }
        for (path, entry) in &self.entries {
            if !path.is_empty() {
                validate_portable_relative_path(path)?;
            }
            if let Some(digest) = &entry.sha256 {
                validate_sha256("removal file", digest)?;
            }
        }
        Ok(())
    }
}

/// The one value type the in-memory plan store holds: any pending review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogResourcePlanDocument {
    Push(CatalogScopePushPlan),
    Pull(CatalogScopePullPlan),
    Fork(CatalogResourceForkPlan),
    Removal(CatalogResourceRemovalPlan),
}

impl CatalogResourcePlanDocument {
    pub fn plan_id(&self) -> &CatalogPlanId {
        match self {
            Self::Push(plan) => &plan.plan_id,
            Self::Pull(plan) => &plan.plan_id,
            Self::Fork(plan) => &plan.plan_id,
            Self::Removal(plan) => &plan.plan_id,
        }
    }

    pub fn created_at(&self) -> u64 {
        match self {
            Self::Push(plan) => plan.created_at,
            Self::Pull(plan) => plan.created_at,
            Self::Fork(plan) => plan.created_at,
            Self::Removal(plan) => plan.created_at,
        }
    }

    pub fn expires_at(&self) -> u64 {
        match self {
            Self::Push(plan) => plan.expires_at,
            Self::Pull(plan) => plan.expires_at,
            Self::Fork(plan) => plan.expires_at,
            Self::Removal(plan) => plan.expires_at,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Push(plan) => plan.validate(),
            Self::Pull(plan) => plan.validate(),
            Self::Fork(plan) => plan.validate(),
            Self::Removal(plan) => plan.validate(),
        }
    }
}

// ---------------------------------------------------------------------------
// Results and blockers
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogActionOutcome {
    Removed,
    Installed,
    Updated,
    PreservedAndUpdated,
    Forked,
    AlreadyMatching,
    KeptLocal,
    Skipped,
    Manual,
    Partial,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogActionResult {
    pub resource_id: CatalogResourceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<CatalogPlanActionId>,
    pub display_name: String,
    pub outcome: CatalogActionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// A fixed message's code, for the interface to translate by, with the
    /// name it carries in `message_name`. Errors and planning details have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_code: Option<CatalogActionMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogActionMessage {
    /// Skipped; local content was left unchanged.
    Skipped,
    /// Forked from the source named in `message_name`.
    Forked,
    /// Your version was kept as the copy named in `message_name`.
    KeptAs,
}

/// A typed catalog blocker. Native contracts that cannot be verified fail
/// through this shape instead of guessed CLI syntax or permissive fallbacks.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CatalogBlocker {
    OverlappingTargets {
        roots: Vec<String>,
    },
    UnsupportedProvider {
        provider: String,
    },
    UnsupportedCoordinate {
        detail: String,
    },
    ProfileUnavailable {
        detail: String,
    },
    NativeContractUnverified {
        provider: ProviderKey,
        operation: String,
        detail: String,
    },
    InventoryFailed {
        detail: String,
    },
    /// The native resource restores by hand: its portable intent carries no
    /// source the provider can resolve.
    ManualInstall {
        detail: String,
    },
}

impl fmt::Display for CatalogBlocker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogBlocker::OverlappingTargets { roots } => write!(
                formatter,
                "overlapping_targets: the active target roots overlap ({})",
                roots.join(", ")
            ),
            CatalogBlocker::UnsupportedProvider { provider } => {
                write!(formatter, "unsupported provider '{}'", provider)
            }
            CatalogBlocker::UnsupportedCoordinate { detail } => {
                write!(formatter, "unsupported Library coordinate: {}", detail)
            }
            CatalogBlocker::ProfileUnavailable { detail } => {
                write!(formatter, "catalog profile unavailable: {}", detail)
            }
            CatalogBlocker::NativeContractUnverified {
                provider,
                operation,
                detail,
            } => write!(
                formatter,
                "{} native {} contract is unverified: {}",
                provider.as_str(),
                operation,
                detail
            ),
            CatalogBlocker::InventoryFailed { detail } => {
                write!(formatter, "catalog inventory failed: {}", detail)
            }
            CatalogBlocker::ManualInstall { detail } => formatter.write_str(detail),
        }
    }
}

/// Deterministic warning digest for exact acknowledgement.
pub fn warning_digest(code: &str, resource_id: &CatalogResourceId, message: &str) -> String {
    length_prefixed_digest(
        b"mallard-catalog-warning-v1",
        &[
            code.as_bytes(),
            resource_id.as_str().as_bytes(),
            message.as_bytes(),
        ],
    )
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Envelope fixtures for common-code tests. They build structurally valid
    //! envelopes at arbitrary coordinates without touching any codec.

    use super::*;
    use serde_json::json;

    pub fn coordinate(provider: &str, kind: &str, format: &str, version: u32) -> CatalogCoordinate {
        CatalogCoordinate::new(provider, kind, format, version).unwrap()
    }

    pub fn sha(byte: u8) -> String {
        hex_lower(&[byte; 32])
    }

    pub fn portable_file(path: &str, byte: u8, size: u64) -> CatalogPortableFile {
        CatalogPortableFile {
            path: path.to_string(),
            sha256: sha(byte),
            size,
            mode: 0o644,
        }
    }

    pub fn envelope(
        coordinate: &CatalogCoordinate,
        identity: serde_json::Value,
        descriptor: serde_json::Value,
        files: Vec<CatalogPortableFile>,
    ) -> CatalogResourceEnvelope {
        CatalogResourceEnvelope::new(
            coordinate,
            BoundedPortableObject::from_json(&identity).unwrap(),
            BoundedPortableObject::from_json(&descriptor).unwrap(),
            files,
        )
    }

    pub fn named_envelope(coordinate: &CatalogCoordinate, name: &str) -> CatalogResourceEnvelope {
        envelope(
            coordinate,
            json!({ "name": name }),
            json!({}),
            vec![portable_file("BODY.md", 1, 64)],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use serde_json::json;

    fn codex_skills() -> CatalogCoordinate {
        coordinate("codex", "agent_skill", "agentskills.io.skill", 1)
    }

    fn codex_pets(version: u32) -> CatalogCoordinate {
        coordinate("codex", "desktop_pet", "openai.codex-pet", version)
    }

    #[test]
    fn resource_ids_are_stable_and_content_digests_change_with_bytes() {
        let identity = json!({ "root": "provider-home", "effective_name": "review", "install_directory": "review" });
        let first = envelope(
            &codex_skills(),
            identity.clone(),
            json!({}),
            vec![portable_file("SKILL.md", 1, 64)],
        );
        let second = envelope(
            &codex_skills(),
            identity,
            json!({}),
            vec![
                portable_file("SKILL.md", 1, 64),
                portable_file("notes.md", 3, 10),
            ],
        );
        assert_eq!(first.resource_id, second.resource_id);
        assert_ne!(first.content_sha256, second.content_sha256);
        first.validate().unwrap();
        second.validate().unwrap();
    }

    #[test]
    fn identity_fields_and_coordinates_participate_in_identity() {
        let home = envelope(
            &codex_skills(),
            json!({ "root": "provider-home", "effective_name": "review", "install_directory": "review" }),
            json!({}),
            vec![portable_file("SKILL.md", 1, 64)],
        );
        let shared = envelope(
            &codex_skills(),
            json!({ "root": "shared-user", "effective_name": "review", "install_directory": "review" }),
            json!({}),
            vec![portable_file("SKILL.md", 1, 64)],
        );
        assert_ne!(home.resource_id, shared.resource_id);

        // Two format versions of one format never share a resource id, and a
        // different provider never shares one either.
        let v1 = envelope(
            &codex_pets(1),
            json!({ "pet_id": "duck" }),
            json!({}),
            vec![],
        );
        let v2 = envelope(
            &codex_pets(2),
            json!({ "pet_id": "duck" }),
            json!({}),
            vec![],
        );
        assert_ne!(v1.resource_id, v2.resource_id);
        let claude = envelope(
            &coordinate("claude", "agent_skill", "agentskills.io.skill", 1),
            json!({ "root": "provider-home", "effective_name": "review", "install_directory": "review" }),
            json!({}),
            vec![portable_file("SKILL.md", 1, 64)],
        );
        assert_ne!(home.resource_id, claude.resource_id);
    }

    #[test]
    fn envelopes_validate_structurally_without_a_registry() {
        // An unknown coordinate parses and validates structurally; nothing
        // here needs a registered codec.
        let unknown = envelope(
            &coordinate("cursor", "agent_skill", "cursor.skill", 3),
            json!({ "name": "x" }),
            json!({ "anything": [1, 2, 3] }),
            vec![portable_file("a/b.txt", 4, 1)],
        );
        unknown.validate().unwrap();
        let text = serde_json::to_string(&unknown).unwrap();
        let parsed: CatalogResourceEnvelope = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, unknown);

        // Structural failures: empty identity, wrong id, wrong digest, bad
        // paths, non-canonical file order, extra fields.
        let mut empty = unknown.clone();
        empty.identity = BoundedPortableObject::empty();
        assert!(empty.validate().unwrap_err().contains("empty identity"));
        let mut foreign_id = unknown.clone();
        foreign_id.resource_id =
            CatalogResourceId::parse(format!("resource-{}", "a".repeat(64))).unwrap();
        assert!(foreign_id
            .validate()
            .unwrap_err()
            .contains("identity derivation"));
        let mut wrong_digest = unknown.clone();
        wrong_digest.content_sha256 = sha(9);
        assert!(wrong_digest
            .validate()
            .unwrap_err()
            .contains("content digest"));
        let mut traversal = unknown.clone();
        traversal.files[0].path = "../escape".to_string();
        traversal.content_sha256 = traversal.compute_content_sha256();
        assert!(traversal.validate().is_err());
        let mut unordered = unknown.clone();
        unordered.files = vec![portable_file("b", 1, 1), portable_file("a", 2, 1)];
        unordered.content_sha256 = unordered.compute_content_sha256();
        assert!(unordered
            .validate()
            .unwrap_err()
            .contains("canonical order"));
        let mut extra: serde_json::Value = serde_json::from_str(&text).unwrap();
        extra["surprise"] = json!(true);
        assert!(serde_json::from_value::<CatalogResourceEnvelope>(extra).is_err());
        assert!(serde_json::from_value::<CatalogResourceEnvelope>(json!({
            "resource_id": "resource-zz"
        }))
        .is_err());
    }

    #[test]
    fn portable_objects_keep_machine_paths_out_of_envelopes() {
        let text = serde_json::to_string(&json!({
            "resource_id": format!("resource-{}", "a".repeat(64)),
            "provider": "codex",
            "resource_kind": "agent_skill",
            "format": "agentskills.io.skill",
            "format_version": 1,
            "identity": { "path": "/Users/someone/.codex/skills/review" },
            "descriptor": {},
            "content_sha256": "a".repeat(64),
        }))
        .unwrap();
        assert!(serde_json::from_str::<CatalogResourceEnvelope>(&text).is_err());
    }

    #[test]
    fn repository_ids_derive_from_the_four_coordinates_only() {
        let skills = CatalogResourceRepositoryId::for_coordinate(&codex_skills());
        assert_eq!(
            skills,
            CatalogResourceRepositoryId::for_coordinate(&codex_skills())
        );
        assert_ne!(
            skills,
            CatalogResourceRepositoryId::for_coordinate(&coordinate(
                "claude",
                "agent_skill",
                "agentskills.io.skill",
                1
            ))
        );
        assert_ne!(
            CatalogResourceRepositoryId::for_coordinate(&codex_pets(1)),
            CatalogResourceRepositoryId::for_coordinate(&codex_pets(2))
        );
        assert_ne!(
            skills,
            CatalogResourceRepositoryId::for_coordinate(&coordinate(
                "codex",
                "agent_skill",
                "other.skill",
                1
            ))
        );
    }

    #[test]
    fn item_streams_derive_from_the_coordinate_and_the_resource_and_bind_their_tag() {
        let resource = CatalogResourceId::parse(format!("resource-{}", "7".repeat(64))).unwrap();
        let other = CatalogResourceId::parse(format!("resource-{}", "8".repeat(64))).unwrap();
        let item = CatalogResourceRepositoryId::for_item(&codex_skills(), &resource);
        // The vector the service pins for the same derivation (`STORE_API.md`
        // section 6): the two implementations must agree byte for byte.
        assert_eq!(item.as_str(), "library-931ae060ce0a57b8eb4256a2747da419");
        assert_eq!(
            item,
            CatalogResourceRepositoryId::for_item(&codex_skills(), &resource)
        );
        assert_ne!(
            item,
            CatalogResourceRepositoryId::for_item(&codex_skills(), &other)
        );
        assert_ne!(
            item,
            CatalogResourceRepositoryId::for_coordinate(&codex_skills())
        );
        assert_ne!(
            item,
            CatalogResourceRepositoryId::for_item(&codex_pets(2), &resource)
        );
        let tag = LibraryResourceTag::for_item(&codex_skills(), &resource);
        assert_eq!(tag.repository_id, item);
        assert_eq!(tag.resource_id.as_ref(), Some(&resource));
        tag.validate().unwrap();
        let foreign = LibraryResourceTag {
            resource_id: Some(other.clone()),
            ..tag.clone()
        };
        assert!(foreign
            .validate()
            .unwrap_err()
            .contains("does not match its coordinate derivation"));
        // A coordinate tag serializes without the field; an item tag with it.
        let coordinate_tag = LibraryResourceTag::new(
            &codex_skills(),
            CatalogResourceRepositoryId::for_coordinate(&codex_skills()),
        );
        assert!(!serde_json::to_string(&coordinate_tag)
            .unwrap()
            .contains("resource_id"));
        assert!(serde_json::to_string(&tag).unwrap().contains("resource_id"));
    }

    #[test]
    fn set_manifests_are_homogeneous_per_repository() {
        let repository_id = CatalogResourceRepositoryId::for_coordinate(&codex_pets(2));
        let duck = envelope(
            &codex_pets(2),
            json!({ "pet_id": "duck" }),
            json!({}),
            vec![],
        );
        let manifest = |resources| LibraryResourceManifest {
            schema_version: CATALOG_SCHEMA,
            repository_id: repository_id.clone(),
            revision: 1,
            commit_id: CatalogCommitId::generate().unwrap(),
            updated_at: 10,
            captured_with: CatalogCapturedWith {
                app: "mallard".to_string(),
                version: "0.1.2".to_string(),
            },
            published_by_origin: derive_library_origin("host", "replica", &repository_id).unwrap(),
            resources,
        };
        manifest(BTreeMap::from([(
            duck.resource_id.clone(),
            PublishedResourceEntry {
                resource: duck.clone(),
                updated_at: 10,
            },
        )]))
        .validate()
        .unwrap();

        // An entry keyed by another resource's id is unrepresentable.
        let foreign_key = manifest(BTreeMap::from([(
            envelope(
                &codex_pets(2),
                json!({ "pet_id": "other" }),
                json!({}),
                vec![],
            )
            .resource_id,
            PublishedResourceEntry {
                resource: duck.clone(),
                updated_at: 10,
            },
        )]));
        assert!(foreign_key
            .validate()
            .unwrap_err()
            .contains("foreign resource id"));

        // A version-1 pet cannot enter the version-2 repository.
        let stranger = envelope(
            &codex_pets(1),
            json!({ "pet_id": "duck" }),
            json!({}),
            vec![],
        );
        let mixed = manifest(BTreeMap::from([(
            stranger.resource_id.clone(),
            PublishedResourceEntry {
                resource: stranger,
                updated_at: 10,
            },
        )]));
        assert!(mixed
            .validate()
            .unwrap_err()
            .contains("does not belong to this repository"));
    }

    #[test]
    fn a_library_tag_must_match_its_coordinate_derivation() {
        let tag = LibraryResourceTag::new(
            &codex_skills(),
            CatalogResourceRepositoryId::for_coordinate(&codex_skills()),
        );
        tag.validate().unwrap();
        // Two first writers converge on identical bytes.
        assert_eq!(
            serde_json::to_vec(&tag).unwrap(),
            serde_json::to_vec(&LibraryResourceTag::new(
                &codex_skills(),
                CatalogResourceRepositoryId::for_coordinate(&codex_skills()),
            ))
            .unwrap()
        );
        assert_eq!(tag.coordinate(), codex_skills());

        let mismatched = LibraryResourceTag {
            format_version: 2,
            ..tag.clone()
        };
        assert!(mismatched
            .validate()
            .unwrap_err()
            .contains("does not match its coordinate derivation"));

        // An unknown coordinate is structurally fine: the registry decides.
        let unknown = coordinate("cursor", "agent_skill", "cursor.skill", 1);
        LibraryResourceTag::new(
            &unknown,
            CatalogResourceRepositoryId::for_coordinate(&unknown),
        )
        .validate()
        .unwrap();
        let zero = LibraryResourceTag {
            format_version: 0,
            ..tag
        };
        assert!(zero.validate().unwrap_err().contains("format version 0"));
    }

    #[test]
    fn a_scope_links_each_storage_at_most_once() {
        let scope_id = CatalogScopeId::generate().unwrap();
        let link = |storage: &str| CatalogScopeLink {
            link_id: CatalogLinkId::generate().unwrap(),
            scope_id: scope_id.clone(),
            storage_id: StorageId::parse(storage).unwrap(),
            repository_id: CatalogResourceRepositoryId::for_coordinate(&codex_pets(2)),
            accepted_head: None,
            accepted_resources: BTreeMap::new(),
            accepted_items: BTreeMap::new(),
            cloud_repositories: BTreeMap::new(),
            revision: 1,
            created_at: 1,
            updated_at: 1,
        };
        let document = |links| CatalogScopeLinksDocument {
            schema: CATALOG_SCHEMA,
            revision: 1,
            links,
        };
        document(vec![link("storage-a"), link("storage-b")])
            .validate()
            .unwrap();
        let error = document(vec![link("storage-a"), link("storage-a")])
            .validate()
            .unwrap_err();
        assert!(error.contains("already has a link"), "{error}");

        // An accepted head must belong to the link's own repository.
        let mut crossed = link("storage-a");
        crossed.accepted_head = Some(LibraryHeadCoordinate {
            repository_id: CatalogResourceRepositoryId::for_coordinate(&codex_skills()),
            revision: 1,
            commit_id: CatalogCommitId::generate().unwrap(),
            manifest_sha256: sha(1),
            manifest_size: 1,
            storage_identity_sha256: sha(2),
        });
        assert!(crossed
            .validate()
            .unwrap_err()
            .contains("accepted a different repository"));
    }

    fn local_path(relative: &str) -> String {
        std::env::temp_dir()
            .join(relative)
            .to_string_lossy()
            .into_owned()
    }

    fn target(member: &str) -> CatalogTargetConfig {
        CatalogTargetConfig::new(
            ProviderKey::parse("codex").unwrap(),
            CatalogTargetKey::parse("provider-home.skills").unwrap(),
            &CatalogTargetIdentity {
                canonical_root: local_path("codex/skills"),
                root_class: "provider-home".to_string(),
                member: member.to_string(),
            },
        )
        .unwrap()
    }

    #[test]
    fn targets_round_trip_their_typed_identity_and_stay_loadable_when_foreign() {
        let target = target("review");
        let identity = target.decode().unwrap();
        assert_eq!(identity.member, "review");
        let copy = target.with_member("review-local-copy").unwrap();
        assert_eq!(copy.decode().unwrap().member, "review-local-copy");
        assert_eq!(
            copy.decode().unwrap().canonical_root,
            local_path("codex/skills")
        );
        assert_ne!(copy, target);

        // A target shape this build does not decode still loads as part of a
        // document and only fails when something tries to operate on it.
        let foreign: CatalogTargetConfig = serde_json::from_value(json!({
            "provider": "cursor",
            "target": "workspace.rules",
            "identity": { "workspace": "main", "rule": "style" }
        }))
        .unwrap();
        foreign.validate().unwrap();
        assert!(foreign.decode().is_err());
        let empty: CatalogTargetConfig = serde_json::from_value(json!({
            "provider": "cursor",
            "target": "workspace.rules",
            "identity": {}
        }))
        .unwrap();
        assert!(empty.validate().unwrap_err().contains("empty identity"));
        assert!(CatalogTargetConfig::new(
            ProviderKey::parse("codex").unwrap(),
            CatalogTargetKey::parse("provider-home.skills").unwrap(),
            &CatalogTargetIdentity {
                canonical_root: "relative/root".to_string(),
                root_class: "provider-home".to_string(),
                member: "review".to_string(),
            },
        )
        .is_err());
    }

    #[test]
    fn claims_dedupe_per_target_and_storage() {
        let claim = |storage: &str| CatalogClaim {
            target: target("review"),
            coordinate: codex_skills(),
            storage_id: StorageId::parse(storage).unwrap(),
            resource_id: named_envelope(&codex_skills(), "review").resource_id,
            content_sha256: sha(1),
            outcome: CatalogActionOutcome::Installed,
            recorded_at: 1,
            plan_id: None,
        };
        let document = |claims| CatalogMaterializationsDocument {
            schema: CATALOG_SCHEMA,
            revision: 1,
            claims,
        };
        document(vec![claim("storage-a"), claim("storage-b")])
            .validate()
            .unwrap();
        assert!(document(vec![claim("storage-a"), claim("storage-a")])
            .validate()
            .unwrap_err()
            .contains("duplicate catalog claim edge"));
    }

    #[test]
    fn pull_actions_pair_protocols_with_matching_targets() {
        let action = CatalogPullAction {
            action_id: CatalogPlanActionId::generate().unwrap(),
            resource_id: named_envelope(&codex_skills(), "review").resource_id,
            kind: CatalogPullActionKind::Install,
            requires_destructive_approval: false,
            target: target("review"),
            target_path: Some(local_path("codex/skills/review")),
            native: Some(NativePluginPlan {
                provider: ProviderKey::parse("codex").unwrap(),
                argv: vec!["plugin".to_string()],
                marketplace_argv: Vec::new(),
                mode: PluginApplyMode::InstallMissing,
            }),
            expected_target_state: CatalogTargetExpectation::Absent,
            incoming_content_sha256: sha(5),
        };
        assert!(action
            .validate()
            .unwrap_err()
            .contains("native invocation with a directory operation"));
        let neither = CatalogPullAction {
            target_path: None,
            native: None,
            ..action.clone()
        };
        assert!(neither.validate().unwrap_err().contains("neither"));
        let destructive = CatalogPullAction {
            kind: CatalogPullActionKind::OverwriteWithStorage,
            native: None,
            ..action.clone()
        };
        assert!(destructive
            .validate()
            .unwrap_err()
            .contains("explicit approval"));
        let directory = CatalogPullAction {
            native: None,
            ..action
        };
        directory.validate().unwrap();
        // Machine-local target identity never crosses IPC.
        let json = serde_json::to_value(&directory).unwrap();
        assert!(json.get("target").is_none());
        assert!(json.get("target_path").is_none());
        assert!(json.get("native").is_none());
    }

    #[test]
    fn plans_serialize_coordinates_with_labels_and_no_machine_paths() {
        let resource = named_envelope(&codex_skills(), "review");
        let row = CatalogPushRow {
            resource_id: resource.resource_id.clone(),
            coordinate: codex_skills(),
            labels: CatalogCoordinateLabels {
                provider_label: "Codex".to_string(),
                kind_label: "Skills".to_string(),
                format_label: "Agent Skill".to_string(),
            },
            display_name: "review".to_string(),
            scope_label: None,
            relation: CatalogResourceRelation::LocalOnly,
            selectable: true,
            selected_by_default: true,
            held_for_pull: false,
            local_content_sha256: Some(resource.content_sha256.clone()),
            storage_content_sha256: None,
            blocked_reason: None,
            warnings: Vec::new(),
            detail: None,
        };
        let plan = CatalogScopePushPlan {
            plan_id: CatalogPlanId::generate().unwrap(),
            scope_id: CatalogScopeId::generate().unwrap(),
            link_id: CatalogLinkId::generate().unwrap(),
            link_revision: 1,
            storage_id: StorageId::parse("storage-a").unwrap(),
            repository_id: CatalogResourceRepositoryId::for_coordinate(&codex_skills()),
            storage_identity_sha256: sha(2),
            profile_pins: vec![CatalogProfilePin {
                profile_id: LocalCatalogProfileId::generate().unwrap(),
                provider: ProviderKey::parse("codex").unwrap(),
                revision: 1,
                canonical_home: "/Users/someone/.codex".to_string(),
            }],
            target_pins: vec![CatalogTargetPin {
                target: CatalogTargetKey::parse("provider-home.skills").unwrap(),
                canonical_root: "/Users/someone/.codex/skills".to_string(),
            }],
            observed_head: None,
            observed_items: BTreeMap::new(),
            destinations: BTreeMap::new(),
            rows: vec![CatalogScopePushRow {
                row,
                resource: Some(resource),
            }],
            warning_digests: Vec::new(),
            created_at: 1,
            expires_at: 2,
        };
        plan.validate().unwrap();
        let json = serde_json::to_value(&plan).unwrap();
        assert!(json.get("profile_pins").is_none());
        assert!(json.get("target_pins").is_none());
        let row = &json["rows"][0]["row"];
        assert_eq!(row["provider"], "codex");
        assert_eq!(row["resource_kind"], "agent_skill");
        assert_eq!(row["format"], "agentskills.io.skill");
        assert_eq!(row["format_version"], 1);
        assert_eq!(row["provider_label"], "Codex");
        assert_eq!(row["kind_label"], "Skills");
        assert!(json["rows"][0].get("resource").is_none());
        assert!(!serde_json::to_string(&plan)
            .unwrap()
            .contains("/Users/someone"));
    }

    #[test]
    fn fork_plans_hide_source_targets_and_paths() {
        let plan = CatalogResourceForkPlan {
            plan_id: CatalogPlanId::generate().unwrap(),
            scope_id: CatalogScopeId::generate().unwrap(),
            profile_pins: Vec::new(),
            target_pins: Vec::new(),
            source_coordinate: codex_skills(),
            source_resource_id: named_envelope(&codex_skills(), "review").resource_id,
            source_display_name: "review".to_string(),
            source_target: target("review"),
            source_path: local_path("codex/skills/review"),
            source_content_sha256: sha(8),
            copy_name: "review-local-copy".to_string(),
            copy_resource_id: named_envelope(&codex_skills(), "review-local-copy").resource_id,
            copy_target_path: local_path("codex/skills/review-local-copy"),
            transformed_content_sha256: sha(9),
            created_at: 1,
            expires_at: 2,
        };
        plan.validate().unwrap();
        let json = serde_json::to_value(&plan).unwrap();
        assert!(json.get("source_target").is_none());
        assert!(json.get("source_path").is_none());
        assert_eq!(json["source_coordinate"]["format_version"], 1);
    }

    #[test]
    fn library_origin_tokens_are_domain_separated() {
        let repository_id = CatalogResourceRepositoryId::for_coordinate(&codex_skills());
        let origin = derive_library_origin("host", "replica", &repository_id).unwrap();
        origin.validate().unwrap();
        let other = derive_library_origin("host", "replica-2", &repository_id).unwrap();
        assert_eq!(origin.origin_host_token, other.origin_host_token);
        assert_ne!(origin.origin_replica_token, other.origin_replica_token);
    }

    #[test]
    fn ids_fail_closed_during_deserialization() {
        assert!(serde_json::from_str::<CatalogResourceId>("\"resource-zz\"").is_err());
        assert!(serde_json::from_str::<CatalogPlanId>("\"plan-abcdef\"").is_err());
        let valid = format!("\"resource-{}\"", "a".repeat(64));
        assert!(serde_json::from_str::<CatalogResourceId>(&valid).is_ok());
    }

    #[test]
    fn generated_ids_keep_the_catalog_grammar() {
        let generated = [
            ("commit", CatalogCommitId::generate().unwrap().to_string()),
            ("catplan", CatalogPlanId::generate().unwrap().to_string()),
            (
                "action",
                CatalogPlanActionId::generate().unwrap().to_string(),
            ),
            ("catop", CatalogOperationId::generate().unwrap().to_string()),
            ("catlink", CatalogLinkId::generate().unwrap().to_string()),
            ("catscope", CatalogScopeId::generate().unwrap().to_string()),
            (
                "catprofile",
                LocalCatalogProfileId::generate().unwrap().to_string(),
            ),
        ];

        for (prefix, value) in generated {
            let random = value
                .strip_prefix(&format!("{}-", prefix))
                .expect("generated ID keeps its prefix");
            assert_eq!(random.len(), 24);
            assert!(random
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')));
        }
    }

    fn scope_row(coordinate: &CatalogCoordinate) -> CatalogScope {
        CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            provider: coordinate.provider.clone(),
            profile_id: LocalCatalogProfileId::generate().unwrap(),
            resource_kind: coordinate.resource_kind.clone(),
            format: coordinate.format.clone(),
            format_version: coordinate.format_version,
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn scopes_load_any_well_formed_coordinate_and_stay_unique_per_profile() {
        let first = scope_row(&codex_skills());
        first.validate().unwrap();
        // An unregistered coordinate is structurally a scope; only the
        // registry marks it unavailable.
        scope_row(&coordinate("cursor", "agent_skill", "cursor.skill", 1))
            .validate()
            .unwrap();
        let duplicate = CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            ..first.clone()
        };
        let document = |scopes| CatalogScopesDocument {
            schema: CATALOG_SCHEMA,
            revision: 1,
            scopes,
        };
        assert!(document(vec![first.clone(), duplicate])
            .validate()
            .unwrap_err()
            .contains("duplicates another row"));

        // Pet versions are distinct coordinates, so one profile holds both.
        let pet_v1 = CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            resource_kind: CatalogResourceKindKey::parse("desktop_pet").unwrap(),
            format: CatalogFormatKey::parse("openai.codex-pet").unwrap(),
            format_version: 1,
            ..first.clone()
        };
        let pet_v2 = CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            format_version: 2,
            ..pet_v1.clone()
        };
        let other_profile = scope_row(&codex_skills());
        document(vec![first, pet_v1, pet_v2, other_profile])
            .validate()
            .unwrap();

        let unknown: Result<CatalogScope, _> = serde_json::from_value(json!({
            "scope_id": "catscope-zz",
        }));
        assert!(unknown.is_err());
    }

    #[test]
    fn profiles_carry_a_versioned_provider_config() {
        let profile = LocalCatalogProfile {
            profile_id: LocalCatalogProfileId::generate().unwrap(),
            provider: ProviderKey::parse("codex").unwrap(),
            display_name: "Codex".to_string(),
            configured_home: local_path("codex"),
            provider_config: CatalogProviderConfig::version_1(),
            revision: 1,
            created_at: 1,
            updated_at: 1,
        };
        profile.validate().unwrap();
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["provider"], "codex");
        assert_eq!(json["provider_config"]["version"], 1);
        let zero = LocalCatalogProfile {
            provider_config: CatalogProviderConfig {
                version: 0,
                settings: BoundedConfigObject::empty(),
            },
            ..profile
        };
        assert!(zero.validate().is_err());
    }
}
