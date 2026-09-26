//! Stateful loopback Feishu for tests: Drive, `user_info`, and the OAuth v3
//! token endpoint with single-use rotating refresh tokens. It never
//! connects to Feishu, and it deliberately permits two children of one
//! name, as Feishu does. Every credential here is synthetic.
use super::*;
use crate::feishu_auth::{FeishuApp, FeishuClient, CALLBACK_URI, SCOPES};
use crate::feishu_connections::{
    ConnectionId, FeishuConnection, FeishuConnectionStore, FEISHU_CONNECTION_SCHEMA,
};
use crate::persistence::LocalStateRepository;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response as HttpResponse};
use hyper_util::rt::TokioIo;
use mallard_domain::{now_secs, SecretValue};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Node {
    pub token: String,
    pub parent: String,
    pub name: String,
    pub kind: &'static str,
    pub bytes: Vec<u8>,
}

struct Session {
    parent: String,
    name: String,
    size: u64,
    parts: Vec<Vec<u8>>,
}

/// What happens to a probe file right after `upload_all` stored it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadFault {
    /// One byte of the stored file is flipped.
    Corrupt,
    /// The stored node disappears.
    Drop,
    /// The stored node is kept but left out of every listing.
    Hide,
}

pub struct State {
    /// Every node by token; tokens are zero-padded so the map is creation
    /// order, which is the listing order.
    pub nodes: BTreeMap<String, Node>,
    sequence: u64,
    pub requests: usize,
    pub routes: BTreeMap<String, usize>,
    /// One entry per folder-list page, for traversal assertions.
    pub listed_folders: Vec<String>,
    /// Serve pages of at most this many entries; 0 honors the request.
    pub page_size: usize,
    pub hidden: BTreeSet<String>,
    sessions: BTreeMap<String, Session>,
    pub children_limit: usize,
    pub block_size: u64,
    pub upload_bodies: Vec<(String, usize)>,
    // Tokens
    pub access_tokens: BTreeSet<String>,
    refresh_tokens: BTreeSet<String>,
    used_refresh: BTreeSet<String>,
    issued: u64,
    pub expires_in: u64,
    pub refresh_expires_in: u64,
    pub scope_override: Option<String>,
    pub deny_refresh: bool,
    /// Rotate, then answer with a server error.
    pub lose_refresh_answer_once: bool,
    /// Answer with a server error without rotating.
    pub drop_refresh_before_rotate_once: bool,
    /// Answer the next Drive request 401 with the expired-token code, once.
    pub expire_once: bool,
    /// Answer every Drive request 401.
    pub reject_tokens: bool,
    /// Answer the next `upload_all` 401 with the expired-token code, once.
    pub expire_upload_once: bool,
    pub rate_limit_once: bool,
    /// Store the file, then answer with a server error.
    pub lose_upload_answer_once: bool,
    /// Answer with a server error without storing.
    pub fail_upload_once: bool,
    /// Store the file and acknowledge it, then apply this fault, once.
    pub upload_fault_once: Option<UploadFault>,
    pub response_status: Option<u16>,
    pub response_once: Option<(&'static str, Value)>,
    /// Raw part acknowledgement after accepting the bytes; defaults to `data: null`.
    pub upload_part_response: Option<String>,
    readonly_documents_on_refresh: bool,
    /// Whether the connection document read `refresh_uncertain: true`
    /// when each refresh request arrived.
    pub uncertain_at_refresh: Vec<bool>,
    documents: Option<(PathBuf, ConnectionId)>,
}

impl State {
    fn add(&mut self, parent: &str, name: &str, kind: &'static str, bytes: Vec<u8>) -> String {
        self.sequence += 1;
        let token = format!("node{:06}", self.sequence);
        self.nodes.insert(
            token.clone(),
            Node {
                token: token.clone(),
                parent: parent.to_owned(),
                name: name.to_owned(),
                kind,
                bytes,
            },
        );
        token
    }
    fn children(&self, parent: &str) -> Vec<&Node> {
        self.nodes
            .values()
            .filter(|node| node.parent == parent && !self.hidden.contains(&node.token))
            .collect()
    }
}

pub struct MockFeishuServer {
    pub state: Arc<Mutex<State>>,
    pub store: FeishuObjectStore,
    pub connections: FeishuConnectionStore,
    pub connection_id: ConnectionId,
    origin: String,
    runtime: tokio::runtime::Runtime,
    _temp: tempfile::TempDir,
}

/// The storage folder every mock store opens.
pub const ROOT: &str = "rootfolder";
/// Feishu's My Space, reached with an empty parent token.
pub const MY_SPACE: &str = "myspace";

fn reply(status: u16, value: Value) -> HttpResponse<Full<Bytes>> {
    HttpResponse::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8")
        .header("x-tt-logid", "req-fixture")
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}
fn ok(data: Value) -> HttpResponse<Full<Bytes>> {
    reply(200, json!({"code": 0, "msg": "success", "data": data}))
}
fn api_error(status: u16, code: u64) -> HttpResponse<Full<Bytes>> {
    reply(
        status,
        json!({"code": code, "msg": "fixture-msg-must-never-escape"}),
    )
}
fn token_error(status: u16, code: u64, error: &str) -> HttpResponse<Full<Bytes>> {
    let mut response = reply(
        status,
        json!({"code": code, "error": error, "error_description": "secret-canary-description"}),
    );
    response
        .headers_mut()
        .insert("x-tt-logid", "req-token-1".parse().unwrap());
    response
}

/// Decode one `multipart/form-data` body into `(name, filename, bytes)`.
pub(crate) fn parse_multipart(
    content_type: &str,
    body: &[u8],
) -> Vec<(String, Option<String>, Vec<u8>)> {
    let boundary = content_type
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("boundary="))
        .expect("multipart boundary");
    let delimiter = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut rest = body;
    // Skip the preamble up to the first delimiter.
    let first = find(rest, delimiter.as_bytes()).expect("first delimiter");
    rest = &rest[first + delimiter.len()..];
    loop {
        if rest.starts_with(b"--") {
            break;
        }
        rest = rest.strip_prefix(b"\r\n").unwrap_or(rest);
        let header_end = find(rest, b"\r\n\r\n").expect("part headers");
        let headers = std::str::from_utf8(&rest[..header_end]).unwrap();
        let mut name = None;
        let mut filename = None;
        for line in headers.split("\r\n") {
            if let Some(value) = line
                .strip_prefix("Content-Disposition:")
                .or_else(|| line.strip_prefix("content-disposition:"))
            {
                for attribute in value.split(';').map(str::trim) {
                    if let Some(v) = attribute.strip_prefix("name=") {
                        name = Some(v.trim_matches('"').to_owned());
                    }
                    if let Some(v) = attribute.strip_prefix("filename=") {
                        filename = Some(v.trim_matches('"').to_owned());
                    }
                }
            }
        }
        let content = &rest[header_end + 4..];
        let next = find(content, format!("\r\n{delimiter}").as_bytes()).expect("next delimiter");
        parts.push((name.expect("part name"), filename, content[..next].to_vec()));
        rest = &content[next + 2 + delimiter.len()..];
    }
    parts
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn read_document(documents: &Option<(PathBuf, ConnectionId)>) -> Option<FeishuConnection> {
    let (root, id) = documents.as_ref()?;
    let repository = LocalStateRepository::from_home_dir(root.parent()?).ok()?;
    FeishuConnectionStore::new(&repository)
        .load(id)
        .ok()
        .flatten()
}

async fn serve(
    request: Request<Incoming>,
    shared: Arc<Mutex<State>>,
) -> std::result::Result<HttpResponse<Full<Bytes>>, std::convert::Infallible> {
    let (parts, body) = request.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let path = parts.uri.path().to_owned();
    let query: BTreeMap<String, String> = parts
        .uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();
    let mut state = shared.lock().unwrap();
    state.requests += 1;
    if path == "/oauth/v3/token" {
        assert!(!parts.headers.contains_key("authorization"));
        let fields: BTreeMap<String, String> =
            url::form_urlencoded::parse(&bytes).into_owned().collect();
        let refresh = fields.get("grant_type").map(String::as_str) == Some("refresh_token");
        *state
            .routes
            .entry(if refresh { "refresh" } else { "exchange" }.into())
            .or_default() += 1;
        if fields.get("client_id").map(String::as_str) != Some(MockFeishuServer::APP_ID)
            || fields.get("client_secret").map(String::as_str) != Some(MockFeishuServer::APP_SECRET)
        {
            return Ok(token_error(400, 20011, "invalid_client"));
        }
        let (scope, expires_in, refresh_expires_in) = (
            state
                .scope_override
                .clone()
                .unwrap_or_else(|| SCOPES.to_owned()),
            state.expires_in,
            state.refresh_expires_in,
        );
        if refresh {
            let uncertain = read_document(&state.documents)
                .map(|document| document.refresh_uncertain)
                .unwrap_or(false);
            state.uncertain_at_refresh.push(uncertain);
            let presented = fields.get("refresh_token").cloned().unwrap_or_default();
            if state.used_refresh.contains(&presented) {
                return Ok(token_error(400, 20073, "invalid_grant"));
            }
            if !state.refresh_tokens.contains(&presented) {
                return Ok(token_error(400, 20072, "invalid_grant"));
            }
            if state.deny_refresh {
                return Ok(token_error(400, 20072, "invalid_grant"));
            }
            if std::mem::take(&mut state.drop_refresh_before_rotate_once) {
                return Ok(token_error(503, 20050, "server_error"));
            }
            state.refresh_tokens.remove(&presented);
            state.used_refresh.insert(presented);
            state.issued += 1;
            let (access, next) = (
                format!("access-{}", state.issued),
                format!("refresh-{}", state.issued),
            );
            state.access_tokens.insert(access.clone());
            state.refresh_tokens.insert(next.clone());
            if std::mem::take(&mut state.readonly_documents_on_refresh) {
                if let Some((root, id)) = &state.documents {
                    let path = root.join("feishu_connections").join(format!("{id}.json"));
                    std::fs::rename(&path, path.with_extension("held")).unwrap();
                    std::fs::create_dir(&path).unwrap();
                }
            }
            if std::mem::take(&mut state.lose_refresh_answer_once) {
                return Ok(token_error(503, 20050, "server_error"));
            }
            return Ok(reply(
                200,
                json!({"code": 0, "access_token": access, "token_type": "Bearer", "expires_in": expires_in,
                       "refresh_token": next, "refresh_token_expires_in": refresh_expires_in, "scope": scope}),
            ));
        }
        assert_eq!(fields["redirect_uri"], CALLBACK_URI);
        assert!(!fields["code_verifier"].is_empty());
        if fields.get("code").map(String::as_str) != Some("callback-code") {
            return Ok(token_error(400, 20072, "invalid_grant"));
        }
        state.issued += 1;
        let (access, next) = (
            format!("access-{}", state.issued),
            format!("refresh-{}", state.issued),
        );
        state.access_tokens.insert(access.clone());
        state.refresh_tokens.insert(next.clone());
        return Ok(reply(
            200,
            json!({"code": 0, "access_token": access, "token_type": "Bearer", "expires_in": expires_in,
                   "refresh_token": next, "refresh_token_expires_in": refresh_expires_in, "scope": scope}),
        ));
    }
    let route: &str = if path == "/open-apis/drive/v1/files" {
        "files"
    } else if path == "/open-apis/authen/v1/user_info" {
        "user_info"
    } else if path.ends_with("/download") {
        "download"
    } else {
        path.rsplit('/').next().unwrap()
    };
    *state.routes.entry(route.to_owned()).or_default() += 1;
    let presented = parts
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    if !state.access_tokens.contains(presented)
        || state.reject_tokens
        || std::mem::take(&mut state.expire_once)
        || (route == "upload_all" && std::mem::take(&mut state.expire_upload_once))
    {
        return Ok(api_error(401, 99991663));
    }
    if let Some(status) = state.response_status {
        let mut response = reply(status, json!({"private_response": "must never escape"}));
        response
            .headers_mut()
            .insert("location", "/never-follow".parse().unwrap());
        return Ok(response);
    }
    if std::mem::take(&mut state.rate_limit_once) {
        return Ok(api_error(429, 99991400));
    }
    if state
        .response_once
        .as_ref()
        .is_some_and(|(target, _)| *target == route)
    {
        let (_, body) = state.response_once.take().unwrap();
        return Ok(reply(200, body));
    }
    let content_type = parts
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let response = match route {
        "user_info" => ok(
            json!({"name": "Test User", "en_name": "Test User", "open_id": "ou_fixture", "user_id": "u-fixture"}),
        ),
        "files" => {
            let parent = match query.get("folder_token").map(String::as_str) {
                Some("") | None => MY_SPACE,
                Some(token) => token,
            };
            state.listed_folders.push(parent.to_owned());
            assert_eq!(query["order_by"], "CreatedTime");
            assert_eq!(query["direction"], "ASC");
            if state
                .nodes
                .get(parent)
                .is_none_or(|node| node.kind != "folder")
            {
                return Ok(api_error(404, 1061003));
            }
            let requested: usize = query["page_size"].parse().unwrap();
            assert!((1..=200).contains(&requested));
            let size = if state.page_size > 0 {
                requested.min(state.page_size)
            } else {
                requested
            };
            let start: usize = query
                .get("page_token")
                .map(|token| token.parse().unwrap())
                .unwrap_or(0);
            let children = state.children(parent);
            let end = (start + size).min(children.len());
            let has_more = end < children.len();
            let files: Vec<Value> = children[start..end]
                .iter()
                .map(|node| {
                    json!({"token": node.token, "name": node.name, "type": node.kind, "parent_token": node.parent,
                           "url": "https://example.invalid/not-followed"})
                })
                .collect();
            let mut data = json!({"files": files, "has_more": has_more});
            if has_more {
                data["next_page_token"] = json!(end.to_string());
            }
            ok(data)
        }
        "create_folder" => {
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let parent = match body["folder_token"].as_str().unwrap() {
                "" => MY_SPACE.to_owned(),
                token => token.to_owned(),
            };
            if state
                .nodes
                .get(&parent)
                .is_none_or(|node| node.kind != "folder")
            {
                return Ok(api_error(404, 1061003));
            }
            if state.children(&parent).len() >= state.children_limit {
                return Ok(api_error(400, 1062507));
            }
            let token = state.add(&parent, body["name"].as_str().unwrap(), "folder", vec![]);
            ok(json!({"token": token, "url": "https://example.invalid/not-followed"}))
        }
        "upload_all" => {
            let form = parse_multipart(&content_type, &bytes);
            let field = |name: &str| -> String {
                String::from_utf8(
                    form.iter()
                        .find(|(n, _, _)| n == name)
                        .unwrap_or_else(|| panic!("form field {name}"))
                        .2
                        .clone(),
                )
                .unwrap()
            };
            let mut names: Vec<&str> = form.iter().map(|(n, _, _)| n.as_str()).collect();
            names.sort_unstable();
            assert_eq!(
                names,
                ["file", "file_name", "parent_node", "parent_type", "size"]
            );
            assert_eq!(field("parent_type"), "explorer");
            let file = form.iter().find(|(n, _, _)| n == "file").unwrap();
            assert_eq!(file.1.as_deref(), Some(field("file_name").as_str()));
            assert_eq!(file.2.len().to_string(), field("size"));
            assert!(file.2.len() as u64 <= SMALL_UPLOAD_LIMIT);
            let parent = field("parent_node");
            let name = field("file_name");
            state.upload_bodies.push((name.clone(), file.2.len()));
            if state
                .nodes
                .get(&parent)
                .is_none_or(|node| node.kind != "folder")
            {
                return Ok(api_error(404, 1061003));
            }
            if state.children(&parent).len() >= state.children_limit {
                return Ok(api_error(400, 1062507));
            }
            if std::mem::take(&mut state.fail_upload_once) {
                return Ok(reply(500, json!({"private_response": "must never escape"})));
            }
            let token = state.add(&parent, &name, "file", file.2.clone());
            if std::mem::take(&mut state.lose_upload_answer_once) {
                return Ok(reply(503, json!({"private_response": "must never escape"})));
            }
            match state.upload_fault_once.take() {
                Some(UploadFault::Corrupt) => {
                    state.nodes.get_mut(&token).unwrap().bytes[0] ^= 0xff;
                }
                Some(UploadFault::Drop) => {
                    state.nodes.remove(&token);
                }
                Some(UploadFault::Hide) => {
                    state.hidden.insert(token.clone());
                }
                None => {}
            }
            ok(json!({"file_token": token}))
        }
        "upload_prepare" => {
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["parent_type"], "explorer");
            let parent = body["parent_node"].as_str().unwrap().to_owned();
            if state
                .nodes
                .get(&parent)
                .is_none_or(|node| node.kind != "folder")
            {
                return Ok(api_error(404, 1061003));
            }
            let size = body["size"].as_u64().unwrap();
            state.sequence += 1;
            let upload_id = format!("upload{:06}", state.sequence);
            let block_size = state.block_size;
            state.sessions.insert(
                upload_id.clone(),
                Session {
                    parent,
                    name: body["file_name"].as_str().unwrap().to_owned(),
                    size,
                    parts: Vec::new(),
                },
            );
            ok(
                json!({"upload_id": upload_id, "block_size": block_size, "block_num": size.div_ceil(block_size)}),
            )
        }
        "upload_part" => {
            let form = parse_multipart(&content_type, &bytes);
            let field = |name: &str| -> String {
                String::from_utf8(form.iter().find(|(n, _, _)| n == name).unwrap().2.clone())
                    .unwrap()
            };
            let file = form.iter().find(|(n, _, _)| n == "file").unwrap().2.clone();
            assert_eq!(file.len().to_string(), field("size"));
            let block_size = state.block_size;
            let session = state
                .sessions
                .get_mut(&field("upload_id"))
                .expect("upload session");
            assert_eq!(field("seq"), session.parts.len().to_string());
            assert!(!file.is_empty() && file.len() as u64 <= block_size);
            session.parts.push(file);
            match &state.upload_part_response {
                Some(body) => HttpResponse::builder()
                    .status(200)
                    .header("content-type", "application/json; charset=utf-8")
                    .body(Full::new(Bytes::from(body.clone())))
                    .unwrap(),
                None => ok(Value::Null),
            }
        }
        "upload_finish" => {
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let session = state
                .sessions
                .remove(body["upload_id"].as_str().unwrap())
                .expect("upload session");
            assert_eq!(
                body["block_num"].as_u64().unwrap() as usize,
                session.parts.len()
            );
            let joined: Vec<u8> = session.parts.concat();
            assert_eq!(joined.len() as u64, session.size);
            state
                .upload_bodies
                .push((session.name.clone(), joined.len()));
            let token = state.add(&session.parent, &session.name, "file", joined);
            ok(json!({"file_token": token}))
        }
        "download" => {
            let token = path
                .trim_start_matches("/open-apis/drive/v1/files/")
                .trim_end_matches("/download");
            match state.nodes.get(token) {
                Some(node) if node.kind == "file" => HttpResponse::builder()
                    .status(200)
                    .header(
                        "content-type",
                        if node.name.ends_with(".json") {
                            "application/json; charset=utf-8"
                        } else {
                            "application/octet-stream"
                        },
                    )
                    .body(Full::new(Bytes::from(node.bytes.clone())))
                    .unwrap(),
                _ => api_error(404, 1061003),
            }
        }
        other => panic!("unexpected route {other}"),
    };
    Ok(response)
}

