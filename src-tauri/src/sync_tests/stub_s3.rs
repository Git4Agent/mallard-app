//! Stub S3 server for the sync integration tests.
//!
//! Objects are plain files under a local root directory (`<root>/<bucket>/
//! <key>`), so tests can assert the published cloud layout by reading the
//! filesystem directly. Implements exactly the surface `lib.rs` uses:
//! GET/PUT/DELETE object, conditional PUT via `If-Match` / `If-None-Match: *`,
//! and ListObjectsV2 with delimiter grouping. ETags are the sha256 of the
//! object bytes. Requests are serialized under one lock; auth is ignored.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug)]
pub struct RequestRecord {
    pub method: String,
    pub key: String,
    pub conditional: bool,
}

#[derive(Default)]
struct StubState {
    log: Mutex<Vec<RequestRecord>>,
}

pub struct StubS3 {
    pub endpoint: String,
    state: Arc<StubState>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for StubS3 {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl StubS3 {
    /// Serve `root` (bucket directories live directly under it) on an
    /// ephemeral localhost port.
    pub async fn start(root: PathBuf) -> StubS3 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub s3");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(StubState::default());
        let server = {
            let root = root.clone();
            let state = state.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let root = root.clone();
                    let state = state.clone();
                    tokio::spawn(async move {
                        let service =
                            service_fn(move |req| handle(req, root.clone(), state.clone()));
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            })
        };
        StubS3 {
            endpoint: format!("http://127.0.0.1:{}", port),
            state,
            server,
        }
    }

    pub fn requests(&self) -> Vec<RequestRecord> {
        self.state.log.lock().unwrap().clone()
    }
}

fn pct_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| pct_decode(value))
    })
}

fn respond(
    status: StatusCode,
    body: impl Into<Bytes>,
    etag: Option<&str>,
) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/xml");
    if let Some(etag) = etag {
        builder = builder.header("etag", format!("\"{}\"", etag));
    }
    builder.body(Full::new(body.into())).unwrap()
}

fn xml_error(status: StatusCode, code: &str) -> Response<Full<Bytes>> {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{}</Code><Message>{}</Message></Error>",
        code, code
    );
    respond(status, body, None)
}

fn collect_keys(bucket_dir: &Path, dir: &Path, keys: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_keys(bucket_dir, &path, keys);
        } else if let Ok(rel) = path.strip_prefix(bucket_dir) {
            keys.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Keys grouped by `delimiter` after `prefix` — also surfaces empty
/// directories as common prefixes, matching how a prefix listing of real
/// uploaded objects behaves for this app's layout.
fn list_response(bucket: &str, bucket_dir: &Path, prefix: &str, delimiter: Option<&str>) -> String {
    let mut keys = Vec::new();
    collect_keys(bucket_dir, bucket_dir, &mut keys);
    if let Ok(entries) = fs::read_dir(bucket_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                keys.push(format!("{}/", entry.file_name().to_string_lossy()));
            }
        }
    }
    keys.sort();

    let mut commons: BTreeSet<String> = BTreeSet::new();
    let mut contents: Vec<String> = Vec::new();
    for key in keys {
        let Some(rest) = key.strip_prefix(prefix) else {
            continue;
        };
        match delimiter {
            Some(delim) => match rest.find(delim) {
                Some(index) => {
                    commons.insert(format!("{}{}{}", prefix, &rest[..index], delim));
                }
                None => contents.push(key.clone()),
            },
            None => contents.push(key.clone()),
        }
    }

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
    );
    xml.push_str(&format!(
        "<Name>{}</Name><Prefix>{}</Prefix><IsTruncated>false</IsTruncated><KeyCount>{}</KeyCount>",
        bucket,
        prefix,
        commons.len() + contents.len()
    ));
    for common in &commons {
        xml.push_str(&format!(
            "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
            common
        ));
    }
    for key in &contents {
        let data = fs::read(bucket_dir.join(key)).unwrap_or_default();
        xml.push_str(&format!(
            "<Contents><Key>{}</Key><Size>{}</Size><ETag>&quot;{}&quot;</ETag>\
             <LastModified>2026-01-01T00:00:00.000Z</LastModified>\
             <StorageClass>STANDARD</StorageClass></Contents>",
            key,
            data.len(),
            sha256_bytes(&data)
        ));
    }
    xml.push_str("</ListBucketResult>");
    xml
}

async fn handle(
    req: Request<Incoming>,
    root: PathBuf,
    state: Arc<StubState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().trim_start_matches('/').to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let if_match = req
        .headers()
        .get("if-match")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let if_none_match = req
        .headers()
        .get("if-none-match")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let checksum_sha256 = req
        .headers()
        .get("x-amz-checksum-sha256")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Ok(xml_error(StatusCode::BAD_REQUEST, "IncompleteBody")),
    };
    // Server-side write verification, exactly like S3/R2: every PUT must
    // carry a sha256 checksum matching its body. The accepted PUT is the
    // client's only write receipt — publish never reads objects back — so
    // the stub enforces what the client relies on.
    if method == Method::PUT {
        let valid = checksum_sha256.as_deref()
            == Some(aws_smithy_types::base64::encode(Sha256::digest(&body)).as_str());
        if !valid {
            return Ok(xml_error(StatusCode::BAD_REQUEST, "BadDigest"));
        }
    }

    let (bucket, key) = match path.split_once('/') {
        Some((bucket, key)) => (bucket.to_string(), pct_decode(key)),
        None => (path.clone(), String::new()),
    };
    let bucket_dir = root.join(&bucket);
    let object_path = bucket_dir.join(&key);
    let conditional = if_match.is_some() || if_none_match.is_some();

    let response = {
        let mut log = state.log.lock().unwrap();

        let response = match (&method, key.is_empty()) {
            (&Method::GET, true) if query.contains("list-type=2") => {
                let prefix = query_param(&query, "prefix").unwrap_or_default();
                let delimiter = query_param(&query, "delimiter");
                respond(
                    StatusCode::OK,
                    list_response(&bucket, &bucket_dir, &prefix, delimiter.as_deref()),
                    None,
                )
            }
            (&Method::GET, false) => match fs::read(&object_path) {
                Ok(data) => {
                    let etag = sha256_bytes(&data);
                    respond(StatusCode::OK, data, Some(&etag))
                }
                Err(_) => xml_error(StatusCode::NOT_FOUND, "NoSuchKey"),
            },
            (&Method::PUT, false) => {
                let mut fail = false;
                let current = fs::read(&object_path).ok();
                if conditional {
                    if let Some(expected) = &if_match {
                        let expected = expected.trim().trim_matches('"');
                        fail = current
                            .as_ref()
                            .is_none_or(|data| sha256_bytes(data) != expected);
                    }
                    if if_none_match.as_deref().map(str::trim) == Some("*") && current.is_some() {
                        fail = true;
                    }
                }
                if fail {
                    xml_error(StatusCode::PRECONDITION_FAILED, "PreconditionFailed")
                } else {
                    if let Some(parent) = object_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    fs::write(&object_path, &body).expect("stub s3 write");
                    let etag = sha256_bytes(&body);
                    respond(StatusCode::OK, Bytes::new(), Some(&etag))
                }
            }
            (&Method::DELETE, false) => {
                let _ = fs::remove_file(&object_path);
                respond(StatusCode::NO_CONTENT, Bytes::new(), None)
            }
            _ => xml_error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
        };

        log.push(RequestRecord {
            method: method.to_string(),
            key: key.clone(),
            conditional,
        });
        response
    };

    Ok(response)
}
