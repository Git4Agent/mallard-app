//! Provider CLI binary resolution.
//!
//! Resolves the `codex` (and other provider) executables without assuming a
//! POSIX shell exists, including unwrapping the npm launcher to the packaged
//! native Rust binary so PTY execution keeps an argv-only, no-shell process
//! contract.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

fn binary_from_path(name: &str, search_path: Option<&OsStr>) -> Option<PathBuf> {
    let names = {
        #[cfg(windows)]
        {
            if Path::new(name).extension().is_some() {
                vec![name.to_string()]
            } else {
                vec![format!("{name}.exe"), name.to_string()]
            }
        }
        #[cfg(not(windows))]
        {
            vec![name.to_string()]
        }
    };
    search_path
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .flat_map(|directory| {
            names
                .iter()
                .map(move |name| directory.join(name))
                .collect::<Vec<_>>()
        })
        .find(|candidate| fs::metadata(candidate).is_ok_and(|metadata| metadata.is_file()))
}

#[cfg(windows)]
fn windows_codex_launcher_from_path(name: &str, search_path: Option<&OsStr>) -> Option<PathBuf> {
    search_path
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(format!("{name}.cmd")))
        .find(|candidate| fs::metadata(candidate).is_ok_and(|metadata| metadata.is_file()))
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

fn packaged_codex_native_candidates(
    anchor: &Path,
    package: &str,
    target: &str,
    executable: &str,
) -> Vec<PathBuf> {
    let canonical_anchor = fs::canonicalize(anchor).unwrap_or_else(|_| anchor.to_path_buf());
    let mut roots = canonical_anchor
        .ancestors()
        .chain(anchor.ancestors())
        .take(24)
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    let vendor_suffix = Path::new("vendor")
        .join(target)
        .join("bin")
        .join(executable);
    let mut candidates = Vec::new();
    for root in roots {
        candidates.push(
            root.join("node_modules")
                .join("@openai")
                .join("codex")
                .join("node_modules")
                .join("@openai")
                .join(package)
                .join(&vendor_suffix),
        );
        candidates.push(
            root.join("node_modules")
                .join("@openai")
                .join(package)
                .join(&vendor_suffix),
        );
        candidates.push(
            root.join("node_modules")
                .join("@openai")
                .join("codex")
                .join(&vendor_suffix),
        );
        candidates.push(root.join("node_modules").join(package).join(&vendor_suffix));
        candidates.push(root.join(&vendor_suffix));
    }
    candidates
}

/// The npm launcher is a Node script on Unix and a `.cmd` shim on Windows.
/// Resolve its installed platform package to the native Rust executable so
/// PTY execution can keep an argv-only, no-shell process contract.
fn resolve_packaged_codex_native(anchor: &Path) -> Option<PathBuf> {
    let (package, target, executable) = packaged_codex_target()?;
    packaged_codex_native_candidates(anchor, package, target, executable)
        .into_iter()
        .find_map(|candidate| {
            let canonical = fs::canonicalize(candidate).ok()?;
            let metadata = fs::symlink_metadata(&canonical).ok()?;
            (metadata.is_file() && !metadata.file_type().is_symlink()).then_some(canonical)
        })
}

fn resolved_binary_candidate(name: &str, path: PathBuf) -> Result<PathBuf, String> {
    let candidate = if name == "codex" {
        resolve_packaged_codex_native(&path).unwrap_or(path)
    } else {
        path
    };
    let resolved = fs::canonicalize(&candidate)
        .map_err(|error| format!("resolve the `{name}` executable: {error}"))?;
    if !resolved.is_absolute() || !resolved.is_file() {
        return Err(format!(
            "the `{name}` executable is not an absolute regular file"
        ));
    }
    Ok(resolved)
}

