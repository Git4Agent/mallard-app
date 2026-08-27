//! The Codex provider adapter. Codec, rollout structure, materialization, and
//! app-refresh machinery live in their long-standing crate modules; this
//! adapter is the sole registered gateway common code uses to reach them.

pub mod actions;
pub mod discovery;
pub mod history;
pub mod materialization;

use super::traits::{
    AgentProviderAdapter, MaterializationTargetRequest, PlannedProviderTarget,
    ProviderCapabilityLevel, ProviderCliLauncher, ProviderDescriptor, SessionActionContext,
    SessionActionDescriptor, SessionActionOutcome, SessionActionRequest, SessionAdapter,
    SessionAnalysis, SessionDiscovery, SessionDiscoveryContext, SessionEventDetail,
    SessionFileMetadata, SessionFileSummary, SessionTimelineContext, SessionTimelineIndex,
    SessionTimelinePageSlice, SessionTimelineQuery,
};
use crate::conversation_codec::{
    analyze_codex_transcript, CODEX_CODEC_VERSION, CODEX_LINEAGE_CODEC_KEY,
};
use crate::provider_capture::{scan_session_metadata_with, ParsedSessionMetadata};
use mallard_domain::ProviderKey;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

pub use actions::codex_app_project_open_command;
pub use history::{first_codex_user_summary, parse_session_index_titles};

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse("codex").expect("static provider key")
}

#[derive(Deserialize)]
enum CodexMetadataRecordType {
    #[serde(rename = "session_meta")]
    SessionMeta,
}

#[derive(Deserialize)]
struct CodexSessionMetadataEnvelope {
    #[serde(rename = "type")]
    record_type: CodexMetadataRecordType,
    payload: CodexSessionMetadataPayload,
}

#[derive(Deserialize)]
struct CodexSessionMetadataPayload {
    id: String,
    cwd: String,
    #[serde(default)]
    thread_source: Option<String>,
    #[serde(default)]
    source: Option<Value>,
}

fn parse_codex_session_metadata(line: &[u8]) -> Option<ParsedSessionMetadata> {
    let envelope: CodexSessionMetadataEnvelope = serde_json::from_slice(line).ok()?;
    match envelope.record_type {
        CodexMetadataRecordType::SessionMeta => {
            let payload = envelope.payload;
            let is_internal_subagent = payload.thread_source.as_deref() == Some("subagent")
                || payload
                    .source
                    .as_ref()
                    .and_then(Value::as_object)
                    .is_some_and(|source| source.contains_key("subagent"));
            Some(ParsedSessionMetadata {
                session_id: payload.id,
                cwd: payload.cwd,
                is_internal_subagent,
            })
        }
    }
}

pub struct CodexAdapter;

impl CodexAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentProviderAdapter for CodexAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: provider_key(),
            display_name: "Codex".to_string(),
            profile_label: ".codex profile".to_string(),
            default_profile_directory: Some(".codex".to_string()),
            setup_default: true,
            cli_launcher: Some(ProviderCliLauncher {
                program: "codex".to_string(),
                display_name: "Codex".to_string(),
            }),
            capabilities: ProviderCapabilityLevel::ResumableWithConflictPreservation,
            adapter_version: 1,
        }
    }

    fn sessions(&self) -> Option<&dyn SessionAdapter> {
        Some(self)
    }
}

impl SessionAdapter for CodexAdapter {
    fn codec_key(&self) -> &str {
        CODEX_LINEAGE_CODEC_KEY
    }

    fn codec_version(&self) -> u32 {
        CODEX_CODEC_VERSION
    }

    fn timeline_version(&self) -> u32 {
        history::TIMELINE_PARSER_VERSION
    }

    fn index_fragment_limit(&self) -> usize {
        1
    }

    fn supports_tail_cursor(&self) -> bool {
        true
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        discovery::discover(context, |path| self.scan_metadata(path))
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_session_metadata_with(path, parse_codex_session_metadata)
    }

    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        let analysis = analyze_codex_transcript(bytes);
        SessionAnalysis {
            source_provider_session_id: analysis.source_thread_id,
            source_cwd: analysis.source_cwd,
            is_internal_subagent: analysis.is_internal_subagent,
            record_count: analysis.record_count,
            lineage: analysis.lineage,
        }
    }

    fn verify_index_fragment(&self, bytes: &[u8], expected_session_id: &str) -> Result<(), String> {
        materialization::validate_codex_index_fragment(bytes, expected_session_id)
    }

    fn materialization_targets(
        &self,
        request: &MaterializationTargetRequest<'_>,
    ) -> Result<Vec<PlannedProviderTarget>, String> {
        materialization::materialization_targets(request)
    }

    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String> {
        history::summarize_rollout_file(path)
    }

    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String> {
        history::summarize_rollout_bytes(bytes)
    }

    fn build_timeline_index(
        &self,
        path: &Path,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelineIndex, String> {
        history::build_timeline_index(path, context)
    }

    fn read_timeline_page(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        query: &SessionTimelineQuery,
    ) -> Result<SessionTimelinePageSlice, String> {
        history::read_timeline_page(path, index, query)
    }

    fn read_event_detail(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        event_id: &str,
    ) -> Result<SessionEventDetail, String> {
        history::read_event_detail(path, index, event_id)
    }

    fn available_actions(
        &self,
        context: &SessionActionContext<'_>,
    ) -> Vec<SessionActionDescriptor> {
        actions::available_actions(context)
    }

    fn execute_action(
        &self,
        request: &mut SessionActionRequest<'_>,
    ) -> Result<SessionActionOutcome, String> {
        actions::execute_action(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn jsonl(records: impl IntoIterator<Item = Value>) -> String {
        records
            .into_iter()
            .map(|record| record.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    #[test]
    fn metadata_scan_rejects_near_match_nested_and_split_fields() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let cwd = temp.path().to_string_lossy().into_owned();
        fs::write(
            &path,
            jsonl([
                serde_json::json!({
                    "type": "session_metadata",
                    "payload": {"id": "near-match", "cwd": cwd}
                }),
                serde_json::json!({
                    "type": "response_item",
                    "payload": {"tool": {"id": "nested-tool-id"}}
                }),
                serde_json::json!({
                    "type": "diagnostic",
                    "payload": {"cwd": cwd}
                }),
            ]),
        )
        .unwrap();

        let error = scan_session_metadata_with(&path, parse_codex_session_metadata).unwrap_err();
        assert!(error.contains("authoritative session metadata"));
    }

    #[test]
    fn metadata_scan_reads_only_exact_session_meta_payload_fields() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let cwd = temp.path().to_string_lossy().into_owned();
        fs::write(
            &path,
            jsonl([serde_json::json!({
                "type": "session_meta",
                "payload": {
                    "id": "current-session",
                    "cwd": cwd,
                    "tool": {"id": "unrelated", "cwd": "/unrelated"}
                }
            })]),
        )
        .unwrap();

        let metadata = scan_session_metadata_with(&path, parse_codex_session_metadata).unwrap();
        assert_eq!(metadata.session_id, "current-session");
        assert_eq!(metadata.cwd, temp.path());
    }
}
