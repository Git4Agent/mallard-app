//! Bounded, versioned Claude Code transcript codec.
//!
//! Claude sessions are JSONL files whose durable records (`user`,
//! `assistant`, `system`, and `attachment`) carry `uuid`/`parentUuid`
//! lineage plus a `sessionId` and `cwd`. Real transcripts are append-only
//! but not linear: a rewind appends a record parented to an earlier uuid, a
//! compaction appends a fresh `parentUuid: null` root mid-file, a resume can
//! copy history that keeps its original `sessionId`, and a record can be
//! re-emitted later under a different parent.
//!
//! The canonical conversation is derived from the **Desktop-sanitized
//! projection** of the transcript, not from the raw rows. Claude Desktop's
//! resume-import rewrites live transcripts in place — it strips
//! `thinking`/`redacted_thinking` blocks from assistant rows, deletes rows
//! left empty, and never repairs children's `parentUuid` — so the codec
//! applies exactly that idempotent rewrite first (`project_claude_record`)
//! and analyzes only what survives. An import then changes bytes but not
//! semantics: chain, digests, identity, and record count are all invariant.
//! The walk back from the last chain record resolves each parent to its
//! latest earlier occurrence; a parent absent from the entire projection
//! *stitches* to the nearest preceding chain record (deterministic and
//! append-monotone, matching both sides of an import by construction), and
//! a parent present only at later positions severs the walk into a root
//! there. Canonical chains record normalized parents — each record's parent
//! is the previous chain record, the root's is `None` — so the chain is a
//! sync instrument over surviving content, not a claim about the original
//! links. Records off the path (abandoned branches, shadowed re-emissions,
//! pre-compaction history) are excluded.
//!
//! It returns typed `Unknown` reasons instead of guessing. This codec does not
//! rewrite transcripts: cross-machine materialization decodes
//! the path-encoded wire form per line and restamps the leaf `sessionId` in
//! the pull flow (`session_path_map` line facades + the command layer), so
//! in-root content paths remap with the structural `cwd` instead of
//! surviving as dead source-machine spellings. Raw bytes stay raw
//! everywhere else: captures keep verbatim transcript bytes (thinking
//! included), and byte pins, probe hashes, and torn-tail handling never see
//! the projection.

use crate::conversation_codec::{canonical_json, CodecLineage, CodecUnknownReason};
use crate::providers::traits::SessionAnalysis;
use mallard_domain::{
    conversation_chain_digests, sha256_hex, validate_conversation_chain,
    CanonicalConversationMessage, MessageId,
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const CLAUDE_LINEAGE_CODEC_KEY: &str = "claude.lineage.v1";
pub const CLAUDE_CODEC_VERSION: u32 = 1;

pub(crate) const MAX_CLAUDE_RECORDS: usize = 100_000;

/// One parsed line, classified for chain derivation.
pub(crate) enum ClaudeRecordClass {
    /// Metadata, sidechain, or uuid-less record: counted, never chained.
    NotChain,
    Chain {
        message_id: MessageId,
        parent: Option<MessageId>,
    },
}

/// Classify one record object under the chain rules shared by the one-shot
/// analyzer and the incremental tail feeder.
pub(crate) fn classify_claude_record(
    object: &Map<String, Value>,
) -> Result<ClaudeRecordClass, CodecUnknownReason> {
    if !is_chain_record(object) {
        return Ok(ClaudeRecordClass::NotChain);
    }
    let Some(uuid) = object.get("uuid").and_then(Value::as_str) else {
        return Err(CodecUnknownReason::MissingMessageId);
    };
    let message_id =
        MessageId::parse(uuid.to_string()).map_err(|_| CodecUnknownReason::InvalidMessageId)?;
    let parent = match object.get("parentUuid") {
        None | Some(Value::Null) => None,
        Some(Value::String(parent)) => Some(
            MessageId::parse(parent.to_string())
                .map_err(|_| CodecUnknownReason::InvalidMessageId)?,
        ),
        Some(_) => return Err(CodecUnknownReason::MalformedJson),
    };
    Ok(ClaudeRecordClass::Chain { message_id, parent })
}

fn is_thinking_block(block: &Value) -> bool {
    matches!(
        block.get("type").and_then(Value::as_str),
        Some("thinking") | Some("redacted_thinking")
    )
}

/// An assistant record whose `message.content` is a non-empty array of only
/// thinking blocks: the projection drops it entirely. Sidechain assistant
/// rows qualify too — Desktop's sanitizer keys on the row type alone.
pub(crate) fn is_thinking_only_record(object: &Map<String, Value>) -> bool {
    object.get("type").and_then(Value::as_str) == Some("assistant")
        && object
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|content| !content.is_empty() && content.iter().all(is_thinking_block))
}

/// The P1 projection of one parsed record: Claude Desktop 1.32885.1's
/// observed import rewrite. Assistant rows lose their
/// `thinking`/`redacted_thinking` content blocks; a row left with empty
/// content is dropped entirely (`None`) — uuid, links, and identity
/// contributions and all. Every other row passes through unchanged. The
/// rewrite is idempotent, which is what makes an import invisible: the
/// original and the imported file project to the same record sequence.
pub fn project_claude_record(mut value: Value) -> Option<Value> {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return Some(value);
    }
    let Some(content) = value
        .get_mut("message")
        .and_then(|message| message.get_mut("content"))
        .and_then(Value::as_array_mut)
    else {
        return Some(value);
    };
    if !content.iter().any(is_thinking_block) {
        return Some(value);
    }
    content.retain(|block| !is_thinking_block(block));
    if content.is_empty() {
        None
    } else {
        Some(value)
    }
}

struct ChainRecord {
    message_id: MessageId,
    parent: Option<MessageId>,
    fingerprint: String,
}

/// The identity-relevant projection of one transcript record, shared by the
/// full-value observers and the bounded metadata scan so the two parse
/// strategies cannot drift apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentityParts {
    pub(crate) session_id: Option<String>,
    /// Non-empty `cwd` text; the empty-string filter is already applied.
    pub(crate) cwd: Option<String>,
    pub(crate) is_chain: bool,
    pub(crate) is_durable_metadata: bool,
}

/// Extract identity parts from a parsed record. `None` skips the record
/// entirely: non-objects and thinking-only assistant rows — the only records
/// the Desktop projection removes — are invisible to identity.
pub(crate) fn identity_parts_from_value(value: &Value) -> Option<IdentityParts> {
    let object = value.as_object()?;
    if is_thinking_only_record(object) {
        return None;
    }
    let sidechain = object
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let meta = object
        .get("isMeta")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(IdentityParts {
        session_id: object
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
        cwd: object
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.is_empty())
            .map(str::to_string),
        is_chain: is_chain_record(object),
        is_durable_metadata: matches!(
            object.get("type").and_then(Value::as_str),
            Some("user") | Some("assistant") | Some("system")
        ) && !sidechain
            && !meta,
    })
}

