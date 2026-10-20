//! Format-agnostic JSONL transcript byte plumbing shared by the per-provider
//! tail engines: bounded whole-file reads, cursor tail-window reads, pinned
//! range reads, and record-boundary arithmetic. Everything here is pure byte
//! I/O under one safety contract — canonical no-follow regular files within
//! the bounded transcript limit — with sync-metrics attribution. Provider
//! semantics (append validation, chain codecs) live in the provider modules.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mallard_domain::{RootPhysicalMapping, SourceFileIdentity, SourceFileState};

pub const PROBE_WINDOW_BYTES: u64 = 4_096;

/// How many times a moving-EOF read retries before failing closed.
const TAIL_WINDOW_READ_RETRIES: usize = 3;

/// One bounded read of a transcript tail: the saved probe window plus every
/// byte from the consumed cursor through the bounded EOF, addressed by the
/// absolute file offset the slice starts at. `stamp` is the open handle's
/// stamp, identical before and after the read.
#[derive(Clone, Debug)]
pub struct TailWindow {
    pub file_len: u64,
    pub window_offset: u64,
    pub bytes: Vec<u8>,
    pub stamp: SourceStamp,
}

pub(crate) fn open_no_follow(path: &Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
        .open(path)
        .map_err(|error| format!("open session transcript '{}': {}", path.display(), error))
}

/// Whether the path names a transcript Codex compressed in place: the same
/// JSONL as one zstd stream, under the plain name plus `.zst`. Every reader
/// here decodes such a file and addresses its decoded bytes; the stamp
/// stays the file's own.
pub fn is_compressed(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl.zst"))
}

/// The decoded length a compressed transcript's frame header declares, when
/// it declares one. A hint for refusing an oversized file before it is
/// read, never a bound: the decoder enforces the limit on its own.
pub fn declared_decoded_len(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut header = [0_u8; 18];
    let read = open_no_follow(path).ok()?.read(&mut header).ok()?;
    zstd::zstd_safe::get_frame_content_size(&header[..read])
        .ok()
        .flatten()
}

/// Bytes as Codex's compressor writes them: one zstd stream at level 3 with
/// the plain length pledged into its frame header. Test fixtures only.
#[cfg(any(test, feature = "test-support"))]
pub fn compress_like_codex(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    encoder
        .set_pledged_src_size(Some(bytes.len() as u64))
        .unwrap();
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// Machine-local identity of the exact captured file. Never portable.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceStamp {
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(any(unix, windows))]
    identity: (u64, u64, i64, i64),
}

impl SourceStamp {
    /// The length of the file on disk: of the stream when it is compressed.
    pub fn file_len(&self) -> u64 {
        self.len
    }

    pub fn mtime_secs(&self) -> u64 {
        self.modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        Self::from_file(&open_no_follow(path)?)
    }

    /// The same stamp without reading the body. Windows needs a no-follow
    /// handle to query file identity and change time.
    pub fn stat(path: &Path) -> Result<Self, String> {
        #[cfg(windows)]
        return Self::read(path);
        #[cfg(not(windows))]
        Self::from_metadata(&fs::symlink_metadata(path).map_err(|error| error.to_string())?)
    }

    #[cfg(test)]
    pub(crate) fn for_tests(len: u64) -> Self {
        Self {
            len,
            modified: std::time::UNIX_EPOCH,
            #[cfg(any(unix, windows))]
            identity: (0, 0, 0, 0),
        }
    }

    /// The state a checkpoint saves beside a cursor into `path`: none for
    /// a compressed transcript, whose cursor addresses decoded bytes the
    /// file's own size cannot cover. A cursor without a state is never
    /// reused, so such a source is read in full on every observation.
    pub fn saved_state(&self, path: &Path) -> Option<SourceFileState> {
        (!is_compressed(path)).then(|| self.file_state())
    }

