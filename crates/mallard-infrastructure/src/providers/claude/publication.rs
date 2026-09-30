//! Claude main-transcript encoding and final wire verification.

use super::codec::{analyze_claude_record_values, ClaudeRecordContext};
use super::subagents::{record_link, ChildLinks};
use crate::app_settings::conversation_file_limit_bytes;
use crate::claude_history_tail::ClaudeWatermark;
use crate::conversation_codec::CodecLineage;
use crate::credential_redaction::{RedactedWireBytes, RedactionRun};
use crate::payload::PayloadBytes;
use crate::providers::SessionAnalysis;
use crate::session_path_map::SessionPathMapper;
use crate::session_path_origins::{EncodeRefusal, OriginLookup, PathOrigins};
use serde_json::Value;

pub struct EncodedMainTranscript {
    pub bytes: RedactedWireBytes,
    pub analysis: SessionAnalysis,
    pub watermark: (u64, u64),
    pub path_origins: PathOrigins,
    pub child_links: ChildLinks,
}

/// Preserve the raw splice rules and analyze the final values without retaining
/// a second transcript tree. Changed lines reuse the splice gate's parsed value.
pub fn encode_main_transcript(
    bytes: &[u8],
    source_id: &str,
    mapper: &SessionPathMapper,
    origins: &PathOrigins,
    lookup: OriginLookup,
    redaction: &mut RedactionRun<'_>,
) -> Result<EncodedMainTranscript, EncodeRefusal> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Claude transcript is not valid UTF-8")?;
    let mut output = String::with_capacity(text.len());
    let mut encoded_length = 0_u64;
    let mut path_origins = PathOrigins::default();
    let mut child_links = ChildLinks::new();
    let mut saved = origins.saved(source_id, lookup);
    let mut source_ordinal = 0;
    let mut watermark = ClaudeWatermark::default();
    let mut failure = None;
    let mut records = text
        .split_inclusive('\n')
        .enumerate()
        .filter_map(|(index, raw)| {
            let line = raw.strip_suffix('\n').unwrap_or(raw);
            if line.trim().is_empty() {
                output.push_str(raw);
                encoded_length += raw.len() as u64;
                // Preserve the verifier's distinction between an empty line and
                // a whitespace-only nonempty JSON line in a nonempty transcript.
                let line = line.strip_suffix('\r').unwrap_or(line);
                if line.is_empty() {
                    let _ = watermark.feed(None, output.len() as u64, ClaudeRecordContext::Main);
                    return None;
                }
                return Some(serde_json::from_str::<Value>(line));
            }
            let result = (|| -> Result<Value, EncodeRefusal> {
                let record = serde_json::from_str(line)
                    .map_err(|_| "Claude source transcript record is not JSON")?;
                record_link(&mut child_links, &record, source_id)?;
                let encoded = mapper
                    .encode_record_with_origins(&record, saved.origin_of(&record)?)
                    .map_err(|refusal| refusal.at(format_args!("Claude record {}", index + 1)))?;
                source_ordinal += 1;
                path_origins.push(source_id, encoded.origins);
                let serialized = serde_json::to_string(&encoded.wire)
                    .map_err(|_| "serialize Claude path-encoded record")?;
                encoded_length += serialized.len() as u64 + u64::from(raw.ends_with('\n'));
                let (serialized, wire, changed) =
                    redaction.redact_serialized_record(encoded.wire, serialized, index + 1)?;
                if changed {
                    mapper.verify_wire_record(&wire)?;
                }
                output.push_str(&serialized);
                if raw.ends_with('\n') {
                    output.push('\n');
                }
                watermark.feed(Some(&wire), output.len() as u64, ClaudeRecordContext::Main)?;
                Ok(wire)
            })();
            Some(result.map_err(|error| {
                failure.get_or_insert(error);
                serde_json::Error::io(std::io::Error::other("Claude wire construction failed"))
            }))
        });
    let analysis = analyze_claude_record_values(
        records.by_ref(),
        ClaudeRecordContext::Main,
        bytes.iter().all(u8::is_ascii_whitespace),
    );
    // Analysis may reject an early record. Still finish construction checks,
    // source accounting, and error capture; never certify a partial suffix.
    for _ in records {}
    crate::sync_metrics::record_transcript_analysis(output.len() as u64);
    if let Some(error) = failure {
        return Err(error);
    }
    if lookup == OriginLookup::ByPosition {
        origins.verify_source_length(source_id, source_ordinal, false)?;
    }
    if encoded_length > conversation_file_limit_bytes() {
        return Err("path-encoded Claude transcript exceeds its size limit".into());
    }
    if let CodecLineage::Unknown { reason } = &analysis.lineage {
        return Err(format!("Claude wire failed re-analysis: {reason:?}").into());
    }
    Ok(EncodedMainTranscript {
        bytes: RedactedWireBytes::from_verified_payload(PayloadBytes::from(output.into_bytes())),
        analysis,
        watermark: watermark.finish(),
        path_origins,
        child_links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_history_tail::{seed_claude_root, seed_claude_root_with_watermark};
    use crate::credential_redaction::shared;
    use crate::providers::claude::analyze_claude_transcript;
    use serde_json::json;
    use std::path::Path;

    fn main_record() -> Value {
        json!({"type":"user","uuid":"u1","parentUuid":null,"sessionId":"parent",
            "cwd":"/fixture/project","message":{"role":"user","content":"read /fixture/project/file; DB_PASSWORD=hunter2"}})
    }

    #[test]
    fn combined_checks_preserve_wire_bytes_analysis_and_pending_watermark() {
        let mapper = SessionPathMapper::new("/fixture/project", None, None).unwrap();
        let records = [
            main_record(),
            json!({"type":"assistant","uuid":"a1",
            "parentUuid":"u1","sessionId":"parent","message":{"role":"assistant",
            "content":[{"type":"thinking","thinking":"pending"}]}}),
        ];
        let source = records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>()
            + "\n";
        for enabled in [false, true] {
            let mut run = shared().begin_run().with_enabled(enabled);
            let result = encode_main_transcript(
                source.as_bytes(),
                "parent",
                &mapper,
                &PathOrigins::default(),
                OriginLookup::ByPosition,
                &mut run,
            )
            .unwrap();
            let expected_encoded = records
                .iter()
                .map(|record| {
                    let encoded = mapper.encode_record_with_origins(record, None).unwrap();
                    format!("{}\n", encoded.wire)
                })
                .collect::<String>()
                + "\n";
            let expected_wire = shared()
                .begin_run()
                .with_enabled(enabled)
                .redact_jsonl_text(&expected_encoded)
                .unwrap()
                .0;
            assert_eq!(result.bytes.as_bytes(), expected_wire.as_bytes());
            let separate = analyze_claude_transcript(result.bytes.as_bytes());
            assert_eq!(result.analysis.lineage, separate.lineage);
            assert_eq!(result.analysis.record_count, separate.record_count);
            let root = Path::new("/fixture/parent.jsonl");
            let old_seed = seed_claude_root(root, result.bytes.as_bytes(), &separate)
                .unwrap()
                .unwrap();
            let new_seed = seed_claude_root_with_watermark(
                root,
                result.bytes.as_bytes(),
                &result.analysis,
                result.watermark,
            )
            .unwrap()
            .unwrap();
            assert_eq!(old_seed.mapping, new_seed.mapping);
            assert_eq!(old_seed.observed_tail, new_seed.observed_tail);
            assert!(result.watermark.0 < result.bytes.as_bytes().len() as u64);
        }
    }

    #[test]
    fn streamed_splice_preserves_layout_final_analysis_and_emitted_byte_count() {
        let mapper = SessionPathMapper::new("/fixture/project", None, None).unwrap();
        let mut record = main_record();
        let secret = "sk-ant-api03-abcdefghijklmnop";
        record["extra"] = json!({
            "visible": {"text": secret},
            "skipped": {"type": "base64", "text": secret},
            "escaped": "quote \" slash \\ newline\n",
            "numbers": [-0.0, 0.9917792118114133, 5e-324]
        });
        for ending in ["", "\n", "\r\n", "\n\n\r\n"] {
            let source = format!("\n\r\n{record}{ending}");
            for enabled in [false, true] {
                // Reference the former serialization/splice composition.
                let encoded = source
                    .split('\n')
                    .map(|line| {
                        if line.trim().is_empty() {
                            line.to_owned()
                        } else {
                            let value = serde_json::from_str(line).unwrap();
                            mapper.encode_record(&value).unwrap().wire.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let expected = shared()
                    .begin_run()
                    .with_enabled(enabled)
                    .redact_jsonl_text(&encoded)
                    .unwrap()
                    .0;
                let before = crate::sync_metrics::analysis_work();
                let result = encode_main_transcript(
                    source.as_bytes(),
                    "parent",
                    &mapper,
                    &PathOrigins::default(),
                    OriginLookup::ByPosition,
                    &mut shared().begin_run().with_enabled(enabled),
                )
                .unwrap();
                let work = crate::sync_metrics::analysis_work().delta_since(before);
                assert_eq!(work.passes, 1);
                assert_eq!(work.bytes, expected.len() as u64);
                assert_eq!(result.bytes.as_bytes(), expected.as_bytes());
                let parsed = analyze_claude_transcript(expected.as_bytes());
                assert_eq!(result.analysis.lineage, parsed.lineage);
                assert_eq!(result.analysis.record_count, parsed.record_count);
                let mut watermark = ClaudeWatermark::default();
                let mut offset = 0;
                for raw in expected.split_inclusive('\n') {
                    offset += raw.len() as u64;
                    let line = raw.trim_end_matches(['\n', '\r']);
                    let value = if line.is_empty() {
                        None
                    } else {
                        Some(serde_json::from_str(line).unwrap())
                    };
                    watermark
                        .feed(value.as_ref(), offset, ClaudeRecordContext::Main)
                        .unwrap();
                }
                assert_eq!(result.watermark, watermark.finish());
            }
        }
    }

    #[test]
    fn early_analysis_failure_still_checks_suffix_and_rejects_empty_input() {
        let mapper = SessionPathMapper::new("/fixture/project", None, None).unwrap();
        let source = format!("{}\n \n{{malformed\n", main_record());
        let error = encode_main_transcript(
            source.as_bytes(),
            "parent",
            &mapper,
            &PathOrigins::default(),
            OriginLookup::ByPosition,
            &mut shared().begin_run(),
        )
        .err()
        .unwrap();
        assert_eq!(
            error,
            EncodeRefusal::from("Claude source transcript record is not JSON")
        );
        for source in ["", " \n\r\n", "null\n", "{\n"] {
            assert!(encode_main_transcript(
                source.as_bytes(),
                "parent",
                &mapper,
                &PathOrigins::default(),
                OriginLookup::ByPosition,
                &mut shared().begin_run(),
            )
            .is_err());
        }
    }

    /// A session as Pull installs it under `/fixture/destination`, with
    /// the origins Pull saves: `u1` refers to the project, `a1` and `u2`
    /// name the destination's directory in text that was plain where it
    /// was written.
    fn pulled_session() -> (SessionPathMapper, Vec<Value>, Vec<Value>, PathOrigins) {
        let mapper = SessionPathMapper::new("/fixture/destination", None, None).unwrap();
        let row = |kind: &str, uuid: &str, parent: Option<&str>, content: Value| {
            json!({"type":kind,"uuid":uuid,"parentUuid":parent,"sessionId":"parent",
                "cwd":"/__mallard_root__/b;","message":{"role":kind,"content":content}})
        };
        let thinking = json!({"type":"thinking","thinking":"hidden","signature":"s"});
        let wire = vec![
            row("user", "u1", None, json!("read /__mallard_root__/b;/file")),
            row("assistant", "t1", Some("u1"), json!([thinking])),
            row(
                "assistant",
                "a1",
                Some("t1"),
                json!([thinking, {"type":"text","text":"see /fixture/destination/notes"}]),
            ),
            row(
                "user",
                "u2",
                Some("a1"),
                json!("notes are in /fixture/destination/notes"),
            ),
        ];
        let mut origins = PathOrigins::default();
        let local = wire
            .iter()
            .map(|record| {
                let mut decoded = mapper.decode_record_with_origins(record).unwrap();
                decoded.origins.rewritten =
                    crate::providers::claude::codec::origins_without_thinking(
                        &decoded.local,
                        &decoded.origins.fields,
                    )
                    .unwrap();
                origins.push("parent", decoded.origins);
                decoded.local
            })
            .collect();
        (mapper, wire, local, origins)
    }

    fn encode(
        mapper: &SessionPathMapper,
        records: &[Value],
        origins: &PathOrigins,
        lookup: OriginLookup,
    ) -> Result<(Vec<Value>, PathOrigins), EncodeRefusal> {
        let source = records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>();
        let encoded = encode_main_transcript(
            source.as_bytes(),
            "parent",
            mapper,
            origins,
            lookup,
            &mut shared().begin_run(),
        )?;
        let wire = crate::conversation_codec::parsed_records(encoded.bytes.as_bytes())
            .map(Result::unwrap)
            .collect();
        Ok((wire, encoded.path_origins))
    }

    #[test]
    fn a_rewritten_transcript_is_refused_by_position_and_encoded_by_digest() {
        let (mapper, wire, local, saved) = pulled_session();
        let unchanged = encode(&mapper, &local, &saved, OriginLookup::ByPosition).unwrap();
        assert_eq!(unchanged, (wire.clone(), saved.clone()));
        assert_eq!(
            encode(&mapper, &local, &saved, OriginLookup::ByDigest).unwrap(),
            unchanged
        );

        // The desktop import: the row of thinking is removed, the row of
        // thinking and text keeps its text. A new message follows.
        let mut imported: Vec<Value> = local
            .iter()
            .filter_map(|record| crate::providers::claude::project_claude_record(record.clone()))
            .collect();
        assert_eq!(imported.len(), 3);
        imported.push(
            json!({"type":"user","uuid":"u3","parentUuid":"u2","sessionId":"parent",
            "cwd":"/fixture/destination","message":{"role":"user",
            "content":"and /fixture/destination/more"}}),
        );
        let refusal = encode(&mapper, &imported, &saved, OriginLookup::ByPosition).unwrap_err();
        assert!(refusal.saved_record_changed);
        assert_eq!(
            refusal.message,
            "Claude record 2: retained transcript record changed; path origins no longer match"
        );

        let (encoded, origins) =
            encode(&mapper, &imported, &saved, OriginLookup::ByDigest).unwrap();
        let (saved, origins) = (&saved.sources["parent"], &origins.sources["parent"]);
        // Records that kept their content have the origins that were saved
        // of them, wherever they are now: the plain text stays plain.
        assert_eq!(encoded[0], wire[0]);
        assert_eq!(encoded[2], wire[3]);
        assert_eq!((&origins[0], &origins[2]), (&saved[0], &saved[3]));
        // The record that the import rewrote has the origins that were
        // saved of it without its thinking: its plain text stays plain. A
        // new scan would store a reference.
        assert_eq!(
            Some(&encoded[1]),
            crate::providers::claude::project_claude_record(wire[2].clone()).as_ref()
        );
        assert_eq!(Some(&origins[1]), saved[2].rewritten.as_deref());
        assert_ne!(
            origins[1],
            mapper.encode_record(&imported[1]).unwrap().origins
        );
        // The new record is scanned.
        assert_eq!(
            encoded[3]["message"]["content"],
            "and /__mallard_root__/b;/more"
        );
        // What is saved of this encoding is read by position next time.
        let origins = PathOrigins {
            sources: [("parent".to_string(), origins.clone())].into(),
        };
        assert_eq!(
            encode(&mapper, &imported, &origins, OriginLookup::ByPosition).unwrap(),
            (encoded, origins)
        );
    }

    #[test]
    fn a_missing_record_is_told_apart_from_a_transcript_that_cannot_be_read() {
        let (mapper, _, local, saved) = pulled_session();
        let missing = encode(&mapper, &local[..3], &saved, OriginLookup::ByPosition).unwrap_err();
        assert!(missing.saved_record_changed);
        assert_eq!(
            missing.message,
            "retained transcript was truncated; path origins no longer match"
        );
        encode(&mapper, &local[..3], &saved, OriginLookup::ByDigest).unwrap();

        for lookup in [OriginLookup::ByPosition, OriginLookup::ByDigest] {
            let source = format!("{}\n{{malformed\n", local[0]);
            let refusal = encode_main_transcript(
                source.as_bytes(),
                "parent",
                &mapper,
                &saved,
                lookup,
                &mut shared().begin_run(),
            )
            .err()
            .unwrap();
            assert!(!refusal.saved_record_changed, "{lookup:?}");
        }
    }

    /// What an ordinary Push costs: one pass, and one hash of each record,
    /// of a retained one to check it and of a new one to save it.
    #[test]
    fn an_appended_transcript_is_encoded_in_one_pass_with_one_hash_a_record() {
        let (mapper, _, mut local, saved) = pulled_session();
        local.push(
            json!({"type":"user","uuid":"u3","parentUuid":"u2","sessionId":"parent",
            "cwd":"/fixture/destination","message":{"role":"user","content":"more"}}),
        );
        let (passes, hashes) = (
            crate::sync_metrics::analysis_work(),
            crate::session_path_origins::record_hash_attempts(),
        );
        encode(&mapper, &local, &saved, OriginLookup::ByPosition).unwrap();
        assert_eq!(
            crate::sync_metrics::analysis_work()
                .delta_since(passes)
                .passes,
            1
        );
        assert_eq!(
            crate::session_path_origins::record_hash_attempts() - hashes,
            local.len() as u64
        );
    }

    #[test]
    fn final_checks_include_records_omitted_from_the_chain() {
        let mapper = SessionPathMapper::new("/fixture/project", None, None).unwrap();
        let mut run = shared().begin_run();
        let head = main_record();
        let thinking = |text: &str| {
            json!({"type":"assistant","uuid":"a1","parentUuid":"u1",
            "sessionId":"parent","message":{"role":"assistant",
            "content":[{"type":"thinking","thinking":text}]}})
        };
        for text in ["DB_PASSWORD=hunter2", "/__mallard_root__/invalid"] {
            let source = format!("{head}\n{}\n", thinking(text));
            let result = encode_main_transcript(
                source.as_bytes(),
                "parent",
                &mapper,
                &PathOrigins::default(),
                OriginLookup::ByPosition,
                &mut run,
            )
            .unwrap();
            for record in crate::conversation_codec::parsed_records(result.bytes.as_bytes()) {
                let record = record.unwrap();
                mapper.verify_wire_record(&record).unwrap();
                run.verify_wire_value(&record).unwrap();
            }
        }
        let malformed = format!("{head}\n{{malformed\n");
        assert!(encode_main_transcript(
            malformed.as_bytes(),
            "parent",
            &mapper,
            &PathOrigins::default(),
            OriginLookup::ByPosition,
            &mut run,
        )
        .is_err());
    }
}
