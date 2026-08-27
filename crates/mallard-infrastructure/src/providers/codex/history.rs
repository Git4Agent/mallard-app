//! Bounded Codex rollout parsing for Activity summaries and detail previews.
//!
//! Moved out of the old Codex-only chat history module: the neutral activity
//! engine now reaches this parser only through the Codex adapter.

use crate::app_settings::{
    conversation_file_limit_bytes, timeline_message_preview_bytes, timeline_page_content_bytes,
    timeline_tool_preview_bytes,
};
use crate::providers::timeparse::parse_timestamp_value;
use crate::providers::traits::{
    most_specific_project_relative_path, timeline_tool_detail_budgets, SessionCount,
    SessionCountQuality, SessionDetailWarning, SessionEventCounts, SessionEventDetail,
    SessionFileSummary, SessionSourceSpan, SessionTimelineContext, SessionTimelineEvent,
    SessionTimelineEventKind, SessionTimelineIndex, SessionTimelineIndexEntry,
    SessionTimelinePageSlice, SessionTimelineQuery, SessionToolCallPreview, SessionToolCallStatus,
    SessionToolCategory, SessionToolIndexEntry, SessionTurnToolSummary,
};
use mallard_domain::NoticeSeverity;
use serde::de::{Deserializer, IgnoredAny, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

/// A single Codex JSONL record may embed a generated image or a tool result.
/// Match the transcript bound so every record admitted by the transcript
/// safety policy can also be indexed and opened by session detail.
pub(crate) fn max_line_bytes() -> usize {
    conversation_file_limit_bytes() as usize
}
const MAX_SUMMARY_CHARS: usize = 180;
const MAX_TIMELINE_EVENTS: usize = 50_000;
const MAX_AFFECTED_PATHS_PER_TOOL: usize = 1_024;
pub(crate) const TIMELINE_PARSER_VERSION: u32 = 4;

pub(crate) fn normalize_summary(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut result = value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

pub(crate) fn value_alias<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| value.get(*name))
}

pub(crate) fn string_alias<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    value_alias(value, names).and_then(Value::as_str)
}

pub(crate) fn session_meta_is_internal_subagent(payload: &Value) -> bool {
    payload.get("thread_source").and_then(Value::as_str) == Some("subagent")
        || payload
            .get("source")
            .and_then(|source| source.get("subagent"))
            .is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundedLine {
    Eof,
    Line,
    Oversized,
}

pub(crate) fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<BoundedLine> {
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
        return Ok(BoundedLine::Oversized);
    }
    if output.last() == Some(&b'\r') {
        output.pop();
    }
    Ok(BoundedLine::Line)
}

// Oversized rollouts commonly contain base64 images, compaction snapshots, or
// tool output. This projection keeps only fields used by history/details;
// serde consumes unknown values through IgnoredAny without retaining them.
#[derive(Default, Serialize)]
#[serde(transparent)]
struct ProjectedText(String);

impl<'de> Deserialize<'de> for ProjectedText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ProjectedTextVisitor;

        impl<'de> Visitor<'de> for ProjectedTextVisitor {
            type Value = ProjectedText;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("text or a nested message-content value")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ProjectedText(value.to_string()))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut parts = Vec::new();
                while let Some(part) = seq.next_element::<ProjectedText>()? {
                    if !part.0.is_empty() {
                        parts.push(part.0);
                    }
                }
                Ok(ProjectedText(parts.join(" ")))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut text = None;
                let mut content = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "text" => text = Some(map.next_value::<String>()?),
                        "content" => content = Some(map.next_value::<ProjectedText>()?),
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(ProjectedText(
                    text.or(content.map(|content| content.0))
                        .unwrap_or_default(),
                ))
            }
        }

        deserializer.deserialize_any(ProjectedTextVisitor)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct ProjectedGit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit_hash: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct ProjectedTokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_tokens: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct ProjectedInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_token_usage: Option<ProjectedTokenUsage>,
}

#[derive(Default, Serialize, Deserialize)]
struct ProjectedRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    r#type: Option<String>,
    #[serde(
        default,
        alias = "created_at",
        alias = "started_at",
        skip_serializing_if = "Option::is_none"
    )]
    timestamp: Option<Value>,
    #[serde(
        default,
        alias = "thread_id",
        alias = "session_id",
        skip_serializing_if = "Option::is_none"
    )]
    id: Option<String>,
    #[serde(
        default,
        alias = "working_directory",
        alias = "project_path",
        skip_serializing_if = "Option::is_none"
    )]
    cwd: Option<String>,
    #[serde(default, alias = "git_branch", skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(
        default,
        alias = "git_commit",
        alias = "commit_hash",
        skip_serializing_if = "Option::is_none"
    )]
    commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git: Option<ProjectedGit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<ProjectedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<ProjectedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<ProjectedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    info: Option<ProjectedInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<Box<ProjectedRecord>>,
}

pub(crate) fn recover_oversized_record<R: BufRead + Seek>(
    reader: &mut R,
    record_start: u64,
    record_end: u64,
) -> Result<Value, String> {
    reader
        .seek(SeekFrom::Start(record_start))
        .map_err(|error| error.to_string())?;
    let parsed = {
        let limited = Read::by_ref(reader).take(record_end.saturating_sub(record_start));
        serde_json::from_reader::<_, ProjectedRecord>(limited).map_err(|error| error.to_string())
    };
    reader
        .seek(SeekFrom::Start(record_end))
        .map_err(|error| error.to_string())?;
    serde_json::to_value(parsed?).map_err(|error| error.to_string())
}

pub(crate) fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items.iter().filter_map(extract_text).collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| object.get("content").and_then(extract_text)),
        _ => None,
    }
}

fn event_message_text(payload: &Value) -> Option<String> {
    payload
        .get("message")
        .and_then(extract_text)
        .or_else(|| payload.get("text").and_then(extract_text))
}

fn genuine_user_event(payload: &Value) -> Option<String> {
    (payload.get("type").and_then(Value::as_str) == Some("user_message"))
        .then(|| event_message_text(payload))
        .flatten()
        .map(|text| normalize_summary(&text))
        .filter(|text| is_meaningful_user_text(text))
}

fn is_meaningful_user_text(value: &str) -> bool {
    let trimmed = value.trim_start();
    !trimmed.is_empty()
        && ![
            "<recommended_plugins>",
            "<skill>",
            "<environment_context>",
            "<permissions instructions>",
            "<app-context>",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

struct RolloutParser {
    thread_id: Option<String>,
    cwd: Option<String>,
    first_timestamp: Option<u64>,
    last_timestamp: Option<u64>,
    metadata_timestamp: Option<u64>,
    branch: Option<String>,
    recorded_sha: Option<String>,
    summary: Option<String>,
    user_round_count: u64,
    agent_message_count: u64,
    event_summary: Option<String>,
    event_user_round_count: u64,
    event_agent_message_count: u64,
    tool_call_count: u64,
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    metrics_complete: bool,
    is_internal_subagent: bool,
}

impl RolloutParser {
    fn new() -> Self {
        Self {
            thread_id: None,
            cwd: None,
            first_timestamp: None,
            last_timestamp: None,
            metadata_timestamp: None,
            branch: None,
            recorded_sha: None,
            summary: None,
            user_round_count: 0,
            agent_message_count: 0,
            event_summary: None,
            event_user_round_count: 0,
            event_agent_message_count: 0,
            tool_call_count: 0,
            total_input_tokens: None,
            total_output_tokens: None,
            total_tokens: None,
            metrics_complete: true,
            is_internal_subagent: false,
        }
    }

    fn consume(&mut self, value: &Value) {
        let payload = value.get("payload").unwrap_or(value);
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(timestamp) = value_alias(value, &["timestamp", "created_at", "updated_at"])
            .or_else(|| value_alias(payload, &["timestamp", "created_at", "updated_at"]))
            .and_then(parse_timestamp_value)
        {
            self.first_timestamp.get_or_insert(timestamp);
            self.last_timestamp = Some(timestamp);
        }
        if event_type == "session_meta" || payload_type == "session_meta" {
            if self.thread_id.is_none() {
                // The first session_meta is the active session. Later records
                // are copied fork ancestors and must not change its identity.
                self.is_internal_subagent = session_meta_is_internal_subagent(payload);
                self.thread_id = string_alias(payload, &["id", "thread_id", "session_id"])
                    .map(ToOwned::to_owned);
            }
            if self.cwd.is_none() {
                self.cwd = string_alias(payload, &["cwd", "working_directory", "project_path"])
                    .map(ToOwned::to_owned);
            }
            if self.metadata_timestamp.is_none() {
                self.metadata_timestamp =
                    value_alias(value, &["timestamp", "created_at", "started_at"])
                        .or_else(|| {
                            value_alias(payload, &["timestamp", "created_at", "started_at"])
                        })
                        .and_then(parse_timestamp_value);
            }
            let git = payload.get("git").unwrap_or(payload);
            if self.branch.is_none() {
                self.branch = string_alias(git, &["branch", "git_branch"])
                    .or_else(|| string_alias(payload, &["git_branch", "branch"]))
                    .map(ToOwned::to_owned);
            }
            if self.recorded_sha.is_none() {
                self.recorded_sha =
                    string_alias(git, &["commit_hash", "commit", "sha", "git_commit"])
                        .or_else(|| string_alias(payload, &["git_commit", "commit_hash"]))
                        .map(ToOwned::to_owned);
            }
        }
        if payload_type == "item_completed" {
            if let Some(item) = payload.get("item") {
                match item.get("type").and_then(Value::as_str) {
                    Some("UserMessage") => {
                        if let Some(text) = canonical_item_text(item)
                            .map(|text| normalize_summary(&text))
                            .filter(|text| is_meaningful_user_text(text))
                        {
                            self.user_round_count = self.user_round_count.saturating_add(1);
                            if self.summary.is_none() {
                                self.summary = Some(truncate_chars(&text, MAX_SUMMARY_CHARS));
                            }
                        }
                    }
                    Some("AgentMessage")
                        if normalized_type(item.get("phase").and_then(Value::as_str))
                            == "finalanswer" =>
                    {
                        self.agent_message_count = self.agent_message_count.saturating_add(1);
                    }
                    _ => {}
                }
            }
        }
        if payload_type == "user_message" {
            if let Some(text) = genuine_user_event(payload) {
                self.event_user_round_count = self.event_user_round_count.saturating_add(1);
                if self.event_summary.is_none() {
                    self.event_summary = Some(truncate_chars(&text, MAX_SUMMARY_CHARS));
                }
            }
        } else if event_type == "event_msg"
            && payload_type == "agent_message"
            && normalized_type(payload.get("phase").and_then(Value::as_str)) == "finalanswer"
        {
            // Current response_item/agent_message records are agent-to-agent
            // coordination. Commentary event messages are visible steps, not
            // final replies.
            self.event_agent_message_count = self.event_agent_message_count.saturating_add(1);
        }
        if matches!(payload_type, "function_call" | "custom_tool_call") {
            self.tool_call_count = self.tool_call_count.saturating_add(1);
        }
        if payload_type == "token_count" {
            if let Some(usage) = payload
                .get("info")
                .and_then(|info| info.get("total_token_usage"))
            {
                if let Some(total) = usage.get("total_tokens").and_then(Value::as_u64) {
                    let should_replace = self.total_tokens.is_none_or(|current| total > current);
                    if should_replace {
                        self.total_input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
                        self.total_output_tokens =
                            usage.get("output_tokens").and_then(Value::as_u64);
                        self.total_tokens = Some(total);
                    }
                }
            }
        }
    }

    fn finish(self) -> Result<SessionFileSummary, String> {
        let thread_id = self
            .thread_id
            .ok_or_else(|| "rollout has no session id metadata".to_string())?;
        self.cwd
            .as_ref()
            .ok_or_else(|| "rollout has no cwd metadata".to_string())?;
        // Codex Desktop can emit both views or only the event view. Select
        // one source per role so event-only sessions work without counting
        // dual-emitted messages twice.
        let summary = self.summary.or(self.event_summary);
        let user_round_count = if self.user_round_count > 0 {
            self.user_round_count
        } else {
            self.event_user_round_count
        };
        let agent_message_count = if self.agent_message_count > 0 {
            self.agent_message_count
        } else {
            self.event_agent_message_count
        };
        let mut recorded_branches = std::collections::BTreeSet::new();
        if let Some(branch) = &self.branch {
            recorded_branches.insert(branch.clone());
        }
        Ok(SessionFileSummary {
            session_id: thread_id,
            title: summary.clone(),
            summary,
            started_at: self.first_timestamp.or(self.metadata_timestamp),
            ended_at: self
                .last_timestamp
                .or(self.first_timestamp)
                .or(self.metadata_timestamp),
            branch: self.branch,
            recorded_sha: self.recorded_sha,
            recorded_branches,
            user_rounds: user_round_count,
            agent_messages: agent_message_count,
            tool_calls: self.tool_call_count,
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            total_tokens: self.total_tokens,
            metrics_complete: self.metrics_complete
                && self.first_timestamp.is_some()
                && self.last_timestamp.is_some(),
            is_internal_subagent: self.is_internal_subagent,
        })
    }
}

fn summarize_rollout_reader<R: BufRead + Seek>(
    reader: &mut R,
    label: &str,
) -> Result<SessionFileSummary, String> {
    let mut parser = RolloutParser::new();
    let mut bytes = Vec::new();
    loop {
        let record_start = reader
            .stream_position()
            .map_err(|error| format!("read {label}: {error}"))?;
        match read_bounded_line(reader, &mut bytes, max_line_bytes())
            .map_err(|error| format!("read {label}: {error}"))?
        {
            BoundedLine::Eof => break,
            BoundedLine::Oversized => {
                let record_end = reader
                    .stream_position()
                    .map_err(|error| format!("read {label}: {error}"))?;
                match recover_oversized_record(reader, record_start, record_end) {
                    Ok(value) => parser.consume(&value),
                    Err(_) => parser.metrics_complete = false,
                }
                continue;
            }
            BoundedLine::Line => {}
        }
        if bytes.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => parser.consume(&value),
            Err(_) => parser.metrics_complete = false,
        }
    }
    parser.finish()
}

pub(crate) fn summarize_rollout_file(path: &Path) -> Result<SessionFileSummary, String> {
    let file = File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    summarize_rollout_reader(&mut BufReader::new(file), &format!("'{}'", path.display()))
}

pub(crate) fn summarize_rollout_bytes(bytes: &[u8]) -> Result<SessionFileSummary, String> {
    summarize_rollout_reader(&mut std::io::Cursor::new(bytes), "captured transcript")
}

#[derive(Deserialize)]
struct SessionIndexRow {
    id: String,
    #[serde(default)]
    thread_name: Option<String>,
    #[serde(default)]
    updated_at: Option<Value>,
}

pub(crate) fn parse_session_index_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, (Option<String>, Option<u64>)> {
    let mut result = BTreeMap::<String, (Option<String>, Option<u64>)>::new();
    for line in lines {
        let Ok(row) = serde_json::from_str::<SessionIndexRow>(line) else {
            continue;
        };
        let entry = result.entry(row.id).or_insert((None, None));
        if let Some(title) = row
            .thread_name
            .as_deref()
            .map(normalize_summary)
            .map(|title| truncate_chars(&title, MAX_SUMMARY_CHARS))
            .filter(|title| !title.is_empty())
        {
            entry.0 = Some(title);
        }
        if let Some(updated) = row.updated_at.as_ref().and_then(parse_timestamp_value) {
            entry.1 = Some(updated);
        }
    }
    result
}

pub fn parse_session_index_titles(bytes: &[u8]) -> BTreeMap<String, String> {
    parse_session_index_lines(
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| line.len() <= max_line_bytes())
            .filter_map(|line| std::str::from_utf8(line).ok()),
    )
    .into_iter()
    .filter_map(|(thread_id, (title, _))| title.map(|title| (thread_id, title)))
    .collect()
}

