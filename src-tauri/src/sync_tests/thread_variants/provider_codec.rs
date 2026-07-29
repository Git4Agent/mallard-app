use super::harness::TranscriptBuilder;
use crate::project_sync::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage, CodecUnknownReason,
};
use crate::project_sync::conversation_sync::{
    default_conversation_decision, ConversationCapabilities,
};
use crate::project_sync::domain::{
    ConversationPlanActionType, ConversationPlanDirection, ConversationRelation,
    MAX_CONVERSATION_FILE_BYTES,
};
use serde_json::{json, Value};

const VALID_TRANSCRIPT: &[u8] =
    include_bytes!("../../../testdata/thread_variants/codex/v1/valid-continuation.jsonl");
const INDEX_FRAGMENT: &[u8] =
    include_bytes!("../../../testdata/thread_variants/codex/v1/session-index-fragment.json");
const SIDECAR: &[u8] =
    include_bytes!("../../../testdata/thread_variants/codex/v1/thread-sidecar.json");

#[test]
fn committed_codex_fixture_proves_structural_rewrite_and_semantic_continuation() {
    let before = analyze_codex_transcript(VALID_TRANSCRIPT);
    assert!(before.is_resumable());
    assert_eq!(before.source_thread_id.as_deref(), Some("thread-fixture"));
    assert_eq!(before.source_cwd.as_deref(), Some("/fixture/project"));
    assert_eq!(before.lineage.chain().unwrap().len(), 5);

    let rewritten = rewrite_codex_transcript(
        VALID_TRANSCRIPT,
        "thread-fixture",
        "thread-destination",
        "/fixture/project",
        "/fixture/destination",
    )
    .unwrap();
    let after = analyze_codex_transcript(&rewritten);
    assert!(after.is_resumable());
    assert_eq!(
        after.source_thread_id.as_deref(),
        Some("thread-destination")
    );
    assert_eq!(after.source_cwd.as_deref(), Some("/fixture/destination"));
    assert_eq!(lineage_digests(&before), lineage_digests(&after));
    assert_eq!(
        normalized_records(VALID_TRANSCRIPT),
        normalized_records(&rewritten)
    );

    let index: Value = serde_json::from_slice(INDEX_FRAGMENT).unwrap();
    let sidecar: Value = serde_json::from_slice(SIDECAR).unwrap();
    assert_eq!(index["id"], "thread-fixture");
    assert_eq!(index["cwd"], "/fixture/project");
    assert_eq!(sidecar["thread_id"], "thread-fixture");
    assert_eq!(sidecar["cwd"], "/fixture/project");

    let mut appended = rewritten;
    appended.extend_from_slice(
        serde_json::to_string(&json!({
            "timestamp": "2026-07-22T00:00:07Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-006",
                "parent_id": "msg-005",
                "role": "user",
                "content": [{"type": "input_text", "text": "Continue on this replica."}]
            }
        }))
        .unwrap()
        .as_bytes(),
    );
    appended.push(b'\n');
    let continued = analyze_codex_transcript(&appended);
    assert!(continued.is_resumable());
    assert_eq!(continued.lineage.chain().unwrap().len(), 6);
    assert_eq!(
        crate::project_sync::domain::classify_conversation_relation(
            continued.lineage.chain(),
            after.lineage.chain(),
        ),
        ConversationRelation::CloudPrefixOfLocal
    );
}

