//! Pure conversation-thread reconciliation and reviewed-plan pinning.
//!
//! Filesystem, Tauri, and object-store code call these functions rather than
//! reimplementing relation-to-action policy. This keeps Push, Pull, recovery,
//! and the integration harness on one state machine.

use std::collections::BTreeMap;

use super::domain::{
    classify_conversation_relation, ActionId, CanonicalConversationMessage, ConversationBindings,
    ConversationPlanAction, ConversationPlanActionType, ConversationPlanDirection,
    ConversationReasonCode, ConversationRelation, HostId, LocalConversationBinding, LocalProjectId,
    LocalProviderProfileId, Provider, ProviderStateExpectation, ReplicaId, ResourceId,
    TargetExpectation, ThreadId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingResolution {
    Bound,
    UniqueCompatibleAncestor,
    Missing,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationCapabilities {
    pub binding: BindingResolution,
    pub local_safely_capturable: bool,
    pub cloud_safely_readable: bool,
    pub destination_codec_resumable: bool,
    pub provider_quiescent: bool,
    /// True only when raw bytes differ solely through fields the verified
    /// codec declares structural (thread ID and mapped cwd).
    pub structural_rewrite_only: bool,
}

impl Default for ConversationCapabilities {
    fn default() -> Self {
        Self {
            binding: BindingResolution::Bound,
            local_safely_capturable: true,
            cloud_safely_readable: true,
            destination_codec_resumable: true,
            provider_quiescent: true,
            structural_rewrite_only: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationDecision {
    pub action: ConversationPlanActionType,
    pub reason_code: ConversationReasonCode,
    pub selectable: bool,
    pub selected_by_default: bool,
    pub requires_explicit_approval: bool,
    pub blocked_reason: Option<String>,
}

/// Map a proven relation to one typed default action. Fork is always the
/// divergence default; the explicit forces (`ReplaceCloud`, `ReplaceLocal`)
/// and `TombstoneThread` are never synthesized here; callers add those only
/// from reviewed user intent.
pub fn default_conversation_decision(
    direction: ConversationPlanDirection,
    relation: ConversationRelation,
    capabilities: ConversationCapabilities,
) -> ConversationDecision {
    use ConversationPlanActionType as Action;
    use ConversationReasonCode as Reason;
    use ConversationRelation as Relation;

    if relation == Relation::Identical && capabilities.structural_rewrite_only {
        return decision(Action::NoOp, Reason::StructuralRewriteOnly);
    }

    match direction {
        ConversationPlanDirection::Push => match relation {
            Relation::Identical => decision(Action::NoOp, Reason::Identical),
            Relation::CloudPrefixOfLocal => {
                if !capabilities.local_safely_capturable {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                } else if matches!(
                    capabilities.binding,
                    BindingResolution::Missing | BindingResolution::Ambiguous
                ) {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else {
                            Reason::MissingBinding
                        },
                    )
                } else {
                    decision(Action::FastForwardCloud, Reason::CloudPrefixOfLocal)
                }
            }
            Relation::LocalPrefixOfCloud => {
                decision(Action::RequirePull, Reason::LocalPrefixOfCloud)
            }
            Relation::Diverged => {
                if !capabilities.local_safely_capturable {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                } else if matches!(
                    capabilities.binding,
                    BindingResolution::Bound | BindingResolution::UniqueCompatibleAncestor
                ) {
                    decision(Action::Fork, Reason::Diverged)
                } else {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else {
                            Reason::MissingBinding
                        },
                    )
                }
            }
            Relation::Unknown => {
                if capabilities.local_safely_capturable
                    && capabilities.binding == BindingResolution::Bound
                {
                    // Content is preserved on both sides, so forking under
                    // unproven ancestry is safe by construction.
                    decision(Action::Fork, Reason::UnknownLineage)
                } else {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else if !capabilities.local_safely_capturable {
                            Reason::CodecUnsupported
                        } else {
                            Reason::MissingBinding
                        },
                    )
                }
            }
            Relation::LocalMissing => decision(Action::NoOp, Reason::LocalMissing),
            Relation::CloudMissing => {
                if capabilities.binding == BindingResolution::Ambiguous {
                    decision(Action::Blocked, Reason::AmbiguousBinding)
                } else if capabilities.local_safely_capturable {
                    decision(Action::CreateCloud, Reason::CloudMissing)
                } else {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                }
            }
            Relation::BothMissing => decision(Action::NoOp, Reason::BothMissing),
        },
        ConversationPlanDirection::Pull => match relation {
            Relation::Identical => decision(Action::NoOp, Reason::Identical),
            Relation::CloudPrefixOfLocal => decision(Action::KeepLocal, Reason::CloudPrefixOfLocal),
            Relation::LocalPrefixOfCloud => {
                if !capabilities.cloud_safely_readable || !capabilities.destination_codec_resumable
                {
                    decision(Action::StoreOnly, Reason::CodecUnsupported)
                } else if !capabilities.provider_quiescent {
                    decision(Action::StoreOnly, Reason::ProviderNotQuiescent)
                } else if matches!(
                    capabilities.binding,
                    BindingResolution::Missing | BindingResolution::Ambiguous
                ) {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else {
                            Reason::MissingBinding
                        },
                    )
                } else {
                    decision(Action::FastForwardLocal, Reason::LocalPrefixOfCloud)
                }
            }
            Relation::Diverged => {
                if !capabilities.cloud_safely_readable || !capabilities.destination_codec_resumable
                {
                    decision(Action::StoreOnly, Reason::CodecUnsupported)
                } else if matches!(
                    capabilities.binding,
                    BindingResolution::Bound | BindingResolution::UniqueCompatibleAncestor
                ) {
                    decision(Action::Fork, Reason::Diverged)
                } else {
                    decision(
                        Action::Blocked,
                        if capabilities.binding == BindingResolution::Ambiguous {
                            Reason::AmbiguousBinding
                        } else {
                            Reason::MissingBinding
                        },
                    )
                }
            }
            Relation::Unknown => decision(Action::StoreOnly, Reason::UnknownLineage),
            Relation::LocalMissing => {
                if capabilities.cloud_safely_readable && capabilities.destination_codec_resumable {
                    decision(Action::CreateLocal, Reason::LocalMissing)
                } else {
                    decision(Action::StoreOnly, Reason::CodecUnsupported)
                }
            }
            Relation::CloudMissing => decision(Action::KeepLocal, Reason::CloudMissing),
            Relation::BothMissing => decision(Action::NoOp, Reason::BothMissing),
        },
    }
}

fn decision(
    action: ConversationPlanActionType,
    reason_code: ConversationReasonCode,
) -> ConversationDecision {
    let requires_explicit_approval = matches!(
        action,
        ConversationPlanActionType::ReplaceLocal
            | ConversationPlanActionType::ReplaceCloud
            | ConversationPlanActionType::TombstoneThread
    );
    let selectable = matches!(
        action,
        ConversationPlanActionType::FastForwardLocal
            | ConversationPlanActionType::FastForwardCloud
            | ConversationPlanActionType::Fork
            | ConversationPlanActionType::CreateCloud
            | ConversationPlanActionType::CreateLocal
            | ConversationPlanActionType::ReplaceLocal
            | ConversationPlanActionType::ReplaceCloud
            | ConversationPlanActionType::TombstoneThread
    );
    let selected_by_default = selectable && !requires_explicit_approval;
    ConversationDecision {
        action,
        reason_code,
        selectable,
        selected_by_default,
        requires_explicit_approval,
        blocked_reason: (action == ConversationPlanActionType::Blocked)
            .then(|| blocked_reason(reason_code).to_string()),
    }
}

fn blocked_reason(reason: ConversationReasonCode) -> &'static str {
    match reason {
        ConversationReasonCode::ProviderNotQuiescent => {
            "The provider may still be writing this conversation."
        }
        ConversationReasonCode::CodecUnsupported => {
            "This conversation cannot be transformed safely by the active codec."
        }
        ConversationReasonCode::AmbiguousBinding => {
            "More than one local-to-cloud conversation binding is compatible."
        }
        ConversationReasonCode::MissingBinding => {
            "No verified local-to-cloud conversation binding exists."
        }
        ConversationReasonCode::StalePlan => {
            "The reviewed conversation state changed. Refresh before applying."
        }
        _ => "This conversation cannot be changed safely.",
    }
}

