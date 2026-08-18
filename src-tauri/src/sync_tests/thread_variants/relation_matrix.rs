use super::harness::{analyzed_chain, chain, codex};
use crate::project_sync::conversation_sync::{
    build_conversation_plan_action, validate_conversation_action_pins, ConversationApplyContext,
    ConversationPlanActionRequest,
};
use crate::project_sync::domain::{
    compare_conversation_tail, compute_portable_conversation_tail, default_conversation_decision,
    ActionId, ClassifiedThread, ConversationActionPins, ConversationCapabilities,
    ConversationContextPins, ConversationPlanAction, ConversationPlanActionType,
    ConversationPlanDirection, ConversationReviewAction, ConversationRevisionPins,
    ConversationTailPins, ConversationTargetPins, HostId, KeepBothPlan, LocalProjectId,
    LocalProviderProfileId, PortableConversationTail, PreservedProvenance,
    ProviderStateExpectation, RawRangePin, ReplicaId, ResourceId, SyncCurrency, TailClassification,
    TailEvidenceBlocker, TailPartitionKind, TailSideEvidence, TargetExpectation, ThreadId,
    VerifiedTailEvidence,
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
            let planned = build_conversation_plan_action(ConversationPlanActionRequest {
                action_id: ActionId::parse(action_id).unwrap(),
                direction,
                classified,
                provider: codex(),
                thread_id: ThreadId::parse("thread-a").unwrap(),
                source_resource_id: Some(ResourceId::parse("codex:conversation:source").unwrap()),
                proposed_provider_thread_id: template_action
                    .as_ref()
                    .and_then(|action| action.proposed_provider_thread_id.clone()),
                keep_both: template_action
                    .as_ref()
                    .and_then(|action| action.keep_both.clone()),
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

#[test]
fn malformed_evidence_is_a_typed_blocker_not_a_classifier_state() {
    let tail = whole_tail(&[("m-a", "one")]);
    assert_eq!(
        evidence_of(None, Some(&tail), Some(&tail)),
        Err(TailEvidenceBlocker::UnacceptedOverlap)
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
    let proposed_provider_thread_id = matches!(
        action_type,
        ConversationPlanActionType::MaterializeCloud | ConversationPlanActionType::CreateLocal
    )
    .then(|| "0199a1b2-0000-7000-8000-0000000000aa".to_string());
    let keep_both = (action_type == ConversationPlanActionType::KeepBoth).then(|| KeepBothPlan {
        preserved_thread_id: ThreadId::parse("thread-preserved").unwrap(),
        remote_provider_thread_id: "0199a1b2-0000-7000-8000-0000000000bb".to_string(),
        preserved: PreservedProvenance {
            preserved_from: ThreadId::parse("thread-a").unwrap(),
            ordinal: 1,
        },
        preserved_display_title: "Thread test conversation (Preserved 1 · Machine A)".to_string(),
    });
    let action = ConversationPlanAction {
        action_id: ActionId::parse("action-thread").unwrap(),
        action: action_type,
        provider: codex(),
        thread_id: ThreadId::parse("thread-a").unwrap(),
        source_resource_id: Some(ResourceId::parse("codex:conversation:source").unwrap()),
        proposed_provider_thread_id,
        keep_both,
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
