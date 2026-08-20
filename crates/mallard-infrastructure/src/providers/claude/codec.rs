//! Bounded, versioned Claude Code transcript codec.
//!
//! Claude sessions are JSONL files whose durable records (`user`,
//! `assistant`, `system`, and `attachment`) carry `uuid`/`parentUuid`
//! lineage plus a `sessionId` and `cwd`. Real transcripts are append-only
//! but not linear: a rewind appends a record parented to an earlier uuid, a
//! compaction appends a fresh `parentUuid: null` root mid-file, a resume can
//! copy history that keeps its original `sessionId`, and a record can be
//! re-emitted later under a different parent. The codec derives the one
//! canonical chain Claude itself would resume: walk back from the last chain
//! record via `parentUuid`, resolving every parent reference to its latest
//! occurrence at an earlier file position. Records off that path (abandoned
//! branches, shadowed re-emissions, pre-compaction history) are excluded.
//! It returns typed `Unknown` reasons instead of guessing and rewrites only
//! the declared structural `sessionId` and `cwd` fields — round-tripping the
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

struct ChainRecord {
    message_id: MessageId,
    parent: Option<MessageId>,
    session_id: Option<String>,
    cwd: Option<String>,
    fingerprint: String,
}

/// Analyze a Claude session transcript. Content failures become bounded
/// reason codes so capture can safely offer StoredOnly instead of blocking a
/// Push of everything else.
pub fn analyze_claude_transcript(bytes: &[u8]) -> SessionAnalysis {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return unknown(None, None, 0, CodecUnknownReason::Empty);
    }
    let mut record_count = 0_usize;
    let mut last_session_id: Option<String> = None;
    let mut last_cwd: Option<String> = None;
    let mut records: Vec<ChainRecord> = Vec::new();
    let mut occurrences = BTreeMap::<String, Vec<usize>>::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        record_count = record_count.saturating_add(1);
        if record_count > MAX_CLAUDE_RECORDS {
            return unknown(
                last_session_id,
                last_cwd,
                record_count,
                CodecUnknownReason::TooManyRecords,
            );
        }
        let value: Value = match serde_json::from_slice(raw) {
            Ok(value) => value,
            Err(_) => {
                return unknown(
                    last_session_id,
                    last_cwd,
                    record_count,
                    CodecUnknownReason::MalformedJson,
                )
            }
        };
        let Some(object) = value.as_object() else {
            return unknown(
                last_session_id,
                last_cwd,
                record_count,
                CodecUnknownReason::MalformedJson,
            );
        };
        let session_id = object
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(observed) = &session_id {
            last_session_id = Some(observed.clone());
        }
        let cwd = object
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.is_empty())
            .map(str::to_string);
        if let Some(observed) = &cwd {
            last_cwd = Some(observed.clone());
        }
        let (message_id, parent) = match classify_claude_record(object) {
            Ok(ClaudeRecordClass::NotChain) => continue,
            Ok(ClaudeRecordClass::Chain { message_id, parent }) => (message_id, parent),
            Err(reason) => {
                return unknown(last_session_id, last_cwd, record_count, reason);
            }
        };
        occurrences
            .entry(message_id.as_str().to_string())
            .or_default()
            .push(records.len());
        records.push(ChainRecord {
            message_id,
            parent,
            session_id,
            cwd,
            fingerprint: semantic_record_fingerprint(value),
        });
    }

    if records.is_empty() {
        return unknown(
            last_session_id,
            last_cwd,
            record_count,
            CodecUnknownReason::MissingMessageId,
        );
    }

    // Walk back from the leaf (the last chain record) to a root, resolving
    // each parent reference to its latest occurrence at an earlier position.
    // Resolution is strictly position-decreasing, so the walk terminates
    // without cycle bookkeeping.
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
        let Some(resolved) = resolved else {
            // A dangling parent on the active path fails closed; dangling
            // references off the path were simply never visited.
            return unknown(
                last_session_id,
                last_cwd,
                record_count,
                CodecUnknownReason::NonLinearParentage,
            );
        };
        position = resolved;
    }
    path.reverse();

    // Leaf-wins identity: resume-copied history legitimately mixes ids, and
    // the leaf record carries the identity Claude itself resumes under.
    let leaf = &records[*path.last().expect("walk visits the leaf")];
    let session_id = leaf.session_id.clone().or(last_session_id);
    let source_cwd = leaf.cwd.clone().or(last_cwd);
    if session_id.is_none() {
        return unknown(
            None,
            source_cwd,
            record_count,
            CodecUnknownReason::MissingSessionMetadata,
        );
    }

    let chain: Vec<CanonicalConversationMessage> = path
        .iter()
        .map(|&position| CanonicalConversationMessage {
            message_id: records[position].message_id.clone(),
            parent_message_id: records[position].parent.clone(),
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
/// bound by the chain fold, not the fingerprint.
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
/// prove the result round-trips with identical lineage and semantics. Only
/// occurrences equal to the leaf identity are restamped; resume-copied
/// history keeps its original ids verbatim as provenance.
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

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
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

    fn chain_ids(analysis: &SessionAnalysis) -> Vec<&str> {
        analysis
            .lineage
            .chain()
            .unwrap()
            .iter()
            .map(|message| message.message_id.as_str())
            .collect()
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
        // Resume-copied history keeps its original sessionId and cwd; the
        // leaf record's identity wins and the mix is tolerated.
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
        let analysis = analyze_claude_transcript(&mixed);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        assert_eq!(
            analysis.source_provider_session_id.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
        assert_eq!(analysis.source_cwd.as_deref(), Some("/tmp/q"));
        assert_eq!(chain_ids(&analysis), ["u-1", "a-1"]);
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
    fn claude_codec_rejects_dangling_parents_duplicates_and_malformed_records() {
        // A dangling parent on the active path fails closed.
        let dangling = transcript(
            "11111111-1111-4111-8111-111111111111",
            "/tmp/p",
            &[
                ("u-1", None, "user", "one"),
                ("a-1", Some("ghost"), "assistant", "active"),
            ],
        );
        assert!(matches!(
            analyze_claude_transcript(&dangling).lineage,
            CodecLineage::Unknown {
                reason: CodecUnknownReason::NonLinearParentage
            }
        ));

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
