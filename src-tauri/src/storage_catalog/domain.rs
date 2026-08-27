//! Scope-rowed Library resource domain: portable schema-1 kind
//! repositories with set manifests, machine-local profiles, scopes, and
//! scope-storage links, immutable Push/Pull/Fork plans, and their validation.
//!
//! This module is deliberately independent of the project-sync domain. It
//! imports only storage-neutral identifiers (`StorageId`) and shared digest
//! validation from `mallard_domain`; it never uses bundle, project, thread,
//! or restore-plan types. Keep it free of Tauri and filesystem concerns.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use mallard_domain::{validate_absolute_clean_path, validate_sha256, StorageId};

/// Portable catalog schema. Exactly 1; unknown versions block catalog work.
pub const CATALOG_SCHEMA: u32 = 1;

// Enforced limits applied consistently in validation, capture, storage reads,
// and tests.
pub const MAX_CATALOG_RESOURCES: usize = 512;
pub const MAX_CATALOG_FILES: usize = 8_192;
pub const MAX_SKILL_FILES: usize = 256;
pub const MAX_SKILL_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_SKILL_TREE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_PET_MANIFEST_BYTES: u64 = 16 * 1024;
pub const MAX_PET_ATLAS_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PET_DECODED_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_REVISION_PAYLOAD_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_CATALOG_PROFILES: usize = 64;
// Every scope is a distinct (profile, kind) pair, so the ceiling is exactly
// profiles times the four resource kinds.
pub const MAX_CATALOG_SCOPES: usize = MAX_CATALOG_PROFILES * 4;
pub const MAX_CATALOG_LINKS_PER_SCOPE: usize = 128;
pub const MAX_CATALOG_CLAIMS: usize = 4_096;
pub const MAX_CATALOG_WITNESSES: usize = 512;
pub const MAX_CATALOG_OPERATIONS: usize = 256;
pub const MAX_GENERATED_NAME_ATTEMPTS: u32 = 64;

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{:02x}", byte);
    }
    output
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn random_hex(byte_count: usize) -> Result<String, String> {
    let mut bytes = vec![0_u8; byte_count];
    getrandom::fill(&mut bytes).map_err(|error| format!("gather randomness: {}", error))?;
    Ok(hex_lower(&bytes))
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
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

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
        Self::parse(format!("commit-{}", random_hex(12)?))
    }
}

impl CatalogPlanId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(format!("catplan-{}", random_hex(12)?))
    }
}

impl CatalogPlanActionId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(format!("action-{}", random_hex(12)?))
    }
}

impl CatalogOperationId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(format!("catop-{}", random_hex(12)?))
    }
}

impl CatalogResourceRepositoryId {
    /// The one repository a (provider, kind) pair maps to, on every machine.
    /// Deriving instead of minting is what makes two replicas that push
    /// resources of the same kind converge on the same repository without
    /// ever seeing each other first.
    pub fn for_kind(provider: ProviderKey, kind: CatalogResourceKindTag) -> Self {
        let digest = length_prefixed_digest(
            b"mallard-catalog-kind-repository-v1",
            &[provider.as_str().as_bytes(), kind.kind_token().as_bytes()],
        );
        Self::parse(format!("library-{}", &digest[..32]))
            .expect("derived catalog repository ids are always valid")
    }
}

impl CatalogLinkId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(format!("catlink-{}", random_hex(12)?))
    }
}

impl CatalogScopeId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(format!("catscope-{}", random_hex(12)?))
    }
}

impl LocalCatalogProfileId {
    pub fn generate() -> Result<Self, String> {
        Self::parse(format!("catprofile-{}", random_hex(12)?))
    }
}

/// The closed provider registry. Exactly Codex and Claude; a portable provider
/// key never selects an executable by itself.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKey {
    Codex,
    Claude,
}

impl ProviderKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKey::Codex => "codex",
            ProviderKey::Claude => "claude",
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            ProviderKey::Codex => "Codex",
            ProviderKey::Claude => "Claude",
        }
    }
}

/// The closed portable skill scopes: two Codex root classes and one Claude
/// root. Root class participates in resource identity, so equal names across
/// `<CODEX_HOME>/skills` and `$HOME/.agents/skills` stay distinct resources.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentSkillScope {
    CodexHome,
    CodexSharedUser,
    ClaudeUser,
}

impl AgentSkillScope {
    /// Stable identity token used in the resource-ID tuple.
    pub fn root_class_token(&self) -> &'static str {
        match self {
            AgentSkillScope::CodexHome => "provider/codex/codex_home",
            AgentSkillScope::CodexSharedUser => "agentskills/shared",
            AgentSkillScope::ClaudeUser => "provider/claude",
        }
    }

    pub fn provider(&self) -> ProviderKey {
        match self {
            AgentSkillScope::CodexHome | AgentSkillScope::CodexSharedUser => ProviderKey::Codex,
            AgentSkillScope::ClaudeUser => ProviderKey::Claude,
        }
    }

    /// Review label distinguishing the two Codex root classes.
    pub fn display_label(&self) -> &'static str {
        match self {
            AgentSkillScope::CodexHome => "Codex home",
            AgentSkillScope::CodexSharedUser => "Shared user",
            AgentSkillScope::ClaudeUser => "Claude",
        }
    }
}

/// The two supported Codex pet formats. The portable resource kind is the
/// format discriminator; there is no conversion between them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CodexPetFormat {
    V1,
    V2,
}

impl CodexPetFormat {
    pub fn kind_token(&self) -> &'static str {
        match self {
            CodexPetFormat::V1 => "codex_pet_v1",
            CodexPetFormat::V2 => "codex_pet_v2",
        }
    }

    /// Exact atlas pixel dimensions for this format.
    pub fn atlas_dimensions(&self) -> (u32, u32) {
        match self {
            CodexPetFormat::V1 => (1536, 1872),
            CodexPetFormat::V2 => (1536, 2288),
        }
    }

    /// Sprite grid: 8 columns of 192x208 cells; 9 rows for v1, 11 for v2.
    pub fn cell_grid(&self) -> (u32, u32) {
        match self {
            CodexPetFormat::V1 => (8, 9),
            CodexPetFormat::V2 => (8, 11),
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            CodexPetFormat::V1 => "v1",
            CodexPetFormat::V2 => "v2",
        }
    }
}

pub fn validate_display_text(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(format!("invalid {}: '{}'", label, value.escape_debug()));
    }
    Ok(())
}

/// Skill and pet install-directory / effective-name rule: ASCII alphanumeric
/// start, `[A-Za-z0-9._-]` body, bounded, never a dot path, no trailing dot.
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

pub fn validate_skill_name(label: &str, value: &str) -> Result<(), String> {
    validate_component_name(label, value, 128)
}

pub fn validate_pet_id(label: &str, value: &str) -> Result<(), String> {
    validate_component_name(label, value, 64)
}

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

