//! Mallard Cloud sign-in for a storage draft. The token lives in one
//! ephemeral Rust session between the browser step and the prepare; the
//! link is handed to the page, never opened here.
use mallard_application::storage_setup::begin_storage_draft_operation;
use mallard_domain::{now_secs, HostId, SecretValue, StorageSetupDraft, StorageSetupDraftId};
use mallard_infrastructure::{
    managed_cloud::{client::User, default_origin, origin_allowed, sign_in},
    persistence::LocalStateRepository,
    storage_providers::{managed_cloud::provider_key, StorageProviderRegistry},
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
}
impl Failure {
    fn code(code: &str) -> Self {
        Self { code: code.into() }
    }
    fn local() -> Self {
        Self::code("managed_cloud_setup_failed")
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Progress {
    AuthorizationLink { url: String, expires_in: u32 },
}

#[derive(Serialize)]
pub struct Account {
    pub id: String,
    pub login: String,
}

#[derive(Serialize)]
pub struct Login {
    session_id: String,
    account: Account,
    service_origin: String,
}

struct Session {
    id: String,
    draft_id: StorageSetupDraftId,
    revision: u64,
    origin: String,
    user: User,
    token: SecretValue,
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
pub struct ManagedCloudSetupControl(Arc<Mutex<State>>);

struct Run<'a> {
    control: &'a ManagedCloudSetupControl,
    cancel: Arc<AtomicBool>,
}
impl Drop for Run<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.control.0.lock() {
            state.active = None;
        }
    }
}

