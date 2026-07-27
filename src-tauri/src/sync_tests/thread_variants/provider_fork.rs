//! End-to-end provider-native fork: fork the Codex session, push the fork as
//! a new cloud thread, pull it on a second host, reconstruct the Codex
//! SQLite thread index there, and prove Codex-side discovery and capture
//! still work on both hosts afterwards.

use super::harness::{chain, publish_request, sha256, ThreadSpec, VariantWorld};
use super::recovery::{install_scope, install_scope_for_provider};
use crate::project_sync_v3::bundle_engine::{
    HeadToken, PublishBundleRequest, PublishContent, PublishFile,
};
use crate::project_sync_v3::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage,
};
use crate::project_sync_v3::conversation_materialization::{
    apply_conversation_cloud_publication, apply_conversation_materialization,
    ConversationCloudPublication, ConversationMaterializationRequest, ConversationTargetPayload,
    NoConversationFailpoints,
};
use crate::project_sync_v3::domain::{
    classify_conversation_relation, generated_uuid_v4, ConversationFileRole,
    ConversationMaterializationState, ConversationOperationId, ConversationPlanDirection,
    ConversationRelation, ConversationSyncOperation, ConversationSyncPhase,
    LocalConversationBinding, PlanId, Provider, TargetExpectation, ThreadId,
};
use crate::project_sync_v3::domain::{
    conversation_resource_id, ApplyPolicy, BundleHead, BundleIdentity, BundleKind, BundleRecipe,
    CapturedWith, ConversationFileRef, ConversationLineageStatus, ConversationThread, LogicalPath,
    PortableOrigin, Provenance, RecipeEntry, ResourceDescriptor, ResourceKind, ResourceScope,
};
use crate::project_sync_v3::provider_capture::{
    capture_selected, discover_project, CaptureRequest,
};
use crate::project_sync_v3::provider_fork::{
    claude_project_dir_name, fork_claude_session, fork_codex_session,
    reconstruct_codex_sqlite_index, rewrite_claude_transcript, ClaudeForkRequest, CodexForkRequest,
    CodexIndexOutcome, ProviderCliInvocation, ProviderCliOutput, ProviderCliRunner,
    ProviderForkMethod,
};
use chrono::Utc;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A host without the Codex binary: every CLI attempt fails, forcing the
/// verified codec fallback. The CLI-success path is covered by the module
/// tests with a stub binary.
struct RefusingCli;

impl ProviderCliRunner for RefusingCli {
    fn run(&self, _invocation: &ProviderCliInvocation) -> Result<ProviderCliOutput, String> {
        Err("codex binary is not installed on this host".to_string())
    }
}

fn install_codex_state_fixture(codex_home: &Path) {
    fs::create_dir_all(codex_home).unwrap();
    let connection = rusqlite::Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT,
                forked_from_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT NOT NULL,
                PRIMARY KEY (parent_thread_id, child_thread_id)
            );",
        )
        .unwrap();
}

fn indexed_rollout_path(codex_home: &Path, thread_id: &str) -> Option<String> {
    let connection = rusqlite::Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    connection
        .query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            [thread_id],
            |row| row.get(0),
        )
        .ok()
}

fn complete_chain(
    bytes: &[u8],
) -> Vec<crate::project_sync_v3::domain::CanonicalConversationMessage> {
    match analyze_codex_transcript(bytes).lineage {
        CodecLineage::Complete { chain, .. } => chain,
        other => panic!("fixture transcript is not resumable: {other:?}"),
    }
}

