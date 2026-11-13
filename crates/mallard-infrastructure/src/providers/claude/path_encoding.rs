//! Path encoding for Claude's structured attachments and their rendered text.

use crate::session_path_map::{EncodedRecord, SessionPathMapper};
use crate::session_path_origins::{EncodeRefusal, RecordPathOrigins};
use serde_json::Value;

const DIRECTORY_PREFIX: &str = " - Primary working directory: ";

/// The directory line is generated from structured environment fields. Match
/// that complete line only; punctuation in arbitrary prose or sibling paths
/// must not become a new boundary in the resource-neutral path mapper.
fn directory_line(record: &Value) -> Option<String> {
    if record["type"] != "attachment" || record["attachment"]["type"] != "environment" {
        return None;
    }
    let attachment = &record["attachment"];
    let directory = attachment["snapshot"]["workingDirectory"].as_str()?;
    let mut line = format!("{DIRECTORY_PREFIX}{directory}");
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

fn encode_attachment_environment(record: &Value, wire: &mut Value) -> bool {
    let (Some(source_line), Some(wire_line)) = (directory_line(record), directory_line(wire))
    else {
        return false;
    };
    let (Some(source_blocks), Some(wire_blocks)) = (
        record["rendered"].as_array(),
        wire.get_mut("rendered").and_then(Value::as_array_mut),
    ) else {
        return false;
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
    changed
}

/// API request blobs repeat rendered environment reminders without the
/// attachment's snapshot. Only their complete, provider-defined reminder
/// blocks give a bare root a structural boundary; arbitrary prose does not.
fn encode_request_text(mapper: &SessionPathMapper, source: &str) -> Result<String, String> {
    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    const OPEN: &str = "<system-reminder>\n";
    const CLOSE: &str = "\n</system-reminder>";
    while let Some((before, after)) = rest.split_once(OPEN) {
        output.push_str(before);
        output.push_str(OPEN);
        let Some((body, after)) = after.split_once(CLOSE) else {
            output.push_str(after);
            return Ok(output);
        };
        let (header, _) = body.split_once('\n').unwrap_or((body, ""));
        let environment = matches!(header, "# Environment" | "# Environment update");
        let mut lines = Vec::new();
        for line in body.split('\n') {
            let Some(directory) = line.strip_prefix(DIRECTORY_PREFIX).filter(|_| environment)
            else {
                lines.push(line.to_string());
                continue;
            };
            let (directory, previous) = if header == "# Environment update" {
                match directory
                    .strip_suffix(')')
                    .and_then(|text| text.rsplit_once(" (was "))
                {
                    Some((directory, previous)) => (directory, Some(previous)),
                    None => (directory, None),
                }
            } else {
                (directory, None)
            };
            let encode = |path: &str| -> Result<String, String> {
                let mapped = mapper.encode_record(&Value::String(path.to_string()))?;
                Ok(mapped.wire.as_str().expect("mapped string").to_string())
            };
            let mut mapped = format!("{DIRECTORY_PREFIX}{}", encode(directory)?);
            if let Some(previous) = previous {
                mapped.push_str(&format!(" (was {})", encode(previous)?));
            }
            lines.push(mapped);
        }
        output.push_str(&lines.join("\n"));
        output.push_str(CLOSE);
        rest = after;
    }
    output.push_str(rest);
    Ok(output)
}

fn encode_request_content(
    mapper: &SessionPathMapper,
    source: &Value,
    wire: &mut Value,
) -> Result<bool, String> {
    if let (Some(source), Some(text)) = (source.as_str(), wire.as_str()) {
        // Start with the normal encoding for text outside environment lines.
        let mapped = encode_request_text(mapper, source)?;
        if mapped != source {
            // Encoding an already encoded value would escape the wire tokens.
            // Merge only the changed lines into the normal encoding instead.
            let combined = source
                .split('\n')
                .zip(mapped.split('\n'))
                .zip(text.split('\n'))
                .map(|((source, mapped), wire)| if source != mapped { mapped } else { wire })
                .collect::<Vec<_>>()
                .join("\n");
            let changed = combined != text;
            *wire = Value::String(combined);
            return Ok(changed);
        }
        return Ok(false);
    }
    let (Some(source), Some(wire)) = (source.as_array(), wire.as_array_mut()) else {
        return Ok(false);
    };
    let mut changed = false;
    for (source, wire) in source.iter().zip(wire) {
        let field = match source["type"].as_str() {
            Some("text") => "text",
            Some("tool_result") => "content",
            _ => continue,
        };
        if let Some(wire) = wire.get_mut(field) {
            changed |= encode_request_content(mapper, &source[field], wire)?;
        }
    }
    Ok(changed)
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
    let mut changed = encode_attachment_environment(record, &mut encoded.wire);
    if record["type"] == "api-request-blob" && record["message"]["role"] == "user" {
        if let Some(content) = encoded.wire.pointer_mut("/message/content") {
            changed |= encode_request_content(mapper, &record["message"]["content"], content)?;
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

    #[test]
    fn api_request_environment_copies_relocate_and_preserve_origins() {
        let a = SessionPathMapper::new("/fixture/source", None, None).unwrap();
        let b = SessionPathMapper::new("/fixture/destination", None, None).unwrap();
        let initial = "<system-reminder>\n# Environment\nYou have been invoked in the following environment:\n - Primary working directory: /fixture/source\n - Platform: darwin\n</system-reminder>";
        let update = "tool output\n\n<system-reminder>\n# Environment update\n - Primary working directory: /fixture/source/part5 (was /fixture/source)\n</system-reminder>\n\nOther /fixture/source/file and literal /__mallard_root__/b;";
        for content in [
            json!(initial),
            json!([{"type":"text","text":initial}, {"type":"text","text":"user prompt"}]),
            json!([{"type":"tool_result","tool_use_id":"call","content":update}]),
            json!([{"type":"tool_result","tool_use_id":"call","content":[{"type":"text","text":update}]}]),
        ] {
            let original = json!({"type":"api-request-blob","sessionId":"session","hash":"request-message-id",
                "message":{"role":"user","content":content}});
            let encoded = encode_record(&a, &original, None).unwrap();
            assert!(!encoded.wire.to_string().contains("/fixture/source"));
            assert_eq!(a.decode_record(&encoded.wire).unwrap(), original);
            let restored = b.decode_record_with_origins(&encoded.wire).unwrap();
            assert!(!restored.local.to_string().contains("/fixture/source"));
            assert!(restored.local.to_string().contains("/fixture/destination"));
            assert_eq!(restored.local["hash"], original["hash"]);
            assert_eq!(
                encode_record(&b, &restored.local, Some(&restored.origins))
                    .unwrap()
                    .wire,
                encoded.wire
            );
            let mut edited = restored.local;
            edited["message"]["content"] = json!("changed");
            assert!(
                encode_record(&b, &edited, Some(&restored.origins))
                    .err()
                    .unwrap()
                    .saved_record_changed
            );
        }
    }

    #[test]
    fn api_request_environment_mapping_does_not_widen_prose_boundaries() {
        let mapper = SessionPathMapper::new("/fixture/source", None, None).unwrap();
        for (kind, role, content) in [
            ("user", "user", "<system-reminder>\n# Environment\n - Primary working directory: /fixture/source\n</system-reminder>"),
            ("api-request-blob", "assistant", "<system-reminder>\n# Environment\n - Primary working directory: /fixture/source\n</system-reminder>"),
            ("api-request-blob", "user", " - Primary working directory: /fixture/source\nLiteral /fixture/source)"),
            ("api-request-blob", "user", "<system-reminder>\n# Environment\n - Primary working directory: /fixture/source\n"),
            ("api-request-blob", "user", "<system-reminder>\n# Other\n - Primary working directory: /fixture/source\n</system-reminder>"),
            ("api-request-blob", "user", "<system-reminder>\n# Environment\n - Primary working directory: /fixture/source-copy\n</system-reminder>"),
        ] {
            let record = json!({"type":kind,"message":{"role":role,"content":[{"type":"text","text":content}]}});
            assert_eq!(encode_record(&mapper, &record, None).unwrap().wire, record);
        }
    }
}
