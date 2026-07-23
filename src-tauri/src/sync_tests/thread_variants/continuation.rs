use super::harness::{assert_manifest_consistent, chain, sha256, VariantSpec, VariantWorld};
use super::recovery::{install_scope, InstalledScope};
use crate::project_sync_v3::commands::ThreadSyncState;
use crate::project_sync_v3::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage, CODEX_LINEAGE_CODEC_KEY,
};
use crate::project_sync_v3::conversation_materialization::{
    apply_conversation_cloud_publication, apply_conversation_materialization,
    ConversationCloudPublication, ConversationMaterializationRequest, ConversationTargetPayload,
    NoConversationFailpoints,
};
use crate::project_sync_v3::domain::{
    ConflictGroupId, ConversationFileRole, ConversationMaterializationState,
    ConversationOperationId, ConversationPlanActionType, ConversationPlanDirection,
    ConversationRelation, ConversationSyncOperation, ConversationSyncPhase,
    LocalConversationBinding, LocalConversationPreference, PlanId, TargetExpectation, VariantId,
};
use crate::project_sync_v3::persistence::V3Repository;
use crate::project_sync_v3::provider_capture::{
    CapturedConversation, CapturedFile, CapturedResources, Provider as CaptureProvider,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn schema5_true_noop_does_not_publish_or_change_provenance() {
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let variant_id = VariantId::parse("variant-v1").unwrap();
    let variant = VariantSpec::new(
        "group-main",
        variant_id.as_str(),
        "source-thread",
        chain("source-thread", &[("m-a", "one"), ("m-b", "two")]),
        origin_a.clone(),
        10,
    );
    let first = world.publish(&[variant.clone()], &variant_id, origin_a.clone(), 10);
    assert!(first.published);
    let first_head = first.snapshot.head.clone();
    let first_manifest = first.snapshot.manifest.clone();

    let origin_b = world.origin(&world.repo_b, "replica-b");
    let repeated = world.publish(&[variant], &variant_id, origin_b, 20);
    assert!(!repeated.published);
    assert_eq!(repeated.snapshot.head, first_head);
    assert_eq!(repeated.snapshot.manifest, first_manifest);
    assert_eq!(
        repeated.snapshot.manifest.published_by_origin,
        Some(origin_a)
    );
    assert_manifest_consistent(&world);
}

#[test]
fn bound_alternate_push_updates_only_its_variant() {
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let v1 = VariantSpec::new(
        "group-main",
        v1_id.as_str(),
        "source-thread",
        chain(
            "source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "base answer"),
                ("m-c1", "branch one"),
            ],
        ),
        origin_a.clone(),
        10,
    );
    let v2 = VariantSpec::new(
        "group-main",
        v2_id.as_str(),
        "source-thread",
        chain(
            "source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "base answer"),
                ("m-c2", "branch two"),
            ],
        ),
        origin_b.clone(),
        10,
    );
    let first = world.publish(&[v1.clone(), v2.clone()], &v1_id, origin_a, 10);
    let group = ConflictGroupId::parse("group-main").unwrap();
    let v1_path = first.snapshot.manifest.conversation_groups[&group].variants[&v1_id].files[0]
        .logical_path
        .clone();
    let v2_path = first.snapshot.manifest.conversation_groups[&group].variants[&v2_id].files[0]
        .logical_path
        .clone();
    let v1_entry = first.snapshot.manifest.files[&v1_path].clone();
    let v2_entry = first.snapshot.manifest.files[&v2_path].clone();
    let v1_descriptor = first.snapshot.manifest.resources
        [&first.snapshot.manifest.files[&v1_path].resource_id]
        .clone();

    let mut advanced_v2 = v2;
    advanced_v2.transcript = chain(
        "source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "base answer"),
            ("m-c2", "branch two"),
            ("m-d2", "continued on B"),
        ],
    );
    advanced_v2.updated_at = 20;
    advanced_v2.last_updated_by = Some(origin_b.clone());
    let second = world.publish(&[v1, advanced_v2], &v1_id, origin_b, 20);
    assert!(second.published);
    assert_eq!(second.snapshot.head.generation, 2);
    assert_eq!(
        second.snapshot.manifest.conversation_groups[&group].primary_variant_id,
        v1_id
    );
    assert_eq!(second.snapshot.manifest.files[&v1_path], v1_entry);
    assert_eq!(
        second.snapshot.manifest.resources[&v1_entry.resource_id],
        v1_descriptor
    );
    assert_ne!(second.snapshot.manifest.files[&v2_path], v2_entry);
    assert_manifest_consistent(&world);
}

