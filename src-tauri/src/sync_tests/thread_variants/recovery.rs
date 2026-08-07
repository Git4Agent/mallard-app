use super::harness::{sha256, TranscriptBuilder, VariantWorld};
use super::relation_matrix::reviewed_action;
use crate::project_sync::codex_native_fork::CodexForkExecutionRequest;
use crate::project_sync::commands::{
    diverged_fork_preservation, pending_codex_divergence_with_repository, LocalThreadReview,
};
use crate::project_sync::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage,
};
use crate::project_sync::conversation_materialization::{
    apply_codex_divergence_locked, apply_conversation_cloud_publication,
    apply_conversation_materialization, recover_conversation_cloud_publications,
    recover_conversation_materializations, retry_codex_divergence_locked,
    CodexDivergenceApplyOutcome, CodexDivergenceMaterializationRequest,
    ConversationCloudPublication, ConversationFailpoint, ConversationFailpoints,
    ConversationMaterializationRequest, ConversationTargetPayload, NoConversationFailpoints,
};
use crate::project_sync::conversation_sync::{
    validate_conversation_action_pins, BindingResolution, ConversationStalePin,
};
use crate::project_sync::domain::{
    conversation_resource_id, ActionId, BindingState, BundleId, BundleRecipe,
    CodexDivergenceOperation, CodexForkAttempt, CodexForkAttemptStatus, CodexForkExecutor,
    CodexInventoryRefreshStatus, ConversationCloudAnchor, ConversationDivergenceResolution,
    ConversationFileRole, ConversationJournalTarget, ConversationMaterializationState,
    ConversationNameStatus, ConversationOperationId, ConversationPlanActionType,
    ConversationPlanDirection, ConversationPushPlan, ConversationReasonCode, ConversationRelation,
    ConversationSyncOperation, ConversationSyncOperationKind, ConversationSyncPhase, ForkOrigin,
    ForkStamp, LocalConversationBinding, LocalProjectId, LocalProjectRegistration,
    LocalProviderProfileId, PlanId, ProjectBinding, Provider, ProviderProfile, ReplicaId,
    StorageId, TargetExpectation, ThreadId, PUSH_PLAN_SCHEMA,
};
use crate::project_sync::persistence::LocalStateRepository;
use crate::project_sync::provider_capture::{
    self, capture_selected, discover_project, CaptureRequest,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[test]
fn every_reviewed_pin_rejects_independently_before_mutation() {
    let (action, baseline) = reviewed_action(
        ConversationPlanDirection::Pull,
        ConversationPlanActionType::FastForwardLocal,
        ConversationReasonCode::LocalPrefixOfCloud,
        ConversationRelation::LocalPrefixOfCloud,
    );
    let cases: Vec<(
        ConversationStalePin,
        Box<dyn Fn(&mut crate::project_sync::conversation_sync::ConversationApplyContext)>,
    )> = vec![
        (
            ConversationStalePin::Expired,
            Box::new(|value| value.now = 9_999),
        ),
        (
            ConversationStalePin::Host,
            Box::new(|value| {
                value.host_id =
                    crate::project_sync::domain::HostId::parse("22222222222222222222222222222222")
                        .unwrap()
            }),
        ),
        (
            ConversationStalePin::Replica,
            Box::new(|value| {
                value.replica_id =
                    crate::project_sync::domain::ReplicaId::parse("replica-other").unwrap()
            }),
        ),
        (
            ConversationStalePin::LocalProject,
            Box::new(|value| {
                value.local_project_id = LocalProjectId::parse("project-other").unwrap()
            }),
        ),
        (
            ConversationStalePin::ProviderProfile,
            Box::new(|value| {
                value.provider_profile_id = LocalProviderProfileId::parse("profile-other").unwrap()
            }),
        ),
        (
            ConversationStalePin::HostIdentityRevision,
            Box::new(|value| value.host_identity_revision += 1),
        ),
        (
            ConversationStalePin::Revision,
            Box::new(|value| value.revision += 1),
        ),
        (
            ConversationStalePin::Commit,
            Box::new(|value| value.commit_id = "commit-other".to_string()),
        ),
        (
            ConversationStalePin::Manifest,
            Box::new(|value| value.manifest_sha256 = "e".repeat(64)),
        ),
        (
            ConversationStalePin::LineageDigest,
            Box::new(|value| value.lineage_digest = Some("e".repeat(64))),
        ),
        (
            ConversationStalePin::SemanticDigest,
            Box::new(|value| value.semantic_digest = Some("e".repeat(64))),
        ),
        (
            ConversationStalePin::LocalTarget,
            Box::new(|value| value.local_target = TargetExpectation::Sha256("e".repeat(64))),
        ),
        (
            ConversationStalePin::ProviderState,
            Box::new(|value| {
                value.provider_state.insert(
                    "/tmp/provider/occupied.jsonl".to_string(),
                    TargetExpectation::Absent,
                );
            }),
        ),
        (
            ConversationStalePin::ProfileRevision,
            Box::new(|value| value.profile_revision += 1),
        ),
        (
            ConversationStalePin::ProjectBindingRevision,
            Box::new(|value| value.project_binding_revision += 1),
        ),
        (
            ConversationStalePin::BindingsDocumentRevision,
            Box::new(|value| value.bindings_document_revision += 1),
        ),
        (
            ConversationStalePin::BindingRevision,
            Box::new(|value| value.binding_revision = Some(99)),
        ),
        (
            ConversationStalePin::Codec,
            Box::new(|value| value.codec = "codex.lineage.v2".to_string()),
        ),
        (
            ConversationStalePin::ProviderQuiescence,
            Box::new(|value| value.provider_quiescent = false),
        ),
    ];
    for (expected, mutate) in cases {
        let mut current = baseline.clone();
        mutate(&mut current);
        let before = current.clone();
        assert_eq!(
            validate_conversation_action_pins(&action, &current),
            Err(expected)
        );
        assert_eq!(current, before, "pin validation must not mutate state");
    }
}

#[test]
fn every_typed_action_rejects_a_stale_plan_before_apply() {
    let actions = [
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::NoOp,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::FastForwardLocal,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::FastForwardCloud,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::KeepLocal,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::RequirePull,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::ResolveDivergence,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::StoreOnly,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::CreateLocal,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::CreateCloud,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::ReplaceLocal,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::ReplaceCloud,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::TombstoneThread,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::Blocked,
        ),
    ];

    for (direction, action_type) in actions {
        let relation = if action_type == ConversationPlanActionType::ResolveDivergence {
            ConversationRelation::Diverged
        } else {
            ConversationRelation::Unknown
        };
        let (action, mut current) = reviewed_action(
            direction,
            action_type,
            ConversationReasonCode::StalePlan,
            relation,
        );
        action.validate().unwrap();
        current.revision += 1;
        assert_eq!(
            validate_conversation_action_pins(&action, &current),
            Err(ConversationStalePin::Revision),
            "{direction:?} {action_type:?}",
        );
    }
}

#[test]
fn reviewed_push_plan_is_immutable_restart_readable_and_scope_pinned() {
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let (action, _) = reviewed_action(
        ConversationPlanDirection::Push,
        ConversationPlanActionType::FastForwardCloud,
        ConversationReasonCode::CloudPrefixOfLocal,
        ConversationRelation::CloudPrefixOfLocal,
    );
    let plan = ConversationPushPlan {
        schema_version: PUSH_PLAN_SCHEMA,
        plan_id: PlanId::parse("plan-reviewed-push").unwrap(),
        storage_id: StorageId::parse("storage-reviewed-push").unwrap(),
        bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        host_id: action.host_id.clone(),
        replica_id: action.replica_id.clone(),
        local_project_id: action.local_project_id.clone(),
        provider_profile_id: action.provider_profile_id.clone(),
        expected_host_identity_revision: action.expected_host_identity_revision,
        expected_project_binding_revision: action.expected_project_binding_revision,
        expected_bindings_document_revision: action.expected_bindings_document_revision,
        expected_profile_revision: action.expected_profile_revision,
        revision: action.pinned_revision,
        commit_id: action.pinned_commit_id.clone(),
        manifest_sha256: action.pinned_manifest_sha256.clone(),
        created_at: 1_000,
        expires_at: action.expires_at,
        actions: vec![action],
    };
    repository.save_conversation_push_plan(&plan).unwrap();
    assert!(repository.save_conversation_push_plan(&plan).is_err());
    let reopened = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    assert_eq!(
        reopened.load_conversation_push_plan(&plan.plan_id).unwrap(),
        plan
    );

    let mut wrong_scope = plan.clone();
    wrong_scope.local_project_id = LocalProjectId::parse("project-other").unwrap();
    assert!(wrong_scope.validate().is_err());
    let mut duplicate = plan.clone();
    duplicate.actions.push(duplicate.actions[0].clone());
    assert!(duplicate.validate().is_err());
    assert!(reopened
        .discard_conversation_push_plan(&plan.plan_id)
        .unwrap());
    assert!(!reopened
        .discard_conversation_push_plan(&plan.plan_id)
        .unwrap());
}

#[test]
fn conversation_journal_is_revisioned_bounded_and_restart_readable() {
    let world = VariantWorld::new();
    let identity = world.repo_a.load_or_create_host_identity().unwrap();
    let operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-a").unwrap(),
        plan_id: Some(PlanId::parse("plan-a").unwrap()),
        plan_action_id: None,
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        cloud: Some(ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision: 1,
            commit_id: "commit-a".to_string(),
            manifest_sha256: "a".repeat(64),
        }),
        host_id: identity.host_id,
        replica_id: crate::project_sync::domain::ReplicaId::parse("replica-a").unwrap(),
        local_project_id: crate::project_sync::domain::LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: crate::project_sync::domain::LocalProviderProfileId::parse(
            "profile-a",
        )
        .unwrap(),
        provider: Provider::Codex,
        thread_id: ThreadId::parse("thread-journal").unwrap(),
        operation: ConversationSyncOperationKind::ProviderMaterialization,
        binding_forked_from: None,
        provider_thread_id: Some("thread-a".to_string()),
        materialization_state: ConversationMaterializationState::Pending,
        lineage_digest: Some("b".repeat(64)),
        semantic_digest: Some("c".repeat(64)),
        expected_host_identity_revision: 0,
        expected_bindings_document_revision: 0,
        expected_binding_revision: None,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: None,
        targets: vec![ConversationJournalTarget {
            target_path: "/tmp/provider/thread-a.jsonl".to_string(),
            role: crate::project_sync::domain::ConversationFileRole::Transcript,
            expected: TargetExpectation::Absent,
            staged_path: Some("/tmp/staging/thread-a.jsonl".to_string()),
            backup_path: None,
            staged_sha256: Some("d".repeat(64)),
            commit_started: false,
            committed: false,
        }],
        created_at: 1,
        updated_at: 1,
    };
    operation.validate().unwrap();
    world
        .repo_a
        .mutate_conversation_sync_journal(0, |journal| {
            journal.operations.push(operation.clone());
            Ok(())
        })
        .unwrap();
    let reopened = crate::project_sync::persistence::LocalStateRepository::from_home_dir(
        world.repo_a.root().parent().unwrap(),
    )
    .unwrap();
    let journal = reopened.load_conversation_sync_journal().unwrap();
    assert_eq!(journal.revision, 1);
    assert_eq!(journal.operations, vec![operation]);

    let stale = reopened.mutate_conversation_sync_journal(0, |_| Ok(()));
    assert!(stale.unwrap_err().contains("changed after review"));
}

