use super::harness::{analyzed_chain, chain, codex};
use mallard_domain::{
    compare_conversation_tail, compute_portable_conversation_tail, default_conversation_decision,
    ActionId, ClassifiedThread, CloudThreadId, ConversationActionPins, ConversationCapabilities,
    ConversationContextPins, ConversationPlanAction, ConversationPlanActionType,
    ConversationPlanDirection, ConversationReviewAction, ConversationRevisionPins,
    ConversationTailPins, ConversationTargetPins, HostId, LocalProjectId, LocalProviderProfileId,
    PortableConversationTail, ProviderStateExpectation, RawRangePin, ReplicaId, ResourceId,
    SyncCurrency, TailClassification, TailEvidenceBlocker, TailPartitionKind, TailSideEvidence,
    TargetExpectation, ThreadId, VerifiedTailEvidence,
};
use mallard_infrastructure::conversation_sync::{
    build_conversation_plan_action, validate_conversation_action_pins, ConversationApplyContext,
    ConversationPlanActionRequest,
};

fn whole_tail(ids: &[(&str, &str)]) -> PortableConversationTail {
    compute_portable_conversation_tail(
        TailPartitionKind::WholeThread,
        0,
        &analyzed_chain(&chain("thread-a", ids)),
    )
    .unwrap()
}

fn side(value: Option<&PortableConversationTail>) -> TailSideEvidence {
    match value {
        Some(tail) => TailSideEvidence::Present(tail.clone()),
        None => TailSideEvidence::ConfirmedAbsent,
    }
}

fn evidence_of(
    accepted: Option<&PortableConversationTail>,
    local: Option<&PortableConversationTail>,
    cloud: Option<&PortableConversationTail>,
) -> Result<VerifiedTailEvidence, TailEvidenceBlocker> {
    VerifiedTailEvidence::new(side(accepted), side(accepted), side(local), side(cloud))
}

fn tails_for_relation(
    relation: TailClassification,
) -> (
    Option<PortableConversationTail>,
    Option<PortableConversationTail>,
    Option<PortableConversationTail>,
) {
    let base = whole_tail(&[("m-a", "one"), ("m-b", "two")]);
    let longer = whole_tail(&[("m-a", "one"), ("m-b", "two"), ("m-c", "local")]);
    match relation {
        TailClassification::Sync(SyncCurrency::Current) => {
            (Some(base.clone()), Some(base.clone()), Some(base))
        }
        TailClassification::Sync(SyncCurrency::Advanced) => {
            (Some(base), Some(longer.clone()), Some(longer))
        }
        TailClassification::LocalOutstanding => (Some(base.clone()), Some(longer), Some(base)),
        TailClassification::CloudOutstanding => (Some(base.clone()), Some(base), Some(longer)),
        TailClassification::Divergent => (
            Some(base),
            Some(longer),
            Some(whole_tail(&[
                ("m-a", "one"),
                ("m-b", "two"),
                ("m-d", "cloud"),
            ])),
        ),
        TailClassification::MissingCloud => (None, Some(base), None),
        TailClassification::MissingLocal => (None, None, Some(base)),
        TailClassification::MissingBoth => (None, None, None),
        TailClassification::Unknown => panic!("unknown evidence cannot become verified pins"),
    }
}

#[derive(Clone, Copy)]
struct MatrixCase {
    name: &'static str,
    relation: TailClassification,
    push: ConversationReviewAction,
    pull: ConversationReviewAction,
}

