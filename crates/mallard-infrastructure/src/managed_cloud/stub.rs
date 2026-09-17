//! An in-process stand-in for the Mallard Cloud Worker with the guard
//! semantics the desktop relies on: bearer sessions, bundle → repository,
//! one open upload per repository, the missing list, the commit CAS on the
//! private head, carried-forward views, and the desktop sign-in dance.
//! State lives in memory; tests reach it directly.
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mallard_domain::{sha256_hex, BundleManifest, ConversationFileRole};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StubRequest {
    pub method: String,
    pub path: String,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StubUser {
    pub id: String,
    pub login: String,
}

#[derive(Clone, Debug)]
pub struct Publication {
    pub commit_id: String,
    pub manifest: Bytes,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub manifest_revision: u64,
    pub summary: Value,
    /// resource id → (source transcript digest, view digest)
    pub views: BTreeMap<String, (String, String)>,
    pub created_by: String,
    pub created_at: u64,
}

/// The repository's one invitation code (section 4.3), kept in clear here.
#[derive(Clone, Debug)]
pub struct InviteCode {
    /// Normalized: upper case, no separators.
    pub code: String,
    pub set_by: String,
    pub set_at: u64,
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub id: String,
    pub class: String,
    /// The address segment after the Owner's login, as the service stores it.
    pub name: String,
    /// The creator, who is the Owner; the address namespace.
    pub owner_id: String,
    pub bundle_id: String,
    pub repository_fingerprint: Option<String>,
    /// `private` until a test releases it; a public repository's private
    /// head doubles as its public head here.
    pub visibility: String,
    /// The canonical GitHub address the service discloses to every reader.
    pub github_url: Option<String>,
    pub revision: u64,
    pub private_revision: u64,
    pub private_head: Option<Publication>,
    pub publications: Vec<Publication>,
    pub objects: BTreeMap<String, Bytes>,
    /// user id → (login, role)
    pub members: BTreeMap<String, (String, String)>,
    pub release_requests: Vec<Value>,
    pub invite_code: Option<InviteCode>,
}

#[derive(Clone, Debug)]
struct InventoryEntry {
    sha256: String,
    size: u64,
    role: String,
    resource_id: Option<String>,
    source_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Upload {
    pub id: String,
    pub repository_id: String,
    pub user_id: String,
    pub state: String,
    expected_repository_revision: u64,
    expected_private_revision: u64,
    pub commit_id: String,
    manifest: Bytes,
    manifest_revision: u64,
    summary: Value,
    inventory: Vec<InventoryEntry>,
    received: BTreeSet<String>,
    pub expires_at: u64,
    pub created_at: u64,
}

struct Attempt {
    challenge: String,
    port: u16,
    grant: Option<String>,
    user_id: Option<String>,
}

#[derive(Default)]
pub struct StubState {
    pub users: BTreeMap<String, StubUser>,
    tokens: BTreeMap<String, String>,
    pub repositories: BTreeMap<String, Repository>,
    pub uploads: BTreeMap<String, Upload>,
    attempts: BTreeMap<String, Attempt>,
    pub log: Vec<StubRequest>,
    next: u64,
    /// The next commit answers `head_changed` regardless of the head.
    pub fail_next_commit: bool,
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
    /// Run the expiry job: every open upload past its lifetime is closed.
    pub fn expire_uploads(&mut self, now: u64) {
        for upload in self.uploads.values_mut() {
            if upload.state == "open" && upload.expires_at <= now {
                upload.state = "expired".to_string();
            }
        }
    }
    /// Make a repository public: its private head becomes the public one,
    /// and `github_url` is what non-members learn about its Git remote.
    pub fn release(&mut self, repository_id: &str, github_url: Option<&str>) {
        let repository = self
            .repositories
            .get_mut(repository_id)
            .expect("repository");
        repository.visibility = "public".to_owned();
        repository.github_url = github_url.map(str::to_owned);
    }
    pub fn repository_for_bundle(&self, bundle_id: &str) -> Option<&Repository> {
        self.repositories
            .values()
            .find(|r| r.bundle_id == bundle_id)
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
    format!("{}-{}-{}-{}", &raw[..5], &raw[5..10], &raw[10..15], &raw[15..20])
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
        "not_found" => 404,
        "invalid_request" => 400,
        "manifest_invalid" | "object_integrity_failed" => 422,
        "upload_expired" | "repository_removed" => 410,
        "object_too_large" | "quota_exceeded" => 413,
        _ => 409,
    };
    json(
        StatusCode::from_u16(status).unwrap(),
        json!({"error": code, "detail": detail, "request_id": "stub-request"}),
    )
}

fn head_body(
    repository: &Repository,
    publication: &Publication,
    users: &BTreeMap<String, StubUser>,
) -> Value {
    json!({
        "scope": "private",
        "revision": publication.manifest_revision,
        "private_revision": repository.private_revision,
        "commit_id": publication.commit_id,
        "manifest_sha256": publication.manifest_sha256,
        "manifest_size": publication.manifest_size,
        "updated_at": publication.created_at,
        "kind": "project",
        "bundle_id": repository.bundle_id,
        "pushed_by": users.get(&publication.created_by).map(|u| json!({"user_id": u.id, "github_login": u.login})),
    })
}

/// The service caps a member's own open uploads; other members' never block.
const OPEN_UPLOADS_PER_MEMBER: usize = 64;

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

/// The summary as a writer reads it (a collaborator, or anyone signed in
/// on a public repository: section 4.3), or, with `writer` false, the public
/// face: fingerprint, private head, open uploads and requests withheld; the
/// released head shown as the public one.
fn summary(state: &StubState, repository: &Repository, role: Option<&str>, writer: bool) -> Value {
    let member = writer;
    let public = repository.visibility == "public";
    let mut open: Vec<&Upload> = state
        .uploads
        .values()
        .filter(|u| u.repository_id == repository.id && u.state == "open" && u.expires_at > now())
        .collect();
    open.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    let open_uploads = open
        .iter()
        .map(|u| {
            json!({
                "id": u.id,
                "user_id": u.user_id,
                "commit_id": u.commit_id,
                "expires_at": u.expires_at,
                "created_at": u.created_at,
                "stale": u.expected_repository_revision != repository.revision
                    || u.expected_private_revision != repository.private_revision,
            })
        })
        .collect::<Vec<_>>();
    let head = repository
        .private_head
        .as_ref()
        .map(|p| head_body(repository, p, &state.users));
    let tag = repository.private_head.as_ref().map(|p| p.summary.clone());
    json!({
        "id": repository.id,
        "owner": state.users.get(&repository.owner_id).map(|u| json!({"user_id": u.id, "github_login": u.login})),
        "name": repository.name,
        "repository_class": repository.class,
        "bundle_id": repository.bundle_id,
        "repository_fingerprint": if member { repository.repository_fingerprint.clone() } else { None },
        "github_url": repository.github_url,
        "catalog": null,
        "revision": repository.revision,
        "state": "active",
        "visibility": repository.visibility,
        "private_revision": repository.private_revision,
        "public_revision": if public { repository.private_revision } else { 0 },
        "role": role,
        "writer": writer,
        "private_head": if member { head.clone() } else { None },
        "public_head": if public { head } else { None },
        "private_summary": if member { tag.clone() } else { None },
        "public_summary": if public { tag } else { None },
        "open_release_request": if member { repository.release_requests.last().cloned() } else { None },
        "open_uploads": if member { open_uploads } else { vec![] },
    })
}

/// The validator's summary of a manifest (section 5.3).
fn tag_summary(manifest: &BundleManifest) -> Value {
    let providers = manifest
        .provider_scopes()
        .map(|(provider, _)| provider.as_str().to_owned())
        .collect::<Vec<_>>();
    json!({
        "display_name": manifest.bundle.display_name,
        "kind": "project",
        "revision": manifest.revision,
        "updated_at": manifest.updated_at,
        "resources": manifest.all_resources().count(),
        "files": manifest.all_files().count(),
        "conversations": manifest.all_conversations().count(),
        "registration_only": manifest.is_registration_only(),
        "providers": providers,
    })
}

fn parse_multipart(body: &[u8], content_type: &str) -> Option<BTreeMap<String, Vec<u8>>> {
    let boundary = content_type
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("boundary="))?
        .trim_matches('"');
    let marker = format!("--{boundary}");
    let mut parts = BTreeMap::new();
    let text = body;
    let mut cursor = 0;
    let find = |from: usize, needle: &[u8]| -> Option<usize> {
        text[from..]
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|position| position + from)
    };
    loop {
        let start = find(cursor, marker.as_bytes())?;
        let after = start + marker.len();
        if text[after..].starts_with(b"--") {
            break;
        }
        let headers_start = after + 2; // \r\n
        let headers_end = find(headers_start, b"\r\n\r\n")?;
        let headers = String::from_utf8_lossy(&text[headers_start..headers_end]).to_string();
        let name = headers
            .split(';')
            .map(str::trim)
            .find_map(|part| part.strip_prefix("name="))?
            .trim_matches('"')
            .to_owned();
        let body_start = headers_end + 4;
        let next = find(body_start, marker.as_bytes())?;
        let body_end = next.saturating_sub(2);
        parts.insert(name, text[body_start..body_end].to_vec());
        cursor = next;
    }
    Some(parts)
}

