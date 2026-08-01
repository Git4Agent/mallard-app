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

use super::codex_rollout::{
    analyze_codex_rollout_structure, session_meta_id, structural_record_type,
    CodexStructureErrorKind, MAX_CODEX_RECORDS,
};
use super::domain::{
    conversation_chain_digests, validate_conversation_chain, CanonicalConversationMessage,
    MessageId,
};

pub const CODEX_LINEAGE_CODEC_KEY: &str = "codex.lineage.v2";
pub const CODEX_CODEC_VERSION: u32 = 2;

#[cfg(test)]
const MAX_TRANSCRIPT_BYTES: usize = super::domain::MAX_CONVERSATION_FILE_BYTES as usize;
const MAX_RECORDS: usize = MAX_CODEX_RECORDS;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodecUnknownReason {
    Empty,
    OversizedFile,
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
    pub is_internal_subagent: bool,
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
    let structure = match analyze_codex_rollout_structure(bytes, None) {
        Ok(structure) => structure,
        Err(error) => return unknown(None, None, 0, structure_error_reason(error.kind), false),
    };
    let source_thread_id = Some(structure.current.id.clone());
    let source_cwd = Some(structure.current.cwd.clone());
    let is_internal_subagent = structure.current.is_internal_subagent;
    let mut chain = Vec::new();
    let mut seen = BTreeMap::<MessageId, (String, Option<MessageId>)>::new();
    let mut idless_turn_ordinals = BTreeMap::<String, usize>::new();
    let mut record_count = 0_usize;
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
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
                is_internal_subagent,
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
                    is_internal_subagent,
                )
            }
        };

        let identity = match durable_message_identity(&value, &mut idless_turn_ordinals) {
            DurableMessageIdentity::Ignore => continue,
            DurableMessageIdentity::MissingId => {
                return unknown(
                    source_thread_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::MissingMessageId,
                    is_internal_subagent,
                )
            }
            DurableMessageIdentity::Present(identity) => identity,
        };
        let message_id = match MessageId::parse(identity.message_id) {
            Ok(id) => id,
            Err(_) => {
                return unknown(
                    source_thread_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::InvalidMessageId,
                    is_internal_subagent,
                )
            }
        };
        let explicit_parent = match identity.parent_message_id {
            Some(parent) => match MessageId::parse(parent) {
                Ok(parent) => Some(parent),
                Err(_) => {
                    return unknown(
                        source_thread_id,
                        source_cwd,
                        record_count,
                        CodecUnknownReason::InvalidMessageId,
                        is_internal_subagent,
                    )
                }
            },
            None => None,
        };
        let fingerprint = semantic_record_fingerprint(value);
        if let Some((existing_fingerprint, existing_parent)) = seen.get(&message_id) {
            if existing_fingerprint == &fingerprint
                && explicit_parent
                    .as_ref()
                    .is_none_or(|parent| Some(parent) == existing_parent.as_ref())
            {
                continue;
            }
            if existing_fingerprint == &fingerprint {
                return unknown(
                    source_thread_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::NonLinearParentage,
                    is_internal_subagent,
                );
            }
            return unknown(
                source_thread_id,
                source_cwd,
                record_count,
                CodecUnknownReason::DuplicateMessageId,
                is_internal_subagent,
            );
        }
        let previous = chain
            .last()
            .map(|message: &CanonicalConversationMessage| message.message_id.clone());
        if explicit_parent.is_some() && explicit_parent != previous {
            return unknown(
                source_thread_id,
                source_cwd,
                record_count,
                CodecUnknownReason::NonLinearParentage,
                is_internal_subagent,
            );
        }
        let parent_message_id = explicit_parent.or(previous);
        seen.insert(
            message_id.clone(),
            (fingerprint.clone(), parent_message_id.clone()),
        );
        chain.push(CanonicalConversationMessage {
            message_id,
            parent_message_id,
            semantic_fingerprint: fingerprint,
        });
    }

    if chain.is_empty() {
        return unknown(
            source_thread_id,
            source_cwd,
            record_count,
            CodecUnknownReason::MissingMessageId,
            is_internal_subagent,
        );
    }
    if let Err(error) = validate_conversation_chain(&chain) {
        let reason = if error.contains("duplicate") {
            CodecUnknownReason::DuplicateMessageId
        } else {
            CodecUnknownReason::NonLinearParentage
        };
        return unknown(
            source_thread_id,
            source_cwd,
            record_count,
            reason,
            is_internal_subagent,
        );
    }
    let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain)
        .expect("validated conversation chains always produce digests");
    CodexTranscriptAnalysis {
        source_thread_id,
        source_cwd,
        is_internal_subagent,
        record_count,
        lineage: CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        },
    }
}