#[test]
fn journal_rejects_duplicate_operation_ids() {
    let world = VariantWorld::new();
    let identity = world.repo_a.load_or_create_host_identity().unwrap();
    let base = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-a").unwrap(),
        plan_id: None,
        plan_action_id: None,
        direction: ConversationPlanDirection::Push,
        phase: ConversationSyncPhase::CloudPublished,
        cloud: Some(ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision: 2,
            commit_id: "commit-a".to_string(),
            manifest_sha256: "a".repeat(64),
        }),
        host_id: identity.host_id,
        replica_id: crate::project_sync::domain::ReplicaId::parse("replica-a").unwrap(),
        local_project_id: crate::project_sync::domain::LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: crate::project_sync::domain::LocalProviderProfileId::parse(
            "profile-a",
        )
        .unwrap(),
        provider: Provider::Codex,
        thread_id: ThreadId::parse("thread-journal").unwrap(),
        operation: ConversationSyncOperationKind::CloudPublication,
        binding_forked_from: None,
        provider_thread_id: None,
        materialization_state: ConversationMaterializationState::StoredOnly,
        lineage_digest: None,
        semantic_digest: None,
        expected_host_identity_revision: 0,
        expected_bindings_document_revision: 0,
        expected_binding_revision: None,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: None,
        targets: vec![],
        created_at: 1,
        updated_at: 1,
    };
    let result = world.repo_a.mutate_conversation_sync_journal(0, |journal| {
        journal.operations.push(base.clone());
        journal.operations.push(base.clone());
        Ok(())
    });
    assert!(result.unwrap_err().contains("duplicate"));
    assert!(world
        .repo_a
        .load_conversation_sync_journal()
        .unwrap()
        .operations
        .is_empty());
    let _ = Provider::Codex;
}

#[test]
fn every_local_commit_checkpoint_recovers_to_one_valid_fixed_point() {
    let checkpoints = vec![
        (ConversationFailpoint::BeforePrepared, false),
        (ConversationFailpoint::AfterPrepared, false),
        (
            ConversationFailpoint::BeforeStage {
                index: 0,
                role: ConversationFileRole::Transcript,
            },
            false,
        ),
        (
            ConversationFailpoint::AfterStage {
                index: 0,
                role: ConversationFileRole::Transcript,
            },
            false,
        ),
        (
            ConversationFailpoint::BeforeStage {
                index: 1,
                role: ConversationFileRole::ProviderIndexFragment,
            },
            false,
        ),
        (
            ConversationFailpoint::AfterStage {
                index: 1,
                role: ConversationFileRole::ProviderIndexFragment,
            },
            false,
        ),
        (
            ConversationFailpoint::BeforeStage {
                index: 2,
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: "thread-state".to_string(),
                },
            },
            false,
        ),
        (
            ConversationFailpoint::AfterStage {
                index: 2,
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: "thread-state".to_string(),
                },
            },
            true,
        ),
        (ConversationFailpoint::BeforeStaged, true),
        (ConversationFailpoint::AfterStaged, true),
        (
            ConversationFailpoint::BeforeTargetCommit {
                index: 0,
                role: ConversationFileRole::Transcript,
            },
            true,
        ),
        (
            ConversationFailpoint::AfterTargetWrite {
                index: 0,
                role: ConversationFileRole::Transcript,
            },
            true,
        ),
        (
            ConversationFailpoint::AfterTargetCommit {
                index: 0,
                role: ConversationFileRole::Transcript,
            },
            true,
        ),
        (
            ConversationFailpoint::AfterTargetWrite {
                index: 1,
                role: ConversationFileRole::ProviderIndexFragment,
            },
            true,
        ),
        (
            ConversationFailpoint::BeforeTargetCommit {
                index: 1,
                role: ConversationFileRole::ProviderIndexFragment,
            },
            true,
        ),
        (
            ConversationFailpoint::AfterTargetCommit {
                index: 1,
                role: ConversationFileRole::ProviderIndexFragment,
            },
            true,
        ),
        (
            ConversationFailpoint::BeforeTargetCommit {
                index: 2,
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: "thread-state".to_string(),
                },
            },
            true,
        ),
        (
            ConversationFailpoint::AfterTargetWrite {
                index: 2,
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: "thread-state".to_string(),
                },
            },
            true,
        ),
        (
            ConversationFailpoint::AfterTargetCommit {
                index: 2,
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: "thread-state".to_string(),
                },
            },
            true,
        ),
        (ConversationFailpoint::BeforeProviderCommitted, true),
        (ConversationFailpoint::AfterProviderCommitted, true),
        (ConversationFailpoint::BeforeBindingCommit, true),
        (ConversationFailpoint::AfterBindingCommit, true),
        (ConversationFailpoint::BeforeReceipt, true),
        (ConversationFailpoint::AfterReceipt, true),
        (ConversationFailpoint::BeforeLocalCommitted, true),
        (ConversationFailpoint::AfterLocalCommitted, true),
        (ConversationFailpoint::BeforeCleanup, true),
        (ConversationFailpoint::AfterCleanup, true),
    ];

    for (checkpoint, should_complete) in checkpoints {
        let fixture = MaterializationFixture::new();
        let mut fail_once = FailOnce::new(checkpoint.clone());
        let result = apply_conversation_materialization(
            &fixture.repository,
            fixture.request(),
            &mut fail_once,
        );
        assert!(result.is_err(), "checkpoint {checkpoint:?} did not fire");

        let reopened = LocalStateRepository::from_home_dir(&fixture.home).unwrap();
        let first = recover_conversation_materializations(&reopened).unwrap();
        let second = recover_conversation_materializations(&reopened).unwrap();
        assert!(second.completed.is_empty());
        assert!(second.rolled_back.is_empty());
        assert!(second.recovery_required.is_empty());
        assert!(reopened
            .load_conversation_sync_journal()
            .unwrap()
            .operations
            .is_empty());
        assert_materialization_fixed_point(&fixture, &reopened, should_complete);
        if should_complete {
            assert_eq!(
                first.completed.len(),
                usize::from(checkpoint != ConversationFailpoint::AfterCleanup)
            );
        } else if checkpoint != ConversationFailpoint::BeforePrepared {
            assert_eq!(first.rolled_back.len(), 1);
        }
    }
}

#[test]
fn every_cloud_publication_checkpoint_recovers_to_one_valid_fixed_point() {
    let checkpoints = [
        (ConversationFailpoint::BeforePrepared, false),
        (ConversationFailpoint::AfterPrepared, false),
        (ConversationFailpoint::BeforeCloudPublication, false),
        (ConversationFailpoint::AfterCloudHeadCas, true),
        (ConversationFailpoint::BeforeCloudPublished, true),
        (ConversationFailpoint::AfterCloudPublished, true),
        (ConversationFailpoint::BeforeBindingCommit, true),
        (ConversationFailpoint::AfterBindingCommit, true),
        (ConversationFailpoint::BeforeReceipt, true),
        (ConversationFailpoint::AfterReceipt, true),
        (ConversationFailpoint::BeforeLocalCommitted, true),
        (ConversationFailpoint::AfterLocalCommitted, true),
        (ConversationFailpoint::BeforeCleanup, true),
        (ConversationFailpoint::AfterCleanup, true),
    ];

    for (checkpoint, publication_committed) in checkpoints {
        let world = VariantWorld::new();
        let origin = world.origin(&world.repo_a, "replica-cloud-checkpoint");
        let thread_id = ThreadId::parse("thread-cloud-checkpoint").unwrap();
        let initial_transcript = super::harness::chain("source-thread", &[("m-a", "base")]);
        let initial_thread = super::harness::ThreadSpec::new(
            thread_id.as_str(),
            "source-thread",
            initial_transcript,
            origin.clone(),
            10,
        );
        let initial = world.publish(&[initial_thread.clone()], origin.clone(), 10);
        let scope = install_scope(
            &world.repo_a,
            &world.bundle_id,
            "cloud-checkpoint",
            "replica-cloud-checkpoint",
        );
        fs::create_dir(scope.provider_root.join("sessions")).unwrap();
        let advanced_transcript =
            super::harness::chain("source-thread", &[("m-a", "base"), ("m-b", "continued")]);
        fs::write(
            scope.provider_root.join("sessions/source-thread.jsonl"),
            &advanced_transcript,
        )
        .unwrap();
        let analysis = analyze_codex_transcript(&advanced_transcript);
        let (lineage, semantic) = match analysis.lineage {
            CodecLineage::Complete {
                lineage_digest,
                semantic_digest,
                ..
            } => (lineage_digest, semantic_digest),
            other => panic!("checkpoint transcript is not resumable: {other:?}"),
        };
        // The binding starts pinned at the revision-1 content; a committed
        // publication must advance the pin to the pushed digests, because
        // manifest-only refresh reads the pin as "nothing local is unsaved".
        let initial_analysis = analyze_codex_transcript(&initial_thread.transcript);
        let (initial_lineage, initial_semantic) = match initial_analysis.lineage {
            CodecLineage::Complete {
                lineage_digest,
                semantic_digest,
                ..
            } => (lineage_digest, semantic_digest),
            other => panic!("initial checkpoint transcript is not resumable: {other:?}"),
        };
        world
            .repo_a
            .mutate_conversation_bindings(|document| {
                document.bindings.push(LocalConversationBinding {
                    host_id: scope.host_id.clone(),
                    replica_id: scope.replica_id.clone(),
                    local_project_id: scope.project_id.clone(),
                    provider_profile_id: scope.profile_id.clone(),
                    thread_id: thread_id.clone(),
                    forked_from: None,
                    fork_stamp: None,
                    provider_thread_id: Some("source-thread".to_string()),
                    materialization_state: ConversationMaterializationState::Resumable,
                    last_observed_file_sha256: Some(sha256(&initial_thread.transcript)),
                    last_observed_head_message_id: Some("m-a".to_string()),
                    last_observed_lineage_digest: Some(initial_lineage.clone()),
                    last_observed_semantic_digest: Some(initial_semantic.clone()),
                    source_revision: Some(initial.snapshot.head.revision),
                    source_manifest_sha256: Some(initial.snapshot.head.manifest_sha256.clone()),
                    revision: 0,
                    created_at: 10,
                    updated_at: 10,
                });
                Ok(())
            })
            .unwrap();
        let bindings = world.repo_a.load_conversation_bindings().unwrap();
        let mut advanced_thread = initial_thread;
        advanced_thread.transcript = advanced_transcript.clone();
        advanced_thread.updated_at = 20;
        advanced_thread.last_updated_by = Some(origin.clone());
        let request = super::harness::publish_request(
            world.bundle_id.clone(),
            &[advanced_thread],
            origin,
            20,
            Some(&initial.snapshot.manifest),
            crate::project_sync::bundle_engine::PublishExpectation::Match(initial.head_token),
        );
        let operation = push_operation(
            &world,
            &scope,
            "operation-cloud-checkpoint",
            "plan-cloud-checkpoint",
            initial.snapshot.head.revision,
            initial.snapshot.head.commit_id,
            initial.snapshot.head.manifest_sha256,
            &thread_id,
            bindings.revision,
            Some(bindings.bindings[0].revision),
            &advanced_transcript,
            &lineage,
            &semantic,
        );
        let engine = world.engine();
        let mut fail_once = FailOnce::new(checkpoint.clone());
        let result = apply_conversation_cloud_publication(
            &world.repo_a,
            operation,
            || engine.publish(request).map(cloud_publication),
            || Ok(None),
            &mut fail_once,
        );
        assert!(result.is_err(), "checkpoint {checkpoint:?} did not fire");

        let reopened =
            LocalStateRepository::from_home_dir(world.repo_a.root().parent().unwrap()).unwrap();
        let publication = if world
            .engine()
            .inspect(&world.bundle_id)
            .unwrap()
            .head
            .revision
            == 2
        {
            let snapshot = world.engine().inspect(&world.bundle_id).unwrap();
            Some(ConversationCloudPublication {
                revision: snapshot.head.revision,
                commit_id: snapshot.head.commit_id,
                manifest_sha256: snapshot.head.manifest_sha256,
            })
        } else {
            None
        };
        let first = recover_conversation_cloud_publications(&reopened, |_| Ok(publication.clone()))
            .unwrap();
        let second = recover_conversation_cloud_publications(&reopened, |_| Ok(None)).unwrap();
        assert!(second.completed.is_empty());
        assert!(second.rolled_back.is_empty());
        assert!(second.recovery_required.is_empty());
        assert!(reopened
            .load_conversation_sync_journal()
            .unwrap()
            .operations
            .is_empty());

        let final_snapshot = world.engine().inspect(&world.bundle_id).unwrap();
        let final_bindings = reopened.load_conversation_bindings().unwrap();
        let final_receipts = reopened.load_materializations().unwrap();
        if publication_committed {
            assert_eq!(final_snapshot.head.revision, 2, "{checkpoint:?}");
            assert_eq!(final_bindings.bindings.len(), 1);
            assert_eq!(
                final_bindings.bindings[0].source_revision,
                Some(2),
                "{checkpoint:?}",
            );
            // The push receipt advanced the last-synced pin to the pushed
            // digests; manifest-only refresh depends on this to classify
            // later cloud growth as a fast-forward instead of divergence.
            assert_eq!(
                final_bindings.bindings[0]
                    .last_observed_lineage_digest
                    .as_deref(),
                Some(lineage.as_str()),
                "{checkpoint:?}",
            );
            assert_eq!(
                final_bindings.bindings[0]
                    .last_observed_semantic_digest
                    .as_deref(),
                Some(semantic.as_str()),
                "{checkpoint:?}",
            );
            assert_eq!(final_receipts.records.len(), 1, "{checkpoint:?}");
            assert_eq!(final_receipts.records[0].receipts.len(), 1);
            assert_eq!(
                first.completed.len(),
                usize::from(checkpoint != ConversationFailpoint::AfterCleanup),
            );
        } else {
            assert_eq!(final_snapshot.head.revision, 1, "{checkpoint:?}");
            assert_eq!(final_bindings.bindings[0].source_revision, Some(1));
            // An uncommitted publication leaves the pin untouched.
            assert_eq!(
                final_bindings.bindings[0]
                    .last_observed_lineage_digest
                    .as_deref(),
                Some(initial_lineage.as_str()),
                "{checkpoint:?}",
            );
            assert!(final_receipts.records.is_empty());
            if checkpoint != ConversationFailpoint::BeforePrepared {
                assert_eq!(first.rolled_back.len(), 1, "{checkpoint:?}");
            }
        }
    }
}

