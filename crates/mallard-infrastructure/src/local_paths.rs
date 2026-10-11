//! Canonical path spellings at the machine-local state boundary.
//!
//! Bindings use ordinary Windows drive paths when they are lossless. Native
//! filesystem operations can still return verbatim paths for the same root.

use std::io;
use std::path::{Path, PathBuf};

/// A lossless local spelling for display and transcript path mapping. This
/// does not resolve aliases or change the meaning of a saved path.
pub fn simplified(path: &str) -> String {
    dunce::simplified(Path::new(path))
        .to_string_lossy()
        .into_owned()
}

/// Resolve and compare bindings in the same local spelling, while keeping
/// every identity and revision field intact. This never rewrites saved state.
pub fn normalized_binding(
    binding: &mallard_domain::ProjectBinding,
) -> mallard_domain::ProjectBinding {
    let mut binding = binding.clone();
    binding.project_root = simplified(&binding.project_root);
    binding.canonical_project_root = simplified(&binding.canonical_project_root);
    binding
}

/// Resolve a path for a binding, preserving verbatim paths when simplifying
/// them would change their meaning (for example, a trailing dot in a name).
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Compare already-canonical paths without resolving the saved path again.
/// Re-resolving it could follow a newly introduced symlink and conceal a
/// changed binding. Case, component boundaries, and special names stay exact.
pub fn same_canonical_path(left: &Path, right: &Path) -> bool {
    dunce::simplified(left) == dunce::simplified(right)
}

/// Keep containment checks consistent with canonical path comparisons.
pub fn strip_prefix<'a>(
    path: &'a Path,
    root: &Path,
) -> Result<&'a Path, std::path::StripPrefixError> {
    dunce::simplified(path).strip_prefix(dunce::simplified(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_path_matches_native_resolution_but_not_a_different_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let other = temp.path().join("project-other");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&other).unwrap();
        let saved = canonicalize(&root).unwrap();
        let native = std::fs::canonicalize(&root).unwrap();
        assert!(same_canonical_path(&native, &saved));
        assert!(!same_canonical_path(
            &std::fs::canonicalize(&other).unwrap(),
            &saved,
        ));
        assert!(!same_canonical_path(&native.join("child"), &saved));
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_comparison_only_simplifies_lossless_drive_spellings() {
        assert!(same_canonical_path(
            Path::new(r"\\?\C:\Projects\mallard"),
            Path::new(r"C:\Projects\mallard"),
        ));
        assert_eq!(
            strip_prefix(
                Path::new(r"\\?\C:\Projects\mallard\sessions\chat.jsonl"),
                Path::new(r"C:\Projects\mallard"),
            )
            .unwrap(),
            Path::new(r"sessions\chat.jsonl"),
        );
        assert!(strip_prefix(
            Path::new(r"\\?\C:\Projects\mallard-other\chat.jsonl"),
            Path::new(r"C:\Projects\mallard"),
        )
        .is_err());
        for (verbatim, ordinary) in [
            (r"\\?\C:\Projects\mallard.", r"C:\Projects\mallard."),
            (r"\\?\C:\Projects\mallard ", r"C:\Projects\mallard "),
            (r"\\?\C:\Projects\NUL", r"C:\Projects\NUL"),
            (r"\\?\C:\Projects\Mallard", r"C:\Projects\mallard"),
        ] {
            assert!(!same_canonical_path(
                Path::new(verbatim),
                Path::new(ordinary)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn saved_root_is_not_resolved_again_after_symlink_retargeting() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let moved = temp.path().join("moved");
        let other = temp.path().join("other");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&other).unwrap();
        let saved = canonicalize(&root).unwrap();
        std::fs::rename(&root, &moved).unwrap();
        std::os::unix::fs::symlink(&other, &root).unwrap();
        let current = std::fs::canonicalize(&root).unwrap();
        assert!(!same_canonical_path(&current, &saved));
        assert!(!same_canonical_path(
            Path::new(r"\\?\C:\Projects\mallard"),
            Path::new(r"C:\Projects\mallard"),
        ));
    }
}