    /// The plain-data form a checkpoint saves beside its cursor.
    pub fn file_state(&self) -> SourceFileState {
        let (modified_secs, modified_nanos) =
            match self.modified.duration_since(std::time::UNIX_EPOCH) {
                Ok(after) => (whole_seconds(after), after.subsec_nanos()),
                // Before the epoch the seconds round down, so the
                // nanoseconds still count forward.
                Err(before) => match before.duration() {
                    before if before.subsec_nanos() == 0 => (-whole_seconds(before), 0),
                    before => (
                        -whole_seconds(before) - 1,
                        1_000_000_000 - before.subsec_nanos(),
                    ),
                },
            };
        SourceFileState {
            size: self.len,
            modified_secs,
            modified_nanos,
            #[cfg(any(unix, windows))]
            identity: Some(SourceFileIdentity {
                device: self.identity.0,
                inode: self.identity.1,
                changed_secs: self.identity.2,
                changed_nanos: self.identity.3,
            }),
            #[cfg(not(any(unix, windows)))]
            identity: None::<SourceFileIdentity>,
        }
    }

    /// Whether a cursor saved from a file in the `saved` state may be used
    /// on the file this stamp describes: the same file, untouched or grown
    /// since. A file that changed without growing, shrank, or is another
    /// file is read in full and judged by its content; so is one whose
    /// metadata alone changed, because a stamp cannot tell that from an
    /// edit. An edit that also grows the file passes, and past this check
    /// the probe window witnesses only the bytes just before the cursor. A
    /// platform that reports no file identity compares size and
    /// modification time alone.
    pub fn continues(&self, saved: &SourceFileState) -> bool {
        let now = self.file_state();
        let file = |state: &SourceFileState| {
            state
                .identity
                .as_ref()
                .map(|identity| (identity.device, identity.inode))
        };
        file(&now) == file(saved) && (now == *saved || now.size > saved.size)
    }

    #[cfg(not(windows))]
    fn from_file(file: &fs::File) -> Result<Self, String> {
        Self::from_metadata(&file.metadata().map_err(|error| error.to_string())?)
    }

    #[cfg(windows)]
    fn from_file(file: &fs::File) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
            BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
        };

        let meta = file.metadata().map_err(|error| error.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("session transcript is not a regular file".to_string());
        }
        let mut identity = BY_HANDLE_FILE_INFORMATION::default();
        let mut basic = FILE_BASIC_INFO::default();
        // Both queries use this live handle, so a path replacement cannot
        // lend its identity to the file whose bytes we read.
        let success = unsafe {
            GetFileInformationByHandle(file.as_raw_handle() as _, &mut identity) != 0
                && GetFileInformationByHandleEx(
                    file.as_raw_handle() as _,
                    FileBasicInfo,
                    (&mut basic as *mut FILE_BASIC_INFO).cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                ) != 0
        };
        if !success {
            return Err(format!(
                "inspect session transcript identity: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            len: meta.len(),
            modified: meta.modified().map_err(|error| error.to_string())?,
            identity: (
                u64::from(identity.dwVolumeSerialNumber),
                (u64::from(identity.nFileIndexHigh) << 32) | u64::from(identity.nFileIndexLow),
                basic.ChangeTime.div_euclid(10_000_000) - 11_644_473_600,
                basic.ChangeTime.rem_euclid(10_000_000) * 100,
            ),
        })
    }

    #[cfg(not(windows))]
    fn from_metadata(meta: &fs::Metadata) -> Result<Self, String> {
        if !meta.is_file() {
            return Err("session transcript is not a regular file".to_string());
        }
        Ok(Self {
            len: meta.len(),
            modified: meta.modified().map_err(|error| error.to_string())?,
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec())
            },
        })
    }

    pub fn verify(&self, path: &Path) -> Result<(), String> {
        let file = open_no_follow(path)?;
        if &Self::from_file(&file)? != self {
            return Err(
                "session transcript changed after capture; refresh and push again".to_string(),
            );
        }
        Ok(())
    }
}

fn whole_seconds(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

/// Every file one derivation read, each under the stamp it was read at. The
/// result may be reused only while all of them still carry those stamps,
/// checked at the moment of reuse. A file that could not be stamped, or one
/// path read under two stamps, leaves the set unpinned: the result serves
/// the operation that derived it and is never reused.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SourceStamps {
    stamps: BTreeMap<PathBuf, SourceStamp>,
    unpinned: bool,
}

