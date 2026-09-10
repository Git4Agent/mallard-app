//! Claude main-transcript encoding and final wire verification.

use super::codec::{analyze_claude_records, ClaudeRecordContext};
use super::subagents::{record_link, ChildLinks};
use crate::app_settings::conversation_file_limit_bytes;
use crate::claude_history_tail::ClaudeWatermark;
use crate::conversation_codec::CodecLineage;
use crate::credential_redaction::{RedactedWireBytes, RedactionRun};
use crate::payload::PayloadBytes;
use crate::providers::SessionAnalysis;
use crate::session_path_map::SessionPathMapper;
use crate::session_path_origins::PathOrigins;
use serde_json::Value;

pub struct EncodedMainTranscript {
    pub bytes: RedactedWireBytes,
    pub analysis: SessionAnalysis,
    pub watermark: (u64, u64),
    pub path_origins: PathOrigins,
    pub child_links: ChildLinks,
}

/// Preserve source-origin and redaction rules; parse the final wire only once.
pub fn encode_main_transcript(
    bytes: &[u8],
    source_id: &str,
    mapper: &SessionPathMapper,
    origins: &PathOrigins,
    redaction: &mut RedactionRun<'_>,
) -> Result<EncodedMainTranscript, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Claude transcript is not valid UTF-8")?;
    let mut encoded_text = String::with_capacity(text.len());
    let mut path_origins = PathOrigins::default();
    let mut child_links = ChildLinks::new();
    let mut source_ordinal = 0;
    let mut record_offset = 0_u64;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            encoded_text.push('\n');
        }
        let offset = record_offset;
        record_offset += line.len() as u64 + 1;
        if line.trim().is_empty() {
            encoded_text.push_str(line);
            continue;
        }
        let record = serde_json::from_str(line)
            .map_err(|_| "Claude source transcript record is not JSON")?;
        record_link(&mut child_links, &record, offset, source_id)?;
        let encoded = mapper
            .encode_record_with_origins(&record, origins.record(source_id, source_ordinal))
            .map_err(|error| format!("Claude record {}: {error}", index + 1))?;
        source_ordinal += 1;
        path_origins.push(source_id, encoded.origins);
        encoded_text.push_str(
            &serde_json::to_string(&encoded.wire)
                .map_err(|_| "serialize Claude path-encoded record")?,
        );
    }
    origins.verify_source_length(source_id, source_ordinal, false)?;
    if encoded_text.len() as u64 > conversation_file_limit_bytes() {
        return Err("path-encoded Claude transcript exceeds its size limit".into());
    }
    let (redacted, _) = redaction.redact_jsonl_text(&encoded_text)?;
    let bytes = PayloadBytes::from(redacted.into_bytes());
    let (analysis, watermark) = verify_main_wire(&bytes, mapper, redaction)?;
    Ok(EncodedMainTranscript {
        bytes: RedactedWireBytes::from_verified_payload(bytes),
        analysis,
        watermark,
        path_origins,
        child_links,
    })
}

/// All records, including those omitted from the canonical chain, pass the
/// path, credential, and cursor checks before the immutable bytes are certified.
fn verify_main_wire(
    bytes: &[u8],
    mapper: &SessionPathMapper,
    redaction: &mut RedactionRun<'_>,
) -> Result<(SessionAnalysis, (u64, u64)), String> {
    let mut failure = None;
    let mut watermark = ClaudeWatermark::default();
    let mut offset = 0_u64;
    let records = bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter_map(|raw| {
            offset += raw.len() as u64;
            let line = raw.strip_suffix(b"\n").unwrap_or(raw);
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                let _ = watermark.feed(None, offset, ClaudeRecordContext::Main);
                return None;
            }
            let parsed = serde_json::from_slice::<Value>(line);
            if let Ok(record) = &parsed {
                if failure.is_none() {
                    if let Err(error) = mapper
                        .verify_wire_record(record)
                        .and_then(|_| redaction.verify_wire_value(record))
                        .and_then(|_| {
                            watermark.feed(Some(record), offset, ClaudeRecordContext::Main)
                        })
                    {
                        failure = Some(error);
                    }
                }
            }
            Some(parsed)
        });
    let analysis = analyze_claude_records(bytes, records);
    if let Some(error) = failure {
        return Err(error);
    }
    // An early analyzer failure must never certify an unvisited suffix.
    if let CodecLineage::Unknown { reason } = &analysis.lineage {
        return Err(format!("Claude wire failed re-analysis: {reason:?}"));
    }
    Ok((analysis, watermark.finish()))
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
    fn final_checks_include_records_omitted_from_the_chain() {
        let mapper = SessionPathMapper::new("/fixture/project", None, None).unwrap();
        let mut run = shared().begin_run();
        let head = mapper
            .encode_record_with_origins(&main_record(), None)
            .unwrap()
            .wire;
        let head = run.redact_record_gated(&head).unwrap();
        let thinking = |text: &str| {
            json!({"type":"assistant","uuid":"a1","parentUuid":"u1",
            "sessionId":"parent","message":{"role":"assistant",
            "content":[{"type":"thinking","thinking":text}]}})
        };
        for text in ["DB_PASSWORD=hunter2", "/__mallard_root__/invalid"] {
            let bytes = format!("{head}\n{}\n", thinking(text));
            assert!(analyze_claude_transcript(bytes.as_bytes())
                .lineage
                .chain()
                .is_some());
            assert!(verify_main_wire(bytes.as_bytes(), &mapper, &mut run).is_err());
        }
        let malformed = format!("{head}\n{{malformed\n");
        assert!(verify_main_wire(malformed.as_bytes(), &mapper, &mut run).is_err());
    }
}
