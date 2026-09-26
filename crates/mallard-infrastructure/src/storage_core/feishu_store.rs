//! Feishu Drive transport for one folder the customer's self-built app can
//! reach. Logical keys never change; below the storage folder three growth
//! families get one shard folder each so no folder outgrows Feishu's 1,500
//! children. One request in flight, 200 ms apart. No claims, no delete.
//! An object upload with an unknown outcome is settled by one listing;
//! a record is one request, never retried.
#[cfg(any(test, feature = "test-support"))]
pub mod mock;
mod probe;
#[cfg(test)]
mod tests;
mod upload;
mod listing;

use super::{
    ConnectionEvidence, ConnectionHealth, ExpectedObject, ImmutablePutOutcome, ListedObject,
    ObjectKey, ObjectPrefix, ObjectStore, StageOutcome, StoreListEntry, StoreListPage,
    StoredMetadata, VerifiedBody, MAX_LIST_PAGE,
};
use crate::feishu_auth::{validate_id, AuthClass, AuthError, FeishuAuthorization};
use crate::storage_providers::StorageAdapterError;
use bytes::Bytes;
use mallard_domain::hex_lower;
use reqwest::{header::HeaderValue, Client, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use tokio::runtime::Handle;

/// Feishu's per-file ceiling for ordinary files.
pub const FEISHU_UPLOAD_LIMIT: u64 = 100 * 1024 * 1024;
/// Above this one `upload_all` request becomes prepare, parts, finish.
pub const SMALL_UPLOAD_LIMIT: u64 = 20 * 1024 * 1024;
/// Feishu documents 1,500 direct children per folder.
pub const MAX_FOLDER_CHILDREN: usize = 1_500;
/// Feishu documents that upload and download calls must not be concurrent
/// and allows five per second.
pub const REQUEST_GAP: Duration = Duration::from_millis(200);
const JSON_LIMIT: u64 = 1024 * 1024;
const PAGE_SIZE: usize = 200;
const MAX_PAGES_PER_FOLDER: usize = 64;
const MAX_LIST_ENTRIES: usize = 100_000;
const MAX_LIST_FOLDERS: usize = 8_192;
const MAX_NAME_BYTES: usize = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    /// The sign-in is refused: only a reconnect recovers.
    Auth,
    /// The access token ran out under a cached grant; the caller refreshes
    /// and repeats a read once.
    Expired,
    Permission,
    Missing,
    RateLimit,
    /// The folder or the space is full.
    Quota,
    /// A read whose request never completed. A write that never got a
    /// response may still have landed, so its result stays `Unknown`.
    Unreachable,
    Unknown,
    Invalid,
    /// Fixed operation and field names only; never remote text or object names.
    InvalidResponse(&'static str, &'static str),
    ApiRejected(&'static str, u64),
    Integrity,
    Limit,
    UploadLimit,
    Local,
    NotSaved,
}

impl Error {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Auth => {
                "Feishu rejected the saved sign-in. Reconnect Feishu in Storage settings."
            }
            Self::Expired => {
                "The Feishu access token expired during the request. Run the operation again."
            }
            Self::Permission => {
                "Feishu denied access. Check the app's permissions and the folder's sharing."
            }
            Self::Missing => "The Feishu object is missing.",
            Self::RateLimit => "Feishu rate limit reached. Wait and try again.",
            Self::Quota => "This Feishu folder or space is full.",
            Self::Unreachable => "Feishu could not be reached. Check the network and try again.",
            Self::Unknown => {
                "The Feishu request result is unknown. Refresh remote state before retrying."
            }
            Self::Invalid => "Feishu returned an invalid response or a name Mallard cannot store.",
            Self::InvalidResponse(_, _) => "Feishu response failed validation.",
            Self::ApiRejected(_, _) => "Feishu rejected the request.",
            Self::Integrity => "Feishu object integrity verification failed.",
            Self::Limit => "Feishu response or listing exceeds its safety limit.",
            Self::UploadLimit => "Feishu uploads are limited to 100 MiB per object.",
            Self::Local => "The Feishu staging file could not be saved.",
            Self::NotSaved => "Mallard could not save the renewed Feishu connection.",
        }
    }
}

impl From<Error> for String {
    fn from(error: Error) -> Self {
        match error {
            Error::InvalidResponse(operation, field) => {
                format!("Feishu response failed validation ({operation}: {field}).")
            }
            Error::ApiRejected(operation, code) => {
                format!("Feishu rejected the request ({operation}, code {code}).")
            }
            other => other.message().to_owned(),
        }
    }
}

