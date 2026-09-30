//! An in-process stand-in for the Mallard Cloud Worker with the gateway
//! semantics the desktop relies on (`STORE_API.md` in the service tree):
//! bearer sessions, one R2-like key space per repository with create-only
//! writes, the per-class checks, the claim guard on a record, the index
//! kept the way the Worker keeps it, the stream listing that carries the
//! repository's status, by-bundle and by-library resolution, the coordinate
//! answer, releases, removal with its three repository states, discard as
//! the service runs it, and the desktop sign-in dance. State lives in
//! memory; tests reach it directly.
use crate::storage_core::publication::record_name_of;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mallard_domain::{sha256_hex, BundleManifest, Mainline, PublicationToken, RecordName};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

const MAX_OBJECT_BYTES: usize = 90 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const LISTING_PAGE: usize = 1_000;
const INLINE_MANIFEST_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StubRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StubUser {
    pub id: String,
    pub login: String,
}

/// One stored key: the bytes, their digest, and, on a claim, the writer.
#[derive(Clone, Debug)]
pub struct StoredObject {
    pub bytes: Bytes,
    pub sha256: String,
    pub user_id: Option<String>,
}

/// One indexed publication: the record's facts as the Worker indexes them.
#[derive(Clone, Debug)]
pub struct Publication {
    pub record_name: String,
    pub commit_id: String,
    pub manifest_revision: u64,
    pub manifest_key: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub summary: Value,
    pub created_by: String,
    pub created_at: u64,
    pub state: String,
}

/// The repository's one invitation code (section 4.3), kept in clear here.
#[derive(Clone, Debug)]
pub struct InviteCode {
    /// Normalized: upper case, no separators.
    pub code: String,
    pub set_by: String,
    pub set_at: u64,
}

/// The coordinate a catalog repository is bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogItem {
    pub provider: String,
    pub resource_kind: String,
    pub format: String,
    pub format_version: u32,
    pub resource_id: String,
    pub library_repository_id: String,
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub id: String,
    /// `active`, `deleting` from the request that confirms its removal, and
    /// `removed` once the worker has erased its prefix. Only an active
    /// repository stands for its bundle or item, holds its name, or counts.
    pub state: String,
    pub class: String,
    /// The address segment after the Owner's login, as the service stores it.
    pub name: String,
    /// The creator, who is the Owner; the address namespace.
    pub owner_id: String,
    pub bundle_id: Option<String>,
    pub repository_fingerprint: Option<String>,
    pub catalog: Option<CatalogItem>,
    /// `private` until a test releases it.
    pub visibility: String,
    /// The canonical GitHub address the service discloses to every reader.
    pub github_url: Option<String>,
    pub revision: u64,
    pub private_revision: u64,
    pub public_revision: u64,
    /// The record name the index points at as the private head.
    pub private_head: Option<String>,
    /// The commit id of the released publication.
    pub public_commit_id: Option<String>,
    pub publications: Vec<Publication>,
    /// Desktop key → object: the repository's R2 prefix.
    pub objects: BTreeMap<String, StoredObject>,
    /// Object keys counted toward the quota.
    counted: BTreeSet<String>,
    pub committed_bytes: u64,
    /// user id → (login, role)
    pub members: BTreeMap<String, (String, String)>,
    pub release_requests: Vec<Value>,
    pub invite_code: Option<InviteCode>,
}

fn keep_manifest_content(value: &Value, prefix: &str, keys: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(key) = fields.get("object_key").and_then(Value::as_str) {
                keys.insert(format!("{prefix}/{key}"));
            }
            if let Some(sha) = fields.get("transcript_sha256").and_then(Value::as_str) {
                keys.insert(format!("{prefix}/_views/1-{sha}.json"));
            }
            for child in fields.values() {
                keep_manifest_content(child, prefix, keys);
            }
        }
        Value::Array(values) => {
            for child in values {
                keep_manifest_content(child, prefix, keys);
            }
        }
        _ => {}
    }
}

impl Repository {
    fn stream_prefix(&self) -> String {
        match (&self.bundle_id, &self.catalog) {
            (Some(bundle), _) => format!(".mallard/v1/repositories/_commits/{bundle}"),
            (None, Some(item)) => format!(
                ".mallard/v1/library/{}/_commits",
                item.library_repository_id
            ),
            (None, None) => unreachable!("a repository is a project or an item"),
        }
    }

    fn claims_prefix(&self) -> String {
        match (&self.bundle_id, &self.catalog) {
            (Some(bundle), _) => format!(".mallard/v1/repositories/_claims/{bundle}"),
            (None, Some(item)) => {
                format!(".mallard/v1/library/{}/_claims", item.library_repository_id)
            }
            (None, None) => unreachable!("a repository is a project or an item"),
        }
    }

    fn content_prefix(&self) -> String {
        match (&self.bundle_id, &self.catalog) {
            (Some(bundle), _) => format!(".mallard/v1/repositories/{bundle}"),
            (None, Some(item)) => format!(".mallard/v1/library/{}", item.library_repository_id),
            (None, None) => unreachable!("a repository is a project or an item"),
        }
    }

    /// The keys directly or transitively under `prefix`, in key order.
    pub fn keys_under(&self, prefix: &str) -> Vec<String> {
        let directory = format!("{prefix}/");
        self.objects
            .keys()
            .filter(|key| key.starts_with(&directory))
            .cloned()
            .collect()
    }

    /// The names of the record stream, and its selection.
    pub fn stream(&self) -> (Vec<RecordName>, Mainline) {
        let prefix = self.stream_prefix();
        let names = self
            .keys_under(&prefix)
            .iter()
            .filter_map(|key| RecordName::parse(key.rsplit('/').next()?))
            .collect::<Vec<_>>();
        let mainline = Mainline::select(names.iter().cloned());
        (names, mainline)
    }

    pub fn publication(&self, record_name: &str) -> Option<&Publication> {
        self.publications
            .iter()
            .find(|publication| publication.record_name == record_name)
    }

    pub fn head(&self) -> Option<&Publication> {
        self.private_head
            .as_deref()
            .and_then(|name| self.publication(name))
    }

    fn public_head(&self) -> Option<&Publication> {
        let commit = self.public_commit_id.as_deref()?;
        self.publications
            .iter()
            .find(|publication| publication.commit_id == commit)
    }

    fn active(&self) -> bool {
        self.state == "active"
    }

    /// Public with a released head: anyone signed in reads and pushes.
    fn open(&self) -> bool {
        self.active() && self.visibility == "public" && self.public_commit_id.is_some()
    }

    /// How many pushes a discard keeps: on a public repository those up to
    /// the push the public version was released from, on any other none.
    /// `None` when there is no draft to discard.
    fn discard_keeps(&self) -> Option<usize> {
        let (names, mainline) = self.stream();
        if !self.open() {
            return (!names.is_empty()).then_some(0);
        }
        let released = &self.public_head()?.record_name;
        let kept = mainline
            .names()
            .iter()
            .position(|name| &name.file_name() == released)?
            + 1;
        (mainline.names().len() > kept).then_some(kept)
    }
}

struct Attempt {
    challenge: String,
    port: u16,
    grant: Option<String>,
    user_id: Option<String>,
}

/// When an injected failure strikes a write: before the bytes land, or
/// after they landed and before the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Before,
    After,
}

/// How an injected failure answers a store listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingFault {
    /// `503 temporarily_unavailable`, the service's own.
    Unavailable,
    /// A `404` that is not the service's: no error envelope, as a proxy in
    /// front of it would answer.
    ForeignNotFound,
}

#[derive(Default)]
pub struct StubState {
    pub users: BTreeMap<String, StubUser>,
    tokens: BTreeMap<String, String>,
    pub repositories: BTreeMap<String, Repository>,
    attempts: BTreeMap<String, Attempt>,
    pub log: Vec<StubRequest>,
    next: u64,
    /// Fail only the supplemental repository-details read.
    pub fail_next_summary: bool,
    /// The next claim write fails: before or after the claim lands.
    pub fail_next_claim: Option<Fault>,
    /// The next record write fails: before or after the record lands.
    pub fail_next_record: Option<Fault>,
    /// The next store listing fails.
    pub fail_next_listing: Option<ListingFault>,
    /// Accounts the service has suspended: every request of theirs answers
    /// `403 forbidden`.
    pub suspended: BTreeSet<String>,
    /// The next record lands without its index batch: the index lags R2.
    pub skip_next_index: bool,
}

