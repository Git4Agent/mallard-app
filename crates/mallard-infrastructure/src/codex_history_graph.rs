//! Bounded `history_base` graph resolution, flattening, seeding, and
//! cursor-based tail refresh for Codex rollouts.
//!
//! A paginated rollout carries only its final segment plus a reference to the
//! physical segment it continues. This module resolves that chain base-first,
//! projects every segment through one shared canonical chain builder, fixes
//! the immutable portable boundary at the leaf's exact native cutoff, and
//! folds the portable tail. Refresh extends a saved accumulator over newly
//! appended complete records without re-reading consumed bytes; every
//! disagreement is a typed invalid outcome, never an empty tail.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use crate::codex_rollout::{
    authoritative_session_meta, structural_record_type, CodexSessionMetaRecord,
    CodexStructureAccumulator, MAX_CODEX_FORK_DEPTH,
};
use crate::conversation_codec::{CanonicalChainBuilder, ChainFeedOutcome};
use mallard_domain::{
    compute_portable_conversation_tail, conversation_chain_digests, conversation_prefix_witness,
    validate_conversation_chain, CanonicalConversationMessage, ConversationPrefixWitness,
    MessageId, PortableConversationTail, RootPhysicalMapping, RootTailLayout, TailAccumulator,
    TailPartitionKind, MAX_CONVERSATION_FILE_BYTES, PORTABLE_TAIL_CODEC_V1,
};

pub const PROBE_WINDOW_BYTES: u64 = 4_096;

/// One physical segment of a resolved logical root, base-most first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSegment {
    pub provider_thread_id: String,
    pub path: PathBuf,
    /// Bytes consumed: the child's byte cutoff for ancestors, the safe end
    /// for the leaf.
    pub consumed_byte_len: u64,
    /// Non-empty JSONL records inside the consumed bytes.
    pub consumed_record_count: u64,
    /// Canonical messages this segment contributed to the flattened chain.
    pub canonical_messages: u64,
}

/// A fully resolved and flattened logical root.
#[derive(Clone, Debug)]
pub struct ResolvedCodexRoot {
    pub chain: Vec<CanonicalConversationMessage>,
    pub partition_kind: TailPartitionKind,
    pub start_record_count: u64,
    /// Base-most first; the final entry is the append-bearing leaf.
    pub segments: Vec<ResolvedSegment>,
    pub leaf_meta: CodexSessionMetaRecord,
    pub lineage_digest: String,
    pub semantic_digest: String,
    pub leaf_file_size: u64,
    /// Position after the last complete record in the leaf.
    pub leaf_safe_end: u64,
    pub leaf_last_message_id: Option<MessageId>,
    pub leaf_synthetic_turn: Option<(String, usize)>,
}

/// Everything first-contact seeding persists, derived in one full parse.
#[derive(Clone, Debug)]
pub struct CodexSeededRoot {
    pub layout: RootTailLayout,
    pub observed_tail: PortableConversationTail,
    pub prefix_witness: Option<ConversationPrefixWitness>,
    pub mapping: RootPhysicalMapping,
    pub lineage_digest: String,
    pub semantic_digest: String,
}

/// Outcome of a cursor-based refresh over the append-bearing leaf.
#[derive(Clone, Debug)]
// The extension payload dwarfs the marker variants by design; boxing it
// would change every consumer for no behavioral gain.
#[allow(clippy::large_enum_variant)]
pub enum CodexTailRefresh {
    Unchanged,
    Extended {
        observed_tail: PortableConversationTail,
        mapping: RootPhysicalMapping,
    },
    /// The source no longer matches the checkpoint's identity contract.
    Invalid {
        reason: CodexTailInvalidReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexTailInvalidReason {
    Truncated,
    Replaced,
    MalformedAppend,
}

/// Read one rollout with the module's safety contract: a canonical no-follow
/// regular file within the bounded transcript limit.
pub fn read_rollout_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("stat Codex rollout '{}': {}", path.display(), error))?;
    if !metadata.is_file() {
        return Err(format!(
            "Codex rollout '{}' is not a regular file",
            path.display()
        ));
    }
    if metadata.len() > MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Codex rollout '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| format!("read Codex rollout '{}': {}", path.display(), error))
}

struct SegmentSource {
    path: PathBuf,
    bytes: Vec<u8>,
    consumed_byte_len: u64,
    declared_record_count: Option<u64>,
    meta: CodexSessionMetaRecord,
}

