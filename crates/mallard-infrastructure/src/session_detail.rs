//! Provider-neutral, local-only rich session detail orchestration.
//!
//! Provider adapters own transcript syntax. This module owns registered
//! project/profile resolution, local transcript ownership, bounded summary
//! and metadata-index caching, snapshot/cursor validation, Git checkpoint correlation,
//! response bounds, and DTOs that may cross the Tauri boundary.

use crate::app_settings::{
    conversation_file_limit_bytes, timeline_message_preview_bytes, timeline_page_content_bytes,
    timeline_tool_preview_bytes,
};
use crate::path_text_boundary::replace_sensitive_path_occurrences;
use crate::persistence::{
    persistence_guard, read_json_bounded, write_bytes_atomic, LocalStateRepository,
};
use crate::project_activity::{discover_github_repository_url, resolve_activity_project};
use crate::provider_capture::LocalSessionView;
use crate::providers::traits::{timeline_tool_detail_budgets, SessionMessageSettings};
use crate::providers::{
    AgentProviderRegistry, SessionActionContext, SessionActionDescriptor, SessionAgentPreview,
    SessionCheckpoint, SessionChildIdentity, SessionCount, SessionCountQuality,
    SessionDetailWarning, SessionEventCounts, SessionEventDetail, SessionFileSummary,
    SessionTimelineContext, SessionTimelineEvent, SessionTimelineEventKind, SessionTimelineIndex,
    SessionTimelinePageSlice, SessionTimelineQuery, SessionToolCategory, AGENT_RESOURCE_SEGMENT,
};
use crate::stored_session::StoredSessionTranscript;
use mallard_domain::{
    hex_lower, parse_conversation_resource_id, sha256_hex, LocalConversationBinding,
    LocalProjectId, NoticeSeverity, ProviderKey,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::UNIX_EPOCH;

const TIMELINE_CACHE_SCHEMA: u32 = 1;
const MAX_TIMELINE_CACHE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TIMELINE_CACHE_ENTRIES: usize = 50_000;
const MAX_TIMELINE_EVENTS: usize = 50_000;
const MAX_TIMELINE_PAGE_EVENTS: usize = 50;
const MAX_SESSION_DETAIL_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_TIMELINE_PAGE_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENT_DETAIL_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 512;
const MAX_WARNING_BYTES: usize = 2 * 1024;
const MAX_CHECKPOINT_COMMITS: usize = 10_000;
const MAX_CHECKPOINT_BRANCHES: usize = 10_000;
const MAX_PROJECT_MARKDOWN_PATH_BYTES: usize = 4 * 1024;
/// Raw-byte cap for inline Markdown image embedding. Base64 inflates the IPC
/// payload by 4/3, so the transported response stays under ~11 MiB.
const MAX_PROJECT_MARKDOWN_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
/// Raw-byte cap for an inline Markdown track or clip. The bytes cross the
/// boundary unencoded and play from a `blob:` URL, so the cap is what the page
/// holds in memory, the website's limit for the same files.
const MAX_PROJECT_MARKDOWN_MEDIA_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionDetailErrorCode {
    InvalidRequest,
    NotFound,
    NotLocal,
    OwnershipChanged,
    SnapshotChanged,
    ProviderError,
    ResponseTooLarge,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionDetailError {
    pub code: SessionDetailErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_snapshot_id: Option<String>,
}

impl SessionDetailError {
    fn new(code: SessionDetailErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            current_snapshot_id: None,
        }
    }

    fn snapshot_changed(current_snapshot_id: String) -> Self {
        Self {
            code: SessionDetailErrorCode::SnapshotChanged,
            message: "The session changed; refresh before loading more events.".to_string(),
            current_snapshot_id: Some(current_snapshot_id),
        }
    }
}

pub type SessionDetailResult<T> = Result<T, SessionDetailError>;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectSessionDetail {
    pub resource_id: String,
    pub provider: ProviderKey,
    pub provider_display_name: String,
    pub provider_session_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_sha: Option<String>,
    pub tokens: SessionCount,
    pub event_counts: SessionEventCounts,
    #[serde(default)]
    pub actions: Vec<SessionActionDescriptor>,
    /// Where this project's commits can be read on GitHub, when a GitHub remote
    /// is configured. The checkpoint marker links its sha through this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_repository_url: Option<String>,
    pub snapshot_id: String,
    #[serde(default)]
    pub warnings: Vec<SessionDetailWarning>,
    /// Set when this session is a child transcript: the parent to return to
    /// and the row the parent shows for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<SessionDetailAgent>,
    /// What the session started with: its first message's model and thinking
    /// level, when any message recorded them. The transcript marks changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SessionMessageSettings>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionDetailAgent {
    pub parent_resource_id: String,
    pub parent_title: String,
    pub preview: SessionAgentPreview,
}

/// A child resource id is its parent's with one `/agent/<id>` segment per
/// hop. This splits the root off and returns the hops in order.
pub fn split_agent_resource_id(resource_id: &str) -> (&str, Vec<&str>) {
    let mut parts = resource_id.split(AGENT_RESOURCE_SEGMENT);
    let root = parts.next().unwrap_or_default();
    (root, parts.collect())
}

/// The conversation resource a child page is reached through.
pub fn root_conversation_resource_id(resource_id: &str) -> &str {
    split_agent_resource_id(resource_id).0
}

fn valid_agent_hop(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectSessionTimelinePage {
    pub resource_id: String,
    pub snapshot_id: String,
    pub events: Vec<SessionTimelineEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub warnings: Vec<SessionDetailWarning>,
}

#[derive(Clone, Debug)]
pub struct ProjectSessionTimelineRequest {
    pub snapshot_id: String,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub included_event_kinds: BTreeSet<SessionTimelineEventKind>,
    pub included_tool_categories: BTreeSet<SessionToolCategory>,
    pub turn_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TimelineCacheEntry {
    schema: u32,
    resource_id: String,
    provider: ProviderKey,
    size: u64,
    modified_nanos: u64,
    timeline_version: u32,
    /// The tuning limits the entry was built under. Quality flags bake the
    /// preview budget in and admission bakes the transcript bound in, so an
    /// entry only serves while both still hold; the serde defaults make
    /// pre-stamp entries (zeros) never match instead of failing the parse.
    #[serde(default)]
    preview_budget_bytes: u64,
    #[serde(default)]
    conversation_limit_bytes: u64,
    summary: SessionFileSummary,
    index: SessionTimelineIndex,
}

/// True while `entry` was built under exactly the given limits.
fn entry_matches_limits(
    entry: &TimelineCacheEntry,
    preview_budget_bytes: u64,
    conversation_limit_bytes: u64,
) -> bool {
    entry.preview_budget_bytes == preview_budget_bytes
        && entry.conversation_limit_bytes == conversation_limit_bytes
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct TimelineCursor {
    schema: u32,
    provider: ProviderKey,
    resource_id: String,
    snapshot_id: String,
    filter_fingerprint: String,
    next_index: usize,
}

struct ResolvedLocalSession {
    provider: ProviderKey,
    resource_id: String,
    project_root: PathBuf,
    project_root_aliases: Vec<PathBuf>,
    provider_home: PathBuf,
    provider_session_id: String,
    transcript_path: PathBuf,
    display_name: String,
    file_size: u64,
    modified_nanos: u64,
    /// The binding of a root session. It says what a Pull installed, and
    /// so which agent calls name a child without a transcript.
    binding: Option<LocalConversationBinding>,
    /// Set on a child transcript: whose child, and which one. The parent it
    /// was reached through is resolved alongside it.
    child: Option<SessionChildIdentity>,
}

impl ResolvedLocalSession {
    fn timeline_context(&self) -> SessionTimelineContext<'_> {
        SessionTimelineContext {
            resource_id: &self.resource_id,
            provider_session_id: &self.provider_session_id,
            project_root: &self.project_root,
            project_root_aliases: &self.project_root_aliases,
            provider_home: &self.provider_home,
            child: self.child.as_ref(),
        }
    }
}

/// The session a request names, plus the parent a child was reached
/// through. A root session has no parent.
struct ResolvedSession {
    session: ResolvedLocalSession,
    parent: Option<ResolvedLocalSession>,
}

/// Resolve a root conversation or a child transcript addressed through it.
/// Every hop is proven the same way: the parent owns the child by the
/// provider's own rule, and the child file is a regular, unlinked file under
/// the provider home.
fn resolve_session(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    views: &[LocalSessionView],
) -> SessionDetailResult<ResolvedSession> {
    ensure_identifier("resource id", resource_id)?;
    let (root_id, hops) = split_agent_resource_id(resource_id);
    if hops.iter().any(|hop| !valid_agent_hop(hop)) {
        return Err(invalid_request("The agent resource ID is invalid."));
    }
    let mut session =
        resolve_local_session(repository, registry, local_project_id, root_id, views)?;
    let mut parent = None;
    for hop in hops {
        let adapter = registry.require(&session.provider).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::NotFound,
                "No registered provider can read this session.",
            )
        })?;
        let child = SessionChildIdentity {
            parent_provider_session_id: session.provider_session_id.clone(),
            provider_agent_id: hop.to_string(),
        };
        let path = adapter
            .locate_child_transcript(&session.provider_home, &session.transcript_path, &child)
            .map_err(|_| {
                SessionDetailError::new(
                    SessionDetailErrorCode::OwnershipChanged,
                    "The agent transcript could not be proven to belong to this session.",
                )
            })?
            .ok_or_else(|| {
                SessionDetailError::new(
                    SessionDetailErrorCode::NotLocal,
                    "This agent's transcript is not available locally.",
                )
            })?;
        let canonical_path = fs::canonicalize(&path).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::NotLocal,
                "This agent's transcript is not available locally.",
            )
        })?;
        if !canonical_path.starts_with(&session.provider_home) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The agent transcript escaped its provider profile.",
            ));
        }
        let metadata = fs::metadata(&canonical_path).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::NotLocal,
                "This agent's transcript is not available locally.",
            )
        })?;
        let resource_id = crate::providers::agent_resource_id(&session.resource_id, hop);
        let next = ResolvedLocalSession {
            provider: session.provider.clone(),
            resource_id,
            project_root: session.project_root.clone(),
            project_root_aliases: session.project_root_aliases.clone(),
            provider_home: session.provider_home.clone(),
            provider_session_id: hop.to_string(),
            transcript_path: canonical_path,
            display_name: hop.to_string(),
            file_size: metadata.len(),
            modified_nanos: modified_nanos(&metadata),
            binding: None,
            child: Some(child),
        };
        parent = Some(std::mem::replace(&mut session, next));
    }
    Ok(ResolvedSession { session, parent })
}

enum CheckpointSupport {
    Unsupported,
    Supported(Option<SessionCheckpoint>),
}

static RESOURCE_LOCKS: OnceLock<Mutex<BTreeMap<String, Weak<Mutex<()>>>>> = OnceLock::new();

fn resource_lock(key: &str) -> Arc<Mutex<()>> {
    let locks = RESOURCE_LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key.to_string(), Arc::downgrade(&lock));
    lock
}

fn modified_nanos(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn snapshot_id(session: &ResolvedLocalSession, timeline_version: u32) -> String {
    snapshot_id_with_limits(
        session,
        timeline_version,
        timeline_message_preview_bytes() as u64,
        conversation_file_limit_bytes(),
    )
}

/// What the binding says a Pull installed of the children. It decides
/// which agent cards the page shows.
fn installed_children(binding: Option<&LocalConversationBinding>) -> String {
    let Some(binding) = binding.filter(|binding| binding.installed_end.is_some()) else {
        return String::new();
    };
    let ids = |ids: &mut dyn Iterator<Item = &str>| ids.collect::<Vec<_>>().join("\u{1f}");
    format!(
        "{}\u{1e}{}\u{1e}{}",
        binding.installed_end.unwrap_or(0),
        ids(&mut binding
            .installed_subagents
            .values()
            .map(|child| child.provider_thread_id.as_str())),
        ids(&mut binding
            .installed_absent_subagents
            .iter()
            .map(String::as_str)),
    )
}

/// Limits and what a Pull installed join the snapshot identity, so changes
/// invalidate open cursors through the existing `SnapshotChanged` refresh path.
fn snapshot_id_with_limits(
    session: &ResolvedLocalSession,
    timeline_version: u32,
    preview_budget_bytes: u64,
    conversation_limit_bytes: u64,
) -> String {
    sha256_hex(
        format!(
            "session-detail-v1\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            session.provider,
            session.resource_id,
            session.file_size,
            session.modified_nanos,
            timeline_version,
            preview_budget_bytes,
            conversation_limit_bytes,
            installed_children(session.binding.as_ref()),
            session.transcript_path.display()
        )
        .as_bytes(),
    )
}

fn cache_key(session: &ResolvedLocalSession) -> String {
    sha256_hex(
        format!(
            "timeline-cache-v1\0{}\0{}\0{}",
            session.provider,
            session.resource_id,
            session.transcript_path.display()
        )
        .as_bytes(),
    )
}

fn invalid_request(message: impl Into<String>) -> SessionDetailError {
    SessionDetailError::new(SessionDetailErrorCode::InvalidRequest, message)
}

fn ensure_identifier(label: &str, value: &str) -> SessionDetailResult<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || value.chars().any(|character| character == '\0')
    {
        return Err(invalid_request(format!("{label} is invalid")));
    }
    Ok(())
}

fn safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn canonical_project_markdown_path(
    project_root: &Path,
    relative_path: &str,
) -> SessionDetailResult<PathBuf> {
    if relative_path.is_empty()
        || relative_path.len() > MAX_PROJECT_MARKDOWN_PATH_BYTES
        || relative_path.contains('\0')
        || !safe_relative_path(relative_path)
    {
        return Err(invalid_request(
            "Only a clean relative path inside the project can be opened.",
        ));
    }
    let canonical_root = fs::canonicalize(project_root).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The registered project folder is no longer readable.",
        )
    })?;
    let components = Path::new(relative_path).components().collect::<Vec<_>>();
    let mut target = canonical_root.clone();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(segment) = component else {
            return Err(invalid_request(
                "Only a clean relative path inside the project can be opened.",
            ));
        };
        target.push(segment);
        let metadata = fs::symlink_metadata(&target).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::NotFound,
                "The linked project path no longer exists.",
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_request(
                "Linked project paths cannot traverse symbolic links.",
            ));
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::NotFound,
                "The linked project path no longer exists.",
            ));
        }
    }
    let canonical_target = fs::canonicalize(&target).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "The linked project path no longer exists.",
        )
    })?;
    if !canonical_target.starts_with(&canonical_root) || canonical_target == canonical_root {
        return Err(invalid_request(
            "Only paths inside the registered project can be opened.",
        ));
    }
    let metadata = fs::symlink_metadata(&canonical_target).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "The linked project path no longer exists.",
        )
    })?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(invalid_request(
            "Only project files and folders can be opened.",
        ));
    }
    Ok(canonical_target)
}

/// Resolve one sanitized Markdown destination against the current active
/// binding. The returned path is canonical, no-follow, and remains strictly
/// below the registered project root.
pub fn resolve_project_markdown_path(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    relative_path: &str,
) -> SessionDetailResult<PathBuf> {
    let resolved = resolve_activity_project(repository, local_project_id).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The project binding changed; open Project Settings.",
        )
    })?;
    canonical_project_markdown_path(&resolved.project_root, relative_path)
}

/// One embeddable project image crossing the Tauri boundary as a bounded
/// base64 payload. Only clean relative descendants of the registered project
/// root are ever served; the webview renders the result as a `data:` URI, so
/// no filesystem protocol is exposed to Markdown content.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectMarkdownImage {
    pub mime_type: String,
    pub base64_data: String,
}

