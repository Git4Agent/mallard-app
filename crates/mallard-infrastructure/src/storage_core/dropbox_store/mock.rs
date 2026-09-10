//! Stateful loopback Dropbox API for tests. It never connects to Dropbox.
//! Response shapes follow dropbox/dropbox-api-spec files.stone and users.stone.
use super::*;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response as HttpResponse};
use hyper_util::rt::TokioIo;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct State {
    pub(super) entries: BTreeMap<String, (Metadata, Vec<u8>)>,
    pub(super) revision: u64,
    pub(super) uploads: usize,
    upload_bodies: Vec<(String, usize)>,
    pub(super) requests: usize,
    pub(super) routes: BTreeMap<String, usize>,
    pub(super) metadata_paths: BTreeMap<String, usize>,
    pub(super) sessions: BTreeMap<String, (String, Vec<u8>)>,
    pub(super) next_session: usize,
    pub(super) active_starts: usize,
    pub(super) max_active_starts: usize,
    pub(super) start_barrier: Option<Arc<tokio::sync::Barrier>>,
    pub(super) active_downloads: usize,
    pub(super) max_active_downloads: usize,
    pub(super) download_barrier: Option<Arc<tokio::sync::Barrier>>,
    pub(super) fail_start_body: Option<Vec<u8>>,
    pub(super) session_id_override: Option<String>,
    pub(super) entry_failure: Option<(usize, Value)>,
    payloads_until_failure: Option<usize>,
    pub(super) finish_fault: Option<&'static str>,
    pub(super) fail_upload: bool,
    pub(super) bad_hash: bool,
    pub(super) home: bool,
    pub(super) deny: bool,
    /// Answer the next authorized request with 401 `expired_access_token`,
    /// once, the way Dropbox does after a token's four hours run out.
    pub(super) expire_once: bool,
    /// Answer every authorized request with this 401 `AuthError` tag.
    pub(super) auth_tag: Option<&'static str>,
    pub(super) response_status: Option<u16>,
}
pub struct MockDropboxServer {
    pub(super) state: Arc<Mutex<State>>,
    pub store: DropboxObjectStore,
    _runtime: tokio::runtime::Runtime,
}
pub(super) fn folder(path: &str) -> Metadata {
    Metadata {
        kind: "folder".into(),
        name: path.rsplit('/').next().unwrap().into(),
        path_lower: path.into(),
        rev: String::new(),
        id: "id:folder".into(),
        size: 0,
        content_hash: String::new(),
        is_downloadable: None,
        symlink_info: None,
    }
}
fn reply(status: u16, value: Value) -> HttpResponse<Full<Bytes>> {
    HttpResponse::builder()
        .status(status)
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}
fn file_response(metadata: &Metadata) -> Value {
    let mut value = json!(metadata);
    value.as_object_mut().unwrap().remove(".tag");
    value
}
fn missing() -> HttpResponse<Full<Bytes>> {
    reply(
        409,
        json!({"error":{".tag":"path","path":{".tag":"not_found"}}}),
    )
}
/// Ancestor folders of `path`, root first, excluding the namespace root.
fn ancestors(path: &str) -> Vec<String> {
    path.match_indices('/')
        .filter(|(index, _)| *index > 0)
        .map(|(index, _)| path[..index].to_owned())
        .collect()
}
/// Dropbox answers `not_found` below a file and refuses to commit under one.
fn has_file_ancestor(entries: &BTreeMap<String, (Metadata, Vec<u8>)>, path: &str) -> bool {
    ancestors(path)
        .iter()
        .any(|ancestor| entries.get(ancestor).is_some_and(|(m, _)| m.kind == "file"))
}
struct ActiveRequest {
    state: Arc<Mutex<State>>,
    download: bool,
}
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        if self.download {
            state.active_downloads -= 1;
        } else {
            state.active_starts -= 1;
        }
    }
}

