//! Resumable, exact-folder Dropbox provisioning. This journal has no secrets.
pub mod discovery;
#[cfg(any(test, feature = "test-support"))]
pub mod mock;
#[cfg(test)]
mod tests;
use crate::dropbox_auth::{validate_id, AuthError, AuthorizationGrant, DropboxClient};
use crate::persistence::LocalStateRepository;
use crate::storage_names::{select_word, NameError};
use crate::storage_setup_journal::{
    StorageSetupId, StorageSetupJournalEntry, StorageSetupJournalStore, StorageSetupStage,
};
use mallard_domain::{
    now_secs as unix_timestamp, sha256_hex, BoundedConfigObject, SecretValue, StorageSetupDraftId,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    Auth(AuthError),
    Journal,
    Identity,
    Uncertain,
    Incomplete,
    NamesExhausted,
    NameTaken,
}
impl From<AuthError> for SetupError {
    fn from(value: AuthError) -> Self {
        Self::Auth(value)
    }
}
impl From<crate::dropbox_auth::AuthFailure> for SetupError {
    fn from(value: crate::dropbox_auth::AuthFailure) -> Self {
        Self::Auth(value.error)
    }
}
impl SetupError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Auth(error) => error.code(),
            Self::Journal => "dropbox_setup_journal_failed",
            Self::Identity => "dropbox_setup_identity_mismatch",
            Self::Uncertain => "dropbox_setup_outcome_unknown",
            Self::Incomplete => "dropbox_setup_share_pending",
            Self::NamesExhausted => "dropbox_setup_names_exhausted",
            Self::NameTaken => "dropbox_folder_name_taken",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Auth(error) => error.message(),
            Self::Journal => "Mallard could not read or save Dropbox setup recovery state.",
            Self::Identity => "The Dropbox account or folder does not match this setup. No replacement was accepted.",
            Self::Uncertain => "A Dropbox folder request has an unknown result. This draft will not create or share another folder. Inspect Dropbox before starting a new setup.",
            Self::Incomplete => "Dropbox is still preparing the shared folder. Run setup again to check the same job.",
            Self::NamesExhausted => "Mallard could not allocate another Dropbox folder name.",
            Self::NameTaken => "This folder name is already in use. Choose another name.",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    draft_id: StorageSetupDraftId,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    account_id: String,
    home_namespace_id: String,
    path: String,
    stage: FolderStage,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
enum FolderStage {
    Planned,
    Conflict,
    Creating,
    Created { folder_id: String },
    Sharing { folder_id: String },
    Job { job_id: String },
    Shared { namespace_id: String },
}

pub struct DropboxSetupJournal {
    store: StorageSetupJournalStore,
    entry: StorageSetupJournalEntry,
}
impl DropboxSetupJournal {
    pub fn open(
        repository: &LocalStateRepository,
        draft_id: &StorageSetupDraftId,
    ) -> Result<Self, SetupError> {
        let store = StorageSetupJournalStore::dropbox_provisioning(repository);
        let digest = sha256_hex(draft_id.as_str().as_bytes());
        let setup_id = StorageSetupId::parse(format!("setup-{}", &digest[..24]))
            .map_err(|_| SetupError::Journal)?;
        let facts = BoundedConfigObject::encode(&Facts {
            draft_id: draft_id.clone(),
        })
        .map_err(|_| SetupError::Journal)?;
        let entry = match store.load(&setup_id).map_err(|_| SetupError::Journal)? {
            Some(entry) => {
                if entry.provider != crate::storage_providers::dropbox::provider_key()
                    || entry.config_version != 1
                    || entry.facts != facts
                    || entry.stage != StorageSetupStage::Provisioning
                {
                    return Err(SetupError::Journal);
                }
                entry
            }
            None => StorageSetupJournalEntry {
                schema: 1,
                setup_id,
                provider: crate::storage_providers::dropbox::provider_key(),
                config_version: 1,
                stage: StorageSetupStage::Provisioning,
                error_code: None,
                created_at: unix_timestamp(),
                updated_at: unix_timestamp(),
                facts,
                recovery: BoundedConfigObject::empty(),
            },
        };
        Ok(Self { store, entry })
    }

    fn recovery_named(&self, account: &Account, requested: &str) -> Result<Recovery, SetupError> {
        if !valid_folder_name(requested) {
            return Err(SetupError::Identity);
        }
        if self.entry.recovery == BoundedConfigObject::empty() {
            return Ok(Recovery {
                account_id: account.id.clone(),
                home_namespace_id: account.home.clone(),
                path: format!("/{requested}"),
                stage: FolderStage::Planned,
            });
        }
        let mut recovery: Recovery = self
            .entry
            .recovery
            .decode()
            .map_err(|_| SetupError::Journal)?;
        if recovery.account_id != account.id || recovery.home_namespace_id != account.home {
            return Err(SetupError::Identity);
        }
        {
            if recovery.path != format!("/{requested}") {
                if !matches!(recovery.stage, FolderStage::Planned | FolderStage::Conflict) {
                    return Err(SetupError::Identity);
                }
                recovery.path = format!("/{requested}");
                recovery.stage = FolderStage::Planned;
            }
        }
        if !recovery
            .path
            .strip_prefix('/')
            .is_some_and(valid_folder_name)
        {
            return Err(SetupError::Journal);
        }
        match &recovery.stage {
            FolderStage::Created { folder_id } | FolderStage::Sharing { folder_id } => {
                validate_id(folder_id)?;
                if !folder_id.starts_with("id:") {
                    return Err(SetupError::Journal);
                }
            }
            FolderStage::Job { job_id } => validate_id(job_id)?,
            FolderStage::Shared { namespace_id } => validate_id(namespace_id)?,
            FolderStage::Planned | FolderStage::Conflict | FolderStage::Creating => {}
        }
        Ok(recovery)
    }