/// Read one Markdown-referenced project image for inline embedding: the same
/// canonical/no-follow descendant validation links use, then a regular-file
/// check, a size cap read before the bytes, and a magic-byte sniff — the
/// declared extension is never trusted.
pub fn read_project_markdown_image(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    relative_path: &str,
) -> SessionDetailResult<ProjectMarkdownImage> {
    let path = resolve_project_markdown_path(repository, local_project_id, relative_path)?;
    bounded_project_markdown_image(&path)
}

/// The bounded read half of [`read_project_markdown_image`], over an
/// already-validated canonical project path.
fn bounded_project_markdown_image(path: &Path) -> SessionDetailResult<ProjectMarkdownImage> {
    let bytes = bounded_project_markdown_bytes(path, MAX_PROJECT_MARKDOWN_IMAGE_BYTES, "image")?;
    let mime_type = sniff_markdown_image_mime(&bytes).ok_or_else(|| {
        invalid_request("Only GIF, PNG, JPEG, and WebP project images can be embedded.")
    })?;
    Ok(ProjectMarkdownImage {
        mime_type: mime_type.to_string(),
        base64_data: aws_smithy_types::base64::encode(&bytes),
    })
}

/// Read one Markdown-referenced project track or clip for an inline player:
/// the same canonical/no-follow validation, regular-file check and size cap
/// read before the bytes as images, then a magic-byte whitelist of audio and
/// video containers. The raw bytes go to the webview, which plays them from a
/// `blob:` URL with the type its extension names.
pub fn read_project_markdown_media(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    relative_path: &str,
) -> SessionDetailResult<Vec<u8>> {
    let path = resolve_project_markdown_path(repository, local_project_id, relative_path)?;
    bounded_project_markdown_media(&path)
}

/// The bounded read half of [`read_project_markdown_media`], over an
/// already-validated canonical project path.
fn bounded_project_markdown_media(path: &Path) -> SessionDetailResult<Vec<u8>> {
    let bytes =
        bounded_project_markdown_bytes(path, MAX_PROJECT_MARKDOWN_MEDIA_BYTES, "media file")?;
    if !is_markdown_media(&bytes) {
        return Err(invalid_request(
            "Only WAV, MP3, AAC, FLAC, Ogg, MP4, QuickTime, and WebM project media can play.",
        ));
    }
    Ok(bytes)
}

/// One regular project file, read whole under `cap`: the size is checked from
/// the metadata before the read and again on the bytes, so a file growing
/// between the two never reaches the webview past the cap.
fn bounded_project_markdown_bytes(
    path: &Path,
    cap: u64,
    noun: &str,
) -> SessionDetailResult<Vec<u8>> {
    let gone = || {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            format!("The linked project {noun} no longer exists."),
        )
    };
    let too_large = || {
        SessionDetailError::new(
            SessionDetailErrorCode::ResponseTooLarge,
            format!("The linked project {noun} exceeds Mallard's embedding limit."),
        )
    };
    let metadata = fs::symlink_metadata(path).map_err(|_| gone())?;
    if !metadata.is_file() {
        return Err(invalid_request(format!(
            "Only project {noun}s can be embedded."
        )));
    }
    if metadata.len() > cap {
        return Err(too_large());
    }
    let bytes = fs::read(path).map_err(|_| gone())?;
    if bytes.len() as u64 > cap {
        return Err(too_large());
    }
    Ok(bytes)
}

/// The audio and video containers an inline player takes, by magic bytes.
/// Whether the webview can decode a given codec is the page's question; this
/// only keeps anything that is not a media container out of a player.
fn is_markdown_media(bytes: &[u8]) -> bool {
    let riff_wave = bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE";
    // ISO base media (MP4, M4A, MOV): a `ftyp` box first.
    let iso_media = bytes.len() >= 8 && &bytes[4..8] == b"ftyp";
    // MPEG audio frames and ADTS AAC share the 11-bit sync; either is audio.
    let mpeg_sync = bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0;
    riff_wave
        || iso_media
        || mpeg_sync
        || bytes.starts_with(b"ID3")
        || bytes.starts_with(b"fLaC")
        || bytes.starts_with(b"OggS")
        // EBML: WebM and Matroska.
        || bytes.starts_with(b"\x1A\x45\xDF\xA3")
}

/// The supported embeddable formats by magic bytes. SVG is deliberately
/// excluded: it is the one candidate whose bytes are an active document
/// rather than pixels.
fn sniff_markdown_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return Some("image/jpeg");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// The two kinds that wear a model stamp: an input and a final reply.
fn is_message_kind(kind: SessionTimelineEventKind) -> bool {
    matches!(
        kind,
        SessionTimelineEventKind::Prompt | SessionTimelineEventKind::Response
    )
}

fn validate_index(index: &SessionTimelineIndex, file_size: u64) -> SessionDetailResult<()> {
    let file_size = if index.sources.is_empty() {
        file_size
    } else {
        if index.sources.len() > crate::codex_rollout::MAX_CODEX_FORK_DEPTH + 1 {
            return Err(invalid_request(
                "Timeline source count exceeds the history limit.",
            ));
        }
        let mut total = 0u64;
        let mut paths = BTreeSet::new();
        for source in &index.sources {
            if !safe_relative_path(&source.relative_path)
                || source.relative_path.len() > 4096
                || !paths.insert(&source.relative_path)
            {
                return Err(invalid_request("Timeline source path is invalid."));
            }
            total = total
                .checked_add(source.length)
                .filter(|total| *total <= crate::provider_capture::MAX_CAPTURE_BYTES)
                .ok_or_else(|| {
                    invalid_request("Timeline source bytes exceed the history limit.")
                })?;
        }
        if index
            .sources
            .last()
            .is_some_and(|source| source.length > file_size)
        {
            return Err(invalid_request("Timeline leaf exceeds the current file."));
        }
        total
    };
    if index.events.len() > MAX_TIMELINE_EVENTS {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The provider timeline exceeded Mallard's event limit.",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut ids_by_turn: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut calls_by_turn: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut summaries_by_turn = BTreeSet::new();
    let mut previous_ordinal = None;
    for event in &index.events {
        ensure_identifier("event id", &event.event_id)?;
        if event.kind == SessionTimelineEventKind::Checkpoint {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "Provider adapters cannot index Git checkpoint events.",
            ));
        }
        if !ids.insert(event.event_id.as_str()) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned duplicate timeline event IDs.",
            ));
        }
        if previous_ordinal.is_some_and(|ordinal| event.ordinal <= ordinal) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned an invalid timeline order.",
            ));
        }
        previous_ordinal = Some(event.ordinal);
        if event
            .source
            .offset
            .checked_add(event.source.length)
            .is_none_or(|end| end > file_size)
        {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned an out-of-bounds timeline record.",
            ));
        }
        if let Some(turn_id) = &event.turn_id {
            ensure_identifier("turn id", turn_id)?;
        }
        if !event.settings.is_valid()
            || (!is_message_kind(event.kind)
                && event.kind != SessionTimelineEventKind::IntermediateStep
                && event.settings != SessionMessageSettings::none())
        {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned invalid message settings.",
            ));
        }
        if let Some(summary) = &event.turn_tool_summary {
            if event.turn_id.as_deref() != Some(summary.turn_id.as_str())
                || !summaries_by_turn.insert(summary.turn_id.as_str())
            {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned an invalid per-turn tool summary.",
                ));
            }
        }
        if let Some(parent) = &event.parent_event_id {
            ensure_identifier("parent event id", parent)?;
        }
        let is_call = matches!(
            event.kind,
            SessionTimelineEventKind::ToolCall | SessionTimelineEventKind::AgentCall
        );
        if event.agent.is_some() != (event.kind == SessionTimelineEventKind::AgentCall) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned agent metadata on an invalid event.",
            ));
        }
        if let Some(agent) = &event.agent {
            if let Some(id) = &agent.provider_agent_id {
                ensure_identifier("agent id", id)?;
            }
            if agent
                .transcript
                .as_ref()
                .is_some_and(|pointer| pointer.len() > 1024 || !safe_relative_path(pointer))
            {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned an unsafe agent transcript pointer.",
                ));
            }
        }
        if let Some(tool) = &event.tool {
            if !is_call || event.turn_id.is_none() {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned tool metadata on an invalid event.",
                ));
            }
            ensure_identifier("tool call id", &tool.provider_call_id)?;
            ensure_identifier("tool name", &tool.native_name)?;
            if tool.result_source.is_some_and(|span| {
                span.offset
                    .checked_add(span.length)
                    .is_none_or(|end| end > file_size)
            }) {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned an out-of-bounds tool result.",
                ));
            }
            if tool
                .affected_paths
                .iter()
                .any(|path| path.len() > 1024 || !safe_relative_path(path))
            {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned an unsafe affected path.",
                ));
            }
            let turn_id = event.turn_id.as_deref().expect("checked above");
            if !ids_by_turn
                .entry(turn_id)
                .or_default()
                .insert(tool.provider_call_id.as_str())
            {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned a duplicate tool call for one turn.",
                ));
            }
            // The per-turn summary counts tool calls; a delegation is not one.
            if event.kind == SessionTimelineEventKind::ToolCall {
                calls_by_turn
                    .entry(turn_id)
                    .or_default()
                    .insert(tool.provider_call_id.as_str());
            }
        } else if is_call {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned a tool event without tool metadata.",
            ));
        }
    }
    let agent_calls = index
        .events
        .iter()
        .filter(|event| event.kind == SessionTimelineEventKind::AgentCall)
        .count() as u64;
    if index
        .counts
        .agent_calls
        .value
        .is_some_and(|counted| counted != agent_calls)
    {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The agent call count did not match the indexed agent calls.",
        ));
    }
    for event in &index.events {
        let Some(summary) = &event.turn_tool_summary else {
            continue;
        };
        let known = calls_by_turn
            .get(summary.turn_id.as_str())
            .map(|calls| calls.len() as u64)
            .unwrap_or(0);
        if summary.tool_calls.value != Some(known) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "A per-turn tool summary did not match its de-duplicated calls.",
            ));
        }
    }
    if calls_by_turn
        .keys()
        .any(|turn_id| !summaries_by_turn.contains(turn_id))
    {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "A turn with tool calls was missing its de-duplicated tool summary.",
        ));
    }
    if let (Some(total), Some(file_edits), Some(shell_calls), Some(reads), Some(other_tools)) = (
        index.counts.tool_calls.value,
        index.counts.file_edits.value,
        index.counts.shell_calls.value,
        index.counts.reads.value,
        index.counts.other_tools.value,
    ) {
        if file_edits
            .checked_add(shell_calls)
            .and_then(|value| value.checked_add(reads))
            .and_then(|value| value.checked_add(other_tools))
            != Some(total)
        {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "Tool category counts did not partition the unique tool total.",
            ));
        }
    }
    if index
        .warnings
        .iter()
        .any(|warning| warning.code.len() > 128 || warning.message.len() > MAX_WARNING_BYTES)
    {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The provider returned an invalid timeline warning.",
        ));
    }
    Ok(())
}

fn validate_page_slice(
    slice: &SessionTimelinePageSlice,
    index: &SessionTimelineIndex,
    limit: usize,
) -> SessionDetailResult<()> {
    if slice.events.len() > limit {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The provider returned too many timeline events.",
        ));
    }
    if slice
        .next_index
        .is_some_and(|next| next > index.events.len())
    {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The provider returned an invalid timeline cursor.",
        ));
    }
    let mut page_content_bytes = 0usize;
    for event in &slice.events {
        ensure_identifier("event id", &event.event_id)?;
        if event.kind == SessionTimelineEventKind::Checkpoint || event.checkpoint.is_some() {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "Provider adapters cannot create Git checkpoint events.",
            ));
        }
        if event
            .text
            .as_ref()
            .is_some_and(|text| text.len() > timeline_message_preview_bytes())
        {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned an oversized message preview.",
            ));
        }
        if let Some(tool) = &event.tool {
            if tool
                .argument_preview
                .as_ref()
                .is_some_and(|text| text.len() > timeline_tool_preview_bytes())
                || tool
                    .result_preview
                    .as_ref()
                    .is_some_and(|text| text.len() > timeline_tool_preview_bytes())
                || tool
                    .affected_paths
                    .iter()
                    .any(|path| path.len() > 1024 || !safe_relative_path(path))
            {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned an oversized or unsafe tool preview.",
                ));
            }
        }
        if event.agent.is_some() != (event.kind == SessionTimelineEventKind::AgentCall) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned agent metadata on an invalid event.",
            ));
        }
        if let Some(settings) = &event.settings {
            if !is_message_kind(event.kind) || !settings.is_valid() {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned message settings on an invalid event.",
                ));
            }
        }
        if let Some(agent) = &event.agent {
            if agent.label.is_empty()
                || agent.label.len() > 1024
                || agent
                    .brief
                    .as_ref()
                    .is_some_and(|text| text.len() > timeline_tool_preview_bytes())
                || agent
                    .report
                    .as_ref()
                    .is_some_and(|text| text.len() > timeline_tool_preview_bytes())
                || agent
                    .transcript_resource_id
                    .as_ref()
                    .is_some_and(|id| ensure_identifier("agent resource id", id).is_err())
            {
                return Err(SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider returned an oversized or unsafe agent preview.",
                ));
            }
        }
        page_content_bytes = page_content_bytes
            .saturating_add(event.text.as_ref().map_or(0, String::len))
            .saturating_add(
                event
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.argument_preview.as_ref())
                    .map_or(0, String::len),
            )
            .saturating_add(
                event
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.result_preview.as_ref())
                    .map_or(0, String::len),
            )
            .saturating_add(
                event
                    .agent
                    .as_ref()
                    .map_or(0, crate::providers::claude::history::agent_preview_bytes),
            );
        if page_content_bytes > timeline_page_content_bytes() {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned an oversized timeline page.",
            ));
        }
    }
    Ok(())
}

