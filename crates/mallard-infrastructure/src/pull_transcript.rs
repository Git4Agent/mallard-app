//! Pure transcript preparation for Pull. No provider-home writes or shared-state updates.

use crate::codex_history_graph::resolve_standalone_codex_bytes;
use crate::codex_standalone_builder::decode_codex_tree_member;
use crate::conversation_codec::CodecLineage;
use crate::credential_redaction;
use crate::payload::PayloadBytes;
use crate::providers::SessionAdapter;
use crate::session_path_map::{validate_pathmap_root, SessionPathMapper};
use crate::session_path_origins::PathOrigins;
use crate::verified_transcript::VerifiedTranscript;
use mallard_domain::{
    ConversationPrefixWitness, ConversationWirePathMap, ConversationWireRedaction,
    PortableConversationTail, RootTailLayout, TailPartitionKind,
};
use std::time::{Duration, Instant};

pub struct PullTranscriptRequest<'a> {
    pub cloud_thread_id: &'a str,
    pub cloud_tail: &'a PortableConversationTail,
    pub cloud_wire_redaction: Option<&'a ConversationWireRedaction>,
    pub cloud_wire_path_map: Option<&'a ConversationWirePathMap>,
    pub lineage_digest: Option<&'a str>,
    pub semantic_digest: Option<&'a str>,
    pub destination_provider_thread_id: &'a str,
    pub project_root: &'a str,
    pub canonical_project_root: &'a str,
}

pub struct PreparedPullTranscript {
    /// Creation records retained from the verified wire graph for tree validation.
    pub child_references: crate::owned_subagents::ChildReferences,
    pub cloud_provider_thread_id: String,
    pub transcript: VerifiedTranscript,
    pub path_origins: PathOrigins,
    pub layout: RootTailLayout,
    pub checkpoint: PullTranscriptCheckpoint,
    pub elapsed: Duration,
    pub input_bytes: u64,
    pub analysis_work: crate::sync_metrics::AnalysisWork,
    pub wire_verification_elapsed: Duration,
    pub reconstruction_elapsed: Duration,
    pub output_verification_elapsed: Duration,
}

pub struct PullTranscriptCheckpoint {
    pub prefix_witness: Option<ConversationPrefixWitness>,
    pub artifact_sha256: String,
    pub accepted_local_tail: PortableConversationTail,
    pub lineage_digest: String,
    pub semantic_digest: String,
    pub last_canonical_message_id: Option<String>,
    pub last_synthetic_turn_id: Option<String>,
    pub next_synthetic_turn_ordinal: u64,
}

fn verify_wire_digests(
    lineage: &CodecLineage,
    request: &PullTranscriptRequest<'_>,
) -> Result<(), String> {
    let CodecLineage::Complete {
        lineage_digest,
        semantic_digest,
        ..
    } = lineage
    else {
        return Err("cloud conversation failed its wire analysis".to_string());
    };
    if request.lineage_digest != Some(lineage_digest.as_str())
        || request.semantic_digest != Some(semantic_digest.as_str())
    {
        return Err("cloud conversation changed lineage or semantic content".to_string());
    }
    Ok(())
}

pub fn prepare_pull_transcript(
    adapter: &dyn SessionAdapter,
    cloud_bytes: &[u8],
    request: &PullTranscriptRequest<'_>,
) -> Result<PreparedPullTranscript, String> {
    prepare_pull_transcript_with_identities(
        adapter,
        cloud_bytes,
        request,
        &std::collections::BTreeMap::new(),
    )
}

