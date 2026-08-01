//! S3/R2 transport for the current bundle object-store contract.
//!
//! [`BundleObjectStore`] is deliberately synchronous because the bundle
//! engine also supports a filesystem implementation.  Consequently, command
//! handlers must run every engine operation which can reach this store inside
//! `tokio::task::spawn_blocking`.  The store uses the supplied runtime handle
//! to drive AWS SDK futures; a nested-runtime panic is caught and converted to
//! an error with that instruction, but catching it is not a substitute for
//! keeping blocking work off an async runtime worker.

use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::{Client as S3Client, Config as S3Config};
use aws_smithy_http_client::{tls, Builder as HttpClientBuilder};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::time::timeout;

use super::bundle_engine::{
    BundleObjectStore, CasExpectation, CasOutcome, ImmutablePutOutcome, ObjectKey, ObjectPrefix,
    StoreListPage, StoredObject,
};
use super::domain::{StorageConfig, StorageKind, MAX_FILE_BYTES};

const MAX_LIST_PAGE: usize = 10_000;
const S3_MAX_KEYS: usize = 1_000;
const MAX_LIST_REQUESTS: usize = MAX_LIST_PAGE + 2;
const R2_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const BULK_GET_CONCURRENCY: usize = 6;
const BULK_PUT_CONCURRENCY: usize = 6;

#[derive(Clone)]
pub struct S3BundleObjectStore {
    client: S3Client,
    bucket: String,
    runtime: Handle,
    request_timeout: Duration,
}

impl S3BundleObjectStore {
    /// Build a store using the application's existing S3/R2 client
    /// configuration path.  An explicitly unsupported conditional-write
    /// capability is rejected: project bundles publish mutable heads only by CAS.
    pub fn from_config(config: &StorageConfig, runtime: Handle) -> Result<Self, String> {
        config.validate()?;
        if config.kind != StorageKind::S3 {
            return Err(format!(
                "storage '{}' is not an S3-compatible storage",
                config.id
            ));
        }
        if config.supports_conditional_writes == Some(false) {
            return Err(format!(
                "storage '{}' does not support the conditional writes required by project bundles",
                config.id
            ));
        }

        let client = make_s3_client(config)?;
        Self::from_client(client, config.bucket.clone(), runtime)
    }

    /// Convenience constructor for callers already running in a Tokio
    /// context.  Capture the handle before moving the engine operation into
    /// `spawn_blocking`.
    pub fn from_current_runtime(config: &StorageConfig) -> Result<Self, String> {
        let runtime = Handle::try_current()
            .map_err(|_| "creating an S3 bundle store requires a Tokio runtime".to_string())?;
        Self::from_config(config, runtime)
    }

    /// Construct from an already configured client.  This is useful for
    /// focused transport tests and for callers which own AWS client setup.
    pub fn from_client(
        client: S3Client,
        bucket: impl Into<String>,
        runtime: Handle,
    ) -> Result<Self, String> {
        let bucket = bucket.into();
        if bucket.is_empty()
            || bucket.len() > 1_024
            || bucket.trim() != bucket
            || bucket.chars().any(char::is_control)
        {
            return Err("S3 bundle store bucket is invalid".to_string());
        }
        Ok(Self {
            client,
            bucket,
            runtime,
            request_timeout: R2_REQUEST_TIMEOUT,
        })
    }

    fn run<F, T>(&self, operation: &str, future: F) -> Result<T, String>
    where
        F: Future<Output = Result<T, String>>,
    {
        match catch_unwind(AssertUnwindSafe(|| self.runtime.block_on(future))) {
            Ok(result) => result,
            Err(_) => Err(format!(
                "S3 bundle-store {} cannot block an async runtime worker; run the enclosing bundle-engine operation with tokio::task::spawn_blocking",
                operation
            )),
        }
    }

