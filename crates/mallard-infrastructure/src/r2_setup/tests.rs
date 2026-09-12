use super::discovery::MarkerState;
use super::*;
use crate::storage_names::valid_word;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const DATE: &str = "2026-09-10T00:00:00Z";

#[derive(Default)]
struct State {
    buckets: BTreeMap<String, String>,
    /// Buckets listed under a non-default jurisdiction, by jurisdiction.
    elsewhere: BTreeMap<String, Vec<String>>,
    /// Jurisdictions the account cannot use; they refuse the list outright.
    unavailable: Vec<String>,
    attempted: Vec<String>,
    conflict_on_create: usize,
    fail_after_create: bool,
    reject: Option<u16>,
    requests: usize,
    markers: BTreeMap<String, (u16, Value)>,
    repeat_cursor: bool,
    deny_list: bool,
    /// The jurisdiction header of every creation request, in order.
    created_in: Vec<String>,
}

struct Mock {
    state: Arc<Mutex<State>>,
    runtime: tokio::runtime::Runtime,
    client: R2SetupClient,
}

fn reply(status: u16, value: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}

async fn serve(
    request: Request<Incoming>,
    state: Arc<Mutex<State>>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let (parts, body) = request.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    assert_eq!(parts.headers["authorization"], "Bearer token-canary");
    assert!(parts
        .uri
        .path()
        .starts_with(&format!("/accounts/{ACCOUNT}/r2/buckets")));
    let jurisdiction = parts.headers["cf-r2-jurisdiction"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(R2Jurisdiction::parse(&jurisdiction).is_some());
    let mut state = state.lock().unwrap();
    state.requests += 1;
    if parts.method == Method::GET {
        if parts.uri.path().ends_with("/buckets") {
            if state.deny_list {
                return Ok(reply(403, json!({"success":false})));
            }
            if state.unavailable.contains(&jurisdiction) {
                return Ok(reply(
                    400,
                    json!({"success":false,"errors":[{"code":10086}]}),
                ));
            }
            let buckets: Vec<_> = if jurisdiction == "default" {
                state
                    .buckets
                    .iter()
                    .map(|(name, date)| json!({"name":name,"creation_date":date}))
                    .collect()
            } else {
                state
                    .elsewhere
                    .get(&jurisdiction)
                    .into_iter()
                    .flatten()
                    .map(|name| json!({"name":name,"creation_date":DATE,"jurisdiction":jurisdiction}))
                    .collect()
            };
            return Ok(reply(
                200,
                json!({"success":true,"result":{"buckets":buckets},
                "result_info":{"cursor":if state.repeat_cursor && jurisdiction == "default" { "same" } else { "" }}}),
            ));
        }
        if let Some((prefix, _)) = parts.uri.path().split_once("/objects/") {
            assert!(parts
                .uri
                .path()
                .ends_with("/objects/.mallard/_storage.json"));
            let bucket = prefix.rsplit('/').next().unwrap();
            let (status, marker) = state
                .markers
                .get(bucket)
                .cloned()
                .unwrap_or((404, json!({})));
            return Ok(reply(status, marker));
        }
        let name = parts.uri.path().rsplit('/').next().unwrap();
        if jurisdiction != "default" {
            // A bucket answers only under the jurisdiction it lives in.
            let listed = state
                .elsewhere
                .get(&jurisdiction)
                .is_some_and(|names| names.iter().any(|listed| listed == name));
            return Ok(if listed {
                reply(
                    200,
                    json!({"success":true,"result":{"name":name,"creation_date":DATE}}),
                )
            } else {
                reply(404, json!({"success":false,"errors":[{"code":10006}]}))
            });
        }
        let date = state.buckets.get(name).cloned();
        return Ok(match date {
            Some(date) => reply(
                200,
                json!({"success":true,"result":{"name":name,"creation_date":date}}),
            ),
            None => reply(404, json!({"success":false,"errors":[{"code":10006}]})),
        });
    }
    assert_eq!(parts.method, Method::POST);
    let args: Value = serde_json::from_slice(&bytes).unwrap();
    let name = args["name"].as_str().unwrap().to_string();
    state.attempted.push(name.clone());
    state.created_in.push(jurisdiction.clone());
    if let Some(status) = state.reject {
        return Ok(reply(
            status,
            json!({"success":false,"errors":[{"code":10001,"message":"secret-canary"}]}),
        ));
    }
    if state.conflict_on_create > 0 {
        state.conflict_on_create -= 1;
        state.buckets.insert(name.clone(), DATE.into());
        return Ok(reply(
            400,
            json!({"success":false,"errors":[{"code":10004}]}),
        ));
    }
    assert!(
        !state.buckets.contains_key(&name),
        "must not replace an existing bucket"
    );
    state.buckets.insert(name.clone(), DATE.into());
    if state.fail_after_create {
        return Ok(reply(500, json!({"success":false})));
    }
    Ok(reply(
        200,
        json!({"success":true,"result":{"name":name,"creation_date":DATE}}),
    ))
}

impl Mock {
    fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let address = listener.local_addr().unwrap();
        let shared = state.clone();
        runtime.spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let shared = shared.clone();
                tokio::spawn(async move {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |request| serve(request, shared.clone())),
                        )
                        .await;
                });
            }
        });
        let client = R2SetupClient::with_endpoint(
            SecretValue::parse("token-canary").unwrap(),
            &format!("http://{address}"),
        )
        .unwrap();
        Self {
            state,
            runtime,
            client,
        }
    }
    fn prepare(
        &self,
        journal: &mut R2SetupJournal,
        name: &str,
    ) -> Result<R2Destination, SetupError> {
        self.runtime
            .block_on(prepare_bucket(&self.client, journal, name))
    }
    fn discover(&self) -> Result<discovery::BucketScan, SetupError> {
        self.runtime.block_on(self.client.discover(ACCOUNT))
    }
    fn verify(&self, jurisdiction: R2Jurisdiction, bucket: &str, date: &str) -> bool {
        self.runtime
            .block_on(
                self.client
                    .verify_existing(ACCOUNT, jurisdiction, bucket, date),
            )
            .is_ok()
    }
}