pub fn prepare_pull_transcript_with_identities(
    adapter: &dyn SessionAdapter,
    cloud_bytes: &[u8],
    request: &PullTranscriptRequest<'_>,
    identities: &std::collections::BTreeMap<String, String>,
) -> Result<PreparedPullTranscript, String> {
    let started = Instant::now();
    let analysis_before = crate::sync_metrics::analysis_work();
    let wire_verification_elapsed;
    let reconstruction_elapsed;
    let output_verification_elapsed;
    let child_references;
    let cloud_tail = request.cloud_tail;
    let cloud_thread_id = request.cloud_thread_id;
    let destination_provider_thread_id = request.destination_provider_thread_id.to_string();
    validate_pathmap_root("project root", request.project_root)?;
    let mut redaction =
        credential_redaction::shared().run_from_stamp(request.cloud_wire_redaction)?;
    let (cloud_provider_thread_id, transcript, path_origins, layout, checkpoint) = if adapter
        .reconstructs_rollout_graph()
    {
        let resolved = resolve_standalone_codex_bytes(cloud_bytes)?;
        if resolved.leaf_meta.is_internal_subagent {
            return Err("cloud conversation is an internal subagent".to_string());
        }
        if request.lineage_digest != Some(resolved.lineage_digest.as_str())
            || request.semantic_digest != Some(resolved.semantic_digest.as_str())
        {
            return Err("cloud conversation changed lineage or semantic content".to_string());
        }
        let layout = RootTailLayout {
            codec: mallard_domain::PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: cloud_tail.partition_kind,
            start_record_count: cloud_tail.start_record_count,
        };
        let refolded = mallard_domain::compute_conversation_tail_state(
            cloud_tail.partition_kind,
            cloud_tail.start_record_count,
            cloud_tail.accepted_end_record_count,
            &resolved.chain,
        )?;
        if refolded != cloud_tail.accepted_tail_state
            || cloud_tail.accepted_end_record_count != resolved.chain.len() as u64
        {
            return Err(
                "reconstructed conversation does not refold to the accepted cloud tail".to_string(),
            );
        }
        wire_verification_elapsed = started.elapsed();
        let reconstruction_started = Instant::now();
        let artifact = decode_codex_tree_member(
            &resolved,
            cloud_bytes,
            &destination_provider_thread_id,
            request.project_root,
            (request.canonical_project_root != request.project_root)
                .then_some(request.canonical_project_root),
            &layout,
            identities,
            &mut redaction,
        )?;
        output_verification_elapsed = artifact.output_verification_elapsed;
        reconstruction_elapsed = reconstruction_started
            .elapsed()
            .saturating_sub(output_verification_elapsed);
        if artifact.portable_tail.partition_kind != cloud_tail.partition_kind
            || artifact.portable_tail.start_record_count != cloud_tail.start_record_count
            || artifact.portable_tail.accepted_end_record_count
                != cloud_tail.accepted_end_record_count
        {
            return Err(
                "decoded conversation disagrees with the accepted cloud coordinates".to_string(),
            );
        }
        child_references = crate::owned_subagents::ChildReferences::Codex(resolved.leaf_creations);
        (
            resolved.leaf_meta.id,
            artifact.verification,
            artifact.path_origins,
            layout,
            PullTranscriptCheckpoint {
                prefix_witness: artifact.prefix_witness,
                artifact_sha256: artifact.artifact_sha256,
                accepted_local_tail: artifact.portable_tail,
                lineage_digest: artifact.lineage_digest,
                semantic_digest: artifact.semantic_digest,
                last_canonical_message_id: artifact.last_canonical_message_id,
                last_synthetic_turn_id: artifact.last_synthetic_turn_id,
                next_synthetic_turn_ordinal: artifact.next_synthetic_turn_ordinal,
            },
        )
    } else {
        match request.cloud_wire_path_map {
            Some(stamp) if stamp.codec == crate::session_path_map::PATH_MAP_CODEC_V1 => {},
            _ => return Err(format!("path_map_codec_mismatch: conversation '{cloud_thread_id}' has no matching path-map stamp")),
        }
        // Claude: wire-form verification ends before any decode. The
        // downloaded wire object must be a redaction fixpoint (the
        // pull-side leak/tamper certificate) and must refold to the
        // accepted cloud pin; only then is it decoded per line — wire
        // self-check, token paths to the destination root, leaf
        // `sessionId` restamped — and the final values are analyzed
        // as the genuine local form, whose fold becomes
        // `accepted_local_tail`. Local and wire folds are distinct.
        redaction
            .certify_wire_document(cloud_bytes.to_vec())
            .map_err(|error| format!("conversation '{}': {error}", cloud_thread_id))?;
        if cloud_tail.codec != mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1
            || cloud_tail.partition_kind != TailPartitionKind::WholeThread
            || cloud_tail.start_record_count != 0
        {
            return Err("accepted cloud tail is not a Claude wire value".to_string());
        }
        let wire_analysis = crate::providers::claude::analyze_claude_transcript(cloud_bytes);
        let CodecLineage::Complete {
            chain: wire_chain, ..
        } = &wire_analysis.lineage
        else {
            return Err("cloud conversation failed its Claude wire analysis".to_string());
        };
        let cloud_provider_thread_id = wire_analysis
            .source_provider_session_id
            .clone()
            .ok_or("cloud transcript lacks an active provider session id")?;
        verify_wire_digests(&wire_analysis.lineage, request)?;
        let refolded = mallard_domain::compute_conversation_tail_state(
            TailPartitionKind::WholeThread,
            0,
            cloud_tail.accepted_end_record_count,
            wire_chain,
        )?;
        if refolded != cloud_tail.accepted_tail_state
            || cloud_tail.accepted_end_record_count != wire_chain.len() as u64
        {
            return Err(
                "cloud conversation does not refold to the accepted cloud tail".to_string(),
            );
        }
        // Decode each record and retain inverse edits. Literal destination
        // paths remain literal on later Push. Restamp only session IDs that
        // name this physical leaf; copied history keeps its provenance IDs.
        wire_verification_elapsed = started.elapsed();
        let reconstruction_started = Instant::now();
        let mapper = SessionPathMapper::new(
            request.project_root,
            (request.canonical_project_root != request.project_root)
                .then_some(request.canonical_project_root),
            std::env::var("HOME").ok().as_deref(),
        )?;
        let wire_text = std::str::from_utf8(cloud_bytes)
            .map_err(|_| format!("conversation '{}' wire is not valid UTF-8", cloud_thread_id))?;
        let mut decoded_text = String::with_capacity(wire_text.len());
        let mut path_origins = PathOrigins::default();
        let mut child_links = crate::providers::claude::subagents::ChildLinks::new();
        let mut offset = 0_u64;
        let mut failure = None;
        let mut records = wire_text
            .split('\n')
            .enumerate()
            .filter_map(|(index, line)| {
                let record_offset = offset;
                offset += line.len() as u64 + 1;
                if index > 0 {
                    decoded_text.push('\n');
                }
                if line.trim().is_empty() {
                    decoded_text.push_str(line);
                    // Match parsed_records: only empty lines (after removing CR)
                    // are skipped; a nonempty whitespace line is invalid JSON.
                    let line = line.strip_suffix('\r').unwrap_or(line);
                    return (!line.is_empty()).then(|| serde_json::from_str(line));
                }
                let result = (|| -> Result<serde_json::Value, String> {
                    let mut wire: serde_json::Value = serde_json::from_str(line)
                        .map_err(|_| "wire transcript record is not valid JSON")?;
                    crate::providers::claude::subagents::record_link(
                        &mut child_links,
                        &wire,
                        record_offset,
                        &cloud_provider_thread_id,
                    )?;
                    // Identity changes precede origin capture. The record hash then
                    // covers the exact value installed in the provider transcript.
                    if let Some(session_id) = wire.get_mut("sessionId") {
                        if session_id.as_str() == Some(cloud_provider_thread_id.as_str()) {
                            *session_id =
                                serde_json::Value::String(destination_provider_thread_id.clone());
                        }
                    }
                    let decoded = mapper.decode_record_with_origins(&wire).map_err(|error| {
                        format!(
                            "conversation '{}' record {}: {error}",
                            cloud_thread_id,
                            index + 1
                        )
                    })?;
                    path_origins.push(&destination_provider_thread_id, decoded.origins);
                    let value = decoded.local;
                    let serialized = serde_json::to_string(&value).map_err(|error| {
                        format!("serialize decoded conversation record: {error}")
                    })?;
                    decoded_text.push_str(&serialized);
                    Ok(value)
                })();
                Some(result.map_err(|error| {
                    failure.get_or_insert(error);
                    serde_json::Error::io(std::io::Error::other("Claude local construction failed"))
                }))
            });
        let staged = crate::providers::claude::codec::analyze_claude_record_values(
            records.by_ref(),
            crate::providers::claude::codec::ClaudeRecordContext::Main,
            cloud_bytes.iter().all(u8::is_ascii_whitespace),
        );
        // An early analysis failure must not hide construction errors later
        // in the input or leave the output accounting incomplete.
        for _ in records {}
        crate::sync_metrics::record_transcript_analysis(decoded_text.len() as u64);
        if let Some(error) = failure {
            return Err(error);
        }
        child_references = crate::owned_subagents::ChildReferences::Claude(child_links);
        let rewritten = decoded_text.into_bytes();
        // Local analysis now runs inside reconstruction. The remaining output
        // phase checks its result, folds the tail, and hashes the final bytes.
        reconstruction_elapsed = reconstruction_started.elapsed();
        let output_verification_started = Instant::now();
        let CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        } = &staged.lineage
        else {
            return Err("decoded conversation failed its Claude re-analysis".to_string());
        };
        if staged.source_provider_session_id.as_deref()
            != Some(destination_provider_thread_id.as_str())
            || chain.len() != wire_chain.len()
        {
            return Err(
                "decoded conversation lost its destination identity or record layout".to_string(),
            );
        }
        let layout = RootTailLayout {
            codec: mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: TailPartitionKind::WholeThread,
            start_record_count: 0,
        };
        // The genuine local fold of the installed bytes: the twin pin the
        // form ledger requires, never a relabel of the wire state.
        let accepted_local_state = mallard_domain::compute_conversation_tail_state(
            TailPartitionKind::WholeThread,
            0,
            cloud_tail.accepted_end_record_count,
            chain,
        )?;
        let accepted_local_tail = PortableConversationTail {
            codec: mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: TailPartitionKind::WholeThread,
            start_record_count: 0,
            accepted_end_record_count: cloud_tail.accepted_end_record_count,
            accepted_tail_state: accepted_local_state,
        };
        let rewritten = PayloadBytes::from(rewritten);
        let verification = VerifiedTranscript::from_analysis(adapter, rewritten.clone(), &staged)?;
        let sha = rewritten.sha256().to_string();
        let leaf = chain
            .last()
            .map(|message| message.message_id.as_str().to_string());
        output_verification_elapsed = output_verification_started.elapsed();
        (
            cloud_provider_thread_id,
            verification,
            path_origins,
            layout,
            PullTranscriptCheckpoint {
                prefix_witness: None,
                artifact_sha256: sha,
                accepted_local_tail,
                lineage_digest: lineage_digest.clone(),
                semantic_digest: semantic_digest.clone(),
                last_canonical_message_id: leaf,
                last_synthetic_turn_id: None,
                next_synthetic_turn_ordinal: 0,
            },
        )
    };
    Ok(PreparedPullTranscript {
        child_references,
        cloud_provider_thread_id,
        transcript,
        path_origins,
        layout,
        checkpoint,
        elapsed: started.elapsed(),
        input_bytes: cloud_bytes.len() as u64,
        analysis_work: crate::sync_metrics::analysis_work().delta_since(analysis_before),
        wire_verification_elapsed,
        reconstruction_elapsed,
        output_verification_elapsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounded_preparation::{for_each_prepared, PREPARATION_BYTE_BUDGET};
    use crate::providers::{claude, codex, AgentProviderRegistry};
    use mallard_domain::{compute_portable_conversation_tail_with_codec, ProviderKey};
    use serde_json::json;

    struct WireFixture {
        provider: ProviderKey,
        bytes: PayloadBytes,
        tail: PortableConversationTail,
        lineage: String,
        semantic: String,
    }

    impl WireFixture {
        fn from_bytes(provider: ProviderKey, bytes: Vec<u8>, start: u64) -> Self {
            let adapter = AgentProviderRegistry::production()
                .require(&provider)
                .unwrap();
            let analysis = adapter.analyze(&bytes);
            let CodecLineage::Complete {
                chain,
                lineage_digest,
                semantic_digest,
            } = analysis.lineage
            else {
                panic!("fixture analysis failed");
            };
            let codec = if provider == codex::provider_key() {
                mallard_domain::PORTABLE_TAIL_CODEC_WIRE_V1
            } else {
                mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1
            };
            let kind = if start == 0 {
                TailPartitionKind::WholeThread
            } else {
                TailPartitionKind::FixedPaginatedTail
            };
            Self {
                provider,
                bytes: bytes.into(),
                tail: compute_portable_conversation_tail_with_codec(codec, kind, start, &chain)
                    .unwrap(),
                lineage: lineage_digest,
                semantic: semantic_digest,
            }
        }

        fn codex() -> Self {
            Self::from_bytes(
                codex::provider_key(),
                include_bytes!("fixtures/standalone_codex_wire.jsonl").to_vec(),
                2,
            )
        }

        fn claude() -> Self {
            let source = json!({
                "type": "user", "uuid": "m1", "parentUuid": null,
                "sessionId": "11111111-1111-4111-8111-111111111111",
                "cwd": "/fixture/source",
                "message": { "role": "user", "content": "Read /fixture/source/readme." }
            });
            let mapper = SessionPathMapper::new("/fixture/source", None, None).unwrap();
            let value = mapper.encode_record_gated(&source).unwrap();
            let value = credential_redaction::shared()
                .begin_run()
                .redact_record_gated(&value)
                .unwrap()
                .0;
            let mut bytes = serde_json::to_vec(&value).unwrap();
            bytes.push(b'\n');
            Self::from_bytes(claude::provider_key(), bytes, 0)
        }

        fn prepare(&self, destination: &str) -> Result<PreparedPullTranscript, String> {
            prepare_pull_transcript(
                AgentProviderRegistry::production().require(&self.provider)?,
                &self.bytes,
                &PullTranscriptRequest {
                    cloud_thread_id: "cloud-fixture",
                    cloud_tail: &self.tail,
                    cloud_wire_redaction: Some(&credential_redaction::shared().stamp()),
                    cloud_wire_path_map: Some(&ConversationWirePathMap {
                        codec: crate::session_path_map::PATH_MAP_CODEC_V1.to_string(),
                    }),
                    lineage_digest: Some(&self.lineage),
                    semantic_digest: Some(&self.semantic),
                    destination_provider_thread_id: destination,
                    project_root: "/fixture/destination",
                    canonical_project_root: "/fixture/destination",
                },
            )
        }
    }

    fn fingerprint(output: PreparedPullTranscript) -> serde_json::Value {
        json!({
            "bytes": output.transcript.bytes().sha256(),
            "artifact": output.checkpoint.artifact_sha256,
            "tail": output.checkpoint.accepted_local_tail,
            "witness": output.checkpoint.prefix_witness,
            "lineage": output.checkpoint.lineage_digest,
            "semantic": output.checkpoint.semantic_digest,
            "last_message": output.checkpoint.last_canonical_message_id,
            "last_turn": output.checkpoint.last_synthetic_turn_id,
            "next_turn": output.checkpoint.next_synthetic_turn_ordinal,
            "layout": output.layout,
        })
    }

    #[test]
    fn serial_and_parallel_preparation_match_for_both_providers() {
        for fixture in [WireFixture::codex(), WireFixture::claude()] {
            let run = |workers| {
                let mut results = Vec::new();
                for_each_prepared(
                    (0..5).collect(),
                    workers,
                    PREPARATION_BYTE_BUDGET,
                    |_| fixture.bytes.len() as u64,
                    |id| fixture.prepare(&format!("22222222-2222-4222-8222-{id:012}")),
                    |index, result| {
                        let output = result.unwrap();
                        // One semantic analysis for the wire and one for the
                        // final local form. Reconstruction reuses both facts.
                        assert_eq!(output.analysis_work.passes, 2);
                        assert_eq!(
                            output.analysis_work.bytes,
                            fixture.bytes.len() as u64 + output.transcript.bytes().len() as u64
                        );
                        assert!(output.transcript.matches_target(
                            &fixture.provider,
                            AgentProviderRegistry::production()
                                .require(&fixture.provider)
                                .unwrap()
                                .codec_key(),
                            Some(&format!("22222222-2222-4222-8222-{index:012}")),
                            "/fixture/destination",
                            "/fixture/destination",
                            &output.checkpoint.lineage_digest,
                            &output.checkpoint.semantic_digest,
                            output.transcript.bytes(),
                        ));
                        results.push((index, fingerprint(output)));
                    },
                );
                // Results arrive in completion order.
                results.sort_by_key(|(index, _)| *index);
                results
            };
            assert_eq!(run(1), run(4));
        }
    }

    #[test]
    fn claude_streamed_decode_matches_emitted_bytes_and_local_analysis() {
        let destination = "22222222-2222-4222-8222-222222222222";
        let base = WireFixture::claude();
        let mut record: serde_json::Value = serde_json::from_slice(&base.bytes).unwrap();
        let source_id = record["sessionId"].as_str().unwrap().to_string();
        record["numbers"] =
            serde_json::from_str("[-0.0,0.99177921181141328,5e-324,1.7976931348623157e308]")
                .unwrap();
        let mut copied = record.clone();
        copied["sessionId"] = json!("33333333-3333-4333-8333-333333333333");
        let mut leaf = record.clone();
        leaf["uuid"] = json!("m2");
        // Cover repeated IDs, copied history, and missing-parent stitching.
        for parent in ["m1", "missing"] {
            leaf["parentUuid"] = json!(parent);
            for ending in ["", "\n", "\r\n", "\n\n\r\n"] {
                let wire = format!("\n\r\n{copied}\n{record}\r\n{leaf}{ending}");
                let fixture = WireFixture::from_bytes(claude::provider_key(), wire.into_bytes(), 0);
                let output = fixture.prepare(destination).unwrap();
                let mapper = SessionPathMapper::new("/fixture/destination", None, None).unwrap();
                let mut expected_origins = PathOrigins::default();
                let expected = std::str::from_utf8(&fixture.bytes)
                    .unwrap()
                    .split('\n')
                    .map(|line| {
                        if line.trim().is_empty() {
                            return line.to_string();
                        }
                        let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
                        if value["sessionId"].as_str() == Some(source_id.as_str()) {
                            value["sessionId"] = json!(destination);
                        }
                        let decoded = mapper.decode_record_with_origins(&value).unwrap();
                        expected_origins.push(destination, decoded.origins);
                        serde_json::to_string(&decoded.local).unwrap()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_eq!(output.transcript.bytes().as_bytes(), expected.as_bytes());
                assert_eq!(output.path_origins, expected_origins);
                let separate = claude::analyze_claude_transcript(expected.as_bytes());
                assert_eq!(
                    separate.source_provider_session_id.as_deref(),
                    Some(destination)
                );
                assert_eq!(separate.record_count, 3);
                let CodecLineage::Complete {
                    chain,
                    lineage_digest,
                    semantic_digest,
                } = separate.lineage
                else {
                    panic!("emitted local transcript failed analysis");
                };
                assert_eq!(chain.len(), 2);
                assert_eq!(output.checkpoint.lineage_digest, lineage_digest);
                assert_eq!(output.checkpoint.semantic_digest, semantic_digest);
                assert_eq!(
                    output.checkpoint.accepted_local_tail,
                    compute_portable_conversation_tail_with_codec(
                        mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_V1,
                        TailPartitionKind::WholeThread,
                        0,
                        &chain,
                    )
                    .unwrap()
                );
                assert_eq!(
                    output.checkpoint.artifact_sha256,
                    mallard_domain::sha256_hex(expected.as_bytes())
                );
                for parsed in crate::conversation_codec::parsed_records(expected.as_bytes()) {
                    let parsed = parsed.unwrap();
                    for (before, after) in record["numbers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(parsed["numbers"].as_array().unwrap())
                    {
                        assert_eq!(
                            before.as_f64().unwrap().to_bits(),
                            after.as_f64().unwrap().to_bits()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn claude_wire_rejects_nonempty_whitespace_and_malformed_suffixes() {
        for suffix in [" \n", "\t\r\n", "{malformed\n"] {
            let mut fixture = WireFixture::claude();
            let mut bytes = fixture.bytes.to_vec();
            bytes.extend_from_slice(suffix.as_bytes());
            fixture.bytes = bytes.into();
            assert!(fixture.prepare("fresh").is_err());
        }
    }

    #[test]
    fn incorrect_cloud_pins_fail_only_their_preparation() {
        for mut fixture in [WireFixture::codex(), WireFixture::claude()] {
            fixture.semantic = "0".repeat(64);
            assert!(fixture
                .prepare("fresh")
                .err()
                .unwrap()
                .contains("changed lineage or semantic content"));
        }
    }

    #[test]
    fn codex_wire_requires_complete_records_and_valid_ordinals() {
        let fixture = WireFixture::codex();
        let mut incomplete = fixture.bytes.to_vec();
        incomplete.extend_from_slice(b"{\"type\":");
        assert!(resolve_standalone_codex_bytes(&incomplete).is_err());
        let changed = String::from_utf8(fixture.bytes.to_vec()).unwrap().replacen(
            "\"ordinal\":1",
            "\"ordinal\":999",
            1,
        );
        assert_ne!(changed.as_bytes(), fixture.bytes.as_ref());
        assert!(resolve_standalone_codex_bytes(changed.as_bytes()).is_err());
    }

    /// Run the same deterministic workload with one or four workers. The
    /// caller can measure process peak RSS with /usr/bin/time -l.
    #[test]
    #[ignore = "manual Pull preparation timing and memory measurement"]
    fn preparation_benchmark() {
        let workers = std::env::var("MALLARD_PULL_BENCH_WORKERS")
            .unwrap_or_else(|_| "4".to_string())
            .parse::<usize>()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.jsonl");
        let mut records = vec![json!({"type": "session_meta", "payload": {
            "id": "benchmark-source", "cwd": "/fixture/source", "timestamp": "2026-09-01T00:00:00Z"
        }})];
        for id in 0..512 {
            records.push(json!({"type": "response_item", "payload": {
                "id": format!("m{id}"), "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "Read /fixture/source/readme. ".repeat(146)}]
            }}));
        }
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend(serde_json::to_vec(&record).unwrap());
            bytes.push(b'\n');
        }
        std::fs::write(&source, &bytes).unwrap();
        drop(bytes);
        let resolved =
            crate::codex_history_graph::resolve_codex_root(&source, &|_| Ok(None)).unwrap();
        let wire = crate::codex_standalone_builder::encode_standalone_codex_root(
            &resolved,
            "/fixture/source",
            None,
            None,
            &mut credential_redaction::shared().begin_run(),
            &crate::session_path_origins::PathOrigins::default(),
        )
        .unwrap();
        let fixture = WireFixture::from_bytes(codex::provider_key(), wire.root_bytes.to_vec(), 0);
        drop(wire);
        drop(resolved);
        let metrics = crate::sync_metrics::register_root(temp.path());
        let started = Instant::now();
        let mut completed = 0;
        let mut work = Duration::ZERO;
        let mut output_bytes = 0;
        let bounds = for_each_prepared(
            (0..8).collect(),
            workers,
            PREPARATION_BYTE_BUDGET,
            |_| fixture.bytes.len() as u64,
            |id| fixture.prepare(&format!("bench-{id}")),
            |_, result| {
                let output = result.unwrap();
                assert_eq!(output.analysis_work.passes, 2);
                work += output.elapsed;
                output_bytes += output.transcript.bytes().len();
                completed += 1;
            },
        );
        assert_eq!(completed, 8);
        assert_eq!(metrics.snapshot().transcript_bytes_read, 0);
        eprintln!("pull-preparation-bench workers={workers} threads={completed} wall_ms={:.3} work_ms={:.3} source_bytes={} output_bytes={output_bytes} peak_jobs={} peak_source_bytes={} temporary_transcript_reads={}",
            started.elapsed().as_secs_f64() * 1000.0, work.as_secs_f64() * 1000.0,
            fixture.bytes.len() * 8, bounds.peak_jobs, bounds.peak_source_bytes, metrics.snapshot().transcript_bytes_read,
        );
    }
}
