//! Where an operation shows what it does. An operation persists nothing of
//! its narration itself: it hands every activity-log entry and every
//! transfer-progress update to the sink its adapter chose. The desktop's sink
//! appends the entry to this machine's activity log and forwards it to the
//! window; the command line's sink appends it and prints it; a test's sink
//! collects it and touches no home directory.
use mallard_infrastructure::activity_log::ActivityLogEntry;
use serde::Serialize;

pub trait SyncEventSink: Send + Sync {
    /// One activity-log entry an operation narrated.
    fn activity_log(&self, entry: &ActivityLogEntry);

    /// One coalesced step of a bundle transfer: at most one every 150 ms,
    /// and always the last.
    fn fetch_progress(&self, _event: &FetchProgressEvent) {}

    /// A browser sign-in is waiting: the person opens `url` within
    /// `expires_in` seconds. Never persisted, never opened by Mallard itself.
    fn authorization_link(&self, _url: &str, _expires_in: u32) {}

    /// One step of a Library Push or Pull: which item, out of how many.
    fn catalog_progress(&self, _progress: &crate::storage_catalog::service::CatalogSyncProgress) {}

    /// One step of getting a project's folder from Mallard Cloud: cloning
    /// (with a percentage when git reports one) or creating.
    fn clone_progress(
        &self,
        _progress: &crate::project_sync::cloud_project::CloudProjectFolderProgress,
    ) {
    }
}

/// One step of a Push upload or a Pull download, as the desktop's
/// `project-sync-fetch-progress` event carries it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FetchProgressEvent {
    pub phase: &'static str,
    pub storage_id: String,
    pub bundle_id: String,
    pub index: usize,
    pub total_objects: usize,
    pub fetched_bytes: u64,
    pub total_bytes: u64,
}

/// Shows nothing: for an operation run for its result alone.
pub struct SilentSink;

impl SyncEventSink for SilentSink {
    fn activity_log(&self, _entry: &ActivityLogEntry) {}
}
