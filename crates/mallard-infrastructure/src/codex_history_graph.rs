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
    validate_conversation_chain, CanonicalConversationMessage, ConversationCheckpoint,
    ConversationPrefixWitness, MessageId, PhysicalMappingState, PortableConversationTail,
    RootPhysicalMapping, RootTailLayout, TailAccumulator, TailPartitionKind,
    MAX_CONVERSATION_FILE_BYTES, PORTABLE_TAIL_CODEC_V1,
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
    /// Appended structural metadata needs the bounded whole-root validator.
    /// It may be a legitimate repeated active identity or evidence that the
    /// rollout was replaced; the cursor alone cannot distinguish them.
    NeedsFullResolve,
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
    use std::io::Read;

    let mut file = open_rollout_no_follow(path)?;
    let before = file
        .metadata()
        .map_err(|error| format!("inspect Codex rollout '{}': {}", path.display(), error))?;
    if !before.is_file() {
        return Err(format!(
            "Codex rollout '{}' is not a regular file",
            path.display()
        ));
    }
    if before.len() > MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Codex rollout '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take(MAX_CONVERSATION_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read Codex rollout '{}': {}", path.display(), error))?;
    let after = file
        .metadata()
        .map_err(|error| format!("reinspect Codex rollout '{}': {}", path.display(), error))?;
    if bytes.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Codex rollout '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    if after.len() != before.len() || bytes.len() as u64 != before.len() {
        return Err(format!(
            "Codex rollout '{}' changed while it was read",
            path.display()
        ));
    }
    crate::sync_metrics::record_transcript_bytes_read(path, bytes.len() as u64);
    Ok(bytes)
}

/// Prove that a retained copy-on-write source still contains exactly the
/// bytes pinned when its binding moved. Only checkpoints whose raw pin spans
/// the complete physical file qualify; partial trailing records and older
/// cursor-only pins deliberately return `false` so later provider work can
/// never be hidden on incomplete evidence.
pub fn retained_root_is_byte_identical(
    path: &Path,
    checkpoint: &ConversationCheckpoint,
) -> Result<bool, String> {
    use std::io::Read;

    let PhysicalMappingState::Ready {
        root_mapping,
        raw_range_pin,
    } = &checkpoint.physical
    else {
        return Ok(false);
    };
    if Path::new(&root_mapping.root_path) != path
        || root_mapping.consumed_byte_offset != root_mapping.source_file_size
        || raw_range_pin.start_byte_offset != 0
        || raw_range_pin.end_byte_offset != root_mapping.source_file_size
    {
        return Ok(false);
    }
    let mut file = open_rollout_no_follow(path)?;
    let before = file
        .metadata()
        .map_err(|error| format!("inspect retained Codex rollout: {error}"))?;
    if !before.is_file()
        || before.len() != root_mapping.source_file_size
        || before.len() > MAX_CONVERSATION_FILE_BYTES
    {
        return Ok(false);
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take(root_mapping.source_file_size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read retained Codex rollout: {error}"))?;
    let after = file
        .metadata()
        .map_err(|error| format!("reinspect retained Codex rollout: {error}"))?;
    if after.len() != before.len() || bytes.len() as u64 != before.len() {
        return Ok(false);
    }
    crate::sync_metrics::record_transcript_bytes_read(path, bytes.len() as u64);
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(digest == raw_range_pin.range_sha256)
}

/// One bounded read of a rollout tail: the saved probe window plus every
/// byte from the consumed cursor through the bounded EOF, addressed by the
/// absolute file offset the slice starts at.
#[derive(Clone, Debug)]
pub struct RolloutTailWindow {
    pub file_len: u64,
    pub window_offset: u64,
    pub bytes: Vec<u8>,
}

/// How many times a moving-EOF read retries before failing closed.
const TAIL_WINDOW_READ_RETRIES: usize = 3;

fn open_rollout_no_follow(path: &Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|error| format!("open Codex rollout '{}': {}", path.display(), error))
}

/// Read a saved cursor's probe window and appended tail without reading the
/// consumed prefix: open the exact path without following a final symlink,
/// require a regular file within the transcript bound, read from the probe
/// window's start through EOF, and re-stat the open handle afterwards,
/// retrying a bounded number of times while the EOF is moving. A file
/// already shorter than the cursor returns an empty window whose `file_len`
/// lets `refresh_codex_tail` classify the truncation.
pub fn read_rollout_tail_window(
    path: &Path,
    mapping: &RootPhysicalMapping,
) -> Result<RolloutTailWindow, String> {
    use std::io::{Read, Seek, SeekFrom};
    let window_offset = mapping.consumed_byte_offset - mapping.probe_window_len;
    let file = open_rollout_no_follow(path)?;
    let handle_meta = file
        .metadata()
        .map_err(|error| format!("stat Codex rollout '{}': {}", path.display(), error))?;
    if !handle_meta.is_file() {
        return Err(format!(
            "Codex rollout '{}' is not a regular file",
            path.display()
        ));
    }
    let mut file = file;
    let mut attempts = 0;
    loop {
        let len_before = file
            .metadata()
            .map_err(|error| format!("stat Codex rollout '{}': {}", path.display(), error))?
            .len();
        if len_before > MAX_CONVERSATION_FILE_BYTES {
            return Err(format!(
                "Codex rollout '{}' exceeds the bounded transcript limit",
                path.display()
            ));
        }
        if len_before < mapping.consumed_byte_offset {
            return Ok(RolloutTailWindow {
                file_len: len_before,
                window_offset,
                bytes: Vec::new(),
            });
        }
        file.seek(SeekFrom::Start(window_offset))
            .map_err(|error| format!("seek Codex rollout '{}': {}", path.display(), error))?;
        let expected = len_before - window_offset;
        let mut bytes = Vec::with_capacity(expected as usize);
        (&file)
            .take(expected)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read Codex rollout '{}': {}", path.display(), error))?;
        let len_after = file
            .metadata()
            .map_err(|error| format!("stat Codex rollout '{}': {}", path.display(), error))?
            .len();
        if len_after == len_before && bytes.len() as u64 == expected {
            crate::sync_metrics::record_probe_bytes_read(path, mapping.probe_window_len);
            crate::sync_metrics::record_append_bytes_read(
                path,
                expected.saturating_sub(mapping.probe_window_len),
            );
            return Ok(RolloutTailWindow {
                file_len: len_before,
                window_offset,
                bytes,
            });
        }
        attempts += 1;
        if attempts > TAIL_WINDOW_READ_RETRIES {
            return Err(format!(
                "Codex rollout '{}' kept changing during the bounded tail read",
                path.display()
            ));
        }
    }
}

/// Read one absolute byte range of a rollout under the same no-follow
/// contract, for re-proving an accepted-to-current raw interval after the
/// cursor advanced. The caller must verify the overlap with bytes it
/// already trusts.
pub fn read_rollout_range(path: &Path, start: u64, end: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    if end < start || end > MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "Codex rollout '{}' range request is out of bounds",
            path.display()
        ));
    }
    let mut file = open_rollout_no_follow(path)?;
    let handle_meta = file
        .metadata()
        .map_err(|error| format!("stat Codex rollout '{}': {}", path.display(), error))?;
    if !handle_meta.is_file() {
        return Err(format!(
            "Codex rollout '{}' is not a regular file",
            path.display()
        ));
    }
    if handle_meta.len() < end {
        return Err(format!(
            "Codex rollout '{}' is shorter than the pinned range",
            path.display()
        ));
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|error| format!("seek Codex rollout '{}': {}", path.display(), error))?;
    let mut bytes = Vec::with_capacity((end - start) as usize);
    (&file)
        .take(end - start)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read Codex rollout '{}': {}", path.display(), error))?;
    if bytes.len() as u64 != end - start {
        return Err(format!(
            "Codex rollout '{}' raced the pinned range read",
            path.display()
        ));
    }
    crate::sync_metrics::record_append_bytes_read(path, end - start);
    Ok(bytes)
}

