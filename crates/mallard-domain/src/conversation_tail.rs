//! Portable tail codec v1: the sole conversation-content comparison value.
//!
//! The tail is the cumulative canonical content of one flattened root chain
//! from an immutable portable start (the canonical boundary seeded from the
//! root's native `history_base`, or zero).  The five-field wire value below is
//! the only conversation identity the cloud stores; classification equality is
//! field-wise equality of the complete value, with no derived fingerprint.
//! Any change to hash domains, framing, folded fields, or the canonical
//! projection requires a new codec string — there is no in-place v1 evolution.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    hex_lower, update_length_prefixed, update_optional_length_prefixed,
    validate_conversation_chain, validate_sha256, CanonicalConversationMessage, MessageId,
    MAX_FILES,
};

pub const PORTABLE_TAIL_CODEC_V1: &str = "mallard-codex-portable-tail-v1";
/// The wire-form twin of [`PORTABLE_TAIL_CODEC_V1`]: folds token-form
/// artifact bytes (the canonical path-mapped wire encoding) instead of the
/// machine's real rollout bytes. Fold mechanics are identical; the distinct
/// label makes any accidental cross-form comparison fail structurally
/// (codec inequality ⇒ Unknown ⇒ Blocked) instead of silently mis-comparing.
pub const PORTABLE_TAIL_CODEC_WIRE_V1: &str = "mallard-codex-portable-tail-wire-v1";

/// Claude's local-form tail codec: the same fold over `claude.lineage.v1`
/// canonical chains. Claude publishes transcripts verbatim, so its local and
/// wire folds carry identical states; the twin labels exist to keep the
/// local/wire columns structurally incomparable, exactly as for Codex.
pub const CLAUDE_PORTABLE_TAIL_CODEC_V1: &str = "mallard-claude-portable-tail-v1";
/// The wire-form twin of [`CLAUDE_PORTABLE_TAIL_CODEC_V1`], mandatory on
/// every Claude `ConversationThread`.
pub const CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1: &str = "mallard-claude-portable-tail-wire-v1";

// The hash domains are shared by every provider's fold; the historical
// "codex" in the byte strings is a frozen misnomer (renaming them would
// change every existing tail state for zero semantic gain).
const TAIL_START_DOMAIN_V1: &[u8] = b"mallard-codex-tail-start-v1";
const TAIL_RECORD_DOMAIN_V1: &[u8] = b"mallard-codex-tail-record-v1";

/// How the portable start was fixed.  Kinds never reclassify: a
/// `FixedPaginatedTail` whose verified boundary is zero stays explicit and is
/// a different identity from `WholeThread`, because the partition tag is
/// bound into the empty-tail state.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TailPartitionKind {
    WholeThread,
    FixedPaginatedTail,
}

impl TailPartitionKind {
    /// The tag folded into `S0`; identical to the serde wire tag so the codec
    /// and the schema share one vocabulary.
    fn partition_tag(self) -> &'static str {
        match self {
            TailPartitionKind::WholeThread => "whole_thread",
            TailPartitionKind::FixedPaginatedTail => "fixed_paginated_tail",
        }
    }
}

/// The portable wire value, mandatory for every Codex `ConversationThread`.
/// It contains no source paths, byte offsets, provider ordinals, provider or
/// machine IDs, `history_base` references, or subagent data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortableConversationTail {
    pub codec: String,
    pub partition_kind: TailPartitionKind,
    pub start_record_count: u64,
    pub accepted_end_record_count: u64,
    pub accepted_tail_state: String,
}

impl PortableConversationTail {
    /// Structural validity for any provider's five-field evidence.  Codex
    /// wire values are additionally pinned to [`PORTABLE_TAIL_CODEC_WIRE_V1`]
    /// by [`Self::validate_codex_wire`] and Claude wire values to
    /// [`CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1`] by [`Self::validate_claude_wire`];
    /// cursor-less providers supply `WholeThread`-equivalent evidence under
    /// their own codec string, and classification equality remains
    /// field-wise, codec included.
    pub fn validate(&self) -> Result<(), String> {
        validate_tail_codec(&self.codec)?;
        validate_tail_start(self.partition_kind, self.start_record_count)?;
        if self.start_record_count > self.accepted_end_record_count {
            return Err("portable tail start exceeds its accepted end".to_string());
        }
        if self.accepted_end_record_count > MAX_FILES as u64 {
            return Err("portable tail accepted end exceeds the record limit".to_string());
        }
        validate_sha256("portable tail accepted state", &self.accepted_tail_state)
    }

