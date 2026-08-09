//! A minimal third provider used only by tests to prove the common setup,
//! Activity, continuity, and review layers are registration-driven. Its
//! transcript format reuses the Claude JSONL shape under its own codec key
//! and home layout (`<home>/sessions/*.jsonl`).

use super::claude::codec::{analyze_claude_transcript, rewrite_claude_transcript};
use super::traits::{
    AgentProviderAdapter, DiscoveredSession, DivergenceStyle, MaterializationTargetRequest,
    PlannedProviderTarget, ProviderCapabilityLevel, ProviderDescriptor, SessionActionContext,
    SessionActionDescriptor, SessionActionOutcome, SessionActionRequest, SessionAdapter,
    SessionAnalysis, SessionDiscovery, SessionDiscoveryContext, SessionEventDetail,
    SessionFileMetadata, SessionFileSummary, SessionRewriteRequest, SessionTimelineContext,
    SessionTimelineIndex, SessionTimelinePageSlice, SessionTimelineQuery,
};
use crate::provider_capture::{
    canonical_existing_dir, owned_relative_cwd, provider_walk_entry, scan_session_metadata_with,
    MetadataScanSpec,
};
use mallard_domain::{ConversationFileRole, ProviderKey};
use std::path::Path;
use walkdir::WalkDir;

pub const TEST_PROVIDER_KEY: &str = "testprov";
pub const TEST_PROVIDER_CODEC_KEY: &str = "testprov.lineage.v1";

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse(TEST_PROVIDER_KEY).expect("static provider key")
}

const TEST_SCAN_SPEC: MetadataScanSpec = MetadataScanSpec {
    authoritative: |_event_type, value| value.get("sessionId").is_some(),
    id_keys_authoritative: &["sessionId"],
    id_keys_fallback: &["sessionId"],
    detect_subagent: |_value| false,
};

pub struct TestProviderAdapter;

impl TestProviderAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TestProviderAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentProviderAdapter for TestProviderAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: provider_key(),
            display_name: "Test Provider".to_string(),
            profile_label: ".testprov profile".to_string(),
            default_profile_directory: Some(".testprov".to_string()),
            setup_default: false,
            capabilities: ProviderCapabilityLevel::Resumable,
            adapter_version: 1,
        }
    }

    fn sessions(&self) -> Option<&dyn SessionAdapter> {
        Some(self)
    }
}

impl SessionAdapter for TestProviderAdapter {
    fn codec_key(&self) -> &str {
        TEST_PROVIDER_CODEC_KEY
    }

    fn codec_version(&self) -> u32 {
        1
    }

    fn timeline_version(&self) -> u32 {
        super::claude::history::TIMELINE_PARSER_VERSION
    }

    fn capability(&self) -> ProviderCapabilityLevel {
        ProviderCapabilityLevel::Resumable
    }

    fn divergence_style(&self) -> DivergenceStyle {
        DivergenceStyle::AdapterFileFork
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        let canonical_home = canonical_existing_dir(context.provider_home, "test provider home")?;
        let mut discovery = SessionDiscovery::default();
        let sessions_root = canonical_home.join("sessions");
        if !sessions_root.exists() {
            return Ok(discovery);
        }
        for entry in WalkDir::new(&sessions_root)
            .follow_links(false)
            .max_depth(2)
            .into_iter()
            .filter_entry(provider_walk_entry)
        {
            let entry = entry.map_err(|e| format!("walk test provider sessions: {}", e))?;
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            let meta = match self.scan_metadata(entry.path()) {
                Ok(meta) => meta,
                Err(error) => {
                    discovery
                        .warnings
                        .push(format!("ignored test provider session: {}", error));
                    continue;
                }
            };
            let Some(relative_cwd) =
                owned_relative_cwd(context.project_root, &meta.cwd, context.excluded_roots)?
            else {
                continue;
            };
            discovery.sessions.push(DiscoveredSession {
                session_id: meta.session_id.clone(),
                relative_cwd,
                physical_path: entry.path().to_path_buf(),
                display_name: meta.session_id,
                blocked_reason: None,
            });
        }
        Ok(discovery)
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_session_metadata_with(path, &TEST_SCAN_SPEC)
    }

    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        analyze_claude_transcript(bytes)
    }

    fn rewrite(&self, request: &SessionRewriteRequest<'_>) -> Result<Vec<u8>, String> {
        let source_id = request
            .expected_source_session_id
            .ok_or_else(|| "test provider rewrite requires a source session id".to_string())?;
        let source_cwd = request
            .expected_source_cwd
            .ok_or_else(|| "test provider rewrite requires a source cwd".to_string())?;
        rewrite_claude_transcript(
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
        if !request.sidecars.is_empty() {
            return Err("test provider sessions have no sidecars".to_string());
        }
        let transcript = match request.existing_transcript_path {
            Some(existing) => existing.to_path_buf(),
            None => request
                .provider_home
                .join("sessions")
                .join(format!("{}.jsonl", request.provider_session_id)),
        };
        Ok(vec![PlannedProviderTarget {
            target_path: transcript,
            role: ConversationFileRole::Transcript,
        }])
    }

    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String> {
        super::claude::history::summarize_file(path)
    }

    fn build_timeline_index(
        &self,
        path: &Path,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelineIndex, String> {
        super::claude::history::build_timeline_index(path, context)
    }

    fn read_timeline_page(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        query: &SessionTimelineQuery,
    ) -> Result<SessionTimelinePageSlice, String> {
        super::claude::history::read_timeline_page(path, index, query)
    }

    fn read_event_detail(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        event_id: &str,
    ) -> Result<SessionEventDetail, String> {
        super::claude::history::read_event_detail(path, index, event_id)
    }

    fn available_actions(
        &self,
        _context: &SessionActionContext<'_>,
    ) -> Vec<SessionActionDescriptor> {
        Vec::new()
    }

    fn execute_action(
        &self,
        request: &mut SessionActionRequest<'_>,
    ) -> Result<SessionActionOutcome, String> {
        Err(format!(
            "test provider has no session action '{}'",
            request.action_id
        ))
    }
}
