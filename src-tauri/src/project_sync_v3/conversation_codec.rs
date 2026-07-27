//! Versioned, bounded provider transcript codecs used by variant sync.
//!
//! A codec returns `Unknown` instead of guessing whenever it cannot prove one
//! linear message chain. Rewrites are limited to declared structural fields
//! and are round-tripped through the same parser before bytes can become a
//! resumable provider transcript.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::domain::{
    conversation_chain_digests, validate_conversation_chain, CanonicalConversationMessage,
    MessageId, MAX_CONVERSATION_FILE_BYTES,
};

pub const CODEX_LINEAGE_CODEC_KEY: &str = "codex.lineage.v1";
pub const CODEX_CODEC_VERSION: u32 = 2;

/// The capture codec version a manifest declares for a lineage codec key.
/// Codecs without a versioned parser declare 1: the key itself carries the
/// semantic identity and strict manifest validation only requires presence.
pub fn codec_version_for(lineage_codec: &str) -> u32 {
    if lineage_codec == CODEX_LINEAGE_CODEC_KEY {
        CODEX_CODEC_VERSION
    } else {
        1
    }
}

const MAX_TRANSCRIPT_BYTES: usize = MAX_CONVERSATION_FILE_BYTES as usize;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_RECORDS: usize = 100_000;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodecUnknownReason {
    Empty,
    OversizedFile,
    OversizedLine,
    TooManyRecords,
    MalformedJson,
    MissingSessionMetadata,
    ConflictingSessionMetadata,
    UnsupportedCodec,
    MissingMessageId,
    DuplicateMessageId,
    NonLinearParentage,
    InvalidMessageId,
}

impl CodecUnknownReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty_transcript",
            Self::OversizedFile => "oversized_file",
            Self::OversizedLine => "oversized_line",
            Self::TooManyRecords => "too_many_records",
            Self::MalformedJson => "malformed_json",
            Self::MissingSessionMetadata => "missing_session_metadata",
            Self::ConflictingSessionMetadata => "conflicting_session_metadata",
            Self::UnsupportedCodec => "unsupported_codec",
            Self::MissingMessageId => "missing_message_id",
            Self::DuplicateMessageId => "duplicate_message_id",
            Self::NonLinearParentage => "non_linear_parentage",
            Self::InvalidMessageId => "invalid_message_id",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecLineage {
    Complete {
        chain: Vec<CanonicalConversationMessage>,
        lineage_digest: String,
        semantic_digest: String,
    },
    Unknown {
        reason: CodecUnknownReason,
    },
}

