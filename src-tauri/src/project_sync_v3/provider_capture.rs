//! Project-scoped discovery and capture for schema-4 bundles.
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Claude,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CaptureResourceKind {
    ProjectFile,
    ProjectContentFile,
    ProjectContentDirectory,
    ProjectSettings,
    Conversation,
    Memory,
    Agent,
    Command,
    Rule,
    Skill,
    StandaloneSkill,
    Plugin,
    Hook,
    McpServer,
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
    Project,
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
    /// Local typed codec output used by schema-5 planning. It is deliberately
    /// separate from descriptor metadata so provider IDs and parsed chains do
    /// not leak into a schema-4 manifest or presentation fields.
    pub conversations: BTreeMap<String, CapturedConversation>,
    pub dependency_actions: Vec<DependencyAction>,
    pub unavailable_resource_ids: Vec<String>,
    pub warnings: Vec<String>,
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

impl CapturedConversation {
    pub fn unknown_reason(&self) -> Option<CodecUnknownReason> {
        match &self.lineage {
            CodecLineage::Unknown { reason } => Some(*reason),
            CodecLineage::Complete { .. } => None,
        }
    }
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

#[derive(Clone, Debug)]
struct Discovery {
    resources: BTreeMap<String, DiscoveredResource>,
    directories: BTreeMap<String, SourceDirectory>,
    warnings: Vec<String>,
    ignored_count: usize,
    blocked_count: usize,
}

/// Discover resources without retaining file bytes. Absolute source paths are
/// kept only inside this module and never become resource identity.
pub fn discover_project(request: &CaptureRequest) -> Result<ProjectInventory, String> {
    let discovery = discover(request)?;
    Ok(ProjectInventory {
        resources: discovery
            .resources
            .into_values()
            .map(|candidate| candidate.descriptor)
            .collect(),
        warnings: discovery.warnings,
        ignored_count: discovery.ignored_count,
        blocked_count: discovery.blocked_count,
    })
}

/// Capture exactly the live resource IDs selected by the persistent recipe.
/// Missing or blocked resources are reported as unavailable; they are not
/// turned into deselections or tombstones.
pub fn capture_selected(
    request: &CaptureRequest,
    selected_resource_ids: &BTreeSet<String>,
) -> Result<CapturedResources, String> {
    let discovery = discover(request)?;
    let canonical_project = canonical_existing_dir(&request.project_root, "project root")?;
    let mut result = CapturedResources {
        warnings: discovery.warnings,
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
            )?;
            let meta = fs::metadata(&source.physical_path)
                .map_err(|e| format!("inspect '{}': {}", source.physical_path.display(), e))?;
            let content_len = source
                .derived_bytes
                .as_ref()
                .map(|bytes| bytes.len() as u64)
                .unwrap_or(meta.len());
            if content_len > MAX_FILE_BYTES {
                return Err(format!(
                    "resource '{}' file '{}' exceeds {} bytes",
                    resource_id,
                    source.physical_path.display(),
                    MAX_FILE_BYTES
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
    recipe.validate()?;
    let selected = recipe
        .entries
        .keys()
        .map(|id| id.as_str().to_string())
        .collect();
    capture_selected(request, &selected)
}

/// Convert captured descriptors into the shared schema-3 domain model. This
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
            CaptureResourceKind::ProjectFile
            | CaptureResourceKind::ProjectContentFile
            | CaptureResourceKind::ProjectContentDirectory
            | CaptureResourceKind::ProjectSettings
            | CaptureResourceKind::Agent
            | CaptureResourceKind::Command
            | CaptureResourceKind::Rule
            | CaptureResourceKind::Hook
            | CaptureResourceKind::McpServer => {
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
        CaptureResourceKind::ProjectFile => domain::ResourceKind::ProjectFile,
        CaptureResourceKind::ProjectContentFile => domain::ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => {
            domain::ResourceKind::ProjectContentDirectory
        }
        CaptureResourceKind::ProjectSettings => domain::ResourceKind::Setting,
        CaptureResourceKind::Conversation => match provider {
            Some(Provider::Claude) => domain::ResourceKind::ClaudeConversation,
            _ => domain::ResourceKind::CodexConversation,
        },
        CaptureResourceKind::Memory => domain::ResourceKind::ProjectMemory,
        CaptureResourceKind::Agent => domain::ResourceKind::Agent,
        CaptureResourceKind::Command => domain::ResourceKind::Command,
        CaptureResourceKind::Rule => domain::ResourceKind::Rule,
        CaptureResourceKind::Skill => domain::ResourceKind::ProjectSkill,
        CaptureResourceKind::StandaloneSkill => domain::ResourceKind::StandaloneSkill,
        CaptureResourceKind::Plugin => domain::ResourceKind::Plugin,
        CaptureResourceKind::Hook => domain::ResourceKind::Hook,
        CaptureResourceKind::McpServer => domain::ResourceKind::McpServer,
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
        DependencyScope::Project => "project",
        DependencyScope::ProviderHome => "provider_home",
    }
}

fn discover(request: &CaptureRequest) -> Result<Discovery, String> {
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
    };
    discover_project_files(&canonical_project, &mut discovery)?;
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

fn discover_project_files(root: &Path, discovery: &mut Discovery) -> Result<(), String> {
    // AGENTS.md files belong to the repository and travel with it through Git.
    // Do not inventory them here, including nested files or symlink aliases.
    for (relative, kind, provider, policy, default) in [
        (
            "CLAUDE.md",
            CaptureResourceKind::ProjectFile,
            Some(Provider::Claude),
            CaptureApplyPolicy::Merge,
            true,
        ),
        (
            ".claude/CLAUDE.md",
            CaptureResourceKind::ProjectFile,
            Some(Provider::Claude),
            CaptureApplyPolicy::Merge,
            true,
        ),
        (
            ".codex/config.toml",
            CaptureResourceKind::ProjectSettings,
            Some(Provider::Codex),
            CaptureApplyPolicy::Review,
            true,
        ),
        (
            ".codex/hooks.json",
            CaptureResourceKind::Hook,
            Some(Provider::Codex),
            CaptureApplyPolicy::Review,
            false,
        ),
        (
            ".claude/settings.json",
            CaptureResourceKind::ProjectSettings,
            Some(Provider::Claude),
            CaptureApplyPolicy::Review,
            true,
        ),
        (
            ".mcp.json",
            CaptureResourceKind::McpServer,
            Some(Provider::Claude),
            CaptureApplyPolicy::Review,
            false,
        ),
        (
            ".agents/plugins/marketplace.json",
            CaptureResourceKind::ProjectSettings,
            Some(Provider::Codex),
            CaptureApplyPolicy::Review,
            false,
        ),
    ] {
        let path = root.join(relative);
        if path_exists_no_follow(&path)? {
            add_single_project_file(discovery, root, relative, kind, provider, policy, default)?;
        }
    }

    for (directory, kind, provider) in [
        (
            ".agents/skills",
            CaptureResourceKind::Skill,
            Provider::Codex,
        ),
        (
            ".claude/skills",
            CaptureResourceKind::Skill,
            Provider::Claude,
        ),
        (
            ".claude/agents",
            CaptureResourceKind::Agent,
            Provider::Claude,
        ),
        (
            ".claude/commands",
            CaptureResourceKind::Command,
            Provider::Claude,
        ),
        (".claude/rules", CaptureResourceKind::Rule, Provider::Claude),
        (".codex/rules", CaptureResourceKind::Rule, Provider::Codex),
    ] {
        discover_grouped_directory(root, directory, kind, provider, discovery)?;
    }

    discover_plugin_intents(root, discovery)?;
    Ok(())
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

fn add_single_project_file(
    discovery: &mut Discovery,
    root: &Path,
    relative: &str,
    kind: CaptureResourceKind,
    provider: Option<Provider>,
    apply_policy: CaptureApplyPolicy,
    selected_by_default: bool,
) -> Result<(), String> {
    validate_logical_project_relative(relative)?;
    let path = root.join(relative);
    let mut blocked_reason = inspect_candidate_files(root, &[path.clone()]).err();
    let mut derived_bytes = None;
    if blocked_reason.is_none()
        && matches!(
            kind,
            CaptureResourceKind::ProjectSettings
                | CaptureResourceKind::Hook
                | CaptureResourceKind::McpServer
        )
    {
        match portable_project_projection(relative, &path) {
            Ok((bytes, warnings)) => {
                derived_bytes = Some(bytes);
                discovery.warnings.extend(warnings);
            }
            Err(error) => blocked_reason = Some(error),
        }
    }
    let resource_id = format!("project:file:{}", resource_id_component(relative));
    let source = SourceFile {
        physical_path: path,
        logical_path: format!("project/{}", relative),
        opaque_content: matches!(kind, CaptureResourceKind::ProjectFile),
        derived_bytes,
    };
    insert_resource(
        discovery,
        DiscoveredResource {
            descriptor: ResourceCandidate {
                metadata: BTreeMap::new(),
                resource_id,
                kind,
                provider,
                display_name: relative.to_string(),
                relative_cwd: None,
                apply_policy,
                selected_by_default,
                blocked_reason,
                logical_paths: vec![source.logical_path.clone()],
                dependency: None,
            },
            files: vec![source],
        },
    )
}

/// Project settings cross a stricter boundary than instructions or session
/// text.  Only a small provider-supported projection enters a bundle; known
/// auth, trust, approval, permission, and literal environment values are
/// removed structurally before the generic secret scan runs.
fn portable_project_projection(
    relative: &str,
    path: &Path,
) -> Result<(Vec<u8>, Vec<String>), String> {
    let bytes = read_bounded(path)?;
    match relative {
        ".codex/config.toml" => project_codex_config_projection(&bytes),
        ".claude/settings.json" => project_claude_settings_projection(&bytes),
        ".mcp.json" => project_mcp_projection(&bytes),
        ".codex/hooks.json" | ".agents/plugins/marketplace.json" => {
            reviewed_json_projection(relative, &bytes)
        }
        _ => Err(format!(
            "'{}' has no schema-3 portable settings codec",
            relative
        )),
    }
}

fn project_codex_config_projection(bytes: &[u8]) -> Result<(Vec<u8>, Vec<String>), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("Codex project config is not UTF-8: {}", error))?;
    let source: toml::Value =
        toml::from_str(text).map_err(|error| format!("parse Codex project config: {}", error))?;
    let table = source
        .as_table()
        .ok_or_else(|| "Codex project config must be a TOML table".to_string())?;
    // Deliberately excludes approval/sandbox/trust, MCP, hooks, marketplaces,
    // provider endpoints, environment values, and absolute skill paths.
    const ALLOWED: &[&str] = &[
        "model",
        "model_reasoning_effort",
        "model_reasoning_summary",
        "personality",
        "web_search",
        "hide_agent_reasoning",
        "show_raw_agent_reasoning",
        "project_doc_max_bytes",
        "project_doc_fallback_filenames",
    ];
    let mut projected = toml::map::Map::new();
    for key in ALLOWED {
        if let Some(value) = table.get(*key) {
            validate_portable_toml_value(value, key)?;
            projected.insert((*key).to_string(), value.clone());
        }
    }
    let removed = table.len().saturating_sub(projected.len());
    let mut warnings = Vec::new();
    if removed > 0 {
        warnings.push(format!(
            ".codex/config.toml: omitted {} machine, trust, executable, or unsupported setting entries",
            removed
        ));
    }
    let mut output = toml::to_string_pretty(&toml::Value::Table(projected))
        .map_err(|error| format!("serialize portable Codex config: {}", error))?
        .into_bytes();
    if !output.ends_with(b"\n") {
        output.push(b'\n');
    }
    Ok((output, warnings))
}

fn project_claude_settings_projection(bytes: &[u8]) -> Result<(Vec<u8>, Vec<String>), String> {
    let source: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse Claude project settings: {}", error))?;
    let object = source
        .as_object()
        .ok_or_else(|| "Claude project settings must be a JSON object".to_string())?;
    // Plugin IDs are captured as typed dependency intent.  Hooks, permissions,
    // env, auth, and trust never hitch a ride in this settings projection.
    const ALLOWED: &[&str] = &[
        "model",
        "language",
        "outputStyle",
        "includeCoAuthoredBy",
        "cleanupPeriodDays",
        "autoUpdatesChannel",
        "spinnerTipsEnabled",
    ];
    let mut projected = serde_json::Map::new();
    for key in ALLOWED {
        if let Some(value) = object.get(*key) {
            validate_portable_json_value(value, key)?;
            projected.insert((*key).to_string(), value.clone());
        }
    }
    let removed = object.len().saturating_sub(projected.len());
    let warnings = (removed > 0)
        .then(|| {
            format!(
                ".claude/settings.json: omitted {} permission, hook, plugin-enable, environment, or unsupported setting entries",
                removed
            )
        })
        .into_iter()
        .collect();
    Ok((
        canonical_json(serde_json::Value::Object(projected))?,
        warnings,
    ))
}

fn project_mcp_projection(bytes: &[u8]) -> Result<(Vec<u8>, Vec<String>), String> {
    let source: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("parse .mcp.json: {}", error))?;
    let servers = source
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ".mcp.json must contain an mcpServers object".to_string())?;
    let mut projected_servers = serde_json::Map::new();
    let mut warnings = Vec::new();
    for (name, server) in servers {
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(".mcp.json contains an invalid server name".to_string());
        }
        let server = server
            .as_object()
            .ok_or_else(|| format!("MCP server '{}' must be an object", name))?;
        let mut projected = serde_json::Map::new();
        for key in ["type", "command", "args", "url", "cwd"] {
            if let Some(value) = server.get(key) {
                validate_portable_json_value(value, &format!("mcpServers.{}.{}", name, key))?;
                projected.insert(key.to_string(), value.clone());
            }
        }
        for key in ["env", "headers"] {
            if let Some(values) = server.get(key) {
                let values = values
                    .as_object()
                    .ok_or_else(|| format!("MCP server '{}.{}' must be an object", name, key))?;
                let mut placeholders = serde_json::Map::new();
                for (field, value) in values {
                    let env_name = portable_environment_name(field);
                    let placeholder = value
                        .as_str()
                        .filter(|value| exact_environment_reference(value))
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("${{{}}}", env_name));
                    if value.as_str() != Some(placeholder.as_str()) {
                        warnings.push(format!(
                            ".mcp.json: replaced literal {} value for {}.{} with an environment reference",
                            key, name, field
                        ));
                    }
                    placeholders.insert(field.clone(), serde_json::Value::String(placeholder));
                }
                projected.insert(key.to_string(), serde_json::Value::Object(placeholders));
            }
        }
        let removed = server.len().saturating_sub(projected.len());
        if removed > 0 {
            warnings.push(format!(
                ".mcp.json: omitted {} auth or unsupported fields from server '{}'",
                removed, name
            ));
        }
        projected_servers.insert(name.clone(), serde_json::Value::Object(projected));
    }
    let projected = serde_json::json!({ "mcpServers": projected_servers });
    Ok((canonical_json(projected)?, warnings))
}

