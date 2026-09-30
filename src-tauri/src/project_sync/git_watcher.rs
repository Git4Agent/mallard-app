//! Watch the open project's Git refs and announce that its history moved.
//!
//! Activity rebuilds the session-to-commit mapping from `git log` on every
//! load, so a checkout or a commit made outside Mallard only needs a signal.
//! One project is watched at a time: the one whose Activity page is open. The
//! operating system pushes file events, so an idle watch costs nothing, and
//! only `HEAD`, `packed-refs`, and loose branch refs pass the filter — the
//! index, objects, logs, and lock files never wake the page. The event names
//! the project only; no path leaves the backend.

use super::service::repository;
use mallard_domain::LocalProjectId;
use mallard_infrastructure::project_activity::{project_git_dirs, ProjectGitDirs};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tauri::{Emitter as _, Manager as _};

pub const PROJECT_GIT_CHANGED_EVENT: &str = "project-git-changed";
/// A rebase or a pull rewrites many refs in a burst; one reload covers it.
const DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectGitChangedEvent {
    pub local_project_id: String,
}

/// The single active watch, kept as managed Tauri state.
#[derive(Default)]
pub struct ProjectGitWatcher {
    active: Option<ActiveWatch>,
}

struct ActiveWatch {
    local_project_id: LocalProjectId,
    dirs: ProjectGitDirs,
    _watch: GitRefWatch,
}

impl ProjectGitWatcher {
    /// Watch `dirs` for `local_project_id`, replacing any other project's
    /// watch. The same project at the same dirs keeps its running watch.
    fn follow(
        &mut self,
        local_project_id: LocalProjectId,
        dirs: ProjectGitDirs,
        on_change: impl Fn() + Send + 'static,
    ) -> Result<(), String> {
        if self.active.as_ref().is_some_and(|active| {
            active.local_project_id == local_project_id && active.dirs == dirs
        }) {
            return Ok(());
        }
        self.active = None;
        let watch = GitRefWatch::start(&dirs, on_change)?;
        self.active = Some(ActiveWatch {
            local_project_id,
            dirs,
            _watch: watch,
        });
        Ok(())
    }

    fn stop(&mut self, local_project_id: &LocalProjectId) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| &active.local_project_id == local_project_id)
        {
            self.active = None;
        }
    }
}

/// Dropping the native watcher drops its event handler, which closes the
/// worker's channel and ends the worker.
struct GitRefWatch {
    _watcher: RecommendedWatcher,
    _worker: thread::JoinHandle<()>,
}