async fn serve(
    request: Request<Incoming>,
    state: Arc<Mutex<State>>,
) -> std::result::Result<HttpResponse<Full<Bytes>>, std::convert::Infallible> {
    let (parts, body) = request.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let args: Value = if let Some(header) = parts.headers.get("dropbox-api-arg") {
        serde_json::from_slice(header.as_bytes()).unwrap()
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    let user = match parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        Some("Bearer test-token") => Some("first"),
        Some("Bearer second-token") => Some("second"),
        _ => None,
    };
    assert!(!parts.headers.contains_key("dropbox-api-select-user"));
    assert!(!parts.headers.contains_key("dropbox-api-select-admin"));
    let route = parts.uri.path().trim_start_matches("/2/");
    let (barrier, _active) = {
        let mut locked = state.lock().unwrap();
        locked.requests += 1;
        *locked.routes.entry(route.to_owned()).or_default() += 1;
        // Dropbox's AuthError shape: a fixed `.tag`, a summary, and a
        // request ID header. The summary must never reach a message.
        let tag = if user.is_none() {
            Some("invalid_access_token")
        } else if std::mem::take(&mut locked.expire_once) {
            Some("expired_access_token")
        } else {
            locked.auth_tag
        };
        if let Some(tag) = tag {
            let mut response = reply(
                401,
                json!({"error_summary": format!("{tag}/summary-canary"), "error": {".tag": tag}}),
            );
            response.headers_mut().insert(
                "x-dropbox-request-id",
                format!("req-{}", locked.requests).parse().unwrap(),
            );
            return Ok(response);
        }
        if route == "files/upload_session/start" {
            locked.upload_bodies.push((sha256_hex(&bytes), bytes.len()));
            locked.active_starts += 1;
            locked.max_active_starts = locked.max_active_starts.max(locked.active_starts);
            (
                locked.start_barrier.clone(),
                Some(ActiveRequest {
                    state: state.clone(),
                    download: false,
                }),
            )
        } else if route == "files/download" {
            locked.active_downloads += 1;
            locked.max_active_downloads = locked.max_active_downloads.max(locked.active_downloads);
            (
                locked.download_barrier.clone(),
                Some(ActiveRequest {
                    state: state.clone(),
                    download: true,
                }),
            )
        } else {
            (None, None)
        }
    };
    // Never hold the mock state lock across a wait. A serial client must fail
    // the barrier test rather than make the mock serialize all requests.
    if let Some(barrier) = barrier {
        tokio::time::timeout(Duration::from_secs(5), barrier.wait())
            .await
            .expect("requests did not run concurrently");
    }
    let user = user.expect("an unauthorized request was answered above");
    let mut state = state.lock().unwrap();
    if route == "users/get_current_account" {
        return Ok(reply(
            200,
            json!({"account_id":format!("dbid:{user}"),"root_info":{".tag":"team","home_namespace_id":if state.home {"shared"} else {"home"},"root_namespace_id":"team"}}),
        ));
    }
    assert_eq!(
        serde_json::from_slice::<Value>(parts.headers["dropbox-api-path-root"].as_bytes()).unwrap(),
        json!({".tag":"namespace_id","namespace_id":"shared"})
    );
    if let Some(status) = state.response_status {
        let mut response = reply(status, json!({"private_response": "must never escape"}));
        response
            .headers_mut()
            .insert("location", "/2/never-follow".parse().unwrap());
        response
            .headers_mut()
            .insert("retry-after", "60".parse().unwrap());
        return Ok(response);
    }
    if state.deny {
        return Ok(reply(401, json!({"secret":"must never escape"})));
    }
    let path = args["path"].as_str().unwrap_or("").to_owned();
    let response = match route {
        "files/get_metadata" => {
            *state.metadata_paths.entry(path.clone()).or_default() += 1;
            if has_file_ancestor(&state.entries, &path) {
                missing()
            } else {
                state
                    .entries
                    .get(&path)
                    .map(|(m, _)| reply(200, json!(m)))
                    .unwrap_or_else(missing)
            }
        }
        "files/create_folder_v2" => panic!("obsolete folder creation route"),
        "files/upload" => panic!("obsolete direct upload route"),
        "files/upload_session/start" => {
            assert_eq!(args["close"], true);
            assert!(args.get("session_type").is_none());
            assert_eq!(args["content_hash"], content_hash(&bytes));
            if state.fail_start_body.as_deref() == Some(bytes.as_ref()) {
                reply(503, json!({"private_response":"must never escape"}))
            } else {
                state.next_session += 1;
                let id = format!("session-{}", state.next_session);
                state
                    .sessions
                    .insert(id.clone(), (user.to_owned(), bytes.to_vec()));
                reply(
                    200,
                    json!({"session_id":state.session_id_override.as_ref().unwrap_or(&id)}),
                )
            }
        }
        "files/upload_session/finish_batch_v2" => {
            let entries = args["entries"].as_array().unwrap();
            assert!(!entries.is_empty() && entries.len() <= 1000);
            let mut results = Vec::new();
            for (index, entry) in entries.iter().enumerate() {
                state.uploads += 1;
                let commit = &entry["commit"];
                assert_eq!(commit["strict_conflict"], true);
                assert_eq!(commit["autorename"], false);
                assert_eq!(commit["mute"], true);
                let path = commit["path"].as_str().unwrap().to_owned();
                let id = entry["cursor"]["session_id"].as_str().unwrap();
                let (owner, bytes) = state.sessions.remove(id).expect("unknown session");
                assert_eq!(owner, user);
                assert_eq!(entry["cursor"]["offset"], bytes.len());
                if let Some((failed_index, error)) = &state.entry_failure {
                    if index == *failed_index {
                        results.push(json!({".tag":"failure","failure":error}));
                        continue;
                    }
                }
                if path.contains("/_objects/") {
                    if let Some(remaining) = &mut state.payloads_until_failure {
                        if *remaining == 0 {
                            results.push(json!({".tag":"failure","failure":{".tag":"too_many_write_operations"}}));
                            continue;
                        }
                        *remaining -= 1;
                    }
                }
                if has_file_ancestor(&state.entries, &path) {
                    results.push(json!({".tag":"failure","failure":{".tag":"path","path":{".tag":"conflict","conflict":{".tag":"file_ancestor"}}}}));
                    continue;
                }
                let mode = &commit["mode"];
                let allowed = match state.entries.get(&path) {
                    None => mode[".tag"] == "add",
                    Some((m, _)) => {
                        m.kind == "file" && mode[".tag"] == "update" && mode["update"] == m.rev
                    }
                };
                if !allowed {
                    results.push(json!({".tag":"failure","failure":{".tag":"path","path":{".tag":"conflict","conflict":{".tag":"file"}}}}));
                    continue;
                }
                // Dropbox creates missing parent folders on commit.
                for ancestor in ancestors(&path) {
                    let m = folder(&ancestor);
                    state.entries.entry(ancestor).or_insert((m, vec![]));
                }
                state.revision += 1;
                let metadata = Metadata {
                    kind: "file".into(),
                    name: path.rsplit('/').next().unwrap().into(),
                    path_lower: path.clone(),
                    rev: format!("{:09x}", state.revision),
                    id: format!("id:{}", state.revision),
                    size: bytes.len() as u64,
                    content_hash: content_hash(&bytes),
                    is_downloadable: Some(true),
                    symlink_info: None,
                };
                state.entries.insert(path, (metadata.clone(), bytes));
                // The documented shape: FileMetadata fields flattened beside
                // the entry's `.tag`, never nested under "success".
                let mut success = file_response(&metadata);
                success[".tag"] = json!("success");
                results.push(success);
            }
            match state.finish_fault {
                Some("count") => {
                    results.pop();
                }
                Some("order") => results.reverse(),
                Some("hash") => results[0]["content_hash"] = json!("0".repeat(64)),
                Some("path") => results[0]["path_lower"] = json!("/.mallard/elsewhere"),
                Some("revision") => results[0]["rev"] = json!("invalid"),
                Some("size") => results[0]["size"] = json!(u64::MAX),
                Some("symlink") => results[0]["symlink_info"] = json!({"target":"elsewhere"}),
                Some("tag") => results[0][".tag"] = json!("unrecognized"),
                // The pre-cutover mock shape: metadata nested under "success".
                Some("nested") => {
                    let fields = results[0].take();
                    results[0] = json!({".tag":"success","success":fields});
                }
                Some("json") => return Ok(HttpResponse::new(Full::new(Bytes::from_static(b"{")))),
                None => {}
                _ => panic!("unknown finish fault"),
            }
            if std::mem::take(&mut state.fail_upload) {
                reply(503, json!({"private_response":"must never escape"}))
            } else {
                reply(200, json!({"entries":results}))
            }
        }
        "files/download" => {
            if has_file_ancestor(&state.entries, &path) {
                missing()
            } else if let Some((m, bytes)) = state.entries.get(&path) {
                let mut m = m.clone();
                if state.bad_hash {
                    m.content_hash = "0".repeat(64);
                }
                HttpResponse::builder()
                    .header(
                        "dropbox-api-result",
                        serde_json::to_string(&file_response(&m)).unwrap(),
                    )
                    .body(Full::new(Bytes::copy_from_slice(bytes)))
                    .unwrap()
            } else {
                missing()
            }
        }
        "files/delete_v2" => {
            assert!(args["parent_rev"].as_str().is_some());
            if let Some((m, _)) = state.entries.get(&path) {
                assert_eq!(m.kind, "file", "must never delete folders");
                if args["parent_rev"] != m.rev {
                    reply(
                        409,
                        json!({"error":{".tag":"path_write","path_write":{".tag":"conflict","conflict":{".tag":"file"}}}}),
                    )
                } else {
                    let (m, _) = state.entries.remove(&path).unwrap();
                    reply(200, json!({"metadata":m}))
                }
            } else {
                missing()
            }
        }
        "files/list_folder" | "files/list_folder/continue" => {
            let (prefix, offset) = if route.ends_with("continue") {
                let c = args["cursor"].as_str().unwrap();
                let (p, n) = c.rsplit_once('|').unwrap();
                (p.to_owned(), n.parse::<usize>().unwrap())
            } else {
                (path, 0)
            };
            let entries: Vec<_> = state
                .entries
                .iter()
                // Recursive listings can include the requested folder itself.
                .filter(|(p, _)| *p == &prefix || p.starts_with(&format!("{prefix}/")))
                .map(|(_, (m, _))| json!(m))
                .collect();
            let end = (offset + 2).min(entries.len());
            reply(
                200,
                json!({"entries":entries[offset..end],"has_more":end<entries.len(),"cursor":format!("{prefix}|{end}")}),
            )
        }
        _ => panic!("unexpected route {route}"),
    };
    Ok(response)
}
impl MockDropboxServer {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let address = listener.local_addr().unwrap();
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
        let origin = format!("http://{address}/2");
        let store = DropboxObjectStore::with_origins(
            "shared".into(),
            &SecretValue::parse("test-token").unwrap(),
            runtime.handle().clone(),
            &origin,
            &origin,
        )
        .unwrap();
        Self {
            state,
            store,
            _runtime: runtime,
        }
    }
}

