//! Safe Claude session actions surfaced through the generic action contract.

#[cfg(target_os = "macos")]
use crate::providers::launch::shell_quote;
use crate::providers::launch::{launch_terminal, percent_encode_uri_component, TerminalCommand};
use crate::providers::traits::{
    NewSessionRequest, NewSessionSurface, SessionActionContext, SessionActionDescriptor,
    SessionActionKind, SessionActionOutcome, SessionActionRequest,
};
use std::path::Path;
#[cfg(any(target_os = "macos", test))]
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

/// Let Launch Services reuse Claude's running instance. `--env` selects the
/// home only on a cold start; an existing instance keeps its own environment.
#[cfg(any(target_os = "macos", test))]
fn claude_app_open_command(claude_home: &Path, deep_link: Option<&str>) -> Command {
    let mut command = Command::new("/usr/bin/open");
    command
        .args(["-a", "/Applications/Claude.app", "--env"])
        .arg(format!("CLAUDE_CONFIG_DIR={}", claude_home.to_string_lossy()));
    if let Some(deep_link) = deep_link {
        command.arg(deep_link);
    }
    command
}

fn launch_claude_app(
    session_id: &str,
    claude_home: &Path,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let deep_link = claude_app_deep_link(session_id)?;
    open_app_link(claude_home, &deep_link, "session", before_launch)
}

/// The desktop app's link for a new Claude Code session rooted at a folder.
/// The app asks the person to confirm a folder that arrives in a link.
fn claude_code_new_deep_link(project_root: &Path) -> String {
    format!(
        "claude://code/new?folder={}",
        percent_encode_uri_component(&dunce::simplified(project_root).to_string_lossy())
    )
}

