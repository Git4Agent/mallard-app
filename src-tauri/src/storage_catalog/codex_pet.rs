//! The `openai.codex-pet` format codec: one codec instance per registered
//! format version (1 and 2), with exact parsing, bounded image validation,
//! canonical manifest regeneration, and the reviewed copy transform.
//!
//! The two versions are distinct portable resources with separate Library
//! repositories; there is no conversion between them. The absent
//! `spriteVersionNumber` field is the declared version-1 contract shape, not
//! a compatibility fallback; unknown version markers block. Which version a
//! package declares is a typed detail inside this codec, never a wire tag.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::codec::{
    read_file_no_follow, CapturedFile, CapturedResource, CatalogFormatCodec, PayloadReader,
};
use super::domain::{
    validate_component_name, validate_display_text, BoundedPortableObject, CatalogResourceEnvelope,
};

pub const FORMAT_KEY: &str = "openai.codex-pet";
pub const KIND_KEY: &str = "desktop_pet";
pub const MAX_PET_MANIFEST_BYTES: u64 = 16 * 1024;
pub const MAX_PET_ATLAS_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PET_DECODED_BYTES: u64 = 16 * 1024 * 1024;

/// The registered format versions, selected by `format_version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodexPetFormat {
    V1,
    V2,
}

impl CodexPetFormat {
    pub(crate) fn for_version(version: u32) -> Option<Self> {
        match version {
            1 => Some(CodexPetFormat::V1),
            2 => Some(CodexPetFormat::V2),
            _ => None,
        }
    }

    pub(crate) fn format_version(&self) -> u32 {
        match self {
            CodexPetFormat::V1 => 1,
            CodexPetFormat::V2 => 2,
        }
    }

    /// Exact atlas pixel dimensions for this version.
    pub(crate) fn atlas_dimensions(&self) -> (u32, u32) {
        match self {
            CodexPetFormat::V1 => (1536, 1872),
            CodexPetFormat::V2 => (1536, 2288),
        }
    }

    /// Sprite grid: 8 columns of 192x208 cells; 9 rows for v1, 11 for v2.
    pub(crate) fn cell_grid(&self) -> (u32, u32) {
        match self {
            CodexPetFormat::V1 => (8, 9),
            CodexPetFormat::V2 => (8, 11),
        }
    }

    fn label(&self) -> String {
        format!("v{}", self.format_version())
    }

    /// The manifest marker: absent for v1, exactly `2` for v2.
    fn sprite_version_marker(&self) -> Option<u32> {
        match self {
            CodexPetFormat::V1 => None,
            CodexPetFormat::V2 => Some(2),
        }
    }

