//! Project-scoped media opened without following links and read in bounded
//! ranges. Videos may be up to 500 MiB; playback never copies the whole file.

use crate::persistence::LocalStateRepository;
use crate::project_activity::resolve_activity_project;
use crate::session_detail::{
    canonical_project_markdown_path, SessionDetailError, SessionDetailErrorCode,
};
use http_range::HttpRange;
use mallard_domain::LocalProjectId;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const MAX_MEDIA_RANGE_BYTES: u64 = 1024 * 1024;
pub const MAX_PROJECT_VIDEO_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectMarkdownMedia {
    pub mime_type: String,
    pub size_bytes: u64,
}

pub struct ProjectMediaFile {
    file: File,
    pub metadata: ProjectMarkdownMedia,
}

pub struct ProjectMediaRange {
    pub status: u16,
    pub content_range: Option<String>,
    pub content_length: u64,
    pub bytes: Vec<u8>,
}

fn invalid(message: &str) -> SessionDetailError {
    SessionDetailError::new(SessionDetailErrorCode::InvalidRequest, message)
}

fn unreadable() -> SessionDetailError {
    SessionDetailError::new(
        SessionDetailErrorCode::NotFound,
        "The linked media file is no longer readable.",
    )
}

pub fn open_project_markdown_media(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    relative_path: &str,
) -> Result<ProjectMediaFile, SessionDetailError> {
    let project = resolve_activity_project(repository, project_id).map_err(|_| {
        SessionDetailError::new(
            SessionDetailErrorCode::OwnershipChanged,
            "The project binding changed; open Project Settings.",
        )
    })?;
    open_project_media_at_root(&project.project_root, relative_path)
}

/// Only the project identifier and a relative descendant travel in a media
/// URL. Decode exactly once, before the same path validation as the command.
pub fn open_project_media_request(
    repository: &LocalStateRepository,
    request_path: &str,
) -> Result<ProjectMediaFile, SessionDetailError> {
    if request_path.len() > 16 * 1024 {
        return Err(invalid("The media URL is too long."));
    }
    let decoded = percent_encoding::percent_decode_str(
        request_path.strip_prefix('/').unwrap_or(request_path),
    )
    .decode_utf8()
    .map_err(|_| invalid("The media URL is invalid."))?;
    let (project, path) = decoded
        .split_once('/')
        .ok_or_else(|| invalid("The media URL is invalid."))?;
    let project =
        LocalProjectId::parse(project).map_err(|_| invalid("The media project is invalid."))?;
    open_project_markdown_media(repository, &project, path)
}

fn open_project_media_at_root(
    root: &Path,
    relative_path: &str,
) -> Result<ProjectMediaFile, SessionDetailError> {
    let path = canonical_project_markdown_path(root, relative_path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(&path).map_err(|_| unreadable())?;
    let metadata = file.metadata().map_err(|_| unreadable())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("Only regular project media files can play."));
    }
    // Recheck the entire no-follow path after opening, before reading bytes.
    if canonical_project_markdown_path(root, relative_path)? != path {
        return Err(invalid("The linked media path changed while opening."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let current = fs::symlink_metadata(&path).map_err(|_| unreadable())?;
        if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
            return Err(invalid("The linked media file changed while opening."));
        }
    }
    let mut header = [0u8; 16];
    let count = metadata.len().min(header.len() as u64) as usize;
    file.read_exact(&mut header[..count])
        .map_err(|_| unreadable())?;
    let mime_type = media_mime(&path, &header[..count])
        .ok_or_else(|| invalid("This file is not a supported audio or video container."))?;
    if mime_type.starts_with("video/") && metadata.len() > MAX_PROJECT_VIDEO_BYTES {
        return Err(SessionDetailError::new(
            SessionDetailErrorCode::ResponseTooLarge,
            "Videos up to 500 MiB can play here.",
        ));
    }
    Ok(ProjectMediaFile {
        file,
        metadata: ProjectMarkdownMedia {
            mime_type: mime_type.to_string(),
            size_bytes: metadata.len(),
        },
    })
}