/// A skill-relative payload path: bounded depth and length, no traversal, no
/// nested VCS metadata, portable component names.
pub fn validate_skill_relative_path(value: &str) -> Result<(), String> {
    let invalid = || format!("invalid skill payload path: '{}'", value.escape_debug());
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

/// Closed Library logical-path grammar:
/// `resources/<resource-id>/skill/<relative-path>` or
/// `resources/<resource-id>/pet/<atlas-basename>`. Catalog paths can never
/// begin with the project namespaces.
pub fn catalog_logical_path(
    resource_id: &CatalogResourceId,
    segment: &str,
    relative: &str,
) -> Result<String, String> {
    if !matches!(segment, "skill" | "pet") {
        return Err(format!("unknown catalog payload segment '{}'", segment));
    }
    if segment == "pet" && relative.contains('/') {
        return Err(format!(
            "pet payloads own exactly one atlas basename: '{}'",
            relative.escape_debug()
        ));
    }
    validate_skill_relative_path(relative)?;
    let path = format!("resources/{}/{}/{}", resource_id, segment, relative);
    for reserved in ["project/", "state/", "dependencies/", "requirements/"] {
        if path.starts_with(reserved) {
            return Err(format!("catalog path cannot begin with '{}'", reserved));
        }
    }
    Ok(path)
}

/// One immutable payload file: exact size, SHA-256, safe portable mode, and
/// its content-addressed object reference.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogFileEntry {
    pub sha256: String,
    pub size: u64,
    pub mode: u32,
    pub object_key: String,
}

