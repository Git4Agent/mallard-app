//! Project-scoped discovery and capture for portable project bundles.
//!
//! This module deliberately has no Tauri or cloud dependency. It converts
//! provider state and the explicit project allowlist into stable logical
//! resources. Callers persist the selected resource IDs in the bundle recipe;
//! capture never interprets a missing checkbox as a deletion.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::{DirEntry, WalkDir};

use super::chat_history::parse_session_index_titles;
use super::codex_rollout::session_meta_is_internal_subagent;
use super::conversation_codec::{
    analyze_codex_transcript, CodecLineage, CodecUnknownReason, CODEX_LINEAGE_CODEC_KEY,
};
use super::domain;
use super::global_inventory;

const MAX_DISCOVERED_FILES: usize = 20_000;
pub(crate) const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CAPTURE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_METADATA_LINES: usize = 128;
const MAX_TREE_DEPTH: usize = 32;

fn capture_file_limit(kind: &CaptureResourceKind) -> u64 {
    if matches!(kind, CaptureResourceKind::Conversation) {
        domain::MAX_CONVERSATION_FILE_BYTES
    } else {
        MAX_FILE_BYTES
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Claude,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CaptureResourceKind {
    ProjectContentFile,
    ProjectContentDirectory,
    Conversation,
    Memory,
    StandaloneSkill,
    Plugin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureApplyPolicy {
    SafeFile,
    Merge,
    Review,
    Dependency,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    CodexPlugin,
    ClaudePlugin,
    StandaloneSkill,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyScope {
    ProviderHome,
}

/// An executable operation is portable intent, never a shell command. The
/// eventual dependency runner must pass `argv` directly to a process API and
/// require the action's explicit approval.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DependencyAction {
    pub action_id: String,
    pub resource_id: String,
    pub provider: Provider,
    pub kind: DependencyKind,
    pub scope: DependencyScope,
    pub display_name: String,
    pub program: Option<String>,
    #[serde(default)]
    pub argv: Vec<String>,
    pub requires_review: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_logical_prefix: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResourceCandidate {
    pub resource_id: String,
    pub kind: CaptureResourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_cwd: Option<String>,
    pub apply_policy: CaptureApplyPolicy,
    pub selected_by_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub logical_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency: Option<DependencyAction>,
    /// Kind-specific descriptor facts (provenance, naming evidence, plugin
    /// source data). Copied verbatim into the bundle descriptor metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectInventory {
    pub resources: Vec<ResourceCandidate>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub ignored_count: usize,
    #[serde(default)]
    pub blocked_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedFile {
    pub logical_path: String,
    pub resource_id: String,
    pub bytes: Vec<u8>,
    pub source_mtime: u64,
    /// Portable permission bits. Set-id and other non-permission bits are
    /// always stripped; Windows capture uses 0o600.
    pub mode: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedDirectory {
    pub logical_path: String,
    pub resource_id: String,
    pub source_mtime: u64,
    pub mode: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CapturedResource {
    pub descriptor: ResourceCandidate,
    pub content_sha256: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapturedResources {
    pub resources: BTreeMap<String, CapturedResource>,
    pub files: BTreeMap<String, CapturedFile>,
    pub directories: BTreeMap<String, CapturedDirectory>,
    /// Local typed codec output used by conversation planning. It is kept
    /// separate from descriptor metadata so provider IDs and parsed chains do
    /// not leak into portable presentation fields.
    pub conversations: BTreeMap<String, CapturedConversation>,
    pub dependency_actions: Vec<DependencyAction>,
    pub unavailable_resource_ids: Vec<String>,
    pub warnings: Vec<String>,
    pub(crate) excluded_codex_subagent_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedConversation {
    pub provider: Provider,
    pub lineage_codec: String,
    pub source_thread_id: Option<String>,
    pub source_cwd: Option<String>,
    pub transcript_logical_path: String,
    pub record_count: usize,
    pub lineage: CodecLineage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandaloneSkillSource {
    pub provider: Provider,
    /// Runtime-visible capability name declared by the provider-supported
    /// `SKILL.md` metadata. This is the portable identity and may differ from
    /// the directory used to install the skill.
    pub effective_name: String,
    /// Physical directory name below the provider home's `skills/` root.
    /// Preserve it on restore because skill content may refer to its own
    /// installed path.
    pub install_dir_name: String,
    /// Stable provenance identifier (for example a sanitized Git URL plus
    /// subdirectory). It must not include a content digest.
    pub stable_key: String,
    pub source_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub project_root: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub claude_home: Option<PathBuf>,
    /// Canonical nested registrations owned by other bundles. A session whose
    /// cwd falls below one is excluded from this project's inventory.
    pub excluded_project_roots: Vec<PathBuf>,
    pub standalone_skills: Vec<StandaloneSkillSource>,
    /// Inventoried global plugins from the mapped provider homes. Observations
    /// of a plugin also declared in project config coalesce into one resource.
    pub global_plugins: Vec<global_inventory::GlobalPluginSource>,
    /// Skill candidates the ownership classifier refused; surfaced as blocked
    /// resources so the UI can show the evidence, never captured.
    pub blocked_global_skills: Vec<global_inventory::BlockedSkillCandidate>,
    /// Ordinary project content is intentionally lazy. Normal setup and
    /// provider inventory leave this false; the explicit Project files scan
    /// and a reviewed Push turn it on.
    pub include_project_content: bool,
    /// Additional canonical roots that generic discovery must not enter.
    pub excluded_content_roots: Vec<PathBuf>,
}

impl CaptureRequest {
    #[cfg(test)]
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            codex_home: None,
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct SourceFile {
    physical_path: PathBuf,
    logical_path: String,
    opaque_content: bool,
    /// Derived indexes retain the physical source only for containment and
    /// freshness validation; these filtered bytes are what enter the bundle.
    derived_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct SourceDirectory {
    physical_path: PathBuf,
    logical_path: String,
}

#[derive(Clone, Debug)]
struct DiscoveredResource {
    descriptor: ResourceCandidate,
    files: Vec<SourceFile>,
}

/// One full provider walk. Fields stay private to this module: callers hold
/// a Discovery opaquely and feed it back into the inventory/capture views,
/// sharing the walk (and derived candidates such as the filtered session
/// index) across several consumers without re-scanning.
#[derive(Clone, Debug)]
pub struct Discovery {
    resources: BTreeMap<String, DiscoveredResource>,
    directories: BTreeMap<String, SourceDirectory>,
    warnings: Vec<String>,
    ignored_count: usize,
    blocked_count: usize,
    excluded_codex_subagent_ids: BTreeSet<String>,
    excluded_codex_subagent_resource_ids: BTreeSet<String>,
}

impl Discovery {
    /// Descriptor-only view for inventory-style callers. The Discovery keeps
    /// its physical paths for a later capture.
    pub fn project_inventory(&self) -> ProjectInventory {
        ProjectInventory {
            resources: self
                .resources
                .values()
                .map(|candidate| candidate.descriptor.clone())
                .collect(),
            warnings: self.warnings.clone(),
            ignored_count: self.ignored_count,
            blocked_count: self.blocked_count,
        }
    }

    pub(crate) fn excluded_codex_subagent_resource_ids(&self) -> &BTreeSet<String> {
        &self.excluded_codex_subagent_resource_ids
    }
}

/// Discover resources without retaining file bytes. Absolute source paths are
/// kept only inside this module and never become resource identity.
pub fn discover_project(request: &CaptureRequest) -> Result<ProjectInventory, String> {
    Ok(discover_full(request)?.project_inventory())
}

/// Capture exactly the live resource IDs selected by the persistent recipe.
/// Missing or blocked resources are reported as unavailable; they are not
/// turned into deselections or tombstones.
#[cfg(test)]
pub fn capture_selected(
    request: &CaptureRequest,
    selected_resource_ids: &BTreeSet<String>,
) -> Result<CapturedResources, String> {
    let discovery = discover_full(request)?;
    capture_from_discovery(request, &discovery, selected_resource_ids)
}

/// `capture_selected` over an existing Discovery. Only the walk and its
/// derived candidates are shared: file bytes and metadata are re-read at
/// call time, so captured content is always current.
pub fn capture_from_discovery(
    request: &CaptureRequest,
    discovery: &Discovery,
    selected_resource_ids: &BTreeSet<String>,
) -> Result<CapturedResources, String> {
    let canonical_project = canonical_existing_dir(&request.project_root, "project root")?;
    let mut result = CapturedResources {
        warnings: discovery.warnings.clone(),
        excluded_codex_subagent_ids: discovery.excluded_codex_subagent_ids.clone(),
        ..CapturedResources::default()
    };
    let mut total_bytes = 0_u64;
    let mut casefold_paths = BTreeMap::<String, String>::new();

    for resource_id in selected_resource_ids {
        let Some(discovered) = discovery.resources.get(resource_id) else {
            result.unavailable_resource_ids.push(resource_id.clone());
            continue;
        };
        if discovered.descriptor.blocked_reason.is_some() {
            result.unavailable_resource_ids.push(resource_id.clone());
            continue;
        }

        let file_limit = capture_file_limit(&discovered.descriptor.kind);
        let mut resource_files = Vec::new();
        for source in &discovered.files {
            validate_logical_path(&source.logical_path)?;
            let folded = source.logical_path.to_lowercase();
            if let Some(existing) = casefold_paths.insert(folded, source.logical_path.clone()) {
                if existing != source.logical_path {
                    return Err(format!(
                        "case-insensitive logical path collision: '{}' and '{}'",
                        existing, source.logical_path
                    ));
                }
            }
            validate_source_file(
                &source.physical_path,
                &canonical_project,
                request,
                resource_id,
                file_limit,
            )?;
            let meta = fs::metadata(&source.physical_path)
                .map_err(|e| format!("inspect '{}': {}", source.physical_path.display(), e))?;
            let content_len = source
                .derived_bytes
                .as_ref()
                .map(|bytes| bytes.len() as u64)
                .unwrap_or(meta.len());
            if content_len > file_limit {
                return Err(format!(
                    "resource '{}' file '{}' exceeds {} bytes",
                    resource_id,
                    source.physical_path.display(),
                    file_limit
                ));
            }
            total_bytes = total_bytes
                .checked_add(content_len)
                .ok_or_else(|| "capture size overflow".to_string())?;
            if total_bytes > MAX_CAPTURE_BYTES {
                return Err(format!(
                    "selected resources exceed {} bytes",
                    MAX_CAPTURE_BYTES
                ));
            }
            let bytes = match &source.derived_bytes {
                Some(bytes) => bytes.clone(),
                None => fs::read(&source.physical_path)
                    .map_err(|e| format!("read '{}': {}", source.physical_path.display(), e))?,
            };
            if let Some(secret) = detect_secret_material(&bytes) {
                if source.opaque_content {
                    result.warnings.push(format!(
                        "{} may contain {} (opaque user content requires review)",
                        source.logical_path, secret
                    ));
                } else {
                    return Err(format!(
                        "resource '{}' contains possible {} in '{}'",
                        resource_id, secret, source.logical_path
                    ));
                }
            }
            let captured = CapturedFile {
                logical_path: source.logical_path.clone(),
                resource_id: resource_id.clone(),
                source_mtime: modified_secs(&meta),
                mode: safe_mode(&meta),
                bytes,
            };
            if result
                .files
                .insert(source.logical_path.clone(), captured.clone())
                .is_some()
            {
                return Err(format!(
                    "logical path '{}' is owned by more than one resource",
                    source.logical_path
                ));
            }
            resource_files.push(captured);
        }

        let captured_directory = if let Some(source) = discovery.directories.get(resource_id) {
            validate_logical_path(&source.logical_path)?;
            let folded = source.logical_path.to_lowercase();
            if let Some(existing) = casefold_paths.insert(folded, source.logical_path.clone()) {
                if existing != source.logical_path {
                    return Err(format!(
                        "case-insensitive logical path collision: '{}' and '{}'",
                        existing, source.logical_path
                    ));
                }
            }
            validate_source_directory(
                &source.physical_path,
                &canonical_project,
                request,
                resource_id,
            )?;
            let meta = fs::symlink_metadata(&source.physical_path).map_err(|error| {
                format!("inspect '{}': {}", source.physical_path.display(), error)
            })?;
            let captured = CapturedDirectory {
                logical_path: source.logical_path.clone(),
                resource_id: resource_id.clone(),
                source_mtime: modified_secs(&meta),
                mode: safe_mode(&meta),
            };
            if result
                .directories
                .insert(source.logical_path.clone(), captured.clone())
                .is_some()
            {
                return Err(format!(
                    "logical directory '{}' is owned by more than one resource",
                    source.logical_path
                ));
            }
            Some(captured)
        } else {
            None
        };

        let content_sha256 = resource_digest(
            &resource_files,
            captured_directory.as_ref(),
            discovered.descriptor.dependency.as_ref(),
        );
        if let Some(action) = &discovered.descriptor.dependency {
            result.dependency_actions.push(action.clone());
        }
        if discovered.descriptor.kind == CaptureResourceKind::Conversation {
            let provider = discovered.descriptor.provider.unwrap_or(Provider::Codex);
            if let Some(transcript) = resource_files.first() {
                let (lineage_codec, source_thread_id, source_cwd, record_count, lineage) =
                    match provider {
                        Provider::Codex if resource_files.len() == 1 => {
                            let analysis = analyze_codex_transcript(&transcript.bytes);
                            (
                                CODEX_LINEAGE_CODEC_KEY.to_string(),
                                analysis.source_thread_id,
                                analysis.source_cwd,
                                analysis.record_count,
                                analysis.lineage,
                            )
                        }
                        Provider::Codex => (
                            CODEX_LINEAGE_CODEC_KEY.to_string(),
                            None,
                            None,
                            0,
                            CodecLineage::Unknown {
                                reason: CodecUnknownReason::UnsupportedCodec,
                            },
                        ),
                        Provider::Claude => (
                            "claude.lineage.unavailable".to_string(),
                            None,
                            None,
                            0,
                            CodecLineage::Unknown {
                                reason: CodecUnknownReason::UnsupportedCodec,
                            },
                        ),
                    };
                result.conversations.insert(
                    resource_id.clone(),
                    CapturedConversation {
                        provider,
                        lineage_codec,
                        source_thread_id,
                        source_cwd,
                        transcript_logical_path: transcript.logical_path.clone(),
                        record_count,
                        lineage,
                    },
                );
            }
        }
        result.resources.insert(
            resource_id.clone(),
            CapturedResource {
                descriptor: discovered.descriptor.clone(),
                content_sha256,
            },
        );
    }
    result.unavailable_resource_ids.sort();
    result
        .dependency_actions
        .sort_by(|a, b| a.action_id.cmp(&b.action_id));
    result.warnings.sort();
    result.warnings.dedup();
    Ok(result)
}

/// Domain-facing capture entry point. The recipe remains the sole selection
/// authority; no transient UI filter is accepted here.
pub fn capture_recipe(
    request: &CaptureRequest,
    recipe: &domain::BundleRecipe,
) -> Result<CapturedResources, String> {
    let discovery = discover_full(request)?;
    capture_recipe_from_discovery(request, &discovery, recipe)
}

/// `capture_recipe` over an existing Discovery, sharing one walk between
/// selection persistence, freshness checks, and the byte capture.
pub fn capture_recipe_from_discovery(
    request: &CaptureRequest,
    discovery: &Discovery,
    recipe: &domain::BundleRecipe,
) -> Result<CapturedResources, String> {
    recipe.validate()?;
    let selected = recipe
        .entries
        .keys()
        .map(|id| id.as_str().to_string())
        .collect();
    capture_from_discovery(request, discovery, &selected)
}

/// Convert captured descriptors into the shared current domain model. This
/// is the only translation boundary; provider scanning stays independent of
/// manifest/storage concerns.
pub fn domain_resources(
    captured: &CapturedResources,
) -> Result<BTreeMap<domain::ResourceId, domain::ResourceDescriptor>, String> {
    let mut resources = BTreeMap::new();
    for resource in captured.resources.values() {
        let candidate = &resource.descriptor;
        let resource_id = domain::ResourceId::parse(candidate.resource_id.clone())?;
        let provider = candidate.provider.map(domain_provider);
        let kind = domain_kind(&candidate.kind, candidate.provider);
        let scope = match candidate.kind {
            CaptureResourceKind::Conversation
            | CaptureResourceKind::Memory
            | CaptureResourceKind::StandaloneSkill => domain::ResourceScope::ProviderState,
            CaptureResourceKind::Plugin => domain::ResourceScope::Dependency,
            _ => domain::ResourceScope::Project,
        };
        let provenance = match candidate.kind {
            CaptureResourceKind::Plugin => domain::Provenance::Plugin {
                provider: provider.ok_or_else(|| "plugin lacks provider".to_string())?,
                plugin_id: candidate.display_name.clone(),
            },
            CaptureResourceKind::StandaloneSkill => domain::Provenance::StandaloneSnapshot {
                stable_key: candidate
                    .metadata
                    .get("stable_key")
                    .cloned()
                    .unwrap_or_else(|| candidate.display_name.clone()),
                source_digest: resource.content_sha256.clone(),
            },
            CaptureResourceKind::ProjectContentFile
            | CaptureResourceKind::ProjectContentDirectory => {
                let relative_path = candidate
                    .logical_paths
                    .first()
                    .and_then(|path| path.strip_prefix("project/"))
                    .unwrap_or(&candidate.display_name)
                    .to_string();
                domain::Provenance::ProjectLocal { relative_path }
            }
            _ => domain::Provenance::Unknown,
        };
        let mut metadata = candidate.metadata.clone();
        metadata.retain(|key, _| !key.starts_with("_local_"));
        metadata.insert(
            "content_sha256".to_string(),
            resource.content_sha256.clone(),
        );
        if let Some(action) = &candidate.dependency {
            metadata.insert(
                "dependency_kind".to_string(),
                dependency_kind_name(&action.kind).to_string(),
            );
            metadata.insert(
                "dependency_scope".to_string(),
                dependency_scope_name(&action.scope).to_string(),
            );
            if let Some(program) = &action.program {
                metadata.insert("dependency_program".to_string(), program.clone());
            }
            metadata.insert(
                "dependency_argv_json".to_string(),
                serde_json::to_string(&action.argv).map_err(|e| e.to_string())?,
            );
        }
        let descriptor = domain::ResourceDescriptor {
            resource_id: resource_id.clone(),
            kind,
            provider,
            scope,
            display_name: candidate.display_name.clone(),
            provenance,
            apply_policy: domain_apply_policy(&candidate.apply_policy),
            relative_cwd: candidate.relative_cwd.clone(),
            codec_version: 1,
            metadata,
        };
        descriptor.validate()?;
        resources.insert(resource_id, descriptor);
    }
    Ok(resources)
}

#[cfg(test)]
pub fn domain_dependency_actions(
    captured: &CapturedResources,
) -> Result<Vec<domain::DependencyAction>, String> {
    captured
        .dependency_actions
        .iter()
        .map(|action| {
            Ok(domain::DependencyAction {
                action_id: domain::ActionId::parse(action.action_id.clone())?,
                resource_id: domain::ResourceId::parse(action.resource_id.clone())?,
                kind: match action.kind {
                    DependencyKind::CodexPlugin => domain::DependencyActionKind::InstallCodexPlugin,
                    DependencyKind::ClaudePlugin => {
                        domain::DependencyActionKind::InstallClaudePlugin
                    }
                    DependencyKind::StandaloneSkill => {
                        domain::DependencyActionKind::InstallStandaloneSkill
                    }
                },
                display_name: action.display_name.clone(),
                provider: Some(domain_provider(action.provider)),
                argv: action.argv.clone(),
                requires_explicit_approval: action.requires_review,
            })
        })
        .collect()
}

fn domain_provider(provider: Provider) -> domain::Provider {
    match provider {
        Provider::Codex => domain::Provider::Codex,
        Provider::Claude => domain::Provider::Claude,
    }
}

fn domain_kind(kind: &CaptureResourceKind, provider: Option<Provider>) -> domain::ResourceKind {
    match kind {
        CaptureResourceKind::ProjectContentFile => domain::ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => {
            domain::ResourceKind::ProjectContentDirectory
        }
        CaptureResourceKind::Conversation => match provider {
            Some(Provider::Claude) => domain::ResourceKind::ClaudeConversation,
            _ => domain::ResourceKind::CodexConversation,
        },
        CaptureResourceKind::Memory => domain::ResourceKind::ProjectMemory,
        CaptureResourceKind::StandaloneSkill => domain::ResourceKind::StandaloneSkill,
        CaptureResourceKind::Plugin => domain::ResourceKind::Plugin,
    }
}

fn domain_apply_policy(policy: &CaptureApplyPolicy) -> domain::ApplyPolicy {
    match policy {
        CaptureApplyPolicy::SafeFile => domain::ApplyPolicy::SafeFile,
        CaptureApplyPolicy::Merge => domain::ApplyPolicy::Merge,
        CaptureApplyPolicy::Review => domain::ApplyPolicy::ExplicitReview,
        CaptureApplyPolicy::Dependency => domain::ApplyPolicy::ExplicitInstall,
    }
}

fn dependency_kind_name(kind: &DependencyKind) -> &'static str {
    match kind {
        DependencyKind::CodexPlugin => "codex_plugin",
        DependencyKind::ClaudePlugin => "claude_plugin",
        DependencyKind::StandaloneSkill => "standalone_skill",
    }
}

fn dependency_scope_name(scope: &DependencyScope) -> &'static str {
    match scope {
        DependencyScope::ProviderHome => "provider_home",
    }
}

/// The one full provider walk behind every inventory and capture view.
pub fn discover_full(request: &CaptureRequest) -> Result<Discovery, String> {
    let canonical_project = canonical_existing_dir(&request.project_root, "project root")?;
    let excluded_roots = request
        .excluded_project_roots
        .iter()
        .map(|path| canonical_existing_dir(path, "nested project root"))
        .collect::<Result<Vec<_>, _>>()?;
    for excluded in &excluded_roots {
        if excluded == &canonical_project || !excluded.starts_with(&canonical_project) {
            return Err(format!(
                "nested project root '{}' must be a strict descendant of '{}'",
                excluded.display(),
                canonical_project.display()
            ));
        }
    }

    let mut discovery = Discovery {
        resources: BTreeMap::new(),
        directories: BTreeMap::new(),
        warnings: Vec::new(),
        ignored_count: 0,
        blocked_count: 0,
        excluded_codex_subagent_ids: BTreeSet::new(),
        excluded_codex_subagent_resource_ids: BTreeSet::new(),
    };
    if request.include_project_content {
        discover_generic_project_content(
            request,
            &canonical_project,
            &excluded_roots,
            &mut discovery,
        )?;
    }
    if let Some(home) = &request.codex_home {
        discover_codex(home, &canonical_project, &excluded_roots, &mut discovery)?;
    }
    if let Some(home) = &request.claude_home {
        discover_claude(home, &canonical_project, &excluded_roots, &mut discovery)?;
    }
    for skill in &request.standalone_skills {
        discover_standalone_skill(skill, &canonical_project, &mut discovery)?;
    }
    for plugin in &request.global_plugins {
        discover_global_plugin(plugin, &mut discovery)?;
    }
    for blocked in &request.blocked_global_skills {
        discover_blocked_global_skill(blocked, &mut discovery)?;
    }
    if discovery.resources.len() > MAX_DISCOVERED_FILES {
        return Err(format!(
            "project inventory exceeds {} resources",
            MAX_DISCOVERED_FILES
        ));
    }
    Ok(discovery)
}

#[derive(Clone, Debug)]
struct ProjectIgnoreRule {
    pattern: String,
    negated: bool,
    directory_only: bool,
    rooted: bool,
}

fn discover_generic_project_content(
    request: &CaptureRequest,
    root: &Path,
    excluded_project_roots: &[PathBuf],
    discovery: &mut Discovery,
) -> Result<(), String> {
    let ignore_rules = load_project_ignore_rules(root, &mut discovery.warnings)?;
    let mut excluded_roots = excluded_project_roots.to_vec();
    for excluded in &request.excluded_content_roots {
        match canonical_existing_dir(excluded, "excluded project-content root") {
            Ok(path) if path.starts_with(root) && path != root => excluded_roots.push(path),
            Ok(_) => {}
            Err(error) => discovery.warnings.push(error),
        }
    }
    excluded_roots.sort();
    excluded_roots.dedup();

    let owned_logical_paths = discovery
        .resources
        .values()
        .flat_map(|resource| resource.descriptor.logical_paths.iter())
        .filter(|logical_path| logical_path.starts_with("project/"))
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut walker = WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .max_depth(MAX_TREE_DEPTH.saturating_add(1))
        .into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|error| format!("walk '{}': {}", root.display(), error))?;
        if entry.depth() == 0 {
            continue;
        }
        let relative = match normalized_relative(root, entry.path()) {
            Ok(relative) => relative,
            Err(error) => {
                discovery.blocked_count = discovery.blocked_count.saturating_add(1);
                discovery.warnings.push(error);
                if entry.file_type().is_dir() {
                    walker.skip_current_dir();
                }
                continue;
            }
        };
        if entry.depth() > MAX_TREE_DEPTH {
            discovery.blocked_count = discovery.blocked_count.saturating_add(1);
            discovery.warnings.push(format!(
                "project content below '{}' exceeds the {}-component depth limit",
                relative, MAX_TREE_DEPTH
            ));
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if hard_excluded_project_content_path(&relative) {
            discovery.ignored_count = discovery.ignored_count.saturating_add(1);
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if excluded_roots
            .iter()
            .any(|excluded| entry.path().starts_with(excluded))
        {
            discovery.ignored_count = discovery.ignored_count.saturating_add(1);
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        let is_directory = entry.file_type().is_dir();
        if project_path_is_ignored(&ignore_rules, &relative, is_directory) {
            discovery.ignored_count = discovery.ignored_count.saturating_add(1);
            if is_directory {
                walker.skip_current_dir();
            }
            continue;
        }

        let logical_path = format!("project/{}", relative);
        if owned_logical_paths.contains(&logical_path) {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("inspect '{}': {}", entry.path().display(), error))?;

        if meta.file_type().is_symlink() {
            add_blocked_project_content_candidate(
                discovery,
                &relative,
                "Symlinks are not portable project content",
            )?;
            continue;
        }
        if meta.is_dir() {
            add_project_content_directory(discovery, root, entry.path(), &relative, &meta)?;
            continue;
        }
        if !meta.is_file() {
            add_blocked_project_content_candidate(
                discovery,
                &relative,
                "Sockets, devices, FIFOs, and other special files are not portable",
            )?;
            continue;
        }
        if relative
            .split('/')
            .any(|component| denied_file_name(component))
        {
            add_blocked_project_content_candidate(
                discovery,
                &relative,
                "Known credential and private-key paths cannot be synced",
            )?;
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() > 1 {
                add_blocked_project_content_candidate(
                    discovery,
                    &relative,
                    "Hard-linked files are not portable project content",
                )?;
                continue;
            }
        }
        if meta.len() > MAX_FILE_BYTES {
            add_blocked_project_content_candidate(
                discovery,
                &relative,
                &format!(
                    "File exceeds the {} byte project-content limit",
                    MAX_FILE_BYTES
                ),
            )?;
            continue;
        }
        add_project_content_file(discovery, root, entry.path(), &relative, &meta)?;
    }
    Ok(())
}

fn add_project_content_directory(
    discovery: &mut Discovery,
    root: &Path,
    path: &Path,
    relative: &str,
    meta: &fs::Metadata,
) -> Result<(), String> {
    validate_logical_project_relative(relative)?;
    reject_unsafe_ancestors(path, root)?;
    let logical_path = format!("project/{}", relative);
    let resource_id = project_content_resource_id("dir", relative);
    let mode = safe_mode(meta);
    let source_mtime = modified_secs(meta);
    let mut metadata = BTreeMap::new();
    metadata.insert("entry_type".to_string(), "directory".to_string());
    metadata.insert("_local_relative_path".to_string(), relative.to_string());
    metadata.insert("_local_mode".to_string(), mode.to_string());
    metadata.insert("_local_source_mtime".to_string(), source_mtime.to_string());
    metadata.insert(
        "_local_review_digest".to_string(),
        project_content_review_digest("dir", relative, "", mode),
    );
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                resource_id: resource_id.clone(),
                kind: CaptureResourceKind::ProjectContentDirectory,
                provider: None,
                display_name: relative.to_string(),
                relative_cwd: None,
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: true,
                blocked_reason: None,
                logical_paths: vec![logical_path.clone()],
                dependency: None,
                metadata,
            },
            files: Vec::new(),
        },
    )?;
    discovery.directories.insert(
        resource_id,
        SourceDirectory {
            physical_path: path.to_path_buf(),
            logical_path,
        },
    );
    Ok(())
}

fn add_project_content_file(
    discovery: &mut Discovery,
    root: &Path,
    path: &Path,
    relative: &str,
    before: &fs::Metadata,
) -> Result<(), String> {
    validate_logical_project_relative(relative)?;
    reject_unsafe_ancestors(path, root)?;
    let bytes = fs::read(path).map_err(|error| format!("read '{}': {}", path.display(), error))?;
    let after = fs::symlink_metadata(path)
        .map_err(|error| format!("reinspect '{}': {}", path.display(), error))?;
    if after.file_type().is_symlink()
        || !after.is_file()
        || before.len() != after.len()
        || modified_secs(before) != modified_secs(&after)
        || safe_mode(before) != safe_mode(&after)
    {
        return Err(format!(
            "project-content file '{}' changed while it was scanned",
            relative
        ));
    }
    let detected_secret = detect_secret_material(&bytes);
    if detected_secret == Some("private key material") {
        return add_blocked_project_content_candidate(
            discovery,
            relative,
            "Private-key material cannot be synced",
        );
    }
    let logical_path = format!("project/{}", relative);
    let resource_id = project_content_resource_id("file", relative);
    let mode = safe_mode(&after);
    let source_mtime = modified_secs(&after);
    let content_sha256 = sha256_bytes(&bytes);
    let warning_code = detected_secret
        .map(|_| "possible_secret")
        .or_else(|| is_executable_path(path).then_some("executable"));
    let mut metadata = BTreeMap::new();
    metadata.insert("entry_type".to_string(), "file".to_string());
    metadata.insert("_local_relative_path".to_string(), relative.to_string());
    metadata.insert("_local_size".to_string(), bytes.len().to_string());
    metadata.insert("_local_mode".to_string(), mode.to_string());
    metadata.insert("_local_source_mtime".to_string(), source_mtime.to_string());
    metadata.insert("_local_content_sha256".to_string(), content_sha256.clone());
    metadata.insert(
        "_local_review_digest".to_string(),
        project_content_review_digest("file", relative, &content_sha256, mode),
    );
    if let Some(warning_code) = warning_code {
        metadata.insert("_local_warning_code".to_string(), warning_code.to_string());
    }
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                resource_id,
                kind: CaptureResourceKind::ProjectContentFile,
                provider: None,
                display_name: relative.to_string(),
                relative_cwd: None,
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: true,
                blocked_reason: None,
                logical_paths: vec![logical_path.clone()],
                dependency: None,
                metadata,
            },
            files: vec![SourceFile {
                physical_path: path.to_path_buf(),
                logical_path,
                opaque_content: true,
                derived_bytes: None,
            }],
        },
    )
}

fn add_blocked_project_content_candidate(
    discovery: &mut Discovery,
    relative: &str,
    reason: &str,
) -> Result<(), String> {
    validate_logical_project_relative(relative)?;
    discovery.blocked_count = discovery.blocked_count.saturating_add(1);
    let resource_id = project_content_resource_id("file", relative);
    let mut metadata = BTreeMap::new();
    metadata.insert("entry_type".to_string(), "blocked".to_string());
    metadata.insert("_local_relative_path".to_string(), relative.to_string());
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                resource_id,
                kind: CaptureResourceKind::ProjectContentFile,
                provider: None,
                display_name: relative.to_string(),
                relative_cwd: None,
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: false,
                blocked_reason: Some(reason.to_string()),
                logical_paths: vec![format!("project/{}", relative)],
                dependency: None,
                metadata,
            },
            files: Vec::new(),
        },
    )
}

fn project_content_resource_id(entry_type: &str, relative: &str) -> String {
    let digest = sha256_bytes(format!("{}\0{}", entry_type, relative).as_bytes());
    match entry_type {
        "dir" => format!("project:content-dir:{}", digest),
        _ => format!("project:content-file:{}", digest),
    }
}

fn project_content_review_digest(
    entry_type: &str,
    relative: &str,
    content_sha256: &str,
    mode: u32,
) -> String {
    sha256_bytes(
        format!(
            "{}\0{}\0{}\0{:03o}",
            entry_type, relative, content_sha256, mode
        )
        .as_bytes(),
    )
}

fn hard_excluded_project_content_path(relative: &str) -> bool {
    relative.split('/').any(|component| {
        matches!(
            component.to_ascii_lowercase().as_str(),
            ".git"
                | ".hg"
                | ".svn"
                | ".mallard"
                | ".agent-sync"
                | ".codex-sync"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".cache"
                | "__pycache__"
        )
    })
}

fn load_project_ignore_rules(
    root: &Path,
    warnings: &mut Vec<String>,
) -> Result<Vec<ProjectIgnoreRule>, String> {
    let mut rules = Vec::new();
    for name in [".gitignore", ".ignore", ".mallardignore"] {
        let path = root.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("inspect '{}': {}", path.display(), error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            warnings.push(format!("Ignore file '{}' is not a regular file", name));
            continue;
        }
        if metadata.len() > MAX_METADATA_BYTES as u64 {
            warnings.push(format!("Ignore file '{}' exceeds the metadata limit", name));
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("read '{}': {}", path.display(), error))?;
        for raw in text.lines().take(MAX_METADATA_LINES.saturating_mul(8)) {
            let mut line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let negated = line.starts_with('!');
            if negated {
                line = &line[1..];
            }
            if line.is_empty() || line.chars().any(char::is_control) {
                continue;
            }
            let rooted = line.starts_with('/');
            if rooted {
                line = &line[1..];
            }
            let directory_only = line.ends_with('/');
            if directory_only {
                line = line.trim_end_matches('/');
            }
            if line.is_empty() {
                continue;
            }
            rules.push(ProjectIgnoreRule {
                pattern: line.replace('\\', "/"),
                negated,
                directory_only,
                rooted,
            });
            if rules.len() > 8_192 {
                return Err("project ignore rules exceed 8192 entries".to_string());
            }
        }
    }
    Ok(rules)
}

fn project_path_is_ignored(
    rules: &[ProjectIgnoreRule],
    relative: &str,
    is_directory: bool,
) -> bool {
    let mut ignored = false;
    for rule in rules {
        if rule.directory_only && !is_directory {
            let prefix_match = relative
                .split('/')
                .scan(String::new(), |prefix, component| {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(component);
                    Some(prefix.clone())
                })
                .any(|prefix| ignore_rule_matches(rule, &prefix));
            if !prefix_match {
                continue;
            }
        } else if !ignore_rule_matches(rule, relative) {
            continue;
        }
        ignored = !rule.negated;
    }
    ignored
}

fn ignore_rule_matches(rule: &ProjectIgnoreRule, relative: &str) -> bool {
    if rule.rooted || rule.pattern.contains('/') {
        glob_path_matches(&rule.pattern, relative)
            || relative
                .strip_prefix(&format!("{}/", rule.pattern.trim_end_matches('/')))
                .is_some()
    } else {
        relative
            .split('/')
            .any(|component| glob_path_matches(&rule.pattern, component))
    }
}

fn glob_path_matches(pattern: &str, value: &str) -> bool {
    fn matches_from(
        pattern: &[u8],
        value: &[u8],
        pattern_index: usize,
        value_index: usize,
        memo: &mut BTreeMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(pattern_index, value_index)) {
            return *result;
        }
        let result = if pattern_index == pattern.len() {
            value_index == value.len()
        } else if pattern[pattern_index] == b'*' {
            let double_star = pattern.get(pattern_index + 1) == Some(&b'*');
            let next_pattern = pattern_index + if double_star { 2 } else { 1 };
            matches_from(pattern, value, next_pattern, value_index, memo)
                || (value_index < value.len()
                    && (double_star || value[value_index] != b'/')
                    && matches_from(pattern, value, pattern_index, value_index + 1, memo))
        } else if value_index < value.len()
            && (pattern[pattern_index] == b'?' && value[value_index] != b'/'
                || pattern[pattern_index] == value[value_index])
        {
            matches_from(pattern, value, pattern_index + 1, value_index + 1, memo)
        } else {
            false
        };
        memo.insert((pattern_index, value_index), result);
        result
    }
    matches_from(
        pattern.as_bytes(),
        value.as_bytes(),
        0,
        0,
        &mut BTreeMap::new(),
    )
}

fn dependency_resource(
    resource_id: String,
    display_name: String,
    provider: Provider,
    kind: CaptureResourceKind,
    action: DependencyAction,
) -> DiscoveredResource {
    DiscoveredResource {
        descriptor: ResourceCandidate {
            metadata: BTreeMap::new(),
            resource_id,
            kind,
            provider: Some(provider),
            display_name,
            relative_cwd: None,
            apply_policy: CaptureApplyPolicy::Dependency,
            selected_by_default: false,
            blocked_reason: None,
            logical_paths: Vec::new(),
            dependency: Some(action),
        },
        files: Vec::new(),
    }
}

fn discover_codex(
    home: &Path,
    project_root: &Path,
    excluded_roots: &[PathBuf],
    discovery: &mut Discovery,
) -> Result<(), String> {
    let canonical_home = canonical_existing_dir(home, "Codex home")?;
    let mut selected_thread_ids = BTreeSet::new();
    for directory in ["sessions", "archived_sessions"] {
        let state_root = canonical_home.join(directory);
        if !state_root.exists() {
            continue;
        }
        for entry in WalkDir::new(&state_root)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
            .filter_entry(provider_walk_entry)
        {
            let entry = entry.map_err(|e| format!("walk Codex {}: {}", directory, e))?;
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            let meta = match scan_jsonl_metadata(entry.path(), Provider::Codex) {
                Ok(meta) => meta,
                Err(error) => {
                    discovery.warnings.push(format!(
                        "ignored Codex session '{}': {}",
                        entry.path().display(),
                        error
                    ));
                    continue;
                }
            };
            let Some(relative_cwd) = owned_relative_cwd(project_root, &meta.cwd, excluded_roots)?
            else {
                continue;
            };
            let session_id = resource_id_component(&meta.session_id);
            let resource_id = format!("codex:session:{}", session_id);
            if meta.is_internal_subagent {
                // TODO(sync-subagent-task): Parent/child rollout graphs need
                // atomic selection and complete parent-ID rewriting before
                // they can be portable. See docs/TODO.md.
                discovery
                    .excluded_codex_subagent_ids
                    .insert(meta.session_id);
                if discovery
                    .excluded_codex_subagent_resource_ids
                    .insert(resource_id)
                {
                    discovery.ignored_count = discovery.ignored_count.saturating_add(1);
                }
                continue;
            }
            selected_thread_ids.insert(meta.session_id.clone());
            let state_relative = normalized_relative(&state_root, entry.path())?;
            let logical_path = format!("state/codex/{}/{}", directory, state_relative);

            // Codex can briefly leave the same transcript in both `sessions`
            // and `archived_sessions` while an archive operation settles. A
            // session ID is the stable resource identity, so keep the active
            // `sessions` copy discovered first and ignore later copies. This
            // must not make an otherwise valid Pull review unusable.
            if directory == "archived_sessions" {
                if let Some(existing_path) = discovery
                    .resources
                    .get(&resource_id)
                    .and_then(|resource| resource.files.first())
                    .map(|file| file.physical_path.display().to_string())
                {
                    discovery.warnings.push(format!(
                        "ignored duplicate Codex session '{}' at '{}'; using '{}'",
                        meta.session_id,
                        entry.path().display(),
                        existing_path,
                    ));
                    continue;
                }
            }

            insert_resource(
                discovery,
                DiscoveredResource {
                    descriptor: ResourceCandidate {
                        metadata: BTreeMap::new(),
                        resource_id: resource_id.clone(),
                        kind: CaptureResourceKind::Conversation,
                        provider: Some(Provider::Codex),
                        display_name: meta.session_id,
                        relative_cwd: Some(relative_cwd),
                        apply_policy: CaptureApplyPolicy::SafeFile,
                        selected_by_default: true,
                        blocked_reason: inspect_candidate_files_with_limit(
                            &canonical_home,
                            &[entry.path().to_path_buf()],
                            domain::MAX_CONVERSATION_FILE_BYTES,
                        )
                        .err(),
                        logical_paths: vec![logical_path.clone()],
                        dependency: None,
                    },
                    files: vec![SourceFile {
                        physical_path: entry.path().to_path_buf(),
                        logical_path,
                        opaque_content: true,
                        derived_bytes: None,
                    }],
                },
            )?;
        }
    }

    let index = canonical_home.join("session_index.jsonl");
    if !selected_thread_ids.is_empty() && index.is_file() {
        // The index is local provider state, not a portable resource. Read only
        // the selected rows to enrich the typed conversation titles; Pull
        // materialization maintains the destination index transactionally.
        let filtered = filter_jsonl_by_ids(&index, &selected_thread_ids)?;
        if !filtered.is_empty() {
            for (thread_id, title) in parse_session_index_titles(&filtered) {
                let resource_id = format!("codex:session:{}", resource_id_component(&thread_id));
                if let Some(resource) = discovery.resources.get_mut(&resource_id) {
                    resource.descriptor.display_name = title;
                }
            }
        }
    }
    Ok(())
}

fn discover_claude(
    home: &Path,
    project_root: &Path,
    excluded_roots: &[PathBuf],
    discovery: &mut Discovery,
) -> Result<(), String> {
    let canonical_home = canonical_existing_dir(home, "Claude home")?;
    let projects = canonical_home.join("projects");
    if !projects.exists() {
        return Ok(());
    }
    let mut session_ids = BTreeSet::new();
    for entry in WalkDir::new(&projects)
        .follow_links(false)
        .min_depth(2)
        .max_depth(3)
        .into_iter()
        .filter_entry(provider_walk_entry)
    {
        let entry = entry.map_err(|e| format!("walk Claude projects: {}", e))?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
        {
            continue;
        }
        let meta = match scan_jsonl_metadata(entry.path(), Provider::Claude) {
            Ok(meta) => meta,
            Err(error) => {
                discovery.warnings.push(format!(
                    "ignored Claude session '{}': {}",
                    entry.path().display(),
                    error
                ));
                continue;
            }
        };
        let Some(relative_cwd) = owned_relative_cwd(project_root, &meta.cwd, excluded_roots)?
        else {
            continue;
        };
        session_ids.insert(meta.session_id.clone());
        let relative_key = relative_cwd_key(&relative_cwd);
        let file_name = entry
            .path()
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("non-UTF-8 Claude session name: {}", entry.path().display()))?;
        let logical_path = format!("state/claude/projects/{}/{}", relative_key, file_name);
        let resource_id = format!("claude:session:{}", resource_id_component(&meta.session_id));
        insert_resource(
            discovery,
            DiscoveredResource {
                descriptor: ResourceCandidate {
                    metadata: BTreeMap::new(),
                    resource_id: resource_id.clone(),
                    kind: CaptureResourceKind::Conversation,
                    provider: Some(Provider::Claude),
                    display_name: meta.session_id,
                    relative_cwd: Some(relative_cwd),
                    apply_policy: CaptureApplyPolicy::SafeFile,
                    selected_by_default: true,
                    blocked_reason: inspect_candidate_files_with_limit(
                        &canonical_home,
                        &[entry.path().to_path_buf()],
                        domain::MAX_CONVERSATION_FILE_BYTES,
                    )
                    .err(),
                    logical_paths: vec![logical_path.clone()],
                    dependency: None,
                },
                files: vec![SourceFile {
                    physical_path: entry.path().to_path_buf(),
                    logical_path,
                    opaque_content: true,
                    derived_bytes: None,
                }],
            },
        )?;
    }

    // Sidecars are associated only through an already-proven session ID.
    for (directory, kind) in [
        ("file-history", CaptureResourceKind::Conversation),
        ("todos", CaptureResourceKind::Conversation),
    ] {
        let sidecar_root = canonical_home.join(directory);
        if !sidecar_root.exists() {
            continue;
        }
        for entry in WalkDir::new(&sidecar_root)
            .follow_links(false)
            .max_depth(4)
            .into_iter()
            .filter_entry(provider_walk_entry)
        {
            let entry = entry.map_err(|e| format!("walk Claude {}: {}", directory, e))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = normalized_relative(&sidecar_root, entry.path())?;
            let Some(session_id) = session_ids
                .iter()
                .find(|id| relative.contains(id.as_str()))
                .cloned()
            else {
                continue;
            };
            let resource_id = format!(
                "claude:{}:{}",
                directory,
                resource_id_component(&session_id)
            );
            let logical_path = format!("state/claude/{}/{}", directory, relative);
            insert_resource(
                discovery,
                DiscoveredResource {
                    descriptor: ResourceCandidate {
                        metadata: BTreeMap::new(),
                        resource_id: resource_id.clone(),
                        kind: kind.clone(),
                        provider: Some(Provider::Claude),
                        display_name: format!("Claude {} for {}", directory, session_id),
                        relative_cwd: None,
                        apply_policy: CaptureApplyPolicy::SafeFile,
                        selected_by_default: true,
                        blocked_reason: inspect_candidate_files_with_limit(
                            &canonical_home,
                            &[entry.path().to_path_buf()],
                            domain::MAX_CONVERSATION_FILE_BYTES,
                        )
                        .err(),
                        logical_paths: vec![logical_path.clone()],
                        dependency: None,
                    },
                    files: vec![SourceFile {
                        physical_path: entry.path().to_path_buf(),
                        logical_path,
                        opaque_content: true,
                        derived_bytes: None,
                    }],
                },
            )?;
        }
    }

    // Project memories live within a verified bucket. We discover them only
    // when a session in that bucket proved project ownership.
    let mut bucket_roots = BTreeMap::<PathBuf, String>::new();
    for resource in discovery.resources.values() {
        if resource.descriptor.provider == Some(Provider::Claude)
            && resource.descriptor.kind == CaptureResourceKind::Conversation
        {
            for source in &resource.files {
                if source.physical_path.starts_with(&projects) {
                    if let Some(parent) = source.physical_path.parent() {
                        if let Some(relative_cwd) = &resource.descriptor.relative_cwd {
                            bucket_roots.insert(parent.to_path_buf(), relative_cwd.clone());
                        }
                    }
                }
            }
        }
    }
    for (bucket, relative_cwd) in bucket_roots {
        let memory = bucket.join("memory");
        if !memory.exists() {
            continue;
        }
        let files = collect_regular_tree_files(&canonical_home, memory)?;
        if files.is_empty() {
            continue;
        }
        let relative_key = relative_cwd_key(&relative_cwd);
        let resource_id = format!("claude:memory:{}", resource_id_component(&relative_cwd));
        let logical_prefix = format!("state/claude/memory/{}", relative_key);
        let logical_paths = files
            .iter()
            .map(|path| {
                normalized_relative(&bucket.join("memory"), path)
                    .map(|rel| format!("{}/{}", logical_prefix, rel))
            })
            .collect::<Result<Vec<_>, _>>()?;
        insert_resource(
            discovery,
            DiscoveredResource {
                descriptor: ResourceCandidate {
                    metadata: BTreeMap::new(),
                    resource_id: resource_id.clone(),
                    kind: CaptureResourceKind::Memory,
                    provider: Some(Provider::Claude),
                    display_name: "Claude project memory".to_string(),
                    relative_cwd: Some(relative_cwd),
                    apply_policy: CaptureApplyPolicy::Merge,
                    selected_by_default: true,
                    blocked_reason: inspect_candidate_files(&canonical_home, &files).err(),
                    logical_paths: logical_paths.clone(),
                    dependency: None,
                },
                files: files
                    .into_iter()
                    .zip(logical_paths)
                    .map(|(physical_path, logical_path)| SourceFile {
                        physical_path,
                        logical_path,
                        opaque_content: true,
                        derived_bytes: None,
                    })
                    .collect(),
            },
        )?;
    }
    Ok(())
}

