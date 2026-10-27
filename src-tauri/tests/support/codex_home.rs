//! Codex profile checks. Native arg0 helpers are runtime scratch, not synced state.

use mallard_infrastructure::providers::codex::history::parse_session_index_titles;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

type FileInventory = BTreeMap<PathBuf, [u8; 32]>;

pub const ARG0_DIRECTORY: &str = if cfg!(target_os = "macos") {
    "arg0-macos"
} else {
    "arg0"
};

#[cfg(windows)]
fn verified_runtime_copy(path: &Path, executable: &Path) -> [u8; 32] {
    use std::io::Read as _;

    fn digest(path: &Path) -> [u8; 32] {
        const MAX_RUNTIME_BYTES: u64 = 512 * 1024 * 1024;
        let metadata = std::fs::symlink_metadata(path).expect("inspect native sandbox executable");
        assert!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= MAX_RUNTIME_BYTES,
            "native sandbox executable must be a bounded regular file"
        );
        let mut file = std::fs::File::open(path)
            .expect("open native sandbox executable")
            .take(MAX_RUNTIME_BYTES + 1);
        let mut hasher = Sha256::new();
        let count = std::io::copy(&mut file, &mut hasher).expect("hash native sandbox executable");
        assert_eq!(count, metadata.len(), "native sandbox executable changed");
        hasher.finalize().into()
    }

    let actual = digest(path);
    assert_eq!(
        actual,
        digest(executable),
        "native sandbox copy differs from the installed Codex executable"
    );
    actual
}

/// Windows stages its executable in each sandbox home. Verify the exact
/// runtime copy separately from the much smaller provider-data byte budget.
#[cfg(windows)]
pub fn native_runtime_digest(home: &Path, relative: &Path) -> Option<[u8; 32]> {
    (relative == Path::new(".sandbox-bin/codex.exe"))
        .then(|| verified_runtime_copy(&home.join(relative), &super::codex_binary()))
}

pub fn inventory(home: &Path) -> FileInventory {
    let metadata = std::fs::symlink_metadata(home).expect("cannot inspect isolated Codex home");
    assert!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "isolated Codex home must be a no-follow directory"
    );
    let scratch = home.join("tmp").join(ARG0_DIRECTORY);
    let mut inventory = BTreeMap::new();
    let mut entries_seen = 0;
    for entry in WalkDir::new(home)
        .follow_links(false)
        .max_depth(16)
        // Skip only the real scratch directory. A symlink at its root still fails.
        .into_iter()
        .filter_entry(|entry| entry.path() != scratch || !entry.file_type().is_dir())
    {
        let entry = entry.expect("cannot inventory isolated Codex home");
        entries_seen += 1;
        assert!(
            entries_seen <= 50_000,
            "isolated Codex home exceeds its entry bound"
        );
        assert!(
            !entry.file_type().is_symlink(),
            "isolated Codex home contains a symlink outside runtime scratch"
        );
        if entry.file_type().is_dir() {
            assert!(
                entry.depth() < 16,
                "isolated Codex home exceeds its depth bound"
            );
            continue;
        }
        assert!(
            entry.file_type().is_file(),
            "isolated Codex home contains a special file"
        );
        let relative = entry
            .path()
            .strip_prefix(home)
            .expect("Codex home entry escaped its root");
        #[cfg(windows)]
        if let Some(digest) = native_runtime_digest(home, relative) {
            inventory.insert(relative.to_path_buf(), digest);
            continue;
        }
        let bytes =
            super::read_bounded_file(entry.path(), 64 * 1024 * 1024, "isolated Codex home file");
        inventory.insert(relative.to_path_buf(), Sha256::digest(bytes).into());
    }
    inventory
}

#[derive(Debug, PartialEq, Eq)]
pub struct ConversationInventory {
    transcripts: FileInventory,
    titles: BTreeMap<String, String>,
}