impl MockFeishuServer {
    pub const APP_ID: &'static str = "cli_test";
    pub const APP_SECRET: &'static str = "secret-canary";
    /// The app every mock store signs in with: what a build carries.
    pub fn app() -> FeishuApp {
        FeishuApp::new(Self::APP_ID, SecretValue::parse(Self::APP_SECRET).unwrap()).unwrap()
    }
    /// The refresh token the connection document starts with.
    pub const FIRST_REFRESH: &'static str = "refresh-1";

    pub fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let connections = FeishuConnectionStore::new(&repository);
        let connection_id = ConnectionId::generate().unwrap();
        connections
            .save(&FeishuConnection {
                schema: FEISHU_CONNECTION_SCHEMA,
                connection_id: connection_id.clone(),
                refresh_token: SecretValue::parse(Self::FIRST_REFRESH).unwrap(),
                reauthorize_at: now_secs() + 365 * 86_400,
                refresh_uncertain: false,
                updated_at: now_secs(),
            })
            .unwrap();
        let mut state = State {
            nodes: BTreeMap::new(),
            sequence: 0,
            requests: 0,
            routes: BTreeMap::new(),
            listed_folders: Vec::new(),
            page_size: 0,
            hidden: BTreeSet::new(),
            sessions: BTreeMap::new(),
            children_limit: MAX_FOLDER_CHILDREN,
            block_size: 4 * 1024 * 1024,
            upload_bodies: Vec::new(),
            access_tokens: BTreeSet::from(["access-0".to_owned()]),
            refresh_tokens: BTreeSet::from([Self::FIRST_REFRESH.to_owned()]),
            used_refresh: BTreeSet::new(),
            issued: 1,
            expires_in: 7200,
            refresh_expires_in: 365 * 86_400,
            scope_override: None,
            deny_refresh: false,
            lose_refresh_answer_once: false,
            drop_refresh_before_rotate_once: false,
            expire_once: false,
            reject_tokens: false,
            expire_upload_once: false,
            rate_limit_once: false,
            lose_upload_answer_once: false,
            fail_upload_once: false,
            upload_fault_once: None,
            response_status: None,
            response_once: None,
            upload_part_response: None,
            readonly_documents_on_refresh: false,
            uncertain_at_refresh: Vec::new(),
            documents: Some((repository.root().to_path_buf(), connection_id.clone())),
        };
        state.nodes.insert(
            MY_SPACE.into(),
            Node {
                token: MY_SPACE.into(),
                parent: String::new(),
                name: "My Space".into(),
                kind: "folder",
                bytes: vec![],
            },
        );
        state.nodes.insert(
            ROOT.into(),
            Node {
                token: ROOT.into(),
                parent: MY_SPACE.into(),
                name: "Mallard fixture".into(),
                kind: "folder",
                bytes: vec![],
            },
        );
        let state = Arc::new(Mutex::new(state));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let shared = state.clone();
        runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
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
        let client = FeishuClient::for_test(&origin, &format!("{origin}/oauth/v3/token"));
        let store = FeishuObjectStore::with_origin(
            ROOT.into(),
            FeishuAuthorization::new(
                Self::app(),
                connection_id.clone(),
                connections.clone(),
                client,
            ),
            runtime.handle().clone(),
            &origin,
        )
        .unwrap();
        Self {
            state,
            store,
            connections,
            connection_id,
            origin,
            runtime,
            _temp: temp,
        }
    }

    pub fn client(&self) -> FeishuClient {
        FeishuClient::for_test(&self.origin, &self.token_url())
    }
    pub fn token_url(&self) -> String {
        format!("{}/oauth/v3/token", self.origin)
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
    pub fn runtime(&self) -> &Handle {
        self.runtime.handle()
    }
    pub fn count(&self, route: &str) -> usize {
        *self.state.lock().unwrap().routes.get(route).unwrap_or(&0)
    }
    pub fn requests(&self) -> usize {
        self.state.lock().unwrap().requests
    }
    /// An access token the mock accepts without a refresh.
    pub fn access_token(&self) -> SecretValue {
        SecretValue::parse("access-0").unwrap()
    }
    /// The local state root the connection documents live under.
    pub fn state_root(&self) -> PathBuf {
        self.state
            .lock()
            .unwrap()
            .documents
            .as_ref()
            .unwrap()
            .0
            .clone()
    }
    pub fn document_path(&self) -> PathBuf {
        self.state_root()
            .join("feishu_connections")
            .join(format!("{}.json", self.connection_id))
    }
    pub fn uncertain_at_refreshes(&self) -> Vec<bool> {
        self.state.lock().unwrap().uncertain_at_refresh.clone()
    }
    /// After the next refresh answer, the document cannot be written until
    /// `restore_documents`, which puts the pre-answer document back.
    pub fn fail_document_write_after_next_refresh(&self) {
        self.state.lock().unwrap().readonly_documents_on_refresh = true;
    }
    pub fn restore_documents(&self) {
        let path = self.document_path();
        let _ = std::fs::remove_dir(&path);
        let _ = std::fs::rename(path.with_extension("held"), &path);
    }
    /// A second sign-in with its own document and refresh chain, sharing
    /// the same folder: what another computer holds.
    pub fn second_store(&self) -> FeishuObjectStore {
        let connection_id = ConnectionId::generate().unwrap();
        self.connections
            .save(&FeishuConnection {
                schema: FEISHU_CONNECTION_SCHEMA,
                connection_id: connection_id.clone(),
                refresh_token: SecretValue::parse("refresh-second").unwrap(),
                reauthorize_at: now_secs() + 365 * 86_400,
                refresh_uncertain: false,
                updated_at: now_secs(),
            })
            .unwrap();
        self.state
            .lock()
            .unwrap()
            .refresh_tokens
            .insert("refresh-second".into());
        FeishuObjectStore::with_origin(
            ROOT.into(),
            FeishuAuthorization::new(
                Self::app(),
                connection_id,
                self.connections.clone(),
                self.client(),
            ),
            self.runtime.handle().clone(),
            &self.origin,
        )
        .unwrap()
    }
    /// Plant a folder or file directly, as another writer or a person
    /// dropping something in would.
    pub fn add_folder(&self, parent: &str, name: &str) -> String {
        self.state
            .lock()
            .unwrap()
            .add(parent, name, "folder", vec![])
    }
    pub fn add_file(&self, parent: &str, name: &str, bytes: &[u8]) -> String {
        self.state
            .lock()
            .unwrap()
            .add(parent, name, "file", bytes.to_vec())
    }
    /// The folder token(s) at a physical path below the storage folder.
    pub fn folders_at(&self, path: &str) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut current = vec![ROOT.to_owned()];
        for part in path.split('/') {
            current = current
                .iter()
                .flat_map(|parent| {
                    state
                        .children(parent)
                        .into_iter()
                        .filter(|node| node.kind == "folder" && node.name == part)
                        .map(|node| node.token.clone())
                        .collect::<Vec<_>>()
                })
                .collect();
        }
        current
    }
    /// Every file's physical path below the storage folder.
    pub fn file_paths(&self) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut paths = Vec::new();
        for node in state.nodes.values() {
            if node.kind != "file" {
                continue;
            }
            let mut parts = vec![node.name.clone()];
            let mut parent = node.parent.clone();
            while parent != ROOT && parent != MY_SPACE {
                let folder = &state.nodes[&parent];
                parts.push(folder.name.clone());
                parent = folder.parent.clone();
            }
            if parent == ROOT {
                parts.reverse();
                paths.push(parts.join("/"));
            }
        }
        paths.sort();
        paths
    }
    pub fn file_count(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .nodes
            .values()
            .filter(|node| node.kind == "file")
            .count()
    }
    /// A registry adapter over this transport and this connection store.
    pub fn adapter(&self) -> Arc<dyn crate::storage_providers::traits::StorageProviderAdapter> {
        Arc::new(MockFeishuAdapter {
            origin: self.origin.clone(),
            token_url: self.token_url(),
            connections: self.connections.clone(),
        })
    }
    /// A saved configuration this mock's adapter opens.
    pub fn config(&self, id: &str) -> mallard_domain::StorageConfig {
        mallard_domain::StorageConfig {
            id: mallard_domain::StorageId::parse(id).unwrap(),
            name: "Feishu".into(),
            provider: crate::storage_providers::feishu::provider_key(),
            provider_config: mallard_domain::StorageProviderConfig::new(
                1,
                BoundedConfigObject::encode(&crate::storage_providers::feishu::FeishuSettingsV1 {
                    connection_id: self.connection_id.to_string(),
                    folder_token: ROOT.into(),
                    folder_name: Some("Mallard fixture".into()),
                    account_name: "Test User".into(),
                })
                .unwrap(),
                mallard_domain::SecretMap::empty(),
            ),
        }
    }
}

