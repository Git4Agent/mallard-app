//! Replay a completed native append through a writer kept open during comparison.
//! This uses only the disposable E2E transcript. It makes no provider calls.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub fn with_replayed_native_append<T>(
    fixture_root: &Path,
    path: &Path,
    before: &[u8],
    warm: impl FnOnce(),
    compare: impl FnOnce() -> T,
) -> T {
    let root = fixture_root.canonicalize().expect("canonical fixture root");
    let metadata = std::fs::symlink_metadata(path).expect("native transcript metadata");
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert!(path
        .canonicalize()
        .expect("canonical transcript")
        .starts_with(&root));
    let limit = mallard_domain::MAX_CONVERSATION_FILE_BYTES;
    let mut after = Vec::new();
    std::fs::File::open(path)
        .expect("open native transcript")
        .take(limit + 1)
        .read_to_end(&mut after)
        .expect("read native transcript");
    assert!(
        after.len() as u64 <= limit,
        "native fixture exceeds the file limit"
    );
    assert!(
        after.len() > before.len() && after.starts_with(before),
        "native resume must append to this fixture"
    );

    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open replay writer");
    writer
        .set_len(before.len() as u64)
        .expect("restore native prefix");
    writer.sync_all().expect("flush native prefix");
    warm();
    writer
        .seek(SeekFrom::End(0))
        .expect("seek to native suffix");
    writer
        .write_all(&after[before.len()..])
        .expect("replay native suffix");
    writer.sync_all().expect("flush native suffix");
    let result = compare();
    assert_eq!(
        writer.metadata().expect("writer remains open").len(),
        after.len() as u64
    );
    assert!(
        std::fs::read(path).expect("read replayed transcript") == after,
        "replay changed native bytes"
    );
    drop(writer);
    result
}
