//! Neutral storage transport shared by the repository bundle engine and the
//! storage-scoped catalog.
//!
//! This layer owns the closed `.mallard/` key namespaces, the object-store
//! trait with its local-folder and S3 implementations, and the storage layout
//! marker. It knows nothing about bundles, projects, or catalog resources
//! beyond the shape of their object keys; each protocol keeps its own engine
//! on top.

pub mod local_store;
pub mod marker;
pub mod object_store;
pub mod s3_store;

pub(crate) use local_store::open_read_only_no_follow;
pub use local_store::LocalObjectStore;
pub use marker::{ensure_storage_marker, StorageMarker};
pub(crate) use object_store::sha256_reader;
pub use object_store::{
    CasExpectation, CasOutcome, DeleteOutcome, ExpectedObject, ImmutablePutOutcome, ObjectStore,
    StageOutcome, StoreListEntry, StoreListPage, StoredMetadata, VerifiedBody, VerifiedBodyReader,
};
pub use s3_store::{
    R2CapabilityProbeError, R2CapabilityProbeErrorKind, R2CapabilityProbeReceipt,
    R2CapabilityProbeStage, S3Connection, S3ObjectStore,
};

use crate::storage_providers::StorageProviderRegistry;
use mallard_domain::{BundleId, StorageConfig};
use serde::Deserialize;
use std::io::{BufReader, Read};
use std::sync::Arc;
use tokio::runtime::Handle;

pub const MALLARD_ROOT: &str = ".mallard";
pub const STORAGE_MARKER_KEY: &str = ".mallard/_storage.json";
pub const REPOSITORY_PREFIX: &str = ".mallard/v1/repositories";
/// Flat discovery namespace: one tag object per bundle. Listing it costs
/// bundle count, never payload-object count, so tags live beside the bundle
/// prefixes rather than inside them. `_tags` can never collide with a bundle
/// component because bundle IDs are exactly 32 hex characters.
pub const REPOSITORY_TAGS_PREFIX: &str = ".mallard/v1/repositories/_tags";
pub const LIBRARY_PREFIX: &str = ".mallard/v1/library";
pub const LOCAL_STORAGE_LOCK: &str = ".storage.lock";
pub const HEAD_FILE: &str = "_head.json";
pub const TAG_FILE: &str = "_tag.json";
pub const MAX_STORAGE_MARKER_BYTES: u64 = 4 * 1024;
pub(crate) const MAX_LIST_PAGE: usize = 10_000;
const STORAGE_CLEAR_PAGE_SIZE: usize = 1_000;
const MAX_STORAGE_CLEAR_OBJECTS: u64 = 1_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManagedStorageClearResult {
    pub repository_objects_deleted: u64,
    pub library_objects_deleted: u64,
}

