//! Project-bundle storage, publication, fetch, restore planning, and apply.
//!
//! The engine operates on validated logical/store keys and a small object
//! store trait. Its local implementation is complete; an S3 adapter can reuse
//! the same immutable-object and head-CAS protocol without exposing UI paths
//! to manifests.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

use super::domain::{
    self, ActionId, ActionStatus, ApplyPolicy, ApplyReceipt, BindingState, BundleDirectoryEntry,
    BundleFileEntry, BundleHead, BundleId, BundleIdentity, BundleKind, BundleManifest,
    BundleRecipe, BundleSnapshot, CapturedWith, ConversationThread, LogicalPath, PlanId,
    PortableOrigin, ProjectBinding, Provider, ResourceDescriptor, ResourceId, ResourceKind,
    RestoreAction, RestoreActionKind, RestorePlan, StorageId, ThreadId, Tombstone, TombstoneTarget,
    BUNDLE_SCHEMA, RESTORE_PLAN_SCHEMA,
};
#[cfg(test)]
use super::provider_capture;
use super::provider_capture::CapturedResources;

const MALLARD_ROOT: &str = ".mallard";
const STORAGE_MARKER_KEY: &str = ".mallard/_storage.json";
const REPOSITORY_PREFIX: &str = ".mallard/v1/repositories";
const STORAGE_MARKER_FORMAT: &str = "mallard-storage";
const STORAGE_LAYOUT_VERSION: u32 = 1;
const LOCAL_STORAGE_LOCK: &str = ".storage.lock";
const HEAD_FILE: &str = "_head.json";
const TAG_FILE: &str = "_tag.json";
const MAX_STORAGE_MARKER_BYTES: u64 = 4 * 1024;
const MAX_BUNDLE_HEAD_BYTES: u64 = 64 * 1024;
const MAX_BUNDLE_TAG_BYTES: u64 = 64 * 1024;
const MAX_LIST_PAGE: usize = 10_000;
const DEFAULT_PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;

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
        if !value.starts_with(&format!("{}/", REPOSITORY_PREFIX)) {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPrefix(String);

impl ObjectPrefix {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into().trim_end_matches('/').to_string();
        validate_store_path(&value, 3)?;
        if value != REPOSITORY_PREFIX && !value.starts_with(&format!("{}/", REPOSITORY_PREFIX)) {
            return Err(format!(
                "object prefix is outside {}: '{}'",
                REPOSITORY_PREFIX, value
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMetadata {
    pub bytes: Vec<u8>,
    pub etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedObject {
    pub size: u64,
    pub sha256: String,
}

struct VerifiedBodyInner {
    file: Mutex<NamedTempFile>,
}

pub struct VerifiedBodyReader {
    inner: Arc<VerifiedBodyInner>,
    position: u64,
}

impl Read for VerifiedBodyReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let mut staged = self
            .inner
            .file
            .lock()
            .map_err(|_| std::io::Error::other("verified object lock is poisoned"))?;
        staged.as_file_mut().seek(SeekFrom::Start(self.position))?;
        let count = staged.as_file_mut().read(buffer)?;
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| std::io::Error::other("verified object reader position overflow"))?;
        Ok(count)
    }
}

/// A fully received and integrity-verified object backed by a private
/// machine-local temporary file. Clones share ownership of the staged bytes;
/// each [`Self::open`] call returns an independent reader at byte zero.
#[derive(Clone)]
pub struct VerifiedBody {
    inner: Arc<VerifiedBodyInner>,
    size: u64,
    sha256: String,
    etag: String,
}

impl std::fmt::Debug for VerifiedBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedBody")
            .field("size", &self.size)
            .field("sha256", &self.sha256)
            .field("etag", &self.etag)
            .finish_non_exhaustive()
    }
}

impl VerifiedBody {
    pub(crate) fn from_temp_file(
        mut file: NamedTempFile,
        size: u64,
        sha256: String,
        etag: String,
    ) -> Result<Self, String> {
        domain::validate_sha256("verified object", &sha256)?;
        let actual_size = file
            .as_file()
            .metadata()
            .map_err(|error| format!("inspect verified object: {}", error))?
            .len();
        if actual_size != size {
            return Err("verified object size differs from its staged file".to_string());
        }
        file.as_file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(|error| format!("rewind verified object: {}", error))?;
        Ok(Self {
            inner: Arc::new(VerifiedBodyInner {
                file: Mutex::new(file),
            }),
            size,
            sha256,
            etag,
        })
    }

    pub fn open(&self) -> Result<VerifiedBodyReader, String> {
        Ok(VerifiedBodyReader {
            inner: self.inner.clone(),
            position: 0,
        })
    }

    pub fn read_bounded(&self, max_bytes: u64, label: &str) -> Result<Vec<u8>, String> {
        if self.size > max_bytes {
            return Err(format!("{} exceeds its read limit", label));
        }
        let capacity = usize::try_from(self.size)
            .map_err(|_| format!("{} cannot fit in memory on this platform", label))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| format!("reserve memory for {}", label))?;
        Read::take(self.open()?, max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read {}: {}", label, error))?;
        if bytes.len() as u64 != self.size || sha256(&bytes) != self.sha256 {
            return Err(format!("{} changed after verification", label));
        }
        Ok(bytes)
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn etag(&self) -> &str {
        &self.etag
    }
}

#[derive(Clone, Debug)]
pub enum StageOutcome {
    Missing,
    Mismatch,
    Verified(VerifiedBody),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasExpectation {
    Absent,
    Match(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasOutcome {
    Written { etag: String },
    Conflict { current_etag: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImmutablePutOutcome {
    Written,
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreListPage {
    pub keys: Vec<ObjectKey>,
    pub next_cursor: Option<String>,
}

/// The narrow transport contract required by the bundle protocol.
pub trait BundleObjectStore: Send + Sync {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String>;

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String>;

    /// Fetch several verified objects; results line up with `requests`.
    /// Remote stores override this with a bounded-concurrency implementation.
    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> Result<Vec<StageOutcome>, String> {
        requests
            .iter()
            .map(|(key, expected)| self.stage_verified(key, expected))
            .collect()
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String>;

    /// Store several immutable objects; outcomes line up with `objects`.
    /// Remote stores override this with a bounded-concurrency implementation.
    fn put_many(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        objects
            .iter()
            .map(|(key, bytes)| self.put_immutable(key, bytes))
            .collect()
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String>;
    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String>;

    /// Used only for local isolation checks; remote stores return `None`.
    fn local_root(&self) -> Option<&Path> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct LocalBundleObjectStore {
    root: PathBuf,
}

impl LocalBundleObjectStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(format!(
                "local bundle store must be absolute: '{}'",
                root.display()
            ));
        }
        if let Ok(meta) = fs::symlink_metadata(root) {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!(
                    "local store '{}' must be a real directory",
                    root.display()
                ));
            }
        } else {
            fs::create_dir_all(root)
                .map_err(|e| format!("create local store '{}': {}", root.display(), e))?;
        }
        let root = fs::canonicalize(root)
            .map_err(|e| format!("resolve local store '{}': {}", root.display(), e))?;
        let mallard_root = root.join(MALLARD_ROOT);
        match fs::symlink_metadata(&mallard_root) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "Mallard storage namespace '{}' must be a real directory",
                    mallard_root.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&mallard_root) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(format!(
                            "create Mallard storage namespace '{}': {}",
                            mallard_root.display(),
                            error
                        ))
                    }
                }
                let meta = fs::symlink_metadata(&mallard_root).map_err(|error| {
                    format!(
                        "inspect Mallard storage namespace '{}': {}",
                        mallard_root.display(),
                        error
                    )
                })?;
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    return Err(format!(
                        "Mallard storage namespace '{}' must be a real directory",
                        mallard_root.display()
                    ));
                }
            }
            Err(error) => {
                return Err(format!(
                    "inspect Mallard storage namespace '{}': {}",
                    mallard_root.display(),
                    error
                ))
            }
        }
        Ok(Self { root })
    }

    fn lock(&self) -> Result<fs::File, String> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(MALLARD_ROOT).join(LOCAL_STORAGE_LOCK))
            .map_err(|e| format!("open Mallard storage lock: {}", e))?;
        file.lock()
            .map_err(|e| format!("lock local Mallard storage: {}", e))?;
        Ok(file)
    }

    fn open_object_unlocked(
        &self,
        key: &ObjectKey,
    ) -> Result<Option<(fs::File, fs::Metadata)>, String> {
        let path = checked_existing_object_path(&self.root, key)?;
        let Some(path) = path else {
            return Ok(None);
        };
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("inspect '{}': {}", key, e))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("object '{}' is not a regular no-follow file", key));
        }
        let file = open_read_only_no_follow(&path, &format!("object '{}'", key))?;
        let opened_meta = file
            .metadata()
            .map_err(|error| format!("inspect opened object '{}': {}", key, error))?;
        if !opened_meta.is_file() || opened_meta.len() != meta.len() {
            return Err(format!("object '{}' changed while opening", key));
        }
        Ok(Some((file, opened_meta)))
    }

    fn read_metadata_unlocked(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        let Some((mut file, metadata)) = self.open_object_unlocked(key)? else {
            return Ok(None);
        };
        if metadata.len() > max_bytes {
            return Err(format!("metadata object '{}' exceeds its read limit", key));
        }
        let capacity = usize::try_from(metadata.len())
            .map_err(|_| format!("metadata object '{}' is too large for this platform", key))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| format!("reserve memory for metadata object '{}'", key))?;
        Read::by_ref(&mut file)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read metadata object '{}': {}", key, error))?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!("metadata object '{}' exceeds its read limit", key));
        }
        Ok(Some(StoredMetadata {
            etag: sha256(&bytes),
            bytes,
        }))
    }

    fn stage_verified_unlocked(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        domain::validate_sha256("expected object", &expected.sha256)?;
        let Some((mut source, metadata)) = self.open_object_unlocked(key)? else {
            return Ok(StageOutcome::Missing);
        };
        if metadata.len() != expected.size {
            return Ok(StageOutcome::Mismatch);
        }
        let mut staged = NamedTempFile::new()
            .map_err(|error| format!("create staged object '{}': {}", key, error))?;
        let mut hasher = Sha256::new();
        let mut received = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|error| format!("read object '{}': {}", key, error))?;
            if count == 0 {
                break;
            }
            received = received
                .checked_add(count as u64)
                .ok_or_else(|| format!("object '{}' byte count overflow", key))?;
            if received > expected.size {
                return Ok(StageOutcome::Mismatch);
            }
            hasher.update(&buffer[..count]);
            staged
                .as_file_mut()
                .write_all(&buffer[..count])
                .map_err(|error| format!("stage object '{}': {}", key, error))?;
        }
        let digest = hex_lower(&hasher.finalize());
        if received != expected.size || digest != expected.sha256 {
            return Ok(StageOutcome::Mismatch);
        }
        staged
            .as_file_mut()
            .sync_all()
            .map_err(|error| format!("sync staged object '{}': {}", key, error))?;
        Ok(StageOutcome::Verified(VerifiedBody::from_temp_file(
            staged,
            received,
            digest.clone(),
            digest,
        )?))
    }

    fn current_etag_unlocked(&self, key: &ObjectKey) -> Result<Option<String>, String> {
        let Some((mut file, _)) = self.open_object_unlocked(key)? else {
            return Ok(None);
        };
        Ok(Some(sha256_reader(
            &mut file,
            &format!("object '{}'", key),
        )?))
    }

    fn write_atomic_unlocked(&self, key: &ObjectKey, bytes: &[u8]) -> Result<(), String> {
        let path = checked_create_object_path(&self.root, key)?;
        let parent = path
            .parent()
            .ok_or_else(|| format!("object '{}' has no parent", key))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("create object temp in '{}': {}", parent.display(), e))?;
        temp.as_file_mut()
            .write_all(bytes)
            .map_err(|e| format!("write object '{}': {}", key, e))?;
        temp.as_file_mut()
            .sync_all()
            .map_err(|e| format!("sync object '{}': {}", key, e))?;
        temp.persist(&path)
            .map_err(|e| format!("publish object '{}': {}", key, e.error))?;
        Ok(())
    }
}

impl BundleObjectStore for LocalBundleObjectStore {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        self.read_metadata_unlocked(key, max_bytes)
    }

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        self.stage_verified_unlocked(key, expected)
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        let _guard = self.lock()?;
        if let Some((mut existing, metadata)) = self.open_object_unlocked(key)? {
            if metadata.len() == bytes.len() as u64
                && sha256_reader(&mut existing, &format!("object '{}'", key))? == sha256(bytes)
            {
                return Ok(ImmutablePutOutcome::AlreadyPresent);
            }
            return Err(format!(
                "immutable object '{}' already exists with different bytes",
                key
            ));
        }
        self.write_atomic_unlocked(key, bytes)?;
        Ok(ImmutablePutOutcome::Written)
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        let _guard = self.lock()?;
        let current_etag = self.current_etag_unlocked(key)?;
        let matches = match expectation {
            CasExpectation::Absent => current_etag.is_none(),
            CasExpectation::Match(expected) => current_etag.as_ref() == Some(expected),
        };
        if !matches {
            return Ok(CasOutcome::Conflict { current_etag });
        }
        self.write_atomic_unlocked(key, bytes)?;
        Ok(CasOutcome::Written {
            etag: sha256(bytes),
        })
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err(format!(
                "list limit must be between 1 and {}",
                MAX_LIST_PAGE
            ));
        }
        if let Some(cursor) = cursor {
            ObjectKey::parse(cursor.to_string())?;
        }
        let namespace_root = self.root.join(prefix.as_str());
        let namespace_meta = match fs::symlink_metadata(&namespace_root) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoreListPage {
                    keys: Vec::new(),
                    next_cursor: None,
                })
            }
            Err(error) => {
                return Err(format!(
                    "inspect local Mallard repository namespace '{}': {}",
                    namespace_root.display(),
                    error
                ))
            }
        };
        if namespace_meta.file_type().is_symlink() || !namespace_meta.is_dir() {
            return Err(format!(
                "local Mallard repository namespace '{}' must be a real directory",
                namespace_root.display()
            ));
        }

        let mut keys = Vec::new();
        for entry in WalkDir::new(&namespace_root)
            .follow_links(false)
            .into_iter()
        {
            let entry = entry.map_err(|e| format!("walk local bundle store: {}", e))?;
            if entry.depth() == 0 {
                continue;
            }
            if entry.file_type().is_symlink() {
                return Err(format!(
                    "symlink '{}' is forbidden in local bundle storage",
                    entry.path().display()
                ));
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = normalized_relative_path(&self.root, entry.path())?;
            if relative.contains("/.tmp") {
                continue;
            }
            if relative == prefix.as_str() || relative.starts_with(&format!("{}/", prefix.as_str()))
            {
                let key = ObjectKey::parse(relative)?;
                if cursor.is_none_or(|cursor| key.as_str() > cursor) {
                    keys.push(key);
                }
            }
        }
        keys.sort();
        let has_more = keys.len() > limit;
        keys.truncate(limit);
        let next_cursor = has_more.then(|| keys.last().unwrap().as_str().to_string());
        Ok(StoreListPage { keys, next_cursor })
    }

    fn local_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadToken {
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishExpectation {
    Absent,
    Match(HeadToken),
}

#[derive(Clone, Debug)]
#[cfg(test)]
pub struct CapturedResourcesPublishRequest {
    pub identity: BundleIdentity,
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub captured: CapturedResources,
    /// Head observed by the caller's single read; the head CAS on its etag is
    /// the concurrency guard, so publish never re-reads the head itself.
    pub observed_head: Option<(BundleHead, HeadToken)>,
    pub updated_at: u64,
}

#[derive(Clone, Debug)]
pub struct PublishedBundle {
    /// False when the candidate manifest was semantically identical to the
    /// observed head's: nothing was written and `snapshot` is the observed
    /// revision's.
    pub published: bool,
    pub snapshot: BundleSnapshot,
}

#[derive(Clone, Debug)]
pub struct CapturedBundleFile {
    pub resource_id: ResourceId,
    pub bytes: Vec<u8>,
    pub source_mtime: u64,
    pub mode: Option<u32>,
}

/// Result of the explicit object durability audit (`verify_objects`).
#[derive(Clone, Debug, Serialize)]
pub struct BundleObjectAudit {
    pub revision: u64,
    pub commit_id: String,
    pub objects_total: usize,
    pub objects_verified: usize,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PublishBundleRequest {
    pub identity: BundleIdentity,
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub published_by_origin: PortableOrigin,
    pub conversations: BTreeMap<ThreadId, ConversationThread>,
    pub resources: BTreeMap<ResourceId, ResourceDescriptor>,
    /// Entries copied byte-for-byte from the reviewed previous manifest. The
    /// engine verifies every referenced immutable object before publishing.
    pub retained_files: BTreeMap<LogicalPath, BundleFileEntry>,
    /// New or changed payloads. A path cannot also appear in retained_files.
    pub captured_files: BTreeMap<LogicalPath, CapturedBundleFile>,
    pub directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
    pub tombstones: BTreeMap<String, Tombstone>,
    pub expected_head: PublishExpectation,
    pub updated_at: u64,
}

/// One payload object finished downloading. `fetched_bytes`/`total_bytes`
/// are manifest-declared sizes, so consumers can render byte-accurate
/// progress without waiting for the transfer to finish.
#[derive(Clone, Copy, Debug)]
pub struct FetchProgress {
    pub index: usize,
    pub total_objects: usize,
    pub fetched_bytes: u64,
    pub total_bytes: u64,
}

const OBJECT_CACHE_MAX_STAGED_BYTES: u64 = 256 * 1024 * 1024;
/// Objects per `stage_many_verified` round in `EngineByteSource::prefetch` — one
/// concurrent store window per batch, with progress after each object.
const PREFETCH_BATCH_OBJECTS: usize = 6;
/// Objects per `put_many` round in `publish_with_progress`; same shape as
/// the prefetch batching, with progress after each object.
const UPLOAD_BATCH_OBJECTS: usize = 6;

/// Process-wide LRU for staged verified payload objects, keyed by storage,
/// bundle, content hash, and exact size. Bundle objects are immutable, so an
/// entry can never go stale; eviction exists only to bound retained staging
/// space. One Pull review warms the cache for the apply that follows it.
pub struct ObjectCache {
    inner: Mutex<ObjectCacheInner>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ObjectCacheKey {
    storage_id: StorageId,
    bundle_id: BundleId,
    sha256: String,
    size: u64,
}

struct ObjectCacheInner {
    entries: HashMap<ObjectCacheKey, VerifiedBody>,
    order: VecDeque<ObjectCacheKey>,
    total_bytes: u64,
}

impl Default for ObjectCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(ObjectCacheInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
                total_bytes: 0,
            }),
        }
    }
}