fn reviewed_json_projection(
    relative: &str,
    bytes: &[u8],
) -> Result<(Vec<u8>, Vec<String>), String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("parse {}: {}", relative, error))?;
    let mut warnings = Vec::new();
    let projected = sanitize_reviewed_json(value, relative, &mut warnings)?;
    Ok((canonical_json(projected)?, warnings))
}

fn sanitize_reviewed_json(
    value: serde_json::Value,
    context: &str,
    warnings: &mut Vec<String>,
) -> Result<serde_json::Value, String> {
    match value {
        serde_json::Value::Object(object) => {
            let mut projected = serde_json::Map::new();
            for (key, value) in object {
                if forbidden_portable_field(&key) {
                    warnings.push(format!(
                        "{}: omitted credential/trust field '{}'",
                        context, key
                    ));
                    continue;
                }
                if key.eq_ignore_ascii_case("env") || key.eq_ignore_ascii_case("headers") {
                    let values = value
                        .as_object()
                        .ok_or_else(|| format!("{}.{} must be an object", context, key))?;
                    let mut placeholders = serde_json::Map::new();
                    for (field, value) in values {
                        let env_name = portable_environment_name(field);
                        let placeholder = value
                            .as_str()
                            .filter(|value| exact_environment_reference(value))
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("${{{}}}", env_name));
                        placeholders.insert(field.clone(), serde_json::Value::String(placeholder));
                    }
                    warnings.push(format!(
                        "{}: converted {} values to environment references",
                        context, key
                    ));
                    projected.insert(key, serde_json::Value::Object(placeholders));
                    continue;
                }
                projected.insert(
                    key.clone(),
                    sanitize_reviewed_json(value, &format!("{}.{}", context, key), warnings)?,
                );
            }
            Ok(serde_json::Value::Object(projected))
        }
        serde_json::Value::Array(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                sanitize_reviewed_json(value, &format!("{}[{}]", context, index), warnings)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::String(value) => {
            validate_portable_string(&value, context)?;
            Ok(serde_json::Value::String(value))
        }
        other => Ok(other),
    }
}

