//! Standalone reconstruction of one resolved Codex logical root.
//!
//! Every selected conversation passes through this one builder: dependencies
//! resolved, the in-cwd path layer transformed through the shared
//! `codex_path_map` matcher, supported identity edges rewritten, output
//! emitted as one paginated null-base rollout, and every output record gated
//! and re-verified against the resolved canonical chain before anything is
//! published or installed.
//!
//! `Encode` builds the machine-neutral **wire form** Push publishes: every
//! in-root path occurrence becomes the `/__mallard_root__` token, the
//! authoritative head carries the `__mallard_path_map__` codec marker, and
//! the artifact's tail and digests are wire-form values. `Decode` builds the
//! **local form** Pull installs: tokens expand to the pulling machine's
//! project root, the marker is stripped, and the artifact's tail, digests,
//! and prefix witness are local-form values. The two §4.4 round-trip gates —
//! the span-bounded structural diff at encode and the `encode∘decode = id`
//! wire self-check — run inside the matching build; the identity layer
//! (fresh root id, head fold, `item_completed` thread stamps, ordinals) is
//! proven by the structural checks, never by byte comparison across it.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::codex_history_graph::{read_rollout_bounded, ResolvedCodexRoot};
use crate::codex_paginated_projection::{
    complete_paginated_item_projection, verify_projection_fixpoint,
};
use crate::codex_path_map::{
    path_map_marker, validate_path_map_marker, CodexPathMapper, PATH_MAP_FIELD,
};
use crate::codex_rollout::{session_meta_id, structural_record_type};
use crate::conversation_codec::{analyze_codex_transcript, CodecLineage};
use mallard_domain::{
    compute_portable_conversation_tail_with_codec, conversation_prefix_witness,
    ConversationPrefixWitness, PortableConversationTail, MAX_CONVERSATION_FILE_BYTES,
    PORTABLE_TAIL_CODEC_V1, PORTABLE_TAIL_CODEC_WIRE_V1,
};

const ARTIFACT_DOMAIN_V1: &[u8] = b"mallard-codex-artifact-v1";

/// How the builder transforms the path layer and assigns the output root's
/// provider identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StandaloneBuildMode {
    /// Build the token-form wire object from local bytes: Push objects.
    /// Keeps the selected local identity.
    Encode {
        /// The binding's project root — the only base paths are remapped
        /// under.
        root: String,
        /// The binding's canonical alias spelling when it differs (macOS
        /// `/var` ↔ `/private/var`); matched identically.
        canonical_root: Option<String>,
    },
    /// Materialize the local form from a staged wire object under a
    /// plan-persisted fresh identity: Pull targets.
    Decode {
        root_provider_thread_id: String,
        /// The pulling machine's binding project root.
        target_root: String,
        /// The pulling binding's canonical alias spelling when it differs.
        /// The pull gate re-encodes under the same patterns a future push
        /// from this machine will use, so alias collisions surface at pull.
        canonical_target_root: Option<String>,
    },
}

/// One transported file of a standalone artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandaloneSubagentArtifact {
    pub provider_thread_id: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// A plan-persisted identity allocation consumed by `Decode` builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandaloneThreadAllocation {
    pub source_provider_thread_id: String,
    pub output_provider_thread_id: String,
}

