//! Direct Feishu setup with Mallard's own app. The grant lives in one
//! ephemeral Rust session between the browser step and the prepare; the
//! link is handed to the page, never opened here.
use mallard_application::feishu_setup::FeishuPrepared;
use mallard_application::storage_setup::begin_storage_draft_operation;
use mallard_domain::{now_secs, HostId, StorageSetupDraft, StorageSetupDraftId};
use mallard_infrastructure::{
    dropbox_setup::valid_folder_name,
    feishu_auth::{
        folder_token_from_link, Account, AuthError, AuthFailure, AuthorizationGrant,
        AuthorizationRequest, FeishuApp, FeishuClient, PkceDiagnostics, CALLBACK_PORT,
        CALLBACK_URI, FEISHU_CALLBACK,
    },
    feishu_connections, loopback_browser,
    persistence::LocalStateRepository,
    storage_providers::{feishu::provider_key, StorageProviderRegistry},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Create,
    Reconnect,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<mallard_infrastructure::storage_core::ConnectionEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_code: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pkce: Option<PkceDiagnostics>,
}
impl Failure {
    fn code(code: &str) -> Self {
        Self {
            code: code.into(),
            evidence: None,
            provider_code: None,
            pkce: None,
        }
    }
    fn local() -> Self {
        Self::code("feishu_setup_failed")
    }
}
impl From<AuthError> for Failure {
    fn from(error: AuthError) -> Self {
        Self::code(error.code())
    }
}

impl From<AuthFailure> for Failure {
    fn from(failure: AuthFailure) -> Self {
        Self {
            code: failure.error.code().into(),
            evidence: failure.evidence,
            provider_code: failure.provider_code,
            pkce: None,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Progress {
    AuthorizationLink { url: String, expires_in: u32 },
}

#[derive(Serialize)]
pub struct LoginAccount {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct Login {
    session_id: String,
    account: LoginAccount,
}

struct Session {
    id: String,
    draft_id: StorageSetupDraftId,
    revision: u64,
    mode: Mode,
    grant: AuthorizationGrant,
    account: Account,
    expires: u64,
}
struct Active {
    draft_id: StorageSetupDraftId,
    revision: u64,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    active: Option<Active>,
    session: Option<Session>,
}
#[derive(Default)]
pub struct FeishuSetupControl(Arc<Mutex<State>>);

struct Run<'a> {
    control: &'a FeishuSetupControl,
    cancel: Arc<AtomicBool>,
}
impl Drop for Run<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.control.0.lock() {
            state.active = None;
        }
    }
}
impl FeishuSetupControl {
    fn begin(
        &self,
        draft_id: &StorageSetupDraftId,
        revision: u64,
        session_id: Option<&str>,
    ) -> Result<(Run<'_>, Option<Session>), Failure> {
        let mut state = self.0.lock().map_err(|_| Failure::local())?;
        if state.active.is_some() {
            return Err(Failure::code("feishu_setup_busy"));
        }
        let session = state.session.take();
        if let Some(id) = session_id {
            if !session.as_ref().is_some_and(|session| {
                session.id == id
                    && session.draft_id == *draft_id
                    && session.revision == revision
                    && now_secs() < session.expires
            }) {
                return Err(Failure::code("feishu_session_expired"));
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.active = Some(Active {
            draft_id: draft_id.clone(),
            revision,
            cancel: cancel.clone(),
        });
        Ok((
            Run {
                control: self,
                cancel,
            },
            if session_id.is_some() { session } else { None },
        ))
    }

    fn keep(&self, mut session: Session, cancel: &AtomicBool) -> Result<(), Failure> {
        if cancel.load(Ordering::Acquire) {
            return Err(Failure::code("feishu_setup_cancelled"));
        }
        session.expires = now_secs() + 600;
        let id = session.id.clone();
        let expires = session.expires;
        {
            let mut state = self.0.lock().map_err(|_| Failure::local())?;
            if cancel.load(Ordering::Acquire) {
                return Err(Failure::code("feishu_setup_cancelled"));
            }
            state.session = Some(session);
        }
        let weak = Arc::downgrade(&self.0);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(600)).await;
            if let Some(state) = weak.upgrade() {
                if let Ok(mut state) = state.lock() {
                    if state.session.as_ref().is_some_and(|s| {
                        s.id == id && s.expires == expires && now_secs() >= expires
                    }) {
                        state.session = None;
                    }
                }
            }
        });
        Ok(())
    }