fn discover_standalone_skill(
    skill: &StandaloneSkillSource,
    project_root: &Path,
    discovery: &mut Discovery,
) -> Result<(), String> {
    domain::validate_skill_name("standalone skill effective name", &skill.effective_name)?;
    domain::validate_skill_name(
        "standalone skill install directory",
        &skill.install_dir_name,
    )?;
    if skill.stable_key.is_empty() || skill.stable_key.len() > 1024 {
        return Err(format!(
            "invalid stable provenance for skill '{}'",
            skill.effective_name
        ));
    }
    let canonical_source = canonical_existing_dir(&skill.source_dir, "standalone skill")?;
    if canonical_source.starts_with(project_root) {
        return Err(format!(
            "standalone skill '{}' is already project-local",
            skill.source_dir.display()
        ));
    }
    let files = collect_regular_tree_files(&canonical_source, canonical_source.clone())?;
    // Global custom skills restore into their original physical directory
    // below the mapped provider home's `skills/` root. Runtime identity is
    // independent and comes from `effective_name`.
    let target_root = format!(
        "state/{}/skills/{}",
        provider_name(skill.provider),
        skill.install_dir_name
    );
    let logical_paths = files
        .iter()
        .map(|path| {
            normalized_relative(&canonical_source, path)
                .map(|rel| format!("{}/{}", target_root, rel))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Identity is provider + effective skill name: editing content preserves
    // it, renaming the skill is an explicit remove/add. The stable key stays
    // provenance metadata only.
    let resource_id = format!(
        "{}:standalone-skill:{}",
        provider_name(skill.provider),
        resource_id_component(&skill.effective_name)
    );
    let executable = files.iter().any(|path| is_executable_path(path));
    let dependency = executable.then(|| DependencyAction {
        action_id: format!("dependency:{}", resource_id),
        resource_id: resource_id.clone(),
        provider: skill.provider,
        kind: DependencyKind::StandaloneSkill,
        scope: DependencyScope::ProviderHome,
        display_name: skill.effective_name.clone(),
        program: None,
        argv: Vec::new(),
        requires_review: true,
        payload_logical_prefix: Some(target_root.clone()),
    });
    let mut metadata = BTreeMap::new();
    metadata.insert("stable_key".to_string(), skill.stable_key.clone());
    metadata.insert("effective_name".to_string(), skill.effective_name.clone());
    metadata.insert(
        "install_dir_name".to_string(),
        skill.install_dir_name.clone(),
    );
    metadata.insert(
        "provider_adapter_version".to_string(),
        global_inventory::PROVIDER_ADAPTER_VERSION.to_string(),
    );
    metadata.insert(
        "ownership_evidence".to_string(),
        "no plugin claim; regular global skills directory".to_string(),
    );
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                metadata,
                resource_id,
                kind: CaptureResourceKind::StandaloneSkill,
                provider: Some(skill.provider),
                display_name: skill.effective_name.clone(),
                relative_cwd: None,
                apply_policy: if executable {
                    CaptureApplyPolicy::Dependency
                } else {
                    CaptureApplyPolicy::SafeFile
                },
                selected_by_default: false,
                blocked_reason: inspect_candidate_files(&canonical_source, &files).err(),
                logical_paths: logical_paths.clone(),
                dependency,
            },
            files: files
                .into_iter()
                .zip(logical_paths)
                .map(|(physical_path, logical_path)| SourceFile {
                    physical_path,
                    logical_path,
                    opaque_content: true,
                    derived_bytes: None,
                })
                .collect(),
        },
    )
}