fn structure_error_reason(kind: CodexStructureErrorKind) -> CodecUnknownReason {
    match kind {
        CodexStructureErrorKind::Empty => CodecUnknownReason::Empty,
        CodexStructureErrorKind::OversizedFile => CodecUnknownReason::OversizedFile,
        CodexStructureErrorKind::TooManyRecords => CodecUnknownReason::TooManyRecords,
        CodexStructureErrorKind::MalformedJson => CodecUnknownReason::MalformedJson,
        CodexStructureErrorKind::MissingSessionMetadata
        | CodexStructureErrorKind::MalformedSessionMetadata => {
            CodecUnknownReason::MissingSessionMetadata
        }
        CodexStructureErrorKind::ActiveIdentityMismatch
        | CodexStructureErrorKind::ConflictingParent
        | CodexStructureErrorKind::SelfParent
        | CodexStructureErrorKind::AncestryCycle
        | CodexStructureErrorKind::ForkDepthExceeded
        | CodexStructureErrorKind::UnrelatedMetadata => {
            CodecUnknownReason::ConflictingSessionMetadata
        }
    }
}

fn unknown(
    source_thread_id: Option<String>,
    source_cwd: Option<String>,
    record_count: usize,
    reason: CodecUnknownReason,
    is_internal_subagent: bool,
) -> CodexTranscriptAnalysis {
    CodexTranscriptAnalysis {
        source_thread_id,
        source_cwd,
        is_internal_subagent,
        record_count,
        lineage: CodecLineage::Unknown { reason },
    }
}

struct MessageIdentity {
    message_id: String,
    parent_message_id: Option<String>,
}

enum DurableMessageIdentity {
    Ignore,
    MissingId,
    Present(MessageIdentity),
}

fn durable_message_identity(
    value: &Value,
    idless_turn_ordinals: &mut BTreeMap<String, usize>,
) -> DurableMessageIdentity {
    let Some(outer) = value.as_object() else {
        return DurableMessageIdentity::Ignore;
    };
    let outer_kind = string_field(outer, &["type"]);
    if outer_kind == Some("event_msg") {
        return DurableMessageIdentity::Ignore;
    }
    let payload = payload_object(value).unwrap_or(outer);
    let payload_kind = string_field(payload, &["type"]);
    let durable = outer_kind == Some("response_item")
        || outer_kind.is_some_and(is_durable_message_kind)
        || payload_kind.is_some_and(is_durable_message_kind);
    if !durable
        || !payload_kind
            .or(outer_kind)
            .is_some_and(is_durable_message_kind)
    {
        return DurableMessageIdentity::Ignore;
    }
    let message_id = string_field(payload, &["message_id", "id"])
        .or_else(|| string_field(outer, &["message_id", "id"]))
        .map(str::to_string)
        .or_else(|| {
            synthetic_turn_message_id(payload, idless_turn_ordinals).or_else(|| {
                if std::ptr::eq(payload, outer) {
                    None
                } else {
                    synthetic_turn_message_id(outer, idless_turn_ordinals)
                }
            })
        });
    let Some(message_id) = message_id else {
        return DurableMessageIdentity::MissingId;
    };
    let parent_message_id = string_field(payload, &["parent_message_id", "parent_id"])
        .or_else(|| string_field(outer, &["parent_message_id", "parent_id"]))
        .map(str::to_string);
    DurableMessageIdentity::Present(MessageIdentity {
        message_id,
        parent_message_id,
    })
}

