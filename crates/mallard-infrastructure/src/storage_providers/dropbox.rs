//! User-linked Dropbox connection. The namespace is the storage identity.
use super::traits::*;
use crate::dropbox_auth::{validate_token, DropboxCredential};
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
            key:provider_key(),config_version:1,display_name:"Dropbox".into(),description:"An existing shared or team namespace. Uploads up to 150 MiB per object.".into(),
            default_name:"Dropbox storage".into(),name_suggestion:None,display_rank:30,icon:"cloud".into(),clear_mode:StorageClearMode::RevisionChecked,
            fields:vec![StorageFieldDescriptor {field_id:FieldKey::parse("namespace_id").expect("static key"),label:"Namespace ID".into(),immutable_on_edit:true,kind:StorageFieldKind::Text,required:true,
                placeholder:Some("Shared or team namespace ID".into()),help:Some("Use the same namespace ID on each machine. This value cannot change after setup.".into()),help_url:Some(guide.into()),derive:None}],
            secrets:vec![StorageSecretDescriptor {secret_id:token_key(),label:"Access token".into(),required:true,placeholder:Some("User-linked Dropbox access token".into()),
                help:Some("For manual setup, supply account_info.read, files.metadata.read, files.content.read, and files.content.write. Replace this token when it expires. Agent setup renews access automatically.".into()),help_url:Some(guide.into())},
                StorageSecretDescriptor {secret_id:refresh_token_key(),label:"Refresh token".into(),required:true,placeholder:Some("Dropbox refresh token".into()),
                help:Some("Agent setup saves this token to renew access automatically. Show it to review the value, or enter a replacement issued for this Mallard app and Dropbox account.".into()),help_url:Some(guide.into())}],
            setup_assistant:Some(StorageSetupAssistantDescriptor {
                label: "Set up with an agent".into(),
                description: "Approve Dropbox access in your browser. The agent creates a Mallard shared folder and fills this connection. Dropbox permission covers your account, beyond this folder.".into(),
                managed_secret_id: Some(refresh_token_key()),
            }),setup_guide_url:Some(guide.into()),manual_setup_hint:Some("Enter an existing shared or team namespace and a user-linked token. Mallard stores data in .mallard within that namespace.".into()),
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
    fn summary(&self, config: &StorageProviderConfig) -> Result<Vec<StorageSummaryField>, String> {
        let typed = decode(config).map_err(|e| e.to_string())?;
        Ok(vec![StorageSummaryField {
            field_id: FieldKey::parse("namespace_id")?,
            label: "Namespace".into(),
            value: typed.namespace_id,
            is_path: false,
        }])
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

    fn setup_assistant_instruction(
        &self,
        context: &StorageSetupAssistantContext<'_>,
    ) -> Result<String, String> {
        let cli_path = serde_json::to_string(context.cli_path).map_err(|_| "invalid CLI path")?;
        let name =
            serde_json::to_string(context.storage_name).map_err(|_| "invalid storage name")?;
        let draft_id = serde_json::to_string(context.draft_id).map_err(|_| "invalid draft ID")?;
        let revision = context.draft_revision;
        let command = format!(
            "Use this literal Mallard CLI executable path: {cli_path}\n\nRun its storage dropbox setup command with --name {name}, --draft-id {draft_id}, and --expected-revision {revision}. Treat the executable path and option values as literal arguments, quote them for the current shell, and never interpret them as shell syntax. No external CLI installation or typed app key is required."
        );
        let browser = "I complete Dropbox login, MFA, and consent in the browser. Do not read or control the browser or Mallard window. Do not request, read, print, or copy credentials, local state files, or browser data.";
        if context.reconnect {
            return Ok(format!(
                "Reconnect my existing Dropbox storage so Mallard regains access. Dropbox rejected the saved authorization; the storage, its namespace, and its project links stay exactly as they are. {command}\n\nThis command is authorized to save the exact draft in Mallard's current local state and open the system browser. It reopens Dropbox consent for the same account, obtains a new refresh token for the existing namespace, and saves that token in this exact draft. It creates no folder and changes no namespace. If execution restrictions require approval, request it for this exact command only. Do not change HOME or use another Mallard state directory.\n\n{browser}\n\nWait for the CLI result. It prepares this exact schema-1 draft without committing storage. Report only success or the fixed error, then stop. I review the connection and select Save changes. Do not run setup again automatically after an unknown result."
            ));
        }
        Ok(format!(
            "Prepare the Dropbox storage draft for my review. {command}\n\nThis command is authorized to save the exact draft in Mallard's current local state, open the system browser, and create a new Mallard shared folder when this draft has no namespace. If execution restrictions require approval, request it for this exact command only. Do not change HOME or use another Mallard state directory.\n\n{browser}\n\nWait for the CLI result. It prepares this exact schema-1 draft without committing storage or sending invitations. Report only success or the fixed error, then stop. I review the connection and select Finish setup. Do not run setup again automatically after an unknown result."
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
            })
            .unwrap(),
            secrets,
        )
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
            descriptor.setup_assistant.unwrap().managed_secret_id,
            Some(refresh_token_key())
        );
        let text = serde_json::to_string(&DropboxAdapter.journal_facts(&managed).unwrap()).unwrap();
        assert!(!text.contains("refresh-canary"));
        let context = |reconnect| StorageSetupAssistantContext {
            storage_name: "Dropbox storage 1",
            cli_path: "/fixture/mallard-cli",
            draft_id: "storagedraft-111111111111111111111111",
            draft_revision: 7,
            reconnect,
        };
        let instruction = DropboxAdapter
            .setup_assistant_instruction(&context(false))
            .unwrap();
        assert!(
            instruction.contains("storage dropbox setup")
                && instruction.contains("--expected-revision 7")
                && instruction.contains("create a new Mallard shared folder")
                && instruction.contains("Finish setup")
        );
        assert!(!instruction.contains("refresh-canary") && !instruction.contains("wrangler"));
        // Reconnection tells the agent it is regaining access to the same
        // namespace with the same command, and hands the user Save changes.
        let reconnect = DropboxAdapter
            .setup_assistant_instruction(&context(true))
            .unwrap();
        assert!(
            reconnect.contains("regains access")
                && reconnect.contains("existing namespace")
                && reconnect.contains("creates no folder")
                && reconnect.contains("Save changes")
                && reconnect.contains("--expected-revision 7")
        );
        assert!(
            !reconnect.contains("Finish setup") && !reconnect.contains("new Mallard shared folder")
        );
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
        assert!(DropboxAdapter.descriptor().fields[0].immutable_on_edit);
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
