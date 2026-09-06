//! Versioned, bounded provider transcript codecs used by variant sync.
//!
//! A codec returns `Unknown` instead of guessing whenever it cannot prove one
//! linear message chain. Rewrites are limited to declared structural fields
//! and are round-tripped through the same parser before bytes can become a
//! resumable provider transcript.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::codex_rollout::{
    check_rollout_bounds, CodexStructureAccumulator, CodexStructureErrorKind,
};
use mallard_domain::{
    conversation_chain_digests, sha256_hex, validate_conversation_chain,
    CanonicalConversationMessage, MessageId,
};

pub const CODEX_LINEAGE_CODEC_KEY: &str = "codex.lineage.v1";
pub const CODEX_CODEC_VERSION: u32 = 1;

#[cfg(test)]
const MAX_TRANSCRIPT_BYTES: usize = mallard_domain::MAX_CONVERSATION_FILE_BYTES as usize;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodecUnknownReason {
    Empty,
    OversizedFile,
    TooManyRecords,
    MalformedJson,
    MissingSessionMetadata,
    ConflictingSessionMetadata,
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
    /// Resume state for an append-only projection over these exact bytes.
    pub last_canonical_message_id: Option<String>,
    pub last_synthetic_turn_id: Option<String>,
    pub next_synthetic_turn_ordinal: u64,
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
///
/// One fused pass: each JSONL record is parsed exactly once and handed to
/// both the structure accumulator and the chain builder. A transcript that
/// yields `Complete` passes both validators over every record, so fusion
/// cannot change a successful analysis. On unsound transcripts the first
/// failure in file order wins, and its diagnostics carry whatever structural
/// identity was already proven — the authoritative session_meta precedes
/// every conversation item in any rollout that can succeed.
pub fn analyze_codex_transcript(bytes: &[u8]) -> CodexTranscriptAnalysis {
    analyze_codex_records(bytes, parsed_records(bytes))
}

pub(crate) fn parsed_records(
    bytes: &[u8],
) -> impl Iterator<Item = Result<Value, serde_json::Error>> + '_ {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|raw| raw.strip_suffix(b"\r").unwrap_or(raw))
        .filter(|raw| !raw.is_empty())
        .map(serde_json::from_slice)
}

pub(crate) fn analyze_codex_records(
    bytes: &[u8],
    records: impl Iterator<Item = Result<Value, serde_json::Error>>,
) -> CodexTranscriptAnalysis {
    crate::sync_metrics::record_transcript_analysis(bytes.len() as u64);
    if let Err(error) = check_rollout_bounds(bytes) {
        return unknown(None, None, 0, structure_error_reason(error.kind), false);
    }
    let mut structure = CodexStructureAccumulator::new(None);
    let mut builder = CanonicalChainBuilder::new();
    for record in records {
        let value = match record {
            Ok(value) => value,
            Err(_) => return unknown_from(&structure, CodecUnknownReason::MalformedJson),
        };
        if let Err(error) = structure.feed(&value) {
            return unknown_from(&structure, structure_error_reason(error.kind));
        }
        if let Err(reason) = builder.feed(value) {
            return unknown_from(&structure, reason);
        }
    }
    let last_canonical_message_id = builder
        .last_message_id()
        .map(|message_id| message_id.as_str().to_string());
    let synthetic_turn_resume = builder.synthetic_turn_resume();
    let chain = builder.finish();

    // Whole-file structural verdict first, preserving the pre-fusion
    // end-of-file precedence: ancestry problems outrank an empty chain.
    let record_count = structure.record_count();
    let fallback_thread_id = structure.current_meta().map(|meta| meta.id.clone());
    let fallback_cwd = structure.current_meta().map(|meta| meta.cwd.clone());
    let fallback_subagent = structure
        .current_meta()
        .is_some_and(|meta| meta.is_internal_subagent);
    let structure = match structure.finalize() {
        Ok(structure) => structure,
        Err(finalize_error) => {
            return unknown(
                fallback_thread_id,
                fallback_cwd,
                record_count,
                structure_error_reason(finalize_error.kind),
                fallback_subagent,
            )
        }
    };
    let source_thread_id = Some(structure.current.id.clone());
    let source_cwd = Some(structure.current.cwd.clone());
    let is_internal_subagent = structure.current.is_internal_subagent;

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
        last_canonical_message_id,
        last_synthetic_turn_id: synthetic_turn_resume
            .as_ref()
            .map(|(turn_id, _)| turn_id.clone()),
        next_synthetic_turn_ordinal: synthetic_turn_resume
            .map(|(_, next_ordinal)| next_ordinal as u64)
            .unwrap_or(0),
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
        last_canonical_message_id: None,
        last_synthetic_turn_id: None,
        next_synthetic_turn_ordinal: 0,
        lineage: CodecLineage::Unknown { reason },
    }
}

