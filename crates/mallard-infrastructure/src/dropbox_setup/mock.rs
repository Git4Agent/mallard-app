//! Loopback-only setup API. All credentials and account IDs are synthetic.
use crate::dropbox_auth::DropboxClient;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct State {
    pub counts: BTreeMap<String, usize>,
    pub folder_path: Option<String>,
    pub shared: bool,
    pub asynchronous: bool,
    pub in_progress: bool,
    pub fail_after: Option<String>,
    pub deny_refresh: bool,
    pub missing_scope: bool,
    pub replacement: bool,
    pub account_changed: bool,
    pub redirect_token: bool,
}
pub struct MockDropboxSetup {
    pub state: Arc<Mutex<State>>,
    pub runtime: tokio::runtime::Runtime,
    origin: String,
}
fn reply(status: u16, value: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}
async fn serve(
    request: Request<Incoming>,
    shared: Arc<Mutex<State>>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let (parts, body) = request.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let route = parts
        .uri
        .path()
        .trim_start_matches('/')
        .trim_start_matches("2/");
    let mut state = shared.lock().unwrap();
    *state.counts.entry(route.into()).or_default() += 1;
    if route == "oauth2/token" {
        assert!(!parts.headers.contains_key("authorization"));
        let fields = url::form_urlencoded::parse(&bytes)
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert!(!fields.contains_key("client_secret"));
        assert_eq!(fields["client_id"], crate::dropbox_auth::APP_KEY);
        if state.redirect_token {
            return Ok(Response::builder()
                .status(302)
                .header("Location", "http://127.0.0.1:1/forbidden")
                .body(Full::new(Bytes::new()))
                .unwrap());
        }
        let refresh = fields["grant_type"] == "refresh_token";
        *state
            .counts
            .entry(if refresh { "refresh" } else { "exchange" }.into())
            .or_default() += 1;
        if refresh && state.deny_refresh {
            let mut response = reply(
                400,
                json!({"error":"invalid_grant", "error_description":"secret-canary"}),
            );
            response
                .headers_mut()
                .insert("x-dropbox-request-id", "req-refresh-1".parse().unwrap());
            return Ok(response);
        }
        let scope = if state.missing_scope {
            "account_info.read"
        } else if refresh {
            crate::dropbox_auth::STORAGE_SCOPES
        } else {
            crate::dropbox_auth::SETUP_SCOPES
        };
        let mut token = json!({"access_token":"test-token", "token_type":"bearer", "expires_in":14400, "scope":scope});
        if !refresh {
            token["refresh_token"] = json!("refresh-canary");
            token["account_id"] = json!("dbid:fixture");
        }
        return Ok(reply(200, token));
    }
    if parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        != Some("Bearer test-token")
    {
        return Ok(reply(401, Value::Null));
    }
    let args: Value = serde_json::from_slice(&bytes).unwrap();
    let response = match route {
        "users/get_current_account" => {
            json!({"account_id":if state.account_changed { "dbid:other" } else { "dbid:fixture" }, "root_info": {".tag":"user", "home_namespace_id":"home", "root_namespace_id":"home"}})
        }
        "files/create_folder_v2" => {
            assert_eq!(args["autorename"], false);
            assert!(
                state.folder_path.is_none(),
                "a second folder must never be created"
            );
            state.folder_path = Some(args["path"].as_str().unwrap().to_lowercase());
            json!({"metadata":{"id":"id:created", "name":"Mallard", "path_lower":state.folder_path}})
        }
        "files/get_metadata" => {
            let path = args["path"].as_str().unwrap();
            assert!(path == "id:created" || Some(path.to_lowercase()) == state.folder_path);
            let mut metadata = json!({".tag":"folder", "id":if state.shared { "id:after-sharing" } else { "id:created" }, "path_lower":state.folder_path});
            if state.shared {
                metadata["sharing_info"] = json!({"shared_folder_id":if state.replacement { "replacement" } else { "shared" }});
            }
            metadata
        }
        "sharing/share_folder" => {
            assert_eq!(args["path"], "id:created");
            state.shared = true;
            if state.asynchronous {
                json!({".tag":"async_job_id", "async_job_id":"job:fixture"})
            } else {
                json!({".tag":"complete", "shared_folder_id":"shared", "access_type":{".tag":"owner"}, "path_lower":state.folder_path})
            }
        }
        "sharing/check_share_job_status" => {
            assert_eq!(args["async_job_id"], "job:fixture");
            if state.in_progress {
                json!({".tag":"in_progress"})
            } else {
                json!({".tag":"complete", "shared_folder_id":"shared", "access_type":{".tag":"owner"}, "path_lower":state.folder_path})
            }
        }
        "files/list_folder" => {
            let root: Value =
                serde_json::from_slice(parts.headers["dropbox-api-path-root"].as_bytes()).unwrap();
            assert_eq!(
                root,
                json!({".tag":"namespace_id", "namespace_id":"shared"})
            );
            json!({"entries":[], "has_more":false, "cursor":"cursor"})
        }
        _ => panic!("unexpected mock route"),
    };
    if state.fail_after.as_deref() == Some(route) {
        state.fail_after = None;
        return Ok(reply(500, json!({"error":"response-lost-canary"})));
    }
    Ok(reply(200, response))
}
impl MockDropboxSetup {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
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
        Self {
            state,
            runtime,
            origin,
        }
    }
    pub fn client(&self) -> DropboxClient {
        DropboxClient::for_test(&format!("{}/2", self.origin), &self.token_url())
    }
    pub fn token_url(&self) -> String {
        format!("{}/oauth2/token", self.origin)
    }
    pub fn count(&self, route: &str) -> usize {
        *self.state.lock().unwrap().counts.get(route).unwrap_or(&0)
    }
}
impl Default for MockDropboxSetup {
    fn default() -> Self {
        Self::new()
    }
}