/// Register one inventoried global plugin as portable install intent.
fn discover_global_plugin(
    plugin: &global_inventory::GlobalPluginSource,
    discovery: &mut Discovery,
) -> Result<(), String> {
    let resource_id = format!(
        "{}:plugin:{}",
        provider_name(plugin.provider),
        resource_id_component(&plugin.plugin_id)
    );
    let mut metadata = BTreeMap::new();
    metadata.insert("plugin_origin".to_string(), "global".to_string());
    if let Some(marketplace) = &plugin.marketplace {
        metadata.insert("plugin_marketplace".to_string(), marketplace.clone());
    }
    if let Some(source_type) = &plugin.source_type {
        metadata.insert("plugin_source_type".to_string(), source_type.clone());
    }
    if let Some(source) = &plugin.source {
        metadata.insert("plugin_source".to_string(), source.clone());
    }
    if let Some(version) = &plugin.observed_version {
        // Observational only: install may resolve a different marketplace
        // version unless the native CLI can pin exactly.
        metadata.insert("plugin_observed_version".to_string(), version.clone());
    }
    metadata.insert("plugin_enabled".to_string(), plugin.enabled.to_string());
    if !plugin.provided_skills.is_empty() {
        metadata.insert(
            "plugin_provided_skills_json".to_string(),
            serde_json::to_string(&plugin.provided_skills).map_err(|e| e.to_string())?,
        );
    }
    let (kind, program, argv) = match plugin.provider {
        Provider::Codex => (
            DependencyKind::CodexPlugin,
            "codex",
            vec![
                "plugin".to_string(),
                "add".to_string(),
                plugin.plugin_id.clone(),
            ],
        ),
        Provider::Claude => (
            DependencyKind::ClaudePlugin,
            "claude",
            vec![
                "plugin".to_string(),
                "install".to_string(),
                plugin.plugin_id.clone(),
            ],
        ),
    };
    let action = DependencyAction {
        action_id: format!("dependency:{}", resource_id),
        resource_id: resource_id.clone(),
        provider: plugin.provider,
        kind,
        scope: DependencyScope::ProviderHome,
        display_name: plugin.plugin_id.clone(),
        program: Some(program.to_string()),
        argv,
        requires_review: true,
        payload_logical_prefix: None,
    };
    let mut resource = dependency_resource(
        resource_id,
        plugin.plugin_id.clone(),
        plugin.provider,
        CaptureResourceKind::Plugin,
        action,
    );
    resource.descriptor.metadata = metadata;
    insert_resource(discovery, resource)
}

