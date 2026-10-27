use super::harness::{
    assert_snapshot_consistent, chain, codex_scope, publish_request, publish_with_head, ThreadSpec,
    VariantWorld,
};
use crate::sync_tests::stub_s3::StubS3;
use mallard_domain::CloudThreadId;
use mallard_infrastructure::bundle_engine::{BundleEngine, PublishExpectation};
use mallard_infrastructure::storage_core::publication::REMOTE_CHANGED;
use mallard_infrastructure::storage_core::{
    DeleteOutcome, ExpectedObject, ImmutablePutOutcome, ObjectKey, ObjectPrefix, ObjectStore,
    S3Connection, S3ObjectStore, StageOutcome, StoreListPage, StoredMetadata,
};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[test]
fn local_store_rejects_unreviewed_remote_thread_loss_and_a_stale_writer() {
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
    // Both requests used the same apply observation. The later writer lists a
    // child under its parent and stops before its record.
    assert_eq!(
        world.engine().publish(stale_request).unwrap_err(),
        REMOTE_CHANGED
    );
}

#[test]
fn local_store_immutable_and_record_failures_leave_a_retryable_fixed_point() {
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
    // The record request keeps failing: every attempt lists first and sends
    // once, and the operation ends with nothing published.
    store.fail_records(true);
    assert!(engine
        .publish(advanced_request.clone())
        .unwrap_err()
        .contains("injected record failure"));
    store.fail_records(false);
    let unchanged = engine.inspect(&world.bundle_id).unwrap();
    assert_eq!(unchanged.head.revision, 1);
    assert_eq!(unchanged.head.commit_id, initial.snapshot.head.commit_id);

    let retried = engine.publish(advanced_request).unwrap();
    assert_eq!(retried.snapshot().head.revision, 2);
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
    // The record is the publication: no head and no tag is ever written.
    let requests = server.requests();
    assert!(requests
        .iter()
        .any(|request| request.method == "PUT" && request.key.contains("/_commits/")));
    assert!(!requests
        .iter()
        .any(|request| request.key.ends_with("/_head.json") || request.key.contains("/_tags/")));
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
    // A writer using the old apply observation lists a child under its
    // parent and stops before its record.
    assert_eq!(stale_error, REMOTE_CHANGED);

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
    assert!(!server
        .requests()
        .iter()
        .any(|request| request.key.ends_with("/_head.json") || request.key.contains("/_tags/")));
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
}

#[derive(Clone)]
struct FailingStore {
    inner: mallard_infrastructure::storage_core::LocalObjectStore,
    next_failure: Arc<Mutex<Option<StoreFailure>>>,
    records_fail: Arc<Mutex<bool>>,
}

impl FailingStore {
    fn new(inner: mallard_infrastructure::storage_core::LocalObjectStore) -> Self {
        Self {
            inner,
            next_failure: Arc::new(Mutex::new(None)),
            records_fail: Arc::new(Mutex::new(false)),
        }
    }

    fn fail_next(&self, failure: StoreFailure) {
        *self.next_failure.lock().unwrap() = Some(failure);
    }

