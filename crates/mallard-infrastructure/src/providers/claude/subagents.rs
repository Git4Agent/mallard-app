//! Direct Claude children: one parent result plus an exact native file.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mallard_domain::{
    LocalConversationBinding, RootPhysicalMapping, SubagentContentState, SubagentKey,
    SubagentSelection,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::codec::{analyze_claude_records_in_context, ClaudeRecordContext};
use crate::claude_history_tail::{self, ClaudeTailRefresh};
use crate::conversation_codec::{CodecLineage, CodecUnknownReason};
use crate::jsonl_tail_io::{self, SourceStamp, SourceStamps};
use crate::providers::SessionAnalysis;

/// The key of each child the main refers to, under the child's agent ID.
pub type ChildLinks = BTreeMap<String, SubagentKey>;

/// Called from the required parent parse. Copied references are filtered here,
/// before any filesystem lookup. Resumes retain the first result's key.
pub fn record_link(links: &mut ChildLinks, record: &Value, parent: &str) -> Result<(), String> {
    if record.get("sessionId").and_then(Value::as_str) != Some(parent)
        || record.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || record.get("type").and_then(Value::as_str) != Some("user")
    {
        return Ok(());
    }
    let Some(agent) = record
        .get("toolUseResult")
        .and_then(|result| result.get("agentId"))
    else {
        return Ok(());
    };
    let id = agent.as_str().ok_or("Claude child agent ID is invalid")?;
    validate_id(id)?;
    let blocks = record
        .pointer("/message/content")
        .and_then(Value::as_array)
        .ok_or("Claude child result has no tool-result block")?;
    let mut results = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"));
    let creation = results
        .next()
        .and_then(|block| block.get("tool_use_id"))
        .and_then(Value::as_str)
        .ok_or("Claude child result has no creation ID")?;
    if results.next().is_some() {
        return Err("Claude child result has ambiguous tool-result blocks".into());
    }
    let key = SubagentKey::child(None, creation)?;
    if links
        .iter()
        .any(|(other, known)| other != id && *known == key)
    {
        return Err("Claude child key names multiple agent IDs".into());
    }
    if !links.contains_key(id) {
        if links.len() + 1 >= mallard_domain::MAX_CONVERSATION_FILES {
            return Err("Claude child count limit".into());
        }
        links.insert(id.to_string(), key);
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err("unsafe Claude native identity".into());
    }
    Ok(())
}

/// The identity charset a child file name is derived from.
pub(crate) fn valid_agent_id(id: &str) -> bool {
    validate_id(id).is_ok()
}

pub fn child_path(main: &Path, parent: &str, agent: &str) -> Result<PathBuf, String> {
    validate_id(parent)?;
    validate_id(agent)?;
    Ok(main
        .parent()
        .ok_or("Claude main has no directory")?
        .join(parent)
        .join("subagents")
        .join(format!("agent-{agent}.jsonl")))
}

/// Check lexical components before following any child path. A missing file
/// is recoverable; a symlink or inaccessible component is not.
pub fn child_exists(path: &Path, root: &Path) -> Result<bool, String> {
    let mut current = root.to_path_buf();
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Claude child escapes its parent")?;
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("inspect Claude child path: {error}")),
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || (current == path && !metadata.is_file())
                    || (current != path && !metadata.is_dir()) =>
            {
                return Err("unsafe Claude child path".into())
            }
            Ok(_) => {}
        }
    }
    Ok(true)
}

/// Cold membership scan when the existing main cursor can supply tail evidence.
pub fn collect_main_links(bytes: &[u8], parent: &str) -> Result<ChildLinks, String> {
    let mut links = ChildLinks::new();
    for raw in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !raw.iter().all(u8::is_ascii_whitespace) {
            let value = serde_json::from_slice::<Value>(raw)
                .map_err(|_| "Claude child discovery found malformed JSON")?;
            record_link(&mut links, &value, parent)?;
        }
    }
    Ok(links)
}