    async fn get_async(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        let request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send();
        let response = match timeout(self.request_timeout, request).await {
            Err(_) => {
                return Err(format!(
                    "S3 get '{}' timed out after {} ms",
                    key,
                    self.request_timeout.as_millis()
                ));
            }
            Ok(Ok(response)) => response,
            Ok(Err(error)) if sdk_status(&error) == Some(404) => return Ok(None),
            Ok(Err(error)) => return Err(format!("S3 get '{}': {}", key, error)),
        };

        if let Some(length) = response.content_length() {
            if length < 0 || length as u64 > MAX_FILE_BYTES {
                return Err(format!("S3 object '{}' exceeds the read limit", key));
            }
        }
        let etag = response
            .e_tag()
            .ok_or_else(|| format!("S3 get '{}' returned no ETag", key))
            .and_then(|etag| validated_service_etag(etag, key.as_str()))?;
        let body = timeout(self.request_timeout, response.body.collect())
            .await
            .map_err(|_| {
                format!(
                    "S3 body '{}' timed out after {} ms",
                    key,
                    self.request_timeout.as_millis()
                )
            })?
            .map_err(|error| format!("S3 body '{}': {}", key, error))?
            .into_bytes()
            .to_vec();
        if body.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("S3 object '{}' exceeds the read limit", key));
        }
        Ok(Some(StoredObject { bytes: body, etag }))
    }

    async fn conditional_put_async(
        &self,
        key: &ObjectKey,
        body: Bytes,
        expectation: &CasExpectation,
    ) -> Result<PutAttempt, String> {
        // The service verifies the received bytes against this digest before
        // accepting, so an acknowledged PUT is a content-verified receipt and
        // no write path ever reads its object back.
        let checksum = aws_smithy_types::base64::encode(Sha256::digest(&body));
        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .checksum_sha256(checksum)
            .body(ByteStream::from(body));
        request = match expectation {
            CasExpectation::Absent => request.if_none_match("*"),
            CasExpectation::Match(etag) => {
                validate_expected_etag(etag)?;
                request.if_match(format!("\"{}\"", etag))
            }
        };

        match timeout(self.request_timeout, request.send()).await {
            Err(_) => Ok(PutAttempt::Ambiguous(format!(
                "request timed out after {} ms",
                self.request_timeout.as_millis()
            ))),
            Ok(Ok(response)) => {
                let etag = response
                    .e_tag()
                    .map(|etag| validated_service_etag(etag, key.as_str()))
                    .transpose()?;
                Ok(PutAttempt::Written(etag))
            }
            Ok(Err(error)) if matches!(sdk_status(&error), Some(409) | Some(412)) => {
                Ok(PutAttempt::PreconditionFailed)
            }
            Ok(Err(error)) if ambiguous_sdk_error(&error) => {
                Ok(PutAttempt::Ambiguous(error.to_string()))
            }
            Ok(Err(error)) => Err(format!("S3 conditional put '{}': {}", key, error)),
        }
    }

    async fn put_immutable_async(
        &self,
        key: &ObjectKey,
        bytes: Bytes,
    ) -> Result<ImmutablePutOutcome, String> {
        match self
            .conditional_put_async(key, bytes.clone(), &CasExpectation::Absent)
            .await?
        {
            PutAttempt::Written(_) => Ok(ImmutablePutOutcome::Written),
            PutAttempt::PreconditionFailed => match self.get_async(key).await? {
                Some(existing) => immutable_existing_outcome(key, &bytes, existing),
                None => Err(format!(
                    "immutable put '{}' failed its absent precondition but the object is missing",
                    key
                )),
            },
            PutAttempt::Ambiguous(cause) => match self.get_async(key).await {
                Ok(Some(existing)) if existing.bytes == bytes => {
                    Ok(ImmutablePutOutcome::AlreadyPresent)
                }
                Ok(Some(_)) => Err(format!(
                    "immutable put '{}' was ambiguous ({}) and a different object is present",
                    key, cause
                )),
                Ok(None) => Err(format!(
                    "immutable put '{}' was ambiguous ({}) and no object is present",
                    key, cause
                )),
                Err(read_error) => Err(format!(
                    "immutable put '{}' was ambiguous ({}); resolution read failed: {}",
                    key, cause, read_error
                )),
            },
        }
    }

    async fn compare_and_swap_async(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: Bytes,
    ) -> Result<CasOutcome, String> {
        match self
            .conditional_put_async(key, bytes.clone(), expectation)
            .await?
        {
            PutAttempt::Written(Some(etag)) => Ok(CasOutcome::Written { etag }),
            PutAttempt::Written(None) => self.resolve_cas_write(key, &bytes).await,
            PutAttempt::PreconditionFailed => Ok(CasOutcome::Conflict {
                current_etag: self.get_async(key).await?.map(|object| object.etag),
            }),
            PutAttempt::Ambiguous(_) => self.resolve_cas_write(key, &bytes).await,
        }
    }

    async fn resolve_cas_write(
        &self,
        key: &ObjectKey,
        intended_bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        let current = self.get_async(key).await?;
        if let Some(current) = current {
            if current.bytes == intended_bytes {
                return Ok(CasOutcome::Written { etag: current.etag });
            }
            return Ok(CasOutcome::Conflict {
                current_etag: Some(current.etag),
            });
        }
        Ok(CasOutcome::Conflict { current_etag: None })
    }

    async fn list_async(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        let target = limit + 1;
        let mut keys = BTreeSet::new();
        let mut continuation: Option<String> = None;
        let mut start_after = cursor.map(str::to_string);
        let mut request_count = 0_usize;

        loop {
            request_count += 1;
            if request_count > MAX_LIST_REQUESTS {
                return Err(format!(
                    "S3 list '{}' exceeded its pagination safety limit",
                    prefix.as_str()
                ));
            }
            let remaining = target.saturating_sub(keys.len()).max(1);
            let mut request = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix.as_str())
                .max_keys(remaining.min(S3_MAX_KEYS) as i32);
            if let Some(token) = &continuation {
                request = request.continuation_token(token);
            } else if let Some(after) = &start_after {
                request = request.start_after(after);
            }
            let response = timeout(self.request_timeout, request.send())
                .await
                .map_err(|_| {
                    format!(
                        "S3 list '{}' timed out after {} ms",
                        prefix.as_str(),
                        self.request_timeout.as_millis()
                    )
                })?
                .map_err(|error| format!("S3 list '{}': {}", prefix.as_str(), error))?;

            let mut last_raw_key: Option<String> = None;
            for object in response.contents() {
                let raw_key = object.key().ok_or_else(|| {
                    format!(
                        "S3 list '{}' returned an object without a key",
                        prefix.as_str()
                    )
                })?;
                if last_raw_key.as_deref().is_none_or(|last| raw_key > last) {
                    last_raw_key = Some(raw_key.to_string());
                }
                // S3's byte-prefix match can also return lexical neighbors
                // (for example `.mallard/v1/repositories-old` for the
                // repository namespace). Ignore
                // those before applying the stricter ObjectKey validator;
                // malformed keys inside the requested namespace still fail.
                if !raw_key_is_under_prefix(raw_key, prefix) {
                    continue;
                }
                let key = ObjectKey::parse(raw_key.to_string())?;
                if cursor.is_none_or(|cursor| key.as_str() > cursor) {
                    keys.insert(key);
                }
            }

            if keys.len() >= target {
                break;
            }
            if response.is_truncated() != Some(true) {
                break;
            }
            if let Some(next) = response.next_continuation_token() {
                if continuation.as_deref() == Some(next) {
                    return Err(format!(
                        "S3 list '{}' repeated a continuation token",
                        prefix.as_str()
                    ));
                }
                continuation = Some(next.to_string());
                start_after = None;
            } else if let Some(last) = last_raw_key {
                if start_after.as_deref() == Some(last.as_str()) {
                    return Err(format!(
                        "S3 list '{}' made no pagination progress",
                        prefix.as_str()
                    ));
                }
                continuation = None;
                start_after = Some(last);
            } else {
                return Err(format!(
                    "S3 list '{}' was truncated without a continuation token or key",
                    prefix.as_str()
                ));
            }
        }

        let has_more = keys.len() > limit;
        let mut keys = keys.into_iter().take(limit).collect::<Vec<_>>();
        keys.sort();
        let next_cursor = has_more.then(|| {
            keys.last()
                .expect("a nonzero list limit with more results has a last key")
                .as_str()
                .to_string()
        });
        Ok(StoreListPage { keys, next_cursor })
    }
}

