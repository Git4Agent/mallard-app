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
//! Append validation is parent-pointer continuity over the canonical
//! *projection*: an
//! appended chain record parented to the saved leaf — or to a thinking-only
//! record the projection dropped since the leaf (the skipped-set) — folds
//! incrementally with its parent normalized to the leaf, exactly as the full
//! walk stitches. A thinking-only append is `Pending`: projected out, so the
//! consumed watermark must not advance past it until a chain record lands
//! beyond it. That invariant keeps the skipped-set purely in memory — a
//! refresh that resumes in front of a pending run re-parses the run and
//! rebuilds the set, the way a torn tail re-parses its partial record — so
//! no checkpoint schema changes and warm appends stay O(append) even when a
//! review lands between a thinking record and its continuation. Anything
//! that changes the canonical chain's shape (a rewind to an earlier uuid, a
//! fresh mid-file root, a re-emission of the leaf, a foreign `sessionId`) is
//! `NeedsFullResolve`, and the caller degrades to the full path whose result
//! is whatever a from-scratch analysis concludes. The cursor retains only the
//! leaf identity, so a re-emission of an *earlier* on-chain uuid — which a
//! full parse would fail closed as a duplicate — is outside its view, exactly
//! like Codex's resumed builder; full-parse verification points (push review
//! capture, apply re-proofs) re-prove the projection and block on divergence.

use std::collections::BTreeSet;
use std::path::Path;

use crate::conversation_codec::CodecLineage;
use crate::jsonl_tail_io::{safe_record_end, TailWindow};
use crate::providers::claude::codec::{
    classify_claude_record_in_context, is_thinking_only_record, project_claude_record,
    semantic_record_fingerprint, ClaudeRecordClass, ClaudeRecordContext, MAX_CLAUDE_RECORDS,
};
use crate::providers::traits::SessionAnalysis;
use mallard_domain::{
    compute_portable_conversation_tail_with_codec, sha256_hex, CanonicalConversationMessage,
    MessageId, PortableConversationTail, RawRangePin, RootPhysicalMapping, RootTailLayout,
    TailAccumulator, TailPartitionKind, CLAUDE_PORTABLE_TAIL_CODEC_V1,
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
    /// A chain record parented to the saved leaf (or stitched to it through
    /// the skipped-set): fold it.
    Appended(CanonicalConversationMessage),
    /// Metadata, sidechain, or uuid-less record: advance cursors only.
    Ignored,
    /// A thinking-only record the projection drops: its uuid joins the
    /// skipped-set, and the consumed watermark must not advance past it
    /// until a chain record lands beyond it.
    Pending,
    /// The canonical chain changed shape (rewind, mid-file root, leaf
    /// re-emission, or foreign session identity).
    NeedsFullResolve,
    /// Malformed JSON structure or invalid identity fields.
    Malformed,
}

/// The resumable analogue of the one-shot walk-back: `analyze` derives the
/// chain from the whole file's projection, the feeder extends it across
/// appends that keep the leaf lineage linear. Both apply the same
/// projection, record classification, and semantic fingerprint
/// (`project_claude_record`, `classify_claude_record`,
/// `semantic_record_fingerprint`), so a linear append folds to the identical
/// canonical message the next full parse would derive: a chain record
/// parented to a skipped thinking-only record stitches to the leaf — the
/// nearest preceding chain record — exactly as the full walk does.
pub struct ClaudeChainFeeder {
    last_message_id: MessageId,
    provider_session_id: String,
    /// Uuids of thinking-only records fed since the last chain append. The
    /// set lives only in memory: the consumed-watermark invariant guarantees
    /// a resumed refresh re-parses any pending run and rebuilds it.
    skipped: BTreeSet<String>,
}

impl ClaudeChainFeeder {
    pub fn resume(last_message_id: MessageId, provider_session_id: String) -> Self {
        Self {
            last_message_id,
            provider_session_id,
            skipped: BTreeSet::new(),
        }
    }

    pub fn feed(&mut self, value: &Value) -> ClaudeFeedOutcome {
        self.feed_in_context(value, ClaudeRecordContext::Main)
    }

