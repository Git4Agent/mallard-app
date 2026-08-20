//! Cursor-based tail seeding and refresh for Claude session transcripts.
//!
//! One Claude session is one append-bearing JSONL file — there is no
//! `history_base` pagination, so "full resolve" is a single bounded read plus
//! one `analyze_claude_transcript`. This module gives that file the same
//! checkpoint machinery Codex rollouts have: `seed_claude_root` derives the
//! layout, observed tail, and physical mapping from an already-run analysis,
//! and `refresh_claude_tail` extends the saved accumulator over newly
//! appended complete records from a bounded tail window.
//!
//! Append validation is parent-pointer continuity: an appended chain record
//! parented to the saved leaf folds incrementally; anything that changes the
//! canonical chain's shape (a rewind to an earlier uuid, a fresh mid-file
//! root, a re-emission of the leaf, a foreign `sessionId`) is
//! `NeedsFullResolve`, and the caller degrades to the full path whose result
//! is whatever a from-scratch analysis concludes. The cursor retains only the
//! leaf identity, so a re-emission of an *earlier* on-chain uuid — which a
//! full parse would fail closed as a duplicate — is outside its view, exactly
//! like Codex's resumed builder; full-parse verification points (push review
//! capture, apply re-proofs) re-prove the projection and block on divergence.

use std::path::Path;

use crate::conversation_codec::CodecLineage;
use crate::jsonl_tail_io::{hex_sha256, safe_record_end, TailWindow};
use crate::providers::claude::codec::{
    classify_claude_record, semantic_record_fingerprint, ClaudeRecordClass, MAX_CLAUDE_RECORDS,
};
use crate::providers::traits::SessionAnalysis;
use mallard_domain::{
    compute_portable_conversation_tail_with_codec, CanonicalConversationMessage, MessageId,
    PortableConversationTail, RawRangePin, RootPhysicalMapping, RootTailLayout, TailAccumulator,
    TailPartitionKind, CLAUDE_PORTABLE_TAIL_CODEC_V1,
};
use serde_json::Value;

/// Everything first-contact seeding persists, derived from one full parse.
#[derive(Clone, Debug)]
pub struct ClaudeSeededRoot {
    pub layout: RootTailLayout,
    pub observed_tail: PortableConversationTail,
    pub mapping: RootPhysicalMapping,
    pub raw_range_pin: RawRangePin,
    pub lineage_digest: String,
    pub semantic_digest: String,
}