fn validate_portable_toml_value(value: &toml::Value, context: &str) -> Result<(), String> {
    match value {
        toml::Value::String(value) => validate_portable_string(value, context),
        toml::Value::Array(values) => values.iter().enumerate().try_for_each(|(index, value)| {
            validate_portable_toml_value(value, &format!("{}[{}]", context, index))
        }),
        toml::Value::Table(table) => table.iter().try_for_each(|(key, value)| {
            if forbidden_portable_field(key) {
                Err(format!("{} contains forbidden field '{}'", context, key))
            } else {
                validate_portable_toml_value(value, &format!("{}.{}", context, key))
            }
        }),
        _ => Ok(()),
    }
}

fn validate_portable_json_value(value: &serde_json::Value, context: &str) -> Result<(), String> {
    match value {
        serde_json::Value::String(value) => validate_portable_string(value, context),
        serde_json::Value::Array(values) => {
            values.iter().enumerate().try_for_each(|(index, value)| {
                validate_portable_json_value(value, &format!("{}[{}]", context, index))
            })
        }
        serde_json::Value::Object(object) => object.iter().try_for_each(|(key, value)| {
            if forbidden_portable_field(key) {
                Err(format!("{} contains forbidden field '{}'", context, key))
            } else {
                validate_portable_json_value(value, &format!("{}.{}", context, key))
            }
        }),
        _ => Ok(()),
    }
}

