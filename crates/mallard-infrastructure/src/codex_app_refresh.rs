//! Optional Codex App refresh after a committed Pull conversation
//! transaction.
//!
//! Provider correctness never depends on this module. The adapter recognizes
//! only exact, validated application bundles, requests a graceful quit, and
//! relaunches that same bundle after the process has stopped.

#[cfg(target_os = "macos")]
use crate::codex_app::{find_installed_mac_app, InstalledMacApp};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
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
    /// The project's Codex home a restart relaunches the app with, when it
    /// is not the default one.
    pub relaunch_home: Option<String>,
    pub message: String,
}

impl CodexAppRefreshOffer {
    #[cfg(all(not(test), any(target_os = "macos", windows)))]
    fn new(
        operation_id: mallard_domain::ConversationOperationId,
        app_name: &str,
        state: CodexAppRefreshState,
        relaunch_home: Option<String>,
    ) -> Self {
        Self {
            operation_id,
            app_kind: CodexAppKind::Codex,
            app_name: app_name.to_string(),
            message: offer_message(app_name, state, relaunch_home.as_deref()),
            state,
            relaunch_home,
        }
    }
}

/// What an offer says: a running app may need a restart, and the home it
/// comes back with; an app Mallard could not check is restarted by hand.
pub fn offer_message(
    app_name: &str,
    state: CodexAppRefreshState,
    relaunch_home: Option<&str>,
) -> String {
    match state {
        CodexAppRefreshState::RestartAvailable => format!(
            "{app_name} may need a restart to refresh its conversation list.{}",
            relaunch_home
                .map(|home| format!(
                    " Mallard will relaunch it with this project's Codex home ({home})."
                ))
                .unwrap_or_default()
        ),
        CodexAppRefreshState::ManualRestart => {
            format!("Restart {app_name} manually if the synced conversation is not visible.")
        }
    }
}

/// What a finished restart did. A failure carries no outcome: its message
/// is an error.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexAppRestartOutcome {
    AlreadyClosed,
    Restarted,
    RestartedWithHome,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexAppRestartResult {
    pub success: bool,
    pub message: String,
    pub outcome: Option<CodexAppRestartOutcome>,
    pub app_name: Option<String>,
    /// The Codex home a `restarted_with_home` relaunch pinned.
    pub home_path: Option<String>,
}

impl CodexAppRestartResult {
    #[cfg(any(target_os = "macos", windows))]
    fn done(outcome: CodexAppRestartOutcome, app_name: &str, home_path: Option<String>) -> Self {
        Self {
            success: true,
            message: restart_message(outcome, app_name, home_path.as_deref()),
            outcome: Some(outcome),
            app_name: Some(app_name.to_string()),
            home_path,
        }
    }

    pub fn failed(message: String) -> Self {
        Self {
            success: false,
            message,
            outcome: None,
            app_name: None,
            home_path: None,
        }
    }
}

/// What a finished restart says.
pub fn restart_message(
    outcome: CodexAppRestartOutcome,
    app_name: &str,
    home_path: Option<&str>,
) -> String {
    match outcome {
        CodexAppRestartOutcome::AlreadyClosed => format!(
            "{app_name} is already closed; its conversation list will refresh next time it opens."
        ),
        CodexAppRestartOutcome::Restarted => format!("{app_name} restarted."),
        CodexAppRestartOutcome::RestartedWithHome => format!(
            "{app_name} restarted with this project's Codex home ({}).",
            home_path.unwrap_or_default()
        ),
    }
}

