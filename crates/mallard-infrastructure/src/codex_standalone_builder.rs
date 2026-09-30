//! Standalone reconstruction of one resolved Codex logical root.
//!
//! Push verifies physical source origins, maps new records, and redacts
//! credentials. The wire form keeps source records with tagged path
//! references and one authoritative metadata head. Redundant display
//! records are checked and omitted. The leaf of a copy that Keep both
//! preserved can be read by record digest instead of by position.
//!
//! Pull decodes references and captures their inverse edits. It then builds
//! native display records from the decoded sources. Generated display and
//! lifecycle records are marked derived in local origins. Thus HTML entity
//! decoding and file-URI formatting cannot turn display text into path syntax.
//!
//! Output analysis checks canonical message identities, digests, and the
//! fixed tail partition. Local hashes cover final provider IDs and ordinals.
//! Credential redaction is the only permitted conversation-text loss.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::time::{Duration, Instant};

use crate::app_settings::conversation_file_limit_bytes;
use crate::codex_history_graph::{ResolvedCodexRoot, ResolvedSegment};
use crate::codex_paginated_projection::project_with_sources;
use crate::codex_paginated_projection::verify_projection_fixpoint;
use crate::codex_rollout::{session_meta_id, structural_record_type};
use crate::conversation_codec::{analyze_codex_records, CodecLineage, CodexTranscriptAnalysis};
use crate::credential_redaction;
use crate::jsonl_tail_io::read_bounded;
use crate::payload::PayloadBytes;
use crate::session_path_map::{
    path_map_marker, validate_path_map_marker, SessionPathMapper, PATH_MAP_FIELD,
};
use crate::session_path_origins::{EncodeRefusal, OriginLookup, PathOrigins, RecordPathOrigins};
use crate::verified_transcript::VerifiedTranscript;
use mallard_domain::{
    compute_portable_conversation_tail_with_codec, conversation_prefix_witness, hex_lower,
    update_length_prefixed, ConversationPrefixWitness, PortableConversationTail,
    PORTABLE_TAIL_CODEC_V1, PORTABLE_TAIL_CODEC_WIRE_V1,
};

const ARTIFACT_DOMAIN_V1: &[u8] = b"mallard-codex-artifact-v1";

enum BuildMode<'a> {
    Encode {
        root: &'a str,
        canonical_root: Option<&'a str>,
    },
    Decode {
        root_provider_thread_id: &'a str,
        target_root: &'a str,
        canonical_target_root: Option<&'a str>,
    },
}

/// The verified output of one build. Every derived value — tail, digests,
/// witness — belongs to the **output form**: wire values from `Encode`,
/// local values from `Decode`. Forms are never cross-compared.
#[derive(Clone, Debug)]
pub struct StandaloneConversationArtifact<B> {
    pub root_provider_thread_id: String,
    pub root_bytes: B,
    pub path_origins: PathOrigins,
    /// Output-form portable tail: `PORTABLE_TAIL_CODEC_WIRE_V1` from
    /// `Encode`, `PORTABLE_TAIL_CODEC_V1` from `Decode`.
    pub portable_tail: PortableConversationTail,
    /// Output-form prefix witness over the analyzed output chain (present
    /// exactly when the effective partition has a nonzero fixed start).
    pub prefix_witness: Option<ConversationPrefixWitness>,
    pub message_count: u64,
    /// Canonical projection cursors for append-only refresh of these exact
    /// output bytes after Pull installs them.
    pub last_canonical_message_id: Option<String>,
    pub last_synthetic_turn_id: Option<String>,
    pub next_synthetic_turn_ordinal: u64,
    pub lineage_digest: String,
    pub semantic_digest: String,
    pub artifact_sha256: String,
    pub(crate) verification: VerifiedTranscript,
    pub(crate) output_verification_elapsed: Duration,
}

/// Build the deterministic standalone encoding of one resolved root and
/// verify it end to end: the emitted bytes must re-analyze to a canonical
/// chain whose message identities exactly match the resolved chain's, and
/// the output-form tail is refolded from that analysis. Fingerprint equality
/// across forms is deliberately not required — the path layer changes
/// content, and the §4.4 gates prove it changed nothing else. `layout`
/// carries the stored binding layout when one exists, so a republish from a
/// pulled machine keeps the imported paginated partition instead of moving
/// the start to the local file's null native base.
pub fn encode_standalone_codex_root(
    resolved: &ResolvedCodexRoot,
    root: &str,
    canonical_root: Option<&str>,
    layout: Option<&mallard_domain::RootTailLayout>,
    redactor: &mut credential_redaction::RedactionRun<'_>,
    origins: &PathOrigins,
) -> Result<StandaloneConversationArtifact<credential_redaction::RedactedWireBytes>, String> {
    encode_codex_root_with_reader(
        resolved,
        root,
        canonical_root,
        layout,
        redactor,
        origins,
        &|segment| read_bounded(&segment.path).map(Cow::Owned),
    )
}

/// Encode the same snapshots used for resolution and checkpoint seeding.
pub fn encode_captured_codex_root(
    resolved: &ResolvedCodexRoot,
    sources: &crate::codex_history_graph::CodexSourceSnapshots,
    root: &str,
    canonical_root: Option<&str>,
    layout: Option<&mallard_domain::RootTailLayout>,
    redactor: &mut credential_redaction::RedactionRun<'_>,
    origins: &PathOrigins,
) -> Result<StandaloneConversationArtifact<credential_redaction::RedactedWireBytes>, String> {
    let verify_sources = || -> Result<(), String> {
        for (path, (_, stamp)) in sources {
            stamp.verify(path)?;
        }
        Ok(())
    };
    verify_sources()?;
    let artifact = encode_codex_root_with_reader(
        resolved,
        root,
        canonical_root,
        layout,
        redactor,
        origins,
        &|segment| {
            sources
                .get(&segment.path)
                .map(|(bytes, _)| Cow::Borrowed(bytes.as_bytes()))
                .ok_or_else(|| "resolved Codex segment has no captured bytes".to_string())
        },
    )?;
    verify_sources()?;
    Ok(artifact)
}

fn encode_codex_root_with_reader(
    resolved: &ResolvedCodexRoot,
    root: &str,
    canonical_root: Option<&str>,
    layout: Option<&mallard_domain::RootTailLayout>,
    redactor: &mut credential_redaction::RedactionRun<'_>,
    origins: &PathOrigins,
    read_segment: &SegmentReader<'_>,
) -> Result<StandaloneConversationArtifact<credential_redaction::RedactedWireBytes>, String> {
    let artifact = reconstruct_codex_root(
        resolved,
        BuildMode::Encode {
            root,
            canonical_root,
        },
        layout,
        redactor,
        origins,
        OriginLookup::ByPosition,
        &std::collections::BTreeMap::new(),
        read_segment,
    )?;
    Ok(StandaloneConversationArtifact {
        root_provider_thread_id: artifact.root_provider_thread_id,
        root_bytes: credential_redaction::RedactedWireBytes::from_verified_payload(
            artifact.root_bytes,
        ),
        path_origins: artifact.path_origins,
        portable_tail: artifact.portable_tail,
        prefix_witness: artifact.prefix_witness,
        message_count: artifact.message_count,
        last_canonical_message_id: artifact.last_canonical_message_id,
        last_synthetic_turn_id: artifact.last_synthetic_turn_id,
        next_synthetic_turn_ordinal: artifact.next_synthetic_turn_ordinal,
        lineage_digest: artifact.lineage_digest,
        semantic_digest: artifact.semantic_digest,
        artifact_sha256: artifact.artifact_sha256,
        verification: artifact.verification,
        output_verification_elapsed: artifact.output_verification_elapsed,
    })
}

/// Filesystem fixture helper. Production Pull decodes its downloaded bytes
/// directly; this helper is absent from production builds.
#[cfg(any(test, feature = "test-support"))]
pub fn decode_standalone_codex_root(
    resolved: &ResolvedCodexRoot,
    root_provider_thread_id: &str,
    target_root: &str,
    canonical_target_root: Option<&str>,
    layout: Option<&mallard_domain::RootTailLayout>,
) -> Result<StandaloneConversationArtifact<PayloadBytes>, String> {
    let mut redaction = credential_redaction::shared().begin_run();
    Ok(reconstruct_codex_root(
        resolved,
        BuildMode::Decode {
            root_provider_thread_id,
            target_root,
            canonical_target_root,
        },
        layout,
        &mut redaction,
        &PathOrigins::default(),
        OriginLookup::ByPosition,
        &std::collections::BTreeMap::new(),
        &|segment| read_bounded(&segment.path).map(Cow::Owned),
    )?)
}

/// Encode one member of the selected tree from its captured sources.
/// `leaf_lookup` is where the saved origins of the leaf segment are looked
/// for; the segments of its ancestors are always read by position.
#[allow(clippy::too_many_arguments)]
pub fn encode_codex_tree_member(
    resolved: &ResolvedCodexRoot,
    sources: &crate::codex_history_graph::CodexSourceSnapshots,
    root: &str,
    canonical_root: Option<&str>,
    layout: Option<&mallard_domain::RootTailLayout>,
    redactor: &mut credential_redaction::RedactionRun<'_>,
    origins: &PathOrigins,
    leaf_lookup: OriginLookup,
    identities: &std::collections::BTreeMap<String, String>,
) -> Result<StandaloneConversationArtifact<credential_redaction::RedactedWireBytes>, EncodeRefusal>
{
    let artifact = reconstruct_codex_root(
        resolved,
        BuildMode::Encode {
            root,
            canonical_root,
        },
        layout,
        redactor,
        origins,
        leaf_lookup,
        identities,
        &|segment| {
            sources
                .get(&segment.path)
                .map(|(bytes, _)| Cow::Borrowed(bytes.as_bytes()))
                .ok_or_else(|| "tree source was not captured".into())
        },
    )?;
    Ok(StandaloneConversationArtifact {
        root_bytes: credential_redaction::RedactedWireBytes::from_verified_payload(
            artifact.root_bytes,
        ),
        root_provider_thread_id: artifact.root_provider_thread_id,
        path_origins: artifact.path_origins,
        portable_tail: artifact.portable_tail,
        prefix_witness: artifact.prefix_witness,
        message_count: artifact.message_count,
        last_canonical_message_id: artifact.last_canonical_message_id,
        last_synthetic_turn_id: artifact.last_synthetic_turn_id,
        next_synthetic_turn_ordinal: artifact.next_synthetic_turn_ordinal,
        lineage_digest: artifact.lineage_digest,
        semantic_digest: artifact.semantic_digest,
        artifact_sha256: artifact.artifact_sha256,
        verification: artifact.verification,
        output_verification_elapsed: artifact.output_verification_elapsed,
    })
}

