//! Bundle storage, publication, fetch, restore planning, and apply.
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
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use super::conversation_codec::CodecLineage;
use super::domain::{
    self, ActionId, ActionStatus, ApplyPolicy, ApplyReceipt, BindingState, BundleDirectoryEntry,
    BundleFileEntry, BundleHead, BundleId, BundleIdentity, BundleKind, BundleManifest,
    BundleRecipe, BundleSnapshot, BundleVersionRef, BundleVersionSnapshot, CapturedWith,
    ConversationFileRef, ConversationFileRole, ConversationLineageStatus, ConversationThread,
    DependencyAction, DependencyActionKind, LogicalPath, PlanId, PortableOrigin, ProjectBinding,
    Provider, ResourceDescriptor, ResourceId, ResourceKind, RestoreAction, RestoreActionKind,
    RestorePlan, StorageId, ThreadId, Tombstone, TombstoneTarget, BUNDLE_SCHEMA,
    RESTORE_PLAN_SCHEMA,
};
use super::provider_capture::{self, CapturedResources};

const MALLARD_ROOT: &str = ".mallard";
const STORAGE_MARKER_KEY: &str = ".mallard/_storage.json";
const REPOSITORY_PREFIX: &str = ".mallard/v1/repositories";
const STORAGE_MARKER_FORMAT: &str = "mallard-storage";
const STORAGE_LAYOUT_VERSION: u32 = 1;
const LOCAL_STORAGE_LOCK: &str = ".storage.lock";
const HEAD_FILE: &str = "_head.json";
const TAG_FILE: &str = "_tag.json";
const MAX_OBJECT_BYTES: usize = 512 * 1024 * 1024;
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
pub struct StoredObject {
    pub bytes: Vec<u8>,
    pub etag: String,
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
    fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String>;

