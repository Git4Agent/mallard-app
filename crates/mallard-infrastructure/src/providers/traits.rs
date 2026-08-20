//! Typed adapter contracts for built-in agent providers.
//!
//! Ownership boundary: adapters parse provider files and construct provider
//! targets; the core validates plans, digests, revisions, approvals, and
//! paths; Tauri commands orchestrate but hold no provider format logic; the
//! frontend renders returned descriptors and actions but never constructs
//! provider commands.

use crate::conversation_codec::CodecLineage;
use mallard_domain::{ConversationFileRole, ProviderKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Maximum UTF-8 bytes returned for either the arguments or result preview of
/// one timeline tool call. A tool event can contain both fields, so 8 KiB per
/// field keeps its total preview budget aligned with a 16-KiB message event.
pub(crate) const TIMELINE_TOOL_PREVIEW_BYTES: usize = 8 * 1024;

/// Typed capability ladder. Levels are ordered: each level includes every
/// weaker level's abilities.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapabilityLevel {
    /// Local/remote metadata may be displayed; no payload sync.
    ActivityOnly,
    /// Selected local sessions may be stored, but Pull cannot make them
    /// resumable.
    BackupOnly,
    /// Create and proven fast-forward operations; divergence is blocked.
    Resumable,
    /// The complete decision matrix, including conflict preservation.
    ResumableWithConflictPreservation,
}

/// Serializable provider facts for setup and Activity. The frontend iterates
/// these; it never holds a fixed provider array.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub key: ProviderKey,
    pub display_name: String,
    /// Short label for the profile picker, e.g. ".codex profile".
    pub profile_label: String,
    /// Home-relative default directory name, e.g. ".codex".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile_directory: Option<String>,
    /// May pre-enable this provider for a new draft; never a requirement.
    pub setup_default: bool,
    pub capabilities: ProviderCapabilityLevel,
    pub adapter_version: u32,
}

/// Bounded metadata scanned from the head of one provider session file.
#[derive(Clone, Debug)]
pub struct SessionFileMetadata {
    pub session_id: String,
    pub cwd: PathBuf,
    pub is_internal_subagent: bool,
}

/// Normalized typed-codec output the core reconciliation engine consumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAnalysis {
    pub source_provider_session_id: Option<String>,
    pub source_cwd: Option<String>,
    pub is_internal_subagent: bool,
    pub record_count: usize,
    pub lineage: CodecLineage,
}

impl SessionAnalysis {
    pub fn is_resumable(&self) -> bool {
        self.source_provider_session_id.is_some()
            && self.source_cwd.is_some()
            && self.lineage.is_complete()
    }
}

/// A structural rewrite of declared session-identity and cwd fields only.
/// Adapters must round-trip the rewritten transcript through their own codec
/// and prove unchanged lineage and semantic digests before returning.
#[derive(Clone, Debug)]
pub struct SessionRewriteRequest<'a> {
    pub bytes: &'a [u8],
    pub expected_source_session_id: Option<&'a str>,
    pub destination_session_id: &'a str,
    pub expected_source_cwd: Option<&'a str>,
    pub destination_cwd: &'a str,
}

pub struct SessionDiscoveryContext<'a> {
    /// Canonicalized provider home for the selected profile.
    pub provider_home: &'a Path,
    /// Canonicalized project root.
    pub project_root: &'a Path,
    /// Canonical nested registrations owned by other bundles.
    pub excluded_roots: &'a [PathBuf],
}