impl Default for MockDropboxServer {
    fn default() -> Self {
        Self::new()
    }
}
impl MockDropboxServer {
    /// Actual HTTP content bodies received, including metadata and repeated uploads.
    pub fn take_upload_bodies(&self) -> Vec<(String, usize)> {
        std::mem::take(&mut self.state.lock().unwrap().upload_bodies)
    }

    /// Allow this many more payload writes, then reject payload commits.
    pub fn reject_payloads_after(&self, successful: usize) {
        self.state.lock().unwrap().payloads_until_failure = Some(successful);
    }

    /// Answer every authorized request with this 401 `AuthError` tag, or
    /// accept the token again with `None`.
    pub fn set_auth_tag(&self, tag: Option<&'static str>) {
        self.state.lock().unwrap().auth_tag = tag;
    }

    /// A second user shares the same mock namespace and revision history.
    pub fn second_user(&self) -> DropboxObjectStore {
        DropboxObjectStore::with_origins(
            "shared".into(),
            &SecretValue::parse("second-token").unwrap(),
            self._runtime.handle().clone(),
            &self.store.api,
            &self.store.content,
        )
        .unwrap()
    }
    pub fn probe_file_count(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|(path, (metadata, _))| {
                path.starts_with("/.mallard/v1/_setup_probes/") && metadata.kind == "file"
            })
            .count()
    }
    /// A registry adapter over this transport for manual-token configs.
    pub fn adapter(&self) -> Arc<dyn crate::storage_providers::traits::StorageProviderAdapter> {
        Arc::new(MockDropboxAdapter {
            origin: self.store.api.clone(),
            token_url: None,
        })
    }
    /// A registry adapter over this transport that also accepts refresh
    /// credentials, renewing them against the loopback setup mock.
    pub fn adapter_with_refresh(
        &self,
        oauth: &crate::dropbox_setup::mock::MockDropboxSetup,
    ) -> Arc<dyn crate::storage_providers::traits::StorageProviderAdapter> {
        Arc::new(MockDropboxAdapter {
            origin: self.store.api.clone(),
            token_url: Some(oauth.token_url()),
        })
    }
    pub fn runtime(&self) -> &Handle {
        self._runtime.handle()
    }
}