/// The verified output of one build. Every derived value — tail, digests,
/// witness — belongs to the **output form**: wire values from `Encode`,
/// local values from `Decode`. Forms are never cross-compared.
#[derive(Clone, Debug)]
pub struct StandaloneConversationArtifact {
    pub root_provider_thread_id: String,
    pub root_bytes: Vec<u8>,
    /// Empty in codec v1: Codex declares no structural parent-to-subagent
    /// reference records, so there is no bounded closure to transport.
    pub subagents: Vec<StandaloneSubagentArtifact>,
    pub id_map: Vec<StandaloneThreadAllocation>,
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
pub fn build_standalone_codex_root(
    resolved: &ResolvedCodexRoot,
    mode: &StandaloneBuildMode,
    layout: Option<&mallard_domain::RootTailLayout>,
) -> Result<StandaloneConversationArtifact, String> {
    let home = std::env::var("HOME").ok();
    let (output_root_id, mapper, encode_mode) = match mode {
        StandaloneBuildMode::Encode {
            root,
            canonical_root,
        } => (
            resolved.leaf_meta.id.clone(),
            CodexPathMapper::new(root, canonical_root.as_deref(), home.as_deref())?,
            true,
        ),
        StandaloneBuildMode::Decode {
            root_provider_thread_id,
            target_root,
            canonical_target_root,
        } => (
            root_provider_thread_id.clone(),
            CodexPathMapper::new(target_root, canonical_target_root.as_deref(), home.as_deref())?,
            false,
        ),
    };

    // A standalone paginated rollout has exactly one authoritative metadata
    // record at ordinal zero. Metadata from history-source segments is not
    // conversation history, while later same-thread metadata in the leaf is
    // folded into this effective head record. The path layer transforms
    // before the identity stage; each direction's §4.4 gate runs strictly
    // within the path layer.
    let folded_head = effective_authoritative_meta(resolved)?;
    let authoritative = if encode_mode {
        let mut head = mapper
            .encode_record_gated(&folded_head)
            .map_err(|error| format!("authoritative session metadata: {error}"))?;
        rewrite_meta_record(&mut head, &output_root_id)?;
        head_payload_mut(&mut head)?.insert(PATH_MAP_FIELD.to_string(), path_map_marker());
        head
    } else {
        let mut head = folded_head;
        let marker = head_payload_mut(&mut head)?
            .remove(PATH_MAP_FIELD)
            .ok_or_else(|| {
                "conversation wire object lacks its path-map declaration".to_string()
            })?;
        validate_path_map_marker(&marker)?;
        mapper
            .verify_wire_record(&head)
            .map_err(|error| format!("authoritative session metadata: {error}"))?;
        let mut head = mapper.decode_record(&head)?;
        rewrite_meta_record(&mut head, &output_root_id)?;
        head
    };
    let original = collect_flattened_non_metadata_records(resolved, &mapper, encode_mode)?;
    let projected = complete_paginated_item_projection(original, &output_root_id)?;

    // Wire self-check over the complete emitted body, projection-inserted
    // records included: every emitted record must sit in a declared wire
    // form (`encode(decode(w)) == w`). Any projection change minting an
    // undeclared spelling fails here, at push, not at some later pull.
    if encode_mode {
        let mut normalized_head = authoritative.clone();
        head_payload_mut(&mut normalized_head)?.remove(PATH_MAP_FIELD);
        mapper
            .verify_wire_record(&normalized_head)
            .map_err(|error| format!("authoritative session metadata: {error}"))?;
        for (index, record) in projected.iter().enumerate() {
            mapper
                .verify_wire_record(record)
                .map_err(|error| format!("emitted record {}: {error}", index + 1))?;
        }
    }

    let mut output = Vec::new();
    let mut next_ordinal = append_output_record(&mut output, authoritative, 0)?;
    for record in projected {
        next_ordinal = append_output_record(&mut output, record, next_ordinal)?;
    }
    if output.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
        return Err("standalone Codex output exceeds the bounded transcript limit".to_string());
    }
    validate_standalone_record_layout(&output)?;
    validate_projection_fixpoint(&output, &output_root_id)?;

    // Full verification: the output must re-analyze to a canonical chain
    // with exactly the resolved chain's message identities under the output
    // identity, and the output-form tail is refolded from that analysis.
    let analysis = analyze_codex_transcript(&output);
    if analysis.source_thread_id.as_deref() != Some(output_root_id.as_str()) {
        return Err("standalone output lost its root identity".to_string());
    }
    if analysis.is_internal_subagent {
        return Err("standalone output claims to be an internal subagent".to_string());
    }
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
            return Err(format!(
                "standalone output failed re-analysis: {:?}",
                reason
            ));
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
        return Err("standalone output changed the canonical chain identities".to_string());
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