#[test]
fn a_cloud_b_continuation_preserves_each_active_branch() {
    let world = VariantWorld::new();
    let scope_a = install_scope(
        &world.repo_a,
        &world.bundle_id,
        "continuation-a",
        "replica-a",
    );
    let scope_b = install_scope(
        &world.repo_b,
        &world.bundle_id,
        "continuation-b",
        "replica-b",
    );
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    assert_ne!(origin_a.origin_host_token, origin_b.origin_host_token);
    assert_ne!(
        world.repo_a.load_or_create_host_identity().unwrap().host_id,
        world.repo_b.load_or_create_host_identity().unwrap().host_id
    );

    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let mut v1 = VariantSpec::new(
        "group-main",
        v1_id.as_str(),
        "shared-source-thread",
        chain(
            "shared-source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "base answer"),
                ("m-c1", "A branch"),
            ],
        ),
        origin_a.clone(),
        10,
    );
    let mut v2 = VariantSpec::new(
        "group-main",
        v2_id.as_str(),
        "shared-source-thread",
        chain(
            "shared-source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "base answer"),
                ("m-c2", "B branch"),
            ],
        ),
        origin_b.clone(),
        10,
    );
    let initial = world.publish(&[v1.clone(), v2.clone()], &v1_id, origin_a.clone(), 10);
    assert_eq!(initial.snapshot.head.generation, 1);

    let a_active = rewrite_codex_transcript(
        &v1.transcript,
        "shared-source-thread",
        "a-active-thread",
        "/tmp/project",
        scope_a.project_root.to_str().unwrap(),
    )
    .unwrap();
    let b_active = rewrite_codex_transcript(
        &v2.transcript,
        "shared-source-thread",
        "b-active-thread",
        "/tmp/project",
        scope_b.project_root.to_str().unwrap(),
    )
    .unwrap();
    let a_active_path = install_active_branch(
        &world.repo_a,
        &scope_a,
        &ConflictGroupId::parse("group-main").unwrap(),
        &v1_id,
        "a-active-thread",
        &a_active,
        &initial,
        10,
    );
    let b_active_path = install_active_branch(
        &world.repo_b,
        &scope_b,
        &ConflictGroupId::parse("group-main").unwrap(),
        &v2_id,
        "b-active-thread",
        &b_active,
        &initial,
        10,
    );

    let a_after_first_append = append_message(&a_active, "m-d1", "continued on A");
    fs::write(&a_active_path, &a_after_first_append).unwrap();
    v1.transcript = rewrite_codex_transcript(
        &a_after_first_append,
        "a-active-thread",
        "shared-source-thread",
        scope_a.project_root.to_str().unwrap(),
        "/tmp/project",
    )
    .unwrap();
    v1.updated_at = 20;
    v1.last_updated_by = Some(origin_a.clone());
    let after_a = publish_bound_variant(
        &world,
        &world.repo_a,
        &scope_a,
        &[v1.clone(), v2.clone()],
        &v1_id,
        &v1_id,
        "a-active-thread",
        &a_after_first_append,
        origin_a.clone(),
        "operation-continuation-a-first",
        20,
    );
    let group = ConflictGroupId::parse("group-main").unwrap();
    assert_eq!(
        after_a.snapshot.manifest.conversation_groups[&group]
            .variants
            .len(),
        2
    );
    assert_eq!(
        after_a.snapshot.manifest.conversation_groups[&group].primary_variant_id,
        v1_id
    );

    let b_active_before_pull = fs::read(&b_active_path).unwrap();
    let b_incoming_v1 = materialize_variant(
        &world.repo_b,
        &scope_b,
        &world,
        &group,
        &v1_id,
        "shared-source-thread",
        "b-alternate-v1",
        &v1.transcript,
        "operation-pull-b-v1",
        25,
    );
    assert_eq!(fs::read(&b_active_path).unwrap(), b_active_before_pull);
    assert_ne!(fs::read(&b_incoming_v1).unwrap(), v1.transcript);
    assert_eq!(
        digests(&analyze_codex_transcript(&v1.transcript)),
        digests(&analyze_codex_transcript(
            &fs::read(&b_incoming_v1).unwrap()
        )),
    );
    assert_local_branch_state(
        &world.repo_b,
        &scope_b,
        &group,
        &v2_id,
        &[(&v1_id, "b-alternate-v1"), (&v2_id, "b-active-thread")],
    );

    let b_after_append = append_message(&b_active_before_pull, "m-d2", "continued on B");
    fs::write(&b_active_path, &b_after_append).unwrap();
    v2.transcript = rewrite_codex_transcript(
        &b_after_append,
        "b-active-thread",
        "shared-source-thread",
        scope_b.project_root.to_str().unwrap(),
        "/tmp/project",
    )
    .unwrap();
    v2.updated_at = 30;
    v2.last_updated_by = Some(origin_b.clone());
    let after_b = publish_bound_variant(
        &world,
        &world.repo_b,
        &scope_b,
        &[v1.clone(), v2.clone()],
        &v1_id,
        &v2_id,
        "b-active-thread",
        &b_after_append,
        origin_b.clone(),
        "operation-continuation-b",
        30,
    );
    assert_eq!(after_b.snapshot.head.generation, 3);
    assert_eq!(
        after_b.snapshot.manifest.conversation_groups[&group]
            .variants
            .len(),
        2
    );
    assert_eq!(
        after_b.snapshot.manifest.conversation_groups[&group].primary_variant_id,
        v1_id
    );

    let a_active_before_pull = fs::read(&a_active_path).unwrap();
    let a_incoming_v2 = materialize_variant(
        &world.repo_a,
        &scope_a,
        &world,
        &group,
        &v2_id,
        "shared-source-thread",
        "a-alternate-v2",
        &v2.transcript,
        "operation-pull-a-v2",
        35,
    );
    assert_eq!(fs::read(&a_active_path).unwrap(), a_active_before_pull);
    assert_eq!(
        digests(&analyze_codex_transcript(&v2.transcript)),
        digests(&analyze_codex_transcript(
            &fs::read(&a_incoming_v2).unwrap()
        )),
    );
    assert_local_branch_state(
        &world.repo_a,
        &scope_a,
        &group,
        &v1_id,
        &[(&v1_id, "a-active-thread"), (&v2_id, "a-alternate-v2")],
    );

    let a_after_second_append =
        append_message(&a_active_before_pull, "m-e1", "continued again on A");
    fs::write(&a_active_path, &a_after_second_append).unwrap();
    v1.transcript = rewrite_codex_transcript(
        &a_after_second_append,
        "a-active-thread",
        "shared-source-thread",
        scope_a.project_root.to_str().unwrap(),
        "/tmp/project",
    )
    .unwrap();
    v1.updated_at = 40;
    v1.last_updated_by = Some(origin_a.clone());
    let after_a_second = publish_bound_variant(
        &world,
        &world.repo_a,
        &scope_a,
        &[v1.clone(), v2.clone()],
        &v1_id,
        &v1_id,
        "a-active-thread",
        &a_after_second_append,
        origin_a.clone(),
        "operation-continuation-a-second",
        40,
    );
    assert_eq!(after_a_second.snapshot.head.generation, 4);

    let b_preferred_before_refresh = fs::read(&b_active_path).unwrap();
    let refreshed_b_v1 = materialize_variant(
        &world.repo_b,
        &scope_b,
        &world,
        &group,
        &v1_id,
        "shared-source-thread",
        "b-alternate-v1",
        &v1.transcript,
        "operation-pull-b-v1-refresh",
        45,
    );
    assert_eq!(refreshed_b_v1, b_incoming_v1);
    assert_eq!(
        fs::read(&b_active_path).unwrap(),
        b_preferred_before_refresh
    );
    assert_local_branch_state(
        &world.repo_b,
        &scope_b,
        &group,
        &v2_id,
        &[(&v1_id, "b-alternate-v1"), (&v2_id, "b-active-thread")],
    );

    let local_a_before_noop = world.repo_a.load_conversation_bindings().unwrap();
    let local_b_before_noop = world.repo_b.load_conversation_bindings().unwrap();
    let fixed_point = world.publish(&[v1.clone(), v2.clone()], &v1_id, origin_b.clone(), 50);
    assert!(!fixed_point.published);
    assert_eq!(fixed_point.snapshot.head.generation, 4);
    assert_eq!(
        world.repo_a.load_conversation_bindings().unwrap(),
        local_a_before_noop,
    );
    assert_eq!(
        world.repo_b.load_conversation_bindings().unwrap(),
        local_b_before_noop,
    );

    let reopened_a = V3Repository::from_home_dir(world.repo_a.root().parent().unwrap()).unwrap();
    let reopened_b = V3Repository::from_home_dir(world.repo_b.root().parent().unwrap()).unwrap();
    assert!(crate::project_sync_v3::conversation_materialization::recover_conversation_materializations(&reopened_a)
        .unwrap()
        .completed
        .is_empty());
    assert!(crate::project_sync_v3::conversation_materialization::recover_conversation_materializations(&reopened_b)
        .unwrap()
        .completed
        .is_empty());
    assert_local_branch_state(
        &reopened_a,
        &scope_a,
        &group,
        &v1_id,
        &[(&v1_id, "a-active-thread"), (&v2_id, "a-alternate-v2")],
    );
    assert_local_branch_state(
        &reopened_b,
        &scope_b,
        &group,
        &v2_id,
        &[(&v1_id, "b-alternate-v1"), (&v2_id, "b-active-thread")],
    );
    assert_eq!(
        reopened_a
            .resolve_origin_host_label(&world.bundle_id, &origin_a.origin_host_token)
            .unwrap(),
        ("This host".to_string(), true),
    );
    assert!(
        !reopened_a
            .resolve_origin_host_label(&world.bundle_id, &origin_b.origin_host_token)
            .unwrap()
            .1
    );
    assert_manifest_consistent(&world);
}

