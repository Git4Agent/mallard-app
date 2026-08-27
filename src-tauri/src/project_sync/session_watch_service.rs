//! Watcher-backed reuse of provider session inventory and cursor-provider
//! tail observations.
//!
//! One watcher per exact canonical provider home, armed lazily on the first
//! operation that consults it and shared by every project bound to that
//! profile. Watcher silence between reliability signals is the reuse
//! witness: a clean profile serves cached results with no filesystem work,
//! a dirty path recomputes only itself, and any reliability doubt drops the
//! profile's caches and rebaselines through the same bounded path a cold
//! start uses. The watcher is a reuse oracle, never a correctness
//! authority: nothing here promotes accepted state, authorizes an action,
//! or replaces an authoritative plan/apply digest, and apply keeps every
//! existing verification, so stale reuse can at worst produce a plan that
//! apply rejects.
//!
//! Tests run the manual-poll backend (content-compared, polled at every
//! drain), so event delivery is deterministic: a mutation between two
//! operations is always visible to the second one.

use super::commands::LocalTailObservation;
use mallard_domain::{LocalProjectId, ProviderKey, StorageId, TargetExpectation, ThreadId};
use mallard_infrastructure::persistence::write_json_atomic;
use mallard_infrastructure::provider_capture::{self, CaptureRequest, Discovery, LocalSessionView};
use mallard_infrastructure::sync_metrics;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Bounded process-local state: past any bound the profile drops to the
/// bounded path rather than evicting part of its evidence.
const MAX_WATCHED_PROFILES: usize = 64;
const MAX_DIRTY_ROOTS: usize = 4_096;
const MAX_OBSERVATION_ENTRIES: usize = 4_096;
/// On-disk observation cache: bounded like every other `~/.mallard` file,
/// versioned so a shape change discards rather than misreads.
const OBSERVATION_DISK_SCHEMA: u32 = 1;
const MAX_OBSERVATION_DISK_BYTES: u64 = 16 * 1024 * 1024;
const OBSERVATION_DISK_FILE: &str = "observation_cache.json";
const EVENT_QUEUE_DEPTH: usize = 8_192;

enum WatchBackend {
    Recommended(RecommendedWatcher),
    /// Deterministic test backend: a synchronous scanner that diffs full
    /// file contents on every drain, so a mutation between two operations
    /// — same-size, same-second included — is always visible to the second
    /// one, and never earlier or later.
    Manual(ManualScanner),
}

impl WatchBackend {
    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        match self {
            WatchBackend::Recommended(watcher) => watcher.watch(path, mode),
            WatchBackend::Manual(scanner) => {
                scanner.watch(path, mode);
                Ok(())
            }
        }
    }

    fn poll(&mut self) -> notify::Result<()> {
        match self {
            WatchBackend::Recommended(_) => Ok(()),
            WatchBackend::Manual(scanner) => {
                scanner.poll();
                Ok(())
            }
        }
    }
}

/// The deterministic test scanner behind `WatchBackend::Manual`.
struct ManualScanner {
    sender: SyncSender<notify::Result<Event>>,
    roots: Vec<(PathBuf, RecursiveMode)>,
    baseline: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

impl ManualScanner {
    fn new(sender: SyncSender<notify::Result<Event>>) -> Self {
        Self {
            sender,
            roots: Vec::new(),
            baseline: BTreeMap::new(),
        }
    }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) {
        self.roots.push((path.to_path_buf(), mode));
        // Baseline without emitting: arming precedes the first scan.
        self.baseline = self.scan();
    }

    fn scan(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut seen = BTreeMap::new();
        for (root, mode) in &self.roots {
            if !root.exists() {
                continue;
            }
            seen.insert(root.clone(), None);
            let mut stack = vec![root.clone()];
            while let Some(directory) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&directory) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    if file_type.is_dir() {
                        seen.insert(path.clone(), None);
                        if matches!(mode, RecursiveMode::Recursive) {
                            stack.push(path);
                        }
                    } else if file_type.is_file() {
                        // Content comparison is what makes the scanner
                        // deterministic; files past the conversation bound
                        // compare by length alone — they are blocked rows
                        // either way.
                        let content = match entry.metadata() {
                            Ok(meta) if meta.len() <= mallard_infrastructure::app_settings::conversation_file_limit_bytes() => {
                                std::fs::read(&path).ok()
                            }
                            Ok(meta) => Some(meta.len().to_le_bytes().to_vec()),
                            Err(_) => None,
                        };
                        seen.insert(path.clone(), content);
                    }
                }
            }
        }
        seen
    }

    fn emit(&self, kind: EventKind, path: &Path) {
        let _ = self
            .sender
            .try_send(Ok(Event::new(kind).add_path(path.to_path_buf())));
    }

    fn poll(&mut self) {
        use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind};
        let current = self.scan();
        for (path, content) in &current {
            match self.baseline.get(path) {
                None => self.emit(EventKind::Create(CreateKind::Any), path),
                Some(previous) if previous != content => self.emit(
                    EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                    path,
                ),
                _ => {}
            }
        }
        for path in self.baseline.keys() {
            if !current.contains_key(path) {
                self.emit(EventKind::Remove(RemoveKind::Any), path);
            }
        }
        self.baseline = current;
    }
}

struct ProfileWatch {
    provider: ProviderKey,
    canonical_home: PathBuf,
    backend: WatchBackend,
    receiver: Receiver<notify::Result<Event>>,
    overflowed: Arc<AtomicBool>,
    watched_session_roots: BTreeSet<PathBuf>,
    healthy: bool,
    generation: u64,
    dirty_roots: BTreeSet<PathBuf>,
    inventory_dirty: bool,
    titles_dirty: bool,
    /// Root paths named by the cached inventory, so an event on an unknown
    /// JSONL path is classified as inventory work.
    known_roots: BTreeSet<PathBuf>,
    last_used: std::time::Instant,
}

