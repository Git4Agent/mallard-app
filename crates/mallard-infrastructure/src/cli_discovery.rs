//! Neutral executable discovery: where a command-line tool is on this
//! machine, with the same answer whether Mallard started from the Dock or
//! from a terminal.
//!
//! The order is an explicit selection (validated, never substituted), the
//! inherited `PATH`, on Unix the `PATH` the user's interactive login shell
//! has, then a few documented installation directories. Only absolute
//! directories are searched, so a relative or empty `PATH` entry can never
//! turn a project-local file into a tool. Provider launcher handling (npm
//! shims, platform packages) lives beside each provider; this module knows
//! files, not products.

use crate::bounded_process::{run_bounded, BoundedRunError};
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

/// Longest UTF-8 path a saved selection may hold.
pub const MAX_SELECTED_PATH_BYTES: usize = 4096;
#[cfg(unix)]
const SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(unix)]
const SHELL_PROBE_OUTPUT_LIMIT: usize = 64 * 1024;
const VERSION_PROBE_OUTPUT_LIMIT: usize = 4 * 1024;
const VERSION_LINE_LIMIT: usize = 128;
/// `package.json` is small; a larger file is not one npm wrote.
const PACKAGE_METADATA_LIMIT: u64 = 64 * 1024;

/// Which step of the policy named the launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliSelectionSource {
    /// The path the user saved in Settings.
    Selected,
    /// Mallard's own `PATH`.
    InheritedPath,
    /// The `PATH` the user's interactive login shell has.
    ShellPath,
    /// A documented installation directory.
    StandardLocation,
}

/// Whether discovery may spend a shell probe at all. General tools such as
/// `git` never do; a provider CLI may, and even then only when the probe can
/// change the answer (see [`discover_launcher`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellDiscovery {
    Use,
    Skip,
}

/// The launcher discovery found, as found: an installer's symlink stays a
/// symlink, so the saved or shown path survives the next update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredLauncher {
    pub launcher: PathBuf,
    pub source: CliSelectionSource,
    /// Every directory discovery searched, joined for a child's `PATH`.
    pub search_path: OsString,
}

/// One provider CLI, resolved for one operation: the launcher the policy
/// chose and the executable that launcher runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCli {
    pub launcher: PathBuf,
    pub executable: PathBuf,
    pub source: CliSelectionSource,
    /// The `PATH` for a direct-process child. A terminal launch ignores it:
    /// the terminal's shell has its own.
    pub search_path: OsString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliDiscoveryError {
    /// The saved selection is not an executable file. Discovery stops here
    /// rather than quietly running a different installation.
    SelectedUnavailable,
    NotFound,
}

/// Find `program` by the selection policy. `program` is one plain name, the
/// way it is typed in a terminal; platform extensions are the search's job.
///
/// The shell probe is spent only where it can change the answer: not for a
/// saved selection, not when the inherited `PATH` already names the
/// launcher, and not in a process a terminal started, whose `PATH` is the
/// shell's already. The order of precedence is unchanged by the saving.
pub fn discover_launcher(
    program: &str,
    explicit: Option<&Path>,
    shell: ShellDiscovery,
) -> Result<DiscoveredLauncher, CliDiscoveryError> {
    discover_launcher_with(program, explicit, inherited_path_dirs(), || match shell {
        ShellDiscovery::Use if !launched_from_terminal() => shell_path().unwrap_or_default(),
        _ => Vec::new(),
    })
}

fn discover_launcher_with(
    program: &str,
    explicit: Option<&Path>,
    inherited: Vec<PathBuf>,
    probe: impl FnOnce() -> Vec<PathBuf>,
) -> Result<DiscoveredLauncher, CliDiscoveryError> {
    let standard = standard_dirs();
    let found =
        |launcher: PathBuf, source: CliSelectionSource, recovered: &[PathBuf]| DiscoveredLauncher {
            launcher,
            source,
            search_path: join_dirs(
                inherited
                    .iter()
                    .chain(recovered.iter())
                    .chain(standard.iter()),
            ),
        };
    if let Some(explicit) = explicit {
        return match validate_executable(explicit) {
            Ok(_) => Ok(found(
                explicit.to_path_buf(),
                CliSelectionSource::Selected,
                &[],
            )),
            Err(_) => Err(CliDiscoveryError::SelectedUnavailable),
        };
    }
    if !is_plain_program_name(program) {
        return Err(CliDiscoveryError::NotFound);
    }
    if let Some(launcher) = find_in(program, &inherited) {
        return Ok(found(launcher, CliSelectionSource::InheritedPath, &[]));
    }
    let recovered = probe();
    if let Some(launcher) = find_in(program, &recovered) {
        return Ok(found(launcher, CliSelectionSource::ShellPath, &recovered));
    }
    if let Some(launcher) = find_in(program, &standard) {
        return Ok(found(
            launcher,
            CliSelectionSource::StandardLocation,
            &recovered,
        ));
    }
    Err(CliDiscoveryError::NotFound)
}

