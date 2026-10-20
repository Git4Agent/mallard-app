//! Test-only Pi v3 experiment. Run the native SDK round trip with
//! `node scripts/run-pi-harness-poc.mjs`. This is not a production provider.

use super::traits::*;
use super::AgentProviderRegistry;
use crate::conversation_codec::{canonical_json, CodecLineage, CodecUnknownReason};
use crate::provider_capture::{
    self, canonical_existing_dir, session_home_is_project_root, CaptureRequest,
};
use crate::session_path_map::SessionPathMapper;
use mallard_domain::{
    conversation_chain_digests, sha256_hex, CanonicalConversationMessage, ConversationFileRole,
    MessageId, ProviderKey,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;
const MAX_SCAN_ENTRIES: usize = 4096;
const OUTSIDE_POC: &str = "outside the Pi feasibility POC";

fn key() -> ProviderKey {
    ProviderKey::parse("pi").unwrap()
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id.as_bytes()[id.len() - 1].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

struct PiSession {
    records: Vec<Value>,
    session_id: String,
    cwd: String,
    linear: bool,
}

impl PiSession {
    fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("Pi POC byte limit");
        }
        // A moving/torn last record must never become a successful capture.
        if !bytes.ends_with(b"\n") {
            return Err("Pi POC requires complete JSONL records");
        }
        let mut records = Vec::new();
        for line in bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            if records.len() >= MAX_RECORDS {
                return Err("Pi POC record limit");
            }
            records.push(serde_json::from_slice::<Value>(line).map_err(|_| "invalid Pi JSONL")?);
        }
        let header = records.first().ok_or("missing Pi header")?;
        if header["type"] != "session" || header["version"] != 3 {
            return Err("Pi POC accepts only session version 3");
        }
        let session_id = header["id"]
            .as_str()
            .filter(|id| safe_id(id))
            .ok_or("invalid Pi id")?
            .to_owned();
        let cwd = header["cwd"].as_str().ok_or("missing Pi cwd")?.to_owned();
        if cwd.len() > 4096 || cwd.chars().any(char::is_control) || !Path::new(&cwd).is_absolute() {
            return Err("invalid Pi cwd");
        }
        let mut seen = BTreeSet::new();
        let mut previous = None;
        let mut linear = true;
        for record in records.iter().skip(1) {
            if !matches!(
                record["type"].as_str(),
                Some(
                    "message"
                        | "model_change"
                        | "thinking_level_change"
                        | "compaction"
                        | "branch_summary"
                        | "custom"
                        | "custom_message"
                        | "label"
                        | "session_info"
                        | "usage"
                        | "context_edit"
                )
            ) {
                return Err("unsupported Pi entry type");
            }
            let id = record["id"]
                .as_str()
                .filter(|id| safe_id(id))
                .ok_or("invalid Pi entry id")?;
            let parent = match record.get("parentId") {
                Some(Value::Null) => None,
                Some(Value::String(parent)) if seen.contains(parent.as_str()) => {
                    Some(parent.as_str())
                }
                _ => return Err("missing or forward Pi parent"),
            };
            if !seen.insert(id) {
                return Err("duplicate Pi entry id");
            }
            linear &= parent == previous;
            previous = Some(id);
        }
        Ok(Self {
            records,
            session_id,
            cwd,
            linear,
        })
    }

    fn summary(&self) -> SessionFileSummary {
        let mut summary = SessionFileSummary {
            session_id: self.session_id.clone(),
            ..Default::default()
        };
        for record in self.records.iter().skip(1) {
            if record["type"] == "session_info" {
                summary.title = record["name"].as_str().map(str::to_owned);
            }
            if record["type"] == "message" {
                match record["message"]["role"].as_str() {
                    Some("user") => summary.user_rounds += 1,
                    Some("assistant") => summary.agent_messages += 1,
                    _ => {}
                }
                if let Some(blocks) = record["message"]["content"].as_array() {
                    summary.tool_calls += blocks
                        .iter()
                        .filter(|block| block["type"] == "toolCall")
                        .count() as u64;
                }
            }
        }
        // Counts are whole-file preview counts, not active model context.
        summary
    }

    fn analysis(&self) -> SessionAnalysis {
        let lineage = if self.records.len() < 2 {
            CodecLineage::Unknown {
                reason: CodecUnknownReason::Empty,
            }
        } else if !self.linear {
            // Do not pretend an active-branch projection covers all stored branches.
            CodecLineage::Unknown {
                reason: CodecUnknownReason::NonLinearParentage,
            }
        } else {
            let chain = self
                .records
                .iter()
                .skip(1)
                .map(|record| CanonicalConversationMessage {
                    message_id: MessageId::parse(record["id"].as_str().unwrap()).unwrap(),
                    parent_message_id: record["parentId"]
                        .as_str()
                        .map(|id| MessageId::parse(id).unwrap()),
                    semantic_fingerprint: sha256_hex(&canonical_json(record.clone())),
                })
                .collect::<Vec<_>>();
            let (lineage_digest, semantic_digest) = conversation_chain_digests(&chain).unwrap();
            CodecLineage::Complete {
                chain,
                lineage_digest,
                semantic_digest,
            }
        };
        SessionAnalysis {
            source_provider_session_id: Some(self.session_id.clone()),
            source_cwd: Some(self.cwd.clone()),
            is_internal_subagent: false,
            record_count: self.records.len(),
            lineage,
        }
    }
}

