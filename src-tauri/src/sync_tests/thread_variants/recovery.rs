use super::harness::{codex, sha256, TranscriptBuilder, VariantWorld};
use super::relation_matrix::reviewed_action;
use crate::project_sync::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage,
};
use crate::project_sync::conversation_materialization::{
    apply_conversation_cloud_publication, apply_conversation_materialization,
    recover_conversation_cloud_publications, recover_conversation_materializations,
    ConversationCloudPublication, ConversationFailpoint, ConversationFailpoints,
    ConversationMaterializationRequest, ConversationTargetPayload, NoConversationFailpoints,
};
use crate::project_sync::conversation_sync::{
    validate_conversation_action_pins, ConversationStalePin,
};
use crate::project_sync::domain::{
    compute_portable_conversation_tail, BindingState, BundleId, BundleRecipe,
    ConversationCloudAnchor, ConversationFileRole, ConversationJournalTarget,
    ConversationMaterializationOperation, ConversationMaterializationState,
    ConversationOperationId, ConversationPlanActionType, ConversationPlanDirection,
    ConversationPublicationOperation, ConversationPushPlan, ConversationReasonCode,
    ConversationSyncOperation, ConversationSyncOperationKind, LocalConversationBinding,
    LocalProjectId, LocalProjectRegistration, LocalProviderProfileId, PlanId,
    PortableConversationTail, ProjectBinding, ProviderKey, ProviderProfile, PullJournalPhase,
    PushJournalPhase, ReplicaId, RootTailLayout, StorageId, SyncConfig, SyncCurrency,
    TailClassification, TailPartitionKind, TargetExpectation, ThreadId, PORTABLE_TAIL_CODEC_V1,
    PUSH_PLAN_SCHEMA,
};
use crate::project_sync::persistence::LocalStateRepository;
use crate::project_sync::provider_capture::{capture_selected, discover_project, CaptureRequest};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[test]
fn every_reviewed_pin_rejects_independently_before_mutation() {
    let (action, baseline) = reviewed_action(
        ConversationPlanDirection::Pull,
        ConversationPlanActionType::MaterializeCloud,
        ConversationReasonCode::CloudOutstanding,
        TailClassification::CloudOutstanding,
    );
    type StaleCase = (
        ConversationStalePin,
        Box<dyn Fn(&mut crate::project_sync::conversation_sync::ConversationApplyContext)>,
    );
    let cases: Vec<StaleCase> = vec![
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
            ConversationStalePin::ScopeRevision,
            Box::new(|value| value.scope_revision += 1),
        ),
        (
            ConversationStalePin::ScopeDigest,
            Box::new(|value| value.scope_sha256 = "e".repeat(64)),
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
            Box::new(|value| value.codec = "codex.lineage.unsupported".to_string()),
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
            ConversationPlanDirection::Push,
            ConversationPlanActionType::NoChanges,
            TailClassification::Sync(SyncCurrency::Current),
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::RequirePullMetadata,
            TailClassification::Sync(SyncCurrency::Advanced),
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::Publish,
            TailClassification::LocalOutstanding,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::RequirePull,
            TailClassification::CloudOutstanding,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::CreateCloud,
            TailClassification::MissingCloud,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::NotPushable,
            TailClassification::MissingLocal,
        ),
        (
            ConversationPlanDirection::Push,
            ConversationPlanActionType::Blocked,
            TailClassification::Unknown,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::AdoptCloudMetadata,
            TailClassification::Sync(SyncCurrency::Current),
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::ReportPushEligible,
            TailClassification::LocalOutstanding,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::MaterializeCloud,
            TailClassification::CloudOutstanding,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::KeepBoth,
            TailClassification::Divergent,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::CreateLocal,
            TailClassification::MissingLocal,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::ReportLocalOnly,
            TailClassification::MissingCloud,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::NotFound,
            TailClassification::MissingBoth,
        ),
        (
            ConversationPlanDirection::Pull,
            ConversationPlanActionType::Blocked,
            TailClassification::Unknown,
        ),
    ];

    for (direction, action_type, relation) in actions {
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
        ConversationPlanActionType::Publish,
        ConversationReasonCode::LocalOutstanding,
        TailClassification::LocalOutstanding,
    );
    let plan = ConversationPushPlan {
        schema_version: PUSH_PLAN_SCHEMA,
        plan_id: PlanId::parse("plan-reviewed-push").unwrap(),
        storage_id: StorageId::parse("storage-reviewed-push").unwrap(),
        bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        host_id: action.host_id.clone(),
        replica_id: action.replica_id.clone(),
        local_project_id: action.local_project_id.clone(),
        expected_host_identity_revision: action.expected_host_identity_revision,
        expected_project_binding_revision: action.expected_project_binding_revision,
        expected_bindings_document_revision: action.expected_bindings_document_revision,
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
        cloud: ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision: 1,
            commit_id: "commit-a".to_string(),
            manifest_sha256: "a".repeat(64),
        },
        host_id: identity.host_id,
        replica_id: crate::project_sync::domain::ReplicaId::parse("replica-a").unwrap(),
        local_project_id: crate::project_sync::domain::LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: crate::project_sync::domain::LocalProviderProfileId::parse(
            "profile-a",
        )
        .unwrap(),
        provider: codex(),
        thread_id: ThreadId::parse("thread-journal").unwrap(),
        operation: canned_materialization(TailClassification::MissingLocal),
        provider_thread_id: Some("thread-a".to_string()),
        materialization_state: ConversationMaterializationState::Pending,
        expected_host_identity_revision: 0,
        expected_bindings_document_revision: 0,
        expected_binding_revision: None,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
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
        cloud: ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision: 2,
            commit_id: "commit-a".to_string(),
            manifest_sha256: "a".repeat(64),
        },
        host_id: identity.host_id,
        replica_id: crate::project_sync::domain::ReplicaId::parse("replica-a").unwrap(),
        local_project_id: crate::project_sync::domain::LocalProjectId::parse("project-a").unwrap(),
        provider_profile_id: crate::project_sync::domain::LocalProviderProfileId::parse(
            "profile-a",
        )
        .unwrap(),
        provider: codex(),
        thread_id: ThreadId::parse("thread-journal").unwrap(),
        operation: canned_publication(),
        provider_thread_id: None,
        materialization_state: ConversationMaterializationState::StoredOnly,
        expected_host_identity_revision: 0,
        expected_bindings_document_revision: 0,
        expected_binding_revision: None,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
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
}