pub fn conversations(home: &Path) -> ConversationInventory {
    let files = inventory(home);
    let index = home.join("session_index.jsonl");
    let titles = if files.contains_key(Path::new("session_index.jsonl")) {
        parse_session_index_titles(&super::read_bounded_file(
            &index,
            64 * 1024 * 1024,
            "isolated Codex session index",
        ))
    } else {
        BTreeMap::new()
    };
    ConversationInventory {
        transcripts: files
            .into_iter()
            .filter(|(path, _)| {
                path.starts_with("sessions") || path.starts_with("archived_sessions")
            })
            .collect(),
        titles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(windows)]
    #[test]
    fn native_sandbox_runtime_copy_must_match_the_installed_executable() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("installed.exe");
        let copied = root.path().join("copy.exe");
        fs::write(&executable, b"native executable").unwrap();
        fs::copy(&executable, &copied).unwrap();
        assert_eq!(
            verified_runtime_copy(&copied, &executable),
            <[u8; 32]>::from(Sha256::digest(b"native executable")),
        );
        fs::write(&copied, b"forged executable").unwrap();
        assert!(std::panic::catch_unwind(|| verified_runtime_copy(&copied, &executable)).is_err());
        assert!(native_runtime_digest(root.path(), Path::new("sessions/codex.exe")).is_none());
        assert!(native_runtime_digest(root.path(), Path::new(".sandbox-bin/other.exe")).is_none());
        let directory = root.path().join("directory.exe");
        fs::create_dir(&directory).unwrap();
        assert!(
            std::panic::catch_unwind(|| verified_runtime_copy(&directory, &executable)).is_err()
        );
    }

    fn fixture() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("sessions/2026/09/30")).unwrap();
        fs::write(
            home.path().join("sessions/2026/09/30/rollout.jsonl"),
            b"unchanged transcript\n",
        )
        .unwrap();
        fs::write(
            home.path().join("config.toml"),
            b"unchanged configuration\n",
        )
        .unwrap();
        fs::write(home.path().join("session_index.jsonl"), b"{\"id\":\"fixture\",\"thread_name\":\"Expected title\",\"updated_at\":\"2026-09-30T00:00:00Z\"}\n").unwrap();
        home
    }

    #[cfg(unix)]
    #[test]
    fn native_arg0_symlinks_are_excluded_only_from_the_home_inventory() {
        let home = fixture();
        let before = inventory(home.path());
        let scratch = home.path().join("tmp").join(ARG0_DIRECTORY).join("native");
        fs::create_dir_all(&scratch).unwrap();
        // A dangling link proves the inventory does not read its target.
        std::os::unix::fs::symlink(
            home.path().join("missing-target"),
            scratch.join("apply_patch"),
        )
        .unwrap();
        assert_eq!(inventory(home.path()), before);
        assert!(
            std::panic::catch_unwind(|| super::super::storage_digest_inventory(home.path()))
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn home_inventory_rejects_symlinks_outside_scratch_and_special_files() {
        for relative in [
            "sessions/linked.jsonl",
            "tmp/arg0",
            "tmp/arg0-macos",
            "tmp/other",
            "session_index.jsonl",
        ] {
            let home = fixture();
            let link = home.path().join(relative);
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            if link.is_file() {
                fs::remove_file(&link).unwrap();
            }
            std::os::unix::fs::symlink(home.path().join("missing-target"), &link).unwrap();
            assert!(std::panic::catch_unwind(|| inventory(home.path())).is_err());
        }
        let home = fixture();
        let _socket = std::os::unix::net::UnixListener::bind(home.path().join("socket")).unwrap();
        assert!(std::panic::catch_unwind(|| inventory(home.path())).is_err());
    }

    #[test]
    fn native_arg0_wrapper_requires_the_current_namespace_and_known_helper() {
        let directory = Path::new("tmp")
            .join(ARG0_DIRECTORY)
            .join("codex-arg0Fixture");
        for name in ["apply_patch", "applypatch", "codex-execve-wrapper"] {
            assert!(super::super::is_generated_arg0_wrapper(&directory.join(name)));
        }
        for name in ["other", "nested/apply_patch"] {
            assert!(!super::super::is_generated_arg0_wrapper(&directory.join(name)));
        }
        let other_namespace = if cfg!(target_os = "macos") {
            "arg0"
        } else {
            "arg0-macos"
        };
        let relative = Path::new("tmp")
            .join(other_namespace)
            .join("codex-arg0Fixture/apply_patch");
        assert!(!super::super::is_generated_arg0_wrapper(&relative));
    }

    #[test]
    fn conversation_checks_preserve_content_and_names_across_native_metadata_updates() {
        let home = fixture();
        let before_clear = inventory(home.path());
        let before_pull = conversations(home.path());
        fs::write(home.path().join("state_5.sqlite"), b"native metadata").unwrap();
        let mut index = fs::read(home.path().join("session_index.jsonl")).unwrap();
        index.extend_from_slice(b"{\"id\":\"fixture\",\"thread_name\":\"Expected title\",\"updated_at\":\"2026-09-30T00:00:01Z\"}\n");
        fs::write(home.path().join("session_index.jsonl"), &index).unwrap();
        assert_ne!(inventory(home.path()), before_clear);
        assert_eq!(conversations(home.path()), before_pull);
        fs::write(
            home.path().join("sessions/2026/09/30/rollout.jsonl"),
            b"changed transcript\n",
        )
        .unwrap();
        assert_ne!(conversations(home.path()), before_pull);
        fs::write(
            home.path().join("sessions/2026/09/30/rollout.jsonl"),
            b"unchanged transcript\n",
        )
        .unwrap();
        index.extend_from_slice(b"{\"id\":\"fixture\",\"thread_name\":\"Wrong title\",\"updated_at\":\"2026-09-30T00:00:02Z\"}\n");
        fs::write(home.path().join("session_index.jsonl"), &index).unwrap();
        assert_ne!(conversations(home.path()), before_pull);
    }
}