struct CachedInventory {
    request: CaptureRequest,
    discovery: Discovery,
    views: Vec<LocalSessionView>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ObservationKey {
    canonical_home: PathBuf,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    thread_id: ThreadId,
}

struct CachedObservation {
    /// Every physical segment the derivation read; a dirty event on any of
    /// them invalidates this entry. The cursor path depends on its leaf
    /// alone, exactly like the authoritative cursor; a full resolve depends
    /// on its complete segment chain.
    depends_on: Vec<PathBuf>,
    checkpoint_generation: Option<u64>,
    binding_revision: Option<u64>,
    observation: LocalTailObservation,
}

/// One candidate's cached byte pin: the review-time local-target digest
/// for a push/pull candidate whose transcript the watcher proves
/// unchanged. The pin freezes review-to-apply state, so serving a stale
/// one can only make apply fail the stale-pin check. Advisory secret
/// warnings surface whenever a candidate is actually captured — the first
/// review, and any review after its transcript changed.
struct CachedCandidatePin {
    depends_on: Vec<PathBuf>,
    expected_local_target: TargetExpectation,
}

/// One freshly captured candidate pin to retain.
pub(crate) struct StoredCandidatePin {
    pub(crate) resource_id: String,
    pub(crate) depends_on: Vec<PathBuf>,
    pub(crate) expected_local_target: TargetExpectation,
}

/// One reusable-observation lookup.
pub(crate) struct ObservationQuery {
    pub(crate) thread_id: ThreadId,
    pub(crate) checkpoint_generation: Option<u64>,
    pub(crate) binding_revision: Option<u64>,
}

/// One derived observation to retain for the next operation.
pub(crate) struct StoredObservation {
    pub(crate) thread_id: ThreadId,
    pub(crate) source_path: PathBuf,
    pub(crate) depends_on: Vec<PathBuf>,
    pub(crate) checkpoint_generation: Option<u64>,
    pub(crate) binding_revision: Option<u64>,
    pub(crate) observation: LocalTailObservation,
    /// Discovery-time stat of `source_path` (whole seconds, bytes). The
    /// disk layer persists an entry only when the store-time stat still
    /// matches: a drift means the source changed while the review ran, and
    /// a pin taken now would vouch for bytes the derivation never read.
    pub(crate) source_mtime_secs: u64,
    pub(crate) source_size: u64,
}

/// One dependency's identity pin for the disk cache: the exact stat of the
/// file the derivation read. Serving requires every pin to match a fresh
/// stat, so validity never rests on watcher events that a past process
/// consumed. Nanosecond mtime plus size is the strongest identity a stat
/// gives; a same-size same-mtime rewrite is the accepted residual risk, and
/// review output is a reuse artifact, never a correctness authority.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct DiskPathPin {
    path: PathBuf,
    size: u64,
    mtime_nanos: u64,
}

/// One persisted observation. `checkpoint_generation` and
/// `binding_revision` repeat the in-memory query pins; `depends_on` carries
/// stat pins instead of bare paths because the disk cache outlives the
/// watcher that would otherwise witness quiet.
#[derive(Serialize, Deserialize, Clone)]
struct DiskObservationRecord {
    canonical_home: PathBuf,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    thread_id: ThreadId,
    checkpoint_generation: Option<u64>,
    binding_revision: Option<u64>,
    depends_on: Vec<DiskPathPin>,
    observation: LocalTailObservation,
    stored_at: u64,
}

impl DiskObservationRecord {
    fn key(&self) -> ObservationKey {
        ObservationKey {
            canonical_home: self.canonical_home.clone(),
            local_project_id: self.local_project_id.clone(),
            storage_id: self.storage_id.clone(),
            thread_id: self.thread_id.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ObservationDiskCache {
    schema: u32,
    entries: Vec<DiskObservationRecord>,
}

fn observation_cache_path(state_root: &Path) -> PathBuf {
    state_root.join(OBSERVATION_DISK_FILE)
}

fn modified_nanos(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn path_pin(path: &Path) -> Option<DiskPathPin> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some(DiskPathPin {
        path: path.to_path_buf(),
        size: metadata.len(),
        mtime_nanos: modified_nanos(&metadata),
    })
}

/// A pin holds when a fresh stat reproduces it exactly.
fn pin_matches(pin: &DiskPathPin) -> bool {
    path_pin(&pin.path).as_ref() == Some(pin)
}

fn load_observation_disk_cache(
    state_root: &Path,
) -> BTreeMap<ObservationKey, DiskObservationRecord> {
    let path = observation_cache_path(state_root);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return BTreeMap::new();
    };
    if !metadata.is_file() || metadata.len() > MAX_OBSERVATION_DISK_BYTES {
        return BTreeMap::new();
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return BTreeMap::new();
    };
    let Ok(cache) = serde_json::from_slice::<ObservationDiskCache>(&bytes) else {
        return BTreeMap::new();
    };
    if cache.schema != OBSERVATION_DISK_SCHEMA || cache.entries.len() > MAX_OBSERVATION_ENTRIES {
        return BTreeMap::new();
    }
    cache
        .entries
        .into_iter()
        .map(|record| (record.key(), record))
        .collect()
}

/// Rewrite the cache file from the mirror, evicting the oldest records past
/// the entry bound. A failed write only costs the next cold start its reuse.
fn write_observation_disk_cache(
    state_root: &Path,
    entries: &mut BTreeMap<ObservationKey, DiskObservationRecord>,
) {
    while entries.len() > MAX_OBSERVATION_ENTRIES {
        let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, record)| record.stored_at)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        entries.remove(&oldest);
    }
    let cache = ObservationDiskCache {
        schema: OBSERVATION_DISK_SCHEMA,
        entries: entries.values().cloned().collect(),
    };
    let _ = write_json_atomic(
        state_root,
        &observation_cache_path(state_root),
        &cache,
        MAX_OBSERVATION_DISK_BYTES,
    );
}

#[derive(Default)]
struct ServiceInner {
    profiles: BTreeMap<PathBuf, ProfileWatch>,
    inventories: BTreeMap<(ProviderKey, PathBuf, LocalProjectId), CachedInventory>,
    observations: BTreeMap<ObservationKey, CachedObservation>,
    candidate_pins: BTreeMap<(PathBuf, LocalProjectId, String), CachedCandidatePin>,
    /// Per state-root mirror of the on-disk observation cache, loaded
    /// lazily on first use and written through on every store.
    disk: BTreeMap<PathBuf, BTreeMap<ObservationKey, DiskObservationRecord>>,
}

pub(crate) struct SessionWatchService {
    inner: Mutex<ServiceInner>,
}

pub(crate) fn session_watch_service() -> &'static SessionWatchService {
    static SERVICE: OnceLock<SessionWatchService> = OnceLock::new();
    SERVICE.get_or_init(|| SessionWatchService {
        inner: Mutex::new(ServiceInner::default()),
    })
}

