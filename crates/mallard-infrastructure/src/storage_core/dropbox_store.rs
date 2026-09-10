//! Dropbox transport for a user-linked shared or team namespace.
//! Bounded upload sessions with strict batch commits. No automatic mutation retries.
#[cfg(any(test, feature = "test-support"))]
pub mod mock;
mod probe;
#[cfg(test)]
mod tests;
mod upload;

use super::{
    CasExpectation, CasOutcome, ConnectionEvidence, ConnectionHealth, DeleteOutcome,
    ExpectedObject, ImmutablePutOutcome, ObjectKey, ObjectPrefix, ObjectStore, StageOutcome,
    StoreListEntry, StoreListPage, StoredMetadata, VerifiedBody, MAX_LIST_PAGE,
};
use crate::dropbox_auth::{
    auth_error_tag, AuthClass, AuthError, DropboxAuthorization, DropboxClient, DropboxCredential,
};
use crate::storage_providers::StorageAdapterError;
use bytes::Bytes;
#[cfg(any(test, feature = "test-support"))]
use mallard_domain::SecretValue;
use mallard_domain::{hex_lower, sha256_hex};
use reqwest::{header::HeaderValue, Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use tokio::runtime::Handle;

pub const DROPBOX_UPLOAD_LIMIT: u64 = 150 * 1024 * 1024;
const RPC_LIMIT: u64 = 4 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 100_000;
const MAX_LIST_BYTES: usize = 32 * 1024 * 1024;
const MAX_LIST_PAGES: usize = 200;
const CONTENT_BLOCK: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Auth,
    /// The access token ran out. A refresh credential renews on the next
    /// request; a manual token needs a replacement.
    Expired,
    Permission,
    Namespace,
    Missing,
    Conflict,
    RateLimit,
    Unknown,
    Invalid,
    Integrity,
    Limit,
    UploadLimit,
    Local,
    Cleanup,
}
impl Error {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Auth => "Dropbox authorization failed. Reconnect Dropbox or replace the manual access token, then test the connection.",
            Self::Expired => "The Dropbox access token expired during the request. Run the operation again.",
            Self::Permission => "Dropbox access is denied. Check the token scopes and namespace permissions.",
            Self::Namespace => "Dropbox rejected the namespace. Use an existing shared or team namespace.",
            Self::Missing => "The Dropbox object is missing.",
            Self::Conflict => "The Dropbox object changed. Refresh the review and try again.",
            Self::RateLimit => "Dropbox rate limit reached. Wait and try again.",
            Self::Unknown => "The Dropbox request result is unknown. Refresh remote state before retrying.",
            Self::Invalid => "Dropbox returned an invalid response or a noncanonical storage path.",
            Self::Integrity => "Dropbox object integrity verification failed.",
            Self::Limit => "Dropbox response or listing exceeds its safety limit.",
            Self::UploadLimit => "Dropbox uploads are limited to 150 MiB per object.",
            Self::Local => "The Dropbox staging file or setup journal could not be saved.",
            Self::Cleanup => "Dropbox probe cleanup needs attention. Restore access and test again; an unknown revision needs manual cleanup.",
        }
    }
}
impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.message().to_owned()
    }
}
type Result<T> = std::result::Result<T, Error>;

/// Routes whose failure cannot have changed anything remote. Only these
/// are retried after a refresh when Dropbox reports the token expired.
fn is_read(route: &str) -> bool {
    matches!(
        route,
        "files/get_metadata"
            | "files/download"
            | "files/list_folder"
            | "files/list_folder/continue"
            | "users/get_current_account"
    )
}

/// Which host and headers a route needs.
#[derive(Clone, Copy)]
enum Endpoint {
    /// RPC host, inside the namespace.
    Rpc,
    /// Content host, inside the namespace.
    Content,
    /// RPC host, the user's own account: no namespace root.
    Account,
}