/// Resolve the complete `history_base` chain of one append-bearing leaf and
/// flatten it into one canonical root chain. `base_lookup` maps a base
/// thread ID to its rollout path from the caller's bounded provider
/// inventory; an unresolved base blocks.
pub fn resolve_codex_root(
    leaf_path: &Path,
    base_lookup: &dyn Fn(&str) -> Result<Option<PathBuf>, String>,
) -> Result<ResolvedCodexRoot, String> {
    let leaf_bytes = read_rollout_bounded(leaf_path)?;
    let leaf_file_size = leaf_bytes.len() as u64;
    let leaf_safe_end = safe_record_end(&leaf_bytes);
    if leaf_safe_end == 0 {
        return Err(format!(
            "Codex rollout '{}' has no complete records",
            leaf_path.display()
        ));
    }

    // Walk leaf -> base, slicing each ancestor at its child's exact cutoffs.
    let mut segments = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    let leaf_meta = authoritative_session_meta(&leaf_bytes).map_err(|error| error.to_string())?;
    let mut next_reference = leaf_meta.history_base.clone();
    visited.insert(leaf_meta.id.clone());
    segments.push(SegmentSource {
        path: leaf_path.to_path_buf(),
        consumed_byte_len: leaf_safe_end,
        declared_record_count: None,
        meta: leaf_meta.clone(),
        bytes: {
            let mut bytes = leaf_bytes;
            bytes.truncate(leaf_safe_end as usize);
            bytes
        },
    });
    while let Some(reference) = next_reference {
        if segments.len() > MAX_CODEX_FORK_DEPTH {
            return Err("Codex history chain exceeds the supported depth".to_string());
        }
        if !visited.insert(reference.thread_id.clone()) {
            return Err("Codex history chain contains a cycle".to_string());
        }
        let path = base_lookup(&reference.thread_id)?.ok_or_else(|| {
            format!(
                "Codex history base '{}' has no resolvable rollout",
                reference.thread_id
            )
        })?;
        let bytes = read_rollout_bounded(&path)?;
        if reference.end_byte_offset > bytes.len() as u64 {
            return Err(format!(
                "Codex history base '{}' is shorter than its declared cutoff",
                reference.thread_id
            ));
        }
        let cutoff = reference.end_byte_offset as usize;
        if cutoff == 0 || bytes[cutoff - 1] != b'\n' {
            return Err(format!(
                "Codex history base '{}' cutoff does not land on a record boundary",
                reference.thread_id
            ));
        }
        let mut slice = bytes;
        slice.truncate(cutoff);
        let meta = authoritative_session_meta(&slice).map_err(|error| error.to_string())?;
        if meta.id != reference.thread_id {
            return Err(format!(
                "Codex history base '{}' resolved to rollout '{}'",
                reference.thread_id, meta.id
            ));
        }
        next_reference = meta.history_base.clone();
        segments.push(SegmentSource {
            path,
            consumed_byte_len: reference.end_byte_offset,
            declared_record_count: Some(reference.end_ordinal_exclusive),
            meta,
            bytes: slice,
        });
    }
    segments.reverse();

    // One shared canonical projection across every segment, base -> leaf.
    let mut builder = CanonicalChainBuilder::new();
    let mut resolved_segments = Vec::with_capacity(segments.len());
    let mut boundary = 0_u64;
    let leaf_index = segments.len() - 1;
    for (index, segment) in segments.into_iter().enumerate() {
        let mut structure = CodexStructureAccumulator::new(Some(&segment.meta.id));
        let appended_before = builder.appended_len() as u64;
        let mut record_count = 0_u64;
        for raw in segment.bytes.split(|byte| *byte == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            record_count += 1;
            let value: serde_json::Value = serde_json::from_slice(raw).map_err(|_| {
                format!(
                    "Codex segment '{}' contains malformed JSON",
                    segment.meta.id
                )
            })?;
            structure
                .feed(&value)
                .map_err(|error| format!("Codex segment '{}': {}", segment.meta.id, error))?;
            builder
                .feed(value)
                .map_err(|reason| format!("Codex segment '{}': {:?}", segment.meta.id, reason))?;
        }
        structure
            .finalize()
            .map_err(|error| format!("Codex segment '{}': {}", segment.meta.id, error))?;
        if let Some(declared) = segment.declared_record_count {
            if declared != record_count {
                return Err(format!(
                    "Codex history base '{}' cutoffs disagree: declared {} records, found {}",
                    segment.meta.id, declared, record_count
                ));
            }
        }
        let canonical_messages = builder.appended_len() as u64 - appended_before;
        if index < leaf_index {
            boundary = builder.appended_len() as u64;
        }
        resolved_segments.push(ResolvedSegment {
            provider_thread_id: segment.meta.id.clone(),
            path: segment.path,
            consumed_byte_len: segment.consumed_byte_len,
            consumed_record_count: record_count,
            canonical_messages,
        });
    }

    let leaf_last_message_id = builder.last_message_id().cloned();
    let leaf_synthetic_turn = builder.synthetic_turn_resume();
    let chain = builder.finish();
    validate_conversation_chain(&chain)?;
    let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain)?;
    let partition_kind = if leaf_meta.history_base.is_some() {
        TailPartitionKind::FixedPaginatedTail
    } else {
        TailPartitionKind::WholeThread
    };
    if partition_kind == TailPartitionKind::WholeThread && boundary != 0 {
        return Err("Codex root without a native base cannot have a boundary".to_string());
    }
    Ok(ResolvedCodexRoot {
        chain,
        partition_kind,
        start_record_count: boundary,
        segments: resolved_segments,
        leaf_meta,
        lineage_digest,
        semantic_digest,
        leaf_file_size,
        leaf_safe_end,
        leaf_last_message_id,
        leaf_synthetic_turn,
    })
}