/// Analyze the main, collect links, and calculate its cursor in one parse.
/// Retain link errors even outside the final canonical chain.
pub fn analyze_main(
    bytes: &[u8],
    parent: &str,
    links: &mut ChildLinks,
) -> Result<(SessionAnalysis, (u64, u64)), String> {
    links.clear();
    let mut failure = None;
    let mut offset = 0;
    let mut watermark = claude_history_tail::ClaudeWatermark::default();
    let records = bytes.split_inclusive(|b| *b == b'\n').filter_map(|raw| {
        offset += raw.len() as u64;
        if raw.iter().all(u8::is_ascii_whitespace) {
            let _ = watermark.feed(None, offset, ClaudeRecordContext::Main);
            return None;
        }
        let parsed = serde_json::from_slice::<Value>(raw);
        if let Ok(value) = &parsed {
            if let Err(error) = watermark.feed(Some(value), offset, ClaudeRecordContext::Main) {
                failure = Some(error);
            }
            if let Err(error) = record_link(links, value, parent) {
                failure = Some(error);
            }
        }
        Some(parsed)
    });
    let analysis = analyze_claude_records_in_context(bytes, records, ClaudeRecordContext::Main);
    if let Some(error) = failure {
        return Err(error);
    }
    Ok((analysis, watermark.finish()))
}

pub fn refresh_main(
    layout: &mallard_domain::RootTailLayout,
    state: &mallard_domain::PortableConversationTail,
    mapping: &RootPhysicalMapping,
    window: &jsonl_tail_io::TailWindow,
    links: &mut ChildLinks,
) -> Result<ClaudeTailRefresh, String> {
    claude_history_tail::refresh_claude_tail_with_records(
        layout,
        state,
        mapping,
        window,
        ClaudeRecordContext::Main,
        &mut |record| record_link(links, record, &mapping.provider_thread_id),
    )
}

pub fn analyze_child(bytes: &[u8], parent: &str, agent: &str) -> SessionAnalysis {
    analyze_claude_records_in_context(
        bytes,
        crate::conversation_codec::parsed_records(bytes),
        ClaudeRecordContext::Child {
            session_id: parent,
            agent_id: agent,
        },
    )
}

pub fn content_state(analysis: &SessionAnalysis) -> Result<SubagentContentState, String> {
    let chain = analysis
        .lineage
        .chain()
        .ok_or("Claude child has invalid content or ownership")?;
    SubagentContentState::from_tail(
        &mallard_domain::compute_portable_conversation_tail_with_codec(
            mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_V1,
            mallard_domain::TailPartitionKind::WholeThread,
            0,
            chain,
        )?,
    )
}

