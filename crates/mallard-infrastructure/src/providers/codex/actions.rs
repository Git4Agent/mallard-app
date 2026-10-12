//! Safe Codex session actions: desktop-app deep links and Terminal resume.

use super::cli_version::{require_session_resume, ResumeCli};
use crate::cli_discovery::ResolvedCli;
#[cfg(target_os = "macos")]
use crate::codex_app::{find_installed_mac_app, InstalledMacApp};
#[cfg(target_os = "macos")]
use crate::providers::launch::shell_quote;
use crate::providers::launch::{launch_terminal, percent_encode_uri_component, TerminalCommand};
use crate::providers::traits::{
    NewSessionRequest, NewSessionSurface, SessionActionContext, SessionActionDescriptor,
    SessionActionKind, SessionActionOutcome, SessionActionRequest,
};
use std::ffi::OsStr;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;

pub(super) const OPEN_IN_APP: &str = "codex.open-in-app";
pub(super) const OPEN_IN_TERMINAL: &str = "codex.open-in-terminal";

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
        codex_project_deep_link(project_root),
        format!("codex://threads/{thread_id}"),
    ])
}

#[cfg(target_os = "macos")]
fn codex_app_open_command(bundle_path: &Path, codex_home: &Path, deep_link: &str) -> String {
    format!(
        "'/usr/bin/open' -n -a {} --env {} {}",
        shell_quote(&bundle_path.to_string_lossy()),
        shell_quote(&format!("CODEX_HOME={}", codex_home.to_string_lossy())),
        shell_quote(deep_link)
    )
}

