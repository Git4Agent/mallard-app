//! Filename-based physical identity and current selection for Codex rollouts.
//!
//! Mallard trusts native filename order. It does not consult Codex's database.

use std::path::{Path, PathBuf};

fn native_ids(path: &Path) -> Option<(&str, &str)> {
    let core = path
        .file_name()?
        .to_str()?
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
            Some(previous) => match path.file_name().cmp(&previous.file_name()) {
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
}
