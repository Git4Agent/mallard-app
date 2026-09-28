use std::{fs, io::ErrorKind, path::Path};

use crate::providers::DiscoveredSession;

/// User-approved best effort: the SDK's file scope, without a Node sidecar.
/// Unlike the SDK, still attempt child cleanup after a root-file failure.
pub(super) fn delete(home: &Path, session: &DiscoveredSession) -> Result<(), String> {
    let path = &session.physical_path;
    let expected = format!("{}.jsonl", session.session_id);
    if !subagents_id_is_safe(&session.session_id)
        || path.file_name().and_then(|name| name.to_str()) != Some(expected.as_str())
        || !path.starts_with(home.join("projects"))
    {
        return Err("Claude session path does not match its identity.".into());
    }
    let parent = path.parent().ok_or("Claude session has no directory.")?;
    // Keep basic path addressing checks; there is intentionally no digest gate.
    crate::persistence::ensure_no_symlinks(home, path)?;
    let children = parent.join(&session.session_id);
    crate::persistence::ensure_no_symlinks(home, &children)?;
    let mut errors = Vec::new();
    for (label, result) in [
        ("transcript", fs::remove_file(path)),
        ("subagent directory", fs::remove_dir_all(&children)),
    ] {
        if let Err(error) = result {
            if error.kind() != ErrorKind::NotFound {
                errors.push(format!("Could not delete Claude {label}: {}", error.kind()));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" "))
    }
}

fn subagents_id_is_safe(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_removes_session_tree_and_keeps_parent_and_sidecars() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let bucket = home.join("projects/project");
        let children = bucket.join("selected/subagents/nested");
        fs::create_dir_all(&children).unwrap();
        fs::write(bucket.join("selected.jsonl"), b"selected").unwrap();
        fs::write(bucket.join("parent.jsonl"), b"parent").unwrap();
        fs::write(bucket.join("fork.jsonl"), b"fork").unwrap();
        fs::write(children.join("agent-child.jsonl"), b"child").unwrap();
        fs::create_dir_all(home.join("file-history/selected")).unwrap();
        fs::write(home.join("file-history/selected/file"), b"sidecar").unwrap();
        let session = DiscoveredSession {
            session_id: "selected".into(),
            physical_path: bucket.join("selected.jsonl"),
            display_name: "Selected".into(),
            blocked_reason: None,
        };
        delete(home, &session).unwrap();
        assert!(!session.physical_path.exists());
        assert!(!bucket.join("selected").exists());
        assert_eq!(fs::read(bucket.join("parent.jsonl")).unwrap(), b"parent");
        assert_eq!(fs::read(bucket.join("fork.jsonl")).unwrap(), b"fork");
        assert!(home.join("file-history/selected/file").exists());
        // Missing root does not prevent orphan-child cleanup or repeat deletion.
        fs::create_dir_all(&children).unwrap();
        fs::write(children.join("orphan.jsonl"), b"orphan").unwrap();
        delete(home, &session).unwrap();
        delete(home, &session).unwrap();
        assert!(!bucket.join("selected").exists());
    }

    #[test]
    fn root_failure_does_not_skip_children() {
        let temp = tempfile::tempdir().unwrap();
        let bucket = temp.path().join("projects/project");
        fs::create_dir_all(bucket.join("selected.jsonl")).unwrap();
        fs::create_dir_all(bucket.join("selected/subagents")).unwrap();
        let session = DiscoveredSession {
            session_id: "selected".into(),
            physical_path: bucket.join("selected.jsonl"),
            display_name: "Selected".into(),
            blocked_reason: None,
        };
        assert!(delete(temp.path(), &session).is_err());
        assert!(!bucket.join("selected").exists());
    }
}