    /// The codex wire pin: schema-1 `ConversationThread` values for
    /// `provider == codex` must carry exactly the wire-form codec — cloud
    /// artifacts are token-form, and their tails fold token-form bytes.
    pub(crate) fn validate_codex_wire(&self) -> Result<(), String> {
        self.validate()?;
        if self.codec != PORTABLE_TAIL_CODEC_WIRE_V1 {
            return Err(format!(
                "unknown codex portable tail wire codec '{}'",
                self.codec.escape_debug()
            ));
        }
        Ok(())
    }

    /// The claude wire pin: schema-1 `ConversationThread` values for
    /// `provider == claude` must carry exactly the Claude wire-form codec.
    pub(crate) fn validate_claude_wire(&self) -> Result<(), String> {
        self.validate()?;
        if self.codec != CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1 {
            return Err(format!(
                "unknown claude portable tail wire codec '{}'",
                self.codec.escape_debug()
            ));
        }
        Ok(())
    }
}

fn validate_tail_codec(codec: &str) -> Result<(), String> {
    if codec.is_empty()
        || codec.len() > 128
        || codec
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(format!(
            "invalid portable tail codec '{}'",
            codec.escape_debug()
        ));
    }
    Ok(())
}

fn validate_tail_start(
    partition_kind: TailPartitionKind,
    start_record_count: u64,
) -> Result<(), String> {
    if partition_kind == TailPartitionKind::WholeThread && start_record_count != 0 {
        return Err("whole-thread portable tail must start at record zero".to_string());
    }
    if start_record_count > MAX_FILES as u64 {
        return Err("portable tail start exceeds the record limit".to_string());
    }
    Ok(())
}

/// Incremental tail fold.  `S0` binds the partition tag and start, so empty
/// tails at different boundaries or under different kinds are unequal; each
/// record fold binds the absolute canonical index, so the state also proves
/// the window position.  One-shot computation and incremental extension from
/// any saved `(count, state)` pair produce identical states; saved pairs are
/// machine-local resume values, never wire values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TailAccumulator {
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    next_record_count: u64,
    state: [u8; 32],
}

impl TailAccumulator {
    pub fn seed(
        partition_kind: TailPartitionKind,
        start_record_count: u64,
    ) -> Result<Self, String> {
        validate_tail_start(partition_kind, start_record_count)?;
        let mut hasher = Sha256::new();
        update_length_prefixed(&mut hasher, TAIL_START_DOMAIN_V1);
        update_length_prefixed(&mut hasher, partition_kind.partition_tag().as_bytes());
        hasher.update(start_record_count.to_be_bytes());
        Ok(Self {
            partition_kind,
            start_record_count,
            next_record_count: start_record_count,
            state: hasher.finalize().into(),
        })
    }

    /// Resume from a saved `(folded_end_record_count, state)` checkpoint pair.
    pub fn resume(
        partition_kind: TailPartitionKind,
        start_record_count: u64,
        folded_end_record_count: u64,
        state: &str,
    ) -> Result<Self, String> {
        validate_tail_start(partition_kind, start_record_count)?;
        if folded_end_record_count < start_record_count
            || folded_end_record_count > MAX_FILES as u64
        {
            return Err("portable tail resume count is outside the partition".to_string());
        }
        Ok(Self {
            partition_kind,
            start_record_count,
            next_record_count: folded_end_record_count,
            state: decode_sha256_hex("portable tail resume state", state)?,
        })
    }

    pub fn fold_message(&mut self, message: &CanonicalConversationMessage) -> Result<(), String> {
        message.validate()?;
        if self.next_record_count >= MAX_FILES as u64 {
            return Err("portable tail fold exceeds the record limit".to_string());
        }
        let mut hasher = Sha256::new();
        update_length_prefixed(&mut hasher, TAIL_RECORD_DOMAIN_V1);
        hasher.update(self.next_record_count.to_be_bytes());
        hasher.update(self.state);
        update_length_prefixed(&mut hasher, message.message_id.as_str().as_bytes());
        update_optional_length_prefixed(
            &mut hasher,
            message.parent_message_id.as_ref().map(MessageId::as_str),
        );
        update_length_prefixed(&mut hasher, message.semantic_fingerprint.as_bytes());
        self.state = hasher.finalize().into();
        self.next_record_count += 1;
        Ok(())
    }

