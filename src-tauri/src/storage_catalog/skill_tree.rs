//! Bounded Agent Skill directory handling: capture, validation, semantic
//! rename, materialization, and verification.
//!
//! Capture preserves the exact validated tree; it never rewrites frontmatter
//! to convert a skill between providers. The one deliberate transform is the
//! reviewed copy rename, which changes the directory name and the `SKILL.md`
//! frontmatter `name` together so the copy stays a valid, independently
//! addressable Agent Skill.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use super::domain::{
    validate_skill_name, validate_skill_relative_path, AgentSkillDescriptor, AgentSkillResource,
    AgentSkillScope, CatalogFileEntry, MAX_SKILL_FILES, MAX_SKILL_FILE_BYTES, MAX_SKILL_TREE_BYTES,
};

pub const MAX_SKILL_MD_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedSkillFile {
    pub bytes: Vec<u8>,
    /// Portable safe mode: `0o755` for executables, `0o644` otherwise.
    pub mode: u32,
}

#[derive(Clone, Debug, Default)]
pub struct SkillTreeWarnings {
    pub executable_paths: Vec<String>,
    pub secret_shaped_paths: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CapturedSkillTree {
    pub files: BTreeMap<String, CapturedSkillFile>,
    pub declared_name: Option<String>,
    pub warnings: SkillTreeWarnings,
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{:02x}", byte);
    }
    output
}

fn sha256(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

/// The `name:` declaration in SKILL.md's leading YAML frontmatter block.
pub fn declared_skill_name(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let value = value.trim().trim_matches(['"', '\'']);
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn is_executable(mode_bits: u32, bytes: &[u8]) -> bool {
    mode_bits & 0o111 != 0 || bytes.starts_with(b"#!")
}

fn looks_secret_shaped(bytes: &[u8]) -> bool {
    const MARKERS: [&str; 7] = [
        "api_key",
        "apikey",
        "secret_access_key",
        "private key-----",
        "-----begin",
        "bearer ",
        "authorization:",
    ];
    if bytes.len() > MAX_SKILL_FILE_BYTES as usize || bytes.contains(&0) {
        return false;
    }
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    MARKERS.iter().any(|marker| lower.contains(marker))
}

fn read_stable(path: &Path, label: &str, max_bytes: u64) -> Result<(Vec<u8>, u32), String> {
    let before =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {}: {}", label, error))?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(format!("{} is not a regular file", label));
    }
    if before.len() > max_bytes {
        return Err(format!("{} exceeds its size limit", label));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("open {}: {}", label, error))?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {}: {}", label, error))?;
    let after =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {}: {}", label, error))?;
    if after.len() != before.len() || bytes.len() as u64 != before.len() {
        return Err(format!("{} changed while being captured", label));
    }
    #[cfg(unix)]
    let mode_bits = {
        use std::os::unix::fs::PermissionsExt;
        before.permissions().mode()
    };
    #[cfg(not(unix))]
    let mode_bits = 0_u32;
    Ok((bytes, mode_bits))
}

/// Capture one immediate skill directory as an exact bounded tree. Rejects
/// symlinks, special files, unsafe components, nested VCS metadata, case
/// collisions, oversized content, and unstable reads.
pub fn capture_skill_tree(directory: &Path) -> Result<CapturedSkillTree, String> {
    let mut files = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut total: u64 = 0;
    let mut pending = vec![(directory.to_path_buf(), String::new())];
    while let Some((current, prefix)) = pending.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("read skill directory '{}': {}", current.display(), error))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("walk skill directory: {}", error))?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| "skill file name is not UTF-8".to_string())?
                .to_string();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };
            validate_skill_relative_path(&relative)?;
            let meta = entry
                .path()
                .symlink_metadata()
                .map_err(|error| format!("inspect '{}': {}", relative, error))?;
            if meta.file_type().is_symlink() {
                return Err(format!("skill contains a symlink at '{}'", relative));
            }
            if meta.is_dir() {
                pending.push((entry.path(), relative));
                continue;
            }
            if !meta.is_file() {
                return Err(format!("skill contains a special file at '{}'", relative));
            }
            if !folded.insert(relative.to_lowercase()) {
                return Err(format!(
                    "skill has a case-folded path collision at '{}'",
                    relative
                ));
            }
            if files.len() >= MAX_SKILL_FILES {
                return Err("skill exceeds its file-count limit".to_string());
            }
            let (bytes, mode_bits) = read_stable(
                &entry.path(),
                &format!("skill file '{}'", relative),
                MAX_SKILL_FILE_BYTES,
            )?;
            total = total.saturating_add(bytes.len() as u64);
            if total > MAX_SKILL_TREE_BYTES {
                return Err("skill exceeds its tree byte limit".to_string());
            }
            let mode = if is_executable(mode_bits, &bytes) {
                0o755
            } else {
                0o644
            };
            files.insert(relative, CapturedSkillFile { bytes, mode });
        }
    }

    let skill_md = files
        .get("SKILL.md")
        .ok_or_else(|| "no SKILL.md declaration; not a recognizable skill".to_string())?;
    if skill_md.bytes.len() as u64 > MAX_SKILL_MD_BYTES {
        return Err("SKILL.md exceeds the supported size".to_string());
    }
    let declared_name = std::str::from_utf8(&skill_md.bytes)
        .ok()
        .and_then(declared_skill_name);
    if let Some(declared) = &declared_name {
        validate_skill_name("declared skill name", declared)?;
    }

    let mut warnings = SkillTreeWarnings::default();
    for (path, file) in &files {
        if file.mode == 0o755 {
            warnings.executable_paths.push(path.clone());
        }
        if looks_secret_shaped(&file.bytes) {
            warnings.secret_shaped_paths.push(path.clone());
        }
    }

    Ok(CapturedSkillTree {
        files,
        declared_name,
        warnings,
    })
}

