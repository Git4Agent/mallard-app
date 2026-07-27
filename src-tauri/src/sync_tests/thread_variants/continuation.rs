use super::harness::{assert_manifest_consistent, chain, sha256, ThreadSpec, VariantWorld};
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
    ConversationFileRole, ConversationMaterializationState, ConversationOperationId,
    ConversationPlanActionType, ConversationPlanDirection, ConversationRelation,
    ConversationSyncOperation, ConversationSyncPhase, LocalConversationBinding, PlanId,
    TargetExpectation, ThreadId,
};
use crate::project_sync_v3::persistence::V3Repository;
use crate::project_sync_v3::provider_capture::{
    CapturedConversation, CapturedFile, CapturedResources, Provider as CaptureProvider,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn schema6_true_noop_does_not_publish_or_change_provenance() {
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let thread = ThreadSpec::new(
        "thread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "one"), ("m-b", "two")]),
        origin_a.clone(),
        10,
    );
    let first = world.publish(&[thread.clone()], origin_a.clone(), 10);
    assert!(first.published);
    let first_head = first.snapshot.head.clone();
    let first_manifest = first.snapshot.manifest.clone();

    let origin_b = world.origin(&world.repo_b, "replica-b");
    let repeated = world.publish(&[thread], origin_b, 20);
    assert!(!repeated.published);
    assert_eq!(repeated.snapshot.head, first_head);
    assert_eq!(repeated.snapshot.manifest, first_manifest);
    assert_eq!(repeated.snapshot.manifest.published_by_origin, origin_a);
    assert_manifest_consistent(&world);
}

#[test]
fn bound_thread_push_updates_only_its_thread() {
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let t1_id = ThreadId::parse("thread-v1").unwrap();
    let t2_id = ThreadId::parse("thread-v2").unwrap();
    let t1 = ThreadSpec::new(
        t1_id.as_str(),
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
    let mut t2 = ThreadSpec::new(
        t2_id.as_str(),
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
    t2.forked_from = Some(t1_id.clone());
    let first = world.publish(&[t1.clone(), t2.clone()], origin_a, 10);
    let t1_path = first.snapshot.manifest.conversations[&t1_id].files[0]
        .logical_path
        .clone();
    let t2_path = first.snapshot.manifest.conversations[&t2_id].files[0]
        .logical_path
        .clone();
    let t1_entry = first.snapshot.manifest.files[&t1_path].clone();
    let t2_entry = first.snapshot.manifest.files[&t2_path].clone();
    let t1_descriptor = first.snapshot.manifest.resources
        [&first.snapshot.manifest.files[&t1_path].resource_id]
        .clone();

    let mut advanced_t2 = t2;
    advanced_t2.transcript = chain(
        "source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "base answer"),
            ("m-c2", "branch two"),
            ("m-d2", "continued on B"),
        ],
    );
    advanced_t2.updated_at = 20;
    advanced_t2.last_updated_by = Some(origin_b.clone());
    let second = world.publish(&[t1, advanced_t2], origin_b, 20);
    assert!(second.published);
    assert_eq!(second.snapshot.head.latest.generation, 2);
    assert_eq!(second.snapshot.manifest.files[&t1_path], t1_entry);
    assert_eq!(
        second.snapshot.manifest.resources[&t1_entry.resource_id],
        t1_descriptor
    );
    assert_eq!(
        second.snapshot.manifest.conversations[&t2_id].forked_from,
        Some(t1_id)
    );
    assert_ne!(second.snapshot.manifest.files[&t2_path], t2_entry);
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

    let t1_id = ThreadId::parse("thread-v1").unwrap();
    let t2_id = ThreadId::parse("thread-v2").unwrap();
    let mut t1 = ThreadSpec::new(
        t1_id.as_str(),
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
    let mut t2 = ThreadSpec::new(
        t2_id.as_str(),
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
    t2.forked_from = Some(t1_id.clone());
    let initial = world.publish(&[t1.clone(), t2.clone()], origin_a.clone(), 10);
    assert_eq!(initial.snapshot.head.latest.generation, 1);

    let a_active = rewrite_codex_transcript(
        &t1.transcript,
        "shared-source-thread",
        "a-active-thread",
        "/tmp/project",
        scope_a.project_root.to_str().unwrap(),
    )
    .unwrap();
    let b_active = rewrite_codex_transcript(
        &t2.transcript,
        "shared-source-thread",
        "b-active-thread",
        "/tmp/project",
        scope_b.project_root.to_str().unwrap(),
    )
    .unwrap();
    let a_active_path = install_active_branch(
        &world.repo_a,
        &scope_a,
        &t1_id,
        "a-active-thread",
        &a_active,
        &initial,
        10,
    );
    let b_active_path = install_active_branch(
        &world.repo_b,
        &scope_b,
        &t2_id,
        "b-active-thread",
        &b_active,
        &initial,
        10,
    );

    let a_after_first_append = append_message(&a_active, "m-d1", "continued on A");
    fs::write(&a_active_path, &a_after_first_append).unwrap();
    t1.transcript = rewrite_codex_transcript(
        &a_after_first_append,
        "a-active-thread",
        "shared-source-thread",
        scope_a.project_root.to_str().unwrap(),
        "/tmp/project",
    )
    .unwrap();
    t1.updated_at = 20;
    t1.last_updated_by = Some(origin_a.clone());
    let after_a = publish_bound_thread(
        &world,
        &world.repo_a,
        &scope_a,
        &[t1.clone(), t2.clone()],
        &t1_id,
        "a-active-thread",
        &a_after_first_append,
        origin_a.clone(),
        "operation-continuation-a-first",
        20,
    );
    assert_eq!(after_a.snapshot.manifest.conversations.len(), 2);

    let b_active_before_pull = fs::read(&b_active_path).unwrap();
    let b_incoming_t1 = materialize_thread(
        &world.repo_b,
        &scope_b,
        &world,
        &t1_id,
        "shared-source-thread",
        "b-alternate-v1",
        &t1.transcript,
        "operation-pull-b-v1",
        25,
    );
    assert_eq!(fs::read(&b_active_path).unwrap(), b_active_before_pull);
    assert_ne!(fs::read(&b_incoming_t1).unwrap(), t1.transcript);
    assert_eq!(
        digests(&analyze_codex_transcript(&t1.transcript)),
        digests(&analyze_codex_transcript(
            &fs::read(&b_incoming_t1).unwrap()
        )),
    );
    assert_local_branch_state(
        &world.repo_b,
        &scope_b,
        &[(&t1_id, "b-alternate-v1"), (&t2_id, "b-active-thread")],
    );

    let b_after_append = append_message(&b_active_before_pull, "m-d2", "continued on B");
    fs::write(&b_active_path, &b_after_append).unwrap();
    t2.transcript = rewrite_codex_transcript(
        &b_after_append,
        "b-active-thread",
        "shared-source-thread",
        scope_b.project_root.to_str().unwrap(),
        "/tmp/project",
    )
    .unwrap();
    t2.updated_at = 30;
    t2.last_updated_by = Some(origin_b.clone());
    let after_b = publish_bound_thread(
        &world,
        &world.repo_b,
        &scope_b,
        &[t1.clone(), t2.clone()],
        &t2_id,
        "b-active-thread",
        &b_after_append,
        origin_b.clone(),
        "operation-continuation-b",
        30,
    );
    assert_eq!(after_b.snapshot.head.latest.generation, 3);
    assert_eq!(after_b.snapshot.manifest.conversations.len(), 2);

    let a_active_before_pull = fs::read(&a_active_path).unwrap();
    let a_incoming_t2 = materialize_thread(
        &world.repo_a,
        &scope_a,
        &world,
        &t2_id,
        "shared-source-thread",
        "a-alternate-v2",
        &t2.transcript,
        "operation-pull-a-v2",
        35,
    );
    assert_eq!(fs::read(&a_active_path).unwrap(), a_active_before_pull);
    assert_eq!(
        digests(&analyze_codex_transcript(&t2.transcript)),
        digests(&analyze_codex_transcript(
            &fs::read(&a_incoming_t2).unwrap()
        )),
    );
    assert_local_branch_state(
        &world.repo_a,
        &scope_a,
        &[(&t1_id, "a-active-thread"), (&t2_id, "a-alternate-v2")],
    );

    let a_after_second_append =
        append_message(&a_active_before_pull, "m-e1", "continued again on A");
    fs::write(&a_active_path, &a_after_second_append).unwrap();
    t1.transcript = rewrite_codex_transcript(
        &a_after_second_append,
        "a-active-thread",
        "shared-source-thread",
        scope_a.project_root.to_str().unwrap(),
        "/tmp/project",
    )
    .unwrap();
    t1.updated_at = 40;
    t1.last_updated_by = Some(origin_a.clone());
    let after_a_second = publish_bound_thread(
        &world,
        &world.repo_a,
        &scope_a,
        &[t1.clone(), t2.clone()],
        &t1_id,
        "a-active-thread",
        &a_after_second_append,
        origin_a.clone(),
        "operation-continuation-a-second",
        40,
    );
    assert_eq!(after_a_second.snapshot.head.latest.generation, 4);

    let b_active_before_refresh = fs::read(&b_active_path).unwrap();
    let refreshed_b_t1 = materialize_thread(
        &world.repo_b,
        &scope_b,
        &world,
        &t1_id,
        "shared-source-thread",
        "b-alternate-v1",
        &t1.transcript,
        "operation-pull-b-v1-refresh",
        45,
    );
    assert_eq!(refreshed_b_t1, b_incoming_t1);
    assert_eq!(fs::read(&b_active_path).unwrap(), b_active_before_refresh);
    assert_local_branch_state(
        &world.repo_b,
        &scope_b,
        &[(&t1_id, "b-alternate-v1"), (&t2_id, "b-active-thread")],
    );

    let local_a_before_noop = world.repo_a.load_conversation_bindings().unwrap();
    let local_b_before_noop = world.repo_b.load_conversation_bindings().unwrap();
    let fixed_point = world.publish(&[t1.clone(), t2.clone()], origin_b.clone(), 50);
    assert!(!fixed_point.published);
    assert_eq!(fixed_point.snapshot.head.latest.generation, 4);
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
        &[(&t1_id, "a-active-thread"), (&t2_id, "a-alternate-v2")],
    );
    assert_local_branch_state(
        &reopened_b,
        &scope_b,
        &[(&t1_id, "b-alternate-v1"), (&t2_id, "b-active-thread")],
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
fn schema6_comparison_exposes_distinct_typed_threads_without_guessing_by_title() {
    let world = VariantWorld::new();
    let scope = install_scope(
        &world.repo_fresh,
        &world.bundle_id,
        "comparison",
        "replica-comparison",
    );
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let t1_id = ThreadId::parse("thread-v1").unwrap();
    let t2_id = ThreadId::parse("thread-v2").unwrap();
    let t1 = ThreadSpec::new(
        t1_id.as_str(),
        "shared-source-thread",
        chain(
            "shared-source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "answer"),
                ("m-c1", "cloud original"),
            ],
        ),
        origin_a,
        10,
    );
    let mut t2 = ThreadSpec::new(
        t2_id.as_str(),
        "shared-source-thread",
        chain(
            "shared-source-thread",
            &[("m-a", "base"), ("m-b", "answer"), ("m-c2", "local branch")],
        ),
        origin_b,
        10,
    );
    t2.forked_from = Some(t1_id.clone());
    let published = world.publish(
        &[t1, t2.clone()],
        world.origin(&world.repo_a, "replica-a"),
        10,
    );

    let local_transcript = append_message(&t2.transcript, "m-d2", "continued here");
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
                thread_id: t2_id.clone(),
                provider_thread_id: Some("local-v2".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(&local_transcript)),
                last_observed_head_message_id: Some(head_message_id.to_string()),
                last_observed_lineage_digest: Some(lineage_digest.clone()),
                last_observed_semantic_digest: Some(semantic_digest.clone()),
                source_generation: Some(published.snapshot.head.latest.generation),
                source_manifest_sha256: Some(
                    published.snapshot.head.latest.manifest_sha256.clone(),
                ),
                revision: 0,
                created_at: 10,
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
            title: None,
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
    let mut bytes = crate::project_sync_v3::bundle_engine::PreloadedBytes::new(&fetched);
    let entries = crate::project_sync_v3::commands::stored_thread_entries(
        &world.repo_fresh,
        &fetched.snapshot,
        &mut bytes,
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
        vec!["Thread test conversation", "Thread test conversation"]
    );
    let origin_thread = entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&t1_id))
        .unwrap();
    assert!(origin_thread.forked_from.is_none());
    assert!(!origin_thread.local_present);
    assert_eq!(origin_thread.state, ThreadSyncState::StorageOnly);
    assert_eq!(
        origin_thread.relation,
        Some(ConversationRelation::LocalMissing)
    );
    assert_eq!(
        origin_thread.proposed_action,
        Some(ConversationPlanActionType::CreateLocal)
    );
    assert_eq!(
        origin_thread.materialization_state,
        Some(ConversationMaterializationState::StoredOnly)
    );
    assert!(!origin_thread.resumable);
    assert!(origin_thread.provider_thread_id.is_none());
    assert!(origin_thread.origin_label.is_some());
    assert!(!origin_thread.requires_explicit_approval);

    let bound = entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&t2_id))
        .unwrap();
    assert_eq!(
        bound.forked_from.as_ref(),
        Some(&t1_id),
        "fork provenance is surfaced as a hint, never used for matching",
    );
    assert!(bound.local_present);
    assert_eq!(bound.state, ThreadSyncState::LocalAhead);
    assert_eq!(
        bound.relation,
        Some(ConversationRelation::CloudPrefixOfLocal)
    );
    assert_eq!(
        bound.proposed_action,
        Some(ConversationPlanActionType::FastForwardCloud)
    );
    assert_eq!(
        bound.materialization_state,
        Some(ConversationMaterializationState::Resumable)
    );
    assert!(bound.resumable);
    assert_eq!(bound.provider_thread_id.as_deref(), Some("local-v2"));
    assert!(bound.origin_label.is_some());
    assert_ne!(origin_thread.thread_id, bound.thread_id);
}