#[test]
fn every_local_commit_checkpoint_recovers_to_one_valid_fixed_point() {
    let checkpoints = vec![
        (ConversationFailpoint::BeforeJournal, false),
        (ConversationFailpoint::AfterJournal, false),
        (
            ConversationFailpoint::BeforeStage {
                index: 0,
                role: ConversationFileRole::Transcript,
            },
            true,
        ),
        (
            ConversationFailpoint::AfterStage {
                index: 0,
                role: ConversationFileRole::Transcript,
            },
            true,
        ),
        (
            ConversationFailpoint::BeforeStage {
                index: 1,
                role: ConversationFileRole::ProviderIndexFragment,
            },
            true,
        ),
        (
            ConversationFailpoint::AfterStage {
                index: 1,
                role: ConversationFileRole::ProviderIndexFragment,
            },
            true,
        ),
        (
            ConversationFailpoint::BeforeStage {
                index: 2,
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: "thread-state".to_string(),
                },
            },
            true,
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
        } else if checkpoint == ConversationFailpoint::AfterJournal {
            assert_eq!(first.rolled_back.len(), 1);
        }
    }
}

#[test]
fn every_cloud_publication_checkpoint_recovers_to_one_valid_fixed_point() {
    let checkpoints = [
        (ConversationFailpoint::BeforeJournal, false),
        (ConversationFailpoint::AfterJournal, false),
        (ConversationFailpoint::BeforeCloudPublication, false),
        (ConversationFailpoint::AfterCloudHeadCas, true),
        (ConversationFailpoint::BeforeHeadAdvanced, true),
        (ConversationFailpoint::AfterHeadAdvanced, true),
        (ConversationFailpoint::BeforeConfigCommit, true),
        (ConversationFailpoint::AfterConfigCommit, true),
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
        let initial = world.publish(std::slice::from_ref(&initial_thread), origin.clone(), 10);
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
        let (_initial_lineage, _initial_semantic) = match initial_analysis.lineage {
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
                    root_tail_layout: Some(RootTailLayout {
                        codec: PORTABLE_TAIL_CODEC_V1.to_string(),
                        partition_kind: TailPartitionKind::WholeThread,
                        start_record_count: 0,
                    }),
                    provider_thread_id: Some("source-thread".to_string()),
                    materialization_state: ConversationMaterializationState::Resumable,
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
        let mut operation = push_operation(
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
        if let ConversationSyncOperationKind::CloudPublication(payload) = &mut operation.operation {
            let config = world.repo_a.load_config().unwrap();
            payload.sync_config_before = config.clone();
            let mut config_after = config;
            config_after.revision = config_after.revision.saturating_add(1);
            payload.sync_config_after = config_after;
            let bindings = world.repo_a.load_conversation_bindings().unwrap();
            payload.bindings_before = bindings.clone();
            let mut bindings_after = bindings;
            bindings_after.revision = bindings_after.revision.saturating_add(1);
            payload.bindings_after = bindings_after;
        }
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
                manifest_size: snapshot.head.manifest_size,
                scope_pins: BTreeMap::new(),
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
            // The receipt records the accepted tail the publication
            // committed; observation state now lives in destination
            // checkpoints, never on the binding.
            assert_eq!(final_receipts.records.len(), 1, "{checkpoint:?}");
            assert_eq!(final_receipts.records[0].receipts.len(), 1);
            let receipt = final_receipts.records[0].receipts[0]
                .conversation
                .as_ref()
                .unwrap();
            assert!(receipt.accepted_tail.is_some(), "{checkpoint:?}");
            assert_eq!(
                first.completed.len(),
                usize::from(checkpoint != ConversationFailpoint::AfterCleanup),
            );
        } else {
            assert_eq!(final_snapshot.head.revision, 1, "{checkpoint:?}");
            assert!(final_receipts.records.is_empty());
            if checkpoint != ConversationFailpoint::BeforeJournal {
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
    if let ConversationSyncOperationKind::ProviderMaterialization(payload) =
        &mut fixture.operation.operation
    {
        payload.pinned_lineage_digest = lineage_digest;
        payload.pinned_semantic_digest = semantic_digest;
        payload.accepted_tail_after = match rewritten_analysis.lineage {
            CodecLineage::Complete { ref chain, .. } => {
                compute_portable_conversation_tail(TailPartitionKind::WholeThread, 0, chain)
                    .unwrap()
            }
            _ => unreachable!(),
        };
    }
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

    let registry = crate::project_sync::providers::ProviderRegistry::production();
    let mut capture_request = CaptureRequest::for_project(PathBuf::from(&project_root));
    capture_request
        .provider_homes
        .insert(codex(), provider_root);
    let inventory = discover_project(registry, &capture_request).unwrap();
    let conversation = inventory
        .resources
        .iter()
        .find(|resource| resource.resource_id == "codex:session:destination-thread")
        .unwrap();
    assert!(conversation.blocked_reason.is_none());
    let captured = capture_selected(
        registry,
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
        registry,
        &capture_request,
        &BTreeSet::from(["codex:session:destination-thread".to_string()]),
    )
    .unwrap();
    let recaptured_conversation = recaptured
        .conversations
        .get("codex:session:destination-thread")
        .unwrap();
    {
        let recaptured_chain = recaptured_conversation.lineage.chain().unwrap();
        let captured_chain = captured_conversation.lineage.chain().unwrap();
        assert_eq!(&recaptured_chain[..captured_chain.len()], captured_chain);
        assert!(recaptured_chain.len() > captured_chain.len());
    }
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
    install_scope_for_provider(repository, bundle_id, suffix, replica, codex())
}

pub(super) fn install_scope_for_provider(
    repository: &LocalStateRepository,
    bundle_id: &BundleId,
    suffix: &str,
    replica: &str,
    provider: ProviderKey,
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
                provider: provider.clone(),
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
                profile_ids: BTreeMap::from([(provider.clone(), profile_id.clone())]),
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
    _lineage_digest: &str,
    _semantic_digest: &str,
) -> ConversationSyncOperation {
    let tail_after = match analyze_codex_transcript(transcript).lineage {
        CodecLineage::Complete { chain, .. } => {
            compute_portable_conversation_tail(TailPartitionKind::WholeThread, 0, &chain).unwrap()
        }
        other => panic!("push fixture transcript is not resumable: {other:?}"),
    };
    ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(plan_id).unwrap()),
        plan_action_id: None,
        cloud: ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision,
            commit_id: commit_id.clone(),
            manifest_sha256: manifest_sha256.clone(),
        },
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: codex(),
        thread_id: thread_id.clone(),
        operation: ConversationSyncOperationKind::CloudPublication(
            ConversationPublicationOperation {
                phase: PushJournalPhase::CandidateUploaded,
                candidate_commit_id: commit_id,
                candidate_manifest_sha256: manifest_sha256,
                artifact_sha256: sha256(transcript),
                tail_before: None,
                tail_after,
                planned_relation: TailClassification::MissingCloud,
                sync_config_before: SyncConfig::default(),
                sync_config_after: SyncConfig::default(),
                bindings_before: crate::project_sync::domain::ConversationBindings::empty(
                    scope.host_id.clone(),
                ),
                bindings_after: crate::project_sync::domain::ConversationBindings::empty(
                    scope.host_id.clone(),
                ),
            },
        ),
        provider_thread_id: Some("source-thread".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings_revision,
        expected_binding_revision: binding_revision,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
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
        manifest_size: outcome.snapshot.head.manifest_size,
        scope_pins: BTreeMap::new(),
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
            revision: 0,
            created_at: 1,
            updated_at: 1,
        });
        repository.save_config(config).unwrap();
        repository
            .mutate_bindings(|_, machine| {
                machine.profiles.push(ProviderProfile {
                    profile_id: profile_id.clone(),
                    provider: codex(),
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
                    profile_ids: BTreeMap::from([(codex(), profile_id.clone())]),
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
                bytes: transcript.clone(),
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
        let accepted_tail_after = match analyze_codex_transcript(&transcript).lineage {
            CodecLineage::Complete { ref chain, .. } => {
                compute_portable_conversation_tail(TailPartitionKind::WholeThread, 0, chain)
                    .unwrap()
            }
            ref other => panic!("materialize fixture is not resumable: {other:?}"),
        };
        let operation = ConversationSyncOperation {
            operation_id: ConversationOperationId::parse("operation-materialize").unwrap(),
            plan_id: Some(PlanId::parse("plan-materialize").unwrap()),
            plan_action_id: None,
            cloud: ConversationCloudAnchor {
                storage_id: crate::project_sync::domain::StorageId::parse("storage-materialize")
                    .unwrap(),
                bundle_id,
                revision: 7,
                commit_id: "commit-materialize".to_string(),
                manifest_sha256: "a".repeat(64),
            },
            host_id: identity.host_id.clone(),
            replica_id,
            local_project_id,
            provider_profile_id: profile_id,
            provider: codex(),
            thread_id: ThreadId::parse("thread-materialize").unwrap(),
            operation: ConversationSyncOperationKind::ProviderMaterialization(
                ConversationMaterializationOperation {
                    phase: PullJournalPhase::TargetStaged,
                    planned_relation: TailClassification::MissingLocal,
                    accepted_tail_after,
                    pinned_lineage_digest: lineage_digest,
                    pinned_semantic_digest: semantic_digest,
                    keep_both: None,
                    sync_config_before: SyncConfig::default(),
                    sync_config_after: SyncConfig::default(),
                    bindings_before: crate::project_sync::domain::ConversationBindings::empty(
                        identity.host_id.clone(),
                    ),
                    bindings_after: crate::project_sync::domain::ConversationBindings::empty(
                        identity.host_id,
                    ),
                },
            ),
            provider_thread_id: Some(thread_id.to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            expected_host_identity_revision: identity.revision,
            expected_bindings_document_revision: 0,
            expected_binding_revision: None,
            expected_profile_revision: 0,
            expected_project_binding_revision: 0,
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
        let mut operation = self.operation.clone();
        if let ConversationSyncOperationKind::ProviderMaterialization(payload) =
            &mut operation.operation
        {
            let config = self.repository.load_config().unwrap();
            payload.sync_config_before = config.clone();
            let mut config_after = config;
            config_after.revision = config_after.revision.saturating_add(1);
            payload.sync_config_after = config_after;
            let bindings = self.repository.load_conversation_bindings().unwrap();
            payload.bindings_before = bindings.clone();
            let mut bindings_after = bindings;
            // The after-state the materialization commits: the destination
            // binding row under the imported whole-thread layout.
            bindings_after.bindings.push(LocalConversationBinding {
                host_id: operation.host_id.clone(),
                replica_id: operation.replica_id.clone(),
                local_project_id: operation.local_project_id.clone(),
                provider_profile_id: operation.provider_profile_id.clone(),
                thread_id: operation.thread_id.clone(),
                forked_from: None,
                root_tail_layout: Some(RootTailLayout {
                    codec: PORTABLE_TAIL_CODEC_V1.to_string(),
                    partition_kind: TailPartitionKind::WholeThread,
                    start_record_count: 0,
                }),
                provider_thread_id: operation.provider_thread_id.clone(),
                materialization_state: ConversationMaterializationState::Resumable,
                revision: 0,
                created_at: operation.created_at,
                updated_at: operation.updated_at,
            });
            bindings_after.revision = bindings_after.revision.saturating_add(1);
            payload.bindings_after = bindings_after;
        }
        ConversationMaterializationRequest {
            operation,
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

fn canned_tail(end: u64) -> PortableConversationTail {
    PortableConversationTail {
        codec: PORTABLE_TAIL_CODEC_V1.to_string(),
        partition_kind: TailPartitionKind::WholeThread,
        start_record_count: 0,
        accepted_end_record_count: end,
        accepted_tail_state: "e".repeat(64),
    }
}

fn canned_publication() -> ConversationSyncOperationKind {
    ConversationSyncOperationKind::CloudPublication(ConversationPublicationOperation {
        phase: PushJournalPhase::CandidateUploaded,
        candidate_commit_id: "commit-candidate".to_string(),
        candidate_manifest_sha256: "f".repeat(64),
        artifact_sha256: "a".repeat(64),
        tail_before: None,
        tail_after: canned_tail(1),
        planned_relation: TailClassification::MissingCloud,
        sync_config_before: SyncConfig::default(),
        sync_config_after: SyncConfig::default(),
        bindings_before: crate::project_sync::domain::ConversationBindings::empty(
            crate::project_sync::domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        ),
        bindings_after: crate::project_sync::domain::ConversationBindings::empty(
            crate::project_sync::domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        ),
    })
}

fn canned_materialization(relation: TailClassification) -> ConversationSyncOperationKind {
    ConversationSyncOperationKind::ProviderMaterialization(ConversationMaterializationOperation {
        phase: PullJournalPhase::TargetStaged,
        planned_relation: relation,
        accepted_tail_after: canned_tail(1),
        pinned_lineage_digest: "b".repeat(64),
        pinned_semantic_digest: "c".repeat(64),
        keep_both: None,
        sync_config_before: SyncConfig::default(),
        sync_config_after: SyncConfig::default(),
        bindings_before: crate::project_sync::domain::ConversationBindings::empty(
            crate::project_sync::domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        ),
        bindings_after: crate::project_sync::domain::ConversationBindings::empty(
            crate::project_sync::domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
        ),
    })
}
