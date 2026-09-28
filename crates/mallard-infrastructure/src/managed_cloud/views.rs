//! Session views for the website (architecture section 5.4). A view is the
//! desktop's normalized timeline of one conversation, built from the wire
//! bytes that are being pushed, in a private temporary provider home with
//! an empty project root: no checkpoint rows, no child tail facts. The
//! service validates its shape and the website renders it as-is.
use crate::codex_paginated_projection::complete_paginated_item_projection;
use crate::codex_rollout::{
    authoritative_session_meta_with_record, structural_record_type, MAX_CODEX_RECORDS,
};
use crate::conversation_codec::parsed_records;
use crate::providers::traits::{
    SessionCount, SessionCountQuality, SessionDetailWarning, SessionEventCounts,
    SessionTimelineEvent, SessionTimelineEventKind, SessionTimelineQuery, SessionToolCategory,
};
use crate::providers::{AgentProviderRegistry, SessionTimelineContext};
use mallard_domain::{
    sha256_hex, BundleManifest, ConversationFileRole, NoticeSeverity, ProviderKey,
};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

/// The view's schema, which names its key: `_views/<schema>-<transcript sha256>.json`.
pub const VIEW_SCHEMA: u32 = 1;
/// The service's view bounds (`LIMITS.viewBytes`, `LIMITS.viewEvents`).
pub const VIEW_BYTES: usize = 8 * 1024 * 1024;
pub const VIEW_EVENTS: usize = 20_000;
const PAGE: usize = 50;

#[derive(Serialize)]
struct SessionView<'a> {
    schema_version: u32,
    resource_id: &'a str,
    provider: &'a str,
    provider_session_id: String,
    title: String,
    summary: Option<String>,
    started_at: Option<u64>,
    ended_at: Option<u64>,
    branch: Option<String>,
    recorded_sha: Option<String>,
    tokens: SessionCount,
    event_counts: SessionEventCounts,
    events: &'a [SessionTimelineEvent],
    truncated: bool,
    warnings: Vec<ViewWarning>,
}

#[derive(Serialize, Clone)]
struct ViewWarning {
    code: String,
    message: String,
    severity: NoticeSeverity,
}

/// Everything but the events, which the size loop slices.
struct ViewFacts<'a> {
    resource_id: &'a str,
    provider: &'a str,
    provider_session_id: String,
    title: String,
    summary: Option<String>,
    started_at: Option<u64>,
    ended_at: Option<u64>,
    branch: Option<String>,
    recorded_sha: Option<String>,
    tokens: SessionCount,
    event_counts: SessionEventCounts,
    warnings: Vec<ViewWarning>,
}

impl<'a> ViewFacts<'a> {
    fn view<'b>(&'b self, events: &'b [SessionTimelineEvent], truncated: bool) -> SessionView<'b> {
        SessionView {
            schema_version: VIEW_SCHEMA,
            resource_id: self.resource_id,
            provider: self.provider,
            provider_session_id: self.provider_session_id.clone(),
            title: self.title.clone(),
            summary: self.summary.clone(),
            started_at: self.started_at,
            ended_at: self.ended_at,
            branch: self.branch.clone(),
            recorded_sha: self.recorded_sha.clone(),
            tokens: self.tokens.clone(),
            event_counts: self.event_counts.clone(),
            events,
            truncated,
            warnings: self.warnings.clone(),
        }
    }
}

/// One built view, ready for the inventory and the object PUT.
#[derive(Clone, Debug)]
pub struct BuiltView {
    pub resource_id: String,
    pub source_sha256: String,
    pub sha256: String,
    pub bytes: bytes::Bytes,
    pub events: usize,
    pub truncated: bool,
}

