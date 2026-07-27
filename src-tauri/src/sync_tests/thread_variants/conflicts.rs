use super::harness::{assert_manifest_consistent, chain, sha256, ThreadSpec, VariantWorld};
use super::recovery::install_scope;
use crate::project_sync_v3::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage,
};
use crate::project_sync_v3::conversation_materialization::{
    apply_conversation_materialization, recover_conversation_materializations,
    ConversationMaterializationRequest, ConversationTargetPayload, NoConversationFailpoints,
};
use crate::project_sync_v3::conversation_sync::{
    default_conversation_decision, rekey_thread_binding, reserve_provider_thread_ids,
    ConversationBindingScope, ConversationCapabilities, ProviderIdRequest,
};
use crate::project_sync_v3::domain::{
    classify_conversation_relation, ConversationBindings, ConversationFileRole,
    ConversationMaterializationState, ConversationOperationId, ConversationPlanActionType,
    ConversationPlanDirection, ConversationRelation, ConversationSyncOperation,
    ConversationSyncPhase, HostId, LocalConversationBinding, LocalProjectId,
    LocalProviderProfileId, PlanId, Provider, ReplicaId, TargetExpectation, ThreadId,
};
use crate::project_sync_v3::persistence::V3Repository;
use std::collections::BTreeSet;
use std::fs;

#[test]
fn fork_rekey_moves_the_binding_once_without_renaming_provider_thread() {
    let (mut document, scope, old, minted) = rekey_fixture();
    let before_provider = document.bindings[0].provider_thread_id.clone();
    let result = rekey_thread_binding(&mut document, &scope, &old, &minted, 1, 1, 2).unwrap();
    assert_eq!(result.thread_id, minted);
    assert_eq!(result.provider_thread_id, before_provider);
    assert_eq!(
        result.source_generation, None,
        "a minted thread has no published counterpart to pin",
    );
    assert_eq!(result.source_manifest_sha256, None);
    assert_eq!(document.bindings.len(), 1);

    let fixed_point = document.clone();
    let retry = rekey_thread_binding(
        &mut document,
        &scope,
        &old,
        &ThreadId::parse("thread-another").unwrap(),
        1,
        1,
        3,
    );
    assert!(retry.is_err(), "the fork source binding no longer exists");
    assert_eq!(document, fixed_point);
}

#[test]
fn stale_binding_revision_rejects_rekey_before_any_mutation() {
    let (mut document, scope, old, minted) = rekey_fixture();
    let before = document.clone();
    let error = rekey_thread_binding(&mut document, &scope, &old, &minted, 1, 999, 2).unwrap_err();
    assert!(error.contains("binding changed"));
    assert_eq!(document, before);
}

#[test]
fn rekey_to_an_already_bound_thread_is_rejected() {
    let (mut document, scope, old, _) = rekey_fixture();
    let occupied = ThreadId::parse("thread-occupied").unwrap();
    let mut second = document.bindings[0].clone();
    second.thread_id = occupied.clone();
    second.provider_thread_id = Some("thread-existing-2".to_string());
    document.bindings.push(second);
    let before = document.clone();
    let error = rekey_thread_binding(&mut document, &scope, &old, &occupied, 1, 1, 2).unwrap_err();
    assert!(error.contains("already bound"));
    assert_eq!(document, before);
}