/// Current Codex rollouts can omit `payload.id` from turn-injected durable
/// messages while retaining a stable turn ID. Turn ID plus the record's
/// id-less ordinal gives Mallard a deterministic, transcript-local identity
/// without weakening the missing-ID rule for records that have no durable
/// anchor.
fn synthetic_turn_message_id(
    object: &Map<String, Value>,
    idless_turn_ordinals: &mut BTreeMap<String, usize>,
) -> Option<String> {
    let metadata = object
        .get("internal_chat_message_metadata_passthrough")
        .and_then(Value::as_object)?;
    let turn_id = string_field(metadata, &["turn_id", "turnId"])?;
    MessageId::parse(turn_id.to_string()).ok()?;
    let ordinal = idless_turn_ordinals.entry(turn_id.to_string()).or_default();
    let identity = format!(
        "codex-turn-message-v1:{}",
        hex_sha256(format!("codex-turn-message-v1\0{turn_id}\0{ordinal}").as_bytes())
    );
    *ordinal += 1;
    Some(identity)
}

fn is_durable_message_kind(kind: &str) -> bool {
    matches!(
        kind,
        "message"
            | "user_message"
            | "agent_message"
            | "tool_call"
            | "tool_result"
            | "function_call"
            | "function_call_output"
            | "custom_tool_call"
            | "custom_tool_call_output"
    )
}

