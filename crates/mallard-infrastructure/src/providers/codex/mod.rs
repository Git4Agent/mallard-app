//! The Codex provider adapter. Codec, rollout structure, materialization, and
//! app-refresh machinery live in their long-standing crate modules; this
//! adapter is the sole registered gateway common code uses to reach them.

pub mod actions;
pub mod discovery;
pub mod history;
pub mod materialization;

use super::traits::{
    shell_quote, AfterPullActionOffer, MaintenanceActionDescriptor, MaintenanceActionScope,
    MaterializationTargetRequest, PlannedProviderTarget, ProjectOpenCommand, ProviderCliLauncher,
    ProviderDescriptor, SessionActionContext, SessionActionDescriptor, SessionActionOutcome,
    SessionActionRequest, SessionAdapter, SessionAgentPreview, SessionAnalysis,
    SessionChildIdentity, SessionDiscovery, SessionDiscoveryContext, SessionEventDetail,
    SessionFileMetadata, SessionFileSummary, SessionTimelineContext, SessionTimelineIndex,
    SessionTimelineIndexEntry, SessionTimelinePageSlice, SessionTimelineQuery,
};
use crate::conversation_codec::{
    analyze_codex_transcript, CODEX_CODEC_VERSION, CODEX_LINEAGE_CODEC_KEY,
};
use crate::provider_capture::{
    scan_session_metadata_with, Discovery, ParsedSessionMetadata, MAX_FILE_BYTES,
};
use mallard_domain::ProviderKey;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
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
                authorized_home: payload.cwd,
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