/// Outcome of a cursor-based refresh over one Claude session file.
#[derive(Clone, Debug)]
// The extension payload dwarfs the marker variants by design; boxing it
// would change every consumer for no behavioral gain.
#[allow(clippy::large_enum_variant)]
pub enum ClaudeTailRefresh {
    Unchanged,
    Extended {
        observed_tail: PortableConversationTail,
        mapping: RootPhysicalMapping,
    },
    /// The canonical chain changed shape; one full re-analyze re-derives it.
    NeedsFullResolve,
    /// The source no longer matches the checkpoint's identity contract.
    Invalid {
        reason: ClaudeTailInvalidReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeTailInvalidReason {
    Truncated,
    Replaced,
    MalformedAppend,
}

/// The outcome of feeding one appended record to the resumable chain feeder.
#[derive(Clone, Debug)]
pub enum ClaudeFeedOutcome {
    /// A chain record parented to the saved leaf: fold it.
    Appended(CanonicalConversationMessage),
    /// Metadata, sidechain, or uuid-less record: advance cursors only.
    Ignored,
    /// The canonical chain changed shape (rewind, mid-file root, leaf
    /// re-emission, or foreign session identity).
    NeedsFullResolve,
    /// Malformed JSON structure or invalid identity fields.
    Malformed,
}

/// The resumable analogue of the one-shot walk-back: `analyze` derives the
/// chain from the whole file, the feeder extends it across appends that keep
/// the leaf lineage linear. Both apply the same record classification and
/// semantic fingerprint (`classify_claude_record`,
/// `semantic_record_fingerprint`), so a linear append folds to the identical
/// canonical message the next full parse would derive.
pub struct ClaudeChainFeeder {
    last_message_id: MessageId,
    provider_session_id: String,
}

impl ClaudeChainFeeder {
    pub fn resume(last_message_id: MessageId, provider_session_id: String) -> Self {
        Self {
            last_message_id,
            provider_session_id,
        }
    }

    pub fn feed(&mut self, value: &Value) -> ClaudeFeedOutcome {
        let Some(object) = value.as_object() else {
            return ClaudeFeedOutcome::Malformed;
        };
        let (message_id, parent) = match classify_claude_record(object) {
            Ok(ClaudeRecordClass::NotChain) => return ClaudeFeedOutcome::Ignored,
            Ok(ClaudeRecordClass::Chain { message_id, parent }) => (message_id, parent),
            Err(_) => return ClaudeFeedOutcome::Malformed,
        };
        if let Some(session_id) = object.get("sessionId").and_then(Value::as_str) {
            if session_id != self.provider_session_id {
                return ClaudeFeedOutcome::NeedsFullResolve;
            }
        }
        if message_id == self.last_message_id {
            // A re-emission of the leaf re-parents the resume point.
            return ClaudeFeedOutcome::NeedsFullResolve;
        }
        match parent {
            Some(parent) if parent == self.last_message_id => {
                let message = CanonicalConversationMessage {
                    message_id: message_id.clone(),
                    parent_message_id: Some(parent),
                    semantic_fingerprint: semantic_record_fingerprint(value.clone()),
                };
                self.last_message_id = message_id;
                ClaudeFeedOutcome::Appended(message)
            }
            // A rewind to an earlier uuid or a fresh mid-file root.
            _ => ClaudeFeedOutcome::NeedsFullResolve,
        }
    }

    pub fn last_message_id(&self) -> &MessageId {
        &self.last_message_id
    }
}

/// Derive the layout, observed tail, physical mapping, and raw-range pin
/// from one already-run analysis over `bytes`. Returns `Ok(None)` when the
/// file does not end at a record boundary (a torn tail): the analysis covers
/// bytes the byte cursor could not, so no checkpoint is written and the next
/// review retries. Observation-class work: nothing here touches cloud state.
pub fn seed_claude_root(
    root_path: &Path,
    bytes: &[u8],
    analysis: &SessionAnalysis,
) -> Result<Option<ClaudeSeededRoot>, String> {
    let CodecLineage::Complete {
        chain,
        lineage_digest,
        semantic_digest,
    } = &analysis.lineage
    else {
        return Err("Claude seeding requires a complete analysis".to_string());
    };
    let provider_thread_id = analysis
        .source_provider_session_id
        .clone()
        .ok_or_else(|| "Claude seeding requires a session identity".to_string())?;
    let consumed = safe_record_end(bytes);
    if consumed != bytes.len() as u64 {
        return Ok(None);
    }
    let leaf = chain
        .last()
        .ok_or_else(|| "complete Claude analysis has an empty chain".to_string())?;
    let observed_tail = compute_portable_conversation_tail_with_codec(
        CLAUDE_PORTABLE_TAIL_CODEC_V1,
        TailPartitionKind::WholeThread,
        0,
        chain,
    )?;
    let probe_window_len = consumed.min(crate::jsonl_tail_io::PROBE_WINDOW_BYTES);
    let window_start = (consumed - probe_window_len) as usize;
    let mapping = RootPhysicalMapping {
        provider_thread_id,
        root_path: root_path
            .to_str()
            .ok_or_else(|| "Claude session path is not valid UTF-8".to_string())?
            .to_string(),
        source_file_size: bytes.len() as u64,
        consumed_byte_offset: consumed,
        consumed_record_ordinal: analysis.record_count as u64,
        probe_window_len,
        probe_window_sha256: hex_sha256(&bytes[window_start..consumed as usize]),
        last_canonical_message_id: Some(leaf.message_id.as_str().to_string()),
        last_synthetic_turn_id: None,
        next_synthetic_turn_ordinal: 0,
    };
    let raw_range_pin = RawRangePin {
        start_byte_offset: 0,
        end_byte_offset: consumed,
        range_sha256: hex_sha256(&bytes[..consumed as usize]),
    };
    let layout = RootTailLayout {
        codec: CLAUDE_PORTABLE_TAIL_CODEC_V1.to_string(),
        partition_kind: TailPartitionKind::WholeThread,
        start_record_count: 0,
    };
    Ok(Some(ClaudeSeededRoot {
        layout,
        observed_tail,
        mapping,
        raw_range_pin,
        lineage_digest: lineage_digest.clone(),
        semantic_digest: semantic_digest.clone(),
    }))
}

/// Extend a saved accumulator over newly appended complete records, from a
/// bounded tail window holding only the probe window and appended bytes.
/// Reads no byte before the cursor beyond the bounded probe window. Every
/// non-fast-path outcome degrades to the full path at the call site — the
/// cursor is a pure fast path over what full capture would conclude.
pub fn refresh_claude_tail(
    layout: &RootTailLayout,
    observed_tail: &PortableConversationTail,
    mapping: &RootPhysicalMapping,
    tail: &TailWindow,
) -> Result<ClaudeTailRefresh, String> {
    if tail.window_offset != mapping.consumed_byte_offset - mapping.probe_window_len {
        return Err("Claude tail window does not start at the mapping's probe window".to_string());
    }
    let current_len = tail.file_len;
    if current_len < mapping.consumed_byte_offset {
        return Ok(ClaudeTailRefresh::Invalid {
            reason: ClaudeTailInvalidReason::Truncated,
        });
    }
    if tail.bytes.len() as u64 != current_len - tail.window_offset {
        return Err("Claude tail window is shorter than its declared span".to_string());
    }
    let consumed = mapping.consumed_byte_offset as usize;
    let window = mapping.probe_window_len as usize;
    crate::sync_metrics::record_bytes_hashed(Path::new(&mapping.root_path), window as u64);
    let probe = hex_sha256(&tail.bytes[..window]);
    if probe != mapping.probe_window_sha256 {
        return Ok(ClaudeTailRefresh::Invalid {
            reason: ClaudeTailInvalidReason::Replaced,
        });
    }
    let safe_end = consumed as u64 + safe_record_end(&tail.bytes[window..]);
    if safe_end == mapping.consumed_byte_offset {
        return Ok(ClaudeTailRefresh::Unchanged);
    }

    let resume_message_id = mapping
        .last_canonical_message_id
        .as_ref()
        .map(|id| MessageId::parse(id.clone()))
        .transpose()?
        .ok_or_else(|| "Claude mapping lacks its resume cursor".to_string())?;
    let mut feeder =
        ClaudeChainFeeder::resume(resume_message_id, mapping.provider_thread_id.clone());
    let mut accumulator = TailAccumulator::resume(
        layout.partition_kind,
        layout.start_record_count,
        observed_tail.accepted_end_record_count,
        &observed_tail.accepted_tail_state,
    )?;
    let relative = |absolute: u64| (absolute - tail.window_offset) as usize;
    let mut appended_records = 0_u64;
    let mut next_record_ordinal = mapping.consumed_record_ordinal;
    for raw in tail.bytes[window..relative(safe_end)].split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        next_record_ordinal = next_record_ordinal.saturating_add(1);
        if next_record_ordinal > MAX_CLAUDE_RECORDS as u64 {
            // The full parse would fail closed on the record cap; so does
            // the cursor, and the full path decides.
            return Ok(ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::MalformedAppend,
            });
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            return Ok(ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::MalformedAppend,
            });
        };
        appended_records += 1;
        match feeder.feed(&value) {
            ClaudeFeedOutcome::Appended(message) => {
                accumulator.fold_message(&message)?;
            }
            ClaudeFeedOutcome::Ignored => {}
            ClaudeFeedOutcome::NeedsFullResolve => {
                return Ok(ClaudeTailRefresh::NeedsFullResolve);
            }
            ClaudeFeedOutcome::Malformed => {
                return Ok(ClaudeTailRefresh::Invalid {
                    reason: ClaudeTailInvalidReason::MalformedAppend,
                });
            }
        }
    }

    crate::sync_metrics::record_records_parsed(Path::new(&mapping.root_path), appended_records);
    let probe_window_len = safe_end.min(crate::jsonl_tail_io::PROBE_WINDOW_BYTES);
    let window_start = safe_end - probe_window_len;
    let new_mapping = RootPhysicalMapping {
        provider_thread_id: mapping.provider_thread_id.clone(),
        root_path: mapping.root_path.clone(),
        source_file_size: current_len,
        consumed_byte_offset: safe_end,
        consumed_record_ordinal: next_record_ordinal,
        probe_window_len,
        probe_window_sha256: hex_sha256(&tail.bytes[relative(window_start)..relative(safe_end)]),
        last_canonical_message_id: Some(feeder.last_message_id().as_str().to_string()),
        last_synthetic_turn_id: None,
        next_synthetic_turn_ordinal: 0,
    };
    let observed = PortableConversationTail {
        codec: layout.codec.clone(),
        partition_kind: layout.partition_kind,
        start_record_count: layout.start_record_count,
        accepted_end_record_count: accumulator.folded_end_record_count(),
        accepted_tail_state: accumulator.state_hex(),
    };
    observed.validate()?;
    Ok(ClaudeTailRefresh::Extended {
        observed_tail: observed,
        mapping: new_mapping,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::claude::codec::analyze_claude_transcript;
    use mallard_domain::compute_portable_conversation_tail_with_codec;
    use serde_json::json;

    const SESSION: &str = "11111111-1111-4111-8111-111111111111";
    const CWD: &str = "/tmp/project-a";

    fn record(uuid: &str, parent: Option<&str>, role: &str, text: &str) -> String {
        serde_json::to_string(&json!({
            "type": role,
            "uuid": uuid,
            "parentUuid": parent.map(str::to_string),
            "sessionId": SESSION,
            "cwd": CWD,
            "timestamp": "2026-08-20T00:00:00.000Z",
            "version": "2.0.0",
            "message": {"role": role, "content": [{"type": "text", "text": text}]}
        }))
        .unwrap()
    }

    fn metadata_line() -> String {
        serde_json::to_string(&json!({
            "type": "file-history-snapshot",
            "messageId": "u-1",
            "snapshot": {"path": "/tmp/project-a/file.txt"}
        }))
        .unwrap()
    }

    fn sidechain_line(uuid: &str) -> String {
        serde_json::to_string(&json!({
            "type": "user",
            "uuid": uuid,
            "parentUuid": null,
            "isSidechain": true,
            "sessionId": SESSION,
            "cwd": CWD,
            "message": {"role": "user", "content": "subagent prompt"}
        }))
        .unwrap()
    }

    fn encode(lines: &[String]) -> Vec<u8> {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        bytes
    }

    fn fixture_lines() -> Vec<String> {
        vec![
            record("u-1", None, "user", "one"),
            metadata_line(),
            record("a-1", Some("u-1"), "assistant", "two"),
            sidechain_line("side-1"),
            record("u-2", Some("a-1"), "user", "three"),
            record("a-2", Some("u-2"), "assistant", "four"),
        ]
    }

    fn seed(path: &Path, bytes: &[u8]) -> ClaudeSeededRoot {
        let analysis = analyze_claude_transcript(bytes);
        assert!(analysis.is_resumable(), "{:?}", analysis.lineage);
        seed_claude_root(path, bytes, &analysis)
            .unwrap()
            .expect("newline-terminated fixture seeds")
    }

    /// A tail window equivalent to handing the whole buffer to the cursor
    /// path, for equivalence tests.
    fn tail_window_from(mapping: &RootPhysicalMapping, full: &[u8]) -> TailWindow {
        let window_offset = mapping.consumed_byte_offset - mapping.probe_window_len;
        if (full.len() as u64) < mapping.consumed_byte_offset {
            return TailWindow {
                file_len: full.len() as u64,
                window_offset,
                bytes: Vec::new(),
            };
        }
        TailWindow {
            file_len: full.len() as u64,
            window_offset,
            bytes: full[window_offset as usize..].to_vec(),
        }
    }

    fn one_shot_tail(bytes: &[u8]) -> PortableConversationTail {
        let analysis = analyze_claude_transcript(bytes);
        let chain = analysis.lineage.chain().expect("complete fixture");
        compute_portable_conversation_tail_with_codec(
            CLAUDE_PORTABLE_TAIL_CODEC_V1,
            TailPartitionKind::WholeThread,
            0,
            chain,
        )
        .unwrap()
    }

    /// For every record-boundary split, refreshing a seed of the prefix over
    /// the full file reproduces the one-shot fold and cursors.
    #[test]
    fn refresh_from_any_split_matches_the_one_shot_fold() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let lines = fixture_lines();
        let full = encode(&lines);
        let expected = one_shot_tail(&full);
        let full_analysis = analyze_claude_transcript(&full);

        let mut covered_splits = 0;
        for split in 1..lines.len() {
            let prefix = encode(&lines[..split]);
            let analysis = analyze_claude_transcript(&prefix);
            if !analysis.is_resumable() {
                continue;
            }
            covered_splits += 1;
            let seeded = seed_claude_root(&path, &prefix, &analysis)
                .unwrap()
                .unwrap();
            match refresh_claude_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &full),
            )
            .unwrap()
            {
                ClaudeTailRefresh::Extended {
                    observed_tail,
                    mapping,
                } => {
                    assert_eq!(observed_tail, expected, "split {split}");
                    assert_eq!(mapping.consumed_byte_offset, full.len() as u64);
                    assert_eq!(mapping.source_file_size, full.len() as u64);
                    assert_eq!(
                        mapping.consumed_record_ordinal,
                        full_analysis.record_count as u64
                    );
                    assert_eq!(mapping.last_canonical_message_id.as_deref(), Some("a-2"));
                }
                other => panic!("split {split} expected extension, got {other:?}"),
            }
        }
        assert!(covered_splits >= 4, "fixture must exercise several splits");

        // The complete file refreshes to Unchanged.
        let seeded = seed(&path, &full);
        assert!(matches!(
            refresh_claude_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &full),
            )
            .unwrap(),
            ClaudeTailRefresh::Unchanged
        ));
    }

    #[test]
    fn seeding_matches_the_domain_one_shot_and_pins_the_whole_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let full = encode(&fixture_lines());
        let seeded = seed(&path, &full);
        assert_eq!(seeded.observed_tail, one_shot_tail(&full));
        assert_eq!(seeded.layout.codec, CLAUDE_PORTABLE_TAIL_CODEC_V1);
        assert_eq!(seeded.layout.partition_kind, TailPartitionKind::WholeThread);
        assert_eq!(seeded.layout.start_record_count, 0);
        assert_eq!(seeded.mapping.consumed_byte_offset, full.len() as u64);
        assert_eq!(seeded.mapping.consumed_record_ordinal, 6);
        assert_eq!(
            seeded.mapping.last_canonical_message_id.as_deref(),
            Some("a-2")
        );
        assert_eq!(seeded.mapping.last_synthetic_turn_id, None);
        assert_eq!(seeded.mapping.next_synthetic_turn_ordinal, 0);
        assert_eq!(seeded.raw_range_pin.start_byte_offset, 0);
        assert_eq!(seeded.raw_range_pin.end_byte_offset, full.len() as u64);
        seeded.mapping.validate().unwrap();
        seeded.raw_range_pin.validate().unwrap();

        // A torn tail (no trailing newline) skips seeding entirely.
        let mut torn = full.clone();
        torn.pop();
        let analysis = analyze_claude_transcript(&torn);
        assert!(analysis.is_resumable(), "the torn record still parses");
        assert!(seed_claude_root(&path, &torn, &analysis).unwrap().is_none());
    }

    #[test]
    fn sidechain_and_metadata_appends_advance_the_cursor_without_folding() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let full = encode(&fixture_lines());
        let seeded = seed(&path, &full);

        let mut appended = full.clone();
        appended.extend_from_slice(sidechain_line("side-2").as_bytes());
        appended.push(b'\n');
        appended.extend_from_slice(metadata_line().as_bytes());
        appended.push(b'\n');
        let ClaudeTailRefresh::Extended {
            observed_tail,
            mapping,
        } = refresh_claude_tail(
            &seeded.layout,
            &seeded.observed_tail,
            &seeded.mapping,
            &tail_window_from(&seeded.mapping, &appended),
        )
        .unwrap()
        else {
            panic!("ignored appends still extend the byte cursor");
        };
        assert_eq!(observed_tail, seeded.observed_tail, "tail count unchanged");
        assert_eq!(mapping.consumed_byte_offset, appended.len() as u64);
        assert_eq!(
            mapping.consumed_record_ordinal,
            seeded.mapping.consumed_record_ordinal + 2
        );
        assert_eq!(
            mapping.last_canonical_message_id,
            seeded.mapping.last_canonical_message_id
        );
    }

    #[test]
    fn torn_appended_records_stay_unconsumed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let full = encode(&fixture_lines());
        let seeded = seed(&path, &full);

        let complete = record("u-3", Some("a-2"), "user", "five");
        let mut appended = full.clone();
        appended.extend_from_slice(complete.as_bytes());
        appended.push(b'\n');
        let consumed_end = appended.len() as u64;
        appended.extend_from_slice(b"{\"type\":\"user\",\"uuid\":\"torn");
        let ClaudeTailRefresh::Extended {
            observed_tail,
            mapping,
        } = refresh_claude_tail(
            &seeded.layout,
            &seeded.observed_tail,
            &seeded.mapping,
            &tail_window_from(&seeded.mapping, &appended),
        )
        .unwrap()
        else {
            panic!("the complete appended record extends the cursor");
        };
        assert_eq!(mapping.consumed_byte_offset, consumed_end);
        assert!(mapping.consumed_byte_offset < appended.len() as u64);
        assert_eq!(mapping.last_canonical_message_id.as_deref(), Some("u-3"));
        assert_eq!(
            observed_tail.accepted_end_record_count,
            seeded.observed_tail.accepted_end_record_count + 1
        );
    }

    #[test]
    fn chain_shape_changes_need_a_full_resolve() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let full = encode(&fixture_lines());
        let seeded = seed(&path, &full);
        let refresh_with = |appended_line: &str| {
            let mut appended = full.clone();
            appended.extend_from_slice(appended_line.as_bytes());
            appended.push(b'\n');
            refresh_claude_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &appended),
            )
            .unwrap()
        };

        // A rewind to an earlier uuid.
        assert!(matches!(
            refresh_with(&record("u-3", Some("u-2"), "user", "rewound")),
            ClaudeTailRefresh::NeedsFullResolve
        ));
        // A fresh mid-file root (compaction).
        assert!(matches!(
            refresh_with(&record("compact-1", None, "user", "summary")),
            ClaudeTailRefresh::NeedsFullResolve
        ));
        // A re-emission of the leaf.
        assert!(matches!(
            refresh_with(&record("a-2", Some("u-2"), "assistant", "re-emitted")),
            ClaudeTailRefresh::NeedsFullResolve
        ));
        // A foreign session identity on a chain record.
        let foreign = serde_json::to_string(&json!({
            "type": "user",
            "uuid": "u-3",
            "parentUuid": "a-2",
            "sessionId": "22222222-2222-4222-8222-222222222222",
            "cwd": CWD,
            "message": {"role": "user", "content": "copied"}
        }))
        .unwrap();
        assert!(matches!(
            refresh_with(&foreign),
            ClaudeTailRefresh::NeedsFullResolve
        ));

        // Malformed JSON and malformed identity fields are invalid appends.
        assert!(matches!(
            refresh_with("{not json}"),
            ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::MalformedAppend
            }
        ));
        let bad_parent = serde_json::to_string(&json!({
            "type": "user",
            "uuid": "u-3",
            "parentUuid": 7,
            "sessionId": SESSION,
            "message": {"role": "user", "content": "bad"}
        }))
        .unwrap();
        assert!(matches!(
            refresh_with(&bad_parent),
            ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::MalformedAppend
            }
        ));
    }

    #[test]
    fn truncation_replacement_and_the_record_cap_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let full = encode(&fixture_lines());
        let seeded = seed(&path, &full);

        // Truncation beneath the cursor.
        assert!(matches!(
            refresh_claude_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &full[..full.len() - 10]),
            )
            .unwrap(),
            ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::Truncated
            }
        ));

        // An in-window mutation is a replacement.
        let mut replaced = full.clone();
        let flip = replaced.len() - 20;
        replaced[flip] = b'X';
        assert!(matches!(
            refresh_claude_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &replaced),
            )
            .unwrap(),
            ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::Replaced
            }
        ));

        // Appends past the bounded record cap fail closed; the full path
        // (which fails the same cap) decides.
        let mut capped = seeded.clone();
        capped.mapping.consumed_record_ordinal = MAX_CLAUDE_RECORDS as u64;
        let mut appended = full.clone();
        appended.extend_from_slice(record("u-3", Some("a-2"), "user", "over").as_bytes());
        appended.push(b'\n');
        assert!(matches!(
            refresh_claude_tail(
                &capped.layout,
                &capped.observed_tail,
                &capped.mapping,
                &tail_window_from(&capped.mapping, &appended),
            )
            .unwrap(),
            ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::MalformedAppend
            }
        ));
    }
}
