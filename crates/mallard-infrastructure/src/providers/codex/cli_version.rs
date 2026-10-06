//! The Codex CLI that resumes a session must be 0.145.0 or later: older
//! releases reject Mallard's paginated threads during TUI bootstrap
//! (`paginated_threads is not supported yet`, -32601).
//!
//! The version is read only when the user opens a session — never per row,
//! never cached — from the exact executable that launch will run.

use semver::Version;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

const MINIMUM_RESUME_VERSION: Version = Version::new(0, 145, 0);
/// Generous for a version print: the first run of a freshly updated app
/// bundle passes Gatekeeper's assessment first.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Which CLI a launch runs, which decides what the user is told to update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResumeCli {
    /// The CLI on PATH that Open in Terminal runs.
    Terminal,
    /// The CLI bundled inside the Codex desktop app, which Open in Codex runs.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    DesktopApp,
}

/// Refuse a session resume the CLI at `executable` cannot perform.
/// `search_path` is the discovery `PATH` for a resolved CLI; the bundled
/// desktop-app CLI runs without one.
pub(super) fn require_session_resume(
    executable: &Path,
    search_path: Option<&OsStr>,
    cli: ResumeCli,
) -> Result<(), String> {
    check(probe(executable, search_path, PROBE_TIMEOUT), cli)
}

fn check(version: Option<Version>, cli: ResumeCli) -> Result<(), String> {
    match version {
        Some(version) if version >= MINIMUM_RESUME_VERSION => Ok(()),
        Some(version) => Err(match cli {
            ResumeCli::Terminal => format!(
                "Cannot open this session with Codex CLI {version}. Mallard requires Codex CLI \
                 {MINIMUM_RESUME_VERSION} or later for session resume. Update Codex CLI, then try again."
            ),
            ResumeCli::DesktopApp => format!(
                "Cannot open this session in the Codex desktop app: its bundled Codex CLI is \
                 {version}. Mallard requires Codex CLI {MINIMUM_RESUME_VERSION} or later for session \
                 resume. Update the Codex desktop app, then try again."
            ),
        }),
        None => Err(match cli {
            ResumeCli::Terminal => "Cannot determine the Codex CLI version. Check your Codex CLI \
                                    installation, then try again."
                .to_string(),
            ResumeCli::DesktopApp => "Cannot determine the Codex CLI version bundled with the \
                                      Codex desktop app. Check your Codex desktop app installation, \
                                      then try again."
                .to_string(),
        }),
    }
}

/// `codex --version` still prepares Codex's arg0 helpers under `CODEX_HOME`,
/// so it runs against a throwaway home and never writes into a bound one.
fn probe(executable: &Path, search_path: Option<&OsStr>, timeout: Duration) -> Option<Version> {
    let home = tempfile::tempdir().ok()?;
    let banner = crate::cli_discovery::probe_version(
        executable,
        search_path,
        &[("CODEX_HOME", home.path().as_os_str())],
        timeout,
    )
    .ok()?;
    parse_version(banner.as_bytes())
}