/// Immutable inputs used to build one reviewed conversation action. Keeping
/// these values together makes it difficult for Push, Pull, or a test facade
/// to omit one of the concurrency pins.
#[derive(Clone, Debug)]
pub struct ConversationPlanActionRequest<'a> {
    pub action_id: ActionId,
    pub direction: ConversationPlanDirection,
    pub local_chain: Option<&'a [CanonicalConversationMessage]>,
    pub cloud_chain: Option<&'a [CanonicalConversationMessage]>,
    pub capabilities: ConversationCapabilities,
    pub provider: Provider,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub thread_id: ThreadId,
    pub result_thread_id: Option<ThreadId>,
    pub source_resource_id: Option<ResourceId>,
    pub proposed_provider_thread_id: Option<String>,
    pub required_codec: String,
    pub pinned_generation: u64,
    pub pinned_commit_id: String,
    pub pinned_manifest_sha256: String,
    pub pinned_lineage_digest: Option<String>,
    pub pinned_semantic_digest: Option<String>,
    pub expected_local_target: TargetExpectation,
    pub expected_provider_state: Vec<ProviderStateExpectation>,
    pub expected_profile_revision: u64,
    pub expected_project_binding_revision: u64,
    pub expected_host_identity_revision: u64,
    pub expected_bindings_document_revision: u64,
    pub expected_binding_revision: Option<u64>,
    pub expires_at: u64,
}