#[test]
fn committed_codex_provider_files_materialize_resume_and_rebind_one_variant() {
    const TRANSCRIPT: &[u8] =
        include_bytes!("../../../testdata/thread_variants/codex/v1/valid-continuation.jsonl");
    const INDEX: &[u8] =
        include_bytes!("../../../testdata/thread_variants/codex/v1/session-index-fragment.json");
    const SIDECAR: &[u8] =
        include_bytes!("../../../testdata/thread_variants/codex/v1/thread-sidecar.json");

    let mut fixture = MaterializationFixture::new();
    let machine = fixture.repository.load_bindings().unwrap();
    let project = machine
        .bindings
        .iter()
        .find(|binding| binding.local_project_id == fixture.operation.local_project_id)
        .unwrap();
    let project_root = project.project_root.clone();
    let profile = machine
        .profiles
        .iter()
        .find(|profile| profile.profile_id == fixture.operation.provider_profile_id)
        .unwrap();
    let provider_root = PathBuf::from(&profile.canonical_path);
    let rewritten = rewrite_codex_transcript(
        TRANSCRIPT,
        "thread-fixture",
        "destination-thread",
        "/fixture/project",
        &project_root,
    )
    .unwrap();
    let source = analyze_codex_transcript(TRANSCRIPT);
    let rewritten_analysis = analyze_codex_transcript(&rewritten);
    let (lineage_digest, semantic_digest) = match &rewritten_analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest.clone(), semantic_digest.clone()),
        other => panic!("rewritten committed fixture is not resumable: {other:?}"),
    };
    assert_eq!(
        source.lineage.chain().unwrap(),
        rewritten_analysis.lineage.chain().unwrap()
    );
    fixture.operation.lineage_digest = Some(lineage_digest);
    fixture.operation.semantic_digest = Some(semantic_digest);
    fixture.targets[0].bytes = rewritten.clone();

    let mut index: serde_json::Value = serde_json::from_slice(INDEX).unwrap();
    index["id"] = serde_json::Value::String("destination-thread".to_string());
    fixture.targets[1].bytes = serde_json::to_vec(&index).unwrap();
    fixture.targets[1].bytes.push(b'\n');
    let mut sidecar: serde_json::Value = serde_json::from_slice(SIDECAR).unwrap();
    sidecar["thread_id"] = serde_json::Value::String("destination-thread".to_string());
    sidecar["cwd"] = serde_json::Value::String(project_root.clone());
    fixture.targets[2].bytes = serde_json::to_vec(&sidecar).unwrap();
    fixture.targets[2].bytes.push(b'\n');

    apply_conversation_materialization(
        &fixture.repository,
        fixture.request(),
        &mut NoConversationFailpoints,
    )
    .unwrap();
    assert_materialization_fixed_point(&fixture, &fixture.repository, true);

    let committed = fs::read(&fixture.targets[0].target_path).unwrap();
    let committed_analysis = analyze_codex_transcript(&committed);
    assert_eq!(
        committed_analysis.source_thread_id.as_deref(),
        Some("destination-thread")
    );
    assert_eq!(
        committed_analysis.source_cwd.as_deref(),
        Some(project_root.as_str())
    );
    assert_eq!(
        rewritten_analysis.lineage.chain(),
        committed_analysis.lineage.chain()
    );
    let committed_index: serde_json::Value =
        serde_json::from_slice(&fixture.targets[1].bytes).unwrap();
    let committed_sidecar: serde_json::Value =
        serde_json::from_slice(&fixture.targets[2].bytes).unwrap();
    assert_eq!(committed_index["id"], "destination-thread");
    assert_eq!(committed_index["thread_name"], "Fixture continuation");
    assert_eq!(committed_index["updated_at"], "2026-07-22T00:00:06.000000Z");
    assert!(
        committed_index.get("cwd").is_none(),
        "index rows use the provider schema, not Mallard-invented fields"
    );
    assert_eq!(committed_sidecar["thread_id"], "destination-thread");
    assert_eq!(committed_sidecar["cwd"], project_root);

    let mut capture_request = CaptureRequest::for_project(PathBuf::from(&project_root));
    capture_request.codex_home = Some(provider_root);
    let inventory = discover_project(&capture_request).unwrap();
    let conversation = inventory
        .resources
        .iter()
        .find(|resource| resource.resource_id == "codex:session:destination-thread")
        .unwrap();
    assert!(conversation.blocked_reason.is_none());
    let captured = capture_selected(
        &capture_request,
        &BTreeSet::from([conversation.resource_id.clone()]),
    )
    .unwrap();
    let captured_conversation = captured
        .conversations
        .get("codex:session:destination-thread")
        .unwrap();
    assert!(captured_conversation.lineage.is_complete());

    let mut continued = committed;
    continued.extend_from_slice(
        serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-07-22T00:00:07Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-006",
                "parent_id": "msg-005",
                "role": "user",
                "content": [{"type": "input_text", "text": "Continue after restore."}]
            }
        }))
        .unwrap()
        .as_bytes(),
    );
    continued.push(b'\n');
    fs::write(&fixture.targets[0].target_path, &continued).unwrap();
    let recaptured = capture_selected(
        &capture_request,
        &BTreeSet::from(["codex:session:destination-thread".to_string()]),
    )
    .unwrap();
    let recaptured_conversation = recaptured
        .conversations
        .get("codex:session:destination-thread")
        .unwrap();
    assert_eq!(
        crate::project_sync::domain::classify_conversation_relation(
            recaptured_conversation.lineage.chain(),
            captured_conversation.lineage.chain(),
        ),
        ConversationRelation::CloudPrefixOfLocal
    );
    let bindings = fixture.repository.load_conversation_bindings().unwrap();
    assert_eq!(bindings.bindings.len(), 1);
    assert_eq!(
        bindings.bindings[0].thread_id,
        ThreadId::parse("thread-materialize").unwrap()
    );
    assert_eq!(
        bindings.bindings[0].provider_thread_id.as_deref(),
        Some("destination-thread")
    );
}

#[test]
fn replacement_is_digest_pinned_backed_up_and_restart_idempotent() {
    let mut fixture = MaterializationFixture::new();
    let old = b"old-index-row\n".to_vec();
    fs::write(&fixture.targets[1].target_path, &old).unwrap();
    fixture.targets[1].expected = TargetExpectation::Sha256(sha256(&old));
    let mut fail_once = FailOnce::new(ConversationFailpoint::AfterTargetWrite {
        index: 1,
        role: ConversationFileRole::ProviderIndexFragment,
    });
    assert!(apply_conversation_materialization(
        &fixture.repository,
        fixture.request(),
        &mut fail_once,
    )
    .is_err());
    let reopened = LocalStateRepository::from_home_dir(&fixture.home).unwrap();
    recover_conversation_materializations(&reopened).unwrap();
    recover_conversation_materializations(&reopened).unwrap();
    assert_eq!(
        fs::read(&fixture.targets[1].target_path).unwrap(),
        fixture.targets[1].bytes
    );
    let backup = reopened
        .root()
        .join("backups/conversations/operation-materialize/1.bak");
    assert_eq!(fs::read(backup).unwrap(), old);
    assert_materialization_fixed_point(&fixture, &reopened, true);
}

#[test]
fn stale_pending_fork_becomes_unacknowledged_without_adopting_an_exact_child() {
    let fixture = CodexDivergenceFixture::new();
    fixture.begin_pending_checkpoint(CodexForkExecutor::AppServerStdio);
    fixture.write_exact_child();

    let recovered = recover_conversation_materializations(&fixture.repository).unwrap();
    assert_eq!(
        recovered.recovery_required,
        vec![fixture.operation.operation_id.clone()]
    );
    assert!(recovered.completed.is_empty());
    let operation = fixture.current_operation();
    assert_eq!(operation.phase, ConversationSyncPhase::ForkUnacknowledged);
    let divergence = operation.divergence().unwrap();
    assert_eq!(
        divergence.attempts,
        vec![CodexForkAttempt {
            executor: CodexForkExecutor::AppServerStdio,
            status: CodexForkAttemptStatus::Unacknowledged,
        }]
    );
    assert_eq!(
        divergence.current_executor,
        Some(CodexForkExecutor::AppServerStdio)
    );
    assert!(divergence.successful_executor.is_none());
    assert!(divergence.fork_provider_thread_id.is_none());
    assert!(divergence.fork_transcript_path.is_none());
    assert!(divergence.fork_transcript_sha256.is_none());
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
    assert_eq!(fixture.native_child_count(), 1);
    assert_eq!(
        fixture
            .repository
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .len(),
        1,
        "recovery must not adopt even an exact child from the abandoned attempt"
    );
}

