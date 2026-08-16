//! Pure conversation-thread reconciliation and reviewed-plan pinning.
//!
//! Filesystem, Tauri, and object-store code call these functions rather than
//! reimplementing relation-to-action policy. This keeps Push, Pull, recovery,
//! and the integration harness on one state machine. The relation itself is
//! computed only by `mallard_domain::compare_conversation_tail`; this module
//! maps the returned classification to typed actions and never refines it.

use std::collections::BTreeMap;

use mallard_domain::{
    ActionId, ConversationPlanAction, ConversationPlanActionType, ConversationPlanDirection,
    ConversationReasonCode, HostId, KeepBothPlan, LocalProjectId, LocalProviderProfileId,
    PortableConversationTail, ProviderKey, ProviderStateExpectation, RawRangePin, ReplicaId,
    ResourceId, SyncCurrency, TailClassification, TargetExpectation, ThreadId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingResolution {
    Bound,
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
    /// True while a journaled sync operation still owns this thread. Every
    /// new mutation is blocked until that operation reaches a terminal phase.
    pub resolution_pending: bool,
    /// True when the destination checkpoint is `Detached`: the bound alias
    /// was replaced by another operation, so Push has no derivable local
    /// value and the thread needs a Pull first.
    pub physical_detached: bool,
    /// True when the destination checkpoint carried a durable accepted pin
    /// at review time. False when acceptance was proven only by the
    /// first-contact adoption probe, whose durable pin is not written until
    /// a Pull adopts it.
    pub acceptance_durably_pinned: bool,
}

impl Default for ConversationCapabilities {
    fn default() -> Self {
        Self {
            binding: BindingResolution::Bound,
            local_safely_capturable: true,
            cloud_safely_readable: true,
            destination_codec_resumable: true,
            provider_quiescent: true,
            resolution_pending: false,
            physical_detached: false,
            acceptance_durably_pinned: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationDecision {
    pub action: ConversationPlanActionType,
    pub reason_code: ConversationReasonCode,
    pub selectable: bool,
    pub blocked_reason: Option<String>,
}

/// Map a classified relation to one typed default action. The map is fixed by
/// the resolver contract: Push never runs Pull's path, publishes only
/// `LocalOutstanding`/`MissingCloud`, and defers every conflict to Pull;
/// Pull's only divergence resolution is keep-both. A durably pinned
/// `Sync(Current)` thread is `NoChanges` in both directions; Pull adopts
/// metadata only when the durable pin is still missing (first contact) or
/// when both sides agree on a value newer than the pins (`Advanced`).
pub fn default_conversation_decision(
    direction: ConversationPlanDirection,
    relation: TailClassification,
    capabilities: ConversationCapabilities,
) -> ConversationDecision {
    use ConversationPlanActionType as Action;
    use ConversationReasonCode as Reason;
    use TailClassification as Relation;

    if capabilities.resolution_pending {
        return decision(Action::Blocked, Reason::Busy);
    }
    if capabilities.physical_detached && relation == Relation::Unknown {
        return decision(Action::Blocked, Reason::DetachedRequiresPull);
    }

    let binding_block = |capabilities: ConversationCapabilities| match capabilities.binding {
        BindingResolution::Ambiguous => Some(Reason::AmbiguousBinding),
        BindingResolution::Missing => Some(Reason::MissingBinding),
        BindingResolution::Bound => None,
    };

    match direction {
        ConversationPlanDirection::Push => match relation {
            Relation::Sync(SyncCurrency::Current) => {
                decision(Action::NoChanges, Reason::SyncCurrent)
            }
            Relation::Sync(SyncCurrency::Advanced) => {
                decision(Action::RequirePullMetadata, Reason::SyncAdvanced)
            }
            Relation::LocalOutstanding => {
                if let Some(reason) = binding_block(capabilities) {
                    decision(Action::Blocked, reason)
                } else if !capabilities.local_safely_capturable {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                } else {
                    decision(Action::Publish, Reason::LocalOutstanding)
                }
            }
            Relation::CloudOutstanding => decision(Action::RequirePull, Reason::CloudOutstanding),
            Relation::Divergent => decision(Action::RequirePull, Reason::Divergent),
            Relation::MissingCloud => {
                if capabilities.binding == BindingResolution::Ambiguous {
                    decision(Action::Blocked, Reason::AmbiguousBinding)
                } else if !capabilities.local_safely_capturable {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                } else {
                    decision(Action::CreateCloud, Reason::MissingCloud)
                }
            }
            Relation::MissingLocal => decision(Action::NotPushable, Reason::MissingLocal),
            Relation::MissingBoth => decision(Action::NotPushable, Reason::MissingBoth),
            Relation::Unknown => decision(Action::Blocked, Reason::UnknownEvidence),
        },
        ConversationPlanDirection::Pull => match relation {
            // A durably pinned current thread has nothing to record: the
            // plan-level apply advances scope bases without per-thread rows.
            Relation::Sync(SyncCurrency::Current) if capabilities.acceptance_durably_pinned => {
                decision(Action::NoChanges, Reason::SyncCurrent)
            }
            Relation::Sync(currency) => decision(
                Action::AdoptCloudMetadata,
                match currency {
                    SyncCurrency::Current => Reason::SyncCurrent,
                    SyncCurrency::Advanced => Reason::SyncAdvanced,
                },
            ),
            Relation::LocalOutstanding => {
                decision(Action::ReportPushEligible, Reason::LocalOutstanding)
            }
            Relation::CloudOutstanding => materializing_decision(
                Action::MaterializeCloud,
                Reason::CloudOutstanding,
                capabilities,
            ),
            Relation::Divergent => {
                if !capabilities.local_safely_capturable {
                    decision(Action::Blocked, Reason::CodecUnsupported)
                } else {
                    materializing_decision(Action::KeepBoth, Reason::Divergent, capabilities)
                }
            }
            Relation::MissingLocal => {
                materializing_decision(Action::CreateLocal, Reason::MissingLocal, capabilities)
            }
            Relation::MissingCloud => decision(Action::ReportLocalOnly, Reason::MissingCloud),
            Relation::MissingBoth => decision(Action::NotFound, Reason::MissingBoth),
            Relation::Unknown => decision(Action::Blocked, Reason::UnknownEvidence),
        },
    }
}

fn materializing_decision(
    action: ConversationPlanActionType,
    reason: ConversationReasonCode,
    capabilities: ConversationCapabilities,
) -> ConversationDecision {
    use ConversationPlanActionType as Action;
    use ConversationReasonCode as Reason;
    if !capabilities.cloud_safely_readable || !capabilities.destination_codec_resumable {
        decision(Action::Blocked, Reason::CodecUnsupported)
    } else if !capabilities.provider_quiescent {
        decision(Action::Blocked, Reason::ProviderNotQuiescent)
    } else if capabilities.binding == BindingResolution::Ambiguous {
        decision(Action::Blocked, Reason::AmbiguousBinding)
    } else if capabilities.binding == BindingResolution::Missing && action != Action::CreateLocal {
        decision(Action::Blocked, Reason::MissingBinding)
    } else {
        decision(action, reason)
    }
}

fn decision(
    action: ConversationPlanActionType,
    reason_code: ConversationReasonCode,
) -> ConversationDecision {
    let selectable = matches!(
        action,
        ConversationPlanActionType::Publish
            | ConversationPlanActionType::CreateCloud
            | ConversationPlanActionType::AdoptCloudMetadata
            | ConversationPlanActionType::MaterializeCloud
            | ConversationPlanActionType::KeepBoth
            | ConversationPlanActionType::CreateLocal
    );
    ConversationDecision {
        action,
        reason_code,
        selectable,
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
        ConversationReasonCode::Busy => {
            "A journaled sync operation still owns this conversation. \
             Finish or recover it first."
        }
        ConversationReasonCode::DetachedRequiresPull => {
            "This conversation's local mapping was replaced. Pull to \
             re-establish it."
        }
        ConversationReasonCode::CorruptionBlocked => {
            "This conversation's local state failed verification and needs \
             explicit repair."
        }
        _ => "This conversation cannot be changed safely.",
    }
}

/// Immutable inputs used to build one reviewed conversation action. Keeping
/// these values together makes it difficult for Push, Pull, or a test facade
/// to omit one of the concurrency pins.
#[derive(Clone, Debug)]
pub struct ConversationPlanActionRequest {
    pub action_id: ActionId,
    pub direction: ConversationPlanDirection,
    /// Classified by `compare_conversation_tail`; the action pins the exact
    /// evidence and apply re-proves it before any mutation.
    pub relation: TailClassification,
    pub capabilities: ConversationCapabilities,
    pub provider: ProviderKey,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub local_project_id: LocalProjectId,
    pub provider_profile_id: LocalProviderProfileId,
    pub thread_id: ThreadId,
    pub source_resource_id: Option<ResourceId>,
    pub proposed_provider_thread_id: Option<String>,
    pub keep_both: Option<KeepBothPlan>,
    pub required_codec: String,
    pub pinned_revision: u64,
    pub pinned_commit_id: String,
    pub pinned_manifest_sha256: String,
    /// Reviewed provider-scope coordinates: `(0, all-zero digest)` pins an
    /// absent scope.
    pub pinned_scope_revision: u64,
    pub pinned_scope_sha256: String,
    pub pinned_accepted_tail: Option<PortableConversationTail>,
    pub pinned_accepted_local_tail: Option<PortableConversationTail>,
    pub pinned_local_tail: Option<PortableConversationTail>,
    pub pinned_cloud_tail: Option<PortableConversationTail>,
    pub pinned_checkpoint_generation: Option<u64>,
    pub pinned_lineage_digest: Option<String>,
    pub pinned_semantic_digest: Option<String>,
    pub pinned_raw_range: Option<RawRangePin>,
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
    request: ConversationPlanActionRequest,
) -> Result<ConversationPlanAction, String> {
    let relation = request.relation;
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
        source_resource_id: request.source_resource_id,
        proposed_provider_thread_id: request.proposed_provider_thread_id,
        keep_both: request.keep_both,
        required_codec: request.required_codec,
        pinned_revision: request.pinned_revision,
        pinned_commit_id: request.pinned_commit_id,
        pinned_manifest_sha256: request.pinned_manifest_sha256,
        pinned_scope_revision: request.pinned_scope_revision,
        pinned_scope_sha256: request.pinned_scope_sha256,
        pinned_accepted_tail: request.pinned_accepted_tail,
        pinned_accepted_local_tail: request.pinned_accepted_local_tail,
        pinned_local_tail: request.pinned_local_tail,
        pinned_cloud_tail: request.pinned_cloud_tail,
        pinned_checkpoint_generation: request.pinned_checkpoint_generation,
        pinned_lineage_digest: request.pinned_lineage_digest,
        pinned_semantic_digest: request.pinned_semantic_digest,
        pinned_raw_range: request.pinned_raw_range,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationStalePin {
    Expired,
    Host,
    Replica,
    LocalProject,
    ProviderProfile,
    HostIdentityRevision,
    Revision,
    Commit,
    Manifest,
    ScopeRevision,
    ScopeDigest,
    AcceptedTail,
    AcceptedLocalTail,
    LocalTail,
    CloudTail,
    CheckpointGeneration,
    LineageDigest,
    SemanticDigest,
    RawRange,
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
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub scope_revision: u64,
    pub scope_sha256: String,
    pub accepted_tail: Option<PortableConversationTail>,
    pub accepted_local_tail: Option<PortableConversationTail>,
    pub local_tail: Option<PortableConversationTail>,
    pub cloud_tail: Option<PortableConversationTail>,
    /// The live checkpoint's generation (`None` = confirmed absent).
    pub checkpoint_generation: Option<u64>,
    pub lineage_digest: Option<String>,
    pub semantic_digest: Option<String>,
    pub raw_range: Option<RawRangePin>,
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
    if current.revision != action.pinned_revision {
        return Err(ConversationStalePin::Revision);
    }
    if current.commit_id != action.pinned_commit_id {
        return Err(ConversationStalePin::Commit);
    }
    if current.manifest_sha256 != action.pinned_manifest_sha256 {
        return Err(ConversationStalePin::Manifest);
    }
    if current.scope_revision != action.pinned_scope_revision {
        return Err(ConversationStalePin::ScopeRevision);
    }
    if current.scope_sha256 != action.pinned_scope_sha256 {
        return Err(ConversationStalePin::ScopeDigest);
    }
    if current.accepted_tail != action.pinned_accepted_tail {
        return Err(ConversationStalePin::AcceptedTail);
    }
    if current.accepted_local_tail != action.pinned_accepted_local_tail {
        return Err(ConversationStalePin::AcceptedLocalTail);
    }
    if current.local_tail != action.pinned_local_tail {
        return Err(ConversationStalePin::LocalTail);
    }
    if current.cloud_tail != action.pinned_cloud_tail {
        return Err(ConversationStalePin::CloudTail);
    }
    if current.checkpoint_generation != action.pinned_checkpoint_generation {
        return Err(ConversationStalePin::CheckpointGeneration);
    }
    if current.lineage_digest != action.pinned_lineage_digest {
        return Err(ConversationStalePin::LineageDigest);
    }
    if current.semantic_digest != action.pinned_semantic_digest {
        return Err(ConversationStalePin::SemanticDigest);
    }
    if current.raw_range != action.pinned_raw_range {
        return Err(ConversationStalePin::RawRange);
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
        ConversationPlanActionType::MaterializeCloud
            | ConversationPlanActionType::KeepBoth
            | ConversationPlanActionType::CreateLocal
    ) && !current.provider_quiescent
    {
        return Err(ConversationStalePin::ProviderQuiescence);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_maps_every_relation_exactly() {
        use ConversationPlanActionType as Action;
        let capabilities = ConversationCapabilities::default();
        let rows = [
            (
                TailClassification::Sync(SyncCurrency::Current),
                Action::NoChanges,
            ),
            (
                TailClassification::Sync(SyncCurrency::Advanced),
                Action::RequirePullMetadata,
            ),
            (TailClassification::LocalOutstanding, Action::Publish),
            (TailClassification::CloudOutstanding, Action::RequirePull),
            (TailClassification::Divergent, Action::RequirePull),
            (TailClassification::MissingCloud, Action::CreateCloud),
            (TailClassification::MissingLocal, Action::NotPushable),
            (TailClassification::MissingBoth, Action::NotPushable),
            (TailClassification::Unknown, Action::Blocked),
        ];
        for (relation, expected) in rows {
            assert_eq!(
                default_conversation_decision(
                    ConversationPlanDirection::Push,
                    relation,
                    capabilities
                )
                .action,
                expected,
                "{relation:?}"
            );
        }
    }

    #[test]
    fn pull_maps_every_relation_exactly() {
        use ConversationPlanActionType as Action;
        let capabilities = ConversationCapabilities::default();
        let rows = [
            (
                TailClassification::Sync(SyncCurrency::Current),
                Action::NoChanges,
            ),
            (
                TailClassification::Sync(SyncCurrency::Advanced),
                Action::AdoptCloudMetadata,
            ),
            (
                TailClassification::LocalOutstanding,
                Action::ReportPushEligible,
            ),
            (
                TailClassification::CloudOutstanding,
                Action::MaterializeCloud,
            ),
            (TailClassification::Divergent, Action::KeepBoth),
            (TailClassification::MissingLocal, Action::CreateLocal),
            (TailClassification::MissingCloud, Action::ReportLocalOnly),
            (TailClassification::MissingBoth, Action::NotFound),
            (TailClassification::Unknown, Action::Blocked),
        ];
        for (relation, expected) in rows {
            assert_eq!(
                default_conversation_decision(
                    ConversationPlanDirection::Pull,
                    relation,
                    capabilities
                )
                .action,
                expected,
                "{relation:?}"
            );
        }
    }

    #[test]
    fn pull_splits_current_sync_on_acceptance_durability() {
        use ConversationPlanActionType as Action;
        // Durable pin: nothing to record, the row is not selectable.
        let durable = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::Sync(SyncCurrency::Current),
            ConversationCapabilities::default(),
        );
        assert_eq!(durable.action, Action::NoChanges);
        assert_eq!(durable.reason_code, ConversationReasonCode::SyncCurrent);
        assert!(!durable.selectable);

        // Probe-proven acceptance: Pull is what writes the first durable pin.
        let probe = ConversationCapabilities {
            acceptance_durably_pinned: false,
            ..ConversationCapabilities::default()
        };
        let first_contact = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::Sync(SyncCurrency::Current),
            probe,
        );
        assert_eq!(first_contact.action, Action::AdoptCloudMetadata);
        assert_eq!(
            first_contact.reason_code,
            ConversationReasonCode::SyncCurrent
        );
        assert!(first_contact.selectable);

        // The same-form advanced agreement adopts regardless of durability,
        // and Push's mapping ignores the flag entirely.
        for capabilities in [ConversationCapabilities::default(), probe] {
            assert_eq!(
                default_conversation_decision(
                    ConversationPlanDirection::Pull,
                    TailClassification::Sync(SyncCurrency::Advanced),
                    capabilities,
                )
                .action,
                Action::AdoptCloudMetadata
            );
            assert_eq!(
                default_conversation_decision(
                    ConversationPlanDirection::Push,
                    TailClassification::Sync(SyncCurrency::Current),
                    capabilities,
                )
                .action,
                Action::NoChanges
            );
        }
    }

    #[test]
    fn gates_block_instead_of_degrading() {
        let unquiescent = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::Divergent,
            ConversationCapabilities {
                provider_quiescent: false,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(unquiescent.action, ConversationPlanActionType::Blocked);
        assert_eq!(
            unquiescent.reason_code,
            ConversationReasonCode::ProviderNotQuiescent
        );
        assert!(unquiescent.blocked_reason.is_some());

        let ambiguous = default_conversation_decision(
            ConversationPlanDirection::Push,
            TailClassification::LocalOutstanding,
            ConversationCapabilities {
                binding: BindingResolution::Ambiguous,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(ambiguous.action, ConversationPlanActionType::Blocked);
        assert_eq!(
            ambiguous.reason_code,
            ConversationReasonCode::AmbiguousBinding
        );

        // CreateLocal is the one materializing action that legitimately has
        // no existing binding.
        let create_local = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::MissingLocal,
            ConversationCapabilities {
                binding: BindingResolution::Missing,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(create_local.action, ConversationPlanActionType::CreateLocal);
    }

    #[test]
    fn pending_and_detached_states_block_with_typed_reasons() {
        for direction in [
            ConversationPlanDirection::Push,
            ConversationPlanDirection::Pull,
        ] {
            let busy = default_conversation_decision(
                direction,
                TailClassification::Sync(SyncCurrency::Current),
                ConversationCapabilities {
                    resolution_pending: true,
                    ..ConversationCapabilities::default()
                },
            );
            assert_eq!(busy.action, ConversationPlanActionType::Blocked);
            assert_eq!(busy.reason_code, ConversationReasonCode::Busy);
            assert!(!busy.selectable);
        }

        let detached = default_conversation_decision(
            ConversationPlanDirection::Push,
            TailClassification::Unknown,
            ConversationCapabilities {
                physical_detached: true,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(detached.action, ConversationPlanActionType::Blocked);
        assert_eq!(
            detached.reason_code,
            ConversationReasonCode::DetachedRequiresPull
        );
    }

    #[test]
    fn only_mutating_actions_are_selectable() {
        let publish = default_conversation_decision(
            ConversationPlanDirection::Push,
            TailClassification::LocalOutstanding,
            ConversationCapabilities::default(),
        );
        assert!(publish.selectable);
        let require_pull = default_conversation_decision(
            ConversationPlanDirection::Push,
            TailClassification::Divergent,
            ConversationCapabilities::default(),
        );
        assert!(!require_pull.selectable);
        let report = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::LocalOutstanding,
            ConversationCapabilities::default(),
        );
        assert!(!report.selectable);
        let keep_both = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::Divergent,
            ConversationCapabilities::default(),
        );
        assert!(keep_both.selectable);
    }
}