fn validate_portable_string(value: &str, context: &str) -> Result<(), String> {
    if value.len() > 16 * 1024 || value.chars().any(char::is_control) {
        return Err(format!("{} contains invalid text", context));
    }
    if looks_like_local_path(value) || contains_url_credentials(value) {
        return Err(format!(
            "{} contains a machine-local or credentialed path",
            context
        ));
    }
    if detect_secret_material(value.as_bytes()).is_some() {
        return Err(format!("{} contains credential-shaped material", context));
    }
    Ok(())
}

fn forbidden_portable_field(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "token",
        "secret",
        "password",
        "credential",
        "authorization",
        "authentication",
        "oauth",
        "cookie",
        "trust",
        "approval",
        "permission",
        "apikey",
        "accesskey",
        "privatekey",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn exact_environment_reference(value: &str) -> bool {
    value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .is_some_and(|name| {
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

fn portable_environment_name(value: &str) -> String {
    let mut name = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .take(128)
        .collect::<String>();
    if name.is_empty() || name.as_bytes()[0].is_ascii_digit() {
        name.insert_str(0, "AGENT_SYNC_");
    }
    name
}

fn canonical_json(value: serde_json::Value) -> Result<Vec<u8>, String> {
    let mut output = serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("serialize portable JSON: {}", error))?;
    output.push(b'\n');
    Ok(output)
}

fn discover_grouped_directory(
    project_root: &Path,
    relative_root: &str,
    kind: CaptureResourceKind,
    provider: Provider,
    discovery: &mut Discovery,
) -> Result<(), String> {
    let root = project_root.join(relative_root);
    let meta = match fs::symlink_metadata(&root) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("inspect '{}': {}", root.display(), e)),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        discovery.warnings.push(format!(
            "{} is not a regular project directory and was not discovered",
            relative_root
        ));
        return Ok(());
    }
    let mut children = fs::read_dir(&root)
        .map_err(|e| format!("read '{}': {}", root.display(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read '{}': {}", root.display(), e))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let name = child
            .file_name()
            .to_str()
            .ok_or_else(|| format!("non-UTF-8 resource name in '{}'", root.display()))?
            .to_string();
        if denied_file_name(&name) || name == ".git" {
            discovery.warnings.push(format!(
                "{}/{} is credential/cache/VCS material and was excluded",
                relative_root, name
            ));
            continue;
        }
        let files = collect_regular_tree_files(project_root, child.path())?;
        let logical_paths = files
            .iter()
            .map(|path| {
                normalized_relative(project_root, path).map(|rel| format!("project/{}", rel))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let relative = normalized_relative(project_root, &child.path())?;
        let resource_id = format!(
            "{}:{}:{}",
            provider_name(provider),
            resource_kind_name(&kind),
            resource_id_component(&relative)
        );
        let executable =
            kind == CaptureResourceKind::Skill && files.iter().any(|path| is_executable_path(path));
        let dependency = executable.then(|| DependencyAction {
            action_id: format!("dependency:{}", resource_id),
            resource_id: resource_id.clone(),
            provider,
            kind: DependencyKind::StandaloneSkill,
            scope: DependencyScope::Project,
            display_name: name.clone(),
            program: None,
            argv: Vec::new(),
            requires_review: true,
            payload_logical_prefix: Some(format!("project/{}/{}", relative_root, name)),
        });
        let blocked_reason = inspect_candidate_files(project_root, &files).err();
        insert_resource(
            discovery,
            DiscoveredResource {
                descriptor: ResourceCandidate {
                    metadata: BTreeMap::new(),
                    resource_id,
                    kind: kind.clone(),
                    provider: Some(provider),
                    display_name: name,
                    relative_cwd: None,
                    apply_policy: if executable {
                        CaptureApplyPolicy::Dependency
                    } else {
                        CaptureApplyPolicy::SafeFile
                    },
                    selected_by_default: true,
                    blocked_reason,
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
        )?;
    }
    Ok(())
}

fn discover_plugin_intents(root: &Path, discovery: &mut Discovery) -> Result<(), String> {
    let claude_settings = root.join(".claude/settings.json");
    if claude_settings.is_file() {
        match read_bounded_json(&claude_settings) {
            Ok(value) => {
                for plugin_id in extract_claude_plugin_ids(&value) {
                    let resource_id =
                        format!("claude:plugin:{}", resource_id_component(&plugin_id));
                    let action = DependencyAction {
                        action_id: format!("dependency:{}", resource_id),
                        resource_id: resource_id.clone(),
                        provider: Provider::Claude,
                        kind: DependencyKind::ClaudePlugin,
                        scope: DependencyScope::Project,
                        display_name: plugin_id.clone(),
                        program: Some("claude".to_string()),
                        argv: vec![
                            "plugin".to_string(),
                            "install".to_string(),
                            plugin_id.clone(),
                            "--scope".to_string(),
                            "project".to_string(),
                        ],
                        requires_review: true,
                        payload_logical_prefix: None,
                    };
                    insert_resource(
                        discovery,
                        dependency_resource(
                            resource_id,
                            plugin_id,
                            Provider::Claude,
                            CaptureResourceKind::Plugin,
                            action,
                        ),
                    )?;
                }
            }
            Err(error) => discovery
                .warnings
                .push(format!("could not inspect Claude plugin intent: {}", error)),
        }
    }

    let codex_config = root.join(".codex/config.toml");
    if codex_config.is_file() {
        match read_bounded_toml(&codex_config) {
            Ok(value) => {
                for plugin_id in extract_codex_plugin_ids(&value) {
                    let resource_id = format!("codex:plugin:{}", resource_id_component(&plugin_id));
                    let action = DependencyAction {
                        action_id: format!("dependency:{}", resource_id),
                        resource_id: resource_id.clone(),
                        provider: Provider::Codex,
                        kind: DependencyKind::CodexPlugin,
                        scope: DependencyScope::ProviderHome,
                        display_name: plugin_id.clone(),
                        program: Some("codex".to_string()),
                        argv: vec!["plugin".to_string(), "add".to_string(), plugin_id.clone()],
                        requires_review: true,
                        payload_logical_prefix: None,
                    };
                    insert_resource(
                        discovery,
                        dependency_resource(
                            resource_id,
                            plugin_id,
                            Provider::Codex,
                            CaptureResourceKind::Plugin,
                            action,
                        ),
                    )?;
                }
            }
            Err(error) => discovery
                .warnings
                .push(format!("could not inspect Codex plugin intent: {}", error)),
        }
    }
    Ok(())
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
            selected_thread_ids.insert(meta.session_id.clone());
            let resource_id = format!("codex:session:{}", session_id);
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
                        blocked_reason: inspect_candidate_files(
                            &canonical_home,
                            &[entry.path().to_path_buf()],
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
        // The derived index is written to a temporary, capture-owned file so
        // only rows belonging to selected project sessions can enter a bundle.
        let filtered = filter_jsonl_by_ids(&index, &selected_thread_ids)?;
        if !filtered.is_empty() {
            let resource_id = "codex:session-index".to_string();
            insert_resource(
                discovery,
                DiscoveredResource {
                    descriptor: ResourceCandidate {
                        metadata: BTreeMap::new(),
                        resource_id: resource_id.clone(),
                        kind: CaptureResourceKind::Conversation,
                        provider: Some(Provider::Codex),
                        display_name: "Codex project session index".to_string(),
                        relative_cwd: None,
                        apply_policy: CaptureApplyPolicy::Merge,
                        selected_by_default: true,
                        blocked_reason: None,
                        logical_paths: vec!["state/codex/session_index.jsonl".to_string()],
                        dependency: None,
                    },
                    files: vec![SourceFile {
                        physical_path: index.clone(),
                        logical_path: "state/codex/session_index.jsonl".to_string(),
                        opaque_content: true,
                        derived_bytes: Some(filtered),
                    }],
                },
            )?;
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
                    blocked_reason: inspect_candidate_files(
                        &canonical_home,
                        &[entry.path().to_path_buf()],
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
                        blocked_reason: inspect_candidate_files(
                            &canonical_home,
                            &[entry.path().to_path_buf()],
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

/// Register one inventoried global plugin as portable install intent. If the
/// project configuration already declared the same plugin, the two
/// observations coalesce into the existing resource instead of creating a
/// second install action.
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
    if let Some(existing) = discovery.resources.get_mut(&resource_id) {
        // One plugin with two discovery origins, not two install actions.
        existing
            .descriptor
            .metadata
            .insert("plugin_origin".to_string(), "project+global".to_string());
        for (key, value) in metadata {
            existing.descriptor.metadata.entry(key).or_insert(value);
        }
        return Ok(());
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
        }
        if authoritative {
            if let (Some(id), Some(cwd)) = (id, cwd) {
                return validated_session_metadata(id, cwd);
            }
        }
    }
    let id =
        fallback_id.ok_or_else(|| "session ID was not found in bounded metadata".to_string())?;
    let cwd = fallback_cwd.ok_or_else(|| "cwd was not found in bounded metadata".to_string())?;
    validated_session_metadata(id, cwd)
}

fn validated_session_metadata(id: String, cwd: String) -> Result<SessionMetadata, String> {
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

fn extract_claude_plugin_ids(value: &serde_json::Value) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    let Some(enabled) = value.get("enabledPlugins") else {
        return result;
    };
    match enabled {
        serde_json::Value::Object(map) => {
            for (id, enabled) in map {
                if enabled.as_bool().unwrap_or(false) && portable_dependency_id(id) {
                    result.insert(id.clone());
                }
            }
        }
        serde_json::Value::Array(values) => {
            for id in values.iter().filter_map(serde_json::Value::as_str) {
                if portable_dependency_id(id) {
                    result.insert(id.to_string());
                }
            }
        }
        _ => {}
    }
    result
}

fn extract_codex_plugin_ids(value: &toml::Value) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    let Some(plugins) = value.get("plugins").and_then(toml::Value::as_table) else {
        return result;
    };
    for (id, definition) in plugins {
        let enabled = definition
            .get("enabled")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true);
        if enabled && portable_dependency_id(id) {
            result.insert(id.clone());
        }
    }
    result
}

fn portable_dependency_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.chars().any(char::is_control)
        && !value.contains("..")
        && !looks_like_local_path(value)
        && !contains_url_credentials(value)
}

fn looks_like_local_path(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with('~')
        || (value.len() >= 3
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'))
}

fn contains_url_credentials(value: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, tail)| tail.split('/').next())
        .is_some_and(|authority| authority.contains('@'))
}

fn read_bounded_json(path: &Path) -> Result<serde_json::Value, String> {
    let bytes = read_bounded(path)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse '{}': {}", path.display(), e))
}

fn read_bounded_toml(path: &Path) -> Result<toml::Value, String> {
    let bytes = read_bounded(path)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| format!("'{}' is not UTF-8: {}", path.display(), e))?;
    toml::from_str(text).map_err(|e| format!("parse '{}': {}", path.display(), e))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let meta =
        fs::symlink_metadata(path).map_err(|e| format!("inspect '{}': {}", path.display(), e))?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "'{}' is not a bounded regular file",
            path.display()
        ));
    }
    fs::read(path).map_err(|e| format!("read '{}': {}", path.display(), e))
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
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!(
                "'{}' exceeds {} bytes",
                file.display(),
                MAX_FILE_BYTES
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
) -> Result<(), String> {
    let canonical_path = fs::canonicalize(path)
        .map_err(|e| format!("resolve source '{}': {}", path.display(), e))?;
    let mut approved_roots = vec![canonical_project.to_path_buf()];
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
    inspect_candidate_files(&approved_root, &[path.to_path_buf()])
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

fn path_exists_no_follow(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "allowlisted path '{}' is a symlink and cannot be captured",
            path.display()
        )),
        Ok(meta) if meta.is_file() => Ok(true),
        Ok(_) => Err(format!(
            "allowlisted path '{}' is not a file",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("inspect '{}': {}", path.display(), e)),
    }
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

fn resource_kind_name(kind: &CaptureResourceKind) -> &'static str {
    match kind {
        CaptureResourceKind::ProjectFile => "project-file",
        CaptureResourceKind::ProjectContentFile => "project-content-file",
        CaptureResourceKind::ProjectContentDirectory => "project-content-directory",
        CaptureResourceKind::ProjectSettings => "settings",
        CaptureResourceKind::Conversation => "session",
        CaptureResourceKind::Memory => "memory",
        CaptureResourceKind::Agent => "agent",
        CaptureResourceKind::Command => "command",
        CaptureResourceKind::Rule => "rule",
        CaptureResourceKind::Skill => "skill",
        CaptureResourceKind::StandaloneSkill => "custom-skill",
        CaptureResourceKind::Plugin => "plugin",
        CaptureResourceKind::Hook => "hook",
        CaptureResourceKind::McpServer => "mcp",
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
    fn git_owned_agents_files_are_not_discovered() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("AGENTS.md"), b"project guidance");
        write(&project.join("apps/web/AGENTS.md"), b"nested guidance");
        write(&project.join("CLAUDE.md"), b"claude guidance");
        write(&project.join("src/main.rs"), b"ordinary source");
        write(
            &project.join(".claude/skills/review/SKILL.md"),
            b"review instructions",
        );

        let request = CaptureRequest::for_project(&project);
        let inventory = discover_project(&request).unwrap();
        assert!(!inventory
            .resources
            .iter()
            .any(|resource| resource.display_name.ends_with("AGENTS.md")));
        assert!(!inventory
            .resources
            .iter()
            .flat_map(|resource| &resource.logical_paths)
            .any(|path| path.contains("src/main.rs")));

        let claude_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "CLAUDE.md")
            .unwrap()
            .resource_id
            .clone();
        let selected = BTreeSet::from([claude_id]);
        let captured = capture_selected(&request, &selected).unwrap();
        assert_eq!(captured.resources.len(), 1);
        assert_eq!(captured.files.len(), 1);
        assert_eq!(
            captured.files["project/CLAUDE.md"].bytes,
            b"claude guidance"
        );
    }

    #[cfg(unix)]
    #[test]
    fn git_owned_agents_symlink_does_not_block_discovery() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("CLAUDE.md"), b"shared guidance");
        symlink("CLAUDE.md", project.join("AGENTS.md")).unwrap();

        let inventory = discover_project(&CaptureRequest::for_project(&project)).unwrap();
        assert!(!inventory
            .resources
            .iter()
            .any(|resource| resource.display_name == "AGENTS.md"));
        assert!(inventory
            .resources
            .iter()
            .any(|resource| resource.display_name == "CLAUDE.md"));
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
    fn codex_subagent_uses_its_own_id_instead_of_parent_session_id() {
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
            &codex.join("sessions/2026/07/17/rollout-child.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"parent-thread\",\"id\":\"child-thread\",\"parent_thread_id\":\"parent-thread\",\"cwd\":{cwd},\"source\":{{\"subagent\":{{\"other\":\"guardian\"}}}}}}}}\n"
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
        let inventory = discover_project(&request).unwrap();
        let conversation_ids = inventory
            .resources
            .iter()
            .filter(|resource| resource.kind == CaptureResourceKind::Conversation)
            .map(|resource| resource.resource_id.as_str())
            .collect::<BTreeSet<_>>();

        assert!(conversation_ids.contains("codex:session:parent-thread"));
        assert!(conversation_ids.contains("codex:session:child-thread"));
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
    fn plugin_intent_is_a_typed_dependency_and_cache_is_never_copied() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(
            &project.join(".claude/settings.json"),
            br#"{"enabledPlugins":{"reviewer@team":true}}"#,
        );
        write(
            &project.join(".claude/plugins/cache/reviewer/payload.js"),
            b"installed payload",
        );
        let request = CaptureRequest::for_project(&project);
        let inventory = discover_project(&request).unwrap();
        let plugin = inventory
            .resources
            .iter()
            .find(|resource| resource.kind == CaptureResourceKind::Plugin)
            .unwrap();
        let action = plugin.dependency.as_ref().unwrap();
        assert_eq!(action.program.as_deref(), Some("claude"));
        assert_eq!(action.scope, DependencyScope::Project);
        assert!(!inventory
            .resources
            .iter()
            .flat_map(|resource| &resource.logical_paths)
            .any(|path| path.contains("plugins/cache")));
    }

    #[test]
    fn settings_capture_is_a_structural_portable_projection() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(
            &project.join(".claude/settings.json"),
            br#"{
              "model":"sonnet",
              "language":"en",
              "permissions":{"allow":["Bash(*)"]},
              "env":{"API_TOKEN":"literal-secret"},
              "enabledPlugins":{"reviewer@team":true},
              "apiKey":"must-not-sync"
            }"#,
        );
        let request = CaptureRequest::for_project(&project);
        let inventory = discover_project(&request).unwrap();
        let settings = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == ".claude/settings.json")
            .unwrap();
        assert!(settings.blocked_reason.is_none());
        assert!(inventory
            .resources
            .iter()
            .any(|resource| resource.kind == CaptureResourceKind::Plugin));

        let captured =
            capture_selected(&request, &BTreeSet::from([settings.resource_id.clone()])).unwrap();
        let projected = String::from_utf8(
            captured.files["project/.claude/settings.json"]
                .bytes
                .clone(),
        )
        .unwrap();
        assert!(projected.contains("\"model\": \"sonnet\""));
        assert!(projected.contains("\"language\": \"en\""));
        for denied in [
            "permissions",
            "API_TOKEN",
            "literal-secret",
            "enabledPlugins",
            "apiKey",
        ] {
            assert!(
                !projected.contains(denied),
                "projected settings leaked {denied}"
            );
        }
    }

    #[test]
    fn mcp_projection_replaces_literal_secrets_with_environment_references() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(
            &project.join(".mcp.json"),
            br#"{
              "mcpServers": {
                "docs": {
                  "command": "npx",
                  "args": ["-y", "docs-server"],
                  "env": {"DOCS_TOKEN": "literal-value"},
                  "headers": {"Authorization": "Bearer literal"},
                  "oauthToken": "never"
                }
              }
            }"#,
        );
        let request = CaptureRequest::for_project(&project);
        let inventory = discover_project(&request).unwrap();
        let mcp = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == ".mcp.json")
            .unwrap();
        assert!(mcp.blocked_reason.is_none());
        let captured =
            capture_selected(&request, &BTreeSet::from([mcp.resource_id.clone()])).unwrap();
        let projected =
            String::from_utf8(captured.files["project/.mcp.json"].bytes.clone()).unwrap();
        assert!(projected.contains("${DOCS_TOKEN}"));
        assert!(projected.contains("${AUTHORIZATION}"));
        assert!(!projected.contains("literal-value"));
        assert!(!projected.contains("Bearer literal"));
        assert!(!projected.contains("oauthToken"));
    }

    #[test]
    fn credential_file_and_symlink_block_skill_capture() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join(".agents/skills/bad")).unwrap();
        write(&project.join(".agents/skills/bad/.env"), b"TOKEN=secret");
        let request = CaptureRequest::for_project(&project);
        let error = discover_project(&request).unwrap_err();
        assert!(error.contains("no regular files") || error.contains("credential"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let clean = temp.path().join("clean");
            fs::create_dir_all(clean.join(".claude/skills/link")).unwrap();
            write(&clean.join("outside"), b"outside");
            symlink(
                clean.join("outside"),
                clean.join(".claude/skills/link/SKILL.md"),
            )
            .unwrap();
            assert!(discover_project(&CaptureRequest::for_project(clean)).is_err());
        }
    }

    #[test]
    fn filtered_index_contains_only_project_thread_ids() {
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