#[test]
fn schema5_comparison_exposes_distinct_typed_variants_without_guessing_by_title() {
    let world = VariantWorld::new();
    let scope = install_scope(
        &world.repo_fresh,
        &world.bundle_id,
        "comparison",
        "replica-comparison",
    );
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let v1 = VariantSpec::new(
        "group-main",
        v1_id.as_str(),
        "shared-source-thread",
        chain(
            "shared-source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "answer"),
                ("m-c1", "cloud primary"),
            ],
        ),
        origin_a,
        10,
    );
    let v2 = VariantSpec::new(
        "group-main",
        v2_id.as_str(),
        "shared-source-thread",
        chain(
            "shared-source-thread",
            &[("m-a", "base"), ("m-b", "answer"), ("m-c2", "local branch")],
        ),
        origin_b,
        10,
    );
    let published = world.publish(
        &[v1, v2.clone()],
        &v1_id,
        world.origin(&world.repo_a, "replica-a"),
        10,
    );

    let local_transcript = append_message(&v2.transcript, "m-d2", "continued here");
    let local_analysis = analyze_codex_transcript(&local_transcript);
    let (head_message_id, lineage_digest, semantic_digest, record_count) =
        match &local_analysis.lineage {
            CodecLineage::Complete {
                chain,
                lineage_digest,
                semantic_digest,
            } => (
                chain.last().unwrap().message_id.clone(),
                lineage_digest.clone(),
                semantic_digest.clone(),
                local_analysis.record_count,
            ),
            other => panic!("local comparison fixture is not resumable: {other:?}"),
        };
    world
        .repo_fresh
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                conflict_group_id: ConflictGroupId::parse("group-main").unwrap(),
                variant_id: v2_id.clone(),
                provider_thread_id: Some("local-v2".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(&local_transcript)),
                last_observed_head_message_id: Some(head_message_id.to_string()),
                last_observed_lineage_digest: Some(lineage_digest.clone()),
                last_observed_semantic_digest: Some(semantic_digest.clone()),
                source_generation: Some(published.snapshot.head.generation),
                source_manifest_sha256: Some(published.snapshot.head.manifest_sha256.clone()),
                revision: 0,
                created_at: 10,
                updated_at: 20,
            });
            document.preferences.push(LocalConversationPreference {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                conflict_group_id: ConflictGroupId::parse("group-main").unwrap(),
                preferred_variant_id: v2_id.clone(),
                revision: 0,
                updated_at: 20,
            });
            Ok(())
        })
        .unwrap();

    let resource_id = "codex:session:local-v2".to_string();
    let transcript_path = "state/codex/sessions/local-v2.jsonl".to_string();
    let mut captured = CapturedResources::default();
    captured.conversations.insert(
        resource_id.clone(),
        CapturedConversation {
            provider: CaptureProvider::Codex,
            lineage_codec: CODEX_LINEAGE_CODEC_KEY.to_string(),
            source_thread_id: Some("local-v2".to_string()),
            source_cwd: Some(scope.provider_root.to_string_lossy().into_owned()),
            transcript_logical_path: transcript_path.clone(),
            record_count,
            lineage: local_analysis.lineage,
        },
    );
    captured.files.insert(
        transcript_path.clone(),
        CapturedFile {
            logical_path: transcript_path,
            resource_id,
            bytes: local_transcript,
            source_mtime: 20,
            mode: 0o600,
        },
    );
    let machine = world.repo_fresh.load_bindings().unwrap();
    let binding = machine.active_for(&scope.project_id).unwrap();
    let fetched = world.engine().fetch(&world.bundle_id).unwrap();
    let entries = crate::project_sync_v3::commands::schema5_thread_variant_entries(
        &world.repo_fresh,
        &fetched,
        &captured,
        binding,
    )
    .unwrap();

    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["Variant test conversation", "Variant test conversation"]
    );
    let primary = entries
        .iter()
        .find(|entry| entry.variant_id.as_ref() == Some(&v1_id))
        .unwrap();
    assert!(primary.cloud_primary);
    assert!(!primary.local_preferred);
    assert!(!primary.local_present);
    assert_eq!(primary.state, ThreadSyncState::StorageOnly);
    assert_eq!(primary.relation, Some(ConversationRelation::LocalMissing));
    assert_eq!(
        primary.proposed_action,
        Some(ConversationPlanActionType::AddAlternate)
    );
    assert_eq!(
        primary.materialization_state,
        Some(ConversationMaterializationState::StoredOnly)
    );
    assert!(!primary.resumable);
    assert!(primary.provider_thread_id.is_none());
    assert!(primary.origin_label.is_some());

    let preferred = entries
        .iter()
        .find(|entry| entry.variant_id.as_ref() == Some(&v2_id))
        .unwrap();
    assert!(!preferred.cloud_primary);
    assert!(preferred.local_preferred);
    assert!(preferred.local_present);
    assert_eq!(preferred.state, ThreadSyncState::LocalAhead);
    assert_eq!(
        preferred.relation,
        Some(ConversationRelation::CloudPrefixOfLocal)
    );
    assert_eq!(
        preferred.proposed_action,
        Some(ConversationPlanActionType::FastForwardCloud)
    );
    assert_eq!(
        preferred.materialization_state,
        Some(ConversationMaterializationState::Resumable)
    );
    assert!(preferred.resumable);
    assert_eq!(preferred.provider_thread_id.as_deref(), Some("local-v2"));
    assert!(preferred.origin_label.is_some());
    assert_eq!(
        preferred.conflict_group_id.as_ref().unwrap().as_str(),
        "group-main"
    );
    assert_ne!(primary.thread_id, preferred.thread_id);
}

