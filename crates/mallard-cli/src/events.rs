//! Where the command line shows an operation's narration: every activity-log
//! entry is appended to this machine's log, as the desktop appends it, and
//! then written to stderr — one NDJSON line under `--json`, one plain line
//! otherwise. Stdout stays reserved for the command's one result.
use std::io::Write;

use mallard_application::activity_log::record;
use mallard_application::events::{FetchProgressEvent, SyncEventSink};
use mallard_infrastructure::activity_log::{ActivityLogEntry, ActivityLogLevel};
use serde::Serialize;

pub(crate) struct StderrSink {
    json: bool,
}

impl StderrSink {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    fn write_json(&self, value: &impl Serialize) {
        let mut stderr = std::io::stderr().lock();
        if serde_json::to_writer(&mut stderr, value).is_ok() {
            let _ = writeln!(stderr);
        }
    }
}

#[derive(Serialize)]
struct EventLine<'a> {
    schema: u32,
    status: &'static str,
    entry: &'a ActivityLogEntry,
}

#[derive(Serialize)]
struct LinkLine<'a> {
    schema: u32,
    status: &'static str,
    kind: &'static str,
    url: &'a str,
    expires_in: u32,
}

#[derive(Serialize)]
struct ProgressLine<'a> {
    schema: u32,
    status: &'static str,
    kind: &'static str,
    #[serde(flatten)]
    progress: &'a FetchProgressEvent,
}

/// The plain-text line for one entry: the message, which already carries the
/// product's glyphs, with a prefix only where the level matters.
pub(crate) fn text_line(entry: &ActivityLogEntry) -> String {
    match entry.level {
        ActivityLogLevel::Warning => format!("warning: {}", entry.message),
        ActivityLogLevel::Error => format!("error: {}", entry.message),
        _ => entry.message.clone(),
    }
}

impl SyncEventSink for StderrSink {
    fn activity_log(&self, entry: &ActivityLogEntry) {
        record(entry, |entry| {
            if self.json {
                self.write_json(&EventLine {
                    schema: 1,
                    status: "event",
                    entry,
                });
            } else if !entry.event.ends_with("timing") {
                // Phase timings are diagnostics for the log; a person reading
                // the terminal gets the narration without them.
                let mut stderr = std::io::stderr().lock();
                let _ = writeln!(stderr, "{}", text_line(entry));
            }
        });
    }

    fn authorization_link(&self, url: &str, expires_in: u32) {
        if self.json {
            self.write_json(&LinkLine {
                schema: 1,
                status: "progress",
                kind: "authorization_link",
                url,
                expires_in,
            });
        } else {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "Open this link in your browser to sign in (valid for {expires_in} seconds):\n{url}"
            );
        }
    }

    fn catalog_progress(
        &self,
        progress: &mallard_application::storage_catalog::service::CatalogSyncProgress,
    ) {
        if self.json {
            self.write_json(&serde_json::json!({
                "schema": 1,
                "status": "progress",
                "kind": "catalog",
                "progress": progress,
            }));
        } else {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "   {} ({}/{})",
                progress.message, progress.completed, progress.total
            );
        }
    }

    fn clone_progress(
        &self,
        progress: &mallard_application::project_sync::cloud_project::CloudProjectFolderProgress,
    ) {
        if self.json {
            self.write_json(&serde_json::json!({
                "schema": 1,
                "status": "progress",
                "kind": "clone",
                "progress": progress,
            }));
        } else {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "   {}",
                serde_json::to_string(progress).unwrap_or_default()
            );
        }
    }

    fn fetch_progress(&self, event: &FetchProgressEvent) {
        if self.json {
            self.write_json(&ProgressLine {
                schema: 1,
                status: "progress",
                kind: "fetch",
                progress: event,
            });
        } else {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "   {} {}/{} objects, {} of {} bytes",
                if event.phase == "push" { "↑" } else { "↓" },
                event.index,
                event.total_objects,
                event.fetched_bytes,
                event.total_bytes
            );
        }
    }
}
