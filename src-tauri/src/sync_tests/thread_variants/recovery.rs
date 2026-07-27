use super::harness::{sha256, TranscriptBuilder, VariantWorld};
use super::relation_matrix::reviewed_action;
use crate::project_sync_v3::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage,
};
use crate::project_sync_v3::conversation_materialization::{
    apply_conversation_cloud_publication, apply_conversation_materialization,
    recover_conversation_cloud_publications, recover_conversation_materializations,
    ConversationCloudPublication, ConversationFailpoint, ConversationFailpoints,
    ConversationMaterializationRequest, ConversationTargetPayload, NoConversationFailpoints,
};
use crate::project_sync_v3::conversation_sync::{
    validate_conversation_action_pins, ConversationStalePin,
};
use crate::project_sync_v3::domain::{
    BindingState, BundleId, BundleRecipe, ConversationFileRole, ConversationJournalTarget,
    ConversationMaterializationState, ConversationOperationId, ConversationPlanActionType,
    ConversationPlanDirection, ConversationPushPlan, ConversationReasonCode, ConversationRelation,
    ConversationSyncOperation, ConversationSyncPhase, LocalConversationBinding, LocalProjectId,
    LocalProjectRegistration, LocalProviderProfileId, PlanId, ProjectBinding, Provider,
    ProviderProfile, ReplicaId, StorageId, TargetExpectation, ThreadId, PUSH_PLAN_SCHEMA,
};
use crate::project_sync_v3::persistence::V3Repository;
use crate::project_sync_v3::provider_capture::{
    capture_selected, discover_project, CaptureRequest,
};
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
        Box<dyn Fn(&mut crate::project_sync_v3::conversation_sync::ConversationApplyContext)>,
    )> = vec![
        (
            ConversationStalePin::Expired,
            Box::new(|value| value.now = 9_999),
        ),
        (
            ConversationStalePin::Host,
            Box::new(|value| {
                value.host_id = crate::project_sync_v3::domain::HostId::parse(
                    "22222222222222222222222222222222",
                )
                .unwrap()
            }),
        ),
        (
            ConversationStalePin::Replica,
            Box::new(|value| {
                value.replica_id =
                    crate::project_sync_v3::domain::ReplicaId::parse("replica-other").unwrap()
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
            ConversationStalePin::Generation,
            Box::new(|value| value.generation += 1),
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
            ConversationPlanActionType::Fork,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::Fork,
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
        let (action, mut current) = reviewed_action(
            direction,
            action_type,
            ConversationReasonCode::StalePlan,
            ConversationRelation::Unknown,
        );
        action.validate().unwrap();
        current.generation += 1;
        assert_eq!(
            validate_conversation_action_pins(&action, &current),
            Err(ConversationStalePin::Generation),
            "{direction:?} {action_type:?}",
        );
    }
}

#[test]
fn reviewed_push_plan_is_immutable_restart_readable_and_scope_pinned() {
    let temp = tempfile::tempdir().unwrap();
    let repository = V3Repository::from_home_dir(temp.path()).unwrap();
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
        generation: action.pinned_generation,
        commit_id: action.pinned_commit_id.clone(),
        manifest_sha256: action.pinned_manifest_sha256.clone(),
        created_at: 1_000,
        expires_at: action.expires_at,
        actions: vec![action],
    };
    repository.save_conversation_push_plan(&plan).unwrap();
    assert!(repository.save_conversation_push_plan(&plan).is_err());
    let reopened = V3Repository::from_home_dir(temp.path()).unwrap();
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
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: 1,
        commit_id: "commit-a".to_string(),
        manifest_sha256: "a".repeat(64),
        host_id: identity.host_id,
        replica_id: crate::project_sync_v3::domain::ReplicaId::parse("replica-a").unwrap(),
        local_project_id: crate::project_sync_v3::domain::LocalProjectId::parse("project-a")
            .unwrap(),
        provider_profile_id: crate::project_sync_v3::domain::LocalProviderProfileId::parse(
            "profile-a",
        )
        .unwrap(),
        provider: Provider::Codex,
        thread_id: ThreadId::parse("thread-journal").unwrap(),
        old_thread_id: None,
        rekey_committed: false,
        result_thread_id: ThreadId::parse("thread-journal").unwrap(),
        forked_from_provider_thread_id: None,
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
            role: crate::project_sync_v3::domain::ConversationFileRole::Transcript,
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
    let reopened = crate::project_sync_v3::persistence::V3Repository::from_home_dir(
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
        direction: ConversationPlanDirection::Push,
        phase: ConversationSyncPhase::CloudPublished,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: 2,
        commit_id: "commit-a".to_string(),
        manifest_sha256: "a".repeat(64),
        host_id: identity.host_id,
        replica_id: crate::project_sync_v3::domain::ReplicaId::parse("replica-a").unwrap(),
        local_project_id: crate::project_sync_v3::domain::LocalProjectId::parse("project-a")
            .unwrap(),
        provider_profile_id: crate::project_sync_v3::domain::LocalProviderProfileId::parse(
            "profile-a",
        )
        .unwrap(),
        provider: Provider::Codex,
        thread_id: ThreadId::parse("thread-journal").unwrap(),
        old_thread_id: None,
        rekey_committed: false,
        result_thread_id: ThreadId::parse("thread-journal").unwrap(),
        forked_from_provider_thread_id: None,
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

        let reopened = V3Repository::from_home_dir(&fixture.home).unwrap();
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
        world
            .repo_a
            .mutate_conversation_bindings(|document| {
                document.bindings.push(LocalConversationBinding {
                    host_id: scope.host_id.clone(),
                    replica_id: scope.replica_id.clone(),
                    local_project_id: scope.project_id.clone(),
                    provider_profile_id: scope.profile_id.clone(),
                    thread_id: thread_id.clone(),
                    provider_thread_id: Some("source-thread".to_string()),
                    materialization_state: ConversationMaterializationState::Resumable,
                    last_observed_file_sha256: Some(sha256(&advanced_transcript)),
                    last_observed_head_message_id: Some("m-b".to_string()),
                    last_observed_lineage_digest: Some(lineage.clone()),
                    last_observed_semantic_digest: Some(semantic.clone()),
                    source_generation: Some(initial.snapshot.head.latest.generation),
                    source_manifest_sha256: Some(
                        initial.snapshot.head.latest.manifest_sha256.clone(),
                    ),
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
            Some((initial.snapshot.head.clone(), initial.head_token)),
        );
        let operation = push_operation(
            &world,
            &scope,
            "operation-cloud-checkpoint",
            "plan-cloud-checkpoint",
            initial.snapshot.head.latest.generation,
            initial.snapshot.head.latest.commit_id,
            initial.snapshot.head.latest.manifest_sha256,
            &thread_id,
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

        let reopened = V3Repository::from_home_dir(world.repo_a.root().parent().unwrap()).unwrap();
        let publication = if world
            .engine()
            .inspect(&world.bundle_id)
            .unwrap()
            .head
            .latest
            .generation
            == 2
        {
            let snapshot = world.engine().inspect(&world.bundle_id).unwrap();
            Some(ConversationCloudPublication {
                generation: snapshot.head.latest.generation,
                commit_id: snapshot.head.latest.commit_id,
                manifest_sha256: snapshot.head.latest.manifest_sha256,
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
            assert_eq!(final_snapshot.head.latest.generation, 2, "{checkpoint:?}");
            assert_eq!(final_bindings.bindings.len(), 1);
            assert_eq!(
                final_bindings.bindings[0].source_generation,
                Some(2),
                "{checkpoint:?}",
            );
            assert_eq!(final_receipts.records.len(), 1, "{checkpoint:?}");
            assert_eq!(final_receipts.records[0].receipts.len(), 1);
            assert_eq!(
                first.completed.len(),
                usize::from(checkpoint != ConversationFailpoint::AfterCleanup),
            );
        } else {
            assert_eq!(final_snapshot.head.latest.generation, 1, "{checkpoint:?}");
            assert_eq!(final_bindings.bindings[0].source_generation, Some(1));
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
    index["cwd"] = serde_json::Value::String(project_root.clone());
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
    assert_eq!(committed_index["cwd"], project_root);
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
        crate::project_sync_v3::domain::classify_conversation_relation(
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
    let reopened = V3Repository::from_home_dir(&fixture.home).unwrap();
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
fn fork_push_reuses_rekey_after_cas_failure_and_post_cas_restart() {
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-push");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let cloud_id = ThreadId::parse("thread-shared").unwrap();
    let forked_id = ThreadId::parse("thread-forked-local").unwrap();
    let cloud_initial = super::harness::ThreadSpec::new(
        cloud_id.as_str(),
        "source-thread",
        super::harness::chain(
            "source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "answer"),
                ("m-cloud", "cloud fork"),
            ],
        ),
        origin_b.clone(),
        10,
    );
    let initial = world.publish(&[cloud_initial.clone()], origin_b.clone(), 10);
    let local_transcript = super::harness::chain(
        "source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-local", "local fork"),
        ],
    );
    let local_analysis = analyze_codex_transcript(&local_transcript);
    let (lineage, semantic) = match local_analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        _ => unreachable!(),
    };
    let scope = install_scope(&world.repo_a, &world.bundle_id, "push", "replica-push");
    let local_file = scope.provider_root.join("source-thread.jsonl");
    fs::write(&local_file, &local_transcript).unwrap();
    world
        .repo_a
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                thread_id: cloud_id.clone(),
                provider_thread_id: Some("source-thread".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(&local_transcript)),
                last_observed_head_message_id: Some("m-local".to_string()),
                last_observed_lineage_digest: Some(lineage.clone()),
                last_observed_semantic_digest: Some(semantic.clone()),
                source_generation: Some(initial.snapshot.head.latest.generation),
                source_manifest_sha256: Some(initial.snapshot.head.latest.manifest_sha256.clone()),
                revision: 0,
                created_at: 10,
                updated_at: 10,
            });
            Ok(())
        })
        .unwrap();

    let mut forked_spec = super::harness::ThreadSpec::new(
        forked_id.as_str(),
        "source-thread",
        local_transcript.clone(),
        origin_a.clone(),
        20,
    );
    forked_spec.forked_from = Some(cloud_id.clone());
    let old_token = world.current_head().unwrap();
    let stale_request = super::harness::publish_request(
        world.bundle_id.clone(),
        &[cloud_initial.clone(), forked_spec.clone()],
        origin_a.clone(),
        20,
        Some(&initial.snapshot.manifest),
        Some(old_token),
    );
    let mut cloud_advanced = cloud_initial.clone();
    cloud_advanced.transcript = super::harness::chain(
        "source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-cloud", "cloud fork"),
            ("m-cloud-2", "remote continuation"),
        ],
    );
    cloud_advanced.updated_at = 21;
    cloud_advanced.last_updated_by = Some(origin_b.clone());
    world.publish(&[cloud_advanced.clone()], origin_b, 21);

    let first_operation = push_operation(
        &world,
        &scope,
        "operation-push-cas",
        "plan-push-cas",
        initial.snapshot.head.latest.generation,
        initial.snapshot.head.latest.commit_id.clone(),
        initial.snapshot.head.latest.manifest_sha256.clone(),
        &cloud_id,
        &forked_id,
        1,
        Some(0),
        &local_transcript,
        &lineage,
        &semantic,
    );
    let engine = world.engine();
    let mut no_failpoints = NoConversationFailpoints;
    let error = apply_conversation_cloud_publication(
        &world.repo_a,
        first_operation,
        || engine.publish(stale_request).map(cloud_publication),
        || Ok(None),
        &mut no_failpoints,
    )
    .unwrap_err();
    assert!(error.contains("changed") || error.contains("conflict"));
    // The reviewed rekey survives the failed publication: the conversation is
    // now bound to the minted fork thread and simply classifies CloudMissing
    // until the next push publishes it under the already-minted ID.
    let after_failure = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(after_failure.bindings.len(), 1);
    assert_eq!(after_failure.bindings[0].thread_id, forked_id);
    assert_eq!(
        after_failure.bindings[0].provider_thread_id.as_deref(),
        Some("source-thread")
    );
    assert_eq!(after_failure.bindings[0].source_generation, None);
    assert!(world
        .repo_a
        .load_conversation_sync_journal()
        .unwrap()
        .operations
        .is_empty());

    let current = world.engine().inspect(&world.bundle_id).unwrap();
    let current_token = world.current_head().unwrap();
    let mut retry_forked = forked_spec.clone();
    retry_forked.updated_at = 30;
    let retry_request = super::harness::publish_request(
        world.bundle_id.clone(),
        &[cloud_advanced, retry_forked],
        origin_a,
        30,
        Some(&current.manifest),
        Some(current_token),
    );
    let retry_state = world.repo_a.load_conversation_bindings().unwrap();
    let retry_operation = push_operation(
        &world,
        &scope,
        "operation-push-retry",
        "plan-push-retry",
        current.head.latest.generation,
        current.head.latest.commit_id,
        current.head.latest.manifest_sha256,
        &forked_id,
        &forked_id,
        retry_state.revision,
        Some(retry_state.bindings[0].revision),
        &local_transcript,
        &lineage,
        &semantic,
    );
    let engine = world.engine();
    let mut fail_after_cas = FailOnce::new(ConversationFailpoint::AfterCloudHeadCas);
    assert!(apply_conversation_cloud_publication(
        &world.repo_a,
        retry_operation,
        || engine.publish(retry_request).map(cloud_publication),
        || Ok(None),
        &mut fail_after_cas,
    )
    .is_err());
    let published_head = world.current_head().unwrap();
    let published_snapshot = world.engine().inspect(&world.bundle_id).unwrap();
    assert!(published_snapshot
        .manifest
        .conversations
        .contains_key(&forked_id));
    let discovered_publication = ConversationCloudPublication {
        generation: published_snapshot.head.latest.generation,
        commit_id: published_snapshot.head.latest.commit_id,
        manifest_sha256: published_snapshot.head.latest.manifest_sha256,
    };
    let recovered = recover_conversation_cloud_publications(&world.repo_a, |_| {
        Ok(Some(discovered_publication.clone()))
    })
    .unwrap();
    assert_eq!(recovered.completed.len(), 1);
    assert!(
        recover_conversation_cloud_publications(&world.repo_a, |_| Ok(None))
            .unwrap()
            .completed
            .is_empty()
    );
    assert_eq!(world.current_head().unwrap(), published_head);
    let final_manifest = world.engine().inspect(&world.bundle_id).unwrap().manifest;
    assert_eq!(final_manifest.conversations.len(), 2);
    assert!(final_manifest.conversations.contains_key(&cloud_id));
    assert!(final_manifest.conversations.contains_key(&forked_id));
    assert_eq!(
        final_manifest.conversations[&forked_id].forked_from,
        Some(cloud_id.clone()),
        "fork provenance survives publication",
    );
    let final_bindings = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(final_bindings.bindings.len(), 1);
    assert_eq!(final_bindings.bindings[0].thread_id, forked_id);
    assert_eq!(
        final_bindings.bindings[0].source_generation,
        Some(published_snapshot.head.latest.generation)
    );
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
    repository: V3Repository,
    operation: ConversationSyncOperation,
    targets: Vec<ConversationTargetPayload>,
}

pub(super) struct InstalledScope {
    pub(super) host_id: crate::project_sync_v3::domain::HostId,
    pub(super) host_identity_revision: u64,
    pub(super) replica_id: ReplicaId,
    pub(super) project_id: LocalProjectId,
    pub(super) profile_id: LocalProviderProfileId,
    pub(super) project_root: PathBuf,
    pub(super) provider_root: PathBuf,
}

pub(super) fn install_scope(
    repository: &V3Repository,
    bundle_id: &BundleId,
    suffix: &str,
    replica: &str,
) -> InstalledScope {
    install_scope_for_provider(repository, bundle_id, suffix, replica, Provider::Codex)
}

pub(super) fn install_scope_for_provider(
    repository: &V3Repository,
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
    generation: u64,
    commit_id: String,
    manifest_sha256: String,
    thread_id: &ThreadId,
    result_thread_id: &ThreadId,
    bindings_revision: u64,
    binding_revision: Option<u64>,
    transcript: &[u8],
    lineage_digest: &str,
    semantic_digest: &str,
) -> ConversationSyncOperation {
    // A distinct result thread means this push is a fork: the operation
    // rekeys the binding from the planned thread to the minted one.
    let old_thread_id = (thread_id != result_thread_id).then(|| thread_id.clone());
    ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(plan_id).unwrap()),
        direction: ConversationPlanDirection::Push,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation,
        commit_id,
        manifest_sha256,
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: Provider::Codex,
        thread_id: thread_id.clone(),
        old_thread_id,
        rekey_committed: false,
        result_thread_id: result_thread_id.clone(),
        forked_from_provider_thread_id: None,
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
    outcome: crate::project_sync_v3::bundle_engine::PublishedBundle,
) -> ConversationCloudPublication {
    ConversationCloudPublication {
        generation: outcome.snapshot.head.latest.generation,
        commit_id: outcome.snapshot.head.latest.commit_id,
        manifest_sha256: outcome.snapshot.head.latest.manifest_sha256,
    }
}

impl MaterializationFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home_unresolved = temp.path().join("home");
        fs::create_dir(&home_unresolved).unwrap();
        let home = fs::canonicalize(&home_unresolved).unwrap();
        let repository = V3Repository::from_home_dir(&home).unwrap();
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
        let index = format!("{{\"id\":\"{thread_id}\",\"title\":\"Fixture\"}}\n").into_bytes();
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
            direction: ConversationPlanDirection::Pull,
            phase: ConversationSyncPhase::Prepared,
            storage_id: crate::project_sync_v3::domain::StorageId::parse("storage-materialize")
                .unwrap(),
            bundle_id,
            generation: 7,
            commit_id: "commit-materialize".to_string(),
            manifest_sha256: "a".repeat(64),
            host_id: identity.host_id,
            replica_id,
            local_project_id,
            provider_profile_id: profile_id,
            provider: Provider::Codex,
            thread_id: ThreadId::parse("thread-materialize").unwrap(),
            old_thread_id: None,
            rekey_committed: false,
            result_thread_id: ThreadId::parse("thread-materialize").unwrap(),
            forked_from_provider_thread_id: None,
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
    repository: &V3Repository,
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
