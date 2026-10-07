//! Typed adapter contracts for built-in agent providers.
//!
//! Ownership boundary: adapters parse provider files and construct provider
//! targets; the core validates plans, digests, revisions, approvals, and
//! paths; Tauri commands orchestrate but hold no provider format logic; the
//! frontend renders returned descriptors and actions but never constructs
//! provider commands.

use crate::cli_discovery::ResolvedCli;
use crate::conversation_codec::CodecLineage;
use mallard_domain::{ConversationFileRole, NoticeSeverity, ProviderKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Split the expanded-detail budget (the installed preview limit) evenly,
/// then lend any unused share from a shorter field to the longer one. A lone
/// field can therefore use the whole budget without allowing the pair to
/// exceed it.
pub(crate) fn timeline_tool_detail_budgets(
    arguments: Option<&str>,
    result: Option<&str>,
) -> (usize, usize) {
    let detail_budget = crate::app_settings::timeline_tool_detail_bytes();
    let argument_len = arguments.map_or(0, str::len);
    let result_len = result.map_or(0, str::len);
    if argument_len.saturating_add(result_len) <= detail_budget {
        return (argument_len, result_len);
    }

    let half = detail_budget / 2;
    let mut argument_budget = argument_len.min(half);
    let mut result_budget = result_len.min(half);
    let mut remaining = detail_budget
        .saturating_sub(argument_budget)
        .saturating_sub(result_budget);

    let argument_extra = argument_len.saturating_sub(argument_budget).min(remaining);
    argument_budget += argument_extra;
    remaining -= argument_extra;
    result_budget += result_len.saturating_sub(result_budget).min(remaining);
    (argument_budget, result_budget)
}

/// Attach each turn's tool count to its first non-tool entry, or its first
/// entry if the turn contains only tools. Entries must be in timeline order.
pub(crate) fn attach_turn_tool_summaries(
    entries: &mut [SessionTimelineIndexEntry],
    quality: SessionCountQuality,
    explanation: Option<&str>,
) {
    struct TurnTools {
        tool_calls: u64,
        first_position: usize,
        first_non_tool_position: Option<usize>,
    }

    let mut turns = BTreeMap::<String, TurnTools>::new();
    for (position, entry) in entries.iter().enumerate() {
        let Some(turn_id) = &entry.turn_id else {
            continue;
        };
        let turn = turns.entry(turn_id.clone()).or_insert(TurnTools {
            tool_calls: 0,
            first_position: position,
            first_non_tool_position: None,
        });
        if entry.kind == SessionTimelineEventKind::ToolCall {
            turn.tool_calls += 1;
        } else {
            turn.first_non_tool_position.get_or_insert(position);
        }
    }

    for (turn_id, turn) in turns {
        if turn.tool_calls == 0 {
            continue;
        }
        let position = turn.first_non_tool_position.unwrap_or(turn.first_position);
        entries[position].turn_tool_summary = Some(SessionTurnToolSummary {
            turn_id,
            tool_calls: SessionCount {
                value: Some(turn.tool_calls),
                quality,
                explanation: explanation.map(ToOwned::to_owned),
            },
        });
    }
}

/// The provider's command-line tool, resolved per operation by one policy:
/// the saved selection, Mallard's `PATH`, the login shell's `PATH`, then
/// documented install directories. Adapters own the launcher-to-binary
/// step; discovery itself is neutral.
pub trait ProviderCli: Send + Sync {
    /// The CLI an operation runs. `explicit` is the path saved in Settings;
    /// when it is set and invalid the result is an error, never another
    /// installation.
    fn resolve(&self, explicit: Option<&Path>) -> Result<ResolvedCli, String>;
    /// `--version` of a resolved CLI, run where it can touch nothing of the
    /// user's. Returns the version alone.
    fn probe_version(&self, cli: &ResolvedCli) -> Result<String, String>;
}

/// Where a provider-declared maintenance action applies.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceActionScope {
    Profile,
    AfterPull,
}

/// One provider-declared maintenance action. The frontend renders declared
/// actions and never checks a provider key to decide whether to show one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceActionDescriptor {
    pub action_id: mallard_domain::ActionKey,
    pub label: String,
    pub description: String,
    pub scope: MaintenanceActionScope,
    pub requires_confirmation: bool,
    pub read_only: bool,
    /// Display order among one provider's actions.
    pub display_rank: u32,
}

