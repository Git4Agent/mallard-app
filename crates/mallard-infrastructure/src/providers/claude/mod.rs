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
use crate::provider_capture::{scan_session_metadata_with, MetadataScanSpec};
use mallard_domain::ProviderKey;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub use codec::{
    analyze_claude_transcript, rewrite_claude_transcript, CLAUDE_CODEC_VERSION,
    CLAUDE_LINEAGE_CODEC_KEY,
};
pub use materialization::encode_claude_project_path;

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse("claude").expect("static provider key")
}

const CLAUDE_SCAN_SPEC: MetadataScanSpec = MetadataScanSpec {
    authoritative: |event_type, value| {
        event_type.contains("system")
            || event_type.contains("init")
            || value.get("sessionId").is_some()
    },
    id_keys_authoritative: &["sessionId", "session_id", "threadId", "thread_id", "id"],
    id_keys_fallback: &["sessionId", "session_id", "threadId", "thread_id", "id"],
    detect_subagent: |_value: &Value| false,
};

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
            setup_default: false,
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

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        discovery::discover(context, |path| self.scan_metadata(path))
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_session_metadata_with(path, &CLAUDE_SCAN_SPEC)
    }

    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        codec::analyze_claude_transcript(bytes)
    }

    fn rewrite(&self, request: &SessionRewriteRequest<'_>) -> Result<Vec<u8>, String> {
        let source_id = request
            .expected_source_session_id
            .ok_or_else(|| "Claude rewrite requires the reviewed source session id".to_string())?;
        let source_cwd = request
            .expected_source_cwd
            .ok_or_else(|| "Claude rewrite requires the reviewed source cwd".to_string())?;
        codec::rewrite_claude_transcript(
            request.bytes,
            source_id,
            request.destination_session_id,
            source_cwd,
            request.destination_cwd,
        )
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
