//! The format-codec contract and the bounded directory primitives every
//! directory-backed codec shares.
//!
//! A codec owns exactly one registered (format, format version): it decodes
//! the envelope's bounded `identity` and `descriptor` into typed structures
//! that reject unknown fields, validates portable files, and — for formats
//! whose resources live in a directory — captures, materializes, verifies,
//! and renames that directory. Common code speaks only this trait; it never
//! matches on a format key.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use bytes::Bytes;
use serde::Serialize;

use super::domain::{
    sha256_hex, validate_portable_relative_path, BoundedPortableObject, CatalogCoordinate,
    CatalogPortableFile, CatalogResourceEnvelope,
};

/// One captured payload file: exact bytes and its portable safe mode
/// (`0o755` for executables, `0o644` otherwise).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedFile {
    pub bytes: Vec<u8>,
    pub mode: u32,
}

/// A reviewable capture warning: acknowledged by digest before publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureWarning {
    pub code: String,
    pub message: String,
}

/// Everything a codec captured from one local resource: the typed identity
/// and descriptor already encoded as bounded portable objects, the exact
/// payload bytes, the name to show, reviewed warnings, and scan notes.
#[derive(Clone, Debug)]
pub struct CapturedResource {
    pub identity: BoundedPortableObject,
    pub descriptor: BoundedPortableObject,
    pub files: BTreeMap<String, CapturedFile>,
    pub display_name: String,
    pub warnings: Vec<CaptureWarning>,
    /// Notes about the whole capture rather than a reviewed warning, such as
    /// files that were excluded.
    pub notes: Vec<String>,
}

impl CapturedResource {
    /// The portable envelope of this capture at one coordinate.
    pub fn to_envelope(&self, coordinate: &CatalogCoordinate) -> CatalogResourceEnvelope {
        let files = self
            .files
            .iter()
            .map(|(path, file)| CatalogPortableFile {
                path: path.clone(),
                sha256: sha256_hex(&file.bytes),
                size: file.bytes.len() as u64,
                mode: file.mode,
            })
            .collect();
        CatalogResourceEnvelope::new(
            coordinate,
            self.identity.clone(),
            self.descriptor.clone(),
            files,
        )
    }

    /// Content-addressed payload bytes for publication.
    pub fn payloads(&self) -> BTreeMap<String, Bytes> {
        self.files
            .values()
            .map(|file| (sha256_hex(&file.bytes), Bytes::from(file.bytes.clone())))
            .collect()
    }
}

pub type PayloadReader<'a> = dyn Fn(&str, u64) -> Result<Vec<u8>, String> + 'a;

/// The compiled codec contract for one registered format version.
pub trait CatalogFormatCodec: Send + Sync {
    /// Typed validation for the exact registered coordinate: decode identity
    /// and descriptor with unknown fields rejected, then apply the format's
    /// own rules over the portable files.
    fn validate(&self, envelope: &CatalogResourceEnvelope) -> Result<(), String>;

    /// The name shown for one resource.
    fn display_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String>;

    /// The member this resource occupies inside its target root: the
    /// directory name for directory resources, the native identifier for
    /// provider-inventory resources.
    fn member_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String>;

    /// The portable root class this resource belongs to, when the format
    /// distinguishes target roots inside its identity.
    fn root_class(&self, _envelope: &CatalogResourceEnvelope) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// A secondary per-resource label for review rows (a root class, a format
    /// version); `None` when the coordinate says it all.
    fn resource_label(&self, _envelope: &CatalogResourceEnvelope) -> Option<String> {
        None
    }

    /// A one-line description from the descriptor, when the format has one.
    fn description(&self, _envelope: &CatalogResourceEnvelope) -> Option<String> {
        None
    }

    /// The identity two local candidates in one root must not share (a
    /// provider resolves skills by name, so equal names collide); `None`
    /// when the directory name alone is the identity.
    fn collision_key(&self, _captured: &CapturedResource) -> Option<String> {
        None
    }

    /// Capture one directory as a resource of this exact coordinate. Fails
    /// when the directory is not a resource of this format version; the
    /// inventory then offers it to the format's sibling versions.
    fn capture_directory(
        &self,
        _directory: &Path,
        _root_class: &str,
    ) -> Result<CapturedResource, String> {
        Err("this format has no directory payload".to_string())
    }

