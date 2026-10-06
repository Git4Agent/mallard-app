//! Which Claude executable a CLI action runs.
//!
//! Every supported Claude installation is one native binary. The installer's
//! `~/.local/bin/claude` is a symlink into its versions directory; the npm
//! package's postinstall places the same binary at `<package>/bin/claude.exe`
//! on every platform and npm links or shims `claude` to it. Canonicalizing
//! the launcher is therefore the whole resolution, except for npm's Windows
//! `claude.cmd` shim, which is read as a name and never run.

use crate::cli_discovery::{
    self, npm_package_name, validate_executable, CliDiscoveryError, ResolvedCli, ShellDiscovery,
};
use crate::providers::traits::ProviderCli;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

const PACKAGE_NAME: &str = "@anthropic-ai/claude-code";
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ClaudeCli;

impl ProviderCli for ClaudeCli {
    fn resolve(&self, explicit: Option<&Path>) -> Result<ResolvedCli, String> {
        resolve(explicit)
    }

    /// `--version` against a throwaway config directory, with the updater
    /// and every nonessential request switched off.
    fn probe_version(&self, cli: &ResolvedCli) -> Result<String, String> {
        let home = tempfile::tempdir().map_err(|_| "Could not create a temporary home.")?;
        let banner = cli_discovery::probe_version(
            &cli.executable,
            Some(&cli.search_path),
            &[
                ("CLAUDE_CONFIG_DIR", home.path().as_os_str()),
                ("DISABLE_AUTOUPDATER", OsStr::new("1")),
                ("DISABLE_TELEMETRY", OsStr::new("1")),
                ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", OsStr::new("1")),
            ],
            VERSION_PROBE_TIMEOUT,
        )
        .map_err(|_| "Claude did not report a version.".to_string())?;
        version_from_banner(&banner).ok_or_else(|| "Claude did not report a version.".to_string())
    }
}

/// `2.1.280 (Claude Code)` → `2.1.280`.
fn version_from_banner(banner: &str) -> Option<String> {
    let version = banner.split_whitespace().next()?;
    version
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
        .then(|| version.to_string())
}

/// The Claude CLI for one operation: the saved selection when there is one,
/// otherwise the discovery policy.
pub fn resolve(explicit: Option<&Path>) -> Result<ResolvedCli, String> {
    let found = cli_discovery::discover_launcher("claude", explicit, ShellDiscovery::Use).map_err(
        |error| match error {
            CliDiscoveryError::SelectedUnavailable => {
                "Selected Claude executable is unavailable. Choose it again in Settings."
            }
            CliDiscoveryError::NotFound => {
                "Claude CLI not found. Install Claude Code, or choose its executable in Settings."
            }
        },
    )?;
    let executable = native_target(&found.launcher)?;
    Ok(ResolvedCli {
        launcher: found.launcher,
        executable,
        source: found.source,
        search_path: found.search_path,
    })
}

fn native_target(launcher: &Path) -> Result<PathBuf, String> {
    if let Some(package_root) = npm_shim_package(launcher) {
        return validate_executable(&package_root.join("bin").join("claude.exe")).map_err(|_| {
            "Claude installation is incomplete: its native binary is missing. Reinstall \
             Claude Code, then try again."
                .to_string()
        });
    }
    validate_executable(launcher)
        .map_err(|reason| format!("Claude executable is unavailable: {reason}."))
}

/// The package root when `launcher` is npm's `claude.cmd` shim, which sits
/// beside the `node_modules` holding the package.
fn npm_shim_package(launcher: &Path) -> Option<PathBuf> {
    launcher
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|name| name.eq_ignore_ascii_case("claude.cmd"))
        .and_then(|_| launcher.parent())
        .map(|prefix| {
            prefix
                .join("node_modules")
                .join("@anthropic-ai")
                .join("claude-code")
        })
        .filter(|root| npm_package_name(root).as_deref() == Some(PACKAGE_NAME))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\necho '2.1.280 (Claude Code)'\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn installer_and_npm_launchers_canonicalize_to_the_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let versioned = temp.path().join(".local/share/claude/versions/2.1.280");
        executable(&versioned);
        let launcher = temp.path().join(".local/bin/claude");
        fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&versioned, &launcher).unwrap();
        assert_eq!(
            native_target(&launcher).unwrap(),
            fs::canonicalize(&versioned).unwrap()
        );

        let package_binary = temp
            .path()
            .join("lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe");
        executable(&package_binary);
        let npm_launcher = temp.path().join("bin/claude");
        fs::create_dir_all(npm_launcher.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&package_binary, &npm_launcher).unwrap();
        assert_eq!(
            native_target(&npm_launcher).unwrap(),
            fs::canonicalize(&package_binary).unwrap()
        );
        assert!(native_target(&temp.path().join("gone")).is_err());
    }

    #[test]
    fn the_windows_shim_is_read_as_a_name_and_mapped_to_the_package_binary() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("npm");
        let package_root = prefix.join("node_modules/@anthropic-ai/claude-code");
        fs::create_dir_all(&package_root).unwrap();
        fs::write(
            package_root.join("package.json"),
            format!("{{\"name\":\"{PACKAGE_NAME}\"}}"),
        )
        .unwrap();
        let shim = prefix.join("claude.CMD");
        fs::write(&shim, b"@echo off\r\n").unwrap();
        assert_eq!(npm_shim_package(&shim), Some(package_root.clone()));
        let error = native_target(&shim).unwrap_err();
        assert!(error.contains("incomplete"), "{error}");
        executable(&package_root.join("bin/claude.exe"));
        assert_eq!(
            native_target(&shim).unwrap(),
            fs::canonicalize(package_root.join("bin/claude.exe")).unwrap()
        );
        fs::write(package_root.join("package.json"), b"{\"name\":\"other\"}").unwrap();
        assert_eq!(npm_shim_package(&shim), None);
    }

    #[test]
    fn version_banners_keep_only_the_number() {
        assert_eq!(
            version_from_banner("2.1.280 (Claude Code)"),
            Some("2.1.280".to_string())
        );
        assert_eq!(version_from_banner("Claude Code 2.1.280"), None);
        assert_eq!(version_from_banner(""), None);
    }

    #[test]
    fn an_explicit_selection_resolves_without_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let chosen = temp.path().join("my claude");
        executable(&chosen);
        let cli = resolve(Some(&chosen)).unwrap();
        assert_eq!(cli.executable, fs::canonicalize(&chosen).unwrap());
        assert!(resolve(Some(&temp.path().join("gone")))
            .unwrap_err()
            .starts_with("Selected Claude executable is unavailable"));
    }

    /// Resolves the Claude installed on this machine and probes its version.
    #[test]
    #[ignore = "requires an installed Claude CLI; prints local paths"]
    fn live_installation_resolves_and_reports_a_version() {
        let cli = resolve(None).unwrap();
        eprintln!(
            "Claude: launcher {} -> executable {} ({:?})",
            cli.launcher.display(),
            cli.executable.display(),
            cli.source
        );
        let version = ClaudeCli.probe_version(&cli).unwrap();
        eprintln!("Claude version: {version}");
        assert!(version.starts_with(|character: char| character.is_ascii_digit()));
    }
}