/// Surface an unclassifiable global skill candidate as a blocked resource so
/// review can show the ownership evidence. It carries no files and can never
/// be captured.
fn discover_blocked_global_skill(
    blocked: &global_inventory::BlockedSkillCandidate,
    discovery: &mut Discovery,
) -> Result<(), String> {
    let resource_id = format!(
        "{}:standalone-skill:{}",
        provider_name(blocked.provider),
        resource_id_component(&blocked.name)
    );
    if discovery.resources.contains_key(&resource_id) {
        return Ok(());
    }
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                metadata: BTreeMap::new(),
                resource_id,
                kind: CaptureResourceKind::StandaloneSkill,
                provider: Some(blocked.provider),
                display_name: blocked.name.clone(),
                relative_cwd: None,
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: false,
                blocked_reason: Some(blocked.reason.clone()),
                logical_paths: Vec::new(),
                dependency: None,
            },
            files: Vec::new(),
        },
    )
}

#[derive(Debug)]
pub(crate) struct SessionMetadata {
    pub(crate) session_id: String,
    pub(crate) cwd: PathBuf,
    pub(crate) is_internal_subagent: bool,
}

pub(crate) fn scan_jsonl_metadata(
    path: &Path,
    provider: Provider,
) -> Result<SessionMetadata, String> {
    let file = fs::File::open(path).map_err(|e| format!("open: {}", e))?;
    let mut reader = BufReader::new(file);
    let mut total = 0_usize;
    let mut fallback_id = None;
    let mut fallback_cwd = None;
    let mut fallback_is_internal_subagent = false;
    for _ in 0..MAX_METADATA_LINES {
        let mut line = Vec::new();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("read metadata: {}", e))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read);
        if total > MAX_METADATA_BYTES {
            break;
        }
        if line.len() > 256 * 1024 {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let event_type = find_string_for_keys(&value, &["type"])
            .unwrap_or_default()
            .to_ascii_lowercase();
        let cwd = find_string_for_keys(
            &value,
            &[
                "cwd",
                "working_directory",
                "workingDirectory",
                "projectPath",
            ],
        );
        let authoritative = match provider {
            Provider::Codex => event_type.contains("session_meta"),
            Provider::Claude => {
                event_type.contains("system")
                    || event_type.contains("init")
                    || value.get("sessionId").is_some()
            }
        };
        let is_internal_subagent =
            provider == Provider::Codex && session_meta_is_internal_subagent(&value);
        let id = match provider {
            // Modern Codex subagent metadata carries both `id` (this
            // transcript) and `session_id` (its parent). Prefer `id` on the
            // authoritative session_meta record so parent and child sessions
            // remain distinct resources. Keep the older fallback precedence
            // for non-authoritative compatibility records.
            Provider::Codex if authoritative => find_string_for_keys(
                &value,
                &["id", "thread_id", "threadId", "session_id", "sessionId"],
            ),
            Provider::Codex => find_string_for_keys(
                &value,
                &["thread_id", "threadId", "session_id", "sessionId", "id"],
            ),
            Provider::Claude => find_string_for_keys(
                &value,
                &["sessionId", "session_id", "threadId", "thread_id", "id"],
            ),
        };
        if fallback_cwd.is_none() {
            fallback_cwd = cwd.clone();
        }
        if fallback_id.is_none() {
            fallback_id = id.clone();
            fallback_is_internal_subagent = is_internal_subagent;
        }
        if authoritative {
            if let (Some(id), Some(cwd)) = (id, cwd) {
                return validated_session_metadata(id, cwd, is_internal_subagent);
            }
        }
    }
    let id =
        fallback_id.ok_or_else(|| "session ID was not found in bounded metadata".to_string())?;
    let cwd = fallback_cwd.ok_or_else(|| "cwd was not found in bounded metadata".to_string())?;
    validated_session_metadata(id, cwd, fallback_is_internal_subagent)
}

