//! The Mallard Cloud HTTP client: one reqwest client per opened storage,
//! no redirects, no retries, the bearer token marked sensitive, and every
//! answer classified into a typed error. The token never enters a message;
//! evidence keeps the route literal, the status, the service's fixed error
//! code, and its request ID.
//!
//! The service is a gateway onto one R2 prefix per repository
//! (`STORE_API.md` in the service tree): the store routes carry the
//! desktop's own keys, and the three operations a store needs, put, get and
//! list, are the three requests here. A put is one request with no retry.
use crate::storage_core::{
    ConnectionEvidence, ConnectionHealth, ConnectionHealthCell, ConnectionState,
};
use bytes::Bytes;
use mallard_domain::{generated_named_id, SecretValue};
use reqwest::{header::HeaderValue, Client, Method, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const JSON_LIMIT: u64 = 16 * 1024 * 1024;
const ERROR_BODY_LIMIT: u64 = 64 * 1024;

/// The service's fixed error codes (`STORE_API.md` section 7 and the
/// architecture's section 7.1). Nothing outside this list is retained from
/// an error body.
pub const SERVICE_ERROR_CODES: [&str; 26] = [
    "authentication_required",
    "forbidden",
    "not_found",
    "absent",
    "invalid_request",
    "manifest_invalid",
    "revision_conflict",
    "repository_discarding",
    "already_present",
    "claim_taken",
    "claim_mismatch",
    "key_invalid",
    "object_invalid",
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
    /// No such repository, or none the account may see.
    NotFound,
    /// The repository exists and the key does not: an absent object.
    Absent,
    /// A create-only write of a key that already holds these bytes.
    AlreadyPresent,
    /// A claim key that holds another writer's bytes.
    ClaimTaken,
    /// A record whose parent's claim is absent or holds another record.
    ClaimMismatch,
    /// The service's revision guard refused a stale request.
    Conflict,
    /// A repository for this bundle already exists for the account.
    AccountExists,
    Quota,
    TooLarge,
    /// The service rejected the request shape, the key, or the bytes.
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
/// for a refusal, so a push can say what the validator disliked and a
/// refused action who may take it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub route: &'static str,
    pub status: Option<u16>,
    pub code: Option<&'static str>,
    pub request_id: Option<String>,
    pub detail: Option<String>,
    diagnostics: ErrorDiagnostics,
}

/// Only local transfer facts, validated request IDs, and fixed failure tags
/// may enter the activity log. Never retain URLs, headers, or response bodies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ErrorDiagnostics {
    cf_ray: Option<String>,
    transport: Option<&'static str>,
    service_reason: Option<&'static str>,
    upload: Option<UploadDiagnostics>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UploadDiagnostics {
    kind: &'static str,
    bytes: usize,
    sha256: Option<String>,
    elapsed: Duration,
}

fn upload_kind(key: &str) -> &'static str {
    key.split('/')
        .find_map(|part| match part {
            "_objects" => Some("object"),
            "_views" => Some("view"),
            "_manifests" => Some("manifest"),
            "_claims" => Some("claim"),
            "_commits" => Some("record"),
            "_tag.json" => Some("tag"),
            _ => None,
        })
        .unwrap_or("unknown")
}

