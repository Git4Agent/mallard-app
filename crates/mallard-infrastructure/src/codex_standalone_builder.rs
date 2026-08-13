//! Standalone reconstruction of one resolved Codex logical root.
//!
//! Every selected conversation passes through this one builder: dependencies
//! resolved, supported identity edges rewritten, output emitted as one
//! paginated null-base rollout, and every output byte hashed and re-verified
//! against the resolved canonical chain before anything is published or
//! installed. Source `session_meta` records are folded into one authoritative
//! head record, the flattened body is completed with the current Codex
//! `item_completed` projection records paginated history requires, and all
//! emitted records receive fresh contiguous ordinals.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::codex_history_graph::{read_rollout_bounded, ResolvedCodexRoot};
use crate::codex_paginated_projection::{
    complete_paginated_item_projection, verify_projection_fixpoint,
};
use crate::codex_rollout::{session_meta_id, structural_record_type};
use crate::conversation_codec::{analyze_codex_transcript, CodecLineage};
use mallard_domain::{
    compute_portable_conversation_tail, PortableConversationTail, MAX_CONVERSATION_FILE_BYTES,
};

const ARTIFACT_DOMAIN_V1: &[u8] = b"mallard-codex-artifact-v1";

/// How the builder assigns the output root's provider identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StandaloneBuildMode {
    /// Keep the selected local identity: Push objects.
    PreserveIdentity,
    /// Install under a plan-persisted fresh identity and project root: Pull
    /// targets.
    Reidentify {
        root_provider_thread_id: String,
        target_cwd: String,
    },
}

/// One transported file of a standalone artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandaloneSubagentArtifact {
    pub provider_thread_id: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// A plan-persisted identity allocation consumed by `Reidentify` builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandaloneThreadAllocation {
    pub source_provider_thread_id: String,
    pub output_provider_thread_id: String,
}

#[derive(Clone, Debug)]
pub struct StandaloneConversationArtifact {
    pub root_provider_thread_id: String,
    pub root_bytes: Vec<u8>,
    /// Empty in codec v1: Codex declares no structural parent-to-subagent
    /// reference records, so there is no bounded closure to transport.
    pub subagents: Vec<StandaloneSubagentArtifact>,
    pub id_map: Vec<StandaloneThreadAllocation>,
    pub portable_tail: PortableConversationTail,
    pub message_count: u64,
    pub lineage_digest: String,
    pub semantic_digest: String,
    pub artifact_sha256: String,
}