/// One action a completed Pull offers, declared by the provider adapter.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AfterPullActionOffer {
    pub provider: ProviderKey,
    pub action_id: mallard_domain::ActionKey,
    pub label: String,
    pub message: String,
    /// The action can run now; otherwise the message explains what to do
    /// by hand.
    pub available: bool,
    /// Safe, adapter-typed arguments the shell hands back on execution.
    pub arguments: mallard_domain::BoundedConfigObject,
}

/// Serializable provider facts for setup and Activity. The frontend iterates
/// these; it never holds a fixed provider array.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub key: ProviderKey,
    pub display_name: String,
    /// Display order: lower ranks first, then key order.
    pub display_rank: u32,
    /// Short label for the profile picker, e.g. ".codex profile".
    pub profile_label: String,
    /// Home-relative default directory name, e.g. ".codex".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile_directory: Option<String>,
    /// May pre-enable this provider for a new draft; never a requirement.
    pub setup_default: bool,
    /// Whether [`SessionAdapter::cli`] names a command-line tool, so
    /// Settings can offer its executable selection.
    #[serde(default)]
    pub has_cli: bool,
    /// Declared maintenance actions, in display order.
    #[serde(default)]
    pub maintenance_actions: Vec<MaintenanceActionDescriptor>,
}

/// Bounded provider-authoritative identity metadata from one session file.
#[derive(Clone, Debug)]
pub struct SessionFileMetadata {
    pub session_id: String,
    /// The session's immutable ownership home, established when the session
    /// was authorized. Later per-turn `cwd` values are mutable execution
    /// context — activity metadata that never moves the session.
    pub authorized_home: PathBuf,
    pub is_internal_subagent: bool,
    /// Proven to contain only session metadata, with no inherited history.
    pub is_empty: bool,
}

/// Why a metadata scan named no session. The two are kept apart because
/// only a verdict on the bytes holds for as long as the bytes do.
#[derive(Debug)]
pub(crate) enum ScanFailure {
    /// The file could not be read: nothing was learned about its bytes.
    Unreadable(String),
    /// The bytes were read and do not name a session the provider accepts.
    Rejected(String),
}

impl ScanFailure {
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::Unreadable(message) | Self::Rejected(message) => message,
        }
    }
}

/// A folder a profile's recent sessions ran in. A hint for adding a project,
/// read from a bounded number of files — never ownership evidence, which
/// stays with [`SessionAdapter::discover`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHomeSighting {
    pub home: PathBuf,
    /// The newest modification time, in Unix seconds, among the session
    /// files that named this folder.
    pub last_active: u64,
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

pub struct SessionDiscoveryContext<'a> {
    /// Canonicalized provider home for the selected profile.
    pub provider_home: &'a Path,
    /// Canonicalized project root. Sessions are owned iff their
    /// authoritative home equals this root exactly — a subdirectory is a
    /// different project.
    pub project_root: &'a Path,
}

#[derive(Clone, Debug)]
pub struct DiscoveredSession {
    pub session_id: String,
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
    /// The last human input: the time that places the session on a commit.
    pub last_prompt_at: Option<u64>,
    pub branch: Option<String>,
    pub recorded_sha: Option<String>,
    pub user_rounds: u64,
    pub agent_messages: u64,
    pub tool_calls: u64,
    pub total_input_tokens: Option<u64>,
    pub total_output_tokens: Option<u64>,
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
    /// Delegations to a subagent. Counted apart from tool calls: the row is a
    /// speaker being created, and the page shows it by default.
    pub agent_calls: SessionCount,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SessionTimelineEventKind {
    Prompt,
    Response,
    IntermediateStep,
    Checkpoint,
    ToolCall,
    AgentCall,
}

/// How the parent ran the child: waiting for it (`Sync`), in the background
/// (`Async`), or as a Codex collaborator thread (`Spawn`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionAgentMode {
    Sync,
    Async,
    Spawn,
}

/// What the parent and the child transcript together prove about the run.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionAgentStatus {
    Completed,
    Running,
    Failed,
    Interrupted,
    /// The child transcript ends without a final message and is not live.
    Incomplete,
    /// No child transcript can be read for this call.
    Unavailable,
}

/// Metadata-only agent facts for the index: identities, a provider-home-
/// relative pointer to the child transcript, and the lifecycle the parent
/// itself recorded. Never a label, brief, or report — those are read from
/// the records at page time like every other text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct SessionAgentIndexEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_agent_id: Option<String>,
    /// The child transcript relative to the provider home, when the provider
    /// located it while indexing (Codex rollouts are found by id; Claude
    /// derives the path from the parent instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_status: Option<SessionAgentStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_end: Option<u64>,
}