impl Default for MockFeishuServer {
    fn default() -> Self {
        Self::new()
    }
}

use crate::storage_providers::{
    feishu::{FeishuAdapter, FeishuSettingsV1},
    traits::*,
};
use mallard_domain::{BoundedConfigObject, StorageProviderConfig};

/// Inject the loopback transport through the existing registry in tests
/// only: the production adapter's contract over the mock's origin and the
/// mock's connection documents.
pub struct MockFeishuAdapter {
    origin: String,
    token_url: String,
    connections: FeishuConnectionStore,
}

impl MockFeishuAdapter {
    fn connect(
        &self,
        config: &StorageProviderConfig,
        runtime: Handle,
    ) -> std::result::Result<FeishuObjectStore, StorageAdapterError> {
        FeishuAdapter.validate_config(config)?;
        let settings: FeishuSettingsV1 = config.settings.decode().unwrap();
        FeishuObjectStore::with_origin(
            settings.folder_token,
            FeishuAuthorization::new(
                MockFeishuServer::app(),
                ConnectionId::parse(settings.connection_id).unwrap(),
                self.connections.clone(),
                FeishuClient::for_test(&self.origin, &self.token_url),
            ),
            runtime,
            &self.origin,
        )
        .map_err(|error| StorageAdapterError::remote("feishu_probe_failed", error.message()))
    }
    pub fn connections(&self) -> &FeishuConnectionStore {
        &self.connections
    }
    pub fn origin(&self) -> &Path {
        Path::new(&self.origin)
    }
}

