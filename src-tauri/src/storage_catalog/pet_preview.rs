//! Pet preview derivation: the atlas's first cell — the idle frame — cropped
//! and smoothly downscaled to twice the tile the card draws, then cached by
//! the atlas digest. One decode per content, wherever the bytes came from:
//! this Mac's provider home, or a storage payload fetched for a stored-only
//! row. Previews are decoration — callers degrade to the kind glyph on any
//! failure — but the derivation itself fails closed: bounded input, digest
//! verification before decode, and the format's pinned dimensions.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::domain::{CodexPetFormat, MAX_PET_ATLAS_BYTES};

/// 2x the 44x48 tile the card draws, at the atlas cell's own 192:208.
const PREVIEW_WIDTH: u32 = 96;
const PREVIEW_HEIGHT: u32 = 104;

/// A derived preview is a small PNG. A cache entry bigger than this is not
/// something this module wrote, and is not served.
const MAX_PREVIEW_BYTES: u64 = 262_144;

/// Machine-local, digest-keyed, and disposable: wiping `~/.mallard` wipes it,
/// and every entry can be re-derived from its source.
pub fn preview_cache_dir(user_home: &Path) -> PathBuf {
    user_home.join(".mallard").join("pet_preview_cache")
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{:02x}", byte));
    }
    output
}

fn data_uri(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        aws_smithy_types::base64::encode(png)
    )
}

/// Crop cell (0,0) of a full atlas and scale it to the preview size. Refuses
/// oversized input, undecodable bytes, and dimensions that are not exactly
/// the format's — a wrong-shaped sheet has no well-defined first cell.
pub(crate) fn derive_preview_png(
    format: CodexPetFormat,
    atlas_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    if atlas_bytes.len() as u64 > MAX_PET_ATLAS_BYTES {
        return Err("pet atlas exceeds its encoded size limit".to_string());
    }
    let image = image::load_from_memory(atlas_bytes)
        .map_err(|error| format!("decode pet atlas: {}", error))?;
    let (width, height) = format.atlas_dimensions();
    if image.width() != width || image.height() != height {
        return Err(format!(
            "pet atlas is {}x{}, not the {}x{} its format pins",
            image.width(),
            image.height(),
            width,
            height
        ));
    }
    let (columns, rows) = format.cell_grid();
    let cell = image.crop_imm(0, 0, width / columns, height / rows);
    let preview = cell.resize_exact(
        PREVIEW_WIDTH,
        PREVIEW_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let mut bytes = Vec::new();
    preview
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|error| format!("encode pet preview: {}", error))?;
    Ok(bytes)
}