impl SourceStamps {
    pub fn of(path: &Path, stamp: &SourceStamp) -> Self {
        let mut stamps = Self::default();
        stamps.record(path, stamp);
        stamps
    }

    pub fn record(&mut self, path: &Path, stamp: &SourceStamp) {
        match self.stamps.get(path) {
            Some(recorded) if recorded != stamp => self.unpinned = true,
            Some(_) => {}
            None => {
                self.stamps.insert(path.to_path_buf(), stamp.clone());
            }
        }
    }

    /// A file the derivation consulted and could not stamp.
    pub fn record_unstamped(&mut self) {
        self.unpinned = true;
    }

    pub fn extend(&mut self, other: &Self) {
        self.unpinned |= other.unpinned;
        for (path, stamp) in &other.stamps {
            self.record(path, stamp);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stamps.is_empty() && !self.unpinned
    }

    /// Whether the set names at least one file and each under one stamp.
    pub fn is_pinned(&self) -> bool {
        !self.unpinned && !self.stamps.is_empty()
    }

    /// Whether every file still carries its recorded stamp.
    pub fn hold(&self) -> bool {
        self.is_pinned()
            && self
                .stamps
                .iter()
                .all(|(path, stamp)| SourceStamp::stat(path).is_ok_and(|now| now == *stamp))
    }
}

impl<'a> FromIterator<(&'a PathBuf, &'a SourceStamp)> for SourceStamps {
    fn from_iter<I: IntoIterator<Item = (&'a PathBuf, &'a SourceStamp)>>(sources: I) -> Self {
        let mut stamps = Self::default();
        for (path, stamp) in sources {
            stamps.record(path, stamp);
        }
        stamps
    }
}

/// Read one transcript with the module's safety contract: a canonical
/// no-follow regular file within the bounded transcript limit.
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    read_snapshot(path).map(|(bytes, _)| bytes)
}

pub fn read_snapshot(path: &Path) -> Result<(Vec<u8>, SourceStamp), String> {
    if is_compressed(path) {
        decoded_snapshot(path)
    } else {
        plain_snapshot(path)
    }
}

/// A compressed transcript, decoded. The file itself is read under the plain
/// contract (stamped before and after, within the limit), then its stream is
/// decoded through a reader bounded by the same limit. The frame header's
/// declared size is never trusted to skip that bound.
fn decoded_snapshot(path: &Path) -> Result<(Vec<u8>, SourceStamp), String> {
    use std::io::Read;

    let (compressed, stamp) = plain_snapshot(path)?;
    let limit = mallard_domain::MAX_CONVERSATION_FILE_BYTES;
    let mut bytes = Vec::new();
    zstd::stream::read::Decoder::with_buffer(std::io::Cursor::new(compressed.as_slice()))
        .and_then(|decoder| {
            decoder
                .take(limit.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|error| format!("decode session transcript '{}': {}", path.display(), error))?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "session transcript '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    Ok((bytes, stamp))
}

/// A bounded head read: the transcript's first bytes as a buffered reader,
/// decoded when the file is compressed, with the open handle's stamp.
/// Callers bound what they read with `take`; zstd decodes block by block,
/// so a short head of a compressed file costs about a short head.
pub fn open_head(path: &Path) -> Result<(SourceStamp, Box<dyn std::io::BufRead>), String> {
    let file = open_no_follow(path)?;
    let stamp = handle_stamp(&file, path)?;
    if is_compressed(path) {
        let decoder = zstd::stream::read::Decoder::new(file).map_err(|error| {
            format!("decode session transcript '{}': {}", path.display(), error)
        })?;
        Ok((stamp, Box::new(std::io::BufReader::new(decoder))))
    } else {
        Ok((stamp, Box::new(std::io::BufReader::new(file))))
    }
}

fn plain_snapshot(path: &Path) -> Result<(Vec<u8>, SourceStamp), String> {
    use std::io::Read;

    let mut file = open_no_follow(path)?;
    let stamp = handle_stamp(&file, path)?;
    if stamp.len > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "session transcript '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(stamp.len as usize);
    file.by_ref()
        .take(mallard_domain::MAX_CONVERSATION_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read session transcript '{}': {}", path.display(), error))?;
    let after = handle_stamp(&file, path)?;
    if bytes.len() as u64 > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "session transcript '{}' exceeds the bounded transcript limit",
            path.display()
        ));
    }
    if after != stamp || bytes.len() as u64 != stamp.len {
        return Err(format!(
            "session transcript '{}' changed while it was read",
            path.display()
        ));
    }
    crate::sync_metrics::record_transcript_bytes_read(path, bytes.len() as u64);
    stamp.verify(path)?;
    Ok((bytes, stamp))
}