fn arm_backend(
    sender: SyncSender<notify::Result<Event>>,
    overflowed: Arc<AtomicBool>,
) -> notify::Result<WatchBackend> {
    if cfg!(test) {
        return Ok(WatchBackend::Manual(ManualScanner::new(sender)));
    }
    let handler = move |event: notify::Result<Event>| {
        // The callback only forwards: no filesystem work, no parsing, no
        // classification. A full queue is a reliability signal.
        if sender.try_send(event).is_err() {
            overflowed.store(true, Ordering::Relaxed);
        }
    };
    Ok(WatchBackend::Recommended(RecommendedWatcher::new(
        handler,
        Config::default(),
    )?))
}

/// The exact canonical home every cache key, watch root, and dirty path
/// uses. Discovery canonicalizes its walk roots, so service state must
/// live on the same canonical paths or dirty events would never match
/// cached dependencies (macOS tempdirs alias /var to /private/var).
fn canonical_home_of(home: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(home).ok()
}

/// Canonicalize every stored dependency path; an entry whose sources
/// cannot be canonicalized is simply not retained.
fn canonical_paths(paths: &[PathBuf]) -> Option<Vec<PathBuf>> {
    paths
        .iter()
        .map(|path| std::fs::canonicalize(path).ok())
        .collect()
}

/// Normalize a delivered watcher path to the same spelling used by profile
/// and dependency keys. Existing paths canonicalize directly. Removal events
/// may name a path that has already disappeared, so canonicalize its nearest
/// existing ancestor and append the missing suffix without following it.
fn canonical_event_path(path: &Path) -> PathBuf {
    let mut current = path;
    let mut missing = Vec::new();
    loop {
        if let Ok(mut canonical) = std::fs::canonicalize(current) {
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        let Some(name) = current.file_name() else {
            return path.to_path_buf();
        };
        missing.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            return path.to_path_buf();
        };
        current = parent;
    }
}

fn session_root_names(provider: &ProviderKey) -> &'static [&'static str] {
    match provider.as_str() {
        "codex" => &["sessions", "archived_sessions"],
        "claude" => &["projects", "file-history", "todos"],
        _ => &[],
    }
}

fn session_roots(provider: &ProviderKey, canonical_home: &Path) -> Vec<PathBuf> {
    session_root_names(provider)
        .iter()
        .map(|name| canonical_home.join(name))
        .filter(|path| path.is_dir())
        .collect()
}

fn arm_profile(
    provider: &ProviderKey,
    canonical_home: &Path,
) -> Result<ProfileWatch, notify::Error> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(EVENT_QUEUE_DEPTH);
    let overflowed = Arc::new(AtomicBool::new(false));
    let mut backend = arm_backend(sender, Arc::clone(&overflowed))?;
    let mut watched_session_roots = BTreeSet::new();
    for root in session_roots(provider, canonical_home) {
        backend.watch(&root, RecursiveMode::Recursive)?;
        watched_session_roots.insert(root);
    }
    // The home itself, non-recursively: creation or replacement of the
    // session trees and atomic replacement of `session_index.jsonl`.
    backend.watch(canonical_home, RecursiveMode::NonRecursive)?;
    Ok(ProfileWatch {
        provider: provider.clone(),
        canonical_home: canonical_home.to_path_buf(),
        backend,
        receiver,
        overflowed,
        watched_session_roots,
        healthy: true,
        generation: 0,
        dirty_roots: BTreeSet::new(),
        inventory_dirty: false,
        titles_dirty: false,
        known_roots: BTreeSet::new(),
        last_used: std::time::Instant::now(),
    })
}

impl ProfileWatch {
    /// Classify one delivered event into the profile's dirty state.
    fn classify(&mut self, event: &Event) {
        if event.need_rescan() {
            self.healthy = false;
            return;
        }
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if event.paths.is_empty() {
            // Without a path the change cannot be attributed.
            self.healthy = false;
            return;
        }
        for delivered_path in &event.paths {
            let canonical_path = canonical_event_path(delivered_path);
            let path = canonical_path.as_path();
            if self.provider.as_str() == "codex"
                && path.file_name().and_then(|name| name.to_str()) == Some("session_index.jsonl")
                && path.parent() == Some(self.canonical_home.as_path())
            {
                self.titles_dirty = true;
                continue;
            }
            let under_sessions = self
                .watched_session_roots
                .iter()
                .any(|root| path.starts_with(root));
            if self.watched_session_roots.contains(path)
                && matches!(event.kind, EventKind::Remove(_) | EventKind::Modify(_))
            {
                // A watched session root moved or was replaced: the watch
                // itself can no longer be trusted.
                self.healthy = false;
                return;
            }
            if !under_sessions && path != self.canonical_home {
                // Direct children of the home (session tree creation) are
                // membership changes; unrelated home files are not.
                if path.parent() == Some(self.canonical_home.as_path())
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| session_root_names(&self.provider).contains(&name))
                {
                    self.inventory_dirty = true;
                    self.generation = self.generation.wrapping_add(1);
                }
                continue;
            }
            let is_jsonl = path.extension().and_then(|ext| ext.to_str()) == Some("jsonl");
            if is_jsonl {
                self.generation = self.generation.wrapping_add(1);
                self.dirty_roots.insert(path.to_path_buf());
                let membership_change =
                    matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_))
                        || !self.known_roots.contains(path);
                if membership_change {
                    self.inventory_dirty = true;
                }
            } else {
                // Directory churn or non-transcript files inside the
                // session trees are membership work.
                self.inventory_dirty = true;
                self.generation = self.generation.wrapping_add(1);
            }
        }
        if self.dirty_roots.len() > MAX_DIRTY_ROOTS {
            self.healthy = false;
        }
    }

    /// Poll (manual backend), then drain and classify every queued event.
    fn drain(&mut self) {
        if self.backend.poll().is_err() {
            self.healthy = false;
            return;
        }
        loop {
            match self.receiver.try_recv() {
                Ok(Ok(event)) => self.classify(&event),
                Ok(Err(_)) => {
                    self.healthy = false;
                    return;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.healthy = false;
                    return;
                }
            }
        }
        if self.overflowed.swap(false, Ordering::Relaxed) {
            self.healthy = false;
        }
    }
}