impl From<AuthError> for Error {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Reconnect | AuthError::AppRejected => Self::Auth,
            AuthError::Permission | AuthError::ScopeMismatch => Self::Permission,
            AuthError::Invalid => Self::InvalidResponse("Feishu sign-in", "token response"),
            // No Drive request was sent: nothing can have landed.
            AuthError::Remote => Self::Unreachable,
            AuthError::Local => Self::Local,
            AuthError::NotSaved => Self::NotSaved,
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Routes whose failure cannot have changed anything remote. Only these
/// are sent again after a refresh when Feishu reports the token invalid.
fn is_read(route: &str) -> bool {
    matches!(
        route,
        "drive/v1/files" | "drive/v1/files/download" | "authen/v1/user_info"
    )
}

/// The Feishu business codes Mallard keeps as evidence. Nothing outside
/// this list is retained from a body.
const KNOWN_CODES: [(u64, &str); 7] = [
    (99991400, "99991400"),
    (99991661, "99991661"),
    (99991663, "99991663"),
    (99991664, "99991664"),
    (99991668, "99991668"),
    (1061003, "1061003"),
    (1062507, "1062507"),
];

fn code_tag(code: u64) -> Option<&'static str> {
    KNOWN_CODES
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, tag)| *tag)
}

/// The envelope's `code`, when the body is one.
fn envelope_code(bytes: &[u8]) -> Option<u64> {
    serde_json::from_slice::<Value>(bytes).ok()?["code"].as_u64()
}

fn is_token_code(code: u64) -> bool {
    (99991661..=99991679).contains(&code)
}

/// One direct child as the listing reports it.
#[derive(Clone, Debug, Deserialize)]
struct Entry {
    token: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    parent_token: String,
}

/// A name Mallard writes: lowercase letters, digits, `.`, `_`, `-`.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// The three families whose files are placed one shard folder down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Objects,
    Manifests,
    Commits,
}

fn is_hex32(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Which family a logical directory is, if any.
fn family_of(logical_dir: &str) -> Option<Family> {
    let parts: Vec<&str> = logical_dir.split('/').collect();
    match parts.as_slice() {
        [".mallard", "v1", "repositories", bundle, "_objects"] if is_hex32(bundle) => {
            Some(Family::Objects)
        }
        [".mallard", "v1", "repositories", bundle, "_manifests"] if is_hex32(bundle) => {
            Some(Family::Manifests)
        }
        [".mallard", "v1", "repositories", "_commits", bundle] if is_hex32(bundle) => {
            Some(Family::Commits)
        }
        [".mallard", "v1", "library", repository, "_objects"]
            if repository.strip_prefix("library-").is_some_and(is_hex32) =>
        {
            Some(Family::Objects)
        }
        [".mallard", "v1", "library", repository, "_manifests"]
            if repository.strip_prefix("library-").is_some_and(is_hex32) =>
        {
            Some(Family::Manifests)
        }
        [".mallard", "v1", "library", repository, "_commits"]
            if repository.strip_prefix("library-").is_some_and(is_hex32) =>
        {
            Some(Family::Commits)
        }
        _ => None,
    }
}

/// The shard folder a file of `family` named `name` lives in, or nothing
/// when the name is not one Mallard writes there.
fn shard_of(family: Family, name: &str) -> Option<String> {
    match family {
        Family::Objects => {
            if name.len() == 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                Some(name[..2].to_owned())
            } else {
                None
            }
        }
        Family::Manifests => {
            let digits = name.split('-').next()?;
            if digits.is_empty() || digits.len() > 19 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            let revision: u64 = digits.parse().ok()?;
            Some(format!("{:07}", revision / 1000))
        }
        Family::Commits => {
            let digits = name.split('.').next()?;
            if digits.len() != 10 || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let revision: u64 = digits.parse().ok()?;
            Some(format!("{:07}", revision / 1000))
        }
    }
}

