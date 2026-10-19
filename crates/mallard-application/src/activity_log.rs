//! Activity-log narration shared by every adapter: the scope an operation
//! logs under, and the append each adapter's sink performs before it shows
//! an entry. The store itself is `mallard_infrastructure::activity_log`.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mallard_domain::generated_named_id;
use mallard_infrastructure::activity_log::{
    day_for_timestamp, normalized_level_name, ActivityLogContext, ActivityLogEntry,
    ActivityLogStore, ActivityLogType,
};
use mallard_infrastructure::persistence::LocalStateRepository;

use crate::events::SyncEventSink;

/// What one operation's entries share: their type, one run id, and the
/// project and storage they concern.
#[derive(Clone, Debug)]
pub struct ActivityLogScope {
    log_type: ActivityLogType,
    run_id: Option<String>,
    context: ActivityLogContext,
}

impl ActivityLogScope {
    pub fn new(log_type: ActivityLogType) -> Self {
        Self {
            log_type,
            run_id: generated_named_id(log_type.as_str()).ok(),
            context: ActivityLogContext::default(),
        }
    }

    pub fn project(mut self, project_id: impl ToString, project_name: Option<&str>) -> Self {
        self.context.project_id = Some(project_id.to_string());
        self.context.project_name = project_name.map(ToOwned::to_owned);
        self
    }

    pub fn storage(mut self, storage_id: impl ToString, storage_name: Option<&str>) -> Self {
        self.context.storage_id = Some(storage_id.to_string());
        self.context.storage_name = storage_name.map(ToOwned::to_owned);
        self
    }

    pub fn emit(&self, sink: &dyn SyncEventSink, level: &str, event: &str, message: &str) {
        sink.activity_log(&ActivityLogEntry::new(
            self.log_type,
            level,
            event,
            message,
            self.run_id.clone(),
            self.context.clone(),
        ));
    }
}

/// One entry outside any operation, named `<type>.<level>`.
pub fn emit_typed_log(
    sink: &dyn SyncEventSink,
    log_type: ActivityLogType,
    level: &str,
    message: &str,
) {
    let event = format!("{}.{}", log_type.as_str(), normalized_level_name(level));
    sink.activity_log(&ActivityLogEntry::new(
        log_type,
        level,
        &event,
        message,
        None,
        ActivityLogContext::default(),
    ));
}

/// Append one entry to this machine's activity log and enforce retention.
pub fn persist_entry(entry: &ActivityLogEntry) -> Result<(), String> {
    let repository = LocalStateRepository::from_user_home()?;
    let store = ActivityLogStore::from_repository(&repository)?;
    store.append(entry)?;
    let day = day_for_timestamp(entry.ts)?;
    store.maybe_cleanup(&day);
    Ok(())
}

static PERSISTENCE_WARNING_EMITTED: AtomicBool = AtomicBool::new(false);

/// Append `entry`, then show it through `show`. The first append a process
/// fails is followed by one warning entry; entries stay visible where the
/// adapter shows them until the process ends.
pub fn record(entry: &ActivityLogEntry, show: impl Fn(&ActivityLogEntry)) {
    let persisted = persist_entry(entry);
    show(entry);
    if persisted.is_err() && !PERSISTENCE_WARNING_EMITTED.swap(true, Ordering::SeqCst) {
        show(&ActivityLogEntry::new(
            ActivityLogType::System,
            "warning",
            "system.log_persistence_unavailable",
            "Activity log persistence is unavailable; current entries remain visible until Mallard closes.",
            None,
            ActivityLogContext::default(),
        ));
    }
}

/// Keep Cloud upload evidence in the activity log, separate from the short
/// command error used by the notice. Subscribe before operations can run.
pub fn start_cloud_diagnostic_forwarder(sink: Arc<dyn SyncEventSink>) {
    let mut events = mallard_infrastructure::managed_cloud::diagnostics::upload_diagnostics();
    std::thread::Builder::new()
        .name("mallard-cloud-diagnostics".to_string())
        .spawn(move || loop {
            let message = match events.blocking_recv() {
                Ok(message) => message,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    format!("Cloud upload diagnostics omitted {count} entries because the log queue was full.")
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            sink.activity_log(&ActivityLogEntry::new(
                ActivityLogType::Storage,
                "warning",
                "storage.cloud_upload_diagnostic",
                &message,
                None,
                ActivityLogContext::default(),
            ));
        })
        .ok();
}