fn draft_id() -> StorageSetupDraftId {
    StorageSetupDraftId::parse("storagedraft-111111111111111111111111").unwrap()
}

fn open(repository: &LocalStateRepository) -> R2SetupJournal {
    R2SetupJournal::open(repository, &draft_id(), ACCOUNT, R2Jurisdiction::Default).unwrap()
}

#[test]
fn success_and_restart_keep_the_same_bucket_and_no_secrets() {
    let mock = Mock::new();
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let first = mock
        .prepare(&mut open(&repository), "mallard-otter")
        .unwrap();
    assert_eq!(first.bucket(), "mallard-otter");
    let mut resumed = open(&repository);
    assert_eq!(
        mock.prepare(&mut resumed, "mallard-otter")
            .unwrap()
            .api_url(),
        first.api_url()
    );
    assert_eq!(mock.state.lock().unwrap().attempted.len(), 1);
    let entries = StorageSetupJournalStore::r2_provisioning(&repository)
        .list()
        .unwrap();
    assert_eq!(entries[0].schema, 1);
    assert!(!serde_json::to_string(&entries)
        .unwrap()
        .contains("token-canary"));
    resumed.finish().unwrap();
    assert!(StorageSetupJournalStore::r2_provisioning(&repository)
        .list()
        .unwrap()
        .is_empty());
}

#[test]
fn proposals_are_readable_mallard_names() {
    for _ in 0..8 {
        let name = propose_bucket_name().unwrap();
        assert!(valid_word(name.strip_prefix("mallard-").unwrap()));
        assert!(destination(ACCOUNT, R2Jurisdiction::Default, &name).is_ok());
    }
}