/// One agent row, read at page time from the call record, its result, and
/// the head and tail of the child transcript.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionAgentPreview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_agent_id: Option<String>,
    /// Claude's task description, Codex's nickname; `Agent` when neither.
    pub label: String,
    /// Codex's task path, the slug the agent was spawned under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub mode: SessionAgentMode,
    pub status: SessionAgentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Only when the parent recorded them (a synchronous Claude result);
    /// never computed from the child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    pub brief_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    pub report_truncated: bool,
    /// The resource id that opens the child as its own session page, when a
    /// child transcript exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_resource_id: Option<String>,
}

/// A child transcript addressed through its parent: the parent's native
/// session id and the child's own agent id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionChildIdentity {
    pub parent_provider_session_id: String,
    pub provider_agent_id: String,
}

/// The segment that joins a parent resource id to one child agent id.
pub const AGENT_RESOURCE_SEGMENT: &str = "/agent/";

pub fn agent_resource_id(parent_resource_id: &str, provider_agent_id: &str) -> String {
    format!("{parent_resource_id}{AGENT_RESOURCE_SEGMENT}{provider_agent_id}")
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

/// Where a message's model and thinking level were read from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMessageSettingsSource {
    /// The Codex `turn_context` of the message's turn.
    Turn,
    /// A Claude reply's own record.
    Reply,
    /// The first Claude reply that answered this input.
    Linked,
    /// Nothing recorded.
    None,
}

/// The longest admitted model or effort value.
pub const MAX_SETTING_VALUE_BYTES: usize = 128;

/// The model and thinking level a message was generated with, as the
/// transcript recorded them: a field is absent when it recorded none. An
/// input wears the first reply that answered it; nothing is read from the
/// current configuration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionMessageSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub source: SessionMessageSettingsSource,
}

impl SessionMessageSettings {
    /// Nothing recorded.
    pub fn none() -> Self {
        Self {
            model: None,
            effort: None,
            source: SessionMessageSettingsSource::None,
        }
    }

    /// What one source recorded. Two absent fields read as nothing recorded,
    /// whatever the source was.
    pub fn recorded(
        model: Option<String>,
        effort: Option<String>,
        source: SessionMessageSettingsSource,
    ) -> Self {
        if model.is_none() && effort.is_none() {
            return Self::none();
        }
        Self {
            model,
            effort,
            source,
        }
    }

    /// Whether either field was recorded.
    pub fn is_recorded(&self) -> bool {
        self.model.is_some() || self.effort.is_some()
    }

    /// Each value is an admitted token, and an empty record names no source.
    pub fn is_valid(&self) -> bool {
        [&self.model, &self.effort]
            .into_iter()
            .flatten()
            .all(|value| is_setting_token(value))
            && self.is_recorded() == (self.source != SessionMessageSettingsSource::None)
    }
}

/// A recorded model or effort value the index admits: a short ASCII token
/// such as `gpt-5.6-sol` or `xhigh`. Anything else reads as not recorded.
pub fn is_setting_token(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_SETTING_VALUE_BYTES
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'/' | b'+' | b'-')
        })
}

/// The recorded value, when it is an admitted token.
pub fn setting_token(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| is_setting_token(value))
        .map(ToOwned::to_owned)
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
    /// Present exactly on `AgentCall` entries, beside the call's `tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<SessionAgentIndexEntry>,
    /// What generated a prompt or an assistant message; the empty record on
    /// every other kind. Required: a cache written without it is rebuilt.
    pub settings: SessionMessageSettings,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionDetailWarning {
    pub code: String,
    pub message: String,
    /// How loudly this reads. A transcript the reader cannot see whole is a
    /// `Warning`; a note about how the index was built is `Info`. The page
    /// shows every warning in one box at the loudest rung present, so this is
    /// what decides that box's tone and title.
    pub severity: NoticeSeverity,
}

/// Ordered physical sources for a logical Codex timeline. Paths are relative
/// to the provider home; only retained byte lengths enter the local cache.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionTimelineSource {
    pub relative_path: String,
    pub length: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionTimelineIndex {
    pub sources: Vec<SessionTimelineSource>,
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
    /// Canonical provider home. Child transcripts are located under it and
    /// pointed to relative to it.
    pub provider_home: &'a Path,
    /// Set when the transcript being indexed is itself a child: the provider
    /// then reads the child's own chain as the main one.
    pub child: Option<&'a SessionChildIdentity>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<SessionAgentPreview>,
    /// On a prompt or a response: what generated it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SessionMessageSettings>,
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
}

