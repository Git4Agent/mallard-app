//! Feishu Drive through Mallard's own app. The folder token is the storage
//! identity; the connection and the account are connection facts. The
//! configuration holds no secret: the app's credentials ship with the
//! build, and the refresh chain lives in the connection document under
//! `~/.mallard`.
use super::traits::*;
use crate::dropbox_setup::valid_folder_name;
use crate::feishu_auth::{FeishuApp, FeishuAuthorization, FeishuClient};
use crate::feishu_connections::{self, ConnectionId};
use crate::storage_core::{feishu_store::FeishuObjectStore, ObjectStore};
use mallard_domain::{BoundedConfigObject, FieldKey, StorageProviderConfig, StorageProviderKey};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::runtime::Handle;

pub const SETUP_GUIDE_URL: &str =
    "https://github.com/Git4Agent/mallard-app/blob/main/docs/FEISHU_APP_SETUP_GUIDE.md";
const MAX_ACCOUNT_NAME_BYTES: usize = 256;

pub struct FeishuAdapter;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FeishuSettingsV1 {
    /// Names this machine's connection document; minted at sign-in.
    pub connection_id: String,
    /// The Drive folder the storage lives in: the identity.
    pub folder_token: String,
    /// What listings call the folder. Never an identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
    /// The signed-in user's name, for the storage page.
    pub account_name: String,
}

pub fn provider_key() -> StorageProviderKey {
    StorageProviderKey::parse("feishu").expect("static key")
}

fn invalid() -> StorageAdapterError {
    StorageAdapterError::invalid(
        "feishu_config_invalid",
        "Sign in with Feishu and choose a folder.",
    )
}

fn decode(config: &StorageProviderConfig) -> Result<FeishuSettingsV1, StorageAdapterError> {
    FeishuAdapter.validate_config(config)?;
    config.settings.decode().map_err(|_| invalid())
}

/// The connection document a saved configuration names.
pub fn connection_id(config: &StorageProviderConfig) -> Option<ConnectionId> {
    config
        .settings
        .get_str("connection_id")
        .and_then(|id| ConnectionId::parse(id).ok())
}

/// Delete the connection document a removed storage named. An orphan is
/// harmless, so a failure here is not one.
pub fn forget_connection(config: &StorageProviderConfig) {
    if let (Some(store), Some(id)) = (feishu_connections::installed(), connection_id(config)) {
        let _ = store.remove(&id);
    }
}

/// The Unix time this machine must sign in again by, from the document.
fn reauthorize_at(settings: &FeishuSettingsV1) -> Option<u64> {
    let store = feishu_connections::installed()?;
    let id = ConnectionId::parse(&settings.connection_id).ok()?;
    store
        .load(&id)
        .ok()
        .flatten()
        .map(|document| document.reauthorize_at)
}

fn store(
    config: &StorageProviderConfig,
    runtime: Handle,
) -> Result<FeishuObjectStore, StorageAdapterError> {
    let settings = decode(config)?;
    let connections = feishu_connections::installed().ok_or_else(|| {
        StorageAdapterError::invalid(
            "feishu_connections_unavailable",
            "This build cannot open a Feishu storage: it keeps no Feishu sign-in.",
        )
    })?;
    let app = FeishuApp::built_in().ok_or_else(|| {
        StorageAdapterError::invalid(
            "feishu_app_unavailable",
            "This build carries no Feishu app: a release build carries it, and a development build reads MALLARD_FEISHU_APP_ID and MALLARD_FEISHU_APP_SECRET from its environment.",
        )
    })?;
    let client = FeishuClient::new()
        .map_err(|error| StorageAdapterError::internal("feishu_client_failed", error.message()))?;
    FeishuObjectStore::new(
        settings.folder_token,
        FeishuAuthorization::new(
            app,
            ConnectionId::parse(&settings.connection_id).map_err(|_| invalid())?,
            connections,
            client,
        ),
        runtime,
    )
    .map_err(|_| invalid())
}

