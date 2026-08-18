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
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::provider_capture::CapturedResources;
use crate::providers::ProviderRegistry;
use mallard_domain::{
    self as domain, ActionId, ActionStatus, ApplyPolicy, ApplyReceipt, BindingState,
    BundleDirectoryEntry, BundleFileEntry, BundleHead, BundleId, BundleIdentity, BundleKind,
    BundleManifest, BundleRecipe, BundleScope, BundleSnapshot, CapturedWith, ConversationThread,
    LogicalPath, PlanId, PortableOrigin, ProjectBinding, ProviderKey, ResourceDescriptor,
    ResourceId, ResourceKind, RestoreAction, RestoreActionKind, RestorePlan, StorageId,
    SyncScopeKey, ThreadId, Tombstone, TombstoneTarget, BUNDLE_SCHEMA, RESTORE_PLAN_SCHEMA,
};

use crate::storage_core::{
    hex_lower, open_read_only_no_follow, parse_json_bytes, parse_json_reader,
    repository_object_key, sha256, sha256_reader, validate_bundle_relative_object_key,
    CasExpectation, CasOutcome, ExpectedObject, ObjectPrefix, ObjectStore, StageOutcome,
    VerifiedBody, HEAD_FILE, MAX_LIST_PAGE, REPOSITORY_PREFIX, TAG_FILE,
};

const MAX_BUNDLE_HEAD_BYTES: u64 = 64 * 1024;
const MAX_BUNDLE_TAG_BYTES: u64 = 64 * 1024;
const DEFAULT_PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;

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