impl ObjectCache {
    fn get(&self, key: &ObjectCacheKey) -> Option<VerifiedBody> {
        let mut inner = self.inner.lock().ok()?;
        let hit = inner.entries.get(key).cloned()?;
        if let Some(position) = inner.order.iter().position(|current| current == key) {
            inner.order.remove(position);
        }
        inner.order.push_back(key.clone());
        Some(hit)
    }

    fn insert(&self, key: ObjectCacheKey, body: VerifiedBody) {
        if body.size() > OBJECT_CACHE_MAX_STAGED_BYTES {
            return;
        }
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.entries.contains_key(&key) {
            return;
        }
        inner.total_bytes = inner.total_bytes.saturating_add(body.size());
        inner.entries.insert(key.clone(), body);
        inner.order.push_back(key);
        while inner.total_bytes > OBJECT_CACHE_MAX_STAGED_BYTES {
            let Some(oldest) = inner.order.pop_front() else {
                break;
            };
            if let Some(evicted) = inner.entries.remove(&oldest) {
                inner.total_bytes = inner.total_bytes.saturating_sub(evicted.size());
            }
        }
    }
}

/// Payload access for planning and apply. Implementations verify each
/// object against the manifest exactly once — callers trust returned bodies.
pub trait BundleBytes {
    fn body(&mut self, logical_path: &LogicalPath) -> Result<VerifiedBody, String>;

    fn read_bounded(
        &mut self,
        logical_path: &LogicalPath,
        max_bytes: u64,
        label: &str,
    ) -> Result<Vec<u8>, String>;

    /// Bulk-load a known set so progress totals reflect the real transfer.
    /// The default sequential loop is sufficient for simple sources.
    fn prefetch(&mut self, paths: &[LogicalPath]) -> Result<(), String> {
        for path in paths {
            self.body(path)?;
        }
        Ok(())
    }
}

/// Store-backed lazy byte source: downloads objects on demand, verifies them
/// against the manifest, and shares them through the process-wide
/// [`ObjectCache`].
pub struct EngineByteSource<'a, S: BundleObjectStore> {
    engine: &'a BundleEngine<S>,
    snapshot: &'a BundleSnapshot,
    cache: Option<Arc<ObjectCache>>,
    loaded: BTreeMap<LogicalPath, VerifiedBody>,
    on_progress: Option<&'a mut dyn FnMut(FetchProgress)>,
}

impl<'a, S: BundleObjectStore> EngineByteSource<'a, S> {
    fn ensure_loaded(&mut self, logical_path: &LogicalPath) -> Result<(), String> {
        if self.loaded.contains_key(logical_path) {
            return Ok(());
        }
        let entry = self
            .snapshot
            .manifest
            .files
            .get(logical_path)
            .ok_or_else(|| format!("manifest has no file entry for '{}'", logical_path))?;
        let bundle_id = &self.snapshot.head.bundle_id;
        let cache_key = ObjectCacheKey {
            storage_id: self.snapshot.storage_id.clone(),
            bundle_id: bundle_id.clone(),
            sha256: entry.sha256.clone(),
            size: entry.size,
        };
        if let Some(hit) = self.cache.as_ref().and_then(|cache| cache.get(&cache_key)) {
            self.loaded.insert(logical_path.clone(), hit);
            return Ok(());
        }
        let key = repository_object_key(bundle_id, &entry.object_key)?;
        let outcome = self.engine.store.stage_verified(
            &key,
            &ExpectedObject {
                size: entry.size,
                sha256: entry.sha256.clone(),
            },
        )?;
        let body = match outcome {
            StageOutcome::Missing => return Err(format!("bundle object '{}' is missing", key)),
            StageOutcome::Mismatch => {
                return Err(format!(
                    "bundle object '{}' does not match manifest size/hash",
                    key
                ))
            }
            StageOutcome::Verified(body) => body,
        };
        if let Some(cache) = &self.cache {
            cache.insert(cache_key, body.clone());
        }
        self.loaded.insert(logical_path.clone(), body);
        Ok(())
    }
}

