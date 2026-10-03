//! Process-local reuse of session metadata scans.
//!
//! Discovery walks a provider home on every call and asks each transcript
//! which session it is and which folder owns it. That verdict is a function
//! of the file's bytes, so it is reused while the file's stamp is
//! unchanged. The stamp comes from a no-follow stat taken before the scan:
//! a stat shows every write at once, a writer that keeps the file open
//! included, and costs a fraction of the open a scan begins with.
//!
//! A rejection is a verdict too and is kept like an answer: a transcript
//! that names no session costs a whole read to find out. A file that could
//! not be read has no verdict, and is read again on the next call.

use crate::app_settings::conversation_file_limit_bytes;
use crate::jsonl_tail_io::SourceStamp;
use crate::providers::traits::{ScanFailure, SessionFileMetadata};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Several full profiles' worth of transcripts. Every entry stands on its
/// own stamp, so past the bound the memo simply starts over.
const MAX_ENTRIES: usize = 65_536;

struct Scanned {
    stamp: SourceStamp,
    /// A scan rejects a transcript past the conversation limit, so a
    /// verdict holds only under the limit it was reached with.
    conversation_limit: u64,
    verdict: Result<SessionFileMetadata, String>,
}

type Memo = HashMap<(&'static str, PathBuf), Scanned>;

fn memo() -> &'static Mutex<Memo> {
    static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
    MEMO.get_or_init(Default::default)
}

/// `scan(path)`, or its remembered verdict while the file is unchanged.
pub(crate) fn scan_once(
    provider: &'static str,
    path: &Path,
    scan: impl FnOnce(&Path) -> Result<SessionFileMetadata, ScanFailure>,
) -> Result<SessionFileMetadata, String> {
    // A path that cannot be stamped has no state to remember a verdict
    // for; the scan reports why.
    let Ok(stamp) = SourceStamp::stat(path) else {
        return scan(path).map_err(ScanFailure::into_message);
    };
    let conversation_limit = conversation_file_limit_bytes();
    let key = (provider, path.to_path_buf());
    if let Some(scanned) = memo()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .filter(|scanned| {
            scanned.stamp == stamp && scanned.conversation_limit == conversation_limit
        })
    {
        return scanned.verdict.clone();
    }
    let verdict = match scan(path) {
        Ok(metadata) => Ok(metadata),
        Err(ScanFailure::Rejected(message)) => Err(message),
        Err(ScanFailure::Unreadable(message)) => return Err(message),
    };
    let mut memo = memo().lock().unwrap_or_else(|error| error.into_inner());
    if memo.len() >= MAX_ENTRIES && !memo.contains_key(&key) {
        memo.clear();
    }
    memo.insert(
        key,
        Scanned {
            stamp,
            conversation_limit,
            verdict: verdict.clone(),
        },
    );
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::Write;

    fn metadata(session_id: &str) -> SessionFileMetadata {
        SessionFileMetadata {
            session_id: session_id.to_string(),
            authorized_home: PathBuf::from("/work/project"),
            is_internal_subagent: false,
            is_empty: false,
        }
    }

    struct Scans<'a> {
        path: &'a Path,
        count: Cell<u32>,
    }

    impl Scans<'_> {
        fn scan(&self, outcome: Result<&str, ScanFailure>) -> Result<String, String> {
            scan_once("memo-test", self.path, |_| {
                self.count.set(self.count.get() + 1);
                outcome.map(metadata)
            })
            .map(|metadata| metadata.session_id)
        }
    }

    #[test]
    fn an_answer_is_reused_until_the_file_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        std::fs::write(&path, b"{\"turn\":1}\n").unwrap();
        let scans = Scans {
            path: &path,
            count: Cell::new(0),
        };
        assert_eq!(scans.scan(Ok("first")).unwrap(), "first");
        assert_eq!(scans.scan(Ok("unused")).unwrap(), "first");
        assert_eq!(scans.count.get(), 1);

        // The writer keeps its handle open: no event, a new stamp.
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writer.write_all(b"{\"turn\":2}\n").unwrap();
        writer.flush().unwrap();
        assert_eq!(scans.scan(Ok("second")).unwrap(), "second");
        assert_eq!(scans.count.get(), 2);
        drop(writer);
        assert_eq!(scans.scan(Ok("unused")).unwrap(), "second");
        assert_eq!(scans.count.get(), 2);

        // A different file at the same path, byte for byte the same size.
        let replacement = temp.path().join("replacement.jsonl");
        std::fs::write(&replacement, std::fs::read(&path).unwrap()).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(scans.scan(Ok("third")).unwrap(), "third");
        assert_eq!(scans.count.get(), 3);
    }

    #[test]
    fn a_rejection_is_kept_and_a_read_failure_is_not() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        std::fs::write(&path, b"{}\n").unwrap();
        let scans = Scans {
            path: &path,
            count: Cell::new(0),
        };
        let unreadable = || Err(ScanFailure::Unreadable("too many open files".into()));
        assert_eq!(scans.scan(unreadable()).unwrap_err(), "too many open files");
        assert_eq!(scans.scan(unreadable()).unwrap_err(), "too many open files");
        assert_eq!(scans.count.get(), 2, "a read failure is never remembered");

        let rejected = || Err(ScanFailure::Rejected("names no session".into()));
        assert_eq!(scans.scan(rejected()).unwrap_err(), "names no session");
        assert_eq!(scans.scan(Ok("unused")).unwrap_err(), "names no session");
        assert_eq!(scans.count.get(), 3);

        std::fs::write(&path, b"{\"sessionId\":\"now\"}\n").unwrap();
        assert_eq!(scans.scan(Ok("now")).unwrap(), "now");
        assert_eq!(scans.count.get(), 4);
    }

    #[test]
    fn a_path_without_a_stamp_is_always_scanned() {
        let temp = tempfile::tempdir().unwrap();
        let scans = Scans {
            path: temp.path(),
            count: Cell::new(0),
        };
        assert_eq!(scans.scan(Ok("directory")).unwrap(), "directory");
        assert_eq!(scans.scan(Ok("directory")).unwrap(), "directory");
        assert_eq!(scans.count.get(), 2);
    }
}
