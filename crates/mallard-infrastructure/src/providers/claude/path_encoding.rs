//! Path encoding for Claude's structured attachments and their rendered text.

use crate::session_path_map::{EncodedRecord, SessionPathMapper};
use crate::session_path_origins::{EncodeRefusal, RecordPathOrigins};
use serde_json::Value;

/// The directory line is generated from structured environment fields. Match
/// that complete line only; punctuation in arbitrary prose or sibling paths
/// must not become a new boundary in the resource-neutral path mapper.
fn directory_line(record: &Value) -> Option<String> {
    if record["type"] != "attachment" || record["attachment"]["type"] != "environment" {
        return None;
    }
    let attachment = &record["attachment"];
    let directory = attachment["snapshot"]["workingDirectory"].as_str()?;
    let mut line = format!(" - Primary working directory: {directory}");
    if let Some(changes) = attachment["changes"].as_array() {
        if let Some(change) = changes
            .iter()
            .find(|change| change["field"] == "workingDirectory")
        {
            line.push_str(&format!(" (was {})", change["from"].as_str()?));
        }
    }
    Some(line)
}

pub(super) fn encode_record(
    mapper: &SessionPathMapper,
    record: &Value,
    previous: Option<&RecordPathOrigins>,
) -> Result<EncodedRecord, EncodeRefusal> {
    let mut encoded = mapper.encode_record_with_origins(record, previous)?;
    // Retained origins are authoritative, including literal destination text.
    if previous.is_some() {
        return Ok(encoded);
    }
    let (Some(source_line), Some(wire_line)) =
        (directory_line(record), directory_line(&encoded.wire))
    else {
        return Ok(encoded);
    };
    let (Some(source_blocks), Some(wire_blocks)) = (
        record["rendered"].as_array(),
        encoded
            .wire
            .get_mut("rendered")
            .and_then(Value::as_array_mut),
    ) else {
        return Ok(encoded);
    };
    let mut changed = false;
    for (source, wire) in source_blocks.iter().zip(wire_blocks) {
        let (Some(source), Some(wire)) = (source["content"].as_str(), wire.get_mut("content"))
        else {
            continue;
        };
        let Some(text) = wire.as_str() else { continue };
        let mapped = source
            .split('\n')
            .zip(text.split('\n'))
            .map(|(source, wire)| {
                if source == source_line {
                    wire_line.as_str()
                } else {
                    wire
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if mapped != text {
            *wire = Value::String(mapped);
            changed = true;
        }
    }
    if changed {
        let decoded = mapper.decode_record_with_origins(&encoded.wire)?;
        if decoded.local != *record {
            return Err("Claude environment rendering failed exact source restoration".into());
        }
        encoded.origins = decoded.origins;
        encoded.changed = true;
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_environment_reminders_relocate_and_republish_without_losing_text() {
        let a = SessionPathMapper::new("/fixture/source", None, None).unwrap();
        let b = SessionPathMapper::new("/fixture/destination", None, None).unwrap();
        for update in [false, true] {
            let directory = if update {
                "/fixture/source/part5"
            } else {
                "/fixture/source"
            };
            let mut record = json!({"type":"attachment", "cwd":directory,
                "attachment":{"type":"environment","snapshot":{"workingDirectory":directory}},
                "renderedRole":"user"});
            if update {
                record["attachment"]["changes"] =
                    json!([{"field":"workingDirectory","from":"/fixture/source"}]);
            }
            let line = if update {
                " - Primary working directory: /fixture/source/part5 (was /fixture/source)"
            } else {
                " - Primary working directory: /fixture/source"
            };
            record["rendered"] = json!([{"content":format!("<system-reminder>\n# Environment\n{line}\n</system-reminder>")}]);
            let original = record.clone();
            let encoded = encode_record(&a, &record, None).unwrap();
            assert!(!encoded.wire.to_string().contains("/fixture/source"));
            assert_eq!(a.decode_record(&encoded.wire).unwrap(), original);
            assert_eq!(record, original);
            let restored = b.decode_record_with_origins(&encoded.wire).unwrap();
            assert!(restored.local["rendered"][0]["content"]
                .as_str()
                .unwrap()
                .contains(" - Primary working directory: /fixture/destination"));
            let republished = encode_record(&b, &restored.local, Some(&restored.origins)).unwrap();
            assert_eq!(republished.wire, encoded.wire);
            let mut edited = restored.local;
            edited["rendered"][0]["content"] = json!("changed");
            assert!(
                encode_record(&b, &edited, Some(&restored.origins))
                    .err()
                    .unwrap()
                    .saved_record_changed
            );
        }
    }

    #[test]
    fn unrelated_renderings_and_outside_directories_keep_their_literal_text() {
        let mapper = SessionPathMapper::new("/fixture/source", None, None).unwrap();
        for kind in ["environment", "other"] {
            let record = json!({"type":"attachment",
                "attachment":{"type":kind,"snapshot":{"workingDirectory":"/fixture/source-copy"}},
                "rendered":[{"content":" - Primary working directory: /fixture/source-copy\nLiteral /fixture/source)\n - Primary working directory: /fixture/source"}]});
            assert_eq!(
                encode_record(&mapper, &record, None).unwrap().wire,
                mapper.encode_record(&record).unwrap().wire
            );
        }
    }
}