impl StubState {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}_{:016x}", self.next)
    }
    pub fn add_user(&mut self, login: &str) -> (StubUser, String) {
        let user = StubUser {
            id: self.id("user"),
            login: login.to_owned(),
        };
        let token = format!("tok_{}_{}", user.id, "x".repeat(24));
        self.users.insert(user.id.clone(), user.clone());
        self.tokens.insert(token.clone(), user.id.clone());
        (user, token)
    }
    /// One more session of an account: another computer signed in to it.
    pub fn add_session(&mut self, user_id: &str) -> String {
        self.next += 1;
        let token = format!("tok_{user_id}_{:024x}", self.next);
        self.tokens.insert(token.clone(), user_id.to_owned());
        token
    }
    pub fn add_member(&mut self, repository_id: &str, user_id: &str, role: &str) {
        let login = self.users[user_id].login.clone();
        self.repositories
            .get_mut(repository_id)
            .expect("repository")
            .members
            .insert(user_id.to_owned(), (login, role.to_owned()));
    }
    pub fn revoke_token(&mut self, token: &str) {
        self.tokens.remove(token);
    }
    /// Set (or rotate) a repository's invitation code, as its Owner would on
    /// the website. Returns the code in the service's printed shape.
    pub fn set_invite_code(&mut self, repository_id: &str, set_by: &str) -> String {
        self.next += 1;
        let raw = invitation_code_from(self.next);
        let repository = self
            .repositories
            .get_mut(repository_id)
            .expect("repository");
        repository.invite_code = Some(InviteCode {
            code: normalize_invitation_code(&raw),
            set_by: set_by.to_owned(),
            set_at: now(),
        });
        repository.revision += 1;
        raw
    }
    /// Release the private head: the repository turns public, its public
    /// head is the current private one, and `github_url` is what
    /// non-members learn about its Git remote.
    pub fn release(&mut self, repository_id: &str, github_url: Option<&str>) {
        let repository = self
            .repositories
            .get_mut(repository_id)
            .expect("repository");
        repository.visibility = "public".to_owned();
        repository.github_url = github_url.map(str::to_owned);
        if let Some(head) = repository.head().map(|head| head.commit_id.clone()) {
            repository.public_commit_id = Some(head);
            repository.public_revision += 1;
        }
        repository.release_requests.clear();
        repository.revision += 1;
    }
    /// Make a repository private, as its Owner would on the website. Its
    /// released version stays on the row and plays no part from here on.
    pub fn make_private(&mut self, repository_id: &str) {
        let repository = self
            .repositories
            .get_mut(repository_id)
            .expect("repository");
        repository.visibility = "private".to_owned();
        repository.release_requests.clear();
        repository.revision += 1;
    }
    /// Discard the draft, as the service runs it in the request. On a public
    /// repository the draft returns to the public version: the claim of the
    /// push it was released from goes, then every push after it and their
    /// claims. On any other repository every claim and every push goes.
    /// Content used only by removed pushes is deleted. The depth of the push that is
    /// kept, or `None` when nothing is; `Err` when there is no draft.
    pub fn discard(&mut self, repository_id: &str) -> Result<Option<u64>, &'static str> {
        let repository = self
            .repositories
            .get_mut(repository_id)
            .expect("repository");
        let kept = repository
            .discard_keeps()
            .ok_or("There is no draft to discard.")?;
        let (_, mainline) = repository.stream();
        let chain = &mainline.names()[..kept];
        let kept_records = chain
            .iter()
            .map(RecordName::file_name)
            .collect::<BTreeSet<_>>();
        // The claims of the root and of every kept push but the last hold kept records.
        let kept_claims = chain
            .iter()
            .map(|name| {
                name.parent()
                    .map_or_else(|| "root.json".to_owned(), |parent| format!("{parent}.json"))
            })
            .collect::<BTreeSet<_>>();
        let file = |key: &str| key.rsplit('/').next().unwrap_or_default().to_owned();
        for key in repository.keys_under(&repository.claims_prefix()) {
            if !kept_claims.contains(&file(&key)) {
                repository.objects.remove(&key);
            }
        }
        for key in repository.keys_under(&repository.stream_prefix()) {
            if !kept_records.contains(&file(&key)) {
                repository.objects.remove(&key);
            }
        }
        let head = chain.last().map(RecordName::file_name);
        for publication in &mut repository.publications {
            if publication.state == "removed" {
                continue;
            }
            publication.state = if !kept_records.contains(&publication.record_name) {
                "removed"
            } else if Some(&publication.record_name) == head.as_ref() {
                "available"
            } else {
                "retired"
            }
            .to_owned();
        }
        let content = repository.content_prefix();
        let mut protected = BTreeSet::new();
        for publication in &repository.publications {
            if publication.state != "removed"
                || Some(&publication.commit_id) == repository.public_commit_id.as_ref()
            {
                let key = format!("{content}/{}", publication.manifest_key);
                let Some(object) = repository.objects.get(&key) else {
                    continue;
                };
                let value: Value = serde_json::from_slice(&object.bytes).expect("stored manifest");
                protected.insert(key);
                keep_manifest_content(&value, &content, &mut protected);
            }
        }
        for key in repository.keys_under(&content) {
            if (key.contains("/_objects/")
                || key.contains("/_views/")
                || key.contains("/_manifests/"))
                && !protected.contains(&key)
            {
                repository.objects.remove(&key);
                repository.counted.remove(&key);
            }
        }
        repository.committed_bytes = repository
            .counted
            .iter()
            .filter_map(|key| repository.objects.get(key))
            .map(|object| object.bytes.len() as u64)
            .sum();
        repository.private_head = head;
        repository.release_requests.clear();
        repository.revision += 1;
        Ok(chain.last().map(RecordName::revision))
    }
    /// Confirm a repository's removal, as its Owner would on the website: it
    /// is `deleting` and stops counting at once.
    pub fn remove(&mut self, repository_id: &str) {
        let repository = self
            .repositories
            .get_mut(repository_id)
            .expect("repository");
        repository.state = "deleting".to_owned();
        repository.visibility = "private".to_owned();
        repository.invite_code = None;
        repository.release_requests.clear();
        repository.revision += 1;
    }
    /// The removal worker's run: the prefix is erased and the row is `removed`.
    pub fn finish_removal(&mut self, repository_id: &str) {
        let repository = self
            .repositories
            .get_mut(repository_id)
            .expect("repository");
        repository.state = "removed".to_owned();
        repository.objects.clear();
        repository.counted.clear();
        repository.committed_bytes = 0;
        repository.private_head = None;
        repository.public_commit_id = None;
        for publication in &mut repository.publications {
            publication.state = "removed".to_owned();
        }
    }
    /// The active repository of a bundle: the one the service resolves.
    pub fn repository_for_bundle(&self, bundle_id: &str) -> Option<&Repository> {
        self.repositories
            .values()
            .find(|r| r.active() && r.bundle_id.as_deref() == Some(bundle_id))
    }
    pub fn repository_for_library(&self, library_repository_id: &str) -> Option<&Repository> {
        self.repositories.values().find(|r| {
            r.active()
                && r.catalog
                    .as_ref()
                    .is_some_and(|item| item.library_repository_id == library_repository_id)
        })
    }
    /// Move the index's head pointer by hand: what the desktop must never
    /// notice.
    pub fn misplace_head(&mut self, repository_id: &str, record_name: Option<&str>) {
        self.repositories
            .get_mut(repository_id)
            .expect("repository")
            .private_head = record_name.map(str::to_owned);
    }
}

pub struct StubManagedCloud {
    pub endpoint: String,
    pub state: Arc<Mutex<StubState>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for StubManagedCloud {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl StubManagedCloud {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub service");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(StubState::default()));
        let endpoint = format!("http://127.0.0.1:{port}");
        let server = {
            let state = state.clone();
            let endpoint = endpoint.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let state = state.clone();
                    let endpoint = endpoint.clone();
                    tokio::spawn(async move {
                        let service = service_fn(move |request| {
                            handle(request, state.clone(), endpoint.clone())
                        });
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            })
        };
        Self {
            endpoint,
            state,
            server,
        }
    }

    pub fn requests(&self) -> Vec<StubRequest> {
        self.state.lock().unwrap().log.clone()
    }

    /// The requests since the log was last cleared, as `METHOD path`
    /// lines with the listing prefix appended: what a test asserts.
    pub fn request_lines(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|request| match &request.query {
                Some(query) => format!("{} {}?{}", request.method, request.path, query),
                None => format!("{} {}", request.method, request.path),
            })
            .collect()
    }