#[test]
fn unacknowledged_codex_checkpoint_is_rehydrated_after_reopening() {
    let fixture = CodexDivergenceFixture::new();
    fixture.begin_unacknowledged_checkpoint(CodexForkExecutor::AppServerStdio);
    let reopened = LocalStateRepository::from_home_dir(&fixture.home).unwrap();

    let pending =
        pending_codex_divergence_with_repository(&reopened, &fixture.operation.local_project_id)
            .unwrap()
            .unwrap();
    assert!(!pending.success);
    assert!(matches!(
        pending.status,
        crate::project_sync::commands::RestoreApplyStatus::ForkUnacknowledged
    ));
    assert_eq!(pending.plan_id, fixture.operation.plan_id.clone());
    assert_eq!(pending.codex_fork_checkpoints.len(), 1);
    let checkpoint = &pending.codex_fork_checkpoints[0];
    assert_eq!(checkpoint.operation_id, fixture.operation.operation_id);
    assert_eq!(checkpoint.plan_id, fixture.operation.plan_id.clone());
    assert_eq!(checkpoint.thread_id, fixture.operation.thread_id);
    assert!(matches!(
        checkpoint.status,
        crate::project_sync::commands::RestoreApplyStatus::ForkUnacknowledged
    ));
    assert_eq!(
        checkpoint.source_provider_thread_id,
        fixture.source_provider_id
    );
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
}

#[test]
fn multiple_pending_codex_checkpoints_surface_as_an_ordered_queue() {
    let fixture = CodexDivergenceFixture::new();
    fixture.begin_unacknowledged_checkpoint(CodexForkExecutor::AppServerStdio);
    // A second thread's paused divergence coexists in the same project
    // journal; the pending listing returns both, oldest first, instead of
    // the retired multiple-operations error. The journaled first operation
    // is the template so the clone carries a valid unacknowledged state.
    let journal = fixture.repository.load_conversation_sync_journal().unwrap();
    let mut second = journal
        .operations
        .iter()
        .find(|operation| operation.operation_id == fixture.operation.operation_id)
        .cloned()
        .unwrap();
    second.operation_id =
        ConversationOperationId::parse("operation-codex-divergence-second").unwrap();
    second.thread_id = ThreadId::parse("thread-divergent-second").unwrap();
    second.created_at = fixture.operation.created_at + 5;
    second.updated_at = second.updated_at.max(second.created_at);
    fixture
        .repository
        .mutate_conversation_sync_journal(journal.revision, |journal| {
            journal.operations.push(second.clone());
            Ok(())
        })
        .unwrap();

    let pending = pending_codex_divergence_with_repository(
        &fixture.repository,
        &fixture.operation.local_project_id,
    )
    .unwrap()
    .unwrap();
    assert!(!pending.success);
    assert_eq!(pending.codex_fork_checkpoints.len(), 2);
    assert_eq!(
        pending.codex_fork_checkpoints[0].operation_id,
        fixture.operation.operation_id
    );
    assert_eq!(
        pending.codex_fork_checkpoints[1].operation_id,
        second.operation_id
    );
    assert_eq!(
        pending.codex_fork_checkpoints[1].thread_id,
        second.thread_id
    );
    for checkpoint in &pending.codex_fork_checkpoints {
        assert!(matches!(
            checkpoint.status,
            crate::project_sync::commands::RestoreApplyStatus::ForkUnacknowledged
        ));
    }
}

#[test]
fn a_thread_with_a_pending_checkpoint_rejects_a_second_operation() {
    let fixture = CodexDivergenceFixture::new();
    fixture.begin_unacknowledged_checkpoint(CodexForkExecutor::AppServerStdio);
    let mut duplicate = fixture.operation.clone();
    duplicate.operation_id =
        ConversationOperationId::parse("operation-codex-divergence-retry").unwrap();
    let mut failpoints = NoConversationFailpoints;
    let error = apply_codex_divergence_locked(
        &fixture.repository,
        CodexDivergenceMaterializationRequest {
            operation: duplicate,
            targets: vec![fixture.target.clone()],
            fork: fixture.fork.clone(),
        },
        &mut failpoints,
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(
        error.contains("already has a pending sync operation"),
        "{error}"
    );
    // The original checkpoint is untouched and still explicitly retryable.
    let pending = pending_codex_divergence_with_repository(
        &fixture.repository,
        &fixture.operation.local_project_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pending.codex_fork_checkpoints.len(), 1);
    assert_eq!(
        pending.codex_fork_checkpoints[0].operation_id,
        fixture.operation.operation_id
    );
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
}

#[cfg(unix)]
#[test]
fn cloud_only_codex_divergence_replaces_original_without_creating_a_fork() {
    let mut fixture = CodexDivergenceFixture::new();
    fixture.enable_fake_idle_app_server();
    let divergence = match &mut fixture.operation.operation {
        ConversationSyncOperationKind::CodexDivergence(divergence) => divergence,
        _ => unreachable!(),
    };
    divergence.resolution = ConversationDivergenceResolution::UseCloudOnly;
    fixture.operation.validate().unwrap();

    let mut failpoints = NoConversationFailpoints;
    let outcome = apply_codex_divergence_locked(
        &fixture.repository,
        CodexDivergenceMaterializationRequest {
            operation: fixture.operation.clone(),
            targets: vec![fixture.target.clone()],
            fork: fixture.fork.clone(),
        },
        &mut failpoints,
        &mut |_, _| {},
    )
    .unwrap();
    let CodexDivergenceApplyOutcome::Committed(receipt) = outcome else {
        panic!("cloud-only divergence did not commit");
    };

    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.cloud_bytes);
    assert_eq!(fixture.native_child_count(), 0);
    let bindings = fixture.repository.load_conversation_bindings().unwrap();
    assert_eq!(bindings.bindings.len(), 1);
    assert_eq!(bindings.bindings[0].thread_id, fixture.source_thread_id);
    assert_eq!(
        bindings.bindings[0].provider_thread_id.as_deref(),
        Some(fixture.source_provider_id.as_str())
    );
    let divergence = receipt.divergence.unwrap();
    assert_eq!(
        divergence.resolution,
        ConversationDivergenceResolution::UseCloudOnly
    );
    assert!(divergence.preserved_thread_id.is_none());
    assert!(divergence.preserved_provider_thread_id.is_none());
    assert!(divergence.executor.is_none());
    assert_eq!(
        divergence.original_name_status,
        Some(ConversationNameStatus::Unchanged)
    );
    assert_eq!(
        divergence.inventory_refresh_status,
        CodexInventoryRefreshStatus::Deferred
    );
    let backup = fixture
        .repository
        .root()
        .join("backups/conversations/operation-codex-divergence/0.bak");
    assert_eq!(fs::read(backup).unwrap(), fixture.local_bytes);
}