impl<S: BundleObjectStore> BundleBytes for EngineByteSource<'_, S> {
    fn body(&mut self, logical_path: &LogicalPath) -> Result<VerifiedBody, String> {
        self.ensure_loaded(logical_path)?;
        Ok(self
            .loaded
            .get(logical_path)
            .expect("object loaded by ensure_loaded")
            .clone())
    }

    fn read_bounded(
        &mut self,
        logical_path: &LogicalPath,
        max_bytes: u64,
        label: &str,
    ) -> Result<Vec<u8>, String> {
        let entry = self
            .snapshot
            .manifest
            .files
            .get(logical_path)
            .ok_or_else(|| format!("manifest has no file entry for '{}'", logical_path))?;
        if entry.size > max_bytes {
            return Err(format!("{} exceeds its read limit", label));
        }
        self.body(logical_path)?.read_bounded(max_bytes, label)
    }

    fn prefetch(&mut self, paths: &[LogicalPath]) -> Result<(), String> {
        // Serve cache hits first so progress totals cover the real transfer.
        let mut pending = Vec::new();
        for path in paths {
            if self.loaded.contains_key(path) {
                continue;
            }
            let entry = self
                .snapshot
                .manifest
                .files
                .get(path)
                .ok_or_else(|| format!("manifest has no file entry for '{}'", path))?;
            let cache_key = ObjectCacheKey {
                storage_id: self.snapshot.storage_id.clone(),
                bundle_id: self.snapshot.head.bundle_id.clone(),
                sha256: entry.sha256.clone(),
                size: entry.size,
            };
            if let Some(hit) = self.cache.as_ref().and_then(|cache| cache.get(&cache_key)) {
                self.loaded.insert(path.clone(), hit);
                continue;
            }
            if pending.iter().any(|(current, _)| current == path) {
                continue;
            }
            pending.push((path.clone(), entry.clone()));
        }
        let total_objects = pending.len();
        let total_bytes = pending
            .iter()
            .fold(0_u64, |sum, (_, entry)| sum.saturating_add(entry.size));
        let mut fetched_bytes = 0_u64;
        let mut index = 0_usize;
        for batch in pending.chunks(PREFETCH_BATCH_OBJECTS) {
            let requests = batch
                .iter()
                .map(|(_, entry)| {
                    Ok((
                        repository_object_key(&self.snapshot.head.bundle_id, &entry.object_key)?,
                        ExpectedObject {
                            size: entry.size,
                            sha256: entry.sha256.clone(),
                        },
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let objects = self.engine.store.stage_many_verified(&requests)?;
            for (((path, entry), (key, _)), outcome) in batch.iter().zip(&requests).zip(objects) {
                let body = match outcome {
                    StageOutcome::Missing => {
                        return Err(format!("bundle object '{}' is missing", key))
                    }
                    StageOutcome::Mismatch => {
                        return Err(format!(
                            "bundle object '{}' does not match manifest size/hash",
                            key
                        ))
                    }
                    StageOutcome::Verified(body) => body,
                };
                if let Some(cache) = &self.cache {
                    cache.insert(
                        ObjectCacheKey {
                            storage_id: self.snapshot.storage_id.clone(),
                            bundle_id: self.snapshot.head.bundle_id.clone(),
                            sha256: entry.sha256.clone(),
                            size: entry.size,
                        },
                        body.clone(),
                    );
                }
                self.loaded.insert(path.clone(), body);
                index += 1;
                fetched_bytes = fetched_bytes.saturating_add(entry.size);
                if let Some(on_progress) = self.on_progress.as_mut() {
                    on_progress(FetchProgress {
                        index,
                        total_objects,
                        fetched_bytes,
                        total_bytes,
                    });
                }
            }
        }
        Ok(())
    }
}

/// Per-file planning decision from pass 1 of restore planning.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PlannedOutcome {
    /// Blocked or unmaterializable — becomes a manual action.
    Manual(String),
    /// The local conversation diverged; the reviewed Pull keeps it.
    KeepLocal,
    /// The local target already holds exactly the manifest bytes, so the
    /// plan emits nothing: a review must not ask about a write that changes
    /// nothing, and an all-deselected apply must not strand the reviewed
    /// baseline behind a no-op.
    UpToDate,
    /// Emit a restore action. Current-schema generic files materialize
    /// byte-for-byte, so the manifest digest is also the local digest.
    Action,
}

struct PlannedFile<'m> {
    logical_path: &'m LogicalPath,
    entry: &'m BundleFileEntry,
    descriptor: &'m ResourceDescriptor,
    target: Option<PathBuf>,
    target_digest: Option<String>,
    target_mode: Option<u32>,
    outcome: PlannedOutcome,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct CommitRecord {
    schema_version: u32,
    bundle_id: BundleId,
    revision: u64,
    commit_id: String,
    manifest_key: String,
    manifest_sha256: String,
    created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_commit_id: Option<String>,
    added_files: u64,
    changed_files: u64,
    removed_files: u64,
    published_by_origin: PortableOrigin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct BundleTag {
    schema_version: u32,
    bundle_id: BundleId,
    display_name: String,
    kind: BundleKind,
    revision: u64,
    updated_at: u64,
    resources: u64,
    files: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct StorageMarker {
    format: String,
    layout_version: u32,
}

impl StorageMarker {
    fn current() -> Self {
        Self {
            format: STORAGE_MARKER_FORMAT.to_string(),
            layout_version: STORAGE_LAYOUT_VERSION,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.format != STORAGE_MARKER_FORMAT {
            return Err(format!(
                "storage marker format '{}' is not supported; expected '{}'",
                self.format, STORAGE_MARKER_FORMAT
            ));
        }
        if self.layout_version != STORAGE_LAYOUT_VERSION {
            return Err(format!(
                "Mallard storage layout version {} is not supported; expected version {}",
                self.layout_version, STORAGE_LAYOUT_VERSION
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RemoteBundleSummary {
    pub bundle_id: BundleId,
    pub display_name: String,
    pub kind: BundleKind,
    pub revision: u64,
    pub updated_at: u64,
    pub resources: u64,
    pub files: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RemoteBundlePage {
    pub bundles: Vec<RemoteBundleSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub struct BundleEngine<S> {
    store: S,
    storage_id: StorageId,
}

impl<S: BundleObjectStore> BundleEngine<S> {
    pub fn open(store: S, storage_id: StorageId) -> Result<Self, String> {
        let engine = Self { store, storage_id };
        engine.ensure_storage_marker()?;
        Ok(engine)
    }

    fn ensure_storage_marker(&self) -> Result<(), String> {
        let key = ObjectKey::parse(STORAGE_MARKER_KEY)?;
        if let Some(stored) = self.store.get_metadata(&key, MAX_STORAGE_MARKER_BYTES)? {
            let marker: StorageMarker = parse_json_bytes(&stored.bytes, "Mallard storage marker")?;
            marker.validate()?;
            return Ok(());
        }

        let bytes = serde_json::to_vec(&StorageMarker::current())
            .map_err(|error| format!("serialize Mallard storage marker: {}", error))?;
        self.store.put_immutable(&key, &bytes)?;
        let stored = self
            .store
            .get_metadata(&key, MAX_STORAGE_MARKER_BYTES)?
            .ok_or_else(|| "Mallard storage marker disappeared after creation".to_string())?;
        let marker: StorageMarker = parse_json_bytes(&stored.bytes, "Mallard storage marker")?;
        marker.validate()
    }

    pub fn read_head(
        &self,
        bundle_id: &BundleId,
    ) -> Result<Option<(BundleHead, HeadToken)>, String> {
        let key = repository_object_key(bundle_id, HEAD_FILE)?;
        let Some(stored) = self.store.get_metadata(&key, MAX_BUNDLE_HEAD_BYTES)? else {
            return Ok(None);
        };
        let head: BundleHead = parse_json_bytes(&stored.bytes, "bundle head")?;
        head.validate()?;
        if &head.bundle_id != bundle_id {
            return Err(format!(
                "bundle head '{}' is stored under bundle '{}'",
                head.bundle_id, bundle_id
            ));
        }
        let token = HeadToken {
            revision: head.revision,
            commit_id: head.commit_id.clone(),
            manifest_sha256: head.manifest_sha256.clone(),
            manifest_size: head.manifest_size,
            etag: stored.etag,
        };
        Ok(Some((head, token)))
    }

    /// Publish immutable objects and history first, then compare-and-swap the
    /// single bundle head. A failed CAS can leave only unreachable immutable
    /// objects, never a partially visible revision. Payloads are content-
    /// addressed under `_objects/<sha256>`, so bytes already referenced by the
    /// previous manifest never travel again and a semantically unchanged
    /// publish writes nothing at all.
    #[cfg(test)]
    pub fn publish_test_resources(
        &self,
        request: CapturedResourcesPublishRequest,
    ) -> Result<PublishedBundle, String> {
        self.publish_test_resources_with_progress(request, &mut |_| {})
    }

    #[cfg(test)]
    pub fn publish_test_resources_with_progress(
        &self,
        mut request: CapturedResourcesPublishRequest,
        on_progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<PublishedBundle, String> {
        request.identity.validate()?;
        request.recipe.validate()?;
        let observed = request.observed_head.take();
        let previous = observed
            .as_ref()
            .map(|(head, _)| self.read_manifest(head))
            .transpose()?;
        let descriptors = provider_capture::domain_resources(&request.captured)?;
        let (resources, mut retained_files, mut directories, tombstones) =
            reconcile_manifest_content(
                &request.recipe,
                &descriptors,
                &request.captured,
                previous.as_ref(),
                request.updated_at,
            )?;
        let mut captured_files = BTreeMap::new();
        for captured in request.captured.files.values() {
            let path = LogicalPath::parse(captured.logical_path.clone())?;
            let resource_id = ResourceId::parse(captured.resource_id.clone())?;
            let digest = sha256(&captured.bytes);
            let mode = Some(captured.mode & 0o777);
            let retained = previous
                .as_ref()
                .and_then(|manifest| manifest.files.get(&path))
                .filter(|entry| {
                    entry.resource_id == resource_id
                        && entry.sha256 == digest
                        && entry.size == captured.bytes.len() as u64
                        && entry.source_mtime == captured.source_mtime
                        && entry.mode == mode
                })
                .cloned();
            if let Some(entry) = retained {
                retained_files.insert(path, entry);
            } else {
                retained_files.remove(&path);
                captured_files.insert(
                    path,
                    CapturedBundleFile {
                        resource_id,
                        bytes: captured.bytes.clone(),
                        source_mtime: captured.source_mtime,
                        mode,
                    },
                );
            }
        }
        for captured in request.captured.directories.values() {
            let path = LogicalPath::parse(captured.logical_path.clone())?;
            directories.insert(
                path,
                BundleDirectoryEntry {
                    resource_id: ResourceId::parse(captured.resource_id.clone())?,
                    mode: Some(captured.mode & 0o777),
                    source_mtime: captured.source_mtime,
                },
            );
        }
        let mut captured_with = request.captured_with;
        if let Some(previous) = &previous {
            for (codec, version) in &previous.captured_with.codec_versions {
                captured_with
                    .codec_versions
                    .entry(codec.clone())
                    .or_insert(*version);
            }
        }
        let published_by_origin = PortableOrigin {
            origin_host_token: domain::OriginHostToken::parse("0".repeat(64))?,
            origin_replica_token: domain::OriginReplicaToken::parse("0".repeat(64))?,
            shared_label: None,
        };
        self.publish_with_progress(
            PublishBundleRequest {
                identity: request.identity,
                recipe: request.recipe,
                captured_with,
                published_by_origin,
                conversations: previous
                    .as_ref()
                    .map(|manifest| manifest.conversations.clone())
                    .unwrap_or_default(),
                resources,
                retained_files,
                captured_files,
                directories,
                tombstones,
                expected_head: observed
                    .map(|(_, token)| PublishExpectation::Match(token))
                    .unwrap_or(PublishExpectation::Absent),
                updated_at: request.updated_at,
            },
            on_progress,
            &mut |_, _| {},
        )
    }
    /// Publish one complete reviewed bundle snapshot. The caller supplies the
    /// live resource and conversation sets; the engine enforces retention
    /// against the previous hash-verified manifest, skips true no-ops, and
    /// advances the head only through compare-and-swap. Uploads carry their
    /// sha256 checksum so the accepted PUT is the verification receipt;
    /// publish downloads nothing back. Durability audits are the separate
    /// `verify_objects` pass.
    #[cfg(test)]
    pub fn publish(&self, request: PublishBundleRequest) -> Result<PublishedBundle, String> {
        self.publish_with_progress(request, &mut |_| {}, &mut |_, _| {})
    }

    /// `on_phase` receives one timing per publish stage — head read,
    /// manifest build, uploads, immutable writes, head swing — so callers
    /// can log where a slow Push actually spends its time.
    pub fn publish_with_progress(
        &self,
        request: PublishBundleRequest,
        on_progress: &mut dyn FnMut(FetchProgress),
        on_phase: &mut dyn FnMut(&str, Duration),
    ) -> Result<PublishedBundle, String> {
        let phase_started = Instant::now();
        request.identity.validate()?;
        request.recipe.validate()?;
        request.published_by_origin.validate()?;
        if request.identity.kind != BundleKind::Project {
            return Err("bundle publication supports only project bundles".to_string());
        }
        if request.updated_at == 0 {
            return Err("publish timestamp must be non-zero".to_string());
        }
        for path in request.captured_files.keys() {
            if request.retained_files.contains_key(path) {
                return Err(format!(
                    "bundle file '{}' is both retained and captured",
                    path
                ));
            }
        }

        let current = self.read_head(&request.identity.bundle_id)?;
        let cas_expectation = match (&request.expected_head, &current) {
            (PublishExpectation::Absent, None) => CasExpectation::Absent,
            (PublishExpectation::Absent, Some(_)) => {
                return Err("bundle head changed: expected an absent head".to_string())
            }
            (PublishExpectation::Match(expected), Some((head, token)))
                if expected == token
                    && expected.revision == head.revision
                    && expected.commit_id == head.commit_id
                    && expected.manifest_sha256 == head.manifest_sha256
                    && expected.manifest_size == head.manifest_size =>
            {
                CasExpectation::Match(token.etag.clone())
            }
            (PublishExpectation::Match(_), _) => {
                return Err("bundle head changed before publication".to_string())
            }
        };
        let previous_manifest = current
            .as_ref()
            .map(|(head, _)| self.read_manifest(head))
            .transpose()?;
        on_phase(
            "publish: read head and previous manifest",
            phase_started.elapsed(),
        );

        let phase_started = Instant::now();
        for (path, entry) in &request.retained_files {
            entry.validate()?;
            let previous = previous_manifest
                .as_ref()
                .ok_or_else(|| format!("retained file '{}' has no previous manifest", path))?;
            let was_published = previous.files.values().any(|candidate| {
                candidate.object_key == entry.object_key
                    && candidate.sha256 == entry.sha256
                    && candidate.size == entry.size
            });
            if !was_published {
                return Err(format!(
                    "retained file '{}' was not in the reviewed manifest",
                    path
                ));
            }
        }

        let revision = current
            .as_ref()
            .map(|(head, _)| head.revision.saturating_add(1))
            .unwrap_or(1);
        let commit_id = random_named_id("commit")?;
        let mut files = request.retained_files.clone();
        for (path, captured) in &request.captured_files {
            if captured.mode.is_some_and(|mode| mode & !0o777 != 0) {
                return Err(format!("bundle file '{}' has unsafe mode bits", path));
            }
            if !request.resources.contains_key(&captured.resource_id) {
                return Err(format!(
                    "bundle file '{}' belongs to missing resource '{}'",
                    path, captured.resource_id
                ));
            }
            let digest = sha256(&captured.bytes);
            files.insert(
                path.clone(),
                BundleFileEntry {
                    resource_id: captured.resource_id.clone(),
                    sha256: digest.clone(),
                    size: captured.bytes.len() as u64,
                    source_mtime: captured.source_mtime,
                    object_key: format!("_objects/{}", digest),
                    mode: captured.mode,
                },
            );
        }

        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            revision,
            commit_id: commit_id.clone(),
            updated_at: request.updated_at,
            bundle: request.identity.clone(),
            recipe: request.recipe,
            captured_with: request.captured_with,
            published_by_origin: request.published_by_origin.clone(),
            conversations: request.conversations,
            resources: request.resources,
            files,
            directories: request.directories,
            tombstones: request.tombstones,
        };
        manifest.validate()?;
        if let Some(previous) = &previous_manifest {
            validate_conversation_retention(previous, &manifest)?;
        }
        on_phase(
            "publish: validate retention and build manifest",
            phase_started.elapsed(),
        );

        if let (Some((head, _)), Some(previous)) = (&current, &previous_manifest) {
            if manifest_is_semantic_noop(previous, &manifest) {
                return Ok(PublishedBundle {
                    published: false,
                    snapshot: BundleSnapshot {
                        storage_id: self.storage_id.clone(),
                        head: head.clone(),
                        manifest: previous.clone(),
                        fetched_at: request.updated_at,
                    },
                });
            }
        }

        let phase_started = Instant::now();
        let mut pending = BTreeMap::<String, Bytes>::new();
        for captured in request.captured_files.values() {
            pending
                .entry(sha256(&captured.bytes))
                .or_insert_with(|| Bytes::from(captured.bytes.clone()));
        }
        let total_objects = pending.len();
        let total_bytes = pending.values().map(|bytes| bytes.len() as u64).sum();
        let pending = pending.into_iter().collect::<Vec<_>>();
        let mut uploaded_objects = 0;
        let mut uploaded_bytes = 0_u64;
        for chunk in pending.chunks(UPLOAD_BATCH_OBJECTS) {
            let mut batch = Vec::with_capacity(chunk.len());
            for (digest, bytes) in chunk {
                let key = repository_object_key(
                    &request.identity.bundle_id,
                    &format!("_objects/{}", digest),
                )?;
                batch.push((key, bytes.clone()));
            }
            self.store.put_many(batch)?;
            for (_, bytes) in chunk {
                uploaded_objects += 1;
                uploaded_bytes = uploaded_bytes.saturating_add(bytes.len() as u64);
                on_progress(FetchProgress {
                    index: uploaded_objects,
                    total_objects,
                    fetched_bytes: uploaded_bytes,
                    total_bytes,
                });
            }
        }

        on_phase("publish: upload objects", phase_started.elapsed());

        let phase_started = Instant::now();
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|error| format!("serialize bundle manifest: {error}"))?;
        let manifest_size = u64::try_from(manifest_bytes.len())
            .map_err(|_| "serialized bundle manifest is too large".to_string())?;
        let manifest_sha256 = sha256(&manifest_bytes);
        let manifest_relative_key = format!("_manifests/{}-{}.json", revision, commit_id);
        let manifest_key =
            repository_object_key(&request.identity.bundle_id, &manifest_relative_key)?;
        self.store.put_immutable(&manifest_key, &manifest_bytes)?;

        let (added_files, changed_files, removed_files) =
            file_delta(previous_manifest.as_ref(), &manifest);
        let commit = CommitRecord {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: request.identity.bundle_id.clone(),
            revision,
            commit_id: commit_id.clone(),
            manifest_key: manifest_relative_key.clone(),
            manifest_sha256: manifest_sha256.clone(),
            created_at: request.updated_at,
            previous_commit_id: current.as_ref().map(|(head, _)| head.commit_id.clone()),
            added_files,
            changed_files,
            removed_files,
            published_by_origin: request.published_by_origin,
        };
        let commit_bytes = serde_json::to_vec(&commit)
            .map_err(|error| format!("serialize bundle commit: {error}"))?;
        let commit_key = repository_object_key(
            &request.identity.bundle_id,
            &format!("_commits/{}-{}.json", revision, commit_id),
        )?;
        self.store.put_immutable(&commit_key, &commit_bytes)?;
        on_phase(
            "publish: write manifest and commit",
            phase_started.elapsed(),
        );

        let phase_started = Instant::now();
        let head = BundleHead {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: request.identity.bundle_id.clone(),
            kind: request.identity.kind,
            revision,
            commit_id,
            manifest_key: manifest_relative_key,
            manifest_sha256,
            manifest_size,
            updated_at: request.updated_at,
        };
        head.validate()?;
        let head_bytes =
            serde_json::to_vec(&head).map_err(|error| format!("serialize bundle head: {error}"))?;
        let head_key = repository_object_key(&request.identity.bundle_id, HEAD_FILE)?;
        match self
            .store
            .compare_and_swap(&head_key, &cas_expectation, &head_bytes)?
        {
            CasOutcome::Written { .. } => {}
            CasOutcome::Conflict { .. } => {
                return Err("bundle head CAS conflict; fetch and rebase before retrying".to_string())
            }
        }
        let _ = self.publish_tag(&manifest);
        on_phase("publish: swing head", phase_started.elapsed());
        Ok(PublishedBundle {
            published: true,
            snapshot: BundleSnapshot {
                storage_id: self.storage_id.clone(),
                head: head.clone(),
                manifest,
                fetched_at: request.updated_at,
            },
        })
    }

    /// Lazy payload access for `snapshot`, sharing verified objects through
    /// `cache` and reporting bulk downloads through `on_progress`.
    pub fn byte_source<'a>(
        &'a self,
        snapshot: &'a BundleSnapshot,
        cache: Option<Arc<ObjectCache>>,
        on_progress: Option<&'a mut dyn FnMut(FetchProgress)>,
    ) -> EngineByteSource<'a, S> {
        EngineByteSource {
            engine: self,
            snapshot,
            cache,
            loaded: BTreeMap::new(),
            on_progress,
        }
    }

    /// Verify a bundle's mutable head and immutable manifest without loading
    /// payload objects. Discovery uses this to match repository identities
    /// without downloading every session or project file in a storage.
    pub fn inspect(&self, bundle_id: &BundleId) -> Result<BundleSnapshot, String> {
        self.inspect_optional(bundle_id)?
            .ok_or_else(|| format!("bundle '{}' does not exist", bundle_id))
    }

    /// The optional form is used by read-only status views where an empty
    /// linked destination is a valid state rather than an error.
    pub fn inspect_optional(&self, bundle_id: &BundleId) -> Result<Option<BundleSnapshot>, String> {
        Ok(self
            .inspect_optional_with_token(bundle_id)?
            .map(|(snapshot, _)| snapshot))
    }

    /// `inspect_optional` that also returns the head token, so a command can
    /// take one remote snapshot and later hand the observed head to `publish`
    /// without a second read.
    pub fn inspect_optional_with_token(
        &self,
        bundle_id: &BundleId,
    ) -> Result<Option<(BundleSnapshot, HeadToken)>, String> {
        let Some((head, token)) = self.read_head(bundle_id)? else {
            return Ok(None);
        };
        let manifest = self.read_manifest(&head)?;
        Ok(Some((
            BundleSnapshot {
                storage_id: self.storage_id.clone(),
                head,
                manifest,
                fetched_at: now_secs(),
            },
            token,
        )))
    }

    /// Load one immutable historical manifest using the coordinates retained
    /// by a reviewed local sync base. This reads metadata only; payload
    /// objects such as conversation rollouts are not downloaded.
    pub fn inspect_manifest_version(
        &self,
        bundle_id: &BundleId,
        revision: u64,
        commit_id: &str,
        manifest_size: u64,
        manifest_sha256: &str,
    ) -> Result<BundleManifest, String> {
        domain::validate_sha256("historical manifest", manifest_sha256)?;
        if manifest_size == 0 {
            return Err("historical manifest size must be nonzero".to_string());
        }
        let relative_key = format!("_manifests/{revision}-{commit_id}.json");
        validate_bundle_relative_object_key(&relative_key)?;
        let key = repository_object_key(bundle_id, &relative_key)?;
        let outcome = self.store.stage_verified(
            &key,
            &ExpectedObject {
                size: manifest_size,
                sha256: manifest_sha256.to_string(),
            },
        )?;
        let body = match outcome {
            StageOutcome::Missing => return Err(format!("bundle manifest '{}' is missing", key)),
            StageOutcome::Mismatch => {
                return Err(format!("bundle manifest '{}' failed verification", key))
            }
            StageOutcome::Verified(body) => body,
        };
        let manifest: BundleManifest = parse_json_reader(body.open()?, "bundle manifest")?;
        manifest.validate()?;
        if &manifest.bundle.bundle_id != bundle_id
            || manifest.revision != revision
            || manifest.commit_id != commit_id
        {
            return Err("historical manifest coordinates do not match its contents".to_string());
        }
        Ok(manifest)
    }

    /// Continuity probe (DESIGN_REMOTE_HISTORY_CONTINUITY.md): report whether
    /// the immutable manifest named by a reviewed base is still present in
    /// the store with exactly the recorded digest. This reads the store
    /// directly — never an object cache — because the answer must witness the
    /// remote repository, not a local copy of bytes it once held. `Ok(None)`
    /// covers both an absent key and bytes that no longer reproduce the
    /// digest; transport failures stay errors so the caller classifies
    /// continuity as unknown instead of guessing.
    pub fn fetch_manifest_version_optional(
        &self,
        bundle_id: &BundleId,
        revision: u64,
        commit_id: &str,
        manifest_size: u64,
        manifest_sha256: &str,
    ) -> Result<Option<BundleManifest>, String> {
        domain::validate_sha256("historical manifest", manifest_sha256)?;
        if manifest_size == 0 {
            return Err("historical manifest size must be nonzero".to_string());
        }
        let relative_key = format!("_manifests/{revision}-{commit_id}.json");
        validate_bundle_relative_object_key(&relative_key)?;
        let key = repository_object_key(bundle_id, &relative_key)?;
        let outcome = self.store.stage_verified(
            &key,
            &ExpectedObject {
                size: manifest_size,
                sha256: manifest_sha256.to_string(),
            },
        )?;
        let body = match outcome {
            StageOutcome::Missing | StageOutcome::Mismatch => return Ok(None),
            StageOutcome::Verified(body) => body,
        };
        let manifest: BundleManifest = parse_json_reader(body.open()?, "bundle manifest")?;
        manifest.validate()?;
        if &manifest.bundle.bundle_id != bundle_id
            || manifest.revision != revision
            || manifest.commit_id != commit_id
        {
            return Err("historical manifest coordinates do not match its contents".to_string());
        }
        Ok(Some(manifest))
    }

    /// Explicit durability audit: download every object the current manifest
    /// references and verify each against its recorded digest and size. This
    /// is the read-back publish deliberately does not perform; it runs only
    /// when invoked, never as part of Push or Pull.
    pub fn verify_objects(&self, bundle_id: &BundleId) -> Result<BundleObjectAudit, String> {
        let snapshot = self.inspect(bundle_id)?;
        let mut entries = BTreeMap::<String, (String, u64)>::new();
        for entry in snapshot.manifest.files.values() {
            entries
                .entry(entry.object_key.clone())
                .or_insert((entry.sha256.clone(), entry.size));
        }
        let entries = entries.into_iter().collect::<Vec<_>>();
        let mut verified = 0_usize;
        let mut failures = Vec::new();
        for batch in entries.chunks(PREFETCH_BATCH_OBJECTS) {
            let requests = batch
                .iter()
                .map(|(object_key, (expected_sha, expected_size))| {
                    Ok((
                        repository_object_key(bundle_id, object_key)?,
                        ExpectedObject {
                            size: *expected_size,
                            sha256: expected_sha.clone(),
                        },
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let objects = self.store.stage_many_verified(&requests)?;
            for ((key, _), outcome) in requests.iter().zip(objects) {
                match outcome {
                    StageOutcome::Missing => {
                        failures.push(format!("bundle object '{}' is missing", key))
                    }
                    StageOutcome::Mismatch => {
                        failures.push(format!(
                            "bundle object '{}' does not match manifest size/hash",
                            key
                        ));
                    }
                    StageOutcome::Verified(_) => verified += 1,
                }
            }
        }
        Ok(BundleObjectAudit {
            revision: snapshot.head.revision,
            commit_id: snapshot.head.commit_id.clone(),
            objects_total: entries.len(),
            objects_verified: verified,
            failures,
        })
    }

    /// Cursor-paginated remote discovery. The cursor is the last returned
    /// bundle ID, not a mutable global catalog offset.
    pub fn list_remote_bundles(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RemoteBundlePage, String> {
        if limit == 0 || limit > 500 {
            return Err("bundle page limit must be between 1 and 500".to_string());
        }
        let after = cursor.map(BundleId::parse).transpose()?;
        let prefix = ObjectPrefix::parse(REPOSITORY_PREFIX)?;
        let mut object_cursor = None;
        let mut tag_keys = Vec::new();
        let mut observed = 0_usize;
        loop {
            let page = self
                .store
                .list(&prefix, object_cursor.as_deref(), MAX_LIST_PAGE)?;
            observed = observed.saturating_add(page.keys.len());
            if observed > 1_000_000 {
                return Err("remote bundle listing exceeds one million objects".to_string());
            }
            tag_keys.extend(
                page.keys
                    .into_iter()
                    .filter(|key| key.as_str().ends_with("/_tag.json")),
            );
            let Some(next) = page.next_cursor else {
                break;
            };
            object_cursor = Some(next);
        }

        let mut summaries = Vec::new();
        for key in tag_keys {
            let Some(bundle_component) = key
                .as_str()
                .strip_prefix(&format!("{}/", REPOSITORY_PREFIX))
                .and_then(|rest| rest.strip_suffix("/_tag.json"))
            else {
                continue;
            };
            let bundle_id = BundleId::parse(bundle_component.to_string())?;
            if after.as_ref().is_some_and(|after| &bundle_id <= after) {
                continue;
            }
            let stored = self
                .store
                .get_metadata(&key, MAX_BUNDLE_TAG_BYTES)?
                .ok_or_else(|| format!("listed bundle tag '{}' disappeared", key))?;
            let tag: BundleTag = parse_json_bytes(&stored.bytes, "bundle tag")?;
            if tag.schema_version != BUNDLE_SCHEMA || tag.bundle_id != bundle_id {
                return Err(format!("bundle tag '{}' has inconsistent identity", key));
            }
            summaries.push(RemoteBundleSummary {
                bundle_id: tag.bundle_id,
                display_name: tag.display_name,
                kind: tag.kind,
                revision: tag.revision,
                updated_at: tag.updated_at,
                resources: tag.resources,
                files: tag.files,
            });
        }
        summaries.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
        let has_more = summaries.len() > limit;
        summaries.truncate(limit);
        let next_cursor = has_more.then(|| summaries.last().unwrap().bundle_id.to_string());
        Ok(RemoteBundlePage {
            bundles: summaries,
            next_cursor,
        })
    }

    /// Build a restore plan from a verified current-schema manifest.
    ///
    /// Generic resources materialize byte-for-byte, so planning derives every
    /// local digest from manifest metadata and never downloads payloads.
    /// Typed conversations are omitted here and added as reviewed
    /// conversation actions by the command layer.
    pub fn build_restore_plan_from_snapshot(
        &self,
        snapshot: &BundleSnapshot,
        binding: &ProjectBinding,
        created_at: u64,
        expected_host_identity_revision: u64,
    ) -> Result<RestorePlan, String> {
        snapshot.validate()?;
        self.validate_binding(binding, None)?;
        if binding.state != BindingState::Active {
            return Err("restore requires an active project binding".to_string());
        }
        if binding.bundle_id != snapshot.head.bundle_id {
            return Err("binding and fetched bundle IDs differ".to_string());
        }
        if snapshot.storage_id != self.storage_id {
            return Err("fetched bundle belongs to another storage".to_string());
        }
        let plan_id = PlanId::parse(random_named_id("plan")?)?;
        let mut actions = Vec::new();
        let mut target_owners = BTreeMap::<String, LogicalPath>::new();
        let mut continuation_ids = BTreeSet::new();
        let typed_conversation_resources = snapshot
            .manifest
            .conversations
            .values()
            .map(|thread| thread.resource_id.clone())
            .collect::<BTreeSet<_>>();

        for (logical_path, entry) in &snapshot.manifest.directories {
            let descriptor = snapshot
                .manifest
                .resources
                .get(&entry.resource_id)
                .ok_or_else(|| format!("directory '{}' has no resource", logical_path))?;
            if descriptor.kind != ResourceKind::ProjectContentDirectory {
                return Err(format!(
                    "directory '{}' is not project-content metadata",
                    logical_path
                ));
            }
            let target = project_content_target(logical_path, binding)?;
            let expected_target_mode = match inspect_project_directory_target(&target) {
                Ok(mode) => mode,
                Err(error) => {
                    actions.push(manual_action(
                        &entry.resource_id,
                        &format!(
                            "Project directory '{}' is blocked at '{}': {}",
                            logical_path,
                            target.display(),
                            error
                        ),
                    )?);
                    continue;
                }
            };
            let mode = entry.mode.unwrap_or(0o700) & 0o777;
            let kind = RestoreActionKind::EnsureProjectDirectory {
                logical_path: logical_path.clone(),
                mode,
                source_mtime: entry.source_mtime,
            };
            actions.push(RestoreAction {
                action_id: action_id_for(
                    &entry.resource_id,
                    logical_path.as_str(),
                    kind.action_type(),
                )?,
                resource_id: entry.resource_id.clone(),
                kind,
                target_path: Some(path_text(&target)?),
                source_sha256: Some(directory_entry_digest(logical_path, mode)),
                expected_target_sha256: None,
                expected_target_mode,
                materialized_sha256: None,
                requires_explicit_approval: true,
            });
        }

        // Files, pass 1: inspect targets and decide the metadata-only outcome.
        let mut planned_files = Vec::new();
        for (logical_path, entry) in &snapshot.manifest.files {
            let descriptor = snapshot
                .manifest
                .resources
                .get(&entry.resource_id)
                .ok_or_else(|| format!("file '{}' has no resource", logical_path))?;
            if typed_conversation_resources.contains(&entry.resource_id) {
                continue;
            }
            let Some(target) = map_logical_target(logical_path, descriptor, binding)? else {
                planned_files.push(PlannedFile {
                    logical_path,
                    entry,
                    descriptor,
                    target: None,
                    target_digest: None,
                    target_mode: None,
                    outcome: PlannedOutcome::Manual(format!(
                        "No safe materializer is available for {}",
                        logical_path
                    )),
                });
                continue;
            };
            let folded = target.to_string_lossy().to_lowercase();
            if let Some(previous) = target_owners.insert(folded, logical_path.clone()) {
                return Err(format!(
                    "logical files '{}' and '{}' map to one target",
                    previous, logical_path
                ));
            }
            let target_state = match inspect_restore_target(&target) {
                Ok(state) => state,
                Err(error) if descriptor.kind == ResourceKind::ProjectContentFile => {
                    planned_files.push(PlannedFile {
                        logical_path,
                        entry,
                        descriptor,
                        target: None,
                        target_digest: None,
                        target_mode: None,
                        outcome: PlannedOutcome::Manual(format!(
                            "Project file '{}' is blocked at '{}': {}",
                            logical_path,
                            target.display(),
                            error
                        )),
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            let target_digest = target_state.digest;
            let target_mode = target_state.mode;
            let conversation_kind = matches!(
                descriptor.kind,
                ResourceKind::CodexConversation | ResourceKind::ClaudeConversation
            );
            let outcome = if conversation_kind
                && target_digest
                    .as_ref()
                    .is_some_and(|digest| digest != &entry.sha256)
            {
                PlannedOutcome::KeepLocal
            } else if !conversation_kind
                && descriptor.kind != ResourceKind::ProjectContentFile
                && target_digest.as_deref() == Some(entry.sha256.as_str())
            {
                // Generic files have no enforced mode, so byte identity is
                // the whole restore; conversations and project files keep
                // their own receipt semantics.
                PlannedOutcome::UpToDate
            } else {
                PlannedOutcome::Action
            };
            planned_files.push(PlannedFile {
                logical_path,
                entry,
                descriptor,
                target: Some(target),
                target_digest,
                target_mode,
                outcome,
            });
        }

        // Files, pass 2: emit actions in manifest order.
        for planned in planned_files {
            let logical_path = planned.logical_path;
            let entry = planned.entry;
            let descriptor = planned.descriptor;
            match planned.outcome {
                PlannedOutcome::Manual(message) => {
                    actions.push(manual_action(&entry.resource_id, &message)?);
                    continue;
                }
                PlannedOutcome::KeepLocal => {
                    let target = planned.target.as_ref().expect("keep-local has a target");
                    let digest = planned
                        .target_digest
                        .as_deref()
                        .expect("keep-local requires a target digest");
                    actions.push(conversation_keep_local_action(
                        &entry.resource_id,
                        logical_path,
                        target,
                        &entry.sha256,
                        digest,
                    )?);
                    continue;
                }
                PlannedOutcome::UpToDate => continue,
                PlannedOutcome::Action => {}
            };
            let target = planned.target.as_ref().expect("file action has a target");
            let kind = restore_kind_for_file(logical_path, descriptor, entry)?;
            let requires_explicit_approval = action_needs_explicit_approval(
                descriptor,
                planned.target_digest.as_deref(),
                &entry.sha256,
            );
            actions.push(RestoreAction {
                action_id: action_id_for(
                    &entry.resource_id,
                    logical_path.as_str(),
                    kind.action_type(),
                )?,
                resource_id: entry.resource_id.clone(),
                kind,
                target_path: Some(path_text(target)?),
                source_sha256: Some(entry.sha256.clone()),
                expected_target_sha256: planned.target_digest.clone(),
                expected_target_mode: (descriptor.kind == ResourceKind::ProjectContentFile)
                    .then_some(planned.target_mode)
                    .flatten(),
                materialized_sha256: Some(entry.sha256.clone()),
                requires_explicit_approval,
            });
            if descriptor.relative_cwd.is_some()
                && matches!(descriptor.kind, ResourceKind::ClaudeConversation)
                && continuation_ids.insert(descriptor.resource_id.clone())
            {
                actions.push(continuation_action(descriptor, binding)?);
            }
        }
        for tombstone in snapshot.manifest.tombstones.values() {
            match &tombstone.target {
                TombstoneTarget::ProjectContentFile {
                    resource_id,
                    logical_path,
                } => {
                    let last_sha256 = tombstone.last_sha256.clone().ok_or_else(|| {
                        format!(
                            "project-content file tombstone '{}' lacks its last digest",
                            logical_path
                        )
                    })?;
                    let target = project_content_target(logical_path, binding)?;
                    let target_state = match inspect_restore_target(&target) {
                        Ok(state) => state,
                        Err(error) => {
                            actions.push(manual_action(
                                resource_id,
                                &format!(
                                    "Project-file deletion '{}' is blocked at '{}': {}",
                                    logical_path,
                                    target.display(),
                                    error
                                ),
                            )?);
                            continue;
                        }
                    };
                    if target_state.digest.is_none() {
                        continue;
                    }
                    let kind = RestoreActionKind::DeleteProjectFile {
                        logical_path: logical_path.clone(),
                        last_sha256: last_sha256.clone(),
                    };
                    actions.push(RestoreAction {
                        action_id: action_id_for(
                            resource_id,
                            logical_path.as_str(),
                            kind.action_type(),
                        )?,
                        resource_id: resource_id.clone(),
                        kind,
                        target_path: Some(path_text(&target)?),
                        source_sha256: None,
                        expected_target_sha256: Some(last_sha256),
                        expected_target_mode: target_state.mode,
                        materialized_sha256: None,
                        requires_explicit_approval: true,
                    });
                }
                TombstoneTarget::ProjectContentDirectory {
                    resource_id,
                    logical_path,
                } => {
                    let target = project_content_target(logical_path, binding)?;
                    let expected_target_mode = match inspect_project_directory_target(&target) {
                        Ok(Some(mode)) => Some(mode),
                        Ok(None) => continue,
                        Err(error) => {
                            actions.push(manual_action(
                                resource_id,
                                &format!(
                                    "Project-directory deletion '{}' is blocked at '{}': {}",
                                    logical_path,
                                    target.display(),
                                    error
                                ),
                            )?);
                            continue;
                        }
                    };
                    let kind = RestoreActionKind::DeleteProjectDirectory {
                        logical_path: logical_path.clone(),
                    };
                    actions.push(RestoreAction {
                        action_id: action_id_for(
                            resource_id,
                            logical_path.as_str(),
                            kind.action_type(),
                        )?,
                        resource_id: resource_id.clone(),
                        kind,
                        target_path: Some(path_text(&target)?),
                        source_sha256: None,
                        expected_target_sha256: None,
                        expected_target_mode,
                        materialized_sha256: None,
                        requires_explicit_approval: true,
                    });
                }
                _ => {}
            }
        }
        actions.sort_by(|a, b| a.action_id.cmp(&b.action_id));
        let plan = RestorePlan {
            schema_version: RESTORE_PLAN_SCHEMA,
            plan_id,
            storage_id: self.storage_id.clone(),
            bundle_id: snapshot.head.bundle_id.clone(),
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            expected_host_identity_revision,
            revision: snapshot.head.revision,
            commit_id: snapshot.head.commit_id.clone(),
            manifest_sha256: snapshot.head.manifest_sha256.clone(),
            binding_revision: binding.revision,
            created_at,
            expires_at: created_at.saturating_add(DEFAULT_PLAN_LIFETIME_SECS),
            actions,
            conversation_actions: Vec::new(),
            project_content_eligibility: None,
            // Continuity is a plan-phase input the command layer supplies;
            // the engine builds the plan from the snapshot alone.
            remote_continuity: None,
        };
        plan.validate()?;
        Ok(plan)
    }

    fn read_manifest(&self, head: &BundleHead) -> Result<BundleManifest, String> {
        head.validate()?;
        validate_bundle_relative_object_key(&head.manifest_key)?;
        let key = repository_object_key(&head.bundle_id, &head.manifest_key)?;
        let outcome = self.store.stage_verified(
            &key,
            &ExpectedObject {
                size: head.manifest_size,
                sha256: head.manifest_sha256.clone(),
            },
        )?;
        let body = match outcome {
            StageOutcome::Missing => return Err(format!("bundle manifest '{}' is missing", key)),
            StageOutcome::Mismatch => {
                return Err(format!("bundle manifest '{}' failed verification", key))
            }
            StageOutcome::Verified(body) => body,
        };
        let manifest: BundleManifest = parse_json_reader(body.open()?, "bundle manifest")?;
        manifest.validate_against_head(head)?;
        Ok(manifest)
    }

    fn publish_tag(&self, manifest: &BundleManifest) -> Result<(), String> {
        let tag = BundleTag {
            schema_version: manifest.schema_version,
            bundle_id: manifest.bundle.bundle_id.clone(),
            display_name: manifest.bundle.display_name.clone(),
            kind: manifest.bundle.kind,
            revision: manifest.revision,
            updated_at: manifest.updated_at,
            resources: manifest.resources.len() as u64,
            files: manifest.files.len() as u64,
        };
        let bytes = serde_json::to_vec(&tag).map_err(|e| format!("serialize bundle tag: {}", e))?;
        let key = repository_object_key(&manifest.bundle.bundle_id, TAG_FILE)?;
        for _ in 0..4 {
            let expectation = match self.store.get_metadata(&key, MAX_BUNDLE_TAG_BYTES)? {
                Some(existing) => CasExpectation::Match(existing.etag),
                None => CasExpectation::Absent,
            };
            if matches!(
                self.store.compare_and_swap(&key, &expectation, &bytes)?,
                CasOutcome::Written { .. }
            ) {
                return Ok(());
            }
        }
        Err("bundle tag CAS remained contended after publication".to_string())
    }

    fn validate_binding(
        &self,
        binding: &ProjectBinding,
        backup_root: Option<&Path>,
    ) -> Result<(), String> {
        binding.validate_structure()?;
        let project = canonical_real_dir(Path::new(&binding.project_root), "project root")?;
        let recorded = PathBuf::from(&binding.canonical_project_root);
        if project != recorded {
            return Err(format!(
                "project binding changed: '{}' now resolves to '{}'",
                binding.project_root,
                project.display()
            ));
        }
        let mut targets = vec![project];
        for (label, home) in [
            ("Codex home", binding.codex_home.as_deref()),
            ("Claude home", binding.claude_home.as_deref()),
        ] {
            if let Some(home) = home {
                targets.push(canonical_real_dir(Path::new(home), label)?);
            }
        }
        if let Some(store_root) = self.store.local_root() {
            for target in &targets {
                if paths_overlap(store_root, target) {
                    return Err(format!(
                        "local bundle store '{}' overlaps restore target '{}'",
                        store_root.display(),
                        target.display()
                    ));
                }
            }
        }
        if let Some(backup_root) = backup_root {
            let backup = canonical_or_prospective_dir(backup_root, "backup root")?;
            for target in &targets {
                if paths_overlap(&backup, target) {
                    return Err(format!(
                        "backup root '{}' overlaps restore target '{}'",
                        backup.display(),
                        target.display()
                    ));
                }
            }
            if self
                .store
                .local_root()
                .is_some_and(|store| paths_overlap(store, &backup))
            {
                return Err("backup root overlaps local bundle storage".to_string());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_sync::domain::{LocalProjectId, RecipeEntry, ReplicaId};
    use crate::project_sync::provider_capture::{capture_recipe, discover_project, CaptureRequest};

    fn bundle_id(number: u64) -> BundleId {
        BundleId::parse(format!("{:032x}", number)).unwrap()
    }

    fn storage_id() -> StorageId {
        StorageId::parse("storage-local").unwrap()
    }

    fn identity(id: BundleId, name: &str) -> BundleIdentity {
        BundleIdentity {
            bundle_id: id,
            display_name: name.to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        }
    }

    fn recipe_for(resource_ids: impl IntoIterator<Item = String>) -> BundleRecipe {
        let mut recipe = BundleRecipe::default();
        for resource_id in resource_ids {
            let resource_id = ResourceId::parse(resource_id).unwrap();
            recipe.entries.insert(
                resource_id.clone(),
                RecipeEntry {
                    resource_id,
                    apply_policy: ApplyPolicy::Merge,
                    required: false,
                },
            );
        }
        recipe
    }

    fn captured_with() -> CapturedWith {
        CapturedWith {
            app_version: "test".to_string(),
            codex_version: None,
            claude_version: None,
            codec_versions: BTreeMap::from([("codex".to_string(), 1), ("claude".to_string(), 1)]),
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn active_binding(
        id: BundleId,
        project_root: &Path,
        codex_home: Option<&Path>,
        claude_home: Option<&Path>,
    ) -> ProjectBinding {
        ProjectBinding {
            host_id: super::domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
            replica_id: ReplicaId::parse("replica-test").unwrap(),
            local_project_id: LocalProjectId::parse("project-test").unwrap(),
            bundle_id: id,
            project_root: project_root.to_str().unwrap().to_string(),
            canonical_project_root: fs::canonicalize(project_root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            profile_ids: BTreeMap::new(),
            codex_home: codex_home.map(|path| path.to_str().unwrap().to_string()),
            claude_home: claude_home.map(|path| path.to_str().unwrap().to_string()),
            state: BindingState::Active,
            revision: 3,
            updated_at: 1,
        }
    }

    #[test]
    fn local_store_enforces_immutable_objects_and_head_cas() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalBundleObjectStore::open(temp.path().join("store")).unwrap();
        let key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{}/_uploads/upload-a/files/project/AGENTS.md",
            bundle_id(1)
        ))
        .unwrap();
        assert_eq!(
            store.put_immutable(&key, b"one").unwrap(),
            ImmutablePutOutcome::Written
        );
        assert_eq!(
            store.put_immutable(&key, b"one").unwrap(),
            ImmutablePutOutcome::AlreadyPresent
        );
        assert!(store.put_immutable(&key, b"two").is_err());

        let head = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{}/_head.json",
            bundle_id(1)
        ))
        .unwrap();
        let first = store
            .compare_and_swap(&head, &CasExpectation::Absent, b"first")
            .unwrap();
        let CasOutcome::Written { etag } = first else {
            panic!("initial CAS did not write")
        };
        assert!(matches!(
            store
                .compare_and_swap(&head, &CasExpectation::Absent, b"lost")
                .unwrap(),
            CasOutcome::Conflict { .. }
        ));
        assert!(matches!(
            store
                .compare_and_swap(&head, &CasExpectation::Match(etag), b"second")
                .unwrap(),
            CasOutcome::Written { .. }
        ));
        assert_eq!(
            store
                .get_metadata(&head, MAX_BUNDLE_HEAD_BYTES)
                .unwrap()
                .unwrap()
                .bytes,
            b"second"
        );
    }

    #[test]
    fn verified_body_readers_have_independent_positions() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalBundleObjectStore::open(temp.path().join("store")).unwrap();
        let key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{}/_objects/{}",
            bundle_id(1),
            sha256(b"abcdef")
        ))
        .unwrap();
        store.put_immutable(&key, b"abcdef").unwrap();
        let outcome = store
            .stage_verified(
                &key,
                &ExpectedObject {
                    size: 6,
                    sha256: sha256(b"abcdef"),
                },
            )
            .unwrap();
        let StageOutcome::Verified(body) = outcome else {
            panic!("object was not staged")
        };

        let mut first = body.open().unwrap();
        let mut second = body.open().unwrap();
        let mut first_prefix = [0_u8; 2];
        let mut second_prefix = [0_u8; 2];
        first.read_exact(&mut first_prefix).unwrap();
        second.read_exact(&mut second_prefix).unwrap();

        assert_eq!(&first_prefix, b"ab");
        assert_eq!(&second_prefix, b"ab");
    }

    #[test]
    fn mallard_storage_marker_is_created_idempotently_and_validated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("store");
        let store = LocalBundleObjectStore::open(&root).unwrap();
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        drop(engine);

        let marker_path = root.join(STORAGE_MARKER_KEY);
        let marker: StorageMarker =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        assert_eq!(marker, StorageMarker::current());
        assert!(root.join(MALLARD_ROOT).join(LOCAL_STORAGE_LOCK).is_file());

        let store = LocalBundleObjectStore::open(&root).unwrap();
        BundleEngine::open(store, storage_id()).unwrap();

        fs::write(
            &marker_path,
            br#"{"format":"mallard-storage","layout_version":2}"#,
        )
        .unwrap();
        let store = LocalBundleObjectStore::open(&root).unwrap();
        let error = match BundleEngine::open(store, storage_id()) {
            Ok(_) => panic!("unsupported marker should be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("layout version 2"));
    }

    #[test]
    fn current_manifest_read_enforces_the_head_manifest_size() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("CLAUDE.md"), b"portable guidance");
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(91);
        let store_root = temp.path().join("store");
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(&store_root).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Sized manifest"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();

        let head_path = store_root
            .join(REPOSITORY_PREFIX)
            .join(id.as_str())
            .join(HEAD_FILE);
        let mut head: BundleHead = serde_json::from_slice(&fs::read(&head_path).unwrap()).unwrap();
        head.manifest_size += 1;
        fs::write(&head_path, serde_json::to_vec(&head).unwrap()).unwrap();

        let error = engine.inspect(&id).unwrap_err();
        assert!(error.contains("failed verification"), "{error}");
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
    fn publish_fetch_update_and_tombstone_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("CLAUDE.md"), b"version one");
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let resource_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "CLAUDE.md")
            .unwrap()
            .resource_id
            .clone();
        let recipe = recipe_for([resource_id]);
        let captured = capture_recipe(&request, &recipe).unwrap();
        let id = bundle_id(2);
        let store_root = temp.path().join("store");
        let store = LocalBundleObjectStore::open(&store_root).unwrap();
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let first = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured,
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        assert_eq!(first.snapshot.head.revision, 1);
        let first_token = engine.read_head(&id).unwrap().unwrap().1;
        let repository_root = store_root.join(format!("{}/{}", REPOSITORY_PREFIX, id));
        assert!(store_root.join(STORAGE_MARKER_KEY).is_file());
        assert!(repository_root.join(HEAD_FILE).is_file());
        assert!(repository_root.join(TAG_FILE).is_file());
        assert!(repository_root.join("_manifests").is_dir());
        assert!(repository_root.join("_commits").is_dir());
        assert!(repository_root.join("_objects").is_dir());
        assert_eq!(
            fs::metadata(repository_root.join(&first.snapshot.head.manifest_key))
                .unwrap()
                .len(),
            first.snapshot.head.manifest_size
        );
        let snapshot = engine.inspect(&id).unwrap();
        let path = snapshot.manifest.files.keys().next().unwrap().clone();
        let cache = Arc::new(ObjectCache::default());
        let mut bytes = engine.byte_source(&snapshot, Some(cache.clone()), None);
        assert_eq!(
            bytes.read_bounded(&path, 1024, "test payload").unwrap(),
            b"version one"
        );
        drop(bytes);

        let mut wrong_size = snapshot.clone();
        wrong_size.manifest.files.get_mut(&path).unwrap().size += 1;
        let mut bytes = engine.byte_source(&wrong_size, Some(cache), None);
        let error = bytes.body(&path).unwrap_err();
        assert!(
            error.contains("does not match manifest size/hash"),
            "{error}"
        );

        write(&project.join("CLAUDE.md"), b"version two");
        let captured = capture_recipe(&request, &recipe).unwrap();
        let second = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Renamed Project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured,
                observed_head: Some((first.snapshot.head.clone(), first_token.clone())),
                updated_at: 20,
            })
            .unwrap();
        assert_eq!(second.snapshot.head.revision, 2);
        let second_token = engine.read_head(&id).unwrap().unwrap().1;
        assert!(engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Stale"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: Some((first.snapshot.head.clone(), first_token)),
                updated_at: 21,
            })
            .is_err());

        let empty_recipe = BundleRecipe::default();
        let removed = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Renamed Project"),
                recipe: empty_recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &empty_recipe).unwrap(),
                observed_head: Some((second.snapshot.head.clone(), second_token)),
                updated_at: 30,
            })
            .unwrap();
        assert!(removed.snapshot.manifest.files.is_empty());
        assert!(removed.snapshot.manifest.resources.is_empty());
        assert!(removed
            .snapshot
            .manifest
            .tombstones
            .values()
            .any(|tombstone| matches!(tombstone.target, TombstoneTarget::Resource { .. })));
    }

    #[test]
    fn remote_bundle_listing_pages_more_than_one_thousand_bundles() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalBundleObjectStore::open(temp.path().join("store")).unwrap();
        for number in 1..=1_005_u64 {
            let id = bundle_id(number);
            let tag = BundleTag {
                schema_version: BUNDLE_SCHEMA,
                bundle_id: id.clone(),
                display_name: format!("Bundle {}", number),
                kind: BundleKind::Project,
                revision: 1,
                updated_at: number,
                resources: 1,
                files: 1,
            };
            let key = repository_object_key(&id, TAG_FILE).unwrap();
            store
                .compare_and_swap(
                    &key,
                    &CasExpectation::Absent,
                    &serde_json::to_vec(&tag).unwrap(),
                )
                .unwrap();
        }
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let mut cursor = None;
        let mut listed = Vec::new();
        loop {
            let page = engine.list_remote_bundles(cursor.as_deref(), 113).unwrap();
            listed.extend(page.bundles.into_iter().map(|bundle| bundle.bundle_id));
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        assert_eq!(listed.len(), 1_005);
        assert!(listed.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn restore_plan_remaps_project_files_and_apply_creates_backup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        let target = temp.path().join("target-project");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write(&source.join("CLAUDE.md"), b"portable guidance");
        write(&target.join("CLAUDE.md"), b"target edits");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let resource_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "CLAUDE.md")
            .unwrap()
            .resource_id
            .clone();
        let recipe = recipe_for([resource_id]);
        let id = bundle_id(3);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Portable project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        let snapshot = engine.inspect(&id).unwrap();
        let binding = active_binding(id, &target, None, None);
        let plan = engine
            .build_restore_plan_from_snapshot(&snapshot, &binding, 100, 0)
            .unwrap();
        let write_action = plan
            .actions
            .iter()
            .find(|action| {
                action
                    .target_path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("CLAUDE.md"))
            })
            .unwrap();
        assert!(write_action.requires_explicit_approval);
        let approved = BTreeSet::from([write_action.action_id.clone()]);
        let mut bytes = engine.byte_source(&snapshot, None, None);
        let result = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut bytes,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(
            fs::read(target.join("CLAUDE.md")).unwrap(),
            b"portable guidance"
        );
        assert_eq!(result.backups.len(), 1);
        assert_eq!(
            fs::read(&result.backups[0].backup_path).unwrap(),
            b"target edits"
        );
        assert!(result
            .receipts
            .iter()
            .any(|receipt| receipt.status == ActionStatus::Applied));
    }

    /// Store wrapper recording every staged/metadata key and every write so
    /// tests can prove which payload objects an operation actually downloaded
    /// or uploaded. The trait-default `put_many` routes through
    /// `put_immutable`, so bulk uploads are recorded per object.
    #[derive(Clone)]
    struct CountingStore {
        inner: LocalBundleObjectStore,
        keys: Arc<Mutex<Vec<String>>>,
        puts: Arc<Mutex<Vec<String>>>,
        cas_writes: Arc<Mutex<Vec<String>>>,
    }

    impl CountingStore {
        fn open(root: &Path) -> Self {
            Self {
                inner: LocalBundleObjectStore::open(root).unwrap(),
                keys: Arc::new(Mutex::new(Vec::new())),
                puts: Arc::new(Mutex::new(Vec::new())),
                cas_writes: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn reset(&self) {
            self.keys.lock().unwrap().clear();
            self.puts.lock().unwrap().clear();
            self.cas_writes.lock().unwrap().clear();
        }

        /// Immutable payload writes under the content-addressed namespace.
        fn payload_puts(&self) -> Vec<String> {
            self.puts
                .lock()
                .unwrap()
                .iter()
                .filter(|key| key.contains("/_objects/"))
                .cloned()
                .collect()
        }

        fn put_count(&self) -> usize {
            self.puts.lock().unwrap().len()
        }

        fn cas_count(&self) -> usize {
            self.cas_writes.lock().unwrap().len()
        }

        /// Repository object reads that are payloads — heads, tags, and
        /// manifests are metadata and excluded.
        fn payload_gets(&self) -> usize {
            self.keys
                .lock()
                .unwrap()
                .iter()
                .filter(|key| {
                    key.contains("/repositories/")
                        && !key.ends_with("/_head.json")
                        && !key.ends_with("/_tag.json")
                        && !key.contains("/_manifests/")
                })
                .count()
        }
    }

    impl BundleObjectStore for CountingStore {
        fn get_metadata(
            &self,
            key: &ObjectKey,
            max_bytes: u64,
        ) -> Result<Option<StoredMetadata>, String> {
            self.keys.lock().unwrap().push(key.as_str().to_string());
            self.inner.get_metadata(key, max_bytes)
        }

        fn stage_verified(
            &self,
            key: &ObjectKey,
            expected: &ExpectedObject,
        ) -> Result<StageOutcome, String> {
            self.keys.lock().unwrap().push(key.as_str().to_string());
            self.inner.stage_verified(key, expected)
        }

        fn stage_many_verified(
            &self,
            requests: &[(ObjectKey, ExpectedObject)],
        ) -> Result<Vec<StageOutcome>, String> {
            let mut recorded = self.keys.lock().unwrap();
            for (key, _) in requests {
                recorded.push(key.as_str().to_string());
            }
            drop(recorded);
            self.inner.stage_many_verified(requests)
        }

        fn put_immutable(
            &self,
            key: &ObjectKey,
            bytes: &[u8],
        ) -> Result<ImmutablePutOutcome, String> {
            self.puts.lock().unwrap().push(key.as_str().to_string());
            self.inner.put_immutable(key, bytes)
        }

        fn compare_and_swap(
            &self,
            key: &ObjectKey,
            expectation: &CasExpectation,
            bytes: &[u8],
        ) -> Result<CasOutcome, String> {
            self.cas_writes
                .lock()
                .unwrap()
                .push(key.as_str().to_string());
            self.inner.compare_and_swap(key, expectation, bytes)
        }

        fn list(
            &self,
            prefix: &ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<StoreListPage, String> {
            self.inner.list(prefix, cursor, limit)
        }

        fn local_root(&self) -> Option<&Path> {
            self.inner.local_root()
        }
    }

    #[test]
    fn publish_skips_payloads_already_in_previous_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("stable.md"), b"stable contents");
        write(&source.join("changing.md"), b"first draft");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(41);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let first = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Incremental"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        assert!(first.published);
        let first_token = engine.read_head(&id).unwrap().unwrap().1;
        assert_eq!(store.payload_puts().len(), 2);
        for entry in first.snapshot.manifest.files.values() {
            assert_eq!(entry.object_key, format!("_objects/{}", entry.sha256));
        }

        write(&source.join("changing.md"), b"second draft");
        store.reset();
        let second = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Incremental"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: Some((first.snapshot.head.clone(), first_token)),
                updated_at: 20,
            })
            .unwrap();
        assert!(second.published);
        assert_eq!(second.snapshot.head.revision, 2);
        let changed_key =
            repository_object_key(&id, &format!("_objects/{}", sha256(b"second draft"))).unwrap();
        assert_eq!(
            store.payload_puts(),
            vec![changed_key.as_str().to_string()],
            "only the changed payload uploads"
        );
        assert_eq!(
            store.payload_gets(),
            0,
            "publish downloads nothing — retained entries are witnessed by the previous \
             manifest and the checksummed PUT is the upload receipt"
        );
        let stable_path = second
            .snapshot
            .manifest
            .files
            .iter()
            .find(|(_, entry)| entry.sha256 == sha256(b"stable contents"))
            .map(|(path, _)| path.clone())
            .unwrap();
        assert_eq!(
            second.snapshot.manifest.files[&stable_path],
            first.snapshot.manifest.files[&stable_path],
            "the unchanged entry carries over verbatim, object key included"
        );
    }

    /// The continuity probe answers from the store on every call — a local
    /// cache hit would witness nothing about a wiped remote
    /// (DESIGN_REMOTE_HISTORY_CONTINUITY.md, warm-cache tripwire).
    #[test]
    fn manifest_presence_probe_reads_the_store_every_time() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("stable.md"), b"stable contents");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(77);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let published = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Probe"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        let head = published.snapshot.head.clone();
        let manifest_key_reads = |store: &CountingStore| {
            store
                .keys
                .lock()
                .unwrap()
                .iter()
                .filter(|key| key.contains("/_manifests/"))
                .count()
        };

        store.reset();
        let hit = engine
            .fetch_manifest_version_optional(
                &id,
                head.revision,
                &head.commit_id,
                head.manifest_size,
                &head.manifest_sha256,
            )
            .unwrap();
        assert_eq!(hit.map(|manifest| manifest.revision), Some(1));
        assert_eq!(manifest_key_reads(&store), 1);

        // A second identical call reads the store again: presence answers
        // are always live, never remembered.
        assert!(engine
            .fetch_manifest_version_optional(
                &id,
                head.revision,
                &head.commit_id,
                head.manifest_size,
                &head.manifest_sha256,
            )
            .unwrap()
            .is_some());
        assert_eq!(manifest_key_reads(&store), 2);

        // Bytes that no longer reproduce the recorded digest are absence,
        // not an error — row 8 treats them exactly like a missing key.
        assert!(engine
            .fetch_manifest_version_optional(
                &id,
                head.revision,
                &head.commit_id,
                head.manifest_size,
                &"f".repeat(64),
            )
            .unwrap()
            .is_none());
        assert!(engine
            .fetch_manifest_version_optional(
                &id,
                99,
                "commit-nowhere",
                head.manifest_size,
                &head.manifest_sha256,
            )
            .unwrap()
            .is_none());

        // A wiped repository answers absent although nothing local changed:
        // the verdict comes from the store alone.
        fs::remove_dir_all(
            store
                .local_root()
                .unwrap()
                .join(REPOSITORY_PREFIX)
                .join(id.as_str()),
        )
        .unwrap();
        assert!(engine
            .fetch_manifest_version_optional(
                &id,
                head.revision,
                &head.commit_id,
                head.manifest_size,
                &head.manifest_sha256,
            )
            .unwrap()
            .is_none());
    }

    #[test]
    fn publish_dedupes_identical_bytes_within_one_push() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("left.md"), b"same bytes either way");
        write(&source.join("right.md"), b"same bytes either way");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(42);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let published = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Dedupe"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        assert_eq!(store.payload_puts().len(), 1);
        let shared_key = format!("_objects/{}", sha256(b"same bytes either way"));
        let file_entries: Vec<_> = published.snapshot.manifest.files.values().collect();
        assert_eq!(file_entries.len(), 2);
        for entry in file_entries {
            assert_eq!(entry.object_key, shared_key);
        }
    }

    #[test]
    fn publish_semantic_noop_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("CLAUDE.md"), b"steady state");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(43);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let first = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Noop"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        assert!(first.published);
        let first_token = engine.read_head(&id).unwrap().unwrap().1;

        store.reset();
        let noop = engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Noop"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: Some((first.snapshot.head.clone(), first_token.clone())),
                updated_at: 20,
            })
            .unwrap();
        assert!(!noop.published);
        assert_eq!(noop.snapshot.head.revision, 1);
        assert_eq!(engine.read_head(&id).unwrap().unwrap().1, first_token);
        assert_eq!(store.put_count(), 0, "a semantic no-op writes no objects");
        assert_eq!(
            store.cas_count(),
            0,
            "a semantic no-op never touches the head"
        );
        assert_eq!(engine.inspect(&id).unwrap().head.revision, 1);
    }

    #[test]
    fn publish_reports_upload_progress_for_pending_objects_only() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("small.md"), b"tiny");
        write(&source.join("large.md"), b"a considerably longer payload");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(44);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let mut events: Vec<FetchProgress> = Vec::new();
        let first = engine
            .publish_test_resources_with_progress(
                CapturedResourcesPublishRequest {
                    identity: identity(id.clone(), "Progress"),
                    recipe: recipe.clone(),
                    captured_with: captured_with(),
                    captured: capture_recipe(&request, &recipe).unwrap(),
                    observed_head: None,
                    updated_at: 10,
                },
                &mut |progress| events.push(progress),
            )
            .unwrap();
        let first_token = engine.read_head(&id).unwrap().unwrap().1;
        let full_bytes = (b"tiny".len() + b"a considerably longer payload".len()) as u64;
        assert_eq!(events.len(), 2);
        assert_eq!(events.last().unwrap().index, 2);
        assert_eq!(events.last().unwrap().total_objects, 2);
        assert_eq!(events.last().unwrap().fetched_bytes, full_bytes);
        assert_eq!(events.last().unwrap().total_bytes, full_bytes);

        events.clear();
        let noop = engine
            .publish_test_resources_with_progress(
                CapturedResourcesPublishRequest {
                    identity: identity(id.clone(), "Progress"),
                    recipe: recipe.clone(),
                    captured_with: captured_with(),
                    captured: capture_recipe(&request, &recipe).unwrap(),
                    observed_head: Some((first.snapshot.head.clone(), first_token.clone())),
                    updated_at: 20,
                },
                &mut |progress| events.push(progress),
            )
            .unwrap();
        assert!(!noop.published);
        assert!(events.is_empty(), "a no-op publish reports no transfer");

        write(
            &source.join("large.md"),
            b"a different considerably longer payload",
        );
        events.clear();
        engine
            .publish_test_resources_with_progress(
                CapturedResourcesPublishRequest {
                    identity: identity(id.clone(), "Progress"),
                    recipe: recipe.clone(),
                    captured_with: captured_with(),
                    captured: capture_recipe(&request, &recipe).unwrap(),
                    observed_head: Some((first.snapshot.head.clone(), first_token)),
                    updated_at: 30,
                },
                &mut |progress| events.push(progress),
            )
            .unwrap();
        let changed_bytes = b"a different considerably longer payload".len() as u64;
        assert_eq!(events.len(), 1, "unchanged payloads never enter the totals");
        assert_eq!(events[0].total_objects, 1);
        assert_eq!(events[0].total_bytes, changed_bytes);
        assert_eq!(events[0].fetched_bytes, changed_bytes);
    }

    #[test]
    fn manifest_plan_and_lazy_apply_touch_only_needed_payload_objects() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        let target = temp.path().join("target-project");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write(&source.join("CLAUDE.md"), b"portable guidance");
        // The target already holds the exact bytes this plan would write.
        write(&target.join("CLAUDE.md"), b"portable guidance");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(&request).unwrap();
        let resource_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "CLAUDE.md")
            .unwrap()
            .resource_id
            .clone();
        let recipe = recipe_for([resource_id]);
        let id = bundle_id(97);
        let store_root = temp.path().join("store");
        let store = CountingStore::open(&store_root);
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Lazy project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        let binding = active_binding(id.clone(), &target, None, None);

        store.reset();
        let snapshot = engine.inspect(&id).unwrap();
        let logical_path = snapshot.manifest.files.keys().next().unwrap();
        let entry_size = snapshot.manifest.files[logical_path].size;
        let mut limited_bytes = engine.byte_source(&snapshot, None, None);
        let error = limited_bytes
            .read_bounded(
                logical_path,
                entry_size.saturating_sub(1),
                "bounded test payload",
            )
            .unwrap_err();
        assert!(error.contains("exceeds its read limit"), "{error}");
        assert_eq!(
            store.payload_gets(),
            0,
            "resource limits must reject before downloading an object"
        );
        let plan = engine
            .build_restore_plan_from_snapshot(&snapshot, &binding, 100, 0)
            .unwrap();
        assert_eq!(
            store.payload_gets(),
            0,
            "planning must not download payload objects"
        );
        let write_action = plan
            .actions
            .iter()
            .find(|action| {
                action
                    .target_path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("CLAUDE.md"))
            })
            .unwrap();
        assert_eq!(write_action.materialized_sha256, write_action.source_sha256);
        assert_eq!(
            write_action.materialized_sha256,
            write_action.expected_target_sha256
        );

        // In-sync apply: the pinned no-op completes without any payload.
        let approved = BTreeSet::from([write_action.action_id.clone()]);
        store.reset();
        let mut apply_bytes = engine.byte_source(&snapshot, None, None);
        let result = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut apply_bytes,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(result
            .receipts
            .iter()
            .any(|receipt| receipt.status == ActionStatus::Applied));
        assert_eq!(
            store.payload_gets(),
            0,
            "an in-sync apply must not download payload objects"
        );

        // A missing target is genuinely new work: exactly one object moves.
        fs::remove_file(target.join("CLAUDE.md")).unwrap();
        let snapshot = engine.inspect(&id).unwrap();
        let plan = engine
            .build_restore_plan_from_snapshot(&snapshot, &binding, 102, 0)
            .unwrap();
        let write_action = plan
            .actions
            .iter()
            .find(|action| {
                action
                    .target_path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("CLAUDE.md"))
            })
            .unwrap();
        let approved = BTreeSet::from([write_action.action_id.clone()]);
        store.reset();
        let mut apply_bytes = engine.byte_source(&snapshot, None, None);
        engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut apply_bytes,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                103,
            )
            .unwrap();
        assert_eq!(
            store.payload_gets(),
            1,
            "apply downloads exactly the objects it writes"
        );
        assert_eq!(
            fs::read(target.join("CLAUDE.md")).unwrap(),
            b"portable guidance"
        );

        let head_path = store_root
            .join(REPOSITORY_PREFIX)
            .join(id.as_str())
            .join(HEAD_FILE);
        let mut changed_head: BundleHead =
            serde_json::from_slice(&fs::read(&head_path).unwrap()).unwrap();
        changed_head.manifest_size += 1;
        fs::write(&head_path, serde_json::to_vec(&changed_head).unwrap()).unwrap();
        let mut apply_bytes = engine.byte_source(&snapshot, None, None);
        let error = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut apply_bytes,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                104,
            )
            .unwrap_err();
        assert!(
            error.contains("bundle head changed after restore planning"),
            "{error}"
        );
    }

    #[test]
    fn restore_plan_skips_generic_files_already_matching_the_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let source_claude = temp.path().join("source-claude");
        let target_claude = temp.path().join("target-claude");
        for directory in [&source, &target, &source_claude, &target_claude] {
            fs::create_dir_all(directory).unwrap();
        }
        // Memories are only discovered from a bucket a session has proven to
        // belong to the project, so the fixture needs one transcript beside
        // the memory directory.
        let transcript = format!(
            "{{\"type\":\"system\",\"sessionId\":\"session-m\",\"cwd\":{}}}\n",
            serde_json::to_string(source.to_str().unwrap()).unwrap()
        );
        write(
            &source_claude.join("projects/memory-bucket/session-m.jsonl"),
            transcript.as_bytes(),
        );
        write(
            &source_claude.join("projects/memory-bucket/memory/MEMORY.md"),
            b"remember the ducks",
        );
        let request = CaptureRequest {
            project_root: source.clone(),
            codex_home: None,
            claude_home: Some(source_claude),
            excluded_project_roots: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let resource = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id.starts_with("claude:memory:"))
            .unwrap();
        let recipe = recipe_for([resource.resource_id.clone()]);
        let id = bundle_id(98);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Memory project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        let snapshot = engine.inspect(&id).unwrap();
        let binding = active_binding(id, &target, None, Some(&target_claude));

        let is_generic_write = |action: &RestoreAction| {
            matches!(
                action.kind,
                RestoreActionKind::WriteFile { .. } | RestoreActionKind::MergeFile { .. }
            )
        };
        let plan = engine
            .build_restore_plan_from_snapshot(&snapshot, &binding, 100, 0)
            .unwrap();
        let action = plan
            .actions
            .iter()
            .find(|action| is_generic_write(action))
            .unwrap();
        let memory_target = PathBuf::from(action.target_path.clone().unwrap());

        // The target already holds the manifest bytes: the plan stops asking,
        // so an all-deselected apply cannot strand the reviewed baseline
        // behind a write that would change nothing.
        write(&memory_target, b"remember the ducks");
        let plan = engine
            .build_restore_plan_from_snapshot(&snapshot, &binding, 101, 0)
            .unwrap();
        assert!(
            !plan.actions.iter().any(|action| is_generic_write(action)),
            "identical generic files must not re-enter review: {:?}",
            plan.actions
        );

        // A drifted target is real work again.
        write(&memory_target, b"remember the geese");
        let plan = engine
            .build_restore_plan_from_snapshot(&snapshot, &binding, 102, 0)
            .unwrap();
        assert!(plan.actions.iter().any(|action| is_generic_write(action)));
    }

    #[test]
    fn claude_session_materializes_under_remapped_relative_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let source_child = source.join("apps/web");
        let target = temp.path().join("target");
        let target_child = target.join("apps/web");
        let source_claude = temp.path().join("source-claude");
        let target_claude = temp.path().join("target-claude");
        fs::create_dir_all(&source_child).unwrap();
        fs::create_dir_all(&target_child).unwrap();
        fs::create_dir_all(&source_claude).unwrap();
        fs::create_dir_all(&target_claude).unwrap();
        let transcript = format!(
            "{{\"type\":\"system\",\"sessionId\":\"session-a\",\"cwd\":{}}}\n",
            serde_json::to_string(source_child.to_str().unwrap()).unwrap()
        );
        write(
            &source_claude.join("projects/source-bucket/session-a.jsonl"),
            transcript.as_bytes(),
        );
        let request = CaptureRequest {
            project_root: source,
            codex_home: None,
            claude_home: Some(source_claude),
            excluded_project_roots: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let resource = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "claude:session:session-a")
            .unwrap();
        assert_eq!(resource.relative_cwd.as_deref(), Some("apps/web"));
        let recipe = recipe_for([resource.resource_id.clone()]);
        let id = bundle_id(4);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        engine
            .publish_test_resources(CapturedResourcesPublishRequest {
                identity: identity(id.clone(), "Claude project"),
                recipe: recipe.clone(),
                captured_with: captured_with(),
                captured: capture_recipe(&request, &recipe).unwrap(),
                observed_head: None,
                updated_at: 10,
            })
            .unwrap();
        let snapshot = engine.inspect(&id).unwrap();
        let binding = active_binding(id, &target, None, Some(&target_claude));
        let plan = engine
            .build_restore_plan_from_snapshot(&snapshot, &binding, 100, 0)
            .unwrap();
        let action = plan
            .actions
            .iter()
            .find(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::MaterializeConversation { .. }
                )
            })
            .unwrap();
        let encoded_target = encode_claude_project_path(target_child.to_str().unwrap());
        assert!(action
            .target_path
            .as_deref()
            .unwrap()
            .contains(&encoded_target));
        let approved = BTreeSet::from([action.action_id.clone()]);
        let mut bytes = engine.byte_source(&snapshot, None, None);
        engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut bytes,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(
            fs::read(action.target_path.as_ref().unwrap()).unwrap(),
            transcript.as_bytes()
        );
    }
}