pub fn first_codex_user_summary(bytes: &[u8]) -> Option<String> {
    let mut parser = RolloutParser::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty() && line.len() <= max_line_bytes())
    {
        if let Ok(value) = serde_json::from_slice::<Value>(line) {
            parser.consume(&value);
            if parser.summary.is_some() {
                break;
            }
        }
    }
    parser.summary.or(parser.event_summary)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssistantPhase {
    Known,
    Unknown,
}

#[derive(Clone, Debug)]
struct PendingTimelineEvent {
    entry: SessionTimelineIndexEntry,
    order: (u64, u16),
    text_key: Option<String>,
    assistant_phase: AssistantPhase,
}

#[derive(Default)]
struct TimelineBuilder {
    events: Vec<PendingTimelineEvent>,
    tool_positions: BTreeMap<String, usize>,
    visible_ids: BTreeSet<String>,
    current_turn_id: Option<String>,
    turn_sequence: u64,
    degraded_all: bool,
    degraded_tools: bool,
    inferred_responses: bool,
    inferred_intermediate: bool,
    truncated: bool,
    saw_family: bool,
    warnings: Vec<SessionDetailWarning>,
}

fn merge_affected_paths(target: &mut Vec<String>, incoming: Vec<String>) -> bool {
    target.extend(incoming);
    target.sort();
    target.dedup();
    let truncated = target.len() > MAX_AFFECTED_PATHS_PER_TOOL;
    target.truncate(MAX_AFFECTED_PATHS_PER_TOOL);
    truncated
}

impl TimelineBuilder {
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

    fn next_legacy_turn(&mut self, source: SessionSourceSpan) -> String {
        self.turn_sequence = self.turn_sequence.saturating_add(1);
        let turn_id = format!("codex:turn:{:06}:{}", self.turn_sequence, source.offset);
        self.current_turn_id = Some(turn_id.clone());
        turn_id
    }