    fn fail_records(&self, fail: bool) {
        *self.records_fail.lock().unwrap() = fail;
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

    fn put_record(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        if *self.records_fail.lock().unwrap() {
            return Err("injected record failure".to_string());
        }
        self.inner.put_record(key, bytes)
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

/// The claim over the S3 client and a service that honors `If-None-Match`:
/// the parent of a Push is claimed with the record's bytes, a claim left by
/// a writer that died before its record stops the next Push onto that
/// version with nothing written, and a claim whose record is listed is
/// another computer's finished push.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stub_s3_claims_the_parent_and_reports_an_unfinished_push() {
    use mallard_infrastructure::storage_core::publication::{
        list_stream, read_record, record_name_of, RecordStream, MAX_RECORD_BYTES, PUSH_UNFINISHED,
    };
    use mallard_infrastructure::storage_core::ClaimOutcome;
    let temp = tempfile::tempdir().unwrap();
    let cloud = temp.path().join("cloud");
    fs::create_dir_all(cloud.join("variant-bucket")).unwrap();
    let server = StubS3::start(cloud).await;
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let t1 = ThreadSpec::new(
        "cloudthread-v1",
        "shared-source",
        chain("shared-source", &[("m-a", "base"), ("m-b", "answer")]),
        origin_a.clone(),
        10,
    );
    let storage_id = mallard_domain::StorageId::parse("s3-claim-store").unwrap();
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
    assert!(store.supports_claims());
    let bundle_id = world.bundle_id.clone();
    let stream = RecordStream::project(&bundle_id).unwrap();

    let initial_request = publish_request(
        bundle_id.clone(),
        std::slice::from_ref(&t1),
        origin_a.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let (initial_store, initial_storage) = (store.clone(), storage_id.clone());
    let initial = tokio::task::spawn_blocking(move || {
        let engine = BundleEngine::open(initial_store, initial_storage).unwrap();
        publish_with_head(&engine, initial_request).unwrap()
    })
    .await
    .unwrap();

    // The first version claimed the root with its own bytes.
    let (claim_store, claim_stream) = (store.clone(), stream.clone());
    let (selected, record_bytes, root_claim) = tokio::task::spawn_blocking(move || {
        let selected = list_stream(&claim_store, &claim_stream)
            .unwrap()
            .into_mainline()
            .selected()
            .cloned()
            .unwrap();
        let record_bytes = read_record(&claim_store, &claim_stream, &selected).unwrap();
        let root_claim = claim_store
            .get_metadata(&claim_stream.claim_key(None).unwrap(), MAX_RECORD_BYTES)
            .unwrap()
            .unwrap()
            .bytes;
        (selected, record_bytes, root_claim)
    })
    .await
    .unwrap();
    assert_eq!(root_claim, record_bytes);
    assert_eq!(selected.commit_id(), initial.snapshot.head.commit_id);

    // Another computer claimed the version and died before its record.
    let mut theirs: serde_json::Value = serde_json::from_slice(&record_bytes).unwrap();
    theirs["revision"] = serde_json::json!(selected.revision() + 1);
    theirs["commit_id"] = serde_json::Value::String(format!("commit-{}", "d".repeat(24)));
    theirs["manifest_key"] = serde_json::Value::String(format!(
        "_manifests/{}-commit-{}.json",
        selected.revision() + 1,
        "d".repeat(24)
    ));
    theirs["previous_commit_id"] = serde_json::Value::String(selected.commit_id().to_string());
    theirs["previous_record_sha256"] =
        serde_json::Value::String(selected.token().record_sha256().to_string());
    theirs["created_at_ms"] = serde_json::json!(selected.created_at_ms() + 1);
    theirs["published_by_origin"]["origin_replica_token"] =
        serde_json::Value::String("e".repeat(64));
    let their_bytes = serde_json::to_vec(&theirs).unwrap();
    let their_name = record_name_of(&their_bytes, Some(selected.token())).unwrap();
    let their_claim = stream.claim_key(Some(selected.token())).unwrap();
    let (plant_store, plant_key, plant_bytes) =
        (store.clone(), their_claim.clone(), their_bytes.clone());
    assert_eq!(
        tokio::task::spawn_blocking(move || plant_store.put_claim(&plant_key, &plant_bytes))
            .await
            .unwrap()
            .unwrap(),
        ClaimOutcome::Mine
    );

    // This computer's Push onto that version stops before its record: the
    // claim is taken and its record is listed nowhere.
    let mut advanced = t1.clone();
    advanced.transcript = chain(
        "shared-source",
        &[("m-a", "base"), ("m-b", "answer"), ("m-c", "more")],
    );
    let stopped_request = publish_request(
        bundle_id.clone(),
        &[advanced.clone()],
        origin_a.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
    );
    let (stopped_store, stopped_storage) = (store.clone(), storage_id.clone());
    let error = tokio::task::spawn_blocking(move || {
        BundleEngine::open(stopped_store, stopped_storage)
            .unwrap()
            .publish(stopped_request)
            .unwrap_err()
    })
    .await
    .unwrap();
    assert_eq!(error, PUSH_UNFINISHED);
    let records_before = server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.key.contains("/_commits/"))
        .count();
    assert_eq!(
        records_before, 1,
        "only the first version's record was ever sent"
    );

    // Finishing that push, the user's verb, writes their record once with
    // no claim. Once it is listed, the Push stops at its listing: another
    // computer got in first, and nothing claims or writes.
    let (finish_store, finish_storage, finish_bundle) =
        (store.clone(), storage_id.clone(), bundle_id.clone());
    tokio::task::spawn_blocking(move || {
        BundleEngine::open(finish_store, finish_storage)
            .unwrap()
            .finish_unfinished_push(&finish_bundle)
    })
    .await
    .unwrap()
    .unwrap();
    let record_puts = server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.key.contains("/_commits/"))
        .map(|request| request.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(record_puts.len(), 2);
    assert!(record_puts[1].ends_with(&their_name.file_name()));
    let changed_request = publish_request(
        bundle_id.clone(),
        &[advanced],
        origin_a,
        30,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(Box::new(initial.snapshot.clone()), initial.head_token),
    );
    let (changed_store, changed_storage) = (store.clone(), storage_id.clone());
    let error = tokio::task::spawn_blocking(move || {
        BundleEngine::open(changed_store, changed_storage)
            .unwrap()
            .publish(changed_request)
            .unwrap_err()
    })
    .await
    .unwrap();
    assert_eq!(error, REMOTE_CHANGED);
    let claims = server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.key.contains("/_claims/"))
        .count();
    assert_eq!(
        claims, 3,
        "the first version's, the planted one, and the refused one"
    );
}

/// Mallard Cloud is the R2 store's twin: the same protocol, the same
/// requests in the same order, and the session views beside the objects.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_cloud_noop_fast_forward_views_and_stale_writer_match_local_protocol() {
    use mallard_infrastructure::managed_cloud::client::ManagedCloudClient;
    use mallard_infrastructure::managed_cloud::stub::StubManagedCloud;
    use mallard_infrastructure::managed_cloud::ManagedObjectStore;