/// Build one conversation's view from its wire transcript bytes.
pub fn build_view(
    registry: &AgentProviderRegistry,
    provider: &ProviderKey,
    resource_id: &str,
    provider_session_id: &str,
    display_title: Option<&str>,
    transcript: &[u8],
) -> Result<BuiltView, String> {
    let adapter = registry.require(provider)?;
    let temp = tempfile::tempdir().map_err(|error| format!("view workspace: {error}"))?;
    let home = temp.path().join("home");
    let root = temp.path().join("project");
    let sessions = home.join("sessions");
    std::fs::create_dir_all(&sessions).map_err(|error| format!("view workspace: {error}"))?;
    std::fs::create_dir_all(&root).map_err(|error| format!("view workspace: {error}"))?;
    let path = sessions.join(format!("{}.jsonl", safe_stem(provider_session_id)));
    let display_transcript = if provider.as_str() == "codex" {
        Cow::Owned(codex_display_transcript(transcript)?)
    } else {
        Cow::Borrowed(transcript)
    };
    std::fs::write(&path, display_transcript)
        .map_err(|error| format!("view workspace: {error}"))?;
    let context = SessionTimelineContext {
        resource_id,
        provider_session_id,
        project_root: &root,
        project_root_aliases: &[],
        provider_home: &home,
        child: None,
    };
    let index = adapter.build_timeline_index(&path, &context)?;
    let summary = adapter.summarize_timeline(&path, &index, &context)?;
    let mut events = Vec::new();
    let mut start_index = None;
    let mut truncated = index.truncated;
    loop {
        let query = SessionTimelineQuery {
            start_index,
            limit: PAGE,
            included_event_kinds: all_kinds(),
            included_tool_categories: all_categories(),
            turn_id: None,
        };
        let page = adapter.read_timeline_page(&path, &index, &query, &context)?;
        if page.events.is_empty() {
            break;
        }
        events.extend(page.events);
        if events.len() >= VIEW_EVENTS {
            events.truncate(VIEW_EVENTS);
            truncated = truncated || page.next_index.is_some();
            break;
        }
        match page.next_index {
            Some(next) if Some(next) != start_index => start_index = Some(next),
            _ => break,
        }
    }
    let mut counts = index.counts.clone();
    counts.checkpoints = SessionCount::unsupported(
        "Checkpoints are not carried in Mallard Cloud views: the pushing desktop builds the view without the project's Git history.",
    );
    let tokens = match summary.total_tokens {
        Some(value) => SessionCount {
            value: Some(value),
            quality: if summary.metrics_complete {
                SessionCountQuality::Exact
            } else {
                SessionCountQuality::LowerBound
            },
            explanation: (!summary.metrics_complete)
                .then(|| "The provider transcript may contain additional token usage.".to_string()),
        },
        None => SessionCount::unsupported(
            "This provider transcript does not expose a reliable token total.",
        ),
    };
    let title = display_title
        .filter(|title| !title.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            summary
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
        })
        .unwrap_or_else(|| "Untitled session".to_string());
    let warnings = index
        .warnings
        .iter()
        .map(|warning: &SessionDetailWarning| ViewWarning {
            code: warning.code.clone(),
            message: warning.message.clone(),
            severity: warning.severity,
        })
        .collect::<Vec<_>>();
    let facts = ViewFacts {
        resource_id,
        provider: provider.as_str(),
        provider_session_id: summary.session_id.clone(),
        title,
        summary: summary.summary.clone(),
        started_at: summary.started_at,
        ended_at: summary.ended_at,
        branch: summary.branch.clone(),
        recorded_sha: summary.recorded_sha.clone(),
        tokens,
        event_counts: counts,
        warnings,
    };
    // The size cap is enforced by dropping events from the end; the flag
    // tells the website the tail is missing.
    let mut kept = events.len();
    loop {
        let bytes =
            serde_json::to_vec(&facts.view(&events[..kept], truncated || kept < events.len()))
                .map_err(|error| format!("encode session view: {error}"))?;
        if bytes.len() <= VIEW_BYTES {
            return Ok(BuiltView {
                resource_id: resource_id.to_owned(),
                source_sha256: sha256_hex(transcript),
                sha256: sha256_hex(&bytes),
                bytes: bytes.into(),
                events: kept,
                truncated: truncated || kept < events.len(),
            });
        }
        if kept == 0 {
            return Err("a session view exceeds 8 MiB with no events".to_string());
        }
        kept /= 2;
    }
}