#[test]
fn every_codex_unknown_reason_is_bounded_and_never_resumable() {
    let session = |id: &str, cwd: &str| {
        serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {"id": id, "cwd": cwd}
        }))
        .unwrap()
    };
    let message = |id: Option<&str>, parent: Option<&str>| {
        let mut payload = json!({
            "type": "message",
            "parent_id": parent,
            "role": "user",
            "content": [{"type": "input_text", "text": "fixture"}]
        });
        if let Some(id) = id {
            payload["id"] = Value::String(id.to_string());
        }
        serde_json::to_string(&json!({
            "type": "response_item",
            "payload": payload
        }))
        .unwrap()
    };
    let lines = |records: Vec<String>| format!("{}\n", records.join("\n")).into_bytes();

    let mut too_many_records = Vec::with_capacity(400_004);
    for _ in 0..100_001 {
        too_many_records.extend_from_slice(b"{}\n");
    }
    let cases = vec![
        ("empty", Vec::new(), CodecUnknownReason::Empty),
        (
            "oversized_file",
            vec![b'x'; MAX_CONVERSATION_FILE_BYTES as usize + 1],
            CodecUnknownReason::OversizedFile,
        ),
        (
            "too_many_records",
            too_many_records,
            CodecUnknownReason::TooManyRecords,
        ),
        (
            "malformed_json",
            b"{not-json}\n".to_vec(),
            CodecUnknownReason::MalformedJson,
        ),
        (
            "missing_session_metadata",
            lines(vec![message(Some("msg-a"), None)]),
            CodecUnknownReason::MissingSessionMetadata,
        ),
        (
            "conflicting_session_metadata",
            lines(vec![session("thread-a", "/a"), session("thread-b", "/a")]),
            CodecUnknownReason::ConflictingSessionMetadata,
        ),
        (
            "missing_message_id",
            lines(vec![session("thread-a", "/a"), message(None, None)]),
            CodecUnknownReason::MissingMessageId,
        ),
        (
            "duplicate_message_id",
            lines(vec![
                session("thread-a", "/a"),
                message(Some("msg-a"), None),
                message(Some("msg-a"), Some("msg-a")).replace("fixture", "conflicting"),
            ]),
            CodecUnknownReason::DuplicateMessageId,
        ),
        (
            "cycle",
            lines(vec![
                session("thread-a", "/a"),
                message(Some("msg-a"), Some("msg-b")),
                message(Some("msg-b"), Some("msg-a")),
            ]),
            CodecUnknownReason::NonLinearParentage,
        ),
        (
            "branched_parentage",
            TranscriptBuilder::new("thread-a", "/a")
                .message_with_parent("msg-a", None, "user", "one")
                .message_with_parent("msg-b", Some("msg-a"), "assistant", "two")
                .message_with_parent("msg-c", Some("msg-a"), "user", "fork")
                .build(),
            CodecUnknownReason::NonLinearParentage,
        ),
        (
            "invalid_message_id",
            lines(vec![
                session("thread-a", "/a"),
                message(Some("bad\nmessage"), None),
            ]),
            CodecUnknownReason::InvalidMessageId,
        ),
    ];

    for (name, bytes, expected_reason) in cases {
        let analysis = analyze_codex_transcript(&bytes);
        assert!(!analysis.is_resumable(), "{name} became resumable");
        assert_eq!(
            analysis.lineage,
            CodecLineage::Unknown {
                reason: expected_reason,
            },
            "{name} returned the wrong reason",
        );
        let push = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::Unknown,
            ConversationCapabilities {
                local_safely_capturable: false,
                ..ConversationCapabilities::default()
            },
        );
        let pull = default_conversation_decision(
            ConversationPlanDirection::Pull,
            ConversationRelation::Unknown,
            ConversationCapabilities {
                cloud_safely_readable: false,
                destination_codec_resumable: false,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(
            push.action,
            ConversationPlanActionType::Blocked,
            "{name} Push"
        );
        assert_eq!(
            pull.action,
            ConversationPlanActionType::StoreOnly,
            "{name} Pull"
        );
    }
    assert_eq!(
        CodecUnknownReason::UnsupportedCodec.as_str(),
        "unsupported_codec"
    );
}

fn lineage_digests(
    analysis: &crate::project_sync::conversation_codec::CodexTranscriptAnalysis,
) -> (&str, &str) {
    match &analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        CodecLineage::Unknown { reason } => panic!("unexpected unknown lineage: {reason:?}"),
    }
}

fn normalized_records(bytes: &[u8]) -> Vec<Value> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut value: Value = serde_json::from_slice(line).unwrap();
            let kind = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                if kind == "session_meta" {
                    if payload.contains_key("id") {
                        payload.insert("id".to_string(), Value::String("<thread>".to_string()));
                    }
                    if payload.contains_key("cwd") {
                        payload.insert("cwd".to_string(), Value::String("<cwd>".to_string()));
                    }
                }
                if kind == "turn_context" && payload.contains_key("cwd") {
                    payload.insert("cwd".to_string(), Value::String("<cwd>".to_string()));
                }
            }
            value
        })
        .collect()
}