fn resolve_local_session(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    views: &[LocalSessionView],
) -> SessionDetailResult<ResolvedLocalSession> {
    ensure_identifier("resource id", resource_id)?;
    let (provider, thread_id) = parse_conversation_resource_id(resource_id)
        .map_err(|_| invalid_request("The session resource ID is invalid."))?;
    let resolved = resolve_activity_project(repository, local_project_id).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The project or its provider profile changed; open Project Settings.",
        )
    })?;
    let adapter = registry.require(&provider).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "No registered provider can read this session.",
        )
    })?;
    let home = resolved.homes.get(&provider).ok_or_else(|| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotLocal,
            "This session is not materialized in a readable local provider profile.",
        )
    })?;
    let bindings = repository.load_conversation_bindings().map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "Mallard could not revalidate this session's local ownership.",
        )
    })?;
    let local_binding = bindings.bindings.iter().find(|binding| {
        binding.local_project_id == resolved.project_id
            && binding.thread_id == thread_id
            && binding.provider_profile_id == home.profile_id
    });
    let session_for_thread = local_binding.and_then(|binding| binding.provider_thread_id.clone());
    let mut matches = Vec::new();
    for session in views.iter().filter(|session| session.provider == provider) {
        let identity_matches = session_for_thread
            .as_ref()
            .map(|expected| expected == &session.provider_thread_id)
            .map(Ok)
            .unwrap_or_else(|| {
                crate::project_activity::resolve_local_provider_alias_thread(
                    repository,
                    &resolved,
                    &provider,
                    &session.provider_thread_id,
                    &session.source_path,
                )
                .map(|derived| derived.as_ref() == Some(&thread_id))
                .map_err(|_| {
                    SessionDetailError::new(
                        SessionDetailErrorCode::OwnershipChanged,
                        "Mallard could not revalidate this session's local ownership.",
                    )
                })
            })?;
        if !identity_matches {
            continue;
        }
        if ensure_identifier("provider session id", &session.provider_thread_id).is_err() {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned an invalid native session ID.",
            ));
        }
        if session.descriptor.blocked_reason.is_some() {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The provider blocked access to this local session.",
            ));
        }
        let link_metadata = fs::symlink_metadata(&session.source_path).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The local session file is no longer readable.",
            )
        })?;
        if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The local session is not a regular no-follow file.",
            ));
        }
        let canonical_path = fs::canonicalize(&session.source_path).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The local session path changed during validation.",
            )
        })?;
        if !canonical_path.starts_with(&home.home) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The local session escaped its registered provider profile.",
            ));
        }
        let native = adapter.scan_metadata(&canonical_path).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider could not revalidate this session's identity.",
            )
        })?;
        if native.session_id != session.provider_thread_id || native.is_internal_subagent {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The local session identity changed since discovery.",
            ));
        }
        if !cwd_belongs_to_project(&native.authorized_home, &resolved.project_root) {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The session no longer belongs to the selected project.",
            ));
        }
        let metadata = fs::metadata(&canonical_path).map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::OwnershipChanged,
                "The local session changed during validation.",
            )
        })?;
        matches.push(ResolvedLocalSession {
            provider: provider.clone(),
            resource_id: resource_id.to_string(),
            project_root: resolved.project_root.clone(),
            project_root_aliases: project_root_aliases(&resolved),
            provider_home: home.home.clone(),
            provider_session_id: session.provider_thread_id.clone(),
            transcript_path: canonical_path,
            display_name: session.descriptor.display_name.clone(),
            file_size: metadata.len(),
            modified_nanos: modified_nanos(&metadata),
            binding: local_binding.cloned(),
            child: None,
        });
    }
    match matches.len() {
        0 => Err(SessionDetailError::new(
            SessionDetailErrorCode::NotLocal,
            "This session is not materialized locally for the selected project.",
        )),
        1 => Ok(matches.remove(0)),
        _ => Err(SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "More than one local transcript matched this session; access was refused.",
        )),
    }
}

pub(crate) fn project_root_aliases(
    resolved: &crate::project_activity::ResolvedActivityProject,
) -> Vec<PathBuf> {
    let mut spellings = vec![
        PathBuf::from(&resolved.binding.project_root),
        PathBuf::from(&resolved.binding.canonical_project_root),
        resolved.project_root.clone(),
    ];
    spellings.retain(|spelling| !spelling.as_os_str().is_empty());
    spellings.sort_by(|left, right| {
        right
            .as_os_str()
            .len()
            .cmp(&left.as_os_str().len())
            .then_with(|| left.as_os_str().cmp(right.as_os_str()))
    });
    spellings.dedup();
    spellings
}

fn cwd_belongs_to_project(cwd: &Path, project_root: &Path) -> bool {
    if !cwd.is_absolute()
        || cwd
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return false;
    }
    fs::canonicalize(cwd)
        .map(|resolved| resolved == project_root || resolved.starts_with(project_root))
        .unwrap_or(false)
}

fn load_timeline_cache(
    repository: &LocalStateRepository,
    key: &str,
) -> (Option<TimelineCacheEntry>, Vec<SessionDetailWarning>) {
    let path = repository
        .root()
        .join("session_timeline_cache")
        .join(format!("{key}.json"));
    match read_json_bounded::<TimelineCacheEntry>(repository.root(), &path, MAX_TIMELINE_CACHE_BYTES) {
        Ok(Some(entry)) if entry.schema == TIMELINE_CACHE_SCHEMA => (Some(entry), Vec::new()),
        Ok(_) => (None, Vec::new()),
        Err(_) => (
            None,
            vec![SessionDetailWarning {
                code: "timeline_cache_ignored".to_string(),
                message: "Mallard rebuilt its local timeline cache because the saved cache was outdated or unreadable."
                    .to_string(),
                severity: NoticeSeverity::Info,
            }],
        ),
    }
}

fn write_timeline_cache(
    repository: &LocalStateRepository,
    key: &str,
    entry: &TimelineCacheEntry,
    max_bytes: u64,
) -> Result<(), String> {
    // Measure the exact bytes that will be written before evicting anything.
    let bytes = serde_json::to_vec(entry).map_err(|error| error.to_string())?;
    let incoming_bytes = bytes.len() as u64;
    if incoming_bytes > max_bytes {
        return Err("The session timeline cache entry exceeds the byte limit.".to_string());
    }
    // Serialize eviction and publication across sessions and processes.
    let _guard = persistence_guard(repository.root())?;
    let directory = repository.root().join("session_timeline_cache");
    repository.ensure_directory(&directory)?;
    let destination = directory.join(format!("{key}.json"));
    let mut entries = Vec::new();
    let mut total_bytes = incoming_bytes;
    for (position, item) in fs::read_dir(&directory)
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if position >= MAX_TIMELINE_CACHE_ENTRIES {
            return Err("The session timeline cache exceeds the entry limit.".to_string());
        }
        let item = item.map_err(|error| error.to_string())?;
        let name = item.file_name();
        let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
            continue;
        };
        if stem.len() != 64 || !stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let path = item.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.file_type().is_file() {
            return Err("The session timeline cache contains a non-regular file.".to_string());
        }
        // A replacement contributes only its new size to the budget.
        if path == destination {
            continue;
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        entries.push((modified_nanos(&metadata), path, metadata.len()));
    }
    entries.sort_unstable();
    let mut entry_count = entries.len() + 1;
    for (_, path, size) in entries {
        if total_bytes <= max_bytes && entry_count <= MAX_TIMELINE_CACHE_ENTRIES {
            break;
        }
        fs::remove_file(path).map_err(|error| error.to_string())?;
        total_bytes = total_bytes.saturating_sub(size);
        entry_count -= 1;
    }
    write_bytes_atomic(repository.root(), &destination, &bytes, max_bytes)
}

fn refresh_file_state(session: &mut ResolvedLocalSession) -> SessionDetailResult<()> {
    let link_metadata = fs::symlink_metadata(&session.transcript_path).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The local session file is no longer readable.",
        )
    })?;
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The local session is no longer a regular no-follow file.",
        ));
    }
    let canonical = fs::canonicalize(&session.transcript_path).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The local session path changed during validation.",
        )
    })?;
    if canonical != session.transcript_path || !canonical.starts_with(&session.provider_home) {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The local session path changed or escaped its provider profile.",
        ));
    }
    let metadata = fs::metadata(&canonical).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The local session changed during validation.",
        )
    })?;
    session.file_size = metadata.len();
    session.modified_nanos = modified_nanos(&metadata);
    Ok(())
}

/// Take agent calls out of an index. A call may hold the tool summary of
/// its turn; the summary goes to the entry that would have held it.
fn leave_out_agent_calls(index: &mut SessionTimelineIndex, calls: &BTreeSet<String>) {
    let before = index.events.len();
    let mut summaries = Vec::new();
    index.events.retain_mut(|entry| {
        let left_out =
            entry.kind == SessionTimelineEventKind::AgentCall && calls.contains(&entry.event_id);
        if left_out {
            summaries.extend(entry.turn_tool_summary.take());
        }
        !left_out
    });
    for summary in summaries {
        let of_turn = |entry: &crate::providers::SessionTimelineIndexEntry| {
            entry.turn_id.as_deref() == Some(summary.turn_id.as_str())
        };
        let position = index
            .events
            .iter()
            .position(|entry| of_turn(entry) && entry.kind != SessionTimelineEventKind::ToolCall)
            .or_else(|| index.events.iter().position(of_turn));
        if let Some(position) = position {
            index.events[position].turn_tool_summary = Some(summary);
        }
    }
    let left_out = (before - index.events.len()) as u64;
    if let Some(shown) = index.counts.agent_calls.value.as_mut() {
        *shown = shown.saturating_sub(left_out);
    }
}

/// The index as the page shows it: without the agent calls whose children
/// came with a Pull without their transcripts. The saved index holds every
/// call, because a later Pull may bring the transcripts.
fn load_or_build_index(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    session: &mut ResolvedLocalSession,
) -> SessionDetailResult<(
    SessionTimelineIndex,
    SessionFileSummary,
    String,
    Vec<SessionDetailWarning>,
)> {
    let (mut index, summary, snapshot_id, warnings) =
        load_or_build_whole_index(repository, registry, session)?;
    if let Some(binding) = &session.binding {
        let calls = registry
            .require(&session.provider)
            .map(|adapter| adapter.agent_calls_without_transcript(&index, binding))
            .unwrap_or_default();
        leave_out_agent_calls(&mut index, &calls);
    }
    Ok((index, summary, snapshot_id, warnings))
}

fn load_or_build_whole_index(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    session: &mut ResolvedLocalSession,
) -> SessionDetailResult<(
    SessionTimelineIndex,
    SessionFileSummary,
    String,
    Vec<SessionDetailWarning>,
)> {
    let adapter = registry.require(&session.provider).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "No registered provider can read this session.",
        )
    })?;
    let timeline_version = adapter.timeline_version();
    let lock = resource_lock(&session.resource_id);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = cache_key(session);
    let (mut cached_entry, mut warnings) = load_timeline_cache(repository, &key);

    for attempt in 0..2 {
        refresh_file_state(session)?;
        let expected_size = session.file_size;
        let expected_nanos = session.modified_nanos;
        let current_snapshot_id = snapshot_id(session, timeline_version);
        if let Some(entry) = cached_entry.as_ref().filter(|entry| {
            entry.resource_id == session.resource_id
                && entry.provider == session.provider
                && entry.size == expected_size
                && entry.modified_nanos == expected_nanos
                && entry.timeline_version == timeline_version
                && entry_matches_limits(
                    entry,
                    timeline_message_preview_bytes() as u64,
                    conversation_file_limit_bytes(),
                )
                && entry.summary.session_id == session.provider_session_id
                && entry.summary.is_internal_subagent == session.child.is_some()
                && entry.index.parser_version == timeline_version
        }) {
            if validate_index(&entry.index, expected_size).is_ok() {
                let index = entry.index.clone();
                let summary = entry.summary.clone();
                refresh_file_state(session)?;
                if session.file_size == expected_size && session.modified_nanos == expected_nanos {
                    return Ok((index, summary, current_snapshot_id, warnings));
                }
                if attempt == 0 {
                    continue;
                }
            }
            cached_entry = None;
        }

        let index = adapter
            .build_timeline_index(&session.transcript_path, &session.timeline_context())
            .map_err(|_| {
                SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider could not build a safe timeline for this session.",
                )
            })?;
        if index.parser_version != timeline_version {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned a timeline with an unexpected parser version.",
            ));
        }
        validate_index(&index, expected_size)?;
        let summary = summarize_session(registry, session, &index)?;
        refresh_file_state(session)?;
        if session.file_size != expected_size || session.modified_nanos != expected_nanos {
            if attempt == 0 {
                continue;
            }
            return Err(SessionDetailError::snapshot_changed(snapshot_id(
                session,
                timeline_version,
            )));
        }
        let fresh_entry = TimelineCacheEntry {
            schema: TIMELINE_CACHE_SCHEMA,
            resource_id: session.resource_id.clone(),
            provider: session.provider.clone(),
            size: expected_size,
            modified_nanos: expected_nanos,
            timeline_version,
            preview_budget_bytes: timeline_message_preview_bytes() as u64,
            conversation_limit_bytes: conversation_file_limit_bytes(),
            summary: summary.clone(),
            index: index.clone(),
        };
        if write_timeline_cache(repository, &key, &fresh_entry, MAX_TIMELINE_CACHE_BYTES).is_err() {
            warnings.push(SessionDetailWarning {
                code: "timeline_cache_write_failed".to_string(),
                message: "Mallard could not update the local timeline cache.".to_string(),
                severity: NoticeSeverity::Info,
            });
        }
        return Ok((index, summary, current_snapshot_id, warnings));
    }
    Err(SessionDetailError::new(
        SessionDetailErrorCode::ProviderError,
        "The session changed repeatedly while Mallard indexed it.",
    ))
}

fn provider_display_name(registry: &AgentProviderRegistry, provider: &ProviderKey) -> String {
    registry
        .get(provider)
        .map(|adapter| adapter.descriptor().display_name)
        .unwrap_or_else(|| provider.to_string())
}

fn checkpoint_event_id(checkpoint: &SessionCheckpoint) -> String {
    format!("checkpoint:{}", checkpoint.sha)
}

fn checkpoint_event(
    index: &SessionTimelineIndex,
    checkpoint: SessionCheckpoint,
) -> SessionTimelineEvent {
    SessionTimelineEvent {
        event_id: checkpoint_event_id(&checkpoint),
        ordinal: index
            .events
            .last()
            .map(|event| event.ordinal.saturating_add(1))
            .unwrap_or(0),
        timestamp: Some(checkpoint.committed_at),
        kind: SessionTimelineEventKind::Checkpoint,
        quality: SessionCountQuality::Inferred,
        turn_id: None,
        turn_tool_summary: None,
        parent_event_id: None,
        text: None,
        tool: None,
        checkpoint: Some(checkpoint),
        agent: None,
        settings: None,
    }
}

/// The session's one commit, placed the way Activity places it: the closest
/// first-parent commit at or after the last human input (or the end when no
/// input was recorded), the older of two in the same second.
fn checkpoint_support(
    project_root: &Path,
    branch: Option<&str>,
    placed_at: Option<u64>,
) -> SessionDetailResult<CheckpointSupport> {
    let inside = git_output(project_root, &["rev-parse", "--is-inside-work-tree"])
        .ok()
        .is_some_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        });
    if !inside {
        return Ok(CheckpointSupport::Unsupported);
    }
    let branch_count = format!("--count={MAX_CHECKPOINT_BRANCHES}");
    let branches = git_output(
        project_root,
        &[
            "for-each-ref",
            &branch_count,
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )
    .map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "Git could not enumerate checkpoint branches.",
        )
    })?;
    if !branches.status.success() {
        return Ok(CheckpointSupport::Supported(None));
    }
    let available = String::from_utf8_lossy(&branches.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    let selected = match branch.filter(|name| available.contains(*name)) {
        Some(branch) => Some(branch.to_string()),
        None if branch.is_some() => None,
        None => git_output(project_root, &["symbolic-ref", "--short", "-q", "HEAD"])
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|name| available.contains(name)),
    };
    let Some(selected) = selected else {
        return Ok(CheckpointSupport::Supported(None));
    };
    let count = MAX_CHECKPOINT_COMMITS.to_string();
    let branch_ref = format!("refs/heads/{selected}");
    let output = git_output(
        project_root,
        &[
            "log",
            "--first-parent",
            "--date-order",
            "--format=%H%x1f%ct%x1f%<(500,trunc)%s",
            "-n",
            &count,
            &branch_ref,
            "--",
        ],
    )
    .map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "Git could not read checkpoint history.",
        )
    })?;
    if !output.status.success() {
        return Ok(CheckpointSupport::Supported(None));
    }
    let placed_at = placed_at.unwrap_or(0);
    let checkpoint = parse_git_commits(&output, &selected)
        .into_iter()
        .rev()
        .filter(|commit| commit.committed_at >= placed_at)
        .min_by_key(|commit| commit.committed_at);
    Ok(CheckpointSupport::Supported(checkpoint))
}

fn parse_git_commits(output: &Output, branch: &str) -> Vec<SessionCheckpoint> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\u{1f}');
            let sha = fields.next()?;
            let committed_at = fields.next()?.parse::<u64>().ok()?;
            let subject = fields.next().unwrap_or_default();
            valid_full_sha(sha).then(|| SessionCheckpoint {
                sha: sha.to_ascii_lowercase(),
                short_sha: sha.chars().take(12).collect(),
                subject: subject.chars().take(500).collect(),
                committed_at,
                branch: branch.to_string(),
            })
        })
        .collect()
}

