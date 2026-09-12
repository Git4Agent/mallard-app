//! A minimal storage provider used only to prove registry-driven storage
//! extension: a declared setup assistant, a typed setting, an optional
//! secret, and an object store backed by a local folder.

use super::traits::{
    StorageAdapterError, StorageFieldDescriptor, StorageFieldKind, StorageProviderAdapter,
    StorageProviderDescriptor, StorageSecretDescriptor,
     StorageSummaryField,
};
use crate::storage_core::{LocalObjectStore, ObjectStore};
use mallard_domain::{
    sha256_hex, validate_absolute_clean_path, BoundedConfigObject, FieldKey, SecretKey, SecretMap,
    StorageConfig, StorageId, StorageProviderConfig, StorageProviderKey,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Handle;

pub const TEST_STORAGE_PROVIDER: &str = "teststore";
pub const TEST_STORAGE_CONFIG_VERSION: u32 = 1;

pub fn provider_key() -> StorageProviderKey {
    StorageProviderKey::parse(TEST_STORAGE_PROVIDER).expect("static provider key")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestStorageSettingsV1 {
    pub directory: String,
    pub label: String,
}

pub fn test_storage_config(id: StorageId, name: &str, directory: &Path) -> StorageConfig {
    StorageConfig {
        id,
        name: name.to_string(),
        provider: provider_key(),
        provider_config: StorageProviderConfig::new(
            TEST_STORAGE_CONFIG_VERSION,
            BoundedConfigObject::encode(&TestStorageSettingsV1 {
                directory: directory.to_string_lossy().into_owned(),
                label: "test".to_string(),
            })
            .expect("test settings encode"),
            SecretMap::empty(),
        ),
    }
}

fn invalid() -> StorageAdapterError {
    StorageAdapterError::invalid(
        "teststore_settings_invalid",
        "The test storage settings are invalid.",
    )
}

fn decode(config: &StorageProviderConfig) -> Result<TestStorageSettingsV1, StorageAdapterError> {
    if config.version != TEST_STORAGE_CONFIG_VERSION {
        return Err(invalid());
    }
    config.settings.decode().map_err(|_| invalid())
}

pub struct TestStorageAdapter;

impl StorageProviderAdapter for TestStorageAdapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        StorageProviderDescriptor {
            key: provider_key(),
            config_version: TEST_STORAGE_CONFIG_VERSION,
            display_name: "Test storage".to_string(),
            description: "A test-only storage provider.".to_string(),
            default_name: "Test storage".to_string(),
            name_suggestion: None,
            display_rank: 90,
            icon: "drive".to_string(),
            clear_mode: super::traits::StorageClearMode::RevisionChecked,
            fields: vec![
                StorageFieldDescriptor {
                    immutable_on_edit: false,
                    field_id: FieldKey::parse("directory").unwrap(),
                    label: "Folder".to_string(),
                    kind: StorageFieldKind::Directory,
                    required: true,
                    placeholder: None,
                    help: None,
                    help_url: None,
                    derive: None,
                },
                StorageFieldDescriptor {
                    immutable_on_edit: false,
                    field_id: FieldKey::parse("label").unwrap(),
                    label: "Label".to_string(),
                    kind: StorageFieldKind::Text,
                    required: true,
                    placeholder: None,
                    help: None,
                    help_url: None,
                    derive: None,
                },
            ],
            secrets: vec![StorageSecretDescriptor {
                secret_id: SecretKey::parse("token").unwrap(),
                label: "Token".to_string(),
                required: false,
                help: None,
                placeholder: None,
                help_url: None,
            }],
            local_setup: None,
            managed_secret_id: None,
            setup_guide_url: None,
            manual_setup_hint: None,
            discovery: None,
        }
    }

    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> Result<BoundedConfigObject, StorageAdapterError> {
        let typed: TestStorageSettingsV1 = settings.decode().map_err(|_| invalid())?;
        validate_absolute_clean_path("test storage directory", &typed.directory)
            .map_err(|_| invalid())?;
        if typed.label.is_empty() || typed.label.len() > 64 {
            return Err(invalid());
        }
        BoundedConfigObject::encode(&typed).map_err(|_| invalid())
    }

    fn validate_config(&self, config: &StorageProviderConfig) -> Result<(), StorageAdapterError> {
        self.validate_settings(&config.settings)?;
        Ok(())
    }

    fn summary(&self, config: &StorageProviderConfig) -> Result<Vec<StorageSummaryField>, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        Ok(vec![StorageSummaryField {
            field_id: FieldKey::parse("label").unwrap(),
            label: "Label".to_string(),
            value: typed.label,
            is_path: false,
        }])
    }

    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> Result<Vec<String>, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        Ok(vec![typed.directory])
    }

    fn local_root(&self, config: &StorageProviderConfig) -> Result<Option<PathBuf>, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        Ok(Some(PathBuf::from(typed.directory)))
    }

    fn journal_facts(&self, config: &StorageProviderConfig) -> Result<BoundedConfigObject, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        BoundedConfigObject::from_json(&serde_json::json!({
            "directory_sha256": sha256_hex(typed.directory.as_bytes()),
        }))
    }

    fn probe(
        &self,
        config: &StorageProviderConfig,
        _setup_id: &str,
        _runtime: &Handle,
        _journal: &mut dyn super::traits::StorageProbeJournal,
    ) -> Result<(), StorageAdapterError> {
        let typed = decode(config)?;
        if Path::new(&typed.directory).is_dir() {
            Ok(())
        } else {
            Err(StorageAdapterError::remote(
                "teststore_unreachable",
                "The test storage folder does not exist.",
            ))
        }
    }

    fn reconcile_probe(
        &self,
        _config: &StorageProviderConfig,
        _setup_id: &str,
        _runtime: &Handle,
        _journal: &mut dyn super::traits::StorageProbeJournal,
    ) -> Result<(), StorageAdapterError> {
        Ok(())
    }



    fn open(
        &self,
        config: &StorageProviderConfig,
        _runtime: Option<Handle>,
    ) -> Result<Arc<dyn ObjectStore>, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        Ok(Arc::new(LocalObjectStore::open(&typed.directory)?))
    }
}
