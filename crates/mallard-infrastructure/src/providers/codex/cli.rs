//! Which Codex executable a CLI action runs.
//!
//! Discovery names a launcher; this module turns it into the native binary.
//! The npm package's launcher is `bin/codex.js`, a Node script that spawns
//! `@openai/codex-<platform>/vendor/<triple>/bin/codex` and edits nothing
//! else, so Mallard runs that binary directly and keeps every launch
//! argv-only. Any other launcher is the executable itself.

use crate::cli_discovery::{
    self, npm_package_name, validate_executable, CliDiscoveryError, ResolvedCli, ShellDiscovery,
};
use crate::providers::traits::ProviderCli;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

const PACKAGE_NAME: &str = "@openai/codex";
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct CodexCli;

impl ProviderCli for CodexCli {
    fn resolve(&self, explicit: Option<&Path>) -> Result<ResolvedCli, String> {
        resolve(explicit)
    }

    fn probe_version(&self, cli: &ResolvedCli) -> Result<String, String> {
        let home = tempfile::tempdir().map_err(|_| "Could not create a temporary home.")?;
        let banner = cli_discovery::probe_version(
            &cli.executable,
            Some(&cli.search_path),
            &[("CODEX_HOME", home.path().as_os_str())],
            VERSION_PROBE_TIMEOUT,
        )
        .map_err(|_| "Codex did not report a version.".to_string())?;
        version_from_banner(&banner).ok_or_else(|| "Codex did not report a version.".to_string())
    }
}

