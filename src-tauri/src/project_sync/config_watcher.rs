//! Watch machine-local project-sync state and announce externally visible changes.
//!
//! Persistence replaces `sync_config.json` atomically, so the parent directory
//! is watched instead of a file inode. Storage setup drafts live one directory
//! below that root and may be created after application startup, so the root is
//! watched recursively. Event payloads deliberately carry no configuration or
//! draft values; storage credentials remain behind the command boundary.

use mallard_infrastructure::persistence::LocalStateRepository;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use tauri::Emitter as _;

pub const CONFIG_CHANGED_EVENT: &str = "project-sync-config-changed";
pub const STORAGE_SETUP_DRAFTS_CHANGED_EVENT: &str = "project-sync-storage-setup-drafts-changed";
/// A storage's runtime connection health changed or was forgotten. The
/// payload names the storage only; the listing carries the health itself.
pub const STORAGE_CONNECTION_CHANGED_EVENT: &str = "project-sync-storage-connection-changed";

#[derive(Serialize, Clone)]
pub struct StorageConnectionChangedEvent {
    pub storage_id: String,
}

/// A Push this computer had confirmed was outranked by another computer's
/// and rolled back on one link. The payload names the link only.
pub const PUSH_OUTRANKED_EVENT: &str = "project-sync-push-outranked";

#[derive(Serialize, Clone)]
pub struct PushOutrankedEvent {
    pub local_project_id: String,
    pub storage_id: String,
}