#[test]
fn cloud_only_status_failure_rolls_back_staging_without_replacing_original() {
    let mut fixture = CodexDivergenceFixture::new();
    let divergence = match &mut fixture.operation.operation {
        ConversationSyncOperationKind::CodexDivergence(divergence) => divergence,
        _ => unreachable!(),
    };
    divergence.resolution = ConversationDivergenceResolution::UseCloudOnly;
    fixture.operation.validate().unwrap();

    let mut failpoints = NoConversationFailpoints;
    let error = apply_codex_divergence_locked(
        &fixture.repository,
        CodexDivergenceMaterializationRequest {
            operation: fixture.operation.clone(),
            targets: vec![fixture.target.clone()],
            fork: fixture.fork.clone(),
        },
        &mut failpoints,
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(error.contains("status is unavailable"), "{error}");
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
    assert!(fixture
        .repository
        .load_conversation_sync_journal()
        .unwrap()
        .operations
        .is_empty());
    assert_eq!(
        fixture
            .repository
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .len(),
        1
    );
    assert!(fixture
        .repository
        .load_materializations()
        .unwrap()
        .records
        .is_empty());
}

#[test]
fn cloud_only_staging_crash_rolls_back_when_provider_status_cannot_be_rechecked() {
    let mut fixture = CodexDivergenceFixture::new();
    let divergence = match &mut fixture.operation.operation {
        ConversationSyncOperationKind::CodexDivergence(divergence) => divergence,
        _ => unreachable!(),
    };
    divergence.resolution = ConversationDivergenceResolution::UseCloudOnly;
    fixture.operation.validate().unwrap();
    let mut fail_once = FailOnce::new(ConversationFailpoint::AfterStaged);
    assert!(apply_codex_divergence_locked(
        &fixture.repository,
        CodexDivergenceMaterializationRequest {
            operation: fixture.operation.clone(),
            targets: vec![fixture.target.clone()],
            fork: fixture.fork.clone(),
        },
        &mut fail_once,
        &mut |_, _| {},
    )
    .is_err());

    let recovered = recover_conversation_materializations(&fixture.repository).unwrap();
    assert_eq!(
        recovered.rolled_back,
        vec![fixture.operation.operation_id.clone()]
    );
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
    assert!(fixture
        .repository
        .load_conversation_sync_journal()
        .unwrap()
        .operations
        .is_empty());
}

#[cfg(unix)]
#[test]
fn preserved_fork_staging_crash_rolls_back_without_starting_a_provider() {
    use std::os::unix::fs::PermissionsExt;

    let mut fixture = CodexDivergenceFixture::new();
    let sentinel = fixture.home.join("provider-must-not-start");
    let script = format!(
        "#!/bin/sh\n: > '{}'\nexit 1\n",
        sentinel.to_string_lossy().replace('\'', "'\\''")
    );
    fs::write(&fixture.fork.codex_executable, script.as_bytes()).unwrap();
    fs::set_permissions(
        &fixture.fork.codex_executable,
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let executable_sha256 = sha256(script.as_bytes());
    fixture.fork.codex_executable_sha256 = executable_sha256.clone();
    match &mut fixture.operation.operation {
        ConversationSyncOperationKind::CodexDivergence(divergence) => {
            divergence.codex_executable_sha256 = executable_sha256
        }
        _ => unreachable!(),
    }

    let mut fail_once = FailOnce::new(ConversationFailpoint::AfterStaged);
    let error = apply_codex_divergence_locked(
        &fixture.repository,
        CodexDivergenceMaterializationRequest {
            operation: fixture.operation.clone(),
            targets: vec![fixture.target.clone()],
            fork: fixture.fork.clone(),
        },
        &mut fail_once,
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(error.contains("injected failure"), "{error}");
    assert!(!sentinel.exists());

    let recovered = recover_conversation_materializations(&fixture.repository).unwrap();
    assert_eq!(
        recovered.rolled_back,
        vec![fixture.operation.operation_id.clone()]
    );
    assert!(recovered.completed.is_empty());
    assert!(recovered.recovery_required.is_empty());
    assert!(!sentinel.exists());
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
    assert!(fixture
        .repository
        .load_conversation_sync_journal()
        .unwrap()
        .operations
        .is_empty());
}

#[cfg(unix)]
#[test]
fn every_post_verification_codex_checkpoint_recovers_without_a_second_fork() {
    let checkpoints = [
        ConversationFailpoint::AfterForkVerified,
        ConversationFailpoint::AfterForkNamingRecorded,
        ConversationFailpoint::AfterLocalForkBound,
        ConversationFailpoint::AfterProviderCommitted,
        ConversationFailpoint::AfterBindingCommit,
        ConversationFailpoint::AfterBindingsCommitted,
        ConversationFailpoint::AfterOriginalNamingRecorded,
        ConversationFailpoint::AfterReceipt,
        ConversationFailpoint::AfterLocalCommitted,
    ];
    for checkpoint in checkpoints {
        let mut fixture = CodexDivergenceFixture::new();
        fixture.write_exact_child();
        fixture.enable_fake_fork_app_server();
        let mut fail_once = FailOnce::new(checkpoint.clone());
        let error = apply_codex_divergence_locked(
            &fixture.repository,
            CodexDivergenceMaterializationRequest {
                operation: fixture.operation.clone(),
                targets: vec![fixture.target.clone()],
                fork: fixture.fork.clone(),
            },
            &mut fail_once,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(
            error.contains("injected failure"),
            "{checkpoint:?}: {error}"
        );
        assert_eq!(
            fixture.native_child_count(),
            1,
            "{checkpoint:?} created an unexpected fork before recovery"
        );

        let reopened = LocalStateRepository::from_home_dir(&fixture.home).unwrap();
        let recovered = recover_conversation_materializations(&reopened).unwrap();
        assert_eq!(
            recovered.completed,
            vec![fixture.operation.operation_id.clone()],
            "{checkpoint:?}"
        );
        assert!(recovered.recovery_required.is_empty(), "{checkpoint:?}");
        let repeated = recover_conversation_materializations(&reopened).unwrap();
        assert!(repeated.completed.is_empty(), "{checkpoint:?}");
        let receipt = fixture.persisted_receipt(&reopened);
        fixture.assert_complete_in(&reopened, &receipt, CodexForkExecutor::AppServerStdio);
        assert_eq!(
            fixture.native_child_count(),
            1,
            "{checkpoint:?} created a second native fork"
        );
    }
}

#[cfg(unix)]
#[test]
fn changed_verified_fork_blocks_original_replacement_during_recovery() {
    let mut fixture = CodexDivergenceFixture::new();
    fixture.write_exact_child();
    fixture.enable_fake_fork_app_server();
    let mut fail_once = FailOnce::new(ConversationFailpoint::AfterForkVerified);
    assert!(apply_codex_divergence_locked(
        &fixture.repository,
        CodexDivergenceMaterializationRequest {
            operation: fixture.operation.clone(),
            targets: vec![fixture.target.clone()],
            fork: fixture.fork.clone(),
        },
        &mut fail_once,
        &mut |_, _| {},
    )
    .is_err());
    let mut changed_child = fixture.child_bytes.clone();
    changed_child.push(b'\n');
    fs::write(&fixture.child_path, changed_child).unwrap();

    let recovered = recover_conversation_materializations(&fixture.repository).unwrap();
    assert_eq!(
        recovered.recovery_required,
        vec![fixture.operation.operation_id.clone()]
    );
    assert!(recovered.completed.is_empty());
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
    assert_eq!(
        fixture
            .repository
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .len(),
        1
    );
    assert!(fixture
        .repository
        .load_materializations()
        .unwrap()
        .records
        .is_empty());
    assert_eq!(
        fixture.current_operation().phase,
        ConversationSyncPhase::RecoveryRequired
    );
    let pending = pending_codex_divergence_with_repository(
        &fixture.repository,
        &fixture.operation.local_project_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        pending.codex_fork_checkpoints[0].status,
        crate::project_sync::commands::RestoreApplyStatus::RecoveryRequired
    );
}

#[cfg(unix)]
#[test]
fn unacknowledged_recovery_is_inert_until_an_explicit_retry() {
    use std::os::unix::fs::PermissionsExt;

    let mut fixture = CodexDivergenceFixture::new();
    fixture.begin_unacknowledged_checkpoint(CodexForkExecutor::AppServerStdio);
    let sentinel = fixture.home.join("provider-must-not-start");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 1\n",
        sentinel.to_string_lossy().replace('\'', "'\\''")
    );
    fs::write(&fixture.fork.codex_executable, script.as_bytes()).unwrap();
    fs::set_permissions(
        &fixture.fork.codex_executable,
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let executable_sha256 = sha256(script.as_bytes());
    fixture.fork.codex_executable_sha256 = executable_sha256.clone();

    let journal = fixture.repository.load_conversation_sync_journal().unwrap();
    fixture
        .repository
        .mutate_conversation_sync_journal(journal.revision, |journal| {
            let operation = journal
                .operations
                .iter_mut()
                .find(|operation| operation.operation_id == fixture.operation.operation_id)
                .unwrap();
            let divergence = match &mut operation.operation {
                ConversationSyncOperationKind::CodexDivergence(divergence) => divergence,
                _ => unreachable!(),
            };
            divergence.codex_executable_sha256 = executable_sha256;
            Ok(())
        })
        .unwrap();

    let before = fixture.current_operation();
    let recovered = recover_conversation_materializations(&fixture.repository).unwrap();
    assert_eq!(
        recovered.recovery_required,
        vec![fixture.operation.operation_id.clone()]
    );
    assert!(recovered.completed.is_empty());
    assert_eq!(fixture.current_operation(), before);
    assert!(!sentinel.exists());
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);

    let outcome = retry_codex_divergence_locked(
        &fixture.repository,
        &fixture.operation.operation_id,
        fixture.fork.clone(),
        &mut NoConversationFailpoints,
        &mut |_, _| {},
    )
    .unwrap();
    assert!(matches!(
        outcome,
        CodexDivergenceApplyOutcome::ForkUnacknowledged { .. }
    ));
    assert!(
        sentinel.exists(),
        "only the explicit retry may invoke the provider again"
    );
    let invocations = fs::read_to_string(&sentinel).unwrap();
    assert!(invocations.lines().any(|line| line.contains("app-server")));
    assert!(
        invocations
            .lines()
            .any(|line| line.contains("login status")),
        "a definitive app-server failure during explicit retry may fall through to native PTY"
    );
    assert_eq!(
        fixture.current_operation().phase,
        ConversationSyncPhase::ForkUnacknowledged
    );
    assert_eq!(fs::read(&fixture.source_path).unwrap(), fixture.local_bytes);
    assert_eq!(fixture.native_child_count(), 0);
}

fn codex_divergence_rollout(
    provider_thread_id: &str,
    forked_from_id: Option<&str>,
    cwd: &PathBuf,
    branch_message_id: &str,
    branch_text: &str,
) -> Vec<u8> {
    let records = [
        json!({
            "timestamp": "2026-07-29T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": provider_thread_id,
                "forked_from_id": forked_from_id,
                "cwd": cwd,
            },
        }),
        json!({
            "timestamp": "2026-07-29T00:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "m1",
                "parent_id": null,
                "role": "user",
                "content": [{"type": "input_text", "text": "question"}],
            },
        }),
        json!({
            "timestamp": "2026-07-29T00:00:02Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "m2",
                "parent_id": "m1",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "answer"}],
            },
        }),
        json!({
            "timestamp": "2026-07-29T00:00:03Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": branch_message_id,
                "parent_id": "m2",
                "role": "user",
                "content": [{"type": "input_text", "text": branch_text}],
            },
        }),
        // Real rollouts carry the turn's environment, and Codex re-emits it
        // under its current schema when forking.  Without one here the fixture
        // cannot show that verification tolerates that rewrite.
        json!({
            "timestamp": "2026-07-29T00:00:04Z",
            "type": "turn_context",
            "payload": {
                "cwd": cwd,
                "approval_policy": "on-request",
                "file_system_sandbox_policy": {
                    "entries": [{
                        "path": {"type": "path", "path": cwd},
                        "access": "write",
                        "missing_path_behavior": "skip",
                    }],
                },
            },
        }),
        json!({
            "timestamp": "2026-07-29T00:00:05Z",
            "type": "event_msg",
            "payload": {
                "type": "thread_settings_applied",
                "thread_settings": {
                    "cwd": cwd,
                    "approval_policy": "on-request",
                    "permission_profile": {
                        "type": "managed",
                        "file_system": {
                            "type": "restricted",
                            "entries": [{
                                "path": cwd,
                                "access": "write",
                                "missing_path_behavior": "skip",
                            }],
                        },
                    },
                },
            },
        }),
    ];
    let mut bytes = records
        .into_iter()
        .map(|record| record.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    bytes.push(b'\n');
    bytes
}

/// Derive a fork from a source rollout the way Codex actually does: new
/// identity and parent pointer, every record restamped at fork time, and the
/// sandbox policy re-emitted without its default fields.  Building the fork
/// from the source — rather than fabricating it from the same literals —
/// is what keeps this fixture honest about the shape verification must accept.
fn codex_fork_of(source: &[u8], fork_provider_id: &str, source_provider_id: &str) -> Vec<u8> {
    let mut lines = Vec::new();
    for raw in source.split(|byte| *byte == b'\n') {
        if raw.is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_slice(raw).unwrap();
        value["timestamp"] = json!("2026-07-29T12:00:00Z");
        match value.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                value["payload"]["id"] = json!(fork_provider_id);
                value["payload"]["forked_from_id"] = json!(source_provider_id);
            }
            Some("turn_context") => {
                let entries = value
                    .pointer_mut("/payload/file_system_sandbox_policy/entries")
                    .and_then(Value::as_array_mut)
                    .expect("fixture turn_context carries sandbox entries");
                for entry in entries {
                    entry
                        .as_object_mut()
                        .expect("sandbox entries are objects")
                        .remove("missing_path_behavior");
                }
            }
            Some("event_msg")
                if value.pointer("/payload/type").and_then(Value::as_str)
                    == Some("thread_settings_applied") =>
            {
                let entries = value
                    .pointer_mut("/payload/thread_settings/permission_profile/file_system/entries")
                    .and_then(Value::as_array_mut)
                    .expect("fixture thread settings carry sandbox entries");
                for entry in entries {
                    entry
                        .as_object_mut()
                        .expect("sandbox entries are objects")
                        .remove("missing_path_behavior");
                }
            }
            _ => {}
        }
        lines.push(value.to_string());
    }
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    bytes
}

fn complete_digests(bytes: &[u8]) -> (String, String) {
    match analyze_codex_transcript(bytes).lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        other => panic!("Codex divergence fixture is not resumable: {other:?}"),
    }
}

struct CodexDivergenceFixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    repository: LocalStateRepository,
    operation: ConversationSyncOperation,
    target: ConversationTargetPayload,
    fork: CodexForkExecutionRequest,
    source_path: PathBuf,
    child_path: PathBuf,
    source_thread_id: ThreadId,
    preserved_thread_id: ThreadId,
    source_provider_id: String,
    child_provider_id: String,
    local_bytes: Vec<u8>,
    child_bytes: Vec<u8>,
    cloud_bytes: Vec<u8>,
    local_lineage_digest: String,
    local_semantic_digest: String,
    cloud_lineage_digest: String,
    cloud_semantic_digest: String,
}