    pub(crate) fn feed_in_context(
        &mut self,
        value: &Value,
        context: ClaudeRecordContext<'_>,
    ) -> ClaudeFeedOutcome {
        if context.validate(value).is_err() {
            return ClaudeFeedOutcome::Malformed;
        }
        let Some(object) = value.as_object() else {
            return ClaudeFeedOutcome::Malformed;
        };
        // Projected-out rows come first: the full analyzer drops them before
        // identity, session, or classification checks ever see them.
        if is_thinking_only_record(object) {
            if let Some(uuid) = object.get("uuid").and_then(Value::as_str) {
                self.skipped.insert(uuid.to_string());
            }
            return ClaudeFeedOutcome::Pending;
        }
        let (message_id, parent) = match classify_claude_record_in_context(object, context) {
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
        let continues_leaf = parent.as_ref().is_some_and(|parent| {
            *parent == self.last_message_id || self.skipped.contains(parent.as_str())
        });
        if continues_leaf {
            let Some(projected) = project_claude_record(value.clone()) else {
                // Unreachable: thinking-only rows returned Pending above.
                return ClaudeFeedOutcome::Malformed;
            };
            let message = CanonicalConversationMessage {
                message_id: message_id.clone(),
                // Normalized parent = the leaf: a parent in the skipped-set
                // stitches to the nearest preceding chain record, which is
                // exactly the leaf.
                parent_message_id: Some(self.last_message_id.clone()),
                semantic_fingerprint: semantic_record_fingerprint(projected),
            };
            self.last_message_id = message_id;
            self.skipped.clear();
            ClaudeFeedOutcome::Appended(message)
        } else {
            // A rewind to an earlier uuid or a fresh mid-file root.
            ClaudeFeedOutcome::NeedsFullResolve
        }
    }

    pub fn last_message_id(&self) -> &MessageId {
        &self.last_message_id
    }
}

/// The consumed-watermark boundary over one record-aligned byte slice: the
/// end offset (and projected record ordinal) of the last record that may be
/// consumed under the pending invariant — the watermark never passes a
/// thinking-only record until a chain record lands beyond it, so a trailing
/// pending run stays unconsumed and is re-parsed by the next refresh, which
/// rebuilds the in-memory skipped-set from it. Thinking-only records never
/// count toward the ordinal: it is the projected record count at the
/// boundary, matching what a full analysis of the consumed prefix reports.
#[derive(Default)]
pub(crate) struct ClaudeWatermark {
    consumed_end: u64,
    consumed_ordinal: u64,
    candidate_ordinal: u64,
    holding: bool,
}

impl ClaudeWatermark {
    pub(crate) fn feed(
        &mut self,
        value: Option<&Value>,
        record_end: u64,
        context: ClaudeRecordContext<'_>,
    ) -> Result<(), String> {
        let Some(value) = value else {
            if !self.holding {
                self.consumed_end = record_end;
            }
            return Ok(());
        };
        let object = value
            .as_object()
            .ok_or("Claude watermark found a non-object record")?;
        if is_thinking_only_record(object) {
            self.holding = true;
            return Ok(());
        }
        self.candidate_ordinal += 1;
        if matches!(
            classify_claude_record_in_context(object, context),
            Ok(ClaudeRecordClass::Chain { .. })
        ) {
            self.holding = false;
        }
        if !self.holding {
            self.consumed_end = record_end;
            self.consumed_ordinal = self.candidate_ordinal;
        }
        Ok(())
    }
    pub(crate) fn finish(&self) -> (u64, u64) {
        (self.consumed_end, self.consumed_ordinal)
    }
}

fn pending_watermark(bytes: &[u8], context: ClaudeRecordContext<'_>) -> Result<(u64, u64), String> {
    let mut watermark = ClaudeWatermark::default();
    let mut end = 0;
    for raw in bytes.split_inclusive(|byte| *byte == b'\n') {
        end += raw.len() as u64;
        let line = raw.strip_suffix(b"\n").unwrap_or(raw);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let value = if line.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice::<Value>(line)
                    .map_err(|_| "Claude watermark found malformed JSON")?,
            )
        };
        watermark.feed(value.as_ref(), end, context)?;
    }
    Ok(watermark.finish())
}

/// Derive the layout, observed tail, physical mapping, and raw-range pin
/// from one already-run analysis over `bytes`. Returns `Ok(None)` when the
/// file does not end at a record boundary (a torn tail): the analysis covers
/// bytes the byte cursor could not, so no checkpoint is written and the next
/// review retries. The consumed watermark stops before a trailing
/// thinking-only run (`pending_watermark`) — those records are projected out
/// of the analysis, so leaving them unconsumed changes nothing the tail
/// observed. Observation-class work: nothing here touches cloud state.
pub fn seed_claude_root(
    root_path: &Path,
    bytes: &[u8],
    analysis: &SessionAnalysis,
) -> Result<Option<ClaudeSeededRoot>, String> {
    seed_claude_root_in_context(root_path, bytes, analysis, ClaudeRecordContext::Main)
}