#[test]
fn fork_pull_rekeys_local_then_materializes_cloud_as_two_journaled_steps() {
    let world = VariantWorld::new();
    let scope = install_scope(
        &world.repo_a,
        &world.bundle_id,
        "fork-pull",
        "replica-fork-pull",
    );
    let cloud_thread_id = ThreadId::parse("thread-shared").unwrap();
    let minted_thread_id = ThreadId::parse("thread-minted-local").unwrap();
    let origin = world.origin(&world.repo_b, "replica-cloud");
    let cloud_transcript = chain(
        "shared-source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-cloud", "cloud branch"),
        ],
    );
    let cloud_spec = ThreadSpec::new(
        cloud_thread_id.as_str(),
        "shared-source-thread",
        cloud_transcript.clone(),
        origin.clone(),
        10,
    );
    let published = world.publish(&[cloud_spec], origin, 10);

    let local_portable = chain(
        "shared-source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-local", "local branch"),
        ],
    );
    let local_transcript = rewrite_codex_transcript(
        &local_portable,
        "shared-source-thread",
        "existing-provider-thread",
        "/tmp/project",
        scope.project_root.to_str().unwrap(),
    )
    .unwrap();
    let incoming_transcript = rewrite_codex_transcript(
        &cloud_transcript,
        "shared-source-thread",
        "incoming-cloud-thread",
        "/tmp/project",
        scope.project_root.to_str().unwrap(),
    )
    .unwrap();
    fs::create_dir_all(scope.provider_root.join("sessions")).unwrap();
    let local_path = scope
        .provider_root
        .join("sessions/existing-provider-thread.jsonl");
    let incoming_path = scope
        .provider_root
        .join("sessions/incoming-cloud-thread.jsonl");
    let index_path = scope.provider_root.join("session_index.jsonl");
    fs::write(&local_path, &local_transcript).unwrap();
    let initial_index = format!(
        "{{\"id\":\"existing-provider-thread\",\"cwd\":{}}}\n",
        serde_json::to_string(scope.project_root.to_str().unwrap()).unwrap(),
    )
    .into_bytes();
    fs::write(&index_path, &initial_index).unwrap();
    let local_analysis = analyze_codex_transcript(&local_transcript);
    let (local_head, local_lineage, local_semantic) = match local_analysis.lineage {
        CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        } => (
            chain.last().unwrap().message_id.to_string(),
            lineage_digest,
            semantic_digest,
        ),
        other => panic!("local divergence fixture is not resumable: {other:?}"),
    };
    world
        .repo_a
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                thread_id: cloud_thread_id.clone(),
                provider_thread_id: Some("existing-provider-thread".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(&local_transcript)),
                last_observed_head_message_id: Some(local_head.clone()),
                last_observed_lineage_digest: Some(local_lineage.clone()),
                last_observed_semantic_digest: Some(local_semantic.clone()),
                source_generation: Some(published.snapshot.head.latest.generation),
                source_manifest_sha256: Some(
                    published.snapshot.head.latest.manifest_sha256.clone(),
                ),
                revision: 0,
                created_at: 10,
                updated_at: 10,
            });
            Ok(())
        })
        .unwrap();
    let before = world.repo_a.load_conversation_bindings().unwrap();

    // Step 1: mint a thread for the divergent local conversation and rekey
    // its binding. No provider file is touched and nothing is published; the
    // conversation now classifies CloudMissing under the minted thread.
    let rekey_operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-fork-pull-rekey").unwrap(),
        plan_id: Some(PlanId::parse("plan-fork-pull").unwrap()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: published.snapshot.head.latest.generation,
        commit_id: published.snapshot.head.latest.commit_id.clone(),
        manifest_sha256: published.snapshot.head.latest.manifest_sha256.clone(),
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: Provider::Codex,
        thread_id: cloud_thread_id.clone(),
        old_thread_id: Some(cloud_thread_id.clone()),
        rekey_committed: false,
        result_thread_id: minted_thread_id.clone(),
        forked_from_provider_thread_id: None,
        provider_thread_id: Some("existing-provider-thread".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(local_lineage.clone()),
        semantic_digest: Some(local_semantic.clone()),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: before.revision,
        expected_binding_revision: Some(before.bindings[0].revision),
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(&local_transcript)),
        targets: Vec::new(),
        created_at: 20,
        updated_at: 20,
    };
    apply_conversation_materialization(
        &world.repo_a,
        ConversationMaterializationRequest {
            operation: rekey_operation,
            targets: Vec::new(),
        },
        &mut NoConversationFailpoints,
    )
    .unwrap();
    let after_rekey = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(after_rekey.bindings.len(), 1);
    assert_eq!(after_rekey.bindings[0].thread_id, minted_thread_id);
    assert_eq!(
        after_rekey.bindings[0].provider_thread_id.as_deref(),
        Some("existing-provider-thread")
    );
    assert_eq!(after_rekey.bindings[0].source_generation, None);
    assert_eq!(fs::read(&local_path).unwrap(), local_transcript);

    // Step 2: materialize the cloud chain as a new local conversation bound
    // to the (now free) cloud thread.
    let incoming_analysis = analyze_codex_transcript(&incoming_transcript);
    let (incoming_lineage, incoming_semantic) = match incoming_analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        other => panic!("incoming divergence fixture is not resumable: {other:?}"),
    };
    let merged_index = format!(
        "{}{{\"id\":\"incoming-cloud-thread\",\"cwd\":{}}}\n",
        String::from_utf8(initial_index.clone()).unwrap(),
        serde_json::to_string(scope.project_root.to_str().unwrap()).unwrap(),
    )
    .into_bytes();
    let materialize_operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-fork-pull-materialize").unwrap(),
        plan_id: Some(PlanId::parse("plan-fork-pull").unwrap()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: published.snapshot.head.latest.generation,
        commit_id: published.snapshot.head.latest.commit_id.clone(),
        manifest_sha256: published.snapshot.head.latest.manifest_sha256.clone(),
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: Provider::Codex,
        thread_id: cloud_thread_id.clone(),
        old_thread_id: None,
        rekey_committed: false,
        result_thread_id: cloud_thread_id.clone(),
        forked_from_provider_thread_id: None,
        provider_thread_id: Some("incoming-cloud-thread".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(incoming_lineage),
        semantic_digest: Some(incoming_semantic),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: after_rekey.revision,
        expected_binding_revision: None,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(&local_transcript)),
        targets: Vec::new(),
        created_at: 21,
        updated_at: 21,
    };
    let request = ConversationMaterializationRequest {
        operation: materialize_operation,
        targets: vec![
            ConversationTargetPayload {
                target_path: incoming_path.clone(),
                role: ConversationFileRole::Transcript,
                expected: TargetExpectation::Absent,
                bytes: incoming_transcript.clone(),
            },
            ConversationTargetPayload {
                target_path: index_path.clone(),
                role: ConversationFileRole::ProviderIndexFragment,
                expected: TargetExpectation::Sha256(sha256(&initial_index)),
                bytes: merged_index,
            },
        ],
    };
    apply_conversation_materialization(
        &world.repo_a,
        request.clone(),
        &mut NoConversationFailpoints,
    )
    .unwrap();

    assert_eq!(fs::read(&local_path).unwrap(), local_transcript);
    assert_eq!(fs::read(&incoming_path).unwrap(), incoming_transcript);
    let document = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(document.bindings.len(), 2);
    let minted = document
        .bindings
        .iter()
        .find(|binding| binding.thread_id == minted_thread_id)
        .unwrap();
    assert_eq!(
        minted.provider_thread_id.as_deref(),
        Some("existing-provider-thread")
    );
    let cloud = document
        .bindings
        .iter()
        .find(|binding| binding.thread_id == cloud_thread_id)
        .unwrap();
    assert_eq!(
        cloud.provider_thread_id.as_deref(),
        Some("incoming-cloud-thread")
    );
    assert_eq!(
        world
            .engine()
            .inspect(&world.bundle_id)
            .unwrap()
            .head
            .latest
            .generation,
        1,
        "Pull must not publish or rename the cloud thread",
    );
    let fixed_point = document.clone();
    assert!(apply_conversation_materialization(
        &world.repo_a,
        request,
        &mut NoConversationFailpoints,
    )
    .is_err());
    assert_eq!(
        world.repo_a.load_conversation_bindings().unwrap(),
        fixed_point
    );

    let reopened = V3Repository::from_home_dir(world.repo_a.root().parent().unwrap()).unwrap();
    let first = recover_conversation_materializations(&reopened).unwrap();
    let second = recover_conversation_materializations(&reopened).unwrap();
    assert!(first.completed.is_empty());
    assert!(second.completed.is_empty());
    assert_eq!(reopened.load_conversation_bindings().unwrap(), fixed_point);
    assert_manifest_consistent(&world);
}

