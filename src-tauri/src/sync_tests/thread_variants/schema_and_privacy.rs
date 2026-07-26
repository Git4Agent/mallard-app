use super::harness::{chain, ThreadSpec, VariantWorld};
use crate::project_sync_v3::bundle_engine::{BundleObjectStore, PublishBundleRequest};
use crate::project_sync_v3::domain::{
    BundleId, BundleIdentity, BundleKind, CapturedWith,
    ConversationFileRef, ConversationFileRole, ConversationLineageStatus,
    ConversationMaterializationState, ConversationPlanActionType, ConversationPlanDirection,
    ConversationRelation, LocalConversationBinding, LogicalPath, OriginHostAlias, Provider,
    ReplicaId, ThreadId, Tombstone, TombstoneTarget, MAX_FILE_BYTES,
};
use crate::project_sync_v3::provider_capture::CapturedResources;
use std::collections::BTreeMap;
use std::fs;

#[test]
fn host_replica_alias_rotation_and_bundle_privacy_invariants_hold_together() {
    let world = VariantWorld::new();
    let identity_a = world.repo_a.load_or_create_host_identity().unwrap();
    let identity_b = world.repo_b.load_or_create_host_identity().unwrap();
    assert_ne!(identity_a.host_id, identity_b.host_id);
    let replica_a1 = ReplicaId::parse("replica-a1").unwrap();
    let replica_a2 = ReplicaId::parse("replica-a2").unwrap();
    let same_a1 = world
        .repo_a
        .derive_origin_tokens(&world.bundle_id, &replica_a1)
        .unwrap();
    assert_eq!(
        same_a1,
        world
            .repo_a
            .derive_origin_tokens(&world.bundle_id, &replica_a1)
            .unwrap()
    );
    let same_a2 = world
        .repo_a
        .derive_origin_tokens(&world.bundle_id, &replica_a2)
        .unwrap();
    assert_eq!(same_a1.0, same_a2.0);
    assert_ne!(same_a1.1, same_a2.1);
    let other_bundle = BundleId::parse("fedcba9876543210fedcba9876543210").unwrap();
    assert_ne!(
        same_a1.0,
        world
            .repo_a
            .derive_origin_tokens(&other_bundle, &replica_a1)
            .unwrap()
            .0
    );
    assert_eq!(
        world
            .repo_a
            .resolve_origin_host_label(&world.bundle_id, &same_a1.0)
            .unwrap(),
        ("This host".to_string(), true)
    );

    let remote = world
        .repo_b
        .derive_origin_tokens(&world.bundle_id, &replica_a1)
        .unwrap()
        .0;
    let fallback = world
        .repo_a
        .resolve_origin_host_label(&world.bundle_id, &remote)
        .unwrap();
    assert!(!fallback.1);
    assert!(fallback.0.starts_with("Host "));
    assert!(fallback.0.len() <= 16);
    world
        .repo_a
        .mutate_origin_aliases(|aliases| {
            aliases.push(OriginHostAlias {
                bundle_id: world.bundle_id.clone(),
                origin_host_token: remote.clone(),
                display_name: "Private local alias".to_string(),
                revision: 0,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(
        world
            .repo_a
            .resolve_origin_host_label(&world.bundle_id, &remote)
            .unwrap(),
        ("Private local alias".to_string(), false)
    );

    let (mut action, mut current) = super::relation_matrix::reviewed_action(
        ConversationPlanDirection::Push,
        ConversationPlanActionType::NoOp,
        crate::project_sync_v3::domain::ConversationReasonCode::Identical,
        ConversationRelation::Identical,
    );
    action.host_id = identity_a.host_id.clone();
    action.expected_host_identity_revision = identity_a.revision;
    current.host_id = identity_a.host_id.clone();
    current.host_identity_revision = identity_a.revision;
    crate::project_sync_v3::conversation_sync::validate_conversation_action_pins(&action, &current)
        .unwrap();
    let rotated = world
        .repo_a
        .rotate_host_identity(
            Some((&identity_a.host_id, identity_a.revision)),
            Some("Rotated host".to_string()),
        )
        .unwrap();
    assert_ne!(rotated.host_id, identity_a.host_id);
    assert_ne!(
        world
            .repo_a
            .derive_origin_tokens(&world.bundle_id, &replica_a1)
            .unwrap()
            .0,
        same_a1.0
    );
    current.host_id = rotated.host_id;
    current.host_identity_revision = rotated.revision;
    assert_eq!(
        crate::project_sync_v3::conversation_sync::validate_conversation_action_pins(
            &action, &current,
        ),
        Err(crate::project_sync_v3::conversation_sync::ConversationStalePin::Host)
    );

    let origin = world.origin(&world.repo_b, "replica-b");
    world.publish(
        &[ThreadSpec::new(
            "thread-alias-scan",
            "source-thread",
            chain("source-thread", &[("m-a", "safe")]),
            origin.clone(),
            10,
        )],
        origin,
        10,
    );
    for entry in walkdir::WalkDir::new(world.store.local_root().unwrap()).follow_links(false) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            assert!(!String::from_utf8_lossy(&fs::read(entry.path()).unwrap())
                .contains("Private local alias"));
        }
    }
}

#[test]
fn schema6_manifest_rejects_each_conversation_reference_corruption() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let published = world.publish(
        &[
            ThreadSpec::new(
                thread_id().as_str(),
                "source-thread",
                chain("source-thread", &[("m-a", "one")]),
                origin.clone(),
                10,
            ),
            ThreadSpec::new(
                "thread-v2",
                "source-thread-v2",
                chain("source-thread-v2", &[("m-a", "one"), ("m-b", "two")]),
                origin.clone(),
                10,
            ),
        ],
        world.origin(&world.repo_a, "replica-a"),
        10,
    );
    let baseline = published.snapshot.manifest;
    baseline.validate().unwrap();
    let mut mutations: Vec<Box<dyn Fn(&mut crate::project_sync_v3::domain::BundleManifest)>> =
        Vec::new();
    mutations.push(Box::new(|manifest| {
        let thread = manifest.conversations.remove(&thread_id()).unwrap();
        manifest
            .conversations
            .insert(ThreadId::parse("thread-wrong").unwrap(), thread);
    }));
    mutations.push(Box::new(|manifest| {
        manifest
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .files
            .clear();
    }));
    mutations.push(Box::new(|manifest| {
        let thread = manifest.conversations.get_mut(&thread_id()).unwrap();
        let path = thread.files[0].logical_path.clone();
        thread.files.push(ConversationFileRef {
            logical_path: path,
            role: ConversationFileRole::Transcript,
        });
    }));
    mutations.push(Box::new(|manifest| {
        let thread = manifest.conversations.get_mut(&thread_id()).unwrap();
        thread.lineage_status = ConversationLineageStatus::Unknown {
            reason_code: "parser_gap".to_string(),
        };
    }));
    mutations.push(Box::new(|manifest| {
        manifest
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .lineage_codec = "codex.lineage.missing".to_string();
    }));
    mutations.push(Box::new(|manifest| {
        manifest.conversations.get_mut(&thread_id()).unwrap().provider = Provider::Claude;
    }));
    mutations.push(Box::new(|manifest| {
        let thread = &manifest.conversations[&thread_id()];
        let path = thread.files[0].logical_path.clone();
        manifest.files.remove(&path);
    }));
    mutations.push(Box::new(|manifest| {
        let resource_id = manifest.conversations[&thread_id()].resource_id.clone();
        manifest.resources.remove(&resource_id);
    }));
    mutations.push(Box::new(|manifest| {
        manifest
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .head_message_id = None;
    }));
    mutations.push(Box::new(|manifest| {
        let thread = manifest.conversations.get_mut(&thread_id()).unwrap();
        thread.forked_from = Some(thread.thread_id.clone());
    }));
    mutations.push(Box::new(|manifest| {
        let v2 = ThreadId::parse("thread-v2").unwrap();
        let v1_resource = manifest.conversations[&thread_id()].resource_id.clone();
        manifest.conversations.get_mut(&v2).unwrap().resource_id = v1_resource;
    }));
    mutations.push(Box::new(|manifest| {
        let v2 = ThreadId::parse("thread-v2").unwrap();
        let v1_path = manifest.conversations[&thread_id()].files[0]
            .logical_path
            .clone();
        manifest.conversations.get_mut(&v2).unwrap().files[0].logical_path = v1_path;
    }));
    mutations.push(Box::new(|manifest| {
        let v1 = &manifest.conversations[&thread_id()];
        let entry = manifest.files[&v1.files[0].logical_path].clone();
        manifest.files.insert(
            LogicalPath::parse("state/conversations/codex/thread-v1/undeclared.json").unwrap(),
            entry,
        );
    }));
    mutations.push(Box::new(|manifest| {
        let v2 = ThreadId::parse("thread-v2").unwrap();
        manifest.conversations.remove(&v2);
    }));
    mutations.push(Box::new(|manifest| {
        let resource = manifest.conversations[&thread_id()].resource_id.clone();
        manifest.recipe.entries.remove(&resource);
    }));
    mutations.push(Box::new(|manifest| {
        let resource = manifest.conversations[&thread_id()].resource_id.clone();
        manifest
            .resources
            .get_mut(&resource)
            .unwrap()
            .metadata
            .insert("oversized".to_string(), "x".repeat(4_097));
    }));
    mutations.push(Box::new(|manifest| {
        let path = manifest.conversations[&thread_id()].files[0]
            .logical_path
            .clone();
        manifest.files.get_mut(&path).unwrap().size = MAX_FILE_BYTES + 1;
    }));
    mutations.push(Box::new(|manifest| {
        let tombstone = Tombstone {
            target: TombstoneTarget::ConversationThread {
                thread_id: thread_id(),
            },
            last_sha256: Some("a".repeat(64)),
            deleted_at: 20,
        };
        manifest
            .tombstones
            .insert(tombstone.canonical_key(), tombstone);
    }));

    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut invalid = baseline.clone();
        mutate(&mut invalid);
        assert!(
            invalid.validate().is_err(),
            "mutation {index} unexpectedly validated"
        );
    }

    let mut serialized = serde_json::to_value(&baseline).unwrap();
    serialized["conversations"][thread_id().as_str()]["files"][0]["role"] =
        serde_json::json!({"kind": "unrecognized_provider_role"});
    assert!(
        serde_json::from_value::<crate::project_sync_v3::domain::BundleManifest>(serialized)
            .is_err()
    );
}