/// The new-thread deep link for a project.
pub(crate) fn codex_project_deep_link(project_root: &Path) -> String {
    // Codex rejects the verbatim Windows prefix added by fs::canonicalize.
    let project_root = dunce::simplified(project_root);
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
    #[cfg(any(target_os = "macos", windows))]
    let deep_link = codex_project_deep_link(project_root);
    #[cfg(windows)]
    {
        let app = crate::providers::windows_app::scheme_handler("codex").ok()?;
        Some(crate::providers::windows_app::display_launch(
            &app,
            &[("CODEX_HOME", codex_home)],
            &[deep_link.as_str()],
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let app = find_installed_mac_app()?;
        Some(codex_app_open_command(
            &app.bundle_path,
            codex_home,
            &deep_link,
        ))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn run_codex_app_deep_link(
    app: &InstalledMacApp,
    codex_home: &Path,
    deep_link: &str,
    target: &str,
) -> Result<(), String> {
    let environment = format!("CODEX_HOME={}", codex_home.to_string_lossy());
    let output = Command::new("/usr/bin/open")
        .args(["-n", "-a"])
        .arg(&app.bundle_path)
        .args(["--env", &environment, deep_link])
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

pub(crate) fn launch_codex_app(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
    cli_path: Option<&Path>,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = cli_path;
        let deep_links = codex_app_deep_links(thread_id, project_root, codex_home)?;
        let app = find_installed_mac_app().ok_or_else(|| {
            "the official Codex desktop app is not installed or its bundle identity is invalid"
                .to_string()
        })?;
        open_session_in_mac_app(
            &app,
            &deep_links,
            codex_home,
            project_root,
            before_launch,
            run_codex_app_deep_link,
            || super::project_assignment::macos::prepare(&app, codex_home),
            |existing_helpers| {
                super::project_assignment::macos::assign_and_open(
                    codex_home,
                    thread_id,
                    project_root,
                    existing_helpers,
                )
            },
        )
    }
    #[cfg(windows)]
    {
        use crate::providers::windows_app::{display_launch, launch_app, scheme_handler};
        let _ = cli_path;
        codex_app_deep_links(thread_id, project_root, codex_home)?;
        let app = scheme_handler("codex")?;
        let env = [("CODEX_HOME", codex_home)];

        // A running Desktop can ignore the launch environment. Start/focus it
        // without a route, then verify its actual profile before any move or
        // navigation. Both operations use that same native connection.
        before_launch(&display_launch(&app, &env, &[]));
        launch_app(&app, &env, &[])?;
        super::project_assignment::assign_and_open(
            &app.executable,
            codex_home,
            thread_id,
            project_root,
            || {
                let link = codex_project_deep_link(project_root);
                before_launch(&display_launch(&app, &env, &[&link]));
                launch_app(&app, &env, &[&link])
            },
        )
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (thread_id, project_root, codex_home, cli_path, before_launch);
        Err(CODEX_APP_UNAVAILABLE.to_string())
    }
}

/// The macOS half of Open in Codex once the app is found: refuse an app whose
/// bundled CLI cannot resume the session, then register the native helper,
/// register the folder, and load the chat before Desktop's native move.
#[cfg(target_os = "macos")]
fn open_session_in_mac_app(
    app: &InstalledMacApp,
    deep_links: &[String; 2],
    codex_home: &Path,
    project_root: &Path,
    before_launch: &mut dyn FnMut(&str),
    mut open: impl FnMut(&InstalledMacApp, &Path, &str, &str) -> Result<(), String>,
    prepare: impl FnOnce() -> Result<(), String>,
    assign: impl FnOnce(&[String]) -> Result<(), String>,
) -> Result<(), String> {
    use super::project_assignment::macos;
    require_session_resume(&app.bundled_cli(), None, ResumeCli::DesktopApp)?;
    prepare()?;
    // Desktop keeps only the latest pending route. Wait for folder registration
    // before sending the chat route, which also loads its MCP helper.
    let mut launch = |deep_link: &str, target: &str| {
        before_launch(&codex_app_open_command(
            &app.bundle_path,
            codex_home,
            deep_link,
        ));
        open(app, codex_home, deep_link, target)
    };
    if !macos::project_is_registered(codex_home, project_root)? {
        launch(&deep_links[0], "project")?;
        macos::wait_for_project_registration(
            codex_home,
            project_root,
            std::time::Instant::now() + std::time::Duration::from_secs(30),
        )?;
    }
    // A helper from an earlier window can close as the chat is resumed. Bind
    // placement to a helper started by this session launch.
    let existing_helpers = macos::existing_helpers(codex_home)?;
    launch(&deep_links[1], "thread")?;
    assign(&existing_helpers)
}

#[cfg(not(any(target_os = "macos", windows)))]
const CODEX_APP_UNAVAILABLE: &str =
    "Open in Codex is supported only on macOS and Windows; use Open in Terminal";

/// Use the same CLI selection as provider maintenance and plugin commands.
#[cfg(any(target_os = "macos", windows))]
fn resolve_codex_cli(cli_path: Option<&Path>) -> Result<ResolvedCli, String> {
    super::cli::resolve(cli_path)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn resolve_codex_cli(_cli_path: Option<&Path>) -> Result<ResolvedCli, String> {
    Err("Open in Terminal is supported only on macOS and Windows".to_string())
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
    cli_path: Option<&Path>,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let codex_cli = resolve_codex_cli(cli_path)?;
    resume_in_terminal(
        &codex_cli.executable,
        Some(&codex_cli.search_path),
        thread_id,
        project_root,
        codex_home,
        before_launch,
        launch_terminal,
    )
}

/// Refuse a CLI that cannot resume the session, then hand Terminal the
/// command. `launch` is the Terminal launcher, injected so the refusal is
/// testable.
fn resume_in_terminal(
    codex_cli: &Path,
    search_path: Option<&OsStr>,
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
    before_launch: &mut dyn FnMut(&str),
    launch: impl FnOnce(&TerminalCommand) -> Result<(), String>,
) -> Result<(), String> {
    let command = terminal_resume_command(codex_cli, thread_id, project_root, codex_home)?;
    require_session_resume(codex_cli, search_path, ResumeCli::Terminal)?;
    before_launch(&command.display_line());
    launch(&command)
}

/// Open the Codex desktop app reading one profile home, with no thread and
/// no project — the Library's launch. Existing-thread deep links carry no
/// workspace, so the new-thread route on the caller's default workspace is
/// the only home-scoped entry point.
pub fn launch_codex_home_app(codex_home: &Path, workspace: &Path) -> Result<(), String> {
    open_app_new_thread(codex_home, workspace, &mut |_| {})
}

/// Open the Codex desktop app on a new thread in `workspace`, reading
/// `codex_home`. Shared by the Library's launch and a project's new session.
fn open_app_new_thread(
    codex_home: &Path,
    workspace: &Path,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if !codex_home.is_absolute() || !workspace.is_absolute() {
        return Err("Codex home and workspace must be absolute".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let app = find_installed_mac_app().ok_or_else(|| {
            "the official Codex desktop app is not installed or its bundle identity is invalid"
                .to_string()
        })?;
        let deep_link = codex_project_deep_link(workspace);
        before_launch(&codex_app_open_command(
            &app.bundle_path,
            codex_home,
            &deep_link,
        ));
        run_codex_app_deep_link(&app, codex_home, &deep_link, "workspace")
    }
    #[cfg(windows)]
    {
        use crate::providers::windows_app::{display_launch, launch_app, scheme_handler};
        let deep_link = codex_project_deep_link(workspace);
        let app = scheme_handler("codex")?;
        let env = [("CODEX_HOME", codex_home)];
        before_launch(&display_launch(&app, &env, &[deep_link.as_str()]));
        launch_app(&app, &env, &[deep_link.as_str()])
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = before_launch;
        Err(CODEX_APP_UNAVAILABLE.to_string())
    }
}

/// The Terminal line a new session in a project runs: the CLI reading the
/// bound home, pinned to the checkout. The after-Pull hint prints this line.
pub(super) fn new_session_terminal_command(
    codex_cli: &str,
    codex_home: &Path,
    project_root: &Path,
) -> TerminalCommand {
    TerminalCommand::new(codex_cli)
        .env("CODEX_HOME", codex_home)
        .arg("-C")
        .arg(project_root.to_string_lossy())
}

/// Start a new Codex session in the project: the desktop app on a new
/// thread there, or the CLI in a new terminal window. A new thread resumes
/// nothing, so the resume version gate does not apply.
pub(super) fn start_session(request: &mut NewSessionRequest<'_>) -> Result<(), String> {
    if !request.provider_home.is_absolute() || !request.project_root.is_absolute() {
        return Err("Codex home and project root must be absolute".to_string());
    }
    match request.surface {
        NewSessionSurface::App => open_app_new_thread(
            request.provider_home,
            request.project_root,
            request.before_launch,
        ),
        NewSessionSurface::Terminal => {
            let cli = resolve_codex_cli(request.cli_path)?;
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
/// `cli_path` is the executable saved in Settings, if any.
pub fn launch_codex_home_terminal(
    codex_home: &Path,
    cli_path: Option<&Path>,
) -> Result<(), String> {
    let codex_cli = resolve_codex_cli(cli_path)?;
    let command = codex_home_terminal_command(&codex_cli.executable, codex_home)?;
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
                request.cli_path,
                request.before_launch,
            )?;
            Ok(SessionActionOutcome::Launched)
        }
        OPEN_IN_TERMINAL => {
            launch_terminal_resume(
                request.provider_session_id,
                request.project_root,
                request.provider_home,
                request.cli_path,
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
    #[cfg(target_os = "macos")]
    fn app_command_uses_the_resolved_bundle_and_quotes_the_profile() {
        assert_eq!(
            codex_app_open_command(
                Path::new("/tmp/owner's/Applications/ChatGPT.app"),
                Path::new("/tmp/my codex's home"),
                "codex://threads/0193b6c1-5352-7b12-9c6e-000000000001",
            ),
            "'/usr/bin/open' -n -a '/tmp/owner'\\''s/Applications/ChatGPT.app' --env \
             'CODEX_HOME=/tmp/my codex'\\''s home' \
             'codex://threads/0193b6c1-5352-7b12-9c6e-000000000001'"
        );
    }

    #[test]
    fn deep_links_and_commands_are_argument_safe() {
        let (cli, project, home) = if cfg!(windows) {
            (
                r"C:\bin\codex.exe",
                r"C:\Users\dev\my project",
                r"C:\Users\dev\.codex",
            )
        } else {
            (
                "/usr/local/bin/codex",
                "/Users/dev/my project",
                "/Users/dev/.codex",
            )
        };
        let links = codex_app_deep_links(
            "0193b6c1-5352-7b12-9c6e-000000000001",
            Path::new(project),
            Path::new(home),
        )
        .unwrap();
        assert!(links[0].contains(if cfg!(windows) {
            "C%3A%5CUsers%5Cdev%5Cmy%20project"
        } else {
            "%2FUsers%2Fdev%2Fmy%20project"
        }));
        assert_eq!(
            links[1],
            "codex://threads/0193b6c1-5352-7b12-9c6e-000000000001"
        );
        assert!(
            codex_app_deep_links("../../etc/passwd", Path::new("/p"), Path::new("/h")).is_err()
        );

        let command = terminal_resume_command(
            Path::new(cli),
            "0193b6c1-5352-7b12-9c6e-000000000001",
            Path::new(project),
            Path::new(home),
        )
        .unwrap();
        assert_eq!(
            command.display_line(),
            if cfg!(windows) {
                r"$env:CODEX_HOME = 'C:\Users\dev\.codex'; & 'C:\bin\codex.exe' resume 0193b6c1-5352-7b12-9c6e-000000000001 -C 'C:\Users\dev\my project'"
            } else {
                "CODEX_HOME='/Users/dev/.codex' '/usr/local/bin/codex' resume \
                 0193b6c1-5352-7b12-9c6e-000000000001 -C '/Users/dev/my project'"
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn app_deep_links_accept_native_canonical_project_paths() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project with spaces & 测试");
        std::fs::create_dir(&project).unwrap();
        let canonical = std::fs::canonicalize(&project).unwrap();
        assert!(canonical.to_string_lossy().starts_with(r"\\?\"));
        let ordinary = crate::local_paths::canonicalize(&project).unwrap();
        let expected = format!(
            "codex://threads/new?path={}",
            percent_encode_uri_component(&ordinary.to_string_lossy())
        );

        assert_eq!(codex_project_deep_link(&canonical), expected);
        let links = codex_app_deep_links(
            "0193b6c1-5352-7b12-9c6e-000000000001",
            &canonical,
            &temp.path().join("selected codex home"),
        )
        .unwrap();
        assert_eq!(links[0], expected);
    }

    #[test]
    fn home_terminal_command_carries_only_the_home_and_cli() {
        let (cli, home) = if cfg!(windows) {
            (r"C:\bin\codex.exe", r"C:\Users\dev\my codex's home")
        } else {
            ("/usr/local/bin/codex", "/Users/dev/my codex's home")
        };
        let command = codex_home_terminal_command(Path::new(cli), Path::new(home)).unwrap();
        assert_eq!(
            command.display_line(),
            if cfg!(windows) {
                r"$env:CODEX_HOME = 'C:\Users\dev\my codex''s home'; & 'C:\bin\codex.exe'"
            } else {
                "CODEX_HOME='/Users/dev/my codex'\\''s home' '/usr/local/bin/codex'"
            }
        );
        assert!(command.args.is_empty());
        assert!(codex_home_terminal_command(Path::new("codex"), Path::new(home)).is_err());
        assert!(launch_codex_home_app(Path::new("relative"), Path::new("/Users/dev")).is_err());
    }

    #[test]
    fn new_session_pins_the_bound_home_and_the_checkout() {
        let command = new_session_terminal_command(
            "/usr/local/bin/codex",
            Path::new("/Users/dev/my codex's home"),
            Path::new("/Users/dev/my project"),
        );
        assert_eq!(
            command.shell_line(),
            "CODEX_HOME='/Users/dev/my codex'\\''s home' '/usr/local/bin/codex' \
             -C '/Users/dev/my project'"
        );
        assert_eq!(
            codex_project_deep_link(Path::new("/Users/dev/测试 文件")),
            "codex://threads/new?path=%2FUsers%2Fdev%2F%E6%B5%8B%E8%AF%95%20%E6%96%87%E4%BB%B6"
        );

        let mut refuse = |command: &str| panic!("a refused start launched {command}");
        for surface in [NewSessionSurface::App, NewSessionSurface::Terminal] {
            assert!(start_session(&mut NewSessionRequest {
                surface,
                provider_home: Path::new("relative/.codex"),
                project_root: Path::new("/Users/dev/project"),
                cli_path: None,
                before_launch: &mut refuse,
            })
            .is_err());
        }
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

    #[cfg(unix)]
    use super::super::cli_version::tests::fake_cli;

    #[cfg(unix)]
    const THREAD: &str = "0193b6c1-5352-7b12-9c6e-000000000001";

    /// A provider home whose every entry is listed, so a test can prove a
    /// refused launch left it exactly as it was.
    #[cfg(unix)]
    fn home_entries(home: &Path) -> Vec<std::path::PathBuf> {
        let mut entries = walkdir::WalkDir::new(home)
            .into_iter()
            .map(|entry| entry.unwrap().into_path())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    #[test]
    #[cfg(unix)]
    fn terminal_resume_refuses_an_old_or_unreadable_cli_before_launching() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        let before = home_entries(&home);
        let cli = temp.path().join("bin/codex");
        for (body, expected) in [
            (
                "mkdir -p \"$CODEX_HOME/tmp/arg0\"; echo 'codex-cli 0.144.0'",
                "Cannot open this session with Codex CLI 0.144.0.",
            ),
            (
                "echo 'codex-cli 0.145.0-alpha.3'",
                "with Codex CLI 0.145.0-alpha.3.",
            ),
            ("exit 1", "Cannot determine the Codex CLI version."),
        ] {
            fake_cli(&cli, body);
            let mut announced = Vec::new();
            let error = resume_in_terminal(
                &cli,
                None,
                THREAD,
                Path::new("/Users/dev/project"),
                &home,
                &mut |line: &str| announced.push(line.to_string()),
                |_| panic!("a refused resume reached the Terminal launcher"),
            )
            .unwrap_err();
            assert!(error.contains(expected), "{body}: {error}");
            assert!(announced.is_empty(), "{body}: before_launch ran");
            assert_eq!(home_entries(&home), before, "{body}: the home changed");
        }
    }

    #[test]
    #[cfg(unix)]
    fn terminal_resume_launches_a_supported_cli() {
        let temp = tempfile::tempdir().unwrap();
        let cli = temp.path().join("bin/codex");
        for version in ["0.145.0", "0.147.0", "0.156.0"] {
            fake_cli(&cli, &format!("echo 'codex-cli {version}'"));
            let mut announced = Vec::new();
            let mut launched = Vec::new();
            resume_in_terminal(
                &cli,
                None,
                THREAD,
                Path::new("/Users/dev/project"),
                Path::new("/Users/dev/.codex"),
                &mut |line: &str| announced.push(line.to_string()),
                |command| {
                    launched.push(command.display_line());
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(announced.len(), 1, "{version}");
            assert_eq!(launched, announced, "{version}");
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn open_in_codex_checks_the_cli_bundled_in_the_app() {
        use crate::codex_app::{InstalledMacApp, OFFICIAL_MAC_APP};
        use std::os::unix::fs::DirBuilderExt;
        let temp = tempfile::tempdir().unwrap();
        let app = InstalledMacApp {
            specification: OFFICIAL_MAC_APP,
            bundle_path: temp.path().join("ChatGPT.app"),
        };
        // Build the shipped layout independently of the resolver so a wrong
        // bundled path cannot also silently change this test's fixture.
        let bundled_cli = app
            .bundle_path
            .join("Contents/Resources/codex-cli/bin/codex");
        let home = temp.path().join("codex-home");
        std::fs::create_dir_all(&home).unwrap();
        let before = home_entries(&home);
        let project = temp.path().join("project");
        let links = codex_app_deep_links(THREAD, &project, &home).unwrap();

        // A newer CLI on PATH does not matter: the app runs its own copy.
        for (body, expected) in [
            (
                Some("echo 'codex-cli 0.144.0'"),
                "its bundled Codex CLI is 0.144.0.",
            ),
            (None, "Cannot determine the Codex CLI version bundled with"),
        ] {
            match body {
                Some(body) => fake_cli(&bundled_cli, body),
                None => {
                    let _ = std::fs::remove_file(&bundled_cli);
                }
            }
            let mut announced = Vec::new();
            let error = open_session_in_mac_app(
                &app,
                &links,
                &home,
                &project,
                &mut |line: &str| announced.push(line.to_string()),
                |_, _, _, _| panic!("a refused session reached `open`"),
                || panic!("a refused session registered a helper"),
                |_| panic!("a refused session changed project membership"),
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(announced.is_empty());
            assert_eq!(home_entries(&home), before);
        }

        fake_cli(&bundled_cli, "echo 'codex-cli 0.155.0-alpha.2.6'");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(home.join(".mallard-project-placement"))
            .unwrap();
        let registry_path = home.join(".codex-global-state.json");
        let registered = serde_json::to_vec(&serde_json::json!({
            "local-projects": {"new": {"rootPaths": [&project]}}
        }))
        .unwrap();
        let mut announced = Vec::new();
        let mut opened = Vec::new();
        let phases = std::cell::RefCell::new(Vec::new());
        open_session_in_mac_app(
            &app,
            &links,
            &home,
            &project,
            &mut |line: &str| announced.push(line.to_string()),
            |_, _, deep_link, target| {
                opened.push(deep_link.to_string());
                phases.borrow_mut().push(target.to_string());
                if target == "project" {
                    std::fs::write(&registry_path, &registered).unwrap();
                } else {
                    assert!(
                        super::super::project_assignment::macos::project_is_registered(
                            &home, &project
                        )
                        .unwrap()
                    );
                }
                Ok(())
            },
            || {
                phases.borrow_mut().push("prepare".into());
                Ok(())
            },
            |_| {
                phases.borrow_mut().push("assign".into());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(opened, links);
        assert_eq!(announced.len(), 2);
        assert_eq!(*phases.borrow(), ["prepare", "project", "thread", "assign"]);

        open_session_in_mac_app(
            &app,
            &links,
            &home,
            &project,
            &mut |_| {},
            |_, _, link, target| {
                assert_eq!(
                    target, "thread",
                    "an existing project needs no folder launch"
                );
                assert_eq!(link, links[1]);
                Ok(())
            },
            || Ok(()),
            |_| Ok(()),
        )
        .unwrap();

        std::fs::remove_file(&registry_path).unwrap();
        let error = open_session_in_mac_app(
            &app,
            &links,
            &home,
            &project,
            &mut |_| {},
            |_, _, link, target| {
                assert_eq!(target, "project");
                assert_eq!(link, links[0]);
                Err("Folder link failed".into())
            },
            || Ok(()),
            |_| panic!("failed registration must not assign the chat"),
        )
        .unwrap_err();
        assert_eq!(error, "Folder link failed");

        std::fs::write(&registry_path, &registered).unwrap();
        let error = open_session_in_mac_app(
            &app,
            &links,
            &home,
            &project,
            &mut |_| {},
            |_, _, _, _| Ok(()),
            || Ok(()),
            |_| Err("Native assignment failed".into()),
        )
        .unwrap_err();
        assert_eq!(error, "Native assignment failed");
    }

    /// The opt-in runner creates a completed local mock chat and an isolated
    /// Desktop. Only the launcher is injected, to keep that Desktop's separate
    /// Electron data directory. Registration and placement use production code.
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires the disposable macOS native placement fixture"]
    fn macos_native_project_placement_uses_the_selected_home() {
        use serde_json::Value;
        use std::process::Stdio;

        let root = std::path::PathBuf::from(
            std::env::var_os("MALLARD_MAC_NATIVE_PLACEMENT_ROOT").expect("fixture root required"),
        );
        assert_eq!(root.parent(), Some(Path::new("/private/tmp")));
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("mallard-mac-placement-"));
        assert_eq!(std::fs::canonicalize(&root).unwrap(), root);
        let state: Value =
            serde_json::from_slice(&std::fs::read(root.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["marker"], "mallard-mac-placement-v1");
        assert_eq!(state["root"], root.to_string_lossy().as_ref());
        let home = root.join("selected-home");
        let project = root.join("Placement fixture");
        let user_data = root.join("desktop-data");
        assert_eq!(state["home"], home.to_string_lossy().as_ref());
        assert_eq!(state["project"], project.to_string_lossy().as_ref());
        assert_eq!(state["userData"], user_data.to_string_lossy().as_ref());
        let thread_id = state["threadId"].as_str().unwrap();
        let app = find_installed_mac_app().expect("official Desktop required");
        if std::env::var_os("MALLARD_MAC_NATIVE_PLACEMENT_PREPARE_ONLY").is_some() {
            super::super::project_assignment::macos::prepare(&app, &home).unwrap();
            return;
        }
        let before =
            super::super::project_assignment::macos::fixture_thread(&home, thread_id, None);
        let project_before = super::super::project_assignment::macos::fixture_project(
            &home, thread_id, &project, false,
        );
        let project_id_path = root.join("registered-project-id");
        let mut project_id = std::fs::read_to_string(&project_id_path).ok();
        if std::env::var("MALLARD_MAC_NATIVE_PLACEMENT_EXPECT_PROJECTLESS").as_deref() == Ok("1") {
            assert!(
                before["projectId"].is_null(),
                "fixture must start projectless"
            );
            assert_eq!(
                project_before, None,
                "fixture folder must not be registered"
            );
            assert_eq!(project_id, None);
        } else {
            assert!(project_id.is_some());
            assert_eq!(project_before, project_id);
            assert_eq!(
                before["projectId"].as_str(),
                project_id.as_deref(),
                "membership must survive restart before another move"
            );
        }
        let links = codex_app_deep_links(thread_id, &project, &home).unwrap();
        let mut launch_number = 0;
        let mut launch = |app: &InstalledMacApp, home: &Path, link: &str, target: &str| {
            // An explicit Electron data path enables Desktop's single-instance
            // mode. Give each launch its own data, matching production `open -n`.
            // The runner closes these fixture instances and their children.
            launch_number += 1;
            let launch_data =
                user_data.join(format!("{target}-{}-{launch_number}", std::process::id()));
            std::fs::create_dir(&launch_data).unwrap();
            let mut command = Command::new(app.bundle_path.join("Contents/MacOS/ChatGPT"));
            for (key, _) in std::env::vars_os() {
                let name = key.to_string_lossy();
                if [
                    "CODEX_",
                    "OPENAI_",
                    "ANTHROPIC_",
                    "CLAUDE_",
                    "CHATGPT_",
                    "ELECTRON_",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix))
                {
                    command.env_remove(key);
                }
            }
            command
                .arg(format!("--user-data-dir={}", launch_data.display()))
                .arg(link)
                .env("CODEX_HOME", home)
                .env("CODEX_ELECTRON_USER_DATA_PATH", &launch_data)
                .current_dir(&project)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            Ok(())
        };
        for _ in 0..2 {
            open_session_in_mac_app(
                &app,
                &links,
                &home,
                &project,
                &mut |_| {},
                &mut launch,
                || super::super::project_assignment::macos::prepare(&app, &home),
                |existing_helpers| {
                    super::super::project_assignment::macos::assign_and_open(
                        &home,
                        thread_id,
                        &project,
                        existing_helpers,
                    )
                },
            )
            .unwrap();
            let registered = super::super::project_assignment::macos::fixture_project(
                &home, thread_id, &project, true,
            )
            .expect("Open in Codex must register the fixture folder");
            if let Some(expected) = &project_id {
                assert_eq!(&registered, expected, "repeat open must reuse the project");
            } else {
                std::fs::write(&project_id_path, &registered).unwrap();
                project_id = Some(registered);
            }
            let thread = super::super::project_assignment::macos::fixture_thread(
                &home,
                thread_id,
                Some(&project),
            );
            assert_eq!(thread["projectId"].as_str(), project_id.as_deref());
            assert_eq!(thread["hostId"], "local");
        }
        super::super::project_assignment::macos::verify_fixture_request_boundaries(
            &home, thread_id,
        );
        assert_eq!(
            super::super::project_assignment::macos::fixture_thread(
                &home,
                thread_id,
                Some(&project)
            )["projectId"]
                .as_str(),
            project_id.as_deref()
        );
        let configured = std::fs::read(home.join("config.toml")).unwrap();
        super::super::project_assignment::macos::prepare(&app, &home).unwrap();
        assert_eq!(std::fs::read(home.join("config.toml")).unwrap(), configured);
    }
}