    fn turn_or_orphan(&mut self) -> String {
        self.current_turn_id
            .clone()
            .unwrap_or_else(|| "codex:turn:orphan".to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_visible(
        &mut self,
        event_id: String,
        kind: SessionTimelineEventKind,
        quality: SessionCountQuality,
        turn_id: String,
        timestamp: Option<u64>,
        source: SessionSourceSpan,
        text: String,
        assistant_phase: AssistantPhase,
    ) {
        if self.events.len() >= MAX_TIMELINE_EVENTS {
            self.mark_all_lower_bound(
                "timeline_event_limit",
                "The Codex transcript contains more timeline events than Mallard can index.",
            );
            return;
        }
        if text.trim().is_empty() || !self.visible_ids.insert(event_id.clone()) {
            return;
        }
        let quality = if text.len() > timeline_message_preview_bytes() {
            self.warning(
                "message_preview_truncated",
                "At least one Codex message exceeds the 1-MiB timeline display limit and is shown as a truncated preview.",
                NoticeSeverity::Warning,
            );
            SessionCountQuality::LowerBound
        } else {
            quality
        };
        if self.events.last().is_some_and(|previous| {
            previous.entry.kind == kind
                && previous.entry.turn_id.as_deref() == Some(turn_id.as_str())
                && previous.entry.timestamp == timestamp
                && previous.text_key.as_deref() == Some(text.as_str())
        }) {
            return;
        }
        self.events.push(PendingTimelineEvent {
            entry: SessionTimelineIndexEntry {
                event_id,
                ordinal: 0,
                timestamp,
                kind,
                quality,
                turn_id: Some(turn_id),
                turn_tool_summary: None,
                parent_event_id: None,
                source,
                tool: None,
            },
            order: (source.offset, 0),
            text_key: Some(text),
            assistant_phase,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn add_tool(
        &mut self,
        call_id: String,
        native_name: String,
        category: SessionToolCategory,
        status: SessionToolCallStatus,
        turn_id: String,
        timestamp: Option<u64>,
        source: SessionSourceSpan,
        result_source: Option<SessionSourceSpan>,
        affected_paths: Vec<String>,
        parent_event_id: Option<String>,
    ) {
        let tool_key = format!("{turn_id}\0{call_id}");
        if let Some(position) = self.tool_positions.get(&tool_key).copied() {
            let paths_truncated = {
                let pending = &mut self.events[position];
                let Some(tool) = pending.entry.tool.as_mut() else {
                    return;
                };
                if tool.native_name == "unknown" && native_name != "unknown" {
                    tool.native_name = native_name;
                }
                if tool.category == SessionToolCategory::Other
                    && category != SessionToolCategory::Other
                {
                    tool.category = category;
                }
                tool.status = merge_tool_status(tool.status, status);
                if result_source.is_some() {
                    tool.result_source = result_source;
                }
                let paths_truncated =
                    merge_affected_paths(&mut tool.affected_paths, affected_paths);
                if pending.entry.parent_event_id.is_none() {
                    pending.entry.parent_event_id = parent_event_id;
                }
                paths_truncated
            };
            if paths_truncated {
                self.mark_tools_lower_bound(
                    "affected_path_limit",
                    "At least one Codex tool call changed more paths than Mallard can display.",
                );
            }
            return;
        }
        if self.events.len() >= MAX_TIMELINE_EVENTS {
            self.mark_tools_lower_bound(
                "timeline_event_limit",
                "The Codex transcript contains more tool calls than Mallard can index.",
            );
            self.truncated = true;
            return;
        }
        let mut affected_paths_bounded = Vec::new();
        let paths_truncated = merge_affected_paths(&mut affected_paths_bounded, affected_paths);
        if paths_truncated {
            self.mark_tools_lower_bound(
                "affected_path_limit",
                "At least one Codex tool call changed more paths than Mallard can display.",
            );
        }
        let position = self.events.len();
        self.tool_positions.insert(tool_key, position);
        self.events.push(PendingTimelineEvent {
            entry: SessionTimelineIndexEntry {
                event_id: format!("codex:tool:{call_id}"),
                ordinal: 0,
                timestamp,
                kind: SessionTimelineEventKind::ToolCall,
                quality: if status == SessionToolCallStatus::Incomplete {
                    SessionCountQuality::LowerBound
                } else {
                    SessionCountQuality::Exact
                },
                turn_id: Some(turn_id),
                turn_tool_summary: None,
                parent_event_id,
                source,
                tool: Some(SessionToolIndexEntry {
                    provider_call_id: call_id,
                    native_name,
                    category,
                    status,
                    result_source,
                    affected_paths: affected_paths_bounded,
                }),
            },
            order: (source.offset, 1),
            text_key: None,
            assistant_phase: AssistantPhase::Known,
        });
    }

    fn complete_tool(
        &mut self,
        call_id: &str,
        status: SessionToolCallStatus,
        result_source: SessionSourceSpan,
        allow_nested_completion: bool,
        affected_paths: Vec<String>,
    ) {
        let current_turn = self.current_turn_id.as_deref();
        let position = self
            .events
            .iter()
            .enumerate()
            .rev()
            .find_map(|(position, pending)| {
                let tool = pending.entry.tool.as_ref()?;
                (tool.provider_call_id == call_id
                    && current_turn
                        .is_none_or(|turn_id| pending.entry.turn_id.as_deref() == Some(turn_id)))
                .then_some(position)
            });
        let Some(position) = position else {
            // Code mode durably records the parent `exec` call, but may emit
            // completion-only lifecycle events for tools invoked inside it.
            // Those inner events are telemetry for the open parent, not
            // orphaned top-level calls.
            let nested_under_code_mode = allow_nested_completion
                && self.events.iter().rev().any(|pending| {
                    let Some(tool) = pending.entry.tool.as_ref() else {
                        return false;
                    };
                    tool.native_name.eq_ignore_ascii_case("exec")
                        && tool.status == SessionToolCallStatus::Incomplete
                        && current_turn
                            .is_none_or(|turn_id| pending.entry.turn_id.as_deref() == Some(turn_id))
                });
            if nested_under_code_mode {
                let mut paths_truncated = false;
                if !affected_paths.is_empty() {
                    if let Some(parent) = self.events.iter_mut().rev().find(|pending| {
                        let Some(tool) = pending.entry.tool.as_ref() else {
                            return false;
                        };
                        tool.native_name.eq_ignore_ascii_case("exec")
                            && tool.status == SessionToolCallStatus::Incomplete
                            && current_turn.is_none_or(|turn_id| {
                                pending.entry.turn_id.as_deref() == Some(turn_id)
                            })
                    }) {
                        if let Some(tool) = parent.entry.tool.as_mut() {
                            paths_truncated =
                                merge_affected_paths(&mut tool.affected_paths, affected_paths);
                        }
                    }
                }
                if paths_truncated {
                    self.mark_tools_lower_bound(
                        "affected_path_limit",
                        "At least one Codex tool call changed more paths than Mallard can display.",
                    );
                }
                return;
            }
            self.mark_tools_lower_bound(
                "orphan_tool_result",
                "A Codex tool result had no durable matching call start.",
            );
            return;
        };
        let paths_truncated = {
            let pending = &mut self.events[position];
            let Some(tool) = pending.entry.tool.as_mut() else {
                return;
            };
            tool.status = merge_tool_status(tool.status, status);
            tool.result_source = Some(result_source);
            let paths_truncated = merge_affected_paths(&mut tool.affected_paths, affected_paths);
            pending.entry.quality = if tool.status == SessionToolCallStatus::Incomplete {
                SessionCountQuality::LowerBound
            } else {
                SessionCountQuality::Exact
            };
            paths_truncated
        };
        if paths_truncated {
            self.mark_tools_lower_bound(
                "affected_path_limit",
                "At least one Codex tool call changed more paths than Mallard can display.",
            );
        }
    }

    fn latest_assistant_event_id(&self, turn_id: &str) -> Option<String> {
        self.events
            .iter()
            .rev()
            .find(|pending| {
                pending.entry.turn_id.as_deref() == Some(turn_id)
                    && matches!(
                        pending.entry.kind,
                        SessionTimelineEventKind::Response
                            | SessionTimelineEventKind::IntermediateStep
                    )
            })
            .map(|pending| pending.entry.event_id.clone())
    }

    fn finalize(mut self) -> SessionTimelineIndex {
        for position in 0..self.events.len() {
            if self.events[position].assistant_phase != AssistantPhase::Unknown {
                continue;
            }
            let turn_id = self.events[position].entry.turn_id.clone();
            let order = self.events[position].order;
            let followed_by_tool = self.events.iter().any(|candidate| {
                candidate.entry.kind == SessionTimelineEventKind::ToolCall
                    && candidate.entry.turn_id == turn_id
                    && candidate.order > order
            });
            let entry = &mut self.events[position].entry;
            entry.quality = SessionCountQuality::Inferred;
            if followed_by_tool {
                entry.kind = SessionTimelineEventKind::IntermediateStep;
                self.inferred_intermediate = true;
            } else {
                entry.kind = SessionTimelineEventKind::Response;
                self.inferred_responses = true;
            }
        }

        if self.inferred_intermediate || self.inferred_responses {
            self.warning(
                "legacy_message_phase_inferred",
                "This Codex rollout predates authoritative message phases; final and progress placement was inferred from nearby tool calls.",
                NoticeSeverity::Info,
            );
        }

        let mut has_incomplete = false;
        for pending in &mut self.events {
            if let Some(tool) = pending.entry.tool.as_mut() {
                if tool.status == SessionToolCallStatus::Incomplete {
                    has_incomplete = true;
                    pending.entry.quality = SessionCountQuality::LowerBound;
                }
            }
        }
        if has_incomplete {
            self.mark_tools_lower_bound(
                "incomplete_tool_call",
                "At least one Codex tool call has no durable completion record.",
            );
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
        let mut tools_by_turn = BTreeMap::<String, u64>::new();
        for pending in &self.events {
            if pending.entry.kind != SessionTimelineEventKind::ToolCall {
                continue;
            }
            if let Some(turn_id) = &pending.entry.turn_id {
                *tools_by_turn.entry(turn_id.clone()).or_default() += 1;
            }
        }
        for (turn_id, value) in tools_by_turn {
            let leading_position = self
                .events
                .iter()
                .position(|pending| {
                    pending.entry.turn_id.as_deref() == Some(turn_id.as_str())
                        && pending.entry.kind != SessionTimelineEventKind::ToolCall
                })
                .or_else(|| {
                    self.events.iter().position(|pending| {
                        pending.entry.turn_id.as_deref() == Some(turn_id.as_str())
                    })
                });
            if let Some(position) = leading_position {
                let leading = &mut self.events[position];
                leading.entry.turn_tool_summary = Some(SessionTurnToolSummary {
                    turn_id: turn_id.clone(),
                    tool_calls: count_with_quality(
                        value,
                        tool_quality,
                        tool_count_explanation(tool_quality),
                    ),
                });
            }
        }

        let entries = self
            .events
            .into_iter()
            .map(|pending| pending.entry)
            .collect::<Vec<_>>();
        let counts = counts_for_entries(
            &entries,
            self.degraded_all,
            self.degraded_tools,
            self.inferred_responses,
            self.inferred_intermediate,
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

fn merge_tool_status(
    current: SessionToolCallStatus,
    observed: SessionToolCallStatus,
) -> SessionToolCallStatus {
    match (current, observed) {
        (SessionToolCallStatus::Failed, _) | (_, SessionToolCallStatus::Failed) => {
            SessionToolCallStatus::Failed
        }
        (SessionToolCallStatus::Completed, _) | (_, SessionToolCallStatus::Completed) => {
            SessionToolCallStatus::Completed
        }
        _ => SessionToolCallStatus::Incomplete,
    }
}

fn count_with_quality(
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

fn tool_count_explanation(quality: SessionCountQuality) -> Option<&'static str> {
    (quality == SessionCountQuality::LowerBound).then_some(
        "Some Codex tool activity may be absent because the durable rollout was incomplete.",
    )
}

fn counts_for_entries(
    entries: &[SessionTimelineIndexEntry],
    degraded_all: bool,
    degraded_tools: bool,
    inferred_responses: bool,
    inferred_intermediate: bool,
) -> SessionEventCounts {
    let count_kind = |kind| entries.iter().filter(|entry| entry.kind == kind).count() as u64;
    let count_category = |category| {
        entries
            .iter()
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
    let intermediate_quality = if degraded_all {
        SessionCountQuality::LowerBound
    } else if inferred_intermediate {
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
        .then_some("Some Codex records could not be safely indexed.");
    let phase_explanation = Some(
        "The rollout did not record authoritative assistant phases; placement was inferred from surrounding tool activity.",
    );
    SessionEventCounts {
        prompts: count_with_quality(
            count_kind(SessionTimelineEventKind::Prompt),
            visible_quality,
            visible_explanation,
        ),
        responses: count_with_quality(
            count_kind(SessionTimelineEventKind::Response),
            response_quality,
            if response_quality == SessionCountQuality::Inferred {
                phase_explanation
            } else {
                visible_explanation
            },
        ),
        intermediate_steps: count_with_quality(
            count_kind(SessionTimelineEventKind::IntermediateStep),
            intermediate_quality,
            if intermediate_quality == SessionCountQuality::Inferred {
                phase_explanation
            } else {
                visible_explanation
            },
        ),
        checkpoints: SessionCount::unsupported(
            "Git checkpoints are correlated by Mallard outside the Codex transcript.",
        ),
        tool_calls: count_with_quality(
            count_kind(SessionTimelineEventKind::ToolCall),
            tool_quality,
            tool_count_explanation(tool_quality),
        ),
        file_edits: count_with_quality(
            count_category(SessionToolCategory::FileEdit),
            tool_quality,
            tool_count_explanation(tool_quality),
        ),
        shell_calls: count_with_quality(
            count_category(SessionToolCategory::Shell),
            tool_quality,
            tool_count_explanation(tool_quality),
        ),
        reads: count_with_quality(
            count_category(SessionToolCategory::Read),
            tool_quality,
            tool_count_explanation(tool_quality),
        ),
        other_tools: count_with_quality(
            count_category(SessionToolCategory::Other),
            tool_quality,
            tool_count_explanation(tool_quality),
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

fn record_timestamp(value: &Value) -> Option<u64> {
    value_alias(value, &["timestamp", "completed_at_ms", "started_at_ms"])
        .or_else(|| {
            value.get("payload").and_then(|payload| {
                value_alias(payload, &["timestamp", "completed_at_ms", "started_at_ms"])
            })
        })
        .and_then(parse_timestamp_value)
}

fn source_span(start: u64, end: u64) -> SessionSourceSpan {
    SessionSourceSpan {
        offset: start,
        length: end.saturating_sub(start),
    }
}

fn canonical_turn_id(payload: &Value, item: &Value, source: SessionSourceSpan) -> String {
    string_alias(payload, &["turn_id", "turnId"])
        .or_else(|| {
            item.get("internal_chat_message_metadata_passthrough")
                .and_then(|metadata| string_alias(metadata, &["turn_id", "turnId"]))
        })
        .filter(|turn_id| !turn_id.is_empty())
        .map(|turn_id| format!("codex:turn:{turn_id}"))
        .unwrap_or_else(|| format!("codex:turn:item:{}", source.offset))
}

fn canonical_item_text(item: &Value) -> Option<String> {
    item.get("text")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| item.get("content").and_then(extract_text))
}

fn canonical_item_status(item: &Value) -> SessionToolCallStatus {
    if let Some(success) = item.get("success").and_then(Value::as_bool) {
        return if success {
            SessionToolCallStatus::Completed
        } else {
            SessionToolCallStatus::Failed
        };
    }
    match normalized_type(item.get("status").and_then(Value::as_str)).as_str() {
        "completed" | "success" | "succeeded" => SessionToolCallStatus::Completed,
        "failed" | "error" | "declined" | "cancelled" | "canceled" => SessionToolCallStatus::Failed,
        _ => SessionToolCallStatus::Incomplete,
    }
}

fn classify_codex_tool(name: &str) -> SessionToolCategory {
    match name.to_ascii_lowercase().as_str() {
        "apply_patch" | "file_change" | "filechange" => SessionToolCategory::FileEdit,
        "exec_command" | "shell_command" | "local_shell_call" | "command_execution" | "bash"
        | "shell" => SessionToolCategory::Shell,
        "read" | "read_file" | "readfile" | "read_mcp_resource" => SessionToolCategory::Read,
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

fn affected_paths_from_value(
    value: &Value,
    project_root: &Path,
    project_root_aliases: &[PathBuf],
    include_direct_path: bool,
) -> (Vec<String>, bool) {
    let mut result = Vec::new();
    let mut truncated = false;
    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for path in changes.keys() {
            if let Some(relative) =
                safe_project_relative_path(path, project_root, project_root_aliases)
            {
                if result.len() >= MAX_AFFECTED_PATHS_PER_TOOL {
                    truncated = true;
                    break;
                }
                result.push(relative);
            }
        }
    }
    if include_direct_path {
        for key in [
            "file_path",
            "filePath",
            "notebook_path",
            "notebookPath",
            "path",
        ] {
            if let Some(path) = value.get(key).and_then(Value::as_str) {
                if let Some(relative) =
                    safe_project_relative_path(path, project_root, project_root_aliases)
                {
                    if !result.contains(&relative) {
                        if result.len() >= MAX_AFFECTED_PATHS_PER_TOOL {
                            truncated = true;
                        } else {
                            result.push(relative);
                        }
                    }
                }
            }
        }
    }
    result.sort();
    result.dedup();
    (result, truncated)
}

fn canonical_tool(
    item: &Value,
    context: &SessionTimelineContext<'_>,
) -> Option<(
    String,
    String,
    SessionToolCategory,
    SessionToolCallStatus,
    Vec<String>,
    bool,
)> {
    let item_type = normalized_type(item.get("type").and_then(Value::as_str));
    let id = string_alias(item, &["id", "call_id", "callId"])?.to_string();
    let (native_name, classifier_name) = match item_type.as_str() {
        "commandexecution" => ("exec_command".to_string(), "exec_command".to_string()),
        "filechange" => ("apply_patch".to_string(), "apply_patch".to_string()),
        "dynamictoolcall" => {
            let name = string_alias(item, &["tool", "name"]).unwrap_or("dynamic_tool");
            (name.to_string(), name.to_string())
        }
        "collabagenttoolcall" => {
            let name = string_alias(item, &["tool", "name"]).unwrap_or("collab_tool");
            (name.to_string(), name.to_string())
        }
        "websearch" => ("web_search".to_string(), "web_search".to_string()),
        "imageview" => ("view_image".to_string(), "view_image".to_string()),
        "imagegeneration" => (
            "image_generation".to_string(),
            "image_generation".to_string(),
        ),
        "mcptoolcall" => {
            let tool = string_alias(item, &["tool", "name"]).unwrap_or("mcp_tool");
            let display = item
                .get("server")
                .and_then(Value::as_str)
                .map(|server| format!("{server}/{tool}"))
                .unwrap_or_else(|| tool.to_string());
            (display, tool.to_string())
        }
        "extension" => {
            let name = string_alias(item, &["kind", "name", "tool"]).unwrap_or("extension");
            (name.to_string(), name.to_string())
        }
        _ => return None,
    };
    let category = classify_codex_tool(&classifier_name);
    let (paths, paths_truncated) = affected_paths_from_value(
        item,
        context.project_root,
        context.project_root_aliases,
        category == SessionToolCategory::FileEdit,
    );
    let mut status = canonical_item_status(item);
    let completed_web_extension =
        item_type == "extension" && classifier_name.eq_ignore_ascii_case("web.search");
    if status == SessionToolCallStatus::Incomplete
        && (matches!(item_type.as_str(), "websearch" | "imageview" | "filechange")
            || completed_web_extension)
    {
        status = SessionToolCallStatus::Completed;
    }
    Some((id, native_name, category, status, paths, paths_truncated))
}

fn consume_canonical_item(
    builder: &mut TimelineBuilder,
    payload: &Value,
    source: SessionSourceSpan,
    timestamp: Option<u64>,
    completed: bool,
    context: &SessionTimelineContext<'_>,
) {
    let Some(item) = payload.get("item") else {
        builder.mark_all_lower_bound(
            "malformed_canonical_item",
            "A canonical Codex timeline item had no item payload.",
        );
        return;
    };
    builder.saw_family = true;
    let item_type = normalized_type(item.get("type").and_then(Value::as_str));
    let turn_id = canonical_turn_id(payload, item, source);
    if !completed {
        if let Some((call_id, name, category, _, paths, paths_truncated)) =
            canonical_tool(item, context)
        {
            if paths_truncated {
                builder.mark_tools_lower_bound(
                    "affected_path_limit",
                    "At least one Codex tool call changed more paths than Mallard can display.",
                );
            }
            let parent = builder.latest_assistant_event_id(&turn_id);
            builder.add_tool(
                call_id,
                name,
                category,
                SessionToolCallStatus::Incomplete,
                turn_id,
                timestamp,
                source,
                None,
                paths,
                parent,
            );
        }
        return;
    }

    match item_type.as_str() {
        "usermessage" => {
            let Some(text) = canonical_item_text(item)
                .filter(|text| is_meaningful_user_text(&normalize_summary(text)))
            else {
                return;
            };
            let native_id = string_alias(item, &["id"]).unwrap_or_default();
            builder.add_visible(
                if native_id.is_empty() {
                    format!("codex:event:{}", source.offset)
                } else {
                    format!("codex:item:{native_id}")
                },
                SessionTimelineEventKind::Prompt,
                SessionCountQuality::Exact,
                turn_id,
                timestamp,
                source,
                text,
                AssistantPhase::Known,
            );
        }
        "agentmessage" => {
            let Some(text) = canonical_item_text(item) else {
                return;
            };
            let phase = normalized_type(item.get("phase").and_then(Value::as_str));
            let (kind, quality, assistant_phase) = match phase.as_str() {
                "commentary" => (
                    SessionTimelineEventKind::IntermediateStep,
                    SessionCountQuality::Exact,
                    AssistantPhase::Known,
                ),
                "finalanswer" => (
                    SessionTimelineEventKind::Response,
                    SessionCountQuality::Exact,
                    AssistantPhase::Known,
                ),
                _ => (
                    SessionTimelineEventKind::Response,
                    SessionCountQuality::Inferred,
                    AssistantPhase::Unknown,
                ),
            };
            let native_id = string_alias(item, &["id"]).unwrap_or_default();
            builder.add_visible(
                if native_id.is_empty() {
                    format!("codex:event:{}", source.offset)
                } else {
                    format!("codex:item:{native_id}")
                },
                kind,
                quality,
                turn_id,
                timestamp,
                source,
                text,
                assistant_phase,
            );
        }
        "plan" => {
            let Some(text) = canonical_item_text(item) else {
                return;
            };
            let native_id = string_alias(item, &["id"]).unwrap_or_default();
            builder.add_visible(
                if native_id.is_empty() {
                    format!("codex:event:{}", source.offset)
                } else {
                    format!("codex:item:{native_id}")
                },
                SessionTimelineEventKind::IntermediateStep,
                SessionCountQuality::Exact,
                turn_id,
                timestamp,
                source,
                text,
                AssistantPhase::Known,
            );
        }
        "reasoning" | "contextcompaction" | "hookprompt" | "enteredreviewmode"
        | "exitedreviewmode" | "subagentactivity" => {}
        _ => {
            if let Some((call_id, name, category, status, paths, paths_truncated)) =
                canonical_tool(item, context)
            {
                if paths_truncated {
                    builder.mark_tools_lower_bound(
                        "affected_path_limit",
                        "At least one Codex tool call changed more paths than Mallard can display.",
                    );
                }
                let parent = builder.latest_assistant_event_id(&turn_id);
                builder.add_tool(
                    call_id,
                    name,
                    category,
                    status,
                    turn_id,
                    timestamp,
                    source,
                    Some(source),
                    paths,
                    parent,
                );
            } else {
                builder.mark_all_lower_bound(
                    "unknown_canonical_item",
                    "The Codex rollout contains a canonical item type this Mallard version does not recognize.",
                );
            }
        }
    }
}

fn legacy_call_id(payload: &Value, source: SessionSourceSpan) -> String {
    string_alias(payload, &["call_id", "callId", "id"])
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("offset-{}", source.offset))
}

fn legacy_turn(builder: &mut TimelineBuilder) -> String {
    builder.turn_or_orphan()
}

fn legacy_tool_status(payload: &Value) -> SessionToolCallStatus {
    if let Some(success) = payload.get("success").and_then(Value::as_bool) {
        return if success {
            SessionToolCallStatus::Completed
        } else {
            SessionToolCallStatus::Failed
        };
    }
    if let Some(exit_code) = payload.get("exit_code").and_then(Value::as_i64) {
        return if exit_code == 0 {
            SessionToolCallStatus::Completed
        } else {
            SessionToolCallStatus::Failed
        };
    }
    canonical_item_status(payload)
}

fn consume_legacy_response_item(
    builder: &mut TimelineBuilder,
    payload: &Value,
    source: SessionSourceSpan,
    timestamp: Option<u64>,
    context: &SessionTimelineContext<'_>,
) {
    let payload_type = normalized_type(payload.get("type").and_then(Value::as_str));
    let turn_id = legacy_turn(builder);
    match payload_type.as_str() {
        "functioncall" | "customtoolcall" => {
            let name = string_alias(payload, &["name"]).unwrap_or("unknown");
            let category = classify_codex_tool(name);
            let (paths, paths_truncated) = affected_paths_from_value(
                payload
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                    .as_ref()
                    .unwrap_or(payload),
                context.project_root,
                context.project_root_aliases,
                category == SessionToolCategory::FileEdit,
            );
            if paths_truncated {
                builder.mark_tools_lower_bound(
                    "affected_path_limit",
                    "At least one Codex tool call changed more paths than Mallard can display.",
                );
            }
            let parent = builder.latest_assistant_event_id(&turn_id);
            builder.add_tool(
                legacy_call_id(payload, source),
                name.to_string(),
                category,
                SessionToolCallStatus::Incomplete,
                turn_id,
                timestamp,
                source,
                None,
                paths,
                parent,
            );
        }
        "localshellcall" => {
            let status = legacy_tool_status(payload);
            let call_id = legacy_call_id(payload, source);
            let parent = builder.latest_assistant_event_id(&turn_id);
            builder.add_tool(
                call_id,
                "local_shell_call".to_string(),
                SessionToolCategory::Shell,
                status,
                turn_id,
                timestamp,
                source,
                (status != SessionToolCallStatus::Incomplete).then_some(source),
                Vec::new(),
                parent,
            );
        }
        "toolsearchcall" | "websearchcall" | "imagegenerationcall" => {
            let name = match payload_type.as_str() {
                "toolsearchcall" => "tool_search",
                "websearchcall" => "web_search",
                _ => "image_generation",
            };
            let status = legacy_tool_status(payload);
            let call_id = legacy_call_id(payload, source);
            let parent = builder.latest_assistant_event_id(&turn_id);
            builder.add_tool(
                call_id,
                name.to_string(),
                SessionToolCategory::Other,
                status,
                turn_id,
                timestamp,
                source,
                (status != SessionToolCallStatus::Incomplete).then_some(source),
                Vec::new(),
                parent,
            );
        }
        "functioncalloutput" | "customtoolcalloutput" | "toolsearchoutput" => {
            let call_id = legacy_call_id(payload, source);
            let status = legacy_tool_status(payload);
            builder.complete_tool(
                &call_id,
                if status == SessionToolCallStatus::Incomplete {
                    SessionToolCallStatus::Completed
                } else {
                    status
                },
                source,
                false,
                Vec::new(),
            );
        }
        "message" | "agentmessage" | "reasoning" | "compaction" | "contextcompaction"
        | "additionaltools" => {}
        "" => builder.mark_all_lower_bound(
            "malformed_response_item",
            "A Codex response item had no discriminator.",
        ),
        _ => builder.mark_all_lower_bound(
            "unknown_response_item",
            "The Codex rollout contains a response item type this Mallard version does not recognize.",
        ),
    }
}

fn consume_legacy_event(
    builder: &mut TimelineBuilder,
    payload: &Value,
    source: SessionSourceSpan,
    timestamp: Option<u64>,
    context: &SessionTimelineContext<'_>,
) {
    let payload_type = normalized_type(payload.get("type").and_then(Value::as_str));
    match payload_type.as_str() {
        "usermessage" => {
            let Some(text) = genuine_user_event(payload) else {
                return;
            };
            let turn_id = builder.next_legacy_turn(source);
            let native = string_alias(payload, &["client_id", "clientId"]);
            builder.add_visible(
                native
                    .map(|id| format!("codex:user:{id}"))
                    .unwrap_or_else(|| format!("codex:event:{}", source.offset)),
                SessionTimelineEventKind::Prompt,
                SessionCountQuality::Exact,
                turn_id,
                timestamp,
                source,
                event_message_text(payload).unwrap_or(text),
                AssistantPhase::Known,
            );
        }
        "agentmessage" => {
            let Some(text) = event_message_text(payload) else {
                return;
            };
            let turn_id = legacy_turn(builder);
            let phase = normalized_type(payload.get("phase").and_then(Value::as_str));
            let (kind, quality, assistant_phase) = match phase.as_str() {
                "commentary" => (
                    SessionTimelineEventKind::IntermediateStep,
                    SessionCountQuality::Exact,
                    AssistantPhase::Known,
                ),
                "finalanswer" => (
                    SessionTimelineEventKind::Response,
                    SessionCountQuality::Exact,
                    AssistantPhase::Known,
                ),
                _ => (
                    SessionTimelineEventKind::Response,
                    SessionCountQuality::Inferred,
                    AssistantPhase::Unknown,
                ),
            };
            builder.add_visible(
                format!("codex:event:{}", source.offset),
                kind,
                quality,
                turn_id,
                timestamp,
                source,
                text,
                assistant_phase,
            );
        }
        "planupdate" => {
            let text = payload
                .get("explanation")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| plan_text(payload));
            if let Some(text) = text {
                let turn_id = legacy_turn(builder);
                builder.add_visible(
                    format!("codex:event:{}", source.offset),
                    SessionTimelineEventKind::IntermediateStep,
                    SessionCountQuality::Exact,
                    turn_id,
                    timestamp,
                    source,
                    text,
                    AssistantPhase::Known,
                );
            }
        }
        "execcommandbegin"
        | "patchapplybegin"
        | "mcptoolcallbegin"
        | "websearchbegin"
        | "imagegenerationbegin"
        | "dynamictoolcallrequest" => {
            let call_id = legacy_call_id(payload, source);
            let (name, classifier) = match payload_type.as_str() {
                "execcommandbegin" => ("exec_command".to_string(), "exec_command".to_string()),
                "patchapplybegin" => ("apply_patch".to_string(), "apply_patch".to_string()),
                "mcptoolcallbegin" => {
                    let invocation = payload.get("invocation").unwrap_or(payload);
                    let tool = string_alias(invocation, &["tool", "name"]).unwrap_or("mcp_tool");
                    let display = invocation
                        .get("server")
                        .and_then(Value::as_str)
                        .map(|server| format!("{server}/{tool}"))
                        .unwrap_or_else(|| tool.to_string());
                    (display, tool.to_string())
                }
                "websearchbegin" => ("web_search".to_string(), "web_search".to_string()),
                "imagegenerationbegin" => (
                    "image_generation".to_string(),
                    "image_generation".to_string(),
                ),
                _ => {
                    let name = string_alias(payload, &["tool", "name"]).unwrap_or("dynamic_tool");
                    (name.to_string(), name.to_string())
                }
            };
            let category = classify_codex_tool(&classifier);
            let (paths, paths_truncated) = affected_paths_from_value(
                payload,
                context.project_root,
                context.project_root_aliases,
                category == SessionToolCategory::FileEdit,
            );
            if paths_truncated {
                builder.mark_tools_lower_bound(
                    "affected_path_limit",
                    "At least one Codex tool call changed more paths than Mallard can display.",
                );
            }
            let turn_id = legacy_turn(builder);
            let parent = builder.latest_assistant_event_id(&turn_id);
            builder.add_tool(
                call_id,
                name,
                category,
                SessionToolCallStatus::Incomplete,
                turn_id,
                timestamp,
                source,
                None,
                paths,
                parent,
            );
        }
        "execcommandend"
        | "patchapplyend"
        | "mcptoolcallend"
        | "websearchend"
        | "imagegenerationend"
        | "dynamictoolcallresponse" => {
            let call_id = legacy_call_id(payload, source);
            let mut status = legacy_tool_status(payload);
            if status == SessionToolCallStatus::Incomplete {
                status = SessionToolCallStatus::Completed;
            }
            let (affected_paths, paths_truncated) = if payload_type == "patchapplyend" {
                affected_paths_from_value(
                    payload,
                    context.project_root,
                    context.project_root_aliases,
                    true,
                )
            } else {
                (Vec::new(), false)
            };
            if paths_truncated {
                builder.mark_tools_lower_bound(
                    "affected_path_limit",
                    "At least one Codex tool call changed more paths than Mallard can display.",
                );
            }
            builder.complete_tool(&call_id, status, source, true, affected_paths);
        }
        "viewimagetoolcall" | "requestuserinput" => {
            let call_id = legacy_call_id(payload, source);
            let name = if payload_type == "viewimagetoolcall" {
                "view_image"
            } else {
                "request_user_input"
            };
            let turn_id = legacy_turn(builder);
            let parent = builder.latest_assistant_event_id(&turn_id);
            builder.add_tool(
                call_id,
                name.to_string(),
                SessionToolCategory::Other,
                SessionToolCallStatus::Completed,
                turn_id,
                timestamp,
                source,
                Some(source),
                Vec::new(),
                parent,
            );
        }
        "agentreasoning"
        | "agentreasoningrawcontent"
        | "agentreasoningsectionbreak"
        | "contextcompacted"
        | "compacted"
        | "taskstarted"
        | "turnstarted"
        | "taskcomplete"
        | "turncomplete"
        | "tokencount"
        | "sessionconfigured"
        | "rawresponseitem"
        | "rawresponsecompleted"
        | "itemstarted"
        | "itemcompleted"
        | "agentmessagecontentdelta"
        | "plandelta"
        | "reasoningcontentdelta"
        | "reasoningrawcontentdelta"
        | "turnaborted"
        | "turndiff"
        | "warning"
        | "error"
        | "streamerror"
        | "shutdowncomplete"
        | "enteredreviewmode"
        | "exitedreviewmode"
        | "threadsettingsapplied"
        | "threadrolleedback"
        | "threadgoalupdated"
        | "threadqueuechanged"
        | "mcpstartupupdate"
        | "mcpstartupcomplete"
        | "execcommandoutputdelta"
        | "terminalinteraction"
        | "patchapplyupdated"
        | "execapprovalrequest"
        | "applypatchapprovalrequest"
        | "permissionrequest"
        | "requestpermissions"
        | "hookstarted"
        | "hookcompleted"
        | "deprecationnotice"
        | "modelreroute"
        | "modelverification"
        | "turnmoderationmetadata"
        | "safetybuffering" => {}
        "" => {
            builder.mark_all_lower_bound("malformed_event", "A Codex event had no discriminator.")
        }
        other => {
            if other.contains("tool")
                || other.contains("command")
                || other.contains("patch")
                || other.contains("search")
            {
                builder.mark_tools_lower_bound(
                    "unknown_tool_event",
                    "The Codex rollout contains a tool lifecycle event this Mallard version does not recognize.",
                );
            }
        }
    }
}

fn plan_text(payload: &Value) -> Option<String> {
    let plan = payload.get("plan")?.as_array()?;
    let parts = plan
        .iter()
        .filter_map(|step| {
            step.get("step")
                .or_else(|| step.get("text"))
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

pub(crate) fn build_timeline_index(
    path: &Path,
    context: &SessionTimelineContext<'_>,
) -> Result<SessionTimelineIndex, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("inspect '{}': {error}", path.display()))?;
    if metadata.len() > conversation_file_limit_bytes() {
        return Err(format!(
            "Codex transcript '{}' exceeds {} bytes",
            path.display(),
            conversation_file_limit_bytes()
        ));
    }
    let file = File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut canonical = TimelineBuilder::default();
    let mut legacy = TimelineBuilder::default();
    let mut bytes = Vec::new();
    let mut record_count = 0usize;
    loop {
        let record_start = reader
            .stream_position()
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        let line = read_bounded_line(&mut reader, &mut bytes, max_line_bytes())
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        if line == BoundedLine::Eof {
            break;
        }
        let record_end = reader
            .stream_position()
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        record_count = record_count.saturating_add(1);
        if record_count > crate::codex_rollout::MAX_CODEX_RECORDS {
            canonical.mark_all_lower_bound(
                "record_limit",
                "The Codex transcript contains more records than Mallard can index.",
            );
            legacy.mark_all_lower_bound(
                "record_limit",
                "The Codex transcript contains more records than Mallard can index.",
            );
            break;
        }
        if line == BoundedLine::Oversized {
            canonical.mark_all_lower_bound(
                "oversized_record",
                "At least one Codex record exceeded the 100-MiB timeline parsing limit.",
            );
            legacy.mark_all_lower_bound(
                "oversized_record",
                "At least one Codex record exceeded the 100-MiB timeline parsing limit.",
            );
            continue;
        }
        if bytes.is_empty() {
            continue;
        }
        let value = match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(_) => {
                canonical.mark_all_lower_bound(
                    "malformed_record",
                    "At least one Codex record was malformed and could not be indexed.",
                );
                legacy.mark_all_lower_bound(
                    "malformed_record",
                    "At least one Codex record was malformed and could not be indexed.",
                );
                continue;
            }
        };
        let source = source_span(record_start, record_end);
        let timestamp = record_timestamp(&value);
        let outer_type = normalized_type(value.get("type").and_then(Value::as_str));
        let payload = value.get("payload").unwrap_or(&value);
        match outer_type.as_str() {
            "eventmsg" => {
                let payload_type = normalized_type(payload.get("type").and_then(Value::as_str));
                match payload_type.as_str() {
                    "itemstarted" => consume_canonical_item(
                        &mut canonical,
                        payload,
                        source,
                        timestamp,
                        false,
                        context,
                    ),
                    "itemcompleted" => consume_canonical_item(
                        &mut canonical,
                        payload,
                        source,
                        timestamp,
                        true,
                        context,
                    ),
                    _ => consume_legacy_event(&mut legacy, payload, source, timestamp, context),
                }
            }
            "responseitem" => {
                consume_legacy_response_item(&mut legacy, payload, source, timestamp, context)
            }
            _ => {}
        }
    }
    if canonical.saw_family {
        Ok(canonical.finalize())
    } else {
        Ok(legacy.finalize())
    }
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

fn timeline_text(value: &Value, kind: SessionTimelineEventKind) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    let payload_type = normalized_type(payload.get("type").and_then(Value::as_str));
    if matches!(payload_type.as_str(), "itemcompleted" | "itemstarted") {
        let item = payload.get("item")?;
        return match kind {
            SessionTimelineEventKind::Prompt
            | SessionTimelineEventKind::Response
            | SessionTimelineEventKind::IntermediateStep => canonical_item_text(item),
            _ => None,
        };
    }
    match kind {
        SessionTimelineEventKind::Prompt | SessionTimelineEventKind::Response => {
            event_message_text(payload)
        }
        SessionTimelineEventKind::IntermediateStep => event_message_text(payload)
            .or_else(|| {
                payload
                    .get("explanation")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .or_else(|| plan_text(payload)),
        _ => None,
    }
}

fn value_as_preview(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => serde_json::to_string_pretty(other).ok(),
    }
}

/// Keys that name a call, richest first. Mirrors the Claude reader's list so
/// the two providers label an object-shaped argument the same way.
const TOOL_LABEL_KEYS: &[&str] = &[
    "command",
    "cmd",
    "path",
    "file_path",
    "query",
    "url",
    "pattern",
    "prompt",
    "title",
    "description",
    "message",
    // After the prose: a call that carries both a title and a body is named by
    // the title, and only a call with nothing else is named by its code.
    "script",
    "code",
    "text",
];

/// `["/bin/zsh", "-lc", "<script>"]` is named by its script, not by the shell
/// that ran it: the two elements before it are identical on every row. Tested
/// by shape rather than by position, because a bare argv like `["rg", "parser"]`
/// is equally legal and there the command is the whole vector.
fn shell_script<'a>(parts: &[&'a str]) -> Option<&'a str> {
    match parts {
        [shell, flag, script] => {
            let name = shell.rsplit('/').next().unwrap_or(shell);
            let shell_name = matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish");
            let command_flag = flag.strip_prefix('-').is_some_and(|rest| {
                rest.ends_with('c') && rest.chars().all(|char| char.is_ascii_alphabetic())
            });
            (shell_name && command_flag).then_some(*script)
        }
        _ => None,
    }
}

/// The row's label, read from the argument's structure rather than from its
/// rendered text. Rendering first and then keeping the opening line names
/// nothing: a pretty-printed array begins with a bare `[`, and every shell
/// call this provider records is one.
fn value_as_label(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Array(values) => {
            let parts = values
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?;
            match shell_script(&parts) {
                Some(script) => Some(script.to_string()),
                None => Some(parts.join(" ")),
            }
        }
        Value::Object(fields) => TOOL_LABEL_KEYS
            .iter()
            .find_map(|key| fields.get(*key).and_then(value_as_label)),
    }
}

fn join_outputs(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let parts = values
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// The argument value itself, not its rendering: the preview and the row's
/// label are two different readings of it, and flattening here would leave the
/// label with only the punctuation the pretty-printer emitted first.
fn tool_arguments_from_record(value: &Value, call_id: &str) -> Option<Value> {
    let payload = value.get("payload").unwrap_or(value);
    let payload_type = normalized_type(payload.get("type").and_then(Value::as_str));
    if matches!(payload_type.as_str(), "itemcompleted" | "itemstarted") {
        let item = payload.get("item")?;
        let item_type = normalized_type(item.get("type").and_then(Value::as_str));
        return match item_type.as_str() {
            "commandexecution" => item.get("command").cloned(),
            "filechange" => item
                .get("changes")
                .and_then(Value::as_object)
                .map(joined_keys),
            "dynamictoolcall" | "mcptoolcall" => item.get("arguments").cloned(),
            "websearch" => item.get("query").and_then(text_value),
            "imageview" => item.get("path").and_then(text_value),
            _ => item.get("input").cloned(),
        };
    }
    match payload_type.as_str() {
        "functioncall" => payload.get("arguments").and_then(text_value),
        "customtoolcall" => payload.get("input").and_then(text_value),
        "localshellcall" => payload.get("action").cloned(),
        "execcommandbegin" | "execcommandend" => payload.get("command").cloned(),
        "patchapplybegin" | "patchapplyend" => payload
            .get("changes")
            .and_then(Value::as_object)
            .map(joined_keys),
        "mcptoolcallbegin" | "mcptoolcallend" => payload
            .get("invocation")
            .and_then(|invocation| invocation.get("arguments"))
            .cloned(),
        "dynamictoolcallrequest" => payload.get("arguments").cloned(),
        "websearchbegin" | "websearchend" => payload
            .get("query")
            .or_else(|| payload.get("search_query"))
            .cloned(),
        _ => {
            let observed = legacy_call_id(
                payload,
                SessionSourceSpan {
                    offset: 0,
                    length: 0,
                },
            );
            (observed == call_id)
                .then(|| payload.get("arguments").cloned())
                .flatten()
        }
    }
}

/// These fields carried a string or nothing before the argument stayed a
/// `Value`; a non-string here is not an argument.
fn text_value(value: &Value) -> Option<Value> {
    value.as_str().map(|text| Value::String(text.to_string()))
}

fn joined_keys(changes: &serde_json::Map<String, Value>) -> Value {
    Value::String(changes.keys().cloned().collect::<Vec<_>>().join("\n"))
}

fn tool_result_from_record(value: &Value) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    let payload_type = normalized_type(payload.get("type").and_then(Value::as_str));
    if matches!(payload_type.as_str(), "itemcompleted" | "itemstarted") {
        let item = payload.get("item")?;
        return join_outputs([
            item.get("aggregated_output").and_then(value_as_preview),
            item.get("stdout").and_then(value_as_preview),
            item.get("stderr").and_then(value_as_preview),
            item.get("result").and_then(value_as_preview),
            item.get("error").and_then(value_as_preview),
            item.get("content_items").and_then(value_as_preview),
        ]);
    }
    join_outputs([
        payload.get("output").and_then(value_as_preview),
        payload.get("stdout").and_then(value_as_preview),
        payload.get("stderr").and_then(value_as_preview),
        payload.get("result").and_then(value_as_preview),
        payload.get("error").and_then(value_as_preview),
        payload.get("formatted_output").and_then(value_as_preview),
    ])
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
        return (Some("[binary or base64 content omitted]".to_string()), true);
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

fn tool_payloads(
    file: &mut File,
    entry: &SessionTimelineIndexEntry,
) -> Result<(Option<Value>, Option<String>), String> {
    let tool = entry
        .tool
        .as_ref()
        .ok_or_else(|| "timeline event is not a tool call".to_string())?;
    let call_value = read_source_value(file, entry.source)?;
    let arguments = tool_arguments_from_record(&call_value, &tool.provider_call_id);
    let result = if let Some(result_source) = tool.result_source {
        if result_source == entry.source {
            tool_result_from_record(&call_value)
        } else {
            let result_value = read_source_value(file, result_source)?;
            tool_result_from_record(&result_value)
        }
    } else {
        tool_result_from_record(&call_value)
    };
    Ok((arguments, result))
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
    if let Some(tool) = &entry.tool {
        query.included_tool_categories.contains(&tool.category)
    } else {
        true
    }
}

pub(crate) fn read_timeline_page(
    path: &Path,
    index: &SessionTimelineIndex,
    query: &SessionTimelineQuery,
) -> Result<SessionTimelinePageSlice, String> {
    if index.parser_version != TIMELINE_PARSER_VERSION {
        return Err("Codex timeline index version is stale".to_string());
    }
    let mut file =
        File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let limit = query.limit.clamp(1, 50);
    let start = query.start_index.unwrap_or(0).min(index.events.len());
    let mut summaries = BTreeMap::<String, SessionTurnToolSummary>::new();
    for entry in &index.events {
        if let (Some(turn_id), Some(summary)) = (&entry.turn_id, &entry.turn_tool_summary) {
            summaries.insert(turn_id.clone(), summary.clone());
        }
    }
    let mut summary_already_emitted = BTreeSet::new();
    for entry in index.events.iter().take(start) {
        if matching_entry(entry, query) && entry.kind != SessionTimelineEventKind::ToolCall {
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
        let text = if entry.kind == SessionTimelineEventKind::ToolCall {
            None
        } else {
            let value = read_source_value(&mut file, entry.source)?;
            bounded_text(
                timeline_text(&value, entry.kind),
                timeline_message_preview_bytes(),
            )
            .0
        };
        let tool = if let Some(tool_index) = &entry.tool {
            let (arguments, result) = tool_payloads(&mut file, entry)?;
            let summary = tool_summary(tool_index, arguments.as_ref());
            let (argument_preview, argument_truncated) = bounded_text(
                arguments.as_ref().and_then(value_as_preview),
                timeline_tool_preview_bytes(),
            );
            let (result_preview, result_truncated) =
                bounded_text(result, timeline_tool_preview_bytes());
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
            );
        if !events.is_empty()
            && page_content_bytes.saturating_add(event_content_bytes)
                > timeline_page_content_bytes()
        {
            break;
        }
        page_content_bytes = page_content_bytes.saturating_add(event_content_bytes);
        position += 1;
        let mut turn_tool_summary = None;
        if entry.kind != SessionTimelineEventKind::ToolCall {
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
        });
    }
    let next_index = index.events[position..]
        .iter()
        .any(|entry| matching_entry(entry, query))
        .then_some(position);
    Ok(SessionTimelinePageSlice { events, next_index })
}

fn tool_summary(tool: &SessionToolIndexEntry, arguments: Option<&Value>) -> String {
    if let Some(path) = tool.affected_paths.first() {
        return path.clone();
    }
    // Line breaks survive: the row caps and ellipsizes the label itself, and
    // the hover title is the better place for the rest of a wrapped command.
    arguments
        .and_then(value_as_label)
        .map(|label| truncate_chars(label.trim(), 160))
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| tool.native_name.clone())
}

pub(crate) fn read_event_detail(
    path: &Path,
    index: &SessionTimelineIndex,
    event_id: &str,
) -> Result<SessionEventDetail, String> {
    if index.parser_version != TIMELINE_PARSER_VERSION {
        return Err("Codex timeline index version is stale".to_string());
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
    let (arguments, result) = tool_payloads(&mut file, entry)?;
    // The pane shows the argument as written, pretty-printed; only the row's
    // one-line label reads its structure.
    let arguments = arguments.as_ref().and_then(value_as_preview);
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

    fn write_jsonl(path: &Path, values: Vec<Value>) {
        let mut transcript = values
            .into_iter()
            .map(|value| serde_json::to_string(&value).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        transcript.push('\n');
        fs::write(path, transcript).unwrap();
    }

    fn context<'a>(project_root: &'a Path) -> SessionTimelineContext<'a> {
        SessionTimelineContext {
            resource_id: "conversation:codex:test",
            provider_session_id: "session-1",
            project_root,
            project_root_aliases: &[],
        }
    }

    fn count_value(count: &SessionCount) -> u64 {
        count.value.expect("supported count")
    }

    #[test]
    fn active_session_metadata_alone_controls_subagent_classification() {
        let temp = tempfile::tempdir().unwrap();
        let user_leaf = temp.path().join("user-leaf.jsonl");
        write_jsonl(
            &user_leaf,
            vec![
                json!({"type":"session_meta","payload":{"id":"leaf-user","cwd":"/tmp/project","thread_source":"user","forked_from_id":"ancestor-subagent"}}),
                json!({"type":"session_meta","payload":{"id":"ancestor-subagent","cwd":"/tmp/project","thread_source":"subagent","forked_from_id":"root-user"}}),
                json!({"type":"session_meta","payload":{"id":"root-user","cwd":"/tmp/project","thread_source":"user"}}),
            ],
        );

        let user_summary = summarize_rollout_file(&user_leaf).unwrap();
        assert_eq!(user_summary.session_id, "leaf-user");
        assert!(!user_summary.is_internal_subagent);

        let subagent_leaf = temp.path().join("subagent-leaf.jsonl");
        write_jsonl(
            &subagent_leaf,
            vec![
                json!({"type":"session_meta","payload":{"id":"leaf-subagent","cwd":"/tmp/project","thread_source":"subagent","forked_from_id":"ancestor-user"}}),
                json!({"type":"session_meta","payload":{"id":"ancestor-user","cwd":"/tmp/project","thread_source":"user"}}),
            ],
        );

        let subagent_summary = summarize_rollout_file(&subagent_leaf).unwrap();
        assert_eq!(subagent_summary.session_id, "leaf-subagent");
        assert!(subagent_summary.is_internal_subagent);
    }

    /// A shell call is named by the script it ran. Rendering the argument and
    /// keeping the first line named every such row `[`, because the argument is
    /// an argv array and that is what a pretty-printed array opens with.
    #[test]
    fn a_tool_row_is_named_by_its_command_not_its_punctuation() {
        fn label(argument: Value) -> String {
            let tool = SessionToolIndexEntry {
                provider_call_id: "call-1".to_string(),
                native_name: "exec_command".to_string(),
                category: SessionToolCategory::Shell,
                status: SessionToolCallStatus::Completed,
                affected_paths: Vec::new(),
                result_source: None,
            };
            tool_summary(&tool, Some(&argument))
        }

        // The shape every recorded shell call takes: the shell and its flag are
        // identical on every row, so neither names the call.
        assert_eq!(
            label(json!(["/bin/zsh", "-lc", "sed -n '1,260p' SKILL.md"])),
            "sed -n '1,260p' SKILL.md"
        );
        assert_eq!(label(json!(["bash", "-c", "git status"])), "git status");
        // A bare argv is equally legal, and there the command is the whole
        // vector — which is why the shell shape is tested, not the last slot.
        assert_eq!(label(json!(["rg", "parser"])), "rg parser");
        // A shell name in the first slot is not enough on its own.
        assert_eq!(
            label(json!(["zsh", "script.sh", "arg"])),
            "zsh script.sh arg"
        );
        // Objects are read by key, the way the Claude reader reads them.
        assert_eq!(
            label(json!({"title": "Inspect the setup page", "code": "print(1)"})),
            "Inspect the setup page"
        );
        // Nothing to say falls back to the tool's own name rather than to a
        // brace: an empty object used to render as `{`.
        assert_eq!(label(json!({})), "exec_command");
        assert_eq!(label(json!([])), "exec_command");
        // A plain string argument is already its own label.
        assert_eq!(label(json!("rg -n needle")), "rg -n needle");
    }

    #[test]
    fn session_index_parser_accepts_only_current_field_names() {
        let parsed = parse_session_index_lines([
            r#"{"id":"current","thread_name":"Current title","updated_at":"2026-08-07T00:00:00Z"}"#,
            r#"{"thread_id":"old-id","thread_name":"Old identity"}"#,
            r#"{"id":"old-title","title":"Old title"}"#,
            r#"{"id":"old-time","thread_name":"Current fields","updatedAt":"2026-08-07T00:00:00Z"}"#,
        ]);

        assert_eq!(parsed["current"].0.as_deref(), Some("Current title"));
        assert!(parsed["current"].1.is_some());
        assert!(!parsed.contains_key("old-id"));
        assert_eq!(parsed["old-title"].0, None);
        assert_eq!(parsed["old-time"].1, None);
    }

    #[test]
    fn current_canonical_items_drive_activity_message_metrics() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({
                    "timestamp": "2026-08-07T00:00:00Z",
                    "type": "session_meta",
                    "payload": {"id": "session-1", "cwd": project_root}
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:00Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "developer",
                        "content": [{"type": "input_text", "text": "Injected instructions"}]
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:00Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "Raw duplicate prompt"}]
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": "Obsolete duplicate prompt"
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:01Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "item_completed",
                        "turn_id": "turn-1",
                        "item": {
                            "type": "UserMessage",
                            "id": "user-1",
                            "content": [{"type": "text", "text": "Canonical prompt"}]
                        }
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:02Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "item_completed",
                        "turn_id": "turn-1",
                        "item": {
                            "type": "AgentMessage",
                            "id": "agent-commentary",
                            "phase": "commentary",
                            "content": [{"type": "text", "text": "Working on it."}]
                        }
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:03Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "item_completed",
                        "turn_id": "turn-1",
                        "item": {
                            "type": "AgentMessage",
                            "id": "agent-final",
                            "phase": "final_answer",
                            "content": [{"type": "text", "text": "Done."}]
                        }
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:03Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": "Obsolete duplicate response"
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:04Z",
                    "type": "response_item",
                    "payload": {
                        "type": "custom_tool_call",
                        "call_id": "call-1",
                        "name": "exec_command",
                        "input": "cargo test"
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:04Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {"total_token_usage": {
                            "input_tokens": 80,
                            "output_tokens": 20,
                            "total_tokens": 100
                        }}
                    }
                }),
                json!({
                    "timestamp": "2026-08-07T00:00:05Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {"total_token_usage": {
                            "input_tokens": 100,
                            "output_tokens": 23,
                            "total_tokens": 123
                        }}
                    }
                }),
            ],
        );

        let bytes = fs::read(&transcript).unwrap();
        let from_file = summarize_rollout_file(&transcript).unwrap();
        let from_bytes = summarize_rollout_bytes(&bytes).unwrap();
        assert_eq!(from_file, from_bytes);
        assert_eq!(from_file.user_rounds, 1);
        assert_eq!(from_file.agent_messages, 1);
        assert_eq!(from_file.tool_calls, 1);
        assert_eq!(from_file.total_input_tokens, Some(100));
        assert_eq!(from_file.total_output_tokens, Some(23));
        assert_eq!(from_file.total_tokens, Some(123));
        assert_eq!(from_file.summary.as_deref(), Some("Canonical prompt"));
        assert_eq!(from_file.title.as_deref(), Some("Canonical prompt"));
        assert!(from_file.metrics_complete);
        assert_eq!(
            first_codex_user_summary(&bytes).as_deref(),
            Some("Canonical prompt")
        );
    }

    #[test]
    fn message_events_drive_activity_metrics_when_canonical_items_are_absent() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({
                    "timestamp": "2026-08-20T00:00:00Z",
                    "type": "session_meta",
                    "payload": {"id": "session-1", "cwd": project_root}
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:00Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "Raw duplicate prompt"}]
                    }
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": "Debug the empty message count"
                    }
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:01Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": "I will inspect it.",
                        "phase": "commentary"
                    }
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "agent_message",
                        "id": "inter-agent-1",
                        "author": "agent-a",
                        "recipient": "agent-b",
                        "content": "Coordinate the parser audit"
                    }
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:01Z",
                    "type": "inter_agent_communication_metadata",
                    "payload": {
                        "sender": "agent-a",
                        "recipients": ["agent-b"]
                    }
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:02Z",
                    "type": "response_item",
                    "payload": {
                        "type": "custom_tool_call",
                        "call_id": "call-1",
                        "name": "exec",
                        "input": "inspect records"
                    }
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:03Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": "The count is fixed.",
                        "phase": "final_answer"
                    }
                }),
                json!({
                    "timestamp": "2026-08-20T00:00:04Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {"total_token_usage": {
                            "input_tokens": 456000,
                            "output_tokens": 447,
                            "total_tokens": 456447
                        }}
                    }
                }),
            ],
        );

        let bytes = fs::read(&transcript).unwrap();
        let from_file = summarize_rollout_file(&transcript).unwrap();
        let from_bytes = summarize_rollout_bytes(&bytes).unwrap();
        assert_eq!(from_file, from_bytes);
        assert_eq!(from_file.user_rounds, 1);
        assert_eq!(from_file.agent_messages, 1);
        assert_eq!(from_file.tool_calls, 1);
        assert_eq!(from_file.total_input_tokens, Some(456000));
        assert_eq!(from_file.total_output_tokens, Some(447));
        assert_eq!(from_file.total_tokens, Some(456447));
        assert_eq!(
            from_file.summary.as_deref(),
            Some("Debug the empty message count")
        );
        assert_eq!(
            first_codex_user_summary(&bytes).as_deref(),
            Some("Debug the empty message count")
        );
        assert!(from_file.metrics_complete);
    }

    #[test]
    fn canonical_items_win_without_duplicates_and_hide_reasoning() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"timestamp":"2026-08-07T00:00:00Z","type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"UserMessage","id":"user-1","content":[{"type":"text","text":"Please inspect the parser"}]},"completed_at_ms":1786060800000_i64}}),
                json!({"timestamp":"2026-08-07T00:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"Please inspect the parser"}}),
                json!({"timestamp":"2026-08-07T00:00:01Z","type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"AgentMessage","id":"agent-progress","phase":"commentary","content":[{"type":"text","text":"I am checking it."}]},"completed_at_ms":1786060801000_i64}}),
                json!({"timestamp":"2026-08-07T00:00:02Z","type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"Reasoning","id":"reasoning-1","summary_text":["private reasoning"],"raw_content":["never show"]},"completed_at_ms":1786060802000_i64}}),
                json!({"timestamp":"2026-08-07T00:00:03Z","type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"CommandExecution","id":"call-1","command":["rg","parser"],"status":"completed","stdout":"found parser","stderr":""},"completed_at_ms":1786060803000_i64}}),
                json!({"timestamp":"2026-08-07T00:00:03Z","type":"response_item","payload":{"type":"function_call","call_id":"call-1","name":"exec_command","arguments":"{\"cmd\":\"rg parser\"}"}}),
                json!({"timestamp":"2026-08-07T00:00:04Z","type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"AgentMessage","id":"agent-final","phase":"final_answer","content":[{"type":"text","text":"The parser is sound."}]},"completed_at_ms":1786060804000_i64}}),
                json!({"timestamp":"2026-08-07T00:00:04Z","type":"event_msg","payload":{"type":"agent_message","message":"The parser is sound.","phase":"final_answer"}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.prompts), 1);
        assert_eq!(count_value(&index.counts.responses), 1);
        assert_eq!(count_value(&index.counts.intermediate_steps), 1);
        assert_eq!(count_value(&index.counts.tool_calls), 1);
        assert_eq!(count_value(&index.counts.shell_calls), 1);
        assert_eq!(index.events.len(), 4);
        assert!(!index.events.iter().any(|entry| {
            entry.event_id.contains("reasoning")
                || entry.kind == SessionTimelineEventKind::Checkpoint
        }));
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
        let tool = index
            .events
            .iter()
            .find_map(|entry| entry.tool.as_ref())
            .unwrap();
        assert_eq!(tool.provider_call_id, "call-1");
        assert_eq!(tool.category, SessionToolCategory::Shell);
        assert_eq!(tool.status, SessionToolCallStatus::Completed);
    }

    #[test]
    fn completed_web_search_extensions_are_not_reported_as_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"UserMessage","id":"user-1","content":[{"type":"text","text":"Search the web"}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","completed_at_ms":1786060801000_i64,"item":{"type":"Extension","id":"search-1","kind":"web.search","query":"mallards","results":[{"title":"Mallard"}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","completed_at_ms":1786060802000_i64,"item":{"type":"Extension","id":"search-2","kind":"web.search","status":"failed","error":"network unavailable"}}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.tool_calls), 2);
        assert_eq!(index.counts.tool_calls.quality, SessionCountQuality::Exact);
        assert!(!index
            .warnings
            .iter()
            .any(|warning| warning.code == "incomplete_tool_call"));
        let statuses = index
            .events
            .iter()
            .filter_map(|entry| entry.tool.as_ref())
            .map(|tool| (tool.provider_call_id.as_str(), tool.status))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(statuses["search-1"], SessionToolCallStatus::Completed);
        assert_eq!(statuses["search-2"], SessionToolCallStatus::Failed);
    }

    #[test]
    fn message_previews_use_the_one_mib_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        let mut prompt_text = "p ".repeat(8 * 1024);
        prompt_text.push('p');
        let mut response_text = "r ".repeat(timeline_message_preview_bytes() / 2);
        response_text.push('r');
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"UserMessage","id":"user-1","content":[{"type":"text","text":prompt_text}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"AgentMessage","id":"response-1","phase":"final_answer","content":[{"type":"text","text":response_text}]}}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        let prompt = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::Prompt)
            .unwrap();
        let response = index
            .events
            .iter()
            .find(|entry| entry.kind == SessionTimelineEventKind::Response)
            .unwrap();
        assert_eq!(prompt.quality, SessionCountQuality::Exact);
        assert_eq!(response.quality, SessionCountQuality::LowerBound);
        assert!(index.warnings.iter().any(|warning| {
            warning.code == "message_preview_truncated" && warning.message.contains("1-MiB")
        }));

        let query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([
                SessionTimelineEventKind::Prompt,
                SessionTimelineEventKind::Response,
            ]),
            included_tool_categories: BTreeSet::new(),
            turn_id: None,
        };
        let page = read_timeline_page(&transcript, &index, &query).unwrap();
        assert_eq!(page.events.len(), 2);
        assert_eq!(page.events[0].text.as_ref().unwrap().len(), 16 * 1024 + 1);
        let response_preview = page.events[1].text.as_ref().unwrap();
        assert!(response_preview.ends_with('…'));
        assert!(response_preview.len() <= timeline_message_preview_bytes());
    }

    #[test]
    fn legacy_calls_pair_once_and_unphased_messages_are_inferred() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"timestamp":"2026-08-07T00:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"Read the file"}}),
                json!({"timestamp":"2026-08-07T00:00:01Z","type":"event_msg","payload":{"type":"agent_message","message":"I will inspect it."}}),
                json!({"timestamp":"2026-08-07T00:00:02Z","type":"response_item","payload":{"type":"function_call","call_id":"read-1","name":"read_file","arguments":"{\"file_path\":\"src/lib.rs\"}"}}),
                json!({"timestamp":"2026-08-07T00:00:03Z","type":"response_item","payload":{"type":"function_call_output","call_id":"read-1","output":"file contents"}}),
                json!({"timestamp":"2026-08-07T00:00:04Z","type":"event_msg","payload":{"type":"agent_message","message":"It is complete."}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.tool_calls), 1);
        assert_eq!(count_value(&index.counts.reads), 1);
        assert_eq!(
            index.counts.responses.quality,
            SessionCountQuality::Inferred
        );
        assert_eq!(
            index.counts.intermediate_steps.quality,
            SessionCountQuality::Inferred
        );
        let tool_entry = index
            .events
            .iter()
            .find(|entry| entry.tool.is_some())
            .unwrap();
        assert_eq!(
            tool_entry.tool.as_ref().unwrap().status,
            SessionToolCallStatus::Completed
        );
        assert!(tool_entry.parent_event_id.is_some());

        let query = SessionTimelineQuery {
            start_index: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::Response]),
            included_tool_categories: BTreeSet::new(),
            turn_id: None,
        };
        let page = read_timeline_page(&transcript, &index, &query).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].text.as_deref(), Some("It is complete."));
        assert_eq!(
            page.events[0]
                .turn_tool_summary
                .as_ref()
                .and_then(|summary| summary.tool_calls.value),
            Some(1),
            "the summary moves to the leading event that survives filtering"
        );

        let detail = read_event_detail(&transcript, &index, &tool_entry.event_id).unwrap();
        assert!(detail.arguments.as_deref().unwrap().contains("src/lib.rs"));
        assert_eq!(detail.result.as_deref(), Some("file contents"));
    }

    #[test]
    fn nested_code_mode_completions_do_not_degrade_durable_tool_calls() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Generate and apply the result"}}),
                json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"exec-1","name":"exec","input":"run nested tools"}}),
                json!({"type":"event_msg","payload":{"type":"image_generation_end","call_id":"image-1","status":"completed","result":"generated"}}),
                json!({"type":"event_msg","payload":{"type":"patch_apply_end","call_id":"patch-1","success":true,"changes":{}}}),
                json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"exec-1","output":"done"}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(index.parser_version, TIMELINE_PARSER_VERSION);
        assert_eq!(count_value(&index.counts.tool_calls), 1);
        assert_eq!(index.counts.tool_calls.quality, SessionCountQuality::Exact);
        assert!(index.warnings.is_empty(), "{:?}", index.warnings);
        let tool = index
            .events
            .iter()
            .find_map(|event| event.tool.as_ref())
            .unwrap();
        assert_eq!(tool.provider_call_id, "exec-1");
        assert_eq!(tool.status, SessionToolCallStatus::Completed);
    }

    #[test]
    fn current_codex_0147_code_mode_patch_paths_attach_to_the_durable_exec() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project1");
        let external_root = temp.path().join("project1_external");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&external_root).unwrap();
        let inside_alpha = project_root.join("generated/session-paths/inside-alpha.txt");
        let inside_beta = project_root.join("generated/session-paths/nested/inside-beta.json");
        let outside = external_root.join("outside-alpha.txt");
        let transcript = temp.path().join("rollout.jsonl");
        let exec_input = format!(
            "await tools.apply_patch for {}, {}, and {}",
            inside_alpha.display(),
            inside_beta.display(),
            outside.display()
        );
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","session_id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Create the path fixtures"}}),
                json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"exec-1","name":"exec","input":exec_input}}),
                json!({"type":"event_msg","payload":{"type":"patch_apply_end","call_id":"patch-1","success":true,"status":"completed","changes":{
                    (inside_alpha.to_string_lossy().to_string()): {"type":"add"},
                    (inside_beta.to_string_lossy().to_string()): {"type":"add"},
                    (outside.to_string_lossy().to_string()): {"type":"add"}
                }}}),
                json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"exec-1","output":"done"}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.tool_calls), 1);
        assert_eq!(index.counts.tool_calls.quality, SessionCountQuality::Exact);
        assert!(index.warnings.is_empty(), "{:?}", index.warnings);
        let tool_entry = index
            .events
            .iter()
            .find(|event| event.tool.is_some())
            .unwrap();
        let tool = tool_entry.tool.as_ref().unwrap();
        assert_eq!(tool.provider_call_id, "exec-1");
        assert_eq!(tool.native_name, "exec");
        assert_eq!(tool.status, SessionToolCallStatus::Completed);
        assert_eq!(
            tool.affected_paths,
            vec![
                "generated/session-paths/inside-alpha.txt",
                "generated/session-paths/nested/inside-beta.json",
            ]
        );
        assert!(tool
            .affected_paths
            .iter()
            .all(|path| !Path::new(path).is_absolute()));

        let detail = read_event_detail(&transcript, &index, &tool_entry.event_id).unwrap();
        let arguments = detail.arguments.unwrap();
        assert!(arguments.contains(inside_alpha.to_string_lossy().as_ref()));
        assert!(arguments.contains(inside_beta.to_string_lossy().as_ref()));
        assert!(arguments.contains(outside.to_string_lossy().as_ref()));
        assert!(!tool
            .affected_paths
            .iter()
            .any(|path| path.contains("project1_external")));
    }

    #[test]
    fn current_codex_0147_patch_paths_accept_a_validated_project_root_alias() {
        let temp = tempfile::tempdir().unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        let canonical_root = PathBuf::from("/private/var/folders/run/project1_copy");
        let selected_root = PathBuf::from("/var/folders/run/project1_copy");
        let aliases = vec![canonical_root.clone(), selected_root.clone()];
        let inside = selected_root.join("generated/session-paths/inside-alpha.txt");
        let outside = PathBuf::from("/var/folders/run/project1_copy_external/outside-alpha.txt");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","session_id":"session-1","cwd":selected_root}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Create the path fixtures"}}),
                json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"exec-1","name":"exec","input":"run nested tools"}}),
                json!({"type":"event_msg","payload":{"type":"patch_apply_end","call_id":"patch-1","success":true,"status":"completed","changes":{
                    (inside.to_string_lossy().to_string()): {"type":"add"},
                    (outside.to_string_lossy().to_string()): {"type":"add"}
                }}}),
                json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"exec-1","output":"done"}}),
            ],
        );
        let context = SessionTimelineContext {
            resource_id: "conversation:codex:test",
            provider_session_id: "session-1",
            project_root: &canonical_root,
            project_root_aliases: &aliases,
        };

        let index = build_timeline_index(&transcript, &context).unwrap();
        let tool = index
            .events
            .iter()
            .find_map(|event| event.tool.as_ref())
            .unwrap();
        assert_eq!(tool.provider_call_id, "exec-1");
        assert_eq!(
            tool.affected_paths,
            vec!["generated/session-paths/inside-alpha.txt"]
        );
    }

    #[test]
    fn unmatched_completion_outside_code_mode_remains_an_orphan() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Apply the result"}}),
                json!({"type":"event_msg","payload":{"type":"patch_apply_end","call_id":"patch-1","success":true,"changes":{}}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.tool_calls), 0);
        assert_eq!(
            index.counts.tool_calls.quality,
            SessionCountQuality::LowerBound
        );
        assert!(index
            .warnings
            .iter()
            .any(|warning| warning.code == "orphan_tool_result"));
    }

    #[test]
    fn timeline_indexes_multi_mib_tool_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        let large_output = "tool output ".repeat(5 * 1024 * 1024 / 12);
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Read the generated output"}}),
                json!({"type":"response_item","payload":{"type":"function_call","call_id":"read-1","name":"read_file","arguments":"{}"}}),
                json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"read-1","output":large_output}}),
            ],
        );

        assert_eq!(max_line_bytes() as u64, conversation_file_limit_bytes());
        assert!(fs::metadata(&transcript).unwrap().len() > 4 * 1024 * 1024);
        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert!(!index.truncated);
        assert!(index.warnings.is_empty(), "{:?}", index.warnings);
        assert_eq!(index.counts.tool_calls.quality, SessionCountQuality::Exact);
        assert!(!index
            .warnings
            .iter()
            .any(|warning| warning.code == "incomplete_tool_call"));
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
        let page = read_timeline_page(&transcript, &index, &query).unwrap();
        let preview = page.events[0]
            .tool
            .as_ref()
            .and_then(|tool| tool.result_preview.as_ref())
            .unwrap();
        assert!(page.events[0].tool.as_ref().unwrap().result_truncated);
        assert!(preview.ends_with('…'));
        assert!(preview.len() <= timeline_tool_preview_bytes());
        let detail = read_event_detail(&transcript, &index, &tool_entry.event_id).unwrap();
        let detail_result = detail.result.as_ref().unwrap();
        assert!(detail.result_truncated);
        assert!(detail_result.len() > 64 * 1024);
        assert!(detail_result.len() <= crate::app_settings::timeline_tool_detail_bytes());
    }

    #[test]
    fn large_tool_previews_paginate_without_skipping_the_next_event() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        let arguments = "a ".repeat(300 * 1024);
        let result = "r ".repeat(300 * 1024);
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"UserMessage","id":"user-1","content":[{"type":"text","text":"Run both tools"}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"McpToolCall","id":"tool-1","tool":"large_tool","status":"completed","arguments":arguments.clone(),"result":result.clone()}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"McpToolCall","id":"tool-2","tool":"large_tool","status":"completed","arguments":arguments.clone(),"result":result.clone()}}}),
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
        let first_page = read_timeline_page(&transcript, &index, &first_query).unwrap();
        assert_eq!(first_page.events.len(), 1);
        let first_tool = first_page.events[0].tool.as_ref().unwrap();
        assert_eq!(first_tool.provider_call_id, "tool-1");
        assert_eq!(
            first_tool.argument_preview.as_ref().unwrap().len(),
            arguments.len()
        );
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
        let second_page = read_timeline_page(&transcript, &index, &second_query).unwrap();
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
    fn bounded_text_includes_ellipsis_within_the_byte_limit() {
        let (preview, truncated) = bounded_text(
            Some("鹅".repeat(timeline_tool_preview_bytes())),
            timeline_tool_preview_bytes(),
        );
        let preview = preview.unwrap();

        assert!(truncated);
        assert!(preview.ends_with('…'));
        assert!(preview.len() <= timeline_tool_preview_bytes());
    }

    #[test]
    fn tool_categories_partition_calls_and_paths_stay_project_relative() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join("src")).unwrap();
        let inside = project_root.join("src/lib.rs");
        let outside = temp.path().join("outside.txt");
        let transcript = temp.path().join("rollout.jsonl");
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"UserMessage","id":"user-1","content":[{"type":"text","text":"Change the file"}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"FileChange","id":"edit-1","status":"completed","changes":{(inside.to_string_lossy().to_string()):{"type":"update"},(outside.to_string_lossy().to_string()):{"type":"update"}}}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"McpToolCall","id":"read-1","server":"fs","tool":"read_file","status":"completed","arguments":{"file_path":"src/lib.rs"}}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"WebSearch","id":"other-1","query":"Rust","action":{"type":"search"}}}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        assert_eq!(count_value(&index.counts.tool_calls), 3);
        assert_eq!(count_value(&index.counts.file_edits), 1);
        assert_eq!(count_value(&index.counts.shell_calls), 0);
        assert_eq!(count_value(&index.counts.reads), 1);
        assert_eq!(count_value(&index.counts.other_tools), 1);
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
    fn affected_paths_are_bounded_and_reported_as_a_lower_bound() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let transcript = temp.path().join("rollout.jsonl");
        let changes = (0..MAX_AFFECTED_PATHS_PER_TOOL + 32)
            .map(|index| {
                (
                    project_root
                        .join(format!("generated/file-{index:04}.txt"))
                        .to_string_lossy()
                        .into_owned(),
                    json!({"type": "add"}),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        write_jsonl(
            &transcript,
            vec![
                json!({"type":"session_meta","payload":{"id":"session-1","cwd":project_root}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"UserMessage","id":"user-1","content":[{"type":"text","text":"Create many files"}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"turn-1","item":{"type":"FileChange","id":"edit-1","status":"completed","changes":changes}}}),
            ],
        );

        let index = build_timeline_index(&transcript, &context(&project_root)).unwrap();
        let tool = index
            .events
            .iter()
            .find_map(|event| event.tool.as_ref())
            .unwrap();
        assert_eq!(tool.affected_paths.len(), MAX_AFFECTED_PATHS_PER_TOOL);
        assert_eq!(
            index.counts.tool_calls.quality,
            SessionCountQuality::LowerBound
        );
        assert!(index
            .warnings
            .iter()
            .any(|warning| warning.code == "affected_path_limit"));
    }
}
