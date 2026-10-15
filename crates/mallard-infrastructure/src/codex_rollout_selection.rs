//! Filename-based physical identity and current selection for Codex rollouts.
//!
//! Mallard trusts native filename order. It does not consult Codex's database.
//! A rollout Codex compressed in place (`X.jsonl.zst`) is the rollout
//! `X.jsonl`: Codex names it by the plain name, and so does every rule here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::jsonl_tail_io::is_compressed;

/// The rollout's name without Codex's `.zst` suffix: `Some` for `*.jsonl`
/// and `*.jsonl.zst`, `None` for any other file. This is the one test for
/// "is this a rollout file" and the name every comparison uses.
pub(crate) fn plain_name(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    let name = name.strip_suffix(".zst").unwrap_or(name);
    name.ends_with(".jsonl").then_some(name)
}

/// The path under the plain name: the file Codex writes to, and the only
/// name Mallard ever writes.
pub(crate) fn plain_path(path: &Path) -> PathBuf {
    match plain_name(path) {
        Some(name) if is_compressed(path) => path.with_file_name(name),
        _ => path.to_path_buf(),
    }
}

/// The compressed rollouts in `paths` whose plain sibling is also listed.
/// Codex leaves both after an interrupted compression and reads the plain
/// file; so does Mallard, and a listing drops these.
pub(crate) fn hidden_siblings<'a>(paths: impl Iterator<Item = &'a Path>) -> BTreeSet<PathBuf> {
    let mut plain = BTreeSet::new();
    let mut compressed = Vec::new();
    for path in paths {
        if is_compressed(path) {
            compressed.push(path);
        } else {
            plain.insert(path);
        }
    }
    compressed
        .into_iter()
        .filter(|path| plain.contains(plain_path(path).as_path()))
        .map(Path::to_path_buf)
        .collect()
}

fn native_ids(path: &Path) -> Option<(&str, &str)> {
    let core = plain_name(path)?
        .strip_prefix("rollout-")?
        .strip_suffix(".jsonl")?;
    let date = core.get(..19)?;
    if !date.bytes().enumerate().all(|(i, c)| match i {
        4 | 7 | 13 | 16 => c == b'-',
        10 => c == b'T',
        _ => c.is_ascii_digit(),
    }) || core.as_bytes().get(19) != Some(&b'-')
    {
        return None;
    }
    let ids = core.get(20..)?;
    Some(ids.split_once('_').unwrap_or((ids, ids)))
}

/// The thread a native rollout file name claims; `None` for any other name.
pub(crate) fn logical_id(path: &Path) -> Option<&str> {
    native_ids(path).map(|(logical, _)| logical)
}

pub(crate) fn select_current(
    thread_id: &str,
    candidates: &[PathBuf],
) -> Result<Option<PathBuf>, String> {
    let mut selected: Option<&PathBuf> = None;
    for path in candidates {
        physical_id(path, thread_id)?;
        if candidates.len() > 1 && native_ids(path).is_none() {
            return Err("Codex current rollout has ambiguous filenames".into());
        }
        match selected {
            None => selected = Some(path),
            Some(previous) => match plain_name(path).cmp(&plain_name(previous)) {
                std::cmp::Ordering::Greater => selected = Some(path),
                std::cmp::Ordering::Equal => {
                    return Err("Codex current rollout has duplicate filenames".into())
                }
                std::cmp::Ordering::Less => {}
            },
        }
    }
    Ok(selected.cloned())
}