/// The deployed service uses these fixed sentences for its three 503 paths.
/// Map exact matches to local tags; arbitrary service text is not diagnostic data.
fn unavailable_reason(code: Option<&str>, detail: &str) -> Option<&'static str> {
    if code != Some("temporarily_unavailable") {
        return None;
    }
    match detail {
        "The bytes could not be stored; retry." => Some("r2_write_failed"),
        "The object is being cleaned up; push again shortly." => Some("object_cleanup_in_progress"),
        "The request could not be completed." => Some("service_request_failed"),
        _ => None,
    }
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
            diagnostics: ErrorDiagnostics::default(),
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
            diagnostics: ErrorDiagnostics::default(),
        }
    }
    /// Whether the service said the repository is gone: deleted, being
    /// deleted, or out of this account's reach, which it answers alike. A
    /// `404` without the service's own code is not that answer.
    pub fn repository_gone(&self) -> bool {
        self.kind == ErrorKind::NotFound
            && matches!(self.code, Some("not_found" | "repository_removed"))
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
            ErrorKind::Permission => "Mallard Cloud refused this request.",
            ErrorKind::NotFound => "Mallard Cloud has no such repository, or you no longer have access to it.",
            ErrorKind::Absent => "Mallard Cloud has no such object.",
            ErrorKind::AlreadyPresent => "Mallard Cloud already holds this object.",
            ErrorKind::ClaimTaken => "Another computer claimed this version on Mallard Cloud.",
            ErrorKind::ClaimMismatch => "Mallard Cloud refused the record: the claim of its parent does not hold it.",
            ErrorKind::Conflict if self.code == Some("repository_discarding") => "Mallard Cloud is discarding this repository. Wait for deletion to finish.",
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
        if let Some(upload) = &self.diagnostics.upload {
            facts.push(format!(
                "method=PUT, object_kind={}, bytes={}, elapsed_ms={}",
                upload.kind,
                upload.bytes,
                upload.elapsed.as_millis()
            ));
            if let Some(sha256) = &upload.sha256 {
                facts.push(format!("sha256={sha256}"));
            }
        }
        if let Some(transport) = self.diagnostics.transport {
            facts.push(format!("transport={transport}"));
        }
        if let Some(reason) = self.diagnostics.service_reason {
            facts.push(format!("service_reason={reason}"));
        }
        if let Some(ray) = &self.diagnostics.cf_ray {
            facts.push(format!("cf_ray={ray}"));
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

/// A head as the index describes it (architecture section 7.3): the facts
/// of the selected record, as the service indexed them. Display only; the
/// store never reads it.
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

/// The publication summary a listing row is synthesized from (section 5.3).
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OpenReleaseRequest {
    pub id: String,
    pub state: String,
    pub requested_by: String,
    pub created_at: u64,
}

/// The coordinate a catalog repository is bound to, with the item's
/// resource id and the item stream the desktop publishes to.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CatalogBinding {
    pub provider: String,
    pub resource_kind: String,
    pub format: String,
    pub format_version: u32,
    pub resource_id: String,
    #[serde(default)]
    pub library_repository_id: Option<String>,
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
    #[serde(default)]
    pub catalog: Option<CatalogBinding>,
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
    /// The Owner's own line about the repository, and GitHub's short
    /// description mirrored for a linked git project.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub github_description: Option<String>,
    #[serde(default)]
    pub star_count: u64,
    /// Whether the caller has starred it.
    #[serde(default)]
    pub viewer_starred: bool,
}

impl RepositorySummary {
    /// The line a row prints: the Owner's own, else GitHub's.
    pub fn blurb(&self) -> Option<String> {
        self.description
            .as_deref()
            .or(self.github_description.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RepositoryList {
    repositories: Vec<RepositorySummary>,
}

/// One row of a listing the website shows too — the account's starred
/// repositories (`GET /v1/me/starred`) or the public ones
/// (`GET /v1/public/repositories`): less than a summary, enough for a
/// row. A pick resolves the address for the summary.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ListingRow {
    pub repository_id: String,
    pub owner_login: String,
    pub name: String,
    /// `{owner_login}/{name}`, the service's spelling.
    pub address: String,
    pub website_url: String,
    pub repository_class: String,
    pub catalog: Option<ListingCatalog>,
    pub visibility: String,
    /// `member` when the account belongs to it (a starred repository of its
    /// own); `None` on a public face.
    pub role: Option<String>,
    pub github_url: Option<String>,
    /// The Owner's line, else GitHub's.
    pub description: Option<String>,
    pub star_count: u64,
    pub viewer_starred: bool,
    /// The version the account reads; 0 before any push.
    pub revision: u64,
    pub updated_at: u64,
}

/// The coordinate a listed catalog repository is bound to.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListingCatalog {
    pub provider: String,
    pub resource_kind: String,
    pub format: String,
    pub format_version: u32,
}

/// One page of a listing: its rows, and the cursor of the next page.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RepositoryListing {
    pub rows: Vec<ListingRow>,
    pub next_cursor: Option<String>,
}

/// The order of the public listing. `Stars` has no pages: the service
/// sorts the whole set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicSort {
    Newest,
    Stars,
}

impl PublicSort {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "newest" => Some(Self::Newest),
            "stars" => Some(Self::Stars),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Stars => "stars",
        }
    }
}

/// What the public listing is asked for: a class, and for a Catalog row
/// its coordinate's provider and kind.
pub struct PublicFilter<'a> {
    pub class: &'a str,
    pub provider: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub sort: PublicSort,
}

#[derive(Deserialize)]
struct ListingHeadWire {
    scope: String,
    revision: u64,
    updated_at: u64,
}

#[derive(Deserialize)]
struct StarredRowWire {
    id: String,
    owner: PushedBy,
    name: String,
    repository_class: String,
    #[serde(default)]
    catalog: Option<ListingCatalog>,
    #[serde(default)]
    github_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    github_description: Option<String>,
    visibility: String,
    #[serde(default)]
    star_count: u64,
    #[serde(default)]
    viewer_starred: bool,
    #[serde(default)]
    head: Option<ListingHeadWire>,
    #[serde(default)]
    summary: Option<serde_json::Value>,
    updated_at: u64,
}