fn is_shard_name(family: Family, name: &str) -> bool {
    match family {
        Family::Objects => {
            name.len() == 2
                && name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        Family::Manifests | Family::Commits => {
            name.len() == 7 && name.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

/// Where one logical key lives: the physical folder path below the
/// storage folder, and the file's name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Location {
    dirs: Vec<String>,
    name: String,
}

impl Location {
    fn of(key: &ObjectKey) -> Result<Self> {
        let mut parts: Vec<&str> = key.as_str().split('/').collect();
        let name = parts.pop().ok_or(Error::Invalid)?;
        if !valid_name(name) || !parts.iter().all(|part| valid_name(part)) {
            return Err(Error::Invalid);
        }
        let logical_dir = parts.join("/");
        let mut dirs: Vec<String> = parts.iter().map(|part| (*part).to_owned()).collect();
        if let Some(family) = family_of(&logical_dir) {
            dirs.push(shard_of(family, name).ok_or(Error::Invalid)?);
        }
        Ok(Self {
            dirs,
            name: name.to_owned(),
        })
    }

    fn path(&self) -> String {
        format!("{}/{}", self.dirs.join("/"), self.name)
    }

    /// Whether the name is the SHA-256 of the bytes: an `_objects` payload.
    fn content_addressed(&self) -> bool {
        self.dirs.len() >= 2 && self.dirs[self.dirs.len() - 2] == "_objects"
    }
}

/// What this process learned of the folder tree: each physical directory
/// path's folder tokens (the union of same-named siblings) and each file's
/// token. Filled on resolve, create, and listing; dropped on a not-found
/// answer.
#[derive(Default)]
struct PathCache {
    folders: BTreeMap<String, BTreeSet<String>>,
    files: BTreeMap<String, String>,
}

/// A store over one Feishu folder. Cloned handles share the gate, the
/// authorization, and the caches.
#[derive(Clone)]
pub struct FeishuObjectStore {
    client: Client,
    authorization: Arc<FeishuAuthorization>,
    /// One request in flight; the instant the last one finished.
    gate: Arc<tokio::sync::Mutex<Option<Instant>>>,
    cache: Arc<Mutex<PathCache>>,
    root: String,
    runtime: Handle,
    api: String,
}

impl FeishuObjectStore {
    pub fn new(
        folder_token: String,
        authorization: FeishuAuthorization,
        runtime: Handle,
    ) -> std::result::Result<Self, String> {
        Self::with_origin(
            folder_token,
            authorization,
            runtime,
            crate::feishu_auth::API_ORIGIN,
        )
        .map_err(Into::into)
    }

    pub(crate) fn with_origin(
        folder_token: String,
        authorization: FeishuAuthorization,
        runtime: Handle,
        api: &str,
    ) -> Result<Self> {
        validate_id(&folder_token).map_err(|_| Error::Invalid)?;
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
            gate: Arc::new(tokio::sync::Mutex::new(None)),
            cache: Arc::default(),
            root: folder_token,
            runtime,
            api: api.to_owned(),
        })
    }

    pub fn runtime(&self) -> &Handle {
        &self.runtime
    }

    fn url(&self, route: &str) -> String {
        format!("{}/open-apis/{route}", self.api)
    }

    /// Send one request to `route` and classify the answer. A read whose
    /// token Feishu reports invalid is sent once more after a refresh; a
    /// write is never sent twice.
    async fn send_checked(
        &self,
        route: &'static str,
        build: impl Fn(&Client) -> RequestBuilder,
    ) -> Result<Response> {
        match self.send_once(route, &build, true).await {
            Err(Error::Expired) if is_read(route) => self.send_once(route, &build, false).await,
            first => first,
        }
    }

    /// One request under the gate: the previous request finished at least
    /// `REQUEST_GAP` ago, and nothing else is in flight.
    async fn send_once(
        &self,
        route: &'static str,
        build: &impl Fn(&Client) -> RequestBuilder,
        may_refresh: bool,
    ) -> Result<Response> {
        let mut last = self.gate.lock().await;
        if let Some(finished) = *last {
            let wait = REQUEST_GAP.saturating_sub(finished.elapsed());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
        let result = async {
            let header = self.authorization.header().await.map_err(Error::from)?;
            let request = build(&self.client).header("Authorization", header);
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
                    Err(if is_read(route) {
                        Error::Unreachable
                    } else {
                        Error::Unknown
                    })
                }
            }
        }
        .await;
        *last = Some(Instant::now());
        result
    }

    /// The message for a failure, with the recorded evidence when Feishu
    /// attributed the failure to the sign-in.
    fn describe(&self, error: Error) -> String {
        match error {
            Error::Auth | Error::Permission => self
                .authorization
                .health()
                .filter(ConnectionHealth::suspends)
                .map(|health| health.describe())
                .unwrap_or_else(|| error.message().to_owned()),
            other => other.into(),
        }
    }

    /// The typed setup error for a probe failure, coded by cause.
    pub(crate) fn adapter_error(&self, error: Error) -> StorageAdapterError {
        let message = self.describe(error);
        match error {
            Error::Auth => StorageAdapterError::remote("feishu_reconnect_required", message),
            Error::Expired => StorageAdapterError::remote("feishu_token_expired", message),
            Error::Permission => StorageAdapterError::remote("feishu_permission_denied", message),
            Error::Unreachable => StorageAdapterError::remote("feishu_unreachable", message),
            Error::Unknown => StorageAdapterError::remote("feishu_request_failed", message),
            Error::RateLimit => StorageAdapterError::remote("feishu_rate_limited", message),
            Error::Quota => StorageAdapterError::remote("feishu_quota_reached", message),
            Error::NotSaved => StorageAdapterError::remote("feishu_connection_not_saved", message),
            _ => StorageAdapterError::remote("feishu_probe_failed", message),
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

    /// Classify one answer by status. A failure body is read within
    /// bounds for its fixed `code` only.
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
            return Err(Error::InvalidResponse(route, "response headers"));
        }
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let request_id = ConnectionEvidence::request_id(
            response
                .headers()
                .get("x-tt-logid")
                .map(HeaderValue::as_bytes),
        );
        let bytes = Self::bounded(response, JSON_LIMIT).await?;
        let code = envelope_code(&bytes);
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
                self.token_refused(route, code, may_refresh, evidence(code.and_then(code_tag)))
                    .await
            }
            StatusCode::FORBIDDEN => {
                self.authorization
                    .record(AuthClass::Denied, evidence(code.and_then(code_tag)));
                Error::Permission
            }
            StatusCode::NOT_FOUND => Error::Missing,
            StatusCode::TOO_MANY_REQUESTS => Error::RateLimit,
            _ if status.is_server_error() => {
                if is_read(route) {
                    Error::Unreachable
                } else {
                    Error::Unknown
                }
            }
            _ => {
                self.business_failure(route, code, may_refresh, evidence)
                    .await
            }
        })
    }

    /// A refused token: under a cached grant the caller refreshes and, for
    /// a read, repeats once; a token Feishu refuses right after a refresh
    /// is a rejected sign-in.
    async fn token_refused(
        &self,
        _route: &'static str,
        _code: Option<u64>,
        may_refresh: bool,
        evidence: Option<ConnectionEvidence>,
    ) -> Error {
        if may_refresh {
            self.authorization.invalidate().await;
            Error::Expired
        } else {
            self.authorization.record(AuthClass::Rejected, evidence);
            Error::Auth
        }
    }

    /// A non-zero envelope code, on any status.
    async fn business_failure(
        &self,
        route: &'static str,
        code: Option<u64>,
        may_refresh: bool,
        evidence: impl Fn(Option<&'static str>) -> Option<ConnectionEvidence>,
    ) -> Error {
        let tag = code.and_then(code_tag);
        match code {
            Some(99991400) => Error::RateLimit,
            Some(code) if is_token_code(code) => {
                self.token_refused(route, Some(code), may_refresh, evidence(tag))
                    .await
            }
            Some(1061003) => Error::Missing,
            Some(1062507) => Error::Quota,
            Some(code) => {
                if is_read(route) {
                    Error::ApiRejected(route, code)
                } else {
                    Error::Unknown
                }
            }
            None => Error::InvalidResponse(route, "API error code"),
        }
    }

    /// A JSON request that requires an object payload on `code: 0`.
    async fn rpc(
        &self,
        route: &'static str,
        build: impl Fn(&Client) -> RequestBuilder,
    ) -> Result<Value> {
        let mut value = match self.rpc_once(route, &build, true).await {
            Err(Error::Expired) if is_read(route) => self.rpc_once(route, &build, false).await,
            first => first,
        }?;
        value
            .get_mut("data")
            .map(Value::take)
            .filter(Value::is_object)
            .ok_or(Error::InvalidResponse(route, "data"))
    }

    /// One write acknowledgement: validate success without requiring a payload.
    async fn rpc_ack(
        &self,
        route: &'static str,
        build: impl Fn(&Client) -> RequestBuilder,
    ) -> Result<()> {
        self.rpc_once(route, &build, true).await.map(|_| ())
    }

    /// Validate the HTTP response and JSON success envelope for either contract.
    async fn rpc_once(
        &self,
        route: &'static str,
        build: &impl Fn(&Client) -> RequestBuilder,
        may_refresh: bool,
    ) -> Result<Value> {
        let response = self.send_once(route, build, may_refresh).await?;
        let request_id = ConnectionEvidence::request_id(
            response
                .headers()
                .get("x-tt-logid")
                .map(HeaderValue::as_bytes),
        );
        let bytes = Self::bounded(response, JSON_LIMIT).await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::InvalidResponse(route, "JSON body"))?;
        let code = value["code"].as_u64();
        if code == Some(0) {
            self.authorization.record_connected();
            return Ok(value);
        }
        let evidence = |tag| {
            Some(ConnectionEvidence {
                endpoint: route,
                http_status: Some(200),
                error_tag: tag,
                request_id: request_id.clone(),
            })
        };
        Err(self
            .business_failure(route, code, may_refresh, evidence)
            .await)
    }

    /// The direct children of one folder, every page.
    async fn list_folder(&self, folder: &str) -> Result<Vec<Entry>> {
        validate_id(folder).map_err(|_| Error::Invalid)?;
        let mut entries: Vec<Entry> = Vec::new();
        let mut cursors: BTreeSet<String> = BTreeSet::new();
        let mut cursor: Option<String> = None;
        for page in 0..=MAX_PAGES_PER_FOLDER {
            if page == MAX_PAGES_PER_FOLDER {
                return Err(Error::Limit);
            }
            let mut query = vec![
                ("folder_token", folder.to_owned()),
                ("page_size", PAGE_SIZE.to_string()),
                ("order_by", "CreatedTime".to_owned()),
                ("direction", "ASC".to_owned()),
            ];
            if let Some(cursor) = &cursor {
                query.push(("page_token", cursor.clone()));
            }
            let url = url::Url::parse_with_params(&self.url("drive/v1/files"), &query)
                .map_err(|_| Error::Invalid)?;
            let data = self
                .rpc("drive/v1/files", move |client| client.get(url.clone()))
                .await?;
            let invalid = |field| Error::InvalidResponse("drive/v1/files", field);
            let files = data["files"].as_array().ok_or_else(|| invalid("files"))?;
            if files.len() > PAGE_SIZE {
                return Err(invalid("files count"));
            }
            for value in files {
                let entry: Entry =
                    serde_json::from_value(value.clone()).map_err(|_| invalid("file entry"))?;
                validate_id(&entry.token).map_err(|_| invalid("file token"))?;
                if entry.parent_token != folder {
                    return Err(invalid("parent_token mismatch"));
                }
                if entry.name.len() > 1024 {
                    return Err(invalid("file name length"));
                }
                entries.push(entry);
                if entries.len() > MAX_FOLDER_CHILDREN.saturating_mul(2) {
                    return Err(Error::Limit);
                }
            }
            if !data["has_more"]
                .as_bool()
                .ok_or_else(|| invalid("has_more"))?
            {
                break;
            }
            let next = data["next_page_token"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 4096)
                .ok_or_else(|| invalid("next_page_token"))?;
            if !cursors.insert(next.to_owned()) {
                return Err(invalid("repeated next_page_token"));
            }
            cursor = Some(next.to_owned());
        }
        Ok(entries)
    }

    async fn create_folder(&self, parent: &str, name: &str) -> Result<String> {
        validate_id(parent).map_err(|_| Error::Invalid)?;
        if !valid_name(name) {
            return Err(Error::Invalid);
        }
        let url = self.url("drive/v1/files/create_folder");
        let body = serde_json::json!({"name": name, "folder_token": parent});
        let data = self
            .rpc("drive/v1/files/create_folder", move |client| {
                client.post(&url).json(&body)
            })
            .await?;
        let invalid = Error::InvalidResponse("drive/v1/files/create_folder", "token");
        let token = data["token"].as_str().ok_or(invalid)?;
        validate_id(token).map_err(|_| invalid)?;
        Ok(token.to_owned())
    }

    fn cached_folders(&self, path: &str) -> Option<BTreeSet<String>> {
        self.cache.lock().ok()?.folders.get(path).cloned()
    }

    fn remember_folders(&self, path: &str, tokens: &BTreeSet<String>) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.folders.insert(path.to_owned(), tokens.clone());
        }
    }

    fn cached_file(&self, path: &str) -> Option<String> {
        self.cache.lock().ok()?.files.get(path).cloned()
    }

    fn remember_file(&self, path: &str, token: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.files.insert(path.to_owned(), token.to_owned());
        }
    }

    /// A not-found answer for `path`: forget it and every path below.
    fn forget(&self, path: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            let below = format!("{path}/");
            cache.files.remove(path);
            cache
                .folders
                .retain(|known, _| known != path && !known.starts_with(&below));
            cache.files.retain(|known, _| !known.starts_with(&below));
        }
    }

    /// The folder tokens of one physical directory path: every same-named
    /// sibling at each level, since two computers may have created one
    /// folder at once. With `create`, a missing level is created in the
    /// parent with the smallest token and the level is listed again, so a
    /// sibling created meanwhile is part of the union.
    async fn resolve(
        &self,
        dirs: &[String],
        create: bool,
        use_cache: bool,
    ) -> Result<Option<BTreeSet<String>>> {
        let mut current: BTreeSet<String> = BTreeSet::from([self.root.clone()]);
        let mut path = String::new();
        for part in dirs {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            if use_cache {
                if let Some(known) = self.cached_folders(&path) {
                    current = known;
                    continue;
                }
            }
            let mut found = BTreeSet::new();
            for parent in &current {
                let listed = match self.list_folder(parent).await {
                    Ok(listed) => listed,
                    Err(Error::Missing) => {
                        self.forget(&path);
                        return Err(Error::Missing);
                    }
                    Err(error) => return Err(error),
                };
                for entry in listed {
                    if entry.kind == "folder" && entry.name == *part {
                        found.insert(entry.token);
                    }
                }
            }
            if found.is_empty() {
                if !create {
                    return Ok(None);
                }
                let parent = current.iter().next().ok_or(Error::Invalid)?.clone();
                let created = self.create_folder(&parent, part).await?;
                for entry in self.list_folder(&parent).await? {
                    if entry.kind == "folder" && entry.name == *part {
                        found.insert(entry.token);
                    }
                }
                if !found.contains(&created) {
                    return Err(Error::InvalidResponse(
                        "drive/v1/files/create_folder",
                        "created folder missing from listing",
                    ));
                }
            }
            self.remember_folders(&path, &found);
            current = found;
        }
        Ok(Some(current))
    }

    /// The files in the directory by name, each with its smallest token,
    /// how many children the directory holds, and the folder writes go to.
    /// Every file token seen is remembered.
    async fn files_in(
        &self,
        dirs: &[String],
        create: bool,
    ) -> Result<Option<(BTreeMap<String, String>, usize, String)>> {
        let Some(folders) = self.resolve(dirs, create, true).await? else {
            return Ok(None);
        };
        self.files_in_resolved(dirs, &folders).await.map(Some)
    }

    async fn files_in_resolved(
        &self,
        dirs: &[String],
        folders: &BTreeSet<String>,
    ) -> Result<(BTreeMap<String, String>, usize, String)> {
        let path = dirs.join("/");
        let mut count = 0;
        let mut by_name: BTreeMap<String, String> = BTreeMap::new();
        for folder in folders {
            let listed = match self.list_folder(folder).await {
                Ok(listed) => listed,
                Err(Error::Missing) => {
                    self.forget(&path);
                    return Err(Error::Missing);
                }
                Err(error) => return Err(error),
            };
            count += listed.len();
            for entry in listed {
                if entry.kind != "file" {
                    continue;
                }
                let token = by_name
                    .entry(entry.name)
                    .or_insert_with(|| entry.token.clone());
                if entry.token < *token {
                    *token = entry.token;
                }
            }
        }
        for (name, token) in &by_name {
            self.remember_file(&format!("{path}/{name}"), token);
        }
        let target = folders.iter().next().ok_or(Error::Invalid)?.clone();
        Ok((by_name, count, target))
    }

    /// The token of the file at `location`: from the cache, else from one
    /// listing of its directory.
    async fn find_file(&self, location: &Location) -> Result<Option<String>> {
        let path = location.path();
        if let Some(token) = self.cached_file(&path) {
            return Ok(Some(token));
        }
        let Some((files, _, _)) = self.files_in(&location.dirs, false).await? else {
            return Ok(None);
        };
        Ok(files.get(&location.name).cloned())
    }

    /// Fetch one file's bytes into `sink`, returning the size and SHA-256.
    /// A missing token drops the cached path and reads as absent.
    async fn download_into(
        &self,
        location: &Location,
        token: &str,
        max: u64,
        sink: &mut impl Write,
    ) -> Result<Option<(u64, String)>> {
        validate_id(token).map_err(|_| Error::Invalid)?;
        let url = self.url(&format!("drive/v1/files/{token}/download"));
        let response = self
            .send_checked("drive/v1/files/download", move |client| client.get(&url))
            .await;
        let mut response = match response {
            Ok(response) => response,
            Err(Error::Missing) => {
                self.forget(&location.path());
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        // Successful downloads contain file bytes with the file's MIME type,
        // including application/json. HTTP failures were classified above.
        if response.content_length().is_some_and(|size| size > max) {
            return Err(Error::Limit);
        }
        let mut received = 0u64;
        let mut hash = Sha256::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unknown)? {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or(Error::Limit)?;
            if received > max {
                return Err(Error::Limit);
            }
            hash.update(&chunk);
            sink.write_all(&chunk).map_err(|_| Error::Local)?;
        }
        self.authorization.record_connected();
        Ok(Some((received, hex_lower(&hash.finalize()))))
    }

    async fn get_metadata_async(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>> {
        let location = Location::of(key)?;
        let Some(token) = self.find_file(&location).await? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        Ok(self
            .download_into(&location, &token, max_bytes, &mut bytes)
            .await?
            .map(|_| StoredMetadata { bytes, etag: token }))
    }

    async fn stage_verified_async(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome> {
        mallard_domain::validate_sha256("expected object", &expected.sha256)
            .map_err(|_| Error::Invalid)?;
        let location = Location::of(key)?;
        let Some(token) = self.find_file(&location).await? else {
            return Ok(StageOutcome::Missing);
        };
        self.stage_token(&location, &token, expected).await
    }

    /// Download the file at `token` into a private temporary file and check
    /// it against `expected`. A token that is gone reads as missing and its
    /// path is forgotten; nothing is resolved again here.
    async fn stage_token(
        &self,
        location: &Location,
        token: &str,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome> {
        let mut file = NamedTempFile::new().map_err(|_| Error::Local)?;
        let Some((size, hash)) = self
            .download_into(location, token, expected.size, &mut file)
            .await?
        else {
            return Ok(StageOutcome::Missing);
        };
        if size != expected.size || hash != expected.sha256 {
            return Ok(StageOutcome::Mismatch);
        }
        Ok(StageOutcome::Verified(
            VerifiedBody::from_temp_file(file, size, hash, token.to_owned())
                .map_err(|_| Error::Local)?,
        ))
    }

    /// Every request together: a cached token is used as it is, the rest
    /// are resolved through one listing of each physical folder, and each
    /// request is then one download by token. The tokens live only in this
    /// call. `on_staged` hears each request's expected size once it is
    /// settled.
    async fn stage_many_async(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
        on_staged: &mut dyn FnMut(u64),
    ) -> Result<Vec<StageOutcome>> {
        // Validate every request before any I/O.
        let mut located = Vec::with_capacity(requests.len());
        for (key, expected) in requests {
            mallard_domain::validate_sha256("expected object", &expected.sha256)
                .map_err(|_| Error::Invalid)?;
            located.push(Location::of(key)?);
        }
        let mut tokens: Vec<Option<String>> = located
            .iter()
            .map(|location| self.cached_file(&location.path()))
            .collect();
        let unresolved: BTreeSet<Vec<String>> = located
            .iter()
            .zip(&tokens)
            .filter(|(_, token)| token.is_none())
            .map(|(location, _)| location.dirs.clone())
            .collect();
        if !unresolved.is_empty() {
            let directories: Vec<Vec<String>> = unresolved.into_iter().collect();
            let folders = self.resolve_directories(&directories, false).await?;
            let mut files: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
            for (path, tokens_of) in &folders {
                let dirs: Vec<String> = path.split('/').map(str::to_owned).collect();
                let (listed, _, _) = self.files_in_resolved(&dirs, tokens_of).await?;
                files.insert(path.clone(), listed);
            }
            for (location, token) in located.iter().zip(tokens.iter_mut()) {
                if token.is_none() {
                    *token = files
                        .get(&location.dirs.join("/"))
                        .and_then(|listed| listed.get(&location.name).cloned());
                }
            }
        }
        let mut outcomes = Vec::with_capacity(requests.len());
        for ((location, token), (_, expected)) in located.iter().zip(&tokens).zip(requests) {
            let outcome = match token {
                Some(token) => self.stage_token(location, token, expected).await?,
                None => StageOutcome::Missing,
            };
            on_staged(expected.size);
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    /// Every file below `prefix`, breadth first, with shard folders
    /// dropped from the keys. A name Mallard never writes is reported and
    /// skipped; two files of one name are one key with the smallest token.
    async fn enumerate(
        &self,
        prefix: &ObjectPrefix,
        max_objects: usize,
    ) -> Result<Vec<StoreListEntry>> {
        let parts: Vec<String> = prefix.as_str().split('/').map(str::to_owned).collect();
        if !parts.iter().all(|part| valid_name(part)) {
            return Err(Error::Invalid);
        }
        let Some(roots) = self.resolve(&parts, false, false).await? else {
            return Ok(vec![]);
        };
        let started = Instant::now();
        // (logical directory, physical directory path, folder token, the
        // shard folder this is, if it is one)
        let mut queue: std::collections::VecDeque<(String, String, String, Option<String>)> = roots
            .into_iter()
            .map(|token| (prefix.as_str().to_owned(), parts.join("/"), token, None))
            .collect();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut files: BTreeMap<String, String> = BTreeMap::new();
        let mut folder_cache: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        while let Some((logical, physical, token, shard)) = queue.pop_front() {
            if !visited.insert(token.clone()) {
                continue;
            }
            if visited.len() > MAX_LIST_FOLDERS || started.elapsed() > Duration::from_secs(120) {
                return Err(Error::Limit);
            }
            let family = family_of(&logical);
            for entry in self.list_folder(&token).await? {
                if !valid_name(&entry.name) {
                    super::publication::report_ignored_name(&format!("{logical}/{}", entry.name));
                    continue;
                }
                match entry.kind.as_str() {
                    "folder" => {
                        let transparent = shard.is_none()
                            && family.is_some_and(|family| is_shard_name(family, &entry.name));
                        let child_physical = format!("{physical}/{}", entry.name);
                        folder_cache
                            .entry(child_physical.clone())
                            .or_default()
                            .insert(entry.token.clone());
                        if transparent {
                            queue.push_back((
                                logical.clone(),
                                child_physical,
                                entry.token,
                                Some(entry.name),
                            ));
                        } else {
                            queue.push_back((
                                format!("{logical}/{}", entry.name),
                                child_physical,
                                entry.token,
                                None,
                            ));
                        }
                    }
                    "file" => {
                        let placed = match (family, &shard) {
                            (Some(family), Some(shard)) => {
                                shard_of(family, &entry.name).as_deref() == Some(shard.as_str())
                            }
                            (Some(_), None) => false,
                            (None, _) => true,
                        };
                        let key = format!("{logical}/{}", entry.name);
                        if !placed {
                            super::publication::report_ignored_name(&key);
                            continue;
                        }
                        let Ok(parsed) = ObjectKey::parse(&key) else {
                            super::publication::report_ignored_name(&key);
                            continue;
                        };
                        let current = files
                            .entry(parsed.as_str().to_owned())
                            .or_insert_with(|| entry.token.clone());
                        if entry.token < *current {
                            *current = entry.token.clone();
                        }
                        self.remember_file(&format!("{physical}/{}", entry.name), current);
                        if files.len() > max_objects.min(MAX_LIST_ENTRIES) {
                            return Err(Error::Limit);
                        }
                    }
                    _ => {
                        super::publication::report_ignored_name(&format!(
                            "{logical}/{}",
                            entry.name
                        ));
                    }
                }
            }
        }
        for (path, tokens) in folder_cache {
            self.remember_folders(&path, &tokens);
        }
        files
            .into_iter()
            .map(|(key, token)| {
                Ok(StoreListEntry {
                    key: ObjectKey::parse(key).map_err(|_| Error::Invalid)?,
                    etag: token,
                })
            })
            .collect()
    }
}

impl ObjectStore for FeishuObjectStore {
    fn list_objects_batch(
        &self,
        queries: &[super::ObjectQuery],
        max_objects: usize,
    ) -> Vec<std::result::Result<Vec<ListedObject>, String>> {
        self.list_batch(queries, max_objects)
    }

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

    fn list_objects(
        &self,
        prefix: &ObjectPrefix,
        max_objects: usize,
    ) -> std::result::Result<Vec<ListedObject>, String> {
        self.runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(120),
                    self.enumerate(prefix, max_objects),
                )
                .await
                .map_err(|_| Error::Limit)?
            })
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| ListedObject {
                        key: entry.key,
                        feishu: Some(super::object_store::FeishuReadAddress {
                            root: self.root.clone(),
                            api: self.api.clone(),
                            token: entry.etag,
                        }),
                    })
                    .collect()
            })
            .map_err(|error| self.describe(error))
    }

    fn get_listed_metadata_many(
        &self,
        objects: &[ListedObject],
        max_bytes: u64,
    ) -> std::result::Result<Vec<Option<StoredMetadata>>, String> {
        // Reject foreign references before sending any request.
        if objects.iter().any(|object| {
            object
                .feishu
                .as_ref()
                .is_some_and(|address| address.root != self.root || address.api != self.api)
        }) {
            return Err("listed object belongs to another Feishu storage folder".to_string());
        }
        self.runtime
            .block_on(async {
                let mut results = Vec::with_capacity(objects.len());
                for object in objects {
                    let Some(address) = &object.feishu else {
                        results.push(self.get_metadata_async(&object.key, max_bytes).await?);
                        continue;
                    };
                    let location = Location::of(&object.key)?;
                    let mut bytes = Vec::new();
                    results.push(
                        self.download_into(&location, &address.token, max_bytes, &mut bytes)
                            .await?
                            .map(|_| StoredMetadata {
                                bytes,
                                etag: address.token.clone(),
                            }),
                    );
                }
                Ok(results)
            })
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

    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> std::result::Result<Vec<StageOutcome>, String> {
        self.stage_many_with_progress(requests, &mut |_| {})
    }

    /// Every request at once: each folder is resolved and listed once, and
    /// every download is by token.
    fn stage_many_with_progress(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
        on_staged: &mut dyn FnMut(u64),
    ) -> std::result::Result<Vec<StageOutcome>, String> {
        self.runtime
            .block_on(self.stage_many_async(requests, on_staged))
            .map_err(|error| self.describe(error))
    }

    fn put_immutable(
        &self,
        key: &ObjectKey,
        bytes: &[u8],
    ) -> std::result::Result<ImmutablePutOutcome, String> {
        self.put_many(vec![(key.clone(), Bytes::copy_from_slice(bytes))])?
            .pop()
            .ok_or_else(|| Error::Invalid.into())
    }

    fn put_many(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
    ) -> std::result::Result<Vec<ImmutablePutOutcome>, String> {
        self.put_many_with_progress(objects, &mut |_| {})
    }

    /// The whole operation at once: each folder is resolved and listed once,
    /// however many transfer windows its objects would fill elsewhere.
    fn put_many_with_progress(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
        on_stored: &mut dyn FnMut(u64),
    ) -> std::result::Result<Vec<ImmutablePutOutcome>, String> {
        self.runtime
            .block_on(self.put_many_async(objects, on_stored))
            .map_err(|error| self.describe(error))
    }

    fn put_record(
        &self,
        key: &ObjectKey,
        bytes: &[u8],
    ) -> std::result::Result<ImmutablePutOutcome, String> {
        self.runtime
            .block_on(self.upload_record(key, Bytes::copy_from_slice(bytes)))
            .map(|_| ImmutablePutOutcome::Written)
            .map_err(|error| self.describe(error))
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> std::result::Result<StoreListPage, String> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err(self.describe(Error::Limit));
        }
        if let Some(cursor) = cursor {
            ObjectKey::parse(cursor).map_err(|_| self.describe(Error::Invalid))?;
            if !cursor.starts_with(&format!("{}/", prefix.as_str())) {
                return Err(self.describe(Error::Invalid));
            }
        }
        self.runtime
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(120),
                    self.enumerate(prefix, MAX_LIST_ENTRIES),
                )
                .await
                .map_err(|_| Error::Limit)?
            })
            .map(|entries| {
                let mut remaining = entries
                    .into_iter()
                    .filter(|entry| cursor.is_none_or(|cursor| entry.key.as_str() > cursor));
                let entries: Vec<_> = remaining.by_ref().take(limit).collect();
                let next_cursor = remaining
                    .next()
                    .and_then(|_| entries.last().map(|entry| entry.key.as_str().to_owned()));
                StoreListPage {
                    entries,
                    next_cursor,
                }
            })
            .map_err(|error| self.describe(error))
    }
}