pub struct SessionActionRequest<'a> {
    pub action_id: &'a str,
    pub provider_home: &'a Path,
    pub project_root: &'a Path,
    pub provider_session_id: &'a str,
    /// The CLI executable saved in Settings for this provider, if any.
    pub cli_path: Option<&'a Path>,
    /// Observer for the exact shell command before it is launched, so the
    /// activity log records what ran.
    pub before_launch: &'a mut dyn FnMut(&str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionActionOutcome {
    Launched,
}

/// One pasteable way to open a pulled project with this provider, as the
/// post-Pull log lists it: `label: command`. A `command` may also be a
/// short instruction when no single shell line exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectOpenCommand {
    pub label: String,
    pub command: String,
}

/// Single-quote a value for a POSIX shell line.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub trait SessionAdapter: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn codec_key(&self) -> &str;
    fn codec_version(&self) -> u32;
    /// Whether this provider's conversations are physical rollout graphs
    /// that Mallard reconstructs as standalone artifacts (alias-owner lookup,
    /// relocated cloud materialization under freshly minted ids, and the
    /// graph seeding primitives). Wire-tail providers materialize in place.
    fn reconstructs_rollout_graph(&self) -> bool {
        false
    }
    /// Whether a Pull that updates a session leaves the children that only
    /// this computer has in their place, as children of the session. Such
    /// a child then stands against nothing that the storage holds.
    fn update_keeps_local_only_children(&self) -> bool {
        false
    }
    /// Mint one provider thread id for a new local copy. `taken` reports
    /// ids already occupied; the default is the UUIDv4 convention.
    fn mint_provider_thread_id(&self, taken: &dyn Fn(&str) -> bool) -> Result<String, String> {
        loop {
            let candidate = mallard_domain::generated_uuid_v4()?;
            if !taken(&candidate) {
                return Ok(candidate);
            }
        }
    }
    /// The root tail layout codec a materialized binding of this provider
    /// must carry; `None` when the provider declares no layout.
    fn tail_layout_codec(&self) -> Option<&'static str> {
        None
    }
    /// The wire rules published conversations of this provider must
    /// satisfy; `None` accepts any structurally valid thread.
    fn conversation_wire_rules(&self) -> Option<mallard_domain::ConversationWireRules> {
        None
    }
    /// Decode a portable display title into the local spelling. The
    /// default keeps the title as published.
    fn decode_portable_display_title(
        &self,
        title: &str,
        _project_root: &str,
        _canonical_project_root: &str,
    ) -> Result<String, String> {
        Ok(title.to_string())
    }
    /// Provider-declared maintenance actions, in display order.
    fn maintenance_actions(&self) -> Vec<MaintenanceActionDescriptor> {
        Vec::new()
    }
    /// The ways to open a pulled project with this provider from its
    /// configured home, listed after a Pull. Empty when the provider has no
    /// pasteable launch. `cli_path` is the saved selection; without one the
    /// line names the program and the terminal that runs it resolves it.
    /// Guidance never runs discovery or a provider process.
    fn project_open_commands(
        &self,
        _home: &Path,
        _project_root: &str,
        _cli_path: Option<&Path>,
    ) -> Vec<ProjectOpenCommand> {
        Vec::new()
    }
    /// This provider's command-line tool, when it has one Mallard runs.
    fn cli(&self) -> Option<&dyn ProviderCli> {
        None
    }
    /// The action a completed Pull offers for this provider, if any.
    fn after_pull_offer(
        &self,
        _operation_id: &mallard_domain::ConversationOperationId,
        _provider_home: Option<&Path>,
    ) -> Option<AfterPullActionOffer> {
        None
    }
    /// Version of the normalized timeline projection. Changing event
    /// semantics or indexed metadata must increment this value so cached
    /// indexes and snapshot cursors invalidate safely.
    fn timeline_version(&self) -> u32 {
        self.codec_version()
    }
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
    /// Discover project-owned roots for bulk deletion, including empty sessions
    /// hidden from normal discovery. Keep the same ownership and child checks.
    fn discover_for_deletion(
        &self,
        context: &SessionDiscoveryContext<'_>,
    ) -> Result<SessionDiscovery, String> {
        self.discover(context)
    }
    /// The one project-owned session that `is_requested` accepts, given its
    /// provider session id and transcript path, with the same ownership
    /// evidence `discover` gathers for it. A provider whose file names carry
    /// the session identity reads only the requested transcript's head; one
    /// whose names carry nothing discovers every session and selects.
    fn locate_session(
        &self,
        context: &SessionDiscoveryContext<'_>,
        is_requested: &mut dyn FnMut(&str, &Path) -> Result<bool, String>,
    ) -> Result<Option<DiscoveredSession>, String> {
        for session in self.discover(context)?.sessions {
            if is_requested(&session.session_id, &session.physical_path)? {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }
    /// The folders the newest sessions under one provider home ran in,
    /// from a bounded read that never follows links. A provider without a
    /// cheap way to answer offers none.
    fn recent_session_homes(
        &self,
        _provider_home: &Path,
    ) -> Result<Vec<SessionHomeSighting>, String> {
        Ok(Vec::new())
    }
    /// Bounded head-of-file metadata scan.
    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String>;
    /// Full typed-codec analysis of one transcript.
    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis;
    /// Capture facts and presentation from one parsed record stream.
    fn analyze_and_summarize(
        &self,
        bytes: &[u8],
    ) -> (SessionAnalysis, Result<SessionFileSummary, String>);
    /// Verify one staged provider index fragment. Only meaningful when
    /// `index_fragment_limit() > 0`.
    fn verify_index_fragment(
        &self,
        _bytes: &[u8],
        _expected_session_id: &str,
    ) -> Result<(), String> {
        Err("provider has no index fragments".to_string())
    }
    /// Verify one staged sidecar payload of `sidecar_kind` before it is
    /// committed into the provider home. Providers without sidecars refuse
    /// every kind.
    fn verify_sidecar(&self, _sidecar_kind: &str, _bytes: &[u8]) -> Result<(), String> {
        Err("provider has no sidecar files".to_string())
    }
    /// Propose materialization targets for one conversation.
    fn materialization_targets(
        &self,
        request: &MaterializationTargetRequest<'_>,
    ) -> Result<Vec<PlannedProviderTarget>, String>;
    /// Resolve a child under the destination parent's native namespace.
    fn child_transcript_target(
        &self,
        request: &MaterializationTargetRequest<'_>,
        _child_id: &str,
    ) -> Result<PathBuf, String> {
        let _ = request;
        Err("provider does not support child transcripts".into())
    }
    /// Map one provider-state logical suffix (under
    /// `state/providers/<key>/`) to its machine path. Providers without
    /// provider-state resources refuse.
    fn state_target(
        &self,
        _provider_home: &Path,
        _project_root: &str,
        _logical_suffix: &str,
    ) -> Result<PathBuf, String> {
        Err("provider has no provider-state resources".to_string())
    }
    /// Bounded full-file summary for Activity.
    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String>;
    /// Summarize an already captured transcript snapshot. Capture callers
    /// must use this instead of reopening the mutable source path.
    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String>;
    /// Summarize a child transcript for its own session page. The summary's
    /// `session_id` is the child's agent id.
    fn summarize_child_file(
        &self,
        _path: &Path,
        _child: &SessionChildIdentity,
    ) -> Result<SessionFileSummary, String> {
        Err("provider has no child transcripts".to_string())
    }
    /// Locate one child transcript under the provider home, proving it
    /// belongs to the parent. `Ok(None)` when the parent names a child that
    /// has no readable file.
    fn locate_child_transcript(
        &self,
        _provider_home: &Path,
        _parent_transcript: &Path,
        _child: &SessionChildIdentity,
    ) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }
    /// Build a metadata-only, bounded rich-timeline index. Implementations
    /// must not retain visible message text, tool arguments, or tool output.
    fn build_timeline_index(
        &self,
        path: &Path,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelineIndex, String>;
    /// Summarize the same retained sources used by the detail index.
    fn summarize_timeline(
        &self,
        path: &Path,
        _index: &SessionTimelineIndex,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionFileSummary, String> {
        match context.child {
            Some(child) => self.summarize_child_file(path, child),
            None => self.summarize_file(path),
        }
    }
    /// Read and normalize one filtered page from a validated index. Filtering
    /// happens before pagination. Agent rows are described here too: the
    /// call and result records, then the child's head and tail.
    fn read_timeline_page(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        query: &SessionTimelineQuery,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelinePageSlice, String>;
    /// Describe one indexed agent call the way its page row would, for the
    /// child's own page header.
    fn describe_agent_call(
        &self,
        _path: &Path,
        _entry: &SessionTimelineIndexEntry,
        _context: &SessionTimelineContext<'_>,
    ) -> Result<SessionAgentPreview, String> {
        Err("provider has no agent calls".to_string())
    }
    /// The agent calls of `index` whose children came with a Pull without
    /// their transcripts, by event ID. `binding` says what the Pull
    /// installed. The session page shows no card of them.
    fn agent_calls_without_transcript(
        &self,
        _index: &SessionTimelineIndex,
        _binding: &mallard_domain::LocalConversationBinding,
    ) -> BTreeSet<String> {
        BTreeSet::new()
    }
    /// Read one bounded expanded event body on demand.
    fn read_event_detail(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        event_id: &str,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionEventDetail, String>;
    fn available_actions(&self, context: &SessionActionContext<'_>)
        -> Vec<SessionActionDescriptor>;
    fn execute_action(
        &self,
        request: &mut SessionActionRequest<'_>,
    ) -> Result<SessionActionOutcome, String>;

    /// Explicit best-effort deletion. This does not coordinate active writers.
    /// `cli_path` is the saved CLI selection, for a provider whose deletion
    /// goes through its CLI.
    fn delete_session(
        &self,
        _home: &Path,
        _session: &DiscoveredSession,
        _cli_path: Option<&Path>,
    ) -> Result<(), String> {
        Err("This provider does not support local session deletion.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_tool_summaries_preserve_placement_and_quality() {
        use SessionTimelineEventKind::{Prompt, Response, ToolCall};

        let entries = [
            (None, Prompt),
            (Some("a"), ToolCall),
            (Some("tools-only"), ToolCall),
            (Some("no-tools"), Prompt),
            (Some("b"), Prompt),
            (None, ToolCall),
            (Some("a"), Response),
            (Some("b"), ToolCall),
            (Some("a"), Prompt),
            (Some("tools-only"), ToolCall),
            (Some("a"), ToolCall),
            (Some("b"), Response),
            (Some("no-tools"), Response),
        ]
        .into_iter()
        .enumerate()
        .map(|(position, (turn_id, kind))| SessionTimelineIndexEntry {
            event_id: format!("event-{position}"),
            ordinal: position as u64,
            timestamp: None,
            kind,
            quality: SessionCountQuality::Exact,
            turn_id: turn_id.map(ToOwned::to_owned),
            turn_tool_summary: None,
            parent_event_id: None,
            source: SessionSourceSpan {
                offset: position as u64,
                length: 1,
            },
            settings: SessionMessageSettings::none(),
            tool: (kind == ToolCall).then(|| SessionToolIndexEntry {
                provider_call_id: format!("call-{position}"),
                native_name: "shell".to_string(),
                category: SessionToolCategory::Shell,
                status: SessionToolCallStatus::Completed,
                affected_paths: Vec::new(),
                result_source: None,
            }),
            agent: None,
        })
        .collect::<Vec<_>>();

        for (quality, explanation) in [
            (SessionCountQuality::Exact, None),
            (SessionCountQuality::LowerBound, Some("Incomplete history.")),
        ] {
            let mut actual = entries.clone();
            let mut expected = entries.clone();
            for (position, turn_id, count) in [(2, "tools-only", 2), (4, "b", 1), (6, "a", 2)] {
                expected[position].turn_tool_summary = Some(SessionTurnToolSummary {
                    turn_id: turn_id.to_string(),
                    tool_calls: SessionCount {
                        value: Some(count),
                        quality,
                        explanation: explanation.map(ToOwned::to_owned),
                    },
                });
            }

            attach_turn_tool_summaries(&mut actual, quality, explanation);

            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn expanded_tool_detail_shares_one_mib_and_lends_unused_space() {
        assert_eq!(
            timeline_tool_detail_budgets(Some("short"), Some("result")),
            (5, 6)
        );

        let balanced = "x".repeat(crate::app_settings::timeline_tool_detail_bytes());
        assert_eq!(
            timeline_tool_detail_budgets(Some(&balanced), Some(&balanced)),
            (
                crate::app_settings::timeline_tool_detail_bytes() / 2,
                crate::app_settings::timeline_tool_detail_bytes() / 2,
            )
        );

        let small = "x".repeat(100 * 1024);
        assert_eq!(
            timeline_tool_detail_budgets(Some(&small), Some(&balanced)),
            (
                100 * 1024,
                crate::app_settings::timeline_tool_detail_bytes() - 100 * 1024
            )
        );
        assert_eq!(
            timeline_tool_detail_budgets(None, Some(&balanced)),
            (0, crate::app_settings::timeline_tool_detail_bytes())
        );
    }
}