#[test]
fn cloud_primary_local_preference_and_stored_only_state_change_independently() {
    let world = VariantWorld::new();
    let scope = install_scope(
        &world.repo_a,
        &world.bundle_id,
        "primary-preference",
        "replica-primary-preference",
    );
    let origin = world.origin(&world.repo_a, "replica-primary-preference");
    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let v1 = VariantSpec::new(
        "group-main",
        v1_id.as_str(),
        "thread-v1",
        chain("thread-v1", &[("m-a", "one")]),
        origin.clone(),
        10,
    );
    let v2 = VariantSpec::new(
        "group-main",
        v2_id.as_str(),
        "thread-v2",
        chain("thread-v2", &[("m-a", "one"), ("m-b", "alternate")]),
        origin.clone(),
        10,
    );
    let initial = world.publish(&[v1.clone(), v2.clone()], &v1_id, origin.clone(), 10);
    world
        .repo_a
        .mutate_conversation_bindings(|document| {
            for (variant_id, provider_thread_id) in [(&v1_id, "local-v1"), (&v2_id, "local-v2")] {
                document.bindings.push(LocalConversationBinding {
                    host_id: scope.host_id.clone(),
                    replica_id: scope.replica_id.clone(),
                    local_project_id: scope.project_id.clone(),
                    provider_profile_id: scope.profile_id.clone(),
                    conflict_group_id: ConflictGroupId::parse("group-main").unwrap(),
                    variant_id: variant_id.clone(),
                    provider_thread_id: Some(provider_thread_id.to_string()),
                    materialization_state: ConversationMaterializationState::Resumable,
                    last_observed_file_sha256: None,
                    last_observed_head_message_id: None,
                    last_observed_lineage_digest: None,
                    last_observed_semantic_digest: None,
                    source_generation: Some(initial.snapshot.head.generation),
                    source_manifest_sha256: Some(initial.snapshot.head.manifest_sha256.clone()),
                    revision: 0,
                    created_at: 10,
                    updated_at: 10,
                });
            }
            document.preferences.push(LocalConversationPreference {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                conflict_group_id: ConflictGroupId::parse("group-main").unwrap(),
                preferred_variant_id: v2_id.clone(),
                revision: 0,
                updated_at: 10,
            });
            Ok(())
        })
        .unwrap();

    let promoted_cloud = world.publish(&[v1, v2], &v2_id, origin, 20);
    assert!(promoted_cloud.published);
    assert_eq!(
        promoted_cloud.snapshot.manifest.conversation_groups
            [&ConflictGroupId::parse("group-main").unwrap()]
            .primary_variant_id,
        v2_id
    );
    let after_cloud = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(after_cloud.preferences[0].preferred_variant_id, v2_id);
    assert_eq!(
        after_cloud.bindings[0].provider_thread_id.as_deref(),
        Some("local-v1")
    );
    assert_eq!(
        after_cloud.bindings[1].provider_thread_id.as_deref(),
        Some("local-v2")
    );

    world
        .repo_a
        .mutate_conversation_bindings(|document| {
            let preference = document
                .preferences
                .iter_mut()
                .find(|preference| {
                    preference.conflict_group_id == ConflictGroupId::parse("group-main").unwrap()
                })
                .unwrap();
            preference.preferred_variant_id = v1_id.clone();
            preference.revision += 1;
            preference.updated_at = 30;
            document.bindings.push(LocalConversationBinding {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                conflict_group_id: ConflictGroupId::parse("group-stored-only").unwrap(),
                variant_id: VariantId::parse("variant-stored-only").unwrap(),
                provider_thread_id: None,
                materialization_state: ConversationMaterializationState::StoredOnly,
                last_observed_file_sha256: None,
                last_observed_head_message_id: None,
                last_observed_lineage_digest: None,
                last_observed_semantic_digest: None,
                source_generation: Some(promoted_cloud.snapshot.head.generation),
                source_manifest_sha256: Some(promoted_cloud.snapshot.head.manifest_sha256.clone()),
                revision: 0,
                created_at: 30,
                updated_at: 30,
            });
            Ok(())
        })
        .unwrap();
    let after_local = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(after_local.preferences.len(), 1);
    assert_eq!(after_local.preferences[0].preferred_variant_id, v1_id);
    assert_eq!(
        world
            .engine()
            .inspect(&world.bundle_id)
            .unwrap()
            .manifest
            .conversation_groups[&ConflictGroupId::parse("group-main").unwrap()]
            .primary_variant_id,
        v2_id
    );
    let stored_only = after_local
        .bindings
        .iter()
        .find(|binding| {
            binding.conflict_group_id == ConflictGroupId::parse("group-stored-only").unwrap()
        })
        .unwrap();
    assert_eq!(
        stored_only.materialization_state,
        ConversationMaterializationState::StoredOnly
    );
    assert!(stored_only.provider_thread_id.is_none());
    assert!(!after_local.preferences.iter().any(|preference| {
        preference.conflict_group_id == ConflictGroupId::parse("group-stored-only").unwrap()
    }));
}