fn validated_session_metadata(
    id: String,
    cwd: String,
    is_internal_subagent: bool,
) -> Result<SessionMetadata, String> {
    if id.is_empty()
        || id.len() > 512
        || id.chars().any(char::is_control)
        || cwd.is_empty()
        || cwd.len() > 4096
        || cwd.chars().any(char::is_control)
    {
        return Err("invalid session ID or cwd".to_string());
    }
    let cwd = PathBuf::from(cwd);
    if !cwd.is_absolute() {
        return Err("session cwd is not absolute".to_string());
    }
    Ok(SessionMetadata {
        session_id: id,
        cwd,
        is_internal_subagent,
    })
}

fn find_string_for_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(text) = map.get(*key).and_then(serde_json::Value::as_str) {
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
            // Provider metadata normally lives in `payload`; searching
            // recursively also accommodates recorded older codec fixtures.
            for nested in map.values() {
                if let Some(value) = find_string_for_keys(nested, keys) {
                    return Some(value);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|nested| find_string_for_keys(nested, keys)),
        _ => None,
    }
}

fn owned_relative_cwd(
    project_root: &Path,
    cwd: &Path,
    excluded_roots: &[PathBuf],
) -> Result<Option<String>, String> {
    let canonical_cwd = match fs::canonicalize(cwd) {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    if canonical_cwd != project_root && !canonical_cwd.starts_with(project_root) {
        return Ok(None);
    }
    if excluded_roots
        .iter()
        .any(|excluded| canonical_cwd == *excluded || canonical_cwd.starts_with(excluded))
    {
        return Ok(None);
    }
    let relative = canonical_cwd
        .strip_prefix(project_root)
        .map_err(|_| "session cwd membership changed during discovery".to_string())?;
    if relative.as_os_str().is_empty() {
        Ok(Some(".".to_string()))
    } else {
        normalized_path(relative).map(Some)
    }
}

fn filter_jsonl_by_ids(path: &Path, ids: &BTreeSet<String>) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("inspect '{}': {}", path.display(), e))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "'{}' exceeds {} bytes",
            path.display(),
            MAX_FILE_BYTES
        ));
    }
    let file = fs::File::open(path).map_err(|e| format!("open '{}': {}", path.display(), e))?;
    let mut output = Vec::new();
    for line in BufReader::new(file).split(b'\n') {
        let line = line.map_err(|e| format!("read '{}': {}", path.display(), e))?;
        if line.len() > 1024 * 1024 {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let matches = ["thread_id", "threadId", "session_id", "sessionId", "id"]
            .iter()
            .filter_map(|key| find_string_for_keys(&value, &[*key]))
            .any(|value| ids.contains(&value));
        if matches {
            output.extend_from_slice(&line);
            output.push(b'\n');
        }
    }
    Ok(output)
}

