//! Process-local observation metrics.
//!
//! Counts the filesystem and parse work of session discovery, capture,
//! and Codex tail observation so tests can prove work-proportionality
//! claims ("a warm no-change refresh reads no transcript bytes") and so
//! a benchmark run can print where the bytes went.
//!
//! Test-support builds can attribute counters by path so parallel tests do not
//! observe each other's work. The process-global counters back the
//! `MALLARD_SYNC_METRICS=1` benchmark log in every build.

use std::cell::Cell;
use std::path::Path;
#[cfg(any(test, feature = "test-support"))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, RwLock};

/// Per-worker semantic analysis work. Unlike path counters this also covers
/// immutable cloud bytes, and stays isolated from other preparation workers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnalysisWork {
    pub passes: u64,
    pub bytes: u64,
}

thread_local! {
    static ANALYSIS_WORK: Cell<AnalysisWork> = const { Cell::new(AnalysisWork { passes: 0, bytes: 0 }) };
}

pub fn analysis_work() -> AnalysisWork {
    ANALYSIS_WORK.with(Cell::get)
}

pub fn record_transcript_analysis(bytes: u64) {
    ANALYSIS_WORK.with(|work| {
        let current = work.get();
        work.set(AnalysisWork {
            passes: current.passes + 1,
            bytes: current.bytes + bytes,
        });
    });
}

impl AnalysisWork {
    pub fn delta_since(self, earlier: Self) -> Self {
        Self {
            passes: self.passes - earlier.passes,
            bytes: self.bytes - earlier.bytes,
        }
    }
}

#[derive(Default)]
struct Counters {
    directories_visited: AtomicU64,
    files_head_read: AtomicU64,
    transcript_bytes_read: AtomicU64,
    probe_bytes_read: AtomicU64,
    append_bytes_read: AtomicU64,
    records_parsed: AtomicU64,
    bytes_hashed: AtomicU64,
    full_graph_resolves: AtomicU64,
    selected_captures: AtomicU64,
    inventory_reuse_hits: AtomicU64,
    observation_reuse_hits: AtomicU64,
}

/// One point-in-time reading of a counter set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub directories_visited: u64,
    pub files_head_read: u64,
    pub transcript_bytes_read: u64,
    pub probe_bytes_read: u64,
    pub append_bytes_read: u64,
    pub records_parsed: u64,
    pub bytes_hashed: u64,
    pub full_graph_resolves: u64,
    pub selected_captures: u64,
    pub inventory_reuse_hits: u64,
    pub observation_reuse_hits: u64,
}

impl MetricsSnapshot {
    /// The counter movement since an earlier snapshot of the same set.
    pub fn delta_since(&self, earlier: &MetricsSnapshot) -> MetricsSnapshot {
        MetricsSnapshot {
            directories_visited: self.directories_visited - earlier.directories_visited,
            files_head_read: self.files_head_read - earlier.files_head_read,
            transcript_bytes_read: self.transcript_bytes_read - earlier.transcript_bytes_read,
            probe_bytes_read: self.probe_bytes_read - earlier.probe_bytes_read,
            append_bytes_read: self.append_bytes_read - earlier.append_bytes_read,
            records_parsed: self.records_parsed - earlier.records_parsed,
            bytes_hashed: self.bytes_hashed - earlier.bytes_hashed,
            full_graph_resolves: self.full_graph_resolves - earlier.full_graph_resolves,
            selected_captures: self.selected_captures - earlier.selected_captures,
            inventory_reuse_hits: self.inventory_reuse_hits - earlier.inventory_reuse_hits,
            observation_reuse_hits: self.observation_reuse_hits - earlier.observation_reuse_hits,
        }
    }
}

impl Counters {
    fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            directories_visited: self.directories_visited.load(Ordering::Relaxed),
            files_head_read: self.files_head_read.load(Ordering::Relaxed),
            transcript_bytes_read: self.transcript_bytes_read.load(Ordering::Relaxed),
            probe_bytes_read: self.probe_bytes_read.load(Ordering::Relaxed),
            append_bytes_read: self.append_bytes_read.load(Ordering::Relaxed),
            records_parsed: self.records_parsed.load(Ordering::Relaxed),
            bytes_hashed: self.bytes_hashed.load(Ordering::Relaxed),
            full_graph_resolves: self.full_graph_resolves.load(Ordering::Relaxed),
            selected_captures: self.selected_captures.load(Ordering::Relaxed),
            inventory_reuse_hits: self.inventory_reuse_hits.load(Ordering::Relaxed),
            observation_reuse_hits: self.observation_reuse_hits.load(Ordering::Relaxed),
        }
    }
}

fn global() -> &'static Counters {
    static GLOBAL: OnceLock<Counters> = OnceLock::new();
    GLOBAL.get_or_init(Counters::default)
}

#[cfg(any(test, feature = "test-support"))]
type RegisteredRoots = RwLock<Vec<(PathBuf, Arc<Counters>)>>;

#[cfg(any(test, feature = "test-support"))]
fn roots() -> &'static RegisteredRoots {
    static ROOTS: OnceLock<RegisteredRoots> = OnceLock::new();
    ROOTS.get_or_init(|| RwLock::new(Vec::new()))
}

/// A registered attribution root. Work on paths under the root increments
/// this handle's counters until it is dropped.
#[cfg(any(test, feature = "test-support"))]
pub struct RegisteredRoot {
    root: PathBuf,
    counters: Arc<Counters>,
}