    fn from_marker(pet_id: &str, marker: Option<u32>) -> Result<Self, String> {
        match marker {
            None => Ok(CodexPetFormat::V1),
            Some(2) => Ok(CodexPetFormat::V2),
            Some(other) => Err(format!(
                "pet '{}' declares unsupported sprite version {}",
                pet_id, other
            )),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodexPetIdentity {
    pub pet_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodexPetDescriptor {
    pub display_name: String,
    pub description: String,
    pub atlas_basename: String,
}

/// The exact local manifest shape shared by both versions. Unknown fields
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

pub fn validate_pet_id(label: &str, value: &str) -> Result<(), String> {
    validate_component_name(label, value, 64)
}

fn validate_atlas_basename(basename: &str) -> Result<(), String> {
    validate_component_name("pet atlas basename", basename, 255)?;
    let lowered = basename.to_ascii_lowercase();
    if !(lowered.ends_with(".png") || lowered.ends_with(".webp")) {
        return Err(format!(
            "pet atlas '{}' must be a PNG or WebP basename",
            basename
        ));
    }
    Ok(())
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

/// Validate atlas bytes against the exact version contract: bounded encoded
/// size, bounded decoded allocation, container/extension agreement, and the
/// version's exact `192x208`-cell grid dimensions.
fn validate_atlas(format: CodexPetFormat, basename: &str, bytes: &[u8]) -> Result<(), String> {
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
            format.label(),
            expected_width,
            expected_height,
            columns,
            rows
        ));
    }
    Ok(())
}

struct CapturedPet {
    format: CodexPetFormat,
    identity: CodexPetIdentity,
    descriptor: CodexPetDescriptor,
    atlas_bytes: Vec<u8>,
    extra_files: Vec<String>,
}

/// Capture one `<CODEX_HOME>/pets/<pet-id>/` package at whatever version it
/// declares.
fn capture_pet(directory: &Path) -> Result<CapturedPet, String> {
    let directory_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "pet directory name is not UTF-8".to_string())?
        .to_string();
    validate_pet_id("pet directory", &directory_name)?;

    let (manifest_bytes, _) = read_file_no_follow(
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
    let format = CodexPetFormat::from_marker(&directory_name, manifest.sprite_version_number)?;
    validate_display_text("pet display name", &manifest.display_name, 128)?;
    validate_display_text("pet description", &manifest.description, 1024)?;
    validate_atlas_basename(&manifest.spritesheet_path)?;

    let (atlas_bytes, _) = read_file_no_follow(
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
        identity: CodexPetIdentity {
            pet_id: directory_name,
        },
        descriptor: CodexPetDescriptor {
            display_name: manifest.display_name,
            description: manifest.description,
            atlas_basename: manifest.spritesheet_path,
        },
        atlas_bytes,
        extra_files,
    })
}

fn captured_resource(
    identity: &CodexPetIdentity,
    descriptor: &CodexPetDescriptor,
    atlas_bytes: Vec<u8>,
    extra_files: &[String],
) -> Result<CapturedResource, String> {
    let mut notes = Vec::new();
    if !extra_files.is_empty() {
        notes.push(format!(
            "pet '{}' has extra files that will not sync: {}",
            identity.pet_id,
            extra_files.join(", ")
        ));
    }
    Ok(CapturedResource {
        identity: BoundedPortableObject::encode(identity)?,
        descriptor: BoundedPortableObject::encode(descriptor)?,
        files: BTreeMap::from([(
            descriptor.atlas_basename.clone(),
            CapturedFile {
                bytes: atlas_bytes,
                mode: 0o644,
            },
        )]),
        display_name: descriptor.display_name.clone(),
        warnings: Vec::new(),
        notes,
    })
}

fn decode(
    envelope: &CatalogResourceEnvelope,
) -> Result<(CodexPetIdentity, CodexPetDescriptor), String> {
    let identity: CodexPetIdentity = envelope
        .identity
        .decode()
        .map_err(|error| format!("pet identity: {}", error))?;
    let descriptor: CodexPetDescriptor = envelope
        .descriptor
        .decode()
        .map_err(|error| format!("pet descriptor: {}", error))?;
    validate_pet_id("pet id", &identity.pet_id)?;
    validate_display_text("pet display name", &descriptor.display_name, 128)?;
    validate_display_text("pet description", &descriptor.description, 1024)?;
    validate_atlas_basename(&descriptor.atlas_basename)?;
    Ok((identity, descriptor))
}

/// Canonical local manifest bytes for one version: v1 omits the version
/// marker entirely; v2 pins `spriteVersionNumber: 2`.
fn canonical_manifest_bytes(
    format: CodexPetFormat,
    identity: &CodexPetIdentity,
    descriptor: &CodexPetDescriptor,
) -> Result<Vec<u8>, String> {
    let manifest = LocalPetManifest {
        id: identity.pet_id.clone(),
        display_name: descriptor.display_name.clone(),
        description: descriptor.description.clone(),
        sprite_version_number: format.sprite_version_marker(),
        spritesheet_path: descriptor.atlas_basename.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("serialize pet manifest: {}", error))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub struct CodexPetCodec {
    format: CodexPetFormat,
}

impl CodexPetCodec {
    pub fn for_version(version: u32) -> Option<Self> {
        CodexPetFormat::for_version(version).map(|format| Self { format })
    }
}

impl CatalogFormatCodec for CodexPetCodec {
    fn validate(&self, envelope: &CatalogResourceEnvelope) -> Result<(), String> {
        let (identity, descriptor) = decode(envelope)?;
        if envelope.format_version != self.format.format_version() {
            return Err(format!(
                "pet '{}' is not a {} pet",
                identity.pet_id,
                self.format.label()
            ));
        }
        let [atlas] = envelope.files.as_slice() else {
            return Err(format!(
                "pet '{}' must own exactly one atlas file",
                identity.pet_id
            ));
        };
        if atlas.path != descriptor.atlas_basename {
            return Err(format!(
                "pet '{}' atlas file does not match its declared basename",
                identity.pet_id
            ));
        }
        atlas.validate(
            &format!("pet '{}' atlas", identity.pet_id),
            MAX_PET_ATLAS_BYTES,
        )?;
        if atlas.mode != 0o644 {
            return Err(format!(
                "pet '{}' atlas must use the portable regular-file mode",
                identity.pet_id
            ));
        }
        Ok(())
    }

    fn display_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
        Ok(decode(envelope)?.1.display_name)
    }

    fn member_name(&self, envelope: &CatalogResourceEnvelope) -> Result<String, String> {
        Ok(decode(envelope)?.0.pet_id)
    }

    fn resource_label(&self, _envelope: &CatalogResourceEnvelope) -> Option<String> {
        Some(self.format.label())
    }

    fn description(&self, envelope: &CatalogResourceEnvelope) -> Option<String> {
        decode(envelope)
            .ok()
            .map(|(_, descriptor)| descriptor.description)
    }

    fn capture_directory(
        &self,
        directory: &Path,
        _root_class: &str,
    ) -> Result<CapturedResource, String> {
        let captured = capture_pet(directory)?;
        if captured.format != self.format {
            return Err(format!(
                "pet '{}' is a {} pet, not {}",
                captured.identity.pet_id,
                captured.format.label(),
                self.format.label()
            ));
        }
        captured_resource(
            &captured.identity,
            &captured.descriptor,
            captured.atlas_bytes,
            &captured.extra_files,
        )
    }

    fn materialize(
        &self,
        stage: &Path,
        envelope: &CatalogResourceEnvelope,
        files: &BTreeMap<String, CapturedFile>,
    ) -> Result<(), String> {
        let (identity, descriptor) = decode(envelope)?;
        let atlas = files
            .get(&descriptor.atlas_basename)
            .ok_or_else(|| format!("pet '{}' atlas payload is missing", identity.pet_id))?;
        validate_atlas(self.format, &descriptor.atlas_basename, &atlas.bytes)?;
        fs::write(
            stage.join("pet.json"),
            canonical_manifest_bytes(self.format, &identity, &descriptor)?,
        )
        .map_err(|error| format!("write pet manifest: {}", error))?;
        fs::write(stage.join(&descriptor.atlas_basename), &atlas.bytes)
            .map_err(|error| format!("write pet atlas: {}", error))?;
        Ok(())
    }

    /// A new directory name, a matching manifest id and display name, the
    /// same version, and exact atlas bytes.
    fn rename(
        &self,
        captured: &CapturedResource,
        new_name: &str,
    ) -> Result<CapturedResource, String> {
        validate_pet_id("pet copy name", new_name)?;
        let descriptor: CodexPetDescriptor = captured
            .descriptor
            .decode()
            .map_err(|error| format!("pet descriptor: {}", error))?;
        let atlas = captured
            .files
            .get(&descriptor.atlas_basename)
            .ok_or("pet copy source has no atlas")?;
        captured_resource(
            &CodexPetIdentity {
                pet_id: new_name.to_string(),
            },
            &CodexPetDescriptor {
                display_name: new_name.to_string(),
                description: descriptor.description,
                atlas_basename: descriptor.atlas_basename,
            },
            atlas.bytes.clone(),
            &[],
        )
    }

    fn preview(
        &self,
        envelope: &CatalogResourceEnvelope,
        cache_dir: &Path,
        read_payload: &PayloadReader<'_>,
    ) -> Result<Option<String>, String> {
        let (identity, descriptor) = decode(envelope)?;
        let atlas = envelope
            .file(&descriptor.atlas_basename)
            .ok_or_else(|| format!("pet '{}' has no atlas entry", identity.pet_id))?;
        atlas.validate("pet atlas", MAX_PET_ATLAS_BYTES)?;
        super::pet_preview::cached_preview_data_uri(cache_dir, self.format, &atlas.sha256, || {
            read_payload(&atlas.sha256, atlas.size)
        })
        .map(Some)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::CodexPetFormat;

    fn format(version: u32) -> CodexPetFormat {
        CodexPetFormat::for_version(version).expect("registered pet version")
    }

    /// A minimal, header-valid lossless WebP with exact atlas dimensions.
    pub fn webp_atlas(version: u32) -> Vec<u8> {
        let (width, height) = format(version).atlas_dimensions();
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
    pub fn png_atlas(version: u32) -> Vec<u8> {
        let (width, height) = format(version).atlas_dimensions();
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    /// A real, decodable PNG atlas with the version's exact dimensions — what
    /// the header-only fixtures cannot provide.
    pub fn decodable_png_atlas(version: u32) -> Vec<u8> {
        let (width, height) = format(version).atlas_dimensions();
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([90, 60, 30, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    /// The manifest bytes a local Codex home holds for one version.
    pub fn manifest(
        version: u32,
        pet_id: &str,
        display_name: &str,
        description: &str,
        atlas_basename: &str,
    ) -> Vec<u8> {
        let marker = format(version)
            .sprite_version_marker()
            .map(|value| format!("\n  \"spriteVersionNumber\": {},", value))
            .unwrap_or_default();
        format!(
            "{{\n  \"id\": \"{}\",\n  \"displayName\": \"{}\",\n  \"description\": \"{}\",{}\n  \"spritesheetPath\": \"{}\"\n}}\n",
            pet_id, display_name, description, marker, atlas_basename
        )
        .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{manifest, png_atlas, webp_atlas, webp_with_dimensions};
    use super::*;
    use crate::storage_catalog::domain::CatalogCoordinate;

    fn coordinate(version: u32) -> CatalogCoordinate {
        CatalogCoordinate::new("codex", KIND_KEY, FORMAT_KEY, version).unwrap()
    }

    fn write_pet(directory: &Path, version: u32, pet_id: &str, atlas_name: &str, atlas: &[u8]) {
        fs::create_dir_all(directory).unwrap();
        fs::write(
            directory.join("pet.json"),
            manifest(version, pet_id, "Pet", "One short sentence.", atlas_name),
        )
        .unwrap();
        fs::write(directory.join(atlas_name), atlas).unwrap();
    }

    fn codec(version: u32) -> CodexPetCodec {
        CodexPetCodec::for_version(version).unwrap()
    }

    #[test]
    fn png_and_webp_atlases_round_trip_for_both_versions() {
        for version in [1, 2] {
            for (name, bytes) in [
                ("spritesheet.webp", webp_atlas(version)),
                ("spritesheet.png", png_atlas(version)),
            ] {
                let temp = tempfile::tempdir().unwrap();
                let dir = temp.path().join("duck");
                write_pet(&dir, version, "duck", name, &bytes);
                let captured = codec(version)
                    .capture_directory(&dir, "provider-home")
                    .unwrap();
                let envelope = captured.to_envelope(&coordinate(version));
                codec(version).validate(&envelope).unwrap();
                assert_eq!(codec(version).display_name(&envelope).unwrap(), "Pet");
                assert_eq!(codec(version).member_name(&envelope).unwrap(), "duck");
                assert_eq!(
                    codec(version).resource_label(&envelope).as_deref(),
                    Some(format!("v{version}").as_str())
                );
                assert_eq!(
                    codec(version).description(&envelope).as_deref(),
                    Some("One short sentence.")
                );

                // Restore regenerates the canonical manifest for the same
                // version and identical atlas bytes; recapture agrees.
                let stage = temp.path().join("stage").join("duck");
                fs::create_dir_all(&stage).unwrap();
                codec(version)
                    .materialize(&stage, &envelope, &captured.files)
                    .unwrap();
                let restaged = codec(version)
                    .capture_directory(&stage, "provider-home")
                    .unwrap()
                    .to_envelope(&coordinate(version));
                assert_eq!(restaged, envelope);
                let text = fs::read_to_string(stage.join("pet.json")).unwrap();
                assert_eq!(text.contains("spriteVersionNumber"), version == 2);
            }
        }
    }

    #[test]
    fn a_version_codec_refuses_the_other_version_and_marker_mismatches_block() {
        let temp = tempfile::tempdir().unwrap();
        let v2 = temp.path().join("drake");
        write_pet(&v2, 2, "drake", "spritesheet.webp", &webp_atlas(2));
        let error = codec(1)
            .capture_directory(&v2, "provider-home")
            .unwrap_err();
        assert!(error.contains("is a v2 pet, not v1"), "{error}");
        codec(2).capture_directory(&v2, "provider-home").unwrap();

        // v1 atlas paired with a v2 marker.
        let dir = temp.path().join("mismatch");
        write_pet(&dir, 2, "mismatch", "spritesheet.webp", &webp_atlas(1));
        assert!(codec(2)
            .capture_directory(&dir, "provider-home")
            .unwrap_err()
            .contains("1536x1872"));
        // Unknown version markers are blocked, never defaulted.
        let dir = temp.path().join("future");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"future","displayName":"F","description":"d","spriteVersionNumber":3,"spritesheetPath":"s.webp"}"#,
        )
        .unwrap();
        fs::write(dir.join("s.webp"), webp_atlas(1)).unwrap();
        assert!(codec(1)
            .capture_directory(&dir, "provider-home")
            .unwrap_err()
            .contains("unsupported sprite version 3"));
        // Wrong dimensions entirely.
        let dir = temp.path().join("tiny");
        write_pet(
            &dir,
            1,
            "tiny",
            "spritesheet.webp",
            &webp_with_dimensions(10, 10),
        );
        assert!(codec(1)
            .capture_directory(&dir, "provider-home")
            .unwrap_err()
            .contains("requires exactly"));
        assert!(CodexPetCodec::for_version(3).is_none());
    }

    #[test]
    fn manifest_shape_violations_block() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("duck");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"d","spritesheetPath":"s.webp","theme":"pond"}"#,
        )
        .unwrap();
        fs::write(dir.join("s.webp"), webp_atlas(1)).unwrap();
        assert!(codec(1)
            .capture_directory(&dir, "provider-home")
            .unwrap_err()
            .contains("invalid"));
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"swan","displayName":"Duck","description":"d","spritesheetPath":"s.webp"}"#,
        )
        .unwrap();
        assert!(codec(1)
            .capture_directory(&dir, "provider-home")
            .unwrap_err()
            .contains("does not match its directory"));
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"d","spritesheetPath":"../escape.webp"}"#,
        )
        .unwrap();
        assert!(codec(1).capture_directory(&dir, "provider-home").is_err());
        fs::write(
            dir.join("pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"d","spritesheetPath":"missing.webp"}"#,
        )
        .unwrap();
        assert!(codec(1).capture_directory(&dir, "provider-home").is_err());
    }