fn insert_resource(discovery: &mut Discovery, resource: DiscoveredResource) -> Result<(), String> {
    let id = resource.descriptor.resource_id.clone();
    if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
        return Err(format!("invalid resource ID '{}'", id));
    }
    if discovery.resources.insert(id.clone(), resource).is_some() {
        return Err(format!("duplicate resource ID '{}'", id));
    }
    Ok(())
}

fn inspect_candidate_files(approved_root: &Path, files: &[PathBuf]) -> Result<(), String> {
    inspect_candidate_files_with_limit(approved_root, files, MAX_FILE_BYTES)
}

fn inspect_candidate_files_with_limit(
    approved_root: &Path,
    files: &[PathBuf],
    max_file_bytes: u64,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("resource has no regular files".to_string());
    }
    if files.len() > MAX_DISCOVERED_FILES {
        return Err(format!("resource exceeds {} files", MAX_DISCOVERED_FILES));
    }
    let canonical_root = fs::canonicalize(approved_root)
        .map_err(|e| format!("resolve '{}': {}", approved_root.display(), e))?;
    for file in files {
        reject_unsafe_ancestors(file, &canonical_root)?;
        let meta = fs::symlink_metadata(file)
            .map_err(|e| format!("inspect '{}': {}", file.display(), e))?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!(
                "'{}' is not a regular no-follow file",
                file.display()
            ));
        }
        if meta.len() > max_file_bytes {
            return Err(format!(
                "'{}' exceeds {} bytes",
                file.display(),
                max_file_bytes
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() > 1 {
                return Err(format!(
                    "hard-linked file '{}' is not portable",
                    file.display()
                ));
            }
        }
        let relative = file
            .strip_prefix(&canonical_root)
            .or_else(|_| file.strip_prefix(approved_root))
            .map_err(|_| format!("'{}' escapes approved root", file.display()))?;
        for component in relative.components() {
            if let Component::Normal(name) = component {
                let name = name
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 path '{}'", file.display()))?;
                if denied_file_name(name) {
                    return Err(format!(
                        "credential/cache file '{}' is structurally excluded",
                        file.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_source_file(
    path: &Path,
    canonical_project: &Path,
    request: &CaptureRequest,
    resource_id: &str,
    max_file_bytes: u64,
) -> Result<(), String> {
    let canonical_path = fs::canonicalize(path)
        .map_err(|e| format!("resolve source '{}': {}", path.display(), e))?;
    let mut approved_roots = Vec::new();
    if request.include_project_content {
        approved_roots.push(canonical_project.to_path_buf());
    }
    if let Some(home) = &request.codex_home {
        approved_roots.push(canonical_existing_dir(home, "Codex home")?);
    }
    if let Some(home) = &request.claude_home {
        approved_roots.push(canonical_existing_dir(home, "Claude home")?);
    }
    for skill in &request.standalone_skills {
        approved_roots.push(canonical_existing_dir(
            &skill.source_dir,
            "standalone skill",
        )?);
    }
    let Some(approved_root) = approved_roots
        .into_iter()
        .filter(|root| canonical_path.starts_with(root))
        .max_by_key(|root| root.components().count())
    else {
        return Err(format!(
            "resource '{}' source '{}' is outside approved roots",
            resource_id,
            path.display()
        ));
    };
    inspect_candidate_files_with_limit(&approved_root, &[path.to_path_buf()], max_file_bytes)
}

fn validate_source_directory(
    path: &Path,
    canonical_project: &Path,
    request: &CaptureRequest,
    resource_id: &str,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect directory '{}': {}", path.display(), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "resource '{}' source '{}' is not a real no-follow directory",
            resource_id,
            path.display()
        ));
    }
    reject_unsafe_ancestors(path, canonical_project)?;
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("resolve directory '{}': {}", path.display(), error))?;
    if !canonical.starts_with(canonical_project)
        || request
            .excluded_project_roots
            .iter()
            .chain(request.excluded_content_roots.iter())
            .any(|excluded| {
                fs::canonicalize(excluded).is_ok_and(|excluded| canonical.starts_with(excluded))
            })
    {
        return Err(format!(
            "resource '{}' directory '{}' is outside approved project content",
            resource_id,
            path.display()
        ));
    }
    Ok(())
}

fn collect_regular_tree_files(approved_root: &Path, tree: PathBuf) -> Result<Vec<PathBuf>, String> {
    let meta =
        fs::symlink_metadata(&tree).map_err(|e| format!("inspect '{}': {}", tree.display(), e))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "symlinked resource '{}' is not captured",
            tree.display()
        ));
    }
    if meta.is_file() {
        inspect_candidate_files(approved_root, &[tree.clone()])?;
        return Ok(vec![tree]);
    }
    if !meta.is_dir() {
        return Err(format!(
            "special resource '{}' is not captured",
            tree.display()
        ));
    }
    preflight_resource_tree(&tree)?;
    let mut result = Vec::new();
    for entry in WalkDir::new(&tree)
        .follow_links(false)
        .max_depth(MAX_TREE_DEPTH)
        .into_iter()
        .filter_entry(resource_walk_entry)
    {
        let entry = entry.map_err(|e| format!("walk '{}': {}", tree.display(), e))?;
        if entry.depth() == 0 {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err(format!(
                "symlink '{}' blocks resource capture",
                entry.path().display()
            ));
        }
        if entry.file_type().is_file() {
            result.push(entry.path().to_path_buf());
            if result.len() > MAX_DISCOVERED_FILES {
                return Err(format!("resource '{}' has too many files", tree.display()));
            }
        } else if !entry.file_type().is_dir() {
            return Err(format!(
                "special file '{}' blocks resource capture",
                entry.path().display()
            ));
        }
    }
    result.sort();
    inspect_candidate_files(approved_root, &result)?;
    Ok(result)
}