/// Loop freedom: one divergence forks exactly once. After the fork the pair
/// is Identical, so a second sync round proposes no new threads and no
/// publication.
#[test]
fn diverge_once_sync_twice_forks_exactly_once_then_settles_identical() {
    let world = VariantWorld::new();
    let origin_b = world.origin(&world.repo_b, "replica-cloud");
    let origin_a = world.origin(&world.repo_a, "replica-local");
    let cloud_id = ThreadId::parse("thread-origin").unwrap();
    let forked_id = ThreadId::parse("thread-fork-once").unwrap();
    let cloud_transcript = chain(
        "source-thread",
        &[("m-a", "base"), ("m-b", "answer"), ("m-c0", "cloud tail")],
    );
    let local_transcript = chain(
        "source-thread",
        &[("m-a", "base"), ("m-b", "answer"), ("m-c2", "local tail")],
    );
    let published = world.publish(
        &[ThreadSpec::new(
            cloud_id.as_str(),
            "source-thread",
            cloud_transcript.clone(),
            origin_b.clone(),
            10,
        )],
        origin_b,
        10,
    );
    assert_eq!(published.snapshot.head.latest.generation, 1);
    let local_chain = analyze_codex_transcript(&local_transcript)
        .lineage
        .chain()
        .unwrap()
        .to_vec();
    let cloud_chain = analyze_codex_transcript(&cloud_transcript)
        .lineage
        .chain()
        .unwrap()
        .to_vec();

    // Round one: the bound pair diverges, so the default is Fork.
    let relation = classify_conversation_relation(Some(&local_chain), Some(&cloud_chain));
    assert_eq!(relation, ConversationRelation::Diverged);
    let round_one = default_conversation_decision(
        ConversationPlanDirection::Push,
        relation,
        ConversationCapabilities::default(),
    );
    assert_eq!(round_one.action, ConversationPlanActionType::Fork);
    let mut forked_spec = ThreadSpec::new(
        forked_id.as_str(),
        "source-thread",
        local_transcript.clone(),
        origin_a.clone(),
        20,
    );
    forked_spec.forked_from = Some(cloud_id.clone());
    let cloud_spec = ThreadSpec {
        transcript: cloud_transcript,
        ..ThreadSpec::new(
            cloud_id.as_str(),
            "source-thread",
            Vec::new(),
            origin_a.clone(),
            10,
        )
    };
    let after_fork = world.publish(
        &[cloud_spec.clone(), forked_spec.clone()],
        origin_a.clone(),
        20,
    );
    assert_eq!(
        after_fork.snapshot.manifest.conversations.len(),
        2,
        "exactly one fork thread appears after round one",
    );
    assert_eq!(after_fork.snapshot.head.latest.generation, 2);

    // Round two: identity converged — the local chain is Identical to its
    // rekeyed thread, the origin thread is untouched, and a republish of the
    // same reviewed state is a semantic no-op that mints nothing.
    let forked_cloud_chain = analyze_codex_transcript(
        &world.engine().fetch(&world.bundle_id).unwrap().files
            [&after_fork.snapshot.manifest.conversations[&forked_id].files[0].logical_path],
    )
    .lineage
    .chain()
    .unwrap()
    .to_vec();
    assert_eq!(
        classify_conversation_relation(Some(&local_chain), Some(&forked_cloud_chain)),
        ConversationRelation::Identical
    );
    let round_two = default_conversation_decision(
        ConversationPlanDirection::Push,
        ConversationRelation::Identical,
        ConversationCapabilities::default(),
    );
    assert_eq!(round_two.action, ConversationPlanActionType::NoOp);
    assert_eq!(
        classify_conversation_relation(None, Some(&cloud_chain)),
        ConversationRelation::LocalMissing,
        "the origin thread is a cloud-only row on this host",
    );
    let republish = world.publish(&[cloud_spec, forked_spec], origin_a, 30);
    assert!(!republish.published, "round two publishes nothing");
    assert_eq!(republish.snapshot.head.latest.generation, 2);
    assert_eq!(republish.snapshot.manifest.conversations.len(), 2);
    assert_manifest_consistent(&world);
}

