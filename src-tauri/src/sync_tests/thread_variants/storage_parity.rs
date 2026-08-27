use super::harness::{
    assert_snapshot_consistent, chain, codex_scope, codex_scope_key, harness_captured_with,
    publish_request, publish_with_head, scope_pin, ThreadSpec, VariantWorld,
};
use crate::sync_tests::stub_s3::StubS3;
use mallard_domain::{
    BundleRecipe, CloudThreadId, StorageConfig, StorageKind, Tombstone, TombstoneTarget,
};
use mallard_infrastructure::bundle_engine::{
    BundleEngine, PublishBundleRequest, PublishExpectation, ScopePublishInput,
};
use mallard_infrastructure::storage_core::{
    CasExpectation, CasOutcome, ExpectedObject, ImmutablePutOutcome, ObjectKey, ObjectPrefix,
    ObjectStore, S3ObjectStore, StageOutcome, StoreListPage, StoredMetadata,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[test]
fn local_store_rejects_unreviewed_remote_thread_loss_and_stale_cas() {
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let t1_id = CloudThreadId::parse("cloudthread-v1").unwrap();
    let t1 = ThreadSpec::new(
        t1_id.as_str(),
        "source-thread",
        chain("source-thread", &[("m-a", "one"), ("m-b1", "v1")]),
        origin.clone(),
        10,
    );
    let t2 = second_thread(&origin);
    let initial = world.publish(&[t1.clone(), t2], origin.clone(), 10);
    let dropping = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&t1),
        origin.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let error = world.engine().publish(dropping).unwrap_err();
    assert!(error.contains("drops conversation thread"));
    assert_eq!(
        world
            .engine()
            .inspect(&world.bundle_id)
            .unwrap()
            .head
            .revision,
        1
    );

    let mut advanced = t1;
    advanced.transcript = chain(
        "source-thread",
        &[("m-a", "one"), ("m-b1", "v1"), ("m-c1", "advance")],
    );
    advanced.updated_at = 20;
    let first_request = publish_request(
        world.bundle_id.clone(),
        &[advanced.clone(), second_thread(&origin)],
        origin.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let stale_request = publish_request(
        world.bundle_id.clone(),
        &[advanced, second_thread(&origin)],
        origin,
        21,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token),
    );
    world.engine().publish(first_request).unwrap();
    // The lost update now fails through the per-scope firewall — the stale
    // writer's reviewed Codex-scope pin no longer matches — instead of the
    // repository-wide head CAS.
    assert!(world
        .engine()
        .publish(stale_request)
        .unwrap_err()
        .contains("changed since review"));
}

