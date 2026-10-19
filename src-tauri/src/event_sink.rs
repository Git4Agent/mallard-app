//! The desktop's event sink: every activity-log entry an operation narrates
//! is appended to this machine's log and forwarded to the window as
//! `sync-log`; transfer progress goes out as `project-sync-fetch-progress`.
use std::sync::Arc;

use mallard_application::activity_log::record;
use mallard_application::events::{FetchProgressEvent, SyncEventSink};
use mallard_infrastructure::activity_log::ActivityLogEntry;
use tauri::Emitter as _;

pub struct DesktopSink<R: tauri::Runtime = tauri::Wry>(tauri::AppHandle<R>);

impl<R: tauri::Runtime> DesktopSink<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self(app)
    }
}

impl<R: tauri::Runtime> SyncEventSink for DesktopSink<R> {
    fn activity_log(&self, entry: &ActivityLogEntry) {
        record(entry, |entry| {
            let _ = self.0.emit("sync-log", entry);
        });
    }

    fn fetch_progress(&self, event: &FetchProgressEvent) {
        let _ = self.0.emit("project-sync-fetch-progress", event);
    }
}

/// The sink a command hands to an operation on a blocking thread.
pub fn desktop_sink(app: &tauri::AppHandle) -> Arc<dyn SyncEventSink> {
    Arc::new(DesktopSink(app.clone()))
}
