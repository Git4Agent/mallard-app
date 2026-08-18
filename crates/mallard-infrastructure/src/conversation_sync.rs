//! Pure conversation-thread reconciliation and reviewed-plan pinning.
//!
//! Filesystem, Tauri, and object-store code call these functions rather than
//! reimplementing relation-to-action policy. This keeps Push, Pull, recovery,
//! and the integration harness on one state machine. The relation itself is
//! computed only by `mallard_domain::compare_conversation_tail`; this module
//! maps the returned classification to typed actions and never refines it.

use std::collections::BTreeMap;

use mallard_domain::{
    ActionId, ClassifiedThread, ConversationActionPins, ConversationPlanAction,
    ConversationPlanDirection, KeepBothPlan, ProviderKey, ResourceId, ThreadId,
};

/// Immutable inputs used to build one reviewed conversation action. Keeping
/// these values together makes it difficult for Push, Pull, or a test facade
/// to omit one of the concurrency pins.
#[derive(Clone, Debug)]
pub struct ConversationPlanActionRequest {
    pub action_id: ActionId,
    pub direction: ConversationPlanDirection,
    pub classified: ClassifiedThread,
    pub provider: ProviderKey,
    pub thread_id: ThreadId,
    pub source_resource_id: Option<ResourceId>,
    pub proposed_provider_thread_id: Option<String>,
    pub keep_both: Option<KeepBothPlan>,
    pub pins: ConversationActionPins,
}

