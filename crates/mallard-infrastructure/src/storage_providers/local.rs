//! The local-folder storage adapter (`local`, config version 1).

use super::traits::{
    StorageAdapterError, StorageFieldDescriptor, StorageFieldKind, StorageNameSuggestion,
    StorageProviderAdapter, StorageProviderDescriptor, StorageSummaryField,
};
use crate::storage_core::{LocalObjectStore, ObjectStore};
use mallard_domain::{
    sha256_hex, validate_absolute_clean_path, BoundedConfigObject, FieldKey, SecretMap,
    StorageConfig, StorageId, StorageProviderConfig, StorageProviderKey,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Handle;

pub const LOCAL_STORAGE_PROVIDER: &str = "local";
pub const LOCAL_CONFIG_VERSION: u32 = 1;

pub fn provider_key() -> StorageProviderKey {
    StorageProviderKey::parse(LOCAL_STORAGE_PROVIDER).expect("static provider key")
}

/// The version-1 settings shape. Unknown fields are rejected.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalSettingsV1 {
    pub directory: String,
}

/// Build the version-1 provider configuration for one folder.
pub fn local_provider_config(directory: &str) -> Result<StorageProviderConfig, String> {
    Ok(StorageProviderConfig::new(
        LOCAL_CONFIG_VERSION,
        BoundedConfigObject::encode(&LocalSettingsV1 {
            directory: directory.to_string(),
        })?,
        SecretMap::empty(),
    ))
}

/// A complete local-folder storage record.
pub fn local_storage_config(id: StorageId, name: &str, directory: &Path) -> StorageConfig {
    StorageConfig {
        id,
        name: name.to_string(),
        provider: provider_key(),
        provider_config: local_provider_config(&directory.to_string_lossy())
            .expect("a local directory encodes as bounded settings"),
    }
}

fn invalid_directory() -> StorageAdapterError {
    StorageAdapterError::invalid(
        "local_directory_invalid",
        "The local storage folder must be an absolute, clean path.",
    )
}

fn decode(config: &StorageProviderConfig) -> Result<LocalSettingsV1, StorageAdapterError> {
    if config.version != LOCAL_CONFIG_VERSION {
        return Err(StorageAdapterError::invalid(
            "local_config_version_unsupported",
            "The local storage configuration version is not supported.",
        ));
    }
    config.settings.decode().map_err(|_| invalid_directory())
}

fn prospective_canonical_directory(path: &Path) -> Result<PathBuf, String> {
    validate_absolute_clean_path("local storage directory", &path.to_string_lossy())?;
    let mut missing = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&cursor) {
            Ok(_) => {
                let mut resolved = std::fs::canonicalize(&cursor)
                    .map_err(|error| format!("resolve '{}': {}", cursor.display(), error))?;
                if !std::fs::metadata(&resolved)
                    .map_err(|error| format!("inspect '{}': {}", resolved.display(), error))?
                    .is_dir()
                {
                    return Err(format!("'{}' is not a directory", cursor.display()));
                }
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    format!(
                        "local storage path '{}' has no existing ancestor",
                        path.display()
                    )
                })?;
                missing.push(name.to_os_string());
                cursor = cursor
                    .parent()
                    .ok_or_else(|| {
                        format!(
                            "local storage path '{}' has no existing ancestor",
                            path.display()
                        )
                    })?
                    .to_path_buf();
            }
            Err(error) => return Err(format!("inspect '{}': {}", cursor.display(), error)),
        }
    }
}

pub struct LocalFolderAdapter;