#[test]
fn canonical_policy_covers_each_verified_relation_and_persists_only_mutations() {
    use ConversationReviewAction as Review;
    let cases = [
        MatrixCase {
            name: "sync_current",
            relation: TailClassification::Sync(SyncCurrency::Current),
            push: Review::NoChanges,
            pull: Review::NoChanges,
        },
        MatrixCase {
            name: "sync_advanced",
            relation: TailClassification::Sync(SyncCurrency::Advanced),
            push: Review::RequirePullMetadata,
            pull: Review::AdoptCloudMetadata,
        },
        MatrixCase {
            name: "local_outstanding",
            relation: TailClassification::LocalOutstanding,
            push: Review::Publish,
            pull: Review::ReportPushEligible,
        },
        MatrixCase {
            name: "cloud_outstanding",
            relation: TailClassification::CloudOutstanding,
            push: Review::RequirePull,
            pull: Review::MaterializeCloud,
        },
        MatrixCase {
            name: "divergent",
            relation: TailClassification::Divergent,
            push: Review::RequirePull,
            pull: Review::KeepBoth,
        },
        MatrixCase {
            name: "missing_local",
            relation: TailClassification::MissingLocal,
            push: Review::NotPushable,
            pull: Review::CreateLocal,
        },
        MatrixCase {
            name: "missing_cloud",
            relation: TailClassification::MissingCloud,
            push: Review::CreateCloud,
            pull: Review::ReportLocalOnly,
        },
        MatrixCase {
            name: "missing_both",
            relation: TailClassification::MissingBoth,
            push: Review::NotPushable,
            pull: Review::NotFound,
        },
    ];

    for case in cases {
        let (accepted, local, cloud) = tails_for_relation(case.relation);
        let verified = evidence_of(accepted.as_ref(), local.as_ref(), cloud.as_ref()).unwrap();
        assert_eq!(
            compare_conversation_tail(&verified),
            case.relation,
            "{}",
            case.name
        );

        for (direction, expected) in [
            (ConversationPlanDirection::Push, case.push),
            (ConversationPlanDirection::Pull, case.pull),
        ] {
            let classified =
                ClassifiedThread::new(case.relation, ConversationCapabilities::default());
            let decision = classified.decision(direction);
            assert_eq!(decision.action, expected, "{} {direction:?}", case.name);
            assert_eq!(decision.selectable, expected.planned_action().is_some());

            let template_action = expected.planned_action().map(|action| {
                reviewed_action_with_tails(action, accepted.clone(), local.clone(), cloud.clone()).0
            });
            let pins = template_action
                .as_ref()
                .map(|action| action.pins.clone())
                .unwrap_or_else(|| {
                    review_only_pins(accepted.clone(), local.clone(), cloud.clone())
                });
            let action_id = format!("action-{}-{direction:?}", case.name).to_lowercase();
            // Plans carry intent only: the id pair follows the D5 presence
            // matrix (no local id exactly for CreateLocal, no cloud id
            // exactly for an unmapped CreateCloud), and no minted identity
            // of any kind exists at plan time.
            let planned_type = expected.planned_action();
            let planned = build_conversation_plan_action(ConversationPlanActionRequest {
                action_id: ActionId::parse(action_id).unwrap(),
                direction,
                classified,
                provider: codex(),
                thread_id: (planned_type != Some(ConversationPlanActionType::CreateLocal))
                    .then(|| ThreadId::parse("thread-a").unwrap()),
                cloud_thread_id: (planned_type != Some(ConversationPlanActionType::CreateCloud))
                    .then(|| CloudThreadId::parse("cloudthread-a").unwrap()),
                source_resource_id: Some(ResourceId::parse("codex:conversation:source").unwrap()),
                pins,
            })
            .unwrap();
            assert_eq!(
                planned.as_ref().map(|action| action.action),
                expected.planned_action()
            );
            if let Some(action) = planned {
                let current = ConversationApplyContext {
                    now: 1_500,
                    pins: action.pins.clone(),
                };
                validate_conversation_action_pins(&action, &current).unwrap();
            }
        }
    }
}

/// S4: both copies exist and no accepted pins relate them, because the
/// acceptance is missing or stale and adoption could not prove the stored copy
/// equal to or an earlier part of the local one. Nothing is replaced: Push
/// asks for a Pull, and Pull keeps both. The plan check accepts that action
/// on pins that carry no acceptance.
#[test]
fn a_pair_without_an_acceptance_keeps_both_instead_of_blocking() {
    let local = whole_tail(&[("m-a", "one"), ("m-b", "two"), ("m-c", "local")]);
    let cloud = whole_tail(&[("m-a", "one"), ("m-b", "two"), ("m-d", "cloud")]);
    let verified = evidence_of(None, Some(&local), Some(&cloud)).unwrap();
    assert_eq!(
        compare_conversation_tail(&verified),
        TailClassification::Divergent
    );
    let classified = ClassifiedThread::new(
        TailClassification::Divergent,
        ConversationCapabilities::default(),
    );
    assert_eq!(
        classified.decision(ConversationPlanDirection::Push).action,
        ConversationReviewAction::RequirePull
    );
    let pull = classified.decision(ConversationPlanDirection::Pull);
    assert_eq!(pull.action, ConversationReviewAction::KeepBoth);
    assert!(pull.selectable);

    let (action, current) = reviewed_action_with_tails(
        ConversationPlanActionType::KeepBoth,
        None,
        Some(local),
        Some(cloud),
    );
    assert!(action.pins.tail.accepted.is_none());
    action.validate().unwrap();
    validate_conversation_action_pins(&action, &current).unwrap();
}

/// A plan says what its Pull keeps of the children that only this computer
/// has. When the provider keeps something else at Apply, the plan is stale.
#[test]
fn a_plan_is_stale_when_the_pull_keeps_other_children_than_reviewed() {
    let (action, mut current) = reviewed_action(
        ConversationPlanActionType::MaterializeCloud,
        TailClassification::CloudOutstanding,
    );
    validate_conversation_action_pins(&action, &current).unwrap();
    current.pins.tail.local_only_subagents_kept = !action.pins.tail.local_only_subagents_kept;
    assert_eq!(
        validate_conversation_action_pins(&action, &current),
        Err(
            mallard_infrastructure::conversation_sync::ConversationStalePin::Tail(
                mallard_infrastructure::conversation_sync::ConversationTailPin::Subagents,
            )
        )
    );
}

