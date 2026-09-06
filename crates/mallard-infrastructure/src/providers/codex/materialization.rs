//! Codex materialization target planning and index-fragment verification.

use crate::providers::traits::{MaterializationTargetRequest, PlannedProviderTarget};
use chrono::TimeZone as _;
use chrono::Utc;
use mallard_domain::{validate_absolute_clean_path, validate_title, ConversationFileRole};

pub(super) fn materialization_targets(
    request: &MaterializationTargetRequest<'_>,
) -> Result<Vec<PlannedProviderTarget>, String> {
    if !request.sidecars.is_empty() {
        return Err("Codex conversations have no sidecar files".to_string());
    }
    let transcript = match request.existing_transcript_path {
        Some(existing) => existing.to_path_buf(),
        None => {
            let timestamp = Utc
                .timestamp_opt(request.planned_at as i64, 0)
                .single()
                .ok_or_else(|| "conversation plan timestamp is out of range".to_string())?;
            let path = request
                .provider_home
                .join("sessions")
                .join(timestamp.format("%Y").to_string())
                .join(timestamp.format("%m").to_string())
                .join(timestamp.format("%d").to_string())
                .join(format!(
                    "rollout-{}-{}.jsonl",
                    timestamp.format("%Y-%m-%dT%H-%M-%S"),
                    request.provider_session_id
                ));
            validate_absolute_clean_path("reserved Codex transcript", &path.to_string_lossy())?;
            path
        }
    };
    Ok(vec![PlannedProviderTarget {
        target_path: transcript,
        role: ConversationFileRole::Transcript,
    }])
}

pub(super) fn validate_codex_index_fragment(bytes: &[u8], thread_id: &str) -> Result<(), String> {
    let mut row_count = 0_usize;
    let mut destination_count = 0_usize;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        if line.len() > 1024 * 1024 {
            return Err("Codex index row exceeds one MiB".to_string());
        }
        let value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| "Codex index fragment contains malformed JSON".to_string())?;
        let object = value
            .as_object()
            .ok_or_else(|| "Codex index fragment row is not an object".to_string())?;
        let id = object
            .get("id")
            .or_else(|| object.get("session_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "Codex index fragment row lacks a thread id".to_string())?;
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Err("Codex index fragment has an invalid thread id".to_string());
        }
        if id == thread_id {
            // Rows Mallard appends must match the provider schema exactly;
            // pre-existing rows stay Codex's business. In particular, the
            // append-only index can contain multiple rename rows for another
            // thread ID.
            let named = object
                .get("thread_name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| validate_title("Codex thread name", name).is_ok());
            let updated = object
                .get("updated_at")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok());
            if !named || !updated {
                return Err(
                    "Codex index destination row must carry a thread name and an RFC 3339 \
                     update time"
                        .to_string(),
                );
            }
        }
        destination_count += usize::from(id == thread_id);
        row_count += 1;
    }
    if row_count == 0 || destination_count != 1 {
        return Err(
            "Codex index target must contain exactly one destination thread row".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_codex_index_fragment;

    #[test]
    fn accepts_control_characters_in_destination_titles() {
        for title in [
            "Imported\u{0002}\u{0013}",
            "\0\u{001b}\r\n\t\u{007f}\u{0085}\u{009f}",
            " \t\n",
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "id": "destination",
                "thread_name": title,
                "updated_at": "2026-07-31T22:00:03.000000Z",
            }))
            .unwrap();

            validate_codex_index_fragment(&bytes, "destination").unwrap();
            let decoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(decoded["thread_name"].as_str(), Some(title));
        }
    }

    #[test]
    fn destination_titles_remain_bounded_and_thread_ids_reject_controls() {
        for title in [String::new(), "a".repeat(1_025), "é".repeat(513)] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "id": "destination",
                "thread_name": title,
                "updated_at": "2026-07-31T22:00:03.000000Z",
            }))
            .unwrap();
            assert!(validate_codex_index_fragment(&bytes, "destination").is_err());
        }

        let thread_id = "destination\u{0002}";
        let bytes = serde_json::to_vec(&serde_json::json!({
            "id": thread_id,
            "thread_name": "Imported",
            "updated_at": "2026-07-31T22:00:03.000000Z",
        }))
        .unwrap();
        assert!(validate_codex_index_fragment(&bytes, thread_id)
            .unwrap_err()
            .contains("invalid thread id"));
    }

    #[test]
    fn accepts_repeated_pre_existing_ids_in_append_only_index() {
        let bytes = br#"{"id":"existing","thread_name":"Before"}
{"id":"existing","thread_name":"After"}
{"id":"destination","thread_name":"Imported","updated_at":"2026-07-31T22:00:03.000000Z"}
"#;

        validate_codex_index_fragment(bytes, "destination").unwrap();
    }

    #[test]
    fn rejects_more_than_one_destination_row() {
        let bytes = br#"{"id":"destination","thread_name":"Imported","updated_at":"2026-07-31T22:00:03.000000Z"}
{"id":"destination","thread_name":"Imported again","updated_at":"2026-07-31T22:01:03.000000Z"}
"#;

        assert!(validate_codex_index_fragment(bytes, "destination")
            .unwrap_err()
            .contains("exactly one destination"));
    }
}