impl CodecLineage {
    pub fn chain(&self) -> Option<&[CanonicalConversationMessage]> {
        match self {
            Self::Complete { chain, .. } => Some(chain),
            Self::Unknown { .. } => None,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexTranscriptAnalysis {
    pub source_thread_id: Option<String>,
    pub source_cwd: Option<String>,
    pub record_count: usize,
    pub lineage: CodecLineage,
}

impl CodexTranscriptAnalysis {
    pub fn is_resumable(&self) -> bool {
        self.source_thread_id.is_some() && self.source_cwd.is_some() && self.lineage.is_complete()
    }
}

/// Analyze a Codex rollout without ever treating malformed ancestry as a
/// divergent branch. Content failures are represented as bounded reason codes
/// so capture can safely offer StoredOnly.
pub fn analyze_codex_transcript(bytes: &[u8]) -> CodexTranscriptAnalysis {
    if bytes.is_empty() {
        return unknown(None, None, 0, CodecUnknownReason::Empty);
    }
    if bytes.len() > MAX_TRANSCRIPT_BYTES {
        return unknown(None, None, 0, CodecUnknownReason::OversizedFile);
    }

    let mut source_thread_id = None;
    let mut source_cwd = None;
    let mut chain: Vec<CanonicalConversationMessage> = Vec::new();
    let mut record_count = 0_usize;
    for raw in bytes.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        record_count += 1;
        if record_count > MAX_RECORDS {
            return unknown(
                source_thread_id,
                source_cwd,
                record_count,
                CodecUnknownReason::TooManyRecords,
            );
        }
        if raw.len() > MAX_LINE_BYTES {
            return unknown(
                source_thread_id,
                source_cwd,
                record_count,
                CodecUnknownReason::OversizedLine,
            );
        }
        let value: Value = match serde_json::from_slice(raw) {
            Ok(value) => value,
            Err(_) => {
                return unknown(
                    source_thread_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::MalformedJson,
                )
            }
        };

        if is_record_type(&value, "session_meta") {
            let payload = payload_object(&value);
            let id = payload
                .and_then(|object| string_field(object, &["id", "session_id"]))
                .map(str::to_string);
            let cwd = payload
                .and_then(|object| string_field(object, &["cwd", "working_directory"]))
                .map(str::to_string);
            if merge_unique(&mut source_thread_id, id).is_err()
                || merge_unique(&mut source_cwd, cwd).is_err()
            {
                return unknown(
                    source_thread_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::ConflictingSessionMetadata,
                );
            }
        }

        let Some(identity) =
            message_identity(&value, chain.last().map(|message| &message.message_id))
        else {
            if record_requires_message_id(&value) {
                return unknown(
                    source_thread_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::MissingMessageId,
                );
            }
            continue;
        };
        let message_id = match MessageId::parse(identity.message_id) {
            Ok(id) => id,
            Err(_) => {
                return unknown(
                    source_thread_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::InvalidMessageId,
                )
            }
        };
        let parent_message_id = match identity.parent_message_id {
            Some(parent) => match MessageId::parse(parent) {
                Ok(parent) => Some(parent),
                Err(_) => {
                    return unknown(
                        source_thread_id,
                        source_cwd,
                        record_count,
                        CodecUnknownReason::InvalidMessageId,
                    )
                }
            },
            None => None,
        };
        chain.push(CanonicalConversationMessage {
            message_id,
            parent_message_id,
            semantic_fingerprint: semantic_record_fingerprint(value),
        });
    }

    if source_thread_id.is_none() || source_cwd.is_none() {
        return unknown(
            source_thread_id,
            source_cwd,
            record_count,
            CodecUnknownReason::MissingSessionMetadata,
        );
    }
    if chain.is_empty() {
        return unknown(
            source_thread_id,
            source_cwd,
            record_count,
            CodecUnknownReason::MissingMessageId,
        );
    }
    if let Err(error) = validate_conversation_chain(&chain) {
        let reason = if error.contains("duplicate") {
            CodecUnknownReason::DuplicateMessageId
        } else {
            CodecUnknownReason::NonLinearParentage
        };
        return unknown(source_thread_id, source_cwd, record_count, reason);
    }
    let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain)
        .expect("validated conversation chains always produce digests");
    CodexTranscriptAnalysis {
        source_thread_id,
        source_cwd,
        record_count,
        lineage: CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        },
    }
}

fn unknown(
    source_thread_id: Option<String>,
    source_cwd: Option<String>,
    record_count: usize,
    reason: CodecUnknownReason,
) -> CodexTranscriptAnalysis {
    CodexTranscriptAnalysis {
        source_thread_id,
        source_cwd,
        record_count,
        lineage: CodecLineage::Unknown { reason },
    }
}

struct MessageIdentity {
    message_id: String,
    parent_message_id: Option<String>,
}

fn message_identity(
    value: &Value,
    previous_message_id: Option<&MessageId>,
) -> Option<MessageIdentity> {
    if is_event_message_mirror(value) {
        return None;
    }
    let outer = value.as_object()?;
    let payload = payload_object(value).unwrap_or(outer);
    let message_id = string_field(payload, &["message_id"])
        .or_else(|| string_field(outer, &["message_id"]))
        .or_else(|| {
            record_requires_message_id(value)
                .then(|| string_field(payload, &["id"]))
                .flatten()
        })?;
    let parent_fields = ["parent_message_id", "parent_id"];
    let has_explicit_parent = parent_fields
        .iter()
        .any(|key| payload.contains_key(*key) || outer.contains_key(*key));
    let parent_message_id = string_field(payload, &parent_fields)
        .or_else(|| string_field(outer, &parent_fields))
        .map(str::to_string)
        .or_else(|| {
            (outer.get("type").and_then(Value::as_str) == Some("response_item")
                && !has_explicit_parent)
                .then(|| previous_message_id.map(|message_id| message_id.as_str().to_string()))
                .flatten()
        });
    Some(MessageIdentity {
        message_id: message_id.to_string(),
        parent_message_id,
    })
}