fn media_mime(path: &Path, bytes: &[u8]) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let iso = bytes.len() >= 8 && &bytes[4..8] == b"ftyp";
    let ogg = bytes.starts_with(b"OggS");
    let ebml = bytes.starts_with(b"\x1a\x45\xdf\xa3");
    let mpeg = bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0;
    match extension.as_str() {
        "mp4" | "m4v" if iso => Some("video/mp4"),
        "mov" if iso => Some("video/quicktime"),
        "m4a" if iso => Some("audio/mp4"),
        "webm" if ebml => Some("video/webm"),
        "mkv" if ebml => Some("video/x-matroska"),
        "ogv" if ogg => Some("video/ogg"),
        "ogg" | "oga" | "opus" if ogg => Some("audio/ogg"),
        "mp3" if mpeg || bytes.starts_with(b"ID3") => Some("audio/mpeg"),
        "aac" if mpeg => Some("audio/aac"),
        "flac" if bytes.starts_with(b"fLaC") => Some("audio/flac"),
        "wav" if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" => {
            Some("audio/wav")
        }
        _ => None,
    }
}

impl ProjectMediaFile {
    /// Native players use byte ranges, including the final MP4 metadata when
    /// seeking. HEAD never reads a body. Large non-range requests are refused
    /// instead of allocating an unbounded whole-file response.
    pub fn read_range(
        &mut self,
        range: Option<&str>,
        head: bool,
    ) -> Result<ProjectMediaRange, SessionDetailError> {
        let size = self.metadata.size_bytes;
        if head {
            return Ok(ProjectMediaRange {
                status: 200,
                content_range: None,
                content_length: size,
                bytes: Vec::new(),
            });
        }
        let selected = match range {
            Some(header) if header.len() <= 256 && !header.contains(',') => {
                HttpRange::parse(header, size)
                    .ok()
                    .filter(|ranges| ranges.len() == 1)
                    .and_then(|ranges| ranges.first().copied())
            }
            None if size <= MAX_MEDIA_RANGE_BYTES => Some(HttpRange {
                start: 0,
                length: size,
            }),
            _ => None,
        };
        let Some(selected) = selected.filter(|range| range.length > 0 && range.start < size) else {
            return Ok(ProjectMediaRange {
                status: 416,
                content_range: Some(format!("bytes */{size}")),
                content_length: 0,
                bytes: Vec::new(),
            });
        };
        let length = selected
            .length
            .min(MAX_MEDIA_RANGE_BYTES)
            .min(size - selected.start);
        self.file
            .seek(SeekFrom::Start(selected.start))
            .map_err(|_| unreadable())?;
        let mut bytes = vec![0; length as usize];
        self.file.read_exact(&mut bytes).map_err(|_| unreadable())?;
        Ok(ProjectMediaRange {
            status: if range.is_some() { 206 } else { 200 },
            content_range: range.map(|_| {
                format!(
                    "bytes {}-{}/{size}",
                    selected.start,
                    selected.start + length - 1
                )
            }),
            content_length: length,
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn supported_media_headers_get_their_container_type_and_small_response() {
        let temp = tempfile::tempdir().unwrap();
        let samples: [(&str, &[u8], &str); 14] = [
            ("song.wav", b"RIFF\x24\0\0\0WAVEfmt ", "audio/wav"),
            ("song.mp3", b"ID3\x04\0\0\0\0\0\0", "audio/mpeg"),
            ("frames.mp3", b"\xFF\xFB\x90\x64\0", "audio/mpeg"),
            ("voice.aac", b"\xFF\xF1\x50\x80\0", "audio/aac"),
            ("take.flac", b"fLaC\0\0\0\x22", "audio/flac"),
            ("loop.ogg", b"OggS\0\x02\0\0", "audio/ogg"),
            ("loop.oga", b"OggS\0\x02\0\0", "audio/ogg"),
            ("voice.opus", b"OggS\0\x02\0\0", "audio/ogg"),
            ("clip.mp4", b"\0\0\0\x18ftypmp42", "video/mp4"),
            ("clip.mov", b"\0\0\0\x18ftypqt  ", "video/quicktime"),
            ("track.m4a", b"\0\0\0\x18ftypM4A ", "audio/mp4"),
            ("clip.webm", b"\x1A\x45\xDF\xA3\x9F", "video/webm"),
            ("clip.mkv", b"\x1A\x45\xDF\xA3\x9F", "video/x-matroska"),
            ("clip.ogv", b"OggS\0\x02\0\0", "video/ogg"),
        ];
        for (name, bytes, mime) in samples {
            fs::write(temp.path().join(name), bytes).unwrap();
            let mut media = open_project_media_at_root(temp.path(), name).unwrap();
            assert_eq!(media.metadata.mime_type, mime);
            let response = media.read_range(None, false).unwrap();
            assert_eq!(response.status, 200);
            assert_eq!(response.content_length, bytes.len() as u64);
            assert_eq!(response.content_range, None);
            assert_eq!(response.bytes, bytes);
        }
    }

    #[test]
    fn a_500_mib_video_is_accepted_and_its_ranges_are_bounded_and_seekable() {
        let temp = tempfile::tempdir().unwrap();
        let mut file = File::create(temp.path().join("film.mp4")).unwrap();
        file.write_all(b"\0\0\0\x18ftypmp42").unwrap();
        let size = 524_288_000;
        file.set_len(size).unwrap();
        file.seek(SeekFrom::End(-4)).unwrap();
        file.write_all(b"tail").unwrap();
        drop(file);
        let mut media = open_project_media_at_root(temp.path(), "film.mp4").unwrap();
        assert_eq!(media.metadata.size_bytes, size);
        assert_eq!(media.metadata.mime_type, "video/mp4");
        let head = media.read_range(None, true).unwrap();
        assert_eq!(head.content_length, size);
        assert!(head.bytes.is_empty());
        let start = media.read_range(Some("bytes=0-"), false).unwrap();
        assert_eq!(start.status, 206);
        assert_eq!(start.bytes.len() as u64, MAX_MEDIA_RANGE_BYTES);
        assert_eq!(
            start.content_range.as_deref(),
            Some("bytes 0-1048575/524288000")
        );
        let tail = media.read_range(Some("bytes=-4"), false).unwrap();
        assert_eq!(tail.bytes, b"tail");
        assert_eq!(
            media
                .read_range(Some("bytes=524287996-999999999"), false)
                .unwrap()
                .bytes,
            b"tail"
        );
        for range in [
            None,
            Some("bytes=524288000-"),
            Some("bytes=9-1"),
            Some("bytes=0-1,3-4"),
            Some("bytes=-0"),
            Some("bytes=wat"),
        ] {
            assert_eq!(media.read_range(range, false).unwrap().status, 416);
        }
    }

    #[test]
    fn a_video_one_byte_over_500_mib_is_rejected_including_after_it_grows() {
        let temp = tempfile::tempdir().unwrap();
        let mut file = File::create(temp.path().join("film.mp4")).unwrap();
        file.write_all(b"\0\0\0\x18ftypmp42").unwrap();
        file.set_len(524_288_000).unwrap();
        assert!(open_project_media_at_root(temp.path(), "film.mp4").is_ok());
        file.set_len(524_288_001).unwrap();
        let error = open_project_media_at_root(temp.path(), "film.mp4")
            .err()
            .unwrap();
        assert_eq!(error.code, SessionDetailErrorCode::ResponseTooLarge);
        assert_eq!(error.message, "Videos up to 500 MiB can play here.");

        // The requested video limit does not change audio playback.
        fs::write(temp.path().join("track.flac"), b"fLaC\0\0\0\x22").unwrap();
        OpenOptions::new()
            .write(true)
            .open(temp.path().join("track.flac"))
            .unwrap()
            .set_len(524_288_001)
            .unwrap();
        assert!(open_project_media_at_root(temp.path(), "track.flac").is_ok());
    }

    #[test]
    fn media_rejects_outside_paths_missing_files_and_disguised_content() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("fake.mp4"), b"<script>not media</script>").unwrap();
        for path in ["../outside.mp4", "fake.mp4", "."] {
            assert!(open_project_media_at_root(temp.path(), path).is_err());
        }
        assert!(
            matches!(open_project_media_at_root(temp.path(), "missing.mp4"), Err(error) if error.code == SessionDetailErrorCode::NotFound)
        );
    }

    #[cfg(unix)]
    #[test]
    fn media_rejects_file_and_directory_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("output")).unwrap();
        fs::write(temp.path().join("output/clip.mp4"), b"\0\0\0\x18ftypmp42").unwrap();
        symlink("output/clip.mp4", temp.path().join("link.mp4")).unwrap();
        symlink("output", temp.path().join("alias")).unwrap();
        for path in ["link.mp4", "alias/clip.mp4"] {
            assert!(
                matches!(open_project_media_at_root(temp.path(), path), Err(error) if error.code == SessionDetailErrorCode::InvalidRequest)
            );
        }
    }
}