/// Build the portable resource for a captured tree.
pub fn skill_resource_from_tree(
    scope: AgentSkillScope,
    effective_name: &str,
    install_directory: &str,
    files: &BTreeMap<String, CapturedSkillFile>,
) -> Result<AgentSkillResource, String> {
    let mut entries = BTreeMap::new();
    for (path, file) in files {
        let digest = sha256(&file.bytes);
        entries.insert(
            path.clone(),
            CatalogFileEntry {
                object_key: format!("_objects/{}", digest),
                sha256: digest,
                size: file.bytes.len() as u64,
                mode: file.mode,
            },
        );
    }
    let mut resource = AgentSkillResource {
        descriptor: AgentSkillDescriptor {
            scope,
            effective_name: effective_name.to_string(),
            install_directory: install_directory.to_string(),
        },
        content_sha256: String::new(),
        files: entries,
    };
    resource.content_sha256 = resource.compute_content_sha256();
    resource.validate()?;
    Ok(resource)
}

/// The reviewed copy transform: a new directory identity plus a rewritten
/// frontmatter `name` when one is declared, preserving every other byte. A
/// A skill without a declaration keeps its bytes; the directory rename
/// is its whole identity change.
pub fn rename_skill_tree(
    files: &BTreeMap<String, CapturedSkillFile>,
    new_name: &str,
) -> Result<BTreeMap<String, CapturedSkillFile>, String> {
    validate_skill_name("fork copy name", new_name)?;
    let mut renamed = files.clone();
    let Some(skill_md) = renamed.get_mut("SKILL.md") else {
        return Err("skill copy source has no SKILL.md".to_string());
    };
    let contents = std::str::from_utf8(&skill_md.bytes)
        .map_err(|_| "SKILL.md is not UTF-8; cannot rewrite its name".to_string())?;
    if declared_skill_name(contents).is_some() {
        let mut rewritten = String::with_capacity(contents.len());
        let mut inside_frontmatter = false;
        let mut frontmatter_closed = false;
        let mut replaced = false;
        for (index, line) in contents.lines().enumerate() {
            let trimmed = line.trim();
            if index == 0 && trimmed == "---" {
                inside_frontmatter = true;
            } else if inside_frontmatter && !frontmatter_closed && trimmed == "---" {
                frontmatter_closed = true;
            } else if inside_frontmatter
                && !frontmatter_closed
                && !replaced
                && trimmed.starts_with("name:")
            {
                let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
                rewritten.push_str(&format!("{}name: {}\n", indent, new_name));
                replaced = true;
                continue;
            }
            rewritten.push_str(line);
            rewritten.push('\n');
        }
        if !contents.ends_with('\n') {
            rewritten.pop();
        }
        skill_md.bytes = rewritten.into_bytes();
    }
    Ok(renamed)
}