impl BundleObjectStore for S3BundleObjectStore {
    fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        self.run("get", self.get_async(key))
    }

    /// Bounded-concurrency bulk download. Order matches `keys`; the first
    /// failure aborts the remaining transfers, like the sequential loop it
    /// replaces.
    fn get_many(&self, keys: &[ObjectKey]) -> Result<Vec<Option<StoredObject>>, String> {
        if keys.len() <= 1 {
            return keys.iter().map(|key| self.get(key)).collect();
        }
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(BULK_GET_CONCURRENCY));
        self.run("bulk get", async {
            let mut join_set = tokio::task::JoinSet::new();
            for (index, key) in keys.iter().cloned().enumerate() {
                let store = self.clone();
                let semaphore = semaphore.clone();
                join_set.spawn(async move {
                    let _permit = semaphore
                        .acquire_owned()
                        .await
                        .map_err(|_| "bulk get concurrency gate closed".to_string())?;
                    Ok::<_, String>((index, store.get_async(&key).await?))
                });
            }
            let mut objects: Vec<Option<Option<StoredObject>>> = Vec::new();
            objects.resize_with(keys.len(), || None);
            while let Some(joined) = join_set.join_next().await {
                let (index, object) =
                    joined.map_err(|error| format!("bulk get worker failed: {}", error))??;
                objects[index] = Some(object);
            }
            Ok(objects
                .into_iter()
                .map(|slot| slot.expect("every bulk-get index resolves"))
                .collect())
        })
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("object '{}' exceeds the write limit", key));
        }
        self.run(
            "immutable put",
            self.put_immutable_async(key, Bytes::copy_from_slice(bytes)),
        )
    }

    /// Bounded-concurrency bulk upload. Outcomes line up with `objects`; the
    /// first failure aborts the remaining transfers, like the sequential loop
    /// it replaces.
    fn put_many(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        for (key, bytes) in &objects {
            if bytes.len() as u64 > MAX_FILE_BYTES {
                return Err(format!("object '{}' exceeds the write limit", key));
            }
        }
        if objects.len() <= 1 {
            return objects
                .into_iter()
                .map(|(key, bytes)| {
                    self.run("immutable put", self.put_immutable_async(&key, bytes))
                })
                .collect();
        }
        let count = objects.len();
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(BULK_PUT_CONCURRENCY));
        self.run("bulk put", async {
            let mut join_set = tokio::task::JoinSet::new();
            for (index, (key, bytes)) in objects.into_iter().enumerate() {
                let store = self.clone();
                let semaphore = semaphore.clone();
                join_set.spawn(async move {
                    let _permit = semaphore
                        .acquire_owned()
                        .await
                        .map_err(|_| "bulk put concurrency gate closed".to_string())?;
                    Ok::<_, String>((index, store.put_immutable_async(&key, bytes).await?))
                });
            }
            let mut outcomes: Vec<Option<ImmutablePutOutcome>> = Vec::new();
            outcomes.resize_with(count, || None);
            while let Some(joined) = join_set.join_next().await {
                let (index, outcome) =
                    joined.map_err(|error| format!("bulk put worker failed: {}", error))??;
                outcomes[index] = Some(outcome);
            }
            Ok(outcomes
                .into_iter()
                .map(|slot| slot.expect("every bulk-put index resolves"))
                .collect())
        })
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("object '{}' exceeds the write limit", key));
        }
        if let CasExpectation::Match(etag) = expectation {
            validate_expected_etag(etag)?;
        }
        self.run(
            "compare-and-swap",
            self.compare_and_swap_async(key, expectation, Bytes::copy_from_slice(bytes)),
        )
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err(format!(
                "list limit must be between 1 and {}",
                MAX_LIST_PAGE
            ));
        }
        if let Some(cursor) = cursor {
            let cursor_key = ObjectKey::parse(cursor.to_string())?;
            if !key_is_under_prefix(&cursor_key, prefix) {
                return Err(format!(
                    "list cursor '{}' is outside prefix '{}'",
                    cursor,
                    prefix.as_str()
                ));
            }
        }
        self.run("list", self.list_async(prefix, cursor, limit))
    }
}

