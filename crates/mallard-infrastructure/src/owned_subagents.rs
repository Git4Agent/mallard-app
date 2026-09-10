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
    pub fn prepare(
        self,
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
                references,
                children,
                identities,
                root,
                canonical_root,
                redaction,
            ),
            Self::Claude(references) => {
                crate::providers::claude::subagents::prepare_child_payloads(
                    &references,
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
