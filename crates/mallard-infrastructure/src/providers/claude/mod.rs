//! The typed Claude provider adapter: profile facts, session discovery,
//! bounded history parsing, the versioned lineage codec, materialization
//! target planning, and safe session actions.

pub mod actions;
pub mod codec;
pub mod discovery;
pub mod history;
pub mod materialization;

use super::traits::{
    MaterializationTargetRequest, PlannedProviderTarget, ProviderCliLauncher, ProviderDescriptor,
    SessionActionContext, SessionActionDescriptor, SessionActionOutcome, SessionActionRequest,
    SessionAdapter, SessionAnalysis, SessionDiscovery, SessionDiscoveryContext, SessionEventDetail,
    SessionFileMetadata, SessionFileSummary, SessionTimelineContext, SessionTimelineIndex,
    SessionTimelinePageSlice, SessionTimelineQuery,
};
use crate::app_settings::conversation_file_limit_bytes;
use crate::provider_capture::validated_session_metadata;
use mallard_domain::ProviderKey;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub use codec::{
    analyze_claude_transcript, project_claude_record, CLAUDE_CODEC_VERSION,
    CLAUDE_LINEAGE_CODEC_KEY,
};
pub use materialization::encode_claude_project_path;

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse("claude").expect("static provider key")
}

fn scan_claude_session_metadata(path: &Path) -> Result<SessionFileMetadata, String> {
    crate::sync_metrics::record_file_head_read(path);
    let file = crate::jsonl_tail_io::open_no_follow(path)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect Claude session '{}': {}", path.display(), error))?;
    if !metadata.is_file() {
        return Err("Claude session transcript is not a regular file".to_string());
    }
    if metadata.len() > conversation_file_limit_bytes() {
        return Err(format!(
            "Claude session exceeds {} bytes",
            conversation_file_limit_bytes()
        ));
    }

    let mut reader = BufReader::new(file);
    let mut identity = codec::ClaudeSessionIdentityTracker::default();
    let mut bytes_read = 0_u64;
    let mut record_count = 0_usize;
    loop {
        let mut line = Vec::new();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("read Claude session metadata: {}", error))?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read as u64);
        if bytes_read > conversation_file_limit_bytes() {
            return Err(format!(
                "Claude session exceeds {} bytes",
                conversation_file_limit_bytes()
            ));
        }
        // The provider is allowed to be appending. A non-newline-terminated
        // tail is not a durable record yet and cannot change leaf identity.
        if line.last() != Some(&b'\n') {
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        record_count = record_count.saturating_add(1);
        if record_count > codec::MAX_CLAUDE_RECORDS {
            return Err(format!(
                "Claude session exceeds {} metadata records",
                codec::MAX_CLAUDE_RECORDS
            ));
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&line) {
            identity.observe(&value);
        }
    }
    crate::sync_metrics::record_transcript_bytes_read(path, bytes_read);

    let (session_id, cwd) = identity.leaf_identity();
    let session_id = session_id
        .ok_or_else(|| "authoritative Claude leaf session ID was not found".to_string())?;
    let cwd = cwd.ok_or_else(|| "authoritative Claude leaf cwd was not found".to_string())?;
    let file_session_id = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".jsonl"))
        .ok_or_else(|| "Claude session filename is invalid".to_string())?;
    if file_session_id != session_id {
        return Err("Claude session filename does not match its leaf session ID".to_string());
    }
    validated_session_metadata(session_id, cwd, false)
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

impl SessionAdapter for ClaudeAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: provider_key(),
            display_name: "Claude".to_string(),
            profile_label: ".claude profile".to_string(),
            default_profile_directory: Some(".claude".to_string()),
            setup_default: true,
            cli_launcher: Some(ProviderCliLauncher {
                program: "claude".to_string(),
                display_name: "Claude Code".to_string(),
            }),
        }
    }

    fn codec_key(&self) -> &str {
        CLAUDE_LINEAGE_CODEC_KEY
    }

    fn codec_version(&self) -> u32 {
        CLAUDE_CODEC_VERSION
    }

    fn timeline_version(&self) -> u32 {
        history::TIMELINE_PARSER_VERSION
    }

    fn supports_tail_cursor(&self) -> bool {
        true
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        discovery::discover(context, |path| self.scan_metadata(path))
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_claude_session_metadata(path)
    }

    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        codec::analyze_claude_transcript(bytes)
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
        logical_suffix: &str,
    ) -> Result<PathBuf, String> {
        materialization::state_target(provider_home, project_root, logical_suffix)
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

        let error = scan_claude_session_metadata(&path).unwrap_err();
        assert!(error.contains("authoritative Claude leaf"));
    }

    #[test]
    fn metadata_scan_reads_complete_durable_leaf_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("current-session.jsonl");
        let cwd = temp.path().to_string_lossy().into_owned();
        fs::write(
            &path,
            jsonl([serde_json::json!({
                "type": "user",
                "uuid": "user-current",
                "parentUuid": null,
                "sessionId": "current-session",
                "cwd": cwd,
                "message": {
                    "id": "unrelated",
                    "input": {"cwd": "/unrelated"}
                }
            })]),
        )
        .unwrap();

        let metadata = scan_claude_session_metadata(&path).unwrap();
        assert_eq!(metadata.session_id, "current-session");
        assert_eq!(metadata.cwd, temp.path());
    }

    #[test]
    fn metadata_scan_uses_resumed_leaf_identity_and_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let original_cwd = temp.path().join("original");
        let resumed_cwd = temp.path().join("resumed");
        let resumed_turn_cwd = resumed_cwd.join("src-tauri");
        fs::create_dir_all(&original_cwd).unwrap();
        fs::create_dir_all(&resumed_turn_cwd).unwrap();
        let path = temp.path().join("resumed-session.jsonl");
        fs::write(
            &path,
            jsonl([
                serde_json::json!({
                    "type": "user",
                    "uuid": "user-original",
                    "parentUuid": null,
                    "sessionId": "original-session",
                    "cwd": original_cwd,
                    "message": {"role": "user", "content": "original"}
                }),
                serde_json::json!({
                    "type": "assistant",
                    "uuid": "assistant-resumed",
                    "parentUuid": "user-original",
                    "sessionId": "resumed-session",
                    "cwd": resumed_cwd,
                    "message": {"role": "assistant", "content": [{
                        "type": "text", "text": "resumed"
                    }]}
                }),
                serde_json::json!({
                    "type": "user",
                    "uuid": "user-after-cd",
                    "parentUuid": "assistant-resumed",
                    "sessionId": "resumed-session",
                    "cwd": resumed_turn_cwd,
                    "message": {"role": "user", "content": "continue"}
                }),
            ]),
        )
        .unwrap();

        let metadata = scan_claude_session_metadata(&path).unwrap();
        assert_eq!(metadata.session_id, "resumed-session");
        assert_eq!(metadata.cwd, resumed_cwd);

        let mismatched = temp.path().join("wrong-session.jsonl");
        fs::copy(&path, &mismatched).unwrap();
        assert!(scan_claude_session_metadata(&mismatched)
            .unwrap_err()
            .contains("filename does not match"));
    }
}