impl StorageProviderAdapter for FeishuAdapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        StorageProviderDescriptor {
            key: provider_key(),
            config_version: 1,
            display_name: "Feishu".into(),
            description: "A Feishu Drive folder. Up to 100 MiB per object.".into(),
            default_name: "Feishu storage".into(),
            name_suggestion: Some(StorageNameSuggestion::whole_setting(
                "folder_name",
                "Feishu {1}",
            )),
            display_rank: 25,
            icon: "cloud".into(),
            clear_mode: StorageClearMode::Unavailable,
            fields: vec![StorageFieldDescriptor {
                field_id: FieldKey::parse("folder_token").expect("static key"),
                label: "Folder token".into(),
                kind: StorageFieldKind::Text,
                required: true,
                placeholder: Some("The Drive folder's token".into()),
                help: Some("The Drive folder this storage lives in. Fixed after setup; share it in Feishu with each user who connects.".into()),
                help_url: Some(SETUP_GUIDE_URL.into()),
                derive: None,
            }],
            secrets: vec![],
            managed_secret_id: None,
            local_setup: Some(StorageLocalSetup::Feishu),
            setup_guide_url: Some(SETUP_GUIDE_URL.into()),
            manual_setup_hint: None,
            setup_doors: Some(StorageSetupDoors {
                create_hint: "A new folder in your My Space.".into(),
                existing_hint: "A folder shared with you in Feishu.".into(),
                existing_help: None,
            }),
            discovery: None,
        }
    }

    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> Result<BoundedConfigObject, StorageAdapterError> {
        let typed: FeishuSettingsV1 = settings.decode().map_err(|_| invalid())?;
        if ConnectionId::parse(&typed.connection_id).is_err()
            || crate::feishu_auth::validate_id(&typed.folder_token).is_err()
            || typed
                .folder_name
                .as_deref()
                .is_some_and(|name| !valid_folder_name(name))
            || typed.account_name.trim().is_empty()
            || typed.account_name.len() > MAX_ACCOUNT_NAME_BYTES
            || typed.account_name.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        BoundedConfigObject::encode(&typed).map_err(|_| invalid())
    }

    /// No secret at all: the app's credentials are the build's, and the
    /// refresh chain is the connection document's.
    fn validate_config(&self, config: &StorageProviderConfig) -> Result<(), StorageAdapterError> {
        if config.version != 1 || !config.secrets.is_empty() {
            return Err(invalid());
        }
        self.validate_settings(&config.settings)?;
        Ok(())
    }

    /// The folder's name leads, then the token that identifies the storage,
    /// the account, and the date this machine must sign in again by.
    fn summary(&self, config: &StorageProviderConfig) -> Result<Vec<StorageSummaryField>, String> {
        let typed = decode(config).map_err(|e| e.to_string())?;
        let field = |id: &str, label: &str, value: String| -> Result<StorageSummaryField, String> {
            Ok(StorageSummaryField {
                field_id: FieldKey::parse(id)?,
                label: label.into(),
                value,
                is_path: false,
            })
        };
        let mut facts = Vec::new();
        if let Some(name) = &typed.folder_name {
            facts.push(field("folder_name", "Folder", name.clone())?);
        }
        facts.push(field(
            "folder_token",
            "Folder token",
            typed.folder_token.clone(),
        )?);
        facts.push(field(
            "account_name",
            "Account",
            typed.account_name.clone(),
        )?);
        let by = match reauthorize_at(&typed) {
            Some(at) => chrono::DateTime::from_timestamp(at as i64, 0)
                .map(|date| date.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "unknown".into()),
            None => "reconnect needed".into(),
        };
        facts.push(field("reauthorize_by", "Sign in again by", by)?);
        Ok(facts)
    }

    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> Result<Vec<String>, String> {
        Ok(vec![
            decode(config).map_err(|e| e.to_string())?.folder_token,
        ])
    }

    fn local_root(&self, _config: &StorageProviderConfig) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }

    fn probe(
        &self,
        config: &StorageProviderConfig,
        runtime: &Handle,
    ) -> Result<(), StorageAdapterError> {
        let store = store(config, runtime.clone())?;
        store.probe().map_err(|error| store.adapter_error(error))
    }

    fn open(
        &self,
        config: &StorageProviderConfig,
        runtime: Option<Handle>,
    ) -> Result<Arc<dyn ObjectStore>, String> {
        Ok(Arc::new(
            store(
                config,
                runtime.ok_or("Feishu requires an asynchronous runtime")?,
            )
            .map_err(|e| e.to_string())?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::{SecretKey, SecretMap, SecretValue};

    pub(crate) fn settings(folder_token: &str) -> FeishuSettingsV1 {
        FeishuSettingsV1 {
            connection_id: "0123456789abcdef0123456789abcdef".into(),
            folder_token: folder_token.into(),
            folder_name: Some("Mallard fox".into()),
            account_name: "Test User".into(),
        }
    }

    pub(crate) fn config(folder_token: &str) -> StorageProviderConfig {
        StorageProviderConfig::new(
            1,
            BoundedConfigObject::encode(&settings(folder_token)).unwrap(),
            SecretMap::empty(),
        )
    }

    #[test]
    fn descriptor_declares_one_field_no_secret_no_managed_secret_and_no_clear() {
        let descriptor = FeishuAdapter.descriptor();
        assert!(descriptor.secrets.is_empty());
        assert_eq!(
            descriptor
                .fields
                .iter()
                .map(|field| field.field_id.as_str())
                .collect::<Vec<_>>(),
            vec!["folder_token"]
        );
        assert!(descriptor.managed_secret_id.is_none());
        assert_eq!(descriptor.local_setup, Some(StorageLocalSetup::Feishu));
        assert_eq!(descriptor.clear_mode, StorageClearMode::Unavailable);
        assert!(descriptor.manual_setup_hint.is_none());
        // Create new or Use existing, but no look through the Drive: an
        // existing folder arrives as a pasted link.
        assert!(descriptor.setup_doors.is_some());
        assert!(descriptor.discovery.is_none());
        assert_eq!(descriptor.display_rank, 25);
    }

    #[test]
    fn validation_refuses_any_secret_and_malformed_ids() {
        let good = config("fldcnabc123");
        FeishuAdapter.validate_config(&good).unwrap();
        let mut with_secret = good.clone();
        with_secret
            .secrets
            .insert(
                SecretKey::parse("app_secret").unwrap(),
                SecretValue::parse("never-here").unwrap(),
            )
            .unwrap();
        assert!(FeishuAdapter.validate_config(&with_secret).is_err());
        for (field, value) in [
            ("connection_id", "0123"),
            ("connection_id", "0123456789ABCDEF0123456789abcdef"),
            ("folder_token", ""),
            ("folder_token", "has space"),
            ("folder_name", "a/b"),
            ("account_name", " "),
        ] {
            let mut json = serde_json::to_value(settings("fldcnabc123")).unwrap();
            json[field] = serde_json::json!(value);
            let settings = BoundedConfigObject::from_json(&json).unwrap();
            assert!(
                FeishuAdapter.validate_settings(&settings).is_err(),
                "{field}={value:?}"
            );
        }
        for extra in ["refresh_token", "app_id", "app_secret"] {
            let mut json = serde_json::to_value(settings("fldcnabc123")).unwrap();
            json[extra] = serde_json::json!("never-here");
            assert!(FeishuAdapter
                .validate_settings(&BoundedConfigObject::from_json(&json).unwrap())
                .is_err());
        }
        let mut next = good.clone();
        next.version = 2;
        assert!(FeishuAdapter.validate_config(&next).is_err());
    }

    #[test]
    fn identity_is_the_folder_and_nothing_private_reaches_the_summary() {
        let first = config("fldcnabc123");
        let mut other_connection = settings("fldcnabc123");
        other_connection.connection_id = "ffffffffffffffffffffffffffffffff".into();
        other_connection.account_name = "Another User".into();
        let second = StorageProviderConfig::new(
            1,
            BoundedConfigObject::encode(&other_connection).unwrap(),
            SecretMap::empty(),
        );
        assert_eq!(
            FeishuAdapter.continuity_coordinates(&first).unwrap(),
            vec!["fldcnabc123"]
        );
        assert_eq!(
            FeishuAdapter.continuity_coordinates(&first).unwrap(),
            FeishuAdapter.continuity_coordinates(&second).unwrap()
        );
        let summary = FeishuAdapter.summary(&first).unwrap();
        assert!(!serde_json::to_string(&summary)
            .unwrap()
            .contains("connection_id"));
        assert_eq!(
            summary.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(),
            vec!["Folder", "Folder token", "Account", "Sign in again by"]
        );
        // No connection store is installed in this process: the date reads
        // as a reconnect.
        assert_eq!(summary[3].value, "reconnect needed");
    }

    #[test]
    fn opening_without_an_installed_connection_store_is_refused() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = FeishuAdapter
            .open(&config("fldcnabc123"), Some(runtime.handle().clone()))
            .err()
            .unwrap();
        assert!(error.contains("keeps no Feishu sign-in"), "{error}");
        assert!(FeishuAdapter.open(&config("fldcnabc123"), None).is_err());
    }
}
