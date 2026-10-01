//! Codex profile checks. Native arg0 helpers are runtime scratch, not synced state.

use mallard_infrastructure::providers::codex::history::parse_session_index_titles;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

type FileInventory = BTreeMap<PathBuf, [u8; 32]>;

pub fn inventory(home: &Path) -> FileInventory {
    let metadata = std::fs::symlink_metadata(home).expect("cannot inspect isolated Codex home");
    assert!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "isolated Codex home must be a no-follow directory"
    );
    let scratch = home.join("tmp/arg0");
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
        let scratch = home.path().join("tmp/arg0/native");
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