fn valid_full_sha(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_output(root: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| error.to_string())
}

fn with_checkpoint_count(
    mut counts: SessionEventCounts,
    checkpoint: &CheckpointSupport,
) -> SessionEventCounts {
    counts.checkpoints = match checkpoint {
        CheckpointSupport::Unsupported => SessionCount::unsupported(
            "Git checkpoints are unavailable because this project is not a Git worktree.",
        ),
        CheckpointSupport::Supported(value) => SessionCount {
            value: Some(u64::from(value.is_some())),
            quality: SessionCountQuality::Inferred,
            explanation: Some(
                "Checkpoints are inferred from the first successor commit on the provider-recorded branch."
                    .to_string(),
            ),
        },
    };
    counts
}

fn append_unique_warnings(
    target: &mut Vec<SessionDetailWarning>,
    warnings: impl IntoIterator<Item = SessionDetailWarning>,
) {
    for warning in warnings {
        if !target
            .iter()
            .any(|existing| existing.code == warning.code && existing.message == warning.message)
        {
            target.push(warning);
        }
    }
}

/// Preserve the provider's typed reason for a lower-bound timeline. The
/// generic `truncated` bit covers malformed, oversized, record-limited, and
/// event-limited inputs, so it cannot safely be translated into an event-limit
/// warning on its own.
fn timeline_warnings(
    index: &SessionTimelineIndex,
    cache_warnings: Vec<SessionDetailWarning>,
) -> Vec<SessionDetailWarning> {
    let mut warnings = index.warnings.clone();
    append_unique_warnings(&mut warnings, cache_warnings);
    warnings
}

fn encode_response_bound<T: Serialize>(value: &T, max_bytes: usize) -> SessionDetailResult<String> {
    let encoded = serde_json::to_string(value).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "Mallard could not encode the normalized session response.",
        )
    })?;
    if encoded.len() > max_bytes {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ResponseTooLarge,
            "The normalized session response exceeded Mallard's safety limit.",
        ));
    }
    Ok(encoded)
}

/// Encode the exact JSON payload that the session-detail IPC command returns.
pub fn encode_project_session_detail_response(
    value: &ProjectSessionDetail,
) -> SessionDetailResult<String> {
    encode_response_bound(value, MAX_SESSION_DETAIL_RESPONSE_BYTES)
}

/// Encode the exact JSON payload that the timeline-page IPC command returns.
pub fn encode_project_session_timeline_page_response(
    value: &ProjectSessionTimelinePage,
) -> SessionDetailResult<String> {
    encode_response_bound(value, MAX_TIMELINE_PAGE_RESPONSE_BYTES)
}

/// Encode the exact JSON payload that the event-detail IPC command returns.
pub fn encode_session_event_detail_response(
    value: &SessionEventDetail,
) -> SessionDetailResult<String> {
    encode_response_bound(value, MAX_EVENT_DETAIL_RESPONSE_BYTES)
}

fn summarize_session(
    registry: &AgentProviderRegistry,
    session: &ResolvedLocalSession,
    index: &SessionTimelineIndex,
) -> SessionDetailResult<crate::providers::SessionFileSummary> {
    let adapter = registry.require(&session.provider).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "No registered provider can read this session.",
        )
    })?;
    let summary = adapter
        .summarize_timeline(&session.transcript_path, index, &session.timeline_context())
        .map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider could not summarize this local session.",
            )
        })?;
    if summary.session_id != session.provider_session_id
        || summary.is_internal_subagent != session.child.is_some()
    {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The local session identity changed while loading its details.",
        ));
    }
    Ok(summary)
}

fn sanitize_boundary_text(value: &str, session: &ResolvedLocalSession) -> String {
    let transcript = session.transcript_path.to_string_lossy();
    let provider_home = session.provider_home.to_string_lossy();
    let mut redacted = value
        .replace(transcript.as_ref(), "<session-transcript>")
        .replace(provider_home.as_ref(), "<provider-home>");
    for project_root in &session.project_root_aliases {
        redacted = replace_sensitive_path_occurrences(
            &redacted,
            project_root.to_string_lossy().as_ref(),
            ".",
        );
    }
    redacted
}

fn sanitize_timeline_events(events: &mut [SessionTimelineEvent], session: &ResolvedLocalSession) {
    for event in events {
        if let Some(text) = event.text.as_mut() {
            *text = sanitize_boundary_text(text, session);
        }
        if let Some(tool) = event.tool.as_mut() {
            tool.native_name = sanitize_boundary_text(&tool.native_name, session);
            tool.summary = sanitize_boundary_text(&tool.summary, session);
            if let Some(arguments) = tool.argument_preview.as_mut() {
                *arguments = sanitize_boundary_text(arguments, session);
            }
            if let Some(result) = tool.result_preview.as_mut() {
                *result = sanitize_boundary_text(result, session);
            }
        }
        if let Some(checkpoint) = event.checkpoint.as_mut() {
            checkpoint.subject = sanitize_boundary_text(&checkpoint.subject, session);
        }
        if let Some(agent) = event.agent.as_mut() {
            sanitize_agent_preview(agent, session);
        }
    }
}

fn sanitize_agent_preview(agent: &mut SessionAgentPreview, session: &ResolvedLocalSession) {
    agent.label = sanitize_boundary_text(&agent.label, session);
    for text in [
        agent.task.as_mut(),
        agent.role.as_mut(),
        agent.brief.as_mut(),
        agent.report.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        *text = sanitize_boundary_text(text, session);
    }
}

fn sanitize_warnings(warnings: &mut [SessionDetailWarning], session: &ResolvedLocalSession) {
    for warning in warnings {
        warning.message = sanitize_boundary_text(&warning.message, session);
    }
}

fn sanitize_actions(
    actions: &mut [SessionActionDescriptor],
    session: &ResolvedLocalSession,
) -> SessionDetailResult<()> {
    for action in actions {
        ensure_identifier("session action id", &action.action_id)?;
        action.label = sanitize_boundary_text(&action.label, session);
        if let Some(reason) = action.disabled_reason.as_mut() {
            *reason = sanitize_boundary_text(reason, session);
        }
    }
    Ok(())
}

fn decode_hex(value: &str) -> SessionDetailResult<Vec<u8>> {
    if value.len() > 16 * 1024 || !value.len().is_multiple_of(2) {
        return Err(invalid_request("The timeline cursor is invalid."));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(byte: u8) -> SessionDetailResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(invalid_request("The timeline cursor is invalid.")),
    }
}

fn encode_cursor(cursor: &TimelineCursor) -> SessionDetailResult<String> {
    let bytes = serde_json::to_vec(cursor)
        .map_err(|_| invalid_request("Mallard could not encode the timeline cursor."))?;
    Ok(format!("v1-{}", hex_lower(&bytes)))
}

fn decode_cursor(value: &str) -> SessionDetailResult<TimelineCursor> {
    let encoded = value
        .strip_prefix("v1-")
        .ok_or_else(|| invalid_request("The timeline cursor is invalid."))?;
    let bytes = decode_hex(encoded)?;
    let cursor: TimelineCursor = serde_json::from_slice(&bytes)
        .map_err(|_| invalid_request("The timeline cursor is invalid."))?;
    if cursor.schema != 1 {
        return Err(invalid_request(
            "The timeline cursor version is unsupported.",
        ));
    }
    Ok(cursor)
}

fn filter_fingerprint(request: &ProjectSessionTimelineRequest) -> SessionDetailResult<String> {
    let bytes = serde_json::to_vec(&(
        &request.included_event_kinds,
        &request.included_tool_categories,
        &request.turn_id,
    ))
    .map_err(|_| invalid_request("The timeline filters are invalid."))?;
    Ok(sha256_hex(&bytes))
}

fn validate_slice_against_query(
    slice: &SessionTimelinePageSlice,
    index: &SessionTimelineIndex,
    query: &SessionTimelineQuery,
    provider: &ProviderKey,
) -> SessionDetailResult<()> {
    validate_page_slice(slice, index, query.limit)?;
    let indexed = index
        .events
        .iter()
        .map(|event| (event.event_id.as_str(), event))
        .collect::<BTreeMap<_, _>>();
    for event in &slice.events {
        let Some(source) = indexed.get(event.event_id.as_str()) else {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned an event outside the validated index.",
            ));
        };
        let tool_matches_index = match (&event.tool, &source.tool) {
            (Some(tool), Some(source_tool)) => {
                tool.provider_call_id == source_tool.provider_call_id
                    && tool.native_name == source_tool.native_name
                    && tool.category == source_tool.category
                    && tool.status == source_tool.status
                    && tool.affected_paths == source_tool.affected_paths
            }
            (None, None) => true,
            _ => false,
        };
        let reply_widget = provider.as_str() == "claude"
            && crate::providers::claude::history::is_reply_widget(source, query);
        if event.kind != source.kind
            || event.ordinal != source.ordinal
            || event.timestamp != source.timestamp
            || event.quality != source.quality
            || event.turn_id != source.turn_id
            || event.parent_event_id != source.parent_event_id
            || !tool_matches_index
            || (!query.included_event_kinds.contains(&event.kind) && !reply_widget)
            || (event.kind == SessionTimelineEventKind::ToolCall
                && !reply_widget
                && event
                    .tool
                    .as_ref()
                    .is_none_or(|tool| !query.included_tool_categories.contains(&tool.category)))
            || query
                .turn_id
                .as_ref()
                .is_some_and(|turn_id| event.turn_id.as_deref() != Some(turn_id.as_str()))
        {
            return Err(SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider returned an event outside the requested filters.",
            ));
        }
    }
    if slice
        .next_index
        .is_some_and(|next| query.start_index.is_some_and(|start| next <= start))
    {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The provider returned a non-advancing timeline cursor.",
        ));
    }
    Ok(())
}

/// Load the authoritative header and build/reuse the metadata-only timeline
/// index. Remote-only rows fail with `not_local`; Pull remains the only path
/// that makes a remote session resumable and richly viewable.
pub fn get_project_session_detail(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    views: &[LocalSessionView],
) -> SessionDetailResult<ProjectSessionDetail> {
    let ResolvedSession {
        mut session,
        parent,
    } = resolve_session(repository, registry, local_project_id, resource_id, views)?;
    let (index, summary, snapshot_id, cache_warnings) =
        load_or_build_index(repository, registry, &mut session)?;
    // A child page is the parent's row opened: the parent's index (cached)
    // holds the call, and the provider describes it the way the row does.
    let agent = match parent {
        Some(mut parent) => Some(describe_child(repository, registry, &mut parent, &session)?),
        None => None,
    };
    // A child is no session to resume, so it offers no verbs.
    let actions = if session.child.is_some() {
        Vec::new()
    } else {
        registry
            .require(&session.provider)
            .map(|adapter| {
                adapter.available_actions(&SessionActionContext {
                    provider_session_id: &session.provider_session_id,
                    is_local: true,
                })
            })
            .unwrap_or_default()
    };
    session_detail_body(
        registry,
        &session,
        SessionDetailParts {
            index: &index,
            summary: &summary,
            snapshot_id,
            cache_warnings,
            agent,
            actions,
        },
    )
}

/// What a page's detail is built from once its transcript is resolved and
/// indexed, however it was reached: a local file, or a stored copy.
struct SessionDetailParts<'a> {
    index: &'a SessionTimelineIndex,
    summary: &'a crate::providers::SessionFileSummary,
    snapshot_id: String,
    cache_warnings: Vec<SessionDetailWarning>,
    agent: Option<SessionDetailAgent>,
    actions: Vec<SessionActionDescriptor>,
}

fn session_detail_body(
    registry: &AgentProviderRegistry,
    session: &ResolvedLocalSession,
    parts: SessionDetailParts<'_>,
) -> SessionDetailResult<ProjectSessionDetail> {
    let SessionDetailParts {
        index,
        summary,
        snapshot_id,
        cache_warnings,
        agent,
        mut actions,
    } = parts;
    // Checkpoints are the parent's; a child neither commits nor is a
    // session to resume.
    let checkpoint = if session.child.is_some() {
        CheckpointSupport::Unsupported
    } else {
        checkpoint_support(
            &session.project_root,
            summary.branch.as_deref(),
            summary.last_prompt_at.or(summary.ended_at),
        )?
    };
    let event_counts = with_checkpoint_count(index.counts.clone(), &checkpoint);
    let tokens = match summary.total_tokens {
        Some(value) => SessionCount {
            value: Some(value),
            quality: if summary.metrics_complete {
                SessionCountQuality::Exact
            } else {
                SessionCountQuality::LowerBound
            },
            explanation: (!summary.metrics_complete)
                .then(|| "The provider transcript may contain additional token usage.".to_string()),
        },
        None => SessionCount::unsupported(
            "This provider transcript does not expose a reliable token total.",
        ),
    };
    sanitize_actions(&mut actions, session)?;
    let mut warnings = timeline_warnings(index, cache_warnings);
    sanitize_warnings(&mut warnings, session);
    // The provider index title leads: keep-both preservation markers are
    // title renames in the provider's session index, which the transcript
    // summary never reflects. The transcript title covers unnamed sessions.
    // A child is titled by the parent's row: the task it was given.
    let title = agent
        .as_ref()
        .map(|agent| agent.preview.label.clone())
        .or_else(|| {
            (session.display_name != session.provider_session_id)
                .then(|| session.display_name.clone())
                .filter(|title| !title.trim().is_empty())
        })
        .or_else(|| {
            summary
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
        })
        .unwrap_or_else(|| session.display_name.clone());
    let detail = ProjectSessionDetail {
        resource_id: session.resource_id.clone(),
        provider: session.provider.clone(),
        provider_display_name: provider_display_name(registry, &session.provider),
        provider_session_id: session.provider_session_id.clone(),
        title: sanitize_boundary_text(&title, session),
        summary: summary
            .summary
            .as_deref()
            .map(|summary| sanitize_boundary_text(summary, session)),
        started_at: summary.started_at,
        ended_at: summary.ended_at,
        is_active: false,
        branch: summary
            .branch
            .as_deref()
            .map(|branch| sanitize_boundary_text(branch, session)),
        recorded_sha: summary
            .recorded_sha
            .clone()
            .filter(|sha| valid_full_sha(sha)),
        tokens,
        event_counts,
        actions,
        github_repository_url: matches!(checkpoint, CheckpointSupport::Supported(_))
            .then(|| discover_github_repository_url(&session.project_root))
            .flatten(),
        snapshot_id,
        warnings,
        agent,
        settings: session_settings(index),
    };
    Ok(detail)
}

/// What the session started with: its first message's settings, when any
/// message recorded a value at all. A session recorded before the clients
/// wrote these fields has none to state.
fn session_settings(index: &SessionTimelineIndex) -> Option<SessionMessageSettings> {
    let mut messages = index
        .events
        .iter()
        .filter(|entry| is_message_kind(entry.kind))
        .map(|entry| &entry.settings);
    let first = messages.next()?.clone();
    (first.is_recorded() || messages.any(SessionMessageSettings::is_recorded)).then_some(first)
}