    /// Absolute canonical count folded through; with `state_hex` this is the
    /// saved checkpoint pair.
    pub fn folded_end_record_count(&self) -> u64 {
        self.next_record_count
    }

    pub fn state_hex(&self) -> String {
        hex_lower(&self.state)
    }
}

/// One-shot fold over `chain[start..end]`, where the slice index in the
/// complete flattened chain is the absolute canonical index.
pub fn compute_conversation_tail_state(
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    end_record_count: u64,
    chain: &[CanonicalConversationMessage],
) -> Result<String, String> {
    validate_conversation_chain(chain)?;
    if start_record_count > end_record_count || end_record_count > chain.len() as u64 {
        return Err("portable tail window is outside the canonical chain".to_string());
    }
    let mut accumulator = TailAccumulator::seed(partition_kind, start_record_count)?;
    for message in &chain[start_record_count as usize..end_record_count as usize] {
        accumulator.fold_message(message)?;
    }
    Ok(accumulator.state_hex())
}

/// The complete five-field value for a fully parsed chain.  The accepted tail
/// always ends at the complete canonical root end, so the end is the chain
/// length, never a caller-chosen cut.
pub fn compute_portable_conversation_tail(
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    chain: &[CanonicalConversationMessage],
) -> Result<PortableConversationTail, String> {
    compute_portable_conversation_tail_with_codec(
        PORTABLE_TAIL_CODEC_V1,
        partition_kind,
        start_record_count,
        chain,
    )
}

/// [`compute_portable_conversation_tail`] under an explicit codec label. The
/// fold binds partition, start, and per-record fingerprints — never the
/// label — so the wire-form and local-form tails of the same conversation
/// share mechanics and differ exactly where their record bytes differ.
pub fn compute_portable_conversation_tail_with_codec(
    codec: &str,
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    chain: &[CanonicalConversationMessage],
) -> Result<PortableConversationTail, String> {
    let end_record_count = chain.len() as u64;
    let accepted_tail_state = compute_conversation_tail_state(
        partition_kind,
        start_record_count,
        end_record_count,
        chain,
    )?;
    let tail = PortableConversationTail {
        codec: codec.to_string(),
        partition_kind,
        start_record_count,
        accepted_end_record_count: end_record_count,
        accepted_tail_state,
    };
    tail.validate()?;
    Ok(tail)
}

/// The immutable `(codec, partition_kind, start)` slice of the portable
/// value, stored on the machine-local conversation binding.  The start never
/// mutates in place; the only layout replacement is the divergent keep-both
/// binding move, which transfers the local layout unchanged to the preserved
/// thread before the tracked thread imports the verified remote layout.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RootTailLayout {
    pub codec: String,
    pub partition_kind: TailPartitionKind,
    pub start_record_count: u64,
}

impl RootTailLayout {
    pub fn validate(&self) -> Result<(), String> {
        validate_tail_codec(&self.codec)?;
        validate_tail_start(self.partition_kind, self.start_record_count)
    }

    pub fn matches(&self, tail: &PortableConversationTail) -> bool {
        self.codec == tail.codec
            && self.partition_kind == tail.partition_kind
            && self.start_record_count == tail.start_record_count
    }
}

impl PortableConversationTail {
    pub fn layout(&self) -> RootTailLayout {
        RootTailLayout {
            codec: self.codec.clone(),
            partition_kind: self.partition_kind,
            start_record_count: self.start_record_count,
        }
    }
}

/// The accepted cloud version for one destination: integrity/provenance
/// witnesses plus the accepted portable tail.  The commit/manifest pair is
/// immutable provenance for ancestry proofs, never the CAS base.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcceptedConversationVersion {
    pub artifact_sha256: String,
    pub accepted_at_commit_id: String,
    pub accepted_at_manifest_sha256: String,
    pub portable_tail: PortableConversationTail,
}

impl AcceptedConversationVersion {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("accepted conversation artifact", &self.artifact_sha256)?;
        crate::validate_named_id("accepted conversation commit", &self.accepted_at_commit_id)?;
        validate_sha256(
            "accepted conversation manifest",
            &self.accepted_at_manifest_sha256,
        )?;
        self.portable_tail.validate()
    }
}