#[test]
fn codex_fork_push_pull_reconstruction_round_trip() {
    let world = VariantWorld::new();
    let scope_a = install_scope(&world.repo_a, &world.bundle_id, "fork-cli-a", "replica-a");
    let scope_b = install_scope(&world.repo_b, &world.bundle_id, "fork-cli-b", "replica-b");
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let cloud_id = ThreadId::parse("thread-origin").unwrap();
    let forked_cloud_id = ThreadId::parse("thread-forked-local").unwrap();

    // Cloud thread t published by host B; host A holds a diverged local
    // session for it under a provider-native UUID.
    let cloud_portable = chain(
        "shared-source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-cloud", "cloud tail"),
        ],
    );
    let local_portable = chain(
        "shared-source-thread",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-local", "local tail"),
        ],
    );
    let published = world.publish(
        &[ThreadSpec::new(
            cloud_id.as_str(),
            "shared-source-thread",
            cloud_portable.clone(),
            origin_b.clone(),
            10,
        )],
        origin_b,
        10,
    );
    let source_provider_id = generated_uuid_v4().unwrap();
    let local_transcript = rewrite_codex_transcript(
        &local_portable,
        "shared-source-thread",
        &source_provider_id,
        "/tmp/project",
        scope_a.project_root.to_str().unwrap(),
    )
    .unwrap();
    fs::create_dir_all(scope_a.provider_root.join("sessions")).unwrap();
    let source_session_path = scope_a
        .provider_root
        .join(format!("sessions/{source_provider_id}.jsonl"));
    fs::write(&source_session_path, &local_transcript).unwrap();
    let (local_lineage, local_semantic) = match analyze_codex_transcript(&local_transcript).lineage
    {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        other => panic!("local fixture is not resumable: {other:?}"),
    };
    world
        .repo_a
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: scope_a.host_id.clone(),
                replica_id: scope_a.replica_id.clone(),
                local_project_id: scope_a.project_id.clone(),
                provider_profile_id: scope_a.profile_id.clone(),
                thread_id: cloud_id.clone(),
                provider_thread_id: Some(source_provider_id.clone()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(&local_transcript)),
                last_observed_head_message_id: Some("m-local".to_string()),
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

    // Step 1 — codex fork: the provider session is forked on host A. The CLI
    // is attempted first and refused, so the codec fallback copies the
    // rollout and reconstructs the SQLite thread index.
    install_codex_state_fixture(&scope_a.provider_root);
    let fork_request = CodexForkRequest {
        codex_home: scope_a.provider_root.clone(),
        project_root: scope_a.project_root.clone(),
        source_thread_id: source_provider_id.clone(),
        new_thread_id: generated_uuid_v4().unwrap(),
        source_transcript: local_transcript.clone(),
        binary: PathBuf::from("codex"),
        timeout: Duration::from_secs(5),
        allow_cli: true,
        now: Utc::now(),
    };
    let forked = fork_codex_session(&RefusingCli, &fork_request).unwrap();
    assert!(matches!(
        forked.method,
        ProviderForkMethod::CodexCodec {
            index: CodexIndexOutcome::Inserted
        }
    ));
    assert!(!forked.already_existed);
    let forked_transcript = fs::read(&forked.transcript_path).unwrap();
    assert_eq!(
        classify_conversation_relation(
            Some(&complete_chain(&forked_transcript)),
            Some(&complete_chain(&local_transcript)),
        ),
        ConversationRelation::Identical,
        "the provider fork copies content exactly",
    );
    assert_eq!(
        indexed_rollout_path(&scope_a.provider_root, &forked.provider_thread_id).as_deref(),
        Some(forked.transcript_path.to_string_lossy().as_ref()),
        "host A's SQLite index knows the forked thread",
    );
    // A crash between fork and journaling retries into the same fork.
    let retried = fork_codex_session(&RefusingCli, &fork_request).unwrap();
    assert!(retried.already_existed);
    assert_eq!(retried.provider_thread_id, forked.provider_thread_id);

    // Step 2 — push: publish the fork as cloud thread t' and hand the
    // Mallard binding over to the forked provider session.
    let bindings_before_push = world.repo_a.load_conversation_bindings().unwrap();
    let mut forked_spec = ThreadSpec::new(
        forked_cloud_id.as_str(),
        &forked.provider_thread_id,
        local_portable.clone(),
        origin_a.clone(),
        20,
    );
    forked_spec.forked_from = Some(cloud_id.clone());
    let push_request = publish_request(
        world.bundle_id.clone(),
        &[
            ThreadSpec::new(
                cloud_id.as_str(),
                "shared-source-thread",
                cloud_portable.clone(),
                origin_a.clone(),
                10,
            ),
            forked_spec,
        ],
        origin_a,
        20,
        Some(&published.snapshot.manifest),
        Some((
            published.snapshot.head.clone(),
            published.head_token.clone(),
        )),
    );
    let push_operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-provider-fork-push").unwrap(),
        plan_id: Some(PlanId::parse("plan-provider-fork-push").unwrap()),
        direction: ConversationPlanDirection::Push,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: published.snapshot.head.latest.generation,
        commit_id: published.snapshot.head.latest.commit_id.clone(),
        manifest_sha256: published.snapshot.head.latest.manifest_sha256.clone(),
        host_id: scope_a.host_id.clone(),
        replica_id: scope_a.replica_id.clone(),
        local_project_id: scope_a.project_id.clone(),
        provider_profile_id: scope_a.profile_id.clone(),
        provider: Provider::Codex,
        thread_id: cloud_id.clone(),
        old_thread_id: Some(cloud_id.clone()),
        rekey_committed: false,
        result_thread_id: forked_cloud_id.clone(),
        forked_from_provider_thread_id: Some(source_provider_id.clone()),
        provider_thread_id: Some(forked.provider_thread_id.clone()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(local_lineage.clone()),
        semantic_digest: Some(local_semantic.clone()),
        expected_host_identity_revision: scope_a.host_identity_revision,
        expected_bindings_document_revision: bindings_before_push.revision,
        expected_binding_revision: Some(bindings_before_push.bindings[0].revision),
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(&forked_transcript)),
        targets: Vec::new(),
        created_at: 20,
        updated_at: 20,
    };
    let engine = world.engine();
    apply_conversation_cloud_publication(
        &world.repo_a,
        push_operation,
        || {
            engine
                .publish(push_request)
                .map(|outcome| ConversationCloudPublication {
                    generation: outcome.snapshot.head.latest.generation,
                    commit_id: outcome.snapshot.head.latest.commit_id,
                    manifest_sha256: outcome.snapshot.head.latest.manifest_sha256,
                })
        },
        || Ok(None),
        &mut NoConversationFailpoints,
    )
    .unwrap();
    let after_push = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(after_push.bindings.len(), 1);
    assert_eq!(after_push.bindings[0].thread_id, forked_cloud_id);
    assert_eq!(
        after_push.bindings[0].provider_thread_id.as_deref(),
        Some(forked.provider_thread_id.as_str()),
        "the binding hands over to the provider-forked session",
    );
    let manifest = world.engine().inspect(&world.bundle_id).unwrap().manifest;
    assert_eq!(manifest.conversations.len(), 2);
    assert_eq!(
        manifest.conversations[&forked_cloud_id].forked_from,
        Some(cloud_id.clone())
    );

    // Step 3 — pull on host B: materialize the forked thread as a fresh
    // provider session under a UUID minted for host B.
    let fetched = world.engine().fetch(&world.bundle_id).unwrap();
    let forked_thread = &fetched.snapshot.manifest.conversations[&forked_cloud_id];
    let forked_cloud_bytes = &fetched.files[&forked_thread.files[0].logical_path];
    let destination_provider_id = generated_uuid_v4().unwrap();
    let materialized_transcript = rewrite_codex_transcript(
        forked_cloud_bytes,
        "shared-source-thread",
        &destination_provider_id,
        "/tmp/project",
        scope_b.project_root.to_str().unwrap(),
    )
    .unwrap();
    let (incoming_lineage, incoming_semantic) =
        match analyze_codex_transcript(&materialized_transcript).lineage {
            CodecLineage::Complete {
                lineage_digest,
                semantic_digest,
                ..
            } => (lineage_digest, semantic_digest),
            other => panic!("incoming fixture is not resumable: {other:?}"),
        };
    fs::create_dir_all(scope_b.provider_root.join("sessions")).unwrap();
    let materialized_path = scope_b
        .provider_root
        .join(format!("sessions/{destination_provider_id}.jsonl"));
    let index_path = scope_b.provider_root.join("session_index.jsonl");
    let index_row = format!(
        "{{\"id\":\"{destination_provider_id}\",\"cwd\":{}}}\n",
        serde_json::to_string(scope_b.project_root.to_str().unwrap()).unwrap(),
    )
    .into_bytes();
    let bindings_b = world.repo_b.load_conversation_bindings().unwrap();
    let pull_operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-provider-fork-pull").unwrap(),
        plan_id: Some(PlanId::parse("plan-provider-fork-pull").unwrap()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: fetched.snapshot.head.latest.generation,
        commit_id: fetched.snapshot.head.latest.commit_id.clone(),
        manifest_sha256: fetched.snapshot.head.latest.manifest_sha256.clone(),
        host_id: scope_b.host_id.clone(),
        replica_id: scope_b.replica_id.clone(),
        local_project_id: scope_b.project_id.clone(),
        provider_profile_id: scope_b.profile_id.clone(),
        provider: Provider::Codex,
        thread_id: forked_cloud_id.clone(),
        old_thread_id: None,
        rekey_committed: false,
        result_thread_id: forked_cloud_id.clone(),
        forked_from_provider_thread_id: None,
        provider_thread_id: Some(destination_provider_id.clone()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: Some(incoming_lineage),
        semantic_digest: Some(incoming_semantic),
        expected_host_identity_revision: scope_b.host_identity_revision,
        expected_bindings_document_revision: bindings_b.revision,
        expected_binding_revision: None,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: None,
        targets: Vec::new(),
        created_at: 30,
        updated_at: 30,
    };
    apply_conversation_materialization(
        &world.repo_b,
        ConversationMaterializationRequest {
            operation: pull_operation,
            targets: vec![
                ConversationTargetPayload {
                    target_path: materialized_path.clone(),
                    role: ConversationFileRole::Transcript,
                    expected: TargetExpectation::Absent,
                    bytes: materialized_transcript.clone(),
                },
                ConversationTargetPayload {
                    target_path: index_path.clone(),
                    role: ConversationFileRole::ProviderIndexFragment,
                    expected: TargetExpectation::Absent,
                    bytes: index_row,
                },
            ],
        },
        &mut NoConversationFailpoints,
    )
    .unwrap();
    let bindings_b = world.repo_b.load_conversation_bindings().unwrap();
    assert_eq!(bindings_b.bindings.len(), 1);
    assert_eq!(bindings_b.bindings[0].thread_id, forked_cloud_id);
    assert_eq!(
        bindings_b.bindings[0].provider_thread_id.as_deref(),
        Some(destination_provider_id.as_str())
    );

    // Step 4 — reconstruction on host B: the materialized thread is inserted
    // into B's Codex SQLite index, idempotently.
    install_codex_state_fixture(&scope_b.provider_root);
    let reconstructed = reconstruct_codex_sqlite_index(
        &scope_b.provider_root,
        &destination_provider_id,
        None,
        &materialized_path,
        Utc::now().timestamp(),
    );
    assert_eq!(reconstructed, CodexIndexOutcome::Inserted);
    assert_eq!(
        reconstruct_codex_sqlite_index(
            &scope_b.provider_root,
            &destination_provider_id,
            None,
            &materialized_path,
            Utc::now().timestamp(),
        ),
        CodexIndexOutcome::AlreadyIndexed,
    );
    assert_eq!(
        indexed_rollout_path(&scope_b.provider_root, &destination_provider_id).as_deref(),
        Some(materialized_path.to_string_lossy().as_ref()),
    );

    // Step 5 — codex works again, on both hosts: discovery and capture see
    // the forked sessions, and the captured chains match the cloud fork.
    let forked_cloud_chain = complete_chain(forked_cloud_bytes);
    for (scope, provider_id) in [
        (&scope_a, &forked.provider_thread_id),
        (&scope_b, &destination_provider_id),
    ] {
        let mut capture_request = CaptureRequest::for_project(scope.project_root.clone());
        capture_request.codex_home = Some(scope.provider_root.clone());
        let resource_id = format!("codex:session:{provider_id}");
        let discovered = discover_project(&capture_request).unwrap();
        let session = discovered
            .resources
            .iter()
            .find(|resource| resource.resource_id == resource_id)
            .unwrap_or_else(|| panic!("codex did not discover '{resource_id}'"));
        assert!(session.blocked_reason.is_none());
        let captured =
            capture_selected(&capture_request, &BTreeSet::from([resource_id.clone()])).unwrap();
        let conversation = captured.conversations.get(&resource_id).unwrap();
        assert!(conversation.lineage.is_complete());
        assert_eq!(
            classify_conversation_relation(conversation.lineage.chain(), Some(&forked_cloud_chain),),
            ConversationRelation::Identical,
            "captured fork on '{}' matches the published fork thread",
            scope.project_id,
        );
    }

    // Loop freedom holds across the provider fork: the handed-over session
    // is Identical to its cloud thread, so a second sync proposes nothing.
    assert_eq!(
        classify_conversation_relation(
            Some(&complete_chain(&forked_transcript)),
            Some(&forked_cloud_chain),
        ),
        ConversationRelation::Identical
    );
}

