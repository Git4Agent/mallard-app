//! Format-agnostic JSONL transcript byte plumbing shared by the per-provider
//! tail engines: bounded whole-file reads, cursor tail-window reads, pinned
//! range reads, and record-boundary arithmetic. Everything here is pure byte
//! I/O under one safety contract — canonical no-follow regular files within
//! the bounded transcript limit — with sync-metrics attribution. Provider
//! semantics (append validation, chain codecs) live in the provider modules.

use std::fs;
use std::path::Path;

use crate::app_settings::conversation_file_limit_bytes;
use mallard_domain::RootPhysicalMapping;

pub const PROBE_WINDOW_BYTES: u64 = 4_096;

/// How many times a moving-EOF read retries before failing closed.
const TAIL_WINDOW_READ_RETRIES: usize = 3;

/// One bounded read of a transcript tail: the saved probe window plus every
/// byte from the consumed cursor through the bounded EOF, addressed by the
/// absolute file offset the slice starts at.
#[derive(Clone, Debug)]
pub struct TailWindow {
    pub file_len: u64,
    pub window_offset: u64,
    pub bytes: Vec<u8>,
}

pub(crate) fn open_no_follow(path: &Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|error| format!("open session transcript '{}': {}", path.display(), error))
}

/// Read one transcript with the module's safety contract: a canonical
/// no-follow regular file within the bounded transcript limit.
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;

    let mut file = open_no_follow(path)?;
    let before = file
        .metadata()
        .map_err(|error| format!("inspect session transcript '{}': {}", path.display(), error))?;
    if !before.is_file() {
        return Err(format!(
            "session transcript '{}' is not a regular file",
            path.display()
        ));
    }
    if before.len() > conversation_file_limit_bytes() {
        return Err(format!(
            "session transcript '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take(conversation_file_limit_bytes().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read session transcript '{}': {}", path.display(), error))?;
    let after = file.metadata().map_err(|error| {
        format!(
            "reinspect session transcript '{}': {}",
            path.display(),
            error
        )
    })?;
    if bytes.len() as u64 > conversation_file_limit_bytes() {
        return Err(format!(
            "session transcript '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    if after.len() != before.len() || bytes.len() as u64 != before.len() {
        return Err(format!(
            "session transcript '{}' changed while it was read",
            path.display()
        ));
    }
    crate::sync_metrics::record_transcript_bytes_read(path, bytes.len() as u64);
    Ok(bytes)
}

/// Read a saved cursor's probe window and appended tail without reading the
/// consumed prefix: open the exact path without following a final symlink,
/// require a regular file within the transcript bound, read from the probe
/// window's start through EOF, and re-stat the open handle afterwards,
/// retrying a bounded number of times while the EOF is moving. A file
/// already shorter than the cursor returns an empty window whose `file_len`
/// lets the provider refresh classify the truncation.
pub fn read_tail_window(path: &Path, mapping: &RootPhysicalMapping) -> Result<TailWindow, String> {
    use std::io::{Read, Seek, SeekFrom};
    let window_offset = mapping.consumed_byte_offset - mapping.probe_window_len;
    let file = open_no_follow(path)?;
    let handle_meta = file
        .metadata()
        .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))?;
    if !handle_meta.is_file() {
        return Err(format!(
            "session transcript '{}' is not a regular file",
            path.display()
        ));
    }
    let mut file = file;
    let mut attempts = 0;
    loop {
        let len_before = file
            .metadata()
            .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))?
            .len();
        if len_before > conversation_file_limit_bytes() {
            return Err(format!(
                "session transcript '{}' exceeds the bounded transcript limit",
                path.display()
            ));
        }
        if len_before < mapping.consumed_byte_offset {
            return Ok(TailWindow {
                file_len: len_before,
                window_offset,
                bytes: Vec::new(),
            });
        }
        file.seek(SeekFrom::Start(window_offset))
            .map_err(|error| format!("seek session transcript '{}': {}", path.display(), error))?;
        let expected = len_before - window_offset;
        let mut bytes = Vec::with_capacity(expected as usize);
        (&file)
            .take(expected)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read session transcript '{}': {}", path.display(), error))?;
        let len_after = file
            .metadata()
            .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))?
            .len();
        if len_after == len_before && bytes.len() as u64 == expected {
            crate::sync_metrics::record_probe_bytes_read(path, mapping.probe_window_len);
            crate::sync_metrics::record_append_bytes_read(
                path,
                expected.saturating_sub(mapping.probe_window_len),
            );
            return Ok(TailWindow {
                file_len: len_before,
                window_offset,
                bytes,
            });
        }
        attempts += 1;
        if attempts > TAIL_WINDOW_READ_RETRIES {
            return Err(format!(
                "session transcript '{}' kept changing during the bounded tail read",
                path.display()
            ));
        }
    }
}

