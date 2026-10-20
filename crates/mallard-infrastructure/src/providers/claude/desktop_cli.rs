//! Claude Desktop downloads its host CLI separately from the desktop app,
//! under its application data directory. Each install is
//! `<version>/<checksum prefix>/claude.app` on macOS or `claude.exe` on Windows; `.verified`
//! is written last. Inspect only that layout, never the sibling VM installs.

use crate::cli_discovery::validate_executable;
use semver::Version;
use std::fs;
#[cfg(target_os = "macos")]
use std::io::Cursor;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[cfg(target_os = "macos")]
const INSTALL_ROOT: &str = "Library/Application Support/Claude/claude-code";
const MAX_INSTALL_ENTRIES: usize = 256;
#[cfg(target_os = "macos")]
const MAX_PLIST_BYTES: u64 = 64 * 1024;
const CHECKSUM_BYTES: usize = 64;
const PREFIX_BYTES: usize = 12;

/// The newest complete, valid Desktop installation. Resolve afresh for each
/// operation so Desktop updates do not leave an automatically selected path
/// pinned to an old version. The home is injected for fixture-only tests.
#[cfg(target_os = "macos")]
pub(super) fn installation_dir_in(home: &Path) -> Option<PathBuf> {
    if !home.is_absolute() {
        return None;
    }
    let root = fs::canonicalize(home).ok()?.join(INSTALL_ROOT);
    installation_dir_at(&root)
}

#[cfg(windows)]
pub(super) fn windows_installation_dir() -> Option<PathBuf> {
    let app = crate::providers::windows_app::scheme_handler("claude").ok()?;
    let root = app
        .roaming_data_dir(&dirs::data_local_dir()?, &dirs::config_dir()?)?
        .join("Claude/claude-code");
    installation_dir_at(&root)
}

fn installation_dir_at(root: &Path) -> Option<PathBuf> {
    if !root.is_absolute() || !fs::symlink_metadata(root).ok()?.is_dir() {
        return None;
    }
    let canonical = fs::canonicalize(root).ok()?;
    if dunce::simplified(&canonical) != dunce::simplified(root) {
        return None;
    }
    let root = canonical;

    let mut candidates = Vec::new();
    for (count, entry) in WalkDir::new(&root)
        .min_depth(1)
        .max_depth(2)
        .follow_links(false)
        .follow_root_links(false)
        .into_iter()
        .enumerate()
    {
        // Refuse a truncated scan: filesystem enumeration order must not
        // decide which version is selected when the installation is too big.
        if count >= MAX_INSTALL_ENTRIES {
            return None;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if entry.depth() != 2 || !entry.file_type().is_dir() {
            continue;
        }
        let Some(prefix) = entry.file_name().to_str() else {
            continue;
        };
        if prefix.len() != PREFIX_BYTES
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            continue;
        }
        let Some(version) = entry
            .path()
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .and_then(|name| Version::parse(name).ok())
        else {
            continue;
        };
        candidates.push((version, entry.into_path()));
    }
    // Version order is numeric; equal versions use a stable path order.
    candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates
        .into_iter()
        .find_map(|(version, path)| validate_installation(&path, &version))
}

fn validate_installation(path: &Path, version: &Version) -> Option<PathBuf> {
    if fs::canonicalize(path).ok()? != path {
        return None;
    }
    let prefix = path.file_name()?.to_str()?;
    let marker = read_metadata_file(&path.join(".verified"), CHECKSUM_BYTES as u64)?;
    // This is Desktop's completed-download marker, not an executable hash
    // or an authenticity check. The marker must belong to this build.
    if marker.len() != CHECKSUM_BYTES
        || !marker.iter().all(u8::is_ascii_hexdigit)
        || !marker[..PREFIX_BYTES].eq_ignore_ascii_case(prefix.as_bytes())
    {
        return None;
    }

    #[cfg(target_os = "macos")]
    let launcher = mac_launcher(path, version)?;
    #[cfg(windows)]
    let launcher = {
        let _ = version;
        path.join("claude.exe")
    };
    if !fs::symlink_metadata(&launcher).ok()?.is_file()
        || validate_executable(&launcher).ok()? != launcher
    {
        return None;
    }
    launcher.parent().map(Path::to_path_buf)
}

