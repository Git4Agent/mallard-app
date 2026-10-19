//! Reuse of cursor-provider tail observations between operations and
//! processes.
//!
//! An observation is reused only while every file it was read from still
//! carries the stamp it was read under, checked at the moment of reuse. No
//! filesystem event takes part: a writer that keeps its file open reports
//! nothing until it closes, while a stat shows every write at once. The
//! same check decides what is retained, so a result whose files moved on
//! while the review ran is never stored.
//!
//! The cache is a reuse artifact, never a correctness authority: nothing
//! here promotes accepted state, authorizes an action, or replaces an
//! authoritative plan/apply digest, and apply keeps every existing
//! verification.

use super::service::LocalTailObservation;
use mallard_domain::{now_secs, LocalProjectId, StorageId, ThreadId};
use mallard_infrastructure::codex_subagents::RolloutLookup;
use mallard_infrastructure::jsonl_tail_io::{SourceStamp, SourceStamps};
use mallard_infrastructure::owned_subagents::TreeObservation;
use mallard_infrastructure::persistence::write_json_atomic;
use mallard_infrastructure::sync_metrics;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Past the bound the oldest records go first; each record stands on its
/// own stamps, so evicting one never weakens another.
const MAX_OBSERVATION_ENTRIES: usize = 4_096;
/// On-disk observation cache: bounded like every other `~/.mallard` file.
/// A file of another schema, or one whose records have another shape, is
/// discarded, never misread.
const OBSERVATION_DISK_SCHEMA: u32 = 1;
const MAX_OBSERVATION_DISK_BYTES: u64 = 16 * 1024 * 1024;
const OBSERVATION_DISK_FILE: &str = "observation_cache.json";

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ObservationKey {
    canonical_home: PathBuf,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    thread_id: ThreadId,
}

/// One retained observation, identical in memory and on disk except for
/// the Push evidence an observation never serializes.
#[derive(Serialize, Deserialize, Clone)]
struct ObservationRecord {
    canonical_home: PathBuf,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    thread_id: ThreadId,
    checkpoint_generation: Option<u64>,
    binding_revision: Option<u64>,
    depends_on: SourceStamps,
    observation: LocalTailObservation,
    stored_at: u64,
}

impl ObservationRecord {
    fn key(&self) -> ObservationKey {
        ObservationKey {
            canonical_home: self.canonical_home.clone(),
            local_project_id: self.local_project_id.clone(),
            storage_id: self.storage_id.clone(),
            thread_id: self.thread_id.clone(),
        }
    }

    fn answers(&self, query: &ObservationQuery) -> bool {
        self.checkpoint_generation == query.checkpoint_generation
            && self.binding_revision == query.binding_revision
    }
}

#[derive(Serialize, Deserialize)]
struct ObservationDiskCache {
    schema: u32,
    entries: Vec<ObservationRecord>,
}

/// One reusable-observation lookup.
pub(crate) struct ObservationQuery {
    pub(crate) thread_id: ThreadId,
    pub(crate) checkpoint_generation: Option<u64>,
    pub(crate) binding_revision: Option<u64>,
}

/// Push supplies the exact selected capture.
pub(crate) struct PushObservationQuery<'a> {
    pub(crate) observation: ObservationQuery,
    pub(crate) source: &'a Path,
    pub(crate) stamp: &'a SourceStamp,
    pub(crate) provider_thread_id: &'a str,
    pub(crate) layout: Option<&'a mallard_domain::RootTailLayout>,
}

/// One derived observation to retain for the next operation.
pub(crate) struct StoredObservation {
    pub(crate) thread_id: ThreadId,
    /// Every file the derivation read, under its read-time stamp.
    pub(crate) depends_on: SourceStamps,
    pub(crate) checkpoint_generation: Option<u64>,
    pub(crate) binding_revision: Option<u64>,
    pub(crate) observation: LocalTailObservation,
}

type Records = BTreeMap<ObservationKey, ObservationRecord>;

fn observation_cache_path(state_root: &Path) -> PathBuf {
    state_root.join(OBSERVATION_DISK_FILE)
}

fn load_records(state_root: &Path) -> Records {
    let path = observation_cache_path(state_root);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return Records::new();
    };
    if !metadata.is_file() || metadata.len() > MAX_OBSERVATION_DISK_BYTES {
        return Records::new();
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return Records::new();
    };
    let Ok(cache) = serde_json::from_slice::<ObservationDiskCache>(&bytes) else {
        return Records::new();
    };
    if cache.schema != OBSERVATION_DISK_SCHEMA || cache.entries.len() > MAX_OBSERVATION_ENTRIES {
        return Records::new();
    }
    cache
        .entries
        .into_iter()
        .map(|record| (record.key(), record))
        .collect()
}

