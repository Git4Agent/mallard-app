//! Codex pet packages: exact v1/v2 parsing, bounded image validation,
//! canonical manifest regeneration, and the reviewed copy transform.
//!
//! The two formats are distinct portable resource kinds. The absent
//! `spriteVersionNumber` field is the declared v1 contract shape, not a
//! compatibility fallback; unknown version markers block. Restore regenerates
//! the canonical manifest for the same format — never an upgrade, downgrade,
//! or conversion.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;

use super::domain::{
    validate_component_name, validate_display_text, validate_pet_id, CatalogFileEntry,
    CodexPetDescriptor, CodexPetFormat, CodexPetResource, MAX_PET_ATLAS_BYTES,
    MAX_PET_DECODED_BYTES, MAX_PET_MANIFEST_BYTES,
};

/// The exact local manifest shape shared by both formats. Unknown fields
/// block; `spriteVersionNumber` is absent for v1 and exactly `2` for v2.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LocalPetManifest {
    id: String,
    #[serde(rename = "displayName")]
    display_name: String,
    description: String,
    #[serde(
        rename = "spriteVersionNumber",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    sprite_version_number: Option<u32>,
    #[serde(rename = "spritesheetPath")]
    spritesheet_path: String,
}

#[derive(Clone, Debug)]
pub struct CapturedPet {
    pub format: CodexPetFormat,
    pub pet_id: String,
    pub display_name: String,
    pub description: String,
    pub atlas_basename: String,
    pub atlas_bytes: Vec<u8>,
    /// Files in the package directory beyond the manifest and atlas; they are
    /// excluded from capture and reported.
    pub extra_files: Vec<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasContainer {
    Png,
    Webp,
}

/// Parse PNG or WebP header dimensions without decoding pixel data. The
/// decoded allocation is bounded before any dimension comparison.
pub fn atlas_dimensions(bytes: &[u8]) -> Result<(AtlasContainer, u32, u32), String> {
    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() >= 24 && bytes[..8] == PNG_SIGNATURE {
        if &bytes[12..16] != b"IHDR" {
            return Err("PNG atlas is missing its IHDR chunk".to_string());
        }
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Ok((AtlasContainer::Png, width, height));
    }
    if bytes.len() >= 30 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let fourcc = &bytes[12..16];
        if fourcc == b"VP8 " {
            if bytes[23..26] != [0x9D, 0x01, 0x2A] {
                return Err("WebP atlas has an invalid lossy start code".to_string());
            }
            let width = (u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF) as u32;
            let height = (u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF) as u32;
            return Ok((AtlasContainer::Webp, width, height));
        }
        if fourcc == b"VP8L" {
            if bytes[20] != 0x2F {
                return Err("WebP atlas has an invalid lossless signature".to_string());
            }
            let b = [bytes[21], bytes[22], bytes[23], bytes[24]];
            let width = 1 + (((b[1] as u32 & 0x3F) << 8) | b[0] as u32);
            let height =
                1 + (((b[3] as u32 & 0x0F) << 10) | ((b[2] as u32) << 2) | ((b[1] as u32) >> 6));
            return Ok((AtlasContainer::Webp, width, height));
        }
        if fourcc == b"VP8X" {
            let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            return Ok((AtlasContainer::Webp, width, height));
        }
        return Err("WebP atlas has an unsupported chunk layout".to_string());
    }
    Err("pet atlas is neither PNG nor WebP".to_string())
}

/// Validate atlas bytes against the exact format contract: bounded encoded
/// size, bounded decoded allocation, container/extension agreement, and the
/// format's exact `192x208`-cell grid dimensions.
pub fn validate_atlas(format: CodexPetFormat, basename: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_PET_ATLAS_BYTES {
        return Err("pet atlas exceeds its encoded size limit".to_string());
    }
    let (container, width, height) = atlas_dimensions(bytes)?;
    let lowered = basename.to_ascii_lowercase();
    let extension_matches = match container {
        AtlasContainer::Png => lowered.ends_with(".png"),
        AtlasContainer::Webp => lowered.ends_with(".webp"),
    };
    if !extension_matches {
        return Err(format!(
            "pet atlas '{}' does not match its container format",
            basename
        ));
    }
    let decoded = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(4);
    if decoded > MAX_PET_DECODED_BYTES {
        return Err("pet atlas exceeds its decoded allocation limit".to_string());
    }
    let (expected_width, expected_height) = format.atlas_dimensions();
    if (width, height) != (expected_width, expected_height) {
        let (columns, rows) = format.cell_grid();
        return Err(format!(
            "pet atlas is {}x{}, but {} requires exactly {}x{} ({}x{} cells of 192x208)",
            width,
            height,
            format.display_label(),
            expected_width,
            expected_height,
            columns,
            rows
        ));
    }
    Ok(())
}