impl StorageProviderAdapter for MockFeishuAdapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        FeishuAdapter.descriptor()
    }
    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> std::result::Result<BoundedConfigObject, StorageAdapterError> {
        FeishuAdapter.validate_settings(settings)
    }
    fn validate_config(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<(), StorageAdapterError> {
        FeishuAdapter.validate_config(config)
    }
    fn summary(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<Vec<StorageSummaryField>, String> {
        FeishuAdapter.summary(config)
    }
    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<Vec<String>, String> {
        FeishuAdapter.continuity_coordinates(config)
    }
    fn local_root(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<Option<PathBuf>, String> {
        FeishuAdapter.local_root(config)
    }
    fn probe(
        &self,
        config: &StorageProviderConfig,
        runtime: &Handle,
    ) -> std::result::Result<(), StorageAdapterError> {
        let store = self.connect(config, runtime.clone())?;
        store.probe().map_err(|error| store.adapter_error(error))
    }
    fn open(
        &self,
        config: &StorageProviderConfig,
        runtime: Option<Handle>,
    ) -> std::result::Result<Arc<dyn ObjectStore>, String> {
        Ok(Arc::new(
            self.connect(config, runtime.ok_or("test runtime required")?)
                .map_err(|e| e.to_string())?,
        ))
    }
}

/// Test transport that makes every listed read independent of the path cache.
/// A project or Library caller that discards the reference fails immediately.
pub struct ListedReadStore(pub FeishuObjectStore);

impl ObjectStore for ListedReadStore {
    fn list_objects_batch(
        &self,
        queries: &[crate::storage_core::ObjectQuery],
        max: usize,
    ) -> Vec<std::result::Result<Vec<ListedObject>, String>> {
        self.0.list_objects_batch(queries, max)
    }

    fn get_metadata(
        &self,
        key: &ObjectKey,
        max: u64,
    ) -> std::result::Result<Option<StoredMetadata>, String> {
        assert!(
            !key.as_str().contains("/_commits/"),
            "listed record was read by key"
        );
        self.0.get_metadata(key, max)
    }
    fn get_listed_metadata_many(
        &self,
        objects: &[ListedObject],
        max: u64,
    ) -> std::result::Result<Vec<Option<StoredMetadata>>, String> {
        assert!(
            objects.iter().all(|object| object.feishu.is_some()),
            "listed token was dropped"
        );
        *self.0.cache.lock().unwrap() = PathCache::default();
        self.0.get_listed_metadata_many(objects, max)
    }
    fn list_objects(
        &self,
        prefix: &ObjectPrefix,
        max: usize,
    ) -> std::result::Result<Vec<ListedObject>, String> {
        let objects = self.0.list_objects(prefix, max)?;
        *self.0.cache.lock().unwrap() = PathCache::default();
        Ok(objects)
    }
    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> std::result::Result<StoreListPage, String> {
        self.0.list(prefix, cursor, limit)
    }
    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> std::result::Result<StageOutcome, String> {
        self.0.stage_verified(key, expected)
    }
    fn put_immutable(
        &self,
        key: &ObjectKey,
        bytes: &[u8],
    ) -> std::result::Result<ImmutablePutOutcome, String> {
        self.0.put_immutable(key, bytes)
    }
    fn put_record(
        &self,
        key: &ObjectKey,
        bytes: &[u8],
    ) -> std::result::Result<ImmutablePutOutcome, String> {
        self.0.put_record(key, bytes)
    }
}