/// Evict the oldest records past the entry bound, then rewrite the file. A
/// failed write only costs the next cold start its reuse.
fn write_records(state_root: &Path, records: &mut Records) {
    while records.len() > MAX_OBSERVATION_ENTRIES {
        let Some(oldest) = records
            .iter()
            .min_by_key(|(_, record)| record.stored_at)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        records.remove(&oldest);
    }
    let cache = ObservationDiskCache {
        schema: OBSERVATION_DISK_SCHEMA,
        entries: records.values().cloned().collect(),
    };
    let _ = write_json_atomic(
        state_root,
        &observation_cache_path(state_root),
        &cache,
        MAX_OBSERVATION_DISK_BYTES,
    );
}

/// The exact canonical home every record is keyed by, so two spellings of
/// one provider home share their observations.
fn canonical_home_of(home: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(home).ok()
}

pub struct ObservationCache {
    /// Per state root: loaded from disk on first use, written through on
    /// every store.
    records: Mutex<BTreeMap<PathBuf, Records>>,
}

pub fn observation_cache() -> &'static ObservationCache {
    static CACHE: OnceLock<ObservationCache> = OnceLock::new();
    CACHE.get_or_init(|| ObservationCache {
        records: Mutex::new(BTreeMap::new()),
    })
}

impl ObservationCache {
    fn with_records<T>(&self, state_root: &Path, read: impl FnOnce(&mut Records) -> T) -> T {
        let mut roots = self
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        read(
            roots
                .entry(state_root.to_path_buf())
                .or_insert_with(|| load_records(state_root)),
        )
    }