    fn cancel(&self, draft_id: &StorageSetupDraftId, revision: u64) -> Result<(), Failure> {
        let mut state = self.0.lock().map_err(|_| Failure::local())?;
        if let Some(active) = &state.active {
            if active.draft_id == *draft_id && active.revision == revision {
                active.cancel.store(true, Ordering::Release);
            }
        }
        if state
            .session
            .as_ref()
            .is_some_and(|s| s.draft_id == *draft_id && s.revision == revision)
        {
            state.session = None;
        }
        Ok(())
    }
}

fn repository() -> Result<LocalStateRepository, Failure> {
    LocalStateRepository::from_user_home().map_err(|_| Failure::local())
}

fn draft(
    repository: &LocalStateRepository,
    id: &StorageSetupDraftId,
    revision: u64,
    mode: Mode,
) -> Result<StorageSetupDraft, Failure> {
    let draft = repository
        .load_storage_setup_draft(id)
        .map_err(|_| Failure::local())?
        .ok_or_else(|| Failure::code("feishu_draft_changed"))?;
    if draft.revision != revision
        || draft.storage.provider != provider_key()
        || matches!(
            draft.source,
            mallard_domain::StorageSetupDraftSource::Edit { .. }
        ) != (mode == Mode::Reconnect)
    {
        return Err(Failure::code("feishu_draft_changed"));
    }
    Ok(draft)
}

async fn cancellable<T>(
    cancel: &AtomicBool,
    future: impl std::future::Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    tokio::select! {
        biased;
        _ = async { while !cancel.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(25)).await; } } => Err(Failure::code("feishu_setup_cancelled")),
        value = future => value,
    }
}

fn valid_link(request: &AuthorizationRequest, app_id: &str) -> bool {
    let url = &request.url;
    url.as_str().len() <= 8192
        && url.scheme() == "https"
        && url.host_str() == Some("accounts.feishu.cn")
        && url.path() == "/oauth/v3/authorize"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url
            .query_pairs()
            .any(|(k, v)| k == "client_id" && v == app_id)
        && url
            .query_pairs()
            .any(|(k, v)| k == "redirect_uri" && v == CALLBACK_URI)
}

/// The caller must first accept the callback through the state-checking receiver.
async fn exchange_verified_callback(
    client: &FeishuClient,
    app: &FeishuApp,
    code: &str,
    request: &AuthorizationRequest,
) -> Result<AuthorizationGrant, Failure> {
    let pkce = request.diagnostics_after_verified_callback();
    client
        .exchange(app, code, &request.verifier)
        .await
        .map_err(|error| {
            let mut failure = Failure::from(error);
            failure.pkce = Some(pkce);
            failure
        })
}

#[tauri::command]
pub async fn authorize_feishu_setup(
    control: tauri::State<'_, FeishuSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    mode: Mode,
    on_progress: tauri::ipc::Channel<Progress>,
) -> Result<Login, Failure> {
    let app = FeishuApp::built_in().ok_or_else(|| Failure::code("feishu_app_unavailable"))?;
    let repository = repository()?;
    draft(&repository, &draft_id, expected_revision, mode)?;
    let (run, _) = control.begin(&draft_id, expected_revision, None)?;
    let request = AuthorizationRequest::new(&app.app_id)?;
    if !valid_link(&request, &app.app_id) {
        return Err(Failure::local());
    }
    let cancel = run.cancel.clone();
    // Always join the receiver. Cancellation must release the fixed port before retry.
    let (code, request) = tauri::async_runtime::spawn_blocking(move || {
        let code = loopback_browser::authorize(
            &FEISHU_CALLBACK,
            &request.url,
            &request.state,
            CALLBACK_PORT,
            |url| {
                on_progress
                    .send(Progress::AuthorizationLink {
                        url: url.to_string(),
                        expires_in: 180,
                    })
                    .map_err(|_| ())
            },
            &cancel,
        )
        .map_err(|e| Failure::code(e.code()))?;
        Ok::<_, Failure>((code, request))
    })
    .await
    .map_err(|_| Failure::local())??;
    let client = FeishuClient::new()?;
    let (grant, account) = cancellable(&run.cancel, async {
        let grant = exchange_verified_callback(&client, &app, &code, &request).await?;
        let account = client
            .user_info(&grant.access.token)
            .await
            .map_err(Failure::from)?;
        Ok((grant, account))
    })
    .await?;
    draft(&repository, &draft_id, expected_revision, mode)?;
    let id = HostId::generate()
        .map_err(|_| Failure::local())?
        .to_string();
    let result = Login {
        session_id: id.clone(),
        account: LoginAccount {
            id: account.id.clone(),
            name: account.name.clone(),
        },
    };
    control.keep(
        Session {
            id,
            draft_id,
            revision: expected_revision,
            mode,
            grant,
            account,
            expires: 0,
        },
        &run.cancel,
    )?;
    Ok(result)
}