fn preflight_resource_tree(tree: &Path) -> Result<(), String> {
    let mut walker = WalkDir::new(tree)
        .follow_links(false)
        .max_depth(MAX_TREE_DEPTH)
        .into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|e| format!("walk '{}': {}", tree.display(), e))?;
        if entry.depth() == 0 {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if matches!(name.as_str(), ".git" | ".svn" | ".hg") {
            return Err(format!(
                "nested VCS metadata '{}' blocks resource capture",
                entry.path().display()
            ));
        }
        if entry.file_type().is_symlink() {
            return Err(format!(
                "symlink '{}' blocks resource capture",
                entry.path().display()
            ));
        }
        if !entry.file_type().is_file() && !entry.file_type().is_dir() {
            return Err(format!(
                "special file '{}' blocks resource capture",
                entry.path().display()
            ));
        }
        if entry.file_type().is_dir()
            && matches!(
                name.as_str(),
                "node_modules"
                    | "target"
                    | "cache"
                    | "caches"
                    | "plugins"
                    | "marketplaces"
                    | "repos"
            )
        {
            walker.skip_current_dir();
        }
    }
    Ok(())
}

fn reject_unsafe_ancestors(path: &Path, root: &Path) -> Result<(), String> {
    let canonical_path =
        fs::canonicalize(path).map_err(|e| format!("resolve '{}': {}", path.display(), e))?;
    if !canonical_path.starts_with(root) {
        return Err(format!(
            "'{}' resolves outside '{}'",
            path.display(),
            root.display()
        ));
    }
    let relative = canonical_path
        .strip_prefix(root)
        .map_err(|_| format!("'{}' escapes '{}'", path.display(), root.display()))?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        let meta = fs::symlink_metadata(&cursor)
            .map_err(|e| format!("inspect '{}': {}", cursor.display(), e))?;
        if meta.file_type().is_symlink() {
            return Err(format!("symlink traversal at '{}'", cursor.display()));
        }
    }
    Ok(())
}

fn canonical_existing_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{} must be absolute: '{}'", label, path.display()));
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("inspect {} '{}': {}", label, path.display(), e))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} '{}' must be a real directory",
            label,
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|e| format!("resolve {} '{}': {}", label, path.display(), e))
}

fn normalized_relative(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("'{}' is outside '{}'", path.display(), root.display()))?;
    normalized_path(relative)
}

fn normalized_path(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| "path contains non-UTF-8 component".to_string())?;
                if value.is_empty()
                    || value == "."
                    || value == ".."
                    || value.contains(['/', '\\'])
                    || value.chars().any(char::is_control)
                {
                    return Err(format!("unsafe path component '{}'", value));
                }
                parts.push(value);
            }
            _ => return Err(format!("unsafe relative path '{}'", path.display())),
        }
    }
    if parts.is_empty() {
        return Err("empty relative path".to_string());
    }
    Ok(parts.join("/"))
}

fn validate_logical_project_relative(value: &str) -> Result<(), String> {
    validate_logical_path(&format!("project/{}", value))
}

pub fn validate_logical_path(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || value.starts_with('/') || value.ends_with('/') {
        return Err(format!("invalid logical path '{}'", value));
    }
    if value.contains('\\') || value.chars().any(char::is_control) {
        return Err(format!("invalid logical path '{}'", value));
    }
    for component in value.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > 255
            || is_windows_reserved(component)
        {
            return Err(format!("unsafe logical path component '{}'", component));
        }
    }
    Ok(())
}

fn is_windows_reserved(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches([' ', '.'])
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn provider_walk_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    !matches!(
        name.as_str(),
        ".git"
            | "cache"
            | "caches"
            | "tmp"
            | "temp"
            | "logs"
            | "plugins"
            | "marketplaces"
            | "repos"
    )
}

fn resource_walk_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !denied_file_name(&name)
        && name != ".git"
        && name != ".svn"
        && name != ".hg"
        && !matches!(
            name.to_ascii_lowercase().as_str(),
            "node_modules" | "target" | "cache" | "caches" | "plugins" | "marketplaces" | "repos"
        )
}

fn denied_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower == "auth.json"
        || lower == "credentials.json"
        || lower == "credentials"
        || lower == "oauth.json"
        || lower == "tokens.json"
        || lower == ".npmrc"
        || lower == ".netrc"
        || lower == "id_rsa"
        || lower == "id_ed25519"
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || matches!(
            lower.as_str(),
            "cache" | "caches" | "plugins" | "marketplaces" | "repos"
        )
}

fn detect_secret_material(bytes: &[u8]) -> Option<&'static str> {
    let text = std::str::from_utf8(bytes).ok()?;
    let lower = text.to_ascii_lowercase();
    if lower.contains("-----begin private key-----")
        || lower.contains("-----begin rsa private key-----")
        || lower.contains("-----begin openssh private key-----")
    {
        return Some("private key material");
    }
    for marker in [
        "aws_secret_access_key",
        "authorization: bearer ",
        "\"access_token\"",
        "\"refresh_token\"",
        "api_key =",
        "api-key =",
        "client_secret =",
    ] {
        if lower.contains(marker) {
            return Some("credential-bearing field");
        }
    }
    // Common high-confidence token prefixes. This is intentionally only a
    // warning for opaque user content and cannot certify it secret-free.
    if text.contains("ghp_") || text.contains("github_pat_") || text.contains("AKIA") {
        return Some("credential-shaped token");
    }
    None
}

fn resource_digest(
    files: &[CapturedFile],
    directory: Option<&CapturedDirectory>,
    dependency: Option<&DependencyAction>,
) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update((file.logical_path.len() as u64).to_be_bytes());
        hasher.update(file.logical_path.as_bytes());
        hasher.update((file.bytes.len() as u64).to_be_bytes());
        hasher.update(&file.bytes);
        hasher.update(file.mode.to_be_bytes());
    }
    if let Some(directory) = directory {
        hasher.update((directory.logical_path.len() as u64).to_be_bytes());
        hasher.update(directory.logical_path.as_bytes());
        hasher.update(directory.mode.to_be_bytes());
    }
    if let Some(dependency) = dependency {
        if let Ok(bytes) = serde_json::to_vec(dependency) {
            hasher.update(bytes);
        }
    }
    hex_digest(hasher.finalize().as_slice())
}

fn resource_id_component(value: &str) -> String {
    let valid = !value.is_empty()
        && value.len() <= 72
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
        && !value.ends_with(['-', '_', '.'])
        && !value.contains("..");
    if valid {
        value.to_string()
    } else {
        format!("sha256-{}", sha256_bytes(value.as_bytes()))
    }
}

fn relative_cwd_key(relative_cwd: &str) -> String {
    if relative_cwd == "." {
        "_root".to_string()
    } else {
        format!("cwd-{}", sha256_bytes(relative_cwd.as_bytes()))
    }
}

fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_digest(digest.as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut value, "{:02x}", byte);
    }
    value
}

fn modified_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(unix)]
fn safe_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn safe_mode(_meta: &fs::Metadata) -> u32 {
    0o600
}

