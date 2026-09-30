//! Native Codex child ownership and history validation.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use mallard_domain::{
    compute_portable_conversation_tail, SubagentContentState, SubagentKey, TailPartitionKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codex_rollout::{authoritative_session_meta_with_record, MAX_CODEX_RECORDS};
use crate::conversation_codec::parsed_records;
use crate::jsonl_tail_io::{open_no_follow, safe_record_end};
use crate::owned_subagents::InstalledChildPayload;

const MAX_METADATA_BYTES: u64 = 1024 * 1024;

use crate::owned_subagents::PUSH_CHANGED;

pub const PORTABLE_MAIN_ID: &str = "mallard-main";
pub fn portable_child_id(key: &SubagentKey) -> String {
    format!("mallard-child:{key}")
}

pub type CreationRecords = BTreeMap<String, (String, u64, Value)>;

pub fn record_creation(
    records: &mut CreationRecords,
    record: &Value,
    offset: u64,
) -> Result<(), String> {
    let Some(activity) = SubagentActivity::from_record(record)? else {
        return Ok(());
    };
    if activity.kind != SubagentActivityKind::Started {
        return Ok(());
    }
    if let Some((creation, _, path)) = records.get(&activity.provider_thread_id) {
        if creation != &activity.event_id || path != &activity.agent_path {
            return Err("child has conflicting creation IDs".into());
        }
    } else {
        if records.values().any(|(id, _, _)| id == &activity.event_id) {
            return Err("creation ID names multiple children".into());
        }
        records.insert(
            activity.provider_thread_id,
            (activity.event_id, offset, activity.agent_path),
        );
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryObservation {
    pub mapping: mallard_domain::RootPhysicalMapping,
    pub stamp: crate::jsonl_tail_io::SourceStamp,
    pub creations: CreationRecords,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChildObservation {
    pub discovery: DiscoveryObservation,
    pub metadata: ChildMetadata,
    pub state: SubagentContentState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeObservation {
    pub main: DiscoveryObservation,
    pub children: BTreeMap<SubagentKey, ChildObservation>,
    pub missing: BTreeSet<SubagentKey>,
    pub omitted: bool,
    /// Files read for their header alone, under the stamp each was read at.
    pub header_dependencies: crate::jsonl_tail_io::SourceStamps,
}

impl TreeObservation {
    pub fn selection(&self) -> mallard_domain::SubagentSelection {
        mallard_domain::SubagentSelection {
            wire_children: mallard_domain::ConversationWireChildren::On,
            omitted: self.omitted,
            keys: self.children.keys().cloned().collect(),
            estimated_bytes: self
                .children
                .values()
                .map(|child| child.discovery.mapping.source_file_size)
                .sum(),
        }
    }
    /// Every file this tree was read from, under its read-time stamp.
    pub fn dependencies(&self) -> crate::jsonl_tail_io::SourceStamps {
        let mut stamps = crate::jsonl_tail_io::SourceStamps::of(
            Path::new(&self.main.mapping.root_path),
            &self.main.stamp,
        );
        for child in self.children.values() {
            stamps.record(
                Path::new(&child.discovery.mapping.root_path),
                &child.discovery.stamp,
            );
        }
        stamps.extend(&self.header_dependencies);
        stamps
    }
}

fn filtered_creations(
    discovery: &DiscoveryObservation,
    cutoff: Option<u64>,
) -> Result<BTreeMap<String, String>, String> {
    if cutoff.is_some_and(|cutoff| cutoff > discovery.mapping.consumed_byte_offset) {
        return Err("installed Codex cutoff exceeds the source".into());
    }
    if let Some(cutoff) = cutoff {
        verify_cutoff(Path::new(&discovery.mapping.root_path), cutoff)?;
    }
    Ok(discovery
        .creations
        .iter()
        .filter(|(_, (_, offset, _))| *offset >= cutoff.unwrap_or(0))
        .map(|(id, (creation, _, _))| (id.clone(), creation.clone()))
        .collect())
}

/// The agent calls whose children came with a Pull without their
/// transcripts. A rollout is only appended to: a call before the installed
/// end came with the Pull, and its child is here when the Pull installed it.
pub fn agent_calls_without_transcript(
    index: &crate::providers::SessionTimelineIndex,
    binding: &mallard_domain::LocalConversationBinding,
) -> BTreeSet<String> {
    let Some(cutoff) = binding.installed_end else {
        return BTreeSet::new();
    };
    // The index counts its places from the first retained source. The
    // cutoff is a place in the current rollout, which is the last one.
    let current = index.sources.split_last().map_or(0, |(_, earlier)| {
        earlier.iter().map(|source| source.length).sum::<u64>()
    });
    index
        .events
        .iter()
        .filter(|entry| {
            entry
                .agent
                .as_ref()
                .and_then(|agent| agent.provider_agent_id.as_ref())
                .is_some_and(|id| {
                    entry
                        .source
                        .offset
                        .checked_sub(current)
                        .is_some_and(|place| place < cutoff)
                        && !binding
                            .installed_subagents
                            .values()
                            .any(|child| &child.provider_thread_id == id)
                })
        })
        .map(|entry| entry.event_id.clone())
        .collect()
}

fn verify_cutoff(path: &Path, cutoff: u64) -> Result<(), String> {
    if cutoff == 0 {
        return Err("invalid installed Codex cutoff".into());
    }
    let mut file = open_no_follow(path)?;
    file.seek(SeekFrom::Start(cutoff - 1))
        .map_err(|error| error.to_string())?;
    let mut byte = [0];
    file.read_exact(&mut byte)
        .map_err(|_| "installed Codex cutoff exceeds the source")?;
    if byte != [b'\n'] {
        return Err("installed Codex cutoff is not a record boundary".into());
    }
    Ok(())
}

fn resolved_boundary(
    metadata: &ChildMetadata,
    resolved: &crate::codex_history_graph::ResolvedCodexRoot,
) -> bool {
    metadata.has_supported_header()
        && metadata.history_start_ordinal.is_some_and(|start| {
            resolved
                .segments
                .iter()
                .map(|segment| segment.consumed_record_count)
                .sum::<u64>()
                >= start
        })
}

// Merge installed identities and new references before reading metadata. The
// bool marks an installed child: foreign references can be skipped, while an
// installed identity must still prove ownership.
fn child_candidates(
    installed: Option<&BTreeMap<SubagentKey, mallard_domain::InstalledSubagent>>,
    owner: Option<&SubagentKey>,
    references: BTreeMap<String, String>,
) -> Result<BTreeMap<SubagentKey, (String, bool)>, String> {
    let mut candidates: BTreeMap<_, _> = installed
        .into_iter()
        .flat_map(|map| map.iter())
        .filter(|(key, _)| key.owner().as_ref() == owner)
        .map(|(key, child)| (key.clone(), (child.provider_thread_id.clone(), true)))
        .collect();
    for (id, creation) in references {
        let key = SubagentKey::child(owner, &creation)?;
        match candidates.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get().0 != id {
                    return Err("installed child key conflicts".into());
                }
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((id, false));
            }
        }
    }
    Ok(candidates)
}

/// Cache data is only a starting cursor. Each changed source must prove its
/// identity and append. An unchanged sibling needs no transcript body read.
pub fn observe_tree(
    main: DiscoveryObservation,
    binding: Option<&mallard_domain::LocalConversationBinding>,
    lookup: &RolloutLookup,
    previous: Option<&TreeObservation>,
) -> Result<TreeObservation, String> {
    let installed = binding.map(|binding| &binding.installed_subagents);
    let mut tree = TreeObservation {
        main,
        children: BTreeMap::new(),
        missing: BTreeSet::new(),
        omitted: false,
        header_dependencies: Default::default(),
    };
    let mut pending = vec![(
        None,
        tree.main.mapping.provider_thread_id.clone(),
        filtered_creations(
            &tree.main,
            binding.and_then(|binding| binding.installed_end),
        )?,
    )];
    let mut identities = BTreeSet::from([tree.main.mapping.provider_thread_id.clone()]);
    let mut total = tree.main.mapping.source_file_size;
    while let Some((owner_key, owner_id, refs)) = pending.pop() {
        let candidates = child_candidates(installed, owner_key.as_ref(), refs)?;
        for (key, (id, required)) in candidates {
            let previous = previous
                .and_then(|tree| tree.children.get(&key))
                .filter(|child| child.metadata.provider_thread_id == id);
            let path = match lookup.find_current(&id)? {
                Some(path) => path,
                None => {
                    tree.missing.insert(key.clone());
                    pending.push((Some(key), id, BTreeMap::new()));
                    continue;
                }
            };
            let previous =
                previous.filter(|child| Path::new(&child.discovery.mapping.root_path) == path);
            if std::fs::symlink_metadata(&path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                // Listed, then gone: there is no file state to pin.
                tree.header_dependencies.record_unstamped();
                tree.missing.insert(key.clone());
                pending.push((Some(key), id, BTreeMap::new()));
                continue;
            }
            let stamp = crate::jsonl_tail_io::SourceStamp::read(&path)?;
            let cached = previous.filter(|child| child.discovery.stamp == stamp);
            let metadata = match cached {
                Some(child) => child.metadata.clone(),
                None => read_child_metadata(&path, &id)?,
            };
            if metadata.spawning_parent_id.as_deref() != Some(&owner_id) {
                if required || previous.is_some() {
                    return Err("child ownership conflicts with the selected tree".into());
                }
                tree.header_dependencies.record(&path, &stamp);
                continue;
            }
            if identities.len() >= mallard_domain::MAX_CONVERSATION_FILES
                || !identities.insert(id.clone())
            {
                return Err("Codex child ownership cycle or count limit".into());
            }
            if !metadata.has_supported_header() {
                tree.header_dependencies.record(&path, &stamp);
                tree.omitted = true;
                tree.children.clear();
                tree.missing.clear();
                return Ok(tree);
            }
            let child = if let Some(child) = cached {
                child.clone()
            } else {
                // The saved cursor serves only the same file, grown; any
                // other change reads the child in full.
                let extended = previous
                    .filter(|child| {
                        child.metadata == metadata
                            && stamp.continues(&child.discovery.stamp.file_state())
                    })
                    .map(|child| -> Result<Option<ChildObservation>, String> {
                        let window = crate::jsonl_tail_io::read_tail_window(
                            &path,
                            &child.discovery.mapping,
                        )?;
                        let mut creations = child.discovery.creations.clone();
                        let refreshed =
                            crate::codex_history_graph::refresh_codex_tail_with_records(
                                &whole_child_layout(),
                                &native_child_tail(&child.state, false),
                                &child.discovery.mapping,
                                &window,
                                &mut |record, offset| {
                                    record_creation(&mut creations, record, offset)
                                },
                            )?;
                        stamp.verify(&path)?;
                        match refreshed {
                            crate::codex_history_graph::CodexTailRefresh::Unchanged => {
                                let mut updated = child.clone();
                                updated.discovery.stamp = stamp.clone();
                                updated.discovery.mapping.source_file_size = window.file_len;
                                Ok(Some(updated))
                            }
                            crate::codex_history_graph::CodexTailRefresh::Extended {
                                observed_tail,
                                mapping,
                            } => {
                                let discovery = DiscoveryObservation {
                                    mapping,
                                    stamp: stamp.clone(),
                                    creations,
                                };
                                Ok(Some(ChildObservation {
                                    discovery,
                                    metadata: metadata.clone(),
                                    state: SubagentContentState::from_tail(&observed_tail)?,
                                }))
                            }
                            crate::codex_history_graph::CodexTailRefresh::NeedsFullResolve => {
                                Ok(None)
                            }
                            crate::codex_history_graph::CodexTailRefresh::Invalid { .. } => {
                                Err("child source was truncated or rewritten".into())
                            }
                        }
                    })
                    .transpose()?
                    .flatten();
                match extended {
                    Some(child) => child,
                    None => {
                        let (bytes, stamp) = crate::jsonl_tail_io::read_snapshot(&path)?;
                        if ChildMetadata::from_bytes(&bytes, &id)? != metadata {
                            return Err("child metadata changed during observation".into());
                        }
                        let bytes: crate::payload::PayloadBytes = bytes.into();
                        let (resolved, _) =
                            crate::codex_history_graph::resolve_captured_codex_root(
                                &path,
                                bytes.clone(),
                                stamp.clone(),
                                &|id| lookup.find_rollout(id),
                            )?;
                        if !resolved_boundary(&metadata, &resolved) {
                            tree.header_dependencies.record(&path, &stamp);
                            tree.omitted = true;
                            tree.children.clear();
                            tree.missing.clear();
                            return Ok(tree);
                        }
                        let seeded = crate::codex_history_graph::seed_codex_root(
                            &resolved,
                            &bytes,
                            Some(&whole_child_layout()),
                        )?;
                        ChildObservation {
                            discovery: DiscoveryObservation {
                                mapping: seeded.mapping,
                                stamp,
                                creations: resolved.leaf_creations,
                            },
                            metadata,
                            state: SubagentContentState::from_tail(&seeded.observed_tail)?,
                        }
                    }
                }
            };
            total = total
                .checked_add(child.discovery.mapping.source_file_size)
                .ok_or("Codex tree size overflow")?;
            if total > crate::provider_capture::MAX_CAPTURE_BYTES {
                return Err("Codex tree exceeds capture limit".into());
            }
            let cutoff = installed
                .and_then(|map| map.get(&key))
                .map(|child| child.installed_end);
            pending.push((
                Some(key.clone()),
                id,
                filtered_creations(&child.discovery, cutoff)?,
            ));
            tree.children.insert(key, child);
        }
    }
    Ok(tree)
}

/// Only native structural identity fields are mapped. Conversation text and
/// activity creation IDs retain their exact values.
pub fn rewrite_references(record: &mut Value, identities: &BTreeMap<String, String>) {
    fn replace(value: &mut Value, identities: &BTreeMap<String, String>) {
        match value {
            Value::String(id) => {
                if let Some(mapped) = identities.get(id) {
                    *id = mapped.clone();
                }
            }
            Value::Array(values) => {
                for value in values {
                    replace(value, identities);
                }
            }
            _ => {}
        }
    }
    fn fields(value: &mut Value, identities: &BTreeMap<String, String>) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if matches!(
                        key.as_str(),
                        "agent_thread_id"
                            | "parent_thread_id"
                            | "thread_id"
                            | "agent_id"
                            | "agent_ids"
                            | "receiver_thread_ids"
                            | "sender_thread_id"
                            | "target"
                    ) {
                        replace(value, identities);
                    } else if value.is_object() || value.is_array() {
                        fields(value, identities);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    fields(value, identities);
                }
            }
            _ => {}
        }
    }
    match record["type"].as_str() {
        Some("session_meta" | "event_msg") => fields(&mut record["payload"], identities),
        Some("response_item") => {
            let payload = &mut record["payload"];
            let is_agent_call = payload["name"].as_str().is_some_and(|name| {
                matches!(
                    name.rsplit('.').next(),
                    Some(
                        "spawn_agent"
                            | "send_message"
                            | "send_message_to_agent"
                            | "wait_agent"
                            | "wait"
                            | "close_agent"
                            | "resume_agent"
                            | "interrupt_agent"
                            | "followup_task"
                    )
                )
            });
            if is_agent_call {
                if let Some(arguments) = payload["arguments"].as_str() {
                    if let Ok(mut parsed) = serde_json::from_str::<Value>(arguments) {
                        fields(&mut parsed, identities);
                        payload["arguments"] = Value::String(parsed.to_string());
                    }
                }
            }
            // Structured tool results can name agents. Plain text stays intact.
            if payload["type"] == "function_call_output" {
                if let Some(output) = payload["output"].as_str() {
                    if let Ok(mut parsed) = serde_json::from_str::<Value>(output) {
                        fields(&mut parsed, identities);
                        payload["output"] = Value::String(parsed.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

/// One bounded filename inventory per operation. No unrelated body reads.
/// One bounded provider inventory. Current selections follow native filename order; history references always name physical rollout IDs.
pub struct RolloutLookup {
    home: PathBuf,
    paths: std::sync::OnceLock<Result<Vec<PathBuf>, String>>,
}

impl RolloutLookup {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            paths: Default::default(),
        }
    }

    fn paths(&self) -> Result<&[PathBuf], String> {
        let paths = self
            .paths
            .get_or_init(|| {
                let mut pending = vec![
                    self.home.join("sessions"),
                    self.home.join("archived_sessions"),
                ];
                let mut directories = 0;
                let mut entries = 0;
                let mut paths = Vec::new();
                while let Some(directory) = pending.pop() {
                    directories += 1;
                    if directories > 4096 {
                        return Err("Codex session directory limit exceeded".into());
                    }
                    let metadata = match std::fs::symlink_metadata(&directory) {
                        Ok(meta) => meta,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error.to_string()),
                    };
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err("Codex session directory is unsafe".into());
                    }
                    crate::sync_metrics::record_directory_visited(&directory);
                    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
                        entries += 1;
                        if entries > 80000 {
                            return Err("Codex session entry limit exceeded".into());
                        }
                        let entry = entry.map_err(|error| error.to_string())?;
                        let kind = entry.file_type().map_err(|error| error.to_string())?;
                        if kind.is_dir() {
                            pending.push(entry.path());
                        } else if kind.is_file()
                            && entry.path().extension().is_some_and(|ext| ext == "jsonl")
                        {
                            paths.push(entry.path());
                        }
                    }
                }
                paths.sort();
                Ok(paths)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        Ok(paths)
    }

    pub fn find_rollout(&self, id: &str) -> Result<Option<PathBuf>, String> {
        let original = format!("-{id}.jsonl");
        let reverted = format!("_{id}.jsonl");
        let mut matches = self.paths()?.iter().filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&original) || name.ends_with(&reverted))
        });
        let found = matches.next().cloned();
        if matches.next().is_some() {
            return Err("Codex rollout identity has multiple files".into());
        }
        Ok(found)
    }

    pub fn find_current(&self, thread_id: &str) -> Result<Option<PathBuf>, String> {
        let original = format!("-{thread_id}.jsonl");
        let reverted = format!("-{thread_id}_");
        let candidates: Vec<_> = self
            .paths()?
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(&original) || name.contains(&reverted))
            })
            .cloned()
            .collect();
        self.select_current(thread_id, &candidates)
    }

    pub fn select_current(
        &self,
        thread_id: &str,
        candidates: &[PathBuf],
    ) -> Result<Option<PathBuf>, String> {
        crate::codex_rollout_selection::select_current(thread_id, candidates)
    }

    pub fn verify_current(&self, thread_id: &str, path: &Path) -> Result<(), String> {
        if Self::new(&self.home).find_current(thread_id)?.as_deref() != Some(path) {
            return Err(PUSH_CHANGED.into());
        }
        Ok(())
    }
}