fn manual_action(resource_id: &ResourceId, message: &str) -> Result<RestoreAction, String> {
    let suffix = &sha256(message.as_bytes())[..24];
    let action = RestoreAction {
        action_id: ActionId::parse(format!("manual-{}", suffix))?,
        resource_id: resource_id.clone(),
        kind: RestoreActionKind::Manual {
            message: message.to_string(),
        },
        target_path: None,
        source_sha256: None,
        expected_target_sha256: None,
        expected_target_mode: None,
        materialized_sha256: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
}

fn conversation_keep_local_action(
    resource_id: &ResourceId,
    logical_path: &LogicalPath,
    target: &Path,
    source_sha256: &str,
    target_sha256: &str,
) -> Result<RestoreAction, String> {
    let message = format!(
        "Conversation '{}' differs at '{}'; this Pull will keep the local version",
        logical_path,
        target.display()
    );
    let suffix = &sha256(message.as_bytes())[..24];
    let action = RestoreAction {
        action_id: ActionId::parse(format!("manual-{}", suffix))?,
        resource_id: resource_id.clone(),
        kind: RestoreActionKind::Manual { message },
        target_path: Some(path_text(target)?),
        source_sha256: Some(source_sha256.to_string()),
        expected_target_sha256: Some(target_sha256.to_string()),
        expected_target_mode: None,
        materialized_sha256: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
}

fn is_conversation_keep_local_action(action: &RestoreAction) -> bool {
    matches!(action.kind, RestoreActionKind::Manual { .. })
        && action.resource_id.as_str().starts_with("claude:session:")
        && action.target_path.is_some()
        && action.source_sha256.is_some()
        && action.expected_target_sha256.is_some()
}

fn continuation_action(
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<RestoreAction, String> {
    let target_cwd = remapped_resource_cwd(descriptor, binding)?;
    let command = match descriptor.kind {
        ResourceKind::CodexConversation => format!(
            "codex resume {} -C {}",
            descriptor.display_name,
            target_cwd.display()
        ),
        ResourceKind::ClaudeConversation => format!(
            "cd {} && claude --resume {}",
            target_cwd.display(),
            descriptor.display_name
        ),
        _ => return Err("continuation action requires a conversation resource".to_string()),
    };
    manual_action(&descriptor.resource_id, &command)
}

fn remapped_resource_cwd(
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<PathBuf, String> {
    let relative_cwd = descriptor.relative_cwd.as_deref().unwrap_or(".");
    if relative_cwd == "." {
        Ok(PathBuf::from(&binding.project_root))
    } else {
        safe_lexical_join(Path::new(&binding.project_root), relative_cwd)
    }
}

fn restore_kind_for_file(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    entry: &BundleFileEntry,
) -> Result<RestoreActionKind, String> {
    Ok(match descriptor.kind {
        ResourceKind::CodexConversation => RestoreActionKind::MaterializeConversation {
            provider: Provider::Codex,
            logical_path: logical_path.clone(),
        },
        ResourceKind::ClaudeConversation => RestoreActionKind::MaterializeConversation {
            provider: Provider::Claude,
            logical_path: logical_path.clone(),
        },
        ResourceKind::ProjectContentFile => RestoreActionKind::WriteProjectFile {
            logical_path: logical_path.clone(),
            mode: entry.mode.unwrap_or(0o600) & 0o777,
            source_mtime: entry.source_mtime,
        },
        _ if descriptor.apply_policy == ApplyPolicy::Merge => RestoreActionKind::MergeFile {
            logical_path: logical_path.clone(),
        },
        _ => RestoreActionKind::WriteFile {
            logical_path: logical_path.clone(),
        },
    })
}

fn action_needs_explicit_approval(
    descriptor: &ResourceDescriptor,
    target_digest: Option<&str>,
    source_digest: &str,
) -> bool {
    matches!(
        descriptor.apply_policy,
        ApplyPolicy::ExplicitReview | ApplyPolicy::ManualOnly | ApplyPolicy::Merge
    ) || target_digest.is_some_and(|target| target != source_digest)
}

fn action_id_for(
    resource_id: &ResourceId,
    logical_path: &str,
    action_type: domain::RestoreActionType,
) -> Result<ActionId, String> {
    let material = format!("{}\0{}\0{:?}", resource_id, logical_path, action_type);
    ActionId::parse(format!("action-{}", &sha256(material.as_bytes())[..24]))
}

fn map_logical_target(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<Option<PathBuf>, String> {
    let logical = logical_path.as_str();
    if let Some(relative) = logical.strip_prefix("project/") {
        return Ok(Some(safe_lexical_join(
            Path::new(&binding.canonical_project_root),
            relative,
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/codex/skills/") {
        let home = canonical_bound_home(binding.codex_home.as_deref(), "Codex")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("skills/{}", relative),
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/claude/skills/") {
        let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("skills/{}", relative),
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/claude/projects/") {
        let (_, file_relative) = relative
            .split_once('/')
            .ok_or_else(|| format!("invalid Claude project logical path '{}'", logical))?;
        let relative_cwd = descriptor.relative_cwd.as_deref().unwrap_or(".");
        let cwd = if relative_cwd == "." {
            PathBuf::from(&binding.project_root)
        } else {
            safe_lexical_join(Path::new(&binding.project_root), relative_cwd)?
        };
        let bucket = encode_claude_project_path(&path_text(&cwd)?);
        let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("projects/{}/{}", bucket, file_relative),
        )?));
    }
    for directory in ["file-history", "todos"] {
        if let Some(relative) = logical.strip_prefix(&format!("state/claude/{}/", directory)) {
            let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
            return Ok(Some(safe_lexical_join(
                &home,
                &format!("{}/{}", directory, relative),
            )?));
        }
    }
    if let Some(relative) = logical.strip_prefix("state/claude/memory/") {
        let (_, memory_relative) = relative
            .split_once('/')
            .ok_or_else(|| format!("invalid Claude memory path '{}'", logical))?;
        let relative_cwd = descriptor.relative_cwd.as_deref().unwrap_or(".");
        let cwd = if relative_cwd == "." {
            PathBuf::from(&binding.project_root)
        } else {
            safe_lexical_join(Path::new(&binding.project_root), relative_cwd)?
        };
        let bucket = encode_claude_project_path(&path_text(&cwd)?);
        let home = canonical_bound_home(binding.claude_home.as_deref(), "Claude")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("projects/{}/memory/{}", bucket, memory_relative),
        )?));
    }
    Ok(None)
}

fn project_content_target(
    logical_path: &LogicalPath,
    binding: &ProjectBinding,
) -> Result<PathBuf, String> {
    let relative = logical_path
        .as_str()
        .strip_prefix("project/")
        .ok_or_else(|| format!("'{}' is not project content", logical_path))?;
    safe_lexical_join(Path::new(&binding.canonical_project_root), relative)
}

fn directory_entry_digest(logical_path: &LogicalPath, mode: u32) -> String {
    sha256(format!("dir\0{}\0{:03o}", logical_path, mode & 0o777).as_bytes())
}

fn inspect_project_directory_target(path: &Path) -> Result<Option<u32>, String> {
    inspect_existing_ancestors_no_symlink(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "inspect project directory '{}': {}",
                path.display(),
                error
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "target '{}' is not a real no-follow directory",
            path.display()
        ));
    }
    Ok(Some(safe_mode(&metadata)))
}