#[allow(clippy::too_many_arguments)]
fn install_active_branch(
    repository: &V3Repository,
    scope: &InstalledScope,
    group_id: &ConflictGroupId,
    variant_id: &VariantId,
    provider_thread_id: &str,
    transcript: &[u8],
    source: &crate::project_sync_v3::bundle_engine::Schema5PublishOutcome,
    updated_at: u64,
) -> PathBuf {
    let sessions = scope.provider_root.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let transcript_path = sessions.join(format!("{provider_thread_id}.jsonl"));
    fs::write(&transcript_path, transcript).unwrap();
    let index_path = scope.provider_root.join("session_index.jsonl");
    fs::write(
        &index_path,
        merge_index_target(None, provider_thread_id, &scope.project_root),
    )
    .unwrap();
    let analysis = analyze_codex_transcript(transcript);
    let (head_message_id, lineage_digest, semantic_digest) = match analysis.lineage {
        CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        } => (
            chain.last().unwrap().message_id.clone(),
            lineage_digest,
            semantic_digest,
        ),
        other => panic!("active branch is not resumable: {other:?}"),
    };
    repository
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                conflict_group_id: group_id.clone(),
                variant_id: variant_id.clone(),
                provider_thread_id: Some(provider_thread_id.to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(transcript)),
                last_observed_head_message_id: Some(head_message_id.to_string()),
                last_observed_lineage_digest: Some(lineage_digest),
                last_observed_semantic_digest: Some(semantic_digest),
                source_generation: Some(source.snapshot.head.generation),
                source_manifest_sha256: Some(source.snapshot.head.manifest_sha256.clone()),
                revision: 0,
                created_at: updated_at,
                updated_at,
            });
            document.preferences.push(LocalConversationPreference {
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                conflict_group_id: group_id.clone(),
                preferred_variant_id: variant_id.clone(),
                revision: 0,
                updated_at,
            });
            Ok(())
        })
        .unwrap();
    transcript_path
}