/// Offer a refresh for the app that will show the synced threads. The
/// restart is home-aware: the default `~/.codex` profile relaunches through
/// LaunchServices, while any other profile relaunches the validated bundle
/// executable directly with `CODEX_HOME` pinned to that profile.
pub fn refresh_offer(
    operation_id: mallard_domain::ConversationOperationId,
    codex_home: Option<&Path>,
) -> Option<CodexAppRefreshOffer> {
    #[cfg(all(not(test), any(target_os = "macos", windows)))]
    let relaunch_home = codex_home
        .filter(|home| !is_default_codex_home(home))
        .map(|home| home.display().to_string());
    #[cfg(all(not(test), target_os = "macos"))]
    {
        let app = find_installed_mac_app()?;
        let app_name = app.specification.app_name;
        match mac_app_is_running(&app) {
            Ok(true) => Some(CodexAppRefreshOffer::new(
                operation_id,
                app_name,
                CodexAppRefreshState::RestartAvailable,
                relaunch_home,
            )),
            Ok(false) => None,
            Err(_) => Some(CodexAppRefreshOffer::new(
                operation_id,
                app_name,
                CodexAppRefreshState::ManualRestart,
                None,
            )),
        }
    }
    #[cfg(all(not(test), windows))]
    {
        let app = find_installed_windows_app()?;
        match windows_app_is_running(&app) {
            Ok(true) => Some(CodexAppRefreshOffer::new(
                operation_id,
                WINDOWS_CODEX_APP_NAME,
                CodexAppRefreshState::RestartAvailable,
                relaunch_home,
            )),
            Ok(false) => None,
            Err(_) => Some(CodexAppRefreshOffer::new(
                operation_id,
                WINDOWS_CODEX_APP_NAME,
                CodexAppRefreshState::ManualRestart,
                None,
            )),
        }
    }
    #[cfg(any(test, not(any(target_os = "macos", windows))))]
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
        let CodexAppKind::Codex = app_kind;
        let app = find_installed_mac_app().ok_or_else(|| {
            "the reviewed Codex App installation is no longer available".to_string()
        })?;
        if !mac_app_is_running(&app)? {
            return Ok(CodexAppRestartResult::done(
                CodexAppRestartOutcome::AlreadyClosed,
                app.specification.app_name,
                None,
            ));
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
            app.specification.app_name
        ))
    }
    #[cfg(windows)]
    {
        let CodexAppKind::Codex = app_kind;
        let app = find_installed_windows_app().ok_or_else(|| {
            "the reviewed Codex App installation is no longer available".to_string()
        })?;
        if !windows_app_is_running(&app)? {
            return Ok(CodexAppRestartResult::done(
                CodexAppRestartOutcome::AlreadyClosed,
                WINDOWS_CODEX_APP_NAME,
                None,
            ));
        }
        request_windows_app_quit(&app)?;
        let deadline = Instant::now() + APP_QUIT_TIMEOUT;
        while Instant::now() < deadline {
            if !windows_app_is_running(&app)? {
                return relaunch_windows_app(&app, codex_home);
            }
            thread::sleep(Duration::from_millis(250));
        }
        Err(format!(
            "{WINDOWS_CODEX_APP_NAME} did not finish a graceful quit; Mallard did not force it \
             closed"
        ))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (app_kind, codex_home);
        Err("automatic Codex App restart is unavailable on this platform".to_string())
    }
}

/// Send a graceful quit request after the user selects Close Codex.
pub fn request_close() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let app = find_installed_mac_app()
            .ok_or("Codex App was not found. Close it manually, then try Delete again.")?;
        if mac_app_is_running(&app)? {
            request_mac_app_quit(&app)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let app = find_installed_windows_app()
            .ok_or("Codex App was not found. Close it manually, then try Delete again.")?;
        if windows_app_is_running(&app)? {
            request_windows_app_quit(&app)?;
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err("Close Codex manually, then try Delete again.".into())
    }
}

#[cfg(windows)]
const WINDOWS_CODEX_APP_NAME: &str = "Codex App";

/// The Codex desktop app on Windows, located through its `codex://` handler
/// rather than a fixed install path; its image name drives process checks.
#[cfg(windows)]
struct InstalledWindowsApp {
    executable: PathBuf,
    image_name: String,
}