fn project_action_logical_path(action: &RestoreAction) -> Option<LogicalPath> {
    match &action.kind {
        RestoreActionKind::WriteProjectFile { logical_path, .. }
        | RestoreActionKind::EnsureProjectDirectory { logical_path, .. }
        | RestoreActionKind::DeleteProjectFile { logical_path, .. }
        | RestoreActionKind::DeleteProjectDirectory { logical_path } => Some(logical_path.clone()),
        _ => None,
    }
}

fn project_action_depth(action: &RestoreAction) -> usize {
    project_action_logical_path(action)
        .map(|path| path.as_str().split('/').count())
        .unwrap_or(0)
}

fn logical_path_is_ancestor(directory: &LogicalPath, child: &LogicalPath) -> bool {
    child
        .as_str()
        .strip_prefix(directory.as_str())
        .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(unix)]
fn set_path_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))
        .map_err(|error| format!("set mode '{}': {}", path.display(), error))
}

#[cfg(not(unix))]
fn set_path_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

fn set_path_mtime_best_effort(path: &Path, source_mtime: u64) {
    if source_mtime == 0 {
        return;
    }
    if let Ok(file) = fs::OpenOptions::new().read(true).open(path) {
        let _ = file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(source_mtime));
    }
}

fn required_home<'a>(home: Option<&'a str>, provider: &str) -> Result<&'a Path, String> {
    home.map(Path::new)
        .ok_or_else(|| format!("{} home is required by this bundle", provider))
}

