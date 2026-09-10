//! Owned child identities and compact content evidence.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    deserialize_required_nullable, CloudThreadId, LogicalPath, PortableConversationTail,
    ProviderKey, TailPartitionKind, CLAUDE_PORTABLE_TAIL_CODEC_V1,
    CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1, MAX_CONVERSATION_FILES, MAX_CONVERSATION_FILE_BYTES,
    MAX_LOGICAL_PATH_BYTES, PORTABLE_TAIL_CODEC_V1, PORTABLE_TAIL_CODEC_WIRE_V1,
};

pub const SUBAGENT_TAIL_CODEC_V1: &str = "mallard-subagent-tail-v1";
pub const SUBAGENT_TAIL_CODEC_WIRE_V1: &str = "mallard-subagent-tail-wire-v1";

/// Creation IDs scoped by the chain of owning children, inside one cloud task.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct SubagentKey(String);

/// Export scope of a cloud task. Required on the wire, including empty exports.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConversationWireChildren {
    #[default]
    On,
    Off,
}

/// Temporary review scope. Sizes are estimates, never source byte pins.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct SubagentSelection {
    pub wire_children: ConversationWireChildren,
    pub keys: BTreeSet<SubagentKey>,
    pub omitted: bool,
    pub estimated_bytes: u64,
}

impl SubagentSelection {
    pub fn validate(&self) -> Result<(), String> {
        if self.keys.len() >= MAX_CONVERSATION_FILES
            || (self.omitted && (!self.keys.is_empty() || self.estimated_bytes != 0))
            || (self.wire_children == ConversationWireChildren::Off && !self.omitted)
        {
            return Err("invalid outgoing child selection".into());
        }
        Ok(())
    }
}

