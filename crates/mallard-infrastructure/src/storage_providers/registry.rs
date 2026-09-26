//! The compile-time storage-provider registry, keyed by provider key and
//! config version. Duplicate registrations are startup errors.

use super::traits::{StorageProviderAdapter, StorageProviderDescriptor};
use mallard_domain::StorageProviderKey;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct StorageProviderRegistry {
    adapters: BTreeMap<(StorageProviderKey, u32), Arc<dyn StorageProviderAdapter>>,
}

impl StorageProviderRegistry {
    /// Build a registry, refusing a repeated (key, version) coordinate.
    pub fn from_adapters(adapters: Vec<Arc<dyn StorageProviderAdapter>>) -> Result<Self, String> {
        let mut map: BTreeMap<(StorageProviderKey, u32), Arc<dyn StorageProviderAdapter>> =
            BTreeMap::new();
        for adapter in adapters {
            let descriptor = adapter.descriptor();
            if descriptor.config_version == 0 {
                return Err(format!(
                    "storage provider '{}' declares config version 0",
                    descriptor.key
                ));
            }
            let coordinate = (descriptor.key.clone(), descriptor.config_version);
            if map.insert(coordinate, adapter).is_some() {
                return Err(format!(
                    "storage provider '{}' config version {} is registered twice",
                    descriptor.key, descriptor.config_version
                ));
            }
        }
        Ok(Self { adapters: map })
    }

    /// The process-wide production registry: Mallard Cloud, R2, Dropbox,
    /// Feishu, and the local folder. Feishu is offered by every build; one
    /// that carries no Feishu app says so on its Sign in line.
    pub fn production() -> &'static StorageProviderRegistry {
        static REGISTRY: OnceLock<StorageProviderRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            StorageProviderRegistry::from_adapters(vec![
                Arc::new(super::managed_cloud::ManagedCloudAdapter),
                Arc::new(super::local::LocalFolderAdapter),
                Arc::new(super::r2::R2Adapter),
                Arc::new(super::dropbox::DropboxAdapter),
                Arc::new(super::feishu::FeishuAdapter),
            ])
            .expect("production storage registry has no duplicate registrations")
        })
    }

    /// A registry that additionally exposes the test storage provider.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_provider() -> StorageProviderRegistry {
        StorageProviderRegistry::from_adapters(vec![
            Arc::new(super::managed_cloud::ManagedCloudAdapter),
            Arc::new(super::local::LocalFolderAdapter),
            Arc::new(super::r2::R2Adapter),
            Arc::new(super::dropbox::DropboxAdapter),
            Arc::new(super::feishu::FeishuAdapter),
            Arc::new(super::test_provider::TestStorageAdapter),
        ])
        .expect("test storage registry has no duplicate registrations")
    }

    pub fn get(
        &self,
        key: &StorageProviderKey,
        version: u32,
    ) -> Option<&dyn StorageProviderAdapter> {
        self.adapters.get(&(key.clone(), version)).map(Arc::as_ref)
    }

    pub fn require(
        &self,
        key: &StorageProviderKey,
        version: u32,
    ) -> Result<&dyn StorageProviderAdapter, String> {
        self.get(key, version).ok_or_else(|| {
            if self.adapters.keys().any(|(candidate, _)| candidate == key) {
                format!(
                    "storage provider '{}' config version {} is not supported by this build",
                    key, version
                )
            } else {
                format!("no storage provider is registered for '{}'", key)
            }
        })
    }

    /// The newest registered config version for a provider: what a new
    /// draft starts from.
    pub fn latest(&self, key: &StorageProviderKey) -> Option<&dyn StorageProviderAdapter> {
        self.adapters
            .iter()
            .filter(|((candidate, _), _)| candidate == key)
            .max_by_key(|((_, version), _)| *version)
            .map(|(_, adapter)| adapter.as_ref())
    }

    pub fn contains(&self, key: &StorageProviderKey, version: u32) -> bool {
        self.adapters.contains_key(&(key.clone(), version))
    }

    /// Descriptors of the newest version of every provider, in display order:
    /// rank first, then key.
    pub fn descriptors(&self) -> Vec<StorageProviderDescriptor> {
        let mut latest: BTreeMap<StorageProviderKey, StorageProviderDescriptor> = BTreeMap::new();
        for adapter in self.adapters.values() {
            let descriptor = adapter.descriptor();
            let replace = latest
                .get(&descriptor.key)
                .is_none_or(|current| current.config_version < descriptor.config_version);
            if replace {
                latest.insert(descriptor.key.clone(), descriptor);
            }
        }
        let mut descriptors = latest.into_values().collect::<Vec<_>>();
        descriptors.sort_by(|left, right| {
            left.display_rank
                .cmp(&right.display_rank)
                .then_with(|| left.key.cmp(&right.key))
        });
        descriptors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_registrations_are_rejected() {
        let error = StorageProviderRegistry::from_adapters(vec![
            Arc::new(super::super::local::LocalFolderAdapter),
            Arc::new(super::super::local::LocalFolderAdapter),
        ])
        .err()
        .expect("duplicate registration is rejected");
        assert_eq!(
            error,
            "storage provider 'local' config version 1 is registered twice"
        );
    }

    #[test]
    fn unknown_provider_and_unsupported_version_fail_distinctly() {
        let registry = StorageProviderRegistry::production();
        let local = StorageProviderKey::parse("local").unwrap();
        assert!(registry.get(&local, 1).is_some());
        assert_eq!(
            registry.require(&local, 2).err().as_deref(),
            Some("storage provider 'local' config version 2 is not supported by this build")
        );
        let unknown = StorageProviderKey::parse("unregistered").unwrap();
        assert_eq!(
            registry.require(&unknown, 1).err().as_deref(),
            Some("no storage provider is registered for 'unregistered'")
        );
        assert!(registry.latest(&unknown).is_none());
    }

    #[test]
    fn descriptors_follow_display_rank_then_key() {
        let descriptors = StorageProviderRegistry::with_test_provider().descriptors();
        let keys = descriptors
            .iter()
            .map(|descriptor| (descriptor.display_rank, descriptor.key.as_str().to_string()))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                (5, "managed_cloud".to_string()),
                (10, "r2".to_string()),
                (20, "dropbox".to_string()),
                (25, "feishu".to_string()),
                (30, "local".to_string()),
                (90, "teststore".to_string())
            ]
        );
        assert_eq!(
            descriptors[0].clear_mode,
            crate::storage_providers::traits::StorageClearMode::Unavailable
        );
        assert_eq!(
            descriptors[1].clear_mode,
            crate::storage_providers::traits::StorageClearMode::Unconditional
        );
        assert_eq!(
            descriptors[2].clear_mode,
            crate::storage_providers::traits::StorageClearMode::RevisionChecked
        );
        assert_eq!(
            descriptors[3].clear_mode,
            crate::storage_providers::traits::StorageClearMode::Unavailable
        );
        assert_eq!(
            descriptors[4].clear_mode,
            crate::storage_providers::traits::StorageClearMode::RevisionChecked
        );
        // Every build offers Mallard Cloud: the service is a gateway onto
        // the layout every storage has. Every build offers Feishu too; the
        // app it needs is a build fact its Sign in line reports.
        assert!(StorageProviderRegistry::production()
            .latest(&StorageProviderKey::parse("managed_cloud").unwrap())
            .is_some());
        assert!(StorageProviderRegistry::production()
            .latest(&StorageProviderKey::parse("feishu").unwrap())
            .is_some());
    }
}