#[derive(Deserialize)]
struct StarredListWire {
    repositories: Vec<StarredRowWire>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct PublicRowWire {
    id: String,
    owner: PushedBy,
    name: String,
    repository_class: String,
    #[serde(default)]
    catalog: Option<ListingCatalog>,
    #[serde(default)]
    github_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    github_description: Option<String>,
    #[serde(default)]
    public_revision: u64,
    #[serde(default)]
    manifest_revision: u64,
    published_at: u64,
    #[serde(default)]
    star_count: u64,
    #[serde(default)]
    viewer_starred: bool,
    #[serde(default)]
    summary: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct PublicListWire {
    repositories: Vec<PublicRowWire>,
    #[serde(default)]
    next_cursor: Option<String>,
}

/// The publication summary's revision, when the service inlined one.
fn summary_revision(summary: Option<&serde_json::Value>) -> Option<u64> {
    summary?.get("revision")?.as_u64()
}

fn blurb_of(description: Option<String>, github_description: Option<String>) -> Option<String> {
    description
        .or(github_description)
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
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

/// The catalog half of a create request: the item's coordinate, its
/// resource id, and the item stream id the desktop derived, which the
/// service derives again and checks.
#[derive(Clone, Debug, Serialize)]
pub struct CreateCatalogItem {
    pub provider: String,
    pub resource_kind: String,
    pub format: String,
    pub format_version: u32,
    pub resource_id: String,
    pub library_repository_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateRepository {
    pub repository_class: &'static str,
    /// The requested name. The service stores it normalized to GitHub's
    /// character set ("Horror Game" becomes `Horror-Game`) and suffixed
    /// `-2`, `-3`, … while the account already holds it; the summary it
    /// answers with carries the stored `name`.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<CreateCatalogItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseRequest {
    pub id: String,
    pub state: String,
}

/// What a discard of the draft answered (`STORE_API.md` section 3).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiscardedDraft {
    pub repository_id: String,
    pub removal_id: String,
    /// `done`, or `deleting` while the service finishes the removal; every
    /// listing selects the kept push either way.
    pub state: String,
    /// The depth of the push the draft returned to: the one the public
    /// version was released from. `None` when no push is kept.
    #[serde(default)]
    pub kept_revision: Option<u64>,
}

/// What a streamed object answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Downloaded {
    pub size: u64,
    pub sha256: String,
    pub etag: String,
}

/// What one create-only write ended in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PutOutcome {
    Written,
    /// The key already holds these bytes.
    AlreadyPresent,
    /// A claim key that holds another writer's bytes; the caller reads it.
    ClaimTaken,
}

/// One page of a store listing. The stream form carries the repository's
/// status beside the keys (`STORE_API.md` section 3).
#[derive(Clone, Debug, Deserialize)]
pub struct StoreKeyListing {
    pub keys: Vec<String>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub link: Option<ManagedCloudLink>,
    #[serde(default)]
    pub publication: Option<PublicationStatus>,
}

#[derive(Deserialize)]
struct RepositoryReference {
    repository_id: String,
}

/// The coordinate the Library page asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinateQuery {
    pub provider: String,
    pub resource_kind: String,
    pub format: String,
    pub format_version: u32,
}

/// One item of the caller's, as the coordinate route answers it: the
/// stream's record names, and the selected record and manifest inline.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct LibraryItemAnswer {
    pub repository_id: String,
    pub resource_id: String,
    pub library_repository_id: String,
    pub names: Vec<String>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, deserialize_with = "base64_bytes")]
    pub record: Option<Vec<u8>>,
    #[serde(default, deserialize_with = "base64_bytes")]
    pub manifest: Option<Vec<u8>>,
}

fn base64_bytes<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Vec<u8>>, D::Error> {
    let encoded: Option<String> = Option::deserialize(deserializer)?;
    encoded
        .map(|encoded| {
            aws_smithy_types::base64::decode(&encoded)
                .map_err(|_| serde::de::Error::custom("bytes are not base64"))
        })
        .transpose()
}

#[derive(Deserialize)]
struct LibraryCoordinateAnswer {
    repositories: Vec<LibraryItemAnswer>,
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
            Err(failure) => {
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
                let mut error = Error::of(
                    if read {
                        ErrorKind::Unreachable
                    } else {
                        ErrorKind::Unknown
                    },
                    route,
                );
                error.diagnostics.transport = Some(if failure.is_timeout() {
                    "timeout"
                } else if failure.is_connect() {
                    "connect"
                } else if failure.is_body() {
                    "body"
                } else if failure.is_request() {
                    "request"
                } else {
                    "transport"
                });
                return Err(error);
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
        let cf_ray = ConnectionEvidence::request_id(
            response.headers().get("cf-ray").map(HeaderValue::as_bytes),
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
        let service_reason = body.as_ref().and_then(|body| {
            status
                .is_server_error()
                .then(|| unavailable_reason(code, &body.detail))
                .flatten()
        });
        let detail = body
            .filter(|body| !body.detail.is_empty() && body.detail.len() <= 512)
            .map(|body| body.detail);
        let kind = match (status, code) {
            (StatusCode::UNAUTHORIZED, _) => ErrorKind::Auth,
            (StatusCode::FORBIDDEN, _) => ErrorKind::Permission,
            (_, Some("absent")) => ErrorKind::Absent,
            (StatusCode::NOT_FOUND, _) => ErrorKind::NotFound,
            (_, Some("already_present")) => ErrorKind::AlreadyPresent,
            (_, Some("claim_taken")) => ErrorKind::ClaimTaken,
            (_, Some("claim_mismatch")) => ErrorKind::ClaimMismatch,
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
                ErrorKind::Invalid
                    | ErrorKind::Integrity
                    | ErrorKind::Conflict
                    | ErrorKind::ClaimMismatch
                    | ErrorKind::Permission
            )
            .then_some(detail)
            .flatten(),
            diagnostics: ErrorDiagnostics {
                cf_ray,
                service_reason,
                ..ErrorDiagnostics::default()
            },
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

    /// A read that answers `204` when there is nothing.
    async fn optional_json<T: for<'de> Deserialize<'de>>(
        &self,
        route: &'static str,
        request: RequestBuilder,
    ) -> Result<Option<T>> {
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

    /// The repository the caller may push to under a bundle: their own
    /// first, else an open public one. `None` when the bundle is unknown
    /// to the caller (204).
    pub async fn repository_by_bundle(&self, bundle_id: &str) -> Result<Option<String>> {
        let route = "v1/repositories/by-bundle";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/by-bundle/{bundle_id}"),
        )?;
        Ok(self
            .optional_json::<RepositoryReference>(route, request)
            .await?
            .map(|reference| reference.repository_id))
    }

    /// The same for a catalog repository, by its item stream id.
    pub async fn repository_by_library(
        &self,
        library_repository_id: &str,
    ) -> Result<Option<String>> {
        let route = "v1/repositories/by-library";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/by-library/{library_repository_id}"),
        )?;
        Ok(self
            .optional_json::<RepositoryReference>(route, request)
            .await?
            .map(|reference| reference.repository_id))
    }

    /// The caller's catalog repositories of one coordinate, each with its
    /// item stream's names and, inline, the selected record and manifest:
    /// the one request a Library refresh makes.
    pub async fn library_coordinate(
        &self,
        query: &CoordinateQuery,
    ) -> Result<Vec<LibraryItemAnswer>> {
        let route = "v1/repositories/library";
        let path = format!(
            "/v1/repositories/library?{}",
            query_string(&[
                ("provider", query.provider.as_str()),
                ("resource_kind", query.resource_kind.as_str()),
                ("format", query.format.as_str()),
                ("format_version", &query.format_version.to_string()),
            ])
        );
        let request = self.request(Method::GET, route, &path)?;
        let answer: LibraryCoordinateAnswer = self.json(route, true, request).await?;
        Ok(answer.repositories)
    }

    pub async fn list_repositories(&self, class: &str) -> Result<Vec<RepositorySummary>> {
        let route = "v1/repositories";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories?{}", query_string(&[("class", class)])),
        )?;
        let list: RepositoryList = self.json(route, true, request).await?;
        Ok(list.repositories)
    }

