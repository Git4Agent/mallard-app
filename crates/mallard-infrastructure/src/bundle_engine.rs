//! Project-bundle storage, publication, fetch, restore planning, and apply.
//!
//! The engine operates on validated logical/store keys and a small object
//! store trait. A provider needs three operations: write bytes at a key, read
//! a key, list a prefix. There is no mutable head: every publish ends by
//! writing one immutable record, and the current snapshot of a bundle is
//! selected from the names of those records.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::payload::PayloadBytes;
use crate::portable_resources::PortableResourceRegistry;
use crate::provider_capture::CapturedResources;
use crate::providers::AgentProviderRegistry;
use mallard_domain::portable::contracts;
use mallard_domain::{
    self as domain, hex_lower, now_secs, sha256_hex, ActionId, ActionStatus, ApplyPolicyPrimitive,
    ApplyReceipt, BindingState, BundleDirectoryEntry, BundleFileEntry, BundleHead, BundleId,
    BundleIdentity, BundleKind, BundleManifest, BundleRecipe, BundleScope, BundleSnapshot,
    CapturedWith, CloudThreadId, ConversationThread, LogicalPath, Mainline, PlanId,
    PortableContractResolver, PortableOrigin, ProjectBinding, ProjectFilePreservationPin,
    ProviderKey, PublicationToken, RecordName, ResolvedScope, ResourceDescriptor, ResourceId,
    ResourceKindPrimitive, RestoreAction, RestoreActionKind, RestorePlan, StorageId, SyncScopeKey,
    Tombstone, TombstoneTargetPrimitive, BUNDLE_SCHEMA, RESTORE_PLAN_SCHEMA,
};

use crate::storage_core::publication::{
    list_project_streams, list_stream, read_record, read_records, write_record, RecordStream,
    REMOTE_CHANGED,
};
use crate::storage_core::{
    open_read_only_no_follow, parse_json_bytes, parse_json_reader, repository_object_key,
    sha256_reader, validate_bundle_relative_object_key, ExpectedObject, ObjectStore, StageOutcome,
    VerifiedBody,
};

const MAX_LISTED_BUNDLES: usize = 10_000;
const DEFAULT_PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;

/// The publication one observation selected: the coordinates of its manifest
/// and the digest of its record. A publish names it as its parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadToken {
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub record_sha256: String,
}

impl HeadToken {
    fn publication(&self) -> Result<PublicationToken, String> {
        PublicationToken::new(self.commit_id.clone(), self.record_sha256.clone())
    }
}

/// What one listing of a bundle's stream established: the selected chain,
/// the listed records that are kept and not selected, and the selected
/// publication with its verified manifest.
#[derive(Clone, Debug)]
pub struct BundleObservation {
    pub mainline: Mainline,
    pub unselected: Vec<RecordName>,
    pub current: Option<(BundleSnapshot, HeadToken)>,
}

impl BundleObservation {
    pub fn snapshot(&self) -> Option<&BundleSnapshot> {
        self.current.as_ref().map(|(snapshot, _)| snapshot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishExpectation {
    Absent,
    /// The verified apply observation and the publication it selected. It
    /// never crosses the IPC or persistence boundary.
    Match(Box<BundleSnapshot>, HeadToken),
}

#[derive(Clone, Debug)]
pub enum PublishedBundle {
    /// The record was written and is the selected publication.
    Stored { snapshot: BundleSnapshot },
    /// The candidate manifest was semantically identical to the publication
    /// it was compared to. Nothing was written and `snapshot` is that
    /// publication's.
    NoChange { snapshot: BundleSnapshot },
}

impl PublishedBundle {
    pub fn snapshot(&self) -> &BundleSnapshot {
        match self {
            Self::Stored { snapshot } | Self::NoChange { snapshot } => snapshot,
        }
    }

    pub fn into_snapshot(self) -> BundleSnapshot {
        match self {
            Self::Stored { snapshot } | Self::NoChange { snapshot } => snapshot,
        }
    }

    /// Whether this publish wrote a record, which is then the selected one.
    pub fn stored(&self) -> bool {
        matches!(self, Self::Stored { .. })
    }
}

#[derive(Clone, Debug)]
pub struct CapturedBundleFile {
    pub resource_id: ResourceId,
    pub bytes: PayloadBytes,
    pub source_mtime: u64,
    pub mode: Option<u32>,
}

/// The reviewed next content for one writable sync scope.
#[derive(Clone, Debug, Default)]
pub struct ScopePublishInput {
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub conversations: BTreeMap<CloudThreadId, ConversationThread>,
    pub resources: BTreeMap<ResourceId, ResourceDescriptor>,
    /// Entries copied byte-for-byte from the reviewed previous scope. The
    /// engine verifies each against the current manifest's same scope.
    pub retained_files: BTreeMap<LogicalPath, BundleFileEntry>,
    /// New or changed payloads. A path cannot also appear in retained_files.
    pub captured_files: BTreeMap<LogicalPath, CapturedBundleFile>,
    pub directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
    pub tombstones: BTreeMap<String, Tombstone>,
}

/// A scoped publication. Only the named writable scopes may change; every
/// other scope is carried from the verified current manifest with exactly
/// the same revision and content digest (the mutation firewall). Apply
/// observes the stream once. A changed publication ends in list-then-write;
/// a no-op lists once more, because it makes no write that would.
#[derive(Clone, Debug)]
pub struct PublishBundleRequest {
    pub identity: BundleIdentity,
    pub published_by_origin: PortableOrigin,
    pub writable_scopes: BTreeMap<SyncScopeKey, ScopePublishInput>,
    /// The reviewed `(revision, content_sha256)` coordinates per writable
    /// scope. `(0, all-zero digest)` pins an absent scope.
    pub reviewed_scope_pins: BTreeMap<SyncScopeKey, (u64, String)>,
    pub expected_head: PublishExpectation,
    pub updated_at: u64,
}

pub fn absent_scope_pin() -> (u64, String) {
    (0, "0".repeat(64))
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
    /// The process-wide cache every production engine shares.
    pub fn global() -> Arc<ObjectCache> {
        static CACHE: OnceLock<Arc<ObjectCache>> = OnceLock::new();
        CACHE
            .get_or_init(|| Arc::new(ObjectCache::default()))
            .clone()
    }

    /// Forget every staged object, as an app restart does: a test of an apply
    /// whose review ran in another process starts from a cold cache.
    #[cfg(any(test, feature = "test-support"))]
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.entries.clear();
            inner.order.clear();
            inner.total_bytes = 0;
        }
    }

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

/// The text a byte source reports for an object of a published version that
/// the storage does not hold. The storage was cleared, or a folder that a
/// sync product replicates has not received the object yet; nothing else
/// removes an object of a listed version, and the two cannot be told apart,
/// so the text names no cause. A stopped Pull prints it with the storage's
/// name (PLAN_PULL_PINNED_VERSION.md, section 4.3).
pub const VERSION_INCOMPLETE: &str = "part of this version is missing from the storage";

/// The receipt of a ticked item that a stopped Pull did not reach.
pub const NOT_PULLED: &str = "Not pulled";

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
pub struct EngineByteSource<'a, S: ObjectStore> {
    engine: &'a BundleEngine<S>,
    snapshot: &'a BundleSnapshot,
    cache: Option<Arc<ObjectCache>>,
    loaded: BTreeMap<LogicalPath, VerifiedBody>,
    on_progress: Option<&'a mut dyn FnMut(FetchProgress)>,
}

impl<'a, S: ObjectStore> EngineByteSource<'a, S> {
    fn ensure_loaded(&mut self, logical_path: &LogicalPath) -> Result<(), String> {
        if self.loaded.contains_key(logical_path) {
            return Ok(());
        }
        let entry = self
            .snapshot
            .manifest
            .find_file(logical_path)
            .map(|(_, entry)| entry)
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
            StageOutcome::Missing => return Err(VERSION_INCOMPLETE.to_string()),
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

impl<S: ObjectStore> BundleBytes for EngineByteSource<'_, S> {
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
            .find_file(logical_path)
            .map(|(_, entry)| entry)
            .ok_or_else(|| format!("manifest has no file entry for '{}'", logical_path))?;
        if entry.size > max_bytes {
            return Err(format!("{} exceeds its read limit", label));
        }
        self.body(logical_path)?.read_bounded(max_bytes, label)
    }

    fn prefetch(&mut self, paths: &[LogicalPath]) -> Result<(), String> {
        // Serve cache hits first so progress totals cover the real transfer.
        let mut pending = Vec::new();
        let mut pending_paths = HashSet::<&LogicalPath>::new();
        for path in paths {
            if self.loaded.contains_key(path) {
                continue;
            }
            let entry = self
                .snapshot
                .manifest
                .find_file(path)
                .map(|(_, entry)| entry)
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
            if !pending_paths.insert(path) {
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
        // One concurrent store window per batch, with progress after each
        // object. The batch size and the store's bulk semaphore read the same
        // setting: effective parallelism is their minimum, so they only move
        // together.
        for batch in pending.chunks(crate::app_settings::cloud_transfer_concurrency()) {
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
                    StageOutcome::Missing => return Err(VERSION_INCOMPLETE.to_string()),
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
    target: Option<PathBuf>,
    target_digest: Option<String>,
    target_mode: Option<u32>,
    outcome: PlannedOutcome,
}

/// The publication record: written last, and the only thing that publishes
/// a snapshot. Its object name repeats its graph facts and carries the digest
/// of these exact bytes, so it never contains its own digest, and whether it
/// is current is derived from a listing, never stored.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct CommitRecord {
    schema_version: u32,
    bundle_id: BundleId,
    revision: u64,
    commit_id: String,
    /// The parent's token. Both halves are absent for a first publication
    /// and both present otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_commit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_record_sha256: Option<String>,
    /// UTC Unix milliseconds, taken when the record is built. It ranks
    /// siblings and proves nothing about arrival order.
    created_at_ms: u64,
    manifest_key: String,
    manifest_sha256: String,
    manifest_size: u64,
    added_files: u64,
    changed_files: u64,
    removed_files: u64,
    published_by_origin: PortableOrigin,
    summary: RecordSummary,
}

/// Every fact bundle discovery shows, so discovery reads records and never
/// manifests. It is exactly as fresh as its publication.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct RecordSummary {
    display_name: String,
    kind: BundleKind,
    resources: u64,
    files: u64,
    registration_only: bool,
    providers: Vec<ProviderKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repository_fingerprint: Option<String>,
}

fn manifest_relative_key(revision: u64, commit_id: &str) -> String {
    format!("_manifests/{revision}-{commit_id}.json")
}

fn now_millis() -> Result<u64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?;
    u64::try_from(elapsed.as_millis()).map_err(|_| "system clock is out of range".to_string())
}

impl CommitRecord {
    /// Build the record of `manifest` on `parent`, and its name.
    fn build(
        manifest: &BundleManifest,
        manifest_bytes: &[u8],
        parent: Option<&PublicationToken>,
        file_delta: (u64, u64, u64),
    ) -> Result<(RecordName, Vec<u8>), String> {
        let created_at_ms = now_millis()?;
        let (added_files, changed_files, removed_files) = file_delta;
        let record = Self {
            schema_version: BUNDLE_SCHEMA,
            bundle_id: manifest.bundle.bundle_id.clone(),
            revision: manifest.revision,
            commit_id: manifest.commit_id.clone(),
            previous_commit_id: parent.map(|parent| parent.commit_id().to_string()),
            previous_record_sha256: parent.map(|parent| parent.record_sha256().to_string()),
            created_at_ms,
            manifest_key: manifest_relative_key(manifest.revision, &manifest.commit_id),
            manifest_sha256: sha256_hex(manifest_bytes),
            manifest_size: u64::try_from(manifest_bytes.len())
                .map_err(|_| "serialized bundle manifest is too large".to_string())?,
            added_files,
            changed_files,
            removed_files,
            published_by_origin: manifest.published_by_origin.clone(),
            summary: RecordSummary {
                display_name: manifest.bundle.display_name.clone(),
                kind: manifest.bundle.kind,
                resources: manifest.all_resources().count() as u64,
                files: manifest.all_files().count() as u64,
                registration_only: manifest.is_registration_only(),
                providers: manifest
                    .provider_scopes()
                    .map(|(provider, _)| provider.clone())
                    .collect(),
                repository_fingerprint: manifest.bundle.repository_fingerprint.clone(),
            },
        };
        let bytes = serde_json::to_vec(&record)
            .map_err(|error| format!("serialize publication record: {error}"))?;
        let name = RecordName::new(
            record.revision,
            PublicationToken::new(record.commit_id.clone(), sha256_hex(&bytes))?,
            parent.cloned(),
            created_at_ms,
        )?;
        Ok((name, bytes))
    }

    /// Parse verified record bytes. A name is self-certifying: the bytes
    /// already hash to the digest in it, and the fields must equal its facts.
    fn parse(bundle_id: &BundleId, name: &RecordName, bytes: &[u8]) -> Result<Self, String> {
        let record: Self = parse_json_bytes(bytes, "publication record")?;
        let parent = match (&record.previous_commit_id, &record.previous_record_sha256) {
            (None, None) => None,
            (Some(commit_id), Some(record_sha256)) => Some(PublicationToken::new(
                commit_id.clone(),
                record_sha256.clone(),
            )?),
            _ => return Err("publication record names half a parent".to_string()),
        };
        if record.schema_version != BUNDLE_SCHEMA
            || &record.bundle_id != bundle_id
            || record.revision != name.revision()
            || record.commit_id != name.commit_id()
            || parent.as_ref() != name.parent()
            || record.created_at_ms != name.created_at_ms()
            || record.manifest_key != manifest_relative_key(record.revision, &record.commit_id)
        {
            return Err(format!(
                "publication record '{}' of bundle '{}' disagrees with its name",
                name.token(),
                bundle_id
            ));
        }
        record.published_by_origin.validate()?;
        Ok(record)
    }

    /// The in-memory view of this record that every reader of
    /// `snapshot.head` sees, field for field.
    fn head(&self) -> Result<BundleHead, String> {
        let head = BundleHead {
            schema_version: self.schema_version,
            bundle_id: self.bundle_id.clone(),
            kind: self.summary.kind,
            revision: self.revision,
            commit_id: self.commit_id.clone(),
            manifest_key: self.manifest_key.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            manifest_size: self.manifest_size,
            updated_at: self.created_at_ms / 1000,
        };
        head.validate()?;
        Ok(head)
    }
}

fn head_token(head: &BundleHead, name: &RecordName) -> HeadToken {
    HeadToken {
        revision: head.revision,
        commit_id: head.commit_id.clone(),
        manifest_sha256: head.manifest_sha256.clone(),
        manifest_size: head.manifest_size,
        record_sha256: name.token().record_sha256().to_string(),
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
    pub registration_only: bool,
    pub providers: Vec<ProviderKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
}

pub struct BundleEngine<S> {
    store: S,
    storage_id: StorageId,
    /// The portable contracts this build can execute. Publish resolves every
    /// writable scope through it; planning and apply resolve every fetched
    /// scope and leave unresolvable ones preserved and inert.
    contracts: Arc<PortableResourceRegistry>,
    /// Verified immutable objects shared across operations: payload bodies
    /// for planning and apply, and manifests. `None` reads the store every
    /// time.
    object_cache: Option<Arc<ObjectCache>>,
}

impl<S: ObjectStore> BundleEngine<S> {
    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn open(store: S, storage_id: StorageId) -> Result<Self, String> {
        Self::open_with_contracts(
            store,
            storage_id,
            Arc::new(PortableResourceRegistry::production().clone()),
        )
    }

    /// Attach an engine to a store whose opener already verified the Mallard
    /// storage marker: `ConfiguredObjectStore::shared` does so once per
    /// storage configuration, and tests that build bare stores call
    /// `ensure_storage_marker` themselves.
    pub fn open_with_contracts(
        store: S,
        storage_id: StorageId,
        contracts: Arc<PortableResourceRegistry>,
    ) -> Result<Self, String> {
        Ok(Self {
            store,
            storage_id,
            contracts,
            object_cache: None,
        })
    }

    /// Share verified objects through `cache`; production engines pass
    /// [`ObjectCache::global`].
    pub fn with_object_cache(mut self, cache: Arc<ObjectCache>) -> Self {
        self.object_cache = Some(cache);
        self
    }

    pub fn contracts(&self) -> &PortableResourceRegistry {
        self.contracts.as_ref()
    }

    pub fn storage_id(&self) -> &StorageId {
        &self.storage_id
    }

    /// Select, then read: list the bundle's stream, select from the names,
    /// and read the selected record. An unchanged selection costs the one
    /// listing; its record is already held by the store's process cache.
    pub fn read_head(
        &self,
        bundle_id: &BundleId,
    ) -> Result<Option<(BundleHead, HeadToken)>, String> {
        list_stream(&self.store, &RecordStream::project(bundle_id)?)?
            .into_mainline()
            .selected()
            .map(|name| self.read_head_at(bundle_id, name))
            .transpose()
    }

    /// The head view of one listed publication. Reading health never changes
    /// selection: a selected record that cannot be read or fails verification
    /// is an error, never a reason to try another one.
    fn read_head_at(
        &self,
        bundle_id: &BundleId,
        name: &RecordName,
    ) -> Result<(BundleHead, HeadToken), String> {
        let bytes = read_record(&self.store, &RecordStream::project(bundle_id)?, name)?;
        let head = CommitRecord::parse(bundle_id, name, &bytes)?.head()?;
        let token = head_token(&head, name);
        Ok((head, token))
    }

    /// One listed publication with its verified manifest.
    pub fn snapshot_at(
        &self,
        bundle_id: &BundleId,
        name: &RecordName,
    ) -> Result<(BundleSnapshot, HeadToken), String> {
        let (head, token) = self.read_head_at(bundle_id, name)?;
        let manifest = self.read_manifest(&head)?;
        Ok((
            BundleSnapshot {
                storage_id: self.storage_id.clone(),
                head,
                manifest,
                fetched_at: now_secs(),
            },
            token,
        ))
    }

    /// The origin one listed record carries: who published it. Read only
    /// while a link's history reads `Displaced`, to tell whether this
    /// computer's copy of the project published the displaced publication.
    pub fn published_by(
        &self,
        bundle_id: &BundleId,
        name: &RecordName,
    ) -> Result<PortableOrigin, String> {
        let bytes = read_record(&self.store, &RecordStream::project(bundle_id)?, name)?;
        Ok(CommitRecord::parse(bundle_id, name, &bytes)?.published_by_origin)
    }

    /// One listing of a bundle's stream, selected: the chain and the records
    /// it does not hold, with nothing read. A Pull apply looks the version
    /// its review pinned up on the chain before it reads that version by
    /// name.
    pub fn select(&self, bundle_id: &BundleId) -> Result<(Mainline, Vec<RecordName>), String> {
        Ok(list_stream(&self.store, &RecordStream::project(bundle_id)?)?.into_selection())
    }

    /// One observation of a bundle: the selected chain, the records that are
    /// kept and not selected, and the selected publication with its verified
    /// manifest. A command takes it once, classifies continuity from the
    /// chain, and later hands the observed publication to `publish` as the
    /// parent.
    pub fn observe(&self, bundle_id: &BundleId) -> Result<BundleObservation, String> {
        let (mainline, unselected) = self.select(bundle_id)?;
        let current = mainline
            .selected()
            .map(|name| self.snapshot_at(bundle_id, name))
            .transpose()?;
        Ok(BundleObservation {
            mainline,
            unselected,
            current,
        })
    }

    /// Put the manifest, then publish it with list-then-write. The record is
    /// built once, held in memory, and never outlives this call: nothing is
    /// journaled and nothing is resent after a restart.
    fn store_publication(
        &self,
        manifest: BundleManifest,
        parent: Option<&PublicationToken>,
        file_delta: (u64, u64, u64),
        fetched_at: u64,
    ) -> Result<PublishedBundle, String> {
        let bundle_id = manifest.bundle.bundle_id.clone();
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|error| format!("serialize bundle manifest: {error}"))?;
        let (name, record_bytes) =
            CommitRecord::build(&manifest, &manifest_bytes, parent, file_delta)?;
        let head = CommitRecord::parse(&bundle_id, &name, &record_bytes)?.head()?;
        self.store.put_immutable(
            &repository_object_key(&bundle_id, &head.manifest_key)?,
            &manifest_bytes,
        )?;
        write_record(
            &self.store,
            &RecordStream::project(&bundle_id)?,
            &name,
            &record_bytes,
        )?;
        Ok(PublishedBundle::Stored {
            snapshot: BundleSnapshot {
                storage_id: self.storage_id.clone(),
                head,
                manifest,
                fetched_at,
            },
        })
    }

    /// Publish the empty registration manifest that makes a just-linked
    /// bundle discoverable before its first push: revision 1, zero scopes,
    /// identity and repository fingerprint only. This can only create a
    /// bundle, never advance one: a stream that already selects a
    /// publication, including one that lands while this one is being
    /// written, is returned as the no-op outcome, so registration is
    /// idempotent and race-safe.
    pub fn publish_registration(
        &self,
        identity: &BundleIdentity,
        published_by_origin: &PortableOrigin,
        updated_at: u64,
    ) -> Result<PublishedBundle, String> {
        identity.validate()?;
        published_by_origin.validate()?;
        if identity.kind != BundleKind::Project {
            return Err("bundle registration supports only project bundles".to_string());
        }
        if updated_at == 0 {
            return Err("registration timestamp must be non-zero".to_string());
        }
        let established = |fetched_at: u64| -> Result<Option<PublishedBundle>, String> {
            let Some((head, _)) = self.read_head(&identity.bundle_id)? else {
                return Ok(None);
            };
            let manifest = self.read_manifest(&head)?;
            Ok(Some(PublishedBundle::NoChange {
                snapshot: BundleSnapshot {
                    storage_id: self.storage_id.clone(),
                    head,
                    manifest,
                    fetched_at,
                },
            }))
        };
        if let Some(existing) = established(updated_at)? {
            return Ok(existing);
        }

        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA,
            revision: 1,
            commit_id: random_named_id("commit")?,
            updated_at,
            bundle: identity.clone(),
            published_by_origin: published_by_origin.clone(),
            scopes: BTreeMap::new(),
        };
        manifest.validate()?;
        match self.store_publication(manifest, None, (0, 0, 0), updated_at) {
            Err(error) if error == REMOTE_CHANGED => established(updated_at)?.ok_or_else(|| {
                "bundle publications disappeared during registration; the repository was removed"
                    .to_string()
            }),
            outcome => outcome,
        }
    }

