use super::harness::{analyzed_chain, chain, codex, TranscriptBuilder};
use crate::project_sync::conversation_codec::rewrite_codex_transcript;
use crate::project_sync::conversation_sync::{
    build_conversation_plan_action, default_conversation_decision,
    validate_conversation_action_pins, ConversationApplyContext, ConversationCapabilities,
    ConversationPlanActionRequest,
};
use crate::project_sync::domain::{
    classify_conversation_relation, ActionId, CodexForkExecutor, ConversationDivergencePlan,
    ConversationPlanAction, ConversationPlanActionType, ConversationPlanDirection,
    ConversationReasonCode, ConversationRelation, DivergenceExecution, HostId, LocalProjectId,
    LocalProviderProfileId, PlanId, ProviderStateExpectation, ReplicaId, ResourceId,
    TargetExpectation, ThreadId,
};
use std::collections::BTreeMap;

#[derive(Clone)]
struct MatrixCase {
    name: &'static str,
    local: Option<Vec<crate::project_sync::domain::CanonicalConversationMessage>>,
    cloud: Option<Vec<crate::project_sync::domain::CanonicalConversationMessage>>,
    relation: ConversationRelation,
    push: ConversationPlanActionType,
    pull: ConversationPlanActionType,
}

fn messages(
    ids: &[(&str, &str)],
) -> Vec<crate::project_sync::domain::CanonicalConversationMessage> {
    analyzed_chain(&chain("thread-a", ids))
}

#[test]
fn complete_push_pull_relation_matrix_plans_and_revalidates() {
    let base = messages(&[("m-a", "one"), ("m-b", "two")]);
    let local_long = messages(&[("m-a", "one"), ("m-b", "two"), ("m-c", "local")]);
    let cloud_long = messages(&[("m-a", "one"), ("m-b", "two"), ("m-d", "cloud")]);
    let changed_semantics = messages(&[("m-a", "changed"), ("m-b", "two")]);
    let different_ids = messages(&[("m-x", "one"), ("m-y", "two")]);
    let cases = vec![
        MatrixCase {
            name: "identical",
            local: Some(base.clone()),
            cloud: Some(base.clone()),
            relation: ConversationRelation::Identical,
            push: ConversationPlanActionType::NoOp,
            pull: ConversationPlanActionType::NoOp,
        },
        MatrixCase {
            name: "cloud_prefix_of_local",
            local: Some(local_long.clone()),
            cloud: Some(base.clone()),
            relation: ConversationRelation::CloudPrefixOfLocal,
            push: ConversationPlanActionType::FastForwardCloud,
            pull: ConversationPlanActionType::KeepLocal,
        },
        MatrixCase {
            name: "local_prefix_of_cloud",
            local: Some(base.clone()),
            cloud: Some(cloud_long.clone()),
            relation: ConversationRelation::LocalPrefixOfCloud,
            push: ConversationPlanActionType::RequirePull,
            pull: ConversationPlanActionType::FastForwardLocal,
        },
        MatrixCase {
            name: "fork_after_common_prefix",
            local: Some(local_long),
            cloud: Some(cloud_long),
            relation: ConversationRelation::Diverged,
            push: ConversationPlanActionType::RequirePull,
            pull: ConversationPlanActionType::ResolveDivergence,
        },
        MatrixCase {
            name: "reused_ids_changed_semantics",
            local: Some(changed_semantics),
            cloud: Some(base.clone()),
            relation: ConversationRelation::Unknown,
            push: ConversationPlanActionType::Blocked,
            pull: ConversationPlanActionType::StoreOnly,
        },
        MatrixCase {
            name: "same_text_different_ids",
            local: Some(different_ids),
            cloud: Some(base),
            relation: ConversationRelation::Diverged,
            push: ConversationPlanActionType::RequirePull,
            pull: ConversationPlanActionType::ResolveDivergence,
        },
        MatrixCase {
            name: "local_missing",
            local: None,
            cloud: Some(messages(&[("m-a", "one")])),
            relation: ConversationRelation::LocalMissing,
            push: ConversationPlanActionType::NoOp,
            pull: ConversationPlanActionType::CreateLocal,
        },
        MatrixCase {
            name: "cloud_missing",
            local: Some(messages(&[("m-a", "one")])),
            cloud: None,
            relation: ConversationRelation::CloudMissing,
            push: ConversationPlanActionType::CreateCloud,
            pull: ConversationPlanActionType::KeepLocal,
        },
        MatrixCase {
            name: "both_missing",
            local: None,
            cloud: None,
            relation: ConversationRelation::BothMissing,
            push: ConversationPlanActionType::NoOp,
            pull: ConversationPlanActionType::NoOp,
        },
    ];

    for case in cases {
        let relation = classify_conversation_relation(case.local.as_deref(), case.cloud.as_deref());
        assert_eq!(relation, case.relation, "{} relation", case.name);
        for (direction, expected) in [
            (ConversationPlanDirection::Push, case.push),
            (ConversationPlanDirection::Pull, case.pull),
        ] {
            let decision = default_conversation_decision(
                direction,
                relation,
                ConversationCapabilities::default(),
            );
            assert_eq!(decision.action, expected, "{} {direction:?}", case.name);
            let (template, current) =
                reviewed_action(direction, decision.action, decision.reason_code, relation);
            let action = build_conversation_plan_action(ConversationPlanActionRequest {
                action_id: template.action_id,
                direction,
                relation,
                capabilities: ConversationCapabilities::default(),
                provider: template.provider,
                host_id: template.host_id,
                replica_id: template.replica_id,
                local_project_id: template.local_project_id,
                provider_profile_id: template.provider_profile_id,
                thread_id: template.thread_id,
                divergence: template.divergence,
                source_resource_id: template.source_resource_id,
                proposed_provider_thread_id: template.proposed_provider_thread_id,
                required_codec: template.required_codec,
                pinned_revision: template.pinned_revision,
                pinned_commit_id: template.pinned_commit_id,
                pinned_manifest_sha256: template.pinned_manifest_sha256,
                pinned_scope_revision: template.pinned_scope_revision,
                pinned_scope_sha256: template.pinned_scope_sha256,
                pinned_lineage_digest: template.pinned_lineage_digest,
                pinned_semantic_digest: template.pinned_semantic_digest,
                expected_local_target: template.expected_local_target,
                expected_provider_state: template.expected_provider_state,
                expected_profile_revision: template.expected_profile_revision,
                expected_project_binding_revision: template.expected_project_binding_revision,
                expected_host_identity_revision: template.expected_host_identity_revision,
                expected_bindings_document_revision: template.expected_bindings_document_revision,
                expected_binding_revision: template.expected_binding_revision,
                expires_at: template.expires_at,
            })
            .unwrap();
            assert_eq!(
                action.relation, case.relation,
                "{} {direction:?}",
                case.name
            );
            assert_eq!(action.action, expected, "{} {direction:?}", case.name);
            validate_conversation_action_pins(&action, &current).unwrap();
        }
    }
}

