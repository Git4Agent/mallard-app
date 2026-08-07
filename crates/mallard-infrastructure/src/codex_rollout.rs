//! Bounded structural parsing for Codex rollout JSONL.
//!
//! A forked rollout starts with metadata for the active thread and then
//! contains copied history from its ancestors. The first `session_meta` is
//! therefore authoritative; later metadata is accepted only when it belongs
//! to the active thread's validated `forked_from_id` chain.

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use mallard_domain::MAX_CONVERSATION_FILE_BYTES;

pub const MAX_CODEX_FORK_DEPTH: usize = 256;
pub const MAX_CODEX_RECORDS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexSessionMetaRecord {
    pub record_index: usize,
    pub id: String,
    pub session_id: Option<String>,
    pub forked_from_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub cwd: String,
    pub thread_source: Option<String>,
    pub is_internal_subagent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexRolloutStructure {
    pub current: CodexSessionMetaRecord,
    /// Immediate parent first, followed by increasingly distant ancestors.
    /// The final entry may be a referenced parent whose metadata was pruned
    /// from a truncated rollout.
    pub fork_ancestry: Vec<String>,
    pub accepted_metadata_ids: BTreeSet<String>,
    pub structural_cwds: BTreeSet<String>,
    pub record_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexStructureErrorKind {
    Empty,
    OversizedFile,
    TooManyRecords,
    MalformedJson,
    MissingSessionMetadata,
    MalformedSessionMetadata,
    ActiveIdentityMismatch,
    ConflictingParent,
    SelfParent,
    AncestryCycle,
    ForkDepthExceeded,
    UnrelatedMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexStructureError {
    pub kind: CodexStructureErrorKind,
    pub message: String,
}

impl std::fmt::Display for CodexStructureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodexStructureError {}

/// Byte-level bounds shared by every rollout scan.
pub(crate) fn check_rollout_bounds(bytes: &[u8]) -> Result<(), CodexStructureError> {
    if bytes.is_empty() {
        return Err(error(
            CodexStructureErrorKind::Empty,
            "Codex rollout is empty",
        ));
    }
    if bytes.len() > MAX_CONVERSATION_FILE_BYTES as usize {
        return Err(error(
            CodexStructureErrorKind::OversizedFile,
            "Codex rollout exceeds the bounded transcript limit",
        ));
    }
    Ok(())
}

/// Streaming form of the structural analysis: `feed` one parsed record at a
/// time in file order, then `finalize` for the whole-file ancestry checks.
/// `analyze_codex_rollout_structure` drives it standalone; the fused
/// transcript analysis in `conversation_codec` feeds it from the same single
/// parse that builds the canonical chain, so no caller parses a record
/// twice. This is the only implementation of the structural semantics.
pub struct CodexStructureAccumulator<'a> {
    expected_active_id: Option<&'a str>,
    current: Option<CodexSessionMetaRecord>,
    metadata: Vec<CodexSessionMetaRecord>,
    structural_cwds: BTreeSet<String>,
    record_count: usize,
    conversation_item_seen: bool,
}

impl<'a> CodexStructureAccumulator<'a> {
    pub fn new(expected_active_id: Option<&'a str>) -> Self {
        Self {
            expected_active_id,
            current: None,
            metadata: Vec::new(),
            structural_cwds: BTreeSet::new(),
            record_count: 0,
            conversation_item_seen: false,
        }
    }

    /// Non-empty records fed so far.
    pub fn record_count(&self) -> usize {
        self.record_count
    }

    /// The authoritative session metadata, available as soon as it is fed —
    /// which is before any conversation item in every accepted rollout.
    pub fn current_meta(&self) -> Option<&CodexSessionMetaRecord> {
        self.current.as_ref()
    }

    pub fn feed(&mut self, value: &Value) -> Result<(), CodexStructureError> {
        self.record_count = self.record_count.saturating_add(1);
        if self.record_count > MAX_CODEX_RECORDS {
            return Err(error(
                CodexStructureErrorKind::TooManyRecords,
                "Codex rollout has too many records",
            ));
        }
        let record_type = structural_record_type(value);
        if self.current.is_none()
            && record_type
                .is_some_and(|kind| matches!(kind, "response_item" | "event_msg" | "compacted"))
        {
            self.conversation_item_seen = true;
        }
        match record_type {
            Some("session_meta") => {
                if self.current.is_none() && self.conversation_item_seen {
                    return Err(error(
                        CodexStructureErrorKind::MissingSessionMetadata,
                        "Codex rollout contains conversation items before its authoritative session_meta",
                    ));
                }
                let record = parse_session_meta(value, self.record_count - 1)?;
                self.structural_cwds.insert(record.cwd.clone());
                if self.current.is_none() {
                    if let Some(expected) = self.expected_active_id {
                        if record.id != expected {
                            return Err(error(
                                CodexStructureErrorKind::ActiveIdentityMismatch,
                                format!(
                                    "authoritative session_meta thread ID '{}' does not match assigned thread '{}'",
                                    record.id, expected
                                ),
                            ));
                        }
                    }
                    self.current = Some(record.clone());
                }
                self.metadata.push(record);
            }
            Some("turn_context") => {
                if let Some(cwd) = structural_cwd(value)? {
                    self.structural_cwds.insert(cwd);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Whole-file checks: an authoritative session_meta exists and the fork
    /// ancestry is acyclic, bounded, and fully related.
    pub fn finalize(self) -> Result<CodexRolloutStructure, CodexStructureError> {
        let Self {
            current,
            metadata,
            structural_cwds,
            record_count,
            ..
        } = self;
        let current = current.ok_or_else(|| {
            error(
                CodexStructureErrorKind::MissingSessionMetadata,
                "Codex rollout has no session_meta record",
            )
        })?;

        let mut parents = BTreeMap::<String, Option<String>>::new();
        for record in &metadata {
            if record.forked_from_id.as_deref() == Some(record.id.as_str()) {
                return Err(error(
                    CodexStructureErrorKind::SelfParent,
                    format!("Codex thread '{}' forks from itself", record.id),
                ));
            }
            match parents.entry(record.id.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(record.forked_from_id.clone());
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    match (entry.get(), &record.forked_from_id) {
                        (Some(existing), Some(candidate)) if existing != candidate => {
                            return Err(error(
                                CodexStructureErrorKind::ConflictingParent,
                                format!(
                                    "Codex thread '{}' claims conflicting fork parents",
                                    record.id
                                ),
                            ));
                        }
                        (None, Some(candidate)) => {
                            entry.insert(Some(candidate.clone()));
                        }
                        _ => {}
                    }
                }
            }
        }

        let mut reachable = BTreeSet::from([current.id.clone()]);
        let mut ancestry = Vec::new();
        let mut cursor = current.id.clone();
        for _ in 0..MAX_CODEX_FORK_DEPTH {
            let Some(parent) = parents.get(&cursor).and_then(Clone::clone) else {
                break;
            };
            if !reachable.insert(parent.clone()) {
                return Err(error(
                    CodexStructureErrorKind::AncestryCycle,
                    "Codex fork ancestry contains a cycle",
                ));
            }
            ancestry.push(parent.clone());
            cursor = parent;
        }
        if parents.get(&cursor).and_then(Clone::clone).is_some() {
            return Err(error(
                CodexStructureErrorKind::ForkDepthExceeded,
                format!(
                    "Codex fork ancestry exceeds the supported depth of {}",
                    MAX_CODEX_FORK_DEPTH
                ),
            ));
        }
        if metadata.len() > MAX_CODEX_RECORDS
            || parents.len() > MAX_CODEX_FORK_DEPTH.saturating_add(1)
        {
            return Err(error(
                CodexStructureErrorKind::ForkDepthExceeded,
                "Codex rollout contains too many distinct metadata identities",
            ));
        }
        if let Some(unrelated) = parents.keys().find(|id| !reachable.contains(*id)) {
            return Err(error(
                CodexStructureErrorKind::UnrelatedMetadata,
                format!(
                    "Codex rollout contains unrelated session_meta thread '{}'",
                    unrelated
                ),
            ));
        }

        Ok(CodexRolloutStructure {
            current,
            fork_ancestry: ancestry,
            accepted_metadata_ids: reachable,
            structural_cwds,
            record_count,
        })
    }
}

/// Validate the structural identity, ancestry, and cwd records in one Codex
/// rollout. Identity fields are read only from declared structural records;
/// message content is never recursively searched.
pub fn analyze_codex_rollout_structure(
    bytes: &[u8],
    expected_active_id: Option<&str>,
) -> Result<CodexRolloutStructure, CodexStructureError> {
    check_rollout_bounds(bytes)?;
    let mut structure = CodexStructureAccumulator::new(expected_active_id);
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        // Codex compaction and tool-result records can legitimately occupy
        // several MiB on one JSONL line. The transcript-wide byte and record
        // limits provide the bounded-read contract; a smaller per-line limit
        // would reject otherwise valid conversations.
        let value: Value = serde_json::from_slice(raw).map_err(|_| {
            error(
                CodexStructureErrorKind::MalformedJson,
                "Codex rollout contains malformed JSON",
            )
        })?;
        structure.feed(&value)?;
    }
    structure.finalize()
}

pub fn structural_record_type(value: &Value) -> Option<&str> {
    value
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| payload_object(value).and_then(|payload| string_field(payload, &["type"])))
}

pub(crate) fn session_meta_id(payload: &Map<String, Value>) -> Option<&str> {
    string_field(
        payload,
        &["id", "thread_id", "threadId", "session_id", "sessionId"],
    )
}

pub fn session_meta_is_internal_subagent(value: &Value) -> bool {
    let Some(payload) = payload_object(value) else {
        return false;
    };
    payload.get("thread_source").and_then(Value::as_str) == Some("subagent")
        || payload
            .get("source")
            .and_then(Value::as_object)
            .is_some_and(|source| source.contains_key("subagent"))
}

fn parse_session_meta(
    value: &Value,
    record_index: usize,
) -> Result<CodexSessionMetaRecord, CodexStructureError> {
    let payload = payload_object(value).ok_or_else(|| malformed_meta("has no payload"))?;
    let id = session_meta_id(payload)
        .ok_or_else(|| malformed_meta("has no thread ID"))
        .and_then(|value| validate_text("thread ID", value, 128))?;
    let cwd = string_field(payload, &["cwd", "working_directory", "workingDirectory"])
        .ok_or_else(|| malformed_meta("has no cwd"))
        .and_then(|value| validate_text("cwd", value, 4096))?;
    if !std::path::Path::new(&cwd).is_absolute() {
        return Err(malformed_meta("has a non-absolute cwd"));
    }
    let session_id = optional_text(payload, &["session_id", "sessionId"], "session ID", 128)?;
    let forked_from_id = optional_text(
        payload,
        &["forked_from_id", "forkedFromId"],
        "fork parent",
        128,
    )?;
    let parent_thread_id = optional_text(
        payload,
        &["parent_thread_id", "parentThreadId"],
        "parent thread ID",
        128,
    )?;
    let thread_source = payload
        .get("source")
        .map(|source| {
            serde_json::to_string(source)
                .map_err(|_| malformed_meta("has an invalid source descriptor"))
        })
        .transpose()?;
    Ok(CodexSessionMetaRecord {
        record_index,
        id,
        session_id,
        forked_from_id,
        parent_thread_id,
        cwd,
        thread_source,
        is_internal_subagent: session_meta_is_internal_subagent(value),
    })
}

fn structural_cwd(value: &Value) -> Result<Option<String>, CodexStructureError> {
    let payload = payload_object(value).ok_or_else(|| {
        error(
            CodexStructureErrorKind::MalformedSessionMetadata,
            "Codex turn_context row has no payload",
        )
    })?;
    optional_text(
        payload,
        &["cwd", "working_directory", "workingDirectory"],
        "turn_context cwd",
        4096,
    )
}

fn optional_text(
    object: &Map<String, Value>,
    keys: &[&str],
    label: &str,
    max: usize,
) -> Result<Option<String>, CodexStructureError> {
    let Some(value) = keys.iter().find_map(|key| object.get(*key)) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| malformed_meta(format!("has a non-string {label}")))?;
    validate_text(label, value, max).map(Some)
}

fn validate_text(label: &str, value: &str, max: usize) -> Result<String, CodexStructureError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(malformed_meta(format!("has an invalid {label}")))
    } else {
        Ok(value.to_string())
    }
}

