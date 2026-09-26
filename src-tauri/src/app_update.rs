//! In-app updates from GitHub Releases, owned by the backend.
//!
//! A check that finds a newer published release downloads it at once; the
//! plugin verifies the minisign signature before `download` returns, so a
//! package in the ready slot is always one the pinned key signed. The slot
//! lives here rather than in the webview so the package outlives the window:
//! the user installs it with the sidebar's Update button (install, then
//! restart), or by quitting, when `install_on_exit` puts it in place for the
//! next launch. Every failure is returned to the caller, which stays quiet.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);

struct ReadyUpdate {
    update: Update,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub struct AppUpdateState {
    /// Serializes check and download, which can take minutes. Exit never
    /// waits on it: a quit during a download simply leaves nothing to install.
    fetch: tokio::sync::Mutex<()>,
    /// The verified package waiting for a restart or a quit.
    ready: Mutex<Option<ReadyUpdate>>,
}

impl AppUpdateState {
    fn ready(&self) -> MutexGuard<'_, Option<ReadyUpdate>> {
        self.ready
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AppUpdateStatus {
    pub current_version: String,
    /// A newer release whose package is downloaded and verified.
    pub ready_version: Option<String>,
}

/// The `.app` bundle an executable runs from: `<bundle>.app/Contents/MacOS/<bin>`.
fn macos_bundle_of(executable: &Path) -> Option<&Path> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then_some(bundle)
}

/// Only an installed app replaces itself. Under `tauri dev` the executable
/// is `target/debug/<bin>`, and the plugin would take that folder for the
/// application and replace it.
fn is_installed_app(dev: bool, executable: &Path) -> bool {
    !dev && (!cfg!(target_os = "macos") || macos_bundle_of(executable).is_some())
}

fn running_installed_app() -> bool {
    std::env::current_exe().is_ok_and(|executable| is_installed_app(tauri::is_dev(), &executable))
}

/// A quit installs only where no password is needed. Replacing a bundle in
/// a folder this user cannot write makes the plugin ask for an
/// administrator password, which must never appear unasked at quit or
/// logout; the Update button, where the user expects it, still installs.
#[cfg(target_os = "macos")]
fn installs_without_prompt(executable: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Some(folder) = macos_bundle_of(executable).and_then(Path::parent) else {
        return false;
    };
    let Ok(folder) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `folder` is a valid NUL-terminated path for the call's duration.
    unsafe { libc::access(folder.as_ptr(), libc::W_OK) == 0 }
}

/// The Windows installer runs per user (`nsis.installMode: currentUser`).
#[cfg(not(target_os = "macos"))]
fn installs_without_prompt(_executable: &Path) -> bool {
    true
}

/// Checks the published release and, when it is newer, downloads and
/// verifies it into the ready slot. A package already downloaded for the
/// same version is kept rather than fetched again.
#[tauri::command]
pub async fn prepare_app_update(
    app: AppHandle,
    state: State<'_, AppUpdateState>,
) -> Result<AppUpdateStatus, String> {
    if !running_installed_app() {
        return Err("Updates install only into the packaged Mallard app.".into());
    }
    let _fetch = state.fetch.lock().await;
    let current_version = app.package_info().version.to_string();
    let found = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    let Some(mut update) = found else {
        // The published release is no longer newer: a withdrawn release
        // must not install at the next quit.
        *state.ready() = None;
        return Ok(AppUpdateStatus {
            current_version,
            ready_version: None,
        });
    };
    let version = update.version.clone();
    if state
        .ready()
        .as_ref()
        .is_some_and(|ready| ready.update.version == version)
    {
        return Ok(AppUpdateStatus {
            current_version,
            ready_version: Some(version),
        });
    }
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    *state.ready() = Some(ReadyUpdate { update, bytes });
    Ok(AppUpdateStatus {
        current_version,
        ready_version: Some(version),
    })
}

/// Installs the ready package and restarts into it. A failed install drops
/// the package; the next check downloads it again.
#[tauri::command]
pub async fn restart_to_update(
    app: AppHandle,
    state: State<'_, AppUpdateState>,
) -> Result<(), String> {
    // Held through the restart: a check already in flight would otherwise
    // refill the slot, and the exit would install the same release again.
    let _fetch = state.fetch.lock().await;
    let Some(ReadyUpdate { update, bytes }) = state.ready().take() else {
        return Err("No update is ready to install.".into());
    };
    // Off the async runtime: the macOS install extracts the archive and may
    // ask the main thread for an administrator prompt. On Windows it starts
    // the installer, which relaunches Mallard, and exits this process.
    tauri::async_runtime::spawn_blocking(move || update.install(&bytes))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    // Not `restart()`, which parks the calling thread forever.
    app.request_restart();
    Ok(())
}

/// Puts a ready package in place as the app exits, so the next launch runs
/// it. Called from `RunEvent::Exit`, which every way out reaches — Quit,
/// closing the last window, the Dock, logout — on the main thread, after the
/// windows are gone. A failure changes nothing: the next launch checks again.
pub fn install_on_exit<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppUpdateState>() else {
        return;
    };
    let Some(ReadyUpdate { update, bytes }) = state.ready().take() else {
        return;
    };
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    if !is_installed_app(tauri::is_dev(), &executable) || !installs_without_prompt(&executable) {
        return;
    }
    let _ = update.restart_after_install(false).install(&bytes);
}

#[cfg(test)]
mod tests {
    use super::{is_installed_app, macos_bundle_of};
    use std::path::Path;

    #[test]
    fn only_an_app_bundle_executable_names_a_bundle() {
        assert_eq!(
            macos_bundle_of(Path::new(
                "/Applications/Mallard.app/Contents/MacOS/mallard"
            )),
            Some(Path::new("/Applications/Mallard.app"))
        );
        for executable in [
            "/repo/src-tauri/target/debug/mallard",
            "/repo/src-tauri/target/release/mallard",
            "/Applications/Mallard/Contents/MacOS/mallard",
            "/Applications/Mallard.app/Contents/Resources/mallard",
            "/Applications/Mallard.app/MacOS/mallard",
            "mallard",
        ] {
            assert_eq!(macos_bundle_of(Path::new(executable)), None, "{executable}");
        }
    }

    /// The install replaces the folder it believes is the application. A
    /// development run must never reach it, bundle or not.
    #[test]
    fn a_development_run_never_installs() {
        for executable in [
            "/Applications/Mallard.app/Contents/MacOS/mallard",
            "/repo/src-tauri/target/debug/mallard",
        ] {
            assert!(
                !is_installed_app(true, Path::new(executable)),
                "{executable}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_macos_release_installs_only_from_a_bundle() {
        assert!(is_installed_app(
            false,
            Path::new("/Applications/Mallard.app/Contents/MacOS/mallard")
        ));
        assert!(!is_installed_app(
            false,
            Path::new("/repo/src-tauri/target/release/mallard")
        ));
    }
}
