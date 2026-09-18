//! Verify native watcher delivery before a gated E2E scenario starts.

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::io::Write as _;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const EVENTS_UNAVAILABLE: &str = "Native file-watcher events are unavailable in this environment. Run this E2E test outside the outer sandbox.";

pub fn require_native_watcher_events() -> Result<(), &'static str> {
    let temporary =
        tempfile::tempdir().map_err(|_| "Cannot create the watcher probe directory.")?;
    let directory = temporary
        .path()
        .canonicalize()
        .map_err(|_| "Cannot resolve the watcher probe directory.")?;
    let transcript = directory.join("session.jsonl");
    std::fs::write(&transcript, b"{\"turn\":1}\n")
        .map_err(|_| "Cannot create the watcher probe file.")?;

    // Use the application's native backend. Watch registration can succeed
    // even when the outer sandbox prevents event delivery.
    let (sender, receiver) = mpsc::sync_channel(16);
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = sender.try_send(event);
        },
        Config::default(),
    )
    .map_err(|_| EVENTS_UNAVAILABLE)?;
    watcher
        .watch(&directory, RecursiveMode::Recursive)
        .map_err(|_| EVENTS_UNAVAILABLE)?;

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .map_err(|_| "Cannot open the watcher probe file.")?;
    file.write_all(b"{\"turn\":2}\n")
        .and_then(|()| file.sync_all())
        .map_err(|_| "Cannot append to the watcher probe file.")?;
    drop(file);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let event = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| EVENTS_UNAVAILABLE)?
            .map_err(|_| EVENTS_UNAVAILABLE)?;
        if event.need_rescan() {
            return Err(EVENTS_UNAVAILABLE);
        }
        if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
            && event
                .paths
                .iter()
                .any(|path| path.canonicalize().ok().as_ref() == Some(&transcript))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(EVENTS_UNAVAILABLE);
        }
    }
}
