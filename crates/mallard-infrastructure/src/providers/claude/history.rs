//! Bounded Claude session summaries and detail previews for Activity.

use crate::providers::timeparse::parse_timestamp_value;
use crate::providers::traits::{
    most_specific_project_relative_path, SessionCount, SessionCountQuality, SessionDetailWarning,
    SessionEventCounts, SessionEventDetail, SessionFileSummary, SessionSourceSpan,
    SessionTimelineContext, SessionTimelineEvent, SessionTimelineEventKind, SessionTimelineIndex,
    SessionTimelineIndexEntry, SessionTimelinePageSlice, SessionTimelineQuery,
    SessionToolCallPreview, SessionToolCallStatus, SessionToolCategory, SessionToolIndexEntry,
    SessionTurnToolSummary, TIMELINE_TOOL_PREVIEW_BYTES,
};
use mallard_domain::MAX_CONVERSATION_FILE_BYTES;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

const TITLE_CHARS: usize = 120;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_TIMELINE_EVENTS: usize = 50_000;
const MAX_CLAUDE_RECORDS: usize = 100_000;
const TIMELINE_MESSAGE_PREVIEW_BYTES: usize = 16 * 1024;
const TIMELINE_DETAIL_BYTES: usize = 64 * 1024;
pub(crate) const TIMELINE_PARSER_VERSION: u32 = 2;

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("inspect '{}': {}", path.display(), e))?;
    if meta.len() > MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Claude session '{}' exceeds {} bytes",
            path.display(),
            MAX_CONVERSATION_FILE_BYTES
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

pub(crate) fn summarize_file(path: &Path) -> Result<SessionFileSummary, String> {
    let bytes = read_bounded(path)?;
    summarize_bytes(&bytes)
}

pub(crate) fn summarize_bytes(bytes: &[u8]) -> Result<SessionFileSummary, String> {
    if bytes.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Claude session exceeds {} bytes",
            MAX_CONVERSATION_FILE_BYTES
        ));
    }
    let mut summary = SessionFileSummary {
        metrics_complete: true,
        ..SessionFileSummary::default()
    };
    let mut first_user_text: Option<String> = None;
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            summary.metrics_complete = false;
            continue;
        };
        let Some(object) = value.as_object() else {
            summary.metrics_complete = false;
            continue;
        };
        if summary.session_id.is_empty() {
            if let Some(session) = object.get("sessionId").and_then(Value::as_str) {
                summary.session_id = session.to_string();
            }
        }
        if let Some(timestamp) = object.get("timestamp").and_then(parse_timestamp_value) {
            if summary.started_at.is_none_or(|start| timestamp < start) {
                summary.started_at = Some(timestamp);
            }
            if summary.ended_at.is_none_or(|end| timestamp > end) {
                summary.ended_at = Some(timestamp);
            }
        }
        if let Some(branch) = object.get("gitBranch").and_then(Value::as_str) {
            if !branch.is_empty() {
                summary.branch = Some(branch.to_string());
                summary.recorded_branches.insert(branch.to_string());
            }
        }
        match object.get("type").and_then(Value::as_str) {
            Some("summary") => {
                if let Some(text) = object.get("summary").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        summary.title = Some(truncate_chars(text, TITLE_CHARS));
                    }
                }
            }
            Some("user") if !is_meta_record(object) => {
                summary.user_rounds = summary.user_rounds.saturating_add(1);
                if first_user_text.is_none() {
                    if let Some(text) = record_text(&value) {
                        if !text.trim().is_empty() {
                            first_user_text = Some(truncate_chars(&text, TITLE_CHARS));
                        }
                    }
                }
            }
            Some("assistant") if !is_meta_record(object) => {
                summary.agent_messages = summary.agent_messages.saturating_add(1);
                if let Some(Value::Array(items)) = value
                    .get("message")
                    .and_then(|message| message.get("content"))
                {
                    let calls = items
                        .iter()
                        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
                        .count() as u64;
                    summary.tool_calls = summary.tool_calls.saturating_add(calls);
                }
                if let Some(usage) = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    let input = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if input.saturating_add(output) > 0 {
                        summary.total_tokens = Some(input.saturating_add(output));
                    }
                }
            }
            _ => {}
        }
    }
    if summary.session_id.is_empty() {
        return Err("Claude session file has no session ID".to_string());
    }
    summary.summary = first_user_text.clone();
    if summary.title.is_none() {
        summary.title = first_user_text;
    }
    Ok(summary)
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

#[derive(Default)]
struct ClaudeTimelineBuilder {
    events: Vec<PendingTimelineEvent>,
    assistant_groups: BTreeMap<String, AssistantGroup>,
    tool_positions: BTreeMap<String, usize>,
    prompt_ids: BTreeSet<String>,
    current_turn_id: Option<String>,
    degraded_all: bool,
    degraded_tools: bool,
    inferred_responses: bool,
    truncated: bool,
    warnings: Vec<SessionDetailWarning>,
}