/// `codex-cli 0.156.0` → `0.156.0`.
pub(super) fn version_from_banner(banner: &str) -> Option<String> {
    let version = banner.trim().strip_prefix("codex-cli ")?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// The Codex CLI for one operation: the saved selection when there is one,
/// otherwise the discovery policy.
pub fn resolve(explicit: Option<&Path>) -> Result<ResolvedCli, String> {
    let found = cli_discovery::discover_launcher("codex", explicit, ShellDiscovery::Use).map_err(
        |error| match error {
            CliDiscoveryError::SelectedUnavailable => {
                "Selected Codex executable is unavailable. Choose it again in Settings."
            }
            CliDiscoveryError::NotFound => {
                "Codex CLI not found. Install Codex, or choose its executable in Settings."
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

/// The binary `launcher` runs: the platform package's native Codex for the
/// npm launcher, the launcher's own file otherwise.
fn native_target(launcher: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(launcher)
        .map_err(|_| "Codex launcher is unavailable. Check the installation.".to_string())?;
    if let Some(package_root) = npm_launcher_package(launcher, &canonical) {
        return packaged_native(&package_root);
    }
    validate_executable(&canonical)
        .map_err(|reason| format!("Codex executable is unavailable: {reason}."))
}

/// The `@openai/codex` package root when `launcher` is its npm launcher:
/// `bin/codex.js` on Unix (behind npm's symlink), or the `codex.cmd` shim
/// npm writes beside `node_modules` on Windows.
fn npm_launcher_package(launcher: &Path, canonical: &Path) -> Option<PathBuf> {
    let script_root = canonical
        .file_name()
        .filter(|name| *name == OsStr::new("codex.js"))
        .and_then(|_| canonical.parent())
        .filter(|bin| bin.file_name() == Some(OsStr::new("bin")))
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let shim_root = launcher
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|name| name.eq_ignore_ascii_case("codex.cmd"))
        .and_then(|_| launcher.parent())
        .map(|prefix| prefix.join("node_modules").join("@openai").join("codex"));
    script_root
        .into_iter()
        .chain(shim_root)
        .find(|root| npm_package_name(root).as_deref() == Some(PACKAGE_NAME))
}

/// The platform package npm installs beside or under `@openai/codex`, and
/// the executable inside it. npm keeps it under the directory
/// `@openai/codex-<platform>`, but the manifest inside is published with
/// `name` `@openai/codex` and the platform in its version, so identity is
/// that directory plus a manifest naming either.
fn packaged_native(package_root: &Path) -> Result<PathBuf, String> {
    let (platform_package, triple, executable) = packaged_codex_target().ok_or_else(|| {
        "Codex has no native package for this platform. Choose its executable in Settings."
            .to_string()
    })?;
    let qualified = format!("@openai/{platform_package}");
    let nested = package_root
        .join("node_modules")
        .join("@openai")
        .join(platform_package);
    let hoisted = package_root
        .parent()
        .map(|scope| scope.join(platform_package));
    [nested]
        .into_iter()
        .chain(hoisted)
        .find(|candidate| {
            npm_package_name(candidate)
                .is_some_and(|name| name == qualified || name == PACKAGE_NAME)
        })
        .and_then(|platform_root| {
            validate_executable(
                &platform_root
                    .join("vendor")
                    .join(triple)
                    .join("bin")
                    .join(executable),
            )
            .ok()
        })
        .ok_or_else(|| {
            format!(
                "Codex installation is incomplete: its native package {qualified} is missing. \
                 Reinstall Codex, then try again."
            )
        })
}

fn packaged_codex_target() -> Option<(&'static str, &'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Some(("codex-darwin-x64", "x86_64-apple-darwin", "codex")),
        ("macos", "aarch64") => Some(("codex-darwin-arm64", "aarch64-apple-darwin", "codex")),
        ("linux", "x86_64") => Some(("codex-linux-x64", "x86_64-unknown-linux-musl", "codex")),
        ("linux", "aarch64") => Some(("codex-linux-arm64", "aarch64-unknown-linux-musl", "codex")),
        ("windows", "x86_64") => Some(("codex-win32-x64", "x86_64-pc-windows-msvc", "codex.exe")),
        ("windows", "aarch64") => {
            Some(("codex-win32-arm64", "aarch64-pc-windows-msvc", "codex.exe"))
        }
        _ => None,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\necho 'codex-cli 0.156.0'\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn package(root: &Path, name: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("package.json"),
            format!("{{\"name\":\"{name}\",\"version\":\"0.156.0\"}}"),
        )
        .unwrap();
    }

    /// npm's global layout: `bin/codex -> ../lib/node_modules/@openai/codex/bin/codex.js`.
    /// The nested platform package carries the manifest name npm really
    /// ships (`@openai/codex`); the hoisted one the directory's own name.
    fn npm_install(prefix: &Path, platform_nested: bool) -> PathBuf {
        let package_root = prefix.join("lib/node_modules/@openai/codex");
        package(&package_root, PACKAGE_NAME);
        fs::create_dir_all(package_root.join("bin")).unwrap();
        fs::write(package_root.join("bin/codex.js"), b"#!/usr/bin/env node\n").unwrap();
        let (platform_package, triple, name) = packaged_codex_target().unwrap();
        let (platform_root, manifest_name) = if platform_nested {
            (
                package_root
                    .join("node_modules/@openai")
                    .join(platform_package),
                PACKAGE_NAME.to_string(),
            )
        } else {
            (
                prefix
                    .join("lib/node_modules/@openai")
                    .join(platform_package),
                format!("@openai/{platform_package}"),
            )
        };
        package(&platform_root, &manifest_name);
        executable(
            &platform_root
                .join("vendor")
                .join(triple)
                .join("bin")
                .join(name),
        );
        let launcher = prefix.join("bin/codex");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        std::os::unix::fs::symlink(&package_root.join("bin/codex.js"), &launcher).unwrap();
        launcher
    }

    #[test]
    fn the_npm_launcher_resolves_to_the_platform_package_binary() {
        for nested in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let launcher = npm_install(temp.path(), nested);
            let (platform_package, triple, name) = packaged_codex_target().unwrap();
            let expected = if nested {
                temp.path()
                    .join("lib/node_modules/@openai/codex/node_modules/@openai")
                    .join(platform_package)
            } else {
                temp.path()
                    .join("lib/node_modules/@openai")
                    .join(platform_package)
            }
            .join("vendor")
            .join(triple)
            .join("bin")
            .join(name);
            assert_eq!(
                native_target(&launcher).unwrap(),
                fs::canonicalize(expected).unwrap(),
                "nested={nested}"
            );
        }
    }

    #[test]
    fn a_missing_or_foreign_platform_package_is_an_incomplete_installation() {
        let temp = tempfile::tempdir().unwrap();
        let launcher = npm_install(temp.path(), true);
        let (platform_package, _, _) = packaged_codex_target().unwrap();
        let platform_root = temp
            .path()
            .join("lib/node_modules/@openai/codex/node_modules/@openai")
            .join(platform_package);
        // A package of another name in the platform slot does not count.
        package(&platform_root, "@openai/not-codex");
        let error = native_target(&launcher).unwrap_err();
        assert!(error.contains("incomplete"), "{error}");
        assert!(!error.contains(temp.path().to_str().unwrap()), "{error}");
        fs::remove_dir_all(&platform_root).unwrap();
        assert!(native_target(&launcher).unwrap_err().contains("incomplete"));
    }

    #[test]
    fn a_native_launcher_is_the_executable_itself_even_through_a_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let native = temp.path().join("releases/0.156.0/bin/codex");
        executable(&native);
        let launcher = temp.path().join(".local/bin/codex");
        fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&native, &launcher).unwrap();
        assert_eq!(
            native_target(&launcher).unwrap(),
            fs::canonicalize(&native).unwrap()
        );
        // A `codex.js` that is not inside `@openai/codex` is run as itself.
        let script = temp.path().join("other/bin/codex.js");
        executable(&script);
        assert_eq!(
            native_target(&script).unwrap(),
            fs::canonicalize(&script).unwrap()
        );
        assert!(native_target(&temp.path().join("gone")).is_err());
    }

    #[test]
    fn the_shim_name_is_recognised_without_running_it() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("npm");
        let package_root = prefix.join("node_modules/@openai/codex");
        package(&package_root, PACKAGE_NAME);
        let shim = prefix.join("Codex.CMD");
        fs::create_dir_all(&prefix).unwrap();
        fs::write(&shim, b"@echo off\r\n").unwrap();
        assert_eq!(
            npm_launcher_package(&shim, &shim),
            Some(package_root.clone())
        );
        fs::write(package_root.join("package.json"), b"{\"name\":\"other\"}").unwrap();
        assert_eq!(npm_launcher_package(&shim, &shim), None);
    }

    #[test]
    fn version_banners_parse_only_from_the_cli_prefix() {
        assert_eq!(
            version_from_banner("codex-cli 0.156.0"),
            Some("0.156.0".to_string())
        );
        assert_eq!(version_from_banner("0.156.0"), None);
        assert_eq!(version_from_banner("codex-cli "), None);
    }

    #[test]
    fn an_explicit_selection_resolves_without_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let chosen = temp.path().join("my codex");
        executable(&chosen);
        let cli = resolve(Some(&chosen)).unwrap();
        assert_eq!(cli.launcher, chosen);
        assert_eq!(cli.executable, fs::canonicalize(&chosen).unwrap());
        assert_eq!(
            cli.source,
            crate::cli_discovery::CliSelectionSource::Selected
        );
        let error = resolve(Some(&temp.path().join("gone"))).unwrap_err();
        assert!(
            error.starts_with("Selected Codex executable is unavailable"),
            "{error}"
        );
    }

    /// Resolves the Codex installed on this machine and probes its version.
    #[test]
    #[ignore = "requires an installed Codex CLI; prints local paths"]
    fn live_installation_resolves_and_reports_a_version() {
        let cli = resolve(None).unwrap();
        eprintln!(
            "Codex: launcher {} -> executable {} ({:?})",
            cli.launcher.display(),
            cli.executable.display(),
            cli.source
        );
        let version = CodexCli.probe_version(&cli).unwrap();
        eprintln!("Codex version: {version}");
        assert!(version.starts_with(|character: char| character.is_ascii_digit()));
    }
}
