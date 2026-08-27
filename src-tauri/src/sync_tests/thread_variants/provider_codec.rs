use super::harness::TranscriptBuilder;
use mallard_domain::{
    compare_conversation_tail, compute_portable_conversation_tail, default_conversation_decision,
    ConversationCapabilities, ConversationPlanDirection, ConversationReviewAction, SyncCurrency,
    TailClassification, TailPartitionKind, TailSideEvidence, VerifiedTailEvidence,
    MAX_CONVERSATION_FILE_BYTES,
};
use mallard_infrastructure::codex_history_graph::resolve_codex_root;
use mallard_infrastructure::codex_standalone_builder::{
    build_standalone_codex_root, StandaloneBuildMode,
};
use mallard_infrastructure::conversation_codec::{
    analyze_codex_transcript, CodecLineage, CodecUnknownReason,
};
use serde_json::{json, Value};

const VALID_TRANSCRIPT: &[u8] =
    include_bytes!("../../../testdata/thread_variants/codex/v1/valid-continuation.jsonl");
const INDEX_FRAGMENT: &[u8] =
    include_bytes!("../../../testdata/thread_variants/codex/v1/session-index-fragment.json");
const SIDECAR: &[u8] =
    include_bytes!("../../../testdata/thread_variants/codex/v1/thread-sidecar.json");

#[test]
fn committed_codex_fixture_proves_wire_round_trip_and_semantic_continuation() {
    let before = analyze_codex_transcript(VALID_TRANSCRIPT);
    assert!(before.is_resumable());
    assert_eq!(before.source_thread_id.as_deref(), Some("thread-fixture"));
    assert_eq!(before.source_cwd.as_deref(), Some("/fixture/project"));
    assert_eq!(before.lineage.chain().unwrap().len(), 5);

    // Push encode: token wire with no source paths, then pull decode onto a
    // different root under a fresh identity — the canonical wire pipeline
    // that replaced the structural-field rewrite. In-cwd content mentions
    // (the fixture's `pwd` output) remap with everything else.
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("thread-fixture.jsonl");
    std::fs::write(&source_path, VALID_TRANSCRIPT).unwrap();
    let no_bases = |_: &str| Ok(None);
    let resolved = resolve_codex_root(&source_path, &no_bases).unwrap();
    let wire = build_standalone_codex_root(
        &resolved,
        &StandaloneBuildMode::Encode {
            root: "/fixture/project".to_string(),
            canonical_root: None,
        },
        None,
    )
    .unwrap();
    let wire_text = String::from_utf8(wire.root_bytes.clone()).unwrap();
    assert!(!wire_text.contains("/fixture/project"));
    assert!(wire_text.contains("/__mallard_root__"));

    let staged_path = directory.path().join("staged.jsonl");
    std::fs::write(&staged_path, &wire.root_bytes).unwrap();
    let staged = resolve_codex_root(&staged_path, &no_bases).unwrap();
    let decoded = build_standalone_codex_root(
        &staged,
        &StandaloneBuildMode::Decode {
            root_provider_thread_id: "thread-destination".to_string(),
            target_root: "/fixture/destination".to_string(),
            canonical_target_root: None,
        },
        None,
    )
    .unwrap();
    let after = analyze_codex_transcript(&decoded.root_bytes);
    assert!(after.is_resumable());
    assert_eq!(
        after.source_thread_id.as_deref(),
        Some("thread-destination")
    );
    assert_eq!(after.source_cwd.as_deref(), Some("/fixture/destination"));
    // Message identities survive the hop one-to-one; content digests are
    // per-form because in-cwd paths remap (the old structural rewrite left
    // the `pwd` output stale — the canonical wire fixes exactly that).
    let before_ids: Vec<_> = before
        .lineage
        .chain()
        .unwrap()
        .iter()
        .map(|message| message.message_id.clone())
        .collect();
    let after_ids: Vec<_> = after
        .lineage
        .chain()
        .unwrap()
        .iter()
        .map(|message| message.message_id.clone())
        .collect();
    assert_eq!(before_ids, after_ids);
    let decoded_text = String::from_utf8(decoded.root_bytes.clone()).unwrap();
    assert!(decoded_text.contains("Final output:\\n/fixture/destination"));
    assert_eq!(
        normalized_records(VALID_TRANSCRIPT).len(),
        normalized_records(&decoded.root_bytes).len()
    );
    let rewritten = decoded.root_bytes.clone();

    let index: Value = serde_json::from_slice(INDEX_FRAGMENT).unwrap();
    let sidecar: Value = serde_json::from_slice(SIDECAR).unwrap();
    assert_eq!(index["id"], "thread-fixture");
    assert_eq!(index["thread_name"], "Fixture continuation");
    assert_eq!(index["updated_at"], "2026-07-22T00:00:06.000000Z");
    assert!(
        index.get("cwd").is_none(),
        "index rows use the provider schema, not Mallard-invented fields"
    );
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
    let continued_chain = continued.lineage.chain().unwrap();
    let after_chain = after.lineage.chain().unwrap();
    assert_eq!(&continued_chain[..after_chain.len()], after_chain);
}

