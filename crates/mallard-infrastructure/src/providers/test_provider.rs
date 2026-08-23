//! A minimal third provider used only by tests to prove the common setup,
//! Activity, continuity, and review layers are registration-driven. Its
//! transcript format reuses the Claude JSONL shape under its own codec key
//! and home layout (`<home>/sessions/*.jsonl`).

use super::claude::codec::analyze_claude_transcript;
use super::traits::{
    AgentProviderAdapter, DiscoveredSession, MaterializationTargetRequest, PlannedProviderTarget,
    ProviderCapabilityLevel, ProviderDescriptor, SessionActionContext, SessionActionDescriptor,
    SessionActionOutcome, SessionActionRequest, SessionAdapter, SessionAnalysis, SessionDiscovery,
    SessionDiscoveryContext, SessionEventDetail, SessionFileMetadata, SessionFileSummary,
    SessionRewriteRequest, SessionTimelineContext, SessionTimelineIndex, SessionTimelinePageSlice,
    SessionTimelineQuery,
};
use crate::provider_capture::{
    canonical_existing_dir, provider_walk_entry, scan_session_metadata_with,
    session_home_is_project_root, ParsedSessionMetadata,
};
use mallard_domain::{ConversationFileRole, ProviderKey};
use serde::Deserialize;
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
        cwd: record.cwd,
        is_internal_subagent: false,
    })
}

/// The generic structural rewrite the non-cursor pull branch exercises:
/// restamp `sessionId`/`cwd` occurrences equal to the reviewed source
/// identity and prove the result round-trips with identical lineage and
/// semantics. This moved here from the Claude codec when Claude cut over to
/// the wire decode — real providers materialize through their wire forms,
/// and only this registration-driven test adapter still rewrites.
fn structural_session_rewrite(
    bytes: &[u8],
    expected_source_session_id: &str,
    destination_session_id: &str,
    expected_source_cwd: &str,
    destination_cwd: &str,
) -> Result<Vec<u8>, String> {
    use serde_json::{Map, Value};

    fn rewrite_matching_string(
        object: &mut Map<String, Value>,
        key: &str,
        expected: &str,
        replacement: &str,
    ) -> Result<(), String> {
        let Some(value) = object.get_mut(key) else {
            return Ok(());
        };
        let observed = value
            .as_str()
            .ok_or_else(|| format!("structural field '{key}' is not a string"))?;
        if observed == expected {
            *value = Value::String(replacement.to_string());
        }
        Ok(())
    }

    fn validate_rewrite_value(label: &str, value: &str, max: usize) -> Result<(), String> {
        if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
            Err(format!("invalid rewrite {label}"))
        } else {
            Ok(())
        }
    }

    fn lineage_digests(analysis: &SessionAnalysis) -> Option<(String, String)> {
        match &analysis.lineage {
            crate::conversation_codec::CodecLineage::Complete {
                lineage_digest,
                semantic_digest,
                ..
            } => Some((lineage_digest.clone(), semantic_digest.clone())),
            crate::conversation_codec::CodecLineage::Unknown { .. } => None,
        }
    }

    validate_rewrite_value("source session id", expected_source_session_id, 128)?;
    validate_rewrite_value("destination session id", destination_session_id, 128)?;
    validate_rewrite_value("source cwd", expected_source_cwd, 4096)?;
    validate_rewrite_value("destination cwd", destination_cwd, 4096)?;
    let before = analyze_claude_transcript(bytes);
    if !before.is_resumable()
        || before.source_provider_session_id.as_deref() != Some(expected_source_session_id)
        || before.source_cwd.as_deref() != Some(expected_source_cwd)
    {
        return Err("transcript does not match the reviewed source identity".to_string());
    }

    let final_newline = bytes.ends_with(b"\n");
    let mut lines = Vec::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_slice(raw)
            .map_err(|_| "transcript contains malformed JSON".to_string())?;
        if let Some(object) = value.as_object_mut() {
            rewrite_matching_string(
                object,
                "sessionId",
                expected_source_session_id,
                destination_session_id,
            )?;
            rewrite_matching_string(object, "cwd", expected_source_cwd, destination_cwd)?;
        }
        lines.push(
            serde_json::to_vec(&value)
                .map_err(|error| format!("serialize rewritten record: {error}"))?,
        );
    }
    let mut rewritten = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            rewritten.push(b'\n');
        }
        rewritten.extend_from_slice(line);
    }
    if final_newline {
        rewritten.push(b'\n');
    }

    let after = analyze_claude_transcript(&rewritten);
    if !after.is_resumable()
        || after.source_provider_session_id.as_deref() != Some(destination_session_id)
        || after.source_cwd.as_deref() != Some(destination_cwd)
        || lineage_digests(&before) != lineage_digests(&after)
    {
        return Err("rewritten transcript failed semantic round-trip validation".to_string());
    }
    Ok(rewritten)
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
            if !session_home_is_project_root(context.project_root, &meta.cwd) {
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
        structural_session_rewrite(
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

    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String> {
        super::claude::history::summarize_bytes(bytes)
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