    /// The records answering `queries`, cloned out so no filesystem check
    /// runs under the lock.
    fn candidates<'a>(
        &self,
        home: &Path,
        state_root: &Path,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        queries: impl Iterator<Item = &'a ObservationQuery>,
    ) -> Vec<ObservationRecord> {
        self.with_records(state_root, |records| {
            queries
                .filter_map(|query| {
                    records
                        .get(&ObservationKey {
                            canonical_home: home.to_path_buf(),
                            local_project_id: local_project_id.clone(),
                            storage_id: storage_id.clone(),
                            thread_id: query.thread_id.clone(),
                        })
                        .filter(|record| record.answers(query))
                        .cloned()
                })
                .collect()
        })
    }

    /// Forget every record in memory. The whole-`~/.mallard` reset deletes
    /// the file and the registrations the records are keyed by, and the
    /// process stays alive across it.
    pub fn reset(&self) {
        if let Ok(mut roots) = self.records.lock() {
            roots.clear();
        }
    }

    /// Forget one state root's records and delete its persisted file. A
    /// settings change rewrites what an observation would contain and
    /// touches no session file, so every stamp would still hold; the lock
    /// is held across the unlink so no concurrent observation can reload
    /// the pre-change file and later write those rows back out.
    pub fn reset_and_purge_disk(&self, state_root: &Path) -> Result<(), String> {
        let mut guard = self.records.lock();
        if let Ok(roots) = guard.as_mut() {
            roots.insert(state_root.to_path_buf(), Records::new());
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

    /// Untrusted starting cursors. The child observer revalidates every
    /// source, so a record is a cursor whether or not its stamps hold.
    pub(crate) fn subagent_cursors(
        &self,
        home: &Path,
        state_root: &Path,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        threads: &[ThreadId],
    ) -> BTreeMap<ThreadId, TreeObservation> {
        let Some(home) = canonical_home_of(home) else {
            return BTreeMap::new();
        };
        self.with_records(state_root, |records| {
            threads
                .iter()
                .filter_map(|thread_id| {
                    let record = records.get(&ObservationKey {
                        canonical_home: home.clone(),
                        local_project_id: local_project_id.clone(),
                        storage_id: storage_id.clone(),
                        thread_id: thread_id.clone(),
                    })?;
                    Some((thread_id.clone(), record.observation.subagents.clone()?))
                })
                .collect()
        })
    }

    /// Observations for a comparison: every queried thread whose record
    /// still describes the selected rollouts and whose files all carry
    /// their recorded stamps.
    pub(crate) fn take_reusable_observations(
        &self,
        home: &Path,
        state_root: &Path,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        queries: &[ObservationQuery],
    ) -> BTreeMap<ThreadId, LocalTailObservation> {
        let Some(home) = canonical_home_of(home) else {
            return BTreeMap::new();
        };
        let lookup = RolloutLookup::new(&home);
        self.candidates(
            &home,
            state_root,
            local_project_id,
            storage_id,
            queries.iter(),
        )
        .into_iter()
        .filter(|record| {
            record.observation.matches_codex_selection(&lookup) && record.depends_on.hold()
        })
        .map(|record| {
            sync_metrics::record_observation_reuse_hit(&home);
            (record.thread_id, record.observation)
        })
        .collect()
    }

    /// Observations for a Push plan: only one derived in this process
    /// carries the read-time evidence Push needs, and that evidence must
    /// match the selected capture.
    pub(crate) fn take_push_observations(
        &self,
        home: &Path,
        state_root: &Path,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        queries: &[PushObservationQuery<'_>],
    ) -> BTreeMap<ThreadId, LocalTailObservation> {
        let Some(home) = canonical_home_of(home) else {
            return BTreeMap::new();
        };
        let candidates: BTreeMap<ThreadId, ObservationRecord> = self
            .candidates(
                &home,
                state_root,
                local_project_id,
                storage_id,
                queries.iter().map(|query| &query.observation),
            )
            .into_iter()
            .map(|record| (record.thread_id.clone(), record))
            .collect();
        let lookup = RolloutLookup::new(&home);
        queries
            .iter()
            .filter_map(|query| {
                let record = candidates.get(&query.observation.thread_id)?;
                (record.observation.matches_codex_selection(&lookup)
                    && record.observation.matches_push_capture(
                        query.source,
                        query.stamp,
                        query.provider_thread_id,
                        query.layout,
                    ))
                .then(|| {
                    sync_metrics::record_observation_reuse_hit(&home);
                    (record.thread_id.clone(), record.observation.clone())
                })
            })
            .collect()
    }

    /// Retain freshly derived observations. One whose files no longer
    /// carry their read-time stamps describes a state that is already gone
    /// and is dropped; the record it would have replaced stays, and fails
    /// the same check when it is next consulted.
    pub(crate) fn store_observations(
        &self,
        home: &Path,
        state_root: &Path,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        stored: Vec<StoredObservation>,
    ) {
        let Some(home) = canonical_home_of(home) else {
            return;
        };
        let stored_at = now_secs();
        let fresh: Vec<ObservationRecord> = stored
            .into_iter()
            .filter(|entry| entry.depends_on.hold())
            .map(|entry| ObservationRecord {
                canonical_home: home.clone(),
                local_project_id: local_project_id.clone(),
                storage_id: storage_id.clone(),
                thread_id: entry.thread_id,
                checkpoint_generation: entry.checkpoint_generation,
                binding_revision: entry.binding_revision,
                depends_on: entry.depends_on,
                observation: entry.observation,
                stored_at,
            })
            .collect();
        if fresh.is_empty() {
            return;
        }
        self.with_records(state_root, |records| {
            for record in fresh {
                records.insert(record.key(), record);
            }
            write_records(state_root, records);
        });
    }
}

#[cfg(test)]
mod tests {
    include!("observation_cache_regression_tests.rs");
    use super::*;
    use mallard_domain::TailSideEvidence;
    use std::io::Write;

    fn cache() -> ObservationCache {
        ObservationCache {
            records: Mutex::new(BTreeMap::new()),
        }
    }

    fn ids() -> (LocalProjectId, StorageId, ThreadId) {
        (
            LocalProjectId::parse("cache-test-project").unwrap(),
            StorageId::parse("cache-test-storage").unwrap(),
            ThreadId::parse("cache-test-thread").unwrap(),
        )
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        home: PathBuf,
        state_root: PathBuf,
        rollout: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout-a.jsonl");
        std::fs::write(&rollout, b"{\"a\":1}\n").unwrap();
        let state_root = temp.path().join("state");
        std::fs::create_dir_all(&state_root).unwrap();
        Fixture {
            _temp: temp,
            home,
            state_root,
            rollout,
        }
    }

    /// A derivation of `sources` as they are right now.
    fn stored(thread: &ThreadId, sources: &[&Path], generation: u64) -> StoredObservation {
        let mut depends_on = SourceStamps::default();
        for source in sources {
            depends_on.record(source, &SourceStamp::read(source).unwrap());
        }
        StoredObservation {
            thread_id: thread.clone(),
            depends_on,
            checkpoint_generation: Some(generation),
            binding_revision: Some(1),
            observation: LocalTailObservation::for_observation_cache_tests(
                TailSideEvidence::ConfirmedAbsent,
            ),
        }
    }

    fn query(thread: &ThreadId, generation: u64) -> Vec<ObservationQuery> {
        vec![ObservationQuery {
            thread_id: thread.clone(),
            checkpoint_generation: Some(generation),
            binding_revision: Some(1),
        }]
    }

    fn served(cache: &ObservationCache, fixture: &Fixture, generation: u64) -> usize {
        let (project, storage, thread) = ids();
        cache
            .take_reusable_observations(
                &fixture.home,
                &fixture.state_root,
                &project,
                &storage,
                &query(&thread, generation),
            )
            .len()
    }

    fn store(cache: &ObservationCache, fixture: &Fixture, entry: StoredObservation) {
        let (project, storage, _) = ids();
        cache.store_observations(
            &fixture.home,
            &fixture.state_root,
            &project,
            &storage,
            vec![entry],
        );
    }

    #[test]
    fn an_unchanged_source_is_served_and_a_moved_pin_is_not() {
        let fixture = fixture();
        let cache = cache();
        let (_, _, thread) = ids();
        assert_eq!(served(&cache, &fixture, 3), 0, "cold: nothing retained");

        store(&cache, &fixture, stored(&thread, &[&fixture.rollout], 3));
        assert_eq!(served(&cache, &fixture, 3), 1);
        assert_eq!(
            served(&cache, &fixture, 4),
            0,
            "a moved checkpoint generation is a miss while the stamps hold"
        );
    }

    /// The reported failure: the writer keeps its handle open, so no
    /// filesystem event is delivered for the append. The stamp moves at
    /// once and the stale observation is refused while the handle is open.
    #[test]
    fn an_append_through_a_held_open_handle_is_never_served() {
        let fixture = fixture();
        let cache = cache();
        let (_, _, thread) = ids();
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.rollout)
            .unwrap();
        store(&cache, &fixture, stored(&thread, &[&fixture.rollout], 3));
        assert_eq!(served(&cache, &fixture, 3), 1);

        writer.write_all(b"{\"a\":2}\n").unwrap();
        writer.flush().unwrap();
        assert_eq!(
            served(&cache, &fixture, 3),
            0,
            "the writer still holds the file open"
        );

        store(&cache, &fixture, stored(&thread, &[&fixture.rollout], 3));
        assert_eq!(served(&cache, &fixture, 3), 1);
        drop(writer);
        assert_eq!(served(&cache, &fixture, 3), 1, "closing writes nothing");
    }

    #[test]
    fn truncation_rewrite_and_replacement_are_never_served() {
        let (_, _, thread) = ids();
        // Only the change time tells a same-size rewrite with a restored
        // mtime apart, and only Unix stamps carry it.
        let mutations: &[&str] = if cfg!(unix) {
            &["truncate", "replace", "rewrite"]
        } else {
            &["truncate", "replace"]
        };
        for mutation in mutations {
            let fixture = fixture();
            let cache = cache();
            store(&cache, &fixture, stored(&thread, &[&fixture.rollout], 3));
            assert_eq!(served(&cache, &fixture, 3), 1, "{mutation}");
            let path = &fixture.rollout;
            let open = || std::fs::OpenOptions::new().write(true).open(path).unwrap();
            match *mutation {
                "truncate" => open().set_len(4).unwrap(),
                "replace" => {
                    let replacement = path.with_extension("replacement");
                    std::fs::write(&replacement, b"{\"a\":1}\n").unwrap();
                    std::fs::rename(replacement, path).unwrap();
                }
                _ => {
                    let modified = std::fs::metadata(path).unwrap().modified().unwrap();
                    std::fs::write(path, b"{\"a\":9}\n").unwrap();
                    open().set_modified(modified).unwrap();
                }
            }
            assert_eq!(served(&cache, &fixture, 3), 0, "{mutation}");
        }
    }

    #[test]
    fn a_changed_dependency_refuses_a_parent_whose_own_file_is_unchanged() {
        let fixture = fixture();
        let cache = cache();
        let (_, _, thread) = ids();
        let child = fixture.home.join("sessions/rollout-child.jsonl");
        std::fs::write(&child, b"{\"child\":1}\n").unwrap();
        store(
            &cache,
            &fixture,
            stored(&thread, &[&fixture.rollout, &child], 3),
        );
        assert_eq!(served(&cache, &fixture, 3), 1);

        std::fs::OpenOptions::new()
            .append(true)
            .open(&child)
            .unwrap()
            .write_all(b"{\"child\":2}\n")
            .unwrap();
        assert_eq!(served(&cache, &fixture, 3), 0);

        std::fs::remove_file(&child).unwrap();
        assert_eq!(served(&cache, &fixture, 3), 0, "a vanished dependency");
    }

    #[test]
    fn a_result_older_than_the_file_is_not_retained() {
        let fixture = fixture();
        let cache = cache();
        let (_, _, thread) = ids();
        // Derived, then the file grows before the review stores its result.
        let late = stored(&thread, &[&fixture.rollout], 3);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.rollout)
            .unwrap()
            .write_all(b"{\"a\":2}\n")
            .unwrap();
        let current = stored(&thread, &[&fixture.rollout], 3);
        store(&cache, &fixture, current);
        assert_eq!(served(&cache, &fixture, 3), 1);

        store(&cache, &fixture, late);
        assert_eq!(
            served(&cache, &fixture, 3),
            1,
            "the newer record survives the late store"
        );
        assert_eq!(
            served(&self::cache(), &fixture, 3),
            1,
            "and is what reached the disk"
        );
    }

    #[test]
    fn an_unpinned_derivation_is_not_retained() {
        let fixture = fixture();
        let cache = cache();
        let (_, _, thread) = ids();
        let mut unstamped = stored(&thread, &[&fixture.rollout], 3);
        unstamped.depends_on.record_unstamped();
        store(&cache, &fixture, unstamped);
        assert_eq!(served(&cache, &fixture, 3), 0, "a file without a stamp");

        let mut twice = stored(&thread, &[&fixture.rollout], 3);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.rollout)
            .unwrap()
            .write_all(b"{\"a\":2}\n")
            .unwrap();
        twice.depends_on.record(
            &fixture.rollout,
            &SourceStamp::read(&fixture.rollout).unwrap(),
        );
        store(&cache, &fixture, twice);
        assert_eq!(served(&cache, &fixture, 3), 0, "one path under two stamps");
        assert!(!observation_cache_path(&fixture.state_root).exists());
    }

    #[test]
    fn a_record_serves_a_fresh_process_until_its_stamp_moves() {
        let fixture = fixture();
        let (_, _, thread) = ids();
        store(&cache(), &fixture, stored(&thread, &[&fixture.rollout], 3));
        assert!(observation_cache_path(&fixture.state_root).is_file());

        // A fresh cache is a fresh process: nothing in memory.
        assert_eq!(served(&cache(), &fixture, 3), 1);
        assert_eq!(served(&cache(), &fixture, 4), 0);

        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.rollout)
            .unwrap()
            .write_all(b"{\"a\":2}\n")
            .unwrap();
        assert_eq!(served(&cache(), &fixture, 3), 0);
    }

    #[test]
    fn an_unreadable_or_older_schema_file_starts_empty() {
        let fixture = fixture();
        let (_, _, thread) = ids();
        store(&cache(), &fixture, stored(&thread, &[&fixture.rollout], 3));
        let path = observation_cache_path(&fixture.state_root);

        let mut document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        document["schema"] = serde_json::json!(OBSERVATION_DISK_SCHEMA - 1);
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(served(&cache(), &fixture, 3), 0, "an older schema");

        // The schema number does not change with the shape of a record. A
        // record of another shape does not parse.
        document["schema"] = serde_json::json!(OBSERVATION_DISK_SCHEMA);
        document["entries"][0]["depends_on"] = serde_json::json!([]);
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(served(&cache(), &fixture, 3), 0, "another shape");

        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(served(&cache(), &fixture, 3), 0, "corruption is a miss");
    }

    #[test]
    fn a_purge_forgets_memory_and_disk() {
        let fixture = fixture();
        let cache = cache();
        let (_, _, thread) = ids();
        store(&cache, &fixture, stored(&thread, &[&fixture.rollout], 3));
        assert_eq!(served(&cache, &fixture, 3), 1);

        cache.reset_and_purge_disk(&fixture.state_root).unwrap();
        assert_eq!(served(&cache, &fixture, 3), 0);
        assert!(!observation_cache_path(&fixture.state_root).exists());
        cache.reset_and_purge_disk(&fixture.state_root).unwrap();
    }

    #[test]
    fn two_spellings_of_one_home_share_a_record() {
        let fixture = fixture();
        let cache = cache();
        let (project, storage, thread) = ids();
        store(&cache, &fixture, stored(&thread, &[&fixture.rollout], 3));
        let respelled = fixture.home.join("sessions").join("..");
        assert_eq!(
            cache
                .take_reusable_observations(
                    &respelled,
                    &fixture.state_root,
                    &project,
                    &storage,
                    &query(&thread, 3),
                )
                .len(),
            1
        );
    }
}