#[cfg(target_os = "macos")]
fn mac_launcher(path: &Path, version: &Version) -> Option<PathBuf> {
    let contents = path.join("claude.app/Contents");
    let bytes = read_metadata_file(&contents.join("Info.plist"), MAX_PLIST_BYTES)?;
    let dictionary = plist::Value::from_reader(Cursor::new(bytes))
        .ok()?
        .into_dictionary()?;
    let string = |key: &str| dictionary.get(key).and_then(plist::Value::as_string);
    if string("CFBundleIdentifier") != Some("com.anthropic.claude-code")
        || string("CFBundleExecutable") != Some("claude")
        || string("CFBundleShortVersionString") != Some(version.to_string().as_str())
    {
        return None;
    }

    Some(contents.join("MacOS/claude"))
}

/// Bound metadata reads and reject symlinks in both the leaf and its parents.
fn read_metadata_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit || fs::canonicalize(path).ok()? != path {
        return None;
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    const BUILD: &str = "012345abcdef";

    fn installation(root: &Path, version: &str, build: &str) -> PathBuf {
        let path = root.join(version).join(build);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("claude.exe"), b"fixture").unwrap();
        fs::write(path.join(".verified"), format!("{build}{}", "0".repeat(52))).unwrap();
        path
    }

    #[test]
    fn windows_downloads_use_numeric_versions_and_observe_completed_updates() {
        let root = tempfile::tempdir().unwrap();
        installation(root.path(), "2.1.9", BUILD);
        installation(root.path(), "2.1.10-beta.1", BUILD);
        installation(root.path(), "2.1.10", "ffffffffffff");
        let current = installation(root.path(), "2.1.10", BUILD);
        assert_eq!(
            installation_dir_at(root.path()),
            Some(fs::canonicalize(&current).unwrap())
        );
        let update = installation(root.path(), "2.1.11", BUILD);
        fs::remove_file(update.join(".verified")).unwrap();
        assert_eq!(
            installation_dir_at(root.path()),
            Some(fs::canonicalize(&current).unwrap())
        );
        fs::write(
            update.join(".verified"),
            format!("{BUILD}{}", "0".repeat(52)),
        )
        .unwrap();
        assert_eq!(
            installation_dir_at(root.path()),
            Some(fs::canonicalize(&update).unwrap())
        );
    }

    #[test]
    fn windows_downloads_reject_incomplete_or_malformed_installations() {
        for invalid in [
            "missing marker",
            "wrong marker",
            "oversized marker",
            "missing executable",
            "executable directory",
        ] {
            let root = tempfile::tempdir().unwrap();
            let valid = installation(root.path(), "2.1.9", BUILD);
            let bad = installation(root.path(), "2.1.10", BUILD);
            match invalid {
                "missing marker" => fs::remove_file(bad.join(".verified")).unwrap(),
                "wrong marker" => fs::write(bad.join(".verified"), "f".repeat(64)).unwrap(),
                "oversized marker" => fs::write(bad.join(".verified"), "0".repeat(65)).unwrap(),
                "missing executable" => fs::remove_file(bad.join("claude.exe")).unwrap(),
                "executable directory" => {
                    fs::remove_file(bad.join("claude.exe")).unwrap();
                    fs::create_dir(bad.join("claude.exe")).unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                installation_dir_at(root.path()),
                Some(fs::canonicalize(&valid).unwrap()),
                "{invalid}"
            );
        }
    }

    #[test]
    fn windows_download_scan_is_bounded_and_uses_only_the_host_layout() {
        let root = tempfile::tempdir().unwrap();
        installation(root.path(), "latest", BUILD);
        installation(root.path(), "2.1.10", "not-a-build");
        installation(root.path(), "2.1.11", "ABCDEF012345");
        installation(&root.path().join("vm"), "2.1.12", BUILD);
        assert_eq!(installation_dir_at(root.path()), None);
        assert_eq!(installation_dir_at(Path::new(".")), None);
        installation(root.path(), "2.1.10", BUILD);
        for index in 0..MAX_INSTALL_ENTRIES {
            fs::write(root.path().join(format!("extra-{index}")), b"").unwrap();
        }
        assert_eq!(installation_dir_at(root.path()), None);
    }

    #[test]
    fn windows_downloads_reject_directory_junctions() {
        use std::os::windows::process::CommandExt;
        for relative in ["", "2.1.10", "2.1.10/012345abcdef"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("claude-code");
            installation(&root, "2.1.10", BUILD);
            let redirected = root.join(relative);
            let outside = temp.path().join("outside");
            fs::rename(&redirected, &outside).unwrap();
            let result = std::process::Command::new("cmd.exe")
                .args(["/c", "mklink", "/J"])
                .arg(redirected.to_string_lossy().replace('/', "\\"))
                .arg(outside.to_string_lossy().replace('/', "\\"))
                .creation_flags(0x0800_0000)
                .output()
                .unwrap();
            assert!(result.status.success(), "{result:?}");
            assert_eq!(installation_dir_at(&root), None, "{relative}");
            fs::remove_dir(redirected).unwrap();
        }
    }

    #[test]
    #[ignore = "requires installed Claude Desktop runtime and PATH without standalone CLIs; uses a temporary home"]
    fn installed_desktop_cli_resolves_and_reports_its_version() {
        use crate::providers::traits::ProviderCli;
        let dir = windows_installation_dir().expect("Desktop must have downloaded its CLI");
        let cli = super::super::cli::resolve(None).unwrap();
        assert_eq!(cli.executable, dir.join("claude.exe"));
        assert_eq!(
            cli.source,
            crate::cli_discovery::CliSelectionSource::StandardLocation
        );
        let version = super::super::cli::ClaudeCli.probe_version(&cli).unwrap();
        println!("Desktop Claude {version}: discovery and isolated version probe passed");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    const BUILD: &str = "012345abcdef";

    fn installation(home: &Path, version: &str, build: &str) -> PathBuf {
        let path = home.join(INSTALL_ROOT).join(version).join(build);
        let contents = path.join("claude.app/Contents");
        fs::create_dir_all(contents.join("MacOS")).unwrap();
        fs::write(path.join(".verified"), format!("{build}{}", "0".repeat(52))).unwrap();
        let mut info = plist::Dictionary::new();
        for (key, value) in [
            ("CFBundleIdentifier", "com.anthropic.claude-code"),
            ("CFBundleExecutable", "claude"),
            ("CFBundleShortVersionString", version),
        ] {
            info.insert(key.to_string(), plist::Value::String(value.to_string()));
        }
        plist::Value::Dictionary(info)
            .to_file_xml(contents.join("Info.plist"))
            .unwrap();
        let executable = contents.join("MacOS/claude");
        fs::write(&executable, b"#!/bin/sh\necho '2.1.299 (Claude Code)'\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn expected_dir(install: &Path) -> PathBuf {
        fs::canonicalize(install.join("claude.app/Contents/MacOS")).unwrap()
    }

    #[test]
    fn newest_complete_version_wins_and_updates_are_seen_without_a_cache() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(installation_dir_in(home.path()), None);
        installation(home.path(), "2.1.9", BUILD);
        installation(home.path(), "2.1.10-beta.1", BUILD);
        let newer = installation(home.path(), "2.1.10", BUILD);
        assert_eq!(installation_dir_in(home.path()), Some(expected_dir(&newer)));

        let updating = installation(home.path(), "2.1.11", BUILD);
        fs::remove_file(updating.join(".verified")).unwrap();
        assert_eq!(installation_dir_in(home.path()), Some(expected_dir(&newer)));
        fs::write(
            updating.join(".verified"),
            format!("{BUILD}{}", "0".repeat(52)),
        )
        .unwrap();
        assert_eq!(
            installation_dir_in(home.path()),
            Some(expected_dir(&updating))
        );
    }

    #[test]
    fn equal_versions_use_a_deterministic_build_order() {
        let home = tempfile::tempdir().unwrap();
        installation(home.path(), "2.1.10", "ffffffffffff");
        let first = installation(home.path(), "2.1.10", BUILD);
        assert_eq!(installation_dir_in(home.path()), Some(expected_dir(&first)));
    }

    #[test]
    fn invalid_or_incomplete_installations_do_not_hide_a_valid_version() {
        for invalid in [
            "missing marker",
            "malformed marker",
            "wrong build marker",
            "oversized marker",
            "missing executable",
            "nonexecutable file",
            "executable directory",
            "missing plist",
            "malformed plist",
            "oversized plist",
            "wrong bundle",
            "wrong executable name",
            "wrong bundle version",
        ] {
            let home = tempfile::tempdir().unwrap();
            let valid = installation(home.path(), "2.1.9", BUILD);
            let bad = installation(home.path(), "2.1.10", BUILD);
            let executable = bad.join("claude.app/Contents/MacOS/claude");
            let plist = bad.join("claude.app/Contents/Info.plist");
            match invalid {
                "missing marker" => fs::remove_file(bad.join(".verified")).unwrap(),
                "malformed marker" => fs::write(bad.join(".verified"), "z".repeat(64)).unwrap(),
                "wrong build marker" => fs::write(bad.join(".verified"), "f".repeat(64)).unwrap(),
                "oversized marker" => fs::write(bad.join(".verified"), "0".repeat(65)).unwrap(),
                "missing executable" => fs::remove_file(executable).unwrap(),
                "nonexecutable file" => {
                    fs::set_permissions(executable, fs::Permissions::from_mode(0o644)).unwrap()
                }
                "executable directory" => {
                    fs::remove_file(&executable).unwrap();
                    fs::create_dir(executable).unwrap();
                }
                "missing plist" => fs::remove_file(plist).unwrap(),
                "malformed plist" => fs::write(plist, b"not a plist").unwrap(),
                "oversized plist" => {
                    fs::write(plist, vec![b' '; MAX_PLIST_BYTES as usize + 1]).unwrap()
                }
                other => {
                    let key = match other {
                        "wrong bundle" => "CFBundleIdentifier",
                        "wrong executable name" => "CFBundleExecutable",
                        "wrong bundle version" => "CFBundleShortVersionString",
                        _ => unreachable!(),
                    };
                    let mut value = plist::Value::from_file(&plist).unwrap();
                    value
                        .as_dictionary_mut()
                        .unwrap()
                        .insert(key.to_string(), plist::Value::String("other".to_string()));
                    value.to_file_xml(plist).unwrap();
                }
            }
            assert_eq!(
                installation_dir_in(home.path()),
                Some(expected_dir(&valid)),
                "{invalid}"
            );
            fs::remove_dir_all(valid).unwrap();
            assert_eq!(installation_dir_in(home.path()), None, "{invalid}");
        }
    }

    #[test]
    fn symlinks_are_rejected_at_every_level_of_the_installation() {
        for relative in [
            "",
            "2.1.10",
            "2.1.10/012345abcdef",
            "2.1.10/012345abcdef/.verified",
            "2.1.10/012345abcdef/claude.app",
            "2.1.10/012345abcdef/claude.app/Contents",
            "2.1.10/012345abcdef/claude.app/Contents/Info.plist",
            "2.1.10/012345abcdef/claude.app/Contents/MacOS",
            "2.1.10/012345abcdef/claude.app/Contents/MacOS/claude",
        ] {
            let home = tempfile::tempdir().unwrap();
            installation(home.path(), "2.1.10", BUILD);
            let root = home.path().join(INSTALL_ROOT);
            let path = if relative.is_empty() {
                root
            } else {
                root.join(relative)
            };
            let outside = home.path().join("outside");
            fs::rename(&path, &outside).unwrap();
            symlink(&outside, &path).unwrap();
            assert_eq!(installation_dir_in(home.path()), None, "{relative}");
        }
    }

    #[test]
    fn only_the_host_version_and_build_layout_is_searched() {
        let home = tempfile::tempdir().unwrap();
        installation(home.path(), "latest", BUILD);
        installation(home.path(), "2.1.10", "not-a-build");
        installation(home.path(), "2.1.11", "ABCDEF012345");
        let host = installation(home.path(), "2.1.12", BUILD);
        fs::rename(
            host,
            home.path()
                .join("Library/Application Support/Claude/claude-code-vm"),
        )
        .unwrap();
        assert_eq!(installation_dir_in(home.path()), None);
        assert_eq!(installation_dir_in(Path::new(".")), None);
    }

    #[test]
    fn exceeding_the_scan_bound_does_not_select_an_arbitrary_version() {
        let home = tempfile::tempdir().unwrap();
        installation(home.path(), "2.1.10", BUILD);
        let root = home.path().join(INSTALL_ROOT);
        for index in 0..MAX_INSTALL_ENTRIES {
            fs::write(root.join(format!("extra-{index}")), b"").unwrap();
        }
        assert_eq!(installation_dir_in(home.path()), None);
    }
}
