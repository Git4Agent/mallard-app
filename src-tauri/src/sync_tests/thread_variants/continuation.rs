use super::harness::{
    assert_manifest_consistent, chain, chain_at, codex, codex_scope, ThreadSpec, TranscriptBuilder,
    VariantWorld,
};
use super::scope::install_scope;
use crate::project_sync::service::ThreadSyncState;
use mallard_domain::{
    ConversationBlocker, ConversationMaterializationState, ConversationReviewAction,
    LocalConversationBinding, SessionPresentation, ThreadId,
};
use mallard_infrastructure::conversation_codec::{
    analyze_codex_transcript, CODEX_LINEAGE_CODEC_KEY,
};
use mallard_infrastructure::provider_capture::{
    CapturedConversation, CapturedFile, CapturedResources,
};
use std::fs;

#[test]
fn current_schema_true_noop_does_not_publish_or_change_provenance() {
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let thread = ThreadSpec::new(
        "cloudthread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "one"), ("m-b", "two")]),
        origin_a.clone(),
        10,
    );
    let first = world.publish(std::slice::from_ref(&thread), origin_a.clone(), 10);
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
    let t1_id = CloudThreadId::parse("cloudthread-v1").unwrap();
    let t2_id = CloudThreadId::parse("cloudthread-v2").unwrap();
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
    let first_scope = codex_scope(&first.snapshot.manifest);
    let t1_path = first_scope.conversations[&t1_id].files[0]
        .logical_path
        .clone();
    let t2_path = first_scope.conversations[&t2_id].files[0]
        .logical_path
        .clone();
    let t1_entry = first_scope.files[&t1_path].clone();
    let t2_entry = first_scope.files[&t2_path].clone();
    let t1_descriptor = first_scope.resources[&first_scope.files[&t1_path].resource_id].clone();

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
    assert_eq!(second.snapshot.head.revision, 2);
    let second_scope = codex_scope(&second.snapshot.manifest);
    assert_eq!(second_scope.files[&t1_path], t1_entry);
    assert_eq!(second_scope.resources[&t1_entry.resource_id], t1_descriptor);
    assert_eq!(second_scope.conversations[&t2_id].forked_from, Some(t1_id));
    assert_ne!(second_scope.files[&t2_path], t2_entry);
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

    let t1_id = CloudThreadId::parse("cloudthread-v1").unwrap();
    let t2_id = CloudThreadId::parse("cloudthread-v2").unwrap();
    // Each machine registers its own local identity for a cloud thread; the
    // pairing lives only in the binding rows' mappings.
    let t1_local = ThreadId::parse("thread-v1").unwrap();
    let t2_local = ThreadId::parse("thread-v2").unwrap();
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
    assert_eq!(initial.snapshot.head.revision, 1);

    let a_active = chain_at(
        "a-active-thread",
        scope_a.project_root.to_str().unwrap(),
        &[
            ("m-a", "base"),
            ("m-b", "base answer"),
            ("m-c1", "A branch"),
        ],
    );
    let b_active = chain_at(
        "b-active-thread",
        scope_b.project_root.to_str().unwrap(),
        &[
            ("m-a", "base"),
            ("m-b", "base answer"),
            ("m-c2", "B branch"),
        ],
    );
    let a_active_path = install_active_branch(
        &world.repo_a,
        &scope_a,
        &t1_local,
        &t1_id,
        "a-active-thread",
        &a_active,
        &initial,
        10,
    );
    let b_active_path = install_active_branch(
        &world.repo_b,
        &scope_b,
        &t2_local,
        &t2_id,
        "b-active-thread",
        &b_active,
        &initial,
        10,
    );

    let a_after_first_append = append_message(&a_active, "m-d1", "continued on A");
    fs::write(&a_active_path, &a_after_first_append).unwrap();
    t1.transcript = TranscriptBuilder::new("shared-source-thread", "/tmp/project")
        .message("m-a", "user", "base")
        .message("m-b", "assistant", "base answer")
        .message("m-c1", "user", "A branch")
        .message("m-d1", "user", "continued on A")
        .build();
    t1.updated_at = 20;
    t1.last_updated_by = Some(origin_a.clone());
    let after_a = publish_bound_thread(
        &world,
        &world.repo_a,
        &scope_a,
        &[t1.clone(), t2.clone()],
        &t1_local,
        &t1_id,
        "a-active-thread",
        &a_after_first_append,
        origin_a.clone(),
        "operation-continuation-a-first",
        20,
    );
    assert_eq!(codex_scope(&after_a.manifest).conversations.len(), 2);

    let b_active_before_pull = fs::read(&b_active_path).unwrap();
    let b_incoming_t1 = materialize_thread(
        &world.repo_b,
        &scope_b,
        &world,
        &t1_local,
        &t1_id,
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
        &[
            (&t1_local, "b-alternate-v1"),
            (&t2_local, "b-active-thread"),
        ],
    );

    let b_after_append = append_message(&b_active_before_pull, "m-d2", "continued on B");
    fs::write(&b_active_path, &b_after_append).unwrap();
    t2.transcript = TranscriptBuilder::new("shared-source-thread", "/tmp/project")
        .message("m-a", "user", "base")
        .message("m-b", "assistant", "base answer")
        .message("m-c2", "user", "B branch")
        .message("m-d2", "user", "continued on B")
        .build();
    t2.updated_at = 30;
    t2.last_updated_by = Some(origin_b.clone());
    let after_b = publish_bound_thread(
        &world,
        &world.repo_b,
        &scope_b,
        &[t1.clone(), t2.clone()],
        &t2_local,
        &t2_id,
        "b-active-thread",
        &b_after_append,
        origin_b.clone(),
        "operation-continuation-b",
        30,
    );
    assert_eq!(after_b.head.revision, 3);
    assert_eq!(codex_scope(&after_b.manifest).conversations.len(), 2);

    let a_active_before_pull = fs::read(&a_active_path).unwrap();
    let a_incoming_t2 = materialize_thread(
        &world.repo_a,
        &scope_a,
        &world,
        &t2_local,
        &t2_id,
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
        &[
            (&t1_local, "a-active-thread"),
            (&t2_local, "a-alternate-v2"),
        ],
    );

    let a_after_second_append =
        append_message(&a_active_before_pull, "m-e1", "continued again on A");
    fs::write(&a_active_path, &a_after_second_append).unwrap();
    t1.transcript = TranscriptBuilder::new("shared-source-thread", "/tmp/project")
        .message("m-a", "user", "base")
        .message("m-b", "assistant", "base answer")
        .message("m-c1", "user", "A branch")
        .message("m-d1", "user", "continued on A")
        .message("m-e1", "user", "continued again on A")
        .build();
    t1.updated_at = 40;
    t1.last_updated_by = Some(origin_a.clone());
    let after_a_second = publish_bound_thread(
        &world,
        &world.repo_a,
        &scope_a,
        &[t1.clone(), t2.clone()],
        &t1_local,
        &t1_id,
        "a-active-thread",
        &a_after_second_append,
        origin_a.clone(),
        "operation-continuation-a-second",
        40,
    );
    assert_eq!(after_a_second.head.revision, 4);

    let b_active_before_refresh = fs::read(&b_active_path).unwrap();
    let refreshed_b_t1 = materialize_thread(
        &world.repo_b,
        &scope_b,
        &world,
        &t1_local,
        &t1_id,
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
        &[
            (&t1_local, "b-alternate-v1"),
            (&t2_local, "b-active-thread"),
        ],
    );

    let local_a_before_noop = world.repo_a.load_conversation_bindings().unwrap();
    let local_b_before_noop = world.repo_b.load_conversation_bindings().unwrap();
    let fixed_point = world.publish(&[t1.clone(), t2.clone()], origin_b.clone(), 50);
    assert!(!fixed_point.published);
    assert_eq!(fixed_point.snapshot.head.revision, 4);
    assert_eq!(
        world.repo_a.load_conversation_bindings().unwrap(),
        local_a_before_noop,
    );
    assert_eq!(
        world.repo_b.load_conversation_bindings().unwrap(),
        local_b_before_noop,
    );

    let reopened_a =
        LocalStateRepository::from_home_dir(world.repo_a.root().parent().unwrap()).unwrap();
    let reopened_b =
        LocalStateRepository::from_home_dir(world.repo_b.root().parent().unwrap()).unwrap();
    assert_local_branch_state(
        &reopened_a,
        &scope_a,
        &[
            (&t1_local, "a-active-thread"),
            (&t2_local, "a-alternate-v2"),
        ],
    );
    assert_local_branch_state(
        &reopened_b,
        &scope_b,
        &[
            (&t1_local, "b-alternate-v1"),
            (&t2_local, "b-active-thread"),
        ],
    );
    assert_manifest_consistent(&world);
}

