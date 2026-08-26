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

use crate::conversation_codec::CodecLineage;
use mallard_domain as domain;

const MAX_DISCOVERED_FILES: usize = 20_000;
pub(crate) const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CAPTURE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_METADATA_LINES: usize = 128;
const MAX_TREE_DEPTH: usize = 32;

fn capture_file_limit(kind: &CaptureResourceKind) -> u64 {
    if matches!(kind, CaptureResourceKind::Conversation) {
        crate::app_settings::conversation_file_limit_bytes()
    } else {
        MAX_FILE_BYTES
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CaptureResourceKind {
    ProjectContentFile,
    ProjectContentDirectory,
    Conversation,
    ProviderState,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureApplyPolicy {
    SafeFile,
    Merge,
    Review,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResourceCandidate {
    pub resource_id: String,
    pub kind: CaptureResourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<domain::ProviderKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_resource_type: Option<String>,
    pub display_name: String,
    pub apply_policy: CaptureApplyPolicy,
    pub selected_by_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub logical_paths: Vec<String>,
    /// Kind-specific descriptor facts (provenance and naming evidence).
    /// Copied verbatim into the bundle descriptor metadata.
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
    pub unavailable_resource_ids: Vec<String>,
    pub warnings: Vec<String>,
    /// Provider-native session ids excluded as internal subagents, keyed by
    /// provider.
    pub excluded_subagent_session_ids: BTreeMap<domain::ProviderKey, BTreeSet<String>>,
}

/// A sidecar file consolidated under its conversation resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedSidecarRef {
    pub logical_path: String,
    pub sidecar_kind: String,
    /// Path relative to the provider's sidecar-kind directory. Materialization
    /// derives destination names from this.
    pub relative_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedConversation {
    pub provider: domain::ProviderKey,
    pub lineage_codec: String,
    pub source_thread_id: Option<String>,
    pub source_cwd: Option<String>,
    pub transcript_logical_path: String,
    /// Machine-local transcript source. Never serialized into a bundle;
    /// reviewed plans use it to pin local target expectations.
    pub source_physical_path: Option<PathBuf>,
    #[allow(clippy::struct_field_names)]
    pub sidecar_files: Vec<CapturedSidecarRef>,
    pub record_count: usize,
    pub lineage: CodecLineage,
    pub presentation: domain::SessionPresentation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub project_root: PathBuf,
    /// One home per bound provider, resolved from the selected profiles.
    pub provider_homes: BTreeMap<domain::ProviderKey, PathBuf>,
    /// Ordinary project content is intentionally lazy. Normal setup and
    /// provider inventory leave this false; the explicit Project files scan
    /// and a reviewed Push turn it on.
    pub include_project_content: bool,
    /// Additional canonical roots that generic content discovery must not
    /// enter (repository state, storage directories, nested bound project
    /// trees). Session ownership needs no exclusions: a nested project's
    /// sessions record a different home and are never owned here.
    pub excluded_content_roots: Vec<PathBuf>,
}

impl CaptureRequest {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            provider_homes: BTreeMap::new(),
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

/// Conversation identity discovered by a provider adapter, retained so
/// capture can run the typed codec and consolidate sidecars.
#[derive(Clone, Debug)]
struct DiscoveredConversationMeta {
    provider: domain::ProviderKey,
    /// The provider-native session id from the bounded head-of-file scan.
    provider_session_id: String,
    transcript_logical_path: String,
    sidecars: Vec<CapturedSidecarRef>,
}

#[derive(Clone, Debug)]
struct DiscoveredResource {
    descriptor: ResourceCandidate,
    files: Vec<SourceFile>,
    conversation: Option<DiscoveredConversationMeta>,
}

/// Profile-wide provider-native identities collected by the adapter's
/// existing bounded metadata discovery. This is deliberately retained once
/// per provider rather than cloned into every project-owned session view.
#[derive(Clone, Debug, Default)]
pub struct ProviderSessionInventory {
    provider_session_ids: BTreeSet<String>,
}

impl ProviderSessionInventory {
    pub fn provider_session_ids(&self) -> &BTreeSet<String> {
        &self.provider_session_ids
    }
}

/// One full provider walk. Fields stay private to this module: callers hold
/// a Discovery opaquely and feed it back into the inventory/capture views,
/// sharing the walk (and derived candidates such as the filtered session
/// index) across several consumers without re-scanning.
#[derive(Clone, Debug, Default)]
pub struct Discovery {
    resources: BTreeMap<String, DiscoveredResource>,
    directories: BTreeMap<String, SourceDirectory>,
    provider_session_inventories: BTreeMap<domain::ProviderKey, ProviderSessionInventory>,
    warnings: Vec<String>,
    ignored_count: usize,
    blocked_count: usize,
    excluded_subagent_session_ids: BTreeMap<domain::ProviderKey, BTreeSet<String>>,
    excluded_subagent_resource_ids: BTreeSet<String>,
}

impl Discovery {
    /// Combine independently cached provider-profile discoveries into the
    /// same opaque witness produced by one multi-provider walk.
    pub fn combine(discoveries: Vec<Self>) -> Result<Self, String> {
        let mut combined = Self::default();
        for discovery in discoveries {
            for (resource_id, resource) in discovery.resources {
                if combined
                    .resources
                    .insert(resource_id.clone(), resource)
                    .is_some()
                {
                    return Err(format!(
                        "provider discoveries produced duplicate resource '{}'",
                        resource_id
                    ));
                }
            }
            for (logical_path, directory) in discovery.directories {
                if combined
                    .directories
                    .insert(logical_path.clone(), directory)
                    .is_some()
                {
                    return Err(format!(
                        "provider discoveries produced duplicate directory '{}'",
                        logical_path
                    ));
                }
            }
            for (provider, inventory) in discovery.provider_session_inventories {
                let combined_inventory = combined
                    .provider_session_inventories
                    .entry(provider)
                    .or_default();
                combined_inventory
                    .provider_session_ids
                    .extend(inventory.provider_session_ids);
            }
            combined.warnings.extend(discovery.warnings);
            combined.ignored_count = combined
                .ignored_count
                .saturating_add(discovery.ignored_count);
            combined.blocked_count = combined
                .blocked_count
                .saturating_add(discovery.blocked_count);
            for (provider, session_ids) in discovery.excluded_subagent_session_ids {
                combined
                    .excluded_subagent_session_ids
                    .entry(provider)
                    .or_default()
                    .extend(session_ids);
            }
            combined
                .excluded_subagent_resource_ids
                .extend(discovery.excluded_subagent_resource_ids);
        }
        Ok(combined)
    }

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

    pub fn excluded_subagent_resource_ids(&self) -> &BTreeSet<String> {
        &self.excluded_subagent_resource_ids
    }

    pub fn provider_session_inventory(
        &self,
        provider: &domain::ProviderKey,
    ) -> Option<&ProviderSessionInventory> {
        self.provider_session_inventories.get(provider)
    }
}

/// Discover resources without retaining file bytes. Absolute source paths are
/// kept only inside this module and never become resource identity.
#[cfg(any(test, feature = "test-support"))]
pub fn discover_project(
    registry: &crate::providers::ProviderRegistry,
    request: &CaptureRequest,
) -> Result<ProjectInventory, String> {
    Ok(discover_full(registry, request)?.project_inventory())
}

/// Capture exactly the live resource IDs selected by the persistent recipe.
/// Missing or blocked resources are reported as unavailable; they are not
/// turned into deselections or tombstones.
#[cfg(any(test, feature = "test-support"))]
pub fn capture_selected(
    registry: &crate::providers::ProviderRegistry,
    request: &CaptureRequest,
    selected_resource_ids: &BTreeSet<String>,
) -> Result<CapturedResources, String> {
    let discovery = discover_full(registry, request)?;
    capture_from_discovery(registry, request, &discovery, selected_resource_ids)
}

/// `capture_selected` over an existing Discovery. Only the walk and its
/// derived candidates are shared: file bytes and metadata are re-read at
/// call time, so captured content is always current.
pub fn capture_from_discovery(
    registry: &crate::providers::ProviderRegistry,
    request: &CaptureRequest,
    discovery: &Discovery,
    selected_resource_ids: &BTreeSet<String>,
) -> Result<CapturedResources, String> {
    capture_from_discovery_with_workers(
        registry,
        request,
        discovery,
        selected_resource_ids,
        bounded_refresh_workers(selected_resource_ids.len()),
    )
}

/// Bounded worker count for the parallel refresh stages. The per-item work
/// is CPU-bound (serde and SHA-256), so past a few workers the wall clock
/// erodes into the largest-file floor while transient parse-tree memory
/// multiplies — hence the default cap of four; the Settings page moves it.
/// Retained capture bytes are unconditional and do not scale with workers.
pub fn bounded_refresh_workers(items: usize) -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .min(crate::app_settings::comparison_worker_cap())
        .min(items)
        .max(1)
}

/// One selected resource's read-free preflight output: the shared walk data
/// plus single-stat file metadata, with every cross-resource check already
/// settled so the parallel worker touches no shared state.
struct CapturePlanItem<'a> {
    resource_id: &'a str,
    discovered: &'a DiscoveredResource,
    directory: Option<&'a SourceDirectory>,
    /// `(source_mtime, mode)` per source file, in file order, from the
    /// preflight's single stat.
    file_meta: Vec<(u64, u32)>,
    directory_meta: Option<(u64, u32)>,
}

/// One resource captured by a worker, merged serially in resource-ID order.
struct CapturedUnit {
    files: Vec<CapturedFile>,
    directory: Option<CapturedDirectory>,
    conversation: Option<CapturedConversation>,
    resource: CapturedResource,
    warnings: Vec<String>,
}

fn take_capture_outcome(
    resource_id: &str,
    slot: &mut std::sync::OnceLock<Result<CapturedUnit, String>>,
) -> Result<CapturedUnit, String> {
    slot.take().unwrap_or_else(|| {
        Err(format!(
            "resource capture worker produced no outcome for resource '{}'",
            resource_id
        ))
    })
}

/// The per-resource parallel stage: read, secret-scan, digest, analyze, and
/// summarize one preflighted resource. Conversation analysis and
/// presentation are both derived from the retained byte snapshot; the
/// mutable provider path is never reopened after capture.
fn capture_resource(
    registry: &crate::providers::ProviderRegistry,
    item: &CapturePlanItem<'_>,
) -> Result<CapturedUnit, String> {
    let resource_id = item.resource_id;
    if let Some(source) = item.discovered.files.first() {
        crate::sync_metrics::record_selected_capture(&source.physical_path);
    }
    let mut warnings = Vec::new();
    let mut files = Vec::with_capacity(item.discovered.files.len());
    for (source, (source_mtime, mode)) in item
        .discovered
        .files
        .iter()
        .zip(item.file_meta.iter().copied())
    {
        let bytes = match &source.derived_bytes {
            Some(bytes) => bytes.clone(),
            None => fs::read(&source.physical_path)
                .map_err(|e| format!("read '{}': {}", source.physical_path.display(), e))?,
        };
        crate::sync_metrics::record_transcript_bytes_read(
            &source.physical_path,
            bytes.len() as u64,
        );
        // Sidecar and provider-state bytes are captured in wire form: the
        // credential scrub runs here, once, so review diffing, retention
        // dedup, and publish all see one consistent redacted byte stream.
        // Conversation transcripts stay local-form — review seeding and the
        // pull-time local target expectations read them — and are redacted
        // by their own wire builders at publish instead.
        let is_transcript = item
            .discovered
            .conversation
            .as_ref()
            .is_some_and(|meta| meta.transcript_logical_path == source.logical_path);
        let scrub = match item.discovered.descriptor.kind {
            CaptureResourceKind::ProviderState => true,
            CaptureResourceKind::Conversation => !is_transcript,
            CaptureResourceKind::ProjectContentFile
            | CaptureResourceKind::ProjectContentDirectory => false,
        };
        let bytes = if scrub {
            crate::credential_redaction::shared()
                .redact_document(&bytes)
                .map_err(|error| {
                    format!(
                        "resource '{}' failed credential redaction in '{}': {}",
                        resource_id, source.logical_path, error
                    )
                })?
                .0
        } else {
            bytes
        };
        if let Some(secret) = detect_secret_material(&bytes) {
            if source.opaque_content {
                warnings.push(format!(
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
        files.push(CapturedFile {
            logical_path: source.logical_path.clone(),
            resource_id: resource_id.to_string(),
            source_mtime,
            mode,
            bytes,
        });
    }
    let directory = item.directory.map(|source| {
        let (source_mtime, mode) = item
            .directory_meta
            .expect("preflight recorded directory metadata");
        CapturedDirectory {
            logical_path: source.logical_path.clone(),
            resource_id: resource_id.to_string(),
            source_mtime,
            mode,
        }
    });
    if let Some(source) = item.discovered.files.first() {
        // The resource digest folds every captured byte (plus bounded
        // metadata strings the count ignores).
        crate::sync_metrics::record_bytes_hashed(
            &source.physical_path,
            files.iter().map(|file| file.bytes.len() as u64).sum(),
        );
    }
    let content_sha256 = resource_digest(&files, directory.as_ref());
    let conversation = if let Some(conversation_meta) = &item.discovered.conversation {
        let adapter = registry.require_sessions(&conversation_meta.provider)?;
        let transcript = files
            .iter()
            .find(|file| file.logical_path == conversation_meta.transcript_logical_path)
            .ok_or_else(|| format!("conversation '{}' transcript was not captured", resource_id))?;
        let transcript_source = item
            .discovered
            .files
            .iter()
            .find(|source| source.logical_path == conversation_meta.transcript_logical_path)
            .ok_or_else(|| {
                format!(
                    "conversation '{}' transcript source is missing",
                    resource_id
                )
            })?;
        let analysis = adapter.analyze(&transcript.bytes);
        let presentation = match adapter.summarize_bytes(&transcript.bytes) {
            Ok(file_summary) => session_presentation(&file_summary),
            Err(_) => domain::SessionPresentation::default(),
        };
        Some(CapturedConversation {
            provider: conversation_meta.provider.clone(),
            lineage_codec: adapter.codec_key().to_string(),
            source_thread_id: analysis.source_provider_session_id,
            source_cwd: analysis.source_cwd,
            transcript_logical_path: conversation_meta.transcript_logical_path.clone(),
            source_physical_path: Some(transcript_source.physical_path.clone()),
            sidecar_files: conversation_meta.sidecars.clone(),
            record_count: analysis.record_count,
            lineage: analysis.lineage,
            presentation,
        })
    } else {
        None
    };
    Ok(CapturedUnit {
        files,
        directory,
        conversation,
        resource: CapturedResource {
            descriptor: item.discovered.descriptor.clone(),
            content_sha256,
        },
        warnings,
    })
}

/// The staged capture pipeline with an explicit worker count; `workers = 1`
/// is the serial degenerate of the same phases. Tests pin worker-count
/// invariance through this entry.
fn capture_from_discovery_with_workers(
    registry: &crate::providers::ProviderRegistry,
    request: &CaptureRequest,
    discovery: &Discovery,
    selected_resource_ids: &BTreeSet<String>,
    workers: usize,
) -> Result<CapturedResources, String> {
    let canonical_project = canonical_existing_dir(&request.project_root, "project root")?;
    let mut result = CapturedResources {
        warnings: discovery.warnings.clone(),
        excluded_subagent_session_ids: discovery.excluded_subagent_session_ids.clone(),
        ..CapturedResources::default()
    };

    // Phase 1 — serial, read-free preflight. Every cross-resource check
    // (case-fold collisions, logical-path ownership, per-file and aggregate
    // size gates) settles here from one stat per file, before any worker
    // reads a byte.
    let mut total_bytes = 0_u64;
    let mut casefold_paths = BTreeMap::<String, String>::new();
    let mut owned_file_paths = BTreeSet::<String>::new();
    let mut owned_directory_paths = BTreeSet::<String>::new();
    let mut items = Vec::new();
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
        let mut file_meta = Vec::with_capacity(discovered.files.len());
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
            if !owned_file_paths.insert(source.logical_path.clone()) {
                return Err(format!(
                    "logical path '{}' is owned by more than one resource",
                    source.logical_path
                ));
            }
            file_meta.push((modified_secs(&meta), safe_mode(&meta)));
        }
        let directory = discovery.directories.get(resource_id);
        let directory_meta = if let Some(source) = directory {
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
            if !owned_directory_paths.insert(source.logical_path.clone()) {
                return Err(format!(
                    "logical directory '{}' is owned by more than one resource",
                    source.logical_path
                ));
            }
            Some((modified_secs(&meta), safe_mode(&meta)))
        } else {
            None
        };
        items.push(CapturePlanItem {
            resource_id,
            discovered,
            directory,
            file_meta,
            directory_meta,
        });
    }

    // Phase 2 — bounded parallel per-resource capture. Items are dispatched
    // in resource-ID order: an error at item k proves every earlier item was
    // already dispatched and will complete, so the merge's earliest-failure
    // rule stays deterministic at every worker count even with fast-fail
    // cancellation.
    let mut slots: Vec<std::sync::OnceLock<Result<CapturedUnit, String>>> =
        items.iter().map(|_| std::sync::OnceLock::new()).collect();
    if !items.is_empty() {
        let cursor = std::sync::atomic::AtomicUsize::new(0);
        let failed = std::sync::atomic::AtomicBool::new(false);
        let item_slots = &slots;
        let work_items = &items;
        std::thread::scope(|scope| {
            for _ in 0..workers.clamp(1, work_items.len()) {
                scope.spawn(|| loop {
                    if failed.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let index = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = work_items.get(index) else {
                        break;
                    };
                    let outcome = capture_resource(registry, item);
                    if outcome.is_err() {
                        failed.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    let _ = item_slots[index].set(outcome);
                });
            }
        });
    }

    // Phase 3 — serial merge in resource-ID order: deterministic output and
    // the earliest failing resource's error, regardless of completion order.
    // An unexpectedly unset slot is a controlled stage error, never a panic.
    for (item, slot) in items.iter().zip(slots.iter_mut()) {
        let unit = take_capture_outcome(item.resource_id, slot)?;
        result
            .resources
            .insert(item.resource_id.to_string(), unit.resource);
        if let Some(conversation) = unit.conversation {
            result
                .conversations
                .insert(item.resource_id.to_string(), conversation);
        }
        for file in unit.files {
            result.files.insert(file.logical_path.clone(), file);
        }
        if let Some(directory) = unit.directory {
            result
                .directories
                .insert(directory.logical_path.clone(), directory);
        }
        result.warnings.extend(unit.warnings);
    }
    result.unavailable_resource_ids.sort();
    result.warnings.sort();
    result.warnings.dedup();
    Ok(result)
}

/// Bounded, provider-neutral presentation metadata for one captured
/// conversation. Recorded SHAs are dropped unless they already satisfy the
/// portable lowercase-hex contract.
fn session_presentation(
    file_summary: &crate::providers::SessionFileSummary,
) -> domain::SessionPresentation {
    let recorded_sha = file_summary.recorded_sha.clone().filter(|sha| {
        !sha.is_empty()
            && sha.len() <= 64
            && sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    let branch = file_summary
        .branch
        .clone()
        .filter(|branch| !branch.trim().is_empty() && branch.len() <= 1_024);
    domain::SessionPresentation {
        started_at: file_summary.started_at,
        ended_at: file_summary.ended_at,
        branch,
        recorded_sha,
        metrics: domain::SessionMetrics {
            user_rounds: file_summary.user_rounds,
            agent_messages: file_summary.agent_messages,
            tool_calls: file_summary.tool_calls,
            total_input_tokens: file_summary.total_input_tokens,
            total_output_tokens: file_summary.total_output_tokens,
            total_tokens: file_summary.total_tokens,
            complete: file_summary.metrics_complete,
        },
    }
}

/// Domain-facing capture entry point. The recipe remains the sole selection
/// authority; no transient UI filter is accepted here.
#[cfg_attr(not(test), allow(dead_code))]
pub fn capture_recipe(
    registry: &crate::providers::ProviderRegistry,
    request: &CaptureRequest,
    recipe: &domain::BundleRecipe,
) -> Result<CapturedResources, String> {
    let discovery = discover_full(registry, request)?;
    capture_recipe_from_discovery(registry, request, &discovery, recipe)
}

/// `capture_recipe` over an existing Discovery, sharing one walk between
/// selection persistence, freshness checks, and the byte capture.
pub fn capture_recipe_from_discovery(
    registry: &crate::providers::ProviderRegistry,
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
    capture_from_discovery(registry, request, discovery, &selected)
}

/// Convert captured descriptors into the shared current domain model. This
/// is the only translation boundary; provider scanning stays independent of
/// manifest/storage concerns.
pub fn domain_resources(
    captured: &CapturedResources,
) -> Result<BTreeMap<domain::ResourceId, domain::ResourceDescriptor>, String> {
    let mut resources = BTreeMap::new();
    for resource in captured.resources.values() {
        let descriptor =
            domain_resource_descriptor(&resource.descriptor, Some(&resource.content_sha256))?;
        resources.insert(descriptor.resource_id.clone(), descriptor);
    }
    Ok(resources)
}

/// The typed domain descriptor for one discovered candidate. A captured
/// resource carries its content digest; a metadata-only view has none and
/// omits the `content_sha256` metadata key.
pub fn domain_resource_descriptor(
    candidate: &ResourceCandidate,
    content_sha256: Option<&str>,
) -> Result<domain::ResourceDescriptor, String> {
    let resource_id = domain::ResourceId::parse(candidate.resource_id.clone())?;
    let kind = domain_kind(&candidate.kind);
    let provenance = match candidate.kind {
        CaptureResourceKind::ProjectContentFile | CaptureResourceKind::ProjectContentDirectory => {
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
    if let Some(content_sha256) = content_sha256 {
        metadata.insert("content_sha256".to_string(), content_sha256.to_string());
    }
    let descriptor = domain::ResourceDescriptor {
        resource_id,
        kind,
        provider: candidate.provider.clone(),
        provider_resource_type: candidate
            .provider_resource_type
            .as_deref()
            .map(domain::ResourceTypeKey::parse)
            .transpose()?,
        display_name: candidate.display_name.clone(),
        provenance,
        apply_policy: domain_apply_policy(&candidate.apply_policy),
        codec_version: 1,
        metadata,
    };
    descriptor.validate()?;
    Ok(descriptor)
}

/// One discovered session as bounded metadata: identity, ownership, display
/// name, physical source, and a fresh no-follow stamp — everything status
/// and preliminary review classification need, with no transcript read.
#[derive(Clone, Debug)]
pub struct LocalSessionView {
    pub descriptor: ResourceCandidate,
    pub provider: domain::ProviderKey,
    pub provider_thread_id: String,
    pub transcript_logical_path: String,
    pub source_path: PathBuf,
    pub source_mtime: u64,
    pub source_size: u64,
}

/// Re-run only the session-index title enrichment over a cached discovery:
/// a replaced `session_index.jsonl` invalidates display titles, never tail
/// evidence or membership.
pub fn refresh_codex_session_titles(
    discovery: &mut Discovery,
    canonical_home: &Path,
) -> Result<(), String> {
    let codex = domain::ProviderKey::parse("codex")?;
    let ids = discovery
        .resources
        .values()
        .filter_map(|resource| resource.conversation.as_ref())
        .filter(|conversation| conversation.provider == codex)
        .map(|conversation| conversation.provider_session_id.clone())
        .collect::<BTreeSet<_>>();
    let index = canonical_home.join("session_index.jsonl");
    let titles = if !ids.is_empty() && index.is_file() {
        crate::providers::codex::history::parse_session_index_titles(
            &filter_codex_session_index_by_ids(&index, &ids)?,
        )
    } else {
        BTreeMap::new()
    };
    for resource in discovery.resources.values_mut() {
        let Some(conversation) = &resource.conversation else {
            continue;
        };
        if conversation.provider != codex {
            continue;
        }
        resource.descriptor.display_name = titles
            .get(&conversation.provider_session_id)
            .cloned()
            .unwrap_or_else(|| conversation.provider_session_id.clone());
    }
    Ok(())
}

/// Project the discovery's conversation candidates into metadata-only
/// session views. A source that fails its fresh no-follow stat becomes a
/// blocked view, matching the row discovery would have blocked.
pub fn local_session_views(discovery: &Discovery) -> Vec<LocalSessionView> {
    let mut views = Vec::new();
    for resource in discovery.resources.values() {
        let Some(conversation) = &resource.conversation else {
            continue;
        };
        let Some(source) = resource
            .files
            .iter()
            .find(|file| file.logical_path == conversation.transcript_logical_path)
        else {
            continue;
        };
        let mut descriptor = resource.descriptor.clone();
        let (source_mtime, source_size) = match fs::symlink_metadata(&source.physical_path) {
            Ok(meta) if meta.is_file() => (modified_secs(&meta), meta.len()),
            _ => {
                if descriptor.blocked_reason.is_none() {
                    descriptor.blocked_reason =
                        Some("session file is no longer readable".to_string());
                }
                (0, 0)
            }
        };
        views.push(LocalSessionView {
            provider: conversation.provider.clone(),
            provider_thread_id: conversation.provider_session_id.clone(),
            transcript_logical_path: conversation.transcript_logical_path.clone(),
            source_path: source.physical_path.clone(),
            source_mtime,
            source_size,
            descriptor,
        });
    }
    views
}

fn domain_kind(kind: &CaptureResourceKind) -> domain::ResourceKind {
    match kind {
        CaptureResourceKind::ProjectContentFile => domain::ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => {
            domain::ResourceKind::ProjectContentDirectory
        }
        CaptureResourceKind::Conversation => domain::ResourceKind::Conversation,
        CaptureResourceKind::ProviderState => domain::ResourceKind::ProviderState,
    }
}

fn domain_apply_policy(policy: &CaptureApplyPolicy) -> domain::ApplyPolicy {
    match policy {
        CaptureApplyPolicy::SafeFile => domain::ApplyPolicy::SafeFile,
        CaptureApplyPolicy::Merge => domain::ApplyPolicy::Merge,
        CaptureApplyPolicy::Review => domain::ApplyPolicy::ExplicitReview,
    }
}

/// The one full provider walk behind every inventory and capture view.
pub fn discover_full(
    registry: &crate::providers::ProviderRegistry,
    request: &CaptureRequest,
) -> Result<Discovery, String> {
    let canonical_project = canonical_existing_dir(&request.project_root, "project root")?;
    let mut discovery = Discovery {
        resources: BTreeMap::new(),
        directories: BTreeMap::new(),
        provider_session_inventories: BTreeMap::new(),
        warnings: Vec::new(),
        ignored_count: 0,
        blocked_count: 0,
        excluded_subagent_session_ids: BTreeMap::new(),
        excluded_subagent_resource_ids: BTreeSet::new(),
    };
    if request.include_project_content {
        discover_generic_project_content(request, &canonical_project, &mut discovery)?;
    }
    for (provider, home) in &request.provider_homes {
        let adapter = registry.require_sessions(provider)?;
        let context = crate::providers::SessionDiscoveryContext {
            provider_home: home,
            project_root: &canonical_project,
        };
        let session_discovery = adapter.discover(&context)?;
        install_provider_discovery(&mut discovery, provider, session_discovery)?;
    }
    if discovery.resources.len() > MAX_DISCOVERED_FILES {
        return Err(format!(
            "project inventory exceeds {} resources",
            MAX_DISCOVERED_FILES
        ));
    }
    Ok(discovery)
}

/// Fold one adapter's discovery into the shared candidate set: session
/// resources with consolidated sidecars, provider-state resources, warnings,
/// and subagent exclusions. Resource IDs and logical namespaces are composed
/// here so every provider follows one identity scheme.
fn install_provider_discovery(
    discovery: &mut Discovery,
    provider: &domain::ProviderKey,
    result: crate::providers::SessionDiscovery,
) -> Result<(), String> {
    let inventory = discovery
        .provider_session_inventories
        .entry(provider.clone())
        .or_default();
    inventory
        .provider_session_ids
        .extend(result.profile_session_ids.iter().cloned());
    discovery.warnings.extend(result.warnings);
    for excluded in result.excluded_subagent_session_ids {
        let resource_id = format!("{}:session:{}", provider, resource_id_component(&excluded));
        discovery
            .excluded_subagent_session_ids
            .entry(provider.clone())
            .or_default()
            .insert(excluded);
        if discovery.excluded_subagent_resource_ids.insert(resource_id) {
            discovery.ignored_count = discovery.ignored_count.saturating_add(1);
        }
    }
    let mut sidecars_by_session: BTreeMap<String, Vec<crate::providers::DiscoveredSidecar>> =
        BTreeMap::new();
    for sidecar in result.sidecars {
        sidecars_by_session
            .entry(sidecar.session_id.clone())
            .or_default()
            .push(sidecar);
    }
    for session in result.sessions {
        let component = resource_id_component(&session.session_id);
        let resource_id = format!("{}:session:{}", provider, component);
        let transcript_logical = format!(
            "state/providers/{}/sessions/{}/transcript.jsonl",
            provider, component
        );
        let mut files = vec![SourceFile {
            physical_path: session.physical_path,
            logical_path: transcript_logical.clone(),
            opaque_content: true,
            derived_bytes: None,
        }];
        let mut logical_paths = vec![transcript_logical.clone()];
        let mut blocked_reason = session.blocked_reason;
        let mut sidecar_refs = Vec::new();
        for sidecar in sidecars_by_session
            .remove(&session.session_id)
            .unwrap_or_default()
        {
            let logical = format!(
                "state/providers/{}/sessions/{}/sidecars/{}/{}",
                provider, component, sidecar.sidecar_kind, sidecar.file_name
            );
            if blocked_reason.is_none() {
                blocked_reason = sidecar.blocked_reason;
            }
            files.push(SourceFile {
                physical_path: sidecar.physical_path,
                logical_path: logical.clone(),
                opaque_content: true,
                derived_bytes: None,
            });
            logical_paths.push(logical.clone());
            sidecar_refs.push(CapturedSidecarRef {
                logical_path: logical,
                sidecar_kind: sidecar.sidecar_kind,
                relative_path: sidecar.file_name,
            });
        }
        insert_resource(
            discovery,
            DiscoveredResource {
                descriptor: ResourceCandidate {
                    metadata: BTreeMap::new(),
                    resource_id,
                    kind: CaptureResourceKind::Conversation,
                    provider: Some(provider.clone()),
                    provider_resource_type: Some("session".to_string()),
                    display_name: session.display_name,
                    apply_policy: CaptureApplyPolicy::SafeFile,
                    selected_by_default: true,
                    blocked_reason,
                    logical_paths,
                },
                files,
                conversation: Some(DiscoveredConversationMeta {
                    provider: provider.clone(),
                    provider_session_id: session.session_id,
                    transcript_logical_path: transcript_logical,
                    sidecars: sidecar_refs,
                }),
            },
        )?;
    }
    for state in result.state_resources {
        let resource_id = format!(
            "{}:{}:{}",
            provider,
            state.resource_type,
            resource_id_component(&state.state_component)
        );
        let files = state
            .files
            .into_iter()
            .map(|file| SourceFile {
                physical_path: file.physical_path,
                logical_path: format!("state/providers/{}/{}", provider, file.logical_suffix),
                opaque_content: true,
                derived_bytes: None,
            })
            .collect::<Vec<_>>();
        if files.is_empty() {
            continue;
        }
        let logical_paths = files
            .iter()
            .map(|file| file.logical_path.clone())
            .collect::<Vec<_>>();
        insert_resource(
            discovery,
            DiscoveredResource {
                descriptor: ResourceCandidate {
                    metadata: BTreeMap::new(),
                    resource_id,
                    kind: CaptureResourceKind::ProviderState,
                    provider: Some(provider.clone()),
                    provider_resource_type: Some(state.resource_type),
                    display_name: state.display_name,
                    apply_policy: if state.merge_apply {
                        CaptureApplyPolicy::Merge
                    } else {
                        CaptureApplyPolicy::SafeFile
                    },
                    selected_by_default: true,
                    blocked_reason: state.blocked_reason,
                    logical_paths,
                },
                files,
                conversation: None,
            },
        )?;
    }
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
    discovery: &mut Discovery,
) -> Result<(), String> {
    let ignore_rules = load_project_ignore_rules(root, &mut discovery.warnings)?;
    let mut excluded_roots = Vec::new();
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
        if relative.split('/').any(denied_file_name) {
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
                provider_resource_type: None,
                display_name: relative.to_string(),
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: true,
                blocked_reason: None,
                logical_paths: vec![logical_path.clone()],
                metadata,
            },
            files: Vec::new(),
            conversation: None,
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
                provider_resource_type: None,
                display_name: relative.to_string(),
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: true,
                blocked_reason: None,
                logical_paths: vec![logical_path.clone()],
                metadata,
            },
            files: vec![SourceFile {
                physical_path: path.to_path_buf(),
                logical_path,
                opaque_content: true,
                derived_bytes: None,
            }],
            conversation: None,
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
                provider_resource_type: None,
                display_name: relative.to_string(),
                apply_policy: CaptureApplyPolicy::Review,
                selected_by_default: false,
                blocked_reason: Some(reason.to_string()),
                logical_paths: vec![format!("project/{}", relative)],
                metadata,
            },
            files: Vec::new(),
            conversation: None,
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

/// One complete provider-owned metadata record found in a transcript head.
/// Adapters construct this only from their exact current wire shape; the
/// shared scanner never searches arbitrary JSON for identity fields.
pub(crate) struct ParsedSessionMetadata {
    pub session_id: String,
    pub cwd: String,
    pub is_internal_subagent: bool,
}

pub(crate) fn scan_session_metadata_with(
    path: &Path,
    parse_record: fn(&[u8]) -> Option<ParsedSessionMetadata>,
) -> Result<crate::providers::SessionFileMetadata, String> {
    crate::sync_metrics::record_file_head_read(path);
    let file = fs::File::open(path).map_err(|e| format!("open: {}", e))?;
    let mut reader = BufReader::new(file);
    let mut total = 0_usize;
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
        if let Some(metadata) = parse_record(&line) {
            return validated_session_metadata(
                metadata.session_id,
                metadata.cwd,
                metadata.is_internal_subagent,
            );
        }
    }
    Err("authoritative session metadata was not found in bounded file head".to_string())
}

pub(crate) fn validated_session_metadata(
    id: String,
    cwd: String,
    is_internal_subagent: bool,
) -> Result<crate::providers::SessionFileMetadata, String> {
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
    Ok(crate::providers::SessionFileMetadata {
        session_id: id,
        cwd,
        is_internal_subagent,
    })
}

/// The exact-home ownership predicate: a session belongs to the project
/// iff its authoritative home canonicalizes to the project root itself.
/// A subdirectory is a different project; containment never confers
/// ownership.
pub(crate) fn session_home_is_project_root(project_root: &Path, cwd: &Path) -> bool {
    match fs::canonicalize(cwd) {
        Ok(canonical_cwd) => canonical_cwd == project_root,
        Err(_) => false,
    }
}

#[derive(Deserialize)]
struct CodexSessionIndexIdentity {
    id: String,
}

pub(crate) fn filter_codex_session_index_by_ids(
    path: &Path,
    ids: &BTreeSet<String>,
) -> Result<Vec<u8>, String> {
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
        let row: CodexSessionIndexIdentity = match serde_json::from_slice(&line) {
            Ok(row) => row,
            Err(_) => continue,
        };
        if ids.contains(&row.id) {
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

pub(crate) fn inspect_candidate_files(
    approved_root: &Path,
    files: &[PathBuf],
) -> Result<(), String> {
    inspect_candidate_files_with_limit(approved_root, files, MAX_FILE_BYTES)
}

pub(crate) fn inspect_candidate_files_with_limit(
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
    for (provider, home) in &request.provider_homes {
        approved_roots.push(canonical_existing_dir(home, &format!("{} home", provider))?);
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
        || request.excluded_content_roots.iter().any(|excluded| {
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

pub(crate) fn collect_regular_tree_files(
    approved_root: &Path,
    tree: PathBuf,
) -> Result<Vec<PathBuf>, String> {
    let meta =
        fs::symlink_metadata(&tree).map_err(|e| format!("inspect '{}': {}", tree.display(), e))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "symlinked resource '{}' is not captured",
            tree.display()
        ));
    }
    if meta.is_file() {
        inspect_candidate_files(approved_root, std::slice::from_ref(&tree))?;
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
    if !result.is_empty() {
        inspect_candidate_files(approved_root, &result)?;
    }
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

pub(crate) fn canonical_existing_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
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

pub(crate) fn normalized_relative(root: &Path, path: &Path) -> Result<String, String> {
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

pub(crate) fn provider_walk_entry(entry: &DirEntry) -> bool {
    if entry.file_type().is_dir() {
        crate::sync_metrics::record_directory_visited(entry.path());
    }
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

fn resource_digest(files: &[CapturedFile], directory: Option<&CapturedDirectory>) -> String {
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
    hex_digest(hasher.finalize().as_slice())
}

pub(crate) fn resource_id_component(value: &str) -> String {
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
    use crate::providers::ProviderRegistry;
    use std::io::Write;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn registry() -> &'static ProviderRegistry {
        ProviderRegistry::production()
    }

    fn codex_key() -> domain::ProviderKey {
        domain::ProviderKey::parse("codex").unwrap()
    }

    fn claude_key() -> domain::ProviderKey {
        domain::ProviderKey::parse("claude").unwrap()
    }

    #[test]
    fn codex_discovery_retains_profile_wide_ids_and_unions_duplicate_sources() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let outside = temp.path().join("outside-project");
        let codex = temp.path().join("codex");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let outside_cwd = serde_json::to_string(outside.to_str().unwrap()).unwrap();
        let child = "019f7900-0000-7000-8000-000000000001";
        for directory in ["sessions", "archived_sessions"] {
            write(
                &codex.join(format!("{directory}/2026/08/19/rollout-{child}.jsonl")),
                format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{child}\",\"cwd\":{outside_cwd}}}}}\n"
                )
                .as_bytes(),
            );
        }

        let request = CaptureRequest {
            project_root: project,
            provider_homes: BTreeMap::from([(codex_key(), codex)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let discovery = discover_full(registry(), &request).unwrap();
        assert!(
            discovery.project_inventory().resources.is_empty(),
            "the fixture is intentionally outside the selected project"
        );
        let inventory = discovery.provider_session_inventory(&codex_key()).unwrap();
        assert_eq!(
            inventory.provider_session_ids(),
            &BTreeSet::from([child.to_string()])
        );
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
            provider_homes: BTreeMap::from([(codex_key(), codex)]),
            include_project_content: true,
            excluded_content_roots: Vec::new(),
        };
        let discovery = discover_full(registry(), &request).unwrap();
        let all_ids: BTreeSet<String> = discovery
            .project_inventory()
            .resources
            .iter()
            .map(|resource| resource.resource_id.clone())
            .collect();
        assert!(all_ids.len() >= 2, "fixture must span several resources");

        let direct = capture_selected(registry(), &request, &all_ids).unwrap();
        let shared = capture_from_discovery(registry(), &request, &discovery, &all_ids).unwrap();
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
        let first =
            capture_from_discovery(registry(), &request, &discovery, &conversation_ids).unwrap();
        let second = capture_from_discovery(registry(), &request, &discovery, &other_ids).unwrap();
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
    fn codex_and_claude_membership_requires_the_exact_home() {
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
                serde_json::to_string(project.to_str().unwrap()).unwrap()
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
        // A subdirectory is a different project: its sessions are never
        // owned here, with no exclusion list involved.
        write(
            &codex.join("sessions/2026/07/17/rollout-child.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-c\",\"cwd\":{}}}}}\n",
                serde_json::to_string(child.to_str().unwrap()).unwrap()
            )
            .as_bytes(),
        );
        write(
            &claude.join("projects/source/session-a.jsonl"),
            format!(
                concat!(
                    "{{\"type\":\"attachment\",\"uuid\":\"attachment-a\",",
                    "\"parentUuid\":null,\"sessionId\":\"session-a\",\"cwd\":{},",
                    "\"attachment\":{{\"type\":\"hook_success\"}}}}\n",
                    "{{\"type\":\"user\",\"uuid\":\"user-a\",",
                    "\"parentUuid\":\"attachment-a\",\"sessionId\":\"session-a\",",
                    "\"cwd\":{},\"message\":{{\"role\":\"user\",",
                    "\"content\":\"test request\"}}}}\n"
                ),
                serde_json::to_string(project.to_str().unwrap()).unwrap(),
                serde_json::to_string(project.to_str().unwrap()).unwrap(),
            )
            .as_bytes(),
        );
        fs::create_dir_all(claude.join("projects/source/memory")).unwrap();

        let request = CaptureRequest {
            project_root: project,
            provider_homes: BTreeMap::from([(codex_key(), codex), (claude_key(), claude)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(registry(), &request).unwrap();
        assert!(inventory
            .resources
            .iter()
            .any(|resource| resource.resource_id == "codex:session:thread-a"));
        for foreign in ["codex:session:thread-b", "codex:session:thread-c"] {
            assert!(
                !inventory
                    .resources
                    .iter()
                    .any(|resource| resource.resource_id == foreign),
                "{foreign} must not be owned"
            );
        }
        let claude = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "claude:session:session-a")
            .unwrap();
        assert_eq!(
            claude.logical_paths[0],
            "state/providers/claude/sessions/session-a/transcript.jsonl"
        );

        let selected = BTreeSet::from([claude.resource_id.clone()]);
        let captured = capture_selected(registry(), &request, &selected).unwrap();
        let conversation = captured
            .conversations
            .get("claude:session:session-a")
            .unwrap();
        assert_eq!(conversation.record_count, 2);
        assert_eq!(conversation.lineage.chain().unwrap().len(), 2);
    }

    #[test]
    fn claude_discovery_follows_the_resumed_leaf_home_not_copied_history() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let original_cwd = temp.path().join("original");
        let claude = temp.path().join("claude");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&original_cwd).unwrap();
        fs::create_dir_all(&claude).unwrap();

        // session-a lives in a foreign directory; session-b copied its
        // history but was resumed with a leaf inside the project. Ownership
        // follows each session's own leaf home: the copied foreign record
        // neither blocks session-b nor pulls session-a in.
        let copied_record = serde_json::json!({
            "type": "user",
            "uuid": "user-original",
            "parentUuid": null,
            "sessionId": "session-a",
            "cwd": original_cwd,
            "message": {"role": "user", "content": "original"}
        });
        write(
            &claude.join("projects/source/session-a.jsonl"),
            format!("{}\n", copied_record).as_bytes(),
        );
        let resumed_leaf = serde_json::json!({
            "type": "assistant",
            "uuid": "assistant-resumed",
            "parentUuid": "user-original",
            "sessionId": "session-b",
            "cwd": project,
            "message": {"role": "assistant", "content": [{
                "type": "text", "text": "resumed"
            }]}
        });
        write(
            &claude.join("projects/source/session-b.jsonl"),
            format!("{}\n{}\n", copied_record, resumed_leaf).as_bytes(),
        );

        let request = CaptureRequest {
            project_root: project,
            provider_homes: BTreeMap::from([(claude_key(), claude)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(registry(), &request).unwrap();
        let sessions = inventory
            .resources
            .iter()
            .filter(|resource| resource.kind == CaptureResourceKind::Conversation)
            .map(|resource| resource.resource_id.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(sessions, BTreeSet::from(["claude:session:session-b"]));
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
            provider_homes: BTreeMap::from([(codex_key(), codex)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(registry(), &request).unwrap();
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
            provider_homes: BTreeMap::from([(codex_key(), codex)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let discovery = discover_full(registry(), &request).unwrap();
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
        let captured = capture_from_discovery(registry(), &request, &discovery, &selected).unwrap();
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
            captured.excluded_subagent_session_ids,
            BTreeMap::from([(
                codex_key(),
                BTreeSet::from([
                    "child-source".to_string(),
                    "child-thread-source".to_string(),
                ]),
            )])
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
            provider_homes: BTreeMap::from([(codex_key(), codex)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(registry(), &request).unwrap();
        let matching = inventory
            .resources
            .iter()
            .filter(|resource| resource.resource_id == "codex:session:duplicate-thread")
            .collect::<Vec<_>>();

        assert_eq!(matching.len(), 1);
        assert!(matching[0].logical_paths[0].starts_with("state/providers/codex/sessions/"));
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
        writeln!(file, "{{\"id\":\"a\",\"thread_name\":\"Unselected\"}}").unwrap();
        writeln!(file, "{{\"id\":\"b\",\"thread_name\":\"Selected\"}}").unwrap();
        writeln!(
            file,
            "{{\"event\":{{\"id\":\"b\"}},\"thread_name\":\"Nested\"}}"
        )
        .unwrap();
        writeln!(file, "{{\"sessionId\":\"b\",\"thread_name\":\"Alias\"}}").unwrap();
        let bytes =
            filter_codex_session_index_by_ids(&path, &BTreeSet::from(["b".to_string()])).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let rows = text.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(rows[0]).unwrap()["thread_name"],
            "Selected"
        );
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
            let inventory = discover_project(registry(), &request).unwrap();
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
        let captured = capture_selected(registry(), &first_request, &selected).unwrap();
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
        request.excluded_content_roots = vec![project.join("child-project")];
        let inventory = discover_project(registry(), &request).unwrap();
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
        let inventory = discover_project(registry(), &request).unwrap();
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
        let inventory = discover_project(registry(), &request).unwrap();
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
        let error = discover_project(registry(), &request).unwrap_err();
        assert!(error.contains("exceeds 20000 resources"), "{error}");
    }

    /// Build a Discovery directly so the pipeline tests control resources
    /// and physical files without walk heuristics. All files are
    /// provider-state resources under the request's codex home.
    fn synthetic_discovery(resources: Vec<(&str, Vec<SourceFile>)>) -> Discovery {
        Discovery {
            resources: resources
                .into_iter()
                .map(|(id, files)| {
                    (
                        id.to_string(),
                        DiscoveredResource {
                            descriptor: ResourceCandidate {
                                resource_id: id.to_string(),
                                kind: CaptureResourceKind::ProviderState,
                                provider: None,
                                provider_resource_type: None,
                                display_name: id.to_string(),
                                apply_policy: CaptureApplyPolicy::SafeFile,
                                selected_by_default: true,
                                blocked_reason: None,
                                logical_paths: files
                                    .iter()
                                    .map(|file| file.logical_path.clone())
                                    .collect(),
                                metadata: BTreeMap::new(),
                            },
                            files,
                            conversation: None,
                        },
                    )
                })
                .collect(),
            directories: BTreeMap::new(),
            provider_session_inventories: BTreeMap::new(),
            warnings: Vec::new(),
            ignored_count: 0,
            blocked_count: 0,
            excluded_subagent_session_ids: BTreeMap::new(),
            excluded_subagent_resource_ids: BTreeSet::new(),
        }
    }

    fn pipeline_fixture(temp: &Path) -> (CaptureRequest, PathBuf) {
        let project = temp.join("project");
        let home = temp.join("home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&home).unwrap();
        // Physical fixture paths must be canonical, as discovery's are.
        let project = fs::canonicalize(&project).unwrap();
        let home = fs::canonicalize(&home).unwrap();
        let mut request = CaptureRequest::for_project(&project);
        request.provider_homes.insert(codex_key(), home.clone());
        (request, home)
    }

    fn source_file(home: &Path, name: &str, bytes: &[u8], opaque: bool) -> SourceFile {
        let physical = home.join(name);
        fs::write(&physical, bytes).unwrap();
        SourceFile {
            physical_path: physical,
            logical_path: format!("state/codex/{name}"),
            opaque_content: opaque,
            derived_bytes: None,
        }
    }

    #[test]
    fn capture_worker_outcome_preserves_errors_and_rejects_missing_slots() {
        let mut failed = std::sync::OnceLock::new();
        assert!(failed
            .set(Err("sentinel capture error".to_string()))
            .is_ok());
        assert_eq!(
            take_capture_outcome("resource-a", &mut failed)
                .err()
                .as_deref(),
            Some("sentinel capture error")
        );

        let mut missing = std::sync::OnceLock::new();
        assert_eq!(
            take_capture_outcome("resource-b", &mut missing)
                .err()
                .as_deref(),
            Some("resource capture worker produced no outcome for resource 'resource-b'")
        );
    }

    #[test]
    fn parallel_capture_is_worker_count_invariant() {
        let temp = tempfile::tempdir().unwrap();
        let (request, home) = pipeline_fixture(temp.path());
        let discovery = synthetic_discovery(vec![
            (
                "r0",
                vec![source_file(&home, "r0.json", b"{\"a\":1}", false)],
            ),
            (
                "r1",
                vec![source_file(&home, "r1.json", &vec![b'x'; 200_000], false)],
            ),
            (
                "r2",
                vec![
                    source_file(&home, "r2-a.json", b"first", false),
                    source_file(&home, "r2-b.json", &vec![b'y'; 40_000], false),
                ],
            ),
            (
                "r3",
                // Opaque user content with a token shape captures with a
                // warning instead of failing.
                vec![source_file(&home, "r3.json", b"note ghp_abcdef123", true)],
            ),
            (
                "r4",
                vec![source_file(
                    &home,
                    "r4.json",
                    "unicode \u{2713}".as_bytes(),
                    false,
                )],
            ),
        ]);
        let selected: BTreeSet<String> = ["r0", "r1", "r2", "r3", "r4"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let serial =
            capture_from_discovery_with_workers(registry(), &request, &discovery, &selected, 1)
                .unwrap();
        let parallel =
            capture_from_discovery_with_workers(registry(), &request, &discovery, &selected, 4)
                .unwrap();
        assert_eq!(serial, parallel);
        assert_eq!(serial.resources.len(), 5);
        assert_eq!(serial.files.len(), 6);
        assert!(serial
            .warnings
            .iter()
            .any(|warning| warning.contains("credential-shaped token")));
    }

    #[test]
    fn parallel_capture_reports_the_earliest_failing_resource() {
        let temp = tempfile::tempdir().unwrap();
        let (request, home) = pipeline_fixture(temp.path());
        let secret = b"-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n";
        let discovery = synthetic_discovery(vec![
            (
                "a-clean",
                vec![source_file(&home, "a.json", &vec![b'z'; 100_000], false)],
            ),
            (
                "b-secret",
                vec![source_file(&home, "b.json", secret, false)],
            ),
            (
                "c-secret",
                vec![source_file(&home, "c.json", secret, false)],
            ),
        ]);
        let selected: BTreeSet<String> = ["a-clean", "b-secret", "c-secret"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let serial =
            capture_from_discovery_with_workers(registry(), &request, &discovery, &selected, 1)
                .unwrap_err();
        let parallel =
            capture_from_discovery_with_workers(registry(), &request, &discovery, &selected, 4)
                .unwrap_err();
        // Both failing resources race; the merge always reports the
        // earliest resource ID with today's exact message.
        assert_eq!(serial, parallel);
        assert!(serial.contains("'b-secret'"), "{serial}");
        assert!(serial.contains("private key material"), "{serial}");
    }

    #[cfg(unix)]
    #[test]
    fn aggregate_capture_gate_fails_in_preflight_without_reading() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let (request, home) = pipeline_fixture(temp.path());
        // 33 sparse files at exactly the per-file limit total 528 MiB,
        // crossing the 512 MiB aggregate cap. Every file is unreadable, so
        // any attempt to read bytes would surface a permission error
        // instead of the aggregate message.
        let mut files = Vec::new();
        for index in 0..33 {
            let physical = home.join(format!("big-{index:02}.bin"));
            let file = fs::File::create(&physical).unwrap();
            file.set_len(MAX_FILE_BYTES).unwrap();
            drop(file);
            let mut permissions = fs::metadata(&physical).unwrap().permissions();
            permissions.set_mode(0o000);
            fs::set_permissions(&physical, permissions).unwrap();
            files.push(SourceFile {
                physical_path: physical,
                logical_path: format!("state/codex/big-{index:02}.bin"),
                opaque_content: false,
                derived_bytes: None,
            });
        }
        let discovery = synthetic_discovery(vec![("big", files)]);
        let selected: BTreeSet<String> = [String::from("big")].into_iter().collect();
        for workers in [1, 4] {
            let error = capture_from_discovery_with_workers(
                registry(),
                &request,
                &discovery,
                &selected,
                workers,
            )
            .unwrap_err();
            assert!(
                error.contains("selected resources exceed"),
                "workers {workers}: {error}"
            );
        }
    }
}