#[test]
fn provider_id_collisions_are_resolved_profile_wide_and_stably() {
    let requests = vec![
        ProviderIdRequest {
            thread_id: ThreadId::parse("thread-v2").unwrap(),
            source_hint: Some("shared-source".to_string()),
        },
        ProviderIdRequest {
            thread_id: ThreadId::parse("thread-v1").unwrap(),
            source_hint: Some("shared-source".to_string()),
        },
        ProviderIdRequest {
            thread_id: ThreadId::parse("thread-v3").unwrap(),
            source_hint: Some("occupied-by-other-project".to_string()),
        },
    ];
    let occupied = BTreeSet::from(["occupied-by-other-project".to_string()]);
    let mut generated = ["generated-a", "generated-b", "generated-c"].into_iter();
    let first = reserve_provider_thread_ids(requests.clone(), &occupied, || {
        Ok(generated.next().unwrap().to_string())
    })
    .unwrap();
    assert_eq!(
        first[&ThreadId::parse("thread-v1").unwrap()],
        "shared-source"
    );
    assert_eq!(first.len(), first.values().collect::<BTreeSet<_>>().len());
    assert!(!first.values().any(|id| id == "occupied-by-other-project"));

    // A reviewed plan persists these reservations. Reusing the result is the
    // retry behavior; recalculating against its occupied values must allocate
    // different IDs and is therefore correctly treated as a stale replan.
    let persisted = first.clone();
    assert_eq!(persisted, first);
}

fn rekey_fixture() -> (
    ConversationBindings,
    ConversationBindingScope,
    ThreadId,
    ThreadId,
) {
    let host_id = HostId::parse("11111111111111111111111111111111").unwrap();
    let replica_id = ReplicaId::parse("replica-a").unwrap();
    let project_id = LocalProjectId::parse("project-a").unwrap();
    let profile_id = LocalProviderProfileId::parse("profile-a").unwrap();
    let old = ThreadId::parse("thread-old").unwrap();
    let minted = ThreadId::parse("thread-minted").unwrap();
    let document = ConversationBindings {
        schema: 2,
        revision: 1,
        host_id: host_id.clone(),
        bindings: vec![LocalConversationBinding {
            host_id: host_id.clone(),
            replica_id: replica_id.clone(),
            local_project_id: project_id.clone(),
            provider_profile_id: profile_id.clone(),
            thread_id: old.clone(),
            provider_thread_id: Some("thread-existing".to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            last_observed_file_sha256: None,
            last_observed_head_message_id: None,
            last_observed_lineage_digest: None,
            last_observed_semantic_digest: None,
            source_generation: Some(3),
            source_manifest_sha256: Some("d".repeat(64)),
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }],
        origin_aliases: vec![],
    };
    (
        document,
        ConversationBindingScope {
            host_id,
            replica_id,
            local_project_id: project_id,
            provider_profile_id: profile_id,
        },
        old,
        minted,
    )
}