/// The header identifies the thread; the native suffix identifies the file.
/// Standalone decoding and installed bindings can use non-native filenames.
pub(crate) fn physical_id(path: &Path, thread_id: &str) -> Result<String, String> {
    let Some((logical, physical)) = native_ids(path) else {
        return Ok(thread_id.into());
    };
    if logical != thread_id || physical.is_empty() || physical.contains('_') {
        return Err("Codex rollout filename disagrees with its thread identity".into());
    }
    Ok(physical.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_subagents::RolloutLookup;

    #[test]
    fn filename_order_selects_current_and_exact_lookup_keeps_history() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let original = sessions.join("rollout-2026-09-12T01-03-07-thread.jsonl");
        let reverted = sessions.join("rollout-2026-09-13T00-19-15-thread_rollout-1.jsonl");
        std::fs::write(&reverted, b"current").unwrap();
        // Write the old file last: modification time must not select it.
        std::fs::write(&original, b"old").unwrap();
        let lookup = RolloutLookup::new(home.path());
        assert_eq!(
            lookup.find_current("thread").unwrap(),
            Some(reverted.clone())
        );
        assert_eq!(lookup.find_rollout("thread").unwrap(), Some(original));
        assert_eq!(
            lookup.find_rollout("rollout-1").unwrap(),
            Some(reverted.clone())
        );
        lookup.verify_current("thread", &reverted).unwrap();
        let next = sessions.join("rollout-2026-09-13T00-19-16-thread_rollout-2.jsonl");
        std::fs::write(next, b"next").unwrap();
        assert!(lookup.verify_current("thread", &reverted).is_err());
    }

    #[test]
    fn duplicate_filenames_and_header_mismatches_are_blocked() {
        let filename = "rollout-2026-09-13T00-19-15-thread_rollout-1.jsonl";
        assert!(select_current(
            "thread",
            &[
                PathBuf::from("sessions").join(filename),
                PathBuf::from("archived_sessions").join(filename)
            ]
        )
        .is_err());
        assert!(physical_id(Path::new(filename), "other").is_err());
        assert!(physical_id(
            Path::new("rollout-2026-09-13T00-19-15-other.jsonl"),
            "thread"
        )
        .is_err());
        assert_eq!(
            physical_id(Path::new(filename), "thread").unwrap(),
            "rollout-1"
        );
    }

    /// A rollout Codex compressed in place keeps its name, its ids, and its
    /// place in the order; beside its plain sibling it is hidden; the same
    /// plain name in two roots is still a duplicate.
    #[test]
    fn compressed_rollouts_are_named_by_their_plain_name() {
        let plain = Path::new("sessions/rollout-2026-09-12T01-03-07-thread.jsonl");
        let compressed = Path::new("sessions/rollout-2026-09-12T01-03-07-thread.jsonl.zst");
        assert_eq!(plain_name(compressed), plain_name(plain));
        assert_eq!(logical_id(compressed), Some("thread"));
        assert_eq!(physical_id(compressed, "thread").unwrap(), "thread");
        assert_eq!(plain_path(compressed), plain);
        assert_eq!(plain_path(plain), plain);
        assert_eq!(plain_name(Path::new("sessions/notes.zst")), None);
        assert_eq!(
            plain_name(Path::new("sessions/rollout-thread.jsonl.gz")),
            None
        );
        assert!(!crate::jsonl_tail_io::is_compressed(Path::new(
            "rollout.zst"
        )));

        let older = PathBuf::from("rollout-2026-09-12T01-03-07-thread.jsonl.zst");
        let newer = PathBuf::from("rollout-2026-09-13T00-19-15-thread_rollout-1.jsonl");
        assert_eq!(
            select_current("thread", &[newer.clone(), older]).unwrap(),
            Some(newer)
        );
        assert!(select_current(
            "thread",
            &[
                PathBuf::from("sessions").join("rollout-2026-09-12T01-03-07-thread.jsonl"),
                PathBuf::from("archived_sessions")
                    .join("rollout-2026-09-12T01-03-07-thread.jsonl.zst"),
            ]
        )
        .is_err());

        let other = Path::new("sessions/rollout-2026-09-12T01-03-08-other.jsonl.zst");
        assert_eq!(
            hidden_siblings([plain, compressed, other].into_iter()),
            BTreeSet::from([compressed.to_path_buf()])
        );
        assert!(hidden_siblings([compressed, other].into_iter()).is_empty());
    }
}