    pub fn pinned_account(&self) -> Result<Option<String>, SetupError> {
        if self.entry.recovery == BoundedConfigObject::empty() {
            return Ok(None);
        }
        let recovery: Recovery = self
            .entry
            .recovery
            .decode()
            .map_err(|_| SetupError::Journal)?;
        Ok(Some(recovery.account_id))
    }

    pub fn folder_name(&self) -> Result<Option<String>, SetupError> {
        if self.entry.recovery == BoundedConfigObject::empty() {
            return Ok(None);
        }
        let recovery: Recovery = self
            .entry
            .recovery
            .decode()
            .map_err(|_| SetupError::Journal)?;
        Ok(recovery.path.strip_prefix('/').map(str::to_string))
    }

    fn checkpoint(&mut self, recovery: &Recovery) -> Result<(), SetupError> {
        self.entry.recovery =
            BoundedConfigObject::encode(recovery).map_err(|_| SetupError::Journal)?;
        self.entry.updated_at = unix_timestamp().max(self.entry.updated_at);
        self.store
            .save(&self.entry)
            .map_err(|_| SetupError::Journal)
    }

    pub fn finish(self) -> Result<(), SetupError> {
        self.store
            .remove(&self.entry.setup_id)
            .map(|_| ())
            .map_err(|_| SetupError::Journal)
    }
}

fn name_error(error: NameError) -> SetupError {
    match error {
        NameError::Exhausted => SetupError::NamesExhausted,
        NameError::Random => SetupError::Auth(AuthError::Local),
    }
}

struct Account {
    id: String,
    home: String,
    root: String,
    team: bool,
}
async fn account(
    client: &DropboxClient,
    token: &SecretValue,
    expected: &str,
) -> Result<Account, SetupError> {
    let value = client
        .rpc("users/get_current_account", Value::Null, token, None)
        .await?;
    let result = Account {
        id: id(&value["account_id"])?,
        home: id(&value["root_info"]["home_namespace_id"])?,
        root: id(&value["root_info"]["root_namespace_id"])?,
        team: value["root_info"][".tag"] == "team",
    };
    if result.id != expected {
        return Err(SetupError::Identity);
    }
    Ok(result)
}
fn id(value: &Value) -> Result<String, SetupError> {
    let value = value.as_str().ok_or(SetupError::Identity)?;
    validate_id(value)?;
    Ok(value.into())
}
fn home_root() -> Value {
    json!({".tag":"home"})
}

async fn verify_namespace(
    client: &DropboxClient,
    token: &SecretValue,
    account: &Account,
    namespace: &str,
) -> Result<(), SetupError> {
    validate_id(namespace)?;
    if namespace == account.home || namespace == account.root && !account.team {
        return Err(SetupError::Identity);
    }
    let value = client
        .rpc(
            "files/list_folder",
            json!({"path":"", "recursive":false, "limit":1}),
            token,
            Some(&json!({".tag":"namespace_id", "namespace_id":namespace})),
        )
        .await?;
    if !value["entries"]
        .as_array()
        .is_some_and(|entries| entries.len() <= 1)
        || !value["has_more"].is_boolean()
        || !value["cursor"].is_string()
    {
        return Err(SetupError::Identity);
    }
    Ok(())
}

pub fn valid_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.trim() == name
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
}

pub fn propose_folder_name() -> Result<String, SetupError> {
    Ok(format!("Mallard {}", select_word(&[]).map_err(name_error)?))
}