/// The provider identity carried by a Claude transcript's active session.
///
/// Two facts are deliberately separated. The **authorized home** is immutable
/// session ownership, established when the leaf session was authorized; the
/// per-turn `cwd` on later records is mutable execution context — activity
/// metadata that must never move the session. Claude resume/fork files copy
/// historical records without restamping their `sessionId` or `cwd`, so the
/// last projected chain record selects the active session ID, and the
/// authorized home is the first non-empty `cwd` in that ID's final
/// chain-record segment. A session whose authorized home cannot be proven is
/// owned by nobody: `leaf_identity` returns no home rather than falling back
/// to drift.
#[derive(Default)]
pub(crate) struct ClaudeSessionIdentityTracker {
    last_session_id: Option<String>,
    last_cwd: Option<String>,
    first_metadata_identity: Option<(String, String)>,
    leaf_session_id: Option<String>,
    authorized_home_session_id: Option<String>,
    authorized_home: Option<String>,
    saw_chain_record: bool,
}

impl ClaudeSessionIdentityTracker {
    /// Observe one raw or already-projected record.
    pub(crate) fn observe(&mut self, value: &Value) {
        if let Some(parts) = identity_parts_from_value(value) {
            self.observe_parts(parts);
        }
    }

    /// The single identity state machine; both parse strategies feed it.
    pub(crate) fn observe_parts(&mut self, parts: IdentityParts) {
        let IdentityParts {
            session_id,
            cwd,
            is_chain,
            is_durable_metadata,
        } = parts;
        if let Some(observed) = &session_id {
            self.last_session_id = Some(observed.clone());
        }
        if let Some(observed) = &cwd {
            self.last_cwd = Some(observed.clone());
        }
        if is_durable_metadata && self.first_metadata_identity.is_none() {
            if let (Some(session_id), Some(cwd)) = (&session_id, &cwd) {
                self.first_metadata_identity = Some((session_id.clone(), cwd.clone()));
            }
        }
        if is_chain {
            self.saw_chain_record = true;
            let effective_session_id = session_id.clone().or_else(|| self.last_session_id.clone());
            if self.authorized_home_session_id != effective_session_id {
                self.authorized_home_session_id = effective_session_id;
                self.authorized_home = cwd;
            } else if self.authorized_home.is_none() {
                self.authorized_home = cwd;
            }
            self.leaf_session_id = session_id;
        }
    }

    pub(crate) fn observed_identity(&self) -> (Option<String>, Option<String>) {
        (self.last_session_id.clone(), self.last_cwd.clone())
    }

    pub(crate) fn leaf_identity(&self) -> (Option<String>, Option<String>) {
        if !self.saw_chain_record {
            return self
                .first_metadata_identity
                .clone()
                .map_or((None, None), |(session_id, home)| {
                    (Some(session_id), Some(home))
                });
        }
        let session_id = self
            .leaf_session_id
            .clone()
            .or_else(|| self.last_session_id.clone());
        // Only the authorized home confers ownership. When the leaf session's
        // home was never established, the session is unowned — never homed by
        // the last cwd drift happened to leave behind.
        let authorized_home =
            if session_id.is_some() && self.authorized_home_session_id == session_id {
                self.authorized_home.clone()
            } else {
                None
            };
        (session_id, authorized_home)
    }
}

/// One transcript line parsed to exactly the fields identity reads; serde
/// skips every other field — the multi-megabyte content bodies above all —
/// without materializing it. Field extraction is lenient the way
/// `Value::get(..).and_then(as_..)` is: a wrong-typed field reads as absent,
/// never as a parse failure, so this arm ignores exactly the lines the
/// `Value` arm ignores (invalid JSON) and no others.
#[derive(Default)]
pub(crate) struct ClaudeScanRecord {
    session_id: Option<String>,
    cwd: Option<String>,
    record_type: Option<String>,
    is_sidechain: bool,
    is_meta: bool,
    /// `uuid` key presence — `is_chain_record` counts even a present null.
    has_uuid: bool,
    /// `Some` iff `message.content` was an array: each element's `type`
    /// string, or `None` for elements without a string `type`.
    content_block_types: Option<Vec<Option<String>>>,
    /// The line was a JSON object. Non-objects are skipped, matching the
    /// `Value` arm's early return.
    is_object: bool,
}

impl ClaudeScanRecord {
    pub(crate) fn into_identity_parts(self) -> Option<IdentityParts> {
        if !self.is_object || self.is_thinking_only() {
            return None;
        }
        let durable = matches!(
            self.record_type.as_deref(),
            Some("user") | Some("assistant") | Some("system") | Some("attachment")
        );
        let is_durable_metadata = matches!(
            self.record_type.as_deref(),
            Some("user") | Some("assistant") | Some("system")
        ) && !self.is_sidechain
            && !self.is_meta;
        Some(IdentityParts {
            session_id: self.session_id,
            cwd: self.cwd.filter(|cwd| !cwd.is_empty()),
            is_chain: durable && !self.is_sidechain && self.has_uuid,
            is_durable_metadata,
        })
    }

    fn is_thinking_only(&self) -> bool {
        self.record_type.as_deref() == Some("assistant")
            && self.content_block_types.as_ref().is_some_and(|blocks| {
                !blocks.is_empty()
                    && blocks.iter().all(|block_type| {
                        matches!(
                            block_type.as_deref(),
                            Some("thinking") | Some("redacted_thinking")
                        )
                    })
            })
    }
}

/// Map keys the scan record reads, decoded without allocating.
enum ScanField {
    SessionId,
    Cwd,
    Type,
    IsSidechain,
    IsMeta,
    Uuid,
    Message,
    Content,
    Other,
}

impl<'de> serde::Deserialize<'de> for ScanField {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FieldVisitor;
        impl serde::de::Visitor<'_> for FieldVisitor {
            type Value = ScanField;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a field name")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(match value {
                    "sessionId" => ScanField::SessionId,
                    "cwd" => ScanField::Cwd,
                    "type" => ScanField::Type,
                    "isSidechain" => ScanField::IsSidechain,
                    "isMeta" => ScanField::IsMeta,
                    "uuid" => ScanField::Uuid,
                    "message" => ScanField::Message,
                    "content" => ScanField::Content,
                    _ => ScanField::Other,
                })
            }
        }
        deserializer.deserialize_str(FieldVisitor)
    }
}

/// Mirrors `Value::as_str`: any non-string value is consumed and reads as
/// `None`.
struct LenientString(Option<String>);

impl<'de> serde::Deserialize<'de> for LenientString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;
        impl<'de> serde::de::Visitor<'de> for StringVisitor {
            type Value = Option<String>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("any JSON value")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(Some(value.to_string()))
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(None)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(None)
            }
        }
        deserializer
            .deserialize_any(StringVisitor)
            .map(LenientString)
    }
}

/// Mirrors `Value::as_bool`: any non-boolean value is consumed and reads as
/// `None`.
struct LenientBool(Option<bool>);

impl<'de> serde::Deserialize<'de> for LenientBool {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BoolVisitor;
        impl<'de> serde::de::Visitor<'de> for BoolVisitor {
            type Value = Option<bool>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("any JSON value")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(Some(value))
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(None)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(None)
            }
        }
        deserializer.deserialize_any(BoolVisitor).map(LenientBool)
    }
}

/// One content block reduced to `block.get("type").and_then(as_str)`: a
/// non-object element, or one without a string `type`, reads as `None`.
struct ScanBlock(Option<String>);

impl<'de> serde::Deserialize<'de> for ScanBlock {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BlockVisitor;
        impl<'de> serde::de::Visitor<'de> for BlockVisitor {
            type Value = Option<String>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a content block")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut block_type = None;
                while let Some(field) = map.next_key::<ScanField>()? {
                    if matches!(field, ScanField::Type) {
                        block_type = map.next_value::<LenientString>()?.0;
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                Ok(block_type)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(None)
            }
        }
        deserializer.deserialize_any(BlockVisitor).map(ScanBlock)
    }
}