/// The open handle's stamp, for a read that must name the exact file state
/// its bytes came from.
fn handle_stamp(file: &fs::File, path: &Path) -> Result<SourceStamp, String> {
    SourceStamp::from_file(file)
        .map_err(|error| format!("stat session transcript '{}': {}", path.display(), error))
}

/// Read a saved cursor's probe window and appended tail without reading the
/// consumed prefix: open the exact path without following a final symlink,
/// require a regular file within the transcript bound, read from the probe
/// window's start through EOF, and re-stamp the open handle afterwards,
/// retrying a bounded number of times while the file is changing. A file
/// already shorter than the cursor returns an empty window whose `file_len`
/// lets the provider refresh classify the truncation.
pub fn read_tail_window(path: &Path, mapping: &RootPhysicalMapping) -> Result<TailWindow, String> {
    use std::io::{Read, Seek, SeekFrom};
    let window_offset = mapping.consumed_byte_offset - mapping.probe_window_len;
    if is_compressed(path) {
        // Immutable while compressed: one decoded snapshot is the read.
        let (mut bytes, stamp) = decoded_snapshot(path)?;
        let file_len = bytes.len() as u64;
        let bytes = if file_len < mapping.consumed_byte_offset {
            Vec::new()
        } else {
            bytes.split_off(window_offset as usize)
        };
        return Ok(TailWindow {
            file_len,
            window_offset,
            bytes,
            stamp,
        });
    }
    let mut file = open_no_follow(path)?;
    let mut attempts = 0;
    loop {
        let before = handle_stamp(&file, path)?;
        if before.len > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
            return Err(format!(
                "session transcript '{}' exceeds the bounded transcript limit",
                path.display()
            ));
        }
        if before.len < mapping.consumed_byte_offset {
            return Ok(TailWindow {
                file_len: before.len,
                window_offset,
                bytes: Vec::new(),
                stamp: before,
            });
        }
        file.seek(SeekFrom::Start(window_offset))
            .map_err(|error| format!("seek session transcript '{}': {}", path.display(), error))?;
        let expected = before.len - window_offset;
        let mut bytes = Vec::with_capacity(expected as usize);
        (&file)
            .take(expected)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read session transcript '{}': {}", path.display(), error))?;
        if handle_stamp(&file, path)? == before && bytes.len() as u64 == expected {
            crate::sync_metrics::record_probe_bytes_read(path, mapping.probe_window_len);
            crate::sync_metrics::record_append_bytes_read(
                path,
                expected.saturating_sub(mapping.probe_window_len),
            );
            return Ok(TailWindow {
                file_len: before.len,
                window_offset,
                bytes,
                stamp: before,
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
/// of times while the file is changing. Addressed by size rather than by a
/// saved cursor: this is the metadata scan's bounded tail read, so the
/// bytes are attributed as transcript bytes, not probe/append bytes.
pub fn read_suffix_window(path: &Path, max_window_bytes: u64) -> Result<TailWindow, String> {
    use std::io::{Read, Seek, SeekFrom};
    if is_compressed(path) {
        let (mut bytes, stamp) = decoded_snapshot(path)?;
        let file_len = bytes.len() as u64;
        let window_offset = file_len.saturating_sub(max_window_bytes);
        let bytes = bytes.split_off(window_offset as usize);
        return Ok(TailWindow {
            file_len,
            window_offset,
            bytes,
            stamp,
        });
    }
    let mut file = open_no_follow(path)?;
    let mut attempts = 0;
    loop {
        let before = handle_stamp(&file, path)?;
        if before.len > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
            return Err(format!(
                "session transcript '{}' exceeds the bounded transcript limit",
                path.display()
            ));
        }
        let window_offset = before.len.saturating_sub(max_window_bytes);
        file.seek(SeekFrom::Start(window_offset))
            .map_err(|error| format!("seek session transcript '{}': {}", path.display(), error))?;
        let expected = before.len - window_offset;
        let mut bytes = Vec::with_capacity(expected as usize);
        (&file)
            .take(expected)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read session transcript '{}': {}", path.display(), error))?;
        if handle_stamp(&file, path)? == before && bytes.len() as u64 == expected {
            crate::sync_metrics::record_transcript_bytes_read(path, expected);
            return Ok(TailWindow {
                file_len: before.len,
                window_offset,
                bytes,
                stamp: before,
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
/// already trusts. The returned stamp is the open handle's, taken before
/// the read: a file that changes afterwards no longer carries it.
pub fn read_range(path: &Path, start: u64, end: u64) -> Result<(Vec<u8>, SourceStamp), String> {
    use std::io::{Read, Seek, SeekFrom};
    if end < start || end > mallard_domain::MAX_CONVERSATION_FILE_BYTES {
        return Err(format!(
            "session transcript '{}' range request is out of bounds",
            path.display()
        ));
    }
    if is_compressed(path) {
        let (bytes, stamp) = decoded_snapshot(path)?;
        if (bytes.len() as u64) < end {
            return Err(format!(
                "session transcript '{}' is shorter than the pinned range",
                path.display()
            ));
        }
        return Ok((bytes[start as usize..end as usize].to_vec(), stamp));
    }
    let mut file = open_no_follow(path)?;
    let before = handle_stamp(&file, path)?;
    if before.len < end {
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
    Ok((bytes, before))
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

    /// Every read names the file state its bytes came from, and a stat of
    /// the path yields the same stamp as the open handle did.
    #[test]
    fn reads_return_the_stamp_a_stat_reproduces() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("transcript.jsonl");
        fs::write(&path, b"aaaa\nbbbb\ncccc\n").unwrap();
        let mapping = RootPhysicalMapping {
            provider_thread_id: "thread".to_string(),
            root_path: path.to_string_lossy().into_owned(),
            source_file_size: 15,
            consumed_byte_offset: 10,
            consumed_record_ordinal: 2,
            probe_window_len: 5,
            probe_window_sha256: String::new(),
            last_canonical_message_id: None,
            last_synthetic_turn_id: None,
            next_synthetic_turn_ordinal: 0,
        };

        let stamp = SourceStamp::stat(&path).unwrap();
        assert_eq!(stamp, SourceStamp::read(&path).unwrap());
        assert_eq!(read_snapshot(&path).unwrap().1, stamp);
        assert_eq!(read_suffix_window(&path, 6).unwrap().stamp, stamp);
        let window = read_tail_window(&path, &mapping).unwrap();
        assert_eq!(window.bytes, b"bbbb\ncccc\n");
        assert_eq!(window.stamp, stamp);
        let (range, range_stamp) = read_range(&path, 0, 5).unwrap();
        assert_eq!(range, b"aaaa\n");
        assert_eq!(range_stamp, stamp);

        // A file shorter than the cursor still names its state.
        fs::write(&path, b"aaaa\n").unwrap();
        let truncated = read_tail_window(&path, &mapping).unwrap();
        assert!(truncated.bytes.is_empty());
        assert_eq!(truncated.stamp, SourceStamp::stat(&path).unwrap());
        assert_ne!(truncated.stamp, stamp);
    }

    #[test]
    fn stamps_hold_only_while_every_file_is_unchanged() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent.jsonl");
        let child = temp.path().join("child.jsonl");
        fs::write(&parent, b"{}\n").unwrap();
        fs::write(&child, b"{}\n").unwrap();
        let read = |path: &Path| SourceStamp::read(path).unwrap();

        assert!(
            !SourceStamps::default().hold(),
            "nothing read, nothing held"
        );
        let mut stamps = SourceStamps::of(&parent, &read(&parent));
        stamps.record(&child, &read(&child));
        stamps.record(&child, &read(&child));
        assert!(stamps.hold());

        // A writer that never closes its handle.
        let mut writer = fs::OpenOptions::new().append(true).open(&child).unwrap();
        writer.write_all(b"{}\n").unwrap();
        writer.flush().unwrap();
        assert!(!stamps.hold());

        // The same path under a second stamp leaves the set unpinned for
        // good, whatever the file does next.
        let mut twice = SourceStamps::of(&parent, &read(&parent));
        twice.record(&child, &read(&child));
        let mut merged = stamps.clone();
        merged.extend(&twice);
        assert!(!merged.is_pinned());
        assert!(twice.hold());

        let mut unstamped = SourceStamps::of(&parent, &read(&parent));
        unstamped.record_unstamped();
        assert!(!unstamped.is_pinned() && !unstamped.is_empty());

        fs::remove_file(&child).unwrap();
        assert!(!twice.hold(), "a vanished file holds nothing");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&parent, &child).unwrap();
            assert!(!twice.hold(), "a link is never followed");
        }
    }

    /// A saved cursor is used only on the file it was saved from, untouched
    /// or grown since. Every other change sends the reader to the whole
    /// file.
    #[test]
    fn a_cursor_continues_only_on_its_own_file_untouched_or_grown() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("transcript.jsonl");
        fs::write(&path, b"aaaa\nbbbb\n").unwrap();
        let now = || SourceStamp::stat(&path).unwrap();
        let restore_modified = |modified| {
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(modified)
                .unwrap()
        };

        let saved = now().file_state();
        saved.validate().unwrap();
        assert_eq!(saved.size, 10);
        assert_eq!(saved.identity.is_some(), cfg!(any(unix, windows)));
        assert!(now().continues(&saved), "untouched");
        assert_eq!(read_snapshot(&path).unwrap().1.file_state(), saved);

        // Grown through a handle that stays open.
        let mut writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(b"cccc\n").unwrap();
        writer.flush().unwrap();
        assert!(now().continues(&saved), "grown");
        let grown = now().file_state();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        drop(writer);

        // The change time gives away what a restored modification time
        // hides; a platform without one sees the size and that time only.
        fs::write(&path, b"aaaa\nBBBB\ncccc\n").unwrap();
        restore_modified(modified);
        assert_eq!(now().file_state().size, grown.size);
        assert_eq!(
            now().continues(&grown),
            cfg!(not(any(unix, windows))),
            "rewritten"
        );
        // Measured from the older state the same file has only grown: an
        // edit that comes with growth is not told apart here.
        assert!(now().continues(&saved));

        let rewritten = now().file_state();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
            assert!(!now().continues(&rewritten), "metadata alone");
        }

        fs::write(&path, b"aaaa\n").unwrap();
        assert!(!now().continues(&rewritten), "shrunk");
        assert!(!now().continues(&saved));

        // Another file under the same name, longer than the saved one.
        let replacement = temp.path().join("replacement.jsonl");
        fs::write(&replacement, b"aaaa\nbbbb\ncccc\ndddd\n").unwrap();
        fs::rename(&replacement, &path).unwrap();
        restore_modified(modified);
        assert_eq!(
            now().continues(&saved),
            cfg!(not(any(unix, windows))),
            "replaced"
        );

        // A modification time before the epoch keeps a forward nanosecond
        // count.
        restore_modified(std::time::UNIX_EPOCH - std::time::Duration::new(2, 250_000_000));
        let early = now().file_state();
        assert_eq!(
            (early.modified_secs, early.modified_nanos),
            (-3, 750_000_000)
        );
        early.validate().unwrap();
        assert!(now().continues(&early));
    }

    /// A compressed transcript reads as its decoded bytes under the file's
    /// own stamp, every window addresses decoded offsets, a head read decodes
    /// only what it takes, and a damaged stream is refused.
    #[test]
    fn compressed_transcripts_read_as_their_decoded_bytes() {
        use std::io::BufRead;
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("transcript.jsonl");
        let compressed = temp.path().join("transcript.jsonl.zst");
        let content = b"aaaa\nbbbb\ncccc\n";
        fs::write(&plain, content).unwrap();
        let stream = crate::jsonl_tail_io::compress_like_codex(&content[..]);
        fs::write(&compressed, &stream).unwrap();
        assert!(is_compressed(&compressed) && !is_compressed(&plain));
        assert_eq!(declared_decoded_len(&compressed), Some(15));
        assert_eq!(declared_decoded_len(&plain), None);

        let stamp = SourceStamp::stat(&compressed).unwrap();
        assert_eq!(stamp.file_len(), stream.len() as u64);
        let (bytes, read_stamp) = read_snapshot(&compressed).unwrap();
        assert_eq!(bytes, content);
        assert_eq!(read_stamp, stamp);
        let mapping = RootPhysicalMapping {
            provider_thread_id: "thread".to_string(),
            root_path: compressed.to_string_lossy().into_owned(),
            source_file_size: 15,
            consumed_byte_offset: 10,
            consumed_record_ordinal: 2,
            probe_window_len: 5,
            probe_window_sha256: String::new(),
            last_canonical_message_id: None,
            last_synthetic_turn_id: None,
            next_synthetic_turn_ordinal: 0,
        };
        let window = read_tail_window(&compressed, &mapping).unwrap();
        assert_eq!((window.file_len, window.window_offset), (15, 5));
        assert_eq!(window.bytes, b"bbbb\ncccc\n");
        assert_eq!(window.stamp, stamp);
        let suffix = read_suffix_window(&compressed, 6).unwrap();
        assert_eq!((suffix.file_len, suffix.window_offset), (15, 9));
        assert_eq!(suffix.bytes, b"\ncccc\n");
        let (range, range_stamp) = read_range(&compressed, 0, 5).unwrap();
        assert_eq!(range, b"aaaa\n");
        assert_eq!(range_stamp, stamp);
        assert!(read_range(&compressed, 0, 16)
            .unwrap_err()
            .contains("shorter than the pinned range"));
        let past_end = RootPhysicalMapping {
            consumed_byte_offset: 40,
            ..mapping
        };
        let truncated = read_tail_window(&compressed, &past_end).unwrap();
        assert!(truncated.bytes.is_empty());
        assert_eq!(truncated.file_len, 15);

        let (head_stamp, mut head) = open_head(&compressed).unwrap();
        assert_eq!(head_stamp, stamp);
        let mut line = Vec::new();
        head.read_until(b'\n', &mut line).unwrap();
        assert_eq!(line, b"aaaa\n");
        assert!(!head.fill_buf().unwrap().is_empty());

        fs::write(&compressed, b"\x28\xb5\x2f\xfd not a frame").unwrap();
        assert!(read_snapshot(&compressed)
            .unwrap_err()
            .contains("decode session transcript"));
        fs::write(&compressed, &stream[..stream.len() - 3]).unwrap();
        assert!(read_snapshot(&compressed).is_err());
        assert!(read_suffix_window(&compressed, 6).is_err());
    }

    /// The bound is on decoded bytes: a few KiB that decode past the limit
    /// are refused whatever the header declares, and every reader refuses
    /// them the same way.
    #[test]
    fn decoded_output_is_bounded_by_the_transcript_limit() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bomb.jsonl.zst");
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        let chunk = vec![b'\n'; 1024 * 1024];
        for _ in 0..(mallard_domain::MAX_CONVERSATION_FILE_BYTES / chunk.len() as u64 + 1) {
            encoder.write_all(&chunk).unwrap();
        }
        let stream = encoder.finish().unwrap();
        assert!(stream.len() < 1024 * 1024);
        fs::write(&path, stream).unwrap();
        assert_eq!(declared_decoded_len(&path), None);
        for error in [
            read_snapshot(&path).unwrap_err(),
            read_suffix_window(&path, 64).unwrap_err(),
            read_range(&path, 0, 8).unwrap_err(),
        ] {
            assert!(
                error.contains("exceeds the bounded transcript limit"),
                "{error}"
            );
        }
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