fn record_requires_message_id(value: &Value) -> bool {
    if is_event_message_mirror(value) {
        return false;
    }
    if value.get("type").and_then(Value::as_str) == Some("response_item") {
        return payload_object(value)
            .and_then(|payload| string_field(payload, &["type"]))
            .is_some();
    }
    let Some(object) = payload_object(value).or_else(|| value.as_object()) else {
        return false;
    };
    let Some(kind) = string_field(object, &["type"]) else {
        return false;
    };
    is_lineage_item_kind(kind)
}

fn is_event_message_mirror(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("event_msg")
        && payload_object(value)
            .and_then(|payload| string_field(payload, &["type"]))
            .is_some_and(|kind| matches!(kind, "user_message" | "agent_message"))
}

fn is_lineage_item_kind(kind: &str) -> bool {
    matches!(
        kind,
        "message"
            | "user_message"
            | "agent_message"
            | "reasoning"
            | "tool_call"
            | "tool_result"
            | "function_call"
            | "function_call_output"
            | "custom_tool_call"
            | "custom_tool_call_output"
            | "web_search_call"
            | "tool_search_call"
            | "tool_search_output"
    )
}

fn semantic_record_fingerprint(mut value: Value) -> String {
    let remove_payload_id = value.get("type").and_then(Value::as_str) == Some("response_item")
        || payload_object(&value)
            .and_then(|payload| string_field(payload, &["type"]))
            .is_some_and(is_lineage_item_kind);
    if let Some(outer) = value.as_object_mut() {
        outer.remove("message_id");
        outer.remove("parent_message_id");
        outer.remove("parent_id");
        if let Some(payload) = outer.get_mut("payload").and_then(Value::as_object_mut) {
            payload.remove("message_id");
            payload.remove("parent_message_id");
            payload.remove("parent_id");
            if remove_payload_id {
                payload.remove("id");
            }
        }
    }
    let canonical = canonical_json(&value);
    hex_sha256(canonical.as_bytes())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("JSON string serialization"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let sorted = object.iter().collect::<BTreeMap<_, _>>();
            let entries = sorted
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("JSON key serialization"),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{entries}}}")
        }
    }
}

/// Rewrite only the authoritative Codex session ID and declared cwd fields,
/// then prove the result has identical lineage and semantic digests.
pub fn rewrite_codex_transcript(
    bytes: &[u8],
    expected_source_thread_id: &str,
    destination_thread_id: &str,
    expected_source_cwd: &str,
    destination_cwd: &str,
) -> Result<Vec<u8>, String> {
    validate_rewrite_value("source thread id", expected_source_thread_id, 128)?;
    validate_rewrite_value("destination thread id", destination_thread_id, 128)?;
    validate_rewrite_value("source cwd", expected_source_cwd, 4096)?;
    validate_rewrite_value("destination cwd", destination_cwd, 4096)?;
    let before = analyze_codex_transcript(bytes);
    if !before.is_resumable()
        || before.source_thread_id.as_deref() != Some(expected_source_thread_id)
        || before.source_cwd.as_deref() != Some(expected_source_cwd)
    {
        return Err("Codex transcript does not match the reviewed source identity".to_string());
    }

    let final_newline = bytes.ends_with(b"\n");
    let mut lines = Vec::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        if raw.len() > MAX_LINE_BYTES {
            return Err("Codex transcript line exceeds the rewrite limit".to_string());
        }
        let mut value: Value = serde_json::from_slice(raw)
            .map_err(|_| "Codex transcript contains malformed JSON".to_string())?;
        rewrite_codex_value(
            &mut value,
            expected_source_thread_id,
            destination_thread_id,
            expected_source_cwd,
            destination_cwd,
        )?;
        lines.push(
            serde_json::to_vec(&value)
                .map_err(|error| format!("serialize rewritten Codex record: {error}"))?,
        );
    }
    let mut rewritten = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            rewritten.push(b'\n');
        }
        rewritten.extend_from_slice(line);
    }
    if final_newline {
        rewritten.push(b'\n');
    }

    let after = analyze_codex_transcript(&rewritten);
    if !after.is_resumable()
        || after.source_thread_id.as_deref() != Some(destination_thread_id)
        || after.source_cwd.as_deref() != Some(destination_cwd)
        || lineage_digests(&before) != lineage_digests(&after)
    {
        return Err("rewritten Codex transcript failed semantic round-trip validation".to_string());
    }
    Ok(rewritten)
}