fn query(uri: &hyper::Uri, name: &str) -> Option<String> {
    uri.query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| value.to_owned())
    })
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
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_default();
    let declared_sha = request
        .headers()
        .get("x-mallard-sha256")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
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
        &content_type,
        declared_sha.as_deref(),
        body,
    );
    Ok(response)
}

fn segments(path: &str) -> Vec<&str> {
    path.trim_start_matches('/').split('/').collect()
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
    content_type: &str,
    declared_sha: Option<&str>,
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
    let user = state.users[&user_id].clone();
    if method == Method::GET && path == "/v1/me" {
        return json(
            StatusCode::OK,
            json!({"user": {"id": user.id, "github_login": user.login, "service_role": "user"}, "client": "desktop", "limits": {}}),
        );
    }
    if method == Method::GET && path == "/v1/repositories" {
        let rows = state
            .repositories
            .values()
            .filter(|r| r.members.contains_key(&user_id))
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
    if method == Method::POST && path == "/v1/repositories" {
        if idempotency.is_none() {
            return error("invalid_request", "Idempotency-Key is required.");
        }
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let class = value["repository_class"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
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
        if state
            .repositories
            .values()
            .any(|r| r.bundle_id == bundle_id && r.members.contains_key(&user_id))
        {
            return error("storage_managed_account_exists", "already registered");
        }
        let base = normalize_repository_name(value["name"].as_str().unwrap_or_default());
        let held: BTreeSet<String> = state
            .repositories
            .values()
            .filter(|r| r.owner_id == user_id)
            .map(|r| r.name.to_ascii_lowercase())
            .collect();
        let id = state.id("repo");
        let repository = Repository {
            id: id.clone(),
            class,
            name: pick_available_name(&base, &held),
            owner_id: user_id.clone(),
            bundle_id,
            repository_fingerprint: fingerprint,
            visibility: "private".to_owned(),
            github_url: None,
            revision: 1,
            private_revision: 0,
            private_head: None,
            publications: vec![],
            objects: BTreeMap::new(),
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
            .find(|r| r.invite_code.as_ref().is_some_and(|c| c.code == normalized))
            .map(|r| r.id.clone())
        else {
            return error("invitation_invalid", "The invitation code is unknown or was revoked.");
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
        return json(StatusCode::OK, summary(state, &repository, Some(&role), true));
    }
    if method == Method::GET
        && parts.len() == 5
        && parts[..3] == ["v1", "repositories", "by-bundle"]
        && parts[4] == "head"
    {
        // A member's repository first; otherwise a public one the bundle
        // was released under, whose head is readable by everyone.
        let Some(repository) = state
            .repositories
            .values()
            .find(|r| r.bundle_id == parts[3] && r.members.contains_key(&user_id))
            .or_else(|| {
                state.repositories.values().find(|r| {
                    r.bundle_id == parts[3] && r.visibility == "public" && r.private_head.is_some()
                })
            })
        else {
            return Reply(
                Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            );
        };
        let head = repository
            .private_head
            .as_ref()
            .map(|p| head_body(repository, p, &state.users));
        return json(
            StatusCode::OK,
            json!({"repository_id": repository.id, "head": head}),
        );
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
        let Some(repository) = state
            .repositories
            .values()
            .find(|r| r.owner_id == owner.id && r.name.eq_ignore_ascii_case(parts[4]))
        else {
            return error("not_found", "no such repository");
        };
        if let Some((_, role)) = repository.members.get(&user_id) {
            return json(StatusCode::OK, summary(state, repository, Some(role), true));
        }
        if repository.visibility == "public" && repository.private_head.is_some() {
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
        // Section 4.3: a collaborator, or anyone signed in on an open
        // repository (public with a released head), is a writer and reads and
        // pushes alike; everyone else sees nothing. The stub has no anonymous
        // callers and no Super Admin.
        let released = repository.visibility == "public" && repository.private_head.is_some();
        if role.is_none() && !released {
            return error("not_found", "not visible");
        }
        let manages = matches!(role.as_deref(), Some("owner") | Some("admin"));
        match (method, rest) {
            (&Method::GET, []) => {
                return json(StatusCode::OK, summary(state, &repository, role.as_deref(), true))
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
            (&Method::GET, ["manifests", commit_id]) => {
                let Some(publication) = repository
                    .publications
                    .iter()
                    .find(|p| p.commit_id == *commit_id)
                else {
                    return error("not_found", "no such publication");
                };
                return bytes_reply(
                    publication.manifest.clone(),
                    &publication.manifest_sha256,
                    "application/json",
                );
            }
            (&Method::GET, ["objects", sha256]) => {
                let Some(bytes) = repository.objects.get(*sha256) else {
                    return error("not_found", "no such object");
                };
                return bytes_reply(bytes.clone(), sha256, "application/octet-stream");
            }
            (&Method::GET, ["views", resource_id]) => {
                let commit = query(uri, "commit_id");
                let publication = match commit {
                    Some(commit) => repository
                        .publications
                        .iter()
                        .find(|p| p.commit_id == commit),
                    None => repository.private_head.as_ref(),
                };
                let Some((_, view_sha)) = publication.and_then(|p| p.views.get(*resource_id))
                else {
                    return error("not_found", "no such view");
                };
                let bytes = repository.objects[view_sha].clone();
                return bytes_reply(bytes, view_sha, "application/json");
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
                if repository.private_head.is_none() {
                    return error("release_request_invalid", "nothing to release");
                }
                let id = state.id("rr");
                let request = json!({"id": id, "state": "pending_owner", "requested_by": user_id, "created_at": now()});
                let repository = state.repositories.get_mut(&repository_id).unwrap();
                repository.release_requests.push(request.clone());
                repository.revision += 1;
                return json(StatusCode::CREATED, request);
            }
            (&Method::POST, ["uploads"]) => {
                if idempotency.is_none() {
                    return error("invalid_request", "Idempotency-Key is required.");
                }
                let Some(form) = parse_multipart(&body, content_type) else {
                    return error("invalid_request", "multipart");
                };
                let (Some(request), Some(manifest)) = (form.get("request"), form.get("manifest"))
                else {
                    return error("invalid_request", "parts");
                };
                let request: Value = serde_json::from_slice(request).unwrap_or(Value::Null);
                let manifest_bytes = Bytes::from(manifest.clone());
                let parsed: BundleManifest = match serde_json::from_slice(manifest) {
                    Ok(parsed) => parsed,
                    Err(e) => return error("manifest_invalid", &format!("parse: {e}")),
                };
                if parsed.bundle.bundle_id.as_str() != repository.bundle_id {
                    return error("manifest_invalid", "identity mismatch");
                }
                if (repository.class == "git_project")
                    != parsed.bundle.repository_fingerprint.is_some()
                    || parsed.bundle.repository_fingerprint != repository.repository_fingerprint
                {
                    return error("manifest_invalid", "class mismatch");
                }
                let mut inventory = Vec::new();
                for entry in request["objects"].as_array().cloned().unwrap_or_default() {
                    inventory.push(InventoryEntry {
                        sha256: entry["sha256"].as_str().unwrap_or_default().to_owned(),
                        size: entry["size"].as_u64().unwrap_or_default(),
                        role: entry["role"].as_str().unwrap_or_default().to_owned(),
                        resource_id: entry["resource_id"].as_str().map(str::to_owned),
                        source_sha256: entry["source_sha256"].as_str().map(str::to_owned),
                    });
                }
                let references = parsed
                    .all_files()
                    .map(|(_, entry)| (entry.sha256.clone(), entry.size))
                    .collect::<BTreeMap<_, _>>();
                let conversations = parsed
                    .provider_scopes()
                    .flat_map(|(_, scope)| {
                        scope.conversations.values().filter_map(|thread| {
                            let transcript = thread
                                .files
                                .iter()
                                .find(|f| f.role == ConversationFileRole::Transcript)?;
                            let entry = scope.files.get(&transcript.logical_path)?;
                            Some((thread.resource_id.as_str().to_owned(), entry.sha256.clone()))
                        })
                    })
                    .collect::<BTreeMap<_, _>>();
                let payloads = inventory
                    .iter()
                    .filter(|e| e.role == "payload")
                    .map(|e| (e.sha256.clone(), e.size))
                    .collect::<BTreeMap<_, _>>();
                if payloads != references {
                    return error(
                        "manifest_invalid",
                        "missing reference: inventory and manifest references differ",
                    );
                }
                for view in inventory.iter().filter(|e| e.role == "view") {
                    let Some(resource_id) = &view.resource_id else {
                        return error("invalid_request", "view resource_id");
                    };
                    if conversations.get(resource_id) != view.source_sha256.as_ref() {
                        return error(
                            "manifest_invalid",
                            "view cites a transcript digest that differs from the manifest",
                        );
                    }
                }
                if request["expected_repository_revision"].as_u64() != Some(repository.revision) {
                    return error("revision_conflict", "repository version");
                }
                if request["expected_private_revision"].as_u64()
                    != Some(repository.private_revision)
                {
                    return error("head_changed", "private version");
                }
                // Uploads run side by side; only a member's own count is capped.
                if state
                    .uploads
                    .values()
                    .filter(|u| {
                        u.repository_id == repository_id
                            && u.user_id == user_id
                            && u.state == "open"
                            && u.expires_at > now()
                    })
                    .count()
                    >= OPEN_UPLOADS_PER_MEMBER
                {
                    return error(
                        "upload_in_progress",
                        "too many of this member's uploads are open",
                    );
                }
                if repository
                    .publications
                    .iter()
                    .any(|p| p.commit_id == parsed.commit_id)
                    || state.uploads.values().any(|u| {
                        u.repository_id == repository_id
                            && u.commit_id == parsed.commit_id
                            && u.state != "abandoned"
                            && u.state != "expired"
                    })
                {
                    return error("head_changed", "commit id already used");
                }
                let id = state.id("upl");
                let missing = inventory
                    .iter()
                    .filter(|e| !repository.objects.contains_key(&e.sha256))
                    .map(|e| json!({"sha256": e.sha256, "size": e.size, "role": e.role}))
                    .collect::<Vec<_>>();
                let received = inventory
                    .iter()
                    .filter(|e| repository.objects.contains_key(&e.sha256))
                    .map(|e| e.sha256.clone())
                    .collect();
                let expires_at = now() + 24 * 3600;
                state.uploads.insert(
                    id.clone(),
                    Upload {
                        id: id.clone(),
                        repository_id: repository_id.clone(),
                        user_id: user_id.clone(),
                        state: "open".into(),
                        expected_repository_revision: repository.revision,
                        expected_private_revision: repository.private_revision,
                        commit_id: parsed.commit_id.clone(),
                        manifest: manifest_bytes.clone(),
                        manifest_revision: parsed.revision,
                        summary: tag_summary(&parsed),
                        inventory,
                        received,
                        expires_at,
                        created_at: now(),
                    },
                );
                return json(
                    StatusCode::CREATED,
                    json!({"upload_id": id, "repository_id": repository_id, "commit_id": parsed.commit_id, "manifest_sha256": sha256_hex(&manifest_bytes), "manifest_size": manifest_bytes.len(), "expires_at": expires_at, "reserved_bytes": 0, "missing": missing}),
                );
            }
            (&Method::PUT, ["uploads", upload_id, "objects", sha256]) => {
                let Some(upload) = state
                    .uploads
                    .get_mut(*upload_id)
                    .filter(|u| u.repository_id == repository_id)
                else {
                    return error("not_found", "no such upload");
                };
                if upload.state != "open" || upload.expires_at <= now() {
                    return error("upload_expired", "closed");
                }
                if declared_sha != Some(*sha256) {
                    return error("invalid_request", "X-Mallard-Sha256");
                }
                let Some(entry) = upload
                    .inventory
                    .iter()
                    .find(|e| e.sha256 == *sha256)
                    .cloned()
                else {
                    return error("invalid_request", "not in inventory");
                };
                if body.len() as u64 != entry.size || sha256_hex(&body) != *sha256 {
                    return error("object_integrity_failed", "digest or size");
                }
                if entry.role == "view" {
                    let value: Value = match serde_json::from_slice(&body) {
                        Ok(value) => value,
                        Err(_) => return error("manifest_invalid", "view is not JSON"),
                    };
                    if value["schema_version"] != 1
                        || value["resource_id"].as_str() != entry.resource_id.as_deref()
                        || !value["events"].is_array()
                    {
                        return error("manifest_invalid", "view shape");
                    }
                }
                upload.received.insert(sha256.to_string());
                let repository = state.repositories.get_mut(&repository_id).unwrap();
                repository.objects.insert(sha256.to_string(), body);
                return json(
                    StatusCode::OK,
                    json!({"sha256": sha256, "size": entry.size, "state": "ready"}),
                );
            }
            (&Method::POST, ["uploads", upload_id, "commit"]) => {
                if idempotency.is_none() {
                    return error("invalid_request", "Idempotency-Key is required.");
                }
                let value: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
                let expected = value["expected_commit_id"].as_str().map(str::to_owned);
                let Some(upload) = state
                    .uploads
                    .get(*upload_id)
                    .filter(|u| u.repository_id == repository_id)
                    .cloned()
                else {
                    return error("not_found", "no such upload");
                };
                if upload.state != "open" || upload.expires_at <= now() {
                    return error("upload_expired", "closed");
                }
                if let Some(missing) = upload
                    .inventory
                    .iter()
                    .find(|e| !upload.received.contains(&e.sha256))
                {
                    return error(
                        "invalid_request",
                        &format!("Objects are not ready: {}", missing.sha256),
                    );
                }
                if upload.expected_repository_revision != repository.revision {
                    return error("revision_conflict", "version moved");
                }
                // The service's private_revision guard is the divergence signal.
                if upload.expected_private_revision != repository.private_revision {
                    return error("head_changed", "the private head moved");
                }
                let current = repository
                    .private_head
                    .as_ref()
                    .map(|p| p.commit_id.clone());
                if std::mem::take(&mut state.fail_next_commit) || current != expected {
                    return error("head_changed", "the private head moved");
                }
                let parsed: BundleManifest = serde_json::from_slice(&upload.manifest).unwrap();
                let conversations = parsed
                    .provider_scopes()
                    .flat_map(|(_, scope)| {
                        scope.conversations.values().filter_map(|thread| {
                            let transcript = thread
                                .files
                                .iter()
                                .find(|f| f.role == ConversationFileRole::Transcript)?;
                            let entry = scope.files.get(&transcript.logical_path)?;
                            Some((thread.resource_id.as_str().to_owned(), entry.sha256.clone()))
                        })
                    })
                    .collect::<BTreeMap<_, _>>();
                let mut views = BTreeMap::new();
                for entry in upload.inventory.iter().filter(|e| e.role == "view") {
                    views.insert(
                        entry.resource_id.clone().unwrap(),
                        (entry.source_sha256.clone().unwrap(), entry.sha256.clone()),
                    );
                }
                // Carry forward the previous head's views whose transcript is unchanged.
                if let Some(previous) = &repository.private_head {
                    for (resource_id, (source, view)) in &previous.views {
                        if !views.contains_key(resource_id)
                            && conversations.get(resource_id) == Some(source)
                        {
                            views.insert(resource_id.clone(), (source.clone(), view.clone()));
                        }
                    }
                }
                let publication = Publication {
                    commit_id: upload.commit_id.clone(),
                    manifest: upload.manifest.clone(),
                    manifest_sha256: sha256_hex(&upload.manifest),
                    manifest_size: upload.manifest.len() as u64,
                    manifest_revision: upload.manifest_revision,
                    summary: upload.summary.clone(),
                    views,
                    created_by: user_id.clone(),
                    created_at: now(),
                };
                let repository = state.repositories.get_mut(&repository_id).unwrap();
                repository.publications.push(publication.clone());
                repository.private_head = Some(publication.clone());
                repository.private_revision += 1;
                repository.release_requests.clear();
                state.uploads.get_mut(*upload_id).unwrap().state = "committed".into();
                let repository = state.repositories[&repository_id].clone();
                return json(
                    StatusCode::OK,
                    json!({"head": head_body(&repository, &publication, &state.users), "repository_revision": repository.revision}),
                );
            }
            (&Method::POST, ["uploads", upload_id, "abandon"]) => {
                let Some(upload) = state
                    .uploads
                    .get_mut(*upload_id)
                    .filter(|u| u.repository_id == repository_id)
                else {
                    return error("not_found", "no such upload");
                };
                if upload.state == "open" {
                    upload.state = "abandoned".into();
                }
                return json(
                    StatusCode::OK,
                    json!({"ok": true, "upload_id": upload_id, "state": upload.state}),
                );
            }
            _ => {}
        }
    }
    error("not_found", "No such route.")
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
            .body(Full::new(bytes))
            .unwrap(),
    )
}