/// Machine-specific resume state mapping the portable partition into the
/// append-bearing local rollout.  Never serialized to cloud.  The probe
/// window is a bounded trailing-byte digest ending at the consumed offset;
/// it witnesses file identity for cursor reuse without re-reading the file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RootPhysicalMapping {
    pub provider_thread_id: String,
    pub root_path: String,
    pub source_file_size: u64,
    pub consumed_byte_offset: u64,
    pub consumed_record_ordinal: u64,
    pub probe_window_len: u64,
    pub probe_window_sha256: String,
    /// Canonical-projection resume cursors: the last folded message identity
    /// (implicit parents of appended records) and the id-less synthetic-turn
    /// resume pair. Without these an incremental fold could not reproduce
    /// the one-shot projection from the saved split point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_canonical_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synthetic_turn_id: Option<String>,
    pub next_synthetic_turn_ordinal: u64,
}

impl RootPhysicalMapping {
    pub fn validate(&self) -> Result<(), String> {
        crate::validate_provider_thread_id(
            "conversation mapping provider thread",
            &self.provider_thread_id,
        )?;
        crate::validate_absolute_clean_path("conversation mapping root", &self.root_path)?;
        if self.consumed_byte_offset > self.source_file_size {
            return Err("conversation mapping cursor exceeds its source size".to_string());
        }
        // The cursor ordinal is global across a native paginated lineage,
        // while the byte cursor is local to the append-bearing segment. They
        // are different coordinate spaces and cannot be ordered here.
        if self.probe_window_len > self.consumed_byte_offset {
            return Err("conversation mapping probe window exceeds its cursor".to_string());
        }
        if self.probe_window_len == 0 && self.consumed_byte_offset != 0 {
            return Err("conversation mapping probe window is empty".to_string());
        }
        if self
            .last_canonical_message_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err("conversation mapping has an invalid resume message id".to_string());
        }
        if self
            .last_synthetic_turn_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err("conversation mapping has an invalid resume turn id".to_string());
        }
        if self.next_synthetic_turn_ordinal > 0 && self.last_synthetic_turn_id.is_none() {
            return Err("conversation mapping resume ordinal lacks its turn".to_string());
        }
        validate_sha256(
            "conversation mapping probe window",
            &self.probe_window_sha256,
        )
    }
}

/// Staleness pin over the raw unaccepted physical interval
/// `[accepted physical end or tail start, observed safe end]`.  Planning
/// copies it into the plan; apply re-reads and re-hashes the same interval.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RawRangePin {
    pub start_byte_offset: u64,
    pub end_byte_offset: u64,
    pub range_sha256: String,
}

impl RawRangePin {
    pub fn validate(&self) -> Result<(), String> {
        if self.start_byte_offset > self.end_byte_offset {
            return Err("conversation raw range pin is inverted".to_string());
        }
        validate_sha256("conversation raw range pin", &self.range_sha256)
    }
}

/// Physical validity of the checkpoint's mapping.  `Detached` is the single
/// physical-invalidation state, set when another operation replaces the bound
/// alias: it preserves accepted cloud evidence, drops the mapping, and does
/// not pre-classify the eventual relation.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PhysicalMappingState {
    Ready {
        root_mapping: RootPhysicalMapping,
        raw_range_pin: RawRangePin,
    },
    Detached {
        expected_provider_thread_id: String,
        expected_binding_revision: u64,
    },
}

impl PhysicalMappingState {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Ready {
                root_mapping,
                raw_range_pin,
            } => {
                root_mapping.validate()?;
                raw_range_pin.validate()?;
                if raw_range_pin.end_byte_offset != root_mapping.consumed_byte_offset {
                    return Err(
                        "conversation raw range pin disagrees with its mapping cursor".to_string(),
                    );
                }
                Ok(())
            }
            Self::Detached {
                expected_provider_thread_id,
                ..
            } => crate::validate_provider_thread_id(
                "detached conversation mapping provider thread",
                expected_provider_thread_id,
            ),
        }
    }
}

/// The existing whole-chain digest pair truncated at the portable boundary,
/// over canonical messages `0..start_record_count`.  Machine-local, never
/// serialized to cloud, never a classification input: it is recomputed and
/// verified only by operations that full-parse the chain anyway.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPrefixWitness {
    pub lineage_digest: String,
    pub semantic_digest: String,
}