fn payload_object(value: &Value) -> Option<&Map<String, Value>> {
    value.get("payload").and_then(Value::as_object)
}

fn string_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn malformed_meta(detail: impl std::fmt::Display) -> CodexStructureError {
    error(
        CodexStructureErrorKind::MalformedSessionMetadata,
        format!("Codex session_meta {detail}"),
    )
}

fn error(kind: CodexStructureErrorKind, message: impl Into<String>) -> CodexStructureError {
    CodexStructureError {
        kind,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rollout(records: Vec<Value>) -> Vec<u8> {
        let mut bytes = records
            .into_iter()
            .map(|record| serde_json::to_string(&record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');
        bytes
    }

    fn meta(id: &str, parent: Option<&str>, cwd: &str) -> Value {
        json!({
            "type": "session_meta",
            "payload": {
                "id": id,
                "forked_from_id": parent,
                "cwd": cwd,
            },
        })
    }

    #[test]
    fn accepts_root_and_truncated_parent() {
        let root = analyze_codex_rollout_structure(
            &rollout(vec![meta("p", None, "/tmp/project")]),
            Some("p"),
        )
        .unwrap();
        assert!(root.fork_ancestry.is_empty());

        let truncated = analyze_codex_rollout_structure(
            &rollout(vec![meta("p1", Some("p"), "/tmp/project")]),
            Some("p1"),
        )
        .unwrap();
        assert_eq!(truncated.fork_ancestry, vec!["p"]);
        assert!(truncated.accepted_metadata_ids.contains("p"));
    }

    #[test]
    fn recognizes_internal_subagent_metadata_variants() {
        for source in [
            json!({"thread_source": "subagent"}),
            json!({"source": {"subagent": {"other": "guardian"}}}),
        ] {
            let mut record = meta("child", None, "/tmp/project");
            let payload = record["payload"].as_object_mut().unwrap();
            payload.extend(source.as_object().unwrap().clone());
            let structure =
                analyze_codex_rollout_structure(&rollout(vec![record]), Some("child")).unwrap();
            assert!(structure.current.is_internal_subagent);
        }

        let regular = analyze_codex_rollout_structure(
            &rollout(vec![meta("parent", None, "/tmp/project")]),
            None,
        )
        .unwrap();
        assert!(!regular.current.is_internal_subagent);
    }

    #[test]
    fn accepts_repeated_three_level_fork_history() {
        let mut records = Vec::new();
        for _ in 0..2 {
            records.extend([
                meta("p111", Some("p11"), "/tmp/new"),
                meta("p11", Some("p1"), "/tmp/old"),
                meta("p1", Some("p"), "/tmp/old"),
                meta("p", None, "/tmp/old"),
            ]);
        }
        let structure = analyze_codex_rollout_structure(&rollout(records), Some("p111")).unwrap();
        assert_eq!(structure.fork_ancestry, vec!["p11", "p1", "p"]);
        assert_eq!(structure.accepted_metadata_ids.len(), 4);
        assert_eq!(
            structure.structural_cwds,
            BTreeSet::from(["/tmp/new".to_string(), "/tmp/old".to_string()])
        );
    }

    #[test]
    fn accepts_large_jsonl_records_within_the_transcript_limit() {
        let padding = "x".repeat(2 * 1024 * 1024);
        let bytes = rollout(vec![
            meta("p", None, "/tmp/project"),
            json!({
                "type": "compacted",
                "payload": {
                    "message": "",
                    "replacement_history": padding.clone(),
                },
            }),
            json!({
                "type": "turn_context",
                "payload": {
                    "cwd": "/tmp/project",
                    "large_non_structural_field": padding,
                },
            }),
        ]);

        let structure = analyze_codex_rollout_structure(&bytes, Some("p")).unwrap();
        assert_eq!(structure.record_count, 3);
        assert_eq!(
            structure.structural_cwds,
            BTreeSet::from(["/tmp/project".to_string()])
        );
    }

    #[test]
    fn rejects_unrelated_conflicting_self_and_cycles() {
        let unrelated = analyze_codex_rollout_structure(
            &rollout(vec![
                meta("p1", Some("p"), "/tmp/project"),
                meta("x", None, "/tmp/project"),
            ]),
            Some("p1"),
        )
        .unwrap_err();
        assert_eq!(unrelated.kind, CodexStructureErrorKind::UnrelatedMetadata);

        let conflicting = analyze_codex_rollout_structure(
            &rollout(vec![
                meta("p1", Some("p"), "/tmp/project"),
                meta("p1", Some("x"), "/tmp/project"),
            ]),
            Some("p1"),
        )
        .unwrap_err();
        assert_eq!(conflicting.kind, CodexStructureErrorKind::ConflictingParent);

        let self_parent = analyze_codex_rollout_structure(
            &rollout(vec![meta("p", Some("p"), "/tmp/project")]),
            Some("p"),
        )
        .unwrap_err();
        assert_eq!(self_parent.kind, CodexStructureErrorKind::SelfParent);

        let cycle = analyze_codex_rollout_structure(
            &rollout(vec![
                meta("p1", Some("p"), "/tmp/project"),
                meta("p", Some("p1"), "/tmp/project"),
            ]),
            Some("p1"),
        )
        .unwrap_err();
        assert_eq!(cycle.kind, CodexStructureErrorKind::AncestryCycle);
    }

    #[test]
    fn authoritative_identity_is_only_the_first_session_meta() {
        let mismatch = analyze_codex_rollout_structure(
            &rollout(vec![
                meta("child", Some("parent"), "/tmp/project"),
                meta("parent", None, "/tmp/project"),
            ]),
            Some("parent"),
        )
        .unwrap_err();
        assert_eq!(
            mismatch.kind,
            CodexStructureErrorKind::ActiveIdentityMismatch
        );
    }

    #[test]
    fn rejects_conversation_items_before_authoritative_metadata() {
        let error = analyze_codex_rollout_structure(
            &rollout(vec![
                json!({
                    "type": "response_item",
                    "payload": {"type": "message", "id": "m-a"}
                }),
                meta("p", None, "/tmp/project"),
            ]),
            Some("p"),
        )
        .unwrap_err();
        assert_eq!(error.kind, CodexStructureErrorKind::MissingSessionMetadata);
    }
}