impl GitRefWatch {
    fn start(dirs: &ProjectGitDirs, on_change: impl Fn() + Send + 'static) -> Result<Self, String> {
        let unavailable = || "Mallard could not watch this project's Git history.".to_string();
        let (sender, receiver) = mpsc::channel::<()>();
        let filter_dirs = dirs.clone();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| {
                if result.is_ok_and(|event| is_ref_event(&event, &filter_dirs)) {
                    let _ = sender.send(());
                }
            },
            Config::default(),
        )
        .map_err(|_| unavailable())?;
        watcher
            .watch(&dirs.git_dir, RecursiveMode::NonRecursive)
            .map_err(|_| unavailable())?;
        if dirs.common_dir != dirs.git_dir {
            watcher
                .watch(&dirs.common_dir, RecursiveMode::NonRecursive)
                .map_err(|_| unavailable())?;
        }
        let heads = dirs.common_dir.join("refs").join("heads");
        if heads.is_dir() {
            watcher
                .watch(&heads, RecursiveMode::Recursive)
                .map_err(|_| unavailable())?;
        }
        let worker = thread::Builder::new()
            .name("mallard-git-watcher".to_string())
            .spawn(move || {
                while receiver.recv().is_ok() {
                    loop {
                        match receiver.recv_timeout(DEBOUNCE) {
                            Ok(()) => {}
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    on_change();
                }
            })
            .map_err(|_| unavailable())?;
        Ok(Self {
            _watcher: watcher,
            _worker: worker,
        })
    }
}

fn is_ref_event(event: &Event, dirs: &ProjectGitDirs) -> bool {
    matches!(
        event.kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) && event
        .paths
        .iter()
        .any(|path| is_ref_path(&canonical_event_path(path), dirs))
}

/// Normalize watcher paths to the same spelling as the Git directories.
/// A removal event may name a missing path, so canonicalize its nearest
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

/// `HEAD` moves on a checkout, a branch tip on a commit, and `packed-refs` on
/// a pack or a fetch. Git writes each through a `.lock` sibling it renames
/// into place, so the lock itself is never the signal.
fn is_ref_path(path: &Path, dirs: &ProjectGitDirs) -> bool {
    if path
        .extension()
        .is_some_and(|extension| extension == "lock")
    {
        return false;
    }
    path == dirs.git_dir.join("HEAD")
        || path == dirs.common_dir.join("packed-refs")
        || path.starts_with(dirs.common_dir.join("refs").join("heads"))
}

/// Watch the project whose Activity page is open. A project that is not a Git
/// work tree clears any watch, since its page replaced the watched one.
#[tauri::command]
pub async fn watch_project_git(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<(), String> {
    let repository = repository()?;
    let id = local_project_id.clone();
    let dirs = tauri::async_runtime::spawn_blocking(move || project_git_dirs(&repository, &id))
        .await
        .map_err(|error| format!("project-sync worker failed: {error}"))??;
    let state = app.state::<Mutex<ProjectGitWatcher>>();
    let mut watcher = state
        .lock()
        .map_err(|_| "The Git watcher is unavailable.".to_string())?;
    let Some(dirs) = dirs else {
        watcher.active = None;
        return Ok(());
    };
    let emitter = app.clone();
    let event = ProjectGitChangedEvent {
        local_project_id: local_project_id.to_string(),
    };
    watcher.follow(local_project_id, dirs, move || {
        let _ = emitter.emit(PROJECT_GIT_CHANGED_EVENT, event.clone());
    })
}

#[tauri::command]
pub async fn unwatch_project_git(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<(), String> {
    let state = app.state::<Mutex<ProjectGitWatcher>>();
    state
        .lock()
        .map_err(|_| "The Git watcher is unavailable.".to_string())?
        .stop(&local_project_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_infrastructure::project_activity::git_dirs;
    use std::process::Command;
    use std::time::Instant;

    fn dirs(git_dir: &str, common_dir: &str) -> ProjectGitDirs {
        ProjectGitDirs {
            git_dir: PathBuf::from(git_dir),
            common_dir: PathBuf::from(common_dir),
        }
    }

    #[test]
    fn only_head_branch_refs_and_packed_refs_are_history() {
        let clone = dirs("/repo/.git", "/repo/.git");
        for path in [
            "/repo/.git/HEAD",
            "/repo/.git/packed-refs",
            "/repo/.git/refs/heads/main",
            "/repo/.git/refs/heads/feature/cli",
        ] {
            assert!(is_ref_path(Path::new(path), &clone), "{path}");
        }
        for path in [
            "/repo/.git/index",
            "/repo/.git/objects/ab/cdef",
            "/repo/.git/logs/HEAD",
            "/repo/.git/HEAD.lock",
            "/repo/.git/refs/heads/main.lock",
            "/repo/.git/refs/tags/v1",
            "/repo/.git/FETCH_HEAD",
            "/repo/HEAD",
        ] {
            assert!(!is_ref_path(Path::new(path), &clone), "{path}");
        }
    }

    #[test]
    fn a_linked_worktree_reads_head_from_its_own_dir_and_refs_from_the_common_one() {
        let worktree = dirs("/repo/.git/worktrees/feature", "/repo/.git");
        assert!(is_ref_path(
            Path::new("/repo/.git/worktrees/feature/HEAD"),
            &worktree
        ));
        assert!(is_ref_path(
            Path::new("/repo/.git/refs/heads/feature"),
            &worktree
        ));
        // Another worktree's checkout is not this one's history.
        assert!(!is_ref_path(Path::new("/repo/.git/HEAD"), &worktree));
        assert!(!is_ref_path(
            Path::new("/repo/.git/worktrees/other/HEAD"),
            &worktree
        ));
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn a_checkout_outside_mallard_signals_once() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "--initial-branch=main"]);
        git(root, &["config", "user.email", "mallard@example.test"]);
        git(root, &["config", "user.name", "Mallard Test"]);
        git(root, &["commit", "--allow-empty", "-m", "Initial"]);
        let dirs = git_dirs(root).expect("the temporary project is a Git repository");
        let (sender, receiver) = mpsc::channel();
        let _watch = GitRefWatch::start(&dirs, move || {
            let _ = sender.send(());
        })
        .unwrap();
        // Let the native stream arm before the change it must observe.
        thread::sleep(Duration::from_millis(300));
        // Status refreshes the index; that is not history and must stay quiet.
        git(root, &["status"]);

        git(root, &["checkout", "-b", "feature"]);
        git(root, &["commit", "--allow-empty", "-m", "Feature"]);

        let deadline = Instant::now() + Duration::from_secs(10);
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the checkout and commit signal a history change");
        assert!(
            receiver.recv_timeout(DEBOUNCE * 3).is_err(),
            "one burst of ref updates is one signal"
        );
    }
}