fn lineage_digests(analysis: &CodexTranscriptAnalysis) -> Option<(&str, &str)> {
    match &analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => Some((lineage_digest, semantic_digest)),
        CodecLineage::Unknown { .. } => None,
    }
}

fn rewrite_codex_value(
    value: &mut Value,
    source_thread_id: &str,
    destination_thread_id: &str,
    source_cwd: &str,
    destination_cwd: &str,
) -> Result<(), String> {
    let is_session = is_record_type(value, "session_meta");
    let is_turn = is_record_type(value, "turn_context");
    let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if is_session {
        rewrite_matching_string(payload, "id", source_thread_id, destination_thread_id)?;
        rewrite_matching_string(
            payload,
            "session_id",
            source_thread_id,
            destination_thread_id,
        )?;
    }
    if is_session || is_turn {
        rewrite_matching_string(payload, "cwd", source_cwd, destination_cwd)?;
        rewrite_matching_string(payload, "working_directory", source_cwd, destination_cwd)?;
    }
    Ok(())
}

fn rewrite_matching_string(
    object: &mut Map<String, Value>,
    key: &str,
    expected: &str,
    replacement: &str,
) -> Result<(), String> {
    let Some(value) = object.get_mut(key) else {
        return Ok(());
    };
    let observed = value
        .as_str()
        .ok_or_else(|| format!("Codex structural field '{key}' is not a string"))?;
    // A session_meta `session_id` can identify a parent for subagents. Only
    // rewrite it when it is the reviewed transcript's own ID.
    if observed == expected {
        *value = Value::String(replacement.to_string());
    }
    Ok(())
}

fn payload_object(value: &Value) -> Option<&Map<String, Value>> {
    value.get("payload").and_then(Value::as_object)
}

fn is_record_type(value: &Value, expected: &str) -> bool {
    value.get("type").and_then(Value::as_str) == Some(expected)
        || payload_object(value).and_then(|payload| string_field(payload, &["type"]))
            == Some(expected)
}

fn string_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn merge_unique(target: &mut Option<String>, candidate: Option<String>) -> Result<(), ()> {
    let Some(candidate) = candidate else {
        return Ok(());
    };
    match target {
        Some(existing) if existing != &candidate => Err(()),
        Some(_) => Ok(()),
        None => {
            *target = Some(candidate);
            Ok(())
        }
    }
}