#[test]
fn malformed_evidence_is_a_typed_blocker_not_a_classifier_state() {
    let tail = whole_tail(&[("m-a", "one")]);
    assert_eq!(
        evidence_of(Some(&tail), None, Some(&tail)),
        Err(TailEvidenceBlocker::ExpectedSideMissing)
    );
    let decision = default_conversation_decision(
        ConversationPlanDirection::Pull,
        TailClassification::Unknown,
        ConversationCapabilities::default(),
    );
    assert_eq!(decision.action, ConversationReviewAction::Blocked);
    assert!(!decision.selectable);
}

fn review_only_pins(
    accepted: Option<PortableConversationTail>,
    local: Option<PortableConversationTail>,
    cloud: Option<PortableConversationTail>,
) -> ConversationActionPins {
    action_pins(None, accepted, local, cloud)
}

pub(super) fn reviewed_action(
    action_type: ConversationPlanActionType,
    relation: TailClassification,
) -> (ConversationPlanAction, ConversationApplyContext) {
    let (accepted, local, cloud) = tails_for_relation(relation);
    reviewed_action_with_tails(action_type, accepted, local, cloud)
}

fn reviewed_action_with_tails(
    action_type: ConversationPlanActionType,
    accepted: Option<PortableConversationTail>,
    local: Option<PortableConversationTail>,
    cloud: Option<PortableConversationTail>,
) -> (ConversationPlanAction, ConversationApplyContext) {
    let pins = action_pins(Some(action_type), accepted, local, cloud);
    // Intent only: identities (provider ids, keep-both facts) are minted at
    // apply and cannot inhabit a plan.
    let action = ConversationPlanAction {
        action_id: ActionId::parse("action-thread").unwrap(),
        action: action_type,
        provider: codex(),
        thread_id: (action_type != ConversationPlanActionType::CreateLocal)
            .then(|| ThreadId::parse("thread-a").unwrap()),
        cloud_thread_id: (action_type != ConversationPlanActionType::CreateCloud)
            .then(|| CloudThreadId::parse("cloudthread-a").unwrap()),
        source_resource_id: Some(ResourceId::parse("codex:conversation:source").unwrap()),
        pins,
    };
    action.validate().unwrap();
    let current = ConversationApplyContext {
        now: 1_500,
        pins: action.pins.clone(),
    };
    (action, current)
}

fn action_pins(
    action_type: Option<ConversationPlanActionType>,
    accepted: Option<PortableConversationTail>,
    local: Option<PortableConversationTail>,
    cloud: Option<PortableConversationTail>,
) -> ConversationActionPins {
    let publishes = matches!(
        action_type,
        Some(ConversationPlanActionType::Publish | ConversationPlanActionType::CreateCloud)
    );
    let materializes = matches!(
        action_type,
        Some(
            ConversationPlanActionType::MaterializeCloud
                | ConversationPlanActionType::KeepBoth
                | ConversationPlanActionType::CreateLocal
        )
    );
    let provider_state = materializes
        .then(|| ProviderStateExpectation {
            target_path: "/tmp/provider/rollout-thread-reserved.jsonl".to_string(),
            expectation: TargetExpectation::Absent,
        })
        .into_iter()
        .collect();
    ConversationActionPins {
        subagent_selection: Default::default(),
        credential_redaction_enabled: true,
export_settings_changed: false,
context: ConversationContextPins {
            host_id: HostId::parse("11111111111111111111111111111111").unwrap(),
            replica_id: ReplicaId::parse("replica-a").unwrap(),
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            provider_profile_id: LocalProviderProfileId::parse("profile-a").unwrap(),
            host_identity_revision: 2,
            profile_revision: 3,
            project_binding_revision: 8,
            bindings_document_revision: 4,
            binding_revision: Some(5),
        },
        revision: ConversationRevisionPins {
            bundle_revision: 7,
            commit_id: "commit-abcdef".to_string(),
            manifest_sha256: "a".repeat(64),
            scope_revision: 6,
            scope_sha256: "f".repeat(64),
        },
        tail: ConversationTailPins {
            subagents: Default::default(),
            subagents_omitted: false,
            local_only_subagents_kept: false,
            missing_local_subagents: Default::default(),
            accepted: accepted.clone(),
            accepted_local: accepted,
            local,
            cloud,
            checkpoint_generation: None,
            lineage_digest: materializes.then(|| "b".repeat(64)),
            semantic_digest: materializes.then(|| "c".repeat(64)),
            raw_range: publishes.then(|| RawRangePin {
                start_byte_offset: 0,
                end_byte_offset: 256,
                range_sha256: "9".repeat(64),
            }),
        },
        target: ConversationTargetPins {
            local: TargetExpectation::Sha256("d".repeat(64)),
            provider_state,
        },
        codec: "codex.lineage.v1".to_string(),
        expires_at: 2_000,
    }
}
