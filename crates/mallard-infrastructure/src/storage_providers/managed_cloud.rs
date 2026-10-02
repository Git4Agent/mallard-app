//! Mallard Cloud: the managed storage service, signed into with GitHub.
//! The origin and the account are the identity; the session token is the
//! managed secret the sign-in lane renews.
use super::traits::*;
use crate::managed_cloud::client::ManagedCloudClient;
use crate::managed_cloud::{origin_allowed, ManagedObjectStore};
use crate::storage_core::ObjectStore;
use mallard_domain::{
    BoundedConfigObject, FieldKey, SecretKey, StorageProviderConfig, StorageProviderKey,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::runtime::Handle;

pub struct ManagedCloudAdapter;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedCloudSettingsV1 {
    pub service_origin: String,
    pub account_id: String,
    pub account_login: String,
}

pub fn provider_key() -> StorageProviderKey {
    StorageProviderKey::parse("managed_cloud").expect("static key")
}
pub fn token_key() -> SecretKey {
    SecretKey::parse("account_token").expect("static key")
}
fn invalid() -> StorageAdapterError {
    StorageAdapterError::invalid(
        "managed_cloud_config_invalid",
        "Sign in to Mallard Cloud through the setup lane; the connection cannot be typed in.",
    )
}
fn decode(config: &StorageProviderConfig) -> Result<ManagedCloudSettingsV1, StorageAdapterError> {
    ManagedCloudAdapter.validate_config(config)?;
    config.settings.decode().map_err(|_| invalid())
}
fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 64
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn valid_account_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_graphic())
}
/// The settings and a client for one saved storage, for callers outside
/// the object-store contract (the website bridge).
pub fn client_for(
    config: &StorageProviderConfig,
) -> Result<(ManagedCloudSettingsV1, ManagedCloudClient), String> {
    client(config).map_err(|error| error.to_string())
}
fn client(
    config: &StorageProviderConfig,
) -> Result<(ManagedCloudSettingsV1, ManagedCloudClient), StorageAdapterError> {
    let settings = decode(config)?;
    let token = config
        .secrets
        .get(&token_key())
        .ok_or_else(invalid)?
        .clone();
    let client = ManagedCloudClient::new(&settings.service_origin, Some(token))
        .map_err(|message| StorageAdapterError::internal("managed_cloud_client_failed", message))?;
    Ok((settings, client))
}

