//! Shared child selection and immutable installation evidence.

use mallard_domain::{SubagentContentState, SubagentKey, SubagentSelection};
use std::collections::{BTreeMap, BTreeSet};

pub type SourceSnapshots = BTreeMap<
    std::path::PathBuf,
    (
        crate::payload::PayloadBytes,
        crate::jsonl_tail_io::SourceStamp,
    ),
>;

pub const PUSH_CHANGED: &str =
    "This task changed during Push. Push it again to include the latest work.";

/// The task was edited after it was last pushed or pulled: an earlier
/// message is no longer what it was. Pushing again cannot succeed; the next
/// comparison reads the task in full and shows how it stands.
pub const PUSH_EDITED: &str =
    "Earlier messages in this task changed on this computer. Refresh to compare it again.";

/// Prove the cloud prefix using already parsed local and wire chains.
pub fn adopt_chain_prefix(
    local: &[mallard_domain::CanonicalConversationMessage],
    wire: &[mallard_domain::CanonicalConversationMessage],
    cloud: &SubagentContentState,
) -> Option<mallard_domain::AcceptedSubagentState> {
    use mallard_domain::{compute_conversation_tail_state, TailPartitionKind};
    if local.len() != wire.len() || cloud.message_count > local.len() as u64 {
        return None;
    }
    let state = |chain: &[mallard_domain::CanonicalConversationMessage]| {
        compute_conversation_tail_state(
            TailPartitionKind::WholeThread,
            0,
            cloud.message_count,
            chain,
        )
        .ok()
    };
    if state(wire)? != cloud.sha256 {
        return None;
    }
    Some(mallard_domain::AcceptedSubagentState {
        local: SubagentContentState {
            message_count: cloud.message_count,
            sha256: state(local)?,
        },
        normalized: cloud.clone(),
    })
}

pub enum ChildReferences {
    Codex(crate::codex_subagents::CreationRecords),
    Claude(crate::providers::claude::subagents::ChildLinks),
}

impl ChildReferences {
    /// The provider IDs of the children the main refers to whose
    /// transcripts are not among the installed ones.
    pub fn absent(
        &self,
        installed: &BTreeMap<SubagentKey, mallard_domain::InstalledSubagent>,
    ) -> BTreeSet<String> {
        match self {
            // A Codex rollout is never rewritten: its cutoff tells them apart.
            Self::Codex(_) => BTreeSet::new(),
            Self::Claude(links) => links
                .iter()
                .filter(|(_, key)| !installed.contains_key(key))
                .map(|(id, _)| id.clone())
                .collect(),
        }
    }

    pub fn prepare(
        &self,
        children: &BTreeMap<SubagentKey, (Vec<u8>, SubagentContentState)>,
        identities: &BTreeMap<String, String>,
        source_parent: &str,
        destination_parent: &str,
        root: &str,
        canonical_root: Option<&str>,
        redaction: &mut crate::credential_redaction::RedactionRun<'_>,
    ) -> Result<Vec<InstalledChildPayload>, String> {
        match self {
            Self::Codex(references) => crate::codex_subagents::prepare_child_payloads(
                references.clone(),
                children,
                identities,
                root,
                canonical_root,
                redaction,
            ),
            Self::Claude(references) => {
                crate::providers::claude::subagents::prepare_child_payloads(
                    references,
                    children,
                    source_parent,
                    destination_parent,
                    root,
                    canonical_root,
                    redaction,
                )
            }
        }
    }
}