    /// Write a resource into a staging directory from its envelope and the
    /// fetched payload bytes.
    fn materialize(
        &self,
        stage: &Path,
        _envelope: &CatalogResourceEnvelope,
        files: &BTreeMap<String, CapturedFile>,
    ) -> Result<(), String> {
        write_directory_tree(stage, files)
    }

    /// The reviewed copy transform: the same resource under a new member
    /// name, with whatever the format requires to keep the copy valid and
    /// independently addressable.
    fn rename(
        &self,
        _captured: &CapturedResource,
        _new_name: &str,
    ) -> Result<CapturedResource, String> {
        Err("this format has no copy transform".to_string())
    }

    /// A small preview image as a PNG data URI derived from the payload,
    /// cached by digest under `cache_dir`. `read_payload` yields the bytes of
    /// one payload file by digest and exact size.
    fn preview(
        &self,
        _envelope: &CatalogResourceEnvelope,
        _cache_dir: &Path,
        _read_payload: &PayloadReader<'_>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// What the resource's page shows, read from the envelope and, where the
    /// format needs them, its payload files through `read_payload`. Every
    /// registered format has a page; a format without one is an error.
    fn detail(
        &self,
        _envelope: &CatalogResourceEnvelope,
        _read_payload: &PayloadReader<'_>,
    ) -> Result<CatalogResourceDetail, String> {
        Err("this format has no page".to_string())
    }
}

/// One payload file as a resource's page lists it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogDetailFile {
    pub path: String,
    pub size: u64,
    pub executable: bool,
}

/// The display words an installed plugin's own manifest gives it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogDetailPluginListing {
    pub display_name: Option<String>,
    pub developer: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub website_url: Option<String>,
    pub privacy_url: Option<String>,
    pub terms_url: Option<String>,
}

/// A resource's page, by the shape its format presents. Display only: it
/// never feeds an identity, a digest, a plan or a manifest.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogResourceDetail {
    /// A document and the files beside it (an Agent Skill's `SKILL.md`).
    /// `text` is absent when the document is not UTF-8.
    Document {
        path: String,
        text: Option<String>,
        files: Vec<CatalogDetailFile>,
    },
    /// What an install intent names. `install` is the provider's own
    /// terminal lines for installing it by hand, filled by its adapter; the
    /// installed version and listing are this Mac's, filled only when the
    /// plugin is installed here.
    Install {
        plugin_id: String,
        marketplace: String,
        location: Option<String>,
        git_ref: Option<String>,
        install: Vec<String>,
        installed_version: Option<String>,
        listing: Option<Box<CatalogDetailPluginListing>>,
    },
    /// An animated sprite atlas (a desktop pet) as a data URI, with its cell
    /// grid and the format's one-line description.
    Sprite {
        atlas: String,
        columns: u32,
        rows: u32,
        description: Option<String>,
    },
}

/// The largest file a resource's page opens: an Agent Skill's own per-file
/// capture cap, so every captured skill file fits.
pub const MAX_DETAIL_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// One payload file as a resource's page opens it. Display only, like
/// `CatalogResourceDetail`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogResourceFile {
    pub path: String,
    pub size: u64,
    pub executable: bool,
    /// Absent when the bytes are not text (`detail_text`).
    pub text: Option<String>,
    /// This Mac's copy of the file, for Reveal in Finder; absent when the
    /// page reads the storage's copy.
    pub local_path: Option<String>,
}

/// Bytes as a page shows them: text when they are strict UTF-8 holding no
/// NUL byte, else nothing — the page says the file is not text.
pub fn detail_text(bytes: Vec<u8>) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// One payload file's bytes for a page, checked against the envelope's size
/// and digest whatever the reader is, and refused above `max_bytes`.
pub fn read_detail_payload(
    file: &CatalogPortableFile,
    max_bytes: u64,
    read_payload: &PayloadReader<'_>,
) -> Result<Vec<u8>, String> {
    if file.size > max_bytes {
        return Err(format!("'{}' is too large to show", file.path));
    }
    let bytes = read_payload(&file.sha256, file.size)?;
    if bytes.len() as u64 != file.size || sha256_hex(&bytes) != file.sha256 {
        return Err(format!("'{}' does not match its digest", file.path));
    }
    Ok(bytes)
}