pub fn decode_codex_tree_member(
    resolved: &ResolvedCodexRoot,
    bytes: &[u8],
    destination: &str,
    root: &str,
    canonical_root: Option<&str>,
    layout: &mallard_domain::RootTailLayout,
    identities: &std::collections::BTreeMap<String, String>,
    redaction: &mut credential_redaction::RedactionRun<'_>,
) -> Result<StandaloneConversationArtifact<PayloadBytes>, String> {
    Ok(reconstruct_codex_root(
        resolved,
        BuildMode::Decode {
            root_provider_thread_id: destination,
            target_root: root,
            canonical_target_root: canonical_root,
        },
        Some(layout),
        redaction,
        &PathOrigins::default(),
        OriginLookup::ByPosition,
        identities,
        &|_| Ok(Cow::Borrowed(bytes)),
    )?)
}

type SegmentReader<'a> = dyn Fn(&ResolvedSegment) -> Result<Cow<'a, [u8]>, String> + 'a;

#[allow(clippy::too_many_arguments)]
fn reconstruct_codex_root(
    resolved: &ResolvedCodexRoot,
    mode: BuildMode<'_>,
    layout: Option<&mallard_domain::RootTailLayout>,
    redactor: &mut credential_redaction::RedactionRun<'_>,
    previous_origins: &PathOrigins,
    leaf_lookup: OriginLookup,
    identities: &std::collections::BTreeMap<String, String>,
    read_segment: &SegmentReader<'_>,
) -> Result<StandaloneConversationArtifact<PayloadBytes>, EncodeRefusal> {
    let home = crate::session_path_map::home_dir_text();
    let (mut output_root_id, mapper, encode_mode) = match mode {
        BuildMode::Encode {
            root,
            canonical_root,
        } => (
            resolved.leaf_meta.id.clone(),
            SessionPathMapper::new(root, canonical_root, home.as_deref())?,
            true,
        ),
        BuildMode::Decode {
            root_provider_thread_id,
            target_root,
            canonical_target_root,
        } => (
            root_provider_thread_id.to_string(),
            SessionPathMapper::new(target_root, canonical_target_root, home.as_deref())?,
            false,
        ),
    };

    if encode_mode {
        if let Some(mapped) = identities.get(&resolved.leaf_meta.id) {
            output_root_id = mapped.clone();
        }
    }
    // Transform each physical source record before folding metadata. This
    // preserves exact record ownership through metadata updates and forks.
    let mut path_origins = PathOrigins::default();
    let (mut authoritative, mut original) = read_source_records(
        resolved,
        read_segment,
        &mapper,
        encode_mode,
        redactor,
        previous_origins,
        leaf_lookup,
        &mut path_origins,
    )?;
    if encode_mode {
        rewrite_meta_record(&mut authoritative, &output_root_id)?;
        head_payload_mut(&mut authoritative)?.insert(PATH_MAP_FIELD.to_string(), path_map_marker());
        redactor.verify_wire_value(&authoritative)?;
    } else {
        let marker = head_payload_mut(&mut authoritative)?
            .remove(PATH_MAP_FIELD)
            .ok_or("conversation wire object lacks its path-map declaration")?;
        validate_path_map_marker(&marker)?;
        rewrite_meta_record(&mut authoritative, &output_root_id)?;
    }
    let child_boundary = if resolved.leaf_meta.is_internal_subagent {
        let boundary = authoritative["payload"]["subagent_history_start_ordinal"]
            .as_u64()
            .ok_or("child has no native history boundary")?;
        Some(boundary)
    } else {
        None
    };
    crate::codex_subagents::rewrite_references(&mut authoritative, identities);
    let mut output = Vec::new();
    let mut output_records = Vec::new();
    let mut output_boundary = None;
    let mut output_head = authoritative.clone();
    if encode_mode {
        // Classify redundant display records in the native source form.
        // Path mapping must not precede HTML decoding or URI projection.
        let native = project_with_sources(
            original.iter().map(|record| &record.local),
            &output_root_id,
            &mut |_| Ok(()),
        )?
        .into_iter()
        .map(|record| (record.source_index, record.derived))
        .collect::<Vec<_>>();
        // Retain the redaction gate for secrets exposed by display decoding.
        // Generated wire projections are proof inputs, never published.
        project_with_sources(
            original.iter().map(|record| &record.wire),
            &output_root_id,
            &mut |record| redactor.verify_wire_value(record),
        )?;
        let mut ordinal = 1;
        for (source_index, derived) in native {
            let Some(index) = source_index else {
                continue;
            };
            let source = &mut original[index];
            if derived && !source.covered {
                path_origins
                    .sources
                    .get_mut(source.source)
                    .expect("captured source")[source.ordinal]
                    .derived = true;
                continue;
            }
            if child_boundary.is_some_and(|boundary| source.native_ordinal >= boundary)
                && output_boundary.is_none()
            {
                output_boundary = Some(ordinal);
            }
            let mut wire = std::mem::take(&mut source.wire);
            crate::codex_subagents::rewrite_references(&mut wire, identities);
            normalize_item_identity(&mut wire, &output_root_id);
            mapper.verify_wire_record(&wire)?;
            ordinal = append_output_record(&mut output, &mut output_records, wire, ordinal)?;
        }
        output_boundary.get_or_insert(ordinal);
    } else {
        authoritative
            .as_object_mut()
            .ok_or("metadata is not an object")?
            .insert("ordinal".into(), Value::from(0));
        let head = mapper.decode_record_with_origins(&authoritative)?;
        output_head = head.local;
        let source_ordinals: Vec<_> = original
            .iter()
            .map(|source| source.native_ordinal)
            .collect();
        let decoded = original
            .into_iter()
            .map(|source| {
                let mut wire = source.wire;
                crate::codex_subagents::rewrite_references(&mut wire, identities);
                normalize_item_identity(&mut wire, &output_root_id);
                mapper.decode_record_with_origins(&wire)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let projected = project_with_sources(
            decoded.iter().map(|record| &record.local),
            &output_root_id,
            &mut |_| Ok(()),
        )?;
        for (index, projected) in projected.into_iter().enumerate() {
            if child_boundary.is_some_and(|boundary| {
                projected
                    .source_index
                    .is_some_and(|source| source_ordinals[source] >= boundary)
            }) && output_boundary.is_none()
            {
                output_boundary = Some((index + 1) as u64);
            }
            let mut record = projected.value.into_owned();
            let (fields, derived) = if let Some(source) = projected.source_index {
                if record != decoded[source].local {
                    return Err("projection changed retained source text".into());
                }
                (decoded[source].origins.fields.clone(), false)
            } else {
                (Vec::new(), true)
            };
            record
                .as_object_mut()
                .ok_or("projected record is not an object")?
                .insert("ordinal".into(), Value::from((index + 1) as u64));
            let mut origins = RecordPathOrigins::capture(&record, fields)?;
            origins.derived = derived;
            path_origins.push(&output_root_id, origins);
            append_output_record(&mut output, &mut output_records, record, (index + 1) as u64)?;
        }
    }
    if child_boundary.is_some() {
        let boundary = output_boundary
            .unwrap_or_else(|| output.iter().filter(|byte| **byte == b'\n').count() as u64 + 1);
        output_head["payload"]["subagent_history_start_ordinal"] = Value::from(boundary);
    }
    let mut with_head = Vec::new();
    let mut final_records = Vec::with_capacity(output_records.len() + 1);
    append_output_record(&mut with_head, &mut final_records, output_head.clone(), 0)?;
    final_records.extend(output_records);
    with_head.extend(output);
    output = with_head;
    if !encode_mode {
        let head = mapper.decode_record_with_origins(&authoritative)?;
        let head_origins = RecordPathOrigins::capture(&output_head, head.origins.fields)?;
        path_origins
            .sources
            .entry(output_root_id.clone())
            .or_default()
            .insert(0, head_origins);
    }
    if output.len() as u64 > conversation_file_limit_bytes() {
        return Err("standalone Codex output exceeds the bounded transcript limit".into());
    }
    let output_verification_started = Instant::now();
    if !encode_mode {
        validate_projection_fixpoint(&final_records, &output_root_id)?;
    }
    let analysis = analyze_standalone_output(&output, final_records.into_iter().map(Ok))?;
    let output_record_count = analysis.record_count as u64;
    if child_boundary.is_some() {
        let boundary = output_head["payload"]["subagent_history_start_ordinal"].as_u64();
        if !boundary.is_some_and(|boundary| boundary > 0 && boundary <= output_record_count) {
            return Err("standalone child output has an invalid native boundary".into());
        }
    }

    // Full verification: the output must re-analyze to a canonical chain
    // with exactly the resolved chain's message identities under the output
    // identity, and the output-form tail is refolded from that analysis.
    if analysis.source_thread_id.as_deref() != Some(output_root_id.as_str()) {
        return Err("standalone output lost its root identity".into());
    }
    if analysis.is_internal_subagent != resolved.leaf_meta.is_internal_subagent {
        return Err("standalone output changed its subagent role".into());
    }
    let output = PayloadBytes::from(output);
    let verification = VerifiedTranscript::from_codex_analysis(output.clone(), &analysis)?;
    let last_canonical_message_id = analysis.last_canonical_message_id.clone();
    let last_synthetic_turn_id = analysis.last_synthetic_turn_id.clone();
    let next_synthetic_turn_ordinal = analysis.next_synthetic_turn_ordinal;
    let (chain, lineage_digest, semantic_digest) = match analysis.lineage {
        CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        } => (chain, lineage_digest, semantic_digest),
        CodecLineage::Unknown { reason } => {
            return Err(format!("standalone output failed re-analysis: {:?}", reason).into());
        }
    };
    if chain.len() != resolved.chain.len()
        || chain
            .iter()
            .zip(resolved.chain.iter())
            .any(|(output_message, source_message)| {
                output_message.message_id != source_message.message_id
                    || output_message.parent_message_id != source_message.parent_message_id
            })
    {
        return Err("standalone output changed the canonical chain identities".into());
    }
    let (partition_kind, start_record_count) =
        crate::codex_history_graph::effective_tail_partition(layout, resolved)?;
    let tail_codec = if encode_mode {
        PORTABLE_TAIL_CODEC_WIRE_V1
    } else {
        PORTABLE_TAIL_CODEC_V1
    };
    let portable_tail = compute_portable_conversation_tail_with_codec(
        tail_codec,
        partition_kind,
        start_record_count,
        &chain,
    )?;
    let prefix_witness = conversation_prefix_witness(&chain, start_record_count)?;

    let artifact_sha256 = artifact_digest(&output, &output_root_id);
    Ok(StandaloneConversationArtifact {
        root_provider_thread_id: output_root_id,
        message_count: chain.len() as u64,
        root_bytes: output,
        path_origins,
        portable_tail,
        prefix_witness,
        last_canonical_message_id,
        last_synthetic_turn_id,
        next_synthetic_turn_ordinal,
        lineage_digest,
        semantic_digest,
        artifact_sha256,
        verification,
        output_verification_elapsed: output_verification_started.elapsed(),
    })
}

/// Prove the exact serialized values are a projection fixpoint. The complete
/// records retain their ordinals for the separate standalone layout check.
fn validate_projection_fixpoint(output: &[Value], output_root_id: &str) -> Result<(), String> {
    let mut records = Vec::new();
    for value in output {
        if structural_record_type(value) == Some("session_meta") {
            continue;
        }
        let mut value = value.clone();
        if let Some(record) = value.as_object_mut() {
            record.remove("ordinal");
        }
        records.push(value);
    }
    verify_projection_fixpoint(&records, output_root_id)
}

/// Domain-separated integrity digest over the complete root output and the
/// ordered `(file role, provider ID, byte length, SHA-256)` file list.
fn artifact_digest(root_bytes: &PayloadBytes, root_provider_thread_id: &str) -> String {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, ARTIFACT_DOMAIN_V1);
    hasher.update(1_u64.to_be_bytes());
    update_length_prefixed(&mut hasher, b"transcript");
    update_length_prefixed(&mut hasher, root_provider_thread_id.as_bytes());
    hasher.update((root_bytes.len() as u64).to_be_bytes());
    hasher.update(root_bytes.sha256_digest());
    hex_lower(&hasher.finalize())
}