fn canonical_bound_home(home: Option<&str>, provider: &str) -> Result<PathBuf, String> {
    canonical_real_dir(
        required_home(home, provider)?,
        &format!("{} home", provider),
    )
}

fn encode_claude_project_path(path: &str) -> String {
    path.encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => byte as char,
            _ => '-',
        })
        .collect()
}

fn safe_lexical_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !root.is_absolute() || relative.is_empty() || relative.contains('\\') {
        return Err("unsafe restore path".to_string());
    }
    let mut result = root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(value) => result.push(value),
            _ => return Err(format!("unsafe restore-relative path '{}'", relative)),
        }
    }
    Ok(result)
}

#[derive(Debug)]
struct RestoreTargetState {
    digest: Option<String>,
    size: Option<u64>,
    mode: Option<u32>,
}

/// Inspect a restore target without materializing it in memory.
fn inspect_restore_target(path: &Path) -> Result<RestoreTargetState, String> {
    inspect_existing_ancestors_no_symlink(path)?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RestoreTargetState {
                digest: None,
                size: None,
                mode: None,
            })
        }
        Err(e) => {
            return Err(format!(
                "inspect restore target '{}': {}",
                path.display(),
                e
            ))
        }
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!(
            "restore target '{}' is not a regular no-follow file",
            path.display()
        ));
    }
    let mut file = open_read_only_no_follow(path, &format!("restore target '{}'", path.display()))?;
    let opened_meta = file
        .metadata()
        .map_err(|e| format!("inspect opened restore target '{}': {}", path.display(), e))?;
    if !opened_meta.is_file() || opened_meta.len() != meta.len() {
        return Err(format!(
            "restore target '{}' changed while opening",
            path.display()
        ));
    }
    let digest = sha256_reader(&mut file, &format!("restore target '{}'", path.display()))?;
    Ok(RestoreTargetState {
        digest: Some(digest),
        size: Some(opened_meta.len()),
        mode: Some(safe_mode(&opened_meta)),
    })
}

#[cfg(unix)]
fn safe_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn safe_mode(_metadata: &fs::Metadata) -> u32 {
    0o600
}

fn validate_target_for_logical(
    target: &Path,
    logical_path: &LogicalPath,
    binding: &ProjectBinding,
) -> Result<(), String> {
    let root = if logical_path.as_str().starts_with("project/") {
        PathBuf::from(&binding.canonical_project_root)
    } else if logical_path.as_str().starts_with("state/codex/") {
        canonical_bound_home(binding.codex_home.as_deref(), "Codex")?
    } else if logical_path.as_str().starts_with("state/claude/") {
        canonical_bound_home(binding.claude_home.as_deref(), "Claude")?
    } else {
        return Err(format!(
            "logical path '{}' has no writable target root",
            logical_path
        ));
    };
    let canonical_root = canonical_real_dir(&root, "restore root")?;
    if !target.starts_with(&root) {
        return Err(format!(
            "restore target '{}' escapes '{}';",
            target.display(),
            root.display()
        ));
    }
    let prospective = prospective_canonical_path(target)?;
    if !prospective.starts_with(&canonical_root) {
        return Err(format!(
            "restore target '{}' resolves outside '{}'",
            target.display(),
            root.display()
        ));
    }
    inspect_existing_ancestors_no_symlink(target)
}

fn inspect_existing_ancestors_no_symlink(path: &Path) -> Result<(), String> {
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(format!("symlink traversal at '{}'", existing.display()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("inspect '{}': {}", existing.display(), e)),
        }
        let Some(parent) = existing.parent() else {
            break;
        };
        if parent == existing {
            break;
        }
        existing = parent;
    }
    Ok(())
}

fn open_read_only_no_follow(path: &Path, label: &str) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|error| format!("open {}: {}", label, error))
}

fn prospective_canonical_path(path: &Path) -> Result<PathBuf, String> {
    let mut missing = Vec::new();
    let mut cursor = path;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(format!("symlink traversal at '{}'", cursor.display()));
                }
                let mut resolved = fs::canonicalize(cursor)
                    .map_err(|e| format!("resolve '{}': {}", cursor.display(), e))?;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    format!("cannot resolve prospective path '{}'", path.display())
                })?;
                missing.push(name.to_os_string());
                cursor = cursor.parent().ok_or_else(|| {
                    format!("cannot resolve prospective path '{}'", path.display())
                })?;
            }
            Err(e) => return Err(format!("inspect '{}': {}", cursor.display(), e)),
        }
    }
}