    pub fn clear_log(&self) {
        self.state.lock().unwrap().log.clear();
    }
}

fn now() -> u64 {
    mallard_domain::now_secs()
}

/// The service's code alphabet has no 0/O/1/I/L/U; a sequence number spelled
/// in its first ten letters is a valid, distinct code.
const CODE_LETTERS: &[u8; 10] = b"ABCDEFGHJK";

fn invitation_code_from(sequence: u64) -> String {
    let raw = format!("{sequence:020}")
        .bytes()
        .map(|digit| CODE_LETTERS[(digit - b'0') as usize] as char)
        .collect::<String>();
    format!(
        "{}-{}-{}-{}",
        &raw[..5],
        &raw[5..10],
        &raw[10..15],
        &raw[15..20]
    )
}

/// The service's `normalizeInvitationCode`: upper case, separators dropped.
fn normalize_invitation_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

struct Reply(Response<Full<Bytes>>);

fn json(status: StatusCode, value: Value) -> Reply {
    Reply(
        Response::builder()
            .status(status)
            .header("content-type", "application/json; charset=utf-8")
            .header("x-request-id", "stub-request")
            .body(Full::new(Bytes::from(value.to_string())))
            .unwrap(),
    )
}

fn error(code: &str, detail: &str) -> Reply {
    let status = match code {
        "authentication_required" => 401,
        "forbidden" | "owner_required" => 403,
        "not_found" | "absent" => 404,
        "invalid_request" | "invitation_invalid" => 400,
        "manifest_invalid" | "object_integrity_failed" | "key_invalid" | "object_invalid" => 422,
        "repository_removed" => 410,
        "object_too_large" | "quota_exceeded" => 413,
        "temporarily_unavailable" => 503,
        _ => 409,
    };
    json(
        StatusCode::from_u16(status).unwrap(),
        json!({"error": code, "detail": detail, "request_id": "stub-request"}),
    )
}

fn no_content() -> Reply {
    Reply(
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header("x-request-id", "stub-request")
            .body(Full::new(Bytes::new()))
            .unwrap(),
    )
}

fn head_body(
    repository: &Repository,
    publication: &Publication,
    scope: &str,
    users: &BTreeMap<String, StubUser>,
) -> Value {
    let mut body = json!({
        "scope": scope,
        "revision": publication.manifest_revision,
        "commit_id": publication.commit_id,
        "manifest_sha256": publication.manifest_sha256,
        "manifest_size": publication.manifest_size,
        "updated_at": publication.created_at,
        "kind": if repository.class == "catalog" { "catalog" } else { "project" },
        "bundle_id": repository.bundle_id,
        "pushed_by": users.get(&publication.created_by).map(|u| json!({"user_id": u.id, "github_login": u.login})),
    });
    if scope == "public" {
        body["public_revision"] = json!(repository.public_revision);
    } else {
        body["private_revision"] = json!(repository.private_revision);
    }
    body
}

const REPOSITORY_NAME_MAX: usize = 100;

/// Preserve valid names exactly; otherwise apply the service's title cleanup.
fn normalize_repository_name(requested: &str) -> String {
    let trimmed = requested.trim();
    if !trimmed.is_empty()
        && trimmed.len() <= REPOSITORY_NAME_MAX
        && trimmed != "."
        && trimmed != ".."
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return trimmed.to_owned();
    }
    let mut out = String::new();
    let mut pending = false;
    for ch in requested.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            if pending && !out.is_empty() {
                out.push('-');
            }
            pending = false;
            out.push(ch);
        } else {
            pending = true;
        }
    }
    out.truncate(REPOSITORY_NAME_MAX);
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() || out == "." || out == ".." {
        "repository".to_owned()
    } else {
        out
    }
}

/// The first of `base`, `base-2`, `base-3`, … not in `held` (lower-cased names).
fn pick_available_name(base: &str, held: &BTreeSet<String>) -> String {
    if !held.contains(&base.to_ascii_lowercase()) {
        return base.to_owned();
    }
    (2u64..)
        .map(|n| {
            let suffix = format!("-{n}");
            let mut head = base[..base.len().min(REPOSITORY_NAME_MAX - suffix.len())].to_owned();
            while head.ends_with('-') {
                head.pop();
            }
            format!("{head}{suffix}")
        })
        .find(|candidate| !held.contains(&candidate.to_ascii_lowercase()))
        .expect("an unbounded sequence has a free name")
}

fn catalog_value(item: &CatalogItem) -> Value {
    json!({
        "provider": item.provider, "resource_kind": item.resource_kind, "format": item.format,
        "format_version": item.format_version, "resource_id": item.resource_id,
        "library_repository_id": item.library_repository_id,
    })
}

/// The summary as a writer reads it (a collaborator, or anyone signed in
/// on a public repository: section 4.3), or, with `writer` false, the public
/// face: fingerprint, private head and open requests withheld.
fn summary(state: &StubState, repository: &Repository, role: Option<&str>, writer: bool) -> Value {
    let member = writer;
    let private_head = repository
        .head()
        .map(|p| head_body(repository, p, "private", &state.users));
    let public_head = repository
        .public_head()
        .map(|p| head_body(repository, p, "public", &state.users));
    let private_tag = repository.head().map(|p| p.summary.clone());
    let public_tag = repository.public_head().map(|p| p.summary.clone());
    let open_request = repository
        .release_requests
        .last()
        .filter(|r| r["state"] == "pending_owner" || r["state"] == "pending_super_admin")
        .cloned();
    json!({
        "id": repository.id,
        "owner": state.users.get(&repository.owner_id).map(|u| json!({"user_id": u.id, "github_login": u.login})),
        "name": repository.name,
        "repository_class": repository.class,
        "bundle_id": repository.bundle_id,
        "repository_fingerprint": if member { repository.repository_fingerprint.clone() } else { None },
        "github_url": repository.github_url,
        "catalog": repository.catalog.as_ref().map(catalog_value),
        "revision": repository.revision,
        "state": repository.state,
        "visibility": repository.visibility,
        "private_revision": repository.private_revision,
        "public_revision": repository.public_revision,
        "committed_bytes": repository.committed_bytes,
        "role": role,
        "writer": writer,
        "private_head": if member { private_head } else { None },
        "public_head": public_head,
        "private_summary": if member { private_tag } else { None },
        "public_summary": public_tag,
        "open_release_request": if member { open_request } else { None },
    })
}

fn query(uri: &hyper::Uri, name: &str) -> Option<String> {
    uri.query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| pct_decode(value))
    })
}

fn pct_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn handle(
    request: Request<Incoming>,
    state: Arc<Mutex<StubState>>,
    endpoint: String,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_owned();
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .map(str::to_owned);
    let idempotency = request
        .headers()
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let declared_sha = request
        .headers()
        .get("x-mallard-sha256")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let content_length = request
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    let repository_revision = request
        .headers()
        .get("x-mallard-repository-revision")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let body = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Ok(error("invalid_request", "incomplete body").0),
    };
    // The sign-in consent page performs the loopback redirect itself,
    // like a browser would; it must not hold the state lock across the
    // request it makes.
    if method == Method::GET && path == "/stub/consent" {
        let state_value = query(&uri, "state").unwrap_or_default();
        let login = query(&uri, "login");
        let target = {
            let mut guard = state.lock().unwrap();
            let user_id = match login {
                Some(login) => guard
                    .users
                    .values()
                    .find(|u| u.login == login)
                    .map(|u| u.id.clone()),
                None => guard.users.values().next().map(|u| u.id.clone()),
            };
            let Some(user_id) = user_id else {
                return Ok(error("not_found", "no such user").0);
            };
            let grant = format!("grant_{}", "g".repeat(40));
            let Some(attempt) = guard.attempts.get_mut(&state_value) else {
                return Ok(error("not_found", "no such attempt").0);
            };
            attempt.grant = Some(grant.clone());
            attempt.user_id = Some(user_id);
            format!(
                "http://127.0.0.1:{}/mallard/callback?state={state_value}&grant={grant}",
                attempt.port
            )
        };
        let _ = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(target)
            .send()
            .await;
        return Ok(json(StatusCode::OK, json!({"ok": true})).0);
    }
    let mut guard = state.lock().unwrap();
    guard.log.push(StubRequest {
        method: method.to_string(),
        path: path.clone(),
        query: uri.query().map(pct_decode),
        idempotency_key: idempotency.clone(),
    });
    let Reply(response) = route(
        &mut guard,
        &endpoint,
        &method,
        &uri,
        &path,
        bearer.as_deref(),
        idempotency.as_deref(),
        declared_sha.as_deref(),
        content_length,
        repository_revision,
        body,
    );
    Ok(response)
}