#[allow(clippy::too_many_arguments)]
fn install_active_branch(
    repository: &V3Repository,
    scope: &InstalledScope,
    thread_id: &ThreadId,
    provider_thread_id: &str,
    transcript: &[u8],
    source: &crate::project_sync_v3::bundle_engine::PublishedBundle,
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
                thread_id: thread_id.clone(),
                provider_thread_id: Some(provider_thread_id.to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(transcript)),
                last_observed_head_message_id: Some(head_message_id.to_string()),
                last_observed_lineage_digest: Some(lineage_digest),
                last_observed_semantic_digest: Some(semantic_digest),
                source_generation: Some(source.snapshot.head.latest.generation),
                source_manifest_sha256: Some(source.snapshot.head.latest.manifest_sha256.clone()),
                revision: 0,
                created_at: updated_at,
                updated_at,
            });
            Ok(())
        })
        .unwrap();
    transcript_path
}

#[allow(clippy::too_many_arguments)]
fn publish_bound_thread(
    world: &VariantWorld,
    repository: &V3Repository,
    scope: &InstalledScope,
    threads: &[ThreadSpec],
    bound_thread_id: &ThreadId,
    provider_thread_id: &str,
    local_transcript: &[u8],
    publisher: crate::project_sync_v3::domain::PortableOrigin,
    operation_id: &str,
    updated_at: u64,
) -> crate::project_sync_v3::bundle_engine::FetchedBundle {
    let engine = world.engine();
    let current = engine.inspect(&world.bundle_id).unwrap();
    let head_token = world.current_head().unwrap();
    let request = super::harness::publish_request(
        world.bundle_id.clone(),
        threads,
        publisher,
        updated_at,
        Some(&current.manifest),
        Some(head_token),
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
                && binding.thread_id == *bound_thread_id
        })
        .unwrap();
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
        generation: current.head.latest.generation,
        commit_id: current.head.latest.commit_id,
        manifest_sha256: current.head.latest.manifest_sha256,
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: crate::project_sync_v3::domain::Provider::Codex,
        thread_id: bound_thread_id.clone(),
        old_thread_id: None,
        rekey_committed: false,
        result_thread_id: bound_thread_id.clone(),
        forked_from_provider_thread_id: None,
        provider_thread_id: Some(provider_thread_id.to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(lineage_digest),
        semantic_digest: Some(semantic_digest),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings.revision,
        expected_binding_revision: Some(binding.revision),
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(local_transcript)),
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
                .publish(request)
                .map(|published| ConversationCloudPublication {
                    generation: published.snapshot.head.latest.generation,
                    commit_id: published.snapshot.head.latest.commit_id,
                    manifest_sha256: published.snapshot.head.latest.manifest_sha256,
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
fn materialize_thread(
    repository: &V3Repository,
    scope: &InstalledScope,
    world: &VariantWorld,
    thread_id: &ThreadId,
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
            && binding.thread_id == *thread_id
    });
    let operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(format!("plan-{operation_id}")).unwrap()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: fetched.snapshot.head.latest.generation,
        commit_id: fetched.snapshot.head.latest.commit_id.clone(),
        manifest_sha256: fetched.snapshot.head.latest.manifest_sha256.clone(),
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: crate::project_sync_v3::domain::Provider::Codex,
        thread_id: thread_id.clone(),
        old_thread_id: None,
        rekey_committed: false,
        result_thread_id: thread_id.clone(),
        forked_from_provider_thread_id: None,
        provider_thread_id: Some(destination_thread_id.to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(lineage_digest),
        semantic_digest: Some(semantic_digest),
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings.revision,
        expected_binding_revision: existing_binding.map(|binding| binding.revision),
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: existing_binding
            .and_then(|binding| binding.last_observed_file_sha256.clone()),
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
            "title": "Thread continuation fixture"
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
    expected: &[(&ThreadId, &str)],
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
        })
        .map(|binding| {
            assert_eq!(
                binding.materialization_state,
                ConversationMaterializationState::Resumable,
            );
            (
                binding.thread_id.clone(),
                binding.provider_thread_id.clone().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let expected = expected
        .iter()
        .map(|(thread_id, provider_id)| ((*thread_id).clone(), (*provider_id).to_string()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual, expected);
    assert_eq!(
        actual.values().collect::<BTreeSet<_>>().len(),
        actual.len(),
        "provider IDs must be injective throughout the profile",
    );
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
