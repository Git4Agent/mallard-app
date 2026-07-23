use super::harness::{chain, schema5_request, VariantSpec, VariantWorld};
use crate::project_sync_v3::bundle_engine::{
    build_schema5_cutover_request, BundleEngine, BundleObjectStore, CasExpectation, CasOutcome,
    ImmutablePutOutcome, ObjectKey, ObjectPrefix, PublishExpectation, PublishSchema5Request,
    StoreListPage, StoredObject,
};
use crate::project_sync_v3::domain::{
    BundleHead, BundleManifest, BundleRecipe, ConflictGroupId, StorageConfigV3, StorageKind,
    Tombstone, TombstoneTarget, VariantId, BUNDLE_SCHEMA_V4, BUNDLE_SCHEMA_V5,
};
use crate::project_sync_v3::s3_store::S3BundleObjectStore;
use crate::sync_tests::stub_s3::StubS3;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[test]
fn local_store_rejects_unreviewed_remote_variant_loss_and_stale_cas() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let v1 = VariantSpec::new(
        "group-main",
        v1_id.as_str(),
        "source-thread",
        chain("source-thread", &[("m-a", "one"), ("m-b1", "v1")]),
        origin.clone(),
        10,
    );
    let v2 = VariantSpec::new(
        "group-main",
        v2_id.as_str(),
        "source-thread",
        chain("source-thread", &[("m-a", "one"), ("m-b2", "v2")]),
        origin.clone(),
        10,
    );
    let initial = world.publish(&[v1.clone(), v2], &v1_id, origin.clone(), 10);
    let dropping = schema5_request(
        world.bundle_id.clone(),
        &[v1.clone()],
        &v1_id,
        origin.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let error = world.engine().publish_schema5(dropping).unwrap_err();
    assert!(error.contains("drops conversation variant"));
    assert_eq!(
        world
            .engine()
            .inspect(&world.bundle_id)
            .unwrap()
            .head
            .generation,
        1
    );

    let mut advanced = v1;
    advanced.transcript = chain(
        "source-thread",
        &[("m-a", "one"), ("m-b1", "v1"), ("m-c1", "advance")],
    );
    advanced.updated_at = 20;
    let first_request = schema5_request(
        world.bundle_id.clone(),
        &[advanced.clone(), variant_v2(&origin)],
        &v1_id,
        origin.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let stale_request = schema5_request(
        world.bundle_id.clone(),
        &[advanced, variant_v2(&origin)],
        &v1_id,
        origin,
        21,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token),
    );
    world.engine().publish_schema5(first_request).unwrap();
    assert!(world
        .engine()
        .publish_schema5(stale_request)
        .unwrap_err()
        .contains("head changed"));
}

