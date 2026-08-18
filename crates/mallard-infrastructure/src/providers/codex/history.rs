//! Bounded Codex rollout parsing for Activity summaries and detail previews.
//!
//! Moved out of the old Codex-only chat history module: the neutral activity
//! engine now reaches this parser only through the Codex adapter.

use crate::providers::timeparse::parse_timestamp_value;
use crate::providers::traits::{
    SessionCount, SessionCountQuality, SessionDetailWarning, SessionEventCounts,
    SessionEventDetail, SessionFileSummary, SessionSourceSpan, SessionTimelineContext,
    SessionTimelineEvent, SessionTimelineEventKind, SessionTimelineIndex,
    SessionTimelineIndexEntry, SessionTimelinePageSlice, SessionTimelineQuery,
    SessionToolCallPreview, SessionToolCallStatus, SessionToolCategory, SessionToolIndexEntry,
    SessionTurnToolSummary, TIMELINE_TOOL_PREVIEW_BYTES,
};
use serde::de::{Deserializer, IgnoredAny, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path};

/// A single Codex JSONL record may embed a generated image or a tool result.
/// Match the transcript bound so every record admitted by the transcript
/// safety policy can also be indexed and opened by session detail.
pub(crate) const MAX_LINE_BYTES: usize = mallard_domain::MAX_CONVERSATION_FILE_BYTES as usize;
const MAX_SUMMARY_CHARS: usize = 180;
const TIMELINE_MESSAGE_PREVIEW_BYTES: usize = 16 * 1024;
const TIMELINE_DETAIL_BYTES: usize = 64 * 1024;
const MAX_TIMELINE_EVENTS: usize = 50_000;
pub(crate) const TIMELINE_PARSER_VERSION: u32 = 3;

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

