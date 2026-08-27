//! Optional Codex App refresh after a committed Pull conversation
//! transaction.
//!
//! Provider correctness never depends on this module. The adapter recognizes
//! only exact, validated application bundles, requests a graceful quit, and
//! relaunches that same bundle after the process has stopped.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const APP_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const APP_QUIT_TIMEOUT: Duration = Duration::from_secs(15);
const APP_OUTPUT_LIMIT: usize = 4 * 1024;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexAppKind {
    Codex,
    CodexPlusPlus,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexAppRefreshState {
    RestartAvailable,
    ManualRestart,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexAppRefreshOffer {
    pub operation_id: mallard_domain::ConversationOperationId,
    pub app_kind: CodexAppKind,
    pub app_name: String,
    pub state: CodexAppRefreshState,
    pub message: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexAppRestartResult {
    pub success: bool,
    pub message: String,
}

#[derive(Clone, Copy, Debug)]
struct SupportedMacApp {
    kind: CodexAppKind,
    app_name: &'static str,
    bundle_name: &'static str,
    bundle_id: &'static str,
    executable_name: &'static str,
}

#[derive(Clone, Debug)]
struct InstalledMacApp {
    specification: SupportedMacApp,
    bundle_path: PathBuf,
}

const SUPPORTED_MAC_APPS: [SupportedMacApp; 2] = [
    SupportedMacApp {
        kind: CodexAppKind::Codex,
        app_name: "Codex App",
        bundle_name: "Codex.app",
        bundle_id: "com.openai.codex",
        executable_name: "Codex",
    },
    SupportedMacApp {
        kind: CodexAppKind::CodexPlusPlus,
        app_name: "Codex++",
        bundle_name: "Codex++.app",
        bundle_id: "com.bigpizzav3.codexplusplus",
        executable_name: "CodexPlusPlus",
    },
];

/// Offer a refresh for the app that will show the synced threads. The
/// restart is home-aware: the default `~/.codex` profile relaunches through
/// LaunchServices, while any other profile relaunches the validated bundle
/// executable directly with `CODEX_HOME` pinned to that profile.
pub fn refresh_offer(
    operation_id: mallard_domain::ConversationOperationId,
    codex_home: Option<&Path>,
) -> Option<CodexAppRefreshOffer> {
    #[cfg(all(not(test), target_os = "macos"))]
    {
        let profile_note = codex_home
            .filter(|home| !is_default_codex_home(home))
            .map(|home| {
                format!(
                    " Mallard will relaunch it with this project's Codex home ({}).",
                    home.display()
                )
            })
            .unwrap_or_default();
        let mut manual_offer = None;
        for specification in SUPPORTED_MAC_APPS {
            let Some(app) = find_installed_mac_app(specification) else {
                continue;
            };
            match mac_app_is_running(&app) {
                Ok(true) => {
                    return Some(CodexAppRefreshOffer {
                        operation_id,
                        app_kind: specification.kind,
                        app_name: specification.app_name.to_string(),
                        state: CodexAppRefreshState::RestartAvailable,
                        message: format!(
                            "{} may need a restart to refresh its conversation list.{}",
                            specification.app_name, profile_note
                        ),
                    });
                }
                Err(_) if manual_offer.is_none() => {
                    manual_offer = Some(CodexAppRefreshOffer {
                        operation_id: operation_id.clone(),
                        app_kind: specification.kind,
                        app_name: specification.app_name.to_string(),
                        state: CodexAppRefreshState::ManualRestart,
                        message: format!(
                            "Restart {} manually if the synced conversation is not visible.",
                            specification.app_name
                        ),
                    });
                }
                Ok(false) | Err(_) => {}
            }
        }
        manual_offer
    }
    #[cfg(any(test, not(target_os = "macos")))]
    {
        let _ = (operation_id, codex_home);
        None
    }
}

pub fn restart_after_pull(
    app_kind: CodexAppKind,
    codex_home: &Path,
) -> Result<CodexAppRestartResult, String> {
    #[cfg(target_os = "macos")]
    {
        let specification = SUPPORTED_MAC_APPS
            .into_iter()
            .find(|candidate| candidate.kind == app_kind)
            .ok_or_else(|| "Codex App restart adapter is unavailable".to_string())?;
        let app = find_installed_mac_app(specification).ok_or_else(|| {
            "the reviewed Codex App installation is no longer available".to_string()
        })?;
        if !mac_app_is_running(&app)? {
            return Ok(CodexAppRestartResult {
                success: true,
                message: format!(
                    "{} is already closed; its conversation list will refresh next time it opens.",
                    specification.app_name
                ),
            });
        }
        request_mac_app_quit(&app)?;
        let deadline = Instant::now() + APP_QUIT_TIMEOUT;
        while Instant::now() < deadline {
            if !mac_app_is_running(&app)? {
                return relaunch_mac_app(&app, codex_home);
            }
            thread::sleep(Duration::from_millis(250));
        }
        Err(format!(
            "{} did not finish a graceful quit; Mallard did not force it closed",
            specification.app_name
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_kind, codex_home);
        Err("automatic Codex App restart is unavailable on this platform".to_string())
    }
}

/// Relaunch a stopped, validated app bundle. The default `~/.codex` profile
/// goes through LaunchServices, exactly as before. Any other profile starts
/// the bundle executable directly so `CODEX_HOME` can be pinned — a
/// LaunchServices relaunch would silently come up with the GUI session
/// environment and read the wrong home. If the pinned launch cannot be
/// proven, the caller gets manual guidance with the exact command; the
/// graceful quit has already happened either way.
#[cfg(target_os = "macos")]
fn relaunch_mac_app(
    app: &InstalledMacApp,
    codex_home: &Path,
) -> Result<CodexAppRestartResult, String> {
    if is_default_codex_home(codex_home) {
        let status = run_status_bounded(
            Command::new("/usr/bin/open")
                .arg("-a")
                .arg(&app.bundle_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
            APP_COMMAND_TIMEOUT,
        )?;
        if !status {
            return Err(format!(
                "{} closed, but the exact application bundle could not be relaunched",
                app.specification.app_name
            ));
        }
        return Ok(CodexAppRestartResult {
            success: true,
            message: format!("{} restarted.", app.specification.app_name),
        });
    }
    let executable = app
        .bundle_path
        .join("Contents/MacOS")
        .join(app.specification.executable_name);
    let manual_guidance = format!(
        "{} closed, but relaunching it with this project's Codex home did not \
         complete. Start it manually, for example: CODEX_HOME={} {}",
        app.specification.app_name,
        shell_quote_path(codex_home),
        shell_quote_path(&executable),
    );
    let mut child = match Command::new(&executable)
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Err(manual_guidance),
    };
    let deadline = Instant::now() + APP_QUIT_TIMEOUT;
    while Instant::now() < deadline {
        // Reap an early exit so a crashed launch never lingers as a zombie;
        // the LaunchServices running check stays the only success authority.
        let _ = child.try_wait();
        if let Ok(true) = mac_app_is_running(app) {
            return Ok(CodexAppRestartResult {
                success: true,
                message: format!(
                    "{} restarted with this project's Codex home ({}).",
                    app.specification.app_name,
                    codex_home.display()
                ),
            });
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(manual_guidance)
}

/// Whether this profile home is the one a plain LaunchServices relaunch
/// would already read. Unresolvable paths compare as non-default so the
/// pinned-environment launch path is chosen — the safe direction.
#[cfg(target_os = "macos")]
fn is_default_codex_home(codex_home: &Path) -> bool {
    let Some(default_home) = dirs::home_dir().map(|home| home.join(".codex")) else {
        return false;
    };
    if codex_home == default_home {
        return true;
    }
    match (
        fs::canonicalize(codex_home),
        fs::canonicalize(&default_home),
    ) {
        (Ok(profile), Ok(default_home)) => profile == default_home,
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn shell_quote_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn find_installed_mac_app(specification: SupportedMacApp) -> Option<InstalledMacApp> {
    mac_app_candidate_paths(specification.bundle_name)
        .into_iter()
        .find_map(|path| validate_mac_app_bundle(&path, specification))
}

#[cfg(target_os = "macos")]
fn mac_app_candidate_paths(bundle_name: &str) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/Applications").join(bundle_name)];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join("Applications").join(bundle_name));
    }
    paths
}

#[cfg(target_os = "macos")]
fn validate_mac_app_bundle(path: &Path, specification: SupportedMacApp) -> Option<InstalledMacApp> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let canonical = fs::canonicalize(path).ok()?;
    if canonical != path {
        return None;
    }
    let plist_path = canonical.join("Contents/Info.plist");
    let plist_metadata = fs::symlink_metadata(&plist_path).ok()?;
    if !plist_metadata.is_file() || plist_metadata.file_type().is_symlink() {
        return None;
    }
    let dictionary = plist::Value::from_file(&plist_path)
        .ok()?
        .into_dictionary()?;
    if dictionary
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)
        != Some(specification.bundle_id)
        || dictionary
            .get("CFBundleExecutable")
            .and_then(plist::Value::as_string)
            != Some(specification.executable_name)
    {
        return None;
    }
    let executable = canonical
        .join("Contents/MacOS")
        .join(specification.executable_name);
    let executable_metadata = fs::symlink_metadata(&executable).ok()?;
    if !executable_metadata.is_file() || executable_metadata.file_type().is_symlink() {
        return None;
    }
    let canonical_executable = fs::canonicalize(&executable).ok()?;
    if !canonical_executable.starts_with(&canonical) {
        return None;
    }
    Some(InstalledMacApp {
        specification,
        bundle_path: canonical,
    })
}

#[cfg(target_os = "macos")]
fn mac_app_is_running(app: &InstalledMacApp) -> Result<bool, String> {
    let script = format!(
        "application id {} is running",
        apple_script_string(app.specification.bundle_id)
    );
    let (success, output) = run_capture_bounded(
        Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .stdin(Stdio::null())
            .stderr(Stdio::null()),
        APP_COMMAND_TIMEOUT,
    )?;
    if !success {
        return Err("Codex App running state could not be queried".to_string());
    }
    match std::str::from_utf8(&output).ok().map(str::trim) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err("Codex App running state was not recognized".to_string()),
    }
}

#[cfg(target_os = "macos")]
fn request_mac_app_quit(app: &InstalledMacApp) -> Result<(), String> {
    let script = format!(
        "tell application id {} to quit",
        apple_script_string(app.specification.bundle_id)
    );
    if run_status_bounded(
        Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        APP_COMMAND_TIMEOUT,
    )? {
        Ok(())
    } else {
        Err(format!(
            "{} did not accept a graceful quit request",
            app.specification.app_name
        ))
    }
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
fn run_capture_bounded(
    command: &mut Command,
    timeout: Duration,
) -> Result<(bool, Vec<u8>), String> {
    command.stdout(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|_| "start Codex App platform adapter".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex App platform adapter output is unavailable".to_string())?;
    let output_worker = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout
            .take(APP_OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut output);
        output
    });
    let deadline = Instant::now() + timeout;
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex App platform adapter timed out".to_string());
            }
        }
    };
    let output = output_worker
        .join()
        .map_err(|_| "Codex App platform adapter output failed".to_string())?;
    if output.len() > APP_OUTPUT_LIMIT {
        return Err("Codex App platform adapter output exceeded its limit".to_string());
    }
    Ok((success, output))
}

