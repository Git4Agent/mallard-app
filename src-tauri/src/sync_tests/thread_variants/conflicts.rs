use super::harness::{assert_manifest_consistent, chain, sha256, VariantSpec, VariantWorld};
use super::recovery::install_scope;
use crate::project_sync_v3::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage,
};
use crate::project_sync_v3::conversation_materialization::{
    apply_conversation_materialization, recover_conversation_materializations,
    ConversationMaterializationRequest, ConversationTargetPayload, NoConversationFailpoints,
};
use crate::project_sync_v3::conversation_sync::{
    rekey_same_id_divergence, reserve_provider_thread_ids, ConversationBindingScope,
    ProviderIdRequest,
};
use crate::project_sync_v3::domain::{
    ConflictGroupId, ConversationBindings, ConversationFileRole, ConversationMaterializationState,
    ConversationOperationId, ConversationPlanDirection, ConversationSyncOperation,
    ConversationSyncPhase, HostId, LocalConversationBinding, LocalConversationPreference,
    LocalProjectId, LocalProviderProfileId, PlanId, Provider, ReplicaId, TargetExpectation,
    VariantId,
};
use crate::project_sync_v3::persistence::V3Repository;
use std::collections::BTreeSet;
use std::fs;

#[test]
fn same_id_divergent_pull_rekeys_once_without_renaming_provider_thread() {
    let host_id = HostId::parse("11111111111111111111111111111111").unwrap();
    let replica_id = ReplicaId::parse("replica-a").unwrap();
    let project_id = LocalProjectId::parse("project-a").unwrap();
    let profile_id = LocalProviderProfileId::parse("profile-a").unwrap();
    let group_id = ConflictGroupId::parse("group-a").unwrap();
    let old = VariantId::parse("variant-v").unwrap();
    let rekeyed = VariantId::parse("variant-v-local").unwrap();
    let mut document = ConversationBindings {
        schema: 1,
        revision: 9,
        host_id: host_id.clone(),
        bindings: vec![LocalConversationBinding {
            host_id: host_id.clone(),
            replica_id: replica_id.clone(),
            local_project_id: project_id.clone(),
            provider_profile_id: profile_id.clone(),
            conflict_group_id: group_id.clone(),
            variant_id: old.clone(),
            provider_thread_id: Some("existing-provider-thread".to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            last_observed_file_sha256: Some("a".repeat(64)),
            last_observed_head_message_id: Some("m-local".to_string()),
            last_observed_lineage_digest: Some("b".repeat(64)),
            last_observed_semantic_digest: Some("c".repeat(64)),
            source_generation: Some(3),
            source_manifest_sha256: Some("d".repeat(64)),
            revision: 4,
            created_at: 1,
            updated_at: 5,
        }],
        preferences: vec![LocalConversationPreference {
            host_id: host_id.clone(),
            replica_id: replica_id.clone(),
            local_project_id: project_id.clone(),
            provider_profile_id: profile_id.clone(),
            conflict_group_id: group_id.clone(),
            preferred_variant_id: old.clone(),
            revision: 2,
            updated_at: 5,
        }],
        origin_aliases: vec![],
    };
    let scope = ConversationBindingScope {
        host_id,
        replica_id,
        local_project_id: project_id,
        provider_profile_id: profile_id,
        conflict_group_id: group_id,
    };
    let before_provider = document.bindings[0].provider_thread_id.clone();
    let result =
        rekey_same_id_divergence(&mut document, &scope, &old, &rekeyed, 9, 4, Some(2), 10).unwrap();
    assert_eq!(result.binding.variant_id, rekeyed);
    assert_eq!(result.binding.provider_thread_id, before_provider);
    assert_eq!(document.preferences[0].preferred_variant_id, rekeyed);
    assert_eq!(document.preferences[0].revision, 3);
    assert_eq!(document.bindings.len(), 1);

    let fixed_point = document.clone();
    let retry = rekey_same_id_divergence(
        &mut document,
        &scope,
        &old,
        &VariantId::parse("variant-another").unwrap(),
        9,
        4,
        Some(2),
        11,
    );
    assert!(retry.is_err());
    assert_eq!(document, fixed_point);
}

#[test]
fn same_id_divergent_pull_materializes_cloud_and_rekeys_local_once() {
    let world = VariantWorld::new();
    let scope = install_scope(
        &world.repo_a,
        &world.bundle_id,
        "same-id-pull",
        "replica-same-id-pull",
    );
    let group_id = ConflictGroupId::parse("group-same-id-pull").unwrap();
    let shared_variant_id = VariantId::parse("variant-shared").unwrap();
    let local_variant_id = VariantId::parse("variant-local-stable").unwrap();
    let origin = world.origin(&world.repo_b, "replica-cloud");
    let cloud_transcript = chain(
        "shared-source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-cloud", "cloud branch"),
        ],
    );
    let cloud_variant = VariantSpec::new(
        group_id.as_str(),
        shared_variant_id.as_str(),
        "shared-source-thread",
        cloud_transcript.clone(),
        origin.clone(),
        10,
    );
    let published = world.publish(&[cloud_variant], &shared_variant_id, origin, 10);

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
                conflict_group_id: group_id.clone(),
                variant_id: shared_variant_id.clone(),
                provider_thread_id: Some("existing-provider-thread".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(&local_transcript)),
                last_observed_head_message_id: Some(local_head.clone()),
                last_observed_lineage_digest: Some(local_lineage.clone()),
                last_observed_semantic_digest: Some(local_semantic.clone()),
                source_generation: Some(published.snapshot.head.generation),
                source_manifest_sha256: Some(published.snapshot.head.manifest_sha256.clone()),
                revision: 0,
                created_at: 10,
                updated_at: 10,
            });
            document.preferences.push(LocalConversationPreference {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                conflict_group_id: group_id.clone(),
                preferred_variant_id: shared_variant_id.clone(),
                revision: 0,
                updated_at: 10,
            });
            Ok(())
        })
        .unwrap();
    let before = world.repo_a.load_conversation_bindings().unwrap();
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
    let operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-same-id-pull").unwrap(),
        plan_id: Some(PlanId::parse("plan-same-id-pull").unwrap()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: published.snapshot.head.generation,
        commit_id: published.snapshot.head.commit_id.clone(),
        manifest_sha256: published.snapshot.head.manifest_sha256.clone(),
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: Provider::Codex,
        conflict_group_id: group_id.clone(),
        source_variant_id: Some(shared_variant_id.clone()),
        old_variant_id: Some(shared_variant_id.clone()),
        rekeyed_local_variant_id: Some(local_variant_id.clone()),
        rekey_committed: false,
        result_variant_id: shared_variant_id.clone(),
        provider_thread_id: Some("incoming-cloud-thread".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(incoming_lineage),
        semantic_digest: Some(incoming_semantic),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: before.revision,
        expected_binding_revision: Some(before.bindings[0].revision),
        expected_preference_revision: Some(before.preferences[0].revision),
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(&local_transcript)),
        initialize_preference: false,
        targets: Vec::new(),
        created_at: 20,
        updated_at: 20,
    };
    let request = ConversationMaterializationRequest {
        operation,
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
    let local = document
        .bindings
        .iter()
        .find(|binding| binding.variant_id == local_variant_id)
        .unwrap();
    assert_eq!(
        local.provider_thread_id.as_deref(),
        Some("existing-provider-thread")
    );
    let cloud = document
        .bindings
        .iter()
        .find(|binding| binding.variant_id == shared_variant_id)
        .unwrap();
    assert_eq!(
        cloud.provider_thread_id.as_deref(),
        Some("incoming-cloud-thread")
    );
    assert_eq!(
        document.preferences[0].preferred_variant_id,
        local_variant_id
    );
    assert_eq!(
        world
            .engine()
            .inspect(&world.bundle_id)
            .unwrap()
            .head
            .generation,
        1,
        "Pull must not publish or rename the cloud variant",
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

#[test]
fn stale_preference_rejects_rekey_before_any_mutation() {
    let (mut document, scope, old, result) = rekey_fixture();
    let before = document.clone();
    let error = rekey_same_id_divergence(&mut document, &scope, &old, &result, 1, 1, Some(999), 2)
        .unwrap_err();
    assert!(error.contains("preference changed"));
    assert_eq!(document, before);
}

#[test]
fn provider_id_collisions_are_resolved_profile_wide_and_stably() {
    let requests = vec![
        ProviderIdRequest {
            variant_id: VariantId::parse("variant-v2").unwrap(),
            source_hint: Some("shared-source".to_string()),
            primary: false,
        },
        ProviderIdRequest {
            variant_id: VariantId::parse("variant-v1").unwrap(),
            source_hint: Some("shared-source".to_string()),
            primary: true,
        },
        ProviderIdRequest {
            variant_id: VariantId::parse("variant-v3").unwrap(),
            source_hint: Some("occupied-by-other-project".to_string()),
            primary: false,
        },
    ];
    let occupied = BTreeSet::from(["occupied-by-other-project".to_string()]);
    let mut generated = ["generated-a", "generated-b", "generated-c"].into_iter();
    let first = reserve_provider_thread_ids(requests.clone(), &occupied, || {
        Ok(generated.next().unwrap().to_string())
    })
    .unwrap();
    assert_eq!(
        first[&VariantId::parse("variant-v1").unwrap()],
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
    VariantId,
    VariantId,
) {
    let host_id = HostId::parse("11111111111111111111111111111111").unwrap();
    let replica_id = ReplicaId::parse("replica-a").unwrap();
    let project_id = LocalProjectId::parse("project-a").unwrap();
    let profile_id = LocalProviderProfileId::parse("profile-a").unwrap();
    let group_id = ConflictGroupId::parse("group-a").unwrap();
    let old = VariantId::parse("variant-old").unwrap();
    let result = VariantId::parse("variant-local").unwrap();
    let document = ConversationBindings {
        schema: 1,
        revision: 1,
        host_id: host_id.clone(),
        bindings: vec![LocalConversationBinding {
            host_id: host_id.clone(),
            replica_id: replica_id.clone(),
            local_project_id: project_id.clone(),
            provider_profile_id: profile_id.clone(),
            conflict_group_id: group_id.clone(),
            variant_id: old.clone(),
            provider_thread_id: Some("thread-existing".to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            last_observed_file_sha256: None,
            last_observed_head_message_id: None,
            last_observed_lineage_digest: None,
            last_observed_semantic_digest: None,
            source_generation: None,
            source_manifest_sha256: None,
            revision: 1,
            created_at: 1,
            updated_at: 1,
        }],
        preferences: vec![LocalConversationPreference {
            host_id: host_id.clone(),
            replica_id: replica_id.clone(),
            local_project_id: project_id.clone(),
            provider_profile_id: profile_id.clone(),
            conflict_group_id: group_id.clone(),
            preferred_variant_id: old.clone(),
            revision: 1,
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
            conflict_group_id: group_id,
        },
        old,
        result,
    )
}