/// The page's file list: every payload file with its size, executables
/// marked by their portable mode.
pub fn detail_files(envelope: &CatalogResourceEnvelope) -> Vec<CatalogDetailFile> {
    envelope
        .files
        .iter()
        .map(|file| CatalogDetailFile {
            path: file.path.clone(),
            size: file.size,
            executable: file.mode & 0o111 != 0,
        })
        .collect()
}

/// Bounds for one bounded directory read.
#[derive(Clone, Copy, Debug)]
pub struct TreeLimits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

fn is_executable(mode_bits: u32, bytes: &[u8]) -> bool {
    mode_bits & 0o111 != 0 || bytes.starts_with(b"#!")
}

pub fn looks_secret_shaped(bytes: &[u8]) -> bool {
    if bytes.len() > 2 * 1024 * 1024 || bytes.contains(&0) {
        return false;
    }
    mallard_infrastructure::credential_redaction::shared()
        .redact_str(&String::from_utf8_lossy(bytes))
        .is_some()
}

/// Read one regular file without following symlinks, refusing special files,
/// oversized content, and reads that changed underneath. Returns the bytes
/// and the raw mode bits.
pub fn read_file_no_follow(
    path: &Path,
    label: &str,
    max_bytes: u64,
) -> Result<(Vec<u8>, u32), String> {
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

/// Capture one directory as an exact bounded tree. Rejects symlinks, special
/// files, unsafe components, nested VCS metadata, case collisions, oversized
/// content, and unstable reads.
pub fn read_directory_tree(
    directory: &Path,
    limits: &TreeLimits,
) -> Result<BTreeMap<String, CapturedFile>, String> {
    let mut files = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut total: u64 = 0;
    let mut pending = vec![(directory.to_path_buf(), String::new())];
    while let Some((current, prefix)) = pending.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("read directory '{}': {}", current.display(), error))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("walk directory: {}", error))?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| "payload file name is not UTF-8".to_string())?
                .to_string();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };
            validate_portable_relative_path(&relative)?;
            let meta = entry
                .path()
                .symlink_metadata()
                .map_err(|error| format!("inspect '{}': {}", relative, error))?;
            if meta.file_type().is_symlink() {
                return Err(format!("payload contains a symlink at '{}'", relative));
            }
            if meta.is_dir() {
                pending.push((entry.path(), relative));
                continue;
            }
            if !meta.is_file() {
                return Err(format!("payload contains a special file at '{}'", relative));
            }
            if !folded.insert(relative.to_lowercase()) {
                return Err(format!(
                    "payload has a case-folded path collision at '{}'",
                    relative
                ));
            }
            if files.len() >= limits.max_files {
                return Err("payload exceeds its file-count limit".to_string());
            }
            let (bytes, mode_bits) = read_file_no_follow(
                &entry.path(),
                &format!("payload file '{}'", relative),
                limits.max_file_bytes,
            )?;
            total = total.saturating_add(bytes.len() as u64);
            if total > limits.max_total_bytes {
                return Err("payload exceeds its byte limit".to_string());
            }
            let mode = if is_executable(mode_bits, &bytes) {
                0o755
            } else {
                0o644
            };
            files.insert(relative, CapturedFile { bytes, mode });
        }
    }
    Ok(files)
}

/// Executable and credential-shaped content warnings over a captured tree.
pub fn content_warnings(files: &BTreeMap<String, CapturedFile>) -> Vec<CaptureWarning> {
    let executable: Vec<&str> = files
        .iter()
        .filter(|(_, file)| file.mode == 0o755)
        .map(|(path, _)| path.as_str())
        .collect();
    let secret_shaped: Vec<&str> = files
        .iter()
        .filter(|(_, file)| looks_secret_shaped(&file.bytes))
        .map(|(path, _)| path.as_str())
        .collect();
    let mut warnings = Vec::new();
    if !executable.is_empty() {
        warnings.push(CaptureWarning {
            code: "executable_content".to_string(),
            message: format!("contains executable content: {}", executable.join(", ")),
        });
    }
    if !secret_shaped.is_empty() {
        warnings.push(CaptureWarning {
            code: "secret_shaped".to_string(),
            message: format!(
                "contains credential-shaped content: {}",
                secret_shaped.join(", ")
            ),
        });
    }
    warnings
}