/// The effective tail partition for observing a resolved root under a
/// stored binding layout. A stored layout must equal the native derivation,
/// with one exception: a `FixedPaginatedTail` layout imported at Pull
/// install stays authoritative over a null-base local file, whose native
/// reading is `WholeThread` at zero — the boundary comes from the verified
/// cloud value and is never re-derived from the file's null native base.
/// Any other disagreement is corruption, never a reseed.
pub fn effective_tail_partition(
    layout: Option<&RootTailLayout>,
    resolved: &ResolvedCodexRoot,
) -> Result<(TailPartitionKind, u64), String> {
    let Some(layout) = layout else {
        return Ok((resolved.partition_kind, resolved.start_record_count));
    };
    if layout.partition_kind == resolved.partition_kind
        && layout.start_record_count == resolved.start_record_count
    {
        return Ok((resolved.partition_kind, resolved.start_record_count));
    }
    if layout.partition_kind == TailPartitionKind::FixedPaginatedTail
        && resolved.partition_kind == TailPartitionKind::WholeThread
        && resolved.start_record_count == 0
        && layout.start_record_count <= resolved.chain.len() as u64
    {
        return Ok((
            TailPartitionKind::FixedPaginatedTail,
            layout.start_record_count,
        ));
    }
    Err("Codex root derivation disagrees with the stored tail layout".to_string())
}

/// Derive the layout, observed tail, witness, and physical mapping from one
/// resolved root, folding under the stored binding layout when one exists
/// (imported paginated layouts stay authoritative over null-base files).
/// Observation-class work: nothing here builds an artifact or touches cloud
/// state.
pub fn seed_codex_root(
    resolved: &ResolvedCodexRoot,
    leaf_bytes: &[u8],
    layout: Option<&RootTailLayout>,
) -> Result<CodexSeededRoot, String> {
    let (partition_kind, start_record_count) = effective_tail_partition(layout, resolved)?;
    let observed_tail =
        compute_portable_conversation_tail(partition_kind, start_record_count, &resolved.chain)?;
    let prefix_witness = conversation_prefix_witness(&resolved.chain, start_record_count)?;
    let leaf = resolved
        .segments
        .last()
        .ok_or_else(|| "resolved Codex root has no segments".to_string())?;
    let consumed = resolved.leaf_safe_end;
    let probe_window_len = consumed.min(PROBE_WINDOW_BYTES);
    let window_start = (consumed - probe_window_len) as usize;
    let probe_window_sha256 = hex_sha256(&leaf_bytes[window_start..consumed as usize]);
    let mapping = RootPhysicalMapping {
        provider_thread_id: resolved.leaf_meta.id.clone(),
        root_path: leaf
            .path
            .to_str()
            .ok_or_else(|| "Codex rollout path is not valid UTF-8".to_string())?
            .to_string(),
        source_file_size: resolved.leaf_file_size,
        consumed_byte_offset: consumed,
        consumed_record_ordinal: leaf.consumed_record_count,
        probe_window_len,
        probe_window_sha256,
        last_canonical_message_id: resolved
            .leaf_last_message_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        last_synthetic_turn_id: resolved
            .leaf_synthetic_turn
            .as_ref()
            .map(|(turn, _)| turn.clone()),
        next_synthetic_turn_ordinal: resolved
            .leaf_synthetic_turn
            .as_ref()
            .map(|(_, next)| *next as u64)
            .unwrap_or(0),
    };
    let layout = RootTailLayout {
        codec: PORTABLE_TAIL_CODEC_V1.to_string(),
        partition_kind,
        start_record_count,
    };
    Ok(CodexSeededRoot {
        layout,
        observed_tail,
        prefix_witness,
        mapping,
        lineage_digest: resolved.lineage_digest.clone(),
        semantic_digest: resolved.semantic_digest.clone(),
    })
}

