//! Direct Dropbox setup. Credentials stay in one ephemeral Rust session.
use mallard_application::storage_setup::begin_storage_draft_operation;
use mallard_domain::{now_secs, HostId, StorageSetupDraft, StorageSetupDraftId};
use mallard_infrastructure::{
    dropbox_auth::{
        AuthError, AuthorizationGrant, AuthorizationRequest, DropboxClient, CALLBACK_PORT,
        DISCOVERY_SCOPES, DROPBOX_CALLBACK, SETUP_SCOPES, STORAGE_SCOPES,
    },
    dropbox_setup::discovery::{self, Account, Folder, Scan},
    loopback_browser,
    persistence::LocalStateRepository,
    storage_core::{MarkerDetailCode, MarkerState},
    storage_providers::{dropbox::provider_key, StorageProviderRegistry},
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
    Existing,
    Reconnect,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: String,
}
impl Failure {
    fn code(code: &str) -> Self {
        Self { code: code.into() }
    }
    fn local() -> Self {
        Self::code("dropbox_setup_failed")
    }
}
impl From<AuthError> for Failure {
    fn from(error: AuthError) -> Self {
        Self::code(error.code())
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Progress {
    AuthorizationLink { url: String, expires_in: u32 },
}

#[derive(Serialize)]
pub struct Login {
    session_id: String,
    account: Account,
}

struct Session {
    id: String,
    draft_id: StorageSetupDraftId,
    revision: u64,
    mode: Mode,
    account: Account,
    grant: AuthorizationGrant,
    expires: u64,
    scan: Option<(String, u64, Scan)>,
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
pub struct DropboxSetupControl(Arc<Mutex<State>>);

struct Run<'a> {
    control: &'a DropboxSetupControl,
    cancel: Arc<AtomicBool>,
}
impl Drop for Run<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.control.0.lock() {
            state.active = None;
        }
    }
}
impl DropboxSetupControl {
    fn begin(
        &self,
        draft_id: &StorageSetupDraftId,
        revision: u64,
        session_id: Option<&str>,
    ) -> Result<(Run<'_>, Option<Session>), Failure> {
        let mut state = self.0.lock().map_err(|_| Failure::local())?;
        if state.active.is_some() {
            return Err(Failure::code("dropbox_setup_busy"));
        }
        let session = state.session.take();
        if let Some(id) = session_id {
            if !session.as_ref().is_some_and(|session| {
                session.id == id
                    && session.draft_id == *draft_id
                    && session.revision == revision
                    && now_secs() < session.expires
            }) {
                return Err(Failure::code("dropbox_session_expired"));
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
            return Err(Failure::code("dropbox_setup_cancelled"));
        }
        session.expires = now_secs() + 600;
        let id = session.id.clone();
        let expires = session.expires;
        {
            let mut state = self.0.lock().map_err(|_| Failure::local())?;
            if cancel.load(Ordering::Acquire) {
                return Err(Failure::code("dropbox_setup_cancelled"));
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
        .ok_or_else(|| Failure::code("dropbox_draft_changed"))?;
    if draft.revision != revision
        || draft.storage.provider != provider_key()
        || matches!(
            draft.source,
            mallard_domain::StorageSetupDraftSource::Edit { .. }
        ) != (mode == Mode::Reconnect)
    {
        return Err(Failure::code("dropbox_draft_changed"));
    }
    Ok(draft)
}

async fn cancellable<T>(
    cancel: &AtomicBool,
    future: impl std::future::Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    tokio::select! {
        biased;
        _ = async { while !cancel.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(25)).await; } } => Err(Failure::code("dropbox_setup_cancelled")),
        value = future => value,
    }
}

fn valid_link(request: &AuthorizationRequest) -> bool {
    use mallard_infrastructure::dropbox_auth::{APP_KEY, CALLBACK_URI};
    let url = &request.url;
    url.as_str().len() <= 8192
        && url.scheme() == "https"
        && url.host_str() == Some("www.dropbox.com")
        && url.path() == "/oauth2/authorize"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url
            .query_pairs()
            .any(|(k, v)| k == "client_id" && v == APP_KEY)
        && url
            .query_pairs()
            .any(|(k, v)| k == "redirect_uri" && v == CALLBACK_URI)
}

#[tauri::command]
pub async fn authorize_dropbox_setup(
    control: tauri::State<'_, DropboxSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    mode: Mode,
    switch_account: bool,
    on_progress: tauri::ipc::Channel<Progress>,
) -> Result<Login, Failure> {
    let repository = repository()?;
    let current = draft(&repository, &draft_id, expected_revision, mode)?;
    if mode == Mode::Create
        && current
            .storage
            .provider_config
            .settings
            .get_str("namespace_id")
            .is_some_and(|id| !id.is_empty())
    {
        return Err(Failure::code("dropbox_new_setup_required"));
    }
    let (run, _) = control.begin(&draft_id, expected_revision, None)?;
    let scopes = match mode {
        Mode::Create => SETUP_SCOPES,
        Mode::Existing => DISCOVERY_SCOPES,
        Mode::Reconnect => STORAGE_SCOPES,
    };
    let request = AuthorizationRequest::for_login(scopes, switch_account)?;
    if !valid_link(&request) {
        return Err(Failure::local());
    }
    let cancel = run.cancel.clone();
    // Always join the receiver. Cancellation must release the fixed port before retry.
    let (code, verifier) = tauri::async_runtime::spawn_blocking(move || {
        let code = loopback_browser::authorize(
            &DROPBOX_CALLBACK,
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
        Ok::<_, Failure>((code, request.verifier))
    })
    .await
    .map_err(|_| Failure::local())??;
    let client = DropboxClient::new()?;
    let (grant, account) = cancellable(&run.cancel, async {
        let grant = client.exchange(&code, &verifier, scopes).await?;
        let account = discovery::account(&client, &grant.access.token, &grant.account_id).await?;
        Ok((grant, account))
    })
    .await?;
    draft(&repository, &draft_id, expected_revision, mode)?;
    let id = HostId::generate()
        .map_err(|_| Failure::local())?
        .to_string();
    let result = Login {
        session_id: id.clone(),
        account: account.clone(),
    };
    control.keep(
        Session {
            id,
            draft_id,
            revision: expected_revision,
            mode,
            account,
            grant,
            scan: None,
            expires: 0,
        },
        &run.cancel,
    )?;
    Ok(result)
}

#[tauri::command]
pub fn propose_dropbox_folder_name() -> Result<String, Failure> {
    mallard_infrastructure::dropbox_setup::propose_folder_name()
        .map_err(|e| Failure::code(e.code()))
}

/// Every sentence here is the English the setup screens print; a code
/// travels beside each one for the interface to translate by.
#[derive(Serialize)]
pub struct DiscoveryResult {
    scan_id: String,
    scope: String,
    /// `dropbox_shared_folders`.
    scope_code: &'static str,
    status: String,
    issues: Vec<String>,
    /// Each issue's code, in `issues` order.
    issue_codes: Vec<Issue>,
    candidates: Vec<Candidate>,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
struct Issue {
    code: &'static str,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
struct Candidate {
    candidate_id: String,
    name: String,
    marker: MarkerState,
    marker_detail: Option<String>,
    marker_detail_code: Option<MarkerDetailCode>,
    /// The version a `layout` detail names.
    marker_layout_version: Option<u32>,
    facts: Vec<Fact>,
    cautions: Vec<String>,
    /// Each caution's code, in `cautions` order.
    caution_codes: Vec<&'static str>,
    connected_storage_id: Option<String>,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
struct Fact {
    /// `access` or `location`.
    key: &'static str,
    label: String,
    value: String,
    /// The access level's Dropbox tag, or `in_dropbox` / `not_mounted`.
    value_code: String,
}

/// One shared folder's row: what the marker said, the folder's access and
/// whether it is mounted, and that read access cannot Push.
fn folder_candidate(
    index: usize,
    folder: &Folder,
    connected_storage_id: Option<String>,
) -> Candidate {
    let detail = folder.detail.as_ref();
    let read_only = folder.access.starts_with("viewer");
    let location = if folder.mounted {
        ("In Dropbox", "in_dropbox")
    } else {
        ("Not mounted", "not_mounted")
    };
    Candidate {
        candidate_id: index.to_string(),
        name: folder.name.clone(),
        marker: folder.marker.clone(),
        marker_detail: detail.map(|detail| detail.text.clone()),
        marker_detail_code: detail.map(|detail| detail.code),
        marker_layout_version: detail.and_then(|detail| detail.layout_version),
        facts: vec![
            Fact {
                key: "access",
                label: "Access".into(),
                value: folder.access.clone(),
                value_code: folder.access.clone(),
            },
            Fact {
                key: "location",
                label: "Location".into(),
                value: location.0.into(),
                value_code: location.1.into(),
            },
        ],
        cautions: if read_only {
            vec!["Read access does not permit Push.".into()]
        } else {
            vec![]
        },
        caution_codes: if read_only { vec!["read_only"] } else { vec![] },
        connected_storage_id,
    }
}

#[tauri::command]
pub async fn discover_dropbox_storage(
    control: tauri::State<'_, DropboxSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    session_id: String,
) -> Result<DiscoveryResult, Failure> {
    let repository = repository()?;
    draft(&repository, &draft_id, expected_revision, Mode::Existing)?;
    let (run, session) = control.begin(&draft_id, expected_revision, Some(&session_id))?;
    let mut session = session.ok_or_else(Failure::local)?;
    if session.mode != Mode::Existing {
        return Err(Failure::code("dropbox_draft_changed"));
    }
    session.scan = None;
    let client = DropboxClient::new()?;
    let scan = cancellable(&run.cancel, async {
        discovery::discover(&client, &session.grant.access.token, &session.account.id)
            .await
            .map_err(Failure::from)
    })
    .await?;
    draft(&repository, &draft_id, expected_revision, Mode::Existing)?;
    let config = repository.load_config().map_err(|_| Failure::local())?;
    let scan_id = HostId::generate()
        .map_err(|_| Failure::local())?
        .to_string();
    let result = DiscoveryResult {
        scan_id: scan_id.clone(),
        scope: "Accessible Dropbox shared folders".into(),
        scope_code: "dropbox_shared_folders",
        status: if scan.partial { "partial" } else { "complete" }.into(),
        issues: if scan.partial {
            vec!["Some folders could not be listed or checked. Scan again to retry.".into()]
        } else {
            vec![]
        },
        issue_codes: if scan.partial {
            vec![Issue {
                code: "folders_partial",
            }]
        } else {
            vec![]
        },
        candidates: scan
            .folders
            .iter()
            .enumerate()
            .map(|(index, folder)| {
                let connected = config
                    .storages
                    .iter()
                    .find(|s| {
                        s.provider == provider_key()
                            && s.provider_config.settings.get_str("namespace_id")
                                == Some(folder.namespace.as_str())
                    })
                    .map(|s| s.id.to_string());
                folder_candidate(index, folder, connected)
            })
            .collect(),
    };
    session.scan = Some((scan_id, now_secs(), scan));
    control.keep(session, &run.cancel)?;
    Ok(result)
}

#[tauri::command]
pub async fn prepare_dropbox_storage(
    control: tauri::State<'_, DropboxSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    session_id: String,
    folder_name: Option<String>,
    scan_id: Option<String>,
    candidate_id: Option<String>,
) -> Result<(), Failure> {
    let repository = repository()?;
    let (run, session) = control.begin(&draft_id, expected_revision, Some(&session_id))?;
    let session = session.ok_or_else(Failure::local)?;
    let draft = draft(&repository, &draft_id, expected_revision, session.mode)?;
    let client = DropboxClient::new()?;
    let selected = match session.mode {
        Mode::Existing => {
            let (id, created, scan) = session
                .scan
                .as_ref()
                .ok_or_else(|| Failure::code("dropbox_scan_expired"))?;
            if scan_id.as_ref() != Some(id)
                || now_secs().saturating_sub(*created) >= 600
                || folder_name.is_some()
            {
                return Err(Failure::code("dropbox_scan_expired"));
            }
            let folder = candidate_id
                .as_deref()
                .and_then(|id| id.parse::<usize>().ok())
                .and_then(|index| scan.folders.get(index))
                .filter(|folder| folder.marker == MarkerState::Storage)
                .ok_or_else(|| Failure::code("dropbox_scan_expired"))?;
            Some((folder.namespace.clone(), folder.name.clone()))
        }
        Mode::Create => {
            if scan_id.is_some()
                || candidate_id.is_some()
                || !folder_name
                    .as_deref()
                    .is_some_and(mallard_infrastructure::dropbox_setup::valid_folder_name)
            {
                return Err(Failure::code("dropbox_folder_name_invalid"));
            }
            None
        }
        Mode::Reconnect => {
            if folder_name.is_some() || scan_id.is_some() || candidate_id.is_some() {
                return Err(Failure::code("dropbox_draft_changed"));
            }
            None
        }
    };
    let result = cancellable(&run.cancel, async {
        discovery::account(&client, &session.grant.access.token, &session.account.id).await?;
        if let Some((namespace, _)) = &selected {
            if discovery::marker(&client, &session.grant.access.token, namespace)
                .await?
                .0
                != MarkerState::Storage
            {
                return Err(Failure::code("dropbox_folder_changed"));
            }
        }
        let operation = begin_storage_draft_operation(
            &repository,
            &draft_id,
            expected_revision,
            &draft.storage.name,
        )
        .map_err(|e| Failure::code(e.code()))?;
        mallard_application::dropbox_setup::prepare(
            &repository,
            StorageProviderRegistry::production(),
            &operation,
            &draft_id,
            &client,
            &session.grant,
            selected.as_ref().map(|(namespace, _)| namespace.as_str()),
            selected.as_ref().map(|(_, name)| name.as_str()),
            folder_name.as_deref(),
        )
        .await
        .map_err(|e| Failure::code(e.code()))?;
        Ok(())
    })
    .await;
    // A confirmed name conflict is editable with the same grant. Other failures
    // require a fresh sign-in.
    if result
        .as_ref()
        .is_err_and(|e| e.code == "dropbox_folder_name_taken")
    {
        control.keep(session, &run.cancel)?;
    }
    result
}

#[tauri::command]
pub fn cancel_dropbox_setup(
    control: tauri::State<'_, DropboxSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
) -> Result<(), Failure> {
    control.cancel(&draft_id, expected_revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> StorageSetupDraftId {
        StorageSetupDraftId::parse("storagedraft-111111111111111111111111").unwrap()
    }
    #[test]
    fn single_flight_cancellation_is_scoped_to_the_exact_draft_revision() {
        let control = DropboxSetupControl::default();
        let id = id();
        let (run, _) = control.begin(&id, 4, None).unwrap();
        assert!(control.begin(&id, 4, None).is_err());
        control.cancel(&id, 3).unwrap();
        assert!(!run.cancel.load(Ordering::Acquire));
        control.cancel(&id, 4).unwrap();
        assert!(run.cancel.load(Ordering::Acquire));
        drop(run);
        assert!(control.begin(&id, 4, None).is_ok());
    }
    #[test]
    fn stale_or_missing_session_fails_before_remote_work() {
        let control = DropboxSetupControl::default();
        assert!(
            matches!(control.begin(&id(),1,Some("stale")),Err(Failure { code }) if code == "dropbox_session_expired")
        );
        assert!(control.0.lock().unwrap().active.is_none());
    }
    #[test]
    fn authorization_links_pin_app_callback_and_origin_and_switch_is_explicit() {
        let normal = AuthorizationRequest::for_login(SETUP_SCOPES, false).unwrap();
        let switched = AuthorizationRequest::for_login(SETUP_SCOPES, true).unwrap();
        assert!(valid_link(&normal) && valid_link(&switched));
        assert!(!normal
            .url
            .query_pairs()
            .any(|(key, _)| key == "force_reauthentication"));
        assert!(switched
            .url
            .query_pairs()
            .any(|(key, value)| key == "force_reauthentication" && value == "true"));
        assert_ne!(normal.state, switched.state);
        let mut foreign = switched;
        foreign.url.set_host(Some("example.test")).unwrap();
        assert!(!valid_link(&foreign));
    }
    #[test]
    fn a_folder_row_keeps_its_english_beside_the_codes() {
        use mallard_infrastructure::storage_core::MarkerDetail;
        let folder = Folder {
            namespace: "1234".into(),
            name: "Team".into(),
            access: "viewer_no_comment".into(),
            mounted: false,
            marker: MarkerState::Unreadable,
            detail: Some(MarkerDetail::reason(
                MarkerDetailCode::ScanTimedOut,
                "Scan timed out",
            )),
        };
        let row = folder_candidate(2, &folder, None);
        assert_eq!(row.marker_detail.as_deref(), Some("Scan timed out"));
        assert_eq!(row.marker_detail_code, Some(MarkerDetailCode::ScanTimedOut));
        assert_eq!(row.marker_layout_version, None);
        assert_eq!(
            row.facts,
            [
                Fact {
                    key: "access",
                    label: "Access".into(),
                    value: "viewer_no_comment".into(),
                    value_code: "viewer_no_comment".into(),
                },
                Fact {
                    key: "location",
                    label: "Location".into(),
                    value: "Not mounted".into(),
                    value_code: "not_mounted".into(),
                },
            ]
        );
        assert_eq!(row.cautions, ["Read access does not permit Push."]);
        assert_eq!(row.caution_codes, ["read_only"]);
        let owned = folder_candidate(
            0,
            &Folder {
                access: "owner".into(),
                mounted: true,
                marker: MarkerState::Storage,
                detail: Some(MarkerDetail::layout(1)),
                ..folder
            },
            Some("storage-1".into()),
        );
        assert_eq!(owned.marker_detail.as_deref(), Some("layout 1"));
        assert_eq!(owned.marker_layout_version, Some(1));
        assert_eq!(owned.facts[1].value, "In Dropbox");
        assert!(owned.cautions.is_empty() && owned.caution_codes.is_empty());
    }
    #[test]
    fn draft_revision_provider_and_mode_are_checked_before_auth() {
        use mallard_application::storage_setup::create_storage_setup_draft;
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let saved = create_storage_setup_draft(
            &repository,
            StorageProviderRegistry::production(),
            &provider_key(),
        )
        .unwrap();
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

#[cfg(test)]
mod session_tests {
    use super::*;
    use mallard_domain::SecretValue;
    use mallard_infrastructure::dropbox_auth::AccessGrant;
    fn session(id: &StorageSetupDraftId) -> Session {
        Session {
            id: "session-fixture".into(),
            draft_id: id.clone(),
            revision: 4,
            mode: Mode::Existing,
            account: Account {
                id: "dbid:test".into(),
                name: "Test User".into(),
                email: "test@example.test".into(),
                home: "home".into(),
                root: "home".into(),
                team: false,
            },
            grant: AuthorizationGrant {
                access: AccessGrant {
                    token: SecretValue::parse("access-canary").unwrap(),
                    expires_at: now_secs() + 3600,
                },
                refresh: SecretValue::parse("refresh-canary").unwrap(),
                account_id: "dbid:test".into(),
            },
            expires: now_secs() + 600,
            scan: None,
        }
    }
    #[test]
    fn cancelled_results_cannot_restore_credentials_and_idle_cancel_checks_revision() {
        let id = StorageSetupDraftId::parse("storagedraft-111111111111111111111111").unwrap();
        let control = DropboxSetupControl::default();
        assert!(control.keep(session(&id), &AtomicBool::new(true)).is_err());
        assert!(control.0.lock().unwrap().session.is_none());
        control.0.lock().unwrap().session = Some(session(&id));
        control.cancel(&id, 3).unwrap();
        assert!(control.0.lock().unwrap().session.is_some());
        control.cancel(&id, 4).unwrap();
        assert!(control.0.lock().unwrap().session.is_none());
        let mut expired = session(&id);
        expired.expires = now_secs();
        control.0.lock().unwrap().session = Some(expired);
        assert!(control.begin(&id, 4, Some("session-fixture")).is_err());
        assert!(control.0.lock().unwrap().session.is_none());
    }
}
