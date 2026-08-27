use super::harness::{chain, codex_scope, codex_scope_key, ThreadSpec, VariantWorld};
use mallard_domain::{
    BundleId, BundleManifest, BundleScope, CloudThreadId, ConversationFileRef,
    ConversationFileRole, ConversationLineageStatus, ConversationMaterializationState,
    ConversationPlanActionType, LocalConversationBinding, LogicalPath, OriginHostAlias,
    PreservedProvenance, ProviderKey, ReplicaId, ThreadId, Tombstone, TombstoneTarget,
};
use mallard_infrastructure::storage_core::ObjectStore;
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
        ConversationPlanActionType::Publish,
        mallard_domain::TailClassification::LocalOutstanding,
    );
    action.pins.context.host_id = identity_a.host_id.clone();
    action.pins.context.host_identity_revision = identity_a.revision;
    current.pins.context.host_id = identity_a.host_id.clone();
    current.pins.context.host_identity_revision = identity_a.revision;
    mallard_infrastructure::conversation_sync::validate_conversation_action_pins(&action, &current)
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
    current.pins.context.host_id = rotated.host_id;
    current.pins.context.host_identity_revision = rotated.revision;
    assert_eq!(
        mallard_infrastructure::conversation_sync::validate_conversation_action_pins(
            &action, &current,
        ),
        Err(
            mallard_infrastructure::conversation_sync::ConversationStalePin::Context(
                mallard_infrastructure::conversation_sync::ConversationContextPin::Host,
            )
        )
    );

    let origin = world.origin(&world.repo_b, "replica-b");
    world.publish(
        &[ThreadSpec::new(
            "cloudthread-alias-scan",
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
fn current_schema_manifest_rejects_each_conversation_reference_corruption() {
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
                "cloudthread-v2",
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
    let large_path = codex_scope(&baseline).conversations[&thread_id()].files[0]
        .logical_path
        .clone();
    let large_file_manifest = mutate_codex_scope(&baseline, |scope| {
        scope.files.get_mut(&large_path).unwrap().size = 512 * 1024 * 1024 + 1;
    });
    large_file_manifest.validate().unwrap();

    type ScopeMutation = Box<dyn Fn(&mut BundleScope)>;
    let mut mutations: Vec<ScopeMutation> = Vec::new();
    mutations.push(Box::new(|scope| {
        let thread = scope.conversations.remove(&thread_id()).unwrap();
        scope
            .conversations
            .insert(CloudThreadId::parse("cloudthread-wrong").unwrap(), thread);
    }));
    mutations.push(Box::new(|scope| {
        scope
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .files
            .clear();
    }));
    mutations.push(Box::new(|scope| {
        let thread = scope.conversations.get_mut(&thread_id()).unwrap();
        let path = thread.files[0].logical_path.clone();
        thread.files.push(ConversationFileRef {
            logical_path: path,
            role: ConversationFileRole::Transcript,
        });
    }));
    mutations.push(Box::new(|scope| {
        let thread = scope.conversations.get_mut(&thread_id()).unwrap();
        thread.lineage_status = ConversationLineageStatus::Unknown {
            reason_code: "parser_gap".to_string(),
        };
    }));
    mutations.push(Box::new(|scope| {
        scope
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .lineage_codec = "codex.lineage.missing".to_string();
    }));
    mutations.push(Box::new(|scope| {
        scope.conversations.get_mut(&thread_id()).unwrap().provider =
            ProviderKey::parse("claude").unwrap();
    }));
    mutations.push(Box::new(|scope| {
        let thread = &scope.conversations[&thread_id()];
        let path = thread.files[0].logical_path.clone();
        scope.files.remove(&path);
    }));
    mutations.push(Box::new(|scope| {
        let resource_id = scope.conversations[&thread_id()].resource_id.clone();
        scope.resources.remove(&resource_id);
    }));
    mutations.push(Box::new(|scope| {
        scope
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .head_message_id = None;
    }));
    mutations.push(Box::new(|scope| {
        let thread = scope.conversations.get_mut(&thread_id()).unwrap();
        thread.forked_from = Some(thread.thread_id.clone());
    }));
    // Preservation provenance fails closed under the same reference rules:
    // no self-preserve and no zero ordinal.
    mutations.push(Box::new(|scope| {
        let thread = scope.conversations.get_mut(&thread_id()).unwrap();
        thread.preserved = Some(PreservedProvenance {
            preserved_from: thread.thread_id.clone(),
            ordinal: 1,
        });
    }));
    mutations.push(Box::new(|scope| {
        let thread = scope.conversations.get_mut(&thread_id()).unwrap();
        thread.preserved = Some(PreservedProvenance {
            preserved_from: CloudThreadId::parse("cloudthread-v2").unwrap(),
            ordinal: 0,
        });
    }));
    mutations.push(Box::new(|scope| {
        let v2 = CloudThreadId::parse("cloudthread-v2").unwrap();
        let v1_resource = scope.conversations[&thread_id()].resource_id.clone();
        scope.conversations.get_mut(&v2).unwrap().resource_id = v1_resource;
    }));
    mutations.push(Box::new(|scope| {
        let v2 = CloudThreadId::parse("cloudthread-v2").unwrap();
        let v1_path = scope.conversations[&thread_id()].files[0]
            .logical_path
            .clone();
        scope.conversations.get_mut(&v2).unwrap().files[0].logical_path = v1_path;
    }));
    mutations.push(Box::new(|scope| {
        let v1 = &scope.conversations[&thread_id()];
        let entry = scope.files[&v1.files[0].logical_path].clone();
        scope.files.insert(
            LogicalPath::parse("state/conversations/codex/cloudthread-v1/undeclared.json").unwrap(),
            entry,
        );
    }));
    mutations.push(Box::new(|scope| {
        let v2 = CloudThreadId::parse("cloudthread-v2").unwrap();
        scope.conversations.remove(&v2);
    }));
    mutations.push(Box::new(|scope| {
        let resource = scope.conversations[&thread_id()].resource_id.clone();
        scope.recipe.entries.remove(&resource);
    }));
    mutations.push(Box::new(|scope| {
        let resource = scope.conversations[&thread_id()].resource_id.clone();
        scope
            .resources
            .get_mut(&resource)
            .unwrap()
            .metadata
            .insert("oversized".to_string(), "x".repeat(4_097));
    }));
    mutations.push(Box::new(|scope| {
        let tombstone = Tombstone {
            target: TombstoneTarget::ConversationThread {
                thread_id: thread_id(),
            },
            last_sha256: Some("a".repeat(64)),
            deleted_at: 20,
        };
        scope
            .tombstones
            .insert(tombstone.canonical_key(), tombstone);
    }));
    // A simulated old writer publishing a tail-less schema-1 Codex successor
    // is blocked on the next verified read, as is a tail whose accepted end
    // disagrees with the recorded message count.
    mutations.push(Box::new(|scope| {
        scope
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .portable_tail = None;
    }));
    mutations.push(Box::new(|scope| {
        scope
            .conversations
            .get_mut(&thread_id())
            .unwrap()
            .portable_tail
            .as_mut()
            .unwrap()
            .accepted_end_record_count += 1;
    }));

    for (index, mutate) in mutations.into_iter().enumerate() {
        // The scope digest is resealed over the corrupted content, so every
        // rejection below comes from the reference rule itself and not from
        // a trivially stale content digest.
        let invalid = mutate_codex_scope(&baseline, |scope| mutate(scope));
        assert!(
            invalid.validate().is_err(),
            "mutation {index} unexpectedly validated"
        );
    }

    let mut serialized = serde_json::to_value(&baseline).unwrap();
    serialized["scopes"][codex_scope_key().canonical()]["conversations"][thread_id().as_str()]
        ["files"][0]["role"] = serde_json::json!({"kind": "unrecognized_provider_role"});
    assert!(serde_json::from_value::<BundleManifest>(serialized).is_err());
}

/// Apply one corruption to the manifest's Codex scope and reseal that
/// scope's content digest so validation exercises the structural rules.
fn mutate_codex_scope(
    baseline: &BundleManifest,
    mutate: impl FnOnce(&mut BundleScope),
) -> BundleManifest {
    let mut manifest = baseline.clone();
    let scope_key = codex_scope_key();
    let scope = manifest.scopes.get_mut(&scope_key).unwrap();
    mutate(scope);
    scope.content_sha256 = scope.compute_content_sha256(&scope_key).unwrap();
    manifest
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
        cloud_thread_id: None,
        forked_from: None,
        preserved: None,
        provider_thread_id: Some("same-provider-thread".to_string()),
        materialization_state: ConversationMaterializationState::Resumable,
        root_tail_layout: Some(mallard_domain::RootTailLayout {
            codec: mallard_domain::PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: mallard_domain::TailPartitionKind::WholeThread,
            start_record_count: 0,
        }),
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
            "cloudthread-v1",
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

fn thread_id() -> CloudThreadId {
    CloudThreadId::parse("cloudthread-v1").unwrap()
}