struct SegmentSource {
    path: PathBuf,
    bytes: Vec<u8>,
    consumed_byte_len: u64,
    declared_end_ordinal_exclusive: Option<u64>,
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
    crate::sync_metrics::record_full_graph_resolve(leaf_path);
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
        declared_end_ordinal_exclusive: None,
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
            declared_end_ordinal_exclusive: Some(reference.end_ordinal_exclusive),
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
        let paginated = segment.meta.history_mode.as_deref() == Some("paginated");
        if (segment.meta.history_base.is_some() || segment.declared_end_ordinal_exclusive.is_some())
            && !paginated
        {
            return Err(format!(
                "Codex history segment '{}' is outside paginated mode",
                segment.meta.id
            ));
        }
        let start_ordinal = segment
            .meta
            .history_base
            .as_ref()
            .map_or(0, |base| base.end_ordinal_exclusive);
        for raw in segment.bytes.split(|byte| *byte == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_slice(raw).map_err(|_| {
                format!(
                    "Codex segment '{}' contains malformed JSON",
                    segment.meta.id
                )
            })?;
            if paginated {
                let expected_ordinal =
                    start_ordinal.checked_add(record_count).ok_or_else(|| {
                        format!("Codex segment '{}' ordinal overflow", segment.meta.id)
                    })?;
                let actual_ordinal = value
                    .get("ordinal")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| {
                        format!(
                            "Codex paginated segment '{}' record {} has no ordinal",
                            segment.meta.id,
                            record_count + 1
                        )
                    })?;
                if actual_ordinal != expected_ordinal {
                    return Err(format!(
                        "Codex paginated segment '{}' record {} expected ordinal {}, found {}",
                        segment.meta.id,
                        record_count + 1,
                        expected_ordinal,
                        actual_ordinal
                    ));
                }
            }
            record_count += 1;
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
        let end_ordinal_exclusive = start_ordinal
            .checked_add(record_count)
            .ok_or_else(|| format!("Codex segment '{}' ordinal overflow", segment.meta.id))?;
        if let Some(declared) = segment.declared_end_ordinal_exclusive {
            if declared != end_ordinal_exclusive {
                return Err(format!(
                    "Codex history base '{}' cutoffs disagree: declared end ordinal {}, computed {} from {} physical records",
                    segment.meta.id, declared, end_ordinal_exclusive, record_count
                ));
            }
        }
        let canonical_messages = builder.appended_len() as u64 - appended_before;
        crate::sync_metrics::record_records_parsed(&segment.path, record_count);
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
    let consumed_record_ordinal = if resolved.leaf_meta.history_mode.as_deref() == Some("paginated")
    {
        resolved
            .leaf_meta
            .history_base
            .as_ref()
            .map_or(0, |base| base.end_ordinal_exclusive)
            .checked_add(leaf.consumed_record_count)
            .ok_or_else(|| "Codex leaf ordinal overflow".to_string())?
    } else {
        leaf.consumed_record_count
    };
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
        consumed_record_ordinal,
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

/// Extend a saved accumulator over newly appended complete records, from a
/// bounded tail window holding only the probe window and appended bytes.
/// Reads no byte before the cursor beyond the bounded probe window;
/// `WholeThread` callers recompute through `resolve_codex_root` instead.
pub fn refresh_codex_tail(
    layout: &RootTailLayout,
    observed_tail: &PortableConversationTail,
    mapping: &RootPhysicalMapping,
    tail: &RolloutTailWindow,
) -> Result<CodexTailRefresh, String> {
    if tail.window_offset != mapping.consumed_byte_offset - mapping.probe_window_len {
        return Err("Codex tail window does not start at the mapping's probe window".to_string());
    }
    let current_len = tail.file_len;
    if current_len < mapping.consumed_byte_offset {
        return Ok(CodexTailRefresh::Invalid {
            reason: CodexTailInvalidReason::Truncated,
        });
    }
    if tail.bytes.len() as u64 != current_len - tail.window_offset {
        return Err("Codex tail window is shorter than its declared span".to_string());
    }
    let consumed = mapping.consumed_byte_offset as usize;
    let window = mapping.probe_window_len as usize;
    crate::sync_metrics::record_bytes_hashed(Path::new(&mapping.root_path), window as u64);
    let probe = hex_sha256(&tail.bytes[..window]);
    if probe != mapping.probe_window_sha256 {
        return Ok(CodexTailRefresh::Invalid {
            reason: CodexTailInvalidReason::Replaced,
        });
    }
    let safe_end = consumed as u64 + safe_record_end(&tail.bytes[window..]);
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
    let relative = |absolute: u64| (absolute - tail.window_offset) as usize;
    let mut appended_records = 0_u64;
    let mut next_record_ordinal = mapping.consumed_record_ordinal;
    for raw in tail.bytes[window..relative(safe_end)].split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(raw) else {
            return Ok(CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::MalformedAppend,
            });
        };
        let Some(actual_ordinal) = value.get("ordinal").and_then(serde_json::Value::as_u64) else {
            if appended_records == 0 {
                // Codex 0.147 emits both legacy and paginated histories. An
                // ordinal-less first append must be re-read with its session
                // metadata so the authoritative history mode chooses the
                // validation path. Once an ordinal sequence has started,
                // every later row must continue it.
                return Ok(CodexTailRefresh::NeedsFullResolve);
            }
            return Ok(CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::MalformedAppend,
            });
        };
        if actual_ordinal != next_record_ordinal {
            return Ok(CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::MalformedAppend,
            });
        }
        let Some(next_ordinal) = next_record_ordinal.checked_add(1) else {
            return Ok(CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::MalformedAppend,
            });
        };
        next_record_ordinal = next_ordinal;
        appended_records += 1;
        // Codex can repeat the active session metadata when a task resumes.
        // The cursor does not retain the full ancestry/cwd structure needed
        // to validate it, so defer to the bounded whole-root parser. That
        // parser accepts a consistent repetition and rejects replacement or
        // unrelated identities.
        if structural_record_type(&value) == Some("session_meta") {
            return Ok(CodexTailRefresh::NeedsFullResolve);
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

    crate::sync_metrics::record_records_parsed(Path::new(&mapping.root_path), appended_records);
    let probe_window_len = safe_end.min(PROBE_WINDOW_BYTES);
    let window_start = safe_end - probe_window_len;
    let new_mapping = RootPhysicalMapping {
        provider_thread_id: mapping.provider_thread_id.clone(),
        root_path: mapping.root_path.clone(),
        source_file_size: current_len,
        consumed_byte_offset: safe_end,
        consumed_record_ordinal: next_record_ordinal,
        probe_window_len,
        probe_window_sha256: hex_sha256(&tail.bytes[relative(window_start)..relative(safe_end)]),
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
    use mallard_domain::RawRangePin;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn retained_checkpoint(path: &Path, bytes: &[u8]) -> ConversationCheckpoint {
        ConversationCheckpoint {
            generation: 1,
            accepted: None,
            accepted_local_tail: None,
            observed_tail: PortableConversationTail {
                codec: PORTABLE_TAIL_CODEC_V1.to_string(),
                partition_kind: TailPartitionKind::WholeThread,
                start_record_count: 0,
                accepted_end_record_count: 0,
                accepted_tail_state: "0".repeat(64),
            },
            physical: PhysicalMappingState::Ready {
                root_mapping: RootPhysicalMapping {
                    provider_thread_id: "019f7500-0000-7000-8000-00000000ce01".to_string(),
                    root_path: path.to_string_lossy().into_owned(),
                    source_file_size: bytes.len() as u64,
                    consumed_byte_offset: bytes.len() as u64,
                    consumed_record_ordinal: 1,
                    probe_window_len: bytes.len() as u64,
                    probe_window_sha256: hex_sha256(bytes),
                    last_canonical_message_id: None,
                    last_synthetic_turn_id: None,
                    next_synthetic_turn_ordinal: 0,
                },
                raw_range_pin: RawRangePin {
                    start_byte_offset: 0,
                    end_byte_offset: bytes.len() as u64,
                    range_sha256: hex_sha256(bytes),
                },
            },
            prefix_witness: None,
            binding_revision: 0,
        }
    }

    #[test]
    fn retained_source_witness_hides_only_exact_complete_no_follow_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("retained.jsonl");
        let bytes = b"{}\n";
        fs::write(&path, bytes).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let checkpoint = retained_checkpoint(&path, bytes);
        assert!(retained_root_is_byte_identical(&path, &checkpoint).unwrap());

        fs::write(&path, b"{}\n{}\n").unwrap();
        assert!(!retained_root_is_byte_identical(&path, &checkpoint).unwrap());

        fs::write(&path, bytes).unwrap();
        let mut partial = checkpoint.clone();
        let PhysicalMappingState::Ready { raw_range_pin, .. } = &mut partial.physical else {
            unreachable!()
        };
        raw_range_pin.start_byte_offset = 1;
        assert!(!retained_root_is_byte_identical(&path, &partial).unwrap());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let linked = temp.path().join("retained-link.jsonl");
            symlink(&path, &linked).unwrap();
            let linked_checkpoint = retained_checkpoint(&linked, bytes);
            assert!(retained_root_is_byte_identical(&linked, &linked_checkpoint).is_err());
        }
    }

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

    fn encode_paginated_lines(lines: &[String]) -> Vec<u8> {
        let mut values = lines
            .iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let mut ordinal = values
            .first()
            .and_then(|value| value.get("payload"))
            .and_then(|payload| payload.get("history_base"))
            .and_then(|base| base.get("end_ordinal_exclusive"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let mut bytes = Vec::new();
        for value in &mut values {
            value["ordinal"] = json!(ordinal);
            ordinal = ordinal.checked_add(1).unwrap();
            serde_json::to_writer(&mut bytes, value).unwrap();
            bytes.push(b'\n');
        }
        bytes
    }

    fn next_paginated_ordinal(bytes: &[u8]) -> u64 {
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|raw| !raw.is_empty())
            .map(|raw| serde_json::from_slice::<serde_json::Value>(raw).unwrap())
            .last()
            .and_then(|value| value.get("ordinal").and_then(serde_json::Value::as_u64))
            .unwrap()
            .checked_add(1)
            .unwrap()
    }

    fn append_paginated_line(bytes: &mut Vec<u8>, line: &str) {
        let mut value = serde_json::from_str::<serde_json::Value>(line).unwrap();
        value["ordinal"] = json!(next_paginated_ordinal(bytes));
        serde_json::to_writer(&mut *bytes, &value).unwrap();
        bytes.push(b'\n');
    }

    struct Graph {
        directory: tempfile::TempDir,
        files: BTreeMap<String, PathBuf>,
    }

    impl Graph {
        fn write(&mut self, thread: &str, lines: &[String]) -> PathBuf {
            let path = self.directory.path().join(format!("{thread}.jsonl"));
            fs::write(&path, encode_paginated_lines(lines)).unwrap();
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
            meta_line("p2", Some(("p1", 5, p1_len))),
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
        assert_eq!(
            resolved
                .segments
                .iter()
                .map(|segment| segment.consumed_record_count)
                .collect::<Vec<_>>(),
            vec![3, 2, 3]
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
        assert_eq!(seeded.mapping.consumed_record_ordinal, 8);
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

        // A referenced physical segment is part of paginated history even
        // when it has no base of its own; a legacy-mode parent is not an
        // ordinal-less compatibility path.
        let legacy_meta = serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {
                "id": "legacy-parent",
                "cwd": "/tmp/project",
                "history_mode": "legacy",
            },
        }))
        .unwrap();
        let legacy = graph.write(
            "legacy-parent",
            &[legacy_meta, message_line("legacy-m1", "one")],
        );
        let legacy_len = fs::metadata(&legacy).unwrap().len();
        let leaf = graph.write(
            "legacy-child",
            &[meta_line(
                "legacy-child",
                Some(("legacy-parent", 2, legacy_len)),
            )],
        );
        let error = resolve_codex_root(&leaf, &graph.lookup()).unwrap_err();
        assert!(error.contains("outside paginated mode"), "{error}");
    }

    #[test]
    fn paginated_segment_ordinals_must_be_present_and_contiguous() {
        let mut graph = graph();
        let leaf = graph.write(
            "solo",
            &[
                meta_line("solo", None),
                message_line("m1", "one"),
                message_line("m2", "two"),
            ],
        );
        let original = fs::read(&leaf).unwrap();
        let values = original
            .split(|byte| *byte == b'\n')
            .filter(|raw| !raw.is_empty())
            .map(|raw| serde_json::from_slice::<serde_json::Value>(raw).unwrap())
            .collect::<Vec<_>>();
        let write_values = |values: &[serde_json::Value]| {
            let mut bytes = Vec::new();
            for value in values {
                serde_json::to_writer(&mut bytes, value).unwrap();
                bytes.push(b'\n');
            }
            fs::write(&leaf, bytes).unwrap();
        };

        let mut missing = values.clone();
        missing[1].as_object_mut().unwrap().remove("ordinal");
        write_values(&missing);
        let error = resolve_codex_root(&leaf, &graph.lookup()).unwrap_err();
        assert!(error.contains("has no ordinal"), "{error}");

        let mut gap = values;
        gap[1]["ordinal"] = json!(2);
        write_values(&gap);
        let error = resolve_codex_root(&leaf, &graph.lookup()).unwrap_err();
        assert!(error.contains("expected ordinal 1, found 2"), "{error}");
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
    fn current_legacy_leaf_accepts_ordinal_less_records() {
        let graph = graph();
        let leaf = graph.directory.path().join("legacy.jsonl");
        let metadata = serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {
                "id": "legacy",
                "cwd": "/tmp/project",
                "history_mode": "legacy",
            },
        }))
        .unwrap();
        fs::write(
            &leaf,
            format!(
                "{metadata}\n{}\n{}\n",
                message_line("m1", "one"),
                message_line("m2", "two"),
            ),
        )
        .unwrap();

        let resolved = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        assert_eq!(resolved.partition_kind, TailPartitionKind::WholeThread);
        assert_eq!(resolved.chain.len(), 2);
        let bytes = fs::read(&leaf).unwrap();
        let seeded = seed_codex_root(&resolved, &bytes, None).unwrap();
        assert_eq!(seeded.mapping.consumed_record_ordinal, 3);
    }

    /// A tail window equivalent to handing the whole buffer to the old
    /// whole-file cursor path, for equivalence tests.
    fn tail_window_from(mapping: &RootPhysicalMapping, full: &[u8]) -> RolloutTailWindow {
        let window_offset = mapping.consumed_byte_offset - mapping.probe_window_len;
        if (full.len() as u64) < mapping.consumed_byte_offset {
            return RolloutTailWindow {
                file_len: full.len() as u64,
                window_offset,
                bytes: Vec::new(),
            };
        }
        RolloutTailWindow {
            file_len: full.len() as u64,
            window_offset,
            bytes: full[window_offset as usize..].to_vec(),
        }
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
                &tail_window_from(&seeded.mapping, &leaf_bytes),
            )
            .unwrap(),
            CodexTailRefresh::Unchanged
        )
        .then_some(())
        .unwrap();

        // An ordinal-less first append could belong to Codex's current
        // legacy history mode, so refresh defers to a full metadata-aware
        // resolve. A started ordinal sequence remains strict: a later
        // missing or skipped ordinal is malformed.
        let mut missing_ordinal = leaf_bytes.clone();
        missing_ordinal.extend_from_slice(message_line("bad-missing", "bad").as_bytes());
        missing_ordinal.push(b'\n');
        assert!(matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &missing_ordinal),
            )
            .unwrap(),
            CodexTailRefresh::NeedsFullResolve
        ));
        let mut mixed_missing = leaf_bytes.clone();
        append_paginated_line(&mut mixed_missing, &message_line("valid-first", "valid"));
        mixed_missing.extend_from_slice(message_line("bad-second", "bad").as_bytes());
        mixed_missing.push(b'\n');
        assert!(matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &mixed_missing),
            )
            .unwrap(),
            CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::MalformedAppend
            }
        ));
        let mut skipped_ordinal = leaf_bytes.clone();
        let mut skipped =
            serde_json::from_str::<serde_json::Value>(&message_line("bad-skipped", "bad")).unwrap();
        skipped["ordinal"] = json!(seeded.mapping.consumed_record_ordinal + 1);
        serde_json::to_writer(&mut skipped_ordinal, &skipped).unwrap();
        skipped_ordinal.push(b'\n');
        assert!(matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &skipped_ordinal),
            )
            .unwrap(),
            CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::MalformedAppend
            }
        ));

        // Append m6 and refresh: the incremental state equals the one-shot
        // over the re-resolved chain.
        let mut appended = leaf_bytes.clone();
        append_paginated_line(&mut appended, &message_line("m6", "six"));
        fs::write(&leaf, &appended).unwrap();
        let CodexTailRefresh::Extended {
            observed_tail,
            mapping,
        } = refresh_codex_tail(
            &seeded.layout,
            &seeded.observed_tail,
            &seeded.mapping,
            &tail_window_from(&seeded.mapping, &appended),
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
                &tail_window_from(&seeded.mapping, truncated)
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
                &tail_window_from(&seeded.mapping, &replaced),
            )
            .unwrap(),
            CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::Replaced
            }
        )
        .then_some(())
        .unwrap();

        // Structural metadata cannot be decided from the cursor alone.
        let mut meta_appended = leaf_bytes.clone();
        append_paginated_line(&mut meta_appended, &meta_line("p2-reborn", None));
        matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &meta_appended),
            )
            .unwrap(),
            CodexTailRefresh::NeedsFullResolve
        )
        .then_some(())
        .unwrap();
    }

    #[test]
    fn repeated_active_metadata_revalidates_while_unrelated_metadata_stays_blocked() {
        let mut graph = graph();
        let leaf = nested_graph(&mut graph);
        let leaf_bytes = fs::read(&leaf).unwrap();
        let resolved = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        let seeded = seed_codex_root(&resolved, &leaf_bytes, None).unwrap();
        let first_record_end = leaf_bytes.iter().position(|byte| *byte == b'\n').unwrap() + 1;

        // Resuming can append the same active session_meta before new work.
        // The cursor requests a full resolve, whose structural validator
        // accepts the repeated identity and derives the advanced tail.
        let mut resumed = leaf_bytes.clone();
        let repeated_meta = std::str::from_utf8(&leaf_bytes[..first_record_end - 1]).unwrap();
        append_paginated_line(&mut resumed, repeated_meta);
        append_paginated_line(&mut resumed, &message_line("m6", "six"));
        assert!(matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &resumed),
            )
            .unwrap(),
            CodexTailRefresh::NeedsFullResolve
        ));
        fs::write(&leaf, &resumed).unwrap();
        let resumed_root = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        let resumed_seed = seed_codex_root(&resumed_root, &resumed, Some(&seeded.layout)).unwrap();
        assert_eq!(resumed_root.chain.len(), 6);
        assert_eq!(resumed_seed.observed_tail.accepted_end_record_count, 6);

        // A different identity takes the same full-resolve path but fails
        // structural validation instead of being accepted as continuation.
        let mut replaced = leaf_bytes.clone();
        append_paginated_line(&mut replaced, &meta_line("unrelated", None));
        assert!(matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &tail_window_from(&seeded.mapping, &replaced),
            )
            .unwrap(),
            CodexTailRefresh::NeedsFullResolve
        ));
        fs::write(&leaf, &replaced).unwrap();
        let error = resolve_codex_root(&leaf, &graph.lookup()).unwrap_err();
        assert!(error.contains("unrelated session_meta"), "{error}");
    }

    /// The bounded reader hands `refresh_codex_tail` exactly the probe
    /// window plus appended bytes, counts them, leaves an incomplete
    /// trailing record unconsumed, and fails closed on truncation beneath
    /// the cursor.
    #[test]
    fn tail_window_reader_reads_probe_and_append_only_and_skips_incomplete_suffix() {
        let mut graph = graph();
        let leaf = nested_graph(&mut graph);
        let resolved = resolve_codex_root(&leaf, &graph.lookup()).unwrap();
        let leaf_bytes = fs::read(&leaf).unwrap();
        let seeded = seed_codex_root(&resolved, &leaf_bytes, None).unwrap();

        let metrics = crate::sync_metrics::register_root(graph.directory.path());
        let complete = message_line("m6", "six");
        let mut appended = leaf_bytes.clone();
        append_paginated_line(&mut appended, &complete);
        let complete_len = appended.len() - leaf_bytes.len();
        appended.extend_from_slice(b"{\"type\":\"response_item\",\"payload\"");
        fs::write(&leaf, &appended).unwrap();

        let window = read_rollout_tail_window(&leaf, &seeded.mapping).unwrap();
        assert_eq!(window.file_len, appended.len() as u64);
        assert_eq!(
            window.window_offset,
            seeded.mapping.consumed_byte_offset - seeded.mapping.probe_window_len
        );
        assert_eq!(window.bytes, appended[window.window_offset as usize..]);
        let counted = metrics.snapshot();
        assert_eq!(counted.probe_bytes_read, seeded.mapping.probe_window_len);
        assert_eq!(
            counted.append_bytes_read,
            appended.len() as u64 - seeded.mapping.consumed_byte_offset
        );
        assert_eq!(counted.transcript_bytes_read, 0);

        let CodexTailRefresh::Extended { mapping, .. } = refresh_codex_tail(
            &seeded.layout,
            &seeded.observed_tail,
            &seeded.mapping,
            &window,
        )
        .unwrap() else {
            panic!("complete appended record extends the cursor");
        };
        // The cursor lands on the last complete record boundary, never
        // inside the dirty suffix.
        assert_eq!(
            mapping.consumed_byte_offset,
            (leaf_bytes.len() + complete_len) as u64
        );
        assert!(mapping.consumed_byte_offset < appended.len() as u64);

        // Truncation beneath the cursor is typed by refresh, not the reader.
        fs::write(&leaf, &leaf_bytes[..leaf_bytes.len() - 10]).unwrap();
        let truncated = read_rollout_tail_window(&leaf, &seeded.mapping).unwrap();
        assert_eq!(truncated.file_len, leaf_bytes.len() as u64 - 10);
        assert!(truncated.bytes.is_empty());
        assert!(matches!(
            refresh_codex_tail(
                &seeded.layout,
                &seeded.observed_tail,
                &seeded.mapping,
                &truncated,
            )
            .unwrap(),
            CodexTailRefresh::Invalid {
                reason: CodexTailInvalidReason::Truncated
            }
        ));

        // A symlinked rollout is refused at open.
        #[cfg(unix)]
        {
            let alias = graph.directory.path().join("alias.jsonl");
            std::os::unix::fs::symlink(&leaf, &alias).unwrap();
            let error = read_rollout_tail_window(&alias, &seeded.mapping).unwrap_err();
            assert!(error.contains("open Codex rollout"), "{error}");
        }
    }

    /// The complete A→B→A lifecycle over the real machinery, with in-content
    /// paths riding the canonical wire: A pushes a paginated root (Encode),
    /// B pulls the token-form object and maps the same fixed start (Decode),
    /// divergence is pinned and resolved by keep-both, the tracked thread
    /// republishes from the pulled machine reproducing the accepted wire
    /// prefix, A pulls the extension as CloudOutstanding and byte-restores
    /// its own canonical content, and the preserved branch publishes
    /// independently under its own layout.
    #[test]
    fn a_to_b_to_a_round_trip_is_stable_after_canonicalization() {
        use crate::codex_standalone_builder::{build_standalone_codex_root, StandaloneBuildMode};
        use mallard_domain::{
            compare_conversation_tail, compute_conversation_tail_state,
            compute_portable_conversation_tail, PortableConversationTail, RootTailLayout,
            SyncCurrency, TailAccumulator, TailClassification, TailSideEvidence,
            VerifiedTailEvidence, PORTABLE_TAIL_CODEC_V1, PORTABLE_TAIL_CODEC_WIRE_V1,
        };

        const ROOT_A: &str = "/tmp/machine-a";
        const ROOT_B: &str = "/tmp/machine-b";

        fn evidence(
            accepted: TailSideEvidence,
            accepted_local: TailSideEvidence,
            local: TailSideEvidence,
            cloud: TailSideEvidence,
        ) -> VerifiedTailEvidence {
            VerifiedTailEvidence::new(accepted, accepted_local, local, cloud).unwrap()
        }
        let present = |tail: &PortableConversationTail| TailSideEvidence::Present(tail.clone());
        let paginated_layout = |start: u64| RootTailLayout {
            codec: PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: TailPartitionKind::FixedPaginatedTail,
            start_record_count: start,
        };
        let encode = |root: &str| StandaloneBuildMode::Encode {
            root: root.to_string(),
            canonical_root: None,
        };
        let decode = |id: &str, root: &str| StandaloneBuildMode::Decode {
            root_provider_thread_id: id.to_string(),
            target_root: root.to_string(),
            canonical_target_root: None,
        };
        let append = |path: &PathBuf, id: &str, body: &str| {
            let mut bytes = fs::read(path).unwrap();
            append_paginated_line(&mut bytes, &message_line(id, body));
            fs::write(path, &bytes).unwrap();
        };

        // Machine A: base rollout b1..b5 with in-root and out-of-root
        // content paths across the covered spellings, leaf continuing at
        // base@6 with m1,m2,m3.
        let mut machine_a = graph();
        let a_meta = |id: &str, base: Option<(&str, u64, u64)>| {
            let mut payload = json!({"id": id, "cwd": ROOT_A, "history_mode": "paginated"});
            if let Some((thread, ordinal, byte)) = base {
                payload["history_base"] = json!({
                    "thread_id": thread,
                    "end_ordinal_exclusive": ordinal,
                    "end_byte_offset": byte,
                });
            }
            serde_json::to_string(&json!({"type": "session_meta", "payload": payload})).unwrap()
        };
        let base_lines: Vec<String> = std::iter::once(a_meta("base", None))
            .chain([
                message_line("b1", "plain text"),
                message_line("b2", "created /tmp/machine-a/readme"),
                message_line("b3", "open file:///tmp/machine-a/read%20me"),
                message_line("b4", "outside /tmp/other-proj/x stays"),
                message_line("b5", "base done"),
            ])
            .collect();
        let base = machine_a.write("base", &base_lines);
        let base_len = fs::metadata(&base).unwrap().len();
        let a_leaf = machine_a.write(
            "a-leaf",
            &[
                a_meta("a-leaf", Some(("base", 6, base_len))),
                message_line("m1", "one"),
                message_line("m2", "edit /tmp/machine-a/src/main.rs"),
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
        let a_local_v1 = compute_portable_conversation_tail(
            TailPartitionKind::FixedPaginatedTail,
            5,
            &resolved_a.chain,
        )
        .unwrap();

        // A pushes: the artifact folds the wire-form tail it publishes; the
        // local twin is a different value under a different codec because
        // in-root content paths tokenized.
        let push_one = build_standalone_codex_root(&resolved_a, &encode(ROOT_A), None).unwrap();
        let cloud_v1 = push_one.portable_tail.clone();
        assert_eq!(cloud_v1.codec, PORTABLE_TAIL_CODEC_WIRE_V1);
        assert_eq!(cloud_v1.start_record_count, 5);
        assert_eq!(cloud_v1.accepted_end_record_count, 8);
        assert_eq!(a_local_v1.codec, PORTABLE_TAIL_CODEC_V1);
        assert_ne!(cloud_v1.accepted_tail_state, a_local_v1.accepted_tail_state);

        // Machine B pulls: the staged token-form object parses as a normal
        // rollout, its wire chain refolds to the accepted cloud value, and
        // the decode build materializes B's local form under the imported
        // fixed start.
        let mut machine_b = graph();
        let staged = machine_b.directory.path().join("staged.jsonl");
        fs::write(&staged, &push_one.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let resolved_staged = resolve_codex_root(&staged, &no_bases).unwrap();
        assert_eq!(
            resolved_staged.partition_kind,
            TailPartitionKind::WholeThread
        );
        let refolded = compute_conversation_tail_state(
            TailPartitionKind::FixedPaginatedTail,
            5,
            8,
            &resolved_staged.chain,
        )
        .unwrap();
        assert_eq!(refolded, cloud_v1.accepted_tail_state);
        let imported = paginated_layout(5);
        let pulled = build_standalone_codex_root(
            &resolved_staged,
            &decode("b-local", ROOT_B),
            Some(&imported),
        )
        .unwrap();
        let b_local_v1 = pulled.portable_tail.clone();
        assert_eq!(b_local_v1.codec, PORTABLE_TAIL_CODEC_V1);
        assert_eq!(b_local_v1.accepted_end_record_count, 8);
        let text = String::from_utf8(pulled.root_bytes.clone()).unwrap();
        assert!(text.contains("created /tmp/machine-b/readme"));
        assert!(text.contains("file:///tmp/machine-b/read%20me"));
        assert!(text.contains("/tmp/other-proj/x"));
        let b_local = machine_b.directory.path().join("b-local.jsonl");
        fs::write(&b_local, &pulled.root_bytes).unwrap();
        machine_b
            .files
            .insert("b-local".to_string(), b_local.clone());

        // B observes under the imported layout: the seeded value equals the
        // accepted local twin, and B is in sync under its dual pins.
        let resolved_b = resolve_codex_root(&b_local, &machine_b.lookup()).unwrap();
        let b_bytes = fs::read(&b_local).unwrap();
        let seeded_b = seed_codex_root(&resolved_b, &b_bytes, Some(&imported)).unwrap();
        assert_eq!(seeded_b.observed_tail, b_local_v1);
        assert_eq!(seeded_b.layout, imported);
        assert_eq!(
            compare_conversation_tail(&evidence(
                present(&cloud_v1),
                present(&b_local_v1),
                present(&seeded_b.observed_tail),
                present(&cloud_v1),
            )),
            TailClassification::Sync(SyncCurrency::Current)
        );

        // A appends m4: LocalOutstanding under its dual pins, then pushed.
        append(&a_leaf, "m4", "more /tmp/machine-a/src/lib.rs");
        let resolved_a2 = resolve_codex_root(&a_leaf, &machine_a.lookup()).unwrap();
        let a_local_v2 = compute_portable_conversation_tail(
            TailPartitionKind::FixedPaginatedTail,
            5,
            &resolved_a2.chain,
        )
        .unwrap();
        assert_eq!(
            compare_conversation_tail(&evidence(
                present(&cloud_v1),
                present(&a_local_v1),
                present(&a_local_v2),
                present(&cloud_v1),
            )),
            TailClassification::LocalOutstanding
        );
        let push_two = build_standalone_codex_root(&resolved_a2, &encode(ROOT_A), None).unwrap();
        let cloud_v2 = push_two.portable_tail.clone();
        assert_eq!(cloud_v2.accepted_end_record_count, 9);

        // Map-and-extend identity across machines on the wire column:
        // resuming from the pulled wire state and folding the appended wire
        // record reproduces A's published value.
        let staged_two = machine_b.directory.path().join("staged-two.jsonl");
        fs::write(&staged_two, &push_two.root_bytes).unwrap();
        let resolved_staged_two = resolve_codex_root(&staged_two, &no_bases).unwrap();
        let mut resumed = TailAccumulator::resume(
            TailPartitionKind::FixedPaginatedTail,
            5,
            8,
            &cloud_v1.accepted_tail_state,
        )
        .unwrap();
        resumed.fold_message(&resolved_staged_two.chain[8]).unwrap();
        assert_eq!(resumed.state_hex(), cloud_v2.accepted_tail_state);

        // B appends a conflicting m5 before seeing the new head: a pinned
        // divergence, not an unprovable pair.
        append(&b_local, "m5-b", "five on b");
        let resolved_b2 = resolve_codex_root(&b_local, &machine_b.lookup()).unwrap();
        let b_bytes2 = fs::read(&b_local).unwrap();
        let diverged_b = seed_codex_root(&resolved_b2, &b_bytes2, Some(&imported)).unwrap();
        assert_eq!(
            compare_conversation_tail(&evidence(
                present(&cloud_v1),
                present(&b_local_v1),
                present(&diverged_b.observed_tail),
                present(&cloud_v2),
            )),
            TailClassification::Divergent
        );

        // Keep-both: the preserved branch keeps B's bytes untouched under the
        // unchanged local layout; the tracked thread re-materializes the
        // cloud graph under a fresh alias with the remote layout.
        let preserved_bytes = fs::read(&b_local).unwrap();
        let tracked = build_standalone_codex_root(
            &resolved_staged_two,
            &decode("b-tracked", ROOT_B),
            Some(&imported),
        )
        .unwrap();
        let b_tracked_v2 = tracked.portable_tail.clone();
        assert_eq!(b_tracked_v2.accepted_end_record_count, 9);
        let b_tracked = machine_b.directory.path().join("b-tracked.jsonl");
        fs::write(&b_tracked, &tracked.root_bytes).unwrap();
        machine_b
            .files
            .insert("b-tracked".to_string(), b_tracked.clone());
        assert_eq!(fs::read(&b_local).unwrap(), preserved_bytes);

        // B appends m6 to the tracked thread and republishes from the pulled
        // machine: the artifact preserves the imported partition and
        // reproduces the accepted wire prefix — the wire fold extends from
        // cloud_v2's exact state even though the bytes were re-encoded from
        // B's decoded local form.
        append(&b_tracked, "m6", "six under /tmp/machine-b/src/main.rs");
        let resolved_tracked = resolve_codex_root(&b_tracked, &machine_b.lookup()).unwrap();
        let tracked_bytes = fs::read(&b_tracked).unwrap();
        let seeded_tracked =
            seed_codex_root(&resolved_tracked, &tracked_bytes, Some(&imported)).unwrap();
        assert_eq!(
            compare_conversation_tail(&evidence(
                present(&cloud_v2),
                present(&b_tracked_v2),
                present(&seeded_tracked.observed_tail),
                present(&cloud_v2),
            )),
            TailClassification::LocalOutstanding
        );
        let push_three =
            build_standalone_codex_root(&resolved_tracked, &encode(ROOT_B), Some(&imported))
                .unwrap();
        let cloud_v3 = push_three.portable_tail.clone();
        assert_eq!(cloud_v3.codec, PORTABLE_TAIL_CODEC_WIRE_V1);
        assert_eq!(cloud_v3.start_record_count, 5);
        assert_eq!(cloud_v3.accepted_end_record_count, 10);
        let staged_three = machine_a.directory.path().join("staged-three.jsonl");
        fs::write(&staged_three, &push_three.root_bytes).unwrap();
        let resolved_staged_three = resolve_codex_root(&staged_three, &no_bases).unwrap();
        let mut resumed = TailAccumulator::resume(
            TailPartitionKind::FixedPaginatedTail,
            5,
            9,
            &cloud_v2.accepted_tail_state,
        )
        .unwrap();
        resumed
            .fold_message(&resolved_staged_three.chain[9])
            .unwrap();
        assert_eq!(resumed.state_hex(), cloud_v3.accepted_tail_state);

        // A pulls the extension: CloudOutstanding under A's dual pins, and
        // the decoded materialization byte-restores A's already-canonical
        // content — the fold state over the restored prefix equals A's own
        // local value.
        assert_eq!(
            compare_conversation_tail(&evidence(
                present(&cloud_v2),
                present(&a_local_v2),
                present(&a_local_v2),
                present(&cloud_v3),
            )),
            TailClassification::CloudOutstanding
        );
        let a_adopted = build_standalone_codex_root(
            &resolved_staged_three,
            &decode("a-tracked", ROOT_A),
            Some(&imported),
        )
        .unwrap();
        assert_eq!(a_adopted.portable_tail.codec, PORTABLE_TAIL_CODEC_V1);
        assert_eq!(a_adopted.portable_tail.accepted_end_record_count, 10);
        let a_tracked = machine_a.directory.path().join("a-tracked.jsonl");
        fs::write(&a_tracked, &a_adopted.root_bytes).unwrap();
        machine_a
            .files
            .insert("a-tracked".to_string(), a_tracked.clone());
        let resolved_a3 = resolve_codex_root(&a_tracked, &machine_a.lookup()).unwrap();
        assert_eq!(&resolved_a3.chain[..9], &resolved_a2.chain[..]);
        let restored_prefix_state = compute_conversation_tail_state(
            TailPartitionKind::FixedPaginatedTail,
            5,
            9,
            &resolved_a3.chain,
        )
        .unwrap();
        assert_eq!(restored_prefix_state, a_local_v2.accepted_tail_state);

        // The preserved branch is its own thread: locally present, missing
        // from the cloud, and its first Push publishes the preserved layout
        // with no family metadata.
        assert_eq!(
            compare_conversation_tail(&evidence(
                TailSideEvidence::ConfirmedAbsent,
                TailSideEvidence::ConfirmedAbsent,
                present(&diverged_b.observed_tail),
                TailSideEvidence::ConfirmedAbsent,
            )),
            TailClassification::MissingCloud
        );
        let preserved_push =
            build_standalone_codex_root(&resolved_b2, &encode(ROOT_B), Some(&imported)).unwrap();
        assert_eq!(
            preserved_push.portable_tail.codec,
            PORTABLE_TAIL_CODEC_WIRE_V1
        );
        assert_eq!(preserved_push.portable_tail.start_record_count, 5);
        assert_eq!(preserved_push.portable_tail.accepted_end_record_count, 9);
        assert!(preserved_push.subagents.is_empty());
        assert!(preserved_push.id_map.is_empty());
    }
}