fn semantic_record_fingerprint(mut value: Value) -> String {
    if let Some(outer) = value.as_object_mut() {
        outer.remove("timestamp");
        outer.remove("created_at");
        outer.remove("updated_at");
        outer.remove("message_id");
        outer.remove("parent_message_id");
        outer.remove("parent_id");
        outer.remove("id");
        if let Some(payload) = outer.get_mut("payload").and_then(Value::as_object_mut) {
            payload.remove("timestamp");
            payload.remove("created_at");
            payload.remove("updated_at");
            payload.remove("message_id");
            payload.remove("parent_message_id");
            payload.remove("parent_id");
            payload.remove("id");
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
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
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
    let before_structure = analyze_codex_rollout_structure(bytes, Some(expected_source_thread_id))
        .map_err(|error| error.to_string())?;
    let after_structure = analyze_codex_rollout_structure(&rewritten, Some(destination_thread_id))
        .map_err(|error| error.to_string())?;
    if !after.is_resumable()
        || after.source_thread_id.as_deref() != Some(destination_thread_id)
        || after.source_cwd.as_deref() != Some(destination_cwd)
        || lineage_digests(&before) != lineage_digests(&after)
        || before_structure.fork_ancestry != after_structure.fork_ancestry
        || after_structure.structural_cwds
            != std::collections::BTreeSet::from([destination_cwd.to_string()])
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
    _source_cwd: &str,
    destination_cwd: &str,
) -> Result<(), String> {
    let record_type = structural_record_type(value);
    let is_session = record_type == Some("session_meta");
    let is_turn = record_type == Some("turn_context");
    let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if is_session {
        if session_meta_id(payload) == Some(source_thread_id) {
            for key in ["id", "thread_id", "threadId", "session_id", "sessionId"] {
                rewrite_matching_string(payload, key, source_thread_id, destination_thread_id)?;
            }
        }
    }
    if is_session || is_turn {
        for key in ["cwd", "working_directory", "workingDirectory"] {
            rewrite_structural_cwd(payload, key, destination_cwd)?;
        }
    }
    Ok(())
}

fn rewrite_structural_cwd(
    object: &mut Map<String, Value>,
    key: &str,
    destination_cwd: &str,
) -> Result<(), String> {
    let Some(value) = object.get_mut(key) else {
        return Ok(());
    };
    if !value.is_string() {
        return Err(format!("Codex structural field '{key}' is not a string"));
    }
    *value = Value::String(destination_cwd.to_string());
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

pub(crate) fn payload_object(value: &Value) -> Option<&Map<String, Value>> {
    value.get("payload").and_then(Value::as_object)
}

fn string_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
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
    use crate::project_sync::domain::{classify_conversation_relation, ConversationRelation};
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
        assert!(!analysis.is_internal_subagent);
        assert_eq!(analysis.source_thread_id.as_deref(), Some("thread-a"));
        assert_eq!(analysis.lineage.chain().unwrap().len(), 2);
    }

    #[test]
    fn codex_codec_marks_internal_subagent_rollouts() {
        let bytes = transcript("thread-child", "/tmp/project-a", &[("m-a", None, "one")]);
        let first_newline = bytes.iter().position(|byte| *byte == b'\n').unwrap();
        let mut metadata: Value = serde_json::from_slice(&bytes[..first_newline]).unwrap();
        metadata["payload"]["thread_source"] = Value::String("subagent".to_string());
        let mut marked = serde_json::to_vec(&metadata).unwrap();
        marked.push(b'\n');
        marked.extend_from_slice(&bytes[first_newline + 1..]);

        let analysis = analyze_codex_transcript(&marked);
        assert!(analysis.is_resumable());
        assert!(analysis.is_internal_subagent);
    }

    #[test]
    fn codex_codec_infers_ordered_parents_when_ids_are_stable() {
        let bytes = transcript(
            "thread-a",
            "/tmp/project-a",
            &[
                ("m-a", None, "one"),
                ("m-b", None, "two"),
                ("m-c", None, "three"),
            ],
        );
        let chain = analyze_codex_transcript(&bytes)
            .lineage
            .chain()
            .unwrap()
            .to_vec();
        assert_eq!(chain[0].parent_message_id, None);
        assert_eq!(
            chain[1].parent_message_id.as_ref().map(MessageId::as_str),
            Some("m-a")
        );
        assert_eq!(
            chain[2].parent_message_id.as_ref().map(MessageId::as_str),
            Some("m-b")
        );
    }

    #[test]
    fn codex_codec_tracks_turn_scoped_messages_without_payload_ids() {
        let base = transcript(
            "thread-a",
            "/tmp/project-a",
            &[("m-a", None, "one"), ("m-b", Some("m-a"), "two")],
        );
        let base_analysis = analyze_codex_transcript(&base);
        let base_chain = base_analysis.lineage.chain().unwrap().to_vec();
        let mut continued = base;
        for (role, text) in [
            ("developer", "turn-scoped context"),
            ("user", "continue this conversation"),
        ] {
            continued.extend_from_slice(
                &serde_json::to_vec(&json!({
                    "timestamp": "2026-07-22T00:01:00Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": role,
                        "content": [{"type": "input_text", "text": text}],
                        "internal_chat_message_metadata_passthrough": {
                            "turn_id": "turn-continued"
                        }
                    }
                }))
                .unwrap(),
            );
            continued.push(b'\n');
        }
        continued.extend_from_slice(
            &serde_json::to_vec(&json!({
                "timestamp": "2026-07-22T00:01:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": "m-c",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "continued"}]
                }
            }))
            .unwrap(),
        );
        continued.push(b'\n');

        let continued_analysis = analyze_codex_transcript(&continued);
        assert!(continued_analysis.is_resumable());
        let continued_chain = continued_analysis.lineage.chain().unwrap();
        assert_eq!(&continued_chain[..base_chain.len()], base_chain);
        assert_eq!(continued_chain.len(), base_chain.len() + 3);
        assert_ne!(
            continued_chain[base_chain.len()].message_id,
            continued_chain[base_chain.len() + 1].message_id
        );
        assert_eq!(
            classify_conversation_relation(Some(continued_chain), Some(&base_chain)),
            ConversationRelation::CloudPrefixOfLocal
        );

        let rewritten = rewrite_codex_transcript(
            &continued,
            "thread-a",
            "thread-b",
            "/tmp/project-a",
            "/tmp/project-b",
        )
        .unwrap();
        let rewritten_analysis = analyze_codex_transcript(&rewritten);
        assert!(rewritten_analysis.is_resumable());
        assert_eq!(
            lineage_digests(&continued_analysis),
            lineage_digests(&rewritten_analysis)
        );
    }

    #[test]
    fn nested_fork_metadata_does_not_change_active_identity_or_lineage() {
        let mut bytes = transcript(
            "p111",
            "/tmp/project",
            &[("m-a", None, "one"), ("m-b", None, "two")],
        );
        let first_newline = bytes.iter().position(|byte| *byte == b'\n').unwrap();
        let mut active: Value = serde_json::from_slice(&bytes[..first_newline]).unwrap();
        active["payload"]["forked_from_id"] = Value::String("p11".to_string());
        let mut records = vec![active];
        records.extend([
            json!({"type":"session_meta","payload":{"id":"p11","forked_from_id":"p1","cwd":"/tmp/project"}}),
            json!({"type":"session_meta","payload":{"id":"p1","forked_from_id":"p","cwd":"/tmp/project"}}),
            json!({"type":"session_meta","payload":{"id":"p","cwd":"/tmp/project"}}),
        ]);
        let history = bytes.split_off(first_newline + 1);
        bytes = records
            .into_iter()
            .map(|record| serde_json::to_string(&record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');
        bytes.extend_from_slice(&history);

        let analysis = analyze_codex_transcript(&bytes);
        assert!(analysis.is_resumable());
        assert_eq!(analysis.source_thread_id.as_deref(), Some("p111"));
        assert_eq!(analysis.lineage.chain().unwrap().len(), 2);
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
    fn codex_codec_accepts_single_records_larger_than_one_mib() {
        let mut bytes = transcript(
            "thread-large-record",
            "/tmp/project-large",
            &[("m-a", None, "one")],
        );
        bytes.extend_from_slice(
            &serde_json::to_vec(&json!({
                "type": "compacted",
                "payload": {
                    "message": "",
                    "replacement_history": "x".repeat(2 * 1024 * 1024),
                }
            }))
            .unwrap(),
        );
        bytes.push(b'\n');
        assert!(bytes.len() < MAX_TRANSCRIPT_BYTES);

        let analysis = analyze_codex_transcript(&bytes);
        assert!(analysis.is_resumable());
        assert_eq!(
            analysis.source_thread_id.as_deref(),
            Some("thread-large-record")
        );

        let rewritten = rewrite_codex_transcript(
            &bytes,
            "thread-large-record",
            "thread-large-record-copy",
            "/tmp/project-large",
            "/tmp/project-large-copy",
        )
        .unwrap();
        let rewritten_analysis = analyze_codex_transcript(&rewritten);
        assert!(rewritten_analysis.is_resumable());
        assert_eq!(
            rewritten_analysis.source_thread_id.as_deref(),
            Some("thread-large-record-copy")
        );
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

    #[test]
    fn exact_duplicate_items_are_ignored_but_conflicting_parents_are_rejected() {
        let mut exact = transcript(
            "thread-a",
            "/tmp/project",
            &[("m-a", None, "one"), ("m-b", Some("m-a"), "two")],
        );
        let lines = exact
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        exact.extend_from_slice(&lines[2]);
        exact.push(b'\n');
        assert_eq!(
            analyze_codex_transcript(&exact)
                .lineage
                .chain()
                .unwrap()
                .len(),
            2
        );

        let mut conflicting: Value = serde_json::from_slice(&lines[2]).unwrap();
        conflicting["payload"]["parent_id"] = Value::String("m-other".to_string());
        let mut conflicting_bytes = exact;
        conflicting_bytes.extend_from_slice(&serde_json::to_vec(&conflicting).unwrap());
        conflicting_bytes.push(b'\n');
        assert!(matches!(
            analyze_codex_transcript(&conflicting_bytes).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::NonLinearParentage
            }
        ));
    }
}