/// The parent's row for this child, read from the parent's cached index.
fn describe_child(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    parent: &mut ResolvedLocalSession,
    child: &ResolvedLocalSession,
) -> SessionDetailResult<SessionDetailAgent> {
    let (index, parent_summary, _, _) = load_or_build_index(repository, registry, parent)?;
    let entry = index
        .events
        .iter()
        .find(|event| {
            event.kind == SessionTimelineEventKind::AgentCall
                && event
                    .agent
                    .as_ref()
                    .and_then(|agent| agent.provider_agent_id.as_deref())
                    == Some(child.provider_session_id.as_str())
        })
        .ok_or_else(|| {
            SessionDetailError::new(
                SessionDetailErrorCode::NotFound,
                "The parent session does not record this agent.",
            )
        })?;
    let adapter = registry.require(&parent.provider).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "No registered provider can read this session.",
        )
    })?;
    let mut preview = adapter
        .describe_agent_call(&parent.transcript_path, entry, &parent.timeline_context())
        .map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider could not describe this agent.",
            )
        })?;
    sanitize_agent_preview(&mut preview, parent);
    let parent_title = (parent.display_name != parent.provider_session_id)
        .then(|| parent.display_name.clone())
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            parent_summary
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
        })
        .unwrap_or_else(|| parent.display_name.clone());
    Ok(SessionDetailAgent {
        parent_resource_id: parent.resource_id.clone(),
        parent_title: sanitize_boundary_text(&parent_title, parent),
        preview,
    })
}

/// Read one filtered, chronological page. The opaque cursor is bound to the
/// provider, resource, transcript snapshot, filter fingerprint, and raw next
/// index. Provider filtering occurs before pagination.
pub fn get_project_session_timeline_page(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    request: ProjectSessionTimelineRequest,
    views: &[LocalSessionView],
) -> SessionDetailResult<ProjectSessionTimelinePage> {
    ensure_identifier("snapshot id", &request.snapshot_id)?;
    if let Some(turn_id) = &request.turn_id {
        ensure_identifier("turn id", turn_id)?;
    }
    let ResolvedSession {
        mut session,
        parent: _,
    } = resolve_session(repository, registry, local_project_id, resource_id, views)?;
    let (index, summary, current_snapshot_id, cache_warnings) =
        load_or_build_index(repository, registry, &mut session)?;
    if request.snapshot_id != current_snapshot_id {
        return Err(SessionDetailError::snapshot_changed(current_snapshot_id));
    }
    timeline_page_body(
        registry,
        &session,
        &index,
        &summary,
        current_snapshot_id,
        cache_warnings,
        &request,
    )
}

/// One filtered page of a resolved, indexed session whose snapshot the
/// request already matched.
fn timeline_page_body(
    registry: &AgentProviderRegistry,
    session: &ResolvedLocalSession,
    index: &SessionTimelineIndex,
    summary: &crate::providers::SessionFileSummary,
    current_snapshot_id: String,
    cache_warnings: Vec<SessionDetailWarning>,
    request: &ProjectSessionTimelineRequest,
) -> SessionDetailResult<ProjectSessionTimelinePage> {
    let fingerprint = filter_fingerprint(request)?;
    let start_index = if let Some(cursor) = &request.cursor {
        let cursor = decode_cursor(cursor)?;
        if cursor.provider != session.provider
            || cursor.resource_id != session.resource_id
            || cursor.snapshot_id != current_snapshot_id
            || cursor.filter_fingerprint != fingerprint
        {
            return Err(invalid_request(
                "The timeline cursor does not match this session, snapshot, or filter set.",
            ));
        }
        cursor.next_index
    } else {
        0
    };
    if start_index > index.events.len() {
        return Err(invalid_request("The timeline cursor position is invalid."));
    }
    let limit = request
        .limit
        .unwrap_or(MAX_TIMELINE_PAGE_EVENTS)
        .clamp(1, 50);
    let adapter = registry.require(&session.provider).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "No registered provider can read this session.",
        )
    })?;
    let query = SessionTimelineQuery {
        start_index: Some(start_index),
        limit,
        included_event_kinds: request.included_event_kinds.clone(),
        included_tool_categories: request.included_tool_categories.clone(),
        turn_id: request.turn_id.clone(),
    };
    let mut slice = if start_index < index.events.len() {
        adapter
            .read_timeline_page(
                &session.transcript_path,
                index,
                &query,
                &session.timeline_context(),
            )
            .map_err(|_| {
                SessionDetailError::new(
                    SessionDetailErrorCode::ProviderError,
                    "The provider could not read the requested timeline page.",
                )
            })?
    } else {
        SessionTimelinePageSlice {
            events: Vec::new(),
            next_index: None,
        }
    };
    validate_slice_against_query(&slice, index, &query, &session.provider)?;
    let checkpoint_requested = request.turn_id.is_none()
        && session.child.is_none()
        && request
            .included_event_kinds
            .contains(&SessionTimelineEventKind::Checkpoint);
    if slice.next_index.is_none() && checkpoint_requested {
        if let CheckpointSupport::Supported(Some(checkpoint)) = checkpoint_support(
            &session.project_root,
            summary.branch.as_deref(),
            summary.last_prompt_at.or(summary.ended_at),
        )? {
            if slice.events.len() < limit {
                slice.events.push(checkpoint_event(index, checkpoint));
            } else {
                slice.next_index = Some(index.events.len());
            }
        }
    }
    sanitize_timeline_events(&mut slice.events, session);
    let next_cursor = slice
        .next_index
        .map(|next_index| {
            encode_cursor(&TimelineCursor {
                schema: 1,
                provider: session.provider.clone(),
                resource_id: session.resource_id.clone(),
                snapshot_id: current_snapshot_id.clone(),
                filter_fingerprint: fingerprint.clone(),
                next_index,
            })
        })
        .transpose()?;
    let mut warnings = timeline_warnings(index, cache_warnings);
    sanitize_warnings(&mut warnings, session);
    let page = ProjectSessionTimelinePage {
        resource_id: session.resource_id.clone(),
        snapshot_id: current_snapshot_id,
        events: slice.events,
        next_cursor,
        warnings,
    };
    Ok(page)
}

fn truncate_to_bytes(value: &mut Option<String>, limit: usize, truncated: &mut bool) {
    let Some(text) = value.as_mut() else {
        return;
    };
    if text.len() <= limit {
        return;
    }
    let mut boundary = limit.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
    *truncated = true;
}

/// Expand one indexed tool event on demand. Prompt/response text is already
/// carried by timeline pages; raw reasoning and unindexed records are never
/// addressable through this command.
pub fn get_project_session_event_detail(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    snapshot_id_value: &str,
    event_id: &str,
    views: &[LocalSessionView],
) -> SessionDetailResult<SessionEventDetail> {
    ensure_identifier("snapshot id", snapshot_id_value)?;
    ensure_identifier("event id", event_id)?;
    let ResolvedSession {
        mut session,
        parent: _,
    } = resolve_session(repository, registry, local_project_id, resource_id, views)?;
    let (index, _, current_snapshot_id, _) =
        load_or_build_index(repository, registry, &mut session)?;
    if snapshot_id_value != current_snapshot_id {
        return Err(SessionDetailError::snapshot_changed(current_snapshot_id));
    }
    event_detail_body(registry, &session, &index, event_id)
}

/// One indexed tool event's body, from a resolved, indexed session whose
/// snapshot the request already matched.
fn event_detail_body(
    registry: &AgentProviderRegistry,
    session: &ResolvedLocalSession,
    index: &SessionTimelineIndex,
    event_id: &str,
) -> SessionDetailResult<SessionEventDetail> {
    let indexed = index
        .events
        .iter()
        .find(|event| event.event_id == event_id)
        .ok_or_else(|| invalid_request("The timeline event does not exist."))?;
    if !matches!(
        indexed.kind,
        SessionTimelineEventKind::ToolCall | SessionTimelineEventKind::AgentCall
    ) || indexed.tool.is_none()
    {
        return Err(invalid_request(
            "Only tool-call events have expanded detail.",
        ));
    }
    let adapter = registry.require(&session.provider).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "No registered provider can read this session.",
        )
    })?;
    let mut detail = adapter
        .read_event_detail(
            &session.transcript_path,
            index,
            event_id,
            &session.timeline_context(),
        )
        .map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider could not expand the requested tool event.",
            )
        })?;
    if detail.event_id != event_id
        || detail
            .affected_paths
            .iter()
            .any(|path| path.len() > 1024 || !safe_relative_path(path))
    {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The provider returned unsafe event detail.",
        ));
    }
    if let Some(arguments) = detail.arguments.as_mut() {
        *arguments = sanitize_boundary_text(arguments, session);
    }
    if let Some(result) = detail.result.as_mut() {
        *result = sanitize_boundary_text(result, session);
    }
    let (argument_budget, result_budget) =
        timeline_tool_detail_budgets(detail.arguments.as_deref(), detail.result.as_deref());
    truncate_to_bytes(
        &mut detail.arguments,
        argument_budget,
        &mut detail.arguments_truncated,
    );
    truncate_to_bytes(
        &mut detail.result,
        result_budget,
        &mut detail.result_truncated,
    );
    Ok(detail)
}

// ---------------------------------------------------------------------------
// Stored sessions: a row that is only in a storage, read without a Pull
// ---------------------------------------------------------------------------

/// A stored session, resolved the way a local one is: its private temporary
/// home is the provider home and its project root is this project's, so
/// checkpoints come from the local repository and path links open here.
/// It has no binding (nothing was installed) and is never a child.
fn resolve_stored_session(
    stored: &StoredSessionTranscript,
) -> SessionDetailResult<ResolvedLocalSession> {
    ensure_identifier("resource id", stored.resource_id())?;
    let (_, hops) = split_agent_resource_id(stored.resource_id());
    if !hops.is_empty() {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::NotLocal,
            "This agent's transcript is not available for a stored session.",
        ));
    }
    let metadata = fs::symlink_metadata(stored.transcript_path()).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The stored session could not be read.",
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The stored session could not be read.",
        ));
    }
    Ok(ResolvedLocalSession {
        provider: stored.provider().clone(),
        resource_id: stored.resource_id().to_string(),
        project_root: stored.project_root().to_path_buf(),
        project_root_aliases: stored.project_root_aliases().to_vec(),
        provider_home: stored.provider_home().to_path_buf(),
        provider_session_id: stored.provider_session_id().to_string(),
        transcript_path: stored.transcript_path().to_path_buf(),
        display_name: stored.display_name().to_string(),
        file_size: metadata.len(),
        modified_nanos: modified_nanos(&metadata),
        binding: None,
        child: None,
    })
}

/// A stored session's snapshot is its content: the stored object's digest
/// and the limits its index is built under. A new publication changes it;
/// nothing else does, so a page's cursors stay valid across requests even
/// though each request prepares the transcript afresh.
fn stored_snapshot_id(stored: &StoredSessionTranscript, timeline_version: u32) -> String {
    sha256_hex(
        format!(
            "stored-session-v1\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            stored.provider(),
            stored.resource_id(),
            stored.wire_sha256(),
            timeline_version,
            timeline_message_preview_bytes() as u64,
            conversation_file_limit_bytes(),
            stored.project_root().display()
        )
        .as_bytes(),
    )
}

/// Index the prepared transcript for this request only: a stored session
/// keeps no cache, on disk or in memory.
fn build_fresh_index(
    registry: &AgentProviderRegistry,
    session: &ResolvedLocalSession,
) -> SessionDetailResult<(SessionTimelineIndex, crate::providers::SessionFileSummary)> {
    let adapter = registry.require(&session.provider).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::NotFound,
            "No registered provider can read this session.",
        )
    })?;
    let timeline_version = adapter.timeline_version();
    let index = adapter
        .build_timeline_index(&session.transcript_path, &session.timeline_context())
        .map_err(|_| {
            SessionDetailError::new(
                SessionDetailErrorCode::ProviderError,
                "The provider could not build a safe timeline for this session.",
            )
        })?;
    if index.parser_version != timeline_version {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ProviderError,
            "The provider returned a timeline with an unexpected parser version.",
        ));
    }
    validate_index(&index, session.file_size)?;
    let summary = summarize_session(registry, session, &index)?;
    Ok((index, summary))
}

/// The page's detail for a stored session. It offers no launch verbs:
/// nothing here opens in the provider or in Terminal.
pub fn get_stored_session_detail(
    registry: &AgentProviderRegistry,
    stored: &StoredSessionTranscript,
) -> SessionDetailResult<ProjectSessionDetail> {
    let session = resolve_stored_session(stored)?;
    let (index, summary) = build_fresh_index(registry, &session)?;
    let snapshot_id = stored_snapshot_id(stored, index.parser_version);
    session_detail_body(
        registry,
        &session,
        SessionDetailParts {
            index: &index,
            summary: &summary,
            snapshot_id,
            cache_warnings: Vec::new(),
            agent: None,
            actions: Vec::new(),
        },
    )
}

pub fn get_stored_session_timeline_page(
    registry: &AgentProviderRegistry,
    stored: &StoredSessionTranscript,
    request: ProjectSessionTimelineRequest,
) -> SessionDetailResult<ProjectSessionTimelinePage> {
    ensure_identifier("snapshot id", &request.snapshot_id)?;
    if let Some(turn_id) = &request.turn_id {
        ensure_identifier("turn id", turn_id)?;
    }
    let session = resolve_stored_session(stored)?;
    let (index, summary) = build_fresh_index(registry, &session)?;
    let current_snapshot_id = stored_snapshot_id(stored, index.parser_version);
    if request.snapshot_id != current_snapshot_id {
        return Err(SessionDetailError::snapshot_changed(current_snapshot_id));
    }
    timeline_page_body(
        registry,
        &session,
        &index,
        &summary,
        current_snapshot_id,
        Vec::new(),
        &request,
    )
}

