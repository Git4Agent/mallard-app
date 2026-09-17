use super::harness::{
    assert_snapshot_consistent, chain, codex_scope, publish_request, publish_with_head, ThreadSpec,
    VariantWorld,
};
use crate::sync_tests::stub_s3::StubS3;
use mallard_domain::CloudThreadId;
use mallard_infrastructure::bundle_engine::{BundleEngine, PublishExpectation};
use mallard_infrastructure::storage_core::{
    CasExpectation, CasOutcome, DeleteOutcome, ExpectedObject, ImmutablePutOutcome, ObjectKey,
    ObjectPrefix, ObjectStore, S3Connection, S3ObjectStore, StageOutcome, StoreListPage,
    StoredMetadata,
};
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
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
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
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
    );
    let stale_request = publish_request(
        world.bundle_id.clone(),
        &[advanced, second_thread(&origin)],
        origin,
        21,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(Box::new(initial.snapshot.clone()), initial.head_token),
    );
    world.engine().publish(first_request).unwrap();
    // Both requests used the same apply observation. The later write loses at CAS.
    assert!(world
        .engine()
        .publish(stale_request)
        .unwrap_err()
        .contains("CAS conflict"));
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
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
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
    let store = S3ObjectStore::from_connection(
        &S3Connection {
            endpoint: &server.endpoint,
            region: "auto",
            bucket: "variant-bucket",
            access_key_id: "test-access-key",
            secret_access_key: "test-secret",
        },
        tokio::runtime::Handle::current(),
    )
    .unwrap();
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
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
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
        PublishExpectation::Match(Box::new(noop.snapshot.clone()), noop.head_token),
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
    let store = S3ObjectStore::from_connection(
        &S3Connection {
            endpoint: &server.endpoint,
            region: "auto",
            bucket: "variant-bucket",
            access_key_id: "test-access-key",
            secret_access_key: "test-secret",
        },
        tokio::runtime::Handle::current(),
    )
    .unwrap();
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
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
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
        PublishExpectation::Match(Box::new(initial.snapshot.clone()), initial.head_token),
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
    // A writer using the old apply observation must lose at CAS.
    assert!(stale_error.contains("CAS conflict"), "{stale_error}");

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
        PublishExpectation::Match(Box::new(after_a.snapshot.clone()), after_a.head_token),
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

    fn delete(&self, key: &ObjectKey, expected_etag: &str) -> Result<DeleteOutcome, String> {
        self.inner.delete(key, expected_etag)
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

/// The managed store answers the same protocol as the local and S3 stores:
/// a first publish creates the repository and commits, an unchanged
/// publish is a no-op, an advance fast-forwards through the upload and the
/// commit CAS, a stale writer loses at the head, and every conversation
/// pushed carries a session view the service can serve.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_cloud_noop_fast_forward_views_and_stale_writer_match_local_protocol() {
    use mallard_infrastructure::managed_cloud::{
        client::ManagedCloudClient, stub::StubManagedCloud, ManagedObjectStore,
    };
    let stub = StubManagedCloud::start().await;
    let (user, token) = stub.state.lock().unwrap().add_user("octocat");
    let client = ManagedCloudClient::new(
        &stub.endpoint,
        Some(mallard_domain::SecretValue::parse(&token).unwrap()),
    )
    .unwrap();
    let store = ManagedObjectStore::new(client, user.id.clone(), tokio::runtime::Handle::current());
    let world = VariantWorld::new();
    let origin = world.origin(&world.repo_a, "replica-a");
    let thread = ThreadSpec::new(
        "cloudthread-v1",
        "source-thread",
        chain("source-thread", &[("m-a", "one")]),
        origin.clone(),
        10,
    );
    let storage_id = mallard_domain::StorageId::parse("managed-variant-store").unwrap();
    let initial_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial = {
        let store = store.clone();
        let storage_id = storage_id.clone();
        tokio::task::spawn_blocking(move || {
            let engine = BundleEngine::open(store, storage_id).unwrap();
            publish_with_head(&engine, initial_request).unwrap()
        })
        .await
        .unwrap()
    };
    assert!(initial.published);
    assert_eq!(initial.snapshot.head.revision, 1);
    {
        let state = stub.state.lock().unwrap();
        let repository = state
            .repository_for_bundle(world.bundle_id.as_str())
            .expect("the first publish created the repository");
        assert_eq!(repository.class, "non_git_project");
        let head = repository.private_head.as_ref().unwrap();
        assert_eq!(head.manifest_revision, 1);
        assert_eq!(head.commit_id, initial.snapshot.head.commit_id);
        // The conversation's view was built from the wire bytes and stored.
        let resource_id = codex_scope(&initial.snapshot.manifest).conversations
            [&CloudThreadId::parse("cloudthread-v1").unwrap()]
            .resource_id
            .as_str()
            .to_owned();
        let (_, view_sha) = head.views.get(&resource_id).expect("view carried");
        let view: serde_json::Value = serde_json::from_slice(&repository.objects[view_sha]).unwrap();
        assert_eq!(view["schema_version"], 1);
        assert_eq!(view["provider"], "codex");
        assert_eq!(view["resource_id"], resource_id);
        // The harness transcripts are minimal Codex rollouts; the timeline
        // builder's event extraction is proved on a real wire fixture in
        // `managed_cloud::views`. Here the view exists, validates, and names
        // its transcript.
        assert!(view["events"].is_array());
        assert_eq!(view["event_counts"]["checkpoints"]["quality"], "unsupported");
    }

    let noop_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        world.origin(&world.repo_b, "replica-b"),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
    );
    let noop = {
        let store = store.clone();
        let storage_id = storage_id.clone();
        tokio::task::spawn_blocking(move || {
            let engine = BundleEngine::open(store, storage_id).unwrap();
            publish_with_head(&engine, noop_request).unwrap()
        })
        .await
        .unwrap()
    };
    assert!(!noop.published);
    assert_eq!(noop.snapshot.head.revision, 1);

    let mut advanced = thread.clone();
    advanced.transcript = chain("source-thread", &[("m-a", "one"), ("m-b", "continued")]);
    advanced.updated_at = 30;
    let advance_request = publish_request(
        world.bundle_id.clone(),
        &[advanced.clone()],
        origin.clone(),
        30,
        Some(&noop.snapshot.manifest),
        PublishExpectation::Match(Box::new(noop.snapshot.clone()), noop.head_token.clone()),
    );
    let stale_request = publish_request(
        world.bundle_id.clone(),
        &[advanced],
        origin,
        31,
        Some(&noop.snapshot.manifest),
        PublishExpectation::Match(Box::new(noop.snapshot.clone()), noop.head_token),
    );
    let after = {
        let store = store.clone();
        let storage_id = storage_id.clone();
        tokio::task::spawn_blocking(move || {
            let engine = BundleEngine::open(store, storage_id).unwrap();
            publish_with_head(&engine, advance_request).unwrap()
        })
        .await
        .unwrap()
    };
    assert!(after.published);
    assert_eq!(after.snapshot.head.revision, 2);
    // Both requests used the same apply observation. The later write loses
    // at the commit, and its upload is abandoned so the next push is free.
    let stale_error = {
        let store = store.clone();
        let storage_id = storage_id.clone();
        tokio::task::spawn_blocking(move || {
            BundleEngine::open(store, storage_id)
                .unwrap()
                .publish(stale_request)
                .unwrap_err()
        })
        .await
        .unwrap()
    };
    assert!(stale_error.contains("CAS conflict"), "{stale_error}");
    {
        let state = stub.state.lock().unwrap();
        assert!(state.uploads.values().all(|upload| upload.state != "open"));
        let repository = state.repository_for_bundle(world.bundle_id.as_str()).unwrap();
        assert_eq!(repository.private_head.as_ref().unwrap().manifest_revision, 2);
    }

    // A fresh store (another process) reads the head, the manifest, and
    // every object back through the service.
    let (user2, token2) = stub.state.lock().unwrap().add_user("rika321");
    let repository_id = stub
        .state
        .lock()
        .unwrap()
        .repository_for_bundle(world.bundle_id.as_str())
        .unwrap()
        .id
        .clone();
    stub.state.lock().unwrap().add_member(&repository_id, &user2.id, "member");
    let reader = ManagedObjectStore::new(
        ManagedCloudClient::new(
            &stub.endpoint,
            Some(mallard_domain::SecretValue::parse(&token2).unwrap()),
        )
        .unwrap(),
        user2.id,
        tokio::runtime::Handle::current(),
    );
    let bundle_id = world.bundle_id.clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(reader, storage_id).unwrap();
        let snapshot = engine.inspect(&bundle_id).unwrap();
        assert_snapshot_consistent(&engine, &snapshot);
        let listed = engine.list_remote_bundles().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].revision, 2);
        assert!(!listed[0].registration_only);
        snapshot
    })
    .await
    .unwrap();
    snapshot.validate().unwrap();
    assert_eq!(snapshot.head.commit_id, after.snapshot.head.commit_id);
    let requests = stub.requests();
    assert!(requests
        .iter()
        .filter(|request| request.path.ends_with("/commit"))
        .all(|request| request.idempotency_key.is_some()));
    assert!(!requests.iter().any(|request| request.path.contains("_tags")));
}