    /// `GET /v1/me/starred?cursor=`: the repositories the account starred
    /// that it may still read, newest star first, 50 a page. Every class
    /// comes back; the caller keeps the ones its picker lists.
    pub async fn list_starred(&self, cursor: Option<&str>) -> Result<RepositoryListing> {
        let route = "v1/me/starred";
        let path = match cursor {
            Some(cursor) => format!("/v1/me/starred?{}", query_string(&[("cursor", cursor)])),
            None => "/v1/me/starred".to_owned(),
        };
        let request = self.request(Method::GET, route, &path)?;
        let list: StarredListWire = self.json(route, true, request).await?;
        let rows = list
            .repositories
            .into_iter()
            .map(|row| {
                let member = row
                    .head
                    .as_ref()
                    .is_some_and(|head| head.scope == "private");
                let revision = summary_revision(row.summary.as_ref())
                    .or(row.head.as_ref().map(|head| head.revision))
                    .unwrap_or(0);
                let updated_at = row
                    .head
                    .as_ref()
                    .map(|head| head.updated_at)
                    .unwrap_or(row.updated_at);
                ListingRow {
                    address: format!("{}/{}", row.owner.github_login, row.name),
                    website_url: self.website_url(&row.owner.github_login, &row.name),
                    repository_id: row.id,
                    owner_login: row.owner.github_login,
                    name: row.name,
                    repository_class: row.repository_class,
                    catalog: row.catalog,
                    visibility: row.visibility,
                    role: member.then(|| "member".to_owned()),
                    github_url: row.github_url,
                    description: blurb_of(row.description, row.github_description),
                    star_count: row.star_count,
                    viewer_starred: row.viewer_starred,
                    revision,
                    updated_at,
                }
            })
            .collect();
        Ok(RepositoryListing {
            rows,
            next_cursor: list.next_cursor,
        })
    }

    /// `GET /v1/public/repositories`: the released repositories of one
    /// class, newest first or by stars, 50 a page.
    pub async fn list_public(
        &self,
        filter: &PublicFilter<'_>,
        cursor: Option<&str>,
    ) -> Result<RepositoryListing> {
        let route = "v1/public/repositories";
        let mut pairs: Vec<(&str, &str)> =
            vec![("class", filter.class), ("sort", filter.sort.as_str())];
        if let Some(provider) = filter.provider {
            pairs.push(("provider", provider));
        }
        if let Some(kind) = filter.kind {
            pairs.push(("kind", kind));
        }
        if let Some(cursor) = cursor {
            pairs.push(("cursor", cursor));
        }
        let path = format!("/v1/public/repositories?{}", query_string(&pairs));
        let request = self.request(Method::GET, route, &path)?;
        let list: PublicListWire = self.json(route, true, request).await?;
        let rows = list
            .repositories
            .into_iter()
            .map(|row| {
                let revision = summary_revision(row.summary.as_ref()).unwrap_or(
                    if row.manifest_revision > 0 {
                        row.manifest_revision
                    } else {
                        row.public_revision
                    },
                );
                ListingRow {
                    address: format!("{}/{}", row.owner.github_login, row.name),
                    website_url: self.website_url(&row.owner.github_login, &row.name),
                    repository_id: row.id,
                    owner_login: row.owner.github_login,
                    name: row.name,
                    repository_class: row.repository_class,
                    catalog: row.catalog,
                    visibility: "public".to_owned(),
                    role: None,
                    github_url: row.github_url,
                    description: blurb_of(row.description, row.github_description),
                    star_count: row.star_count,
                    viewer_starred: row.viewer_starred,
                    revision,
                    updated_at: row.published_at,
                }
            })
            .collect();
        Ok(RepositoryListing {
            rows,
            next_cursor: list.next_cursor,
        })
    }