use crate::storage_providers::{
    dropbox::{refresh_token_key, token_key, DropboxAdapter},
    traits::*,
};
use mallard_domain::{BoundedConfigObject, StorageProviderConfig};
use std::path::PathBuf;

/// Inject the loopback transport through the existing registry in tests only.
struct MockDropboxAdapter {
    origin: String,
    token_url: Option<String>,
}
impl MockDropboxAdapter {
    fn connect(
        &self,
        config: &StorageProviderConfig,
        runtime: Handle,
    ) -> std::result::Result<DropboxObjectStore, StorageAdapterError> {
        DropboxAdapter.validate_config(config)?;
        let settings: crate::storage_providers::dropbox::DropboxSettingsV1 =
            config.settings.decode().unwrap();
        let failed =
            |error: Error| StorageAdapterError::remote("dropbox_probe_failed", error.message());
        if let (Some(token_url), Some(refresh)) =
            (&self.token_url, config.secrets.get(&refresh_token_key()))
        {
            return DropboxObjectStore::with_authorization(
                settings.namespace_id,
                DropboxAuthorization::new(
                    DropboxCredential::RefreshToken(refresh.clone()),
                    DropboxClient::for_test(&self.origin, token_url),
                ),
                runtime,
                &self.origin,
                &self.origin,
            )
            .map_err(failed);
        }
        DropboxObjectStore::with_origins(
            settings.namespace_id,
            config.secrets.get(&token_key()).ok_or_else(|| {
                StorageAdapterError::invalid(
                    "dropbox_config_invalid",
                    "the mock adapter has no token endpoint for a refresh credential",
                )
            })?,
            runtime,
            &self.origin,
            &self.origin,
        )
        .map_err(failed)
    }
}
impl StorageProviderAdapter for MockDropboxAdapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        DropboxAdapter.descriptor()
    }
    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> std::result::Result<BoundedConfigObject, StorageAdapterError> {
        DropboxAdapter.validate_settings(settings)
    }
    fn validate_config(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<(), StorageAdapterError> {
        DropboxAdapter.validate_config(config)
    }
    fn summary(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<Vec<StorageSummaryField>, String> {
        DropboxAdapter.summary(config)
    }
    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<Vec<String>, String> {
        DropboxAdapter.continuity_coordinates(config)
    }
    fn local_root(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<Option<PathBuf>, String> {
        DropboxAdapter.local_root(config)
    }
    fn journal_facts(
        &self,
        config: &StorageProviderConfig,
    ) -> std::result::Result<BoundedConfigObject, String> {
        DropboxAdapter.journal_facts(config)
    }
    fn probe(
        &self,
        config: &StorageProviderConfig,
        setup_id: &str,
        runtime: &Handle,
        journal: &mut dyn StorageProbeJournal,
    ) -> std::result::Result<(), StorageAdapterError> {
        let store = self.connect(config, runtime.clone())?;
        store
            .probe(setup_id, journal)
            .map_err(|error| store.adapter_error(error))
    }
    fn reconcile_probe(
        &self,
        config: &StorageProviderConfig,
        setup_id: &str,
        runtime: &Handle,
        journal: &mut dyn StorageProbeJournal,
    ) -> std::result::Result<(), StorageAdapterError> {
        let store = self.connect(config, runtime.clone())?;
        store
            .reconcile_probe(setup_id, journal)
            .map_err(|error| store.adapter_error(error))
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
