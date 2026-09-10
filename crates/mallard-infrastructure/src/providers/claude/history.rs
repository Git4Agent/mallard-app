//! Bounded Claude session summaries and detail previews for Activity.

use super::codec::ClaudeSessionIdentityTracker;
use crate::app_settings::{
    conversation_file_limit_bytes, timeline_message_preview_bytes, timeline_page_content_bytes,
    timeline_tool_preview_bytes,
};
use crate::providers::timeparse::parse_timestamp_value;
use crate::providers::traits::{
    agent_resource_id, attach_turn_tool_summaries, most_specific_project_relative_path,
    timeline_tool_detail_budgets, SessionAgentIndexEntry, SessionAgentMode, SessionAgentPreview,
    SessionAgentStatus, SessionChildIdentity, SessionCount, SessionCountQuality,
    SessionDetailWarning, SessionEventCounts, SessionEventDetail, SessionFileSummary,
    SessionSourceSpan, SessionTimelineContext, SessionTimelineEvent, SessionTimelineEventKind,
    SessionTimelineIndex, SessionTimelineIndexEntry, SessionTimelinePageSlice,
    SessionTimelineQuery, SessionToolCallPreview, SessionToolCallStatus, SessionToolCategory,
    SessionToolIndexEntry,
};
use mallard_domain::NoticeSeverity;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

const TITLE_CHARS: usize = 120;
/// One record's read bound tracks the transcript bound: every admitted file
/// can be indexed line by line.
fn max_line_bytes() -> usize {
    conversation_file_limit_bytes() as usize
}
const MAX_TIMELINE_EVENTS: usize = 50_000;
const MAX_CLAUDE_RECORDS: usize = 100_000;
/// 2: `Task`/`Agent` calls index as agent calls, with child identities.
pub(crate) const TIMELINE_PARSER_VERSION: u32 = 2;
/// The suffix of a child transcript that its row is read from: the last
/// record's clock and the final message. The same bound the scan window uses.
const CHILD_TAIL_WINDOW_BYTES: u64 = 256 * 1024;
/// A transcript written this recently is still being written to.
const LIVE_TRANSCRIPT_SECS: u64 = 300;

/// The tools that create a subagent. `Task` is the older spelling of `Agent`.
fn is_agent_tool(name: &str) -> bool {
    matches!(name, "Task" | "Agent")
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("inspect '{}': {}", path.display(), e))?;
    if meta.len() > conversation_file_limit_bytes() {
        return Err(format!(
            "Claude session '{}' exceeds {} bytes",
            path.display(),
            conversation_file_limit_bytes()
        ));
    }
    fs::read(path).map_err(|e| format!("read '{}': {}", path.display(), e))
}

fn record_text(value: &Value) -> Option<String> {
    let message = value.get("message")?;
    match message.get("content") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(items)) => {
            let mut collected = String::new();
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !collected.is_empty() {
                            collected.push(' ');
                        }
                        collected.push_str(text);
                    }
                }
            }
            (!collected.is_empty()).then_some(collected)
        }
        _ => None,
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= limit {
        normalized
    } else {
        let mut truncated: String = normalized.chars().take(limit).collect();
        truncated.push('…');
        truncated
    }
}

fn title_candidate(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| truncate_chars(text, TITLE_CHARS))
}