/// Project one classified row into an executable action. Informational and
/// blocked decisions return `None` and therefore cannot enter persisted plan
/// state.
pub fn build_conversation_plan_action(
    request: ConversationPlanActionRequest,
) -> Result<Option<ConversationPlanAction>, String> {
    if request.pins.tail.classification()? != request.classified.relation {
        return Err("conversation action pins disagree with its classification".to_string());
    }
    let Some(planned_action) = request.classified.planned_action(request.direction) else {
        return Ok(None);
    };
    let action = ConversationPlanAction {
        action_id: request.action_id,
        action: planned_action,
        provider: request.provider,
        thread_id: request.thread_id,
        source_resource_id: request.source_resource_id,
        proposed_provider_thread_id: request.proposed_provider_thread_id,
        keep_both: request.keep_both,
        pins: request.pins,
    };
    action.validate()?;
    Ok(Some(action))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationContextPin {
    Host,
    Replica,
    LocalProject,
    ProviderProfile,
    HostIdentityRevision,
    ProfileRevision,
    ProjectBindingRevision,
    BindingsDocumentRevision,
    BindingRevision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationRevisionPin {
    BundleRevision,
    Commit,
    Manifest,
    ScopeRevision,
    ScopeDigest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationTailPin {
    AcceptedTail,
    AcceptedLocalTail,
    LocalTail,
    CloudTail,
    CheckpointGeneration,
    LineageDigest,
    SemanticDigest,
    RawRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationTargetPin {
    LocalTarget,
    ProviderState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationStalePin {
    Expired,
    Context(ConversationContextPin),
    Revision(ConversationRevisionPin),
    Tail(ConversationTailPin),
    Target(ConversationTargetPin),
    Codec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationApplyContext {
    pub now: u64,
    pub pins: ConversationActionPins,
}

/// Revalidate every reviewed pin before a caller writes a journal record or
/// mutates provider/cloud state. The stable comparison order gives callers a
/// deterministic, typed rejection reason while still rejecting all changes.
pub fn validate_conversation_action_pins(
    action: &ConversationPlanAction,
    current: &ConversationApplyContext,
) -> Result<(), ConversationStalePin> {
    use ConversationContextPin as Context;
    use ConversationRevisionPin as Revision;
    use ConversationTailPin as Tail;
    use ConversationTargetPin as Target;

    if current.now > action.pins.expires_at {
        return Err(ConversationStalePin::Expired);
    }
    let planned_context = &action.pins.context;
    let current_context = &current.pins.context;
    if current_context.host_id != planned_context.host_id {
        return Err(ConversationStalePin::Context(Context::Host));
    }
    if current_context.replica_id != planned_context.replica_id {
        return Err(ConversationStalePin::Context(Context::Replica));
    }
    if current_context.local_project_id != planned_context.local_project_id {
        return Err(ConversationStalePin::Context(Context::LocalProject));
    }
    if current_context.provider_profile_id != planned_context.provider_profile_id {
        return Err(ConversationStalePin::Context(Context::ProviderProfile));
    }
    if current_context.host_identity_revision != planned_context.host_identity_revision {
        return Err(ConversationStalePin::Context(Context::HostIdentityRevision));
    }
    if current_context.profile_revision != planned_context.profile_revision {
        return Err(ConversationStalePin::Context(Context::ProfileRevision));
    }
    if current_context.project_binding_revision != planned_context.project_binding_revision {
        return Err(ConversationStalePin::Context(
            Context::ProjectBindingRevision,
        ));
    }
    if current_context.bindings_document_revision != planned_context.bindings_document_revision {
        return Err(ConversationStalePin::Context(
            Context::BindingsDocumentRevision,
        ));
    }
    if current_context.binding_revision != planned_context.binding_revision {
        return Err(ConversationStalePin::Context(Context::BindingRevision));
    }

    let planned_revision = &action.pins.revision;
    let current_revision = &current.pins.revision;
    if current_revision.bundle_revision != planned_revision.bundle_revision {
        return Err(ConversationStalePin::Revision(Revision::BundleRevision));
    }
    if current_revision.commit_id != planned_revision.commit_id {
        return Err(ConversationStalePin::Revision(Revision::Commit));
    }
    if current_revision.manifest_sha256 != planned_revision.manifest_sha256 {
        return Err(ConversationStalePin::Revision(Revision::Manifest));
    }
    if current_revision.scope_revision != planned_revision.scope_revision {
        return Err(ConversationStalePin::Revision(Revision::ScopeRevision));
    }
    if current_revision.scope_sha256 != planned_revision.scope_sha256 {
        return Err(ConversationStalePin::Revision(Revision::ScopeDigest));
    }

    let planned_tail = &action.pins.tail;
    let current_tail = &current.pins.tail;
    if current_tail.accepted != planned_tail.accepted {
        return Err(ConversationStalePin::Tail(Tail::AcceptedTail));
    }
    if current_tail.accepted_local != planned_tail.accepted_local {
        return Err(ConversationStalePin::Tail(Tail::AcceptedLocalTail));
    }
    if current_tail.local != planned_tail.local {
        return Err(ConversationStalePin::Tail(Tail::LocalTail));
    }
    if current_tail.cloud != planned_tail.cloud {
        return Err(ConversationStalePin::Tail(Tail::CloudTail));
    }
    if current_tail.checkpoint_generation != planned_tail.checkpoint_generation {
        return Err(ConversationStalePin::Tail(Tail::CheckpointGeneration));
    }
    if current_tail.lineage_digest != planned_tail.lineage_digest {
        return Err(ConversationStalePin::Tail(Tail::LineageDigest));
    }
    if current_tail.semantic_digest != planned_tail.semantic_digest {
        return Err(ConversationStalePin::Tail(Tail::SemanticDigest));
    }
    if current_tail.raw_range != planned_tail.raw_range {
        return Err(ConversationStalePin::Tail(Tail::RawRange));
    }

    if current.pins.target.local != action.pins.target.local {
        return Err(ConversationStalePin::Target(Target::LocalTarget));
    }
    let provider_state = |targets: &[mallard_domain::ProviderStateExpectation]| {
        targets
            .iter()
            .map(|target| (target.target_path.clone(), target.expectation.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    if provider_state(&current.pins.target.provider_state)
        != provider_state(&action.pins.target.provider_state)
    {
        return Err(ConversationStalePin::Target(Target::ProviderState));
    }
    if current.pins.codec != action.pins.codec {
        return Err(ConversationStalePin::Codec);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::{
        default_conversation_decision, BindingResolution, ConversationBlocker,
        ConversationCapabilities, ConversationOperationState, ConversationReviewAction,
        SyncCurrency, TailClassification,
    };

    #[test]
    fn push_maps_every_relation_exactly() {
        use ConversationReviewAction as Action;
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
        use ConversationReviewAction as Action;
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
        use ConversationReviewAction as Action;
        // Durable pin: nothing to record, the row is not selectable.
        let durable = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::Sync(SyncCurrency::Current),
            ConversationCapabilities::default(),
        );
        assert_eq!(durable.action, Action::NoChanges);
        assert_eq!(durable.blocker, None);
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
        assert_eq!(first_contact.blocker, None);
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
        let unreadable = default_conversation_decision(
            ConversationPlanDirection::Pull,
            TailClassification::Divergent,
            ConversationCapabilities {
                cloud_materializable: false,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(unreadable.action, ConversationReviewAction::Blocked);
        assert_eq!(
            unreadable.blocker,
            Some(ConversationBlocker::CodecUnsupported)
        );
        assert!(unreadable.blocked_reason.is_some());

        let ambiguous = default_conversation_decision(
            ConversationPlanDirection::Push,
            TailClassification::LocalOutstanding,
            ConversationCapabilities {
                binding: BindingResolution::Ambiguous,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(ambiguous.action, ConversationReviewAction::Blocked);
        assert_eq!(
            ambiguous.blocker,
            Some(ConversationBlocker::AmbiguousBinding)
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
        assert_eq!(create_local.action, ConversationReviewAction::CreateLocal);
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
                    operation_state: ConversationOperationState::Pending,
                    ..ConversationCapabilities::default()
                },
            );
            assert_eq!(busy.action, ConversationReviewAction::Blocked);
            assert_eq!(busy.blocker, Some(ConversationBlocker::Busy));
            assert!(!busy.selectable);
        }

        let detached = default_conversation_decision(
            ConversationPlanDirection::Push,
            TailClassification::Unknown,
            ConversationCapabilities {
                operation_state: ConversationOperationState::Detached,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(detached.action, ConversationReviewAction::Blocked);
        assert_eq!(
            detached.blocker,
            Some(ConversationBlocker::DetachedRequiresPull)
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
