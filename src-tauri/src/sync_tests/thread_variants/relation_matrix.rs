use super::harness::{analyzed_chain, chain, codex, rewrite_transcript_identity, TranscriptBuilder};
use crate::project_sync::conversation_sync::{
    build_conversation_plan_action, default_conversation_decision,
    validate_conversation_action_pins, ConversationApplyContext, ConversationCapabilities,
    ConversationPlanActionRequest,
};
use crate::project_sync::domain::{
    compare_conversation_tail, compute_portable_conversation_tail, ActionId,
    ConversationPlanAction, ConversationPlanActionType, ConversationPlanDirection,
    ConversationReasonCode, HostId, KeepBothPlan, LocalProjectId, LocalProviderProfileId,
    PortableConversationTail, ProviderStateExpectation, RawRangePin, ReplicaId, ResourceId,
    SyncCurrency, TailClassification, TailComparisonEvidence, TailPartitionKind, TailSideEvidence,
    TargetExpectation, ThreadId,
};
use std::collections::BTreeMap;

fn whole_tail(ids: &[(&str, &str)]) -> PortableConversationTail {
    compute_portable_conversation_tail(
        TailPartitionKind::WholeThread,
        0,
        &analyzed_chain(&chain("thread-a", ids)),
    )
    .unwrap()
}

fn evidence_of(
    accepted: Option<&PortableConversationTail>,
    local: Option<&PortableConversationTail>,
    cloud: Option<&PortableConversationTail>,
) -> TailComparisonEvidence {
    let side = |value: Option<&PortableConversationTail>| match value {
        Some(tail) => TailSideEvidence::Present(tail.clone()),
        None => TailSideEvidence::ConfirmedAbsent,
    };
    TailComparisonEvidence {
        accepted: side(accepted),
        accepted_local: side(accepted),
        local: side(local),
        cloud: side(cloud),
        local_provenance_verified: true,
        accepted_witness_verified: true,
        cloud_ancestry_verified: true,
        coordinates_verified: true,
    }
}

#[derive(Clone)]
struct MatrixCase {
    name: &'static str,
    accepted: Option<PortableConversationTail>,
    local: Option<PortableConversationTail>,
    cloud: Option<PortableConversationTail>,
    relation: TailClassification,
    push: ConversationPlanActionType,
    pull: ConversationPlanActionType,
}