/// Extend a saved accumulator over newly appended complete records. Reads no
/// byte before the cursor beyond the bounded probe window; `WholeThread`
/// callers recompute through `resolve_codex_root` instead.
pub fn refresh_codex_tail(
    layout: &RootTailLayout,
    observed_tail: &PortableConversationTail,
    mapping: &RootPhysicalMapping,
    current_bytes: &[u8],
) -> Result<CodexTailRefresh, String> {
    let current_len = current_bytes.len() as u64;
    if current_len < mapping.consumed_byte_offset {
        return Ok(CodexTailRefresh::Invalid {
            reason: CodexTailInvalidReason::Truncated,
        });
    }
    let consumed = mapping.consumed_byte_offset as usize;
    let window = mapping.probe_window_len as usize;
    let probe = hex_sha256(&current_bytes[consumed - window..consumed]);
    if probe != mapping.probe_window_sha256 {
        return Ok(CodexTailRefresh::Invalid {
            reason: CodexTailInvalidReason::Replaced,
        });
    }
    let safe_end = consumed as u64 + safe_record_end(&current_bytes[consumed..]);
    if safe_end == mapping.consumed_byte_offset {
        return Ok(CodexTailRefresh::Unchanged);
    }

    let mut builder = CanonicalChainBuilder::resume(
        mapping
            .last_canonical_message_id
            .as_ref()
            .map(|id| MessageId::parse(id.clone()))
            .transpose()?,
        mapping
            .last_synthetic_turn_id
            .as_ref()
            .map(|turn| (turn.clone(), mapping.next_synthetic_turn_ordinal as usize)),
    );
    let mut accumulator = TailAccumulator::resume(
        layout.partition_kind,
        layout.start_record_count,
        observed_tail.accepted_end_record_count,
        &observed_tail.accepted_tail_state,
    )?;
    let mut appended_records = 0_u64;
    for raw in current_bytes[consumed..safe_end as usize].split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        appended_records += 1;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(raw) else {
            return Ok(CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::MalformedAppend,
            });
        };
        // A structural identity record appearing after the consumed cursor
        // means the file is no longer the same append-only segment.
        if structural_record_type(&value) == Some("session_meta") {
            return Ok(CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::Replaced,
            });
        }
        match builder.feed(value) {
            Ok(ChainFeedOutcome::Appended) => {
                let message = builder
                    .last_appended()
                    .cloned()
                    .ok_or_else(|| "chain builder lost its appended message".to_string())?;
                accumulator.fold_message(&message)?;
            }
            Ok(_) => {}
            Err(_) => {
                return Ok(CodexTailRefresh::Invalid {
                    reason: CodexTailInvalidReason::MalformedAppend,
                });
            }
        }
    }

    let probe_window_len = safe_end.min(PROBE_WINDOW_BYTES);
    let window_start = (safe_end - probe_window_len) as usize;
    let new_mapping = RootPhysicalMapping {
        provider_thread_id: mapping.provider_thread_id.clone(),
        root_path: mapping.root_path.clone(),
        source_file_size: current_len,
        consumed_byte_offset: safe_end,
        consumed_record_ordinal: mapping.consumed_record_ordinal + appended_records,
        probe_window_len,
        probe_window_sha256: hex_sha256(&current_bytes[window_start..safe_end as usize]),
        last_canonical_message_id: builder
            .last_message_id()
            .map(|id| id.as_str().to_string())
            .or_else(|| mapping.last_canonical_message_id.clone()),
        last_synthetic_turn_id: builder
            .synthetic_turn_resume()
            .map(|(turn, _)| turn)
            .or_else(|| mapping.last_synthetic_turn_id.clone()),
        next_synthetic_turn_ordinal: builder
            .synthetic_turn_resume()
            .map(|(_, next)| next as u64)
            .unwrap_or(mapping.next_synthetic_turn_ordinal),
    };
    let observed = PortableConversationTail {
        codec: layout.codec.clone(),
        partition_kind: layout.partition_kind,
        start_record_count: layout.start_record_count,
        accepted_end_record_count: accumulator.folded_end_record_count(),
        accepted_tail_state: accumulator.state_hex(),
    };
    observed.validate()?;
    Ok(CodexTailRefresh::Extended {
        observed_tail: observed,
        mapping: new_mapping,
    })
}