/// Write a captured tree into a staging directory with portable modes.
pub fn materialize_skill_tree(
    stage: &Path,
    files: &BTreeMap<String, CapturedSkillFile>,
) -> Result<(), String> {
    for (relative, file) in files {
        validate_skill_relative_path(relative)?;
        let destination = stage.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create '{}': {}", parent.display(), error))?;
        }
        fs::write(&destination, &file.bytes)
            .map_err(|error| format!("write '{}': {}", destination.display(), error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(file.mode))
                .map_err(|error| format!("set mode on '{}': {}", destination.display(), error))?;
        }
    }
    Ok(())
}

/// Verify an on-disk directory matches an exact captured tree byte for byte.
pub fn verify_tree_matches(
    directory: &Path,
    files: &BTreeMap<String, CapturedSkillFile>,
) -> Result<(), String> {
    let observed = capture_skill_tree(directory)?;
    if observed.files.len() != files.len() {
        return Err(format!(
            "'{}' does not match its verified tree",
            directory.display()
        ));
    }
    for (path, expected) in files {
        let Some(actual) = observed.files.get(path) else {
            return Err(format!(
                "'{}' is missing '{}' after activation",
                directory.display(),
                path
            ));
        };
        if actual.bytes != expected.bytes {
            return Err(format!(
                "'{}' differs at '{}' after activation",
                directory.display(),
                path
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn capture_preserves_exact_bytes_and_detects_warnings() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("review");
        write(&dir.join("SKILL.md"), b"---\nname: review\n---\nBody\n");
        write(&dir.join("scripts/run.sh"), b"#!/bin/sh\necho ok\n");
        write(&dir.join("notes.txt"), b"api_key = secret\n");
        let tree = capture_skill_tree(&dir).unwrap();
        assert_eq!(tree.declared_name.as_deref(), Some("review"));
        assert_eq!(tree.files.len(), 3);
        assert_eq!(tree.warnings.executable_paths, vec!["scripts/run.sh"]);
        assert_eq!(tree.warnings.secret_shaped_paths, vec!["notes.txt"]);
        assert_eq!(tree.files["scripts/run.sh"].mode, 0o755);
    }

    #[test]
    fn symlinks_and_nested_vcs_metadata_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("skill");
        write(&dir.join("SKILL.md"), b"---\nname: skill\n---\n");
        write(&dir.join(".git/config"), b"[core]\n");
        assert!(capture_skill_tree(&dir).unwrap_err().contains("invalid"));
        fs::remove_dir_all(dir.join(".git")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", dir.join("link")).unwrap();
            assert!(capture_skill_tree(&dir).unwrap_err().contains("symlink"));
        }
    }

    #[test]
    fn rename_rewrites_directory_and_frontmatter_together() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("review");
        write(
            &dir.join("SKILL.md"),
            b"---\nname: review\ndescription: keep\n---\nBody stays byte-identical.\n",
        );
        write(&dir.join("extra.md"), b"unchanged");
        let tree = capture_skill_tree(&dir).unwrap();
        let renamed = rename_skill_tree(&tree.files, "review-local-copy").unwrap();
        let skill_md = String::from_utf8(renamed["SKILL.md"].bytes.clone()).unwrap();
        assert!(skill_md.contains("name: review-local-copy"));
        assert!(skill_md.contains("description: keep"));
        assert!(skill_md.contains("Body stays byte-identical."));
        assert_eq!(renamed["extra.md"].bytes, b"unchanged");
        assert_eq!(
            declared_skill_name(&skill_md).as_deref(),
            Some("review-local-copy")
        );
    }

    #[test]
    fn skills_without_a_declaration_keep_their_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("legacy");
        write(&dir.join("SKILL.md"), b"No frontmatter here.\n");
        let tree = capture_skill_tree(&dir).unwrap();
        assert_eq!(tree.declared_name, None);
        let renamed = rename_skill_tree(&tree.files, "legacy-local-copy").unwrap();
        assert_eq!(renamed["SKILL.md"].bytes, b"No frontmatter here.\n");
    }

    #[test]
    fn materialize_and_verify_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("skill");
        write(&dir.join("SKILL.md"), b"---\nname: skill\n---\n");
        write(&dir.join("deep/file.txt"), b"content");
        let tree = capture_skill_tree(&dir).unwrap();
        let stage = temp.path().join("stage");
        fs::create_dir_all(&stage).unwrap();
        materialize_skill_tree(&stage, &tree.files).unwrap();
        verify_tree_matches(&stage, &tree.files).unwrap();
        let resource =
            skill_resource_from_tree(AgentSkillScope::CodexHome, "skill", "skill", &tree.files)
                .unwrap();
        resource.validate().unwrap();
    }
}