    let id_map = match mode {
        StandaloneBuildMode::Encode { .. } => Vec::new(),
        StandaloneBuildMode::Decode { .. } => vec![StandaloneThreadAllocation {
            source_provider_thread_id: resolved.leaf_meta.id.clone(),
            output_provider_thread_id: output_root_id.clone(),
        }],
    };
    let subagents: Vec<StandaloneSubagentArtifact> = Vec::new();
    let artifact_sha256 = artifact_digest(&output, &output_root_id, &subagents);
    Ok(StandaloneConversationArtifact {
        root_provider_thread_id: output_root_id,
        message_count: chain.len() as u64,
        root_bytes: output,
        subagents,
        id_map,
        portable_tail,
        prefix_witness,
        last_canonical_message_id,
        last_synthetic_turn_id,
        next_synthetic_turn_ordinal,
        lineage_digest,
        semantic_digest,
        artifact_sha256,
    })
}

/// Collect the flattened non-metadata records of every resolved segment in
/// source order with the path layer applied: encode (with its span-bounded
/// gate) toward the wire, or the pull gate followed by decode toward the
/// local form. Stale top-level ordinals are removed; serialization assigns
/// fresh ordinals later.
fn collect_flattened_non_metadata_records(
    resolved: &ResolvedCodexRoot,
    mapper: &CodexPathMapper,
    encode_mode: bool,
) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    for segment in &resolved.segments {
        let bytes = read_rollout_bounded(&segment.path)?;
        if (bytes.len() as u64) < segment.consumed_byte_len {
            return Err(format!(
                "Codex segment '{}' shrank below its resolved cutoff",
                segment.provider_thread_id
            ));
        }
        let consumed = &bytes[..segment.consumed_byte_len as usize];
        let mut record_number = 0_usize;
        for raw in consumed.split(|byte| *byte == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            record_number += 1;
            let mut value: Value = serde_json::from_slice(raw).map_err(|_| {
                format!(
                    "Codex segment '{}' contains malformed JSON",
                    segment.provider_thread_id
                )
            })?;
            if structural_record_type(&value) == Some("session_meta") {
                continue;
            }
            value = if encode_mode {
                mapper.encode_record_gated(&value).map_err(|error| {
                    format!(
                        "Codex segment '{}' record {}: {error}",
                        segment.provider_thread_id, record_number
                    )
                })?
            } else {
                mapper.verify_wire_record(&value).map_err(|error| {
                    format!(
                        "Codex segment '{}' record {}: {error}",
                        segment.provider_thread_id, record_number
                    )
                })?;
                mapper.decode_record(&value)?
            };
            if let Some(record) = value.as_object_mut() {
                record.remove("ordinal");
            }
            records.push(value);
        }
    }
    Ok(records)
}

/// Prove the serialized output is a fixpoint of the projection pass: parsing
/// the body back and projecting it again must change nothing.
fn validate_projection_fixpoint(bytes: &[u8], output_root_id: &str) -> Result<(), String> {
    let mut records = Vec::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_slice(raw)
            .map_err(|_| "standalone Codex output contains malformed JSON".to_string())?;
        if structural_record_type(&value) == Some("session_meta") {
            continue;
        }
        if let Some(record) = value.as_object_mut() {
            record.remove("ordinal");
        }
        records.push(value);
    }
    verify_projection_fixpoint(&records, output_root_id)
}