#[test]
fn verified_structural_rewrite_is_a_true_noop() {
    let source = TranscriptBuilder::new("thread-a", "/tmp/project-a")
        .message("m-a", "user", "same")
        .message("m-b", "assistant", "same answer")
        .build();
    let rewritten = rewrite_codex_transcript(
        &source,
        "thread-a",
        "thread-b",
        "/tmp/project-a",
        "/tmp/project-b",
    )
    .unwrap();
    assert_ne!(source, rewritten);
    let local = analyzed_chain(&rewritten);
    let cloud = analyzed_chain(&source);
    assert_eq!(
        classify_conversation_relation(Some(&local), Some(&cloud)),
        ConversationRelation::Identical
    );
    for direction in [
        ConversationPlanDirection::Push,
        ConversationPlanDirection::Pull,
    ] {
        let decision = default_conversation_decision(
            direction,
            ConversationRelation::Identical,
            ConversationCapabilities {
                structural_rewrite_only: true,
                ..ConversationCapabilities::default()
            },
        );
        assert_eq!(decision.action, ConversationPlanActionType::NoOp);
        assert_eq!(
            decision.reason_code,
            ConversationReasonCode::StructuralRewriteOnly
        );
    }
}

#[test]
fn pull_fast_forward_without_quiescence_is_store_only() {
    let decision = default_conversation_decision(
        ConversationPlanDirection::Pull,
        ConversationRelation::LocalPrefixOfCloud,
        ConversationCapabilities {
            provider_quiescent: false,
            ..ConversationCapabilities::default()
        },
    );
    assert_eq!(decision.action, ConversationPlanActionType::StoreOnly);
    assert_eq!(
        decision.reason_code,
        ConversationReasonCode::ProviderNotQuiescent
    );
}

