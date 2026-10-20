//! Codex Desktop's Windows runtime. MSIX resources cannot be executed outside
//! their package context, so Desktop extracts these four files into LocalAppData.
//! Its directory key hashes the ordered filenames and their SHA-256 digests.
//! Resolve only an existing, complete copy of the currently registered app.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};

const RUNTIME_FILES: [&str; 4] = [
    "codex.exe",
    "codex-code-mode-host.exe",
    "codex-windows-sandbox-setup.exe",
    "codex-command-runner.exe",
];
const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;

pub(super) fn installation_dir() -> Option<PathBuf> {
    let app = crate::providers::windows_app::scheme_handler("codex").ok()?;
    installation_dir_for(&app.executable, app.is_packaged(), &dirs::data_local_dir()?)
}

/// Desktop's runtime has no standalone CLI package for the shared daemon.
/// Its interactive CLI supports running an embedded server instead.
pub(super) fn requires_no_daemon(executable: &Path) -> bool {
    if let Some(local) = dirs::data_local_dir() {
        if is_extracted_executable(executable, &local) {
            return true;
        }
    }
    let Ok(app) = crate::providers::windows_app::scheme_handler("codex") else {
        return false;
    };
    app.executable.parent().is_some_and(|dir| {
        dunce::simplified(executable) == dunce::simplified(&dir.join("resources/codex.exe"))
    })
}

fn is_extracted_executable(executable: &Path, local: &Path) -> bool {
    let executable = dunce::simplified(executable);
    let Some(build) = executable.parent() else {
        return false;
    };
    executable
        .file_name()
        .is_some_and(|name| name == "codex.exe")
        && build.parent() == Some(local.join("OpenAI/Codex/bin").as_path())
        && build
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.len() == 16
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            })
}

fn installation_dir_for(executable: &Path, packaged: bool, local: &Path) -> Option<PathBuf> {
    let resources = executable.parent()?.join("resources");
    let hashes = runtime_hashes(&resources)?;
    if !packaged {
        return fs::canonicalize(resources).ok();
    }
    if !local.is_absolute() {
        return None;
    }
    let runtime = local.join("OpenAI/Codex/bin").join(runtime_key(&hashes));
    // Do not pick an unrelated cached version, a staging directory, or a
    // partially extracted runtime. Desktop alone owns extraction and updates.
    (runtime_hashes(&runtime)? == hashes)
        .then(|| fs::canonicalize(runtime).ok())
        .flatten()
}

fn runtime_key(hashes: &[[u8; 32]]) -> String {
    let mut key = Sha256::new();
    for (name, hash) in RUNTIME_FILES.iter().zip(hashes) {
        let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
        key.update(format!("{name}\0{hex}\0"));
    }
    format!("{:x}", key.finalize())[..16].to_string()
}

fn runtime_hashes(dir: &Path) -> Option<Vec<[u8; 32]>> {
    if !dir.is_absolute()
        || dunce::simplified(&fs::canonicalize(dir).ok()?) != dunce::simplified(dir)
    {
        return None;
    }
    RUNTIME_FILES
        .iter()
        .map(|name| binary_hash(&dir.join(name)))
        .collect()
}