pub struct CapturedChild {
    pub path: PathBuf,
    pub bytes: crate::payload::PayloadBytes,
    pub resolved: crate::codex_history_graph::ResolvedCodexRoot,
    pub sources: crate::codex_history_graph::CodexSourceSnapshots,
    pub local: SubagentContentState,
}

impl CapturedTree {
    pub fn omitted() -> Self { Self { omitted: true, ..Self::default() } }
}

#[derive(Default)]
pub struct CapturedTree {
    headers: Vec<(
        PathBuf,
        ChildMetadata,
        Option<crate::jsonl_tail_io::SourceStamp>,
    )>,
    pub children: BTreeMap<SubagentKey, CapturedChild>,
    pub omitted: bool,
}

fn validate_declared_references(
    references: &CreationRecords,
    children: &BTreeMap<SubagentKey, (Vec<u8>, SubagentContentState)>,
) -> Result<(), String> {
    // Native omitted references remain outside the portable namespace.
    for id in references.keys() {
        if let Some(key) = id.strip_prefix("mallard-child:") {
            if !children.contains_key(&SubagentKey::parse(key)?) {
                return Err("cloud tree references an unlisted child".into());
            }
        }
    }
    Ok(())
}

/// Validate the manifest's complete declared tree before any local file write.
/// Each wire graph is resolved once; its parse supplies ownership references,
/// ordinal validation, and content state. Each builder verifies its output once.
pub fn prepare_child_payloads(
    main_creations: CreationRecords,
    children: &BTreeMap<SubagentKey, (Vec<u8>, SubagentContentState)>,
    destinations: &BTreeMap<String, String>,
    root: &str,
    canonical_root: Option<&str>,
    redaction: &mut crate::credential_redaction::RedactionRun<'_>,
) -> Result<Vec<InstalledChildPayload>, String> {
    validate_declared_references(&main_creations, children)?;
    let mut references = BTreeMap::from([(None, main_creations)]);
    let mut result = Vec::with_capacity(children.len());
    // Owner-scoped keys sort before their descendants, so the owner's verified
    // references are already available when its child is prepared.
    for (key, (bytes, normalized)) in children {
        let portable_id = portable_child_id(key);
        let owner_id = key
            .owner()
            .as_ref()
            .map(portable_child_id)
            .unwrap_or_else(|| PORTABLE_MAIN_ID.to_string());
        let refs = references
            .get(&key.owner())
            .ok_or("cloud child owning rollout is missing")?;
        let (creation, _, _) = refs
            .get(&portable_id)
            .ok_or("cloud child has no owning creation reference")?;
        if SubagentKey::child(key.owner().as_ref(), creation)? != *key {
            return Err("cloud child creation key disagrees".into());
        }
        let resolved = crate::codex_history_graph::resolve_standalone_codex_bytes(bytes)?;
        let metadata = ChildMetadata::from_bytes(bytes, &portable_id)?;
        if metadata.spawning_parent_id.as_deref() != Some(&owner_id) {
            return Err("cloud child has the wrong spawning parent".into());
        }
        if resolved.leaf_safe_end != bytes.len() as u64 || !resolved_boundary(&metadata, &resolved)
        {
            return Err("cloud child has unsupported native history".into());
        }
        let wire_state = SubagentContentState::from_tail(&compute_portable_conversation_tail(
            TailPartitionKind::WholeThread,
            0,
            &resolved.chain,
        )?)?;
        if &wire_state != normalized {
            return Err("cloud child normalized content differs from manifest".into());
        }
        validate_declared_references(&resolved.leaf_creations, children)?;
        let destination = destinations
            .get(&portable_id)
            .ok_or("cloud child has no allocated native identity")?;
        let artifact = crate::codex_standalone_builder::decode_codex_tree_member(
            &resolved,
            bytes,
            destination,
            root,
            canonical_root,
            &whole_child_layout(),
            destinations,
            redaction,
        )?;
        let expected_parent = destinations
            .get(&owner_id)
            .ok_or("cloud child parent has no allocated identity")?;
        let output_metadata = ChildMetadata::from_bytes(&artifact.root_bytes, destination)?;
        if output_metadata.spawning_parent_id.as_deref() != Some(expected_parent) {
            return Err("installed Codex child has the wrong spawning parent".into());
        }
        let local = SubagentContentState::from_tail(&artifact.portable_tail)?;
        references.insert(Some(key.clone()), resolved.leaf_creations);
        result.push(InstalledChildPayload {
            key: key.clone(),
            provider_thread_id: destination.clone(),
            parent_provider_thread_id: expected_parent.clone(),
            transcript: artifact.verification,
            accepted: mallard_domain::AcceptedSubagentState {
                local,
                normalized: normalized.clone(),
            },
            origins: artifact.path_origins,
        });
    }
    Ok(result)
}