pub(super) fn reviewed_action(
    direction: ConversationPlanDirection,
    action_type: ConversationPlanActionType,
    reason_code: ConversationReasonCode,
    relation: ConversationRelation,
) -> (ConversationPlanAction, ConversationApplyContext) {
    let mut provider_state = Vec::new();
    let mut current_provider_state = BTreeMap::new();
    let mut provider_thread_id = None;
    if matches!(
        action_type,
        ConversationPlanActionType::FastForwardLocal
            | ConversationPlanActionType::CreateLocal
            | ConversationPlanActionType::ReplaceLocal
    ) {
        provider_thread_id = Some("thread-reserved".to_string());
        let target = "/tmp/provider/rollout-thread-reserved.jsonl".to_string();
        provider_state.push(ProviderStateExpectation {
            target_path: target.clone(),
            expectation: TargetExpectation::Absent,
        });
        current_provider_state.insert(target, TargetExpectation::Absent);
    }
    let divergence = (action_type == ConversationPlanActionType::ResolveDivergence).then(|| {
        let target = "/tmp/provider/rollout-source-native.jsonl".to_string();
        provider_state.push(ProviderStateExpectation {
            target_path: target.clone(),
            expectation: TargetExpectation::Sha256("d".repeat(64)),
        });
        current_provider_state.insert(target, TargetExpectation::Sha256("d".repeat(64)));
        ConversationDivergencePlan {
            available_resolutions: vec![
                crate::project_sync::domain::ConversationDivergenceResolution::PreserveLocalWithNativeFork,
                crate::project_sync::domain::ConversationDivergenceResolution::UseCloudOnly,
            ],
            recommended_resolution:
                crate::project_sync::domain::ConversationDivergenceResolution::PreserveLocalWithNativeFork,
            local_branch_preserved: false,
            preserved_thread_id: ThreadId::parse("thread-preserved").unwrap(),
            source_provider_thread_id: "source-native".to_string(),
            local_lineage_digest: "b".repeat(64),
            local_semantic_digest: "c".repeat(64),
            local_message_count: 3,
            pinned_local_title: Some("Local title".to_string()),
            pinned_cloud_title: Some("Cloud title".to_string()),
            desired_fork_name:
                "Local title — local before Pull · 2026-07-29 18:42Z".to_string(),
            desired_original_name: Some("Cloud title".to_string()),
            execution: DivergenceExecution::CodexNativeFork {
                codex_executable: "/tmp/codex".to_string(),
                codex_executable_sha256: "e".repeat(64),
                available_executors: vec![
                    CodexForkExecutor::AppServerStdio,
                    CodexForkExecutor::NativePty,
                ],
            },
        }
    });
    let host_id = HostId::parse("11111111111111111111111111111111").unwrap();
    let replica_id = ReplicaId::parse("replica-a").unwrap();
    let local_project_id = LocalProjectId::parse("project-a").unwrap();
    let provider_profile_id = LocalProviderProfileId::parse("profile-a").unwrap();
    let action = ConversationPlanAction {
        action_id: ActionId::parse("action-thread").unwrap(),
        direction,
        action: action_type,
        reason_code,
        relation,
        provider: codex(),
        host_id: host_id.clone(),
        replica_id: replica_id.clone(),
        local_project_id: local_project_id.clone(),
        provider_profile_id: provider_profile_id.clone(),
        thread_id: ThreadId::parse("thread-a").unwrap(),
        divergence,
        source_resource_id: Some(ResourceId::parse("codex:conversation:source").unwrap()),
        proposed_provider_thread_id: provider_thread_id,
        required_codec: "codex.lineage.v1".to_string(),
        pinned_revision: 7,
        pinned_commit_id: "commit-abcdef".to_string(),
        pinned_manifest_sha256: "a".repeat(64),
        pinned_scope_revision: 6,
        pinned_scope_sha256: "f".repeat(64),
        pinned_lineage_digest: Some("b".repeat(64)),
        pinned_semantic_digest: Some("c".repeat(64)),
        expected_local_target: TargetExpectation::Sha256("d".repeat(64)),
        expected_provider_state: provider_state,
        expected_profile_revision: 3,
        expected_project_binding_revision: 8,
        expected_host_identity_revision: 2,
        expected_bindings_document_revision: 4,
        expected_binding_revision: Some(5),
        expires_at: 2_000,
    };
    let current = ConversationApplyContext {
        now: 1_500,
        host_id,
        replica_id,
        local_project_id,
        provider_profile_id,
        host_identity_revision: 2,
        revision: 7,
        commit_id: "commit-abcdef".to_string(),
        manifest_sha256: "a".repeat(64),
        scope_revision: 6,
        scope_sha256: "f".repeat(64),
        lineage_digest: Some("b".repeat(64)),
        semantic_digest: Some("c".repeat(64)),
        local_target: TargetExpectation::Sha256("d".repeat(64)),
        provider_state: current_provider_state,
        profile_revision: 3,
        project_binding_revision: 8,
        bindings_document_revision: 4,
        binding_revision: Some(5),
        codec: "codex.lineage.v1".to_string(),
        provider_quiescent: true,
    };
    let _ = PlanId::parse("plan-unused").unwrap();
    (action, current)
}