fn binary_hash(path: &Path) -> Option<[u8; 32]> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || dunce::simplified(&fs::canonicalize(path).ok()?) != dunce::simplified(path)
    {
        return None;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || metadata.len() == 0
        || metadata.len() > MAX_BINARY_BYTES
    {
        return None;
    }
    let mut reader = file.take(MAX_BINARY_BYTES + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0;
    loop {
        let count = reader.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        total += count as u64;
        hash.update(&buffer[..count]);
    }
    (total == metadata.len()).then(|| hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(dir: &Path, build: &str) {
        fs::create_dir_all(dir).unwrap();
        for name in RUNTIME_FILES {
            fs::write(dir.join(name), format!("{build}/{name}")).unwrap();
        }
    }

    fn extracted(resources: &Path, local: &Path) -> PathBuf {
        let dest = local
            .join("OpenAI/Codex/bin")
            .join(runtime_key(&runtime_hashes(resources).unwrap()));
        fs::create_dir_all(&dest).unwrap();
        for name in RUNTIME_FILES {
            fs::copy(resources.join(name), dest.join(name)).unwrap();
        }
        dest
    }

    #[test]
    fn packaged_apps_use_only_the_complete_matching_extracted_build() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("package/app/ChatGPT.exe");
        let resources = app.parent().unwrap().join("resources");
        let local = temp.path().join("local");
        runtime(&resources, "first");
        assert_eq!(installation_dir_for(&app, true, &local), None);
        let first = extracted(&resources, &local);
        assert_eq!(
            installation_dir_for(&app, true, &local),
            Some(fs::canonicalize(&first).unwrap())
        );
        runtime(&resources, "updated");
        assert_eq!(installation_dir_for(&app, true, &local), None);
        let current = extracted(&resources, &local);
        assert_eq!(
            installation_dir_for(&app, true, &local),
            Some(fs::canonicalize(&current).unwrap())
        );
        fs::write(current.join(RUNTIME_FILES[1]), b"mixed version").unwrap();
        assert_eq!(installation_dir_for(&app, true, &local), None);
        extracted(&resources, &local);
        fs::remove_file(current.join(RUNTIME_FILES[3])).unwrap();
        assert_eq!(installation_dir_for(&app, true, &local), None);
    }

    #[test]
    fn unpackaged_apps_use_complete_bundled_runtime_and_reject_relative_roots() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("app/ChatGPT.exe");
        let resources = app.parent().unwrap().join("resources");
        runtime(&resources, "build");
        assert_eq!(
            installation_dir_for(&app, false, temp.path()),
            Some(fs::canonicalize(&resources).unwrap())
        );
        assert_eq!(installation_dir_for(&app, true, Path::new(".")), None);
        assert_eq!(
            installation_dir_for(Path::new("app/ChatGPT.exe"), false, temp.path()),
            None
        );
        fs::write(resources.join(RUNTIME_FILES[0]), b"").unwrap();
        assert_eq!(installation_dir_for(&app, false, temp.path()), None);
    }

    #[test]
    fn redirected_resources_and_extracted_runtimes_are_rejected() {
        use std::os::windows::process::CommandExt;
        for redirect_cache in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let app = temp.path().join("app/ChatGPT.exe");
            let resources = app.parent().unwrap().join("resources");
            let local = temp.path().join("local");
            runtime(&resources, "build");
            let cached = extracted(&resources, &local);
            let redirected = if redirect_cache { &cached } else { &resources };
            let outside = temp.path().join("outside");
            fs::rename(redirected, &outside).unwrap();
            let result = std::process::Command::new("cmd.exe")
                .args(["/c", "mklink", "/J"])
                .arg(redirected.to_string_lossy().replace('/', "\\"))
                .arg(outside.to_string_lossy().replace('/', "\\"))
                .creation_flags(0x0800_0000)
                .output()
                .unwrap();
            assert!(result.status.success(), "{result:?}");
            assert_eq!(installation_dir_for(&app, true, &local), None);
            fs::remove_dir(redirected).unwrap();
        }
    }

    #[test]
    #[ignore = "requires installed Codex Desktop runtime and PATH without standalone CLIs; uses a temporary home"]
    fn installed_desktop_cli_resolves_and_serves_an_isolated_profile() {
        use crate::providers::traits::ProviderCli;
        let dir = installation_dir().expect("Desktop must have extracted its runtime");
        let cli = super::super::cli::resolve(None).unwrap();
        assert_eq!(cli.executable, dir.join("codex.exe"));
        assert!(requires_no_daemon(&cli.executable));
        assert_eq!(
            cli.source,
            crate::cli_discovery::CliSelectionSource::StandardLocation
        );
        let version = super::super::cli::CodexCli.probe_version(&cli).unwrap();
        let home = tempfile::tempdir().unwrap();
        let result = super::super::app_server::request(
            &cli.executable,
            Some(&cli.search_path),
            home.path(),
            "thread/list",
            serde_json::json!({"limit": 1}),
            std::time::Duration::from_secs(20),
        )
        .unwrap();
        assert_eq!(result["data"], serde_json::json!([]));
        println!("Desktop Codex {version}: version and isolated app-server request passed");
    }

    #[test]
    fn embedded_server_mode_is_limited_to_the_desktop_runtime_layout() {
        let local = Path::new(r"C:\Users\dev\AppData\Local");
        assert!(is_extracted_executable(
            &local.join("OpenAI/Codex/bin/0123456789abcdef/codex.exe"),
            local,
        ));
        for path in [
            local.join("npm/codex.exe"),
            local.join("OpenAI/Codex/bin/staging/codex.exe"),
            local.join("OpenAI/Codex/bin/0123456789abcdef/other.exe"),
            PathBuf::from(r"C:\Other\OpenAI\Codex\bin\0123456789abcdef\codex.exe"),
        ] {
            assert!(!is_extracted_executable(&path, local));
        }
    }
}