/// Read the last `max_window_bytes` of a transcript (the whole file when it
/// is shorter) under the module's safety contract, retrying a bounded number
/// of times while the EOF is moving. Addressed by size rather than by a
/// saved cursor: this is the metadata scan's bounded tail read, so the
/// bytes are attributed as transcript bytes, not probe/append bytes.
pub fn read_suffix_window(path: &Path, max_window_bytes: u64) -> Result<TailWindow, String> {
    use std::io::{Read, Seek, SeekFrom};
    let file = open_no_follow(path)?;
    let handle_meta = file
        .metadata()
        .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))?;
    if !handle_meta.is_file() {
        return Err(format!(
            "session transcript '{}' is not a regular file",
            path.display()
        ));
    }
    let mut file = file;
    let mut attempts = 0;
    loop {
        let len_before = file
            .metadata()
            .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))?
            .len();
        if len_before > conversation_file_limit_bytes() {
            return Err(format!(
                "session transcript '{}' exceeds the bounded transcript limit",
                path.display()
            ));
        }
        let window_offset = len_before.saturating_sub(max_window_bytes);
        file.seek(SeekFrom::Start(window_offset))
            .map_err(|error| format!("seek session transcript '{}': {}", path.display(), error))?;
        let expected = len_before - window_offset;
        let mut bytes = Vec::with_capacity(expected as usize);
        (&file)
            .take(expected)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read session transcript '{}': {}", path.display(), error))?;
        let len_after = file
            .metadata()
            .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))?
            .len();
        if len_after == len_before && bytes.len() as u64 == expected {
            crate::sync_metrics::record_transcript_bytes_read(path, expected);
            return Ok(TailWindow {
                file_len: len_before,
                window_offset,
                bytes,
            });
        }
        attempts += 1;
        if attempts > TAIL_WINDOW_READ_RETRIES {
            return Err(format!(
                "session transcript '{}' kept changing during the bounded tail read",
                path.display()
            ));
        }
    }
}

/// Read one absolute byte range of a transcript under the same no-follow
/// contract, for re-proving an accepted-to-current raw interval after the
/// cursor advanced. The caller must verify the overlap with bytes it
/// already trusts.
pub fn read_range(path: &Path, start: u64, end: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    if end < start || end > conversation_file_limit_bytes() {
        return Err(format!(
            "session transcript '{}' range request is out of bounds",
            path.display()
        ));
    }
    let mut file = open_no_follow(path)?;
    let handle_meta = file
        .metadata()
        .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))?;
    if !handle_meta.is_file() {
        return Err(format!(
            "session transcript '{}' is not a regular file",
            path.display()
        ));
    }
    if handle_meta.len() < end {
        return Err(format!(
            "session transcript '{}' is shorter than the pinned range",
            path.display()
        ));
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|error| format!("seek session transcript '{}': {}", path.display(), error))?;
    let mut bytes = Vec::with_capacity((end - start) as usize);
    (&file)
        .take(end - start)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read session transcript '{}': {}", path.display(), error))?;
    if bytes.len() as u64 != end - start {
        return Err(format!(
            "session transcript '{}' raced the pinned range read",
            path.display()
        ));
    }
    crate::sync_metrics::record_append_bytes_read(path, end - start);
    Ok(bytes)
}

/// Position immediately after the final newline: the safe record end.
pub fn safe_record_end(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|position| position as u64 + 1)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_window_addresses_the_tail_by_size() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("transcript.jsonl");
        fs::write(&path, b"aaaa\nbbbb\ncccc\n").unwrap();

        let whole = read_suffix_window(&path, 64).unwrap();
        assert_eq!(whole.window_offset, 0);
        assert_eq!(whole.file_len, 15);
        assert_eq!(whole.bytes, b"aaaa\nbbbb\ncccc\n");

        let tail = read_suffix_window(&path, 6).unwrap();
        assert_eq!(tail.window_offset, 9);
        assert_eq!(tail.file_len, 15);
        assert_eq!(tail.bytes, b"\ncccc\n");

        let empty = read_suffix_window(&path, 0).unwrap();
        assert_eq!(empty.window_offset, 15);
        assert!(empty.bytes.is_empty());
    }

    #[test]
    fn suffix_window_rejects_non_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        let error = read_suffix_window(temp.path(), 64).unwrap_err();
        assert!(
            error.contains("not a regular file") || error.contains("open session transcript"),
            "{error}"
        );
    }
}