#[allow(clippy::too_many_arguments)]
fn publish_bound_variant(
    world: &VariantWorld,
    repository: &V3Repository,
    scope: &InstalledScope,
    variants: &[VariantSpec],
    primary_variant_id: &VariantId,
    bound_variant_id: &VariantId,
    provider_thread_id: &str,
    local_transcript: &[u8],
    publisher: crate::project_sync_v3::domain::PortableOrigin,
    operation_id: &str,
    updated_at: u64,
) -> crate::project_sync_v3::bundle_engine::FetchedBundle {
    let engine = world.engine();
    let current = engine.inspect(&world.bundle_id).unwrap();
    let head_token = world.current_head().unwrap();
    let request = super::harness::schema5_request(
        world.bundle_id.clone(),
        variants,
        primary_variant_id,
        publisher,
        updated_at,
        Some(&current.manifest),
        crate::project_sync_v3::bundle_engine::PublishExpectation::Match(head_token),
    );
    let bindings = repository.load_conversation_bindings().unwrap();
    let binding = bindings
        .bindings
        .iter()
        .find(|binding| {
            binding.host_id == scope.host_id
                && binding.replica_id == scope.replica_id
                && binding.local_project_id == scope.project_id
                && binding.provider_profile_id == scope.profile_id
                && binding.conflict_group_id == ConflictGroupId::parse("group-main").unwrap()
                && binding.variant_id == *bound_variant_id
        })
        .unwrap();
    let preference_revision = bindings
        .preferences
        .iter()
        .find(|preference| {
            preference.host_id == scope.host_id
                && preference.replica_id == scope.replica_id
                && preference.local_project_id == scope.project_id
                && preference.provider_profile_id == scope.profile_id
                && preference.conflict_group_id == ConflictGroupId::parse("group-main").unwrap()
        })
        .map(|preference| preference.revision);
    let analysis = analyze_codex_transcript(local_transcript);
    let (lineage_digest, semantic_digest) = match analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        other => panic!("published local branch is not resumable: {other:?}"),
    };
    let operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(format!("plan-{operation_id}")).unwrap()),
        direction: ConversationPlanDirection::Push,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: current.head.generation,
        commit_id: current.head.commit_id,
        manifest_sha256: current.head.manifest_sha256,
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: crate::project_sync_v3::domain::Provider::Codex,
        conflict_group_id: ConflictGroupId::parse("group-main").unwrap(),
        source_variant_id: Some(bound_variant_id.clone()),
        old_variant_id: None,
        rekeyed_local_variant_id: None,
        rekey_committed: false,
        result_variant_id: bound_variant_id.clone(),
        provider_thread_id: Some(provider_thread_id.to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(lineage_digest),
        semantic_digest: Some(semantic_digest),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings.revision,
        expected_binding_revision: Some(binding.revision),
        expected_preference_revision: preference_revision,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(local_transcript)),
        initialize_preference: false,
        targets: Vec::new(),
        created_at: updated_at,
        updated_at,
    };
    let mut no_failpoints = NoConversationFailpoints;
    apply_conversation_cloud_publication(
        repository,
        operation,
        || {
            engine
                .publish_schema5(request)
                .map(|published| ConversationCloudPublication {
                    generation: published.snapshot.head.generation,
                    commit_id: published.snapshot.head.commit_id,
                    manifest_sha256: published.snapshot.head.manifest_sha256,
                })
        },
        || Ok(None),
        &mut no_failpoints,
    )
    .unwrap();
    assert!(repository
        .load_conversation_sync_journal()
        .unwrap()
        .operations
        .is_empty());
    engine.fetch(&world.bundle_id).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn materialize_variant(
    repository: &V3Repository,
    scope: &InstalledScope,
    world: &VariantWorld,
    group_id: &ConflictGroupId,
    variant_id: &VariantId,
    source_thread_id: &str,
    destination_thread_id: &str,
    source_transcript: &[u8],
    operation_id: &str,
    updated_at: u64,
) -> PathBuf {
    let fetched = world.engine().fetch(&world.bundle_id).unwrap();
    let rewritten = rewrite_codex_transcript(
        source_transcript,
        source_thread_id,
        destination_thread_id,
        "/tmp/project",
        scope.project_root.to_str().unwrap(),
    )
    .unwrap();
    let analysis = analyze_codex_transcript(&rewritten);
    let (lineage_digest, semantic_digest) = match analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        other => panic!("incoming branch is not resumable: {other:?}"),
    };
    let sessions = scope.provider_root.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let transcript_path = sessions.join(format!("{destination_thread_id}.jsonl"));
    let index_path = scope.provider_root.join("session_index.jsonl");
    let existing_index = fs::read(&index_path).ok();
    let index = merge_index_target(
        existing_index.as_deref(),
        destination_thread_id,
        &scope.project_root,
    );
    let bindings = repository.load_conversation_bindings().unwrap();
    let existing_binding = bindings.bindings.iter().find(|binding| {
        binding.host_id == scope.host_id
            && binding.replica_id == scope.replica_id
            && binding.local_project_id == scope.project_id
            && binding.provider_profile_id == scope.profile_id
            && binding.conflict_group_id == *group_id
            && binding.variant_id == *variant_id
    });
    let preference_revision = bindings
        .preferences
        .iter()
        .find(|preference| {
            preference.host_id == scope.host_id
                && preference.replica_id == scope.replica_id
                && preference.local_project_id == scope.project_id
                && preference.provider_profile_id == scope.profile_id
                && preference.conflict_group_id == *group_id
        })
        .map(|preference| preference.revision);
    let operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(format!("plan-{operation_id}")).unwrap()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: fetched.snapshot.head.generation,
        commit_id: fetched.snapshot.head.commit_id.clone(),
        manifest_sha256: fetched.snapshot.head.manifest_sha256.clone(),
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: crate::project_sync_v3::domain::Provider::Codex,
        conflict_group_id: group_id.clone(),
        source_variant_id: Some(variant_id.clone()),
        old_variant_id: None,
        rekeyed_local_variant_id: None,
        rekey_committed: false,
        result_variant_id: variant_id.clone(),
        provider_thread_id: Some(destination_thread_id.to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(lineage_digest),
        semantic_digest: Some(semantic_digest),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings.revision,
        expected_binding_revision: existing_binding.map(|binding| binding.revision),
        expected_preference_revision: preference_revision,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: existing_binding
            .and_then(|binding| binding.last_observed_file_sha256.clone()),
        initialize_preference: false,
        targets: Vec::new(),
        created_at: updated_at,
        updated_at,
    };
    let request = ConversationMaterializationRequest {
        operation,
        targets: vec![
            ConversationTargetPayload {
                target_path: transcript_path.clone(),
                role: ConversationFileRole::Transcript,
                expected: target_expectation(&transcript_path),
                bytes: rewritten,
            },
            ConversationTargetPayload {
                target_path: index_path.clone(),
                role: ConversationFileRole::ProviderIndexFragment,
                expected: target_expectation(&index_path),
                bytes: index,
            },
        ],
    };
    apply_conversation_materialization(repository, request, &mut NoConversationFailpoints).unwrap();
    transcript_path
}

