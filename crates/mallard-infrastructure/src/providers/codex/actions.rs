//! Safe Codex session actions: desktop-app deep links and Terminal resume.

use crate::providers::launch::{launch_terminal, shell_quote, TerminalCommand};
use crate::providers::traits::{
    SessionActionContext, SessionActionDescriptor, SessionActionKind, SessionActionOutcome,
    SessionActionRequest,
};
use std::path::Path;
#[cfg(any(target_os = "macos", windows))]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(any(target_os = "macos", windows))]
use std::time::{Duration, SystemTime};

pub(super) const OPEN_IN_APP: &str = "codex.open-in-app";
pub(super) const OPEN_IN_TERMINAL: &str = "codex.open-in-terminal";

#[cfg(any(target_os = "macos", windows))]
const CODEX_PROJECT_ROUTE_TIMEOUT: Duration = Duration::from_millis(2500);
#[cfg(any(target_os = "macos", windows))]
const CODEX_PROJECT_ROUTE_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn validate_thread_uuid(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    valid
        .then_some(())
        .ok_or_else(|| "Codex thread id must be a UUID".to_string())
}

fn percent_encode_uri_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn codex_app_deep_links(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
) -> Result<[String; 2], String> {
    validate_thread_uuid(thread_id)?;
    if !project_root.is_absolute() || !codex_home.is_absolute() {
        return Err("Project root and Codex home must be absolute".to_string());
    }
    Ok([
        format!(
            "codex://threads/new?path={}",
            percent_encode_uri_component(&project_root.to_string_lossy())
        ),
        format!("codex://threads/{thread_id}"),
    ])
}

fn codex_app_open_command(codex_home: &Path, deep_link: &str) -> String {
    format!(
        "'/usr/bin/open' -n -a '/Applications/ChatGPT.app' --env {} {}",
        shell_quote(&format!("CODEX_HOME={}", codex_home.to_string_lossy())),
        shell_quote(deep_link)
    )
}

/// The new-thread deep link for a project.
pub(crate) fn codex_project_deep_link(project_root: &Path) -> String {
    format!(
        "codex://threads/new?path={}",
        percent_encode_uri_component(&project_root.to_string_lossy())
    )
}

/// The invocation Open-in-Codex performs for a project, rendered as the
/// line this platform runs. Logs quote this so the command they teach is
/// the one that runs. Windows needs the registered handler to render it.
pub fn codex_app_project_open_command(codex_home: &Path, project_root: &Path) -> Option<String> {
    if !codex_home.is_absolute() || !project_root.is_absolute() {
        return None;
    }
    let deep_link = codex_project_deep_link(project_root);
    #[cfg(windows)]
    {
        let executable = crate::providers::windows_app::scheme_handler_executable("codex").ok()?;
        Some(crate::providers::windows_app::display_launch(
            &executable,
            &[("CODEX_HOME", codex_home)],
            &[deep_link.as_str()],
        ))
    }
    #[cfg(not(windows))]
    {
        Some(codex_app_open_command(codex_home, &deep_link))
    }
}

#[cfg(target_os = "macos")]
fn run_codex_app_deep_link(codex_home: &Path, deep_link: &str, target: &str) -> Result<(), String> {
    let environment = format!("CODEX_HOME={}", codex_home.to_string_lossy());
    let output = Command::new("/usr/bin/open")
        .args([
            "-n",
            "-a",
            "/Applications/ChatGPT.app",
            "--env",
            &environment,
            deep_link,
        ])
        .output()
        .map_err(|error| format!("open the Codex desktop app {target}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Codex desktop app could not open the {target}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", windows))]
fn file_stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

#[cfg(any(target_os = "macos", windows))]
fn wait_for_file_change(path: &Path, initial: Option<(u64, SystemTime)>) {
    let started = std::time::Instant::now();
    while started.elapsed() < CODEX_PROJECT_ROUTE_TIMEOUT {
        if file_stamp(path) != initial {
            return;
        }
        std::thread::sleep(CODEX_PROJECT_ROUTE_POLL_INTERVAL);
    }
}

