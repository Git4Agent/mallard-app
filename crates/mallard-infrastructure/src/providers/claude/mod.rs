//! The typed Claude provider adapter: profile facts, session discovery,
//! bounded history parsing, the versioned lineage codec, materialization
//! target planning, and safe session actions.

pub mod actions;
pub mod codec;
pub mod discovery;
pub mod history;
pub mod materialization;

use super::traits::{
    AgentProviderAdapter, MaterializationTargetRequest, PlannedProviderTarget,
    ProviderCapabilityLevel, ProviderDescriptor, SessionActionContext, SessionActionDescriptor,
    SessionActionOutcome, SessionActionRequest, SessionAdapter, SessionAnalysis, SessionDiscovery,
    SessionDiscoveryContext, SessionEventDetail, SessionFileMetadata, SessionFileSummary,
    SessionRewriteRequest, SessionTimelineContext, SessionTimelineIndex, SessionTimelinePageSlice,
    SessionTimelineQuery,
};
use crate::provider_capture::{scan_session_metadata_with, ParsedSessionMetadata};
use mallard_domain::ProviderKey;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub use codec::{
    analyze_claude_transcript, project_claude_record, CLAUDE_CODEC_VERSION,
    CLAUDE_LINEAGE_CODEC_KEY,
};
pub use materialization::encode_claude_project_path;

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse("claude").expect("static provider key")
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ClaudeMetadataRecordType {
    User,
    Assistant,
    System,
}

#[derive(Deserialize)]
struct ClaudeSessionMetadataRecord {
    #[serde(rename = "type")]
    record_type: ClaudeMetadataRecordType,
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
    #[serde(default, rename = "isSidechain")]
    is_sidechain: bool,
    #[serde(default, rename = "isMeta")]
    is_meta: bool,
}

fn parse_claude_session_metadata(line: &[u8]) -> Option<ParsedSessionMetadata> {
    let record: ClaudeSessionMetadataRecord = serde_json::from_slice(line).ok()?;
    match record.record_type {
        ClaudeMetadataRecordType::User
        | ClaudeMetadataRecordType::Assistant
        | ClaudeMetadataRecordType::System => {}
    }
    if record.is_sidechain || record.is_meta {
        return None;
    }
    Some(ParsedSessionMetadata {
        session_id: record.session_id,
        cwd: record.cwd,
        is_internal_subagent: false,
    })
}

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentProviderAdapter for ClaudeAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: provider_key(),
            display_name: "Claude".to_string(),
            profile_label: ".claude profile".to_string(),
            default_profile_directory: Some(".claude".to_string()),
            setup_default: true,
            capabilities: ProviderCapabilityLevel::ResumableWithConflictPreservation,
            adapter_version: 1,
        }
    }

    fn sessions(&self) -> Option<&dyn SessionAdapter> {
        Some(self)
    }
}

impl SessionAdapter for ClaudeAdapter {
    fn codec_key(&self) -> &str {
        CLAUDE_LINEAGE_CODEC_KEY
    }

    fn codec_version(&self) -> u32 {
        CLAUDE_CODEC_VERSION
    }

    fn timeline_version(&self) -> u32 {
        history::TIMELINE_PARSER_VERSION
    }

    fn capability(&self) -> ProviderCapabilityLevel {
        ProviderCapabilityLevel::ResumableWithConflictPreservation
    }

    fn supports_tail_cursor(&self) -> bool {
        true
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        discovery::discover(context, |path| self.scan_metadata(path))
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_session_metadata_with(path, parse_claude_session_metadata)
    }

    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        codec::analyze_claude_transcript(bytes)
    }

    fn rewrite(&self, _request: &SessionRewriteRequest<'_>) -> Result<Vec<u8>, String> {
        // Claude conversations materialize exclusively through the pull
        // flow's wire decode (verify + path decode + sessionId restamp);
        // the generic structural rewrite has no correct meaning for
        // token-form cloud objects and was removed with the path-map
        // cutover.
        Err("Claude conversations materialize through the wire decode".to_string())
    }

    fn materialization_targets(
        &self,
        request: &MaterializationTargetRequest<'_>,
    ) -> Result<Vec<PlannedProviderTarget>, String> {
        materialization::materialization_targets(request)
    }

    fn state_target(
        &self,
        provider_home: &Path,
        project_root: &str,
        relative_cwd: Option<&str>,
        logical_suffix: &str,
    ) -> Result<PathBuf, String> {
        materialization::state_target(provider_home, project_root, relative_cwd, logical_suffix)
    }

    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String> {
        history::summarize_file(path)
    }

    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String> {
        history::summarize_bytes(bytes)
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
    use serde_json::Value;
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
        let path = temp.path().join("session.jsonl");
        let cwd = temp.path().to_string_lossy().into_owned();
        fs::write(
            &path,
            jsonl([
                serde_json::json!({
                    "type": "system-init",
                    "sessionId": "near-match",
                    "cwd": cwd
                }),
                serde_json::json!({
                    "type": "tool_result",
                    "payload": {"sessionId": "nested-tool-id"}
                }),
                serde_json::json!({"type": "diagnostic", "cwd": cwd}),
                serde_json::json!({
                    "type": "user",
                    "sessionId": "sidechain",
                    "cwd": cwd,
                    "isSidechain": true
                }),
            ]),
        )
        .unwrap();

        let error = scan_session_metadata_with(&path, parse_claude_session_metadata).unwrap_err();
        assert!(error.contains("authoritative session metadata"));
    }

    #[test]
    fn metadata_scan_reads_complete_durable_top_level_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let cwd = temp.path().to_string_lossy().into_owned();
        fs::write(
            &path,
            jsonl([serde_json::json!({
                "type": "user",
                "sessionId": "current-session",
                "cwd": cwd,
                "message": {
                    "id": "unrelated",
                    "input": {"cwd": "/unrelated"}
                }
            })]),
        )
        .unwrap();

        let metadata = scan_session_metadata_with(&path, parse_claude_session_metadata).unwrap();
        assert_eq!(metadata.session_id, "current-session");
        assert_eq!(metadata.cwd, temp.path());
    }
}