/// Forward the infrastructure's connection-health transitions to the window
/// for the life of the process. A lagged receiver just keeps forwarding; the
/// frontend re-reads the whole listing on every event.
pub fn start_connection_forwarder<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    let mut events = mallard_infrastructure::storage_core::connection_events().subscribe();
    thread::Builder::new()
        .name("mallard-connection-events".to_string())
        .spawn(move || loop {
            match events.blocking_recv() {
                Ok(storage_id) => {
                    let _ = app.emit(
                        STORAGE_CONNECTION_CHANGED_EVENT,
                        StorageConnectionChangedEvent {
                            storage_id: storage_id.to_string(),
                        },
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        })
        .ok();
}

/// Write every object name the storage listings ignored to the activity log,
/// for the life of the process. A `.DS_Store` or a sync client's conflicted
/// copy inside a Mallard namespace is never a record and never fails a
/// listing; the infrastructure announces each such name once.
pub fn start_ignored_name_forwarder<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    let mut events =
        mallard_infrastructure::storage_core::publication::ignored_name_events().subscribe();
    thread::Builder::new()
        .name("mallard-ignored-storage-names".to_string())
        .spawn(move || loop {
            match events.blocking_recv() {
                Ok(name) => crate::activity_log::emit_typed_log(
                    &app,
                    mallard_infrastructure::activity_log::ActivityLogType::Storage,
                    "warn",
                    &format!("Ignored '{name}' in the storage: it is not a Mallard object."),
                ),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        })
        .ok();
}

/// Tell the window, and the activity log, each time a link was rolled back
/// because another computer's Push outranked one this computer had confirmed.
pub fn start_outranked_push_forwarder<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    let mut events = super::service::outranked_push_events().subscribe();
    thread::Builder::new()
        .name("mallard-outranked-push-events".to_string())
        .spawn(move || loop {
            match events.blocking_recv() {
                Ok(outranked) => {
                    crate::activity_log::emit_typed_log(
                        &app,
                        mallard_infrastructure::activity_log::ActivityLogType::Push,
                        "warn",
                        &format!(
                            "Another computer's push to {} was selected over the last push of {}. That push is kept in the storage and counts as not made; push again.",
                            outranked.storage, outranked.project
                        ),
                    );
                    let _ = app.emit(
                        PUSH_OUTRANKED_EVENT,
                        PushOutrankedEvent {
                            local_project_id: outranked.local_project_id.to_string(),
                            storage_id: outranked.storage_id.to_string(),
                        },
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        })
        .ok();
}

const DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ProjectSyncConfigChangedEvent {
    pub revision: u64,
}

/// A value-free signal. The frontend reloads drafts through the existing
/// redacting command boundary instead of receiving draft contents in an event.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct StorageSetupDraftsChangedEvent {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct WatchChanges {
    config: bool,
    storage_setup_drafts: bool,
}

impl WatchChanges {
    fn merge(&mut self, other: Self) {
        self.config |= other.config;
        self.storage_setup_drafts |= other.storage_setup_drafts;
    }

    fn is_empty(self) -> bool {
        !self.config && !self.storage_setup_drafts
    }
}

/// Kept as managed Tauri state so the native watcher remains alive for the
/// application lifetime.  Dropping the watcher closes the worker channel.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    _worker: thread::JoinHandle<()>,
}

impl ConfigWatcher {
    pub fn start<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<Self, String> {
        let repository = LocalStateRepository::from_user_home()
            .map_err(|_| "Mallard could not initialize live configuration refresh.".to_string())?;
        let config_path = repository.config_path();
        let storage_setup_drafts_path = repository.root().join("storage_drafts");
        // A malformed document is surfaced by the normal shell load. It must
        // not prevent the desktop process from starting, and it must not seed
        // an event until a later valid revision can be loaded.
        let initial_revision = repository.load_config().ok().map(|config| config.revision);
        let (sender, receiver) = mpsc::channel::<WatchChanges>();

        let watched_config_path = config_path.clone();
        let watched_storage_setup_drafts_path = storage_setup_drafts_path.clone();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| {
                if let Ok(event) = result {
                    let changes = event_changes(
                        &event,
                        &watched_config_path,
                        &watched_storage_setup_drafts_path,
                    );
                    if !changes.is_empty() {
                        let _ = sender.send(changes);
                    }
                }
            },
            Config::default(),
        )
        .map_err(|_| "Mallard could not initialize live configuration refresh.".to_string())?;
        watcher
            .watch(repository.root(), RecursiveMode::Recursive)
            .map_err(|_| "Mallard could not start live configuration refresh.".to_string())?;

        let worker = thread::Builder::new()
            .name("mallard-config-watcher".to_string())
            .spawn(move || {
                let mut last_emitted_revision = initial_revision;
                while let Ok(mut changes) = receiver.recv() {
                    loop {
                        match receiver.recv_timeout(DEBOUNCE) {
                            Ok(next) => changes.merge(next),
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }

                    if changes.config {
                        // `load_config` takes the normal cross-process lock and
                        // validates the bounded document. A partial, malformed,
                        // or older external write produces no config event.
                        if let Ok(config) = repository.load_config() {
                            if last_emitted_revision
                                .is_none_or(|revision| config.revision > revision)
                            {
                                last_emitted_revision = Some(config.revision);
                                let _ = app.emit(
                                    CONFIG_CHANGED_EVENT,
                                    ProjectSyncConfigChangedEvent {
                                        revision: config.revision,
                                    },
                                );
                            }
                        }
                    }
                    if changes.storage_setup_drafts {
                        let _ = app.emit(
                            STORAGE_SETUP_DRAFTS_CHANGED_EVENT,
                            StorageSetupDraftsChangedEvent {},
                        );
                    }
                }
            })
            .map_err(|_| "Mallard could not start live configuration refresh.".to_string())?;

        Ok(Self {
            _watcher: watcher,
            _worker: worker,
        })
    }
}

fn event_changes(
    event: &Event,
    config_path: &Path,
    storage_setup_drafts_path: &Path,
) -> WatchChanges {
    if !matches!(
        event.kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return WatchChanges::default();
    }
    WatchChanges {
        config: event
            .paths
            .iter()
            .any(|path| same_config_filename(path, config_path)),
        // Include the directory itself and its direct children. Named
        // temporary files are part of an atomic save; the eventual rename is
        // debounced into the same value-free refresh signal.
        storage_setup_drafts: event.paths.iter().any(|path| {
            path == storage_setup_drafts_path
                || path
                    .parent()
                    .is_some_and(|parent| parent == storage_setup_drafts_path)
        }),
    }
}

fn same_config_filename(path: &Path, config_path: &Path) -> bool {
    path.file_name() == config_path.file_name()
        && path.parent().map(PathBuf::from) == config_path.parent().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, ModifyKind, RemoveKind};

    fn event(kind: EventKind, path: &str) -> Event {
        Event::new(kind).add_path(PathBuf::from(path))
    }

    #[test]
    fn storage_draft_change_event_payload_is_value_free() {
        assert_eq!(
            serde_json::to_value(StorageSetupDraftsChangedEvent {}).unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn classifies_atomic_replace_shapes_for_only_the_config_file() {
        let config_path = Path::new("/tmp/.mallard/sync_config.json");
        let drafts_path = Path::new("/tmp/.mallard/storage_drafts");
        assert_eq!(
            event_changes(
                &event(
                    EventKind::Create(CreateKind::File),
                    "/tmp/.mallard/sync_config.json"
                ),
                config_path,
                drafts_path,
            ),
            WatchChanges {
                config: true,
                storage_setup_drafts: false
            }
        );
        assert_eq!(
            event_changes(
                &event(
                    EventKind::Modify(ModifyKind::Any),
                    "/tmp/.mallard/sync_config.json"
                ),
                config_path,
                drafts_path,
            ),
            WatchChanges {
                config: true,
                storage_setup_drafts: false
            }
        );
        assert_eq!(
            event_changes(
                &event(
                    EventKind::Remove(RemoveKind::File),
                    "/tmp/.mallard/sync_config.json"
                ),
                config_path,
                drafts_path,
            ),
            WatchChanges {
                config: true,
                storage_setup_drafts: false
            }
        );
        assert_eq!(
            event_changes(
                &event(
                    EventKind::Modify(ModifyKind::Any),
                    "/tmp/.mallard/other.json"
                ),
                config_path,
                drafts_path,
            ),
            WatchChanges::default()
        );
    }

    #[test]
    fn classifies_storage_draft_directory_and_atomic_file_events() {
        let config_path = Path::new("/tmp/.mallard/sync_config.json");
        let drafts_path = Path::new("/tmp/.mallard/storage_drafts");
        for path in [
            "/tmp/.mallard/storage_drafts",
            "/tmp/.mallard/storage_drafts/.tmpAbCd",
            "/tmp/.mallard/storage_drafts/storage-draft-123.json",
        ] {
            assert_eq!(
                event_changes(
                    &event(EventKind::Modify(ModifyKind::Any), path),
                    config_path,
                    drafts_path,
                ),
                WatchChanges {
                    config: false,
                    storage_setup_drafts: true,
                },
            );
        }
    }

    #[test]
    fn ignores_nested_and_unrelated_machine_state() {
        let config_path = Path::new("/tmp/.mallard/sync_config.json");
        let drafts_path = Path::new("/tmp/.mallard/storage_drafts");
        for path in [
            "/tmp/.mallard/logs/activity.jsonl",
            "/tmp/.mallard/storage_drafts/nested/draft.json",
            "/tmp/.mallard/project_drafts/project-draft-123.json",
        ] {
            assert_eq!(
                event_changes(
                    &event(EventKind::Modify(ModifyKind::Any), path),
                    config_path,
                    drafts_path,
                ),
                WatchChanges::default(),
            );
        }
    }
}