fn segments(path: &str) -> Vec<&str> {
    path.trim_start_matches('/').split('/').collect()
}

/// The class of one desktop key inside one repository's namespace.
#[derive(Debug, PartialEq, Eq)]
enum KeyClass {
    Marker,
    Object { sha256: String },
    View { schema: u32, sha256: String },
    Manifest { revision: u64, commit_id: String },
    Tag,
    Claim { parent: String },
    Record { name: String },
}

fn classify_key(repository: &Repository, key: &str) -> Result<KeyClass, String> {
    if key.is_empty()
        || key.len() > 1024
        || key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("the key is outside the grammar".to_string());
    }
    if key == ".mallard/_storage.json" {
        return Ok(KeyClass::Marker);
    }
    let stream = repository.stream_prefix();
    let claims = repository.claims_prefix();
    let content = repository.content_prefix();
    if let Some(name) = key.strip_prefix(&format!("{stream}/")) {
        return if name.contains('/') {
            Err("the key is outside the grammar".to_string())
        } else {
            Ok(KeyClass::Record {
                name: name.to_owned(),
            })
        };
    }
    if let Some(name) = key.strip_prefix(&format!("{claims}/")) {
        return match name.strip_suffix(".json") {
            Some(parent) if !parent.contains('/') => Ok(KeyClass::Claim {
                parent: parent.to_owned(),
            }),
            _ => Err("the key is outside the grammar".to_string()),
        };
    }
    let Some(relative) = key.strip_prefix(&format!("{content}/")) else {
        return Err("the key names another repository".to_string());
    };
    let hex64 = |value: &str| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
    if let Some(sha256) = relative.strip_prefix("_objects/") {
        return if hex64(sha256) {
            Ok(KeyClass::Object {
                sha256: sha256.to_owned(),
            })
        } else {
            Err("the object key is not a digest".to_string())
        };
    }
    if let Some(name) = relative.strip_prefix("_manifests/") {
        let (revision, commit_id) = name
            .strip_suffix(".json")
            .and_then(|stem| stem.split_once('-'))
            .ok_or("the manifest key is outside the grammar")?;
        let revision: u64 = revision
            .parse()
            .map_err(|_| "the manifest key is outside the grammar".to_string())?;
        if !valid_commit_id(commit_id) || revision == 0 {
            return Err("the manifest key is outside the grammar".to_string());
        }
        return Ok(KeyClass::Manifest {
            revision,
            commit_id: commit_id.to_owned(),
        });
    }
    if repository.class != "catalog" {
        if let Some(name) = relative.strip_prefix("_views/") {
            let (schema, sha256) = name
                .strip_suffix(".json")
                .and_then(|stem| stem.split_once('-'))
                .ok_or("the view key is outside the grammar")?;
            let schema: u32 = schema
                .parse()
                .map_err(|_| "the view key is outside the grammar".to_string())?;
            if !hex64(sha256) {
                return Err("the view key is outside the grammar".to_string());
            }
            return Ok(KeyClass::View {
                schema,
                sha256: sha256.to_owned(),
            });
        }
    } else if relative == "_tag.json" {
        return Ok(KeyClass::Tag);
    }
    Err("the key is outside the grammar".to_string())
}