/// Whether a shell started this process. A terminal's shell exports `SHLVL`
/// of one or more; launchd, the Dock, and `open` (which hands the launch to
/// launchd) export none. The same test keeps JetBrains IDEs from probing.
fn launched_from_terminal() -> bool {
    shell_level_names_a_shell(std::env::var_os("SHLVL").as_deref())
}

fn shell_level_names_a_shell(level: Option<&OsStr>) -> bool {
    level
        .and_then(OsStr::to_str)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .is_some_and(|level| level > 0)
}

/// Find a general tool such as `git` the same way, without a provider's
/// launcher handling: the executable is the launcher.
pub fn find_tool(program: &str, shell: ShellDiscovery) -> Result<PathBuf, CliDiscoveryError> {
    let found = discover_launcher(program, None, shell)?;
    validate_executable(&found.launcher).map_err(|_| CliDiscoveryError::NotFound)
}

fn is_plain_program_name(program: &str) -> bool {
    !program.is_empty()
        && program
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// The first match in directory order. `which` applies the platform's
/// executable rules: `access(X_OK)` on Unix, `PATHEXT` on Windows.
fn find_in(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if dirs.is_empty() {
        return None;
    }
    let joined = std::env::join_paths(dirs).ok()?;
    which::which_in_global(program, Some(joined)).ok()?.next()
}

fn join_dirs<'a>(dirs: impl Iterator<Item = &'a PathBuf>) -> OsString {
    let mut seen = Vec::new();
    for dir in dirs {
        if !seen.contains(dir) {
            seen.push(dir.clone());
        }
    }
    std::env::join_paths(seen).unwrap_or_default()
}

/// Absolute entries of Mallard's own `PATH`, in order.
pub fn inherited_path_dirs() -> Vec<PathBuf> {
    absolute_dirs(std::env::var_os("PATH").as_deref())
}

fn absolute_dirs(path: Option<&OsStr>) -> Vec<PathBuf> {
    path.map(std::env::split_paths)
        .into_iter()
        .flatten()
        .filter(|dir| dir.is_absolute())
        .collect()
}

/// The documented installation directories a minimal desktop `PATH` lacks.
fn standard_dirs() -> Vec<PathBuf> {
    let home = crate::persistence::user_home_dir();
    #[cfg(unix)]
    {
        home.map(|home| home.join(".local").join("bin"))
            .into_iter()
            .chain([
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ])
            .collect()
    }
    #[cfg(windows)]
    {
        home.map(|home| home.join(".local").join("bin"))
            .into_iter()
            .chain(dirs::config_dir().map(|roaming| roaming.join("npm")))
            .collect()
    }
}