fn write_target_atomic_from_reader(
    path: &Path,
    reader: &mut dyn Read,
    expected_size: u64,
    expected_sha256: &str,
    mode: u32,
    expected_target: Option<&RestoreTargetState>,
) -> Result<(), String> {
    domain::validate_sha256("target bytes", expected_sha256)?;
    if mode & !0o777 != 0 {
        return Err("unsafe target mode".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("target '{}' has no parent", path.display()))?;
    let base = nearest_existing_directory(parent)?;
    create_safe_directory_tree(&base, parent)?;
    inspect_existing_ancestors_no_symlink(path)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create target temp in '{}': {}", parent.display(), e))?;
    copy_and_verify(
        reader,
        temp.as_file_mut(),
        expected_size,
        expected_sha256,
        &format!("target '{}'", path.display()),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file_mut()
            .set_permissions(fs::Permissions::from_mode(mode & 0o777))
            .map_err(|e| format!("set target mode '{}': {}", path.display(), e))?;
    }
    temp.as_file_mut()
        .sync_all()
        .map_err(|e| format!("sync target '{}': {}", path.display(), e))?;
    if let Some(expected) = expected_target {
        let current = inspect_restore_target(path)?;
        if current.digest != expected.digest
            || current.size != expected.size
            || current.mode != expected.mode
        {
            return Err("restore target changed while staging its replacement".to_string());
        }
    }
    temp.persist(path)
        .map_err(|e| format!("publish target '{}': {}", path.display(), e.error))?;
    Ok(())
}

pub(crate) fn write_target_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let expected_size = u64::try_from(bytes.len())
        .map_err(|_| "target bytes cannot fit the portable size field".to_string())?;
    let expected_sha256 = sha256(bytes);
    write_target_atomic_from_reader(
        path,
        &mut std::io::Cursor::new(bytes),
        expected_size,
        &expected_sha256,
        mode,
        None,
    )
}

fn write_verified_body_atomic(
    path: &Path,
    body: &VerifiedBody,
    mode: u32,
    expected_target: &RestoreTargetState,
) -> Result<(), String> {
    write_target_atomic_from_reader(
        path,
        &mut body.open()?,
        body.size(),
        body.sha256(),
        mode,
        Some(expected_target),
    )
}

pub(crate) fn write_immutable_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let expected_size = u64::try_from(bytes.len())
        .map_err(|_| "backup bytes cannot fit the portable size field".to_string())?;
    let expected_sha256 = sha256(bytes);
    write_immutable_backup_from_reader(
        path,
        &mut std::io::Cursor::new(bytes),
        expected_size,
        &expected_sha256,
    )
}

fn write_immutable_backup_from_path(
    backup_path: &Path,
    source_path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    inspect_existing_ancestors_no_symlink(source_path)?;
    let source_meta = fs::symlink_metadata(source_path).map_err(|error| {
        format!(
            "inspect backup source '{}': {}",
            source_path.display(),
            error
        )
    })?;
    if source_meta.file_type().is_symlink() || !source_meta.is_file() {
        return Err(format!(
            "backup source '{}' is not a regular no-follow file",
            source_path.display()
        ));
    }
    let mut source = open_read_only_no_follow(
        source_path,
        &format!("backup source '{}'", source_path.display()),
    )?;
    let opened_meta = source.metadata().map_err(|error| {
        format!(
            "inspect opened backup source '{}': {}",
            source_path.display(),
            error
        )
    })?;
    if !opened_meta.is_file() || opened_meta.len() != source_meta.len() {
        return Err(format!(
            "backup source '{}' changed while opening",
            source_path.display()
        ));
    }
    write_immutable_backup_from_reader(backup_path, &mut source, expected_size, expected_sha256)
}

fn write_immutable_backup_from_reader(
    path: &Path,
    reader: &mut dyn Read,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    domain::validate_sha256("backup bytes", expected_sha256)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "backup '{}' is not a regular no-follow file",
                    path.display()
                ));
            }
            if metadata.len() != expected_size {
                return Err(format!(
                    "backup '{}' already exists with different bytes",
                    path.display()
                ));
            }
            let mut existing =
                open_read_only_no_follow(path, &format!("backup '{}'", path.display()))?;
            let opened_metadata = existing.metadata().map_err(|error| {
                format!("inspect opened backup '{}': {}", path.display(), error)
            })?;
            if !opened_metadata.is_file() || opened_metadata.len() != expected_size {
                return Err(format!(
                    "backup '{}' already exists with different bytes",
                    path.display()
                ));
            }
            let digest = sha256_reader(&mut existing, &format!("backup '{}'", path.display()))?;
            return if digest == expected_sha256 {
                Ok(())
            } else {
                Err(format!(
                    "backup '{}' already exists with different bytes",
                    path.display()
                ))
            };
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect backup '{}': {}", path.display(), error)),
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("backup '{}' has no parent", path.display()))?;
    let base = nearest_existing_directory(parent)?;
    create_safe_directory_tree(&base, parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create backup temp: {}", e))?;
    copy_and_verify(
        reader,
        temp.as_file_mut(),
        expected_size,
        expected_sha256,
        &format!("backup '{}'", path.display()),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file_mut()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set backup mode: {}", e))?;
    }
    temp.persist_noclobber(path)
        .map_err(|e| format!("publish backup '{}': {}", path.display(), e.error))?;
    Ok(())
}

fn copy_and_verify(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    expected_size: u64,
    expected_sha256: &str,
    label: &str,
) -> Result<(), String> {
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {}", label, error))?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .ok_or_else(|| format!("{} byte count overflow", label))?;
        if copied > expected_size {
            return Err(format!("{} exceeds its expected size", label));
        }
        hasher.update(&buffer[..count]);
        writer
            .write_all(&buffer[..count])
            .map_err(|error| format!("write {}: {}", label, error))?;
    }
    if copied != expected_size || hex_lower(&hasher.finalize()) != expected_sha256 {
        return Err(format!("{} failed size/hash verification", label));
    }
    Ok(())
}

fn nearest_existing_directory(path: &Path) -> Result<PathBuf, String> {
    let mut cursor = path;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                return fs::canonicalize(cursor)
                    .map_err(|e| format!("resolve '{}': {}", cursor.display(), e))
            }
            Ok(_) => return Err(format!("'{}' is not a real directory", cursor.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                cursor = cursor
                    .parent()
                    .ok_or_else(|| format!("no existing ancestor for '{}'", path.display()))?;
            }
            Err(e) => return Err(format!("inspect '{}': {}", cursor.display(), e)),
        }
    }
}

fn create_safe_directory_tree(base: &Path, target: &Path) -> Result<(), String> {
    let base = fs::canonicalize(base)
        .map_err(|e| format!("resolve directory base '{}': {}", base.display(), e))?;
    let prospective = prospective_canonical_path(target)?;
    if !prospective.starts_with(&base) {
        return Err(format!(
            "directory target '{}' escapes '{}';",
            target.display(),
            base.display()
        ));
    }
    let relative = prospective
        .strip_prefix(&base)
        .map_err(|_| "directory containment changed".to_string())?;
    let mut cursor = base;
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("unsafe directory component".to_string());
        }
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(format!("'{}' is not a real directory", cursor.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&cursor)
                .map_err(|e| format!("create directory '{}': {}", cursor.display(), e))?,
            Err(e) => return Err(format!("inspect '{}': {}", cursor.display(), e)),
        }
    }
    Ok(())
}

fn validate_plan_pin(
    plan: &RestorePlan,
    snapshot: &BundleSnapshot,
    binding: &ProjectBinding,
    storage_id: &StorageId,
    now: u64,
) -> Result<(), String> {
    if now < plan.created_at || now > plan.expires_at {
        return Err("restore plan has expired or is not active yet".to_string());
    }
    if &plan.storage_id != storage_id
        || plan.storage_id != snapshot.storage_id
        || plan.bundle_id != snapshot.head.bundle_id
        || plan.host_id != binding.host_id
        || plan.replica_id != binding.replica_id
        || plan.bundle_id != binding.bundle_id
        || plan.revision != snapshot.head.revision
        || plan.commit_id != snapshot.head.commit_id
        || plan.manifest_sha256 != snapshot.head.manifest_sha256
        || plan.binding_revision != binding.revision
    {
        return Err("restore plan pin no longer matches bundle/binding state".to_string());
    }
    Ok(())
}

fn receipt_for(
    action: &RestoreAction,
    status: ActionStatus,
    applied_at: u64,
    logical_path: Option<LogicalPath>,
    target_sha256_after: Option<String>,
    error: Option<String>,
) -> ApplyReceipt {
    ApplyReceipt {
        action_id: action.action_id.clone(),
        resource_id: action.resource_id.clone(),
        action_type: action.kind.action_type(),
        logical_path,
        source_sha256: action.source_sha256.clone(),
        target_path: action.target_path.clone(),
        target_sha256_after,
        status,
        applied_at,
        error,
        conversation: None,
    }
}

fn canonical_real_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{} must be absolute: '{}'", label, path.display()));
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("inspect {} '{}': {}", label, path.display(), e))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} '{}' must be a real directory",
            label,
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|e| format!("resolve {} '{}': {}", label, path.display(), e))
}

fn canonical_or_prospective_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{} must be absolute", label));
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(format!(
                "{} '{}' is not a real directory",
                label,
                path.display()
            ));
        }
    }
    prospective_canonical_path(path)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn path_text(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("path '{}' is not UTF-8", path.display()))?
        .to_string();
    domain::validate_absolute_clean_path("filesystem path", &text)?;
    Ok(text)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn random_named_id(prefix: &str) -> Result<String, String> {
    domain::generated_named_id(prefix)
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{:02x}", byte);
    }
    output
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{:02x}", byte);
    }
    output
}

fn sha256_reader(reader: &mut impl Read, label: &str) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {}", label, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn parse_json_bytes<T: for<'de> Deserialize<'de>>(bytes: &[u8], label: &str) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("parse {}: {}", label, e))
}

fn parse_json_reader<T: for<'de> Deserialize<'de>>(
    reader: impl Read,
    label: &str,
) -> Result<T, String> {
    serde_json::from_reader(BufReader::new(reader))
        .map_err(|error| format!("parse {}: {}", label, error))
}

fn validate_store_path(value: &str, minimum_components: usize) -> Result<(), String> {
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

fn validate_bundle_relative_object_key(value: &str) -> Result<(), String> {
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
        return Err(format!("unsafe bundle-relative object key '{}'", value));
    }
    if !(value == HEAD_FILE
        || value == TAG_FILE
        || value.starts_with("_manifests/")
        || value.starts_with("_commits/")
        || value.starts_with("_objects/")
        || value.starts_with("_uploads/"))
    {
        return Err(format!("unknown bundle object namespace '{}'", value));
    }
    Ok(())
}

fn repository_object_key(bundle_id: &BundleId, relative: &str) -> Result<ObjectKey, String> {
    validate_bundle_relative_object_key(relative)?;
    ObjectKey::parse(format!("{}/{}/{}", REPOSITORY_PREFIX, bundle_id, relative))
}

fn checked_existing_object_path(root: &Path, key: &ObjectKey) -> Result<Option<PathBuf>, String> {
    let mut cursor = root.to_path_buf();
    for component in key.as_str().split('/') {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("symlink traversal in object key '{}'", key))
            }
            Ok(meta) if cursor != root.join(key.as_str()) && !meta.is_dir() => {
                return Err(format!("non-directory ancestor in object key '{}'", key))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("inspect object '{}': {}", key, e)),
        }
    }
    let canonical_parent = fs::canonicalize(
        cursor
            .parent()
            .ok_or_else(|| format!("object '{}' has no parent", key))?,
    )
    .map_err(|e| format!("resolve object parent '{}': {}", key, e))?;
    if !canonical_parent.starts_with(root) {
        return Err(format!("object '{}' escapes local store", key));
    }
    Ok(Some(cursor))
}

fn checked_create_object_path(root: &Path, key: &ObjectKey) -> Result<PathBuf, String> {
    let parts = key.as_str().split('/').collect::<Vec<_>>();
    let mut cursor = root.to_path_buf();
    for component in &parts[..parts.len() - 1] {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(format!("unsafe object ancestor '{}'", cursor.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&cursor)
                .map_err(|e| format!("create object directory '{}': {}", cursor.display(), e))?,
            Err(e) => {
                return Err(format!(
                    "inspect object directory '{}': {}",
                    cursor.display(),
                    e
                ))
            }
        }
    }
    let canonical_parent = fs::canonicalize(&cursor)
        .map_err(|e| format!("resolve object directory '{}': {}", cursor.display(), e))?;
    if !canonical_parent.starts_with(root) {
        return Err(format!("object '{}' escapes local store", key));
    }
    cursor.push(parts.last().unwrap());
    if let Ok(meta) = fs::symlink_metadata(&cursor) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("object target '{}' is unsafe", cursor.display()));
        }
    }
    Ok(cursor)
}

fn normalized_relative_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("'{}' is outside '{}';", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 store path '{}';", path.display()))?,
            ),
            _ => return Err(format!("unsafe store path '{}';", path.display())),
        }
    }
    Ok(parts.join("/"))
}

pub(crate) fn reconcile_manifest_content(
    recipe: &BundleRecipe,
    captured_descriptors: &BTreeMap<ResourceId, ResourceDescriptor>,
    captured: &CapturedResources,
    previous: Option<&BundleManifest>,
    updated_at: u64,
) -> Result<
    (
        BTreeMap<ResourceId, ResourceDescriptor>,
        BTreeMap<LogicalPath, BundleFileEntry>,
        BTreeMap<LogicalPath, BundleDirectoryEntry>,
        BTreeMap<String, Tombstone>,
    ),
    String,