fn read_session(path: &Path) -> Result<PiSession, String> {
    let (stamp, reader) = crate::jsonl_tail_io::open_head(path)?;
    if stamp.file_len() > MAX_BYTES as u64 {
        return Err("Pi POC byte limit".into());
    }
    let mut bytes = Vec::new();
    reader
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "read Pi POC session")?;
    stamp.verify(path)?;
    PiSession::parse(&bytes).map_err(str::to_owned)
}

fn bucket(cwd: &str) -> String {
    // Pi's encoding is lossy; the header, not this name, proves ownership.
    format!(
        "--{}--",
        cwd.strip_prefix(['/', '\\'])
            .unwrap_or(cwd)
            .replace(['/', '\\', ':'], "-")
    )
}

struct PiPocAdapter;

impl SessionAdapter for PiPocAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: key(),
            display_name: "Pi POC".into(),
            display_rank: 90,
            profile_label: "Pi agent home".into(),
            default_profile_directory: Some(".pi/agent".into()),
            setup_default: false,
            has_cli: false,
            maintenance_actions: Vec::new(),
        }
    }
    fn codec_key(&self) -> &str {
        "pi.poc.linear.v1"
    }
    fn codec_version(&self) -> u32 {
        1
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        let home = canonical_existing_dir(context.provider_home, "Pi POC home")?;
        let root = home.join("sessions");
        let mut result = SessionDiscovery::default();
        if !root.try_exists().map_err(|_| "inspect Pi sessions")? {
            return Ok(result);
        }
        // In particular do not let WalkDir follow a symlink supplied as its root.
        canonical_existing_dir(&root, "Pi POC sessions")?;
        let mut canonical_homes = BTreeMap::new();
        for (count, entry) in WalkDir::new(root)
            .follow_links(false)
            .max_depth(2)
            .into_iter()
            .enumerate()
        {
            if count >= MAX_SCAN_ENTRIES {
                return Err("Pi POC discovery entry limit".into());
            }
            let entry = entry.map_err(|_| "walk Pi sessions")?;
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }
            let session = match read_session(entry.path()) {
                Ok(session) => session,
                Err(_) => {
                    result
                        .warnings
                        .push("ignored invalid Pi POC session".into());
                    continue;
                }
            };
            if !session_home_is_project_root(
                context.project_root,
                Path::new(&session.cwd),
                &mut canonical_homes,
            ) {
                continue;
            }
            result.sessions.push(DiscoveredSession {
                display_name: session
                    .summary()
                    .title
                    .unwrap_or_else(|| session.session_id.clone()),
                session_id: session.session_id,
                physical_path: entry.path().to_owned(),
                blocked_reason: None,
            });
        }
        Ok(result)
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        let session = read_session(path)?;
        Ok(SessionFileMetadata {
            session_id: session.session_id,
            authorized_home: session.cwd.into(),
            is_internal_subagent: false,
            is_empty: session.records.len() == 1,
        })
    }
    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        match PiSession::parse(bytes) {
            Ok(session) => session.analysis(),
            Err(_) => SessionAnalysis {
                source_provider_session_id: None,
                source_cwd: None,
                is_internal_subagent: false,
                record_count: 0,
                lineage: CodecLineage::Unknown {
                    reason: CodecUnknownReason::MalformedJson,
                },
            },
        }
    }
    fn analyze_and_summarize(
        &self,
        bytes: &[u8],
    ) -> (SessionAnalysis, Result<SessionFileSummary, String>) {
        (self.analyze(bytes), self.summarize_bytes(bytes))
    }
    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String> {
        Ok(read_session(path)?.summary())
    }
    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String> {
        PiSession::parse(bytes)
            .map(|session| session.summary())
            .map_err(str::to_owned)
    }
    fn materialization_targets(
        &self,
        request: &MaterializationTargetRequest<'_>,
    ) -> Result<Vec<PlannedProviderTarget>, String> {
        if !safe_id(request.provider_session_id)
            || !request.sidecars.is_empty()
            || request.existing_transcript_path.is_some()
        {
            return Err("Pi POC accepts only new standalone session targets".into());
        }
        Ok(vec![PlannedProviderTarget {
            target_path: request
                .provider_home
                .join("sessions")
                .join(bucket(request.project_root))
                .join(format!("poc_{}.jsonl", request.provider_session_id)),
            role: ConversationFileRole::Transcript,
        }])
    }
    fn build_timeline_index(
        &self,
        _: &Path,
        _: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelineIndex, String> {
        Err(OUTSIDE_POC.into())
    }
    fn read_timeline_page(
        &self,
        _: &Path,
        _: &SessionTimelineIndex,
        _: &SessionTimelineQuery,
        _: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelinePageSlice, String> {
        Err(OUTSIDE_POC.into())
    }
    fn read_event_detail(
        &self,
        _: &Path,
        _: &SessionTimelineIndex,
        _: &str,
        _: &SessionTimelineContext<'_>,
    ) -> Result<SessionEventDetail, String> {
        Err(OUTSIDE_POC.into())
    }
    fn available_actions(&self, _: &SessionActionContext<'_>) -> Vec<SessionActionDescriptor> {
        Vec::new()
    }
    fn execute_action(
        &self,
        _: &mut SessionActionRequest<'_>,
    ) -> Result<SessionActionOutcome, String> {
        Err(OUTSIDE_POC.into())
    }
}

fn registry() -> AgentProviderRegistry {
    AgentProviderRegistry::from_adapters(vec![Arc::new(PiPocAdapter)]).unwrap()
}

fn request(root: &Path) -> CaptureRequest {
    CaptureRequest {
        project_root: root.join("source-project"),
        provider_homes: BTreeMap::from([(key(), root.join("source-agent"))]),
        include_project_content: false,
        excluded_content_roots: Vec::new(),
    }
}

fn jsonl(records: &[Value]) -> Vec<u8> {
    (records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n")
        .into_bytes()
}

fn fixture(root: &Path) -> Vec<Value> {
    vec![
        json!({"type":"session","version":3,"id":"pi-poc","timestamp":"2026-10-15T00:00:00Z","cwd":root.join("source-project")}),
        json!({"type":"message","id":"entry001","parentId":null,"message":{"role":"user","content":"hello"}}),
        json!({"type":"message","id":"entry002","parentId":"entry001","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}),
    ]
}

fn fixture_world() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for directory in [
        "source-project",
        "other-project",
        "source-agent/sessions/bucket",
        "destination-project",
        "destination-agent",
    ] {
        fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    temp
}

#[test]
fn pi_poc_registry_capture_and_exact_project_ownership() {
    let temp = fixture_world();
    let root = fs::canonicalize(temp.path()).unwrap();
    let records = fixture(&root);
    let bytes = jsonl(&records);
    fs::write(root.join("source-agent/sessions/bucket/poc.jsonl"), &bytes).unwrap();
    let mut foreign = records.clone();
    foreign[0]["id"] = json!("pi-foreign");
    foreign[0]["cwd"] = json!(root.join("other-project"));
    fs::write(
        root.join("source-agent/sessions/bucket/foreign.jsonl"),
        jsonl(&foreign),
    )
    .unwrap();
    // A project subfolder is also a different project, regardless of bucket.
    fs::create_dir(root.join("source-project/child")).unwrap();
    foreign[0]["cwd"] = json!(root.join("source-project/child"));
    fs::write(
        root.join("source-agent/sessions/bucket/child.jsonl"),
        jsonl(&foreign),
    )
    .unwrap();
    fs::write(root.join("source-agent/auth.json"), b"excluded fixture").unwrap();
    fs::write(root.join("source-agent/settings.json"), b"excluded fixture").unwrap();
    fs::create_dir(root.join("source-project/.pi")).unwrap();
    fs::write(
        root.join("source-project/.pi/settings.json"),
        b"excluded fixture",
    )
    .unwrap();
    let registry = registry();
    assert!(!AgentProviderRegistry::production().contains(&key()));
    let request = request(&root);
    let discovery = provider_capture::discover_full(&registry, &request).unwrap();
    let inventory = discovery.project_inventory();
    assert_eq!(inventory.resources.len(), 1);
    let selected = BTreeSet::from([inventory.resources[0].resource_id.clone()]);
    let captured =
        provider_capture::capture_from_discovery(&registry, &request, &discovery, &selected, true)
            .unwrap();
    assert_eq!(captured.files.len(), 1);
    assert_eq!(
        captured.files.values().next().unwrap().bytes.as_ref(),
        bytes
    );
    assert!(captured
        .conversations
        .values()
        .next()
        .unwrap()
        .lineage
        .is_complete());
}

#[test]
fn pi_poc_refuses_invalid_versions_parentage_and_bounds() {
    let temp = fixture_world();
    let valid = fixture(temp.path());
    for version in [1, 2, 4] {
        let mut records = valid.clone();
        records[0]["version"] = json!(version);
        assert!(PiSession::parse(&jsonl(&records)).is_err());
    }
    for bad_id in ["../escape", "a/b", "", ".hidden"] {
        let mut records = valid.clone();
        records[0]["id"] = json!(bad_id);
        assert!(PiSession::parse(&jsonl(&records)).is_err());
    }
    let mut records = valid.clone();
    records[2]["id"] = records[1]["id"].clone();
    assert!(PiSession::parse(&jsonl(&records)).is_err());
    let mut records = valid.clone();
    records[1]["parentId"] = json!("entry002");
    assert!(PiSession::parse(&jsonl(&records)).is_err());
    let mut records = valid.clone();
    records[1].as_object_mut().unwrap().remove("parentId");
    assert!(PiSession::parse(&jsonl(&records)).is_err());
    let mut records = valid.clone();
    records[0]["cwd"] = json!("relative/project");
    assert!(PiSession::parse(&jsonl(&records)).is_err());
    let bytes = jsonl(&valid);
    assert!(PiSession::parse(&bytes[..bytes.len() - 1]).is_err());
    assert!(PiSession::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    assert!(PiSession::parse(&b"{}\n".repeat(MAX_RECORDS + 1)).is_err());
    assert!(!PiPocAdapter.analyze(b"invalid\n").is_resumable());
}

#[test]
fn pi_poc_keeps_branches_out_of_linear_reconciliation() {
    let temp = fixture_world();
    let mut records = fixture(temp.path());
    records.push(json!({"type":"message","id":"entry003","parentId":"entry001","message":{"role":"user","content":"another branch"}}));
    let session = PiSession::parse(&jsonl(&records)).unwrap();
    assert_eq!(session.records.len(), 4);
    assert_eq!(
        session.analysis().lineage,
        CodecLineage::Unknown {
            reason: CodecUnknownReason::NonLinearParentage
        }
    );
    assert!(!session.analysis().is_resumable());
}

#[cfg(unix)]
#[test]
fn pi_poc_discovery_does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let temp = fixture_world();
    let root = fs::canonicalize(temp.path()).unwrap();
    let outside = root.join("outside.jsonl");
    fs::write(&outside, jsonl(&fixture(&root))).unwrap();
    symlink(
        &outside,
        root.join("source-agent/sessions/bucket/linked.jsonl"),
    )
    .unwrap();
    symlink(
        root.join("other-project"),
        root.join("source-agent/sessions/linked-directory"),
    )
    .unwrap();
    let context = SessionDiscoveryContext {
        provider_home: &root.join("source-agent"),
        project_root: &root.join("source-project"),
    };
    assert!(PiPocAdapter.discover(&context).unwrap().sessions.is_empty());
    assert!(PiPocAdapter
        .scan_metadata(&root.join("source-agent/sessions/bucket/linked.jsonl"))
        .is_err());
    fs::rename(
        root.join("source-agent/sessions"),
        root.join("saved-sessions"),
    )
    .unwrap();
    symlink(
        root.join("saved-sessions"),
        root.join("source-agent/sessions"),
    )
    .unwrap();
    assert!(PiPocAdapter.discover(&context).is_err());
}

/// Pure experiment: the native JSONL stays intact except project-path mapping.
/// Fork-file links need a selected-session identity map; refuse them here.
fn relocate(session: &PiSession, destination: &str) -> Result<Vec<u8>, String> {
    if session.records[0].get("parentSession").is_some() {
        return Err("Pi POC does not relocate parentSession file links".into());
    }
    let source = SessionPathMapper::new(&session.cwd, None, None)?;
    let target = SessionPathMapper::new(destination, None, None)?;
    let records = session
        .records
        .iter()
        .map(|record| {
            let wire = source.encode_record(record)?.wire;
            target.decode_record(&wire)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let bytes = jsonl(&records);
    PiSession::parse(&bytes).map_err(str::to_owned)?;
    Ok(bytes)
}

#[test]
fn pi_poc_relocation_refuses_linked_forks_and_unsafe_targets() {
    let temp = fixture_world();
    let mut records = fixture(temp.path());
    records[0]["parentSession"] = json!(temp.path().join("source-agent/parent.jsonl"));
    let session = PiSession::parse(&jsonl(&records)).unwrap();
    assert!(relocate(
        &session,
        temp.path().join("destination-project").to_str().unwrap()
    )
    .is_err());
    let request = MaterializationTargetRequest {
        provider_home: &temp.path().join("destination-agent"),
        project_root: temp.path().to_str().unwrap(),
        provider_session_id: "../escape",
        source_provider_session_id: None,
        existing_transcript_path: None,
        sidecars: &[],
        planned_at: 0,
    };
    assert!(PiPocAdapter.materialization_targets(&request).is_err());
    assert_eq!(bucket("/work/my project"), "--work-my project--");
    assert_eq!(bucket("C:\\work\\project"), "--C--work-project--");
}

#[test]
#[ignore = "requires synthetic fixtures from scripts/run-pi-harness-poc.mjs"]
fn pi_poc_native_sdk_roundtrip() {
    let root =
        PathBuf::from(std::env::var_os("MALLARD_PI_POC_ROOT").expect("run the Pi POC script"));
    let root = canonical_existing_dir(&root, "Pi POC fixture root").unwrap();
    assert_eq!(
        fs::read(root.join("poc-marker")).unwrap(),
        b"mallard-pi-synthetic-only-v1\n"
    );
    let registry = registry();
    let request = request(&root);
    let discovery = provider_capture::discover_full(&registry, &request).unwrap();
    let inventory = discovery.project_inventory();
    assert_eq!(inventory.resources.len(), 1);
    let selected = BTreeSet::from([inventory.resources[0].resource_id.clone()]);
    let capture =
        provider_capture::capture_from_discovery(&registry, &request, &discovery, &selected, true)
            .unwrap();
    assert_eq!(capture.files.len(), 1);
    let conversation = capture.conversations.values().next().unwrap();
    assert_eq!(conversation.provider, key());
    assert!(
        !conversation.lineage.is_complete(),
        "branched Pi log must not enter the experimental linear codec"
    );
    let bytes = capture.files.values().next().unwrap().bytes.as_ref();
    let session = PiSession::parse(bytes).unwrap();
    let destination = root.join("destination-project");
    let output = relocate(&session, destination.to_str().unwrap()).unwrap();
    let target = PiPocAdapter
        .materialization_targets(&MaterializationTargetRequest {
            provider_home: &root.join("destination-agent"),
            project_root: destination.to_str().unwrap(),
            provider_session_id: &session.session_id,
            source_provider_session_id: None,
            existing_transcript_path: None,
            sidecars: &[],
            planned_at: 0,
        })
        .unwrap()
        .remove(0)
        .target_path;
    // This test creates one new artifact only in the runner's disposable home.
    // Production Pull plans/apply are deliberately not bypassed or changed.
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let mut staged = tempfile::NamedTempFile::new_in(target.parent().unwrap()).unwrap();
    staged.write_all(&output).unwrap();
    staged.as_file().sync_all().unwrap();
    staged.persist_noclobber(&target).unwrap();
    let discovered = PiPocAdapter
        .discover(&SessionDiscoveryContext {
            provider_home: &root.join("destination-agent"),
            project_root: &destination,
        })
        .unwrap();
    assert_eq!(discovered.sessions.len(), 1);
    assert_eq!(discovered.sessions[0].session_id, session.session_id);
}