#[test]
fn a_taken_name_is_reported_and_a_renamed_request_replaces_the_plan() {
    let mock = Mock::new();
    mock.state
        .lock()
        .unwrap()
        .buckets
        .insert("team-backups".into(), DATE.into());
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let mut journal = open(&repository);
    assert_eq!(
        mock.prepare(&mut journal, "team-backups").unwrap_err(),
        SetupError::Taken
    );
    assert!(mock.state.lock().unwrap().attempted.is_empty());
    let saved = journal.recovery().unwrap().unwrap();
    assert_eq!(saved.bucket, "team-backups");
    assert!(matches!(saved.stage, BucketStage::Planned));
    let created = mock
        .prepare(&mut open(&repository), "team-backups-2")
        .unwrap();
    assert_eq!(created.bucket(), "team-backups-2");
    assert_eq!(mock.state.lock().unwrap().attempted, ["team-backups-2"]);
}

#[test]
fn a_creation_race_reports_the_name_taken_without_adopting_the_conflict() {
    let mock = Mock::new();
    mock.state.lock().unwrap().conflict_on_create = 1;
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let mut journal = open(&repository);
    assert_eq!(
        mock.prepare(&mut journal, "mallard-otter").unwrap_err(),
        SetupError::Taken
    );
    assert_eq!(mock.state.lock().unwrap().attempted, ["mallard-otter"]);
    assert!(matches!(
        journal.recovery().unwrap().unwrap().stage,
        BucketStage::Planned
    ));
    let created = mock.prepare(&mut journal, "mallard-brave").unwrap();
    assert_eq!(created.bucket(), "mallard-brave");
    assert_eq!(mock.state.lock().unwrap().attempted.len(), 2);
}

#[test]
fn two_drafts_asking_for_the_same_name_make_one_bucket() {
    let mock = Mock::new();
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let mut first = open(&repository);
    let second_id = StorageSetupDraftId::parse("storagedraft-222222222222222222222222").unwrap();
    let mut second =
        R2SetupJournal::open(&repository, &second_id, ACCOUNT, R2Jurisdiction::Default).unwrap();
    assert_eq!(
        mock.prepare(&mut first, "mallard-otter").unwrap().bucket(),
        "mallard-otter"
    );
    assert_eq!(
        mock.prepare(&mut second, "mallard-otter").unwrap_err(),
        SetupError::Taken
    );
    assert_eq!(mock.state.lock().unwrap().attempted.len(), 1);
    // The first draft still resumes its own bucket.
    assert_eq!(
        mock.prepare(&mut first, "mallard-otter").unwrap().bucket(),
        "mallard-otter"
    );
}

#[test]
fn unknown_creation_is_never_repeated_after_restart() {
    let mock = Mock::new();
    mock.state.lock().unwrap().fail_after_create = true;
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    assert_eq!(
        mock.prepare(&mut open(&repository), "mallard-otter")
            .unwrap_err(),
        SetupError::Uncertain
    );
    // Neither the same name nor another one may create again.
    for name in ["mallard-otter", "mallard-brave"] {
        assert_eq!(
            mock.prepare(&mut open(&repository), name).unwrap_err(),
            SetupError::Uncertain
        );
    }
    assert_eq!(mock.state.lock().unwrap().attempted.len(), 1);
}

#[test]
fn other_errors_keep_the_name_and_do_not_retry_automatically() {
    for status in [400, 401, 403, 409, 429, 500] {
        let mock = Mock::new();
        mock.state.lock().unwrap().reject = Some(status);
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let mut journal = open(&repository);
        let error = mock.prepare(&mut journal, "mallard-otter").unwrap_err();
        assert!(!format!("{error:?}").contains("secret-canary"));
        assert_ne!(error, SetupError::Taken);
        let saved = journal.recovery().unwrap().unwrap();
        assert_eq!(saved.bucket, "mallard-otter");
        assert_eq!(mock.state.lock().unwrap().attempted.len(), 1);
        if status < 500 {
            assert!(matches!(saved.stage, BucketStage::Planned));
            assert!(mock
                .prepare(&mut open(&repository), "mallard-otter")
                .is_err());
            assert!(mock
                .state
                .lock()
                .unwrap()
                .attempted
                .iter()
                .all(|name| name == "mallard-otter"));
        } else {
            assert!(matches!(saved.stage, BucketStage::Creating));
        }
    }
}