impl CodexDivergenceFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home_unresolved = temp.path().join("home");
        fs::create_dir(&home_unresolved).unwrap();
        let home = fs::canonicalize(home_unresolved).unwrap();
        let repository = LocalStateRepository::from_home_dir(&home).unwrap();
        let bundle_id = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let scope = install_scope(
            &repository,
            &bundle_id,
            "codex-divergence",
            "replica-codex-divergence",
        );
        let source_thread_id = ThreadId::parse("thread-divergent").unwrap();
        let preserved_thread_id = ThreadId::parse("thread-preserved").unwrap();
        let source_provider_id = "019fac21-4aba-7fc2-bb7d-df711c950987".to_string();
        let child_provider_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c".to_string();
        let session_directory = scope.provider_root.join("sessions/2026/07/29");
        fs::create_dir_all(&session_directory).unwrap();
        let source_path = session_directory.join(format!(
            "rollout-2026-07-29T00-00-00-{source_provider_id}.jsonl"
        ));
        let child_path = session_directory.join(format!(
            "rollout-2026-07-29T00-00-01-{child_provider_id}.jsonl"
        ));
        let local_bytes = codex_divergence_rollout(
            &source_provider_id,
            None,
            &scope.project_root,
            "m3",
            "local branch",
        );
        let child_bytes = codex_fork_of(&local_bytes, &child_provider_id, &source_provider_id);
        let cloud_bytes = codex_divergence_rollout(
            &source_provider_id,
            None,
            &scope.project_root,
            "m4",
            "cloud branch",
        );
        fs::write(&source_path, &local_bytes).unwrap();
        let (local_lineage_digest, local_semantic_digest) = complete_digests(&local_bytes);
        let (cloud_lineage_digest, cloud_semantic_digest) = complete_digests(&cloud_bytes);
        let source_sha256 = sha256(&local_bytes);
        repository
            .mutate_conversation_bindings(|document| {
                document.bindings.push(LocalConversationBinding {
                    host_id: scope.host_id.clone(),
                    replica_id: scope.replica_id.clone(),
                    local_project_id: scope.project_id.clone(),
                    provider_profile_id: scope.profile_id.clone(),
                    thread_id: source_thread_id.clone(),
                    forked_from: None,
                    fork_stamp: None,
                    provider_thread_id: Some(source_provider_id.clone()),
                    materialization_state: ConversationMaterializationState::Resumable,
                    last_observed_file_sha256: Some(source_sha256.clone()),
                    last_observed_head_message_id: Some("m3".to_string()),
                    last_observed_lineage_digest: Some(local_lineage_digest.clone()),
                    last_observed_semantic_digest: Some(local_semantic_digest.clone()),
                    source_revision: Some(6),
                    source_manifest_sha256: Some("e".repeat(64)),
                    revision: 0,
                    created_at: 10,
                    updated_at: 10,
                });
                Ok(())
            })
            .unwrap();
        let bindings_revision = repository.load_conversation_bindings().unwrap().revision;
        let executable = home.join("codex-fixture-bin");
        let executable_bytes = b"fixture is intentionally not executable";
        fs::write(&executable, executable_bytes).unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        let executable_sha256 = sha256(executable_bytes);
        let executors = vec![
            CodexForkExecutor::AppServerStdio,
            CodexForkExecutor::NativePty,
        ];
        let operation = ConversationSyncOperation {
            operation_id: ConversationOperationId::parse("operation-codex-divergence").unwrap(),
            plan_id: Some(PlanId::parse("plan-codex-divergence").unwrap()),
            plan_action_id: Some(ActionId::parse("action-codex-divergence").unwrap()),
            direction: ConversationPlanDirection::Pull,
            phase: ConversationSyncPhase::Prepared,
            cloud: Some(ConversationCloudAnchor {
                storage_id: StorageId::parse("storage-codex-divergence").unwrap(),
                bundle_id,
                revision: 7,
                commit_id: "commit-codex-divergence".to_string(),
                manifest_sha256: "a".repeat(64),
            }),
            host_id: scope.host_id.clone(),
            replica_id: scope.replica_id.clone(),
            local_project_id: scope.project_id.clone(),
            provider_profile_id: scope.profile_id.clone(),
            provider: Provider::Codex,
            thread_id: source_thread_id.clone(),
            operation: ConversationSyncOperationKind::CodexDivergence(CodexDivergenceOperation {
                origin: ForkOrigin::PullPreserve,
                resolution: ConversationDivergenceResolution::PreserveLocalWithNativeFork,
                available_resolutions: vec![
                    ConversationDivergenceResolution::PreserveLocalWithNativeFork,
                    ConversationDivergenceResolution::UseCloudOnly,
                ],
                recommended_resolution:
                    ConversationDivergenceResolution::PreserveLocalWithNativeFork,
                local_branch_preserved: false,
                preserved_thread_id: preserved_thread_id.clone(),
                source_provider_thread_id: source_provider_id.clone(),
                source_transcript_path: source_path.to_string_lossy().into_owned(),
                fork_provider_thread_id: None,
                fork_transcript_path: None,
                fork_transcript_sha256: None,
                local_lineage_digest: local_lineage_digest.clone(),
                local_semantic_digest: local_semantic_digest.clone(),
                local_message_count: 3,
                pinned_local_title: None,
                pinned_cloud_title: None,
                desired_fork_name: "Local chat — local before Pull · 2026-07-29 18:42Z".to_string(),
                desired_original_name: None,
                codex_executable: executable.to_string_lossy().into_owned(),
                codex_executable_sha256: executable_sha256.clone(),
                available_executors: executors.clone(),
                attempts: Vec::new(),
                current_executor: None,
                successful_executor: None,
                fork_name_status: None,
                original_name_status: None,
                inventory_refresh_status: CodexInventoryRefreshStatus::NotAttempted,
            }),
            binding_forked_from: None,
            provider_thread_id: Some(source_provider_id.clone()),
            materialization_state: ConversationMaterializationState::Resumable,
            lineage_digest: Some(cloud_lineage_digest.clone()),
            semantic_digest: Some(cloud_semantic_digest.clone()),
            expected_host_identity_revision: scope.host_identity_revision,
            expected_bindings_document_revision: bindings_revision,
            expected_binding_revision: Some(0),
            expected_profile_revision: 0,
            expected_project_binding_revision: 0,
            expected_local_digest: Some(source_sha256.clone()),
            targets: Vec::new(),
            created_at: 20,
            updated_at: 20,
        };
        operation.validate().unwrap();
        let target = ConversationTargetPayload {
            target_path: source_path.clone(),
            role: ConversationFileRole::Transcript,
            expected: TargetExpectation::Sha256(source_sha256.clone()),
            bytes: cloud_bytes.clone(),
        };
        let fork = CodexForkExecutionRequest {
            codex_home: scope.provider_root,
            project_directory: scope.project_root,
            codex_executable: executable,
            codex_executable_sha256: executable_sha256,
            source_provider_thread_id: source_provider_id.clone(),
            source_transcript_path: source_path.clone(),
            source_transcript_sha256: source_sha256,
            local_lineage_digest: local_lineage_digest.clone(),
            local_semantic_digest: local_semantic_digest.clone(),
            pinned_local_title: None,
            desired_fork_name: "Local chat — local before Pull · 2026-07-29 18:42Z".to_string(),
            available_executors: executors,
        };
        Self {
            _temp: temp,
            home,
            repository,
            operation,
            target,
            fork,
            source_path,
            child_path,
            source_thread_id,
            preserved_thread_id,
            source_provider_id,
            child_provider_id,
            local_bytes,
            child_bytes,
            cloud_bytes,
            local_lineage_digest,
            local_semantic_digest,
            cloud_lineage_digest,
            cloud_semantic_digest,
        }
    }

    fn begin_pending_checkpoint(&self, executor: CodexForkExecutor) {
        let mut failpoints = FailOnce::new(ConversationFailpoint::AfterStaged);
        let error = apply_codex_divergence_locked(
            &self.repository,
            CodexDivergenceMaterializationRequest {
                operation: self.operation.clone(),
                targets: vec![self.target.clone()],
                fork: self.fork.clone(),
            },
            &mut failpoints,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(error.contains("injected failure"), "{error}");
        self.set_pending_executor(executor);
    }

    fn begin_unacknowledged_checkpoint(&self, executor: CodexForkExecutor) {
        self.begin_pending_checkpoint(executor);
        let recovered = recover_conversation_materializations(&self.repository).unwrap();
        assert_eq!(
            recovered.recovery_required,
            vec![self.operation.operation_id.clone()]
        );
        assert!(recovered.completed.is_empty());
    }

    fn write_exact_child(&self) {
        fs::write(&self.child_path, &self.child_bytes).unwrap();
    }

    #[cfg(unix)]
    fn enable_fake_fork_app_server(&mut self) {
        use std::os::unix::fs::PermissionsExt;

        let shell_quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let initialize = json!({"id": 1, "result": {}}).to_string();
        let read = json!({
            "id": 2,
            "result": {
                "thread": {
                    "name": null,
                    "status": {"type": "idle"}
                }
            }
        })
        .to_string();
        let fork = json!({
            "id": 3,
            "result": {
                "thread": {
                    "id": self.child_provider_id,
                    "forkedFromId": self.source_provider_id,
                    "path": self.child_path
                }
            }
        })
        .to_string();
        let named = json!({"id": 3, "result": {}}).to_string();
        let listed = json!({"id": 4, "result": {"data": [], "nextCursor": null}}).to_string();
        let script = format!(
            "#!/bin/sh\n\
             while IFS= read -r line\n\
             do\n\
               case \"$line\" in\n\
                 *'\"method\":\"initialize\"'*) printf '%s\\n' {} ;;\n\
                 *'\"method\":\"thread/read\"'*) printf '%s\\n' {} ;;\n\
                 *'\"method\":\"thread/fork\"'*) printf '%s\\n' {} ;;\n\
                 *'\"method\":\"thread/name/set\"'*) printf '%s\\n' {} ;;\n\
                 *'\"method\":\"thread/list\"'*) printf '%s\\n' {} ;;\n\
               esac\n\
             done\n",
            shell_quote(&initialize),
            shell_quote(&read),
            shell_quote(&fork),
            shell_quote(&named),
            shell_quote(&listed),
        );
        fs::write(&self.fork.codex_executable, script.as_bytes()).unwrap();
        fs::set_permissions(
            &self.fork.codex_executable,
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let digest = sha256(script.as_bytes());
        self.fork.codex_executable_sha256 = digest.clone();
        let divergence = match &mut self.operation.operation {
            ConversationSyncOperationKind::CodexDivergence(divergence) => divergence,
            _ => unreachable!(),
        };
        divergence.codex_executable_sha256 = digest;
    }

    #[cfg(unix)]
    fn enable_fake_idle_app_server(&mut self) {
        use std::os::unix::fs::PermissionsExt;

        let script = br#"#!/bin/sh
while IFS= read -r line
do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{}}'
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' '{"id":2,"result":{"thread":{"name":null,"status":{"type":"idle"}}}}'
      ;;
    *'"method":"thread/list"'*)
      printf '%s\n' '{"id":2,"result":{"data":[],"nextCursor":null}}'
      ;;
  esac
done
"#;
        fs::write(&self.fork.codex_executable, script).unwrap();
        fs::set_permissions(
            &self.fork.codex_executable,
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let digest = sha256(script);
        self.fork.codex_executable_sha256 = digest.clone();
        let divergence = match &mut self.operation.operation {
            ConversationSyncOperationKind::CodexDivergence(divergence) => divergence,
            _ => unreachable!(),
        };
        divergence.codex_executable_sha256 = digest;
    }

    fn current_operation(&self) -> ConversationSyncOperation {
        self.repository
            .load_conversation_sync_journal()
            .unwrap()
            .operations
            .into_iter()
            .find(|operation| operation.operation_id == self.operation.operation_id)
            .unwrap()
    }

    fn set_pending_executor(&self, executor: CodexForkExecutor) {
        let journal = self.repository.load_conversation_sync_journal().unwrap();
        self.repository
            .mutate_conversation_sync_journal(journal.revision, |journal| {
                let operation = journal
                    .operations
                    .iter_mut()
                    .find(|operation| operation.operation_id == self.operation.operation_id)
                    .unwrap();
                let divergence = match &mut operation.operation {
                    ConversationSyncOperationKind::CodexDivergence(divergence) => divergence,
                    _ => unreachable!(),
                };
                divergence.attempts = vec![CodexForkAttempt {
                    executor,
                    status: CodexForkAttemptStatus::Pending,
                }];
                divergence.current_executor = Some(executor);
                divergence.successful_executor = None;
                operation.phase = ConversationSyncPhase::ForkAttemptPending;
                Ok(())
            })
            .unwrap();
    }

    fn native_child_count(&self) -> usize {
        fs::read_dir(self.child_path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path() != self.source_path
                    && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
            })
            .count()
    }

    fn persisted_receipt(
        &self,
        repository: &LocalStateRepository,
    ) -> crate::project_sync::domain::ConversationApplyReceipt {
        repository
            .load_materializations()
            .unwrap()
            .records
            .into_iter()
            .flat_map(|record| record.receipts)
            .find_map(|receipt| receipt.conversation)
            .unwrap()
    }

    fn assert_complete(
        &self,
        receipt: &crate::project_sync::domain::ConversationApplyReceipt,
        executor: CodexForkExecutor,
    ) {
        self.assert_complete_in(&self.repository, receipt, executor);
    }

    fn assert_complete_in(
        &self,
        repository: &LocalStateRepository,
        receipt: &crate::project_sync::domain::ConversationApplyReceipt,
        executor: CodexForkExecutor,
    ) {
        assert_eq!(fs::read(&self.source_path).unwrap(), self.cloud_bytes);
        assert_eq!(fs::read(&self.child_path).unwrap(), self.child_bytes);
        assert_eq!(receipt.thread_id, self.source_thread_id);
        assert_eq!(
            receipt.provider_thread_id.as_deref(),
            Some(self.source_provider_id.as_str())
        );
        assert_eq!(
            receipt.lineage_digest.as_deref(),
            Some(self.cloud_lineage_digest.as_str())
        );
        assert_eq!(
            receipt.semantic_digest.as_deref(),
            Some(self.cloud_semantic_digest.as_str())
        );
        let divergence = receipt.divergence.as_ref().unwrap();
        assert_eq!(
            divergence.resolution,
            ConversationDivergenceResolution::PreserveLocalWithNativeFork
        );
        assert_eq!(
            divergence.preserved_thread_id.as_ref(),
            Some(&self.preserved_thread_id)
        );
        assert_eq!(
            divergence.preserved_provider_thread_id.as_deref(),
            Some(self.child_provider_id.as_str())
        );
        assert_eq!(divergence.executor, Some(executor));
        assert_eq!(
            divergence.fork_name_status,
            Some(ConversationNameStatus::Deferred)
        );
        assert_eq!(
            divergence.original_name_status,
            Some(ConversationNameStatus::Unchanged)
        );
        assert_eq!(
            divergence.inventory_refresh_status,
            CodexInventoryRefreshStatus::Deferred
        );

        let document = repository.load_conversation_bindings().unwrap();
        assert_eq!(document.bindings.len(), 2);
        let original = document
            .bindings
            .iter()
            .find(|binding| binding.thread_id == self.source_thread_id)
            .unwrap();
        assert_eq!(
            original.provider_thread_id.as_deref(),
            Some(self.source_provider_id.as_str())
        );
        assert!(original.forked_from.is_none());
        assert_eq!(
            original.last_observed_lineage_digest.as_deref(),
            Some(self.cloud_lineage_digest.as_str())
        );
        assert_eq!(
            original.last_observed_semantic_digest.as_deref(),
            Some(self.cloud_semantic_digest.as_str())
        );
        let preserved = document
            .bindings
            .iter()
            .find(|binding| binding.thread_id == self.preserved_thread_id)
            .unwrap();
        assert_eq!(
            preserved.provider_thread_id.as_deref(),
            Some(self.child_provider_id.as_str())
        );
        assert_eq!(preserved.forked_from.as_ref(), Some(&self.source_thread_id));
        assert_eq!(
            preserved.last_observed_lineage_digest.as_deref(),
            Some(self.local_lineage_digest.as_str())
        );
        assert_eq!(
            preserved.last_observed_semantic_digest.as_deref(),
            Some(self.local_semantic_digest.as_str())
        );
        let backup = repository
            .root()
            .join("backups/conversations/operation-codex-divergence/0.bak");
        assert_eq!(fs::read(backup).unwrap(), self.local_bytes);
        assert!(repository
            .load_conversation_sync_journal()
            .unwrap()
            .operations
            .is_empty());
    }
}