impl SubagentKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() > MAX_LOGICAL_PATH_BYTES || !value.split('/').all(valid_creation_id) {
            return Err("invalid subagent creation key".to_string());
        }
        // Apply the shared portable-name rules, including reserved names.
        LogicalPath::parse(format!("state/conversations/codex/children/{value}.jsonl"))?;
        Ok(Self(value))
    }

    pub fn child(owner: Option<&Self>, creation_id: &str) -> Result<Self, String> {
        if !valid_creation_id(creation_id) {
            return Err("invalid subagent creation ID".to_string());
        }
        Self::parse(match owner {
            Some(owner) => format!("{owner}/{creation_id}"),
            None => creation_id.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn owner(&self) -> Option<Self> {
        self.0
            .rsplit_once('/')
            .map(|(owner, _)| Self(owner.to_string()))
    }
}

fn valid_creation_id(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

impl TryFrom<String> for SubagentKey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<SubagentKey> for String {
    fn from(value: SubagentKey) -> Self {
        value.0
    }
}

impl fmt::Display for SubagentKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub fn cloud_subagent_transcript_path(
    provider: &ProviderKey,
    thread_id: &CloudThreadId,
    child_key: &SubagentKey,
) -> Result<LogicalPath, String> {
    LogicalPath::parse(format!(
        "state/conversations/{provider}/{thread_id}/subagents/{child_key}.jsonl"
    ))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct SubagentContentState {
    pub message_count: u64,
    pub sha256: String,
}

impl SubagentContentState {
    pub fn validate(&self) -> Result<(), String> {
        self.normalized_tail().validate()
    }

    pub fn from_tail(tail: &PortableConversationTail) -> Result<Self, String> {
        tail.validate()?;
        if !matches!(
            tail.codec.as_str(),
            PORTABLE_TAIL_CODEC_V1
                | PORTABLE_TAIL_CODEC_WIRE_V1
                | CLAUDE_PORTABLE_TAIL_CODEC_V1
                | CLAUDE_PORTABLE_TAIL_CODEC_WIRE_V1
                | SUBAGENT_TAIL_CODEC_V1
                | SUBAGENT_TAIL_CODEC_WIRE_V1
        ) || tail.partition_kind != TailPartitionKind::WholeThread
            || tail.start_record_count != 0
        {
            return Err("subagent evidence requires a whole conversation".to_string());
        }
        Ok(Self {
            message_count: tail.accepted_end_record_count,
            sha256: tail.accepted_tail_state.clone(),
        })
    }

    pub fn local_tail(&self) -> PortableConversationTail {
        self.tail(SUBAGENT_TAIL_CODEC_V1)
    }

    pub fn normalized_tail(&self) -> PortableConversationTail {
        self.tail(SUBAGENT_TAIL_CODEC_WIRE_V1)
    }

    fn tail(&self, codec: &str) -> PortableConversationTail {
        PortableConversationTail {
            codec: codec.to_string(),
            partition_kind: TailPartitionKind::WholeThread,
            start_record_count: 0,
            accepted_end_record_count: self.message_count,
            accepted_tail_state: self.sha256.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcceptedSubagentState {
    pub local: SubagentContentState,
    pub normalized: SubagentContentState,
}

impl AcceptedSubagentState {
    pub fn validate(&self) -> Result<(), String> {
        self.local.validate()?;
        self.normalized.validate()?;
        if self.local.message_count != self.normalized.message_count {
            return Err("subagent accepted forms have different message counts".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstalledSubagent {
    pub provider_thread_id: String,
    pub installed_end: u64,
}

impl InstalledSubagent {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider_thread_id.is_empty()
            || self.provider_thread_id.len() > 128
            || self.provider_thread_id.chars().any(char::is_control)
            || self.installed_end == 0
            || self.installed_end > MAX_CONVERSATION_FILE_BYTES
        {
            return Err("invalid installed subagent identity or cutoff".to_string());
        }
        Ok(())
    }
}

/// Temporary action evidence. Missing current files must be rejected by the
/// observer before it constructs a confirmed-absence value here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubagentTailPins {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub accepted: Option<AcceptedSubagentState>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub local: Option<SubagentContentState>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub cloud: Option<SubagentContentState>,
}

pub(crate) fn validate_subagent_map<T>(map: &BTreeMap<SubagentKey, T>) -> Result<(), String> {
    if map.len() >= MAX_CONVERSATION_FILES {
        return Err("subagent inventory exceeds the conversation file limit".to_string());
    }
    for key in map.keys() {
        if key.owner().is_some_and(|owner| !map.contains_key(&owner)) {
            return Err("subagent inventory is missing an owning child".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_keys_preserve_case_and_owner_scope() {
        let child = SubagentKey::child(None, "start-D_1").unwrap();
        let nested = SubagentKey::child(Some(&child), "start-E").unwrap();
        assert_eq!(nested.as_str(), "start-D_1/start-E");
        assert_eq!(nested.owner(), Some(child));
        assert_eq!(
            serde_json::from_str::<SubagentKey>("\"start-D_1/start-E\"").unwrap(),
            nested
        );
        for invalid in [
            "",
            "../worker",
            "worker/",
            "/worker",
            "a//b",
            "a.b",
            "a:b",
            "a\\b",
            "a b",
            "é",
        ] {
            assert!(SubagentKey::parse(invalid).is_err(), "{invalid:?}");
        }
        assert!(SubagentKey::child(None, "a/b").is_err());
        assert!(SubagentKey::parse("x".repeat(257)).is_err());
    }

    #[test]
    fn child_evidence_uses_existing_dual_form_tail_layout() {
        let state = SubagentContentState {
            message_count: 4,
            sha256: "a".repeat(64),
        };
        assert_ne!(state.local_tail(), state.normalized_tail());
        assert_eq!(
            SubagentContentState::from_tail(&state.local_tail()).unwrap(),
            state
        );
        let mut tail = state.local_tail();
        tail.partition_kind = TailPartitionKind::FixedPaginatedTail;
        assert!(SubagentContentState::from_tail(&tail).is_err());
        let pins = serde_json::json!({"accepted": null, "local": null, "cloud": null});
        assert!(serde_json::from_value::<SubagentTailPins>(pins.clone()).is_ok());
        let mut missing = pins;
        missing.as_object_mut().unwrap().remove("accepted");
        assert!(serde_json::from_value::<SubagentTailPins>(missing).is_err());
    }
}