/// Position immediately after the final newline: the safe record end.
fn safe_record_end(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|position| position as u64 + 1)
        .unwrap_or(0)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn meta_line(id: &str, base: Option<(&str, u64, u64)>) -> String {
        let mut payload = json!({"id": id, "cwd": "/tmp/project", "history_mode": "paginated"});
        if let Some((thread, ordinal, byte)) = base {
            payload["history_base"] = json!({
                "thread_id": thread,
                "end_ordinal_exclusive": ordinal,
                "end_byte_offset": byte,
            });
        }
        serde_json::to_string(&json!({"type": "session_meta", "payload": payload})).unwrap()
    }

    fn message_line(id: &str, text: &str) -> String {
        serde_json::to_string(&json!({
            "type": "response_item",
            "payload": {"type": "message", "id": id, "content": text},
        }))
        .unwrap()
    }

    struct Graph {
        directory: tempfile::TempDir,
        files: BTreeMap<String, PathBuf>,
    }

    impl Graph {
        fn write(&mut self, thread: &str, lines: &[String]) -> PathBuf {
            let path = self.directory.path().join(format!("{thread}.jsonl"));
            fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
            self.files.insert(thread.to_string(), path.clone());
            path
        }

        fn lookup(&self) -> impl Fn(&str) -> Result<Option<PathBuf>, String> + '_ {
            |thread| Ok(self.files.get(thread).cloned())
        }
    }

    fn graph() -> Graph {
        Graph {
            directory: tempfile::tempdir().unwrap(),
            files: BTreeMap::new(),
        }
    }

    /// Build the P0/P1/P2 lifecycle: P0 carries m1,m2; P1 continues P0 with
    /// m3; P2 continues P1 with m4,m5. Returns the leaf path.
    fn nested_graph(graph: &mut Graph) -> PathBuf {
        let p0_lines = vec![
            meta_line("p0", None),
            message_line("m1", "one"),
            message_line("m2", "two"),
        ];
        let p0 = graph.write("p0", &p0_lines);
        let p0_len = fs::metadata(&p0).unwrap().len();
        let p1_lines = vec![
            meta_line("p1", Some(("p0", 3, p0_len))),
            message_line("m3", "three"),
        ];
        let p1 = graph.write("p1", &p1_lines);
        let p1_len = fs::metadata(&p1).unwrap().len();
        let p2_lines = vec![
            meta_line("p2", Some(("p1", 2, p1_len))),
            message_line("m4", "four"),
            message_line("m5", "five"),
        ];
        graph.write("p2", &p2_lines)
    }

    #[test]
    fn nested_graph_resolves_to_one_boundary_and_matching_folds() {
        let mut graph = graph();
        let leaf = nested_graph(&mut graph);
        let resolved = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        assert_eq!(resolved.chain.len(), 5);
        assert_eq!(resolved.start_record_count, 3);
        assert_eq!(
            resolved.partition_kind,
            TailPartitionKind::FixedPaginatedTail
        );
        assert_eq!(
            resolved
                .segments
                .iter()
                .map(|segment| segment.provider_thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["p0", "p1", "p2"]
        );

        // Seeding derives the same tail the domain one-shot computes, and a
        // witness over exactly the ancestor prefix.
        let leaf_bytes = fs::read(&leaf).unwrap();
        let seeded = seed_codex_root(&resolved, &leaf_bytes, None).unwrap();
        let one_shot = compute_portable_conversation_tail(
            TailPartitionKind::FixedPaginatedTail,
            3,
            &resolved.chain,
        )
        .unwrap();
        assert_eq!(seeded.observed_tail, one_shot);
        let witness = seeded.prefix_witness.unwrap();
        let (lineage, semantic) = conversation_chain_digests(&resolved.chain[..3]).unwrap();
        assert_eq!(witness.lineage_digest, lineage);
        assert_eq!(witness.semantic_digest, semantic);
        assert_eq!(seeded.layout.start_record_count, 3);
        assert_eq!(seeded.mapping.consumed_byte_offset, resolved.leaf_safe_end);
    }

    #[test]
    fn cutoff_disagreements_block() {
        let mut graph = graph();
        let p0_lines = vec![meta_line("p0", None), message_line("m1", "one")];
        let p0 = graph.write("p0", &p0_lines);
        let p0_len = fs::metadata(&p0).unwrap().len();

        // Wrong ordinal.
        let leaf = graph.write(
            "bad-ordinal",
            &[meta_line("bad-ordinal", Some(("p0", 3, p0_len)))],
        );
        let error = resolve_codex_root(&leaf, &graph.lookup()).unwrap_err();
        assert!(error.contains("cutoffs disagree"), "{error}");

        // Mid-record byte cutoff.
        let leaf = graph.write(
            "bad-byte",
            &[meta_line("bad-byte", Some(("p0", 2, p0_len - 3)))],
        );
        let error = resolve_codex_root(&leaf, &graph.lookup()).unwrap_err();
        assert!(error.contains("record boundary"), "{error}");

        // Unresolvable base.
        let leaf = graph.write("orphan", &[meta_line("orphan", Some(("gone", 1, 10)))]);
        let error = resolve_codex_root(&leaf, &graph.lookup()).unwrap_err();
        assert!(error.contains("no resolvable rollout"), "{error}");
    }

    #[test]
    fn rootless_leaf_is_whole_thread_at_zero() {
        let mut graph = graph();
        let leaf = graph.write(
            "solo",
            &[
                meta_line("solo", None),
                message_line("m1", "one"),
                message_line("m2", "two"),
            ],
        );
        let resolved = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        assert_eq!(resolved.partition_kind, TailPartitionKind::WholeThread);
        assert_eq!(resolved.start_record_count, 0);
        assert_eq!(resolved.chain.len(), 2);
    }

    #[test]
    fn refresh_extends_the_saved_accumulator_to_the_one_shot_state() {
        let mut graph = graph();
        let leaf = nested_graph(&mut graph);
        let resolved = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        let leaf_bytes = fs::read(&leaf).unwrap();
        let seeded = seed_codex_root(&resolved, &leaf_bytes, None).unwrap();

        // Unchanged file: no read past the cursor.
        matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &leaf_bytes,
            )
            .unwrap(),
            CodexTailRefresh::Unchanged
        )
        .then_some(())
        .unwrap();

        // Append m6 and refresh: the incremental state equals the one-shot
        // over the re-resolved chain.
        let mut appended = leaf_bytes.clone();
        appended.extend_from_slice(message_line("m6", "six").as_bytes());
        appended.push(b'\n');
        fs::write(&leaf, &appended).unwrap();
        let CodexTailRefresh::Extended {
            observed_tail,
            mapping,
        } = refresh_codex_tail(
            &seeded.layout,
            &seeded.observed_tail,
            &seeded.mapping,
            &appended,
        )
        .unwrap()
        else {
            panic!("expected extension");
        };
        let re_resolved = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        let one_shot = compute_portable_conversation_tail(
            TailPartitionKind::FixedPaginatedTail,
            3,
            &re_resolved.chain,
        )
        .unwrap();
        assert_eq!(observed_tail, one_shot);
        assert_eq!(mapping.consumed_byte_offset, appended.len() as u64);
        assert_eq!(mapping.last_canonical_message_id.as_deref(), Some("m6"));

        // Truncation and replacement are typed invalids, never empty tails.
        let truncated = &appended[..seeded.mapping.consumed_byte_offset as usize - 10];
        matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                truncated
            )
            .unwrap(),
            CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::Truncated
            }
        )
        .then_some(())
        .unwrap();
        let mut replaced = leaf_bytes.clone();
        let window_start = replaced.len() - 20;
        replaced[window_start] = b'X';
        matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &replaced,
            )
            .unwrap(),
            CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::Replaced
            }
        )
        .then_some(())
        .unwrap();

        // A structural identity record after the cursor is a replacement.
        let mut meta_appended = leaf_bytes.clone();
        meta_appended.extend_from_slice(meta_line("p2-reborn", None).as_bytes());
        meta_appended.push(b'\n');
        matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &meta_appended,
            )
            .unwrap(),
            CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::Replaced
            }
        )
        .then_some(())
        .unwrap();
    }

    /// Plan section 13's cross-machine lifecycle over the real machinery:
    /// A pushes a paginated root, B pulls the standalone file and maps the
    /// same fixed start, divergence is pinned and resolved by keep-both, the
    /// tracked thread republishes from the pulled machine with the imported
    /// partition preserved, A pulls the extension as CloudOutstanding, and
    /// the preserved branch publishes independently under its own layout.
    #[test]
    fn cross_machine_paginated_tail_survives_pull_divergence_and_republish() {
        use crate::codex_standalone_builder::{build_standalone_codex_root, StandaloneBuildMode};
        use mallard_domain::{
            compare_conversation_tail, PortableConversationTail, RootTailLayout, SyncCurrency,
            TailAccumulator, TailClassification, TailComparisonEvidence, TailSideEvidence,
            PORTABLE_TAIL_CODEC_V1,
        };

        fn evidence(
            accepted: TailSideEvidence,
            local: TailSideEvidence,
            cloud: TailSideEvidence,
        ) -> TailComparisonEvidence {
            TailComparisonEvidence {
                accepted,
                local,
                cloud,
                local_provenance_verified: true,
                accepted_witness_verified: true,
                cloud_ancestry_verified: true,
                coordinates_verified: true,
            }
        }
        let present = |tail: &PortableConversationTail| TailSideEvidence::Present(tail.clone());
        let paginated_layout = |start: u64| RootTailLayout {
            codec: PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: TailPartitionKind::FixedPaginatedTail,
            start_record_count: start,
        };
        let append = |path: &PathBuf, id: &str, body: &str| {
            let mut bytes = fs::read(path).unwrap();
            bytes.extend_from_slice(message_line(id, body).as_bytes());
            bytes.push(b'\n');
            fs::write(path, &bytes).unwrap();
        };

        // Machine A: base rollout b1..b5, leaf continuing at base@5 with
        // m1,m2,m3.
        let mut machine_a = graph();
        let base_lines: Vec<String> = std::iter::once(meta_line("base", None))
            .chain((1..=5).map(|index| message_line(&format!("b{index}"), "base")))
            .collect();
        let base = machine_a.write("base", &base_lines);
        let base_len = fs::metadata(&base).unwrap().len();
        let a_leaf = machine_a.write(
            "a-leaf",
            &[
                meta_line("a-leaf", Some(("base", 6, base_len))),
                message_line("m1", "one"),
                message_line("m2", "two"),
                message_line("m3", "three"),
            ],
        );
        let resolved_a = resolve_codex_root(&a_leaf, &machine_a.lookup()).unwrap();
        assert_eq!(
            resolved_a.partition_kind,
            TailPartitionKind::FixedPaginatedTail
        );
        assert_eq!(resolved_a.start_record_count, 5);
        assert_eq!(resolved_a.chain.len(), 8);

        // A pushes: the artifact folds the paginated tail it publishes.
        let push_one =
            build_standalone_codex_root(&resolved_a, &StandaloneBuildMode::PreserveIdentity, None)
                .unwrap();
        let cloud_v1 = push_one.portable_tail.clone();
        assert_eq!(cloud_v1.start_record_count, 5);
        assert_eq!(cloud_v1.accepted_end_record_count, 8);

        // Machine B pulls: reidentify the staged standalone download, then
        // map the same fixed start from the verified cloud value. The
        // null-base file itself resolves as WholeThread at zero and is never
        // used to re-derive the boundary.
        let mut machine_b = graph();
        let staged = machine_b.directory.path().join("staged.jsonl");
        fs::write(&staged, &push_one.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let resolved_staged = resolve_codex_root(&staged, &no_bases).unwrap();
        assert_eq!(
            resolved_staged.partition_kind,
            TailPartitionKind::WholeThread
        );
        assert_eq!(resolved_staged.chain, resolved_a.chain);
        let imported = paginated_layout(cloud_v1.start_record_count);
        let pulled = build_standalone_codex_root(
            &resolved_staged,
            &StandaloneBuildMode::Reidentify {
                root_provider_thread_id: "b-local".to_string(),
                target_cwd: "/tmp/machine-b".to_string(),
            },
            Some(&imported),
        )
        .unwrap();
        assert_eq!(pulled.portable_tail, cloud_v1);
        let b_local = machine_b.directory.path().join("b-local.jsonl");
        fs::write(&b_local, &pulled.root_bytes).unwrap();
        machine_b
            .files
            .insert("b-local".to_string(), b_local.clone());

        // B observes under the imported layout: the seeded value equals the
        // verified cloud value exactly, and B is in sync under its pin.
        let resolved_b = resolve_codex_root(&b_local, &machine_b.lookup()).unwrap();
        let b_bytes = fs::read(&b_local).unwrap();
        let seeded_b = seed_codex_root(&resolved_b, &b_bytes, Some(&imported)).unwrap();
        assert_eq!(seeded_b.observed_tail, cloud_v1);
        assert_eq!(seeded_b.layout, imported);
        assert_eq!(
            compare_conversation_tail(evidence(
                present(&cloud_v1),
                present(&seeded_b.observed_tail),
                present(&cloud_v1),
            )),
            TailClassification::Sync(SyncCurrency::Current)
        );

        // A appends m4: LocalOutstanding under its pin, then pushed.
        append(&a_leaf, "m4", "four");
        let resolved_a2 = resolve_codex_root(&a_leaf, &machine_a.lookup()).unwrap();
        let push_two =
            build_standalone_codex_root(&resolved_a2, &StandaloneBuildMode::PreserveIdentity, None)
                .unwrap();
        let cloud_v2 = push_two.portable_tail.clone();
        assert_eq!(
            compare_conversation_tail(evidence(
                present(&cloud_v1),
                present(&cloud_v2),
                present(&cloud_v1),
            )),
            TailClassification::LocalOutstanding
        );

        // Map-and-extend identity across machines: resuming from the pulled
        // accumulator state and folding m4 reproduces A's one-shot value.
        let mut resumed = TailAccumulator::resume(
            TailPartitionKind::FixedPaginatedTail,
            5,
            8,
            &cloud_v1.accepted_tail_state,
        )
        .unwrap();
        resumed.fold_message(&resolved_a2.chain[8]).unwrap();
        assert_eq!(resumed.state_hex(), cloud_v2.accepted_tail_state);

        // B appends a conflicting m5 before seeing the new head: a pinned
        // divergence, not an unprovable pair.
        append(&b_local, "m5-b", "five on b");
        let resolved_b2 = resolve_codex_root(&b_local, &machine_b.lookup()).unwrap();
        let b_bytes2 = fs::read(&b_local).unwrap();
        let diverged_b = seed_codex_root(&resolved_b2, &b_bytes2, Some(&imported)).unwrap();
        assert_eq!(
            compare_conversation_tail(evidence(
                present(&cloud_v1),
                present(&diverged_b.observed_tail),
                present(&cloud_v2),
            )),
            TailClassification::Divergent
        );

        // Keep-both: the preserved branch keeps B's bytes untouched under the
        // unchanged local layout; the tracked thread re-materializes the
        // cloud graph under a fresh alias with the remote layout.
        let preserved_bytes = fs::read(&b_local).unwrap();
        let staged_two = machine_b.directory.path().join("staged-two.jsonl");
        fs::write(&staged_two, &push_two.root_bytes).unwrap();
        let resolved_staged_two = resolve_codex_root(&staged_two, &no_bases).unwrap();
        let tracked = build_standalone_codex_root(
            &resolved_staged_two,
            &StandaloneBuildMode::Reidentify {
                root_provider_thread_id: "b-tracked".to_string(),
                target_cwd: "/tmp/machine-b".to_string(),
            },
            Some(&imported),
        )
        .unwrap();
        assert_eq!(tracked.portable_tail, cloud_v2);
        let b_tracked = machine_b.directory.path().join("b-tracked.jsonl");
        fs::write(&b_tracked, &tracked.root_bytes).unwrap();
        machine_b
            .files
            .insert("b-tracked".to_string(), b_tracked.clone());
        assert_eq!(fs::read(&b_local).unwrap(), preserved_bytes);

        // B appends m6 to the tracked thread and republishes from the pulled
        // machine: the artifact preserves the imported partition instead of
        // moving the start to the null native base.
        append(&b_tracked, "m6", "six");
        let resolved_tracked = resolve_codex_root(&b_tracked, &machine_b.lookup()).unwrap();
        let tracked_bytes = fs::read(&b_tracked).unwrap();
        let seeded_tracked =
            seed_codex_root(&resolved_tracked, &tracked_bytes, Some(&imported)).unwrap();
        assert_eq!(
            compare_conversation_tail(evidence(
                present(&cloud_v2),
                present(&seeded_tracked.observed_tail),
                present(&cloud_v2),
            )),
            TailClassification::LocalOutstanding
        );
        let push_three = build_standalone_codex_root(
            &resolved_tracked,
            &StandaloneBuildMode::PreserveIdentity,
            Some(&imported),
        )
        .unwrap();
        let cloud_v3 = push_three.portable_tail.clone();
        assert_eq!(cloud_v3, seeded_tracked.observed_tail);
        assert_eq!(
            cloud_v3.partition_kind,
            TailPartitionKind::FixedPaginatedTail
        );
        assert_eq!(cloud_v3.start_record_count, 5);
        assert_eq!(cloud_v3.accepted_end_record_count, 10);

        // A pulls the extension: CloudOutstanding with the partition
        // preserved, and the materialized value refolds to the head.
        assert_eq!(
            compare_conversation_tail(evidence(
                present(&cloud_v2),
                present(&cloud_v2),
                present(&cloud_v3),
            )),
            TailClassification::CloudOutstanding
        );
        let staged_three = machine_a.directory.path().join("staged-three.jsonl");
        fs::write(&staged_three, &push_three.root_bytes).unwrap();
        let resolved_staged_three = resolve_codex_root(&staged_three, &no_bases).unwrap();
        let a_adopted = build_standalone_codex_root(
            &resolved_staged_three,
            &StandaloneBuildMode::Reidentify {
                root_provider_thread_id: "a-tracked".to_string(),
                target_cwd: "/tmp/machine-a".to_string(),
            },
            Some(&imported),
        )
        .unwrap();
        assert_eq!(a_adopted.portable_tail, cloud_v3);

        // The preserved branch is its own thread: locally present, missing
        // from the cloud, and its first Push publishes the preserved layout
        // with no family metadata.
        assert_eq!(
            compare_conversation_tail(evidence(
                TailSideEvidence::ConfirmedAbsent,
                present(&diverged_b.observed_tail),
                TailSideEvidence::ConfirmedAbsent,
            )),
            TailClassification::MissingCloud
        );
        let preserved_push = build_standalone_codex_root(
            &resolved_b2,
            &StandaloneBuildMode::PreserveIdentity,
            Some(&imported),
        )
        .unwrap();
        assert_eq!(preserved_push.portable_tail, diverged_b.observed_tail);
        assert_eq!(preserved_push.portable_tail.start_record_count, 5);
        assert!(preserved_push.subagents.is_empty());
        assert!(preserved_push.id_map.is_empty());
    }
}