impl From<AuthError> for Error {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Reconnect => Self::Auth,
            AuthError::Permission => Self::Permission,
            AuthError::Invalid => Self::Invalid,
            AuthError::Remote => Self::Unknown,
            AuthError::Local => Self::Local,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Metadata {
    #[serde(rename = ".tag")]
    kind: String,
    name: String,
    path_lower: String,
    #[serde(default)]
    rev: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    content_hash: String,
    #[serde(default)]
    is_downloadable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    symlink_info: Option<Value>,
}
impl Metadata {
    // Content routes return FileMetadata, not the Metadata tagged union.
    fn from_file_response(bytes: &[u8]) -> Result<Self> {
        Self::from_file_value(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
    }
    fn from_file_value(mut value: Value) -> Result<Self> {
        let object = value.as_object_mut().ok_or(Error::Invalid)?;
        if object.get(".tag").is_some_and(|tag| tag != "file") {
            return Err(Error::Invalid);
        }
        object.insert(".tag".to_owned(), Value::String("file".to_owned()));
        serde_json::from_value(value).map_err(|_| Error::Invalid)
    }
    fn check(&self, path: &str, folder: bool) -> Result<()> {
        if self.path_lower != path || self.name != path.rsplit('/').next().ok_or(Error::Invalid)? {
            return Err(Error::Invalid);
        }
        if folder {
            if self.kind != "folder" {
                return Err(Error::Invalid);
            }
        } else if self.kind != "file"
            || !valid_rev(&self.rev)
            || !self.id.starts_with("id:")
            || self.id.len() > 256
            || self.is_downloadable == Some(false)
            || self.symlink_info.is_some()
            || self.content_hash.len() != 64
            || !lower_hex(&self.content_hash)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
fn classify_conflict(error: &Value) -> Error {
    let reason = match error[".tag"].as_str() {
        Some("path") if error.get("reason").is_some() => &error["reason"], // UploadWriteFailed is a struct.
        Some("path") => &error["path"],
        Some("path_lookup") => &error["path_lookup"],
        Some("path_write") => &error["path_write"],
        Some("payload_too_large") => return Error::UploadLimit,
        Some("content_hash_mismatch") => return Error::Integrity,
        Some("too_many_write_operations") => return Error::RateLimit,
        Some(
            "lookup_failed"
            | "concurrent_session_data_not_allowed"
            | "concurrent_session_not_closed"
            | "concurrent_session_missing_data",
        ) => return Error::Invalid,
        Some("too_many_shared_folder_targets") => return Error::Limit,
        Some("properties_error") => return Error::Unknown,
        _ => return Error::Permission,
    };
    match reason[".tag"].as_str() {
        Some("not_found") => Error::Missing,
        // A file where a folder belongs: nothing can exist below it.
        Some("conflict") if reason["conflict"][".tag"] == "file_ancestor" => Error::Invalid,
        Some("conflict") => Error::Conflict,
        Some("too_many_write_operations") => Error::RateLimit,
        Some("not_file" | "not_folder" | "malformed_path" | "disallowed_name") => Error::Invalid,
        _ => Error::Permission,
    }
}
fn lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_rev(value: &str) -> bool {
    (9..=128).contains(&value.len()) && lower_hex(value)
}
fn canonical_path(path: &str) -> Result<()> {
    if !path.starts_with("/.mallard")
        || path.len() > 4096
        || path.ends_with('/')
        || !path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/._-".contains(&b))
        || path.split('/').count() > 17
        || path
            .split('/')
            .skip(1)
            .any(|c| c.is_empty() || c == "." || c == ".." || c.len() > 255)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn object_path(key: &ObjectKey) -> Result<String> {
    let path = format!("/{}", key.as_str());
    canonical_path(&path)?;
    Ok(path)
}
fn upload_size(size: u64) -> Result<()> {
    if size > DROPBOX_UPLOAD_LIMIT {
        Err(Error::UploadLimit)
    } else {
        Ok(())
    }
}

/// Dropbox hashes each 4 MiB block, then hashes the concatenated digests.
#[derive(Default)]
struct ContentHash {
    outer: Sha256,
    block: Sha256,
    used: usize,
}
impl ContentHash {
    fn update(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let take = (CONTENT_BLOCK - self.used).min(bytes.len());
            self.block.update(&bytes[..take]);
            self.used += take;
            bytes = &bytes[take..];
            if self.used == CONTENT_BLOCK {
                self.outer
                    .update(std::mem::take(&mut self.block).finalize());
                self.used = 0;
            }
        }
    }
    fn finish(mut self) -> String {
        if self.used > 0 {
            self.outer.update(self.block.finalize());
        }
        hex_lower(&self.outer.finalize())
    }
}
fn content_hash(bytes: &[u8]) -> String {
    let mut hash = ContentHash::default();
    hash.update(bytes);
    hash.finish()
}

#[derive(Clone)]
pub struct DropboxObjectStore {
    client: Client,
    authorization: Arc<DropboxAuthorization>,
    commit_gate: Arc<tokio::sync::Mutex<()>>,
    namespace: String,
    runtime: Handle,
    api: String,
    content: String,
}
impl DropboxObjectStore {
    pub fn new(
        namespace: String,
        credential: DropboxCredential,
        runtime: Handle,
    ) -> std::result::Result<Self, String> {
        Self::with_authorization(
            namespace,
            DropboxAuthorization::new(credential, DropboxClient::new().map_err(Error::from)?),
            runtime,
            "https://api.dropboxapi.com/2",
            "https://content.dropboxapi.com/2",
        )
        .map_err(Into::into)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn with_origins(
        namespace: String,
        token: &SecretValue,
        runtime: Handle,
        api: &str,
        content: &str,
    ) -> Result<Self> {
        Self::with_authorization(
            namespace,
            DropboxAuthorization::new(
                DropboxCredential::AccessToken(token.clone()),
                DropboxClient::new()?,
            ),
            runtime,
            api,
            content,
        )
    }
    fn with_authorization(
        namespace: String,
        authorization: DropboxAuthorization,
        runtime: Handle,
        api: &str,
        content: &str,
    ) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|_| Error::Local)?;
        Ok(Self {
            client,
            authorization: Arc::new(authorization),
            commit_gate: Arc::new(tokio::sync::Mutex::new(())),
            namespace,
            runtime,
            api: api.to_owned(),
            content: content.to_owned(),
        })
    }
    async fn request(&self, route: &'static str, endpoint: Endpoint) -> Result<RequestBuilder> {
        let base = match endpoint {
            Endpoint::Content => &self.content,
            Endpoint::Rpc | Endpoint::Account => &self.api,
        };
        let mut request = self
            .client
            .post(format!("{base}/{route}"))
            .header("Authorization", self.authorization.header().await?);
        if !matches!(endpoint, Endpoint::Account) {
            request = request.header(
                "Dropbox-API-Path-Root",
                json!({".tag":"namespace_id", "namespace_id":self.namespace}).to_string(),
            );
        }
        Ok(request)
    }
    /// Send one request to `route` and classify the answer. A read whose
    /// managed token Dropbox reports expired is sent once more after a
    /// refresh; a write is never sent twice. Every failure Dropbox
    /// attributes to the credential is recorded beside that credential.
    async fn send_checked(
        &self,
        route: &'static str,
        endpoint: Endpoint,
        build: impl Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<Response> {
        let retry = is_read(route) && self.authorization.is_managed();
        match self.send_once(route, endpoint, &build, true).await {
            Err(Error::Expired) if retry => self.send_once(route, endpoint, &build, false).await,
            first => first,
        }
    }
    async fn send_once(
        &self,
        route: &'static str,
        endpoint: Endpoint,
        build: &impl Fn(RequestBuilder) -> RequestBuilder,
        may_refresh: bool,
    ) -> Result<Response> {
        let request = build(self.request(route, endpoint).await?);
        match request.send().await {
            Ok(response) => self.checked(route, response, may_refresh).await,
            Err(_) => {
                self.authorization.record(
                    AuthClass::Unreachable,
                    Some(ConnectionEvidence {
                        endpoint: route,
                        http_status: None,
                        error_tag: None,
                        request_id: None,
                    }),
                );
                Err(Error::Unknown)
            }
        }
    }
    /// The message for a failure, with the recorded evidence when Dropbox
    /// attributed the failure to the credential.
    fn describe(&self, error: Error) -> String {
        match error {
            Error::Auth | Error::Expired | Error::Permission => self
                .authorization
                .health()
                .filter(ConnectionHealth::suspends)
                .map(|health| health.describe())
                .unwrap_or_else(|| error.message().to_owned()),
            other => other.message().to_owned(),
        }
    }
    /// The typed setup error for a probe failure, coded by cause.
    pub(crate) fn adapter_error(&self, error: Error) -> StorageAdapterError {
        let message = self.describe(error);
        match error {
            Error::Cleanup => {
                StorageAdapterError::cleanup_required("dropbox_probe_cleanup_required", message)
            }
            Error::Auth => StorageAdapterError::remote("dropbox_reconnect_required", message),
            Error::Expired => StorageAdapterError::remote("dropbox_token_expired", message),
            Error::Permission => StorageAdapterError::remote("dropbox_permission_denied", message),
            Error::Namespace => StorageAdapterError::remote("dropbox_namespace_rejected", message),
            Error::Unknown => StorageAdapterError::remote("dropbox_request_failed", message),
            Error::RateLimit => StorageAdapterError::remote("dropbox_rate_limited", message),
            _ => StorageAdapterError::remote("dropbox_probe_failed", message),
        }
    }
    async fn bounded(mut response: Response, max: u64) -> Result<Vec<u8>> {
        if response.content_length().is_some_and(|s| s > max) {
            return Err(Error::Limit);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unknown)? {
            if (bytes.len() as u64).saturating_add(chunk.len() as u64) > max {
                return Err(Error::Limit);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    /// Classify one answer. A failure body is read within bounds for its
    /// fixed error identifier only; the identifier, the status, the route,
    /// and the request ID are the whole of what is kept.
    async fn checked(
        &self,
        route: &'static str,
        response: Response,
        may_refresh: bool,
    ) -> Result<Response> {
        if response.headers().len() > 128
            || response
                .headers()
                .iter()
                .map(|(k, v)| k.as_str().len() + v.len())
                .sum::<usize>()
                > 64 * 1024
        {
            return Err(Error::Invalid);
        }
        if response.status().is_success() {
            self.authorization.record_connected();
            return Ok(response);
        }
        let status = response.status();
        let request_id = ConnectionEvidence::request_id(
            response
                .headers()
                .get("x-dropbox-request-id")
                .map(HeaderValue::as_bytes),
        );
        let bytes = Self::bounded(response, RPC_LIMIT).await?;
        let evidence = |tag| {
            Some(ConnectionEvidence {
                endpoint: route,
                http_status: Some(status.as_u16()),
                error_tag: tag,
                request_id: request_id.clone(),
            })
        };
        Err(match status {
            StatusCode::UNAUTHORIZED => {
                let tag = auth_error_tag(&bytes);
                match tag {
                    Some("expired_access_token") if self.authorization.is_managed() => {
                        if may_refresh {
                            // The cached grant is stale; the caller refreshes.
                            self.authorization.invalidate().await;
                            Error::Expired
                        } else {
                            self.authorization
                                .record(AuthClass::Rejected, evidence(tag));
                            Error::Auth
                        }
                    }
                    Some("expired_access_token") => {
                        self.authorization.record(AuthClass::Expired, evidence(tag));
                        Error::Expired
                    }
                    Some("missing_scope" | "user_suspended" | "route_access_denied") => {
                        self.authorization.record(AuthClass::Denied, evidence(tag));
                        Error::Permission
                    }
                    _ => {
                        self.authorization
                            .record(AuthClass::Rejected, evidence(tag));
                        Error::Auth
                    }
                }
            }
            StatusCode::FORBIDDEN => {
                self.authorization.record(AuthClass::Denied, evidence(None));
                Error::Permission
            }
            StatusCode::UNPROCESSABLE_ENTITY => Error::Namespace,
            StatusCode::TOO_MANY_REQUESTS => Error::RateLimit,
            StatusCode::CONFLICT => {
                let error: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
                classify_conflict(&error["error"])
            }
            _ => Error::Unknown,
        })
    }
    async fn rpc(&self, route: &'static str, args: Value) -> Result<Value> {
        let body = serde_json::to_vec(&args).map_err(|_| Error::Invalid)?;
        self.rpc_bytes(route, body).await
    }
    async fn rpc_bytes(&self, route: &'static str, body: Vec<u8>) -> Result<Value> {
        let response = self
            .send_checked(route, Endpoint::Rpc, |request| {
                request
                    .header("Content-Type", "application/json")
                    .body(body.clone())
            })
            .await?;
        let bytes = Self::bounded(response, RPC_LIMIT).await?;
        serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)
    }
    async fn metadata(&self, path: &str) -> Result<Option<Metadata>> {
        match self
            .rpc(
                "files/get_metadata",
                json!({"path":path, "include_deleted":false}),
            )
            .await
        {
            Ok(value) => Ok(Some(
                serde_json::from_value(value).map_err(|_| Error::Invalid)?,
            )),
            Err(Error::Missing) => Ok(None),
            Err(error) => Err(error),
        }
    }
    /// Fetch one object and stream it into `sink`, returning its metadata
    /// and SHA-256. The response proves its own entry: `path_lower` must
    /// equal the requested path and `name` its last component, exact case.
    /// Ancestor folders are never inspected: Dropbox resolves a path to
    /// exactly one entry regardless of case, so a walk could not change
    /// which bytes arrive, and a file where a folder belongs answers
    /// `not_found` below it.
    async fn download_into(
        &self,
        path: &str,
        max: u64,
        sink: &mut impl Write,
    ) -> Result<Option<(Metadata, String)>> {
        let argument = json!({"path":path}).to_string();
        let response = self
            .send_checked("files/download", Endpoint::Content, |request| {
                request.header("Dropbox-API-Arg", argument.clone())
            })
            .await;
        let mut response = match response {
            Ok(r) => r,
            Err(Error::Missing) => return Ok(None),
            Err(e) => return Err(e),
        };
        let header = response
            .headers()
            .get("dropbox-api-result")
            .ok_or(Error::Invalid)?;
        if header.len() > 16 * 1024 {
            return Err(Error::Invalid);
        }
        let metadata: Metadata = Metadata::from_file_response(header.as_bytes())?;
        metadata.check(path, false)?;
        if metadata.size > max || response.content_length().is_some_and(|size| size > max) {
            return Err(Error::Limit);
        }
        let mut received = 0u64;
        let mut hash = Sha256::new();
        let mut dropbox_hash = ContentHash::default();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unknown)? {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or(Error::Limit)?;
            if received > max || received > metadata.size {
                return Err(Error::Limit);
            }
            hash.update(&chunk);
            dropbox_hash.update(&chunk);
            sink.write_all(&chunk).map_err(|_| Error::Local)?;
        }
        if received != metadata.size || dropbox_hash.finish() != metadata.content_hash {
            return Err(Error::Integrity);
        }
        Ok(Some((metadata, hex_lower(&hash.finalize()))))
    }
    /// Stage one object into a private temporary file.
    async fn download(
        &self,
        path: &str,
        max: u64,
    ) -> Result<Option<(NamedTempFile, Metadata, String)>> {
        let mut file = NamedTempFile::new().map_err(|_| Error::Local)?;
        Ok(self
            .download_into(path, max, &mut file)
            .await?
            .map(|(metadata, hash)| (file, metadata, hash)))
    }
    async fn get_metadata_async(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>> {
        let path = object_path(key)?;
        let mut bytes = Vec::new();
        Ok(self
            .download_into(&path, max_bytes, &mut bytes)
            .await?
            .map(|(metadata, _)| StoredMetadata {
                bytes,
                etag: metadata.rev,
            }))
    }
    async fn stage_verified_async(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome> {
        let path = object_path(key)?;
        mallard_domain::validate_sha256("expected object", &expected.sha256)
            .map_err(|_| Error::Invalid)?;
        let Some((file, m, hash)) = self.download(&path, expected.size).await? else {
            return Ok(StageOutcome::Missing);
        };
        if m.size != expected.size || hash != expected.sha256 {
            return Ok(StageOutcome::Mismatch);
        }
        Ok(StageOutcome::Verified(
            VerifiedBody::from_temp_file(file, m.size, hash, m.rev).map_err(|_| Error::Local)?,
        ))
    }
    /// Run `job` once per input with at most `concurrency` in flight.
    /// Results keep input order; the first failure aborts and drains the
    /// rest.
    async fn concurrent<I, T, F, Fut>(
        &self,
        inputs: Vec<I>,
        concurrency: usize,
        job: F,
    ) -> Result<Vec<T>>
    where
        I: Send + 'static,
        T: Send + 'static,
        F: Fn(Self, I) -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    {
        if concurrency == 0 {
            return Err(Error::Limit);
        }
        let count = inputs.len();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut workers = tokio::task::JoinSet::new();
        for (index, input) in inputs.into_iter().enumerate() {
            let store = self.clone();
            let semaphore = semaphore.clone();
            let job = job.clone();
            workers.spawn(async move {
                let _permit = semaphore.acquire_owned().await.map_err(|_| Error::Local)?;
                job(store, input).await.map(|value| (index, value))
            });
        }
        let mut slots: Vec<Option<T>> = Vec::with_capacity(count);
        slots.resize_with(count, || None);
        while let Some(joined) = workers.join_next().await {
            match joined.unwrap_or(Err(Error::Unknown)) {
                Ok((index, value)) => slots[index] = Some(value),
                Err(error) => {
                    workers.abort_all();
                    while workers.join_next().await.is_some() {}
                    return Err(error);
                }
            }
        }
        slots
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(Error::Invalid)
    }
    async fn delete_path(&self, path: &str, revision: &str) -> Result<DeleteOutcome> {
        if !valid_rev(revision) {
            return Err(Error::Invalid);
        }
        match self
            .rpc(
                "files/delete_v2",
                json!({"path":path,"parent_rev":revision}),
            )
            .await
        {
            Ok(value) => {
                let deleted: Metadata = serde_json::from_value(value["metadata"].clone())
                    .map_err(|_| Error::Unknown)?;
                deleted.check(path, false).map_err(|_| Error::Unknown)?;
                if deleted.rev != revision {
                    return Err(Error::Unknown);
                }
                Ok(DeleteOutcome::Deleted)
            }
            Err(Error::Missing) => Ok(DeleteOutcome::Missing),
            // The revision moved: report what is there now.
            Err(Error::Conflict) => match self.metadata(path).await? {
                None => Ok(DeleteOutcome::Missing),
                Some(m) => {
                    m.check(path, false)?;
                    Ok(DeleteOutcome::Conflict {
                        current_etag: m.rev,
                    })
                }
            },
            Err(error) => Err(error),
        }
    }
    async fn enumerate(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err(Error::Limit);
        }
        let path = format!("/{}", prefix.as_str());
        canonical_path(&path)?;
        if let Some(cursor) = cursor {
            let key = ObjectKey::parse(cursor).map_err(|_| Error::Invalid)?;
            object_path(&key)?;
            if !cursor.starts_with(&format!("{}/", prefix.as_str())) {
                return Err(Error::Invalid);
            }
        }
        let mut page = match self
            .rpc(
                "files/list_folder",
                json!({"path":path,"recursive":true,"include_deleted":false,"include_non_downloadable_files":true,"limit":1000}),
            )
            .await
        {
            Ok(page) => page,
            Err(Error::Missing) => {
                return Ok(StoreListPage {
                    entries: vec![],
                    next_cursor: None,
                })
            }
            Err(error) => return Err(error),
        };
        let mut files = BTreeMap::new();
        let mut seen = std::collections::BTreeSet::new();
        let started = Instant::now();
        let mut byte_count = 0usize;
        for page_number in 0..MAX_LIST_PAGES {
            let entries = page["entries"].as_array().ok_or(Error::Invalid)?;
            for value in entries {
                let metadata: Metadata =
                    serde_json::from_value(value.clone()).map_err(|_| Error::Invalid)?;
                canonical_path(&metadata.path_lower)?;
                let is_requested_folder = metadata.kind == "folder" && metadata.path_lower == path;
                if !is_requested_folder && !metadata.path_lower.starts_with(&format!("{path}/")) {
                    return Err(Error::Invalid);
                }
                metadata.check(&metadata.path_lower, metadata.kind == "folder")?;
                byte_count = byte_count
                    .checked_add(metadata.path_lower.len() + metadata.rev.len() + 128)
                    .ok_or(Error::Limit)?;
                if !seen.insert(metadata.path_lower.clone()) {
                    return Err(Error::Invalid);
                }
                if seen.len() > MAX_LIST_ENTRIES
                    || byte_count > MAX_LIST_BYTES
                    || started.elapsed() > Duration::from_secs(120)
                {
                    return Err(Error::Limit);
                }
                if metadata.kind == "file" {
                    let key =
                        ObjectKey::parse(&metadata.path_lower[1..]).map_err(|_| Error::Invalid)?;
                    files.insert(
                        key.as_str().to_owned(),
                        StoreListEntry {
                            key,
                            etag: metadata.rev,
                        },
                    );
                }
            }
            if !page["has_more"].as_bool().ok_or(Error::Invalid)? {
                break;
            }
            if page_number + 1 == MAX_LIST_PAGES {
                return Err(Error::Limit);
            }
            let next = page["cursor"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 16 * 1024)
                .ok_or(Error::Invalid)?;
            page = self
                .rpc("files/list_folder/continue", json!({"cursor":next}))
                .await?;
        }
        let mut remaining = files
            .into_values()
            .filter(|entry| cursor.is_none_or(|c| entry.key.as_str() > c));
        let entries: Vec<_> = remaining.by_ref().take(limit).collect();
        let next_cursor = if remaining.next().is_some() {
            entries.last().map(|e| e.key.as_str().to_owned())
        } else {
            None
        };
        Ok(StoreListPage {
            entries,
            next_cursor,
        })
    }
}
impl ObjectStore for DropboxObjectStore {
    fn connection_health(&self) -> Option<ConnectionHealth> {
        self.authorization.health()
    }
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> std::result::Result<Option<StoredMetadata>, String> {
        self.runtime
            .block_on(self.get_metadata_async(key, max_bytes))
            .map_err(|error| self.describe(error))
    }
    /// Bounded-concurrency metadata reads. Order matches `keys`; the first
    /// failure aborts and drains every remaining request.
    fn get_metadata_many(
        &self,
        keys: &[ObjectKey],
        max_bytes: u64,
    ) -> std::result::Result<Vec<Option<StoredMetadata>>, String> {
        if keys.len() <= 1 {
            return keys
                .iter()
                .map(|key| self.get_metadata(key, max_bytes))
                .collect();
        }
        self.runtime
            .block_on(self.concurrent(
                keys.to_vec(),
                crate::app_settings::cloud_transfer_concurrency(),
                move |store, key| async move { store.get_metadata_async(&key, max_bytes).await },
            ))
            .map_err(|error| self.describe(error))
    }
    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> std::result::Result<StageOutcome, String> {
        self.runtime
            .block_on(self.stage_verified_async(key, expected))
            .map_err(|error| self.describe(error))
    }
    /// Bounded-concurrency verified staging. Order matches `requests`; the
    /// first failure aborts and drains every remaining transfer.
    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> std::result::Result<Vec<StageOutcome>, String> {
        if requests.len() <= 1 {
            return requests
                .iter()
                .map(|(key, expected)| self.stage_verified(key, expected))
                .collect();
        }
        self.runtime
            .block_on(self.concurrent(
                requests.to_vec(),
                crate::app_settings::cloud_transfer_concurrency(),
                |store, (key, expected)| async move {
                    store.stage_verified_async(&key, &expected).await
                },
            ))
            .map_err(|error| self.describe(error))
    }
    fn put_immutable(
        &self,
        key: &ObjectKey,
        bytes: &[u8],
    ) -> std::result::Result<ImmutablePutOutcome, String> {
        object_path(key)?;
        upload_size(bytes.len() as u64)?;
        self.put_many(vec![(key.clone(), Bytes::copy_from_slice(bytes))])?
            .pop()
            .ok_or_else(|| Error::Invalid.into())
    }
    fn put_many(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
    ) -> std::result::Result<Vec<ImmutablePutOutcome>, String> {
        self.runtime
            .block_on(
                self.put_many_async(objects, crate::app_settings::cloud_transfer_concurrency()),
            )
            .map_err(|error| self.describe(error))
    }
    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expected: &CasExpectation,
        bytes: &[u8],
    ) -> std::result::Result<CasOutcome, String> {
        let path = object_path(key)?;
        upload_size(bytes.len() as u64)?;
        let mode = match expected {
            CasExpectation::Absent => json!({".tag":"add"}),
            CasExpectation::Match(rev) => {
                if !valid_rev(rev) {
                    return Err(Error::Invalid.into());
                }
                json!({".tag":"update","update":rev})
            }
        };
        self.runtime
            .block_on(async {
                match self
                    .upload(&path, mode, Bytes::copy_from_slice(bytes))
                    .await
                {
                    Ok(m) => Ok(CasOutcome::Written { etag: m.rev }),
                    Err(Error::Conflict) => {
                        let current = self.metadata(&path).await?;
                        if let Some(m) = &current {
                            m.check(&path, false)?;
                        }
                        Ok(CasOutcome::Conflict {
                            current_etag: current.map(|m| m.rev),
                        })
                    }
                    // Equal bytes after an unknown write do not prove who won the CAS.
                    Err(error) => Err(error),
                }
            })
            .map_err(|error| self.describe(error))
    }
    fn supports_conditional_delete(&self) -> bool {
        true
    }
    fn delete(
        &self,
        key: &ObjectKey,
        expected_etag: &str,
    ) -> std::result::Result<DeleteOutcome, String> {
        let path = object_path(key)?;
        self.runtime
            .block_on(self.delete_path(&path, expected_etag))
            .map_err(|error| self.describe(error))
    }
    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> std::result::Result<StoreListPage, String> {
        self.runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(120),
                    self.enumerate(prefix, cursor, limit),
                )
                .await
                .map_err(|_| Error::Limit)?
            })
            .map_err(|error| self.describe(error))
    }
}
