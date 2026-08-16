use super::harness::{analyzed_chain, chain};
use crate::project_sync::conversation_sync::{
    default_conversation_decision, ConversationCapabilities,
};
use crate::project_sync::domain::{
    compare_conversation_tail, compute_portable_conversation_tail, ConversationPlanActionType,
    ConversationPlanDirection, SyncCurrency, TailClassification, TailComparisonEvidence,
    TailPartitionKind, TailSideEvidence,
};

fn whole_tail(chain: &[mallard_domain::CanonicalConversationMessage]) -> TailSideEvidence {
    TailSideEvidence::Present(
        compute_portable_conversation_tail(TailPartitionKind::WholeThread, 0, chain).unwrap(),
    )
}

/// Single-form evidence: the accepted wire pin and its local twin carry the
/// same value, as for providers whose wire bytes equal their local bytes.
fn evidence(
    accepted: TailSideEvidence,
    local: TailSideEvidence,
    cloud: TailSideEvidence,
) -> TailComparisonEvidence {
    TailComparisonEvidence {
        accepted_local: accepted.clone(),
        accepted,
        local,
        cloud,
        local_provenance_verified: true,
        accepted_witness_verified: true,
        cloud_ancestry_verified: true,
        coordinates_verified: true,
    }
}

#[test]
fn divergence_is_pull_first_and_pull_defaults_to_keep_both() {
    let base = analyzed_chain(&chain(
        "thread-a",
        &[("m-1", "shared"), ("m-2", "shared answer")],
    ));
    let local = analyzed_chain(&chain(
        "thread-a",
        &[
            ("m-1", "shared"),
            ("m-2", "shared answer"),
            ("m-local", "local branch"),
        ],
    ));
    let cloud = analyzed_chain(&chain(
        "thread-a",
        &[
            ("m-1", "shared"),
            ("m-2", "shared answer"),
            ("m-cloud", "cloud branch"),
        ],
    ));
    let relation = compare_conversation_tail(evidence(
        whole_tail(&base),
        whole_tail(&local),
        whole_tail(&cloud),
    ));
    assert_eq!(relation, TailClassification::Divergent);

    let push = default_conversation_decision(
        ConversationPlanDirection::Push,
        relation,
        ConversationCapabilities::default(),
    );
    assert_eq!(push.action, ConversationPlanActionType::RequirePull);
    assert!(!push.selectable);

    let pull = default_conversation_decision(
        ConversationPlanDirection::Pull,
        relation,
        ConversationCapabilities::default(),
    );
    assert_eq!(pull.action, ConversationPlanActionType::KeepBoth);
    assert!(pull.selectable);
}

#[test]
fn unknown_evidence_never_becomes_a_push() {
    let push = default_conversation_decision(
        ConversationPlanDirection::Push,
        TailClassification::Unknown,
        ConversationCapabilities::default(),
    );
    assert_eq!(push.action, ConversationPlanActionType::Blocked);
    assert!(!push.selectable);

    let pull = default_conversation_decision(
        ConversationPlanDirection::Pull,
        TailClassification::Unknown,
        ConversationCapabilities::default(),
    );
    assert_eq!(pull.action, ConversationPlanActionType::Blocked);
    assert!(!pull.selectable);
}

#[test]
fn a_resolved_original_branch_reaches_a_noop_fixed_point() {
    let cloud = analyzed_chain(&chain(
        "thread-a",
        &[("m-1", "shared"), ("m-cloud", "cloud branch")],
    ));
    let relation = compare_conversation_tail(evidence(
        whole_tail(&cloud),
        whole_tail(&cloud),
        whole_tail(&cloud),
    ));
    assert_eq!(relation, TailClassification::Sync(SyncCurrency::Current));
    let push = default_conversation_decision(
        ConversationPlanDirection::Push,
        relation,
        ConversationCapabilities::default(),
    );
    assert_eq!(push.action, ConversationPlanActionType::NoChanges);
    assert!(!push.selectable);
    let pull = default_conversation_decision(
        ConversationPlanDirection::Pull,
        relation,
        ConversationCapabilities::default(),
    );
    assert_eq!(pull.action, ConversationPlanActionType::NoChanges);
    assert!(!pull.selectable);
}