pub(crate) fn seed_claude_root_in_context(
    root_path: &Path,
    bytes: &[u8],
    analysis: &SessionAnalysis,
    context: ClaudeRecordContext<'_>,
) -> Result<Option<ClaudeSeededRoot>, String> {
    seed_claude_root_with_watermark(
        root_path,
        bytes,
        analysis,
        pending_watermark(bytes, context)?,
    )
}

pub(crate) fn claude_root_mapping(
    root_path: &Path,
    bytes: &[u8],
    analysis: &SessionAnalysis,
    watermark: (u64, u64),
) -> Result<RootPhysicalMapping, String> {
    let provider_thread_id = analysis
        .source_provider_session_id
        .clone()
        .ok_or("Claude mapping requires a session identity")?;
    let leaf = analysis
        .lineage
        .chain()
        .and_then(|chain| chain.last())
        .ok_or("Claude mapping requires a complete nonempty chain")?;
    let (consumed, consumed_ordinal) = watermark;
    let probe_window_len = consumed.min(crate::jsonl_tail_io::PROBE_WINDOW_BYTES);
    let window_start = (consumed - probe_window_len) as usize;
    Ok(RootPhysicalMapping {
        provider_thread_id,
        root_path: root_path
            .to_str()
            .ok_or_else(|| "Claude session path is not valid UTF-8".to_string())?
            .to_string(),
        source_file_size: bytes.len() as u64,
        consumed_byte_offset: consumed,
        consumed_record_ordinal: consumed_ordinal,
        probe_window_len,
        probe_window_sha256: sha256_hex(&bytes[window_start..consumed as usize]),
        last_canonical_message_id: Some(leaf.message_id.as_str().to_string()),
        last_synthetic_turn_id: None,
        next_synthetic_turn_ordinal: 0,
    })
}