fn normalize_item_identity(record: &mut Value, root: &str) {
    if record.get("type").and_then(Value::as_str) == Some("event_msg")
        && record.pointer("/payload/type").and_then(Value::as_str) == Some("item_completed")
    {
        if let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut) {
            payload.insert("thread_id".into(), Value::String(root.to_string()));
        }
    }
}

struct SourceRecord<'a> {
    local: Value,
    wire: Value,
    source: &'a str,
    ordinal: usize,
    covered: bool,
    native_ordinal: u64,
}

/// Parse each source once, folding the leaf metadata and collecting its body.
#[allow(clippy::too_many_arguments)]
fn read_source_records<'a>(
    resolved: &'a ResolvedCodexRoot,
    read_segment: &SegmentReader<'_>,
    mapper: &SessionPathMapper,
    encode_mode: bool,
    redactor: &mut credential_redaction::RedactionRun<'_>,
    previous: &PathOrigins,
    leaf_lookup: OriginLookup,
    origins: &mut PathOrigins,
) -> Result<(Value, Vec<SourceRecord<'a>>), EncodeRefusal> {
    let mut effective: Option<Value> = None;
    let mut records = Vec::new();
    let mut activity_events = std::collections::BTreeMap::new();
    for (index, segment) in resolved.segments.iter().enumerate() {
        let leaf = index + 1 == resolved.segments.len();
        let lookup = if leaf {
            leaf_lookup
        } else {
            OriginLookup::ByPosition
        };
        let mut saved = previous.saved(&segment.rollout_id, lookup);
        let bytes = read_segment(segment)?;
        if (bytes.len() as u64) < segment.consumed_byte_len {
            return Err(format!(
                "Codex segment '{}' shrank below its resolved cutoff",
                segment.provider_thread_id
            )
            .into());
        }
        let mut record_number = 0;
        for raw in bytes[..segment.consumed_byte_len as usize].split(|byte| *byte == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            record_number += 1;
            let value: Value = serde_json::from_slice(raw).map_err(|_| {
                format!(
                    "Codex segment '{}' contains malformed JSON",
                    segment.provider_thread_id
                )
            })?;
            if !encode_mode {
                if let Some(activity) =
                    crate::codex_subagents::SubagentActivity::from_record(&value)?
                {
                    crate::codex_subagents::verify_activity_identity(
                        &mut activity_events,
                        &activity,
                    )?;
                }
            }
            let prior = saved.origin_of(&value)?;
            if encode_mode {
                if let Some(prior) = prior.filter(|origin| origin.derived) {
                    prior.verify_record(&value)?;
                    origins.push(&segment.rollout_id, prior.clone());
                    continue;
                }
            }
            let native_ordinal = value["ordinal"]
                .as_u64()
                .unwrap_or((record_number - 1) as u64);
            let mut local = value.clone();
            let mut value = (|| -> Result<Value, EncodeRefusal> {
                if encode_mode {
                    let encoded = mapper.encode_record_with_origins(&value, prior)?;
                    origins.push(&segment.rollout_id, encoded.origins);
                    let (wire, changed) = redactor.redact_record_gated(&encoded.wire)?;
                    if changed {
                        mapper.verify_wire_record(&wire)?;
                    }
                    Ok(wire)
                } else {
                    mapper.verify_wire_record(&value)?;
                    redactor.verify_wire_value(&value)?;
                    Ok(value)
                }
            })()
            .map_err(|refusal| {
                refusal.at(format_args!(
                    "Codex segment '{}' record {}",
                    segment.provider_thread_id, record_number
                ))
            })?;
            if structural_record_type(&value) != Some("session_meta") {
                if let Some(record) = value.as_object_mut() {
                    record.remove("ordinal");
                }
                if let Some(record) = local.as_object_mut() {
                    record.remove("ordinal");
                }
                records.push(SourceRecord {
                    local,
                    wire: value,
                    source: &segment.provider_thread_id,
                    ordinal: record_number - 1,
                    covered: prior.is_some(),
                    native_ordinal,
                });
                continue;
            }
            if !leaf {
                continue;
            }
            let payload = value
                .get("payload")
                .and_then(Value::as_object)
                .ok_or("session metadata record has no payload")?;
            if session_meta_id(payload) != Some(resolved.leaf_meta.id.as_str()) {
                continue;
            }
            match effective.as_mut() {
                Some(current) => fold_session_meta(current, &value)?,
                None => effective = Some(value),
            }
        }
        if encode_mode && lookup == OriginLookup::ByPosition {
            previous.verify_source_length(&segment.provider_thread_id, record_number, !leaf)?;
        }
    }
    Ok((
        effective.ok_or("Codex leaf has no session metadata")?,
        records,
    ))
}