/// The preview for one atlas digest as a PNG data URI, deriving and filling
/// the cache on the first ask. `read_atlas` supplies the full atlas bytes
/// only on a cache miss, and whatever it returns is verified against the
/// digest before anything decodes it — a changed file on disk, or a storage
/// that lies, must not fill this digest's cache slot.
pub(crate) fn cached_preview_data_uri(
    cache_dir: &Path,
    format: CodexPetFormat,
    atlas_sha256: &str,
    read_atlas: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<String, String> {
    // The digest is also the cache file name, so nothing that is not a plain
    // lowercase sha-256 may reach the filesystem.
    if atlas_sha256.len() != 64
        || !atlas_sha256
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err("pet atlas digest is not a lowercase sha-256".to_string());
    }
    let path = cache_dir.join(format!("{}.png", atlas_sha256));
    if let Ok(metadata) = fs::metadata(&path) {
        if metadata.is_file() && metadata.len() <= MAX_PREVIEW_BYTES {
            if let Ok(bytes) = fs::read(&path) {
                return Ok(data_uri(&bytes));
            }
        }
    }
    let atlas = read_atlas()?;
    if sha256_hex(&atlas) != atlas_sha256 {
        return Err("pet atlas bytes do not match their digest".to_string());
    }
    let png = derive_preview_png(format, &atlas)?;
    fs::create_dir_all(cache_dir)
        .map_err(|error| format!("create pet preview cache: {}", error))?;
    let temporary = cache_dir.join(format!(".{}.tmp-{}", atlas_sha256, std::process::id()));
    fs::write(&temporary, &png).map_err(|error| format!("write pet preview cache: {}", error))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("commit pet preview cache: {}", error))?;
    Ok(data_uri(&png))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real, decodable PNG atlas with cell (0,0) painted a known colour and
    /// the rest transparent — what the header-only fixtures cannot provide.
    fn png_atlas(format: CodexPetFormat, cell_rgba: [u8; 4]) -> Vec<u8> {
        let (width, height) = format.atlas_dimensions();
        let (columns, rows) = format.cell_grid();
        let (cell_width, cell_height) = (width / columns, height / rows);
        let mut image = image::RgbaImage::new(width, height);
        for y in 0..cell_height {
            for x in 0..cell_width {
                image.put_pixel(x, y, image::Rgba(cell_rgba));
            }
        }
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn derives_the_first_cell_at_preview_size_for_both_formats() {
        for format in [CodexPetFormat::V1, CodexPetFormat::V2] {
            let atlas = png_atlas(format, [200, 40, 40, 255]);
            let png = derive_preview_png(format, &atlas).unwrap();
            let preview = image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(
                (preview.width(), preview.height()),
                (PREVIEW_WIDTH, PREVIEW_HEIGHT)
            );
            // Every pixel comes from the painted first cell, so the corner
            // keeps its colour after the smooth downscale.
            assert_eq!(preview.get_pixel(0, 0).0, [200, 40, 40, 255]);
            assert_eq!(
                preview.get_pixel(PREVIEW_WIDTH - 1, PREVIEW_HEIGHT - 1).0,
                [200, 40, 40, 255]
            );
        }
    }

    #[test]
    fn refuses_dimensions_the_format_does_not_pin() {
        // A v2-shaped sheet is not a v1 pet, even though it decodes fine.
        let atlas = png_atlas(CodexPetFormat::V2, [0, 0, 0, 255]);
        let error = derive_preview_png(CodexPetFormat::V1, &atlas).unwrap_err();
        assert!(error.contains("its format pins"), "{}", error);
    }

    #[test]
    fn refuses_oversized_and_undecodable_input() {
        let oversized = vec![0_u8; (MAX_PET_ATLAS_BYTES + 1) as usize];
        assert!(derive_preview_png(CodexPetFormat::V1, &oversized)
            .unwrap_err()
            .contains("size limit"));
        // The header-only test fixture is exactly the corrupt-in-practice
        // case: valid dimensions in the header, no image data behind them.
        let header_only = super::super::codex_pet::fixtures::webp_atlas(CodexPetFormat::V1);
        assert!(derive_preview_png(CodexPetFormat::V1, &header_only)
            .unwrap_err()
            .contains("decode pet atlas"));
    }

    #[test]
    fn caches_by_digest_and_reads_the_source_once() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = temporary.path().join("previews");
        let atlas = png_atlas(CodexPetFormat::V1, [10, 120, 60, 255]);
        let digest = sha256_hex(&atlas);
        let mut reads = 0;
        let first = cached_preview_data_uri(&cache, CodexPetFormat::V1, &digest, || {
            reads += 1;
            Ok(atlas.clone())
        })
        .unwrap();
        assert!(first.starts_with("data:image/png;base64,"));
        assert_eq!(reads, 1);
        let second = cached_preview_data_uri(&cache, CodexPetFormat::V1, &digest, || {
            reads += 1;
            Ok(atlas.clone())
        })
        .unwrap();
        assert_eq!(second, first);
        assert_eq!(reads, 1, "a cache hit must not read the source again");
    }

    #[test]
    fn refuses_bytes_that_do_not_match_their_digest() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = temporary.path().join("previews");
        let atlas = png_atlas(CodexPetFormat::V1, [1, 2, 3, 255]);
        let error = cached_preview_data_uri(&cache, CodexPetFormat::V1, &"a".repeat(64), || {
            Ok(atlas.clone())
        })
        .unwrap_err();
        assert!(error.contains("do not match their digest"), "{}", error);
        assert!(
            fs::read_dir(&cache).is_err(),
            "nothing may be cached for a digest the bytes do not carry"
        );
    }

    #[test]
    fn refuses_a_digest_that_is_not_plain_hex() {
        let temporary = tempfile::tempdir().unwrap();
        let error =
            cached_preview_data_uri(temporary.path(), CodexPetFormat::V1, "../escape", || {
                unreachable!("the digest is rejected before any read")
            })
            .unwrap_err();
        assert!(error.contains("lowercase sha-256"), "{}", error);
    }
}