impl ConversationPrefixWitness {
    pub fn validate(&self) -> Result<(), String> {
        validate_sha256("conversation prefix witness lineage", &self.lineage_digest)?;
        validate_sha256(
            "conversation prefix witness semantics",
            &self.semantic_digest,
        )
    }
}

/// Compute the prefix witness for a complete flattened chain.  A zero start
/// has no prefix to witness and yields `None`; a nonzero start folds the
/// existing whole-chain digests over the boundary prefix.
pub fn conversation_prefix_witness(
    chain: &[CanonicalConversationMessage],
    start_record_count: u64,
) -> Result<Option<ConversationPrefixWitness>, String> {
    if start_record_count == 0 {
        return Ok(None);
    }
    if start_record_count > chain.len() as u64 {
        return Err("conversation prefix witness boundary is outside the chain".to_string());
    }
    let (lineage_digest, semantic_digest) =
        crate::conversation_chain_digests(&chain[..start_record_count as usize])?;
    Ok(Some(ConversationPrefixWitness {
        lineage_digest,
        semantic_digest,
    }))
}

/// One destination's per-thread conversation state: the accepted cloud
/// version, the replaceable observed cache, and the machine-specific physical
/// mapping.  Stored on the `ProjectStorageLink`, keyed by scope and thread;
/// two links may have accepted different versions of the same alias.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationCheckpoint {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<AcceptedConversationVersion>,
    /// The local-form twin of `accepted.portable_tail`, pinned atomically at
    /// the same acceptance event: the fold of this machine's real bytes at
    /// the accepted end.  Wire and local bytes differ under the path-mapped
    /// wire encoding, so every comparison is pin-relative — local
    /// observation compares here, never to the wire pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_local_tail: Option<PortableConversationTail>,
    /// Observed cumulative value under the same layout; replaceable Refresh
    /// cache, never approval.  Its `(end, state)` pair is the saved
    /// accumulator resume point, riding forward from `accepted_local_tail`'s
    /// accumulator state.
    pub observed_tail: PortableConversationTail,
    pub physical: PhysicalMappingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_witness: Option<ConversationPrefixWitness>,
    pub binding_revision: u64,
}

impl ConversationCheckpoint {
    pub fn validate(&self) -> Result<(), String> {
        self.observed_tail.validate()?;
        if self.accepted.is_some() != self.accepted_local_tail.is_some() {
            return Err(
                "conversation checkpoint must pin the wire and local accepted tails together"
                    .to_string(),
            );
        }
        if let (Some(accepted), Some(accepted_local)) = (&self.accepted, &self.accepted_local_tail)
        {
            accepted.validate()?;
            accepted_local.validate()?;
            // The local twin shares the observed fold's codec and layout;
            // the wire pin shares the partition and coordinates (records map
            // one-to-one across forms) under its own codec label.
            if !accepted_local.layout().matches(&self.observed_tail) {
                return Err(
                    "conversation checkpoint accepted-local and observed layouts disagree"
                        .to_string(),
                );
            }
            let wire = &accepted.portable_tail;
            if wire.partition_kind != accepted_local.partition_kind
                || wire.start_record_count != accepted_local.start_record_count
                || wire.accepted_end_record_count != accepted_local.accepted_end_record_count
            {
                return Err(
                    "conversation checkpoint wire and local accepted pins disagree on their \
                     partition or coordinates"
                        .to_string(),
                );
            }
        }
        self.physical.validate()?;
        let requires_witness = self.observed_tail.partition_kind
            == TailPartitionKind::FixedPaginatedTail
            && self.observed_tail.start_record_count > 0;
        if requires_witness != self.prefix_witness.is_some() {
            return Err(
                "conversation checkpoint prefix witness disagrees with its layout".to_string(),
            );
        }
        if let Some(witness) = &self.prefix_witness {
            witness.validate()?;
        }
        Ok(())
    }
}

fn decode_sha256_hex(label: &str, digest: &str) -> Result<[u8; 32], String> {
    validate_sha256(label, digest)?;
    let mut bytes = [0_u8; 32];
    for (index, chunk) in digest.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(chunk[0]) << 4) | hex_nibble(chunk[1]);
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        _ => byte - b'a' + 10,
    }
}