/// Write a captured tree into a staging directory with portable modes.
pub fn write_directory_tree(
    stage: &Path,
    files: &BTreeMap<String, CapturedFile>,
) -> Result<(), String> {
    for (relative, file) in files {
        validate_portable_relative_path(relative)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: TreeLimits = TreeLimits {
        max_files: 8,
        max_file_bytes: 1024,
        max_total_bytes: 4096,
    };

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_page_shows_strict_utf8_without_nul_as_text() {
        assert_eq!(
            detail_text(b"def run():\n".to_vec()).as_deref(),
            Some("def run():\n")
        );
        assert_eq!(
            detail_text("名前\n".as_bytes().to_vec()).as_deref(),
            Some("名前\n")
        );
        assert_eq!(detail_text(Vec::new()).as_deref(), Some(""));
        assert_eq!(detail_text(b"a\0b".to_vec()), None);
        assert_eq!(detail_text(vec![0xff, 0xfe, 0x41]), None);
        // Every captured skill file fits the page's cap.
        assert!(super::super::agent_skill::MAX_SKILL_FILE_BYTES <= MAX_DETAIL_FILE_BYTES);
    }

    #[test]
    fn trees_capture_exact_bytes_modes_and_warnings() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("review");
        write(&dir.join("SKILL.md"), b"---\nname: review\n---\nBody\n");
        write(&dir.join("scripts/run.sh"), b"#!/bin/sh\necho ok\n");
        write(
            &dir.join("notes.txt"),
            b"ghp_abcdefghijklmnopqrstuvwxyz0123456789\n",
        );
        let files = read_directory_tree(&dir, &LIMITS).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files["scripts/run.sh"].mode, 0o755);
        assert_eq!(files["SKILL.md"].mode, 0o644);
        let warnings = content_warnings(&files);
        assert_eq!(warnings[0].code, "executable_content");
        assert!(warnings[0].message.contains("scripts/run.sh"));
        assert_eq!(warnings[1].code, "secret_shaped");
        assert!(warnings[1].message.contains("notes.txt"));
    }

    #[test]
    fn symlinks_special_files_vcs_metadata_and_bounds_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("tree");
        write(&dir.join("a.md"), b"a");
        write(&dir.join(".git/config"), b"[core]\n");
        assert!(read_directory_tree(&dir, &LIMITS)
            .unwrap_err()
            .contains("invalid"));
        fs::remove_dir_all(dir.join(".git")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", dir.join("link")).unwrap();
            assert!(read_directory_tree(&dir, &LIMITS)
                .unwrap_err()
                .contains("symlink"));
            fs::remove_file(dir.join("link")).unwrap();
            let socket = std::os::unix::net::UnixListener::bind(dir.join("socket")).unwrap();
            assert!(read_directory_tree(&dir, &LIMITS)
                .unwrap_err()
                .contains("special file"));
            drop(socket);
            fs::remove_file(dir.join("socket")).unwrap();
        }
        write(&dir.join("big.bin"), &vec![0_u8; 1025]);
        assert!(read_directory_tree(&dir, &LIMITS)
            .unwrap_err()
            .contains("size limit"));
        fs::remove_file(dir.join("big.bin")).unwrap();
        for index in 0..9 {
            write(&dir.join(format!("f{index}.txt")), b"x");
        }
        assert!(read_directory_tree(&dir, &LIMITS)
            .unwrap_err()
            .contains("file-count"));
    }

    #[test]
    fn captures_become_canonical_envelopes_with_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("tree");
        write(&dir.join("b.txt"), b"bee");
        write(&dir.join("a.txt"), b"ay");
        let captured = CapturedResource {
            identity: BoundedPortableObject::from_json(&serde_json::json!({ "name": "tree" }))
                .unwrap(),
            descriptor: BoundedPortableObject::empty(),
            files: read_directory_tree(&dir, &LIMITS).unwrap(),
            display_name: "tree".to_string(),
            warnings: Vec::new(),
            notes: Vec::new(),
        };
        let coordinate = CatalogCoordinate::new("codex", "kind", "format", 1).unwrap();
        let envelope = captured.to_envelope(&coordinate);
        envelope.validate().unwrap();
        assert_eq!(
            envelope
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt", "b.txt"]
        );
        let payloads = captured.payloads();
        assert_eq!(payloads.len(), 2);
        assert!(payloads.contains_key(&envelope.files[0].sha256));

        let stage = temp.path().join("stage");
        fs::create_dir_all(&stage).unwrap();
        write_directory_tree(&stage, &captured.files).unwrap();
        assert_eq!(
            read_directory_tree(&stage, &LIMITS).unwrap(),
            captured.files
        );
    }
}