/// Codex restamps every record envelope and re-serializes context records
/// under its current schema when it forks or rewrites a thread.  The
/// canonical digests must ignore all of that, or manifest-only refresh would
/// read every faithful provider rewrite as divergence.
#[test]
fn canonical_digests_survive_envelope_restamps_and_context_reserialization() {
    let before = analyze_codex_transcript(VALID_TRANSCRIPT);
    let rewritten_lines = VALID_TRANSCRIPT
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut record: Value = serde_json::from_slice(line).unwrap();
            record["timestamp"] = json!("2026-07-31T09:00:00Z");
            if record["type"] == "turn_context" {
                // The schema-default field Codex drops or adds when it
                // re-emits the turn's environment.
                record["payload"]["sandbox_policy"] = json!({
                    "type": "workspace-write",
                    "missing_path_behavior": "skip",
                });
            }
            serde_json::to_string(&record).unwrap()
        })
        .collect::<Vec<_>>();
    let rewritten = format!("{}\n", rewritten_lines.join("\n")).into_bytes();
    assert_ne!(VALID_TRANSCRIPT, rewritten.as_slice());

    let after = analyze_codex_transcript(&rewritten);
    assert!(after.is_resumable());
    assert_eq!(lineage_digests(&before), lineage_digests(&after));

    // The stored portable tail of the original classifies the rewritten
    // local as in sync without any payload in hand.
    let before_tail = compute_portable_conversation_tail(
        TailPartitionKind::WholeThread,
        0,
        before.lineage.chain().unwrap(),
    )
    .unwrap();
    let after_tail = compute_portable_conversation_tail(
        TailPartitionKind::WholeThread,
        0,
        after.lineage.chain().unwrap(),
    )
    .unwrap();
    assert_eq!(
        compare_conversation_tail(
            &VerifiedTailEvidence::new(
                TailSideEvidence::Present(before_tail.clone()),
                TailSideEvidence::Present(before_tail.clone()),
                TailSideEvidence::Present(after_tail),
                TailSideEvidence::Present(before_tail),
            )
            .unwrap(),
        ),
        TailClassification::Sync(SyncCurrency::Current)
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
            TailClassification::Unknown,
            ConversationCapabilities {
                local_safely_capturable: false,
                ..ConversationCapabilities::default()
            },
        );
        let pull = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::Unknown,
            ConversationCapabilities {
                cloud_materializable: false,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(
            push.action,
            ConversationReviewAction::Blocked,
            "{name} Push"
        );
        assert_eq!(
            pull.action,
            ConversationReviewAction::Blocked,
            "{name} Pull"
        );
    }
}

fn lineage_digests(
    analysis: &mallard_infrastructure::conversation_codec::CodexTranscriptAnalysis,
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
