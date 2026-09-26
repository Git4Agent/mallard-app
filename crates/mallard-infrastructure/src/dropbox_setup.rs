//! Exact-folder Dropbox provisioning. Nothing about an attempt is recorded
//! locally: a folder whose creation or share was interrupted stays in the
//! account, and the next attempt with the same name reports it taken.
pub mod discovery;
#[cfg(any(test, feature = "test-support"))]
pub mod mock;
#[cfg(test)]
mod tests;
use crate::dropbox_auth::{validate_id, AuthError, AuthorizationGrant, DropboxClient};
use crate::storage_names::{select_word, NameError};
use mallard_domain::SecretValue;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    Auth(AuthError),
    Identity,
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
            Self::Identity => "dropbox_setup_identity_mismatch",
            Self::Incomplete => "dropbox_setup_share_pending",
            Self::NamesExhausted => "dropbox_setup_names_exhausted",
            Self::NameTaken => "dropbox_folder_name_taken",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Auth(error) => error.message(),
            Self::Identity => "The Dropbox account or folder does not match this setup.",
            Self::Incomplete => "Dropbox is still preparing the shared folder. Find it in your Dropbox once it appears, or choose another name.",
            Self::NamesExhausted => "Mallard could not allocate another Dropbox folder name.",
            Self::NameTaken => "This folder name is already in use. Choose another name.",
        }
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

/// The namespace this setup stores in: `existing_namespace` verified, or a
/// new folder `folder_name` created in the account's home, shared, and
/// verified. A name the home already holds is reported as taken and nothing
/// is made. A lost answer is reported as Dropbox gave it; the folder it may
/// have left stays, and the next attempt with that name reports it taken.
pub async fn prepare_namespace(
    client: &DropboxClient,
    grant: &AuthorizationGrant,
    existing_namespace: Option<&str>,
    folder_name: Option<&str>,
) -> Result<String, SetupError> {
    let setup_account = account(client, &grant.access.token, &grant.account_id).await?;
    let root = home_root();
    let mut created_path = None;
    let namespace = if let Some(namespace) = existing_namespace {
        validate_id(namespace)?;
        namespace.to_string()
    } else {
        let name = folder_name
            .filter(|name| valid_folder_name(name))
            .ok_or(SetupError::Identity)?;
        let path = format!("/{name}");
        let folder_id = create_folder(client, grant, &root, &path).await?;
        let namespace =
            share_folder(client, grant, &root, &path, &folder_id, &setup_account).await?;
        created_path = Some(path);
        namespace
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

/// One exact-name creation, never renamed by Dropbox.
async fn create_folder(
    client: &DropboxClient,
    grant: &AuthorizationGrant,
    root: &Value,
    path: &str,
) -> Result<String, SetupError> {
    let response = client
        .rpc_evidenced(
            "files/create_folder_v2",
            json!({"path":path, "autorename":false}),
            &grant.access.token,
            Some(root),
        )
        .await
        .map_err(|failure| {
            let conflict = failure.evidence.as_ref().is_some_and(|evidence| {
                matches!(
                    evidence.error_tag,
                    Some("path/conflict/file" | "path/conflict/folder")
                )
            });
            if conflict {
                SetupError::NameTaken
            } else {
                failure.into()
            }
        })?;
    let metadata = &response["metadata"];
    let folder_id = id(&metadata["id"])?;
    if !folder_id.starts_with("id:")
        || metadata["path_lower"] != path.to_lowercase()
        || !metadata["sharing_info"].is_null()
        || !(metadata[".tag"].is_null() || metadata[".tag"] == "folder")
    {
        return Err(SetupError::Identity);
    }
    Ok(folder_id)
}

/// Share the created folder, waiting up to 30 seconds for an async job.
async fn share_folder(
    client: &DropboxClient,
    grant: &AuthorizationGrant,
    root: &Value,
    path: &str,
    folder_id: &str,
    account: &Account,
) -> Result<String, SetupError> {
    let shared = client
        .rpc(
            "sharing/share_folder",
            json!({"path":folder_id, "force_async":false}),
            &grant.access.token,
            Some(root),
        )
        .await?;
    let job_id = match share_result(&shared, path, account)? {
        Share::Complete(namespace_id) => return Ok(namespace_id),
        Share::Job(job_id) => job_id,
    };
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
        return match share_result(&shared, path, account)? {
            Share::Complete(namespace_id) => Ok(namespace_id),
            Share::Job(_) => Err(SetupError::Identity),
        };
    }
    Err(SetupError::Incomplete)
}

enum Share {
    Job(String),
    Complete(String),
}

fn share_result(value: &Value, path: &str, account: &Account) -> Result<Share, SetupError> {
    if value[".tag"] == "async_job_id" {
        return Ok(Share::Job(id(&value["async_job_id"])?));
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
    Ok(Share::Complete(namespace_id))
}