fn run_status_bounded(command: &mut Command, timeout: Duration) -> Result<bool, String> {
    let mut child = command
        .spawn()
        .map_err(|_| "start Codex App platform adapter".to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex App platform adapter timed out".to_string());
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn app_bundle_validation_requires_exact_identity_and_executable() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = fs::canonicalize(temp.path()).unwrap().join("Codex.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/MacOS/Codex"), b"fixture").unwrap();
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            plist::Value::String("com.openai.codex".to_string()),
        );
        dictionary.insert(
            "CFBundleExecutable".to_string(),
            plist::Value::String("Codex".to_string()),
        );
        plist::Value::Dictionary(dictionary)
            .to_file_xml(bundle.join("Contents/Info.plist"))
            .unwrap();

        assert!(validate_mac_app_bundle(&bundle, SUPPORTED_MAC_APPS[0]).is_some());
        assert!(validate_mac_app_bundle(&bundle, SUPPORTED_MAC_APPS[1]).is_none());
    }

    #[test]
    fn only_the_default_home_relaunches_through_launch_services() {
        let default_home = dirs::home_dir().unwrap().join(".codex");
        assert!(is_default_codex_home(&default_home));

        let temp = tempfile::tempdir().unwrap();
        let profile_home = fs::canonicalize(temp.path()).unwrap().join("conf2/.codex");
        assert!(!is_default_codex_home(&profile_home));
    }

    #[test]
    fn manual_guidance_shell_quotes_the_profile_home() {
        assert_eq!(
            shell_quote_path(Path::new("/tmp/conf 2/.codex")),
            "'/tmp/conf 2/.codex'"
        );
        assert_eq!(
            shell_quote_path(Path::new("/tmp/it's/.codex")),
            "'/tmp/it'\\''s/.codex'"
        );
    }
}