#[test]
fn current_schema_comparison_exposes_distinct_typed_threads_without_guessing_by_title() {
    let world = VariantWorld::new();
    let scope = install_scope(
        &world.repo_fresh,
        &world.bundle_id,
        "comparison",
        "replica-comparison",
    );
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let t1_id = CloudThreadId::parse("cloudthread-v1").unwrap();
    let t2_id = CloudThreadId::parse("cloudthread-v2").unwrap();
    let t2_local = ThreadId::parse("thread-v2").unwrap();
    let t3_local = ThreadId::parse("thread-v3").unwrap();
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
    let t3_id = CloudThreadId::parse("cloudthread-v3").unwrap();
    let mut t3 = ThreadSpec::new(
        t3_id.as_str(),
        "shared-source-thread",
        chain(
            "shared-source-thread",
            &[
                ("m-a", "base"),
                ("m-b", "answer"),
                ("m-c3", "cloud sibling"),
            ],
        ),
        world.origin(&world.repo_b, "replica-b"),
        10,
    );
    t3.forked_from = Some(t1_id.clone());
    world.publish(
        &[t1, t2.clone(), t3],
        world.origin(&world.repo_a, "replica-a"),
        10,
    );

    // The bound local copy lives in a real rollout under its own alias and
    // has advanced past the published branch.
    let rebound = chain_at(
        "local-v2",
        &scope.project_root.to_string_lossy(),
        &[("m-a", "base"), ("m-b", "answer"), ("m-c2", "local branch")],
    );
    let local_transcript = append_message(&rebound, "m-d2", "continued here");
    let local_analysis = analyze_codex_transcript(&local_transcript);
    let record_count = local_analysis.record_count;
    let sessions = scope.provider_root.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let local_path = sessions.join("rollout-1-local-v2.jsonl");
    fs::write(&local_path, &local_transcript).unwrap();
    // The third thread's bound local copy diverges from its published chain
    // at the branch tip: it carries m-c2 where the cloud recorded m-c3.
    let diverged_rebound = chain_at(
        "local-v3",
        &scope.project_root.to_string_lossy(),
        &[("m-a", "base"), ("m-b", "answer"), ("m-c2", "local branch")],
    );
    let diverged_transcript = append_message(&diverged_rebound, "m-d3", "continued elsewhere");
    let diverged_analysis = analyze_codex_transcript(&diverged_transcript);
    let diverged_record_count = diverged_analysis.record_count;
    let diverged_path = sessions.join("rollout-1-local-v3.jsonl");
    fs::write(&diverged_path, &diverged_transcript).unwrap();
    world
        .repo_fresh
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                installed_end: None,
                installed_subagents: Default::default(),
                path_origins_ref: None,
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                thread_id: t2_local.clone(),
                cloud_thread_id: Some(t2_id.clone()),
                forked_from: None,
                preserved: None,
                root_tail_layout: Some(whole_thread_layout()),
                provider_thread_id: Some("local-v2".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                revision: 0,
                created_at: 10,
                updated_at: 20,
            });
            document.bindings.push(LocalConversationBinding {
                installed_end: None,
                installed_subagents: Default::default(),
                path_origins_ref: None,
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                thread_id: t3_local.clone(),
                cloud_thread_id: Some(t3_id.clone()),
                forked_from: None,
                preserved: None,
                root_tail_layout: Some(whole_thread_layout()),
                provider_thread_id: Some("local-v3".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
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
            provider: codex(),
            lineage_codec: CODEX_LINEAGE_CODEC_KEY.to_string(),
            source_thread_id: Some("local-v2".to_string()),
            source_cwd: Some(scope.project_root.to_string_lossy().into_owned()),
            transcript_logical_path: transcript_path.clone(),
            source_physical_path: Some(local_path.clone()),
            sidecar_files: Vec::new(),
            record_count,
            lineage: local_analysis.lineage,
            presentation: SessionPresentation::default(),
        },
    );
    captured.files.insert(
        transcript_path.clone(),
        CapturedFile {
            source_stamp: None,
            logical_path: transcript_path,
            resource_id,
            bytes: local_transcript.into(),
            source_mtime: 20,
            mode: 0o600,
        },
    );
    let diverged_resource_id = "codex:session:local-v3".to_string();
    let diverged_transcript_path = "state/codex/sessions/local-v3.jsonl".to_string();
    captured.conversations.insert(
        diverged_resource_id.clone(),
        CapturedConversation {
            provider: codex(),
            lineage_codec: CODEX_LINEAGE_CODEC_KEY.to_string(),
            source_thread_id: Some("local-v3".to_string()),
            source_cwd: Some(scope.project_root.to_string_lossy().into_owned()),
            transcript_logical_path: diverged_transcript_path.clone(),
            source_physical_path: Some(diverged_path.clone()),
            sidecar_files: Vec::new(),
            record_count: diverged_record_count,
            lineage: diverged_analysis.lineage,
            presentation: SessionPresentation::default(),
        },
    );
    captured.files.insert(
        diverged_transcript_path.clone(),
        CapturedFile {
            source_stamp: None,
            logical_path: diverged_transcript_path,
            resource_id: diverged_resource_id,
            bytes: diverged_transcript.into(),
            source_mtime: 20,
            mode: 0o600,
        },
    );
    let machine = world.repo_fresh.load_bindings().unwrap();
    let binding = machine.active_for(&scope.project_id).unwrap();
    let engine = world.engine();
    let snapshot = engine.inspect(&world.bundle_id).unwrap();
    let mut bytes = engine.byte_source(&snapshot, None);
    let entries = crate::project_sync::service::thread_entries(
        &world.repo_fresh,
        &snapshot,
        &mut bytes,
        &captured,
        binding,
    )
    .unwrap();

    assert_eq!(entries.len(), 3);
    let origin_thread = entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&t1_id))
        .unwrap();
    assert!(origin_thread.forked_from.is_none());
    assert!(!origin_thread.local_present);
    assert_eq!(origin_thread.state, ThreadSyncState::StorageOnly);
    assert_eq!(
        origin_thread.pull_decision.action,
        ConversationReviewAction::CreateLocal
    );
    assert!(origin_thread.provider_thread_id.is_none());
    assert!(origin_thread.pull_decision.selectable);

    // A bound thread with no accepted pin whose reviewed cloud tail refolds
    // exactly as a prefix of the local chain re-establishes acceptance
    // evidence at review: the local advance is proven, so Push publishes and
    // Pull reports eligibility instead of holding a provable pair hostage.
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
    assert_eq!(
        bound.push_decision.action,
        ConversationReviewAction::Publish
    );
    assert_eq!(
        bound.pull_decision.action,
        ConversationReviewAction::ReportPushEligible
    );
    assert_eq!(bound.provider_thread_id.as_deref(), Some("local-v2"));
    assert_ne!(origin_thread.thread_id, bound.thread_id);

    // A bound pair whose cloud value fails to refold against the local chain
    // never bootstraps by guessing from titles, forks, or graph shape: it is
    // Unknown and blocked both ways until a Pull resolves it explicitly.
    let diverged = entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&t3_id))
        .unwrap();
    assert!(diverged.local_present);
    assert_eq!(
        diverged.push_decision.action,
        ConversationReviewAction::Blocked
    );
    assert_eq!(
        diverged.pull_decision.action,
        ConversationReviewAction::Blocked
    );
    assert_eq!(
        diverged.pull_decision.blocker,
        Some(ConversationBlocker::UnknownEvidence)
    );
    assert_eq!(diverged.provider_thread_id.as_deref(), Some("local-v3"));
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
    analysis: &mallard_infrastructure::conversation_codec::CodexTranscriptAnalysis,
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