> {
    for id in captured_descriptors.keys() {
        if !recipe.entries.contains_key(id) {
            return Err(format!("capture returned unselected resource '{}'", id));
        }
    }
    let captured_file_paths = captured
        .files
        .keys()
        .map(|path| LogicalPath::parse(path.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let captured_directory_paths = captured
        .directories
        .keys()
        .map(|path| LogicalPath::parse(path.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let captured_ids = captured_descriptors
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut resources = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut directories = BTreeMap::new();
    let mut tombstones = previous
        .map(|manifest| manifest.tombstones.clone())
        .unwrap_or_default();

    for id in recipe.entries.keys() {
        if let Some(descriptor) = captured_descriptors.get(id) {
            resources.insert(id.clone(), descriptor.clone());
        } else if let Some(descriptor) = previous.and_then(|manifest| manifest.resources.get(id)) {
            resources.insert(id.clone(), descriptor.clone());
            for (path, entry) in &previous.unwrap().files {
                if &entry.resource_id == id {
                    files.insert(path.clone(), entry.clone());
                }
            }
            for (path, entry) in &previous.unwrap().directories {
                if &entry.resource_id == id {
                    directories.insert(path.clone(), entry.clone());
                }
            }
        } else {
            return Err(format!(
                "selected resource '{}' is unavailable and has no previous captured version",
                id
            ));
        }
    }

    if let Some(previous) = previous {
        for (id, descriptor) in &previous.resources {
            if !recipe.entries.contains_key(id) {
                let last_sha256 = descriptor.metadata.get("content_sha256").cloned();
                let tombstone = Tombstone {
                    target: TombstoneTarget::Resource {
                        resource_id: id.clone(),
                    },
                    last_sha256,
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.canonical_key(), tombstone);
            }
        }
        for (path, old_entry) in &previous.files {
            let owner_removed = !recipe.entries.contains_key(&old_entry.resource_id);
            let recaptured_without_file = captured_ids.contains(&old_entry.resource_id)
                && !captured_file_paths.contains(path);
            if owner_removed || recaptured_without_file {
                let is_project_content = previous
                    .resources
                    .get(&old_entry.resource_id)
                    .is_some_and(|descriptor| descriptor.kind == ResourceKind::ProjectContentFile);
                let tombstone = Tombstone {
                    target: if is_project_content {
                        TombstoneTarget::ProjectContentFile {
                            resource_id: old_entry.resource_id.clone(),
                            logical_path: path.clone(),
                        }
                    } else {
                        TombstoneTarget::File {
                            resource_id: old_entry.resource_id.clone(),
                            logical_path: path.clone(),
                        }
                    },
                    last_sha256: Some(old_entry.sha256.clone()),
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.canonical_key(), tombstone);
                files.remove(path);
            }
        }
        for (path, old_entry) in &previous.directories {
            let owner_removed = !recipe.entries.contains_key(&old_entry.resource_id);
            let recaptured_without_directory = captured_ids.contains(&old_entry.resource_id)
                && !captured_directory_paths.contains(path);
            if owner_removed || recaptured_without_directory {
                let tombstone = Tombstone {
                    target: TombstoneTarget::ProjectContentDirectory {
                        resource_id: old_entry.resource_id.clone(),
                        logical_path: path.clone(),
                    },
                    last_sha256: None,
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.canonical_key(), tombstone);
                directories.remove(path);
            }
        }
    }

    tombstones.retain(|_, tombstone| match &tombstone.target {
        TombstoneTarget::Resource { resource_id } => !resources.contains_key(resource_id),
        TombstoneTarget::File {
            resource_id: _,
            logical_path,
        } => !files.contains_key(logical_path) && !captured_file_paths.contains(logical_path),
        TombstoneTarget::ProjectContentFile {
            resource_id: _,
            logical_path,
        } => !files.contains_key(logical_path) && !captured_file_paths.contains(logical_path),
        TombstoneTarget::ProjectContentDirectory {
            resource_id: _,
            logical_path,
        } => {
            !directories.contains_key(logical_path)
                && !captured_directory_paths.contains(logical_path)
        }
        TombstoneTarget::ConversationThread { .. } => true,
    });
    Ok((resources, files, directories, tombstones))
}

fn validate_conversation_retention(
    previous: &BundleManifest,
    next: &BundleManifest,
) -> Result<(), String> {
    for thread_id in previous.conversations.keys() {
        if next.conversations.contains_key(thread_id) {
            continue;
        }
        let deleted = next.tombstones.values().any(|tombstone| {
            matches!(
                &tombstone.target,
                TombstoneTarget::ConversationThread { thread_id: deleted_thread_id }
                    if deleted_thread_id == thread_id
            )
        });
        if !deleted {
            return Err(format!(
                "bundle publication drops conversation thread '{}' without a reviewed tombstone",
                thread_id
            ));
        }
    }
    Ok(())
}

/// Semantic manifest equality excludes `object_key` and revision/commit/time
/// coordinates. Object keys are derived from content and do not define the
/// reviewed bundle state.
fn manifest_is_semantic_noop(previous: &BundleManifest, candidate: &BundleManifest) -> bool {
    previous.bundle == candidate.bundle
        && previous.recipe == candidate.recipe
        && previous.captured_with.codec_versions == candidate.captured_with.codec_versions
        && previous.conversations == candidate.conversations
        && previous.resources == candidate.resources
        && files_are_semantically_equal(&previous.files, &candidate.files)
        && previous.directories == candidate.directories
        && previous.tombstones == candidate.tombstones
}

fn files_are_semantically_equal(
    previous: &BTreeMap<LogicalPath, BundleFileEntry>,
    candidate: &BTreeMap<LogicalPath, BundleFileEntry>,
) -> bool {
    previous.len() == candidate.len()
        && previous.iter().all(|(path, left)| {
            candidate.get(path).is_some_and(|right| {
                left.resource_id == right.resource_id
                    && left.sha256 == right.sha256
                    && left.size == right.size
                    && left.source_mtime == right.source_mtime
                    && left.mode == right.mode
            })
        })
}

fn file_delta(previous: Option<&BundleManifest>, current: &BundleManifest) -> (u64, u64, u64) {
    let Some(previous) = previous else {
        return (current.files.len() as u64, 0, 0);
    };
    let added = current
        .files
        .keys()
        .filter(|path| !previous.files.contains_key(*path))
        .count() as u64;
    let changed = current
        .files
        .iter()
        .filter(|(path, entry)| {
            previous
                .files
                .get(*path)
                .is_some_and(|old| old.sha256 != entry.sha256)
        })
        .count() as u64;
    let removed = previous
        .files
        .keys()
        .filter(|path| !current.files.contains_key(*path))
        .count() as u64;
    (added, changed, removed)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BackupRecord {
    pub action_id: ActionId,
    pub target_path: String,
    pub backup_path: String,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ApplyBundleResult {
    pub plan_id: PlanId,
    pub receipts: Vec<ApplyReceipt>,
    pub backups: Vec<BackupRecord>,
}

impl<S: BundleObjectStore> BundleEngine<S> {
    /// Apply approved byte-materialization actions. This engine never shells
    /// out or silently activates executable configuration.
    /// Apply approved actions, downloading only the payload objects that
    /// will actually be written. Actions whose pinned materialized digest
    /// matches the pinned target digest complete without any bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_restore_plan_lazy(
        &self,
        snapshot: &BundleSnapshot,
        bytes: &mut dyn BundleBytes,
        binding: &ProjectBinding,
        plan: &RestorePlan,
        approved_action_ids: &BTreeSet<ActionId>,
        backup_root: &Path,
        applied_at: u64,
    ) -> Result<ApplyBundleResult, String> {
        plan.validate()?;
        snapshot.validate()?;
        self.validate_binding(binding, Some(backup_root))?;
        validate_plan_pin(plan, snapshot, binding, &self.storage_id, applied_at)?;
        let known_actions = plan
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = approved_action_ids
            .iter()
            .find(|action_id| !known_actions.contains(*action_id))
        {
            return Err(format!("approval references unknown action '{}'", unknown));
        }
        let mut effective_approved = approved_action_ids.clone();
        let approved_project_files = plan
            .actions
            .iter()
            .filter_map(|action| match &action.kind {
                RestoreActionKind::WriteProjectFile { logical_path, .. }
                    if effective_approved.contains(&action.action_id) =>
                {
                    Some(logical_path.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for action in &plan.actions {
            if let RestoreActionKind::EnsureProjectDirectory { logical_path, .. } = &action.kind {
                if approved_project_files
                    .iter()
                    .any(|file| logical_path_is_ancestor(logical_path, file))
                {
                    effective_approved.insert(action.action_id.clone());
                }
            }
        }
        let approves_project_content = plan.actions.iter().any(|action| {
            effective_approved.contains(&action.action_id)
                && matches!(
                    action.kind,
                    RestoreActionKind::WriteProjectFile { .. }
                        | RestoreActionKind::EnsureProjectDirectory { .. }
                        | RestoreActionKind::DeleteProjectFile { .. }
                        | RestoreActionKind::DeleteProjectDirectory { .. }
                )
        });
        if approves_project_content
            && plan
                .project_content_eligibility
                .as_ref()
                .is_some_and(|eligibility| {
                    eligibility.state != domain::ProjectFileSyncEligibilityState::Eligible
                })
        {
            return Err(
                "project content cannot be applied because this folder is Git-managed or eligibility is unknown"
                    .to_string(),
            );
        }

        let current = self
            .read_head(&plan.bundle_id)?
            .ok_or_else(|| "bundle head disappeared after planning".to_string())?;
        if current.0.revision != plan.revision
            || current.0.commit_id != plan.commit_id
            || current.0.manifest_size != snapshot.head.manifest_size
            || current.0.manifest_sha256 != plan.manifest_sha256
        {
            return Err("bundle head changed after restore planning".to_string());
        }
        if !backup_root.is_absolute() {
            return Err("backup root must be absolute".to_string());
        }
        let backup_base = nearest_existing_directory(backup_root)?;
        create_safe_directory_tree(&backup_base, backup_root)?;
        let canonical_backup = canonical_real_dir(backup_root, "backup root")?;
        let plan_backup = canonical_backup.join(plan.plan_id.as_str());
        create_safe_directory_tree(&canonical_backup, &plan_backup)?;

        // Bulk-load the payloads approved actions will actually write, so
        // download progress reflects the real transfer. A pinned
        // materialized digest equal to the pinned target digest is a no-op
        // needing no bytes.
        let mut prefetch_paths = Vec::new();
        for action in &plan.actions {
            if !effective_approved.contains(&action.action_id) {
                continue;
            }
            let logical_path = match &action.kind {
                RestoreActionKind::WriteFile { logical_path }
                | RestoreActionKind::WriteProjectFile { logical_path, .. }
                | RestoreActionKind::MergeFile { logical_path }
                | RestoreActionKind::MaterializeConversation { logical_path, .. } => logical_path,
                _ => continue,
            };
            let pinned_noop = action.materialized_sha256.is_some()
                && action.materialized_sha256 == action.expected_target_sha256;
            if !pinned_noop && snapshot.manifest.files.contains_key(logical_path) {
                prefetch_paths.push(logical_path.clone());
            }
        }
        bytes.prefetch(&prefetch_paths)?;

        let mut receipts = Vec::new();
        let mut backups = Vec::new();

        let mut prepared_directories = Vec::new();
        let mut ensure_actions = plan
            .actions
            .iter()
            .filter(|action| {
                effective_approved.contains(&action.action_id)
                    && matches!(
                        action.kind,
                        RestoreActionKind::EnsureProjectDirectory { .. }
                    )
            })
            .collect::<Vec<_>>();
        ensure_actions.sort_by_key(|action| project_action_depth(action));
        for action in ensure_actions {
            match self.prepare_project_directory(binding, action) {
                Ok(()) => prepared_directories.push(action),
                Err(error) => receipts.push(receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )),
            }
        }

        let mut approved_file_deletions = Vec::new();
        let mut approved_directory_deletions = Vec::new();

        for action in &plan.actions {
            if !effective_approved.contains(&action.action_id) {
                let keeps_local_conversation = is_conversation_keep_local_action(action);
                receipts.push(receipt_for(
                    action,
                    ActionStatus::Skipped,
                    applied_at,
                    None,
                    keeps_local_conversation
                        .then(|| action.expected_target_sha256.clone())
                        .flatten(),
                    Some(if keeps_local_conversation {
                        "local conversation was kept by the reviewed Pull".to_string()
                    } else {
                        "action was not approved".to_string()
                    }),
                ));
                continue;
            }
            match &action.kind {
                RestoreActionKind::EnsureProjectDirectory { .. } => {}
                RestoreActionKind::DeleteProjectFile { .. } => {
                    approved_file_deletions.push(action);
                }
                RestoreActionKind::DeleteProjectDirectory { .. } => {
                    approved_directory_deletions.push(action);
                }
                RestoreActionKind::Manual { .. } => {
                    receipts.push(receipt_for(
                        action,
                        ActionStatus::Blocked,
                        applied_at,
                        None,
                        None,
                        Some(
                            "definition requires its provider-specific review/composer".to_string(),
                        ),
                    ));
                }
                RestoreActionKind::WriteFile { logical_path }
                | RestoreActionKind::WriteProjectFile { logical_path, .. }
                | RestoreActionKind::MergeFile { logical_path }
                | RestoreActionKind::MaterializeConversation { logical_path, .. } => {
                    let receipt = self.apply_file_action(
                        snapshot,
                        bytes,
                        binding,
                        action,
                        logical_path,
                        &plan_backup,
                        applied_at,
                        &mut backups,
                    );
                    receipts.push(match receipt {
                        Ok(receipt) => receipt,
                        Err(error) => receipt_for(
                            action,
                            ActionStatus::Failed,
                            applied_at,
                            None,
                            None,
                            Some(error),
                        ),
                    });
                }
            }
        }

        approved_file_deletions
            .sort_by_key(|action| std::cmp::Reverse(project_action_depth(action)));
        for action in approved_file_deletions {
            let receipt = self.apply_project_file_deletion(
                binding,
                action,
                &plan_backup,
                applied_at,
                &mut backups,
            );
            receipts.push(receipt.unwrap_or_else(|error| {
                receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )
            }));
        }

        approved_directory_deletions
            .sort_by_key(|action| std::cmp::Reverse(project_action_depth(action)));
        for action in approved_directory_deletions {
            let receipt = self.apply_project_directory_deletion(binding, action, applied_at);
            receipts.push(receipt.unwrap_or_else(|error| {
                receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )
            }));
        }

        prepared_directories.sort_by_key(|action| std::cmp::Reverse(project_action_depth(action)));
        for action in prepared_directories {
            let receipt = self.finalize_project_directory(binding, action, applied_at);
            receipts.push(receipt.unwrap_or_else(|error| {
                receipt_for(
                    action,
                    ActionStatus::Failed,
                    applied_at,
                    project_action_logical_path(action),
                    None,
                    Some(error),
                )
            }));
        }
        Ok(ApplyBundleResult {
            plan_id: plan.plan_id.clone(),
            receipts,
            backups,
        })
    }

    fn prepare_project_directory(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
    ) -> Result<(), String> {
        let RestoreActionKind::EnsureProjectDirectory { logical_path, .. } = &action.kind else {
            return Err("directory preparation received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("project directory target differs from the pinned plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        match (
            inspect_project_directory_target(&target)?,
            action.expected_target_mode,
        ) {
            (Some(actual), Some(expected)) if actual == expected => Ok(()),
            (Some(_), Some(_)) => Err("project directory mode changed after planning".to_string()),
            (Some(_), None) => Err("project directory appeared after planning".to_string()),
            (None, Some(_)) => Err("project directory disappeared after planning".to_string()),
            (None, None) => {
                let parent = target
                    .parent()
                    .ok_or_else(|| "project directory has no parent".to_string())?;
                let parent_meta = fs::symlink_metadata(parent).map_err(|error| {
                    format!(
                        "inspect project directory parent '{}': {}",
                        parent.display(),
                        error
                    )
                })?;
                if parent_meta.file_type().is_symlink() || !parent_meta.is_dir() {
                    return Err(format!(
                        "project directory parent '{}' is not a real directory",
                        parent.display()
                    ));
                }
                fs::create_dir(&target).map_err(|error| {
                    format!("create project directory '{}': {}", target.display(), error)
                })?;
                set_path_mode(&target, 0o700)
            }
        }
    }

    fn finalize_project_directory(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
        applied_at: u64,
    ) -> Result<ApplyReceipt, String> {
        let RestoreActionKind::EnsureProjectDirectory {
            logical_path,
            mode,
            source_mtime,
        } = &action.kind
        else {
            return Err("directory finalization received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        validate_target_for_logical(&target, logical_path, binding)?;
        if inspect_project_directory_target(&target)?.is_none() {
            return Err("project directory disappeared before finalization".to_string());
        }
        set_path_mode(&target, *mode)?;
        set_path_mtime_best_effort(&target, *source_mtime);
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            action.source_sha256.clone(),
            None,
        ))
    }

    fn apply_project_file_deletion(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        let RestoreActionKind::DeleteProjectFile {
            logical_path,
            last_sha256,
        } = &action.kind
        else {
            return Err("file deletion received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("project-file deletion target differs from the plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        let before = inspect_restore_target(&target)?;
        let Some(before_digest) = before.digest.as_deref() else {
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                None,
                None,
            ));
        };
        if before_digest != last_sha256
            || action.expected_target_sha256.as_deref() != Some(last_sha256.as_str())
        {
            return Err("local project file changed; deletion was blocked".to_string());
        }
        if action
            .expected_target_mode
            .zip(before.mode)
            .is_some_and(|(expected, actual)| expected != actual)
        {
            return Err("local project file mode changed; deletion was blocked".to_string());
        }
        let backup_path = plan_backup.join(format!("{}.delete.bak", action.action_id.as_str()));
        write_immutable_backup_from_path(
            &backup_path,
            &target,
            before
                .size
                .ok_or_else(|| "restore target size disappeared".to_string())?,
            last_sha256,
        )?;
        let current = inspect_restore_target(&target)?;
        if current.digest.as_deref() != Some(last_sha256.as_str()) || current.mode != before.mode {
            return Err("local project file changed while backing it up".to_string());
        }
        backups.push(BackupRecord {
            action_id: action.action_id.clone(),
            target_path: path_text(&target)?,
            backup_path: path_text(&backup_path)?,
            sha256: last_sha256.clone(),
        });
        fs::remove_file(&target)
            .map_err(|error| format!("delete project file '{}': {}", target.display(), error))?;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            None,
            None,
        ))
    }

    fn apply_project_directory_deletion(
        &self,
        binding: &ProjectBinding,
        action: &RestoreAction,
        applied_at: u64,
    ) -> Result<ApplyReceipt, String> {
        let RestoreActionKind::DeleteProjectDirectory { logical_path } = &action.kind else {
            return Err("directory deletion received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("project-directory deletion target differs from the plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        let Some(actual_mode) = inspect_project_directory_target(&target)? else {
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                None,
                None,
            ));
        };
        if action
            .expected_target_mode
            .is_some_and(|expected| expected != actual_mode)
        {
            return Err("local project directory mode changed; deletion was blocked".to_string());
        }
        if fs::read_dir(&target)
            .map_err(|error| format!("read project directory '{}': {}", target.display(), error))?
            .next()
            .is_some()
        {
            return Err(
                "project directory is not empty; recursive deletion is forbidden".to_string(),
            );
        }
        fs::remove_dir(&target).map_err(|error| {
            format!(
                "delete empty project directory '{}': {}",
                target.display(),
                error
            )
        })?;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            None,
            None,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_file_action(
        &self,
        snapshot: &BundleSnapshot,
        bytes: &mut dyn BundleBytes,
        binding: &ProjectBinding,
        action: &RestoreAction,
        logical_path: &LogicalPath,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        let entry = snapshot
            .manifest
            .files
            .get(logical_path)
            .ok_or_else(|| format!("plan references missing file '{}'", logical_path))?;
        if entry.resource_id != action.resource_id {
            return Err("restore action resource changed after planning".to_string());
        }
        let descriptor = snapshot
            .manifest
            .resources
            .get(&entry.resource_id)
            .ok_or_else(|| "restore action resource is missing".to_string())?;
        let target = map_logical_target(logical_path, descriptor, binding)?
            .ok_or_else(|| format!("'{}' has no file materializer", logical_path))?;
        let planned_target = action
            .target_path
            .as_deref()
            .ok_or_else(|| "file restore action has no target".to_string())?;
        if Path::new(planned_target) != target {
            return Err("restore target differs from the pinned plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding)?;
        let before = inspect_restore_target(&target)?;
        match (&action.expected_target_sha256, &before.digest) {
            (None, None) => {}
            (Some(expected), Some(actual)) if expected == actual => {}
            _ => return Err("restore target changed after planning".to_string()),
        }
        if action.expected_target_mode != before.mode
            && matches!(action.kind, RestoreActionKind::WriteProjectFile { .. })
        {
            return Err("project-file mode changed after planning".to_string());
        }
        // Pinned no-op: the plan proved what these bytes would be, and the
        // target already holds them — finish without touching the payload.
        if let Some(materialized) = action
            .materialized_sha256
            .as_deref()
            .filter(|materialized| before.digest.as_deref() == Some(materialized))
        {
            if let RestoreActionKind::WriteProjectFile {
                mode, source_mtime, ..
            } = &action.kind
            {
                if (entry.mode.unwrap_or(0o600) & 0o777) != (*mode & 0o777) {
                    return Err("project-file metadata changed after planning".to_string());
                }
                set_path_mode(&target, *mode)?;
                set_path_mtime_best_effort(&target, *source_mtime);
            }
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                Some(materialized.to_string()),
                None,
            ));
        }

        let source_body = bytes.body(logical_path)?;
        if source_body.size() != entry.size || source_body.sha256() != entry.sha256 {
            return Err(format!(
                "byte source for '{}' differs from the pinned manifest",
                logical_path
            ));
        }
        let materialized_sha256 = source_body.sha256().to_string();
        if before.digest.as_deref() == Some(materialized_sha256.as_str()) {
            if let RestoreActionKind::WriteProjectFile {
                mode, source_mtime, ..
            } = &action.kind
            {
                if (entry.mode.unwrap_or(0o600) & 0o777) != (*mode & 0o777) {
                    return Err("project-file metadata changed after planning".to_string());
                }
                set_path_mode(&target, *mode)?;
                set_path_mtime_best_effort(&target, *source_mtime);
            }
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                Some(materialized_sha256),
                None,
            ));
        }

        if let (Some(existing_digest), Some(existing_size)) =
            (before.digest.as_deref(), before.size)
        {
            let backup_path = plan_backup.join(format!("{}.bak", action.action_id.as_str()));
            write_immutable_backup_from_path(
                &backup_path,
                &target,
                existing_size,
                existing_digest,
            )?;
            backups.push(BackupRecord {
                action_id: action.action_id.clone(),
                target_path: path_text(&target)?,
                backup_path: path_text(&backup_path)?,
                sha256: existing_digest.to_string(),
            });
        }
        write_verified_body_atomic(&target, &source_body, entry.mode.unwrap_or(0o600), &before)?;
        if let RestoreActionKind::WriteProjectFile {
            mode, source_mtime, ..
        } = &action.kind
        {
            if (entry.mode.unwrap_or(0o600) & 0o777) != (*mode & 0o777) {
                return Err("project-file metadata changed after planning".to_string());
            }
            set_path_mode(&target, *mode)?;
            set_path_mtime_best_effort(&target, *source_mtime);
        }
        let after = materialized_sha256;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            Some(after),
            None,
        ))
    }
}