/// The reviewed next content for one writable sync scope.
#[derive(Clone, Debug, Default)]
pub struct ScopePublishInput {
    pub recipe: BundleRecipe,
    pub captured_with: CapturedWith,
    pub conversations: BTreeMap<ThreadId, ConversationThread>,
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
/// the same revision and content digest (the mutation firewall), and a CAS
/// loss grafts the prepared writable scopes onto the winner as long as the
/// winner's writable-scope coordinates still match the reviewed pins.
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

impl<S: ObjectStore> BundleEngine<S> {
    pub fn open(store: S, storage_id: StorageId) -> Result<Self, String> {
        crate::storage_core::ensure_storage_marker(&store)?;
        Ok(Self { store, storage_id })
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

    /// Publish one complete reviewed bundle snapshot. The caller supplies the
    /// live resource and conversation sets; the engine enforces retention
    /// against the previous hash-verified manifest, skips true no-ops, and
    /// advances the head only through compare-and-swap. Uploads carry their
    /// sha256 checksum so the accepted PUT is the verification receipt;
    /// publish downloads nothing back. Durability audits are the separate
    /// `verify_objects` pass.
    #[cfg(any(test, feature = "test-support"))]
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

        let mut current = self.read_head(&request.identity.bundle_id)?;
        match (&request.expected_head, &current) {
            (PublishExpectation::Absent, None) => {}
            (PublishExpectation::Absent, Some(_)) => {
                return Err("bundle head changed: expected an absent head".to_string())
            }
            (PublishExpectation::Match(_), None) => {
                return Err("bundle head is missing; the remote repository was removed".to_string())
            }
            // A moved head is not by itself fatal: disjoint-scope publishers
            // may land between review and push. The per-scope firewall below
            // decides whether this publication can graft onto the winner.
            (PublishExpectation::Match(_), Some(_)) => {}
        }
        on_phase("publish: read head", phase_started.elapsed());

        const MAX_CAS_ATTEMPTS: usize = 4;
        for attempt in 0..MAX_CAS_ATTEMPTS {
            let phase_started = Instant::now();
            let previous_manifest = current
                .as_ref()
                .map(|(head, _)| self.read_manifest(head))
                .transpose()?;

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
                        "sync scope '{}' changed since review; pull and replan before pushing",
                        scope_key
                    ));
                }
            }
            for (scope_key, input) in &request.writable_scopes {
                let previous_scope = previous_manifest
                    .as_ref()
                    .and_then(|manifest| manifest.scopes.get(scope_key));
                for (path, entry) in &input.retained_files {
                    entry.validate()?;
                    let was_published = previous_scope.is_some_and(|scope| {
                        scope.files.values().any(|candidate| {
                            candidate.object_key == entry.object_key
                                && candidate.sha256 == entry.sha256
                                && candidate.size == entry.size
                        })
                    });
                    if !was_published {
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
                let mut files = input.retained_files.clone();
                for (path, captured) in &input.captured_files {
                    if captured.mode.is_some_and(|mode| mode & !0o777 != 0) {
                        return Err(format!("bundle file '{}' has unsafe mode bits", path));
                    }
                    if !input.resources.contains_key(&captured.resource_id) {
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
                if let Some(previous_scope) = previous_scope {
                    validate_scope_conversation_retention(previous_scope, input)?;
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
                for captured in input.captured_files.values() {
                    pending_uploads
                        .entry(sha256(&captured.bytes))
                        .or_insert_with(|| Bytes::from(captured.bytes.clone()));
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
                let (head, _) = current
                    .as_ref()
                    .expect("a semantic noop requires a current head");
                return Ok(PublishedBundle {
                    published: false,
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
            let commit_id = random_named_id("commit")?;
            let manifest = BundleManifest {
                schema_version: BUNDLE_SCHEMA,
                revision,
                commit_id: commit_id.clone(),
                updated_at: request.updated_at,
                bundle: request.identity.clone(),
                published_by_origin: request.published_by_origin.clone(),
                scopes,
            };
            manifest.validate()?;

            // The final mutation firewall at the request boundary: a
            // publication may change only scopes named by the reviewed
            // writable-scope set. Every other scope must retain its exact
            // revision and content digest from the CAS input manifest.
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
                published_by_origin: request.published_by_origin.clone(),
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
            let head_bytes = serde_json::to_vec(&head)
                .map_err(|error| format!("serialize bundle head: {error}"))?;
            let head_key = repository_object_key(&request.identity.bundle_id, HEAD_FILE)?;
            let cas_expectation = match &current {
                Some((_, token)) => CasExpectation::Match(token.etag.clone()),
                None => CasExpectation::Absent,
            };
            match self
                .store
                .compare_and_swap(&head_key, &cas_expectation, &head_bytes)?
            {
                CasOutcome::Written { .. } => {
                    let _ = self.publish_tag(&manifest);
                    on_phase("publish: swing head", phase_started.elapsed());
                    return Ok(PublishedBundle {
                        published: true,
                        snapshot: BundleSnapshot {
                            storage_id: self.storage_id.clone(),
                            head: head.clone(),
                            manifest,
                            fetched_at: request.updated_at,
                        },
                    });
                }
                CasOutcome::Conflict { .. } => {
                    on_phase("publish: head CAS conflict", phase_started.elapsed());
                    if attempt + 1 == MAX_CAS_ATTEMPTS {
                        return Err("bundle head CAS conflict; fetch and rebase before retrying"
                            .to_string());
                    }
                    // Graft retry: read the winner and re-verify the reviewed
                    // writable-scope pins against it on the next iteration.
                    current = self.read_head(&request.identity.bundle_id)?;
                    if current.is_none() {
                        return Err(
                            "bundle head disappeared during publication; the repository was \
                             removed"
                                .to_string(),
                        );
                    }
                }
            }
        }
        Err("bundle head CAS conflict; fetch and rebase before retrying".to_string())
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
        for (_, entry) in snapshot.manifest.all_files() {
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
    /// Build the file-level restore plan for one verified snapshot.
    ///
    /// Conversation resources never appear here: the typed conversation
    /// planner owns them for every provider. Provider-state files are
    /// planned only for bound providers with a resolved home; unbound and
    /// unknown provider scopes stay view-only and untouched.
    pub fn build_restore_plan_from_snapshot(
        &self,
        registry: &ProviderRegistry,
        snapshot: &BundleSnapshot,
        binding: &ProjectBinding,
        provider_homes: &BTreeMap<ProviderKey, String>,
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
                    requires_explicit_approval: true,
                });
            }

            // Files, pass 1: inspect targets and decide the metadata-only
            // outcome.
            let mut planned_files = Vec::new();
            for (logical_path, entry) in &scope.files {
                let descriptor = scope
                    .resources
                    .get(&entry.resource_id)
                    .ok_or_else(|| format!("file '{}' has no resource", logical_path))?;
                if descriptor.kind == ResourceKind::Conversation {
                    continue;
                }
                let target = match (&scope_home, descriptor.kind) {
                    (None, _) => map_project_logical_target(logical_path, binding)?,
                    (Some((provider, home)), ResourceKind::ProviderState) => provider_state_target(
                        registry,
                        provider,
                        home,
                        binding,
                        descriptor,
                        logical_path,
                    ),
                    (Some(_), _) => None,
                };
                let Some(target) = target else {
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
                let outcome = if descriptor.kind != ResourceKind::ProjectContentFile
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
            }

            for tombstone in scope.tombstones.values() {
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
            resources: manifest.all_resources().count() as u64,
            files: manifest.all_files().count() as u64,
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
    use crate::storage_core::{
        ImmutablePutOutcome, LocalObjectStore, ObjectKey, StoreListPage, StoredMetadata,
        STORAGE_MARKER_KEY,
    };
    use mallard_domain::{LocalProjectId, RecipeEntry, ReplicaId};

    fn bundle_id(number: u64) -> BundleId {
        BundleId::parse(format!("{:032x}", number)).unwrap()
    }

    fn storage_id() -> StorageId {
        StorageId::parse("storage-local").unwrap()
    }

    fn registry() -> &'static ProviderRegistry {
        ProviderRegistry::production()
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
    /// the capture into its sync scopes, treat every selected or previously
    /// present scope as writable, and pin each one against the observed
    /// head's manifest.
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
            let (resources, mut retained_files, mut directories, tombstones) =
                reconcile_scope_content(
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
                let digest = sha256(&file.bytes);
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
                    .map(|(_, token)| PublishExpectation::Match(token))
                    .unwrap_or(PublishExpectation::Absent),
                updated_at,
            },
            on_progress,
            &mut |_, _| {},
        )
    }

    #[test]
    fn current_manifest_read_enforces_the_head_manifest_size() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
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
        let id = bundle_id(91);
        let store_root = temp.path().join("store");
        let engine =
            BundleEngine::open(LocalObjectStore::open(&store_root).unwrap(), storage_id()).unwrap();
        publish_captured(
            &engine,
            identity(id.clone(), "Sized manifest"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
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
        let engine = BundleEngine::open(store, storage_id()).unwrap();
        let first = publish_captured(
            &engine,
            identity(id.clone(), "Project"),
            &recipe,
            &captured,
            None,
            10,
        )
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
        let path = project_scope(&snapshot.manifest)
            .files
            .keys()
            .next()
            .unwrap()
            .clone();
        let cache = Arc::new(ObjectCache::default());
        let mut bytes = engine.byte_source(&snapshot, Some(cache.clone()), None);
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
        let mut bytes = engine.byte_source(&wrong_size, Some(cache), None);
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
            Some((first.snapshot.head.clone(), first_token.clone())),
            20,
        )
        .unwrap();
        assert_eq!(second.snapshot.head.revision, 2);
        let second_token = engine.read_head(&id).unwrap().unwrap().1;
        assert!(publish_captured(
            &engine,
            identity(id.clone(), "Stale"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first_token)),
            21,
        )
        .is_err());

        let empty_recipe = BundleRecipe::default();
        let removed = publish_captured(
            &engine,
            identity(id.clone(), "Renamed Project"),
            &empty_recipe,
            &capture_recipe(registry(), &request, &empty_recipe).unwrap(),
            Some((second.snapshot.head.clone(), second_token)),
            30,
        )
        .unwrap();
        assert!(removed.snapshot.manifest.all_files().next().is_none());
        assert!(removed.snapshot.manifest.all_resources().next().is_none());
        assert!(removed
            .snapshot
            .manifest
            .all_tombstones()
            .any(|(_, _, tombstone)| matches!(tombstone.target, TombstoneTarget::Resource { .. })));
    }

    #[test]
    fn remote_bundle_listing_pages_more_than_one_thousand_bundles() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
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
        let mut bytes = engine.byte_source(&snapshot, None, None);
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
    }

    /// Store wrapper recording every staged/metadata key and every write so
    /// tests can prove which payload objects an operation actually downloaded
    /// or uploaded. The trait-default `put_many` routes through
    /// `put_immutable`, so bulk uploads are recorded per object.
    #[derive(Clone)]
    struct CountingStore {
        inner: LocalObjectStore,
        keys: Arc<Mutex<Vec<String>>>,
        puts: Arc<Mutex<Vec<String>>>,
        cas_writes: Arc<Mutex<Vec<String>>>,
    }

    impl CountingStore {
        fn open(root: &Path) -> Self {
            Self {
                inner: LocalObjectStore::open(root).unwrap(),
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

        fn delete(&self, key: &ObjectKey) -> Result<(), String> {
            self.inner.delete(key)
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
        assert!(first.published);
        let first_token = engine.read_head(&id).unwrap().unwrap().1;
        assert_eq!(store.payload_puts().len(), 2);
        for entry in project_scope(&first.snapshot.manifest).files.values() {
            assert_eq!(entry.object_key, format!("_objects/{}", entry.sha256));
        }

        write(&source.join("changing.md"), b"second draft");
        store.reset();
        let second = publish_captured(
            &engine,
            identity(id.clone(), "Incremental"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first_token)),
            20,
        )
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
        let stable_path = project_scope(&second.snapshot.manifest)
            .files
            .iter()
            .find(|(_, entry)| entry.sha256 == sha256(b"stable contents"))
            .map(|(path, _)| path.clone())
            .unwrap();
        assert_eq!(
            project_scope(&second.snapshot.manifest).files[&stable_path],
            project_scope(&first.snapshot.manifest).files[&stable_path],
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
        let inventory = discover_project(registry(), &request).unwrap();
        let recipe = recipe_for(
            inventory
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone()),
        );
        let id = bundle_id(77);
        let store = CountingStore::open(&temp.path().join("store"));
        let engine = BundleEngine::open(store.clone(), storage_id()).unwrap();
        let published = publish_captured(
            &engine,
            identity(id.clone(), "Probe"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            None,
            10,
        )
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
        let shared_key = format!("_objects/{}", sha256(b"same bytes either way"));
        let file_entries: Vec<_> = project_scope(&published.snapshot.manifest)
            .files
            .values()
            .collect();
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
        assert!(first.published);
        let first_token = engine.read_head(&id).unwrap().unwrap().1;

        store.reset();
        let noop = publish_captured(
            &engine,
            identity(id.clone(), "Noop"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first_token.clone())),
            20,
        )
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
            Some((first.snapshot.head.clone(), first_token.clone())),
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
        publish_captured_with_progress(
            &engine,
            identity(id.clone(), "Progress"),
            &recipe,
            &capture_recipe(registry(), &request, &recipe).unwrap(),
            Some((first.snapshot.head.clone(), first_token)),
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
            .build_restore_plan_from_snapshot(
                registry(),
                &snapshot,
                &binding,
                &provider_homes,
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
                &provider_homes,
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
            provider_homes: BTreeMap::from([(claude_key(), source_claude)]),
            excluded_project_roots: Vec::new(),
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
            excluded_project_roots: Vec::new(),
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

fn restore_kind_for_file(
    logical_path: &LogicalPath,
    descriptor: &ResourceDescriptor,
    entry: &BundleFileEntry,
) -> Result<RestoreActionKind, String> {
    Ok(match descriptor.kind {
        ResourceKind::Conversation => {
            return Err(format!(
                "conversation file '{}' belongs to the typed conversation planner",
                logical_path
            ))
        }
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

/// Map one project-scope logical path to its machine target. `project/`
/// content lands under the canonical project root; `requirements/` and
/// `dependencies/` entries have no safe materializer and stay manual.
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
    registry: &ProviderRegistry,
    provider: &ProviderKey,
    provider_home: &str,
    binding: &ProjectBinding,
    descriptor: &ResourceDescriptor,
    logical_path: &LogicalPath,
) -> Option<PathBuf> {
    let prefix = mallard_domain::provider_state_path_prefix(provider);
    let suffix = logical_path.as_str().strip_prefix(prefix.as_str())?;
    let sessions = registry.sessions(provider)?;
    sessions
        .state_target(
            Path::new(provider_home),
            &binding.project_root,
            descriptor.relative_cwd.as_deref(),
            suffix,
        )
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

pub fn write_immutable_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
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

/// The reconciled next-revision manifest content: descriptors, file entries,
/// directories, and tombstones.
pub type ReconciledManifestContent = (
    BTreeMap<ResourceId, ResourceDescriptor>,
    BTreeMap<LogicalPath, BundleFileEntry>,
    BTreeMap<LogicalPath, BundleDirectoryEntry>,
    BTreeMap<String, Tombstone>,
);

/// Reconcile one sync scope's next content from its reviewed recipe, this
/// capture, and the verified previous scope. Tombstones are minted only
/// inside the scope being reconciled, so no tombstone can ever target an
/// unbound or unknown provider scope.
pub fn reconcile_scope_content(
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
            previous
                .resources
                .get(resource_id)
                .is_some_and(|descriptor| descriptor.kind == ResourceKind::Conversation)
        };
        for (id, descriptor) in &previous.resources {
            if descriptor.kind == ResourceKind::Conversation {
                continue;
            }
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
            if conversation_owned(&old_entry.resource_id) {
                continue;
            }
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

fn validate_scope_conversation_retention(
    previous: &BundleScope,
    input: &ScopePublishInput,
) -> Result<(), String> {
    for thread_id in previous.conversations.keys() {
        if input.conversations.contains_key(thread_id) {
            continue;
        }
        let deleted = input.tombstones.values().any(|tombstone| {
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
            if !pinned_noop && snapshot.manifest.find_file(logical_path).is_some() {
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
        let descriptor = scope
            .resources
            .get(&entry.resource_id)
            .ok_or_else(|| "restore action resource is missing".to_string())?;
        if descriptor.kind == ResourceKind::Conversation {
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