pub(crate) fn launch_codex_app(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let deep_links = codex_app_deep_links(thread_id, project_root, codex_home)?;
        let commands = deep_links
            .each_ref()
            .map(|deep_link| codex_app_open_command(codex_home, deep_link));

        // Existing-thread deep links ignore a workspace path. Send a dedicated
        // new-thread route first, then wait for that profile to persist the
        // project selection before replacing the composer with the target
        // thread.
        let state_path = codex_home.join(".codex-global-state.json");
        let initial_state = file_stamp(&state_path);
        before_launch(&commands[0]);
        run_codex_app_deep_link(codex_home, &deep_links[0], "project")?;
        wait_for_file_change(&state_path, initial_state);

        before_launch(&commands[1]);
        run_codex_app_deep_link(codex_home, &deep_links[1], "thread")?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use crate::providers::windows_app::{display_launch, launch_app, scheme_handler_executable};
        let deep_links = codex_app_deep_links(thread_id, project_root, codex_home)?;
        let executable = scheme_handler_executable("codex")?;
        let env = [("CODEX_HOME", codex_home)];

        // Same two-step route as macOS: the new-thread link selects the
        // project, the thread link then replaces the composer.
        let state_path = codex_home.join(".codex-global-state.json");
        let initial_state = file_stamp(&state_path);
        before_launch(&display_launch(&executable, &env, &[deep_links[0].as_str()]));
        launch_app(&executable, &env, &[deep_links[0].as_str()])?;
        wait_for_file_change(&state_path, initial_state);

        before_launch(&display_launch(&executable, &env, &[deep_links[1].as_str()]));
        launch_app(&executable, &env, &[deep_links[1].as_str()])
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (thread_id, project_root, codex_home, before_launch);
        Err(CODEX_APP_UNAVAILABLE.to_string())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
const CODEX_APP_UNAVAILABLE: &str =
    "Open in Codex is supported only on macOS and Windows; use Open in Terminal";

/// Windows resolves the CLI from PATH (`codex.exe`, or the npm `codex.cmd`
/// launcher unwrapped to its packaged native binary) without a shell.
#[cfg(windows)]
fn resolve_codex_cli() -> Result<PathBuf, String> {
    crate::codex_binary::find_binary("codex")
        .map_err(|_| "Codex CLI is not on PATH; install it or use Open in Codex".to_string())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn resolve_codex_cli() -> Result<PathBuf, String> {
    Err("Open in Terminal is supported only on macOS and Windows".to_string())
}

#[cfg(target_os = "macos")]
fn resolve_codex_cli() -> Result<PathBuf, String> {
    // GUI applications inherit launchd's sparse PATH. A login shell sees the
    // same Homebrew/npm/user configuration as the Terminal window we open.
    let output = Command::new("/bin/zsh")
        .args(["-lc", "command -v codex"])
        .output()
        .map_err(|error| format!("check for Codex CLI in the login shell: {error}"))?;
    if !output.status.success() {
        return Err(
            "Codex CLI is unavailable in the login shell; install it or use Open in Codex"
                .to_string(),
        );
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(value.lines().next().unwrap_or_default().trim());
    if !path.is_absolute() {
        return Err("Codex CLI did not resolve to an absolute executable path".to_string());
    }
    let resolved = std::fs::canonicalize(&path)
        .map_err(|error| format!("resolve Codex CLI '{}': {error}", path.display()))?;
    if !resolved.is_file() {
        return Err(format!(
            "Codex CLI '{}' is not an executable file",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn terminal_resume_command(
    codex_cli: &Path,
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
) -> Result<TerminalCommand, String> {
    validate_thread_uuid(thread_id)?;
    if !codex_cli.is_absolute() || !project_root.is_absolute() || !codex_home.is_absolute() {
        return Err("Codex CLI, project root, and Codex home must be absolute".to_string());
    }
    Ok(TerminalCommand::new(codex_cli.to_string_lossy())
        .env("CODEX_HOME", codex_home)
        .arg("resume")
        .arg(thread_id)
        .arg("-C")
        .arg(project_root.to_string_lossy()))
}

pub(crate) fn launch_terminal_resume(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let codex_cli = resolve_codex_cli()?;
    let command = terminal_resume_command(&codex_cli, thread_id, project_root, codex_home)?;
    before_launch(&command.display_line());
    launch_terminal(&command)
}

/// Open the Codex desktop app reading one profile home, with no thread and
/// no project — the Library's launch. Existing-thread deep links carry no
/// workspace, so the new-thread route on the caller's default workspace is
/// the only home-scoped entry point.
pub fn launch_codex_home_app(codex_home: &Path, workspace: &Path) -> Result<(), String> {
    if !codex_home.is_absolute() || !workspace.is_absolute() {
        return Err("Codex home and workspace must be absolute".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let deep_link = format!(
            "codex://threads/new?path={}",
            percent_encode_uri_component(&workspace.to_string_lossy())
        );
        run_codex_app_deep_link(codex_home, &deep_link, "workspace")
    }
    #[cfg(windows)]
    {
        use crate::providers::windows_app::{launch_app, scheme_handler_executable};
        let deep_link = codex_project_deep_link(workspace);
        let executable = scheme_handler_executable("codex")?;
        launch_app(&executable, &[("CODEX_HOME", codex_home)], &[deep_link.as_str()])
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err(CODEX_APP_UNAVAILABLE.to_string())
    }
}

/// The Terminal command a Library profile launch runs: the CLI reading this
/// home, no thread and no `-C` pin — Terminal's own default directory is the
/// working directory.
fn codex_home_terminal_command(
    codex_cli: &Path,
    codex_home: &Path,
) -> Result<TerminalCommand, String> {
    if !codex_cli.is_absolute() || !codex_home.is_absolute() {
        return Err("Codex CLI and Codex home must be absolute".to_string());
    }
    Ok(TerminalCommand::new(codex_cli.to_string_lossy()).env("CODEX_HOME", codex_home))
}

/// Open a terminal running the Codex CLI against one profile home.
pub fn launch_codex_home_terminal(codex_home: &Path) -> Result<(), String> {
    let codex_cli = resolve_codex_cli()?;
    let command = codex_home_terminal_command(&codex_cli, codex_home)?;
    launch_terminal(&command)
}

pub(super) fn available_actions(
    context: &SessionActionContext<'_>,
) -> Vec<SessionActionDescriptor> {
    let local_uuid = context.is_local && validate_thread_uuid(context.provider_session_id).is_ok();
    let not_local = (!local_uuid).then(|| "This session is not materialized locally.".to_string());
    vec![
        SessionActionDescriptor {
            action_id: OPEN_IN_APP.to_string(),
            kind: SessionActionKind::OpenInApp,
            label: "Open in Codex".to_string(),
            enabled: local_uuid,
            disabled_reason: not_local.clone(),
        },
        SessionActionDescriptor {
            action_id: OPEN_IN_TERMINAL.to_string(),
            kind: SessionActionKind::OpenInTerminal,
            label: "Open in Terminal".to_string(),
            enabled: local_uuid,
            disabled_reason: not_local.clone(),
        },
    ]
}

pub(super) fn execute_action(
    request: &mut SessionActionRequest<'_>,
) -> Result<SessionActionOutcome, String> {
    match request.action_id {
        OPEN_IN_APP => {
            launch_codex_app(
                request.provider_session_id,
                request.project_root,
                request.provider_home,
                request.before_launch,
            )?;
            Ok(SessionActionOutcome::Launched)
        }
        OPEN_IN_TERMINAL => {
            launch_terminal_resume(
                request.provider_session_id,
                request.project_root,
                request.provider_home,
                request.before_launch,
            )?;
            Ok(SessionActionOutcome::Launched)
        }
        other => Err(format!("unknown Codex session action '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_links_and_commands_are_argument_safe() {
        let links = codex_app_deep_links(
            "0193b6c1-5352-7b12-9c6e-000000000001",
            Path::new("/Users/dev/my project"),
            Path::new("/Users/dev/.codex"),
        )
        .unwrap();
        assert!(links[0].contains("%2FUsers%2Fdev%2Fmy%20project"));
        assert_eq!(
            links[1],
            "codex://threads/0193b6c1-5352-7b12-9c6e-000000000001"
        );
        assert!(
            codex_app_deep_links("../../etc/passwd", Path::new("/p"), Path::new("/h")).is_err()
        );

        let command = terminal_resume_command(
            Path::new("/usr/local/bin/codex"),
            "0193b6c1-5352-7b12-9c6e-000000000001",
            Path::new("/Users/dev/my project"),
            Path::new("/Users/dev/.codex"),
        )
        .unwrap();
        assert_eq!(
            command.shell_line(),
            "CODEX_HOME='/Users/dev/.codex' '/usr/local/bin/codex' resume \
             0193b6c1-5352-7b12-9c6e-000000000001 -C '/Users/dev/my project'"
        );
    }

    #[test]
    fn home_terminal_command_carries_only_the_home_and_cli() {
        let command = codex_home_terminal_command(
            Path::new("/usr/local/bin/codex"),
            Path::new("/Users/dev/my codex's home"),
        )
        .unwrap();
        assert_eq!(
            command.shell_line(),
            "CODEX_HOME='/Users/dev/my codex'\\''s home' '/usr/local/bin/codex'"
        );
        assert!(command.args.is_empty());
        assert!(
            codex_home_terminal_command(Path::new("codex"), Path::new("/Users/dev/.codex"))
                .is_err()
        );
        assert!(launch_codex_home_app(Path::new("relative"), Path::new("/Users/dev")).is_err());
    }

    #[test]
    fn available_actions_only_expose_safe_launches() {
        let provider_session_id = "0193b6c1-5352-7b12-9c6e-000000000001";
        let actions = available_actions(&SessionActionContext {
            provider_session_id,
            is_local: true,
        });
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].kind, SessionActionKind::OpenInApp);
        assert_eq!(actions[1].kind, SessionActionKind::OpenInTerminal);
        assert!(actions.iter().all(|action| action.enabled));
    }
}