impl ClaudeTimelineBuilder {
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
        let quality = if text_len > TIMELINE_MESSAGE_PREVIEW_BYTES {
            self.warning(
                "message_preview_truncated",
                "At least one Claude message exceeds the 16-KiB timeline display limit and is shown as a truncated preview.",
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
            u8::from(stop_reason == "endturn"),
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
        let tool_key = format!("{turn_id}\0{call_id}");
        if let Some(position) = self.tool_positions.get(&tool_key).copied() {
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
        self.tool_positions.insert(tool_key, position);
        self.events.push(PendingTimelineEvent {
            entry: SessionTimelineIndexEntry {
                event_id: format!("claude:tool:{call_id}"),
                ordinal: 0,
                timestamp,
                kind: SessionTimelineEventKind::ToolCall,
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
            },
            order: (
                source.offset,
                u16::try_from(block_index.saturating_add(1)).unwrap_or(u16::MAX),
            ),
        });
    }

    fn complete_tool(
        &mut self,
        call_id: &str,
        status: SessionToolCallStatus,
        source: SessionSourceSpan,
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
            let (kind, mut quality) = if group.stop_reason == "endturn" {
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
            if text.len() > TIMELINE_MESSAGE_PREVIEW_BYTES {
                quality = SessionCountQuality::LowerBound;
                self.warning(
                    "message_preview_truncated",
                    "At least one Claude message exceeds the 16-KiB timeline display limit and is shown as a truncated preview.",
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
                },
                order: group.order,
            });
        }
        if self.inferred_responses {
            self.warning(
                "message_phase_inferred",
                "At least one Claude assistant message lacked a terminal stop reason, so its final-response placement was inferred.",
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
            .filter(|pending| pending.entry.kind != SessionTimelineEventKind::ToolCall)
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
        let mut tools_by_turn = BTreeMap::<String, u64>::new();
        for pending in &self.events {
            if pending.entry.kind == SessionTimelineEventKind::ToolCall {
                if let Some(turn_id) = &pending.entry.turn_id {
                    *tools_by_turn.entry(turn_id.clone()).or_default() += 1;
                }
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
                    tool_calls: claude_count(
                        value,
                        tool_quality,
                        claude_tool_explanation(tool_quality),
                    ),
                });
            }
        }
        let entries = self
            .events
            .into_iter()
            .map(|pending| pending.entry)
            .collect::<Vec<_>>();
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
    let tool_quality = if degraded_all || degraded_tools {
        SessionCountQuality::LowerBound
    } else {
        SessionCountQuality::Exact
    };
    let visible_explanation = (visible_quality == SessionCountQuality::LowerBound)
        .then_some("Some Claude records could not be safely indexed.");
    SessionEventCounts {
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
                Component::CurDir => None,
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
                    builder.complete_tool(call_id, tool_result_status(block), source);
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
                            builder.complete_tool(call_id, tool_result_status(block), source);
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
        "assistant" => {}
        "summary"
        | "system"
        | "result"
        | "progress"
        | "compactboundary"
        | "filehistorysnapshot"
        | "queueoperation"
        | "lastprompt" => {}
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
    if metadata.len() > MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Claude session '{}' exceeds {} bytes",
            path.display(),
            MAX_CONVERSATION_FILE_BYTES
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
        let line = read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES)
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
                "At least one Claude record exceeded the one-MiB timeline parsing limit.",
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
        consume_claude_record(&mut builder, &value, source_span(start, end), context);
    }
    Ok(builder.finalize())
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

fn value_as_preview(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => serde_json::to_string_pretty(other).ok(),
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
) -> Result<(Option<String>, Option<String>), String> {
    let tool = entry
        .tool
        .as_ref()
        .ok_or_else(|| "timeline event is not a tool call".to_string())?;
    let call_value = read_source_value(file, entry.source)?;
    let arguments = find_tool_block(&call_value, &tool.provider_call_id)
        .and_then(|block| block.get("input"))
        .and_then(value_as_preview);
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
    Ok((arguments, result))
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

pub(crate) fn read_timeline_page(
    path: &Path,
    index: &SessionTimelineIndex,
    query: &SessionTimelineQuery,
) -> Result<SessionTimelinePageSlice, String> {
    if index.parser_version != TIMELINE_PARSER_VERSION {
        return Err("Claude timeline index version is stale".to_string());
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
    let summaries = index
        .events
        .iter()
        .filter_map(|entry| Some((entry.turn_id.clone()?, entry.turn_tool_summary.clone()?)))
        .collect::<BTreeMap<_, _>>();
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
            bounded_text(record_text(&value), TIMELINE_MESSAGE_PREVIEW_BYTES).0
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
            resource_id: "conversation:claude:test",
            provider_session_id: "session-1",
            project_root,
            project_root_aliases: &[],
        }
    }

    fn count_value(count: &SessionCount) -> u64 {
        count.value.expect("supported count")
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
            anchor_event_id: None,
            limit: 50,
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::Response]),
            included_tool_categories: BTreeSet::new(),
            turn_id: None,
        };
        let page = read_timeline_page(&transcript, &index, &response_query).unwrap();
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
        };

        let index = build_timeline_index(&transcript, &context).unwrap();
        assert_eq!(index.parser_version, 2);
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
        let long_prompt = "x".repeat(TIMELINE_MESSAGE_PREVIEW_BYTES + 100);
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
    fn tool_preview_respects_the_shared_byte_limit() {
        let (preview, truncated) = bounded_text(
            Some("鹅".repeat(TIMELINE_TOOL_PREVIEW_BYTES)),
            TIMELINE_TOOL_PREVIEW_BYTES,
        );
        let preview = preview.unwrap();

        assert!(truncated);
        assert!(preview.ends_with('…'));
        assert!(preview.len() <= TIMELINE_TOOL_PREVIEW_BYTES);
    }
}