fn read_bounded_no_follow(path: &Path, label: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let meta =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {}: {}", label, error))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!("{} is not a regular file", label));
    }
    if meta.len() > max_bytes {
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
    if bytes.len() as u64 > max_bytes {
        return Err(format!("{} exceeds its size limit", label));
    }
    Ok(bytes)
}

/// Capture one `<CODEX_HOME>/pets/<pet-id>/` package.
pub fn capture_pet(directory: &Path) -> Result<CapturedPet, String> {
    let directory_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "pet directory name is not UTF-8".to_string())?
        .to_string();
    validate_pet_id("pet directory", &directory_name)?;

    let manifest_bytes = read_bounded_no_follow(
        &directory.join("pet.json"),
        &format!("pet '{}' manifest", directory_name),
        MAX_PET_MANIFEST_BYTES,
    )?;
    let manifest: LocalPetManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("pet '{}' manifest is invalid: {}", directory_name, error))?;
    if manifest.id != directory_name {
        return Err(format!(
            "pet manifest id '{}' does not match its directory '{}'",
            manifest.id, directory_name
        ));
    }
    let format = match manifest.sprite_version_number {
        None => CodexPetFormat::V1,
        Some(2) => CodexPetFormat::V2,
        Some(other) => {
            return Err(format!(
                "pet '{}' declares unsupported sprite version {}",
                directory_name, other
            ))
        }
    };
    validate_display_text("pet display name", &manifest.display_name, 128)?;
    validate_display_text("pet description", &manifest.description, 1024)?;
    validate_component_name("pet atlas basename", &manifest.spritesheet_path, 255)?;
    if manifest.spritesheet_path.contains('/') || manifest.spritesheet_path.contains('\\') {
        return Err(format!(
            "pet '{}' atlas path must be a plain basename",
            directory_name
        ));
    }

    let atlas_bytes = read_bounded_no_follow(
        &directory.join(&manifest.spritesheet_path),
        &format!("pet '{}' atlas", directory_name),
        MAX_PET_ATLAS_BYTES,
    )?;
    validate_atlas(format, &manifest.spritesheet_path, &atlas_bytes)?;

    let mut extra_files = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("read pet '{}': {}", directory_name, error))?
    {
        let entry = entry.map_err(|error| format!("walk pet '{}': {}", directory_name, error))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry
            .path()
            .symlink_metadata()
            .map_err(|error| format!("inspect pet file '{}': {}", name, error))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "pet '{}' contains a symlink at '{}'",
                directory_name, name
            ));
        }
        if name != "pet.json" && name != manifest.spritesheet_path {
            extra_files.push(name);
        }
    }
    extra_files.sort();

    Ok(CapturedPet {
        format,
        pet_id: directory_name,
        display_name: manifest.display_name,
        description: manifest.description,
        atlas_basename: manifest.spritesheet_path,
        atlas_bytes,
        extra_files,
    })
}

/// Build the portable resource for a captured pet.
pub fn pet_resource_from_capture(captured: &CapturedPet) -> Result<CodexPetResource, String> {
    let digest = sha256(&captured.atlas_bytes);
    let mut resource = CodexPetResource {
        descriptor: CodexPetDescriptor {
            pet_id: captured.pet_id.clone(),
            display_name: captured.display_name.clone(),
            description: captured.description.clone(),
            atlas_basename: captured.atlas_basename.clone(),
            atlas: CatalogFileEntry {
                object_key: format!("_objects/{}", digest),
                sha256: digest,
                size: captured.atlas_bytes.len() as u64,
                mode: 0o644,
            },
        },
        content_sha256: String::new(),
    };
    resource.content_sha256 = resource.compute_content_sha256(captured.format);
    resource.validate(captured.format)?;
    Ok(resource)
}