/// A validated absolute store key inside one of the closed `.mallard/`
/// namespaces: the storage marker, one repository bundle's objects, or one
/// Library resource repository. Anything else never becomes a key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_store_path(&value, 2)?;
        if value == STORAGE_MARKER_KEY {
            return Ok(Self(value));
        }
        validate_store_path(&value, 5)?;
        if !value.starts_with(&format!("{}/", REPOSITORY_PREFIX))
            && !value.starts_with(&format!("{}/", LIBRARY_PREFIX))
        {
            return Err(format!(
                "object key is outside the Mallard storage namespace: '{}'",
                value
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObjectKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A validated listing prefix. Project repositories and draft-linked Library
/// resource repositories are independently enumerable. Only the repository
/// and Library namespaces are representable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPrefix(String);

impl ObjectPrefix {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into().trim_end_matches('/').to_string();
        validate_store_path(&value, 3)?;
        let repository =
            value == REPOSITORY_PREFIX || value.starts_with(&format!("{}/", REPOSITORY_PREFIX));
        let library = value == LIBRARY_PREFIX || value.starts_with(&format!("{}/", LIBRARY_PREFIX));
        if !repository && !library {
            return Err(format!(
                "object prefix is outside the repository and Library namespaces: '{}'",
                value
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Delete every Mallard repository and Library object while retaining the
/// storage marker and every object outside Mallard's two managed namespaces.
/// Repeated first-page deletion avoids cursor gaps as the key set shrinks.
pub fn clear_managed_storage_data<S: ObjectStore + ?Sized>(
    store: &S,
) -> Result<ManagedStorageClearResult, String> {
    store.clear_managed_data()
}

/// The only shared storage-clear loop. The delete callback is private to this
/// module and its transports; it cannot be used by normal object operations.
fn clear_managed_data_with<S: ObjectStore + ?Sized>(
    store: &S,
    delete: impl Fn(&StoreListEntry) -> Result<DeleteOutcome, String>,
) -> Result<ManagedStorageClearResult, String> {
    ensure_storage_marker(store)?;
    let mut deleted = 0_u64;
    let mut attempted = 0_u64;
    let mut clear_prefix = |raw_prefix: &str| -> Result<u64, String> {
        let prefix = ObjectPrefix::parse(raw_prefix)?;
        let before = deleted;
        loop {
            let page = store.list(&prefix, None, STORAGE_CLEAR_PAGE_SIZE)?;
            if page.entries.is_empty() {
                break;
            }
            for entry in page.entries {
                // Check the boundary again before deletion, even if a transport
                // returns a valid key from the wrong namespace.
                if !entry
                    .key
                    .as_str()
                    .strip_prefix(raw_prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                {
                    return Err("storage clear listing crossed a namespace boundary".to_string());
                }
                if attempted >= MAX_STORAGE_CLEAR_OBJECTS {
                    return Err(format!(
                        "storage clear exceeds the {}-object safety limit",
                        MAX_STORAGE_CLEAR_OBJECTS
                    ));
                }
                attempted += 1;
                match delete(&entry)? {
                    DeleteOutcome::Deleted => deleted += 1,
                    DeleteOutcome::Missing => {}
                    DeleteOutcome::Conflict { .. } => {
                        return Err("an object changed during storage clear".to_string());
                    }
                }
            }
        }
        Ok(deleted - before)
    };
    let result = (|| {
        let repository_objects_deleted = clear_prefix(REPOSITORY_PREFIX)?;
        let library_objects_deleted = clear_prefix(LIBRARY_PREFIX)?;
        Ok(ManagedStorageClearResult {
            repository_objects_deleted,
            library_objects_deleted,
        })
    })();
    result.map_err(|error: String| {
        let progress = if deleted > 0 {
            format!("Some data was deleted ({} objects confirmed).", deleted)
        } else if attempted > 0 {
            "Some data may have been deleted.".to_string()
        } else {
            "No objects were deleted.".to_string()
        };
        format!("Storage clear stopped. {progress} Local sync metadata was not reset. Stop sync on all machines, then retry Clear storage data. {error}")
    })
}

pub(crate) fn validate_store_path(value: &str, minimum_components: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 4096
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(format!("unsafe store path '{}'", value.escape_debug()));
    }
    let components = value.split('/').collect::<Vec<_>>();
    if components.len() < minimum_components
        || components.iter().any(|component| {
            component.is_empty() || *component == "." || *component == ".." || component.len() > 255
        })
    {
        return Err(format!("unsafe store path '{}'", value.escape_debug()));
    }
    Ok(())
}

fn validate_relative_object_key_characters(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 2048
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe {} '{}'", label, value));
    }
    Ok(())
}

pub(crate) fn validate_bundle_relative_object_key(value: &str) -> Result<(), String> {
    validate_relative_object_key_characters(value, "bundle-relative object key")?;
    if !(value == HEAD_FILE
        || value.starts_with("_manifests/")
        || value.starts_with("_commits/")
        || value.starts_with("_objects/")
        || value.starts_with("_uploads/"))
    {
        return Err(format!("unknown bundle object namespace '{}'", value));
    }
    Ok(())
}

/// The only constructor for repository object keys. The relative part must
/// stay inside the bundle namespace allowlist; discovery tags are not
/// bundle-relative — they have their own constructor below.
pub fn repository_object_key(bundle_id: &BundleId, relative: &str) -> Result<ObjectKey, String> {
    validate_bundle_relative_object_key(relative)?;
    ObjectKey::parse(format!("{}/{}/{}", REPOSITORY_PREFIX, bundle_id, relative))
}

/// The only constructor for one bundle's discovery tag key.
pub fn repository_tag_key(bundle_id: &BundleId) -> Result<ObjectKey, String> {
    ObjectKey::parse(format!("{}/{}.json", REPOSITORY_TAGS_PREFIX, bundle_id))
}

fn validate_library_relative_object_key(value: &str) -> Result<(), String> {
    validate_relative_object_key_characters(value, "library-relative object key")?;
    if !(value == HEAD_FILE
        || value == TAG_FILE
        || value.starts_with("_manifests/")
        || value.starts_with("_commits/")
        || value.starts_with("_objects/"))
    {
        return Err(format!("unknown Library object namespace '{}'", value));
    }
    Ok(())
}

/// The only constructor for one draft-linked Library resource repository.
/// Repository IDs are validated here again so a raw path supplied by another
/// layer can never escape or alias a sibling repository.
pub fn library_object_key(repository_id: &str, relative: &str) -> Result<ObjectKey, String> {
    let Some(hex) = repository_id.strip_prefix("library-") else {
        return Err(format!("invalid Library repository id '{}'", repository_id));
    };
    if hex.len() != 32
        || !hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!("invalid Library repository id '{}'", repository_id));
    }
    validate_library_relative_object_key(relative)?;
    ObjectKey::parse(format!("{}/{}/{}", LIBRARY_PREFIX, repository_id, relative))
}

pub(crate) fn parse_json_bytes<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    label: &str,
) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("parse {}: {}", label, e))
}

pub(crate) fn parse_json_reader<T: for<'de> Deserialize<'de>>(
    reader: impl Read,
    label: &str,
) -> Result<T, String> {
    serde_json::from_reader(BufReader::new(reader))
        .map_err(|error| format!("parse {}: {}", label, error))
}

pub struct ConfiguredObjectStore {
    inner: Arc<dyn ObjectStore>,
}

impl ConfiguredObjectStore {
    /// Open through the production storage registry.
    pub fn from_config(config: &StorageConfig) -> Result<Self, String> {
        Self::open_with(StorageProviderRegistry::production(), config)
    }

    /// Open through an explicit registry (tests register extra providers).
    pub fn open_with(
        registry: &StorageProviderRegistry,
        config: &StorageConfig,
    ) -> Result<Self, String> {
        config.validate()?;
        let adapter = registry.require(&config.provider, config.provider_config.version)?;
        adapter
            .validate_config(&config.provider_config)
            .map_err(|error| error.to_string())?;
        let inner = adapter.open(&config.provider_config, Handle::try_current().ok())?;
        Ok(Self { inner })
    }
}

impl ObjectStore for ConfiguredObjectStore {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        self.inner.get_metadata(key, max_bytes)
    }

    fn get_metadata_many(
        &self,
        keys: &[ObjectKey],
        max_bytes: u64,
    ) -> Result<Vec<Option<StoredMetadata>>, String> {
        self.inner.get_metadata_many(keys, max_bytes)
    }

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        self.inner.stage_verified(key, expected)
    }

    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> Result<Vec<StageOutcome>, String> {
        self.inner.stage_many_verified(requests)
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        self.inner.put_immutable(key, bytes)
    }

    fn put_many(
        &self,
        objects: Vec<(ObjectKey, bytes::Bytes)>,
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        self.inner.put_many(objects)
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        self.inner.compare_and_swap(key, expectation, bytes)
    }

    fn supports_conditional_delete(&self) -> bool {
        self.inner.supports_conditional_delete()
    }

    fn delete(&self, key: &ObjectKey, expected_etag: &str) -> Result<DeleteOutcome, String> {
        self.inner.delete(key, expected_etag)
    }

    fn clear_managed_data(&self) -> Result<ManagedStorageClearResult, String> {
        self.inner.clear_managed_data()
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        self.inner.list(prefix, cursor, limit)
    }

    fn local_root(&self) -> Option<&std::path::Path> {
        self.inner.local_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn bundle_id(number: u64) -> BundleId {
        BundleId::parse(format!("{:032x}", number)).unwrap()
    }

    #[test]
    fn object_keys_outside_the_repository_namespace_are_rejected() {
        assert!(
            ObjectKey::parse("unsupported/bundles/0123456789abcdef0123456789abcdef/_tag.json")
                .is_err()
        );
    }

    #[test]
    fn content_addressed_object_keys_stay_inside_the_allowlist() {
        let id = bundle_id(7);
        let digest = "a".repeat(64);
        assert!(repository_object_key(&id, &format!("_objects/{}", digest)).is_ok());
        assert!(repository_object_key(&id, "_objects/../escape").is_err());
        assert!(repository_object_key(&id, "_payloads/unknown-namespace").is_err());
    }

    #[test]
    fn discovery_tags_live_in_the_flat_namespace_only() {
        let id = bundle_id(7);
        assert_eq!(
            repository_tag_key(&id).unwrap().as_str(),
            format!(".mallard/v1/repositories/_tags/{}.json", id)
        );
        // The per-bundle namespace no longer admits a tag object.
        assert!(repository_object_key(&id, TAG_FILE).is_err());
        assert!(ObjectPrefix::parse(REPOSITORY_TAGS_PREFIX).is_ok());
    }

    #[test]
    fn repository_and_library_constructors_are_isolated() {
        let id = bundle_id(9);
        let library_id = "library-0123456789abcdef0123456789abcdef";
        assert!(repository_object_key(&id, "library/_head.json").is_err());
        assert!(
            library_object_key(library_id, &format!("repositories/{}/_head.json", id)).is_err()
        );
        let repository = repository_object_key(&id, HEAD_FILE).unwrap();
        assert!(repository.as_str().starts_with(REPOSITORY_PREFIX));
        let library = library_object_key(library_id, HEAD_FILE).unwrap();
        assert!(library.as_str().starts_with(LIBRARY_PREFIX));
        assert!(!library.as_str().starts_with(REPOSITORY_PREFIX));
    }

    #[test]
    fn library_keys_are_per_resource_and_legacy_catalog_listing_stays_closed() {
        let repository_id = "library-0123456789abcdef0123456789abcdef";
        assert_eq!(
            library_object_key(repository_id, HEAD_FILE)
                .unwrap()
                .as_str(),
            ".mallard/v1/library/library-0123456789abcdef0123456789abcdef/_head.json"
        );
        assert!(library_object_key(repository_id, TAG_FILE).is_ok());
        assert!(library_object_key(repository_id, "_objects/../escape").is_err());
        assert!(library_object_key("catalog-0123456789abcdef0123456789abcdef", HEAD_FILE).is_err());
        assert!(ObjectPrefix::parse(LIBRARY_PREFIX).is_ok());
        assert!(ObjectPrefix::parse(".mallard/v1/catalog").is_err());
    }

    #[test]
    fn legacy_catalog_keys_do_not_parse() {
        assert!(ObjectKey::parse(".mallard/v1/catalog/_head.json").is_err());
        // The repository namespace keeps its five-component floor.
        assert!(ObjectKey::parse(".mallard/v1/repositories/deadbeef").is_err());
        // Nothing outside the two versioned namespaces parses.
        assert!(ObjectKey::parse(".mallard/v1/other/_head.json").is_err());
    }

    #[test]
    fn configured_store_validates_the_complete_provider_config_before_opening() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("storage");
        let mut config = crate::storage_providers::local::local_storage_config(
            mallard_domain::StorageId::parse("storage-invalid").unwrap(),
            "Invalid",
            &root,
        );
        config
            .provider_config
            .secrets
            .insert(
                mallard_domain::SecretKey::parse("unexpected").unwrap(),
                mallard_domain::SecretValue::parse("secret-canary").unwrap(),
            )
            .unwrap();
        assert!(config.validate().is_ok());

        let error =
            ConfiguredObjectStore::open_with(StorageProviderRegistry::production(), &config)
                .err()
                .expect("the provider-invalid configuration must be rejected");

        assert_eq!(error, "A local folder storage has no secrets.");
        assert!(!root.exists());
    }

    #[test]
    fn clear_managed_data_deletes_repository_and_library_objects_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("storage");
        let store = LocalObjectStore::open(&root).unwrap();
        let repository = repository_object_key(&bundle_id(11), HEAD_FILE).unwrap();
        let library =
            library_object_key("library-0123456789abcdef0123456789abcdef", HEAD_FILE).unwrap();
        store.put_immutable(&repository, b"repository").unwrap();
        store.put_immutable(&library, b"library").unwrap();
        ensure_storage_marker(&store).unwrap();
        fs::write(root.join("user-file.txt"), b"keep").unwrap();

        let result = clear_managed_storage_data(&store).unwrap();

        assert_eq!(result.repository_objects_deleted, 1);
        assert_eq!(result.library_objects_deleted, 1);
        assert!(store.get_metadata(&repository, 64).unwrap().is_none());
        assert!(store.get_metadata(&library, 64).unwrap().is_none());
        assert!(root.join(STORAGE_MARKER_KEY).is_file());
        assert_eq!(fs::read(root.join("user-file.txt")).unwrap(), b"keep");
        let second_clear = clear_managed_storage_data(&store).unwrap();
        assert_eq!(second_clear.repository_objects_deleted, 0);
        assert_eq!(second_clear.library_objects_deleted, 0);
    }
}