enum PutAttempt {
    Written(Option<String>),
    PreconditionFailed,
    Ambiguous(String),
}

fn immutable_existing_outcome(
    key: &ObjectKey,
    intended_bytes: &[u8],
    existing: StoredObject,
) -> Result<ImmutablePutOutcome, String> {
    if existing.bytes == intended_bytes {
        Ok(ImmutablePutOutcome::AlreadyPresent)
    } else {
        Err(format!(
            "immutable object '{}' already exists with different bytes",
            key
        ))
    }
}

fn key_is_under_prefix(key: &ObjectKey, prefix: &ObjectPrefix) -> bool {
    raw_key_is_under_prefix(key.as_str(), prefix)
}

fn raw_key_is_under_prefix(key: &str, prefix: &ObjectPrefix) -> bool {
    key == prefix.as_str()
        || key
            .strip_prefix(prefix.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn make_s3_client(config: &StorageConfig) -> Result<S3Client, String> {
    if config.bucket.is_empty() {
        return Err("Bucket is not configured".to_string());
    }
    if config.access_key_id.is_empty() {
        return Err("R2 Access Key ID is not configured".to_string());
    }
    if config.secret_access_key.is_empty() {
        return Err("R2 Secret Access Key is not configured".to_string());
    }
    let endpoint = if !config.s3_endpoint.is_empty() {
        config.s3_endpoint.clone()
    } else if !config.account_id.is_empty() {
        format!("https://{}.r2.cloudflarestorage.com", config.account_id)
    } else {
        return Err("Endpoint is not configured".to_string());
    };
    let secret = if config.secret_access_key.starts_with("cfat_") {
        sha256_bytes(config.secret_access_key.as_bytes())
    } else {
        config.secret_access_key.clone()
    };
    let credentials = Credentials::new(
        &config.access_key_id,
        secret,
        None,
        None,
        "mallard-project-sync",
    );
    let region = if config.region.is_empty() {
        "auto".to_string()
    } else {
        config.region.clone()
    };
    let sdk_config = S3Config::builder()
        .credentials_provider(credentials)
        .region(Region::new(region))
        .endpoint_url(endpoint)
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
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        S3Client::from_conf(sdk_config)
    }))
    .map_err(|panic| {
        if let Some(message) = panic.downcast_ref::<&str>() {
            format!("R2 client creation panicked: {message}")
        } else if let Some(message) = panic.downcast_ref::<String>() {
            format!("R2 client creation panicked: {message}")
        } else {
            "R2 client creation panicked".to_string()
        }
    })
}