impl CapturedTree {
    pub fn verify_scope(&self) -> Result<(), String> {
        for (path, metadata, boundary_stamp) in &self.headers {
            let current = read_child_metadata(path, &metadata.provider_thread_id)
                .map_err(|_| PUSH_CHANGED)?;
            if &current != metadata {
                return Err(PUSH_CHANGED.into());
            }
            if let Some(stamp) = boundary_stamp {
                if stamp.verify(path).is_err() {
                    let (bytes, _) =
                        crate::jsonl_tail_io::read_snapshot(path).map_err(|_| PUSH_CHANGED)?;
                    if current
                        .has_supported_boundary(&bytes)
                        .map_err(|_| PUSH_CHANGED)?
                    {
                        return Err(PUSH_CHANGED.into());
                    }
                }
            }
        }
        Ok(())
    }
    /// Historical snapshots are immutable inputs after capture. Only each
    /// child's current role retains a live-file freshness requirement.
    pub fn verify_sources(&self) -> Result<(), String> {
        for child in self.children.values() {
            child
                .sources
                .get(&child.path)
                .ok_or("child source stamp is missing")?
                .1
                .verify(&child.path)
                .map_err(|_| PUSH_CHANGED.to_string())?;
        }
        Ok(())
    }

    pub fn verify_current(&self, lookup: &RolloutLookup) -> Result<(), String> {
        self.verify_sources()?;
        for child in self.children.values() {
            lookup.verify_current(&child.resolved.leaf_meta.id, &child.path)?;
        }
        for (path, metadata, _) in &self.headers {
            lookup.verify_current(&metadata.provider_thread_id, path)?;
        }
        Ok(())
    }
}

