//! Local AWS S3 feasibility POC. Uses the real Mallard transport and fake credentials.
//! Run: cargo run --locked --offline -p mallard-infrastructure --example aws_s3_poc
//! A reproduced defect is reported as REPRODUCED, not as a compatibility pass.

use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::{Client, Config};
use aws_smithy_http_client::{tls, Builder as HttpClientBuilder};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use mallard_domain::{sha256_hex, BundleId};
use mallard_infrastructure::storage_core::publication::{
    list_stream, record_name_of, write_record, RecordStream, WriteOutcome,
};
use mallard_infrastructure::storage_core::{
    ensure_storage_marker, ClaimOutcome, ExpectedObject, ImmutablePutOutcome, ObjectKey,
    ObjectPrefix, ObjectStore, R2CapabilityProbeStage, S3Connection, S3ObjectStore, StageOutcome,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

const BUCKET: &str = "mallard-aws-s3-poc";
const REGION: &str = "us-west-2";
const ACCESS_KEY: &str = "poc-access-key";
const SECRET_KEY: &str = "poc-secret-key";
const SESSION_TOKEN: &str = "poc-session-token";
const ETAG: &str = "\"opaque-fixture-etag\"";
const BUNDLE: &str = "0123456789abcdef0123456789abcdef";

#[derive(Clone, Copy, Default)]
enum RecordReply {
    #[default]
    Store,
    Conflict409,
    Fail503,
    StoreThen503,
}

#[derive(Default)]
struct State {
    objects: BTreeMap<String, Vec<u8>>,
    record_reply: RecordReply,
    record_puts: usize,
    claim_puts: usize,
    signed_requests: usize,
    session_requests: usize,
}

struct Fixture {
    endpoint: String,
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start(record_reply: RecordReply, session: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State {
            record_reply,
            ..State::default()
        }));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let shared = shared.clone();
                tokio::spawn(async move {
                    let service =
                        service_fn(move |request| respond(request, shared.clone(), session));
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            endpoint,
            state,
            task,
        }
    }

    fn store(&self, session: bool) -> S3ObjectStore {
        if !session {
            return S3ObjectStore::from_connection(
                &S3Connection {
                    endpoint: &self.endpoint,
                    region: REGION,
                    bucket: BUCKET,
                    access_key_id: ACCESS_KEY,
                    secret_access_key: SECRET_KEY,
                },
                Handle::current(),
            )
            .unwrap();
        }
        // The existing from_client seam accepts session credentials without
        // changing ObjectStore. This does not test STS issuance or refresh.
        let config = Config::builder()
            .credentials_provider(Credentials::new(
                ACCESS_KEY,
                SECRET_KEY,
                Some(SESSION_TOKEN.into()),
                None,
                "local-poc",
            ))
            .region(Region::new(REGION))
            .endpoint_url(&self.endpoint)
            .force_path_style(true)
            .behavior_version_latest()
            .http_client(
                HttpClientBuilder::new()
                    .tls_provider(tls::Provider::Rustls(
                        tls::rustls_provider::CryptoMode::AwsLc,
                    ))
                    .build_https(),
            )
            .build();
        S3ObjectStore::from_client(Client::from_conf(config), BUCKET, Handle::current()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn response(status: u16, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("etag", ETAG)
        .header("content-type", "application/xml")
        .body(Full::new(body.into()))
        .unwrap()
}

fn error(status: u16, code: &str) -> Response<Full<Bytes>> {
    response(
        status,
        format!("<Error><Code>{code}</Code><Message>Local POC response</Message></Error>"),
    )
}

async fn respond(
    request: Request<Incoming>,
    state: Arc<Mutex<State>>,
    session: bool,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    let authorization = header("authorization").unwrap_or_default();
    // Check the signing scope and token header, not the cryptographic signature.
    if !authorization.starts_with("AWS4-HMAC-SHA256 ")
        || !authorization.contains(&format!("Credential={ACCESS_KEY}/"))
        || !authorization.contains(&format!("/{REGION}/s3/aws4_request"))
        || header("x-amz-security-token") != session.then_some(SESSION_TOKEN)
    {
        return Ok(error(403, "SignatureDoesNotMatch"));
    }
    let bucket_path = format!("/{BUCKET}");
    let Some(suffix) = parts.uri.path().strip_prefix(&bucket_path) else {
        return Ok(error(404, "NoSuchBucket"));
    };
    let key = suffix.trim_start_matches('/');
    let query: BTreeMap<String, String> =
        url::form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let mut state = state.lock().unwrap();
    state.signed_requests += 1;
    state.session_requests += usize::from(session);

    if parts.method == "GET" && query.get("list-type").map(String::as_str) == Some("2") {
        let prefix = query.get("prefix").map(String::as_str).unwrap_or_default();
        let after = query
            .get("start-after")
            .map(String::as_str)
            .unwrap_or_default();
        let offset = query
            .get("continuation-token")
            .and_then(|value| value.strip_prefix("page-"))
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(0);
        let limit = query
            .get("max-keys")
            .unwrap()
            .parse::<usize>()
            .unwrap()
            .min(2);
        let all = state
            .objects
            .iter()
            .filter(|(key, _)| key.starts_with(prefix) && key.as_str() > after)
            .collect::<Vec<_>>();
        let page = all.iter().skip(offset).take(limit).collect::<Vec<_>>();
        let end = offset + page.len();
        let truncated = end < all.len();
        let contents = page.iter().map(|(key, value)| format!(
            "<Contents><Key>{key}</Key><ETag>&quot;opaque-fixture-etag&quot;</ETag><Size>{}</Size></Contents>",
            value.len(),
        )).collect::<String>();
        let next = if truncated {
            format!("<NextContinuationToken>page-{end}</NextContinuationToken>")
        } else {
            String::new()
        };
        return Ok(response(200, format!(
            "<ListBucketResult><IsTruncated>{truncated}</IsTruncated>{contents}{next}</ListBucketResult>",
        )));
    }
    match parts.method.as_str() {
        "GET" => Ok(match state.objects.get(key) {
            Some(body) => response(200, body.clone()),
            None => error(404, "NoSuchKey"),
        }),
        "PUT" => {
            let checksum = aws_smithy_types::base64::encode(Sha256::digest(&bytes));
            if header("if-none-match") != Some("*") {
                return Ok(error(403, "AccessDenied"));
            }
            if header("x-amz-checksum-sha256") != Some(checksum.as_str()) {
                return Ok(error(400, "BadDigest"));
            }
            let record = key.contains("/_commits/");
            state.record_puts += usize::from(record);
            state.claim_puts += usize::from(key.contains("/_claims/"));
            if record {
                match state.record_reply {
                    RecordReply::Conflict409 => {
                        return Ok(error(409, "ConditionalRequestConflict"))
                    }
                    RecordReply::Fail503 => return Ok(error(503, "ServiceUnavailable")),
                    _ => {}
                }
            }
            if state.objects.contains_key(key) {
                return Ok(error(412, "PreconditionFailed"));
            }
            state.objects.insert(key.into(), bytes.to_vec());
            if record && matches!(state.record_reply, RecordReply::StoreThen503) {
                return Ok(error(503, "ServiceUnavailable"));
            }
            Ok(response(200, Bytes::new()))
        }
        _ => Ok(error(405, "MethodNotAllowed")),
    }
}

async fn baseline(session: bool) {
    let fixture = Fixture::start(RecordReply::Store, session).await;
    let store = fixture.store(session);
    tokio::task::spawn_blocking(move || {
        ensure_storage_marker(&store).unwrap();
        let prefix =
            ObjectPrefix::parse(format!(".mallard/v1/repositories/{BUNDLE}/_uploads/poc")).unwrap();
        let key = ObjectKey::parse(format!("{}/payload", prefix.as_str())).unwrap();
        let payload = b"synthetic Mallard payload";
        assert_eq!(
            store.put_immutable(&key, payload).unwrap(),
            ImmutablePutOutcome::Written
        );
        assert_eq!(
            store.put_immutable(&key, payload).unwrap(),
            ImmutablePutOutcome::AlreadyPresent
        );
        assert!(store.put_immutable(&key, b"different bytes").is_err());
        let expected = ExpectedObject {
            size: payload.len() as u64,
            sha256: sha256_hex(payload),
        };
        let StageOutcome::Verified(staged) = store.stage_verified(&key, &expected).unwrap() else {
            panic!("payload did not verify");
        };
        assert_eq!(staged.read_bounded(100, "POC payload").unwrap(), payload);
        for index in 0..4 {
            let key = ObjectKey::parse(format!("{}/item-{index}", prefix.as_str())).unwrap();
            store.put_immutable(&key, payload).unwrap();
        }
        let first = store.list(&prefix, None, 3).unwrap();
        assert_eq!(first.entries.len(), 3);
        let second = store
            .list(&prefix, first.next_cursor.as_deref(), 3)
            .unwrap();
        assert_eq!(second.entries.len(), 2);
        assert!(second.next_cursor.is_none());
        let stream = RecordStream::project(&BundleId::parse(BUNDLE).unwrap()).unwrap();
        let claim = stream.claim_key(None).unwrap();
        assert_eq!(
            store.put_claim(&claim, b"first").unwrap(),
            ClaimOutcome::Mine
        );
        assert_eq!(
            store.put_claim(&claim, b"second").unwrap(),
            ClaimOutcome::Taken(b"first".to_vec())
        );
    })
    .await
    .unwrap();
    let state = fixture.state.lock().unwrap();
    assert!(state.signed_requests > 0);
    assert_eq!(state.session_requests > 0, session);
    println!("PASS: {} credentials; regional signing scope, checksums, immutable objects, verified reads, pagination, claims", if session { "session-token" } else { "static" });
}

async fn publication_case(reply: RecordReply) {
    let fixture = Fixture::start(reply, false).await;
    let store = fixture.store(false);
    let (reported_stored, listed) = tokio::task::spawn_blocking(move || {
        let stream = RecordStream::project(&BundleId::parse(BUNDLE).unwrap()).unwrap();
        let bytes =
            br#"{"revision":1,"commit_id":"commit-010101010101010101010101","created_at_ms":1}"#;
        let name = record_name_of(bytes, None).unwrap();
        let result = write_record(&store, &stream, &name, bytes);
        let listed = list_stream(&store, &stream).unwrap().holds(name.token());
        (matches!(result, Ok(WriteOutcome::Stored(_))), listed)
    })
    .await
    .unwrap();
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.claim_puts, 1, "a claim must not be sent twice");
    assert_eq!(state.record_puts, 1, "a record must not be sent twice");
    let present = state.objects.keys().any(|key| key.contains("/_commits/"));
    assert_eq!(present, listed);
    match reply {
        RecordReply::Store => {
            assert!(reported_stored && listed);
            println!("PASS: publication writes one claim and one record; listing selects the stored record");
        }
        RecordReply::Fail503 => {
            assert!(!reported_stored && !listed);
            println!("PASS: rejected record fails publication without a resend");
        }
        RecordReply::StoreThen503 => {
            assert!(reported_stored && listed);
            println!("PASS: record stored before an error response is settled by listing without a resend");
        }
        RecordReply::Conflict409 => {
            assert!(!listed);
            assert!(
                reported_stored,
                "409 behavior changed; review this defect reproducer"
            );
            println!("REPRODUCED: HTTP 409 reports Stored although the record is absent from storage and listing");
        }
    }
}

#[tokio::main]
async fn main() {
    baseline(false).await;
    baseline(true).await;
    publication_case(RecordReply::Store).await;
    publication_case(RecordReply::Fail503).await;
    publication_case(RecordReply::StoreThen503).await;
    publication_case(RecordReply::Conflict409).await;
    let fixture = Fixture::start(RecordReply::Store, false).await;
    let store = fixture.store(false);
    let failure = tokio::task::spawn_blocking(move || store.probe_r2_capabilities())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(failure.stage(), R2CapabilityProbeStage::Create);
    assert!(fixture.state.lock().unwrap().objects.is_empty());
    println!("REPRODUCED: setup probe cannot create its object when the bucket requires conditional PUTs");
    println!("Local fixture only. AWS credentials, AWS authentication, and a live bucket were not tested.");
}