/// Mid-file failure result for the fused pass: carries whatever structural
/// identity the accumulator has already proven.
fn unknown_from(
    structure: &CodexStructureAccumulator<'_>,
    reason: CodecUnknownReason,
) -> CodexTranscriptAnalysis {
    unknown(
        structure.current_meta().map(|meta| meta.id.clone()),
        structure.current_meta().map(|meta| meta.cwd.clone()),
        structure.record_count(),
        reason,
        structure
            .current_meta()
            .is_some_and(|meta| meta.is_internal_subagent),
    )
}

/// The outcome of feeding one record to the canonical chain builder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainFeedOutcome {
    /// Not a durable conversation message.
    Ignored,
    /// A benign duplicate re-emission of an already-folded message.
    SkippedDuplicate,
    Appended,
}

/// Incremental canonical-message projection: the single implementation of
/// the `codex.lineage.v1` message identity, parent linkage, and semantic
/// fingerprint rules. `analyze_codex_transcript` drives it over one file;
/// the history-graph resolver drives one builder across every resolved
/// segment in base-to-leaf order; tail refresh resumes one mid-stream from
/// the checkpoint's saved cursors. A resumed builder starts with an empty
/// duplicate-detection set, so cross-boundary duplicate re-emission is
/// outside its view; full-parse verification points re-prove the projection
/// and block on any divergence.
pub struct CanonicalChainBuilder {
    chain: Vec<CanonicalConversationMessage>,
    seen: BTreeMap<MessageId, (String, Option<MessageId>)>,
    idless_turn_ordinals: BTreeMap<String, usize>,
    previous_message_id: Option<MessageId>,
    last_synthetic_turn: Option<(String, usize)>,
}

impl CanonicalChainBuilder {
    pub fn new() -> Self {
        Self {
            chain: Vec::new(),
            seen: BTreeMap::new(),
            idless_turn_ordinals: BTreeMap::new(),
            previous_message_id: None,
            last_synthetic_turn: None,
        }
    }

    /// Resume the projection at a saved split point: the last folded message
    /// (implicit parent of the next appended record) and the id-less
    /// synthetic-turn resume pair.
    pub fn resume(
        last_message_id: Option<MessageId>,
        synthetic_turn: Option<(String, usize)>,
    ) -> Self {
        Self {
            chain: Vec::new(),
            seen: BTreeMap::new(),
            idless_turn_ordinals: synthetic_turn.clone().into_iter().collect(),
            previous_message_id: last_message_id,
            last_synthetic_turn: synthetic_turn,
        }
    }