/// The complete relation-to-action matrix: the four-row pinned table, every
/// directional absence, the deliberate absence of containment refinement,
/// and the unpinned-both-present bootstrap refusal — each planned into a
/// validated action and revalidated at apply pins.
#[test]
fn complete_push_pull_relation_matrix_plans_and_revalidates() {
    let base = whole_tail(&[("m-a", "one"), ("m-b", "two")]);
    let local_long = whole_tail(&[("m-a", "one"), ("m-b", "two"), ("m-c", "local")]);
    let cloud_long = whole_tail(&[("m-a", "one"), ("m-b", "two"), ("m-d", "cloud")]);
    let cloud_longer = whole_tail(&[
        ("m-a", "one"),
        ("m-b", "two"),
        ("m-c", "local"),
        ("m-e", "cloud"),
    ]);
    let cases = vec![
        MatrixCase {
            name: "sync_current",
            accepted: Some(base.clone()),
            local: Some(base.clone()),
            cloud: Some(base.clone()),
            relation: TailClassification::Sync(SyncCurrency::Current),
            push: ConversationPlanActionType::NoChanges,
            pull: ConversationPlanActionType::NoChanges,
        },
        MatrixCase {
            name: "sync_advanced",
            accepted: Some(base.clone()),
            local: Some(local_long.clone()),
            cloud: Some(local_long.clone()),
            relation: TailClassification::Sync(SyncCurrency::Advanced),
            push: ConversationPlanActionType::RequirePullMetadata,
            pull: ConversationPlanActionType::AdoptCloudMetadata,
        },
        MatrixCase {
            name: "local_outstanding",
            accepted: Some(base.clone()),
            local: Some(local_long.clone()),
            cloud: Some(base.clone()),
            relation: TailClassification::LocalOutstanding,
            push: ConversationPlanActionType::Publish,
            pull: ConversationPlanActionType::ReportPushEligible,
        },
        MatrixCase {
            name: "cloud_outstanding",
            accepted: Some(base.clone()),
            local: Some(base.clone()),
            cloud: Some(cloud_long.clone()),
            relation: TailClassification::CloudOutstanding,
            push: ConversationPlanActionType::RequirePull,
            pull: ConversationPlanActionType::MaterializeCloud,
        },
        MatrixCase {
            name: "fork_after_common_prefix",
            accepted: Some(base.clone()),
            local: Some(local_long.clone()),
            cloud: Some(cloud_long.clone()),
            relation: TailClassification::Divergent,
            push: ConversationPlanActionType::RequirePull,
            pull: ConversationPlanActionType::KeepBoth,
        },
        // Canonical containment without pin alignment is still Divergent:
        // there is deliberately no containment refinement.
        MatrixCase {
            name: "containment_is_divergent",
            accepted: Some(base.clone()),
            local: Some(local_long.clone()),
            cloud: Some(cloud_longer),
            relation: TailClassification::Divergent,
            push: ConversationPlanActionType::RequirePull,
            pull: ConversationPlanActionType::KeepBoth,
        },
        MatrixCase {
            name: "missing_local",
            accepted: None,
            local: None,
            cloud: Some(base.clone()),
            relation: TailClassification::MissingLocal,
            push: ConversationPlanActionType::NotPushable,
            pull: ConversationPlanActionType::CreateLocal,
        },
        MatrixCase {
            name: "missing_cloud",
            accepted: None,
            local: Some(base.clone()),
            cloud: None,
            relation: TailClassification::MissingCloud,
            push: ConversationPlanActionType::CreateCloud,
            pull: ConversationPlanActionType::ReportLocalOnly,
        },
        MatrixCase {
            name: "missing_both",
            accepted: None,
            local: None,
            cloud: None,
            relation: TailClassification::MissingBoth,
            push: ConversationPlanActionType::NotPushable,
            pull: ConversationPlanActionType::NotFound,
        },
        // Pull does not bootstrap by comparing graphs.
        MatrixCase {
            name: "unpinned_both_present",
            accepted: None,
            local: Some(base.clone()),
            cloud: Some(base.clone()),
            relation: TailClassification::Unknown,
            push: ConversationPlanActionType::Blocked,
            pull: ConversationPlanActionType::Blocked,
        },
    ];

    for case in cases {
        let relation = compare_conversation_tail(evidence_of(
            case.accepted.as_ref(),
            case.local.as_ref(),
            case.cloud.as_ref(),
        ));
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
            // A durably pinned current thread plans no Pull row at all:
            // provider_thread_review skips building one, and `NoChanges`
            // stays a Push-only action inside validated plans.
            if direction == ConversationPlanDirection::Pull
                && decision.action == ConversationPlanActionType::NoChanges
            {
                assert!(!decision.selectable, "{} pull no-op", case.name);
                continue;
            }
            let (template, current) = reviewed_action_with_tails(
                direction,
                decision.action,
                decision.reason_code,
                relation,
                case.accepted.clone(),
                case.local.clone(),
                case.cloud.clone(),
            );
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
                source_resource_id: template.source_resource_id,
                proposed_provider_thread_id: template.proposed_provider_thread_id,
                keep_both: template.keep_both,
                required_codec: template.required_codec,
                pinned_revision: template.pinned_revision,
                pinned_commit_id: template.pinned_commit_id,
                pinned_manifest_sha256: template.pinned_manifest_sha256,
                pinned_scope_revision: template.pinned_scope_revision,
                pinned_scope_sha256: template.pinned_scope_sha256,
                pinned_accepted_tail: template.pinned_accepted_tail,
                pinned_accepted_local_tail: template.pinned_accepted_local_tail,
                pinned_local_tail: template.pinned_local_tail,
                pinned_cloud_tail: template.pinned_cloud_tail,
                pinned_checkpoint_generation: template.pinned_checkpoint_generation,
                pinned_lineage_digest: template.pinned_lineage_digest,
                pinned_semantic_digest: template.pinned_semantic_digest,
                pinned_raw_range: template.pinned_raw_range,
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

    // The same current evidence without a durable accepted pin still adopts
    // on Pull: first-contact adoption is what writes the pin.
    let probe = ConversationCapabilities {
        acceptance_durably_pinned: false,
        ..ConversationCapabilities::default()
    };
    let first_contact = default_conversation_decision(
        ConversationPlanDirection::Pull,
        TailClassification::Sync(SyncCurrency::Current),
        probe,
    );
    assert_eq!(
        first_contact.action,
        ConversationPlanActionType::AdoptCloudMetadata
    );
    assert!(first_contact.selectable);
}

/// A faithful provider rewrite (thread ID and cwd restamp) leaves every
/// canonical value identical, so the tails agree and both directions settle
/// without any special structural-rewrite refinement.
#[test]
fn verified_structural_rewrite_is_in_sync() {
    let source = TranscriptBuilder::new("thread-a", "/tmp/project-a")
        .message("m-a", "user", "same")
        .message("m-b", "assistant", "same answer")
        .build();
    let rewritten = rewrite_transcript_identity(
        &source,
        "thread-a",
        "thread-b",
        "/tmp/project-a",
        "/tmp/project-b",
    )
    .unwrap();
    assert_ne!(source, rewritten);
    let local = compute_portable_conversation_tail(
        TailPartitionKind::WholeThread,
        0,
        &analyzed_chain(&rewritten),
    )
    .unwrap();
    let cloud = compute_portable_conversation_tail(
        TailPartitionKind::WholeThread,
        0,
        &analyzed_chain(&source),
    )
    .unwrap();
    assert_eq!(local, cloud);
    let relation = compare_conversation_tail(evidence_of(Some(&cloud), Some(&local), Some(&cloud)));
    assert_eq!(relation, TailClassification::Sync(SyncCurrency::Current));
}

#[test]
fn pull_materialization_without_quiescence_is_blocked() {
    let decision = default_conversation_decision(
        ConversationPlanDirection::Pull,
        TailClassification::CloudOutstanding,
        ConversationCapabilities {
            provider_quiescent: false,
            ..ConversationCapabilities::default()
        },
    );
    assert_eq!(decision.action, ConversationPlanActionType::Blocked);
    assert_eq!(
        decision.reason_code,
        ConversationReasonCode::ProviderNotQuiescent
    );
}

pub(super) fn reviewed_action(
    direction: ConversationPlanDirection,
    action_type: ConversationPlanActionType,
    reason_code: ConversationReasonCode,
    relation: TailClassification,
) -> (ConversationPlanAction, ConversationApplyContext) {
    let base = whole_tail(&[("m-a", "one"), ("m-b", "two")]);
    let longer = whole_tail(&[("m-a", "one"), ("m-b", "two"), ("m-c", "local")]);
    let (accepted, local, cloud) = match relation {
        TailClassification::Sync(SyncCurrency::Current) => {
            (Some(base.clone()), Some(base.clone()), Some(base.clone()))
        }
        TailClassification::Sync(SyncCurrency::Advanced) => (
            Some(base.clone()),
            Some(longer.clone()),
            Some(longer.clone()),
        ),
        TailClassification::LocalOutstanding => {
            (Some(base.clone()), Some(longer.clone()), Some(base.clone()))
        }
        TailClassification::CloudOutstanding => {
            (Some(base.clone()), Some(base.clone()), Some(longer.clone()))
        }
        TailClassification::Divergent => (
            Some(base.clone()),
            Some(longer.clone()),
            Some(whole_tail(&[("m-a", "one"), ("m-b", "two"), ("m-d", "x")])),
        ),
        TailClassification::MissingCloud => (None, Some(base.clone()), None),
        TailClassification::MissingLocal => (None, None, Some(base.clone())),
        TailClassification::MissingBoth => (None, None, None),
        TailClassification::Unknown => (None, Some(base.clone()), Some(base.clone())),
    };
    reviewed_action_with_tails(
        direction,
        action_type,
        reason_code,
        relation,
        accepted,
        local,
        cloud,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reviewed_action_with_tails(
    direction: ConversationPlanDirection,
    action_type: ConversationPlanActionType,
    reason_code: ConversationReasonCode,
    relation: TailClassification,
    accepted: Option<PortableConversationTail>,
    local: Option<PortableConversationTail>,
    cloud: Option<PortableConversationTail>,
) -> (ConversationPlanAction, ConversationApplyContext) {
    let mut provider_state = Vec::new();
    let mut current_provider_state = BTreeMap::new();
    let mut provider_thread_id = None;
    let mut keep_both = None;
    let publishes = matches!(
        action_type,
        ConversationPlanActionType::Publish | ConversationPlanActionType::CreateCloud
    );
    let materializes = matches!(
        action_type,
        ConversationPlanActionType::MaterializeCloud
            | ConversationPlanActionType::KeepBoth
            | ConversationPlanActionType::CreateLocal
    );
    if matches!(
        action_type,
        ConversationPlanActionType::MaterializeCloud | ConversationPlanActionType::CreateLocal
    ) {
        provider_thread_id = Some("0199a1b2-0000-7000-8000-0000000000aa".to_string());
        let target = "/tmp/provider/rollout-thread-reserved.jsonl".to_string();
        provider_state.push(ProviderStateExpectation {
            target_path: target.clone(),
            expectation: TargetExpectation::Absent,
        });
        current_provider_state.insert(target, TargetExpectation::Absent);
    }
    if action_type == ConversationPlanActionType::KeepBoth {
        let target = "/tmp/provider/rollout-remote-branch.jsonl".to_string();
        provider_state.push(ProviderStateExpectation {
            target_path: target.clone(),
            expectation: TargetExpectation::Absent,
        });
        current_provider_state.insert(target, TargetExpectation::Absent);
        keep_both = Some(KeepBothPlan {
            preserved_thread_id: ThreadId::parse("thread-preserved").unwrap(),
            remote_provider_thread_id: "0199a1b2-0000-7000-8000-0000000000bb".to_string(),
        });
    }
    let raw_range = publishes.then(|| RawRangePin {
        start_byte_offset: 0,
        end_byte_offset: 256,
        range_sha256: "9".repeat(64),
    });
    let digests = materializes.then(|| ("b".repeat(64), "c".repeat(64)));
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
        source_resource_id: Some(ResourceId::parse("codex:conversation:source").unwrap()),
        proposed_provider_thread_id: provider_thread_id,
        keep_both,
        required_codec: "codex.lineage.v1".to_string(),
        pinned_revision: 7,
        pinned_commit_id: "commit-abcdef".to_string(),
        pinned_manifest_sha256: "a".repeat(64),
        pinned_scope_revision: 6,
        pinned_scope_sha256: "f".repeat(64),
        pinned_accepted_tail: accepted.clone(),
        pinned_accepted_local_tail: accepted.clone(),
        pinned_local_tail: local.clone(),
        pinned_cloud_tail: cloud.clone(),
        pinned_checkpoint_generation: None,
        pinned_lineage_digest: digests.as_ref().map(|(lineage, _)| lineage.clone()),
        pinned_semantic_digest: digests.as_ref().map(|(_, semantic)| semantic.clone()),
        pinned_raw_range: raw_range.clone(),
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
        accepted_tail: accepted.clone(),
        accepted_local_tail: accepted,
        local_tail: local,
        cloud_tail: cloud,
        checkpoint_generation: None,
        lineage_digest: digests.as_ref().map(|(lineage, _)| lineage.clone()),
        semantic_digest: digests.as_ref().map(|(_, semantic)| semantic.clone()),
        raw_range,
        local_target: TargetExpectation::Sha256("d".repeat(64)),
        provider_state: current_provider_state,
        profile_revision: 3,
        project_binding_revision: 8,
        bindings_document_revision: 4,
        binding_revision: Some(5),
        codec: "codex.lineage.v1".to_string(),
        provider_quiescent: true,
    };
    (action, current)
}

/// Plan section 13: relations come from exactly one production comparison
/// site. A second P/L/C comparison anywhere outside the shared classifier
/// is an architecture regression, whatever it computes.
#[test]
fn architecture_rejects_any_second_classifier_call_site() {
    fn rust_files(root: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, output);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                output.push(path);
            }
        }
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&manifest.join("src"), &mut files);
    rust_files(&manifest.join("../crates"), &mut files);
    let mut call_sites = Vec::new();
    for path in files {
        let display = path.display().to_string();
        if display.contains("/sync_tests/") || display.ends_with("/tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // Test modules are appended after production code in this codebase.
        let production = text.split("#[cfg(test)]").next().unwrap();
        for line in production.lines() {
            if line.contains("compare_conversation_tail(")
                && !line.contains("fn compare_conversation_tail")
            {
                call_sites.push(display.clone());
            }
        }
    }
    assert_eq!(
        call_sites.len(),
        1,
        "exactly one production classifier call site, found: {call_sites:?}"
    );
    assert!(call_sites[0].ends_with("commands.rs"), "{call_sites:?}");
}