impl StorageProviderAdapter for ManagedCloudAdapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        StorageProviderDescriptor {
            key: provider_key(),
            config_version: 1,
            display_name: "Mallard Cloud".into(),
            description: "Private snapshots with team sharing and web access.".into(),
            default_name: "Mallard Cloud".into(),
            name_suggestion: Some(StorageNameSuggestion::whole_setting(
                "account_login",
                "Mallard Cloud {1}",
            )),
            display_rank: 5,
            icon: "cloud".into(),
            clear_mode: StorageClearMode::Unavailable,
            fields: vec![],
            secrets: vec![StorageSecretDescriptor {
                secret_id: token_key(),
                label: "Sign-in token".into(),
                required: true,
                placeholder: Some("Issued by the browser sign-in".into()),
                help: Some("Issued at sign-in and valid 90 days. Reconnect to renew it.".into()),
                help_url: None,
            }],
            managed_secret_id: Some(token_key()),
            local_setup: Some(StorageLocalSetup::ManagedCloud),
            setup_guide_url: None,
            manual_setup_hint: None,
            setup_doors: None,
            discovery: None,
        }
    }
    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> Result<BoundedConfigObject, StorageAdapterError> {
        let typed: ManagedCloudSettingsV1 = settings.decode().map_err(|_| invalid())?;
        if !origin_allowed(&typed.service_origin)
            || !valid_account_id(&typed.account_id)
            || !valid_login(&typed.account_login)
        {
            return Err(invalid());
        }
        BoundedConfigObject::encode(&typed).map_err(|_| invalid())
    }
    fn validate_config(&self, config: &StorageProviderConfig) -> Result<(), StorageAdapterError> {
        if config.version != 1 || config.secrets.len() != 1 {
            return Err(invalid());
        }
        self.validate_settings(&config.settings)?;
        let token = config.secrets.get(&token_key()).ok_or_else(invalid)?;
        let token = token.expose();
        if !(20..=128).contains(&token.len())
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid());
        }
        Ok(())
    }
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
        Ok(vec![
            field("account_login", "Account", typed.account_login)?,
            field("service_origin", "Service", typed.service_origin)?,
        ])
    }
    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> Result<Vec<String>, String> {
        let typed = decode(config).map_err(|e| e.to_string())?;
        Ok(vec![typed.service_origin, typed.account_id])
    }
    fn local_root(&self, _config: &StorageProviderConfig) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }
    /// `GET /v1/me`: the token works and names the pinned account. The probe
    /// writes nothing.
    fn probe(
        &self,
        config: &StorageProviderConfig,
        runtime: &Handle,
    ) -> Result<(), StorageAdapterError> {
        let (settings, client) = client(config)?;
        let me = runtime.block_on(client.me()).map_err(|error| {
            let message = client
                .health()
                .filter(|health| health.suspends())
                .map(|health| health.describe())
                .unwrap_or_else(|| error.message());
            StorageAdapterError::remote(
                match error.kind {
                    crate::managed_cloud::ErrorKind::Auth => "managed_cloud_reconnect_required",
                    crate::managed_cloud::ErrorKind::Permission => {
                        "managed_cloud_permission_denied"
                    }
                    crate::managed_cloud::ErrorKind::Unreachable => "managed_cloud_unreachable",
                    _ => "managed_cloud_probe_failed",
                },
                message,
            )
        })?;
        if me.user.id != settings.account_id || me.user.github_login != settings.account_login {
            return Err(StorageAdapterError::invalid(
                "managed_cloud_account_mismatch",
                "The sign-in belongs to another Mallard Cloud account than this storage. Reconnect with the storage's account.",
            ));
        }
        Ok(())
    }
    fn open(
        &self,
        config: &StorageProviderConfig,
        runtime: Option<Handle>,
    ) -> Result<Arc<dyn ObjectStore>, String> {
        let (_, client) = client(config).map_err(|e| e.to_string())?;
        Ok(Arc::new(ManagedObjectStore::new(
            client,
            runtime.ok_or("Mallard Cloud requires an asynchronous runtime")?,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::{SecretMap, SecretValue};

    pub(crate) fn config(
        origin: &str,
        account_id: &str,
        login: &str,
        token: &str,
    ) -> StorageProviderConfig {
        let mut secrets = SecretMap::empty();
        secrets
            .insert(token_key(), SecretValue::parse(token).unwrap())
            .unwrap();
        StorageProviderConfig::new(
            1,
            BoundedConfigObject::encode(&ManagedCloudSettingsV1 {
                service_origin: origin.into(),
                account_id: account_id.into(),
                account_login: login.into(),
            })
            .unwrap(),
            secrets,
        )
    }

    const TOKEN: &str = "desktop-token-canary-0123456789";

    #[test]
    fn identity_is_origin_and_account_never_the_token_or_login() {
        let first = config(
            crate::managed_cloud::PRODUCTION_ORIGIN,
            "user_1",
            "octocat",
            TOKEN,
        );
        let second = config(
            crate::managed_cloud::PRODUCTION_ORIGIN,
            "user_1",
            "octocat",
            "another-token-0123456789abcdef",
        );
        ManagedCloudAdapter.validate_config(&first).unwrap();
        assert_eq!(
            ManagedCloudAdapter.continuity_coordinates(&first).unwrap(),
            vec![
                crate::managed_cloud::PRODUCTION_ORIGIN.to_string(),
                "user_1".to_string()
            ]
        );
        assert_eq!(
            ManagedCloudAdapter.continuity_coordinates(&first).unwrap(),
            ManagedCloudAdapter.continuity_coordinates(&second).unwrap()
        );
        let summary = serde_json::to_string(&ManagedCloudAdapter.summary(&first).unwrap()).unwrap();
        assert!(summary.contains("octocat") && !summary.contains(TOKEN));
        assert_eq!(
            ManagedCloudAdapter
                .summary(&first)
                .unwrap()
                .iter()
                .map(|f| f.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Account", "Service"]
        );
        let descriptor = ManagedCloudAdapter.descriptor();
        assert_eq!(descriptor.managed_secret_id, Some(token_key()));
        assert_eq!(
            descriptor.local_setup,
            Some(StorageLocalSetup::ManagedCloud)
        );
        assert!(descriptor.fields.is_empty() && descriptor.discovery.is_none());
        assert_eq!(descriptor.clear_mode, StorageClearMode::Unavailable);
    }

    #[test]
    fn configuration_rejects_bad_origins_logins_tokens_and_extras() {
        for origin in [
            "https://mallard-ai.com/",
            "mallard-ai.com",
            "https://evil.test/x",
        ] {
            assert!(
                ManagedCloudAdapter
                    .validate_config(&config(origin, "user_1", "octocat", TOKEN))
                    .is_err(),
                "{origin}"
            );
        }
        assert!(ManagedCloudAdapter
            .validate_config(&config(
                crate::managed_cloud::PRODUCTION_ORIGIN,
                "",
                "octocat",
                TOKEN
            ))
            .is_err());
        assert!(ManagedCloudAdapter
            .validate_config(&config(
                crate::managed_cloud::PRODUCTION_ORIGIN,
                "user_1",
                "octo cat",
                TOKEN
            ))
            .is_err());
        assert!(ManagedCloudAdapter
            .validate_config(&config(
                crate::managed_cloud::PRODUCTION_ORIGIN,
                "user_1",
                "octocat",
                "short"
            ))
            .is_err());
        let mut extra = config(
            crate::managed_cloud::PRODUCTION_ORIGIN,
            "user_1",
            "octocat",
            TOKEN,
        );
        extra.settings = BoundedConfigObject::from_json(&serde_json::json!({
            "service_origin": crate::managed_cloud::PRODUCTION_ORIGIN, "account_id": "user_1", "account_login": "octocat", "extra": 1
        }))
        .unwrap();
        assert!(ManagedCloudAdapter.validate_config(&extra).is_err());
        let mut two = config(
            crate::managed_cloud::PRODUCTION_ORIGIN,
            "user_1",
            "octocat",
            TOKEN,
        );
        two.secrets
            .insert(
                SecretKey::parse("other").unwrap(),
                SecretValue::parse("x").unwrap(),
            )
            .unwrap();
        assert!(ManagedCloudAdapter.validate_config(&two).is_err());
        let mut next = config(
            crate::managed_cloud::PRODUCTION_ORIGIN,
            "user_1",
            "octocat",
            TOKEN,
        );
        next.version = 2;
        assert!(ManagedCloudAdapter.validate_config(&next).is_err());
        // A release build speaks to production only; a development build
        // may point at a local Worker.
        assert_eq!(
            ManagedCloudAdapter
                .validate_config(&config("http://localhost:8787", "user_1", "octocat", TOKEN))
                .is_ok(),
            cfg!(debug_assertions)
        );
    }
}
