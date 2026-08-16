//! The Codex provider adapter. Codec, rollout structure, materialization, and
//! app-refresh machinery live in their long-standing crate modules; this
//! adapter is the sole registered gateway common code uses to reach them.

pub mod actions;
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
use crate::conversation_codec::{
    analyze_codex_transcript, CODEX_CODEC_VERSION, CODEX_LINEAGE_CODEC_KEY,
};
use crate::provider_capture::{scan_session_metadata_with, MetadataScanSpec};
use mallard_domain::ProviderKey;
use std::path::Path;

pub use actions::codex_app_project_open_command;
pub use history::{first_codex_user_summary, parse_session_index_titles};

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse("codex").expect("static provider key")
}

const CODEX_SCAN_SPEC: MetadataScanSpec = MetadataScanSpec {
    authoritative: |event_type, _value| event_type.contains("session_meta"),
    // Modern Codex subagent metadata carries both `id` (this transcript) and
    // `session_id` (its parent). Prefer `id` on the authoritative
    // session_meta record so parent and child sessions remain distinct.
    id_keys_authoritative: &["id", "thread_id", "threadId", "session_id", "sessionId"],
    id_keys_fallback: &["thread_id", "threadId", "session_id", "sessionId", "id"],
    detect_subagent: |value| crate::codex_rollout::session_meta_is_internal_subagent(value),
};

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

    fn capability(&self) -> ProviderCapabilityLevel {
        ProviderCapabilityLevel::ResumableWithConflictPreservation
    }

    fn index_fragment_limit(&self) -> usize {
        1
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        discovery::discover(context, |path| self.scan_metadata(path))
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_session_metadata_with(path, &CODEX_SCAN_SPEC)
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

    fn rewrite(&self, _request: &SessionRewriteRequest<'_>) -> Result<Vec<u8>, String> {
        // Codex conversations materialize exclusively through the standalone
        // Decode build (wire-form gate + path decode + fresh identity); the
        // generic structural rewrite has no correct meaning for token-form
        // cloud objects and was removed with the canonical wire cutover.
        Err("Codex conversations materialize through the standalone decode build".to_string())
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