impl ManagedCloudSetupControl {
    fn begin(
        &self,
        draft_id: &StorageSetupDraftId,
        revision: u64,
        session_id: Option<&str>,
    ) -> Result<(Run<'_>, Option<Session>), Failure> {
        let mut state = self.0.lock().map_err(|_| Failure::local())?;
        if state.active.is_some() {
            return Err(Failure::code("managed_cloud_setup_busy"));
        }
        let session = state.session.take();
        if let Some(id) = session_id {
            if !session.as_ref().is_some_and(|session| {
                session.id == id
                    && session.draft_id == *draft_id
                    && session.revision == revision
                    && now_secs() < session.expires
            }) {
                return Err(Failure::code("managed_cloud_session_expired"));
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
            return Err(Failure::code("managed_cloud_setup_cancelled"));
        }
        session.expires = now_secs() + 600;
        let id = session.id.clone();
        let expires = session.expires;
        {
            let mut state = self.0.lock().map_err(|_| Failure::local())?;
            if cancel.load(Ordering::Acquire) {
                return Err(Failure::code("managed_cloud_setup_cancelled"));
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
        .ok_or_else(|| Failure::code("managed_cloud_draft_changed"))?;
    if draft.revision != revision
        || draft.storage.provider != provider_key()
        || matches!(
            draft.source,
            mallard_domain::StorageSetupDraftSource::Edit { .. }
        ) != (mode == Mode::Reconnect)
    {
        return Err(Failure::code("managed_cloud_draft_changed"));
    }
    Ok(draft)
}

/// The origin a draft signs into: the one it already names, else the
/// build's default.
fn origin_for(draft: &StorageSetupDraft) -> Result<String, Failure> {
    match draft
        .storage
        .provider_config
        .settings
        .get_str("service_origin")
        .filter(|origin| !origin.is_empty())
    {
        Some(origin) if origin_allowed(origin) => Ok(origin.to_owned()),
        Some(_) => Err(Failure::code("managed_cloud_draft_changed")),
        None => Ok(default_origin()),
    }
}

#[tauri::command]
pub async fn authorize_managed_cloud_setup(
    control: tauri::State<'_, ManagedCloudSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    mode: Mode,
    on_progress: tauri::ipc::Channel<Progress>,
) -> Result<Login, Failure> {
    let repository = repository()?;
    let current = draft(&repository, &draft_id, expected_revision, mode)?;
    let origin = origin_for(&current)?;
    let pinned_account = current
        .storage
        .provider_config
        .settings
        .get_str("account_id")
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let (run, _) = control.begin(&draft_id, expected_revision, None)?;
    let signed = sign_in(
        &origin,
        move |url| {
            on_progress
                .send(Progress::AuthorizationLink {
                    url: url.to_string(),
                    expires_in: 180,
                })
                .map_err(|_| ())
        },
        run.cancel.clone(),
    )
    .await
    .map_err(|error| Failure::code(error.code()))?;
    draft(&repository, &draft_id, expected_revision, mode)?;
    if mode == Mode::Reconnect && pinned_account.as_deref() != Some(signed.user.id.as_str()) {
        return Err(Failure::code("managed_cloud_account_mismatch"));
    }
    let id = HostId::generate()
        .map_err(|_| Failure::local())?
        .to_string();
    let login = Login {
        session_id: id.clone(),
        account: Account {
            id: signed.user.id.clone(),
            login: signed.user.github_login.clone(),
        },
        service_origin: origin.clone(),
    };
    control.keep(
        Session {
            id,
            draft_id,
            revision: expected_revision,
            origin,
            user: signed.user,
            token: signed.token,
            expires: 0,
        },
        &run.cancel,
    )?;
    Ok(login)
}

#[tauri::command]
pub async fn prepare_managed_cloud_storage(
    control: tauri::State<'_, ManagedCloudSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    session_id: String,
) -> Result<(), Failure> {
    let repository = repository()?;
    let (run, session) = control.begin(&draft_id, expected_revision, Some(&session_id))?;
    let session = session.ok_or_else(Failure::local)?;
    let current = repository
        .load_storage_setup_draft(&draft_id)
        .map_err(|_| Failure::local())?
        .ok_or_else(|| Failure::code("managed_cloud_draft_changed"))?;
    if run.cancel.load(Ordering::Acquire) {
        return Err(Failure::code("managed_cloud_setup_cancelled"));
    }
    let operation = begin_storage_draft_operation(
        &repository,
        &draft_id,
        expected_revision,
        &current.storage.name,
    )
    .map_err(|e| Failure::code(e.code()))?;
    mallard_application::managed_cloud_setup::prepare(
        &repository,
        StorageProviderRegistry::production(),
        &operation,
        &draft_id,
        mallard_application::managed_cloud_setup::SignedInAccount {
            service_origin: &session.origin,
            account_id: &session.user.id,
            account_login: &session.user.github_login,
            token: &session.token,
        },
    )
    .map_err(|e| Failure::code(e.code()))?;
    Ok(())
}

#[tauri::command]
pub fn cancel_managed_cloud_setup(
    control: tauri::State<'_, ManagedCloudSetupControl>,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
) -> Result<(), Failure> {
    control.cancel(&draft_id, expected_revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> StorageSetupDraftId {
        StorageSetupDraftId::parse("storagedraft-222222222222222222222222").unwrap()
    }
    fn session(id: &StorageSetupDraftId) -> Session {
        Session {
            id: "session-fixture".into(),
            draft_id: id.clone(),
            revision: 4,
            origin: mallard_infrastructure::managed_cloud::PRODUCTION_ORIGIN.into(),
            user: User {
                id: "user_1".into(),
                github_login: "octocat".into(),
                service_role: "user".into(),
            },
            token: SecretValue::parse("desktop-session-token-canary-0123").unwrap(),
            expires: now_secs() + 600,
        }
    }
    #[test]
    fn single_flight_cancellation_is_scoped_to_the_exact_draft_revision() {
        let control = ManagedCloudSetupControl::default();
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
    fn stale_sessions_and_cancelled_results_never_keep_a_token() {
        let control = ManagedCloudSetupControl::default();
        let id = id();
        assert!(
            matches!(control.begin(&id, 1, Some("stale")), Err(Failure { code }) if code == "managed_cloud_session_expired")
        );
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
    }
    #[test]
    fn draft_revision_provider_mode_and_origin_are_checked_before_sign_in() {
        use mallard_application::storage_setup::create_storage_setup_draft;
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let saved = create_storage_setup_draft(
            &repository,
            StorageProviderRegistry::production(),
            &provider_key(),
        )
        .unwrap();
        let current = draft(&repository, &saved.draft_id, saved.revision, Mode::Create).unwrap();
        assert_eq!(origin_for(&current).unwrap(), default_origin());
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