    /// `PUT /v1/repositories/{id}/catalog-addition`: put a public Catalog
    /// item on the account's Catalog, so the Library listing carries it on
    /// every computer. Idempotent. A collaborator's item is listed already,
    /// which the service answers `already_present`; that is not a failure.
    pub async fn add_catalog_item(&self, repository_id: &str) -> Result<()> {
        let route = "v1/repositories/{id}/catalog-addition";
        let request = self.request(
            Method::PUT,
            route,
            &format!("/v1/repositories/{repository_id}/catalog-addition"),
        )?;
        match self.send(route, false, request).await {
            Ok(_) => Ok(()),
            Err(error) if error.kind == ErrorKind::AlreadyPresent => Ok(()),
            Err(error) => Err(error),
        }
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

    /// One create-only write of a desktop key under the repository's
    /// prefix: one request, no retry. `sha256` is the digest of `bytes`.
    pub async fn put_object(
        &self,
        repository_id: &str,
        key: &str,
        bytes: Bytes,
        sha256: &str,
        repository_revision: u64,
    ) -> Result<PutOutcome> {
        let route = "v1/repositories/{id}/store/{key}";
        let content_type = if key.ends_with(".json") {
            "application/json"
        } else {
            "application/octet-stream"
        };
        let size = bytes.len();
        let request = self
            .request(
                Method::PUT,
                route,
                &format!("/v1/repositories/{repository_id}/store/{key}"),
            )?
            .header("Content-Length", bytes.len())
            .header("X-Mallard-Sha256", sha256)
            .header("X-Mallard-Repository-Revision", repository_revision)
            .header("Content-Type", content_type)
            .body(bytes);
        let started = Instant::now();
        match self.send(route, false, request).await {
            Ok(_) => Ok(PutOutcome::Written),
            Err(error) if error.kind == ErrorKind::AlreadyPresent => Ok(PutOutcome::AlreadyPresent),
            Err(error) if error.kind == ErrorKind::ClaimTaken => Ok(PutOutcome::ClaimTaken),
            Err(mut error) => {
                error.diagnostics.upload = Some(UploadDiagnostics {
                    kind: upload_kind(key),
                    bytes: size,
                    sha256: mallard_domain::validate_sha256("upload", sha256)
                        .is_ok()
                        .then(|| sha256.to_owned()),
                    elapsed: started.elapsed(),
                });
                Err(error)
            }
        }
    }

    /// Stream one key's bytes into `sink`, verifying nothing but the
    /// declared size bound; the caller checks the digest. `None` when the
    /// key is absent.
    pub async fn get_object(
        &self,
        repository_id: &str,
        key: &str,
        max: u64,
        sink: &mut impl Write,
    ) -> Result<Option<Downloaded>> {
        let route = "v1/repositories/{id}/store/{key}";
        let request = self.request(
            Method::GET,
            route,
            &format!("/v1/repositories/{repository_id}/store/{key}"),
        )?;
        let mut response = match self.send(route, true, request).await {
            Ok(response) => response,
            Err(error) if error.kind == ErrorKind::Absent => return Ok(None),
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

    /// One page of the keys under a desktop prefix, from one R2 listing.
    pub async fn list_keys(
        &self,
        repository_id: &str,
        prefix: &str,
        cursor: Option<&str>,
    ) -> Result<StoreKeyListing> {
        let route = "v1/repositories/{id}/store";
        let mut query = vec![("prefix", prefix)];
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor));
        }
        let request = self.request(
            Method::GET,
            route,
            &format!(
                "/v1/repositories/{repository_id}/store?{}",
                query_string(&query)
            ),
        )?;
        let mut listing: StoreKeyListing = self.json(route, true, request).await?;
        if listing.keys.len() > 1_000 {
            return Err(Error::of(ErrorKind::Limit, route));
        }
        if let Some(link) = listing.link.as_mut() {
            if link.repository_id != repository_id {
                return Err(Error::of(ErrorKind::Limit, route));
            }
            link.website_url = self.website_url(&link.owner_login, &link.name);
        }
        Ok(listing)
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

    /// Discard the draft, naming the repository revision the status showed:
    /// on a public repository every push after the public version is removed
    /// from Mallard Cloud. One request, not a read, never sent again. A
    /// refusal carries the service's sentence: `forbidden` for anyone but
    /// the Owner and Admins, `revision_conflict` for a stale revision,
    /// `invalid_request` when there is no draft to discard.
    pub async fn discard_private_snapshot(
        &self,
        repository_id: &str,
        expected_repository_revision: u64,
    ) -> Result<DiscardedDraft> {
        let route = "v1/repositories/{id}/private-snapshot/discard";
        let request = self
            .request(
                Method::POST,
                route,
                &format!("/v1/repositories/{repository_id}/private-snapshot/discard"),
            )?
            .json(&serde_json::json!({
                "confirm": true,
                "expected_repository_revision": expected_repository_revision,
            }));
        self.json(route, false, request).await
    }

    fn website_url(&self, owner_login: &str, name: &str) -> String {
        format!("{}/{}/{}", self.origin(), owner_login, name)
    }

    /// The link a summary describes: what the stream listing carries,
    /// read from the index instead. A release the Owner turned down is
    /// not on the summary, so it reads as `private` here.
    pub fn link_from_summary(&self, summary: &RepositorySummary) -> ManagedCloudLink {
        let registration_only = summary
            .private_summary
            .as_ref()
            .is_some_and(|tag| tag.registration_only);
        let release = match (&summary.private_head, &summary.open_release_request) {
            (None, _) => ReleaseState::NotPushed,
            (Some(_), _) if registration_only => ReleaseState::NotPushed,
            (Some(_), Some(request)) if request.state == "pending_super_admin" => {
                ReleaseState::WaitingSuperAdmin
            }
            (Some(_), Some(request)) if request.state == "pending_owner" => {
                ReleaseState::WaitingOwner {
                    requested_by: request.requested_by.clone(),
                }
            }
            (Some(private), _) if summary.visibility == "public" => {
                if summary
                    .public_head
                    .as_ref()
                    .is_some_and(|public| public.commit_id == private.commit_id)
                {
                    ReleaseState::Public
                } else {
                    ReleaseState::SnapshotChanged
                }
            }
            (Some(_), _) => ReleaseState::Private,
        };
        let can_request_release = summary.writer
            && matches!(
                release,
                ReleaseState::Private
                    | ReleaseState::SnapshotChanged
                    | ReleaseState::Rejected { .. }
            );
        ManagedCloudLink {
            visibility: summary.visibility.clone(),
            can_pull: true,
            repository_id: summary.id.clone(),
            website_url: self.website_url(&summary.owner.github_login, &summary.name),
            owner_login: summary.owner.github_login.clone(),
            name: summary.name.clone(),
            release,
            repository_revision: summary.revision,
            can_request_release,
            role: summary.role.clone(),
            can_push: summary.writer,
        }
    }
}

fn query_string(pairs: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        serializer.append_pair(name, value);
    }
    serializer.finish()
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
    /// The pinned repository is being deleted. It reads as empty, and a Push
    /// creates it again.
    Deleting,
    /// The pinned repository was deleted, or this account lost access to it:
    /// the service answers both alike. It reads as empty, and a Push creates
    /// it again on this account.
    Gone,
    /// The service refused the account. Push and Pull stay closed.
    Inaccessible,
    SignInRequired,
    Unavailable,
}