    let stub = StubManagedCloud::start().await;
    let (_, token) = stub.state.lock().unwrap().add_user("octocat");
    let client = ManagedCloudClient::new(
        &stub.endpoint,
        Some(mallard_domain::SecretValue::parse(&token).unwrap()),
    )
    .unwrap();
    let store = ManagedObjectStore::new(client, tokio::runtime::Handle::current());
    let world = VariantWorld::new();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let t1_id = CloudThreadId::parse("cloudthread-v1").unwrap();
    let thread = ThreadSpec::new(
        t1_id.as_str(),
        "source-thread",
        chain("source-thread", &[("m-a", "one")]),
        origin_a.clone(),
        10,
    );
    let storage_id = mallard_domain::StorageId::parse("managed-variant-store").unwrap();
    let open = |store: ManagedObjectStore, storage_id: mallard_domain::StorageId| {
        BundleEngine::open(store, storage_id).unwrap()
    };

    let initial_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin_a.clone(),
        10,
        None,
        PublishExpectation::Absent,
    );
    let initial = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        move || publish_with_head(&engine, initial_request).unwrap()
    })
    .await
    .unwrap();
    assert!(initial.published);
    let repository_id = stub
        .state
        .lock()
        .unwrap()
        .repository_for_bundle(world.bundle_id.as_str())
        .unwrap()
        .id
        .clone();

    // A replica that captured the same content publishes nothing: one
    // listing proves the compared publication is still selected.
    let noop_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&thread),
        origin_b.clone(),
        20,
        Some(&initial.snapshot.manifest),
        PublishExpectation::Match(
            Box::new(initial.snapshot.clone()),
            initial.head_token.clone(),
        ),
    );
    stub.clear_log();
    let noop = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        move || publish_with_head(&engine, noop_request).unwrap()
    })
    .await
    .unwrap();
    assert!(!noop.published);
    assert_eq!(noop.snapshot.head.revision, 1);
    assert!(stub
        .requests()
        .iter()
        .all(|request| request.method == "GET"));

    let mut advanced = thread.clone();
    advanced.transcript = chain("source-thread", &[("m-a", "one"), ("m-b", "continued")]);
    advanced.updated_at = 30;
    let advance_request = publish_request(
        world.bundle_id.clone(),
        std::slice::from_ref(&advanced),
        origin_a.clone(),
        30,
        Some(&noop.snapshot.manifest),
        PublishExpectation::Match(Box::new(noop.snapshot.clone()), noop.head_token.clone()),
    );
    stub.clear_log();
    let after_a = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        move || publish_with_head(&engine, advance_request).unwrap()
    })
    .await
    .unwrap();
    assert!(after_a.published);
    assert_eq!(after_a.snapshot.head.revision, 2);
    // The push: the object, then its view, then the manifest, then
    // listing, claim, record, listing, and nothing else. What follows is
    // the harness reading the head back through a store without a
    // process cache.
    let lines = stub.request_lines();
    let shape = lines
        .iter()
        .take(7)
        .map(|line| {
            let (verb, rest) = line.split_once(' ').unwrap();
            let class = [
                "/_objects/",
                "/_views/",
                "/_manifests/",
                "/_claims/",
                "/_commits/",
            ]
            .into_iter()
            .find(|class| rest.contains(class))
            .unwrap_or("?");
            if rest.contains("/store?prefix=") {
                format!("{verb} listing")
            } else {
                format!("{verb} {class}")
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shape,
        vec![
            "PUT /_objects/",
            "PUT /_views/",
            "PUT /_manifests/",
            "GET listing",
            "PUT /_claims/",
            "PUT /_commits/",
            "GET listing",
        ],
        "{lines:?}"
    );

    // A writer using the old apply observation lists a child under its
    // parent and stops before its record.
    let mut stale = thread;
    stale.transcript = chain("source-thread", &[("m-a", "one"), ("m-c", "stale")]);
    stale.updated_at = 31;
    let stale_request = publish_request(
        world.bundle_id.clone(),
        &[stale],
        origin_b,
        31,
        Some(&noop.snapshot.manifest),
        PublishExpectation::Match(Box::new(noop.snapshot.clone()), noop.head_token),
    );
    let stale_error = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        move || engine.publish(stale_request).unwrap_err()
    })
    .await
    .unwrap();
    assert_eq!(stale_error, REMOTE_CHANGED);

    let snapshot = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        let bundle_id = world.bundle_id.clone();
        move || {
            let snapshot = engine.inspect(&bundle_id).unwrap();
            assert_snapshot_consistent(&engine, &snapshot);
            snapshot
        }
    })
    .await
    .unwrap();
    snapshot.validate().unwrap();

    // The prefix holds the layout every storage has, plus a view per
    // pushed transcript; the index follows.
    let t1_path = codex_scope(&after_a.snapshot.manifest).conversations[&t1_id].files[0]
        .logical_path
        .clone();
    let transcript_sha = codex_scope(&after_a.snapshot.manifest).files[&t1_path]
        .sha256
        .clone();
    let state = stub.state.lock().unwrap();
    let repository = &state.repositories[&repository_id];
    let bundle = world.bundle_id.as_str();
    assert!(repository.objects.contains_key(".mallard/_storage.json"));
    assert!(repository.objects.contains_key(&format!(
        ".mallard/v1/repositories/{bundle}/_objects/{transcript_sha}"
    )));
    let view_prefix = format!(".mallard/v1/repositories/{bundle}/_views/1-{transcript_sha}-");
    assert_eq!(
        repository.objects.keys().filter(|key| key.starts_with(&view_prefix)).count(),
        1
    );
    assert_eq!(
        repository
            .keys_under(&format!(".mallard/v1/repositories/_commits/{bundle}"))
            .len(),
        2
    );
    assert_eq!(
        repository
            .keys_under(&format!(".mallard/v1/repositories/_claims/{bundle}"))
            .len(),
        2
    );
    assert_eq!(repository.private_revision, 2);
    assert_eq!(repository.head().unwrap().manifest_revision, 2);
    assert!(!repository
        .objects
        .keys()
        .any(|key| key.ends_with("/_head.json") || key.contains("/_tags/")));
}

