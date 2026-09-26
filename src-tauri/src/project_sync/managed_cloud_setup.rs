//! Mallard Cloud sign-in for a storage draft. One command signs in through
//! the browser and writes the account and its token into the exact draft
//! the user reviewed; the link is handed to the page, never opened here.
use mallard_application::storage_setup::begin_storage_draft_operation;
use mallard_domain::{StorageSetupDraft, StorageSetupDraftId};
use mallard_infrastructure::{
    managed_cloud::{default_origin, origin_allowed, sign_in},
    persistence::LocalStateRepository,
    storage_providers::{managed_cloud::provider_key, StorageProviderRegistry},
};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Create,
    Reconnect,
}

/// A failure after the sign-in itself names the account it is about, so
/// the page can open the storage that already holds it.
#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
}
impl Failure {
    fn code(code: &str) -> Self {
        Self {
            code: code.into(),
            account: None,
        }
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

#[derive(Clone, Debug, Serialize)]
pub struct Account {
    pub id: String,
    pub login: String,
}

/// The signed-in account the draft now holds, with its token.
#[derive(Serialize)]
pub struct Login {
    account: Account,
    service_origin: String,
}

/// The one sign-in in flight: a browser wait must be stoppable, and a
/// second run for any draft waits for it.
struct Active {
    draft_id: StorageSetupDraftId,
    revision: u64,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct ManagedCloudSetupControl(Arc<Mutex<Option<Active>>>);

struct Run<'a> {
    control: &'a ManagedCloudSetupControl,
    cancel: Arc<AtomicBool>,
}
impl Drop for Run<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.control.0.lock() {
            *active = None;
        }
    }
}

impl ManagedCloudSetupControl {
    fn begin(&self, draft_id: &StorageSetupDraftId, revision: u64) -> Result<Run<'_>, Failure> {
        let mut active = self.0.lock().map_err(|_| Failure::local())?;
        if active.is_some() {
            return Err(Failure::code("managed_cloud_setup_busy"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some(Active {
            draft_id: draft_id.clone(),
            revision,
            cancel: cancel.clone(),
        });
        Ok(Run {
            control: self,
            cancel,
        })
    }

    fn cancel(&self, draft_id: &StorageSetupDraftId, revision: u64) -> Result<(), Failure> {
        let active = self.0.lock().map_err(|_| Failure::local())?;
        if let Some(active) = active.as_ref() {
            if active.draft_id == *draft_id && active.revision == revision {
                active.cancel.store(true, Ordering::Release);
            }
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

/// Sign in through the browser, then write the account and its token into
/// the draft under its revision check. A reconnect accepts only the
/// storage's own account. Nothing is kept between the two steps.
#[tauri::command]
pub async fn sign_in_managed_cloud_storage(
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
    let run = control.begin(&draft_id, expected_revision)?;
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
    if run.cancel.load(Ordering::Acquire) {
        return Err(Failure::code("managed_cloud_setup_cancelled"));
    }
    let account = Account {
        id: signed.user.id,
        login: signed.user.github_login,
    };
    let about_account = |code: &str| Failure {
        code: code.into(),
        account: Some(account.clone()),
    };
    if mode == Mode::Reconnect && pinned_account.as_deref() != Some(account.id.as_str()) {
        return Err(about_account("managed_cloud_account_mismatch"));
    }
    // The draft operation pins the reviewed revision: a draft that moved
    // during the browser step is refused here.
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
            service_origin: &origin,
            account_id: &account.id,
            account_login: &account.login,
            token: &signed.token,
        },
    )
    .map_err(|e| about_account(e.code()))?;
    Ok(Login {
        account,
        service_origin: origin,
    })
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
    #[test]
    fn single_flight_cancellation_is_scoped_to_the_exact_draft_revision() {
        let control = ManagedCloudSetupControl::default();
        let id = id();
        let run = control.begin(&id, 4).unwrap();
        assert!(matches!(
            control.begin(&id, 4),
            Err(Failure { code, .. }) if code == "managed_cloud_setup_busy"
        ));
        control.cancel(&id, 3).unwrap();
        assert!(!run.cancel.load(Ordering::Acquire));
        control.cancel(&id, 4).unwrap();
        assert!(run.cancel.load(Ordering::Acquire));
        drop(run);
        assert!(control.begin(&id, 4).is_ok());
    }
    #[test]
    fn draft_revision_provider_mode_and_origin_are_checked_before_sign_in() {
        use mallard_application::storage_setup::create_storage_setup_draft;
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        // No build registers Mallard Cloud, so the draft comes from a
        // registry that holds its adapter alone.
        let registry = StorageProviderRegistry::from_adapters(vec![std::sync::Arc::new(
            mallard_infrastructure::storage_providers::managed_cloud::ManagedCloudAdapter,
        )])
        .unwrap();
        let saved = create_storage_setup_draft(&repository, &registry, &provider_key()).unwrap();
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
