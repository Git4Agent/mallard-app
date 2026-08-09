//! Safe Codex session actions: desktop-app deep links, Terminal resume, and
//! the native fork entry surfaced through the generic action contract.

use crate::providers::launch::shell_quote;
use crate::providers::traits::{
    SessionActionContext, SessionActionDescriptor, SessionActionKind, SessionActionOutcome,
    SessionActionRequest, SessionActionSurface,
};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::{Duration, SystemTime};

pub(super) const OPEN_IN_APP: &str = "codex.open-in-app";
pub(super) const OPEN_IN_TERMINAL: &str = "codex.open-in-terminal";
pub const FORK_SESSION: &str = "codex.fork-session";

#[cfg(target_os = "macos")]
const CODEX_PROJECT_ROUTE_TIMEOUT: Duration = Duration::from_millis(2500);
#[cfg(target_os = "macos")]
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

/// The invocation Open-in-Codex performs for a project, rendered as a shell
/// line. Logs quote this so the command they teach is the one that runs.
pub fn codex_app_project_open_command(codex_home: &Path, project_root: &Path) -> Option<String> {
    if !codex_home.is_absolute() || !project_root.is_absolute() {
        return None;
    }
    let deep_link = format!(
        "codex://threads/new?path={}",
        percent_encode_uri_component(&project_root.to_string_lossy())
    );
    Some(codex_app_open_command(codex_home, &deep_link))
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

#[cfg(target_os = "macos")]
fn file_stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

#[cfg(target_os = "macos")]
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
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (thread_id, project_root, codex_home, before_launch);
        Err("Open in Codex is currently supported only on macOS".to_string())
    }
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
) -> Result<String, String> {
    validate_thread_uuid(thread_id)?;
    if !codex_cli.is_absolute() || !project_root.is_absolute() || !codex_home.is_absolute() {
        return Err("Codex CLI, project root, and Codex home must be absolute".to_string());
    }
    Ok(format!(
        "CODEX_HOME={} {} resume {} -C {}",
        shell_quote(&codex_home.to_string_lossy()),
        shell_quote(&codex_cli.to_string_lossy()),
        shell_quote(thread_id),
        shell_quote(&project_root.to_string_lossy())
    ))
}

pub(crate) fn launch_terminal_resume(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let codex_cli = resolve_codex_cli()?;
        let command = terminal_resume_command(&codex_cli, thread_id, project_root, codex_home)?;
        before_launch(&command);
        crate::providers::launch::launch_terminal_command(&command)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (thread_id, project_root, codex_home, before_launch);
        let _ = terminal_resume_command;
        Err("Open in Terminal is currently supported only on macOS".to_string())
    }
}

pub(super) fn available_actions(
    context: &SessionActionContext<'_>,
) -> Vec<SessionActionDescriptor> {
    let local_uuid = context.is_local && validate_thread_uuid(context.provider_session_id).is_ok();
    let not_local = (!local_uuid).then(|| "This session is not materialized locally.".to_string());
    let mut actions = vec![
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
    ];
    if context.surface == SessionActionSurface::Activity {
        actions.push(SessionActionDescriptor {
            action_id: FORK_SESSION.to_string(),
            kind: SessionActionKind::ForkSession,
            label: "Fork in Codex".to_string(),
            enabled: local_uuid,
            disabled_reason: not_local,
        });
    }
    actions
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
        assert!(command.contains("'/Users/dev/my project'"));
    }

    #[test]
    fn fork_is_available_only_on_activity() {
        let provider_session_id = "0193b6c1-5352-7b12-9c6e-000000000001";
        let activity = available_actions(&SessionActionContext {
            provider_session_id,
            is_local: true,
            transcript_path: None,
            surface: SessionActionSurface::Activity,
        });
        let detail = available_actions(&SessionActionContext {
            provider_session_id,
            is_local: true,
            transcript_path: None,
            surface: SessionActionSurface::Detail,
        });

        assert!(activity.iter().any(|action| {
            action.action_id == FORK_SESSION
                && action.kind == SessionActionKind::ForkSession
                && action.label == "Fork in Codex"
                && action.enabled
        }));
        assert!(detail
            .iter()
            .all(|action| action.kind != SessionActionKind::ForkSession));
    }
}