/// The same sequence against a running Worker: a local `wrangler dev` or
/// staging. Ignored by default; run it with the origin and a desktop token
/// of a signed-in account, and it proves the desktop store and the service
/// agree on the wire (`STORE_API.md`), on a bundle it mints for the run:
///
/// ```sh
/// MALLARD_LIVE_CLOUD_ORIGIN=http://localhost:8787 MALLARD_LIVE_CLOUD_TOKEN=… \
///   cargo test --manifest-path src-tauri/Cargo.toml --lib managed_cloud_live_gateway -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a running Mallard Cloud Worker and a desktop token"]
async fn managed_cloud_live_gateway_round_trip() {
    use mallard_infrastructure::managed_cloud::client::ManagedCloudClient;
    use mallard_infrastructure::managed_cloud::ManagedObjectStore;
    use mallard_infrastructure::storage_core::publication::{list_stream, RecordStream};

    let origin = std::env::var("MALLARD_LIVE_CLOUD_ORIGIN").expect("MALLARD_LIVE_CLOUD_ORIGIN");
    let token = std::env::var("MALLARD_LIVE_CLOUD_TOKEN").expect("MALLARD_LIVE_CLOUD_TOKEN");
    let client = ManagedCloudClient::new(
        &origin,
        Some(mallard_domain::SecretValue::parse(&token).unwrap()),
    )
    .unwrap();
    let account = client.me().await.expect("the token signs in");
    let store = ManagedObjectStore::new(client, tokio::runtime::Handle::current());
    let mut world = VariantWorld::new();
    // A bundle of this run alone, so the account's repository list may hold
    // earlier runs.
    world.bundle_id = mallard_domain::BundleId::parse(
        mallard_domain::sha256_hex(format!("live-{}", mallard_domain::now_secs()).as_bytes())[..32]
            .to_string(),
    )
    .unwrap();
    let origin_a = world.origin(&world.repo_a, "replica-a");
    let origin_b = world.origin(&world.repo_b, "replica-b");
    let thread = ThreadSpec::new(
        "cloudthread-live",
        "source-thread",
        chain("source-thread", &[("m-a", "one")]),
        origin_a.clone(),
        10,
    );
    let storage_id = mallard_domain::StorageId::parse("managed-live-store").unwrap();
    let open = |store: ManagedObjectStore, storage_id: mallard_domain::StorageId| {
        BundleEngine::open(store, storage_id).unwrap()
    };

    let initial = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        let request = publish_request(
            world.bundle_id.clone(),
            std::slice::from_ref(&thread),
            origin_a.clone(),
            10,
            None,
            PublishExpectation::Absent,
        );
        move || publish_with_head(&engine, request).unwrap()
    })
    .await
    .unwrap();
    assert!(initial.published);
    eprintln!(
        "signed in as {}; bundle {} published at revision {}",
        account.user.github_login, world.bundle_id, initial.snapshot.head.revision
    );

    let noop = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        let request = publish_request(
            world.bundle_id.clone(),
            std::slice::from_ref(&thread),
            origin_b.clone(),
            20,
            Some(&initial.snapshot.manifest),
            PublishExpectation::Match(
                Box::new(initial.snapshot.clone()),
                initial.head_token.clone(),
            ),
        );
        move || publish_with_head(&engine, request).unwrap()
    })
    .await
    .unwrap();
    assert!(!noop.published);

    let mut advanced = thread.clone();
    advanced.transcript = chain("source-thread", &[("m-a", "one"), ("m-b", "continued")]);
    advanced.updated_at = 30;
    let after_a = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        let request = publish_request(
            world.bundle_id.clone(),
            std::slice::from_ref(&advanced),
            origin_a.clone(),
            30,
            Some(&noop.snapshot.manifest),
            PublishExpectation::Match(Box::new(noop.snapshot.clone()), noop.head_token.clone()),
        );
        move || publish_with_head(&engine, request).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(after_a.snapshot.head.revision, 2);

    // A writer on the old observation stops before its record.
    let mut stale = thread;
    stale.transcript = chain("source-thread", &[("m-a", "one"), ("m-c", "stale")]);
    stale.updated_at = 31;
    let stale_error = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        let request = publish_request(
            world.bundle_id.clone(),
            &[stale],
            origin_b,
            31,
            Some(&noop.snapshot.manifest),
            PublishExpectation::Match(Box::new(noop.snapshot.clone()), noop.head_token),
        );
        move || engine.publish(request).unwrap_err()
    })
    .await
    .unwrap();
    assert_eq!(stale_error, REMOTE_CHANGED);

    // The stream holds two records, both listed by the service; the
    // snapshot reads back whole; the repository is listed for discovery.
    let (listed, snapshot, discovered) = tokio::task::spawn_blocking({
        let engine = open(store.clone(), storage_id.clone());
        let bundle_id = world.bundle_id.clone();
        move || {
            let stream = RecordStream::project(&bundle_id).unwrap();
            let listed = list_stream(engine.store(), &stream).unwrap();
            let snapshot = engine.inspect(&bundle_id).unwrap();
            assert_snapshot_consistent(&engine, &snapshot);
            let discovered = engine
                .store()
                .list_project_repositories()
                .unwrap()
                .into_iter()
                .filter(|repository| repository.bundle_id.as_deref() == Some(bundle_id.as_str()))
                .count();
            (listed, snapshot, discovered)
        }
    })
    .await
    .unwrap();
    assert_eq!(listed.mainline().names().len(), 2);
    assert_eq!(snapshot.head.revision, 2);
    assert_eq!(discovered, 1);
    eprintln!("live round trip complete: two records, revision 2 selected, one repository listed");
}