/// Simulates `claude --resume <src> --fork-session --session-id <new> -p …`:
/// copies the source transcript to the new session ID inside the same
/// project directory, rewriting the session-id fields the way the CLI does.
struct CopyingClaudeCli;

impl ProviderCliRunner for CopyingClaudeCli {
    fn run(&self, invocation: &ProviderCliInvocation) -> Result<ProviderCliOutput, String> {
        assert_eq!(invocation.args[0], "--resume");
        assert_eq!(invocation.args[2], "--fork-session");
        assert_eq!(invocation.args[3], "--session-id");
        assert_eq!(invocation.args[5], "-p");
        let home = invocation
            .envs
            .iter()
            .find(|(key, _)| key == "CLAUDE_CONFIG_DIR")
            .map(|(_, value)| PathBuf::from(value))
            .expect("claude fork must scope CLAUDE_CONFIG_DIR");
        let project_dir = home
            .join("projects")
            .join(claude_project_dir_name(&invocation.cwd));
        let source = project_dir.join(format!("{}.jsonl", invocation.args[1]));
        let bytes = fs::read(&source).map_err(|error| format!("stub read: {error}"))?;
        let cwd = invocation.cwd.to_string_lossy();
        let rewritten = rewrite_claude_transcript(
            &bytes,
            &invocation.args[1],
            &invocation.args[4],
            &cwd,
            &cwd,
        )?;
        fs::write(
            project_dir.join(format!("{}.jsonl", invocation.args[4])),
            rewritten,
        )
        .map_err(|error| format!("stub write: {error}"))?;
        Ok(ProviderCliOutput {
            success: true,
            stdout: "ok".to_string(),
            stderr: String::new(),
        })
    }
}