pub async fn prepare_namespace(
    client: &DropboxClient,
    grant: &AuthorizationGrant,
    existing_namespace: Option<&str>,
    journal: &mut DropboxSetupJournal,
    requested: Option<&str>,
) -> Result<String, SetupError> {
    let setup_account = account(client, &grant.access.token, &grant.account_id).await?;
    let root = home_root();
    let mut created_path = None;
    let namespace = if let Some(namespace) = existing_namespace {
        validate_id(namespace)?;
        namespace.to_string()
    } else {
        let mut recovery =
            journal.recovery_named(&setup_account, requested.ok_or(SetupError::Identity)?)?;
        loop {
            let path = recovery.path.clone();
            match recovery.stage.clone() {
                FolderStage::Conflict => return Err(SetupError::NameTaken),
                FolderStage::Planned => {
                    recovery.stage = FolderStage::Creating;
                    journal.checkpoint(&recovery)?;
                    let response = client
                        .rpc_evidenced(
                            "files/create_folder_v2",
                            json!({"path":path, "autorename":false}),
                            &grant.access.token,
                            Some(&root),
                        )
                        .await;
                    let response = match response {
                        Ok(response) => response,
                        Err(failure) => {
                            let evidence = failure.evidence.as_ref();
                            if evidence.is_some_and(|evidence| {
                                matches!(
                                    evidence.error_tag,
                                    Some("path/conflict/file" | "path/conflict/folder")
                                )
                            }) {
                                recovery.stage = FolderStage::Conflict;
                                journal.checkpoint(&recovery)?;
                                continue;
                            }
                            // A definite rejection permits a later retry of the same name.
                            // Missing, malformed, and server responses keep Creating.
                            if evidence.is_some_and(|evidence| {
                                matches!(evidence.http_status, Some(400 | 401 | 403 | 409 | 429))
                            }) {
                                recovery.stage = FolderStage::Planned;
                                journal.checkpoint(&recovery)?;
                            }
                            return Err(failure.into());
                        }
                    };
                    let metadata = &response["metadata"];
                    let folder_id = id(&metadata["id"])?;
                    if !folder_id.starts_with("id:")
                        || metadata["path_lower"] != path.to_lowercase()
                        || !metadata["sharing_info"].is_null()
                        || !(metadata[".tag"].is_null() || metadata[".tag"] == "folder")
                    {
                        return Err(SetupError::Identity);
                    }
                    recovery.stage = FolderStage::Created { folder_id };
                    journal.checkpoint(&recovery)?;
                }
                FolderStage::Creating | FolderStage::Sharing { .. } => {
                    return Err(SetupError::Uncertain)
                }
                FolderStage::Created { folder_id } => {
                    let metadata = client
                        .rpc(
                            "files/get_metadata",
                            json!({"path":folder_id}),
                            &grant.access.token,
                            Some(&root),
                        )
                        .await?;
                    if metadata[".tag"] != "folder"
                        || metadata["id"] != folder_id
                        || metadata["path_lower"] != path.to_lowercase()
                        || !metadata["sharing_info"].is_null()
                    {
                        return Err(SetupError::Identity);
                    }
                    recovery.stage = FolderStage::Sharing {
                        folder_id: folder_id.clone(),
                    };
                    journal.checkpoint(&recovery)?;
                    let shared = client
                        .rpc(
                            "sharing/share_folder",
                            json!({"path":folder_id, "force_async":false}),
                            &grant.access.token,
                            Some(&root),
                        )
                        .await?;
                    recovery.stage = share_result(&shared, &path, &setup_account)?;
                    journal.checkpoint(&recovery)?;
                }
                FolderStage::Job { job_id } => {
                    let mut complete = false;
                    for _ in 0..30 {
                        let shared = client
                            .rpc(
                                "sharing/check_share_job_status",
                                json!({"async_job_id":job_id}),
                                &grant.access.token,
                                None,
                            )
                            .await?;
                        if shared[".tag"] == "in_progress" {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                        recovery.stage = share_result(&shared, &path, &setup_account)?;
                        if !matches!(recovery.stage, FolderStage::Shared { .. }) {
                            return Err(SetupError::Identity);
                        }
                        journal.checkpoint(&recovery)?;
                        complete = true;
                        break;
                    }
                    if !complete {
                        return Err(SetupError::Incomplete);
                    }
                }
                FolderStage::Shared { namespace_id } => {
                    created_path = Some(path);
                    break namespace_id;
                }
            }
        }
    };
    let refreshed = client.refresh(&grant.refresh).await?;
    let refreshed_account = account(client, &refreshed.token, &grant.account_id).await?;
    if let Some(path) = created_path {
        let metadata = client
            .rpc(
                "files/get_metadata",
                json!({"path":path}),
                &refreshed.token,
                Some(&root),
            )
            .await?;
        if metadata[".tag"] != "folder"
            || metadata["path_lower"] != path.to_lowercase()
            || metadata["sharing_info"]["shared_folder_id"] != namespace
        {
            return Err(SetupError::Identity);
        }
    }
    verify_namespace(client, &refreshed.token, &refreshed_account, &namespace).await?;
    Ok(namespace)
}

fn share_result(value: &Value, path: &str, account: &Account) -> Result<FolderStage, SetupError> {
    if value[".tag"] == "async_job_id" {
        return Ok(FolderStage::Job {
            job_id: id(&value["async_job_id"])?,
        });
    }
    if value[".tag"] != "complete" {
        return Err(SetupError::Incomplete);
    }
    let namespace_id = id(&value["shared_folder_id"])?;
    if namespace_id == account.home
        || namespace_id == account.root
        || value["access_type"][".tag"] != "owner"
        || value["path_lower"] != path.to_lowercase()
    {
        return Err(SetupError::Identity);
    }
    Ok(FolderStage::Shared { namespace_id })
}
