use super::harness::{analyzed_chain, chain};
use crate::project_sync::conversation_sync::{
    default_conversation_decision, ConversationCapabilities,
};
use crate::project_sync::domain::{
    classify_conversation_relation, ConversationPlanActionType, ConversationPlanDirection,
    ConversationRelation,
};

#[test]
fn divergence_is_pull_first_and_pull_defaults_to_keep_both() {
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
    let relation = classify_conversation_relation(Some(&local), Some(&cloud));
    assert_eq!(relation, ConversationRelation::Diverged);

    let push = default_conversation_decision(
        ConversationPlanDirection::Push,
        relation,
        ConversationCapabilities::default(),
    );
    assert_eq!(push.action, ConversationPlanActionType::RequirePull);
    assert!(!push.selectable);
    assert!(!push.selected_by_default);

    let pull = default_conversation_decision(
        ConversationPlanDirection::Pull,
        relation,
        ConversationCapabilities::default(),
    );
    assert_eq!(pull.action, ConversationPlanActionType::ResolveDivergence);
    assert!(pull.selectable);
    assert!(pull.selected_by_default);
    assert!(!pull.requires_explicit_approval);
}

#[test]
fn unknown_lineage_never_becomes_a_push_fork() {
    let push = default_conversation_decision(
        ConversationPlanDirection::Push,
        ConversationRelation::Unknown,
        ConversationCapabilities::default(),
    );
    assert_eq!(push.action, ConversationPlanActionType::Blocked);
    assert!(!push.selectable);

    let pull = default_conversation_decision(
        ConversationPlanDirection::Pull,
        ConversationRelation::Unknown,
        ConversationCapabilities::default(),
    );
    assert_eq!(pull.action, ConversationPlanActionType::StoreOnly);
    assert!(!pull.selectable);
}

#[test]
fn a_resolved_original_branch_reaches_a_noop_fixed_point() {
    let cloud = analyzed_chain(&chain(
        "thread-a",
        &[("m-1", "shared"), ("m-cloud", "cloud branch")],
    ));
    assert_eq!(
        classify_conversation_relation(Some(&cloud), Some(&cloud)),
        ConversationRelation::Identical
    );
    for direction in [
        ConversationPlanDirection::Push,
        ConversationPlanDirection::Pull,
    ] {
        let decision = default_conversation_decision(
            direction,
            ConversationRelation::Identical,
            ConversationCapabilities::default(),
        );
        assert_eq!(decision.action, ConversationPlanActionType::NoOp);
        assert!(!decision.selectable);
    }
}
