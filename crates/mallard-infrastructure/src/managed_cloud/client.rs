//! The Mallard Cloud HTTP client: one reqwest client per opened storage,
//! no redirects, no retries, the bearer token marked sensitive, and every
//! answer classified into a typed error. The token never enters a message;
//! evidence keeps the route literal, the status, the service's fixed error
//! code, and its request ID.
use crate::storage_core::{
    ConnectionEvidence, ConnectionHealth, ConnectionHealthCell, ConnectionState,
};
use bytes::Bytes;
use mallard_domain::{generated_named_id, SecretValue};
use reqwest::{header::HeaderValue, Client, Method, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

const JSON_LIMIT: u64 = 16 * 1024 * 1024;
const ERROR_BODY_LIMIT: u64 = 64 * 1024;

/// The service's fixed error codes (section 7.1). Nothing outside this list
/// is retained from an error body.
pub const SERVICE_ERROR_CODES: [&str; 22] = [
    "authentication_required",
    "forbidden",
    "not_found",
    "invalid_request",
    "manifest_invalid",
    "revision_conflict",
    "head_changed",
    "upload_in_progress",
    "upload_expired",
    "object_too_large",
    "object_integrity_failed",
    "quota_exceeded",
    "release_request_pending",
    "release_request_invalid",
    "owner_required",
    "super_admin_required",
    "invitation_invalid",
    "idempotency_mismatch",
    "repository_removed",
    "storage_managed_account_exists",
    "repository_name_taken",
    "temporarily_unavailable",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The token was rejected: only a new sign-in recovers.
    Auth,
    /// The account or membership lacks the permission.
    Permission,
    NotFound,
    /// The private head moved under a push.
    HeadChanged,
    /// Too many of the account's own uploads of the repository are open.
    UploadInProgress,
    UploadExpired,
    /// The service's revision guard refused a stale request.
    Conflict,
    /// A repository for this bundle already exists for the account.
    AccountExists,
    Quota,
    TooLarge,
    /// The service rejected the request shape or the manifest.
    Invalid,
    Integrity,
    RateLimit,
    /// A read whose request never completed.
    Unreachable,
    /// A request whose outcome is unknown.
    Unknown,
    /// The service answered 5xx.
    Unavailable,
    /// A response exceeded its safety limit or was malformed.
    Limit,
    Local,
}

/// One classified failure. `detail` is the service's own bounded sentence
/// for validation refusals, so a push can say what the validator disliked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub route: &'static str,
    pub status: Option<u16>,
    pub code: Option<&'static str>,
    pub request_id: Option<String>,
    pub detail: Option<String>,
}

impl Error {
    pub fn local(route: &'static str) -> Self {
        Self {
            kind: ErrorKind::Local,
            route,
            status: None,
            code: None,
            request_id: None,
            detail: None,
        }
    }
    fn of(kind: ErrorKind, route: &'static str) -> Self {
        Self {
            kind,
            route,
            status: None,
            code: None,
            request_id: None,
            detail: None,
        }
    }
    pub fn evidence(&self) -> ConnectionEvidence {
        ConnectionEvidence {
            endpoint: self.route,
            http_status: self.status,
            error_tag: self.code,
            request_id: self.request_id.clone(),
        }
    }
    /// The sentence for notices. Fixed per kind; validation refusals carry
    /// the service's own detail.
    pub fn message(&self) -> String {
        let fixed = match self.kind {
            ErrorKind::Auth => "Mallard Cloud rejected the saved sign-in. Reconnect Mallard Cloud in storage settings.",
            ErrorKind::Permission => "Mallard Cloud refused this request. You are not a collaborator of this repository, or the account is suspended.",
            ErrorKind::NotFound => "Mallard Cloud has no such repository or object.",
            ErrorKind::HeadChanged => "The Mallard Cloud snapshot changed since it was compared. Compare again before pushing.",
            ErrorKind::UploadInProgress => "Too many of your pushes of this repository are still open on Mallard Cloud. Wait for them to finish or expire, then push again.",
            ErrorKind::UploadExpired => "The Mallard Cloud upload expired before it was committed. Push again.",
            ErrorKind::Conflict => "Mallard Cloud refused a stale request. Refresh and try again.",
            ErrorKind::AccountExists => "This account already has a Mallard Cloud repository for this project.",
            ErrorKind::Quota => "The Mallard Cloud repository is out of quota.",
            ErrorKind::TooLarge => "An object exceeds the Mallard Cloud size limit of 90 MiB.",
            ErrorKind::Invalid => "Mallard Cloud rejected the request.",
            ErrorKind::Integrity => "Mallard Cloud rejected the object bytes: the digest does not match.",
            ErrorKind::RateLimit => "Mallard Cloud rate limit reached. Wait and try again.",
            ErrorKind::Unreachable => "Mallard Cloud could not be reached. Check the network and try again.",
            ErrorKind::Unknown => "The Mallard Cloud request result is unknown. Refresh remote state before retrying.",
            ErrorKind::Unavailable => "Mallard Cloud is temporarily unavailable. Try again shortly.",
            ErrorKind::Limit => "A Mallard Cloud response exceeded its safety limit or was malformed.",
            ErrorKind::Local => "Mallard could not prepare the Mallard Cloud request.",
        };
        let mut facts = vec![self.route.to_owned()];
        if let Some(status) = self.status {
            facts.push(format!("HTTP {status}"));
        }
        if let Some(code) = self.code {
            facts.push(code.to_owned());
        }
        if let Some(id) = &self.request_id {
            facts.push(format!("request {id}"));
        }
        match &self.detail {
            Some(detail) => format!("{fixed} {detail} ({})", facts.join(", ")),
            None => format!("{fixed} ({})", facts.join(", ")),
        }
    }
}

impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.message()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub github_login: String,
    pub service_role: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Me {
    pub user: User,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DesktopStart {
    pub state: String,
    pub authorize_url: String,
}

#[derive(Deserialize)]
struct DesktopExchangeWire {
    token: String,
    expires_at: u64,
    user: User,
}

pub struct DesktopExchange {
    pub token: Zeroizing<String>,
    pub expires_at: u64,
    pub user: User,
}

impl From<DesktopExchangeWire> for DesktopExchange {
    fn from(wire: DesktopExchangeWire) -> Self {
        Self {
            token: Zeroizing::new(wire.token),
            expires_at: wire.expires_at,
            user: wire.user,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PushedBy {
    pub user_id: String,
    pub github_login: String,
}

/// The head body of section 7.3.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct HeadBody {
    pub scope: String,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub updated_at: u64,
    pub kind: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub private_revision: Option<u64>,
    #[serde(default)]
    pub public_revision: Option<u64>,
    #[serde(default)]
    pub pushed_by: Option<PushedBy>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HeadByBundle {
    pub repository_id: String,
    #[serde(default)]
    pub head: Option<HeadBody>,
}

/// The publication summary a tag row is synthesized from (section 5.3).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TagSummary {
    pub display_name: String,
    pub kind: String,
    pub revision: u64,
    pub updated_at: u64,
    pub resources: u64,
    pub files: u64,
    #[serde(default)]
    pub conversations: u64,
    pub registration_only: bool,
    #[serde(default)]
    pub providers: Vec<String>,
}

/// An upload in flight. `stale` means its pins no longer match the repository,
/// so it can never commit; the store ends its own stale ones once the next
/// upload exists.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OpenUpload {
    pub id: String,
    pub user_id: String,
    pub commit_id: String,
    pub expires_at: u64,
    pub created_at: u64,
    pub stale: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OpenReleaseRequest {
    pub id: String,
    pub state: String,
    pub requested_by: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RepositorySummary {
    pub id: String,
    /// The Owner; the website address is `{owner.github_login}/{name}`.
    pub owner: PushedBy,
    pub name: String,
    pub repository_class: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub repository_fingerprint: Option<String>,
    pub revision: u64,
    pub state: String,
    pub visibility: String,
    pub private_revision: u64,
    pub public_revision: u64,
    /// The caller's membership role; `None` on the public face and for a
    /// contributor (a signed-in account on a public repository).
    #[serde(default)]
    pub role: Option<String>,
    /// Section 4.3: the caller may push, pull the latest push, and request
    /// a release: a collaborator, or anyone signed in on a public
    /// repository. `false` on the public face.
    #[serde(default)]
    pub writer: bool,
    /// The git project's GitHub repository in canonical form, shown to
    /// whoever may read the repository. Its SHA-256 is the fingerprint, so a
    /// non-member can still tell whether the repository is their checkout's.
    #[serde(default)]
    pub github_url: Option<String>,
    #[serde(default)]
    pub private_head: Option<HeadBody>,
    #[serde(default)]
    pub public_head: Option<HeadBody>,
    #[serde(default)]
    pub private_summary: Option<TagSummary>,
    #[serde(default)]
    pub public_summary: Option<TagSummary>,
    #[serde(default)]
    pub open_release_request: Option<OpenReleaseRequest>,
    #[serde(default)]
    pub open_uploads: Vec<OpenUpload>,
}

#[derive(Clone, Debug, Deserialize)]
struct RepositoryList {
    repositories: Vec<RepositorySummary>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Member {
    pub user_id: String,
    pub github_login: String,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize)]
struct MemberList {
    members: Vec<Member>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateRepository {
    pub repository_class: &'static str,
    /// The requested name. The service stores it normalized to GitHub's
    /// character set ("Horror Game" becomes `Horror-Game`) and suffixed
    /// `-2`, `-3`, … while the account already holds it; the summary it
    /// answers with carries the stored `name`.
    pub name: String,
    pub bundle_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InventoryEntry {
    pub sha256: String,
    pub size: u64,
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UploadRequest {
    pub expected_repository_revision: u64,
    pub expected_private_revision: u64,
    pub objects: Vec<InventoryEntry>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct MissingObject {
    pub sha256: String,
    pub size: u64,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UploadCreated {
    pub upload_id: String,
    pub repository_id: String,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub expires_at: u64,
    pub reserved_bytes: u64,
    pub missing: Vec<MissingObject>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CommitResponse {
    pub head: HeadBody,
    pub repository_revision: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseRequest {
    pub id: String,
    pub state: String,
}

/// What a streamed manifest or object answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Downloaded {
    pub size: u64,
    pub sha256: String,
    pub etag: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
    #[serde(default)]
    detail: String,
}

pub fn idempotency_key() -> Result<String> {
    generated_named_id("idem").map_err(|_| Error::local("idempotency"))
}

/// Validate a service origin: an `http(s)` origin with a host, no
/// credentials, no path, query, or fragment.
pub fn parse_origin(origin: &str) -> std::result::Result<url::Url, String> {
    let url = url::Url::parse(origin).map_err(|_| "service origin is not a URL".to_string())?;
    if !(url.scheme() == "https" || url.scheme() == "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || origin.ends_with('/')
        || origin.len() > 256
    {
        return Err("service origin must be a bare https origin".to_string());
    }
    Ok(url)
}

struct Inner {
    client: Client,
    origin: String,
    token: Option<SecretValue>,
    health: ConnectionHealthCell,
}

#[derive(Clone)]
pub struct ManagedCloudClient {
    inner: Arc<Inner>,
}

impl ManagedCloudClient {
    pub fn new(origin: &str, token: Option<SecretValue>) -> std::result::Result<Self, String> {
        parse_origin(origin)?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|_| "Mallard could not start the Mallard Cloud connection.".to_string())?;
        Ok(Self {
            inner: Arc::new(Inner {
                client,
                origin: origin.to_owned(),
                token,
                health: ConnectionHealthCell::default(),
            }),
        })
    }

    pub fn origin(&self) -> &str {
        &self.inner.origin
    }

    pub fn health(&self) -> Option<ConnectionHealth> {
        self.inner.health.get()
    }

    fn request(&self, method: Method, route: &'static str, path: &str) -> Result<RequestBuilder> {
        let mut request = self
            .inner
            .client
            .request(method, format!("{}{}", self.inner.origin, path));
        if let Some(token) = &self.inner.token {
            let authorization = Zeroizing::new(format!("Bearer {}", token.expose()));
            let mut header =
                HeaderValue::from_str(&authorization).map_err(|_| Error::local(route))?;
            header.set_sensitive(true);
            request = request.header("Authorization", header);
        }
        Ok(request)
    }

    /// Send one request and classify the answer. Transport failures on a
    /// read are `Unreachable`; on a mutation the outcome stays `Unknown`.
    async fn send(
        &self,
        route: &'static str,
        read: bool,
        request: RequestBuilder,
    ) -> Result<Response> {
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => {
                self.inner.health.record(ConnectionHealth::failed(
                    ConnectionState::Unreachable,
                    "managed_cloud_unreachable",
                    "Mallard Cloud could not be reached.",
                    Some(ConnectionEvidence {
                        endpoint: route,
                        http_status: None,
                        error_tag: None,
                        request_id: None,
                    }),
                ));
                return Err(Error::of(
                    if read {
                        ErrorKind::Unreachable
                    } else {
                        ErrorKind::Unknown
                    },
                    route,
                ));
            }
        };
        if response.headers().len() > 128 {
            return Err(Error::of(ErrorKind::Limit, route));
        }
        let status = response.status();
        if status.is_success() {
            self.inner.health.record_connected();
            return Ok(response);
        }
        let request_id = ConnectionEvidence::request_id(
            response
                .headers()
                .get("x-request-id")
                .map(HeaderValue::as_bytes),
        );
        let bytes = bounded(response, ERROR_BODY_LIMIT)
            .await
            .unwrap_or_default();
        let body: Option<ErrorBody> = serde_json::from_slice(&bytes).ok();
        let code = body.as_ref().and_then(|body| {
            SERVICE_ERROR_CODES
                .into_iter()
                .find(|known| *known == body.error)
        });
        let detail = body
            .filter(|body| !body.detail.is_empty() && body.detail.len() <= 512)
            .map(|body| body.detail);
        let kind = match (status, code) {
            (StatusCode::UNAUTHORIZED, _) => ErrorKind::Auth,
            (StatusCode::FORBIDDEN, _) => ErrorKind::Permission,
            (StatusCode::NOT_FOUND, _) => ErrorKind::NotFound,
            (_, Some("head_changed")) => ErrorKind::HeadChanged,
            (_, Some("upload_in_progress")) => ErrorKind::UploadInProgress,
            (_, Some("upload_expired")) => ErrorKind::UploadExpired,
            (_, Some("storage_managed_account_exists")) => ErrorKind::AccountExists,
            (_, Some("quota_exceeded")) => ErrorKind::Quota,
            (_, Some("object_too_large")) => ErrorKind::TooLarge,
            (_, Some("object_integrity_failed")) => ErrorKind::Integrity,
            (
                _,
                Some(
                    "revision_conflict"
                    | "idempotency_mismatch"
                    | "release_request_pending"
                    | "release_request_invalid"
                    | "repository_name_taken",
                ),
            ) => ErrorKind::Conflict,
            (StatusCode::CONFLICT, _) => ErrorKind::Conflict,
            (StatusCode::GONE, _) => ErrorKind::NotFound,
            (StatusCode::TOO_MANY_REQUESTS, _) => ErrorKind::RateLimit,
            (status, _) if status.is_server_error() => ErrorKind::Unavailable,
            (status, _) if status.is_client_error() => ErrorKind::Invalid,
            _ => ErrorKind::Unknown,
        };
        let error = Error {
            kind,
            route,
            status: Some(status.as_u16()),
            code,
            request_id,
            detail: matches!(
                kind,
                ErrorKind::Invalid | ErrorKind::Integrity | ErrorKind::Conflict
            )
            .then_some(detail)
            .flatten(),
        };
        match kind {
            ErrorKind::Auth => {
                self.inner.health.record(ConnectionHealth::failed(
                    ConnectionState::ReconnectRequired,
                    "managed_cloud_reconnect_required",
                    "Mallard Cloud rejected the saved sign-in. Reconnect Mallard Cloud to restore access.",
                    Some(error.evidence()),
                ));
            }
            ErrorKind::Permission if route == "v1/me" => {
                self.inner.health.record(ConnectionHealth::failed(
                    ConnectionState::PermissionDenied,
                    "managed_cloud_permission_denied",
                    "Mallard Cloud refused the account. It may be suspended.",
                    Some(error.evidence()),
                ));
            }
            _ => {}
        }
        Err(error)
    }

    async fn json<T: for<'de> Deserialize<'de>>(
        &self,
        route: &'static str,
        read: bool,
        request: RequestBuilder,
    ) -> Result<T> {
        let response = self.send(route, read, request).await?;
        let bytes = bounded(response, JSON_LIMIT)
            .await
            .map_err(|_| Error::of(ErrorKind::Limit, route))?;
        serde_json::from_slice(&bytes).map_err(|_| Error::of(ErrorKind::Limit, route))
    }

    pub async fn desktop_start(
        &self,
        code_challenge: &str,
        loopback_port: u16,
    ) -> Result<DesktopStart> {
        let route = "v1/auth/desktop/start";
        let request = self
            .request(Method::POST, route, "/v1/auth/desktop/start")?
            .json(&serde_json::json!({"code_challenge": code_challenge, "loopback_port": loopback_port}));
        self.json(route, false, request).await
    }

    pub async fn desktop_exchange(
        &self,
        state: &str,
        grant: &str,
        code_verifier: &str,
    ) -> Result<DesktopExchange> {
        let route = "v1/auth/desktop/exchange";
        let request = self
            .request(Method::POST, route, "/v1/auth/desktop/exchange")?
            .json(&serde_json::json!({"state": state, "grant": grant, "code_verifier": code_verifier}));
        let wire: DesktopExchangeWire = self.json(route, false, request).await?;
        Ok(wire.into())
    }

    pub async fn me(&self) -> Result<Me> {
        let route = "v1/me";
        let request = self.request(Method::GET, route, "/v1/me")?;
        self.json(route, true, request).await
    }

    /// `None` when the bundle is unknown to the caller (204).
    pub async fn head_by_bundle(&self, bundle_id: &str) -> Result<Option<HeadByBundle>> {
        let route = "v1/repositories/by-bundle/head";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/by-bundle/{bundle_id}/head"),
        )?;
        let response = self.send(route, true, request).await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let bytes = bounded(response, JSON_LIMIT)
            .await
            .map_err(|_| Error::of(ErrorKind::Limit, route))?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| Error::of(ErrorKind::Limit, route))
    }

    pub async fn list_project_repositories(&self) -> Result<Vec<RepositorySummary>> {
        let route = "v1/repositories";
        let request = self.request(Method::GET, route, "/v1/repositories?class=project")?;
        let list: RepositoryList = self.json(route, true, request).await?;
        Ok(list.repositories)
    }

    /// The repository at an address (section 4.1): the collaborator summary,
    /// or the public face (`role: None`, fingerprint withheld) of a public
    /// repository the caller does not belong to. `None` when nothing the
    /// caller may see lives there.
    pub async fn repository_by_name(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<RepositorySummary>> {
        let route = "v1/repositories/by-name/{owner}/{name}";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/by-name/{owner}/{name}"),
        )?;
        match self.json(route, true, request).await {
            Ok(summary) => Ok(Some(summary)),
            Err(error) if error.kind == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `POST /v1/invitations/accept`: enter a private repository's one
    /// invitation code. The account joins it as a Member (or already
    /// collaborates) and its summary comes back. An unknown or revoked code
    /// answers `invitation_invalid`, which arrives as `ErrorKind::Invalid`
    /// with the service's sentence as the detail. Idempotent on the service,
    /// so it retries like a read.
    pub async fn accept_invitation(&self, code: &str) -> Result<RepositorySummary> {
        let route = "v1/invitations/accept";
        let request = self
            .request(Method::POST, route, "/v1/invitations/accept")?
            .json(&serde_json::json!({"code": code}));
        self.json(route, true, request).await
    }

    pub async fn get_repository(&self, repository_id: &str) -> Result<RepositorySummary> {
        let route = "v1/repositories/{id}";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/{repository_id}"),
        )?;
        self.json(route, true, request).await
    }

    pub async fn list_members(&self, repository_id: &str) -> Result<Vec<Member>> {
        let route = "v1/repositories/{id}/members";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/{repository_id}/members"),
        )?;
        let list: MemberList = self.json(route, true, request).await?;
        Ok(list.members)
    }

    pub async fn create_repository(&self, spec: &CreateRepository) -> Result<RepositorySummary> {
        let route = "v1/repositories";
        let request = self
            .request(Method::POST, route, "/v1/repositories")?
            .header("Idempotency-Key", idempotency_key()?)
            .json(spec);
        self.json(route, false, request).await
    }

    /// Section 6.5 step 1: the multipart create with the inventory and the
    /// raw manifest bytes.
    pub async fn create_upload(
        &self,
        repository_id: &str,
        upload: &UploadRequest,
        manifest: Bytes,
    ) -> Result<UploadCreated> {
        let route = "v1/repositories/{id}/uploads";
        let request_json = serde_json::to_string(upload).map_err(|_| Error::local(route))?;
        let form = reqwest::multipart::Form::new()
            .text("request", request_json)
            .part(
                "manifest",
                reqwest::multipart::Part::stream(manifest)
                    .file_name("manifest.json")
                    .mime_str("application/json")
                    .map_err(|_| Error::local(route))?,
            );
        let request = self
            .request(
                Method::POST,
                route,
                &format!("/v1/repositories/{repository_id}/uploads"),
            )?
            .header("Idempotency-Key", idempotency_key()?)
            .multipart(form);
        self.json(route, false, request).await
    }

    /// Section 6.5 step 2. The service verifies the digest against the bytes.
    pub async fn put_object(
        &self,
        repository_id: &str,
        upload_id: &str,
        sha256: &str,
        bytes: Bytes,
    ) -> Result<()> {
        let route = "v1/repositories/{id}/uploads/{upload}/objects";
        let request = self
            .request(
                Method::PUT,
                route,
                &format!("/v1/repositories/{repository_id}/uploads/{upload_id}/objects/{sha256}"),
            )?
            .header("Content-Length", bytes.len())
            .header("X-Mallard-Sha256", sha256)
            .header("Content-Type", "application/octet-stream")
            .body(bytes);
        self.send(route, false, request).await?;
        Ok(())
    }

    /// Section 6.5 step 3. `expected_commit_id` names the private head the
    /// push was compared against; `None` expects no private head.
    pub async fn commit_upload(
        &self,
        repository_id: &str,
        upload_id: &str,
        expected_commit_id: Option<&str>,
    ) -> Result<CommitResponse> {
        let route = "v1/repositories/{id}/uploads/{upload}/commit";
        let request = self
            .request(
                Method::POST,
                route,
                &format!("/v1/repositories/{repository_id}/uploads/{upload_id}/commit"),
            )?
            .header("Idempotency-Key", idempotency_key()?)
            .json(&serde_json::json!({"expected_commit_id": expected_commit_id}));
        self.json(route, false, request).await
    }

    pub async fn abandon_upload(&self, repository_id: &str, upload_id: &str) -> Result<()> {
        let route = "v1/repositories/{id}/uploads/{upload}/abandon";
        let request = self.request(
            Method::POST,
            route,
            &format!("/v1/repositories/{repository_id}/uploads/{upload_id}/abandon"),
        )?;
        self.send(route, false, request).await?;
        Ok(())
    }

    pub async fn create_release_request(
        &self,
        repository_id: &str,
        expected_repository_revision: u64,
    ) -> Result<ReleaseRequest> {
        let route = "v1/repositories/{id}/release-requests";
        let request = self
            .request(
                Method::POST,
                route,
                &format!("/v1/repositories/{repository_id}/release-requests"),
            )?
            .header("Idempotency-Key", idempotency_key()?)
            .json(
                &serde_json::json!({"expected_repository_revision": expected_repository_revision}),
            );
        self.json(route, false, request).await
    }

    /// Stream one manifest or object into `sink`, verifying nothing but the
    /// declared size bound; the caller checks the digest. `None` on 404.
    pub async fn download(
        &self,
        route: &'static str,
        path: &str,
        max: u64,
        sink: &mut impl Write,
    ) -> Result<Option<Downloaded>> {
        let request = self.request(Method::GET, route, path)?;
        let mut response = match self.send(route, true, request).await {
            Ok(response) => response,
            Err(error) if error.kind == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if response.content_length().is_some_and(|size| size > max) {
            return Err(Error::of(ErrorKind::Limit, route));
        }
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let declared = header("x-mallard-sha256");
        let etag = header("etag").unwrap_or_default();
        let mut received = 0u64;
        let mut hash = <sha2::Sha256 as sha2::Digest>::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::of(ErrorKind::Unreachable, route))?
        {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| Error::of(ErrorKind::Limit, route))?;
            if received > max {
                return Err(Error::of(ErrorKind::Limit, route));
            }
            sha2::Digest::update(&mut hash, &chunk);
            sink.write_all(&chunk).map_err(|_| Error::local(route))?;
        }
        let sha256 = mallard_domain::hex_lower(&sha2::Digest::finalize(hash));
        if declared.is_some_and(|declared| declared != sha256) {
            return Err(Error::of(ErrorKind::Integrity, route));
        }
        Ok(Some(Downloaded {
            size: received,
            sha256,
            etag,
        }))
    }
}

async fn bounded(mut response: Response, max: u64) -> std::result::Result<Vec<u8>, ()> {
    if response.content_length().is_some_and(|size| size > max) {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > max {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_bare_http_origins() {
        assert!(parse_origin("https://mallard-ai.com").is_ok());
        assert!(parse_origin("http://localhost:8787").is_ok());
        for bad in [
            "https://mallard-ai.com/",
            "https://mallard-ai.com/v1",
            "https://user:pw@mallard-ai.com",
            "ftp://mallard-ai.com",
            "mallard-ai.com",
            "https://mallard-ai.com?x=1",
        ] {
            assert!(parse_origin(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn messages_carry_evidence_but_never_the_token() {
        let error = Error {
            kind: ErrorKind::HeadChanged,
            route: "v1/repositories/{id}/uploads/{upload}/commit",
            status: Some(409),
            code: Some("head_changed"),
            request_id: Some("req-1".into()),
            detail: None,
        };
        assert_eq!(
            error.message(),
            "The Mallard Cloud snapshot changed since it was compared. Compare again before pushing. (v1/repositories/{id}/uploads/{upload}/commit, HTTP 409, head_changed, request req-1)"
        );
        assert_eq!(error.evidence().error_tag, Some("head_changed"));
        assert_eq!(idempotency_key().unwrap().len(), "idem-".len() + 24);
    }
}

#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReleaseState {
    /// Registered, nothing pushed yet.
    NotPushed,
    /// A private snapshot nobody has asked to release.
    Private,
    WaitingOwner {
        requested_by: String,
    },
    WaitingSuperAdmin,
    /// The public version is the current private snapshot.
    Public,
    /// A newer private snapshot was pushed since the last release.
    SnapshotChanged,
    /// The latest request on this snapshot was turned down.
    Rejected {
        reason: Option<String>,
    },
}

#[derive(Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManagedCloudLink {
    pub visibility: String,
    pub can_pull: bool,
    pub repository_id: String,
    /// `{origin}/{owner_login}/{name}`: the repository's page on the website.
    #[serde(default)]
    pub website_url: String,
    pub owner_login: String,
    pub name: String,
    pub release: ReleaseState,
    /// The revision a release request must name.
    pub repository_revision: u64,
    /// Whether Request public release is offered: a private snapshot with
    /// no open request that is not already public.
    pub can_request_release: bool,
    /// The account's membership role; `None` for a public repository the
    /// account does not belong to, where it still pushes as a contributor.
    pub role: Option<String>,
    /// Section 4.3: whether Push is accepted: a collaborator, or anyone
    /// signed in on a public repository.
    pub can_push: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedCloudStatusState {
    Ready,
    Deleting,
    Inaccessible,
    SignInRequired,
    Unavailable,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManagedCloudStatus {
    pub repository_id: String,
    pub state: ManagedCloudStatusState,
    pub link: Option<ManagedCloudLink>,
    pub message: Option<String>,
}
impl ManagedCloudStatus {
    pub fn failed(repository_id: &str, error: &Error) -> Self {
        let state = if error.code == Some("repository_removed") {
            ManagedCloudStatusState::Deleting
        } else {
            match error.kind {
                ErrorKind::Auth => ManagedCloudStatusState::SignInRequired,
                ErrorKind::NotFound | ErrorKind::Permission => {
                    ManagedCloudStatusState::Inaccessible
                }
                _ => ManagedCloudStatusState::Unavailable,
            }
        };
        let message = match state {
            ManagedCloudStatusState::Deleting => "This repository is being deleted.".to_string(),
            ManagedCloudStatusState::Inaccessible => {
                "Repository unavailable. It was deleted or you no longer have access.".to_string()
            }
            _ => error.message(),
        };
        Self {
            repository_id: repository_id.to_owned(),
            state,
            link: None,
            message: Some(message),
        }
    }
}
#[derive(Deserialize)]
pub struct RepositoryHead {
    pub repository_id: String,
    pub head: Option<HeadBody>,
    pub link: ManagedCloudLink,
}
impl ManagedCloudClient {
    pub async fn repository_head(&self, repository_id: &str) -> Result<RepositoryHead> {
        let route = "v1/repositories/id/head";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/{repository_id}/head"),
        )?;
        let mut answer: RepositoryHead = self.json(route, true, request).await?;
        if answer.repository_id != repository_id || answer.link.repository_id != repository_id {
            return Err(Error::of(ErrorKind::Limit, route));
        }
        answer.link.website_url = format!(
            "{}/{}/{}",
            self.origin(),
            answer.link.owner_login,
            answer.link.name
        );
        Ok(answer)
    }
}