#[test]
fn local_bindings_reject_duplicate_provider_thread_ownership_profile_wide() {
    let world = VariantWorld::new();
    let scope = super::recovery::install_scope(
        &world.repo_fresh,
        &world.bundle_id,
        "duplicate-provider-owner",
        "replica-duplicate-owner",
    );
    let binding = |thread: &str| LocalConversationBinding {
        host_id: scope.host_id.clone(),
        replica_id: scope.replica_id.clone(),
        local_project_id: scope.project_id.clone(),
        provider_profile_id: scope.profile_id.clone(),
        thread_id: ThreadId::parse(thread).unwrap(),
        provider_thread_id: Some("same-provider-thread".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        last_observed_file_sha256: None,
        last_observed_head_message_id: None,
        last_observed_lineage_digest: None,
        last_observed_semantic_digest: None,
        source_generation: None,
        source_manifest_sha256: None,
        revision: 0,
        created_at: 1,
        updated_at: 1,
    };
    let error = world
        .repo_fresh
        .mutate_conversation_bindings(|document| {
            document.bindings.push(binding("thread-v1"));
            document.bindings.push(binding("thread-v2"));
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("owned by multiple conversations"));
    assert!(world
        .repo_fresh
        .load_conversation_bindings()
        .unwrap()
        .bindings
        .is_empty());
}

#[test]
fn portable_objects_do_not_leak_machine_local_identity_or_paths() {
    let world = VariantWorld::new();
    let identity = world.repo_a.load_or_create_host_identity().unwrap();
    let origin = world.origin(&world.repo_a, "replica-private-sentinel");
    world.publish(
        &[ThreadSpec::new(
            "thread-v1",
            "bounded-source-provenance",
            chain("bounded-source-provenance", &[("m-a", "safe content")]),
            origin.clone(),
            10,
        )],
        origin,
        10,
    );

    let forbidden = [
        identity.host_id.to_string(),
        "replica-private-sentinel".to_string(),
        "profile-private-sentinel".to_string(),
        "journal-private-sentinel".to_string(),
        world.repo_a.root().to_string_lossy().to_string(),
        std::env::var("USER").unwrap_or_default(),
    ];
    let store_root = world.store.local_root().unwrap();
    for entry in walkdir::WalkDir::new(store_root).follow_links(false) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let bytes = fs::read(entry.path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for sentinel in forbidden.iter().filter(|value| !value.is_empty()) {
            assert!(
                !text.contains(sentinel),
                "portable object '{}' leaked forbidden sentinel",
                entry.path().display()
            );
        }
    }
}

#[test]
fn schema4_writer_is_rejected_on_a_schema6_head() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let published = world.publish(
        &[ThreadSpec::new(
            "thread-v1",
            "source-thread",
            chain("source-thread", &[("m-a", "one")]),
            origin.clone(),
            10,
        )],
        origin,
        10,
    );
    let schema4_attempt = world.engine().publish(PublishBundleRequest {
        identity: BundleIdentity {
            bundle_id: world.bundle_id.clone(),
            display_name: "Thread test bundle".to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        },
        recipe: published.snapshot.manifest.recipe,
        captured_with: CapturedWith {
            app_version: "old-writer".to_string(),
            codex_version: None,
            claude_version: None,
            codec_versions: BTreeMap::new(),
        },
        captured: CapturedResources::default(),
        observed_head: Some((published.snapshot.head.clone(), published.head_token)),
        updated_at: 20,
    });
    assert!(schema4_attempt
        .unwrap_err()
        .contains("conversation-thread publication path"));
}

fn thread_id() -> ThreadId {
    ThreadId::parse("thread-v1").unwrap()
}