/// Build the deterministic standalone encoding of one resolved root and
/// verify it end to end: the emitted bytes must re-analyze to exactly the
/// resolved canonical chain, digests, and refolded portable tail. `layout`
/// carries the stored binding layout when one exists, so a republish from a
/// pulled machine keeps the imported paginated partition instead of moving
/// the start to the local file's null native base.
pub fn build_standalone_codex_root(
    resolved: &ResolvedCodexRoot,
    mode: &StandaloneBuildMode,
    layout: Option<&mallard_domain::RootTailLayout>,
) -> Result<StandaloneConversationArtifact, String> {
    let (output_root_id, target_cwd) = match mode {
        StandaloneBuildMode::PreserveIdentity => (resolved.leaf_meta.id.clone(), None),
        StandaloneBuildMode::Reidentify {
            root_provider_thread_id,
            target_cwd,
        } => (root_provider_thread_id.clone(), Some(target_cwd.clone())),
    };

    // A standalone paginated rollout has exactly one authoritative metadata
    // record at ordinal zero. Metadata from history-source segments is not
    // conversation history, while later same-thread metadata in the leaf is
    // folded into this effective head record. The flattened body is
    // collected once, completed with the current `item_completed` projection
    // records paginated Codex history requires, and serialized once with
    // fresh contiguous ordinals.
    let authoritative =
        effective_authoritative_meta(resolved, &output_root_id, target_cwd.as_deref())?;
    let original = collect_flattened_non_metadata_records(resolved, target_cwd.as_deref())?;
    let projected = complete_paginated_item_projection(original, &output_root_id)?;
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

    // Full verification: the output must re-analyze to exactly the resolved
    // chain under the output identity, and the tail must refold to the same
    // portable value.
    let analysis = analyze_codex_transcript(&output);
    if analysis.source_thread_id.as_deref() != Some(output_root_id.as_str()) {
        return Err("standalone output lost its root identity".to_string());
    }
    if analysis.is_internal_subagent {
        return Err("standalone output claims to be an internal subagent".to_string());
    }
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
    if chain != resolved.chain {
        return Err("standalone output changed the canonical chain".to_string());
    }
    if lineage_digest != resolved.lineage_digest || semantic_digest != resolved.semantic_digest {
        return Err("standalone output changed the chain digests".to_string());
    }
    let (partition_kind, start_record_count) =
        crate::codex_history_graph::effective_tail_partition(layout, resolved)?;
    let portable_tail =
        compute_portable_conversation_tail(partition_kind, start_record_count, &chain)?;

    let id_map = match mode {
        StandaloneBuildMode::PreserveIdentity => Vec::new(),
        StandaloneBuildMode::Reidentify { .. } => vec![StandaloneThreadAllocation {
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
        lineage_digest,
        semantic_digest,
        artifact_sha256,
    })
}

/// Collect the flattened non-metadata records of every resolved segment in
/// source order, with the reviewed structural CWD rewrite applied and stale
/// top-level ordinals removed. Serialization assigns fresh ordinals later.
fn collect_flattened_non_metadata_records(
    resolved: &ResolvedCodexRoot,
    target_cwd: Option<&str>,
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
        for raw in consumed.split(|byte| *byte == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            let mut value: Value = serde_json::from_slice(raw).map_err(|_| {
                format!(
                    "Codex segment '{}' contains malformed JSON",
                    segment.provider_thread_id
                )
            })?;
            match structural_record_type(&value) {
                Some("session_meta") => continue,
                Some("turn_context") if target_cwd.is_some() => {
                    value = rewrite_turn_context(value, target_cwd.unwrap_or_default())?;
                }
                _ => {}
            }
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

fn effective_authoritative_meta(
    resolved: &ResolvedCodexRoot,
    output_root_id: &str,
    target_cwd: Option<&str>,
) -> Result<Value, String> {
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
    let mut effective =
        effective.ok_or_else(|| "Codex leaf has no session metadata".to_string())?;
    rewrite_meta_record(&mut effective, output_root_id, target_cwd)?;
    Ok(effective)
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

/// Rewrite the effective session metadata to the output root's identity:
/// paginated mode, explicit null base, and no fork or parent linkage.
fn rewrite_meta_record(
    value: &mut Value,
    output_root_id: &str,
    target_cwd: Option<&str>,
) -> Result<(), String> {
    let payload = value
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "session metadata record has no payload".to_string())?;
    rewrite_identity_fields(payload, output_root_id, target_cwd);
    Ok(())
}

fn rewrite_identity_fields(
    payload: &mut Map<String, Value>,
    output_root_id: &str,
    target_cwd: Option<&str>,
) {
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
    if let Some(cwd) = target_cwd {
        for key in ["cwd", "working_directory", "workingDirectory"] {
            if payload.contains_key(key) {
                payload.insert(key.to_string(), Value::String(cwd.to_string()));
            }
        }
    }
}

fn rewrite_turn_context(mut value: Value, target_cwd: &str) -> Result<Value, String> {
    let payload = value
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "turn context record has no payload".to_string())?;
    for key in ["cwd", "working_directory", "workingDirectory"] {
        if payload.contains_key(key) {
            payload.insert(key.to_string(), Value::String(target_cwd.to_string()));
        }
    }
    Ok(value)
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
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn meta_line(id: &str, base: Option<(&str, u64, u64)>) -> String {
        let mut payload = json!({
            "id": id,
            "cwd": "/tmp/project",
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
    fn standalone_output_is_null_base_verified_and_reidentifiable() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        let p0 = write(
            &directory,
            &mut files,
            "p0",
            &[
                meta_line("p0", None),
                message_line("m1", "one"),
                message_line("m2", "two"),
            ],
        );
        let p0_len = std::fs::metadata(&p0).unwrap().len();
        let leaf = write(
            &directory,
            &mut files,
            "p1",
            &[
                meta_line("p1", Some(("p0", 3, p0_len))),
                message_line("m3", "three"),
            ],
        );
        let lookup = |thread: &str| Ok(files.get(thread).cloned());
        let resolved = resolve_codex_root(&leaf, &lookup).unwrap();
        assert_eq!(resolved.start_record_count, 2);

        // PreserveIdentity: same root id, null base, digests preserved.
        let preserved =
            build_standalone_codex_root(&resolved, &StandaloneBuildMode::PreserveIdentity, None)
                .unwrap();
        assert_eq!(preserved.root_provider_thread_id, "p1");
        assert_eq!(preserved.message_count, 3);
        assert_eq!(preserved.lineage_digest, resolved.lineage_digest);
        assert_eq!(preserved.portable_tail.start_record_count, 2);
        assert!(preserved.subagents.is_empty());
        assert_standalone_record_layout(&preserved.root_bytes);
        let text = String::from_utf8(preserved.root_bytes.clone()).unwrap();
        assert!(text.contains("\"history_base\":null"));
        assert!(!text.contains("history_base\":{"));
        assert!(!text.contains("\"p0\""));

        // The output itself resolves as a rootless whole file whose chain
        // matches, and the tail partition survives via the imported layout,
        // never the null native base.
        let standalone_path = directory.path().join("standalone.jsonl");
        std::fs::write(&standalone_path, &preserved.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let reresolved = resolve_codex_root(&standalone_path, &no_bases).unwrap();
        assert_eq!(reresolved.chain, resolved.chain);
        assert_eq!(
            reresolved.partition_kind,
            mallard_domain::TailPartitionKind::WholeThread
        );

        // Reidentify: fresh id and cwd everywhere identity lives; canonical
        // content untouched.
        let reidentified = build_standalone_codex_root(
            &resolved,
            &StandaloneBuildMode::Reidentify {
                root_provider_thread_id: "fresh-root".to_string(),
                target_cwd: "/home/other/project".to_string(),
            },
            None,
        )
        .unwrap();
        assert_eq!(reidentified.root_provider_thread_id, "fresh-root");
        assert_eq!(reidentified.lineage_digest, resolved.lineage_digest);
        assert_eq!(reidentified.semantic_digest, resolved.semantic_digest);
        assert_ne!(reidentified.artifact_sha256, preserved.artifact_sha256);
        assert_eq!(reidentified.id_map.len(), 1);
        let text = String::from_utf8(reidentified.root_bytes).unwrap();
        assert!(!text.contains("\"p1\""));
        assert!(text.contains("/home/other/project"));

        // The artifact digest is stable for identical inputs.
        let again =
            build_standalone_codex_root(&resolved, &StandaloneBuildMode::PreserveIdentity, None)
                .unwrap();
        assert_eq!(again.artifact_sha256, preserved.artifact_sha256);
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
    fn projection_completes_legacy_display_events_across_segments() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = BTreeMap::new();
        // Turn-1 opens in the base segment and completes in the leaf, so the
        // association state must survive the physical boundary.
        let p0 = write(
            &directory,
            &mut files,
            "p0",
            &[
                meta_line("p0", None),
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
        let leaf = write(
            &directory,
            &mut files,
            "p1",
            &[
                meta_line("p1", Some(("p0", 4, p0_len))),
                event_line(
                    "2026-08-12T10:02:00Z",
                    json!({"type": "agent_message", "message": "hi",
                           "phase": "final_answer", "memory_citation": null}),
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

        let preserved =
            build_standalone_codex_root(&resolved, &StandaloneBuildMode::PreserveIdentity, None)
                .unwrap();
        assert_standalone_record_layout(&preserved.root_bytes);
        assert_eq!(preserved.message_count, 2);
        assert_eq!(preserved.lineage_digest, resolved.lineage_digest);
        assert_eq!(preserved.semantic_digest, resolved.semantic_digest);
        let records = output_records(&preserved.root_bytes);
        let payload_types: Vec<_> = records
            .iter()
            .skip(1)
            .map(|record| {
                record["payload"]["type"]
                    .as_str()
                    .unwrap_or(record["type"].as_str().unwrap())
                    .to_string()
            })
            .collect();
        assert_eq!(
            payload_types,
            vec![
                "task_started",
                "message", // response_item m1
                "user_message",
                "item_completed",
                "agent_message",
                "item_completed",
                "message", // response_item m2
                "task_complete",
            ]
        );
        let items: Vec<&Value> = records
            .iter()
            .filter(|record| record["payload"]["type"] == json!("item_completed"))
            .collect();
        assert_eq!(items.len(), 2);
        for item in &items {
            assert_eq!(item["payload"]["thread_id"], json!("p1"));
            assert_eq!(item["payload"]["turn_id"], json!("turn-1"));
        }
        assert_eq!(items[0]["payload"]["item"]["type"], json!("UserMessage"));
        assert_eq!(items[1]["payload"]["item"]["type"], json!("AgentMessage"));

        // A reidentified build has the same turn/item structure with only
        // the output identity changed.
        let reidentified = build_standalone_codex_root(
            &resolved,
            &StandaloneBuildMode::Reidentify {
                root_provider_thread_id: "fresh-root".to_string(),
                target_cwd: "/home/other/project".to_string(),
            },
            None,
        )
        .unwrap();
        assert_eq!(reidentified.lineage_digest, resolved.lineage_digest);
        let reidentified_records = output_records(&reidentified.root_bytes);
        let reidentified_items: Vec<&Value> = reidentified_records
            .iter()
            .filter(|record| record["payload"]["type"] == json!("item_completed"))
            .collect();
        assert_eq!(reidentified_items.len(), 2);
        for (item, reidentified_item) in items.iter().zip(&reidentified_items) {
            assert_eq!(
                reidentified_item["payload"]["thread_id"],
                json!("fresh-root")
            );
            assert_eq!(
                reidentified_item["payload"]["item"],
                item["payload"]["item"]
            );
        }

        // Rebuilding is deterministic, and rebuilding from the already
        // projected output is byte-identical: the pass is a fixpoint.
        let again =
            build_standalone_codex_root(&resolved, &StandaloneBuildMode::PreserveIdentity, None)
                .unwrap();
        assert_eq!(again.artifact_sha256, preserved.artifact_sha256);
        let standalone_path = directory.path().join("standalone.jsonl");
        std::fs::write(&standalone_path, &preserved.root_bytes).unwrap();
        let no_bases = |_: &str| Ok(None);
        let reresolved = resolve_codex_root(&standalone_path, &no_bases).unwrap();
        let rebuilt =
            build_standalone_codex_root(&reresolved, &StandaloneBuildMode::PreserveIdentity, None)
                .unwrap();
        assert_eq!(rebuilt.root_bytes, preserved.root_bytes);
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
                "cwd": "/tmp/project",
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
                "cwd": "/tmp/project",
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

        let artifact =
            build_standalone_codex_root(&resolved, &StandaloneBuildMode::PreserveIdentity, None)
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