fn is_meta_record(object: &serde_json::Map<String, Value>) -> bool {
    object
        .get("isMeta")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || object
            .get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ClaudeTokenUsage {
    input: u64,
    output: u64,
}

impl ClaudeTokenUsage {
    fn total(self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

pub(crate) fn summarize_file(path: &Path) -> Result<SessionFileSummary, String> {
    let bytes = read_bounded(path)?;
    summarize_bytes(&bytes)
}

/// A child transcript's records are all sidechain records naming the parent
/// session and the agent. Read as the child's own page, that sidechain is the
/// main chain: the flag comes off, and a record that is not this child's is
/// skipped rather than indexed under the wrong identity.
fn normalize_child_record(mut value: Value, child: &SessionChildIdentity) -> Option<Value> {
    let object = value.as_object_mut()?;
    if object.get("sessionId").and_then(Value::as_str)
        != Some(child.parent_provider_session_id.as_str())
        || object.get("agentId").and_then(Value::as_str) != Some(child.provider_agent_id.as_str())
        || object.get("isSidechain").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    object.remove("isSidechain");
    Some(value)
}

/// The Activity summary of a child transcript, identified by its agent id.
pub(crate) fn summarize_child_file(
    path: &Path,
    child: &SessionChildIdentity,
) -> Result<SessionFileSummary, String> {
    let bytes = read_bounded(path)?;
    let mut summary = SummaryAccumulator::new();
    for record in crate::conversation_codec::parsed_records(&bytes) {
        match record {
            Ok(value) => {
                if let Some(value) = normalize_child_record(value, child) {
                    summary.observe(&Ok(value));
                }
            }
            Err(error) => summary.observe(&Err(error)),
        }
    }
    let mut summary = summary.finish()?;
    summary.session_id = child.provider_agent_id.clone();
    summary.is_internal_subagent = true;
    Ok(summary)
}

pub(crate) fn summarize_bytes(bytes: &[u8]) -> Result<SessionFileSummary, String> {
    if bytes.len() as u64 > conversation_file_limit_bytes() {
        return Err(format!(
            "Claude session exceeds {} bytes",
            conversation_file_limit_bytes()
        ));
    }
    let mut summary = SummaryAccumulator::new();
    for record in crate::conversation_codec::parsed_records(bytes) {
        summary.observe(&record);
    }
    summary.finish()
}

pub(crate) fn analyze_and_summarize(
    bytes: &[u8],
) -> (
    crate::providers::SessionAnalysis,
    Result<SessionFileSummary, String>,
) {
    let mut summary = SummaryAccumulator::new();
    let mut records =
        crate::conversation_codec::parsed_records(bytes).inspect(|record| summary.observe(record));
    let analysis = super::codec::analyze_claude_records(bytes, &mut records);
    records.for_each(drop);
    let summary = if bytes.len() as u64 > conversation_file_limit_bytes() {
        Err(format!(
            "Claude session exceeds {} bytes",
            conversation_file_limit_bytes()
        ))
    } else {
        summary.finish()
    };
    (analysis, summary)
}

#[derive(Default)]
struct SummaryAccumulator {
    summary: SessionFileSummary,
    identity: ClaudeSessionIdentityTracker,
    first_user_text: Option<String>,
    summary_title: Option<String>,
    ai_title: Option<String>,
    custom_title: Option<String>,
    user_turn_ids: BTreeSet<String>,
    unidentified_user_turns: u64,
    agent_message_ids: BTreeSet<String>,
    unidentified_agent_messages: u64,
    token_usage_by_message_id: BTreeMap<String, ClaudeTokenUsage>,
}

impl SummaryAccumulator {
    fn new() -> Self {
        Self {
            summary: SessionFileSummary {
                metrics_complete: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn observe(&mut self, record: &Result<Value, serde_json::Error>) {
        let Ok(value) = record else {
            self.summary.metrics_complete = false;
            return;
        };
        self.identity.observe(&value);
        let Some(object) = value.as_object() else {
            self.summary.metrics_complete = false;
            return;
        };
        if let Some(timestamp) = object.get("timestamp").and_then(parse_timestamp_value) {
            if self
                .summary
                .started_at
                .is_none_or(|start| timestamp < start)
            {
                self.summary.started_at = Some(timestamp);
            }
            if self.summary.ended_at.is_none_or(|end| timestamp > end) {
                self.summary.ended_at = Some(timestamp);
            }
        }
        if let Some(branch) = object.get("gitBranch").and_then(Value::as_str) {
            if !branch.is_empty() {
                self.summary.branch = Some(branch.to_string());
            }
        }
        match object.get("type").and_then(Value::as_str) {
            Some("summary") => {
                if let Some(title) = title_candidate(object.get("summary")) {
                    self.summary_title = Some(title);
                }
            }
            Some("ai-title") => {
                if let Some(title) = title_candidate(object.get("aiTitle")) {
                    self.ai_title = Some(title);
                }
            }
            Some("custom-title") => {
                if let Some(title) = title_candidate(object.get("customTitle")) {
                    self.custom_title = Some(title);
                }
            }
            Some("user") if !is_meta_record(object) => {
                if let Some(text) = meaningful_human_prompt(&value) {
                    let is_new_turn = match object
                        .get("uuid")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                    {
                        Some(id) => self.user_turn_ids.insert(id.to_string()),
                        None => {
                            self.unidentified_user_turns =
                                self.unidentified_user_turns.saturating_add(1);
                            true
                        }
                    };
                    if is_new_turn && self.first_user_text.is_none() {
                        self.first_user_text = Some(truncate_chars(&text, TITLE_CHARS));
                    }
                }
            }
            Some("assistant") if !is_meta_record(object) => {
                let stop_reason = normalized_type(
                    value
                        .get("message")
                        .and_then(|message| message.get("stop_reason"))
                        .and_then(Value::as_str),
                );
                if is_terminal_response_stop_reason(&stop_reason)
                    && assistant_record_is_visible(&value)
                    && record_text(&value).is_some_and(|text| !text.trim().is_empty())
                {
                    match value
                        .get("message")
                        .and_then(|message| message.get("id"))
                        .and_then(Value::as_str)
                        .or_else(|| object.get("uuid").and_then(Value::as_str))
                        .filter(|id| !id.is_empty())
                    {
                        Some(id) => {
                            self.agent_message_ids.insert(id.to_string());
                        }
                        None => {
                            self.unidentified_agent_messages =
                                self.unidentified_agent_messages.saturating_add(1);
                        }
                    }
                }
                if let Some(Value::Array(items)) = value
                    .get("message")
                    .and_then(|message| message.get("content"))
                {
                    let calls = items
                        .iter()
                        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
                        .count() as u64;
                    self.summary.tool_calls = self.summary.tool_calls.saturating_add(calls);
                }
                if let Some(usage) = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    let input = [
                        "input_tokens",
                        "cache_creation_input_tokens",
                        "cache_read_input_tokens",
                    ]
                    .into_iter()
                    .fold(0_u64, |total, field| {
                        total.saturating_add(usage.get(field).and_then(Value::as_u64).unwrap_or(0))
                    });
                    let output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let observed = ClaudeTokenUsage { input, output };
                    if observed.total() > 0 {
                        let message_id = value
                            .get("message")
                            .and_then(|message| message.get("id"))
                            .and_then(Value::as_str)
                            .or_else(|| object.get("uuid").and_then(Value::as_str))
                            .filter(|id| !id.is_empty());
                        if let Some(message_id) = message_id {
                            self.token_usage_by_message_id
                                .entry(message_id.to_string())
                                .and_modify(|current| {
                                    if observed.total() > current.total() {
                                        *current = observed;
                                    }
                                })
                                .or_insert(observed);
                        } else {
                            self.summary.metrics_complete = false;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(self) -> Result<SessionFileSummary, String> {
        let Self {
            mut summary,
            identity,
            first_user_text,
            summary_title,
            ai_title,
            custom_title,
            user_turn_ids,
            unidentified_user_turns,
            agent_message_ids,
            unidentified_agent_messages,
            token_usage_by_message_id,
        } = self;
        if !token_usage_by_message_id.is_empty() {
            let (input, output) = token_usage_by_message_id.values().fold(
                (0_u64, 0_u64),
                |(input, output), usage| {
                    (
                        input.saturating_add(usage.input),
                        output.saturating_add(usage.output),
                    )
                },
            );
            summary.total_input_tokens = Some(input);
            summary.total_output_tokens = Some(output);
            summary.total_tokens = Some(input.saturating_add(output));
        }
        summary.session_id = identity
            .leaf_identity()
            .0
            .ok_or_else(|| "Claude session file has no leaf session ID".to_string())?;
        summary.user_rounds = (user_turn_ids.len() as u64).saturating_add(unidentified_user_turns);
        summary.agent_messages =
            (agent_message_ids.len() as u64).saturating_add(unidentified_agent_messages);
        summary.summary = first_user_text.clone();
        summary.title = custom_title
            .or(ai_title)
            .or(summary_title)
            .or(first_user_text);
        Ok(summary)
    }
}

#[derive(Clone, Debug)]
struct PendingTimelineEvent {
    entry: SessionTimelineIndexEntry,
    order: (u64, u16),
}

#[derive(Clone, Debug)]
struct AssistantGroup {
    event_id: String,
    turn_id: String,
    timestamp: Option<u64>,
    order: (u64, u16),
    source: SessionSourceSpan,
    text: Option<String>,
    score: (u8, usize, usize, u64),
    stop_reason: String,
    has_tool_use: bool,
}

fn is_terminal_response_stop_reason(stop_reason: &str) -> bool {
    matches!(stop_reason, "endturn" | "stopsequence")
}

#[derive(Default)]
struct ClaudeTimelineBuilder {
    events: Vec<PendingTimelineEvent>,
    assistant_groups: BTreeMap<String, AssistantGroup>,
    // Claude can re-emit a call after later prompts; the provider call ID is
    // the event identity, independent of the parser's current turn.
    tool_positions_by_call_id: BTreeMap<String, usize>,
    prompt_ids: BTreeSet<String>,
    current_turn_id: Option<String>,
    degraded_all: bool,
    degraded_tools: bool,
    inferred_responses: bool,
    truncated: bool,
    warnings: Vec<SessionDetailWarning>,
}

impl ClaudeTimelineBuilder {
    fn warning(&mut self, code: &str, message: &str, severity: NoticeSeverity) {
        if self.warnings.iter().any(|warning| warning.code == code) {
            return;
        }
        self.warnings.push(SessionDetailWarning {
            code: code.to_string(),
            message: message.to_string(),
            severity,
        });
    }

    /// Degrading every count means the reader is provably seeing less than the
    /// transcript holds, so these always read as `Warning`.
    fn mark_all_lower_bound(&mut self, code: &str, message: &str) {
        self.degraded_all = true;
        self.degraded_tools = true;
        self.truncated = true;
        self.warning(code, message, NoticeSeverity::Warning);
    }

    /// Tool counts degrade to a floor, so the reader is again seeing less than
    /// the transcript holds: `Warning`.
    fn mark_tools_lower_bound(&mut self, code: &str, message: &str) {
        self.degraded_tools = true;
        self.warning(code, message, NoticeSeverity::Warning);
    }

    fn add_prompt(
        &mut self,
        uuid: String,
        timestamp: Option<u64>,
        source: SessionSourceSpan,
        text_len: usize,
    ) {
        let event_id = format!("claude:prompt:{uuid}");
        let turn_id = format!("claude:turn:{uuid}");
        self.current_turn_id = Some(turn_id.clone());
        if !self.prompt_ids.insert(event_id.clone()) {
            return;
        }
        if self.events.len() >= MAX_TIMELINE_EVENTS {
            self.mark_all_lower_bound(
                "timeline_event_limit",
                "The Claude transcript contains more timeline events than Mallard can index.",
            );
            return;
        }
        let quality = if text_len > timeline_message_preview_bytes() {
            self.warning(
                "message_preview_truncated",
                "At least one Claude message exceeds the 1-MiB timeline display limit and is shown as a truncated preview.",
                NoticeSeverity::Warning,
            );
            SessionCountQuality::LowerBound
        } else {
            SessionCountQuality::Exact
        };
        self.events.push(PendingTimelineEvent {
            entry: SessionTimelineIndexEntry {
                event_id,
                ordinal: 0,
                timestamp,
                kind: SessionTimelineEventKind::Prompt,
                quality,
                turn_id: Some(turn_id),
                turn_tool_summary: None,
                parent_event_id: None,
                source,
                tool: None,
                agent: None,
            },
            order: (source.offset, 0),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_assistant(
        &mut self,
        message_id: String,
        timestamp: Option<u64>,
        source: SessionSourceSpan,
        text: Option<String>,
        content_count: usize,
        output_tokens: u64,
        stop_reason: String,
        has_tool_use: bool,
    ) {
        let turn_id = self
            .current_turn_id
            .clone()
            .unwrap_or_else(|| "claude:turn:orphan".to_string());
        let score = (
            u8::from(is_terminal_response_stop_reason(&stop_reason)),
            content_count,
            text.as_ref().map_or(0, String::len),
            output_tokens,
        );
        let event_id = format!("claude:message:{message_id}");
        match self.assistant_groups.get_mut(&message_id) {
            Some(group) => {
                group.has_tool_use |= has_tool_use;
                if score >= group.score {
                    group.timestamp = group.timestamp.or(timestamp);
                    group.source = source;
                    group.text = text;
                    group.score = score;
                    group.stop_reason = stop_reason;
                }
            }
            None => {
                self.assistant_groups.insert(
                    message_id,
                    AssistantGroup {
                        event_id,
                        turn_id,
                        timestamp,
                        order: (source.offset, 0),
                        source,
                        text,
                        score,
                        stop_reason,
                        has_tool_use,
                    },
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_tool(
        &mut self,
        call_id: String,
        native_name: String,
        category: SessionToolCategory,
        turn_id: String,
        timestamp: Option<u64>,
        source: SessionSourceSpan,
        block_index: usize,
        affected_paths: Vec<String>,
        parent_event_id: Option<String>,
    ) {
        if let Some(position) = self.tool_positions_by_call_id.get(&call_id).copied() {
            let pending = &mut self.events[position];
            if let Some(tool) = pending.entry.tool.as_mut() {
                if tool.native_name == "unknown" && native_name != "unknown" {
                    tool.native_name = native_name;
                }
                if tool.category == SessionToolCategory::Other
                    && category != SessionToolCategory::Other
                {
                    tool.category = category;
                }
                for path in affected_paths {
                    if !tool.affected_paths.contains(&path) {
                        tool.affected_paths.push(path);
                    }
                }
            }
            return;
        }
        if self.events.len() >= MAX_TIMELINE_EVENTS {
            self.mark_tools_lower_bound(
                "timeline_event_limit",
                "The Claude transcript contains more tool calls than Mallard can index.",
            );
            self.truncated = true;
            return;
        }
        let position = self.events.len();
        self.tool_positions_by_call_id
            .insert(call_id.clone(), position);
        // A delegation is its own kind: the row is a speaker being created,
        // shown by default, where tool calls stay behind their toggle.
        let (kind, agent) = if is_agent_tool(&native_name) {
            (
                SessionTimelineEventKind::AgentCall,
                Some(SessionAgentIndexEntry::default()),
            )
        } else {
            (SessionTimelineEventKind::ToolCall, None)
        };
        self.events.push(PendingTimelineEvent {
            entry: SessionTimelineIndexEntry {
                event_id: format!("claude:tool:{call_id}"),
                ordinal: 0,
                timestamp,
                kind,
                quality: SessionCountQuality::LowerBound,
                turn_id: Some(turn_id),
                turn_tool_summary: None,
                parent_event_id,
                source,
                tool: Some(SessionToolIndexEntry {
                    provider_call_id: call_id,
                    native_name,
                    category,
                    status: SessionToolCallStatus::Incomplete,
                    result_source: None,
                    affected_paths,
                }),
                agent,
            },
            order: (
                source.offset,
                u16::try_from(block_index.saturating_add(1)).unwrap_or(u16::MAX),
            ),
        });
    }

    /// `agent_result` is the record's `toolUseResult`, which on an agent call
    /// names the child (`agentId`) and says whether the parent waited for it.
    fn complete_tool(
        &mut self,
        call_id: &str,
        status: SessionToolCallStatus,
        source: SessionSourceSpan,
        timestamp: Option<u64>,
        agent_result: Option<&Value>,
    ) {
        let position = self.tool_positions_by_call_id.get(call_id).copied();
        let Some(position) = position else {
            self.mark_tools_lower_bound(
                "orphan_tool_result",
                "A Claude tool result had no durable matching tool-use block.",
            );
            return;
        };
        let pending = &mut self.events[position];
        if let Some(tool) = pending.entry.tool.as_mut() {
            tool.status = if status == SessionToolCallStatus::Failed {
                SessionToolCallStatus::Failed
            } else {
                SessionToolCallStatus::Completed
            };
            tool.result_source = Some(source);
            pending.entry.quality = SessionCountQuality::Exact;
        }
        if let Some(agent) = pending.entry.agent.as_mut() {
            let result = agent_result.unwrap_or(&Value::Null);
            agent.provider_agent_id = result
                .get("agentId")
                .and_then(Value::as_str)
                .filter(|id| super::subagents::valid_agent_id(id))
                .map(ToOwned::to_owned);
            // A synchronous result is the child's end; a background launch
            // records nothing about the end, which the child tail supplies.
            let waited = result.get("isAsync").and_then(Value::as_bool) != Some(true);
            agent.recorded_status = if status == SessionToolCallStatus::Failed {
                Some(SessionAgentStatus::Failed)
            } else if waited {
                Some(SessionAgentStatus::Completed)
            } else {
                None
            };
            agent.recorded_end = (agent.recorded_status.is_some())
                .then_some(timestamp)
                .flatten();
        }
    }

    fn finalize(mut self) -> SessionTimelineIndex {
        let assistant_groups = std::mem::take(&mut self.assistant_groups);
        for group in assistant_groups.into_values() {
            let Some(text) = group.text.filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            if self.events.len() >= MAX_TIMELINE_EVENTS {
                self.mark_all_lower_bound(
                    "timeline_event_limit",
                    "The Claude transcript contains more visible events than Mallard can index.",
                );
                break;
            }
            let (kind, mut quality) = if is_terminal_response_stop_reason(&group.stop_reason) {
                (
                    SessionTimelineEventKind::Response,
                    SessionCountQuality::Exact,
                )
            } else if group.stop_reason == "tooluse" || group.has_tool_use {
                (
                    SessionTimelineEventKind::IntermediateStep,
                    SessionCountQuality::Exact,
                )
            } else {
                self.inferred_responses = true;
                (
                    SessionTimelineEventKind::Response,
                    SessionCountQuality::Inferred,
                )
            };
            if text.len() > timeline_message_preview_bytes() {
                quality = SessionCountQuality::LowerBound;
                self.warning(
                    "message_preview_truncated",
                    "At least one Claude message exceeds the 1-MiB timeline display limit and is shown as a truncated preview.",
                    NoticeSeverity::Warning,
                );
            }
            self.events.push(PendingTimelineEvent {
                entry: SessionTimelineIndexEntry {
                    event_id: group.event_id,
                    ordinal: 0,
                    timestamp: group.timestamp,
                    kind,
                    quality,
                    turn_id: Some(group.turn_id),
                    turn_tool_summary: None,
                    parent_event_id: None,
                    source: group.source,
                    tool: None,
                    agent: None,
                },
                order: group.order,
            });
        }
        if self.inferred_responses {
            self.warning(
                "message_phase_inferred",
                "At least one Claude assistant message lacked a terminal stop reason, so its final-response placement was inferred.",
                NoticeSeverity::Info,
            );
        }

        let mut incomplete = false;
        for pending in &mut self.events {
            if let Some(tool) = pending.entry.tool.as_mut() {
                if tool.status == SessionToolCallStatus::Incomplete {
                    incomplete = true;
                    pending.entry.quality = SessionCountQuality::LowerBound;
                }
            }
        }
        if incomplete {
            self.mark_tools_lower_bound(
                "incomplete_tool_call",
                "At least one Claude tool-use block has no durable matching result.",
            );
        }

        let visible_ids = self
            .events
            .iter()
            .filter(|pending| {
                !matches!(
                    pending.entry.kind,
                    SessionTimelineEventKind::ToolCall | SessionTimelineEventKind::AgentCall
                )
            })
            .map(|pending| pending.entry.event_id.clone())
            .collect::<BTreeSet<_>>();
        for pending in &mut self.events {
            if pending
                .entry
                .parent_event_id
                .as_ref()
                .is_some_and(|parent| !visible_ids.contains(parent))
            {
                pending.entry.parent_event_id = None;
            }
        }

        self.events.sort_by_key(|pending| pending.order);
        for (ordinal, pending) in self.events.iter_mut().enumerate() {
            pending.entry.ordinal = ordinal as u64;
        }
        let tool_quality = if self.degraded_all || self.degraded_tools {
            SessionCountQuality::LowerBound
        } else {
            SessionCountQuality::Exact
        };
        let mut entries = self
            .events
            .into_iter()
            .map(|pending| pending.entry)
            .collect::<Vec<_>>();
        attach_turn_tool_summaries(
            &mut entries,
            tool_quality,
            claude_tool_explanation(tool_quality),
        );
        let counts = claude_counts(
            &entries,
            self.degraded_all,
            self.degraded_tools,
            self.inferred_responses,
        );
        SessionTimelineIndex {
            parser_version: TIMELINE_PARSER_VERSION,
            events: entries,
            counts,
            warnings: self.warnings,
            truncated: self.truncated,
        }
    }
}

fn claude_count(
    value: u64,
    quality: SessionCountQuality,
    explanation: Option<&str>,
) -> SessionCount {
    SessionCount {
        value: Some(value),
        quality,
        explanation: explanation.map(ToOwned::to_owned),
    }
}

fn claude_tool_explanation(quality: SessionCountQuality) -> Option<&'static str> {
    (quality == SessionCountQuality::LowerBound).then_some(
        "Some Claude tool activity may be absent because the durable transcript was incomplete or malformed.",
    )
}

fn claude_counts(
    entries: &[SessionTimelineIndexEntry],
    degraded_all: bool,
    degraded_tools: bool,
    inferred_responses: bool,
) -> SessionEventCounts {
    let count_kind = |kind| entries.iter().filter(|entry| entry.kind == kind).count() as u64;
    // Categories partition the tool calls alone; an agent call keeps its
    // `Other` tag for the payload path but is counted as a delegation.
    let count_category = |category| {
        entries
            .iter()
            .filter(|entry| entry.kind == SessionTimelineEventKind::ToolCall)
            .filter_map(|entry| entry.tool.as_ref())
            .filter(|tool| tool.category == category)
            .count() as u64
    };
    let visible_quality = if degraded_all {
        SessionCountQuality::LowerBound
    } else {
        SessionCountQuality::Exact
    };
    let response_quality = if degraded_all {
        SessionCountQuality::LowerBound
    } else if inferred_responses {
        SessionCountQuality::Inferred
    } else {
        SessionCountQuality::Exact
    };
    let tool_quality = if degraded_all || degraded_tools {
        SessionCountQuality::LowerBound
    } else {
        SessionCountQuality::Exact
    };
    let visible_explanation = (visible_quality == SessionCountQuality::LowerBound)
        .then_some("Some Claude records could not be safely indexed.");
    SessionEventCounts {
        agent_calls: claude_count(
            count_kind(SessionTimelineEventKind::AgentCall),
            tool_quality,
            claude_tool_explanation(tool_quality),
        ),
        prompts: claude_count(
            count_kind(SessionTimelineEventKind::Prompt),
            visible_quality,
            visible_explanation,
        ),
        responses: claude_count(
            count_kind(SessionTimelineEventKind::Response),
            response_quality,
            if response_quality == SessionCountQuality::Inferred {
                Some("The assistant message lacked an authoritative end-turn marker.")
            } else {
                visible_explanation
            },
        ),
        intermediate_steps: claude_count(
            count_kind(SessionTimelineEventKind::IntermediateStep),
            visible_quality,
            visible_explanation,
        ),
        checkpoints: SessionCount::unsupported(
            "Git checkpoints are correlated by Mallard outside the Claude transcript.",
        ),
        tool_calls: claude_count(
            count_kind(SessionTimelineEventKind::ToolCall),
            tool_quality,
            claude_tool_explanation(tool_quality),
        ),
        file_edits: claude_count(
            count_category(SessionToolCategory::FileEdit),
            tool_quality,
            claude_tool_explanation(tool_quality),
        ),
        shell_calls: claude_count(
            count_category(SessionToolCategory::Shell),
            tool_quality,
            claude_tool_explanation(tool_quality),
        ),
        reads: claude_count(
            count_category(SessionToolCategory::Read),
            tool_quality,
            claude_tool_explanation(tool_quality),
        ),
        other_tools: claude_count(
            count_category(SessionToolCategory::Other),
            tool_quality,
            claude_tool_explanation(tool_quality),
        ),
    }
}

fn normalized_type(value: Option<&str>) -> String {
    value
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn source_span(start: u64, end: u64) -> SessionSourceSpan {
    SessionSourceSpan {
        offset: start,
        length: end.saturating_sub(start),
    }
}

fn has_non_null_alias(value: &Value, names: &[&str]) -> bool {
    names
        .iter()
        .find_map(|name| value.get(*name))
        .is_some_and(|value| !value.is_null())
}

fn content_blocks(value: &Value) -> Option<&[Value]> {
    value
        .get("message")?
        .get("content")?
        .as_array()
        .map(Vec::as_slice)
}

fn has_tool_result(value: &Value) -> bool {
    content_blocks(value).is_some_and(|blocks| {
        blocks
            .iter()
            .any(|block| normalized_type(block.get("type").and_then(Value::as_str)) == "toolresult")
    })
}

fn meaningful_human_prompt(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    if is_meta_record(object)
        || object
            .get("isSynthetic")
            .or_else(|| object.get("is_synthetic"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || object
            .get("shouldQuery")
            .or_else(|| object.get("should_query"))
            .and_then(Value::as_bool)
            == Some(false)
        || has_non_null_alias(
            value,
            &["parentToolUseID", "parent_tool_use_id", "parentToolUseId"],
        )
        || has_tool_result(value)
    {
        return None;
    }
    if let Some(kind) = object
        .get("origin")
        .and_then(|origin| origin.get("kind"))
        .and_then(Value::as_str)
    {
        if kind != "human" {
            return None;
        }
    }
    let text = record_text(value)?;
    let trimmed = text.trim_start();
    if trimmed.is_empty()
        || [
            "<system-reminder>",
            "<local-command-caveat>",
            "<command-name>",
            "<command-message>",
            "<local-command-stdout>",
            "<ide_opened_file>",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return None;
    }
    Some(text)
}

fn assistant_record_is_visible(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    !is_meta_record(object)
        && !object
            .get("isSynthetic")
            .or_else(|| object.get("is_synthetic"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && !has_non_null_alias(
            value,
            &["parentToolUseID", "parent_tool_use_id", "parentToolUseId"],
        )
}

fn classify_claude_tool(name: &str) -> SessionToolCategory {
    match name {
        "Write" | "Edit" | "NotebookEdit" => SessionToolCategory::FileEdit,
        "Bash" | "PowerShell" => SessionToolCategory::Shell,
        "Read" => SessionToolCategory::Read,
        _ => SessionToolCategory::Other,
    }
}

fn safe_project_relative_path(
    path: &str,
    project_root: &Path,
    project_root_aliases: &[PathBuf],
) -> Option<String> {
    let candidate = Path::new(path);
    let relative = if candidate.is_absolute() {
        most_specific_project_relative_path(candidate, project_root, project_root_aliases)?
    } else {
        candidate
    };
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    Some(
        relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn affected_paths(
    input: &Value,
    category: SessionToolCategory,
    project_root: &Path,
    project_root_aliases: &[PathBuf],
) -> Vec<String> {
    if category != SessionToolCategory::FileEdit {
        return Vec::new();
    }
    let mut paths = Vec::new();
    for key in ["file_path", "filePath", "notebook_path", "notebookPath"] {
        if let Some(path) = input.get(key).and_then(Value::as_str) {
            if let Some(relative) =
                safe_project_relative_path(path, project_root, project_root_aliases)
            {
                paths.push(relative);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn tool_result_status(block: &Value) -> SessionToolCallStatus {
    if block
        .get("is_error")
        .or_else(|| block.get("isError"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        SessionToolCallStatus::Failed
    } else {
        SessionToolCallStatus::Completed
    }
}

fn consume_claude_record(
    builder: &mut ClaudeTimelineBuilder,
    value: &Value,
    source: SessionSourceSpan,
    context: &SessionTimelineContext<'_>,
) {
    let record_type = normalized_type(value.get("type").and_then(Value::as_str));
    let timestamp = value.get("timestamp").and_then(parse_timestamp_value);
    match record_type.as_str() {
        "user" => {
            if let Some(text) = meaningful_human_prompt(value) {
                let uuid = value
                    .get("uuid")
                    .and_then(Value::as_str)
                    .filter(|uuid| !uuid.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("offset-{}", source.offset));
                builder.add_prompt(uuid, timestamp, source, text.len());
            }
            if let Some(blocks) = content_blocks(value) {
                for block in blocks {
                    if normalized_type(block.get("type").and_then(Value::as_str)) != "toolresult" {
                        continue;
                    }
                    let Some(call_id) = block
                        .get("tool_use_id")
                        .or_else(|| block.get("toolUseId"))
                        .and_then(Value::as_str)
                    else {
                        builder.mark_tools_lower_bound(
                            "tool_result_missing_id",
                            "A Claude tool result had no tool-use ID.",
                        );
                        continue;
                    };
                    builder.complete_tool(
                        call_id,
                        tool_result_status(block),
                        source,
                        timestamp,
                        value.get("toolUseResult"),
                    );
                }
            }
        }
        "assistant" if assistant_record_is_visible(value) => {
            let Some(message) = value.get("message") else {
                builder.mark_all_lower_bound(
                    "malformed_assistant_record",
                    "A Claude assistant record had no message payload.",
                );
                return;
            };
            let Some(blocks) = message.get("content").and_then(Value::as_array) else {
                builder.mark_all_lower_bound(
                    "malformed_assistant_content",
                    "A Claude assistant record had no content block array.",
                );
                return;
            };
            let message_id = message
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.get("uuid").and_then(Value::as_str))
                .filter(|id| !id.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("offset-{}", source.offset));
            let text = record_text(value);
            let stop_reason = normalized_type(message.get("stop_reason").and_then(Value::as_str));
            let output_tokens = message
                .get("usage")
                .and_then(|usage| usage.get("output_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let has_tool_use = blocks.iter().any(|block| {
                matches!(
                    normalized_type(block.get("type").and_then(Value::as_str)).as_str(),
                    "tooluse" | "servertooluse"
                )
            });
            builder.observe_assistant(
                message_id.clone(),
                timestamp,
                source,
                text,
                blocks.len(),
                output_tokens,
                stop_reason,
                has_tool_use,
            );
            let turn_id = builder
                .current_turn_id
                .clone()
                .unwrap_or_else(|| "claude:turn:orphan".to_string());
            let parent_event_id = Some(format!("claude:message:{message_id}"));
            for (block_index, block) in blocks.iter().enumerate() {
                let block_type = normalized_type(block.get("type").and_then(Value::as_str));
                match block_type.as_str() {
                    "tooluse" | "servertooluse" => {
                        let call_id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| {
                                builder.mark_tools_lower_bound(
                                    "tool_use_missing_id",
                                    "A Claude tool-use block had no provider call ID.",
                                );
                                format!("offset-{}-{block_index}", source.offset)
                            });
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let category = classify_claude_tool(&name);
                        let input = block.get("input").unwrap_or(&Value::Null);
                        builder.add_tool(
                            call_id,
                            name,
                            category,
                            turn_id.clone(),
                            timestamp,
                            source,
                            block_index,
                            affected_paths(
                                input,
                                category,
                                context.project_root,
                                context.project_root_aliases,
                            ),
                            parent_event_id.clone(),
                        );
                    }
                    "toolresult" | "servertoolresult" | "websearchtoolresult"
                    | "webfetchtoolresult" | "codeexecutiontoolresult" => {
                        if let Some(call_id) = block
                            .get("tool_use_id")
                            .or_else(|| block.get("toolUseId"))
                            .and_then(Value::as_str)
                        {
                            builder.complete_tool(
                        call_id,
                        tool_result_status(block),
                        source,
                        timestamp,
                        value.get("toolUseResult"),
                    );
                        }
                    }
                    "text" | "thinking" | "redactedthinking" | "image" | "document"
                    | "searchresult" => {}
                    other if other.contains("tool") => builder.mark_tools_lower_bound(
                        "unknown_tool_block",
                        "The Claude transcript contains a tool content block this Mallard version does not recognize.",
                    ),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundedLine {
    Eof,
    Line,
    Oversized,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
    max_bytes: usize,
) -> std::io::Result<BoundedLine> {
    output.clear();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if output.is_empty() && !oversized {
                return Ok(BoundedLine::Eof);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(available.len());
        if !oversized {
            if output.len().saturating_add(content_len) > max_bytes {
                oversized = true;
                output.clear();
            } else {
                output.extend_from_slice(&available[..content_len]);
            }
        }
        let consumed = content_len + usize::from(newline.is_some());
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if oversized {
        Ok(BoundedLine::Oversized)
    } else {
        if output.last() == Some(&b'\r') {
            output.pop();
        }
        Ok(BoundedLine::Line)
    }
}

pub(crate) fn build_timeline_index(
    path: &Path,
    context: &SessionTimelineContext<'_>,
) -> Result<SessionTimelineIndex, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("inspect '{}': {error}", path.display()))?;
    if metadata.len() > conversation_file_limit_bytes() {
        return Err(format!(
            "Claude session '{}' exceeds {} bytes",
            path.display(),
            conversation_file_limit_bytes()
        ));
    }
    let file = File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut builder = ClaudeTimelineBuilder::default();
    let mut bytes = Vec::new();
    let mut record_count = 0usize;
    loop {
        let start = reader
            .stream_position()
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        let line = read_bounded_line(&mut reader, &mut bytes, max_line_bytes())
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        if line == BoundedLine::Eof {
            break;
        }
        let end = reader
            .stream_position()
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        record_count = record_count.saturating_add(1);
        if record_count > MAX_CLAUDE_RECORDS {
            builder.mark_all_lower_bound(
                "record_limit",
                "The Claude transcript contains more records than Mallard can index.",
            );
            break;
        }
        if line == BoundedLine::Oversized {
            builder.mark_all_lower_bound(
                "oversized_record",
                "At least one Claude record exceeded the bounded session timeline parsing limit.",
            );
            continue;
        }
        if bytes.is_empty() {
            continue;
        }
        let value = match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(_) => {
                builder.mark_all_lower_bound(
                    "malformed_record",
                    "At least one Claude record was malformed and could not be indexed.",
                );
                continue;
            }
        };
        let value = match context.child {
            Some(child) => match normalize_child_record(value, child) {
                Some(value) => value,
                None => continue,
            },
            None => value,
        };
        consume_claude_record(&mut builder, &value, source_span(start, end), context);
    }
    Ok(builder.finalize())
}

fn read_source_value(file: &mut File, span: SessionSourceSpan) -> Result<Value, String> {
    if span.length == 0 || span.length > (max_line_bytes() as u64).saturating_add(2) {
        return Err("timeline source span is outside the bounded record limit".to_string());
    }
    let file_len = file
        .metadata()
        .map_err(|error| format!("inspect timeline transcript: {error}"))?
        .len();
    let end = span
        .offset
        .checked_add(span.length)
        .ok_or_else(|| "timeline source span overflowed".to_string())?;
    if end > file_len {
        return Err("timeline source span exceeds the current transcript".to_string());
    }
    file.seek(SeekFrom::Start(span.offset))
        .map_err(|error| format!("seek timeline transcript: {error}"))?;
    let mut bytes = vec![0u8; span.length as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("read timeline transcript: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|_| "timeline source record is malformed".to_string())
}

const BINARY_PREVIEW_OMISSION: &str = "[binary or base64 content omitted]";

fn redact_binary_preview_content(value: &Value) -> Value {
    match value {
        Value::String(text) if looks_like_binary_or_base64(text) => {
            Value::String(BINARY_PREVIEW_OMISSION.to_string())
        }
        Value::Array(values) => {
            Value::Array(values.iter().map(redact_binary_preview_content).collect())
        }
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), redact_binary_preview_content(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn value_as_preview(value: &Value) -> Option<String> {
    match redact_binary_preview_content(value) {
        Value::Null => None,
        Value::String(text) => Some(text),
        other => serde_json::to_string_pretty(&other).ok(),
    }
}

fn find_tool_block<'a>(value: &'a Value, call_id: &str) -> Option<&'a Value> {
    content_blocks(value)?.iter().find(|block| {
        block.get("id").and_then(Value::as_str) == Some(call_id)
            || block
                .get("tool_use_id")
                .or_else(|| block.get("toolUseId"))
                .and_then(Value::as_str)
                == Some(call_id)
    })
}

fn tool_payloads(
    file: &mut File,
    entry: &SessionTimelineIndexEntry,
) -> Result<(Option<String>, Option<String>, String), String> {
    let tool = entry
        .tool
        .as_ref()
        .ok_or_else(|| "timeline event is not a tool call".to_string())?;
    let call_value = read_source_value(file, entry.source)?;
    let input =
        find_tool_block(&call_value, &tool.provider_call_id).and_then(|block| block.get("input"));
    let arguments = input.and_then(value_as_preview);
    let summary = tool_summary(tool, input);
    let result = if let Some(result_source) = tool.result_source {
        let result_value = if result_source == entry.source {
            call_value
        } else {
            read_source_value(file, result_source)?
        };
        find_tool_block(&result_value, &tool.provider_call_id)
            .and_then(|block| block.get("content").or_else(|| block.get("result")))
            .and_then(value_as_preview)
    } else {
        None
    };
    Ok((arguments, result, summary))
}

fn looks_like_binary_or_base64(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.starts_with("data:") || trimmed.contains(";base64,") {
        return true;
    }
    trimmed.len() >= 512
        && trimmed.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_' | b'\r' | b'\n')
        })
}

fn bounded_text(value: Option<String>, limit: usize) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    if looks_like_binary_or_base64(&value) {
        return (Some(BINARY_PREVIEW_OMISSION.to_string()), true);
    }
    if value.len() <= limit {
        return (Some(value), false);
    }
    const ELLIPSIS: &str = "…";
    let mut end = limit.saturating_sub(ELLIPSIS.len()).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut preview = value[..end].to_string();
    if ELLIPSIS.len() <= limit {
        preview.push_str(ELLIPSIS);
    }
    (Some(preview), true)
}

fn matching_entry(entry: &SessionTimelineIndexEntry, query: &SessionTimelineQuery) -> bool {
    if !query.included_event_kinds.contains(&entry.kind) {
        return false;
    }
    if let Some(turn_id) = &query.turn_id {
        if entry.turn_id.as_deref() != Some(turn_id.as_str()) {
            return false;
        }
    }
    // Categories partition tool calls; an agent row answers to its own kind.
    if entry.kind == SessionTimelineEventKind::AgentCall {
        return true;
    }
    if let Some(tool) = &entry.tool {
        query.included_tool_categories.contains(&tool.category)
    } else {
        true
    }
}

const TOOL_SUMMARY_KEYS: &[&str] = &[
    "command",
    "file_path",
    "filePath",
    "notebook_path",
    "notebookPath",
    "path",
    "query",
    "url",
    "pattern",
    "glob",
    "subject",
    "title",
    "summary",
    "description",
    "reason",
    "message",
    "script",
    "code",
    "text",
    "prompt",
    "question",
    "skill",
    "action",
    "preset",
    "caption",
    "planFilePath",
    "task_id",
    "taskId",
    "serverId",
    "name",
    "content",
    "activeForm",
    "questions",
    "todos",
    "files",
    "__unparsedToolInput",
    "status",
];

fn tool_summary_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty() && !looks_like_binary_or_base64(text))
                .then(|| truncate_chars(text, 160))
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        Value::Array(values) => values.iter().find_map(tool_summary_value),
        Value::Object(object) => TOOL_SUMMARY_KEYS
            .iter()
            .find_map(|key| object.get(*key).and_then(tool_summary_value)),
        Value::Null => None,
    }
}

fn tool_summary(tool: &SessionToolIndexEntry, input: Option<&Value>) -> String {
    if let Some(path) = tool.affected_paths.first() {
        return path.clone();
    }
    input
        .and_then(tool_summary_value)
        .unwrap_or_else(|| tool.native_name.clone())
}

/// The kinds that carry a call payload rather than a message body.
fn is_call_kind(kind: SessionTimelineEventKind) -> bool {
    matches!(
        kind,
        SessionTimelineEventKind::ToolCall | SessionTimelineEventKind::AgentCall
    )
}

pub(crate) fn read_timeline_page(
    path: &Path,
    index: &SessionTimelineIndex,
    query: &SessionTimelineQuery,
    context: &SessionTimelineContext<'_>,
) -> Result<SessionTimelinePageSlice, String> {
    if index.parser_version != TIMELINE_PARSER_VERSION {
        return Err("Claude timeline index version is stale".to_string());
    }
    let mut file =
        File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let limit = query.limit.clamp(1, 50);
    let start = query.start_index.unwrap_or(0).min(index.events.len());
    let summaries = index
        .events
        .iter()
        .filter_map(|entry| Some((entry.turn_id.clone()?, entry.turn_tool_summary.clone()?)))
        .collect::<BTreeMap<_, _>>();
    let mut summary_already_emitted = BTreeSet::new();
    for entry in index.events.iter().take(start) {
        if matching_entry(entry, query) && !is_call_kind(entry.kind) {
            if let Some(turn_id) = &entry.turn_id {
                summary_already_emitted.insert(turn_id.clone());
            }
        }
    }
    let mut events = Vec::new();
    let mut page_content_bytes = 0usize;
    let mut position = start;
    while position < index.events.len() && events.len() < limit {
        let entry = &index.events[position];
        if !matching_entry(entry, query) {
            position += 1;
            continue;
        }
        let text = if is_call_kind(entry.kind) {
            None
        } else {
            let value = read_source_value(&mut file, entry.source)?;
            bounded_text(record_text(&value), timeline_message_preview_bytes()).0
        };
        let is_agent = entry.kind == SessionTimelineEventKind::AgentCall;
        let tool = if let Some(tool_index) = &entry.tool {
            let (arguments, result, summary) = tool_payloads(&mut file, entry)?;
            // An agent row carries its brief and report on the agent preview;
            // the raw payload stays one click away through the event detail.
            let (argument_preview, argument_truncated) = if is_agent {
                (None, false)
            } else {
                bounded_text(arguments, timeline_tool_preview_bytes())
            };
            let (result_preview, result_truncated) = if is_agent {
                (None, false)
            } else {
                bounded_text(result, timeline_tool_preview_bytes())
            };
            Some(SessionToolCallPreview {
                provider_call_id: tool_index.provider_call_id.clone(),
                native_name: tool_index.native_name.clone(),
                category: tool_index.category,
                status: tool_index.status,
                summary,
                argument_preview,
                argument_truncated,
                result_preview,
                result_truncated,
                affected_paths: tool_index.affected_paths.clone(),
            })
        } else {
            None
        };
        let agent = is_agent
            .then(|| describe_agent(&mut file, path, entry, context))
            .transpose()?;
        let event_content_bytes = text
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(
                tool.as_ref()
                    .and_then(|preview| preview.argument_preview.as_ref())
                    .map_or(0, String::len),
            )
            .saturating_add(
                tool.as_ref()
                    .and_then(|preview| preview.result_preview.as_ref())
                    .map_or(0, String::len),
            )
            .saturating_add(agent.as_ref().map_or(0, agent_preview_bytes));
        if !events.is_empty()
            && page_content_bytes.saturating_add(event_content_bytes)
                > timeline_page_content_bytes()
        {
            break;
        }
        page_content_bytes = page_content_bytes.saturating_add(event_content_bytes);
        position += 1;
        let mut turn_tool_summary = None;
        if !is_call_kind(entry.kind) {
            if let Some(turn_id) = &entry.turn_id {
                if summary_already_emitted.insert(turn_id.clone()) {
                    turn_tool_summary = summaries.get(turn_id).cloned();
                }
            }
        }
        events.push(SessionTimelineEvent {
            event_id: entry.event_id.clone(),
            ordinal: entry.ordinal,
            timestamp: entry.timestamp,
            kind: entry.kind,
            quality: entry.quality,
            turn_id: entry.turn_id.clone(),
            turn_tool_summary,
            parent_event_id: entry.parent_event_id.clone(),
            text,
            tool,
            checkpoint: None,
            agent,
        });
    }
    let next_index = index.events[position..]
        .iter()
        .any(|entry| matching_entry(entry, query))
        .then_some(position);
    Ok(SessionTimelinePageSlice { events, next_index })
}

/// What an agent preview adds to a page: its two texts.
pub(crate) fn agent_preview_bytes(agent: &SessionAgentPreview) -> usize {
    agent
        .brief
        .as_ref()
        .map_or(0, String::len)
        .saturating_add(agent.report.as_ref().map_or(0, String::len))
}

/// The child transcript's clock and closing message, from its tail alone.
#[derive(Default)]
struct ChildTail {
    last_timestamp: Option<u64>,
    report: Option<String>,
    /// The last durable record is a final assistant message: the child said
    /// its piece and stopped.
    finished: bool,
}

fn claude_child_tail(path: &Path) -> Result<ChildTail, String> {
    let window = crate::jsonl_tail_io::read_suffix_window(path, CHILD_TAIL_WINDOW_BYTES)?;
    let bytes = window.bytes.as_slice();
    // Only complete records: drop the head of a mid-file window and a torn
    // last line, the way the scan window does.
    let start = if window.window_offset == 0 {
        0
    } else {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |position| position + 1)
    };
    let end = bytes[start..]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(start, |position| start + position + 1);
    let mut tail = ChildTail::default();
    for line in bytes[start..end].split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if let Some(timestamp) = value.get("timestamp").and_then(parse_timestamp_value) {
            if tail.last_timestamp.is_none_or(|last| timestamp > last) {
                tail.last_timestamp = Some(timestamp);
            }
        }
        match normalized_type(value.get("type").and_then(Value::as_str)).as_str() {
            "assistant" => {
                let stop_reason = normalized_type(
                    value
                        .get("message")
                        .and_then(|message| message.get("stop_reason"))
                        .and_then(Value::as_str),
                );
                if let Some(text) = record_text(&value).filter(|text| !text.trim().is_empty()) {
                    tail.report = Some(text);
                    tail.finished = is_terminal_response_stop_reason(&stop_reason);
                } else if value
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| !blocks.is_empty())
                {
                    tail.finished = false;
                }
            }
            "user" => tail.finished = false,
            _ => {}
        }
    }
    Ok(tail)
}

/// Written within the last five minutes: the same rule that marks a session
/// active on the activity list.
fn transcript_is_live(path: &Path) -> bool {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed.as_secs() < LIVE_TRANSCRIPT_SECS)
}

pub(crate) fn describe_agent_call(
    path: &Path,
    entry: &SessionTimelineIndexEntry,
    context: &SessionTimelineContext<'_>,
) -> Result<SessionAgentPreview, String> {
    let mut file =
        File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    describe_agent(&mut file, path, entry, context)
}

/// One agent row from the call, its result, and the child's tail. The call
/// record names the task (`description`, `subagent_type`, `prompt`); the
/// result record names the child and, when the parent waited for it, its end,
/// its report, and its own accounting. A background child is read from its
/// transcript's tail instead: last clock, final message, and whether it said
/// one at all.
fn describe_agent(
    file: &mut File,
    path: &Path,
    entry: &SessionTimelineIndexEntry,
    context: &SessionTimelineContext<'_>,
) -> Result<SessionAgentPreview, String> {
    let tool = entry
        .tool
        .as_ref()
        .ok_or_else(|| "agent call has no call metadata".to_string())?;
    let agent = entry.agent.clone().unwrap_or_default();
    let call_value = read_source_value(file, entry.source)?;
    let input = find_tool_block(&call_value, &tool.provider_call_id)
        .and_then(|block| block.get("input"))
        .cloned()
        .unwrap_or(Value::Null);
    let (result_value, result_timestamp) = match tool.result_source {
        Some(span) if span != entry.source => {
            let value = read_source_value(file, span)?;
            let timestamp = value.get("timestamp").and_then(parse_timestamp_value);
            (Some(value), timestamp)
        }
        Some(_) => (Some(call_value.clone()), entry.timestamp),
        None => (None, None),
    };
    let result_meta = result_value
        .as_ref()
        .and_then(|value| value.get("toolUseResult"))
        .cloned()
        .unwrap_or(Value::Null);
    // The report is prose: the text blocks of the result, not their JSON.
    let result_text = result_value
        .as_ref()
        .and_then(|value| find_tool_block(value, &tool.provider_call_id))
        .and_then(|block| block.get("content").or_else(|| block.get("result")))
        .and_then(|content| match content {
            Value::String(text) => Some(text.clone()),
            Value::Array(blocks) => {
                let text = blocks
                    .iter()
                    .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                (!text.is_empty()).then_some(text)
            }
            other => value_as_preview(other),
        });
    let text_field = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| truncate_chars(text, 160))
    };
    let waited = result_meta.get("isAsync").and_then(Value::as_bool) != Some(true);
    let label = text_field(&input, "description")
        .or_else(|| text_field(&result_meta, "description"))
        .unwrap_or_else(|| "Agent".to_string());
    let role =
        text_field(&input, "subagent_type").or_else(|| text_field(&result_meta, "agentType"));
    let model = text_field(&result_meta, "resolvedModel");
    let (brief, brief_truncated) = bounded_text(
        input
            .get("prompt")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        timeline_tool_preview_bytes(),
    );
    // A child of a child is not addressed through this page: nested
    // delegations are read on the parent's page, never followed from here.
    let child_path = agent
        .provider_agent_id
        .as_deref()
        .filter(|_| context.child.is_none())
        .and_then(|id| super::subagents::child_path(path, context.provider_session_id, id).ok());
    let child_exists = child_path.as_ref().is_some_and(|child| {
        super::subagents::child_exists(child, context.provider_home) == Ok(true)
    });
    let mut ended_at = None;
    let mut report = None;
    let mut tool_calls = None;
    let mut tokens = None;
    let status = if tool.status == SessionToolCallStatus::Failed {
        ended_at = result_timestamp;
        report = result_text;
        SessionAgentStatus::Failed
    } else if waited && tool.status == SessionToolCallStatus::Completed {
        ended_at = result_timestamp;
        report = result_text;
        tool_calls = result_meta.get("totalToolUseCount").and_then(Value::as_u64);
        tokens = result_meta.get("totalTokens").and_then(Value::as_u64);
        SessionAgentStatus::Completed
    } else if agent.provider_agent_id.is_none() {
        // Called, never answered: still running if the parent is being
        // written, else cut off with it.
        if transcript_is_live(path) {
            SessionAgentStatus::Running
        } else {
            SessionAgentStatus::Incomplete
        }
    } else if let Some(child) = child_path.as_ref().filter(|_| child_exists) {
        let tail = claude_child_tail(child)?;
        ended_at = tail.last_timestamp;
        report = tail.report;
        if tail.finished {
            SessionAgentStatus::Completed
        } else if transcript_is_live(child) {
            SessionAgentStatus::Running
        } else {
            SessionAgentStatus::Incomplete
        }
    } else {
        SessionAgentStatus::Unavailable
    };
    let (report, report_truncated) = bounded_text(report, timeline_tool_preview_bytes());
    Ok(SessionAgentPreview {
        transcript_resource_id: agent
            .provider_agent_id
            .as_deref()
            .filter(|_| child_exists)
            .map(|id| agent_resource_id(context.resource_id, id)),
        provider_agent_id: agent.provider_agent_id,
        label,
        task: None,
        role,
        model,
        mode: if waited {
            SessionAgentMode::Sync
        } else {
            SessionAgentMode::Async
        },
        status,
        started_at: entry.timestamp,
        ended_at,
        tool_calls,
        tokens,
        brief,
        brief_truncated,
        report,
        report_truncated,
    })
}

pub(crate) fn read_event_detail(
    path: &Path,
    index: &SessionTimelineIndex,
    event_id: &str,
) -> Result<SessionEventDetail, String> {
    if index.parser_version != TIMELINE_PARSER_VERSION {
        return Err("Claude timeline index version is stale".to_string());
    }
    let entry = index
        .events
        .iter()
        .find(|entry| entry.event_id == event_id)
        .ok_or_else(|| "timeline event is not present in this snapshot".to_string())?;
    let tool = entry
        .tool
        .as_ref()
        .ok_or_else(|| "only tool-call events have expandable detail".to_string())?;
    let mut file =
        File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let (arguments, result, _) = tool_payloads(&mut file, entry)?;
    let (argument_budget, result_budget) =
        timeline_tool_detail_budgets(arguments.as_deref(), result.as_deref());
    let (arguments, arguments_truncated) = bounded_text(arguments, argument_budget);
    let (result, result_truncated) = bounded_text(result, result_budget);
    Ok(SessionEventDetail {
        event_id: event_id.to_string(),
        arguments,
        arguments_truncated,
        result,
        result_truncated,
        affected_paths: tool.affected_paths.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn jsonl_bytes(values: Vec<Value>) -> Vec<u8> {
        let mut transcript = values
            .into_iter()
            .map(|value| serde_json::to_string(&value).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        transcript.push('\n');
        transcript.into_bytes()
    }

    fn write_jsonl(path: &Path, values: Vec<Value>) {
        fs::write(path, jsonl_bytes(values)).unwrap();
    }

    fn context<'a>(project_root: &'a Path) -> SessionTimelineContext<'a> {
        SessionTimelineContext {
            resource_id: "conversation:claude:test",
            provider_session_id: "session-1",
            project_root,
            project_root_aliases: &[],
            provider_home: project_root,
            child: None,
        }
    }

    fn count_value(count: &SessionCount) -> u64 {
        count.value.expect("supported count")
    }

    #[test]
    fn activity_summary_uses_resumed_leaf_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let transcript = temp.path().join("resumed-session.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({
                    "type": "user",
                    "uuid": "user-original",
                    "parentUuid": null,
                    "sessionId": "original-session",
                    "cwd": "/tmp/original",
                    "message": {"role": "user", "content": "original"}
                }),
                json!({
                    "type": "assistant",
                    "uuid": "assistant-resumed",
                    "parentUuid": "user-original",
                    "sessionId": "resumed-session",
                    "cwd": "/tmp/resumed",
                    "message": {"role": "assistant", "content": [{
                        "type": "text", "text": "resumed"
                    }]}
                }),
            ],
        );

        let summary = summarize_file(&transcript).unwrap();
        assert_eq!(summary.session_id, "resumed-session");
    }

    #[test]
    fn activity_summary_prefers_current_claude_titles() {
        let base_records = || {
            vec![
                json!({
                    "type": "user",
                    "uuid": "command-1",
                    "parentUuid": null,
                    "sessionId": "session-1",
                    "isSidechain": false,
                    "message": {
                        "role": "user",
                        "content": "<command-name>/autocompact</command-name> <command-message>autocompact</command-message>"
                    }
                }),
                json!({
                    "type": "user",
                    "uuid": "tool-result-1",
                    "parentUuid": "command-1",
                    "sessionId": "session-1",
                    "message": {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "tool-1",
                            "content": "ignored"
                        }]
                    }
                }),
                json!({
                    "type": "user",
                    "uuid": "human-1",
                    "parentUuid": "tool-result-1",
                    "sessionId": "session-1",
                    "message": {
                        "role": "user",
                        "content": "Investigate path remapping"
                    }
                }),
            ]
        };
        let summarize = |title_records: Vec<Value>| {
            let mut records = base_records();
            records.extend(title_records);
            summarize_bytes(&jsonl_bytes(records)).unwrap()
        };

        let prompt_only = summarize(Vec::new());
        assert_eq!(
            prompt_only.title.as_deref(),
            Some("Investigate path remapping")
        );
        assert_eq!(
            prompt_only.summary.as_deref(),
            Some("Investigate path remapping")
        );
        assert_eq!(prompt_only.user_rounds, 1);

        let existing_summary = summarize(vec![json!({
            "type": "summary",
            "sessionId": "session-1",
            "summary": "Existing summary title"
        })]);
        assert_eq!(
            existing_summary.title.as_deref(),
            Some("Existing summary title")
        );

        let generated = summarize(vec![
            json!({
                "type": "summary",
                "sessionId": "session-1",
                "summary": "Existing summary title"
            }),
            json!({
                "type": "ai-title",
                "sessionId": "session-1",
                "aiTitle": "Research path remapping after push/pull across machines"
            }),
        ]);
        assert_eq!(
            generated.title.as_deref(),
            Some("Research path remapping after push/pull across machines")
        );

        let custom = summarize(vec![
            json!({
                "type": "ai-title",
                "sessionId": "session-1",
                "aiTitle": "Generated title"
            }),
            json!({
                "type": "custom-title",
                "sessionId": "session-1",
                "customTitle": "Original title"
            }),
            json!({
                "type": "custom-title",
                "sessionId": "session-1",
                "customTitle": "   "
            }),
            json!({
                "type": "custom-title",
                "sessionId": "session-1",
                "customTitle": "  Remapping\nworkspace  "
            }),
        ]);
        assert_eq!(custom.title.as_deref(), Some("Remapping workspace"));
        assert_eq!(
            custom.summary.as_deref(),
            Some("Investigate path remapping")
        );
    }

    #[test]
    fn human_messages_tools_and_visible_phases_are_normalized_once() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"timestamp":"2026-08-07T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"Read the source"}]}}),
                json!({"type":"user","uuid":"meta-1","isMeta":true,"sessionId":"session-1","message":{"role":"user","content":[{"type":"text","text":"<system-reminder>injected</system-reminder>"}]}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","timestamp":"2026-08-07T00:00:01Z","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","usage":{"output_tokens":10},"content":[{"type":"thinking","thinking":"hidden chain of thought"},{"type":"text","text":"I will inspect it."},{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}),
                json!({"type":"assistant","uuid":"a-1-replay","sessionId":"session-1","timestamp":"2026-08-07T00:00:01Z","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","usage":{"output_tokens":20},"content":[{"type":"thinking","thinking":"hidden chain of thought"},{"type":"text","text":"I will inspect it."},{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}),
                json!({"type":"user","uuid":"tool-result-1","sessionId":"session-1","timestamp":"2026-08-07T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":"source contents"}]}}),
                json!({"type":"assistant","uuid":"a-2","sessionId":"session-1","timestamp":"2026-08-07T00:00:03Z","message":{"id":"msg-2","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"The source is clear."}]}}),
            ],
        );

        let summary = summarize_file(&transcript).unwrap();
        assert_eq!(summary.user_rounds, 1);
        assert_eq!(summary.agent_messages, 1);

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.prompts), 1);
        assert_eq!(count_value(&index.counts.responses), 1);
        assert_eq!(count_value(&index.counts.intermediate_steps), 1);
        assert_eq!(count_value(&index.counts.tool_calls), 1);
        assert_eq!(count_value(&index.counts.reads), 1);
        assert!(!index
            .events
            .iter()
            .any(|entry| entry.event_id.contains("thinking")));
        let tool_entry = index
            .events
            .iter()
            .find(|entry| entry.tool.is_some())
            .unwrap();
        assert_eq!(
            tool_entry.tool.as_ref().unwrap().status,
            SessionToolCallStatus::Completed
        );
        assert_eq!(
            tool_entry.parent_event_id.as_deref(),
            Some("claude:message:msg-1")
        );
        let prompt = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::Prompt)
            .unwrap();
        assert_eq!(
            prompt
                .turn_tool_summary
                .as_ref()
                .and_then(|summary| summary.tool_calls.value),
            Some(1)
        );

        let response_query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::Response]),
            included_tool_categories: BTreeSet::new(),
            turn_id: None,
        };
        let page = read_timeline_page(
            &transcript,
            &index,
            &response_query,
            &context(&project_root),
        )
        .unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].text.as_deref(), Some("The source is clear."));
        assert_eq!(
            page.events[0]
                .turn_tool_summary
                .as_ref()
                .and_then(|summary| summary.tool_calls.value),
            Some(1)
        );

        let detail = read_event_detail(&transcript, &index, &tool_entry.event_id).unwrap();
        assert!(detail.arguments.as_deref().unwrap().contains("src/lib.rs"));
        assert_eq!(detail.result.as_deref(), Some("source contents"));
    }

    #[test]
    fn token_usage_sums_unique_messages_and_includes_cached_input() {
        let summary = summarize_bytes(&jsonl_bytes(vec![
            json!({
                "type": "user",
                "uuid": "u-1",
                "sessionId": "session-1",
                "message": {"role": "user", "content": "Count tokens"}
            }),
            json!({
                "type": "assistant",
                "uuid": "a-1",
                "sessionId": "session-1",
                "message": {
                    "id": "msg-1",
                    "role": "assistant",
                    "usage": {
                        "input_tokens": 2,
                        "output_tokens": 10,
                        "cache_creation_input_tokens": 100,
                        "cache_read_input_tokens": 1000
                    },
                    "content": []
                }
            }),
            json!({
                "type": "assistant",
                "uuid": "a-1-complete",
                "sessionId": "session-1",
                "message": {
                    "id": "msg-1",
                    "role": "assistant",
                    "usage": {
                        "input_tokens": 2,
                        "output_tokens": 20,
                        "cache_creation_input_tokens": 100,
                        "cache_read_input_tokens": 1000
                    },
                    "content": []
                }
            }),
            json!({
                "type": "assistant",
                "uuid": "a-1-zero-replay",
                "sessionId": "session-1",
                "message": {
                    "id": "msg-1",
                    "role": "assistant",
                    "usage": {
                        "input_tokens": 0,
                        "output_tokens": 0,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0
                    },
                    "content": []
                }
            }),
            json!({
                "type": "assistant",
                "uuid": "a-2",
                "sessionId": "session-1",
                "message": {
                    "id": "msg-2",
                    "role": "assistant",
                    "usage": {
                        "input_tokens": 3,
                        "output_tokens": 30,
                        "cache_creation_input_tokens": 200,
                        "cache_read_input_tokens": 2000
                    },
                    "content": []
                }
            }),
        ]))
        .unwrap();

        assert_eq!(summary.total_input_tokens, Some(3305));
        assert_eq!(summary.total_output_tokens, Some(50));
        assert_eq!(summary.total_tokens, Some(3355));
        assert!(summary.metrics_complete);
    }

    #[test]
    fn replayed_tool_call_after_later_prompt_is_deduplicated_by_provider_call_id() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({
                    "type": "user",
                    "uuid": "u-1",
                    "parentUuid": null,
                    "sessionId": "session-1",
                    "timestamp": "2026-08-07T00:00:00Z",
                    "message": {"role": "user", "content": "Read the source"}
                }),
                json!({
                    "type": "assistant",
                    "uuid": "a-1",
                    "parentUuid": "u-1",
                    "sessionId": "session-1",
                    "timestamp": "2026-08-07T00:00:01Z",
                    "message": {
                        "id": "msg-1",
                        "role": "assistant",
                        "stop_reason": "tool_use",
                        "content": [
                            {"type": "text", "text": "I will inspect it."},
                            {
                                "type": "tool_use",
                                "id": "tool-1",
                                "name": "Read",
                                "input": {"file_path": "src/lib.rs"}
                            }
                        ]
                    }
                }),
                json!({
                    "type": "user",
                    "uuid": "result-1",
                    "parentUuid": "a-1",
                    "sessionId": "session-1",
                    "timestamp": "2026-08-07T00:00:02Z",
                    "message": {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "tool-1",
                            "content": "source contents"
                        }]
                    }
                }),
                json!({
                    "type": "user",
                    "uuid": "u-2",
                    "parentUuid": "result-1",
                    "sessionId": "session-1",
                    "timestamp": "2026-08-07T00:00:03Z",
                    "message": {"role": "user", "content": "Now explain it"}
                }),
                json!({
                    "type": "bridge-session",
                    "sessionId": "session-1",
                    "bridgeSessionId": "bridge-1"
                }),
                json!({
                    "type": "assistant",
                    "uuid": "a-1",
                    "parentUuid": "u-1",
                    "sessionId": "session-1",
                    "slug": "replayed-session",
                    "timestamp": "2026-08-07T00:00:01Z",
                    "message": {
                        "id": "msg-1",
                        "role": "assistant",
                        "stop_reason": "tool_use",
                        "content": [
                            {"type": "text", "text": "I will inspect it."},
                            {
                                "type": "tool_use",
                                "id": "tool-1",
                                "name": "Read",
                                "input": {"file_path": "src/lib.rs"}
                            }
                        ]
                    }
                }),
                json!({
                    "type": "user",
                    "uuid": "result-1",
                    "parentUuid": "a-1",
                    "sessionId": "session-1",
                    "slug": "replayed-session",
                    "timestamp": "2026-08-07T00:00:02Z",
                    "message": {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "tool-1",
                            "content": "source contents"
                        }]
                    }
                }),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.prompts), 2);
        assert_eq!(count_value(&index.counts.intermediate_steps), 1);
        assert_eq!(count_value(&index.counts.tool_calls), 1);
        assert_eq!(count_value(&index.counts.reads), 1);
        assert_eq!(
            index
                .events
                .iter()
                .map(|entry| entry.event_id.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            index.events.len()
        );

        let tool_entry = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::ToolCall)
            .unwrap();
        assert_eq!(tool_entry.event_id, "claude:tool:tool-1");
        assert_eq!(tool_entry.turn_id.as_deref(), Some("claude:turn:u-1"));
        assert_eq!(
            tool_entry.tool.as_ref().unwrap().status,
            SessionToolCallStatus::Completed
        );

        let first_prompt = index
            .events
            .iter()
            .find(|entry| entry.event_id == "claude:prompt:u-1")
            .unwrap();
        assert_eq!(
            first_prompt
                .turn_tool_summary
                .as_ref()
                .and_then(|summary| summary.tool_calls.value),
            Some(1)
        );
        let second_prompt = index
            .events
            .iter()
            .find(|entry| entry.event_id == "claude:prompt:u-2")
            .unwrap();
        assert!(second_prompt.turn_tool_summary.is_none());
        assert!(!index
            .warnings
            .iter()
            .any(|warning| warning.code == "orphan_tool_result"));

        let detail = read_event_detail(&transcript, &index, &tool_entry.event_id).unwrap();
        assert_eq!(detail.result.as_deref(), Some("source contents"));
    }

    #[test]
    fn native_tool_categories_partition_and_edit_paths_are_safe() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join("src")).unwrap();
        let edited = project_root.join("src/lib.rs");
        let transcript = temp.path().join("session.jsonl");
        let tools = vec![
            json!({"type":"tool_use","id":"write-1","name":"Write","input":{"file_path":edited,"content":"new"}}),
            json!({"type":"tool_use","id":"bash-1","name":"Bash","input":{"command":"cargo test"}}),
            json!({"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"src/lib.rs"}}),
            json!({"type":"tool_use","id":"grep-1","name":"Grep","input":{"pattern":"needle"}}),
        ];
        let results = ["write-1", "bash-1", "read-1", "grep-1"]
            .into_iter()
            .map(|id| json!({"type":"tool_result","tool_use_id":id,"content":"ok"}))
            .collect::<Vec<_>>();
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"message":{"role":"user","content":"Do the work"}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":tools}}),
                json!({"type":"user","uuid":"r-1","sessionId":"session-1","message":{"role":"user","content":results}}),
                json!({"type":"assistant","uuid":"a-2","sessionId":"session-1","message":{"id":"msg-2","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.tool_calls), 4);
        assert_eq!(count_value(&index.counts.file_edits), 1);
        assert_eq!(count_value(&index.counts.shell_calls), 1);
        assert_eq!(count_value(&index.counts.reads), 1);
        assert_eq!(count_value(&index.counts.other_tools), 1);
        for count in [
            &index.counts.tool_calls,
            &index.counts.file_edits,
            &index.counts.shell_calls,
            &index.counts.reads,
            &index.counts.other_tools,
        ] {
            assert_eq!(count.quality, SessionCountQuality::Exact);
        }
        let edit = index
            .events
            .iter()
            .filter_map(|entry| entry.tool.as_ref())
            .find(|tool| tool.category == SessionToolCategory::FileEdit)
            .unwrap();
        assert_eq!(edit.affected_paths, vec!["src/lib.rs"]);
        assert!(edit
            .affected_paths
            .iter()
            .all(|path| !Path::new(path).is_absolute()));
    }

    #[test]
    fn structured_tool_summaries_use_distinct_native_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        let first_command = "qlmanage -t first.svg";
        let second_command = "qlmanage -t second.svg";
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"message":{"role":"user","content":"Render both files"}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"tool_use","id":"bash-1","name":"Bash","input":{"command":first_command,"description":"Render the first file"}},
                    {"type":"tool_use","id":"bash-2","name":"Bash","input":{"command":second_command,"description":"Render the second file"}},
                    {"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"./pelican.png"}}
                ]}}),
                json!({"type":"user","uuid":"r-1","sessionId":"session-1","message":{"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"bash-1","content":"first result"},
                    {"type":"tool_result","tool_use_id":"bash-2","content":"second result"},
                    {"type":"tool_result","tool_use_id":"read-1","content":"image result"}
                ]}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        let query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::ToolCall]),
            included_tool_categories: BTreeSet::from([
                SessionToolCategory::Shell,
                SessionToolCategory::Read,
            ]),
            turn_id: None,
        };
        let page =
            read_timeline_page(&transcript, &index, &query, &context(&project_root)).unwrap();
        assert_eq!(page.events.len(), 3);
        let tools = page
            .events
            .iter()
            .map(|event| event.tool.as_ref().unwrap())
            .collect::<Vec<_>>();
        assert_ne!(tools[0].provider_call_id, tools[1].provider_call_id);
        assert_eq!(tools[0].summary, first_command);
        assert_eq!(tools[1].summary, second_command);
        assert_eq!(tools[2].summary, "./pelican.png");
        assert_eq!(tools[0].status, SessionToolCallStatus::Completed);
        assert_eq!(tools[1].status, SessionToolCallStatus::Completed);
        assert_eq!(tools[2].status, SessionToolCallStatus::Completed);
        assert!(tools[0]
            .argument_preview
            .as_deref()
            .is_some_and(|arguments| arguments.contains(first_command)));
        assert!(tools[1]
            .argument_preview
            .as_deref()
            .is_some_and(|arguments| arguments.contains(second_command)));
        assert_eq!(tools[0].result_preview.as_deref(), Some("first result"));
        assert_eq!(tools[1].result_preview.as_deref(), Some("second result"));
        assert!(tools[2]
            .argument_preview
            .as_deref()
            .is_some_and(|arguments| arguments.contains("./pelican.png")));
        assert_eq!(tools[2].result_preview.as_deref(), Some("image result"));
    }

    #[test]
    fn structured_tool_summaries_cover_claude_input_shapes() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        let cases = vec![
            (
                "workflow-1",
                "Workflow",
                json!({"script":"export const meta = {\n  name: 'demo'\n};"}),
                "export const meta = { name: 'demo' };",
            ),
            (
                "search-1",
                "ToolSearch",
                json!({"max_results":5,"query":"select:Read"}),
                "select:Read",
            ),
            (
                "web-1",
                "WebSearch",
                json!({"allowed_domains":["example.com"],"query":"release notes"}),
                "release notes",
            ),
            (
                "agent-1",
                "Agent",
                json!({"description":"Inspect the renderer","prompt":"A much longer delegated prompt"}),
                "Inspect the renderer",
            ),
            (
                "question-1",
                "AskUserQuestion",
                json!({"questions":[{"header":"Target","question":"Which target?","options":[]}]}),
                "Which target?",
            ),
            (
                "task-1",
                "TaskCreate",
                json!({"activeForm":"Adding tests","description":"Detailed task body","subject":"Add tests"}),
                "Add tests",
            ),
            (
                "wake-1",
                "ScheduleWakeup",
                json!({"delaySeconds":30,"prompt":"Check again later","reason":"Wait for render"}),
                "Wait for render",
            ),
            (
                "notify-1",
                "PushNotification",
                json!({"message":"Render complete","status":"success"}),
                "Render complete",
            ),
            (
                "file-1",
                "SendUserFile",
                json!({"files":["./build/report.txt"],"status":"ready"}),
                "./build/report.txt",
            ),
            (
                "browser-1",
                "mcp__Claude_Browser__navigate",
                json!({"force":true,"url":"https://example.com/docs"}),
                "https://example.com/docs",
            ),
            (
                "todo-1",
                "TodoWrite",
                json!({"todos":[{"activeForm":"Checking output","content":"Check output","status":"pending"}]}),
                "Check output",
            ),
            (
                "unknown-1",
                "ResizeWindow",
                json!({"height":768,"width":1024}),
                "ResizeWindow",
            ),
        ];
        let tools = cases
            .iter()
            .map(
                |(id, name, input, _)| json!({"type":"tool_use","id":id,"name":name,"input":input}),
            )
            .collect::<Vec<_>>();
        let results = cases
            .iter()
            .map(|(id, _, _, _)| json!({"type":"tool_result","tool_use_id":id,"content":"ok"}))
            .collect::<Vec<_>>();
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"message":{"role":"user","content":"Run the tools"}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":tools}}),
                json!({"type":"user","uuid":"r-1","sessionId":"session-1","message":{"role":"user","content":results}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        let query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([
                SessionTimelineEventKind::ToolCall,
                SessionTimelineEventKind::AgentCall,
            ]),
            included_tool_categories: BTreeSet::from([SessionToolCategory::Other]),
            turn_id: None,
        };
        let page =
            read_timeline_page(&transcript, &index, &query, &context(&project_root)).unwrap();
        assert_eq!(page.events.len(), cases.len());
        let summaries = page
            .events
            .iter()
            .map(|event| {
                let tool = event.tool.as_ref().unwrap();
                assert_eq!(tool.status, SessionToolCallStatus::Completed);
                (tool.native_name.as_str(), tool.summary.as_str())
            })
            .collect::<BTreeMap<_, _>>();
        for (_, name, _, expected) in &cases {
            assert_eq!(summaries.get(name).copied(), Some(*expected));
        }
    }

    #[test]
    fn timeline_indexes_multi_mib_image_results_without_exposing_base64() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        let image_data = "A".repeat(2 * 1024 * 1024);
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"message":{"role":"user","content":"Inspect the image"}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"./preview.png"}}
                ]}}),
                json!({"type":"user","uuid":"r-1","sessionId":"session-1","message":{"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"read-1","content":[
                        {"type":"image","source":{"type":"base64","media_type":"image/png","data":image_data}}
                    ]}
                ]}}),
            ],
        );

        assert_eq!(max_line_bytes() as u64, conversation_file_limit_bytes());
        assert!(fs::metadata(&transcript).unwrap().len() > 2 * 1024 * 1024);
        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(index.parser_version, TIMELINE_PARSER_VERSION);
        assert!(!index.truncated);
        assert!(index.warnings.is_empty(), "{:?}", index.warnings);
        assert_eq!(index.counts.tool_calls.quality, SessionCountQuality::Exact);
        let tool_entry = index
            .events
            .iter()
            .find(|entry| entry.tool.is_some())
            .unwrap();
        assert_eq!(
            tool_entry.tool.as_ref().unwrap().status,
            SessionToolCallStatus::Completed
        );

        let query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::ToolCall]),
            included_tool_categories: BTreeSet::from([SessionToolCategory::Read]),
            turn_id: None,
        };
        let page =
            read_timeline_page(&transcript, &index, &query, &context(&project_root)).unwrap();
        let preview = page.events[0]
            .tool
            .as_ref()
            .and_then(|tool| tool.result_preview.as_ref())
            .unwrap();
        assert!(preview.contains(BINARY_PREVIEW_OMISSION));
        assert!(preview.contains("image/png"));
        assert!(!preview.contains(&"A".repeat(512)));
        assert!(preview.len() <= timeline_tool_preview_bytes());

        let detail = read_event_detail(&transcript, &index, &tool_entry.event_id).unwrap();
        let result = detail.result.unwrap();
        assert!(result.contains(BINARY_PREVIEW_OMISSION));
        assert!(result.contains("image/png"));
        assert!(!result.contains(&"A".repeat(512)));
    }

    #[test]
    fn large_tool_previews_paginate_without_skipping_the_next_event() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        let arguments = "a ".repeat(300 * 1024);
        let result = "r ".repeat(300 * 1024);
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"message":{"role":"user","content":"Run both tools"}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"tool_use","id":"tool-1","name":"Custom","input":{"prompt":arguments.clone()}},
                    {"type":"tool_use","id":"tool-2","name":"Custom","input":{"prompt":arguments.clone()}}
                ]}}),
                json!({"type":"user","uuid":"r-1","sessionId":"session-1","message":{"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"tool-1","content":result.clone()},
                    {"type":"tool_result","tool_use_id":"tool-2","content":result.clone()}
                ]}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        let first_query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::ToolCall]),
            included_tool_categories: BTreeSet::from([SessionToolCategory::Other]),
            turn_id: None,
        };
        let first_page =
            read_timeline_page(&transcript, &index, &first_query, &context(&project_root)).unwrap();
        assert_eq!(first_page.events.len(), 1);
        let first_tool = first_page.events[0].tool.as_ref().unwrap();
        assert_eq!(first_tool.provider_call_id, "tool-1");
        assert!(first_tool.argument_preview.as_ref().unwrap().len() > 512 * 1024);
        assert_eq!(
            first_tool.result_preview.as_ref().unwrap().len(),
            result.len()
        );
        assert!(!first_tool.argument_truncated);
        assert!(!first_tool.result_truncated);

        let second_query = SessionTimelineQuery {
            start_index: first_page.next_index,
            ..first_query
        };
        let second_page =
            read_timeline_page(&transcript, &index, &second_query, &context(&project_root))
                .unwrap();
        assert_eq!(second_page.events.len(), 1);
        assert_eq!(
            second_page.events[0]
                .tool
                .as_ref()
                .unwrap()
                .provider_call_id,
            "tool-2"
        );
        assert_eq!(second_page.next_index, None);

        let first_entry = index
            .events
            .iter()
            .find(|entry| {
                entry
                    .tool
                    .as_ref()
                    .is_some_and(|tool| tool.provider_call_id == "tool-1")
            })
            .unwrap();
        let detail = read_event_detail(&transcript, &index, &first_entry.event_id).unwrap();
        let detail_bytes = detail.arguments.as_ref().map_or(0, String::len)
            + detail.result.as_ref().map_or(0, String::len);
        assert!(detail.arguments_truncated);
        assert!(detail.result_truncated);
        assert!(detail.arguments.as_ref().unwrap().len() > 64 * 1024);
        assert!(detail.result.as_ref().unwrap().len() > 64 * 1024);
        assert!(detail_bytes <= crate::app_settings::timeline_tool_detail_bytes());
    }

    #[test]
    fn edit_paths_choose_the_most_specific_root_alias_and_reject_a_prefix_sibling() {
        let temp = tempfile::tempdir().unwrap();
        let transcript = temp.path().join("session.jsonl");
        let canonical_root = PathBuf::from("/tmp/project");
        let selected_root = PathBuf::from("/tmp/project/alias");
        let aliases = vec![canonical_root.clone(), selected_root.clone()];
        let inside = selected_root.join("src/lib.rs");
        let outside = PathBuf::from("/tmp/project_external/alias_external/outside.rs");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":selected_root,"message":{"role":"user","content":"Do the work"}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"tool_use","id":"write-inside","name":"Write","input":{"file_path":inside,"content":"inside"}},
                    {"type":"tool_use","id":"write-outside","name":"Write","input":{"file_path":outside,"content":"outside"}}
                ]}}),
                json!({"type":"user","uuid":"r-1","sessionId":"session-1","message":{"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"write-inside","content":"ok"},
                    {"type":"tool_result","tool_use_id":"write-outside","content":"ok"}
                ]}}),
            ],
        );
        let context = SessionTimelineContext {
            resource_id: "conversation:claude:test",
            provider_session_id: "session-1",
            project_root: &canonical_root,
            project_root_aliases: &aliases,
            provider_home: &canonical_root,
            child: None,
        };

        let index = build_timeline_index(&transcript, &context).unwrap();
        assert_eq!(index.parser_version, TIMELINE_PARSER_VERSION);
        let tools = index
            .events
            .iter()
            .filter_map(|event| event.tool.as_ref())
            .map(|tool| (tool.provider_call_id.as_str(), tool.affected_paths.clone()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(tools["write-inside"], vec!["src/lib.rs"]);
        assert!(tools["write-outside"].is_empty());
    }

    #[test]
    fn incomplete_or_malformed_history_is_lower_bound_and_thinking_is_not_visible() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        let long_prompt = "x".repeat(timeline_message_preview_bytes() + 100);
        let mut content = vec![
            json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"message":{"role":"user","content":long_prompt}}),
            json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":[{"type":"thinking","thinking":"do not show"},{"type":"tool_use","id":"bash-1","name":"Bash","input":{"command":"cargo test"}}]}}),
        ]
        .into_iter()
        .map(|value| serde_json::to_string(&value).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
        content.push_str("\n{not-json}\n");
        fs::write(&transcript, content).unwrap();

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert!(index.truncated);
        assert_eq!(
            index.counts.tool_calls.quality,
            SessionCountQuality::LowerBound
        );
        assert_eq!(
            index.counts.prompts.quality,
            SessionCountQuality::LowerBound
        );
        assert_eq!(count_value(&index.counts.intermediate_steps), 0);
        assert!(index
            .warnings
            .iter()
            .any(|warning| warning.code == "malformed_record"));
        assert!(index
            .warnings
            .iter()
            .any(|warning| warning.code == "message_preview_truncated"));
        let prompt = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::Prompt)
            .unwrap();
        assert_eq!(prompt.quality, SessionCountQuality::LowerBound);
    }

    #[test]
    fn stop_sequence_is_an_authoritative_response() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("session.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"message":{"role":"user","content":"Continue"}}),
                json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","isApiErrorMessage":true,"error":"authentication_failed","message":{"id":"msg-1","type":"message","role":"assistant","model":"<synthetic>","stop_reason":"stop_sequence","stop_sequence":"","usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"Authentication failed"}]}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.responses), 1);
        assert_eq!(index.counts.responses.quality, SessionCountQuality::Exact);
        assert_eq!(index.events[1].kind, SessionTimelineEventKind::Response);
        assert_eq!(index.events[1].quality, SessionCountQuality::Exact);
        assert!(!index
            .warnings
            .iter()
            .any(|warning| warning.code == "message_phase_inferred"));
    }

    #[test]
    fn tool_preview_respects_the_shared_byte_limit() {
        let (preview, truncated) = bounded_text(
            Some("鹅".repeat(timeline_tool_preview_bytes())),
            timeline_tool_preview_bytes(),
        );
        let preview = preview.unwrap();

        assert!(truncated);
        assert!(preview.ends_with('…'));
        assert!(preview.len() <= timeline_tool_preview_bytes());
    }

    fn claude_agent_parent(project_root: &Path, waited: bool) -> Vec<Value> {
        let result = if waited {
            json!({"status":"completed","agentId":"a1b2c3","agentType":"Explore","resolvedModel":"claude-opus-5","totalDurationMs":239970,"totalTokens":88416,"totalToolUseCount":33})
        } else {
            json!({"isAsync":true,"status":"async_launched","agentId":"a1b2c3","description":"Research term marking","resolvedModel":"claude-opus-5"})
        };
        vec![
            json!({"type":"user","uuid":"u-1","sessionId":"session-1","cwd":project_root,"timestamp":"2026-08-27T06:00:00Z","message":{"role":"user","content":"Research the term marking"}}),
            json!({"type":"assistant","uuid":"a-1","sessionId":"session-1","timestamp":"2026-08-27T06:00:05Z","message":{"id":"msg-1","role":"assistant","stop_reason":"tool_use","content":[
                {"type":"tool_use","id":"task-1","name":"Task","input":{"description":"Research term marking","subagent_type":"general-purpose","prompt":"Research how reading apps mark a target word."}}
            ]}}),
            json!({"type":"user","uuid":"r-1","sessionId":"session-1","timestamp":"2026-08-27T06:00:09Z","toolUseResult":result,"message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"task-1","content":[{"type":"text","text":"I have a complete picture. Here is the map."}]}
            ]}}),
            json!({"type":"assistant","uuid":"a-2","sessionId":"session-1","timestamp":"2026-08-27T06:14:41Z","message":{"id":"msg-2","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"The three reports agree."}]}}),
        ]
    }

    fn claude_agent_child(project_root: &Path) -> Vec<Value> {
        vec![
            json!({"type":"user","uuid":"c-1","parentUuid":null,"isSidechain":true,"sessionId":"session-1","agentId":"a1b2c3","cwd":project_root,"timestamp":"2026-08-27T06:00:09Z","message":{"role":"user","content":"Research how reading apps mark a target word."}}),
            json!({"type":"assistant","uuid":"c-2","parentUuid":"c-1","isSidechain":true,"sessionId":"session-1","agentId":"a1b2c3","timestamp":"2026-08-27T06:10:36Z","message":{"id":"cm-1","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"I have enough. Here are the findings."}],"usage":{"input_tokens":2,"output_tokens":100}}}),
        ]
    }

    fn agent_page(
        transcript: &Path,
        index: &SessionTimelineIndex,
        context: &SessionTimelineContext<'_>,
    ) -> SessionTimelineEvent {
        let query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::AgentCall]),
            included_tool_categories: BTreeSet::from([SessionToolCategory::Other]),
            turn_id: None,
        };
        let page = read_timeline_page(transcript, index, &query, context).unwrap();
        assert_eq!(page.events.len(), 1);
        page.events.into_iter().next().unwrap()
    }

    #[test]
    fn background_agent_calls_index_as_their_own_kind_and_read_the_child_tail() {
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        let project_root = home.join("project");
        let projects = home.join("projects");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(projects.join("session-1/subagents")).unwrap();
        let transcript = projects.join("session-1.jsonl");
        let child = projects.join("session-1/subagents/agent-a1b2c3.jsonl");
        write_jsonl(&transcript, claude_agent_parent(&project_root, false));
        write_jsonl(&child, claude_agent_child(&project_root));
        let context = SessionTimelineContext {
            resource_id: "claude:conversation:test",
            provider_session_id: "session-1",
            project_root: &project_root,
            project_root_aliases: &[],
            provider_home: &home,
            child: None,
        };

        let index = build_timeline_index(&transcript, &context).unwrap();
        assert_eq!(count_value(&index.counts.agent_calls), 1);
        assert_eq!(count_value(&index.counts.tool_calls), 0);
        assert_eq!(count_value(&index.counts.other_tools), 0);
        let entry = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::AgentCall)
            .unwrap();
        let indexed = entry.agent.as_ref().unwrap();
        assert_eq!(indexed.provider_agent_id.as_deref(), Some("a1b2c3"));
        assert_eq!(indexed.recorded_status, None);
        // The index keeps identities only — never the brief or the label.
        let encoded = serde_json::to_string(&index).unwrap();
        assert!(!encoded.contains("Research"));

        let event = agent_page(&transcript, &index, &context);
        let tool = event.tool.as_ref().unwrap();
        assert_eq!(tool.argument_preview, None);
        assert_eq!(tool.result_preview, None);
        let agent = event.agent.unwrap();
        assert_eq!(agent.label, "Research term marking");
        assert_eq!(agent.role.as_deref(), Some("general-purpose"));
        assert_eq!(agent.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(agent.mode, SessionAgentMode::Async);
        assert_eq!(agent.status, SessionAgentStatus::Completed);
        assert_eq!(
            agent.brief.as_deref(),
            Some("Research how reading apps mark a target word.")
        );
        assert_eq!(
            agent.report.as_deref(),
            Some("I have enough. Here are the findings.")
        );
        assert_eq!(agent.started_at, entry.timestamp);
        assert_eq!(
            agent.ended_at,
            parse_timestamp_value(&json!("2026-08-27T06:10:36Z"))
        );
        assert_eq!(agent.tool_calls, None);
        assert_eq!(
            agent.transcript_resource_id.as_deref(),
            Some("claude:conversation:test/agent/a1b2c3")
        );
        assert_eq!(
            describe_agent_call(&transcript, entry, &context).unwrap(),
            agent
        );

        // The child is its own page: the sidechain is its main chain.
        let identity = SessionChildIdentity {
            parent_provider_session_id: "session-1".to_string(),
            provider_agent_id: "a1b2c3".to_string(),
        };
        let summary = summarize_child_file(&child, &identity).unwrap();
        assert_eq!(summary.session_id, "a1b2c3");
        assert!(summary.is_internal_subagent);
        assert_eq!(summary.user_rounds, 1);
        assert_eq!(summary.agent_messages, 1);
        let child_context = SessionTimelineContext {
            resource_id: "claude:conversation:test/agent/a1b2c3",
            provider_session_id: "a1b2c3",
            child: Some(&identity),
            ..context
        };
        let child_index = build_timeline_index(&child, &child_context).unwrap();
        assert_eq!(count_value(&child_index.counts.prompts), 1);
        assert_eq!(count_value(&child_index.counts.responses), 1);
        assert_eq!(count_value(&child_index.counts.agent_calls), 0);

        // Without its transcript the row still stands, and says so.
        fs::remove_file(&child).unwrap();
        let agent = agent_page(&transcript, &index, &context).agent.unwrap();
        assert_eq!(agent.status, SessionAgentStatus::Unavailable);
        assert_eq!(agent.transcript_resource_id, None);
        assert_eq!(agent.report, None);
    }

    #[test]
    fn waited_agent_calls_take_their_end_and_report_from_the_parent() {
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        let project_root = home.join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = home.join("session-1.jsonl");
        write_jsonl(&transcript, claude_agent_parent(&project_root, true));
        let context = SessionTimelineContext {
            resource_id: "claude:conversation:test",
            provider_session_id: "session-1",
            project_root: &project_root,
            project_root_aliases: &[],
            provider_home: &home,
            child: None,
        };
        let index = build_timeline_index(&transcript, &context).unwrap();
        let entry = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::AgentCall)
            .unwrap();
        assert_eq!(
            entry.agent.as_ref().unwrap().recorded_status,
            Some(SessionAgentStatus::Completed)
        );
        let agent = agent_page(&transcript, &index, &context).agent.unwrap();
        assert_eq!(agent.mode, SessionAgentMode::Sync);
        assert_eq!(agent.status, SessionAgentStatus::Completed);
        assert_eq!(agent.role.as_deref(), Some("general-purpose"));
        assert_eq!(agent.tool_calls, Some(33));
        assert_eq!(agent.tokens, Some(88416));
        assert_eq!(
            agent.report.as_deref(),
            Some("I have a complete picture. Here is the map.")
        );
        assert_eq!(
            agent.ended_at,
            parse_timestamp_value(&json!("2026-08-27T06:00:09Z"))
        );
        // No child file was ever written: the row has nothing to open.
        assert_eq!(agent.transcript_resource_id, None);
        // The per-turn call count is tool calls alone.
        let prompt = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::Prompt)
            .unwrap();
        assert_eq!(prompt.turn_tool_summary, None);
    }
}