/// Resolve a CLI without assuming a POSIX shell exists. GUI apps on Unix can
/// launch with a minimal PATH, so the inherited PATH probe is followed by a
/// login-shell lookup and the usual Unix install locations. Native Windows
/// resolves an executable directly from PATH and never invokes WSL or a
/// command shell.
pub fn find_binary(name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        || Path::new(name).components().count() != 1
    {
        return Err("binary name must be one plain path component".to_string());
    }
    let search_path = std::env::var_os("PATH");
    if let Some(path) = binary_from_path(name, search_path.as_deref()) {
        return resolved_binary_candidate(name, path);
    }
    #[cfg(windows)]
    if name == "codex" {
        if let Some(launcher) = windows_codex_launcher_from_path(name, search_path.as_deref()) {
            if let Some(native) = resolve_packaged_codex_native(&launcher) {
                return Ok(native);
            }
        }
    }
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        if let Ok(out) = std::process::Command::new(&shell)
            .args(["-lc", &format!("command -v {}", name)])
            .output()
        {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    return resolved_binary_candidate(name, PathBuf::from(path));
                }
            }
        }
        let home = crate::persistence::user_home_dir().unwrap_or_default();
        for candidate in [
            home.join(format!(".local/bin/{}", name)),
            PathBuf::from(format!("/opt/homebrew/bin/{}", name)),
            PathBuf::from(format!("/usr/local/bin/{}", name)),
        ] {
            if candidate.is_file() {
                return resolved_binary_candidate(name, candidate);
            }
        }
    }
    Err(format!(
        "could not find the `{}` binary — install it or add it to PATH",
        name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lookup_uses_the_first_installed_cli_and_reports_missing_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let search_path = std::env::join_paths([&first, &second]).unwrap();
        assert!(binary_from_path("codex", None).is_none());
        assert!(binary_from_path("codex", Some(&search_path)).is_none());
        let (_, _, executable) = packaged_codex_target().unwrap();
        fs::write(second.join(executable), b"second cli").unwrap();
        assert_eq!(
            binary_from_path("codex", Some(&search_path)),
            Some(second.join(executable))
        );
        fs::write(first.join(executable), b"first cli").unwrap();
        assert_eq!(
            resolved_binary_candidate(
                "codex",
                binary_from_path("codex", Some(&search_path)).unwrap()
            )
            .unwrap(),
            fs::canonicalize(first.join(executable)).unwrap()
        );
    }

    #[test]
    fn resolved_cli_must_be_an_absolute_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        assert!(resolved_binary_candidate("codex", temp.path().join("missing")).is_err());
        assert!(resolved_binary_candidate("codex", temp.path().to_path_buf()).is_err());
        let executable = temp.path().join("native-codex");
        fs::write(&executable, b"fixture").unwrap();
        let resolved = resolved_binary_candidate("codex", executable.clone()).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(resolved, fs::canonicalize(&executable).unwrap());
    }

    #[test]
    #[cfg(windows)]
    fn windows_npm_launcher_is_found_without_a_direct_cli() {
        let temp = tempfile::tempdir().unwrap();
        let launcher = temp.path().join("codex.cmd");
        fs::write(&launcher, b"fixture").unwrap();
        let search_path = std::env::join_paths([temp.path()]).unwrap();
        assert!(binary_from_path("codex", Some(&search_path)).is_none());
        assert_eq!(
            windows_codex_launcher_from_path("codex", Some(&search_path)),
            Some(launcher)
        );
    }

    #[test]
    fn npm_launcher_resolution_finds_the_platform_native_codex_binary() {
        let (package, target, executable) = packaged_codex_target().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("npm-prefix");
        let anchor = prefix.join("codex.cmd");
        fs::create_dir_all(&prefix).unwrap();
        fs::write(&anchor, b"launcher").unwrap();
        let native = prefix
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("node_modules")
            .join("@openai")
            .join(package)
            .join("vendor")
            .join(target)
            .join("bin")
            .join(executable);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"native codex").unwrap();

        assert_eq!(
            resolve_packaged_codex_native(&anchor),
            Some(fs::canonicalize(&native).unwrap())
        );
        assert_eq!(
            resolved_binary_candidate("codex", anchor.clone()).unwrap(),
            fs::canonicalize(&native).unwrap()
        );
        assert_eq!(
            resolved_binary_candidate("claude", anchor.clone()).unwrap(),
            fs::canonicalize(anchor).unwrap()
        );
    }
}