/// Domain-separated integrity digest over the complete root output and the
/// ordered `(file role, provider ID, byte length, SHA-256)` file list.
fn artifact_digest(
    root_bytes: &[u8],
    root_provider_thread_id: &str,
    subagents: &[StandaloneSubagentArtifact],
) -> String {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, ARTIFACT_DOMAIN_V1);
    let mut files = Vec::with_capacity(1 + subagents.len());
    files.push((
        "transcript",
        root_provider_thread_id.to_string(),
        root_bytes.len() as u64,
        Sha256::digest(root_bytes),
    ));
    for subagent in subagents {
        files.push((
            "subagent",
            subagent.provider_thread_id.clone(),
            subagent.bytes.len() as u64,
            Sha256::digest(&subagent.bytes),
        ));
    }
    hasher.update((files.len() as u64).to_be_bytes());
    for (role, provider_thread_id, byte_len, sha) in files {
        update_length_prefixed(&mut hasher, role.as_bytes());
        update_length_prefixed(&mut hasher, provider_thread_id.as_bytes());
        hasher.update(byte_len.to_be_bytes());
        hasher.update(sha);
    }
    hex_digest(&hasher.finalize())
}

/// Fold the leaf's same-thread metadata records into one effective head,
/// without any path or identity transformation.
fn effective_authoritative_meta(resolved: &ResolvedCodexRoot) -> Result<Value, String> {
    let leaf = resolved
        .segments
        .last()
        .ok_or_else(|| "resolved Codex root has no segments".to_string())?;
    let bytes = read_rollout_bounded(&leaf.path)?;
    if (bytes.len() as u64) < leaf.consumed_byte_len {
        return Err(format!(
            "Codex segment '{}' shrank below its resolved cutoff",
            leaf.provider_thread_id
        ));
    }
    let mut effective: Option<Value> = None;
    for raw in bytes[..leaf.consumed_byte_len as usize].split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_slice(raw)
            .map_err(|_| "Codex leaf contains malformed JSON".to_string())?;
        if structural_record_type(&value) != Some("session_meta") {
            continue;
        }
        let payload = value
            .get("payload")
            .and_then(Value::as_object)
            .ok_or_else(|| "session metadata record has no payload".to_string())?;
        if session_meta_id(payload) != Some(resolved.leaf_meta.id.as_str()) {
            continue;
        }
        match effective.as_mut() {
            Some(current) => fold_session_meta(current, &value)?,
            None => effective = Some(value),
        }
    }
    effective.ok_or_else(|| "Codex leaf has no session metadata".to_string())
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
    for key in [
        "forked_from_id",
        "forkedFromId",
        "parent_thread_id",
        "parentThreadId",
    ] {
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
    mut value: Value,
    ordinal: u64,
) -> Result<u64, String> {
    let record = value
        .as_object_mut()
        .ok_or_else(|| "Codex rollout record is not a JSON object".to_string())?;
    record.insert("ordinal".to_string(), Value::from(ordinal));
    let encoded =
        serde_json::to_vec(&value).map_err(|_| "serialize standalone Codex record".to_string())?;
    output.extend_from_slice(&encoded);
    output.push(b'\n');
    ordinal
        .checked_add(1)
        .ok_or_else(|| "standalone Codex ordinal overflow".to_string())
}