use super::harness::{sha256, PublishedFixture};
use super::scope::InstalledScope;
use mallard_domain::{
    compute_portable_conversation_tail, CloudThreadId, ConversationApplyKind,
    ConversationApplyOperation, ConversationCloudAnchor, ConversationFileRole,
    ConversationMaterializationApply, ConversationOperationId, ConversationPublicationApply,
    PlanId, PortableConversationTail, RootTailLayout, TailPartitionKind, TargetExpectation,
    PORTABLE_TAIL_CODEC_V1,
};
use mallard_infrastructure::codex_history_graph::resolve_codex_root;
use mallard_infrastructure::codex_standalone_builder::{
    decode_standalone_codex_root, encode_standalone_codex_root,
};
use mallard_infrastructure::conversation_codec::CodecLineage;
use mallard_infrastructure::conversation_materialization::{
    apply_conversation_cloud_publication, apply_conversation_materialization,
    ConversationCloudPublication, ConversationMaterializationRequest, ConversationTargetPayload,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn whole_thread_layout() -> RootTailLayout {
    RootTailLayout {
        codec: PORTABLE_TAIL_CODEC_V1.to_string(),
        partition_kind: TailPartitionKind::WholeThread,
        start_record_count: 0,
    }
}

/// The wire-form tail of a fixture publication: harness worlds publish
/// transcripts verbatim, so the folded state is form-identical and the wire
/// codec label matches the manifest column the Apply operation joins.
fn transcript_tail(transcript: &[u8]) -> PortableConversationTail {
    let analysis = analyze_codex_transcript(transcript);
    match analysis.lineage {
        CodecLineage::Complete { chain, .. } => {
            mallard_domain::compute_portable_conversation_tail_with_codec(
                mallard_domain::PORTABLE_TAIL_CODEC_WIRE_V1,
                TailPartitionKind::WholeThread,
                0,
                &chain,
            )
            .unwrap()
        }
        other => panic!("fixture transcript is not resumable: {other:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn install_active_branch(
    repository: &LocalStateRepository,
    scope: &InstalledScope,
    thread_id: &ThreadId,
    cloud_thread_id: &CloudThreadId,
    provider_thread_id: &str,
    transcript: &[u8],
    _source: &PublishedFixture,
    updated_at: u64,
) -> PathBuf {
    let sessions = scope.provider_root.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let transcript_path = sessions.join(format!("rollout-1-{provider_thread_id}.jsonl"));
    fs::write(&transcript_path, transcript).unwrap();
    let index_path = scope.provider_root.join("session_index.jsonl");
    fs::write(&index_path, merge_index_target(None, provider_thread_id)).unwrap();
    repository
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                installed_end: None,
                installed_subagents: Default::default(),
                path_origins_ref: None,
                host_id: scope.host_id.clone(),
                replica_id: scope.replica_id.clone(),
                local_project_id: scope.project_id.clone(),
                provider_profile_id: scope.profile_id.clone(),
                thread_id: thread_id.clone(),
                cloud_thread_id: Some(cloud_thread_id.clone()),
                forked_from: None,
                preserved: None,
                root_tail_layout: Some(whole_thread_layout()),
                provider_thread_id: Some(provider_thread_id.to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
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
    repository: &LocalStateRepository,
    scope: &InstalledScope,
    threads: &[ThreadSpec],
    bound_thread_id: &ThreadId,
    cloud_thread_id: &CloudThreadId,
    provider_thread_id: &str,
    local_transcript: &[u8],
    publisher: mallard_domain::PortableOrigin,
    operation_id: &str,
    updated_at: u64,
) -> mallard_domain::BundleSnapshot {
    let engine = world.engine();
    let current = engine.inspect(&world.bundle_id).unwrap();
    let head_token = world.current_head().unwrap();
    let request = super::harness::publish_request(
        world.bundle_id.clone(),
        threads,
        publisher,
        updated_at,
        Some(&current.manifest),
        mallard_infrastructure::bundle_engine::PublishExpectation::Match(head_token),
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
    let tail_after = transcript_tail(local_transcript);
    let tail_before = current
        .manifest
        .find_conversation(cloud_thread_id)
        .and_then(|(_, thread)| thread.portable_tail.clone());
    let planned_relation = if tail_before.is_some() {
        mallard_domain::TailClassification::LocalOutstanding
    } else {
        mallard_domain::TailClassification::MissingCloud
    };
    let config_before = repository.load_config().unwrap();
    let mut config_after = config_before.clone();
    config_after.revision = config_after.revision.saturating_add(1);
    let operation = ConversationApplyOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(format!("plan-{operation_id}")).unwrap()),
        plan_action_id: None,
        cloud: ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision: current.head.revision,
            commit_id: current.head.commit_id.clone(),
            manifest_sha256: current.head.manifest_sha256.clone(),
        },
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: codex(),
        thread_id: bound_thread_id.clone(),
        cloud_thread_id: cloud_thread_id.clone(),
        operation: ConversationApplyKind::CloudPublication(ConversationPublicationApply {
            export_settings_changed: false,
            candidate_commit_id: current.head.commit_id,
            candidate_manifest_sha256: current.head.manifest_sha256,
            artifact_sha256: sha256(local_transcript),
            tail_before,
            tail_after,
            planned_relation,
        }),
        provider_thread_id: Some(provider_thread_id.to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings.revision,
        expected_binding_revision: Some(binding.revision),
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        targets: Vec::new(),
        created_at: updated_at,
        updated_at,
    };
    let mut bindings_after = bindings.clone();
    bindings_after.revision = bindings_after.revision.saturating_add(1);
    let state =
        mallard_infrastructure::conversation_materialization::ConversationPublicationState {
            sync_config_before: config_before,
            sync_config_after: config_after,
            bindings_before: bindings,
            bindings_after,
        };
    apply_conversation_cloud_publication(repository, operation, state, || {
        engine
            .publish(request)
            .map(|published| ConversationCloudPublication {
                revision: published.snapshot.head.revision,
                commit_id: published.snapshot.head.commit_id,
                manifest_sha256: published.snapshot.head.manifest_sha256,
            })
    })
    .unwrap();
    assert!(!repository
        .root()
        .join("conversation_sync_journal.json")
        .exists());
    engine.inspect(&world.bundle_id).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn materialize_thread(
    repository: &LocalStateRepository,
    scope: &InstalledScope,
    world: &VariantWorld,
    thread_id: &ThreadId,
    cloud_thread_id: &CloudThreadId,
    destination_thread_id: &str,
    source_transcript: &[u8],
    operation_id: &str,
    updated_at: u64,
) -> PathBuf {
    let snapshot = world.engine().inspect(&world.bundle_id).unwrap();
    let source_analysis = analyze_codex_transcript(source_transcript);
    let source_cwd = source_analysis
        .source_cwd
        .as_deref()
        .expect("source fixture has a cwd");
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.jsonl");
    fs::write(&source_path, source_transcript).unwrap();
    let no_bases = |_: &str| Ok(None);
    let resolved = resolve_codex_root(&source_path, &no_bases).unwrap();
    let wire = encode_standalone_codex_root(
        &resolved,
        &source_cwd,
        None,
        None,
        &mut mallard_infrastructure::credential_redaction::shared().begin_run(),
        &mallard_infrastructure::session_path_origins::PathOrigins::default(),
    )
    .unwrap();
    let staged_path = directory.path().join("staged.jsonl");
    fs::write(&staged_path, &wire.root_bytes).unwrap();
    let staged = resolve_codex_root(&staged_path, &no_bases).unwrap();
    let rewritten = decode_standalone_codex_root(
        &staged,
        &destination_thread_id,
        &scope.project_root.to_string_lossy().into_owned(),
        None,
        None,
    )
    .unwrap()
    .root_bytes
    .to_vec();
    let analysis = analyze_codex_transcript(&rewritten);
    let (lineage_digest, semantic_digest, accepted_tail) = match analysis.lineage {
        CodecLineage::Complete {
            chain,
            lineage_digest,
            semantic_digest,
        } => (
            lineage_digest,
            semantic_digest,
            compute_portable_conversation_tail(TailPartitionKind::WholeThread, 0, &chain).unwrap(),
        ),
        other => panic!("incoming branch is not resumable: {other:?}"),
    };
    let sessions = scope.provider_root.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let transcript_path = sessions.join(format!("rollout-1-{destination_thread_id}.jsonl"));
    let index_path = scope.provider_root.join("session_index.jsonl");
    let existing_index = fs::read(&index_path).ok();
    let index = merge_index_target(existing_index.as_deref(), destination_thread_id);
    let bindings_before = repository.load_conversation_bindings().unwrap();
    let mut bindings_after = bindings_before.clone();
    let existing_binding = bindings_before.bindings.iter().find(|binding| {
        binding.host_id == scope.host_id
            && binding.replica_id == scope.replica_id
            && binding.local_project_id == scope.project_id
            && binding.provider_profile_id == scope.profile_id
            && binding.thread_id == *thread_id
    });
    let planned_relation = if existing_binding.is_some() {
        mallard_domain::TailClassification::CloudOutstanding
    } else {
        mallard_domain::TailClassification::MissingLocal
    };
    match bindings_after.bindings.iter_mut().find(|binding| {
        binding.host_id == scope.host_id
            && binding.replica_id == scope.replica_id
            && binding.local_project_id == scope.project_id
            && binding.provider_profile_id == scope.profile_id
            && binding.thread_id == *thread_id
    }) {
        Some(row) => {
            row.installed_end = Some(rewritten.len() as u64);
            row.provider_thread_id = Some(destination_thread_id.to_string());
            row.cloud_thread_id = Some(cloud_thread_id.clone());
            row.root_tail_layout = Some(whole_thread_layout());
            row.materialization_state = ConversationMaterializationState::Resumable;
            row.revision = row.revision.saturating_add(1);
            row.updated_at = updated_at;
        }
        None => bindings_after.bindings.push(LocalConversationBinding {
            installed_end: Some(rewritten.len() as u64),
            installed_subagents: Default::default(),
            path_origins_ref: None,
            host_id: scope.host_id.clone(),
            replica_id: scope.replica_id.clone(),
            local_project_id: scope.project_id.clone(),
            provider_profile_id: scope.profile_id.clone(),
            thread_id: thread_id.clone(),
            cloud_thread_id: Some(cloud_thread_id.clone()),
            forked_from: None,
            preserved: None,
            root_tail_layout: Some(whole_thread_layout()),
            provider_thread_id: Some(destination_thread_id.to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            revision: 0,
            created_at: updated_at,
            updated_at,
        }),
    }
    bindings_after.revision = bindings_after.revision.saturating_add(1);
    let bindings_document_revision = bindings_before.revision;
    let existing_binding_revision = existing_binding.map(|binding| binding.revision);
    let config_before = repository.load_config().unwrap();
    let mut config_after = config_before.clone();
    config_after.revision = config_after.revision.saturating_add(1);
    let operation = ConversationApplyOperation {
        operation_id: ConversationOperationId::parse(operation_id).unwrap(),
        plan_id: Some(PlanId::parse(format!("plan-{operation_id}")).unwrap()),
        plan_action_id: None,
        cloud: ConversationCloudAnchor {
            storage_id: world.storage_id.clone(),
            bundle_id: world.bundle_id.clone(),
            revision: snapshot.head.revision,
            commit_id: snapshot.head.commit_id.clone(),
            manifest_sha256: snapshot.head.manifest_sha256.clone(),
        },
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        provider: codex(),
        thread_id: thread_id.clone(),
        cloud_thread_id: cloud_thread_id.clone(),
        operation: ConversationApplyKind::ProviderMaterialization(
            ConversationMaterializationApply {
                planned_relation,
                accepted_tail_after: accepted_tail,
                pinned_lineage_digest: lineage_digest,
                pinned_semantic_digest: semantic_digest,
                keep_both: None,
                expected_config_revision: config_before.revision,
                sync_config_after: config_after,
                bindings_after,
            },
        ),
        provider_thread_id: Some(destination_thread_id.to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        expected_host_identity_revision: scope.host_identity_revision,
        expected_bindings_document_revision: bindings_document_revision,
        expected_binding_revision: existing_binding_revision,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        targets: Vec::new(),
        created_at: updated_at,
        updated_at,
    };
    let request = ConversationMaterializationRequest {
        subagents: vec![],
        transcript: mallard_infrastructure::verified_transcript::VerifiedTranscript::analyze(
            &mallard_infrastructure::providers::codex::CodexAdapter::new(),
            rewritten.clone().into(),
        )
        .unwrap(),
        operation,
        checked_transcript: match target_expectation(&transcript_path) {
            TargetExpectation::Sha256(_) => Some(fs::read(&transcript_path).unwrap()),
            TargetExpectation::Absent => None,
        },
        targets: vec![
            ConversationTargetPayload {
                target_path: transcript_path.clone(),
                role: ConversationFileRole::Transcript,
                expected: target_expectation(&transcript_path),
                index_rows: vec![],
                bytes: rewritten.into(),
            },
            ConversationTargetPayload {
                target_path: index_path.clone(),
                role: ConversationFileRole::ProviderIndexFragment,
                expected: target_expectation(&index_path),
                index_rows: vec![destination_thread_id.to_string()],
                bytes: index.into(),
            },
        ],
    };
    let analysis_before = mallard_infrastructure::sync_metrics::analysis_work();
    apply_conversation_materialization(repository, request).unwrap();
    assert_eq!(
        mallard_infrastructure::sync_metrics::analysis_work()
            .delta_since(analysis_before)
            .passes,
        0,
        "materialization must reuse the verified transcript analysis",
    );
    transcript_path
}

fn merge_index_target(existing: Option<&[u8]>, thread_id: &str) -> Vec<u8> {
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
            "thread_name": "Thread continuation fixture",
            "updated_at": "2026-07-18T00:00:00.000000Z"
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

use std::collections::BTreeSet;

fn assert_local_branch_state(
    repository: &LocalStateRepository,
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