/// Build the exact typed action that is serialized into a reviewed plan.
/// This function is pure: ID reservations and target expectations are copied
/// into the plan, but provider, binding, and cloud state are never changed.
pub fn build_conversation_plan_action(
    request: ConversationPlanActionRequest<'_>,
) -> Result<ConversationPlanAction, String> {
    let relation = classify_conversation_relation(request.local_chain, request.cloud_chain);
    let decision = default_conversation_decision(request.direction, relation, request.capabilities);
    let action = ConversationPlanAction {
        action_id: request.action_id,
        direction: request.direction,
        action: decision.action,
        reason_code: decision.reason_code,
        relation,
        provider: request.provider,
        host_id: request.host_id,
        replica_id: request.replica_id,
        local_project_id: request.local_project_id,
        provider_profile_id: request.provider_profile_id,
        thread_id: request.thread_id,
        result_thread_id: request.result_thread_id,
        source_resource_id: request.source_resource_id,
        proposed_provider_thread_id: request.proposed_provider_thread_id,
        required_codec: request.required_codec,
        pinned_generation: request.pinned_generation,
        pinned_commit_id: request.pinned_commit_id,
        pinned_manifest_sha256: request.pinned_manifest_sha256,
        pinned_lineage_digest: request.pinned_lineage_digest,
        pinned_semantic_digest: request.pinned_semantic_digest,
        expected_local_target: request.expected_local_target,
        expected_provider_state: request.expected_provider_state,
        expected_profile_revision: request.expected_profile_revision,
        expected_project_binding_revision: request.expected_project_binding_revision,
        expected_host_identity_revision: request.expected_host_identity_revision,
        expected_bindings_document_revision: request.expected_bindings_document_revision,
        expected_binding_revision: request.expected_binding_revision,
        expires_at: request.expires_at,
    };
    action.validate()?;
    Ok(action)
}

#[derive(Clone, Debug)]
pub struct BindingRecoveryCandidate<'a> {
    pub thread_id: ThreadId,
    pub chain: Option<&'a [CanonicalConversationMessage]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingRecoveryResult {
    Unique(ThreadId),
    Missing,
    Ambiguous,
    Unknown,
}