/// Merge fields explicitly carried by a later same-thread metadata update.
/// Missing optional fields do not erase earlier values (notably `git` when a
/// memory-only update is appended). The original creation timestamp remains
/// authoritative.
fn fold_session_meta(current: &mut Value, update: &Value) -> Result<(), String> {
    let current_payload = current
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "authoritative session metadata has no payload".to_string())?;
    let update_payload = update
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| "session metadata update has no payload".to_string())?;
    for (key, value) in update_payload {
        if key != "timestamp" {
            current_payload.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn head_payload_mut(value: &mut Value) -> Result<&mut Map<String, Value>, String> {
    value
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "session metadata record has no payload".to_string())
}

/// Rewrite the effective session metadata to the output root's identity:
/// paginated mode, explicit null base, and no fork or parent linkage. The
/// cwd is part of the path layer, never touched here.
fn rewrite_meta_record(value: &mut Value, output_root_id: &str) -> Result<(), String> {
    let payload = head_payload_mut(value)?;
    rewrite_identity_fields(payload, output_root_id);
    Ok(())
}

fn rewrite_identity_fields(payload: &mut Map<String, Value>, output_root_id: &str) {
    for key in ["id", "thread_id", "threadId"] {
        if payload.contains_key(key) {
            payload.insert(key.to_string(), Value::String(output_root_id.to_string()));
        }
    }
    if !payload.contains_key("id") {
        payload.insert("id".to_string(), Value::String(output_root_id.to_string()));
    }
    for key in ["session_id", "sessionId"] {
        if payload.contains_key(key) {
            payload.insert(key.to_string(), Value::String(output_root_id.to_string()));
        }
    }
    for key in ["forked_from_id", "forkedFromId"] {
        payload.remove(key);
    }
    payload.insert(
        "history_mode".to_string(),
        Value::String("paginated".to_string()),
    );
    payload.remove("historyMode");
    payload.insert("history_base".to_string(), Value::Null);
    payload.remove("historyBase");
}

fn append_output_record(
    output: &mut Vec<u8>,
    records: &mut Vec<Value>,
    mut value: Value,
    ordinal: u64,
) -> Result<u64, String> {
    let record = value
        .as_object_mut()
        .ok_or_else(|| "Codex rollout record is not a JSON object".to_string())?;
    record.insert("ordinal".to_string(), Value::from(ordinal));
    serde_json::to_writer(&mut *output, &value)
        .map_err(|_| "serialize standalone Codex record".to_string())?;
    output.push(b'\n');
    records.push(value);
    ordinal
        .checked_add(1)
        .ok_or_else(|| "standalone Codex ordinal overflow".to_string())
}

/// Check standalone layout and analyze the exact values serialized above.
fn analyze_standalone_output(
    bytes: &[u8],
    input: impl Iterator<Item = Result<Value, serde_json::Error>>,
) -> Result<CodexTranscriptAnalysis, String> {
    let mut expected_ordinal = 0_u64;
    let mut session_meta_count = 0_u64;
    let mut failure = None;
    let records = input.inspect(|record| {
        if failure.is_some() {
            return;
        }
        let checked = (|| -> Result<(), String> {
            let value = record
                .as_ref()
                .map_err(|_| "standalone Codex output contains malformed JSON".to_string())?;
            let ordinal = value
                .get("ordinal")
                .and_then(Value::as_u64)
                .ok_or_else(|| "standalone Codex output record has no ordinal".to_string())?;
            if ordinal != expected_ordinal {
                return Err(format!(
                    "standalone Codex output expected ordinal {expected_ordinal}, got {ordinal}"
                ));
            }
            if structural_record_type(value) == Some("session_meta") {
                session_meta_count += 1;
                if ordinal != 0 {
                    return Err("standalone Codex output has non-head session metadata".into());
                }
            }
            expected_ordinal = expected_ordinal
                .checked_add(1)
                .ok_or_else(|| "standalone Codex ordinal overflow".to_string())?;
            Ok(())
        })();
        if let Err(error) = checked {
            failure = Some(error);
        }
    });
    let analysis = analyze_codex_records(bytes, records);
    if let Some(error) = failure {
        return Err(error);
    }
    if let CodecLineage::Unknown { reason } = &analysis.lineage {
        return Err(format!("standalone output failed re-analysis: {reason:?}"));
    }
    if session_meta_count != 1 {
        return Err(format!(
            "standalone Codex output has {session_meta_count} session metadata records"
        ));
    }
    Ok(analysis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_history_graph::resolve_codex_root;
    use crate::conversation_codec::{analyze_codex_transcript, parsed_records};
    use crate::session_path_map::ROOT_TOKEN;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const SOURCE_ROOT: &str = "/tmp/project";

    fn meta_line(id: &str, base: Option<(&str, u64, u64)>) -> String {
        let mut payload = json!({
            "id": id,
            "cwd": SOURCE_ROOT,
            "history_mode": "paginated",
            "forked_from_id": null,
        });
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

    #[test]
    fn combined_output_check_rejects_invalid_layout_and_malformed_suffix() {
        let meta: Value = serde_json::from_str(&meta_line("root", None)).unwrap();
        let message: Value = serde_json::from_str(&message_line("m1", "hello")).unwrap();
        let mut valid = Vec::new();
        let mut retained = Vec::new();
        append_output_record(&mut valid, &mut retained, meta.clone(), 0).unwrap();
        append_output_record(&mut valid, &mut retained, message.clone(), 1).unwrap();
        let combined = analyze_standalone_output(&valid, retained.into_iter().map(Ok)).unwrap();
        let separate = analyze_codex_transcript(&valid);
        assert_eq!(combined.record_count, separate.record_count);
        assert_eq!(combined.lineage, separate.lineage);
        for suffix in [
            format!("{message}\n"),
            format!(
                "{}\n",
                json!({"type":"event_msg","ordinal":3,"payload":{"type":"token_count"}})
            ),
            format!(
                "{}\n",
                json!({"type":"session_meta","ordinal":2,"payload":meta["payload"]})
            ),
            "{malformed\n".to_string(),
        ] {
            let mut invalid = valid.clone();
            invalid.extend_from_slice(suffix.as_bytes());
            assert!(analyze_standalone_output(&invalid, parsed_records(&invalid)).is_err());
        }
        let mut missing_head = Vec::new();
        append_output_record(&mut missing_head, &mut Vec::new(), message, 0).unwrap();
        assert!(analyze_standalone_output(&missing_head, parsed_records(&missing_head)).is_err());
    }

    #[test]
    fn retained_projection_check_matches_serialized_output_and_rejects_missing_display_records() {
        let wire = include_bytes!("fixtures/standalone_codex_wire.jsonl");
        let resolved = crate::codex_history_graph::resolve_standalone_codex_bytes(wire).unwrap();
        let artifact = decode_codex_tree_member(
            &resolved,
            wire,
            "fresh-root",
            "/fixture/destination",
            None,
            &mallard_domain::RootTailLayout {
                codec: PORTABLE_TAIL_CODEC_V1.into(),
                partition_kind: mallard_domain::TailPartitionKind::WholeThread,
                start_record_count: 0,
            },
            &BTreeMap::new(),
            &mut credential_redaction::shared().begin_run(),
        )
        .unwrap();
        let records = parsed_records(&artifact.root_bytes)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let before = records.clone();
        validate_projection_fixpoint(&records, "fresh-root").unwrap();
        assert_eq!(records, before);
        let normalized: Vec<_> = records
            .iter()
            .filter(|record| structural_record_type(record) != Some("session_meta"))
            .cloned()
            .map(|mut record| {
                record.as_object_mut().unwrap().remove("ordinal");
                record
            })
            .collect();
        verify_projection_fixpoint(&normalized, "fresh-root").unwrap();
        let mut missing_display = records;
        for payload in [
            json!({"type": "task_started", "turn_id": "turn-1"}),
            json!({"type": "user_message", "message": "Hello", "images": [],
                "local_images": [], "text_elements": []}),
        ] {
            missing_display.push(json!({
                "ordinal": missing_display.len(),
                "timestamp": "2026-08-12T10:01:00Z", "type": "event_msg", "payload": payload,
            }));
        }
        assert!(validate_projection_fixpoint(&missing_display, "fresh-root")
            .unwrap_err()
            .contains("not a fixpoint"));
    }

    #[test]
    fn retained_output_values_match_serialized_ordinals_and_float_bits() {
        let meta: Value = serde_json::from_str(&meta_line("root", None)).unwrap();
        let mut message: Value = serde_json::from_str(&message_line("m1", "hello")).unwrap();
        message["ordinal"] = json!(99);
        message["numbers"] = serde_json::from_str::<Value>(
            "[-0.0,0.99177921181141328,5e-324,1.7976931348623157e308]",
        )
        .unwrap();
        let mut bytes = Vec::new();
        let mut retained = Vec::new();
        append_output_record(&mut bytes, &mut retained, meta, 0).unwrap();
        append_output_record(&mut bytes, &mut retained, message, 1).unwrap();
        let reparsed = parsed_records(&bytes)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(retained[1]["ordinal"], json!(1));
        for (before, after) in retained[1]["numbers"]
            .as_array()
            .unwrap()
            .iter()
            .zip(reparsed[1]["numbers"].as_array().unwrap())
        {
            assert_eq!(
                before.as_f64().unwrap().to_bits(),
                after.as_f64().unwrap().to_bits()
            );
        }
        let expected = retained
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>();
        assert_eq!(bytes, expected.as_bytes());
        let direct = analyze_standalone_output(&bytes, retained.into_iter().map(Ok)).unwrap();
        let parsed = analyze_standalone_output(&bytes, reparsed.into_iter().map(Ok)).unwrap();
        assert_eq!(direct.lineage, parsed.lineage);
        assert_eq!(direct.record_count, parsed.record_count);
    }

    fn write(
        directory: &tempfile::TempDir,
        files: &mut BTreeMap<String, PathBuf>,
        thread: &str,
        lines: &[String],
    ) -> PathBuf {
        let path = directory.path().join(format!("{thread}.jsonl"));
        let mut values = lines
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let mut ordinal = values
            .first()
            .and_then(|value| value.get("payload"))
            .and_then(|payload| payload.get("history_base"))
            .and_then(|base| base.get("end_ordinal_exclusive"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mut bytes = Vec::new();
        for value in &mut values {
            value["ordinal"] = Value::from(ordinal);
            ordinal = ordinal.checked_add(1).unwrap();
            serde_json::to_writer(&mut bytes, value).unwrap();
            bytes.push(b'\n');
        }
        std::fs::write(&path, bytes).unwrap();
        files.insert(thread.to_string(), path.clone());
        path
    }

    fn output_records(bytes: &[u8]) -> Vec<Value> {
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|raw| !raw.is_empty())
            .map(|raw| serde_json::from_slice(raw).unwrap())
            .collect()
    }

    fn assert_standalone_record_layout(bytes: &[u8]) {
        let records = output_records(bytes);
        assert_eq!(
            records
                .iter()
                .filter(|record| structural_record_type(record) == Some("session_meta"))
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .map(|record| record.get("ordinal").and_then(Value::as_u64).unwrap())
                .collect::<Vec<_>>(),
            (0..records.len() as u64).collect::<Vec<_>>()
        );
        assert_eq!(
            structural_record_type(records.first().unwrap()),
            Some("session_meta")
        );
    }

    #[test]
    fn captured_encoding_reuses_reads_and_matches_serial_output() {
        let directory = tempfile::tempdir().unwrap();
        let metrics = crate::sync_metrics::register_root(directory.path());
        let mut files = BTreeMap::new();
        let base = write(
            &directory,
            &mut files,
            "base",
            &[
                meta_line("base", None),
                message_line("m1", "Read /tmp/project/readme."),
            ],
        );
        let base_len = std::fs::metadata(&base).unwrap().len();
        let leaf = write(
            &directory,
            &mut files,
            "leaf",
            &[
                meta_line("leaf", Some(("base", 2, base_len))),
                message_line("m2", "DB_PASSWORD=hunter2"),
            ],
        );
        let lookup = |id: &str| Ok(files.get(id).cloned());
        let (bytes, stamp) = crate::jsonl_tail_io::read_snapshot(&leaf).unwrap();
        let leaf_len = bytes.len() as u64;
        let bytes = PayloadBytes::from(bytes);
        let (resolved, sources) = crate::codex_history_graph::resolve_captured_codex_root(
            &leaf,
            bytes.clone(),
            stamp,
            &lookup,
        )
        .unwrap();
        let seed = crate::codex_history_graph::seed_codex_root(&resolved, &bytes, None).unwrap();
        assert_eq!(sources[&leaf].0.as_ptr(), bytes.as_ptr());
        let encode = || {
            encode_captured_codex_root(
                &resolved,
                &sources,
                SOURCE_ROOT,
                None,
                None,
                &mut credential_redaction::shared().begin_run(),
                &PathOrigins::default(),
            )
        };
        let mut outputs = Vec::new();
        for (workers, admitted_jobs) in [(1, 1), (4, 4), (4, 2)] {
            let mut run = Vec::new();
            let bounds = crate::bounded_preparation::for_each_prepared(
                (0..8).collect(),
                workers,
                admitted_jobs * (base_len + leaf_len),
                |_| base_len + leaf_len,
                |index| {
                    if index == 3 {
                        return Err("injected source failure".to_string());
                    }
                    let artifact = encode()?;
                    Ok((
                        artifact.root_bytes.into_bytes(),
                        artifact.portable_tail,
                        artifact.prefix_witness,
                        artifact.artifact_sha256,
                        artifact.path_origins,
                    ))
                },
                |index, result| run.push((index, result)),
            );
            assert!(bounds.peak_jobs <= workers.min(admitted_jobs as usize));
            assert!(bounds.peak_source_bytes <= admitted_jobs * (base_len + leaf_len));
            // Results arrive in completion order.
            run.sort_by_key(|(index, _)| *index);
            outputs.push(run);
        }
        assert_eq!(outputs[0], outputs[1]);
        assert_eq!(outputs[0], outputs[2]);
        assert!(outputs[1][3].1.is_err());
        assert!(outputs[1][7].1.is_ok());
        assert_eq!(
            metrics.snapshot().transcript_bytes_read,
            base_len + leaf_len
        );
        let disk = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &PathOrigins::default(),
        )
        .unwrap();
        let captured = outputs[0][0].1.as_ref().unwrap();
        assert_eq!(captured.0.as_bytes(), disk.root_bytes.as_bytes());
        assert_eq!(captured.1, disk.portable_tail);
        assert_eq!(captured.2, disk.prefix_witness);
        assert_eq!(captured.3, disk.artifact_sha256);
        assert_eq!(
            seed.observed_tail.start_record_count,
            disk.portable_tail.start_record_count
        );
        // An ancestor edit after resolution must invalidate the snapshot too.
        let before = std::fs::read(&base).unwrap();
        let mut edited = before.clone();
        let position = edited.iter().position(|byte| *byte == b'R').unwrap();
        edited[position] = b'W';
        std::fs::write(&base, edited).unwrap();
        assert!(encode().unwrap_err().contains("changed after capture"));
    }

    #[test]
    fn captured_source_changes_fail_before_resolution() {
        for change in ["append", "truncate", "rewrite", "replace", "symlink"] {
            let directory = tempfile::tempdir().unwrap();
            let mut files = BTreeMap::new();
            let leaf = write(
                &directory,
                &mut files,
                "leaf",
                &[meta_line("leaf", None), message_line("m1", "hello")],
            );
            let (bytes, stamp) = crate::jsonl_tail_io::read_snapshot(&leaf).unwrap();
            match change {
                "append" => {
                    let mut new = bytes.clone();
                    new.push(b'\n');
                    std::fs::write(&leaf, new).unwrap();
                }
                "truncate" => std::fs::write(&leaf, &bytes[..bytes.len() - 1]).unwrap(),
                "rewrite" => {
                    let mut new = bytes.clone();
                    let index = new.windows(5).position(|part| part == b"hello").unwrap();
                    new[index] = b'j';
                    std::fs::write(&leaf, new).unwrap();
                }
                "replace" => {
                    let replacement = directory.path().join("replacement");
                    std::fs::write(&replacement, &bytes).unwrap();
                    std::fs::rename(replacement, &leaf).unwrap();
                }
                "symlink" => {
                    #[cfg(unix)]
                    {
                        let target = directory.path().join("target");
                        std::fs::rename(&leaf, &target).unwrap();
                        std::os::unix::fs::symlink(target, &leaf).unwrap();
                    }
                    #[cfg(not(unix))]
                    {
                        continue;
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                crate::codex_history_graph::resolve_captured_codex_root(
                    &leaf,
                    bytes.into(),
                    stamp,
                    &|_| Ok(None),
                )
                .is_err(),
                "{change}"
            );
        }
    }

    #[test]
    fn encoded_fixture_preserves_bytes_hashes_and_tail() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let base = write(
            &directory,
            &mut files,
            "fixture-base",
            &[
                meta_line("fixture-base", None),
                message_line("m1", "Read /tmp/project/readme."),
                message_line("m2", "DB_PASSWORD=hunter2"),
            ],
        );
        let base_len = std::fs::metadata(base).unwrap().len();
        let leaf = write(
            &directory,
            &mut files,
            "fixture-leaf",
            &[
                meta_line("fixture-leaf", Some(("fixture-base", 3, base_len))),
                message_line("m3", "Literal /__mallard_root__/example."),
                event_line(
                    "2026-09-04T10:00:00Z",
                    json!({"type": "agent_reasoning", "text": "Check the source."}),
                ),
                event_line(
                    "2026-09-04T10:00:01Z",
                    json!({"type": "agent_message", "message": "Updated /tmp/project/readme."}),
                ),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        assert_eq!(
            &wire.root_bytes[..],
            include_bytes!("fixtures/standalone_codex_wire.jsonl")
        );
        credential_redaction::shared()
            .certify_wire_document(wire.root_bytes.clone().into_bytes())
            .unwrap();
        let payload = wire.root_bytes.clone().into_bytes();
        assert_eq!(payload.as_bytes().as_ptr(), wire.root_bytes.as_ptr());
        assert_eq!(payload.sha256(), mallard_domain::sha256_hex(&payload));
        let actual = json!({
            "content_sha256": payload.sha256(),
            "artifact_sha256": wire.artifact_sha256,
            "portable_tail": wire.portable_tail,
            "prefix_witness": wire.prefix_witness,
            "lineage_digest": wire.lineage_digest,
            "semantic_digest": wire.semantic_digest,
            "message_count": wire.message_count,
        });
        assert_eq!(
            actual,
            json!({
                "artifact_sha256": "9bf92ec55209f4ae8a7bf51e2750126601237003d83978f2f73778094dd84e57",
                "content_sha256": "935f9ac76ce614272b84383874310e0125e2fc4f275f5757a0c8ad75bd8eebb6",
                "lineage_digest": "e5f9fa8338870ba4ee8c353f1000204247ad6903fc9c90e99ac425df3fa771a6",
                "semantic_digest": "dd5b4127c3e51f7ec87357b216e9b340a7ce6d911b6589d340e947b79d371bda",
                "message_count": 3,
                "portable_tail": {
                    "accepted_end_record_count": 3,
                    "accepted_tail_state": "e4fd3bdcfccb6a413faad80e2d860fca3debaf88d716d42aa2696403b2a35bc8",
                    "codec": "mallard-codex-portable-tail-wire-v1",
                    "partition_kind": "fixed_paginated_tail",
                    "start_record_count": 2
                },
                "prefix_witness": {
                    "lineage_digest": "0b9e8f6c2ff396e7977fbdd091408bbfb1c06f5604ffdfdd1d1eee41cbc4709f",
                    "semantic_digest": "587832fca4d8fa0cac08fe8203e90368325f8daf2b64efd29af5336e4a088f83"
                }
            })
        );
    }

    #[test]
    fn encode_rejects_credentials_revealed_by_hook_projection() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let hook = json!({
            "timestamp": "2026-09-04T10:00:00Z",
            "type": "response_item",
            "payload": {
                "type": "message", "id": "hook-1", "role": "user",
                "content": [{"type": "input_text", "text":
                    "<hook_prompt hook_run_id=\"run-1\">DB_PASSWOR&#68;=hunter2</hook_prompt>"}]
            }
        });
        let mut redactor = credential_redaction::shared().begin_run();
        // The encoded spelling is clean; XML decoding creates the finding.
        assert_eq!(
            redactor.redact_record_gated(&hook).unwrap(),
            (hook.clone(), false)
        );
        let leaf = write(
            &directory,
            &mut files,
            "hook-root",
            &[meta_line("hook-root", None), hook.to_string()],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        let error = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut redactor,
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap_err();
        assert!(error.contains("inserted record"), "{error}");
        assert!(error.contains("wire_secret_detected"), "{error}");
        assert!(!error.contains("hunter2"), "{error}");
    }

    #[test]
    fn mcp_projection_retains_redaction_from_source_credential_context() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let leaf = write(
            &directory,
            &mut files,
            "mcp-root",
            &[
                meta_line("mcp-root", None),
                message_line("m1", "Read the result."),
                event_line(
                    "2026-09-04T10:00:00Z",
                    json!({
                        "type": "mcp_tool_call_end", "call_id": "call-1",
                        "host": "db.example", "user": "fixture",
                        "invocation": {"server": "fixture", "tool": "read", "arguments": null},
                        "result": {"Ok": {"password": "hunter2"}},
                        "duration": {"secs": 1, "nanos": 0}
                    }),
                ),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        let staged = directory.path().join("mcp-wire.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let restaged = resolve_codex_root(&staged, &|_| Ok(None)).unwrap();
        let local =
            decode_standalone_codex_root(&restaged, "fresh", SOURCE_ROOT, None, None).unwrap();
        let records = output_records(&local.root_bytes);
        let item = records
            .iter()
            .find(|record| record["payload"]["item"]["type"] == "McpToolCall")
            .unwrap();
        // Projection drops the source host/user context. Its copied result
        // must still carry the placeholder produced before projection.
        assert_eq!(item["payload"]["item"]["result"]["password"], "<password>");
        credential_redaction::shared()
            .certify_wire_document(wire.root_bytes.into_bytes())
            .unwrap();
    }

    #[test]
    fn unfamiliar_records_round_trip_with_paths_redaction_and_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let details = json!({
            "nested": [null, true, 42, {"path": "/tmp/project/result.txt"}],
            "text": "DB_PASSWORD=hunter2",
            "literal": "/__mallard_root__/example"
        });
        let leaf = write(
            &directory,
            &mut files,
            "source",
            &[
                meta_line("source", None),
                message_line("m1", "hello"),
                event_line(
                    "2026-09-22T10:00:00Z",
                    json!({"type": "task_started", "turn_id": "turn-1"}),
                ),
                event_line(
                    "2026-09-22T10:00:01Z",
                    json!({
                        "type": "item_completed", "thread_id": "source", "turn_id": "turn-1",
                        "item": {"type": "FunctionCallOutput", "id": "output-1",
                            "name": "request_user_input", "output": "answer", "extra": details}
                    }),
                ),
                event_line(
                    "2026-09-22T10:00:02Z",
                    json!({"type": "future_event", "extra": details}),
                ),
                event_line(
                    "2026-09-22T10:00:03Z",
                    json!({
                        "type": "item_completed", "thread_id": "source", "turn_id": "turn-1",
                        "extra_envelope": {"enabled": true},
                        "item": {"type": "FunctionCallInput", "id": "input-1", "extra": details}
                    }),
                ),
                event_line(
                    "2026-09-22T10:00:04Z",
                    json!({"type": "task_complete", "turn_id": "turn-1"}),
                ),
            ],
        );
        let source_bytes = std::fs::read(&leaf).unwrap();
        let resolved = resolve_codex_root(&leaf, &|_| Ok(None)).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &PathOrigins::default(),
        )
        .unwrap();
        let wire_text = std::str::from_utf8(&wire.root_bytes).unwrap();
        assert!(!wire_text.contains(SOURCE_ROOT));
        assert!(!wire_text.contains("hunter2"));
        credential_redaction::shared()
            .certify_wire_document(wire.root_bytes.clone().into_bytes())
            .unwrap();

        // Compare every retained record, including opaque nested fields and order.
        let mut expected_wire = output_records(&source_bytes);
        expected_wire[0] = output_records(&wire.root_bytes)[0].clone();
        let mut wire_details = details.clone();
        wire_details["nested"][3]["path"] = json!(format!("{ROOT_TOKEN}/result.txt"));
        wire_details["text"] = json!("DB_PASSWORD=<password>");
        wire_details["literal"] = json!("/__mallard_escape__/r/example");
        for index in [3, 5] {
            expected_wire[index]["payload"]["item"]["extra"] = wire_details.clone();
        }
        expected_wire[4]["payload"]["extra"] = wire_details;
        assert_eq!(output_records(&wire.root_bytes), expected_wire);

        let staged = directory.path().join("wire.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let restaged = resolve_codex_root(&staged, &|_| Ok(None)).unwrap();
        let destination = "/fixture/destination";
        let local =
            decode_standalone_codex_root(&restaged, "restored", destination, None, None).unwrap();
        assert_standalone_record_layout(&local.root_bytes);
        let mut expected_local = output_records(&source_bytes);
        expected_local[0] = output_records(&local.root_bytes)[0].clone();
        let mut local_details = details;
        local_details["nested"][3]["path"] = json!(format!("{destination}/result.txt"));
        local_details["text"] = json!("DB_PASSWORD=<password>");
        for index in [3, 5] {
            expected_local[index]["payload"]["thread_id"] = json!("restored");
            expected_local[index]["payload"]["item"]["extra"] = local_details.clone();
        }
        expected_local[4]["payload"]["extra"] = local_details;
        assert_eq!(output_records(&local.root_bytes), expected_local);

        let local_path = directory.path().join("restored.jsonl");
        std::fs::write(&local_path, &local.root_bytes).unwrap();
        let resolved_local = resolve_codex_root(&local_path, &|_| Ok(None)).unwrap();
        let again = encode_standalone_codex_root(
            &resolved_local,
            destination,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &local.path_origins,
        )
        .unwrap();
        expected_wire[0]["payload"]["id"] = json!("restored");
        for index in [3, 5] {
            expected_wire[index]["payload"]["thread_id"] = json!("restored");
        }
        assert_eq!(output_records(&again.root_bytes), expected_wire);
        assert_eq!(again.portable_tail, wire.portable_tail);
        assert_eq!(std::fs::read(&leaf).unwrap(), source_bytes);
    }

    #[test]
    fn encode_emits_token_wire_and_decode_restores_target_paths() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let p0 = write(
            &directory,
            &mut files,
            "p0",
            &[
                meta_line("p0", None),
                message_line("m1", "created /tmp/project/readme"),
                message_line("m2", "out of root: /tmp/other/readme"),
            ],
        );
        let p0_len = std::fs::metadata(&p0).unwrap().len();
        let leaf = write(
            &directory,
            &mut files,
            "p1",
            &[
                meta_line("p1", Some(("p0", 3, p0_len))),
                message_line("m3", "see file:///tmp/project/read%20me"),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        assert_eq!(resolved.start_record_count, 2);

        // Encode: token wire, wire codec, marker on the head, no source
        // paths on the wire, out-of-root bytes verbatim.
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        assert_eq!(wire.root_provider_thread_id, "p1");
        assert_eq!(wire.message_count, 3);
        assert_eq!(wire.portable_tail.codec, PORTABLE_TAIL_CODEC_WIRE_V1);
        assert_eq!(wire.portable_tail.start_record_count, 2);
        assert_standalone_record_layout(&wire.root_bytes);
        let text = String::from_utf8(wire.root_bytes.to_vec()).unwrap();
        assert!(text.contains("\"history_base\":null"));
        assert!(text.contains(ROOT_TOKEN));
        assert!(text.contains("__mallard_path_map__"));
        assert!(text.contains("mallard.pathmap.v1"));
        assert!(!text.contains("/tmp/project"), "source root leaked to wire");
        assert!(
            text.contains("/tmp/other/readme"),
            "out-of-root bytes must ship verbatim"
        );
        assert!(text.contains("/__mallard_root__/bu;/read%20me"));
        let head_cwd = output_records(&wire.root_bytes)[0]["payload"]["cwd"].clone();
        assert_eq!(head_cwd, json!(ROOT_TOKEN));

        // The wire parses as an ordinary rollout.
        let staged = directory.path().join("staged.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();
        assert_eq!(restaged.chain.len(), 3);

        // Decode onto a different machine: fresh identity, marker stripped,
        // paths under the target root, local codec, out-of-root verbatim.
        let imported = mallard_domain::RootTailLayout {
            codec: PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: mallard_domain::TailPartitionKind::FixedPaginatedTail,
            start_record_count: 2,
        };
        let local = decode_standalone_codex_root(
            &restaged,
            "fresh-root",
            "/home/other/project",
            None,
            Some(&imported),
        )
        .unwrap();
        assert_eq!(local.root_provider_thread_id, "fresh-root");
        assert_eq!(local.portable_tail.codec, PORTABLE_TAIL_CODEC_V1);
        assert_eq!(local.portable_tail.start_record_count, 2);
        assert_eq!(local.portable_tail.accepted_end_record_count, 3);
        assert_eq!(local.last_canonical_message_id.as_deref(), Some("m3"));
        // The artifact witness is local-form: it folds the *decoded* chain's
        // prefix, not the staged wire chain's — the two differ exactly
        // because the excluded prefix contains a remapped path. A later
        // push's local recomputation must reproduce it bit for bit.
        let local_analysis = analyze_codex_transcript(&local.root_bytes);
        let local_chain = local_analysis.lineage.chain().unwrap();
        let expected_witness = mallard_domain::conversation_prefix_witness(local_chain, 2).unwrap();
        assert_eq!(local.prefix_witness, expected_witness);
        let wire_witness = mallard_domain::conversation_prefix_witness(&restaged.chain, 2).unwrap();
        assert_ne!(local.prefix_witness, wire_witness);
        let text = String::from_utf8(local.root_bytes.to_vec()).unwrap();
        assert!(!text.contains("__mallard_path_map__"));
        assert!(!text.contains("__mallard_root__"));
        assert!(!text.contains("\"p1\""));
        assert!(text.contains("created /home/other/project/readme"));
        assert!(text.contains("file:///home/other/project/read%20me"));
        assert!(text.contains("/tmp/other/readme"));
        let records = output_records(&local.root_bytes);
        assert_eq!(records[0]["payload"]["cwd"], json!("/home/other/project"));

        // The wire tail and the local tail share coordinates but not codecs
        // or states: forms are never cross-compared.
        assert_eq!(
            wire.portable_tail.accepted_end_record_count,
            local.portable_tail.accepted_end_record_count
        );
        assert_ne!(wire.portable_tail.codec, local.portable_tail.codec);

        // Determinism: the artifact digest is stable for identical inputs.
        let again = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        assert_eq!(again.artifact_sha256, wire.artifact_sha256);
    }

    #[test]
    fn subdirectory_sessions_materialize_under_target_root_plus_suffix() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let subdir_meta = serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {
                "id": "sub",
                "cwd": "/tmp/project/frontend",
                "history_mode": "paginated",
            }
        }))
        .unwrap();
        let turn_context = serde_json::to_string(&json!({
            "type": "turn_context",
            "payload": {"cwd": "/tmp/project/frontend", "approval_policy": "never"}
        }))
        .unwrap();
        let leaf = write(
            &directory,
            &mut files,
            "sub",
            &[
                subdir_meta,
                turn_context,
                // Root-level path from a subdir-only session: the dynamic
                // base-table design's coverage hole, pinned as a regression.
                message_line("m1", "edited /tmp/project/README.md"),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        let text = String::from_utf8(wire.root_bytes.to_vec()).unwrap();
        assert!(text.contains("/__mallard_root__/b;/frontend"));
        assert!(text.contains("/__mallard_root__/b;/README.md"));

        let staged = directory.path().join("staged.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();
        let local =
            decode_standalone_codex_root(&restaged, "fresh", "/homeB/projact_beta", None, None)
                .unwrap();
        let records = output_records(&local.root_bytes);
        assert_eq!(
            records[0]["payload"]["cwd"],
            json!("/homeB/projact_beta/frontend")
        );
        let turn = records
            .iter()
            .find(|record| structural_record_type(record) == Some("turn_context"))
            .unwrap();
        assert_eq!(
            turn["payload"]["cwd"],
            json!("/homeB/projact_beta/frontend")
        );
        let text = String::from_utf8(local.root_bytes.to_vec()).unwrap();
        assert!(text.contains("/homeB/projact_beta/README.md"));
    }

    #[test]
    fn foreign_cwds_and_literal_markers_hold_their_contracts() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let foreign_turn = serde_json::to_string(&json!({
            "type": "turn_context",
            "payload": {"cwd": "/somewhere/else", "approval_policy": "never"}
        }))
        .unwrap();
        let leaf = write(
            &directory,
            &mut files,
            "p",
            &[
                meta_line("p", None),
                foreign_turn,
                message_line("m1", "one"),
            ],
        );
        {
            let lookup = |thread: &str| Ok(files.get(thread).cloned());
            let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
            let wire = encode_standalone_codex_root(
                &resolved,
                SOURCE_ROOT,
                None,
                None,
                &mut crate::credential_redaction::shared().begin_run(),
                &crate::session_path_origins::PathOrigins::default(),
            )
            .unwrap();
            let text = String::from_utf8(wire.root_bytes.to_vec()).unwrap();
            // The foreign structural cwd never tokenizes.
            assert!(text.contains("/somewhere/else"));
        }

        let command_output = concat!(
            "literal file:///__mallard_root__/readme\n",
            "project /tmp/project/src\n",
            "escape /__mallard_escape__/r"
        );
        let command_result = event_line(
            "2026-09-03T18:00:00Z",
            json!({
                "type": "exec_command_end",
                "call_id": "call-marker",
                "turn_id": "turn-marker",
                "command": ["rg", "__mallard_root__"],
                "cwd": SOURCE_ROOT,
                "parsed_cmd": [{"type": "search"}],
                "source": "agent",
                "status": "completed",
                "stdout": command_output,
                "stderr": "",
                "aggregated_output": command_output,
                "formatted_output": command_output,
                "exit_code": 0,
                "duration": {"secs": 0, "nanos": 1},
            }),
        );
        let leaf = write(
            &directory,
            &mut files,
            "p2",
            &[
                meta_line("p2", None),
                message_line("m1", "mention of /__mallard_root__/x"),
                command_result,
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        let records = output_records(&wire.root_bytes);
        let message = records
            .iter()
            .find(|record| record["payload"]["id"] == json!("m1"))
            .unwrap();
        assert_eq!(
            message["payload"]["content"],
            json!("mention of /__mallard_escape__/r/x")
        );
        let command = records
            .iter()
            .find(|record| record["payload"]["type"] == json!("exec_command_end"))
            .unwrap();
        let command = &command["payload"];
        let expected_wire_output = concat!(
            "literal file:///__mallard_escape__/r/readme\n",
            "project /__mallard_root__/b;/src\n",
            "escape /__mallard_escape__/e/r"
        );
        assert_eq!(command["stdout"], json!(expected_wire_output));
        assert_eq!(command["aggregated_output"], json!(expected_wire_output));

        let staged = directory.path().join("literal-markers.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();
        let local =
            decode_standalone_codex_root(&restaged, "fresh", "/homeB/projact_beta", None, None)
                .unwrap();
        let records = output_records(&local.root_bytes);
        let message = records
            .iter()
            .find(|record| record["payload"]["id"] == json!("m1"))
            .unwrap();
        assert_eq!(
            message["payload"]["content"],
            json!("mention of /__mallard_root__/x")
        );
        let command = records
            .iter()
            .find(|record| record["payload"]["item"]["type"] == json!("CommandExecution"))
            .unwrap();
        let command = &command["payload"]["item"];
        let expected_local_output = concat!(
            "literal file:///__mallard_root__/readme\n",
            "project /homeB/projact_beta/src\n",
            "escape /__mallard_escape__/r"
        );
        assert_eq!(command["stdout"], json!(expected_local_output));
        assert_eq!(command["aggregated_output"], json!(expected_local_output));
    }

    #[test]
    fn decode_preserves_destination_literals_and_blocks_invalid_declarations() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let leaf = write(
            &directory,
            &mut files,
            "p",
            &[
                meta_line("p", None),
                message_line("m1", "created /tmp/project/readme"),
                message_line("m2", "verbatim other machine path /b/project/x"),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        let staged = directory.path().join("staged.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();

        // The literal target path and a rendered reference coexist. Replay
        // uses saved origin spans and must not tokenize the literal path.
        let local =
            decode_standalone_codex_root(&restaged, "fresh", "/b/project", None, None).unwrap();
        let local_path = directory.path().join("local.jsonl");
        std::fs::write(&local_path, &local.root_bytes).unwrap();
        let resolved_local = resolve_codex_root(&local_path, &no_bases).unwrap();
        let again = encode_standalone_codex_root(
            &resolved_local,
            "/b/project",
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &local.path_origins,
        )
        .unwrap();
        assert_eq!(again.portable_tail, wire.portable_tail);
        std::fs::write(&staged, &again.root_bytes).unwrap();
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();
        let returned =
            decode_standalone_codex_root(&restaged, "returned", SOURCE_ROOT, None, None).unwrap();
        let records = output_records(&returned.root_bytes);
        assert_eq!(
            records[1]["payload"]["content"],
            "created /tmp/project/readme"
        );
        assert_eq!(
            records[2]["payload"]["content"],
            "verbatim other machine path /b/project/x"
        );

        // A wire object without the marker, or with an unknown codec, is a
        // typed block.
        let mut records = output_records(&wire.root_bytes);
        records[0]["payload"]
            .as_object_mut()
            .unwrap()
            .remove(PATH_MAP_FIELD);
        let stripped = directory.path().join("stripped.jsonl");
        let mut bytes = Vec::new();
        for record in &records {
            bytes.extend_from_slice(&serde_json::to_vec(record).unwrap());
            bytes.push(b'\n');
        }
        std::fs::write(&stripped, &bytes).unwrap();
        let restaged = resolve_codex_root(&stripped, &no_bases).unwrap();
        let error = decode_standalone_codex_root(&restaged, "fresh", "/homeB/clean", None, None)
            .unwrap_err();
        assert!(error.contains("path-map declaration"), "{error}");

        let mut records = output_records(&wire.root_bytes);
        records[0]["payload"][PATH_MAP_FIELD] = json!({"codec": "mallard.pathmap.v9"});
        let unknown = directory.path().join("unknown.jsonl");
        let mut bytes = Vec::new();
        for record in &records {
            bytes.extend_from_slice(&serde_json::to_vec(record).unwrap());
            bytes.push(b'\n');
        }
        std::fs::write(&unknown, &bytes).unwrap();
        let restaged = resolve_codex_root(&unknown, &no_bases).unwrap();
        let error = decode_standalone_codex_root(&restaged, "fresh", "/homeB/clean", None, None)
            .unwrap_err();
        assert!(error.contains("unsupported path-map codec"), "{error}");
    }

    fn event_line(timestamp: &str, payload: Value) -> String {
        serde_json::to_string(&json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": payload,
        }))
        .unwrap()
    }

    #[test]
    fn projection_completes_legacy_display_events_and_synthesized_uris_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        // Turn-1 opens in the base segment and completes in the leaf, so the
        // association state must survive the physical boundary. The exec
        // event carries a plain cwd with a space. The portable form keeps
        // that source; the local display must use a percent-encoded URI.
        let space_meta = serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {"id": "p0", "cwd": "/tmp/my project", "history_mode": "paginated"}
        }))
        .unwrap();
        let p0 = write(
            &directory,
            &mut files,
            "p0",
            &[
                space_meta,
                event_line(
                    "2026-08-12T10:00:00Z",
                    json!({"type": "task_started", "turn_id": "turn-1"}),
                ),
                message_line("m1", "one"),
                event_line(
                    "2026-08-12T10:01:00Z",
                    json!({"type": "user_message", "message": "hello",
                           "local_images": [], "text_elements": []}),
                ),
            ],
        );
        let p0_len = std::fs::metadata(&p0).unwrap().len();
        let leaf_meta = serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {"id": "p1", "cwd": "/tmp/my project", "history_mode": "paginated",
                        "history_base": {"thread_id": "p0", "end_ordinal_exclusive": 4,
                                          "end_byte_offset": p0_len}}
        }))
        .unwrap();
        let leaf = write(
            &directory,
            &mut files,
            "p1",
            &[
                leaf_meta,
                event_line(
                    "2026-08-12T10:02:00Z",
                    json!({"type": "exec_command_end", "call_id": "call-1",
                           "turn_id": "turn-1",
                           "command": ["ls"], "cwd": "/tmp/my project/sub dir",
                           "parsed_cmd": [], "status": "completed",
                           "exit_code": 0, "duration": {"secs": 0, "nanos": 1},
                           "formatted_output": "", "aggregated_output": "listing"}),
                ),
                message_line("m2", "two"),
                event_line(
                    "2026-08-12T10:03:00Z",
                    json!({"type": "task_complete", "turn_id": "turn-1",
                           "last_agent_message": "hi"}),
                ),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();

        let wire = encode_standalone_codex_root(
            &resolved,
            "/tmp/my project",
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        assert_standalone_record_layout(&wire.root_bytes);
        assert_eq!(wire.message_count, 2);
        let text = String::from_utf8(wire.root_bytes.to_vec()).unwrap();
        // Only the source cwd is portable. Pull builds the URI after
        // path decoding, so URI formatting cannot alter a wire token.
        assert!(text.contains("/__mallard_root__/b;/sub dir"), "{text}");
        assert!(!text.contains("/tmp/my project"), "source root leaked");

        // Pull builds native display records from decoded source values.
        // Their URI values must be valid on the destination.
        let staged = directory.path().join("staged.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();
        let imported = mallard_domain::RootTailLayout {
            codec: PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: wire.portable_tail.partition_kind,
            start_record_count: wire.portable_tail.start_record_count,
        };
        let local =
            decode_standalone_codex_root(&restaged, "fresh", "/homeB/plain", None, Some(&imported))
                .unwrap();
        let text = String::from_utf8(local.root_bytes.to_vec()).unwrap();
        assert!(text.contains("file:///homeB/plain/sub%20dir"), "{text}");
        let records = output_records(&local.root_bytes);
        let items: Vec<&Value> = records
            .iter()
            .filter(|record| record["payload"]["type"] == json!("item_completed"))
            .collect();
        assert_eq!(items.len(), 2);
        for item in &items {
            assert_eq!(item["payload"]["thread_id"], json!("fresh"));
        }

        // Republishing the decoded local output from the pulled machine
        // reproduces the accepted wire tail exactly: identity stamps are
        // chain-ignored, and the path layer is byte-stable across the hop.
        let local_path = directory.path().join("local.jsonl");
        std::fs::write(&local_path, &local.root_bytes).unwrap();
        let re_resolved = resolve_codex_root(&local_path, &no_bases).unwrap();
        let republished = encode_standalone_codex_root(
            &re_resolved,
            "/homeB/plain",
            None,
            Some(&imported),
            &mut crate::credential_redaction::shared().begin_run(),
            &local.path_origins,
        )
        .unwrap();
        assert_eq!(republished.portable_tail, wire.portable_tail);
        assert_eq!(republished.lineage_digest, wire.lineage_digest);
        assert_eq!(republished.semantic_digest, wire.semantic_digest);
    }

    #[test]
    fn folds_leaf_metadata_updates_and_skips_segment_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let p0 = write(
            &directory,
            &mut files,
            "p0",
            &[meta_line("p0", None), message_line("m1", "one")],
        );
        let p0_len = std::fs::metadata(&p0).unwrap().len();
        let leaf_head = serde_json::to_string(&json!({
            "timestamp": "2026-08-08T21:42:48Z",
            "type": "session_meta",
            "payload": {
                "id": "p1",
                "cwd": SOURCE_ROOT,
                "history_mode": "paginated",
                "history_base": {
                    "thread_id": "p0",
                    "end_ordinal_exclusive": 2,
                    "end_byte_offset": p0_len,
                },
                "git": {"branch": "main"},
                "timestamp": "2026-08-08T21:42:48Z",
            }
        }))
        .unwrap();
        let metadata_update = serde_json::to_string(&json!({
            "timestamp": "2026-08-09T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "p1",
                "cwd": SOURCE_ROOT,
                "memory_mode": "enabled",
                "timestamp": "2026-08-09T00:00:00Z",
            }
        }))
        .unwrap();
        let leaf = write(
            &directory,
            &mut files,
            "p1",
            &[
                leaf_head,
                message_line("m2", "two"),
                metadata_update,
                message_line("m3", "three"),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();

        let artifact = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut crate::credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        assert_standalone_record_layout(&artifact.root_bytes);
        let records = output_records(&artifact.root_bytes);
        let metadata = records[0]
            .get("payload")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(metadata.get("git"), Some(&json!({"branch": "main"})));
        assert_eq!(metadata.get("memory_mode"), Some(&json!("enabled")));
        assert_eq!(
            metadata.get("timestamp"),
            Some(&json!("2026-08-08T21:42:48Z"))
        );
        assert_eq!(metadata.get("history_base"), Some(&Value::Null));
        assert_eq!(metadata.get("cwd"), Some(&json!(ROOT_TOKEN)));
        assert!(metadata.contains_key(PATH_MAP_FIELD));
        assert_eq!(
            records
                .iter()
                .filter(|record| structural_record_type(record) == Some("response_item"))
                .map(|record| record["payload"]["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["m1", "m2", "m3"]
        );
    }
    #[test]
    fn hook_entities_and_literal_markers_survive_projection_and_republication() {
        let directory = tempfile::tempdir().unwrap();
        let source_root = "/tmp/a&amp;b";
        let target_root = "/tmp/target";
        let original = concat!(
            "<hook_prompt hook_run_id=\"run-1\">",
            "&#47;__mallard_root__/b; /__mallard_escape__/e ",
            "/tmp/a&amp;b/file and /tmp/target/literal ",
            "é 🦆 \\n &#0;",
            "</hook_prompt>"
        );
        let mut files = BTreeMap::new();
        let leaf = write(
            &directory,
            &mut files,
            "source",
            &[
                json!({"type":"session_meta", "payload":{"id":"source", "cwd":source_root,
                "history_mode":"paginated"}})
                .to_string(),
                json!({"type":"response_item", "timestamp":"2026-09-04T10:00:00Z", "payload":{
                "id":"hook", "type":"message", "role":"user",
                "content":[{"type":"input_text", "text":original}]}})
                .to_string(),
            ],
        );
        let resolved = resolve_codex_root(&leaf, &|_| Ok(None)).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            source_root,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &PathOrigins::default(),
        )
        .unwrap();
        let wire_path = directory.path().join("wire.jsonl");
        std::fs::write(&wire_path, &wire.root_bytes).unwrap();
        let resolved = resolve_codex_root(&wire_path, &|_| Ok(None)).unwrap();
        let local = decode_standalone_codex_root(&resolved, "destination", target_root, None, None)
            .unwrap();
        assert!(local.path_origins.sources["destination"]
            .iter()
            .any(|record| record.derived));
        let local_path = directory.path().join("local.jsonl");
        std::fs::write(&local_path, &local.root_bytes).unwrap();
        let resolved = resolve_codex_root(&local_path, &|_| Ok(None)).unwrap();
        let again = encode_standalone_codex_root(
            &resolved,
            target_root,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &local.path_origins,
        )
        .unwrap();
        assert_eq!(again.portable_tail, wire.portable_tail);
        assert_eq!(
            output_records(&again.root_bytes)[1],
            output_records(&wire.root_bytes)[1]
        );
        std::fs::write(&wire_path, &again.root_bytes).unwrap();
        let resolved = resolve_codex_root(&wire_path, &|_| Ok(None)).unwrap();
        let returned =
            decode_standalone_codex_root(&resolved, "returned", source_root, None, None).unwrap();
        let records = output_records(&returned.root_bytes);
        let message = records
            .iter()
            .find(|record| record["payload"]["id"] == "hook")
            .unwrap();
        assert_eq!(message["payload"]["content"][0]["text"], original);
    }

    /// By digest, the mark of a display record that Pull generated is used
    /// for the record it was saved of and for no other. A display record
    /// that was changed is classified from its content, like a record that
    /// was never covered.
    #[test]
    fn a_changed_display_record_is_classified_from_its_content_by_digest() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let leaf = write(
            &directory,
            &mut files,
            "source",
            &[
                json!({"type":"session_meta", "payload":{"id":"source", "cwd":SOURCE_ROOT,
                "history_mode":"paginated"}})
                .to_string(),
                json!({"type":"response_item", "timestamp":"2026-09-04T10:00:00Z", "payload":{
                "id":"hook", "type":"message", "role":"user", "content":[{"type":"input_text",
                "text":"<hook_prompt hook_run_id=\"run-1\">read /tmp/project/file</hook_prompt>"}]}})
                .to_string(),
            ],
        );
        let resolved = resolve_codex_root(&leaf, &|_| Ok(None)).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &PathOrigins::default(),
        )
        .unwrap();
        let wire_path = directory.path().join("wire.jsonl");
        std::fs::write(&wire_path, &wire.root_bytes).unwrap();
        let resolved = resolve_codex_root(&wire_path, &|_| Ok(None)).unwrap();
        let pulled =
            decode_standalone_codex_root(&resolved, "destination", "/b/project", None, None)
                .unwrap();
        let saved = &pulled.path_origins.sources["destination"];
        assert_eq!(
            saved
                .iter()
                .map(|origin| origin.derived)
                .collect::<Vec<_>>(),
            [false, true, false, true, true]
        );
        let encode = |records: &[Value], lookup| {
            let path = directory.path().join("local.jsonl");
            std::fs::write(
                &path,
                records
                    .iter()
                    .map(|record| format!("{record}\n"))
                    .collect::<String>(),
            )
            .unwrap();
            reconstruct_codex_root(
                &resolve_codex_root(&path, &|_| Ok(None)).unwrap(),
                BuildMode::Encode {
                    root: "/b/project",
                    canonical_root: None,
                },
                None,
                &mut credential_redaction::shared().begin_run(),
                &pulled.path_origins,
                lookup,
                &BTreeMap::new(),
                &|segment| read_bounded(&segment.path).map(Cow::Owned),
            )
        };
        let records = output_records(&pulled.root_bytes);
        let unchanged = encode(&records, OriginLookup::ByPosition).unwrap();
        let by_digest = encode(&records, OriginLookup::ByDigest).unwrap();
        assert_eq!(by_digest.root_bytes, unchanged.root_bytes);
        assert_eq!(by_digest.path_origins, unchanged.path_origins);
        assert_eq!(output_records(&unchanged.root_bytes).len(), 2);

        // Changed in what it says: no longer the record Pull generated, so
        // it is a record of the session and is published.
        let mut said = records.clone();
        said[1]["payload"]["extra"] = json!(true);
        assert!(
            encode(&said, OriginLookup::ByPosition)
                .unwrap_err()
                .saved_record_changed
        );
        let published = encode(&said, OriginLookup::ByDigest).unwrap();
        let output = output_records(&published.root_bytes);
        assert_eq!(output.len(), 3);
        assert_eq!(output[1]["payload"]["extra"], true);
        assert_eq!(
            output[2]["payload"],
            output_records(&unchanged.root_bytes)[1]["payload"]
        );
        assert_eq!(
            published.path_origins.sources["destination"]
                .iter()
                .map(|origin| origin.derived)
                .collect::<Vec<_>>(),
            [false, false, false, true, true]
        );

        // Changed in the thread it names alone: still the record Pull
        // would generate, so it is left out as before, under its new digest.
        let mut named = records.clone();
        named[3]["payload"]["thread_id"] = json!("elsewhere");
        assert!(
            encode(&named, OriginLookup::ByPosition)
                .unwrap_err()
                .saved_record_changed
        );
        let omitted = encode(&named, OriginLookup::ByDigest).unwrap();
        assert_eq!(omitted.root_bytes, unchanged.root_bytes);
        let origins = &omitted.path_origins.sources["destination"];
        assert_eq!(origins[..3], saved[..3]);
        assert_eq!(origins[4], saved[4]);
        assert!(origins[3].derived);
        assert_ne!(origins[3].local_sha256, saved[3].local_sha256);
        origins[3].verify_record(&named[3]).unwrap();
    }

    #[test]
    fn origin_coverage_survives_append_metadata_and_native_fork() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let root = write(
            &directory,
            &mut files,
            "source",
            &[
                meta_line("source", None),
                message_line("m1", "/b/project/literal /tmp/project/mapped"),
            ],
        );
        let resolved = resolve_codex_root(&root, &|_| Ok(None)).unwrap();
        let wire = encode_standalone_codex_root(
            &resolved,
            SOURCE_ROOT,
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &PathOrigins::default(),
        )
        .unwrap();
        let staged = directory.path().join("wire.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let resolved = resolve_codex_root(&staged, &|_| Ok(None)).unwrap();
        let local =
            decode_standalone_codex_root(&resolved, "parent", "/b/project", None, None).unwrap();
        let parent = directory.path().join("parent.jsonl");
        std::fs::write(&parent, &local.root_bytes).unwrap();
        files.insert("parent".into(), parent.clone());
        let length = local.root_bytes.len() as u64;
        let child = write(&directory, &mut files, "child", &[
            json!({"type":"session_meta", "payload":{"id":"child", "cwd":"/b/project",
                "history_mode":"paginated", "history_base":{"thread_id":"parent",
                "end_ordinal_exclusive":2,"end_byte_offset":length}}}).to_string(),
            message_line("m2", "/b/project/new"),
            json!({"type":"session_meta", "payload":{"id":"child", "cwd":"/b/project", "memory_mode":"enabled"}}).to_string(),
        ]);
        let resolved = resolve_codex_root(&child, &|id| Ok(files.get(id).cloned())).unwrap();
        let branch = encode_standalone_codex_root(
            &resolved,
            "/b/project",
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &local.path_origins,
        )
        .unwrap();
        let records = output_records(&branch.root_bytes);
        assert_eq!(
            records[1]["payload"]["content"],
            "/b/project/literal /__mallard_root__/b;/mapped"
        );
        assert_eq!(records[2]["payload"]["content"], "/__mallard_root__/b;/new");
        assert_eq!(records[0]["payload"]["memory_mode"], "enabled");
        assert!(branch.path_origins.sources.contains_key("parent"));
        assert_eq!(branch.path_origins.sources["child"].len(), 3);
        let again = encode_standalone_codex_root(
            &resolved,
            "/b/project",
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &branch.path_origins,
        )
        .unwrap();
        assert_eq!(&again.root_bytes[..], &branch.root_bytes[..]);
        let mut edited = output_records(&local.root_bytes);
        edited[1]["payload"]["content"] = json!("changed");
        std::fs::write(
            &parent,
            edited
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let resolved = resolve_codex_root(&parent, &|_| Ok(None)).unwrap();
        let error = encode_standalone_codex_root(
            &resolved,
            "/b/project",
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &local.path_origins,
        )
        .unwrap_err();
        assert!(error.contains("path origins no longer match"), "{error}");
    }
}