#[cfg(any(test, feature = "test-support"))]
impl RegisteredRoot {
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.counters.snapshot()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for RegisteredRoot {
    fn drop(&mut self) {
        let mut roots = roots().write().unwrap_or_else(|e| e.into_inner());
        if let Some(position) = roots.iter().position(|(root, counters)| {
            root == &self.root && Arc::ptr_eq(counters, &self.counters)
        }) {
            roots.remove(position);
        }
    }
}

/// Register one attribution root (a canonical provider home or project
/// root). Registering the same root twice yields independent counter sets
/// that both observe its work.
#[cfg(any(test, feature = "test-support"))]
pub fn register_root(root: &Path) -> RegisteredRoot {
    let counters = Arc::new(Counters::default());
    roots()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push((root.to_path_buf(), Arc::clone(&counters)));
    RegisteredRoot {
        root: root.to_path_buf(),
        counters,
    }
}

/// The process-global reading, for the benchmark log.
pub fn global_snapshot() -> MetricsSnapshot {
    global().snapshot()
}

fn record(path: &Path, apply: impl Fn(&Counters)) {
    apply(global());
    #[cfg(any(test, feature = "test-support"))]
    {
        let roots = roots().read().unwrap_or_else(|e| e.into_inner());
        for (root, counters) in roots.iter() {
            if path.starts_with(root) {
                apply(counters);
            }
        }
    }
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = path;
}

pub fn record_directory_visited(path: &Path) {
    record(path, |c| {
        c.directories_visited.fetch_add(1, Ordering::Relaxed);
    });
}

pub fn record_file_head_read(path: &Path) {
    record(path, |c| {
        c.files_head_read.fetch_add(1, Ordering::Relaxed);
    });
}

pub fn record_transcript_bytes_read(path: &Path, bytes: u64) {
    record(path, |c| {
        c.transcript_bytes_read.fetch_add(bytes, Ordering::Relaxed);
    });
}

pub fn record_probe_bytes_read(path: &Path, bytes: u64) {
    record(path, |c| {
        c.probe_bytes_read.fetch_add(bytes, Ordering::Relaxed);
    });
}

pub fn record_append_bytes_read(path: &Path, bytes: u64) {
    record(path, |c| {
        c.append_bytes_read.fetch_add(bytes, Ordering::Relaxed);
    });
}

pub fn record_records_parsed(path: &Path, records: u64) {
    record(path, |c| {
        c.records_parsed.fetch_add(records, Ordering::Relaxed);
    });
}

pub fn record_bytes_hashed(path: &Path, bytes: u64) {
    record(path, |c| {
        c.bytes_hashed.fetch_add(bytes, Ordering::Relaxed);
    });
}

pub fn record_full_graph_resolve(path: &Path) {
    record(path, |c| {
        c.full_graph_resolves.fetch_add(1, Ordering::Relaxed);
    });
}

pub fn record_selected_capture(path: &Path) {
    record(path, |c| {
        c.selected_captures.fetch_add(1, Ordering::Relaxed);
    });
}

pub fn record_inventory_reuse_hit(root: &Path) {
    record(root, |c| {
        c.inventory_reuse_hits.fetch_add(1, Ordering::Relaxed);
    });
}

pub fn record_observation_reuse_hit(root: &Path) {
    record(root, |c| {
        c.observation_reuse_hits.fetch_add(1, Ordering::Relaxed);
    });
}

/// Print the counter movement for one labeled operation when the
/// `MALLARD_SYNC_METRICS=1` benchmark environment is set.
pub fn log_operation_delta(label: &str, before: &MetricsSnapshot) {
    if std::env::var("MALLARD_SYNC_METRICS").as_deref() != Ok("1") {
        return;
    }
    let delta = global_snapshot().delta_since(before);
    eprintln!(
        "sync-metrics {label}: dirs={} head_reads={} transcript_bytes={} probe_bytes={} \
         append_bytes={} records={} hashed_bytes={} full_resolves={} captures={} \
         inventory_hits={} observation_hits={}",
        delta.directories_visited,
        delta.files_head_read,
        delta.transcript_bytes_read,
        delta.probe_bytes_read,
        delta.append_bytes_read,
        delta.records_parsed,
        delta.bytes_hashed,
        delta.full_graph_resolves,
        delta.selected_captures,
        delta.inventory_reuse_hits,
        delta.observation_reuse_hits,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_roots_attribute_by_path_prefix_and_stay_isolated() {
        let inside = Path::new("/metrics-test-root-a/sessions/x.jsonl");
        let outside = Path::new("/metrics-test-root-b/sessions/y.jsonl");
        let handle = register_root(Path::new("/metrics-test-root-a"));
        let other = register_root(Path::new("/metrics-test-root-b"));
        record_transcript_bytes_read(inside, 7);
        record_full_graph_resolve(outside);
        let a = handle.snapshot();
        let b = other.snapshot();
        assert_eq!(a.transcript_bytes_read, 7);
        assert_eq!(a.full_graph_resolves, 0);
        assert_eq!(b.transcript_bytes_read, 0);
        assert_eq!(b.full_graph_resolves, 1);
        let before = handle.snapshot();
        record_transcript_bytes_read(inside, 3);
        assert_eq!(
            handle.snapshot().delta_since(&before).transcript_bytes_read,
            3
        );
    }

    #[test]
    fn dropped_registration_stops_counting() {
        let path = Path::new("/metrics-test-root-c/sessions/z.jsonl");
        let handle = register_root(Path::new("/metrics-test-root-c"));
        record_records_parsed(path, 2);
        let counters = handle.snapshot();
        assert_eq!(counters.records_parsed, 2);
        drop(handle);
        // A new registration over the same root starts clean and is
        // unaffected by pre-registration work.
        let fresh = register_root(Path::new("/metrics-test-root-c"));
        assert_eq!(fresh.snapshot().records_parsed, 0);
    }
}