#[test]
fn changed_account_jurisdiction_or_name_is_refused_once_a_bucket_exists() {
    let mock = Mock::new();
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let created = mock
        .prepare(&mut open(&repository), "mallard-otter")
        .unwrap();
    assert!(matches!(
        R2SetupJournal::open(
            &repository,
            &draft_id(),
            "fedcba9876543210fedcba9876543210",
            R2Jurisdiction::Default
        ),
        Err(SetupError::Identity)
    ));
    // The jurisdiction is a fact of the setup too: it cannot move after a
    // bucket exists in one.
    assert!(matches!(
        R2SetupJournal::open(&repository, &draft_id(), ACCOUNT, R2Jurisdiction::Eu),
        Err(SetupError::Identity)
    ));
    // Nor can the bucket be renamed once it exists.
    assert_eq!(
        mock.prepare(&mut open(&repository), "mallard-brave")
            .unwrap_err(),
        SetupError::Identity
    );
    mock.state
        .lock()
        .unwrap()
        .buckets
        .insert(created.bucket().into(), "2026-09-11T00:00:00Z".into());
    assert_eq!(
        mock.prepare(&mut open(&repository), "mallard-otter")
            .unwrap_err(),
        SetupError::Identity
    );
    assert_eq!(mock.state.lock().unwrap().attempted.len(), 1);
}