impl ServiceInner {
    fn drop_profile_caches(&mut self, canonical_home: &Path) {
        self.inventories
            .retain(|(_, home, _), _| home != canonical_home);
        self.observations
            .retain(|key, _| key.canonical_home != canonical_home);
        self.candidate_pins
            .retain(|(home, _, _), _| home != canonical_home);
    }

    /// Arm (or re-arm) the profile and drain its queue. Returns whether the
    /// profile is healthy; an unhealthy or unarmable profile has no caches
    /// and every operation runs the bounded path.
    fn ensure_profile(&mut self, provider: &ProviderKey, canonical_home: &Path) -> bool {
        if self.profiles.len() > MAX_WATCHED_PROFILES {
            self.evict_stale_profiles();
        }
        let rearm = match self.profiles.get(canonical_home) {
            Some(profile) => {
                // A session tree that appeared after arming needs a
                // recursive watch it never got.
                profile.provider != *provider
                    || !profile.healthy
                    || session_roots(provider, canonical_home)
                        .iter()
                        .any(|root| !profile.watched_session_roots.contains(root))
            }
            None => true,
        };
        if rearm {
            self.drop_profile_caches(canonical_home);
            self.profiles.remove(canonical_home);
            match arm_profile(provider, canonical_home) {
                Ok(profile) => {
                    self.profiles.insert(canonical_home.to_path_buf(), profile);
                }
                Err(_) => return false,
            }
        }
        let Some(profile) = self.profiles.get_mut(canonical_home) else {
            return false;
        };
        profile.last_used = std::time::Instant::now();
        profile.drain();
        if !profile.healthy {
            let home = profile.canonical_home.clone();
            self.drop_profile_caches(&home);
            return false;
        }
        true
    }

    /// Drop profiles whose home no longer exists (finished test fixtures,
    /// removed provider setups), then the least recently used ones.
    fn disk_entries(
        &mut self,
        state_root: &Path,
    ) -> &mut BTreeMap<ObservationKey, DiskObservationRecord> {
        self.disk
            .entry(state_root.to_path_buf())
            .or_insert_with(|| load_observation_disk_cache(state_root))
    }

    fn evict_stale_profiles(&mut self) {
        let stale: Vec<PathBuf> = self
            .profiles
            .keys()
            .filter(|home| !home.is_dir())
            .cloned()
            .collect();
        for home in stale {
            self.drop_profile_caches(&home);
            self.profiles.remove(&home);
        }
        while self.profiles.len() > MAX_WATCHED_PROFILES {
            let Some(oldest) = self
                .profiles
                .values()
                .min_by_key(|profile| profile.last_used)
                .map(|profile| profile.canonical_home.clone())
            else {
                break;
            };
            self.drop_profile_caches(&oldest);
            self.profiles.remove(&oldest);
        }
    }
}