    pub fn feed(&mut self, value: Value) -> Result<ChainFeedOutcome, CodecUnknownReason> {
        let identity = match durable_message_identity(&value, &mut self.idless_turn_ordinals) {
            DurableMessageIdentity::Ignore => return Ok(ChainFeedOutcome::Ignored),
            DurableMessageIdentity::MissingId => return Err(CodecUnknownReason::MissingMessageId),
            DurableMessageIdentity::Present(identity) => identity,
        };
        let message_id = MessageId::parse(identity.message_id)
            .map_err(|_| CodecUnknownReason::InvalidMessageId)?;
        let explicit_parent = identity
            .parent_message_id
            .map(MessageId::parse)
            .transpose()
            .map_err(|_| CodecUnknownReason::InvalidMessageId)?;
        if let Some(turn) = &identity.synthetic_turn_id {
            self.last_synthetic_turn = self
                .idless_turn_ordinals
                .get(turn)
                .map(|next| (turn.clone(), *next));
        }
        let fingerprint = semantic_record_fingerprint(value);
        if let Some((existing_fingerprint, existing_parent)) = self.seen.get(&message_id) {
            if existing_fingerprint == &fingerprint
                && explicit_parent
                    .as_ref()
                    .is_none_or(|parent| Some(parent) == existing_parent.as_ref())
            {
                return Ok(ChainFeedOutcome::SkippedDuplicate);
            }
            if existing_fingerprint == &fingerprint {
                return Err(CodecUnknownReason::NonLinearParentage);
            }
            return Err(CodecUnknownReason::DuplicateMessageId);
        }
        if explicit_parent.is_some() && explicit_parent != self.previous_message_id {
            return Err(CodecUnknownReason::NonLinearParentage);
        }
        let parent_message_id = explicit_parent.or_else(|| self.previous_message_id.clone());
        self.seen.insert(
            message_id.clone(),
            (fingerprint.clone(), parent_message_id.clone()),
        );
        self.previous_message_id = Some(message_id.clone());
        self.chain.push(CanonicalConversationMessage {
            message_id,
            parent_message_id,
            semantic_fingerprint: fingerprint,
        });
        Ok(ChainFeedOutcome::Appended)
    }

    pub fn appended_len(&self) -> usize {
        self.chain.len()
    }

    pub fn last_appended(&self) -> Option<&CanonicalConversationMessage> {
        self.chain.last()
    }

    pub fn last_message_id(&self) -> Option<&MessageId> {
        self.previous_message_id.as_ref()
    }

    /// The id-less synthetic-turn resume pair after the last fed record.
    pub fn synthetic_turn_resume(&self) -> Option<(String, usize)> {
        self.last_synthetic_turn.clone()
    }

    pub fn finish(self) -> Vec<CanonicalConversationMessage> {
        self.chain
    }
}

impl Default for CanonicalChainBuilder {
    fn default() -> Self {
        Self::new()
    }
}