/// `codex-cli 0.156.0` → 0.156.0. Prerelease and build parts are kept, so
/// `0.145.0-alpha.3` sorts below `0.145.0`.
fn parse_version(stdout: &[u8]) -> Option<Version> {
    let text = std::str::from_utf8(stdout).ok()?;
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Version::parse(&super::cli::version_from_banner(line)?).ok()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn versions_parse_only_from_the_cli_banner() {
        assert_eq!(
            parse_version(b"codex-cli 0.156.0\n"),
            Some(Version::new(0, 156, 0))
        );
        assert_eq!(
            parse_version(b"\ncodex-cli 0.158.0-alpha.2.1\nextra\n"),
            Some(Version::parse("0.158.0-alpha.2.1").unwrap())
        );
        for malformed in [
            &b""[..],
            b"0.156.0",
            b"codex 0.156.0",
            b"codex-cli",
            b"codex-cli banana",
            b"codex-cli 0.156",
            b"\xff\xfe",
        ] {
            assert_eq!(parse_version(malformed), None, "{malformed:?}");
        }
    }

    #[test]
    fn resume_needs_0_145_0_or_later_by_semantic_order() {
        for refused in ["0.143.0", "0.144.0", "0.144.99", "0.145.0-alpha.3", "0.9.0"] {
            assert!(
                check(Version::parse(refused).ok(), ResumeCli::Terminal).is_err(),
                "{refused}"
            );
        }
        for allowed in [
            "0.145.0",
            "0.145.1",
            "0.147.0",
            "0.156.0",
            "0.158.0-alpha.2.1",
            "0.159.2",
            "1.0.0",
        ] {
            assert_eq!(
                check(Version::parse(allowed).ok(), ResumeCli::Terminal),
                Ok(()),
                "{allowed}"
            );
        }
    }

    #[test]
    fn messages_name_the_cli_the_user_must_update() {
        let old = Version::new(0, 144, 0);
        assert_eq!(
            check(Some(old.clone()), ResumeCli::Terminal).unwrap_err(),
            "Cannot open this session with Codex CLI 0.144.0. Mallard requires Codex CLI \
             0.145.0 or later for session resume. Update Codex CLI, then try again."
        );
        assert_eq!(
            check(Some(old), ResumeCli::DesktopApp).unwrap_err(),
            "Cannot open this session in the Codex desktop app: its bundled Codex CLI is \
             0.144.0. Mallard requires Codex CLI 0.145.0 or later for session resume. Update \
             the Codex desktop app, then try again."
        );
        assert_eq!(
            check(None, ResumeCli::Terminal).unwrap_err(),
            "Cannot determine the Codex CLI version. Check your Codex CLI installation, then \
             try again."
        );
        assert_eq!(
            check(None, ResumeCli::DesktopApp).unwrap_err(),
            "Cannot determine the Codex CLI version bundled with the Codex desktop app. Check \
             your Codex desktop app installation, then try again."
        );
    }

    /// A fake CLI at `path`; `body` runs under `/bin/sh` and sees the
    /// probe's `CODEX_HOME`.
    #[cfg(unix)]
    pub(in crate::providers::codex) fn fake_cli(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn the_probe_reads_the_executable_it_is_given() {
        let temp = tempfile::tempdir().unwrap();
        let cli = temp.path().join("codex");
        fake_cli(&cli, "echo 'codex-cli 0.144.0'");
        assert!(require_session_resume(&cli, None, ResumeCli::Terminal)
            .unwrap_err()
            .contains("Codex CLI 0.144.0."));
        fake_cli(&cli, "echo 'codex-cli 0.145.0'");
        assert_eq!(
            require_session_resume(&cli, None, ResumeCli::Terminal),
            Ok(())
        );
    }

    #[test]
    #[cfg(unix)]
    fn unreadable_versions_are_undetermined() {
        let temp = tempfile::tempdir().unwrap();
        let cli = temp.path().join("codex");
        let undetermined = check(None, ResumeCli::Terminal).unwrap_err();
        assert_eq!(
            require_session_resume(&cli, None, ResumeCli::Terminal).unwrap_err(),
            undetermined,
            "missing executable"
        );
        for body in [
            "echo 'codex-cli 0.156.0'; exit 1",
            "echo 'not a version'",
            "true",
            "head -c 8192 /dev/zero",
        ] {
            fake_cli(&cli, body);
            assert_eq!(
                require_session_resume(&cli, None, ResumeCli::Terminal).unwrap_err(),
                undetermined,
                "{body}"
            );
        }
        fake_cli(&cli, "sleep 30");
        assert_eq!(
            probe(&cli, None, Duration::from_millis(200)),
            None,
            "timed out"
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_probe_writes_only_into_a_throwaway_home() {
        let temp = tempfile::tempdir().unwrap();
        let cli = temp.path().join("codex");
        let seen = temp.path().join("seen-home");
        fake_cli(
            &cli,
            &format!(
                "mkdir -p \"$CODEX_HOME/tmp/arg0\"\nprintf '%s' \"$CODEX_HOME\" > '{}'\n\
                 echo 'codex-cli 0.156.0'",
                seen.display()
            ),
        );
        assert_eq!(
            require_session_resume(&cli, None, ResumeCli::Terminal),
            Ok(())
        );
        let probe_home = std::fs::read_to_string(&seen).unwrap();
        assert!(!probe_home.is_empty());
        assert!(
            !Path::new(&probe_home).exists(),
            "the throwaway home is removed"
        );
    }
}