#[test]
fn a_journal_that_created_nothing_follows_the_latest_account_and_jurisdiction() {
    let mock = Mock::new();
    mock.state
        .lock()
        .unwrap()
        .buckets
        .insert("mallard-otter".into(), DATE.into());
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    assert_eq!(
        mock.prepare(&mut open(&repository), "mallard-otter")
            .unwrap_err(),
        SetupError::Taken
    );
    // Only a plan was written: the EU request opens the same journal afresh
    // and creates there.
    let mut moved =
        R2SetupJournal::open(&repository, &draft_id(), ACCOUNT, R2Jurisdiction::Eu).unwrap();
    assert!(moved.recovery().unwrap().is_none());
    let created = mock.prepare(&mut moved, "mallard-brave").unwrap();
    assert_eq!(created.jurisdiction(), R2Jurisdiction::Eu);
    assert_eq!(
        mock.state.lock().unwrap().created_in,
        vec!["eu".to_string()]
    );
    // From here on the facts are pinned, as a bucket exists.
    assert!(matches!(
        R2SetupJournal::open(
            &repository,
            &draft_id(),
            "fedcba9876543210fedcba9876543210",
            R2Jurisdiction::Eu
        ),
        Err(SetupError::Identity)
    ));
    // Before anything exists, another account is followed too.
    let other = tempfile::tempdir().unwrap();
    let fresh = LocalStateRepository::from_home_dir(other.path()).unwrap();
    assert_eq!(
        mock.prepare(&mut open(&fresh), "mallard-otter")
            .unwrap_err(),
        SetupError::Taken
    );
    let followed = R2SetupJournal::open(
        &fresh,
        &draft_id(),
        "fedcba9876543210fedcba9876543210",
        R2Jurisdiction::Default,
    )
    .unwrap();
    assert!(followed.recovery().unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn journal_rejects_symlink_directory() {
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    std::os::unix::fs::symlink(temp.path(), repository.root().join("r2_setup_journal")).unwrap();
    assert!(
        R2SetupJournal::open(&repository, &draft_id(), ACCOUNT, R2Jurisdiction::Default).is_err()
    );
}

#[test]
fn a_bucket_is_created_in_the_journal_jurisdiction() {
    let mock = Mock::new();
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let mut journal =
        R2SetupJournal::open(&repository, &draft_id(), ACCOUNT, R2Jurisdiction::Eu).unwrap();
    let created = mock.prepare(&mut journal, "mallard-otter").unwrap();
    assert_eq!(created.jurisdiction(), R2Jurisdiction::Eu);
    assert!(created
        .api_url()
        .starts_with(&format!("https://{ACCOUNT}.eu.r2.cloudflarestorage.com/")));
    assert_eq!(
        mock.state.lock().unwrap().created_in,
        vec!["eu".to_string()]
    );
}

#[test]
fn discovery_reads_only_markers_and_revalidates_bucket_identity() {
    let mock = Mock::new();
    {
        let mut state = mock.state.lock().unwrap();
        for name in ["known", "empty", "future", "broken", "denied"] {
            state.buckets.insert(name.into(), DATE.into());
        }
        state.markers.insert(
            "known".into(),
            (200, json!({"format":"mallard-storage","layout_version":1})),
        );
        state.markers.insert(
            "future".into(),
            (200, json!({"format":"mallard-storage","layout_version":2})),
        );
        state.markers.insert(
            "broken".into(),
            (200, json!({"format":"other","layout_version":1})),
        );
        state.markers.insert("denied".into(), (403, json!({})));
    }
    let scan = mock.discover().unwrap();
    assert!(scan.validate(ACCOUNT));
    assert!(scan.partial);
    assert!(!scan.cut, "an unreadable marker is not a cut listing");
    assert_eq!(scan.jurisdictions, ["default", "eu", "us", "fedramp"]);
    for (name, expected, detail) in [
        ("known", MarkerState::Storage, Some("layout 1")),
        ("empty", MarkerState::Missing, None),
        ("future", MarkerState::Unsupported, Some("layout 2")),
        ("broken", MarkerState::Invalid, None),
        ("denied", MarkerState::Unreadable, Some("access denied")),
    ] {
        let bucket = scan.buckets.iter().find(|b| b.name == name).unwrap();
        assert_eq!(bucket.marker, expected);
        assert_eq!(bucket.marker_detail.as_deref(), detail);
        assert_eq!(bucket.jurisdiction, "default");
    }
    assert!(mock.verify(R2Jurisdiction::Default, "known", DATE));
    assert!(!mock.verify(R2Jurisdiction::Eu, "known", DATE));
    assert!(!mock.verify(R2Jurisdiction::Default, "empty", DATE));
    mock.state
        .lock()
        .unwrap()
        .buckets
        .insert("known".into(), "2026-09-11T00:00:00Z".into());
    assert!(!mock.verify(R2Jurisdiction::Default, "known", DATE));
    mock.state
        .lock()
        .unwrap()
        .buckets
        .insert("known".into(), DATE.into());
    mock.state.lock().unwrap().markers.remove("known");
    assert!(!mock.verify(R2Jurisdiction::Default, "known", DATE));
    assert!(
        mock.state.lock().unwrap().attempted.is_empty(),
        "discovery and validation must not create a bucket"
    );
}

#[test]
fn discovery_lists_every_jurisdiction_and_leaves_out_one_the_account_cannot_use() {
    let mock = Mock::new();
    {
        let mut state = mock.state.lock().unwrap();
        state.buckets.insert("home".into(), DATE.into());
        state.elsewhere.insert("eu".into(), vec!["team-eu".into()]);
        state.unavailable.push("fedramp".into());
        state.markers.insert(
            "team-eu".into(),
            (200, json!({"format":"mallard-storage","layout_version":1})),
        );
    }
    let scan = mock.discover().unwrap();
    assert!(scan.validate(ACCOUNT));
    assert!(
        !scan.partial,
        "an unavailable jurisdiction is not a cut scan"
    );
    assert_eq!(scan.jurisdictions, ["default", "eu", "us"]);
    let team = scan.buckets.iter().find(|b| b.name == "team-eu").unwrap();
    assert_eq!(team.jurisdiction, "eu");
    assert_eq!(team.marker, MarkerState::Storage);
    assert_eq!(
        scan.buckets
            .iter()
            .find(|b| b.name == "home")
            .unwrap()
            .marker,
        MarkerState::Missing
    );
    assert_eq!(scan.buckets.len(), 2);
}

#[test]
fn discovery_bounds_repeated_cursors_and_reports_permission_errors() {
    let mock = Mock::new();
    mock.state.lock().unwrap().repeat_cursor = true;
    let scan = mock.discover().unwrap();
    assert!(scan.partial);
    assert!(scan.cut);
    // Two pages in the default jurisdiction, then one empty list each elsewhere.
    assert_eq!(mock.state.lock().unwrap().requests, 5);
    mock.state.lock().unwrap().deny_list = true;
    assert_eq!(mock.discover().unwrap_err(), SetupError::Permission);
}