fn validate_standalone_record_layout(bytes: &[u8]) -> Result<(), String> {
    let mut expected_ordinal = 0_u64;
    let mut session_meta_count = 0_u64;
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_slice(raw)
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
        if structural_record_type(&value) == Some("session_meta") {
            session_meta_count = session_meta_count.saturating_add(1);
            if ordinal != 0 {
                return Err("standalone Codex output has non-head session metadata".to_string());
            }
        }
        expected_ordinal = expected_ordinal
            .checked_add(1)
            .ok_or_else(|| "standalone Codex ordinal overflow".to_string())?;
    }
    if session_meta_count != 1 {
        return Err(format!(
            "standalone Codex output has {session_meta_count} session metadata records"
        ));
    }
    Ok(())
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_history_graph::resolve_codex_root;
    use crate::codex_path_map::ROOT_TOKEN;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const SOURCE_ROOT: &str = "/tmp/project";

    fn encode_mode() -> StandaloneBuildMode {
        StandaloneBuildMode::Encode {
            root: SOURCE_ROOT.to_string(),
            canonical_root: None,
        }
    }

    fn decode_mode(root_id: &str, target_root: &str) -> StandaloneBuildMode {
        StandaloneBuildMode::Decode {
            root_provider_thread_id: root_id.to_string(),
            target_root: target_root.to_string(),
            canonical_target_root: None,
        }
    }

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

    fn write(
        directory: &tempfile::TempDir,
        files: &mut BTreeMap<String, PathBuf>,
        thread: &str,
        lines: &[String],
    ) -> PathBuf {
        let path = directory.path().join(format!("{thread}.jsonl"));
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
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
        let wire = build_standalone_codex_root(&resolved, &encode_mode(), None).unwrap();
        assert_eq!(wire.root_provider_thread_id, "p1");
        assert_eq!(wire.message_count, 3);
        assert_eq!(wire.portable_tail.codec, PORTABLE_TAIL_CODEC_WIRE_V1);
        assert_eq!(wire.portable_tail.start_record_count, 2);
        assert_standalone_record_layout(&wire.root_bytes);
        let text = String::from_utf8(wire.root_bytes.clone()).unwrap();
        assert!(text.contains("\"history_base\":null"));
        assert!(text.contains(ROOT_TOKEN));
        assert!(text.contains("__mallard_path_map__"));
        assert!(text.contains("mallard.pathmap.v1"));
        assert!(!text.contains("/tmp/project"), "source root leaked to wire");
        assert!(text.contains("/tmp/other/readme"), "out-of-root bytes must ship verbatim");
        assert!(text.contains("file:///__mallard_root__/read%20me"));
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
        let local = build_standalone_codex_root(
            &restaged,
            &decode_mode("fresh-root", "/home/other/project"),
            Some(&imported),
        )
        .unwrap();
        assert_eq!(local.root_provider_thread_id, "fresh-root");
        assert_eq!(local.portable_tail.codec, PORTABLE_TAIL_CODEC_V1);
        assert_eq!(local.portable_tail.start_record_count, 2);
        assert_eq!(local.portable_tail.accepted_end_record_count, 3);
        assert_eq!(local.last_canonical_message_id.as_deref(), Some("m3"));
        assert_eq!(local.id_map.len(), 1);
        // The artifact witness is local-form: it folds the *decoded* chain's
        // prefix, not the staged wire chain's — the two differ exactly
        // because the excluded prefix contains a remapped path. A later
        // push's local recomputation must reproduce it bit for bit.
        let local_analysis = analyze_codex_transcript(&local.root_bytes);
        let local_chain = local_analysis.lineage.chain().unwrap();
        let expected_witness =
            mallard_domain::conversation_prefix_witness(local_chain, 2).unwrap();
        assert_eq!(local.prefix_witness, expected_witness);
        let wire_witness =
            mallard_domain::conversation_prefix_witness(&restaged.chain, 2).unwrap();
        assert_ne!(local.prefix_witness, wire_witness);
        let text = String::from_utf8(local.root_bytes.clone()).unwrap();
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
        let again = build_standalone_codex_root(&resolved, &encode_mode(), None).unwrap();
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
        let wire = build_standalone_codex_root(&resolved, &encode_mode(), None).unwrap();
        let text = String::from_utf8(wire.root_bytes.clone()).unwrap();
        assert!(text.contains("/__mallard_root__/frontend"));
        assert!(text.contains("/__mallard_root__/README.md"));

        let staged = directory.path().join("staged.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();
        let local = build_standalone_codex_root(
            &restaged,
            &decode_mode("fresh", "/homeB/projact_beta"),
            None,
        )
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
        let text = String::from_utf8(local.root_bytes).unwrap();
        assert!(text.contains("/homeB/projact_beta/README.md"));
    }

    #[test]
    fn foreign_cwds_and_reserved_tokens_hold_their_contracts() {
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
            let wire = build_standalone_codex_root(&resolved, &encode_mode(), None).unwrap();
            let text = String::from_utf8(wire.root_bytes.clone()).unwrap();
            // The foreign structural cwd never tokenizes.
            assert!(text.contains("/somewhere/else"));
        }

        // A reserved token in source content blocks the build with a typed
        // error naming the record.
        let leaf = write(
            &directory,
            &mut files,
            "p2",
            &[
                meta_line("p2", None),
                message_line("m1", "mention of /__mallard_root__/x"),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        let error = build_standalone_codex_root(&resolved, &encode_mode(), None).unwrap_err();
        assert!(error.contains("reserved wire token"), "{error}");
        assert!(error.contains("record 2"), "{error}");
    }

    #[test]
    fn decode_blocks_on_missing_marker_unknown_codec_and_collisions() {
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
        let wire = build_standalone_codex_root(&resolved, &encode_mode(), None).unwrap();
        let staged = directory.path().join("staged.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();

        // Target-root collision: the verbatim out-of-root path equals the
        // target root + suffix, so the pull gate blocks with the record
        // named; a non-colliding root pulls the same content cleanly.
        let error = build_standalone_codex_root(
            &restaged,
            &decode_mode("fresh", "/b/project"),
            None,
        )
        .unwrap_err();
        assert!(error.contains("round-trip gate"), "{error}");
        assert!(error.contains("record 3"), "{error}");
        build_standalone_codex_root(&restaged, &decode_mode("fresh", "/homeB/clean"), None)
            .unwrap();

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
        let error = build_standalone_codex_root(
            &restaged,
            &decode_mode("fresh", "/homeB/clean"),
            None,
        )
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
        let error = build_standalone_codex_root(
            &restaged,
            &decode_mode("fresh", "/homeB/clean"),
            None,
        )
        .unwrap_err();
        assert!(error.contains("update Mallard"), "{error}");
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
        // event carries a legacy plain cwd with a space, so projection must
        // emit the escaped URI wire context (the undeclared-third-form
        // hazard, pinned).
        let space_root = StandaloneBuildMode::Encode {
            root: "/tmp/my project".to_string(),
            canonical_root: None,
        };
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

        let wire = build_standalone_codex_root(&resolved, &space_root, None).unwrap();
        assert_standalone_record_layout(&wire.root_bytes);
        assert_eq!(wire.message_count, 2);
        let text = String::from_utf8(wire.root_bytes.clone()).unwrap();
        // Projection converted the token-form exec cwd through
        // path_to_file_uri into the declared URI wire context.
        assert!(
            text.contains("file:///__mallard_root__/sub%20dir"),
            "{text}"
        );
        assert!(!text.contains("/tmp/my project"), "source root leaked");

        // Pull decode → re-encode reproduces the wire record (the §4.4 gate
        // over projection-inserted records), and the synthesized URI stays
        // valid end to end.
        let staged = directory.path().join("staged.jsonl");
        std::fs::write(&staged, &wire.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let restaged = resolve_codex_root(&staged, &no_bases).unwrap();
        let imported = mallard_domain::RootTailLayout {
            codec: PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: wire.portable_tail.partition_kind,
            start_record_count: wire.portable_tail.start_record_count,
        };
        let local = build_standalone_codex_root(
            &restaged,
            &decode_mode("fresh", "/homeB/plain"),
            Some(&imported),
        )
        .unwrap();
        let text = String::from_utf8(local.root_bytes.clone()).unwrap();
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
        let republished = build_standalone_codex_root(
            &re_resolved,
            &StandaloneBuildMode::Encode {
                root: "/homeB/plain".to_string(),
                canonical_root: None,
            },
            Some(&imported),
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
            "ordinal": 20,
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
            "ordinal": 99,
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

        let artifact = build_standalone_codex_root(&resolved, &encode_mode(), None).unwrap();
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
}