    #[test]
    fn typed_validation_pins_the_atlas_entry_and_rejects_unknown_fields() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("duck");
        write_pet(&dir, 1, "duck", "spritesheet.webp", &webp_atlas(1));
        let envelope = codec(1)
            .capture_directory(&dir, "provider-home")
            .unwrap()
            .to_envelope(&coordinate(1));
        assert!(codec(2)
            .validate(&envelope)
            .unwrap_err()
            .contains("not a v2 pet"));
        let mut renamed = envelope.clone();
        renamed.files[0].path = "other.webp".to_string();
        assert!(codec(1)
            .validate(&renamed)
            .unwrap_err()
            .contains("declared basename"));
        let mut extra = envelope.clone();
        extra.descriptor = BoundedPortableObject::from_json(&serde_json::json!({
            "display_name": "Duck",
            "description": "d",
            "atlas_basename": "spritesheet.webp",
            "surprise": true
        }))
        .unwrap();
        assert!(codec(1).validate(&extra).is_err());
    }

    #[test]
    fn extra_files_are_excluded_and_reported() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("duck");
        write_pet(&dir, 1, "duck", "spritesheet.webp", &webp_atlas(1));
        fs::write(dir.join("notes.txt"), b"extra").unwrap();
        let captured = codec(1).capture_directory(&dir, "provider-home").unwrap();
        assert_eq!(captured.files.len(), 1);
        assert!(captured.notes[0].contains("notes.txt"));
    }

    #[test]
    fn copies_preserve_version_and_atlas_while_renaming_identity() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("duck");
        write_pet(&dir, 2, "duck", "spritesheet.webp", &webp_atlas(2));
        let captured = codec(2).capture_directory(&dir, "provider-home").unwrap();
        let copy = codec(2).rename(&captured, "duck-local-copy").unwrap();
        assert_eq!(copy.display_name, "duck-local-copy");
        let identity: CodexPetIdentity = copy.identity.decode().unwrap();
        assert_eq!(identity.pet_id, "duck-local-copy");
        assert_eq!(copy.files, captured.files);
        let envelope = copy.to_envelope(&coordinate(2));
        codec(2).validate(&envelope).unwrap();
        assert_ne!(
            envelope.resource_id,
            captured.to_envelope(&coordinate(2)).resource_id
        );
    }
}