#[test]
fn local_store_immutable_and_head_failures_leave_a_retryable_fixed_point() {
    let temp = tempfile::tempdir().unwrap();
    let inner = crate::project_sync_v3::bundle_engine::LocalBundleObjectStore::open(
        temp.path().join("store"),
    )
    .unwrap();
    let store = FailingStore::new(inner);
    let storage_id = crate::project_sync_v3::domain::StorageId::parse("failure-store").unwrap();
    let engine = BundleEngine::open(store.clone(), storage_id.clone()).unwrap();
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let variant_id = VariantId::parse("variant-v1").unwrap();
    let variant = VariantSpec::new(
        "group-main",
        variant_id.as_str(),
        "source-thread",
        chain("source-thread", &[("m-a", "one")]),
        origin.clone(),
        10,
    );
    let initial_request = schema5_request(
        world.bundle_id.clone(),
        &[variant.clone()],
        &variant_id,
        origin.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    store.fail_next(StoreFailure::Immutable);
    assert!(engine.publish_schema5(initial_request.clone()).is_err());
    assert!(engine.read_head(&world.bundle_id).unwrap().is_none());

    let initial = engine.publish_schema5(initial_request).unwrap();
    assert_eq!(initial.snapshot.head.generation, 1);
    let mut advanced = variant;
    advanced.transcript = chain("source-thread", &[("m-a", "one"), ("m-b", "continued")]);
    advanced.updated_at = 20;
    let advanced_request = schema5_request(
        world.bundle_id.clone(),
        &[advanced],
        &variant_id,
        origin,
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    store.fail_next(StoreFailure::HeadCas);
    assert!(engine.publish_schema5(advanced_request.clone()).is_err());
    let unchanged = engine.inspect(&world.bundle_id).unwrap();
    assert_eq!(unchanged.head.generation, 1);
    assert_eq!(unchanged.head.commit_id, initial.snapshot.head.commit_id);

    let retried = engine.publish_schema5(advanced_request).unwrap();
    assert_eq!(retried.snapshot.head.generation, 2);
    let fetched = engine.fetch(&world.bundle_id).unwrap();
    fetched.snapshot.validate().unwrap();
    for (path, entry) in &fetched.snapshot.manifest.files {
        assert_eq!(super::harness::sha256(&fetched.files[path]), entry.sha256);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_schema5_noop_and_fast_forward_match_local_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let variant_id = VariantId::parse("variant-v1").unwrap();
    let variant = VariantSpec::new(
        "group-main",
        variant_id.as_str(),
        "source-thread",
        chain("source-thread", &[("m-a", "one")]),
        origin.clone(),
        10,
    );
    let storage_id = crate::project_sync_v3::domain::StorageId::parse("s3-variant-store").unwrap();
    let config = StorageConfigV3 {
        id: storage_id.clone(),
        name: "Variant stub S3".to_string(),
        kind: StorageKind::S3,
        bucket: "variant-bucket".to_string(),
        access_key_id: "test-access-key".to_string(),
        secret_access_key: "test-secret".to_string(),
        account_id: String::new(),
        s3_endpoint: server.endpoint.clone(),
        region: "auto".to_string(),
        local_dir: String::new(),
        included_default_exclusions: Vec::new(),
        supports_conditional_writes: None,
    };
    let store =
        S3BundleObjectStore::from_config(&config, tokio::runtime::Handle::current()).unwrap();
    let initial_request = schema5_request(
        world.bundle_id.clone(),
        &[variant.clone()],
        &variant_id,
        origin.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial_store = store.clone();
    let initial_storage = storage_id.clone();
    let initial = tokio::task::spawn_blocking(move || {
        BundleEngine::open(initial_store, initial_storage)
            .unwrap()
            .publish_schema5(initial_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert!(initial.published);

    let noop_request = schema5_request(
        world.bundle_id.clone(),
        &[variant.clone()],
        &variant_id,
        world.origin(&world.repo_b, "replica-b"),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let noop_store = store.clone();
    let noop_storage = storage_id.clone();
    let noop = tokio::task::spawn_blocking(move || {
        BundleEngine::open(noop_store, noop_storage)
            .unwrap()
            .publish_schema5(noop_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!noop.published);
    assert_eq!(noop.snapshot.head.generation, 1);

    let mut advanced = variant;
    advanced.transcript = chain("source-thread", &[("m-a", "one"), ("m-b", "continued")]);
    advanced.updated_at = 30;
    let advance_request = schema5_request(
        world.bundle_id.clone(),
        &[advanced],
        &variant_id,
        origin,
        30,
        Some(&noop.snapshot.manifest),
        PublishExpectation::Match(noop.head_token),
    );
    let advance_store = store.clone();
    let advance_storage = storage_id.clone();
    let advanced = tokio::task::spawn_blocking(move || {
        BundleEngine::open(advance_store, advance_storage)
            .unwrap()
            .publish_schema5(advance_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert!(advanced.published);
    assert_eq!(advanced.snapshot.head.generation, 2);

    let fetch_store = store;
    let fetched = tokio::task::spawn_blocking(move || {
        BundleEngine::open(fetch_store, storage_id)
            .unwrap()
            .fetch(&world.bundle_id)
            .unwrap()
    })
    .await
    .unwrap();
    fetched.snapshot.validate().unwrap();
    assert!(server.requests().iter().any(|request| {
        request.method == "PUT" && request.key.ends_with("/_head.json") && request.conditional
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_retains_remote_variants_and_one_stale_writer_replans() {
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let v1_id = VariantId::parse("variant-v1").unwrap();
    let v2_id = VariantId::parse("variant-v2").unwrap();
    let v1 = VariantSpec::new(
        "group-main",
        v1_id.as_str(),
        "shared-source",
        chain(
            "shared-source",
            &[("m-a", "base"), ("m-b", "answer"), ("m-v1", "branch A")],
        ),
        origin_a.clone(),
        10,
    );
    let v2 = VariantSpec::new(
        "group-main",
        v2_id.as_str(),
        "shared-source",
        chain(
            "shared-source",
            &[("m-a", "base"), ("m-b", "answer"), ("m-v2", "branch B")],
        ),
        origin_b.clone(),
        10,
    );
    let storage_id = crate::project_sync_v3::domain::StorageId::parse("s3-cas-store").unwrap();
    let config = StorageConfigV3 {
        id: storage_id.clone(),
        name: "Variant CAS stub S3".to_string(),
        kind: StorageKind::S3,
        bucket: "variant-bucket".to_string(),
        access_key_id: "test-access-key".to_string(),
        secret_access_key: "test-secret".to_string(),
        account_id: String::new(),
        s3_endpoint: server.endpoint.clone(),
        region: "auto".to_string(),
        local_dir: String::new(),
        included_default_exclusions: Vec::new(),
        supports_conditional_writes: None,
    };
    let store =
        S3BundleObjectStore::from_config(&config, tokio::runtime::Handle::current()).unwrap();
    let initial_request = schema5_request(
        world.bundle_id.clone(),
        &[v1.clone(), v2.clone()],
        &v1_id,
        origin_a.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial_store = store.clone();
    let initial_storage = storage_id.clone();
    let initial = tokio::task::spawn_blocking(move || {
        BundleEngine::open(initial_store, initial_storage)
            .unwrap()
            .publish_schema5(initial_request)
            .unwrap()
    })
    .await
    .unwrap();

    let mut advanced_v1 = v1.clone();
    advanced_v1.transcript = chain(
        "shared-source",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-v1", "branch A"),
            ("m-a2", "A continues"),
        ],
    );
    advanced_v1.updated_at = 20;
    let writer_a = schema5_request(
        world.bundle_id.clone(),
        &[advanced_v1.clone(), v2.clone()],
        &v1_id,
        origin_a,
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let mut advanced_v2 = v2.clone();
    advanced_v2.transcript = chain(
        "shared-source",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-v2", "branch B"),
            ("m-b2", "B continues"),
        ],
    );
    advanced_v2.updated_at = 21;
    let stale_writer_b = schema5_request(
        world.bundle_id.clone(),
        &[v1, advanced_v2.clone()],
        &v1_id,
        origin_b.clone(),
        21,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token),
    );
    let writer_a_store = store.clone();
    let writer_a_storage = storage_id.clone();
    let after_a = tokio::task::spawn_blocking(move || {
        BundleEngine::open(writer_a_store, writer_a_storage)
            .unwrap()
            .publish_schema5(writer_a)
            .unwrap()
    })
    .await
    .unwrap();
    let writer_b_store = store.clone();
    let writer_b_storage = storage_id.clone();
    let stale_error = tokio::task::spawn_blocking(move || {
        BundleEngine::open(writer_b_store, writer_b_storage)
            .unwrap()
            .publish_schema5(stale_writer_b)
            .unwrap_err()
    })
    .await
    .unwrap();
    assert!(stale_error.contains("head changed") || stale_error.contains("conflict"));

    let group_id = crate::project_sync_v3::domain::ConflictGroupId::parse("group-main").unwrap();
    let v1_path = after_a.snapshot.manifest.conversation_groups[&group_id].variants[&v1_id].files
        [0]
    .logical_path
    .clone();
    let v1_object = after_a.snapshot.manifest.files[&v1_path].object_key.clone();
    let retry_b = schema5_request(
        world.bundle_id.clone(),
        &[advanced_v1, advanced_v2],
        &v1_id,
        origin_b,
        30,
        Some(&after_a.snapshot.manifest),
        PublishExpectation::Match(after_a.head_token),
    );
    let retry_store = store.clone();
    let retry_storage = storage_id.clone();
    let after_b = tokio::task::spawn_blocking(move || {
        BundleEngine::open(retry_store, retry_storage)
            .unwrap()
            .publish_schema5(retry_b)
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(after_b.snapshot.head.generation, 3);
    let group = &after_b.snapshot.manifest.conversation_groups[&group_id];
    assert_eq!(group.variants.len(), 2);
    assert_eq!(group.primary_variant_id, v1_id);
    assert!(group.variants.contains_key(&v2_id));
    assert_eq!(
        after_b.snapshot.manifest.files[&v1_path].object_key,
        v1_object
    );

    let fetch_store = store;
    let fetched = tokio::task::spawn_blocking(move || {
        BundleEngine::open(fetch_store, storage_id)
            .unwrap()
            .fetch(&world.bundle_id)
            .unwrap()
    })
    .await
    .unwrap();
    fetched.snapshot.validate().unwrap();
    for (path, entry) in &fetched.snapshot.manifest.files {
        assert_eq!(super::harness::sha256(&fetched.files[path]), entry.sha256);
    }
    assert!(server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.key.ends_with("/_head.json"))
        .all(|request| request.conditional));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_same_id_divergence_and_tombstone_resurrection_match_local() {
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let group_id = ConflictGroupId::parse("group-s3-divergence").unwrap();
    let cloud_id = VariantId::parse("variant-shared").unwrap();
    let local_id = VariantId::parse("variant-local-stable").unwrap();
    let cloud_variant = VariantSpec::new(
        group_id.as_str(),
        cloud_id.as_str(),
        "shared-source",
        chain(
            "shared-source",
            &[
                ("m-a", "base"),
                ("m-b", "answer"),
                ("m-cloud", "cloud fork"),
            ],
        ),
        origin_b.clone(),
        10,
    );
    let local_variant = VariantSpec::new(
        group_id.as_str(),
        local_id.as_str(),
        "shared-source",
        chain(
            "shared-source",
            &[
                ("m-a", "base"),
                ("m-b", "answer"),
                ("m-local", "local fork"),
            ],
        ),
        origin_a.clone(),
        20,
    );
    let storage_id =
        crate::project_sync_v3::domain::StorageId::parse("s3-divergence-store").unwrap();
    let config = StorageConfigV3 {
        id: storage_id.clone(),
        name: "Variant divergence stub S3".to_string(),
        kind: StorageKind::S3,
        bucket: "variant-bucket".to_string(),
        access_key_id: "test-access-key".to_string(),
        secret_access_key: "test-secret".to_string(),
        account_id: String::new(),
        s3_endpoint: server.endpoint.clone(),
        region: "auto".to_string(),
        local_dir: String::new(),
        included_default_exclusions: Vec::new(),
        supports_conditional_writes: None,
    };
    let store =
        S3BundleObjectStore::from_config(&config, tokio::runtime::Handle::current()).unwrap();
    let initial_request = schema5_request(
        world.bundle_id.clone(),
        &[cloud_variant.clone()],
        &cloud_id,
        origin_b,
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial_store = store.clone();
    let initial_storage = storage_id.clone();
    let initial = tokio::task::spawn_blocking(move || {
        BundleEngine::open(initial_store, initial_storage)
            .unwrap()
            .publish_schema5(initial_request)
            .unwrap()
    })
    .await
    .unwrap();
    let cloud_path = initial.snapshot.manifest.conversation_groups[&group_id].variants[&cloud_id]
        .files[0]
        .logical_path
        .clone();
    let cloud_object = initial.snapshot.manifest.files[&cloud_path]
        .object_key
        .clone();

    let divergent_request = schema5_request(
        world.bundle_id.clone(),
        &[cloud_variant.clone(), local_variant.clone()],
        &cloud_id,
        origin_a.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token),
    );
    let divergent_store = store.clone();
    let divergent_storage = storage_id.clone();
    let divergent = tokio::task::spawn_blocking(move || {
        BundleEngine::open(divergent_store, divergent_storage)
            .unwrap()
            .publish_schema5(divergent_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(divergent.snapshot.head.generation, 2);
    assert_eq!(
        divergent.snapshot.manifest.conversation_groups[&group_id]
            .variants
            .len(),
        2,
    );
    assert_eq!(
        divergent.snapshot.manifest.files[&cloud_path].object_key,
        cloud_object,
    );

    let no_op_request = schema5_request(
        world.bundle_id.clone(),
        &[cloud_variant, local_variant.clone()],
        &cloud_id,
        origin_a.clone(),
        25,
        Some(&divergent.snapshot.manifest),
        PublishExpectation::Match(divergent.head_token),
    );
    let no_op_store = store.clone();
    let no_op_storage = storage_id.clone();
    let no_op = tokio::task::spawn_blocking(move || {
        BundleEngine::open(no_op_store, no_op_storage)
            .unwrap()
            .publish_schema5(no_op_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!no_op.published);
    assert_eq!(no_op.snapshot.head.generation, 2);

    let group_tombstone = Tombstone {
        target: TombstoneTarget::ConversationGroup {
            conflict_group_id: group_id.clone(),
        },
        last_sha256: Some(super::harness::sha256(&local_variant.transcript)),
        deleted_at: 30,
    };
    let delete_request = PublishSchema5Request {
        identity: no_op.snapshot.manifest.bundle.clone(),
        recipe: BundleRecipe::default(),
        captured_with: no_op.snapshot.manifest.captured_with.clone(),
        published_by_origin: origin_a.clone(),
        conversation_groups: BTreeMap::new(),
        resources: BTreeMap::new(),
        retained_files: BTreeMap::new(),
        captured_files: BTreeMap::new(),
        directories: BTreeMap::new(),
        tombstones: BTreeMap::from([(group_tombstone.canonical_key(), group_tombstone)]),
        expected_head: PublishExpectation::Match(no_op.head_token),
        updated_at: 30,
    };
    let delete_store = store.clone();
    let delete_storage = storage_id.clone();
    let deleted = tokio::task::spawn_blocking(move || {
        BundleEngine::open(delete_store, delete_storage)
            .unwrap()
            .publish_schema5(delete_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(deleted.snapshot.head.generation, 3);
    assert!(deleted.snapshot.manifest.conversation_groups.is_empty());

    let mut modified = local_variant;
    modified.transcript = chain(
        "shared-source",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-local", "local fork"),
            ("m-local-2", "modified after deletion"),
        ],
    );
    modified.updated_at = 40;
    let resurrect_request = schema5_request(
        world.bundle_id.clone(),
        &[modified.clone()],
        &local_id,
        origin_a.clone(),
        40,
        Some(&deleted.snapshot.manifest),
        PublishExpectation::Match(deleted.head_token),
    );
    let resurrect_store = store.clone();
    let resurrect_storage = storage_id.clone();
    let resurrected = tokio::task::spawn_blocking(move || {
        BundleEngine::open(resurrect_store, resurrect_storage)
            .unwrap()
            .publish_schema5(resurrect_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(resurrected.snapshot.head.generation, 4);
    assert!(resurrected
        .snapshot
        .manifest
        .conversation_groups
        .contains_key(&group_id));
    assert!(!resurrected
        .snapshot
        .manifest
        .tombstones
        .values()
        .any(|tombstone| matches!(
            &tombstone.target,
            TombstoneTarget::ConversationGroup { conflict_group_id }
                if conflict_group_id == &group_id
        )));

    let retry_request = schema5_request(
        world.bundle_id.clone(),
        &[modified],
        &local_id,
        origin_a,
        50,
        Some(&resurrected.snapshot.manifest),
        PublishExpectation::Match(resurrected.head_token),
    );
    let retry_store = store.clone();
    let retried = tokio::task::spawn_blocking(move || {
        BundleEngine::open(retry_store, storage_id)
            .unwrap()
            .publish_schema5(retry_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!retried.published);
    let fetched = {
        let fetch_store = store;
        let fetch_storage = config.id.clone();
        let fetch_bundle = world.bundle_id.clone();
        tokio::task::spawn_blocking(move || {
            BundleEngine::open(fetch_store, fetch_storage)
                .unwrap()
                .fetch(&fetch_bundle)
                .unwrap()
        })
        .await
        .unwrap()
    };
    fetched.snapshot.validate().unwrap();
    for (path, entry) in &fetched.snapshot.manifest.files {
        assert_eq!(super::harness::sha256(&fetched.files[path]), entry.sha256);
    }
    assert!(server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.key.ends_with("/_head.json"))
        .all(|request| request.conditional));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_schema4_golden_cutover_preserves_bytes_and_cas_history() {
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let storage_id = crate::project_sync_v3::domain::StorageId::parse("s3-cutover-store").unwrap();
    let config = StorageConfigV3 {
        id: storage_id.clone(),
        name: "Variant cutover stub S3".to_string(),
        kind: StorageKind::S3,
        bucket: "variant-bucket".to_string(),
        access_key_id: "test-access-key".to_string(),
        secret_access_key: "test-secret".to_string(),
        account_id: String::new(),
        s3_endpoint: server.endpoint.clone(),
        region: "auto".to_string(),
        local_dir: String::new(),
        included_default_exclusions: Vec::new(),
        supports_conditional_writes: None,
    };
    let store =
        S3BundleObjectStore::from_config(&config, tokio::runtime::Handle::current()).unwrap();
    let publisher = world.origin(&world.repo_a, "replica-cutover");
    let bundle_id = world.bundle_id.clone();
    let task_store = store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(task_store.clone(), storage_id).unwrap();
        let manifest: BundleManifest = serde_json::from_str(
            super::schema_cutover::LEGACY_MANIFEST,
        )
        .unwrap();
        manifest.validate().unwrap();
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_sha256 = super::harness::sha256(&manifest_bytes);
        let manifest_key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{bundle_id}/_manifests/7-commit-legacy-fixture.json"
        ))
        .unwrap();
        let transcript_key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{bundle_id}/_uploads/upload-legacy/files/state/codex/sessions/legacy-thread.jsonl"
        ))
        .unwrap();
        task_store
            .put_immutable(&manifest_key, &manifest_bytes)
            .unwrap();
        task_store
            .put_immutable(
                &transcript_key,
                super::schema_cutover::LEGACY_TRANSCRIPT,
            )
            .unwrap();
        let head = BundleHead {
            schema_version: BUNDLE_SCHEMA_V4,
            bundle_id: bundle_id.clone(),
            kind: manifest.bundle.kind,
            generation: manifest.generation,
            commit_id: manifest.commit_id.clone(),
            manifest_key: "_manifests/7-commit-legacy-fixture.json".to_string(),
            manifest_sha256,
            updated_at: manifest.updated_at,
        };
        let head_key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{bundle_id}/_head.json"
        ))
        .unwrap();
        assert!(matches!(
            task_store
                .compare_and_swap(
                    &head_key,
                    &CasExpectation::Absent,
                    &serde_json::to_vec(&head).unwrap(),
                )
                .unwrap(),
            CasOutcome::Written { .. }
        ));
        let legacy = engine.fetch(&bundle_id).unwrap();
        assert_eq!(legacy.snapshot.head.schema_version, BUNDLE_SCHEMA_V4);
        assert!(legacy
            .files
            .values()
            .any(|bytes| bytes == super::schema_cutover::LEGACY_TRANSCRIPT));
        let (_, token) = engine.read_head(&bundle_id).unwrap().unwrap();
        let request = build_schema5_cutover_request(&legacy, publisher, token, 1784678410)
            .unwrap();
        let cutover = engine.publish_schema5(request).unwrap();
        assert_eq!(cutover.snapshot.head.schema_version, BUNDLE_SCHEMA_V5);
        let fetched = engine.fetch(&bundle_id).unwrap();
        fetched.snapshot.validate().unwrap();
        assert!(fetched
            .files
            .values()
            .any(|bytes| bytes == super::schema_cutover::LEGACY_TRANSCRIPT));
        fetched.snapshot.head.generation
    })
    .await
    .unwrap();
    assert_eq!(result, 8);
    let head_puts = server
        .requests()
        .into_iter()
        .filter(|request| request.method == "PUT" && request.key.ends_with("/_head.json"))
        .collect::<Vec<_>>();
    assert_eq!(head_puts.len(), 2);
    assert!(head_puts.iter().all(|request| request.conditional));
}

fn variant_v2(origin: &crate::project_sync_v3::domain::PortableOrigin) -> VariantSpec {
    VariantSpec::new(
        "group-main",
        "variant-v2",
        "source-thread",
        chain("source-thread", &[("m-a", "one"), ("m-b2", "v2")]),
        origin.clone(),
        10,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreFailure {
    Immutable,
    HeadCas,
}

#[derive(Clone)]
struct FailingStore {
    inner: crate::project_sync_v3::bundle_engine::LocalBundleObjectStore,
    next_failure: Arc<Mutex<Option<StoreFailure>>>,
}

impl FailingStore {
    fn new(inner: crate::project_sync_v3::bundle_engine::LocalBundleObjectStore) -> Self {
        Self {
            inner,
            next_failure: Arc::new(Mutex::new(None)),
        }
    }

    fn fail_next(&self, failure: StoreFailure) {
        *self.next_failure.lock().unwrap() = Some(failure);
    }

    fn take(&self, failure: StoreFailure) -> bool {
        let mut next = self.next_failure.lock().unwrap();
        if *next == Some(failure) {
            *next = None;
            true
        } else {
            false
        }
    }
}

impl BundleObjectStore for FailingStore {
    fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        self.inner.get(key)
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        if self.take(StoreFailure::Immutable) {
            return Err("injected immutable publication failure".to_string());
        }
        self.inner.put_immutable(key, bytes)
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        if self.take(StoreFailure::HeadCas) {
            return Err("injected head compare-and-swap failure".to_string());
        }
        self.inner.compare_and_swap(key, expectation, bytes)
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        self.inner.list(prefix, cursor, limit)
    }

    fn local_root(&self) -> Option<&Path> {
        self.inner.local_root()
    }
}