#[derive(Clone)]
struct FailOnce {
    target: ConversationFailpoint,
    fired: bool,
}

impl FailOnce {
    fn new(target: ConversationFailpoint) -> Self {
        Self {
            target,
            fired: false,
        }
    }
}

impl ConversationFailpoints for FailOnce {
    fn hit(&mut self, point: &ConversationFailpoint) -> Result<(), String> {
        if !self.fired && point == &self.target {
            self.fired = true;
            Err(format!("injected failure at {point:?}"))
        } else {
            Ok(())
        }
    }
}

struct MaterializationFixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    repository: LocalStateRepository,
    operation: ConversationSyncOperation,
    targets: Vec<ConversationTargetPayload>,
}

pub(super) struct InstalledScope {
    pub(super) host_id: crate::project_sync::domain::HostId,
    pub(super) host_identity_revision: u64,
    pub(super) replica_id: ReplicaId,
    pub(super) project_id: LocalProjectId,
    pub(super) profile_id: LocalProviderProfileId,
    pub(super) project_root: PathBuf,
    pub(super) provider_root: PathBuf,
}

pub(super) fn install_scope(
    repository: &LocalStateRepository,
    bundle_id: &BundleId,
    suffix: &str,
    replica: &str,
) -> InstalledScope {
    install_scope_for_provider(repository, bundle_id, suffix, replica, Provider::Codex)
}