/// Recover a missing binding only from one unique longest compatible cloud
/// ancestor among all threads. Names, titles, and provider IDs are
/// intentionally absent from this API and therefore cannot influence the
/// result. Callers must skip conversations that are already bound: a
/// deliberately forked conversation is always bound to its minted thread and
/// must never be silently re-attached to a compatible ancestor.
pub fn recover_unique_compatible_binding(
    local_chain: Option<&[CanonicalConversationMessage]>,
    candidates: &[BindingRecoveryCandidate<'_>],
) -> BindingRecoveryResult {
    let Some(local_chain) = local_chain else {
        return BindingRecoveryResult::Unknown;
    };
    if candidates.iter().any(|candidate| candidate.chain.is_none()) {
        return BindingRecoveryResult::Unknown;
    }
    let compatible = candidates
        .iter()
        .filter_map(|candidate| {
            let chain = candidate.chain?;
            matches!(
                classify_conversation_relation(Some(local_chain), Some(chain)),
                ConversationRelation::Identical | ConversationRelation::CloudPrefixOfLocal
            )
            .then_some((chain.len(), candidate.thread_id.clone()))
        })
        .collect::<Vec<_>>();
    let Some(longest) = compatible.iter().map(|(length, _)| *length).max() else {
        return BindingRecoveryResult::Missing;
    };
    let mut longest_matches = compatible
        .into_iter()
        .filter(|(length, _)| *length == longest)
        .map(|(_, thread_id)| thread_id);
    let Some(result) = longest_matches.next() else {
        return BindingRecoveryResult::Missing;
    };
    if longest_matches.next().is_some() {
        BindingRecoveryResult::Ambiguous
    } else {
        BindingRecoveryResult::Unique(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationStalePin {
    Expired,
    Host,
    Replica,
    LocalProject,
    ProviderProfile,
    HostIdentityRevision,
    Generation,
    Commit,
    Manifest,
    LineageDigest,
    SemanticDigest,
    LocalTarget,
    ProviderState,
    ProfileRevision,
    ProjectBindingRevision,
    BindingsDocumentRevision,
    BindingRevision,
    Codec,
    ProviderQuiescence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationApplyContext {
    pub now: u64,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub host_identity_revision: u64,
    pub generation: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub lineage_digest: Option<String>,
    pub semantic_digest: Option<String>,
    pub local_target: TargetExpectation,
    pub provider_state: BTreeMap<String, TargetExpectation>,
    pub profile_revision: u64,
    pub project_binding_revision: u64,
    pub bindings_document_revision: u64,
    pub binding_revision: Option<u64>,
    pub codec: String,
    pub provider_quiescent: bool,
}

/// Revalidate every reviewed pin before a caller writes a journal record or
/// mutates provider/cloud state. The stable comparison order gives callers a
/// deterministic, typed rejection reason while still rejecting all changes.
pub fn validate_conversation_action_pins(
    action: &ConversationPlanAction,
    current: &ConversationApplyContext,
) -> Result<(), ConversationStalePin> {
    if current.now > action.expires_at {
        return Err(ConversationStalePin::Expired);
    }
    if current.host_id != action.host_id {
        return Err(ConversationStalePin::Host);
    }
    if current.replica_id != action.replica_id {
        return Err(ConversationStalePin::Replica);
    }
    if current.local_project_id != action.local_project_id {
        return Err(ConversationStalePin::LocalProject);
    }
    if current.provider_profile_id != action.provider_profile_id {
        return Err(ConversationStalePin::ProviderProfile);
    }
    if current.host_identity_revision != action.expected_host_identity_revision {
        return Err(ConversationStalePin::HostIdentityRevision);
    }
    if current.generation != action.pinned_generation {
        return Err(ConversationStalePin::Generation);
    }
    if current.commit_id != action.pinned_commit_id {
        return Err(ConversationStalePin::Commit);
    }
    if current.manifest_sha256 != action.pinned_manifest_sha256 {
        return Err(ConversationStalePin::Manifest);
    }
    if current.lineage_digest != action.pinned_lineage_digest {
        return Err(ConversationStalePin::LineageDigest);
    }
    if current.semantic_digest != action.pinned_semantic_digest {
        return Err(ConversationStalePin::SemanticDigest);
    }
    if current.local_target != action.expected_local_target {
        return Err(ConversationStalePin::LocalTarget);
    }
    let planned_provider_state = action
        .expected_provider_state
        .iter()
        .map(|target| (target.target_path.clone(), target.expectation.clone()))
        .collect::<BTreeMap<_, _>>();
    if current.provider_state != planned_provider_state {
        return Err(ConversationStalePin::ProviderState);
    }
    if current.profile_revision != action.expected_profile_revision {
        return Err(ConversationStalePin::ProfileRevision);
    }
    if current.project_binding_revision != action.expected_project_binding_revision {
        return Err(ConversationStalePin::ProjectBindingRevision);
    }
    if current.bindings_document_revision != action.expected_bindings_document_revision {
        return Err(ConversationStalePin::BindingsDocumentRevision);
    }
    if current.binding_revision != action.expected_binding_revision {
        return Err(ConversationStalePin::BindingRevision);
    }
    if current.codec != action.required_codec {
        return Err(ConversationStalePin::Codec);
    }
    if matches!(
        action.action,
        ConversationPlanActionType::FastForwardLocal | ConversationPlanActionType::ReplaceLocal
    ) && !current.provider_quiescent
    {
        return Err(ConversationStalePin::ProviderQuiescence);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationBindingScope {
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
}

/// Move one local conversation binding to a freshly minted thread. This is
/// the core fork primitive: identity converges while content never does.
/// Callers invoke it inside `LocalStateRepository::mutate_conversation_bindings`, so
/// the rekey is one atomic document replacement. The provider thread ID and
/// transcript are deliberately untouched, and the cloud source pins are
/// cleared because the minted thread has no published counterpart yet — the
/// journal's binding commit re-pins them when a publication completes.
pub fn rekey_thread_binding(
    document: &mut ConversationBindings,
    scope: &ConversationBindingScope,
    old_thread_id: &ThreadId,
    result_thread_id: &ThreadId,
    expected_document_revision: u64,
    expected_binding_revision: u64,
    updated_at: u64,
) -> Result<LocalConversationBinding, String> {
    if document.revision != expected_document_revision {
        return Err("conversation bindings changed after review".to_string());
    }
    if old_thread_id == result_thread_id {
        return Err("thread fork must mint a distinct thread id".to_string());
    }
    if document
        .bindings
        .iter()
        .any(|binding| binding_in_scope(binding, scope) && binding.thread_id == *result_thread_id)
    {
        return Err("fork result thread is already bound".to_string());
    }

    let binding_index = document
        .bindings
        .iter()
        .position(|binding| binding_in_scope(binding, scope) && binding.thread_id == *old_thread_id)
        .ok_or_else(|| "fork source binding disappeared".to_string())?;
    if document.bindings[binding_index].revision != expected_binding_revision {
        return Err("conversation binding changed after review".to_string());
    }

    let binding = &mut document.bindings[binding_index];
    binding.thread_id = result_thread_id.clone();
    binding.forked_from = Some(old_thread_id.clone());
    binding.source_generation = None;
    binding.source_manifest_sha256 = None;
    binding.revision = binding.revision.saturating_add(1);
    binding.updated_at = updated_at;
    Ok(binding.clone())
}

fn binding_in_scope(binding: &LocalConversationBinding, scope: &ConversationBindingScope) -> bool {
    binding.host_id == scope.host_id
        && binding.replica_id == scope.replica_id
        && binding.local_project_id == scope.local_project_id
        && binding.provider_profile_id == scope.provider_profile_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_policy_exposes_both_push_prefix_outcomes() {
        let capabilities = ConversationCapabilities::default();
        assert_eq!(
            default_conversation_decision(
                ConversationPlanDirection::Push,
                ConversationRelation::CloudPrefixOfLocal,
                capabilities,
            )
            .action,
            ConversationPlanActionType::FastForwardCloud
        );
        assert_eq!(
            default_conversation_decision(
                ConversationPlanDirection::Push,
                ConversationRelation::LocalPrefixOfCloud,
                capabilities,
            )
            .action,
            ConversationPlanActionType::RequirePull
        );
    }

    #[test]
    fn unknown_policy_never_guesses_a_missing_binding() {
        let bound = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::Unknown,
            ConversationCapabilities::default(),
        );
        assert_eq!(bound.action, ConversationPlanActionType::Fork);

        let missing = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::Unknown,
            ConversationCapabilities {
                binding: BindingResolution::Missing,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(missing.action, ConversationPlanActionType::Blocked);
        assert_eq!(missing.reason_code, ConversationReasonCode::MissingBinding);

        let pull = default_conversation_decision(
            ConversationPlanDirection::Pull,
            ConversationRelation::Unknown,
            ConversationCapabilities::default(),
        );
        assert_eq!(pull.action, ConversationPlanActionType::StoreOnly);
    }

    #[test]
    fn fork_is_the_divergence_default_in_both_directions() {
        let push = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::Diverged,
            ConversationCapabilities::default(),
        );
        assert_eq!(push.action, ConversationPlanActionType::Fork);
        let pull = default_conversation_decision(
            ConversationPlanDirection::Pull,
            ConversationRelation::Diverged,
            ConversationCapabilities::default(),
        );
        assert_eq!(pull.action, ConversationPlanActionType::Fork);
        let create = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::CloudMissing,
            ConversationCapabilities::default(),
        );
        assert_eq!(create.action, ConversationPlanActionType::CreateCloud);
        let materialize = default_conversation_decision(
            ConversationPlanDirection::Pull,
            ConversationRelation::LocalMissing,
            ConversationCapabilities::default(),
        );
        assert_eq!(materialize.action, ConversationPlanActionType::CreateLocal);
    }

    #[test]
    fn pull_mutations_require_one_verified_binding() {
        for relation in [
            ConversationRelation::LocalPrefixOfCloud,
            ConversationRelation::Diverged,
        ] {
            for (binding, reason) in [
                (
                    BindingResolution::Missing,
                    ConversationReasonCode::MissingBinding,
                ),
                (
                    BindingResolution::Ambiguous,
                    ConversationReasonCode::AmbiguousBinding,
                ),
            ] {
                let blocked = default_conversation_decision(
                    ConversationPlanDirection::Pull,
                    relation,
                    ConversationCapabilities {
                        binding,
                        ..ConversationCapabilities::default()
                    },
                );
                assert_eq!(blocked.action, ConversationPlanActionType::Blocked);
                assert_eq!(blocked.reason_code, reason);
                assert!(!blocked.selectable);
                assert!(!blocked.selected_by_default);
            }
        }

        for binding in [
            BindingResolution::Bound,
            BindingResolution::UniqueCompatibleAncestor,
        ] {
            let fast_forward = default_conversation_decision(
                ConversationPlanDirection::Pull,
                ConversationRelation::LocalPrefixOfCloud,
                ConversationCapabilities {
                    binding,
                    ..ConversationCapabilities::default()
                },
            );
            assert_eq!(
                fast_forward.action,
                ConversationPlanActionType::FastForwardLocal
            );
            assert!(fast_forward.selected_by_default);

            let fork = default_conversation_decision(
                ConversationPlanDirection::Pull,
                ConversationRelation::Diverged,
                ConversationCapabilities {
                    binding,
                    ..ConversationCapabilities::default()
                },
            );
            assert_eq!(fork.action, ConversationPlanActionType::Fork);
            assert!(fork.selected_by_default);
        }

        let ambiguous_new_thread = default_conversation_decision(
            ConversationPlanDirection::Push,
            ConversationRelation::CloudMissing,
            ConversationCapabilities {
                binding: BindingResolution::Ambiguous,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(
            ambiguous_new_thread.action,
            ConversationPlanActionType::Blocked
        );
        assert_eq!(
            ambiguous_new_thread.reason_code,
            ConversationReasonCode::AmbiguousBinding
        );
    }
}
