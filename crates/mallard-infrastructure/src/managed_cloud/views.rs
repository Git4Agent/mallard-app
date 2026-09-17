//! Session views for the website (architecture section 5.4). A view is the
//! desktop's normalized timeline of one conversation, built from the wire
//! bytes that are being pushed, in a private temporary provider home with
//! an empty project root: no checkpoint rows, no child tail facts. The
//! service validates its shape and the website renders it as-is.
use crate::providers::traits::{
    SessionCount, SessionCountQuality, SessionDetailWarning, SessionEventCounts,
    SessionTimelineEvent, SessionTimelineEventKind, SessionTimelineQuery, SessionToolCategory,
};
use crate::providers::{AgentProviderRegistry, SessionTimelineContext};
use mallard_domain::{
    sha256_hex, BundleManifest, ConversationFileRole, NoticeSeverity, ProviderKey,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

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
            schema_version: 1,
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
    std::fs::write(&path, transcript).map_err(|error| format!("view workspace: {error}"))?;
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