pub(super) fn install_scope_for_provider(
    repository: &LocalStateRepository,
    bundle_id: &BundleId,
    suffix: &str,
    replica: &str,
    provider: Provider,
) -> InstalledScope {
    let identity = repository.load_or_create_host_identity().unwrap();
    let home = repository.root().parent().unwrap();
    let project_root = home.join(format!("project-{suffix}"));
    let provider_root = home.join(format!("codex-{suffix}"));
    fs::create_dir(&project_root).unwrap();
    fs::create_dir(&provider_root).unwrap();
    let project_root = fs::canonicalize(project_root).unwrap();
    let provider_root = fs::canonicalize(provider_root).unwrap();
    let project_id = LocalProjectId::parse(format!("project-{suffix}")).unwrap();
    let profile_id = LocalProviderProfileId::parse(format!("profile-{suffix}")).unwrap();
    let replica_id = ReplicaId::parse(replica).unwrap();
    repository
        .mutate_config(|config| {
            config.projects.push(LocalProjectRegistration {
                local_project_id: project_id.clone(),
                bundle_id: bundle_id.clone(),
                display_name: format!("Project {suffix}"),
                local_alias: None,
                repository_fingerprint: None,
                recipe: BundleRecipe::default(),
                recipe_bases: BTreeMap::new(),
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
    repository
        .mutate_bindings(|_, machine| {
            machine.profiles.push(ProviderProfile {
                profile_id: profile_id.clone(),
                provider,
                display_name: format!("{} {suffix}", provider.as_str()),
                path: provider_root.to_string_lossy().into_owned(),
                canonical_path: provider_root.to_string_lossy().into_owned(),
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            machine.bindings.push(ProjectBinding {
                host_id: identity.host_id.clone(),
                replica_id: replica_id.clone(),
                local_project_id: project_id.clone(),
                bundle_id: bundle_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                canonical_project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(provider, profile_id.clone())]),
                codex_home: None,
                claude_home: None,
                state: BindingState::Active,
                revision: 0,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
    InstalledScope {
        host_id: identity.host_id,
        host_identity_revision: identity.revision,
        replica_id,
        project_id,
        profile_id,
        project_root,
        provider_root,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_operation(
    world: &VariantWorld,
    scope: &InstalledScope,
    operation_id: &str,
    plan_id: &str,
    revision: u64,
    commit_id: String,
    manifest_sha256: String,
    thread_id: &ThreadId,
    bindings_revision: u64,
    binding_revision: Option<u64>,
    transcript: &[u8],
    lineage_digest: &str,
    semantic_digest: &str,
) -> ConversationSyncOperation {
    ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(plan_id).unwrap()),
        plan_action_id: None,
        direction: ConversationPlanDirection::Push,
        phase: ConversationSyncPhase::Prepared,
        cloud: Some(ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision,
            commit_id,
            manifest_sha256,
        }),
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: Provider::Codex,
        thread_id: thread_id.clone(),
        operation: ConversationSyncOperationKind::CloudPublication,
        binding_forked_from: None,
        provider_thread_id: Some("source-thread".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(lineage_digest.to_string()),
        semantic_digest: Some(semantic_digest.to_string()),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings_revision,
        expected_binding_revision: binding_revision,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(transcript)),
        targets: vec![],
        created_at: 30,
        updated_at: 30,
    }
}

fn cloud_publication(
    outcome: crate::project_sync::bundle_engine::PublishedBundle,
) -> ConversationCloudPublication {
    ConversationCloudPublication {
        revision: outcome.snapshot.head.revision,
        commit_id: outcome.snapshot.head.commit_id,
        manifest_sha256: outcome.snapshot.head.manifest_sha256,
    }
}

impl MaterializationFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home_unresolved = temp.path().join("home");
        fs::create_dir(&home_unresolved).unwrap();
        let home = fs::canonicalize(&home_unresolved).unwrap();
        let repository = LocalStateRepository::from_home_dir(&home).unwrap();
        let identity = repository.load_or_create_host_identity().unwrap();
        let project_root = home.join("project");
        let provider_root = home.join("codex");
        fs::create_dir(&project_root).unwrap();
        fs::create_dir(&provider_root).unwrap();
        fs::create_dir(provider_root.join("sessions")).unwrap();
        fs::create_dir(provider_root.join("sidecars")).unwrap();
        let project_root = fs::canonicalize(project_root).unwrap();
        let provider_root = fs::canonicalize(provider_root).unwrap();

        let local_project_id = LocalProjectId::parse("project-materialize").unwrap();
        let bundle_id = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let profile_id = LocalProviderProfileId::parse("profile-materialize").unwrap();
        let replica_id = ReplicaId::parse("replica-materialize").unwrap();
        let mut config = repository.load_config().unwrap();
        config.projects.push(LocalProjectRegistration {
            local_project_id: local_project_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: "Materialization fixture".to_string(),
            local_alias: None,
            repository_fingerprint: None,
            recipe: BundleRecipe::default(),
            recipe_bases: BTreeMap::new(),
            revision: 0,
            created_at: 1,
            updated_at: 1,
        });
        repository.save_config(config).unwrap();
        repository
            .mutate_bindings(|_, machine| {
                machine.profiles.push(ProviderProfile {
                    profile_id: profile_id.clone(),
                    provider: Provider::Codex,
                    display_name: "Codex fixture".to_string(),
                    path: provider_root.to_string_lossy().into_owned(),
                    canonical_path: provider_root.to_string_lossy().into_owned(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                machine.bindings.push(ProjectBinding {
                    host_id: identity.host_id.clone(),
                    replica_id: replica_id.clone(),
                    local_project_id: local_project_id.clone(),
                    bundle_id: bundle_id.clone(),
                    project_root: project_root.to_string_lossy().into_owned(),
                    canonical_project_root: project_root.to_string_lossy().into_owned(),
                    profile_ids: BTreeMap::from([(Provider::Codex, profile_id.clone())]),
                    codex_home: None,
                    claude_home: None,
                    state: BindingState::Active,
                    revision: 0,
                    updated_at: 1,
                });
                Ok(())
            })
            .unwrap();

        let thread_id = "destination-thread";
        let transcript = TranscriptBuilder::new(thread_id, project_root.to_str().unwrap())
            .message("m-a", "user", "question")
            .message("m-b", "assistant", "answer")
            .build();
        let analysis = analyze_codex_transcript(&transcript);
        let (lineage_digest, semantic_digest) = match analysis.lineage {
            CodecLineage::Complete {
                lineage_digest,
                semantic_digest,
                ..
            } => (lineage_digest, semantic_digest),
            other => panic!("fixture transcript is not resumable: {other:?}"),
        };
        let index = format!(
            "{{\"id\":\"{thread_id}\",\"thread_name\":\"Fixture\",\
             \"updated_at\":\"2026-07-18T00:00:00.000000Z\"}}\n"
        )
        .into_bytes();
        let sidecar =
            format!("{{\"thread_id\":\"{thread_id}\",\"state\":\"ready\"}}\n").into_bytes();
        let targets = vec![
            ConversationTargetPayload {
                target_path: provider_root.join("sessions/destination-thread.jsonl"),
                role: ConversationFileRole::Transcript,
                expected: TargetExpectation::Absent,
                bytes: transcript,
            },
            ConversationTargetPayload {
                target_path: provider_root.join("session_index.jsonl"),
                role: ConversationFileRole::ProviderIndexFragment,
                expected: TargetExpectation::Absent,
                bytes: index,
            },
            ConversationTargetPayload {
                target_path: provider_root.join("sidecars/destination-thread.json"),
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: "thread-state".to_string(),
                },
                expected: TargetExpectation::Absent,
                bytes: sidecar,
            },
        ];
        let operation = ConversationSyncOperation {
            operation_id: ConversationOperationId::parse("operation-materialize").unwrap(),
            plan_id: Some(PlanId::parse("plan-materialize").unwrap()),
            plan_action_id: None,
            direction: ConversationPlanDirection::Pull,
            phase: ConversationSyncPhase::Prepared,
            cloud: Some(ConversationCloudAnchor {
                storage_id: crate::project_sync::domain::StorageId::parse("storage-materialize")
                    .unwrap(),
                bundle_id,
                revision: 7,
                commit_id: "commit-materialize".to_string(),
                manifest_sha256: "a".repeat(64),
            }),
            host_id: identity.host_id,
            replica_id,
            local_project_id,
            provider_profile_id: profile_id,
            provider: Provider::Codex,
            thread_id: ThreadId::parse("thread-materialize").unwrap(),
            operation: ConversationSyncOperationKind::ProviderMaterialization,
            binding_forked_from: None,
            provider_thread_id: Some(thread_id.to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            lineage_digest: Some(lineage_digest),
            semantic_digest: Some(semantic_digest),
            expected_host_identity_revision: identity.revision,
            expected_bindings_document_revision: 0,
            expected_binding_revision: None,
            expected_profile_revision: 0,
            expected_project_binding_revision: 0,
            expected_local_digest: None,
            targets: vec![],
            created_at: 10,
            updated_at: 10,
        };
        Self {
            _temp: temp,
            home,
            repository,
            operation,
            targets,
        }
    }

    fn request(&self) -> ConversationMaterializationRequest {
        ConversationMaterializationRequest {
            operation: self.operation.clone(),
            targets: self.targets.clone(),
        }
    }
}

fn assert_materialization_fixed_point(
    fixture: &MaterializationFixture,
    repository: &LocalStateRepository,
    complete: bool,
) {
    for target in &fixture.targets {
        if complete {
            assert_eq!(fs::read(&target.target_path).unwrap(), target.bytes);
        } else {
            assert!(!target.target_path.exists());
        }
        let parent = target.target_path.parent().unwrap();
        assert!(!fs::read_dir(parent).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".mallard-")
        }));
    }
    let bindings = repository.load_conversation_bindings().unwrap();
    let receipts = repository.load_materializations().unwrap();
    if complete {
        assert_eq!(bindings.bindings.len(), 1);
        assert_eq!(
            bindings.bindings[0].provider_thread_id.as_deref(),
            Some("destination-thread")
        );
        assert_eq!(
            bindings.bindings[0].materialization_state,
            ConversationMaterializationState::Resumable
        );
        assert_eq!(receipts.records.len(), 1);
        assert_eq!(receipts.records[0].receipts.len(), 1);
        assert_eq!(
            receipts.records[0].receipts[0]
                .conversation
                .as_ref()
                .unwrap()
                .journal_operation_id
                .as_str(),
            "operation-materialize"
        );
    } else {
        assert!(bindings.bindings.is_empty());
        assert!(receipts.records.is_empty());
    }
    assert!(!repository
        .root()
        .join("conversation_operations/operation-materialize")
        .exists());
}

#[cfg(unix)]
#[test]
fn exact_app_server_fork_is_stamped_before_the_original_is_replaced() {
    let mut fixture = CodexDivergenceFixture::new();
    fixture.write_exact_child();
    fixture.enable_fake_fork_app_server();
    let outcome = apply_codex_divergence_locked(
        &fixture.repository,
        CodexDivergenceMaterializationRequest {
            operation: fixture.operation.clone(),
            targets: vec![fixture.target.clone()],
            fork: fixture.fork.clone(),
        },
        &mut NoConversationFailpoints,
        &mut |_, _| {},
    )
    .unwrap();
    let CodexDivergenceApplyOutcome::Committed(receipt) = outcome else {
        panic!("exact app-server fork path did not commit the divergence");
    };
    fixture.assert_complete(&receipt, CodexForkExecutor::AppServerStdio);

    let document = fixture.repository.load_conversation_bindings().unwrap();
    let source = document
        .bindings
        .iter()
        .find(|binding| binding.thread_id == fixture.source_thread_id)
        .unwrap();
    let stamp = source
        .fork_stamp
        .as_ref()
        .expect("verified fork stamps the source binding");
    assert_eq!(stamp.fork_thread_id, fixture.preserved_thread_id);
    assert_eq!(stamp.fork_provider_thread_id, fixture.child_provider_id);
    assert_eq!(
        stamp.fork_transcript_path,
        fixture.child_path.to_string_lossy()
    );
    assert_eq!(stamp.lineage_digest, fixture.local_lineage_digest);
    assert_eq!(stamp.semantic_digest, fixture.local_semantic_digest);
    assert_eq!(stamp.message_count, 3);
    assert_eq!(
        source.last_observed_lineage_digest.as_deref(),
        Some(fixture.cloud_lineage_digest.as_str())
    );
    assert_eq!(
        source.last_observed_semantic_digest.as_deref(),
        Some(fixture.cloud_semantic_digest.as_str())
    );
}

#[cfg(test)]
fn preservation_review(bytes: &[u8], provider_thread_id: &str) -> LocalThreadReview {
    let analysis = analyze_codex_transcript(bytes);
    let logical_path = format!("state/codex/{provider_thread_id}.jsonl");
    LocalThreadReview {
        binding: None,
        binding_resolution: BindingResolution::Bound,
        resource_id: conversation_resource_id(
            Provider::Codex,
            &ThreadId::parse("thread-preservation-review").unwrap(),
        )
        .unwrap(),
        descriptor: None,
        conversation: provider_capture::CapturedConversation {
            provider: provider_capture::Provider::Codex,
            lineage_codec: "codex-jsonl".to_string(),
            source_thread_id: Some(provider_thread_id.to_string()),
            source_cwd: analysis.source_cwd.clone(),
            transcript_logical_path: logical_path.clone(),
            record_count: analysis.record_count,
            lineage: analysis.lineage,
        },
        transcript: provider_capture::CapturedFile {
            logical_path,
            resource_id: format!("codex:session:{provider_thread_id}"),
            bytes: bytes.to_vec(),
            source_mtime: 0,
            mode: 0o600,
        },
    }
}

#[test]
fn diverged_forked_witness_requires_stamp_pair_fork_and_prefix() {
    let cwd = PathBuf::from("/tmp/preservation-project");
    let source_provider_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
    let fork_provider_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
    let preserved_thread_id = ThreadId::parse("thread-preserved").unwrap();
    let source_bytes =
        codex_divergence_rollout(source_provider_id, None, &cwd, "m3", "local branch");
    let fork_bytes = codex_fork_of(&source_bytes, fork_provider_id, source_provider_id);
    let (lineage, semantic) = complete_digests(&source_bytes);
    let stamp = ForkStamp {
        fork_thread_id: preserved_thread_id.clone(),
        fork_provider_thread_id: fork_provider_id.to_string(),
        fork_transcript_path: "/tmp/preservation-fork.jsonl".to_string(),
        lineage_digest: lineage.clone(),
        semantic_digest: semantic.clone(),
        message_count: 3,
        forked_at: 40,
    };
    let binding = LocalConversationBinding {
        host_id: crate::project_sync::domain::HostId::parse(&"1".repeat(32)).unwrap(),
        replica_id: ReplicaId::parse("replica-preservation").unwrap(),
        local_project_id: LocalProjectId::parse("project-preservation").unwrap(),
        provider_profile_id: LocalProviderProfileId::parse("profile-preservation").unwrap(),
        thread_id: ThreadId::parse("thread-divergent").unwrap(),
        forked_from: None,
        fork_stamp: Some(stamp),
        provider_thread_id: Some(source_provider_id.to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        last_observed_file_sha256: None,
        last_observed_head_message_id: None,
        last_observed_lineage_digest: None,
        last_observed_semantic_digest: None,
        source_revision: None,
        source_manifest_sha256: None,
        revision: 1,
        created_at: 10,
        updated_at: 40,
    };
    let source_review = preservation_review(&source_bytes, source_provider_id);
    let mut local_threads = BTreeMap::new();

    // Fork untouched since adoption: the full witness holds.
    local_threads.insert(
        preserved_thread_id.clone(),
        preservation_review(&fork_bytes, fork_provider_id),
    );
    let preservation =
        diverged_fork_preservation(Some(&binding), Some(&source_review), &local_threads)
            .expect("an exact fork satisfies the witness");
    assert_eq!(preservation.fork_thread_id, preserved_thread_id);
    assert_eq!(preservation.forked_at, 40);

    // A continued fork still carries the stamped chain as a prefix.
    let mut continued = fork_bytes.clone();
    continued.extend_from_slice(
        json!({
            "timestamp": "2026-07-29T13:00:00Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "m4",
                "parent_id": "m3",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "continued in the fork"}],
            },
        })
        .to_string()
        .as_bytes(),
    );
    continued.push(b'\n');
    local_threads.insert(
        preserved_thread_id.clone(),
        preservation_review(&continued, fork_provider_id),
    );
    assert!(
        diverged_fork_preservation(Some(&binding), Some(&source_review), &local_threads).is_some(),
        "a continued fork preserves the stamped prefix"
    );

    // A fork whose early history was rewritten no longer preserves the branch.
    let rewritten_origin =
        codex_divergence_rollout(source_provider_id, None, &cwd, "m9", "different branch");
    let rewritten = codex_fork_of(&rewritten_origin, fork_provider_id, source_provider_id);
    local_threads.insert(
        preserved_thread_id.clone(),
        preservation_review(&rewritten, fork_provider_id),
    );
    assert!(
        diverged_fork_preservation(Some(&binding), Some(&source_review), &local_threads).is_none(),
        "a rewritten fork fails the prefix condition"
    );

    // A deleted fork fails the presence condition.
    local_threads.remove(&preserved_thread_id);
    assert!(
        diverged_fork_preservation(Some(&binding), Some(&source_review), &local_threads).is_none(),
        "a missing fork fails the witness"
    );

    // A source that moved past the stamp fails the pair condition.
    local_threads.insert(
        preserved_thread_id.clone(),
        preservation_review(&fork_bytes, fork_provider_id),
    );
    let moved_bytes =
        codex_divergence_rollout(source_provider_id, None, &cwd, "m5", "new local work");
    let moved_review = preservation_review(&moved_bytes, source_provider_id);
    assert!(
        diverged_fork_preservation(Some(&binding), Some(&moved_review), &local_threads).is_none(),
        "a moved source fails the pair condition"
    );

    // A stamp longer than the fork chain fails the length condition.
    let mut short_binding = binding.clone();
    short_binding.fork_stamp.as_mut().unwrap().message_count = 4;
    assert!(
        diverged_fork_preservation(Some(&short_binding), Some(&source_review), &local_threads)
            .is_none(),
        "a fork shorter than the stamped chain fails the witness"
    );

    // No stamp, no witness.
    let mut unstamped = binding.clone();
    unstamped.fork_stamp = None;
    assert!(
        diverged_fork_preservation(Some(&unstamped), Some(&source_review), &local_threads)
            .is_none()
    );
}