fn merge_index_target(existing: Option<&[u8]>, thread_id: &str, cwd: &Path) -> Vec<u8> {
    let mut rows = BTreeMap::<String, serde_json::Value>::new();
    for line in existing
        .unwrap_or_default()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = serde_json::from_slice(line).unwrap();
        let id = value
            .get("id")
            .or_else(|| value.get("session_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap()
            .to_string();
        rows.insert(id, value);
    }
    rows.insert(
        thread_id.to_string(),
        serde_json::json!({
            "id": thread_id,
            "cwd": cwd.to_str().unwrap(),
            "title": "Variant continuation fixture"
        }),
    );
    let mut output = Vec::new();
    for row in rows.into_values() {
        output.extend_from_slice(&serde_json::to_vec(&row).unwrap());
        output.push(b'\n');
    }
    output
}

fn target_expectation(path: &Path) -> TargetExpectation {
    match fs::read(path) {
        Ok(bytes) => TargetExpectation::Sha256(sha256(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => TargetExpectation::Absent,
        Err(error) => panic!("read target '{}': {error}", path.display()),
    }
}

fn assert_local_branch_state(
    repository: &V3Repository,
    scope: &InstalledScope,
    group_id: &ConflictGroupId,
    preferred_variant_id: &VariantId,
    expected: &[(&VariantId, &str)],
) {
    let document = repository.load_conversation_bindings().unwrap();
    let actual = document
        .bindings
        .iter()
        .filter(|binding| {
            binding.host_id == scope.host_id
                && binding.replica_id == scope.replica_id
                && binding.local_project_id == scope.project_id
                && binding.provider_profile_id == scope.profile_id
                && binding.conflict_group_id == *group_id
        })
        .map(|binding| {
            assert_eq!(
                binding.materialization_state,
                ConversationMaterializationState::Resumable,
            );
            (
                binding.variant_id.clone(),
                binding.provider_thread_id.clone().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let expected = expected
        .iter()
        .map(|(variant_id, thread_id)| ((*variant_id).clone(), (*thread_id).to_string()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual, expected);
    assert_eq!(
        actual.values().collect::<BTreeSet<_>>().len(),
        actual.len(),
        "provider IDs must be injective throughout the profile",
    );
    let preference = document
        .preferences
        .iter()
        .find(|preference| {
            preference.host_id == scope.host_id
                && preference.replica_id == scope.replica_id
                && preference.local_project_id == scope.project_id
                && preference.provider_profile_id == scope.profile_id
                && preference.conflict_group_id == *group_id
        })
        .unwrap();
    assert_eq!(&preference.preferred_variant_id, preferred_variant_id);
    let index = fs::read(scope.provider_root.join("session_index.jsonl")).unwrap();
    let index_ids = index
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_slice(line).unwrap();
            value["id"].as_str().unwrap().to_string()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        index_ids,
        expected.values().cloned().collect::<BTreeSet<_>>()
    );
}

fn append_message(bytes: &[u8], id: &str, text: &str) -> Vec<u8> {
    let analysis = analyze_codex_transcript(bytes);
    let parent = analysis
        .lineage
        .chain()
        .unwrap()
        .last()
        .unwrap()
        .message_id
        .as_str()
        .to_string();
    let mut output = bytes.to_vec();
    output.extend_from_slice(
        serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-07-22T00:01:00Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": id,
                "parent_id": parent,
                "role": "user",
                "content": [{"type": "input_text", "text": text}]
            }
        }))
        .unwrap()
        .as_bytes(),
    );
    output.push(b'\n');
    output
}

fn digests(
    analysis: &crate::project_sync_v3::conversation_codec::CodexTranscriptAnalysis,
) -> (&str, &str) {
    match &analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        CodecLineage::Unknown { reason } => panic!("unexpected unknown lineage: {reason:?}"),
    }
}
