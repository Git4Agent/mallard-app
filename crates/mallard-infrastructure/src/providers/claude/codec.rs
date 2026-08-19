//! Bounded, versioned Claude Code transcript codec.
//!
//! Claude sessions are JSONL files whose durable records (`user`,
//! `assistant`, `system`, and `attachment`) carry `uuid`/`parentUuid`
//! lineage plus a `sessionId` and `cwd` on every record. The codec derives
//! one linear canonical chain from the main (non-sidechain) records, returns
//! typed `Unknown` reasons instead of guessing, and rewrites only the
//! declared structural `sessionId` and `cwd` fields — round-tripping the
//! result through this same parser and proving unchanged lineage and
//! semantic digests before bytes may become a resumable provider transcript.

use crate::conversation_codec::{CodecLineage, CodecUnknownReason};
use crate::providers::traits::SessionAnalysis;
use mallard_domain::{
    conversation_chain_digests, validate_conversation_chain, CanonicalConversationMessage,
    MessageId,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const CLAUDE_LINEAGE_CODEC_KEY: &str = "claude.lineage.v1";
pub const CLAUDE_CODEC_VERSION: u32 = 1;

const MAX_CLAUDE_RECORDS: usize = 100_000;

/// Analyze a Claude session transcript. Content failures become bounded
/// reason codes so capture can safely offer StoredOnly instead of blocking a
/// Push of everything else.
pub fn analyze_claude_transcript(bytes: &[u8]) -> SessionAnalysis {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return unknown(None, None, 0, CodecUnknownReason::Empty);
    }
    let mut record_count = 0_usize;
    let mut session_id: Option<String> = None;
    let mut source_cwd: Option<String> = None;
    let mut chain: Vec<CanonicalConversationMessage> = Vec::new();
    let mut seen = BTreeMap::<MessageId, (String, Option<MessageId>)>::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        record_count = record_count.saturating_add(1);
        if record_count > MAX_CLAUDE_RECORDS {
            return unknown(
                session_id,
                source_cwd,
                record_count,
                CodecUnknownReason::TooManyRecords,
            );
        }
        let value: Value = match serde_json::from_slice(raw) {
            Ok(value) => value,
            Err(_) => {
                return unknown(
                    session_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::MalformedJson,
                )
            }
        };
        let Some(object) = value.as_object() else {
            return unknown(
                session_id,
                source_cwd,
                record_count,
                CodecUnknownReason::MalformedJson,
            );
        };
        if let Some(observed) = object.get("sessionId").and_then(Value::as_str) {
            match &session_id {
                Some(existing) if existing != observed => {
                    return unknown(
                        session_id,
                        source_cwd,
                        record_count,
                        CodecUnknownReason::ConflictingSessionMetadata,
                    );
                }
                Some(_) => {}
                None => session_id = Some(observed.to_string()),
            }
        }
        if source_cwd.is_none() {
            if let Some(cwd) = object.get("cwd").and_then(Value::as_str) {
                if !cwd.is_empty() {
                    source_cwd = Some(cwd.to_string());
                }
            }
        }
        if !is_chain_record(object) {
            continue;
        }
        let Some(uuid) = object.get("uuid").and_then(Value::as_str) else {
            return unknown(
                session_id,
                source_cwd,
                record_count,
                CodecUnknownReason::MissingMessageId,
            );
        };
        let message_id = match MessageId::parse(uuid.to_string()) {
            Ok(id) => id,
            Err(_) => {
                return unknown(
                    session_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::InvalidMessageId,
                )
            }
        };
        let explicit_parent = match object.get("parentUuid") {
            None | Some(Value::Null) => None,
            Some(Value::String(parent)) => match MessageId::parse(parent.to_string()) {
                Ok(parent) => Some(parent),
                Err(_) => {
                    return unknown(
                        session_id,
                        source_cwd,
                        record_count,
                        CodecUnknownReason::InvalidMessageId,
                    )
                }
            },
            Some(_) => {
                return unknown(
                    session_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::MalformedJson,
                )
            }
        };
        let fingerprint = semantic_record_fingerprint(value);
        if let Some((existing_fingerprint, existing_parent)) = seen.get(&message_id) {
            // An exact duplicate replay is idempotent; anything else is a
            // duplicate ID or a branch.
            if existing_fingerprint == &fingerprint
                && explicit_parent
                    .as_ref()
                    .is_none_or(|parent| Some(parent) == existing_parent.as_ref())
            {
                continue;
            }
            if existing_fingerprint == &fingerprint {
                return unknown(
                    session_id,
                    source_cwd,
                    record_count,
                    CodecUnknownReason::NonLinearParentage,
                );
            }
            return unknown(
                session_id,
                source_cwd,
                record_count,
                CodecUnknownReason::DuplicateMessageId,
            );
        }
        let previous = chain
            .last()
            .map(|message: &CanonicalConversationMessage| message.message_id.clone());
        if explicit_parent != previous {
            return unknown(
                session_id,
                source_cwd,
                record_count,
                CodecUnknownReason::NonLinearParentage,
            );
        }
        seen.insert(
            message_id.clone(),
            (fingerprint.clone(), explicit_parent.clone()),
        );
        chain.push(CanonicalConversationMessage {
            message_id,
            parent_message_id: explicit_parent,
            semantic_fingerprint: fingerprint,
        });
    }

    if session_id.is_none() {
        return unknown(
            None,
            source_cwd,
            record_count,
            CodecUnknownReason::MissingSessionMetadata,
        );
    }
    if chain.is_empty() {
        return unknown(
            session_id,
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
fn is_chain_record(object: &Map<String, Value>) -> bool {
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

/// The semantic fingerprint excludes exactly the declared structural fields
/// (`sessionId`, `cwd`) plus lineage identity and envelope stamps, so a
/// structural rewrite provably cannot change semantics.
fn semantic_record_fingerprint(mut value: Value) -> String {
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
    hex_sha256(canonical_json(&value).as_bytes())
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

/// Rewrite only the declared structural `sessionId` and `cwd` fields, then
/// prove the result round-trips with identical lineage and semantics.
pub fn rewrite_claude_transcript(
    bytes: &[u8],
    expected_source_session_id: &str,
    destination_session_id: &str,
    expected_source_cwd: &str,
    destination_cwd: &str,
) -> Result<Vec<u8>, String> {
    validate_rewrite_value("source session id", expected_source_session_id, 128)?;
    validate_rewrite_value("destination session id", destination_session_id, 128)?;
    validate_rewrite_value("source cwd", expected_source_cwd, 4096)?;
    validate_rewrite_value("destination cwd", destination_cwd, 4096)?;
    let before = analyze_claude_transcript(bytes);
    if !before.is_resumable()
        || before.source_provider_session_id.as_deref() != Some(expected_source_session_id)
        || before.source_cwd.as_deref() != Some(expected_source_cwd)
    {
        return Err("Claude transcript does not match the reviewed source identity".to_string());
    }

    let final_newline = bytes.ends_with(b"\n");
    let mut lines = Vec::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_slice(raw)
            .map_err(|_| "Claude transcript contains malformed JSON".to_string())?;
        if let Some(object) = value.as_object_mut() {
            rewrite_matching_string(
                object,
                "sessionId",
                expected_source_session_id,
                destination_session_id,
            )?;
            rewrite_matching_string(object, "cwd", expected_source_cwd, destination_cwd)?;
        }
        lines.push(
            serde_json::to_vec(&value)
                .map_err(|error| format!("serialize rewritten Claude record: {error}"))?,
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

    let after = analyze_claude_transcript(&rewritten);
    if !after.is_resumable()
        || after.source_provider_session_id.as_deref() != Some(destination_session_id)
        || after.source_cwd.as_deref() != Some(destination_cwd)
        || lineage_digests(&before) != lineage_digests(&after)
    {
        return Err(
            "rewritten Claude transcript failed semantic round-trip validation".to_string(),
        );
    }
    Ok(rewritten)
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
        .ok_or_else(|| format!("Claude structural field '{key}' is not a string"))?;
    if observed == expected {
        *value = Value::String(replacement.to_string());
    }
    Ok(())
}

fn validate_rewrite_value(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(format!("invalid Claude {label}"))
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
        let chain = before.lineage.chain().unwrap();
        assert_eq!(
            chain
                .iter()
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>(),
            [
                "attachment-hook",
                "user-1",
                "attachment-tools",
                "attachment-skills",
                "assistant-1",
            ]
        );

        let rewritten = rewrite_claude_transcript(
            &bytes,
            session,
            "33333333-3333-4333-8333-333333333333",
            cwd,
            "/tmp/project-b",
        )
        .unwrap();
        let after = analyze_claude_transcript(&rewritten);
        assert_eq!(lineage_digests(&before), lineage_digests(&after));
        assert_eq!(
            after.source_provider_session_id.as_deref(),
            Some("33333333-3333-4333-8333-333333333333")
        );
        assert_eq!(after.source_cwd.as_deref(), Some("/tmp/project-b"));
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

    #[test]
    fn claude_codec_rejects_conflicting_sessions_branches_and_duplicates() {
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
                "cwd": "/tmp/p",
                "message": {"role": "assistant", "content": []}
            }))
            .unwrap()
            .as_bytes(),
        );
        mixed.push(b'\n');
        assert!(matches!(
            analyze_claude_transcript(&mixed).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::ConflictingSessionMetadata
            }
        ));

        let branched = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-1"), "assistant", "two"),
                ("u-2", Some("u-1"), "user", "branch"),
            ],
        );
        assert!(matches!(
            analyze_claude_transcript(&branched).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::NonLinearParentage
            }
        ));

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
    }

    #[test]
    fn structural_rewrite_preserves_lineage_and_semantics() {
        let bytes = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/project-a",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("u-1"), "assistant", "two"),
            ],
        );
        let before = analyze_claude_transcript(&bytes);
        let rewritten = rewrite_claude_transcript(
            &bytes,
            "11111111-1111-4111-8111-111111111111",
            "33333333-3333-4333-8333-333333333333",
            "/tmp/project-a",
            "/tmp/project-b",
        )
        .unwrap();
        let after = analyze_claude_transcript(&rewritten);
        assert_eq!(lineage_digests(&before), lineage_digests(&after));
        assert_eq!(
            after.source_provider_session_id.as_deref(),
            Some("33333333-3333-4333-8333-333333333333")
        );
        assert_eq!(after.source_cwd.as_deref(), Some("/tmp/project-b"));
        assert_ne!(bytes, rewritten);

        // Rewriting with a mismatched identity is refused.
        assert!(rewrite_claude_transcript(
            &bytes,
            "99999999-9999-4999-8999-999999999999",
            "33333333-3333-4333-8333-333333333333",
            "/tmp/project-a",
            "/tmp/project-b",
        )
        .is_err());
    }

    #[test]
    fn rewrite_touches_only_declared_structural_fields() {
        let mut bytes = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/project-a",
            &[("u-1", None, "user", "mentions /tmp/project-a in text")],
        );
        // A record embedding the cwd inside content must keep it verbatim.
        bytes.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "file-history-snapshot",
                "messageId": "u-1",
                "snapshot": {"path": "/tmp/project-a/file.txt"}
            }))
            .unwrap()
            .as_bytes(),
        );
        bytes.push(b'\n');
        let rewritten = rewrite_claude_transcript(
            &bytes,
            "11111111-1111-4111-8111-111111111111",
            "33333333-3333-4333-8333-333333333333",
            "/tmp/project-a",
            "/tmp/project-b",
        )
        .unwrap();
        let text = String::from_utf8(rewritten).unwrap();
        assert!(text.contains("mentions /tmp/project-a in text"));
        assert!(text.contains("/tmp/project-a/file.txt"));
        assert!(!text.contains("\"cwd\":\"/tmp/project-a\""));
    }
}