/// The canonical path of an executable regular file, or why `path` is not
/// one. The message never repeats the path: the caller shows it beside.
pub fn validate_executable(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("the path is not absolute".to_string());
    }
    let canonical = fs::canonicalize(path).map_err(|_| "the file does not exist".to_string())?;
    let metadata = fs::metadata(&canonical).map_err(|_| "the file cannot be read".to_string())?;
    if !metadata.is_file() {
        return Err("the path is not a regular file".to_string());
    }
    if !is_executable(&canonical) {
        return Err("the file is not executable".to_string());
    }
    Ok(canonical)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: access only reads the path string and checks permissions.
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            ["exe", "cmd", "bat", "com"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// The `name` field of `<package_dir>/package.json`, when npm wrote one.
pub fn npm_package_name(package_dir: &Path) -> Option<String> {
    let path = package_dir.join("package.json");
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > PACKAGE_METADATA_LIMIT {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("name")?.as_str().map(str::to_string)
}

// ---------------------------------------------------------------------------
// Shell PATH recovery
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellPathError {
    /// Not Bash or Zsh. Its flags are not assumed.
    UnsupportedShell,
    NoHome,
    Spawn,
    TimedOut,
    OutputLimit,
    /// No single marked result in the output.
    Malformed,
    /// The shell's `PATH` has no absolute entry.
    Empty,
}

/// Successful probes only. A failure is retried on the next call.
static SHELL_PATH: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

/// The absolute `PATH` entries of the user's interactive login shell, probed
/// once per process. Concurrent callers wait for the one probe.
pub fn shell_path() -> Result<Vec<PathBuf>, ShellPathError> {
    let mut cached = SHELL_PATH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(dirs) = cached.as_ref() {
        return Ok(dirs.clone());
    }
    let dirs = probe_login_shell()?;
    *cached = Some(dirs.clone());
    Ok(dirs)
}

/// Forget the probed `PATH`, so the next discovery runs the shell again.
pub fn refresh_shell_path() {
    *SHELL_PATH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// Spend the probe ahead of the first CLI action, so a launch after startup
/// finds the shell `PATH` already known. Called from a background thread; a
/// discovery that arrives while the probe runs waits for it rather than
/// starting another. A terminal-started process skips it, as discovery does.
pub fn warm_shell_path() {
    if !launched_from_terminal() {
        let _ = shell_path();
    }
}

#[cfg(unix)]
fn probe_login_shell() -> Result<Vec<PathBuf>, ShellPathError> {
    let shell = login_shell().ok_or(ShellPathError::UnsupportedShell)?;
    let home = crate::persistence::user_home_dir().ok_or(ShellPathError::NoHome)?;
    probe_shell_path(&shell, &home)
}

#[cfg(windows)]
fn probe_login_shell() -> Result<Vec<PathBuf>, ShellPathError> {
    Err(ShellPathError::UnsupportedShell)
}

/// `SHELL`, or the account's login shell when a desktop launch left it unset.
#[cfg(unix)]
fn login_shell() -> Option<PathBuf> {
    if let Some(shell) = std::env::var_os("SHELL").filter(|value| !value.is_empty()) {
        let shell = PathBuf::from(shell);
        if shell.is_absolute() {
            return Some(shell);
        }
    }
    account_shell()
}

#[cfg(unix)]
fn account_shell() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut buffer = vec![0u8; 16 * 1024];
    // SAFETY: passwd and result are plain out-parameters; the buffer outlives
    // the call and the string copies made from it.
    unsafe {
        let mut passwd: libc::passwd = std::mem::zeroed();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let status = libc::getpwuid_r(
            libc::getuid(),
            &mut passwd,
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len(),
            &mut result,
        );
        if status != 0 || result.is_null() || passwd.pw_shell.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr(passwd.pw_shell).to_bytes();
        let shell = PathBuf::from(OsStr::from_bytes(shell));
        shell.is_absolute().then_some(shell)
    }
}

#[cfg(unix)]
fn shell_is_supported(shell: &Path) -> bool {
    shell
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| matches!(name, "bash" | "zsh"))
}

/// Run `shell -i -l -c` in `home` and read back only its `PATH`, between two
/// copies of a random mark so startup text cannot pose as the result.
#[cfg(unix)]
fn probe_shell_path(shell: &Path, home: &Path) -> Result<Vec<PathBuf>, ShellPathError> {
    if !shell_is_supported(shell) {
        return Err(ShellPathError::UnsupportedShell);
    }
    let mark = random_mark();
    let script = format!("printf '%s' '{mark}' \"$PATH\" '{mark}'");
    let mut command = Command::new(shell);
    command
        .args(["-i", "-l", "-c", &script])
        .current_dir(home)
        .env("MALLARD_RESOLVING_ENVIRONMENT", "1");
    let output = run_bounded(&mut command, SHELL_PROBE_TIMEOUT, SHELL_PROBE_OUTPUT_LIMIT).map_err(
        |error| match error {
            BoundedRunError::Spawn => ShellPathError::Spawn,
            BoundedRunError::TimedOut => ShellPathError::TimedOut,
            BoundedRunError::OutputLimit => ShellPathError::OutputLimit,
            BoundedRunError::Output => ShellPathError::Malformed,
        },
    )?;
    parse_marked_path(&output.stdout, &mark)
}

#[cfg(unix)]
fn random_mark() -> String {
    let mut bytes = [0u8; 12];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Exactly one block between two marks, split into absolute directories.
fn parse_marked_path(stdout: &[u8], mark: &str) -> Result<Vec<PathBuf>, ShellPathError> {
    let text = String::from_utf8_lossy(stdout);
    let first = text.find(mark).ok_or(ShellPathError::Malformed)?;
    let body_start = first + mark.len();
    let second = text[body_start..]
        .find(mark)
        .ok_or(ShellPathError::Malformed)?;
    let body = &text[body_start..body_start + second];
    let rest = &text[body_start + second + mark.len()..];
    if rest.contains(mark) {
        return Err(ShellPathError::Malformed);
    }
    let dirs = absolute_dirs(Some(OsStr::new(body)));
    if dirs.is_empty() {
        return Err(ShellPathError::Empty);
    }
    Ok(dirs)
}

// ---------------------------------------------------------------------------
// Isolated version probe
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionProbeError {
    Spawn,
    TimedOut,
    /// Exited with failure, or printed nothing usable.
    Failed,
}

/// Run `executable --version` where it can touch nothing of the user's: a
/// throwaway home and working directory, a minimal environment carrying
/// only `search_path` and `extra_env`, and no inherited credentials. Returns
/// the first non-empty line, printable ASCII only.
pub fn probe_version(
    executable: &Path,
    search_path: Option<&OsStr>,
    extra_env: &[(&str, &OsStr)],
    timeout: Duration,
) -> Result<String, VersionProbeError> {
    let temp = tempfile::tempdir().map_err(|_| VersionProbeError::Spawn)?;
    let home = temp.path().join("home");
    fs::create_dir_all(&home).map_err(|_| VersionProbeError::Spawn)?;
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .env_clear()
        .current_dir(temp.path());
    let inherited = std::env::var_os("PATH");
    if let Some(path) = search_path.map(OsStr::to_os_string).or(inherited) {
        command.env("PATH", path);
    }
    command.env("HOME", &home).env("TMPDIR", temp.path());
    #[cfg(windows)]
    {
        for name in ["SYSTEMROOT", "SYSTEMDRIVE", "COMSPEC", "PATHEXT", "WINDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("USERPROFILE", &home)
            .env("APPDATA", home.join("roaming"))
            .env("LOCALAPPDATA", home.join("local"))
            .env("TEMP", temp.path())
            .env("TMP", temp.path());
    }
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let output = run_bounded(&mut command, timeout, VERSION_PROBE_OUTPUT_LIMIT).map_err(
        |error| match error {
            BoundedRunError::Spawn => VersionProbeError::Spawn,
            BoundedRunError::TimedOut => VersionProbeError::TimedOut,
            BoundedRunError::OutputLimit | BoundedRunError::Output => VersionProbeError::Failed,
        },
    )?;
    if !output.status.success() {
        return Err(VersionProbeError::Failed);
    }
    first_version_line(&output.stdout).ok_or(VersionProbeError::Failed)
}

fn first_version_line(stdout: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(stdout).ok()?;
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    (line.len() <= VERSION_LINE_LIMIT
        && line
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' '))
    .then(|| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn marked_path_parsing_accepts_exactly_one_block() {
        let mark = "abc123";
        let dirs = parse_marked_path(
            b"motd\nabc123/opt/tool/bin:relative:/usr/bin::~/binabc123trailer",
            mark,
        )
        .unwrap();
        assert_eq!(
            dirs,
            vec![PathBuf::from("/opt/tool/bin"), PathBuf::from("/usr/bin")]
        );
        for (stdout, expected) in [
            (&b"no marks"[..], ShellPathError::Malformed),
            (b"abc123/usr/bin", ShellPathError::Malformed),
            (b"abc123/usr/binabc123 abc123", ShellPathError::Malformed),
            (b"abc123abc123", ShellPathError::Empty),
            (b"abc123relative:bin/abc123", ShellPathError::Empty),
        ] {
            assert_eq!(parse_marked_path(stdout, mark).unwrap_err(), expected);
        }
    }

    #[test]
    #[cfg(unix)]
    fn executable_validation_rejects_what_cannot_run() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        assert!(validate_executable(Path::new("relative/tool")).is_err());
        assert!(validate_executable(&temp.path().join("missing")).is_err());
        assert!(validate_executable(temp.path()).is_err());
        let plain = temp.path().join("plain");
        fs::write(&plain, b"data").unwrap();
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            validate_executable(&plain).unwrap_err(),
            "the file is not executable"
        );
        let tool = temp.path().join("tool with space é");
        executable(&tool, "true");
        assert_eq!(
            validate_executable(&tool).unwrap(),
            fs::canonicalize(&tool).unwrap()
        );
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&tool, &link).unwrap();
        assert_eq!(
            validate_executable(&link).unwrap(),
            fs::canonicalize(&tool).unwrap()
        );
        let dangling = temp.path().join("dangling");
        std::os::unix::fs::symlink(temp.path().join("gone"), &dangling).unwrap();
        assert!(validate_executable(&dangling).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn search_takes_the_first_executable_in_directory_order() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        executable(&second.join("tool"), "true");
        let dirs = vec![first.clone(), second.clone()];
        assert_eq!(find_in("tool", &dirs), Some(second.join("tool")));
        // A non-executable file in an earlier directory is skipped.
        fs::write(first.join("tool"), b"not runnable").unwrap();
        assert_eq!(find_in("tool", &dirs), Some(second.join("tool")));
        executable(&first.join("tool"), "true");
        assert_eq!(find_in("tool", &dirs), Some(first.join("tool")));
        assert_eq!(find_in("other", &dirs), None);
        assert_eq!(find_in("tool", &[]), None);
        assert!(!is_plain_program_name("../tool"));
        assert!(!is_plain_program_name("tool name"));
        assert!(!is_plain_program_name(""));
    }

    #[test]
    #[cfg(unix)]
    fn an_explicit_selection_is_validated_and_never_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let chosen = temp.path().join("chosen");
        executable(&chosen, "true");
        let found = discover_launcher("tool", Some(&chosen), ShellDiscovery::Skip).unwrap();
        assert_eq!(found.launcher, chosen);
        assert_eq!(found.source, CliSelectionSource::Selected);
        assert_eq!(
            discover_launcher(
                "tool",
                Some(&temp.path().join("gone")),
                ShellDiscovery::Skip
            )
            .unwrap_err(),
            CliDiscoveryError::SelectedUnavailable
        );
        assert_eq!(
            discover_launcher("mallard-no-such-tool-xyz", None, ShellDiscovery::Skip).unwrap_err(),
            CliDiscoveryError::NotFound
        );
    }

    /// The probe is spent only where it can change the answer, and the
    /// order of precedence is the same whether or not it was spent.
    #[test]
    #[cfg(unix)]
    fn the_shell_probe_is_spent_only_where_it_can_change_the_answer() {
        use std::cell::Cell;
        let temp = tempfile::tempdir().unwrap();
        let inherited_dir = temp.path().join("inherited");
        let shell_dir = temp.path().join("shell");
        let tool = "mallard-probe-test-tool";
        executable(&shell_dir.join(tool), "true");
        let probed = Cell::new(0);
        let probe = || {
            probed.set(probed.get() + 1);
            vec![shell_dir.clone()]
        };
        let dirs_of = |found: &DiscoveredLauncher| -> Vec<PathBuf> {
            std::env::split_paths(&found.search_path).collect()
        };

        // A saved selection: no probe; the child PATH holds the inherited
        // and standard directories only.
        let chosen = temp.path().join("chosen");
        executable(&chosen, "true");
        let found = discover_launcher_with(tool, Some(&chosen), vec![inherited_dir.clone()], probe)
            .unwrap();
        assert_eq!(found.source, CliSelectionSource::Selected);
        assert_eq!(probed.get(), 0);
        assert_eq!(dirs_of(&found)[0], inherited_dir);
        assert!(!dirs_of(&found).contains(&shell_dir));

        // The inherited PATH names the launcher: no probe.
        executable(&inherited_dir.join(tool), "true");
        let found = discover_launcher_with(tool, None, vec![inherited_dir.clone()], probe).unwrap();
        assert_eq!(found.launcher, inherited_dir.join(tool));
        assert_eq!(found.source, CliSelectionSource::InheritedPath);
        assert_eq!(probed.get(), 0);

        // Otherwise the probe runs once, and its directories follow the
        // inherited ones in the child PATH.
        fs::remove_file(inherited_dir.join(tool)).unwrap();
        let found = discover_launcher_with(tool, None, vec![inherited_dir.clone()], probe).unwrap();
        assert_eq!(found.launcher, shell_dir.join(tool));
        assert_eq!(found.source, CliSelectionSource::ShellPath);
        assert_eq!(probed.get(), 1);
        assert_eq!(
            dirs_of(&found)[..2],
            [inherited_dir.clone(), shell_dir.clone()]
        );

        // Nothing anywhere: the probe was spent, and that is the answer.
        assert_eq!(
            discover_launcher_with("mallard-no-such-tool-xyz", None, vec![inherited_dir], probe)
                .unwrap_err(),
            CliDiscoveryError::NotFound
        );
        assert_eq!(probed.get(), 2);
    }

    #[test]
    fn a_shell_level_above_zero_means_a_terminal_started_the_process() {
        for (value, expected) in [
            (Some("1"), true),
            (Some("3"), true),
            (Some(" 2 "), true),
            (Some("0"), false),
            (Some(""), false),
            (Some("deep"), false),
            (None, false),
        ] {
            assert_eq!(
                shell_level_names_a_shell(value.map(OsStr::new)),
                expected,
                "{value:?}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_fixture_shell_yields_its_path_through_the_marks() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let zsh = temp.path().join("zsh");
        // Startup noise, then the probe script under the fixture's PATH.
        executable(
            &zsh,
            &format!(
                "echo 'Welcome'; test \"$1\" = -i && test \"$2\" = -l && test \"$3\" = -c || exit 9\n\
                 test \"$PWD\" = '{}' || exit 8\n\
                 test \"$MALLARD_RESOLVING_ENVIRONMENT\" = 1 || exit 7\n\
                 PATH='/fixture/bin:rel::/usr/bin' exec /bin/sh -c \"$4\"",
                fs::canonicalize(&home).unwrap().display()
            ),
        );
        assert_eq!(
            probe_shell_path(&zsh, &home).unwrap(),
            vec![PathBuf::from("/fixture/bin"), PathBuf::from("/usr/bin")]
        );
        let fish = temp.path().join("fish");
        executable(&fish, "true");
        assert_eq!(
            probe_shell_path(&fish, &home).unwrap_err(),
            ShellPathError::UnsupportedShell
        );
        let bash = temp.path().join("bash");
        executable(&bash, "echo 'no marks here'");
        assert_eq!(
            probe_shell_path(&bash, &home).unwrap_err(),
            ShellPathError::Malformed
        );
        executable(&bash, "head -c 70000 /dev/zero");
        assert_eq!(
            probe_shell_path(&bash, &home).unwrap_err(),
            ShellPathError::OutputLimit
        );
        assert_eq!(
            probe_shell_path(&temp.path().join("missing").join("bash"), &home).unwrap_err(),
            ShellPathError::Spawn
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_version_probe_is_isolated_and_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("tool");
        // The fixture proves the shape of its environment: no inherited
        // variables, a throwaway home, the caller's extra variables.
        executable(
            &tool,
            "test \"$1\" = --version || exit 9\n\
             test -z \"$USER\" || exit 7\n\
             test \"$HOME\" != \"$REAL_HOME\" || exit 6\n\
             test -d \"$HOME\" || exit 5\n\
             echo \"tool 1.2.3 ($PROVIDER_HOME)\"",
        );
        let real_home = std::env::var_os("HOME").unwrap_or_default();
        let extra = [
            ("REAL_HOME", real_home.as_os_str()),
            ("PROVIDER_HOME", OsStr::new("/tmp/provider")),
        ];
        assert_eq!(
            probe_version(&tool, None, &extra, Duration::from_secs(5)).unwrap(),
            "tool 1.2.3 (/tmp/provider)"
        );
        for (body, expected) in [
            ("echo 'tool 1.2.3'; exit 1", VersionProbeError::Failed),
            ("echo ''", VersionProbeError::Failed),
            ("printf '\\377\\n'", VersionProbeError::Failed),
            ("head -c 8192 /dev/zero", VersionProbeError::Failed),
        ] {
            executable(&tool, body);
            assert_eq!(
                probe_version(&tool, None, &extra, Duration::from_secs(5)).unwrap_err(),
                expected,
                "{body}"
            );
        }
        executable(&tool, "sleep 30");
        assert_eq!(
            probe_version(&tool, None, &extra, Duration::from_millis(300)).unwrap_err(),
            VersionProbeError::TimedOut
        );
        assert_eq!(
            probe_version(
                &temp.path().join("missing"),
                None,
                &extra,
                Duration::from_secs(1)
            )
            .unwrap_err(),
            VersionProbeError::Spawn
        );
    }

    /// Runs the account's real interactive login shell. Ignored: it depends
    /// on this machine's shell configuration and prints local paths.
    #[test]
    #[ignore = "runs the login shell of the account running the tests"]
    #[cfg(unix)]
    fn live_shell_path_is_recoverable() {
        refresh_shell_path();
        let dirs = shell_path().unwrap();
        assert!(!dirs.is_empty());
        eprintln!("shell PATH ({} dirs): {dirs:?}", dirs.len());
        assert_eq!(shell_path().unwrap(), dirs, "the probe is cached");
    }
}