pub fn whole_child_layout() -> mallard_domain::RootTailLayout {
    mallard_domain::RootTailLayout {
        codec: mallard_domain::PORTABLE_TAIL_CODEC_V1.into(),
        partition_kind: TailPartitionKind::WholeThread,
        start_record_count: 0,
    }
}

/// Native cursor/build APIs retain their provider codec contract. Only the
/// compact comparison evidence uses the neutral child labels.
pub fn native_child_tail(
    state: &SubagentContentState,
    wire: bool,
) -> mallard_domain::PortableConversationTail {
    let mut tail = state.local_tail();
    tail.codec = if wire {
        mallard_domain::PORTABLE_TAIL_CODEC_WIRE_V1
    } else {
        mallard_domain::PORTABLE_TAIL_CODEC_V1
    }
    .into();
    tail
}

pub(crate) fn verify_activity_identity(
    seen: &mut BTreeMap<String, SubagentActivity>,
    activity: &SubagentActivity,
) -> Result<(), String> {
    if let Some(previous) = seen.insert(activity.event_id.clone(), activity.clone()) {
        if previous != *activity {
            return Err("conflicting Codex activity representations".into());
        }
    }
    Ok(())
}

fn verify_captured_cutoff(bytes: &[u8], cutoff: u64) -> Result<(), String> {
    if cutoff == 0 || cutoff > bytes.len() as u64 || bytes[cutoff as usize - 1] != b'\n' {
        return Err("invalid installed Codex discovery cutoff".into());
    }
    Ok(())
}

/// Collect only new creation records; installed identities come from the binding.
/// Ownership validation, not copied record position, excludes ancestor children.
pub fn creation_references(
    bytes: &[u8],
    installed_end: Option<u64>,
) -> Result<BTreeMap<String, String>, String> {
    let start = installed_end.unwrap_or(0) as usize;
    if start > bytes.len() || (start > 0 && bytes[start - 1] != b'\n') {
        return Err("invalid installed Codex discovery cutoff".into());
    }
    let mut events = BTreeMap::new();
    let mut result = BTreeMap::new();
    for (index, record) in
        parsed_records(&bytes[start..safe_record_end(bytes) as usize]).enumerate()
    {
        if index >= MAX_CODEX_RECORDS {
            return Err("Codex record limit exceeded".into());
        }
        let record = record.map_err(|_| "invalid Codex activity record")?;
        let Some(activity) = SubagentActivity::from_record(&record)? else {
            continue;
        };
        verify_activity_identity(&mut events, &activity)?;
        if activity.kind == SubagentActivityKind::Started {
            if let Some(previous) =
                result.insert(activity.provider_thread_id, activity.event_id.clone())
            {
                if previous != activity.event_id {
                    return Err("Codex child has multiple creation events".into());
                }
            }
        }
    }
    Ok(result)
}

/// Capture one complete owned tree. A known unsupported owned child omits all
/// children. Missing evidence fails the caller's parent only.
pub fn capture_tree(
    main_bytes: &[u8],
    main_id: &str,
    binding: Option<&mallard_domain::LocalConversationBinding>,
    lookup: &RolloutLookup,
) -> Result<CapturedTree, String> {
    capture_tree_references(
        main_id,
        creation_references(
            main_bytes,
            binding.and_then(|binding| binding.installed_end),
        )?,
        main_bytes.len() as u64,
        binding,
        lookup,
    )
}