pub fn verify_reviewed_state(
    observed: &SubagentContentState,
    current: &SubagentContentState,
) -> Result<(), String> {
    let pins = mallard_domain::SubagentTailPins {
        accepted: Some(mallard_domain::AcceptedSubagentState {
            local: observed.clone(),
            normalized: observed.clone(),
        }),
        local: Some(current.clone()),
        cloud: Some(observed.clone()),
    };
    if !matches!(
        mallard_domain::compare_subagent_tail(&pins),
        Ok(mallard_domain::TailClassification::Sync(_)
            | mallard_domain::TailClassification::LocalOutstanding)
    ) {
        return Err(PUSH_CHANGED.into());
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum TreeObservation {
    Codex(crate::codex_subagents::TreeObservation),
    Claude(crate::providers::claude::subagents::TreeObservation),
}

impl TreeObservation {
    pub fn codex(&self) -> Option<&crate::codex_subagents::TreeObservation> {
        match self {
            Self::Codex(tree) => Some(tree),
            _ => None,
        }
    }
    pub fn claude(&self) -> Option<&crate::providers::claude::subagents::TreeObservation> {
        match self {
            Self::Claude(tree) => Some(tree),
            _ => None,
        }
    }
    pub fn selection(&self) -> SubagentSelection {
        match self {
            Self::Codex(tree) => tree.selection(),
            Self::Claude(tree) => tree.selection(),
        }
    }
    pub fn omitted(&self) -> bool {
        match self {
            Self::Codex(tree) => tree.omitted,
            Self::Claude(tree) => tree.omitted,
        }
    }
    pub fn missing(&self) -> &BTreeSet<SubagentKey> {
        match self {
            Self::Codex(tree) => &tree.missing,
            Self::Claude(tree) => &tree.missing,
        }
    }
    pub fn states(&self) -> impl Iterator<Item = (&SubagentKey, &SubagentContentState)> {
        self.codex()
            .into_iter()
            .flat_map(|tree| tree.children.iter().map(|(key, child)| (key, &child.state)))
            .chain(
                self.claude()
                    .into_iter()
                    .flat_map(|tree| tree.children.iter().map(|(key, child)| (key, &child.state))),
            )
    }
}

/// Verified child output, retained through Apply. No fields can be replaced by
/// callers; the content state and ownership belong to these immutable bytes.
#[derive(Clone, Debug)]
pub struct InstalledChildPayload {
    pub(crate) key: SubagentKey,
    pub(crate) provider_thread_id: String,
    pub(crate) parent_provider_thread_id: String,
    pub(crate) transcript: crate::verified_transcript::VerifiedTranscript,
    pub(crate) accepted: mallard_domain::AcceptedSubagentState,
    pub(crate) origins: crate::session_path_origins::PathOrigins,
}

impl InstalledChildPayload {
    pub fn key(&self) -> &SubagentKey {
        &self.key
    }
    pub fn provider_thread_id(&self) -> &str {
        &self.provider_thread_id
    }
    pub fn bytes(&self) -> &crate::payload::PayloadBytes {
        self.transcript.bytes()
    }
    pub fn accepted(&self) -> &mallard_domain::AcceptedSubagentState {
        &self.accepted
    }
    pub fn origins(&self) -> &crate::session_path_origins::PathOrigins {
        &self.origins
    }

    pub(crate) fn matches_installation(
        &self,
        provider: &mallard_domain::ProviderKey,
        installed: &mallard_domain::InstalledSubagent,
        owner: &str,
        accepted: &mallard_domain::AcceptedSubagentState,
        project_root: &str,
        canonical_project_root: &str,
        bytes: &crate::payload::PayloadBytes,
    ) -> bool {
        self.parent_provider_thread_id == owner
            && &self.accepted == accepted
            && installed.installed_end == bytes.len() as u64
            && if provider.as_str() == "claude" {
                self.transcript.matches_child_identity_and_bytes(
                    provider,
                    &installed.provider_thread_id,
                    bytes,
                )
            } else {
                self.transcript.matches_identity_and_bytes(
                    provider,
                    Some(&installed.provider_thread_id),
                    project_root,
                    canonical_project_root,
                    bytes,
                )
            }
    }
}

/// What the tests of the agent cards of a pulled session are made of.
#[cfg(test)]
pub(crate) mod agent_card_fixtures {
    use crate::providers::traits::{
        SessionAgentIndexEntry, SessionCount, SessionCountQuality, SessionEventCounts,
        SessionSourceSpan, SessionTimelineEventKind, SessionTimelineIndex,
        SessionTimelineIndexEntry, SessionTimelineSource, SessionToolCallStatus,
        SessionToolCategory, SessionToolIndexEntry,
    };
    use mallard_domain::{InstalledSubagent, LocalConversationBinding, SubagentKey};

    /// An agent call of the turn `turn-1`: its event ID, the child that it
    /// names, and its place.
    pub(crate) fn agent_call(
        event_id: &str,
        agent: Option<&str>,
        offset: u64,
    ) -> SessionTimelineIndexEntry {
        SessionTimelineIndexEntry {
            event_id: event_id.to_string(),
            ordinal: 0,
            timestamp: None,
            kind: SessionTimelineEventKind::AgentCall,
            quality: SessionCountQuality::Exact,
            turn_id: Some("turn-1".to_string()),
            turn_tool_summary: None,
            parent_event_id: None,
            source: SessionSourceSpan { offset, length: 4 },
            tool: Some(SessionToolIndexEntry {
                provider_call_id: event_id.to_string(),
                native_name: "agent".to_string(),
                category: SessionToolCategory::Other,
                status: SessionToolCallStatus::Completed,
                affected_paths: Vec::new(),
                result_source: None,
            }),
            agent: Some(SessionAgentIndexEntry {
                provider_agent_id: agent.map(ToOwned::to_owned),
                ..Default::default()
            }),
        }
    }

    /// An index of agent calls over sources of the given lengths.
    pub(crate) fn index_of_agent_calls(
        sources: &[u64],
        calls: &[(&str, Option<&str>, u64)],
    ) -> SessionTimelineIndex {
        let none = || SessionCount::exact(0);
        SessionTimelineIndex {
            sources: sources
                .iter()
                .enumerate()
                .map(|(position, length)| SessionTimelineSource {
                    relative_path: format!("sessions/rollout-{position}.jsonl"),
                    length: *length,
                })
                .collect(),
            parser_version: 1,
            events: calls
                .iter()
                .enumerate()
                .map(
                    |(ordinal, (event_id, agent, offset))| SessionTimelineIndexEntry {
                        ordinal: ordinal as u64,
                        ..agent_call(event_id, *agent, *offset)
                    },
                )
                .collect(),
            counts: SessionEventCounts {
                prompts: none(),
                responses: none(),
                intermediate_steps: none(),
                checkpoints: none(),
                tool_calls: none(),
                file_edits: none(),
                shell_calls: none(),
                reads: none(),
                other_tools: none(),
                agent_calls: SessionCount::exact(calls.len() as u64),
            },
            warnings: Vec::new(),
            truncated: false,
        }
    }

    /// The binding of a session: what a Pull installed of it, or nothing
    /// for a session that was made on this computer. `installed` pairs a
    /// child key with the provider ID of the child.
    pub(crate) fn binding_of_a_pulled_session(
        installed_end: Option<u64>,
        installed: &[(&str, &str)],
        absent: &[&str],
    ) -> LocalConversationBinding {
        LocalConversationBinding {
            installed_end,
            installed_absent_subagents: absent.iter().map(ToString::to_string).collect(),
            pulled_children_excluded: false,
            installed_subagents: installed
                .iter()
                .map(|(creation, id)| {
                    (
                        SubagentKey::child(None, creation).unwrap(),
                        InstalledSubagent {
                            provider_thread_id: id.to_string(),
                            installed_end: 1,
                        },
                    )
                })
                .collect(),
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
        }
    }
}