    /// Publish one complete reviewed bundle snapshot. The caller supplies the
    /// live resource and conversation sets; the engine enforces retention
    /// against the previous hash-verified manifest, skips true no-ops, and
    /// publishes only by writing the record last, with list-then-write.
    /// Uploads carry their sha256 checksum so the accepted PUT is the
    /// verification receipt; publish downloads nothing back.
    #[cfg(any(test, feature = "test-support"))]
    pub fn publish(&self, request: PublishBundleRequest) -> Result<PublishedBundle, String> {
        self.publish_with_progress(request, &mut |_| {}, &mut |_, _| {})
    }

    /// `on_phase` receives one timing per publish stage — observation check,
    /// manifest build, uploads, manifest and record writes — so callers can
    /// log where a slow Push actually spends its time.
    pub fn publish_with_progress(
        &self,
        mut request: PublishBundleRequest,
        on_progress: &mut dyn FnMut(FetchProgress),
        on_phase: &mut dyn FnMut(&str, Duration),
    ) -> Result<PublishedBundle, String> {
        let phase_started = Instant::now();
        request.identity.validate()?;
        request.published_by_origin.validate()?;
        if request.identity.kind != BundleKind::Project {
            return Err("bundle publication supports only project bundles".to_string());
        }
        if request.updated_at == 0 {
            return Err("publish timestamp must be non-zero".to_string());
        }
        if request.writable_scopes.is_empty() {
            return Err("bundle publication names no writable scopes".to_string());
        }
        for (scope_key, input) in &request.writable_scopes {
            input.recipe.validate()?;
            if !request.reviewed_scope_pins.contains_key(scope_key) {
                return Err(format!(
                    "writable scope '{}' has no reviewed scope pin",
                    scope_key
                ));
            }
            for path in input.captured_files.keys() {
                if input.retained_files.contains_key(path) {
                    return Err(format!(
                        "bundle file '{}' is both retained and captured",
                        path
                    ));
                }
            }
        }

        // Apply owns the live observation. The record names it as its parent,
        // so a publisher that lists another selection stops at the record
        // write and never replaces the manifest used for the apply-time checks.
        let (current, previous_manifest) = match request.expected_head {
            PublishExpectation::Absent => (None, None),
            PublishExpectation::Match(snapshot, token) => {
                snapshot.validate()?;
                let head = &snapshot.head;
                if snapshot.storage_id != self.storage_id
                    || head.bundle_id != request.identity.bundle_id
                    || head.revision != token.revision
                    || head.commit_id != token.commit_id
                    || head.manifest_sha256 != token.manifest_sha256
                    || head.manifest_size != token.manifest_size
                {
                    return Err("publication snapshot and head token do not match".to_string());
                }
                token.publication()?;
                let bytes = serde_json::to_vec(&snapshot.manifest)
                    .map_err(|error| format!("serialize observed manifest: {error}"))?;
                if bytes.len() as u64 != head.manifest_size
                    || sha256_hex(&bytes) != head.manifest_sha256
                {
                    return Err("publication snapshot failed manifest verification".to_string());
                }
                (Some((snapshot.head, token)), Some(snapshot.manifest))
            }
        };
        on_phase("publish: validate observation", phase_started.elapsed());

        // Capture already bound each immutable allocation to its digest.
        // Moving the body into its entry lets manifest construction and the
        // upload batch share the same allocation.
        let mut prepared_captures = BTreeMap::new();
        for (scope_key, input) in &mut request.writable_scopes {
            let mut prepared = BTreeMap::new();
            for (path, captured) in std::mem::take(&mut input.captured_files) {
                if captured.mode.is_some_and(|mode| mode & !0o777 != 0) {
                    return Err(format!("bundle file '{}' has unsafe mode bits", path));
                }
                if !input.resources.contains_key(&captured.resource_id) {
                    return Err(format!(
                        "bundle file '{}' belongs to missing resource '{}'",
                        path, captured.resource_id
                    ));
                }
                let digest = captured.bytes.sha256().to_string();
                prepared.insert(
                    path,
                    (
                        BundleFileEntry {
                            resource_id: captured.resource_id.clone(),
                            sha256: digest.clone(),
                            size: captured.bytes.len() as u64,
                            source_mtime: captured.source_mtime,
                            object_key: format!("_objects/{}", digest),
                            mode: captured.mode,
                        },
                        captured.bytes.into_bytes(),
                    ),
                );
            }
            prepared_captures.insert(scope_key.clone(), prepared);
        }

        {
            let phase_started = Instant::now();
            // Mutation-firewall input check: every writable scope in the
            // verified current manifest must still match its reviewed pin.
            // Anything else means that scope advanced and needs a Pull.
            for (scope_key, pin) in &request.reviewed_scope_pins {
                let observed = previous_manifest
                    .as_ref()
                    .and_then(|manifest| manifest.scopes.get(scope_key))
                    .map(|scope| (scope.revision, scope.content_sha256.clone()))
                    .unwrap_or_else(absent_scope_pin);
                if &observed != pin {
                    return Err(format!(
                        "sync scope '{}' changed since review; pull and compare again before pushing",
                        scope_key
                    ));
                }
            }
            for (scope_key, input) in &request.writable_scopes {
                if input.retained_files.is_empty() {
                    continue;
                }
                let previous_scope = previous_manifest
                    .as_ref()
                    .and_then(|manifest| manifest.scopes.get(scope_key));
                let published_file_identities: BTreeSet<(&str, &str, u64)> = previous_scope
                    .map(|scope| {
                        scope
                            .files
                            .values()
                            .map(|candidate| {
                                (
                                    candidate.object_key.as_str(),
                                    candidate.sha256.as_str(),
                                    candidate.size,
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for (path, entry) in &input.retained_files {
                    entry.validate()?;
                    if !published_file_identities.contains(&(
                        entry.object_key.as_str(),
                        entry.sha256.as_str(),
                        entry.size,
                    )) {
                        return Err(format!(
                            "retained file '{}' was not in the reviewed manifest",
                            path
                        ));
                    }
                }
            }

            // Carry every scope from the verified current manifest, then
            // rebuild only the approved writable scopes.
            let mut scopes = previous_manifest
                .as_ref()
                .map(|manifest| manifest.scopes.clone())
                .unwrap_or_default();
            let mut any_change = false;
            let mut pending_uploads = BTreeMap::<String, Bytes>::new();
            for (scope_key, input) in &request.writable_scopes {
                let previous_scope = previous_manifest
                    .as_ref()
                    .and_then(|manifest| manifest.scopes.get(scope_key));
                let prepared = prepared_captures.get(scope_key);
                let mut files = input.retained_files.clone();
                for (path, (entry, _)) in prepared.into_iter().flatten() {
                    files.insert(path.clone(), entry.clone());
                }
                if let Some(previous_scope) = previous_scope {
                    validate_scope_conversation_retention(
                        self.contracts.as_ref(),
                        previous_scope,
                        input,
                    )?;
                    if scope_content_is_semantic_noop(previous_scope, input, &files) {
                        continue;
                    }
                } else if input.recipe.entries.is_empty()
                    && input.resources.is_empty()
                    && files.is_empty()
                    && input.directories.is_empty()
                    && input.tombstones.is_empty()
                    && input.conversations.is_empty()
                {
                    // Creating an absent scope with no content publishes
                    // nothing for that scope.
                    continue;
                }
                for (_, (entry, bytes)) in prepared.into_iter().flatten() {
                    pending_uploads
                        .entry(entry.sha256.clone())
                        .or_insert_with(|| bytes.clone());
                }
                let revision = previous_scope
                    .map(|scope| scope.revision.saturating_add(1))
                    .unwrap_or(1);
                let sealed = BundleScope::sealed(
                    scope_key,
                    revision,
                    input.recipe.clone(),
                    input.captured_with.clone(),
                    input.conversations.clone(),
                    input.resources.clone(),
                    files,
                    input.directories.clone(),
                    input.tombstones.clone(),
                )?;
                scopes.insert(scope_key.clone(), sealed);
                any_change = true;
            }
            let identity_changed = previous_manifest
                .as_ref()
                .map(|manifest| manifest.bundle != request.identity)
                .unwrap_or(true);
            if !any_change && !identity_changed {
                let (head, token) = current
                    .as_ref()
                    .expect("a semantic noop requires a current head");
                // No write means no list-then-write. Prove with one listing
                // that the compared publication is still the selected one
                // before issuing a receipt for it.
                let phase_started = Instant::now();
                let stream = RecordStream::project(&request.identity.bundle_id)?;
                if list_stream(&self.store, &stream)?
                    .mainline()
                    .selected()
                    .map(RecordName::token)
                    != Some(&token.publication()?)
                {
                    return Err(REMOTE_CHANGED.to_string());
                }
                on_phase(
                    "publish: verify unchanged selection",
                    phase_started.elapsed(),
                );
                return Ok(PublishedBundle::NoChange {
                    snapshot: BundleSnapshot {
                        storage_id: self.storage_id.clone(),
                        head: head.clone(),
                        manifest: previous_manifest
                            .expect("a semantic noop requires a current manifest"),
                        fetched_at: request.updated_at,
                    },
                });
            }

            let revision = current
                .as_ref()
                .map(|(head, _)| head.revision.saturating_add(1))
                .unwrap_or(1);
            let manifest = BundleManifest {
                schema_version: BUNDLE_SCHEMA,
                revision,
                commit_id: random_named_id("commit")?,
                updated_at: request.updated_at,
                bundle: request.identity.clone(),
                published_by_origin: request.published_by_origin.clone(),
                scopes,
            };
            manifest.validate()?;
            // Registry validation for what this publication writes: every
            // writable scope must resolve to primitives this build executes.
            for scope_key in request.writable_scopes.keys() {
                if let Some(scope) = manifest.scopes.get(scope_key) {
                    scope
                        .resolve(scope_key, self.contracts.as_ref())
                        .map_err(|error| format!("scope '{}': {}", scope_key, error))?;
                }
            }

            // The final mutation firewall at the request boundary: a
            // publication may change only scopes named by the reviewed
            // writable-scope set. Every other scope must retain its exact
            // revision and content digest from the parent's manifest.
            if let Some(previous) = &previous_manifest {
                for (scope_key, previous_scope) in &previous.scopes {
                    if request.writable_scopes.contains_key(scope_key) {
                        continue;
                    }
                    let carried = manifest.scopes.get(scope_key).ok_or_else(|| {
                        format!(
                            "mutation firewall: carried scope '{}' was dropped",
                            scope_key
                        )
                    })?;
                    if carried.revision != previous_scope.revision
                        || carried.content_sha256 != previous_scope.content_sha256
                    {
                        return Err(format!(
                            "mutation firewall: non-writable scope '{}' changed",
                            scope_key
                        ));
                    }
                }
            }
            for scope_key in manifest.scopes.keys() {
                let carried_from_previous = previous_manifest
                    .as_ref()
                    .is_some_and(|previous| previous.scopes.contains_key(scope_key));
                if !request.writable_scopes.contains_key(scope_key) && !carried_from_previous {
                    return Err(format!(
                        "mutation firewall: unexpected new scope '{}'",
                        scope_key
                    ));
                }
            }
            on_phase(
                "publish: validate retention and build manifest",
                phase_started.elapsed(),
            );

            let phase_started = Instant::now();
            let total_objects = pending_uploads.len();
            let total_bytes = pending_uploads
                .values()
                .map(|bytes| bytes.len() as u64)
                .sum();
            let pending = pending_uploads.into_iter().collect::<Vec<_>>();
            let mut uploaded_objects = 0;
            let mut uploaded_bytes = 0_u64;
            // Same shape as the prefetch batching, with progress after each object.
            for chunk in pending.chunks(crate::app_settings::cloud_transfer_concurrency()) {
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
            let parent = current
                .as_ref()
                .map(|(_, token)| token.publication())
                .transpose()?;
            let delta = file_delta(previous_manifest.as_ref(), &manifest);
            let published =
                self.store_publication(manifest, parent.as_ref(), delta, request.updated_at);
            on_phase(
                "publish: write manifest and record",
                phase_started.elapsed(),
            );
            published
        }
    }

    /// Lazy payload access for `snapshot`, sharing verified objects through
    /// the engine's cache and reporting bulk downloads through `on_progress`.
    pub fn byte_source<'a>(
        &'a self,
        snapshot: &'a BundleSnapshot,
        on_progress: Option<&'a mut dyn FnMut(FetchProgress)>,
    ) -> EngineByteSource<'a, S> {
        EngineByteSource {
            engine: self,
            snapshot,
            cache: self.object_cache.clone(),
            loaded: BTreeMap::new(),
            on_progress,
        }
    }

    /// Verify a bundle's selected publication and its immutable manifest
    /// without loading payload objects. Discovery uses this to match
    /// repository identities without downloading every session or project
    /// file in a storage.
    pub fn inspect(&self, bundle_id: &BundleId) -> Result<BundleSnapshot, String> {
        self.inspect_optional(bundle_id)?
            .ok_or_else(|| format!("bundle '{}' does not exist", bundle_id))
    }

    /// The optional form is used by read-only status views where an empty
    /// linked destination is a valid state rather than an error.
    pub fn inspect_optional(&self, bundle_id: &BundleId) -> Result<Option<BundleSnapshot>, String> {
        Ok(self
            .observe(bundle_id)?
            .current
            .map(|(snapshot, _)| snapshot))
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
        let relative_key = manifest_relative_key(revision, commit_id);
        let body =
            match self.manifest_body(bundle_id, &relative_key, manifest_size, manifest_sha256)? {
                StageOutcome::Missing => {
                    return Err(format!(
                        "bundle manifest '{}' is missing",
                        repository_object_key(bundle_id, &relative_key)?
                    ))
                }
                StageOutcome::Mismatch => {
                    return Err(format!(
                        "bundle manifest '{}' failed verification",
                        repository_object_key(bundle_id, &relative_key)?
                    ))
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

    /// One-pass remote discovery over the flat record namespace, sorted by
    /// bundle ID: one listing of every project stream's record names, each
    /// stream selected from them, and the selected records read through the
    /// store's bounded-concurrency metadata fetch. No payload key is listed
    /// and no manifest is read; the rows come from each record's summary.
    pub fn list_remote_bundles(&self) -> Result<Vec<RemoteBundleSummary>, String> {
        let mut requests = Vec::new();
        for (bundle_id, mainline) in list_project_streams(&self.store)? {
            if let Some(name) = mainline.selected() {
                requests.push((RecordStream::project(&bundle_id)?, name.clone(), bundle_id));
            }
            if requests.len() > MAX_LISTED_BUNDLES {
                return Err(format!(
                    "remote bundle listing exceeds the {}-bundle safety limit",
                    MAX_LISTED_BUNDLES
                ));
            }
        }
        let records = read_records(
            &self.store,
            &requests
                .iter()
                .map(|(stream, name, _)| (stream.clone(), name.clone()))
                .collect::<Vec<_>>(),
        )?;
        requests
            .iter()
            .zip(records)
            .map(|((_, name, bundle_id), bytes)| {
                let record = CommitRecord::parse(bundle_id, name, &bytes)?;
                Ok(RemoteBundleSummary {
                    bundle_id: record.bundle_id,
                    display_name: record.summary.display_name,
                    kind: record.summary.kind,
                    revision: record.revision,
                    updated_at: record.created_at_ms / 1000,
                    resources: record.summary.resources,
                    files: record.summary.files,
                    registration_only: record.summary.registration_only,
                    providers: record.summary.providers,
                    repository_fingerprint: record.summary.repository_fingerprint,
                })
            })
            .collect()
    }

    /// Build a restore plan from a verified current-schema manifest.
    ///
    /// Generic resources materialize byte-for-byte, so planning derives every
    /// local digest from manifest metadata and never downloads payloads.
    /// Typed conversations are omitted here and added as reviewed
    /// conversation actions by the command layer.
    /// Build the file-level restore plan for one verified snapshot.
    ///
    /// Conversation resources never appear here: the typed conversation
    /// planner owns them for every provider. Provider-state files are
    /// planned only for bound providers with a resolved home; unbound and
    /// unknown provider scopes stay view-only and untouched.
    #[allow(clippy::too_many_arguments)]
    pub fn build_restore_plan_from_snapshot(
        &self,
        registry: &AgentProviderRegistry,
        snapshot: &BundleSnapshot,
        binding: &ProjectBinding,
        provider_homes: &BTreeMap<ProviderKey, String>,
        // Reviewed-base content digests per project-content logical path. A
        // diverged file absent from this map preserves its local bytes.
        project_content_bases: &BTreeMap<LogicalPath, String>,
        created_at: u64,
        expected_host_identity_revision: u64,
    ) -> Result<RestorePlan, String> {
        snapshot.validate()?;
        self.validate_binding(binding, provider_homes, None)?;
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
        let mut copy_claims = BTreeSet::<LogicalPath>::new();

        for (scope_key, scope) in &snapshot.manifest.scopes {
            let scope_home = match scope_key {
                SyncScopeKey::Project => None,
                SyncScopeKey::Provider(provider) => {
                    match provider_homes.get(provider) {
                        Some(home) => Some((provider.clone(), home.clone())),
                        // Unbound or unknown provider: every row stays
                        // view-only and the scope is preserved untouched.
                        None => continue,
                    }
                }
            };
            // Registry validation: a scope whose contracts this build does
            // not register stays preserved and inert. Its rows say why.
            let resolved = match scope.resolve(scope_key, self.contracts.as_ref()) {
                Ok(resolved) => resolved,
                Err(reason) => {
                    for resource_id in scope.resources.keys() {
                        actions.push(manual_action(
                            resource_id,
                            &format!("Not applicable in this build: {}", reason),
                        )?);
                    }
                    continue;
                }
            };

            for (logical_path, entry) in &scope.directories {
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
                    source_size: None,
                    requires_explicit_approval: true,
                });
            }

            // Files, pass 1: inspect targets and decide the metadata-only
            // outcome.
            let mut planned_files = Vec::new();
            for (logical_path, entry) in &scope.files {
                if !scope.resources.contains_key(&entry.resource_id) {
                    return Err(format!("file '{}' has no resource", logical_path));
                }
                let kind = resolved
                    .kind(&entry.resource_id)
                    .ok_or_else(|| format!("file '{}' has no resolved kind", logical_path))?;
                if kind == ResourceKindPrimitive::Conversation {
                    continue;
                }
                let target = match (&scope_home, kind) {
                    (None, _) => map_project_logical_target(logical_path, binding)?,
                    (Some((provider, home)), ResourceKindPrimitive::ProviderState) => {
                        provider_state_target(registry, provider, home, binding, logical_path)
                    }
                    (Some(_), _) => None,
                };
                let Some(target) = target else {
                    planned_files.push(PlannedFile {
                        logical_path,
                        entry,
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
                    Err(error) if kind == ResourceKindPrimitive::ProjectContentFile => {
                        planned_files.push(PlannedFile {
                            logical_path,
                            entry,
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
                let outcome = if kind != ResourceKindPrimitive::ProjectContentFile
                    && target_digest.as_deref() == Some(entry.sha256.as_str())
                {
                    // Generic files have no enforced mode, so byte identity
                    // is the whole restore; project files keep their own
                    // receipt semantics.
                    PlannedOutcome::UpToDate
                } else {
                    PlannedOutcome::Action
                };
                planned_files.push(PlannedFile {
                    logical_path,
                    entry,
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
                match planned.outcome {
                    PlannedOutcome::Manual(message) => {
                        actions.push(manual_action(&entry.resource_id, &message)?);
                        continue;
                    }
                    PlannedOutcome::UpToDate => continue,
                    PlannedOutcome::Action => {}
                };
                let target = planned.target.as_ref().expect("file action has a target");
                let kind_primitive = resolved
                    .kind(&entry.resource_id)
                    .ok_or_else(|| format!("file '{}' has no resolved kind", logical_path))?;
                let policy_primitive = resolved
                    .policy(&entry.resource_id)
                    .ok_or_else(|| format!("file '{}' has no resolved policy", logical_path))?;
                let preserve = if kind_primitive == ResourceKindPrimitive::ProjectContentFile {
                    plan_project_file_preservation(
                        logical_path,
                        entry,
                        planned.target_digest.as_deref(),
                        project_content_bases,
                        scope,
                        &mut copy_claims,
                        binding,
                    )?
                } else {
                    None
                };
                let kind = restore_kind_for_file(
                    logical_path,
                    kind_primitive,
                    policy_primitive,
                    entry,
                    preserve,
                )?;
                let requires_explicit_approval = action_needs_explicit_approval(
                    policy_primitive,
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
                    expected_target_mode: (kind_primitive
                        == ResourceKindPrimitive::ProjectContentFile)
                        .then_some(planned.target_mode)
                        .flatten(),
                    materialized_sha256: Some(entry.sha256.clone()),
                    source_size: Some(entry.size),
                    requires_explicit_approval,
                });
            }

            for (tombstone_key, tombstone) in &scope.tombstones {
                match resolved.tombstone(tombstone_key) {
                    Some(TombstoneTargetPrimitive::ProjectContentFile {
                        resource_id,
                        logical_path,
                    }) => {
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
                            source_size: None,
                            requires_explicit_approval: true,
                        });
                    }
                    Some(TombstoneTargetPrimitive::ProjectContentDirectory {
                        resource_id,
                        logical_path,
                    }) => {
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
                            source_size: None,
                            requires_explicit_approval: true,
                        });
                    }
                    _ => {}
                }
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
            conversation_review: Vec::new(),
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
        let body = match self.manifest_body(
            &head.bundle_id,
            &head.manifest_key,
            head.manifest_size,
            &head.manifest_sha256,
        )? {
            StageOutcome::Missing => {
                return Err(format!(
                    "bundle manifest '{}' is missing",
                    repository_object_key(&head.bundle_id, &head.manifest_key)?
                ))
            }
            StageOutcome::Mismatch => {
                return Err(format!(
                    "bundle manifest '{}' failed verification",
                    repository_object_key(&head.bundle_id, &head.manifest_key)?
                ))
            }
            StageOutcome::Verified(body) => body,
        };
        let manifest: BundleManifest = parse_json_reader(body.open()?, "bundle manifest")?;
        manifest.validate_against_head(head)?;
        Ok(manifest)
    }

    /// Verified bytes of one immutable manifest object. A record names its
    /// manifest by digest, so a digest-verified copy in the object cache is
    /// that manifest.
    fn manifest_body(
        &self,
        bundle_id: &BundleId,
        relative_key: &str,
        size: u64,
        sha256: &str,
    ) -> Result<StageOutcome, String> {
        validate_bundle_relative_object_key(relative_key)?;
        let key = repository_object_key(bundle_id, relative_key)?;
        let cache_key = ObjectCacheKey {
            storage_id: self.storage_id.clone(),
            bundle_id: bundle_id.clone(),
            sha256: sha256.to_string(),
            size,
        };
        if let Some(hit) = self
            .object_cache
            .as_ref()
            .and_then(|cache| cache.get(&cache_key))
        {
            return Ok(StageOutcome::Verified(hit));
        }
        let outcome = self.store.stage_verified(
            &key,
            &ExpectedObject {
                size,
                sha256: sha256.to_string(),
            },
        )?;
        if let (Some(cache), StageOutcome::Verified(body)) = (&self.object_cache, &outcome) {
            cache.insert(cache_key, body.clone());
        }
        Ok(outcome)
    }

    fn validate_binding(
        &self,
        binding: &ProjectBinding,
        provider_homes: &BTreeMap<ProviderKey, String>,
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
        for (provider, home) in provider_homes {
            targets.push(canonical_real_dir(
                Path::new(home),
                &format!("{} home", provider),
            )?);
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
    use crate::provider_capture::{
        capture_recipe, discover_project, domain_resources, CaptureRequest,
    };
    use crate::storage_core::plain_store::PlainStore;
    use crate::storage_core::{
        DeleteOutcome, ImmutablePutOutcome, LocalObjectStore, ObjectKey, ObjectPrefix,
        StoreKeyPage, StoreListPage, StoredMetadata, REPOSITORY_COMMITS_PREFIX, REPOSITORY_PREFIX,
        STORAGE_MARKER_KEY,
    };
    use mallard_domain::{LocalProjectId, RecipeEntry, ReplicaId};

    fn bundle_id(number: u64) -> BundleId {
        BundleId::parse(format!("{:032x}", number)).unwrap()
    }

    fn storage_id() -> StorageId {
        StorageId::parse("storage-local").unwrap()
    }

    fn registry() -> &'static AgentProviderRegistry {
        AgentProviderRegistry::production()
    }

    fn claude_key() -> ProviderKey {
        ProviderKey::parse("claude").unwrap()
    }

    fn identity(id: BundleId, name: &str) -> BundleIdentity {
        BundleIdentity {
            bundle_id: id,
            display_name: name.to_string(),
            kind: BundleKind::Project,
            repository_fingerprint: None,
        }
    }

    #[test]
    fn dropbox_two_users_publish_review_and_restore_project_content() {
        let server = crate::storage_core::dropbox_store::mock::MockDropboxServer::new();
        let second = server.second_user();
        let first = BundleEngine::open(server.store, storage_id()).unwrap();
        let second = BundleEngine::open(second, storage_id()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write(&source.join("note.md"), b"shared project content");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(inventory.resources.iter().map(|r| r.resource_id.clone()));
        let id = bundle_id(94);
        publish_captured(
            &first,
            identity(id.clone(), "Dropbox project"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        assert_eq!(second.list_remote_bundles().unwrap().len(), 1);
        let snapshot = second.inspect(&id).unwrap();
        let binding = active_binding(id, &target);
        let homes = BTreeMap::new();
        let plan = second
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &homes,
                &BTreeMap::new(),
                100,
                0,
            )
            .unwrap();
        let approved = plan.actions.iter().map(|a| a.action_id.clone()).collect();
        let mut bytes = second.byte_source(&snapshot, None);
        second
            .apply_restore_plan_lazy(
                &snapshot,
                &mut bytes,
                &binding,
                &homes,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(
            fs::read(target.join("note.md")).unwrap(),
            b"shared project content"
        );
    }

    #[test]
    fn dropbox_partial_payload_commit_keeps_the_published_head() {
        let server = crate::storage_core::dropbox_store::mock::MockDropboxServer::new();
        let engine = BundleEngine::open(server.store.clone(), storage_id()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        write(&source.join("a.md"), b"first original");
        write(&source.join("b.md"), b"second original");
        let mut capture = CaptureRequest::for_project(&source);
        capture.include_project_content = true;
        let inventory = discover_project(registry(), &capture).unwrap();
        let recipe = recipe_for(inventory.resources.iter().map(|r| r.resource_id.clone()));
        let id = bundle_id(95);
        publish_captured(
            &engine,
            identity(id.clone(), "Partial commit"),
            &recipe,
            &capture_recipe(registry(), &capture, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let before = engine.read_head(&id).unwrap();
        write(&source.join("a.md"), b"first changed");
        write(&source.join("b.md"), b"second changed");
        server.reject_payloads_after(1);
        let error = publish_captured(
            &engine,
            identity(id.clone(), "Partial commit"),
            &recipe,
            &capture_recipe(registry(), &capture, &recipe).unwrap(),
            before.clone(),
            20,
        )
        .unwrap_err();
        assert!(error.contains("rate limit"));
        let after = engine.read_head(&id).unwrap().unwrap();
        let before = before.unwrap();
        assert_eq!(after.0, before.0);
        assert_eq!(after.1, before.1);
        assert_eq!(after.0.schema_version, 1);
        let prefix = crate::storage_core::ObjectPrefix::parse(format!(
            ".mallard/v1/repositories/{id}/_objects"
        ))
        .unwrap();
        assert_eq!(
            server.store.list(&prefix, None, 100).unwrap().entries.len(),
            3
        );
    }

    fn recipe_for(resource_ids: impl IntoIterator<Item = String>) -> BundleRecipe {
        let mut recipe = BundleRecipe::default();
        for resource_id in resource_ids {
            let resource_id = ResourceId::parse(resource_id).unwrap();
            recipe.entries.insert(
                resource_id.clone(),
                RecipeEntry {
                    resource_id,
                    apply_policy: contracts::merge(),
                    required: false,
                },
            );
        }
        recipe
    }

    fn captured_with() -> CapturedWith {
        CapturedWith {
            app_version: "test".to_string(),
            provider_versions: BTreeMap::new(),
            codec_versions: BTreeMap::from([("codex".to_string(), 1), ("claude".to_string(), 1)]),
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn active_binding(id: BundleId, project_root: &Path) -> ProjectBinding {
        ProjectBinding {
            host_id: mallard_domain::HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
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
            state: BindingState::Active,
            revision: 3,
            updated_at: 1,
        }
    }

    fn project_scope(manifest: &BundleManifest) -> &BundleScope {
        manifest
            .scope(&SyncScopeKey::Project)
            .expect("manifest has a project scope")
    }

    /// Publish a full captured selection through the scoped publisher: split
    /// the capture into its sync scopes, treat every selected scope or scope
    /// present in the observed head as writable, and pin each one against the
    /// observed head's manifest.
    fn publish_captured<S: ObjectStore>(
        engine: &BundleEngine<S>,
        identity: BundleIdentity,
        recipe: &BundleRecipe,
        captured: &CapturedResources,
        observed: Option<(BundleHead, HeadToken)>,
        updated_at: u64,
    ) -> Result<PublishedBundle, String> {
        publish_captured_with_progress(
            engine,
            identity,
            recipe,
            captured,
            observed,
            updated_at,
            &mut |_| {},
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_captured_with_progress<S: ObjectStore>(
        engine: &BundleEngine<S>,
        identity: BundleIdentity,
        recipe: &BundleRecipe,
        captured: &CapturedResources,
        observed: Option<(BundleHead, HeadToken)>,
        updated_at: u64,
        on_progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<PublishedBundle, String> {
        let previous = observed
            .as_ref()
            .map(|(head, _)| engine.read_manifest(head))
            .transpose()?;
        let descriptors = domain_resources(captured)?;
        let scope_of = |resource_id: &ResourceId| -> Result<SyncScopeKey, String> {
            if let Some(descriptor) = descriptors.get(resource_id) {
                return Ok(match &descriptor.provider {
                    Some(provider) => SyncScopeKey::Provider(provider.clone()),
                    None => SyncScopeKey::Project,
                });
            }
            previous
                .as_ref()
                .and_then(|manifest| manifest.find_resource(resource_id))
                .map(|(scope_key, _)| scope_key.clone())
                .ok_or_else(|| format!("selected resource '{}' has no sync scope", resource_id))
        };
        let scope_of_raw = |resource_id: &str| -> Result<SyncScopeKey, String> {
            scope_of(&ResourceId::parse(resource_id.to_string())?)
        };

        let mut writable_keys = BTreeSet::new();
        for resource_id in recipe.entries.keys() {
            writable_keys.insert(scope_of(resource_id)?);
        }
        if let Some(previous) = &previous {
            writable_keys.extend(previous.scopes.keys().cloned());
        }

        let mut writable_scopes = BTreeMap::new();
        let mut reviewed_scope_pins = BTreeMap::new();
        for scope_key in writable_keys {
            let previous_scope = previous
                .as_ref()
                .and_then(|manifest| manifest.scopes.get(&scope_key));
            let mut scope_recipe = BundleRecipe::default();
            for (resource_id, entry) in &recipe.entries {
                if scope_of(resource_id)? == scope_key {
                    scope_recipe
                        .entries
                        .insert(resource_id.clone(), entry.clone());
                }
            }
            let mut scope_descriptors = BTreeMap::new();
            for (resource_id, descriptor) in &descriptors {
                if scope_of(resource_id)? == scope_key {
                    scope_descriptors.insert(resource_id.clone(), descriptor.clone());
                }
            }
            align_recipe_policies(&mut scope_recipe, &scope_descriptors);
            let (resources, mut retained_files, mut directories, tombstones) =
                reconcile_scope_content(
                    engine.contracts.as_ref(),
                    &scope_key,
                    &scope_recipe,
                    &scope_descriptors,
                    captured,
                    previous_scope,
                    updated_at,
                )?;
            let mut captured_files = BTreeMap::new();
            for file in captured.files.values() {
                if scope_of_raw(&file.resource_id)? != scope_key {
                    continue;
                }
                let path = LogicalPath::parse(file.logical_path.clone())?;
                let resource_id = ResourceId::parse(file.resource_id.clone())?;
                let digest = file.bytes.sha256().to_string();
                let mode = Some(file.mode & 0o777);
                let retained = previous_scope
                    .and_then(|scope| scope.files.get(&path))
                    .filter(|entry| {
                        entry.resource_id == resource_id
                            && entry.sha256 == digest
                            && entry.size == file.bytes.len() as u64
                            && entry.source_mtime == file.source_mtime
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
                            bytes: file.bytes.clone(),
                            source_mtime: file.source_mtime,
                            mode,
                        },
                    );
                }
            }
            for directory in captured.directories.values() {
                if scope_of_raw(&directory.resource_id)? != scope_key {
                    continue;
                }
                directories.insert(
                    LogicalPath::parse(directory.logical_path.clone())?,
                    BundleDirectoryEntry {
                        resource_id: ResourceId::parse(directory.resource_id.clone())?,
                        mode: Some(directory.mode & 0o777),
                        source_mtime: directory.source_mtime,
                    },
                );
            }
            let mut scope_captured_with = captured_with();
            if let Some(previous_scope) = previous_scope {
                for (codec, version) in &previous_scope.captured_with.codec_versions {
                    scope_captured_with
                        .codec_versions
                        .entry(codec.clone())
                        .or_insert(*version);
                }
            }
            reviewed_scope_pins.insert(
                scope_key.clone(),
                previous_scope
                    .map(|scope| (scope.revision, scope.content_sha256.clone()))
                    .unwrap_or_else(absent_scope_pin),
            );
            writable_scopes.insert(
                scope_key,
                ScopePublishInput {
                    recipe: scope_recipe,
                    captured_with: scope_captured_with,
                    conversations: previous_scope
                        .map(|scope| scope.conversations.clone())
                        .unwrap_or_default(),
                    resources,
                    retained_files,
                    captured_files,
                    directories,
                    tombstones,
                },
            );
        }

        engine.publish_with_progress(
            PublishBundleRequest {
                identity,
                published_by_origin: PortableOrigin {
                    origin_host_token: domain::OriginHostToken::parse("0".repeat(64))?,
                    origin_replica_token: domain::OriginReplicaToken::parse("0".repeat(64))?,
                    shared_label: None,
                },
                writable_scopes,
                reviewed_scope_pins,
                expected_head: observed
                    .map(|(head, token)| {
                        PublishExpectation::Match(
                            Box::new(BundleSnapshot {
                                storage_id: engine.storage_id.clone(),
                                head,
                                manifest: previous.clone().unwrap(),
                                fetched_at: updated_at,
                            }),
                            token,
                        )
                    })
                    .unwrap_or(PublishExpectation::Absent),
                updated_at,
            },
            on_progress,
            &mut |_, _| {},
        )
    }

    /// The selected record of `id`, as stored.
    fn selected_record<S: ObjectStore>(
        engine: &BundleEngine<S>,
        id: &BundleId,
    ) -> (RecordName, CommitRecord) {
        let stream = RecordStream::project(id).unwrap();
        let name = list_stream(engine.store(), &stream)
            .unwrap()
            .mainline()
            .selected()
            .cloned()
            .expect("the bundle has a selected publication");
        let bytes = read_record(engine.store(), &stream, &name).unwrap();
        let record = CommitRecord::parse(id, &name, &bytes).unwrap();
        (name, record)
    }

    /// Write a hand-built record straight to its key, as a writer that skips
    /// list-then-write would. `created_at_ms` is the fact its name states.
    fn plant_record<S: ObjectStore>(
        store: &S,
        record: &CommitRecord,
        created_at_ms: u64,
    ) -> RecordName {
        let bytes = serde_json::to_vec(record).unwrap();
        let parent = record.previous_commit_id.as_ref().map(|commit_id| {
            PublicationToken::new(
                commit_id.clone(),
                record.previous_record_sha256.clone().unwrap(),
            )
            .unwrap()
        });
        let name = RecordName::new(
            record.revision,
            PublicationToken::new(record.commit_id.clone(), sha256_hex(&bytes)).unwrap(),
            parent,
            created_at_ms,
        )
        .unwrap();
        let key = RecordStream::project(&record.bundle_id)
            .unwrap()
            .key(&name)
            .unwrap();
        store.put_record(&key, &bytes).unwrap();
        name
    }

    /// A hand-built child of `parent`. Its manifest is not written.
    fn child_record(parent: &(RecordName, CommitRecord), commit: u8) -> CommitRecord {
        let mut child = parent.1.clone();
        child.revision += 1;
        child.commit_id = format!("commit-{}", format!("{commit:02x}").repeat(12));
        child.manifest_key = manifest_relative_key(child.revision, &child.commit_id);
        child.previous_commit_id = Some(parent.0.commit_id().to_string());
        child.previous_record_sha256 = Some(parent.0.token().record_sha256().to_string());
        child.created_at_ms += 1;
        child
    }

    fn published_project(root: &Path, id: &BundleId) -> BundleEngine<LocalObjectStore> {
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("CLAUDE.md"), b"portable guidance");
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let engine = BundleEngine::open(
            LocalObjectStore::open(root.join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_captured(
            &engine,
            identity(id.clone(), "Recorded"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        engine
    }

    #[test]
    fn the_head_is_the_selected_record_field_for_field() {
        let temp = tempfile::tempdir().unwrap();
        let id = bundle_id(91);
        let engine = published_project(temp.path(), &id);
        let (name, record) = selected_record(&engine, &id);
        let (head, token) = engine.read_head(&id).unwrap().unwrap();
        assert_eq!(
            head,
            BundleHead {
                schema_version: record.schema_version,
                bundle_id: record.bundle_id.clone(),
                kind: record.summary.kind,
                revision: record.revision,
                commit_id: record.commit_id.clone(),
                manifest_key: record.manifest_key.clone(),
                manifest_sha256: record.manifest_sha256.clone(),
                manifest_size: record.manifest_size,
                updated_at: record.created_at_ms / 1000,
            }
        );
        assert_eq!(token.publication().unwrap(), *name.token());
        assert_eq!(engine.inspect(&id).unwrap().head, head);
        assert_eq!(
            (record.previous_commit_id, record.previous_record_sha256),
            (None, None)
        );
        assert!(!record.summary.registration_only);
        assert_eq!((record.summary.resources, record.summary.files), (1, 1));
        assert_eq!(record.created_at_ms, name.created_at_ms());
    }

    #[test]
    fn a_selected_record_that_fails_verification_is_an_error_and_selection_does_not_move() {
        // The record's manifest size is enforced, as the stored head's was.
        let temp = tempfile::tempdir().unwrap();
        let id = bundle_id(92);
        let engine = published_project(temp.path(), &id);
        let first = selected_record(&engine, &id);
        let mut wrong_size = child_record(&first, 2);
        let manifest_bytes = fs::read(
            temp.path()
                .join("store")
                .join(REPOSITORY_PREFIX)
                .join(id.as_str())
                .join(&first.1.manifest_key),
        )
        .unwrap();
        engine
            .store()
            .put_immutable(
                &repository_object_key(&id, &wrong_size.manifest_key).unwrap(),
                &manifest_bytes,
            )
            .unwrap();
        wrong_size.manifest_size += 1;
        plant_record(engine.store(), &wrong_size, wrong_size.created_at_ms);
        let error = engine.inspect(&id).unwrap_err();
        assert!(error.contains("failed verification"), "{error}");

        // Fields that disagree with the name's facts.
        let temp = tempfile::tempdir().unwrap();
        let engine = published_project(temp.path(), &id);
        let first = selected_record(&engine, &id);
        let renamed = child_record(&first, 3);
        let name = plant_record(engine.store(), &renamed, renamed.created_at_ms + 5);
        let error = engine.read_head(&id).unwrap_err();
        assert!(error.contains("disagrees with its name"), "{error}");
        // The first publication still reads, and is still not selected.
        let stream = RecordStream::project(&id).unwrap();
        assert_eq!(
            list_stream(engine.store(), &stream)
                .unwrap()
                .mainline()
                .selected(),
            Some(&name)
        );
        assert!(engine.inspect_optional(&id).is_err());

        // Bytes that do not hash to the digest in the name.
        let temp = tempfile::tempdir().unwrap();
        let engine = published_project(temp.path(), &id);
        let (name, _) = selected_record(&engine, &id);
        let path = temp
            .path()
            .join("store")
            .join(REPOSITORY_COMMITS_PREFIX)
            .join(id.as_str())
            .join(name.file_name());
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b' ');
        fs::write(&path, bytes).unwrap();
        let error = engine.inspect(&id).unwrap_err();
        assert!(
            error.contains("does not match the digest in its name"),
            "{error}"
        );

        // A manifest that does not match the record fails the read too.
        let temp = tempfile::tempdir().unwrap();
        let engine = published_project(temp.path(), &id);
        let (_, record) = selected_record(&engine, &id);
        let manifest_path = temp
            .path()
            .join("store")
            .join(REPOSITORY_PREFIX)
            .join(id.as_str())
            .join(&record.manifest_key);
        let mut manifest = fs::read(&manifest_path).unwrap();
        manifest.push(b' ');
        fs::write(&manifest_path, manifest).unwrap();
        let error = engine.inspect(&id).unwrap_err();
        assert!(error.contains("failed verification"), "{error}");
    }

    #[test]
    fn publish_fetch_update_and_tombstone_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("CLAUDE.md"), b"version one");
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let resource_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "CLAUDE.md")
            .unwrap()
            .resource_id
            .clone();
        let recipe = recipe_for([resource_id]);
        let captured = capture_recipe(registry(), &request, &recipe).unwrap();
        let id = bundle_id(2);
        let store_root = temp.path().join("store");
        let store = LocalObjectStore::open(&store_root).unwrap();
        crate::storage_core::ensure_storage_marker(&store).unwrap();
        let engine = BundleEngine::open(store, storage_id())
            .unwrap()
            .with_object_cache(Arc::new(ObjectCache::default()));
        let first = publish_captured(
            &engine,
            identity(id.clone(), "Project"),
            &recipe,
            &captured,
            None,
            10,
        )
        .unwrap();
        assert!(matches!(first, PublishedBundle::Stored { .. }));
        let first = first.into_snapshot();
        assert_eq!(first.head.revision, 1);
        let first_token = engine.read_head(&id).unwrap().unwrap().1;
        let repository_root = store_root.join(format!("{}/{}", REPOSITORY_PREFIX, id));
        assert!(store_root.join(STORAGE_MARKER_KEY).is_file());
        // The record is the publication: there is no head object and no tag,
        // and records live in the flat namespace beside the bundle folder.
        assert!(!repository_root.join("_head.json").exists());
        assert!(!store_root.join(REPOSITORY_PREFIX).join("_tags").exists());
        assert!(!repository_root.join("_commits").exists());
        let (record_name, _) = selected_record(&engine, &id);
        assert!(store_root
            .join(REPOSITORY_COMMITS_PREFIX)
            .join(id.as_str())
            .join(record_name.file_name())
            .is_file());
        assert!(repository_root.join("_manifests").is_dir());
        assert!(repository_root.join("_objects").is_dir());
        let listed = engine.list_remote_bundles().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].registration_only);
        assert_eq!(
            fs::metadata(repository_root.join(&first.head.manifest_key))
                .unwrap()
                .len(),
            first.head.manifest_size
        );
        let snapshot = engine.inspect(&id).unwrap();
        let path = project_scope(&snapshot.manifest)
            .files
            .keys()
            .next()
            .unwrap()
            .clone();
        let mut bytes = engine.byte_source(&snapshot, None);
        assert_eq!(
            bytes.read_bounded(&path, 1024, "test payload").unwrap(),
            b"version one"
        );
        drop(bytes);

        let mut wrong_size = snapshot.clone();
        wrong_size
            .manifest
            .scopes
            .get_mut(&SyncScopeKey::Project)
            .unwrap()
            .files
            .get_mut(&path)
            .unwrap()
            .size += 1;
        let mut bytes = engine.byte_source(&wrong_size, None);
        let error = bytes.body(&path).unwrap_err();
        assert!(
            error.contains("does not match manifest size/hash"),
            "{error}"
        );

        write(&project.join("CLAUDE.md"), b"version two");
        let captured = capture_recipe(registry(), &request, &recipe).unwrap();
        let second = publish_captured(
            &engine,
            identity(id.clone(), "Renamed Project"),
            &recipe,
            &captured,
            Some((first.head.clone(), first_token.clone())),
            20,
        )
        .unwrap()
        .into_snapshot();
        assert_eq!(second.head.revision, 2);
        let second_token = engine.read_head(&id).unwrap().unwrap().1;
        assert_eq!(
            publish_captured(
                &engine,
                identity(id.clone(), "Stale"),
                &recipe,
                &capture_recipe(registry(), &request, &recipe).unwrap(),
                Some((first.head.clone(), first_token)),
                21,
            )
            .unwrap_err(),
            REMOTE_CHANGED
        );

        let empty_recipe = BundleRecipe::default();
        let removed = publish_captured(
            &engine,
            identity(id.clone(), "Renamed Project"),
            &empty_recipe,
            &capture_recipe(registry(), &request, &empty_recipe).unwrap(),
            Some((second.head.clone(), second_token)),
            30,
        )
        .unwrap()
        .into_snapshot();
        assert!(removed.manifest.all_files().next().is_none());
        assert!(removed.manifest.all_resources().next().is_none());
        assert!(removed
            .manifest
            .all_tombstones()
            .any(|(_, _, tombstone)| tombstone.target.kind.as_str() == contracts::RESOURCE));
    }

    /// Forces tiny list pages so the one-pass listing's internal store
    /// cursor loop is exercised without seeding ten thousand objects.
    struct SmallListPageStore {
        inner: PlainStore,
    }

    impl ObjectStore for SmallListPageStore {
        fn get_metadata(
            &self,
            key: &ObjectKey,
            max_bytes: u64,
        ) -> Result<Option<StoredMetadata>, String> {
            self.inner.get_metadata(key, max_bytes)
        }

        fn stage_verified(
            &self,
            key: &ObjectKey,
            expected: &ExpectedObject,
        ) -> Result<StageOutcome, String> {
            self.inner.stage_verified(key, expected)
        }

        fn put_immutable(
            &self,
            key: &ObjectKey,
            bytes: &[u8],
        ) -> Result<ImmutablePutOutcome, String> {
            self.inner.put_immutable(key, bytes)
        }

        fn list(
            &self,
            prefix: &ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<StoreListPage, String> {
            self.inner.list(prefix, cursor, limit.min(100))
        }

        fn list_keys(
            &self,
            prefix: &ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<StoreKeyPage, String> {
            self.inner.list_keys(prefix, cursor, limit.min(100))
        }
    }

    #[test]
    fn remote_bundle_listing_is_one_pass_over_the_flat_record_namespace() {
        let store = SmallListPageStore {
            inner: PlainStore::default(),
        };
        let origin = PortableOrigin {
            origin_host_token: domain::OriginHostToken::parse("0".repeat(64)).unwrap(),
            origin_replica_token: domain::OriginReplicaToken::parse("0".repeat(64)).unwrap(),
            shared_label: None,
        };
        for number in 1..=1_005_u64 {
            let record = CommitRecord {
                schema_version: BUNDLE_SCHEMA,
                bundle_id: bundle_id(number),
                revision: 1,
                commit_id: format!("commit-{number:024x}"),
                previous_commit_id: None,
                previous_record_sha256: None,
                created_at_ms: number * 1000,
                manifest_key: manifest_relative_key(1, &format!("commit-{number:024x}")),
                manifest_sha256: "c".repeat(64),
                manifest_size: 10,
                added_files: 1,
                changed_files: 0,
                removed_files: 0,
                published_by_origin: origin.clone(),
                summary: RecordSummary {
                    display_name: format!("Bundle {}", number),
                    kind: BundleKind::Project,
                    resources: 1,
                    files: 1,
                    registration_only: number == 1,
                    providers: vec![claude_key()],
                    repository_fingerprint: Some("b".repeat(64)),
                },
            };
            plant_record(&store, &record, record.created_at_ms);
        }
        // Bundle payload objects never influence discovery: they live outside
        // the record namespace and are invisible to the listing.
        store
            .put_immutable(
                &repository_object_key(&bundle_id(1), &format!("_objects/{}", "d".repeat(64)))
                    .unwrap(),
                b"payload",
            )
            .unwrap();
        store.inner.reset_counts();

        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let listed = engine.list_remote_bundles().unwrap();
        assert_eq!(listed.len(), 1_005);
        assert!(listed
            .windows(2)
            .all(|pair| pair[0].bundle_id < pair[1].bundle_id));
        let first = &listed[0];
        assert!(first.registration_only);
        assert!(!listed[1].registration_only);
        assert_eq!(first.providers, vec![claude_key()]);
        assert_eq!(
            first.repository_fingerprint.as_deref(),
            Some("b".repeat(64).as_str())
        );
        assert_eq!((first.revision, first.updated_at), (1, 1));
        assert_eq!((first.resources, first.files), (1, 1));
        // Eleven pages of one listing, one read per selected record, and no
        // manifest or payload read.
        assert_eq!(engine.store().inner.listings(), 11);
        assert_eq!(engine.store().inner.reads(), 1_005);
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
        let inventory = discover_project(registry(), &request).unwrap();
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
            LocalObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_captured(
            &engine,
            identity(id.clone(), "Portable project"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let snapshot = engine.inspect(&id).unwrap();
        let binding = active_binding(id, &target);
        let provider_homes = BTreeMap::new();
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &provider_homes,
                &std::collections::BTreeMap::new(),
                100,
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
        assert!(write_action.requires_explicit_approval);
        let approved = BTreeSet::from([write_action.action_id.clone()]);
        let mut bytes = engine.byte_source(&snapshot, None);
        let result = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut bytes,
                &binding,
                &provider_homes,
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
        // No reviewed base was supplied, so the diverged target fails closed
        // to preservation: the local bytes survive as a visible sibling.
        assert_eq!(
            fs::read(target.join("CLAUDE-local-copy.md")).unwrap(),
            b"target edits"
        );
    }

    #[test]
    fn project_copy_file_names_suffix_before_the_extension() {
        assert_eq!(
            project_copy_file_name("ghost_story.docx", 1),
            "ghost_story-local-copy.docx"
        );
        assert_eq!(
            project_copy_file_name("ghost_story.docx", 2),
            "ghost_story-local-copy-2.docx"
        );
        assert_eq!(project_copy_file_name("Makefile", 1), "Makefile-local-copy");
        // A leading dot is a name, not an extension boundary.
        assert_eq!(project_copy_file_name(".envrc", 1), ".envrc-local-copy");
        assert_eq!(
            project_copy_file_name("archive.tar.gz", 1),
            "archive.tar-local-copy.gz"
        );
        let long = format!("{}.md", "x".repeat(200));
        let copy = project_copy_file_name(&long, 1);
        assert!(copy.starts_with(&"x".repeat(96)));
        assert!(copy.ends_with("-local-copy.md"));
        assert!(copy.len() < 128);
    }

    /// One diverged publish: storage holds "storage v2", the target holds
    /// local edits, and the reviewed base is the caller's to supply.
    fn diverged_project_fixture(
        temp: &tempfile::TempDir,
        target_bytes: &[u8],
    ) -> (
        BundleEngine<LocalObjectStore>,
        BundleSnapshot,
        ProjectBinding,
    ) {
        let source = temp.path().join("source-project");
        let target = temp.path().join("target-project");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write(&source.join("draft.md"), b"storage v2");
        write(&target.join("draft.md"), target_bytes);
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let resource_id = inventory
            .resources
            .iter()
            .find(|resource| resource.display_name == "draft.md")
            .unwrap()
            .resource_id
            .clone();
        let recipe = recipe_for([resource_id]);
        let id = bundle_id(41);
        let engine = BundleEngine::open(
            LocalObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_captured(
            &engine,
            identity(id.clone(), "Diverged project"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let snapshot = engine.inspect(&id).unwrap();
        let binding = active_binding(id, &target);
        (engine, snapshot, binding)
    }

    fn draft_write_action(plan: &RestorePlan) -> &RestoreAction {
        plan.actions
            .iter()
            .find(|action| {
                matches!(&action.kind, RestoreActionKind::WriteProjectFile { logical_path, .. }
                    if logical_path.as_str() == "project/draft.md")
            })
            .unwrap()
    }

    fn preservation_pin(action: &RestoreAction) -> Option<&domain::ProjectFilePreservationPin> {
        match &action.kind {
            RestoreActionKind::WriteProjectFile { preserve, .. } => preserve.as_ref(),
            _ => None,
        }
    }

    #[test]
    fn diverged_project_file_preserves_the_local_version_beside_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, snapshot, binding) = diverged_project_fixture(&temp, b"local twist");
        // The base pins a third version: local != base != storage.
        let bases = BTreeMap::from([(
            LogicalPath::parse("project/draft.md").unwrap(),
            sha256_hex(b"reviewed v1"),
        )]);
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &BTreeMap::new(),
                &bases,
                100,
                0,
            )
            .unwrap();
        let action = draft_write_action(&plan);
        assert_eq!(action.source_size, Some("storage v2".len() as u64));
        let pin = preservation_pin(action).expect("diverged file must preserve");
        assert_eq!(
            pin.copy_logical_path.as_str(),
            "project/draft-local-copy.md"
        );
        assert_eq!(pin.local_content_sha256, sha256_hex(b"local twist"));
        assert_eq!(pin.storage_content_sha256, sha256_hex(b"storage v2"));
        assert!(!pin.reusable_witness);
        let approved = BTreeSet::from([action.action_id.clone()]);
        let mut bytes = engine.byte_source(&snapshot, None);
        let result = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut bytes,
                &binding,
                &BTreeMap::new(),
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        let target = temp.path().join("target-project");
        assert_eq!(fs::read(target.join("draft.md")).unwrap(), b"storage v2");
        assert_eq!(
            fs::read(target.join("draft-local-copy.md")).unwrap(),
            b"local twist"
        );
        assert!(result
            .receipts
            .iter()
            .any(|receipt| receipt.status == ActionStatus::Applied));
    }

    #[test]
    fn fast_forward_project_file_skips_preservation_when_local_matches_base() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, snapshot, binding) = diverged_project_fixture(&temp, b"reviewed v1");
        let bases = BTreeMap::from([(
            LogicalPath::parse("project/draft.md").unwrap(),
            sha256_hex(b"reviewed v1"),
        )]);
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &BTreeMap::new(),
                &bases,
                100,
                0,
            )
            .unwrap();
        assert!(preservation_pin(draft_write_action(&plan)).is_none());
    }

    #[test]
    fn occupied_copy_paths_reuse_witnesses_or_take_the_next_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, snapshot, binding) = diverged_project_fixture(&temp, b"local twist");
        let target = temp.path().join("target-project");
        // A verified copy from an interrupted apply is reused as a witness.
        write(&target.join("draft-local-copy.md"), b"local twist");
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &BTreeMap::new(),
                &BTreeMap::new(),
                100,
                0,
            )
            .unwrap();
        let pin = preservation_pin(draft_write_action(&plan)).unwrap();
        assert_eq!(
            pin.copy_logical_path.as_str(),
            "project/draft-local-copy.md"
        );
        assert!(pin.reusable_witness);
        // A foreign occupant pushes the copy to the next suffix.
        write(&target.join("draft-local-copy.md"), b"someone else's file");
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &BTreeMap::new(),
                &BTreeMap::new(),
                100,
                0,
            )
            .unwrap();
        let pin = preservation_pin(draft_write_action(&plan)).unwrap();
        assert_eq!(
            pin.copy_logical_path.as_str(),
            "project/draft-local-copy-2.md"
        );
        assert!(!pin.reusable_witness);
    }

    #[test]
    fn apply_refuses_a_copy_target_that_changed_after_planning() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, snapshot, binding) = diverged_project_fixture(&temp, b"local twist");
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &BTreeMap::new(),
                &BTreeMap::new(),
                100,
                0,
            )
            .unwrap();
        let action = draft_write_action(&plan);
        let target = temp.path().join("target-project");
        write(
            &target.join("draft-local-copy.md"),
            b"landed after the plan",
        );
        let approved = BTreeSet::from([action.action_id.clone()]);
        let mut bytes = engine.byte_source(&snapshot, None);
        let result = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut bytes,
                &binding,
                &BTreeMap::new(),
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        let receipt = result
            .receipts
            .iter()
            .find(|receipt| receipt.action_id == action.action_id)
            .unwrap();
        assert_eq!(receipt.status, ActionStatus::Failed);
        assert!(receipt
            .error
            .as_deref()
            .unwrap()
            .contains("preservation copy target changed"));
        // Neither side moved: the local bytes and the foreign occupant stand.
        assert_eq!(fs::read(target.join("draft.md")).unwrap(), b"local twist");
        assert_eq!(
            fs::read(target.join("draft-local-copy.md")).unwrap(),
            b"landed after the plan"
        );
    }

    /// Store wrapper recording every staged/metadata key and every write so
    /// tests can prove which payload objects an operation actually downloaded
    /// or uploaded. `put_many` records its batch size and then routes through
    /// `put_immutable`, so bulk uploads are also recorded per object.
    #[derive(Clone)]
    struct CountingStore {
        inner: LocalObjectStore,
        keys: Arc<Mutex<Vec<String>>>,
        puts: Arc<Mutex<Vec<String>>>,
        put_batches: Arc<Mutex<Vec<usize>>>,
        put_pointers: Arc<Mutex<Vec<(String, usize)>>>,
        record_requests: Arc<Mutex<Vec<String>>>,
        listings: Arc<Mutex<Vec<String>>>,
    }

    impl CountingStore {
        fn open(root: &Path) -> Self {
            Self {
                inner: LocalObjectStore::open(root).unwrap(),
                keys: Arc::new(Mutex::new(Vec::new())),
                puts: Arc::new(Mutex::new(Vec::new())),
                put_batches: Arc::new(Mutex::new(Vec::new())),
                put_pointers: Arc::new(Mutex::new(Vec::new())),
                record_requests: Arc::new(Mutex::new(Vec::new())),
                listings: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn reset(&self) {
            self.keys.lock().unwrap().clear();
            self.puts.lock().unwrap().clear();
            self.put_batches.lock().unwrap().clear();
            self.put_pointers.lock().unwrap().clear();
            self.record_requests.lock().unwrap().clear();
            self.listings.lock().unwrap().clear();
        }

        /// Object counts of every bulk write, in call order.
        fn put_batches(&self) -> Vec<usize> {
            self.put_batches.lock().unwrap().clone()
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

        fn payload_put_pointers(&self) -> Vec<usize> {
            self.put_pointers
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.contains("/_objects/"))
                .map(|(_, pointer)| *pointer)
                .collect()
        }

        fn put_count(&self) -> usize {
            self.puts.lock().unwrap().len()
        }

        /// Record requests sent, in call order.
        fn record_requests(&self) -> usize {
            self.record_requests.lock().unwrap().len()
        }

        /// Key listings made, in call order.
        fn listings(&self) -> usize {
            self.listings.lock().unwrap().len()
        }

        /// Reads of publication records.
        fn record_gets(&self) -> usize {
            self.keys
                .lock()
                .unwrap()
                .iter()
                .filter(|key| key.contains("/_commits/"))
                .count()
        }

        /// Repository object reads that are payloads — records and manifests
        /// are metadata and excluded.
        fn payload_gets(&self) -> usize {
            self.keys
                .lock()
                .unwrap()
                .iter()
                .filter(|key| key.contains("/_objects/"))
                .count()
        }
    }

    impl ObjectStore for CountingStore {
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
            self.put_pointers
                .lock()
                .unwrap()
                .push((key.as_str().to_string(), bytes.as_ptr() as usize));
            self.inner.put_immutable(key, bytes)
        }

        fn put_many(
            &self,
            objects: Vec<(ObjectKey, Bytes)>,
        ) -> Result<Vec<ImmutablePutOutcome>, String> {
            self.put_batches.lock().unwrap().push(objects.len());
            objects
                .iter()
                .map(|(key, bytes)| self.put_immutable(key, bytes))
                .collect()
        }

        fn put_record(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
            self.record_requests
                .lock()
                .unwrap()
                .push(key.as_str().to_string());
            self.inner.put_record(key, bytes)
        }

        fn delete(&self, key: &ObjectKey, expected_etag: &str) -> Result<DeleteOutcome, String> {
            self.inner.delete(key, expected_etag)
        }

        fn list(
            &self,
            prefix: &ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<StoreListPage, String> {
            self.inner.list(prefix, cursor, limit)
        }

        fn list_keys(
            &self,
            prefix: &ObjectPrefix,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<StoreKeyPage, String> {
            self.listings
                .lock()
                .unwrap()
                .push(prefix.as_str().to_string());
            self.inner.list_keys(prefix, cursor, limit)
        }

        fn local_root(&self) -> Option<&Path> {
            self.inner.local_root()
        }
    }

    #[test]
    fn publish_sends_one_record_request_after_its_listing_and_stops_when_the_stream_changed() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write(&project.join("CLAUDE.md"), b"one attempt");
        let mut request = CaptureRequest::for_project(&project);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let captured = capture_recipe(registry(), &request, &recipe).unwrap();
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let id = bundle_id(92);

        // Another computer's first publication lands after this one's
        // review: the listing before the write shows it, and nothing is sent.
        let rival = published_project(&temp.path().join("rival"), &id);
        let (_, rival_record) = selected_record(&rival, &id);
        plant_record(&store, &rival_record, rival_record.created_at_ms);
        store.reset();
        let error = publish_captured(
            &engine,
            identity(id.clone(), "One attempt"),
            &recipe,
            &captured,
            None,
            10,
        )
        .unwrap_err();
        assert_eq!(error, REMOTE_CHANGED);
        assert_eq!((store.listings(), store.record_requests()), (1, 0));
        // One payload window; the manifest is its own write. Both are
        // unreferenced residue: without a record they are not history.
        assert_eq!(store.put_batches(), vec![1]);
        assert_eq!(store.put_count(), 2);
        assert_eq!(engine.list_remote_bundles().unwrap().len(), 1);
        assert_eq!(
            engine.read_head(&id).unwrap().unwrap().0.commit_id,
            rival_record.commit_id
        );

        // An undisturbed publish: list, send the record once, select again.
        let other = bundle_id(93);
        store.reset();
        let published = publish_captured(
            &engine,
            identity(other.clone(), "One attempt"),
            &recipe,
            &captured,
            None,
            10,
        )
        .unwrap();
        assert!(matches!(published, PublishedBundle::Stored { .. }));
        assert_eq!((store.listings(), store.record_requests()), (2, 1));
        assert_eq!(store.record_gets(), 0);
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
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(41);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let first = publish_captured(
            &engine,
            identity(id.clone(), "Incremental"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        assert!(first.stored());
        let first = first.into_snapshot();
        let first_token = engine.read_head(&id).unwrap().unwrap().1;
        assert_eq!(store.payload_puts().len(), 2);
        for entry in project_scope(&first.manifest).files.values() {
            assert_eq!(entry.object_key, format!("_objects/{}", entry.sha256));
        }

        write(&source.join("changing.md"), b"second draft");
        store.reset();
        let second = publish_captured(
            &engine,
            identity(id.clone(), "Incremental"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.head.clone(), first_token)),
            20,
        )
        .unwrap();
        assert!(second.stored());
        let second = second.into_snapshot();
        assert_eq!(second.head.revision, 2);
        let changed_key =
            repository_object_key(&id, &format!("_objects/{}", sha256_hex(b"second draft")))
                .unwrap();
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
        let stable_path = project_scope(&second.manifest)
            .files
            .iter()
            .find(|(_, entry)| entry.sha256 == sha256_hex(b"stable contents"))
            .map(|(path, _)| path.clone())
            .unwrap();
        assert_eq!(
            project_scope(&second.manifest).files[&stable_path],
            project_scope(&first.manifest).files[&stable_path],
            "the unchanged entry carries over verbatim, object key included"
        );
    }

    #[test]
    fn publish_rejects_retained_file_identity_not_in_reviewed_scope() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("stable.md"), b"stable contents");
        let mut capture_request = CaptureRequest::for_project(&source);
        capture_request.include_project_content = true;
        let inventory = discover_project(registry(), &capture_request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(93);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let first = publish_captured(
            &engine,
            identity(id.clone(), "Retention firewall"),
            &recipe,
            &capture_recipe(registry(), &capture_request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let first = first.into_snapshot();
        let first_token = engine.read_head(&id).unwrap().unwrap().1;
        let first_scope = project_scope(&first.manifest).clone();
        let mut retained_files = first_scope.files.clone();
        let retained_path = {
            let (path, entry) = retained_files.iter_mut().next().unwrap();
            entry.size = entry.size.saturating_add(1);
            path.clone()
        };

        store.reset();
        let error = engine
            .publish(PublishBundleRequest {
                identity: first.manifest.bundle.clone(),
                published_by_origin: first.manifest.published_by_origin.clone(),
                writable_scopes: BTreeMap::from([(
                    SyncScopeKey::Project,
                    ScopePublishInput {
                        recipe: first_scope.recipe.clone(),
                        captured_with: first_scope.captured_with.clone(),
                        conversations: first_scope.conversations.clone(),
                        resources: first_scope.resources.clone(),
                        retained_files,
                        captured_files: BTreeMap::new(),
                        directories: first_scope.directories.clone(),
                        tombstones: first_scope.tombstones.clone(),
                    },
                )]),
                reviewed_scope_pins: BTreeMap::from([(
                    SyncScopeKey::Project,
                    (first_scope.revision, first_scope.content_sha256.clone()),
                )]),
                expected_head: PublishExpectation::Match(Box::new(first.clone()), first_token),
                updated_at: 20,
            })
            .unwrap_err();

        assert_eq!(
            error,
            format!(
                "retained file '{}' was not in the reviewed manifest",
                retained_path
            )
        );
        assert_eq!(store.put_count(), 0);
        assert_eq!((store.listings(), store.record_requests()), (0, 0));
    }

    #[test]
    fn manifest_reads_are_served_from_the_object_cache_after_one_store_read() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("note.md"), b"cached manifest");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(inventory.resources.iter().map(|r| r.resource_id.clone()));
        let id = bundle_id(78);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id())
            .unwrap()
            .with_object_cache(Arc::new(ObjectCache::default()));
        publish_captured(
            &engine,
            identity(id.clone(), "Cached"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let manifest_key_reads = |store: &CountingStore| {
            store
                .keys
                .lock()
                .unwrap()
                .iter()
                .filter(|key| key.contains("/_manifests/"))
                .count()
        };

        // The first inspect stages the manifest; the second is a cache hit.
        store.reset();
        let first = engine.inspect(&id).unwrap();
        assert_eq!(manifest_key_reads(&store), 1);
        let second = engine.inspect(&id).unwrap();
        assert_eq!(manifest_key_reads(&store), 1);
        assert_eq!(first.manifest.commit_id, second.manifest.commit_id);

        // Publication reuses the observed head and its cached manifest.
        store.reset();
        write(&source.join("note.md"), b"cached manifest, changed");
        let observed = engine.read_head(&id).unwrap();
        publish_captured(
            &engine,
            identity(id.clone(), "Cached"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            observed,
            20,
        )
        .unwrap();
        assert_eq!(manifest_key_reads(&store), 0);

        // A pinned base manifest is addressed by key and digest, so the
        // verified copy the refresh cached serves it without a request.
        store.reset();
        let head = first.head;
        for _ in 0..2 {
            engine
                .inspect_manifest_version(
                    &id,
                    head.revision,
                    &head.commit_id,
                    head.manifest_size,
                    &head.manifest_sha256,
                )
                .unwrap();
        }
        assert_eq!(manifest_key_reads(&store), 0);
        assert_eq!(store.listings(), 0);
    }

    #[test]
    fn registration_publishes_an_empty_discoverable_record_exactly_once() {
        let temp = tempfile::tempdir().unwrap();
        let engine = BundleEngine::open(
            LocalObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        let mut who = identity(bundle_id(7), "Registered");
        who.repository_fingerprint = Some("a".repeat(64));
        let origin = PortableOrigin {
            origin_host_token: domain::OriginHostToken::parse("0".repeat(64)).unwrap(),
            origin_replica_token: domain::OriginReplicaToken::parse("0".repeat(64)).unwrap(),
            shared_label: None,
        };

        let first = engine.publish_registration(&who, &origin, 10).unwrap();
        assert!(matches!(first, PublishedBundle::Stored { .. }));
        let first = first.into_snapshot();
        assert_eq!(first.head.revision, 1);
        assert!(first.manifest.is_registration_only());
        assert!(first.manifest.scopes.is_empty());
        assert_eq!(
            first.manifest.bundle.repository_fingerprint,
            Some("a".repeat(64))
        );

        // Idempotent: a repeat registration adopts the standing publication
        // instead of advancing or replacing it.
        let second = engine.publish_registration(&who, &origin, 20).unwrap();
        assert!(!second.stored());
        assert_eq!(second.snapshot().head.revision, 1);
        assert_eq!(second.snapshot().head.commit_id, first.head.commit_id);

        // The discovery read path verifies and returns the registration.
        let snapshot = engine.inspect(&bundle_id(7)).unwrap();
        assert_eq!(snapshot.manifest.revision, 1);
        assert!(snapshot.manifest.is_registration_only());

        // Listing reads the claim's record alone: registration-only and the
        // fingerprint surface without touching the manifest.
        let listed = engine.list_remote_bundles().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].registration_only);
        assert_eq!(listed[0].repository_fingerprint, Some("a".repeat(64)));
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
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(42);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let published = publish_captured(
            &engine,
            identity(id.clone(), "Dedupe"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        assert_eq!(store.payload_puts().len(), 1);
        let shared_key = format!("_objects/{}", sha256_hex(b"same bytes either way"));
        let file_entries: Vec<_> = project_scope(&published.snapshot().manifest)
            .files
            .values()
            .collect();
        assert_eq!(file_entries.len(), 2);
        for entry in file_entries {
            assert_eq!(entry.object_key, shared_key);
        }
    }

    #[test]
    fn publish_reuses_the_capture_allocation_at_the_store_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("payload.bin"), &vec![0x5a; 64 * 1024]);
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let captured = capture_recipe(registry(), &request, &recipe).unwrap();
        let capture_pointer = captured
            .files
            .values()
            .next()
            .unwrap()
            .bytes
            .as_bytes()
            .as_ptr() as usize;
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();

        publish_captured(
            &engine,
            identity(bundle_id(48), "Allocation"),
            &recipe,
            &captured,
            None,
            10,
        )
        .unwrap();

        assert_eq!(store.payload_put_pointers(), vec![capture_pointer]);
    }

    fn retained_publication(snapshot: &BundleSnapshot, token: &HeadToken) -> PublishBundleRequest {
        PublishBundleRequest {
            identity: snapshot.manifest.bundle.clone(),
            published_by_origin: snapshot.manifest.published_by_origin.clone(),
            writable_scopes: snapshot
                .manifest
                .scopes
                .iter()
                .map(|(key, scope)| {
                    (
                        key.clone(),
                        ScopePublishInput {
                            recipe: scope.recipe.clone(),
                            captured_with: scope.captured_with.clone(),
                            conversations: scope.conversations.clone(),
                            resources: scope.resources.clone(),
                            retained_files: scope.files.clone(),
                            directories: scope.directories.clone(),
                            tombstones: scope.tombstones.clone(),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            reviewed_scope_pins: snapshot
                .manifest
                .scopes
                .iter()
                .map(|(key, scope)| (key.clone(), (scope.revision, scope.content_sha256.clone())))
                .collect(),
            expected_head: PublishExpectation::Match(Box::new(snapshot.clone()), token.clone()),
            updated_at: snapshot.head.updated_at + 1,
        }
    }

    #[test]
    fn publication_reuses_verified_observation_and_rejects_late_changes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("note.md"), b"retained bytes");
        let mut capture_request = CaptureRequest::for_project(&source);
        capture_request.include_project_content = true;
        let inventory = discover_project(registry(), &capture_request).unwrap();
        let recipe = recipe_for(inventory.resources.iter().map(|r| r.resource_id.clone()));
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let id = bundle_id(94);
        publish_captured(
            &engine,
            identity(id.clone(), "Observed"),
            &recipe,
            &capture_recipe(registry(), &capture_request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        store.reset();
        let _review = engine.inspect(&id).unwrap();
        let (snapshot, token) = engine.observe(&id).unwrap().current.unwrap();
        let mut changed = retained_publication(&snapshot, &token);
        changed.identity.display_name = "Changed".into();
        let stale = changed.clone();
        let mut phases = Vec::new();
        engine
            .publish_with_progress(changed, &mut |_| {}, &mut |phase, _| {
                phases.push(phase.to_string())
            })
            .unwrap();
        assert_eq!(
            store.record_gets(),
            2,
            "review and apply read the selected record; publication reuses the observation"
        );
        // Review, apply, the listing before the write, and the one after it.
        assert_eq!((store.listings(), store.record_requests()), (4, 1));
        assert!(phases
            .iter()
            .any(|phase| phase == "publish: write manifest and record"));
        store.reset();
        assert_eq!(engine.publish(stale).unwrap_err(), REMOTE_CHANGED);
        assert_eq!((store.listings(), store.record_requests()), (1, 0));
        assert!(
            store.keys.lock().unwrap().is_empty(),
            "a late change is stopped by the listing without a read"
        );
        store.reset();
        assert_eq!(
            engine
                .publish(retained_publication(&snapshot, &token))
                .unwrap_err(),
            REMOTE_CHANGED
        );
        assert_eq!(store.put_count(), 0);
        assert_eq!(
            (store.listings(), store.record_requests()),
            (1, 0),
            "a no-op proves with one listing that its publication is still selected"
        );
        assert!(store.keys.lock().unwrap().is_empty());

        let (current, token) = engine.observe(&id).unwrap().current.unwrap();
        for corrupt in 0..5 {
            let mut request = retained_publication(&current, &token);
            if let PublishExpectation::Match(snapshot, token) = &mut request.expected_head {
                match corrupt {
                    0 => token.revision += 1,
                    1 => snapshot.storage_id = StorageId::parse("another-store").unwrap(),
                    2 => snapshot.manifest.bundle.display_name = "Tampered".into(),
                    3 => token.record_sha256 = "not a digest".into(),
                    _ => request.identity.bundle_id = bundle_id(95),
                }
            }
            store.reset();
            assert!(engine.publish(request).is_err());
            assert_eq!(store.put_count(), 0);
            assert_eq!((store.listings(), store.record_requests()), (0, 0));
            assert!(store.keys.lock().unwrap().is_empty());
        }
        // A disjoint scope observed before apply is carried unchanged. A later
        // disjoint update still requires a fresh comparison: its parent has a
        // child by then.
        let provider = SyncScopeKey::Provider(domain::ProviderKey::parse("claude").unwrap());
        let mut disjoint = retained_publication(&current, &token);
        let tombstone = Tombstone {
            target: contracts::tombstone_resource(
                &ResourceId::parse("claude:memory:removed").unwrap(),
            )
            .unwrap(),
            last_sha256: None,
            deleted_at: 12,
        };
        disjoint.writable_scopes = BTreeMap::from([(
            provider.clone(),
            ScopePublishInput {
                tombstones: BTreeMap::from([(tombstone.structural_key(), tombstone)]),
                ..Default::default()
            },
        )]);
        disjoint.reviewed_scope_pins = BTreeMap::from([(provider.clone(), absent_scope_pin())]);
        engine.publish(disjoint).unwrap();
        let mut stale = retained_publication(&current, &token);
        stale.identity.display_name = "Late writer".into();
        assert_eq!(engine.publish(stale).unwrap_err(), REMOTE_CHANGED);
        let (latest, latest_token) = engine.observe(&id).unwrap().current.unwrap();
        let mut carry = retained_publication(&latest, &latest_token);
        carry.writable_scopes.remove(&provider);
        carry.reviewed_scope_pins.remove(&provider);
        carry.identity.display_name = "Carry disjoint scope".into();
        let carried = engine.publish(carry).unwrap().into_snapshot();
        assert_eq!(
            carried.manifest.scopes[&provider],
            latest.manifest.scopes[&provider]
        );
        let (current, token) = engine.observe(&id).unwrap().current.unwrap();

        // A stream cleared after observation stops both changed and no-op
        // requests: nothing selects the parent any more.
        fs::remove_dir_all(
            store
                .local_root()
                .unwrap()
                .join(REPOSITORY_COMMITS_PREFIX)
                .join(id.as_str()),
        )
        .unwrap();
        let mut changed = retained_publication(&current, &token);
        changed.identity.display_name = "After deletion".into();
        assert_eq!(engine.publish(changed).unwrap_err(), REMOTE_CHANGED);
        assert_eq!(
            engine
                .publish(retained_publication(&current, &token))
                .unwrap_err(),
            REMOTE_CHANGED
        );
        engine
            .publish_registration(
                &current.manifest.bundle,
                &current.manifest.published_by_origin,
                100,
            )
            .unwrap();
        let mut recreated = retained_publication(&current, &token);
        recreated.identity.display_name = "After recreation".into();
        assert_eq!(engine.publish(recreated).unwrap_err(), REMOTE_CHANGED);
        assert_eq!(
            engine
                .publish(retained_publication(&current, &token))
                .unwrap_err(),
            REMOTE_CHANGED
        );
        // A first publication needs a stream without a selected chain.
        assert_eq!(
            publish_captured(
                &engine,
                identity(id, "Absent race"),
                &recipe,
                &capture_recipe(registry(), &capture_request, &recipe).unwrap(),
                None,
                110
            )
            .unwrap_err(),
            REMOTE_CHANGED
        );
    }

    fn test_origin() -> PortableOrigin {
        PortableOrigin {
            origin_host_token: domain::OriginHostToken::parse("0".repeat(64)).unwrap(),
            origin_replica_token: domain::OriginReplicaToken::parse("0".repeat(64)).unwrap(),
            shared_label: None,
        }
    }

    /// A publish on `snapshot` that changes one provider scope and nothing
    /// else: the scope gains a tombstone.
    fn provider_scope_request(
        snapshot: &BundleSnapshot,
        token: &HeadToken,
        provider: &str,
    ) -> PublishBundleRequest {
        let scope = SyncScopeKey::Provider(domain::ProviderKey::parse(provider).unwrap());
        let tombstone = Tombstone {
            target: contracts::tombstone_resource(
                &ResourceId::parse(format!("{provider}:memory:removed")).unwrap(),
            )
            .unwrap(),
            last_sha256: None,
            deleted_at: 12,
        };
        let pin = snapshot
            .manifest
            .scopes
            .get(&scope)
            .map(|scope| (scope.revision, scope.content_sha256.clone()))
            .unwrap_or_else(absent_scope_pin);
        PublishBundleRequest {
            identity: snapshot.manifest.bundle.clone(),
            published_by_origin: snapshot.manifest.published_by_origin.clone(),
            writable_scopes: BTreeMap::from([(
                scope.clone(),
                ScopePublishInput {
                    tombstones: BTreeMap::from([(tombstone.structural_key(), tombstone)]),
                    ..Default::default()
                },
            )]),
            reviewed_scope_pins: BTreeMap::from([(scope, pin)]),
            expected_head: PublishExpectation::Match(Box::new(snapshot.clone()), token.clone()),
            updated_at: snapshot.head.updated_at + 1,
        }
    }

    fn repository_base(snapshot: &BundleSnapshot) -> domain::RepositoryBase {
        domain::RepositoryBase {
            revision: snapshot.head.revision,
            commit_id: snapshot.head.commit_id.clone(),
            manifest_sha256: snapshot.head.manifest_sha256.clone(),
            manifest_size: snapshot.head.manifest_size,
        }
    }

    #[test]
    fn a_publication_that_loses_the_fork_window_fails_and_its_record_is_kept() {
        use std::sync::mpsc;
        let shared = PlainStore::default();
        let engine = |store: PlainStore| BundleEngine::open(store, storage_id()).unwrap();
        let id = bundle_id(61);
        let claude = SyncScopeKey::Provider(claude_key());
        let codex = SyncScopeKey::Provider(domain::ProviderKey::parse("codex").unwrap());
        engine(shared.clone())
            .publish_registration(&identity(id.clone(), "Forked"), &test_origin(), 10)
            .unwrap();
        // Both computers review the same publication and change different scopes.
        let (reviewed, token) = engine(shared.clone())
            .observe(&id)
            .unwrap()
            .current
            .unwrap();

        // The first computer builds its record and lists, then stalls right
        // before its one record request.
        let (stalled_tx, stalled_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel::<()>();
        shared.before_next_record_request(move || {
            stalled_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        });
        let earlier = std::thread::spawn({
            let store = shared.clone();
            let request = provider_scope_request(&reviewed, &token, "claude");
            move || engine(store).publish(request)
        });
        stalled_rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(5));

        // The second computer lists the same childless parent. Between its
        // listing and its write, the first one's record lands.
        let record_keys = |store: &PlainStore| {
            store
                .keys()
                .into_iter()
                .filter(|key| key.as_str().starts_with(REPOSITORY_COMMITS_PREFIX))
                .collect::<Vec<_>>()
        };
        shared.before_next_record_request({
            let store = shared.clone();
            move || {
                resume_tx.send(()).unwrap();
                while record_keys(&store).len() < 2 {
                    std::thread::yield_now();
                }
            }
        });
        let later = engine(shared.clone())
            .publish(provider_scope_request(&reviewed, &token, "codex"))
            .unwrap_err();
        let earlier = earlier.join().unwrap().unwrap();

        // Both records are kept and exactly one is selected, the same one on
        // every device. The push whose record ranks second failed as a whole.
        assert_eq!(later, REMOTE_CHANGED);
        assert!(earlier.stored());
        let earlier = earlier.into_snapshot();
        let records = record_keys(&shared);
        assert_eq!(records.len(), 3);
        let kept = records
            .iter()
            .filter_map(|key| RecordName::parse(key.as_str().rsplit('/').next()?))
            .find(|name| name.revision() == 2 && name.commit_id() != earlier.head.commit_id)
            .expect("the record that was not selected is kept");
        for device in 0..2 {
            let observed = engine(shared.clone()).observe(&id).unwrap();
            let selected = observed.snapshot().unwrap();
            assert_eq!(selected.head, earlier.head, "device {device}");
            assert!(!observed.mainline.contains(2, kept.commit_id()));
            // The observation also names what is kept and not selected, and
            // the chain says it was outranked: its parent is the publication
            // both computers reviewed, which still reads by its record name.
            assert_eq!(observed.unselected, [kept.clone()]);
            let outranked = observed
                .mainline
                .outranked(&observed.unselected, kept.commit_id())
                .expect("the record that lost is outranked");
            let left_at = observed
                .mainline
                .names()
                .iter()
                .find(|name| Some(name.token()) == outranked.parent())
                .expect("the record that lost names a selected parent");
            let (at_shared, _) = engine(shared.clone()).snapshot_at(&id, left_at).unwrap();
            assert_eq!(at_shared.head, reviewed.head);
            assert_eq!(at_shared.manifest, reviewed.manifest);
            // Nothing is merged: the selected snapshot is the winner's alone.
            assert!(selected.manifest.scopes.contains_key(&claude));
            assert!(!selected.manifest.scopes.contains_key(&codex));
            // A failed push moves no base. The computer that lost still holds
            // the publication both reviewed, and the selected one descends
            // from it: an ordinary advanced storage, never a replaced one.
            let nobody = |_: &RecordName| unreachable!("no base names the record that lost");
            assert_eq!(
                domain::classify_remote_continuity(
                    &observed.mainline,
                    &observed.unselected,
                    [&repository_base(&earlier)],
                    nobody,
                )
                .unwrap(),
                domain::RemoteContinuity::InSync { revision: 2 }
            );
            assert_eq!(
                domain::classify_remote_continuity(
                    &observed.mainline,
                    &observed.unselected,
                    [&repository_base(&reviewed)],
                    nobody,
                )
                .unwrap(),
                domain::RemoteContinuity::Advanced {
                    base_revision: 1,
                    remote_revision: 2
                }
            );
        }

        // The user compares again and pushes again: the same change lands on
        // the selected publication, which carries the winner's scope byte for
        // byte.
        let (selected, selected_token) = engine(shared.clone())
            .observe(&id)
            .unwrap()
            .current
            .unwrap();
        let republished = engine(shared.clone())
            .publish(provider_scope_request(&selected, &selected_token, "codex"))
            .unwrap();
        assert!(republished.stored());
        let republished = republished.into_snapshot();
        assert_eq!(republished.head.revision, 3);
        assert_eq!(
            republished.manifest.scopes[&claude],
            earlier.manifest.scopes[&claude]
        );
        assert!(republished.manifest.scopes.contains_key(&codex));
        // The record that was not selected is still there, on no chain.
        let observed = engine(shared.clone()).observe(&id).unwrap();
        assert_eq!(record_keys(&shared).len(), 4);
        assert!(!observed.mainline.contains(2, kept.commit_id()));
    }

    #[test]
    fn a_registration_that_loses_the_fork_window_returns_the_established_one() {
        use std::sync::mpsc;
        let shared = PlainStore::default();
        let engine = |store: PlainStore| BundleEngine::open(store, storage_id()).unwrap();
        let id = bundle_id(96);
        let record_count = |store: &PlainStore| {
            store
                .keys()
                .iter()
                .filter(|key| key.as_str().starts_with(REPOSITORY_COMMITS_PREFIX))
                .count()
        };

        // Two computers link the same project at once and both list an empty
        // stream. The first one's record is built first, so it ranks first,
        // and it lands between the second one's listing and its write.
        let (stalled_tx, stalled_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel::<()>();
        shared.before_next_record_request(move || {
            stalled_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        });
        let first = std::thread::spawn({
            let (store, identity) = (shared.clone(), identity(id.clone(), "Forked"));
            move || engine(store).publish_registration(&identity, &test_origin(), 10)
        });
        stalled_rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(5));
        shared.before_next_record_request({
            let store = shared.clone();
            move || {
                resume_tx.send(()).unwrap();
                while record_count(&store) < 1 {
                    std::thread::yield_now();
                }
            }
        });
        let second = engine(shared.clone())
            .publish_registration(&identity(id.clone(), "Forked"), &test_origin(), 10)
            .unwrap();
        let first = first.join().unwrap().unwrap();

        // Registration stays idempotent: one computer stored the project's
        // first publication, and the other returns that same publication.
        assert!(first.stored() && !second.stored());
        assert_eq!(second.snapshot().head, first.snapshot().head);
        assert_eq!(record_count(&shared), 2);
        let observed = engine(shared).observe(&id).unwrap();
        assert_eq!(observed.snapshot().unwrap().head, first.snapshot().head);
    }

    #[test]
    fn refresh_costs_one_listing_until_the_selection_changes() {
        let shared = PlainStore::default();
        let id = bundle_id(62);
        let device = |store: &PlainStore| {
            BundleEngine::open(store.clone().with_publication_cache(), storage_id())
                .unwrap()
                .with_object_cache(Arc::new(ObjectCache::default()))
        };
        let publisher = device(&shared);
        publisher
            .publish_registration(&identity(id.clone(), "Counted"), &test_origin(), 10)
            .unwrap();

        // A cold start: the listing, the selected record, and its manifest.
        // Nothing was persisted, so that is all a new process has to read.
        let reader = device(&shared);
        shared.reset_counts();
        let first = reader.observe(&id).unwrap();
        assert_eq!((shared.listings(), shared.reads()), (1, 2));

        // Nothing changed: exactly one listing request, and nothing else.
        shared.reset_counts();
        let again = reader.observe(&id).unwrap();
        assert_eq!((shared.listings(), shared.reads()), (1, 0));
        assert_eq!(again.snapshot(), first.snapshot());
        assert_eq!(again.mainline, first.mainline);

        // The remote moves ahead of this replica's base: the listing, the new
        // record and its manifest. The base is found by walking the names;
        // no probe of the base manifest is read.
        let (current, token) = publisher.observe(&id).unwrap().current.unwrap();
        publisher
            .publish(provider_scope_request(&current, &token, "claude"))
            .unwrap();
        shared.reset_counts();
        let ahead = reader.observe(&id).unwrap();
        assert_eq!((shared.listings(), shared.reads()), (1, 2));
        assert_eq!(
            domain::classify_remote_continuity(
                &ahead.mainline,
                &ahead.unselected,
                [&repository_base(first.snapshot().unwrap())],
                |_| unreachable!("nothing is displaced"),
            )
            .unwrap(),
            domain::RemoteContinuity::Advanced {
                base_revision: 1,
                remote_revision: 2
            }
        );
        assert_eq!(shared.reads(), 2);
    }

    #[test]
    fn objects_and_a_manifest_without_a_record_are_never_history_and_nothing_is_resent() {
        let shared = PlainStore::default();
        let engine = |store: &PlainStore| BundleEngine::open(store.clone(), storage_id()).unwrap();
        let id = bundle_id(63);
        let who = identity(id.clone(), "Interrupted");

        // The push dies at the record request, however often it is tried.
        shared.fail_record_requests(usize::MAX, false);
        let error = engine(&shared)
            .publish_registration(&who, &test_origin(), 10)
            .unwrap_err();
        assert!(error.contains("record request failed"), "{error}");
        shared.fail_record_requests(0, false);
        let residue = shared.keys();
        assert_eq!(residue.len(), 1);
        assert!(residue[0].as_str().contains("/_manifests/1-commit-"));
        assert!(engine(&shared).inspect_optional(&id).unwrap().is_none());
        assert!(engine(&shared).list_remote_bundles().unwrap().is_empty());

        // The app restarts: no record was kept anywhere, so nothing is resent.
        shared.reset_counts();
        let restarted = engine(&shared);
        assert!(restarted.observe(&id).unwrap().current.is_none());
        assert_eq!(shared.record_requests(), 0);
        // The next push builds a new record on a new commit id.
        let published = restarted
            .publish_registration(&who, &test_origin(), 20)
            .unwrap()
            .into_snapshot();
        assert_eq!(shared.record_requests(), 1);
        assert!(!residue[0].as_str().contains(&published.head.commit_id));
        let records = shared
            .keys()
            .into_iter()
            .filter(|key| key.as_str().starts_with(REPOSITORY_COMMITS_PREFIX))
            .count();
        assert_eq!(records, 1);
    }

    #[test]
    fn a_store_with_only_put_get_and_list_runs_publication_refresh_review_and_restore() {
        let temp = tempfile::tempdir().unwrap();
        let shared = PlainStore::default();
        let first = BundleEngine::open(shared.clone(), storage_id()).unwrap();
        let second = BundleEngine::open(shared.clone(), storage_id()).unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        write(&source.join("note.md"), b"version one");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(inventory.resources.iter().map(|r| r.resource_id.clone()));
        let id = bundle_id(64);
        first
            .publish_registration(&identity(id.clone(), "Plain"), &test_origin(), 5)
            .unwrap();
        for (version, bytes) in [(10, b"version one".as_slice()), (20, b"version two")] {
            write(&source.join("note.md"), bytes);
            publish_captured(
                &first,
                identity(id.clone(), "Plain"),
                &recipe,
                &capture_recipe(registry(), &request, &recipe).unwrap(),
                first.read_head(&id).unwrap(),
                version,
            )
            .unwrap();
        }

        // Only three shapes of object are ever written: a record in the flat
        // namespace, a manifest, and a content-addressed payload. No head, no
        // tag, no flag, and nothing moved.
        for key in shared.keys() {
            let key = key.as_str();
            let in_bundle = key
                .strip_prefix(&format!("{REPOSITORY_PREFIX}/{id}/"))
                .is_some_and(|rest| {
                    rest.strip_prefix("_objects/")
                        .is_some_and(|sha| domain::validate_sha256("object", sha).is_ok())
                        || rest.strip_prefix("_manifests/").is_some_and(|name| {
                            name.ends_with(".json") && name.contains("-commit-")
                        })
                });
            let record = key
                .strip_prefix(&format!("{REPOSITORY_COMMITS_PREFIX}/{id}/"))
                .is_some_and(|name| RecordName::parse(name).is_some());
            assert!(in_bundle || record, "unexpected object '{key}'");
        }

        let listed = second.list_remote_bundles().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].revision, listed[0].files), (3, 1));
        let snapshot = second.inspect(&id).unwrap();
        let binding = active_binding(id, &target);
        let homes = BTreeMap::new();
        let plan = second
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &homes,
                &BTreeMap::new(),
                100,
                0,
            )
            .unwrap();
        let approved = plan.actions.iter().map(|a| a.action_id.clone()).collect();
        let mut bytes = second.byte_source(&snapshot, None);
        second
            .apply_restore_plan_lazy(
                &snapshot,
                &mut bytes,
                &binding,
                &homes,
                &plan,
                &approved,
                &temp.path().join("backups"),
                101,
            )
            .unwrap();
        assert_eq!(fs::read(target.join("note.md")).unwrap(), b"version two");
    }

    #[test]
    fn publish_semantic_noop_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        fs::create_dir_all(&source).unwrap();
        write(&source.join("CLAUDE.md"), b"steady state");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(43);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let first = publish_captured(
            &engine,
            identity(id.clone(), "Noop"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        assert!(first.stored());
        let first = first.into_snapshot();
        let first_token = engine.read_head(&id).unwrap().unwrap().1;

        store.reset();
        let noop = publish_captured(
            &engine,
            identity(id.clone(), "Noop"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.head.clone(), first_token.clone())),
            20,
        )
        .unwrap();
        // The compared publication is returned, and nothing is written.
        assert!(
            matches!(&noop, PublishedBundle::NoChange { snapshot } if snapshot.head == first.head)
        );
        assert_eq!(store.put_count(), 0, "a semantic no-op writes no objects");
        assert_eq!(
            (store.listings(), store.record_requests()),
            (1, 0),
            "a semantic no-op lists once and never writes a record"
        );
        assert_eq!(engine.read_head(&id).unwrap().unwrap().1, first_token);
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
        let inventory = discover_project(registry(), &request).unwrap();
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
        let first = publish_captured_with_progress(
            &engine,
            identity(id.clone(), "Progress"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
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
        let noop = publish_captured_with_progress(
            &engine,
            identity(id.clone(), "Progress"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.snapshot().head.clone(), first_token.clone())),
            20,
            &mut |progress| events.push(progress),
        )
        .unwrap();
        assert!(!noop.stored());
        assert!(events.is_empty(), "a no-op publish reports no transfer");

        write(
            &source.join("large.md"),
            b"a different considerably longer payload",
        );
        events.clear();
        publish_captured_with_progress(
            &engine,
            identity(id.clone(), "Progress"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.snapshot().head.clone(), first_token)),
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
    fn prefetch_deduplicates_paths_and_preserves_order_and_progress() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source-project");
        write(&source.join("a.md"), b"short");
        write(&source.join("b.md"), b"a longer payload");
        let mut request = CaptureRequest::for_project(&source);
        request.include_project_content = true;
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id())
            .unwrap()
            .with_object_cache(Arc::new(ObjectCache::default()));
        let snapshot = publish_captured(
            &engine,
            identity(bundle_id(45), "Prefetch"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap()
        .into_snapshot();
        let files = &project_scope(&snapshot.manifest).files;
        let paths = files.keys().rev().cloned().collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);
        let requested = [
            paths[0].clone(),
            paths[1].clone(),
            paths[0].clone(),
            paths[1].clone(),
        ];
        let expected_keys = paths
            .iter()
            .map(|path| {
                repository_object_key(&snapshot.head.bundle_id, &files[path].object_key)
                    .unwrap()
                    .as_str()
                    .to_string()
            })
            .collect::<Vec<_>>();
        let mut events = Vec::new();
        store.reset();
        {
            let mut on_progress = |progress| events.push(progress);
            let mut bytes = engine.byte_source(&snapshot, Some(&mut on_progress));
            bytes.prefetch(&requested).unwrap();
            assert_eq!(*store.keys.lock().unwrap(), expected_keys);

            store.reset();
            bytes.prefetch(&requested).unwrap();
            assert!(store.keys.lock().unwrap().is_empty());
        }
        let total_bytes = files[&paths[0]].size + files[&paths[1]].size;
        assert_eq!(
            events
                .iter()
                .map(|event| (
                    event.index,
                    event.total_objects,
                    event.fetched_bytes,
                    event.total_bytes,
                ))
                .collect::<Vec<_>>(),
            vec![
                (1, 2, files[&paths[0]].size, total_bytes),
                (2, 2, total_bytes, total_bytes),
            ]
        );

        events.clear();
        {
            let mut on_progress = |progress| events.push(progress);
            let mut bytes = engine.byte_source(&snapshot, Some(&mut on_progress));
            bytes.prefetch(&requested).unwrap();
        }
        assert!(store.keys.lock().unwrap().is_empty());
        assert!(events.is_empty());
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
        let inventory = discover_project(registry(), &request).unwrap();
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
        publish_captured(
            &engine,
            identity(id.clone(), "Lazy project"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let binding = active_binding(id.clone(), &target);
        let provider_homes = BTreeMap::new();

        store.reset();
        let snapshot = engine.inspect(&id).unwrap();
        let logical_path = project_scope(&snapshot.manifest)
            .files
            .keys()
            .next()
            .unwrap();
        let entry_size = project_scope(&snapshot.manifest).files[logical_path].size;
        let mut limited_bytes = engine.byte_source(&snapshot, None);
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
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &provider_homes,
                &std::collections::BTreeMap::new(),
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
        let mut apply_bytes = engine.byte_source(&snapshot, None);
        let result = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut apply_bytes,
                &binding,
                &provider_homes,
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
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &provider_homes,
                &std::collections::BTreeMap::new(),
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
        let mut apply_bytes = engine.byte_source(&snapshot, None);
        engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut apply_bytes,
                &binding,
                &provider_homes,
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

        // Another publication is selected after planning. The apply is of
        // exactly the reviewed one, whose bytes cannot change, so it reads no
        // head and finishes against the reviewed snapshot.
        let reviewed = selected_record(&engine, &id);
        let newer = child_record(&reviewed, 9);
        plant_record(engine.store(), &newer, newer.created_at_ms);
        let mut apply_bytes = engine.byte_source(&snapshot, None);
        let applied = engine
            .apply_restore_plan_lazy(
                &snapshot,
                &mut apply_bytes,
                &binding,
                &provider_homes,
                &plan,
                &approved,
                &temp.path().join("backups"),
                104,
            )
            .unwrap();
        assert_eq!(applied.stopped, None);
        // The action itself judges its target, which the first apply wrote
        // after the plan pinned it absent: it reports on its own, and the
        // apply is not refused for the newer publication.
        assert!(applied
            .receipts
            .iter()
            .any(|receipt| receipt.action_id == write_action.action_id));
        assert_eq!(
            fs::read(target.join("CLAUDE.md")).unwrap(),
            b"portable guidance"
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
        // Memory is only discovered from the project root's canonical
        // bucket, and only once a session in it has proven ownership — the
        // fixture needs one transcript beside the memory directory.
        let source_canonical = fs::canonicalize(&source).unwrap();
        let transcript = format!(
            "{{\"type\":\"system\",\"sessionId\":\"session-m\",\"cwd\":{}}}\n",
            serde_json::to_string(source_canonical.to_str().unwrap()).unwrap()
        );
        let bucket = crate::providers::claude::encode_claude_project_path(
            source_canonical.to_str().unwrap(),
        );
        write(
            &source_claude.join(format!("projects/{bucket}/session-m.jsonl")),
            transcript.as_bytes(),
        );
        write(
            &source_claude.join(format!("projects/{bucket}/memory/MEMORY.md")),
            b"remember the ducks",
        );
        let request = CaptureRequest {
            project_root: source.clone(),
            provider_homes: BTreeMap::from([(claude_key(), source_claude)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(registry(), &request).unwrap();
        let resource = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id.starts_with("claude:memory:"))
            .unwrap();
        let recipe = recipe_for([resource.resource_id.clone()]);
        let id = bundle_id(98);
        let engine = BundleEngine::open(
            LocalObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        publish_captured(
            &engine,
            identity(id.clone(), "Memory project"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap();
        let snapshot = engine.inspect(&id).unwrap();
        let binding = active_binding(id, &target);
        let provider_homes = BTreeMap::from([(
            claude_key(),
            fs::canonicalize(&target_claude)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
        )]);

        let is_generic_write = |action: &RestoreAction| {
            matches!(
                action.kind,
                RestoreActionKind::WriteFile { .. } | RestoreActionKind::MergeFile { .. }
            )
        };
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &provider_homes,
                &std::collections::BTreeMap::new(),
                100,
                0,
            )
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
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &provider_homes,
                &std::collections::BTreeMap::new(),
                101,
                0,
            )
            .unwrap();
        assert!(
            !plan.actions.iter().any(is_generic_write),
            "identical generic files must not re-enter review: {:?}",
            plan.actions
        );

        // A drifted target is real work again.
        write(&memory_target, b"remember the geese");
        let plan = engine
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &provider_homes,
                &std::collections::BTreeMap::new(),
                102,
                0,
            )
            .unwrap();
        assert!(plan.actions.iter().any(is_generic_write));
    }

    /// Conversations belong exclusively to the typed pipeline now: a raw
    /// session capture is a conversation-kind resource without typed thread
    /// metadata, and the scoped file publisher refuses to put one into a
    /// provider scope.
    #[test]
    fn scoped_publisher_rejects_session_captures_without_typed_threads() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let source_claude = temp.path().join("source-claude");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&source_claude).unwrap();
        let transcript = format!(
            "{{\"type\":\"system\",\"sessionId\":\"session-a\",\"cwd\":{}}}\n",
            serde_json::to_string(source.to_str().unwrap()).unwrap()
        );
        write(
            &source_claude.join("projects/source-bucket/session-a.jsonl"),
            transcript.as_bytes(),
        );
        let request = CaptureRequest {
            project_root: source,
            provider_homes: BTreeMap::from([(claude_key(), source_claude)]),
            include_project_content: false,
            excluded_content_roots: Vec::new(),
        };
        let inventory = discover_project(registry(), &request).unwrap();
        let resource = inventory
            .resources
            .iter()
            .find(|resource| resource.resource_id == "claude:session:session-a")
            .unwrap();
        let recipe = recipe_for([resource.resource_id.clone()]);
        let id = bundle_id(4);
        let engine = BundleEngine::open(
            LocalObjectStore::open(temp.path().join("store")).unwrap(),
            storage_id(),
        )
        .unwrap();
        let error = publish_captured(
            &engine,
            identity(id.clone(), "Claude project"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
        .unwrap_err();
        assert!(error.contains("belongs to no live thread"), "{error}");
        assert!(engine.inspect_optional(&id).unwrap().is_none());
    }
}

fn manual_action(resource_id: &ResourceId, message: &str) -> Result<RestoreAction, String> {
    let suffix = &sha256_hex(message.as_bytes())[..24];
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
        source_size: None,
        requires_explicit_approval: true,
    };
    action.validate()?;
    Ok(action)
}

fn restore_kind_for_file(
    logical_path: &LogicalPath,
    kind: ResourceKindPrimitive,
    apply_policy: ApplyPolicyPrimitive,
    entry: &BundleFileEntry,
    preserve: Option<ProjectFilePreservationPin>,
) -> Result<RestoreActionKind, String> {
    Ok(match kind {
        ResourceKindPrimitive::Conversation => {
            return Err(format!(
                "conversation file '{}' belongs to the typed conversation planner",
                logical_path
            ))
        }
        ResourceKindPrimitive::ProjectContentFile => RestoreActionKind::WriteProjectFile {
            logical_path: logical_path.clone(),
            mode: entry.mode.unwrap_or(0o600) & 0o777,
            source_mtime: entry.source_mtime,
            preserve,
        },
        _ if apply_policy == ApplyPolicyPrimitive::Merge => RestoreActionKind::MergeFile {
            logical_path: logical_path.clone(),
        },
        _ => RestoreActionKind::WriteFile {
            logical_path: logical_path.clone(),
        },
    })
}

fn action_needs_explicit_approval(
    apply_policy: ApplyPolicyPrimitive,
    target_digest: Option<&str>,
    source_digest: &str,
) -> bool {
    matches!(
        apply_policy,
        ApplyPolicyPrimitive::ExplicitReview
            | ApplyPolicyPrimitive::ManualOnly
            | ApplyPolicyPrimitive::Merge
    ) || target_digest.is_some_and(|target| target != source_digest)
}

fn action_id_for(
    resource_id: &ResourceId,
    logical_path: &str,
    action_type: domain::RestoreActionType,
) -> Result<ActionId, String> {
    let material = format!("{}\0{}\0{:?}", resource_id, logical_path, action_type);
    ActionId::parse(format!("action-{}", &sha256_hex(material.as_bytes())[..24]))
}

/// Map one project-scope logical path to its machine target. `project/`
/// content lands under the canonical project root; `requirements/` and
/// `dependencies/` entries have no safe materializer and stay manual.
/// Candidate copy names for a preserved diverged file: the suffix lands
/// before the final extension (`ghost.docx` → `ghost-local-copy.docx`,
/// then `ghost-local-copy-2.docx`…), matching the Catalog's `-local-copy`
/// convention. The stem is trimmed so the component stays well under the
/// logical-path component cap.
fn project_copy_file_name(file_name: &str, attempt: usize) -> String {
    let (stem, extension) = match file_name.rfind('.') {
        Some(index) if index > 0 => file_name.split_at(index),
        _ => (file_name, ""),
    };
    let mut stem = stem.to_string();
    while stem.len() > 96 {
        stem.pop();
    }
    if attempt <= 1 {
        format!("{stem}-local-copy{extension}")
    } else {
        format!("{stem}-local-copy-{attempt}{extension}")
    }
}

const MAX_COPY_NAME_ATTEMPTS: usize = 32;

/// Decide whether a planned project-file write must preserve the local
/// version first, and mint the pin if so. `None` means a plain write: the
/// target is absent, already holds the storage bytes, or matches the
/// reviewed base (a fast-forward). An unknown base fails closed to
/// preservation — when the plan cannot prove the local bytes are stored, it
/// keeps them.
fn plan_project_file_preservation(
    logical_path: &LogicalPath,
    entry: &BundleFileEntry,
    target_digest: Option<&str>,
    project_content_bases: &BTreeMap<LogicalPath, String>,
    scope: &BundleScope,
    copy_claims: &mut BTreeSet<LogicalPath>,
    binding: &ProjectBinding,
) -> Result<Option<ProjectFilePreservationPin>, String> {
    let Some(local_digest) = target_digest else {
        return Ok(None);
    };
    if local_digest == entry.sha256 {
        return Ok(None);
    }
    if project_content_bases.get(logical_path).map(String::as_str) == Some(local_digest) {
        return Ok(None);
    }
    let (parent, file_name) = match logical_path.as_str().rfind('/') {
        Some(index) => logical_path.as_str().split_at(index + 1),
        None => return Err(format!("'{}' is not project content", logical_path)),
    };
    for attempt in 1..=MAX_COPY_NAME_ATTEMPTS {
        let candidate = LogicalPath::parse(format!(
            "{parent}{}",
            project_copy_file_name(file_name, attempt)
        ))?;
        if scope.files.contains_key(&candidate)
            || scope.directories.contains_key(&candidate)
            || copy_claims.contains(&candidate)
        {
            continue;
        }
        let candidate_target = project_content_target(&candidate, binding)?;
        let reusable_witness = match inspect_restore_target(&candidate_target) {
            Ok(state) => match state.digest {
                // A verified copy from an interrupted apply: reuse it.
                Some(digest) if digest == local_digest => true,
                // Occupied by anything else: this name is taken.
                Some(_) => continue,
                None => false,
            },
            // Unreadable (symlink, special file): the name is unusable.
            Err(_) => continue,
        };
        copy_claims.insert(candidate.clone());
        return Ok(Some(ProjectFilePreservationPin {
            copy_logical_path: candidate,
            local_content_sha256: local_digest.to_string(),
            storage_content_sha256: entry.sha256.clone(),
            reusable_witness,
        }));
    }
    Err(format!(
        "could not find an unused preservation copy name beside '{}'",
        logical_path
    ))
}

/// Copy the diverged local file to its pinned sibling before the storage
/// version overwrites it. A verified copy already in place — left by an
/// interrupted earlier apply — is reused; anything else at the copy path is
/// a refusal, mirroring "restore target changed after planning".
fn preserve_local_project_copy(
    target: &Path,
    pin: &ProjectFilePreservationPin,
    binding: &ProjectBinding,
    before: &RestoreTargetState,
) -> Result<(), String> {
    if before.digest.as_deref() != Some(pin.local_content_sha256.as_str()) {
        return Err("restore target changed after planning".to_string());
    }
    let copy_target = project_content_target(&pin.copy_logical_path, binding)?;
    // The copy is project content by construction, so the containment guard
    // needs no provider homes.
    validate_target_for_logical(
        &copy_target,
        &pin.copy_logical_path,
        binding,
        &BTreeMap::new(),
    )?;
    let existing = inspect_restore_target(&copy_target)?;
    match &existing.digest {
        Some(digest) if digest.as_str() == pin.local_content_sha256 => return Ok(()),
        Some(_) => {
            return Err("preservation copy target changed after planning".to_string());
        }
        None => {}
    }
    let size = before
        .size
        .ok_or_else(|| "restore target vanished before preservation".to_string())?;
    let mut reader = open_read_only_no_follow(
        target,
        &format!("preservation source '{}'", target.display()),
    )?;
    write_target_atomic_from_reader(
        &copy_target,
        &mut reader,
        size,
        &pin.local_content_sha256,
        before.mode.unwrap_or(0o600),
        Some(&existing),
    )?;
    if let Ok(metadata) = fs::symlink_metadata(target) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH) {
                set_path_mtime_best_effort(&copy_target, elapsed.as_secs());
            }
        }
    }
    Ok(())
}

fn map_project_logical_target(
    logical_path: &LogicalPath,
    binding: &ProjectBinding,
) -> Result<Option<PathBuf>, String> {
    let logical = logical_path.as_str();
    if let Some(relative) = logical.strip_prefix("project/") {
        return Ok(Some(safe_lexical_join(
            Path::new(&binding.canonical_project_root),
            relative,
        )?));
    }
    Ok(None)
}

/// Map one provider-state logical path through the registered adapter. A
/// mapping failure downgrades to a manual row rather than blocking the plan.
fn provider_state_target(
    registry: &AgentProviderRegistry,
    provider: &ProviderKey,
    provider_home: &str,
    binding: &ProjectBinding,
    logical_path: &LogicalPath,
) -> Option<PathBuf> {
    let prefix = mallard_domain::provider_state_path_prefix(provider);
    let suffix = logical_path.as_str().strip_prefix(prefix.as_str())?;
    let adapter = registry.get(provider)?;
    adapter
        .state_target(Path::new(provider_home), &binding.project_root, suffix)
        .ok()
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
    sha256_hex(format!("dir\0{}\0{:03o}", logical_path, mode & 0o777).as_bytes())
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
    provider_homes: &BTreeMap<ProviderKey, String>,
) -> Result<(), String> {
    let root = if logical_path.as_str().starts_with("project/") {
        PathBuf::from(&binding.canonical_project_root)
    } else if let Some(provider) = provider_homes.keys().find(|provider| {
        logical_path
            .as_str()
            .starts_with(&mallard_domain::provider_state_path_prefix(provider))
    }) {
        let home = provider_homes
            .get(provider)
            .expect("provider key came from this map");
        canonical_real_dir(Path::new(home), &format!("{} home", provider))?
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

pub fn write_target_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let expected_size = u64::try_from(bytes.len())
        .map_err(|_| "target bytes cannot fit the portable size field".to_string())?;
    let expected_sha256 = sha256_hex(bytes);
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

pub fn write_immutable_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let expected_size = u64::try_from(bytes.len())
        .map_err(|_| "backup bytes cannot fit the portable size field".to_string())?;
    let expected_sha256 = sha256_hex(bytes);
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

fn random_named_id(prefix: &str) -> Result<String, String> {
    domain::generated_named_id(prefix)
}

/// The reconciled next-revision manifest content: descriptors, file entries,
/// directories, and tombstones.
pub type ReconciledManifestContent = (
    BTreeMap<ResourceId, ResourceDescriptor>,
    BTreeMap<LogicalPath, BundleFileEntry>,
    BTreeMap<LogicalPath, BundleDirectoryEntry>,
    BTreeMap<String, Tombstone>,
);

/// A recipe selects resources; it never chooses their policy. Every entry
/// takes the apply policy its registered resource contract declared on the
/// descriptor, so the published scope satisfies the exact-match rule.
pub fn align_recipe_policies(
    recipe: &mut BundleRecipe,
    descriptors: &BTreeMap<ResourceId, ResourceDescriptor>,
) {
    for (resource_id, entry) in recipe.entries.iter_mut() {
        if let Some(descriptor) = descriptors.get(resource_id) {
            entry.apply_policy = descriptor.apply_policy.clone();
        }
    }
}

/// Reconcile one sync scope's next content from its reviewed recipe, this
/// capture, and the verified previous scope. Tombstones are minted only
/// inside the scope being reconciled, so no tombstone can ever target an
/// unbound or unknown provider scope.
pub fn reconcile_scope_content(
    contracts: &PortableResourceRegistry,
    scope: &SyncScopeKey,
    recipe: &BundleRecipe,
    captured_descriptors: &BTreeMap<ResourceId, ResourceDescriptor>,
    captured: &CapturedResources,
    previous: Option<&BundleScope>,
    updated_at: u64,
) -> Result<ReconciledManifestContent, String> {
    for id in captured_descriptors.keys() {
        if !recipe.entries.contains_key(id) {
            return Err(format!("capture returned unselected resource '{}'", id));
        }
    }
    // A previous scope this build cannot resolve cannot be reconciled over:
    // publishing would silently drop records it does not understand.
    let previous_resolved = match previous {
        Some(previous) => previous.resolve(scope, contracts)?,
        None => ResolvedScope::default(),
    };
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
        .map(|scope| scope.tombstones.clone())
        .unwrap_or_default();

    for id in recipe.entries.keys() {
        if let Some(descriptor) = captured_descriptors.get(id) {
            resources.insert(id.clone(), descriptor.clone());
        } else if let Some(descriptor) = previous.and_then(|scope| scope.resources.get(id)) {
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
        // Conversation threads and their files never travel through the
        // generic recipe: they are carried and mutated only by reviewed
        // conversation actions, so their absence from the recipe is not a
        // deletion.
        let conversation_owned = |resource_id: &ResourceId| {
            previous_resolved.kind(resource_id) == Some(ResourceKindPrimitive::Conversation)
        };
        for (id, descriptor) in &previous.resources {
            if conversation_owned(id) {
                continue;
            }
            if !recipe.entries.contains_key(id) {
                let last_sha256 = descriptor
                    .attribute_str("content_sha256")
                    .map(str::to_string);
                let tombstone = Tombstone {
                    target: contracts::tombstone_resource(id)?,
                    last_sha256,
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.structural_key(), tombstone);
            }
        }
        for (path, old_entry) in &previous.files {
            if conversation_owned(&old_entry.resource_id) {
                continue;
            }
            let owner_removed = !recipe.entries.contains_key(&old_entry.resource_id);
            let recaptured_without_file = captured_ids.contains(&old_entry.resource_id)
                && !captured_file_paths.contains(path);
            if owner_removed || recaptured_without_file {
                let is_project_content = previous_resolved.kind(&old_entry.resource_id)
                    == Some(ResourceKindPrimitive::ProjectContentFile);
                let tombstone = Tombstone {
                    target: if is_project_content {
                        contracts::tombstone_project_content_file(&old_entry.resource_id, path)?
                    } else {
                        contracts::tombstone_file(&old_entry.resource_id, path)?
                    },
                    last_sha256: Some(old_entry.sha256.clone()),
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.structural_key(), tombstone);
                files.remove(path);
            }
        }
        for (path, old_entry) in &previous.directories {
            let owner_removed = !recipe.entries.contains_key(&old_entry.resource_id);
            let recaptured_without_directory = captured_ids.contains(&old_entry.resource_id)
                && !captured_directory_paths.contains(path);
            if owner_removed || recaptured_without_directory {
                let tombstone = Tombstone {
                    target: contracts::tombstone_project_content_directory(
                        &old_entry.resource_id,
                        path,
                    )?,
                    last_sha256: None,
                    deleted_at: updated_at,
                };
                tombstones.insert(tombstone.structural_key(), tombstone);
                directories.remove(path);
            }
        }
    }

    // Tombstones this build cannot decode stay preserved and inert.
    tombstones.retain(
        |_, tombstone| match contracts.tombstone_target(&tombstone.target) {
            Ok(Some(TombstoneTargetPrimitive::Resource { resource_id })) => {
                !resources.contains_key(&resource_id)
            }
            Ok(Some(TombstoneTargetPrimitive::File { logical_path, .. }))
            | Ok(Some(TombstoneTargetPrimitive::ProjectContentFile { logical_path, .. })) => {
                !files.contains_key(&logical_path) && !captured_file_paths.contains(&logical_path)
            }
            Ok(Some(TombstoneTargetPrimitive::ProjectContentDirectory {
                logical_path, ..
            })) => {
                !directories.contains_key(&logical_path)
                    && !captured_directory_paths.contains(&logical_path)
            }
            Ok(Some(TombstoneTargetPrimitive::ConversationThread { .. })) => true,
            Ok(None) | Err(_) => true,
        },
    );
    Ok((resources, files, directories, tombstones))
}

fn validate_scope_conversation_retention(
    contracts: &PortableResourceRegistry,
    previous: &BundleScope,
    input: &ScopePublishInput,
) -> Result<(), String> {
    for thread_id in previous.conversations.keys() {
        if input.conversations.contains_key(thread_id) {
            continue;
        }
        let deleted = input.tombstones.values().any(|tombstone| {
            matches!(
                contracts.tombstone_target(&tombstone.target),
                Ok(Some(TombstoneTargetPrimitive::ConversationThread { thread_id: deleted_thread_id }))
                    if &deleted_thread_id == thread_id
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

/// Semantic scope equality excludes `object_key` and revision coordinates.
/// Object keys are derived from content and do not define the reviewed scope
/// state, so a byte-identical recapture publishes nothing for that scope.
fn scope_content_is_semantic_noop(
    previous: &BundleScope,
    input: &ScopePublishInput,
    files: &BTreeMap<LogicalPath, BundleFileEntry>,
) -> bool {
    previous.recipe == input.recipe
        && previous.captured_with.codec_versions == input.captured_with.codec_versions
        && previous.conversations == input.conversations
        && previous.resources == input.resources
        && files_are_semantically_equal(&previous.files, files)
        && previous.directories == input.directories
        && previous.tombstones == input.tombstones
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
    let current_files: BTreeMap<&LogicalPath, &BundleFileEntry> = current.all_files().collect();
    let Some(previous) = previous else {
        return (current_files.len() as u64, 0, 0);
    };
    let previous_files: BTreeMap<&LogicalPath, &BundleFileEntry> = previous.all_files().collect();
    let added = current_files
        .keys()
        .filter(|path| !previous_files.contains_key(*path))
        .count() as u64;
    let changed = current_files
        .iter()
        .filter(|(path, entry)| {
            previous_files
                .get(*path)
                .is_some_and(|old| old.sha256 != entry.sha256)
        })
        .count() as u64;
    let removed = previous_files
        .keys()
        .filter(|path| !current_files.contains_key(*path))
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
    /// Why the apply stopped before it wrote any file, when it did: the
    /// storage could not be read. Every approved action then carries a
    /// "not pulled" receipt.
    pub stopped: Option<String>,
}

/// The result of an apply that could not start its writes: every approved
/// action is not pulled, every other one is skipped as it would have been,
/// and nothing was written or backed up.
pub fn unreached_apply_result(
    plan: &RestorePlan,
    approved_action_ids: &BTreeSet<ActionId>,
    applied_at: u64,
    stopped: Option<String>,
) -> ApplyBundleResult {
    ApplyBundleResult {
        plan_id: plan.plan_id.clone(),
        receipts: plan
            .actions
            .iter()
            .map(|action| {
                if approved_action_ids.contains(&action.action_id) {
                    receipt_for(
                        action,
                        ActionStatus::Failed,
                        applied_at,
                        None,
                        None,
                        Some(NOT_PULLED.to_string()),
                    )
                } else {
                    receipt_for(
                        action,
                        ActionStatus::Skipped,
                        applied_at,
                        None,
                        None,
                        Some("action was not approved".to_string()),
                    )
                }
            })
            .collect(),
        backups: Vec::new(),
        stopped,
    }
}

impl<S: ObjectStore> BundleEngine<S> {
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
        provider_homes: &BTreeMap<ProviderKey, String>,
        plan: &RestorePlan,
        approved_action_ids: &BTreeSet<ActionId>,
        backup_root: &Path,
        applied_at: u64,
    ) -> Result<ApplyBundleResult, String> {
        plan.validate()?;
        snapshot.validate()?;
        self.validate_binding(binding, provider_homes, Some(backup_root))?;
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

        // The apply reads the pinned version only. A newer publication
        // selected since the review does not matter: the pinned bytes are the
        // same, so no head is read here (PLAN_PULL_PINNED_VERSION.md, section
        // 4.2).
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
            if !pinned_noop && snapshot.manifest.find_file(logical_path).is_some() {
                prefetch_paths.push(logical_path.clone());
            }
        }
        if let Err(reason) = bytes.prefetch(&prefetch_paths) {
            // The storage cannot be read: the work ends before any file is
            // written, and every approved action is not pulled. Nothing is
            // rolled back and nothing is retried (PLAN_PULL_PINNED_VERSION.md,
            // section 4.3).
            return Ok(unreached_apply_result(
                plan,
                &effective_approved,
                applied_at,
                Some(reason),
            ));
        }

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
            match self.prepare_project_directory(binding, provider_homes, action) {
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
                receipts.push(receipt_for(
                    action,
                    ActionStatus::Skipped,
                    applied_at,
                    None,
                    None,
                    Some("action was not approved".to_string()),
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
                RestoreActionKind::MaterializeConversation { .. } => {
                    receipts.push(receipt_for(
                        action,
                        ActionStatus::Blocked,
                        applied_at,
                        None,
                        None,
                        Some(
                            "conversation materialization belongs to the typed conversation \
                             pipeline"
                                .to_string(),
                        ),
                    ));
                }
                RestoreActionKind::WriteFile { logical_path }
                | RestoreActionKind::WriteProjectFile { logical_path, .. }
                | RestoreActionKind::MergeFile { logical_path } => {
                    let receipt = self.apply_file_action(
                        snapshot,
                        bytes,
                        binding,
                        provider_homes,
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
                provider_homes,
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
            let receipt =
                self.apply_project_directory_deletion(binding, provider_homes, action, applied_at);
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
            let receipt =
                self.finalize_project_directory(binding, provider_homes, action, applied_at);
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
            stopped: None,
        })
    }

    fn prepare_project_directory(
        &self,
        binding: &ProjectBinding,
        provider_homes: &BTreeMap<ProviderKey, String>,
        action: &RestoreAction,
    ) -> Result<(), String> {
        let RestoreActionKind::EnsureProjectDirectory { logical_path, .. } = &action.kind else {
            return Err("directory preparation received another action kind".to_string());
        };
        let target = project_content_target(logical_path, binding)?;
        if action.target_path.as_deref() != Some(path_text(&target)?.as_str()) {
            return Err("project directory target differs from the pinned plan".to_string());
        }
        validate_target_for_logical(&target, logical_path, binding, provider_homes)?;
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
        provider_homes: &BTreeMap<ProviderKey, String>,
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
        validate_target_for_logical(&target, logical_path, binding, provider_homes)?;
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
        provider_homes: &BTreeMap<ProviderKey, String>,
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
        validate_target_for_logical(&target, logical_path, binding, provider_homes)?;
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
        provider_homes: &BTreeMap<ProviderKey, String>,
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
        validate_target_for_logical(&target, logical_path, binding, provider_homes)?;
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
        provider_homes: &BTreeMap<ProviderKey, String>,
        action: &RestoreAction,
        logical_path: &LogicalPath,
        plan_backup: &Path,
        applied_at: u64,
        backups: &mut Vec<BackupRecord>,
    ) -> Result<ApplyReceipt, String> {
        let (scope_key, entry) = snapshot
            .manifest
            .find_file(logical_path)
            .ok_or_else(|| format!("plan references missing file '{}'", logical_path))?;
        if entry.resource_id != action.resource_id {
            return Err("restore action resource changed after planning".to_string());
        }
        let scope = snapshot
            .manifest
            .scope(scope_key)
            .ok_or_else(|| "restore action scope is missing".to_string())?;
        if !scope.resources.contains_key(&entry.resource_id) {
            return Err("restore action resource is missing".to_string());
        }
        let resolved = scope.resolve(scope_key, self.contracts.as_ref())?;
        if resolved.kind(&entry.resource_id) == Some(ResourceKindPrimitive::Conversation) {
            return Err(format!(
                "conversation file '{}' belongs to the typed conversation pipeline",
                logical_path
            ));
        }
        let target = action
            .target_path
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| "file restore action has no target".to_string())?;
        validate_target_for_logical(&target, logical_path, binding, provider_homes)?;
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
        if let RestoreActionKind::WriteProjectFile {
            preserve: Some(pin),
            ..
        } = &action.kind
        {
            preserve_local_project_copy(&target, pin, binding, &before)?;
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