#[derive(Clone, Debug)]
pub struct DiscoveredSession {
    pub session_id: String,
    pub relative_cwd: String,
    pub physical_path: PathBuf,
    pub display_name: String,
    pub blocked_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DiscoveredSidecar {
    pub session_id: String,
    pub sidecar_kind: String,
    pub file_name: String,
    pub physical_path: PathBuf,
    pub blocked_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DiscoveredStateFile {
    pub physical_path: PathBuf,
    /// Path suffix under `state/providers/<key>/`.
    pub logical_suffix: String,
}

/// Provider state not owned by one conversation (for example memory).
#[derive(Clone, Debug)]
pub struct DiscoveredProviderState {
    /// Stable resource-id component for this state group.
    pub state_component: String,
    /// Presentation subtype recorded as `provider_resource_type`.
    pub resource_type: String,
    pub display_name: String,
    pub relative_cwd: Option<String>,
    /// Merge-applied state (memory) versus safe-file state.
    pub merge_apply: bool,
    pub files: Vec<DiscoveredStateFile>,
    pub blocked_reason: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionDiscovery {
    pub sessions: Vec<DiscoveredSession>,
    pub sidecars: Vec<DiscoveredSidecar>,
    pub state_resources: Vec<DiscoveredProviderState>,
    pub warnings: Vec<String>,
    pub excluded_subagent_session_ids: BTreeSet<String>,
    /// Provider-native session identities observed by the adapter's one
    /// profile-wide discovery walk, before project ownership filtering.
    pub profile_session_ids: BTreeSet<String>,
}

/// Bounded per-file summary for Activity. Fields default to unknown; the
/// adapter fills what its format proves. Serialization exists only for the
/// machine-local activity cache.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionFileSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub branch: Option<String>,
    pub recorded_sha: Option<String>,
    pub recorded_branches: BTreeSet<String>,
    pub user_rounds: u64,
    pub agent_messages: u64,
    pub tool_calls: u64,
    pub total_tokens: Option<u64>,
    pub metrics_complete: bool,
    pub is_internal_subagent: bool,
}

/// Confidence attached to a normalized count or event. Providers must not
/// promote incomplete or inferred evidence to `Exact`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SessionCountQuality {
    Exact,
    LowerBound,
    Inferred,
    Unsupported,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionCount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    pub quality: SessionCountQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

impl SessionCount {
    pub fn exact(value: u64) -> Self {
        Self {
            value: Some(value),
            quality: SessionCountQuality::Exact,
            explanation: None,
        }
    }

    pub fn unsupported(explanation: impl Into<String>) -> Self {
        Self {
            value: None,
            quality: SessionCountQuality::Unsupported,
            explanation: Some(explanation.into()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionEventCounts {
    pub prompts: SessionCount,
    pub responses: SessionCount,
    pub intermediate_steps: SessionCount,
    pub checkpoints: SessionCount,
    pub tool_calls: SessionCount,
    pub file_edits: SessionCount,
    pub shell_calls: SessionCount,
    pub reads: SessionCount,
    pub other_tools: SessionCount,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SessionTimelineEventKind {
    Prompt,
    Response,
    IntermediateStep,
    Checkpoint,
    ToolCall,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SessionToolCategory {
    FileEdit,
    Shell,
    Read,
    Other,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionToolCallStatus {
    Completed,
    Failed,
    Incomplete,
}

/// A byte range in the provider transcript. Ranges are cache-safe metadata;
/// their referenced text is read only for the requested page or event.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSourceSpan {
    pub offset: u64,
    pub length: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionToolIndexEntry {
    pub provider_call_id: String,
    pub native_name: String,
    pub category: SessionToolCategory,
    pub status: SessionToolCallStatus,
    /// Structurally known, sanitized paths relative to the project root.
    /// Absolute provider paths must never enter the index or cross Tauri.
    #[serde(default)]
    pub affected_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_source: Option<SessionSourceSpan>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionTurnToolSummary {
    pub turn_id: String,
    pub tool_calls: SessionCount,
}

/// Metadata-only index entry. Visible message text, tool arguments, tool
/// results, and file contents must never be stored in this structure.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionTimelineIndexEntry {
    pub event_id: String,
    pub ordinal: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    pub kind: SessionTimelineEventKind,
    pub quality: SessionCountQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_tool_summary: Option<SessionTurnToolSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    pub source: SessionSourceSpan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<SessionToolIndexEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionDetailWarning {
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionTimelineIndex {
    pub parser_version: u32,
    pub events: Vec<SessionTimelineIndexEntry>,
    pub counts: SessionEventCounts,
    #[serde(default)]
    pub warnings: Vec<SessionDetailWarning>,
    pub truncated: bool,
}

pub struct SessionTimelineContext<'a> {
    pub resource_id: &'a str,
    pub provider_session_id: &'a str,
    pub project_root: &'a Path,
    /// Other validated spellings of the same bound root, such as macOS's
    /// `/var` and `/private/var` aliases. Provider transcripts may retain the
    /// configured spelling even though ownership checks use the canonical one.
    pub project_root_aliases: &'a [PathBuf],
}

/// Strip the most specific validated spelling of a bound project root.
/// Choosing by component depth prevents a selected symlink spelling nested
/// beneath its canonical target from leaking the alias component into the
/// user-facing relative path.
pub(crate) fn most_specific_project_relative_path<'a>(
    candidate: &'a Path,
    project_root: &Path,
    project_root_aliases: &[PathBuf],
) -> Option<&'a Path> {
    let mut best: Option<((usize, usize), &'a Path)> = None;
    for root in project_root_aliases
        .iter()
        .map(PathBuf::as_path)
        .chain(std::iter::once(project_root))
    {
        let Ok(relative) = candidate.strip_prefix(root) else {
            continue;
        };
        let specificity = (root.components().count(), root.as_os_str().len());
        if best
            .as_ref()
            .is_none_or(|(current, _)| specificity > *current)
        {
            best = Some((specificity, relative));
        }
    }
    best.map(|(_, relative)| relative)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTimelineQuery {
    /// Index position after common-layer cursor validation. `None` starts at
    /// the earliest matching event.
    pub start_index: Option<usize>,
    pub anchor_event_id: Option<String>,
    pub limit: usize,
    pub included_event_kinds: BTreeSet<SessionTimelineEventKind>,
    pub included_tool_categories: BTreeSet<SessionToolCategory>,
    pub turn_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionToolCallPreview {
    pub provider_call_id: String,
    pub native_name: String,
    pub category: SessionToolCategory,
    pub status: SessionToolCallStatus,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_preview: Option<String>,
    pub argument_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    pub result_truncated: bool,
    #[serde(default)]
    pub affected_paths: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionCheckpoint {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub committed_at: u64,
    pub branch: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionTimelineEvent {
    pub event_id: String,
    pub ordinal: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    pub kind: SessionTimelineEventKind,
    pub quality: SessionCountQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_tool_summary: Option<SessionTurnToolSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<SessionToolCallPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<SessionCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTimelinePageSlice {
    pub events: Vec<SessionTimelineEvent>,
    /// Provider-index position for the next matching scan, not a public
    /// cursor. The common layer seals it into an opaque cursor.
    pub next_index: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionEventDetail {
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    pub arguments_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub result_truncated: bool,
    #[serde(default)]
    pub affected_paths: Vec<String>,
}

/// Where a planned pull materialization writes inside the provider home. The
/// adapter proposes provider-relative targets; the core performs the final
/// canonical-boundary, no-follow, special-file, collision, size, and digest
/// checks.
#[derive(Clone, Debug)]
pub struct PlannedProviderTarget {
    pub target_path: PathBuf,
    pub role: ConversationFileRole,
}

pub struct MaterializationTargetRequest<'a> {
    /// Canonicalized provider home for the selected profile.
    pub provider_home: &'a Path,
    /// The user-selected project-root spelling used for provider cwd fields.
    pub project_root: &'a str,
    /// Destination provider session id.
    pub provider_session_id: &'a str,
    /// The portable source provider session id, when it differs from the
    /// destination (sidecar names embed it).
    pub source_provider_session_id: Option<&'a str>,
    /// The already-materialized transcript path when replacing in place.
    pub existing_transcript_path: Option<&'a Path>,
    /// Sidecar files travelling with the conversation:
    /// `(kind, kind-relative path)`.
    pub sidecars: &'a [(String, String)],
    /// Timestamp used for deterministic dated paths.
    pub planned_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionActionKind {
    OpenInApp,
    OpenInTerminal,
}

/// A safe provider action offered to the frontend. The frontend sends the
/// opaque `action_id` and session identity back to a generic command; the
/// backend re-resolves the registered adapter and validates ownership before
/// executing. Remote manifests never supply executable command text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionActionDescriptor {
    pub action_id: String,
    pub kind: SessionActionKind,
    pub label: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

pub struct SessionActionContext<'a> {
    pub provider_session_id: &'a str,
    pub is_local: bool,
    pub transcript_path: Option<&'a Path>,
}

pub struct SessionActionRequest<'a> {
    pub action_id: &'a str,
    pub provider_home: &'a Path,
    pub project_root: &'a Path,
    pub provider_session_id: &'a str,
    pub transcript_path: Option<&'a Path>,
    /// Observer for the exact shell command before it is launched, so the
    /// activity log records what ran.
    pub before_launch: &'a mut dyn FnMut(&str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionActionOutcome {
    Launched,
}

/// One built-in provider. `sessions()` is `None` for providers that manage
/// no session files (activity-only integrations).
pub trait AgentProviderAdapter: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn sessions(&self) -> Option<&dyn SessionAdapter>;
}

pub trait SessionAdapter: Send + Sync {
    fn codec_key(&self) -> &str;
    fn codec_version(&self) -> u32;
    /// Version of the normalized timeline projection. Changing event
    /// semantics or indexed metadata must increment this value so cached
    /// indexes and snapshot cursors invalidate safely.
    fn timeline_version(&self) -> u32 {
        self.codec_version()
    }
    fn capability(&self) -> ProviderCapabilityLevel;
    /// How many provider index fragments one conversation operation may
    /// carry (Codex maintains `session_index.jsonl`; most providers none).
    fn index_fragment_limit(&self) -> usize {
        0
    }
    /// Whether this provider's transcripts support checkpoint-cursor
    /// observation (probe-window plus append-only incremental fold).
    /// Cursor-less providers are captured up front and their tail evidence
    /// is synthesized from the captured lineage.
    fn supports_tail_cursor(&self) -> bool {
        false
    }
    /// Discover project-owned sessions, sidecars, and provider state under
    /// one provider home. Ownership is proven through each session's
    /// recorded cwd; the adapter must not scan outside `provider_home`.
    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String>;
    /// Bounded head-of-file metadata scan.
    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String>;
    /// Full typed-codec analysis of one transcript.
    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis;
    /// Structural rewrite of declared identity/cwd fields, round-tripped
    /// through the same codec with proven digests.
    fn rewrite(&self, request: &SessionRewriteRequest<'_>) -> Result<Vec<u8>, String>;
    /// Verify one staged provider index fragment. Only meaningful when
    /// `index_fragment_limit() > 0`.
    fn verify_index_fragment(
        &self,
        _bytes: &[u8],
        _expected_session_id: &str,
    ) -> Result<(), String> {
        Err("provider has no index fragments".to_string())
    }
    /// Propose materialization targets for one conversation.
    fn materialization_targets(
        &self,
        request: &MaterializationTargetRequest<'_>,
    ) -> Result<Vec<PlannedProviderTarget>, String>;
    /// Map one provider-state logical suffix (under
    /// `state/providers/<key>/`) to its machine path. Providers without
    /// provider-state resources refuse.
    fn state_target(
        &self,
        _provider_home: &Path,
        _project_root: &str,
        _relative_cwd: Option<&str>,
        _logical_suffix: &str,
    ) -> Result<PathBuf, String> {
        Err("provider has no provider-state resources".to_string())
    }
    /// Bounded full-file summary for Activity.
    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String>;
    /// Summarize an already captured transcript snapshot. Capture callers
    /// must use this instead of reopening the mutable source path.
    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String>;
    /// Build a metadata-only, bounded rich-timeline index. Implementations
    /// must not retain visible message text, tool arguments, or tool output.
    fn build_timeline_index(
        &self,
        path: &Path,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelineIndex, String>;
    /// Read and normalize one filtered page from a previously validated
    /// index. Filtering happens before pagination.
    fn read_timeline_page(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        query: &SessionTimelineQuery,
    ) -> Result<SessionTimelinePageSlice, String>;
    /// Read one bounded expanded event body on demand.
    fn read_event_detail(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        event_id: &str,
    ) -> Result<SessionEventDetail, String>;
    fn available_actions(&self, context: &SessionActionContext<'_>)
        -> Vec<SessionActionDescriptor>;
    fn execute_action(
        &self,
        request: &mut SessionActionRequest<'_>,
    ) -> Result<SessionActionOutcome, String>;
}
