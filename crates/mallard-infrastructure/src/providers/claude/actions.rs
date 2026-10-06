//! Safe Claude session actions surfaced through the generic action contract.

use crate::providers::launch::{launch_terminal, shell_quote, TerminalCommand};
use crate::providers::traits::{
    SessionActionContext, SessionActionDescriptor, SessionActionKind, SessionActionOutcome,
    SessionActionRequest,
};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;

pub(super) const OPEN_IN_APP: &str = "claude.open-in-app";
pub(super) const OPEN_IN_TERMINAL: &str = "claude.open-in-terminal";

fn validated_session_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Claude session id is not a safe resume argument".to_string());
    }
    Ok(())
}

fn validated_app_session_id(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    valid
        .then_some(())
        .ok_or_else(|| "Claude app session id must be a UUID".to_string())
}

fn claude_app_deep_link(session_id: &str) -> Result<String, String> {
    validated_app_session_id(session_id)?;
    Ok(format!("claude://resume?session={session_id}"))
}

fn claude_app_open_command(claude_home: &Path, deep_link: &str) -> String {
    format!(
        "'/usr/bin/open' -n -a '/Applications/Claude.app' --env {} {}",
        shell_quote(&format!(
            "CLAUDE_CONFIG_DIR={}",
            claude_home.to_string_lossy()
        )),
        shell_quote(deep_link)
    )
}