/// Push omits reproducible Codex display records. Recreate them only for
/// the preview; its source digest still identifies the original wire bytes.
fn codex_display_transcript(transcript: &[u8]) -> Result<Vec<u8>, String> {
    let (metadata, header) =
        authoritative_session_meta_with_record(transcript).map_err(|error| error.to_string())?;
    let mut records = Vec::new();
    for (index, record) in parsed_records(transcript).enumerate() {
        if index >= MAX_CODEX_RECORDS {
            return Err("Codex view transcript has too many records".to_string());
        }
        let mut record =
            record.map_err(|_| "Codex view transcript contains malformed JSON".to_string())?;
        if structural_record_type(&record) == Some("session_meta") {
            continue;
        }
        record
            .as_object_mut()
            .ok_or("Codex view transcript record is not an object")?
            .remove("ordinal");
        records.push(record);
    }
    let projected = complete_paginated_item_projection(records, &metadata.id, &mut |_| Ok(()))?;
    let mut bytes = Vec::new();
    for record in std::iter::once(header).chain(projected) {
        serde_json::to_writer(&mut bytes, &record)
            .map_err(|error| format!("encode Codex view transcript: {error}"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > crate::app_settings::conversation_file_limit_bytes() {
            return Err("Codex view transcript exceeds the bounded transcript limit".to_string());
        }
    }
    Ok(bytes)
}

/// Views for every conversation whose transcript object is in `staged`:
/// an unchanged transcript keeps the view the service already holds.
pub fn build_views_for_manifest(
    registry: &AgentProviderRegistry,
    manifest: &BundleManifest,
    staged: &BTreeMap<String, bytes::Bytes>,
) -> Result<Vec<BuiltView>, String> {
    let mut views = Vec::new();
    for (provider, scope) in manifest.provider_scopes() {
        for (thread_id, thread) in &scope.conversations {
            let Some(transcript) = thread
                .files
                .iter()
                .find(|file| file.role == ConversationFileRole::Transcript)
            else {
                continue;
            };
            let Some(entry) = scope.files.get(&transcript.logical_path) else {
                continue;
            };
            let Some(bytes) = staged.get(&entry.sha256) else {
                continue;
            };
            let session_id = thread
                .source_thread_id
                .as_deref()
                .unwrap_or(thread_id.as_str());
            views.push(build_view(
                registry,
                provider,
                thread.resource_id.as_str(),
                session_id,
                thread.display_title.as_deref(),
                bytes,
            )?);
        }
    }
    Ok(views)
}

fn all_kinds() -> BTreeSet<SessionTimelineEventKind> {
    [
        SessionTimelineEventKind::Prompt,
        SessionTimelineEventKind::Response,
        SessionTimelineEventKind::IntermediateStep,
        SessionTimelineEventKind::Checkpoint,
        SessionTimelineEventKind::ToolCall,
        SessionTimelineEventKind::AgentCall,
    ]
    .into_iter()
    .collect()
}

fn all_categories() -> BTreeSet<SessionToolCategory> {
    [
        SessionToolCategory::FileEdit,
        SessionToolCategory::Shell,
        SessionToolCategory::Read,
        SessionToolCategory::Other,
    ]
    .into_iter()
    .collect()
}

fn safe_stem(value: &str) -> String {
    let stem = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect::<String>();
    if stem.is_empty() {
        "session".to_string()
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn mixed_codex_records() -> Vec<Value> {
        let mut records = vec![json!({
            "type": "session_meta",
            "payload": {"id": "wire-root", "cwd": "/__mallard_root__/b;", "history_base": null}
        })];
        for turn in 0..6 {
            let turn_id = format!("turn-{turn}");
            let event = |second: u8, payload: Value| {
                json!({
                    "timestamp": format!("2026-08-07T00:{turn:02}:{second:02}Z"),
                    "type": "event_msg",
                    "payload": payload
                })
            };
            records.push(event(
                0,
                json!({"type": "task_started", "turn_id": turn_id}),
            ));
            if turn < 5 {
                records.push(event(
                    1,
                    json!({"type": "user_message", "message": format!("Prompt {turn}")}),
                ));
                if turn < 3 {
                    records.push(event(2, json!({"type": "agent_message", "message": "Working", "phase": "commentary"})));
                    records.push(event(3, json!({
                        "type": "patch_apply_end", "turn_id": turn_id, "call_id": format!("patch-{turn}"),
                        "stdout": "", "stderr": "", "success": true, "changes": {}, "status": "completed"
                    })));
                }
                records.push(event(4, json!({"type": "agent_message", "message": format!("Reply {turn}"), "phase": "final_answer"})));
            } else {
                records.push(event(1, json!({
                    "type": "item_completed", "thread_id": "wire-root", "turn_id": turn_id,
                    "item": {"type": "UserMessage", "id": "native-prompt", "content": [{"type": "text", "text": format!("Prompt {turn}")}]}
                })));
                records.push(event(4, json!({
                    "type": "item_completed", "thread_id": "wire-root", "turn_id": turn_id,
                    "item": {"type": "AgentMessage", "id": "native-reply", "phase": "final_answer", "content": [{"type": "Text", "text": format!("Reply {turn}")}]}
                })));
            }
            records.push(event(
                5,
                json!({"type": "task_complete", "turn_id": turn_id}),
            ));
        }
        records.push(json!({
            "type": "event_msg", "payload": {"type": "token_count", "info": {
                "total_token_usage": {"input_tokens": 1200, "output_tokens": 60, "total_tokens": 1260}
            }}
        }));
        records
    }

    fn codex_view(records: Vec<Value>) -> (Vec<u8>, BuiltView, Value) {
        let mut wire = Vec::new();
        for (ordinal, mut record) in records.into_iter().enumerate() {
            record["ordinal"] = json!(ordinal);
            serde_json::to_writer(&mut wire, &record).unwrap();
            wire.push(b'\n');
        }
        let view = build_view(
            AgentProviderRegistry::production(),
            &ProviderKey::parse("codex").unwrap(),
            "resource-view-test",
            "source-session",
            None,
            &wire,
        )
        .unwrap();
        let value = serde_json::from_slice(&view.bytes).unwrap();
        (wire, view, value)
    }

    #[test]
    fn mixed_codex_wire_keeps_every_turn_and_original_digest() {
        let (wire, view, value) = codex_view(mixed_codex_records());
        assert_eq!(value["event_counts"]["prompts"]["value"], 6);
        assert_eq!(value["event_counts"]["responses"]["value"], 6);
        assert_eq!(value["event_counts"]["intermediate_steps"]["value"], 3);
        assert_eq!(value["event_counts"]["tool_calls"]["value"], 3);
        assert_eq!(value["tokens"]["value"], 1260);
        assert_eq!(value["provider_session_id"], "wire-root");
        assert_eq!(view.events, 18);
        assert!(!view.truncated);
        assert_eq!(view.source_sha256, sha256_hex(&wire));
        assert_eq!(view.sha256, sha256_hex(&view.bytes));
        let messages = value["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == "prompt" || event["kind"] == "response")
            .map(|event| event["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let expected = (0..6)
            .flat_map(|turn| [format!("Prompt {turn}"), format!("Reply {turn}")])
            .collect::<Vec<_>>();
        assert_eq!(messages, expected);
    }

    #[test]
    fn complete_codex_display_records_are_not_duplicated() {
        let mut records = mixed_codex_records();
        let header = records.remove(0);
        let projected =
            complete_paginated_item_projection(records, "wire-root", &mut |_| Ok(())).unwrap();
        let (wire, view, value) = codex_view(std::iter::once(header).chain(projected).collect());
        assert_eq!(value["event_counts"]["prompts"]["value"], 6);
        assert_eq!(value["event_counts"]["responses"]["value"], 6);
        assert_eq!(value["event_counts"]["intermediate_steps"]["value"], 3);
        assert_eq!(value["event_counts"]["tool_calls"]["value"], 3);
        assert_eq!(view.events, 18);
        assert_eq!(view.source_sha256, sha256_hex(&wire));
    }

    #[test]
    fn a_standalone_codex_wire_rollout_builds_a_view_without_a_provider_home() {
        let wire = include_bytes!("../fixtures/standalone_codex_wire.jsonl");
        let provider = ProviderKey::parse("codex").unwrap();
        let view = build_view(
            AgentProviderRegistry::production(),
            &provider,
            "resource-view-test",
            "fixture-leaf",
            Some("Fixture leaf"),
            wire,
        )
        .unwrap();
        assert_eq!(view.source_sha256, sha256_hex(wire));
        assert_eq!(view.sha256, sha256_hex(&view.bytes));
        assert!(!view.truncated);
        let value: serde_json::Value = serde_json::from_slice(&view.bytes).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["resource_id"], "resource-view-test");
        assert_eq!(value["provider"], "codex");
        assert_eq!(value["title"], "Fixture leaf");
        assert_eq!(
            value["event_counts"]["checkpoints"]["quality"],
            "unsupported"
        );
        assert!(!value["events"].as_array().unwrap().is_empty());
        assert_eq!(view.events, value["events"].as_array().unwrap().len());
    }

    #[test]
    fn claude_transcripts_build_views_too() {
        let mut lines = Vec::new();
        lines.push(serde_json::json!({"type":"user","uuid":"u-1","sessionId":"claude-view","cwd":"/__mallard_root__","timestamp":"2026-08-07T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"Read the source"}]}}).to_string());
        lines.push(serde_json::json!({"type":"assistant","uuid":"a-1","parentUuid":"u-1","sessionId":"claude-view","cwd":"/__mallard_root__","timestamp":"2026-08-07T00:00:05Z","message":{"role":"assistant","model":"claude","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":10,"output_tokens":5}}}).to_string());
        let wire = format!("{}\n", lines.join("\n"));
        let provider = ProviderKey::parse("claude").unwrap();
        let view = build_view(
            AgentProviderRegistry::production(),
            &provider,
            "resource-claude",
            "claude-view",
            None,
            wire.as_bytes(),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&view.bytes).unwrap();
        assert_eq!(value["provider"], "claude");
        assert_eq!(value["provider_session_id"], "claude-view");
        assert!(value["events"].as_array().unwrap().len() >= 2);
    }
}