fn validate_rewrite_value(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(format!("invalid Codex {label}"))
    } else {
        Ok(())
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn transcript(thread: &str, cwd: &str, messages: &[(&str, Option<&str>, &str)]) -> Vec<u8> {
        let mut lines = vec![json!({
            "timestamp": "2026-07-22T00:00:00Z",
            "type": "session_meta",
            "payload": {"id": thread, "cwd": cwd}
        })];
        for (index, (id, parent, text)) in messages.iter().enumerate() {
            lines.push(json!({
                "timestamp": format!("2026-07-22T00:00:{:02}Z", index + 1),
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": id,
                    "parent_id": parent,
                    "role": if index % 2 == 0 { "user" } else { "assistant" },
                    "content": [{"type": "input_text", "text": text}]
                }
            }));
        }
        let mut bytes = lines
            .into_iter()
            .map(|line| serde_json::to_string(&line).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn codex_codec_extracts_full_linear_chain() {
        let bytes = transcript(
            "thread-a",
            "/tmp/project-a",
            &[("m-a", None, "one"), ("m-b", Some("m-a"), "two")],
        );
        let analysis = analyze_codex_transcript(&bytes);
        assert!(analysis.is_resumable());
        assert_eq!(analysis.source_thread_id.as_deref(), Some("thread-a"));
        assert_eq!(analysis.lineage.chain().unwrap().len(), 2);
    }

    #[test]
    fn codex_codec_accepts_current_ordered_items_and_ignores_event_mirrors() {
        let records = [
            json!({
                "type": "session_meta",
                "payload": {"id": "thread-current", "cwd": "/tmp/project-current"}
            }),
            json!({
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "mirrored prompt"}
            }),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": "item-a",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "prompt"}]
                }
            }),
            json!({
                "type": "event_msg",
                "payload": {"type": "agent_message", "message": "mirrored answer"}
            }),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "reasoning",
                    "id": "item-b",
                    "summary": []
                }
            }),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "id": "item-c",
                    "call_id": "call-a",
                    "name": "example",
                    "input": "{}"
                }
            }),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call_output",
                    "id": "item-d",
                    "call_id": "call-a",
                    "output": "ok"
                }
            }),
            json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": "item-e",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "answer"}]
                }
            }),
        ];
        let mut bytes = records
            .into_iter()
            .map(|record| serde_json::to_string(&record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');

        let analysis = analyze_codex_transcript(&bytes);
        assert!(analysis.is_resumable());
        let chain = analysis.lineage.chain().unwrap();
        assert_eq!(
            chain
                .iter()
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["item-a", "item-b", "item-c", "item-d", "item-e"]
        );
        for (index, message) in chain.iter().enumerate() {
            assert_eq!(
                message.parent_message_id.as_ref(),
                index
                    .checked_sub(1)
                    .and_then(|previous| chain.get(previous))
                    .map(|previous| &previous.message_id)
            );
        }

        let rewritten = rewrite_codex_transcript(
            &bytes,
            "thread-current",
            "thread-rewritten",
            "/tmp/project-current",
            "/tmp/project-rewritten",
        )
        .unwrap();
        let after = analyze_codex_transcript(&rewritten);
        assert!(after.is_resumable());
        assert_eq!(lineage_digests(&analysis), lineage_digests(&after));
    }

    #[test]
    fn codex_codec_accepts_transcripts_larger_than_16_mib() {
        let mut bytes = transcript(
            "thread-large",
            "/tmp/project-large",
            &[("m-a", None, "one")],
        );
        let padding = serde_json::to_vec(&json!({
            "type": "turn_context",
            "payload": {"padding": "x".repeat(64 * 1024)}
        }))
        .unwrap();
        while bytes.len() <= 16 * 1024 * 1024 {
            bytes.extend_from_slice(&padding);
            bytes.push(b'\n');
        }
        assert!(bytes.len() < MAX_TRANSCRIPT_BYTES);

        let analysis = analyze_codex_transcript(&bytes);
        assert!(analysis.is_resumable());
        assert_eq!(analysis.source_thread_id.as_deref(), Some("thread-large"));
    }

    #[test]
    fn structural_rewrite_preserves_lineage_and_semantics() {
        let bytes = transcript(
            "thread-a",
            "/tmp/project-a",
            &[("m-a", None, "one"), ("m-b", Some("m-a"), "two")],
        );
        let before = analyze_codex_transcript(&bytes);
        let rewritten = rewrite_codex_transcript(
            &bytes,
            "thread-a",
            "thread-b",
            "/tmp/project-a",
            "/tmp/project-b",
        )
        .unwrap();
        let after = analyze_codex_transcript(&rewritten);
        assert_eq!(lineage_digests(&before), lineage_digests(&after));
        assert_eq!(after.source_thread_id.as_deref(), Some("thread-b"));
        assert_eq!(after.source_cwd.as_deref(), Some("/tmp/project-b"));
        assert_ne!(hex_sha256(&bytes), hex_sha256(&rewritten));
    }

    #[test]
    fn malformed_duplicate_and_branched_inputs_are_unknown() {
        let malformed = analyze_codex_transcript(b"{not-json}\n");
        assert_eq!(
            malformed.lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::MalformedJson
            }
        );

        let duplicate = transcript(
            "thread-a",
            "/tmp/project",
            &[("m-a", None, "one"), ("m-a", Some("m-a"), "two")],
        );
        assert!(matches!(
            analyze_codex_transcript(&duplicate).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::DuplicateMessageId
            }
        ));

        let branched = transcript(
            "thread-a",
            "/tmp/project",
            &[
                ("m-a", None, "one"),
                ("m-b", Some("m-a"), "two"),
                ("m-c", Some("m-a"), "fork"),
            ],
        );
        assert!(matches!(
            analyze_codex_transcript(&branched).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::NonLinearParentage
            }
        ));
    }
}