#[test]
fn local_store_immutable_and_head_failures_leave_a_retryable_fixed_point() {
    let temp = tempfile::tempdir().unwrap();
    let inner =
        mallard_infrastructure::storage_core::LocalObjectStore::open(temp.path().join("store"))
            .unwrap();
    let store = FailingStore::new(inner);
    let storage_id = mallard_domain::StorageId::parse("failure-store").unwrap();
    let engine = BundleEngine::open(store.clone(), storage_id.clone()).unwrap();
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let thread = ThreadSpec::new(
        "cloudthread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "one")]),
        origin.clone(),
        10,
    );
    let initial_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    store.fail_next(StoreFailure::Immutable);
    assert!(engine.publish(initial_request.clone()).is_err());
    assert!(engine.read_head(&world.bundle_id).unwrap().is_none());

    let initial = publish_with_head(&engine, initial_request).unwrap();
    assert_eq!(initial.snapshot.head.revision, 1);
    let mut advanced = thread;
    advanced.transcript = chain("source-thread", &[("m-a", "one"), ("m-b", "continued")]);
    advanced.updated_at = 20;
    let advanced_request = publish_request(
        world.bundle_id.clone(),
        &[advanced],
        origin,
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    store.fail_next(StoreFailure::HeadCas);
    assert!(engine.publish(advanced_request.clone()).is_err());
    let unchanged = engine.inspect(&world.bundle_id).unwrap();
    assert_eq!(unchanged.head.revision, 1);
    assert_eq!(unchanged.head.commit_id, initial.snapshot.head.commit_id);

    let retried = engine.publish(advanced_request).unwrap();
    assert_eq!(retried.snapshot.head.revision, 2);
    let snapshot = engine.inspect(&world.bundle_id).unwrap();
    assert_snapshot_consistent(&engine, &snapshot);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_current_schema_noop_and_fast_forward_match_local_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let thread = ThreadSpec::new(
        "cloudthread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "one")]),
        origin.clone(),
        10,
    );
    let storage_id = mallard_domain::StorageId::parse("s3-variant-store").unwrap();
    let config = StorageConfig {
        id: storage_id.clone(),
        name: "Thread stub S3".to_string(),
        kind: StorageKind::S3,
        bucket: "variant-bucket".to_string(),
        access_key_id: "test-access-key".to_string(),
        secret_access_key: "test-secret".to_string(),
        account_id: String::new(),
        s3_endpoint: server.endpoint.clone(),
        region: "auto".to_string(),
        local_dir: String::new(),
        supports_conditional_writes: None,
    };
    let store = S3ObjectStore::from_config(&config, tokio::runtime::Handle::current()).unwrap();
    let initial_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial_store = store.clone();
    let initial_storage = storage_id.clone();
    let initial = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(initial_store, initial_storage).unwrap();
        publish_with_head(&engine, initial_request).unwrap()
    })
    .await
    .unwrap();
    assert!(initial.published);

    let noop_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        world.origin(&world.repo_b, "replica-b"),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let noop_store = store.clone();
    let noop_storage = storage_id.clone();
    let noop = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(noop_store, noop_storage).unwrap();
        publish_with_head(&engine, noop_request).unwrap()
    })
    .await
    .unwrap();
    assert!(!noop.published);
    assert_eq!(noop.snapshot.head.revision, 1);

    let mut advanced = thread;
    advanced.transcript = chain("source-thread", &[("m-a", "one"), ("m-b", "continued")]);
    advanced.updated_at = 30;
    let advance_request = publish_request(
        world.bundle_id.clone(),
        &[advanced],
        origin,
        30,
        Some(&noop.snapshot.manifest),
        PublishExpectation::Match(noop.head_token),
    );
    let advance_store = store.clone();
    let advance_storage = storage_id.clone();
    let advanced = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(advance_store, advance_storage).unwrap();
        publish_with_head(&engine, advance_request).unwrap()
    })
    .await
    .unwrap();
    assert!(advanced.published);
    assert_eq!(advanced.snapshot.head.revision, 2);

    let fetch_store = store;
    let snapshot = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(fetch_store, storage_id).unwrap();
        let snapshot = engine.inspect(&world.bundle_id).unwrap();
        assert_snapshot_consistent(&engine, &snapshot);
        snapshot
    })
    .await
    .unwrap();
    snapshot.validate().unwrap();
    assert!(server.requests().iter().any(|request| {
        request.method == "PUT" && request.key.ends_with("/_head.json") && request.conditional
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_retains_remote_threads_and_one_stale_writer_replans() {
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let t1_id = CloudThreadId::parse("cloudthread-v1").unwrap();
    let t2_id = CloudThreadId::parse("cloudthread-v2").unwrap();
    let t1 = ThreadSpec::new(
        t1_id.as_str(),
        "shared-source",
        chain(
            "shared-source",
            &[("m-a", "base"), ("m-b", "answer"), ("m-v1", "branch A")],
        ),
        origin_a.clone(),
        10,
    );
    let t2 = ThreadSpec::new(
        t2_id.as_str(),
        "shared-source",
        chain(
            "shared-source",
            &[("m-a", "base"), ("m-b", "answer"), ("m-v2", "branch B")],
        ),
        origin_b.clone(),
        10,
    );
    let storage_id = mallard_domain::StorageId::parse("s3-cas-store").unwrap();
    let config = StorageConfig {
        id: storage_id.clone(),
        name: "Thread CAS stub S3".to_string(),
        kind: StorageKind::S3,
        bucket: "variant-bucket".to_string(),
        access_key_id: "test-access-key".to_string(),
        secret_access_key: "test-secret".to_string(),
        account_id: String::new(),
        s3_endpoint: server.endpoint.clone(),
        region: "auto".to_string(),
        local_dir: String::new(),
        supports_conditional_writes: None,
    };
    let store = S3ObjectStore::from_config(&config, tokio::runtime::Handle::current()).unwrap();
    let initial_request = publish_request(
        world.bundle_id.clone(),
        &[t1.clone(), t2.clone()],
        origin_a.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial_store = store.clone();
    let initial_storage = storage_id.clone();
    let initial = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(initial_store, initial_storage).unwrap();
        publish_with_head(&engine, initial_request).unwrap()
    })
    .await
    .unwrap();

    let mut advanced_t1 = t1.clone();
    advanced_t1.transcript = chain(
        "shared-source",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-v1", "branch A"),
            ("m-a2", "A continues"),
        ],
    );
    advanced_t1.updated_at = 20;
    let writer_a = publish_request(
        world.bundle_id.clone(),
        &[advanced_t1.clone(), t2.clone()],
        origin_a,
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token.clone()),
    );
    let mut advanced_t2 = t2.clone();
    advanced_t2.transcript = chain(
        "shared-source",
        &[
            ("m-a", "base"),
            ("m-b", "answer"),
            ("m-v2", "branch B"),
            ("m-b2", "B continues"),
        ],
    );
    advanced_t2.updated_at = 21;
    let stale_writer_b = publish_request(
        world.bundle_id.clone(),
        &[t1, advanced_t2.clone()],
        origin_b.clone(),
        21,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token),
    );
    let writer_a_store = store.clone();
    let writer_a_storage = storage_id.clone();
    let after_a = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(writer_a_store, writer_a_storage).unwrap();
        publish_with_head(&engine, writer_a).unwrap()
    })
    .await
    .unwrap();
    let writer_b_store = store.clone();
    let writer_b_storage = storage_id.clone();
    let stale_error = tokio::task::spawn_blocking(move || {
        BundleEngine::open(writer_b_store, writer_b_storage)
            .unwrap()
            .publish(stale_writer_b)
            .unwrap_err()
    })
    .await
    .unwrap();
    // Both writers change the same Codex scope, so the stale writer is
    // rejected by that scope's reviewed pin rather than the head CAS.
    assert!(
        stale_error.contains("changed since review") || stale_error.contains("conflict"),
        "{stale_error}"
    );

    let t1_path = codex_scope(&after_a.snapshot.manifest).conversations[&t1_id].files[0]
        .logical_path
        .clone();
    let t1_object = codex_scope(&after_a.snapshot.manifest).files[&t1_path]
        .object_key
        .clone();
    let retry_b = publish_request(
        world.bundle_id.clone(),
        &[advanced_t1, advanced_t2],
        origin_b,
        30,
        Some(&after_a.snapshot.manifest),
        PublishExpectation::Match(after_a.head_token),
    );
    let retry_store = store.clone();
    let retry_storage = storage_id.clone();
    let after_b = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(retry_store, retry_storage).unwrap();
        publish_with_head(&engine, retry_b).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(after_b.snapshot.head.revision, 3);
    assert_eq!(
        codex_scope(&after_b.snapshot.manifest).conversations.len(),
        2
    );
    assert!(codex_scope(&after_b.snapshot.manifest)
        .conversations
        .contains_key(&t2_id));
    assert_eq!(
        codex_scope(&after_b.snapshot.manifest).files[&t1_path].object_key,
        t1_object
    );

    let fetch_store = store;
    let snapshot = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(fetch_store, storage_id).unwrap();
        let snapshot = engine.inspect(&world.bundle_id).unwrap();
        assert_snapshot_consistent(&engine, &snapshot);
        snapshot
    })
    .await
    .unwrap();
    snapshot.validate().unwrap();
    assert!(server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.key.ends_with("/_head.json"))
        .all(|request| request.conditional));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_fork_and_tombstone_resurrection_match_local() {
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let cloud_id = CloudThreadId::parse("cloudthread-shared").unwrap();
    let forked_id = CloudThreadId::parse("cloudthread-forked-local").unwrap();
    let cloud_thread = ThreadSpec::new(
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
    let mut forked_thread = ThreadSpec::new(
        forked_id.as_str(),
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
    forked_thread.forked_from = Some(cloud_id.clone());
    let storage_id = mallard_domain::StorageId::parse("s3-divergence-store").unwrap();
    let config = StorageConfig {
        id: storage_id.clone(),
        name: "Thread divergence stub S3".to_string(),
        kind: StorageKind::S3,
        bucket: "variant-bucket".to_string(),
        access_key_id: "test-access-key".to_string(),
        secret_access_key: "test-secret".to_string(),
        account_id: String::new(),
        s3_endpoint: server.endpoint.clone(),
        region: "auto".to_string(),
        local_dir: String::new(),
        supports_conditional_writes: None,
    };
    let store = S3ObjectStore::from_config(&config, tokio::runtime::Handle::current()).unwrap();
    let initial_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&cloud_thread),
        origin_b,
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial_store = store.clone();
    let initial_storage = storage_id.clone();
    let initial = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(initial_store, initial_storage).unwrap();
        publish_with_head(&engine, initial_request).unwrap()
    })
    .await
    .unwrap();
    let cloud_path = codex_scope(&initial.snapshot.manifest).conversations[&cloud_id].files[0]
        .logical_path
        .clone();
    let cloud_object = codex_scope(&initial.snapshot.manifest).files[&cloud_path]
        .object_key
        .clone();

    let fork_request = publish_request(
        world.bundle_id.clone(),
        &[cloud_thread.clone(), forked_thread.clone()],
        origin_a.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(initial.head_token),
    );
    let fork_store = store.clone();
    let fork_storage = storage_id.clone();
    let forked = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(fork_store, fork_storage).unwrap();
        publish_with_head(&engine, fork_request).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(forked.snapshot.head.revision, 2);
    assert_eq!(
        codex_scope(&forked.snapshot.manifest).conversations.len(),
        2
    );
    assert_eq!(
        codex_scope(&forked.snapshot.manifest).conversations[&forked_id].forked_from,
        Some(cloud_id.clone())
    );
    assert_eq!(
        codex_scope(&forked.snapshot.manifest).files[&cloud_path].object_key,
        cloud_object,
    );

    let no_op_request = publish_request(
        world.bundle_id.clone(),
        &[cloud_thread.clone(), forked_thread.clone()],
        origin_a.clone(),
        25,
        Some(&forked.snapshot.manifest),
        PublishExpectation::Match(forked.head_token),
    );
    let no_op_store = store.clone();
    let no_op_storage = storage_id.clone();
    let no_op = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(no_op_store, no_op_storage).unwrap();
        publish_with_head(&engine, no_op_request).unwrap()
    })
    .await
    .unwrap();
    assert!(!no_op.published);
    assert_eq!(no_op.snapshot.head.revision, 2);

    let cloud_tombstone = Tombstone {
        target: TombstoneTarget::ConversationThread {
            thread_id: cloud_id.clone(),
        },
        last_sha256: Some(super::harness::sha256(&cloud_thread.transcript)),
        deleted_at: 30,
    };
    let forked_tombstone = Tombstone {
        target: TombstoneTarget::ConversationThread {
            thread_id: forked_id.clone(),
        },
        last_sha256: Some(super::harness::sha256(&forked_thread.transcript)),
        deleted_at: 30,
    };
    let delete_request = PublishBundleRequest {
        identity: no_op.snapshot.manifest.bundle.clone(),
        published_by_origin: origin_a.clone(),
        reviewed_scope_pins: BTreeMap::from([(
            codex_scope_key(),
            scope_pin(Some(&no_op.snapshot.manifest), &codex_scope_key()),
        )]),
        writable_scopes: BTreeMap::from([(
            codex_scope_key(),
            ScopePublishInput {
                recipe: BundleRecipe::default(),
                captured_with: harness_captured_with(),
                tombstones: BTreeMap::from([
                    (cloud_tombstone.canonical_key(), cloud_tombstone),
                    (forked_tombstone.canonical_key(), forked_tombstone),
                ]),
                ..ScopePublishInput::default()
            },
        )]),
        expected_head: PublishExpectation::Match(no_op.head_token),
        updated_at: 30,
    };
    let delete_store = store.clone();
    let delete_storage = storage_id.clone();
    let deleted = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(delete_store, delete_storage).unwrap();
        publish_with_head(&engine, delete_request).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(deleted.snapshot.head.revision, 3);
    assert_eq!(deleted.snapshot.manifest.all_conversations().count(), 0);

    let mut modified = forked_thread;
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
    let resurrect_request = publish_request(
        world.bundle_id.clone(),
        &[modified.clone()],
        origin_a.clone(),
        40,
        Some(&deleted.snapshot.manifest),
        PublishExpectation::Match(deleted.head_token),
    );
    let resurrect_store = store.clone();
    let resurrect_storage = storage_id.clone();
    let resurrected = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(resurrect_store, resurrect_storage).unwrap();
        publish_with_head(&engine, resurrect_request).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(resurrected.snapshot.head.revision, 4);
    assert!(codex_scope(&resurrected.snapshot.manifest)
        .conversations
        .contains_key(&forked_id));
    assert!(!resurrected
        .snapshot
        .manifest
        .all_tombstones()
        .any(|(_, _, tombstone)| matches!(
            &tombstone.target,
            TombstoneTarget::ConversationThread { thread_id }
                if thread_id.as_str() == forked_id.as_str()
        )));

    let retry_request = publish_request(
        world.bundle_id.clone(),
        &[modified],
        origin_a,
        50,
        Some(&resurrected.snapshot.manifest),
        PublishExpectation::Match(resurrected.head_token),
    );
    let retry_store = store.clone();
    let retried = tokio::task::spawn_blocking(move || {
        BundleEngine::open(retry_store, storage_id)
            .unwrap()
            .publish(retry_request)
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!retried.published);
    let snapshot = {
        let fetch_store = store;
        let fetch_storage = config.id.clone();
        let fetch_bundle = world.bundle_id.clone();
        tokio::task::spawn_blocking(move || {
            let engine = BundleEngine::open(fetch_store, fetch_storage).unwrap();
            let snapshot = engine.inspect(&fetch_bundle).unwrap();
            assert_snapshot_consistent(&engine, &snapshot);
            snapshot
        })
        .await
        .unwrap()
    };
    snapshot.validate().unwrap();
    assert!(server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.key.ends_with("/_head.json"))
        .all(|request| request.conditional));
}

fn second_thread(origin: &mallard_domain::PortableOrigin) -> ThreadSpec {
    ThreadSpec::new(
        "cloudthread-v2",
        "source-thread-v2",
        chain("source-thread-v2", &[("m-a", "one"), ("m-b2", "v2")]),
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
    inner: mallard_infrastructure::storage_core::LocalObjectStore,
    next_failure: Arc<Mutex<Option<StoreFailure>>>,
}

impl FailingStore {
    fn new(inner: mallard_infrastructure::storage_core::LocalObjectStore) -> Self {
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

impl ObjectStore for FailingStore {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        self.inner.get_metadata(key, max_bytes)
    }

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        self.inner.stage_verified(key, expected)
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

    fn delete(&self, key: &ObjectKey) -> Result<(), String> {
        self.inner.delete(key)
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