/// Hand one `claude://` link to the desktop app, reusing it on macOS.
/// `claude_home` selects the profile when starting the app.
/// `target` names what the link opens, for the error.
fn open_app_link(
    claude_home: &Path,
    deep_link: &str,
    target: &str,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if !claude_home.is_absolute() {
        return Err("Claude home must be absolute".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = claude_app_open_command(claude_home, Some(deep_link));
        let display = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| shell_quote(&arg.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");
        before_launch(&display);
        let output = command
            .output()
            .map_err(|error| format!("open the Claude desktop app {target}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Claude desktop app could not open the {target}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use crate::providers::windows_app::{display_launch, launch_app, scheme_handler};
        let _ = target;
        let app = scheme_handler("claude")?;
        let env = [("CLAUDE_CONFIG_DIR", claude_home)];
        before_launch(&display_launch(&app, &env, &[deep_link]));
        launch_app(&app, &env, &[deep_link])
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (deep_link, target, before_launch);
        Err(CLAUDE_APP_UNAVAILABLE.to_string())
    }
}

/// Open Claude without a session for the Library. On macOS this focuses the
/// running app, or starts it with the selected profile home if it is closed.
pub fn launch_claude_home_app(claude_home: &Path) -> Result<(), String> {
    if !claude_home.is_absolute() {
        return Err("Claude home must be absolute".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let output = claude_app_open_command(claude_home, None)
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
        use crate::providers::windows_app::{launch_app, scheme_handler};
        let app = scheme_handler("claude")?;
        launch_app(&app, &[("CLAUDE_CONFIG_DIR", claude_home)], &[])
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

/// The Terminal line a new session in a project runs: the CLI reading the
/// bound home, started in the checkout. The after-Pull hint prints this line.
pub(super) fn new_session_terminal_command(
    claude_cli: &str,
    claude_home: &Path,
    project_root: &Path,
) -> TerminalCommand {
    TerminalCommand::new(claude_cli)
        .cwd(project_root)
        .env("CLAUDE_CONFIG_DIR", claude_home)
}

/// Start a new Claude session in the project: a Claude Code session in the
/// desktop app rooted at the checkout, or the CLI in a new terminal window.
pub(super) fn start_session(request: &mut NewSessionRequest<'_>) -> Result<(), String> {
    if !request.provider_home.is_absolute() || !request.project_root.is_absolute() {
        return Err("Claude home and project root must be absolute".to_string());
    }
    match request.surface {
        NewSessionSurface::App => open_app_link(
            request.provider_home,
            &claude_code_new_deep_link(request.project_root),
            "new session",
            request.before_launch,
        ),
        NewSessionSurface::Terminal => {
            let cli = super::cli::resolve(request.cli_path)?;
            let command = new_session_terminal_command(
                &cli.executable.to_string_lossy(),
                request.provider_home,
                request.project_root,
            );
            (request.before_launch)(&command.display_line());
            launch_terminal(&command)
        }
    }
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
    fn app_deep_links_are_argument_safe() {
        let session_id = "0193b6c1-5352-7b12-9c6e-000000000001";
        let deep_link = claude_app_deep_link(session_id).unwrap();
        assert_eq!(deep_link, format!("claude://resume?session={session_id}"));
        assert!(claude_app_deep_link("../../etc/passwd").is_err());
    }

    #[test]
    fn macos_app_launches_reuse_claude_for_resumes_new_sessions_and_library() {
        let resume = claude_app_deep_link("0193b6c1-5352-7b12-9c6e-000000000001").unwrap();
        let new_session = claude_code_new_deep_link(Path::new("/Users/dev/my project"));
        let home = Path::new("/Users/dev/my claude's home");

        for deep_link in [Some(resume.as_str()), Some(new_session.as_str()), None] {
            let command = claude_app_open_command(home, deep_link);
            assert_eq!(command.get_program(), "/usr/bin/open");
            let args: Vec<_> = command.get_args().collect();
            assert!(
                !args.iter().any(|arg| *arg == "-n" || *arg == "--new"),
                "opening Claude must reuse its running instance"
            );
            // The selected home still reaches a cold launch as one literal
            // argument, including spaces and quotes, without a shell.
            assert_eq!(args[0], "-a");
            assert_eq!(args[1], "/Applications/Claude.app");
            assert_eq!(args[2], "--env");
            assert_eq!(args[3], "CLAUDE_CONFIG_DIR=/Users/dev/my claude's home");
            if let Some(deep_link) = deep_link {
                assert_eq!(args.len(), 5);
                assert_eq!(args[4], deep_link);
            } else {
                assert_eq!(args.len(), 4);
            }
        }
    }

    #[test]
    fn home_launches_pin_the_config_dir_and_nothing_else() {
        let (cli, home, project) = if cfg!(windows) {
            (
                r"C:\bin\claude.exe",
                r"C:\Users\dev\my claude",
                r"C:\Users\dev\my project",
            )
        } else {
            (
                "/Users/dev/.local/bin/claude",
                "/Users/dev/my claude",
                "/Users/dev/my project",
            )
        };
        let cli = Path::new(cli);
        let command = claude_home_terminal_command(cli, Path::new(home)).unwrap();
        assert_eq!(
            command.display_line(),
            if cfg!(windows) {
                r"$env:CLAUDE_CONFIG_DIR = 'C:\Users\dev\my claude'; & 'C:\bin\claude.exe'"
            } else {
                "CLAUDE_CONFIG_DIR='/Users/dev/my claude' '/Users/dev/.local/bin/claude'"
            }
        );
        assert!(command.args.is_empty());
        assert!(command.cwd.is_none());
        let resume = claude_resume_terminal_command(
            cli,
            "0193b6c1-5352-7b12-9c6e-000000000001",
            Path::new(project),
            Path::new(home),
        )
        .unwrap();
        assert_eq!(
            resume.display_line(),
            if cfg!(windows) {
                r"Set-Location -LiteralPath 'C:\Users\dev\my project' -ErrorAction Stop; $env:CLAUDE_CONFIG_DIR = 'C:\Users\dev\my claude'; & 'C:\bin\claude.exe' --resume 0193b6c1-5352-7b12-9c6e-000000000001"
            } else {
                "cd '/Users/dev/my project' && CLAUDE_CONFIG_DIR='/Users/dev/my claude' \
                 '/Users/dev/.local/bin/claude' --resume 0193b6c1-5352-7b12-9c6e-000000000001"
            }
        );
        // The executable is the resolved one, never a bare name the terminal
        // would look up on its own.
        assert!(claude_home_terminal_command(Path::new("claude"), Path::new(home)).is_err());
        assert!(claude_home_terminal_command(cli, Path::new("relative")).is_err());
        assert!(launch_claude_home_app(Path::new("relative")).is_err());
    }

    #[test]
    fn new_session_starts_in_the_checkout_on_the_bound_home() {
        let command = new_session_terminal_command(
            "/usr/local/bin/claude",
            Path::new("/Users/dev/my claude's home"),
            Path::new("/Users/dev/my project"),
        );
        assert_eq!(
            command.shell_line(),
            "cd '/Users/dev/my project' && CLAUDE_CONFIG_DIR='/Users/dev/my claude'\\''s home' \
             '/usr/local/bin/claude'"
        );
        assert!(command.args.is_empty());
        assert_eq!(
            claude_code_new_deep_link(Path::new("/Users/dev/测试 文件")),
            "claude://code/new?folder=%2FUsers%2Fdev%2F%E6%B5%8B%E8%AF%95%20%E6%96%87%E4%BB%B6"
        );
        #[cfg(windows)]
        assert_eq!(
            claude_code_new_deep_link(Path::new(r"\\?\D:\Projects\测试 文件")),
            "claude://code/new?folder=D%3A%5CProjects%5C%E6%B5%8B%E8%AF%95%20%E6%96%87%E4%BB%B6"
        );

        let mut refuse = |command: &str| panic!("a refused start launched {command}");
        for surface in [NewSessionSurface::App, NewSessionSurface::Terminal] {
            assert!(start_session(&mut NewSessionRequest {
                surface,
                provider_home: Path::new("/Users/dev/.claude"),
                project_root: Path::new("relative/project"),
                cli_path: None,
                before_launch: &mut refuse,
            })
            .is_err());
        }
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