    /// Fetch several objects; results line up with `keys`. Remote stores
    /// override this with a bounded-concurrency implementation.
    fn get_many(&self, keys: &[ObjectKey]) -> Result<Vec<Option<StoredObject>>, String> {
        keys.iter().map(|key| self.get(key)).collect()
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

    fn read_unlocked(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        let path = checked_existing_object_path(&self.root, key)?;
        let Some(path) = path else {
            return Ok(None);
        };
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("inspect '{}': {}", key, e))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("object '{}' is not a regular no-follow file", key));
        }
        if meta.len() > MAX_OBJECT_BYTES as u64 {
            return Err(format!("object '{}' exceeds the read limit", key));
        }
        let bytes = fs::read(&path).map_err(|e| format!("read '{}': {}", key, e))?;
        Ok(Some(StoredObject {
            etag: sha256(&bytes),
            bytes,
        }))
    }

    fn write_atomic_unlocked(&self, key: &ObjectKey, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(format!("object '{}' exceeds the write limit", key));
        }
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
    fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        self.read_unlocked(key)
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        let _guard = self.lock()?;
        if let Some(existing) = self.read_unlocked(key)? {
            if existing.bytes == bytes {
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
        let current = self.read_unlocked(key)?;
        let matches = match expectation {
            CasExpectation::Absent => current.is_none(),
            CasExpectation::Match(expected) => current
                .as_ref()
                .is_some_and(|object| object.etag == *expected),
        };
        if !matches {
            return Ok(CasOutcome::Conflict {
                current_etag: current.map(|object| object.etag),
            });
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

const MAX_GENERATION_PAGE: usize = 500;

/// One reachable generation whose record hash was verified against the
/// chain during traversal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedGeneration {
    pub version: BundleVersionRef,
    pub record: domain::BundleGenerationRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedGenerationPage {
    pub generations: Vec<VerifiedGeneration>,
    pub next_cursor: Option<u64>,
    pub observed_head: BundleHead,
}

/// Portable description of one reachable generation for listing and
/// comparison responses. Everything here comes from verified metadata.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleGenerationSummary {
    pub version: BundleVersionRef,
    pub created_at: u64,
    pub published_by_origin: PortableOrigin,
    pub delta: domain::GenerationDelta,
}

impl From<VerifiedGeneration> for BundleGenerationSummary {
    fn from(verified: VerifiedGeneration) -> Self {
        Self {
            version: verified.version,
            created_at: verified.record.created_at,
            published_by_origin: verified.record.published_by_origin,
            delta: verified.record.delta,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleGenerationPage {
    pub generations: Vec<BundleGenerationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<u64>,
    pub observed_head: BundleVersionRef,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundleResourceChangeState {
    Added,
    Removed,
    Changed,
    Unchanged,
}

/// One resource row of a generation comparison, labeled from manifest
/// metadata only. Titles are the stored conversation display titles of each
/// side, so a rename is visible as left/right disagreement.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleResourceChange {
    pub resource_id: ResourceId,
    pub kind: ResourceKind,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_thread_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_thread_title: Option<String>,
    pub state: BundleResourceChangeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_content_sha256: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BundleGenerationComparison {
    pub left: BundleGenerationSummary,
    pub right: BundleGenerationSummary,
    /// Logical snapshot equality via the canonical content digests. This is
    /// deliberately separate from manifest identity: two generations with
    /// different manifests (coordinates, timestamps) can carry the same
    /// snapshot.
    pub same_snapshot: bool,
    pub added: u64,
    pub removed: u64,
    pub changed: u64,
    pub unchanged: u64,
    pub resource_changes: Vec<BundleResourceChange>,
    pub bundle_metadata_changed: bool,
    pub recipe_changed: bool,
    pub codec_changed: bool,
    pub tombstones_changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadToken {
    pub generation: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub etag: String,
}

/// One file in the candidate snapshot. `Retained` entries are copied from the
/// observed previous manifest and re-verified against it by metadata, so their
/// payloads never travel again. `Captured` entries carry fresh bytes and are
/// stored content-addressed under `_objects/<sha256>`.
#[derive(Clone, Debug)]
pub enum PublishFile {
    Retained(BundleFileEntry),
    Captured {
        resource_id: ResourceId,
        bytes: Vec<u8>,
        source_mtime: u64,
        mode: Option<u32>,
    },
}

/// Fully reconciled manifest-shaped content for one candidate generation.
/// Production Push builds this from a provider capture with
/// [`BundleEngine::build_publish_content`]; reviewed flows may construct it
/// directly. The publisher validates it as a whole — it never re-reconciles.
#[derive(Clone, Debug, Default)]
pub struct PublishContent {
    pub conversations: BTreeMap<ThreadId, ConversationThread>,
    pub resources: BTreeMap<ResourceId, ResourceDescriptor>,
    pub files: BTreeMap<LogicalPath, PublishFile>,
    pub directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
    pub tombstones: BTreeMap<String, Tombstone>,
}

#[derive(Clone, Debug)]
pub struct PublishBundleRequest {
    pub identity: BundleIdentity,
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub published_by_origin: PortableOrigin,
    pub content: PublishContent,
    /// Head observed by the caller's single read; the head CAS on its etag is
    /// the concurrency guard, so publish never re-reads the head itself.
    pub observed_head: Option<(BundleHead, HeadToken)>,
    pub updated_at: u64,
}

#[derive(Clone, Debug)]
pub struct PublishedBundle {
    /// False when the candidate manifest was semantically identical to the
    /// observed head's: nothing was written and `snapshot`/`head_token` are
    /// the observed generation's.
    pub published: bool,
    pub snapshot: BundleSnapshot,
    // Read by CAS-chaining tests; command flows re-read the head instead.
    #[cfg_attr(not(test), allow(dead_code))]
    pub head_token: HeadToken,
}

#[derive(Clone, Debug)]
pub struct FetchedBundle {
    pub snapshot: BundleSnapshot,
    pub files: BTreeMap<LogicalPath, Vec<u8>>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub dependency_actions: Vec<DependencyAction>,
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

const OBJECT_CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Objects per `get_many` round in `EngineByteSource::prefetch` — one
/// concurrent store window per batch, with progress after each object.
const PREFETCH_BATCH_OBJECTS: usize = 6;
/// Objects per `put_many` round in `publish_with_progress`; same shape as
/// the prefetch batching, with progress after each object.
const UPLOAD_BATCH_OBJECTS: usize = 6;

/// Process-wide LRU for verified payload objects, keyed by content hash.
/// Bundle objects are immutable, so an entry can never go stale; eviction
/// exists only to bound memory. One Pull review warms the cache for the
/// apply that follows it.
pub struct ObjectCache {
    inner: Mutex<ObjectCacheInner>,
}

struct ObjectCacheInner {
    entries: HashMap<(StorageId, BundleId, String), Arc<Vec<u8>>>,
    order: VecDeque<(StorageId, BundleId, String)>,
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
    fn get(&self, key: &(StorageId, BundleId, String)) -> Option<Arc<Vec<u8>>> {
        let mut inner = self.inner.lock().ok()?;
        let hit = inner.entries.get(key).cloned()?;
        if let Some(position) = inner.order.iter().position(|current| current == key) {
            inner.order.remove(position);
        }
        inner.order.push_back(key.clone());
        Some(hit)
    }

    fn insert(&self, key: (StorageId, BundleId, String), bytes: Arc<Vec<u8>>) {
        if bytes.len() as u64 > OBJECT_CACHE_MAX_BYTES {
            return;
        }
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.entries.contains_key(&key) {
            return;
        }
        inner.total_bytes = inner.total_bytes.saturating_add(bytes.len() as u64);
        inner.entries.insert(key.clone(), bytes);
        inner.order.push_back(key);
        while inner.total_bytes > OBJECT_CACHE_MAX_BYTES {
            let Some(oldest) = inner.order.pop_front() else {
                break;
            };
            if let Some(evicted) = inner.entries.remove(&oldest) {
                inner.total_bytes = inner.total_bytes.saturating_sub(evicted.len() as u64);
            }
        }
    }
}

/// Payload access for planning and apply. Implementations verify each
/// object against the manifest exactly once — callers trust returned bytes.
pub trait BundleBytes {
    fn get(&mut self, logical_path: &LogicalPath) -> Result<&[u8], String>;

    /// Bulk-load a known set so progress totals reflect the real transfer.
    /// The default sequential loop is sufficient for preloaded sources.
    fn prefetch(&mut self, paths: &[LogicalPath]) -> Result<(), String> {
        for path in paths {
            self.get(path)?;
        }
        Ok(())
    }
}

/// Byte source over an already fully downloaded bundle. Existing callers and
/// engine tests use this; it re-verifies on access exactly like the historic
/// apply path did.
pub struct PreloadedBytes<'a> {
    bundle: &'a FetchedBundle,
}

impl<'a> PreloadedBytes<'a> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(bundle: &'a FetchedBundle) -> Self {
        Self { bundle }
    }
}

impl BundleBytes for PreloadedBytes<'_> {
    fn get(&mut self, logical_path: &LogicalPath) -> Result<&[u8], String> {
        let entry = self
            .bundle
            .snapshot
            .manifest
            .files
            .get(logical_path)
            .ok_or_else(|| format!("manifest has no file entry for '{}'", logical_path))?;
        let bytes = self
            .bundle
            .files
            .get(logical_path)
            .ok_or_else(|| format!("fetched bytes for '{}' are missing", logical_path))?;
        if bytes.len() as u64 != entry.size || sha256(bytes) != entry.sha256 {
            return Err(format!(
                "fetched bytes for '{}' failed verification",
                logical_path
            ));
        }
        Ok(bytes)
    }
}

/// Store-backed lazy byte source: downloads objects on demand, verifies them
/// against the manifest, and shares them through the process-wide
/// [`ObjectCache`].
pub struct EngineByteSource<'a, S: BundleObjectStore> {
    engine: &'a BundleEngine<S>,
    snapshot: &'a BundleVersionSnapshot,
    cache: Option<Arc<ObjectCache>>,
    loaded: BTreeMap<LogicalPath, Arc<Vec<u8>>>,
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
        let bundle_id = self.snapshot.bundle_id();
        let cache_key = (
            self.snapshot.storage_id.clone(),
            bundle_id.clone(),
            entry.sha256.clone(),
        );
        if let Some(hit) = self.cache.as_ref().and_then(|cache| cache.get(&cache_key)) {
            self.loaded.insert(logical_path.clone(), hit);
            return Ok(());
        }
        let key = repository_object_key(bundle_id, &entry.object_key)?;
        let object = self
            .engine
            .store
            .get(&key)?
            .ok_or_else(|| format!("bundle object '{}' is missing", key))?;
        if object.bytes.len() as u64 != entry.size || sha256(&object.bytes) != entry.sha256 {
            return Err(format!(
                "bundle object '{}' does not match manifest size/hash",
                key
            ));
        }
        let bytes = Arc::new(object.bytes);
        if let Some(cache) = &self.cache {
            cache.insert(cache_key, bytes.clone());
        }
        self.loaded.insert(logical_path.clone(), bytes);
        Ok(())
    }
}

impl<S: BundleObjectStore> BundleBytes for EngineByteSource<'_, S> {
    fn get(&mut self, logical_path: &LogicalPath) -> Result<&[u8], String> {
        self.ensure_loaded(logical_path)?;
        Ok(self
            .loaded
            .get(logical_path)
            .expect("object loaded by ensure_loaded")
            .as_slice())
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
            let cache_key = (
                self.snapshot.storage_id.clone(),
                self.snapshot.bundle_id().clone(),
                entry.sha256.clone(),
            );
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
            let keys = batch
                .iter()
                .map(|(_, entry)| {
                    repository_object_key(self.snapshot.bundle_id(), &entry.object_key)
                })
                .collect::<Result<Vec<_>, String>>()?;
            let objects = self.engine.store.get_many(&keys)?;
            for (((path, entry), key), object) in batch.iter().zip(&keys).zip(objects) {
                let object = object.ok_or_else(|| format!("bundle object '{}' is missing", key))?;
                if object.bytes.len() as u64 != entry.size || sha256(&object.bytes) != entry.sha256
                {
                    return Err(format!(
                        "bundle object '{}' does not match manifest size/hash",
                        key
                    ));
                }
                let bytes = Arc::new(object.bytes);
                if let Some(cache) = &self.cache {
                    cache.insert(
                        (
                            self.snapshot.storage_id.clone(),
                            self.snapshot.bundle_id().clone(),
                            entry.sha256.clone(),
                        ),
                        bytes.clone(),
                    );
                }
                self.loaded.insert(path.clone(), bytes);
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

/// The reviewed `(storage_digest, local_digest)` pair recorded for a
/// conversation resource by the most recent applied Pull. When the stored
/// object still has `storage_digest`, `local_digest` IS the materialized
/// digest for this binding — planning needs no payload bytes to know it.
#[derive(Clone, Debug)]
pub struct KnownMaterializedBase {
    pub storage_digest: String,
    pub local_digest: String,
    pub kept_local: bool,
}

/// Per-file planning decision from pass 1 of `build_restore_plan_lazy`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PlannedOutcome {
    /// Blocked or unmaterializable — becomes a manual action.
    Manual(String),
    /// The local conversation diverged; the reviewed Pull keeps it.
    KeepLocal,
    /// Emit a restore action; the materialized digest is `Some` whenever it
    /// was derivable without payload bytes.
    Action { materialized: Option<String> },
    /// Divergence cannot be classified from metadata — download the object.
    NeedsBytes,
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

/// Whether materialization rewrites this file's bytes per binding (Codex
/// session cwd remap). Must stay in lockstep with `materialized_file_bytes`.
fn is_remapped_conversation(logical_path: &LogicalPath, descriptor: &ResourceDescriptor) -> bool {
    descriptor.kind == ResourceKind::CodexConversation
        && (logical_path.as_str().starts_with("state/codex/sessions/")
            || logical_path
                .as_str()
                .starts_with("state/codex/archived_sessions/"))
}

/// Decide a remapped conversation's plan outcome without payload bytes.
///
/// `base` must already be filtered to `storage_digest == entry_sha`, meaning
/// the stored object is unchanged since the reviewed base — which makes
/// `base.local_digest` the exact materialized digest for this binding (for a
/// materialized base) or the reviewed kept-local digest (for a kept one).
fn classify_remapped_conversation(
    entry_sha: &str,
    base: Option<&KnownMaterializedBase>,
    target_digest: Option<&str>,
) -> PlannedOutcome {
    if let Some(base) = base {
        if let Some(digest) = target_digest {
            if digest == base.local_digest {
                return if base.kept_local {
                    // The reviewed keep-local decision still describes the
                    // disk state; surface it again (callers routinely drop
                    // it when the stored side is also unchanged).
                    PlannedOutcome::KeepLocal
                } else {
                    PlannedOutcome::Action {
                        materialized: Some(base.local_digest.clone()),
                    }
                };
            }
            if !base.kept_local {
                if digest == entry_sha {
                    return PlannedOutcome::Action {
                        materialized: Some(base.local_digest.clone()),
                    };
                }
                // The target moved away from the known materialized digest,
                // so it cannot equal it — divergence is proven metadata-only.
                return PlannedOutcome::KeepLocal;
            }
        } else if !base.kept_local {
            return PlannedOutcome::Action {
                materialized: Some(base.local_digest.clone()),
            };
        }
    }
    match target_digest {
        None => PlannedOutcome::Action { materialized: None },
        Some(digest) if digest == entry_sha => PlannedOutcome::Action { materialized: None },
        Some(_) => PlannedOutcome::NeedsBytes,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct BundleTag {
    schema_version: u32,
    bundle_id: BundleId,
    display_name: String,
    kind: BundleKind,
    generation: u64,
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
    pub generation: u64,
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
        if let Some(stored) = self.store.get(&key)? {
            let marker: StorageMarker =
                parse_bounded_json(&stored.bytes, "Mallard storage marker")?;
            marker.validate()?;
            return Ok(());
        }

        let bytes = serde_json::to_vec(&StorageMarker::current())
            .map_err(|error| format!("serialize Mallard storage marker: {}", error))?;
        self.store.put_immutable(&key, &bytes)?;
        let stored = self
            .store
            .get(&key)?
            .ok_or_else(|| "Mallard storage marker disappeared after creation".to_string())?;
        let marker: StorageMarker = parse_bounded_json(&stored.bytes, "Mallard storage marker")?;
        marker.validate()
    }

    pub fn read_head(
        &self,
        bundle_id: &BundleId,
    ) -> Result<Option<(BundleHead, HeadToken)>, String> {
        let key = repository_object_key(bundle_id, HEAD_FILE)?;
        let Some(stored) = self.store.get(&key)? else {
            return Ok(None);
        };
        let head: BundleHead = parse_bounded_json(&stored.bytes, "bundle head")?;
        head.validate()?;
        if &head.bundle_id != bundle_id {
            return Err(format!(
                "bundle head '{}' is stored under bundle '{}'",
                head.bundle_id, bundle_id
            ));
        }
        let token = HeadToken {
            generation: head.latest.generation,
            commit_id: head.latest.commit_id.clone(),
            manifest_sha256: head.latest.manifest_sha256.clone(),
            etag: stored.etag,
        };
        Ok(Some((head, token)))
    }

    /// Publish immutable objects and history first, then compare-and-swap the
    /// single bundle head. A failed CAS can leave only unreachable immutable
    /// objects, never a partially visible generation. Payloads are content-
    /// addressed under `_objects/<sha256>`, so bytes already referenced by the
    /// previous manifest never travel again and a semantically unchanged
    /// publish writes nothing at all.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn publish(&self, request: PublishBundleRequest) -> Result<PublishedBundle, String> {
        self.publish_with_progress(request, &mut |_| {})
    }

    /// `publish` with per-object upload progress, mirroring the
    /// `fetch`/`fetch_with_progress` split. Progress totals cover only the
    /// objects that actually upload; a publish with full payload reuse emits
    /// no events.
    pub fn publish_with_progress(
        &self,
        mut request: PublishBundleRequest,
        on_progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<PublishedBundle, String> {
        request.identity.validate()?;
        request.recipe.validate()?;
        request.published_by_origin.validate()?;
        if request.identity.kind != BundleKind::Project {
            return Err("bundle publication supports only project bundles".to_string());
        }
        if request.updated_at == 0 {
            return Err("publish timestamp must be non-zero".to_string());
        }

        let current = request.observed_head.take();
        if current
            .as_ref()
            .is_some_and(|(head, _)| head.bundle_id != request.identity.bundle_id)
        {
            return Err("observed head belongs to a different bundle".to_string());
        }
        let cas_expectation = match &current {
            None => CasExpectation::Absent,
            Some((_, token)) => CasExpectation::Match(token.etag.clone()),
        };
        let previous_manifest = current
            .as_ref()
            .map(|(head, _)| self.read_manifest(head))
            .transpose()?;
        let generation = current
            .as_ref()
            .map(|(head, _)| head.latest.generation.saturating_add(1))
            .unwrap_or(1);
        let commit_id = random_named_id("commit")?;

        let content = std::mem::take(&mut request.content);
        // Content identities the previous manifest already backs with a
        // content-addressed object; those bytes are known-present remotely
        // and never travel again.
        let previous_shas: BTreeSet<&str> = previous_manifest
            .as_ref()
            .map(|manifest| {
                manifest
                    .files
                    .values()
                    .filter(|entry| entry.object_key == format!("_objects/{}", entry.sha256))
                    .map(|entry| entry.sha256.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let mut files = BTreeMap::new();
        // Keyed by digest: identical bytes captured under several logical
        // paths upload once and share one object.
        let mut pending: BTreeMap<String, Bytes> = BTreeMap::new();
        for (logical, file) in content.files {
            match file {
                PublishFile::Retained(entry) => {
                    entry.validate()?;
                    let previous = previous_manifest.as_ref().ok_or_else(|| {
                        format!("retained file '{}' has no previous manifest", logical)
                    })?;
                    let was_published = previous.files.values().any(|candidate| {
                        candidate.object_key == entry.object_key
                            && candidate.sha256 == entry.sha256
                            && candidate.size == entry.size
                    });
                    if !was_published {
                        return Err(format!(
                            "retained file '{}' was not in the observed manifest",
                            logical
                        ));
                    }
                    files.insert(logical, entry);
                }
                PublishFile::Captured {
                    resource_id,
                    bytes,
                    source_mtime,
                    mode,
                } => {
                    if !content.resources.contains_key(&resource_id) {
                        return Err(format!(
                            "captured file '{}' belongs to missing resource '{}'",
                            logical, resource_id
                        ));
                    }
                    // Reused payloads bypass the store's write-time size
                    // check, so enforce the limit here for every entry.
                    if bytes.len() as u64 > domain::MAX_FILE_BYTES {
                        return Err(format!(
                            "captured file '{}' exceeds the write limit",
                            logical
                        ));
                    }
                    if mode.is_some_and(|mode| mode & !0o777 != 0) {
                        return Err(format!("captured file '{}' has unsafe mode bits", logical));
                    }
                    let digest = sha256(&bytes);
                    files.insert(
                        logical,
                        BundleFileEntry {
                            resource_id,
                            sha256: digest.clone(),
                            size: bytes.len() as u64,
                            source_mtime,
                            object_key: format!("_objects/{}", digest),
                            mode,
                        },
                    );
                    if !previous_shas.contains(digest.as_str()) {
                        pending.entry(digest).or_insert_with(|| Bytes::from(bytes));
                    }
                }
            }
        }

        let resource_content_sha256 = domain::compute_resource_content_digests(
            &content.resources,
            &content.conversations,
            &files,
            &content.directories,
            &content.tombstones,
        )?;
        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            generation,
            commit_id: commit_id.clone(),
            updated_at: request.updated_at,
            bundle: request.identity.clone(),
            recipe: request.recipe,
            captured_with: request.captured_with,
            published_by_origin: request.published_by_origin,
            resource_content_sha256,
            conversations: content.conversations,
            resources: content.resources,
            files,
            directories: content.directories,
            tombstones: content.tombstones,
        };
        manifest.validate()?;
        if let Some(previous) = &previous_manifest {
            validate_conversation_retention(previous, &manifest)?;
        }

        // A logically identical publish returns the observed generation
        // untouched: no payloads, no manifest, no commit, no head CAS. The
        // digests are recomputed from both full manifests here, never read
        // from a stored field, so a corrupt stored digest cannot spoof a
        // no-op.
        if let (Some((head, token)), Some(previous)) = (&current, &previous_manifest) {
            if domain::snapshot_content_sha256(previous)?
                == domain::snapshot_content_sha256(&manifest)?
            {
                return Ok(PublishedBundle {
                    published: false,
                    snapshot: BundleSnapshot {
                        storage_id: self.storage_id.clone(),
                        head: head.clone(),
                        manifest: previous.clone(),
                        fetched_at: request.updated_at,
                    },
                    head_token: token.clone(),
                });
            }
        }

        // Upload new payloads in bounded-concurrency batches, all strictly
        // before the manifest/commit/head writes that reference them.
        let total_objects = pending.len();
        let total_bytes: u64 = pending.values().map(|bytes| bytes.len() as u64).sum();
        let pending: Vec<(String, Bytes)> = pending.into_iter().collect();
        let mut uploaded_objects = 0_usize;
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
            // `AlreadyPresent` outcomes are success: the object was adopted
            // from an earlier interrupted publish of the same content.
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

        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|e| format!("serialize bundle manifest: {}", e))?;
        let manifest_sha256 = sha256(&manifest_bytes);
        let manifest_relative_key = format!("_manifests/{}-{}.json", generation, commit_id);
        let manifest_key =
            repository_object_key(&request.identity.bundle_id, &manifest_relative_key)?;
        self.store.put_immutable(&manifest_key, &manifest_bytes)?;

        // The generation record pins this manifest, its logical snapshot
        // digest, and the complete parent reference; the head then pins the
        // record's own bytes. Everything is immutable before the CAS makes
        // the generation visible.
        let record = domain::BundleGenerationRecord {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: request.identity.bundle_id.clone(),
            generation,
            commit_id: commit_id.clone(),
            manifest_sha256: manifest_sha256.clone(),
            snapshot_content_sha256: domain::snapshot_content_sha256(&manifest)?,
            previous: current.as_ref().map(|(head, _)| head.latest.clone()),
            created_at: request.updated_at,
            published_by_origin: manifest.published_by_origin.clone(),
            delta: generation_delta(previous_manifest.as_ref(), &manifest),
        };
        record.validate()?;
        let record_bytes = serde_json::to_vec(&record)
            .map_err(|e| format!("serialize generation record: {}", e))?;
        let record_key = repository_object_key(
            &request.identity.bundle_id,
            &format!("_versions/{}-{}.json", generation, commit_id),
        )?;
        self.store.put_immutable(&record_key, &record_bytes)?;

        let head = BundleHead {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: request.identity.bundle_id.clone(),
            kind: request.identity.kind,
            latest: record.version_ref(sha256(&record_bytes)),
            updated_at: request.updated_at,
        };
        head.validate()?;
        let head_bytes =
            serde_json::to_vec(&head).map_err(|e| format!("serialize bundle head: {}", e))?;
        let head_key = repository_object_key(&request.identity.bundle_id, HEAD_FILE)?;
        let head_etag =
            match self
                .store
                .compare_and_swap(&head_key, &cas_expectation, &head_bytes)?
            {
                CasOutcome::Written { etag } => etag,
                CasOutcome::Conflict { .. } => {
                    return Err(
                        "bundle head CAS conflict; fetch and rebase before retrying".to_string()
                    )
                }
            };
        // The tag is a derived discovery hint, never a second authority. A
        // head that was successfully CAS-published remains a successful
        // publish even if a contended tag update must be repaired later.
        let _ = self.publish_tag(&manifest);

        let snapshot = BundleSnapshot {
            storage_id: self.storage_id.clone(),
            head: head.clone(),
            manifest,
            fetched_at: request.updated_at,
        };
        Ok(PublishedBundle {
            published: true,
            snapshot,
            head_token: HeadToken {
                generation: head.latest.generation,
                commit_id: head.latest.commit_id.clone(),
                manifest_sha256: head.latest.manifest_sha256.clone(),
                etag: head_etag,
            },
        })
    }

    /// Whole-bundle download. Production paths plan and apply lazily via
    /// [`Self::byte_source`]; this remains the exhaustive test surface.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fetch(&self, bundle_id: &BundleId) -> Result<FetchedBundle, String> {
        self.fetch_with_progress(bundle_id, &mut |_| {})
    }

    /// Full-payload fetch reporting per-object progress. The totals come from
    /// the already-verified manifest, so observers can size progress bars
    /// before the first payload byte arrives.
    pub fn fetch_with_progress(
        &self,
        bundle_id: &BundleId,
        on_progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<FetchedBundle, String> {
        let snapshot = self.inspect(bundle_id)?;
        let total_objects = snapshot.manifest.files.len();
        let total_bytes = snapshot
            .manifest
            .files
            .values()
            .fold(0_u64, |sum, entry| sum.saturating_add(entry.size));
        let mut files = BTreeMap::new();
        let mut fetched_bytes = 0_u64;
        for (index, (logical_path, entry)) in snapshot.manifest.files.iter().enumerate() {
            let key = repository_object_key(bundle_id, &entry.object_key)?;
            let object = self
                .store
                .get(&key)?
                .ok_or_else(|| format!("bundle object '{}' is missing", key))?;
            if object.bytes.len() as u64 != entry.size || sha256(&object.bytes) != entry.sha256 {
                return Err(format!(
                    "bundle object '{}' does not match manifest size/hash",
                    key
                ));
            }
            fetched_bytes = fetched_bytes.saturating_add(entry.size);
            on_progress(FetchProgress {
                index: index + 1,
                total_objects,
                fetched_bytes,
                total_bytes,
            });
            files.insert(logical_path.clone(), object.bytes);
        }
        let dependency_actions = dependencies_from_manifest(&snapshot.manifest)?;
        Ok(FetchedBundle {
            snapshot,
            files,
            dependency_actions,
        })
    }

    /// Lazy payload access for `snapshot`, sharing verified objects through
    /// `cache` and reporting bulk downloads through `on_progress`.
    pub fn byte_source<'a>(
        &'a self,
        snapshot: &'a BundleVersionSnapshot,
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

    /// Read and verify one immutable generation record against the digest a
    /// verified parent (or the head) pins for it. The record key is derived
    /// canonically; a caller-supplied object key is never accepted.
    pub fn read_generation_record(
        &self,
        bundle_id: &BundleId,
        version: &BundleVersionRef,
    ) -> Result<domain::BundleGenerationRecord, String> {
        version.validate()?;
        let relative_key = format!(
            "_versions/{}-{}.json",
            version.generation, version.commit_id
        );
        let key = repository_object_key(bundle_id, &relative_key)?;
        let stored = self
            .store
            .get(&key)?
            .ok_or_else(|| format!("generation record '{}' is missing", key))?;
        if sha256(&stored.bytes) != version.generation_record_sha256 {
            return Err(format!(
                "generation record '{}' failed hash verification",
                key
            ));
        }
        let record: domain::BundleGenerationRecord =
            parse_bounded_json(&stored.bytes, "generation record")?;
        record.validate()?;
        if record.bundle_id != *bundle_id
            || record.generation != version.generation
            || record.commit_id != version.commit_id
            || record.manifest_sha256 != version.manifest_sha256
            || record.snapshot_content_sha256 != version.snapshot_content_sha256
        {
            return Err(format!(
                "generation record '{}' does not match its pinned coordinates",
                key
            ));
        }
        Ok(record)
    }

    /// Walk the verified reachable chain newest-first from `start`, calling
    /// `visit` for every hop until it returns `false` or generation 1 is
    /// reached. Contiguous strictly-descending generation numbers make
    /// cycles impossible; the depth bound is a malformed-data backstop far
    /// above any real history length, never a history-age limit. History is
    /// never recovered by listing `_versions/`: an orphaned CAS-loser
    /// record is unreachable from the head and therefore invisible here.
    fn walk_reachable_generations(
        &self,
        bundle_id: &BundleId,
        start: &BundleVersionRef,
        visit: &mut dyn FnMut(
            &BundleVersionRef,
            &domain::BundleGenerationRecord,
        ) -> Result<bool, String>,
    ) -> Result<(), String> {
        const MAX_CHAIN_TRAVERSAL: u64 = 100_000;
        let mut cursor = Some(start.clone());
        let mut hops = 0_u64;
        while let Some(version) = cursor {
            if hops >= MAX_CHAIN_TRAVERSAL {
                return Err("generation chain exceeds the traversal backstop".to_string());
            }
            hops = hops.saturating_add(1);
            let record = self.read_generation_record(bundle_id, &version)?;
            if !visit(&version, &record)? {
                return Ok(());
            }
            cursor = match record.previous {
                Some(previous) => {
                    // record.validate() already proved adjacency; repeat the
                    // check here so traversal safety never depends on a
                    // remote validation path.
                    if previous.generation.saturating_add(1) != version.generation {
                        return Err("generation chain is not contiguous".to_string());
                    }
                    Some(previous)
                }
                None => {
                    if version.generation != 1 {
                        return Err(format!(
                            "generation {} chain ends without reaching generation 1",
                            version.generation
                        ));
                    }
                    None
                }
            };
        }
        Ok(())
    }

    /// One page of the verified reachable history, newest first. `cursor` is
    /// the last generation of the previous page; rows strictly below it are
    /// returned. Every page walks from a freshly read head so an orphaned or
    /// tampered record can never enter a response.
    pub fn list_reachable_generations(
        &self,
        bundle_id: &BundleId,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<VerifiedGenerationPage, String> {
        if limit == 0 || limit > MAX_GENERATION_PAGE {
            return Err(format!(
                "generation page limit must be between 1 and {}",
                MAX_GENERATION_PAGE
            ));
        }
        let (head, _) = self
            .read_head(bundle_id)?
            .ok_or_else(|| format!("bundle '{}' has no published head", bundle_id))?;
        if cursor == Some(0) {
            return Err("generation cursor 0 has no predecessors".to_string());
        }
        let mut generations: Vec<VerifiedGeneration> = Vec::new();
        let mut next_cursor = None;
        self.walk_reachable_generations(bundle_id, &head.latest, &mut |version, record| {
            if cursor.is_some_and(|after| version.generation >= after) {
                return Ok(true);
            }
            generations.push(VerifiedGeneration {
                version: version.clone(),
                record: record.clone(),
            });
            if generations.len() == limit {
                if record.previous.is_some() {
                    next_cursor = Some(version.generation);
                }
                return Ok(false);
            }
            Ok(true)
        })?;
        Ok(VerifiedGenerationPage {
            generations,
            next_cursor,
            observed_head: head,
        })
    }

    /// Resolve requested generation numbers through the verified reachable
    /// chain in a single walk. Numbers are selectors, never trust anchors:
    /// each result carries the full hash-pinned reference the chain proved.
    pub fn resolve_reachable_generations(
        &self,
        bundle_id: &BundleId,
        generations: &BTreeSet<u64>,
    ) -> Result<(BTreeMap<u64, VerifiedGeneration>, BundleHead), String> {
        let (head, _) = self
            .read_head(bundle_id)?
            .ok_or_else(|| format!("bundle '{}' has no published head", bundle_id))?;
        let mut remaining = generations.clone();
        if remaining.is_empty() {
            return Ok((BTreeMap::new(), head));
        }
        if let Some(zero) = remaining.iter().find(|generation| **generation == 0) {
            return Err(format!("generation {} is not a valid selector", zero));
        }
        if let Some(future) = remaining
            .iter()
            .find(|generation| **generation > head.latest.generation)
        {
            return Err(format!(
                "generation {} is not reachable from the current head (latest is {})",
                future, head.latest.generation
            ));
        }
        let mut resolved = BTreeMap::new();
        self.walk_reachable_generations(bundle_id, &head.latest, &mut |version, record| {
            if remaining.remove(&version.generation) {
                resolved.insert(
                    version.generation,
                    VerifiedGeneration {
                        version: version.clone(),
                        record: record.clone(),
                    },
                );
            }
            Ok(!remaining.is_empty()
                && remaining
                    .iter()
                    .next()
                    .is_some_and(|lowest| *lowest < version.generation))
        })?;
        if let Some(unreachable) = remaining.iter().next() {
            return Err(format!(
                "generation {} is not reachable from the current head",
                unreachable
            ));
        }
        Ok((resolved, head))
    }

    /// Resolve one generation number; see `resolve_reachable_generations`.
    pub fn resolve_reachable_generation(
        &self,
        bundle_id: &BundleId,
        generation: u64,
    ) -> Result<(VerifiedGeneration, BundleHead), String> {
        let (mut resolved, head) =
            self.resolve_reachable_generations(bundle_id, &BTreeSet::from([generation]))?;
        let verified = resolved
            .remove(&generation)
            .ok_or_else(|| format!("generation {} is not reachable", generation))?;
        Ok((verified, head))
    }

    /// Load and fully verify the immutable manifest a version reference
    /// pins: exact manifest bytes, validated content, matching coordinates,
    /// and a recomputed logical snapshot digest. Payload objects are never
    /// touched.
    pub fn inspect_version(
        &self,
        bundle_id: &BundleId,
        version: &BundleVersionRef,
    ) -> Result<BundleManifest, String> {
        self.read_generation_record(bundle_id, version)?;
        let manifest = self.inspect_manifest_version(
            bundle_id,
            version.generation,
            &version.commit_id,
            &version.manifest_sha256,
        )?;
        let snapshot_digest = domain::snapshot_content_sha256(&manifest)?;
        if snapshot_digest != version.snapshot_content_sha256 {
            return Err(format!(
                "manifest for generation {} does not match its pinned snapshot digest",
                version.generation
            ));
        }
        Ok(manifest)
    }

    /// Compare two reachable generations from verified metadata only. The
    /// fast path answers equal snapshots straight from the hash-linked
    /// records; the detailed path reads exactly the two immutable manifests
    /// in one bulk metadata fetch and never touches `_objects/*` payloads.
    pub fn compare_generations(
        &self,
        bundle_id: &BundleId,
        left_generation: u64,
        right_generation: u64,
        include_unchanged: bool,
    ) -> Result<BundleGenerationComparison, String> {
        if left_generation == right_generation {
            return Err("comparison requires two distinct generations".to_string());
        }
        let (mut resolved, _head) = self.resolve_reachable_generations(
            bundle_id,
            &BTreeSet::from([left_generation, right_generation]),
        )?;
        let left = resolved
            .remove(&left_generation)
            .ok_or_else(|| format!("generation {} is not reachable", left_generation))?;
        let right = resolved
            .remove(&right_generation)
            .ok_or_else(|| format!("generation {} is not reachable", right_generation))?;

        if left.version.snapshot_content_sha256 == right.version.snapshot_content_sha256 {
            // Identical logical snapshots: both verified refs already carry
            // the digest, so no manifest or payload is fetched and there are
            // no per-resource rows to report.
            return Ok(BundleGenerationComparison {
                left: left.into(),
                right: right.into(),
                same_snapshot: true,
                added: 0,
                removed: 0,
                changed: 0,
                unchanged: 0,
                resource_changes: Vec::new(),
                bundle_metadata_changed: false,
                recipe_changed: false,
                codec_changed: false,
                tombstones_changed: false,
            });
        }

        let [left_manifest, right_manifest] =
            self.read_manifest_pair(bundle_id, &left.version, &right.version)?;

        let mut resource_changes = Vec::new();
        let (mut added, mut removed, mut changed, mut unchanged) = (0_u64, 0_u64, 0_u64, 0_u64);
        let resource_ids: BTreeSet<&ResourceId> = left_manifest
            .resource_content_sha256
            .keys()
            .chain(right_manifest.resource_content_sha256.keys())
            .collect();
        for resource_id in resource_ids {
            let left_digest = left_manifest.resource_content_sha256.get(resource_id);
            let right_digest = right_manifest.resource_content_sha256.get(resource_id);
            let state = match (left_digest, right_digest) {
                (None, Some(_)) => BundleResourceChangeState::Added,
                (Some(_), None) => BundleResourceChangeState::Removed,
                (Some(left), Some(right)) if left == right => BundleResourceChangeState::Unchanged,
                (Some(_), Some(_)) => BundleResourceChangeState::Changed,
                (None, None) => continue,
            };
            match state {
                BundleResourceChangeState::Added => added += 1,
                BundleResourceChangeState::Removed => removed += 1,
                BundleResourceChangeState::Changed => changed += 1,
                BundleResourceChangeState::Unchanged => unchanged += 1,
            }
            if state == BundleResourceChangeState::Unchanged && !include_unchanged {
                continue;
            }
            let descriptor = right_manifest
                .resources
                .get(resource_id)
                .or_else(|| left_manifest.resources.get(resource_id))
                .ok_or_else(|| {
                    format!(
                        "resource '{}' has a content digest but no descriptor",
                        resource_id
                    )
                })?;
            resource_changes.push(BundleResourceChange {
                resource_id: resource_id.clone(),
                kind: descriptor.kind,
                display_name: descriptor.display_name.clone(),
                left_thread_title: manifest_thread_title(&left_manifest, resource_id),
                right_thread_title: manifest_thread_title(&right_manifest, resource_id),
                state,
                left_content_sha256: left_digest.cloned(),
                right_content_sha256: right_digest.cloned(),
            });
        }

        Ok(BundleGenerationComparison {
            left: left.into(),
            right: right.into(),
            same_snapshot: false,
            added,
            removed,
            changed,
            unchanged,
            resource_changes,
            bundle_metadata_changed: left_manifest.bundle != right_manifest.bundle,
            recipe_changed: left_manifest.recipe != right_manifest.recipe,
            codec_changed: left_manifest.captured_with.codec_versions
                != right_manifest.captured_with.codec_versions,
            tombstones_changed: left_manifest.tombstones != right_manifest.tombstones,
        })
    }

    /// Fetch and fully verify two immutable manifests in one bounded
    /// metadata `get_many` round. Each manifest is checked against its
    /// pinned byte digest, validated, and re-hashed against the pinned
    /// logical snapshot digest.
    fn read_manifest_pair(
        &self,
        bundle_id: &BundleId,
        left: &BundleVersionRef,
        right: &BundleVersionRef,
    ) -> Result<[BundleManifest; 2], String> {
        let keys = [left, right]
            .into_iter()
            .map(|version| {
                repository_object_key(
                    bundle_id,
                    &format!(
                        "_manifests/{}-{}.json",
                        version.generation, version.commit_id
                    ),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let stored = self.store.get_many(&keys)?;
        let mut manifests = Vec::with_capacity(2);
        for (version, stored) in [left, right].into_iter().zip(stored) {
            let stored = stored.ok_or_else(|| {
                format!(
                    "bundle manifest for generation {} is missing",
                    version.generation
                )
            })?;
            if sha256(&stored.bytes) != version.manifest_sha256 {
                return Err(format!(
                    "bundle manifest for generation {} failed hash verification",
                    version.generation
                ));
            }
            let manifest: BundleManifest = parse_bounded_json(&stored.bytes, "bundle manifest")?;
            manifest.validate()?;
            if &manifest.bundle.bundle_id != bundle_id
                || manifest.generation != version.generation
                || manifest.commit_id != version.commit_id
            {
                return Err("historical manifest coordinates do not match its contents".to_string());
            }
            if domain::snapshot_content_sha256(&manifest)? != version.snapshot_content_sha256 {
                return Err(format!(
                    "manifest for generation {} does not match its pinned snapshot digest",
                    version.generation
                ));
            }
            manifests.push(manifest);
        }
        manifests
            .try_into()
            .map_err(|_| "manifest pair fetch returned the wrong arity".to_string())
    }

    /// Load one immutable historical manifest using the coordinates retained
    /// by a reviewed local sync base. This reads metadata only; payload
    /// objects such as conversation rollouts are not downloaded.
    pub fn inspect_manifest_version(
        &self,
        bundle_id: &BundleId,
        generation: u64,
        commit_id: &str,
        manifest_sha256: &str,
    ) -> Result<BundleManifest, String> {
        domain::validate_sha256("historical manifest", manifest_sha256)?;
        let relative_key = format!("_manifests/{generation}-{commit_id}.json");
        validate_bundle_relative_object_key(&relative_key)?;
        let key = repository_object_key(bundle_id, &relative_key)?;
        let stored = self
            .store
            .get(&key)?
            .ok_or_else(|| format!("bundle manifest '{}' is missing", key))?;
        if sha256(&stored.bytes) != manifest_sha256 {
            return Err(format!(
                "bundle manifest '{}' failed hash verification",
                key
            ));
        }
        let manifest: BundleManifest = parse_bounded_json(&stored.bytes, "bundle manifest")?;
        manifest.validate()?;
        if &manifest.bundle.bundle_id != bundle_id
            || manifest.generation != generation
            || manifest.commit_id != commit_id
        {
            return Err("historical manifest coordinates do not match its contents".to_string());
        }
        Ok(manifest)
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
                .get(&key)?
                .ok_or_else(|| format!("listed bundle tag '{}' disappeared", key))?;
            let tag: BundleTag = parse_bounded_json(&stored.bytes, "bundle tag")?;
            if tag.schema_version != BUNDLE_SCHEMA || tag.bundle_id != bundle_id {
                return Err(format!("bundle tag '{}' has inconsistent identity", key));
            }
            summaries.push(RemoteBundleSummary {
                bundle_id: tag.bundle_id,
                display_name: tag.display_name,
                kind: tag.kind,
                generation: tag.generation,
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

    /// Plan over a fully fetched bundle — the historic byte-verified entry
    /// point, retained for tests; production plans lazily.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn build_restore_plan(
        &self,
        bundle: &FetchedBundle,
        binding: &ProjectBinding,
        created_at: u64,
        expected_host_identity_revision: u64,
    ) -> Result<RestorePlan, String> {
        validate_bundle_snapshot_bytes(bundle)?;
        let mut bytes = PreloadedBytes::new(bundle);
        self.build_restore_plan_lazy(
            &bundle.snapshot.version_snapshot(),
            &bundle.snapshot.head.latest,
            &mut bytes,
            &BTreeMap::new(),
            binding,
            created_at,
            expected_host_identity_revision,
        )
    }

    /// Build a restore plan from the verified manifest, downloading payload
    /// bytes only for conversations whose materialized digest cannot be
    /// derived from `known_bases` and the manifest alone. In the steady
    /// state (every conversation covered by a reviewed base) this touches
    /// zero payload objects.
    pub fn build_restore_plan_lazy(
        &self,
        snapshot: &BundleVersionSnapshot,
        observed_head: &BundleVersionRef,
        bytes: &mut dyn BundleBytes,
        known_bases: &BTreeMap<ResourceId, KnownMaterializedBase>,
        binding: &ProjectBinding,
        created_at: u64,
        expected_host_identity_revision: u64,
    ) -> Result<RestorePlan, String> {
        snapshot.validate()?;
        self.validate_binding(binding, None)?;
        if binding.state != BindingState::Active {
            return Err("restore requires an active project binding".to_string());
        }
        observed_head.validate()?;
        if snapshot.version.generation > observed_head.generation {
            return Err("restore source cannot be newer than the observed head".to_string());
        }
        if &binding.bundle_id != snapshot.bundle_id() {
            return Err("binding and fetched bundle IDs differ".to_string());
        }
        if snapshot.storage_id != self.storage_id {
            return Err("fetched bundle belongs to another storage".to_string());
        }
        let plan_id = PlanId::parse(random_named_id("plan")?)?;
        let mut actions = Vec::new();
        let mut target_owners = BTreeMap::<String, LogicalPath>::new();
        let mut continuation_ids = BTreeSet::new();

        // Executable dependencies have their own immutable DependencyPlan.
        // Keeping plugin/installer placeholders out of RestorePlan prevents a
        // single logical resource from appearing twice in Pull review and
        // prevents the restore engine from reporting expected native work as
        // a blocked file action.
        // Custom skills are directory units: one typed install/overwrite
        // action per resource, never independently selectable file writes.
        let mut skill_targets = BTreeMap::<String, ResourceId>::new();
        let mut skill_capabilities = BTreeMap::<String, ResourceId>::new();
        for descriptor in snapshot.manifest.resources.values() {
            if descriptor.kind != ResourceKind::StandaloneSkill {
                continue;
            }
            let identity = custom_skill_identity(descriptor)?;
            let capability_key = format!(
                "{:?}:{}",
                identity.provider,
                identity.effective_name.to_ascii_lowercase()
            );
            if let Some(previous) =
                skill_capabilities.insert(capability_key, descriptor.resource_id.clone())
            {
                return Err(format!(
                    "custom skills '{}' and '{}' claim one effective name",
                    previous, descriptor.resource_id
                ));
            }
            let action = custom_skill_action(&snapshot.manifest, descriptor, binding)?;
            if let Some(target) = &action.target_path {
                // Case-folded so two snapshots cannot claim one materialized
                // directory on a case-insensitive filesystem.
                if let Some(previous) =
                    skill_targets.insert(target.to_lowercase(), descriptor.resource_id.clone())
                {
                    return Err(format!(
                        "custom skills '{}' and '{}' claim one target directory",
                        previous, descriptor.resource_id
                    ));
                }
            }
            actions.push(action);
        }
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

        // Files, pass 1: inspect targets and decide per file whether the
        // materialized digest is already known. Only conversations that
        // genuinely diverged from every reviewed base need payload bytes.
        let mut planned_files = Vec::new();
        let mut fetch_paths = Vec::new();
        for (logical_path, entry) in &snapshot.manifest.files {
            let descriptor = snapshot
                .manifest
                .resources
                .get(&entry.resource_id)
                .ok_or_else(|| format!("file '{}' has no resource", logical_path))?;
            if descriptor.kind == ResourceKind::StandaloneSkill {
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
            let target_state = match inspect_restore_target_digest(&target) {
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
            let keep_local_eligible =
                logical_path.as_str() != "state/codex/session_index.jsonl" && conversation_kind;
            let outcome = if !is_remapped_conversation(logical_path, descriptor) {
                // Materialized bytes equal stored bytes, so the manifest
                // digest is the materialized digest — no payload needed.
                if keep_local_eligible
                    && target_digest
                        .as_ref()
                        .is_some_and(|digest| digest != &entry.sha256)
                {
                    PlannedOutcome::KeepLocal
                } else {
                    PlannedOutcome::Action {
                        materialized: Some(entry.sha256.clone()),
                    }
                }
            } else {
                classify_remapped_conversation(
                    &entry.sha256,
                    known_bases
                        .get(&entry.resource_id)
                        .filter(|base| base.storage_digest == entry.sha256),
                    target_digest.as_deref(),
                )
            };
            if matches!(outcome, PlannedOutcome::NeedsBytes) {
                fetch_paths.push(logical_path.clone());
            }
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

        // Files, pass 2: bulk-load the divergent conversations (so progress
        // totals are real), then emit actions in manifest order.
        bytes.prefetch(&fetch_paths)?;
        for planned in planned_files {
            let logical_path = planned.logical_path;
            let entry = planned.entry;
            let descriptor = planned.descriptor;
            let materialized = match planned.outcome {
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
                PlannedOutcome::Action { materialized } => materialized,
                PlannedOutcome::NeedsBytes => {
                    let source_bytes = bytes.get(logical_path)?;
                    let materialized_bytes =
                        materialized_file_bytes(logical_path, descriptor, binding, source_bytes)?;
                    let materialized_sha256 = sha256(&materialized_bytes);
                    if planned.target_digest.as_ref().is_some_and(|digest| {
                        digest != &materialized_sha256 && digest != &entry.sha256
                    }) {
                        let target = planned.target.as_ref().expect("keep-local has a target");
                        actions.push(conversation_keep_local_action(
                            &entry.resource_id,
                            logical_path,
                            target,
                            &entry.sha256,
                            planned.target_digest.as_deref().unwrap(),
                        )?);
                        continue;
                    }
                    Some(materialized_sha256)
                }
            };
            let target = planned.target.as_ref().expect("file action has a target");
            let kind = restore_kind_for_file(logical_path, descriptor, entry)?;
            let requires_explicit_approval = match materialized.as_deref() {
                Some(materialized) => action_needs_explicit_approval(
                    descriptor,
                    planned.target_digest.as_deref(),
                    materialized,
                ),
                // Unknown materialized digest over an existing target cannot
                // be proven byte-identical, so it must be reviewed.
                None => {
                    planned.target_digest.is_some()
                        || action_needs_explicit_approval(descriptor, None, "")
                }
            };
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
                materialized_sha256: materialized,
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
                    let target_state = match inspect_restore_target_digest(&target) {
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
            bundle_id: snapshot.bundle_id().clone(),
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            expected_host_identity_revision,
            source_version: snapshot.version.clone(),
            observed_head: observed_head.clone(),
            binding_revision: binding.revision,
            created_at,
            expires_at: created_at.saturating_add(DEFAULT_PLAN_LIFETIME_SECS),
            actions,
            conversation_actions: Vec::new(),
            project_content_eligibility: None,
        };
        plan.validate()?;
        Ok(plan)
    }

    fn read_manifest(&self, head: &BundleHead) -> Result<BundleManifest, String> {
        head.validate()?;
        // Manifest keys are always derived canonically from the pinned
        // coordinates; a stored key is never trusted or even carried.
        let relative_key = format!(
            "_manifests/{}-{}.json",
            head.latest.generation, head.latest.commit_id
        );
        let key = repository_object_key(&head.bundle_id, &relative_key)?;
        let stored = self
            .store
            .get(&key)?
            .ok_or_else(|| format!("bundle manifest '{}' is missing", key))?;
        if sha256(&stored.bytes) != head.latest.manifest_sha256 {
            return Err(format!(
                "bundle manifest '{}' failed hash verification",
                key
            ));
        }
        let manifest: BundleManifest = parse_bounded_json(&stored.bytes, "bundle manifest")?;
        manifest.validate_against_head(head)?;
        Ok(manifest)
    }

    fn publish_tag(&self, manifest: &BundleManifest) -> Result<(), String> {
        let tag = BundleTag {
            schema_version: manifest.schema_version,
            bundle_id: manifest.bundle.bundle_id.clone(),
            display_name: manifest.bundle.display_name.clone(),
            kind: manifest.bundle.kind,
            generation: manifest.generation,
            updated_at: manifest.updated_at,
            resources: manifest.resources.len() as u64,
            files: manifest.files.len() as u64,
        };
        let bytes = serde_json::to_vec(&tag).map_err(|e| format!("serialize bundle tag: {}", e))?;
        let key = repository_object_key(&manifest.bundle.bundle_id, TAG_FILE)?;
        for _ in 0..4 {
            let expectation = match self.store.get(&key)? {
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
    use crate::project_sync_v3::domain::{
        LocalProjectId, RecipeEntry, ReplicaId, RestoreActionType,
    };
    use crate::project_sync_v3::global_inventory::GlobalPluginSource;
    use crate::project_sync_v3::provider_capture::{
        capture_recipe, capture_selected, discover_project, CaptureRequest, CaptureResourceKind,
        Provider as CaptureProvider,
    };

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

    fn test_publish_origin() -> PortableOrigin {
        PortableOrigin {
            origin_host_token: domain::OriginHostToken::parse("7a".repeat(32)).unwrap(),
            origin_replica_token: domain::OriginReplicaToken::parse("8b".repeat(32)).unwrap(),
            shared_label: None,
        }
    }

    /// Capture-shaped publish used by engine tests: reconciles the capture
    /// against the observed head's manifest exactly like production Push.
    fn publish_capture<S: BundleObjectStore>(
        engine: &BundleEngine<S>,
        identity: BundleIdentity,
        recipe: BundleRecipe,
        captured: CapturedResources,
        observed_head: Option<(BundleHead, HeadToken)>,
        updated_at: u64,
    ) -> Result<PublishedBundle, String> {
        publish_capture_with_progress(
            engine,
            identity,
            recipe,
            captured,
            observed_head,
            updated_at,
            &mut |_| {},
        )
    }

    fn publish_capture_with_progress<S: BundleObjectStore>(
        engine: &BundleEngine<S>,
        identity: BundleIdentity,
        recipe: BundleRecipe,
        captured: CapturedResources,
        observed_head: Option<(BundleHead, HeadToken)>,
        updated_at: u64,
        on_progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<PublishedBundle, String> {
        let previous = observed_head
            .as_ref()
            .map(|(head, _)| engine.read_manifest(head))
            .transpose()?;
        let origin = test_publish_origin();
        let content =
            build_publish_content(&recipe, &captured, previous.as_ref(), &origin, updated_at)?;
        let mut captured_with = captured_with();
        for thread in content.conversations.values() {
            captured_with
                .codec_versions
                .entry(thread.lineage_codec.clone())
                .or_insert(1);
        }
        engine.publish_with_progress(
            PublishBundleRequest {
                identity,
                recipe,
                captured_with,
                published_by_origin: origin,
                content,
                observed_head,
                updated_at,
            },
            on_progress,
        )
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
    fn codex_session_index_merge_is_approved_with_conversations_by_default() {
        let index = ResourceDescriptor {
            resource_id: ResourceId::parse("codex:session-index").unwrap(),
            kind: ResourceKind::CodexConversation,
            provider: Some(Provider::Codex),
            scope: domain::ResourceScope::ProviderState,
            display_name: "Codex project session index".to_string(),
            provenance: domain::Provenance::Unknown,
            apply_policy: ApplyPolicy::Merge,
            relative_cwd: None,
            codec_version: 1,
            metadata: BTreeMap::new(),
        };
        assert!(!action_needs_explicit_approval(
            &index,
            Some("target-differs"),
            "incoming"
        ));

        let mut ordinary_merge = index;
        ordinary_merge.resource_id = ResourceId::parse("memory:claude").unwrap();
        ordinary_merge.kind = ResourceKind::ProjectMemory;
        assert!(action_needs_explicit_approval(
            &ordinary_merge,
            Some("target-differs"),
            "incoming"
        ));
    }

    #[test]
    fn local_store_enforces_immutable_objects_and_head_cas() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalBundleObjectStore::open(temp.path().join("store")).unwrap();
        let key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{}/_objects/{}",
            bundle_id(1),
            "a".repeat(64)
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
        assert_eq!(store.get(&head).unwrap().unwrap().bytes, b"second");
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
    fn object_keys_outside_the_repository_namespace_are_rejected() {
        assert!(ObjectKey::parse("v3/bundles/0123456789abcdef0123456789abcdef/_tag.json").is_err());
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
        let first = publish_capture(
            &engine,
            identity(id.clone(), "Project"),
            recipe.clone(),
            captured,
            None,
            10,
        )
        .unwrap();
        assert_eq!(first.snapshot.head.latest.generation, 1);
        let repository_root = store_root.join(format!("{}/{}", REPOSITORY_PREFIX, id));
        assert!(store_root.join(STORAGE_MARKER_KEY).is_file());
        assert!(repository_root.join(HEAD_FILE).is_file());
        assert!(repository_root.join(TAG_FILE).is_file());
        assert!(repository_root.join("_manifests").is_dir());
        assert!(repository_root.join("_versions").is_dir());
        assert!(repository_root.join("_objects").is_dir());
        let fetched = engine.fetch(&id).unwrap();
        assert_eq!(fetched.files.values().next().unwrap(), b"version one");

        write(&project.join("CLAUDE.md"), b"version two");
        let captured = capture_recipe(&request, &recipe).unwrap();
        let second = publish_capture(
            &engine,
            identity(id.clone(), "Renamed Project"),
            recipe.clone(),
            captured,
            Some((first.snapshot.head.clone(), first.head_token.clone())),
            20,
        )
        .unwrap();
        assert_eq!(second.snapshot.head.latest.generation, 2);
        assert!(publish_capture(
            &engine,
            identity(id.clone(), "Stale"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first.head_token)),
            21,
        )
        .is_err());

        let empty_recipe = BundleRecipe::default();
        let removed = publish_capture(
            &engine,
            identity(id.clone(), "Renamed Project"),
            empty_recipe.clone(),
            capture_recipe(&request, &empty_recipe).unwrap(),
            Some((second.snapshot.head.clone(), second.head_token)),
            30,
        )
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
                generation: 1,
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
        publish_capture(
            &engine,
            identity(id.clone(), "Portable project"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, None, None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
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
        let result = engine
            .apply_restore_plan(
                &fetched,
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

    /// Store wrapper recording every `get`/`get_many` key and every write so
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
                        && !key.contains("/_versions/")
                })
                .count()
        }
    }

    impl BundleObjectStore for CountingStore {
        fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
            self.keys.lock().unwrap().push(key.as_str().to_string());
            self.inner.get(key)
        }

        fn get_many(&self, keys: &[ObjectKey]) -> Result<Vec<Option<StoredObject>>, String> {
            let mut recorded = self.keys.lock().unwrap();
            for key in keys {
                recorded.push(key.as_str().to_string());
            }
            drop(recorded);
            self.inner.get_many(keys)
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
        let first = publish_capture(
            &engine,
            identity(id.clone(), "Incremental"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        assert!(first.published);
        assert_eq!(store.payload_puts().len(), 2);
        for entry in first.snapshot.manifest.files.values() {
            assert_eq!(entry.object_key, format!("_objects/{}", entry.sha256));
        }

        write(&source.join("changing.md"), b"second draft");
        store.reset();
        let second = publish_capture(
            &engine,
            identity(id.clone(), "Incremental"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first.head_token.clone())),
            20,
        )
        .unwrap();
        assert!(second.published);
        assert_eq!(second.snapshot.head.latest.generation, 2);
        let changed_key =
            repository_object_key(&id, &format!("_objects/{}", sha256(b"second draft"))).unwrap();
        assert_eq!(
            store.payload_puts(),
            vec![changed_key.as_str().to_string()],
            "only the changed payload uploads"
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
        let published = publish_capture(
            &engine,
            identity(id.clone(), "Dedupe"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        assert_eq!(store.payload_puts().len(), 1);
        let shared_key = format!("_objects/{}", sha256(b"same bytes either way"));
        let file_entries: Vec<_> = published.snapshot.manifest.files.values().collect();
        assert_eq!(file_entries.len(), 2);
        for entry in file_entries {
            assert_eq!(entry.object_key, shared_key);
        }
    }

    /// Three generations published through the production capture path,
    /// returning (engine, store root, bundle id, published outcomes).
    fn publish_three_generations(
        temp: &tempfile::TempDir,
    ) -> (
        BundleEngine<LocalBundleObjectStore>,
        std::path::PathBuf,
        BundleId,
        Vec<PublishedBundle>,
    ) {
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let id = bundle_id(31);
        let store_root = temp.path().join("store");
        let store = LocalBundleObjectStore::open(&store_root).unwrap();
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let mut outcomes = Vec::new();
        for (index, body) in [
            &b"generation one"[..],
            &b"generation two"[..],
            &b"generation three"[..],
        ]
        .iter()
        .enumerate()
        {
            write(&project.join("CLAUDE.md"), body);
            let inventory = discover_project(&request).unwrap();
            let resource_id = inventory
                .resources
                .iter()
                .find(|resource| resource.display_name == "CLAUDE.md")
                .unwrap()
                .resource_id
                .clone();
            let recipe = recipe_for([resource_id]);
            let observed = outcomes.last().map(|previous: &PublishedBundle| {
                (previous.snapshot.head.clone(), previous.head_token.clone())
            });
            let published = publish_capture(
                &engine,
                identity(id.clone(), "Chained"),
                recipe.clone(),
                capture_recipe(&request, &recipe).unwrap(),
                observed,
                10 + index as u64 * 10,
            )
            .unwrap();
            assert!(published.published);
            outcomes.push(published);
        }
        (engine, store_root, id, outcomes)
    }

    #[test]
    fn generation_chain_lists_resolves_and_paginates_newest_first() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, _root, id, outcomes) = publish_three_generations(&temp);

        let page = engine.list_reachable_generations(&id, None, 10).unwrap();
        assert_eq!(
            page.generations
                .iter()
                .map(|generation| generation.version.generation)
                .collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.observed_head.latest.generation, 3);
        assert_eq!(page.observed_head.latest, outcomes[2].snapshot.head.latest);
        // The chain pins parents completely: each record names the previous
        // version with all three digests.
        assert_eq!(
            page.generations[0].record.previous.as_ref().unwrap(),
            &page.generations[1].version
        );
        assert_eq!(
            page.generations[1].record.previous.as_ref().unwrap(),
            &page.generations[2].version
        );
        assert!(page.generations[2].record.previous.is_none());

        // Pagination without duplicates or gaps.
        let first = engine.list_reachable_generations(&id, None, 2).unwrap();
        assert_eq!(
            first
                .generations
                .iter()
                .map(|generation| generation.version.generation)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert_eq!(first.next_cursor, Some(2));
        let second = engine
            .list_reachable_generations(&id, first.next_cursor, 2)
            .unwrap();
        assert_eq!(
            second
                .generations
                .iter()
                .map(|generation| generation.version.generation)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(second.next_cursor, None);

        // Resolution by number returns the exact pinned version.
        for generation in 1..=3_u64 {
            let (verified, head) = engine
                .resolve_reachable_generation(&id, generation)
                .unwrap();
            assert_eq!(verified.version.generation, generation);
            assert_eq!(head.latest.generation, 3);
            let manifest = engine.inspect_version(&id, &verified.version).unwrap();
            assert_eq!(manifest.generation, generation);
            assert_eq!(
                domain::snapshot_content_sha256(&manifest).unwrap(),
                verified.version.snapshot_content_sha256
            );
        }

        // Invalid selectors are rejected without listing anything.
        assert!(engine
            .resolve_reachable_generation(&id, 0)
            .unwrap_err()
            .contains("not a valid selector"));
        assert!(engine
            .resolve_reachable_generation(&id, 4)
            .unwrap_err()
            .contains("not reachable"));
    }

    #[test]
    fn generation_chain_fails_closed_on_tampered_or_missing_records() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, store_root, id, _outcomes) = publish_three_generations(&temp);
        let two = engine.resolve_reachable_generation(&id, 2).unwrap().0;
        let record_path = store_root.join(format!(
            "{}/{}/_versions/2-{}.json",
            REPOSITORY_PREFIX, id, two.version.commit_id
        ));

        // Tampered bytes: the digest pinned by generation 3 no longer
        // matches, so listing and resolving both fail rather than truncate.
        let original = fs::read(&record_path).unwrap();
        let mut tampered_json: serde_json::Value = serde_json::from_slice(&original).unwrap();
        tampered_json["created_at"] = serde_json::json!(999);
        fs::write(&record_path, serde_json::to_vec(&tampered_json).unwrap()).unwrap();
        assert!(engine
            .list_reachable_generations(&id, None, 10)
            .unwrap_err()
            .contains("hash verification"));
        assert!(engine
            .resolve_reachable_generation(&id, 1)
            .unwrap_err()
            .contains("hash verification"));

        // Missing parent record: same failure mode, never a shortened chain.
        fs::remove_file(&record_path).unwrap();
        assert!(engine
            .list_reachable_generations(&id, None, 10)
            .unwrap_err()
            .contains("is missing"));

        fs::write(&record_path, original).unwrap();
        assert_eq!(
            engine
                .list_reachable_generations(&id, None, 10)
                .unwrap()
                .generations
                .len(),
            3
        );
    }

    #[test]
    fn selected_generation_apply_writes_the_selected_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, _root, id, outcomes) = publish_three_generations(&temp);
        let target = temp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let binding = active_binding(id.clone(), &target, None, None);

        // The head stays at G3 while G1 is reviewed and applied.
        let (verified, head) = engine.resolve_reachable_generation(&id, 1).unwrap();
        assert_eq!(head.latest.generation, 3);
        let manifest = engine.inspect_version(&id, &verified.version).unwrap();
        let snapshot = domain::BundleVersionSnapshot {
            storage_id: storage_id(),
            version: verified.version.clone(),
            manifest,
            fetched_at: 50,
        };
        let mut plan_bytes = engine.byte_source(&snapshot, None, None);
        let plan = engine
            .build_restore_plan_lazy(
                &snapshot,
                &head.latest,
                &mut plan_bytes,
                &BTreeMap::new(),
                &binding,
                50,
                0,
            )
            .unwrap();
        assert_eq!(plan.source_version, verified.version);
        assert_eq!(plan.observed_head, head.latest);
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
        let mut apply_bytes = engine.byte_source(&snapshot, None, None);
        engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut apply_bytes,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                51,
            )
            .unwrap();
        // The selected generation's payload was written, not the head's.
        assert_eq!(
            fs::read(target.join("CLAUDE.md")).unwrap(),
            b"generation one"
        );

        // A tampered plan whose source no longer matches the chain rejects
        // apply even with the head unchanged.
        let mut forged = plan.clone();
        forged.source_version.generation = 2;
        forged.source_version.commit_id = outcomes[1].snapshot.head.latest.commit_id.clone();
        forged.source_version.manifest_sha256 =
            outcomes[1].snapshot.head.latest.manifest_sha256.clone();
        forged.source_version.snapshot_content_sha256 = "a".repeat(64);
        forged.source_version.generation_record_sha256 = "b".repeat(64);
        let mut forged_bytes = engine.byte_source(&snapshot, None, None);
        let forged_error = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut forged_bytes,
                &binding,
                &forged,
                &approved,
                &temp.path().join("backups"),
                52,
            )
            .unwrap_err();
        assert!(
            forged_error.contains("no longer matches bundle/binding state")
                || forged_error.contains("no longer reachable")
                || forged_error.contains("hash verification"),
            "{forged_error}"
        );
    }

    #[test]
    fn comparison_reads_only_metadata_and_classifies_every_state() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let id = bundle_id(41);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();

        let resource_named = |request: &CaptureRequest, name: &str| {
            discover_project(request)
                .unwrap()
                .resources
                .iter()
                .find(|resource| resource.display_name == name)
                .map(|resource| resource.resource_id.clone())
        };

        // G1: two resources. G2: one changed, one kept. G3: the kept one is
        // deselected (removed) while the changed one stays identical.
        write(&project.join("CLAUDE.md"), b"claude v1");
        write(&project.join("AGENTS.md"), b"agents v1");
        let claude = resource_named(&request, "CLAUDE.md").unwrap();
        let agents = resource_named(&request, "AGENTS.md").unwrap();
        let both = recipe_for([claude.clone(), agents.clone()]);
        let first = publish_capture(
            &engine,
            identity(id.clone(), "Compared"),
            both.clone(),
            capture_recipe(&request, &both).unwrap(),
            None,
            10,
        )
        .unwrap();
        write(&project.join("CLAUDE.md"), b"claude v2");
        let second = publish_capture(
            &engine,
            identity(id.clone(), "Compared"),
            both.clone(),
            capture_recipe(&request, &both).unwrap(),
            Some((first.snapshot.head.clone(), first.head_token.clone())),
            20,
        )
        .unwrap();
        let only_claude = recipe_for([claude.clone()]);
        let third = publish_capture(
            &engine,
            identity(id.clone(), "Compared"),
            only_claude.clone(),
            capture_recipe(&request, &only_claude).unwrap(),
            Some((second.snapshot.head.clone(), second.head_token.clone())),
            30,
        )
        .unwrap();
        assert_eq!(third.snapshot.head.latest.generation, 3);

        store.reset();
        let comparison = engine.compare_generations(&id, 1, 2, true).unwrap();
        assert!(!comparison.same_snapshot);
        assert_eq!(
            (
                comparison.added,
                comparison.removed,
                comparison.changed,
                comparison.unchanged
            ),
            (0, 0, 1, 1)
        );
        let changed_row = comparison
            .resource_changes
            .iter()
            .find(|row| row.state == BundleResourceChangeState::Changed)
            .unwrap();
        assert_eq!(changed_row.resource_id.as_str(), claude.as_str());
        assert_ne!(
            changed_row.left_content_sha256,
            changed_row.right_content_sha256
        );
        assert!(comparison
            .resource_changes
            .iter()
            .any(|row| row.state == BundleResourceChangeState::Unchanged));
        assert!(!comparison.bundle_metadata_changed);
        assert!(!comparison.recipe_changed);

        // Removal direction and its reversal.
        let removal = engine.compare_generations(&id, 2, 3, true).unwrap();
        assert_eq!((removal.added, removal.removed), (0, 1));
        assert!(removal.recipe_changed);
        assert!(removal.tombstones_changed);
        let reversed = engine.compare_generations(&id, 3, 2, true).unwrap();
        assert_eq!((reversed.added, reversed.removed), (1, 0));

        // include_unchanged = false keeps counts but omits unchanged rows.
        let compact = engine.compare_generations(&id, 1, 2, false).unwrap();
        assert_eq!(compact.unchanged, 1);
        assert!(compact
            .resource_changes
            .iter()
            .all(|row| row.state != BundleResourceChangeState::Unchanged));

        // Every comparison above read heads, generation records, and at most
        // two immutable manifests per call — never a payload object.
        assert_eq!(store.payload_gets(), 0);

        // One detailed comparison reads exactly one head, the records down
        // to the lower requested generation, and both manifests.
        store.reset();
        engine.compare_generations(&id, 2, 3, true).unwrap();
        let keys = store.keys.lock().unwrap().clone();
        assert_eq!(
            keys.iter()
                .filter(|key| key.ends_with("_head.json"))
                .count(),
            1
        );
        assert_eq!(
            keys.iter()
                .filter(|key| key.contains("/_versions/"))
                .count(),
            2
        );
        assert_eq!(
            keys.iter()
                .filter(|key| key.contains("/_manifests/"))
                .count(),
            2
        );
        assert_eq!(store.payload_gets(), 0);
    }

    #[test]
    fn cas_loser_orphans_are_invisible_to_history() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, store_root, id, outcomes) = publish_three_generations(&temp);
        // A concurrent publisher that lost the head CAS left behind immutable
        // generation-4 objects under a different commit id.
        let orphan_commit = "commit-cas-loser";
        let orphan = domain::BundleGenerationRecord {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: id.clone(),
            generation: 4,
            commit_id: orphan_commit.to_string(),
            manifest_sha256: "a".repeat(64),
            snapshot_content_sha256: "b".repeat(64),
            previous: Some(outcomes[2].snapshot.head.latest.clone()),
            created_at: 99,
            published_by_origin: test_publish_origin(),
            delta: domain::GenerationDelta::default(),
        };
        let orphan_path = store_root.join(format!(
            "{}/{}/_versions/4-{}.json",
            REPOSITORY_PREFIX, id, orphan_commit
        ));
        fs::create_dir_all(orphan_path.parent().unwrap()).unwrap();
        fs::write(&orphan_path, serde_json::to_vec(&orphan).unwrap()).unwrap();

        // The orphan exists in storage but is unreachable from the head, so
        // it cannot be listed or resolved.
        let page = engine.list_reachable_generations(&id, None, 10).unwrap();
        assert_eq!(
            page.generations
                .iter()
                .map(|generation| generation.version.generation)
                .collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert!(engine
            .resolve_reachable_generation(&id, 4)
            .unwrap_err()
            .contains("not reachable"));
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
        let first = publish_capture(
            &engine,
            identity(id.clone(), "Noop"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        assert!(first.published);

        store.reset();
        let noop = publish_capture(
            &engine,
            identity(id.clone(), "Noop"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first.head_token.clone())),
            20,
        )
        .unwrap();
        assert!(!noop.published);
        assert_eq!(noop.snapshot.head.latest.generation, 1);
        assert_eq!(noop.head_token, first.head_token);
        assert_eq!(store.put_count(), 0, "a semantic no-op writes no objects");
        assert_eq!(
            store.cas_count(),
            0,
            "a semantic no-op never touches the head"
        );
        assert_eq!(engine.inspect(&id).unwrap().head.latest.generation, 1);
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
        let first = publish_capture_with_progress(
            &engine,
            identity(id.clone(), "Progress"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
            &mut |progress| events.push(progress),
        )
        .unwrap();
        let full_bytes = (b"tiny".len() + b"a considerably longer payload".len()) as u64;
        assert_eq!(events.len(), 2);
        assert_eq!(events.last().unwrap().index, 2);
        assert_eq!(events.last().unwrap().total_objects, 2);
        assert_eq!(events.last().unwrap().fetched_bytes, full_bytes);
        assert_eq!(events.last().unwrap().total_bytes, full_bytes);

        events.clear();
        let noop = publish_capture_with_progress(
            &engine,
            identity(id.clone(), "Progress"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first.head_token.clone())),
            20,
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
        publish_capture_with_progress(
            &engine,
            identity(id.clone(), "Progress"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first.head_token.clone())),
            30,
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
    fn lazy_plan_and_apply_touch_only_needed_payload_objects() {
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
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        publish_capture(
            &engine,
            identity(id.clone(), "Lazy project"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let binding = active_binding(id.clone(), &target, None, None);

        store.reset();
        let snapshot = engine.inspect(&id).unwrap();
        let observed_head = snapshot.head.latest.clone();
        let snapshot = snapshot.version_snapshot();
        let mut plan_bytes = engine.byte_source(&snapshot, None, None);
        let plan = engine
            .build_restore_plan_lazy(
                &snapshot,
                &observed_head,
                &mut plan_bytes,
                &BTreeMap::new(),
                &binding,
                100,
                0,
            )
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
        let observed_head = snapshot.head.latest.clone();
        let snapshot = snapshot.version_snapshot();
        let mut plan_bytes = engine.byte_source(&snapshot, None, None);
        let plan = engine
            .build_restore_plan_lazy(
                &snapshot,
                &observed_head,
                &mut plan_bytes,
                &BTreeMap::new(),
                &binding,
                102,
                0,
            )
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
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
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
        publish_capture(
            &engine,
            identity(id.clone(), "Claude project"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, None, Some(&target_claude));
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
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
        engine
            .apply_restore_plan(
                &fetched,
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

    #[test]
    fn codex_session_materialization_rebinds_cwd_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let source_child = source.join("apps/web");
        let target = temp.path().join("target");
        let target_child = target.join("apps/web");
        let source_codex = temp.path().join("source-codex");
        let target_codex = temp.path().join("target-codex");
        fs::create_dir_all(&source_child).unwrap();
        fs::create_dir_all(&target_child).unwrap();
        fs::create_dir_all(&source_codex).unwrap();
        fs::create_dir_all(&target_codex).unwrap();

        let source_cwd = serde_json::to_string(source_child.to_str().unwrap()).unwrap();
        let historical_text = format!("keep historical path {}", source_child.display());
        let transcript = format!(
            concat!(
                "{{\"timestamp\":\"2026-07-18T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-a\",\"cwd\":{source_cwd}}}}}\r\n",
                "{{\"type\":\"response_item\",\"payload\":{{\"text\":{historical_text}}}}}\n",
                "{{\"type\":\"turn_context\",\"payload\":{{\"cwd\":{source_cwd},\"workspace_roots\":[{source_cwd}]}}}}\n"
            ),
            source_cwd = source_cwd,
            historical_text = serde_json::to_string(&historical_text).unwrap(),
        );
        let session_relative = "sessions/2026/07/18/rollout-2026-07-18T00-00-00-thread-a.jsonl";
        write(&source_codex.join(session_relative), transcript.as_bytes());

        let request = CaptureRequest {
            project_root: source,
            codex_home: Some(source_codex),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: Vec::new(),
            global_plugins: Vec::new(),
            blocked_global_skills: Vec::new(),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(&request).unwrap();
        let resource = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "codex:session:thread-a")
            .unwrap();
        assert_eq!(resource.relative_cwd.as_deref(), Some("apps/web"));
        let resource_id = resource.resource_id.clone();
        let mut recipe = recipe_for([resource_id.clone()]);
        recipe
            .entries
            .get_mut(&ResourceId::parse(resource_id).unwrap())
            .unwrap()
            .apply_policy = ApplyPolicy::SafeFile;

        let id = bundle_id(8);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_capture(
            &engine,
            identity(id.clone(), "Codex project"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let binding = active_binding(id, &target, Some(&target_codex), None);

        // A pre-existing target file whose bytes are not the materialized form
        // is unknown content: restoring over it needs explicit approval.
        let target_session = target_codex.join(session_relative);
        write(&target_session, transcript.as_bytes());
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        assert!(plan
            .actions
            .iter()
            .all(|action| !matches!(action.kind, RestoreActionKind::Manual { .. })));
        let action = plan
            .actions
            .iter()
            .find(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::MaterializeConversation {
                        provider: Provider::Codex,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(action.requires_explicit_approval);

        let approved = BTreeSet::from([action.action_id.clone()]);
        let result = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(result.backups.len(), 1);

        let restored = fs::read(&target_session).unwrap();
        let rows = String::from_utf8(restored.clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let target_cwd = target_child.to_str().unwrap();
        assert_eq!(rows[0]["payload"]["cwd"], target_cwd);
        assert_eq!(rows[2]["payload"]["cwd"], target_cwd);
        assert_eq!(rows[1]["payload"]["text"], historical_text);
        assert_eq!(
            rows[2]["payload"]["workspace_roots"][0],
            source_child.to_str().unwrap(),
            "only structural cwd fields are rebound"
        );

        let repeat = engine
            .build_restore_plan(&fetched, &binding, 102, 0)
            .unwrap();
        let repeat_action = repeat
            .actions
            .iter()
            .find(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::MaterializeConversation {
                        provider: Provider::Codex,
                        ..
                    }
                )
            })
            .unwrap();
        let restored_sha = sha256(&restored);
        assert_eq!(
            repeat_action.expected_target_sha256.as_deref(),
            Some(restored_sha.as_str())
        );
        assert!(!repeat_action.requires_explicit_approval);
    }

    #[test]
    fn plugin_install_intent_exists_only_in_the_dependency_plan_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        let mut request = CaptureRequest::for_project(&source);
        request.global_plugins.push(GlobalPluginSource {
            provider: CaptureProvider::Codex,
            plugin_id: "computer-use@openai-bundled".to_string(),
            marketplace: Some("openai-bundled".to_string()),
            source_type: None,
            source: None,
            observed_version: Some("1.0.0".to_string()),
            enabled: true,
            provided_skills: vec!["computer-use".to_string()],
        });
        let inventory = discover_project(&request).unwrap();
        let plugin = inventory
            .resources
            .iter()
            .find(|resource| resource.kind == CaptureResourceKind::Plugin)
            .unwrap();
        let recipe = recipe_for([plugin.resource_id.clone()]);
        let id = bundle_id(51);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_capture(
            &engine,
            identity(id.clone(), "Plugin intent"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        assert_eq!(fetched.dependency_actions.len(), 1);
        assert!(matches!(
            fetched.dependency_actions[0].kind,
            DependencyActionKind::InstallCodexPlugin
        ));
        let binding = active_binding(id, &target, None, None);
        let restore = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        assert!(restore.actions.is_empty());
    }

    /// Machine-A provider home with one global skill, ready for capture via
    /// the global inventory adapter.
    fn global_skill_fixture(temp: &Path) -> (PathBuf, CaptureRequest) {
        let project = temp.join("project");
        let codex_home = temp.join("codex-home");
        fs::create_dir_all(&project).unwrap();
        let skill = codex_home.join("skills/deploy");
        write(
            &skill.join("SKILL.md"),
            b"---\nname: deploy\n---\nDeploy helper\n",
        );
        write(&skill.join("bin/run.sh"), b"#!/bin/sh\necho deploy\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(skill.join("bin/run.sh"), fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let inventory = super::super::global_inventory::inventory_provider_home(
            CaptureProvider::Codex,
            &codex_home,
        );
        assert_eq!(inventory.standalone_skills.len(), 1);
        let request = CaptureRequest {
            project_root: project,
            codex_home: Some(codex_home.clone()),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: inventory.standalone_skills,
            global_plugins: inventory.plugins,
            blocked_global_skills: inventory.blocked_skills,
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        (codex_home, request)
    }

    fn publish_skill_bundle(
        temp: &Path,
        request: &CaptureRequest,
        id: BundleId,
    ) -> BundleEngine<LocalBundleObjectStore> {
        let recipe = recipe_for(["codex:standalone-skill:deploy".to_string()]);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_capture(
            &engine,
            identity(id, "Global skill"),
            recipe.clone(),
            capture_recipe(request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        engine
    }

    #[test]
    fn custom_skill_installs_into_mapped_provider_home() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(21);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        // Machine B: fresh project and provider home.
        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let install = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .expect("one custom-skill install action");
        assert!(install.requires_explicit_approval);
        assert!(install.expected_target_sha256.is_none());
        assert!(!plan
            .actions
            .iter()
            .any(|action| matches!(action.kind, RestoreActionKind::WriteFile { .. })));

        // Keep is the default: an unapproved action mutates nothing.
        let skipped = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &BTreeSet::new(),
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(skipped
            .receipts
            .iter()
            .all(|receipt| receipt.status == ActionStatus::Skipped));
        assert!(!target_home.join("skills/deploy").exists());

        let approved = plan
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                102,
            )
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::InstallCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Applied);
        assert!(receipt.target_sha256_after.is_some());
        assert_eq!(
            fs::read(target_home.join("skills/deploy/bin/run.sh")).unwrap(),
            b"#!/bin/sh\necho deploy\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(target_home.join("skills/deploy/bin/run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit survives install");
            assert_eq!(mode & 0o7000, 0, "set-id bits are stripped");
        }

        // Idempotent replan: a matching target is an install no-op pinned to
        // the same digest.
        let replan = engine
            .build_restore_plan(&fetched, &binding, 103, 0)
            .unwrap();
        let noop = replan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .unwrap();
        assert_eq!(noop.expected_target_sha256, noop.source_sha256);
    }

    #[test]
    fn custom_skill_overwrite_is_digest_pinned_and_backed_up() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(22);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        // A different local skill already occupies the target, including an
        // extra file the cloud version does not carry.
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nLocal variant\n",
        );
        write(&target_home.join("skills/deploy/local-note.txt"), b"mine");
        let binding = active_binding(id, &target_project, Some(&target_home), None);

        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let overwrite = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::OverwriteCustomSkill { .. }))
            .expect("a different target requires an explicit overwrite");
        assert!(overwrite.expected_target_sha256.is_some());
        assert_ne!(overwrite.expected_target_sha256, overwrite.source_sha256);

        let approved = BTreeSet::from([overwrite.action_id.clone()]);
        let backups_root = temp.path().join("backups");
        let applied = engine
            .apply_restore_plan(&fetched, &binding, &plan, &approved, &backups_root, 101)
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::OverwriteCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Applied);
        // Whole-directory replacement removes files deleted by the cloud
        // version instead of leaving stale code behind.
        assert!(!target_home.join("skills/deploy/local-note.txt").exists());
        assert_eq!(
            fs::read(target_home.join("skills/deploy/SKILL.md")).unwrap(),
            b"---\nname: deploy\n---\nDeploy helper\n"
        );
        // The displaced directory is recoverable from the plan backup.
        assert_eq!(applied.backups.len(), 1);
        let backup_dir = PathBuf::from(&applied.backups[0].backup_path);
        assert_eq!(
            fs::read(backup_dir.join("local-note.txt")).unwrap(),
            b"mine"
        );
    }

    #[test]
    fn custom_skill_target_change_after_planning_aborts_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        let id = bundle_id(23);
        let engine = publish_skill_bundle(temp.path(), &request, id.clone());
        let fetched = engine.fetch(&id).unwrap();

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nLocal variant\n",
        );
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let overwrite = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::OverwriteCustomSkill { .. }))
            .unwrap();

        // The target changes between review and apply.
        write(
            &target_home.join("skills/deploy/SKILL.md"),
            b"---\nname: deploy\n---\nEdited after review\n",
        );
        let approved = BTreeSet::from([overwrite.action_id.clone()]);
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        let receipt = applied
            .receipts
            .iter()
            .find(|receipt| receipt.action_type == RestoreActionType::OverwriteCustomSkill)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Failed);
        assert!(receipt
            .error
            .as_deref()
            .unwrap()
            .contains("changed after planning"));
        assert_eq!(
            fs::read(target_home.join("skills/deploy/SKILL.md")).unwrap(),
            b"---\nname: deploy\n---\nEdited after review\n"
        );
    }

    #[test]
    fn custom_skill_preserves_install_directory_when_declared_name_differs() {
        let temp = tempfile::tempdir().unwrap();
        let source_project = temp.path().join("source-project");
        let source_home = temp.path().join("source-codex");
        fs::create_dir_all(&source_project).unwrap();
        write(
            &source_home.join("skills/capture-lsservice-detail/SKILL.md"),
            b"---\nname: get-real-hardware-rh-service\n---\nRun ~/.codex/skills/capture-lsservice-detail/scripts/run.py\n",
        );
        write(
            &source_home.join("skills/capture-lsservice-detail/scripts/run.py"),
            b"print('ok')\n",
        );
        let inventory = super::super::global_inventory::inventory_provider_home(
            CaptureProvider::Codex,
            &source_home,
        );
        assert!(inventory.blocked_skills.is_empty());
        assert_eq!(inventory.standalone_skills.len(), 1);
        let request = CaptureRequest {
            project_root: source_project,
            codex_home: Some(source_home),
            claude_home: None,
            excluded_project_roots: Vec::new(),
            standalone_skills: inventory.standalone_skills,
            global_plugins: inventory.plugins,
            blocked_global_skills: inventory.blocked_skills,
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let resource_id = "codex:standalone-skill:get-real-hardware-rh-service";
        let recipe = recipe_for([resource_id.to_string()]);
        let id = bundle_id(24);
        let engine = BundleEngine::open(
            LocalBundleObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_capture(
            &engine,
            identity(id.clone(), "Differently named skill"),
            recipe.clone(),
            capture_recipe(&request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let fetched = engine.fetch(&id).unwrap();
        let descriptor = fetched
            .snapshot
            .manifest
            .resources
            .get(&ResourceId::parse(resource_id).unwrap())
            .unwrap();
        assert_eq!(
            descriptor
                .metadata
                .get("effective_name")
                .map(String::as_str),
            Some("get-real-hardware-rh-service")
        );
        assert_eq!(
            descriptor
                .metadata
                .get("install_dir_name")
                .map(String::as_str),
            Some("capture-lsservice-detail")
        );

        let target_project = temp.path().join("target-project");
        let target_home = temp.path().join("target-codex");
        fs::create_dir_all(&target_project).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let binding = active_binding(id, &target_project, Some(&target_home), None);
        let plan = engine
            .build_restore_plan(&fetched, &binding, 100, 0)
            .unwrap();
        let install = plan
            .actions
            .iter()
            .find(|action| matches!(action.kind, RestoreActionKind::InstallCustomSkill { .. }))
            .unwrap();
        match &install.kind {
            RestoreActionKind::InstallCustomSkill { skill_name, .. } => {
                assert_eq!(skill_name, "get-real-hardware-rh-service")
            }
            _ => unreachable!(),
        }
        assert!(install
            .target_path
            .as_deref()
            .unwrap()
            .ends_with("skills/capture-lsservice-detail"));

        let approved = BTreeSet::from([install.action_id.clone()]);
        let applied = engine
            .apply_restore_plan(
                &fetched,
                &binding,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert!(applied
            .receipts
            .iter()
            .any(|receipt| receipt.status == ActionStatus::Applied));
        assert!(target_home
            .join("skills/capture-lsservice-detail/SKILL.md")
            .is_file());
        assert!(!target_home
            .join("skills/get-real-hardware-rh-service")
            .exists());
    }

    #[test]
    fn unselected_global_resources_never_enter_the_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let (_, request) = global_skill_fixture(temp.path());
        // Discovery sees the skill, but the recipe selects nothing global.
        let captured = capture_selected(&request, &BTreeSet::new()).unwrap();
        assert!(captured.resources.is_empty());
        assert!(captured.files.is_empty());
        assert!(captured.dependency_actions.is_empty());
    }
}

const MAX_SKILL_TREE_FILES: usize = 4_096;
const MAX_SKILL_TREE_BYTES: u64 = 512 * 1024 * 1024;

/// The effective identity of a custom-skill resource: bound provider plus
/// the runtime-visible skill name recorded at capture. The physical install
/// directory is tracked independently because valid skills may declare a
/// different runtime name and may refer to their installed path internally.
struct CustomSkillIdentity {
    provider: Provider,
    effective_name: String,
    install_dir_name: String,
}

fn custom_skill_identity(descriptor: &ResourceDescriptor) -> Result<CustomSkillIdentity, String> {
    let provider = descriptor
        .provider
        .ok_or_else(|| format!("custom skill '{}' lacks a provider", descriptor.resource_id))?;
    let effective_name = descriptor
        .metadata
        .get("effective_name")
        .cloned()
        .unwrap_or_else(|| descriptor.display_name.clone());
    let install_dir_name = descriptor
        .metadata
        .get("install_dir_name")
        .cloned()
        // Schema-3 bundles created under adapter v1 used one name for both
        // identity and installation, so preserve that interpretation.
        .unwrap_or_else(|| effective_name.clone());
    domain::validate_skill_name("custom skill effective name", &effective_name)?;
    domain::validate_skill_name("custom skill install directory", &install_dir_name)?;
    Ok(CustomSkillIdentity {
        provider,
        effective_name,
        install_dir_name,
    })
}

fn custom_skill_logical_root(provider: Provider, install_dir_name: &str) -> String {
    let provider = match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    };
    format!("state/{}/skills/{}", provider, install_dir_name)
}

/// Manifest files belonging to one custom skill, keyed by their path relative
/// to the skill directory.
fn custom_skill_manifest_files<'a>(
    manifest: &'a BundleManifest,
    resource_id: &ResourceId,
    logical_root: &str,
) -> Result<Vec<(String, &'a LogicalPath, &'a BundleFileEntry)>, String> {
    let prefix = format!("{}/", logical_root);
    let mut files = Vec::new();
    for (logical_path, entry) in &manifest.files {
        if &entry.resource_id != resource_id {
            continue;
        }
        let relative = logical_path.as_str().strip_prefix(&prefix).ok_or_else(|| {
            format!(
                "custom skill file '{}' is outside its skill root '{}'",
                logical_path, logical_root
            )
        })?;
        files.push((relative.to_string(), logical_path, entry));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn entry_is_executable(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o111 != 0)
}

/// Canonical tree digest over relative path, file type/executability, and
/// content hash. Computable both from a manifest and from a live directory so
/// "already installed" and "changed after review" are byte-grounded claims.
fn skill_tree_digest(entries: &[(String, bool, String)]) -> String {
    let mut material = String::new();
    for (relative, executable, digest) in entries {
        material.push_str(relative);
        material.push('\0');
        material.push(if *executable { 'x' } else { 'r' });
        material.push('\0');
        material.push_str(digest);
        material.push('\n');
    }
    sha256(material.as_bytes())
}

fn custom_skill_source_digest(files: &[(String, &LogicalPath, &BundleFileEntry)]) -> String {
    let entries = files
        .iter()
        .map(|(relative, _, entry)| {
            (
                relative.clone(),
                entry_is_executable(entry.mode),
                entry.sha256.clone(),
            )
        })
        .collect::<Vec<_>>();
    skill_tree_digest(&entries)
}

enum SkillTargetState {
    Missing,
    Present { digest: String },
    Blocked { reason: String },
}

/// No-follow scan of a live skill directory. Symlinks, special files, and
/// oversized trees block classification instead of being read through.
fn custom_skill_target_state(dir: &Path) -> Result<SkillTargetState, String> {
    inspect_existing_ancestors_no_symlink(dir)?;
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SkillTargetState::Missing)
        }
        Err(error) => return Err(format!("inspect '{}': {}", dir.display(), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(SkillTargetState::Blocked {
            reason: "target is not a regular directory".to_string(),
        });
    }
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(dir).follow_links(false).max_depth(16) {
        let entry = entry.map_err(|error| format!("walk '{}': {}", dir.display(), error))?;
        let file_type = entry.file_type();
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Ok(SkillTargetState::Blocked {
                reason: format!("'{}' is not a regular file", entry.path().display()),
            });
        }
        let relative = entry
            .path()
            .strip_prefix(dir)
            .map_err(|_| "skill tree escaped its root".to_string())?
            .to_str()
            .ok_or_else(|| "skill tree contains a non-UTF-8 path".to_string())?
            .replace('\\', "/");
        let meta = entry
            .metadata()
            .map_err(|error| format!("inspect '{}': {}", entry.path().display(), error))?;
        total_bytes = total_bytes.saturating_add(meta.len());
        if entries.len() >= MAX_SKILL_TREE_FILES || total_bytes > MAX_SKILL_TREE_BYTES {
            return Ok(SkillTargetState::Blocked {
                reason: "target skill tree is too large to verify".to_string(),
            });
        }
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read '{}': {}", entry.path().display(), error))?;
        let executable = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        entries.push((relative, executable, sha256(&bytes)));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(SkillTargetState::Present {
        digest: skill_tree_digest(&entries),
    })
}

/// Resolve the provider-home target directory for one custom skill.
fn custom_skill_target_dir(
    binding: &ProjectBinding,
    provider: Provider,
    install_dir_name: &str,
) -> Result<PathBuf, String> {
    let home = match provider {
        Provider::Codex => canonical_bound_home(binding.codex_home.as_deref(), "Codex")?,
        Provider::Claude => canonical_bound_home(binding.claude_home.as_deref(), "Claude")?,
    };
    safe_lexical_join(&home, &format!("skills/{}", install_dir_name))
}

/// Build the single typed action for one custom-skill resource: Install for a
/// missing or matching target, Overwrite pinned to the current target digest
/// for a different one, Manual when the target cannot be classified.
fn custom_skill_action(
    manifest: &BundleManifest,
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<RestoreAction, String> {
    let identity = custom_skill_identity(descriptor)?;
    let logical_root = custom_skill_logical_root(identity.provider, &identity.install_dir_name);
    let files = custom_skill_manifest_files(manifest, &descriptor.resource_id, &logical_root)?;
    if files.is_empty() {
        return manual_action(
            &descriptor.resource_id,
            &format!(
                "Custom skill '{}' has no payload in this generation",
                identity.effective_name
            ),
        );
    }
    let target =
        match custom_skill_target_dir(binding, identity.provider, &identity.install_dir_name) {
            Ok(target) => target,
            Err(error) => {
                return manual_action(
                    &descriptor.resource_id,
                    &format!(
                        "Custom skill '{}' needs a mapped provider home: {}",
                        identity.effective_name, error
                    ),
                )
            }
        };
    let source_digest = custom_skill_source_digest(&files);
    let state = custom_skill_target_state(&target)?;
    let (kind, expected) = match state {
        SkillTargetState::Missing => (
            RestoreActionKind::InstallCustomSkill {
                provider: identity.provider,
                skill_name: identity.effective_name.clone(),
            },
            None,
        ),
        SkillTargetState::Present { digest } => {
            let kind = if digest == source_digest {
                RestoreActionKind::InstallCustomSkill {
                    provider: identity.provider,
                    skill_name: identity.effective_name.clone(),
                }
            } else {
                RestoreActionKind::OverwriteCustomSkill {
                    provider: identity.provider,
                    skill_name: identity.effective_name.clone(),
                }
            };
            (kind, Some(digest))
        }
        SkillTargetState::Blocked { reason } => {
            return manual_action(
                &descriptor.resource_id,
                &format!(
                    "Custom skill target '{}' cannot be replaced automatically: {}",
                    target.display(),
                    reason
                ),
            )
        }
    };
    let action = RestoreAction {
        action_id: action_id_for(&descriptor.resource_id, &logical_root, kind.action_type())?,
        resource_id: descriptor.resource_id.clone(),
        kind,
        target_path: Some(path_text(&target)?),
        source_sha256: Some(source_digest),
        expected_target_sha256: expected,
        expected_target_mode: None,
        materialized_sha256: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
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
        && (action.resource_id.as_str().starts_with("codex:session:")
            || action.resource_id.as_str().starts_with("claude:session:"))
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

fn materialized_file_bytes(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
    source: &[u8],
) -> Result<Vec<u8>, String> {
    let logical = logical_path.as_str();
    if descriptor.kind == ResourceKind::CodexConversation
        && (logical.starts_with("state/codex/sessions/")
            || logical.starts_with("state/codex/archived_sessions/"))
    {
        return remap_codex_session_cwd(source, descriptor, binding);
    }
    Ok(source.to_vec())
}

/// Codex scopes its default resume picker to the `cwd` stored in a session's
/// structural metadata. A portable conversation therefore needs those fields
/// rebound to this replica's project path when it is materialized. Only the
/// top-level provider records are changed; user messages and historical text
/// remain byte-for-byte untouched.
fn remap_codex_session_cwd(
    source: &[u8],
    descriptor: &ResourceDescriptor,
    binding: &ProjectBinding,
) -> Result<Vec<u8>, String> {
    if descriptor.relative_cwd.is_none() {
        return Err(format!(
            "Codex session '{}' has no project-relative cwd",
            descriptor.display_name
        ));
    }
    let target_cwd = path_text(&remapped_resource_cwd(descriptor, binding)?)?;
    let mut output = Vec::with_capacity(source.len());
    let mut found_session_meta = false;

    for line in source.split_inclusive(|byte| *byte == b'\n') {
        let (json, ending): (&[u8], &[u8]) = if let Some(json) = line.strip_suffix(b"\r\n") {
            (json, b"\r\n")
        } else if let Some(json) = line.strip_suffix(b"\n") {
            (json, b"\n")
        } else {
            (line, b"")
        };
        if json.is_empty() {
            output.extend_from_slice(line);
            continue;
        }
        if !contains_bytes(json, b"session_meta") && !contains_bytes(json, b"turn_context") {
            output.extend_from_slice(line);
            continue;
        }

        let mut value: serde_json::Value = match serde_json::from_slice(json) {
            Ok(value) => value,
            Err(_) => {
                output.extend_from_slice(line);
                continue;
            }
        };
        let record_type = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if !matches!(
            record_type.as_deref(),
            Some("session_meta" | "turn_context")
        ) {
            output.extend_from_slice(line);
            continue;
        }
        let Some(cwd) = value
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|payload| payload.get_mut("cwd"))
            .filter(|cwd| cwd.is_string())
        else {
            output.extend_from_slice(line);
            continue;
        };
        if record_type.as_deref() == Some("session_meta") {
            found_session_meta = true;
        }
        if cwd.as_str() == Some(target_cwd.as_str()) {
            output.extend_from_slice(line);
            continue;
        }
        *cwd = serde_json::Value::String(target_cwd.clone());
        output.extend_from_slice(
            &serde_json::to_vec(&value)
                .map_err(|error| format!("serialize remapped Codex session row: {}", error))?,
        );
        output.extend_from_slice(ending);
    }

    if !found_session_meta {
        return Err(format!(
            "Codex session '{}' has no remappable session_meta cwd",
            descriptor.display_name
        ));
    }
    Ok(output)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn restore_kind_for_file(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    entry: &BundleFileEntry,
) -> Result<RestoreActionKind, String> {
    if logical_path.as_str() == "state/codex/session_index.jsonl" {
        return Ok(RestoreActionKind::MergeFile {
            logical_path: logical_path.clone(),
        });
    }
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
    // The Codex index is a derived, project-filtered list. Restore composes it
    // by stable session ID, preserves target-only rows, and creates a backup,
    // so it is safe to accompany the already-default conversation restores.
    if descriptor.resource_id.as_str() == "codex:session-index" {
        return false;
    }
    matches!(
        descriptor.apply_policy,
        ApplyPolicy::ExplicitInstall
            | ApplyPolicy::ExplicitReview
            | ApplyPolicy::ManualOnly
            | ApplyPolicy::Merge
    ) || target_digest.is_some_and(|target| target != source_digest)
        || matches!(
            descriptor.kind,
            ResourceKind::StandaloneSkill | ResourceKind::Plugin
        )
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
    if let Some(relative) = logical.strip_prefix("state/codex/sessions/") {
        let home = canonical_bound_home(binding.codex_home.as_deref(), "Codex")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("sessions/{}", relative),
        )?));
    }
    if let Some(relative) = logical.strip_prefix("state/codex/archived_sessions/") {
        let home = canonical_bound_home(binding.codex_home.as_deref(), "Codex")?;
        return Ok(Some(safe_lexical_join(
            &home,
            &format!("archived_sessions/{}", relative),
        )?));
    }
    if logical == "state/codex/session_index.jsonl" {
        return Ok(Some(
            canonical_bound_home(binding.codex_home.as_deref(), "Codex")?
                .join("session_index.jsonl"),
        ));
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
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
}

/// Digest-and-mode-only variant of [`inspect_restore_target`] for planning:
/// the target is hashed streaming instead of being read into memory, so a
/// plan over many large conversations never holds their bytes at once.
fn inspect_restore_target_digest(path: &Path) -> Result<RestoreTargetState, String> {
    inspect_existing_ancestors_no_symlink(path)?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RestoreTargetState {
                digest: None,
                bytes: None,
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
    if meta.len() > MAX_OBJECT_BYTES as u64 {
        return Err(format!("restore target '{}' is too large", path.display()));
    }
    let mut file = fs::File::open(path)
        .map_err(|e| format!("read restore target '{}': {}", path.display(), e))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("read restore target '{}': {}", path.display(), e))?;
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(&mut digest, "{:02x}", byte);
    }
    Ok(RestoreTargetState {
        digest: Some(digest),
        bytes: None,
        mode: Some(safe_mode(&meta)),
    })
}

fn inspect_restore_target(path: &Path) -> Result<RestoreTargetState, String> {
    inspect_existing_ancestors_no_symlink(path)?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RestoreTargetState {
                digest: None,
                bytes: None,
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
    if meta.len() > MAX_OBJECT_BYTES as u64 {
        return Err(format!("restore target '{}' is too large", path.display()));
    }
    let bytes =
        fs::read(path).map_err(|e| format!("read restore target '{}': {}", path.display(), e))?;
    Ok(RestoreTargetState {
        digest: Some(sha256(&bytes)),
        bytes: Some(bytes),
        mode: Some(safe_mode(&meta)),
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

pub(crate) fn write_target_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if bytes.len() > MAX_OBJECT_BYTES || mode & !0o777 != 0 {
        return Err("unsafe target bytes or mode".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("target '{}' has no parent", path.display()))?;
    let base = nearest_existing_directory(parent)?;
    create_safe_directory_tree(&base, parent)?;
    inspect_existing_ancestors_no_symlink(path)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create target temp in '{}': {}", parent.display(), e))?;
    temp.as_file_mut()
        .write_all(bytes)
        .map_err(|e| format!("write target '{}': {}", path.display(), e))?;
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
    temp.persist(path)
        .map_err(|e| format!("publish target '{}': {}", path.display(), e.error))?;
    Ok(())
}

pub(crate) fn write_immutable_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Ok(existing) = fs::read(path) {
        return if existing == bytes {
            Ok(())
        } else {
            Err(format!(
                "backup '{}' already exists with different bytes",
                path.display()
            ))
        };
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("backup '{}' has no parent", path.display()))?;
    let base = nearest_existing_directory(parent)?;
    create_safe_directory_tree(&base, parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create backup temp: {}", e))?;
    temp.as_file_mut()
        .write_all(bytes)
        .map_err(|e| format!("write backup '{}': {}", path.display(), e))?;
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
    snapshot: &BundleVersionSnapshot,
    binding: &ProjectBinding,
    storage_id: &StorageId,
    now: u64,
) -> Result<(), String> {
    if now < plan.created_at || now > plan.expires_at {
        return Err("restore plan has expired or is not active yet".to_string());
    }
    if &plan.storage_id != storage_id
        || plan.storage_id != snapshot.storage_id
        || &plan.bundle_id != snapshot.bundle_id()
        || plan.host_id != binding.host_id
        || plan.replica_id != binding.replica_id
        || plan.bundle_id != binding.bundle_id
        || plan.source_version != snapshot.version
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

fn merge_jsonl_index(existing: Option<&[u8]>, incoming: &[u8]) -> Result<Vec<u8>, String> {
    let mut records = BTreeMap::<String, Vec<u8>>::new();
    for (origin, bytes) in [
        ("target", existing.unwrap_or_default()),
        ("bundle", incoming),
    ] {
        for raw in bytes.split(|byte| *byte == b'\n') {
            if raw.is_empty() {
                continue;
            }
            if raw.len() > 1024 * 1024 {
                return Err(format!("{} session-index row exceeds one MiB", origin));
            }
            let value: serde_json::Value = serde_json::from_slice(raw)
                .map_err(|e| format!("parse {} session-index row: {}", origin, e))?;
            let identity = json_identity(&value).unwrap_or_else(|| sha256(raw));
            if let Some(previous) = records.get(&identity) {
                if previous != raw && origin == "target" {
                    continue;
                }
            }
            // Bundle contribution wins for the same stable thread ID; the
            // target-only rows remain untouched.
            records.insert(identity, raw.to_vec());
        }
    }
    let mut output = Vec::new();
    for row in records.into_values() {
        output.extend_from_slice(&row);
        output.push(b'\n');
    }
    Ok(output)
}

fn json_identity(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in ["thread_id", "threadId", "session_id", "sessionId", "id"] {
                if let Some(value) = map.get(key).and_then(serde_json::Value::as_str) {
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
            map.values().find_map(json_identity)
        }
        serde_json::Value::Array(values) => values.iter().find_map(json_identity),
        _ => None,
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

fn parse_bounded_json<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    label: &str,
) -> Result<T, String> {
    if bytes.len() > MAX_OBJECT_BYTES {
        return Err(format!("{} exceeds the JSON read limit", label));
    }
    serde_json::from_slice(bytes).map_err(|e| format!("parse {}: {}", label, e))
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
        || value.starts_with("_versions/")
        || value.starts_with("_objects/"))
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

/// Build the fully reconciled candidate content for a production Push from
/// one provider capture: previous content carries forward for selected but
/// uncaptured resources, deselected resources gain tombstones, and every
/// captured provider session with a recoverable identity publishes portable
/// [`ConversationThread`] metadata (title, lineage, digests) so history
/// listing and comparison never open payload objects.
pub fn build_publish_content(
    recipe: &BundleRecipe,
    captured: &CapturedResources,
    previous: Option<&BundleManifest>,
    publisher: &PortableOrigin,
    updated_at: u64,
) -> Result<PublishContent, String> {
    build_publish_content_with_thread_bindings(
        recipe,
        captured,
        previous,
        publisher,
        updated_at,
        &BTreeMap::new(),
    )
}

/// Build publish content while preserving the portable thread identity of
/// provider-native sessions already materialized on this replica. An initial
/// Push has no bindings and falls back to the deterministic portable ID;
/// subsequent Pushes must keep the bound ID even when Pull assigned a new
/// provider-native session ID locally.
pub fn build_publish_content_with_thread_bindings(
    recipe: &BundleRecipe,
    captured: &CapturedResources,
    previous: Option<&BundleManifest>,
    publisher: &PortableOrigin,
    updated_at: u64,
    bound_thread_ids: &BTreeMap<(Provider, String), ThreadId>,
) -> Result<PublishContent, String> {
    let captured_descriptors = provider_capture::domain_resources(captured)?;
    // Only fetch materializes dependency actions, but invalid captured
    // dependency intent must still fail the publish, not the next pull.
    provider_capture::domain_dependency_actions(captured)?;
    let (resources, retained, mut directories, mut tombstones) = reconcile_manifest_content(
        recipe,
        &captured_descriptors,
        captured,
        previous,
        updated_at,
    )?;

    let mut files: BTreeMap<LogicalPath, PublishFile> = retained
        .into_iter()
        .map(|(path, entry)| (path, PublishFile::Retained(entry)))
        .collect();
    for (logical_path, file) in &captured.files {
        let logical = LogicalPath::parse(logical_path.clone())?;
        let resource_id = ResourceId::parse(file.resource_id.clone())?;
        if !recipe.entries.contains_key(&resource_id) {
            return Err(format!(
                "captured file '{}' belongs to unselected resource '{}'",
                logical, resource_id
            ));
        }
        files.insert(
            logical,
            PublishFile::Captured {
                resource_id,
                bytes: file.bytes.clone(),
                source_mtime: file.source_mtime,
                mode: Some(file.mode & 0o777),
            },
        );
    }
    for (logical_path, directory) in &captured.directories {
        let logical = LogicalPath::parse(logical_path.clone())?;
        let resource_id = ResourceId::parse(directory.resource_id.clone())?;
        if !recipe.entries.contains_key(&resource_id) {
            return Err(format!(
                "captured directory '{}' belongs to unselected resource '{}'",
                logical, resource_id
            ));
        }
        directories.insert(
            logical,
            BundleDirectoryEntry {
                resource_id,
                mode: Some(directory.mode & 0o777),
                source_mtime: directory.source_mtime,
            },
        );
    }

    let conversations = reconcile_conversation_threads(
        captured,
        &resources,
        &files,
        previous,
        publisher,
        bound_thread_ids,
    )?;

    // A thread that left the live set (its resource was deselected) is an
    // explicit deletion; one that returned sheds its old tombstone.
    if let Some(previous) = previous {
        for (thread_id, thread) in &previous.conversations {
            if conversations.contains_key(thread_id) {
                continue;
            }
            let last_sha256 = thread
                .files
                .iter()
                .find(|file| file.role == ConversationFileRole::Transcript)
                .and_then(|file| previous.files.get(&file.logical_path))
                .map(|entry| entry.sha256.clone());
            let tombstone = Tombstone {
                target: TombstoneTarget::ConversationThread {
                    thread_id: thread_id.clone(),
                },
                last_sha256,
                deleted_at: updated_at,
            };
            tombstones.insert(tombstone.canonical_key(), tombstone);
        }
    }
    tombstones.retain(|_, tombstone| {
        !matches!(
            &tombstone.target,
            TombstoneTarget::ConversationThread { thread_id }
                if conversations.contains_key(thread_id)
        )
    });

    Ok(PublishContent {
        conversations,
        resources,
        files,
        directories,
        tombstones,
    })
}

/// The provider-native session identity for one captured conversation
/// resource, from codec analysis when available and from the capture
/// resource key otherwise. Derived resources such as the Codex session
/// index have neither and publish as plain files.
fn captured_conversation_source_id<'a>(
    resource_key: &'a str,
    conversation: &'a provider_capture::CapturedConversation,
) -> Option<&'a str> {
    if let Some(source) = conversation.source_thread_id.as_deref() {
        return Some(source);
    }
    for prefix in ["codex:session:", "claude:session:"] {
        if let Some(suffix) = resource_key.strip_prefix(prefix) {
            if !suffix.is_empty() {
                return Some(suffix);
            }
        }
    }
    None
}

/// Portable thread metadata for the candidate snapshot: fresh entries for
/// captured sessions, carried-forward entries for live but uncaptured
/// resources. Thread identity is stable across replicas, so continuity
/// fields (creation, fork provenance) survive recapture.
fn reconcile_conversation_threads(
    captured: &CapturedResources,
    resources: &BTreeMap<ResourceId, ResourceDescriptor>,
    files: &BTreeMap<LogicalPath, PublishFile>,
    previous: Option<&BundleManifest>,
    publisher: &PortableOrigin,
    bound_thread_ids: &BTreeMap<(Provider, String), ThreadId>,
) -> Result<BTreeMap<ThreadId, ConversationThread>, String> {
    let mut threads = BTreeMap::new();
    // One pass over the candidate files: every payload is hashed exactly
    // once, then grouped by owner. Anything per-conversation after this is
    // a map lookup, so reconcile cost stays linear in captured bytes no
    // matter how many sessions a project carries.
    let mut files_by_owner: BTreeMap<&ResourceId, Vec<(&LogicalPath, String, u64)>> =
        BTreeMap::new();
    for (path, file) in files {
        let (owner, sha256_value, mtime) = match file {
            PublishFile::Retained(entry) => {
                (&entry.resource_id, entry.sha256.clone(), entry.source_mtime)
            }
            PublishFile::Captured {
                resource_id,
                bytes,
                source_mtime,
                ..
            } => (resource_id, sha256(bytes), *source_mtime),
        };
        files_by_owner
            .entry(owner)
            .or_default()
            .push((path, sha256_value, mtime));
    }
    for (resource_key, conversation) in &captured.conversations {
        let Some(source_thread_id) = captured_conversation_source_id(resource_key, conversation)
        else {
            continue;
        };
        let resource_id = ResourceId::parse(resource_key.clone())?;
        if !resources.contains_key(&resource_id) {
            continue;
        }
        let provider = provider_capture::domain_provider(conversation.provider);
        let thread_id = bound_thread_ids
            .get(&(provider, source_thread_id.to_string()))
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| {
                domain::portable_conversation_thread_id(provider, source_thread_id)
            })?;
        let transcript_path = LogicalPath::parse(conversation.transcript_logical_path.clone())?;

        let mut file_refs = Vec::new();
        let mut transcript_sha256 = None;
        let mut latest_mtime = 0_u64;
        for (path, sha256_value, mtime) in files_by_owner.get(&resource_id).into_iter().flatten() {
            let role = if *path == &transcript_path {
                transcript_sha256 = Some(sha256_value.clone());
                ConversationFileRole::Transcript
            } else {
                ConversationFileRole::Sidecar {
                    sidecar_kind: "file".to_string(),
                }
            };
            latest_mtime = latest_mtime.max(*mtime);
            file_refs.push(ConversationFileRef {
                logical_path: (*path).clone(),
                role,
            });
        }
        if transcript_sha256.is_none() {
            return Err(format!(
                "captured conversation '{}' lacks its transcript '{}'",
                resource_key, transcript_path
            ));
        }

        let (lineage_status, head_message_id, message_count, lineage_digest, semantic_digest) =
            match &conversation.lineage {
                CodecLineage::Complete {
                    chain,
                    lineage_digest,
                    semantic_digest,
                } => (
                    ConversationLineageStatus::Complete,
                    chain.last().map(|message| message.message_id.clone()),
                    Some(chain.len() as u64),
                    Some(lineage_digest.clone()),
                    Some(semantic_digest.clone()),
                ),
                CodecLineage::Unknown { reason } => (
                    ConversationLineageStatus::Unknown {
                        reason_code: reason.as_str().to_string(),
                    },
                    None,
                    None,
                    None,
                    None,
                ),
            };

        let previous_thread = previous.and_then(|manifest| manifest.conversations.get(&thread_id));
        let previous_transcript_sha = previous_thread.and_then(|thread| {
            thread
                .files
                .iter()
                .find(|file| file.role == ConversationFileRole::Transcript)
                .and_then(|file| previous.unwrap().files.get(&file.logical_path))
                .map(|entry| entry.sha256.clone())
        });
        let content_changed = previous_transcript_sha != transcript_sha256;
        let display_title = conversation
            .title
            .clone()
            .or_else(|| previous_thread.and_then(|thread| thread.display_title.clone()));
        let thread = ConversationThread {
            thread_id: thread_id.clone(),
            provider,
            display_title,
            resource_id,
            files: file_refs,
            lineage_status,
            lineage_codec: conversation.lineage_codec.clone(),
            head_message_id,
            message_count,
            lineage_digest,
            semantic_digest,
            forked_from: previous_thread.and_then(|thread| thread.forked_from.clone()),
            source_thread_id: Some(source_thread_id.to_string()),
            created_by_origin: previous_thread
                .map(|thread| thread.created_by_origin.clone())
                .unwrap_or_else(|| Some(publisher.clone())),
            last_updated_by_origin: if content_changed {
                Some(publisher.clone())
            } else {
                previous_thread
                    .map(|thread| thread.last_updated_by_origin.clone())
                    .unwrap_or_else(|| Some(publisher.clone()))
            },
            created_at: previous_thread
                .map(|thread| thread.created_at)
                .unwrap_or(latest_mtime),
            updated_at: if content_changed {
                latest_mtime
            } else {
                previous_thread
                    .map(|thread| thread.updated_at)
                    .unwrap_or(latest_mtime)
            },
        };
        if threads.insert(thread_id.clone(), thread).is_some() {
            return Err(format!(
                "captured conversations resolve to duplicate portable thread '{}'",
                thread_id
            ));
        }
    }

    // Live resources that were not recaptured keep their stored thread
    // metadata exactly; their files carried forward byte-for-byte.
    if let Some(previous) = previous {
        for (thread_id, thread) in &previous.conversations {
            if threads.contains_key(thread_id) {
                continue;
            }
            if resources.contains_key(&thread.resource_id) {
                threads.insert(thread_id.clone(), thread.clone());
            }
        }
    }
    Ok(threads)
}

fn reconcile_manifest_content(
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

pub(crate) fn dependencies_from_manifest(
    manifest: &BundleManifest,
) -> Result<Vec<DependencyAction>, String> {
    let mut actions = Vec::new();
    for descriptor in manifest.resources.values() {
        let Some(kind) = descriptor.metadata.get("dependency_kind") else {
            continue;
        };
        let kind = match kind.as_str() {
            "codex_plugin" => DependencyActionKind::InstallCodexPlugin,
            "claude_plugin" => DependencyActionKind::InstallClaudePlugin,
            "standalone_skill" => DependencyActionKind::InstallStandaloneSkill,
            other => return Err(format!("unknown dependency kind '{}'", other)),
        };
        let argv = descriptor
            .metadata
            .get("dependency_argv_json")
            .map(|json| serde_json::from_str::<Vec<String>>(json))
            .transpose()
            .map_err(|e| format!("parse dependency argv: {}", e))?
            .unwrap_or_default();
        if argv.len() > 64
            || argv.iter().any(|argument| {
                argument.len() > 4096 || argument.chars().any(|character| character == '\0')
            })
        {
            return Err(format!(
                "dependency '{}' has unsafe structured arguments",
                descriptor.resource_id
            ));
        }
        actions.push(DependencyAction {
            action_id: ActionId::parse(format!("dependency:{}", descriptor.resource_id))?,
            resource_id: descriptor.resource_id.clone(),
            kind,
            display_name: descriptor.display_name.clone(),
            provider: descriptor.provider,
            argv,
            requires_explicit_approval: true,
        });
    }
    actions.sort_by(|a, b| a.action_id.cmp(&b.action_id));
    Ok(actions)
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
                "publication drops conversation thread '{}' without a reviewed tombstone",
                thread_id
            ));
        }
    }
    Ok(())
}

/// The stored display title of the conversation owning `resource_id` in one
/// manifest, straight from portable metadata.
fn manifest_thread_title(manifest: &BundleManifest, resource_id: &ResourceId) -> Option<String> {
    manifest
        .conversations
        .values()
        .find(|thread| &thread.resource_id == resource_id)
        .and_then(|thread| thread.display_title.clone())
}

/// Summary counts against the direct parent. Resource changes compare the
/// canonical per-resource content digests, so any restore-affecting change
/// counts exactly once.
fn generation_delta(
    previous: Option<&BundleManifest>,
    current: &BundleManifest,
) -> domain::GenerationDelta {
    let Some(previous) = previous else {
        return domain::GenerationDelta {
            added_resources: current.resources.len() as u64,
            added_files: current.files.len() as u64,
            ..domain::GenerationDelta::default()
        };
    };
    domain::GenerationDelta {
        added_resources: current
            .resources
            .keys()
            .filter(|id| !previous.resources.contains_key(*id))
            .count() as u64,
        changed_resources: current
            .resource_content_sha256
            .iter()
            .filter(|(id, digest)| {
                previous
                    .resource_content_sha256
                    .get(*id)
                    .is_some_and(|old| old != *digest)
            })
            .count() as u64,
        removed_resources: previous
            .resources
            .keys()
            .filter(|id| !current.resources.contains_key(*id))
            .count() as u64,
        added_files: current
            .files
            .keys()
            .filter(|path| !previous.files.contains_key(*path))
            .count() as u64,
        changed_files: current
            .files
            .iter()
            .filter(|(path, entry)| {
                previous
                    .files
                    .get(*path)
                    .is_some_and(|old| old.sha256 != entry.sha256)
            })
            .count() as u64,
        removed_files: previous
            .files
            .keys()
            .filter(|path| !current.files.contains_key(*path))
            .count() as u64,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn validate_bundle_snapshot_bytes(bundle: &FetchedBundle) -> Result<(), String> {
    bundle.snapshot.validate()?;
    if bundle.files.len() != bundle.snapshot.manifest.files.len() {
        return Err("fetched bundle file set differs from its manifest".to_string());
    }
    for (path, entry) in &bundle.snapshot.manifest.files {
        let bytes = bundle
            .files
            .get(path)
            .ok_or_else(|| format!("fetched bundle lacks '{}'", path))?;
        if bytes.len() as u64 != entry.size || sha256(bytes) != entry.sha256 {
            return Err(format!(
                "fetched file '{}' failed size/hash verification",
                path
            ));
        }
    }
    let bytes = serde_json::to_vec(&bundle.snapshot.manifest)
        .map_err(|e| format!("serialize fetched manifest: {}", e))?;
    if sha256(&bytes) != bundle.snapshot.head.latest.manifest_sha256 {
        return Err("fetched manifest bytes differ from the pinned head hash".to_string());
    }
    Ok(())
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
    /// Apply approved byte-materialization actions. RestorePlans never carry
    /// executable dependencies — those live in the DependencyPlan and its
    /// native runner. This engine never shells out or silently activates
    /// executable configuration.
    /// Apply over a fully fetched bundle — the historic byte-verified entry
    /// point, retained for tests; production applies lazily.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn apply_restore_plan(
        &self,
        bundle: &FetchedBundle,
        binding: &ProjectBinding,
        plan: &RestorePlan,
        approved_action_ids: &BTreeSet<ActionId>,
        backup_root: &Path,
        applied_at: u64,
    ) -> Result<ApplyBundleResult, String> {
        validate_bundle_snapshot_bytes(bundle)?;
        let mut bytes = PreloadedBytes::new(bundle);
        self.apply_restore_plan_lazy(
            &bundle.snapshot.version_snapshot(),
            &mut bytes,
            binding,
            plan,
            approved_action_ids,
            backup_root,
            applied_at,
        )
    }

    /// Apply approved actions, downloading only the payload objects that
    /// will actually be written. Actions whose pinned materialized digest
    /// matches the pinned target digest complete without any bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_restore_plan_lazy(
        &self,
        snapshot: &BundleVersionSnapshot,
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
        if current.0.latest != plan.observed_head {
            return Err(
                "bundle head changed after restore planning; refresh and review a new plan"
                    .to_string(),
            );
        }
        // The selected source must still be reachable through the verified
        // chain of that exact head — a tampered or orphaned source fails
        // closed even when the head itself is unchanged.
        let (reachable, _) =
            self.resolve_reachable_generation(&plan.bundle_id, plan.source_version.generation)?;
        if reachable.version != plan.source_version {
            return Err("restore source is no longer reachable from the bundle head".to_string());
        }
        if snapshot.version != plan.source_version {
            return Err("restore snapshot does not match the reviewed source".to_string());
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
                RestoreActionKind::InstallCustomSkill {
                    provider,
                    skill_name,
                }
                | RestoreActionKind::OverwriteCustomSkill {
                    provider,
                    skill_name,
                } => {
                    let receipt = self.apply_custom_skill_action(
                        snapshot,
                        bytes,
                        binding,
                        action,
                        *provider,
                        skill_name,
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
        let Some(bytes) = before.bytes else {
            return Ok(receipt_for(
                action,
                ActionStatus::Applied,
                applied_at,
                Some(logical_path.clone()),
                None,
                None,
            ));
        };
        if before.digest.as_deref() != Some(last_sha256.as_str())
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
        write_immutable_backup(&backup_path, &bytes)?;
        if sha256(
            &fs::read(&backup_path)
                .map_err(|error| format!("verify deletion backup: {}", error))?,
        ) != *last_sha256
        {
            return Err("project-file deletion backup failed verification".to_string());
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
    #[allow(clippy::too_many_arguments)]
    fn apply_file_action(
        &self,
        snapshot: &BundleVersionSnapshot,
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

        let source_bytes = bytes.get(logical_path)?;
        let materialized_bytes =
            materialized_file_bytes(logical_path, descriptor, binding, source_bytes)?;
        let materialized_sha256 = sha256(&materialized_bytes);
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

        if let Some(existing) = before.bytes.as_ref() {
            let backup_path = plan_backup.join(format!("{}.bak", action.action_id.as_str()));
            write_immutable_backup(&backup_path, existing)?;
            backups.push(BackupRecord {
                action_id: action.action_id.clone(),
                target_path: path_text(&target)?,
                backup_path: path_text(&backup_path)?,
                sha256: sha256(existing),
            });
        }
        let write_bytes = if logical_path.as_str() == "state/codex/session_index.jsonl" {
            merge_jsonl_index(before.bytes.as_deref(), source_bytes)?
        } else {
            materialized_bytes
        };
        write_target_atomic(&target, &write_bytes, entry.mode.unwrap_or(0o600))?;
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
        let after = sha256(&write_bytes);
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            Some(logical_path.clone()),
            Some(after),
            None,
        ))
    }

    /// Materialize one custom skill as a whole-directory transaction: stage
    /// the complete verified tree next to the target, back up and journal the
    /// existing directory, swap by rename, and roll back on failure. Runs
    /// under the canonical provider-home operation lock.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn apply_custom_skill_action(
        &self,
        snapshot: &BundleVersionSnapshot,
        bytes: &mut dyn BundleBytes,
        binding: &ProjectBinding,
        action: &RestoreAction,
        provider: Provider,
        skill_name: &str,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        domain::validate_skill_name("custom skill name", skill_name)?;
        let descriptor = snapshot
            .manifest
            .resources
            .get(&action.resource_id)
            .ok_or_else(|| "custom-skill resource is missing from the manifest".to_string())?;
        if descriptor.kind != ResourceKind::StandaloneSkill {
            return Err("action resource is not a custom skill".to_string());
        }
        let identity = custom_skill_identity(descriptor)?;
        if identity.provider != provider || identity.effective_name != skill_name {
            return Err("custom-skill action identity differs from its descriptor".to_string());
        }
        let logical_root = custom_skill_logical_root(identity.provider, &identity.install_dir_name);
        let files =
            custom_skill_manifest_files(&snapshot.manifest, &action.resource_id, &logical_root)?;
        if files.is_empty() {
            return Err("custom skill has no payload files".to_string());
        }
        let source_digest = custom_skill_source_digest(&files);
        if action.source_sha256.as_deref() != Some(source_digest.as_str()) {
            return Err("custom-skill payload changed after planning".to_string());
        }
        let target =
            custom_skill_target_dir(binding, identity.provider, &identity.install_dir_name)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("custom-skill target differs from the pinned plan".to_string());
        }
        let skills_dir = target
            .parent()
            .ok_or_else(|| "custom-skill target has no parent".to_string())?
            .to_path_buf();
        let home = skills_dir
            .parent()
            .ok_or_else(|| "skills directory has no parent".to_string())?;
        inspect_existing_ancestors_no_symlink(&skills_dir)?;
        if !skills_dir.exists() {
            fs::create_dir_all(&skills_dir)
                .map_err(|error| format!("create '{}': {}", skills_dir.display(), error))?;
        }
        let canonical_skills = canonical_real_dir(&skills_dir, "global skills directory")?;
        let canonical_home = canonical_real_dir(home, "provider home")?;
        if !canonical_skills.starts_with(&canonical_home) {
            return Err("global skills directory escapes the provider home".to_string());
        }

        // Canonically equivalent provider homes resolve to the same lock file,
        // so concurrent operations on one home serialize regardless of path
        // spelling. `.`-prefixed names stay invisible to inventory and sync.
        let lock_path = canonical_skills.join(".agent-sync.lock");
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("open provider-home lock: {}", error))?;
        lock_file
            .lock()
            .map_err(|error| format!("acquire provider-home lock: {}", error))?;

        // Revalidate the live target under the lock; approval is pinned to
        // the reviewed digest and a changed target aborts without mutation.
        let state = custom_skill_target_state(&target)?;
        let current_digest = match (&state, &action.expected_target_sha256) {
            (SkillTargetState::Missing, None) => None,
            (SkillTargetState::Present { digest }, _) if digest == &source_digest => {
                // Already-matching content: record the installed claim
                // without rewriting anything (install no-op / adopt).
                return Ok(receipt_for(
                    action,
                    ActionStatus::Applied,
                    applied_at,
                    None,
                    Some(source_digest),
                    None,
                ));
            }
            (SkillTargetState::Present { digest }, Some(expected)) if digest == expected => {
                Some(digest.clone())
            }
            (SkillTargetState::Blocked { reason }, _) => {
                return Err(format!("custom-skill target is blocked: {}", reason))
            }
            _ => return Err("custom-skill target changed after planning".to_string()),
        };

        // Stage the complete tree on the same filesystem as the target so the
        // final activation is a rename, then verify every staged byte.
        let staging = canonical_skills.join(format!(".agent-sync-staging-{}", action.action_id));
        if staging.exists() {
            fs::remove_dir_all(&staging)
                .map_err(|error| format!("clear stale staging: {}", error))?;
        }
        let staging_result = (|| -> Result<(), String> {
            for (relative, logical_path, entry) in &files {
                let payload = bytes.get(logical_path)?;
                let staged = safe_lexical_join(&staging, relative)?;
                if let Some(parent) = staged.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("create '{}': {}", parent.display(), error))?;
                }
                // Set-id and non-permission bits were stripped at capture;
                // strip again on the write path for defense in depth.
                write_target_atomic(&staged, payload, entry.mode.unwrap_or(0o600) & 0o777)?;
            }
            match custom_skill_target_state(&staging)? {
                SkillTargetState::Present { digest } if digest == source_digest => Ok(()),
                _ => Err("staged custom skill failed tree verification".to_string()),
            }
        })();
        if let Err(error) = staging_result {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        // Journal before mutating the target so an interrupted swap is
        // detectable and recoverable from the retained plan backup.
        let backup_dir = plan_backup.join(format!("{}.skill-backup", action.action_id));
        let journal_path = plan_backup.join(format!("{}.journal.json", action.action_id));
        let write_journal = |phase: &str| -> Result<(), String> {
            let journal = serde_json::json!({
                "action_id": action.action_id.as_str(),
                "resource_id": action.resource_id.as_str(),
                "target": path_text(&target)?,
                "staging": path_text(&staging)?,
                "backup": path_text(&backup_dir)?,
                "source_tree_sha256": source_digest,
                "phase": phase,
            });
            fs::write(&journal_path, journal.to_string())
                .map_err(|error| format!("write custom-skill journal: {}", error))
        };
        write_journal("staged")?;

        let displaced = canonical_skills.join(format!(".agent-sync-old-{}", action.action_id));
        if let Some(current_digest) = &current_digest {
            if let Err(error) = copy_skill_tree(&target, &backup_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("custom-skill backup failed: {}", error));
            }
            backups.push(BackupRecord {
                action_id: action.action_id.clone(),
                target_path: path_text(&target)?,
                backup_path: path_text(&backup_dir)?,
                sha256: current_digest.clone(),
            });
            write_journal("backed-up")?;
            if let Err(error) = fs::rename(&target, &displaced) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("displace existing skill: {}", error));
            }
        }
        if let Err(error) = fs::rename(&staging, &target) {
            // Restore the displaced directory before reporting failure; the
            // journal and backup stay behind for manual recovery if this
            // rollback rename also fails.
            if current_digest.is_some() {
                let _ = fs::rename(&displaced, &target);
            }
            let _ = fs::remove_dir_all(&staging);
            write_journal("rolled-back")?;
            return Err(format!("activate staged skill: {}", error));
        }
        if current_digest.is_some() {
            let _ = fs::remove_dir_all(&displaced);
        }
        let verified = match custom_skill_target_state(&target)? {
            SkillTargetState::Present { digest } if digest == source_digest => digest,
            _ => {
                write_journal("verify-failed")?;
                return Err(
                    "installed custom skill failed post-activation verification".to_string()
                );
            }
        };
        write_journal("complete")?;
        Ok(receipt_for(
            action,
            ActionStatus::Applied,
            applied_at,
            None,
            Some(verified),
            None,
        ))
    }
}

/// Bounded no-follow recursive copy used for pre-overwrite backups. The
/// source tree was already verified to contain only regular files.
fn copy_skill_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("create '{}': {}", destination.display(), error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(destination, fs::Permissions::from_mode(0o700));
    }
    for entry in WalkDir::new(source).follow_links(false).max_depth(16) {
        let entry = entry.map_err(|error| format!("walk '{}': {}", source.display(), error))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|_| "backup tree escaped its root".to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        let file_type = entry.file_type();
        if file_type.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("create '{}': {}", target.display(), error))?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(|error| format!("copy '{}': {}", entry.path().display(), error))?;
        } else {
            return Err(format!(
                "backup source '{}' is not a regular file",
                entry.path().display()
            ));
        }
    }
    Ok(())
}