pub fn seed_claude_root_with_watermark(
    root_path: &Path,
    bytes: &[u8],
    analysis: &SessionAnalysis,
    watermark: (u64, u64),
) -> Result<Option<ClaudeSeededRoot>, String> {
    let CodecLineage::Complete {
        chain,
        lineage_digest,
        semantic_digest,
    } = &analysis.lineage
    else {
        return Err("Claude seeding requires a complete analysis".to_string());
    };
    if safe_record_end(bytes) != bytes.len() as u64 {
        return Ok(None);
    }
    let consumed = watermark.0;
    let observed_tail = compute_portable_conversation_tail_with_codec(
        CLAUDE_PORTABLE_TAIL_CODEC_V1,
        TailPartitionKind::WholeThread,
        0,
        chain,
    )?;
    let mapping = claude_root_mapping(root_path, bytes, analysis, watermark)?;
    let raw_range_pin = RawRangePin {
        start_byte_offset: 0,
        end_byte_offset: consumed,
        range_sha256: sha256_hex(&bytes[..consumed as usize]),
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
    refresh_claude_tail_with_records(
        layout,
        observed_tail,
        mapping,
        tail,
        ClaudeRecordContext::Main,
        &mut |_| Ok(()),
    )
}

pub(crate) fn refresh_claude_tail_with_records(
    layout: &RootTailLayout,
    observed_tail: &PortableConversationTail,
    mapping: &RootPhysicalMapping,
    tail: &TailWindow,
    context: ClaudeRecordContext<'_>,
    on_record: &mut dyn FnMut(&Value) -> Result<(), String>,
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
    let probe = sha256_hex(&tail.bytes[..window]);
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
    let batch = &tail.bytes[window..relative(safe_end)];
    let mut appended_records = 0_u64;
    // The incremental twin of `pending_watermark`: consumption commits only
    // while no thinking-only record is pending, and a chain append consumes
    // through the whole pending run. Thinking-only records never count
    // toward the ordinal — it is the projected record count.
    let mut consumed_end = mapping.consumed_byte_offset;
    let mut consumed_ordinal = mapping.consumed_record_ordinal;
    let mut candidate_ordinal = mapping.consumed_record_ordinal;
    let mut holding = false;
    let mut piece_start = 0_usize;
    for raw in batch.split(|byte| *byte == b'\n') {
        let piece_offset = piece_start;
        piece_start += raw.len() + 1;
        if piece_offset >= batch.len() {
            break;
        }
        let record_end = mapping.consumed_byte_offset + piece_start.min(batch.len()) as u64;
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            if !holding {
                consumed_end = record_end;
            }
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            return Ok(ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::MalformedAppend,
            });
        };
        appended_records += 1;
        on_record(&value)?;
        let outcome = feeder.feed_in_context(&value, context);
        match outcome {
            ClaudeFeedOutcome::Pending => {
                holding = true;
                continue;
            }
            ClaudeFeedOutcome::NeedsFullResolve => {
                return Ok(ClaudeTailRefresh::NeedsFullResolve);
            }
            ClaudeFeedOutcome::Malformed => {
                return Ok(ClaudeTailRefresh::Invalid {
                    reason: ClaudeTailInvalidReason::MalformedAppend,
                });
            }
            ClaudeFeedOutcome::Appended(_) | ClaudeFeedOutcome::Ignored => {}
        }
        candidate_ordinal = candidate_ordinal.saturating_add(1);
        if candidate_ordinal > MAX_CLAUDE_RECORDS as u64 {
            // The full parse would fail closed on the record cap; so does
            // the cursor, and the full path decides.
            return Ok(ClaudeTailRefresh::Invalid {
                reason: ClaudeTailInvalidReason::MalformedAppend,
            });
        }
        if let ClaudeFeedOutcome::Appended(message) = &outcome {
            accumulator.fold_message(message)?;
            holding = false;
        }
        if !holding {
            consumed_end = record_end;
            consumed_ordinal = candidate_ordinal;
        }
    }

    crate::sync_metrics::record_records_parsed(Path::new(&mapping.root_path), appended_records);
    if consumed_end == mapping.consumed_byte_offset {
        // Everything appended is still pending — a trailing thinking run is
        // the torn-tail analogue: unconsumed, re-parsed next refresh.
        return Ok(ClaudeTailRefresh::Unchanged);
    }
    let probe_window_len = consumed_end.min(crate::jsonl_tail_io::PROBE_WINDOW_BYTES);
    let window_start = consumed_end - probe_window_len;
    let new_mapping = RootPhysicalMapping {
        provider_thread_id: mapping.provider_thread_id.clone(),
        root_path: mapping.root_path.clone(),
        source_file_size: current_len,
        consumed_byte_offset: consumed_end,
        consumed_record_ordinal: consumed_ordinal,
        probe_window_len,
        probe_window_sha256: sha256_hex(
            &tail.bytes[relative(window_start)..relative(consumed_end)],
        ),
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

    fn thinking_line(uuid: &str, parent: Option<&str>) -> String {
        serde_json::to_string(&json!({
            "type": "assistant",
            "uuid": uuid,
            "parentUuid": parent.map(str::to_string),
            "sessionId": SESSION,
            "cwd": CWD,
            "message": {
                "role": "assistant",
                "content": [{"type": "thinking", "thinking": "hidden", "signature": "sig"}]
            }
        }))
        .unwrap()
    }

    fn mixed_thinking_line(uuid: &str, parent: Option<&str>, text: &str) -> String {
        serde_json::to_string(&json!({
            "type": "assistant",
            "uuid": uuid,
            "parentUuid": parent.map(str::to_string),
            "sessionId": SESSION,
            "cwd": CWD,
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "hidden", "signature": "sig"},
                    {"type": "text", "text": text}
                ]
            }
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
        let stamp = crate::jsonl_tail_io::SourceStamp::for_tests(full.len() as u64);
        if (full.len() as u64) < mapping.consumed_byte_offset {
            return TailWindow {
                file_len: full.len() as u64,
                window_offset,
                bytes: Vec::new(),
                stamp,
            };
        }
        TailWindow {
            file_len: full.len() as u64,
            window_offset,
            bytes: full[window_offset as usize..].to_vec(),
            stamp,
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

    /// The projection twin of the split property: with thinking-only rows,
    /// mixed rows, and stitched links in the stream, every record-boundary
    /// split — including splits inside a pending run — still reproduces the
    /// one-shot fold, cursors, and projected ordinal.
    #[test]
    fn refresh_from_any_split_matches_the_one_shot_fold_with_thinking_rows() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let lines = vec![
            record("u-1", None, "user", "one"),
            thinking_line("t-1", Some("u-1")),
            mixed_thinking_line("a-1", Some("t-1"), "two"),
            metadata_line(),
            thinking_line("t-2", Some("a-1")),
            record("u-2", Some("t-2"), "user", "three"),
            record("a-2", Some("u-2"), "assistant", "four"),
        ];
        let full = encode(&lines);
        let expected = one_shot_tail(&full);
        let full_analysis = analyze_claude_transcript(&full);
        assert_eq!(full_analysis.record_count, 5, "the projected count");

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
            assert!(
                seeded.mapping.consumed_byte_offset <= prefix.len() as u64,
                "split {split} watermark never overshoots"
            );
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
                    assert_eq!(
                        mapping.consumed_record_ordinal,
                        full_analysis.record_count as u64
                    );
                    assert_eq!(mapping.last_canonical_message_id.as_deref(), Some("a-2"));
                }
                other => panic!("split {split} expected extension, got {other:?}"),
            }
        }
        assert!(covered_splits >= 6, "fixture must exercise several splits");
    }

    /// The consumed watermark stops before a trailing thinking run: seeding
    /// holds it back, a refresh over the same bytes is Unchanged, and the
    /// next chain record consumes through the run with the stitched fold.
    #[test]
    fn seeding_and_refresh_hold_the_watermark_before_a_trailing_thinking_run() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let consumable = vec![
            record("u-1", None, "user", "one"),
            record("a-1", Some("u-1"), "assistant", "two"),
        ];
        let consumable_len = encode(&consumable).len() as u64;
        let mut lines = consumable;
        lines.push(thinking_line("t-1", Some("a-1")));
        let full = encode(&lines);
        let seeded = seed(&path, &full);
        assert_eq!(seeded.mapping.consumed_byte_offset, consumable_len);
        assert_eq!(seeded.mapping.consumed_record_ordinal, 2);
        assert_eq!(seeded.mapping.source_file_size, full.len() as u64);
        assert_eq!(seeded.raw_range_pin.end_byte_offset, consumable_len);
        assert_eq!(
            seeded.mapping.last_canonical_message_id.as_deref(),
            Some("a-1")
        );
        assert_eq!(seeded.observed_tail, one_shot_tail(&full));

        // The trailing run is the torn-tail analogue: re-parsed, unconsumed.
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

        // A chain record beyond the run consumes through it, stitched to the
        // saved leaf, and the ordinal counts only projected records.
        let mut appended_lines = lines.clone();
        appended_lines.push(record("u-2", Some("t-1"), "user", "three"));
        let appended = encode(&appended_lines);
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
            panic!("a chain record lands beyond the pending run");
        };
        assert_eq!(observed_tail, one_shot_tail(&appended));
        assert_eq!(mapping.consumed_byte_offset, appended.len() as u64);
        assert_eq!(mapping.consumed_record_ordinal, 3);
        assert_eq!(mapping.last_canonical_message_id.as_deref(), Some("u-2"));
    }

    /// Batch shapes over a settled cursor: a lone thinking append stays
    /// pending (Unchanged); thinking plus its continuation folds in one
    /// Extended; a chain record referencing an unseen absent uuid defers to
    /// the full path, which stitches to the same conclusion.
    #[test]
    fn pending_appends_fold_once_a_chain_record_lands() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let full = encode(&fixture_lines());
        let seeded = seed(&path, &full);

        // T alone: pending, nothing consumed, nothing folded.
        let mut with_thinking = full.clone();
        with_thinking.extend_from_slice(thinking_line("t-9", Some("a-2")).as_bytes());
        with_thinking.push(b'\n');
        assert!(matches!(
            refresh_claude_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &with_thinking),
            )
            .unwrap(),
            ClaudeTailRefresh::Unchanged
        ));

        // T then C in one batch: a single Extended, C stitched to the leaf.
        let mut continued = with_thinking.clone();
        continued.extend_from_slice(record("u-3", Some("t-9"), "user", "five").as_bytes());
        continued.push(b'\n');
        let ClaudeTailRefresh::Extended {
            observed_tail,
            mapping,
        } = refresh_claude_tail(
            &seeded.layout,
            &seeded.observed_tail,
            &seeded.mapping,
            &tail_window_from(&seeded.mapping, &continued),
        )
        .unwrap()
        else {
            panic!("the continuation folds through the pending run");
        };
        assert_eq!(observed_tail, one_shot_tail(&continued));
        assert_eq!(
            observed_tail.accepted_end_record_count,
            seeded.observed_tail.accepted_end_record_count + 1
        );
        assert_eq!(mapping.consumed_byte_offset, continued.len() as u64);
        assert_eq!(
            mapping.consumed_record_ordinal,
            seeded.mapping.consumed_record_ordinal + 1,
            "the thinking record never counts"
        );
        assert_eq!(mapping.last_canonical_message_id.as_deref(), Some("u-3"));

        // C referencing an absent uuid the feeder never saw: the cursor
        // defers, and the full path stitches to the identical chain.
        let mut unseen = full.clone();
        unseen.extend_from_slice(record("u-3", Some("never-seen"), "user", "five").as_bytes());
        unseen.push(b'\n');
        assert!(matches!(
            refresh_claude_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &unseen),
            )
            .unwrap(),
            ClaudeTailRefresh::NeedsFullResolve
        ));
        assert_eq!(one_shot_tail(&unseen), one_shot_tail(&continued));
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