/// `message.content` reduced to block types: `Some` iff it was an array,
/// mirroring `Value::as_array`.
struct ScanContent(Option<Vec<Option<String>>>);

impl<'de> serde::Deserialize<'de> for ScanContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ContentVisitor;
        impl<'de> serde::de::Visitor<'de> for ContentVisitor {
            type Value = Option<Vec<Option<String>>>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("message content")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut block_types = Vec::new();
                while let Some(block) = seq.next_element::<ScanBlock>()? {
                    block_types.push(block.0);
                }
                Ok(Some(block_types))
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(None)
            }
        }
        deserializer
            .deserialize_any(ContentVisitor)
            .map(ScanContent)
    }
}

/// `message` reduced to its content block types; a non-object message reads
/// as no content.
struct ScanMessage(Option<Vec<Option<String>>>);

impl<'de> serde::Deserialize<'de> for ScanMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MessageVisitor;
        impl<'de> serde::de::Visitor<'de> for MessageVisitor {
            type Value = Option<Vec<Option<String>>>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a message object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut content = None;
                while let Some(field) = map.next_key::<ScanField>()? {
                    if matches!(field, ScanField::Content) {
                        content = map.next_value::<ScanContent>()?.0;
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                Ok(content)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(None)
            }
        }
        deserializer
            .deserialize_any(MessageVisitor)
            .map(ScanMessage)
    }
}

impl<'de> serde::Deserialize<'de> for ClaudeScanRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecordVisitor;
        impl<'de> serde::de::Visitor<'de> for RecordVisitor {
            type Value = ClaudeScanRecord;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a transcript record")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut record = ClaudeScanRecord {
                    is_object: true,
                    ..ClaudeScanRecord::default()
                };
                while let Some(field) = map.next_key::<ScanField>()? {
                    match field {
                        ScanField::SessionId => {
                            record.session_id = map.next_value::<LenientString>()?.0;
                        }
                        ScanField::Cwd => record.cwd = map.next_value::<LenientString>()?.0,
                        ScanField::Type => {
                            record.record_type = map.next_value::<LenientString>()?.0;
                        }
                        ScanField::IsSidechain => {
                            record.is_sidechain =
                                map.next_value::<LenientBool>()?.0.unwrap_or(false);
                        }
                        ScanField::IsMeta => {
                            record.is_meta = map.next_value::<LenientBool>()?.0.unwrap_or(false);
                        }
                        ScanField::Uuid => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                            record.has_uuid = true;
                        }
                        ScanField::Message => {
                            record.content_block_types = map.next_value::<ScanMessage>()?.0;
                        }
                        ScanField::Content | ScanField::Other => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(record)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(ClaudeScanRecord::default())
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(ClaudeScanRecord::default())
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(ClaudeScanRecord::default())
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(ClaudeScanRecord::default())
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(ClaudeScanRecord::default())
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(ClaudeScanRecord::default())
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(ClaudeScanRecord::default())
            }
        }
        deserializer.deserialize_any(RecordVisitor)
    }
}

/// Analyze a Claude session transcript. Content failures become bounded
/// reason codes so capture can safely offer StoredOnly instead of blocking a
/// Push of everything else.
pub fn analyze_claude_transcript(bytes: &[u8]) -> SessionAnalysis {
    analyze_claude_records(bytes, crate::conversation_codec::parsed_records(bytes))
}

pub(crate) fn analyze_claude_records(
    bytes: &[u8],
    input: impl Iterator<Item = Result<Value, serde_json::Error>>,
) -> SessionAnalysis {
    crate::sync_metrics::record_transcript_analysis(bytes.len() as u64);
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return unknown(None, None, 0, CodecUnknownReason::Empty);
    }
    let mut record_count = 0_usize;
    let mut identity = ClaudeSessionIdentityTracker::default();
    let mut records: Vec<ChainRecord> = Vec::new();
    let mut occurrences = BTreeMap::<String, Vec<usize>>::new();
    // Every uuid in the projection, chain record or not: the walk uses it to
    // separate a stitch (parent absent from the whole projection — Desktop
    // deleted the row) from a sever (parent exists but never earlier).
    let mut all_uuids = BTreeSet::<String>::new();
    for record in input {
        let value = match record {
            Ok(value) => value,
            Err(_) => {
                let (session_id, cwd) = identity.observed_identity();
                return unknown(
                    session_id,
                    cwd,
                    record_count.saturating_add(1),
                    CodecUnknownReason::MalformedJson,
                );
            }
        };
        // The derivation sees only the projection: a projected-out row
        // contributes nothing — no identity fallback, no count, no uuid.
        let Some(value) = project_claude_record(value) else {
            continue;
        };
        record_count = record_count.saturating_add(1);
        if record_count > MAX_CLAUDE_RECORDS {
            let (session_id, cwd) = identity.observed_identity();
            return unknown(
                session_id,
                cwd,
                record_count,
                CodecUnknownReason::TooManyRecords,
            );
        }
        let Some(object) = value.as_object() else {
            let (session_id, cwd) = identity.observed_identity();
            return unknown(
                session_id,
                cwd,
                record_count,
                CodecUnknownReason::MalformedJson,
            );
        };
        identity.observe(&value);
        if let Some(uuid) = object.get("uuid").and_then(Value::as_str) {
            all_uuids.insert(uuid.to_string());
        }
        let (message_id, parent) = match classify_claude_record(object) {
            Ok(ClaudeRecordClass::NotChain) => continue,
            Ok(ClaudeRecordClass::Chain { message_id, parent }) => (message_id, parent),
            Err(reason) => {
                let (session_id, cwd) = identity.observed_identity();
                return unknown(session_id, cwd, record_count, reason);
            }
        };
        occurrences
            .entry(message_id.as_str().to_string())
            .or_default()
            .push(records.len());
        records.push(ChainRecord {
            message_id,
            parent,
            fingerprint: semantic_record_fingerprint(value),
        });
    }

    if records.is_empty() {
        let (session_id, cwd) = identity.observed_identity();
        return unknown(
            session_id,
            cwd,
            record_count,
            CodecUnknownReason::MissingMessageId,
        );
    }

    // Walk back from the leaf (the last chain record) to a root, resolving
    // each parent reference to its latest occurrence at an earlier position.
    // Resolution, stitching, and severing are all strictly
    // position-decreasing or terminal, so the walk terminates without cycle
    // bookkeeping.
    let mut path = Vec::new();
    let mut position = records.len() - 1;
    loop {
        path.push(position);
        let Some(parent) = &records[position].parent else {
            break;
        };
        let resolved = occurrences
            .get(parent.as_str())
            .map(|positions| positions.partition_point(|&occurrence| occurrence < position))
            .filter(|&index| index > 0)
            .map(|index| occurrences[parent.as_str()][index - 1]);
        position = match resolved {
            Some(resolved) => resolved,
            // A parent absent from the entire projection is a row Desktop's
            // import deleted: stitch to the nearest preceding chain record
            // (P2), or root here when none precedes. A parent that exists
            // only at later positions is damage the projection cannot
            // express: sever into a root here (P3).
            None if !all_uuids.contains(parent.as_str()) && position > 0 => position - 1,
            None => break,
        };
    }
    path.reverse();

    // The leaf selects the active session ID. Resume-copied history may carry
    // older IDs, while cwd changes within the active ID are only shell state;
    // the tracker therefore pairs that ID with its segment's launch cwd.
    let (session_id, source_cwd) = identity.leaf_identity();
    if session_id.is_none() {
        return unknown(
            None,
            source_cwd,
            record_count,
            CodecUnknownReason::MissingSessionMetadata,
        );
    }

    // Canonical chains record normalized parents: each record's parent is
    // the previous chain record and the root's is `None`. For an ordinarily
    // resolved link the normalized parent equals the raw one; a stitched or
    // severed link records the effective parent the walk actually took, so
    // both sides of an import derive the identical chain.
    let chain: Vec<CanonicalConversationMessage> = path
        .iter()
        .enumerate()
        .map(|(index, &position)| CanonicalConversationMessage {
            message_id: records[position].message_id.clone(),
            parent_message_id: index
                .checked_sub(1)
                .map(|previous| records[path[previous]].message_id.clone()),
            semantic_fingerprint: records[position].fingerprint.clone(),
        })
        .collect();
    if let Err(error) = validate_conversation_chain(&chain) {
        // Linkage holds by construction; only a uuid re-visited by the walk
        // (its own ancestry re-references it) can fail here.
        let reason = if error.contains("duplicate") {
            CodecUnknownReason::DuplicateMessageId
        } else {
            CodecUnknownReason::NonLinearParentage
        };
        return unknown(session_id, source_cwd, record_count, reason);
    }
    let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain)
        .expect("validated conversation chains always produce digests");
    SessionAnalysis {
        source_provider_session_id: session_id,
        source_cwd,
        is_internal_subagent: false,
        record_count,
        lineage: CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        },
    }
}