#[tauri::command]
pub fn propose_feishu_folder_name() -> Result<String, Failure> {
    mallard_infrastructure::dropbox_setup::propose_folder_name()
        .map_err(|e| Failure::code(e.code()))
}

#[tauri::command]
pub async fn prepare_feishu_storage(
    control: tauri::State<'_, FeishuSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    session_id: String,
    folder_name: Option<String>,
    folder_link: Option<String>,
) -> Result<(), Failure> {
    let repository = repository()?;
    let (run, session) = control.begin(&draft_id, expected_revision, Some(&session_id))?;
    let session = session.ok_or_else(Failure::local)?;
    let draft = draft(&repository, &draft_id, expected_revision, session.mode)?;
    let connections = feishu_connections::installed().ok_or_else(Failure::local)?;
    enum Folder {
        Saved,
        New(String),
        Existing(String),
    }
    let folder = match (session.mode, folder_name, folder_link) {
        (Mode::Reconnect, None, None) => Folder::Saved,
        (Mode::Create, Some(name), None) => {
            if !valid_folder_name(&name) {
                return Err(Failure::code("feishu_folder_name_invalid"));
            }
            Folder::New(name)
        }
        (Mode::Create, None, Some(link)) => Folder::Existing(
            folder_token_from_link(&link)
                .ok_or_else(|| Failure::code("feishu_folder_link_invalid"))?,
        ),
        _ => return Err(Failure::code("feishu_draft_changed")),
    };
    let client = FeishuClient::new()?;
    cancellable(&run.cancel, async {
        let remote = Failure::from;
        let (folder_token, folder_name) = match folder {
            Folder::Saved => (None, None),
            Folder::New(name) => {
                let token = client
                    .create_root_folder(&session.grant.access.token, &name)
                    .await
                    .map_err(remote)?;
                (Some(token), Some(name))
            }
            Folder::Existing(token) => {
                client
                    .folder_reachable(&session.grant.access.token, &token)
                    .await
                    .map_err(remote)?;
                (Some(token), None)
            }
        };
        let operation = begin_storage_draft_operation(
            &repository,
            &draft_id,
            expected_revision,
            &draft.storage.name,
        )
        .map_err(|e| Failure::code(e.code()))?;
        mallard_application::feishu_setup::prepare(
            &repository,
            StorageProviderRegistry::production(),
            &operation,
            &draft_id,
            &connections,
            FeishuPrepared {
                grant: &session.grant,
                account_name: &session.account.name,
                folder_token: folder_token.as_deref(),
                folder_name: folder_name.as_deref(),
            },
        )
        .map_err(|e| Failure::code(e.code()))?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn cancel_feishu_setup(
    control: tauri::State<'_, FeishuSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
) -> Result<(), Failure> {
    control.cancel(&draft_id, expected_revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exchange_failure_keeps_this_attempts_pkce_results_without_secrets() {
        use mallard_infrastructure::storage_core::feishu_store::mock::MockFeishuServer;
        let server = MockFeishuServer::new();
        let app = FeishuApp::new(
            "cli_test",
            mallard_domain::SecretValue::parse("wrong-secret").unwrap(),
        )
        .unwrap();
        let request = AuthorizationRequest::new(&app.app_id).unwrap();
        let failure = server
            .runtime()
            .block_on(exchange_verified_callback(
                &server.client(),
                &app,
                "code-canary",
                &request,
            ))
            .unwrap_err();
        assert_eq!(server.count("exchange"), 1);
        let json = serde_json::to_value(&failure).unwrap();
        assert_eq!(
            json["pkce"],
            serde_json::json!({
                "challenge_present": true, "method_s256": true,
                "verifier_matches_challenge": true, "callback_state_verified": true,
                "verifier_length": 64, "challenge_length": 43
            })
        );
        let diagnostic = format!("{failure:?} {json}");
        for secret in [
            "wrong-secret",
            "code-canary",
            request.verifier.as_str(),
            request.state.as_str(),
            request.url.as_str(),
        ] {
            assert!(!diagnostic.contains(secret));
        }
        assert!(Failure::from(AuthError::Local).pkce.is_none());
    }

    #[test]
    fn setup_failure_serializes_only_safe_provider_diagnostics() {
        use mallard_infrastructure::storage_core::ConnectionEvidence;
        let failure = Failure::from(AuthFailure {
            error: AuthError::Reconnect,
            provider_code: Some(20049),
            evidence: Some(ConnectionEvidence {
                endpoint: "oauth/v3/token",
                http_status: Some(400),
                error_tag: Some("invalid_grant"),
                request_id: ConnectionEvidence::request_id(Some(b"req-fixture")),
            }),
            outcome_unknown: false,
        });
        assert_eq!(
            serde_json::to_value(failure).unwrap(),
            serde_json::json!({
                "code": "feishu_reconnect_required",
                "provider_code": 20049,
                "evidence": {
                    "endpoint": "oauth/v3/token", "http_status": 400,
                    "error_tag": "invalid_grant", "request_id": "req-fixture"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(Failure::local()).unwrap(),
            serde_json::json!({"code": "feishu_setup_failed"})
        );
        assert!(ConnectionEvidence::request_id(Some(b"Bearer secret-canary")).is_none());
    }

    fn id() -> StorageSetupDraftId {
        StorageSetupDraftId::parse("storagedraft-111111111111111111111111").unwrap()
    }
    #[test]
    fn single_flight_cancellation_is_scoped_to_the_exact_draft_revision() {
        let control = FeishuSetupControl::default();
        let id = id();
        let (run, _) = control.begin(&id, 4, None).unwrap();
        assert!(control.begin(&id, 4, None).is_err());
        control.cancel(&id, 3).unwrap();
        assert!(!run.cancel.load(Ordering::Acquire));
        control.cancel(&id, 4).unwrap();
        assert!(run.cancel.load(Ordering::Acquire));
        drop(run);
        assert!(control.begin(&id, 4, None).is_ok());
        assert!(
            matches!(control.begin(&id, 1, Some("stale")), Err(Failure { code, .. }) if code == "feishu_session_expired")
        );
    }
    #[test]
    fn authorization_links_pin_the_app_callback_and_origin() {
        let request = AuthorizationRequest::new("cli_test").unwrap();
        assert!(valid_link(&request, "cli_test"));
        assert!(!valid_link(&request, "cli_other"));
        let mut obsolete = AuthorizationRequest::new("cli_test").unwrap();
        obsolete.url.set_path("/open-apis/authen/v1/authorize");
        assert!(!valid_link(&obsolete, "cli_test"));
        let mut foreign = AuthorizationRequest::new("cli_test").unwrap();
        foreign.url.set_host(Some("example.test")).unwrap();
        assert!(!valid_link(&foreign, "cli_test"));
    }
    #[test]
    fn draft_revision_provider_and_mode_are_checked_before_auth() {
        use mallard_application::storage_setup::create_storage_setup_draft;
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let registry = StorageProviderRegistry::from_adapters(vec![Arc::new(
            mallard_infrastructure::storage_providers::feishu::FeishuAdapter,
        )])
        .unwrap();
        let saved = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
        assert!(draft(&repository, &saved.draft_id, saved.revision, Mode::Create).is_ok());
        assert!(draft(
            &repository,
            &saved.draft_id,
            saved.revision + 1,
            Mode::Create
        )
        .is_err());
        assert!(draft(
            &repository,
            &saved.draft_id,
            saved.revision,
            Mode::Reconnect
        )
        .is_err());
    }
}