impl CatalogFileEntry {
    pub fn validate(&self, label: &str, max_bytes: u64) -> Result<(), String> {
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
        if self.object_key != format!("_objects/{}", self.sha256) {
            return Err(format!("{} object key is not content-addressed", label));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Portable resources
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSkillDescriptor {
    pub scope: AgentSkillScope,
    pub effective_name: String,
    pub install_directory: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSkillResource {
    pub descriptor: AgentSkillDescriptor,
    pub content_sha256: String,
    pub files: BTreeMap<String, CatalogFileEntry>,
}

impl AgentSkillResource {
    pub fn validate(&self) -> Result<(), String> {
        validate_skill_name("skill effective name", &self.descriptor.effective_name)?;
        validate_skill_name(
            "skill install directory",
            &self.descriptor.install_directory,
        )?;
        if self.files.is_empty() || self.files.len() > MAX_SKILL_FILES {
            return Err(format!(
                "skill '{}' must own between 1 and {} files",
                self.descriptor.effective_name, MAX_SKILL_FILES
            ));
        }
        if !self.files.contains_key("SKILL.md") {
            return Err(format!(
                "skill '{}' has no SKILL.md declaration",
                self.descriptor.effective_name
            ));
        }
        let resource_id = skill_resource_id(
            self.descriptor.scope,
            &self.descriptor.effective_name,
            &self.descriptor.install_directory,
        );
        let mut folded = BTreeSet::new();
        let mut total: u64 = 0;
        for (path, entry) in &self.files {
            // The composed catalog logical path enforces the closed grammar
            // and the reserved-root exclusions for every payload file.
            catalog_logical_path(&resource_id, "skill", path)?;
            if !folded.insert(path.to_lowercase()) {
                return Err(format!(
                    "skill '{}' has a case-folded path collision at '{}'",
                    self.descriptor.effective_name, path
                ));
            }
            entry.validate(&format!("skill file '{}'", path), MAX_SKILL_FILE_BYTES)?;
            total = total.saturating_add(entry.size);
        }
        if total > MAX_SKILL_TREE_BYTES {
            return Err(format!(
                "skill '{}' exceeds the skill tree byte limit",
                self.descriptor.effective_name
            ));
        }
        let computed = self.compute_content_sha256();
        if computed != self.content_sha256 {
            return Err(format!(
                "skill '{}' content digest does not match its files",
                self.descriptor.effective_name
            ));
        }
        Ok(())
    }

    pub fn compute_content_sha256(&self) -> String {
        let descriptor = serde_json::to_vec(&self.descriptor).unwrap_or_default();
        let mut hasher = Sha256::new();
        update_length_prefixed(&mut hasher, b"mallard-catalog-content-v1");
        update_length_prefixed(&mut hasher, b"agent_skill");
        update_length_prefixed(&mut hasher, &descriptor);
        for (path, entry) in &self.files {
            update_length_prefixed(&mut hasher, path.as_bytes());
            update_length_prefixed(&mut hasher, entry.sha256.as_bytes());
            hasher.update(entry.size.to_be_bytes());
            hasher.update(entry.mode.to_be_bytes());
        }
        hex_lower(&hasher.finalize())
    }
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

/// The closed portable plugin source forms. A provider-managed marketplace
/// identity always carries its namespace; a credential-free Git location is
/// present when the marketplace itself is portable. A `None` Git source is a
/// machine-local marketplace: the intent stays portable but restores manually.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortablePluginSource {
    ProviderMarketplace {
        namespace: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git: Option<GitMarketplaceSource>,
    },
}

/// An exact version or immutable ref, present only when the native provider
/// operation can enforce it. Neither supported provider proves enforceable
/// exact-version installation today, so validation rejects any requirement.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortablePluginRequirement {
    pub exact: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginInstallIntentDescriptor {
    pub provider: ProviderKey,
    pub provider_native_namespace: String,
    pub plugin_id: String,
    pub source: PortablePluginSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<PortablePluginRequirement>,
    pub resolution: PluginResolutionPolicy,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginInstallIntentResource {
    pub descriptor: PluginInstallIntentDescriptor,
    pub content_sha256: String,
}

impl PluginInstallIntentResource {
    pub fn validate(&self) -> Result<(), String> {
        let descriptor = &self.descriptor;
        validate_plugin_namespace("plugin namespace", &descriptor.provider_native_namespace)?;
        validate_canonical_plugin_id("plugin id", &descriptor.plugin_id)?;
        let Some((_, namespace)) = descriptor.plugin_id.split_once('@') else {
            return Err(format!(
                "plugin id '{}' has no namespace qualifier",
                descriptor.plugin_id
            ));
        };
        if namespace != descriptor.provider_native_namespace {
            return Err(format!(
                "plugin id '{}' does not match its namespace '{}'",
                descriptor.plugin_id, descriptor.provider_native_namespace
            ));
        }
        let PortablePluginSource::ProviderMarketplace {
            namespace: source_namespace,
            git,
        } = &descriptor.source;
        if source_namespace != &descriptor.provider_native_namespace {
            return Err(format!(
                "plugin '{}' source namespace differs from its identity namespace",
                descriptor.plugin_id
            ));
        }
        match git {
            Some(git) => {
                validate_git_location("plugin marketplace source", &git.location)?;
                if let Some(git_ref) = &git.git_ref {
                    validate_display_text("plugin marketplace ref", git_ref, 256)?;
                }
                if descriptor.resolution != PluginResolutionPolicy::ProviderResolved {
                    return Err(format!(
                        "plugin '{}' with a portable marketplace source must be provider-resolved",
                        descriptor.plugin_id
                    ));
                }
            }
            None => {
                if descriptor.resolution != PluginResolutionPolicy::ManualOnly {
                    return Err(format!(
                        "plugin '{}' without a portable marketplace source must be manual-only",
                        descriptor.plugin_id
                    ));
                }
            }
        }
        if descriptor.requirement.is_some() {
            return Err(format!(
                "plugin '{}' declares an exact requirement, but {} cannot natively enforce one",
                descriptor.plugin_id,
                descriptor.provider.as_str()
            ));
        }
        let computed = self.compute_content_sha256();
        if computed != self.content_sha256 {
            return Err(format!(
                "plugin '{}' semantic intent digest does not match its descriptor",
                descriptor.plugin_id
            ));
        }
        Ok(())
    }

    /// Semantic install-intent digest. Machine-local observations (installed
    /// version, install root, exported skills, enablement) never enter it, so
    /// provider auto-updates do not create catalog conflicts.
    pub fn compute_content_sha256(&self) -> String {
        let descriptor = serde_json::to_vec(&self.descriptor).unwrap_or_default();
        length_prefixed_digest(
            b"mallard-catalog-content-v1",
            &[b"plugin_install_intent", &descriptor],
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodexPetDescriptor {
    pub pet_id: String,
    pub display_name: String,
    pub description: String,
    pub atlas_basename: String,
    pub atlas: CatalogFileEntry,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodexPetResource {
    pub descriptor: CodexPetDescriptor,
    pub content_sha256: String,
}

impl CodexPetResource {
    pub fn validate(&self, format: CodexPetFormat) -> Result<(), String> {
        let descriptor = &self.descriptor;
        validate_pet_id("pet id", &descriptor.pet_id)?;
        validate_display_text("pet display name", &descriptor.display_name, 128)?;
        validate_display_text("pet description", &descriptor.description, 1024)?;
        validate_component_name("pet atlas basename", &descriptor.atlas_basename, 255)?;
        let lowered = descriptor.atlas_basename.to_ascii_lowercase();
        if !(lowered.ends_with(".png") || lowered.ends_with(".webp")) {
            return Err(format!(
                "pet '{}' atlas must be a PNG or WebP basename",
                descriptor.pet_id
            ));
        }
        catalog_logical_path(
            &pet_resource_id(format, &descriptor.pet_id),
            "pet",
            &descriptor.atlas_basename,
        )?;
        descriptor.atlas.validate(
            &format!("pet '{}' atlas", descriptor.pet_id),
            MAX_PET_ATLAS_BYTES,
        )?;
        let computed = self.compute_content_sha256(format);
        if computed != self.content_sha256 {
            return Err(format!(
                "pet '{}' content digest does not match its package",
                descriptor.pet_id
            ));
        }
        Ok(())
    }

    pub fn compute_content_sha256(&self, format: CodexPetFormat) -> String {
        let descriptor = serde_json::to_vec(&self.descriptor).unwrap_or_default();
        length_prefixed_digest(
            b"mallard-catalog-content-v1",
            &[format.kind_token().as_bytes(), &descriptor],
        )
    }
}

/// The closed portable resource enum. Unknown kinds block catalog operations;
/// executable content never gains semantics through permissive parsing. The
/// v1 and v2 pet kinds are distinct resources that may coexist in storage.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogResource {
    AgentSkill(AgentSkillResource),
    PluginInstallIntent(PluginInstallIntentResource),
    CodexPetV1(CodexPetResource),
    CodexPetV2(CodexPetResource),
}

impl CatalogResource {
    pub fn provider(&self) -> ProviderKey {
        match self {
            CatalogResource::AgentSkill(skill) => skill.descriptor.scope.provider(),
            CatalogResource::PluginInstallIntent(plugin) => plugin.descriptor.provider,
            CatalogResource::CodexPetV1(_) | CatalogResource::CodexPetV2(_) => ProviderKey::Codex,
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            CatalogResource::AgentSkill(skill) => skill.descriptor.effective_name.clone(),
            CatalogResource::PluginInstallIntent(plugin) => plugin.descriptor.plugin_id.clone(),
            CatalogResource::CodexPetV1(pet) | CatalogResource::CodexPetV2(pet) => {
                pet.descriptor.display_name.clone()
            }
        }
    }

    pub fn content_sha256(&self) -> &str {
        match self {
            CatalogResource::AgentSkill(skill) => &skill.content_sha256,
            CatalogResource::PluginInstallIntent(plugin) => &plugin.content_sha256,
            CatalogResource::CodexPetV1(pet) | CatalogResource::CodexPetV2(pet) => {
                &pet.content_sha256
            }
        }
    }

    pub fn resource_id(&self) -> CatalogResourceId {
        match self {
            CatalogResource::AgentSkill(skill) => skill_resource_id(
                skill.descriptor.scope,
                &skill.descriptor.effective_name,
                &skill.descriptor.install_directory,
            ),
            CatalogResource::PluginInstallIntent(plugin) => plugin_resource_id(
                plugin.descriptor.provider,
                &plugin.descriptor.provider_native_namespace,
                &plugin.descriptor.plugin_id,
            ),
            CatalogResource::CodexPetV1(pet) => {
                pet_resource_id(CodexPetFormat::V1, &pet.descriptor.pet_id)
            }
            CatalogResource::CodexPetV2(pet) => {
                pet_resource_id(CodexPetFormat::V2, &pet.descriptor.pet_id)
            }
        }
    }

    pub fn pet_format(&self) -> Option<CodexPetFormat> {
        match self {
            CatalogResource::CodexPetV1(_) => Some(CodexPetFormat::V1),
            CatalogResource::CodexPetV2(_) => Some(CodexPetFormat::V2),
            _ => None,
        }
    }

    pub fn kind_tag(&self) -> CatalogResourceKindTag {
        match self {
            CatalogResource::AgentSkill(_) => CatalogResourceKindTag::AgentSkill,
            CatalogResource::PluginInstallIntent(_) => CatalogResourceKindTag::PluginInstallIntent,
            CatalogResource::CodexPetV1(_) => CatalogResourceKindTag::CodexPetV1,
            CatalogResource::CodexPetV2(_) => CatalogResourceKindTag::CodexPetV2,
        }
    }

    /// Total portable payload bytes owned by this resource.
    pub fn payload_bytes(&self) -> u64 {
        match self {
            CatalogResource::AgentSkill(skill) => skill
                .files
                .values()
                .fold(0_u64, |total, entry| total.saturating_add(entry.size)),
            CatalogResource::PluginInstallIntent(_) => 0,
            CatalogResource::CodexPetV1(pet) | CatalogResource::CodexPetV2(pet) => {
                pet.descriptor.atlas.size
            }
        }
    }

    pub fn file_count(&self) -> usize {
        match self {
            CatalogResource::AgentSkill(skill) => skill.files.len(),
            CatalogResource::PluginInstallIntent(_) => 0,
            CatalogResource::CodexPetV1(_) | CatalogResource::CodexPetV2(_) => 1,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            CatalogResource::AgentSkill(skill) => skill.validate(),
            CatalogResource::PluginInstallIntent(plugin) => plugin.validate(),
            CatalogResource::CodexPetV1(pet) => pet.validate(CodexPetFormat::V1),
            CatalogResource::CodexPetV2(pet) => pet.validate(CodexPetFormat::V2),
        }
    }
}

/// Resource identity is stable across content updates; identity components
/// are length-delimited and domain-tagged.
fn identity_resource_id(kind_token: &str, components: &[&str]) -> CatalogResourceId {
    let mut fields: Vec<&[u8]> = vec![kind_token.as_bytes()];
    for component in components {
        fields.push(component.as_bytes());
    }
    let digest = length_prefixed_digest(b"mallard-catalog-resource-v1", &fields);
    CatalogResourceId::parse(format!("resource-{}", digest))
        .expect("catalog resource ids derived from digests are always valid")
}

pub fn skill_resource_id(
    scope: AgentSkillScope,
    effective_name: &str,
    install_directory: &str,
) -> CatalogResourceId {
    identity_resource_id(
        "agent_skill",
        &[scope.root_class_token(), effective_name, install_directory],
    )
}

pub fn plugin_resource_id(
    provider: ProviderKey,
    namespace: &str,
    plugin_id: &str,
) -> CatalogResourceId {
    identity_resource_id(
        "plugin_install_intent",
        &[provider.as_str(), namespace, plugin_id],
    )
}

pub fn pet_resource_id(format: CodexPetFormat, pet_id: &str) -> CatalogResourceId {
    identity_resource_id(format.kind_token(), &[pet_id])
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
// Per-kind Library repositories
// ---------------------------------------------------------------------------

/// Immutable discovery metadata for one kind repository. It freezes exactly
/// the identity the repository id derives from — provider and kind — and it
/// contains no local profile/path/storage identity. It is never trusted in
/// place of the verified head and manifest.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryResourceTag {
    pub schema_version: u32,
    pub repository_id: CatalogResourceRepositoryId,
    pub provider: ProviderKey,
    pub resource_kind: CatalogResourceKindTag,
    pub created_at: u64,
}

impl LibraryResourceTag {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported library tag schema {} (expected {})",
                self.schema_version, CATALOG_SCHEMA
            ));
        }
        if !provider_owns_kind(self.provider, self.resource_kind) {
            return Err("library tag pairs a provider with a kind it cannot host".to_string());
        }
        if self.repository_id
            != CatalogResourceRepositoryId::for_kind(self.provider, self.resource_kind)
        {
            return Err(
                "library repository id does not match its provider and kind derivation".to_string(),
            );
        }
        if self.created_at == 0 {
            return Err("library tag requires a creation timestamp".to_string());
        }
        Ok(())
    }
}

/// One resource inside a kind repository's published set.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublishedResourceEntry {
    pub resource: CatalogResource,
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
            return Err("library manifest requires a revision and timestamp".to_string());
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
            if resource_id != &entry.resource.resource_id() {
                return Err("library manifest entry is keyed by a foreign resource id".to_string());
            }
            if entry.updated_at == 0 {
                return Err("library manifest entry requires a timestamp".to_string());
            }
            // Every entry must belong to the repository's own derivation, so
            // one manifest can never mix providers or kinds.
            if CatalogResourceRepositoryId::for_kind(
                entry.resource.provider(),
                entry.resource.kind_tag(),
            ) != self.repository_id
            {
                return Err(
                    "library manifest entry does not belong to this kind repository".to_string(),
                );
            }
            if entry.resource.file_count() > MAX_CATALOG_FILES
                || entry.resource.payload_bytes() > MAX_REVISION_PAYLOAD_BYTES
            {
                return Err("a library resource exceeds its revision limits".to_string());
            }
        }
        Ok(())
    }

    /// The published set as plain resources, the shape Pull execution reads.
    pub fn resource_map(&self) -> BTreeMap<CatalogResourceId, CatalogResource> {
        self.resources
            .iter()
            .map(|(resource_id, entry)| (resource_id.clone(), entry.resource.clone()))
            .collect()
    }
}

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

pub fn library_commit_relative_key(revision: u64, commit_id: &CatalogCommitId) -> String {
    format!("_commits/{}-{}.json", revision, commit_id)
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
            return Err("library head requires a revision and timestamp".to_string());
        }
        validate_sha256("library head manifest digest", &self.manifest_sha256)?;
        if self.manifest_key != library_manifest_relative_key(self.revision, &self.commit_id) {
            return Err("library head manifest key does not match its coordinates".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryResourceCommit {
    pub schema_version: u32,
    pub repository_id: CatalogResourceRepositoryId,
    pub revision: u64,
    pub commit_id: CatalogCommitId,
    pub manifest_key: String,
    pub manifest_sha256: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_commit_id: Option<CatalogCommitId>,
    pub published_by_origin: PortableCatalogOrigin,
}

impl LibraryResourceCommit {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA || self.revision == 0 || self.created_at == 0 {
            return Err("invalid library commit coordinates".to_string());
        }
        validate_sha256("library commit manifest digest", &self.manifest_sha256)?;
        if self.manifest_key != library_manifest_relative_key(self.revision, &self.commit_id) {
            return Err("library commit manifest key does not match its coordinates".to_string());
        }
        Ok(())
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
            return Err("accepted library head requires a revision".to_string());
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

/// Machine-local catalog profile. This is a new catalog-owned domain; it does
/// not widen or reuse the project-sync provider profile.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalCatalogProfile {
    pub profile_id: LocalCatalogProfileId,
    pub provider_key: ProviderKey,
    pub display_name: String,
    pub configured_home: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl LocalCatalogProfile {
    pub fn validate(&self) -> Result<(), String> {
        validate_display_text("catalog profile name", &self.display_name, 128)?;
        validate_absolute_clean_path("catalog profile home", &self.configured_home)
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

/// One user-added Catalog menu row: every resource of one kind under one
/// provider profile. Carries no display name — the label always derives from
/// the kind plus the profile's current name — and no storage, because the
/// scope links to storages through `CatalogScopeLink` records.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScope {
    pub scope_id: CatalogScopeId,
    pub provider: ProviderKey,
    pub profile_id: LocalCatalogProfileId,
    pub resource_kind: CatalogResourceKindTag,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl CatalogScope {
    pub fn validate(&self) -> Result<(), String> {
        if !provider_owns_kind(self.provider, self.resource_kind) {
            return Err(format!(
                "catalog scope '{}' pairs a provider with a kind it cannot host",
                self.scope_id
            ));
        }
        if self.revision == 0 || self.created_at == 0 || self.created_at > self.updated_at {
            return Err(format!(
                "catalog scope '{}' has invalid revision or timestamps",
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
            if !rows.insert((scope.profile_id.clone(), scope.resource_kind)) {
                return Err(format!(
                    "catalog scope '{}' duplicates another row for its profile and kind",
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

/// One scope's connection to one storage: the kind repository it reads, the
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
                "catalog scope link '{}' has invalid revision or timestamps",
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

/// Canonical machine-local target identity used by claims, journals, plans,
/// and locks. Contains canonical absolute roots; never portable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LocalCatalogTargetId {
    SkillDirectory {
        scope: AgentSkillScope,
        canonical_root: String,
        install_directory: String,
    },
    Plugin {
        provider: ProviderKey,
        canonical_home: String,
        namespace: String,
        plugin_id: String,
    },
    PetDirectory {
        canonical_home: String,
        pet_id: String,
    },
}

impl LocalCatalogTargetId {
    pub fn provider(&self) -> ProviderKey {
        match self {
            LocalCatalogTargetId::SkillDirectory { scope, .. } => scope.provider(),
            LocalCatalogTargetId::Plugin { provider, .. } => *provider,
            LocalCatalogTargetId::PetDirectory { .. } => ProviderKey::Codex,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            LocalCatalogTargetId::SkillDirectory {
                canonical_root,
                install_directory,
                ..
            } => {
                validate_absolute_clean_path("skill target root", canonical_root)?;
                validate_skill_name("skill target directory", install_directory)
            }
            LocalCatalogTargetId::Plugin {
                canonical_home,
                namespace,
                plugin_id,
                ..
            } => {
                validate_absolute_clean_path("plugin target home", canonical_home)?;
                validate_plugin_namespace("plugin target namespace", namespace)?;
                validate_canonical_plugin_id("plugin target id", plugin_id)
            }
            LocalCatalogTargetId::PetDirectory {
                canonical_home,
                pet_id,
            } => {
                validate_absolute_clean_path("pet target home", canonical_home)?;
                validate_pet_id("pet target id", pet_id)
            }
        }
    }
}

/// One verified per-target receipt and cross-storage claim edge. The same
/// target and digest can share claims across storages; a different digest
/// requires explicit conflict resolution naming every claiming storage.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogClaim {
    pub target: LocalCatalogTargetId,
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
            validate_sha256("catalog claim digest", &claim.content_sha256)?;
            if !edges.insert((claim.target.clone(), claim.storage_id.clone())) {
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
    pub source_resource_id: CatalogResourceId,
    pub source_target: LocalCatalogTargetId,
    pub local_source_sha256: String,
    pub storage_source_sha256: String,
    pub preserved_resource_id: CatalogResourceId,
    pub preserved_target: LocalCatalogTargetId,
    pub preserved_content_sha256: String,
    pub operation_id: CatalogOperationId,
    pub created_at: u64,
}

impl CatalogPreservationWitness {
    pub fn validate(&self) -> Result<(), String> {
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
    PluginApply,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogOperationPhase {
    Journaled,
    CopyActivated,
    WitnessRecorded,
    OriginalBackedUp,
    Activated,
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
/// name; a retry never mints a second copy.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogOperationJournalEntry {
    pub operation_id: CatalogOperationId,
    pub kind: CatalogOperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_id: Option<StorageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<CatalogPlanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<CatalogPlanActionId>,
    pub target: LocalCatalogTargetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
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
            entry.target.validate()?;
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CatalogResourceKindTag {
    AgentSkill,
    PluginInstallIntent,
    CodexPetV1,
    CodexPetV2,
}

impl CatalogResourceKindTag {
    /// Stable identity token for the kind-repository derivation. Pet tokens
    /// reuse the format's own token so identities stay format-distinct.
    pub fn kind_token(&self) -> &'static str {
        match self {
            CatalogResourceKindTag::AgentSkill => "agent_skill",
            CatalogResourceKindTag::PluginInstallIntent => "plugin_install_intent",
            CatalogResourceKindTag::CodexPetV1 => CodexPetFormat::V1.kind_token(),
            CatalogResourceKindTag::CodexPetV2 => CodexPetFormat::V2.kind_token(),
        }
    }

    pub fn pet_format(&self) -> Option<CodexPetFormat> {
        match self {
            CatalogResourceKindTag::CodexPetV1 => Some(CodexPetFormat::V1),
            CatalogResourceKindTag::CodexPetV2 => Some(CodexPetFormat::V2),
            _ => None,
        }
    }
}

/// The closed provider capability rule: which portable resource kinds a
/// provider can host locally. Pets are Codex-native; skills and plugins exist
/// for both providers. Scope validation and the served capability matrix both
/// derive from this single match, so they cannot drift apart.
pub fn provider_owns_kind(provider: ProviderKey, kind: CatalogResourceKindTag) -> bool {
    match kind {
        CatalogResourceKindTag::AgentSkill | CatalogResourceKindTag::PluginInstallIntent => true,
        CatalogResourceKindTag::CodexPetV1 | CatalogResourceKindTag::CodexPetV2 => {
            provider == ProviderKey::Codex
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogResourceWarning {
    pub code: String,
    pub message: String,
    pub digest: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogProfilePin {
    pub profile_id: LocalCatalogProfileId,
    pub provider_key: ProviderKey,
    pub revision: u64,
    pub canonical_home: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogSkillRootPin {
    pub scope: AgentSkillScope,
    pub canonical_root: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogPushRow {
    pub resource_id: CatalogResourceId,
    pub kind: CatalogResourceKindTag,
    pub display_name: String,
    pub provider: ProviderKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_label: Option<String>,
    pub relation: CatalogResourceRelation,
    pub selectable: bool,
    pub selected_by_default: bool,
    pub held_for_pull: bool,
    pub removal_candidate: bool,
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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

/// Structured native plugin invocation pinned at plan time. Portable strings
/// never become a shell command; argv is revalidated before launch.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativePluginPlan {
    pub provider: ProviderKey,
    pub argv: Vec<String>,
    /// Marketplace registration argv required before install, when the
    /// portable source proves it. Empty when the marketplace already exists.
    #[serde(default)]
    pub marketplace_argv: Vec<Vec<String>>,
    pub mode: PluginApplyMode,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginApplyMode {
    InstallMissing,
    OverwriteExisting,
}

/// The closed executable Pull action kinds (plan §8.1). Only skills and pets
/// can preserve-as-copy; only plugins can overwrite through the provider.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogPullActionKind {
    InstallSkill,
    UpdateSkill,
    PreserveSkillAsCopyAndOverwrite { copy: CatalogPreservationPin },
    OverwriteSkillWithStorage,
    InstallPlugin { plan: NativePluginPlan },
    UpdatePlugin { plan: NativePluginPlan },
    OverwritePluginWithStorage { plan: NativePluginPlan },
    InstallPet,
    UpdatePet,
    PreservePetAsCopyAndOverwrite { copy: CatalogPreservationPin },
    OverwritePetWithStorage,
}

impl CatalogPullActionKind {
    pub fn is_plugin(&self) -> bool {
        matches!(
            self,
            CatalogPullActionKind::InstallPlugin { .. }
                | CatalogPullActionKind::UpdatePlugin { .. }
                | CatalogPullActionKind::OverwritePluginWithStorage { .. }
        )
    }

    pub fn preservation(&self) -> Option<&CatalogPreservationPin> {
        match self {
            CatalogPullActionKind::PreserveSkillAsCopyAndOverwrite { copy }
            | CatalogPullActionKind::PreservePetAsCopyAndOverwrite { copy } => Some(copy),
            _ => None,
        }
    }

    pub fn is_destructive_overwrite(&self) -> bool {
        matches!(
            self,
            CatalogPullActionKind::OverwriteSkillWithStorage
                | CatalogPullActionKind::OverwritePetWithStorage
                | CatalogPullActionKind::OverwritePluginWithStorage { .. }
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogPullAction {
    pub action_id: CatalogPlanActionId,
    pub resource_id: CatalogResourceId,
    pub kind: CatalogPullActionKind,
    pub requires_destructive_approval: bool,
    pub target: LocalCatalogTargetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
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
        let target_is_plugin = matches!(self.target, LocalCatalogTargetId::Plugin { .. });
        if self.kind.is_plugin() != target_is_plugin {
            return Err(format!(
                "pull action '{}' pairs a plugin operation with a directory target or vice versa",
                self.action_id
            ));
        }
        let target_is_pet = matches!(self.target, LocalCatalogTargetId::PetDirectory { .. });
        let kind_is_pet = matches!(
            self.kind,
            CatalogPullActionKind::InstallPet
                | CatalogPullActionKind::UpdatePet
                | CatalogPullActionKind::PreservePetAsCopyAndOverwrite { .. }
                | CatalogPullActionKind::OverwritePetWithStorage
        );
        if kind_is_pet != target_is_pet {
            return Err(format!(
                "pull action '{}' pairs a pet operation with a non-pet target",
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogPullItem {
    pub resource_id: CatalogResourceId,
    pub kind: CatalogResourceKindTag,
    pub display_name: String,
    pub provider: ProviderKey,
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

/// The closed Fork source kinds: a plugin fork is unrepresentable.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogForkSourceKind {
    AgentSkill,
    CodexPetV1,
    CodexPetV2,
}

/// One row inside a scope Push review: the presentation row plus the exact
/// captured resource the publish would upsert. Storage-only rows carry no
/// resource and can never be selected.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScopePushRow {
    pub row: CatalogPushRow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<CatalogResource>,
}

impl CatalogScopePushRow {
    pub fn validate(&self) -> Result<(), String> {
        match &self.resource {
            Some(resource) => {
                resource.validate()?;
                if resource.resource_id() != self.row.resource_id
                    || resource.kind_tag() != self.row.kind
                    || resource.provider() != self.row.provider
                {
                    return Err("catalog Push row does not match its resource".to_string());
                }
                if self.row.local_content_sha256.as_deref() != Some(resource.content_sha256()) {
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

/// Immutable Push review for one scope link: one observed repository head and
/// one row per resource on either side.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScopePushPlan {
    pub schema_version: u32,
    pub plan_id: CatalogPlanId,
    pub scope_id: CatalogScopeId,
    pub link_id: CatalogLinkId,
    pub link_revision: u64,
    pub storage_id: StorageId,
    pub repository_id: CatalogResourceRepositoryId,
    pub storage_identity_sha256: String,
    pub profile_pins: Vec<CatalogProfilePin>,
    pub skill_root_pins: Vec<CatalogSkillRootPin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_head: Option<LibraryHeadCoordinate>,
    pub rows: Vec<CatalogScopePushRow>,
    pub warning_digests: Vec<String>,
    pub created_at: u64,
    pub expires_at: u64,
}

impl CatalogScopePushPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog Push schema {} (expected {})",
                self.schema_version, CATALOG_SCHEMA
            ));
        }
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
            return Err("catalog Push has an invalid revision or lifetime".to_string());
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScopePullItem {
    pub item: CatalogPullItem,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<CatalogResource>,
    #[serde(default)]
    pub conflicting_claims: Vec<CatalogClaimPin>,
}

impl CatalogScopePullItem {
    pub fn validate(&self) -> Result<(), String> {
        self.item.validate()?;
        match &self.resource {
            Some(resource) => {
                resource.validate()?;
                if resource.resource_id() != self.item.resource_id
                    || resource.kind_tag() != self.item.kind
                    || resource.provider() != self.item.provider
                {
                    return Err("catalog Pull item does not match its resource".to_string());
                }
                if self
                    .item
                    .actions
                    .iter()
                    .any(|action| action.incoming_content_sha256 != resource.content_sha256())
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
/// and one item per stored resource.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogScopePullPlan {
    pub schema_version: u32,
    pub plan_id: CatalogPlanId,
    pub scope_id: CatalogScopeId,
    pub link_id: CatalogLinkId,
    pub link_revision: u64,
    pub storage_id: StorageId,
    pub repository_id: CatalogResourceRepositoryId,
    pub storage_identity_sha256: String,
    pub profile_pins: Vec<CatalogProfilePin>,
    pub skill_root_pins: Vec<CatalogSkillRootPin>,
    pub head: LibraryHeadCoordinate,
    pub items: Vec<CatalogScopePullItem>,
    pub created_at: u64,
    pub expires_at: u64,
}

impl CatalogScopePullPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA {
            return Err(format!(
                "unsupported catalog Pull schema {} (expected {})",
                self.schema_version, CATALOG_SCHEMA
            ));
        }
        validate_sha256(
            "catalog Pull storage identity",
            &self.storage_identity_sha256,
        )?;
        self.head.validate()?;
        if self.head.repository_id != self.repository_id {
            return Err("catalog Pull head belongs to another repository".to_string());
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
            return Err("catalog Pull has an invalid revision or lifetime".to_string());
        }
        Ok(())
    }

    pub fn pull_item(&self, resource_id: &CatalogResourceId) -> Option<&CatalogScopePullItem> {
        self.items
            .iter()
            .find(|item| &item.item.resource_id == resource_id)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogResourceForkPlan {
    pub schema_version: u32,
    pub plan_id: CatalogPlanId,
    pub scope_id: CatalogScopeId,
    pub profile_pins: Vec<CatalogProfilePin>,
    pub skill_root_pins: Vec<CatalogSkillRootPin>,
    pub source_kind: CatalogForkSourceKind,
    pub source_resource_id: CatalogResourceId,
    pub source_display_name: String,
    pub source_target: LocalCatalogTargetId,
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
        if self.schema_version != CATALOG_SCHEMA {
            return Err("invalid catalog resource Fork schema".to_string());
        }
        self.source_target.validate()?;
        if matches!(self.source_target, LocalCatalogTargetId::Plugin { .. }) {
            return Err("plugins do not support Fork".to_string());
        }
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogResourcePlanDocument {
    Push(CatalogScopePushPlan),
    Pull(CatalogScopePullPlan),
    Fork(CatalogResourceForkPlan),
}

impl CatalogResourcePlanDocument {
    pub fn plan_id(&self) -> &CatalogPlanId {
        match self {
            Self::Push(plan) => &plan.plan_id,
            Self::Pull(plan) => &plan.plan_id,
            Self::Fork(plan) => &plan.plan_id,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Push(plan) => plan.validate(),
            Self::Pull(plan) => plan.validate(),
            Self::Fork(plan) => plan.validate(),
        }
    }
}

// ---------------------------------------------------------------------------
// Results and blockers
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogActionOutcome {
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
}

/// A typed catalog blocker. Native contracts that cannot be verified fail
/// through this shape instead of guessed CLI syntax or permissive fallbacks.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CatalogBlocker {
    OverlappingSkillRoots {
        roots: Vec<String>,
    },
    UnsupportedProvider {
        provider: String,
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
}

impl std::fmt::Display for CatalogBlocker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogBlocker::OverlappingSkillRoots { roots } => write!(
                formatter,
                "overlapping_skill_roots: the active skill roots overlap ({})",
                roots.join(", ")
            ),
            CatalogBlocker::UnsupportedProvider { provider } => {
                write!(formatter, "unsupported provider '{}'", provider)
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
mod tests {
    use super::*;

    fn sha(byte: u8) -> String {
        hex_lower(&[byte; 32])
    }

    fn skill_resource(name: &str, scope: AgentSkillScope) -> AgentSkillResource {
        let mut files = BTreeMap::new();
        files.insert(
            "SKILL.md".to_string(),
            CatalogFileEntry {
                sha256: sha(1),
                size: 64,
                mode: 0o644,
                object_key: format!("_objects/{}", sha(1)),
            },
        );
        let mut resource = AgentSkillResource {
            descriptor: AgentSkillDescriptor {
                scope,
                effective_name: name.to_string(),
                install_directory: name.to_string(),
            },
            content_sha256: sha(0),
            files,
        };
        resource.content_sha256 = resource.compute_content_sha256();
        resource
    }

    fn pet_resource(pet_id: &str, format: CodexPetFormat) -> CodexPetResource {
        let mut resource = CodexPetResource {
            descriptor: CodexPetDescriptor {
                pet_id: pet_id.to_string(),
                display_name: "Pet".to_string(),
                description: "One short sentence.".to_string(),
                atlas_basename: "spritesheet.webp".to_string(),
                atlas: CatalogFileEntry {
                    sha256: sha(2),
                    size: 1024,
                    mode: 0o644,
                    object_key: format!("_objects/{}", sha(2)),
                },
            },
            content_sha256: sha(0),
        };
        resource.content_sha256 = resource.compute_content_sha256(format);
        resource
    }

    #[test]
    fn resource_ids_are_stable_and_content_digests_change_with_bytes() {
        let first = skill_resource("review", AgentSkillScope::CodexHome);
        let mut second = skill_resource("review", AgentSkillScope::CodexHome);
        second.files.insert(
            "notes.md".to_string(),
            CatalogFileEntry {
                sha256: sha(3),
                size: 10,
                mode: 0o644,
                object_key: format!("_objects/{}", sha(3)),
            },
        );
        second.content_sha256 = second.compute_content_sha256();
        let first_wrapped = CatalogResource::AgentSkill(first);
        let second_wrapped = CatalogResource::AgentSkill(second);
        assert_eq!(first_wrapped.resource_id(), second_wrapped.resource_id());
        assert_ne!(
            first_wrapped.content_sha256(),
            second_wrapped.content_sha256()
        );
    }

    #[test]
    fn codex_skill_root_class_participates_in_identity() {
        let home = skill_resource("review", AgentSkillScope::CodexHome);
        let shared = skill_resource("review", AgentSkillScope::CodexSharedUser);
        assert_ne!(
            CatalogResource::AgentSkill(home).resource_id(),
            CatalogResource::AgentSkill(shared).resource_id()
        );
    }

    #[test]
    fn v1_and_v2_pets_share_a_pet_id_but_never_a_resource_id() {
        let v1 = CatalogResource::CodexPetV1(pet_resource("duck", CodexPetFormat::V1));
        let v2 = CatalogResource::CodexPetV2(pet_resource("duck", CodexPetFormat::V2));
        assert_ne!(v1.resource_id(), v2.resource_id());
        assert_eq!(
            v1.resource_id(),
            pet_resource_id(CodexPetFormat::V1, "duck")
        );
        assert_eq!(
            v2.resource_id(),
            pet_resource_id(CodexPetFormat::V2, "duck")
        );
    }

    #[test]
    fn v1_and_v2_pets_live_in_separate_kind_repositories() {
        let v1 = CatalogResource::CodexPetV1(pet_resource("duck", CodexPetFormat::V1));
        let v2 = CatalogResource::CodexPetV2(pet_resource("duck", CodexPetFormat::V2));
        let v1_repository =
            CatalogResourceRepositoryId::for_kind(ProviderKey::Codex, v1.kind_tag());
        let v2_repository =
            CatalogResourceRepositoryId::for_kind(ProviderKey::Codex, v2.kind_tag());
        assert_ne!(v1_repository, v2_repository);
        for (repository_id, resource) in [(v1_repository, v1), (v2_repository, v2)] {
            let manifest = LibraryResourceManifest {
                schema_version: CATALOG_SCHEMA,
                repository_id: repository_id.clone(),
                revision: 1,
                commit_id: CatalogCommitId::generate().unwrap(),
                updated_at: 10,
                captured_with: CatalogCapturedWith {
                    app: "mallard".to_string(),
                    version: "0.1.2".to_string(),
                },
                published_by_origin: derive_library_origin("host", "replica", &repository_id)
                    .unwrap(),
                resources: BTreeMap::from([(
                    resource.resource_id(),
                    PublishedResourceEntry {
                        resource,
                        updated_at: 10,
                    },
                )]),
            };
            manifest.validate().unwrap();
        }
    }

    #[test]
    fn set_manifests_reject_foreign_keys_and_mixed_kinds() {
        let repository_id = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::CodexPetV2,
        );
        let duck = CatalogResource::CodexPetV2(pet_resource("duck", CodexPetFormat::V2));
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

        // An entry keyed by another resource's id is unrepresentable.
        let foreign_key = manifest(BTreeMap::from([(
            pet_resource_id(CodexPetFormat::V2, "other"),
            PublishedResourceEntry {
                resource: duck.clone(),
                updated_at: 10,
            },
        )]));
        assert!(foreign_key
            .validate()
            .unwrap_err()
            .contains("foreign resource id"));

        // A v1 pet cannot enter the v2 kind repository.
        let stranger = CatalogResource::CodexPetV1(pet_resource("duck", CodexPetFormat::V1));
        let mixed = manifest(BTreeMap::from([(
            stranger.resource_id(),
            PublishedResourceEntry {
                resource: stranger,
                updated_at: 10,
            },
        )]));
        assert!(mixed
            .validate()
            .unwrap_err()
            .contains("does not belong to this kind repository"));
    }

    #[test]
    fn unknown_resource_kinds_and_fields_are_rejected() {
        let unknown_kind = serde_json::json!({
            "kind": "custom_agent",
            "descriptor": {}
        });
        assert!(serde_json::from_value::<CatalogResource>(unknown_kind).is_err());
        let unknown_field = serde_json::json!({
            "kind": "codex_pet_v1",
            "descriptor": {
                "pet_id": "duck",
                "display_name": "Duck",
                "description": "A duck.",
                "atlas_basename": "spritesheet.webp",
                "atlas": {
                    "sha256": sha(2),
                    "size": 10,
                    "mode": 420,
                    "object_key": format!("_objects/{}", sha(2)),
                },
                "surprise": true
            },
            "content_sha256": sha(0)
        });
        assert!(serde_json::from_value::<CatalogResource>(unknown_field).is_err());
    }

    #[test]
    fn plugin_intent_digest_ignores_observations_and_tracks_semantic_fields() {
        let descriptor = PluginInstallIntentDescriptor {
            provider: ProviderKey::Codex,
            provider_native_namespace: "acme".to_string(),
            plugin_id: "tools@acme".to_string(),
            source: PortablePluginSource::ProviderMarketplace {
                namespace: "acme".to_string(),
                git: Some(GitMarketplaceSource {
                    location: "https://example.com/acme/tools".to_string(),
                    git_ref: None,
                }),
            },
            requirement: None,
            resolution: PluginResolutionPolicy::ProviderResolved,
        };
        let mut resource = PluginInstallIntentResource {
            descriptor,
            content_sha256: sha(0),
        };
        resource.content_sha256 = resource.compute_content_sha256();
        resource.validate().unwrap();
        let baseline = resource.content_sha256.clone();

        // A source change alters semantic intent.
        let mut changed = resource.clone();
        changed.descriptor.source = PortablePluginSource::ProviderMarketplace {
            namespace: "acme".to_string(),
            git: Some(GitMarketplaceSource {
                location: "https://example.com/acme/tools-next".to_string(),
                git_ref: None,
            }),
        };
        changed.content_sha256 = changed.compute_content_sha256();
        assert_ne!(baseline, changed.content_sha256);

        // An unenforceable exact requirement is rejected outright.
        let mut with_requirement = resource.clone();
        with_requirement.descriptor.requirement = Some(PortablePluginRequirement {
            exact: "1.2.3".to_string(),
        });
        with_requirement.content_sha256 = with_requirement.compute_content_sha256();
        assert!(with_requirement
            .validate()
            .unwrap_err()
            .contains("cannot natively enforce"));
    }

    #[test]
    fn plugin_namespaces_distinguish_identity() {
        assert_ne!(
            plugin_resource_id(ProviderKey::Claude, "acme", "tools@acme"),
            plugin_resource_id(ProviderKey::Claude, "other", "tools@other")
        );
    }

    #[test]
    fn catalog_logical_paths_follow_the_grammar() {
        let resource_id = pet_resource_id(CodexPetFormat::V1, "duck");
        let path = catalog_logical_path(&resource_id, "pet", "spritesheet.webp").unwrap();
        assert!(path.starts_with("resources/resource-"));
        assert!(catalog_logical_path(&resource_id, "pet", "a/b.webp").is_err());
        assert!(catalog_logical_path(&resource_id, "skill", "../escape").is_err());
        assert!(catalog_logical_path(&resource_id, "skill", ".git/config").is_err());
        assert!(catalog_logical_path(&resource_id, "config", "x").is_err());
    }

    #[test]
    fn pull_actions_pair_kinds_with_matching_targets() {
        let plugin_target = LocalCatalogTargetId::Plugin {
            provider: ProviderKey::Claude,
            canonical_home: "/tmp/claude".to_string(),
            namespace: "acme".to_string(),
            plugin_id: "tools@acme".to_string(),
        };
        let action = CatalogPullAction {
            action_id: CatalogPlanActionId::generate().unwrap(),
            resource_id: plugin_resource_id(ProviderKey::Claude, "acme", "tools@acme"),
            kind: CatalogPullActionKind::InstallSkill,
            requires_destructive_approval: false,
            target: plugin_target,
            target_path: None,
            expected_target_state: CatalogTargetExpectation::Absent,
            incoming_content_sha256: sha(5),
        };
        assert!(action.validate().unwrap_err().contains("plugin operation"));
    }

    #[test]
    fn repository_ids_derive_from_provider_and_kind() {
        let skills = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::AgentSkill,
        );
        // Deterministic on every machine, distinct per provider and per kind.
        assert_eq!(
            skills,
            CatalogResourceRepositoryId::for_kind(
                ProviderKey::Codex,
                CatalogResourceKindTag::AgentSkill,
            )
        );
        assert_ne!(
            skills,
            CatalogResourceRepositoryId::for_kind(
                ProviderKey::Claude,
                CatalogResourceKindTag::AgentSkill,
            )
        );
        assert_ne!(
            CatalogResourceRepositoryId::for_kind(
                ProviderKey::Codex,
                CatalogResourceKindTag::CodexPetV1,
            ),
            CatalogResourceRepositoryId::for_kind(
                ProviderKey::Codex,
                CatalogResourceKindTag::CodexPetV2,
            )
        );
    }

    #[test]
    fn a_library_tag_must_match_its_kind_derivation() {
        let tag = LibraryResourceTag {
            schema_version: CATALOG_SCHEMA,
            repository_id: CatalogResourceRepositoryId::for_kind(
                ProviderKey::Codex,
                CatalogResourceKindTag::AgentSkill,
            ),
            provider: ProviderKey::Codex,
            resource_kind: CatalogResourceKindTag::AgentSkill,
            created_at: 1,
        };
        tag.validate().unwrap();

        let mismatched = LibraryResourceTag {
            resource_kind: CatalogResourceKindTag::CodexPetV1,
            ..tag.clone()
        };
        assert!(mismatched
            .validate()
            .unwrap_err()
            .contains("does not match its provider and kind derivation"));

        let unhostable = LibraryResourceTag {
            provider: ProviderKey::Claude,
            resource_kind: CatalogResourceKindTag::CodexPetV1,
            repository_id: CatalogResourceRepositoryId::for_kind(
                ProviderKey::Claude,
                CatalogResourceKindTag::AgentSkill,
            ),
            ..tag
        };
        assert!(unhostable.validate().unwrap_err().contains("cannot host"));
    }

    #[test]
    fn a_scope_links_each_storage_at_most_once() {
        let scope_id = CatalogScopeId::generate().unwrap();
        let link = |storage: &str| CatalogScopeLink {
            link_id: CatalogLinkId::generate().unwrap(),
            scope_id: scope_id.clone(),
            storage_id: StorageId::parse(storage).unwrap(),
            repository_id: CatalogResourceRepositoryId::for_kind(
                ProviderKey::Codex,
                CatalogResourceKindTag::CodexPetV2,
            ),
            accepted_head: None,
            accepted_resources: BTreeMap::new(),
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
            repository_id: CatalogResourceRepositoryId::for_kind(
                ProviderKey::Codex,
                CatalogResourceKindTag::AgentSkill,
            ),
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

    #[test]
    fn fork_plans_cannot_target_plugins() {
        let plan = CatalogResourceForkPlan {
            schema_version: CATALOG_SCHEMA,
            plan_id: CatalogPlanId::generate().unwrap(),
            scope_id: CatalogScopeId::generate().unwrap(),
            profile_pins: Vec::new(),
            skill_root_pins: Vec::new(),
            source_kind: CatalogForkSourceKind::AgentSkill,
            source_resource_id: skill_resource_id(AgentSkillScope::CodexHome, "review", "review"),
            source_display_name: "review".to_string(),
            source_target: LocalCatalogTargetId::Plugin {
                provider: ProviderKey::Codex,
                canonical_home: "/tmp/codex".to_string(),
                namespace: "acme".to_string(),
                plugin_id: "tools@acme".to_string(),
            },
            source_path: "/tmp/codex/skills/review".to_string(),
            source_content_sha256: sha(8),
            copy_name: "review-local-copy".to_string(),
            copy_resource_id: skill_resource_id(
                AgentSkillScope::CodexHome,
                "review-local-copy",
                "review-local-copy",
            ),
            copy_target_path: "/tmp/codex/skills/review-local-copy".to_string(),
            transformed_content_sha256: sha(9),
            created_at: 1,
            expires_at: 2,
        };
        assert!(plan
            .validate()
            .unwrap_err()
            .contains("plugins do not support Fork"));
    }

    #[test]
    fn library_origin_tokens_are_domain_separated() {
        let repository_id = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::AgentSkill,
        );
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

    fn scope_row(provider: ProviderKey, kind: CatalogResourceKindTag) -> CatalogScope {
        CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            provider,
            profile_id: LocalCatalogProfileId::generate().unwrap(),
            resource_kind: kind,
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn a_scope_requires_a_provider_that_owns_its_kind() {
        scope_row(ProviderKey::Claude, CatalogResourceKindTag::AgentSkill)
            .validate()
            .unwrap();
        scope_row(ProviderKey::Codex, CatalogResourceKindTag::CodexPetV1)
            .validate()
            .unwrap();
        assert!(
            scope_row(ProviderKey::Claude, CatalogResourceKindTag::CodexPetV1)
                .validate()
                .unwrap_err()
                .contains("cannot host")
        );
    }

    #[test]
    fn scope_rows_are_unique_per_profile_and_kind() {
        let first = scope_row(ProviderKey::Codex, CatalogResourceKindTag::AgentSkill);
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

        // Pet v1 and v2 are distinct kinds, so the same profile holds both rows.
        let pet_v1 = CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            resource_kind: CatalogResourceKindTag::CodexPetV1,
            ..first.clone()
        };
        let pet_v2 = CatalogScope {
            scope_id: CatalogScopeId::generate().unwrap(),
            resource_kind: CatalogResourceKindTag::CodexPetV2,
            ..first.clone()
        };
        // The same kind under another profile is another config's row.
        let other_profile = scope_row(ProviderKey::Codex, CatalogResourceKindTag::AgentSkill);
        document(vec![first, pet_v1, pet_v2, other_profile])
            .validate()
            .unwrap();
    }
}