fn valid_commit_id(value: &str) -> bool {
    value
        .strip_prefix("commit-")
        .is_some_and(|hex| hex.len() == 24 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The name the record `bytes` derive to, and its parent's label.
fn derive_record(bytes: &[u8]) -> Result<(RecordName, String), String> {
    let fields: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    let parent = match (
        fields["previous_commit_id"].as_str(),
        fields["previous_record_sha256"].as_str(),
    ) {
        (None, None) => None,
        (Some(commit), Some(sha)) => Some(PublicationToken::new(commit, sha)?),
        _ => return Err("the record names half a parent".to_string()),
    };
    let name = record_name_of(bytes, parent.as_ref())?;
    let label = parent.map_or_else(|| "root".to_string(), |token| token.to_string());
    Ok((name, label))
}

fn identity_holds(repository: &Repository, record: &Value) -> bool {
    match (&repository.bundle_id, &repository.catalog) {
        (Some(bundle), _) => record["bundle_id"].as_str() == Some(bundle),
        (None, Some(item)) => record["repository_id"].as_str() == Some(&item.library_repository_id),
        (None, None) => false,
    }
}

/// The publication state the stream listing carries.
fn publication_status(repository: &Repository, mainline: &Mainline) -> Value {
    let selected = mainline.selected();
    let registration_only = selected.is_some_and(|name| {
        repository
            .publication(&name.file_name())
            .map(|publication| publication.summary["registration_only"] == true)
            .or_else(|| {
                let key = format!("{}/{}", repository.stream_prefix(), name.file_name());
                let record: Value =
                    serde_json::from_slice(&repository.objects.get(&key)?.bytes).ok()?;
                Some(record["summary"]["registration_only"] == true)
            })
            .unwrap_or(false)
    });
    let has_draft = selected.is_some() && !registration_only;
    let live = repository.visibility == "public" && repository.public_commit_id.is_some();
    let state = if !has_draft {
        "no_draft"
    } else if !live {
        "not_public"
    } else if repository.public_commit_id.as_deref() == selected.map(RecordName::commit_id) {
        "matches_published"
    } else {
        "unpublished_draft"
    };
    json!({
        "state": state,
        "private_revision": if has_draft { selected.map(RecordName::revision) } else { None },
        "public_revision": if live { Some(repository.public_revision) } else { None },
    })
}

fn link_body(
    state: &StubState,
    repository: &Repository,
    mainline: &Mainline,
    role: Option<&str>,
    writer: bool,
) -> Value {
    let selected = mainline.selected();
    let registration_only = selected.is_some_and(|name| {
        repository
            .publication(&name.file_name())
            .is_some_and(|publication| publication.summary["registration_only"] == true)
    });
    let request = repository.release_requests.last();
    let release = if selected.is_none() || registration_only {
        json!({"kind": "not_pushed"})
    } else if let Some(request) =
        request.filter(|r| r["state"] == "pending_owner" || r["state"] == "pending_super_admin")
    {
        if request["state"] == "pending_super_admin" {
            json!({"kind": "waiting_super_admin"})
        } else {
            json!({"kind": "waiting_owner", "requested_by": request["requested_by"]})
        }
    } else if let Some(request) = request.filter(|r| {
        r["state"] == "rejected" && r["commit_id"].as_str() == selected.map(RecordName::commit_id)
    }) {
        json!({"kind": "rejected", "reason": request["reason"]})
    } else if repository.visibility == "public" && repository.public_commit_id.is_some() {
        if repository.public_commit_id.as_deref() == selected.map(RecordName::commit_id) {
            json!({"kind": "public"})
        } else {
            json!({"kind": "snapshot_changed"})
        }
    } else {
        json!({"kind": "private"})
    };
    let can_request = writer
        && matches!(
            release["kind"].as_str(),
            Some("private" | "snapshot_changed" | "rejected")
        );
    json!({
        "repository_id": repository.id, "owner_login": state.users[&repository.owner_id].login,
        "name": repository.name, "visibility": repository.visibility, "release": release,
        "repository_revision": repository.revision, "role": role, "can_pull": true, "can_push": writer,
        "can_request_release": can_request
    })
}

fn bytes_reply(bytes: Bytes, sha256: &str, content_type: &str) -> Reply {
    Reply(
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", content_type)
            .header("content-length", bytes.len())
            .header("etag", format!("\"{sha256}\""))
            .header("x-mallard-sha256", sha256)
            .header("cache-control", "no-store")
            .header("x-request-id", "stub-request")
            .body(Full::new(bytes))
            .unwrap(),
    )
}

/// Index a record that just landed, the way the Worker does after R2 has
/// it: a publication row, and, when the record is the selected one, the
/// head pointer and the quota.
fn index_record(state: &mut StubState, repository_id: &str, name: &RecordName, user_id: &str) {
    let repository = state
        .repositories
        .get_mut(repository_id)
        .expect("repository");
    let (_, mainline) = repository.stream();
    let record_key = format!("{}/{}", repository.stream_prefix(), name.file_name());
    let record: Value =
        serde_json::from_slice(&repository.objects[&record_key].bytes).expect("a stored record");
    let manifest_relative = record["manifest_key"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let manifest_key = format!("{}/{}", repository.content_prefix(), manifest_relative);
    let manifest_bytes = repository.objects[&manifest_key].bytes.clone();
    let (summary, references) = if repository.class == "catalog" {
        let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("a stored manifest");
        let (_, entry) = manifest["resources"]
            .as_object()
            .and_then(|map| map.iter().next())
            .expect("one entry");
        let resource = &entry["resource"];
        let files = resource["files"].as_array().cloned().unwrap_or_default();
        let display_name = match resource["resource_kind"].as_str() {
            Some("agent_skill") => resource["identity"]["effective_name"].clone(),
            Some("plugin_install_intent") => resource["identity"]["plugin_id"].clone(),
            _ => resource["descriptor"]["display_name"].clone(),
        };
        let summary = json!({
            "display_name": display_name, "kind": "catalog", "revision": record["revision"],
            "updated_at": manifest["updated_at"], "resources": 1, "files": files.len(), "conversations": 0,
            "registration_only": false, "providers": [resource["provider"]],
        });
        let references = files
            .iter()
            .filter_map(|file| file["sha256"].as_str().map(|sha| format!("_objects/{sha}")))
            .collect::<Vec<_>>();
        (summary, references)
    } else {
        let manifest: BundleManifest =
            serde_json::from_slice(&manifest_bytes).expect("a stored manifest");
        let mut references = manifest
            .all_files()
            .map(|(_, entry)| format!("_objects/{}", entry.sha256))
            .collect::<Vec<_>>();
        for (_, scope) in manifest.provider_scopes() {
            for thread in scope.conversations.values() {
                let Some(transcript) = thread
                    .files
                    .iter()
                    .find(|f| f.role == mallard_domain::ConversationFileRole::Transcript)
                else {
                    continue;
                };
                if let Some(entry) = scope.files.get(&transcript.logical_path) {
                    references.push(format!("_views/1-{}.json", entry.sha256));
                }
            }
        }
        (record["summary"].clone(), references)
    };
    let claim_key = format!(
        "{}/{}.json",
        repository.claims_prefix(),
        name.parent()
            .map_or_else(|| "root".to_string(), |token| token.to_string())
    );
    let created_by = repository
        .objects
        .get(&claim_key)
        .and_then(|claim| claim.user_id.clone())
        .unwrap_or_else(|| user_id.to_owned());
    let publication = Publication {
        record_name: name.file_name(),
        commit_id: name.commit_id().to_owned(),
        manifest_revision: name.revision(),
        manifest_key: manifest_relative,
        manifest_sha256: record["manifest_sha256"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        manifest_size: record["manifest_size"].as_u64().unwrap_or_default(),
        summary,
        created_by,
        created_at: now(),
        state: "available".to_owned(),
    };
    repository.publications.push(publication);
    if mainline.selected() == Some(name) {
        let previous = repository.private_head.take();
        if let Some(previous) = previous {
            if let Some(row) = repository
                .publications
                .iter_mut()
                .find(|p| p.record_name == previous)
            {
                row.state = "retired".to_owned();
            }
        }
        repository.private_head = Some(name.file_name());
        repository.private_revision += 1;
        repository.release_requests.clear();
        let content = repository.content_prefix();
        for reference in references {
            let key = format!("{content}/{reference}");
            if repository.counted.insert(key.clone()) {
                if let Some(object) = repository.objects.get(&key) {
                    repository.committed_bytes += object.bytes.len() as u64;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn route(
    state: &mut StubState,
    endpoint: &str,
    method: &Method,
    uri: &hyper::Uri,
    path: &str,
    bearer: Option<&str>,
    idempotency: Option<&str>,
    declared_sha: Option<&str>,
    content_length: Option<usize>,
    repository_revision: Option<u64>,
    body: Bytes,
) -> Reply {
    let parts = segments(path);
    // Sign-in (section 7.2).
    if method == Method::POST && path == "/v1/auth/desktop/start" {
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let Some(challenge) = value["code_challenge"].as_str() else {
            return error("invalid_request", "code_challenge");
        };
        let Some(port) = value["loopback_port"].as_u64() else {
            return error("invalid_request", "loopback_port");
        };
        state.next += 1;
        let attempt_state = format!("{:0>48x}", state.next);
        state.attempts.insert(
            attempt_state.clone(),
            Attempt {
                challenge: challenge.to_owned(),
                port: port as u16,
                grant: None,
                user_id: None,
            },
        );
        return json(
            StatusCode::OK,
            json!({"state": attempt_state, "authorize_url": format!("{endpoint}/stub/consent?state={attempt_state}")}),
        );
    }
    if method == Method::POST && path == "/v1/auth/desktop/exchange" {
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let attempt_state = value["state"].as_str().unwrap_or_default().to_owned();
        let Some(attempt) = state.attempts.remove(&attempt_state) else {
            return error("authentication_required", "unknown attempt");
        };
        let verifier = value["code_verifier"].as_str().unwrap_or_default();
        let expected = aws_smithy_types::base64::encode(<sha2::Sha256 as sha2::Digest>::digest(
            verifier.as_bytes(),
        ))
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_");
        if attempt.grant.as_deref() != value["grant"].as_str() || expected != attempt.challenge {
            return error("authentication_required", "grant or verifier mismatch");
        }
        let Some(user_id) = attempt.user_id else {
            return error("authentication_required", "incomplete attempt");
        };
        let user = state.users[&user_id].clone();
        let token = format!("tok_{}_{:016x}", user.id, state.next);
        state.next += 1;
        state.tokens.insert(token.clone(), user.id.clone());
        return json(
            StatusCode::OK,
            json!({"token": token, "expires_at": now() + 90 * 24 * 3600, "user": {"id": user.id, "github_login": user.login, "service_role": "user"}}),
        );
    }
    // Everything else needs a session.
    let Some(user_id) = bearer.and_then(|token| state.tokens.get(token).cloned()) else {
        return error("authentication_required", "Sign in to continue.");
    };
    if state.suspended.contains(&user_id) {
        return error("forbidden", "This account is suspended.");
    }
    let user = state.users[&user_id].clone();
    if method == Method::GET && path == "/v1/me" {
        return json(
            StatusCode::OK,
            json!({"user": {"id": user.id, "github_login": user.login, "service_role": "user"}, "client": "desktop", "limits": {}}),
        );
    }
    if method == Method::GET && path == "/v1/repositories" {
        let class = query(uri, "class");
        let rows = state
            .repositories
            .values()
            .filter(|r| r.state != "removed" && r.members.contains_key(&user_id))
            .filter(|r| match class.as_deref() {
                Some("project") => r.class != "catalog",
                Some("catalog") => r.class == "catalog",
                _ => true,
            })
            .map(|r| {
                summary(
                    state,
                    r,
                    r.members.get(&user_id).map(|(_, role)| role.as_str()),
                    true,
                )
            })
            .collect::<Vec<_>>();
        return json(StatusCode::OK, json!({"repositories": rows}));
    }
    if method == Method::GET && path == "/v1/repositories/library" {
        let wanted = |name: &str| query(uri, name).unwrap_or_default();
        let (provider, kind, format) = (
            wanted("provider"),
            wanted("resource_kind"),
            wanted("format"),
        );
        let version: u32 = wanted("format_version").parse().unwrap_or(0);
        let rows = state
            .repositories
            .values()
            .filter(|r| r.active() && r.members.contains_key(&user_id))
            .filter(|r| {
                r.catalog.as_ref().is_some_and(|item| {
                    item.provider == provider
                        && item.resource_kind == kind
                        && item.format == format
                        && item.format_version == version
                })
            })
            .map(|r| {
                let item = r.catalog.as_ref().expect("a catalog repository");
                let (mut names, mainline) = r.stream();
                names.sort_by_key(RecordName::file_name);
                let truncated = names.len() > LISTING_PAGE;
                let selected = if truncated { None } else { mainline.selected() };
                let record = selected.and_then(|name| {
                    let key = format!("{}/{}", r.stream_prefix(), name.file_name());
                    r.objects.get(&key).map(|object| object.bytes.clone())
                });
                let manifest = selected.and_then(|name| {
                    let key = format!(
                        "{}/_manifests/{}-{}.json",
                        r.content_prefix(),
                        name.revision(),
                        name.commit_id()
                    );
                    r.objects
                        .get(&key)
                        .filter(|object| object.bytes.len() <= INLINE_MANIFEST_BYTES)
                        .map(|object| object.bytes.clone())
                });
                json!({
                    "repository_id": r.id, "resource_id": item.resource_id,
                    "library_repository_id": item.library_repository_id,
                    "names": names.iter().take(LISTING_PAGE).map(RecordName::file_name).collect::<Vec<_>>(),
                    "truncated": truncated,
                    "record": record.map(|bytes| aws_smithy_types::base64::encode(&bytes)),
                    "manifest": manifest.map(|bytes| aws_smithy_types::base64::encode(&bytes)),
                })
            })
            .collect::<Vec<_>>();
        return json(StatusCode::OK, json!({"repositories": rows}));
    }
    if method == Method::POST && path == "/v1/repositories" {
        if idempotency.is_none() {
            return error("invalid_request", "Idempotency-Key is required.");
        }
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let class = value["repository_class"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let (bundle_id, fingerprint, catalog) = if class == "catalog" {
            let text = |field: &str| value[field].as_str().map(str::to_owned);
            let Some(item) = text("provider")
                .zip(text("resource_kind"))
                .zip(text("format"))
                .map(|((provider, resource_kind), format)| CatalogItem {
                    provider,
                    resource_kind,
                    format,
                    format_version: value["format_version"].as_u64().unwrap_or(0) as u32,
                    resource_id: text("resource_id").unwrap_or_default(),
                    library_repository_id: text("library_repository_id").unwrap_or_default(),
                })
            else {
                return error("invalid_request", "coordinate");
            };
            if !item.library_repository_id.starts_with("library-")
                || item.library_repository_id.len() != 40
                || item.resource_id.is_empty()
            {
                return error("invalid_request", "library_repository_id");
            }
            if state.repositories.values().any(|r| {
                r.active() && r.catalog.as_ref() == Some(&item) && r.members.contains_key(&user_id)
            }) {
                return error("storage_managed_account_exists", "already registered");
            }
            (None, None, Some(item))
        } else {
            let bundle_id = value["bundle_id"].as_str().unwrap_or_default().to_owned();
            let fingerprint = value["repository_fingerprint"].as_str().map(str::to_owned);
            if !(class == "git_project" && fingerprint.is_some()
                || class == "non_git_project" && fingerprint.is_none())
            {
                return error("invalid_request", "class and fingerprint disagree");
            }
            if bundle_id.len() != 32 {
                return error("invalid_request", "bundle_id");
            }
            if state.repositories.values().any(|r| {
                r.active()
                    && r.bundle_id.as_deref() == Some(bundle_id.as_str())
                    && r.members.contains_key(&user_id)
            }) {
                return error("storage_managed_account_exists", "already registered");
            }
            (Some(bundle_id), fingerprint, None)
        };
        let github_url = value["github_url"].as_str().map(str::to_owned);
        if let Some(url) = &github_url {
            if class != "git_project"
                || crate::project_activity::normalize_github_repository_url(url).as_ref()
                    != Some(url)
                || fingerprint.as_deref() != Some(sha256_hex(url.as_bytes()).as_str())
            {
                return error(
                    "invalid_request",
                    "GitHub origin does not match the fingerprint",
                );
            }
        }
        let base = normalize_repository_name(value["name"].as_str().unwrap_or_default());
        let held: BTreeSet<String> = state
            .repositories
            .values()
            .filter(|r| r.active() && r.owner_id == user_id)
            .map(|r| r.name.to_ascii_lowercase())
            .collect();
        let id = state.id("repo");
        let marker = serde_json::to_vec(&crate::storage_core::StorageMarker::current()).unwrap();
        let mut objects = BTreeMap::new();
        objects.insert(
            ".mallard/_storage.json".to_string(),
            StoredObject {
                sha256: sha256_hex(&marker),
                bytes: Bytes::from(marker),
                user_id: None,
            },
        );
        let repository = Repository {
            id: id.clone(),
            state: "active".to_owned(),
            class,
            name: pick_available_name(&base, &held),
            owner_id: user_id.clone(),
            bundle_id,
            repository_fingerprint: fingerprint,
            catalog,
            visibility: "private".to_owned(),
            github_url,
            revision: 1,
            private_revision: 0,
            public_revision: 0,
            private_head: None,
            public_commit_id: None,
            publications: vec![],
            objects,
            counted: BTreeSet::new(),
            committed_bytes: 0,
            members: BTreeMap::from([(user_id.clone(), (user.login.clone(), "owner".to_owned()))]),
            release_requests: vec![],
            invite_code: None,
        };
        state.repositories.insert(id.clone(), repository);
        let repository = state.repositories[&id].clone();
        return json(
            StatusCode::CREATED,
            summary(state, &repository, Some("owner"), true),
        );
    }
    if method == Method::POST && path == "/v1/invitations/accept" {
        // Section 4.3: the code belongs to the repository; whoever enters it
        // joins as a Member, and entering it again changes nothing.
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let normalized = normalize_invitation_code(value["code"].as_str().unwrap_or_default());
        let Some(id) = state
            .repositories
            .values()
            .find(|r| r.active() && r.invite_code.as_ref().is_some_and(|c| c.code == normalized))
            .map(|r| r.id.clone())
        else {
            return error(
                "invitation_invalid",
                "The invitation code is unknown or was revoked.",
            );
        };
        let repository = state.repositories.get_mut(&id).unwrap();
        if !repository.members.contains_key(&user_id) {
            repository
                .members
                .insert(user_id.clone(), (user.login.clone(), "member".to_owned()));
            repository.revision += 1;
            repository.release_requests.clear();
        }
        let repository = state.repositories[&id].clone();
        let role = repository.members[&user_id].1.clone();
        return json(
            StatusCode::OK,
            summary(state, &repository, Some(&role), true),
        );
    }
    if method == Method::GET
        && parts.len() == 4
        && parts[..2] == ["v1", "repositories"]
        && (parts[2] == "by-bundle" || parts[2] == "by-library")
    {
        // A member's repository first; otherwise a public one, whose head is
        // readable by everyone.
        let matches = |r: &Repository| {
            if parts[2] == "by-bundle" {
                r.bundle_id.as_deref() == Some(parts[3])
            } else {
                r.catalog
                    .as_ref()
                    .is_some_and(|item| item.library_repository_id == parts[3])
            }
        };
        let Some(repository) = state
            .repositories
            .values()
            .find(|r| r.active() && matches(r) && r.members.contains_key(&user_id))
            .or_else(|| state.repositories.values().find(|r| matches(r) && r.open()))
        else {
            return no_content();
        };
        return json(StatusCode::OK, json!({"repository_id": repository.id}));
    }
    if method == Method::GET && parts.len() == 5 && parts[..3] == ["v1", "repositories", "by-name"]
    {
        // Both segments compare case-insensitively, as the service's do.
        let Some(owner) = state
            .users
            .values()
            .find(|u| u.login.eq_ignore_ascii_case(parts[3]))
        else {
            return error("not_found", "no such owner");
        };
        // The active repository of that name, else the newest one being removed.
        let named = |wanted: &str| {
            state.repositories.values().rfind(|r| {
                r.state == wanted && r.owner_id == owner.id && r.name.eq_ignore_ascii_case(parts[4])
            })
        };
        let Some(repository) = named("active").or_else(|| named("deleting")) else {
            return error("not_found", "no such repository");
        };
        if !repository.active() {
            return if repository.members.contains_key(&user_id) {
                error("repository_removed", "This repository is being removed.")
            } else {
                error("not_found", "not visible")
            };
        }
        if let Some((_, role)) = repository.members.get(&user_id) {
            return json(StatusCode::OK, summary(state, repository, Some(role), true));
        }
        if repository.open() {
            // Anyone signed in is a writer on an open repository (section 4.3).
            return json(StatusCode::OK, summary(state, repository, None, true));
        }
        return error("not_found", "not visible");
    }
    if parts.len() >= 3 && parts[0] == "v1" && parts[1] == "repositories" {
        let repository_id = parts[2].to_owned();
        let Some(repository) = state.repositories.get(&repository_id).cloned() else {
            return error("not_found", "no such repository");
        };
        let role = repository
            .members
            .get(&user_id)
            .map(|(_, role)| role.clone());
        let rest = &parts[3..];
        // Section 6.6: a removed repository is missing to everyone; one being
        // removed answers its collaborators `repository_removed` and is
        // missing to everyone else.
        if repository.state == "removed" || (!repository.active() && role.is_none()) {
            return error("not_found", "not visible");
        }
        if !repository.active() {
            return error("repository_removed", "This repository is being removed.");
        }
        // Section 4.3: a collaborator, or anyone signed in on an open
        // repository (public with a released head), is a writer and reads and
        // pushes alike; everyone else sees nothing. The stub has no anonymous
        // callers and no Super Admin.
        if role.is_none() && !repository.open() {
            return error("not_found", "not visible");
        }
        let manages = matches!(role.as_deref(), Some("owner") | Some("admin"));
        if method == Method::GET && rest.is_empty() && std::mem::take(&mut state.fail_next_summary)
        {
            return error("temporarily_unavailable", "details unavailable");
        }
        // The store routes: the key is the rest of the path.
        if rest.first() == Some(&"store") {
            let key = rest[1..].join("/");
            if method == Method::GET && rest.len() == 1 {
                return list_store(state, &repository, role.as_deref(), uri);
            }
            if method == Method::GET {
                let Some(object) = repository.objects.get(&key) else {
                    return error("absent", "no such object");
                };
                let content_type = if key.ends_with(".json") {
                    "application/json"
                } else {
                    "application/octet-stream"
                };
                return bytes_reply(object.bytes.clone(), &object.sha256, content_type);
            }
            if method == Method::PUT {
                let Some(revision) = repository_revision else {
                    return error(
                        "invalid_request",
                        "X-Mallard-Repository-Revision is required.",
                    );
                };
                if revision != repository.revision {
                    return error(
                        "revision_conflict",
                        "The repository changed; start a new Push.",
                    );
                }
                return put_store(
                    state,
                    &repository_id,
                    &key,
                    &user_id,
                    declared_sha,
                    content_length,
                    body,
                );
            }
            return error("invalid_request", "method");
        }
        match (method, rest) {
            (&Method::GET, []) => {
                return json(
                    StatusCode::OK,
                    summary(state, &repository, role.as_deref(), true),
                )
            }
            (&Method::GET, ["members"]) => {
                if role.is_none() {
                    return error("forbidden", "Only collaborators can list members.");
                }
                let members = repository
                    .members
                    .iter()
                    .map(|(id, (login, role))| json!({"user_id": id, "github_login": login, "role": role, "created_at": 0}))
                    .collect::<Vec<_>>();
                let invitation = repository.invite_code.as_ref().map(|code| {
                    json!({"set_by": code.set_by, "set_by_login": state.users.get(&code.set_by).map(|u| u.login.clone()), "set_at": code.set_at})
                });
                return json(
                    StatusCode::OK,
                    json!({"repository_id": repository.id, "revision": repository.revision, "members": members, "invitation": invitation}),
                );
            }
            (&Method::PUT, ["invitation"]) => {
                if !manages {
                    return error("forbidden", "Only the Owner or an Admin can do this.");
                }
                let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                if value["expected_repository_revision"].as_u64() != Some(repository.revision) {
                    return error("revision_conflict", "stale version");
                }
                let code = state.set_invite_code(&repository_id, &user_id);
                let repository = &state.repositories[&repository_id];
                let set = repository.invite_code.as_ref().unwrap();
                return json(
                    StatusCode::OK,
                    json!({"code": code, "set_by": set.set_by, "set_by_login": user.login, "set_at": set.set_at, "repository_revision": repository.revision}),
                );
            }
            (&Method::DELETE, ["invitation"]) => {
                if !manages {
                    return error("forbidden", "Only the Owner or an Admin can do this.");
                }
                let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let expected = value["expected_repository_revision"]
                    .as_u64()
                    .or_else(|| query(uri, "expected_repository_revision")?.parse().ok());
                if expected != Some(repository.revision) {
                    return error("revision_conflict", "stale version");
                }
                let repository = state.repositories.get_mut(&repository_id).unwrap();
                repository.invite_code = None;
                repository.revision += 1;
                return json(
                    StatusCode::OK,
                    json!({"ok": true, "repository_id": repository_id, "repository_revision": repository.revision}),
                );
            }
            (&Method::GET, ["release-requests"]) => {
                return json(
                    StatusCode::OK,
                    json!({"open": repository.release_requests.last().cloned(), "recent": []}),
                );
            }
            (&Method::POST, ["release-requests"]) => {
                let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                if value["expected_repository_revision"].as_u64() != Some(repository.revision) {
                    return error("revision_conflict", "stale version");
                }
                let Some(head) = repository.head() else {
                    return error("release_request_invalid", "nothing to release");
                };
                let id = state.id("rr");
                let request = json!({"id": id, "state": "pending_owner", "requested_by": user_id, "created_at": now(), "commit_id": head.commit_id});
                let repository = state.repositories.get_mut(&repository_id).unwrap();
                repository.release_requests.push(request.clone());
                repository.revision += 1;
                return json(StatusCode::CREATED, request);
            }
            (&Method::POST, ["private-snapshot", "discard"]) => {
                if !manages {
                    return error("forbidden", "Only the Owner or an Admin can do this.");
                }
                let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                if value["confirm"] != true {
                    return error("invalid_request", "confirm must be true.");
                }
                if value["expected_repository_revision"].as_u64() != Some(repository.revision) {
                    return error(
                        "revision_conflict",
                        "The repository changed since you read it. Reload and retry.",
                    );
                }
                let kept_revision = match state.discard(&repository_id) {
                    Ok(kept) => kept,
                    Err(detail) => return error("invalid_request", detail),
                };
                let removal_id = state.id("rmv");
                return json(
                    StatusCode::OK,
                    json!({"ok": true, "repository_id": repository_id, "removal_id": removal_id, "state": "done", "kept_revision": kept_revision}),
                );
            }
            _ => {}
        }
    }
    error("not_found", "No such route.")
}

fn list_store(
    state: &mut StubState,
    repository: &Repository,
    role: Option<&str>,
    uri: &hyper::Uri,
) -> Reply {
    match state.fail_next_listing.take() {
        Some(ListingFault::Unavailable) => {
            return error("temporarily_unavailable", "listing unavailable")
        }
        Some(ListingFault::ForeignNotFound) => {
            return Reply(
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("content-type", "text/html")
                    .body(Full::new(Bytes::from_static(b"<h1>Not Found</h1>")))
                    .unwrap(),
            )
        }
        None => {}
    }
    let Some(prefix) = query(uri, "prefix") else {
        return error("key_invalid", "prefix");
    };
    let prefix = prefix.trim_end_matches('/').to_owned();
    let stream = repository.stream_prefix();
    let claims = repository.claims_prefix();
    let content = repository.content_prefix();
    let allowed = prefix == stream
        || prefix == claims
        || prefix == content
        || [
            format!("{content}/_objects"),
            format!("{content}/_manifests"),
            format!("{content}/_views"),
        ]
        .contains(&prefix);
    if !allowed
        || prefix
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return error("key_invalid", "the prefix is outside this repository");
    }
    let cursor = query(uri, "cursor");
    let mut keys = repository.keys_under(&prefix);
    keys.sort();
    if let Some(cursor) = &cursor {
        keys.retain(|key| key > cursor);
    }
    let next_cursor = (keys.len() > LISTING_PAGE).then(|| keys[LISTING_PAGE - 1].clone());
    keys.truncate(LISTING_PAGE);
    let mut answer = json!({"keys": keys, "next_cursor": next_cursor});
    if prefix == stream {
        let (_, mainline) = repository.stream();
        answer["link"] = link_body(state, repository, &mainline, role, true);
        answer["publication"] = publication_status(repository, &mainline);
    }
    json(StatusCode::OK, answer)
}

fn put_store(
    state: &mut StubState,
    repository_id: &str,
    key: &str,
    user_id: &str,
    declared_sha: Option<&str>,
    content_length: Option<usize>,
    body: Bytes,
) -> Reply {
    let repository = state.repositories[repository_id].clone();
    let Some(declared) = declared_sha else {
        return error("invalid_request", "X-Mallard-Sha256 is required.");
    };
    if content_length != Some(body.len()) {
        return error("invalid_request", "Content-Length is required.");
    }
    let sha256 = sha256_hex(&body);
    if declared != sha256 {
        return error(
            "object_integrity_failed",
            "X-Mallard-Sha256 disagrees with the body",
        );
    }
    let class = match classify_key(&repository, key) {
        Ok(class) => class,
        Err(detail) => return error("key_invalid", &detail),
    };
    let too_large = |limit: usize| body.len() > limit;
    let mut claim_user = None;
    match &class {
        KeyClass::Marker => return error("key_invalid", "the marker is never written"),
        KeyClass::Object { sha256: expected } => {
            if too_large(MAX_OBJECT_BYTES) {
                return error("object_too_large", "over 90 MiB");
            }
            if expected != &sha256 {
                return error("object_integrity_failed", "the key digest differs");
            }
        }
        KeyClass::View {
            schema,
            sha256: source,
        } => {
            if too_large(MAX_MANIFEST_BYTES) {
                return error("object_too_large", "over 8 MiB");
            }
            let value: Value = match serde_json::from_slice(&body) {
                Ok(value) => value,
                Err(_) => return error("object_invalid", "view is not JSON"),
            };
            if value["schema_version"].as_u64() != Some(u64::from(*schema))
                || !value["resource_id"].is_string()
                || !value["events"].is_array()
            {
                return error("object_invalid", "view shape");
            }
            let transcript = format!("{}/_objects/{source}", repository.content_prefix());
            if !repository.objects.contains_key(&transcript) {
                return error("object_invalid", "the view's transcript is not stored");
            }
        }
        KeyClass::Manifest {
            revision,
            commit_id,
        } => {
            if too_large(MAX_MANIFEST_BYTES) {
                return error("object_too_large", "over 8 MiB");
            }
            if repository.class == "catalog" {
                let value: Value = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(_) => return error("object_invalid", "manifest is not JSON"),
                };
                let item = repository.catalog.as_ref().expect("a catalog repository");
                if value["repository_id"].as_str() != Some(&item.library_repository_id)
                    || value["revision"].as_u64() != Some(*revision)
                    || value["commit_id"].as_str() != Some(commit_id)
                {
                    return error("object_invalid", "identity mismatch");
                }
                let Some(entries) = value["resources"].as_object().filter(|map| map.len() == 1)
                else {
                    return error(
                        "object_invalid",
                        "manifest.resources must hold exactly one entry",
                    );
                };
                let (id, entry) = entries.iter().next().expect("one entry");
                let resource = &entry["resource"];
                if id != &item.resource_id
                    || resource["resource_id"].as_str() != Some(&item.resource_id)
                    || resource["provider"].as_str() != Some(&item.provider)
                    || resource["resource_kind"].as_str() != Some(&item.resource_kind)
                    || resource["format"].as_str() != Some(&item.format)
                    || resource["format_version"].as_u64() != Some(u64::from(item.format_version))
                {
                    return error("object_invalid", "identity mismatch: foreign resource_id");
                }
            } else {
                let parsed: BundleManifest = match serde_json::from_slice(&body) {
                    Ok(parsed) => parsed,
                    Err(e) => return error("object_invalid", &format!("parse: {e}")),
                };
                if parsed.validate().is_err() {
                    return error("object_invalid", "manifest failed validation");
                }
                if Some(parsed.bundle.bundle_id.as_str()) != repository.bundle_id.as_deref()
                    || parsed.revision != *revision
                    || &parsed.commit_id != commit_id
                {
                    return error("object_invalid", "identity mismatch");
                }
                if (repository.class == "git_project")
                    != parsed.bundle.repository_fingerprint.is_some()
                    || parsed.bundle.repository_fingerprint != repository.repository_fingerprint
                {
                    return error("object_invalid", "class mismatch");
                }
            }
        }
        KeyClass::Tag => {
            let value: Value = match serde_json::from_slice(&body) {
                Ok(value) => value,
                Err(_) => return error("object_invalid", "tag is not JSON"),
            };
            let item = repository.catalog.as_ref().expect("a catalog repository");
            if value["repository_id"].as_str() != Some(&item.library_repository_id)
                || value["provider"].as_str() != Some(&item.provider)
                || value["resource_kind"].as_str() != Some(&item.resource_kind)
                || value["format"].as_str() != Some(&item.format)
                || value["format_version"].as_u64() != Some(u64::from(item.format_version))
            {
                return error("object_invalid", "the tag names another coordinate");
            }
        }
        KeyClass::Claim { parent } => {
            if too_large(MAX_RECORD_BYTES) {
                return error("object_too_large", "over 64 KiB");
            }
            let (_, label) = match derive_record(&body) {
                Ok(derived) => derived,
                Err(detail) => return error("object_invalid", &detail),
            };
            if &label != parent {
                return error("object_invalid", "the claim's record names another parent");
            }
            let record: Value = serde_json::from_slice(&body).expect("derived above");
            if !identity_holds(&repository, &record) {
                return error("object_invalid", "identity mismatch");
            }
            claim_user = Some(user_id.to_owned());
        }
        KeyClass::Record { name } => {
            if too_large(MAX_RECORD_BYTES) {
                return error("object_too_large", "over 64 KiB");
            }
            let (derived, label) = match derive_record(&body) {
                Ok(derived) => derived,
                Err(detail) => return error("object_invalid", &detail),
            };
            if &derived.file_name() != name {
                return error("object_invalid", "the name does not derive from the bytes");
            }
            let record: Value = serde_json::from_slice(&body).expect("derived above");
            if !identity_holds(&repository, &record) {
                return error("object_invalid", "identity mismatch");
            }
            let expected_key = format!(
                "_manifests/{}-{}.json",
                derived.revision(),
                derived.commit_id()
            );
            if record["manifest_key"].as_str() != Some(&expected_key) {
                return error("object_invalid", "manifest_key");
            }
            let manifest = repository
                .objects
                .get(&format!("{}/{expected_key}", repository.content_prefix()));
            let manifest_matches = manifest.is_some_and(|object| {
                Some(object.sha256.as_str()) == record["manifest_sha256"].as_str()
                    && Some(object.bytes.len() as u64) == record["manifest_size"].as_u64()
            });
            if !manifest_matches {
                return error(
                    "object_invalid",
                    "the record names a manifest the repository does not hold",
                );
            }
            let claim_key = format!("{}/{label}.json", repository.claims_prefix());
            if repository
                .objects
                .get(&claim_key)
                .is_none_or(|claim| claim.sha256 != sha256)
            {
                return error(
                    "claim_mismatch",
                    "the claim of the record's parent does not hold this record",
                );
            }
        }
    }
    // Injected failures around the two requests the protocol sends once.
    let fault = match &class {
        KeyClass::Claim { .. } => state.fail_next_claim.take(),
        KeyClass::Record { .. } => state.fail_next_record.take(),
        _ => None,
    };
    if fault == Some(Fault::Before) {
        return error(
            "temporarily_unavailable",
            "injected failure before the write",
        );
    }
    // Create-only: the key holds what it holds.
    if let Some(existing) = repository.objects.get(key) {
        return if matches!(class, KeyClass::Claim { .. }) && existing.sha256 != sha256 {
            error("claim_taken", "the claim holds another writer's record")
        } else {
            error("already_present", "the key already holds these bytes")
        };
    }
    let repository = state
        .repositories
        .get_mut(repository_id)
        .expect("repository");
    repository.objects.insert(
        key.to_owned(),
        StoredObject {
            bytes: body,
            sha256,
            user_id: claim_user,
        },
    );
    if let KeyClass::Record { name } = &class {
        if !std::mem::take(&mut state.skip_next_index) {
            let name = RecordName::parse(name).expect("validated above");
            index_record(state, repository_id, &name, user_id);
        }
    }
    if fault == Some(Fault::After) {
        return error(
            "temporarily_unavailable",
            "injected failure after the write",
        );
    }
    json(StatusCode::OK, json!({"stored": true}))
}