fn is_executable_path(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0) {
            return true;
        }
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("sh" | "bash" | "zsh" | "fish" | "py" | "rb" | "pl" | "ps1" | "bat" | "cmd" | "exe")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn capture_from_discovery_matches_capture_selected_and_unions_disjoint_sets() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        let codex = temp.path().join("codex");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&codex).unwrap();
        write(
            &codex.join("sessions/2026/07/17/rollout-shared.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-shared\",\"cwd\":{}}}}}\n",
                serde_json::to_string(project.to_str().unwrap()).unwrap()
            )
            .as_bytes(),
        );
        write(&project.join("notes.md"), b"project notes");

        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: true,
            excluded_content_roots: Vec::new(),
        };
        let discovery = discover_full(&request).unwrap();
        let all_ids: BTreeSet<String> = discovery
            .project_inventory()
            .resources
            .iter()
            .map(|resource| resource.resource_id.clone())
            .collect();
        assert!(all_ids.len() >= 2, "fixture must span several resources");

        let direct = capture_selected(&request, &all_ids).unwrap();
        let shared = capture_from_discovery(&request, &discovery, &all_ids).unwrap();
        assert_eq!(
            direct, shared,
            "the wrapper and the seam capture identically"
        );

        let conversation_ids: BTreeSet<String> = all_ids
            .iter()
            .filter(|resource_id| resource_id.starts_with("codex:"))
            .cloned()
            .collect();
        let other_ids: BTreeSet<String> = all_ids.difference(&conversation_ids).cloned().collect();
        assert!(!conversation_ids.is_empty() && !other_ids.is_empty());
        let first = capture_from_discovery(&request, &discovery, &conversation_ids).unwrap();
        let second = capture_from_discovery(&request, &discovery, &other_ids).unwrap();
        let union_files: BTreeSet<String> = first
            .files
            .keys()
            .chain(second.files.keys())
            .cloned()
            .collect();
        assert_eq!(
            union_files,
            shared.files.keys().cloned().collect::<BTreeSet<String>>(),
            "disjoint captures from one Discovery cover the union without overlap"
        );
        assert!(first
            .files
            .keys()
            .all(|logical_path| !second.files.contains_key(logical_path)));
    }

    #[test]
    fn codex_and_claude_membership_uses_cwd_and_relative_mapping() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        let child = project.join("apps/web");
        let other = temp.path().join("other");
        let codex = temp.path().join("codex");
        let claude = temp.path().join("claude");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&claude).unwrap();
        write(
            &codex.join("sessions/2026/07/17/rollout-project.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-a\",\"cwd\":{}}}}}\n",
                serde_json::to_string(child.to_str().unwrap()).unwrap()
            )
            .as_bytes(),
        );
        write(
            &codex.join("sessions/2026/07/17/rollout-other.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-b\",\"cwd\":{}}}}}\n",
                serde_json::to_string(other.to_str().unwrap()).unwrap()
            )
            .as_bytes(),
        );
        write(
            &claude.join("projects/source/session-a.jsonl"),
            format!(
                "{{\"type\":\"system\",\"sessionId\":\"session-a\",\"cwd\":{}}}\n",
                serde_json::to_string(project.to_str().unwrap()).unwrap()
            )
            .as_bytes(),
        );

        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex),
            claude_home: Some(claude),
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let codex = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "codex:session:thread-a")
            .unwrap();
        assert_eq!(codex.relative_cwd.as_deref(), Some("apps/web"));
        assert!(!inventory
            .resources
            .iter()
            .any(|resource| resource.resource_id == "codex:session:thread-b"));
        let claude = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "claude:session:session-a")
            .unwrap();
        assert_eq!(claude.relative_cwd.as_deref(), Some("."));
        assert!(claude.logical_paths[0].contains("/_root/"));
    }

    #[test]
    fn conversation_files_allow_100_mib_and_block_larger_files() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let codex = temp.path().join("codex");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&codex).unwrap();
        let cwd = serde_json::to_string(project.to_str().unwrap()).unwrap();
        let write_sized_session = |name: &str, thread_id: &str, size: u64| {
            let path = codex.join(format!("sessions/2026/07/24/{name}.jsonl"));
            write(
                &path,
                format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{thread_id}\",\"cwd\":{cwd}}}}}\n"
                )
                .as_bytes(),
            );
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(size)
                .unwrap();
        };
        write_sized_session(
            "rollout-at-limit",
            "thread-at-limit",
            domain::MAX_CONVERSATION_FILE_BYTES,
        );
        write_sized_session(
            "rollout-over-limit",
            "thread-over-limit",
            domain::MAX_CONVERSATION_FILE_BYTES + 1,
        );

        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let at_limit = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "codex:session:thread-at-limit")
            .unwrap();
        assert!(at_limit.blocked_reason.is_none());

        let over_limit = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "codex:session:thread-over-limit")
            .unwrap();
        assert!(over_limit
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("exceeds 104857600 bytes")));
    }

    #[test]
    fn codex_subagents_are_excluded_from_sync_discovery_and_capture() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let codex = temp.path().join("codex");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&codex).unwrap();
        let cwd = serde_json::to_string(project.to_str().unwrap()).unwrap();
        write(
            &codex.join("sessions/2026/07/17/rollout-parent.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"parent-thread\",\"cwd\":{cwd}}}}}\n"
            )
            .as_bytes(),
        );
        write(
            &codex.join("sessions/2026/07/17/rollout-child-source.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"parent-thread\",\"id\":\"child-source\",\"parent_thread_id\":\"parent-thread\",\"cwd\":{cwd},\"source\":{{\"subagent\":{{\"other\":\"guardian\"}}}}}}}}\n"
            )
            .as_bytes(),
        );
        write(
            &codex.join("sessions/2026/07/17/rollout-child-thread-source.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"parent-thread\",\"id\":\"child-thread-source\",\"parent_thread_id\":\"parent-thread\",\"cwd\":{cwd},\"thread_source\":\"subagent\"}}}}\n"
            )
            .as_bytes(),
        );

        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let discovery = discover_full(&request).unwrap();
        let inventory = discovery.project_inventory();
        let conversation_ids = inventory
            .resources
            .iter()
            .filter(|resource| resource.kind == CaptureResourceKind::Conversation)
            .map(|resource| resource.resource_id.as_str())
            .collect::<BTreeSet<_>>();

        assert!(conversation_ids.contains("codex:session:parent-thread"));
        assert!(!conversation_ids.contains("codex:session:child-source"));
        assert!(!conversation_ids.contains("codex:session:child-thread-source"));
        assert_eq!(inventory.ignored_count, 2);

        let selected = BTreeSet::from([
            "codex:session:parent-thread".to_string(),
            "codex:session:child-source".to_string(),
            "codex:session:child-thread-source".to_string(),
        ]);
        let captured = capture_from_discovery(&request, &discovery, &selected).unwrap();
        assert!(captured
            .resources
            .contains_key("codex:session:parent-thread"));
        assert_eq!(
            captured.unavailable_resource_ids,
            vec![
                "codex:session:child-source".to_string(),
                "codex:session:child-thread-source".to_string(),
            ]
        );
        assert_eq!(
            captured.excluded_codex_subagent_ids,
            BTreeSet::from([
                "child-source".to_string(),
                "child-thread-source".to_string(),
            ])
        );
    }

    #[test]
    fn duplicate_codex_session_in_active_and_archive_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let codex = temp.path().join("codex");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&codex).unwrap();
        let cwd = serde_json::to_string(project.to_str().unwrap()).unwrap();
        let transcript = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"duplicate-thread\",\"cwd\":{cwd}}}}}\n"
        );
        write(
            &codex.join("sessions/2026/07/20/rollout-duplicate.jsonl"),
            transcript.as_bytes(),
        );
        write(
            &codex.join("archived_sessions/rollout-duplicate.jsonl"),
            transcript.as_bytes(),
        );

        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let matching = inventory
            .resources
            .iter()
            .filter(|resource| resource.resource_id == "codex:session:duplicate-thread")
            .collect::<Vec<_>>();

        assert_eq!(matching.len(), 1);
        assert!(matching[0].logical_paths[0].starts_with("state/codex/sessions/"));
        assert!(inventory
            .warnings
            .iter()
            .any(|warning| warning.contains("ignored duplicate Codex session 'duplicate-thread'")));
    }

    #[test]
    fn index_title_filter_reads_only_project_thread_ids() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("index.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "{{\"thread_id\":\"a\",\"title\":\"A\"}}").unwrap();
        writeln!(file, "{{\"thread_id\":\"b\",\"title\":\"B\"}}").unwrap();
        let bytes = filter_jsonl_by_ids(&path, &BTreeSet::from(["b".to_string()])).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains("\"a\""));
        assert!(text.contains("\"b\""));
    }

    #[test]
    fn project_content_ids_are_stable_and_nested_directories_capture_independently() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("machine-a/project");
        let second = temp.path().join("machine-b/project");
        for root in [&first, &second] {
            write(&root.join("docs/specs/a.md"), b"portable spec");
            fs::create_dir_all(root.join("docs/empty")).unwrap();
        }

        let scan = |root: &Path| {
            let mut request = CaptureRequest::for_project(root);
            request.include_project_content = true;
            let inventory = discover_project(&request).unwrap();
            let by_path = inventory
                .resources
                .iter()
                .filter(|resource| {
                    matches!(
                        resource.kind,
                        CaptureResourceKind::ProjectContentFile
                            | CaptureResourceKind::ProjectContentDirectory
                    )
                })
                .map(|resource| {
                    (
                        resource.logical_paths[0].clone(),
                        (resource.resource_id.clone(), resource.kind.clone()),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            (request, by_path)
        };
        let (first_request, first_entries) = scan(&first);
        let (_, second_entries) = scan(&second);
        assert_eq!(first_entries, second_entries);
        assert!(first_entries.contains_key("project/docs"));
        assert!(first_entries.contains_key("project/docs/specs"));
        assert!(first_entries.contains_key("project/docs/specs/a.md"));
        assert!(first_entries.contains_key("project/docs/empty"));

        let selected = first_entries
            .values()
            .map(|(resource_id, _)| resource_id.clone())
            .collect::<BTreeSet<_>>();
        let captured = capture_selected(&first_request, &selected).unwrap();
        assert_eq!(
            captured.files["project/docs/specs/a.md"].bytes,
            b"portable spec"
        );
        assert!(captured.directories.contains_key("project/docs"));
        assert!(captured.directories.contains_key("project/docs/specs"));
        assert!(captured.directories.contains_key("project/docs/empty"));
    }

    #[test]
    fn project_content_scan_honors_ignores_exclusions_and_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        write(&project.join("keep.txt"), b"keep");
        write(&project.join(".mallardignore"), b"ignored/\n*.tmp\n");
        write(&project.join("ignored/no.txt"), b"ignored");
        write(&project.join("cache.tmp"), b"ignored");
        write(&project.join("node_modules/package/index.js"), b"ignored");
        write(&project.join(".git/config"), b"ignored");
        write(&project.join(".env.production"), b"TOKEN=blocked");
        write(
            &project.join("secret.txt"),
            b"-----BEGIN PRIVATE KEY-----\nblocked",
        );
        write(
            &project.join("child-project/child.txt"),
            b"separate project",
        );

        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        request.excluded_project_roots = vec![project.join("child-project")];
        let inventory = discover_project(&request).unwrap();
        let generic = inventory
            .resources
            .iter()
            .filter(|resource| {
                matches!(
                    resource.kind,
                    CaptureResourceKind::ProjectContentFile
                        | CaptureResourceKind::ProjectContentDirectory
                )
            })
            .collect::<Vec<_>>();
        let paths = generic
            .iter()
            .flat_map(|resource| resource.logical_paths.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        assert!(paths.contains("project/keep.txt"));
        assert!(paths.contains("project/.mallardignore"));
        assert!(!paths.iter().any(|path| path.contains("ignored/no.txt")));
        assert!(!paths.iter().any(|path| path.contains("cache.tmp")));
        assert!(!paths.iter().any(|path| path.contains("node_modules")));
        assert!(!paths.iter().any(|path| path.contains(".git")));
        assert!(!paths
            .iter()
            .any(|path| path.contains("child-project/child.txt")));
        for blocked in [".env.production", "secret.txt"] {
            let resource = generic
                .iter()
                .find(|resource| resource.display_name == blocked)
                .unwrap();
            assert!(resource.blocked_reason.is_some());
            assert!(!resource.selected_by_default);
        }
        assert!(inventory.ignored_count >= 4);
        assert!(inventory.blocked_count >= 2);
    }

    #[cfg(unix)]
    #[test]
    fn project_content_links_are_blocked_and_never_followed() {
        use std::fs::hard_link;
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let outside = temp.path().join("outside.txt");
        write(&outside, b"outside");
        write(&project.join("original.txt"), b"linked");
        hard_link(project.join("original.txt"), project.join("hard-link.txt")).unwrap();
        symlink(&outside, project.join("outside-link.txt")).unwrap();

        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        for path in ["original.txt", "hard-link.txt", "outside-link.txt"] {
            let resource = inventory
                .resources
                .iter()
                .find(|resource| resource.display_name == path)
                .unwrap();
            assert!(
                resource.blocked_reason.is_some(),
                "{path} was unexpectedly selectable"
            );
        }
        assert!(!inventory.resources.iter().any(|resource| {
            resource
                .logical_paths
                .iter()
                .any(|path| path.contains("outside.txt"))
        }));
    }

    #[test]
    fn project_content_size_and_depth_boundaries_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let oversized = fs::File::create(project.join("oversized.bin")).unwrap();
        oversized.set_len(MAX_FILE_BYTES + 1).unwrap();
        let mut deep = project.clone();
        for index in 0..=MAX_TREE_DEPTH {
            deep.push(format!("d{index}"));
        }
        write(&deep.join("too-deep.txt"), b"deep");

        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let oversized = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "oversized.bin")
            .unwrap();
        assert!(oversized
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("exceeds")));
        assert!(inventory.blocked_count >= 2);
        assert!(inventory
            .warnings
            .iter()
            .any(|warning| warning.contains("depth limit")));
        assert!(!inventory
            .resources
            .iter()
            .any(|resource| resource.display_name.ends_with("too-deep.txt")));
    }

    #[test]
    fn project_content_stress_scan_enforces_the_twenty_thousand_resource_cap() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        for index in 0..=MAX_DISCOVERED_FILES {
            fs::write(project.join(format!("entry-{index:05}.txt")), b"").unwrap();
        }
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let error = discover_project(&request).unwrap_err();
        assert!(error.contains("exceeds 20000 resources"), "{error}");
    }
}