impl SessionAdapter for CodexAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: provider_key(),
            display_name: "Codex".to_string(),
            display_rank: 10,
            profile_label: ".codex profile".to_string(),
            default_profile_directory: Some(".codex".to_string()),
            setup_default: true,
            cli_launcher: Some(ProviderCliLauncher {
                program: "codex".to_string(),
                display_name: "Codex".to_string(),
            }),
            maintenance_actions: self.maintenance_actions(),
        }
    }

    fn watch_roots(&self) -> &'static [&'static str] {
        &["sessions", "archived_sessions"]
    }

    fn session_title_index(&self) -> Option<&'static str> {
        Some("session_index.jsonl")
    }

    fn refresh_session_titles(
        &self,
        discovery: &mut crate::provider_capture::Discovery,
        canonical_home: &Path,
    ) -> Result<(), String> {
        refresh_codex_session_titles(discovery, canonical_home)
    }

    fn reconstructs_rollout_graph(&self) -> bool {
        true
    }

    fn mint_provider_thread_id(&self, taken: &dyn Fn(&str) -> bool) -> Result<String, String> {
        crate::codex_thread_allocator::allocate_codex_thread_ids_local(1, &taken)?
            .into_iter()
            .next()
            .ok_or_else(|| "Codex thread allocation returned no id".to_string())
    }

    fn tail_layout_codec(&self) -> Option<&'static str> {
        Some(mallard_domain::PORTABLE_TAIL_CODEC_V1)
    }

    fn conversation_wire_rules(&self) -> Option<mallard_domain::ConversationWireRules> {
        Some(mallard_domain::ConversationWireRules {
            requires_redaction_stamp: true,
            requires_portable_tail: true,
            path_map: mallard_domain::PathMapDeclaration::InBand,
            tail_wire: mallard_domain::TailWireKind::CodexRollout,
        })
    }

    fn decode_portable_display_title(
        &self,
        title: &str,
        project_root: &str,
        canonical_project_root: &str,
    ) -> Result<String, String> {
        crate::session_path_map::validate_pathmap_root("project root", project_root)?;
        let home = std::env::var("HOME").ok();
        let mapper = crate::session_path_map::SessionPathMapper::new(
            project_root,
            (canonical_project_root != project_root).then_some(canonical_project_root),
            home.as_deref(),
        )?;
        let wire = serde_json::Value::String(title.to_string());
        mapper
            .verify_wire_record(&wire)
            .map_err(|error| format!("portable Codex display title: {error}"))?;
        match mapper.decode_record(&wire)? {
            serde_json::Value::String(decoded) => Ok(decoded),
            _ => Err("portable Codex display title did not decode to text".to_string()),
        }
    }

    fn maintenance_actions(&self) -> Vec<MaintenanceActionDescriptor> {
        let action = |id: &str| mallard_domain::ActionKey::parse(id).expect("static action key");
        vec![MaintenanceActionDescriptor {
            action_id: action("codex.app.restart"),
            label: "Restart Codex App".to_string(),
            description: "Relaunch the Codex desktop app so restored sessions appear.".to_string(),
            scope: MaintenanceActionScope::AfterPull,
            requires_confirmation: true,
            read_only: false,
            display_rank: 10,
        }]
    }

    fn project_open_commands(&self, home: &Path, project_root: &str) -> Vec<ProjectOpenCommand> {
        let environment = format!("CODEX_HOME={}", shell_quote(&home.to_string_lossy()));
        let project = shell_quote(project_root);
        let mut commands = vec![
            ProjectOpenCommand {
                label: "Codex — new".to_string(),
                command: format!("{} codex -C {}", environment, project),
            },
            ProjectOpenCommand {
                label: "Codex — resume".to_string(),
                command: format!("{} codex resume -C {}", environment, project),
            },
        ];
        // The desktop app is launched through a deep link, not the CLI, so it
        // needs its own line rather than a variation on the two above.
        #[cfg(target_os = "macos")]
        {
            if let Some(command) = codex_app_project_open_command(home, Path::new(project_root)) {
                commands.push(ProjectOpenCommand {
                    label: "Codex app — new".to_string(),
                    command,
                });
            }
            // Resuming a thread in the app takes two sequenced deep links, so
            // it has no pasteable form; point at the button that performs
            // them instead.
            commands.push(ProjectOpenCommand {
                label: "Codex app — resume".to_string(),
                command: "use Open in Codex (one deep link cannot carry both project and chat)"
                    .to_string(),
            });
        }
        commands
    }
    fn after_pull_offer(
        &self,
        operation_id: &mallard_domain::ConversationOperationId,
        provider_home: Option<&Path>,
    ) -> Option<AfterPullActionOffer> {
        let offer = crate::codex_app_refresh::refresh_offer(operation_id.clone(), provider_home)?;
        let arguments = mallard_domain::BoundedConfigObject::from_json(&serde_json::json!({
            "operation_id": offer.operation_id.as_str(),
            "app_kind": offer.app_kind,
        }))
        .ok()?;
        Some(AfterPullActionOffer {
            provider: provider_key(),
            action_id: mallard_domain::ActionKey::parse("codex.app.restart")
                .expect("static action key"),
            label: format!("Restart {}", offer.app_name),
            message: offer.message,
            available: offer.state
                == crate::codex_app_refresh::CodexAppRefreshState::RestartAvailable,
            arguments,
        })
    }

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

    fn analyze_and_summarize(
        &self,
        bytes: &[u8],
    ) -> (SessionAnalysis, Result<SessionFileSummary, String>) {
        let (analysis, summary) = history::analyze_and_summarize(bytes);
        (
            SessionAnalysis {
                source_provider_session_id: analysis.source_thread_id,
                source_cwd: analysis.source_cwd,
                is_internal_subagent: analysis.is_internal_subagent,
                record_count: analysis.record_count,
                lineage: analysis.lineage,
            },
            summary,
        )
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

    fn child_transcript_target(
        &self,
        request: &MaterializationTargetRequest<'_>,
        child_id: &str,
    ) -> Result<std::path::PathBuf, String> {
        let targets = self.materialization_targets(&MaterializationTargetRequest {
            provider_session_id: child_id,
            source_provider_session_id: None,
            existing_transcript_path: None,
            sidecars: &[],
            ..*request
        })?;
        if targets.len() != 1 {
            return Err("child requires one transcript target".into());
        }
        Ok(targets[0].target_path.clone())
    }

    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String> {
        history::summarize_rollout_file(path)
    }

    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String> {
        history::summarize_rollout_bytes(bytes)
    }

    /// A child rollout is a rollout: its own thread id is the agent id.
    fn summarize_child_file(
        &self,
        path: &Path,
        child: &SessionChildIdentity,
    ) -> Result<SessionFileSummary, String> {
        let summary = history::summarize_rollout_file(path)?;
        if summary.session_id != child.provider_agent_id {
            return Err("Codex child rollout does not carry the expected thread id".to_string());
        }
        Ok(summary)
    }

    /// A Codex child is found by id across the sessions tree, then proven to
    /// name this parent in its own `session_meta`.
    fn locate_child_transcript(
        &self,
        provider_home: &Path,
        _parent_transcript: &Path,
        child: &SessionChildIdentity,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let Some(path) = crate::codex_subagents::RolloutLookup::new(provider_home)
            .find(&child.provider_agent_id)?
        else {
            return Ok(None);
        };
        let head = history::codex_child_head(&path)?;
        if head.thread_id.as_deref() != Some(child.provider_agent_id.as_str())
            || head.parent_thread_id.as_deref() != Some(child.parent_provider_session_id.as_str())
        {
            return Err("Codex child rollout does not name this parent".to_string());
        }
        Ok(Some(path))
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
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelinePageSlice, String> {
        history::read_timeline_page(path, index, query, context)
    }

    fn describe_agent_call(
        &self,
        _path: &Path,
        entry: &SessionTimelineIndexEntry,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionAgentPreview, String> {
        history::describe_agent_call(entry, context)
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

    #[cfg(unix)]
    #[test]
    fn pull_thread_ids_are_minted_without_starting_codex() {
        use std::os::unix::fs::PermissionsExt;

        const CHILD: &str = "MALLARD_TEST_LOCAL_CODEX_IDS";
        if std::env::var_os(CHILD).is_some() {
            let adapter = CodexAdapter::new();
            let mut occupied = BTreeSet::new();
            for _ in 0..35 {
                let id = adapter
                    .mint_provider_thread_id(&|candidate| occupied.contains(candidate))
                    .unwrap();
                crate::codex_thread_allocator::validate_uuid_v7(&id).unwrap();
                assert!(occupied.insert(id));
            }
            return;
        }

        // Isolate PATH and homes in a child test process. A CLI allocation
        // would start this sentinel even if it then fell back to local IDs.
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("codex");
        let launch_log = temp.path().join("codex-launches");
        fs::write(
            &executable,
            "#!/bin/sh\nprintf 'started\\n' >> \"$MALLARD_TEST_CODEX_LAUNCH_LOG\"\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "providers::codex::tests::pull_thread_ids_are_minted_without_starting_codex",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, "1")
            .env("MALLARD_TEST_CODEX_LAUNCH_LOG", &launch_log)
            .env("PATH", temp.path())
            .env("HOME", temp.path())
            .env("USERPROFILE", temp.path())
            .env("CODEX_HOME", temp.path())
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "local ID test failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!launch_log.exists(), "Pull ID allocation started Codex");
    }

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
        assert_eq!(metadata.authorized_home, temp.path());
    }
}

/// Re-run only the session-index title enrichment over a cached discovery:
/// a replaced `session_index.jsonl` invalidates display titles, never tail
/// evidence or membership.
fn refresh_codex_session_titles(
    discovery: &mut Discovery,
    canonical_home: &Path,
) -> Result<(), String> {
    let index = canonical_home.join("session_index.jsonl");
    discovery.refresh_conversation_titles(&provider_key(), |ids| {
        if ids.is_empty() || !index.is_file() {
            return Ok(BTreeMap::new());
        }
        Ok(parse_session_index_titles(
            &filter_codex_session_index_by_ids(&index, ids)?,
        ))
    })
}

pub(crate) fn filter_codex_session_index_by_ids(
    path: &Path,
    ids: &BTreeSet<String>,
) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("inspect '{}': {}", path.display(), e))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "'{}' exceeds {} bytes",
            path.display(),
            MAX_FILE_BYTES
        ));
    }
    let file = fs::File::open(path).map_err(|e| format!("open '{}': {}", path.display(), e))?;
    let mut output = Vec::new();
    for line in BufReader::new(file).split(b'\n') {
        let line = line.map_err(|e| format!("read '{}': {}", path.display(), e))?;
        if line.len() > 1024 * 1024 {
            continue;
        }
        let row: CodexSessionIndexIdentity = match serde_json::from_slice(&line) {
            Ok(row) => row,
            Err(_) => continue,
        };
        if ids.contains(&row.id) {
            output.extend_from_slice(&line);
            output.push(b'\n');
        }
    }
    Ok(output)
}

#[derive(Deserialize)]
struct CodexSessionIndexIdentity {
    id: String,
}