pub fn capture_resolved_tree(
    main: &crate::codex_history_graph::ResolvedCodexRoot,
    sources: &crate::codex_history_graph::CodexSourceSnapshots,
    binding: Option<&mallard_domain::LocalConversationBinding>,
    lookup: &RolloutLookup,
) -> Result<CapturedTree, String> {
    let cutoff = binding.and_then(|binding| binding.installed_end);
    if let Some(cutoff) = cutoff {
        let path = &main.segments.last().ok_or("main source is missing")?.path;
        let (bytes, _) = sources.get(path).ok_or("main source was not captured")?;
        verify_captured_cutoff(bytes, cutoff)?;
    }
    let references = main
        .leaf_creations
        .iter()
        .filter(|(_, (_, offset, _))| *offset >= cutoff.unwrap_or(0))
        .map(|(id, (creation, _, _))| (id.clone(), creation.clone()))
        .collect();
    capture_tree_references(
        &main.leaf_meta.id,
        references,
        sources.values().map(|(bytes, _)| bytes.len() as u64).sum(),
        binding,
        lookup,
    )
}

fn capture_tree_references(
    main_id: &str,
    references: BTreeMap<String, String>,
    initial_bytes: u64,
    binding: Option<&mallard_domain::LocalConversationBinding>,
    lookup: &RolloutLookup,
) -> Result<CapturedTree, String> {
    let mut tree = CapturedTree::default();
    let installed = binding.map(|binding| &binding.installed_subagents);
    let mut pending = vec![(None, main_id.to_string(), references)];
    let mut identities = BTreeSet::from([main_id.to_string()]);
    let mut total = initial_bytes;
    if total > crate::provider_capture::MAX_CAPTURE_BYTES {
        return Err("Codex tree exceeds capture limit".into());
    }
    while let Some((owner_key, owner_id, references)) = pending.pop() {
        let candidates = child_candidates(installed, owner_key.as_ref(), references)?;
        for (key, (id, required)) in candidates {
            let path = lookup
                .find_current(&id)?
                .ok_or("installed Codex child file is missing")?;
            let meta = read_child_metadata(&path, &id)?;
            if meta.spawning_parent_id.as_deref() != Some(owner_id.as_str()) {
                if required {
                    return Err("installed Codex child ownership changed".into());
                }
                tree.headers.push((path, meta, None));
                continue;
            }
            if tree.children.len() + 1 >= mallard_domain::MAX_CONVERSATION_FILES
                || !identities.insert(id.clone())
            {
                return Err("Codex child tree exceeds bounds or repeats an identity".into());
            }
            if !meta.has_supported_header() {
                tree.headers.push((path, meta, None));
                tree.omitted = true;
                tree.children.clear();
                return Ok(tree);
            }
            let (bytes, stamp) = crate::jsonl_tail_io::read_snapshot(&path)?;
            if ChildMetadata::from_bytes(&bytes, &id)? != meta {
                return Err(PUSH_CHANGED.into());
            }
            let bytes: crate::payload::PayloadBytes = bytes.into();
            let cutoff = installed
                .and_then(|map| map.get(&key))
                .map(|child| child.installed_end);
            if let Some(cutoff) = cutoff {
                verify_captured_cutoff(&bytes, cutoff)?;
            }
            let (resolved, sources) = crate::codex_history_graph::resolve_captured_codex_root(
                &path,
                bytes.clone(),
                stamp.clone(),
                &|id| lookup.find_rollout(id),
            )?;
            total = total
                .checked_add(
                    sources
                        .values()
                        .map(|(bytes, _)| bytes.len() as u64)
                        .sum::<u64>(),
                )
                .ok_or("Codex tree size overflow")?;
            if total > crate::provider_capture::MAX_CAPTURE_BYTES {
                return Err("Codex tree exceeds capture limit".into());
            }
            if !resolved_boundary(&meta, &resolved) {
                tree.headers.push((path, meta, Some(stamp)));
                tree.omitted = true;
                tree.children.clear();
                return Ok(tree);
            }
            let refs = resolved
                .leaf_creations
                .iter()
                .filter(|(_, (_, offset, _))| *offset >= cutoff.unwrap_or(0))
                .map(|(id, (creation, _, _))| (id.clone(), creation.clone()))
                .collect();
            let local = SubagentContentState::from_tail(&compute_portable_conversation_tail(
                TailPartitionKind::WholeThread,
                0,
                &resolved.chain,
            )?)?;
            pending.push((Some(key.clone()), id, refs));
            tree.children.insert(
                key,
                CapturedChild {
                    path,
                    bytes,
                    resolved,
                    sources,
                    local,
                },
            );
        }
    }
    Ok(tree)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentActivity {
    pub event_id: String,
    pub kind: SubagentActivityKind,
    pub provider_thread_id: String,
    pub agent_path: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentActivityKind {
    Started,
    Interacted,
    Interrupted,
    Completed,
}

impl SubagentActivity {
    /// Legacy events and their paginated projections identify the same event.
    pub fn from_record(record: &Value) -> Result<Option<Self>, String> {
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            return Ok(None);
        }
        let payload = &record["payload"];
        let (activity, id_field) = match payload["type"].as_str() {
            Some("sub_agent_activity") => (payload, "event_id"),
            Some("item_completed") if payload["item"]["type"] == "SubAgentActivity" => {
                (&payload["item"], "id")
            }
            _ => return Ok(None),
        };
        let event_id = required_id(&activity[id_field], 256)?;
        let kind = match activity["kind"].as_str() {
            Some("started") => SubagentActivityKind::Started,
            Some("interacted") => SubagentActivityKind::Interacted,
            Some("interrupted") => SubagentActivityKind::Interrupted,
            Some("completed") => SubagentActivityKind::Completed,
            _ => return Err("unsupported Codex subagent activity kind".to_string()),
        };
        if kind == SubagentActivityKind::Started {
            SubagentKey::child(None, &event_id)?;
        }
        let provider_thread_id = required_id(&activity["agent_thread_id"], 4096)?;
        let agent_path = activity["agent_path"].clone();
        match &agent_path {
            Value::String(path) => {
                required_id(&Value::String(path.clone()), 1024)?;
            }
            Value::Array(parts) if parts.len() <= crate::codex_rollout::MAX_CODEX_FORK_DEPTH => {
                for part in parts {
                    required_id(part, 256)?;
                }
            }
            _ => return Err("invalid Codex subagent path".into()),
        }
        Ok(Some(Self {
            event_id,
            kind,
            provider_thread_id,
            agent_path,
        }))
    }
}

fn required_id(value: &Value, max_bytes: usize) -> Result<String, String> {
    value
        .as_str()
        .filter(|value| {
            !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
        })
        .map(str::to_string)
        .ok_or_else(|| "invalid Codex subagent identifier".to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildMetadata {
    pub provider_thread_id: String,
    pub spawning_parent_id: Option<String>,
    pub history_mode: Option<String>,
    pub history_start_ordinal: Option<u64>,
}

impl ChildMetadata {
    pub fn from_bytes(bytes: &[u8], expected_id: &str) -> Result<Self, String> {
        let (meta, record) =
            authoritative_session_meta_with_record(bytes).map_err(|error| error.to_string())?;
        if meta.id != expected_id {
            return Err("Codex child reference disagrees with its file identity".to_string());
        }
        let payload = &record["payload"];
        let spawn = payload.pointer("/source/subagent/thread_spawn");
        let spawning_parent_id = spawn
            .map(|spawn| required_id(&spawn["parent_thread_id"], 4096))
            .transpose()?;
        if let Some(parent) = &spawning_parent_id {
            if payload
                .get("parent_thread_id")
                .is_some_and(|value| value.as_str() != Some(parent))
            {
                return Err("Codex child parent metadata conflicts".to_string());
            }
            if parent == &meta.id {
                return Err("Codex child owns itself".to_string());
            }
        }
        Ok(Self {
            provider_thread_id: meta.id,
            spawning_parent_id,
            history_mode: meta.history_mode,
            history_start_ordinal: payload["subagent_history_start_ordinal"].as_u64(),
        })
    }

    /// Missing/invalid native child boundaries select the reviewed omission
    /// policy. They never imply an empty inherited region.
    pub fn has_supported_header(&self) -> bool {
        self.history_mode.as_deref() == Some("paginated")
            && self
                .history_start_ordinal
                .is_some_and(|ordinal| ordinal > 0)
    }

    pub fn has_supported_boundary(&self, bytes: &[u8]) -> Result<bool, String> {
        if !self.has_supported_header() {
            return Ok(false);
        }
        let complete = &bytes[..safe_record_end(bytes) as usize];
        let mut end = None;
        for (index, record) in parsed_records(complete).enumerate() {
            if index >= MAX_CODEX_RECORDS {
                return Err("Codex child exceeds its record limit".to_string());
            }
            let record = record.map_err(|_| "Codex child contains malformed JSON")?;
            let ordinal = record["ordinal"]
                .as_u64()
                .ok_or("Codex paginated child lacks a record ordinal")?;
            if end.is_some_and(|expected| ordinal != expected) {
                return Err("Codex child record ordinals are not contiguous".to_string());
            }
            end = Some(
                ordinal
                    .checked_add(1)
                    .ok_or("Codex child ordinal overflow")?,
            );
        }
        Ok(end
            .zip(self.history_start_ordinal)
            .is_some_and(|(end, start)| start <= end))
    }
}

/// Read only bounded authoritative metadata while checking a candidate's owner.
pub fn read_child_metadata(path: &Path, expected_id: &str) -> Result<ChildMetadata, String> {
    let file = open_no_follow(path)?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > crate::app_settings::conversation_file_limit_bytes()
    {
        return Err("Codex child file is not a bounded regular transcript".to_string());
    }
    let mut reader = BufReader::new(file.take(MAX_METADATA_BYTES + 1));
    let mut bytes = Vec::new();
    loop {
        let read = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|error| error.to_string())?;
        if read == 0 || bytes.len() as u64 > MAX_METADATA_BYTES || bytes.last() != Some(&b'\n') {
            return Err("Codex child metadata is missing, incomplete, or oversized".to_string());
        }
        if bytes.iter().any(|byte| !byte.is_ascii_whitespace()) {
            break;
        }
    }
    crate::sync_metrics::record_transcript_bytes_read(path, bytes.len() as u64);
    ChildMetadata::from_bytes(&bytes, expected_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(home: &Path, id: &str, parent: Option<&str>, refs: &[(&str, &str)]) -> Vec<u8> {
        let mut meta = json!({"type":"session_meta","payload":{"id":id,"cwd":"/tmp/project","history_mode":"paginated"}});
        if let Some(parent) = parent {
            meta["payload"]["source"] =
                json!({"subagent":{"thread_spawn":{"parent_thread_id":parent}}});
            meta["payload"]["parent_thread_id"] = json!(parent);
            meta["payload"]["subagent_history_start_ordinal"] = json!(1);
        }
        let mut records = vec![
            meta,
            json!({"type":"response_item","payload":{"type":"message","id":format!("message-{id}"),"role":"user","content":"hello"}}),
        ];
        for (creation, child) in refs {
            records.push(json!({"type":"event_msg","payload":{"type":"sub_agent_activity","event_id":creation,"kind":"started","agent_thread_id":child,"agent_path":["worker"],"occurred_at_ms":42}}));
        }
        let mut bytes = Vec::new();
        for (ordinal, mut record) in records.into_iter().enumerate() {
            record["ordinal"] = json!(ordinal);
            record["timestamp"] = json!("2026-09-08T00:00:00Z");
            serde_json::to_writer(&mut bytes, &record).unwrap();
            bytes.push(b'\n');
        }
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        std::fs::write(home.join(format!("sessions/rollout-{id}.jsonl")), &bytes).unwrap();
        bytes
    }

    #[test]
    fn owned_nested_tree_round_trip_excludes_inherited_child() {
        let home = tempfile::tempdir().unwrap();
        fixture(home.path(), "C", Some("P"), &[]);
        fixture(home.path(), "E", Some("D"), &[]);
        fixture(home.path(), "D", Some("T"), &[("start-E", "E")]);
        let main = fixture(
            home.path(),
            "T",
            None,
            &[("start-C", "C"), ("start-D", "D")],
        );
        let lookup = RolloutLookup::new(home.path());
        let tree = capture_tree(&main, "T", None, &lookup).unwrap();
        assert!(!tree.omitted);
        assert_eq!(
            tree.children
                .keys()
                .map(SubagentKey::as_str)
                .collect::<Vec<_>>(),
            ["start-D", "start-D/start-E"]
        );
        let mut ids = BTreeMap::from([("T".to_string(), PORTABLE_MAIN_ID.to_string())]);
        ids.extend(
            tree.children
                .iter()
                .map(|(key, child)| (child.resolved.leaf_meta.id.clone(), portable_child_id(key))),
        );
        let mut children = BTreeMap::new();
        let origins = crate::session_path_origins::PathOrigins::default();
        let mut redactor = crate::credential_redaction::shared().begin_run();
        for (key, child) in &tree.children {
            let artifact = crate::codex_standalone_builder::encode_codex_tree_member(
                &child.resolved,
                &child.sources,
                "/tmp/project",
                None,
                Some(&whole_child_layout()),
                &mut redactor,
                &origins,
                crate::session_path_origins::OriginLookup::ByPosition,
                &ids,
            )
            .unwrap();
            children.insert(
                key.clone(),
                (
                    artifact.root_bytes.as_bytes().to_vec(),
                    SubagentContentState::from_tail(&artifact.portable_tail).unwrap(),
                ),
            );
        }
        let path = lookup.find_current("T").unwrap().unwrap();
        let (bytes, stamp) = crate::jsonl_tail_io::read_snapshot(&path).unwrap();
        let (resolved, sources) = crate::codex_history_graph::resolve_captured_codex_root(
            &path,
            bytes.into(),
            stamp,
            &|id| lookup.find_rollout(id),
        )
        .unwrap();
        let main = crate::codex_standalone_builder::encode_codex_tree_member(
            &resolved,
            &sources,
            "/tmp/project",
            None,
            None,
            &mut redactor,
            &origins,
            crate::session_path_origins::OriginLookup::ByPosition,
            &ids,
        )
        .unwrap();
        let destinations = ids
            .values()
            .enumerate()
            .map(|(index, id)| (id.clone(), format!("destination-{index}")))
            .collect();
        let main_creations =
            crate::codex_history_graph::resolve_standalone_codex_bytes(&main.root_bytes)
                .unwrap()
                .leaf_creations;
        let before = crate::sync_metrics::analysis_work();
        let installed = prepare_child_payloads(
            main_creations.clone(),
            &children,
            &destinations,
            "/tmp/destination",
            None,
            &mut crate::credential_redaction::shared().begin_run(),
        )
        .unwrap();
        assert_eq!(
            crate::sync_metrics::analysis_work()
                .delta_since(before)
                .passes,
            children.len() as u64 * 2,
            "one wire graph analysis and one builder output analysis per child"
        );
        assert_eq!(installed.len(), 2);
        for child in &installed {
            let resolved =
                crate::codex_history_graph::resolve_standalone_codex_bytes(child.bytes()).unwrap();
            assert!(resolved.leaf_meta.is_internal_subagent);
        }
        for proof in &installed {
            let installed = mallard_domain::InstalledSubagent {
                provider_thread_id: proof.provider_thread_id().to_string(),
                installed_end: proof.bytes().len() as u64,
            };
            let matches = |installed: &mallard_domain::InstalledSubagent,
                           owner: &str,
                           accepted: &mallard_domain::AcceptedSubagentState,
                           root: &str,
                           bytes: &crate::payload::PayloadBytes| {
                proof.matches_installation(
                    &mallard_domain::ProviderKey::parse("codex").unwrap(),
                    installed,
                    owner,
                    accepted,
                    root,
                    root,
                    bytes,
                )
            };
            assert!(matches(
                &installed,
                &proof.parent_provider_thread_id,
                proof.accepted(),
                "/tmp/destination",
                proof.bytes()
            ));
            assert!(!matches(
                &installed,
                "wrong-owner",
                proof.accepted(),
                "/tmp/destination",
                proof.bytes()
            ));
            assert!(!matches(
                &installed,
                &proof.parent_provider_thread_id,
                proof.accepted(),
                "/tmp/other",
                proof.bytes()
            ));
            let mut wrong_id = installed.clone();
            wrong_id.provider_thread_id = "another-child".into();
            assert!(!matches(
                &wrong_id,
                &proof.parent_provider_thread_id,
                proof.accepted(),
                "/tmp/destination",
                proof.bytes()
            ));
            let mut wrong_end = installed.clone();
            wrong_end.installed_end += 1;
            assert!(!matches(
                &wrong_end,
                &proof.parent_provider_thread_id,
                proof.accepted(),
                "/tmp/destination",
                proof.bytes()
            ));
            for local in [true, false] {
                let mut accepted = proof.accepted().clone();
                if local {
                    accepted.local.sha256 = "0".repeat(64);
                } else {
                    accepted.normalized.sha256 = "0".repeat(64);
                }
                assert!(!matches(
                    &installed,
                    &proof.parent_provider_thread_id,
                    &accepted,
                    "/tmp/destination",
                    proof.bytes()
                ));
            }
            let mut corrupt = proof.bytes().to_vec();
            corrupt[0] = b'!';
            assert!(!matches(
                &installed,
                &proof.parent_provider_thread_id,
                proof.accepted(),
                "/tmp/destination",
                &corrupt.into()
            ));
        }
        let first_key = children.keys().next().unwrap().clone();
        for invalid in ["boundary", "owner", "ordinal", "content", "truncated"] {
            let mut bad = children.clone();
            let bytes = &mut bad.get_mut(&first_key).unwrap().0;
            let mut records = parsed_records(bytes)
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            match invalid {
                "boundary" => {
                    records[0]["payload"]["subagent_history_start_ordinal"] = json!(u64::MAX)
                }
                "owner" => {
                    records[0]["payload"]["source"]["subagent"]["thread_spawn"]
                        ["parent_thread_id"] = json!("wrong-owner");
                    records[0]["payload"]["parent_thread_id"] = json!("wrong-owner");
                }
                "ordinal" => records[1]["ordinal"] = json!(1000),
                "content" => records[1]["payload"]["content"] = json!("changed content"),
                _ => {}
            }
            *bytes = records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>()
                .into_bytes();
            if invalid == "truncated" {
                bytes.pop();
            }
            assert!(
                prepare_child_payloads(
                    main_creations.clone(),
                    &bad,
                    &destinations,
                    "/tmp/destination",
                    None,
                    &mut crate::credential_redaction::shared().begin_run()
                )
                .is_err(),
                "{invalid}"
            );
        }
        let mut missing = children.clone();
        missing.remove(&first_key);
        assert!(prepare_child_payloads(
            main_creations,
            &missing,
            &destinations,
            "/tmp/destination",
            None,
            &mut crate::credential_redaction::shared().begin_run()
        )
        .is_err());
        let refs = creation_references(&main.root_bytes, None).unwrap();
        assert_eq!(refs.get("C").map(String::as_str), Some("start-C"));
    }

    #[test]
    fn unsupported_owned_child_omits_tree_but_foreign_child_does_not() {
        let home = tempfile::tempdir().unwrap();
        let bytes = fixture(home.path(), "D", Some("T"), &[]);
        let bytes = String::from_utf8(bytes)
            .unwrap()
            .replace("\"subagent_history_start_ordinal\":1,", "");
        // Remove through JSON so object-key order is immaterial.
        let mut records = parsed_records(bytes.as_bytes())
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        records[0]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("subagent_history_start_ordinal");
        std::fs::write(
            home.path().join("sessions/rollout-D.jsonl"),
            records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let main = fixture(home.path(), "T", None, &[("start-D", "D")]);
        assert!(
            capture_tree(&main, "T", None, &RolloutLookup::new(home.path()))
                .unwrap()
                .omitted
        );
        let fork = fixture(home.path(), "F", None, &[("start-D", "D")]);
        let tree = capture_tree(&fork, "F", None, &RolloutLookup::new(home.path())).unwrap();
        assert!(!tree.omitted && tree.children.is_empty());
    }

    #[test]
    fn activity_shapes_share_creation_identity_and_kinds() {
        for kind in ["started", "interacted", "interrupted", "completed"] {
            let old = json!({"type":"event_msg", "payload":{
                "type":"sub_agent_activity", "event_id":"creation-1", "kind":kind,
                "agent_thread_id":"child", "agent_path":["worker"], "occurred_at_ms":42
            }});
            let new = json!({"type":"event_msg", "payload":{"type":"item_completed", "item":{
                "type":"SubAgentActivity", "id":"creation-1", "kind":kind,
                "agent_thread_id":"child", "agent_path":["worker"]
            }}});
            assert_eq!(
                SubagentActivity::from_record(&old).unwrap(),
                SubagentActivity::from_record(&new).unwrap()
            );
        }
        let invalid = json!({"type":"event_msg","payload":{"type":"sub_agent_activity","event_id":"creation","kind":"spawned"}});
        assert!(SubagentActivity::from_record(&invalid).is_err());
    }

    #[test]
    fn metadata_proves_owner_without_guessing_a_missing_boundary() {
        let mut meta = json!({"ordinal":0,"type":"session_meta","payload":{
            "id":"child", "cwd":"/fixture/project", "parent_thread_id":"parent",
            "history_mode":"paginated", "subagent_history_start_ordinal":1,
            "source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}
        }});
        let bytes = serde_json::to_vec(&meta).unwrap();
        let parsed = ChildMetadata::from_bytes(&bytes, "child").unwrap();
        assert_eq!(parsed.spawning_parent_id.as_deref(), Some("parent"));
        assert!(parsed.has_supported_header());
        assert!(ChildMetadata::from_bytes(&bytes, "wrong-child").is_err());
        meta["payload"]
            .as_object_mut()
            .unwrap()
            .remove("subagent_history_start_ordinal");
        assert!(
            !ChildMetadata::from_bytes(&serde_json::to_vec(&meta).unwrap(), "child")
                .unwrap()
                .has_supported_header()
        );
        meta["payload"]["parent_thread_id"] = json!("other-parent");
        assert!(ChildMetadata::from_bytes(&serde_json::to_vec(&meta).unwrap(), "child").is_err());
    }
    fn observe_fixture(home: &Path) -> DiscoveryObservation {
        let path = home.join("sessions/rollout-T.jsonl");
        let (bytes, stamp) = crate::jsonl_tail_io::read_snapshot(&path).unwrap();
        let bytes: crate::payload::PayloadBytes = bytes.into();
        let lookup = RolloutLookup::new(home);
        let (resolved, _) = crate::codex_history_graph::resolve_captured_codex_root(
            &path,
            bytes.clone(),
            stamp.clone(),
            &|id| lookup.find_rollout(id),
        )
        .unwrap();
        let seeded = crate::codex_history_graph::seed_codex_root(&resolved, &bytes, None).unwrap();
        DiscoveryObservation {
            mapping: seeded.mapping,
            stamp,
            creations: resolved.leaf_creations,
        }
    }

    #[test]
    fn warm_tree_reads_no_child_bodies_and_append_leaves_siblings_untouched() {
        use std::io::Write;
        let home = tempfile::tempdir().unwrap();
        fixture(home.path(), "D", Some("T"), &[]);
        fixture(home.path(), "E", Some("T"), &[]);
        fixture(
            home.path(),
            "T",
            None,
            &[("start-D", "D"), ("start-E", "E")],
        );
        let metrics = crate::sync_metrics::register_root(home.path());
        let sibling_metrics =
            crate::sync_metrics::register_root(&home.path().join("sessions/rollout-E.jsonl"));
        let main = observe_fixture(home.path());
        let cold =
            observe_tree(main.clone(), None, &RolloutLookup::new(home.path()), None).unwrap();
        assert_eq!(
            metrics.snapshot().directories_visited,
            1,
            "one filename traversal for the tree"
        );
        let before = metrics.snapshot();
        let warm = observe_tree(
            main.clone(),
            None,
            &RolloutLookup::new(home.path()),
            Some(&cold),
        )
        .unwrap();
        let work = metrics.snapshot().delta_since(&before);
        assert_eq!(work.transcript_bytes_read, 0);
        assert_eq!(work.full_graph_resolves, 0);
        assert_eq!(
            work.directories_visited, 1,
            "recheck current rollout filenames"
        );
        let path = home.path().join("sessions/rollout-D.jsonl");
        let record = json!({"ordinal":2,"timestamp":"2026-09-08T00:01:00Z","type":"response_item","payload":{"type":"message","id":"follow-up","role":"user","content":"next"}});
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap(),
            "{record}"
        )
        .unwrap();
        let before = metrics.snapshot();
        let sibling_before = sibling_metrics.snapshot();
        let appended =
            observe_tree(main, None, &RolloutLookup::new(home.path()), Some(&warm)).unwrap();
        let work = metrics.snapshot().delta_since(&before);
        assert_eq!(work.full_graph_resolves, 0);
        assert_eq!(work.records_parsed, 1);
        assert_eq!(
            sibling_metrics
                .snapshot()
                .delta_since(&sibling_before)
                .transcript_bytes_read,
            0
        );
        assert_eq!(
            appended.children[&SubagentKey::parse("start-D").unwrap()]
                .state
                .message_count,
            warm.children[&SubagentKey::parse("start-D").unwrap()]
                .state
                .message_count
                + 1
        );
    }

    #[test]
    fn captured_source_change_and_scope_change_have_the_retry_message() {
        use std::io::Write;
        let home = tempfile::tempdir().unwrap();
        fixture(home.path(), "D", Some("T"), &[]);
        let main = fixture(home.path(), "T", None, &[("start-D", "D")]);
        let tree = capture_tree(&main, "T", None, &RolloutLookup::new(home.path())).unwrap();
        let path = home.path().join("sessions/rollout-D.jsonl");
        write!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap(),
            "partial"
        )
        .unwrap();
        assert_eq!(tree.verify_sources().unwrap_err(), PUSH_CHANGED);
        fixture(home.path(), "D", Some("another-owner"), &[]);
        assert_eq!(tree.verify_sources().unwrap_err(), PUSH_CHANGED);
        fixture(home.path(), "D", Some("another-owner"), &[]);
        let tree = capture_tree(&main, "T", None, &RolloutLookup::new(home.path())).unwrap();
        fixture(home.path(), "D", Some("T"), &[]);
        assert_eq!(tree.verify_scope().unwrap_err(), PUSH_CHANGED);
    }

    #[test]
    fn duplicate_creation_cannot_change_owner_path_and_rewrites_preserve_text() {
        let mut record = json!({"type":"event_msg","payload":{"type":"sub_agent_activity","event_id":"creation","kind":"started","agent_thread_id":"D","agent_path":"/root/child"}});
        let mut records = CreationRecords::new();
        record_creation(&mut records, &record, 10).unwrap();
        record_creation(&mut records, &record, 20).unwrap();
        record["payload"]["agent_path"] = json!("/root/other");
        assert!(record_creation(&mut records, &record, 30).is_err());
        let ids = BTreeMap::from([("D".into(), "mapped-D".into())]);
        rewrite_references(&mut record, &ids);
        assert_eq!(record["payload"]["event_id"], "creation");
        assert_eq!(record["payload"]["agent_thread_id"], "mapped-D");
        let mut message =
            json!({"type":"response_item","payload":{"type":"message","content":"D","id":"D"}});
        let original = message.clone();
        rewrite_references(&mut message, &ids);
        assert_eq!(message, original);
    }

    #[test]
    fn a_call_before_the_installed_end_is_left_out_unless_its_child_was_installed() {
        use crate::owned_subagents::agent_card_fixtures::{
            binding_of_a_pulled_session, index_of_agent_calls,
        };
        // An earlier rollout of 100 bytes, then the current one. The Pull
        // installed 200 bytes of the current one.
        let index = index_of_agent_calls(
            &[100, 500],
            &[
                ("in-the-earlier-rollout", Some("earlier"), 50),
                ("came-without", Some("gone"), 110),
                ("came-with", Some("installed"), 120),
                ("unanswered", None, 130),
                ("last-before-the-end", Some("gone-too"), 299),
                ("first-after-the-end", Some("here"), 300),
                ("made-here", Some("here-too"), 400),
            ],
        );
        let binding =
            binding_of_a_pulled_session(Some(200), &[("start-installed", "installed")], &[]);
        binding.validate().unwrap();
        assert_eq!(
            agent_calls_without_transcript(&index, &binding),
            BTreeSet::from([
                "came-without".to_string(),
                "last-before-the-end".to_string()
            ])
        );
        // One rollout: the places of the index are the places of the file.
        let mut alone = index.clone();
        alone.sources.remove(0);
        assert_eq!(
            agent_calls_without_transcript(&alone, &binding),
            BTreeSet::from([
                "in-the-earlier-rollout".to_string(),
                "came-without".to_string()
            ])
        );
        // A session that was made on this computer has no installed end.
        let native = binding_of_a_pulled_session(None, &[], &[]);
        assert!(agent_calls_without_transcript(&index, &native).is_empty());
    }
}