fn claude_transcript(session_id: &str, cwd: &str, tail: &str) -> Vec<u8> {
    let rows = [
        serde_json::json!({
            "type": "user",
            "sessionId": session_id,
            "cwd": cwd,
            "message": {"role": "user", "content": "base question"}
        }),
        serde_json::json!({
            "type": "assistant",
            "sessionId": session_id,
            "cwd": cwd,
            "message": {"role": "assistant", "content": "base answer"}
        }),
        serde_json::json!({
            "type": "user",
            "sessionId": session_id,
            "cwd": cwd,
            "message": {"role": "user", "content": tail}
        }),
    ];
    let mut bytes = Vec::new();
    for row in rows {
        bytes.extend_from_slice(serde_json::to_string(&row).unwrap().as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// Claude threads publish as stored bytes with Unknown lineage: there is no
/// Claude lineage codec, so the manifest records the transcript without
/// chain metadata.
fn claude_publish_request(
    world: &VariantWorld,
    threads: &[(ThreadId, String, Vec<u8>, Option<ThreadId>, u64)],
    publisher: PortableOrigin,
    updated_at: u64,
    observed_head: Option<(BundleHead, HeadToken)>,
) -> PublishBundleRequest {
    const CLAUDE_CODEC: &str = "claude.lineage.unavailable.v1";
    let mut recipe = BundleRecipe::default();
    let mut resources = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut conversations = BTreeMap::new();
    for (thread_id, session_id, bytes, forked_from, thread_updated_at) in threads {
        let resource_id = conversation_resource_id(Provider::Claude, thread_id).unwrap();
        let logical_path = LogicalPath::parse(format!(
            "state/conversations/claude/{thread_id}/transcript.jsonl"
        ))
        .unwrap();
        conversations.insert(
            thread_id.clone(),
            ConversationThread {
                thread_id: thread_id.clone(),
                provider: Provider::Claude,
                display_title: Some("Claude fork conversation".to_string()),
                resource_id: resource_id.clone(),
                files: vec![ConversationFileRef {
                    logical_path: logical_path.clone(),
                    role: ConversationFileRole::Transcript,
                }],
                lineage_status: ConversationLineageStatus::Unknown {
                    reason_code: "unsupported_codec".to_string(),
                },
                lineage_codec: CLAUDE_CODEC.to_string(),
                head_message_id: None,
                message_count: None,
                lineage_digest: None,
                semantic_digest: None,
                forked_from: forked_from.clone(),
                source_thread_id: Some(session_id.clone()),
                created_by_origin: Some(publisher.clone()),
                last_updated_by_origin: Some(publisher.clone()),
                created_at: 1,
                updated_at: *thread_updated_at,
            },
        );
        resources.insert(
            resource_id.clone(),
            ResourceDescriptor {
                resource_id: resource_id.clone(),
                kind: ResourceKind::ClaudeConversation,
                provider: Some(Provider::Claude),
                scope: ResourceScope::ProviderState,
                display_name: "Claude fork conversation".to_string(),
                provenance: Provenance::Unknown,
                apply_policy: ApplyPolicy::SafeFile,
                relative_cwd: Some(".".to_string()),
                codec_version: 1,
                metadata: BTreeMap::new(),
            },
        );
        recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
        files.insert(
            logical_path,
            PublishFile::Captured {
                resource_id,
                bytes: bytes.clone(),
                source_mtime: *thread_updated_at,
                mode: Some(0o600),
            },
        );
    }
    PublishBundleRequest {
        identity: BundleIdentity {
            bundle_id: world.bundle_id.clone(),
            display_name: "Claude fork bundle".to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        },
        recipe,
        captured_with: CapturedWith {
            app_version: "integration-test".to_string(),
            codex_version: None,
            claude_version: Some("fixture-v1".to_string()),
            codec_versions: BTreeMap::from([(CLAUDE_CODEC.to_string(), 1)]),
        },
        published_by_origin: publisher,
        content: PublishContent {
            conversations,
            resources,
            files,
            directories: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        },
        observed_head,
        updated_at,
    }
}

#[test]
fn claude_fork_push_pull_reconstruction_round_trip() {
    let world = VariantWorld::new();
    let scope_a = install_scope_for_provider(
        &world.repo_a,
        &world.bundle_id,
        "claude-fork-a",
        "replica-a",
        Provider::Claude,
    );
    let scope_b = install_scope_for_provider(
        &world.repo_b,
        &world.bundle_id,
        "claude-fork-b",
        "replica-b",
        Provider::Claude,
    );
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let cloud_id = ThreadId::parse("thread-claude-origin").unwrap();
    let forked_cloud_id = ThreadId::parse("thread-claude-fork").unwrap();

    // Cloud thread t published by host B; host A holds a diverged local
    // Claude session for it.
    let cloud_bytes = claude_transcript("shared-claude-session", "/tmp/project", "cloud tail");
    let published = {
        let request = claude_publish_request(
            &world,
            &[(
                cloud_id.clone(),
                "shared-claude-session".to_string(),
                cloud_bytes.clone(),
                None,
                10,
            )],
            origin_b,
            10,
            None,
        );
        world.engine().publish(request).unwrap()
    };
    let source_session_id = generated_uuid_v4().unwrap();
    let project_dir_a = scope_a
        .provider_root
        .join("projects")
        .join(claude_project_dir_name(&scope_a.project_root));
    fs::create_dir_all(&project_dir_a).unwrap();
    let local_bytes = rewrite_claude_transcript(
        &claude_transcript("shared-claude-session", "/tmp/project", "local tail"),
        "shared-claude-session",
        &source_session_id,
        "/tmp/project",
        scope_a.project_root.to_str().unwrap(),
    )
    .unwrap();
    fs::write(
        project_dir_a.join(format!("{source_session_id}.jsonl")),
        &local_bytes,
    )
    .unwrap();
    world
        .repo_a
        .mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: scope_a.host_id.clone(),
                replica_id: scope_a.replica_id.clone(),
                local_project_id: scope_a.project_id.clone(),
                provider_profile_id: scope_a.profile_id.clone(),
                thread_id: cloud_id.clone(),
                provider_thread_id: Some(source_session_id.clone()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(sha256(&local_bytes)),
                last_observed_head_message_id: None,
                last_observed_lineage_digest: None,
                last_observed_semantic_digest: None,
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

    // Step 1 — claude fork: the provider CLI forks the local session under a
    // pinned UUID. There is deliberately no non-CLI fallback for Claude.
    let fork_request = ClaudeForkRequest {
        claude_home: scope_a.provider_root.clone(),
        project_root: scope_a.project_root.clone(),
        source_session_id: source_session_id.clone(),
        new_session_id: generated_uuid_v4().unwrap(),
        binary: PathBuf::from("claude"),
        timeout: Duration::from_secs(5),
    };
    let forked = fork_claude_session(&CopyingClaudeCli, &fork_request).unwrap();
    assert_eq!(forked.method, ProviderForkMethod::ClaudeCli);
    assert!(!forked.already_existed);
    let forked_bytes = fs::read(&forked.transcript_path).unwrap();
    assert!(String::from_utf8_lossy(&forked_bytes).contains(&fork_request.new_session_id));
    let retried = fork_claude_session(&CopyingClaudeCli, &fork_request).unwrap();
    assert!(
        retried.already_existed,
        "pinned UUID makes retries idempotent"
    );

    // Step 2 — push: publish the fork as cloud thread t' (portable cwd) and
    // hand the binding over to the forked session.
    let portable_fork_bytes = rewrite_claude_transcript(
        &forked_bytes,
        &forked.provider_thread_id,
        &forked.provider_thread_id,
        scope_a.project_root.to_str().unwrap(),
        "/tmp/project",
    )
    .unwrap();
    let bindings_before_push = world.repo_a.load_conversation_bindings().unwrap();
    let push_request = claude_publish_request(
        &world,
        &[
            (
                cloud_id.clone(),
                "shared-claude-session".to_string(),
                cloud_bytes.clone(),
                None,
                10,
            ),
            (
                forked_cloud_id.clone(),
                forked.provider_thread_id.clone(),
                portable_fork_bytes.clone(),
                Some(cloud_id.clone()),
                20,
            ),
        ],
        origin_a,
        20,
        Some((
            published.snapshot.head.clone(),
            published.head_token.clone(),
        )),
    );
    let push_operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-claude-fork-push").unwrap(),
        plan_id: Some(PlanId::parse("plan-claude-fork-push").unwrap()),
        direction: ConversationPlanDirection::Push,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: published.snapshot.head.latest.generation,
        commit_id: published.snapshot.head.latest.commit_id.clone(),
        manifest_sha256: published.snapshot.head.latest.manifest_sha256.clone(),
        host_id: scope_a.host_id.clone(),
        replica_id: scope_a.replica_id.clone(),
        local_project_id: scope_a.project_id.clone(),
        provider_profile_id: scope_a.profile_id.clone(),
        provider: Provider::Claude,
        thread_id: cloud_id.clone(),
        old_thread_id: Some(cloud_id.clone()),
        rekey_committed: false,
        result_thread_id: forked_cloud_id.clone(),
        forked_from_provider_thread_id: Some(source_session_id.clone()),
        provider_thread_id: Some(forked.provider_thread_id.clone()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: None,
        semantic_digest: None,
        expected_host_identity_revision: scope_a.host_identity_revision,
        expected_bindings_document_revision: bindings_before_push.revision,
        expected_binding_revision: Some(bindings_before_push.bindings[0].revision),
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: Some(sha256(&forked_bytes)),
        targets: Vec::new(),
        created_at: 20,
        updated_at: 20,
    };
    let engine = world.engine();
    apply_conversation_cloud_publication(
        &world.repo_a,
        push_operation,
        || {
            engine
                .publish(push_request)
                .map(|outcome| ConversationCloudPublication {
                    generation: outcome.snapshot.head.latest.generation,
                    commit_id: outcome.snapshot.head.latest.commit_id,
                    manifest_sha256: outcome.snapshot.head.latest.manifest_sha256,
                })
        },
        || Ok(None),
        &mut NoConversationFailpoints,
    )
    .unwrap();
    let after_push = world.repo_a.load_conversation_bindings().unwrap();
    assert_eq!(after_push.bindings.len(), 1);
    assert_eq!(after_push.bindings[0].thread_id, forked_cloud_id);
    assert_eq!(
        after_push.bindings[0].provider_thread_id.as_deref(),
        Some(forked.provider_thread_id.as_str())
    );
    let manifest = world.engine().inspect(&world.bundle_id).unwrap().manifest;
    assert_eq!(
        manifest.conversations[&forked_cloud_id].forked_from,
        Some(cloud_id.clone())
    );

    // Step 3 — pull on host B: rewrite the fork for host B's project and
    // materialize it through the journal (Claude: one transcript, no index).
    let fetched = world.engine().fetch(&world.bundle_id).unwrap();
    let forked_thread = &fetched.snapshot.manifest.conversations[&forked_cloud_id];
    let forked_cloud_bytes = &fetched.files[&forked_thread.files[0].logical_path];
    let destination_session_id = generated_uuid_v4().unwrap();
    let materialized_bytes = rewrite_claude_transcript(
        forked_cloud_bytes,
        &forked.provider_thread_id,
        &destination_session_id,
        "/tmp/project",
        scope_b.project_root.to_str().unwrap(),
    )
    .unwrap();
    let project_dir_b = scope_b
        .provider_root
        .join("projects")
        .join(claude_project_dir_name(&scope_b.project_root));
    fs::create_dir_all(&project_dir_b).unwrap();
    let materialized_path = project_dir_b.join(format!("{destination_session_id}.jsonl"));
    let bindings_b = world.repo_b.load_conversation_bindings().unwrap();
    let pull_operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse("operation-claude-fork-pull").unwrap(),
        plan_id: Some(PlanId::parse("plan-claude-fork-pull").unwrap()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: world.storage_id.clone(),
        bundle_id: world.bundle_id.clone(),
        generation: fetched.snapshot.head.latest.generation,
        commit_id: fetched.snapshot.head.latest.commit_id.clone(),
        manifest_sha256: fetched.snapshot.head.latest.manifest_sha256.clone(),
        host_id: scope_b.host_id.clone(),
        replica_id: scope_b.replica_id.clone(),
        local_project_id: scope_b.project_id.clone(),
        provider_profile_id: scope_b.profile_id.clone(),
        provider: Provider::Claude,
        thread_id: forked_cloud_id.clone(),
        old_thread_id: None,
        rekey_committed: false,
        result_thread_id: forked_cloud_id.clone(),
        forked_from_provider_thread_id: None,
        provider_thread_id: Some(destination_session_id.clone()),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: None,
        semantic_digest: None,
        expected_host_identity_revision: scope_b.host_identity_revision,
        expected_bindings_document_revision: bindings_b.revision,
        expected_binding_revision: None,
        expected_profile_revision: 0,
        expected_project_binding_revision: 0,
        expected_local_digest: None,
        targets: Vec::new(),
        created_at: 30,
        updated_at: 30,
    };
    apply_conversation_materialization(
        &world.repo_b,
        ConversationMaterializationRequest {
            operation: pull_operation,
            targets: vec![ConversationTargetPayload {
                target_path: materialized_path.clone(),
                role: ConversationFileRole::Transcript,
                expected: TargetExpectation::Absent,
                bytes: materialized_bytes.clone(),
            }],
        },
        &mut NoConversationFailpoints,
    )
    .unwrap();
    let bindings_b = world.repo_b.load_conversation_bindings().unwrap();
    assert_eq!(bindings_b.bindings.len(), 1);
    assert_eq!(bindings_b.bindings[0].thread_id, forked_cloud_id);
    assert_eq!(
        bindings_b.bindings[0].provider_thread_id.as_deref(),
        Some(destination_session_id.as_str())
    );

    // Step 4 — reconstruction for Claude is placement: the transcript sits
    // exactly where Claude's project-scoped session lookup expects it, under
    // the destination UUID, with no trace of the source session id field.
    assert_eq!(fs::read(&materialized_path).unwrap(), materialized_bytes);
    assert_eq!(
        materialized_path,
        scope_b
            .provider_root
            .join("projects")
            .join(claude_project_dir_name(&scope_b.project_root))
            .join(format!("{destination_session_id}.jsonl")),
    );
    let materialized_text = String::from_utf8(materialized_bytes.clone()).unwrap();
    assert!(!materialized_text.contains(forked.provider_thread_id.as_str()));
    assert!(materialized_text.contains(scope_b.project_root.to_str().unwrap()));

    // Step 5 — claude works again on both hosts: discovery and capture see
    // the forked and materialized sessions with the right identities.
    for (scope, session_id, expected_bytes) in [
        (&scope_a, &forked.provider_thread_id, &forked_bytes),
        (&scope_b, &destination_session_id, &materialized_bytes),
    ] {
        let mut capture_request = CaptureRequest::for_project(scope.project_root.clone());
        capture_request.claude_home = Some(scope.provider_root.clone());
        let resource_id = format!("claude:session:{session_id}");
        let discovered = discover_project(&capture_request).unwrap();
        let session = discovered
            .resources
            .iter()
            .find(|resource| resource.resource_id == resource_id)
            .unwrap_or_else(|| panic!("claude did not discover '{resource_id}'"));
        assert!(session.blocked_reason.is_none());
        let captured =
            capture_selected(&capture_request, &BTreeSet::from([resource_id.clone()])).unwrap();
        let file = captured
            .files
            .values()
            .find(|file| file.resource_id == resource_id)
            .unwrap_or_else(|| panic!("claude capture missed '{resource_id}'"));
        assert_eq!(&file.bytes, expected_bytes, "captured bytes are intact");
    }
}
