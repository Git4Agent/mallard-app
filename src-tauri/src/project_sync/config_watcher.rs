//! Watch the machine-local project-sync document and announce newer revisions.
//!
//! Persistence replaces `sync_config.json` atomically, so the parent directory
//! is watched instead of a file inode.  The event payload deliberately carries
//! only a revision; storage credentials remain behind the command boundary.

use super::persistence::LocalStateRepository;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use tauri::Emitter as _;

pub const CONFIG_CHANGED_EVENT: &str = "project-sync-config-changed";
const DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ProjectSyncConfigChangedEvent {
    pub revision: u64,
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
        // A malformed document is surfaced by the normal shell load. It must
        // not prevent the desktop process from starting, and it must not seed
        // an event until a later valid revision can be loaded.
        let initial_revision = repository.load_config().ok().map(|config| config.revision);
        let (sender, receiver) = mpsc::channel::<()>();

        let watched_config_path = config_path.clone();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| {
                if let Ok(event) = result {
                    if event_is_relevant(&event, &watched_config_path) {
                        let _ = sender.send(());
                    }
                }
            },
            Config::default(),
        )
        .map_err(|_| "Mallard could not initialize live configuration refresh.".to_string())?;
        watcher
            .watch(repository.root(), RecursiveMode::NonRecursive)
            .map_err(|_| "Mallard could not start live configuration refresh.".to_string())?;

        let worker = thread::Builder::new()
            .name("mallard-config-watcher".to_string())
            .spawn(move || {
                let mut last_emitted_revision = initial_revision;
                while receiver.recv().is_ok() {
                    loop {
                        match receiver.recv_timeout(DEBOUNCE) {
                            Ok(()) => continue,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }

                    // `load_config` takes the normal cross-process lock and
                    // validates the bounded document.  A partial, malformed,
                    // or older external write produces no frontend event.
                    let Ok(config) = repository.load_config() else {
                        continue;
                    };
                    if last_emitted_revision.is_some_and(|revision| config.revision <= revision) {
                        continue;
                    }
                    last_emitted_revision = Some(config.revision);
                    let _ = app.emit(
                        CONFIG_CHANGED_EVENT,
                        ProjectSyncConfigChangedEvent {
                            revision: config.revision,
                        },
                    );
                }
            })
            .map_err(|_| "Mallard could not start live configuration refresh.".to_string())?;

        Ok(Self {
            _watcher: watcher,
            _worker: worker,
        })
    }
}

fn event_is_relevant(event: &Event, config_path: &Path) -> bool {
    if !matches!(
        event.kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event
        .paths
        .iter()
        .any(|path| same_config_filename(path, config_path))
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
    fn accepts_atomic_replace_shapes_for_only_the_config_file() {
        let config_path = Path::new("/tmp/.mallard/sync_config.json");
        assert!(event_is_relevant(
            &event(
                EventKind::Create(CreateKind::File),
                "/tmp/.mallard/sync_config.json"
            ),
            config_path
        ));
        assert!(event_is_relevant(
            &event(
                EventKind::Modify(ModifyKind::Any),
                "/tmp/.mallard/sync_config.json"
            ),
            config_path
        ));
        assert!(event_is_relevant(
            &event(
                EventKind::Remove(RemoveKind::File),
                "/tmp/.mallard/sync_config.json"
            ),
            config_path
        ));
        assert!(!event_is_relevant(
            &event(
                EventKind::Modify(ModifyKind::Any),
                "/tmp/.mallard/other.json"
            ),
            config_path
        ));
    }
}