#[cfg(windows)]
fn find_installed_windows_app() -> Option<InstalledWindowsApp> {
    let executable = crate::providers::windows_app::scheme_handler_executable("codex").ok()?;
    let image_name = executable.file_name()?.to_str()?.to_string();
    Some(InstalledWindowsApp {
        executable,
        image_name,
    })
}

/// `tasklist` prints matching rows as CSV, or an `INFO:` line when nothing
/// matches; both exit successfully, so the row itself is the answer.
#[cfg(windows)]
fn windows_app_is_running(app: &InstalledWindowsApp) -> Result<bool, String> {
    let filter = format!("IMAGENAME eq {}", app.image_name);
    let (success, output) = run_capture_bounded(
        Command::new("tasklist.exe")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .stdin(Stdio::null())
            .stderr(Stdio::null()),
        APP_COMMAND_TIMEOUT,
    )?;
    if !success {
        return Err("Codex App running state could not be queried".to_string());
    }
    let text = String::from_utf8_lossy(&output);
    Ok(text.lines().any(|line| {
        line.trim_start_matches('"')
            .to_ascii_lowercase()
            .starts_with(&app.image_name.to_ascii_lowercase())
    }))
}

/// `taskkill` without `/F` posts a close request, the Windows equivalent of
/// an AppleScript `quit`; the app may still refuse or prompt.
#[cfg(windows)]
fn request_windows_app_quit(app: &InstalledWindowsApp) -> Result<(), String> {
    if run_status_bounded(
        Command::new("taskkill.exe")
            .args(["/IM", &app.image_name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        APP_COMMAND_TIMEOUT,
    )? {
        Ok(())
    } else {
        Err(format!(
            "{WINDOWS_CODEX_APP_NAME} did not accept a graceful quit request"
        ))
    }
}

#[cfg(windows)]
fn relaunch_windows_app(
    app: &InstalledWindowsApp,
    codex_home: &Path,
) -> Result<CodexAppRestartResult, String> {
    use crate::providers::windows_app::{display_launch, launch_app};
    let pinned_home = [("CODEX_HOME", codex_home)];
    let pinned: &[(&str, &Path)] = if is_default_codex_home(codex_home) {
        &[]
    } else {
        &pinned_home
    };
    let manual_guidance = format!(
        "{WINDOWS_CODEX_APP_NAME} closed, but relaunching it did not complete. Start it \
         manually, for example: {}",
        display_launch(&app.executable, pinned, &[])
    );
    if launch_app(&app.executable, pinned, &[]).is_err() {
        return Err(manual_guidance);
    }
    let deadline = Instant::now() + APP_QUIT_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(true) = windows_app_is_running(app) {
            return Ok(if pinned.is_empty() {
                CodexAppRestartResult::done(
                    CodexAppRestartOutcome::Restarted,
                    WINDOWS_CODEX_APP_NAME,
                    None,
                )
            } else {
                CodexAppRestartResult::done(
                    CodexAppRestartOutcome::RestartedWithHome,
                    WINDOWS_CODEX_APP_NAME,
                    Some(codex_home.display().to_string()),
                )
            });
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(manual_guidance)
}

/// Relaunch a stopped, validated app bundle. The default `~/.codex` profile
/// goes through LaunchServices. Any other profile starts
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
        return Ok(CodexAppRestartResult::done(
            CodexAppRestartOutcome::Restarted,
            app.specification.app_name,
            None,
        ));
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
            return Ok(CodexAppRestartResult::done(
                CodexAppRestartOutcome::RestartedWithHome,
                app.specification.app_name,
                Some(codex_home.display().to_string()),
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(manual_guidance)
}

/// Whether this profile home is the one a plain LaunchServices relaunch
/// would already read. Unresolvable paths compare as non-default so the
/// pinned-environment launch path is chosen — the safe direction.
#[cfg(any(target_os = "macos", windows))]
fn is_default_codex_home(codex_home: &Path) -> bool {
    let Some(default_home) = crate::persistence::user_home_dir().map(|home| home.join(".codex"))
    else {
        return false;
    };
    codex_home_matches_default(codex_home, &default_home)
}

#[cfg(any(target_os = "macos", windows))]
fn codex_home_matches_default(codex_home: &Path, default_home: &Path) -> bool {
    if codex_home == default_home {
        return true;
    }
    match (fs::canonicalize(codex_home), fs::canonicalize(default_home)) {
        (Ok(profile), Ok(default_home)) => profile == default_home,
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn shell_quote_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
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

#[cfg(any(target_os = "macos", windows))]
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

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn offers_and_results_keep_their_english_beside_the_values() {
        assert_eq!(
            offer_message("Codex", CodexAppRefreshState::RestartAvailable, None),
            "Codex may need a restart to refresh its conversation list."
        );
        assert_eq!(
            offer_message(
                "ChatGPT",
                CodexAppRefreshState::RestartAvailable,
                Some("/work/.codex")
            ),
            "ChatGPT may need a restart to refresh its conversation list. Mallard will relaunch it with this project's Codex home (/work/.codex)."
        );
        assert_eq!(
            offer_message("Codex App", CodexAppRefreshState::ManualRestart, None),
            "Restart Codex App manually if the synced conversation is not visible."
        );
        assert_eq!(
            restart_message(CodexAppRestartOutcome::AlreadyClosed, "Codex", None),
            "Codex is already closed; its conversation list will refresh next time it opens."
        );
        assert_eq!(
            restart_message(CodexAppRestartOutcome::Restarted, "Codex", None),
            "Codex restarted."
        );
        assert_eq!(
            restart_message(
                CodexAppRestartOutcome::RestartedWithHome,
                "Codex",
                Some("/work/.codex")
            ),
            "Codex restarted with this project's Codex home (/work/.codex)."
        );
        let failed = CodexAppRestartResult::failed("did not quit".into());
        assert!(!failed.success && failed.outcome.is_none() && failed.app_name.is_none());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::codex_app::{
        find_installed_mac_app_in, mac_app_candidate_paths, validate_mac_app_bundle,
    };
    use std::path::PathBuf;

    fn bundle_fixture(
        applications: &Path,
        bundle_name: &str,
        bundle_id: &str,
        declared_executable: &str,
        executable_file: &str,
    ) -> PathBuf {
        let bundle = applications.join(bundle_name);
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(
            bundle.join("Contents/MacOS").join(executable_file),
            b"fixture",
        )
        .unwrap();
        let dictionary = plist::Dictionary::from_iter([
            (
                "CFBundleIdentifier".to_string(),
                plist::Value::String(bundle_id.to_string()),
            ),
            (
                "CFBundleExecutable".to_string(),
                plist::Value::String(declared_executable.to_string()),
            ),
        ]);
        plist::Value::Dictionary(dictionary)
            .to_file_xml(bundle.join("Contents/Info.plist"))
            .unwrap();
        bundle
    }

    #[test]
    fn app_bundle_validation_requires_exact_identity_and_executable() {
        for (bundle_name, bundle_id, declared_executable, executable_file, valid) in [
            (
                "ChatGPT.app",
                "com.openai.codex",
                "ChatGPT",
                "ChatGPT",
                true,
            ),
            (
                "ChatGPT.app",
                "com.openai.chat",
                "ChatGPT",
                "ChatGPT",
                false,
            ),
            ("ChatGPT.app", "com.openai.codex", "Codex", "Codex", false),
            ("ChatGPT.app", "com.openai.codex", "ChatGPT", "Codex", false),
            ("Codex.app", "com.openai.codex", "Codex", "Codex", false),
            ("Codex.app", "com.openai.codex", "ChatGPT", "ChatGPT", false),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let bundle = bundle_fixture(
                &fs::canonicalize(temp.path()).unwrap(),
                bundle_name,
                bundle_id,
                declared_executable,
                executable_file,
            );
            assert_eq!(
                validate_mac_app_bundle(&bundle).is_some(),
                valid,
                "{bundle_name}: id={bundle_id}, declared={declared_executable}, file={executable_file}"
            );
        }
    }

    #[test]
    fn app_lookup_selects_only_a_valid_official_bundle_in_install_order() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let applications = root.join("Applications");
        let user_home = root.join("user");
        assert_eq!(
            mac_app_candidate_paths(&applications, Some(&user_home)),
            vec![
                applications.join("ChatGPT.app"),
                user_home.join("Applications/ChatGPT.app"),
            ]
        );
        assert!(find_installed_mac_app_in(&applications, Some(&user_home)).is_none());
        bundle_fixture(
            &applications,
            "Codex.app",
            "com.openai.codex",
            "Codex",
            "Codex",
        );
        assert!(find_installed_mac_app_in(&applications, Some(&user_home)).is_none());

        let user_bundle = bundle_fixture(
            &user_home.join("Applications"),
            "ChatGPT.app",
            "com.openai.codex",
            "ChatGPT",
            "ChatGPT",
        );
        assert_eq!(
            find_installed_mac_app_in(&applications, Some(&user_home))
                .unwrap()
                .bundle_path,
            user_bundle
        );
        bundle_fixture(
            &applications,
            "ChatGPT.app",
            "com.openai.chat",
            "ChatGPT",
            "ChatGPT",
        );
        assert_eq!(
            find_installed_mac_app_in(&applications, Some(&user_home))
                .unwrap()
                .bundle_path,
            user_bundle
        );
        let system_bundle = bundle_fixture(
            &applications,
            "ChatGPT.app",
            "com.openai.codex",
            "ChatGPT",
            "ChatGPT",
        );
        assert_eq!(
            find_installed_mac_app_in(&applications, Some(&user_home))
                .unwrap()
                .bundle_path,
            system_bundle
        );
    }

    #[test]
    fn app_bundle_validation_rejects_symlinked_bundle_plist_and_executable() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let bundle = bundle_fixture(
            &root,
            "ChatGPT.app",
            "com.openai.codex",
            "ChatGPT",
            "ChatGPT",
        );
        for relative_path in ["Contents/Info.plist", "Contents/MacOS/ChatGPT"] {
            let path = bundle.join(relative_path);
            let outside = root.join("outside-file");
            fs::rename(&path, &outside).unwrap();
            symlink(&outside, &path).unwrap();
            assert!(validate_mac_app_bundle(&bundle).is_none());
            fs::remove_file(&path).unwrap();
            fs::rename(&outside, &path).unwrap();
        }
        let outside = root.join("outside-bundle");
        fs::rename(&bundle, &outside).unwrap();
        symlink(&outside, &bundle).unwrap();
        assert!(validate_mac_app_bundle(&bundle).is_none());
    }

    #[test]
    fn app_kind_accepts_only_the_official_app() {
        assert_eq!(
            serde_json::to_string(&CodexAppKind::Codex).unwrap(),
            "\"codex\""
        );
        assert!(serde_json::from_str::<CodexAppKind>("\"codex\"").is_ok());
        assert!(serde_json::from_str::<CodexAppKind>("\"codex_plus_plus\"").is_err());
    }

    #[test]
    fn only_the_default_home_relaunches_through_launch_services() {
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        let default_home = home.join(".codex");
        let profile_home = home.join("conf2/.codex");
        fs::create_dir_all(&default_home).unwrap();
        fs::create_dir_all(&profile_home).unwrap();
        assert!(codex_home_matches_default(&default_home, &default_home));
        assert!(!codex_home_matches_default(&profile_home, &default_home));
        assert!(!codex_home_matches_default(
            &home.join("missing"),
            &default_home
        ));
        let alias = home.join("codex-home-alias");
        std::os::unix::fs::symlink(&default_home, &alias).unwrap();
        assert!(codex_home_matches_default(&alias, &default_home));
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