fn launch_claude_app(
    session_id: &str,
    claude_home: &Path,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if !claude_home.is_absolute() {
            return Err("Claude home must be absolute".to_string());
        }
        let deep_link = claude_app_deep_link(session_id)?;
        let command = claude_app_open_command(claude_home, &deep_link);
        let environment = format!("CLAUDE_CONFIG_DIR={}", claude_home.to_string_lossy());
        before_launch(&command);
        let output = Command::new("/usr/bin/open")
            .args([
                "-n",
                "-a",
                "/Applications/Claude.app",
                "--env",
                &environment,
                &deep_link,
            ])
            .output()
            .map_err(|error| format!("open the Claude desktop app session: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Claude desktop app could not open the session: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use crate::providers::windows_app::{display_launch, launch_app, scheme_handler_executable};
        if !claude_home.is_absolute() {
            return Err("Claude home must be absolute".to_string());
        }
        let deep_link = claude_app_deep_link(session_id)?;
        let executable = scheme_handler_executable("claude")?;
        let env = [("CLAUDE_CONFIG_DIR", claude_home)];
        before_launch(&display_launch(&executable, &env, &[deep_link.as_str()]));
        launch_app(&executable, &env, &[deep_link.as_str()])
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (session_id, claude_home, before_launch);
        Err(CLAUDE_APP_UNAVAILABLE.to_string())
    }
}

/// Open the Claude desktop app reading one profile home, with no session —
/// the Library's launch. Claude's only deep link resumes a session, so a
/// home-scoped open is a plain app launch with the config environment.
pub fn launch_claude_home_app(claude_home: &Path) -> Result<(), String> {
    if !claude_home.is_absolute() {
        return Err("Claude home must be absolute".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let environment = format!("CLAUDE_CONFIG_DIR={}", claude_home.to_string_lossy());
        let output = Command::new("/usr/bin/open")
            .args([
                "-n",
                "-a",
                "/Applications/Claude.app",
                "--env",
                &environment,
            ])
            .output()
            .map_err(|error| format!("open the Claude desktop app: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Claude desktop app could not open: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use crate::providers::windows_app::{launch_app, scheme_handler_executable};
        let executable = scheme_handler_executable("claude")?;
        launch_app(&executable, &[("CLAUDE_CONFIG_DIR", claude_home)], &[])
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err(CLAUDE_APP_UNAVAILABLE.to_string())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
const CLAUDE_APP_UNAVAILABLE: &str =
    "Open in Claude is supported only on macOS and Windows; use Open in Terminal";

/// The Terminal command a Library profile launch runs: the CLI reading this
/// home, no session and no `cd` — Terminal's own default directory is the
/// working directory.
fn claude_home_terminal_command(
    claude_cli: &Path,
    claude_home: &Path,
) -> Result<TerminalCommand, String> {
    if !claude_cli.is_absolute() || !claude_home.is_absolute() {
        return Err("Claude CLI and Claude home must be absolute".to_string());
    }
    Ok(TerminalCommand::new(claude_cli.to_string_lossy()).env("CLAUDE_CONFIG_DIR", claude_home))
}

/// Open a terminal running the Claude CLI against one profile home.
/// `cli_path` is the executable saved in Settings, if any.
pub fn launch_claude_home_terminal(
    claude_home: &Path,
    cli_path: Option<&Path>,
) -> Result<(), String> {
    let cli = super::cli::resolve(cli_path)?;
    let command = claude_home_terminal_command(&cli.executable, claude_home)?;
    launch_terminal(&command)
}

fn claude_resume_terminal_command(
    claude_cli: &Path,
    session_id: &str,
    project_root: &Path,
    claude_home: &Path,
) -> Result<TerminalCommand, String> {
    validated_session_id(session_id)?;
    if !claude_cli.is_absolute() || !project_root.is_absolute() || !claude_home.is_absolute() {
        return Err("Claude CLI, project root, and Claude home must be absolute".to_string());
    }
    Ok(TerminalCommand::new(claude_cli.to_string_lossy())
        .cwd(project_root)
        .env("CLAUDE_CONFIG_DIR", claude_home)
        .arg("--resume")
        .arg(session_id))
}

pub(super) fn available_actions(
    context: &SessionActionContext<'_>,
) -> Vec<SessionActionDescriptor> {
    let resume_ready =
        context.is_local && validated_session_id(context.provider_session_id).is_ok();
    let app_ready =
        context.is_local && validated_app_session_id(context.provider_session_id).is_ok();
    vec![
        SessionActionDescriptor {
            action_id: OPEN_IN_APP.to_string(),
            kind: SessionActionKind::OpenInApp,
            label: "Open in Claude".to_string(),
            enabled: app_ready,
            disabled_reason: (!app_ready)
                .then(|| "This session is not materialized locally.".to_string()),
        },
        SessionActionDescriptor {
            action_id: OPEN_IN_TERMINAL.to_string(),
            kind: SessionActionKind::OpenInTerminal,
            label: "Open in Terminal".to_string(),
            enabled: resume_ready,
            disabled_reason: (!resume_ready)
                .then(|| "This session is not materialized locally.".to_string()),
        },
    ]
}

pub(super) fn execute_action(
    request: &mut SessionActionRequest<'_>,
) -> Result<SessionActionOutcome, String> {
    match request.action_id {
        OPEN_IN_APP => {
            launch_claude_app(
                request.provider_session_id,
                request.provider_home,
                request.before_launch,
            )?;
            Ok(SessionActionOutcome::Launched)
        }
        OPEN_IN_TERMINAL => {
            let cli = super::cli::resolve(request.cli_path)?;
            let command = claude_resume_terminal_command(
                &cli.executable,
                request.provider_session_id,
                request.project_root,
                request.provider_home,
            )?;
            (request.before_launch)(&command.display_line());
            launch_terminal(&command)?;
            Ok(SessionActionOutcome::Launched)
        }
        other => Err(format!("unknown Claude session action '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_deep_links_and_commands_are_argument_safe() {
        let session_id = "0193b6c1-5352-7b12-9c6e-000000000001";
        let deep_link = claude_app_deep_link(session_id).unwrap();
        assert_eq!(deep_link, format!("claude://resume?session={session_id}"));
        assert!(claude_app_deep_link("../../etc/passwd").is_err());

        let command = claude_app_open_command(Path::new("/Users/dev/my claude"), &deep_link);
        assert!(command.contains("'CLAUDE_CONFIG_DIR=/Users/dev/my claude'"));
        assert!(command.contains("'/Applications/Claude.app'"));
    }

    #[test]
    fn home_launches_pin_the_config_dir_and_nothing_else() {
        let cli = Path::new("/Users/dev/.local/bin/claude");
        let command = claude_home_terminal_command(cli, Path::new("/Users/dev/my claude")).unwrap();
        assert_eq!(
            command.shell_line(),
            "CLAUDE_CONFIG_DIR='/Users/dev/my claude' '/Users/dev/.local/bin/claude'"
        );
        assert!(command.args.is_empty());
        assert!(command.cwd.is_none());
        let resume = claude_resume_terminal_command(
            cli,
            "0193b6c1-5352-7b12-9c6e-000000000001",
            Path::new("/Users/dev/my project"),
            Path::new("/Users/dev/.claude"),
        )
        .unwrap();
        assert_eq!(
            resume.shell_line(),
            "cd '/Users/dev/my project' && CLAUDE_CONFIG_DIR='/Users/dev/.claude' \
             '/Users/dev/.local/bin/claude' --resume 0193b6c1-5352-7b12-9c6e-000000000001"
        );
        // The executable is the resolved one, never a bare name the terminal
        // would look up on its own.
        assert!(
            claude_home_terminal_command(Path::new("claude"), Path::new("/Users/dev")).is_err()
        );
        assert!(claude_home_terminal_command(cli, Path::new("relative")).is_err());
        assert!(launch_claude_home_app(Path::new("relative")).is_err());
    }

    #[test]
    fn available_actions_include_the_claude_app() {
        let actions = available_actions(&SessionActionContext {
            provider_session_id: "0193b6c1-5352-7b12-9c6e-000000000001",
            is_local: true,
        });
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].kind, SessionActionKind::OpenInApp);
        assert_eq!(actions[0].label, "Open in Claude");
        assert_eq!(actions[1].kind, SessionActionKind::OpenInTerminal);
        assert!(actions.iter().all(|action| action.enabled));
    }
}