struct MessageIdentity {
    message_id: String,
    parent_message_id: Option<String>,
    synthetic_turn_id: Option<String>,
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
    let explicit_id = string_field(payload, &["message_id", "id"])
        .or_else(|| string_field(outer, &["message_id", "id"]))
        .map(str::to_string);
    let (message_id, synthetic_turn_id) = match explicit_id {
        Some(id) => (Some(id), None),
        None => {
            let synthetic =
                synthetic_turn_message_id(payload, idless_turn_ordinals).or_else(|| {
                    if std::ptr::eq(payload, outer) {
                        None
                    } else {
                        synthetic_turn_message_id(outer, idless_turn_ordinals)
                    }
                });
            match synthetic {
                Some((id, turn)) => (Some(id), Some(turn)),
                None => (None, None),
            }
        }
    };
    let Some(message_id) = message_id else {
        return DurableMessageIdentity::MissingId;
    };
    let parent_message_id = string_field(payload, &["parent_message_id", "parent_id"])
        .or_else(|| string_field(outer, &["parent_message_id", "parent_id"]))
        .map(str::to_string);
    DurableMessageIdentity::Present(MessageIdentity {
        message_id,
        parent_message_id,
        synthetic_turn_id,
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
) -> Option<(String, String)> {
    let metadata = object
        .get("internal_chat_message_metadata_passthrough")
        .and_then(Value::as_object)?;
    let turn_id = string_field(metadata, &["turn_id", "turnId"])?;
    MessageId::parse(turn_id.to_string()).ok()?;
    let ordinal = idless_turn_ordinals.entry(turn_id.to_string()).or_default();
    let identity = format!(
        "codex-turn-message-v1:{}",
        sha256_hex(format!("codex-turn-message-v1\0{turn_id}\0{ordinal}").as_bytes())
    );
    *ordinal += 1;
    Some((identity, turn_id.to_string()))
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
        // Paginated rollout ordinals are physical storage coordinates. A
        // standalone reconstruction assigns new contiguous ordinals without
        // changing the canonical conversation.
        outer.remove("ordinal");
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
    sha256_hex(&canonical_json(value))
}

pub(crate) fn canonical_json(mut value: Value) -> Vec<u8> {
    // Keep the sorted-key contract explicit. This is a no-op when JSON maps
    // already use BTreeMap, as they do with the current dependency features.
    value.sort_all_objects();
    serde_json::to_vec(&value).expect("JSON value serialization")
}

/// The whole-chain digest pair of a complete analysis, `None` when lineage
/// is unknown.
#[cfg(test)]
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

pub(crate) fn payload_object(value: &Value) -> Option<&Map<String, Value>> {
    value.get("payload").and_then(Value::as_object)
}

fn string_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_preserves_byte_and_hash_fixtures() {
        let fixtures: Vec<(String, String, String)> =
            serde_json::from_str(include_str!("fixtures/canonical_json_cases.json")).unwrap();
        for (input, expected, digest) in fixtures {
            let value: Value = serde_json::from_str(&input).unwrap();
            let canonical = canonical_json(value);
            assert_eq!(canonical, expected.as_bytes(), "{input}");
            assert_eq!(sha256_hex(&canonical), digest, "{input}");
        }
    }

    #[test]
    fn codex_semantic_fingerprint_preserves_fixture_hash() {
        let value = json!({
            "ordinal": 41, "timestamp": "2026-09-04T10:00:00Z",
            "id": "envelope-id", "parent_id": "envelope-parent", "type": "response_item",
            "payload": {
                "id": "message-id", "parent_id": "message-parent",
                "timestamp": "2026-09-04T10:00:01Z", "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "Quote: \"; slash: \\; line:\n雪"}],
                "data": {"z": [null, true, false, -0.0, 1.5], "a": {"z": 2, "a": 1}}
            }
        });
        assert_eq!(
            semantic_record_fingerprint(value),
            "72fe986de37367be9dde9a34653f682befe3e76407ef5ca0513adb59c29f37f4"
        );
    }

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
        assert_eq!(analysis.last_canonical_message_id.as_deref(), Some("m-b"));
    }

    #[test]
    fn codex_codec_ignores_physical_rollout_ordinals() {
        let original = transcript(
            "thread-a",
            "/tmp/project-a",
            &[("m-a", None, "one"), ("m-b", Some("m-a"), "two")],
        );
        let mut renumbered = Vec::new();
        for (ordinal, raw) in original
            .split(|byte| *byte == b'\n')
            .filter(|raw| !raw.is_empty())
            .enumerate()
        {
            let mut value: Value = serde_json::from_slice(raw).unwrap();
            value["ordinal"] = Value::from((ordinal as u64) + 41);
            renumbered.extend_from_slice(&serde_json::to_vec(&value).unwrap());
            renumbered.push(b'\n');
        }

        let before = analyze_codex_transcript(&original);
        let after = analyze_codex_transcript(&renumbered);
        assert_eq!(before.lineage.chain(), after.lineage.chain());
        assert_eq!(lineage_digests(&before), lineage_digests(&after));
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
        assert_eq!(&continued_chain[..base_chain.len()], base_chain.as_slice());
        assert!(continued_chain.len() > base_chain.len());
        assert!(lineage_digests(&continued_analysis).is_some());
        assert_eq!(
            continued_analysis.last_synthetic_turn_id.as_deref(),
            Some("turn-continued")
        );
        assert_eq!(continued_analysis.next_synthetic_turn_ordinal, 2);
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