fn unsupported(analysis: &SessionAnalysis) -> bool {
    matches!(
        analysis.lineage,
        CodecLineage::Unknown {
            reason: CodecUnknownReason::UnsupportedChildReference
        }
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MainObservation {
    pub mapping: RootPhysicalMapping,
    pub links: ChildLinks,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChildObservation {
    pub agent_id: String,
    pub mapping: RootPhysicalMapping,
    pub stamp: SourceStamp,
    pub state: SubagentContentState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeObservation {
    pub main: MainObservation,
    pub children: BTreeMap<SubagentKey, ChildObservation>,
    pub missing: BTreeSet<SubagentKey>,
    pub omitted: bool,
    /// Every child file this tree consulted, under the stamp it was read
    /// at. A missing child has no stamp and leaves the set unpinned.
    pub dependencies: SourceStamps,
}

impl TreeObservation {
    pub fn selection(&self) -> SubagentSelection {
        SubagentSelection {
            wire_children: mallard_domain::ConversationWireChildren::On,
            keys: self.children.keys().cloned().collect(),
            omitted: self.omitted,
            estimated_bytes: self
                .children
                .values()
                .map(|child| child.mapping.source_file_size)
                .sum(),
        }
    }
}

fn candidates(
    main: &MainObservation,
    binding: Option<&LocalConversationBinding>,
) -> Result<BTreeMap<SubagentKey, String>, String> {
    let mut candidates = BTreeMap::new();
    if let Some(binding) = binding {
        for (key, child) in &binding.installed_subagents {
            if key.owner().is_some() {
                return Err("Claude supports direct children only".into());
            }
            candidates.insert(key.clone(), child.provider_thread_id.clone());
        }
    }
    for (id, key) in &main.links {
        // A child that came with Pull is known by its ID: installed, or
        // referred to without its transcript.
        if binding.is_some_and(|binding| binding.installed_absent_subagents.contains(id))
            || candidates.values().any(|known| known == id)
        {
            continue;
        }
        if candidates.insert(key.clone(), id.clone()).is_some() {
            return Err("Claude installed key changed identity".into());
        }
    }
    if candidates.len() + 1 >= mallard_domain::MAX_CONVERSATION_FILES {
        return Err("Claude child count limit".into());
    }
    Ok(candidates)
}

/// The agent calls whose children came with a Pull without their
/// transcripts. A call names its child in its result, and the binding names
/// the children that came without a transcript.
pub fn agent_calls_without_transcript(
    index: &crate::providers::SessionTimelineIndex,
    binding: &LocalConversationBinding,
) -> BTreeSet<String> {
    index
        .events
        .iter()
        .filter(|entry| {
            entry
                .agent
                .as_ref()
                .and_then(|agent| agent.provider_agent_id.as_ref())
                .is_some_and(|id| binding.installed_absent_subagents.contains(id))
        })
        .map(|entry| entry.event_id.clone())
        .collect()
}

pub fn observe_tree(
    main: MainObservation,
    binding: Option<&LocalConversationBinding>,
    previous: Option<&TreeObservation>,
) -> Result<TreeObservation, String> {
    let members = candidates(&main, binding)?;
    let path = PathBuf::from(&main.mapping.root_path);
    let parent = main.mapping.provider_thread_id.clone();
    let mut tree = TreeObservation {
        dependencies: SourceStamps::default(),
        main,
        children: BTreeMap::new(),
        missing: BTreeSet::new(),
        omitted: false,
    };
    let mut total = tree.main.mapping.source_file_size;
    for (key, id) in members {
        let child_path = child_path(&path, &parent, &id)?;
        if !child_exists(&child_path, path.parent().unwrap())? {
            tree.dependencies.record_unstamped();
            tree.missing.insert(key);
            continue;
        }
        let previous = previous
            .and_then(|tree| tree.children.get(&key))
            .filter(|child| {
                child.agent_id == id && child.mapping.root_path == child_path.to_string_lossy()
            });
        let stamp = previous
            .map(|_| SourceStamp::read(&child_path))
            .transpose()?;
        let mut child = previous
            .filter(|child| Some(&child.stamp) == stamp.as_ref())
            .cloned();
        if child.is_none() {
            // The saved cursor serves only the same file, grown; any other
            // change reads the child in full.
            if let Some((previous, stamp)) = previous
                .zip(stamp.as_ref())
                .filter(|(child, stamp)| stamp.continues(&child.stamp.file_state()))
            {
                if let Ok(window) = jsonl_tail_io::read_tail_window(&child_path, &previous.mapping)
                {
                    let context = ClaudeRecordContext::Child {
                        session_id: &parent,
                        agent_id: &id,
                    };
                    match claude_history_tail::refresh_claude_tail_with_records(
                        &mallard_domain::RootTailLayout {
                            codec: mallard_domain::CLAUDE_PORTABLE_TAIL_CODEC_V1.into(),
                            partition_kind: mallard_domain::TailPartitionKind::WholeThread,
                            start_record_count: 0,
                        },
                        &previous.state.local_tail(),
                        &previous.mapping,
                        &window,
                        context,
                        &mut |_| Ok(()),
                    )? {
                        ClaudeTailRefresh::Extended {
                            observed_tail,
                            mapping,
                        } => {
                            stamp.verify(&child_path)?;
                            child = Some(ChildObservation {
                                agent_id: id.clone(),
                                mapping,
                                stamp: stamp.clone(),
                                state: SubagentContentState::from_tail(&observed_tail)?,
                            });
                        }
                        ClaudeTailRefresh::Unchanged => {
                            stamp.verify(&child_path)?;
                            let mut updated = previous.clone();
                            updated.stamp = stamp.clone();
                            updated.mapping.source_file_size = window.file_len;
                            child = Some(updated);
                        }
                        _ => {}
                    }
                }
            }
        }
        let child = match child {
            Some(child) => child,
            None => {
                let (bytes, stamp) = jsonl_tail_io::read_snapshot(&child_path)?;
                let complete = jsonl_tail_io::safe_record_end(&bytes) as usize;
                let context = ClaudeRecordContext::Child {
                    session_id: &parent,
                    agent_id: &id,
                };
                let mut watermark = claude_history_tail::ClaudeWatermark::default();
                let mut end = 0;
                let records = bytes[..complete]
                    .split_inclusive(|byte| *byte == b'\n')
                    .filter_map(|raw| {
                        end += raw.len() as u64;
                        let line = raw.strip_suffix(b"\n").unwrap_or(raw);
                        let line = line.strip_suffix(b"\r").unwrap_or(line);
                        if line.is_empty() {
                            let _ = watermark.feed(None, end, context);
                            return None;
                        }
                        let value = serde_json::from_slice::<Value>(line);
                        if let Ok(value) = &value {
                            let _ = watermark.feed(Some(value), end, context);
                        }
                        Some(value)
                    });
                let analysis =
                    analyze_claude_records_in_context(&bytes[..complete], records, context);
                if unsupported(&analysis) {
                    tree.dependencies.record(&child_path, &stamp);
                    tree.omitted = true;
                    tree.children.clear();
                    tree.missing.clear();
                    return Ok(tree);
                }
                let state = content_state(&analysis)?;
                let mut mapping = claude_history_tail::claude_root_mapping(
                    &child_path,
                    &bytes[..complete],
                    &analysis,
                    watermark.finish(),
                )?;
                mapping.source_file_size = bytes.len() as u64;
                ChildObservation {
                    agent_id: id,
                    mapping,
                    stamp,
                    state,
                }
            }
        };
        total = total
            .checked_add(child.mapping.source_file_size)
            .ok_or("Claude child byte overflow")?;
        if total > crate::provider_capture::MAX_CAPTURE_BYTES {
            return Err("Claude parent exceeds capture limit".into());
        }
        tree.dependencies.record(&child_path, &child.stamp);
        tree.children.insert(key, child);
    }
    Ok(tree)
}

pub struct EncodedChild {
    pub source_mtime: u64,
    pub bytes: crate::payload::PayloadBytes,
    pub accepted: mallard_domain::AcceptedSubagentState,
    pub adoption: Option<mallard_domain::AcceptedSubagentState>,
    pub origins: crate::session_path_origins::PathOrigins,
}

impl CapturedChildren {
    pub fn omitted() -> Self {
        Self {
            omitted: true,
            ..Self::default()
        }
    }
}

#[derive(Default)]
pub struct CapturedChildren {
    pub children: BTreeMap<SubagentKey, EncodedChild>,
    pub sources: crate::owned_subagents::SourceSnapshots,
    pub omitted: bool,
    omission: Option<(PathBuf, String, String, SourceStamp)>,
}

impl CapturedChildren {
    pub fn verify_scope(&self) -> Result<(), String> {
        if let Some((path, parent, agent, stamp)) = &self.omission {
            if stamp.verify(path).is_err() {
                let bytes = jsonl_tail_io::read_bounded(path)?;
                if !unsupported(&analyze_child(&bytes, parent, agent)) {
                    return Err(crate::owned_subagents::PUSH_CHANGED.into());
                }
            }
        }
        Ok(())
    }
}

/// Capture, analyze and encode each source with one read. Encoding shares the
/// source JSON parse with chain analysis; the emitted wire has its own fold.
pub fn capture_children(
    main: &MainObservation,
    binding: Option<&LocalConversationBinding>,
    mapper: &crate::session_path_map::SessionPathMapper,
    origins: &crate::session_path_origins::PathOrigins,
    redaction: &mut crate::credential_redaction::RedactionRun<'_>,
    probes: Option<&BTreeMap<SubagentKey, mallard_domain::SubagentTailPins>>,
) -> Result<CapturedChildren, String> {
    let path = Path::new(&main.mapping.root_path);
    let parent = &main.mapping.provider_thread_id;
    let mut captured = CapturedChildren::default();
    let mut total = main.mapping.source_file_size;
    for (key, id) in candidates(main, binding)? {
        let child_path = child_path(path, parent, &id)?;
        if !child_exists(&child_path, path.parent().unwrap())? {
            return Err("required Claude child file is missing".into());
        }
        let (bytes, stamp) = jsonl_tail_io::read_snapshot(&child_path)?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or("Claude child byte overflow")?;
        if total > crate::provider_capture::MAX_CAPTURE_BYTES {
            return Err("Claude parent exceeds capture limit".into());
        }
        if jsonl_tail_io::safe_record_end(&bytes) != bytes.len() as u64 {
            return Err(crate::owned_subagents::PUSH_CHANGED.into());
        }
        let mut output = String::with_capacity(bytes.len());
        let mut output_records = Vec::new();
        let mut output_origins = crate::session_path_origins::PathOrigins::default();
        let mut ordinal = 0;
        let mut failure = None;
        let records = crate::conversation_codec::parsed_records(&bytes).inspect(|record| {
            if let Ok(record) = record {
                match mapper.encode_record_with_origins(record, origins.record(&id, ordinal)) {
                    Ok(mapped) => {
                        output_origins.push(&id, mapped.origins);
                        match redaction.redact_record_gated(&mapped.wire).and_then(
                            |(wire, changed)| {
                                if changed {
                                    mapper.verify_wire_record(&wire)?;
                                }
                                let line = serde_json::to_string(&wire)
                                    .map_err(|error| error.to_string())?;
                                Ok((line, wire))
                            },
                        ) {
                            Ok((line, wire)) => {
                                output.push_str(&line);
                                output.push('\n');
                                output_records.push(wire);
                            }
                            Err(error) => failure = Some(error.to_string()),
                        }
                    }
                    Err(error) => failure = Some(error.message),
                }
                ordinal += 1;
            }
        });
        let analysis = analyze_claude_records_in_context(
            &bytes,
            records,
            ClaudeRecordContext::Child {
                session_id: parent,
                agent_id: &id,
            },
        );
        if unsupported(&analysis) {
            captured.children.clear();
            captured.sources.clear();
            captured.omitted = true;
            captured.omission = Some((child_path, parent.clone(), id, stamp));
            return Ok(captured);
        }
        let local = content_state(&analysis)?;
        if let Some(error) = failure {
            return Err(error);
        }
        origins.verify_source_length(&id, ordinal, false)?;
        if output.len() as u64 > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
            return Err("encoded Claude child exceeds size limit".into());
        }
        let wire: crate::payload::PayloadBytes = output.into_bytes().into();
        let wire_analysis = analyze_claude_records_in_context(
            wire.as_bytes(),
            output_records.into_iter().map(Ok),
            ClaudeRecordContext::Child {
                session_id: parent,
                agent_id: &id,
            },
        );
        let normalized = content_state(&wire_analysis)?;
        let adoption = probes
            .and_then(|probes| probes.get(&key))
            .filter(|pins| pins.accepted.is_none())
            .and_then(|pins| pins.cloud.as_ref())
            .and_then(|cloud| {
                crate::owned_subagents::adopt_chain_prefix(
                    analysis.lineage.chain()?,
                    wire_analysis.lineage.chain()?,
                    cloud,
                )
            });
        let accepted = mallard_domain::AcceptedSubagentState { local, normalized };
        accepted.validate()?;
        let source_mtime = stamp.mtime_secs();
        captured.sources.insert(child_path, (bytes.into(), stamp));
        captured.children.insert(
            key,
            EncodedChild {
                source_mtime,
                bytes: wire,
                accepted,
                adoption,
                origins: output_origins,
            },
        );
    }
    Ok(captured)
}

/// Validate declared ownership and normalized content, then decode once.
/// Native agent IDs remain scoped to the destination parent directory.
pub fn prepare_child_payloads(
    links: &ChildLinks,
    children: &BTreeMap<SubagentKey, (Vec<u8>, SubagentContentState)>,
    source_parent: &str,
    destination_parent: &str,
    root: &str,
    canonical_root: Option<&str>,
    redaction: &mut crate::credential_redaction::RedactionRun<'_>,
) -> Result<Vec<crate::owned_subagents::InstalledChildPayload>, String> {
    let mapper = crate::session_path_map::SessionPathMapper::new(
        root,
        canonical_root,
        crate::session_path_map::home_dir_text().as_deref(),
    )?;
    let mut result = Vec::with_capacity(children.len());
    for (key, (bytes, normalized)) in children {
        if key.owner().is_some() {
            return Err("Claude supports direct children only".into());
        }
        let agent = links
            .iter()
            .find(|(_, known)| *known == key)
            .map(|(agent, _)| agent)
            .ok_or("listed Claude child has no parent tool-result link")?;
        if jsonl_tail_io::safe_record_end(bytes) != bytes.len() as u64 {
            return Err("cloud Claude child has an incomplete record".into());
        }
        let mut output = String::with_capacity(bytes.len());
        let mut origins = crate::session_path_origins::PathOrigins::default();
        let mut failure = None;
        let records = crate::conversation_codec::parsed_records(bytes).inspect(|record| {
            if let Ok(record) = record {
                if let Err(error) = redaction.verify_wire_value(record) {
                    failure = Some(error);
                }
                let mut local = record.clone();
                // The child context independently verifies every source sessionId.
                if let Some(id) = local.get_mut("sessionId") {
                    *id = Value::String(destination_parent.into());
                }
                match mapper.decode_record_with_origins(&local) {
                    Ok(decoded) => {
                        origins.push(agent, decoded.origins);
                        match serde_json::to_string(&decoded.local) {
                            Ok(line) => {
                                output.push_str(&line);
                                output.push('\n');
                            }
                            Err(error) => failure = Some(error.to_string()),
                        }
                    }
                    Err(error) => failure = Some(error),
                }
            }
        });
        let wire = analyze_claude_records_in_context(
            bytes,
            records,
            ClaudeRecordContext::Child {
                session_id: source_parent,
                agent_id: agent,
            },
        );
        if &content_state(&wire)? != normalized {
            return Err("Claude child normalized state differs from manifest".into());
        }
        if let Some(error) = failure {
            return Err(error);
        }
        if output.len() as u64 > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
            return Err("restored Claude child exceeds size limit".into());
        }
        let output: crate::payload::PayloadBytes = output.into_bytes().into();
        let analysis = analyze_child(&output, destination_parent, agent);
        let accepted = mallard_domain::AcceptedSubagentState {
            local: content_state(&analysis)?,
            normalized: normalized.clone(),
        };
        accepted.validate()?;
        let transcript =
            crate::verified_transcript::VerifiedTranscript::from_claude_child_analysis(
                output, &analysis, agent,
            )?;
        result.push(crate::owned_subagents::InstalledChildPayload {
            key: key.clone(),
            provider_thread_id: agent.clone(),
            parent_provider_thread_id: destination_parent.into(),
            transcript,
            accepted,
            origins,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(id: &str, previous: Option<&str>) -> Value {
        json!({"type":"user","uuid":id,"parentUuid":previous,"sessionId":"parent","agentId":"agent",
            "isSidechain":true,"cwd":"/fixture/project","message":{"role":"user","content":"hello"}})
    }
    fn bytes(records: &[Value]) -> Vec<u8> {
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>()
            .into_bytes()
    }
    #[test]
    fn identity_is_checked_before_projection_and_main_excludes_child() {
        let base = record("u1", None);
        assert!(analyze_child(&bytes(&[base.clone()]), "parent", "agent")
            .lineage
            .chain()
            .is_some());
        assert!(
            super::super::codec::analyze_claude_transcript(&bytes(&[base.clone()]))
                .lineage
                .chain()
                .is_none()
        );
        for (field, value) in [
            ("agentId", json!("other")),
            ("sessionId", json!("other")),
            ("isSidechain", json!(false)),
        ] {
            let mut hidden = record("thinking", Some("u1"));
            hidden["type"] = json!("assistant");
            hidden["message"] =
                json!({"role":"assistant","content":[{"type":"thinking","thinking":"private"}]});
            hidden[field] = value;
            let analysis = analyze_child(&bytes(&[base.clone(), hidden]), "parent", "agent");
            assert!(matches!(
                analysis.lineage,
                CodecLineage::Unknown {
                    reason: CodecUnknownReason::InvalidChildIdentity
                }
            ));
        }
    }
    #[test]
    fn result_link_filters_inherited_and_keeps_first_async_key() {
        let result = |parent: &str, key: &str| {
            json!({"type":"user","sessionId":parent,
            "toolUseResult":{"agentId":"agent","isAsync":true,"outputFile":"/absent/fixture.output"},
            "message":{"content":[{"type":"tool_result","tool_use_id":key}]}})
        };
        let mut links = ChildLinks::new();
        record_link(&mut links, &result("ancestor", "old"), "parent").unwrap();
        assert!(links.is_empty());
        record_link(&mut links, &result("parent", "first"), "parent").unwrap();
        record_link(&mut links, &result("parent", "resumed"), "parent").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links["agent"].as_str(), "first");
        let mut conflict = result("parent", "first");
        conflict["toolUseResult"]["agentId"] = json!("different");
        assert!(record_link(&mut links, &conflict, "parent").is_err());
    }
    #[test]
    fn nested_reference_is_unsupported_and_unsafe_path_is_not_missing() {
        let mut nested = record("u1", None);
        nested["toolUseResult"] = json!({"agentId":"nested"});
        assert!(unsupported(&analyze_child(
            &bytes(&[nested]),
            "parent",
            "agent"
        )));
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("parent.jsonl");
        let path = child_path(&main, "parent", "agent").unwrap();
        assert!(!child_exists(&path, temp.path()).unwrap());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temp.path(), temp.path().join("parent")).unwrap();
            assert!(child_exists(&path, temp.path()).is_err());
        }
        assert!(child_path(&main, "parent", "../escape").is_err());
    }
    // A child that came with a Pull is known by its ID, wherever its
    // reference is in the file. The file here is shorter than the one Pull
    // wrote, as it is after a desktop import.
    #[test]
    fn a_child_that_came_with_a_pull_is_known_by_its_id() {
        let result = |uuid: &str, previous: &str, agent: &str| {
            json!({"type":"user","uuid":uuid,"parentUuid":previous,"sessionId":"parent",
            "cwd":"/fixture/project","isSidechain":false,"toolUseResult":{"agentId":agent},
            "message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":format!("call-{agent}"),"content":"done"}
            ]}})
        };
        let mut first = record("u1", None);
        first["isSidechain"] = json!(false);
        first.as_object_mut().unwrap().remove("agentId");
        let raw = bytes(&[
            first,
            result("r1", "u1", "installed"),
            result("r2", "r1", "absent"),
            result("r3", "r2", "made-here"),
        ]);
        let mut links = ChildLinks::new();
        let (analysis, watermark) = analyze_main(&raw, "parent", &mut links).unwrap();
        let mapping = claude_history_tail::seed_claude_root_with_watermark(
            Path::new("parent.jsonl"),
            &raw,
            &analysis,
            watermark,
        )
        .unwrap()
        .unwrap()
        .mapping;
        let main = MainObservation { mapping, links };
        let child = |agent: &str| {
            (
                SubagentKey::child(None, &format!("call-{agent}")).unwrap(),
                agent.to_string(),
            )
        };
        assert_eq!(
            candidates(&main, None).unwrap(),
            BTreeMap::from([child("absent"), child("installed"), child("made-here")])
        );

        let binding = LocalConversationBinding {
            installed_end: Some(raw.len() as u64 * 2),
            installed_absent_subagents: BTreeSet::from(["absent".to_string()]),
            pulled_children_excluded: false,
            installed_subagents: BTreeMap::from([(
                child("installed").0,
                mallard_domain::InstalledSubagent {
                    provider_thread_id: "installed".into(),
                    installed_end: 1,
                },
            )]),
            path_origins_ref: None,
            host_id: mallard_domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
            replica_id: mallard_domain::ReplicaId::parse("replica-a").unwrap(),
            local_project_id: mallard_domain::LocalProjectId::parse("project-a").unwrap(),
            provider_profile_id: mallard_domain::LocalProviderProfileId::parse("profile-a")
                .unwrap(),
            thread_id: mallard_domain::ThreadId::parse("thread-1").unwrap(),
            cloud_thread_id: None,
            forked_from: None,
            preserved: None,
            root_tail_layout: None,
            provider_thread_id: Some("parent".into()),
            materialization_state: mallard_domain::ConversationMaterializationState::Resumable,
            revision: 0,
            created_at: 1,
            updated_at: 1,
        };
        binding.validate().unwrap();
        assert_eq!(
            candidates(&main, Some(&binding)).unwrap(),
            BTreeMap::from([child("installed"), child("made-here")])
        );
    }
    #[test]
    fn fused_main_cursor_retains_pending_thinking_boundary() {
        let mut user = record("u1", None);
        user["isSidechain"] = json!(false);
        user.as_object_mut().unwrap().remove("agentId");
        let mut thinking = user.clone();
        thinking["uuid"] = json!("t1");
        thinking["parentUuid"] = json!("u1");
        thinking["type"] = json!("assistant");
        thinking["message"] =
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"pending"}]});
        let mut following = user.clone();
        following["uuid"] = json!("u2");
        following["parentUuid"] = json!("t1");
        for records in [
            vec![user.clone(), thinking.clone()],
            vec![user, thinking, following],
        ] {
            let raw = bytes(&records);
            let (analysis, watermark) =
                analyze_main(&raw, "parent", &mut ChildLinks::new()).unwrap();
            let path = Path::new("parent.jsonl");
            let fused = claude_history_tail::seed_claude_root_with_watermark(
                path, &raw, &analysis, watermark,
            )
            .unwrap()
            .unwrap();
            let independent = claude_history_tail::seed_claude_root(path, &raw, &analysis)
                .unwrap()
                .unwrap();
            assert_eq!(fused.mapping, independent.mapping);
            assert_eq!(fused.observed_tail, independent.observed_tail);
        }
    }
    #[test]
    fn child_incremental_matches_full_with_thinking_and_reparent_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-agent.jsonl");
        let initial = bytes(&[record("u1", None)]);
        std::fs::write(&path, &initial).unwrap();
        let context = ClaudeRecordContext::Child {
            session_id: "parent",
            agent_id: "agent",
        };
        let analysis = analyze_child(&initial, "parent", "agent");
        let seed =
            claude_history_tail::seed_claude_root_in_context(&path, &initial, &analysis, context)
                .unwrap()
                .unwrap();
        let mut thinking = record("t1", Some("u1"));
        thinking["type"] = json!("assistant");
        thinking["message"] =
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"temporary"}]});
        let all = bytes(&[record("u1", None), thinking, record("u2", Some("t1"))]);
        std::fs::write(&path, &all).unwrap();
        let window = jsonl_tail_io::read_tail_window(&path, &seed.mapping).unwrap();
        let result = claude_history_tail::refresh_claude_tail_with_records(
            &seed.layout,
            &seed.observed_tail,
            &seed.mapping,
            &window,
            context,
            &mut |_| Ok(()),
        )
        .unwrap();
        let ClaudeTailRefresh::Extended { observed_tail, .. } = result else {
            panic!("expected incremental append")
        };
        assert_eq!(
            SubagentContentState::from_tail(&observed_tail).unwrap(),
            content_state(&analyze_child(&all, "parent", "agent")).unwrap()
        );
        let all = bytes(&[record("u1", None), record("branch", None)]);
        std::fs::write(&path, &all).unwrap();
        let window = jsonl_tail_io::read_tail_window(&path, &seed.mapping).unwrap();
        let result = claude_history_tail::refresh_claude_tail_with_records(
            &seed.layout,
            &seed.observed_tail,
            &seed.mapping,
            &window,
            context,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert!(matches!(result, ClaudeTailRefresh::NeedsFullResolve { .. }));
    }

    #[test]
    fn a_call_is_left_out_by_the_id_of_the_child_it_names() {
        use crate::owned_subagents::agent_card_fixtures::{
            binding_of_a_pulled_session, index_of_agent_calls,
        };
        // The places say nothing: the desktop app may rewrite the file.
        let index = index_of_agent_calls(
            &[],
            &[
                ("call-installed", Some("installed"), 10),
                ("call-absent", Some("absent"), 900),
                ("call-resumed", Some("absent"), 20),
                ("call-made-here", Some("made-here"), 30),
                ("call-unanswered", None, 40),
            ],
        );
        let binding =
            binding_of_a_pulled_session(Some(100), &[("call-installed", "installed")], &["absent"]);
        binding.validate().unwrap();
        assert_eq!(
            agent_calls_without_transcript(&index, &binding),
            BTreeSet::from(["call-absent".to_string(), "call-resumed".to_string()])
        );
        // A session that was made on this computer names no such child.
        let native = binding_of_a_pulled_session(None, &[], &[]);
        assert!(agent_calls_without_transcript(&index, &native).is_empty());
    }
}