pub(crate) fn extract_user_message(value: &Value) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(content) = payload.get("content") {
        return extract_text(content);
    }
    payload.get("message").and_then(extract_text)
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
    fallback_summary: Option<String>,
    user_round_count: u64,
    agent_message_count: u64,
    tool_call_count: u64,
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
            fallback_summary: None,
            user_round_count: 0,
            agent_message_count: 0,
            tool_call_count: 0,
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
            self.is_internal_subagent |= session_meta_is_internal_subagent(payload);
            if self.thread_id.is_none() {
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
        if payload_type == "user_message" {
            if let Some(text) = genuine_user_event(payload) {
                self.user_round_count = self.user_round_count.saturating_add(1);
                if self.summary.is_none() {
                    self.summary = Some(truncate_chars(&text, MAX_SUMMARY_CHARS));
                }
            }
        } else if payload_type == "agent_message" {
            self.agent_message_count = self.agent_message_count.saturating_add(1);
        }
        if matches!(payload_type, "function_call" | "custom_tool_call") {
            self.tool_call_count = self.tool_call_count.saturating_add(1);
        }
        if payload_type == "token_count" {
            if let Some(total) = payload
                .get("info")
                .and_then(|info| info.get("total_token_usage"))
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64)
            {
                self.total_tokens = Some(self.total_tokens.unwrap_or(0).max(total));
            }
        }
        if self.fallback_summary.is_none() {
            self.fallback_summary = extract_user_message(value)
                .map(|text| truncate_chars(&normalize_summary(&text), MAX_SUMMARY_CHARS))
                .filter(|text| is_meaningful_user_text(text));
        }
    }

    fn finish(self) -> Result<SessionFileSummary, String> {
        let thread_id = self
            .thread_id
            .ok_or_else(|| "rollout has no session id metadata".to_string())?;
        self.cwd
            .as_ref()
            .ok_or_else(|| "rollout has no cwd metadata".to_string())?;
        let summary = self.summary.or(self.fallback_summary);
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
            user_rounds: self.user_round_count,
            agent_messages: self.agent_message_count,
            tool_calls: self.tool_call_count,
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
        match read_bounded_line(reader, &mut bytes, MAX_LINE_BYTES)
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
            .filter(|line| line.len() <= MAX_LINE_BYTES)
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
        .filter(|line| !line.is_empty() && line.len() <= MAX_LINE_BYTES)
    {
        if let Ok(value) = serde_json::from_slice::<Value>(line) {
            parser.consume(&value);
            if parser.summary.is_some() {
                break;
            }
        }
    }
    parser.summary.or(parser.fallback_summary)
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

impl TimelineBuilder {
    fn warning(&mut self, code: &str, message: &str) {
        if self.warnings.iter().any(|warning| warning.code == code) {
            return;
        }
        self.warnings.push(SessionDetailWarning {
            code: code.to_string(),
            message: message.to_string(),
        });
    }

    fn mark_all_lower_bound(&mut self, code: &str, message: &str) {
        self.degraded_all = true;
        self.degraded_tools = true;
        self.truncated = true;
        self.warning(code, message);
    }

    fn mark_tools_lower_bound(&mut self, code: &str, message: &str) {
        self.degraded_tools = true;
        self.warning(code, message);
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
        let quality = if text.len() > TIMELINE_MESSAGE_PREVIEW_BYTES {
            self.warning(
                "message_preview_truncated",
                "At least one Codex message exceeds the 16-KiB timeline display limit and is shown as a truncated preview.",
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
            let pending = &mut self.events[position];
            let Some(tool) = pending.entry.tool.as_mut() else {
                return;
            };
            if tool.native_name == "unknown" && native_name != "unknown" {
                tool.native_name = native_name;
            }
            if tool.category == SessionToolCategory::Other && category != SessionToolCategory::Other
            {
                tool.category = category;
            }
            tool.status = merge_tool_status(tool.status, status);
            if result_source.is_some() {
                tool.result_source = result_source;
            }
            for path in affected_paths {
                if !tool.affected_paths.contains(&path) {
                    tool.affected_paths.push(path);
                }
            }
            if pending.entry.parent_event_id.is_none() {
                pending.entry.parent_event_id = parent_event_id;
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
                    affected_paths,
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
                return;
            }
            self.mark_tools_lower_bound(
                "orphan_tool_result",
                "A Codex tool result had no durable matching call start.",
            );
            return;
        };
        let pending = &mut self.events[position];
        if let Some(tool) = pending.entry.tool.as_mut() {
            tool.status = merge_tool_status(tool.status, status);
            tool.result_source = Some(result_source);
            pending.entry.quality = if tool.status == SessionToolCallStatus::Incomplete {
                SessionCountQuality::LowerBound
            } else {
                SessionCountQuality::Exact
            };
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

fn safe_project_relative_path(path: &str, project_root: &Path) -> Option<String> {
    let candidate = Path::new(path);
    let relative = if candidate.is_absolute() {
        candidate.strip_prefix(project_root).ok()?
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
                Component::CurDir => None,
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn affected_paths_from_value(
    value: &Value,
    project_root: &Path,
    include_direct_path: bool,
) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for path in changes.keys() {
            if let Some(relative) = safe_project_relative_path(path, project_root) {
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
                if let Some(relative) = safe_project_relative_path(path, project_root) {
                    if !result.contains(&relative) {
                        result.push(relative);
                    }
                }
            }
        }
    }
    result.sort();
    result.dedup();
    result
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
    let paths = affected_paths_from_value(
        item,
        context.project_root,
        category == SessionToolCategory::FileEdit,
    );
    let mut status = canonical_item_status(item);
    if status == SessionToolCallStatus::Incomplete
        && matches!(item_type.as_str(), "websearch" | "imageview" | "filechange")
    {
        status = SessionToolCallStatus::Completed;
    }
    Some((id, native_name, category, status, paths))
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
        if let Some((call_id, name, category, _, paths)) = canonical_tool(item, context) {
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
            if let Some((call_id, name, category, status, paths)) = canonical_tool(item, context) {
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
            let paths = affected_paths_from_value(
                payload
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                    .as_ref()
                    .unwrap_or(payload),
                context.project_root,
                category == SessionToolCategory::FileEdit,
            );
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
            let paths = affected_paths_from_value(
                payload,
                context.project_root,
                category == SessionToolCategory::FileEdit,
            );
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
            builder.complete_tool(&call_id, status, source, true);
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
    if metadata.len() > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Codex transcript '{}' exceeds {} bytes",
            path.display(),
            mallard_domain::MAX_CONVERSATION_FILE_BYTES
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
        let line = read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES)
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
            "sessionmeta"
            | "turncontext"
            | "worldstate"
            | "compacted"
            | "interagentcommunication"
            | "interagentcommunicationmetadata" => {}
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
    if span.length == 0 || span.length > (MAX_LINE_BYTES as u64).saturating_add(2) {
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

fn join_outputs(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let parts = values
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn tool_arguments_from_record(value: &Value, call_id: &str) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    let payload_type = normalized_type(payload.get("type").and_then(Value::as_str));
    if matches!(payload_type.as_str(), "itemcompleted" | "itemstarted") {
        let item = payload.get("item")?;
        let item_type = normalized_type(item.get("type").and_then(Value::as_str));
        return match item_type.as_str() {
            "commandexecution" => item.get("command").and_then(value_as_preview),
            "filechange" => item
                .get("changes")
                .and_then(Value::as_object)
                .map(|changes| changes.keys().cloned().collect::<Vec<_>>().join("\n")),
            "dynamictoolcall" | "mcptoolcall" => item.get("arguments").and_then(value_as_preview),
            "websearch" => item
                .get("query")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            "imageview" => item
                .get("path")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            _ => item.get("input").and_then(value_as_preview),
        };
    }
    match payload_type.as_str() {
        "functioncall" => payload
            .get("arguments")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        "customtoolcall" => payload
            .get("input")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        "localshellcall" => payload.get("action").and_then(value_as_preview),
        "execcommandbegin" | "execcommandend" => payload.get("command").and_then(value_as_preview),
        "patchapplybegin" | "patchapplyend" => payload
            .get("changes")
            .and_then(Value::as_object)
            .map(|changes| changes.keys().cloned().collect::<Vec<_>>().join("\n")),
        "mcptoolcallbegin" | "mcptoolcallend" => payload
            .get("invocation")
            .and_then(|invocation| invocation.get("arguments"))
            .and_then(value_as_preview),
        "dynamictoolcallrequest" => payload.get("arguments").and_then(value_as_preview),
        "websearchbegin" | "websearchend" => payload
            .get("query")
            .or_else(|| payload.get("search_query"))
            .and_then(value_as_preview),
        _ => {
            let observed = legacy_call_id(
                payload,
                SessionSourceSpan {
                    offset: 0,
                    length: 0,
                },
            );
            (observed == call_id)
                .then(|| payload.get("arguments").and_then(value_as_preview))
                .flatten()
        }
    }
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
) -> Result<(Option<String>, Option<String>), String> {
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
    let start = if let Some(anchor) = &query.anchor_event_id {
        index
            .events
            .iter()
            .position(|entry| &entry.event_id == anchor)
            .ok_or_else(|| "timeline anchor is not present in this snapshot".to_string())?
    } else {
        query.start_index.unwrap_or(0).min(index.events.len())
    };
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
    let mut position = start;
    while position < index.events.len() && events.len() < limit {
        let entry = &index.events[position];
        position += 1;
        if !matching_entry(entry, query) {
            continue;
        }
        let text = if entry.kind == SessionTimelineEventKind::ToolCall {
            None
        } else {
            let value = read_source_value(&mut file, entry.source)?;
            bounded_text(
                timeline_text(&value, entry.kind),
                TIMELINE_MESSAGE_PREVIEW_BYTES,
            )
            .0
        };
        let tool = if let Some(tool_index) = &entry.tool {
            let (arguments, result) = tool_payloads(&mut file, entry)?;
            let (argument_preview, argument_truncated) =
                bounded_text(arguments, TIMELINE_TOOL_PREVIEW_BYTES);
            let (result_preview, result_truncated) =
                bounded_text(result, TIMELINE_TOOL_PREVIEW_BYTES);
            let summary = tool_summary(tool_index, argument_preview.as_deref());
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

fn tool_summary(tool: &SessionToolIndexEntry, argument_preview: Option<&str>) -> String {
    if let Some(path) = tool.affected_paths.first() {
        return path.clone();
    }
    argument_preview
        .and_then(|preview| preview.lines().next())
        .map(|line| truncate_chars(line, 160))
        .filter(|line| !line.is_empty())
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
    let argument_budget = TIMELINE_DETAIL_BYTES / 2;
    let (arguments, arguments_truncated) = bounded_text(arguments, argument_budget);
    let used = arguments.as_ref().map_or(0, String::len);
    let result_budget = TIMELINE_DETAIL_BYTES.saturating_sub(used).max(1);
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
        }
    }

    fn count_value(count: &SessionCount) -> u64 {
        count.value.expect("supported count")
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
            anchor_event_id: None,
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

        assert_eq!(
            MAX_LINE_BYTES as u64,
            mallard_domain::MAX_CONVERSATION_FILE_BYTES
        );
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
            anchor_event_id: None,
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
        assert!(preview.len() <= TIMELINE_TOOL_PREVIEW_BYTES);
        assert!(read_event_detail(&transcript, &index, &tool_entry.event_id)
            .unwrap()
            .result
            .is_some());
    }

    #[test]
    fn bounded_text_includes_ellipsis_within_the_byte_limit() {
        let (preview, truncated) = bounded_text(
            Some("鹅".repeat(TIMELINE_TOOL_PREVIEW_BYTES)),
            TIMELINE_TOOL_PREVIEW_BYTES,
        );
        let preview = preview.unwrap();

        assert!(truncated);
        assert!(preview.ends_with('…'));
        assert!(preview.len() <= TIMELINE_TOOL_PREVIEW_BYTES);
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
}