/// Canonical local manifest bytes for one format: v1 omits the
/// version marker entirely; v2 pins `spriteVersionNumber: 2`.
fn canonical_manifest_bytes(
    format: CodexPetFormat,
    descriptor: &CodexPetDescriptor,
) -> Result<Vec<u8>, String> {
    let manifest = LocalPetManifest {
        id: descriptor.pet_id.clone(),
        display_name: descriptor.display_name.clone(),
        description: descriptor.description.clone(),
        sprite_version_number: match format {
            CodexPetFormat::V1 => None,
            CodexPetFormat::V2 => Some(2),
        },
        spritesheet_path: descriptor.atlas_basename.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("serialize pet manifest: {}", error))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Write a pet package into a staging directory: canonical manifest plus the
/// exact atlas bytes.
pub fn materialize_pet(
    stage: &Path,
    format: CodexPetFormat,
    descriptor: &CodexPetDescriptor,
    atlas_bytes: &[u8],
) -> Result<(), String> {
    validate_atlas(format, &descriptor.atlas_basename, atlas_bytes)?;
    fs::write(
        stage.join("pet.json"),
        canonical_manifest_bytes(format, descriptor)?,
    )
    .map_err(|error| format!("write pet manifest: {}", error))?;
    fs::write(stage.join(&descriptor.atlas_basename), atlas_bytes)
        .map_err(|error| format!("write pet atlas: {}", error))?;
    Ok(())
}

/// The reviewed copy transform: a new directory name, a matching manifest ID
/// and display name, the same resource kind, and exact atlas bytes.
pub fn renamed_pet(captured: &CapturedPet, new_pet_id: &str) -> Result<CapturedPet, String> {
    validate_pet_id("pet copy name", new_pet_id)?;
    Ok(CapturedPet {
        format: captured.format,
        pet_id: new_pet_id.to_string(),
        display_name: new_pet_id.to_string(),
        description: captured.description.clone(),
        atlas_basename: captured.atlas_basename.clone(),
        atlas_bytes: captured.atlas_bytes.clone(),
        extra_files: Vec::new(),
    })
}

/// Content digest of the pet currently installed at a directory, with its
/// observed format.
pub fn observed_pet_digest(directory: &Path) -> Result<(CodexPetFormat, String), String> {
    let captured = capture_pet(directory)?;
    let resource = pet_resource_from_capture(&captured)?;
    Ok((captured.format, resource.content_sha256))
}

#[cfg(test)]
pub mod fixtures {
    use super::CodexPetFormat;

    /// A minimal, header-valid lossless WebP with exact atlas dimensions.
    pub fn webp_atlas(format: CodexPetFormat) -> Vec<u8> {
        let (width, height) = format.atlas_dimensions();
        webp_with_dimensions(width, height)
    }

    pub fn webp_with_dimensions(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&20_u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8L");
        bytes.extend_from_slice(&12_u32.to_le_bytes());
        bytes.push(0x2F);
        let w = width - 1;
        let h = height - 1;
        let packed: u32 = (w & 0x3FFF) | ((h & 0x3FFF) << 14);
        bytes.extend_from_slice(&packed.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    /// A minimal, header-valid PNG with exact atlas dimensions.
    pub fn png_atlas(format: CodexPetFormat) -> Vec<u8> {
        let (width, height) = format.atlas_dimensions();
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{png_atlas, webp_atlas, webp_with_dimensions};
    use super::*;

    fn write_pet(
        directory: &Path,
        pet_id: &str,
        version: Option<u32>,
        atlas_name: &str,
        atlas: &[u8],
    ) {
        fs::create_dir_all(directory).unwrap();
        let version_field = version
            .map(|value| format!("\n  \"spriteVersionNumber\": {},", value))
            .unwrap_or_default();
        fs::write(
            directory.join("pet.json"),
            format!(
                "{{\n  \"id\": \"{}\",\n  \"displayName\": \"Pet\",\n  \"description\": \"One short sentence.\",{}\n  \"spritesheetPath\": \"{}\"\n}}\n",
                pet_id, version_field, atlas_name
            ),
        )
        .unwrap();
        fs::write(directory.join(atlas_name), atlas).unwrap();
    }

    #[test]
    fn png_and_webp_atlases_round_trip_for_both_formats() {
        for format in [CodexPetFormat::V1, CodexPetFormat::V2] {
            for (name, bytes) in [
                ("spritesheet.webp", webp_atlas(format)),
                ("spritesheet.png", png_atlas(format)),
            ] {
                let temp = tempfile::tempdir().unwrap();
                let dir = temp.path().join("duck");
                let version = match format {
                    CodexPetFormat::V1 => None,
                    CodexPetFormat::V2 => Some(2),
                };
                write_pet(&dir, "duck", version, name, &bytes);
                let captured = capture_pet(&dir).unwrap();
                assert_eq!(captured.format, format);
                let resource = pet_resource_from_capture(&captured).unwrap();
                resource.validate(format).unwrap();

                // Restore regenerates the canonical manifest for the same
                // format and identical atlas bytes; recapture agrees.
                let stage = temp.path().join("stage").join("duck");
                fs::create_dir_all(&stage).unwrap();
                materialize_pet(&stage, format, &resource.descriptor, &captured.atlas_bytes)
                    .unwrap();
                let restaged = capture_pet(&stage).unwrap();
                assert_eq!(restaged.format, format);
                assert_eq!(restaged.atlas_bytes, captured.atlas_bytes);
                let recomputed = pet_resource_from_capture(&restaged).unwrap();
                assert_eq!(recomputed.content_sha256, resource.content_sha256);
            }
        }
    }

    #[test]
    fn marker_and_dimension_mismatches_block() {
        let temp = tempfile::tempdir().unwrap();
        // v1 atlas paired with a v2 marker.
        let dir = temp.path().join("mismatch");
        write_pet(
            &dir,
            "mismatch",
            Some(2),
            "spritesheet.webp",
            &webp_atlas(CodexPetFormat::V1),
        );
        assert!(capture_pet(&dir).unwrap_err().contains("1536x1872"));
        // v2 atlas paired with a v1 marker.
        let dir = temp.path().join("mismatch2");
        write_pet(
            &dir,
            "mismatch2",
            None,
            "spritesheet.webp",
            &webp_atlas(CodexPetFormat::V2),
        );
        assert!(capture_pet(&dir).unwrap_err().contains("1536x2288"));
        // Unknown version markers are blocked, never defaulted.
        let dir = temp.path().join("future");
        write_pet(
            &dir,
            "future",
            Some(3),
            "spritesheet.webp",
            &webp_atlas(CodexPetFormat::V1),
        );
        assert!(capture_pet(&dir)
            .unwrap_err()
            .contains("unsupported sprite version 3"));
        // Wrong dimensions entirely.
        let dir = temp.path().join("tiny");
        write_pet(
            &dir,
            "tiny",
            None,
            "spritesheet.webp",
            &webp_with_dimensions(10, 10),
        );
        assert!(capture_pet(&dir).unwrap_err().contains("requires exactly"));
    }

    #[test]
    fn manifest_shape_violations_block() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("duck");
        fs::create_dir_all(&dir).unwrap();
        // Unknown manifest field.
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"d","spritesheetPath":"s.webp","theme":"pond"}"#,
        )
        .unwrap();
        fs::write(dir.join("s.webp"), webp_atlas(CodexPetFormat::V1)).unwrap();
        assert!(capture_pet(&dir).unwrap_err().contains("invalid"));
        // Directory/id mismatch.
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"swan","displayName":"Duck","description":"d","spritesheetPath":"s.webp"}"#,
        )
        .unwrap();
        assert!(capture_pet(&dir)
            .unwrap_err()
            .contains("does not match its directory"));
        // Path traversal in the atlas reference.
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"d","spritesheetPath":"../escape.webp"}"#,
        )
        .unwrap();
        assert!(capture_pet(&dir).is_err());
        // Missing atlas.
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"d","spritesheetPath":"missing.webp"}"#,
        )
        .unwrap();
        assert!(capture_pet(&dir).is_err());
    }

    #[test]
    fn extra_files_are_excluded_and_reported() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("duck");
        write_pet(
            &dir,
            "duck",
            None,
            "spritesheet.webp",
            &webp_atlas(CodexPetFormat::V1),
        );
        fs::write(dir.join("notes.txt"), b"extra").unwrap();
        let captured = capture_pet(&dir).unwrap();
        assert_eq!(captured.extra_files, vec!["notes.txt"]);
    }

    #[test]
    fn copies_preserve_format_and_atlas_while_renaming_identity() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("duck");
        write_pet(
            &dir,
            "duck",
            Some(2),
            "spritesheet.webp",
            &webp_atlas(CodexPetFormat::V2),
        );
        let captured = capture_pet(&dir).unwrap();
        let copy = renamed_pet(&captured, "duck-local-copy").unwrap();
        assert_eq!(copy.format, CodexPetFormat::V2);
        assert_eq!(copy.pet_id, "duck-local-copy");
        assert_eq!(copy.display_name, "duck-local-copy");
        assert_eq!(copy.atlas_bytes, captured.atlas_bytes);
        let resource = pet_resource_from_capture(&copy).unwrap();
        resource.validate(CodexPetFormat::V2).unwrap();
    }
}