pub fn get_stored_session_event_detail(
    registry: &AgentProviderRegistry,
    stored: &StoredSessionTranscript,
    snapshot_id_value: &str,
    event_id: &str,
) -> SessionDetailResult<SessionEventDetail> {
    ensure_identifier("snapshot id", snapshot_id_value)?;
    ensure_identifier("event id", event_id)?;
    let session = resolve_stored_session(stored)?;
    let (index, _) = build_fresh_index(registry, &session)?;
    let current_snapshot_id = stored_snapshot_id(stored, index.parser_version);
    if snapshot_id_value != current_snapshot_id {
        return Err(SessionDetailError::snapshot_changed(current_snapshot_id));
    }
    event_detail_body(registry, &session, &index, event_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_capture::modified_secs;
    use crate::providers::{
        SessionSourceSpan, SessionTimelineIndexEntry, SessionToolCallStatus, SessionToolIndexEntry,
    };
    use mallard_domain::{
        conversation_resource_id, initial_conversation_thread_id, BindingState, BundleId,
        BundleRecipe, LocalProjectRegistration, LocalProviderProfileId, ProjectBinding,
        ProviderProfile, ReplicaId,
    };

    #[test]
    fn bounded_response_returns_the_measured_json() {
        let value = vec!["quoted \"value\"", "line\nbreak"];
        let expected = serde_json::to_string(&value).unwrap();
        assert_eq!(
            encode_response_bound(&value, expected.len()).unwrap(),
            expected
        );
        assert_eq!(
            encode_response_bound(&value, expected.len() - 1)
                .unwrap_err()
                .code,
            SessionDetailErrorCode::ResponseTooLarge
        );
    }

    fn counts() -> SessionEventCounts {
        SessionEventCounts {
            prompts: SessionCount::exact(1),
            responses: SessionCount::exact(1),
            intermediate_steps: SessionCount::exact(0),
            checkpoints: SessionCount::unsupported("common Git layer"),
            tool_calls: SessionCount::exact(1),
            file_edits: SessionCount::exact(1),
            shell_calls: SessionCount::exact(0),
            reads: SessionCount::exact(0),
            other_tools: SessionCount::exact(0),
            agent_calls: SessionCount::exact(0),
        }
    }

    fn index_with_path(path: &str) -> SessionTimelineIndex {
        SessionTimelineIndex {
            sources: Vec::new(),
            parser_version: 1,
            events: vec![SessionTimelineIndexEntry {
                event_id: "event-1".to_string(),
                ordinal: 0,
                timestamp: Some(1),
                kind: SessionTimelineEventKind::ToolCall,
                quality: SessionCountQuality::Exact,
                turn_id: Some("turn-1".to_string()),
                turn_tool_summary: Some(crate::providers::SessionTurnToolSummary {
                    turn_id: "turn-1".to_string(),
                    tool_calls: SessionCount::exact(1),
                }),
                parent_event_id: None,
                source: SessionSourceSpan {
                    offset: 0,
                    length: 4,
                },
                tool: Some(SessionToolIndexEntry {
                    provider_call_id: "call-1".to_string(),
                    native_name: "Write".to_string(),
                    category: SessionToolCategory::FileEdit,
                    status: SessionToolCallStatus::Completed,
                    affected_paths: vec![path.to_string()],
                    result_source: None,
                }),
                agent: None,
                settings: SessionMessageSettings::none(),
            }],
            counts: counts(),
            warnings: Vec::new(),
            truncated: false,
        }
    }

    fn sanitization_session() -> ResolvedLocalSession {
        ResolvedLocalSession {
            provider: ProviderKey::parse("codex").unwrap(),
            resource_id: "codex:conversation:thread-1".to_string(),
            project_root: PathBuf::from("/tmp/run/project1"),
            project_root_aliases: vec![PathBuf::from("/tmp/run/project1")],
            provider_home: PathBuf::from("/tmp/run/myconf1"),
            provider_session_id: "thread-1".to_string(),
            transcript_path: PathBuf::from("/tmp/run/myconf1/sessions/thread.jsonl"),
            display_name: "thread-1".to_string(),
            file_size: 0,
            modified_nanos: 0,
            binding: None,
            child: None,
        }
    }

    fn summary_fixture() -> SessionFileSummary {
        SessionFileSummary {
            session_id: "thread-1".to_string(),
            title: None,
            summary: None,
            started_at: None,
            ended_at: None,
            last_prompt_at: None,
            branch: None,
            recorded_sha: None,
            user_rounds: 0,
            agent_messages: 0,
            tool_calls: 0,
            total_input_tokens: None,
            total_output_tokens: None,
            total_tokens: None,
            metrics_complete: false,
            is_internal_subagent: false,
        }
    }

    #[test]
    fn what_a_pull_installed_of_the_children_changes_the_viewer_snapshot() {
        use crate::owned_subagents::agent_card_fixtures::binding_of_a_pulled_session;
        let mut session = sanitization_session();
        let unbound = snapshot_id(&session, 1);
        let mut binding = binding_of_a_pulled_session(None, &[], &[]);
        session.binding = Some(binding.clone());
        assert_eq!(snapshot_id(&session, 1), unbound, "a native session");

        let mut seen = BTreeSet::from([unbound]);
        binding.installed_end = Some(64);
        session.binding = Some(binding.clone());
        assert!(seen.insert(snapshot_id(&session, 1)), "a pulled session");
        binding.installed_absent_subagents.insert("agent-1".into());
        session.binding = Some(binding.clone());
        assert!(
            seen.insert(snapshot_id(&session, 1)),
            "a child without a transcript"
        );
        binding.installed_absent_subagents.clear();
        binding.installed_subagents.insert(
            mallard_domain::SubagentKey::child(None, "call-1").unwrap(),
            mallard_domain::InstalledSubagent {
                provider_thread_id: "agent-1".into(),
                installed_end: 8,
            },
        );
        session.binding = Some(binding);
        assert!(seen.insert(snapshot_id(&session, 1)), "an installed child");
    }

    #[test]
    fn an_agent_call_that_is_left_out_hands_on_the_tool_summary_of_its_turn() {
        use crate::owned_subagents::agent_card_fixtures::agent_call;
        // The turn opens with the agent call, so the call holds the summary.
        let mut index = index_with_path("file");
        index.events[0].ordinal = 1;
        let summary = index.events[0].turn_tool_summary.take();
        let mut call = agent_call("agent-1", None, 0);
        call.turn_tool_summary = summary.clone();
        index.events.insert(0, call);
        index.events.push(SessionTimelineIndexEntry {
            ordinal: 2,
            ..agent_call("agent-2", None, 0)
        });
        index.counts.agent_calls = SessionCount::exact(2);
        validate_index(&index, 4).unwrap();

        leave_out_agent_calls(&mut index, &BTreeSet::from(["agent-1".to_string()]));
        validate_index(&index, 4).unwrap();
        assert_eq!(
            index
                .events
                .iter()
                .map(|entry| entry.event_id.as_str())
                .collect::<Vec<_>>(),
            ["event-1", "agent-2"]
        );
        assert_eq!(index.counts.agent_calls, SessionCount::exact(1));
        // No entry of the turn is left but a tool call and an agent call:
        // the agent call holds the summary, as it would in a new index.
        assert_eq!(index.events[1].turn_tool_summary, summary);
        assert_eq!(index.counts.tool_calls, SessionCount::exact(1));

        // An ID that names no agent call leaves the index as it is.
        let before = index.clone();
        leave_out_agent_calls(&mut index, &BTreeSet::from(["event-1".to_string()]));
        assert_eq!(index, before);
    }

    fn cache_entry_fixture() -> TimelineCacheEntry {
        TimelineCacheEntry {
            schema: TIMELINE_CACHE_SCHEMA,
            resource_id: "codex:conversation:thread-1".to_string(),
            provider: ProviderKey::parse("codex").unwrap(),
            size: 4,
            modified_nanos: 9,
            timeline_version: 1,
            preview_budget_bytes: 1024 * 1024,
            conversation_limit_bytes: 100 * 1024 * 1024,
            summary: summary_fixture(),
            index: index_with_path("file"),
        }
    }

    #[test]
    fn a_cache_entry_without_message_settings_is_outdated_and_rebuilt() {
        let sandbox = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(sandbox.path()).unwrap();
        let key = "d".repeat(64);
        write_timeline_cache(&repository, &key, &cache_entry_fixture(), 1024 * 1024).unwrap();
        assert!(load_timeline_cache(&repository, &key).0.is_some());

        // The shape before per-message settings: the same schema number, no key.
        let path = repository
            .root()
            .join("session_timeline_cache")
            .join(format!("{key}.json"));
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["index"]["events"][0]
            .as_object_mut()
            .unwrap()
            .remove("settings")
            .unwrap();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let (entry, warnings) = load_timeline_cache(&repository, &key);
        assert!(entry.is_none());
        assert_eq!(
            warnings
                .iter()
                .map(|warning| warning.code.as_str())
                .collect::<Vec<_>>(),
            vec!["timeline_cache_ignored"]
        );
    }

    #[test]
    fn timeline_cache_evicts_oldest_by_bytes_and_accounts_for_replacement() {
        let sandbox = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(sandbox.path()).unwrap();
        let entry = cache_entry_fixture();
        let entry_bytes = serde_json::to_vec(&entry).unwrap().len() as u64;
        let budget = entry_bytes * 2;
        let directory = repository.root().join("session_timeline_cache");
        let first = "a".repeat(64);
        let second = "b".repeat(64);
        let third = "c".repeat(64);
        write_timeline_cache(&repository, &first, &entry, budget).unwrap();
        write_timeline_cache(&repository, &second, &entry, budget).unwrap();
        // Replacing a file of the same size at the byte limit needs no eviction.
        write_timeline_cache(&repository, &second, &entry, budget).unwrap();
        assert!(load_timeline_cache(&repository, &first).0.is_some());
        fs::File::open(directory.join(format!("{first}.json")))
            .unwrap()
            .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1))
            .unwrap();

        write_timeline_cache(&repository, &third, &entry, budget).unwrap();
        assert!(load_timeline_cache(&repository, &first).0.is_none());
        assert!(load_timeline_cache(&repository, &second).0.is_some());
        assert!(load_timeline_cache(&repository, &third).0.is_some());
        let total_bytes: u64 = fs::read_dir(&directory)
            .unwrap()
            .map(|item| item.unwrap().metadata().unwrap().len())
            .sum();
        assert_eq!(total_bytes, budget);

        // An entry that cannot fit must fail before it removes other entries.
        let mut oversized = entry;
        oversized.summary.title = Some("x".repeat(budget as usize));
        assert!(write_timeline_cache(&repository, &third, &oversized, budget).is_err());
        assert!(load_timeline_cache(&repository, &second).0.is_some());
        assert!(load_timeline_cache(&repository, &third)
            .0
            .unwrap()
            .summary
            .title
            .is_none());
    }

    #[test]
    fn timeline_cache_reuses_index_and_summary_after_reaching_32_mib() {
        let sandbox = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(sandbox.path()).unwrap();
        let registry = AgentProviderRegistry::with_test_provider();
        let mut session = sanitization_session();
        session.provider = crate::providers::test_provider::provider_key();
        session.provider_home = fs::canonicalize(sandbox.path()).unwrap();
        session.transcript_path = session.provider_home.join("session.jsonl");
        // Rebuilding this fixture cannot produce a valid session summary.
        fs::write(&session.transcript_path, b"null").unwrap();
        refresh_file_state(&mut session).unwrap();
        assert!(summarize_session(&registry, &session, &index_with_path("safe.txt")).is_err());
        let mut entry = cache_entry_fixture();
        entry.provider = session.provider.clone();
        entry.modified_nanos = session.modified_nanos;
        entry.timeline_version = registry
            .require(&session.provider)
            .unwrap()
            .timeline_version();
        entry.index.parser_version = entry.timeline_version;
        entry.preview_budget_bytes = timeline_message_preview_bytes() as u64;
        entry.conversation_limit_bytes = conversation_file_limit_bytes();
        entry.summary.title = Some("Cached summary".to_string());
        let key = cache_key(&session);
        let directory = repository.root().join("session_timeline_cache");
        repository.ensure_directory(&directory).unwrap();
        let old_path = directory.join(format!("{}.json", "a".repeat(64)));
        fs::File::create(&old_path)
            .unwrap()
            .set_len(MAX_TIMELINE_CACHE_BYTES)
            .unwrap();
        write_timeline_cache(&repository, &key, &entry, MAX_TIMELINE_CACHE_BYTES).unwrap();
        assert!(!old_path.exists());

        // An unreadable neighboring cache must not affect this session's reads.
        fs::write(&old_path, b"invalid JSON").unwrap();
        for _ in 0..2 {
            let (index, summary, _, warnings) =
                load_or_build_index(&repository, &registry, &mut session).unwrap();
            assert_eq!(index.events[0].event_id, "event-1");
            assert_eq!(summary.title.as_deref(), Some("Cached summary"));
            assert!(warnings.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn timeline_cache_rejects_symlink_directories_and_files() {
        use std::os::unix::fs::symlink;

        let sandbox = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(sandbox.path()).unwrap();
        let entry = cache_entry_fixture();
        let key = "a".repeat(64);
        let directory = repository.root().join("session_timeline_cache");
        let outside = sandbox.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let target = outside.join(format!("{key}.json"));
        fs::write(&target, b"unchanged").unwrap();
        symlink(&outside, &directory).unwrap();
        assert!(write_timeline_cache(&repository, &key, &entry, MAX_TIMELINE_CACHE_BYTES).is_err());
        assert!(load_timeline_cache(&repository, &key).0.is_none());
        fs::remove_file(&directory).unwrap();
        fs::create_dir(&directory).unwrap();
        symlink(&target, directory.join(format!("{key}.json"))).unwrap();
        assert!(write_timeline_cache(&repository, &key, &entry, MAX_TIMELINE_CACHE_BYTES).is_err());
        assert!(load_timeline_cache(&repository, &key).0.is_none());
        assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    }

    #[test]
    fn tuning_stamps_gate_reuse_and_join_the_snapshot_identity() {
        let stamped = cache_entry_fixture();
        assert!(entry_matches_limits(
            &stamped,
            1024 * 1024,
            100 * 1024 * 1024
        ));
        assert!(!entry_matches_limits(
            &stamped,
            512 * 1024,
            100 * 1024 * 1024
        ));
        assert!(!entry_matches_limits(
            &stamped,
            1024 * 1024,
            50 * 1024 * 1024
        ));

        // Entries without stamps deserialize with zeroed limits and never
        // match.
        let mut legacy = serde_json::to_value(&stamped)
            .unwrap()
            .as_object()
            .cloned()
            .unwrap();
        legacy.remove("preview_budget_bytes");
        legacy.remove("conversation_limit_bytes");
        let legacy: TimelineCacheEntry =
            serde_json::from_value(serde_json::Value::Object(legacy)).unwrap();
        assert_eq!(legacy.preview_budget_bytes, 0);
        assert_eq!(legacy.conversation_limit_bytes, 0);
        assert!(!entry_matches_limits(
            &legacy,
            1024 * 1024,
            100 * 1024 * 1024
        ));

        // The limits join the snapshot identity, so a tuning change retires
        // open cursors through the existing SnapshotChanged refresh path.
        let session = sanitization_session();
        let base = snapshot_id_with_limits(&session, 1, 1024 * 1024, 100 * 1024 * 1024);
        assert_eq!(
            base,
            snapshot_id_with_limits(&session, 1, 1024 * 1024, 100 * 1024 * 1024)
        );
        assert_ne!(
            base,
            snapshot_id_with_limits(&session, 1, 512 * 1024, 100 * 1024 * 1024)
        );
        assert_ne!(
            base,
            snapshot_id_with_limits(&session, 1, 1024 * 1024, 50 * 1024 * 1024)
        );
    }

    #[test]
    fn selected_rollout_path_changes_snapshot_even_with_equal_file_metadata() {
        let mut session = sanitization_session();
        let before = snapshot_id(&session, 6);
        session.transcript_path = session.provider_home.join("reverted.jsonl");
        assert_ne!(snapshot_id(&session, 6), before);
    }

    #[test]
    fn session_text_sanitizes_only_boundary_valid_project_root_occurrences() {
        let session = sanitization_session();
        let cases = [
            ("/tmp/run/project1", "."),
            ("/tmp/run/project1/file", "./file"),
            (
                "/tmp/run/project1_external/file",
                "/tmp/run/project1_external/file",
            ),
            ("/tmp/run/project1+copy/file", "/tmp/run/project1+copy/file"),
            ("/tmp/run/project1:copy/file", "/tmp/run/project1:copy/file"),
            ("/tmp/run/project1=copy/file", "/tmp/run/project1=copy/file"),
            ("/tmp/run/project1?copy/file", "/tmp/run/project1?copy/file"),
            ("/tmp/run/project1!copy/file", "/tmp/run/project1!copy/file"),
            (
                "/tmp/run/project1\\copy/file",
                "/tmp/run/project1\\copy/file",
            ),
            ("/tmp/run/project1é/file", "/tmp/run/project1é/file"),
            ("/tmp/run/project1🦆/file", "/tmp/run/project1🦆/file"),
            ("/tmp/run/project1-copy/file", "/tmp/run/project1-copy/file"),
            ("/tmp/run/project1.bak/file", "/tmp/run/project1.bak/file"),
            ("/tmp/run/project10/file", "/tmp/run/project10/file"),
            ("/host/tmp/run/project1/file", "/host/tmp/run/project1/file"),
            (
                "outside=/tmp/run/project1_external/file inside=/tmp/run/project1/file",
                "outside=/tmp/run/project1_external/file inside=./file",
            ),
            ("wrote /tmp/run/project1.", "wrote ."),
        ];

        for (input, expected) in cases {
            assert_eq!(sanitize_boundary_text(input, &session), expected, "{input}");
        }
    }

    #[test]
    fn session_text_sanitizes_selected_and_canonical_project_root_spellings() {
        let mut session = sanitization_session();
        session.project_root = PathBuf::from("/private/var/folders/run/project1_copy");
        session.project_root_aliases = vec![
            PathBuf::from("/private/var/folders/run/project1_copy"),
            PathBuf::from("/var/folders/run/project1_copy"),
        ];

        let input = concat!(
            "selected=/var/folders/run/project1_copy/generated/a.txt ",
            "canonical=/private/var/folders/run/project1_copy/generated/b.txt ",
            "outside=/var/folders/run/project1_copy_external/outside.txt"
        );
        assert_eq!(
            sanitize_boundary_text(input, &session),
            concat!(
                "selected=./generated/a.txt ",
                "canonical=./generated/b.txt ",
                "outside=/var/folders/run/project1_copy_external/outside.txt"
            )
        );
    }

    #[test]
    fn session_text_keeps_transcript_first_and_provider_home_redactions() {
        let session = sanitization_session();
        let input = concat!(
            "transcript=/tmp/run/myconf1/sessions/thread.jsonl ",
            "cache=/tmp/run/myconf1/cache/index.json ",
            "project=/tmp/run/project1/src/lib.rs"
        );
        assert_eq!(
            sanitize_boundary_text(input, &session),
            concat!(
                "transcript=<session-transcript> ",
                "cache=<provider-home>/cache/index.json ",
                "project=./src/lib.rs"
            )
        );
    }

    #[test]
    fn session_text_uses_the_same_boundaries_in_uri_and_json_text() {
        let session = sanitization_session();
        let input = r#"{"path":"/tmp/run/project1/file","outside":"/tmp/run/project1_external/file","changes":{"/tmp/run/project1/file":{},"/tmp/run/project1_external/file":{}},"uri":"file:///tmp/run/project1/file","outside_uri":"file:///tmp/run/project1_external/file"}"#;
        let expected = r#"{"path":"./file","outside":"/tmp/run/project1_external/file","changes":{"./file":{},"/tmp/run/project1_external/file":{}},"uri":"file://./file","outside_uri":"file:///tmp/run/project1_external/file"}"#;
        let sanitized = sanitize_boundary_text(input, &session);
        assert_eq!(sanitized, expected);
        serde_json::from_str::<serde_json::Value>(&sanitized)
            .expect("sanitization preserves serialized JSON text");
    }

    #[test]
    fn session_text_redacts_exact_json_roots_without_rewriting_root_dot_siblings() {
        let session = sanitization_session();
        let input = r#"{"cwd":"/tmp/run/project1","changes":{"/tmp/run/project1":{}},"outside":"/tmp/run/project1."}"#;
        let expected = r#"{"cwd":".","changes":{".":{}},"outside":"/tmp/run/project1."}"#;
        let sanitized = sanitize_boundary_text(input, &session);
        assert_eq!(sanitized, expected);
        serde_json::from_str::<serde_json::Value>(&sanitized)
            .expect("sanitization preserves exact-root JSON text");
    }

    #[test]
    fn timeline_index_rejects_absolute_affected_paths() {
        for path in [
            "/private/project/secret",
            "/tmp/run/project1_external/outside.txt",
        ] {
            let error = validate_index(&index_with_path(path), 4)
                .expect_err("absolute path must fail closed");
            assert_eq!(error.code, SessionDetailErrorCode::ProviderError);
        }
        assert!(validate_index(&index_with_path("src/main.rs"), 4).is_ok());
    }

    #[test]
    fn timeline_page_accepts_one_mib_fields_and_enforces_the_aggregate_cap() {
        let index = index_with_path("src/main.rs");
        let tool_event = SessionTimelineEvent {
            event_id: "event-1".to_string(),
            ordinal: 0,
            timestamp: Some(1),
            kind: SessionTimelineEventKind::ToolCall,
            quality: SessionCountQuality::Exact,
            turn_id: Some("turn-1".to_string()),
            turn_tool_summary: None,
            parent_event_id: None,
            text: None,
            tool: Some(crate::providers::SessionToolCallPreview {
                provider_call_id: "call-1".to_string(),
                native_name: "Write".to_string(),
                category: SessionToolCategory::FileEdit,
                status: SessionToolCallStatus::Completed,
                summary: "src/main.rs".to_string(),
                argument_preview: Some("a".repeat(timeline_tool_preview_bytes())),
                argument_truncated: false,
                result_preview: Some("r".repeat(timeline_tool_preview_bytes())),
                result_truncated: false,
                affected_paths: vec!["src/main.rs".to_string()],
            }),
            checkpoint: None,
            agent: None,
            settings: None,
        };
        assert!(validate_page_slice(
            &SessionTimelinePageSlice {
                events: vec![tool_event.clone()],
                next_index: None,
            },
            &index,
            1,
        )
        .is_ok());

        let mut oversized_tool = tool_event;
        oversized_tool.tool.as_mut().unwrap().argument_preview =
            Some("a".repeat(timeline_tool_preview_bytes() + 1));
        let error = validate_page_slice(
            &SessionTimelinePageSlice {
                events: vec![oversized_tool],
                next_index: None,
            },
            &index,
            1,
        )
        .unwrap_err();
        assert_eq!(error.code, SessionDetailErrorCode::ProviderError);

        let message = |event_id: &str, text: String| SessionTimelineEvent {
            event_id: event_id.to_string(),
            ordinal: 0,
            timestamp: None,
            kind: SessionTimelineEventKind::Prompt,
            quality: SessionCountQuality::Exact,
            turn_id: Some("turn-1".to_string()),
            turn_tool_summary: None,
            parent_event_id: None,
            text: Some(text),
            tool: None,
            checkpoint: None,
            agent: None,
            settings: None,
        };
        let error = validate_page_slice(
            &SessionTimelinePageSlice {
                events: vec![
                    message("message-1", "m".repeat(timeline_message_preview_bytes())),
                    message("message-2", "m".repeat(timeline_message_preview_bytes())),
                    message("message-3", "m".to_string()),
                ],
                next_index: None,
            },
            &index,
            3,
        )
        .unwrap_err();
        assert_eq!(error.code, SessionDetailErrorCode::ProviderError);
    }

    #[test]
    fn project_markdown_paths_are_existing_no_follow_descendants() {
        let sandbox = tempfile::tempdir().unwrap();
        let project = sandbox.path().join("project");
        let docs = project.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let story = docs.join("story.md");
        fs::write(&story, "story").unwrap();
        let canonical_project = fs::canonicalize(&project).unwrap();

        assert_eq!(
            canonical_project_markdown_path(&canonical_project, "docs/story.md").unwrap(),
            fs::canonicalize(&story).unwrap(),
        );
        assert_eq!(
            canonical_project_markdown_path(&canonical_project, "docs").unwrap(),
            fs::canonicalize(&docs).unwrap(),
        );
        assert_eq!(
            canonical_project_markdown_path(&canonical_project, "../outside.md")
                .unwrap_err()
                .code,
            SessionDetailErrorCode::InvalidRequest,
        );
        assert_eq!(
            canonical_project_markdown_path(&canonical_project, "docs/missing.md")
                .unwrap_err()
                .code,
            SessionDetailErrorCode::NotFound,
        );
    }

    #[cfg(unix)]
    #[test]
    fn project_markdown_paths_reject_symlinks_even_when_the_target_is_inside() {
        use std::os::unix::fs::symlink;

        let sandbox = tempfile::tempdir().unwrap();
        let project = sandbox.path().join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("story.md"), "story").unwrap();
        symlink(project.join("story.md"), project.join("story-link.md")).unwrap();
        let error =
            canonical_project_markdown_path(&fs::canonicalize(&project).unwrap(), "story-link.md")
                .unwrap_err();
        assert_eq!(error.code, SessionDetailErrorCode::InvalidRequest);
    }

    #[test]
    fn project_markdown_images_are_sniffed_bounded_regular_files() {
        let sandbox = tempfile::tempdir().unwrap();
        let project = sandbox.path().join("project");
        fs::create_dir_all(&project).unwrap();

        // Supported formats are recognized by magic bytes and returned as
        // base64 with the sniffed MIME type.
        let gif = project.join("animation.gif");
        fs::write(&gif, b"GIF89a\x01\x00\x01\x00\x00\x00\x00;").unwrap();
        let image = bounded_project_markdown_image(&gif).unwrap();
        assert_eq!(image.mime_type, "image/gif");
        assert_eq!(
            aws_smithy_types::base64::decode(&image.base64_data).unwrap(),
            fs::read(&gif).unwrap(),
        );
        let png = project.join("shot.png");
        fs::write(&png, b"\x89PNG\r\n\x1a\n0000").unwrap();
        assert_eq!(
            bounded_project_markdown_image(&png).unwrap().mime_type,
            "image/png"
        );
        let jpeg = project.join("photo.jpg");
        fs::write(&jpeg, b"\xFF\xD8\xFF\xE0rest").unwrap();
        assert_eq!(
            bounded_project_markdown_image(&jpeg).unwrap().mime_type,
            "image/jpeg"
        );
        let webp = project.join("frame.webp");
        fs::write(&webp, b"RIFF\x04\x00\x00\x00WEBPVP8 ").unwrap();
        assert_eq!(
            bounded_project_markdown_image(&webp).unwrap().mime_type,
            "image/webp"
        );

        // The declared extension is never trusted: non-image bytes are
        // rejected whatever the file is called.
        let fake = project.join("fake.gif");
        fs::write(&fake, b"#!/bin/sh\necho pwned\n").unwrap();
        assert_eq!(
            bounded_project_markdown_image(&fake).unwrap_err().code,
            SessionDetailErrorCode::InvalidRequest,
        );

        // Directories and missing files are typed, not embedded.
        assert_eq!(
            bounded_project_markdown_image(&project).unwrap_err().code,
            SessionDetailErrorCode::InvalidRequest,
        );
        assert_eq!(
            bounded_project_markdown_image(&project.join("missing.gif"))
                .unwrap_err()
                .code,
            SessionDetailErrorCode::NotFound,
        );

        // The size cap rejects before reading the bytes.
        let oversized = project.join("huge.gif");
        let file = fs::File::create(&oversized).unwrap();
        file.set_len(MAX_PROJECT_MARKDOWN_IMAGE_BYTES + 1).unwrap();
        drop(file);
        assert_eq!(
            bounded_project_markdown_image(&oversized).unwrap_err().code,
            SessionDetailErrorCode::ResponseTooLarge,
        );
    }

    #[test]
    fn project_markdown_media_are_sniffed_bounded_regular_files() {
        let sandbox = tempfile::tempdir().unwrap();
        let project = sandbox.path().join("project");
        fs::create_dir_all(&project).unwrap();

        // Each supported container is recognized by its magic bytes and
        // returned whole, unencoded.
        let samples: [(&str, &[u8]); 8] = [
            ("peaceful_music.wav", b"RIFF\x24\x00\x00\x00WAVEfmt "),
            ("song.mp3", b"ID3\x04\x00\x00\x00\x00\x00\x00"),
            ("frames.mp3", b"\xFF\xFB\x90\x64\x00"),
            ("voice.aac", b"\xFF\xF1\x50\x80\x00"),
            ("take.flac", b"fLaC\x00\x00\x00\x22"),
            ("loop.ogg", b"OggS\x00\x02\x00\x00"),
            ("clip.mp4", b"\x00\x00\x00\x18ftypmp42"),
            ("clip.webm", b"\x1A\x45\xDF\xA3\x9F\x42\x86\x81"),
        ];
        for (name, bytes) in samples {
            let path = project.join(name);
            fs::write(&path, bytes).unwrap();
            assert_eq!(
                bounded_project_markdown_media(&path).unwrap(),
                bytes,
                "{name}"
            );
        }

        // The declared extension is never trusted: a script called .wav and a
        // picture called .mp4 are not media.
        let fake = project.join("fake.wav");
        fs::write(&fake, b"#!/bin/sh\necho pwned\n").unwrap();
        assert_eq!(
            bounded_project_markdown_media(&fake).unwrap_err().code,
            SessionDetailErrorCode::InvalidRequest,
        );
        let picture = project.join("picture.mp4");
        fs::write(&picture, b"\x89PNG\r\n\x1a\n0000").unwrap();
        assert_eq!(
            bounded_project_markdown_media(&picture).unwrap_err().code,
            SessionDetailErrorCode::InvalidRequest,
        );

        // Directories and missing files are typed, not played.
        assert_eq!(
            bounded_project_markdown_media(&project).unwrap_err().code,
            SessionDetailErrorCode::InvalidRequest,
        );
        assert_eq!(
            bounded_project_markdown_media(&project.join("missing.wav"))
                .unwrap_err()
                .code,
            SessionDetailErrorCode::NotFound,
        );

        // The size cap rejects before reading the bytes.
        let oversized = project.join("long.mp4");
        let file = fs::File::create(&oversized).unwrap();
        file.set_len(MAX_PROJECT_MARKDOWN_MEDIA_BYTES + 1).unwrap();
        drop(file);
        assert_eq!(
            bounded_project_markdown_media(&oversized).unwrap_err().code,
            SessionDetailErrorCode::ResponseTooLarge,
        );
    }

    #[test]
    fn truncated_timeline_does_not_invent_an_event_limit_warning() {
        let mut index = index_with_path("src/main.rs");
        index.truncated = true;
        index.warnings.push(SessionDetailWarning {
            code: "oversized_record".to_string(),
            message: "A provider record exceeded its parsing limit.".to_string(),
            severity: NoticeSeverity::Warning,
        });

        let warnings = timeline_warnings(&index, Vec::new());
        assert_eq!(warnings, index.warnings);
        assert!(!warnings
            .iter()
            .any(|warning| warning.code == "timeline_truncated"));
    }

    #[test]
    fn opaque_cursor_round_trips_bound_coordinates() {
        let cursor = TimelineCursor {
            schema: 1,
            provider: ProviderKey::parse("codex").unwrap(),
            resource_id: "codex:conversation:thread-1".to_string(),
            snapshot_id: "a".repeat(64),
            filter_fingerprint: "b".repeat(64),
            next_index: 49,
        };
        let encoded = encode_cursor(&cursor).unwrap();
        assert!(encoded.starts_with("v1-"));
        assert_eq!(decode_cursor(&encoded).unwrap(), cursor);
        assert!(decode_cursor("v1-not-hex").is_err());
    }

    #[test]
    fn snapshot_changed_error_is_structured() {
        let error = SessionDetailError::snapshot_changed("current".to_string());
        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["code"], "snapshot_changed");
        assert_eq!(value["current_snapshot_id"], "current");
    }

    #[test]
    fn timeline_page_rejects_a_snapshot_captured_before_transcript_append() {
        let sandbox = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(sandbox.path()).unwrap();
        let provider_home = sandbox.path().join("test-provider-home");
        let sessions_root = provider_home.join("sessions");
        let project_root = sandbox.path().join("project");
        fs::create_dir_all(&sessions_root).unwrap();
        fs::create_dir_all(&project_root).unwrap();

        let provider =
            ProviderKey::parse(crate::providers::test_provider::TEST_PROVIDER_KEY).unwrap();
        let profile_id = LocalProviderProfileId::parse("session-detail-profile").unwrap();
        let local_project_id = LocalProjectId::parse("session-detail-project").unwrap();
        let bundle_id = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let identity = repository.load_or_create_host_identity().unwrap();
        let canonical_project_root = fs::canonicalize(&project_root).unwrap();
        let canonical_provider_home = fs::canonicalize(&provider_home).unwrap();
        let project_binding = ProjectBinding {
            host_id: identity.host_id.clone(),
            replica_id: ReplicaId::parse("session-detail-replica").unwrap(),
            local_project_id: local_project_id.clone(),
            bundle_id: bundle_id.clone(),
            project_root: project_root.to_string_lossy().into_owned(),
            canonical_project_root: canonical_project_root.to_string_lossy().into_owned(),
            profile_ids: BTreeMap::from([(provider.clone(), profile_id.clone())]),
            state: BindingState::Active,
            revision: 0,
            updated_at: 1,
        };
        repository
            .mutate_config(|config| {
                config.projects.push(LocalProjectRegistration {
                    local_project_id: local_project_id.clone(),
                    bundle_id: bundle_id.clone(),
                    display_name: "Session Detail project".to_string(),
                    local_alias: None,
                    repository_fingerprint: None,
                    recipe: BundleRecipe::default(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                Ok(())
            })
            .unwrap();
        repository
            .mutate_bindings(|_, machine| {
                machine.profiles.push(ProviderProfile {
                    profile_id: profile_id.clone(),
                    provider: provider.clone(),
                    display_name: "Test Provider".to_string(),
                    path: provider_home.to_string_lossy().into_owned(),
                    canonical_path: canonical_provider_home.to_string_lossy().into_owned(),
                    provider_config: mallard_domain::AgentProviderConfig::version_1(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                machine.bindings.push(project_binding);
                Ok(())
            })
            .unwrap();

        let provider_session_id = "session-1";
        let transcript = sessions_root.join(format!("{provider_session_id}.jsonl"));
        let initial_records = [
            serde_json::json!({
                "type": "user",
                "uuid": "user-1",
                "sessionId": provider_session_id,
                "cwd": canonical_project_root,
                "timestamp": "2026-08-21T00:00:00Z",
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "First prompt"}],
                },
            }),
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-1",
                "sessionId": provider_session_id,
                "cwd": canonical_project_root,
                "timestamp": "2026-08-21T00:00:01Z",
                "message": {
                    "id": "message-1",
                    "role": "assistant",
                    "stop_reason": "end_turn",
                    "content": [{"type": "text", "text": "First response"}],
                },
            }),
        ];
        let mut transcript_bytes = Vec::new();
        for record in initial_records {
            serde_json::to_writer(&mut transcript_bytes, &record).unwrap();
            transcript_bytes.push(b'\n');
        }
        fs::write(&transcript, &transcript_bytes).unwrap();

        let transcript_metadata = fs::metadata(&transcript).unwrap();
        let thread_id =
            initial_conversation_thread_id(&bundle_id, &provider, provider_session_id).unwrap();
        let resource_id = conversation_resource_id(&provider, &thread_id).unwrap();
        let views = vec![LocalSessionView {
            descriptor: crate::provider_capture::ResourceCandidate {
                resource_id: resource_id.to_string(),
                kind: mallard_domain::portable::contracts::conversation(),
                provider: Some(provider.clone()),
                provider_resource_type: Some("session".to_string()),
                display_name: "Session one".to_string(),
                apply_policy: mallard_domain::portable::contracts::safe_file(),
                selected_by_default: true,
                blocked_reason: None,
                logical_paths: vec!["sessions/session-1.jsonl".to_string()],
                project_content: None,
            },
            provider: provider.clone(),
            provider_thread_id: provider_session_id.to_string(),
            transcript_logical_path: "sessions/session-1.jsonl".to_string(),
            source_path: transcript.clone(),
            source_mtime: modified_secs(&transcript_metadata),
            source_size: transcript_metadata.len(),
            capture_bytes: transcript_metadata.len(),
        }];
        let registry = AgentProviderRegistry::with_test_provider();
        let before = get_project_session_detail(
            &repository,
            &registry,
            &local_project_id,
            resource_id.as_str(),
            &views,
        )
        .unwrap();
        let before_snapshot_id = before.snapshot_id.clone();

        for record in [
            serde_json::json!({
                "type": "user",
                "uuid": "user-2",
                "sessionId": provider_session_id,
                "cwd": canonical_project_root,
                "timestamp": "2026-08-21T00:00:02Z",
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Second prompt"}],
                },
            }),
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-2",
                "sessionId": provider_session_id,
                "cwd": canonical_project_root,
                "timestamp": "2026-08-21T00:00:03Z",
                "message": {
                    "id": "message-2",
                    "role": "assistant",
                    "stop_reason": "end_turn",
                    "content": [{"type": "text", "text": "Second response"}],
                },
            }),
        ] {
            serde_json::to_writer(&mut transcript_bytes, &record).unwrap();
            transcript_bytes.push(b'\n');
        }
        fs::write(&transcript, &transcript_bytes).unwrap();

        let error = get_project_session_timeline_page(
            &repository,
            &registry,
            &local_project_id,
            resource_id.as_str(),
            ProjectSessionTimelineRequest {
                snapshot_id: before_snapshot_id.clone(),
                cursor: None,
                limit: Some(10),
                included_event_kinds: BTreeSet::from([
                    SessionTimelineEventKind::Prompt,
                    SessionTimelineEventKind::Response,
                ]),
                included_tool_categories: BTreeSet::new(),
                turn_id: None,
            },
            &views,
        )
        .unwrap_err();
        assert_eq!(error.code, SessionDetailErrorCode::SnapshotChanged);

        let after = get_project_session_detail(
            &repository,
            &registry,
            &local_project_id,
            resource_id.as_str(),
            &views,
        )
        .unwrap();
        assert_ne!(after.snapshot_id, before_snapshot_id);
        assert_eq!(error.current_snapshot_id, Some(after.snapshot_id));
    }

    #[test]
    fn metadata_index_serialization_contains_no_message_body_fields() {
        let encoded = serde_json::to_string(&index_with_path("src/main.rs")).unwrap();
        assert!(!encoded.contains("argument_preview"));
        assert!(!encoded.contains("result_preview"));
        assert!(!encoded.contains("\"text\""));
    }

    #[test]
    fn identical_thread_ids_remain_provider_scoped_resources() {
        let thread = mallard_domain::ThreadId::parse("same-native-id").unwrap();
        let codex = mallard_domain::conversation_resource_id(
            &ProviderKey::parse("codex").unwrap(),
            &thread,
        )
        .unwrap();
        let claude = mallard_domain::conversation_resource_id(
            &ProviderKey::parse("claude").unwrap(),
            &thread,
        )
        .unwrap();
        assert_ne!(codex, claude);
        assert_eq!(
            parse_conversation_resource_id(codex.as_str()).unwrap().0,
            ProviderKey::parse("codex").unwrap()
        );
        assert_eq!(
            parse_conversation_resource_id(claude.as_str()).unwrap().0,
            ProviderKey::parse("claude").unwrap()
        );
    }

    #[test]
    fn agent_transcripts_open_as_child_pages_through_their_parent() {
        let sandbox = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(sandbox.path()).unwrap();
        let provider_home = sandbox.path().join("test-provider-home");
        let sessions_root = provider_home.join("sessions");
        let project_root = sandbox.path().join("project");
        fs::create_dir_all(sessions_root.join("session-1/subagents")).unwrap();
        fs::create_dir_all(&project_root).unwrap();

        let provider =
            ProviderKey::parse(crate::providers::test_provider::TEST_PROVIDER_KEY).unwrap();
        let profile_id = LocalProviderProfileId::parse("session-detail-profile").unwrap();
        let local_project_id = LocalProjectId::parse("session-detail-project").unwrap();
        let bundle_id = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let identity = repository.load_or_create_host_identity().unwrap();
        let canonical_project_root = fs::canonicalize(&project_root).unwrap();
        let canonical_provider_home = fs::canonicalize(&provider_home).unwrap();
        let project_binding = ProjectBinding {
            host_id: identity.host_id.clone(),
            replica_id: ReplicaId::parse("session-detail-replica").unwrap(),
            local_project_id: local_project_id.clone(),
            bundle_id: bundle_id.clone(),
            project_root: project_root.to_string_lossy().into_owned(),
            canonical_project_root: canonical_project_root.to_string_lossy().into_owned(),
            profile_ids: BTreeMap::from([(provider.clone(), profile_id.clone())]),
            state: BindingState::Active,
            revision: 0,
            updated_at: 1,
        };
        repository
            .mutate_config(|config| {
                config.projects.push(LocalProjectRegistration {
                    local_project_id: local_project_id.clone(),
                    bundle_id: bundle_id.clone(),
                    display_name: "Session Detail project".to_string(),
                    local_alias: None,
                    repository_fingerprint: None,
                    recipe: BundleRecipe::default(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                Ok(())
            })
            .unwrap();
        repository
            .mutate_bindings(|_, machine| {
                machine.profiles.push(ProviderProfile {
                    profile_id: profile_id.clone(),
                    provider: provider.clone(),
                    display_name: "Test Provider".to_string(),
                    path: provider_home.to_string_lossy().into_owned(),
                    canonical_path: canonical_provider_home.to_string_lossy().into_owned(),
                    provider_config: mallard_domain::AgentProviderConfig::version_1(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                machine.bindings.push(project_binding);
                Ok(())
            })
            .unwrap();

        let provider_session_id = "session-1";
        let transcript = sessions_root.join("session-1.jsonl");
        let child = sessions_root.join("session-1/subagents/agent-a1b2c3.jsonl");
        let write = |path: &Path, records: Vec<serde_json::Value>| {
            let mut bytes = Vec::new();
            for record in records {
                serde_json::to_writer(&mut bytes, &record).unwrap();
                bytes.push(b'\n');
            }
            fs::write(path, bytes).unwrap();
        };
        write(
            &transcript,
            vec![
                serde_json::json!({"type":"user","uuid":"u-1","sessionId":provider_session_id,"cwd":canonical_project_root,"timestamp":"2026-08-27T06:00:00Z","message":{"role":"user","content":"Research the term marking"}}),
                serde_json::json!({"type":"assistant","uuid":"a-1","sessionId":provider_session_id,"cwd":canonical_project_root,"timestamp":"2026-08-27T06:00:05Z","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"tool_use","id":"task-1","name":"Task","input":{"description":"Research term marking","subagent_type":"general-purpose","prompt":"Research how reading apps mark a target word."}}
                ]}}),
                serde_json::json!({"type":"user","uuid":"r-1","sessionId":provider_session_id,"cwd":canonical_project_root,"timestamp":"2026-08-27T06:00:09Z","toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a1b2c3","description":"Research term marking","resolvedModel":"claude-opus-5"},"message":{"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"task-1","content":[{"type":"text","text":"Async agent launched."}]}
                ]}}),
                serde_json::json!({"type":"assistant","uuid":"a-2","sessionId":provider_session_id,"cwd":canonical_project_root,"timestamp":"2026-08-27T06:14:41Z","message":{"id":"msg-2","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"The reports agree."}]}}),
            ],
        );
        write(
            &child,
            vec![
                serde_json::json!({"type":"user","uuid":"c-1","parentUuid":null,"isSidechain":true,"sessionId":provider_session_id,"agentId":"a1b2c3","cwd":canonical_project_root,"timestamp":"2026-08-27T06:00:09Z","message":{"role":"user","content":"Research how reading apps mark a target word."}}),
                serde_json::json!({"type":"assistant","uuid":"c-2","parentUuid":"c-1","isSidechain":true,"sessionId":provider_session_id,"agentId":"a1b2c3","cwd":canonical_project_root,"timestamp":"2026-08-27T06:10:36Z","message":{"id":"cm-1","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"I have enough. Here are the findings."}]}}),
            ],
        );

        let transcript_metadata = fs::metadata(&transcript).unwrap();
        let thread_id =
            initial_conversation_thread_id(&bundle_id, &provider, provider_session_id).unwrap();
        let resource_id = conversation_resource_id(&provider, &thread_id).unwrap();
        let views = vec![LocalSessionView {
            descriptor: crate::provider_capture::ResourceCandidate {
                resource_id: resource_id.to_string(),
                kind: mallard_domain::portable::contracts::conversation(),
                provider: Some(provider.clone()),
                provider_resource_type: Some("session".to_string()),
                display_name: "Session one".to_string(),
                apply_policy: mallard_domain::portable::contracts::safe_file(),
                selected_by_default: true,
                blocked_reason: None,
                logical_paths: vec!["sessions/session-1.jsonl".to_string()],
                project_content: None,
            },
            provider: provider.clone(),
            provider_thread_id: provider_session_id.to_string(),
            transcript_logical_path: "sessions/session-1.jsonl".to_string(),
            source_path: transcript.clone(),
            source_mtime: modified_secs(&transcript_metadata),
            source_size: transcript_metadata.len(),
            capture_bytes: transcript_metadata.len(),
        }];
        let registry = AgentProviderRegistry::with_test_provider();

        let parent = get_project_session_detail(
            &repository,
            &registry,
            &local_project_id,
            resource_id.as_str(),
            &views,
        )
        .unwrap();
        assert_eq!(parent.event_counts.agent_calls.value, Some(1));
        assert!(parent.agent.is_none());
        let page = get_project_session_timeline_page(
            &repository,
            &registry,
            &local_project_id,
            resource_id.as_str(),
            ProjectSessionTimelineRequest {
                snapshot_id: parent.snapshot_id.clone(),
                cursor: None,
                limit: None,
                included_event_kinds: BTreeSet::from([SessionTimelineEventKind::AgentCall]),
                included_tool_categories: BTreeSet::new(),
                turn_id: None,
            },
            &views,
        )
        .unwrap();
        assert_eq!(page.events.len(), 1);
        let row = page.events[0].agent.clone().unwrap();
        let child_resource_id = format!("{resource_id}/agent/a1b2c3");
        assert_eq!(
            row.transcript_resource_id.as_deref(),
            Some(child_resource_id.as_str())
        );
        assert_eq!(row.status, crate::providers::SessionAgentStatus::Completed);
        // The row's report is the child's, and no provider path leaks in it.
        assert_eq!(
            row.report.as_deref(),
            Some("I have enough. Here are the findings.")
        );

        let detail = get_project_session_detail(
            &repository,
            &registry,
            &local_project_id,
            &child_resource_id,
            &views,
        )
        .unwrap();
        assert_eq!(detail.resource_id, child_resource_id);
        assert_eq!(detail.provider_session_id, "a1b2c3");
        assert_eq!(detail.title, "Research term marking");
        assert!(detail.actions.is_empty());
        let agent = detail.agent.clone().unwrap();
        assert_eq!(agent.parent_resource_id, resource_id.as_str());
        assert_eq!(agent.parent_title, "Session one");
        assert_eq!(agent.preview.role.as_deref(), Some("general-purpose"));
        assert_eq!(detail.event_counts.prompts.value, Some(1));
        assert_eq!(detail.event_counts.responses.value, Some(1));
        let child_page = get_project_session_timeline_page(
            &repository,
            &registry,
            &local_project_id,
            &child_resource_id,
            ProjectSessionTimelineRequest {
                snapshot_id: detail.snapshot_id.clone(),
                cursor: None,
                limit: None,
                included_event_kinds: BTreeSet::from([
                    SessionTimelineEventKind::Prompt,
                    SessionTimelineEventKind::Response,
                    SessionTimelineEventKind::Checkpoint,
                ]),
                included_tool_categories: BTreeSet::new(),
                turn_id: None,
            },
            &views,
        )
        .unwrap();
        assert_eq!(child_page.events.len(), 2);
        assert_eq!(
            child_page.events[0].text.as_deref(),
            Some("Research how reading apps mark a target word.")
        );

        // A hop the parent never recorded has no file: not local. A hop that
        // is not an identity is refused before any file is touched.
        assert_eq!(
            get_project_session_detail(
                &repository,
                &registry,
                &local_project_id,
                &format!("{resource_id}/agent/missing1"),
                &views,
            )
            .unwrap_err()
            .code,
            SessionDetailErrorCode::NotLocal
        );
        assert_eq!(
            get_project_session_detail(
                &repository,
                &registry,
                &local_project_id,
                &format!("{resource_id}/agent/../escape"),
                &views,
            )
            .unwrap_err()
            .code,
            SessionDetailErrorCode::InvalidRequest
        );
    }
}