impl StorageProviderAdapter for LocalFolderAdapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        StorageProviderDescriptor {
            key: provider_key(),
            config_version: LOCAL_CONFIG_VERSION,
            display_name: "Local folder".to_string(),
            description: "A folder on this Mac or on a mounted drive.".to_string(),
            default_name: "Local storage".to_string(),
            name_suggestion: Some(StorageNameSuggestion {
                source: FieldKey::parse("directory").expect("static field key"),
                pattern: r"([^/\\]+)[/\\]*$".to_string(),
                group: 1,
            }),
            display_rank: 20,
            icon: "drive".to_string(),
            clear_mode: super::traits::StorageClearMode::RevisionChecked,
            fields: vec![StorageFieldDescriptor {
                field_id: FieldKey::parse("directory").expect("static field key"),
                label: "Location path".to_string(),
                kind: StorageFieldKind::Directory,
                required: true,
                placeholder: Some("e.g. /Volumes/backup/mallard-storage".to_string()),
                help: None,
                help_url: None,
                derive: None,
            }],
            secrets: Vec::new(),
            managed_secret_id: None,
            local_setup: None,
            setup_guide_url: None,
            manual_setup_hint: None,
            discovery: None,
        }
    }

    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> Result<BoundedConfigObject, StorageAdapterError> {
        let typed: LocalSettingsV1 = settings.decode().map_err(|_| invalid_directory())?;
        validate_absolute_clean_path("local storage directory", &typed.directory)
            .map_err(|_| invalid_directory())?;
        BoundedConfigObject::encode(&typed).map_err(|_| invalid_directory())
    }

    fn validate_config(&self, config: &StorageProviderConfig) -> Result<(), StorageAdapterError> {
        self.validate_settings(&config.settings)?;
        if !config.secrets.is_empty() {
            return Err(StorageAdapterError::invalid(
                "local_secrets_not_allowed",
                "A local folder storage has no secrets.",
            ));
        }
        Ok(())
    }

    fn summary(&self, config: &StorageProviderConfig) -> Result<Vec<StorageSummaryField>, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        Ok(vec![StorageSummaryField {
            field_id: FieldKey::parse("directory").expect("static field key"),
            label: "Folder".to_string(),
            value: typed.directory,
            is_path: true,
        }])
    }

    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> Result<Vec<String>, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        let canonical = prospective_canonical_directory(Path::new(&typed.directory))?;
        let canonical = canonical
            .to_str()
            .ok_or("local storage directory resolves to a non-UTF-8 path")?;
        Ok(vec![canonical.to_string()])
    }

    fn local_root(&self, config: &StorageProviderConfig) -> Result<Option<PathBuf>, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        Ok(Some(PathBuf::from(typed.directory)))
    }

    fn journal_facts(&self, config: &StorageProviderConfig) -> Result<BoundedConfigObject, String> {
        let typed = decode(config).map_err(|error| error.to_string())?;
        // A digest identifies the folder for reconciliation without writing
        // a machine-local path into the journal.
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
        let metadata = std::fs::metadata(&typed.directory).map_err(|_| {
            StorageAdapterError::invalid(
                "local_directory_missing",
                "The local storage folder does not exist.",
            )
        })?;
        if !metadata.is_dir() {
            return Err(StorageAdapterError::invalid(
                "local_directory_not_directory",
                "The local storage folder is not a directory.",
            ));
        }
        if metadata.permissions().readonly() {
            return Err(StorageAdapterError::invalid(
                "local_directory_read_only",
                "The local storage folder is read-only.",
            ));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_typed_absolute_paths_without_secrets() {
        let adapter = LocalFolderAdapter;
        let settings = BoundedConfigObject::from_json(&serde_json::json!({
            "directory": "/tmp/mallard-store"
        }))
        .unwrap();
        assert_eq!(adapter.validate_settings(&settings).unwrap(), settings);
        for invalid in [
            serde_json::json!({"directory": "relative"}),
            serde_json::json!({"directory": "/tmp/../x"}),
            serde_json::json!({"directory": "/tmp", "extra": 1}),
            serde_json::json!({}),
        ] {
            let settings = BoundedConfigObject::from_json(&invalid).unwrap();
            assert_eq!(
                adapter.validate_settings(&settings).unwrap_err().code(),
                "local_directory_invalid"
            );
        }
        let mut config = local_provider_config("/tmp/mallard-store").unwrap();
        adapter.validate_config(&config).unwrap();
        config
            .secrets
            .insert(
                mallard_domain::SecretKey::parse("token").unwrap(),
                mallard_domain::SecretValue::parse("x").unwrap(),
            )
            .unwrap();
        assert_eq!(
            adapter.validate_config(&config).unwrap_err().code(),
            "local_secrets_not_allowed"
        );
    }

    #[test]
    fn facts_and_coordinates_derive_from_the_directory() {
        let adapter = LocalFolderAdapter;
        let config = local_provider_config("/tmp/mallard-store").unwrap();
        assert_eq!(
            adapter.continuity_coordinates(&config).unwrap(),
            vec![std::fs::canonicalize("/tmp")
                .unwrap()
                .join("mallard-store")
                .to_string_lossy()
                .to_string()]
        );
        assert_eq!(
            adapter.local_root(&config).unwrap(),
            Some(PathBuf::from("/tmp/mallard-store"))
        );
        let facts = adapter.journal_facts(&config).unwrap();
        assert!(facts.get_str("directory_sha256").is_some());
        assert!(!String::from_utf8(facts.canonical_json())
            .unwrap()
            .contains("/tmp"));
        let summary = adapter.summary(&config).unwrap();
        assert_eq!(summary[0].value, "/tmp/mallard-store");
        assert!(summary[0].is_path);
    }

    #[cfg(unix)]
    #[test]
    fn continuity_coordinates_collapse_parent_symlink_aliases() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        let store = real.join("store");
        std::fs::create_dir_all(&store).unwrap();
        let alias = temp.path().join("alias");
        symlink(&real, &alias).unwrap();

        let adapter = LocalFolderAdapter;
        let direct = local_provider_config(&store.to_string_lossy()).unwrap();
        let aliased = local_provider_config(&alias.join("store").to_string_lossy()).unwrap();
        assert_eq!(
            adapter.continuity_coordinates(&direct).unwrap(),
            vec![std::fs::canonicalize(&store)
                .unwrap()
                .to_string_lossy()
                .to_string()]
        );
        assert_eq!(
            adapter.continuity_coordinates(&direct).unwrap(),
            adapter.continuity_coordinates(&aliased).unwrap()
        );
    }

    #[test]
    fn probe_checks_the_folder_without_writing() {
        let adapter = LocalFolderAdapter;
        let temp = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let config = local_provider_config(&temp.path().to_string_lossy()).unwrap();
        adapter
            .probe(
                &config,
                "setup-0123456789abcdef01234567",
                runtime.handle(),
                &mut crate::storage_setup_journal::MemoryProbeJournal::default(),
            )
            .unwrap();
        assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
        let missing = local_provider_config(&temp.path().join("absent").to_string_lossy()).unwrap();
        assert_eq!(
            adapter
                .probe(
                    &missing,
                    "setup-0123456789abcdef01234567",
                    runtime.handle(),
                    &mut crate::storage_setup_journal::MemoryProbeJournal::default()
                )
                .unwrap_err()
                .code(),
            "local_directory_missing"
        );
    }
}
