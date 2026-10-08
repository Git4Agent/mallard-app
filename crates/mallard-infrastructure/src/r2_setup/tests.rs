use super::discovery::{MarkerDetail, MarkerDetailCode, MarkerState};
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
    failure: Option<(Method, String, u16, Value)>,
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
    if let Some((method, suffix, status, value)) = &state.failure {
        if parts.method == *method && parts.uri.path().ends_with(suffix) {
            return Ok(reply(*status, value.clone()));
        }
    }
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
    fn prepare(&self, name: &str) -> Result<R2Destination, SetupError> {
        self.prepare_in(R2Jurisdiction::Default, name)
    }
    fn prepare_in(
        &self,
        jurisdiction: R2Jurisdiction,
        name: &str,
    ) -> Result<R2Destination, SetupError> {
        self.runtime
            .block_on(prepare_bucket(&self.client, ACCOUNT, jurisdiction, name))
    }
    fn attempted(&self) -> Vec<String> {
        self.state.lock().unwrap().attempted.clone()
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

#[test]
fn creation_makes_one_bucket_and_a_repeat_finds_the_name_taken() {
    let mock = Mock::new();
    let created = mock.prepare("mallard-otter").unwrap();
    assert_eq!(created.bucket(), "mallard-otter");
    assert_eq!(created.jurisdiction(), R2Jurisdiction::Default);
    // Nothing remembers the attempt: asking again finds the bucket in the
    // account and creates nothing.
    assert_eq!(
        mock.prepare("mallard-otter").unwrap_err(),
        SetupError::Taken
    );
    assert_eq!(mock.attempted(), ["mallard-otter"]);
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
fn an_invalid_name_is_refused_before_any_request() {
    let mock = Mock::new();
    assert_eq!(mock.prepare("Bad Name").unwrap_err(), SetupError::Identity);
    assert_eq!(mock.state.lock().unwrap().requests, 0);
}

#[test]
fn a_taken_name_is_reported_without_a_creation_request() {
    let mock = Mock::new();
    mock.state
        .lock()
        .unwrap()
        .buckets
        .insert("team-backups".into(), DATE.into());
    assert_eq!(mock.prepare("team-backups").unwrap_err(), SetupError::Taken);
    assert!(mock.attempted().is_empty());
    let created = mock.prepare("team-backups-2").unwrap();
    assert_eq!(created.bucket(), "team-backups-2");
    assert_eq!(mock.attempted(), ["team-backups-2"]);
}

#[test]
fn a_creation_race_reports_the_name_taken_without_adopting_the_conflict() {
    let mock = Mock::new();
    mock.state.lock().unwrap().conflict_on_create = 1;
    assert_eq!(
        mock.prepare("mallard-otter").unwrap_err(),
        SetupError::Taken
    );
    assert_eq!(mock.attempted(), ["mallard-otter"]);
    let created = mock.prepare("mallard-brave").unwrap();
    assert_eq!(created.bucket(), "mallard-brave");
    assert_eq!(mock.attempted().len(), 2);
}

#[test]
fn an_unknown_creation_is_reported_and_the_retry_finds_the_bucket_taken() {
    let mock = Mock::new();
    mock.state.lock().unwrap().fail_after_create = true;
    let error = mock.prepare("mallard-otter").unwrap_err();
    assert_eq!(error, SetupError::Uncertain);
    assert_eq!(error.code(), "r2_setup_outcome_unknown");
    // The bucket was made. The next attempt finds it and creates nothing.
    mock.state.lock().unwrap().fail_after_create = false;
    assert_eq!(
        mock.prepare("mallard-otter").unwrap_err(),
        SetupError::Taken
    );
    assert_eq!(mock.attempted(), ["mallard-otter"]);
}

#[test]
fn r2_activation_errors_are_reported_once_and_a_retry_creates_after_activation() {
    for method in [Method::GET, Method::POST] {
        let mock = Mock::new();
        mock.state.lock().unwrap().failure = Some((
            method.clone(),
            if method == Method::GET {
                "/mallard-otter"
            } else {
                "/buckets"
            }
            .into(),
            403,
            json!({"success":false,"errors":[{"code":10042,"message":"secret-canary"}]}),
        ));
        let error = mock.prepare("mallard-otter").unwrap_err();
        assert_eq!(error, SetupError::R2NotEnabled);
        assert_eq!(error.code(), "r2_setup_not_enabled");
        assert!(!error.message().contains("secret-canary"));
        assert!(mock.state.lock().unwrap().buckets.is_empty());
        assert_eq!(
            mock.state.lock().unwrap().requests,
            if method == Method::GET { 1 } else { 2 }
        );

        mock.state.lock().unwrap().failure = None;
        mock.prepare("mallard-otter").unwrap();
        assert_eq!(mock.attempted(), ["mallard-otter"]);
    }
}

#[test]
fn discovery_reports_r2_activation_errors() {
    let mock = Mock::new();
    mock.state.lock().unwrap().failure = Some((
        Method::GET,
        "/buckets".into(),
        403,
        json!({"success":false,"errors":[{"code":10042}]}),
    ));
    assert_eq!(mock.discover().unwrap_err(), SetupError::R2NotEnabled);
    assert_eq!(mock.state.lock().unwrap().requests, 1);
}

#[test]
fn activation_classification_requires_a_provider_error_code() {
    let activation = br#"{"success":false,"errors":[{"code":10042}]}"#;
    assert_eq!(
        response_error(400, activation),
        Some(SetupError::R2NotEnabled)
    );
    assert_eq!(response_error(200, activation), None);
    assert_eq!(
        response_error(500, activation),
        Some(SetupError::Unavailable)
    );
    for bytes in [
        b"".as_slice(),
        b"not json",
        br#"{"success":false,"errors":[{"code":10001}]}"#,
        br#"{"success":false,"errors":[{"code":"10042"}]}"#,
        br#"{"success":true,"errors":[{"code":10042}]}"#,
        br#"{"success":false,"errors":[{"message":"Please enable R2 through the Cloudflare Dashboard."}]}"#,
    ] {
        assert_eq!(response_error(401, bytes), Some(SetupError::Authentication));
        assert_eq!(response_error(403, bytes), Some(SetupError::Permission));
    }
}

#[test]
fn cloudflare_errors_distinguish_safe_reads_from_unknown_creation_results() {
    for (status, read_error, create_error) in [
        (401, SetupError::Authentication, SetupError::Authentication),
        (403, SetupError::Permission, SetupError::Permission),
        (429, SetupError::RateLimited, SetupError::RateLimited),
        (408, SetupError::Unavailable, SetupError::Uncertain),
        (500, SetupError::Unavailable, SetupError::Uncertain),
        (503, SetupError::Unavailable, SetupError::Uncertain),
    ] {
        let mock = Mock::new();
        for (method, suffix, expected) in [
            (Method::GET, "/mallard-otter", read_error),
            (Method::POST, "/buckets", create_error),
        ] {
            mock.state.lock().unwrap().failure = Some((
                method,
                suffix.into(),
                status,
                json!({"success":false,"errors":[{"code":10001,"message":"secret-canary"}]}),
            ));
            let error = mock.prepare("mallard-otter").unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.message().contains("secret-canary"));
        }
        assert!(mock.state.lock().unwrap().buckets.is_empty());
        mock.state.lock().unwrap().failure = Some((
            Method::GET,
            "/buckets".into(),
            status,
            json!({"success":false}),
        ));
        assert_eq!(mock.discover().unwrap_err(), read_error);
    }
    // Even a non-JSON rate-limit response is a rate limit.
    assert_eq!(
        response_error(429, b"not json"),
        Some(SetupError::RateLimited)
    );
}

#[test]
fn rejections_are_reported_once_and_never_as_taken() {
    for status in [400, 401, 403, 409, 429, 500] {
        let mock = Mock::new();
        mock.state.lock().unwrap().reject = Some(status);
        let error = mock.prepare("mallard-otter").unwrap_err();
        assert!(!format!("{error:?}").contains("secret-canary"));
        assert_ne!(error, SetupError::Taken);
        if status == 401 {
            assert_eq!(error, SetupError::Authentication);
        } else if status == 403 {
            assert_eq!(error, SetupError::Permission);
        }
        assert_eq!(mock.attempted().len(), 1);
    }
}

#[test]
fn a_bucket_is_created_in_the_requested_jurisdiction() {
    let mock = Mock::new();
    let created = mock
        .prepare_in(R2Jurisdiction::Eu, "mallard-otter")
        .unwrap();
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
            (
                200,
                json!({"format":"mallard-append-storage","layout_version":1}),
            ),
        );
        state.markers.insert(
            "future".into(),
            (
                200,
                json!({"format":"mallard-append-storage","layout_version":2}),
            ),
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
        ("known", MarkerState::Storage, Some(MarkerDetail::layout(1))),
        ("empty", MarkerState::Missing, None),
        (
            "future",
            MarkerState::Unsupported,
            Some(MarkerDetail::layout(2)),
        ),
        ("broken", MarkerState::Invalid, None),
        (
            "denied",
            MarkerState::Unreadable,
            Some(MarkerDetail::reason(
                MarkerDetailCode::AccessDenied,
                "access denied",
            )),
        ),
    ] {
        let bucket = scan.buckets.iter().find(|b| b.name == name).unwrap();
        assert_eq!(bucket.marker, expected);
        assert_eq!(bucket.marker_detail, detail);
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
            (
                200,
                json!({"format":"mallard-append-storage","layout_version":1}),
            ),
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