/// Where the private snapshot stands against the public one, as the stream
/// listing reports it: the head it describes is the selection over the
/// names just listed. Computed for the current observation only. Never
/// stored in sync metadata.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublicationStatus {
    pub state: PublicationState,
    pub private_revision: Option<u64>,
    pub public_revision: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    NoDraft,
    NotPublic,
    MatchesPublished,
    UnpublishedDraft,
    Unavailable,
}

impl PublicationStatus {
    pub fn unavailable() -> Self {
        Self {
            state: PublicationState::Unavailable,
            private_revision: None,
            public_revision: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManagedCloudStatus {
    pub repository_id: String,
    pub state: ManagedCloudStatusState,
    pub link: Option<ManagedCloudLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication: Option<PublicationStatus>,
    pub message: Option<String>,
}

impl ManagedCloudStatus {
    /// The status a stream listing reports: the link and the publication
    /// state the listing carried, or `Unavailable` when the service left
    /// them out.
    pub fn observed(repository_id: &str, listing: &StoreKeyListing) -> Self {
        Self {
            repository_id: repository_id.to_owned(),
            state: ManagedCloudStatusState::Ready,
            link: listing.link.clone(),
            publication: Some(
                listing
                    .publication
                    .clone()
                    .unwrap_or_else(PublicationStatus::unavailable),
            ),
            message: None,
        }
    }

    /// The status of a repository that did not answer its listing. Only
    /// `not_found` and `repository_removed` say the repository is gone;
    /// every other answer keeps what it is.
    pub fn failed(repository_id: &str, error: &Error) -> Self {
        let state = match error.kind {
            _ if error.repository_gone() && error.code == Some("repository_removed") => {
                ManagedCloudStatusState::Deleting
            }
            _ if error.repository_gone() => ManagedCloudStatusState::Gone,
            ErrorKind::Auth => ManagedCloudStatusState::SignInRequired,
            ErrorKind::Permission => ManagedCloudStatusState::Inaccessible,
            _ => ManagedCloudStatusState::Unavailable,
        };
        let message = match state {
            ManagedCloudStatusState::Deleting => "Push creates it again.".to_string(),
            ManagedCloudStatusState::Gone => {
                "Deleted, or this account lost access. Push creates it again.".to_string()
            }
            ManagedCloudStatusState::Inaccessible => {
                "Mallard Cloud refused this account. It may be suspended.".to_string()
            }
            _ => error.message(),
        };
        Self {
            repository_id: repository_id.to_owned(),
            state,
            link: None,
            publication: None,
            message: Some(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity_log::{
        ActivityLogContext, ActivityLogEntry, ActivityLogQuery, ActivityLogStore, ActivityLogType,
    };
    use http_body_util::{BodyExt, Full};
    use hyper::{body::Incoming, server::conn::http1, service::service_fn};
    use hyper_util::rt::TokioIo;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn upload_response(
        status: u16,
        body: serde_json::Value,
        request_id: &'static str,
        cf_ray: &'static str,
    ) -> (
        ManagedCloudClient,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let count = count.clone();
                let body = Bytes::from(serde_json::to_vec(&body).unwrap());
                tokio::spawn(async move {
                    let service = service_fn(move |request: hyper::Request<Incoming>| {
                        let count = count.clone();
                        let body = body.clone();
                        async move {
                            count.fetch_add(1, Ordering::SeqCst);
                            let _ = request.into_body().collect().await;
                            Ok::<_, std::convert::Infallible>(
                                hyper::Response::builder()
                                    .status(status)
                                    .header("x-request-id", request_id)
                                    .header("cf-ray", cf_ray)
                                    .body(Full::new(body))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        let client = ManagedCloudClient::new(
            &endpoint,
            Some(SecretValue::parse("private-fixture-bearer-token").unwrap()),
        )
        .unwrap();
        (client, requests, server)
    }

    #[tokio::test]
    async fn failed_upload_diagnostics_reach_the_activity_log_without_retry() {
        let (client, requests, server) = upload_response(
            503,
            json!({"error": "temporarily_unavailable", "detail": "The bytes could not be stored; retry."}),
            "req-upload-failed",
            "1234567890abcdef-SJC",
        ).await;
        let bytes = Bytes::from_static(b"private conversation fixture");
        let sha256 = mallard_domain::sha256_hex(&bytes);
        let key = format!(".mallard/v1/repositories/private-bundle/_objects/{sha256}");
        let error = client
            .put_object("private-repository", &key, bytes.clone(), &sha256, 1)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable);
        assert_eq!(error.status, Some(503));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();

        // The production Push command persists this same error conversion.
        let failure: String = error.into();
        let entry = ActivityLogEntry::new(
            ActivityLogType::Push,
            "error",
            "push.failed",
            &format!("Push failed: {failure}"),
            Some("push-fixture".into()),
            ActivityLogContext::default(),
        );
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        store.append(&entry).unwrap();
        let page = store.query(&ActivityLogQuery::default()).unwrap();
        let message = &page.entries[0].message;
        for fact in [
            "HTTP 503",
            "temporarily_unavailable",
            "request req-upload-failed",
            "method=PUT",
            "object_kind=object",
            "elapsed_ms=",
            "service_reason=r2_write_failed",
            "cf_ray=1234567890abcdef-SJC",
        ] {
            assert!(message.contains(fact), "missing {fact}");
        }
        assert!(message.contains(&format!("bytes={}", bytes.len())));
        assert!(message.contains(&format!("sha256={sha256}")));
        for private in [
            "private-fixture-bearer-token",
            "private conversation",
            "private-repository",
            "private-bundle",
            "http://",
            "The bytes could not be stored",
        ] {
            assert!(!message.contains(private), "retained {private}");
        }
    }

    #[tokio::test]
    async fn upload_diagnostics_drop_unknown_service_text_and_invalid_identifiers() {
        let (client, requests, server) = upload_response(
            503,
            json!({"error": "temporarily_unavailable", "detail": "Bearer remote-secret /Users/private/file?token=private-token"}),
            "request/private-token",
            "ray/private-token",
        ).await;
        let error = client
            .put_object(
                "private-repository",
                "_objects/private-key",
                Bytes::from_static(b"private-body"),
                "private-digest",
                1,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable);
        assert!(error.request_id.is_none());
        assert!(error.detail.is_none());
        assert!(error.diagnostics.cf_ray.is_none());
        assert!(error.diagnostics.service_reason.is_none());
        assert!(!error.message().contains("private"));
        assert!(!error.message().contains("remote-secret"));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn duplicate_upload_still_succeeds_without_retry() {
        let (client, requests, server) = upload_response(
            409,
            json!({"error": "already_present", "detail": "The key already holds these bytes."}),
            "req-duplicate",
            "1234567890abcdef-SJC",
        )
        .await;
        let bytes = Bytes::from_static(b"fixture");
        let sha256 = mallard_domain::sha256_hex(&bytes);
        assert_eq!(
            client
                .put_object("repo", "_objects/fixture", bytes, &sha256, 1)
                .await
                .unwrap(),
            PutOutcome::AlreadyPresent
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn upload_transport_failure_keeps_the_unknown_write_outcome() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let client = ManagedCloudClient::new(&endpoint, None).unwrap();
        let bytes = Bytes::from_static(b"fixture");
        let sha256 = mallard_domain::sha256_hex(&bytes);
        let error = client
            .put_object("repo", "_objects/fixture", bytes, &sha256, 1)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unknown);
        assert!(error.status.is_none());
        assert!(error.message().contains("transport=connect"));
        assert!(error.message().contains("object_kind=object"));
        assert!(!error.message().contains(&endpoint));
    }

    #[test]
    fn unavailable_diagnostics_distinguish_service_paths_without_retaining_text() {
        assert_eq!(
            unavailable_reason(
                Some("temporarily_unavailable"),
                "The object is being cleaned up; push again shortly."
            ),
            Some("object_cleanup_in_progress")
        );
        assert_eq!(
            unavailable_reason(
                Some("temporarily_unavailable"),
                "The request could not be completed."
            ),
            Some("service_request_failed")
        );
        assert_eq!(
            unavailable_reason(Some("unknown"), "The bytes could not be stored; retry."),
            None
        );
        assert_eq!(
            unavailable_reason(
                Some("temporarily_unavailable"),
                "The bytes could not be stored; retry. private data"
            ),
            None
        );
    }

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
            kind: ErrorKind::ClaimMismatch,
            route: "v1/repositories/{id}/store/{key}",
            status: Some(409),
            code: Some("claim_mismatch"),
            request_id: Some("req-1".into()),
            detail: None,
            diagnostics: ErrorDiagnostics::default(),
        };
        assert_eq!(
            error.message(),
            "Mallard Cloud refused the record: the claim of its parent does not hold it. (v1/repositories/{id}/store/{key}, HTTP 409, claim_mismatch, request req-1)"
        );
        assert_eq!(error.evidence().error_tag, Some("claim_mismatch"));
        assert_eq!(idempotency_key().unwrap().len(), "idem-".len() + 24);
    }

    #[test]
    fn a_create_request_flattens_the_catalog_half_and_omits_it_for_a_project() {
        let project = CreateRepository {
            repository_class: "non_git_project",
            github_url: None,
            name: "Project".into(),
            bundle_id: Some("a".repeat(32)),
            repository_fingerprint: None,
            catalog: None,
        };
        assert_eq!(
            serde_json::to_value(&project).unwrap(),
            json!({"repository_class": "non_git_project", "name": "Project", "bundle_id": "a".repeat(32)})
        );
        let item = CreateRepository {
            repository_class: "catalog",
            github_url: None,
            name: "my-skill".into(),
            bundle_id: None,
            repository_fingerprint: None,
            catalog: Some(CreateCatalogItem {
                provider: "codex".into(),
                resource_kind: "agent_skill".into(),
                format: "agentskills.io.skill".into(),
                format_version: 1,
                resource_id: format!("resource-{}", "1".repeat(64)),
                library_repository_id: format!("library-{}", "2".repeat(32)),
            }),
        };
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["provider"], "codex");
        assert_eq!(value["format_version"], 1);
        assert_eq!(
            value["library_repository_id"],
            format!("library-{}", "2".repeat(32))
        );
        assert!(value.get("bundle_id").is_none());
    }

    #[test]
    fn a_coordinate_answer_decodes_its_inline_bytes() {
        let answer: LibraryItemAnswer = serde_json::from_value(json!({
            "repository_id": "repo_1", "resource_id": format!("resource-{}", "1".repeat(64)),
            "library_repository_id": format!("library-{}", "2".repeat(32)),
            "names": ["a", "b"], "truncated": false,
            "record": aws_smithy_types::base64::encode(b"record"), "manifest": null
        }))
        .unwrap();
        assert_eq!(answer.record.as_deref(), Some(b"record".as_slice()));
        assert!(answer.manifest.is_none());
        assert!(!answer.truncated);
    }

    fn summary(value: serde_json::Value) -> RepositorySummary {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_link_of_a_summary_reads_release_from_the_heads_and_the_open_request() {
        let client = ManagedCloudClient::new("https://mallard-ai.com", None).unwrap();
        let head = json!({"scope":"private", "revision":4, "commit_id":"commit-draft", "manifest_sha256":"a".repeat(64), "manifest_size":10, "updated_at":5, "kind":"project", "bundle_id":"b".repeat(32)});
        let mut public = head.clone();
        public["scope"] = json!("public");
        public["commit_id"] = json!("commit-public");
        let base = json!({"id":"repo_one", "owner":{"user_id":"user_one", "github_login":"owner"}, "name":"Project", "repository_class":"git_project", "revision":7, "state":"active", "visibility":"public", "private_revision":4, "public_revision":2, "writer":true, "role":"owner", "private_head":head, "public_head":public});
        let link = client.link_from_summary(&summary(base.clone()));
        assert_eq!(link.release, ReleaseState::SnapshotChanged);
        assert!(link.can_request_release);
        assert_eq!(link.website_url, "https://mallard-ai.com/owner/Project");

        let mut same = base.clone();
        same["public_head"]["commit_id"] = json!("commit-draft");
        assert_eq!(
            client.link_from_summary(&summary(same)).release,
            ReleaseState::Public
        );

        let mut waiting = base.clone();
        waiting["open_release_request"] = json!({"id":"rel_1", "state":"pending_owner", "requested_by":"user_two", "created_at":9});
        let link = client.link_from_summary(&summary(waiting));
        assert_eq!(
            link.release,
            ReleaseState::WaitingOwner {
                requested_by: "user_two".into()
            }
        );
        assert!(!link.can_request_release);

        let mut registered = base.clone();
        registered["visibility"] = json!("private");
        registered["public_head"] = json!(null);
        registered["private_summary"] = json!({"display_name":"Project", "kind":"project", "revision":4, "updated_at":5, "resources":0, "files":0, "registration_only":true});
        assert_eq!(
            client.link_from_summary(&summary(registered)).release,
            ReleaseState::NotPushed
        );

        let mut private = base;
        private["visibility"] = json!("private");
        private["public_head"] = json!(null);
        assert_eq!(
            client.link_from_summary(&summary(private)).release,
            ReleaseState::Private
        );
    }

    #[test]
    fn a_status_observed_from_a_listing_carries_its_link_and_publication() {
        let listing: StoreKeyListing = serde_json::from_value(json!({
            "keys": [], "next_cursor": null,
            "link": {"repository_id":"repo_one", "visibility":"private", "can_pull":true, "owner_login":"owner", "name":"Project", "release":{"kind":"private"}, "repository_revision":3, "can_request_release":true, "role":"owner", "can_push":true},
            "publication": {"state":"not_public", "private_revision":2, "public_revision":null}
        }))
        .unwrap();
        let status = ManagedCloudStatus::observed("repo_one", &listing);
        assert_eq!(status.state, ManagedCloudStatusState::Ready);
        assert_eq!(status.link.unwrap().release, ReleaseState::Private);
        assert_eq!(
            status.publication.unwrap().state,
            PublicationState::NotPublic
        );
        let bare: StoreKeyListing = serde_json::from_value(json!({"keys": ["k"]})).unwrap();
        assert_eq!(
            ManagedCloudStatus::observed("repo_one", &bare)
                .publication
                .unwrap()
                .state,
            PublicationState::Unavailable
        );
    }
}