fn sdk_status<E>(error: &SdkError<E>) -> Option<u16> {
    match error {
        SdkError::ServiceError(context) => Some(context.raw().status().as_u16()),
        SdkError::ResponseError(context) => Some(context.raw().status().as_u16()),
        _ => None,
    }
}

fn normalize_etag(etag: &str) -> String {
    etag.trim_matches('"').to_string()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validated_service_etag(raw: &str, key: &str) -> Result<String, String> {
    let etag = normalize_etag(raw);
    validate_expected_etag(&etag)
        .map_err(|_| format!("S3 object '{}' returned an invalid ETag", key))?;
    Ok(etag)
}

fn validate_expected_etag(etag: &str) -> Result<(), String> {
    if etag.is_empty()
        || etag.len() > 1_024
        || !etag.is_ascii()
        || etag
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'"' | b'\\'))
    {
        return Err("S3 CAS expectation contains an invalid ETag".to_string());
    }
    Ok(())
}

fn ambiguous_sdk_error<E>(error: &SdkError<E>) -> bool {
    matches!(
        error,
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_sync::bundle_engine::BundleEngine;
    use crate::project_sync::domain::StorageId;
    use aws_sdk_s3::config::{Credentials, Region};
    use aws_sdk_s3::Config as S3Config;
    use aws_smithy_http_client::{tls, Builder as HttpClientBuilder};
    use http_body_util::{BodyExt, Full};
    use hyper::body::{Bytes as HyperBytes, Incoming};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use std::collections::BTreeMap;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    const BUCKET: &str = "bundle-tests";

    struct TestS3 {
        endpoint: String,
        objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl TestS3 {
        async fn start() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind test S3");
            let endpoint = format!("http://{}", listener.local_addr().expect("test S3 address"));
            let objects = Arc::new(Mutex::new(BTreeMap::new()));
            let server_objects = objects.clone();
            let server = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let objects = server_objects.clone();
                    tokio::spawn(async move {
                        let service =
                            service_fn(move |request| test_s3_request(request, objects.clone()));
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            });
            Self {
                endpoint,
                objects,
                server,
            }
        }

        fn seed(&self, key: String, bytes: Vec<u8>) {
            self.objects
                .lock()
                .expect("test S3 lock")
                .insert(key, bytes);
        }

        fn store(&self) -> S3BundleObjectStore {
            let credentials = Credentials::new("test", "test", None, None, "bundle-tests");
            let config = S3Config::builder()
                .credentials_provider(credentials)
                .region(Region::new("us-east-1"))
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
            S3BundleObjectStore::from_client(S3Client::from_conf(config), BUCKET, Handle::current())
                .expect("construct test store")
        }
    }

    impl Drop for TestS3 {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn blocking<T: Send + 'static>(operation: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(operation)
            .await
            .expect("blocking store operation")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn engine_initializes_the_mallard_storage_marker_once() {
        let server = TestS3::start().await;
        let store = server.store();
        blocking(move || BundleEngine::open(store, StorageId::parse("storage-r2").unwrap()))
            .await
            .unwrap();

        let marker = server
            .objects
            .lock()
            .expect("test S3 lock")
            .get(".mallard/_storage.json")
            .cloned()
            .expect("Mallard storage marker");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&marker).unwrap(),
            serde_json::json!({
                "format": "mallard-storage",
                "layout_version": 1,
            })
        );

        let store = server.store();
        blocking(move || BundleEngine::open(store, StorageId::parse("storage-r2").unwrap()))
            .await
            .unwrap();
        assert_eq!(
            server
                .objects
                .lock()
                .expect("test S3 lock")
                .get(".mallard/_storage.json")
                .cloned()
                .unwrap(),
            marker
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn immutable_get_and_cas_preserve_service_etags() {
        let server = TestS3::start().await;
        let store = server.store();
        let immutable = ObjectKey::parse(
            ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_uploads/a/file",
        )
        .unwrap();
        let store_for_write = store.clone();
        let immutable_for_write = immutable.clone();
        assert_eq!(
            blocking(move || store_for_write.put_immutable(&immutable_for_write, b"one"))
                .await
                .unwrap(),
            ImmutablePutOutcome::Written
        );
        let store_for_repeat = store.clone();
        let immutable_for_repeat = immutable.clone();
        assert_eq!(
            blocking(move || store_for_repeat.put_immutable(&immutable_for_repeat, b"one"))
                .await
                .unwrap(),
            ImmutablePutOutcome::AlreadyPresent
        );
        let store_for_collision = store.clone();
        let immutable_for_collision = immutable.clone();
        assert!(blocking(
            move || store_for_collision.put_immutable(&immutable_for_collision, b"two")
        )
        .await
        .is_err());

        let store_for_get = store.clone();
        let immutable_for_get = immutable.clone();
        let found = blocking(move || store_for_get.get(&immutable_for_get))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.bytes, b"one");
        assert_eq!(found.etag, sha256_bytes(b"one"));

        let head = ObjectKey::parse(
            ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_head.json",
        )
        .unwrap();
        let store_for_create = store.clone();
        let head_for_create = head.clone();
        let created = blocking(move || {
            store_for_create.compare_and_swap(
                &head_for_create,
                &CasExpectation::Absent,
                b"head-one",
            )
        })
        .await
        .unwrap();
        let CasOutcome::Written { etag } = created else {
            panic!("head create should succeed")
        };
        assert_eq!(etag, sha256_bytes(b"head-one"));

        let store_for_stale = store.clone();
        let head_for_stale = head.clone();
        let stale = blocking(move || {
            store_for_stale.compare_and_swap(
                &head_for_stale,
                &CasExpectation::Match("stale".to_string()),
                b"wrong",
            )
        })
        .await
        .unwrap();
        assert_eq!(
            stale,
            CasOutcome::Conflict {
                current_etag: Some(etag.clone())
            }
        );

        let store_for_update = store.clone();
        let head_for_update = head.clone();
        let updated = blocking(move || {
            store_for_update.compare_and_swap(
                &head_for_update,
                &CasExpectation::Match(etag),
                b"head-two",
            )
        })
        .await
        .unwrap();
        assert_eq!(
            updated,
            CasOutcome::Written {
                etag: sha256_bytes(b"head-two")
            }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn put_many_bulk_uploads_and_adopts_existing_objects() {
        let server = TestS3::start().await;
        let store = server.store();
        let repository = ".mallard/v1/repositories/0123456789abcdef0123456789abcdef";
        let existing =
            ObjectKey::parse(format!("{}/_objects/{}", repository, "b".repeat(64))).unwrap();
        server.seed(existing.as_str().to_string(), b"already there".to_vec());
        let fresh_one =
            ObjectKey::parse(format!("{}/_objects/{}", repository, "c".repeat(64))).unwrap();
        let fresh_two =
            ObjectKey::parse(format!("{}/_objects/{}", repository, "d".repeat(64))).unwrap();
        // More objects than the concurrency window, exercising the semaphore.
        let filler: Vec<ObjectKey> = (0..8)
            .map(|index| {
                ObjectKey::parse(format!("{}/_objects/{:064x}", repository, index)).unwrap()
            })
            .collect();
        let mut objects = vec![
            (fresh_one.clone(), Bytes::from_static(b"payload one")),
            (existing.clone(), Bytes::from_static(b"already there")),
            (fresh_two.clone(), Bytes::from_static(b"payload two")),
        ];
        for (index, key) in filler.iter().enumerate() {
            objects.push((
                key.clone(),
                Bytes::from(format!("filler {}", index).into_bytes()),
            ));
        }
        let store_for_bulk = store.clone();
        let outcomes = blocking(move || store_for_bulk.put_many(objects))
            .await
            .unwrap();
        assert_eq!(outcomes.len(), 11);
        assert_eq!(outcomes[0], ImmutablePutOutcome::Written);
        assert_eq!(outcomes[1], ImmutablePutOutcome::AlreadyPresent);
        assert_eq!(outcomes[2], ImmutablePutOutcome::Written);
        assert!(outcomes[3..]
            .iter()
            .all(|outcome| *outcome == ImmutablePutOutcome::Written));
        {
            let stored = server.objects.lock().expect("test S3 lock");
            assert_eq!(
                stored.get(fresh_one.as_str()).map(Vec::as_slice),
                Some(&b"payload one"[..])
            );
            assert_eq!(
                stored.get(fresh_two.as_str()).map(Vec::as_slice),
                Some(&b"payload two"[..])
            );
            for (index, key) in filler.iter().enumerate() {
                assert_eq!(
                    stored.get(key.as_str()).cloned(),
                    Some(format!("filler {}", index).into_bytes())
                );
            }
        }

        // Conflicting bytes at an existing key fail the batch.
        let conflict = vec![(existing, Bytes::from_static(b"different bytes"))];
        let store_for_conflict = store.clone();
        assert!(blocking(move || store_for_conflict.put_many(conflict))
            .await
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_uses_stable_key_cursors_across_s3_pages() {
        let server = TestS3::start().await;
        for index in 0..1_005 {
            server.seed(
                format!(".mallard/v1/repositories/{:032x}/_tag.json", index),
                index.to_string().into_bytes(),
            );
        }
        // A neighboring key must not leak from the exact prefix.
        server.seed(
            ".mallard/v1/repositories-neighbor/value".to_string(),
            b"no".to_vec(),
        );
        let store = server.store();
        let prefix = ObjectPrefix::parse(".mallard/v1/repositories").unwrap();
        let store_for_first = store.clone();
        let prefix_for_first = prefix.clone();
        let first = blocking(move || store_for_first.list(&prefix_for_first, None, 1_001))
            .await
            .unwrap();
        assert_eq!(first.keys.len(), 1_001);
        let cursor = first.next_cursor.expect("more than 1001 objects");

        let store_for_second = store.clone();
        let prefix_for_second = prefix.clone();
        let second =
            blocking(move || store_for_second.list(&prefix_for_second, Some(&cursor), 1_001))
                .await
                .unwrap();
        assert_eq!(second.keys.len(), 4);
        assert!(second.next_cursor.is_none());
        assert!(second
            .keys
            .windows(2)
            .all(|pair| pair[0].as_str() < pair[1].as_str()));
    }

    #[test]
    fn rejects_unsafe_cas_etags_and_foreign_cursors() {
        assert!(validate_expected_etag("").is_err());
        assert!(validate_expected_etag("bad\"etag").is_err());
        assert!(validate_expected_etag("good-etag").is_ok());
        let prefix =
            ObjectPrefix::parse(".mallard/v1/repositories/0123456789abcdef0123456789abcdef")
                .unwrap();
        let foreign =
            ObjectKey::parse(".mallard/v1/repositories/ffffffffffffffffffffffffffffffff/_tag.json")
                .unwrap();
        assert!(!key_is_under_prefix(&foreign, &prefix));
    }

    async fn test_s3_request(
        request: Request<Incoming>,
        objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    ) -> Result<Response<Full<HyperBytes>>, Infallible> {
        let method = request.method().clone();
        let path = request.uri().path().trim_start_matches('/').to_string();
        let query = request.uri().query().unwrap_or("").to_string();
        let if_match = request
            .headers()
            .get("if-match")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_matches('"').to_string());
        let if_none_match = request
            .headers()
            .get("if-none-match")
            .and_then(|value| value.to_str().ok())
            == Some("*");
        let body = request
            .into_body()
            .collect()
            .await
            .expect("collect test request")
            .to_bytes()
            .to_vec();
        let (bucket, key) = path.split_once('/').unwrap_or((&path, ""));
        if bucket != BUCKET {
            return Ok(xml_error(StatusCode::NOT_FOUND, "NoSuchBucket"));
        }

        if method == Method::GET && key.is_empty() && query.contains("list-type=2") {
            return Ok(list_response(&query, &objects));
        }

        let mut objects = objects.lock().expect("test S3 lock");
        match method {
            Method::GET => Ok(match objects.get(key) {
                Some(bytes) => object_response(StatusCode::OK, bytes.clone(), Some(bytes)),
                None => xml_error(StatusCode::NOT_FOUND, "NoSuchKey"),
            }),
            Method::PUT => {
                let current = objects.get(key);
                let precondition_failed = (if_none_match && current.is_some())
                    || if_match.as_ref().is_some_and(|expected| {
                        current.map(|bytes| sha256_bytes(bytes)).as_deref()
                            != Some(expected.as_str())
                    });
                if precondition_failed {
                    return Ok(xml_error(
                        StatusCode::PRECONDITION_FAILED,
                        "PreconditionFailed",
                    ));
                }
                objects.insert(key.to_string(), body.clone());
                Ok(object_response(StatusCode::OK, Vec::new(), Some(&body)))
            }
            _ => Ok(xml_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "MethodNotAllowed",
            )),
        }
    }

    fn list_response(
        query: &str,
        objects: &Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    ) -> Response<Full<HyperBytes>> {
        let prefix = query_value(query, "prefix").unwrap_or_default();
        let start_after =
            query_value(query, "start-after").or_else(|| query_value(query, "continuation-token"));
        let max_keys = query_value(query, "max-keys")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(S3_MAX_KEYS);
        let objects = objects.lock().expect("test S3 lock");
        let matching = objects
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .filter(|key| start_after.as_ref().is_none_or(|after| *key > after))
            .cloned()
            .collect::<Vec<_>>();
        let truncated = matching.len() > max_keys;
        let page = matching.into_iter().take(max_keys).collect::<Vec<_>>();
        let next = truncated.then(|| page.last().expect("truncated page has a key").clone());
        let mut xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{}</Name><Prefix>{}</Prefix><IsTruncated>{}</IsTruncated><KeyCount>{}</KeyCount>",
            BUCKET,
            prefix,
            truncated,
            page.len()
        );
        for key in &page {
            let bytes = &objects[key];
            xml.push_str(&format!(
                "<Contents><Key>{}</Key><Size>{}</Size><ETag>&quot;{}&quot;</ETag><LastModified>2026-01-01T00:00:00.000Z</LastModified><StorageClass>STANDARD</StorageClass></Contents>",
                key,
                bytes.len(),
                sha256_bytes(bytes)
            ));
        }
        if let Some(next) = next {
            xml.push_str(&format!(
                "<NextContinuationToken>{}</NextContinuationToken>",
                next
            ));
        }
        xml.push_str("</ListBucketResult>");
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/xml")
            .body(Full::new(HyperBytes::from(xml)))
            .unwrap()
    }

    fn object_response(
        status: StatusCode,
        body: Vec<u8>,
        etag_bytes: Option<&[u8]>,
    ) -> Response<Full<HyperBytes>> {
        let mut response = Response::builder().status(status);
        if let Some(bytes) = etag_bytes {
            response = response.header("etag", format!("\"{}\"", sha256_bytes(bytes)));
        }
        response.body(Full::new(HyperBytes::from(body))).unwrap()
    }

    fn xml_error(status: StatusCode, code: &str) -> Response<Full<HyperBytes>> {
        Response::builder()
            .status(status)
            .header("content-type", "application/xml")
            .body(Full::new(HyperBytes::from(format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{}</Code><Message>{}</Message></Error>",
                code, code
            ))))
            .unwrap()
    }

    fn query_value(query: &str, name: &str) -> Option<String> {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == name).then(|| percent_decode(value))
        })
    }

    fn percent_decode(value: &str) -> String {
        let bytes = value.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' && index + 2 < bytes.len() {
                if let Ok(byte) = u8::from_str_radix(
                    std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                    16,
                ) {
                    decoded.push(byte);
                    index += 3;
                    continue;
                }
            }
            decoded.push(if bytes[index] == b'+' {
                b' '
            } else {
                bytes[index]
            });
            index += 1;
        }
        String::from_utf8_lossy(&decoded).into_owned()
    }
}
