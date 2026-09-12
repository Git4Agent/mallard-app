//! User-linked Dropbox connection. The namespace is the storage identity;
//! the folder's name, when setup learned it, is what listings show.
use super::traits::*;
use crate::dropbox_auth::{validate_token, DropboxCredential};
use crate::dropbox_setup::valid_folder_name;
use crate::storage_core::{dropbox_store::DropboxObjectStore, ObjectStore};
use mallard_domain::{
    BoundedConfigObject, FieldKey, SecretKey, StorageProviderConfig, StorageProviderKey,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::runtime::Handle;

pub struct DropboxAdapter;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DropboxSettingsV1 {
    pub namespace_id: String,
    /// The shared folder's name as Dropbox shows it, recorded by the setup
    /// lane for listings. Never an identity: the namespace is. Absent when
    /// the connection was pasted in by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
}
pub fn provider_key() -> StorageProviderKey {
    StorageProviderKey::parse("dropbox").expect("static key")
}
pub fn token_key() -> SecretKey {
    SecretKey::parse("access_token").expect("static key")
}
pub fn refresh_token_key() -> SecretKey {
    SecretKey::parse("refresh_token").expect("static key")
}
fn invalid() -> StorageAdapterError {
    StorageAdapterError::invalid(
        "dropbox_config_invalid",
        "Enter a shared or team namespace ID and a user-linked access token.",
    )
}
fn decode(config: &StorageProviderConfig) -> Result<DropboxSettingsV1, StorageAdapterError> {
    DropboxAdapter.validate_config(config)?;
    config.settings.decode().map_err(|_| invalid())
}
fn store(
    config: &StorageProviderConfig,
    runtime: Handle,
) -> Result<DropboxObjectStore, StorageAdapterError> {
    let settings = decode(config)?;
    DropboxObjectStore::new(settings.namespace_id, credential(config)?, runtime)
        .map_err(|_| invalid())
}
fn credential(config: &StorageProviderConfig) -> Result<DropboxCredential, StorageAdapterError> {
    if config.secrets.len() != 1 {
        return Err(invalid());
    }
    let (key, managed) = if config.secrets.contains(&refresh_token_key()) {
        (refresh_token_key(), true)
    } else {
        (token_key(), false)
    };
    let token = config.secrets.get(&key).ok_or_else(invalid)?;
    validate_token(token.expose()).map_err(|_| invalid())?;
    Ok(if managed {
        DropboxCredential::RefreshToken(token.clone())
    } else {
        DropboxCredential::AccessToken(token.clone())
    })
}
impl StorageProviderAdapter for DropboxAdapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        let guide = "https://www.dropbox.com/developers/reference/auth-types";
        StorageProviderDescriptor {
            key:provider_key(),config_version:1,display_name:"Dropbox".into(),description:"A shared folder in your Dropbox, new or one you already use. Uploads up to 150 MiB per object.".into(),
            default_name:"Dropbox storage".into(),name_suggestion:None,display_rank:30,icon:"cloud".into(),clear_mode:StorageClearMode::RevisionChecked,
            fields:vec![StorageFieldDescriptor {field_id:FieldKey::parse("namespace_id").expect("static key"),label:"Namespace ID".into(),kind:StorageFieldKind::Text,required:true,
                placeholder:Some("Shared or team namespace ID".into()),help:Some("Use the same namespace ID on each machine. This value cannot change after setup.".into()),help_url:Some(guide.into()),derive:None}],
            secrets:vec![StorageSecretDescriptor {secret_id:token_key(),label:"Access token".into(),required:true,placeholder:Some("User-linked Dropbox access token".into()),
                help:Some("For manual setup, supply account_info.read, files.metadata.read, files.content.read, and files.content.write. Replace this token when it expires. Browser setup renews access automatically.".into()),help_url:Some(guide.into())},
                StorageSecretDescriptor {secret_id:refresh_token_key(),label:"Refresh token".into(),required:true,placeholder:Some("Dropbox refresh token".into()),
                help:Some("Browser setup saves this token to renew access automatically. Show it to review the value, or enter a replacement issued for this Mallard app and Dropbox account.".into()),help_url:Some(guide.into())}],
            local_setup: Some(StorageLocalSetup::Dropbox),
            managed_secret_id: Some(refresh_token_key()),setup_guide_url:Some(guide.into()),manual_setup_hint:Some("Enter an existing shared or team namespace and a user-linked token. Mallard stores data in .mallard within that namespace.".into()),
            discovery: Some(StorageDiscoveryDescriptor {
                unit: "shared folder".into(),
                unit_plural: "shared folders".into(),
                place: "your Dropbox".into(),
                create_hint: "A new Mallard shared folder in your Dropbox, created after browser consent.".into(),
                existing_hint: "A shared folder Mallard already uses, found in your Dropbox — or a namespace you paste in.".into(),
                existing_help: None,
                find_label: "Find in my Dropbox".into(),
                access_hint: "Discovery lists accessible folders and reads their Mallard markers without changing them.".into(),
                scanning_hint: "Approve access in your browser, then Mallard looks through your shared folders…".into(),
                prepare_hint: "This is the connection Mallard keeps, renewed automatically.".into(),
            }),
        }
    }
    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> Result<BoundedConfigObject, StorageAdapterError> {
        let typed: DropboxSettingsV1 = settings.decode().map_err(|_| invalid())?;
        if typed.namespace_id.is_empty()
            || typed.namespace_id.len() > 256
            || !typed
                .namespace_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
        {
            return Err(invalid());
        }
        if typed
            .folder_name
            .as_deref()
            .is_some_and(|name| !valid_folder_name(name))
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
        credential(config)?;
        Ok(())
    }
    /// The folder's name leads when setup recorded it — it is what a
    /// listing's subtitle shows — and the namespace ID follows as the fact
    /// that identifies the storage.
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
        if let Some(name) = typed.folder_name {
            facts.push(field("folder_name", "Folder", name)?);
        }
        facts.push(field("namespace_id", "Namespace ID", typed.namespace_id)?);
        Ok(facts)
    }
    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> Result<Vec<String>, String> {
        Ok(vec![
            decode(config).map_err(|e| e.to_string())?.namespace_id,
        ])
    }
    fn local_root(&self, _config: &StorageProviderConfig) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }
    fn journal_facts(&self, config: &StorageProviderConfig) -> Result<BoundedConfigObject, String> {
        BoundedConfigObject::encode(&decode(config).map_err(|e| e.to_string())?)
    }
    fn probe(
        &self,
        config: &StorageProviderConfig,
        setup_id: &str,
        runtime: &Handle,
        journal: &mut dyn StorageProbeJournal,
    ) -> Result<(), StorageAdapterError> {
        let store = store(config, runtime.clone())?;
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
    ) -> Result<(), StorageAdapterError> {
        let store = store(config, runtime.clone())?;
        store
            .reconcile_probe(setup_id, journal)
            .map_err(|error| store.adapter_error(error))
    }
    fn open(
        &self,
        config: &StorageProviderConfig,
        runtime: Option<Handle>,
    ) -> Result<Arc<dyn ObjectStore>, String> {
        Ok(Arc::new(
            store(
                config,
                runtime.ok_or("Dropbox requires an asynchronous runtime")?,
            )
            .map_err(|e| e.to_string())?,
        ))
    }


}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::{SecretMap, SecretValue};
    fn config(namespace: &str, token: &str) -> StorageProviderConfig {
        let mut secrets = SecretMap::empty();
        secrets
            .insert(token_key(), SecretValue::parse(token).unwrap())
            .unwrap();
        StorageProviderConfig::new(
            1,
            BoundedConfigObject::encode(&DropboxSettingsV1 {
                namespace_id: namespace.into(),
                folder_name: None,
            })
            .unwrap(),
            secrets,
        )
    }
    fn named(namespace: &str, folder: &str) -> StorageProviderConfig {
        let mut config = config(namespace, "token");
        config.settings = BoundedConfigObject::encode(&DropboxSettingsV1 {
            namespace_id: namespace.into(),
            folder_name: Some(folder.into()),
        })
        .unwrap();
        config
    }
    #[test]
    fn listings_lead_with_the_folder_name_and_identity_stays_the_namespace() {
        let by_hand = DropboxAdapter.summary(&config("shared", "token")).unwrap();
        assert_eq!(by_hand.len(), 1);
        assert_eq!(by_hand[0].label, "Namespace ID");
        assert_eq!(by_hand[0].value, "shared");
        let from_setup = named("shared", "Mallard otter");
        DropboxAdapter.validate_config(&from_setup).unwrap();
        let summary = DropboxAdapter.summary(&from_setup).unwrap();
        assert_eq!(
            summary.iter().map(|f| (f.label.as_str(), f.value.as_str())).collect::<Vec<_>>(),
            vec![("Folder", "Mallard otter"), ("Namespace ID", "shared")]
        );
        assert_eq!(
            DropboxAdapter.continuity_coordinates(&from_setup).unwrap(),
            DropboxAdapter.continuity_coordinates(&config("shared", "token")).unwrap()
        );
        for bad in ["", " padded", "a/b", "..", &"x".repeat(256)] {
            assert!(DropboxAdapter.validate_config(&named("shared", bad)).is_err(), "{bad:?}");
        }
        // The name is for listings only: the form never shows it, so it is
        // not a declared field.
        assert!(!DropboxAdapter
            .descriptor()
            .fields
            .iter()
            .any(|field| field.field_id.as_str() == "folder_name"));
    }
    #[test]
    fn refresh_credential_keeps_schema_and_namespace_and_declares_its_review_field() {
        let mut managed = config("shared", "manual-canary");
        managed.secrets.remove(&token_key());
        managed
            .secrets
            .insert(
                refresh_token_key(),
                SecretValue::parse("refresh-canary").unwrap(),
            )
            .unwrap();
        DropboxAdapter.validate_config(&managed).unwrap();
        assert_eq!(managed.version, 1);
        assert_eq!(
            DropboxAdapter.continuity_coordinates(&managed).unwrap(),
            vec!["shared"]
        );
        let descriptor = DropboxAdapter.descriptor();
        assert!(descriptor.secrets.iter().any(
            |secret| secret.secret_id == refresh_token_key() && secret.label == "Refresh token"
        ));
        assert_eq!(
            descriptor.managed_secret_id,
            Some(refresh_token_key())
        );
        let text = serde_json::to_string(&DropboxAdapter.journal_facts(&managed).unwrap()).unwrap();
        assert!(!text.contains("refresh-canary"));

    }
    #[test]
    fn namespace_identity_does_not_depend_on_token() {
        let first = config("Ns:Team-42", "first-token");
        let second = config("Ns:Team-42", "second-token");
        assert_eq!(
            DropboxAdapter.continuity_coordinates(&first).unwrap(),
            vec!["Ns:Team-42"]
        );
        assert_eq!(
            DropboxAdapter.continuity_coordinates(&first).unwrap(),
            DropboxAdapter.continuity_coordinates(&second).unwrap()
        );
        assert_eq!(
            DropboxAdapter.journal_facts(&first).unwrap(),
            DropboxAdapter.journal_facts(&second).unwrap()
        );
        let summary = serde_json::to_string(&DropboxAdapter.summary(&first).unwrap()).unwrap();
        assert!(!summary.contains("first-token"));
        assert_eq!(
            DropboxAdapter.descriptor().fields.len() + DropboxAdapter.descriptor().secrets.len(),
            3
        );
    }
    #[test]
    fn configuration_rejects_extra_fields_secrets_and_invalid_values() {
        for namespace in ["", " ns", "ns/child", "ns\\child", "é"] {
            assert!(DropboxAdapter
                .validate_config(&config(namespace, "token"))
                .is_err());
        }
        let mut extra = config("shared", "token");
        extra.settings = BoundedConfigObject::from_json(
            &serde_json::json!({"namespace_id":"shared","endpoint":"https://evil.test"}),
        )
        .unwrap();
        assert!(DropboxAdapter.validate_config(&extra).is_err());
        let mut extra = config("shared", "token");
        extra
            .secrets
            .insert(
                SecretKey::parse("refresh_token").unwrap(),
                SecretValue::parse("private").unwrap(),
            )
            .unwrap();
        assert!(DropboxAdapter.validate_config(&extra).is_err());
        assert!(DropboxAdapter
            .validate_config(&config("shared", "has space"))
            .is_err());
        let mut next = config("shared", "token");
        next.version = 2;
        assert!(DropboxAdapter.validate_config(&next).is_err());
    }
}
