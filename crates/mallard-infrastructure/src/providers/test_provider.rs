//! A minimal third provider used only to test registry-driven descriptors,
//! discovery, capture, and codec analysis. Its transcript format reuses the
//! Claude JSONL shape under its own codec key and home layout
//! (`<home>/sessions/*.jsonl`).

use super::claude::codec::analyze_claude_transcript;
use super::traits::{
    DiscoveredSession, MaintenanceActionDescriptor, MaintenanceActionScope,
    MaterializationTargetRequest, PlannedProviderTarget, ProviderDescriptor, ScanFailure,
    SessionActionContext, SessionActionDescriptor, SessionActionOutcome, SessionActionRequest,
    SessionAdapter, SessionAgentPreview, SessionAnalysis, SessionChildIdentity, SessionDiscovery,
    SessionDiscoveryContext, SessionEventDetail, SessionFileMetadata, SessionFileSummary,
    SessionTimelineContext, SessionTimelineIndex, SessionTimelineIndexEntry,
    SessionTimelinePageSlice, SessionTimelineQuery,
};
use crate::provider_capture::{
    canonical_existing_dir, provider_walk_entry, scan_session_metadata_with,
    session_home_is_project_root, ParsedSessionMetadata,
};
use mallard_domain::{ConversationFileRole, ProviderKey};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use walkdir::WalkDir;

pub const TEST_PROVIDER_KEY: &str = "testprov";
pub const TEST_PROVIDER_CODEC_KEY: &str = "testprov.lineage.v1";

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse(TEST_PROVIDER_KEY).expect("static provider key")
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum TestMetadataRecordType {
    User,
    Assistant,
    System,
}

#[derive(Deserialize)]
struct TestSessionMetadataRecord {
    #[serde(rename = "type")]
    record_type: TestMetadataRecordType,
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
    #[serde(default, rename = "isSidechain")]
    is_sidechain: bool,
}

fn parse_test_session_metadata(line: &[u8]) -> Option<ParsedSessionMetadata> {
    let record: TestSessionMetadataRecord = serde_json::from_slice(line).ok()?;
    match record.record_type {
        TestMetadataRecordType::User
        | TestMetadataRecordType::Assistant
        | TestMetadataRecordType::System => {}
    }
    if record.is_sidechain {
        return None;
    }
    Some(ParsedSessionMetadata {
        session_id: record.session_id,
        authorized_home: record.cwd,
        is_internal_subagent: false,
        can_be_empty: false,
    })
}

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

impl SessionAdapter for TestProviderAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: provider_key(),
            display_name: "Test Provider".to_string(),
            display_rank: 90,
            profile_label: ".testprov profile".to_string(),
            default_profile_directory: Some(".testprov".to_string()),
            setup_default: false,
            cli_launcher: None,
            maintenance_actions: self.maintenance_actions(),
        }
    }

    fn maintenance_actions(&self) -> Vec<MaintenanceActionDescriptor> {
        vec![MaintenanceActionDescriptor {
            action_id: mallard_domain::ActionKey::parse("testprov.home.inspect").unwrap(),
            label: "Inspect test home".to_string(),
            description: "A test-only read-only maintenance action.".to_string(),
            scope: MaintenanceActionScope::Profile,
            requires_confirmation: false,
            read_only: true,
            display_rank: 10,
        }]
    }

    fn codec_key(&self) -> &str {
        TEST_PROVIDER_CODEC_KEY
    }

    fn codec_version(&self) -> u32 {
        1
    }

    fn timeline_version(&self) -> u32 {
        super::claude::history::TIMELINE_PARSER_VERSION
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        let canonical_home = canonical_existing_dir(context.provider_home, "test provider home")?;
        let mut discovery = SessionDiscovery::default();
        let sessions_root = canonical_home.join("sessions");
        if !sessions_root.exists() {
            return Ok(discovery);
        }
        let mut canonical_homes = BTreeMap::new();
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
            if !session_home_is_project_root(
                context.project_root,
                &meta.authorized_home,
                &mut canonical_homes,
            ) {
                continue;
            }
            discovery.sessions.push(DiscoveredSession {
                session_id: meta.session_id.clone(),
                physical_path: entry.path().to_path_buf(),
                display_name: meta.session_id,
                blocked_reason: None,
            });
        }
        Ok(discovery)
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_session_metadata_with(path, parse_test_session_metadata)
            .map_err(ScanFailure::into_message)
    }

    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        analyze_claude_transcript(bytes)
    }

    fn analyze_and_summarize(
        &self,
        bytes: &[u8],
    ) -> (SessionAnalysis, Result<SessionFileSummary, String>) {
        (self.analyze(bytes), self.summarize_bytes(bytes))
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

    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String> {
        super::claude::history::summarize_bytes(bytes)
    }

    fn summarize_child_file(
        &self,
        path: &Path,
        child: &SessionChildIdentity,
    ) -> Result<SessionFileSummary, String> {
        super::claude::history::summarize_child_file(path, child)
    }

    fn locate_child_transcript(
        &self,
        provider_home: &Path,
        parent_transcript: &Path,
        child: &SessionChildIdentity,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let path = super::claude::subagents::child_path(
            parent_transcript,
            &child.parent_provider_session_id,
            &child.provider_agent_id,
        )?;
        Ok(super::claude::subagents::child_exists(&path, provider_home)?.then_some(path))
    }

    fn build_timeline_index(
        &self,
        path: &Path,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelineIndex, String> {
        super::claude::history::build_timeline_index(path, context)
    }

    fn describe_agent_call(
        &self,
        path: &Path,
        entry: &SessionTimelineIndexEntry,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionAgentPreview, String> {
        super::claude::history::describe_agent_call(path, entry, context)
    }

    fn read_timeline_page(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        query: &SessionTimelineQuery,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelinePageSlice, String> {
        super::claude::history::read_timeline_page(path, index, query, context)
    }

    fn read_event_detail(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        event_id: &str,
        _context: &SessionTimelineContext<'_>,
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