/// Main-chain records: durable typed records with lineage identity, including
/// Claude's attachment records that bridge message ancestry. Sidechain
/// (subagent) branches root their own parentage and remain excluded.
pub(crate) fn is_chain_record(object: &Map<String, Value>) -> bool {
    let record_type = object.get("type").and_then(Value::as_str);
    let durable = matches!(
        record_type,
        Some("user") | Some("assistant") | Some("system") | Some("attachment")
    );
    let sidechain = object
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    durable && !sidechain && object.contains_key("uuid")
}

fn unknown(
    session_id: Option<String>,
    source_cwd: Option<String>,
    record_count: usize,
    reason: CodecUnknownReason,
) -> SessionAnalysis {
    SessionAnalysis {
        source_provider_session_id: session_id,
        source_cwd,
        is_internal_subagent: false,
        record_count,
        lineage: CodecLineage::Unknown { reason },
    }
}

/// The semantic fingerprint excludes exactly the rewritten structural fields
/// (`sessionId`, `cwd`), lineage identity, and envelope stamps, so a
/// structural rewrite provably cannot change semantics. Parentage itself is
/// bound by the chain fold, not the fingerprint. Callers hand it the
/// *projected* record (`project_claude_record`), so thinking blocks never
/// reach a digest; parse-then-canonicalize (sorted keys) already keeps
/// Desktop's re-serialization noise out.
pub(crate) fn semantic_record_fingerprint(mut value: Value) -> String {
    if let Some(object) = value.as_object_mut() {
        for key in [
            "sessionId",
            "cwd",
            "uuid",
            "parentUuid",
            "timestamp",
            "version",
            "requestId",
        ] {
            object.remove(key);
        }
    }
    sha256_hex(&canonical_json(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_semantic_fingerprint_preserves_fixture_hash() {
        let value = json!({
            "sessionId": "fixture-session", "cwd": "/tmp/fixture-project",
            "uuid": "message-id", "parentUuid": "message-parent",
            "timestamp": "2026-09-04T10:00:00Z", "version": "fixture-version",
            "requestId": "fixture-request", "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Quote: \"; slash: \\; line:\n雪"}],
                "data": {"z": [null, true, false, -0.0, 1.5], "a": {"z": 2, "a": 1}}
            }
        });
        assert_eq!(
            semantic_record_fingerprint(value),
            "888a1d8f4d60c790c9a83a23e1b452432f1ba00b4c6348b0635c6285dc0fdc13"
        );
    }

    #[test]
    fn scan_record_identity_parts_match_the_value_extraction() {
        let cases = [
            json!({"type":"user","uuid":"u1","sessionId":"s","cwd":"/p"}),
            json!({"type":"assistant","uuid":"a1","sessionId":"s","cwd":"/p",
                "message":{"content":[{"type":"thinking","thinking":"x"}]}}),
            json!({"type":"assistant","uuid":"a1",
                "message":{"content":[{"type":"thinking"},{"type":"redacted_thinking"}]}}),
            json!({"type":"assistant","uuid":"a1",
                "message":{"content":[{"type":"thinking"},{"type":"text","text":"t"}]}}),
            json!({"type":"assistant","uuid":"a1","message":{"content":[]}}),
            json!({"type":"assistant","uuid":"a1","message":{"content":"plain"}}),
            json!({"type":"assistant","uuid":"a1","message":{"content":[["nested"]]}}),
            json!({"type":"assistant","uuid":"a1","message":{"content":[{"type":5}]}}),
            json!({"type":"assistant","uuid":"a1","message":"not-an-object"}),
            json!({"type":"user","uuid":null,"sessionId":"s","cwd":"/p"}),
            json!({"type":"user","sessionId":"s","cwd":"/p"}),
            json!({"type":"user","uuid":"u1","isSidechain":true,"sessionId":"s","cwd":"/p"}),
            json!({"type":"user","uuid":"u1","isMeta":true,"sessionId":"s","cwd":"/p"}),
            json!({"type":"attachment","uuid":"at1","cwd":"/p"}),
            json!({"type":"summary","sessionId":"s","cwd":"/p"}),
            json!({"type":"user","uuid":"u1","sessionId":123,"cwd":42}),
            json!({"type":"user","uuid":"u1","isSidechain":"true","sessionId":"s","cwd":"/p"}),
            json!({"type":"user","uuid":"u1","sessionId":"s","cwd":""}),
            json!(["not", "an", "object"]),
            json!("bare string"),
            json!(12),
            json!(null),
            json!({}),
            json!({"message":{"payload":{"sessionId":"nested"}}}),
        ];
        for value in cases {
            let line = value.to_string();
            let from_value = identity_parts_from_value(&value);
            let from_scan = serde_json::from_str::<ClaudeScanRecord>(&line)
                .expect("the scan record consumes any JSON value")
                .into_identity_parts();
            assert_eq!(from_value, from_scan, "diverged on {line}");
        }
    }

    #[test]
    fn leaf_identity_never_homes_by_drift() {
        let mut tracker = ClaudeSessionIdentityTracker::default();
        tracker.observe(&json!({"type":"user","uuid":"u1","sessionId":"s1","cwd":"/home/a"}));
        tracker
            .observe(&json!({"type":"assistant","uuid":"a1","sessionId":"s1","cwd":"/home/a/sub"}));
        assert_eq!(
            tracker.leaf_identity(),
            (Some("s1".to_string()), Some("/home/a".to_string()))
        );
    }

    #[test]
    fn leaf_identity_fails_closed_when_no_authorized_home_exists() {
        // A chain record without its own sessionId cannot route ownership
        // through file-wide last-seen state: the session ID still resolves,
        // but the home reads as unproven instead of borrowing drift.
        let mut tracker = ClaudeSessionIdentityTracker::default();
        tracker.observe(&json!({"type":"user","uuid":"u1","cwd":"/home/x"}));
        tracker.observe(&json!({"type":"diagnostic","sessionId":"s2"}));
        let (session_id, authorized_home) = tracker.leaf_identity();
        assert_eq!(session_id.as_deref(), Some("s2"));
        assert_eq!(authorized_home, None);
    }

    #[test]
    fn chainless_files_resolve_the_first_metadata_identity() {
        let mut tracker = ClaudeSessionIdentityTracker::default();
        tracker.observe(&json!({"type":"user","sessionId":"first","cwd":"/home/first"}));
        tracker.observe(&json!({"type":"user","sessionId":"later","cwd":"/home/later"}));
        assert_eq!(
            tracker.leaf_identity(),
            (Some("first".to_string()), Some("/home/first".to_string()))
        );
    }

    pub(crate) fn transcript(
        session: &str,
        cwd: &str,
        messages: &[(&str, Option<&str>, &str, &str)],
    ) -> Vec<u8> {
        let mut lines = Vec::new();
        for (uuid, parent, role, text) in messages {
            lines.push(json!({
                "type": role,
                "uuid": uuid,
                "parentUuid": parent.map(str::to_string),
                "sessionId": session,
                "cwd": cwd,
                "timestamp": "2026-08-07T00:00:00.000Z",
                "version": "2.0.0",
                "message": {"role": role, "content": [{"type": "text", "text": text}]}
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

    fn chain_ids(analysis: &SessionAnalysis) -> Vec<&str> {
        analysis
            .lineage
            .chain()
            .unwrap()
            .iter()
            .map(|message| message.message_id.as_str())
            .collect()
    }

    fn lineage_digests(analysis: &SessionAnalysis) -> Option<(&str, &str)> {
        match &analysis.lineage {
            CodecLineage::Complete {
                lineage_digest,
                semantic_digest,
                ..
            } => Some((lineage_digest, semantic_digest)),
            CodecLineage::Unknown { .. } => None,
        }
    }

    #[test]
    fn claude_codec_extracts_full_linear_chain() {
        let bytes = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/project-a",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-1"), "assistant", "two"),
            ],
        );
        let analysis = analyze_claude_transcript(&bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(
            analysis.source_provider_session_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(analysis.source_cwd.as_deref(), Some("/tmp/project-a"));
        assert_eq!(analysis.lineage.chain().unwrap().len(), 2);
        assert_eq!(CLAUDE_LINEAGE_CODEC_KEY, "claude.lineage.v1");
        assert_eq!(CLAUDE_CODEC_VERSION, 1);
        let CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } = &analysis.lineage
        else {
            unreachable!("resumable analysis has complete lineage")
        };
        // Pinned: the branch-tolerant revision must not change linear-session
        // digests.
        assert_eq!(
            lineage_digest,
            "b24af79ca3ab0e0d79c1e0752179d8dca439f27f1f84cc639fbf9923454fa223"
        );
        assert_eq!(
            semantic_digest,
            "93e3295256920d8f80436aef591fb12508758c2f04ff3b1f132034cafa6efd02"
        );
    }

    #[test]
    fn claude_codec_includes_attachment_records_in_linear_chain() {
        let session = "11111111-1111-4111-8111-111111111111";
        let cwd = "/tmp/project-a";
        let records = [
            json!({
                "type": "attachment",
                "uuid": "attachment-hook",
                "parentUuid": null,
                "sessionId": session,
                "cwd": cwd,
                "attachment": {"type": "hook_success", "name": "SessionStart"}
            }),
            json!({
                "type": "user",
                "uuid": "user-1",
                "parentUuid": "attachment-hook",
                "sessionId": session,
                "cwd": cwd,
                "message": {"role": "user", "content": "draw a bird"}
            }),
            json!({
                "type": "attachment",
                "uuid": "attachment-tools",
                "parentUuid": "user-1",
                "sessionId": session,
                "cwd": cwd,
                "attachment": {"type": "deferred_tools_delta", "enabled": ["Read"]}
            }),
            json!({
                "type": "attachment",
                "uuid": "attachment-skills",
                "parentUuid": "attachment-tools",
                "sessionId": session,
                "cwd": cwd,
                "attachment": {"type": "skill_listing", "skills": ["example"]}
            }),
            json!({
                "type": "assistant",
                "uuid": "assistant-1",
                "parentUuid": "attachment-skills",
                "sessionId": session,
                "cwd": cwd,
                "message": {"role": "assistant", "content": "done"}
            }),
            json!({
                "type": "attachment",
                "uuid": "sidechain-attachment",
                "parentUuid": null,
                "isSidechain": true,
                "sessionId": session,
                "cwd": cwd,
                "attachment": {"type": "agent_listing_delta"}
            }),
        ];
        let mut bytes = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');

        let before = analyze_claude_transcript(&bytes);
        assert!(before.is_resumable(), "{:?}", before.lineage);
        assert_eq!(before.record_count, 6);
        assert_eq!(
            chain_ids(&before),
            [
                "attachment-hook",
                "user-1",
                "attachment-tools",
                "attachment-skills",
                "assistant-1",
            ]
        );
    }

    #[test]
    fn claude_codec_derives_the_active_branch_after_a_rewind() {
        // u-1 -> a-1 was answered, the user rewound and re-asked from u-1;
        // the abandoned a-1 branch stays in the file but off the chain.
        let bytes = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-1"), "assistant", "abandoned"),
                ("u-2", Some("u-1"), "user", "re-asked"),
                ("a-2", Some("u-2"), "assistant", "active"),
            ],
        );
        let analysis = analyze_claude_transcript(&bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(analysis.record_count, 4);
        assert_eq!(chain_ids(&analysis), ["u-1", "u-2", "a-2"]);

        // The abandoned branch does not participate in the digests: the same
        // active path without it produces the identical identity.
        let without_branch = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("u-2", Some("u-1"), "user", "re-asked"),
                ("a-2", Some("u-2"), "assistant", "active"),
            ],
        );
        assert_eq!(
            lineage_digests(&analysis),
            lineage_digests(&analyze_claude_transcript(&without_branch))
        );
    }

    #[test]
    fn claude_codec_starts_the_chain_at_a_mid_file_root() {
        // Compaction appends a fresh null-parent root; the chain is the
        // post-compaction segment and earlier records become history.
        let bytes = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "old one"),
                ("a-1", Some("u-1"), "assistant", "old two"),
                ("compact-1", None, "user", "compacted summary"),
                ("a-2", Some("compact-1"), "assistant", "fresh"),
            ],
        );
        let analysis = analyze_claude_transcript(&bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(chain_ids(&analysis), ["compact-1", "a-2"]);
    }

    #[test]
    fn claude_codec_resolves_re_emitted_uuids_to_the_latest_occurrence() {
        // a-1 is re-emitted later under a different parent; the walk from the
        // leaf resolves to the re-emission and the original occurrence (plus
        // its stranded descendant) is shadowed off the chain.
        let bytes = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-1"), "assistant", "original"),
                ("u-2", Some("a-1"), "user", "stranded"),
                ("a-1", Some("u-1"), "assistant", "re-emitted"),
                ("u-3", Some("a-1"), "user", "active"),
            ],
        );
        let analysis = analyze_claude_transcript(&bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(analysis.record_count, 5);
        assert_eq!(chain_ids(&analysis), ["u-1", "a-1", "u-3"]);
        let CodecLineage::Complete { chain, .. } = &analysis.lineage else {
            unreachable!()
        };
        // The re-emission's fingerprint, not the original's, is on the chain.
        let re_emitted = semantic_record_fingerprint(json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "re-emitted"}]}
        }));
        assert_eq!(chain[1].semantic_fingerprint, re_emitted);
    }

    #[test]
    fn claude_codec_takes_the_leaf_identity_over_copied_history() {
        // Resume-copied history keeps its original sessionId and cwd. The new
        // session ID wins, while later cwd changes inside that ID do not move
        // its authoritative launch home.
        let mut mixed = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[("u-1", None, "user", "one")],
        );
        mixed.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "assistant",
                "uuid": "a-1",
                "parentUuid": "u-1",
                "sessionId": "22222222-2222-4222-8222-222222222222",
                "cwd": "/tmp/q",
                "message": {"role": "assistant", "content": []}
            }))
            .unwrap()
            .as_bytes(),
        );
        mixed.push(b'\n');
        mixed.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "user",
                "uuid": "u-2",
                "parentUuid": "a-1",
                "sessionId": "22222222-2222-4222-8222-222222222222",
                "cwd": "/tmp/q/src-tauri",
                "message": {"role": "user", "content": "continue"}
            }))
            .unwrap()
            .as_bytes(),
        );
        mixed.push(b'\n');
        let analysis = analyze_claude_transcript(&mixed);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(
            analysis.source_provider_session_id.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
        assert_eq!(analysis.source_cwd.as_deref(), Some("/tmp/q"));
        assert_eq!(chain_ids(&analysis), ["u-1", "a-1", "u-2"]);
    }

    #[test]
    fn claude_codec_tolerates_forward_references_off_the_active_path() {
        // a-1 references u-2 before it exists; the walk from the leaf never
        // visits it, so the file stays Complete with a-1 trimmed.
        let bytes = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-2"), "assistant", "forward"),
                ("u-2", Some("u-1"), "user", "later"),
            ],
        );
        let analysis = analyze_claude_transcript(&bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(chain_ids(&analysis), ["u-1", "u-2"]);
    }

    #[test]
    fn claude_codec_rejects_duplicates_and_malformed_records() {
        // A uuid the walk visits twice (its own ancestry re-references it)
        // fails closed as a duplicate.
        let duplicate = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("u-1", Some("u-1"), "user", "different"),
            ],
        );
        assert!(matches!(
            analyze_claude_transcript(&duplicate).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::DuplicateMessageId
            }
        ));

        assert!(matches!(
            analyze_claude_transcript(b"\n").lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::Empty
            }
        ));
        assert!(matches!(
            analyze_claude_transcript(b"{not json}\n").lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::MalformedJson
            }
        ));

        // A non-string parentUuid on a chain record is malformed even when
        // the record is off the eventual path: classification fails closed
        // before the walk.
        let bad_parent = serde_json::to_vec(&json!({
            "type": "user",
            "uuid": "u-1",
            "parentUuid": 7,
            "sessionId": "11111111-1111-4111-8111-111111111111",
            "message": {"role": "user", "content": "x"}
        }))
        .unwrap();
        let mut bad = bad_parent.clone();
        bad.push(b'\n');
        assert!(matches!(
            analyze_claude_transcript(&bad).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::MalformedJson
            }
        ));
    }

    fn record_with_content(uuid: &str, parent: Option<&str>, role: &str, content: Value) -> Value {
        json!({
            "type": role,
            "uuid": uuid,
            "parentUuid": parent.map(str::to_string),
            "sessionId": "11111111-1111-4111-8111-111111111111",
            "cwd": "/tmp/p",
            "timestamp": "2026-08-20T00:00:00.000Z",
            "version": "2.0.0",
            "message": {"role": role, "content": content}
        })
    }

    fn encode_records(records: &[Value]) -> Vec<u8> {
        let mut bytes = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');
        bytes
    }

    /// Desktop's import rewrite applied to whole transcript bytes: the
    /// projection of every row, re-serialized. Serialization noise differs
    /// from the real Desktop (key order, escaping), which is exactly what
    /// parse-then-canonicalize fingerprints are proven against.
    pub(crate) fn desktop_import(bytes: &[u8]) -> Vec<u8> {
        let mut lines = Vec::new();
        for raw in bytes.split(|byte| *byte == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_slice(raw).unwrap();
            if let Some(projected) = project_claude_record(value) {
                lines.push(serde_json::to_string(&projected).unwrap());
            }
        }
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn projection_predicate_matches_the_record_drop() {
        let cases = [
            // (record, projected out?)
            (
                record_with_content(
                    "t-1",
                    None,
                    "assistant",
                    json!([{"type": "thinking", "thinking": "plan", "signature": "s"}]),
                ),
                true,
            ),
            (
                record_with_content(
                    "t-2",
                    None,
                    "assistant",
                    json!([{"type": "redacted_thinking", "data": "opaque"}]),
                ),
                true,
            ),
            (
                record_with_content(
                    "a-1",
                    None,
                    "assistant",
                    json!([{"type": "thinking", "thinking": "plan"}, {"type": "text", "text": "kept"}]),
                ),
                false,
            ),
            (
                record_with_content(
                    "a-2",
                    None,
                    "assistant",
                    json!([{"type": "text", "text": "t"}]),
                ),
                false,
            ),
            // Already-empty content has no thinking to strip: untouched.
            (
                record_with_content("a-3", None, "assistant", json!([])),
                false,
            ),
            (
                record_with_content("a-4", None, "assistant", json!("string content")),
                false,
            ),
            // Only assistant rows are sanitized, whatever their content.
            (
                record_with_content(
                    "u-1",
                    None,
                    "user",
                    json!([{"type": "thinking", "thinking": "not stripped"}]),
                ),
                false,
            ),
        ];
        for (record, projected_out) in cases {
            let object = record.as_object().unwrap();
            assert_eq!(
                is_thinking_only_record(object),
                projected_out,
                "predicate for {record}"
            );
            assert_eq!(
                project_claude_record(record.clone()).is_none(),
                projected_out,
                "projection for {record}"
            );
        }
        // A sidechain assistant row is sanitized too: Desktop keys on the
        // row type alone.
        let mut sidechain = record_with_content(
            "t-3",
            None,
            "assistant",
            json!([{"type": "thinking", "thinking": "subagent plan"}]),
        );
        sidechain["isSidechain"] = json!(true);
        assert!(is_thinking_only_record(sidechain.as_object().unwrap()));
        assert!(project_claude_record(sidechain).is_none());
        // A mixed row projects to its surviving blocks.
        let mixed = record_with_content(
            "a-5",
            None,
            "assistant",
            json!([{"type": "thinking", "thinking": "plan"}, {"type": "text", "text": "kept"}]),
        );
        let projected = project_claude_record(mixed).unwrap();
        assert_eq!(
            projected["message"]["content"],
            json!([{"type": "text", "text": "kept"}])
        );
    }

    #[test]
    fn claude_codec_stitches_dangling_parents_instead_of_failing_closed() {
        // The incident shape: a child whose parent uuid is absent from the
        // whole file (Desktop deleted the row) stitches to the nearest
        // preceding chain record and normalizes its parent.
        let stitched = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("ghost"), "assistant", "active"),
            ],
        );
        let analysis = analyze_claude_transcript(&stitched);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(chain_ids(&analysis), ["u-1", "a-1"]);
        let CodecLineage::Complete { chain, .. } = &analysis.lineage else {
            unreachable!()
        };
        assert_eq!(chain[1].parent_message_id.as_ref().unwrap().as_str(), "u-1");

        // Normalized parents make the stitched file identical to its
        // plainly linked twin.
        let plain = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-1"), "assistant", "active"),
            ],
        );
        assert_eq!(
            lineage_digests(&analysis),
            lineage_digests(&analyze_claude_transcript(&plain))
        );
    }

    #[test]
    fn claude_codec_projection_makes_a_desktop_import_invisible() {
        let records = [
            record_with_content(
                "u-1",
                None,
                "user",
                json!([{"type": "text", "text": "ask"}]),
            ),
            record_with_content(
                "t-1",
                Some("u-1"),
                "assistant",
                json!([{"type": "thinking", "thinking": "hidden plan", "signature": "sig"}]),
            ),
            record_with_content(
                "a-1",
                Some("t-1"),
                "assistant",
                json!([
                    {"type": "redacted_thinking", "data": "opaque"},
                    {"type": "text", "text": "visible answer"}
                ]),
            ),
            record_with_content(
                "u-2",
                Some("a-1"),
                "user",
                json!([{"type": "text", "text": "follow-up"}]),
            ),
        ];
        let original = encode_records(&records);
        let analysis = analyze_claude_transcript(&original);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(analysis.record_count, 3, "the projected count");
        assert_eq!(chain_ids(&analysis), ["u-1", "a-1", "u-2"]);

        // The import twin: thinking-only row deleted, mixed row stripped,
        // children's parents left dangling. Idempotent, and equivalent.
        let imported = desktop_import(&original);
        assert_ne!(original, imported);
        assert_eq!(desktop_import(&imported), imported, "idempotent rewrite");
        let after = analyze_claude_transcript(&imported);
        assert!(after.is_resumable(), "{:?}", after.lineage);
        assert_eq!(after.record_count, 3);
        assert_eq!(chain_ids(&after), ["u-1", "a-1", "u-2"]);
        assert_eq!(lineage_digests(&analysis), lineage_digests(&after));
        assert_eq!(
            analysis.source_provider_session_id,
            after.source_provider_session_id
        );
        assert_eq!(analysis.source_cwd, after.source_cwd);

        // Both equal the conversation written plainly with no thinking at
        // all: thinking is invisible to sync comparison, by definition.
        let plain = encode_records(&[
            record_with_content(
                "u-1",
                None,
                "user",
                json!([{"type": "text", "text": "ask"}]),
            ),
            record_with_content(
                "a-1",
                Some("u-1"),
                "assistant",
                json!([{"type": "text", "text": "visible answer"}]),
            ),
            record_with_content(
                "u-2",
                Some("a-1"),
                "user",
                json!([{"type": "text", "text": "follow-up"}]),
            ),
        ]);
        assert_eq!(
            lineage_digests(&analysis),
            lineage_digests(&analyze_claude_transcript(&plain))
        );

        // Pinned: the projection revision's stitch digests.
        let CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } = &analysis.lineage
        else {
            unreachable!()
        };
        assert_eq!(
            lineage_digest,
            "440adfc0a375a9f5159a7aa4fad792d06ea96e4b0fca44eab4fbe4be405ad5c7"
        );
        assert_eq!(
            semantic_digest,
            "cadc7569e90c62dac5db3c1345b9235940096611ad2db9683b97518587dd339a"
        );
    }

    #[test]
    fn claude_codec_stitches_a_rewind_onto_a_deleted_thinking_record() {
        // The shape that killed link-transparency (103/106): a rewind whose
        // target is a thinking-only record. Deleting it destroys the branch
        // attachment; the projection stitches identically on both forms —
        // folding in the abandoned a-1 — so equivalence still holds.
        let records = [
            record_with_content(
                "u-1",
                None,
                "user",
                json!([{"type": "text", "text": "ask"}]),
            ),
            record_with_content(
                "t-1",
                Some("u-1"),
                "assistant",
                json!([{"type": "thinking", "thinking": "branching point"}]),
            ),
            record_with_content(
                "a-1",
                Some("t-1"),
                "assistant",
                json!([{"type": "text", "text": "abandoned"}]),
            ),
            record_with_content(
                "u-2",
                Some("t-1"),
                "user",
                json!([{"type": "text", "text": "re-asked"}]),
            ),
            record_with_content(
                "a-2",
                Some("u-2"),
                "assistant",
                json!([{"type": "text", "text": "active"}]),
            ),
        ];
        let original = encode_records(&records);
        let analysis = analyze_claude_transcript(&original);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(chain_ids(&analysis), ["u-1", "a-1", "u-2", "a-2"]);
        let after = analyze_claude_transcript(&desktop_import(&original));
        assert_eq!(chain_ids(&after), ["u-1", "a-1", "u-2", "a-2"]);
        assert_eq!(lineage_digests(&analysis), lineage_digests(&after));
    }

    #[test]
    fn claude_codec_severs_forward_only_parents_and_roots_stitch_floors() {
        // A parent that exists only at a later position severs: the walk
        // roots at the referencing record instead of guessing.
        let forward_only = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("m-1", Some("f-1"), "user", "early"),
                ("f-1", None, "user", "late"),
                ("l-1", Some("m-1"), "assistant", "leaf"),
            ],
        );
        let analysis = analyze_claude_transcript(&forward_only);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(chain_ids(&analysis), ["m-1", "l-1"]);
        let CodecLineage::Complete { chain, .. } = &analysis.lineage else {
            unreachable!()
        };
        assert_eq!(chain[0].parent_message_id, None, "severed root normalizes");

        // A stitch with no preceding chain record floors into a root.
        let floor = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[("x-1", Some("ghost"), "user", "only")],
        );
        let analysis = analyze_claude_transcript(&floor);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(chain_ids(&analysis), ["x-1"]);

        // A parent pointing at a sidechain row's uuid is present in the
        // projection (severs), not absent (stitches): all_uuids spans every
        // projected row, chain record or not.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "user",
                "uuid": "s-1",
                "parentUuid": null,
                "isSidechain": true,
                "sessionId": "11111111-1111-4111-8111-111111111111",
                "cwd": "/tmp/p",
                "message": {"role": "user", "content": "subagent prompt"}
            }))
            .unwrap()
            .as_bytes(),
        );
        bytes.push(b'\n');
        bytes.extend_from_slice(&transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("c-0", None, "user", "before"),
                ("c-1", Some("s-1"), "user", "into the sidechain"),
            ],
        ));
        let analysis = analyze_claude_transcript(&bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(
            chain_ids(&analysis),
            ["c-1"],
            "severed, not stitched to c-0"
        );
    }

    #[test]
    fn claude_codec_skips_sidechains_and_non_chain_records() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "summary",
                "summary": "A summary",
                "leafUuid": "a-1"
            }))
            .unwrap()
            .as_bytes(),
        );
        bytes.push(b'\n');
        bytes.extend_from_slice(&transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/project-a",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-1"), "assistant", "two"),
            ],
        ));
        bytes.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "user",
                "uuid": "side-1",
                "parentUuid": null,
                "isSidechain": true,
                "sessionId": "11111111-1111-4111-8111-111111111111",
                "cwd": "/tmp/project-a",
                "message": {"role": "user", "content": "subagent prompt"}
            }))
            .unwrap()
            .as_bytes(),
        );
        bytes.push(b'\n');

        let analysis = analyze_claude_transcript(&bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(analysis.lineage.chain().unwrap().len(), 2);
        assert_eq!(analysis.record_count, 4);
    }

    /// Manual corpus gate: the shipped Rust analyzer must hold the
    /// import-equivalence property
    /// on every real transcript and report the outcome counts.
    /// Ignored in CI — it reads the developer's `~/.claude/projects`. Run:
    /// `cargo test -p mallard-infrastructure --lib -- --ignored --nocapture
    /// claude_corpus_import_equivalence`.
    #[test]
    #[ignore = "manual corpus gate: reads ~/.claude/projects"]
    fn claude_corpus_import_equivalence() {
        let Some(home) = std::env::var_os("HOME") else {
            eprintln!("corpus gate skipped: no HOME");
            return;
        };
        let root = std::path::Path::new(&home).join(".claude/projects");
        if !root.is_dir() {
            eprintln!("corpus gate skipped: no corpus at {}", root.display());
            return;
        }
        let mut pending = vec![root];
        let mut files = 0_usize;
        let mut complete = 0_usize;
        let mut equivalent = 0_usize;
        let mut chain_records = 0_usize;
        let mut mismatches: Vec<String> = Vec::new();
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                files += 1;
                let analysis = analyze_claude_transcript(&bytes);
                if !analysis.is_resumable() {
                    continue;
                }
                complete += 1;
                let chain = analysis.lineage.chain().unwrap();
                chain_records += chain.len();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let imported = analyze_claude_transcript(&desktop_import(&bytes));
                let same = lineage_digests(&analysis) == lineage_digests(&imported)
                    && analysis.record_count == imported.record_count
                    && analysis.source_provider_session_id == imported.source_provider_session_id;
                if same {
                    equivalent += 1;
                } else {
                    mismatches.push(name.chars().take(8).collect());
                }
                if name.starts_with("54894825") || name.starts_with("9d825d28") {
                    eprintln!(
                        "{}: records={} chain_len={} leaf={}",
                        &name[..8],
                        analysis.record_count,
                        chain.len(),
                        chain.last().unwrap().message_id.as_str(),
                    );
                }
            }
        }
        eprintln!(
            "corpus: files={files} complete={complete} chain_records={chain_records} \
             equivalent={equivalent}"
        );
        assert_eq!(mismatches, Vec::<String>::new(), "import-equivalence holds");
        assert_eq!(equivalent, complete);
        assert!(complete > 0, "the corpus gate must see real sessions");
    }

    /// A committed two-root fixture whose prose, tool input, tool result,
    /// attachment filename, and file-history snapshot all embed the source
    /// root must leave the wire hop with the source spelling byte-absent.
    /// Content paths travel with the structural `cwd`, per line, under the
    /// shared mapper's gates.
    #[test]
    fn claude_wire_fixture_remaps_content_paths_and_round_trips() {
        use crate::session_path_map::SessionPathMapper;

        let source_root = "/homeA/projact_alpha";
        let session = "11111111-1111-4111-8111-111111111111";
        let records = [
            json!({
                "type": "user",
                "uuid": "u-1",
                "parentUuid": null,
                "sessionId": session,
                "cwd": source_root,
                "message": {"role": "user", "content": [
                    {"type": "text", "text": concat!(
                        "edit /homeA/projact_alpha/readme please; ",
                        "literal /__mallard_root__/readme; ",
                        "escape /__mallard_escape__/r"
                    )}
                ]}
            }),
            json!({
                "type": "assistant",
                "uuid": "a-1",
                "parentUuid": "u-1",
                "sessionId": session,
                "cwd": source_root,
                "message": {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t-1", "name": "Write",
                     "input": {"file_path": "/homeA/projact_alpha/readme", "content": "hi"}}
                ]}
            }),
            json!({
                "type": "user",
                "uuid": "u-2",
                "parentUuid": "a-1",
                "sessionId": session,
                "cwd": format!("{source_root}/frontend"),
                "toolUseResult": {"filePath": "/homeA/projact_alpha/readme"},
                "message": {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t-1", "content": "ok"}
                ]}
            }),
            json!({
                "type": "attachment",
                "uuid": "att-1",
                "parentUuid": "u-2",
                "sessionId": session,
                "cwd": source_root,
                "attachment": {"type": "file", "filename": "/homeA/projact_alpha/notes.md"}
            }),
            json!({
                "type": "file-history-snapshot",
                "messageId": "u-1",
                "snapshot": {"path": "/homeA/projact_alpha/file.txt",
                             "outside": "/homeA/other-project/keep.txt"}
            }),
        ];
        let source = encode_records(&records);
        let before = analyze_claude_transcript(&source);
        assert!(before.is_resumable(), "{:?}", before.lineage);

        // Push half: encode per line under the source mapper.
        let encoder = SessionPathMapper::new(source_root, None, Some("/homeA")).unwrap();
        let wire_text = String::from_utf8(source.clone())
            .unwrap()
            .lines()
            .map(|line| encoder.encode_jsonl_line_gated(line).unwrap() + "\n")
            .collect::<String>();
        assert!(!wire_text.contains(source_root));
        assert!(wire_text.contains("\"cwd\":\"/__mallard_root__/b;/frontend\""));
        assert!(wire_text.contains("literal /__mallard_escape__/r/readme"));
        assert!(wire_text.contains("escape /__mallard_escape__/e/r"));
        // The wire form analyzes Complete with the chain identities intact.
        let wire_analysis = analyze_claude_transcript(wire_text.as_bytes());
        assert_eq!(chain_ids(&wire_analysis), chain_ids(&before));

        // Pull half: verify + decode per line under the destination mapper.
        let decoder = SessionPathMapper::new("/homeB/projact_beta", None, Some("/homeB")).unwrap();
        let decoded_text = wire_text
            .lines()
            .map(|line| {
                let value = decoder.decode_jsonl_line_verified(line).unwrap();
                serde_json::to_string(&value).unwrap() + "\n"
            })
            .collect::<String>();
        assert!(
            !decoded_text.contains(source_root),
            "source-root spellings must not survive the hop"
        );
        assert!(decoded_text.contains(concat!(
            "edit /homeB/projact_beta/readme please; ",
            "literal /__mallard_root__/readme; ",
            "escape /__mallard_escape__/r"
        )));
        assert!(decoded_text.contains("\"file_path\":\"/homeB/projact_beta/readme\""));
        assert!(decoded_text.contains("\"filePath\":\"/homeB/projact_beta/readme\""));
        assert!(decoded_text.contains("\"filename\":\"/homeB/projact_beta/notes.md\""));
        assert!(decoded_text.contains("\"path\":\"/homeB/projact_beta/file.txt\""));
        // Out-of-root paths ship byte-identical — the hard invariant.
        assert!(decoded_text.contains("/homeA/other-project/keep.txt"));
        let after = analyze_claude_transcript(decoded_text.as_bytes());
        assert!(after.is_resumable(), "{:?}", after.lineage);
        assert_eq!(chain_ids(&after), chain_ids(&before));
        // A bound session's leaf cwd is the project root (a subdir launch is
        // its own project); mid-file subdir spellings remap as content.
        assert_eq!(after.source_cwd.as_deref(), Some("/homeB/projact_beta"));
    }
}