impl SessionWatchService {
    /// Forget every armed profile and cache. The whole-`~/.mallard` reset
    /// invalidates the registrations these are keyed by, and the process
    /// stays alive across that reset — dropping the profile watches here is
    /// what disarms their backends.
    pub(crate) fn reset_all_caches(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            *inner = ServiceInner::default();
        }
    }

    /// [`Self::reset_all_caches`] plus deleting the persisted observation
    /// file. A settings change rewrites what an observation would contain and
    /// announces itself with no filesystem event, so the stat-pinned records
    /// would otherwise keep re-serving; the service lock is held across the
    /// unlink so no concurrent observation can reload the pre-change file
    /// into the fresh mirror and later write those stale rows back out.
    pub(crate) fn reset_all_caches_and_purge_disk(&self, state_root: &Path) -> Result<(), String> {
        let mut guard = self.inner.lock();
        if let Ok(inner) = guard.as_mut() {
            **inner = ServiceInner::default();
        }
        let path = observation_cache_path(state_root);
        let result = match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "remove observation cache '{}': {error}",
                path.display()
            )),
        };
        drop(guard);
        result
    }

    /// The discovery and session views for one capture request, reusing the
    /// cached inventory when the provider-profile watcher proves it clean.
    /// Each provider profile owns an independent cache entry. Multi-provider
    /// requests compose those witnesses, so Activity, comparison, and detail
    /// all reuse the same profile-scoped discovery.
    pub(crate) fn discovery_for(
        &self,
        registry: &'static mallard_infrastructure::providers::ProviderRegistry,
        local_project_id: &LocalProjectId,
        request: &CaptureRequest,
    ) -> Result<(Discovery, Vec<LocalSessionView>), String> {
        // Project-content exclusions cannot affect a provider-only walk. Drop
        // them from its cache identity so the same profile witness is reused
        // whether the caller resolved one provider or every bound provider.
        let normalized_request = (!request.include_project_content).then(|| {
            let mut request = request.clone();
            request.excluded_content_roots.clear();
            request
        });
        let request = normalized_request.as_ref().unwrap_or(request);
        if request.provider_homes.len() > 1 && !request.include_project_content {
            let mut discoveries = Vec::with_capacity(request.provider_homes.len());
            let mut views = Vec::new();
            for (provider, home) in &request.provider_homes {
                let mut provider_request = request.clone();
                provider_request.provider_homes =
                    BTreeMap::from([(provider.clone(), home.clone())]);
                let (discovery, mut provider_views) =
                    self.discovery_for(registry, local_project_id, &provider_request)?;
                discoveries.push(discovery);
                views.append(&mut provider_views);
            }
            return Ok((Discovery::combine(discoveries)?, views));
        }
        let cache_profile = (request.provider_homes.len() == 1 && !request.include_project_content)
            .then(|| request.provider_homes.iter().next())
            .flatten()
            .and_then(|(provider, home)| {
                (!session_root_names(provider).is_empty())
                    .then(|| canonical_home_of(home).map(|home| (provider.clone(), home)))
                    .flatten()
            });
        let cacheable = cache_profile.is_some();
        let healthy = match &cache_profile {
            Some((provider, home)) => {
                let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.ensure_profile(provider, home)
            }
            None => false,
        };

        if cacheable && healthy {
            let (provider, home) = cache_profile
                .clone()
                .expect("cacheable requires a provider profile");
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let key = (provider.clone(), home.clone(), local_project_id.clone());
            let titles_dirty = inner
                .profiles
                .get(&home)
                .map(|profile| profile.titles_dirty)
                .unwrap_or(false);
            let inventory_clean = inner
                .profiles
                .get(&home)
                .map(|profile| profile.healthy && !profile.inventory_dirty)
                .unwrap_or(false);
            if inventory_clean {
                let dirty_roots = inner
                    .profiles
                    .get(&home)
                    .map(|profile| profile.dirty_roots.clone())
                    .unwrap_or_default();
                let mut refreshed_titles = false;
                let mut stat_raced = false;
                if let Some(cached) = inner.inventories.get_mut(&key) {
                    if cached.request == *request {
                        if titles_dirty && provider.as_str() == "codex" {
                            let mut discovery = cached.discovery.clone();
                            provider_capture::refresh_codex_session_titles(&mut discovery, &home)?;
                            cached.discovery = discovery;
                            cached.views = provider_capture::local_session_views(&cached.discovery);
                            refreshed_titles = true;
                        }
                        // A dirty root's presentation stamp refreshes with
                        // the observation that is about to re-read it; quiet
                        // rows stay exactly as cached, with no stat at all.
                        for view in cached.views.iter_mut() {
                            if !dirty_roots.contains(&view.source_path) {
                                continue;
                            }
                            match std::fs::symlink_metadata(&view.source_path) {
                                // A source that grew past the conversation
                                // bound must surface the same blocked row
                                // discovery produces: rediscover instead of
                                // serving a stale unblocked view.
                                Ok(meta)
                                    if meta.is_file()
                                        && meta.len() <= mallard_infrastructure::app_settings::conversation_file_limit_bytes() =>
                                {
                                    view.source_mtime = meta
                                        .modified()
                                        .ok()
                                        .and_then(|time| {
                                            time.duration_since(std::time::UNIX_EPOCH).ok()
                                        })
                                        .map(|duration| duration.as_secs())
                                        .unwrap_or(0);
                                    view.source_size = meta.len();
                                }
                                _ => stat_raced = true,
                            }
                        }
                        if !stat_raced {
                            sync_metrics::record_inventory_reuse_hit(&home);
                            let result = (cached.discovery.clone(), cached.views.clone());
                            if refreshed_titles {
                                if let Some(profile) = inner.profiles.get_mut(&home) {
                                    profile.titles_dirty = false;
                                }
                            }
                            return Ok(result);
                        }
                    }
                }
                if stat_raced {
                    // The dirty source vanished between its event and this
                    // operation: treat it as membership work and rediscover.
                    if let Some(profile) = inner.profiles.get_mut(&home) {
                        profile.inventory_dirty = true;
                    }
                }
            }
        }

        // Bounded discovery, then publish the fresh inventory under the
        // already-drained dirty state: events that arrived during the walk
        // are still queued and dirty the next operation.
        let discovery = provider_capture::discover_full(registry, request)?;
        let views = provider_capture::local_session_views(&discovery);
        if cacheable && healthy {
            let (provider, home) = cache_profile.expect("cacheable requires a provider profile");
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(profile) = inner.profiles.get_mut(&home) {
                if profile.healthy {
                    profile.inventory_dirty = false;
                    profile.titles_dirty = false;
                    profile.known_roots = views
                        .iter()
                        .filter(|view| view.provider == provider)
                        .map(|view| view.source_path.clone())
                        .collect();
                    inner.inventories.insert(
                        (provider, home, local_project_id.clone()),
                        CachedInventory {
                            request: request.clone(),
                            discovery: discovery.clone(),
                            views: views.clone(),
                        },
                    );
                }
            }
        }
        Ok((discovery, views))
    }

    /// Serve every queried observation the watcher proves clean. A served
    /// thread performs no filesystem work at all.
    pub(crate) fn take_reusable_observations(
        &self,
        provider: &ProviderKey,
        canonical_home: &Path,
        state_root: &Path,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        queries: &[ObservationQuery],
    ) -> BTreeMap<ThreadId, LocalTailObservation> {
        let mut served = BTreeMap::new();
        let Some(canonical_home) = canonical_home_of(canonical_home) else {
            return served;
        };
        let canonical_home = canonical_home.as_path();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.ensure_profile(provider, canonical_home) {
            return served;
        }
        let Some(profile) = inner.profiles.get(canonical_home) else {
            return served;
        };
        let dirty = profile.dirty_roots.clone();
        let mut misses = Vec::new();
        for query in queries {
            let key = ObservationKey {
                canonical_home: canonical_home.to_path_buf(),
                local_project_id: local_project_id.clone(),
                storage_id: storage_id.clone(),
                thread_id: query.thread_id.clone(),
            };
            let Some(cached) = inner.observations.get(&key) else {
                misses.push((key, query));
                continue;
            };
            if cached.checkpoint_generation != query.checkpoint_generation
                || cached.binding_revision != query.binding_revision
                || cached.depends_on.iter().any(|path| dirty.contains(path))
            {
                continue;
            }
            sync_metrics::record_observation_reuse_hit(canonical_home);
            served.insert(query.thread_id.clone(), cached.observation.clone());
        }
        // Memory misses fall through to the persisted cache. A disk record
        // proves itself by stat: every dependency pin must reproduce
        // exactly, so validity never leans on watcher history this process
        // does not have. A record that fails its pins is dropped — the stat
        // can only keep failing until a fresh derivation replaces it.
        for (key, query) in misses {
            let Some(record) = inner.disk_entries(state_root).get(&key).cloned() else {
                continue;
            };
            if record.checkpoint_generation != query.checkpoint_generation
                || record.binding_revision != query.binding_revision
            {
                continue;
            }
            if record
                .depends_on
                .iter()
                .any(|pin| dirty.contains(&pin.path))
                || !record.depends_on.iter().all(pin_matches)
            {
                inner.disk_entries(state_root).remove(&key);
                continue;
            }
            inner.observations.insert(
                key,
                CachedObservation {
                    depends_on: record
                        .depends_on
                        .iter()
                        .map(|pin| pin.path.clone())
                        .collect(),
                    checkpoint_generation: record.checkpoint_generation,
                    binding_revision: record.binding_revision,
                    observation: record.observation.clone(),
                },
            );
            sync_metrics::record_observation_reuse_hit(canonical_home);
            served.insert(query.thread_id.clone(), record.observation);
        }
        served
    }

    /// Explicit reliability reset: drop the profile's watcher and caches as a
    /// backend failure would, forcing the next operation to rebaseline.
    /// Reviewed application operations use this boundary so asynchronous
    /// event delivery can never authorize a stale Push or Pull decision.
    pub(crate) fn force_rebaseline(&self, canonical_home: &Path) {
        let Some(canonical_home) = canonical_home_of(canonical_home) else {
            return;
        };
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.drop_profile_caches(&canonical_home);
        inner.profiles.remove(&canonical_home);
    }

    /// Serve every candidate pin the watcher proves clean; served
    /// candidates skip review-time capture entirely.
    pub(crate) fn take_reusable_candidate_pins(
        &self,
        provider: &ProviderKey,
        canonical_home: &Path,
        local_project_id: &LocalProjectId,
        resource_ids: &BTreeSet<String>,
    ) -> BTreeMap<String, TargetExpectation> {
        let mut served = BTreeMap::new();
        let Some(canonical_home) = canonical_home_of(canonical_home) else {
            return served;
        };
        let canonical_home = canonical_home.as_path();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.ensure_profile(provider, canonical_home) {
            return served;
        }
        let Some(profile) = inner.profiles.get(canonical_home) else {
            return served;
        };
        let dirty = profile.dirty_roots.clone();
        for resource_id in resource_ids {
            let key = (
                canonical_home.to_path_buf(),
                local_project_id.clone(),
                resource_id.clone(),
            );
            let Some(cached) = inner.candidate_pins.get(&key) else {
                continue;
            };
            if cached.depends_on.iter().any(|path| dirty.contains(path)) {
                continue;
            }
            sync_metrics::record_observation_reuse_hit(canonical_home);
            served.insert(resource_id.clone(), cached.expected_local_target.clone());
        }
        served
    }

    /// Retain freshly captured candidate pins for the next review.
    pub(crate) fn store_candidate_pins(
        &self,
        canonical_home: &Path,
        local_project_id: &LocalProjectId,
        stored: Vec<StoredCandidatePin>,
    ) {
        let Some(canonical_home) = canonical_home_of(canonical_home) else {
            return;
        };
        let canonical_home = canonical_home.as_path();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(profile) = inner.profiles.get(canonical_home) else {
            return;
        };
        if !profile.healthy {
            return;
        }
        for entry in stored {
            let Some(depends_on) = canonical_paths(&entry.depends_on) else {
                continue;
            };
            inner.candidate_pins.insert(
                (
                    canonical_home.to_path_buf(),
                    local_project_id.clone(),
                    entry.resource_id,
                ),
                CachedCandidatePin {
                    depends_on,
                    expected_local_target: entry.expected_local_target,
                },
            );
        }
        if inner.candidate_pins.len() > MAX_OBSERVATION_ENTRIES {
            inner.evict_stale_profiles();
            if inner.candidate_pins.len() > MAX_OBSERVATION_ENTRIES {
                let home = canonical_home.to_path_buf();
                inner.drop_profile_caches(&home);
            }
        }
    }

    /// Retain freshly derived observations and clear their sources from the
    /// dirty set. Events that raced the recompute are still queued and
    /// re-dirty their paths on the next drain.
    pub(crate) fn store_observations(
        &self,
        canonical_home: &Path,
        state_root: &Path,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        stored: Vec<StoredObservation>,
    ) {
        let Some(canonical_home) = canonical_home_of(canonical_home) else {
            return;
        };
        let canonical_home = canonical_home.as_path();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(profile) = inner.profiles.get_mut(canonical_home) else {
            return;
        };
        if !profile.healthy {
            return;
        }
        let prepared: Vec<_> = stored
            .into_iter()
            .filter_map(|entry| {
                let source_path = std::fs::canonicalize(&entry.source_path).ok()?;
                let depends_on = canonical_paths(&entry.depends_on)?;
                Some((entry, source_path, depends_on))
            })
            .collect();
        // Clearing a dirty path declares its change consumed, so every
        // OTHER cached derivation of that path — a candidate pin, another
        // storage's observation — is stale now and must recompute rather
        // than ride the cleared flag.
        let cleared: BTreeSet<PathBuf> = prepared
            .iter()
            .filter(|(_, source_path, _)| profile.dirty_roots.contains(source_path))
            .map(|(_, source_path, _)| source_path.clone())
            .collect();
        for source_path in &cleared {
            profile.dirty_roots.remove(source_path);
        }
        if !cleared.is_empty() {
            inner
                .observations
                .retain(|_, cached| !cached.depends_on.iter().any(|path| cleared.contains(path)));
            inner
                .candidate_pins
                .retain(|_, cached| !cached.depends_on.iter().any(|path| cleared.contains(path)));
        }
        let stored_at = now_secs();
        let mut disk_records = Vec::new();
        for (entry, source_path, depends_on) in prepared {
            // Disk eligibility, decided before the entry moves into the
            // memory map: every dependency must pin, and the source pin
            // must still agree with the discovery-time stat — otherwise
            // the file changed while the review ran and the pin would
            // vouch for bytes the derivation never read.
            let pins: Option<Vec<DiskPathPin>> =
                depends_on.iter().map(|path| path_pin(path)).collect();
            if let Some(pins) = pins {
                let quiet_source = pins.iter().any(|pin| {
                    pin.path == source_path
                        && pin.size == entry.source_size
                        && pin.mtime_nanos / 1_000_000_000 == entry.source_mtime_secs
                });
                if quiet_source {
                    disk_records.push(DiskObservationRecord {
                        canonical_home: canonical_home.to_path_buf(),
                        local_project_id: local_project_id.clone(),
                        storage_id: storage_id.clone(),
                        thread_id: entry.thread_id.clone(),
                        checkpoint_generation: entry.checkpoint_generation,
                        binding_revision: entry.binding_revision,
                        depends_on: pins,
                        observation: entry.observation.clone(),
                        stored_at,
                    });
                }
            }
            inner.observations.insert(
                ObservationKey {
                    canonical_home: canonical_home.to_path_buf(),
                    local_project_id: local_project_id.clone(),
                    storage_id: storage_id.clone(),
                    thread_id: entry.thread_id,
                },
                CachedObservation {
                    depends_on,
                    checkpoint_generation: entry.checkpoint_generation,
                    binding_revision: entry.binding_revision,
                    observation: entry.observation,
                },
            );
        }
        if inner.observations.len() > MAX_OBSERVATION_ENTRIES {
            // Never evict part of the evidence: drop whole profiles, the
            // stalest first.
            inner.evict_stale_profiles();
            if inner.observations.len() > MAX_OBSERVATION_ENTRIES {
                let home = canonical_home.to_path_buf();
                inner.drop_profile_caches(&home);
            }
        }
        if !disk_records.is_empty() {
            let entries = inner.disk_entries(state_root);
            for record in disk_records {
                entries.insert(record.key(), record);
            }
            write_observation_disk_cache(state_root, entries);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::TailSideEvidence;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn service() -> SessionWatchService {
        SessionWatchService {
            inner: Mutex::new(ServiceInner::default()),
        }
    }

    fn observation() -> LocalTailObservation {
        LocalTailObservation::for_watch_service_tests(TailSideEvidence::ConfirmedAbsent)
    }

    fn codex_provider() -> ProviderKey {
        ProviderKey::parse("codex").unwrap()
    }

    fn ids() -> (LocalProjectId, StorageId, ThreadId) {
        (
            LocalProjectId::parse("watch-test-project").unwrap(),
            StorageId::parse("watch-test-storage").unwrap(),
            ThreadId::parse("watch-test-thread").unwrap(),
        )
    }

    /// A StoredObservation whose discovery-time stat is read from the file
    /// itself, as production reads it from the discovery view.
    fn stored(thread: &ThreadId, source: &Path, generation: u64) -> StoredObservation {
        let metadata = std::fs::symlink_metadata(source).unwrap();
        StoredObservation {
            thread_id: thread.clone(),
            source_path: source.to_path_buf(),
            depends_on: vec![source.to_path_buf()],
            checkpoint_generation: Some(generation),
            binding_revision: Some(1),
            observation: observation(),
            source_mtime_secs: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
            source_size: metadata.len(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn watcher_event_paths_match_canonical_dependency_spelling() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let real_home = temp.path().join("real-home");
        let sessions = real_home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let alias_home = temp.path().join("alias-home");
        symlink(&real_home, &alias_home).unwrap();
        let delivered = alias_home.join("sessions/rollout.jsonl");
        let real_file = sessions.join("rollout.jsonl");
        std::fs::write(&real_file, b"{}\n").unwrap();

        assert_eq!(
            canonical_event_path(&delivered),
            std::fs::canonicalize(&real_file).unwrap()
        );

        std::fs::remove_file(&real_file).unwrap();
        assert_eq!(
            canonical_event_path(&delivered),
            std::fs::canonicalize(&sessions)
                .unwrap()
                .join("rollout.jsonl")
        );
    }

    #[test]
    fn multi_provider_activity_populates_the_profile_caches_used_by_detail() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let codex = temp.path().join("codex");
        let claude = temp.path().join("claude");
        std::fs::create_dir_all(&project).unwrap();
        let cwd = serde_json::to_string(project.to_str().unwrap()).unwrap();
        write(
            &codex.join("sessions/2026/08/17/rollout-thread-a.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-a\",\"cwd\":{cwd}}}}}\n"
            )
            .as_bytes(),
        );
        write(
            &claude.join("projects/source/session-b.jsonl"),
            format!("{{\"type\":\"system\",\"sessionId\":\"session-b\",\"cwd\":{cwd}}}\n")
                .as_bytes(),
        );
        let codex_key = ProviderKey::parse("codex").unwrap();
        let claude_key = ProviderKey::parse("claude").unwrap();
        let project_id = LocalProjectId::parse("watch-multi-project").unwrap();
        let request = CaptureRequest {
            project_root: project.clone(),
            provider_homes: BTreeMap::from([
                (codex_key.clone(), codex.clone()),
                (claude_key, claude),
            ]),
            include_project_content: false,
            excluded_content_roots: vec![temp.path().join("unrelated-exclusion")],
        };
        let service = service();
        let (_, views) = service
            .discovery_for(
                mallard_infrastructure::providers::ProviderRegistry::production(),
                &project_id,
                &request,
            )
            .unwrap();
        assert_eq!(views.len(), 2);

        let metrics = sync_metrics::register_root(&std::fs::canonicalize(&codex).unwrap());
        let before = metrics.snapshot();
        let mut detail_request = request;
        detail_request
            .provider_homes
            .retain(|provider, _| provider == &codex_key);
        detail_request.excluded_content_roots = vec![project];
        let (_, detail_views) = service
            .discovery_for(
                mallard_infrastructure::providers::ProviderRegistry::production(),
                &project_id,
                &detail_request,
            )
            .unwrap();
        let delta = metrics.snapshot().delta_since(&before);
        assert_eq!(detail_views.len(), 1);
        assert_eq!(delta.inventory_reuse_hits, 1, "{delta:?}");
        assert_eq!(delta.files_head_read, 0);
    }

    #[test]
    fn a_mutated_source_is_never_served_and_a_quiet_one_is() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout-a.jsonl");
        std::fs::write(&rollout, b"{\"a\":1}\n").unwrap();

        let service = service();
        let (project, storage, thread) = ids();
        let query = || {
            vec![ObservationQuery {
                thread_id: thread.clone(),
                checkpoint_generation: Some(3),
                binding_revision: Some(1),
            }]
        };
        let state_root = temp.path().join("state");
        std::fs::create_dir_all(&state_root).unwrap();
        // Cold: nothing cached.
        assert!(service
            .take_reusable_observations(
                &codex_provider(),
                &home,
                &state_root,
                &project,
                &storage,
                &query()
            )
            .is_empty());
        service.store_observations(
            &home,
            &state_root,
            &project,
            &storage,
            vec![stored(&thread, &rollout, 3)],
        );
        // Quiet watcher: served with no filesystem work.
        assert_eq!(
            service
                .take_reusable_observations(
                    &codex_provider(),
                    &home,
                    &state_root,
                    &project,
                    &storage,
                    &query()
                )
                .len(),
            1
        );
        // Same-length in-place mutation: the content-compared test backend
        // must dirty the root and refuse the cached entry.
        std::fs::write(&rollout, b"{\"a\":2}\n").unwrap();
        assert!(
            service
                .take_reusable_observations(
                    &codex_provider(),
                    &home,
                    &state_root,
                    &project,
                    &storage,
                    &query()
                )
                .is_empty(),
            "a mutated source must never be served"
        );
        // A changed checkpoint generation is a miss even when quiet.
        service.store_observations(
            &home,
            &state_root,
            &project,
            &storage,
            vec![stored(&thread, &rollout, 3)],
        );
        let stale_generation = vec![ObservationQuery {
            thread_id: thread.clone(),
            checkpoint_generation: Some(4),
            binding_revision: Some(1),
        }];
        assert!(service
            .take_reusable_observations(
                &codex_provider(),
                &home,
                &state_root,
                &project,
                &storage,
                &stale_generation
            )
            .is_empty());
    }

    #[test]
    fn a_disk_record_survives_a_fresh_process_until_its_stat_drifts() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout-a.jsonl");
        std::fs::write(&rollout, b"{\"a\":1}\n").unwrap();
        let state_root = temp.path().join("state");
        std::fs::create_dir_all(&state_root).unwrap();
        let (project, storage, thread) = ids();
        let query = || {
            vec![ObservationQuery {
                thread_id: thread.clone(),
                checkpoint_generation: Some(3),
                binding_revision: Some(1),
            }]
        };

        let first_process = service();
        first_process.take_reusable_observations(
            &codex_provider(),
            &home,
            &state_root,
            &project,
            &storage,
            &[],
        );
        first_process.store_observations(
            &home,
            &state_root,
            &project,
            &storage,
            vec![stored(&thread, &rollout, 3)],
        );
        assert!(
            observation_cache_path(&state_root).is_file(),
            "store must persist the observation"
        );

        // A fresh service instance is a fresh process: no watcher history,
        // no memory cache. The stat pins alone must serve the entry.
        let second_process = service();
        assert_eq!(
            second_process
                .take_reusable_observations(
                    &codex_provider(),
                    &home,
                    &state_root,
                    &project,
                    &storage,
                    &query()
                )
                .len(),
            1,
            "a stat-clean disk record must serve across processes"
        );

        // Any byte appended between processes drifts size and mtime; the
        // record must be refused and dropped.
        let mut bytes = std::fs::read(&rollout).unwrap();
        bytes.extend_from_slice(b"{\"a\":2}\n");
        std::fs::write(&rollout, &bytes).unwrap();
        let third_process = service();
        assert!(
            third_process
                .take_reusable_observations(
                    &codex_provider(),
                    &home,
                    &state_root,
                    &project,
                    &storage,
                    &query()
                )
                .is_empty(),
            "a drifted source must never be served from disk"
        );
    }

    #[test]
    fn disk_records_pin_generation_and_an_unreadable_cache_starts_empty() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout-a.jsonl");
        std::fs::write(&rollout, b"{\"a\":1}\n").unwrap();
        let state_root = temp.path().join("state");
        std::fs::create_dir_all(&state_root).unwrap();
        let (project, storage, thread) = ids();

        let writer = service();
        writer.take_reusable_observations(
            &codex_provider(),
            &home,
            &state_root,
            &project,
            &storage,
            &[],
        );
        writer.store_observations(
            &home,
            &state_root,
            &project,
            &storage,
            vec![stored(&thread, &rollout, 3)],
        );

        // A moved checkpoint generation refuses the record even though the
        // stat pins hold.
        let stale_generation = vec![ObservationQuery {
            thread_id: thread.clone(),
            checkpoint_generation: Some(4),
            binding_revision: Some(1),
        }];
        assert!(service()
            .take_reusable_observations(
                &codex_provider(),
                &home,
                &state_root,
                &project,
                &storage,
                &stale_generation
            )
            .is_empty());

        // Corruption is a clean miss, never an error.
        std::fs::write(observation_cache_path(&state_root), b"not json").unwrap();
        assert!(service()
            .take_reusable_observations(
                &codex_provider(),
                &home,
                &state_root,
                &project,
                &storage,
                &[ObservationQuery {
                    thread_id: thread.clone(),
                    checkpoint_generation: Some(3),
                    binding_revision: Some(1),
                }]
            )
            .is_empty());
    }

    #[test]
    fn a_source_that_moved_during_the_review_is_not_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout-a.jsonl");
        std::fs::write(&rollout, b"{\"a\":1}\n").unwrap();
        let state_root = temp.path().join("state");
        std::fs::create_dir_all(&state_root).unwrap();
        let (project, storage, thread) = ids();

        let writer = service();
        writer.take_reusable_observations(
            &codex_provider(),
            &home,
            &state_root,
            &project,
            &storage,
            &[],
        );
        // The discovery-time stat is taken, then the file grows before the
        // store — exactly an active session appended to mid-review.
        let mut entry = stored(&thread, &rollout, 3);
        let mut bytes = std::fs::read(&rollout).unwrap();
        bytes.extend_from_slice(b"{\"a\":2}\n");
        std::fs::write(&rollout, &bytes).unwrap();
        entry.depends_on = vec![rollout.clone()];
        writer.store_observations(&home, &state_root, &project, &storage, vec![entry]);

        // A fresh process must find nothing on disk for it.
        assert!(service()
            .take_reusable_observations(
                &codex_provider(),
                &home,
                &state_root,
                &project,
                &storage,
                &[ObservationQuery {
                    thread_id: thread.clone(),
                    checkpoint_generation: Some(3),
                    binding_revision: Some(1),
                }]
            )
            .is_empty());
    }

    #[test]
    fn membership_changes_dirty_the_inventory_and_removal_forces_rebaseline() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let canonical_home = canonical_home_of(&home).unwrap();
        let service = service();
        let (project, storage, _) = ids();

        // Arm.
        let state_root = temp.path().join("state");
        std::fs::create_dir_all(&state_root).unwrap();
        service.take_reusable_observations(
            &codex_provider(),
            &home,
            &state_root,
            &project,
            &storage,
            &[],
        );
        {
            let mut inner = service.inner.lock().unwrap();
            assert!(inner.ensure_profile(&codex_provider(), &canonical_home));
            let profile = inner.profiles.get(&canonical_home).unwrap();
            assert!(!profile.inventory_dirty);
        }
        // A new rollout is membership work.
        std::fs::write(sessions.join("rollout-new.jsonl"), b"{}\n").unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            assert!(inner.ensure_profile(&codex_provider(), &canonical_home));
            let profile = inner.profiles.get(&canonical_home).unwrap();
            assert!(profile.inventory_dirty, "unknown rollout dirties inventory");
        }
        // Removing the watched sessions root is a reliability signal: the
        // profile rebaselines and stays usable through the bounded path.
        std::fs::remove_dir_all(&sessions).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            // First consult sees the removal and drops the profile...
            let _ = inner.ensure_profile(&codex_provider(), &canonical_home);
        }
        std::fs::create_dir_all(&sessions).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            assert!(
                inner.ensure_profile(&codex_provider(), &canonical_home),
                "rearm after recreation"
            );
        }
    }
}
